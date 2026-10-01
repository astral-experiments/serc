//! Original comparison-failure entries into a named handler's cleanup.

use super::normal_finally_locations::unique_label_positions;
use super::{
    Assembler, ExceptionPolicy, Instruction, Item, Operand, SourceLocation, UnoptimizedCode,
    ends_scope, is_unconditional_jump, trailing_labels,
};
use crate::target::opcodes::{JUMP_FORWARD, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_TOP};

#[cfg(test)]
mod tests;

impl Assembler {
    /// Authenticates an original comparison cleanup before emitting named recovery.
    pub(crate) fn at_original_named_chain_cleanup(
        &self,
        body_start: usize,
        comparison: SourceLocation,
        prefix: Option<(SourceLocation, ExceptionPolicy)>,
    ) -> bool {
        self.original_named_chain_cleanup(body_start, comparison, prefix)
            .is_some()
    }

    fn original_named_chain_cleanup(
        &self,
        body_start: usize,
        comparison: SourceLocation,
        prefix: Option<(SourceLocation, ExceptionPolicy)>,
    ) -> Option<()> {
        let provenance = &self.exception_owner_provenance;
        if self.python_version < crate::PythonVersion::Py314
            || provenance.original_comparison_fallthroughs.is_empty()
            || comparison.line < 0
        {
            return None;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        let joins = trailing_labels(items).collect::<Vec<_>>();
        let [join] = joins.as_slice() else {
            return None;
        };
        let mut origins = provenance
            .original_comparison_fallthroughs
            .iter()
            .filter(|(_, _, records)| records[4].location == comparison);
        let &(cleanup, success, records) = origins.next()?;
        if origins.next().is_some() || cleanup == success || [cleanup, success].contains(join) {
            return None;
        }
        let [failure, condition, marker, bridge, pop, jump] = records;
        let depth = pop.depth_after()?;
        let expected =
            |opcode, operand, depth| Instruction::new(opcode, operand, comparison, Some(depth));
        let mut expected_bridge = expected(JUMP_FORWARD, Operand::Forward(success), depth);
        expected_bridge.jump_policy.prevent_small_exit_inlining();
        if records
            != [
                expected(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(cleanup),
                    depth.checked_add(1)?,
                ),
                expected(POP_JUMP_IF_FALSE, Operand::Forward(*join), depth),
                expected(NOT_TAKEN, Operand::Value(0), depth),
                expected_bridge,
                expected(POP_TOP, Operand::Value(0), depth),
                expected(JUMP_FORWARD, Operand::Forward(*join), depth),
            ]
        {
            return None;
        }
        let positions = unique_label_positions(items)?;
        let cleanup_index = *positions.get(&cleanup)?;
        let success_index = *positions.get(&success)?;
        let join_index = *positions.get(join)?;
        let condition_index = cleanup_index.checked_sub(3)?;
        if condition_index < body_start
            || success_index != cleanup_index.checked_add(3)?
            || success_index.checked_add(1)? >= join_index
            || join_index != items.len().checked_sub(1)?
            || items.get(condition_index..cleanup_index)
                != Some(&[
                    Item::Instruction(condition),
                    Item::Instruction(marker),
                    Item::Instruction(bridge),
                ])
            || items.get(cleanup_index + 1..success_index)
                != Some(&[Item::Instruction(pop), Item::Instruction(jump)])
            || matches!(items.get(success_index + 1), Some(Item::Label(_)))
            || matches!(items.get(join_index.checked_sub(1)?), Some(Item::Label(_)))
            || items[success_index + 1..join_index].iter().any(|item| {
                !matches!(item, Item::Instruction(record)
                    if matches!(record.operand(), Operand::Value(_) | Operand::Deferred(_))
                        && !ends_scope(record.opcode))
            })
        {
            return None;
        }
        let unique_record = |record| {
            let mut found = items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| (*item == Item::Instruction(record)).then_some(index));
            let first = found.next()?;
            found.next().is_none().then_some(first)
        };
        for record in records {
            unique_record(record)?;
        }
        let failure_index = unique_record(failure)?;
        if failure_index <= body_start
            || failure_index >= condition_index
            || items[body_start..failure_index].iter().any(|item| {
                matches!(item, Item::Instruction(record)
                    if ends_scope(record.opcode) || is_unconditional_jump(record.opcode))
            })
        {
            return None;
        }
        let mut labels = vec![cleanup, success, *join];
        let mut edges = vec![failure, condition, bridge, jump];
        if let Some((location, owner)) = prefix {
            if location.line < 0 || location == comparison {
                return None;
            }
            let prefix = Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(*join),
                location,
                Some(depth),
            );
            if provenance
                .original_primitive_condition_boundaries
                .iter()
                .filter(|record| **record == prefix)
                .count()
                != 1
            {
                return None;
            }
            let prefix_index = unique_record(prefix)?;
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, Some(depth));
            marker.set_exception_policy(owner);
            let Item::Label(continuation) = *items.get(prefix_index.checked_add(2)?)? else {
                return None;
            };
            if prefix_index < body_start
                || prefix_index.checked_add(3)? >= failure_index
                || items.get(prefix_index + 1) != Some(&Item::Instruction(marker))
                || labels.contains(&continuation)
                || !provenance.source_boundaries.contains(&continuation)
                || !provenance.block_boundaries.contains(&continuation)
                || items[prefix_index + 3..=failure_index]
                    .iter()
                    .any(|item| matches!(item, Item::Label(_)))
            {
                return None;
            }
            unique_record(marker)?;
            labels.push(continuation);
            edges.push(prefix);
        }
        if self.join_labels_have_authored_location(&labels)
            || provenance.source_boundaries.contains(&cleanup)
            || provenance.block_boundaries.contains(&cleanup)
            || [success, *join].iter().any(|label| {
                !provenance.source_boundaries.contains(label)
                    || !provenance.block_boundaries.contains(label)
            })
            || labels.iter().any(|label| {
                self.folded_away_condition_targets.contains(label)
                    || self.folded_away_loop_else_targets.contains(label)
                    || provenance.omitted_nop_counts.contains_key(label)
            })
        {
            return None;
        }
        // Every entry belongs to this original chain or its authenticated prefix.
        let mut incoming = 0;
        for (index, item) in items.iter().enumerate() {
            let Item::Instruction(record) = item else {
                continue;
            };
            let (Operand::Forward(target) | Operand::Backward(target)) = record.operand() else {
                continue;
            };
            if labels.contains(&target) {
                if index < body_start || index >= join_index || !edges.contains(record) {
                    return None;
                }
                incoming += 1;
            }
        }
        (incoming == edges.len()).then_some(())
    }
}
