//! Original named-handler cleanup predecessors after a specialized None loop.

use super::normal_finally_locations::unique_label_positions;
use super::{Assembler, Instruction, Item, Operand, SourceLocation, trailing_labels};
use crate::target::opcodes::{JUMP_BACKWARD, JUMP_FORWARD, LOAD_FAST, POP_JUMP_IF_NONE};

#[cfg(test)]
mod tests;

impl Assembler {
    /// Keeps one authored break until its original named cleanup has been copied.
    ///
    /// Unlike the ordinary while join, this source has three removed None-test
    /// slots. Its existing source receipt must still occupy the original entry.
    pub(crate) fn defer_original_named_while_none_cleanup(
        &mut self,
        body_start: usize,
        local: u32,
        name: SourceLocation,
        condition: SourceLocation,
        authored_break: SourceLocation,
    ) -> bool {
        if self.python_version < crate::PythonVersion::Py314
            || self
                .exception_owner_provenance
                .source_none_conditions
                .is_empty()
        {
            return false;
        }
        let Some(index) = self.original_named_while_none_break(
            body_start,
            local,
            name,
            condition,
            authored_break,
        ) else {
            return false;
        };
        let Item::Instruction(instruction) = &mut self.linear_items_mut()[index] else {
            unreachable!("the complete authored break was checked above");
        };
        instruction.jump_policy.defer_removal();
        true
    }

    fn original_named_while_none_break(
        &self,
        body_start: usize,
        local: u32,
        name: SourceLocation,
        condition: SourceLocation,
        authored_break: SourceLocation,
    ) -> Option<usize> {
        let items = self.linear_items();
        let labels = trailing_labels(items).collect::<Vec<_>>();
        let [end, false_target] = labels.as_slice() else {
            return None;
        };
        let provenance = &self.exception_owner_provenance;
        if name.line < 0
            || condition.line < 0
            || authored_break.line < 0
            || end == false_target
            || self.join_labels_have_authored_location(&labels)
            || !provenance.source_boundaries.contains(false_target)
            || !provenance.block_boundaries.contains(false_target)
            || provenance.source_boundaries.contains(end)
            || provenance.block_boundaries.contains(end)
            || labels.iter().any(|label| {
                self.folded_away_condition_targets.contains(label)
                    || self.folded_away_loop_else_targets.contains(label)
                    || provenance.omitted_nop_counts.contains_key(label)
                    || provenance.setups.iter().any(|(setup, _)| setup == label)
                    || self
                        .exception_regions
                        .iter()
                        .any(|region| region.target == *label)
            })
        {
            return None;
        }
        let positions = unique_label_positions(items)?;
        let join_start = items.len().checked_sub(2)?;
        let Item::Instruction(backedge) = *items.get(join_start.checked_sub(1)?)? else {
            return None;
        };
        let Operand::Backward(entry) = backedge.operand() else {
            return None;
        };
        let depth = backedge.depth_after()?;
        if backedge.location.line < 0
            || backedge
                != Instruction::new(
                    JUMP_BACKWARD,
                    Operand::Backward(entry),
                    backedge.location,
                    Some(depth),
                )
        {
            return None;
        }
        let mut expected_condition = Instruction::new(
            POP_JUMP_IF_NONE,
            Operand::Forward(*false_target),
            condition,
            Some(depth),
        );
        expected_condition
            .local_policy
            .add_hidden_exception_owner_slots(3);
        let mut expected_break = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(*end),
            authored_break,
            Some(depth),
        );
        expected_break.fusion_policy.mark_authored_break();
        expected_break.jump_policy.prevent_threading_target();
        expected_break.jump_policy.retain_nop_for_direct_target();
        expected_break.add_pre_fold_extra_instructions(1);

        let mut origins = provenance.source_none_conditions.iter().filter(|origin| {
            origin.entry == entry
                || origin.labels.contains(&entry)
                || origin.instruction == expected_condition
        });
        let origin = origins.next()?;
        if origins.next().is_some()
            || origin.entry != entry
            || origin.labels != [entry]
            || origin.entry_prefix.as_deref() != Some([Item::Label(entry)].as_slice())
            || origin.instruction != expected_condition
            || !provenance.source_boundaries.contains(&entry)
            || !provenance.block_boundaries.contains(&entry)
        {
            return None;
        }
        let source_start = *positions.get(&entry)?;
        if source_start < body_start
            || matches!(
                items.get(source_start.wrapping_sub(1)),
                Some(Item::Label(_))
            )
            || items.get(source_start + 1..source_start + 3)
                != Some(
                    [
                        Item::Instruction(Instruction::new(
                            LOAD_FAST,
                            Operand::Value(local),
                            name,
                            Some(depth.checked_add(1)?),
                        )),
                        Item::Instruction(expected_condition),
                    ]
                    .as_slice(),
                )
        {
            return None;
        }
        // Count every incoming record, including backward or duplicate edges.
        // An equal replacement elsewhere cannot stand in for the captured source.
        let mut condition_index = None;
        let mut break_index = None;
        for (index, item) in items.iter().enumerate() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            let target = match instruction.operand() {
                Operand::Forward(target) | Operand::Backward(target)
                    if labels.contains(&target) =>
                {
                    target
                }
                _ => continue,
            };
            if index < body_start || index >= join_start {
                return None;
            }
            if target == *false_target && *instruction == expected_condition {
                if condition_index.replace(index).is_some() {
                    return None;
                }
            } else if target == *end && *instruction == expected_break {
                if break_index.replace(index).is_some() {
                    return None;
                }
            } else {
                return None;
            }
        }
        let break_index = break_index?;
        (condition_index == Some(source_start + 2) && source_start + 2 < break_index)
            .then_some(break_index)
    }
}
