//! A visited Boolean-result STORE survives before synchronous normal cleanup.

use super::fusion_owner_slots::same_manager_region;
use super::normal_finally_locations::{
    anchored_start, array_start, same_priority, unique_label_positions,
};
use super::{
    Assembler, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    Operand, OptimizationPipeline, SourceLocation,
};
use crate::target::opcodes::{
    CALL, COPY, DELETE_FAST, JUMP_FORWARD, LOAD_ATTR, LOAD_CONST, LOAD_FAST, LOAD_SPECIAL, NOP,
    POP_EXCEPT, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, POP_TOP, PUSH_EXC_INFO, STORE_FAST, SWAP,
};

#[cfg(test)]
mod tests;

/// Complete authored sources and their current constant-remapped representation.
#[derive(Debug)]
pub(super) struct RetainedStoreFinallyEntry {
    entry: Label,
    donor: Label,
    target: Label,
    arrays: Vec<Vec<Item>>,
    handler_array: usize,
    handler_author: [Instruction; 5],
    handler_current_none: u32,
    value_author: (Label, Vec<Item>),
    incoming: Vec<(Vec<Label>, Vec<Instruction>)>,
    roles: Vec<(Label, bool, bool, bool)>,
    prepared_roles: Option<Vec<(Label, bool, bool, bool)>>,
    prepared_regions: Option<Vec<ExceptionRegion>>,
    regions: Vec<ExceptionRegion>,
    ordinary: (Label, Label),
    manager: Label,
    handler_target: Label,
}

fn incoming(items: &[Item], labels: &[Label]) -> Vec<(usize, Instruction)> {
    items.iter().enumerate().filter_map(|(index, item)| match item {
        Item::Instruction(record) if matches!(record.operand(), Operand::Forward(label) | Operand::Backward(label) if labels.contains(&label)) => Some((index, *record)),
        _ => None,
    }).collect()
}

fn instruction(item: &Item) -> Option<Instruction> {
    match item {
        Item::Instruction(record) => Some(*record),
        Item::Label(_) => None,
    }
}

impl RetainedStoreFinallyEntry {
    fn handler_projection_matches(&self) -> bool {
        let mut expected = self.handler_author;
        expected[1].set_operand(Operand::Value(self.handler_current_none));
        self.arrays
            .get(self.handler_array)
            .is_some_and(|array| array.iter().filter_map(instruction).eq(expected))
    }

    pub(super) fn remap_constants(
        &mut self,
        map: &[Option<u32>],
        deferred_none: Option<u32>,
    ) -> bool {
        // Only the cleanup's None operand is remapped; all other fields replay
        // against the immutable located writer, including its jump policy.
        if !self.handler_projection_matches() {
            return false;
        }
        let index = if self.handler_current_none == u32::MAX {
            let Some(index) = deferred_none else {
                return false;
            };
            index
        } else {
            self.handler_current_none
        };
        let Some(Some(remapped)) = map.get(index as usize) else {
            return false;
        };
        self.handler_current_none = *remapped;
        let Some(Item::Instruction(load)) = self.arrays[self.handler_array].get_mut(2) else {
            return false;
        };
        load.set_operand(Operand::Value(*remapped));
        self.handler_projection_matches()
    }

    pub(super) fn resolve_priorities(
        &mut self,
        positions: &rustc_hash::FxHashMap<Label, usize>,
    ) -> bool {
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

    pub(super) fn prepare_roles(
        &mut self,
        provenance: &super::exception_owners::ExceptionOwnerProvenance,
        items: &[Item],
        regions: &[ExceptionRegion],
    ) -> bool {
        if self.prepared_roles.is_some()
            || self.prepared_regions.is_some()
            || !self.handler_projection_matches()
            || self.roles.iter().any(|(label, source, block, _)| {
                provenance.source_boundaries.contains(label) != *source
                    || provenance.block_boundaries.contains(label) != *block
            })
        {
            return false;
        }
        self.prepared_roles = Some(
            self.roles
                .iter()
                .map(|&(label, source, block, preserved)| {
                    (
                        label,
                        source,
                        block || !incoming(items, &[label]).is_empty(),
                        preserved,
                    )
                })
                .collect(),
        );
        // Compilation is complete; later owners must not change any array's coverage.
        self.prepared_regions = Some(regions.to_vec());
        true
    }

    fn matches(&self, pipeline: &OptimizationPipeline, items: &[Item]) -> bool {
        let provenance = &pipeline.exception_owner_provenance;
        let Some(roles) = &self.prepared_roles else {
            return false;
        };
        let Some(regions) = &self.prepared_regions else {
            return false;
        };
        if regions.len() != pipeline.exception_regions.len()
            || !regions
                .iter()
                .zip(&pipeline.exception_regions)
                .all(|(original, current)| same_manager_region(original, current))
            || pipeline.borrow_finally_jump_sources.get(&self.donor) != Some(&true)
            || !pipeline.borrow_unreachable_blocks.contains(&self.entry)
            || pipeline.borrow_finally_entries.get(&self.entry) != Some(&None)
            || !pipeline.borrow_jump_entry_labels.contains(&self.entry)
            || !self.handler_projection_matches()
            || !self
                .arrays
                .iter()
                .all(|array| anchored_start(items, array).is_some())
            || provenance
                .compound_try_owner_exits
                .iter()
                .filter(|(_, target)| *target == self.target || *target == self.entry)
                .copied()
                .collect::<Vec<_>>()
                != [(self.donor, self.target)]
            || provenance
                .original_value_jump_prefixes
                .iter()
                .filter(|(label, _)| *label == self.value_author.0)
                .collect::<Vec<_>>()
                != [&self.value_author]
            || provenance.ordinary_try_setups.get(&self.ordinary.0) != Some(&self.ordinary.1)
            || !provenance.synchronous_with_setups.contains(&self.manager)
            || provenance.ordinary_try_setups.contains_key(&self.manager)
            || provenance
                .setups
                .iter()
                .filter(|(label, _)| *label == self.manager)
                .copied()
                .collect::<Vec<_>>()
                != [(self.manager, None)]
            || !provenance
                .named_handler_cleanup_targets
                .contains(&self.handler_target)
        {
            return false;
        }
        if roles.iter().any(|(label, source, block, preserved)| {
            provenance.source_boundaries.contains(label) != *source
                || provenance.block_boundaries.contains(label) != *block
                || pipeline.preserved_block_boundaries.contains(label) != *preserved
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
                    && (pipeline.borrow_unreachable_blocks.contains(label)
                        || pipeline.borrow_finally_entries.contains_key(label)
                        || pipeline.borrow_jump_entry_labels.contains(label)))
        }) {
            return false;
        }
        if self.incoming.iter().any(|(labels, records)| {
            !incoming(items, labels)
                .iter()
                .map(|(_, record)| record)
                .eq(records)
        }) {
            return false;
        }
        let current = pipeline
            .exception_regions
            .iter()
            .filter(|region| {
                self.regions.iter().any(|original| {
                    same_priority(original.priority, region.priority)
                        || original.target == region.target
                })
            })
            .collect::<Vec<_>>();
        current.len() == self.regions.len()
            && self
                .regions
                .iter()
                .zip(current)
                .all(|(original, current)| same_manager_region(original, current))
    }
}

impl Assembler {
    pub(crate) fn record_retained_store_finally_entry(
        &mut self,
        entry: Label,
        ordinal: usize,
        store_location: SourceLocation,
        manager_location: SourceLocation,
    ) {
        if let Some(origin) = self.original_retained_store_finally_entry(
            entry,
            ordinal,
            store_location,
            manager_location,
        ) {
            self.exception_owner_provenance
                .retained_store_finally_entries
                .push(origin);
        }
    }

    fn original_retained_store_finally_entry(
        &self,
        entry: Label,
        ordinal: usize,
        store_location: SourceLocation,
        manager_location: SourceLocation,
    ) -> Option<RetainedStoreFinallyEntry> {
        if self.python_version != crate::PythonVersion::Py314
            || store_location.line < 0
            || manager_location.line < 0
            || !self.borrow_unreachable_blocks.contains(&entry)
            || self.borrow_finally_entries.get(&entry) != Some(&None)
            || !self.borrow_jump_entry_labels.contains(&entry)
            || self.preserved_block_boundaries.contains(&entry)
        {
            return None;
        }
        let items = self.linear_items();
        let positions = unique_label_positions(items)?;
        let provenance = &self.exception_owner_provenance;
        let pop_at = self.instruction_item_index(ordinal)?;
        let start = array_start(items, pop_at)?;
        let [Item::Label(target), Item::Label(actual)] = items.get(start..pop_at)? else {
            return None;
        };
        let target = *target;
        let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
        pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        pop.fusion_policy.mark_converted_pop_block();
        if *actual != entry
            || target == entry
            || items[pop_at] != Item::Instruction(pop)
            || !provenance.source_boundaries.contains(&target)
            || !provenance.block_boundaries.contains(&target)
            || provenance.block_boundaries.contains(&entry)
        {
            return None;
        }
        let prefix_end = pop_at.checked_add(11)?;
        let [
            Item::Instruction(receiver),
            Item::Instruction(attribute),
            Item::Instruction(copy),
            Item::Instruction(exit),
            Item::Instruction(swap2),
            Item::Instruction(swap3),
            Item::Instruction(enter),
            Item::Instruction(call),
            Item::Label(manager),
            Item::Instruction(discard),
        ] = items.get(pop_at + 1..prefix_end)?
        else {
            return None;
        };
        let Operand::Value(receiver_arg) = receiver.operand() else {
            return None;
        };
        let Operand::Value(attribute_arg) = attribute.operand() else {
            return None;
        };
        if receiver.location.line != manager_location.line
            || receiver.location.column != manager_location.column
            || receiver.location.end_column > manager_location.end_column
            || *receiver
                != Instruction::new(
                    LOAD_FAST,
                    Operand::Value(receiver_arg),
                    receiver.location,
                    Some(1),
                )
            || *attribute
                != Instruction::new(
                    LOAD_ATTR,
                    Operand::Value(attribute_arg),
                    manager_location,
                    Some(1),
                )
            || attribute_arg & 1 != 0
            || [*copy, *exit, *swap2, *swap3, *enter, *call, *discard]
                != [
                    Instruction::new(COPY, Operand::Value(1), manager_location, Some(2)),
                    Instruction::new(LOAD_SPECIAL, Operand::Value(1), manager_location, Some(3)),
                    Instruction::new(SWAP, Operand::Value(2), manager_location, Some(3)),
                    Instruction::new(SWAP, Operand::Value(3), manager_location, Some(3)),
                    Instruction::new(LOAD_SPECIAL, Operand::Value(0), manager_location, Some(4)),
                    Instruction::new(CALL, Operand::Value(0), manager_location, Some(3)),
                    Instruction::new(POP_TOP, Operand::Value(0), manager_location, Some(2)),
                ]
            || !provenance.synchronous_with_setups.contains(manager)
            || provenance.ordinary_try_setups.contains_key(manager)
            || provenance
                .setups
                .iter()
                .filter(|(label, _)| label == manager)
                .copied()
                .collect::<Vec<_>>()
                != [(*manager, None)]
        {
            return None;
        }
        let exits = provenance
            .compound_try_owner_exits
            .iter()
            .filter(|(_, endpoint)| *endpoint == target || *endpoint == entry)
            .copied()
            .collect::<Vec<_>>();
        let [(donor, endpoint)] = exits.as_slice() else {
            return None;
        };
        if *endpoint != target {
            return None;
        }
        let donor = *donor;
        let source_at = *positions.get(&donor)?;
        let source_start = source_at.checked_sub(2)?;
        let [
            Item::Label(join),
            Item::Instruction(store),
            Item::Label(_),
            Item::Instruction(normal_jump),
        ] = items.get(source_start..source_at.checked_add(2)?)?
        else {
            return None;
        };
        let Operand::Value(binding) = store.operand() else {
            return None;
        };
        let mut expected_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            store_location,
            Some(0),
        );
        expected_jump.jump_policy.prevent_small_exit_inlining();
        if *store != Instruction::new(STORE_FAST, Operand::Value(binding), store_location, Some(0))
            || *normal_jump != expected_jump
            || !matches!(items.get(source_at + 2), Some(Item::Label(_)))
            || !provenance.source_boundaries.contains(join)
            || !provenance.block_boundaries.contains(join)
            || !self.preserved_block_boundaries.contains(join)
            || !incoming(items, &[donor]).is_empty()
        {
            return None;
        }
        let join_edges = incoming(items, &[*join]);
        let [(boolean_at, boolean)] = join_edges.as_slice() else {
            return None;
        };
        if !matches!(boolean.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
            || boolean.operand() != Operand::Forward(*join)
            || *boolean_at >= source_start
        {
            return None;
        }
        let values = provenance
            .original_value_jump_prefixes
            .iter()
            .filter(|(_, original)| original.contains(&Item::Instruction(*boolean)))
            .collect::<Vec<_>>();
        let [value_author] = values.as_slice() else {
            return None;
        };
        anchored_start(items, &value_author.1)?;
        let target_edges = incoming(items, &[target, entry]);
        let [(normal_at, normal), (handler_at, handler_jump)] = target_edges.as_slice() else {
            return None;
        };
        if *normal_at != source_at + 1 || *normal != *normal_jump {
            return None;
        }
        let handler_start = handler_at.checked_sub(5)?;
        let [
            Item::Label(handler_end),
            Item::Instruction(handler_pop),
            Item::Instruction(none),
            Item::Instruction(handler_store),
            Item::Instruction(delete),
            Item::Instruction(_),
        ] = items.get(handler_start..=*handler_at)?
        else {
            return None;
        };
        let Operand::Value(none_arg) = none.operand() else {
            return None;
        };
        let Operand::Value(handler_binding) = handler_store.operand() else {
            return None;
        };
        let handler_location = handler_jump.location;
        let mut expected_handler_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            handler_location,
            Some(0),
        );
        expected_handler_jump
            .jump_policy
            .prevent_small_exit_inlining();
        let handler_author = [
            Instruction::new(POP_EXCEPT, Operand::Value(0), handler_location, Some(0)),
            Instruction::new(
                LOAD_CONST,
                Operand::Value(none_arg),
                handler_location,
                Some(1),
            ),
            Instruction::new(
                STORE_FAST,
                Operand::Value(handler_binding),
                handler_location,
                Some(0),
            ),
            Instruction::new(
                DELETE_FAST,
                Operand::Value(handler_binding),
                handler_location,
                Some(0),
            ),
            expected_handler_jump,
        ];
        if handler_location.line < 0
            || [*handler_pop, *none, *handler_store, *delete, *handler_jump] != handler_author
            || !matches!(items.get(handler_at + 1), Some(Item::Label(_)))
            || array_start(items, *handler_at)? != handler_start
        {
            return None;
        }
        let ordinary = self.exception_regions.iter().find(|region| matches!(region.priority, ExceptionRegionPriority::OriginalRange(_, end) if end == donor) && region.end == donor && region.depth == 0)?;
        let ExceptionRegionPriority::OriginalRange(ordinary_start, _) = ordinary.priority else {
            return None;
        };
        if provenance.ordinary_try_setups.get(&ordinary_start) != Some(&ordinary.target) {
            return None;
        }
        let handler_region = self.exception_regions.iter().find(|region| {
            region.end == *handler_end
                && provenance
                    .named_handler_cleanup_targets
                    .contains(&region.target)
        })?;
        let manager_region = self.exception_regions.iter().find(|region| region.start == *manager && matches!(region.priority, ExceptionRegionPriority::OriginalRange(start, end) if start == *manager && end == region.end))?;
        let manager_handler = self.exception_regions.iter().find(|region| region.start == manager_region.target && matches!(region.priority, ExceptionRegionPriority::OriginalRange(start, end) if start == region.start && end == region.end))?;
        let mut regions = Vec::new();
        for (owner, depth, lasti) in [
            (ordinary, 0, false),
            (handler_region, 1, true),
            (manager_region, 2, true),
            (manager_handler, 4, true),
        ] {
            let ExceptionRegionPriority::OriginalRange(begin, end) = owner.priority else {
                return None;
            };
            let mut previous = *positions.get(&begin)?;
            let group = self
                .exception_regions
                .iter()
                .filter(|region| same_priority(region.priority, owner.priority))
                .collect::<Vec<_>>();
            if group.first()?.start != begin {
                return None;
            }
            for region in group {
                let start = *positions.get(&region.start)?;
                let stop = *positions.get(&region.end)?;
                if start < previous
                    || start >= stop
                    || stop > *positions.get(&end)?
                    || region.depth != depth
                    || region.preserve_lasti != lasti
                    || region.is_scope_fallback
                    || region.target != owner.target
                {
                    return None;
                }
                previous = stop;
                regions.push(*region);
            }
            if previous != *positions.get(&end)? {
                return None;
            }
        }
        // Capture the original table order, not the order of the four owner groups.
        let regions = self
            .exception_regions
            .iter()
            .filter(|candidate| {
                regions
                    .iter()
                    .any(|original| same_manager_region(original, candidate))
            })
            .copied()
            .collect();
        let mut arrays = vec![
            items[start..prefix_end].to_vec(),
            items[source_start..source_at + 2].to_vec(),
            items[handler_start..=*handler_at].to_vec(),
            value_author.1.clone(),
        ];
        let manager_handler_at = *positions.get(&manager_region.target)?;
        let first = instruction(items.get(manager_handler_at + 1)?)?;
        if first != Instruction::new(PUSH_EXC_INFO, Operand::Value(0), manager_location, Some(5)) {
            return None;
        }
        arrays.push(items[manager_handler_at..=manager_handler_at + 1].to_vec());
        let mut labels = vec![target, entry, donor, *join, *handler_end, *manager];
        for item in &value_author.1 {
            if let Item::Label(label) = item
                && !labels.contains(label)
            {
                labels.push(*label);
            }
        }
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
                    && (self.borrow_unreachable_blocks.contains(label)
                        || self.borrow_finally_entries.contains_key(label)
                        || self.borrow_jump_entry_labels.contains(label)))
        }) {
            return None;
        }
        let incoming_records = arrays
            .iter()
            .map(|array| {
                let heads = array
                    .iter()
                    .filter_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    })
                    .collect::<Vec<_>>();
                let records = incoming(items, &heads)
                    .into_iter()
                    .map(|(_, record)| record)
                    .collect();
                (heads, records)
            })
            .collect();
        Some(RetainedStoreFinallyEntry {
            entry,
            donor,
            target,
            arrays,
            handler_array: 2,
            handler_author,
            handler_current_none: none_arg,
            value_author: (*value_author).clone(),
            incoming: incoming_records,
            roles: labels
                .into_iter()
                .map(|label| {
                    (
                        label,
                        provenance.source_boundaries.contains(&label),
                        provenance.block_boundaries.contains(&label),
                        self.preserved_block_boundaries.contains(&label),
                    )
                })
                .collect(),
            prepared_roles: None,
            prepared_regions: None,
            regions,
            ordinary: (ordinary_start, ordinary.target),
            manager: *manager,
            handler_target: handler_region.target,
        })
    }
}

impl OptimizationPipeline {
    pub(super) fn admit_visited_retained_store_finally_entries(&mut self) {
        let origins = std::mem::take(
            &mut self
                .exception_owner_provenance
                .retained_store_finally_entries,
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
                self.preserved_block_boundaries.insert(origin.entry);
                self.borrow_block_aliases.insert(origin.entry);
            }
        }
    }
}
