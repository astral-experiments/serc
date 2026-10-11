#[cfg(any(test, debug_assertions))]
use super::super::AssemblerStage;
use super::super::{
    Assembler, COMPARE_OP, Instruction, Item, LayoutAssembler, NOP, POP_JUMP_IF_FALSE,
    POP_JUMP_IF_TRUE, STORE_FAST, STORE_FAST_STORE_FAST, TO_BOOL, UNARY_NOT, UNPACK_SEQUENCE,
    UnoptimizedCode, ends_scope, is_unconditional_jump,
};
use crate::CompileError;
use crate::assembler::ConstantTable;
use crate::assembler::block_graph::{ItemMut, fuse_adjacent_instructions};
use crate::target::opcodes::{
    COPY, LOAD_FAST, LOAD_FAST_BORROW, LOAD_FAST_CHECK, LOAD_FAST_LOAD_FAST, NOT_TAKEN,
    POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE,
};

use super::control_flow::{
    clear_redundant_boolean_nop_location, optimize_boolean_pair, optimize_redundant_store_pair,
    redundant_copy_pop_indices, remove_none_comparison_boolean_conversions,
};
use super::locals::{
    fuse_pair, optimize_linear_assertion_load_fast, update_assertion_local_checks,
};

#[derive(Default)]
struct LinearCandidates {
    boolean_conversion: bool,
    adjacent_stores: bool,
    copy_top: bool,
    individual_load: bool,
    combined_load: bool,
    fusable_instructions: usize,
    conditional_targets: u8,
}

impl LinearCandidates {
    fn from_items(items: &[Item]) -> Self {
        Self::collect::<false>(items)
    }

    fn with_conditional_targets(items: &[Item]) -> Self {
        Self::collect::<true>(items)
    }

    fn collect<const TRACK_TARGETS: bool>(items: &[Item]) -> Self {
        let mut candidates = Self::default();
        let mut previous_was_store = false;
        for item in items {
            let Item::Instruction(instruction) = item else {
                previous_was_store = false;
                continue;
            };
            let opcode = instruction.opcode;
            if TRACK_TARGETS
                && let Some(label) = instruction.forward_target()
                && let Some(bit) = 1_u8.checked_shl(label.as_u32())
            {
                candidates.conditional_targets |= bit;
            }
            let is_store = opcode == STORE_FAST;
            candidates.adjacent_stores |= previous_was_store && is_store;
            previous_was_store = is_store;
            candidates.boolean_conversion |= matches!(opcode, TO_BOOL | UNARY_NOT);
            candidates.copy_top |= opcode == COPY && instruction.value_argument() == Some(1);
            candidates.individual_load |= matches!(opcode, LOAD_FAST | LOAD_FAST_CHECK);
            candidates.combined_load |= opcode == LOAD_FAST_LOAD_FAST;
            candidates.fusable_instructions += usize::from(matches!(
                opcode,
                LOAD_FAST | LOAD_FAST_BORROW | LOAD_FAST_CHECK | STORE_FAST
            ));
        }
        candidates
    }
}

fn optimize_boolean_conversions_and_redundant_stores(
    items: &mut [Item],
    constants: Option<&dyn ConstantTable>,
    candidates: &LinearCandidates,
) {
    if !candidates.boolean_conversion {
        if candidates.adjacent_stores {
            for index in 0..items.len().saturating_sub(1) {
                let (before, after) = items.split_at_mut(index + 1);
                if let (Item::Instruction(current), Item::Instruction(next)) =
                    (&mut before[index], &after[0])
                {
                    optimize_redundant_store_pair(current, next);
                }
            }
        }
        return;
    }

    let was_boolean_conversion = items
        .iter()
        // Match the graph pass: a comparison moved into TO_BOOL leaves a folded slot.
        .map(|item| matches!(item, Item::Instruction(instruction) if matches!(instruction.opcode, TO_BOOL | UNARY_NOT | COMPARE_OP)))
        .collect::<Vec<_>>();
    if let Some(constants) = constants {
        remove_none_comparison_boolean_conversions(
            items.iter_mut().map(|item| match item {
                Item::Instruction(instruction) => ItemMut::Instruction(instruction),
                Item::Label(_) => ItemMut::Label,
            }),
            constants,
        );
    }
    for index in 0..items.len().saturating_sub(1) {
        let (before, after) = items.split_at_mut(index + 1);
        let (Item::Instruction(current), Item::Instruction(next)) =
            (&mut before[index], &mut after[0])
        else {
            continue;
        };
        if !optimize_redundant_store_pair(current, next) {
            optimize_boolean_pair(current, next, false);
        }
    }
    let mut previous_line = None;
    for (item, was_boolean_conversion) in items.iter_mut().zip(was_boolean_conversion) {
        match item {
            Item::Instruction(instruction) => clear_redundant_boolean_nop_location(
                instruction,
                was_boolean_conversion,
                &mut previous_line,
            ),
            Item::Label(_) => previous_line = None,
        }
    }
}

fn propagate_assertion_exit_locations(items: &mut [Item], conditional_targets: Option<u8>) {
    let mut position = 0;
    while position < items.len() {
        let Item::Instruction(jump) = items[position] else {
            position += 1;
            continue;
        };
        if !matches!(
            jump.opcode,
            POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
        ) {
            position += 1;
            continue;
        }
        let Some(label) = jump.forward_target() else {
            unreachable!("a terminal guard must jump forward");
        };
        // A physical alias immediately after NOT_TAKEN is still reached only by
        // fallthrough when no jump targets any of its labels. The graph path
        // propagates that predecessor's location into its first instruction.
        if let (Some(targets), Some(Item::Instruction(marker))) =
            (conditional_targets, items.get(position + 1).copied())
            && marker.opcode == NOT_TAKEN
            && marker.location.line >= 0
        {
            let mut next = position + 2;
            let mut has_alias = false;
            let mut unique = true;
            while let Some(Item::Label(alias)) = items.get(next) {
                has_alias = true;
                unique &= 1_u8
                    .checked_shl(alias.as_u32())
                    .is_some_and(|bit| targets & bit == 0);
                next += 1;
            }
            if has_alias
                && unique
                && items[next..]
                    .iter()
                    .take_while(|item| !matches!(item, Item::Label(_)))
                    .all(|item| matches!(item, Item::Instruction(instruction) if instruction.location.line < 0))
                && let Some(Item::Instruction(first)) = items.get_mut(next)
                && first.location.line < 0
                && !first.no_location_policy.preserves_location()
            {
                first.location = marker.location;
            }
        }
        let target = position
            + 1
            + items[position + 1..]
                .iter()
                .position(|item| matches!(item, Item::Label(candidate) if *candidate == label))
                .expect("a terminal guard has a bound continuation");
        let mut end = target + 1;
        while end < items.len() {
            match items[end] {
                Item::Label(_) => break,
                Item::Instruction(instruction) => {
                    end += 1;
                    if instruction.value_argument().is_none() || ends_scope(instruction.opcode) {
                        break;
                    }
                }
            }
        }
        if items[target + 1..end].iter().any(
            |item| matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0),
        ) {
            position = target + 1;
            continue;
        }
        if let Some(Item::Instruction(first)) = items.get_mut(target + 1)
            && !first.no_location_policy.preserves_location()
        {
            first.location = jump.location;
        }
        position = target + 1;
    }
}

fn add_checks_for_uninitialized_loads(
    items: &mut [Item],
    local_count: usize,
    parameter_count: usize,
) {
    if local_count == 0 {
        return;
    }
    let initialized = parameter_count.min(local_count).min(64);
    let mut unsafe_locals = u64::MAX
        .checked_shl(u32::try_from(initialized).unwrap())
        .unwrap_or(0);
    let mut unsafe_high_locals = vec![true; local_count.saturating_sub(64)];
    let mut block_has_instruction = false;
    let mut continuing_state = None;
    for item in items {
        let instruction = match item {
            Item::Label(label) => {
                if continuing_state
                    .as_ref()
                    .is_some_and(|(target, _)| *target == *label)
                    && let Some((_, initialized)) = continuing_state.take()
                {
                    if block_has_instruction {
                        // The last side arm can fall through to the same continuing target.
                        unsafe_locals |= initialized;
                    } else {
                        unsafe_locals = initialized;
                    }
                }
                if block_has_instruction {
                    unsafe_high_locals.fill(true);
                    block_has_instruction = false;
                }
                continue;
            }
            Item::Instruction(instruction) => instruction,
        };
        block_has_instruction = true;
        if matches!(
            instruction.opcode,
            POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
        ) && let Some(target) = instruction.forward_target()
        {
            if let Some((pending_target, pending_unsafe)) = &mut continuing_state {
                debug_assert_eq!(*pending_target, target);
                // A short-circuit continuation can be reached before later operands set or
                // delete locals; a load is safe only if every predecessor initialized it.
                *pending_unsafe |= unsafe_locals;
            } else {
                continuing_state = Some((target, unsafe_locals));
            }
        }
        update_assertion_local_checks(
            instruction,
            local_count,
            &mut unsafe_locals,
            &mut unsafe_high_locals,
        );
        if instruction.value_argument().is_none() || ends_scope(instruction.opcode) {
            unsafe_high_locals.fill(true);
            block_has_instruction = false;
        }
    }
}

fn optimize_guard_load_fast(
    python_version: crate::PythonVersion,
    mut items: &mut [Item],
    conditional_targets: Option<u8>,
) {
    loop {
        let mut block_has_instruction = false;
        let boundary = items.iter().position(|item| match item {
            Item::Label(label) => {
                block_has_instruction
                    && conditional_targets.is_none_or(|targets| {
                        1_u8.checked_shl(label.as_u32())
                            .is_none_or(|bit| targets & bit != 0)
                    })
            }
            Item::Instruction(instruction) => {
                block_has_instruction = instruction.value_argument().is_some()
                    && !ends_scope(instruction.opcode)
                    && instruction.opcode != crate::target::opcodes::NOT_TAKEN
                    && !(instruction.opcode == NOP && instruction.jump_policy.always_retains_nop())
                    && !instruction.jump_policy.borrow_empty_block()
                    && instruction.no_location_policy.borrow_fallthrough()
                        != crate::assembler::ir::BorrowFallthrough::Barrier;
                false
            }
        });
        let Some(boundary) = boundary else {
            optimize_linear_assertion_load_fast(python_version, items);
            return;
        };
        let (before, after) = items.split_at_mut(boundary);
        // The graph resets its reference stack before a targeted label reached both from a
        // conditional and its fallthrough arm. Terminal arms already end the borrow range.
        optimize_linear_assertion_load_fast(python_version, before);
        items = &mut after[1..];
    }
}

fn propagate_locations_and_remove_redundant_nops(items: &mut Vec<Item>) {
    let mut previous_location = None;
    let mut previous_retained = None;
    let mut block_has_instruction = false;
    let original_len = items.len();
    let mut write = 0;
    for read in 0..original_len {
        let remove = match items[read] {
            Item::Label(_) if block_has_instruction => {
                previous_location = None;
                block_has_instruction = false;
                false
            }
            Item::Label(_) => false,
            Item::Instruction(mut instruction) => {
                block_has_instruction = true;
                if instruction.location.line < 0
                    && !instruction.no_location_policy.preserves_location()
                {
                    if let Some(location) = previous_location {
                        instruction.location = location;
                    }
                } else {
                    previous_location = Some(instruction.location);
                }
                items[read] = Item::Instruction(instruction);
                if instruction.value_argument().is_none() || ends_scope(instruction.opcode) {
                    previous_location = None;
                    block_has_instruction = false;
                }

                let mut next_location = previous_location;
                let mut next_block_has_instruction = block_has_instruction;
                let next = (instruction.opcode == NOP)
                    .then(|| {
                        items[read + 1..].iter().find_map(|item| match *item {
                            Item::Label(_) if next_block_has_instruction => {
                                next_location = None;
                                next_block_has_instruction = false;
                                None
                            }
                            Item::Label(_) => None,
                            Item::Instruction(mut next) => {
                                if next.location.line < 0
                                    && !next.no_location_policy.preserves_location()
                                    && let Some(location) = next_location
                                {
                                    next.location = location;
                                }
                                Some(next)
                            }
                        })
                    })
                    .flatten();
                let remove = instruction.opcode == NOP
                    && !instruction.jump_policy.preserves_explicit_nop()
                    && (instruction.location.line < 0
                        || (instruction.fusion_policy.is_converted_pop_block()
                            && previous_retained.is_some_and(|previous: Instruction| {
                                previous.location.line == instruction.location.line
                                    && !(ends_scope(previous.opcode)
                                        || is_unconditional_jump(previous.opcode))
                            }))
                        || next
                            .is_some_and(|next| next.location.line == instruction.location.line)
                        || (previous_retained.is_some_and(|previous: Instruction| {
                            previous.opcode == UNPACK_SEQUENCE
                        }) && next.is_some_and(|next| {
                            matches!(next.opcode, STORE_FAST | STORE_FAST_STORE_FAST)
                        })));
                if !remove {
                    previous_retained = Some(instruction);
                }
                remove
            }
        };
        if !remove {
            if write != read {
                items[write] = items[read];
            }
            write += 1;
        }
    }
    items.truncate(write);
}

impl Assembler {
    pub(in crate::assembler) fn optimize_linear_assertion_side_exits(
        self,
        local_count: usize,
        parameter_count: usize,
        constants: Option<&dyn ConstantTable>,
    ) -> Result<LayoutAssembler, CompileError> {
        let UnoptimizedCode::Linear(mut items) = self.code else {
            unreachable!("linear assertion optimizer requires linear instructions");
        };
        // A small certified guard can contain unreferenced return aliases. The generic
        // borrowing traversal ignores them; all physical labels remain for the other passes.
        let track_conditional_targets = (3..=4).contains(&self.next_label);
        let track_fallthrough_targets = (2..=4).contains(&self.next_label);
        let candidates = if track_fallthrough_targets {
            LinearCandidates::with_conditional_targets(&items)
        } else {
            LinearCandidates::from_items(&items)
        };
        optimize_boolean_conversions_and_redundant_stores(&mut items, constants, &candidates);
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::OptimizeBooleanConversions,
        )?;
        if self.next_label != 0 {
            propagate_assertion_exit_locations(
                &mut items,
                track_fallthrough_targets.then_some(candidates.conditional_targets),
            );
        }
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::DuplicateExitBlocks,
        )?;
        let mut removed_max_depth = None;
        if candidates.copy_top {
            for index in redundant_copy_pop_indices(&items).into_iter().rev() {
                if let Item::Instruction(instruction) = items.remove(index)
                    && let Some(depth) = instruction.depth_after()
                {
                    removed_max_depth =
                        Some(removed_max_depth.map_or(depth, |current: u32| current.max(depth)));
                }
            }
        }
        if candidates.individual_load {
            add_checks_for_uninitialized_loads(&mut items, local_count, parameter_count);
        }
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::AddChecksForUninitializedLoads,
        )?;
        if candidates.fusable_instructions > 1 {
            fuse_adjacent_instructions(&mut items, fuse_pair);
        }
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::FuseSuperinstructions,
        )?;
        propagate_locations_and_remove_redundant_nops(&mut items);
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::PropagateLocationsWithinBlocks,
        )?;
        if candidates.individual_load || candidates.combined_load {
            if self.next_label == 0 {
                optimize_linear_assertion_load_fast(self.python_version, &mut items);
            } else {
                optimize_guard_load_fast(
                    self.python_version,
                    &mut items,
                    track_conditional_targets.then_some(candidates.conditional_targets),
                );
            }
        }
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::OptimizeLoadFast,
        )?;
        let mut exception_owner_provenance = self.exception_owner_provenance;
        // The certified linear path has no exception table to replay these records.
        if !self.exception_regions.is_empty() && !self.original_pass_nops.is_empty() {
            exception_owner_provenance
                .original_pass_nops
                .extend(self.original_pass_nops);
        }
        Ok(LayoutAssembler {
            python_version: self.python_version,
            legacy_uniform_jump_seed: self.legacy_uniform_jump_seed,
            items,
            exception_regions: self.exception_regions,
            exception_owner_provenance,
            preserved_block_boundaries: self.preserved_block_boundaries,
            removed_max_depth,
            next_label: self.next_label,
            has_next_location: self.has_next_location,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{LinearCandidates, optimize_boolean_conversions_and_redundant_stores};
    use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
    use crate::target::opcodes::{
        COPY, LOAD_CONST, LOAD_FAST_CHECK, LOAD_FAST_LOAD_FAST, NOP, POP_TOP, STORE_FAST, TO_BOOL,
    };

    #[test]
    fn linear_candidates_preserve_store_and_boolean_rewrites() {
        let location = SourceLocation::new(1, 1, 0, 1);
        let instruction = |opcode, argument| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                location,
                None,
            ))
        };
        let label = Item::Label(Label::from_u32(0));
        let mut stores = [
            instruction(STORE_FAST, 0),
            instruction(STORE_FAST, 0),
            label,
            instruction(STORE_FAST, 0),
        ];
        let candidates = LinearCandidates::from_items(&stores);
        assert!(candidates.adjacent_stores);
        assert!(!candidates.boolean_conversion);
        optimize_boolean_conversions_and_redundant_stores(&mut stores, None, &candidates);
        assert!(matches!(stores[0], Item::Instruction(current) if current.opcode == POP_TOP));
        assert!(matches!(stores[1], Item::Instruction(current) if current.opcode == STORE_FAST));
        assert!(!LinearCandidates::from_items(&stores).adjacent_stores);

        let mut conversions = [
            instruction(LOAD_CONST, 0),
            instruction(TO_BOOL, 0),
            instruction(TO_BOOL, 0),
            instruction(COPY, 2),
            instruction(LOAD_FAST_CHECK, 0),
            instruction(LOAD_FAST_LOAD_FAST, 1),
        ];
        let candidates = LinearCandidates::from_items(&conversions);
        assert!(candidates.boolean_conversion);
        assert!(candidates.individual_load);
        assert!(candidates.combined_load);
        assert_eq!(candidates.fusable_instructions, 1);
        assert!(!candidates.copy_top);
        assert!(LinearCandidates::from_items(&[instruction(COPY, 1)]).copy_top);
        optimize_boolean_conversions_and_redundant_stores(&mut conversions, None, &candidates);
        assert!(matches!(conversions[1], Item::Instruction(current) if current.opcode == NOP));
        assert!(matches!(conversions[2], Item::Instruction(current) if current.opcode == TO_BOOL));
    }
}
