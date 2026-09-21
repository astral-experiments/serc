#[cfg(any(test, debug_assertions))]
use super::super::AssemblerStage;
use super::super::{
    Assembler, BorrowFallthrough, COMPARE_OP, CONTAINS_OP, DELETE_FAST, IS_OP, Instruction, Item,
    LOAD_FAST, LOAD_FAST_AND_CLEAR, LOAD_FAST_BORROW, LOAD_FAST_BORROW_LOAD_FAST_BORROW,
    LOAD_FAST_CHECK, LOAD_FAST_LOAD_FAST, LayoutAssembler, NOP, Operand, POP_JUMP_IF_FALSE,
    POP_JUMP_IF_TRUE, POP_TOP, STORE_FAST, STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST, TO_BOOL,
    UNARY_NOT, UNPACK_SEQUENCE, UnoptimizedCode, ends_scope, is_unconditional_jump,
};
use crate::CompileError;
use crate::assembler::ConstantTable;
use crate::assembler::block_graph::ItemMut;
use crate::target::opcodes::{POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE};
use crate::target::operands::ComparisonOperation;

use super::control_flow::clear_redundant_boolean_nop_location;
use super::control_flow::redundant_copy_pop_indices;
use super::control_flow::remove_none_comparison_boolean_conversions;
use super::locals::optimize_linear_assertion_load_fast;

fn optimize_boolean_conversions_and_redundant_stores(
    items: &mut [Item],
    constants: Option<&dyn ConstantTable>,
) {
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
        if current.opcode == STORE_FAST
            && next.opcode == STORE_FAST
            // STORE_FAST_MAYBE_NULL remains a distinct pseudo-op through this pass.
            && !current.local_policy.may_store_null()
            && !next.local_policy.may_store_null()
            && matches!(
                (current.operand(), next.operand()),
                (Operand::Value(current), Operand::Value(next)) if current == next
            )
            && current.location.line == next.location.line
        {
            current.opcode = POP_TOP;
            current.set_operand(Operand::Value(0));
            continue;
        }
        match (current.opcode, next.opcode) {
            (TO_BOOL, TO_BOOL) => {
                current.opcode = NOP;
                current.set_operand(Operand::Value(0));
                continue;
            }
            (UNARY_NOT, TO_BOOL) => {
                current.opcode = NOP;
                current.set_operand(Operand::Value(0));
                next.opcode = UNARY_NOT;
                next.set_operand(Operand::Value(0));
                continue;
            }
            (UNARY_NOT, UNARY_NOT) => {
                current.opcode = NOP;
                current.set_operand(Operand::Value(0));
                next.opcode = NOP;
                next.set_operand(Operand::Value(0));
                continue;
            }
            (CONTAINS_OP | IS_OP, UNARY_NOT) => {
                let Operand::Value(argument) = current.operand() else {
                    continue;
                };
                let opcode = current.opcode;
                current.opcode = NOP;
                current.set_operand(Operand::Value(0));
                next.opcode = opcode;
                next.set_operand(Operand::Value(argument ^ 1));
                continue;
            }
            _ => {}
        }
        if next.opcode != TO_BOOL {
            continue;
        }
        let Operand::Value(argument) = current.operand() else {
            continue;
        };
        let replacement = match current.opcode {
            COMPARE_OP => Some((COMPARE_OP, ComparisonOperation::force_boolean(argument))),
            CONTAINS_OP => Some((CONTAINS_OP, argument)),
            IS_OP => Some((IS_OP, argument)),
            _ => None,
        };
        let Some((opcode, argument)) = replacement else {
            continue;
        };
        current.opcode = NOP;
        current.set_operand(Operand::Value(0));
        next.opcode = opcode;
        next.set_operand(Operand::Value(argument));
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

fn propagate_assertion_exit_locations(items: &mut [Item]) {
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
        let target = position + 4;
        debug_assert!(matches!(items.get(target), Some(Item::Label(_))));
        let mut end = target + 1;
        while end < items.len() {
            match items[end] {
                Item::Label(_) => break,
                Item::Instruction(instruction) => {
                    end += 1;
                    if !matches!(instruction.operand(), Operand::Value(_))
                        || ends_scope(instruction.opcode)
                    {
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
    for item in items {
        let instruction = match item {
            Item::Label(_) => {
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
            DELETE_FAST | LOAD_FAST_AND_CLEAR | STORE_FAST | LOAD_FAST_CHECK | LOAD_FAST
        ) && !instruction.local_policy.is_post_analysis_closure_load()
            && !instruction.local_policy.is_known_initialized_load()
            && let Operand::Value(argument) = instruction.operand()
            && let Ok(local) = usize::try_from(argument)
            && local < local_count
        {
            match instruction.opcode {
                DELETE_FAST | LOAD_FAST_AND_CLEAR => {
                    if local < 64 {
                        unsafe_locals |= 1 << local;
                    } else {
                        unsafe_high_locals[local - 64] = true;
                    }
                }
                STORE_FAST => {
                    let may_store_null = instruction.local_policy.may_store_null();
                    if local < 64 {
                        if may_store_null {
                            unsafe_locals |= 1 << local;
                        } else {
                            unsafe_locals &= !(1 << local);
                        }
                    } else {
                        unsafe_high_locals[local - 64] = may_store_null;
                    }
                }
                LOAD_FAST_CHECK | LOAD_FAST => {
                    let is_unsafe = if local < 64 {
                        let bit = 1 << local;
                        let is_unsafe = unsafe_locals & bit != 0;
                        unsafe_locals &= !bit;
                        is_unsafe
                    } else {
                        std::mem::replace(&mut unsafe_high_locals[local - 64], false)
                    };
                    instruction.opcode = if is_unsafe {
                        LOAD_FAST_CHECK
                    } else {
                        LOAD_FAST
                    };
                }
                _ => unreachable!(),
            }
        }
        if !matches!(instruction.operand(), Operand::Value(_)) || ends_scope(instruction.opcode) {
            unsafe_high_locals.fill(true);
            block_has_instruction = false;
        }
    }
}

fn fuse_superinstructions(items: &mut Vec<Item>) {
    fn fuse_pair(first: &Instruction, second: &Instruction) -> Option<Instruction> {
        let opcode = match (first.opcode, second.opcode) {
            (LOAD_FAST_BORROW, LOAD_FAST_BORROW) => LOAD_FAST_BORROW_LOAD_FAST_BORROW,
            (LOAD_FAST, LOAD_FAST) => LOAD_FAST_LOAD_FAST,
            (STORE_FAST, LOAD_FAST | LOAD_FAST_BORROW) => STORE_FAST_LOAD_FAST,
            (STORE_FAST, STORE_FAST) => STORE_FAST_STORE_FAST,
            _ => return None,
        };
        if first.fusion_policy.prevents_with_next()
            || first.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
            || second.fusion_policy.prevents_with_previous()
        {
            return None;
        }
        let (Operand::Value(first_argument), Operand::Value(second_argument)) =
            (first.operand(), second.operand())
        else {
            return None;
        };
        if first_argument >= 16
            || second_argument >= 16
            || first.location.line != second.location.line
        {
            return None;
        }
        Some(Instruction::fused(
            opcode,
            Operand::Value((first_argument << 4) | second_argument),
            *first,
            *second,
        ))
    }

    let original_len = items.len();
    let mut read = 0;
    let mut write = 0;
    while read < original_len {
        let fused = if read + 1 < original_len {
            match (&items[read], &items[read + 1]) {
                (Item::Instruction(first), Item::Instruction(second)) => fuse_pair(first, second),
                (Item::Label(_), _) | (_, Item::Label(_)) => None,
            }
        } else {
            None
        };
        if let Some(instruction) = fused {
            let Item::Instruction(second) = items[read + 1] else {
                unreachable!();
            };
            let second_location = second.location;
            if let Some(Item::Instruction(following)) = items.get_mut(read + 2)
                && following.location.line < 0
                && !following.no_location_policy.preserves_location()
                && second_location.line == instruction.location.line
                && second_location.column > instruction.location.column
            {
                following.location = second_location;
            }
            items[write] = Item::Instruction(instruction);
            read += 2;
        } else {
            if write != read {
                items[write] = items[read];
            }
            read += 1;
        }
        write += 1;
    }
    items.truncate(write);
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
                if !matches!(instruction.operand(), Operand::Value(_))
                    || ends_scope(instruction.opcode)
                {
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
        optimize_boolean_conversions_and_redundant_stores(&mut items, constants);
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::OptimizeBooleanConversions,
        )?;
        propagate_assertion_exit_locations(&mut items);
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::DuplicateExitBlocks,
        )?;
        let mut removed_max_depth = None;
        for index in redundant_copy_pop_indices(&items).into_iter().rev() {
            if let Item::Instruction(instruction) = items.remove(index)
                && let Some(depth) = instruction.depth_after()
            {
                removed_max_depth =
                    Some(removed_max_depth.map_or(depth, |current: u32| current.max(depth)));
            }
        }
        add_checks_for_uninitialized_loads(&mut items, local_count, parameter_count);
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::AddChecksForUninitializedLoads,
        )?;
        fuse_superinstructions(&mut items);
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
        optimize_linear_assertion_load_fast(self.python_version, &mut items);
        #[cfg(any(test, debug_assertions))]
        super::validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::OptimizeLoadFast,
        )?;
        Ok(LayoutAssembler {
            python_version: self.python_version,
            legacy_uniform_jump_seed: self.legacy_uniform_jump_seed,
            items,
            exception_regions: self.exception_regions,
            exception_owner_provenance: self.exception_owner_provenance,
            preserved_block_boundaries: self.preserved_block_boundaries,
            removed_max_depth,
            next_label: self.next_label,
            has_next_location: self.has_next_location,
        })
    }
}
