use super::fusion_owner_slots::same_manager_region;
use super::normal_finally_locations::{array_start, same_priority, unique_label_positions};
use super::{
    Assembler, BorrowFallthrough, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority,
    Instruction, Item, Label, Operand, OptimizationPipeline, SourceLocation,
};
use crate::target::opcodes::{
    CALL, CALL_KW, END_SEND, JUMP_FORWARD, NOP, POP_EXCEPT, POP_JUMP_IF_FALSE, POP_TOP,
    RAISE_VARARGS, SEND,
};

#[cfg(test)]
mod tests;

/// An intact original entry whose independent source must still be normally visited.
#[derive(Debug)]
pub(super) struct IndependentFinallyEntry {
    entry: Label,
    donor: Label,
    labels: [Label; 6],
    prefix: super::normal_finally_locations::AwaitedNormalFinallyPrefix,
    arrays: Vec<(Label, usize, Vec<Item>)>,
    incoming: Vec<(Vec<Label>, Vec<Instruction>)>,
    regions: Vec<ExceptionRegion>,
}

impl IndependentFinallyEntry {
    pub(super) fn remap_constants(
        &mut self,
        map: &[Option<u32>],
        deferred_none: Option<u32>,
    ) -> bool {
        // All additional records are control/await/CALL records without constants.
        self.prefix.remap_constants(map, deferred_none)
    }

    pub(super) fn resolve_priorities(
        &mut self,
        positions: &rustc_hash::FxHashMap<Label, usize>,
    ) -> bool {
        if !self.prefix.resolve_priority(positions) {
            return false;
        }
        for region in &mut self.regions {
            if let ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
                let (Some(start), Some(end)) = (positions.get(&start), positions.get(&end)) else {
                    return false;
                };
                region.priority = ExceptionRegionPriority::Resolved {
                    start: *start,
                    end: *end,
                };
            }
        }
        true
    }

    fn matches(&self, pipeline: &OptimizationPipeline, items: &[Item]) -> bool {
        let Some(positions) = unique_label_positions(items) else {
            return false;
        };
        if pipeline.borrow_finally_jump_sources.get(&self.donor) != Some(&true)
            || !pipeline.borrow_unreachable_blocks.contains(&self.entry)
            || pipeline.borrow_finally_entries.get(&self.entry) != Some(&None)
            || !pipeline.borrow_jump_entry_labels.contains(&self.entry)
            || !pipeline.preserved_block_boundaries.contains(&self.entry)
            || !self
                .prefix
                .matches_original(items, &pipeline.exception_regions)
        {
            return false;
        }
        let provenance = &pipeline.exception_owner_provenance;
        let target = self.labels[0];
        if !provenance.block_boundaries.contains(&target)
            || !provenance.source_boundaries.contains(&target)
            || provenance.block_boundaries.contains(&self.entry)
        {
            return false;
        }
        if self.labels.iter().any(|label| {
            pipeline.original_nop_targets.contains(label)
                || pipeline.borrow_block_aliases.contains(label)
                || pipeline.folded_original_predecessor_targets.contains(label)
                || pipeline.folded_condition_exit_targets.contains(label)
                || provenance.omitted_nop_counts.contains_key(label)
                || provenance.omitted_nop_locations.contains_key(label)
                || pipeline
                    .exception_regions
                    .iter()
                    .any(|region| region.target == *label)
                || (*label != self.entry
                    && (pipeline.preserved_block_boundaries.contains(label)
                        || pipeline.borrow_unreachable_blocks.contains(label)
                        || pipeline.borrow_finally_entries.contains_key(label)
                        || pipeline.borrow_jump_entry_labels.contains(label)))
        }) {
            return false;
        }
        if self.arrays.iter().any(|(label, offset, original)| {
            positions
                .get(label)
                .and_then(|position| position.checked_sub(*offset))
                .and_then(|start| items.get(start..start.checked_add(original.len())?))
                != Some(original.as_slice())
        }) {
            return false;
        }
        for (labels, original) in &self.incoming {
            let current = items.iter().filter_map(|item| match item {
                Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if labels.contains(&label)) => Some(instruction),
                _ => None,
            });
            if !current.eq(original) {
                return false;
            }
        }
        // The handler's common target also owns its separate final dispatch range.
        // Authenticate only the captured original-priority groups, not that other range.
        let current = pipeline
            .exception_regions
            .iter()
            .filter(|region| {
                self.regions
                    .iter()
                    .any(|original| same_priority(region.priority, original.priority))
                    || self
                        .regions
                        .last()
                        .is_some_and(|awaited| region.target == awaited.target)
            })
            .collect::<Vec<_>>();
        current.len() == self.regions.len()
            && self.regions.iter().all(|original| {
                current
                    .iter()
                    .filter(|region| same_manager_region(region, original))
                    .count()
                    == 1
            })
    }
}

impl OptimizationPipeline {
    pub(super) fn admit_visited_independent_finally_entries(&mut self) {
        let origins =
            std::mem::take(&mut self.exception_owner_provenance.independent_finally_entries);
        if origins.is_empty() {
            return;
        }
        let items = self.graph.copy_items();
        for origin in origins {
            if origin.matches(self, &items) {
                self.borrow_unreachable_blocks.remove(&origin.entry);
                self.borrow_finally_entries.remove(&origin.entry);
                self.borrow_jump_entry_labels.remove(&origin.entry);
                self.borrow_block_aliases.insert(origin.entry);
            }
        }
    }
}

impl Assembler {
    /// Records an original awaited cleanup pending its independent source's visit.
    pub(crate) fn record_independent_finally_entry(
        &mut self,
        entry: Label,
        ordinal: usize,
        await_location: SourceLocation,
        condition_location: SourceLocation,
        fallthrough_location: SourceLocation,
    ) {
        if let Some(origin) = self.original_independent_finally_entry(
            entry,
            ordinal,
            await_location,
            condition_location,
            fallthrough_location,
        ) {
            self.exception_owner_provenance
                .independent_finally_entries
                .push(origin);
        }
    }

    fn original_independent_finally_entry(
        &self,
        entry: Label,
        ordinal: usize,
        await_location: SourceLocation,
        condition_location: SourceLocation,
        fallthrough_location: SourceLocation,
    ) -> Option<IndependentFinallyEntry> {
        if self.python_version != crate::PythonVersion::Py314
            || !self.borrow_unreachable_blocks.contains(&entry)
            || self.borrow_finally_entries.get(&entry) != Some(&None)
            || !self.borrow_jump_entry_labels.contains(&entry)
            || !self.preserved_block_boundaries.contains(&entry)
        {
            return None;
        }
        let prefix = self.awaited_normal_finally_prefix(ordinal)?;
        let items = self.linear_items();
        let positions = unique_label_positions(items)?;
        let pop_at = self.instruction_item_index(ordinal)?;
        let [Item::Label(target), Item::Label(actual)] =
            items.get(array_start(items, pop_at)?..pop_at)?
        else {
            return None;
        };
        let (target, actual) = (*target, *actual);
        let provenance = &self.exception_owner_provenance;
        let ordinary_role = |label: Label| {
            !self.original_nop_targets.contains(&label)
                && !self.borrow_block_aliases.contains(&label)
                && !self.folded_away_condition_targets.contains(&label)
                && !self.folded_away_loop_else_targets.contains(&label)
                && !self.folded_condition_exit_targets.contains(&label)
                && !provenance.omitted_nop_counts.contains_key(&label)
                && !provenance.omitted_nop_locations.contains_key(&label)
                && !self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == label)
        };
        let unguarded = |label: Label| {
            ordinary_role(label)
                && !self.preserved_block_boundaries.contains(&label)
                && !self.borrow_unreachable_blocks.contains(&label)
                && !self.borrow_finally_entries.contains_key(&label)
                && !self.borrow_jump_entry_labels.contains(&label)
        };
        let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
        pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        pop.fusion_policy.mark_converted_pop_block();
        if actual != entry
            || target == entry
            || !ordinary_role(entry)
            || !unguarded(target)
            || !provenance.block_boundaries.contains(&target)
            || provenance.block_boundaries.contains(&entry)
            || items[pop_at] != Item::Instruction(pop)
        {
            return None;
        }
        // Skip only the separately authenticated SETUP array, then count the
        // actual original protected records against native's four-slot copy budget.
        let setup_at = (pop_at + 1..items.len())
            .find(|&index| matches!(items[index], Item::Instruction(_)))?;
        let body_at = (setup_at + 1..items.len())
            .find(|&index| matches!(items[index], Item::Instruction(_)))?;
        let body_ordinal = items[..body_at]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        if !self.has_large_linear_prefix_since(body_ordinal) {
            return None;
        }
        let mut origins = provenance
            .compound_try_owner_exits
            .iter()
            .filter(|(_, endpoint)| *endpoint == target || *endpoint == entry);
        let &(source, endpoint) = origins.next()?;
        if origins.next().is_some() || endpoint != target {
            return None;
        }
        let source_at = *positions.get(&source)?;
        let Item::Label(join) = *items.get(source_at.checked_sub(1)?)? else {
            return None;
        };
        let mut normal_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            Some(0),
        );
        normal_jump.jump_policy.prevent_small_exit_inlining();
        normal_jump.no_location_policy.preserve_location();
        normal_jump
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        if join == source
            || !unguarded(join)
            || !unguarded(source)
            || self.join_labels_have_authored_location(&[join, source])
            || array_start(items, source_at + 1)? != source_at - 1
            || items.get(source_at + 1) != Some(&Item::Instruction(normal_jump))
            || !matches!(items.get(source_at + 2), Some(Item::Label(_)))
        {
            return None;
        }
        let incoming = |labels: &[Label]| {
            items.iter().enumerate().filter_map(|(index, item)| match item {
                Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if labels.contains(&label)) => Some((index, *instruction)),
                _ => None,
            }).collect::<Vec<_>>()
        };
        let target_incoming = incoming(&[target, entry]);
        let [(normal_at, original), (dead_at, dead_jump)] = target_incoming.as_slice() else {
            return None;
        };
        if *normal_at != source_at + 1 || *original != normal_jump {
            return None;
        }
        let [
            Item::Instruction(raise),
            Item::Label(dead),
            Item::Instruction(handler_pop),
            Item::Instruction(jump),
        ] = items.get(dead_at.checked_sub(3)?..=*dead_at)?
        else {
            return None;
        };
        let mut expected_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            raise.location,
            Some(0),
        );
        expected_jump.jump_policy.prevent_small_exit_inlining();
        if raise.location.line < 0
            || *raise != Instruction::new(RAISE_VARARGS, Operand::Value(0), raise.location, Some(1))
            || *handler_pop
                != Instruction::new(POP_EXCEPT, Operand::Value(0), raise.location, Some(0))
            || *jump != expected_jump
            || *dead_jump != expected_jump
            || !unguarded(*dead)
            || !incoming(&[*dead]).is_empty()
            || !matches!(items.get(dead_at + 1), Some(Item::Label(_)))
        {
            return None;
        }
        let mut endings = self
            .exception_regions
            .iter()
            .filter(|region| region.end == *dead);
        let end_region = endings.next()?;
        let ExceptionRegionPriority::OriginalRange(handler_start, handler_end) =
            end_region.priority
        else {
            return None;
        };
        if endings.next().is_some()
            || handler_end != *dead
            || end_region.is_scope_fallback
            || !end_region.preserve_lasti
            || end_region.depth != 1
            || *positions.get(&handler_start)? >= dead_at - 3
            || *positions.get(&end_region.target)? <= *dead_at
        {
            return None;
        }
        let mut previous = *positions.get(&handler_start)?;
        let mut regions = Vec::new();
        for region in self
            .exception_regions
            .iter()
            .filter(|region| same_priority(region.priority, end_region.priority))
        {
            let start = *positions.get(&region.start)?;
            let end = *positions.get(&region.end)?;
            if region.target != end_region.target
                || (regions.is_empty() && region.start != handler_start)
                || region.depth != 1
                || !region.preserve_lasti
                || region.is_scope_fallback
                || start < previous
                || start >= end
                || end > *positions.get(dead)?
            {
                return None;
            }
            previous = end;
            regions.push(*region);
        }
        if previous != *positions.get(dead)? {
            return None;
        }
        let join_incoming = incoming(&[join, source]);
        let [(donor_at, donor_jump), (condition_at, condition)] = join_incoming.as_slice() else {
            return None;
        };
        let [
            Item::Label(await_end),
            Item::Instruction(end_send),
            Item::Instruction(discard),
            Item::Instruction(jump),
        ] = items.get(array_start(items, *donor_at)?..=*donor_at)?
        else {
            return None;
        };
        if await_location.line < 0
            || *end_send != Instruction::new(END_SEND, Operand::Value(0), await_location, Some(1))
            || *discard != Instruction::new(POP_TOP, Operand::Value(0), await_location, Some(0))
            || *jump
                != Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(join),
                    await_location,
                    Some(0),
                )
            || *donor_jump != *jump
            || !unguarded(*await_end)
            || *condition
                != Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(join),
                    condition_location,
                    Some(0),
                )
            || !(*donor_at < *condition_at && *condition_at < source_at)
        {
            return None;
        }
        let [Item::Instruction(call), Item::Instruction(discard)] =
            items.get(source_at.checked_sub(3)?..source_at - 1)?
        else {
            return None;
        };
        if !matches!(call.opcode, CALL | CALL_KW)
            || !matches!(call.operand(), Operand::Value(_))
            || *call != Instruction::new(call.opcode, call.operand(), fallthrough_location, Some(1))
            || *discard
                != Instruction::new(POP_TOP, Operand::Value(0), fallthrough_location, Some(0))
        {
            return None;
        }
        let mut awaits = provenance
            .ordinary_await_ends
            .iter()
            .filter(|origin| origin.entry == *await_end);
        let awaited = awaits.next()?;
        if awaits.next().is_some()
            || awaited.bridge.is_some()
            || awaited.end_send != *end_send
            || awaited.send
                != Instruction::new(SEND, Operand::Forward(*await_end), await_location, Some(2))
        {
            return None;
        }
        let await_incoming = incoming(&[*await_end]);
        let [(send_at, send)] = await_incoming.as_slice() else {
            return None;
        };
        if *send != awaited.send || *send_at >= donor_at - 3 {
            return None;
        }
        let await_end_at = *positions.get(await_end)?;
        let cleanup_at = await_end_at.checked_sub(awaited.cleanup.len())?;
        if items.get(cleanup_at..await_end_at)? != awaited.cleanup || *send_at >= cleanup_at {
            return None;
        }
        let Item::Label(cleanup) = *awaited.cleanup.first()? else {
            return None;
        };
        let mut saved = provenance
            .ordinary_await_cleanups
            .iter()
            .filter(|region| region.target == cleanup);
        let region = saved.next()?;
        if saved.next().is_some()
            || region.is_scope_fallback
            || region.preserve_lasti
            || !matches!(region.priority, ExceptionRegionPriority::OriginalRange(a,b) if a == region.start && b == region.end)
        {
            return None;
        }
        let mut current = self.exception_regions.iter().filter(|candidate| {
            candidate.target == cleanup || same_priority(candidate.priority, region.priority)
        });
        if !same_manager_region(current.next()?, region) || current.next().is_some() {
            return None;
        }
        // The handler fragments establish original dispatch ownership and extent;
        // deadness is proved separately by the bare raise and absent incoming edges.
        regions.push(*region);
        Some(IndependentFinallyEntry {
            entry,
            donor: *await_end,
            labels: [target, entry, join, source, *dead, *await_end],
            prefix,
            arrays: vec![
                (*dead, 1, items[dead_at - 3..=*dead_at].to_vec()),
                (*await_end, 0, items[donor_at - 3..=*donor_at].to_vec()),
                (join, 2, items[source_at - 3..=source_at + 1].to_vec()),
                (cleanup, 0, awaited.cleanup.clone()),
            ],
            incoming: vec![
                (
                    vec![target, entry],
                    target_incoming
                        .into_iter()
                        .map(|(_, record)| record)
                        .collect(),
                ),
                (
                    vec![join, source],
                    join_incoming.iter().map(|(_, record)| *record).collect(),
                ),
                (vec![*dead], Vec::new()),
                (
                    vec![*await_end],
                    await_incoming.iter().map(|(_, record)| *record).collect(),
                ),
            ],
            regions,
        })
    }
}
