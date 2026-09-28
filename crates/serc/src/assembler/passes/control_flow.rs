use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::super::{
    BorrowFallthrough, COMPARE_OP, CONTAINS_OP, COPY, ExceptionExclusion, ExceptionPolicy, IS_OP,
    Instruction, Item, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD,
    LOAD_COMMON_CONSTANT, Label, NOP, NOT_TAKEN, NormalizedExceptionOwner, Operand,
    POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, POP_TOP, STORE_FAST, STORE_FAST_STORE_FAST,
    SourceLocation, TO_BOOL, UNARY_NOT, UNPACK_SEQUENCE, ends_scope, is_unconditional_jump,
};
use crate::assembler::ConstantTable;
use crate::assembler::OptimizationPipeline;
use crate::assembler::block_graph::{BasicBlockId, BlockGraph, ItemMut, ItemPosition};
use crate::code_object::Constant;
use crate::target::Opcode;
use crate::target::opcodes::{
    CALL, DELETE_FAST, END_ASYNC_FOR, LOAD_CONST, LOAD_GLOBAL, POP_EXCEPT, POP_JUMP_IF_NONE,
    POP_JUMP_IF_NOT_NONE, RAISE_VARARGS, RERAISE, RETURN_VALUE, SWAP,
};
use crate::target::operands::ComparisonOperation;

/// The late consumers that can observe a scoped NOP cleanup.
pub(in crate::assembler) struct NopCleanup {
    pub(super) has_forward_jump: bool,
    pub(super) may_have_changed: bool,
}

/// Dense emitted-label offsets with a fallback for independently constructed graphs.
struct ThreadLabelOffsets {
    dense: Vec<usize>,
    sparse: FxHashMap<Label, usize>,
}

impl ThreadLabelOffsets {
    fn new(label_count: u32) -> Self {
        Self {
            dense: vec![
                usize::MAX;
                usize::try_from(label_count).expect("label count fits in usize")
            ],
            sparse: FxHashMap::default(),
        }
    }

    fn insert(&mut self, label: Label, offset: usize) {
        // An instruction position originates in a non-ZST Vec, so it cannot be usize::MAX.
        debug_assert_ne!(offset, usize::MAX);
        if let Some(slot) = self.dense.get_mut(label.index()) {
            *slot = offset;
        } else {
            self.sparse.insert(label, offset);
        }
    }

    fn get(&self, label: Label) -> Option<&usize> {
        match self.dense.get(label.index()) {
            Some(slot) => (*slot != usize::MAX).then_some(slot),
            None => self.sparse.get(&label),
        }
    }
}

impl std::ops::Index<&Label> for ThreadLabelOffsets {
    type Output = usize;

    fn index(&self, label: &Label) -> &Self::Output {
        self.get(*label).expect("no entry found for key")
    }
}

// The original pair is tagged before copying passes; physical copies retire that tag.
// Also reject duplicate tagged pairs so an ambiguous graph cannot certify a donor.
fn unique_module_annotation_pair(
    graph: &BlockGraph,
    donor: Instruction,
    next: Instruction,
) -> bool {
    if !donor.is_original_annotation_jump() || !next.is_original_annotation_jump() {
        return false;
    }
    let mut found = false;
    for &block in graph.order() {
        for pair in graph[block].items().windows(2) {
            if matches!(pair, [Item::Instruction(first), Item::Instruction(second)]
                if *first == donor && *second == next)
            {
                if found {
                    return false;
                }
                found = true;
            }
        }
    }
    found
}

// Bind a converted jump to its original block and its surviving adjacent donor.
// A label between the two is an alternate entry and cannot carry the donor's line.
fn certified_module_annotation_jump_nop(
    graph: &BlockGraph,
    certificates: &[(BasicBlockId, Instruction, Instruction)],
    previous: Option<ItemPosition>,
    position: ItemPosition,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.block() != position.block() || previous.offset() + 1 != position.offset() {
        return false;
    }
    let (Item::Instruction(donor), Item::Instruction(nop)) =
        (graph.item(previous), graph.item(position))
    else {
        return false;
    };
    certificates.contains(&(position.block(), *donor, *nop))
        && unique_module_annotation_pair(graph, *donor, *nop)
}

// Revalidate a marked borrow boundary against its original block before flattening.
// The marker remains a barrier through borrowed-load analysis.
pub(super) fn surviving_module_annotation_borrow_markers(
    graph: &BlockGraph,
    certificates: &[(BasicBlockId, Instruction, Instruction)],
) -> Vec<(Instruction, Instruction)> {
    let mut markers = Vec::new();
    for &(block, donor, mut marker) in certificates {
        if !graph.order().contains(&block)
            || marker.opcode != NOP
            || marker.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Barrier
            || !marker.jump_policy.always_retains_nop()
        {
            continue;
        }
        marker.jump_policy.mark_borrow_empty_block();
        if graph[block].items().windows(2).any(|pair| {
            matches!(pair, [Item::Instruction(first), Item::Instruction(second)]
                if *first == donor && *second == marker)
        }) && unique_module_annotation_pair(graph, donor, marker)
        {
            markers.push((donor, marker));
        }
    }
    markers
}

// After borrowed-load analysis, retire only the surviving uniquely certified marker.
// Recheck adjacency and uniqueness because linearization may have inserted labels.
pub(super) fn retire_module_annotation_borrow_markers(
    items: &mut [Item],
    markers: &[(Instruction, Instruction)],
) {
    for &(donor, marker) in markers {
        if !donor.is_original_annotation_jump() || !marker.is_original_annotation_jump() {
            continue;
        }
        let mut matches = items.windows(2).enumerate().filter_map(|(index, pair)| {
            matches!(pair, [Item::Instruction(first), Item::Instruction(second)]
                if *first == donor && *second == marker)
            .then_some(index + 1)
        });
        let Some(index) = matches.next() else {
            continue;
        };
        if matches.next().is_some() {
            continue;
        }
        let Item::Instruction(instruction) = &mut items[index] else {
            unreachable!("certified marker is an instruction");
        };
        instruction
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved);
    }
}

/// Replaces the earlier of two same-line stores to the same local with a stack pop.
pub(super) fn optimize_redundant_store_pair(first: &mut Instruction, second: &Instruction) -> bool {
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
        true
    } else {
        false
    }
}

/// Folds one adjacent Boolean pair while preserving its original instruction slots.
pub(super) fn optimize_boolean_pair(
    current: &mut Instruction,
    next: &mut Instruction,
    retain_contains: bool,
) {
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
            if retain_contains && opcode == CONTAINS_OP {
                current.mark_retired_contains();
            }
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
    if retain_contains && opcode == CONTAINS_OP {
        current.mark_retired_contains();
    }
    current.opcode = NOP;
    current.set_operand(Operand::Value(0));
    next.opcode = opcode;
    next.set_operand(Operand::Value(argument));
}

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
    let Some(target_block) = graph.label_block(target) else {
        return false;
    };
    let items = graph[target_block].items();
    let Some(target_offset) = items
        .iter()
        .position(|item| matches!(item, Item::Label(label) if *label == target))
    else {
        return false;
    };
    let mut size = 0;
    let mut check = |item: &Item| {
        match item {
            Item::Label(_) if size > 0 => return Some(false),
            Item::Label(_) => {}
            Item::Instruction(instruction) => {
                let fused_push_null = instruction.opcode == LOAD_GLOBAL
                    && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0);
                size += 1
                    + usize::from(fused_push_null)
                    + usize::from(instruction.pre_fold_extra_instructions());
                if size > 4 {
                    return Some(false);
                }
                if ends_scope(instruction.opcode) {
                    // Synthetic fallthrough exits retain their existing no-location copy
                    // policy, including the boundaries around normal-finally bodies.
                    return Some(instruction.location.line >= 0);
                }
                if matches!(
                    instruction.operand(),
                    Operand::Forward(_) | Operand::Backward(_)
                ) {
                    return Some(false);
                }
            }
        }
        None
    };
    if let Some(result) = items[target_offset + 1..].iter().find_map(&mut check) {
        return result;
    }
    // Alias-only blocks and blocks whose terminator was removed can continue in the next
    // physical block. Most targets can be decided before looking up their position in order.
    graph
        .iter()
        .skip_while(|(block, _)| *block != target_block)
        .skip(1)
        .flat_map(|(_, block)| block.items())
        .find_map(check)
        .unwrap_or(false)
}

/// Recognizes the three original slots of a same-line explicit constant return.
pub(super) fn is_original_constant_return(graph: &BlockGraph, target: Label) -> bool {
    let Some(block) = graph.label_block(target) else {
        return false;
    };
    let mut started = false;
    let mut instructions = [None; 3];
    let mut count = 0;
    for item in graph[block].items() {
        match item {
            Item::Label(label) if !started => started = *label == target,
            Item::Label(_) if count == 0 => {}
            Item::Label(_) => return false,
            Item::Instruction(instruction) if started => {
                let Some(slot) = instructions.get_mut(count) else {
                    return false;
                };
                *slot = Some(instruction);
                count += 1;
            }
            Item::Instruction(_) => {}
        }
    }
    let [Some(marker), Some(value), Some(ret)] = instructions else {
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
        && [marker, value, ret].into_iter().all(|instruction| {
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
        for index in 0..self.graph.len().saturating_sub(2) {
            let source = self.graph.block_at(index);
            let target = self.graph.block_at(index + 2);
            let Some(jump) = self.graph[source].last_instruction() else {
                continue;
            };
            if !matches!(
                jump.opcode,
                POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
            ) {
                continue;
            }
            if self.graph[target].items().iter().any(
                |item| matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0),
            ) {
                continue;
            }
            let location = jump.location;
            if let Some(first) = self.graph.instructions_mut(target).next()
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
    pub(in crate::assembler) fn optimize_redundant_store_fast(&mut self) -> bool {
        let graph = &mut self.graph;
        if !graph.features().has_store_fast() {
            return false;
        }
        let mut changed = false;
        graph.for_each_adjacent_instruction_pair_mut(|first, second| {
            changed |= optimize_redundant_store_pair(first, second);
        });
        changed
    }

    /// Removes copy/pop pairs before local analysis and superinstruction fusion.
    pub(in crate::assembler) fn remove_redundant_copy_pop_pairs(&mut self) -> Option<u32> {
        if !self.graph.features().has_copy_top() {
            return None;
        }
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
        let removed_max_depth = redundant
            .iter()
            .filter_map(|position| match self.graph.item(*position) {
                Item::Instruction(instruction) => instruction.depth_after(),
                Item::Label(_) => unreachable!("a redundant copy/pop is an instruction"),
            })
            .max();
        self.graph.remove_ordered_instructions(&redundant);
        removed_max_depth
    }

    pub(in crate::assembler) fn remove_unreachable_instructions(&mut self) -> Option<u32> {
        self.remove_unreachable_instructions_with_proof::<true>()
    }

    fn remove_unreachable_instructions_with_proof<const PROVE_ALL_REACHABLE: bool>(
        &mut self,
    ) -> Option<u32> {
        let graph = &mut self.graph;
        if self.exception_regions.is_empty() && graph.len() == 1 {
            return None;
        }
        let reachable = match self.initial_reachable.take() {
            Some(reachable) => reachable,
            None if PROVE_ALL_REACHABLE
                && self.exception_regions.is_empty()
                && graph
                    .order()
                    .iter()
                    .take(graph.len().saturating_sub(1))
                    .all(|block| super::super::block_has_fallthrough(graph[*block].items())) =>
            {
                // The entry reaches every ordered block through physical fallthrough,
                // regardless of additional forward or backward edges.
                return None;
            }
            None => graph.reachable_blocks(&self.exception_regions),
        };
        if PROVE_ALL_REACHABLE
            && graph.order().iter().all(|block| reachable[block.index()])
            && self.exception_regions.iter().all(|region| {
                graph
                    .label_block(region.target)
                    .is_none_or(|target| reachable[target.index()])
            })
        {
            // Reachable blocks alone cannot create discarded admissions, retire handlers,
            // remove instructions, or contribute a removed stack depth.
            return None;
        }
        // CPython clears handler status when erasing an unreachable block. Keep that
        // decision on its original labels before normalization can attach them to a
        // live ordinary exit; the exception regions themselves retain their identity.
        self.retired_exception_handler_targets
            .extend(self.exception_regions.iter().filter_map(|region| {
                let target = graph.label_block(region.target)?;
                (!reachable[target.index()]).then_some(region.target)
            }));
        let mut removed_max_depth = None;
        let mut empty = FxHashSet::default();
        let mut discarded_admissions = Vec::new();
        for block_index in 0..graph.len() {
            let block = graph.block_at(block_index);
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
                    if instruction.depth_after().is_some() {
                        // CLEANUP_THROW consumes the incoming exception before its
                        // recorded depth, so its peak is one higher than that depth.
                        let depth = instruction.maximum_stack_depth();
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
                        self.exception_owner_provenance
                            .copied_return_pop_prefixes
                            .retain(|origin| origin.entry != *label);
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
        let retain_contains = self.python_version == crate::PythonVersion::Py315;
        let graph = &mut self.graph;
        if !graph.features().has_boolean_conversion() {
            return;
        }
        let was_boolean_conversion = graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => {
                    // A comparison folded into its following TO_BOOL also leaves an
                    // original NOP slot whose line may already have been recorded.
                    Some(matches!(
                        instruction.opcode,
                        TO_BOOL | UNARY_NOT | COMPARE_OP
                    ))
                }
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        if graph.features().has_identity_comparison()
            && let Some(constants) = constants
        {
            for block_index in 0..graph.len() {
                let block = graph.block_at(block_index);
                remove_none_comparison_boolean_conversions(graph.iter_items_mut(block), constants);
            }
        }
        graph.for_each_adjacent_instruction_pair_mut(|current, next| {
            optimize_boolean_pair(current, next, retain_contains);
        });
        let mut was_boolean_conversion = was_boolean_conversion.into_iter();
        for block_index in 0..graph.len() {
            let block = graph.block_at(block_index);
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
        self.thread_forward_jumps_with_head_gate::<true>()
    }

    fn thread_forward_jumps_with_head_gate<const GATE_HEADS: bool>(&mut self) -> bool {
        self.thread_forward_jumps_with_source_gates::<GATE_HEADS, true>()
    }

    fn thread_forward_jumps_with_source_gates<const GATE_HEADS: bool, const SOURCE_GATES: bool>(
        &mut self,
    ) -> bool {
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

        let nested_join_blocks = std::mem::take(&mut self.nested_if_unlocated_join_blocks);
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

        if has_forward_conditional && features.has_boolean_conversion() && features.has_copy_top() {
            let mut positions = graph.item_positions();
            let mut conditional_targets = Vec::new();
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
                if copy.opcode != COPY || !matches!(copy.operand(), Operand::Value(1)) {
                    continue;
                }
                let Some((_, to_bool)) = following.next() else {
                    continue;
                };
                if to_bool.opcode != TO_BOOL {
                    continue;
                }
                let Some((_, jump)) = following.next() else {
                    continue;
                };
                if !matches!(jump.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                    || !matches!(jump.operand(), Operand::Forward(_))
                {
                    continue;
                }
                let Some((_, not_taken)) = following.next() else {
                    continue;
                };
                if not_taken.opcode != NOT_TAKEN {
                    continue;
                }
                let Some((fallthrough_index, pop)) = following.next() else {
                    continue;
                };
                if pop.opcode != POP_TOP {
                    continue;
                }
                if conditional_targets.is_empty() {
                    conditional_targets.resize(
                        usize::try_from(self.next_label).expect("label count fits in usize"),
                        None,
                    );
                }
                let target = &mut conditional_targets[label.index()];
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
                        let target = conditional_targets.get(target.index()).copied().flatten()?;
                        (instruction.opcode != target.opcode).then_some(target.fallthrough_index)
                    })
                    .collect::<Vec<_>>();
                fallthrough_indices.sort_unstable();
                fallthrough_indices.dedup();
                let fallthrough_labels = fallthrough_indices
                    .into_iter()
                    .map(|index| (index, self.label()))
                    .collect::<SmallVec<[_; 4]>>();
                let fallthrough_label = |index| {
                    fallthrough_labels
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .ok()
                        .map(|index| &fallthrough_labels[index].1)
                };
                #[cfg(test)]
                {
                    let reference = fallthrough_labels
                        .iter()
                        .copied()
                        .collect::<FxHashMap<_, _>>();
                    for index in 0..positions.len() {
                        assert_eq!(fallthrough_label(index), reference.get(&index));
                    }
                }
                if !fallthrough_labels.is_empty() {
                    changed = true;
                    let graph = &mut self.graph;
                    for &(index, label) in fallthrough_labels.iter().rev() {
                        graph.insert_item(positions[index], Item::Label(label));
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
                        let Some(next) = conditional_targets.get(target.index()).copied().flatten()
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
                            let Some(fallthrough) = fallthrough_label(next.fallthrough_index)
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
        let mut label_offsets = ThreadLabelOffsets::new(self.next_label);
        let mut instruction_positions = Vec::new();
        let mut forward_jump_positions = Vec::new();
        let mut has_authored_forward_break = false;
        let mut has_retained_forward_nop = false;
        for (block, contents) in graph.iter() {
            for (offset, item) in contents.items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        label_offsets.insert(*label, instruction_positions.len());
                    }
                    Item::Instruction(instruction) => {
                        if instruction.opcode == JUMP_FORWARD
                            && matches!(instruction.operand(), Operand::Forward(_))
                        {
                            if SOURCE_GATES {
                                has_authored_forward_break |=
                                    instruction.fusion_policy.is_authored_break();
                                has_retained_forward_nop |=
                                    instruction.jump_policy.always_retains_nop();
                            }
                            forward_jump_positions
                                .push((instruction_positions.len(), (block, offset)));
                        }
                        instruction_positions.push((block, offset));
                    }
                }
            }
        }
        let authored_forward_jump_positions = if !SOURCE_GATES || has_authored_forward_break {
            forward_jump_positions.as_slice()
        } else {
            &[]
        };
        let has_original_boundaries = !self.exception_owner_provenance.source_boundaries.is_empty()
            && !self.exception_owner_provenance.block_boundaries.is_empty();
        let original_authored_forward_jump_positions = if !SOURCE_GATES || has_original_boundaries {
            authored_forward_jump_positions
        } else {
            &[]
        };
        // The compiler certified both incoming paths to this iterator discard.
        // Preserve its copy permission only while the complete original array,
        // including every label and instruction field, still survives unchanged.
        // Only the earlier pass's exact initial-borrow tag can extend its jump.
        let iterator_extra_slot = usize::from(self.python_version == crate::PythonVersion::Py315);
        let mut copied_finally_iterator_breaks =
            std::mem::take(&mut self.copied_finally_iterator_break_origins)
                .into_iter()
                .filter_map(|mut origin| {
                    let original_len = origin.len().checked_sub(iterator_extra_slot)?;
                    let guarded_try = matches!(original_len, 39 | 42 | 44 | 47);
                    // Thirty records are the separately certified named-handler await.
                    let guarded_manager = original_len == 65 + iterator_extra_slot;
                    let awaited_handler = original_len == 30 + iterator_extra_slot;
                    if !matches!(original_len, 12 | 17)
                        && !awaited_handler
                        && !guarded_try
                        && !guarded_manager
                    {
                        return None;
                    }
                    let last = origin.len() - 1;
                    let (Item::Label(start), Item::Label(join), Item::Instruction(mut jump)) = (
                        origin[0],
                        origin[last - 2 - iterator_extra_slot],
                        origin[last],
                    ) else {
                        return None;
                    };
                    let jump_index = label_offsets
                        .get(join)?
                        .checked_add(1 + iterator_extra_slot)?;
                    let &(block, offset) = instruction_positions.get(jump_index)?;
                    let Item::Instruction(current) = graph[block].items()[offset] else {
                        return None;
                    };
                    if let Some(target) = current.initial_borrow_jump_target() {
                        if jump.operand() != Operand::Forward(target) {
                            return None;
                        }
                        // The earlier borrowing pass adds only this proof tag to
                        // the same original direct edge; ownership stays literal.
                        jump.mark_initial_borrow_jump_target(target);
                        origin[last] = Item::Instruction(jump);
                    }
                    if guarded_try || guarded_manager {
                        // Both internal cleanup exits receive the same initial-borrow tag
                        // as the final break. Preserve every other original field.
                        let exits = if guarded_manager {
                            [41 + iterator_extra_slot, 56 + iterator_extra_slot]
                        } else {
                            [
                                last - 24 - iterator_extra_slot,
                                last - 11 - iterator_extra_slot,
                            ]
                        };
                        for offset in exits {
                            let Item::Instruction(mut original) = origin[offset] else {
                                return None;
                            };
                            let Some(Item::Instruction(current)) = graph
                                .iter_items()
                                .skip_while(|item| **item != Item::Label(start))
                                .nth(offset)
                            else {
                                return None;
                            };
                            if let Some(target) = current.initial_borrow_jump_target() {
                                if original.opcode != JUMP_FORWARD
                                    || original.operand() != Operand::Forward(target)
                                {
                                    return None;
                                }
                                original.mark_initial_borrow_jump_target(target);
                                origin[offset] = Item::Instruction(original);
                            }
                        }
                    }
                    if !graph
                        .iter_items()
                        .skip_while(|item| **item != Item::Label(start))
                        .take(origin.len())
                        .eq(origin.iter())
                    {
                        return None;
                    }
                    Some((jump_index, jump))
                })
                .collect::<FxHashMap<_, _>>();
        // Conjunction finalizers have their own producer certificate. Replay the
        // complete emitted array rather than treating its length as a named-handler
        // or guarded-try shape. Only the established initial-borrow tag may change.
        for mut origin in std::mem::take(&mut self.copied_finally_conjunction_break_origins) {
            if origin.len() < 4 {
                continue;
            }
            let last = origin.len() - 1;
            let (Item::Label(start), Item::Label(join), Item::Instruction(mut jump)) = (
                origin[0],
                origin[last - 2 - iterator_extra_slot],
                origin[last],
            ) else {
                continue;
            };
            let Some(jump_index) = label_offsets
                .get(join)
                .and_then(|index| index.checked_add(1 + iterator_extra_slot))
            else {
                continue;
            };
            let Some(&(block, offset)) = instruction_positions.get(jump_index) else {
                continue;
            };
            let Item::Instruction(current) = graph[block].items()[offset] else {
                continue;
            };
            if let Some(target) = current.initial_borrow_jump_target() {
                if jump.operand() != Operand::Forward(target) {
                    continue;
                }
                jump.mark_initial_borrow_jump_target(target);
                origin[last] = Item::Instruction(jump);
            }
            if graph
                .iter_items()
                .skip_while(|item| **item != Item::Label(start))
                .take(origin.len())
                .eq(origin.iter())
            {
                copied_finally_iterator_breaks.insert(jump_index, jump);
            }
        }
        // Cleanup tries have a separate producer certificate and variable-size arrays.
        // Replay every original record; the initial borrowing pass may only tag an
        // unchanged direct forward edge, including internal normal cleanup exits.
        for origin in std::mem::take(&mut self.shared_finally_iterator_break_origins) {
            if origin.len() < 4 {
                continue;
            }
            let last = origin.len() - 1;
            let (Item::Label(start), Item::Label(join), Item::Instruction(_)) = (
                origin[0],
                origin[last - 2 - iterator_extra_slot],
                origin[last],
            ) else {
                continue;
            };
            let Some(jump_index) = label_offsets
                .get(join)
                .and_then(|index| index.checked_add(1 + iterator_extra_slot))
            else {
                continue;
            };
            let Some(&(block, offset)) = instruction_positions.get(jump_index) else {
                continue;
            };
            let Item::Instruction(jump) = graph[block].items()[offset] else {
                continue;
            };
            let mut current = graph
                .iter_items()
                .skip_while(|item| **item != Item::Label(start));
            let unchanged = origin.iter().all(|original| {
                let Some(current) = current.next() else {
                    return false;
                };
                match (*original, *current) {
                    (Item::Instruction(mut original), Item::Instruction(current)) => {
                        if let Some(target) = current.initial_borrow_jump_target() {
                            if original.opcode != JUMP_FORWARD
                                || original.operand() != Operand::Forward(target)
                            {
                                return false;
                            }
                            original.mark_initial_borrow_jump_target(target);
                        }
                        original == current
                    }
                    (original, current) => original == current,
                }
            });
            if unchanged {
                copied_finally_iterator_breaks.insert(jump_index, jump);
            }
        }
        // Bind only a surviving original nested join's complete jump record.
        // Aliased labels containing work or omitted authored NOPs cannot certify it.
        let mut nested_join_sources = FxHashMap::<usize, Vec<(Label, Instruction)>>::default();
        let mut nested_join_blocks: Vec<_> = nested_join_blocks.into_iter().collect();
        nested_join_blocks.sort_unstable_by_key(|label| label.as_u32());
        for entry in nested_join_blocks {
            let Some(&index) = label_offsets.get(entry) else {
                continue;
            };
            let Some(&(block, offset)) = instruction_positions.get(index) else {
                continue;
            };
            let items = graph[block].items();
            let Item::Instruction(jump) = items[offset] else {
                continue;
            };
            if offset + 1 != items.len()
                || !items[..offset].contains(&Item::Label(entry))
                || items[..offset].iter().any(|item| match item {
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
                || jump.opcode != JUMP_FORWARD
                || jump.location != SourceLocation::NONE
                || !jump.no_location_policy.preserves_location()
                || jump.no_location_policy.is_artificial_eval_break()
                || jump.jump_policy.is_interruptible()
                || jump.jump_policy.always_retains_nop()
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
            {
                continue;
            }
            nested_join_sources
                .entry(index)
                .or_default()
                .push((entry, jump));
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
        let planned_sources = if planned_blocks.is_empty() {
            FxHashSet::default()
        } else {
            instruction_positions
                .iter()
                .enumerate()
                .filter_map(|(index, (block, _))| planned_blocks.contains(block).then_some(index))
                .collect::<FxHashSet<_>>()
        };
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
                let entry_index = *label_offsets.get(*entry)?;
                let target_index = *label_offsets.get(*target)?;
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
                let entry_index = *label_offsets.get(*entry)?;
                let target_index = *label_offsets.get(*target)?;
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
        // This producer-certified empty join has no original POP_BLOCK. Its
        // synthetic exclusion label must not transfer the absent target owner
        // onto an incoming branch when the redundant join is threaded away.
        // Validate the complete original jump and all of its current entry aliases;
        // copied, populated, or real ordinary-try exits keep the existing rules.
        let synthetic_manager_join_offsets = self
            .exception_owner_provenance
            .synthetic_manager_join_exits
            .iter()
            .filter_map(|(entry, original)| {
                let index = *label_offsets.get(*entry)?;
                let &(block, offset) = instruction_positions.get(index)?;
                let items = graph[block].items();
                let Item::Instruction(jump) = items[offset] else {
                    return None;
                };
                (jump == *original
                    && jump.opcode == JUMP_FORWARD
                    && jump.location == SourceLocation::NONE
                    && jump.no_location_policy.preserves_location()
                    && jump.exception_policy() == super::super::ExceptionPolicy::default()
                    && jump.pre_fold_extra_instructions() == 0
                    && jump.local_policy.hidden_exception_owner_slots() == 0
                    && !jump.fusion_policy.is_converted_pop_block()
                    && !jump.fusion_policy.is_authored_break()
                    && !jump.jump_policy.is_interruptible()
                    && jump.jump_policy.defers_redundant_removal()
                    && !jump.jump_policy.preserves_nop_after_threading()
                    && offset + 1 == items.len()
                    && items[..offset].contains(&Item::Label(*entry))
                    && items[..offset].iter().all(|item| match item {
                        Item::Instruction(_) => false,
                        Item::Label(label) => {
                            !self.original_nop_targets.contains(label)
                                && !self.borrow_block_aliases.contains(label)
                                && !self
                                    .exception_owner_provenance
                                    .original_manager_pop_blocks
                                    .contains(label)
                                && !self
                                    .exception_owner_provenance
                                    .omitted_nop_counts
                                    .contains_key(label)
                                && !self
                                    .exception_owner_provenance
                                    .omitted_nop_locations
                                    .contains_key(label)
                        }
                    })
                    && !normal_try_exits.contains(&index)
                    && !compound_try_owner_exits.contains(&index))
                .then_some(index)
            })
            .collect::<FxHashSet<_>>();
        let mut jump_targets =
            vec![None; usize::try_from(self.next_label).expect("label count fits in usize")];
        let mut jump_target_count = 0;
        let mut next_instruction = None;
        for block in graph.order().iter().rev().copied() {
            for item in graph[block].items().iter().rev() {
                match item {
                    Item::Instruction(instruction) => next_instruction = Some(instruction),
                    Item::Label(label) => {
                        let Some(&instruction) = next_instruction else {
                            continue;
                        };
                        // Only an eligible unconditional jump can populate this label's
                        // target slot. The original-loop and Pass certificates below are
                        // pure rejection checks, so ordinary heads need none of their lookups.
                        if GATE_HEADS
                            && (!is_unconditional_jump(instruction.opcode)
                                || !matches!(
                                    instruction.operand(),
                                    Operand::Forward(_) | Operand::Backward(_)
                                )
                                || !instruction.jump_policy.allows_threading_target()
                                || instruction.jump_policy.always_retains_nop())
                        {
                            continue;
                        }
                        // CPython still sees the authored NOP before this loop backedge
                        // when it threads jumps. Lowering removed that boundary early.
                        let original_loop_tail_nop = (|| {
                            if !self.original_nop_targets.contains(label)
                                || !self
                                    .exception_owner_provenance
                                    .block_boundaries
                                    .contains(label)
                                || instruction.opcode != JUMP_BACKWARD
                                || instruction.location.line < 0
                                || instruction.pre_fold_extra_instructions() > 1
                            {
                                return false;
                            }
                            let (Some(depth), Operand::Backward(head)) =
                                (instruction.depth_after(), instruction.operand())
                            else {
                                return false;
                            };
                            let Some(head_block) = graph.label_block(head) else {
                                return false;
                            };
                            let Some(Item::Instruction(loop_head)) = graph[head_block]
                                .items()
                                .iter()
                                .find(|item| matches!(item, Item::Instruction(_)))
                            else {
                                return false;
                            };
                            let Some((Item::Instruction(tail), labels)) =
                                graph[block].items().split_last()
                            else {
                                return false;
                            };
                            if *tail != instruction
                                || labels.iter().any(|item| !matches!(item, Item::Label(_)))
                                || !self
                                    .exception_owner_provenance
                                    .block_boundaries
                                    .contains(&head)
                            {
                                return false;
                            }
                            let iterator_head = loop_head.opcode
                                == crate::target::opcodes::FOR_ITER
                                && loop_head.depth_after() == depth.checked_add(1);
                            // The folded-while producer records its exact test NOP and exit.
                            // Require that unchanged one-record header and one omitted Pass;
                            // arbitrary NOP-headed targets do not establish a loop boundary.
                            let folded_while_head = instruction.pre_fold_extra_instructions() == 1
                                && *loop_head
                                    == Instruction::new(
                                        NOP,
                                        Operand::Value(0),
                                        loop_head.location,
                                        Some(depth),
                                    )
                                && graph[head_block]
                                    .items()
                                    .iter()
                                    .filter(|item| matches!(item, Item::Instruction(_)))
                                    .count()
                                    == 1
                                && self.folded_predecessor_sources.iter().any(
                                    |(source, _, location)| {
                                        *source == head && *location == loop_head.location
                                    },
                                );
                            if !iterator_head && !folded_while_head {
                                return false;
                            }
                            let mut original = Instruction::new(
                                JUMP_BACKWARD,
                                Operand::Backward(head),
                                instruction.location,
                                Some(depth),
                            );
                            original.add_pre_fold_extra_instructions(
                                instruction.pre_fold_extra_instructions(),
                            );
                            // A folded condition can leave its source NOP on an excluded
                            // loop backedge. Authenticate that producer before accepting its
                            // exact policy; folded while headers and copied aliases differ.
                            if self.python_version >= crate::PythonVersion::Py314
                                && iterator_head
                                && instruction.pre_fold_extra_instructions() == 0
                                && instruction.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
                                && self
                                    .folded_predecessor_sources
                                    .iter()
                                    .filter(|(source, target, location)| {
                                        labels.contains(&Item::Label(*source))
                                            && self.original_nop_targets.contains(source)
                                            && *location == instruction.location
                                            && !self
                                                .exception_owner_provenance
                                                .block_boundaries
                                                .contains(source)
                                            && self
                                                .folded_original_predecessor_targets
                                                .contains(target)
                                            && *target != head
                                            && !labels.contains(&Item::Label(*target))
                                            && !self.folded_loop_exit_anchors.contains(target)
                                    })
                                    .take(2)
                                    .count()
                                    == 1
                            {
                                original.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                            }
                            instruction == original
                        })();
                        // A nested source join can contain an authored Pass followed only by
                        // its enclosing branch's jump. CPython still sees that NOP when it
                        // visits the earlier incoming jump; compaction transfers the Pass's
                        // location to the branch jump only after all blocks were optimized.
                        let original_forward_pass_head = (|| {
                            if self.python_version < crate::PythonVersion::Py314
                                || !self.original_nop_targets.contains(label)
                                || instruction.opcode != JUMP_FORWARD
                                || instruction.location.line < 0
                            {
                                return false;
                            }
                            let (Some(depth), Operand::Forward(target)) =
                                (instruction.depth_after(), instruction.operand())
                            else {
                                return false;
                            };
                            let Some((Item::Instruction(tail), labels)) =
                                graph[block].items().split_last()
                            else {
                                return false;
                            };
                            // Lowering can compact an emitted Pass into this jump.
                            // Its original slot and unique source record certify the
                            // same head as an omitted Pass, before CFG optimization.
                            let pass = Instruction::new(
                                NOP,
                                Operand::Value(0),
                                instruction.location,
                                Some(depth),
                            );
                            let emitted_pass = instruction.pre_fold_extra_instructions() == 1
                                && self
                                    .exception_owner_provenance
                                    .original_pass_nops
                                    .iter()
                                    .filter(|original| **original == pass)
                                    .count()
                                    == 1;
                            if *tail != instruction
                                || !labels.iter().any(|item| matches!(item, Item::Label(entry)
                                    if self.exception_owner_provenance.source_boundaries.contains(entry)
                                        && self.exception_owner_provenance.block_boundaries.contains(entry)))
                                || !labels.iter().all(|item| matches!(item, Item::Label(entry)
                                    if self.original_nop_targets.contains(entry)
                                        && (if emitted_pass {
                                            !self.exception_owner_provenance.omitted_nop_counts.contains_key(entry)
                                                && !self.exception_owner_provenance.omitted_nop_locations.contains_key(entry)
                                        } else {
                                            self.exception_owner_provenance.omitted_nop_counts.get(entry) == Some(&1)
                                                && self.exception_owner_provenance.omitted_nop_locations.get(entry) == Some(&instruction.location)
                                        })))
                            {
                                return false;
                            }
                            let mut original = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(target),
                                instruction.location,
                                Some(depth),
                            );
                            if emitted_pass {
                                original.add_pre_fold_extra_instructions(1);
                            }
                            instruction == original
                        })();
                        // A jump retained as a source-position NOP remains a CFG boundary.
                        if instruction.jump_policy.allows_threading_target()
                            && !instruction.jump_policy.always_retains_nop()
                            && !original_loop_tail_nop
                            && !original_forward_pass_head
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
                                && !synthetic_manager_join_offsets.contains(&label_offsets[label])
                                && !(instruction.jump_policy.preserves_nop_after_threading()
                                    && instruction.location.line < 0
                                    && !normal_try_exits.contains(&label_offsets[label])
                                    && (!compound_try_owner_exits
                                        .contains(&label_offsets[label])
                                        || self.original_nop_targets.contains(label))))
                            .then_some(label_offsets[label]);
                            let target = &mut jump_targets[label.index()];
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
                                            || (self.borrow_loop_exit_blocks.contains_key(label)
                                                && normal_try_exits
                                                    .contains(&label_offsets[label]))
                                            || !instruction
                                                .jump_policy
                                                .allows_borrow_threading_target(),
                                        instruction.jump_policy.is_interruptible(),
                                        instruction.jump_policy.preserves_nop_after_threading(),
                                        normal_try_exits.contains(&label_offsets[label])
                                            && !self.original_nop_targets.contains(label),
                                        self.original_nop_targets.contains(label)
                                            && instruction.location.line >= 0
                                            && instruction.pre_fold_extra_instructions() == 1
                                            && instruction
                                                .local_policy
                                                .hidden_exception_owner_slots()
                                                == 0,
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
        let original_head_limit = if !SOURCE_GATES || !original_head_blocks.is_empty() {
            usize::MAX
        } else {
            0
        };
        for (block, contents) in graph.iter().take(original_head_limit) {
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
            if jump_targets[target.index()].is_none() {
                surviving_heads.push(block);
            } else {
                dependent_heads.entry(target).or_default().push(block);
            }
        }
        while let Some(block) = surviving_heads.pop() {
            if let Some(labels) = nop_targets.remove(&block) {
                for label in labels {
                    if jump_targets[label.index()].take().is_some()
                        && let Some(dependents) = dependent_heads.remove(&label)
                    {
                        surviving_heads.extend(dependents);
                    }
                }
            }
        }

        // A folded loop's original exit can contain only its enclosing branch jump.
        // CPython copies that unlocated jump before the four-record method return,
        // retaining the authored break line across both copies. Our earlier threading
        // must not discard that direct-copy request. Validate the original loop end
        // and complete small return; arbitrary retargeted breaks remain unchanged.
        let folded_return_sources = if !SOURCE_GATES
            || (has_authored_forward_break
                && has_original_boundaries
                && self.exception_regions.is_empty())
        {
            self.folded_predecessor_sources.as_slice()
        } else {
            &[]
        };
        let folded_loop_return_heads = folded_return_sources
            .iter()
            .filter_map(|(source, entry, location)| {
                if !self.exception_regions.is_empty()
                    || !self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(source)
                    || !self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(source)
                    || !self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(entry)
                    || !self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(entry)
                {
                    return None;
                }
                let (Item::Instruction(jump), labels) =
                    graph[graph.label_block(*entry)?].items().split_last()?
                else {
                    return None;
                };
                let Operand::Forward(target) = jump.operand() else {
                    return None;
                };
                if labels.is_empty()
                    || labels.iter().any(|item| !matches!(item, Item::Label(_)))
                    || jump.opcode != JUMP_FORWARD
                    || jump.location != *location
                    || jump.jump_policy.retained_nop()
                        != super::super::ir::RetainedJumpNop::DirectTarget
                    || jump.pre_fold_extra_instructions() != 0
                    || jump.local_policy.hidden_exception_owner_slots() != 0
                    || jump.fusion_policy.is_authored_break()
                {
                    return None;
                }
                let target_items = graph[graph.label_block(target)?].items();
                let first = target_items
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))?;
                let [
                    Item::Instruction(load),
                    Item::Instruction(attribute),
                    Item::Instruction(call),
                    Item::Instruction(return_),
                ] = &target_items[first..]
                else {
                    return None;
                };
                (load.opcode == crate::target::opcodes::LOAD_FAST
                    && attribute.opcode == crate::target::opcodes::LOAD_ATTR
                    && matches!(attribute.operand(), Operand::Value(argument) if argument & 1 != 0)
                    && call.opcode == crate::target::opcodes::CALL
                    && call.operand() == Operand::Value(0)
                    && return_.opcode == RETURN_VALUE
                    && [load, attribute, call, return_].iter().all(|instruction| {
                        matches!(instruction.operand(), Operand::Value(_))
                            && instruction.pre_fold_extra_instructions() == 0
                            && instruction.local_policy.hidden_exception_owner_slots() == 0
                            && instruction.location.line == return_.location.line
                            && instruction.location.line >= 0
                    }))
                .then_some(*entry)
            })
            .collect::<FxHashSet<_>>();
        // CPython copies an original unlocated join onto a break before copying
        // its small return. Threading through that join must keep the break's
        // own replaced-jump NOP, even though the return has a different location.
        let mut unlocated_return_join_heads = FxHashSet::default();
        let original_join_limit =
            if !SOURCE_GATES || (has_authored_forward_break && has_original_boundaries) {
                usize::MAX
            } else {
                0
            };
        for (_, block) in graph.iter().take(original_join_limit) {
            let Some((Item::Instruction(jump), labels)) = block.items().split_last() else {
                continue;
            };
            let Operand::Forward(target) = jump.operand() else {
                continue;
            };
            if jump.opcode != JUMP_FORWARD
                || jump.location != SourceLocation::NONE
                || !jump.no_location_policy.preserves_location()
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
                || jump.exception_policy() != super::super::ExceptionPolicy::default()
                || jump.fusion_policy.is_authored_break()
                || labels.is_empty()
                || labels.iter().any(|item| {
                    !matches!(item, Item::Label(label)
                    if !self.original_nop_targets.contains(label)
                        && !self.borrow_block_aliases.contains(label))
                })
                || !labels.iter().any(|item| {
                    matches!(item, Item::Label(label)
                    if self.exception_owner_provenance.source_boundaries.contains(label)
                        && self.exception_owner_provenance.block_boundaries.contains(label))
                })
            {
                continue;
            }
            let Some(target_block) = graph.label_block(target) else {
                continue;
            };
            let Some(return_) = graph[target_block].last_instruction() else {
                continue;
            };
            let mut target_width = 0;
            if return_.opcode != RETURN_VALUE
                || return_.location.line < 0
                || !graph[target_block].items().iter().all(|item| match item {
                    Item::Label(_) => target_width == 0,
                    Item::Instruction(instruction) => {
                        // LOAD_GLOBAL's fused PUSH_NULL still occupies an original slot.
                        target_width += 1 + usize::from(
                            instruction.opcode == LOAD_GLOBAL
                                && matches!(instruction.operand(), Operand::Value(argument)
                                if argument & 1 != 0),
                        );
                        target_width <= 4
                            && matches!(instruction.operand(), Operand::Value(_))
                            && instruction.location.line == return_.location.line
                            && instruction.pre_fold_extra_instructions() == 0
                            && instruction.local_policy.hidden_exception_owner_slots() == 0
                            && instruction.exception_policy()
                                == super::super::ExceptionPolicy::default()
                    }
                })
            {
                continue;
            }
            let Item::Label(entry) = labels[0] else {
                unreachable!("validated original join labels");
            };
            let join_index = label_offsets[&entry];
            let return_start = label_offsets[&target];
            let return_end = return_start
                + graph[target_block]
                    .items()
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count();
            // An earlier inlined comprehension may have its own recovery region.
            // Only coverage of this original join or the copied return matters.
            if region_ranges.iter().any(|range| {
                range.contains(&join_index)
                    || (range.start < range.end
                        && range.start < return_end
                        && return_start < range.end)
            }) {
                continue;
            }
            unlocated_return_join_heads.extend(labels.iter().filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            }));
        }
        // A cleanup-free break first copies its original loop-exit bridge, then
        // the small constant, local or tuple return. Keep that break's requested NOP through
        // our earlier threading only when all three original arrays still match.
        let original_break_return_bridges = original_authored_forward_jump_positions
            .iter()
            .filter_map(|&(source_index, (block, offset))| {
                let source_items = graph[block].items();
                let Item::Instruction(source) = source_items[offset] else {
                    return None;
                };
                let Operand::Forward(entry) = source.operand() else {
                    return None;
                };
                if source.opcode != JUMP_FORWARD
                    || !source.fusion_policy.is_authored_break()
                    || source.location.line < 0
                    || source.jump_policy.retained_nop()
                        != super::super::ir::RetainedJumpNop::DirectTarget
                    || !source.jump_policy.inline_small_exit()
                    || source.jump_policy.allows_threading_target()
                    || source.depth_after() != Some(0)
                    // The producer's authored break NOP is the sole omitted slot.
                    || source.pre_fold_extra_instructions() != 1
                    || offset + 1 != source_items.len()
                    || !source_items[..offset].iter().all(|item| matches!(item, Item::Label(_)))
                {
                    return None;
                }
                let (Item::Instruction(bridge), labels) =
                    graph[graph.label_block(entry)?].items().split_last()?
                else {
                    return None;
                };
                let Operand::Forward(target) = bridge.operand() else {
                    return None;
                };
                let clean_label = |item: &Item| matches!(item, Item::Label(label)
                    if self.exception_owner_provenance.block_boundaries.contains(label)
                        && !self.original_nop_targets.contains(label)
                        && !self.borrow_block_aliases.contains(label)
                        && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                        && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label));
                if bridge.opcode != JUMP_FORWARD
                    || bridge.fusion_policy.is_authored_break()
                    || bridge.fusion_policy.is_converted_pop_block()
                    || !bridge.jump_policy.allows_threading_target()
                    || bridge.jump_policy.is_interruptible()
                    || bridge.depth_after() != Some(0)
                    // A constant loop has only its exit label; a dynamic loop can
                    // add an alias. Both identify the same intact native jump array.
                    || labels.is_empty()
                    || !labels.iter().all(clean_label)
                    || !labels.iter().any(|item| matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.source_boundaries.contains(label)))
                    || !((bridge.location == SourceLocation::NONE
                        && bridge.no_location_policy.preserves_location())
                        || (bridge.location.line >= 0
                            && !bridge.no_location_policy.preserves_location()
                            && bridge.jump_policy.retained_nop()
                                == super::super::ir::RetainedJumpNop::DirectTarget))
                {
                    return None;
                }
                let target_items = graph[graph.label_block(target)?].items();
                let first = target_items
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))?;
                let return_items = &target_items[first..];
                let local_return = matches!(return_items,
                    [Item::Instruction(load), Item::Instruction(return_)]
                        if matches!(load.opcode, crate::target::opcodes::LOAD_FAST | crate::target::opcodes::LOAD_DEREF)
                            && load.depth_after() == Some(1)
                            && return_.opcode == RETURN_VALUE);
                let constant_return = is_original_constant_return(graph, target)
                    && return_items.iter().zip([0, 1, 0]).all(|(item, depth)| {
                        matches!(item, Item::Instruction(instruction)
                            if instruction.depth_after() == Some(depth))
                    });
                let tuple_return = super::normal_try_copies::original_local_tuple_return(return_items)
                    .is_some()
                    && return_items.iter().zip([1, 2, 1, 0]).all(|(item, depth)| {
                        matches!(item, Item::Instruction(instruction)
                            if instruction.depth_after() == Some(depth))
                    });
                let Some(Item::Instruction(return_)) = return_items.last() else {
                    return None;
                };
                if !(constant_return || local_return || tuple_return)
                    || !target_items[..first].iter().all(clean_label)
                    || return_.depth_after() != Some(0)
                    || return_.location.line < 0
                    || source.location.line == return_.location.line
                    || return_.operand() != Operand::Value(0)
                    || !return_items.iter().all(|item| matches!(item,
                        Item::Instruction(instruction)
                            if instruction.location.line == return_.location.line
                                && matches!(instruction.operand(), Operand::Value(_))
                                && instruction.pre_fold_extra_instructions() == 0
                                && instruction.local_policy.hidden_exception_owner_slots() == 0
                                && instruction.exception_policy()
                                    == super::super::ExceptionPolicy::default()))
                    || bridge.pre_fold_extra_instructions() != 0
                    || [&source, bridge].iter().any(|instruction| {
                        instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.exception_policy()
                                != super::super::ExceptionPolicy::default()
                    })
                {
                    return None;
                }
                let join_index = label_offsets[&entry];
                let return_index = label_offsets[&target];
                // A coroutine's single StopIteration owner is split around await
                // and cleanup arrays. A constant exit can cross those intervals
                // only when every original slot still has that exact same owner.
                let shared_constant_fallback = constant_return
                    && self.exception_regions.iter().zip(&region_ranges)
                        .find(|(_, range)| range.contains(&source_index))
                        .is_some_and(|(owner, _)| {
                            owner.is_scope_fallback && owner.depth == 0 && owner.preserve_lasti
                                && [source_index, join_index].into_iter()
                                    .chain(return_index..return_index + return_items.len())
                                    .all(|index| {
                                        let mut found = false;
                                        for (region, range) in self.exception_regions.iter().zip(&region_ranges) {
                                            if range.contains(&index) {
                                                if !region.is_scope_fallback || region.target != owner.target
                                                    || region.depth != owner.depth || region.preserve_lasti != owner.preserve_lasti {
                                                    return false;
                                                }
                                                found = true;
                                            }
                                        }
                                        found
                                    })
                        });
                if !(source_index < join_index && join_index < return_index)
                    || (!shared_constant_fallback && self.exception_regions.iter().zip(&region_ranges).any(|(region, range)| {
                        let mut covered = [source_index, join_index].into_iter()
                            .chain(return_index..return_index + return_items.len());
                        let any_covered = covered.clone().any(|index| range.contains(&index));
                        any_covered
                            && (!region.is_scope_fallback || !covered.all(|index| range.contains(&index)))
                    }))
                {
                    return None;
                }
                Some((source_index, (source, target)))
            })
            .collect::<FxHashMap<_, _>>();
        // A folded loop end can be the original bridge between a cleanup-free
        // break and its handler's bare raise. CPython copies the bridge first,
        // preserving the break marker before copying the raise. Keep that request
        // only while all three original arrays have the same exception coverage.
        let folded_break_sources = if !SOURCE_GATES || !self.folded_predecessor_sources.is_empty() {
            original_authored_forward_jump_positions
        } else {
            &[]
        };
        let folded_loop_raise_breaks = folded_break_sources
            .iter()
            .filter_map(|&(source_index, (block, offset))| {
                let source_items = graph[block].items();
                let Item::Instruction(source) = source_items[offset] else {
                    return None;
                };
                let Operand::Forward(entry) = source.operand() else {
                    return None;
                };
                if source.opcode != JUMP_FORWARD
                    || !source.fusion_policy.is_authored_break()
                    || source.jump_policy.retained_nop()
                        != super::super::ir::RetainedJumpNop::DirectTarget
                    || !source.jump_policy.inline_small_exit()
                    || source.jump_policy.allows_threading_target()
                    || source.location.line < 0
                    || offset + 1 != source_items.len()
                    || !source_items[..offset].iter().all(|item| matches!(item, Item::Label(_)))
                {
                    return None;
                }
                let (Item::Instruction(bridge), labels) =
                    graph[graph.label_block(entry)?].items().split_last()?
                else {
                    return None;
                };
                let Operand::Forward(target) = bridge.operand() else {
                    return None;
                };
                let (Item::Instruction(raise), target_labels) =
                    graph[graph.label_block(target)?].items().split_last()?
                else {
                    return None;
                };
                let original_entry = |items: &[Item]| {
                    !items.is_empty()
                        && items.iter().all(|item| matches!(item, Item::Label(label)
                            if !self.original_nop_targets.contains(label)
                                && !self.borrow_block_aliases.contains(label)
                                && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                                && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label)))
                        && items.iter().any(|item| matches!(item, Item::Label(label)
                            if self.exception_owner_provenance.source_boundaries.contains(label)
                                && self.exception_owner_provenance.block_boundaries.contains(label)))
                };
                if !original_entry(labels)
                    || !original_entry(target_labels)
                    || bridge.opcode != JUMP_FORWARD
                    || bridge.fusion_policy.is_authored_break()
                    || !bridge.jump_policy.allows_threading_target()
                    || bridge.jump_policy.retained_nop()
                        != super::super::ir::RetainedJumpNop::DirectTarget
                    || !self.folded_predecessor_sources.iter().any(|(_, end, location)| {
                        *end == entry && *location == bridge.location
                    })
                    || raise.opcode != RAISE_VARARGS
                    || raise.operand() != Operand::Value(0)
                    || raise.location.line < 0
                    || source.location.line == raise.location.line
                    || source.depth_after().is_none()
                    || source.depth_after() != bridge.depth_after()
                    || source.depth_after() != raise.depth_after()
                    || [&source, bridge, raise].iter().any(|instruction| {
                        instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.exception_policy() != ExceptionPolicy::default()
                            || instruction.no_location_policy.preserves_location()
                            || instruction.fusion_policy.is_converted_pop_block()
                            || instruction.jump_policy.is_interruptible()
                    })
                {
                    return None;
                }
                let bridge_index = label_offsets[&entry];
                let raise_index = label_offsets[&target];
                if !(source_index < bridge_index && bridge_index < raise_index)
                    || region_ranges.iter().any(|range| {
                        let covered = range.contains(&source_index);
                        range.contains(&bridge_index) != covered
                            || range.contains(&raise_index) != covered
                    })
                {
                    return None;
                }
                Some((source_index, (source, target)))
            })
            .collect::<FxHashMap<_, _>>();
        // A post-manager break retains a NOP for small-exit copying. That marker
        // does not prevent threading its original, unowned single backedge.
        // Keep all forward exits and cleanup-bearing targets under the usual guard.
        let mut retained_break_policy = super::super::ir::JumpPolicy::default();
        retained_break_policy.retain_nop();
        retained_break_policy.prevent_threading_target();
        let retained_break_backedges = original_authored_forward_jump_positions
            .iter()
            .filter_map(|&(source_index, (block, offset))| {
                let items = graph[block].items();
                let Item::Instruction(jump) = items[offset] else {
                    return None;
                };
                if jump.opcode != JUMP_FORWARD
                    || !jump.fusion_policy.is_authored_break()
                    || jump.jump_policy != retained_break_policy
                    || jump.location.line < 0
                    || jump.pre_fold_extra_instructions() != 0
                    || jump.local_policy.hidden_exception_owner_slots() != 0
                    || jump.exception_policy() != super::super::ExceptionPolicy::default()
                    || jump.no_location_policy != super::super::ir::NoLocationPolicy::default()
                    || offset + 1 != items.len()
                    || items[..offset]
                        .iter()
                        .any(|item| !matches!(item, Item::Label(_)))
                {
                    return None;
                }
                let Operand::Forward(entry) = jump.operand() else {
                    return None;
                };
                let entry_index = *label_offsets.get(entry)?;
                let target_block = graph.label_block(entry)?;
                let [Item::Label(label), Item::Instruction(backedge)] = graph[target_block].items()
                else {
                    return None;
                };
                let Operand::Backward(target) = backedge.operand() else {
                    return None;
                };
                let target_index = *label_offsets.get(target)?;
                (*label == entry
                    && target_index < source_index
                    && source_index < entry_index
                    && self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(&entry)
                    && self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(&entry)
                    && !self.original_nop_targets.contains(&entry)
                    && !self
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .contains_key(&entry)
                    && !self
                        .exception_owner_provenance
                        .omitted_nop_locations
                        .contains_key(&entry)
                    && !planned_sources.contains(&entry_index)
                    && region_ranges.iter().all(|range| {
                        !range.contains(&source_index) && !range.contains(&entry_index)
                    })
                    && backedge.opcode == JUMP_BACKWARD
                    && backedge.location == SourceLocation::NONE
                    && backedge.jump_policy == super::super::ir::JumpPolicy::default()
                    && backedge.no_location_policy == super::super::ir::NoLocationPolicy::default()
                    && backedge.exception_policy() == super::super::ExceptionPolicy::default()
                    && backedge.pre_fold_extra_instructions() == 0
                    && backedge.local_policy.hidden_exception_owner_slots() == 0
                    && backedge.depth_after().is_some()
                    && backedge.depth_after() == jump.depth_after()
                    && jump_targets[entry.index()].is_some()
                    && jump_targets[target.index()].is_none())
                .then_some(source_index)
            })
            .collect::<FxHashSet<_>>();
        // A retained source NOP does not block a located, original loop backedge.
        // Prove the source's authored break slot or conditional false entry, then
        // require a clean single-jump target with the same original exception owner.
        // The usual threading path keeps the source line and accounts for the copied slot.
        let retained_backedge_sources = if !SOURCE_GATES
            || (has_retained_forward_nop
                && has_original_boundaries
                && !self.conditional_threading_targets.is_empty())
        {
            forward_jump_positions.as_slice()
        } else {
            &[]
        };
        let retained_located_backedges = retained_backedge_sources
            .iter()
            .filter_map(|&(source_index, (block, offset))| {
                let items = graph[block].items();
                let Item::Instruction(jump) = items[offset] else {
                    return None;
                };
                let Operand::Forward(entry) = jump.operand() else {
                    return None;
                };
                let source_labels = &items[..offset];
                if jump.opcode != JUMP_FORWARD
                    || !jump.jump_policy.always_retains_nop()
                    || jump.location.line < 0
                    || jump.depth_after().is_none()
                    || offset + 1 != items.len()
                    || source_labels.iter().any(|item| !matches!(item, Item::Label(_)))
                {
                    return None;
                }
                let mut original = Instruction::new(
                    JUMP_FORWARD, Operand::Forward(entry), jump.location, jump.depth_after(),
                );
                original.jump_policy.retain_nop();
                if jump.fusion_policy.is_authored_break() {
                    original.fusion_policy.mark_authored_break();
                    original.jump_policy.prevent_threading_target();
                    original.add_pre_fold_extra_instructions(1);
                    if !source_labels.iter().any(|item| matches!(item, Item::Label(label)
                        if self.original_nop_targets.contains(label)
                            && self.exception_owner_provenance.source_boundaries.contains(label)
                            && self.exception_owner_provenance.block_boundaries.contains(label)))
                    {
                        return None;
                    }
                } else if !instruction_positions.iter().any(|&(source, at)| {
                    matches!(graph[source].items()[at], Item::Instruction(branch)
                        if branch.opcode == POP_JUMP_IF_FALSE
                            && branch.location == jump.location
                            && branch.depth_after() == jump.depth_after()
                            && matches!(branch.operand(), Operand::Forward(label)
                                if source_labels.contains(&Item::Label(label))
                                    && self.exception_owner_provenance.source_boundaries.contains(&label)
                                    && self.exception_owner_provenance.block_boundaries.contains(&label)
                                    && !self.original_nop_targets.contains(&label)))
                }) {
                    return None;
                }
                if jump != original {
                    return None;
                }
                let (Item::Instruction(backedge), labels) =
                    graph[graph.label_block(entry)?].items().split_last()?
                else {
                    return None;
                };
                let Operand::Backward(target) = backedge.operand() else {
                    return None;
                };
                let entry_index = *label_offsets.get(entry)?;
                let target_index = *label_offsets.get(target)?;
                if !(target_index < source_index && source_index < entry_index)
                    || backedge.location.line < 0
                    || backedge.location == jump.location
                    || *backedge != Instruction::new(
                        JUMP_BACKWARD, Operand::Backward(target), backedge.location, jump.depth_after(),
                    )
                    || !labels.contains(&Item::Label(entry))
                    || !self.exception_owner_provenance.block_boundaries.contains(&entry)
                    || self.conditional_threading_targets.get(&entry) != Some(&backedge.location)
                    || labels.iter().any(|item| match item {
                        Item::Instruction(_) => true,
                        Item::Label(label) => self.original_nop_targets.contains(label)
                            || self.borrow_block_aliases.contains(label)
                            || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            || self.exception_owner_provenance.omitted_nop_locations.contains_key(label),
                    })
                    || planned_sources.contains(&entry_index)
                    || pop_block_offsets.contains(&entry_index)
                    || jump_targets[entry.index()].is_none()
                    || jump_targets[target.index()].is_some()
                {
                    return None;
                }
                let mut source_regions = self.exception_regions.iter().zip(&region_ranges)
                    .filter(|(_, range)| range.contains(&source_index));
                let mut target_regions = self.exception_regions.iter().zip(&region_ranges)
                    .filter(|(_, range)| range.contains(&entry_index));
                let same_owner = match (source_regions.next(), target_regions.next()) {
                    (None, None) => true,
                    (Some((source, _)), Some((target, _))) => {
                        !jump.fusion_policy.is_authored_break()
                            && !source.is_scope_fallback
                            && !target.is_scope_fallback
                            && !source.preserve_lasti
                            && !target.preserve_lasti
                            && source.target == target.target
                            && source.depth == target.depth
                            && matches!((source.priority, target.priority), (
                                super::super::ExceptionRegionPriority::Resolved { start: a, end: b },
                                super::super::ExceptionRegionPriority::Resolved { start: c, end: d },
                            ) if a == c && b == d)
                    }
                    _ => false,
                };
                (same_owner && source_regions.next().is_none() && target_regions.next().is_none())
                    .then_some(source_index)
            })
            .collect::<FxHashSet<_>>();
        // The original loop exit still contains its try POP_BLOCK when a final
        // authored break enters it. Keep that edge on the certified empty array:
        // bypassing it would both strand the break's line and enter its cold
        // borrowed-load continuation. Other jumps keep their existing threading.
        let original_try_break_sources = if !SOURCE_GATES || !normal_try_exits.is_empty() {
            original_authored_forward_jump_positions
        } else {
            &[]
        };
        let original_break_empty_try_exits = original_try_break_sources
            .iter()
            .filter_map(|&(source_index, (block, offset))| {
                let items = graph[block].items();
                let Item::Instruction(jump) = items[offset] else {
                    return None;
                };
                let Operand::Forward(entry) = jump.operand() else {
                    return None;
                };
                let mut original = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(entry),
                    jump.location,
                    Some(0),
                );
                original.fusion_policy.mark_authored_break();
                original.jump_policy.retain_nop_for_direct_target();
                original.jump_policy.prevent_threading_target();
                if jump != original
                    || jump.location.line < 0
                    || offset + 1 != items.len()
                    || items[..offset]
                        .iter()
                        .any(|item| !matches!(item, Item::Label(_)))
                {
                    return None;
                }
                let entry_index = *label_offsets.get(entry)?;
                if entry_index != source_index + 1 || !normal_try_exits.contains(&entry_index) {
                    return None;
                }
                let target_items = graph[graph.label_block(entry)?].items();
                let (Item::Instruction(exit), labels) = target_items.split_last()? else {
                    return None;
                };
                let Operand::Forward(target) = exit.operand() else {
                    return None;
                };
                let mut original_exit = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(target),
                    jump.location,
                    Some(0),
                );
                original_exit.jump_policy.mark_borrow_empty_block();
                original_exit
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                if *exit != original_exit
                    || *label_offsets.get(target)? <= entry_index
                    || labels.len() < 2
                    || !labels.iter().all(|item| matches!(item, Item::Label(label)
                        if !self.original_nop_targets.contains(label)
                            && !self.borrow_block_aliases.contains(label)
                            && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label)))
                    || !labels.iter().any(|item| matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.source_boundaries.contains(label)
                            && self.exception_owner_provenance.block_boundaries.contains(label)))
                    || !self.exception_owner_provenance.normal_try_exits.iter().any(|(start, end)| {
                        *end == target && labels.contains(&Item::Label(*start))
                    })
                {
                    return None;
                }
                Some(source_index)
            })
            .collect::<FxHashSet<_>>();
        // Retaining an authored break as a NOP does not retain the owner of a
        // normal try's POP_BLOCK/JUMP array copied onto that edge. Certify the
        // original sole entry before reusing the existing owner-transfer path;
        // keep the break's target, NOP retention, and copying policies unchanged.
        let try_owner_break_sources = if !SOURCE_GATES
            || (self.exception_regions.len() >= 2
                && !(normal_try_exits.is_empty() && compound_try_owner_exits.is_empty()))
        {
            authored_forward_jump_positions
        } else {
            &[]
        };
        let retained_break_try_owners = try_owner_break_sources
            .iter()
            .filter_map(|&(source_index, (block, offset))| {
                let source_items = graph[block].items();
                let Item::Instruction(jump) = source_items[offset] else {
                    return None;
                };
                let Operand::Forward(entry) = jump.operand() else {
                    return None;
                };
                let mut original = Instruction::new(
                    JUMP_FORWARD, Operand::Forward(entry), jump.location, Some(0),
                );
                original.fusion_policy.mark_authored_break();
                original.jump_policy.retain_nop();
                original.jump_policy.prevent_threading_target();
                if jump != original
                    || jump.location.line < 0
                    || offset == 0
                    || offset + 1 != source_items.len()
                    || source_items[..offset].iter().any(|item| !matches!(item, Item::Label(_)))
                    || planned_sources.contains(&source_index)
                {
                    return None;
                }
                let target_index = *label_offsets.get(entry)?;
                if target_index <= source_index
                    || !(normal_try_exits.contains(&target_index)
                        || compound_try_owner_exits.contains(&target_index))
                {
                    return None;
                }
                let target_block = graph.label_block(entry)?;
                let (Item::Instruction(exit), aliases) = graph[target_block].items().split_last()?
                else {
                    return None;
                };
                let Operand::Forward(target) = exit.operand() else {
                    return None;
                };
                let mut original_exit = Instruction::new(
                    JUMP_FORWARD, Operand::Forward(target), SourceLocation::NONE, Some(0),
                );
                original_exit.jump_policy.prevent_small_exit_inlining();
                if *exit != original_exit
                    || aliases.is_empty()
                    || aliases.iter().any(|item| !matches!(item, Item::Label(label)
                        if !self.original_nop_targets.contains(label)
                            && !self.borrow_block_aliases.contains(label)
                            && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label)))
                    || *label_offsets.get(target)? <= target_index
                    || self.exception_regions.iter().any(|region| {
                        graph.label_block(region.target) == Some(target_block)
                    })
                {
                    return None;
                }
                // All aliases must have one original jump predecessor and no
                // physical fallthrough. An unrelated donor cannot move an owner.
                let mut incoming = instruction_positions.iter().enumerate().filter_map(
                    |(index, &(source, at))| {
                        let Item::Instruction(instruction) = graph[source].items()[at] else {
                            return None;
                        };
                        match instruction.operand() {
                            Operand::Forward(label) | Operand::Backward(label)
                                if graph.label_block(label) == Some(target_block) => Some(index),
                            _ => None,
                        }
                    },
                );
                if incoming.next() != Some(source_index)
                    || incoming.next().is_some()
                    || graph.order().windows(2).any(|pair| {
                        pair[1] == target_block
                            && super::super::block_has_fallthrough(graph[pair[0]].items())
                    })
                {
                    return None;
                }
                let mut source_regions = region_ranges.iter().enumerate()
                    .filter_map(|(index, range)| range.contains(&source_index).then_some(index));
                let (Some(first), Some(second), None) =
                    (source_regions.next(), source_regions.next(), source_regions.next())
                else {
                    return None;
                };
                let mut target_regions = region_ranges.iter().enumerate()
                    .filter_map(|(index, range)| range.contains(&target_index).then_some(index));
                let (Some(outer), None) = (target_regions.next(), target_regions.next()) else {
                    return None;
                };
                let inner = if outer == first { second } else if outer == second { first } else {
                    return None;
                };
                if [inner, outer].iter().any(|&index| {
                    let region = &self.exception_regions[index];
                    region.is_scope_fallback || region.preserve_lasti || region.depth != 0
                }) || !aliases.contains(&Item::Label(self.exception_regions[inner].end))
                {
                    return None;
                }
                Some((source_index, target_index))
            })
            .collect::<Vec<_>>();
        // The named-handler clear sequence can enter an original outer try's
        // unlocated normal-exit jump. That exact copy drops the source's ordinary
        // owner and inherits the target's sole scope fallback, including its policy.
        let scope_fallback_sources = if !SOURCE_GATES
            || (!normal_try_exits.is_empty()
                && self.exception_regions.len() >= 2
                && self
                    .exception_regions
                    .iter()
                    .any(|region| region.is_scope_fallback))
        {
            forward_jump_positions.as_slice()
        } else {
            &[]
        };
        let named_handler_scope_fallbacks = scope_fallback_sources
            .iter()
            .filter_map(|&(source_index, (block, offset))| {
                let items = graph[block].items();
                let Item::Instruction(jump) = items[offset] else {
                    return None;
                };
                let Operand::Forward(entry) = jump.operand() else {
                    return None;
                };
                let mut original = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(entry),
                    jump.location,
                    Some(0),
                );
                original.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                if jump != original || jump.location.line < 0 || offset + 1 != items.len() {
                    return None;
                }
                let (labels, cleanup) = items[..offset].split_at(offset.checked_sub(4)?);
                let [
                    Item::Instruction(pop),
                    Item::Instruction(none),
                    Item::Instruction(store),
                    Item::Instruction(delete),
                ] = cleanup
                else {
                    return None;
                };
                let (Operand::Value(none_index), Operand::Value(name)) =
                    (none.operand(), store.operand())
                else {
                    return None;
                };
                let expected = [
                    Instruction::new(
                        crate::target::opcodes::POP_EXCEPT,
                        Operand::Value(0),
                        jump.location,
                        Some(0),
                    ),
                    Instruction::new(
                        LOAD_CONST,
                        Operand::Value(none_index),
                        jump.location,
                        Some(1),
                    ),
                    Instruction::new(STORE_FAST, Operand::Value(name), jump.location, Some(0)),
                    Instruction::new(
                        crate::target::opcodes::DELETE_FAST,
                        Operand::Value(name),
                        jump.location,
                        Some(0),
                    ),
                ];
                if labels.is_empty()
                    || labels.iter().any(|item| !matches!(item, Item::Label(_)))
                    || [*pop, *none, *store, *delete] != expected
                {
                    return None;
                }
                let target_index = *label_offsets.get(entry)?;
                let (Item::Instruction(exit), target_labels) =
                    graph[graph.label_block(entry)?].items().split_last()?
                else {
                    return None;
                };
                let Operand::Forward(target) = exit.operand() else {
                    return None;
                };
                let expected_exit = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(target),
                    SourceLocation::NONE,
                    Some(0),
                );
                let mut empty_exit = expected_exit;
                // A nested recovery leaves the normal exit's original array empty.
                // Its borrowing marker does not change the copied exception policy.
                empty_exit.jump_policy.mark_borrow_empty_block();
                if !normal_try_exits.contains(&target_index)
                    || (*exit != expected_exit && *exit != empty_exit)
                    || !target_labels.contains(&Item::Label(entry))
                    || target_labels.iter().any(|item| match item {
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
                    return None;
                }
                let mut target_regions = self
                    .exception_regions
                    .iter()
                    .zip(&region_ranges)
                    .filter(|(_, range)| range.contains(&target_index));
                let (fallback, fallback_range) = target_regions.next()?;
                if target_regions.next().is_some()
                    || !fallback.is_scope_fallback
                    || fallback.depth != 0
                    || !fallback.preserve_lasti
                    || !fallback_range.contains(&source_index)
                {
                    return None;
                }
                let mut source_regions =
                    self.exception_regions
                        .iter()
                        .zip(&region_ranges)
                        .filter(|(region, range)| {
                            !region.is_scope_fallback && range.contains(&source_index)
                        });
                let (outer, _) = source_regions.next()?;
                if source_regions.next().is_some()
                    || outer.depth != 0
                    || outer.preserve_lasti
                    || !target_labels.contains(&Item::Label(outer.end))
                    || self
                        .exception_regions
                        .iter()
                        .zip(&region_ranges)
                        .filter(|(_, range)| range.contains(&source_index))
                        .count()
                        != 2
                {
                    return None;
                }
                Some((source_index, (jump, target_index)))
            })
            .collect::<FxHashMap<_, _>>();
        let authored_finally_break_targets = Self::take_authored_finally_break_threading_targets(
            &self.graph,
            &mut self.borrow_authored_finally_break_origins,
        );
        // Build this lazily for a match-success jump that needs it. A removed
        // original NOP can be recorded under any co-located label alias.
        let mut original_nop_offsets = None;
        let graph = &mut self.graph;
        let mut instruction_index = 0;
        let mut copied_owners = retained_break_try_owners;
        let mut threaded_borrow_origins = FxHashSet::default();
        let mut threaded_authored_finally_breaks = FxHashSet::default();
        graph.for_each_instruction_mut(|instruction| {
            let source_index = instruction_index;
            instruction_index += 1;
            // Cold ordering settles direction later; a backward-form opcode can still
            // have a forward target here, including an original handler break.
            if original_break_empty_try_exits.contains(&source_index) {
                if !planned_sources.contains(&source_index) {
                    // This authored break is redundant against its next original
                    // array. Keep its own line slot while the equal-line try exit
                    // remains an analysis-only borrowing barrier.
                    instruction.jump_policy.retain_nop();
                    instruction.jump_policy.preserve_explicit_nop();
                }
                return;
            }
            if planned_sources.contains(&source_index)
                || !is_unconditional_jump(instruction.opcode)
                || (instruction.jump_policy.always_retains_nop()
                    && !retained_break_backedges.contains(&source_index)
                    && !retained_located_backedges.contains(&source_index))
            {
                return;
            }
            let Operand::Forward(mut target) = instruction.operand() else {
                return;
            };
            let original_target = target;
            let original_jump = *instruction;
            let nested_join_origins = nested_join_sources.get(&source_index);
            let mut nested_join_target_is_clean = true;
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
                    next_original_nop,
                )) = jump_targets.get(target.index()).copied().flatten()
                else {
                    break;
                };
                if self.python_version >= crate::PythonVersion::Py314
                    && original_jump.jump_policy.is_original_match_success()
                    && original_jump.location == SourceLocation::NONE
                    && original_jump.jump_policy.retained_nop()
                        == super::super::ir::RetainedJumpNop::DirectTarget
                    && !self.original_nop_targets.is_empty()
                    && label_offsets.get(target).is_some_and(|offset| {
                        original_nop_offsets
                            .get_or_insert_with(|| {
                                self.original_nop_targets
                                    .iter()
                                    .filter_map(|label| label_offsets.get(*label).copied())
                                    .collect::<FxHashSet<_>>()
                            })
                            .contains(offset)
                    })
                {
                    // A recorded original NOP may still lead this target; retaining
                    // the direct edge avoids threading past its source-line boundary.
                    break;
                }
                if original_jump.jump_policy.is_original_empty_join()
                    && original_jump.jump_policy.retained_nop()
                        == super::super::ir::RetainedJumpNop::DirectTarget
                    && next_original_nop
                    && label_offsets
                        .get(target)
                        .is_some_and(|target| source_index < *target)
                {
                    // The earlier assertion-success edge still sees the authored NOP
                    // at this forward target. Its later removal must not expose the jump.
                    break;
                }
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
                if nested_join_origins.is_some() {
                    // An authored NOP belongs to the entered original array even if
                    // lowering removed it. Threading past it cannot certify the
                    // nested join as an original unlocated eval-check exit.
                    nested_join_target_is_clean &= !self.original_nop_targets.contains(&target)
                        && !self
                            .exception_owner_provenance
                            .omitted_nop_counts
                            .contains_key(&target)
                        && !self
                            .exception_owner_provenance
                            .omitted_nop_locations
                            .contains_key(&target);
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
                instruction.jump_policy.mark_interruptible_after_threading();
            }
            if retargeted
                && nested_join_target_is_clean
                && (instruction.opcode == JUMP_BACKWARD
                    || instruction.jump_policy.is_interruptible())
                && let Some(origins) = nested_join_origins
            {
                // This exact original join acquired the threaded exit's eval check.
                // Its private copies and sole survivor now inherit their own donors.
                self.shared_jump_copy_location_blocks.extend(
                    origins
                        .iter()
                        .filter_map(|(entry, origin)| (*origin == original_jump).then_some(*entry)),
                );
            }
            if authored_break_copy_nop
                && threaded_jump_count == 1
                && (crosses_normal_try_exit
                    || folded_loop_return_heads.contains(&original_target)
                    || (unlocated_return_join_heads.contains(&original_target)
                        && region_ranges
                            .iter()
                            .all(|range| !range.contains(&source_index)))
                    || original_break_return_bridges
                        .get(&source_index)
                        .or_else(|| folded_loop_raise_breaks.get(&source_index))
                        .is_some_and(|(original, end)| {
                            *original == original_jump && *end == target
                        }))
                && matches!(operand, Operand::Forward(_))
                && !instruction.jump_policy.is_interruptible()
            {
                // A break's direct copy request survives the original POP_BLOCK/JUMP
                // or unlocated return join. The copier still decides whether the target fits,
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
                if threaded_jump_count == 1
                    && named_handler_scope_fallbacks.get(&source_index)
                        == Some(&(original_jump, target_index))
                {
                    // The actual one-hop replacement has the captured target's owner.
                    // A blanket source exclusion must not mask that transported fallback.
                    instruction.set_exception_policy(ExceptionPolicy::default());
                }
                copied_owners.push((source_index, target_index));
            }
            if artificial_eval_break
                && instruction.opcode == JUMP_FORWARD
                && threaded_jump_count == 0
                && copied_finally_iterator_breaks.get(&source_index) != Some(&original_jump)
            {
                // An untouched ordinary head still threads through the original-head path.
                // Only replacement by an early copy leaves the hidden NOP prefix that needs
                // separate predecessor-location clones during the later exit-copy stage.
                instruction.no_location_policy.clear_artificial_eval_break();
            } else if artificial_eval_break {
                instruction.no_location_policy.mark_artificial_eval_break();
            }
            // A captured authored break still enters its exact nonempty finally
            // array after an intermediate jump-only array empties. Its original
            // source and surviving cleanup are checked again before borrowing.
            if (crosses_empty_block
                || (crosses_fallthrough_barrier && original_borrow_route != Some(target)))
                && authored_finally_break_targets.get(&source_index) != Some(&target)
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
                let start = Label::from_u32(self.next_label);
                let end = Label::from_u32(self.next_label + 1);
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
        // A guard's originally empty join can carry its false-edge line early.
        // Its cleanup predecessor still threads past that join before line
        // propagation. Preserve the original two donor paths to a manager POP;
        // the retained guard NOP must not turn them into one cleanup-line donor.
        let recorded_guard_join = |source: BasicBlockId, jump: &Instruction| {
            if jump.location.line < 0
                || !jump.jump_policy.is_original_empty_join()
                || !jump.jump_policy.preserves_nop_after_threading()
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
            {
                return false;
            }
            let Some(predecessors) = incoming.get(&source) else {
                return false;
            };
            let [first, second] = predecessors.as_slice() else {
                return false;
            };
            let (Some(first), Some(second)) = (
                graph[*first].last_instruction(),
                graph[*second].last_instruction(),
            ) else {
                return false;
            };
            let (guard, cleanup) = if first.opcode == POP_JUMP_IF_NONE {
                (first, second)
            } else {
                (second, first)
            };
            guard.opcode == POP_JUMP_IF_NONE
                && guard.location == jump.location
                && guard.local_policy.hidden_exception_owner_slots() == 3
                && self
                    .exception_owner_provenance
                    .source_none_conditions
                    .iter()
                    .filter(|origin| origin.instruction == *guard)
                    .count()
                    == 1
                && cleanup.opcode == JUMP_FORWARD
                && cleanup.location.line >= 0
                && cleanup.local_policy.hidden_exception_owner_slots() == 0
                && guard.operand() != cleanup.operand()
                && guard.depth_after().is_some()
                && guard.depth_after() == cleanup.depth_after()
                && guard.depth_after() == jump.depth_after()
                && [guard, cleanup].iter().all(|instruction| {
                    instruction.pre_fold_extra_instructions() == 0
                        && instruction.exception_policy() == ExceptionPolicy::default()
                        && !instruction.no_location_policy.preserves_location()
                        && !instruction.fusion_policy.is_authored_break()
                        && instruction.jump_policy.inline_small_exit()
                        && matches!(instruction.operand(), Operand::Forward(label)
                            if graph.label_block(label) == Some(source)
                                && self.exception_owner_provenance.source_boundaries.contains(&label)
                                && self.exception_owner_provenance.block_boundaries.contains(&label))
                })
        };
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
                        && instructions.next().is_none()
                        && positions[&source] < positions[&block]
                        && (jump.location.line < 0 || recorded_guard_join(source, jump))
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
    pub(in crate::assembler) fn record_borrow_empty_jump_heads(&mut self) -> bool {
        self.record_borrow_empty_jump_heads_with_preflight::<true>()
    }

    fn borrow_empty_jump_head_target(&self, items: &[Item]) -> Option<Label> {
        let (Item::Instruction(jump), labels) = items.split_last()? else {
            return None;
        };
        if jump.opcode != JUMP_FORWARD
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
        let Operand::Forward(target) = jump.operand() else {
            return None;
        };
        labels
            .iter()
            .all(|item| {
                matches!(item, Item::Label(label)
                    if !self.original_nop_targets.contains(label)
                        && !self.preserved_block_boundaries.contains(label))
            })
            .then_some(target)
    }

    fn record_borrow_empty_jump_heads_with_preflight<const PREFLIGHT: bool>(&mut self) -> bool {
        // Every admitted original array ends in a forward unconditional jump.
        if !self.graph.features().has_forward_jump() {
            return false;
        }
        let graph = &self.graph;
        if PREFLIGHT
            && !graph
                .iter()
                .any(|(_, block)| self.borrow_empty_jump_head_target(block.items()).is_some())
        {
            return false;
        }
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
        let normal_try_exits = if self.python_version == crate::PythonVersion::Py314 {
            self.exception_owner_provenance
                .normal_try_exits
                .iter()
                .copied()
                .collect::<FxHashSet<_>>()
        } else {
            FxHashSet::default()
        };
        let track_normal_try_exits = !normal_try_exits.is_empty();
        let mut conditional_blocked_non_fallthrough = if track_normal_try_exits {
            blocked.clone()
        } else {
            FxHashSet::default()
        };
        let mut fallthrough_entries = FxHashSet::default();
        for blocks in order.windows(2) {
            if super::super::block_has_fallthrough(graph[blocks[0]].items()) {
                blocked.insert(blocks[1]);
                if track_normal_try_exits {
                    fallthrough_entries.insert(blocks[1]);
                }
            }
        }
        let mut incoming = FxHashSet::default();
        let mut conditional_incoming = FxHashMap::<_, usize>::default();
        let mut conditional_sources = FxHashMap::default();
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
                    if track_normal_try_exits {
                        conditional_sources.insert(target, source);
                    }
                } else {
                    conditional_blocked.insert(target);
                    if track_normal_try_exits {
                        conditional_blocked_non_fallthrough.insert(target);
                    }
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
                    // A normal-try exit may be the common target of a conditional
                    // and its uninterrupted fallthrough. If its sole jump vanishes,
                    // 3.14's borrowing traversal stops at that empty original array.
                    || (track_normal_try_exits
                        && conditional_incoming.get(block).copied().unwrap_or_default() == 1
                        && fallthrough_entries.contains(block)
                        && !conditional_blocked_non_fallthrough.contains(block)
                        && conditional_sources.get(block).is_some_and(|source| {
                            let source_index = positions[source];
                            let target_index = positions[block];
                            let exit = self.borrow_empty_jump_head_target(graph[*block].items());
                            let recorded_exit = exit.is_some_and(|target| {
                                graph[*block].items().iter().any(|item| {
                                    matches!(item, Item::Label(entry)
                                        if normal_try_exits.contains(&(*entry, target)))
                                })
                            });
                            recorded_exit
                                && order[source_index..target_index].iter().enumerate().all(
                                    |(offset, path_block)| {
                                        original_reachable[path_block.index()]
                                            && super::super::block_has_fallthrough(graph[*path_block].items())
                                            && (offset == 0
                                                || graph[*path_block].items().iter().all(|item| {
                                                    !matches!(item, Item::Instruction(instruction)
                                                        if instruction.opcode.has_jump())
                                                }))
                                    },
                                )
                        }))
            })
            .filter_map(|block| {
                let items = graph[block].items();
                let label = self.borrow_empty_jump_head_target(items)?;
                let target = graph.label_block(label)?;
                (positions[&block] < positions[&target])
                    .then_some(ItemPosition::new(block, items.len() - 1))
            })
            .collect::<Vec<_>>();
        let changed = !heads.is_empty();
        for head in heads {
            // Threading transfers this provenance to each predecessor jump. If that
            // jump disappears later, its barrier remains at the original fallthrough
            // site even when the emptied target's labels move with cold handlers.
            // Shared conditional entries keep their original head unlocated. Removing
            // that head's jump must also retain its empty borrowing boundary.
            self.graph.mark_borrow_empty_block(head);
        }
        changed
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
        self.record_original_jump_heads_with_preflight::<true>();
    }

    fn is_original_jump_head(instruction: &Instruction) -> bool {
        (instruction.opcode == JUMP_BACKWARD
            || (instruction.opcode == JUMP_FORWARD && instruction.jump_policy.is_interruptible()))
            && instruction.pre_fold_extra_instructions() == 0
    }

    fn record_original_jump_heads_with_preflight<const PREFLIGHT: bool>(&mut self) {
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
        if !self.graph.features().has_pop_conditional()
            || (PREFLIGHT
                && !self.graph.iter().any(|(_, block)| {
                    block
                        .items()
                        .iter()
                        .find_map(|item| match item {
                            Item::Instruction(instruction) => Some(instruction),
                            Item::Label(_) => None,
                        })
                        .is_some_and(Self::is_original_jump_head)
                }))
        {
            return;
        }
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
                        if Self::is_original_jump_head(instruction) {
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

    /// Keeps distinct condition exits through an authored match-success jump.
    fn prepare_original_match_continuation_exits(&mut self) {
        if self
            .exception_owner_provenance
            .original_match_join_boundaries
            .is_empty()
        {
            return;
        }
        // Resolve all incoming edges by original array, including its aliases.
        let mut incoming = FxHashMap::default();
        for (source, array) in self.graph.iter() {
            for item in array.items() {
                if let Item::Instruction(instruction) = item
                    && let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                    && let Some(block) = self.graph.label_block(label)
                {
                    incoming.entry(block).or_insert_with(Vec::new).push((
                        source,
                        label,
                        *instruction,
                    ));
                }
            }
        }
        let clean_label = |item: &Item| {
            matches!(item, Item::Label(label)
                if self.exception_owner_provenance.block_boundaries.contains(label)
                    && !self.original_nop_targets.contains(label)
                    && !self.borrow_block_aliases.contains(label)
                    && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                    && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label))
        };
        let order = self.graph.order();
        let exits = incoming
            .into_iter()
            .filter_map(|(block, incoming)| {
                let mut conditions = incoming.iter().filter(|(_, _, instruction)| {
                    instruction.opcode != JUMP_FORWARD
                });
                let &(source, label, condition) = conditions.next()?;
                if condition.opcode != POP_JUMP_IF_FALSE || conditions.next().is_some() {
                    return None;
                }
                let (Item::Instruction(jump), labels) = self.graph[block].items().split_last()? else {
                    return None;
                };
                if jump.opcode != JUMP_FORWARD
                    || jump.jump_policy.is_interruptible()
                    || !jump.jump_policy.allows_threading_target()
                    || !labels.iter().all(clean_label)
                    || labels.iter().any(|item| {
                        matches!(item, Item::Label(label) if self.original_jump_head_targets.contains(label))
                    })
                {
                    return None;
                }
                let Operand::Forward(target_label) = jump.operand() else { return None; };
                let target = self.graph.label_block(target_label)?;
                let (Item::Instruction(success), labels) = self.graph[target].items().split_last()? else {
                    return None;
                };
                if success.opcode != JUMP_FORWARD
                    || !success.jump_policy.is_original_match_success()
                    || !success.jump_policy.allows_threading_target()
                    || !labels.iter().all(clean_label)
                {
                    return None;
                }
                let Operand::Forward(end) = success.operand() else { return None; };
                let continuation = self.graph.label_block(end)?;
                // The existing return-copy certificates handle small return targets.
                // This route requires an original match end with a live continuation.
                if !self.exception_owner_provenance.original_match_join_boundaries.contains(&end)
                    || ends_scope(self.graph[continuation].last_instruction()?.opcode)
                    || [block, target].contains(&continuation)
                    || incoming.iter().map(|(_, _, instruction)| instruction)
                        .chain([jump, success]).any(|instruction| {
                            instruction.location.line < 0
                                || !matches!(instruction.operand(), Operand::Forward(_))
                                || instruction.pre_fold_extra_instructions() != 0
                                || instruction.local_policy.hidden_exception_owner_slots() != 0
                                || instruction.no_location_policy.preserves_location()
                                || instruction.exception_policy() != super::super::ExceptionPolicy::default()
                                || instruction.depth_after() != jump.depth_after()
                        })
                {
                    return None;
                }
                let position = order.iter().position(|candidate| *candidate == block)?;
                let previous = *order.get(position.checked_sub(1)?)?;
                let last = self.graph[previous].last_instruction()?;
                let fallthrough = super::super::block_has_fallthrough(self.graph[previous].items());
                if last.depth_after() != jump.depth_after()
                    || if fallthrough {
                        last.opcode != POP_TOP
                            || last.location != jump.location
                            || condition.location.line >= jump.location.line
                            || self.conditional_threading_targets.contains_key(&label)
                    } else {
                        incoming.len() != 1
                            || last.opcode != RAISE_VARARGS
                            || !jump.jump_policy.is_original_empty_join()
                            || jump.location != condition.location
                            || self.conditional_threading_targets.get(&label) != Some(&condition.location)
                    }
                {
                    return None;
                }
                // Unrelated protected regions may exist elsewhere in the function.
                // Every participating original array must stay outside their ranges,
                // boundaries and handler entries, including ordinary alias predecessors.
                if !self.exception_regions.iter().all(|region| {
                    let Some(start) = self.graph.label_block(region.start)
                        .and_then(|block| order.iter().position(|candidate| *candidate == block))
                    else { return false; };
                    let Some(end) = self.graph.label_block(region.end)
                        .and_then(|block| order.iter().position(|candidate| *candidate == block))
                    else { return false; };
                    let Some(handler) = self.graph.label_block(region.target) else { return false; };
                    start <= end && [source, previous, block, target, continuation].into_iter()
                        .chain(incoming.iter().map(|(source, _, _)| *source))
                        .all(|block| block != handler && !order[start..=end].contains(&block))
                }) {
                    return None;
                }
                Some((label, block, previous, *jump, condition.location, fallthrough))
            })
            .collect::<Vec<_>>();
        for (label, block, previous, jump, condition_location, fallthrough) in exits {
            if fallthrough {
                self.graph.append_to_block(
                    previous,
                    vec![Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        jump.operand(),
                        jump.location,
                        jump.depth_after(),
                    ))],
                );
                if let Some(jump) = self.graph.instructions_mut(block).next_back() {
                    jump.location = condition_location;
                }
            }
            // The raising arm already has its separate false-edge array. The call
            // arm now has one too. Keep conditional entries while letting ordinary
            // alias predecessors retain their existing unconditional threading.
            self.conditional_threading_targets.remove(&label);
        }
    }

    /// Preserves separate branch locations when a match joins another branch exit.
    pub(in crate::assembler) fn prepare_original_conditional_jump_targets(&mut self) {
        self.prepare_original_match_continuation_exits();
        if self.conditional_threading_targets.is_empty() {
            return;
        }
        let order = self.graph.order().to_vec();
        let mut shared_match_donors = Vec::new();
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
                let previous_falls_through =
                    super::super::block_has_fallthrough(self.graph[previous].items());
                let last = self.graph[previous].last_instruction()?;
                let terminating_case_exit = !previous_falls_through
                    && ends_scope(last.opcode)
                    && jump.location.line >= 0
                    && jump.jump_policy.is_original_empty_join();
                // A nonterminating case arm and its false edge share one original
                // jump. Splitting it before small-exit copying duplicates the return.
                // Keep that array only for an untouched authored match join whose
                // final call falls through to an intact local or constant return.
                // A terminating arm leaves a certified false-edge array instead.
                // Both routes can reach a shared match end copied into a Pass arm;
                // its wildcard-call donor must wait until every copy is finished.
                if jump.opcode == JUMP_FORWARD
                    && ((previous_falls_through
                        && jump.location == SourceLocation::NONE
                        && last.opcode == POP_TOP
                        && last.location.line >= 0)
                        || terminating_case_exit)
                    && self.graph[block]
                        .items()
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count()
                        == 1
                    && self.graph[target]
                        .items()
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count()
                        == 1
                    && first.opcode == JUMP_FORWARD
                    && (first.jump_policy.is_original_match_success()
                        || (terminating_case_exit
                            && first.jump_policy.retains_nop_for(first.operand())))
                    && first.location.line >= 0
                    && [jump, first].iter().all(|instruction| {
                        instruction.pre_fold_extra_instructions() == 0
                            && instruction.local_policy.hidden_exception_owner_slots() == 0
                            && !instruction.no_location_policy.preserves_location()
                            && instruction.exception_policy()
                                == super::super::ExceptionPolicy::default()
                    })
                    && let Operand::Forward(return_label) = first.operand()
                    && let Some(return_block) = self.graph.label_block(return_label)
                    && (is_original_constant_return(&self.graph, return_label)
                        || (!terminating_case_exit
                            && super::normal_try_copies::original_local_return(
                                self.graph[return_block].items(),
                            )))
                    && let Some(entry) = self.graph[target].items().iter().find_map(|item| {
                        match item {
                            Item::Label(entry)
                                if self.exception_owner_provenance.original_match_join_boundaries.contains(entry)
                                    && !self.borrow_block_aliases.contains(entry) => Some(*entry),
                            _ => None,
                        }
                    })
                    && let Some(target_position) = order.iter().position(|block| *block == target)
                    && let Some(donor_block) = target_position.checked_sub(1).map(|index| order[index])
                    && super::super::block_has_fallthrough(self.graph[donor_block].items())
                    && let Some(donor) = self.graph[donor_block].last_instruction()
                    && donor.opcode == POP_TOP
                    && donor.location == first.location
                    // An unrelated comprehension cleanup does not split this local
                    // return. Keep every participating array outside protected ranges,
                    // their boundary blocks, and handler entries. Shared constant-return
                    // donation still requires the existing exception-free unit.
                    && (self.exception_regions.is_empty()
                        || (super::normal_try_copies::original_local_return(
                            self.graph[return_block].items(),
                        ) && self.exception_regions.iter().all(|region| {
                            let Some(start) = self.graph.label_block(region.start)
                                .and_then(|block| order.iter().position(|item| *item == block))
                            else {
                                return false;
                            };
                            let Some(end) = self.graph.label_block(region.end)
                                .and_then(|block| order.iter().position(|item| *item == block))
                            else {
                                return false;
                            };
                            let Some(handler) = self.graph.label_block(region.target) else {
                                return false;
                            };
                            start <= end
                                && [block, previous, target, donor_block, return_block]
                                    .iter()
                                    .all(|candidate| {
                                        *candidate != handler
                                            && !order[start..=end].contains(candidate)
                                    })
                        })))
                {
                    // The existing early passes clear only these two jump tags on an
                    // untouched forward head. Replay every other field literally.
                    let mut expected = *first;
                    expected.no_location_policy.clear_artificial_eval_break();
                    if expected.jump_policy.defers_redundant_removal() {
                        expected.jump_policy.resolve_deferred_removal();
                    }
                    shared_match_donors.push((entry, (expected, *donor)));
                    return None;
                }
                if !previous_falls_through {
                    return None;
                }
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
        self.exception_owner_provenance
            .shared_match_return_donors
            .extend(shared_match_donors);
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

    /// Threads an original Pass-handler exit through its copied-finally break head.
    fn thread_copied_finally_pass_breaks<const PREFLIGHT: bool>(
        &mut self,
    ) -> Vec<(Label, Instruction)> {
        if PREFLIGHT
            && (self.exception_regions.is_empty()
                || self.original_nop_targets.is_empty()
                || self
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .is_empty()
                || self
                    .exception_owner_provenance
                    .omitted_nop_locations
                    .is_empty()
                || !self
                    .conditional_threading_targets
                    .values()
                    .any(|location| *location == SourceLocation::NONE)
                || !self
                    .exception_regions
                    .iter()
                    .any(|region| region.preserve_lasti))
        {
            // An original Pass source, an unlocated copied-break head, and a lasti
            // handler ending at the source are all required by the checks below.
            #[cfg(test)]
            assert!(self.thread_copied_finally_pass_breaks::<false>().is_empty());
            return Vec::new();
        }
        let graph = &self.graph;
        let positions = graph
            .order()
            .iter()
            .copied()
            .enumerate()
            .map(|(position, block)| (block, position))
            .collect::<FxHashMap<_, _>>();
        let ranges = self
            .exception_regions
            .iter()
            .map(|region| {
                let start = positions[&graph.label_block(region.start).expect("region start")];
                let end = positions[&graph.label_block(region.end).expect("region end")];
                start..end
            })
            .collect::<Vec<_>>();
        let exits = graph
            .order()
            .iter()
            .copied()
            .filter_map(|source| {
                let source_items = graph[source].items();
                let [labels @ .., Item::Instruction(pop), Item::Instruction(jump)] = source_items
                else {
                    return None;
                };
                let depth = pop.depth_after()?;
                let Operand::Forward(entry) = jump.operand() else {
                    return None;
                };
                if pop.location.line < 0
                    || *pop != Instruction::new(
                        crate::target::opcodes::POP_EXCEPT,
                        Operand::Value(0),
                        pop.location,
                        Some(depth),
                    )
                    || *jump != Instruction::new(
                        JUMP_FORWARD, Operand::Forward(entry), pop.location, Some(depth),
                    )
                    || !labels.iter().all(|item| matches!(item, Item::Label(_)))
                    || !labels.iter().any(|item| matches!(item, Item::Label(label)
                        if self.original_nop_targets.contains(label)
                            && self.exception_owner_provenance.omitted_nop_counts.get(label) == Some(&1)
                            && self.exception_owner_provenance.omitted_nop_locations.get(label) == Some(&pop.location)))
                {
                    return None;
                }
                let target = graph.label_block(entry)?;
                let [aliases @ .., Item::Instruction(bridge)] = graph[target].items() else {
                    return None;
                };
                let Operand::Forward(destination) = bridge.operand() else {
                    return None;
                };
                let mut original_break = Instruction::new(
                    JUMP_FORWARD, Operand::Forward(destination), bridge.location, Some(depth),
                );
                original_break.fusion_policy.mark_authored_break();
                original_break.jump_policy.prevent_threading_target();
                if *bridge != original_break
                    || aliases.is_empty()
                    || !aliases.iter().all(|item| matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.block_boundaries.contains(label)
                            && self.exception_owner_provenance.source_boundaries.contains(label)
                            && !self.borrow_block_aliases.contains(label)
                            && !self.original_nop_targets.contains(label)
                            && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)))
                    || !aliases.iter().any(|item| matches!(item, Item::Label(label)
                        if self.conditional_threading_targets.get(label) == Some(&SourceLocation::NONE)))
                    || positions[&source] >= positions[&target]
                    || positions[&target] >= positions[&graph.label_block(destination)?]
                    || ranges.iter().any(|range| {
                        range.contains(&positions[&source]) || range.contains(&positions[&target])
                    })
                    || !self.exception_regions.iter().any(|region| {
                        region.preserve_lasti
                            && depth.checked_add(1) == Some(region.depth)
                            && labels.contains(&Item::Label(region.end))
                    })
                {
                    return None;
                }
                Some((
                    ItemPosition::new(source, source_items.len() - 1),
                    destination,
                    entry,
                    *bridge,
                ))
            })
            .collect::<Vec<_>>();
        let mut heads = Vec::with_capacity(exits.len());
        for (position, destination, entry, bridge) in exits {
            // The Pass marker precedes POP_EXCEPT; it does not obstruct the later
            // jump. Its target is an original jump-only alias after copied cleanup,
            // so retain the authored break's interrupt check when cold ordering moves
            // this handler behind the loop continuation. Keep the shared head intact.
            let jump = self.graph.instruction_mut(position).expect("handler exit");
            jump.retarget(Operand::Forward(destination));
            jump.jump_policy.mark_interruptible();
            heads.push((entry, bridge));
        }
        heads
    }

    /// Threads conditionals only through original ordinary jumps that survived exit copying.
    pub(in crate::assembler) fn thread_original_conditional_jump_targets(&mut self) -> bool {
        self.thread_original_conditional_jump_targets_with_preflight::<true>()
    }

    fn thread_original_conditional_jump_targets_with_preflight<const PREFLIGHT: bool>(
        &mut self,
    ) -> bool {
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
        let pass_break_heads = self.thread_copied_finally_pass_breaks::<PREFLIGHT>();
        if PREFLIGHT && targets.is_empty() && pass_break_heads.is_empty() {
            return false;
        }
        let mut changed = !pass_break_heads.is_empty();
        let mut threaded_condition = None;
        let origins = &mut self.exception_owner_provenance.threaded_condition_origins;
        let paired_none_prefixes = &mut self.exception_owner_provenance.copied_return_none_prefixes;
        let handler_none_prefixes = &mut self
            .exception_owner_provenance
            .copied_handler_return_none_prefixes;
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
                for (_, records) in &mut *handler_none_prefixes {
                    if records[5] == original_condition {
                        records[5] = *instruction;
                    }
                }
                threaded_condition = Some((*instruction, *operand != Operand::Forward(target)));
                changed = true;
            }
        });
        if !pass_break_heads.is_empty() {
            // Threading the guard and its Pass handler can leave the original break
            // bridge with one body predecessor. Its earlier shared donor is stale;
            // resolve the line from that one surviving edge, without relocating it.
            let graph = &self.graph;
            let heads = pass_break_heads
                .into_iter()
                .filter_map(|(entry, original)| {
                    let block = graph.label_block(entry)?;
                    (graph[block].last_instruction() == Some(&original))
                        .then_some((block, original))
                })
                .collect::<FxHashMap<_, _>>();
            let mut incoming = FxHashMap::<_, Vec<Option<SourceLocation>>>::default();
            for adjacent in graph.order().windows(2) {
                if heads.contains_key(&adjacent[1])
                    && super::super::block_has_fallthrough(graph[adjacent[0]].items())
                {
                    incoming.entry(adjacent[1]).or_default().push(None);
                }
            }
            for &source in graph.order() {
                for item in graph[source].items() {
                    let Item::Instruction(jump) = item else {
                        continue;
                    };
                    let (Operand::Forward(label) | Operand::Backward(label)) = jump.operand()
                    else {
                        continue;
                    };
                    let Some(target) = graph
                        .label_block(label)
                        .filter(|block| heads.contains_key(block))
                    else {
                        continue;
                    };
                    let donor = (jump.location.line >= 0
                        && *jump
                            == Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(label),
                                jump.location,
                                jump.depth_after(),
                            ))
                    .then_some(jump.location);
                    incoming.entry(target).or_default().push(donor);
                }
            }
            for region in &self.exception_regions {
                if let Some(target) = graph.label_block(region.target)
                    && heads.contains_key(&target)
                {
                    incoming.entry(target).or_default().push(None);
                }
            }
            let ordered_heads = self
                .graph
                .order()
                .iter()
                .copied()
                .filter(|head| heads.contains_key(head))
                .collect::<Vec<_>>();
            for head in ordered_heads {
                if let Some([Some(location)]) = incoming.get(&head).map(Vec::as_slice) {
                    self.graph
                        .instructions_mut(head)
                        .next_back()
                        .expect("certified break bridge")
                        .location = *location;
                }
            }
        }
        changed
    }

    /// Captures shared original jump arrays before threading or copying can merge their entries.
    pub(in crate::assembler) fn record_shared_unlocated_jump_entries(&mut self) {
        // Every admitted original array ends in a forward unconditional jump and
        // has multiple incoming POP_JUMP conditionals.
        if !self.graph.features().has_forward_jump() || !self.graph.features().has_pop_conditional()
        {
            return;
        }
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

    /// Keeps original match successes until their shared named cleanup is copied.
    fn original_named_match_cleanup_jumps(&self) -> FxHashSet<ItemPosition> {
        let mut jumps = FxHashSet::default();
        let mut entries = self
            .exception_owner_provenance
            .original_match_join_boundaries
            .iter()
            .copied()
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.as_u32());
        for entry in entries {
            let Some(block) = self.graph.label_block(entry) else {
                continue;
            };
            let items = self.graph[block].items();
            let head = items
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
                .unwrap_or(items.len());
            let aliases = &items[..head];
            if !aliases.contains(&Item::Label(entry))
                || aliases.iter().any(|item| match item {
                    Item::Label(label) => {
                        self.borrow_block_aliases.contains(label)
                            || self
                                .exception_regions
                                .iter()
                                .any(|region| region.target == *label)
                    }
                    Item::Instruction(_) => true,
                })
            {
                continue;
            }
            let [
                Item::Instruction(pop),
                Item::Instruction(load),
                Item::Instruction(store),
                Item::Instruction(delete),
                Item::Instruction(exit),
            ] = &items[head..]
            else {
                continue;
            };
            let Some(depth) = pop.depth_after() else {
                continue;
            };
            let Some(incoming_depth) = depth.checked_add(1) else {
                continue;
            };
            let (Operand::Value(constant), Operand::Value(name), Operand::Forward(end)) =
                (load.operand(), store.operand(), exit.operand())
            else {
                continue;
            };
            let plain = |opcode, operand, depth| {
                Instruction::new(opcode, operand, SourceLocation::NONE, Some(depth))
            };
            let mut expected_exit = plain(JUMP_FORWARD, Operand::Forward(end), depth);
            expected_exit.jump_policy.prevent_threading_target();
            expected_exit.allow_unconditional_block_inlining();
            if *pop != plain(POP_EXCEPT, Operand::Value(0), depth)
                || *load != plain(LOAD_CONST, Operand::Value(constant), incoming_depth)
                || *store != plain(STORE_FAST, Operand::Value(name), depth)
                || *delete != plain(DELETE_FAST, Operand::Value(name), depth)
                || *exit != expected_exit
                || aliases.contains(&Item::Label(end))
            {
                continue;
            }
            // The original match marker supplies identity. Full record equality rejects
            // retargeted, folded, or policy-modified successes before this first copy pass.
            let mut incoming = Vec::new();
            let mut intact = true;
            'incoming: for (source, contents) in self.graph.iter() {
                for (offset, item) in contents.items().iter().enumerate() {
                    let Item::Instruction(jump) = item else {
                        continue;
                    };
                    let (Operand::Forward(target) | Operand::Backward(target)) = jump.operand()
                    else {
                        continue;
                    };
                    if !aliases.contains(&Item::Label(target)) {
                        continue;
                    }
                    let mut expected = Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(entry),
                        jump.location,
                        Some(incoming_depth),
                    );
                    expected.jump_policy.mark_original_match_success();
                    expected.jump_policy.mark_interruptible();
                    if jump.location.line < 0 || *jump != expected {
                        intact = false;
                        break 'incoming;
                    }
                    incoming.push(ItemPosition::new(source, offset));
                }
            }
            if intact && incoming.len() >= 2 {
                jumps.extend(incoming);
            }
        }
        jumps
    }

    pub(in crate::assembler) fn remove_redundant_forward_jumps(
        &mut self,
        finalize_borrow_empty_blocks: bool,
    ) {
        let features = self.graph.features();
        if !features.has_jump() || (!finalize_borrow_empty_blocks && !features.has_forward_jump()) {
            return;
        }
        // These borrow markers disappear after local analysis if their policy
        // stays unchanged. Keep them in the graph for borrowing, while excluding
        // them from physical adjacency when no later jump can change that policy.
        let is_removable_borrow_marker = |instruction: &Instruction| {
            finalize_borrow_empty_blocks
                && instruction.opcode == NOP
                && instruction.jump_policy.borrow_empty_block()
                && (instruction.location.line < 0
                    || instruction.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::BarrierIfRemoved)
        };
        let mut target = None;
        let mut has_work = false;
        for item in self.graph.iter_items() {
            match item {
                Item::Label(label) if target == Some(*label) => {
                    has_work = true;
                    break;
                }
                Item::Label(_) => {}
                Item::Instruction(instruction) if is_removable_borrow_marker(instruction) => {}
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
        let named_match_cleanup_jumps = if finalize_borrow_empty_blocks {
            FxHashSet::default()
        } else {
            self.original_named_match_cleanup_jumps()
        };
        // A removed original POP_BLOCK can leave its entry and end labels in
        // separate empty graph blocks. Bind them to the next physical record;
        // renamed labels from copied cleanup arrays cannot certify this jump.
        let mut original_fallthrough_exits = Vec::new();
        if finalize_borrow_empty_blocks
            && !self
                .exception_owner_provenance
                .normal_try_fallthrough_donors
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
                    .normal_try_fallthrough_donors
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
                    original_fallthrough_exits.push((position, target, *pop));
                }
            }
        }
        // A compound try's normal jump crosses its own finally boundary. Bind
        // its source-authored wildcard donor before considering that boundary;
        // no ordinary try-copy plan is inferred from this separate certificate.
        let mut compound_fallthrough_exits = Vec::new();
        if finalize_borrow_empty_blocks
            && !self
                .exception_owner_provenance
                .compound_try_fallthrough_donors
                .is_empty()
        {
            let mut pending_labels = Vec::new();
            let mut preceding = [None, None];
            for &block in self.graph.order() {
                for (offset, item) in self.graph[block].items().iter().enumerate() {
                    match item {
                        Item::Label(label) => pending_labels.push(*label),
                        Item::Instruction(instruction) => {
                            for entry in pending_labels.drain(..) {
                                if instruction.opcode == JUMP_FORWARD
                                    && let Operand::Forward(target) = instruction.operand()
                                    && let [Some(call), Some(pop)] = preceding
                                    && self
                                        .exception_owner_provenance
                                        .compound_try_fallthrough_donors
                                        .get(&entry)
                                        == Some(&(call, pop))
                                    && self
                                        .exception_owner_provenance
                                        .compound_try_owner_exits
                                        .contains(&(entry, target))
                                    && self
                                        .graph
                                        .iter_items()
                                        .filter(|item| **item == Item::Label(entry))
                                        .count()
                                        == 1
                                    && self
                                        .graph
                                        .iter_items()
                                        .filter(|item| **item == Item::Label(target))
                                        .count()
                                        == 1
                                    && instruction.location == pop.location
                                    && instruction.depth_after() == pop.depth_after()
                                {
                                    compound_fallthrough_exits.push((
                                        ItemPosition::new(block, offset),
                                        target,
                                        pop,
                                    ));
                                }
                            }
                            preceding = [preceding[1], Some(*instruction)];
                        }
                    }
                }
            }
        }
        // Count only labels certified to enter the same original instruction array.
        // Two separate arrays must not become shared merely because later passes merge them.
        let mut shared_entries = FxHashMap::<Label, usize>::default();
        if !self.original_shared_unlocated_jump_entries.is_empty() {
            for item in self.graph.iter_items() {
                if let Item::Instruction(instruction) = item
                    && matches!(
                        instruction.opcode,
                        POP_JUMP_IF_FALSE
                            | POP_JUMP_IF_TRUE
                            | POP_JUMP_IF_NONE
                            | POP_JUMP_IF_NOT_NONE
                    )
                    && let Operand::Forward(target) = instruction.operand()
                    && let Some(origin) = self.original_shared_unlocated_jump_entries.get(&target)
                {
                    *shared_entries.entry(*origin).or_default() += 1;
                }
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
        // A later removed forward jump may acquire and transfer a barrier to the
        // preceding marker on a retry. Refuse to skip that marker when a forward
        // jump is its next instruction; labels do not separate these records.
        let mut maybe_promoted = FxHashSet::default();
        if finalize_borrow_empty_blocks {
            let mut follows_forward_jump = false;
            for block in graph.order().iter().copied().rev() {
                for (offset, item) in graph[block].items().iter().enumerate().rev() {
                    if let Item::Instruction(instruction) = item {
                        if follows_forward_jump {
                            maybe_promoted.insert(ItemPosition::new(block, offset));
                        }
                        follows_forward_jump = instruction.opcode == JUMP_FORWARD;
                    }
                }
            }
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
                    // CPython copies the unlocated cleanup before deleting this adjacent
                    // case jump. Preserve its interrupt check for the copied final jump.
                    if named_match_cleanup_jumps.contains(&position) {
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
                    let mut crossed_located_folded_marker = false;
                    let mut crossed_other_located_marker = false;
                    let mut crosses_exception_boundary = false;
                    let mut crosses_only_original_match_end = finalize_borrow_empty_blocks
                        && instruction.jump_policy.is_original_match_success()
                        && self
                            .exception_owner_provenance
                            .original_match_join_boundaries
                            .contains(&target)
                        && !self.borrow_block_aliases.contains(&target);
                    let mut original_cleanup_boundaries = 0;
                    let mut crosses_only_original_cleanup = true;
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
                                    if crosses_only_original_match_end
                                        && exception_boundaries.contains(label)
                                    {
                                        let mut ending = self
                                            .exception_regions
                                            .iter()
                                            .filter(|region| region.end == *label);
                                        let original = ending.next();
                                        crosses_only_original_match_end = *label == target
                                            && ending.next().is_none()
                                            && original.is_some_and(|region| {
                                                !region.is_scope_fallback
                                                    && !region.preserve_lasti
                                                    && Some(region.depth)
                                                        == instruction.depth_after()
                                            })
                                            && self.exception_regions.iter().all(|region| {
                                                region.start != *label && region.target != *label
                                            });
                                    }
                                    if finalize_borrow_empty_blocks
                                        && instruction.fusion_policy.is_authored_break()
                                        && exception_boundaries.contains(label)
                                    {
                                        original_cleanup_boundaries += 1;
                                        let mut ending = self
                                            .exception_regions
                                            .iter()
                                            .filter(|region| region.end == *label);
                                        let original = ending.next();
                                        crosses_only_original_cleanup &= *label != target
                                            && self.exception_owner_provenance.source_boundaries.contains(label)
                                            && self.exception_owner_provenance.block_boundaries.contains(label)
                                            && !self.borrow_block_aliases.contains(label)
                                            && !self.original_nop_targets.contains(label)
                                            && ending.next().is_none()
                                            && original.is_some_and(|original| {
                                                !original.is_scope_fallback
                                                    && original.preserve_lasti
                                                    && Some(original.depth) == instruction.depth_after()
                                                    && self.exception_regions.iter().all(|region| {
                                                        region.target != *label
                                                            && (region.start != *label
                                                                || (region.target == original.target
                                                                    && region.depth == original.depth
                                                                    && region.preserve_lasti == original.preserve_lasti
                                                                    && region.is_scope_fallback == original.is_scope_fallback
                                                                    && match (region.priority, original.priority) {
                                                                        (super::super::ExceptionRegionPriority::OriginalRange(a, b), super::super::ExceptionRegionPriority::OriginalRange(c, d)) => a == c && b == d,
                                                                        (super::super::ExceptionRegionPriority::Resolved { start: a, end: b }, super::super::ExceptionRegionPriority::Resolved { start: c, end: d }) => a == c && b == d,
                                                                        _ => false,
                                                                    }))
                                                    })
                                            });
                                    }
                                }
                                Item::Instruction(removed) => {
                                    if is_removable_borrow_marker(removed)
                                        && !(removed.location.line >= 0
                                            && removed.jump_policy.always_retains_nop()
                                            && maybe_promoted.contains(&ItemPosition::new(
                                                next_block,
                                                next_offset,
                                            )))
                                    {
                                        if removed.location.line >= 0 {
                                            if removed.location.line == instruction.location.line
                                                && removed.no_location_policy.borrow_fallthrough()
                                                    == BorrowFallthrough::BarrierIfRemoved
                                            {
                                                crossed_located_folded_marker = true;
                                            } else {
                                                crossed_other_located_marker = true;
                                            }
                                        }
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
                        // CPython compacts an empty block's removable NOP before
                        // redundant jumps. Its same-line successor keeps its own
                        // span, including an authored Pass donor.
                        let removable_empty_block_nop = instruction.opcode == NOP
                            && instruction.no_location_policy.borrow_fallthrough()
                                == BorrowFallthrough::BarrierIfRemoved
                            && !instruction.jump_policy.preserves_explicit_nop();
                        // A raising predecessor cannot supply another path's line.
                        (!matches!(instruction.opcode, RAISE_VARARGS | RERAISE)
                            && !removable_empty_block_nop)
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
                                    .chain(&self.exception_owner_provenance.guarded_try_else_exits)
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
                    // Its captured fallthrough donor belongs to the preceding array, so the
                    // late redundant jump becomes a NOP even with the same source line.
                    let preserves_original_fallthrough_nop = !absorbed_normal_try_exit
                        && !crosses_exception_boundary
                        && !instruction.fusion_policy.is_authored_break()
                        && instruction.pre_fold_extra_instructions() == 0
                        && previous == previous_at_block_entry
                        && instruction.location.line >= 0
                        && next_line
                            .is_some_and(|line| line >= 0 && line != instruction.location.line)
                        && original_fallthrough_exits.iter().any(
                            |(original, original_target, pop)| {
                                *original == position
                                    && *original_target == target
                                    && instruction.location == pop.location
                                    && previous.is_some_and(|previous| {
                                        graph.item(previous) == &Item::Instruction(*pop)
                                    })
                            },
                        );
                    // The original compound exit starts a new array after the
                    // final wildcard call. Its CALL/POP pair and entry/target labels
                    // must still be exact; crossing its finally boundary is expected.
                    let preserves_compound_fallthrough_nop = !absorbed_normal_try_exit
                        && !instruction.fusion_policy.is_authored_break()
                        && instruction.pre_fold_extra_instructions() == 0
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                        && previous == previous_at_block_entry
                        && instruction.location.line >= 0
                        && next_line
                            .is_some_and(|line| line >= 0 && line != instruction.location.line)
                        && compound_fallthrough_exits.iter().any(
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
                    // An original cleanup end does not merge the jump's source array.
                    // A range beginning there must be a split of that same protected region;
                    // unrelated setup/handler boundaries keep the general boundary guard.
                    let preserves_copied_finally_break_nop = finalize_borrow_empty_blocks
                        && (!crosses_exception_boundary
                            || (original_cleanup_boundaries == 1 && crosses_only_original_cleanup))
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
                    // Async exhaustion starts an original empty continuation array after
                    // END_ASYNC_FOR. Cold ordering can make its branch jump redundant, but
                    // CPython resets the preceding line at that certified array boundary.
                    // An unchanged match-success jump can also close its original try body
                    // at the authored match end. Its replacement NOP remains before that
                    // sole region end; setup, handler, and unrelated boundaries still reject.
                    let original_async_for_join = finalize_borrow_empty_blocks
                        && (!crosses_exception_boundary || crosses_only_original_match_end)
                        && !absorbed_normal_try_exit
                        && !instruction.fusion_policy.is_authored_break()
                        && !instruction.fusion_policy.is_converted_pop_block()
                        && instruction.pre_fold_extra_instructions() == 0
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                        && instruction.exception_policy() == super::super::ExceptionPolicy::default()
                        && instruction.location.line >= 0
                        && next_line
                            .is_some_and(|line| line >= 0 && line != instruction.location.line)
                        && previous == previous_at_block_entry
                        && previous.is_some_and(|previous| {
                            matches!(graph.item(previous), Item::Instruction(donor)
                                if donor.opcode == END_ASYNC_FOR
                                    && donor.location == instruction.location)
                        })
                        && graph[block].items()[..offset]
                            .iter()
                            .all(|item| {
                                matches!(item, Item::Label(label)
                                    if !self.original_nop_targets.contains(label)
                                        && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                                        && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label))
                            })
                        && graph[block].items()[..offset].iter().any(|item| {
                            matches!(item, Item::Label(label)
                                if self.exception_owner_provenance.source_boundaries.contains(label)
                                    && self.exception_owner_provenance.block_boundaries.contains(label)
                                    && !self.borrow_block_aliases.contains(label)
                                    && !self.original_nop_targets.contains(label))
                        });

                    // This certified authored break starts its original array before a
                    // synchronous manager's POP_BLOCK and five cleanup records.
                    // That located, non-exit prefix cannot be copied early. Native
                    // NOP compaction therefore resets its previous line here.
                    let original_manager_break_join = !finalize_borrow_empty_blocks
                        && first_scan
                        && previous == previous_at_block_entry
                        && instruction.location.line >= 0
                        && previous_line == Some(instruction.location.line)
                        && next_line.is_some_and(|line| line >= 0 && line != instruction.location.line)
                        && previous.is_some_and(|previous| {
                            previous.block() != block
                                && matches!(graph.item(previous), Item::Instruction(backedge)
                                    if backedge.opcode == JUMP_BACKWARD
                                        && matches!(backedge.operand(), Operand::Backward(_))
                                        && backedge.depth_after() == Some(2))
                        })
                        && {
                            let mut original = Instruction::new(
                                JUMP_FORWARD, Operand::Forward(target), instruction.location, Some(2),
                            );
                            original.fusion_policy.mark_authored_break();
                            original.jump_policy.prevent_threading_target();
                            original.jump_policy.retain_nop_for_direct_target();
                            original.add_pre_fold_extra_instructions(1);
                            instruction == original
                        }
                        && graph[block].items()[..offset].iter().all(|item| {
                            matches!(item, Item::Label(label)
                                if self.original_nop_targets.contains(label)
                                    && !self.borrow_block_aliases.contains(label)
                                    && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                                    && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label))
                        })
                        && graph[block].items()[..offset].iter().any(|item| {
                            matches!(item, Item::Label(label)
                                if self.exception_owner_provenance.block_boundaries.contains(label))
                        })
                        && next.is_some_and(|next| {
                            let items = graph[next.block()].items();
                            let heads = &items[..next.offset()];
                            let Some(prefix) = items.get(next.offset()..next.offset() + 5) else { return false; };
                            let Item::Instruction(first) = prefix[0] else { return false; };
                            let Operand::Value(constant) = first.operand() else { return false; };
                            let plain = |opcode, operand, depth| Item::Instruction(Instruction::new(
                                opcode, Operand::Value(operand), first.location, Some(depth),
                            ));
                            if !heads.contains(&Item::Label(target))
                                || !heads.iter().all(|item| matches!(item, Item::Label(label)
                                    if !self.borrow_block_aliases.contains(label)
                                        && !self.original_nop_targets.contains(label)
                                        && !self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                                        && !self.exception_owner_provenance.omitted_nop_locations.contains_key(label)))
                                || !self.exception_owner_provenance.block_boundaries.contains(&target)
                                || prefix != [plain(LOAD_CONST, constant, 3), plain(LOAD_CONST, constant, 4),
                                    plain(LOAD_CONST, constant, 5), plain(CALL, 3, 1), plain(POP_TOP, 0, 0)]
                            { return false; }
                            let mut endings = self.exception_regions.iter()
                                .filter(|region| heads.contains(&Item::Label(region.end)));
                            let Some(owner) = endings.next() else { return false; };
                            endings.next().is_none()
                                && !owner.is_scope_fallback && owner.depth == 2 && owner.preserve_lasti
                                && graph[block].items()[..offset].contains(&Item::Label(owner.start))
                                && heads.iter().all(|item| *item == Item::Label(target) || *item == Item::Label(owner.end))
                                && self.exception_regions.iter().all(|region| {
                                    !heads.contains(&Item::Label(region.start))
                                        && !heads.contains(&Item::Label(region.target))
                                })
                        });

                    if !preserves_original_fallthrough_nop
                        && !preserves_compound_fallthrough_nop
                        && !preserves_copied_finally_break_nop
                        && (!instruction.jump_policy.always_retains_nop()
                            || absorbed_normal_try_exit)
                        && (instruction.location.line < 0
                            || (!original_async_for_join
                                && !original_manager_break_join
                                && previous_line == Some(instruction.location.line))
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
                            // Synthesized percent literals have a separate predecessor-aware
                            // writer. A removed jump must not donate its line to a shared join.
                            if next.location.line < 0
                                && !next.no_location_policy.preserves_location()
                                && !next.no_location_policy.allows_block_inlining()
                                && next.no_location_policy.percent_target_location == 0
                            {
                                next.location = instruction.location;
                            }
                        }
                    } else {
                        let ownerless_compound_exit = finalize_borrow_empty_blocks
                            && self
                                .exception_owner_provenance
                                .ownerless_compound_try_nops
                                .iter()
                                .any(|(entry, original_target, original)| {
                                    *original_target == target
                                        && *original == instruction
                                        && graph[block].items()[..offset]
                                            .iter()
                                            .all(|item| matches!(item, Item::Label(_)))
                                        && graph[block].items()[..offset]
                                            .contains(&Item::Label(*entry))
                                });
                        let module_annotation_donor = (!finalize_borrow_empty_blocks
                            && !crossed_located_folded_marker
                            && !crossed_other_located_marker
                            && !crosses_exception_boundary
                            && !crosses_removed_line_event
                            && !preserves_original_fallthrough_nop
                            && !preserves_compound_fallthrough_nop
                            && !preserves_copied_finally_break_nop
                            && !ownerless_compound_exit)
                            .then_some(previous)
                            .flatten()
                            .filter(|previous| {
                                previous.block() == block && previous.offset() + 1 == offset
                            })
                            .and_then(|previous| match graph.item(previous) {
                                Item::Instruction(donor)
                                    if self
                                        .exception_owner_provenance
                                        .module_annotation_jump_pairs
                                        .contains(&(*donor, instruction))
                                        && unique_module_annotation_pair(
                                            graph,
                                            *donor,
                                            instruction,
                                        ) =>
                                {
                                    Some(*donor)
                                }
                                _ => None,
                            });
                        let instruction = graph.instruction_mut(position).unwrap();
                        instruction.opcode = NOP;
                        instruction.set_operand(Operand::Value(0));
                        if crossed_located_folded_marker
                            && !crossed_other_located_marker
                            && !crosses_exception_boundary
                            && !preserves_original_fallthrough_nop
                            && !preserves_compound_fallthrough_nop
                            && !preserves_copied_finally_break_nop
                            && !ownerless_compound_exit
                            && instruction.jump_policy.always_retains_nop()
                            && instruction.no_location_policy.borrow_fallthrough()
                                == BorrowFallthrough::BarrierIfRemoved
                            && previous_line == Some(instruction.location.line)
                            && next_line == Some(instruction.location.line)
                        {
                            // Both folded arms have vanished from this source line.
                            // Keep their empty-block barrier for borrowed-load analysis,
                            // then remove the redundant jump's NOP with the other marker.
                            instruction.jump_policy.mark_borrow_empty_block();
                        }
                        if ownerless_compound_exit {
                            // Only this unchanged original jump survived as the normal-exit NOP.
                            // Copied, retargeted, and still-live exits retain their own owners.
                            instruction.set_exception_owner(NormalizedExceptionOwner::Absent);
                        }
                        if let Some(donor) = module_annotation_donor {
                            self.exception_owner_provenance
                                .module_annotation_jump_nops
                                .push((block, donor, *instruction));
                        }
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

        if redundant.is_empty() {
            return;
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

    /// Gives an unchanged normal-finally POP its sole surviving original jump's line.
    fn propagate_original_normal_finally_entry_locations(&mut self) {
        let origins = &self
            .exception_owner_provenance
            .original_normal_finally_arrays;
        if !origins
            .iter()
            .any(|origin| origin.retained_setup.is_some() && origin.incoming.len() > 1)
        {
            return;
        }
        let items = self
            .graph
            .iter()
            .flat_map(|(block, contents)| {
                contents
                    .items()
                    .iter()
                    .enumerate()
                    .map(move |(offset, item)| (ItemPosition::new(block, offset), *item))
            })
            .collect::<Vec<_>>();
        let mut updates = Vec::new();
        for origin in origins {
            if origin.retained_setup.is_none() || origin.incoming.len() <= 1 {
                continue;
            }
            let mut starts =
                items
                    .windows(origin.prefix.len())
                    .enumerate()
                    .filter_map(|(start, current)| {
                        current
                            .iter()
                            .map(|(_, item)| item)
                            .eq(origin.prefix.iter())
                            .then_some(start)
                    });
            let Some(start) = starts.next() else { continue };
            if starts.next().is_some() || start == 0 {
                continue;
            }
            let Item::Instruction(previous) = items[start - 1].1 else {
                continue;
            };
            let jump = if previous.opcode == RERAISE
                && origin.incoming.len() == 2
                && origin
                    .incoming
                    .iter()
                    .any(|jump| jump.location == SourceLocation::NONE)
                && matches!(origin.prefix.last(), Some(Item::Label(end))
                    if self.exception_owner_provenance.normal_try_exits.iter()
                        .any(|(region_end, _)| region_end == end))
            {
                // Cold handler blocks can separate this certified normal-finally
                // entry from its only surviving original predecessor. The retained
                // nested-try prefix and terminating physical boundary stay exact.
                let mut donors = items[..start].iter().filter_map(|(_, item)| match item {
                    Item::Instruction(jump)
                        if jump.opcode == JUMP_FORWARD
                            && jump.operand() == Operand::Forward(origin.entry)
                            && jump.location.line >= 0
                            && origin.incoming.contains(jump) =>
                    {
                        Some(*jump)
                    }
                    _ => None,
                });
                let Some(donor) = donors.next() else { continue };
                if donors.next().is_some() {
                    continue;
                }
                donor
            } else {
                previous
            };
            if jump.opcode != JUMP_FORWARD
                || jump.operand() != Operand::Forward(origin.entry)
                || jump.location.line < 0
                || !origin.incoming.contains(&jump)
            {
                continue;
            }
            let leading = origin
                .prefix
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<FxHashSet<_>>();
            if items.iter().filter(|(_, item)| matches!(item,
                Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if leading.contains(&label))
            )).count() != 1 {
                continue;
            }
            let Some(offset) = origin
                .prefix
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            let Item::Instruction(pop) = items[start + offset].1 else {
                unreachable!()
            };
            if pop.opcode != NOP
                || !pop.fusion_policy.is_converted_pop_block()
                || pop.exception_policy() != super::super::ExceptionPolicy::EXCLUDE_ALWAYS
                || pop.location != SourceLocation::NONE
                || pop.no_location_policy.preserves_location()
            {
                continue;
            }
            updates.push((items[start + offset].0, jump.location));
        }
        for (position, location) in updates {
            self.graph.instruction_mut(position).unwrap().location = location;
        }
    }

    pub(in crate::assembler) fn propagate_locations_within_blocks(&mut self) {
        self.propagate_original_normal_finally_entry_locations();
        let graph = &mut self.graph;
        for block_index in 0..graph.len() {
            let block = graph.block_at(block_index);
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
        self.propagate_copied_finally_iterator_return_locations();
    }

    /// Replays a finalizer donor only through its unchanged original cleanup array.
    fn propagate_copied_finally_iterator_return_locations(&mut self) {
        let origins = std::mem::take(
            &mut self
                .exception_owner_provenance
                .copied_finally_iterator_returns,
        );
        if origins.is_empty() {
            return;
        }
        let mut forbidden = self.exception_owner_provenance.block_boundaries.clone();
        forbidden.extend(&self.exception_owner_provenance.source_boundaries);
        forbidden.extend(&self.preserved_block_boundaries);
        forbidden.extend(self.exception_regions.iter().map(|region| region.target));
        forbidden.extend(
            self.exception_owner_provenance
                .setups
                .iter()
                .map(|(label, _)| *label),
        );
        let items = self
            .graph
            .iter()
            .flat_map(|(block, contents)| {
                contents
                    .items()
                    .iter()
                    .enumerate()
                    .map(move |(offset, item)| (ItemPosition::new(block, offset), *item))
            })
            .collect::<Vec<_>>();
        let mut labels = FxHashMap::default();
        for (index, (_, item)) in items.iter().enumerate() {
            match item {
                Item::Label(label) => {
                    labels.insert(*label, index);
                }
                Item::Instruction(instruction) => {
                    if let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                    {
                        forbidden.insert(label);
                    }
                }
            }
        }
        for (donor, origin) in origins {
            let Some((anchor, label)) =
                origin
                    .iter()
                    .enumerate()
                    .find_map(|(index, item)| match item {
                        Item::Label(label) => Some((index, *label)),
                        Item::Instruction(_) => None,
                    })
            else {
                continue;
            };
            let Some(start) = labels
                .get(&label)
                .and_then(|index| index.checked_sub(anchor))
            else {
                continue;
            };
            let Some(current) = items.get(start..start + origin.len()) else {
                continue;
            };
            if !matches!(current.first(), Some((_, Item::Instruction(instruction)))
                if instruction.location == donor)
            {
                continue;
            }
            if !current.iter().zip(&origin).all(|((_, current), original)| {
                match (*current, *original) {
                    (Item::Label(actual), Item::Label(expected)) => {
                        actual == expected && !forbidden.contains(&actual)
                    }
                    (Item::Instruction(mut actual), Item::Instruction(expected)) => {
                        // The existing within-array pass may already have located the
                        // iterator's SWAP/POP. No other record or metadata may change.
                        if expected.location == SourceLocation::NONE && actual.location == donor {
                            actual.location = SourceLocation::NONE;
                        }
                        actual == expected
                    }
                    _ => false,
                }
            }) {
                continue;
            }
            let (position, _) = current.last().expect("captured return array is nonempty");
            self.graph
                .instruction_mut(*position)
                .expect("captured original return")
                .location = donor;
        }
    }

    /// Removes NOPs and reports possible edits and remaining physical forward jumps.
    pub(in crate::assembler) fn remove_redundant_nops(&mut self) -> NopCleanup {
        self.remove_redundant_nops_scoped(false)
    }

    /// Returns whether manager cleanup ran and may have changed the graph or its provenance.
    pub(in crate::assembler) fn remove_redundant_with_exit_nops(&mut self) -> bool {
        if self
            .exception_owner_provenance
            .with_context_locations
            .is_empty()
        {
            return false;
        }
        self.remove_redundant_nops_scoped(true).may_have_changed
    }

    fn remove_redundant_nops_scoped(&mut self, with_exits_only: bool) -> NopCleanup {
        let is_forward_jump = |instruction: &Instruction| {
            instruction.opcode == JUMP_FORWARD
                && matches!(instruction.operand(), Operand::Forward(_))
        };
        // Late cleanup already returns when the graph has never summarized any jump.
        let track_forward_jumps = !with_exits_only && self.graph.features().has_jump();
        let mut has_forward_jump = false;
        if !self.graph.iter_items().any(|item| {
            let Item::Instruction(instruction) = item else {
                return false;
            };
            if track_forward_jumps && !has_forward_jump {
                has_forward_jump = is_forward_jump(instruction);
            }
            instruction.opcode == NOP
                && (!with_exits_only
                    || (instruction.fusion_policy.is_converted_pop_block()
                        && !instruction.jump_policy.borrow_empty_block()
                        && self
                            .exception_owner_provenance
                            .with_context_locations
                            .contains(&instruction.location)))
        }) {
            // Without a matching NOP the candidate walk already visited every instruction.
            return NopCleanup {
                has_forward_jump,
                may_have_changed: false,
            };
        }
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
            && !copied_jump_nops.is_empty()
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
        let mut original_fallthrough_pops = FxHashSet::default();
        if !self
            .exception_owner_provenance
            .original_manager_pop_blocks
            .is_empty()
            || !self
                .exception_owner_provenance
                .original_fallthrough_pop_blocks
                .is_empty()
        {
            for block in graph.order().iter().copied() {
                let mut original_manager_pop = false;
                let mut original_fallthrough_pop = None;
                for (offset, item) in graph[block].items().iter().enumerate() {
                    match item {
                        Item::Label(label) => {
                            original_manager_pop |= self
                                .exception_owner_provenance
                                .original_manager_pop_blocks
                                .contains(label);
                            if let Some(pop) = self
                                .exception_owner_provenance
                                .original_fallthrough_pop_blocks
                                .get(label)
                            {
                                original_fallthrough_pop = Some(pop);
                            }
                        }
                        Item::Instruction(instruction) => {
                            if original_manager_pop
                                && instruction.opcode == NOP
                                && instruction.fusion_policy.is_converted_pop_block()
                            {
                                original_manager_pops.insert(ItemPosition::new(block, offset));
                            }
                            if original_fallthrough_pop.is_some_and(|original| {
                                if original == instruction {
                                    return true;
                                }
                                // A small return's original POP stays unlocated until
                                // its jumping predecessors have copied it. Only its
                                // location changes during the ensuing propagation.
                                let mut propagated = *original;
                                if propagated.location != SourceLocation::NONE {
                                    return false;
                                }
                                propagated.location = instruction.location;
                                propagated == *instruction
                            }) {
                                original_fallthrough_pops.insert(ItemPosition::new(block, offset));
                            }
                            original_manager_pop = false;
                            original_fallthrough_pop = None;
                        }
                    }
                }
            }
        }
        let items = graph.order().iter().copied().flat_map(|block| {
            graph[block]
                .items()
                .iter()
                .enumerate()
                .map(move |(offset, item)| (block, ItemPosition::new(block, offset), item))
        });
        let mut items = items.peekable();
        let mut preceding_labels = Vec::new();
        let mut following_labels = Vec::new();
        let mut history_block = 0_usize;
        let mut previous_original: Option<(ItemPosition, &Instruction)> = None;
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
        while let Some((graph_block, position, item)) = items.next() {
            let instruction = match item {
                Item::Instruction(instruction) => instruction,
                // Only leading labels reach this arm. Once there is an original instruction,
                // its lookahead consumes all labels before the next one.
                Item::Label(label) => {
                    if track_exception_owners {
                        if track_pop_block_boundaries {
                            preceding_labels.push(*label);
                        }
                        if self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                        {
                            history_block += 1;
                        }
                    }
                    continue;
                }
            };
            // The lookahead consumes labels only, so this sees every instruction. The
            // pass edits NOPs, locations and metadata, but cannot remove or create a jump.
            if track_forward_jumps && !has_forward_jump {
                has_forward_jump = is_forward_jump(instruction);
            }
            if !track_exception_owners && instruction.opcode != NOP {
                previous = Some(instruction);
                previous_graph_block = Some(graph_block);
                previous_position = Some(position);
                removed_unpack_line_block = None;
                continue;
            }
            let block = history_block;
            if track_exception_owners
                && (instruction.opcode == NOT_TAKEN
                    || is_unconditional_jump(instruction.opcode)
                    || ends_scope(instruction.opcode))
            {
                history_block += 1;
            }
            // A setup certifies the instruction before its label, even when that instruction
            // will be removed. The next instruction also remains the next original instruction,
            // whereas `previous` below advances only when an instruction survives.
            let mut setup_nop = false;
            while let Some((_, _, Item::Label(label))) = items.peek().copied() {
                let _ = items.next();
                if track_exception_owners {
                    if track_pop_block_boundaries {
                        following_labels.push(*label);
                        if instruction.opcode == NOP
                            && setup_locations.get(label) == Some(&instruction.location)
                        {
                            setup_nop = true;
                        }
                    }
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label)
                    {
                        history_block += 1;
                    }
                }
            }
            let next_original = items.peek().copied().map(|(next_block, _, next)| {
                let Item::Instruction(next) = next else {
                    unreachable!("lookahead consumes all intervening labels");
                };
                (next_block, next)
            });
            let next = next_original.map(|(_, instruction)| instruction);
            if pending_block != Some(block) {
                pending_slots = 0;
                pending_pop_slots = Some(0);
                if track_pop_block_boundaries {
                    pending_pop_boundaries.clear();
                }
                pending_block = Some(block);
            }
            let discontinuous = track_pop_block_boundaries
                && preceding_labels
                    .iter()
                    .any(|label| nonlinear_labels.contains(label));
            if discontinuous {
                pending_pop_boundaries.clear();
                pending_pop_slots = None;
            }
            // A SETUP's exclusion label need not start an original array. Only the exact
            // surviving fallthrough predecessor can donate across that sole label.
            let setup_nop_predecessor = setup_nop
                && !with_exits_only
                && matches!(preceding_labels.as_slice(), [setup_start]
                if setup_nop_starts.contains(setup_start)
                    && !nonlinear_labels.contains(setup_start)
                    && !self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(setup_start)
                    && !self.original_nop_targets.contains(setup_start)
                    && !self.original_jump_head_targets.contains(setup_start)
                    && previous_original.is_some_and(|(original_position, original)| {
                        previous_position == Some(original_position)
                            // NEXT_LOCATION is resolved after NOP removal and cannot supply a donor.
                            && !original.fusion_policy.location_from_next()
                            && original.location.line == instruction.location.line
                            && original.opcode != NOT_TAKEN
                            && !ends_scope(original.opcode)
                            && !is_unconditional_jump(original.opcode)
                            && matches!(original.operand(), Operand::Value(_))
                    }));
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
                && next_original.is_some_and(|(next_block, next)| {
                    next_block == graph_block
                        && next.opcode == NOP
                        && next.location.line >= 0
                        && next.location.line != instruction.location.line
                });
            let module_annotation_predecessor = previous_graph_block == Some(graph_block)
                && previous.is_some_and(|previous| {
                    self.exception_owner_provenance
                        .module_annotation_nop_pairs
                        .contains(&(*previous, *instruction))
                });
            let module_annotation_jump_predecessor = certified_module_annotation_jump_nop(
                graph,
                &self.exception_owner_provenance.module_annotation_jump_nops,
                previous_position,
                position,
            );
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
                    // Keep empty SETUPs through reachability, then compact their NOP
                    // once the threaded exit carries the same source location.
                    || (self.python_version >= crate::PythonVersion::Py314
                        && setup_nop
                        && instruction.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
                        && following_labels.iter().any(|label| {
                            setup_locations.get(label) == Some(&instruction.location)
                                && self.exception_owner_provenance
                                    .ordinary_try_setups
                                    .contains_key(label)
                        })
                        && next.is_some_and(|next| {
                            next.location == instruction.location
                                && (is_unconditional_jump(next.opcode)
                                    || next.opcode == RERAISE)
                        }))
                    || (original_manager_pops.contains(&position)
                        && (instruction.location.line < 0
                            || next.is_some_and(|next| {
                                next.location.line == instruction.location.line
                            }))))
                && (instruction.location.line < 0
                    || (instruction.fusion_policy.is_converted_pop_block()
                        // This exact first record starts a fresh CPython array. Its
                        // physical donor is in the preceding array; the following-line
                        // and unlocated removal rules below still apply normally.
                        && !original_fallthrough_pops.contains(&position)
                        && previous.is_some_and(|previous: &Instruction| {
                            previous.location.line == instruction.location.line
                                && !(ends_scope(previous.opcode)
                                    || is_unconditional_jump(previous.opcode))
                        }))
                    || setup_nop_predecessor
                    || module_annotation_predecessor
                    || module_annotation_jump_predecessor
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
                        && !setup_nop)
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
                if track_exception_owners && !setup_nop {
                    if track_pop_block_boundaries
                        && instruction.fusion_policy.is_converted_pop_block()
                    {
                        // A source join begins a fresh native instruction array. Otherwise,
                        // the preceding original instruction must fall through without a join.
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
                        if starts_source_array
                            || (!discontinuous
                                && previous_original.is_some_and(|(_, original)| {
                                    original.opcode != NOT_TAKEN
                                        && !is_unconditional_jump(original.opcode)
                                        && !ends_scope(original.opcode)
                                }))
                        {
                            pending_pop_boundaries.extend(
                                preceding_labels
                                    .iter()
                                    .copied()
                                    .filter(|label| region_ends.contains(label)),
                            );
                        }
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
            if track_pop_block_boundaries {
                previous_original = Some((position, instruction));
                preceding_labels.clear();
                std::mem::swap(&mut preceding_labels, &mut following_labels);
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
        graph.remove_ordered_instructions(&redundant);
        NopCleanup {
            has_forward_jump,
            may_have_changed: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::{Assembler, ExceptionRegion, ExceptionRegionPriority};

    #[test]
    fn forward_threading_source_gates_match_unconditional_candidate_scans() {
        let [start, loop_head, source, entry, destination, handler] =
            [0, 1, 2, 3, 4, 5].map(Label::from_u32);
        let located = SourceLocation::new(5, 5, 1, 2);
        let returned = SourceLocation::new(9, 9, 1, 2);
        let make = |variation: &str| {
            let ordinary = |opcode, operand, location, depth| {
                Item::Instruction(Instruction::new(opcode, operand, location, Some(depth)))
            };
            let mut candidate =
                Instruction::new(JUMP_FORWARD, Operand::Forward(entry), located, Some(0));
            if variation.starts_with("authored") {
                candidate.fusion_policy.mark_authored_break();
                candidate.jump_policy.prevent_threading_target();
                if variation == "authored-retained" {
                    candidate.jump_policy.retain_nop();
                } else {
                    candidate.jump_policy.retain_nop_for_direct_target();
                    candidate.add_pre_fold_extra_instructions(1);
                }
            } else if variation == "retained-conditional" {
                candidate.jump_policy.retain_nop();
            }
            let backward = matches!(variation, "authored-retained" | "retained-conditional");
            let mut bridge = if backward {
                Instruction::new(
                    JUMP_BACKWARD,
                    Operand::Backward(loop_head),
                    returned,
                    Some(0),
                )
            } else {
                Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(destination),
                    SourceLocation::NONE,
                    Some(0),
                )
            };
            if !backward && variation != "plain" {
                bridge.no_location_policy.preserve_location();
            }
            let mut assembler = Assembler::from_linear_items(vec![
                Item::Label(start),
                ordinary(LOAD_CONST, Operand::Value(0), located, 1),
                ordinary(POP_JUMP_IF_FALSE, Operand::Forward(source), located, 0),
                Item::Label(loop_head),
                ordinary(NOP, Operand::Value(0), located, 0),
                ordinary(JUMP_FORWARD, Operand::Forward(entry), located, 0),
                Item::Label(source),
                Item::Instruction(candidate),
                Item::Label(entry),
                Item::Instruction(bridge),
                Item::Label(destination),
                ordinary(
                    crate::target::opcodes::LOAD_FAST,
                    Operand::Value(0),
                    returned,
                    1,
                ),
                ordinary(RETURN_VALUE, Operand::Value(0), returned, 0),
                Item::Label(handler),
                ordinary(RAISE_VARARGS, Operand::Value(0), returned, 0),
            ]);
            assembler.next_label = 6;
            let mut pipeline = assembler.into_optimization_pipeline();
            if variation != "plain" {
                let boundaries = [loop_head, source, entry, destination];
                if variation != "no-source-boundaries" {
                    pipeline
                        .exception_owner_provenance
                        .source_boundaries
                        .extend(boundaries);
                }
                if variation != "no-block-boundaries" {
                    pipeline
                        .exception_owner_provenance
                        .block_boundaries
                        .extend(boundaries);
                }
                if variation != "no-original-heads" {
                    pipeline.original_jump_head_targets.insert(entry);
                }
                if variation != "no-folded-sources" {
                    pipeline
                        .folded_predecessor_sources
                        .push((source, entry, SourceLocation::NONE));
                }
                if variation != "no-conditional-targets" {
                    pipeline
                        .conditional_threading_targets
                        .insert(entry, returned);
                }
                if variation != "no-normal-exits" {
                    pipeline
                        .exception_owner_provenance
                        .normal_try_exits
                        .push((entry, destination));
                    pipeline
                        .exception_owner_provenance
                        .compound_try_owner_exits
                        .push((entry, destination));
                }
            }
            if matches!(variation, "two-regions" | "fallback-regions") {
                for (range_start, range_end, is_scope_fallback) in [
                    (start, destination, variation == "fallback-regions"),
                    (source, entry, false),
                ] {
                    pipeline.exception_regions.push(ExceptionRegion {
                        start: range_start,
                        end: range_end,
                        target: handler,
                        depth: 0,
                        preserve_lasti: is_scope_fallback,
                        is_scope_fallback,
                        priority: ExceptionRegionPriority::OriginalRange(range_start, range_end),
                    });
                }
            }
            pipeline
        };

        for variation in [
            "plain",
            "ordinary",
            "authored-return",
            "authored-retained",
            "retained-conditional",
            "no-source-boundaries",
            "no-block-boundaries",
            "no-original-heads",
            "no-folded-sources",
            "no-conditional-targets",
            "no-normal-exits",
            "two-regions",
            "fallback-regions",
        ] {
            let mut gated = make(variation);
            let mut reference = make(variation);
            let changed = gated.thread_forward_jumps();
            assert_eq!(
                changed,
                reference.thread_forward_jumps_with_source_gates::<true, false>(),
                "{variation}"
            );
            if variation == "plain" {
                assert!(changed, "the generic jump-threading pass still runs");
            }
            assert_eq!(
                gated.graph.copy_items(),
                reference.graph.copy_items(),
                "{variation}"
            );
            assert_eq!(
                format!("{gated:?}"),
                format!("{reference:?}"),
                "{variation}"
            );
        }
    }

    #[test]
    fn original_conditional_threading_preflight_matches_handler_and_empty_target_scans() {
        let [start, source, entry, destination] = [0, 1, 2, 3].map(Label::from_u32);
        let located = SourceLocation::new(5, 5, 1, 2);
        let make = |variation: &str| {
            let ordinary = |opcode, operand, depth| {
                Item::Instruction(Instruction::new(opcode, operand, located, Some(depth)))
            };
            let mut bridge = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(destination),
                SourceLocation::NONE,
                Some(1),
            );
            bridge.fusion_policy.mark_authored_break();
            bridge.jump_policy.prevent_threading_target();
            if variation == "changed-bridge" {
                bridge.add_pre_fold_extra_instructions(1);
            }
            let mut assembler = Assembler::from_linear_items(vec![
                Item::Label(start),
                ordinary(LOAD_CONST, Operand::Value(0), 1),
                ordinary(POP_JUMP_IF_FALSE, Operand::Forward(entry), 0),
                ordinary(NOT_TAKEN, Operand::Value(0), 0),
                ordinary(JUMP_FORWARD, Operand::Forward(source), 0),
                Item::Label(source),
                ordinary(POP_EXCEPT, Operand::Value(0), 1),
                ordinary(JUMP_FORWARD, Operand::Forward(entry), 1),
                Item::Label(entry),
                Item::Instruction(bridge),
                Item::Label(destination),
                ordinary(LOAD_CONST, Operand::Value(0), 1),
                ordinary(RETURN_VALUE, Operand::Value(0), 0),
            ]);
            assembler.next_label = 4;
            let mut pipeline = assembler.into_optimization_pipeline();
            if variation != "no-regions" {
                pipeline.exception_regions.push(ExceptionRegion {
                    start,
                    end: source,
                    target: destination,
                    depth: 2,
                    preserve_lasti: variation != "no-lasti",
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(start, source),
                });
            }
            if variation != "no-original-nop" {
                pipeline.original_nop_targets.insert(source);
            }
            if variation != "no-nop-count" {
                pipeline
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(source, 1);
            }
            if variation != "no-nop-location" {
                pipeline
                    .exception_owner_provenance
                    .omitted_nop_locations
                    .insert(source, located);
            }
            pipeline
                .exception_owner_provenance
                .block_boundaries
                .insert(entry);
            pipeline
                .exception_owner_provenance
                .source_boundaries
                .insert(entry);
            if variation != "no-conditional-target" && variation != "no-bound-target" {
                pipeline.conditional_threading_targets.insert(
                    entry,
                    if variation == "located-target" {
                        located
                    } else {
                        SourceLocation::NONE
                    },
                );
            } else if variation == "no-bound-target" {
                // This set enters the pass but its block does not begin with a jump.
                pipeline.original_jump_head_targets.insert(source);
            }
            pipeline
        };
        for variation in [
            "ordinary",
            "no-regions",
            "no-lasti",
            "no-original-nop",
            "no-nop-count",
            "no-nop-location",
            "no-conditional-target",
            "located-target",
            "changed-bridge",
            "no-bound-target",
        ] {
            let mut gated = make(variation);
            let mut original = make(variation);
            let heads = gated.thread_copied_finally_pass_breaks::<true>();
            assert_eq!(
                heads,
                original.thread_copied_finally_pass_breaks::<false>(),
                "{variation}"
            );
            assert_eq!(
                heads.len(),
                usize::from(variation == "ordinary"),
                "{variation}"
            );
            assert_eq!(
                gated.graph.copy_items(),
                original.graph.copy_items(),
                "{variation}"
            );

            let mut gated = make(variation);
            let mut original = make(variation);
            let changed = gated.thread_original_conditional_jump_targets();
            assert_eq!(
                changed,
                original.thread_original_conditional_jump_targets_with_preflight::<false>(),
                "{variation}"
            );
            assert_eq!(
                changed,
                !matches!(variation, "no-conditional-target" | "no-bound-target"),
                "{variation}"
            );
            assert_eq!(
                gated.graph.copy_items(),
                original.graph.copy_items(),
                "{variation}"
            );
            assert_eq!(
                gated.graph.features(),
                original.graph.features(),
                "{variation}"
            );
            assert_eq!(
                gated.exception_owner_provenance.threaded_condition_origins,
                original
                    .exception_owner_provenance
                    .threaded_condition_origins,
                "{variation}"
            );
        }
    }

    #[test]
    fn borrow_empty_jump_head_preflight_matches_full_incoming_analysis() {
        let source = Label::from_u32(0);
        let head = Label::from_u32(1);
        let alias = Label::from_u32(2);
        let exit = Label::from_u32(3);
        let missing = Label::from_u32(4);
        let located = SourceLocation::new(4, 4, 0, 1);
        let make = |variation: &str| {
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(if variation == "unbound" {
                    missing
                } else {
                    exit
                }),
                if variation == "located" {
                    located
                } else {
                    SourceLocation::NONE
                },
                None,
            );
            match variation {
                "folded" => jump.add_pre_fold_extra_instructions(1),
                "interruptible" => jump.jump_policy.mark_interruptible(),
                "retained" => jump.jump_policy.retain_nop(),
                "direct-retained" => jump.jump_policy.retain_nop_for_direct_target(),
                "already-marked" => jump.jump_policy.mark_borrow_empty_block(),
                _ => {}
            }
            let mut items = vec![
                Item::Label(source),
                Item::Instruction(Instruction::new(NOP, Operand::Value(0), located, None)),
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(head),
                    located,
                    None,
                )),
                Item::Label(head),
                Item::Label(alias),
            ];
            if variation == "prefix" {
                items.push(Item::Instruction(Instruction::new(
                    NOP,
                    Operand::Value(0),
                    located,
                    None,
                )));
            }
            items.extend([
                Item::Instruction(jump),
                Item::Label(exit),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    located,
                    None,
                )),
            ]);
            let mut assembler = Assembler::from_linear_items(items);
            assembler.next_label = 5;
            let mut pipeline = assembler.into_optimization_pipeline();
            if variation == "original-nop" {
                pipeline.original_nop_targets.insert(alias);
            }
            if variation == "preserved" {
                pipeline.preserved_block_boundaries.insert(alias);
            }
            pipeline
        };

        for (variation, expected) in [
            ("ordinary", true),
            ("direct-retained", true),
            ("located", false),
            ("folded", false),
            ("interruptible", false),
            ("retained", false),
            ("already-marked", false),
            ("original-nop", false),
            ("preserved", false),
            ("prefix", false),
            ("unbound", false),
        ] {
            let mut gated = make(variation);
            let mut full = make(variation);
            assert_eq!(
                gated.record_borrow_empty_jump_heads(),
                expected,
                "{variation}"
            );
            assert_eq!(
                full.record_borrow_empty_jump_heads_with_preflight::<false>(),
                expected,
                "{variation}",
            );
            assert_eq!(
                gated.graph.copy_items(),
                full.graph.copy_items(),
                "{variation}"
            );
            assert_eq!(gated.graph.features(), full.graph.features(), "{variation}");
        }
    }

    #[test]
    fn original_jump_head_preflight_matches_full_conditional_target_analysis() {
        let source = Label::from_u32(0);
        let head = Label::from_u32(1);
        let alias = Label::from_u32(2);
        let exit = Label::from_u32(3);
        let missing = Label::from_u32(4);
        let located = SourceLocation::new(4, 4, 0, 1);
        let other = SourceLocation::new(5, 5, 0, 1);
        let make = |variation: &str| {
            let backward = variation.starts_with("backward");
            let mut jump = Instruction::new(
                if backward {
                    JUMP_BACKWARD
                } else {
                    JUMP_FORWARD
                },
                if backward {
                    Operand::Backward(source)
                } else {
                    Operand::Forward(exit)
                },
                located,
                None,
            );
            if !backward && variation != "ordinary-forward" {
                jump.jump_policy.mark_interruptible();
            }
            if variation == "folded" {
                jump.add_pre_fold_extra_instructions(1);
            }
            let plain = |opcode, operand| {
                Item::Instruction(Instruction::new(opcode, operand, located, None))
            };
            let mut items = vec![
                Item::Label(source),
                plain(NOP, Operand::Value(0)),
                if variation == "no-conditional" {
                    plain(NOP, Operand::Value(0))
                } else {
                    plain(POP_JUMP_IF_TRUE, Operand::Forward(head))
                },
                plain(NOP, Operand::Value(0)),
                plain(RETURN_VALUE, Operand::Value(0)),
                Item::Label(head),
                Item::Label(alias),
            ];
            if variation == "prefix" {
                items.push(plain(NOP, Operand::Value(0)));
            }
            items.extend([
                Item::Instruction(jump),
                Item::Label(exit),
                plain(RETURN_VALUE, Operand::Value(0)),
            ]);
            let mut assembler = Assembler::from_linear_items(items);
            assembler.next_label = 5;
            let mut pipeline = assembler.into_optimization_pipeline();
            if variation == "backward" {
                pipeline.conditional_threading_targets.insert(head, located);
            } else if variation == "backward-mismatch" || variation == "forward-already-tracked" {
                pipeline.conditional_threading_targets.insert(head, other);
            }
            if variation == "original-nop" {
                pipeline.original_nop_targets.insert(head);
            }
            if variation == "original-cleanup" {
                pipeline.original_unlocated_cleanup_targets.insert(exit);
            }
            pipeline
                .exception_owner_provenance
                .constant_pass_clusters
                .push(crate::assembler::exception_owners::ConstantPassCluster {
                    start: missing,
                    end: missing,
                    constant_location: located,
                    pass_location: located,
                });
            pipeline
        };

        for (variation, expected) in [
            ("interruptible-forward", true),
            ("backward", true),
            ("ordinary-forward", false),
            ("backward-untracked", false),
            ("backward-mismatch", false),
            ("folded", false),
            ("original-nop", false),
            ("original-cleanup", false),
            ("forward-already-tracked", false),
            ("prefix", false),
            ("no-conditional", false),
        ] {
            let mut gated = make(variation);
            let mut full = make(variation);
            gated.record_original_jump_heads();
            full.record_original_jump_heads_with_preflight::<false>();
            assert_eq!(
                gated.original_jump_head_targets.contains(&head),
                expected,
                "{variation}"
            );
            assert_eq!(
                gated.original_jump_head_targets, full.original_jump_head_targets,
                "{variation}"
            );
            assert!(
                gated
                    .exception_owner_provenance
                    .constant_pass_clusters
                    .is_empty(),
                "{variation}"
            );
            assert!(
                full.exception_owner_provenance
                    .constant_pass_clusters
                    .is_empty(),
                "{variation}"
            );
            assert_eq!(
                gated.graph.copy_items(),
                full.graph.copy_items(),
                "{variation}"
            );
        }
    }

    #[test]
    fn thread_label_offsets_match_hash_lookup_across_dense_and_sparse_labels() {
        let labels = [0, 1, 3, 7, 11, 97, 8192].map(Label::from_u32);
        for label_count in [0, 1, 4, 12, 100] {
            let mut actual = ThreadLabelOffsets::new(label_count);
            let mut expected = FxHashMap::default();
            for label in labels {
                assert_eq!(actual.get(label), expected.get(&label));
            }
            for (label, offset) in [
                (labels[0], 0),
                (labels[3], 16),
                (labels[6], 900),
                (labels[2], 16),
                (labels[1], 0),
                (labels[4], 85),
                (labels[0], 42),
                (labels[6], 0),
            ] {
                actual.insert(label, offset);
                expected.insert(label, offset);
                for label in labels {
                    assert_eq!(actual.get(label), expected.get(&label));
                    if expected.contains_key(&label) {
                        assert_eq!(actual[&label], expected[&label]);
                    }
                }
            }
        }
    }

    #[test]
    fn original_constant_return_matches_unbounded_scan() {
        fn original(graph: &BlockGraph, target: Label) -> bool {
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

        let target = Label::from_u32(0);
        let preceding = Label::from_u32(1);
        let alias = Label::from_u32(2);
        let trailing = Label::from_u32(3);
        let missing = Label::from_u32(4);
        let location = SourceLocation::new(7, 7, 2, 10);
        let instructions = [NOP, LOAD_CONST, RETURN_VALUE]
            .map(|opcode| Instruction::new(opcode, Operand::Value(0), location, None));
        let items = |instructions: &[Instruction]| {
            [
                Item::Label(preceding),
                Item::Instruction(instructions[0]),
                Item::Label(target),
                Item::Label(alias),
            ]
            .into_iter()
            .chain(instructions.iter().copied().map(Item::Instruction))
            .collect::<Vec<_>>()
        };
        let mut cases = vec![(items(&instructions), true)];
        for boundary in 1..=3 {
            let mut inline = items(&instructions);
            inline.insert(4 + boundary, Item::Label(trailing));
            cases.push((inline, false));
        }
        for count in 0..=8 {
            let mut inline = items(&instructions);
            if count < 3 {
                inline.truncate(4 + count);
            } else {
                inline.extend(std::iter::repeat_n(
                    Item::Instruction(instructions[0]),
                    count - 3,
                ));
            }
            cases.push((inline, count == 3));
        }
        for value in [0, 255, 256] {
            let mut changed = instructions;
            changed[1].opcode = crate::target::opcodes::LOAD_SMALL_INT;
            changed[1].set_operand(Operand::Value(value));
            cases.push((items(&changed), value <= 255));
        }
        for index in 0..3 {
            let mut changed = instructions;
            changed[index].location = SourceLocation::new(7, 7, 3, 10);
            cases.push((items(&changed), false));
            let mut changed = instructions;
            changed[index].add_pre_fold_extra_instructions(1);
            cases.push((items(&changed), false));
            let mut changed = instructions;
            changed[index].set_operand(Operand::Forward(missing));
            cases.push((items(&changed), false));
        }
        let unlocated = instructions.map(|mut instruction| {
            instruction.location = SourceLocation::NONE;
            instruction
        });
        cases.push((items(&unlocated), false));

        for (items, expected) in cases {
            // Bypass normalization to retain inline aliases and items after RETURN_VALUE.
            let mut graph = BlockGraph::from_items(Vec::new(), &FxHashSet::default());
            graph.push_block(items);
            assert_eq!(original(&graph, target), expected);
            assert_eq!(is_original_constant_return(&graph, target), expected);
            assert!(!is_original_constant_return(&graph, missing));
        }
    }

    #[test]
    fn shared_unlocated_forward_jump_requires_pop_conditionals_including_none_tests() {
        let first = Label::from_u32(0);
        let second = Label::from_u32(1);
        let shared = Label::from_u32(2);
        let exit = Label::from_u32(3);
        let instruction = |opcode, operand, location| {
            Item::Instruction(Instruction::new(opcode, operand, location, None))
        };
        let located = SourceLocation::new(1, 1, 0, 1);

        for (first_opcode, second_opcode, expected) in [
            (JUMP_FORWARD, JUMP_FORWARD, false),
            (POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, true),
        ] {
            let mut assembler = Assembler::from_linear_items(vec![
                Item::Label(first),
                instruction(first_opcode, Operand::Forward(shared), located),
                Item::Label(second),
                instruction(second_opcode, Operand::Forward(shared), located),
                Item::Label(shared),
                instruction(JUMP_FORWARD, Operand::Forward(exit), SourceLocation::NONE),
                Item::Label(exit),
                instruction(RETURN_VALUE, Operand::Value(0), located),
            ]);
            assembler.next_label = 4;
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.record_shared_unlocated_jump_entries();
            assert_eq!(
                pipeline.original_shared_unlocated_jump_entries.get(&shared),
                expected.then_some(&shared),
            );
        }
    }

    #[test]
    fn annotation_jump_certificate_rejects_mutation_labels_and_copies() {
        use crate::target::opcodes::STORE_NAME;

        let entry = Label::from_u32(0);
        let other = Label::from_u32(1);
        let alternate = Label::from_u32(2);
        let location = SourceLocation::new(1, 1, 0, 3);
        let mut donor = Instruction::new(STORE_NAME, Operand::Value(0), location, None);
        let mut nop = Instruction::new(NOP, Operand::Value(0), location, None);
        donor.mark_original_annotation_jump();
        nop.mark_original_annotation_jump();
        let graph_for = |items| BlockGraph::from_items(items, &[other].into_iter().collect());
        let original_items = || {
            vec![
                Item::Label(entry),
                Item::Instruction(donor),
                Item::Instruction(nop),
            ]
        };
        let graph = graph_for(original_items());
        let block = graph.label_block(entry).unwrap();
        let donor_position = ItemPosition::new(block, 1);
        let nop_position = ItemPosition::new(block, 2);
        let certificates = [(block, donor, nop)];
        let matches = |graph: &BlockGraph, previous, position| {
            certified_module_annotation_jump_nop(graph, &certificates, previous, position)
        };
        assert!(matches(&graph, Some(donor_position), nop_position));
        assert!(!matches(&graph, None, nop_position));

        let mut mutated_donor = graph.clone();
        mutated_donor
            .instruction_mut(donor_position)
            .unwrap()
            .set_operand(Operand::Value(1));
        assert!(!matches(&mutated_donor, Some(donor_position), nop_position));
        let mut mutated_nop = graph.clone();
        mutated_nop.instruction_mut(nop_position).unwrap().location = SourceLocation::NONE;
        assert!(!matches(&mutated_nop, Some(donor_position), nop_position));

        let mut labeled = graph.clone();
        labeled.insert_label(block, 2, alternate);
        assert!(!matches(
            &labeled,
            Some(donor_position),
            ItemPosition::new(block, 3)
        ));

        // A sole physical copy has identical executable fields but is not the original pair.
        let mut replaced = graph.clone();
        replaced.remove_item(nop_position);
        replaced.remove_item(donor_position);
        replaced.append_to_block(
            block,
            [
                Item::Instruction(donor.for_exit_copy()),
                Item::Instruction(nop.for_exit_copy()),
            ],
        );
        assert!(!matches(&replaced, Some(donor_position), nop_position));

        let mut cloned = graph;
        cloned.append_to_block(block, [Item::Instruction(donor), Item::Instruction(nop)]);
        assert!(!matches(&cloned, Some(donor_position), nop_position));
        assert!(!matches(
            &cloned,
            Some(ItemPosition::new(block, 3)),
            ItemPosition::new(block, 4)
        ));

        let mut other_items = original_items();
        other_items.extend([
            Item::Label(other),
            Item::Instruction(donor),
            Item::Instruction(nop),
        ]);
        let other_graph = graph_for(other_items);
        let other_block = other_graph.label_block(other).unwrap();
        assert_ne!(block, other_block);
        assert!(!matches(&other_graph, Some(donor_position), nop_position));
        assert!(!matches(
            &other_graph,
            Some(ItemPosition::new(other_block, 1)),
            ItemPosition::new(other_block, 2)
        ));

        // A changed original jump (including a new target) is not the recorded pair.
        let mut jump = Instruction::new(JUMP_FORWARD, Operand::Forward(other), location, None);
        jump.mark_original_annotation_jump();
        let mut jump_graph = graph_for(vec![
            Item::Label(entry),
            Item::Instruction(donor),
            Item::Instruction(jump),
        ]);
        assert!(unique_module_annotation_pair(&jump_graph, donor, jump));
        let jump_block = jump_graph.label_block(entry).unwrap();
        jump_graph
            .instruction_mut(ItemPosition::new(jump_block, 2))
            .unwrap()
            .set_operand(Operand::Forward(alternate));
        assert!(!unique_module_annotation_pair(&jump_graph, donor, jump));
    }

    #[test]
    fn annotation_borrow_marker_retires_only_after_unique_pair_survives() {
        use crate::target::opcodes::STORE_NAME;

        let entry = Label::from_u32(0);
        let other = Label::from_u32(1);
        let location = SourceLocation::new(1, 1, 0, 3);
        let mut donor = Instruction::new(STORE_NAME, Operand::Value(0), location, None);
        let mut nop = Instruction::new(NOP, Operand::Value(0), location, None);
        donor.mark_original_annotation_jump();
        nop.mark_original_annotation_jump();
        nop.jump_policy.retain_nop();
        nop.no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        let mut marked = nop;
        marked.jump_policy.mark_borrow_empty_block();
        let original = vec![
            Item::Label(entry),
            Item::Instruction(donor),
            Item::Instruction(marked),
        ];
        let graph = BlockGraph::from_items(original.clone(), &FxHashSet::default());
        let block = graph.label_block(entry).unwrap();
        let certificates = [(block, donor, nop)];
        let markers = surviving_module_annotation_borrow_markers(&graph, &certificates);
        assert_eq!(markers, [(donor, marked)]);

        let mut items = original.clone();
        assert!(matches!(&items[2], Item::Instruction(instruction)
            if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier));
        retire_module_annotation_borrow_markers(&mut items, &markers);
        assert!(matches!(&items[2], Item::Instruction(instruction)
            if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::BarrierIfRemoved));
        OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
        assert_eq!(items, original[..2]);

        let mut labeled_graph = graph.clone();
        labeled_graph.insert_label(block, 2, other);
        assert!(
            surviving_module_annotation_borrow_markers(&labeled_graph, &certificates).is_empty()
        );
        let mut replaced_graph = graph.clone();
        replaced_graph.remove_item(ItemPosition::new(block, 2));
        replaced_graph.remove_item(ItemPosition::new(block, 1));
        replaced_graph.append_to_block(
            block,
            [
                Item::Instruction(donor.for_exit_copy()),
                Item::Instruction(marked.for_exit_copy()),
            ],
        );
        assert!(
            surviving_module_annotation_borrow_markers(&replaced_graph, &certificates).is_empty()
        );
        let mut replaced_items = original.clone();
        replaced_items[1] = Item::Instruction(donor.for_exit_copy());
        replaced_items[2] = Item::Instruction(marked.for_exit_copy());
        retire_module_annotation_borrow_markers(&mut replaced_items, &markers);
        assert!(matches!(&replaced_items[2], Item::Instruction(instruction)
            if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier));
        let mut cloned_graph = graph;
        cloned_graph.append_to_block(block, [Item::Instruction(donor), Item::Instruction(marked)]);
        assert!(
            surviving_module_annotation_borrow_markers(&cloned_graph, &certificates).is_empty()
        );

        // A copy appearing after graph validation must also invalidate retirement.
        let mut copied_items = original;
        copied_items.extend([Item::Instruction(donor), Item::Instruction(marked)]);
        retire_module_annotation_borrow_markers(&mut copied_items, &markers);
        assert!(matches!(&copied_items[2], Item::Instruction(instruction)
            if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier));
        assert!(matches!(&copied_items[4], Item::Instruction(instruction)
            if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier));
    }

    #[test]
    fn small_scope_exit_follows_aliases_and_unlabeled_physical_blocks() {
        let target = Label::from_u32(0);
        let alias = Label::from_u32(1);
        let boundary = Label::from_u32(2);
        let missing = Label::from_u32(3);
        let separator = Label::from_u32(4);
        let prior = Label::from_u32(5);
        let located = SourceLocation::new(7, 7, 0, 1);
        let instruction = |opcode, operand, location| {
            Item::Instruction(Instruction::new(opcode, operand, location, None))
        };
        for boundaries in [FxHashSet::default(), [alias].into_iter().collect()] {
            let mut items = vec![Item::Label(prior)];
            items.extend([instruction(LOAD_CONST, Operand::Value(0), located); 5]);
            items.extend([
                Item::Label(target),
                Item::Label(alias),
                instruction(LOAD_CONST, Operand::Value(0), located),
                instruction(JUMP_FORWARD, Operand::Forward(boundary), located),
                instruction(RETURN_VALUE, Operand::Value(0), located),
                Item::Label(boundary),
                instruction(RETURN_VALUE, Operand::Value(0), SourceLocation::NONE),
            ]);
            let mut graph = BlockGraph::from_items(items, &boundaries);
            assert!(!targets_small_scope_exit(&graph, prior));
            assert!(!targets_small_scope_exit(&graph, target));
            assert!(!targets_small_scope_exit(&graph, alias));
            assert!(!targets_small_scope_exit(&graph, boundary));
            assert!(!targets_small_scope_exit(&graph, missing));

            let alias_block = graph.label_block(alias).unwrap();
            let alias_position = graph
                .order()
                .iter()
                .position(|block| *block == alias_block)
                .unwrap();
            let continuation = graph.block_at(alias_position + 1);
            graph.remove_item_at(alias_block, graph[alias_block].items().len() - 1);
            assert!(targets_small_scope_exit(&graph, target));
            assert!(targets_small_scope_exit(&graph, alias));

            graph.insert_label(continuation, 0, separator);
            assert!(!targets_small_scope_exit(&graph, target));
            assert!(!targets_small_scope_exit(&graph, alias));
            assert!(targets_small_scope_exit(&graph, separator));
        }
    }

    #[test]
    fn all_reachable_cleanup_matches_full_scan_and_respects_cached_reachability() {
        let labels = [0, 1, 2, 3, 4, 5].map(Label::from_u32);
        let [start, middle, target, end, dormant_handler, stale] = labels;
        let instruction = |opcode, operand, depth| {
            Item::Instruction(Instruction::new(
                opcode,
                operand,
                SourceLocation::new(1, 1, 0, 1),
                Some(depth),
            ))
        };
        let region = |handler| ExceptionRegion {
            start,
            end,
            target: handler,
            depth: 0,
            preserve_lasti: false,
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(start, end),
        };
        let mut folded = Instruction::new(
            LOAD_CONST,
            Operand::Value(3),
            SourceLocation::new(2, 2, 0, 1),
            Some(13),
        );
        folded.add_pre_fold_extra_instructions(1);
        let fixtures = [
            (
                vec![
                    vec![
                        Item::Label(start),
                        instruction(POP_JUMP_IF_TRUE, Operand::Forward(target), 1),
                    ],
                    vec![Item::Label(middle)],
                    vec![Item::Label(target), instruction(NOP, Operand::Value(0), 2)],
                    vec![
                        Item::Label(end),
                        instruction(RETURN_VALUE, Operand::Value(0), 3),
                    ],
                ],
                vec![],
                None,
                false,
                None,
            ),
            (
                vec![
                    vec![
                        Item::Label(start),
                        instruction(POP_JUMP_IF_TRUE, Operand::Forward(target), 1),
                    ],
                    vec![Item::Label(middle), Item::Instruction(folded)],
                    vec![Item::Label(target), instruction(NOP, Operand::Value(0), 2)],
                    vec![
                        Item::Label(end),
                        instruction(RETURN_VALUE, Operand::Value(0), 3),
                    ],
                ],
                vec![],
                Some(vec![true, false, true, true]),
                false,
                Some(13),
            ),
            (
                vec![
                    vec![
                        Item::Label(start),
                        instruction(POP_JUMP_IF_TRUE, Operand::Forward(target), 1),
                    ],
                    vec![
                        Item::Label(middle),
                        instruction(RETURN_VALUE, Operand::Value(0), 4),
                    ],
                    vec![Item::Label(target), instruction(NOP, Operand::Value(0), 2)],
                    vec![
                        Item::Label(end),
                        instruction(RETURN_VALUE, Operand::Value(0), 3),
                    ],
                ],
                vec![],
                None,
                false,
                None,
            ),
            (
                vec![
                    vec![
                        Item::Label(start),
                        instruction(RETURN_VALUE, Operand::Value(0), 1),
                    ],
                    vec![Item::Label(middle), Item::Instruction(folded)],
                    vec![
                        Item::Label(end),
                        instruction(RETURN_VALUE, Operand::Value(0), 3),
                    ],
                ],
                vec![],
                None,
                false,
                Some(13),
            ),
            (
                vec![
                    vec![Item::Label(start), instruction(NOP, Operand::Value(0), 1)],
                    vec![Item::Label(middle), instruction(NOP, Operand::Value(0), 2)],
                    vec![
                        Item::Label(end),
                        instruction(RETURN_VALUE, Operand::Value(0), 3),
                    ],
                ],
                vec![region(dormant_handler)],
                Some(vec![true, true, true, false]),
                true,
                None,
            ),
            (
                vec![
                    vec![Item::Label(start), instruction(NOP, Operand::Value(0), 1)],
                    vec![Item::Label(middle), instruction(NOP, Operand::Value(0), 2)],
                    vec![
                        Item::Label(end),
                        instruction(RETURN_VALUE, Operand::Value(0), 3),
                    ],
                ],
                vec![region(middle)],
                Some(vec![true, true, true]),
                false,
                None,
            ),
        ];
        for (blocks, regions, initial_reachable, dormant, expected_depth) in fixtures {
            let make_pipeline = || {
                let mut pipeline =
                    Assembler::from_linear_items(Vec::new()).into_optimization_pipeline();
                pipeline.graph = BlockGraph::from_items(Vec::new(), &FxHashSet::default());
                for items in &blocks {
                    pipeline.graph.push_block(items.clone());
                }
                if dormant {
                    pipeline
                        .graph
                        .allocate_block(vec![Item::Label(dormant_handler)]);
                }
                pipeline.exception_regions = regions.clone();
                pipeline.initial_reachable = initial_reachable.clone();
                pipeline.discarded_fold_admissions.insert(middle, vec![7]);
                pipeline.retired_exception_handler_targets.insert(stale);
                pipeline
            };
            let mut optimized = make_pipeline();
            let mut full = make_pipeline();
            let actual = optimized.remove_unreachable_instructions_with_proof::<true>();
            let expected = full.remove_unreachable_instructions_with_proof::<false>();
            assert_eq!(actual, expected_depth);
            assert_eq!(actual, expected);
            assert_eq!(optimized.graph.copy_items(), full.graph.copy_items());
            assert_eq!(optimized.graph.order(), full.graph.order());
            assert_eq!(optimized.initial_reachable, full.initial_reachable);
            assert_eq!(
                optimized.discarded_fold_admissions,
                full.discarded_fold_admissions
            );
            assert_eq!(
                optimized.retired_exception_handler_targets,
                full.retired_exception_handler_targets
            );
            if dormant {
                assert!(
                    optimized
                        .retired_exception_handler_targets
                        .contains(&dormant_handler)
                );
            }
        }
    }

    #[test]
    fn nop_pass_forward_jump_summary_matches_exact_late_cleanup_with_stale_features() {
        let [source, fallthrough, target] = [0, 1, 2].map(Label::from_u32);
        let located = SourceLocation::new(1, 1, 0, 1);
        let item = |opcode, operand, location| {
            Item::Instruction(Instruction::new(opcode, operand, location, None))
        };
        let actual_forward_jump = |pipeline: &OptimizationPipeline| {
            pipeline.graph.iter_items().any(|item| {
                matches!(item, Item::Instruction(instruction)
                    if instruction.opcode == JUMP_FORWARD
                        && matches!(instruction.operand(), Operand::Forward(_)))
            })
        };
        for nops in [0, 1, 2, 3] {
            for protected in [false, true] {
                for jump_kind in 0..5 {
                    let make_pipeline = || {
                        let mut items = vec![
                            Item::Label(source),
                            item(LOAD_CONST, Operand::Value(0), located),
                            item(POP_JUMP_IF_FALSE, Operand::Forward(target), located),
                            Item::Label(fallthrough),
                            item(LOAD_CONST, Operand::Value(0), located),
                            Item::Label(target),
                            item(RETURN_VALUE, Operand::Value(0), located),
                        ];
                        if nops & 1 != 0 {
                            items.insert(1, item(NOP, Operand::Value(0), located));
                        }
                        if nops & 2 != 0 {
                            items.insert(items.len() - 1, item(NOP, Operand::Value(0), located));
                        }
                        let mut assembler = Assembler::from_linear_items(items);
                        assembler.next_label = 3;
                        let mut pipeline = assembler.into_optimization_pipeline();
                        assert!(pipeline.graph.features().has_jump());
                        assert!(!pipeline.graph.features().has_forward_jump());
                        if protected {
                            pipeline.exception_regions.push(ExceptionRegion {
                                start: source,
                                end: target,
                                target,
                                depth: 0,
                                preserve_lasti: false,
                                is_scope_fallback: false,
                                priority: ExceptionRegionPriority::OriginalRange(source, target),
                            });
                        }
                        let jump = match jump_kind {
                            0 => None,
                            1 => Some((JUMP_FORWARD, Operand::Forward(target))),
                            2 => Some((JUMP_BACKWARD, Operand::Forward(target))),
                            3 => Some((JUMP_FORWARD, Operand::Backward(source))),
                            4 => Some((POP_JUMP_IF_NONE, Operand::Forward(target))),
                            _ => unreachable!(),
                        };
                        if let Some((opcode, operand)) = jump {
                            let block = pipeline.graph.label_block(fallthrough).unwrap();
                            pipeline.graph.append_to_block(
                                block,
                                [item(opcode, operand, SourceLocation::NONE)],
                            );
                            assert!(!pipeline.graph.features().has_forward_jump());
                        }
                        pipeline
                    };
                    let mut optimized = make_pipeline();
                    let mut reference = make_pipeline();
                    let summary = optimized.remove_redundant_nops();
                    reference.remove_redundant_nops();
                    assert_eq!(summary.has_forward_jump, jump_kind == 1);
                    assert_eq!(summary.has_forward_jump, actual_forward_jump(&optimized));
                    assert_eq!(summary.may_have_changed, nops != 0);
                    assert_eq!(optimized.graph.copy_items(), reference.graph.copy_items());
                    if summary.has_forward_jump {
                        optimized.remove_redundant_forward_jumps(true);
                    }
                    reference.remove_redundant_forward_jumps(true);
                    assert_eq!(optimized.graph.copy_items(), reference.graph.copy_items());
                    assert_eq!(optimized.graph.order(), reference.graph.order());
                    if jump_kind == 1 && !protected && nops == 0 {
                        assert!(!actual_forward_jump(&optimized));
                    }
                }
            }
        }
    }

    #[test]
    fn nop_summary_preserves_borrow_marker_jump_retries() {
        let [start, first_target, second_target] = [0, 1, 2].map(Label::from_u32);
        let located = SourceLocation::new(1, 1, 0, 1);
        for marker_location in [SourceLocation::NONE, SourceLocation::new(2, 2, 0, 1)] {
            for marker_fallthrough in [
                BorrowFallthrough::Reachable,
                BorrowFallthrough::BarrierIfRemoved,
                BorrowFallthrough::Barrier,
            ] {
                for retain_marker in [false, true] {
                    for second_policy in [
                        "none", "ordinary", "deferred", "borrowed", "retained", "barrier",
                    ] {
                        let make_pipeline = || {
                            let mut marker =
                                Instruction::new(NOP, Operand::Value(0), marker_location, None);
                            marker.jump_policy.mark_borrow_empty_block();
                            if retain_marker {
                                marker.jump_policy.retain_nop();
                            }
                            marker
                                .no_location_policy
                                .set_borrow_fallthrough(marker_fallthrough);
                            let mut items = vec![
                                Item::Label(start),
                                Item::Instruction(Instruction::new(
                                    LOAD_CONST,
                                    Operand::Value(0),
                                    located,
                                    None,
                                )),
                                Item::Instruction(Instruction::new(
                                    JUMP_FORWARD,
                                    Operand::Forward(first_target),
                                    located,
                                    None,
                                )),
                                Item::Instruction(marker),
                                Item::Label(first_target),
                            ];
                            if second_policy != "none" {
                                let mut jump = Instruction::new(
                                    JUMP_FORWARD,
                                    Operand::Forward(second_target),
                                    SourceLocation::new(3, 3, 0, 1),
                                    None,
                                );
                                match second_policy {
                                    "deferred" => jump.jump_policy.defer_removal(),
                                    "borrowed" => jump.jump_policy.mark_borrow_empty_block(),
                                    "retained" => jump.jump_policy.retain_nop(),
                                    "barrier" => jump
                                        .no_location_policy
                                        .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                                    "ordinary" => {}
                                    _ => unreachable!(),
                                }
                                items.push(Item::Instruction(jump));
                            }
                            items.extend([
                                Item::Label(second_target),
                                Item::Instruction(Instruction::new(
                                    RETURN_VALUE,
                                    Operand::Value(0),
                                    SourceLocation::new(3, 3, 0, 1),
                                    None,
                                )),
                            ]);
                            let mut assembler = Assembler::from_linear_items(items);
                            assembler.next_label = 3;
                            assembler.into_optimization_pipeline()
                        };
                        let mut gated = make_pipeline();
                        let mut reference = make_pipeline();
                        let summary = gated.remove_redundant_nops();
                        reference.remove_redundant_nops();
                        let actual_forward_jump = gated.graph.iter_items().any(|item| {
                            matches!(item, Item::Instruction(instruction)
                                if instruction.opcode == JUMP_FORWARD
                                    && matches!(instruction.operand(), Operand::Forward(_)))
                        });
                        assert_eq!(summary.has_forward_jump, actual_forward_jump);
                        if summary.has_forward_jump {
                            gated.remove_redundant_forward_jumps(true);
                        }
                        reference.remove_redundant_forward_jumps(true);
                        assert_eq!(gated.graph.copy_items(), reference.graph.copy_items());
                        assert_eq!(gated.graph.order(), reference.graph.order());

                        if marker_location.line >= 0
                            && marker_fallthrough == BorrowFallthrough::BarrierIfRemoved
                            && retain_marker
                            && second_policy == "barrier"
                        {
                            let mut items = gated.graph.into_items();
                            OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
                            let first_jump_survives = items.iter().any(|item| {
                                matches!(item, Item::Instruction(instruction)
                                    if instruction.opcode == JUMP_FORWARD
                                        && instruction.operand() == Operand::Forward(first_target))
                            });
                            let marker_survives = items.iter().any(|item| {
                                matches!(item, Item::Instruction(instruction)
                                    if instruction.opcode == NOP
                                        && instruction.location == marker_location)
                            });
                            assert!(first_jump_survives || !marker_survives);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn manager_nop_reports_edits_only_when_its_candidate_walk_can_mutate() {
        let location = SourceLocation::new(4, 4, 0, 1);
        let other = SourceLocation::new(5, 5, 0, 1);
        let value = |opcode, location| Instruction::new(opcode, Operand::Value(0), location, None);
        for has_nop in [false, true] {
            for converted in [false, true] {
                for borrow_empty in [false, true] {
                    for tracked in [None, Some(location), Some(other)] {
                        let mut items = vec![Item::Instruction(value(POP_TOP, location))];
                        if has_nop {
                            let mut nop = value(NOP, location);
                            if converted {
                                nop.fusion_policy.mark_converted_pop_block();
                            }
                            if borrow_empty {
                                nop.jump_policy.mark_borrow_empty_block();
                            }
                            items.push(Item::Instruction(nop));
                        }
                        items.push(Item::Instruction(value(RETURN_VALUE, location)));
                        let mut pipeline =
                            Assembler::from_linear_items(items).into_optimization_pipeline();
                        pipeline
                            .exception_owner_provenance
                            .with_context_locations
                            .extend(tracked);
                        let before = pipeline.graph.copy_items();
                        let may_have_changed = pipeline.remove_redundant_with_exit_nops();
                        let expected =
                            has_nop && converted && !borrow_empty && tracked == Some(location);
                        assert_eq!(may_have_changed, expected);
                        if expected {
                            assert_ne!(pipeline.graph.copy_items(), before);
                        } else {
                            assert_eq!(pipeline.graph.copy_items(), before);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn match_success_threading_respects_original_nop_aliases() {
        let [entry, head, alias, later, exit, bridge] = [0, 1, 2, 3, 4, 5].map(Label::from_u32);
        let location = SourceLocation::new(2, 2, 0, 1);
        for (through_bridge, marker, expected) in [
            (false, Some(head), alias),
            (false, Some(alias), alias),
            (false, Some(later), exit),
            (false, None, exit),
            (true, Some(head), alias),
            (true, Some(alias), alias),
            (true, Some(later), exit),
            (true, None, exit),
        ] {
            let mut source = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(if through_bridge { bridge } else { alias }),
                SourceLocation::NONE,
                Some(0),
            );
            source.jump_policy.mark_original_match_success();
            source.jump_policy.retain_nop_for_direct_target();
            let mut items = vec![Item::Label(entry), Item::Instruction(source)];
            if through_bridge {
                items.extend([
                    Item::Label(bridge),
                    Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(alias),
                        location,
                        Some(0),
                    )),
                ]);
            }
            items.extend([
                Item::Label(head),
                Item::Label(alias),
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(exit),
                    location,
                    Some(0),
                )),
                Item::Label(later),
                Item::Instruction(Instruction::new(NOP, Operand::Value(0), location, Some(0))),
                Item::Label(exit),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    location,
                    Some(0),
                )),
            ]);
            let mut assembler = Assembler::from_linear_items(items);
            assembler.next_label = 6;
            assembler.python_version = crate::PythonVersion::Py314;
            let mut pipeline = assembler.into_optimization_pipeline();
            if let Some(marker) = marker {
                pipeline.original_nop_targets.insert(marker);
            }
            pipeline.thread_forward_jumps();
            let Item::Instruction(actual) = pipeline
                .graph
                .iter_items()
                .find(|item| matches!(item, Item::Instruction(_)))
                .copied()
                .expect("source jump")
            else {
                unreachable!();
            };
            assert_eq!(
                actual.operand(),
                Operand::Forward(expected),
                "{through_bridge} {marker:?}"
            );
        }
    }

    #[test]
    fn threaded_target_head_gate_matches_literal_unfiltered_pipeline() {
        let [entry, head, alias, exit, tail] = [0, 1, 2, 3, 4].map(Label::from_u32);
        let location = SourceLocation::new(1, 1, 0, 2);
        let ordinary = |opcode, operand| {
            Item::Instruction(Instruction::new(opcode, operand, location, Some(0)))
        };
        let cases = [
            (NOP, Operand::Value(0)),
            (LOAD_CONST, Operand::Value(0)),
            (RETURN_VALUE, Operand::Value(0)),
            (POP_JUMP_IF_FALSE, Operand::Forward(exit)),
            (POP_JUMP_IF_NONE, Operand::Forward(exit)),
            (JUMP_FORWARD, Operand::Forward(exit)),
            (JUMP_BACKWARD, Operand::Backward(entry)),
            (JUMP_BACKWARD_NO_INTERRUPT, Operand::Backward(entry)),
            (JUMP_BACKWARD, Operand::Forward(exit)),
            (JUMP_FORWARD, Operand::Value(0)),
        ];
        for (case, (opcode, operand)) in cases.into_iter().enumerate() {
            for policy in 0..3 {
                for source_provenance in [false, true] {
                    let make = || {
                        let mut target = Instruction::new(opcode, operand, location, Some(0));
                        if policy == 1 {
                            target.jump_policy.prevent_threading_target();
                        } else if policy == 2 {
                            target.jump_policy.retain_nop();
                        }
                        let mut assembler = Assembler::from_linear_items(vec![
                            Item::Label(entry),
                            ordinary(LOAD_CONST, Operand::Value(0)),
                            ordinary(JUMP_FORWARD, Operand::Forward(head)),
                            Item::Label(head),
                            Item::Label(alias),
                            Item::Instruction(target),
                            Item::Label(exit),
                            ordinary(LOAD_CONST, Operand::Value(0)),
                            ordinary(RETURN_VALUE, Operand::Value(0)),
                            Item::Label(tail),
                        ]);
                        assembler.next_label = 5;
                        let mut pipeline = assembler.into_optimization_pipeline();
                        if source_provenance {
                            pipeline.original_nop_targets.insert(alias);
                            pipeline
                                .exception_owner_provenance
                                .block_boundaries
                                .insert(alias);
                            pipeline
                                .exception_owner_provenance
                                .source_boundaries
                                .insert(alias);
                        }
                        pipeline
                    };
                    let mut gated = make();
                    let mut original = make();
                    let actual = gated.thread_forward_jumps_with_head_gate::<true>();
                    let expected = original.thread_forward_jumps_with_head_gate::<false>();
                    assert_eq!(actual, expected, "{case}/{policy}/{source_provenance}");
                    assert_eq!(
                        gated.graph.copy_items(),
                        original.graph.copy_items(),
                        "{case}/{policy}/{source_provenance}"
                    );
                    assert_eq!(gated.graph.order(), original.graph.order());
                    assert_eq!(gated.next_label, original.next_label);
                    assert_eq!(
                        gated.borrow_threaded_jump_targets,
                        original.borrow_threaded_jump_targets
                    );
                    assert_eq!(gated.original_nop_targets, original.original_nop_targets);
                    if opcode == JUMP_FORWARD
                        && operand == Operand::Forward(exit)
                        && policy == 0
                        && !source_provenance
                    {
                        assert!(actual);
                    }
                }
            }
        }
    }

    #[test]
    fn setup_nop_requires_its_original_predecessor_to_survive() {
        let start = Label::from_u32(0);
        let exclusion = Label::from_u32(1);
        let setup = Label::from_u32(2);
        let handler = Label::from_u32(3);
        let location = SourceLocation::new(7, 7, 20, 21);
        let instruction = |opcode, line, column| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(0),
                SourceLocation::new(line, line, column, column + 1),
                None,
            ))
        };

        for removed_original in [false, true] {
            for trailing_setup in [false, true] {
                let mut items = vec![Item::Label(start), instruction(LOAD_CONST, 7, 0)];
                if removed_original {
                    // The next original instruction represents this line, so this NOP is
                    // removed even though its own physical successor is in another block.
                    items.push(instruction(NOP, 7, 10));
                }
                items.extend([Item::Label(exclusion), instruction(NOP, 7, 20)]);
                items.push(Item::Label(setup));
                if !trailing_setup {
                    items.push(instruction(LOAD_CONST, 8, 0));
                }
                items.push(Item::Label(handler));
                if !trailing_setup {
                    items.push(instruction(RETURN_VALUE, 8, 1));
                }

                let mut assembler = Assembler::from_linear_items(items);
                assembler.next_label = 4;
                let mut pipeline = assembler.into_optimization_pipeline();
                pipeline.exception_regions.push(ExceptionRegion {
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(start, exclusion),
                    start,
                    end: exclusion,
                    target: handler,
                    depth: 0,
                    preserve_lasti: false,
                });
                pipeline
                    .exception_owner_provenance
                    .setups
                    .push((setup, Some(location)));
                pipeline
                    .exception_owner_provenance
                    .setup_nop_ranges
                    .push((exclusion, setup));

                pipeline.remove_redundant_nops();

                let remaining_nop_columns = pipeline
                    .graph
                    .iter_items()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) if instruction.opcode == NOP => {
                            Some(instruction.location.column)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    remaining_nop_columns,
                    if removed_original { vec![20] } else { vec![] },
                    "removed original={removed_original}, trailing setup={trailing_setup}",
                );
            }
        }
    }
}
