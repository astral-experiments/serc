use rustc_hash::FxHashSet;

use super::exception_owners::ExceptionOwnerProvenance;
use super::ir::EmissionStreamId;
use super::normal_finally_locations::unique_label_positions;
use super::{
    Assembler, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label, Operand,
    OptimizationPipeline, SourceLocation, UnoptimizedCode,
};
use crate::target::Opcode;
use crate::target::opcodes::{
    DELETE_ATTR, DELETE_FAST, LOAD_COMMON_CONSTANT, LOAD_CONST, NOP, POP_EXCEPT, POP_TOP,
    RETURN_VALUE, STORE_FAST, SWAP,
};

#[cfg(test)]
pub(super) mod tests;

/// The completed authored finalizer, before its named handler emits any cleanup.
pub(crate) struct NamedFinallyReturnPrefix {
    stream: EmissionStreamId,
    index: usize,
    donor: Instruction,
}

/// One unchanged native return array crossing only coverage labels.
#[derive(Clone, Debug)]
pub(super) struct NamedFinallyReturn {
    items: Vec<Item>,
    donor: SourceLocation,
}

/// One actual first-record location donation made by the exit-copy pass.
pub(super) struct NamedFinallyReturnDonation {
    pub(super) head: Vec<Label>,
    pub(super) before: Instruction,
    pub(super) after: Instruction,
}

fn forbidden(
    provenance: &ExceptionOwnerProvenance,
    regions: &[ExceptionRegion],
) -> FxHashSet<Label> {
    let mut labels = provenance.source_boundaries.clone();
    for set in [
        &provenance.block_boundaries,
        &provenance.borrow_only_block_boundaries,
        &provenance.continuation_boundaries,
        &provenance.value_jump_continuation_boundaries,
        &provenance.fusion_boundaries,
        &provenance.setup_nop_boundaries,
        &provenance.return_region_reentries,
        &provenance.loop_finally_region_boundaries,
        &provenance.normal_try_copy_inline_labels,
        &provenance.normal_try_copy_jump_nop_successors,
        &provenance.retired_normal_try_copy_labels,
        &provenance.normal_try_copy_blocks,
        &provenance.original_fallthrough_setups,
        &provenance.named_handler_cleanup_targets,
    ] {
        labels.extend(set);
    }
    labels.extend(&provenance.normal_try_copy_internal_labels);
    labels.extend(provenance.omitted_nop_counts.keys());
    labels.extend(provenance.omitted_nop_locations.keys());
    labels.extend(provenance.setups.iter().map(|(label, _)| label));
    labels.extend(provenance.ordinary_try_setups.keys());
    labels.extend(
        provenance
            .setup_nop_ranges
            .iter()
            .flat_map(|(start, end)| [start, end]),
    );
    labels.extend(regions.iter().map(|region| region.target));
    labels
}

impl NamedFinallyReturn {
    fn start(&self, items: &[Item], forbidden: &FxHashSet<Label>, located: bool) -> Option<usize> {
        let positions = unique_label_positions(items)?;
        let Item::Label(anchor) = self.items[1] else {
            return None;
        };
        let start = positions.get(&anchor)?.checked_sub(1)?;
        let current = items.get(start..start + self.items.len())?;
        if !matches!(
            items.get(start + self.items.len()),
            None | Some(Item::Label(_))
        ) {
            return None;
        }
        let labels = self
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashSet<_>>();
        if labels.iter().any(|label| forbidden.contains(label))
            || items.iter().any(|item| {
                matches!(item, Item::Instruction(record)
                if matches!(record.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if labels.contains(&label)))
            })
            || !current.iter().zip(&self.items).all(|(actual, expected)| {
                match (*actual, *expected) {
                    (Item::Label(actual), Item::Label(expected)) => actual == expected,
                    (Item::Instruction(mut actual), Item::Instruction(expected)) => {
                        if located
                            && expected.location == SourceLocation::NONE
                            && actual.location == self.donor
                        {
                            actual.location = SourceLocation::NONE;
                        }
                        actual == expected
                    }
                    _ => false,
                }
            })
        {
            return None;
        }
        Some(start)
    }

    pub(super) fn remap_constants(&mut self, map: &[Option<u32>]) -> bool {
        for item in &mut self.items {
            if let Item::Instruction(record) = item
                && record.opcode == LOAD_CONST
            {
                let Operand::Value(index) = record.operand() else {
                    return false;
                };
                let Some(Some(index)) = map.get(index as usize) else {
                    return false;
                };
                record.set_operand(Operand::Value(*index));
            }
        }
        true
    }

    pub(super) fn lower_constants(
        &mut self,
        count: usize,
        lower: &impl Fn(&mut Instruction) -> bool,
    ) -> bool {
        for item in &mut self.items {
            if let Item::Instruction(record) = item
                && record.opcode == LOAD_CONST
            {
                if !matches!(record.operand(), Operand::Value(index) if (index as usize) < count) {
                    return false;
                }
                lower(record);
                if record.opcode != LOAD_COMMON_CONSTANT
                    || record.operand()
                        != Operand::Value(
                            crate::target::operands315::CommonConstant::NONE.argument(),
                        )
                {
                    return false;
                }
            }
        }
        true
    }
}

impl Assembler {
    /// Captures only the actual last donor of the selected authored finalizer.
    pub(crate) fn named_finally_return_prefix(
        &self,
        opcode: Opcode,
    ) -> Option<NamedFinallyReturnPrefix> {
        if self.python_version < crate::PythonVersion::Py314 {
            return None;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        let Item::Instruction(donor) = *items.last()? else {
            return None;
        };
        if !matches!(opcode, NOP | POP_TOP | DELETE_ATTR)
            || donor.opcode != opcode
            || donor.location.line < 0
            || !matches!(donor.operand(), Operand::Value(_))
            || (opcode != DELETE_ATTR && donor.operand() != Operand::Value(0))
            || donor != Instruction::new(opcode, donor.operand(), donor.location, Some(2))
        {
            return None;
        }
        Some(NamedFinallyReturnPrefix {
            stream: self.emission_stream,
            index: items.len() - 1,
            donor,
        })
    }

    fn named_finally_return_forbidden(&self) -> FxHashSet<Label> {
        let mut labels = forbidden(&self.exception_owner_provenance, &self.exception_regions);
        for set in [
            &self.preserved_block_boundaries,
            &self.borrow_block_aliases,
            &self.borrow_unreachable_blocks,
            &self.original_nop_targets,
            &self.folded_away_condition_targets,
            &self.folded_away_loop_else_targets,
            &self.borrow_jump_entry_labels,
            &self.borrow_omitted_for_backedges,
        ] {
            labels.extend(set);
        }
        labels.extend(self.borrow_finally_entries.keys());
        labels
    }

    /// Binds the real name-clearing writer and settled ordinary/handler POP labels.
    pub(crate) fn record_named_finally_return(
        &mut self,
        prefix: &NamedFinallyReturnPrefix,
        outer_pop: Label,
        handler_pop: Label,
        handler: Label,
        local: u32,
        none: u32,
    ) {
        if self.python_version < crate::PythonVersion::Py314
            || prefix.stream != self.emission_stream
        {
            return;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return;
        };
        let Some(tail) = items.get(prefix.index..) else {
            return;
        };
        let [
            Item::Instruction(donor),
            Item::Label(swap_start),
            _,
            Item::Label(swap_end),
            ..,
        ] = tail
        else {
            return;
        };
        let record = |opcode, value, depth| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(value),
                SourceLocation::NONE,
                Some(depth),
            ))
        };
        let mut returned = Instruction::new(
            RETURN_VALUE,
            Operand::Value(0),
            SourceLocation::NONE,
            Some(0),
        );
        if prefix.donor.opcode == NOP {
            returned.no_location_policy.allow_block_inlining();
        }
        let expected = vec![
            Item::Instruction(prefix.donor),
            Item::Label(*swap_start),
            record(SWAP, 2, 2),
            Item::Label(*swap_end),
            Item::Label(handler_pop),
            record(POP_EXCEPT, 0, 1),
            record(LOAD_CONST, none, 2),
            record(STORE_FAST, local, 1),
            record(DELETE_FAST, local, 1),
            Item::Label(outer_pop),
            Item::Instruction(returned),
        ];
        if *donor != prefix.donor || tail != expected {
            return;
        }
        let labels = [*swap_start, *swap_end, handler_pop, outer_pop];
        if labels.into_iter().collect::<FxHashSet<_>>().len() != labels.len()
            || self.join_labels_have_authored_location(&labels)
        {
            return;
        }
        // This is capture-time proof of the real SWAP writer, not an owner replay.
        let regions = self
            .exception_regions
            .iter()
            .filter(|region| region.start == *swap_start || region.end == *swap_end)
            .collect::<Vec<_>>();
        if !matches!(regions.as_slice(), [region]
            if region.start == *swap_start && region.end == *swap_end
                && region.target == handler && region.depth == 1 && region.preserve_lasti
                && !region.is_scope_fallback
                && matches!(region.priority, ExceptionRegionPriority::OriginalRange(start, end)
                    if start == *swap_start && end == *swap_end))
        {
            return;
        }
        let origin = NamedFinallyReturn {
            items: expected,
            donor: donor.location,
        };
        if origin.start(items, &self.named_finally_return_forbidden(), false) == Some(prefix.index)
        {
            self.exception_owner_provenance
                .named_finally_returns
                .push(origin);
        }
    }

    /// Reject stale origins before the actual constant-index writer runs.
    pub(super) fn retain_named_finally_returns(&mut self) {
        if self
            .exception_owner_provenance
            .named_finally_returns
            .is_empty()
        {
            return;
        }
        let forbidden = self.named_finally_return_forbidden();
        let items = match &self.code {
            UnoptimizedCode::Linear(items) => items.clone(),
            UnoptimizedCode::Prepared { graph, .. } => graph.copy_items(),
        };
        self.exception_owner_provenance
            .named_finally_returns
            .retain(|origin| origin.start(&items, &forbidden, false).is_some());
    }
}

impl OptimizationPipeline {
    fn named_finally_return_forbidden(&self) -> FxHashSet<Label> {
        let mut labels = forbidden(&self.exception_owner_provenance, &self.exception_regions);
        for set in [
            &self.preserved_block_boundaries,
            &self.borrow_block_aliases,
            &self.borrow_unreachable_blocks,
            &self.original_nop_targets,
            &self.folded_condition_exit_targets,
            &self.folded_original_predecessor_targets,
            &self.borrow_jump_entry_labels,
            &self.borrow_omitted_for_backedges,
            &self.retired_exception_handler_targets,
        ] {
            labels.extend(set);
        }
        labels.extend(self.borrow_finally_entries.keys());
        labels.extend(self.borrow_finally_jump_sources.keys());
        labels.extend(self.borrow_pre_copy_jump_entries.keys());
        labels.extend(self.borrow_pre_copy_jump_entries.values());
        labels
    }

    pub(super) fn retain_named_finally_returns(&mut self) {
        if self
            .exception_owner_provenance
            .named_finally_returns
            .is_empty()
        {
            return;
        }
        let forbidden = self.named_finally_return_forbidden();
        let items = self.graph.copy_items();
        self.exception_owner_provenance
            .named_finally_returns
            .retain(|origin| origin.start(&items, &forbidden, false).is_some());
    }

    /// Carries only real exit-copy donations on the captured `SWAP` and `POP_EXCEPT` heads.
    pub(super) fn transport_named_finally_return_donations(
        &mut self,
        donations: &[NamedFinallyReturnDonation],
    ) {
        if self
            .exception_owner_provenance
            .named_finally_returns
            .is_empty()
        {
            return;
        }
        for origin in &mut self.exception_owner_provenance.named_finally_returns {
            for index in [2, 5] {
                let Item::Instruction(before) = origin.items[index] else {
                    continue;
                };
                let mut head = origin.items[..index]
                    .iter()
                    .rev()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .filter_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    })
                    .collect::<Vec<_>>();
                head.reverse();
                let mut after = before;
                after.location = origin.donor;
                let mut matching = donations.iter().filter(|donation| donation.head == head);
                if let Some(donation) = matching.next()
                    && matching.next().is_none()
                    && donation.before == before
                    && donation.after == after
                {
                    origin.items[index] = Item::Instruction(after);
                }
            }
        }
        self.retain_named_finally_returns();
    }

    /// Replay only the unchanged named cleanup's terminal return location.
    pub(super) fn propagate_named_finally_return_locations(&mut self) {
        let origins = std::mem::take(&mut self.exception_owner_provenance.named_finally_returns);
        if origins.is_empty() {
            return;
        }
        let forbidden = self.named_finally_return_forbidden();
        let items = self.graph.copy_items();
        let positions = self.graph.item_positions();
        for origin in origins {
            if let Some(start) = origin.start(&items, &forbidden, true) {
                self.graph
                    .instruction_mut(positions[start + origin.items.len() - 1])
                    .expect("authenticated original return")
                    .location = origin.donor;
            }
        }
    }
}
