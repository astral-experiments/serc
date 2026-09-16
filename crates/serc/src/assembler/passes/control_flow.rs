use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    BorrowFallthrough, COMPARE_OP, CONTAINS_OP, COPY, ExceptionExclusion, IS_OP, Instruction, Item,
    JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, LOAD_COMMON_CONSTANT, Label, NOP,
    NOT_TAKEN, NormalizedExceptionOwner, Operand, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, POP_TOP,
    STORE_FAST, STORE_FAST_STORE_FAST, SourceLocation, TO_BOOL, UNARY_NOT, UNPACK_SEQUENCE,
    ends_scope, is_unconditional_jump,
};
use crate::assembler::ConstantTable;
use crate::assembler::OptimizationPipeline;
use crate::assembler::block_graph::{BlockGraph, ItemMut, ItemPosition};
use crate::code_object::Constant;
use crate::target::Opcode;
use crate::target::opcodes::{
    LOAD_CONST, LOAD_GLOBAL, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, RAISE_VARARGS, RERAISE,
    RETURN_VALUE, SWAP,
};
use crate::target::operands::ComparisonOperation;

/// Drops a folded Boolean slot's line event when the original block already records that line.
pub(super) fn clear_redundant_boolean_nop_location(
    instruction: &mut Instruction,
    was_boolean_conversion: bool,
    previous_line: &mut Option<i32>,
) {
    if instruction.opcode == NOP {
        if was_boolean_conversion && *previous_line == Some(instruction.location.line) {
            // Keep the original slot for copy-size and exception-owner accounting, but do not
            // restore this redundant line when later passes merge or copy the block.
            instruction.location = SourceLocation::NONE;
            instruction.no_location_policy.preserve_location();
        } else {
            // Another NOP may preserve a distinct line event. Leave its compaction to the
            // ordinary pass rather than treating a physical predecessor as an original one.
            *previous_line = None;
        }
    } else {
        *previous_line = Some(instruction.location.line);
    }
}

/// Keeps the NOP left by CPython's earlier None-comparison constant pass.
pub(super) fn remove_none_comparison_boolean_conversions<'a>(
    items: impl Iterator<Item = ItemMut<'a>>,
    constants: &dyn ConstantTable,
) {
    let mut previous_loads_none = false;
    let mut previous_compares_none = false;
    for item in items {
        let ItemMut::Instruction(instruction) = item else {
            previous_loads_none = false;
            previous_compares_none = false;
            continue;
        };
        let compares_none = previous_loads_none && instruction.opcode == IS_OP;
        let loads_none = match (instruction.opcode, instruction.operand()) {
            (LOAD_CONST, Operand::Value(index)) => matches!(constants.get(index), Constant::None),
            // The constant pass carries its LOAD_CONST through consecutive COPY 1 slots.
            (COPY, Operand::Value(1)) => previous_loads_none,
            _ => false,
        };
        if previous_compares_none && instruction.opcode == TO_BOOL {
            // Preserve the comparison's original slot and the gap before UNARY_NOT.
            // The later Boolean pass must not see them as adjacent instructions.
            instruction.opcode = NOP;
            instruction.set_operand(Operand::Value(0));
        }
        previous_loads_none = loads_none;
        previous_compares_none = compares_none;
    }
}

/// Counts the original target instructions before graph normalization splits inline labels.
fn targets_small_scope_exit(graph: &BlockGraph, target: Label) -> bool {
    let mut started = false;
    let mut size = 0;
    for item in graph.iter_items() {
        if !started {
            started = matches!(item, Item::Label(label) if *label == target);
            continue;
        }
        match item {
            Item::Label(_) if size > 0 => return false,
            Item::Label(_) => {}
            Item::Instruction(instruction) => {
                let fused_push_null = instruction.opcode == LOAD_GLOBAL
                    && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0);
                size += 1
                    + usize::from(fused_push_null)
                    + usize::from(instruction.pre_fold_extra_instructions());
                if size > 4 {
                    return false;
                }
                if ends_scope(instruction.opcode) {
                    // Synthetic fallthrough exits retain their existing no-location copy
                    // policy, including the boundaries around normal-finally bodies.
                    return instruction.location.line >= 0;
                }
                if matches!(
                    instruction.operand(),
                    Operand::Forward(_) | Operand::Backward(_)
                ) {
                    return false;
                }
            }
        }
    }
    false
}

/// Recognizes the three original slots of a same-line explicit constant return.
pub(super) fn is_original_constant_return(graph: &BlockGraph, target: Label) -> bool {
    let Some(block) = graph.label_block(target) else {
        return false;
    };
    let mut started = false;
    let mut instructions = Vec::new();
    for item in graph[block].items() {
        match item {
            Item::Label(label) if !started => started = *label == target,
            Item::Label(_) if instructions.is_empty() => {}
            Item::Label(_) => return false,
            Item::Instruction(instruction) if started => instructions.push(instruction),
            Item::Instruction(_) => {}
        }
    }
    let [marker, value, ret] = instructions.as_slice() else {
        return false;
    };
    marker.opcode == NOP
        // The compiler emits nonnegative u8 literals as LOAD_SMALL_INT before
        // this pass; CPython's original LOAD_CONST still occupies one CFG slot.
        && (value.opcode == LOAD_CONST
            || (value.opcode == crate::target::opcodes::LOAD_SMALL_INT
                && matches!(value.operand(), Operand::Value(0..=255))))
        && ret.opcode == RETURN_VALUE
        && marker.location.line >= 0
        && marker.location == value.location
        && marker.location == ret.location
        && instructions.iter().all(|instruction| {
            matches!(instruction.operand(), Operand::Value(_))
                && instruction.pre_fold_extra_instructions() == 0
        })
}

/// Finds redundant `COPY 1`/`POP_TOP` pairs without crossing labels or other instructions.
///
/// A stack of consecutive copies also catches pairs exposed when an inner pair is removed.
/// CPython performs this compaction after redundant-store replacement and before local analysis.
pub(super) fn redundant_copy_pop_indices(items: &[Item]) -> Vec<usize> {
    let mut copies = Vec::new();
    let mut redundant = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            Item::Instruction(instruction)
                if instruction.opcode == COPY && instruction.operand() == Operand::Value(1) =>
            {
                copies.push(index);
            }
            Item::Instruction(instruction) if instruction.opcode == POP_TOP => {
                if let Some(copy) = copies.pop() {
                    redundant.extend([copy, index]);
                }
            }
            _ => copies.clear(),
        }
    }
    redundant.sort_unstable();
    redundant
}

impl OptimizationPipeline {
    /// Returns whether every branch is an ordinary, terminal assertion side exit.
    ///
    /// These graphs have no unreachable blocks, forward-jump threading, exit copies, or cold
    /// exception handlers. Keeping the proof exact lets the optimizer retain the local and
    /// location passes while skipping their otherwise repeated CFG scans.
    pub(in crate::assembler) fn has_only_assertion_side_exits(&self) -> bool {
        if !self.has_assertion_side_exit_candidate
            || self.graph.len() < 3
            || !self.exception_regions.is_empty()
            || !self.preserved_block_boundaries.is_empty()
            || !self.borrow_unreachable_blocks.is_empty()
            || self.has_synthesized_percent_target_locations
        {
            return false;
        }

        let mut items = self.graph.iter_items();
        let mut assertion_count = 0;
        let mut ended_scope = false;
        while let Some(item) = items.next() {
            if ended_scope {
                return false;
            }
            let Item::Instruction(instruction) = item else {
                return false;
            };
            if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier {
                return false;
            }
            if matches!(
                instruction.opcode,
                POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
            ) {
                let Operand::Forward(target) = instruction.operand() else {
                    return false;
                };
                if !matches!(
                    items.next(),
                    Some(Item::Instruction(instruction))
                        if instruction.opcode == NOT_TAKEN
                            && matches!(instruction.operand(), Operand::Value(0))
                ) || !matches!(
                    items.next(),
                    Some(Item::Instruction(instruction))
                        if instruction.opcode == LOAD_COMMON_CONSTANT
                            && matches!(instruction.operand(), Operand::Value(0))
                ) || !matches!(
                    items.next(),
                    Some(Item::Instruction(instruction))
                        if instruction.opcode == RAISE_VARARGS
                            && matches!(instruction.operand(), Operand::Value(1))
                ) || !matches!(items.next(), Some(Item::Label(label)) if *label == target)
                {
                    return false;
                }
                assertion_count += 1;
                continue;
            }
            if instruction.opcode == NOT_TAKEN
                || !matches!(instruction.operand(), Operand::Value(_))
            {
                return false;
            }
            ended_scope = ends_scope(instruction.opcode);
        }

        assertion_count > 0 && ended_scope
    }

    /// Propagates an assertion condition location into its wholly unlocated continuation block.
    ///
    /// `duplicate_exit_blocks` performs this repair even when it creates no copies. The assertion
    /// fast path has one jump predecessor per continuation, so no predecessor table is needed.
    pub(in crate::assembler) fn propagate_assertion_exit_locations(&mut self) {
        let order = self.graph.order().to_vec();
        for blocks in order.windows(3) {
            let [source, _, target] = blocks else {
                unreachable!();
            };
            let Some(jump) = self.graph[*source].last_instruction() else {
                continue;
            };
            if !matches!(
                jump.opcode,
                POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
            ) {
                continue;
            }
            if self.graph[*target].items().iter().any(
                |item| matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0),
            ) {
                continue;
            }
            let location = jump.location;
            if let Some(first) = self.graph.instructions_mut(*target).next()
                && !first.no_location_policy.preserves_location()
            {
                first.location = location;
            }
        }
    }

    /// Replaces an overwritten local store with a stack pop.
    ///
    /// CPython applies this peephole before superinstruction fusion when two
    /// adjacent stores target the same local on the same traced line.
    pub(in crate::assembler) fn optimize_redundant_store_fast(&mut self) {
        let graph = &mut self.graph;
        if !graph.features().has_store_fast() {
            return;
        }
        graph.for_each_adjacent_instruction_pair_mut(|first, second| {
            if first.opcode == STORE_FAST
                && second.opcode == STORE_FAST
                // STORE_FAST_MAYBE_NULL remains a distinct pseudo-op through this pass.
                && !first.local_policy.may_store_null()
                && !second.local_policy.may_store_null()
                && matches!(
                    (first.operand(), second.operand()),
                    (Operand::Value(first), Operand::Value(second)) if first == second
                )
                && first.location.line == second.location.line
            {
                first.opcode = POP_TOP;
                first.set_operand(Operand::Value(0));
            }
        });
    }

    /// Removes copy/pop pairs before local analysis and superinstruction fusion.
    pub(in crate::assembler) fn remove_redundant_copy_pop_pairs(&mut self) -> Option<u32> {
        let mut redundant = Vec::new();
        let mut vacated_slots = Vec::new();
        for block in self.graph.order().iter().copied() {
            let removed = redundant_copy_pop_indices(self.graph[block].items());
            if removed.is_empty() {
                continue;
            }
            let mut removed_offsets = removed.iter().copied().peekable();
            let mut pending_slots = 0_u16;
            let mut pending_folds = 0_u8;
            for (offset, item) in self.graph[block].items().iter().enumerate() {
                if removed_offsets.peek() == Some(&offset) {
                    let Item::Instruction(instruction) = item else {
                        unreachable!("a redundant copy/pop is an instruction");
                    };
                    removed_offsets.next();
                    pending_slots = pending_slots
                        .saturating_add(1)
                        .saturating_add(instruction.local_policy.hidden_exception_owner_slots());
                    pending_folds =
                        pending_folds.saturating_add(instruction.pre_fold_extra_instructions());
                } else if pending_slots > 0 {
                    if matches!(item, Item::Instruction(_)) {
                        vacated_slots.push((
                            ItemPosition::new(block, offset),
                            pending_slots,
                            pending_folds,
                        ));
                    }
                    // A label or the end of this block cannot donate erased cells
                    // to a different original array.
                    pending_slots = 0;
                    pending_folds = 0;
                }
            }
            redundant.extend(
                removed
                    .into_iter()
                    .map(|offset| ItemPosition::new(block, offset)),
            );
        }
        for (position, slots, folds) in vacated_slots {
            let instruction = self
                .graph
                .instruction_mut(position)
                .expect("surviving copy/pop successor");
            // CPython compacts these cells before local fusion and marker insertion.
            // Preserve earlier fold uncertainty, without creating a late fold record.
            instruction
                .local_policy
                .add_hidden_exception_owner_slots(slots);
            if folds > 0 {
                instruction.add_pre_fold_extra_instructions(folds);
            }
        }
        let mut removed_max_depth = None;
        for position in redundant.into_iter().rev() {
            if let Item::Instruction(instruction) = self.graph.remove_item(position)
                && let Some(depth) = instruction.depth_after()
            {
                removed_max_depth =
                    Some(removed_max_depth.map_or(depth, |current: u32| current.max(depth)));
            }
        }
        removed_max_depth
    }

    pub(in crate::assembler) fn remove_unreachable_instructions(&mut self) -> Option<u32> {
        let graph = &mut self.graph;
        if self.exception_regions.is_empty() && graph.len() == 1 {
            return None;
        }
        let reachable = self
            .initial_reachable
            .take()
            .unwrap_or_else(|| graph.reachable_blocks(&self.exception_regions));
        // CPython clears handler status when erasing an unreachable block. Keep that
        // decision on its original labels before normalization can attach them to a
        // live ordinary exit; the exception regions themselves retain their identity.
        self.retired_exception_handler_targets
            .extend(self.exception_regions.iter().filter_map(|region| {
                let target = graph.label_block(region.target)?;
                (!reachable[target.index()]).then_some(region.target)
            }));
        let mut removed_max_depth = None;
        let order = graph.order().to_vec();
        let mut empty = FxHashSet::default();
        let mut discarded_admissions = Vec::new();
        for block in order {
            if reachable[block.index()] {
                if !discarded_admissions.is_empty()
                    && let Some(label) = graph[block].first_label()
                {
                    if let Some(mut admissions) = self.discarded_fold_admissions.remove(&label) {
                        discarded_admissions.append(&mut admissions);
                    }
                    self.discarded_fold_admissions
                        .insert(label, std::mem::take(&mut discarded_admissions));
                }
                continue;
            }
            let had_instructions = graph[block].last_instruction().is_some();
            graph.retain_items(block, |item| match item {
                Item::Label(label) => {
                    // A label-only unreachable array no longer certifies copy continuity.
                    self.exception_owner_provenance
                        .normal_try_copy_inline_labels
                        .remove(label);
                    self.original_unlocated_discard_targets.remove(label);
                    if let Some(mut admissions) = self.discarded_fold_admissions.remove(label) {
                        discarded_admissions.append(&mut admissions);
                    }
                    true
                }
                Item::Instruction(instruction) => {
                    // Front-folded expressions can disappear before CPython would fold
                    // their original conditional block. Keep their admission position;
                    // surviving loads still determine which constants are retained.
                    if instruction.opcode == LOAD_CONST
                        && instruction.pre_fold_extra_instructions() > 0
                        && let Operand::Value(index) = instruction.operand()
                    {
                        discarded_admissions.push(index);
                    }
                    if let Some(depth) = instruction.depth_after() {
                        removed_max_depth =
                            Some(removed_max_depth.map_or(depth, |max: u32| max.max(depth)));
                    }
                    false
                }
            });
            if had_instructions {
                // The unreachable array's instructions were erased above. Record its
                // original array labels before normalization can attach these labels
                // to a different live array; keep the original copy plan intact.
                for item in graph[block].items() {
                    if let Item::Label(label) = item {
                        self.exception_owner_provenance
                            .retire_async_with_setup(*label);
                    }
                    if let Item::Label(label) = item
                        && self
                            .exception_owner_provenance
                            .original_comprehension_normal_exits
                            .iter()
                            .any(|(entry, _)| entry == label)
                    {
                        self.exception_owner_provenance
                            .retired_comprehension_normal_entries
                            .insert(*label);
                    }
                    if let Item::Label(label) = item
                        && (self
                            .exception_owner_provenance
                            .normal_try_copy_blocks
                            .contains(label)
                            || self
                                .exception_owner_provenance
                                .normal_try_copy_internal_labels
                                .contains(label)
                            || self
                                .exception_owner_provenance
                                .normal_try_copy_join_origins
                                .iter()
                                .any(|(join, origin)| {
                                    join == label || origin.predecessor == *label
                                }))
                    {
                        self.exception_owner_provenance
                            .retired_normal_try_copy_labels
                            .insert(*label);
                    }
                }
            }
            if graph[block].items().is_empty() {
                empty.insert(block);
            }
        }
        if !empty.is_empty() {
            graph.retain_order(|block| !empty.contains(&block));
        }
        removed_max_depth
    }

    #[cfg(any(test, debug_assertions))]
    pub(in crate::assembler) fn reachable_items(&self) -> Vec<bool> {
        self.graph.item_reachability(&self.exception_regions)
    }

    pub(in crate::assembler) fn optimize_boolean_conversions(
        &mut self,
        constants: Option<&dyn ConstantTable>,
    ) {
        let graph = &mut self.graph;
        if !graph.features().has_boolean_conversion() {
            return;
        }
        let was_boolean_conversion = graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => {
                    Some(matches!(instruction.opcode, TO_BOOL | UNARY_NOT))
                }
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        if let Some(constants) = constants {
            for block in graph.order().to_vec() {
                remove_none_comparison_boolean_conversions(graph.iter_items_mut(block), constants);
            }
        }
        graph.for_each_adjacent_instruction_pair_mut(|current, next| {
            match (current.opcode, next.opcode) {
                (TO_BOOL, TO_BOOL) => {
                    current.opcode = NOP;
                    current.set_operand(Operand::Value(0));
                    return;
                }
                (UNARY_NOT, TO_BOOL) => {
                    current.opcode = NOP;
                    current.set_operand(Operand::Value(0));
                    next.opcode = UNARY_NOT;
                    next.set_operand(Operand::Value(0));
                    return;
                }
                (UNARY_NOT, UNARY_NOT) => {
                    current.opcode = NOP;
                    current.set_operand(Operand::Value(0));
                    next.opcode = NOP;
                    next.set_operand(Operand::Value(0));
                    return;
                }
                (CONTAINS_OP | IS_OP, UNARY_NOT) => {
                    let Operand::Value(argument) = current.operand() else {
                        return;
                    };
                    let opcode = current.opcode;
                    current.opcode = NOP;
                    current.set_operand(Operand::Value(0));
                    next.opcode = opcode;
                    next.set_operand(Operand::Value(argument ^ 1));
                    return;
                }
                _ => {}
            }
            if next.opcode != TO_BOOL {
                return;
            }
            let Operand::Value(argument) = current.operand() else {
                return;
            };
            let replacement = match current.opcode {
                COMPARE_OP => Some((COMPARE_OP, ComparisonOperation::force_boolean(argument))),
                CONTAINS_OP => Some((CONTAINS_OP, argument)),
                IS_OP => Some((IS_OP, argument)),
                _ => None,
            };
            let Some((opcode, argument)) = replacement else {
                return;
            };
            current.opcode = NOP;
            current.set_operand(Operand::Value(0));
            next.opcode = opcode;
            next.set_operand(Operand::Value(argument));
        });
        let mut was_boolean_conversion = was_boolean_conversion.into_iter();
        for block in graph.order().to_vec() {
            let mut previous_line = None;
            for instruction in graph.instructions_mut(block) {
                clear_redundant_boolean_nop_location(
                    instruction,
                    was_boolean_conversion
                        .next()
                        .expect("original instruction slot"),
                    &mut previous_line,
                );
            }
        }
    }

    pub(in crate::assembler) fn thread_forward_jumps(&mut self) -> bool {
        #[derive(Clone, Copy)]
        struct ConditionalTarget {
            opcode: Opcode,
            operand: Operand,
            fallthrough_index: usize,
        }

        fn value_preserving_jump(
            graph: &BlockGraph,
            positions: &[ItemPosition],
            index: usize,
        ) -> Option<[ItemPosition; 3]> {
            let Item::Instruction(instruction) = *graph.item(positions[index]) else {
                return None;
            };
            if !matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                || !matches!(instruction.operand(), Operand::Forward(_))
            {
                return None;
            }
            let mut previous = positions[..index].iter().rev().filter_map(|position| {
                let Item::Instruction(instruction) = *graph.item(*position) else {
                    return None;
                };
                (instruction.opcode != NOP).then_some((*position, instruction))
            });
            let (to_bool_position, to_bool) = previous.next()?;
            let (copy_position, copy) = previous.next()?;
            (to_bool.opcode == TO_BOOL
                && copy.opcode == COPY
                && matches!(copy.operand(), Operand::Value(1)))
            .then_some([copy_position, to_bool_position, positions[index]])
        }

        self.borrow_threaded_jump_entries.clear();
        // CPython threads the value-preserving jumps used by boolean
        // expressions through a nested boolean test. If both tests jump on
        // the same truth value, they share the final target. Otherwise the
        // inner jump skips the redundant outer test and enters its POP_TOP
        // fallthrough path directly.
        let graph = &self.graph;
        let features = graph.features();
        let has_forward_jump = features.has_forward_jump();
        let has_forward_conditional = features.has_forward_conditional();
        if !has_forward_jump && !has_forward_conditional {
            self.borrow_authored_finally_break_origins =
                crate::assembler::ir::AuthoredFinallyBreakOrigins::default();
            return false;
        }
        let mut changed = false;

        if has_forward_conditional {
            let mut positions = graph.item_positions();
            let mut conditional_targets =
                vec![None; usize::try_from(self.next_label).expect("label count fits in usize")];
            let mut conditional_target_count = 0;
            for (index, position) in positions.iter().copied().enumerate() {
                let Item::Label(label) = *graph.item(position) else {
                    continue;
                };
                let mut following = positions[index + 1..]
                    .iter()
                    .copied()
                    .enumerate()
                    .filter_map(|(offset, position)| {
                        let Item::Instruction(instruction) = *graph.item(position) else {
                            return None;
                        };
                        (instruction.opcode != NOP).then_some((index + 1 + offset, instruction))
                    });
                let Some((_, copy)) = following.next() else {
                    continue;
                };
                let Some((_, to_bool)) = following.next() else {
                    continue;
                };
                let Some((_, jump)) = following.next() else {
                    continue;
                };
                let Some((_, not_taken)) = following.next() else {
                    continue;
                };
                let Some((fallthrough_index, pop)) = following.next() else {
                    continue;
                };
                if copy.opcode != COPY
                    || !matches!(copy.operand(), Operand::Value(1))
                    || to_bool.opcode != TO_BOOL
                    || !matches!(jump.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                    || !matches!(jump.operand(), Operand::Forward(_))
                    || not_taken.opcode != NOT_TAKEN
                    || pop.opcode != POP_TOP
                {
                    continue;
                }
                let target = &mut conditional_targets
                    [usize::try_from(label.0).expect("label index fits in usize")];
                conditional_target_count += usize::from(
                    target
                        .replace(ConditionalTarget {
                            opcode: jump.opcode,
                            operand: jump.operand(),
                            fallthrough_index,
                        })
                        .is_none(),
                );
            }

            if conditional_target_count > 0 {
                let mut fallthrough_indices = positions
                    .iter()
                    .copied()
                    .enumerate()
                    .filter_map(|(index, position)| {
                        let Item::Instruction(instruction) = *graph.item(position) else {
                            return None;
                        };
                        value_preserving_jump(graph, &positions, index)?;
                        let Operand::Forward(target) = instruction.operand() else {
                            return None;
                        };
                        let target = conditional_targets
                            .get(usize::try_from(target.0).expect("label index fits in usize"))
                            .copied()
                            .flatten()?;
                        (instruction.opcode != target.opcode).then_some(target.fallthrough_index)
                    })
                    .collect::<Vec<_>>();
                fallthrough_indices.sort_unstable();
                fallthrough_indices.dedup();
                let fallthrough_labels = fallthrough_indices
                    .iter()
                    .map(|index| (*index, self.label()))
                    .collect::<FxHashMap<_, _>>();
                if !fallthrough_indices.is_empty() {
                    changed = true;
                    let graph = &mut self.graph;
                    for index in fallthrough_indices.iter().rev().copied() {
                        graph
                            .insert_item(positions[index], Item::Label(fallthrough_labels[&index]));
                    }
                    positions = graph.item_positions();
                }

                for (index, position) in positions.iter().copied().enumerate() {
                    let Some(value_jump) = value_preserving_jump(&self.graph, &positions, index)
                    else {
                        continue;
                    };
                    let Some(instruction) = self.graph.instruction_mut(position) else {
                        continue;
                    };
                    if !matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE) {
                        continue;
                    }
                    let Operand::Forward(mut target) = instruction.operand() else {
                        continue;
                    };
                    let mut replaced = false;
                    for _ in 0..conditional_target_count {
                        let Some(next) = conditional_targets
                            .get(usize::try_from(target.0).expect("label index fits in usize"))
                            .copied()
                            .flatten()
                        else {
                            break;
                        };
                        if instruction.opcode == next.opcode {
                            let Operand::Forward(next_target) = next.operand else {
                                break;
                            };
                            if next_target == target {
                                break;
                            }
                            instruction.retarget(next.operand);
                            replaced = true;
                            changed = true;
                            target = next_target;
                        } else {
                            let Some(fallthrough) = fallthrough_labels.get(&next.fallthrough_index)
                            else {
                                break;
                            };
                            instruction.retarget(Operand::Forward(*fallthrough));
                            changed = true;
                            break;
                        }
                    }
                    if replaced {
                        // CPython appends the same-polarity replacement before the first
                        // compaction. Its fresh ownerless slot supplies COPY, TO_BOOL, and
                        // the conditional when the value-preserving jump is expanded.
                        for position in value_jump {
                            let instruction = self.graph.instruction_mut(position).unwrap();
                            instruction.set_exception_owner(NormalizedExceptionOwner::Unchanged);
                            instruction.exclude_from_exception_table(ExceptionExclusion::Always);
                        }
                        let marker = positions[index + 1..]
                            .iter()
                            .find_map(|position| match *self.graph.item(*position) {
                                Item::Instruction(instruction) => Some(instruction),
                                Item::Label(_) => None,
                            });
                        if let Some(marker) = marker
                            && marker.opcode == NOT_TAKEN
                            && marker.exception_policy().owner()
                                == NormalizedExceptionOwner::Dynamic
                        {
                            let condition = *self.graph.instruction_mut(position).unwrap();
                            self.exception_owner_provenance
                                .threaded_condition_origins
                                .push((condition, marker));
                        }
                    }
                }
            }
        }

        if !has_forward_jump {
            self.borrow_authored_finally_break_origins =
                crate::assembler::ir::AuthoredFinallyBreakOrigins::default();
            return changed;
        }

        let graph = &self.graph;
        let mut label_offsets = FxHashMap::default();
        let mut instruction_positions = Vec::new();
        for (block, contents) in graph.iter() {
            for (offset, item) in contents.items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        label_offsets.insert(*label, instruction_positions.len());
                    }
                    Item::Instruction(_) => instruction_positions.push((block, offset)),
                }
            }
        }
        // A scheduled copy must still observe its original source target.
        let planned_blocks = self
            .exception_owner_provenance
            .normal_try_copy_blocks
            .iter()
            .filter(|label| {
                !self
                    .exception_owner_provenance
                    .retired_normal_try_copy_labels
                    .contains(*label)
            })
            .filter_map(|label| graph.label_block(*label))
            .collect::<FxHashSet<_>>();
        let planned_sources = instruction_positions
            .iter()
            .enumerate()
            .filter_map(|(index, (block, _))| planned_blocks.contains(block).then_some(index))
            .collect::<FxHashSet<_>>();
        let region_ranges = self
            .exception_regions
            .iter()
            .map(|region| label_offsets[&region.start]..label_offsets[&region.end])
            .collect::<Vec<_>>();
        let pop_block_offsets = self
            .exception_regions
            .iter()
            .map(|region| label_offsets[&region.end])
            .collect::<FxHashSet<_>>();
        // Validate the compiler's original exit records against the current graph. An
        // exception-range end alone does not prove that a POP_BLOCK occupied this slot.
        let normal_try_exits = self
            .exception_owner_provenance
            .normal_try_exits
            .iter()
            .filter_map(|(entry, target)| {
                let entry_index = *label_offsets.get(entry)?;
                let target_index = *label_offsets.get(target)?;
                let (block, offset) = *instruction_positions.get(entry_index)?;
                let Item::Instruction(instruction) = graph[block].items()[offset] else {
                    return None;
                };
                (entry_index < target_index
                    && instruction.opcode == JUMP_FORWARD
                    && instruction.operand() == Operand::Forward(*target)
                    && instruction.pre_fold_extra_instructions() == 0)
                    .then_some(entry_index)
            })
            .collect::<FxHashSet<_>>();
        // CPython copies an unlocated branch-tail array before resolving the
        // ordinary try cleanup's line. Preserve its existing line event through
        // our earlier threading; the small-exit copier still decides whether a
        // physical NOP is needed. Do not infer this from an optimized empty head.
        let transparent_normal_try_tails = normal_try_exits
            .iter()
            .copied()
            .filter(|entry_index| {
                let (block, offset) = instruction_positions[*entry_index];
                let items = graph[block].items();
                let Item::Instruction(jump) = items[offset] else {
                    return false;
                };
                let Operand::Forward(target) = jump.operand() else {
                    return false;
                };
                if offset + 1 != items.len()
                    || items[..offset]
                        .iter()
                        .any(|item| !matches!(item, Item::Label(_)))
                    || !items[..offset].iter().any(|item| {
                        matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.block_boundaries.contains(label))
                    })
                    || jump.location.line < 0
                    || !jump.jump_policy.retains_nop_for(jump.operand())
                    || jump.jump_policy.always_retains_nop()
                    || jump.local_policy.hidden_exception_owner_slots() != 0
                {
                    return false;
                }
                let Some(target_block) = graph.label_block(target) else {
                    return false;
                };
                let Some((Item::Instruction(tail), head)) =
                    graph[target_block].items().split_last()
                else {
                    return false;
                };
                let mut tail_policy = super::super::ir::JumpPolicy::default();
                tail_policy.mark_borrow_empty_block();
                !head.is_empty()
                    && head.iter().all(|item| matches!(item, Item::Label(_)))
                    && head.iter().any(|item| {
                        matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.source_boundaries.contains(label))
                    })
                    && tail.opcode == JUMP_FORWARD
                    && matches!(tail.operand(), Operand::Forward(_))
                    && tail.location == SourceLocation::NONE
                    && tail.jump_policy == tail_policy
                    && tail.no_location_policy == super::super::ir::NoLocationPolicy::default()
                    && tail.exception_policy() == super::super::ExceptionPolicy::default()
                    && tail.pre_fold_extra_instructions() == 0
                    && tail.local_policy.hidden_exception_owner_slots() == 0
                    && !tail.fusion_policy.is_authored_break()
                    && !tail.fusion_policy.is_converted_pop_block()
            })
            .collect::<FxHashSet<_>>();
        // A compound try still has the inner try/except POP_BLOCK before its jump.
        // Validate this owner-only origin separately from ordinary-try copy eligibility.
        let compound_try_owner_exits = self
            .exception_owner_provenance
            .compound_try_owner_exits
            .iter()
            .filter_map(|(entry, target)| {
                let entry_index = *label_offsets.get(entry)?;
                let target_index = *label_offsets.get(target)?;
                let (block, offset) = *instruction_positions.get(entry_index)?;
                let Item::Instruction(instruction) = graph[block].items()[offset] else {
                    return None;
                };
                (entry_index < target_index
                    && instruction.opcode == JUMP_FORWARD
                    && instruction.operand() == Operand::Forward(*target)
                    && instruction.pre_fold_extra_instructions() == 0)
                    .then_some(entry_index)
            })
            .collect::<FxHashSet<_>>();
        let mut jump_targets =
            vec![None; usize::try_from(self.next_label).expect("label count fits in usize")];
        let mut jump_target_count = 0;
        let mut next_instruction = None;
        for block in graph.order().iter().rev().copied() {
            for item in graph[block].items().iter().rev() {
                match item {
                    Item::Instruction(instruction) => next_instruction = Some(*instruction),
                    Item::Label(label) => {
                        let Some(instruction) = next_instruction else {
                            continue;
                        };
                        // A jump retained as a source-position NOP remains a CFG boundary.
                        if instruction.jump_policy.allows_threading_target()
                            && !instruction.jump_policy.always_retains_nop()
                            // This alias enters an authored Pass before the inner try/except's
                            // POP_BLOCK/JUMP pair. Its later try-end alias still enters the
                            // pair itself; only that alias can copy the cleanup's owner.
                            && !((normal_try_exits.contains(&label_offsets[label])
                                || compound_try_owner_exits.contains(&label_offsets[label]))
                                && self.original_nop_targets.contains(label)
                                && self.exception_owner_provenance.omitted_nop_counts.get(label) == Some(&1)
                                && self.exception_owner_provenance.omitted_nop_locations.get(label)
                                    .is_some_and(|location| location.line >= 0))
                            && is_unconditional_jump(instruction.opcode)
                            && let operand @ (Operand::Forward(_) | Operand::Backward(_)) =
                                instruction.operand()
                        {
                            // A retained unlocated jump keeps its synthetic exclusion at the
                            // join unless original try-exit provenance proves a POP_BLOCK.
                            // Borrowed-load reachability does not establish that provenance.
                            let pop_block_owner = (pop_block_offsets
                                .contains(&label_offsets[label])
                                && !(instruction.jump_policy.preserves_nop_after_threading()
                                    && instruction.location.line < 0
                                    && !normal_try_exits.contains(&label_offsets[label])
                                    && (!compound_try_owner_exits
                                        .contains(&label_offsets[label])
                                        || self.original_nop_targets.contains(label))))
                            .then_some(label_offsets[label]);
                            let target = &mut jump_targets
                                [usize::try_from(label.0).expect("label index fits in usize")];
                            jump_target_count += usize::from(
                                target
                                    .replace((
                                        operand,
                                        instruction.opcode,
                                        instruction.no_location_policy.is_artificial_eval_break(),
                                        pop_block_owner,
                                        instruction.jump_policy.borrow_empty_block(),
                                        instruction.no_location_policy.borrow_fallthrough()
                                            == BorrowFallthrough::Barrier
                                            || !instruction
                                                .jump_policy
                                                .allows_borrow_threading_target(),
                                        instruction.jump_policy.is_interruptible(),
                                        instruction.jump_policy.preserves_nop_after_threading(),
                                        normal_try_exits.contains(&label_offsets[label])
                                            && !self.original_nop_targets.contains(label),
                                    ))
                                    .is_none(),
                            );
                        }
                    }
                }
            }
        }

        // Only protect NOP predecessors when the ordinary head survives this early
        // copying step. Removing one NOP alias can preserve another head that targets
        // it, so propagate those dependencies until the target map is stable.
        let original_head_blocks = self
            .original_jump_head_targets
            .iter()
            .filter_map(|label| graph.label_block(*label))
            .collect::<FxHashSet<_>>();
        let mut nop_targets = FxHashMap::<_, Vec<_>>::default();
        let mut dependent_heads = FxHashMap::<_, Vec<_>>::default();
        let mut surviving_heads = Vec::new();
        for (block, contents) in graph.iter() {
            if !original_head_blocks.contains(&block) {
                continue;
            }
            for item in contents.items() {
                if let Item::Label(label) = item
                    && self.original_nop_targets.contains(label)
                {
                    nop_targets.entry(block).or_default().push(*label);
                }
            }
            let Some(instruction) = contents.items().iter().find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            }) else {
                continue;
            };
            let Operand::Forward(target) = instruction.operand() else {
                continue;
            };
            if jump_targets[usize::try_from(target.0).expect("label index fits in usize")].is_none()
            {
                surviving_heads.push(block);
            } else {
                dependent_heads.entry(target).or_default().push(block);
            }
        }
        while let Some(block) = surviving_heads.pop() {
            if let Some(labels) = nop_targets.remove(&block) {
                for label in labels {
                    if jump_targets[usize::try_from(label.0).expect("label index fits in usize")]
                        .take()
                        .is_some()
                        && let Some(dependents) = dependent_heads.remove(&label)
                    {
                        surviving_heads.extend(dependents);
                    }
                }
            }
        }

        let authored_finally_break_targets = Self::take_authored_finally_break_threading_targets(
            &self.graph,
            &mut self.borrow_authored_finally_break_origins,
        );
        let graph = &mut self.graph;
        let mut instruction_index = 0;
        let mut copied_owners = Vec::new();
        let mut threaded_borrow_origins = FxHashSet::default();
        let mut threaded_authored_finally_breaks = FxHashSet::default();
        graph.for_each_instruction_mut(|instruction| {
            let source_index = instruction_index;
            instruction_index += 1;
            // Cold ordering settles direction later; a backward-form opcode can still
            // have a forward target here, including an original handler break.
            if planned_sources.contains(&source_index)
                || !is_unconditional_jump(instruction.opcode)
                || instruction.jump_policy.always_retains_nop()
            {
                return;
            }
            let Operand::Forward(mut target) = instruction.operand() else {
                return;
            };
            let original_target = target;
            let authored_break_copy_nop = instruction.fusion_policy.is_authored_break()
                && instruction.opcode == JUMP_FORWARD
                && instruction
                    .jump_policy
                    .retains_nop_for(instruction.operand());
            let original_borrow_route = instruction
                .initial_borrow_jump_target()
                .and_then(|target| self.borrow_threaded_jump_targets.get(&target).copied());
            let folded_condition_exit = self.folded_condition_exit_targets.contains(&target);
            let mut operand = instruction.operand();
            let mut opcode = instruction.opcode;
            let mut artificial_eval_break =
                instruction.no_location_policy.is_artificial_eval_break();
            let mut copied_owner = None;
            let mut crosses_empty_block = false;
            let mut crosses_fallthrough_barrier = false;
            let mut interruptible = instruction.jump_policy.is_interruptible();
            let backward_form = instruction.opcode != JUMP_FORWARD;
            let mut backward_form_interruptible =
                backward_form && (instruction.opcode == JUMP_BACKWARD || interruptible);
            let mut threaded_jump_count = 0_u8;
            let mut crosses_normal_try_exit = false;
            for _ in 0..jump_target_count {
                let Some((
                    next_operand,
                    next_opcode,
                    next_artificial_eval_break,
                    owner,
                    next_empty_block,
                    next_fallthrough_barrier,
                    next_interruptible,
                    next_retains_nop,
                    next_normal_try_exit,
                )) = jump_targets
                    .get(usize::try_from(target.0).expect("label index fits in usize"))
                    .copied()
                    .flatten()
                else {
                    break;
                };
                if folded_condition_exit && next_retains_nop {
                    // The folded condition's synthetic edge must not erase a retained exit.
                    // Ordinary finally exits also carry borrow-empty markers and still thread.
                    // Let block copying preserve the folded edge's target and its NOP policy.
                    break;
                }
                let next = match next_operand {
                    Operand::Forward(next) | Operand::Backward(next) => next,
                    Operand::Value(_) | Operand::Deferred(_) => unreachable!(),
                };
                if next == target {
                    break;
                }
                operand = next_operand;
                threaded_jump_count = threaded_jump_count.saturating_add(1);
                crosses_normal_try_exit |= next_normal_try_exit;
                crosses_empty_block |= next_empty_block;
                crosses_fallthrough_barrier |= next_fallthrough_barrier;
                interruptible |= next_interruptible;
                if backward_form {
                    // CPython's JUMP kind is sticky through JUMP_NO_INTERRUPT targets.
                    backward_form_interruptible |=
                        next_opcode == JUMP_BACKWARD || next_interruptible;
                    if backward_form_interruptible {
                        opcode = JUMP_BACKWARD;
                    }
                }
                if owner.is_some() {
                    copied_owner = owner;
                }
                if next_opcode == JUMP_BACKWARD {
                    opcode = JUMP_BACKWARD;
                    artificial_eval_break |= next_artificial_eval_break;
                } else if next_opcode == JUMP_BACKWARD_NO_INTERRUPT && !backward_form_interruptible
                {
                    opcode = JUMP_BACKWARD_NO_INTERRUPT;
                }
                target = next;
            }
            changed |= target != original_target;
            // CPython inlines these unlocated jump blocks before threading. Their replaced
            // jumps remain NOP slots until after the repeated small-exit copy pass.
            instruction.add_pre_fold_extra_instructions(threaded_jump_count);
            let retargeted = operand != instruction.operand();
            instruction.retarget(operand);
            if target != original_target && original_borrow_route == Some(target) {
                // Hidden copied slots cannot share the compact initial-origin tag. Keep
                // this authorized instruction in a separate, strictly checked snapshot.
                threaded_borrow_origins.insert(source_index);
            }
            if target != original_target
                && authored_finally_break_targets.get(&source_index) == Some(&target)
            {
                threaded_authored_finally_breaks.insert(source_index);
            }
            instruction.opcode = opcode;
            if (interruptible || backward_form_interruptible) && (!backward_form || retargeted) {
                // Threading through JUMP preserves its eval-breaker check, including when
                // another synthetic cleanup edge supplies the final target.
                instruction.jump_policy.mark_interruptible();
            }
            if authored_break_copy_nop
                && threaded_jump_count == 1
                && crosses_normal_try_exit
                && matches!(operand, Operand::Forward(_))
                && !instruction.jump_policy.is_interruptible()
            {
                // A break's direct copy request survives the original POP_BLOCK/JUMP
                // exit. The small-exit copier still decides whether the target fits,
                // and gives the copied NOP this break's own location and exit owner.
                instruction.jump_policy.retain_nop_for_direct_target();
            }
            if transparent_normal_try_tails.contains(&source_index)
                && threaded_jump_count == 1
                && matches!(operand, Operand::Forward(_))
                && !instruction.jump_policy.is_interruptible()
            {
                instruction.jump_policy.retain_nop_for_direct_target();
            }
            if let Some(target_index) = copied_owner
                && region_ranges
                    .iter()
                    .any(|range| range.contains(&source_index) != range.contains(&target_index))
            {
                copied_owners.push((source_index, target_index));
            }
            if artificial_eval_break
                && instruction.opcode == JUMP_FORWARD
                && threaded_jump_count == 0
            {
                // An untouched ordinary head still threads through the original-head path.
                // Only replacement by an early copy leaves the hidden NOP prefix that needs
                // separate predecessor-location clones during the later exit-copy stage.
                instruction.no_location_policy.clear_artificial_eval_break();
            } else if artificial_eval_break {
                instruction.no_location_policy.mark_artificial_eval_break();
            }
            if crosses_empty_block
                || (crosses_fallthrough_barrier
                    && original_borrow_route != Some(target)
                    && authored_finally_break_targets.get(&source_index) != Some(&target))
            {
                // Both captured ordinary jumps and authored finally breaks can prove an
                // independent entry. A fallthrough barrier must not revoke those routes.
                // Threading bypasses an original empty block. If this jump is later
                // removed, borrowed-load traversal still stops at that block; a surviving
                // jump continues to reach its explicit target.
                instruction
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier);
            }
        });
        if !copied_owners.is_empty() {
            // A synthetic normal-exit block begins with POP_BLOCK in CPython. Inlining
            // replaces the source jump with the target jump, including its exception owner.
            // Preserve that ownership when our earlier threading removes the same block.
            let mut exclusions = vec![Vec::new(); self.exception_regions.len()];
            let mut copied_regions = Vec::new();
            for &(source, target) in copied_owners.iter().rev() {
                let start = Label(self.next_label);
                let end = Label(self.next_label + 1);
                self.next_label += 2;
                let (block, offset) = instruction_positions[source];
                graph.insert_label(block, offset + 1, end);
                graph.insert_label(block, offset, start);
                for (index, (region, range)) in self
                    .exception_regions
                    .iter()
                    .zip(&region_ranges)
                    .enumerate()
                {
                    match (range.contains(&source), range.contains(&target)) {
                        (true, false) => exclusions[index].push((start, end)),
                        (false, true) => copied_regions.push(super::super::ExceptionRegion {
                            start,
                            end,
                            ..*region
                        }),
                        _ => {}
                    }
                }
            }
            let original_regions = std::mem::take(&mut self.exception_regions);
            for (region, exclusions) in original_regions.into_iter().zip(exclusions) {
                let mut start = region.start;
                for (exclusion_start, exclusion_end) in exclusions.into_iter().rev() {
                    self.exception_regions.push(super::super::ExceptionRegion {
                        start,
                        end: exclusion_start,
                        ..region
                    });
                    start = exclusion_end;
                }
                self.exception_regions
                    .push(super::super::ExceptionRegion { start, ..region });
            }
            self.exception_regions.extend(copied_regions);
        }
        self.borrow_threaded_jump_entries =
            Self::snapshot_threaded_borrow_entries(&self.graph, &threaded_borrow_origins);
        self.borrow_authored_finally_break_origins.snapshots =
            Self::snapshot_threaded_borrow_entries(&self.graph, &threaded_authored_finally_breaks);
        changed
    }

    /// Keeps a manager POP unlocated when its original upstream donor is not unique.
    /// Jump-only arrays propagate this absence even when conditional entries prevent copying.
    pub(in crate::assembler) fn preserve_shared_original_manager_pop_locations(&mut self) {
        if self
            .exception_owner_provenance
            .original_manager_pop_blocks
            .is_empty()
        {
            return;
        }
        let graph = &self.graph;
        let order = graph.order();
        // CPython discards unreachable arrays before resolving line numbers.
        let reachable = graph.reachable_blocks(&self.exception_regions);
        let positions = order
            .iter()
            .copied()
            .enumerate()
            .map(|(position, block)| (block, position))
            .collect::<FxHashMap<_, _>>();
        let mut incoming = FxHashMap::<_, Vec<_>>::default();
        for adjacent in order.windows(2) {
            if reachable[adjacent[0].index()]
                && super::super::block_has_fallthrough(graph[adjacent[0]].items())
            {
                incoming.entry(adjacent[1]).or_default().push(adjacent[0]);
            }
        }
        for source in order
            .iter()
            .copied()
            .filter(|source| reachable[source.index()])
        {
            for item in graph[source].items() {
                if let Item::Instruction(instruction) = item
                    && let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                    && let Some(target) = graph.label_block(label)
                {
                    incoming.entry(target).or_default().push(source);
                }
            }
        }
        // A shared unlocated jump-only array cannot acquire a line, so it also
        // cannot donate one to its successor. This remains true if conditional
        // entries prevent copying that array. Count upstream donor paths, rather
        // than claiming these are the successor's post-copy predecessor counts.
        // Forward-only expansion permits one source-order pass, saturated at two.
        let mut expanded_incoming = FxHashMap::default();
        for block in order.iter().copied() {
            let mut leaves = 0_usize;
            for source in incoming.get(&block).into_iter().flatten().copied() {
                let mut instructions = graph[source].items().iter().filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                });
                let jump_only = instructions.next().is_some_and(|jump| {
                    jump.opcode == JUMP_FORWARD
                        && jump.location.line < 0
                        && instructions.next().is_none()
                        && positions[&source] < positions[&block]
                });
                let count = if jump_only {
                    expanded_incoming[&source]
                } else {
                    1
                };
                leaves = (leaves + count).min(2);
            }
            expanded_incoming.insert(block, leaves);
        }
        let mut clear = Vec::new();
        for entry in &self.exception_owner_provenance.original_manager_pop_blocks {
            let Some(target) = graph.label_block(*entry) else {
                continue;
            };
            if expanded_incoming[&target] > 1
                && let Some((offset, Item::Instruction(first))) = graph[target]
                    .items()
                    .iter()
                    .enumerate()
                    .find(|(_, item)| matches!(item, Item::Instruction(_)))
                && first.opcode == NOP
                && first.fusion_policy.is_converted_pop_block()
                && first.location == SourceLocation::NONE
            {
                clear.push(ItemPosition::new(target, offset));
            }
        }
        for position in clear {
            self.graph
                .instruction_mut(position)
                .unwrap()
                .no_location_policy
                .preserve_location();
        }
    }

    /// Preserves original empty jump-head boundaries for the final borrowing traversal.
    pub(in crate::assembler) fn record_borrow_empty_jump_heads(&mut self) {
        let graph = &self.graph;
        let order = graph.order();
        let positions = order
            .iter()
            .copied()
            .enumerate()
            .map(|(index, block)| (block, index))
            .collect::<FxHashMap<_, _>>();
        let mut blocked = self
            .exception_regions
            .iter()
            .filter_map(|region| graph.label_block(region.target))
            .collect::<FxHashSet<_>>();
        blocked.extend(order.first().copied());
        for blocks in order.windows(2) {
            if super::super::block_has_fallthrough(graph[blocks[0]].items()) {
                blocked.insert(blocks[1]);
            }
        }
        let mut incoming = FxHashSet::default();
        let mut conditional_incoming = FxHashMap::<_, usize>::default();
        let mut conditional_blocked = blocked.clone();
        let original_reachable = graph.reachable_blocks(&self.exception_regions);
        for source in order.iter().copied() {
            let last = graph[source]
                .items()
                .iter()
                .rposition(|item| matches!(item, Item::Instruction(_)));
            for (offset, item) in graph[source].items().iter().enumerate() {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                let (Operand::Forward(label) | Operand::Backward(label)) = instruction.operand()
                else {
                    continue;
                };
                let Some(target) = graph.label_block(label) else {
                    continue;
                };
                incoming.insert(target);
                if Some(offset) == last
                    && matches!(
                        instruction.opcode,
                        POP_JUMP_IF_FALSE
                            | POP_JUMP_IF_TRUE
                            | POP_JUMP_IF_NONE
                            | POP_JUMP_IF_NOT_NONE
                    )
                    && matches!(instruction.operand(), Operand::Forward(_))
                    && positions[&source] < positions[&target]
                    && original_reachable[source.index()]
                    && !instruction.jump_policy.always_retains_nop()
                {
                    *conditional_incoming.entry(target).or_default() += 1;
                } else {
                    conditional_blocked.insert(target);
                }
                // Every original entry must take the same unconditional-copy path.
                // Conditional, backward, and retained-NOP entries can keep this block live.
                if Some(offset) != last
                    || instruction.opcode != JUMP_FORWARD
                    || !matches!(instruction.operand(), Operand::Forward(_))
                    || instruction.jump_policy.always_retains_nop()
                    || positions[&source] >= positions[&target]
                {
                    blocked.insert(target);
                }
            }
        }
        let heads = order
            .iter()
            .copied()
            .filter(|block| {
                (incoming.contains(block) && !blocked.contains(block))
                    || (conditional_incoming.get(block).copied().unwrap_or_default() > 1
                        && !conditional_blocked.contains(block))
            })
            .filter_map(|block| {
                let items = graph[block].items();
                let (Item::Instruction(jump), labels) = items.split_last()? else {
                    return None;
                };
                if !labels.iter().all(|item| {
                    matches!(item, Item::Label(label)
                        if !self.original_nop_targets.contains(label)
                            && !self.preserved_block_boundaries.contains(label))
                }) || jump.opcode != JUMP_FORWARD
                    || jump.location.line >= 0
                    || jump.pre_fold_extra_instructions() != 0
                    || !jump.jump_policy.allows_threading_target()
                    || jump.jump_policy.always_retains_nop()
                    || jump.jump_policy.preserves_nop_after_threading()
                    || jump.jump_policy.is_interruptible()
                    || jump.jump_policy.borrow_empty_block()
                {
                    return None;
                }
                let Operand::Forward(label) = jump.operand() else {
                    return None;
                };
                let target = graph.label_block(label)?;
                (positions[&block] < positions[&target])
                    .then_some(ItemPosition::new(block, items.len() - 1))
            })
            .collect::<Vec<_>>();
        for head in heads {
            // Threading transfers this provenance to each predecessor jump. If that
            // jump disappears later, its barrier remains at the original fallthrough
            // site even when the emptied target's labels move with cold handlers.
            // Shared conditional entries keep their original head unlocated. Removing
            // that head's jump must also retain its empty borrowing boundary.
            self.graph.mark_borrow_empty_block(head);
        }
    }

    /// Finds intact original constant/Pass arrays without crossing a real entry.
    fn constant_pass_cluster_positions(&self) -> Vec<(Label, [ItemPosition; 3])> {
        if self
            .exception_owner_provenance
            .constant_pass_clusters
            .is_empty()
        {
            return Vec::new();
        }
        let items = self
            .graph
            .order()
            .iter()
            .copied()
            .flat_map(|block| {
                self.graph[block]
                    .items()
                    .iter()
                    .enumerate()
                    .map(move |(offset, item)| (ItemPosition::new(block, offset), *item))
            })
            .collect::<Vec<_>>();
        let positions = items
            .iter()
            .enumerate()
            .filter_map(|(index, (_, item))| {
                if let Item::Label(label) = item {
                    Some((*label, index))
                } else {
                    None
                }
            })
            .collect::<FxHashMap<_, _>>();
        let mut entries = self.exception_owner_provenance.block_boundaries.clone();
        entries.extend(&self.preserved_block_boundaries);
        entries.extend(self.exception_regions.iter().map(|region| region.target));
        entries.extend(
            self.exception_owner_provenance
                .setups
                .iter()
                .map(|(label, _)| *label),
        );
        entries.extend(items.iter().filter_map(|(_, item)| {
            let Item::Instruction(instruction) = item else {
                return None;
            };
            match instruction.operand() {
                Operand::Forward(label) | Operand::Backward(label) => Some(label),
                Operand::Value(_) | Operand::Deferred(_) => None,
            }
        }));
        let plain_nop = |instruction: &Instruction, location: SourceLocation| {
            instruction.opcode == NOP
                && instruction.operand() == Operand::Value(0)
                && instruction.location == location
                && !instruction.fusion_policy.is_converted_pop_block()
                && !instruction.jump_policy.preserves_explicit_nop()
                && !instruction.jump_policy.borrow_empty_block()
                && instruction.pre_fold_extra_instructions() == 0
                && instruction.local_policy.hidden_exception_owner_slots() == 0
        };
        let mut valid = Vec::new();
        for cluster in &self.exception_owner_provenance.constant_pass_clusters {
            if entries.contains(&cluster.start) || entries.contains(&cluster.end) {
                continue;
            }
            let Some(&index) = positions.get(&cluster.start) else {
                continue;
            };
            let Some(start) = index.checked_sub(1) else {
                continue;
            };
            let Some(window) = items.get(start..index + 4) else {
                continue;
            };
            let [
                (previous_position, Item::Instruction(previous)),
                (_, Item::Label(first)),
                (constant_position, Item::Instruction(constant)),
                (_, Item::Label(second)),
                (pass_position, Item::Instruction(pass)),
            ] = window
            else {
                continue;
            };
            if *first == cluster.start
                && *second == cluster.end
                && previous.opcode != NOP
                && previous.opcode != NOT_TAKEN
                && matches!(previous.operand(), Operand::Value(_))
                && !ends_scope(previous.opcode)
                && plain_nop(constant, cluster.constant_location)
                && plain_nop(pass, cluster.pass_location)
            {
                valid.push((
                    cluster.start,
                    [*previous_position, *constant_position, *pass_position],
                ));
            }
        }
        valid
    }

    /// Records physical original JUMP heads before early copying can replace them with NOPs.
    pub(in crate::assembler) fn record_original_jump_heads(&mut self) {
        // Reject original incoming entries before threading can erase their edges.
        // Copied labels never inherit this original-array certificate.
        let constant_pass_starts = self
            .constant_pass_cluster_positions()
            .into_iter()
            .map(|(start, _)| start)
            .collect::<FxHashSet<_>>();
        self.exception_owner_provenance
            .constant_pass_clusters
            .retain(|cluster| constant_pass_starts.contains(&cluster.start));
        let conditional_targets = self
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction)
                    if matches!(
                        instruction.opcode,
                        POP_JUMP_IF_FALSE
                            | POP_JUMP_IF_TRUE
                            | POP_JUMP_IF_NONE
                            | POP_JUMP_IF_NOT_NONE
                    ) =>
                {
                    match instruction.operand() {
                        Operand::Forward(target) => Some(target),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect::<FxHashSet<_>>();
        if conditional_targets.is_empty() {
            return;
        }
        for (_, block) in self.graph.iter() {
            let mut labels = Vec::new();
            for item in block.items() {
                match item {
                    Item::Label(label) => labels.push(*label),
                    Item::Instruction(instruction) => {
                        // Only the compiler's common loop backedge can certify a
                        // backward head. Per-arm and copied backedges have no such entry.
                        if (instruction.opcode == JUMP_BACKWARD
                            || (instruction.opcode == JUMP_FORWARD
                                && instruction.jump_policy.is_interruptible()))
                            && instruction.pre_fold_extra_instructions() == 0
                        {
                            let original_cleanup_copy = match instruction.operand() {
                                Operand::Forward(target) => {
                                    self.original_unlocated_cleanup_targets.contains(&target)
                                }
                                _ => false,
                            };
                            if !original_cleanup_copy {
                                // An explicitly tracked exit can share its block with other
                                // conditional targets. Preserve those aliases independently.
                                self.original_jump_head_targets
                                    .extend(labels.into_iter().filter(|label| {
                                        conditional_targets.contains(label)
                                            && if instruction.opcode == JUMP_BACKWARD {
                                                self.conditional_threading_targets.get(label)
                                                    == Some(&instruction.location)
                                            } else {
                                                !self
                                                    .conditional_threading_targets
                                                    .contains_key(label)
                                            }
                                            && !self.original_nop_targets.contains(label)
                                    }));
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Preserves separate branch locations when a match joins another branch exit.
    pub(in crate::assembler) fn prepare_original_conditional_jump_targets(&mut self) {
        if self.conditional_threading_targets.is_empty() {
            return;
        }
        let order = self.graph.order().to_vec();
        let splits = self
            .conditional_threading_targets
            .iter()
            .filter_map(|(label, location)| {
                let block = self.graph.label_block(*label)?;
                let jump = self.graph[block].last_instruction()?;
                let Operand::Forward(target) = jump.operand() else {
                    return None;
                };
                let target = self.graph.label_block(target)?;
                let first = self.graph[target]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })?;
                if !is_unconditional_jump(first.opcode) {
                    return None;
                }
                let position = order.iter().position(|candidate| *candidate == block)?;
                let previous = *order.get(position.checked_sub(1)?)?;
                if !super::super::block_has_fallthrough(self.graph[previous].items()) {
                    return None;
                }
                let last = self.graph[previous].last_instruction()?;
                Some((
                    *label,
                    block,
                    previous,
                    jump.operand(),
                    *location,
                    last.location,
                    last.depth_after(),
                ))
            })
            .collect::<Vec<_>>();
        for (label, block, previous, operand, condition_location, body_location, depth) in splits {
            self.graph.append_to_block(
                previous,
                vec![Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    operand,
                    body_location,
                    depth,
                ))],
            );
            if let Some(jump) = self.graph.instructions_mut(block).next_back() {
                jump.location = condition_location;
                jump.jump_policy.prevent_threading_target();
            }
            self.conditional_threading_targets.remove(&label);
        }
    }

    /// Threads conditionals only through original ordinary jumps that survived exit copying.
    pub(in crate::assembler) fn thread_original_conditional_jump_targets(&mut self) -> bool {
        if self.conditional_threading_targets.is_empty()
            && self.original_jump_head_targets.is_empty()
        {
            return false;
        }
        let targets = self
            .conditional_threading_targets
            .keys()
            .chain(self.original_jump_head_targets.iter())
            .filter_map(|label| {
                let block = self.graph.label_block(*label)?;
                let first = self.graph[block]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })?;
                ((first.opcode == JUMP_FORWARD
                    || (first.opcode == JUMP_BACKWARD
                        && self.original_jump_head_targets.contains(label)
                        && self.conditional_threading_targets.get(label).is_some_and(
                            |location| {
                                // An original unlocated head can acquire its sole donor
                                // after other incoming edges receive their own copies.
                                *location == SourceLocation::NONE || *location == first.location
                            },
                        )))
                    && (!self.original_jump_head_targets.contains(label)
                        || first.pre_fold_extra_instructions() == 0))
                    .then_some((*label, first.operand()))
            })
            .collect::<FxHashMap<_, _>>();
        let mut changed = false;
        let mut threaded_condition = None;
        let origins = &mut self.exception_owner_provenance.threaded_condition_origins;
        let paired_none_prefixes = &mut self.exception_owner_provenance.copied_return_none_prefixes;
        self.graph.for_each_instruction_mut(|instruction| {
            if let Some((condition, retargeted)) = threaded_condition.take()
                && instruction.opcode == NOT_TAKEN
            {
                // The marker can still reuse an older owner after earlier folds compact the block.
                instruction.set_exception_owner(NormalizedExceptionOwner::DynamicConditionStrict);
                if retargeted {
                    origins.push((condition, *instruction));
                }
            }
            if matches!(
                instruction.opcode,
                POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
            ) && let Operand::Forward(target) = instruction.operand()
                && let Some(operand) = targets.get(&target)
            {
                let original_condition = *instruction;
                instruction.retarget(*operand);
                // CPython replaces the old jump with a NOP and appends a fresh, ownerless jump.
                instruction.exclude_from_exception_table(ExceptionExclusion::Always);
                for origin in &mut *paired_none_prefixes {
                    if origin.condition == original_condition {
                        origin.condition = *instruction;
                    }
                }
                threaded_condition = Some((*instruction, *operand != Operand::Forward(target)));
                changed = true;
            }
        });
        changed
    }

    /// Captures shared original jump arrays before threading or copying can merge their entries.
    pub(in crate::assembler) fn record_shared_unlocated_jump_entries(&mut self) {
        let reachable = self.graph.reachable_blocks(&self.exception_regions);
        let mut predecessors = FxHashMap::<Label, usize>::default();
        for block in self.graph.order().iter().copied() {
            if !reachable[block.index()] {
                continue;
            }
            for item in self.graph[block].items() {
                if let Item::Instruction(instruction) = item
                    && matches!(
                        instruction.opcode,
                        POP_JUMP_IF_FALSE
                            | POP_JUMP_IF_TRUE
                            | POP_JUMP_IF_NONE
                            | POP_JUMP_IF_NOT_NONE
                    )
                    && let Operand::Forward(target) = instruction.operand()
                {
                    *predecessors.entry(target).or_default() += 1;
                }
            }
        }
        for block in self.graph.order().iter().copied() {
            let Some((Item::Instruction(jump), head)) = self.graph[block].items().split_last()
            else {
                continue;
            };
            if jump.opcode != JUMP_FORWARD
                || jump.location != SourceLocation::NONE
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
                || jump.jump_policy.always_retains_nop()
                || jump.jump_policy.is_interruptible()
                || head.is_empty()
                || head.iter().any(|item| match item {
                    Item::Instruction(_) => true,
                    Item::Label(label) => {
                        self.original_nop_targets.contains(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_counts
                                .contains_key(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_locations
                                .contains_key(label)
                    }
                })
            {
                continue;
            }
            // Consecutive original labels denote the same instruction offset in CPython.
            // Retain that identity before optimization can merge distinct jump arrays.
            let conditional_entries = head
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => {
                        Some((*label, predecessors.get(label).copied().unwrap_or(0)))
                    }
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            if conditional_entries
                .iter()
                .map(|(_, count)| count)
                .sum::<usize>()
                < 2
            {
                continue;
            }
            let Some((origin, _)) = conditional_entries.first().copied() else {
                continue;
            };
            for (label, count) in conditional_entries {
                if count != 0 {
                    self.original_shared_unlocated_jump_entries
                        .insert(label, origin);
                }
            }
        }
    }

    pub(in crate::assembler) fn remove_redundant_forward_jumps(
        &mut self,
        finalize_borrow_empty_blocks: bool,
    ) {
        // A removed original POP_BLOCK can leave its entry and end labels in
        // separate empty graph blocks. Bind them to the next physical record;
        // renamed labels from copied cleanup arrays cannot certify this jump.
        let mut wildcard_fallthrough_exits = Vec::new();
        if finalize_borrow_empty_blocks
            && !self
                .exception_owner_provenance
                .normal_try_wildcard_fallthroughs
                .is_empty()
        {
            let mut next_instructions = FxHashMap::default();
            let mut pending_labels = Vec::new();
            for &block in self.graph.order() {
                for (offset, item) in self.graph[block].items().iter().enumerate() {
                    match item {
                        Item::Label(label) => pending_labels.push(*label),
                        Item::Instruction(_) => {
                            let position = ItemPosition::new(block, offset);
                            for label in pending_labels.drain(..) {
                                next_instructions.insert(label, position);
                            }
                        }
                    }
                }
            }
            for &(entry, target) in &self.exception_owner_provenance.normal_try_exits {
                let Some((_, pop)) = self
                    .exception_owner_provenance
                    .normal_try_wildcard_fallthroughs
                    .get(&entry)
                else {
                    continue;
                };
                let Some(&position) = next_instructions.get(&entry) else {
                    continue;
                };
                let retired = &self
                    .exception_owner_provenance
                    .retired_normal_try_copy_labels;
                if !retired.contains(&entry)
                    && self
                        .exception_owner_provenance
                        .normal_try_copy_pop_ranges
                        .iter()
                        .any(|(start, end)| {
                            !retired.contains(start)
                                && !retired.contains(end)
                                && next_instructions.get(start) == Some(&position)
                                && next_instructions.get(end) == Some(&position)
                        })
                {
                    wildcard_fallthrough_exits.push((position, target, *pop));
                }
            }
        }
        // Count only labels certified to enter the same original instruction array.
        // Two separate arrays must not become shared merely because later passes merge them.
        let mut shared_entries = FxHashMap::<Label, usize>::default();
        for item in self.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && matches!(
                    instruction.opcode,
                    POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
                )
                && let Operand::Forward(target) = instruction.operand()
                && let Some(origin) = self.original_shared_unlocated_jump_entries.get(&target)
            {
                *shared_entries.entry(*origin).or_default() += 1;
            }
        }
        let planned_blocks = if finalize_borrow_empty_blocks {
            FxHashSet::default()
        } else {
            self.exception_owner_provenance
                .normal_try_copy_blocks
                .iter()
                .filter(|label| {
                    !self
                        .exception_owner_provenance
                        .retired_normal_try_copy_labels
                        .contains(*label)
                })
                .filter_map(|label| self.graph.label_block(*label))
                .collect::<FxHashSet<_>>()
        };
        let graph = &mut self.graph;
        if !finalize_borrow_empty_blocks && !graph.features().has_forward_jump() {
            return;
        }
        // Unlocated borrow markers always disappear after local analysis. Keep the
        // barrier in the graph, but exclude it from physical jump adjacency.
        let is_unlocated_borrow_marker = |instruction: &Instruction| {
            finalize_borrow_empty_blocks
                && instruction.opcode == NOP
                && instruction.jump_policy.borrow_empty_block()
                && instruction.location.line < 0
        };
        let mut target = None;
        let mut has_work = false;
        for item in graph.iter_items() {
            match item {
                Item::Label(label) if target == Some(*label) => {
                    has_work = true;
                    break;
                }
                Item::Label(_) => {}
                Item::Instruction(instruction) if is_unlocated_borrow_marker(instruction) => {}
                Item::Instruction(instruction) => {
                    target = None;
                    if instruction.opcode != JUMP_FORWARD {
                        continue;
                    }
                    let Operand::Forward(jump_target) = instruction.operand() else {
                        continue;
                    };
                    if instruction.jump_policy.defers_redundant_removal() {
                        has_work = true;
                        break;
                    }
                    if !instruction.jump_policy.borrow_empty_block() || finalize_borrow_empty_blocks
                    {
                        target = Some(jump_target);
                    }
                }
            }
        }
        if !has_work {
            return;
        }
        let exception_boundaries = self
            .exception_regions
            .iter()
            .flat_map(|region| [region.start, region.end])
            .collect::<FxHashSet<_>>();
        let order = graph.order().to_vec();
        let mut redundant = FxHashSet::default();
        let mut removed_cross_block_line_events = FxHashSet::default();
        let mut revisit = FxHashSet::default();
        let mut first_scan = true;
        loop {
            let removed_before = redundant.len();
            let mut unlocated_borrow_marker_created = false;
            let mut previous = None;
            for (block_index, block) in order.iter().copied().enumerate() {
                let previous_at_block_entry = previous;
                for offset in 0..graph[block].items().len() {
                    let position = ItemPosition::new(block, offset);
                    let Item::Instruction(mut instruction) = *graph.item(position) else {
                        continue;
                    };
                    if redundant.contains(&position) {
                        continue;
                    }
                    if !first_scan && !revisit.contains(&position) {
                        previous = Some(position);
                        continue;
                    }
                    let Operand::Forward(target) = instruction.operand() else {
                        previous = Some(position);
                        continue;
                    };
                    if instruction.opcode != JUMP_FORWARD {
                        previous = Some(position);
                        continue;
                    }
                    if planned_blocks.contains(&block) {
                        previous = Some(position);
                        continue;
                    }
                    if instruction.jump_policy.defers_redundant_removal() {
                        graph
                            .instruction_mut(position)
                            .unwrap()
                            .jump_policy
                            .resolve_deferred_removal();
                        previous = Some(position);
                        continue;
                    }
                    if instruction.jump_policy.borrow_empty_block() && !finalize_borrow_empty_blocks
                    {
                        previous = Some(position);
                        continue;
                    }

                    let mut target_is_next = false;
                    let mut crosses_exception_boundary = false;
                    let mut crosses_removed_line_event = false;
                    let mut removed_line_events_are_unlocated = true;
                    let mut next = None;
                    'next: for (next_block_index, next_block) in
                        order.iter().copied().enumerate().skip(block_index)
                    {
                        let start = if next_block_index == block_index {
                            offset + 1
                        } else {
                            0
                        };
                        for (next_offset, item) in
                            graph[next_block].items().iter().enumerate().skip(start)
                        {
                            match item {
                                Item::Label(label) => {
                                    target_is_next |= *label == target;
                                    crosses_exception_boundary |=
                                        exception_boundaries.contains(label);
                                }
                                Item::Instruction(removed) => {
                                    if is_unlocated_borrow_marker(removed) {
                                        continue;
                                    }
                                    let position = ItemPosition::new(next_block, next_offset);
                                    if !redundant.contains(&position) {
                                        next = Some(position);
                                        break 'next;
                                    }
                                    if removed_cross_block_line_events.contains(&position) {
                                        crosses_removed_line_event = true;
                                        removed_line_events_are_unlocated &= removed.location.line
                                            < 0
                                            && removed.no_location_policy.preserves_location();
                                    }
                                }
                            }
                        }
                    }
                    if !target_is_next {
                        if finalize_borrow_empty_blocks {
                            revisit.insert(position);
                        }
                        previous = Some(position);
                        continue;
                    }
                    revisit.remove(&position);
                    if !first_scan && crosses_removed_line_event {
                        // An emptied original array can carry a separate line event.
                        // Ordinary jumps wait for that event to be represented.
                        if !instruction.fusion_policy.is_authored_break()
                            || !removed_line_events_are_unlocated
                        {
                            previous = Some(position);
                            continue;
                        }
                        // Preserve the authored break's own line through normal NOP cleanup,
                        // while its erased successor still stops borrowed-load fallthrough.
                        instruction
                            .no_location_policy
                            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                        graph.instruction_mut(position).unwrap().no_location_policy =
                            instruction.no_location_policy;
                    }
                    if !finalize_borrow_empty_blocks
                        && !crosses_exception_boundary
                        && instruction.fusion_policy.is_authored_break()
                        && instruction.jump_policy.inline_small_exit()
                        && targets_small_scope_exit(graph, target)
                    {
                        // CPython copies a small scope exit onto an authored break before removing
                        // its jump. Preserve these edges until that copy, without retaining
                        // jumps to ordinary loop backedges or larger normal-finally bodies.
                        previous = Some(position);
                        continue;
                    }
                    if instruction.jump_policy.borrow_empty_block() {
                        let instruction = graph.instruction_mut(position).unwrap();
                        instruction.opcode = NOP;
                        instruction.set_operand(Operand::Value(0));
                        unlocated_borrow_marker_created |= instruction.location.line < 0;
                        previous = Some(position);
                        continue;
                    }

                    let previous_line = previous.and_then(|previous| {
                        let Item::Instruction(instruction) = *graph.item(previous) else {
                            unreachable!();
                        };
                        // A raising predecessor cannot supply the line for another path's
                        // exit. Keep ordinary jump and copied-return location handling.
                        (!matches!(instruction.opcode, RAISE_VARARGS | RERAISE))
                            .then_some(instruction.location.line)
                    });
                    let next_line = next.map(|next| {
                        let Item::Instruction(instruction) = *graph.item(next) else {
                            unreachable!();
                        };
                        instruction.location.line
                    });
                    // An original POP_BLOCK owns this line event, including the normal
                    // entry to a finally suite. The try jump retires against that same-line
                    // ownerless record even if its collapsed representation retained a NOP.
                    let absorbed_normal_try_exit = finalize_borrow_empty_blocks
                        && instruction.pre_fold_extra_instructions() == 0
                        && !instruction.fusion_policy.is_authored_break()
                        && next.is_some_and(|next| {
                            let Item::Instruction(pop) = graph.item(next) else {
                                return false;
                            };
                            pop.opcode == NOP
                                && pop.fusion_policy.is_converted_pop_block()
                                && matches!(
                                    pop.exception_policy(),
                                    super::super::ExceptionPolicy::AlwaysUnchanged
                                        | super::super::ExceptionPolicy::AlwaysAbsent
                                )
                                && pop.location == instruction.location
                                && self
                                    .exception_owner_provenance
                                    .normal_try_exits
                                    .iter()
                                    .chain(
                                        &self.exception_owner_provenance.compound_try_owner_exits,
                                    )
                                    .any(|(entry, end)| {
                                        *end == target
                                            && graph[block].items()[..offset]
                                                .iter()
                                                .rev()
                                                .take_while(|item| matches!(item, Item::Label(_)))
                                                .any(|item| *item == Item::Label(*entry))
                                    })
                        });
                    // CPython resets the preceding line at the original cleanup array.
                    // Its physical wildcard donor belongs to the preceding array, so the
                    // late redundant jump becomes a NOP even with the same source line.
                    let preserves_wildcard_fallthrough_nop = !absorbed_normal_try_exit
                        && !crosses_exception_boundary
                        && !instruction.fusion_policy.is_authored_break()
                        && instruction.pre_fold_extra_instructions() == 0
                        && previous == previous_at_block_entry
                        && instruction.location.line >= 0
                        && next_line
                            .is_some_and(|line| line >= 0 && line != instruction.location.line)
                        && wildcard_fallthrough_exits.iter().any(
                            |(original, original_target, pop)| {
                                *original == position
                                    && *original_target == target
                                    && instruction.location == pop.location
                                    && previous.is_some_and(|previous| {
                                        graph.item(previous) == &Item::Instruction(*pop)
                                    })
                            },
                        );
                    // A copied finalizer's conditional join remains a separate original
                    // array after its condition threads past the authored break. CPython
                    // resets the preceding line there, so its located jump becomes a NOP
                    // even when the physical fallthrough donor has that same location.
                    let preserves_copied_finally_break_nop = finalize_borrow_empty_blocks
                        && !crosses_exception_boundary
                        && instruction.fusion_policy.is_authored_break()
                        && !instruction.jump_policy.allows_threading_target()
                        && instruction.pre_fold_extra_instructions() == 0
                        && previous == previous_at_block_entry
                        && instruction.location.line >= 0
                        && next_line
                            .is_some_and(|line| line >= 0 && line != instruction.location.line)
                        && previous.is_some_and(|previous| {
                            matches!(graph.item(previous), Item::Instruction(donor)
                                if donor.location == instruction.location
                                    && !ends_scope(donor.opcode)
                                    && !is_unconditional_jump(donor.opcode))
                        })
                        && graph[block].items()[..offset]
                            .iter()
                            .all(|item| matches!(item, Item::Label(_)))
                        && graph[block].items()[..offset].iter().any(|item| {
                            matches!(item, Item::Label(label)
                                if self.conditional_threading_targets.get(label)
                                    == Some(&SourceLocation::NONE)
                                    && self.exception_owner_provenance.source_boundaries.contains(label)
                                    && self.exception_owner_provenance.block_boundaries.contains(label)
                                    && !self.borrow_block_aliases.contains(label)
                                    && !self.original_nop_targets.contains(label))
                        });
                    if !preserves_wildcard_fallthrough_nop
                        && !preserves_copied_finally_break_nop
                        && (!instruction.jump_policy.always_retains_nop()
                            || absorbed_normal_try_exit)
                        && (instruction.location.line < 0
                            || previous_line == Some(instruction.location.line)
                            || next_line == Some(instruction.location.line))
                    {
                        redundant.insert(position);
                        if (instruction.location.line >= 0
                            && previous_line == Some(instruction.location.line)
                            && next_line != Some(instruction.location.line)
                            && previous == previous_at_block_entry)
                            || (instruction.location.line < 0
                                && instruction.no_location_policy.preserves_location()
                                && !(graph[block].items()[..offset].iter().all(|item| matches!(item, Item::Label(_)))
                                    && graph[block].items()[..offset].iter().any(|item| {
                                        matches!(item, Item::Label(label)
                                            if self.original_shared_unlocated_jump_entries.get(label)
                                                .and_then(|origin| shared_entries.get(origin))
                                                .is_some_and(|count| *count >= 2))
                                    })))
                        {
                            removed_cross_block_line_events.insert(position);
                        }
                        if instruction.exception_policy().exclusion()
                            == ExceptionExclusion::IfExtended
                            && let Some(previous) = previous
                        {
                            graph
                                .instruction_mut(previous)
                                .unwrap()
                                .exclude_from_exception_table(ExceptionExclusion::IfExtended);
                        }
                        if instruction.no_location_policy.borrow_fallthrough()
                            == BorrowFallthrough::Barrier
                        {
                            let prefix = &graph[block].items()[..offset];
                            if prefix.iter().all(|item| matches!(item, Item::Label(_)))
                                && previous.is_some_and(|previous| {
                                    matches!(graph.item(previous), Item::Instruction(instruction)
                                        if ends_scope(instruction.opcode))
                                })
                            {
                                // Removing the only instruction leaves this original array
                                // empty. A predecessor from another array cannot carry its
                                // barrier: an explicit branch may bypass that predecessor.
                                self.borrow_unreachable_blocks
                                    .extend(prefix.iter().filter_map(|item| match item {
                                        Item::Label(label) => Some(*label),
                                        Item::Instruction(_) => None,
                                    }));
                                // The empty source array must remain separate from its
                                // successor. A branch targeting that successor bypasses
                                // this barrier even when both labels share a code offset.
                                self.exception_owner_provenance
                                    .borrow_only_block_boundaries
                                    .insert(target);
                            } else if let Some(previous) = previous {
                                graph
                                    .instruction_mut(previous)
                                    .unwrap()
                                    .no_location_policy
                                    .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                            }
                        }
                        if instruction.location.line >= 0
                            && let Some(next) = next
                        {
                            let next = graph.instruction_mut(next).unwrap();
                            if next.location.line < 0
                                && !next.no_location_policy.preserves_location()
                                && !next.no_location_policy.allows_block_inlining()
                            {
                                next.location = instruction.location;
                            }
                        }
                    } else {
                        let instruction = graph.instruction_mut(position).unwrap();
                        instruction.opcode = NOP;
                        instruction.set_operand(Operand::Value(0));
                        previous = Some(position);
                    }
                }
            }
            // CPython repeats jump and NOP removal when an emptied block exposes an
            // earlier jump. Retry only late jumps whose targets were not adjacent:
            // deferred jumps and retained NOPs must keep their one-scan behavior.
            if revisit.is_empty()
                || (redundant.len() == removed_before && !unlocated_borrow_marker_created)
            {
                break;
            }
            first_scan = false;
        }

        // Positions stay stable during retries. Remove in physical reverse order,
        // since a later retry can enqueue a jump before an earlier deletion.
        for block in order.into_iter().rev() {
            for offset in (0..graph[block].items().len()).rev() {
                let position = ItemPosition::new(block, offset);
                if redundant.contains(&position) {
                    graph.remove_item(position);
                }
            }
        }
    }

    pub(in crate::assembler) fn propagate_locations_within_blocks(&mut self) {
        let graph = &mut self.graph;
        let order = graph.order().to_vec();
        for block in order {
            let mut previous = None;
            let mut block_has_instruction = false;
            for item in graph.iter_items_mut(block) {
                match item {
                    ItemMut::Label if block_has_instruction => {
                        previous = None;
                        block_has_instruction = false;
                    }
                    ItemMut::Label => {}
                    ItemMut::Instruction(instruction) => {
                        block_has_instruction = true;
                        if instruction.location.line < 0
                            && !instruction.no_location_policy.preserves_location()
                        {
                            if let Some(location) = previous {
                                instruction.location = location;
                            }
                        } else {
                            previous = Some(instruction.location);
                        }
                        if !matches!(instruction.operand(), Operand::Value(_))
                            || ends_scope(instruction.opcode)
                        {
                            previous = None;
                            block_has_instruction = false;
                        }
                    }
                }
            }
        }
    }

    pub(in crate::assembler) fn remove_redundant_nops(&mut self) {
        self.remove_redundant_nops_scoped(false);
    }

    pub(in crate::assembler) fn remove_redundant_with_exit_nops(&mut self) {
        if !self
            .exception_owner_provenance
            .with_context_locations
            .is_empty()
        {
            self.remove_redundant_nops_scoped(true);
        }
    }

    fn remove_redundant_nops_scoped(&mut self, with_exits_only: bool) {
        if !with_exits_only
            && !self
                .exception_owner_provenance
                .noop_finally_locations
                .is_empty()
        {
            // CPython resolves lines before compacting NOPs: an authored NOP locates the
            // following original POP_BLOCK, then the equal-next-line rule removes the
            // authored record. Preserve the POP_BLOCK record and its exception owner.
            let mut boundaries = self.exception_owner_provenance.block_boundaries.clone();
            boundaries.extend(&self.preserved_block_boundaries);
            boundaries.extend(self.exception_regions.iter().map(|region| region.target));
            boundaries.extend(self.graph.iter_items().filter_map(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(label) | Operand::Backward(label) => Some(label),
                    Operand::Value(_) | Operand::Deferred(_) => None,
                },
                Item::Label(_) => None,
            }));
            let mut previous_nop = None;
            let mut transfers = Vec::new();
            for block in self.graph.order().iter().copied() {
                for (offset, item) in self.graph[block].items().iter().enumerate() {
                    match item {
                        Item::Label(label) if boundaries.contains(label) => previous_nop = None,
                        Item::Label(_) => {}
                        Item::Instruction(instruction) => {
                            if instruction.opcode == NOP
                                && instruction.fusion_policy.is_converted_pop_block()
                                && instruction.location.line < 0
                                && let Some(location) = previous_nop
                            {
                                transfers.push((ItemPosition::new(block, offset), location));
                            }
                            previous_nop = (instruction.opcode == NOP
                                && instruction.location.line >= 0
                                && self
                                    .exception_owner_provenance
                                    .noop_finally_locations
                                    .contains(&instruction.location)
                                && instruction.no_location_policy.borrow_fallthrough()
                                    != BorrowFallthrough::Barrier)
                                .then_some(instruction.location);
                        }
                    }
                }
            }
            for (position, location) in transfers {
                self.graph.instruction_mut(position).unwrap().location = location;
            }
        }
        let graph = &self.graph;
        let track_exception_owners = !self.exception_regions.is_empty()
            && (!self.exception_owner_provenance.synchronous_with_setups.is_empty()
                || !self.exception_owner_provenance.source_boundaries.is_empty()
                || self.exception_owner_provenance.setups.iter().any(|(_, location)| location.is_some())
                || graph.iter_items().any(|item| matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.exception_policy().owner(), super::super::NormalizedExceptionOwner::Dynamic | super::super::NormalizedExceptionOwner::DynamicCondition | super::super::NormalizedExceptionOwner::DynamicConditionStrict))));
        let setup_locations = self
            .exception_owner_provenance
            .setups
            .iter()
            .filter_map(|(label, location)| location.map(|location| (*label, location)))
            .collect::<FxHashMap<_, _>>();
        let track_pop_block_boundaries = track_exception_owners && !setup_locations.is_empty();
        let mut region_ends = FxHashSet::default();
        let mut nonlinear_labels = FxHashSet::default();
        if track_pop_block_boundaries {
            region_ends.extend(self.exception_regions.iter().map(|region| region.end));
            nonlinear_labels.extend(&self.exception_owner_provenance.block_boundaries);
            nonlinear_labels.extend(&self.preserved_block_boundaries);
            nonlinear_labels.extend(self.exception_regions.iter().map(|region| region.target));
            nonlinear_labels.extend(graph.iter_items().filter_map(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(label) | Operand::Backward(label) => Some(label),
                    Operand::Value(_) | Operand::Deferred(_) => None,
                },
                Item::Label(_) => None,
            }));
        }
        let setup_nop_starts = self
            .exception_owner_provenance
            .setup_nop_ranges
            .iter()
            .map(|(start, _)| *start)
            .collect::<FxHashSet<_>>();
        let mut setup_nop_predecessors = FxHashMap::default();
        let mut converted_pop_boundaries = FxHashMap::default();
        let mut discontinuous_instructions = FxHashSet::default();
        let mut preceding_labels = Vec::new();
        let mut setup_nops = FxHashSet::default();
        let mut last_instruction: Option<(ItemPosition, Instruction)> = None;
        let mut history_blocks = FxHashMap::default();
        let mut history_block = 0_usize;
        for block in graph
            .order()
            .iter()
            .copied()
            .filter(|_| track_exception_owners)
        {
            for (offset, item) in graph[block].items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        if track_pop_block_boundaries {
                            preceding_labels.push(*label);
                        }
                        if let Some(location) = setup_locations.get(label)
                            && let Some((position, instruction)) = last_instruction
                            && instruction.opcode == NOP
                            && instruction.location == *location
                        {
                            setup_nops.insert(position);
                        }
                        if self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                        {
                            history_block += 1;
                        }
                    }
                    Item::Instruction(instruction) => {
                        let position = ItemPosition::new(block, offset);
                        if track_pop_block_boundaries {
                            // A SETUP's exclusion label need not start an original array.
                            // Retain the exact fallthrough predecessor across only that label;
                            // independent entries cannot donate their preceding line.
                            if !with_exits_only
                                && instruction.opcode == NOP
                                && let [setup_start] = preceding_labels.as_slice()
                                && setup_nop_starts.contains(setup_start)
                                && !nonlinear_labels.contains(setup_start)
                                && !self
                                    .exception_owner_provenance
                                    .source_boundaries
                                    .contains(setup_start)
                                && !self.original_nop_targets.contains(setup_start)
                                && !self.original_jump_head_targets.contains(setup_start)
                                && let Some((previous_position, previous)) = last_instruction
                                // NEXT_LOCATION is resolved after NOP removal and cannot
                                // supply an authored predecessor line during this pass.
                                && !previous.fusion_policy.location_from_next()
                                && previous.location.line == instruction.location.line
                                && previous.opcode != NOT_TAKEN
                                && !ends_scope(previous.opcode)
                                && !is_unconditional_jump(previous.opcode)
                                && matches!(previous.operand(), Operand::Value(_))
                            {
                                setup_nop_predecessors.insert(position, previous_position);
                            }
                            if preceding_labels
                                .iter()
                                .any(|label| nonlinear_labels.contains(label))
                            {
                                discontinuous_instructions.insert(position);
                            }
                            // A source join starts a fresh native instruction array. Its first
                            // erased POP_BLOCK belongs to that new epoch; the linear case below
                            // separately requires that no join interrupts the transfer.
                            let starts_source_array = preceding_labels.iter().any(|label| {
                                self.exception_owner_provenance
                                    .source_boundaries
                                    .contains(label)
                            }) && preceding_labels.iter().all(|label| {
                                !nonlinear_labels.contains(label)
                                    || self
                                        .exception_owner_provenance
                                        .source_boundaries
                                        .contains(label)
                            });
                            if instruction.opcode == NOP
                                && instruction.fusion_policy.is_converted_pop_block()
                                && (starts_source_array
                                    || (preceding_labels
                                        .iter()
                                        .all(|label| !nonlinear_labels.contains(label))
                                        && last_instruction.is_some_and(|(_, previous)| {
                                            previous.opcode != NOT_TAKEN
                                                && !is_unconditional_jump(previous.opcode)
                                                && !ends_scope(previous.opcode)
                                        })))
                            {
                                let ends: Vec<_> = preceding_labels
                                    .iter()
                                    .copied()
                                    .filter(|label| region_ends.contains(label))
                                    .collect();
                                if !ends.is_empty() {
                                    converted_pop_boundaries.insert(position, ends);
                                }
                            }
                            preceding_labels.clear();
                        }
                        last_instruction = Some((position, *instruction));
                        history_blocks.insert(position, history_block);
                        if instruction.opcode == NOT_TAKEN
                            || is_unconditional_jump(instruction.opcode)
                            || ends_scope(instruction.opcode)
                        {
                            history_block += 1;
                        }
                    }
                }
            }
        }
        let constant_pass_predecessors = if with_exits_only {
            FxHashMap::default()
        } else {
            self.constant_pass_cluster_positions()
                .into_iter()
                .flat_map(|(_, [previous, constant, pass])| {
                    [(constant, previous), (pass, previous)]
                })
                .collect::<FxHashMap<_, _>>()
        };
        let mut copied_jump_nops = FxHashSet::default();
        if !with_exits_only
            && !self
                .exception_owner_provenance
                .normal_try_copy_jump_nop_successors
                .is_empty()
        {
            let mut previous_nop = None;
            for block in graph.order().iter().copied() {
                for (offset, item) in graph[block].items().iter().enumerate() {
                    match item {
                        Item::Instruction(instruction) => {
                            previous_nop = (instruction.opcode == NOP)
                                .then_some(ItemPosition::new(block, offset));
                        }
                        Item::Label(label)
                            if self
                                .exception_owner_provenance
                                .normal_try_copy_jump_nop_successors
                                .contains(label) =>
                        {
                            if let Some(position) = previous_nop {
                                copied_jump_nops.insert(position);
                            }
                        }
                        Item::Label(_) => {}
                    }
                }
            }
        }
        let mut copied_array_components = FxHashMap::default();
        if !with_exits_only
            && !self
                .exception_owner_provenance
                .normal_try_copy_inline_labels
                .is_empty()
        {
            // Physical labels inside a planned copy do not start another original array.
            // Recheck all current entries after copying, threading and cold-block movement.
            let provenance = &self.exception_owner_provenance;
            let mut entries = self.preserved_block_boundaries.clone();
            entries.extend(&provenance.block_boundaries);
            entries.extend(&provenance.source_boundaries);
            entries.extend(&provenance.continuation_boundaries);
            entries.extend(&provenance.fusion_boundaries);
            entries.extend(&provenance.setup_nop_boundaries);
            entries.extend(&provenance.protected_prefix_boundaries);
            entries.extend(&provenance.synchronous_with_setups);
            entries.extend(provenance.setups.iter().map(|(label, _)| *label));
            entries.extend(&self.original_nop_targets);
            entries.extend(&self.original_jump_head_targets);
            entries.extend(self.exception_regions.iter().map(|region| region.target));
            entries.extend(graph.iter_items().filter_map(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(label) | Operand::Backward(label) => Some(label),
                    Operand::Value(_) | Operand::Deferred(_) => None,
                },
                Item::Label(_) => None,
            }));
            let mut component = 0_usize;
            for block in graph.order().iter().copied() {
                for (offset, item) in graph[block].items().iter().enumerate() {
                    match item {
                        Item::Label(label) => {
                            // A scheduled source can have a physical label immediately
                            // before its original jump without a new original CFG entry.
                            // Keep its copied records in the preceding source array.
                            let inside_original_array = !provenance
                                .retired_normal_try_copy_labels
                                .contains(label)
                                && (provenance.normal_try_copy_inline_labels.contains(label)
                                    || provenance.normal_try_copy_internal_labels.contains(label)
                                    || provenance.normal_try_copy_blocks.contains(label));
                            if !inside_original_array || entries.contains(label) {
                                component += 1;
                            }
                        }
                        Item::Instruction(instruction) => {
                            copied_array_components
                                .insert(ItemPosition::new(block, offset), component);
                            if instruction.opcode == NOT_TAKEN
                                || is_unconditional_jump(instruction.opcode)
                                || ends_scope(instruction.opcode)
                                || matches!(
                                    instruction.operand(),
                                    Operand::Forward(_) | Operand::Backward(_)
                                )
                            {
                                component += 1;
                            }
                        }
                    }
                }
            }
        }
        // A manager POP is the first record of its original array. Its line can
        // come from a unique predecessor, but a still-unlocated record disappears
        // during CPython's NOP compaction. A following equal line also removes it.
        let mut original_manager_pops = FxHashSet::default();
        for block in graph.order().iter().copied() {
            let mut original_manager_pop = false;
            for (offset, item) in graph[block].items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        original_manager_pop |= self
                            .exception_owner_provenance
                            .original_manager_pop_blocks
                            .contains(label);
                    }
                    Item::Instruction(instruction) => {
                        if original_manager_pop
                            && instruction.opcode == NOP
                            && instruction.fusion_policy.is_converted_pop_block()
                        {
                            original_manager_pops.insert(ItemPosition::new(block, offset));
                        }
                        original_manager_pop = false;
                    }
                }
            }
        }
        let instructions = graph.order().iter().copied().flat_map(|block| {
            graph[block]
                .items()
                .iter()
                .enumerate()
                .filter_map(move |(offset, item)| {
                    let Item::Instruction(instruction) = item else {
                        return None;
                    };
                    Some((block, ItemPosition::new(block, offset), instruction))
                })
        });
        let mut instructions = instructions.peekable();
        let mut previous = None;
        let mut previous_graph_block = None;
        let mut previous_position = None;
        let mut removed_unpack_line_block = None;
        let mut redundant = Vec::new();
        let mut borrow_empty_nops = Vec::new();
        let mut vacated_slots = Vec::new();
        let mut pending_slots = 0_u16;
        let mut pending_pop_slots = Some(0_u16);
        let mut pending_pop_boundaries: Vec<Label> = Vec::new();
        let mut certified_pop_boundaries = Vec::new();
        let mut certified_pop_widths = Vec::new();
        let mut pending_block = None;
        while let Some((graph_block, position, instruction)) = instructions.next() {
            let block = if track_exception_owners {
                history_blocks[&position]
            } else {
                0
            };
            if pending_block != Some(block) {
                pending_slots = 0;
                pending_pop_slots = Some(0);
                if track_pop_block_boundaries {
                    pending_pop_boundaries.clear();
                }
                pending_block = Some(block);
            }
            if track_pop_block_boundaries && discontinuous_instructions.contains(&position) {
                pending_pop_boundaries.clear();
                pending_pop_slots = None;
            }
            let next = instructions.peek().map(|(_, _, instruction)| *instruction);
            // A canceled rotation can leave both the unpack line and a later capture line.
            // Record removal of the first so the legacy store rule keeps the latter.
            let removes_unpack_line_nop = instruction.opcode == NOP
                && instruction.location.line >= 0
                && !instruction.fusion_policy.is_converted_pop_block()
                && previous.is_some_and(|previous: &Instruction| {
                    previous.opcode == UNPACK_SEQUENCE
                        && previous_graph_block == Some(graph_block)
                        && previous.location.line == instruction.location.line
                })
                && instructions.peek().is_some_and(|(next_block, _, next)| {
                    *next_block == graph_block
                        && next.opcode == NOP
                        && next.location.line >= 0
                        && next.location.line != instruction.location.line
                });
            let remove = instruction.opcode == NOP
                && (!with_exits_only
                    || (instruction.fusion_policy.is_converted_pop_block()
                        && !instruction.jump_policy.borrow_empty_block()
                        && previous.is_some_and(|previous: &Instruction| {
                            previous.opcode == POP_TOP && previous.location == instruction.location
                        })
                        && self
                            .exception_owner_provenance
                            .with_context_locations
                            .contains(&instruction.location)))
                && (!instruction.jump_policy.preserves_explicit_nop()
                    || (original_manager_pops.contains(&position)
                        && (instruction.location.line < 0
                            || next.is_some_and(|next| {
                                next.location.line == instruction.location.line
                            }))))
                && (instruction.location.line < 0
                    || (instruction.fusion_policy.is_converted_pop_block()
                        && previous.is_some_and(|previous: &Instruction| {
                            previous.location.line == instruction.location.line
                                && !(ends_scope(previous.opcode)
                                    || is_unconditional_jump(previous.opcode))
                        }))
                    || (setup_nops.contains(&position)
                        && setup_nop_predecessors.get(&position).is_some_and(|expected| {
                            previous_position == Some(*expected)
                        }))
                    || (constant_pass_predecessors.get(&position).is_some_and(|expected| {
                        previous_position == Some(*expected)
                    }) && previous.is_some_and(|previous: &Instruction| {
                        previous.location.line == instruction.location.line
                    }))
                    || (copied_jump_nops.contains(&position)
                        && (previous_graph_block == Some(graph_block)
                            || previous_position.is_some_and(|previous| {
                                copied_array_components.get(&position).is_some_and(|component| {
                                    copied_array_components.get(&previous) == Some(component)
                                })
                            }))
                        && previous.is_some_and(|previous: &Instruction| {
                            previous.location.line == instruction.location.line
                                && !(ends_scope(previous.opcode)
                                    || is_unconditional_jump(previous.opcode))
                        }))
                    || next.is_some_and(|next| next.location.line == instruction.location.line)
                    || removes_unpack_line_nop
                    // A rewritten rotation is redundant when UNPACK_SEQUENCE already
                    // represents its line in this block, or local stores consume the unpack.
                    || previous.is_some_and(|previous: &Instruction| {
                        previous.opcode == UNPACK_SEQUENCE
                            && ((previous_graph_block == Some(graph_block)
                                && previous.location.line == instruction.location.line
                                && next.is_some_and(|next| next.opcode == SWAP))
                                || (removed_unpack_line_block != Some(graph_block)
                                    && previous.location.line == instruction.location.line
                                    && next.is_some_and(|next| {
                                        matches!(next.opcode, STORE_FAST | STORE_FAST_STORE_FAST)
                                    })))
                    }));
            if remove {
                if removes_unpack_line_nop {
                    removed_unpack_line_block = Some(graph_block);
                }
                pending_pop_slots = pending_pop_slots.and_then(|count| {
                    (instruction.fusion_policy.is_converted_pop_block()
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                        && !setup_nops.contains(&position))
                    .then(|| count.checked_add(1))
                    .flatten()
                });
                if matches!(
                    instruction.no_location_policy.borrow_fallthrough(),
                    BorrowFallthrough::Barrier | BorrowFallthrough::BarrierIfRemoved
                ) && next.is_none_or(|next| !is_unconditional_jump(next.opcode))
                {
                    // A surviving jump threads through the folded block. Without that jump,
                    // erasing a marked NOP must retain its empty traversal boundary.
                    // The existing marker cleanup removes it after borrowed-load analysis.
                    borrow_empty_nops.push(position);
                } else {
                    redundant.push(position);
                }
                if track_exception_owners && !setup_nops.contains(&position) {
                    if track_pop_block_boundaries
                        && let Some(labels) = converted_pop_boundaries.get(&position)
                    {
                        pending_pop_boundaries.extend(labels.iter().copied());
                    }
                    pending_slots = pending_slots
                        .saturating_add(1)
                        .saturating_add(instruction.local_policy.hidden_exception_owner_slots());
                }
            } else {
                if pending_slots > 0 {
                    if track_pop_block_boundaries {
                        // Certify only actual erased POP_BLOCK slots whose hidden count fits on
                        // the surviving successor. A saturated count cannot prove the epoch.
                        if instruction
                            .local_policy
                            .hidden_exception_owner_slots()
                            .saturating_add(pending_slots)
                            <= super::super::LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS
                        {
                            if pending_pop_slots == Some(pending_slots) && pending_slots > 1 {
                                certified_pop_widths.extend(
                                    pending_pop_boundaries
                                        .iter()
                                        .map(|label| (*label, pending_slots)),
                                );
                            }
                            certified_pop_boundaries.append(&mut pending_pop_boundaries);
                        }
                        pending_pop_boundaries.clear();
                    }
                    vacated_slots.push((position, pending_slots));
                    pending_slots = 0;
                }
                pending_pop_slots = Some(0);
                previous = Some(instruction);
                previous_graph_block = Some(graph_block);
                previous_position = Some(position);
                removed_unpack_line_block = None;
            }
        }

        if track_pop_block_boundaries {
            self.exception_owner_provenance
                .removed_pop_block_boundaries
                .extend(certified_pop_boundaries);
            self.exception_owner_provenance
                .removed_pop_block_widths
                .extend(certified_pop_widths);
        }
        let graph = &mut self.graph;
        for position in borrow_empty_nops {
            graph.mark_borrow_empty_block(position);
        }
        for (position, slots) in vacated_slots {
            graph
                .instruction_mut(position)
                .expect("surviving NOP successor")
                .local_policy
                .add_hidden_exception_owner_slots(slots);
        }
        for position in redundant.into_iter().rev() {
            graph.remove_item(position);
        }
    }
}
