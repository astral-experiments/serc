use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    BorrowFallthrough, COMPARE_OP, CONTAINS_OP, COPY, ExceptionExclusion, IS_OP, Instruction, Item,
    JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, LOAD_COMMON_CONSTANT, NOP, NOT_TAKEN,
    Operand, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, POP_TOP, STORE_FAST, STORE_FAST_STORE_FAST,
    TO_BOOL, UNARY_NOT, UNPACK_SEQUENCE, ends_scope, is_unconditional_jump,
};
use crate::assembler::OptimizationPipeline;
use crate::assembler::block_graph::{BlockGraph, ItemMut, ItemPosition};
use crate::target::Opcode;
use crate::target::opcodes::{POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, RAISE_VARARGS};
use crate::target::operands::ComparisonOperation;

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

    pub(in crate::assembler) fn remove_unreachable_instructions(&mut self) -> Option<u32> {
        let graph = &mut self.graph;
        if self.exception_regions.is_empty() && graph.len() == 1 {
            return None;
        }
        let reachable = self
            .initial_reachable
            .take()
            .unwrap_or_else(|| graph.reachable_blocks(&self.exception_regions));
        let mut removed_max_depth = None;
        let order = graph.order().to_vec();
        let mut empty = FxHashSet::default();
        for block in order {
            if reachable[block.index()] {
                continue;
            }
            graph.retain_items(block, |item| match item {
                Item::Label(_) => true,
                Item::Instruction(instruction) => {
                    if let Some(depth) = instruction.depth_after() {
                        removed_max_depth =
                            Some(removed_max_depth.map_or(depth, |max: u32| max.max(depth)));
                    }
                    false
                }
            });
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

    pub(in crate::assembler) fn optimize_boolean_conversions(&mut self) {
        let graph = &mut self.graph;
        if !graph.features().has_boolean_conversion() {
            return;
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
    }

    pub(in crate::assembler) fn thread_forward_jumps(&mut self) -> bool {
        #[derive(Clone, Copy)]
        struct ConditionalTarget {
            opcode: Opcode,
            operand: Operand,
            fallthrough_index: usize,
        }

        fn is_value_preserving_jump(
            graph: &BlockGraph,
            positions: &[ItemPosition],
            index: usize,
        ) -> bool {
            let Item::Instruction(instruction) = *graph.item(positions[index]) else {
                return false;
            };
            if !matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                || !matches!(instruction.operand(), Operand::Forward(_))
            {
                return false;
            }
            let mut previous = positions[..index].iter().rev().filter_map(|position| {
                let Item::Instruction(instruction) = *graph.item(*position) else {
                    return None;
                };
                (instruction.opcode != NOP).then_some(instruction)
            });
            let Some(to_bool) = previous.next() else {
                return false;
            };
            let Some(copy) = previous.next() else {
                return false;
            };
            to_bool.opcode == TO_BOOL
                && copy.opcode == COPY
                && matches!(copy.operand(), Operand::Value(1))
        }

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
                        if !is_value_preserving_jump(graph, &positions, index) {
                            return None;
                        }
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
                    if !is_value_preserving_jump(&self.graph, &positions, index) {
                        continue;
                    }
                    let Some(instruction) = self.graph.instruction_mut(position) else {
                        continue;
                    };
                    if !matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE) {
                        continue;
                    }
                    let Operand::Forward(mut target) = instruction.operand() else {
                        continue;
                    };
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
                }
            }
        }

        if !has_forward_jump {
            return changed;
        }

        let graph = &self.graph;
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
                            && is_unconditional_jump(instruction.opcode)
                            && let operand @ (Operand::Forward(_) | Operand::Backward(_)) =
                                instruction.operand()
                        {
                            let target = &mut jump_targets
                                [usize::try_from(label.0).expect("label index fits in usize")];
                            jump_target_count += usize::from(
                                target
                                    .replace((
                                        operand,
                                        instruction.opcode,
                                        instruction.no_location_policy.is_artificial_eval_break(),
                                    ))
                                    .is_none(),
                            );
                        }
                    }
                }
            }
        }

        let graph = &mut self.graph;
        graph.for_each_instruction_mut(|instruction| {
            if instruction.opcode != JUMP_FORWARD || instruction.jump_policy.always_retains_nop() {
                return;
            }
            let Operand::Forward(mut target) = instruction.operand() else {
                return;
            };
            let original_target = target;
            let mut operand = instruction.operand();
            let mut opcode = instruction.opcode;
            let mut artificial_eval_break =
                instruction.no_location_policy.is_artificial_eval_break();
            for _ in 0..jump_target_count {
                let Some((next_operand, next_opcode, next_artificial_eval_break)) = jump_targets
                    .get(usize::try_from(target.0).expect("label index fits in usize"))
                    .copied()
                    .flatten()
                else {
                    break;
                };
                let next = match next_operand {
                    Operand::Forward(next) | Operand::Backward(next) => next,
                    Operand::Value(_) | Operand::Deferred(_) => unreachable!(),
                };
                if next == target {
                    break;
                }
                operand = next_operand;
                if next_opcode == JUMP_BACKWARD {
                    opcode = JUMP_BACKWARD;
                    artificial_eval_break |= next_artificial_eval_break;
                } else if next_opcode == JUMP_BACKWARD_NO_INTERRUPT {
                    opcode = JUMP_BACKWARD_NO_INTERRUPT;
                }
                target = next;
            }
            changed |= target != original_target;
            instruction.retarget(operand);
            instruction.opcode = opcode;
            if artificial_eval_break {
                instruction.no_location_policy.mark_artificial_eval_break();
            }
        });
        changed
    }

    pub(in crate::assembler) fn remove_redundant_forward_jumps(
        &mut self,
        finalize_borrow_empty_blocks: bool,
    ) {
        let graph = &mut self.graph;
        if !finalize_borrow_empty_blocks && !graph.features().has_forward_jump() {
            return;
        }
        let mut target = None;
        let mut has_work = false;
        for item in graph.iter_items() {
            match item {
                Item::Label(label) if target == Some(*label) => {
                    has_work = true;
                    break;
                }
                Item::Label(_) => {}
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
        let order = graph.order().to_vec();
        let mut previous = None;
        let mut redundant = Vec::new();
        for (block_index, block) in order.iter().copied().enumerate() {
            for offset in 0..graph[block].items().len() {
                let position = ItemPosition::new(block, offset);
                let Item::Instruction(instruction) = *graph.item(position) else {
                    continue;
                };
                let Operand::Forward(target) = instruction.operand() else {
                    previous = Some(position);
                    continue;
                };
                if instruction.opcode != JUMP_FORWARD {
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
                if instruction.jump_policy.borrow_empty_block() && !finalize_borrow_empty_blocks {
                    previous = Some(position);
                    continue;
                }

                let mut target_is_next = false;
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
                            Item::Label(label) => target_is_next |= *label == target,
                            Item::Instruction(_) => {
                                next = Some(ItemPosition::new(next_block, next_offset));
                                break 'next;
                            }
                        }
                    }
                }
                if !target_is_next {
                    previous = Some(position);
                    continue;
                }
                if instruction.jump_policy.borrow_empty_block() {
                    let instruction = graph.instruction_mut(position).unwrap();
                    instruction.opcode = NOP;
                    instruction.set_operand(Operand::Value(0));
                    previous = Some(position);
                    continue;
                }

                let previous_line = previous.map(|previous| {
                    let Item::Instruction(instruction) = *graph.item(previous) else {
                        unreachable!();
                    };
                    instruction.location.line
                });
                let next_line = next.map(|next| {
                    let Item::Instruction(instruction) = *graph.item(next) else {
                        unreachable!();
                    };
                    instruction.location.line
                });
                if !instruction.jump_policy.always_retains_nop()
                    && (instruction.location.line < 0
                        || previous_line == Some(instruction.location.line)
                        || next_line == Some(instruction.location.line))
                {
                    redundant.push(position);
                    if instruction.exception_policy.exclusion() == ExceptionExclusion::IfExtended
                        && let Some(previous) = previous
                    {
                        graph
                            .instruction_mut(previous)
                            .unwrap()
                            .exception_policy
                            .exclude(ExceptionExclusion::IfExtended);
                    }
                    if instruction.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Barrier
                        && let Some(previous) = previous
                    {
                        graph
                            .instruction_mut(previous)
                            .unwrap()
                            .no_location_policy
                            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
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

        for position in redundant.into_iter().rev() {
            graph.remove_item(position);
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
        let graph = &self.graph;
        let instructions = graph.order().iter().copied().flat_map(|block| {
            graph[block]
                .items()
                .iter()
                .enumerate()
                .filter_map(move |(offset, item)| {
                    let Item::Instruction(instruction) = item else {
                        return None;
                    };
                    Some((ItemPosition::new(block, offset), instruction))
                })
        });
        let mut instructions = instructions.peekable();
        let mut previous = None;
        let mut redundant = Vec::new();
        while let Some((position, instruction)) = instructions.next() {
            let next = instructions.peek().map(|(_, instruction)| *instruction);
            let remove = instruction.opcode == NOP
                && !instruction.jump_policy.preserves_explicit_nop()
                && (instruction.location.line < 0
                    || (instruction.fusion_policy.is_converted_pop_block()
                        && previous.is_some_and(|previous: &Instruction| {
                            previous.location.line == instruction.location.line
                                && !(ends_scope(previous.opcode)
                                    || is_unconditional_jump(previous.opcode))
                        }))
                    || next.is_some_and(|next| next.location.line == instruction.location.line)
                    // CPython deletes this traced rotation slot after applying an unpacked
                    // pattern's stack permutation to its local capture stores.
                    || (previous.is_some_and(|previous: &Instruction| {
                        previous.opcode == UNPACK_SEQUENCE
                    }) && next.is_some_and(|next| {
                        matches!(next.opcode, STORE_FAST | STORE_FAST_STORE_FAST)
                    })));
            if remove {
                redundant.push(position);
            } else {
                previous = Some(instruction);
            }
        }

        let graph = &mut self.graph;
        for position in redundant.into_iter().rev() {
            graph.remove_item(position);
        }
    }
}
