use super::{Assembler, Instruction, Item, Operand, SourceLocation, UnoptimizedCode};
use crate::target::opcodes::{CALL, CALL_KW, LOAD_SMALL_INT, POP_TOP, STORE_FAST};
use crate::{PythonVersion, target::Opcode};

impl Assembler {
    /// Checks the just-emitted call/store suffix in one authored original array.
    pub(crate) fn current_original_call_store_source(
        &self,
        call: (Opcode, u32, SourceLocation),
        value: (u8, SourceLocation),
        store: (u32, SourceLocation),
    ) -> bool {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return false;
        };
        if self.python_version != PythonVersion::Py314
            || !matches!(call.0, CALL | CALL_KW)
            || [call.2, value.1, store.1]
                .iter()
                .any(|location| location.line < 0)
            || !self.current_original_block_has_nonempty_borrow_source()
        {
            return false;
        }
        let expected = [
            Instruction::new(call.0, Operand::Value(call.1), call.2, Some(1)),
            Instruction::new(POP_TOP, Operand::Value(0), call.2, Some(0)),
            Instruction::new(
                LOAD_SMALL_INT,
                Operand::Value(u32::from(value.0)),
                value.1,
                Some(1),
            ),
            Instruction::new(STORE_FAST, Operand::Value(store.0), store.1, Some(0)),
        ]
        .map(Item::Instruction);
        let end = items
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .map_or(0, |index| index + 1);
        let Some(start) = end.checked_sub(expected.len()) else {
            return false;
        };
        if items[start..end] != expected
            || items
                .windows(expected.len())
                .filter(|window| *window == expected)
                .count()
                != 1
        {
            return false;
        }
        let provenance = &self.exception_owner_provenance;
        let Some(boundary) = items[..start].iter().rposition(|item| {
            matches!(item, Item::Label(label) if provenance.block_boundaries.contains(label))
        }) else {
            return false;
        };
        let head_start = items[..boundary]
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .map_or(0, |index| index + 1);
        let head_end = head_start
            + items[head_start..]
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
        let labels = |slice: &[Item]| {
            slice
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>()
        };
        let head = labels(&items[head_start..head_end]);
        let array_labels = labels(&items[head_start..]);
        let source_heads = head
            .iter()
            .filter(|label| provenance.source_boundaries.contains(label))
            .count();
        let original_heads = head
            .iter()
            .filter(|label| provenance.block_boundaries.contains(label))
            .count();
        if source_heads != 1
            || original_heads != 1
            || !head.iter().any(|label| {
                provenance.source_boundaries.contains(label)
                    && provenance.block_boundaries.contains(label)
            })
            || array_labels.iter().any(|label| {
                items
                    .iter()
                    .filter(|item| **item == Item::Label(*label))
                    .count()
                    != 1
                    || self.preserved_block_boundaries.contains(label)
                    || self.borrow_unreachable_blocks.contains(label)
                    || self.borrow_block_aliases.contains(label)
                    || self.borrow_jump_entry_labels.contains(label)
                    || self.borrow_finally_entries.contains_key(label)
                    || self.borrow_loop_exit_blocks.contains_key(label)
                    || self.conditional_threading_targets.contains_key(label)
                    || self.folded_away_condition_targets.contains(label)
                    || self.folded_away_loop_else_targets.contains(label)
                    || self.folded_condition_exit_targets.contains(label)
                    || self.original_nop_targets.contains(label)
                    || self.original_unlocated_cleanup_targets.contains(label)
                    || self.retained_pop_block_origins.contains_key(label)
                    || self.deferred_shared_jump_blocks.contains(label)
                    || self.shared_jump_copy_location_blocks.contains(label)
                    || provenance.omitted_nop_counts.contains_key(label)
                    || provenance.omitted_nop_locations.contains_key(label)
                    || provenance.borrow_only_block_boundaries.contains(label)
                    || provenance.ordinary_try_setups.contains_key(label)
                    || provenance.setups.iter().any(|(target, _)| target == label)
                    || self
                        .exception_regions
                        .iter()
                        .any(|region| region.target == *label)
                    || (!head.contains(label)
                        && (provenance.block_boundaries.contains(label)
                            || provenance.source_boundaries.contains(label)))
            })
        {
            return false;
        }
        // Interior exception aliases cannot introduce a second source entry.
        !items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                    if array_labels.contains(&target) && !head.contains(&target)))
        })
    }
}

#[cfg(test)]
mod tests;
