use rustc_hash::FxHashSet;

use super::super::{
    BorrowFallthrough, CHECK_EXC_MATCH, COMPARE_OP, COPY, DELETE_DEREF, DELETE_FAST, DICT_MERGE,
    DICT_UPDATE, END_SEND, FOR_ITER, FORMAT_SIMPLE, GET_ANEXT, GET_LEN, GET_YIELD_FROM_ITER,
    IMPORT_FROM, Instruction, Item, LIST_APPEND, LIST_EXTEND, LOAD_ATTR, LOAD_COMMON_CONSTANT,
    LOAD_CONST, LOAD_FAST, LOAD_FAST_AND_CLEAR, LOAD_FAST_BORROW,
    LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_CHECK, LOAD_FAST_LOAD_FAST, LOAD_SMALL_INT,
    LOAD_SPECIAL, LOAD_SUPER_ATTR, MAP_ADD, MATCH_KEYS, MATCH_MAPPING, MATCH_SEQUENCE, NOP,
    NOT_TAKEN, Operand, POP_EXCEPT, POP_TOP, PUSH_EXC_INFO, RERAISE, RETURN_VALUE, SEND, SET_ADD,
    SET_FUNCTION_ATTRIBUTE, SET_UPDATE, STORE_DEREF, STORE_FAST, STORE_FAST_LOAD_FAST,
    STORE_FAST_STORE_FAST, STORE_GLOBAL, SWAP, WITH_EXCEPT_START, block_has_fallthrough,
    block_jump_target, ends_scope, opcode_num_popped, opcode_num_pushed, opcode_stack_effect,
};
use crate::assembler::OptimizationPipeline;

impl OptimizationPipeline {
    /// Converts potentially uninitialized local loads to checked loads.
    ///
    /// Mirrors CPython's `add_checks_for_loads_of_uninitialized_variables`
    /// flow-graph pass. A set bit means that a local may be uninitialized on
    /// at least one path into the block.
    pub(in crate::assembler) fn add_checks_for_uninitialized_loads(
        &mut self,
        local_count: usize,
        parameter_count: usize,
    ) {
        if local_count == 0 {
            return;
        }

        let graph = &self.graph;
        if !graph.iter_items().any(|item| {
            matches!(
                item,
                Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_CHECK)
            )
        }) {
            return;
        }
        let order = graph.order().to_vec();
        if order.is_empty() {
            return;
        }
        if self.exception_regions.is_empty()
            && order.len() == 1
            && block_jump_target(graph[order[0]].items()).is_none()
        {
            let graph = &mut self.graph;
            let mut unsafe_locals = vec![true; local_count];
            unsafe_locals[..parameter_count.min(local_count).min(64)].fill(false);
            for instruction in graph.instructions_mut(order[0]) {
                if instruction.local_policy.is_post_analysis_closure_load()
                    || instruction.local_policy.is_known_initialized_load()
                {
                    continue;
                }
                let Operand::Value(argument) = instruction.operand else {
                    continue;
                };
                let Ok(local) = usize::try_from(argument) else {
                    continue;
                };
                if local >= local_count {
                    continue;
                }
                match instruction.opcode {
                    DELETE_FAST | LOAD_FAST_AND_CLEAR => unsafe_locals[local] = true,
                    STORE_FAST => {
                        unsafe_locals[local] = instruction.local_policy.may_store_null();
                    }
                    LOAD_FAST_CHECK => {
                        if !unsafe_locals[local] {
                            instruction.opcode = LOAD_FAST;
                        }
                        unsafe_locals[local] = false;
                    }
                    LOAD_FAST => {
                        if unsafe_locals[local] {
                            instruction.opcode = LOAD_FAST_CHECK;
                        }
                        unsafe_locals[local] = false;
                    }
                    _ => {}
                }
            }
            return;
        }
        let block_slot_count = order
            .iter()
            .map(|block| block.index())
            .max()
            .map_or(0, |index| index + 1);
        let mut block_indices = vec![usize::MAX; block_slot_count];
        let mut block_starts = vec![0; block_slot_count];
        let mut item_count = 0;
        for (index, block) in order.iter().copied().enumerate() {
            block_indices[block.index()] = index;
            block_starts[block.index()] = item_count;
            item_count += graph[block].items().len();
        }
        let positions = graph.item_positions();
        debug_assert_eq!(positions.len(), item_count);
        let mut label_positions = vec![None; usize::try_from(self.next_label).unwrap()];
        for (index, position) in positions.iter().enumerate() {
            if let Item::Label(label) = graph.item(*position) {
                label_positions[usize::try_from(label.0).unwrap()] = Some(index);
            }
        }

        let mut exception_successors = vec![None; positions.len()];
        for (index, position) in positions.iter().copied().enumerate() {
            if !matches!(graph.item(position), Item::Instruction(_)) {
                continue;
            }
            let mut innermost = None::<(usize, usize, usize)>;
            for region in &self.exception_regions {
                let (Some(start), Some(end), Some(target)) = (
                    label_positions
                        .get(usize::try_from(region.start.0).unwrap())
                        .copied()
                        .flatten(),
                    label_positions
                        .get(usize::try_from(region.end.0).unwrap())
                        .copied()
                        .flatten(),
                    graph.label_block(region.target).and_then(|block| {
                        block_indices
                            .get(block.index())
                            .copied()
                            .filter(|index| *index != usize::MAX)
                    }),
                ) else {
                    continue;
                };
                if start <= index && index < end {
                    let span = end - start;
                    if innermost.is_none_or(|(best_start, best_span, _)| {
                        start > best_start || (start == best_start && span < best_span)
                    }) {
                        innermost = Some((start, span, target));
                    }
                }
            }
            if let Some((_, _, target)) = innermost {
                exception_successors[index] = Some(target);
            }
        }

        let initialized = parameter_count.min(local_count).min(64);
        let mut unsafe_at_entry = vec![0_u64; order.len()];
        unsafe_at_entry[0] = u64::MAX
            .checked_shl(u32::try_from(initialized).unwrap())
            .unwrap_or(0);
        let mut unsafe_high_locals = vec![true; local_count.saturating_sub(64)];
        let mut needs_check = vec![false; item_count];
        let mut checked_loads_needing_check = vec![false; item_count];
        let mut pending = (0..order.len()).collect::<Vec<_>>();
        let mut queued = vec![true; order.len()];

        while let Some(block_index) = pending.pop() {
            queued[block_index] = false;
            let mut unsafe_locals = unsafe_at_entry[block_index];
            unsafe_high_locals.fill(true);

            let mut propagate = |target: usize, state: u64| {
                let current = &mut unsafe_at_entry[target];
                let changed = state & !*current != 0;
                *current |= state;
                if changed && !queued[target] {
                    queued[target] = true;
                    pending.push(target);
                }
            };

            let block = order[block_index];
            for (offset, item) in graph[block].items().iter().enumerate() {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                if instruction.local_policy.is_post_analysis_closure_load()
                    || instruction.local_policy.is_known_initialized_load()
                {
                    continue;
                }
                if let Some(target) = exception_successors[block_starts[block.index()] + offset] {
                    propagate(target, unsafe_locals);
                }
                let Operand::Value(argument) = instruction.operand else {
                    continue;
                };
                let Ok(local) = usize::try_from(argument) else {
                    continue;
                };
                if local >= local_count {
                    continue;
                }
                match instruction.opcode {
                    DELETE_FAST | LOAD_FAST_AND_CLEAR => {
                        if local < 64 {
                            unsafe_locals |= 1 << local;
                        } else {
                            unsafe_high_locals[local - 64] = true;
                        }
                    }
                    STORE_FAST => {
                        if local < 64 {
                            if instruction.local_policy.may_store_null() {
                                unsafe_locals |= 1 << local;
                            } else {
                                unsafe_locals &= !(1 << local);
                            }
                        } else {
                            unsafe_high_locals[local - 64] =
                                instruction.local_policy.may_store_null();
                        }
                    }
                    LOAD_FAST_CHECK => {
                        let is_unsafe = if local < 64 {
                            unsafe_locals & (1 << local) != 0
                        } else {
                            unsafe_high_locals[local - 64]
                        };
                        if is_unsafe {
                            checked_loads_needing_check[block_starts[block.index()] + offset] =
                                true;
                        }
                        if local < 64 {
                            unsafe_locals &= !(1 << local);
                        } else {
                            unsafe_high_locals[local - 64] = false;
                        }
                    }
                    LOAD_FAST => {
                        let is_unsafe = if local < 64 {
                            unsafe_locals & (1 << local) != 0
                        } else {
                            unsafe_high_locals[local - 64]
                        };
                        if is_unsafe {
                            needs_check[block_starts[block.index()] + offset] = true;
                        }
                        if local < 64 {
                            unsafe_locals &= !(1 << local);
                        } else {
                            unsafe_high_locals[local - 64] = false;
                        }
                    }
                    _ => {}
                }
            }

            if block_has_fallthrough(graph[block].items()) && block_index + 1 < order.len() {
                propagate(block_index + 1, unsafe_locals);
            }
            if let Some(target) = block_jump_target(graph[block].items())
                && let Some(target) = graph.label_block(target)
            {
                if let Some(target) = block_indices
                    .get(target.index())
                    .copied()
                    .filter(|target| *target != usize::MAX)
                {
                    propagate(target, unsafe_locals);
                }
            }
        }

        let graph = &mut self.graph;
        for (index, position) in positions.into_iter().enumerate() {
            let Some(instruction) = graph.instruction_mut(position) else {
                continue;
            };
            match instruction.opcode {
                LOAD_FAST_CHECK if !checked_loads_needing_check[index] => {
                    instruction.opcode = LOAD_FAST;
                }
                LOAD_FAST if needs_check[index] => {
                    instruction.opcode = LOAD_FAST_CHECK;
                }
                _ => {}
            }
        }
    }

    /// Removes repeated checked loads within a basic block.
    ///
    /// Once a checked load succeeds, CPython's definite-assignment scan treats
    /// that local as initialized for subsequent instructions on the same path.
    pub(in crate::assembler) fn remove_redundant_checked_loads(&mut self) {
        let graph = &mut self.graph;
        if !graph.iter_items().any(|item| {
            matches!(item, Item::Instruction(instruction) if instruction.opcode == LOAD_FAST_CHECK)
        }) {
            return;
        }
        let order = graph.order().to_vec();
        for block in order {
            let mut initialized = FxHashSet::default();
            for instruction in graph.instructions_mut(block) {
                let Operand::Value(argument) = instruction.operand else {
                    break;
                };
                match instruction.opcode {
                    DELETE_DEREF | DELETE_FAST | LOAD_FAST_AND_CLEAR => {
                        initialized.remove(&argument);
                    }
                    LOAD_FAST_CHECK => {
                        if initialized.contains(&argument) {
                            instruction.opcode = LOAD_FAST;
                        }
                        initialized.insert(argument);
                    }
                    LOAD_FAST | STORE_DEREF => {
                        initialized.insert(argument);
                    }
                    STORE_FAST => {
                        if instruction.local_policy.may_store_null() {
                            initialized.remove(&argument);
                        } else {
                            initialized.insert(argument);
                        }
                    }
                    LOAD_FAST_LOAD_FAST | STORE_FAST_LOAD_FAST | STORE_FAST_STORE_FAST => {
                        initialized.extend([argument >> 4, argument & 15]);
                    }
                    _ => {}
                }
                if ends_scope(instruction.opcode) {
                    break;
                }
            }
        }
    }

    /// Mirrors CPython's final `optimize_load_fast` CFG pass.
    ///
    /// An owned local load can become a borrowed load only when its value is
    /// consumed in the same basic block, is not stored into another local, and
    /// remains supported by the original local until it is consumed.
    pub(in crate::assembler) fn optimize_load_fast(&mut self) {
        #[derive(Clone, Copy)]
        struct Reference {
            producer: Option<usize>,
            local: Option<u32>,
        }

        fn pop_reference(stack: &mut Vec<Reference>) -> Reference {
            stack.pop().unwrap_or(Reference {
                producer: None,
                local: None,
            })
        }

        fn kill_local(unsafe_loads: &mut [bool], stack: &[Reference], local: u32) {
            for reference in stack {
                if reference.local == Some(local)
                    && let Some(producer) = reference.producer
                {
                    unsafe_loads[producer] = true;
                }
            }
        }

        fn store_local(
            unsafe_loads: &mut [bool],
            stack: &[Reference],
            local: u32,
            reference: Reference,
        ) {
            kill_local(unsafe_loads, stack, local);
            if let Some(producer) = reference.producer {
                unsafe_loads[producer] = true;
            }
        }

        let graph = &self.graph;
        if !graph.iter_items().any(|item| {
            matches!(
                item,
                Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_LOAD_FAST)
            )
        }) {
            return;
        }

        let mut block_labels = FxHashSet::default();
        for item in graph.iter_items() {
            if let Item::Instruction(Instruction {
                operand: Operand::Forward(label) | Operand::Backward(label),
                ..
            }) = item
            {
                block_labels.insert(*label);
            }
        }
        // CPython's borrow analysis operates on CFG blocks. Protected-range
        // boundaries do not create blocks, but exception handlers do.
        for region in &self.exception_regions {
            block_labels.insert(region.target);
        }
        block_labels.extend(self.borrow_unreachable_blocks.iter().copied());
        block_labels.extend(self.preserved_block_boundaries.iter().copied());

        let graph = &mut self.graph;
        graph.repartition(
            &block_labels,
            |_| false,
            |instruction| {
                !matches!(instruction.operand, Operand::Value(_))
                    || ends_scope(instruction.opcode)
                    || instruction.opcode == NOT_TAKEN
                    || (instruction.opcode == NOP && instruction.jump_policy.always_retains_nop())
                    || instruction.jump_policy.borrow_empty_block()
                    || instruction.borrow_fallthrough == BorrowFallthrough::Barrier
            },
        );
        let blocks = graph.order().to_vec();
        let block_slot_count = blocks
            .iter()
            .map(|block| block.index())
            .max()
            .map_or(0, |index| index + 1);
        let mut borrow_unreachable_blocks = vec![false; block_slot_count];
        for block in blocks.iter().copied() {
            if graph[block].items().iter().any(|item| {
                matches!(
                    item,
                    Item::Label(label) if self.borrow_unreachable_blocks.contains(label)
                )
            }) {
                borrow_unreachable_blocks[block.index()] = true;
            }
        }
        let mut block_positions = vec![usize::MAX; block_slot_count];
        for (position, block) in blocks.iter().copied().enumerate() {
            block_positions[block.index()] = position;
        }
        let mut reachable = vec![false; block_slot_count];
        let mut pending = blocks.first().copied().into_iter().collect::<Vec<_>>();
        while let Some(block) = pending.pop() {
            if borrow_unreachable_blocks[block.index()] {
                continue;
            }
            if std::mem::replace(&mut reachable[block.index()], true) {
                continue;
            }
            let Some(last) = graph[block].last_instruction() else {
                continue;
            };
            let borrow_traversal_stops = last.borrow_fallthrough == BorrowFallthrough::Barrier
                || last.jump_policy.borrow_empty_block();
            let folded_jump_has_no_fallthrough = last.opcode == NOP
                && last.jump_policy.always_retains_nop()
                && last.borrow_fallthrough != BorrowFallthrough::RetainedNopReachable;
            if !borrow_traversal_stops
                && !folded_jump_has_no_fallthrough
                && block_has_fallthrough(&[Item::Instruction(*last)])
                && let Some(next) = blocks.get(block_positions[block.index()] + 1)
            {
                pending.push(*next);
            }
            if !borrow_traversal_stops
                && let Operand::Forward(label) | Operand::Backward(label) = last.operand
                && let Some(target) = graph.label_block(label)
            {
                pending.push(target);
            }
        }

        for block in blocks {
            for instruction in graph.instructions_mut(block) {
                if instruction.local_policy.force_borrowed_load() && instruction.opcode == LOAD_FAST
                {
                    instruction.opcode = LOAD_FAST_BORROW;
                }
            }
            if !reachable[block.index()] {
                continue;
            }
            let mut cumulative_effect = 0_i64;
            let mut start_depth = None;
            for item in graph[block].items() {
                let Item::Instruction(instruction) = *item else {
                    continue;
                };
                let Operand::Value(argument) = instruction.operand else {
                    cumulative_effect += opcode_stack_effect(instruction.opcode, 0);
                    if let Some(depth_after) = instruction.depth_after {
                        start_depth = Some(i64::from(depth_after) - cumulative_effect);
                        break;
                    }
                    continue;
                };
                cumulative_effect += opcode_stack_effect(instruction.opcode, argument);
                if let Some(depth_after) = instruction.depth_after {
                    start_depth = Some(i64::from(depth_after) - cumulative_effect);
                    break;
                }
            }
            let mut stack = vec![
                Reference {
                    producer: None,
                    local: None,
                };
                usize::try_from(start_depth.unwrap_or(0).max(0)).unwrap()
            ];
            let mut unsafe_loads = vec![false; graph[block].items().len()];

            for (offset, item) in graph[block].items().iter().enumerate() {
                let Item::Instruction(instruction) = *item else {
                    continue;
                };
                let argument = match instruction.operand {
                    Operand::Value(argument) => argument,
                    Operand::Forward(_) | Operand::Backward(_) => 0,
                    Operand::Deferred(_) => {
                        unreachable!("operand patches are resolved before assembler passes")
                    }
                };
                match instruction.opcode {
                    DELETE_FAST => kill_local(&mut unsafe_loads, &stack, argument),
                    LOAD_FAST => stack.push(Reference {
                        producer: Some(offset),
                        local: Some(argument),
                    }),
                    LOAD_FAST_AND_CLEAR => {
                        kill_local(&mut unsafe_loads, &stack, argument);
                        stack.push(Reference {
                            producer: Some(offset),
                            local: Some(argument),
                        });
                    }
                    LOAD_FAST_LOAD_FAST => {
                        stack.push(Reference {
                            producer: Some(offset),
                            local: Some(argument >> 4),
                        });
                        stack.push(Reference {
                            producer: Some(offset),
                            local: Some(argument & 15),
                        });
                    }
                    STORE_FAST => {
                        let reference = pop_reference(&mut stack);
                        store_local(&mut unsafe_loads, &stack, argument, reference);
                    }
                    STORE_FAST_LOAD_FAST => {
                        let reference = pop_reference(&mut stack);
                        store_local(&mut unsafe_loads, &stack, argument >> 4, reference);
                        stack.push(Reference {
                            producer: Some(offset),
                            local: Some(argument & 15),
                        });
                    }
                    STORE_FAST_STORE_FAST => {
                        let reference = pop_reference(&mut stack);
                        store_local(&mut unsafe_loads, &stack, argument >> 4, reference);
                        let reference = pop_reference(&mut stack);
                        store_local(&mut unsafe_loads, &stack, argument & 15, reference);
                    }
                    COPY => {
                        let reference = stack
                            .len()
                            .checked_sub(usize::try_from(argument).unwrap())
                            .and_then(|index| stack.get(index))
                            .copied()
                            .unwrap_or(Reference {
                                producer: None,
                                local: None,
                            });
                        stack.push(reference);
                    }
                    SWAP => {
                        if let Some(index) =
                            stack.len().checked_sub(usize::try_from(argument).unwrap())
                        {
                            let top = stack.len().saturating_sub(1);
                            if index < stack.len() {
                                stack.swap(index, top);
                            }
                        }
                    }
                    // These instructions retain all their existing inputs.
                    FORMAT_SIMPLE | GET_ANEXT | GET_LEN | GET_YIELD_FROM_ITER | MATCH_KEYS
                    | MATCH_MAPPING | MATCH_SEQUENCE | WITH_EXCEPT_START | IMPORT_FROM => {
                        let popped = opcode_num_popped(instruction.opcode, argument);
                        let pushed = opcode_num_pushed(instruction.opcode, argument);
                        // CPython's inner loop shadows the instruction index,
                        // attributing these references to the first
                        // instructions in the block.
                        for (producer, _) in graph[block]
                            .items()
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| matches!(item, Item::Instruction(_)))
                            .take(pushed.saturating_sub(popped))
                        {
                            stack.push(Reference {
                                producer: Some(producer),
                                local: None,
                            });
                        }
                    }
                    // These consume only their top inputs and retain the
                    // container deeper on the stack.
                    DICT_MERGE | DICT_UPDATE | LIST_APPEND | LIST_EXTEND | MAP_ADD | RERAISE
                    | SET_ADD | SET_UPDATE => {
                        let popped = opcode_num_popped(instruction.opcode, argument);
                        let pushed = opcode_num_pushed(instruction.opcode, argument);
                        for _ in 0..popped.saturating_sub(pushed) {
                            pop_reference(&mut stack);
                        }
                    }
                    END_SEND | SET_FUNCTION_ATTRIBUTE => {
                        let top = pop_reference(&mut stack);
                        pop_reference(&mut stack);
                        stack.push(top);
                    }
                    CHECK_EXC_MATCH => {
                        pop_reference(&mut stack);
                        stack.push(Reference {
                            producer: None,
                            local: None,
                        });
                    }
                    FOR_ITER => stack.push(Reference {
                        producer: None,
                        local: None,
                    }),
                    LOAD_ATTR | LOAD_SUPER_ATTR => {
                        let receiver = pop_reference(&mut stack);
                        if instruction.opcode == LOAD_SUPER_ATTR {
                            pop_reference(&mut stack);
                            pop_reference(&mut stack);
                        }
                        stack.push(Reference {
                            producer: None,
                            local: None,
                        });
                        if argument & 1 != 0 {
                            stack.push(receiver);
                        }
                    }
                    PUSH_EXC_INFO | LOAD_SPECIAL => {
                        let top = pop_reference(&mut stack);
                        stack.push(Reference {
                            producer: None,
                            local: None,
                        });
                        stack.push(top);
                    }
                    SEND => {
                        pop_reference(&mut stack);
                        stack.push(Reference {
                            producer: None,
                            local: None,
                        });
                    }
                    _ => {
                        for _ in 0..opcode_num_popped(instruction.opcode, argument) {
                            pop_reference(&mut stack);
                        }
                        for _ in 0..opcode_num_pushed(instruction.opcode, argument) {
                            stack.push(Reference {
                                producer: None,
                                local: None,
                            });
                        }
                    }
                }
            }

            for reference in &stack {
                if let Some(producer) = reference.producer {
                    unsafe_loads[producer] = true;
                }
            }
            for (offset, unsafe_load) in unsafe_loads.iter().copied().enumerate() {
                if unsafe_load {
                    continue;
                }
                let Item::Instruction(instruction) = graph[block].items()[offset] else {
                    continue;
                };
                let local_policy = instruction.local_policy;
                if local_policy.strict_owned_load() {
                    continue;
                }
                if local_policy.force_owned_load() {
                    let mut following =
                        graph[block].items()[offset + 1..]
                            .iter()
                            .filter_map(|item| match *item {
                                Item::Instruction(instruction) if instruction.opcode != NOP => {
                                    Some(instruction.opcode)
                                }
                                _ => None,
                            });
                    let first = following.next();
                    let second = following.next();
                    let directly_consumed = matches!(
                        first,
                        Some(RETURN_VALUE | STORE_DEREF | STORE_GLOBAL)
                    ) || matches!((first, second), (Some(POP_TOP), Some(next)) if next != POP_EXCEPT)
                        || matches!(
                            (first, second),
                            (
                                Some(LOAD_COMMON_CONSTANT | LOAD_CONST | LOAD_SMALL_INT),
                                Some(COMPARE_OP)
                            )
                        );
                    if !directly_consumed {
                        continue;
                    }
                }
                let Some(instruction) = graph.instruction_at_mut(block, offset) else {
                    unreachable!();
                };
                match instruction.opcode {
                    LOAD_FAST => {
                        instruction.opcode = LOAD_FAST_BORROW;
                    }
                    LOAD_FAST_LOAD_FAST => {
                        instruction.opcode = LOAD_FAST_BORROW_LOAD_FAST_BORROW;
                    }
                    _ => {}
                }
            }
        }
    }

    pub(in crate::assembler) fn remove_borrow_empty_block_markers(&mut self) {
        let graph = &mut self.graph;
        let blocks = graph.order().to_vec();
        for block in blocks {
            graph.retain_items(block, |item| {
                !matches!(
                    item,
                    Item::Instruction(instruction)
                        if instruction.opcode == NOP
                            && instruction.jump_policy.borrow_empty_block()
                )
            });
            for instruction in graph.instructions_mut(block) {
                instruction.jump_policy.clear_borrow_empty_block();
            }
        }
    }

    pub(in crate::assembler) fn fuse_superinstructions(&mut self) {
        fn fuse_pair(first: Instruction, second: Instruction) -> Option<Instruction> {
            if first.fusion_policy.prevent_with_next
                || first.borrow_fallthrough == BorrowFallthrough::Barrier
                || second.fusion_policy.prevent_with_previous
            {
                return None;
            }
            let (Operand::Value(first_argument), Operand::Value(second_argument)) =
                (first.operand, second.operand)
            else {
                return None;
            };
            if first_argument >= 16
                || second_argument >= 16
                || first.location.line != second.location.line
            {
                return None;
            }
            let opcode = match (first.opcode, second.opcode) {
                (LOAD_FAST_BORROW, LOAD_FAST_BORROW) => LOAD_FAST_BORROW_LOAD_FAST_BORROW,
                (LOAD_FAST, LOAD_FAST) => LOAD_FAST_LOAD_FAST,
                (STORE_FAST, LOAD_FAST | LOAD_FAST_BORROW) => STORE_FAST_LOAD_FAST,
                (STORE_FAST, STORE_FAST) => STORE_FAST_STORE_FAST,
                _ => return None,
            };
            Some(Instruction::fused(
                opcode,
                Operand::Value((first_argument << 4) | second_argument),
                first,
                second,
            ))
        }

        let graph = &mut self.graph;
        let order = graph.order().to_vec();
        for block in order {
            graph.fuse_adjacent_instructions(block, fuse_pair);
        }
    }
}
