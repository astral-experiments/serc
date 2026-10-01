use super::fusion_owner_slots::same_manager_region;
use super::normal_finally_locations::{
    anchored_start, array_start, same_priority, unique_label_positions,
};
use super::{
    Assembler, BorrowFallthrough, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority,
    Instruction, Item, Label, Operand, OptimizationPipeline, SourceLocation, uses_constant_index,
};
use crate::target::opcodes::{
    END_SEND, GET_AWAITABLE, JUMP_FORWARD, LOAD_CONST, NOP, POP_TOP, PUSH_EXC_INFO, SEND,
};

#[cfg(test)]
mod tests;

/// A retained normal edge and a live named-handler edge into an awaited finalizer.
#[derive(Debug)]
pub(super) struct RetainedNormalFinallyEntry {
    entry: Label,
    donor: Label,
    prefix: TwoSetupPrefix,
    author: (Label, [Instruction; 5]),
    roles: Vec<(Label, bool, bool)>,
    prepared_roles: Option<Vec<(Label, bool, bool)>>,
    handler_target: Label,
    arrays: Vec<(Label, usize, Vec<Item>)>,
    incoming: Vec<(Vec<Label>, Vec<Instruction>)>,
    regions: Vec<ExceptionRegion>,
    await_targets: [Label; 2],
}

impl RetainedNormalFinallyEntry {
    pub(super) fn prepare_roles(
        &mut self,
        provenance: &super::exception_owners::ExceptionOwnerProvenance,
        items: &[Item],
    ) -> bool {
        if self.prepared_roles.is_some()
            || self.roles.iter().any(|(label, source, block)| {
                provenance.source_boundaries.contains(label) != *source
                    || provenance.block_boundaries.contains(label) != *block
            })
        {
            return false;
        }
        self.prepared_roles = Some(
            self.roles
                .iter()
                .map(|&(label, source, block)| {
                    (
                        label,
                        source,
                        block || !incoming(items, &[label]).is_empty(),
                    )
                })
                .collect(),
        );
        true
    }
    pub(super) fn remap_constants(
        &mut self,
        map: &[Option<u32>],
        deferred_none: Option<u32>,
    ) -> bool {
        // The prefix owns current handler records. The author snapshot is immutable;
        // the additional await/control arrays contain no constant operands.
        self.prefix.remap_constants(map, deferred_none)
    }

    pub(super) fn resolve_priorities(
        &mut self,
        positions: &rustc_hash::FxHashMap<Label, usize>,
    ) -> bool {
        for region in self.regions.iter_mut().chain(&mut self.prefix.regions) {
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
        let provenance = &pipeline.exception_owner_provenance;
        let Some(roles) = &self.prepared_roles else {
            return false;
        };
        if pipeline.borrow_finally_jump_sources.get(&self.donor) != Some(&true)
            || !pipeline.borrow_unreachable_blocks.contains(&self.entry)
            || pipeline.borrow_finally_entries.get(&self.entry) != Some(&None)
            || !pipeline.borrow_jump_entry_labels.contains(&self.entry)
            || !pipeline.preserved_block_boundaries.contains(&self.entry)
            || !self.prefix.matches(pipeline, items)
            || provenance
                .original_named_handler_cleanup_joins
                .iter()
                .filter(|(label, _)| *label == self.author.0)
                .copied()
                .collect::<Vec<_>>()
                != [self.author]
            || !provenance
                .named_handler_cleanup_targets
                .contains(&self.handler_target)
        {
            return false;
        }
        if roles.iter().any(|(label, source, block)| {
            provenance.source_boundaries.contains(label) != *source
                || provenance.block_boundaries.contains(label) != *block
                || pipeline.original_nop_targets.contains(label)
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
            if !incoming(items, labels)
                .iter()
                .map(|(_, instruction)| instruction)
                .eq(original)
            {
                return false;
            }
        }
        let current = pipeline
            .exception_regions
            .iter()
            .filter(|region| {
                self.regions
                    .iter()
                    .any(|original| same_priority(region.priority, original.priority))
                    || self.await_targets.contains(&region.target)
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

/// The retained outer SETUP and the adjacent unlocated ordinary inner SETUP.
#[derive(Debug)]
struct TwoSetupPrefix {
    arrays: Vec<Vec<Item>>,
    incoming: Vec<(Vec<Label>, Vec<Instruction>)>,
    setup_range: (Label, Label),
    setups: [(Label, Option<SourceLocation>); 2],
    regions: Vec<ExceptionRegion>,
}

impl TwoSetupPrefix {
    fn visit_records_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        for item in self.arrays.iter_mut().flatten() {
            if let Item::Instruction(record) = item {
                visit(record);
            }
        }
    }

    fn remap_constants(&mut self, map: &[Option<u32>], deferred_none: Option<u32>) -> bool {
        let mut valid = true;
        self.visit_records_mut(|record| {
            if !uses_constant_index(record.opcode) {
                return;
            }
            let Operand::Value(index) = record.operand() else {
                valid = false;
                return;
            };
            let index = if index == u32::MAX {
                deferred_none
            } else {
                Some(index)
            };
            if let Some(Some(remapped)) = index.and_then(|index| map.get(index as usize)) {
                record.set_operand(Operand::Value(*remapped));
            } else {
                valid = false;
            }
        });
        valid
    }

    fn matches(&self, pipeline: &OptimizationPipeline, items: &[Item]) -> bool {
        let provenance = &pipeline.exception_owner_provenance;
        self.arrays
            .iter()
            .all(|array| anchored_start(items, array).is_some())
            && self.incoming.iter().all(|(labels, records)| {
                incoming(items, labels)
                    .iter()
                    .map(|(_, record)| record)
                    .eq(records)
            })
            && provenance
                .setup_nop_ranges
                .iter()
                .filter(|range| **range == self.setup_range)
                .count()
                == 1
            && self.setups.iter().all(|setup| {
                provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| *label == setup.0)
                    .copied()
                    .collect::<Vec<_>>()
                    == [*setup]
            })
            && !provenance
                .ordinary_try_setups
                .contains_key(&self.setups[0].0)
            && provenance.ordinary_try_setups.get(&self.setups[1].0)
                == Some(&self.regions[1].target)
            && self.regions.iter().all(|original| {
                let current = pipeline
                    .exception_regions
                    .iter()
                    .filter(|region| {
                        region.target == original.target
                            || same_priority(region.priority, original.priority)
                    })
                    .collect::<Vec<_>>();
                current.len() == 1 && same_manager_region(current[0], original)
            })
    }
}

fn incoming(items: &[Item], labels: &[Label]) -> Vec<(usize, Instruction)> {
    items.iter().enumerate().filter_map(|(index, item)| match item {
        Item::Instruction(instruction) if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label) if labels.contains(&label)) => Some((index, *instruction)),
        _ => None,
    }).collect()
}

struct DiscardedAwait {
    entry: Label,
    completion: Vec<Item>,
    cleanup: Vec<Item>,
    send: Instruction,
    region: ExceptionRegion,
}

impl Assembler {
    fn retained_entry_two_setup_prefix(&self, ordinal: usize) -> Option<TwoSetupPrefix> {
        let items = self.linear_items();
        let positions = unique_label_positions(items)?;
        let index = self.instruction_item_index(ordinal)?;
        let start = array_start(items, index)?;
        if !matches!(items.get(start.checked_sub(1)?), Some(Item::Instruction(previous)) if crate::target::is_scope_exit(previous.opcode))
        {
            return None;
        }
        let [
            Item::Label(setup_start),
            Item::Instruction(marker),
            Item::Label(setup_end),
            Item::Label(outer),
            Item::Label(inner),
        ] = items.get(index + 1..index.checked_add(6)?)?
        else {
            return None;
        };
        let mut expected = Instruction::new(NOP, Operand::Value(0), marker.location, Some(0));
        expected.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        if marker.location.line < 0 || *marker != expected || outer == inner {
            return None;
        }
        let setup_range = (*setup_start, *setup_end);
        let setups = [(*outer, Some(marker.location)), (*inner, None)];
        let provenance = &self.exception_owner_provenance;
        if provenance
            .setup_nop_ranges
            .iter()
            .filter(|range| **range == setup_range)
            .count()
            != 1
            || setups.iter().any(|setup| {
                provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| *label == setup.0)
                    .copied()
                    .collect::<Vec<_>>()
                    != [*setup]
            })
            || provenance.ordinary_try_setups.contains_key(outer)
        {
            return None;
        }
        let inner_handler = *provenance.ordinary_try_setups.get(inner)?;
        let mut owned = Vec::new();
        for owner in [*outer, *inner] {
            let mut regions = self.exception_regions.iter().filter(|region| matches!(region.priority, ExceptionRegionPriority::OriginalRange(begin, _) if begin == owner));
            let region = *regions.next()?;
            if regions.next().is_some()
                || region.start != owner
                || region.depth != 0
                || region.preserve_lasti
                || region.is_scope_fallback
                || !matches!(region.priority, ExceptionRegionPriority::OriginalRange(begin, end) if begin == owner && end == region.end)
                || self
                    .exception_regions
                    .iter()
                    .filter(|candidate| candidate.target == region.target)
                    .count()
                    != 1
                || (owner == *inner && region.target != inner_handler)
            {
                return None;
            }
            owned.push(region);
        }
        let regions: [ExceptionRegion; 2] = owned.try_into().ok()?;
        if *positions.get(&regions[1].end)? >= *positions.get(&regions[0].end)?
            || regions[0].target == regions[1].target
        {
            return None;
        }
        let body = index.checked_add(6)?;
        let mut end = body;
        while let Some(Item::Instruction(record)) = items.get(end) {
            if !matches!(record.operand(), Operand::Value(_))
                || crate::target::is_scope_exit(record.opcode)
                || record.opcode == NOP
            {
                return None;
            }
            end += 1;
        }
        let [.., Item::Instruction(awaitable), Item::Instruction(none)] = items.get(body..end)?
        else {
            return None;
        };
        if awaitable.opcode != GET_AWAITABLE
            || awaitable.operand() != Operand::Value(0)
            || none.opcode != LOAD_CONST
            || end - body <= 4
        {
            return None;
        }
        let [Item::Label(_), Item::Instruction(send)] = items.get(end..end.checked_add(2)?)? else {
            return None;
        };
        if send.opcode != SEND {
            return None;
        }
        let mut awaits = provenance
            .ordinary_await_ends
            .iter()
            .filter(|origin| origin.send == *send);
        let awaited = awaits.next()?;
        if awaits.next().is_some()
            || awaited.bridge.is_some()
            || send.operand() != Operand::Forward(awaited.entry)
        {
            return None;
        }
        let completion_start = *positions.get(&awaited.entry)?;
        let completion_end = *positions.get(&regions[1].end)?;
        let completion = items.get(completion_start..=completion_end)?;
        if !matches!(completion, [Item::Label(_), Item::Instruction(end_send), Item::Instruction(discard), Item::Label(_)] if *end_send == awaited.end_send && end_send.opcode == END_SEND && *discard == Instruction::new(POP_TOP, Operand::Value(0), end_send.location, Some(0)))
        {
            return None;
        }
        let final_await =
            self.retained_entry_discarded_await(awaited.entry, awaited.end_send.location)?;
        let mut arrays = vec![items[start..end + 2].to_vec(), completion.to_vec()];
        for region in &regions {
            let handler = *positions.get(&region.target)?;
            let mut record = handler + 1;
            while matches!(items.get(record), Some(Item::Label(_))) {
                record += 1;
            }
            if !matches!(items.get(record), Some(Item::Instruction(first)) if first.opcode == PUSH_EXC_INFO)
            {
                return None;
            }
            arrays.push(items[handler..=record].to_vec());
        }
        arrays.push(final_await.cleanup);
        let heads = items[start..index]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let interior = items[index + 1..end]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if !incoming(items, &interior).is_empty()
            || self
                .exception_regions
                .iter()
                .any(|region| heads.contains(&region.target) || interior.contains(&region.target))
        {
            return None;
        }
        let incoming_records = arrays
            .iter()
            .map(|array| {
                let labels = array
                    .iter()
                    .filter_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    })
                    .collect::<Vec<_>>();
                let records = incoming(items, &labels)
                    .iter()
                    .map(|(_, record)| *record)
                    .collect();
                (labels, records)
            })
            .collect();
        for (source, _) in incoming(items, &heads) {
            arrays.push(items[array_start(items, source)?..=source].to_vec());
        }
        Some(TwoSetupPrefix {
            arrays,
            incoming: incoming_records,
            setup_range,
            setups,
            regions: regions.into_iter().chain([final_await.region]).collect(),
        })
    }
    fn retained_entry_discarded_await(
        &self,
        entry: Label,
        location: SourceLocation,
    ) -> Option<DiscardedAwait> {
        let items = self.linear_items();
        let positions = unique_label_positions(items)?;
        let end_at = *positions.get(&entry)?;
        let end_send = Instruction::new(END_SEND, Operand::Value(0), location, Some(1));
        let discard = Instruction::new(POP_TOP, Operand::Value(0), location, Some(0));
        let completion = vec![
            Item::Label(entry),
            Item::Instruction(end_send),
            Item::Instruction(discard),
        ];
        if location.line < 0 || items.get(end_at..end_at.checked_add(3)?)? != completion {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut origins = provenance
            .ordinary_await_ends
            .iter()
            .filter(|origin| origin.entry == entry);
        let origin = origins.next()?;
        if origins.next().is_some()
            || origin.bridge.is_some()
            || origin.end_send != end_send
            || origin.send != Instruction::new(SEND, Operand::Forward(entry), location, Some(2))
        {
            return None;
        }
        let edges = incoming(items, &[entry]);
        let [(send_at, send)] = edges.as_slice() else {
            return None;
        };
        let cleanup_at = end_at.checked_sub(origin.cleanup.len())?;
        if *send != origin.send
            || *send_at >= cleanup_at
            || items.get(cleanup_at..end_at)? != origin.cleanup
        {
            return None;
        }
        let Item::Label(cleanup) = *origin.cleanup.first()? else {
            return None;
        };
        let mut saved = provenance
            .ordinary_await_cleanups
            .iter()
            .filter(|region| region.target == cleanup);
        let region = saved.next()?;
        if saved.next().is_some()
            || region.depth != 2
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
        Some(DiscardedAwait {
            entry,
            completion,
            cleanup: origin.cleanup.clone(),
            send: *send,
            region: *region,
        })
    }

    pub(crate) fn record_retained_normal_finally_entry(
        &mut self,
        entry: Label,
        ordinal: usize,
        first: SourceLocation,
        last: SourceLocation,
    ) {
        if let Some(origin) =
            self.original_retained_normal_finally_entry(entry, ordinal, first, last)
        {
            self.exception_owner_provenance
                .retained_normal_finally_entries
                .push(origin);
        }
    }

    fn original_retained_normal_finally_entry(
        &self,
        entry: Label,
        ordinal: usize,
        first: SourceLocation,
        last: SourceLocation,
    ) -> Option<RetainedNormalFinallyEntry> {
        if self.python_version != crate::PythonVersion::Py314
            || !self.borrow_unreachable_blocks.contains(&entry)
            || self.borrow_finally_entries.get(&entry) != Some(&None)
            || !self.borrow_jump_entry_labels.contains(&entry)
            || !self.preserved_block_boundaries.contains(&entry)
        {
            return None;
        }
        let prefix = self.retained_entry_two_setup_prefix(ordinal)?;
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
        let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
        pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        pop.fusion_policy.mark_converted_pop_block();
        if actual != entry
            || target == entry
            || items[pop_at] != Item::Instruction(pop)
            || !provenance.source_boundaries.contains(&target)
            || !provenance.block_boundaries.contains(&target)
            || provenance.block_boundaries.contains(&entry)
        {
            return None;
        }
        let mut exits = provenance
            .compound_try_owner_exits
            .iter()
            .filter(|(_, endpoint)| *endpoint == target || *endpoint == entry);
        let &(source, endpoint) = exits.next()?;
        if exits.next().is_some() || endpoint != target {
            return None;
        }
        let source_at = *positions.get(&source)?;
        let Item::Label(join) = *items.get(source_at.checked_sub(1)?)? else {
            return None;
        };
        let mut normal_jump =
            Instruction::new(JUMP_FORWARD, Operand::Forward(target), last, Some(0));
        normal_jump.jump_policy.mark_original_empty_join();
        normal_jump
            .jump_policy
            .retain_redundant_nop_after_threading();
        normal_jump.jump_policy.prevent_small_exit_inlining();
        normal_jump
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
        if join == source
            || array_start(items, source_at + 1)? != source_at - 1
            || items.get(source_at + 1) != Some(&Item::Instruction(normal_jump))
            || !matches!(items.get(source_at + 2), Some(Item::Label(_)))
            || ![join, source].iter().any(|label| {
                provenance.source_boundaries.contains(label)
                    && provenance.block_boundaries.contains(label)
            })
        {
            return None;
        }
        let target_incoming = incoming(items, &[target, entry]);
        let [(normal_at, normal), (handler_at, handler_jump)] = target_incoming.as_slice() else {
            return None;
        };
        if *normal_at != source_at + 1 || *normal != normal_jump {
            return None;
        }
        let mut authors = provenance
            .original_named_handler_cleanup_joins
            .iter()
            .filter(|(_, array)| array[4].operand() == Operand::Forward(target));
        let &author = authors.next()?;
        if authors.next().is_some() {
            return None;
        }
        // Only these two existing writer steps follow the immutable author snapshot.
        let mut current_handler = author.1;
        current_handler[4].allow_unconditional_block_inlining();
        current_handler[4].jump_policy.prevent_small_exit_inlining();
        let handler_start = array_start(items, *handler_at)?;
        let handler_records = handler_at.checked_sub(4)?;
        let handler_aliases = items
            .get(handler_start..handler_records)?
            .iter()
            .map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Option<Vec<_>>>()?;
        if !handler_aliases.contains(&author.0)
            || items.get(handler_records..=*handler_at)? != current_handler.map(Item::Instruction)
            || *handler_jump != current_handler[4]
            || !matches!(items.get(handler_at + 1), Some(Item::Label(_)))
        {
            return None;
        }
        let mut endings = self.exception_regions.iter().filter(|region| {
            region.end == author.0
                && provenance
                    .named_handler_cleanup_targets
                    .contains(&region.target)
        });
        let end_region = endings.next()?;
        let ExceptionRegionPriority::OriginalRange(handler_body, handler_end) = end_region.priority
        else {
            return None;
        };
        if endings.next().is_some()
            || handler_end != author.0
            || !provenance
                .named_handler_cleanup_targets
                .contains(&end_region.target)
            || *positions.get(&handler_body)? >= handler_start
            || *positions.get(&end_region.target)? <= *handler_at
        {
            return None;
        }
        let mut previous = *positions.get(&handler_body)?;
        let mut regions = Vec::new();
        for region in self
            .exception_regions
            .iter()
            .filter(|region| same_priority(region.priority, end_region.priority))
        {
            let start = *positions.get(&region.start)?;
            let end = *positions.get(&region.end)?;
            if region.target != end_region.target
                || (regions.is_empty() && region.start != handler_body)
                || region.depth != 1
                || !region.preserve_lasti
                || region.is_scope_fallback
                || start < previous
                || start >= end
                || end > *positions.get(&author.0)?
            {
                return None;
            }
            previous = end;
            regions.push(*region);
        }
        if previous != *positions.get(&author.0)? {
            return None;
        }
        let join_incoming = incoming(items, &[join, source]);
        let [(donor_at, donor_jump)] = join_incoming.as_slice() else {
            return None;
        };
        let [Item::Label(donor), _, _, Item::Instruction(jump)] =
            items.get(array_start(items, *donor_at)?..=*donor_at)?
        else {
            return None;
        };
        if *donor_jump != Instruction::new(JUMP_FORWARD, Operand::Forward(join), first, Some(0))
            || jump != donor_jump
        {
            return None;
        }
        let Item::Label(last_end) = *items.get(source_at.checked_sub(4)?)? else {
            return None;
        };
        let first_await = self.retained_entry_discarded_await(*donor, first)?;
        let last_await = self.retained_entry_discarded_await(last_end, last)?;
        if *positions.get(&last_end)? + 3 != source_at - 1
            || *donor_at >= *positions.get(&last_end)?
        {
            return None;
        }
        let mut labels = vec![target, entry, join, source, *donor, last_end];
        labels.extend(&handler_aliases);
        if labels.iter().enumerate().any(|(index, label)| {
            labels[..index].contains(label)
                || self.original_nop_targets.contains(label)
                || self.borrow_block_aliases.contains(label)
                || self.folded_away_condition_targets.contains(label)
                || self.folded_away_loop_else_targets.contains(label)
                || self.folded_condition_exit_targets.contains(label)
                || provenance.omitted_nop_counts.contains_key(label)
                || provenance.omitted_nop_locations.contains_key(label)
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == *label)
                || (*label != entry
                    && (self.preserved_block_boundaries.contains(label)
                        || self.borrow_unreachable_blocks.contains(label)
                        || self.borrow_finally_entries.contains_key(label)
                        || self.borrow_jump_entry_labels.contains(label)))
        }) {
            return None;
        }
        let mut arrays = vec![(join, 0, items[source_at - 1..=source_at + 1].to_vec())];
        let mut incoming_records = vec![
            (
                vec![target, entry],
                target_incoming.iter().map(|(_, record)| *record).collect(),
            ),
            (
                vec![join, source],
                join_incoming.iter().map(|(_, record)| *record).collect(),
            ),
            (
                handler_aliases.clone(),
                incoming(items, &handler_aliases)
                    .iter()
                    .map(|(_, record)| *record)
                    .collect(),
            ),
        ];
        let await_targets = [first_await.region.target, last_await.region.target];
        for awaited in [first_await, last_await] {
            let Item::Label(cleanup) = awaited.cleanup[0] else {
                return None;
            };
            arrays.push((awaited.entry, 0, awaited.completion));
            arrays.push((cleanup, 0, awaited.cleanup));
            incoming_records.push((vec![awaited.entry], vec![awaited.send]));
            regions.push(awaited.region);
        }
        Some(RetainedNormalFinallyEntry {
            entry,
            donor: *donor,
            prefix,
            author,
            roles: labels
                .into_iter()
                .map(|label| {
                    (
                        label,
                        provenance.source_boundaries.contains(&label),
                        provenance.block_boundaries.contains(&label),
                    )
                })
                .collect(),
            prepared_roles: None,
            handler_target: end_region.target,
            arrays,
            incoming: incoming_records,
            regions,
            await_targets,
        })
    }
}

impl OptimizationPipeline {
    pub(super) fn admit_visited_retained_normal_finally_entries(&mut self) {
        let origins = std::mem::take(
            &mut self
                .exception_owner_provenance
                .retained_normal_finally_entries,
        );
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
