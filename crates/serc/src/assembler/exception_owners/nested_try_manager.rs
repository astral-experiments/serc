//! The outer ordinary POP after a value return exits an inner try and one manager.

#[cfg(test)]
mod tests;

use super::{ExceptionOwnerTail, OwnerLabelPositions};
use crate::assembler::fusion_owner_slots::same_manager_region;
use crate::assembler::normal_finally_locations::{anchored_start, unique_label_positions};
use crate::assembler::{
    Assembler, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    LayoutAssembler, Operand, SourceLocation,
};
use crate::target::opcodes::{
    CALL, CALL_KW, LOAD_ATTR, LOAD_CONST, LOAD_DEREF, LOAD_FAST, LOAD_FAST_BORROW, LOAD_GLOBAL,
    NOP, NOT_TAKEN, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, POP_TOP, PUSH_NULL, SWAP,
};
use rustc_hash::FxHashSet;

#[derive(Debug)]
pub(in crate::assembler) struct CopiedNestedTryManagerReturn {
    entry: Label,
    entry_head: Vec<Label>,
    ordinary: [(Label, Label, Label); 2],
    manager: Label,
    finalizer: Label,
    setups: Vec<(Label, Label, Option<SourceLocation>, Vec<Item>)>,
    incoming: Vec<Instruction>,
    regions: Option<[ExceptionRegion; 3]>,
    original_fallbacks: Vec<ExceptionRegion>,
    region_heads: Option<[(Vec<Label>, Vec<Label>); 3]>,
    prefix: Vec<Instruction>,
    inner_pop: Instruction,
    cleanup: [Instruction; 7],
    pop: Instruction,
    load: Instruction,
    pub(super) condition: Instruction,
    marker: Instruction,
}

impl CopiedNestedTryManagerReturn {
    pub(in crate::assembler) fn record_stopiteration_region(&mut self, region: ExceptionRegion) {
        self.original_fallbacks.push(region);
    }

    pub(in crate::assembler) fn remap_constants(
        &mut self,
        indices: &[Option<u32>],
        deferred_none: Option<u32>,
    ) -> bool {
        let mut valid = true;
        self.visit_instructions(&mut |record| {
            if !crate::assembler::uses_constant_index(record.opcode) {
                return;
            }
            let Operand::Value(index) = record.operand() else {
                valid = false;
                return;
            };
            let Some(index) = (if index == u32::MAX {
                deferred_none
            } else {
                Some(index)
            }) else {
                valid = false;
                return;
            };
            let Some(Some(mapped)) = indices.get(index as usize) else {
                valid = false;
                return;
            };
            record.set_operand(Operand::Value(*mapped));
        });
        valid
    }

    pub(super) fn visit_instructions(&mut self, visit: &mut impl FnMut(&mut Instruction)) {
        for record in self.prefix.iter_mut().chain(&mut self.cleanup) {
            visit(record);
        }
        for record in [
            &mut self.inner_pop,
            &mut self.pop,
            &mut self.load,
            &mut self.condition,
            &mut self.marker,
        ] {
            visit(record);
        }
        for record in &mut self.incoming {
            visit(record);
        }
        for (_, _, _, items) in &mut self.setups {
            for item in items {
                if let Item::Instruction(record) = item {
                    visit(record);
                }
            }
        }
    }

    pub(in crate::assembler) fn resolve_priorities(
        &mut self,
        positions: &rustc_hash::FxHashMap<Label, usize>,
    ) {
        let Some(regions) = &mut self.regions else {
            return;
        };
        for region in regions {
            if let ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
                let (Some(start), Some(end)) = (positions.get(&start), positions.get(&end)) else {
                    self.regions = None;
                    return;
                };
                region.priority = ExceptionRegionPriority::Resolved {
                    start: *start,
                    end: *end,
                };
            }
        }
    }
}

fn plain(record: Instruction) -> bool {
    let Some(depth) = record.depth_after() else {
        return false;
    };
    matches!(record.operand(), Operand::Value(_))
        && record
            == Instruction::new(
                record.opcode,
                record.operand(),
                record.location,
                Some(depth),
            )
}

fn null_pop(record: Instruction, depth: u32) -> bool {
    let mut expected = Instruction::new(NOP, Operand::Value(0), record.location, Some(depth));
    expected.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    expected.fusion_policy.mark_converted_pop_block();
    record == expected
}

fn alias_head(items: &[Item], at: usize) -> Vec<Label> {
    let mut start = at;
    while start > 0 && matches!(items[start - 1], Item::Label(_)) {
        start -= 1;
    }
    items[start..]
        .iter()
        .map_while(|item| match item {
            Item::Label(label) => Some(*label),
            Item::Instruction(_) => None,
        })
        .collect()
}

fn same_original_region(
    actual: &ExceptionRegion,
    original: &ExceptionRegion,
    heads: &(Vec<Label>, Vec<Label>),
    items: &[Item],
    positions: &rustc_hash::FxHashMap<Label, usize>,
) -> bool {
    let mut shape = *actual;
    shape.start = original.start;
    shape.end = original.end;
    same_manager_region(&shape, original)
        && [
            (actual.start, original.start, &heads.0),
            (actual.end, original.end, &heads.1),
        ]
        .iter()
        .all(|(actual, original, head)| {
            head.contains(actual)
                && head.contains(original)
                && positions
                    .get(original)
                    .is_some_and(|at| alias_head(items, *at) == **head)
        })
}

impl Assembler {
    /// Captures the emitted inner POP separately from the two omitted outer POPs.
    pub(crate) fn record_copied_nested_try_manager_return(
        &mut self,
        return_item: usize,
        pop_item: usize,
        ordinary: [(Label, Label, Label); 2],
        manager: Label,
        finalizer: Label,
    ) {
        if self.python_version != crate::PythonVersion::Py314 {
            return;
        }
        let Some(setups) = self.capture_ordinary_try_setup_origins(
            &ordinary.map(|(setup, _, target)| (setup, target)),
        ) else {
            return;
        };
        let [_, (_, _, Some(inner_location), _)] = setups.origins.as_slice() else {
            return;
        };
        let items = self.linear_items();
        let Some(before) = items.get(..return_item) else {
            return;
        };
        let Some((entry_item, entry)) =
            before
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, item)| match item {
                    Item::Label(label)
                        if self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                            && self
                                .exception_owner_provenance
                                .block_boundaries
                                .contains(label) =>
                    {
                        Some((index, *label))
                    }
                    _ => None,
                })
        else {
            return;
        };
        let Some(original) = items.get(entry_item..pop_item) else {
            return;
        };
        let records = original
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record) => Some(*record),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let Some(split) = records.len().checked_sub(8) else {
            return;
        };
        let (prefix, unwind) = records.split_at(split);
        let [inner_pop, cleanup @ ..] = unwind else {
            return;
        };
        let Ok(cleanup): Result<[Instruction; 7], _> = cleanup.try_into() else {
            return;
        };
        let Some((authored, expression)) = prefix.split_first() else {
            return;
        };
        let mut expected_nop = Instruction::new(NOP, Operand::Value(0), *inner_location, Some(2));
        expected_nop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        if *authored != expected_nop || expression.is_empty()
            || expression.iter().any(|record| !plain(*record)
                || !matches!(record.opcode, LOAD_GLOBAL | LOAD_FAST | LOAD_DEREF | LOAD_ATTR | LOAD_CONST | PUSH_NULL | CALL | CALL_KW)
                || matches!((record.opcode, record.operand()), (LOAD_GLOBAL, Operand::Value(arg)) if arg & 1 != 0))
            || expression.last().and_then(Instruction::depth_after) != Some(3)
            || !null_pop(*inner_pop, 3)
            || inner_pop.location.line < 0
            || cleanup.iter().map(|record| record.opcode).ne([SWAP, SWAP, LOAD_CONST, LOAD_CONST, LOAD_CONST, CALL, POP_TOP])
            || cleanup.iter().map(Instruction::depth_after).ne([3, 3, 4, 5, 6, 2, 1].map(Some))
            || cleanup[0].operand() != Operand::Value(3) || cleanup[1].operand() != Operand::Value(2)
            || cleanup[2].operand() != cleanup[3].operand() || cleanup[3].operand() != cleanup[4].operand()
            || cleanup[5].operand() != Operand::Value(3) || cleanup[6].operand() != Operand::Value(0)
            || cleanup.iter().any(|record| !plain(*record) || record.location != cleanup[0].location)
        { return; }
        let mut suffix = items[pop_item..].iter().filter_map(|item| match item {
            Item::Instruction(record) => Some(*record),
            Item::Label(_) => None,
        });
        let (Some(pop), Some(load), Some(condition), Some(marker)) =
            (suffix.next(), suffix.next(), suffix.next(), suffix.next())
        else {
            return;
        };
        let mut expected_condition = Instruction::new(
            condition.opcode,
            condition.operand(),
            condition.location,
            Some(1),
        );
        expected_condition
            .local_policy
            .add_hidden_exception_owner_slots(3);
        let mut expected_marker =
            Instruction::new(NOT_TAKEN, Operand::Value(0), condition.location, Some(1));
        expected_marker.set_exception_policy(ExceptionPolicy::NonePresent);
        if !null_pop(pop, 1)
            || pop.location != SourceLocation::NONE
            || !matches!(load.opcode, LOAD_FAST | LOAD_DEREF | LOAD_GLOBAL)
            || !plain(load)
            || load.depth_after() != Some(2)
            || matches!((load.opcode, load.operand()), (LOAD_GLOBAL, Operand::Value(arg)) if arg & 1 != 0)
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition != expected_condition
            || marker != expected_marker
        {
            return;
        }
        let targets = ordinary
            .iter()
            .map(|(_, _, target)| *target)
            .chain([manager, finalizer])
            .collect::<FxHashSet<_>>();
        if targets.len() != 4
            || original
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .any(|label| {
                    label != entry
                        && (self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(&label)
                            || self
                                .exception_owner_provenance
                                .block_boundaries
                                .contains(&label))
                })
        {
            return;
        }
        self.exception_owner_provenance
            .copied_nested_try_manager_returns
            .push(CopiedNestedTryManagerReturn {
                entry,
                entry_head: Vec::new(),
                ordinary,
                manager,
                finalizer,
                setups: setups.origins,
                incoming: Vec::new(),
                regions: None,
                region_heads: None,
                original_fallbacks: Vec::new(),
                prefix: prefix.to_vec(),
                inner_pop: *inner_pop,
                cleanup,
                pop,
                load,
                condition,
                marker,
            });
    }

    /// Binds the real ordinary and `StopIteration` owners after all source regions exist.
    pub(in crate::assembler) fn bind_copied_nested_try_manager_returns(&mut self) {
        let mut origins = std::mem::take(
            &mut self
                .exception_owner_provenance
                .copied_nested_try_manager_returns,
        );
        let items = self.linear_items();
        let Some(item_positions) = unique_label_positions(items) else {
            return;
        };
        let mut positions = OwnerLabelPositions::new(self.next_label as usize);
        let mut index = 0;
        for item in items {
            match item {
                Item::Label(label) => {
                    positions.insert(*label, index);
                }
                Item::Instruction(_) => {
                    index += 1;
                }
            }
        }
        let original_regions = &self.exception_regions;
        if original_regions.iter().any(|region| [region.start, region.end, region.target].iter().any(|label| positions.get(label).is_none())
            || matches!(region.priority, ExceptionRegionPriority::OriginalRange(start, end) if positions.get(&start).is_none() || positions.get(&end).is_none()))
        { return; }
        origins.retain_mut(|origin| {
            if origin.regions.is_some()
                || self.capture_ordinary_try_setup_origins(&origin.ordinary.map(|(setup, _, target)| (setup, target)))
                    .is_none_or(|setups| setups.origins != origin.setups)
            { return false; }
            let Some(start) = positions.get(&origin.entry).copied() else { return false; };
            let cleanup = start + origin.prefix.len() + 1;
            let marker = cleanup + 10;
            let owners = [(start + 1, cleanup - 1), (cleanup, cleanup + 7), (marker, marker + 1)]
                .map(|(begin, end)| LayoutAssembler::uniform_exception_owner_in(original_regions, begin, end, &positions));
            let [Some(inner), Some(outer), Some(fallback)] = owners else { return false; };
            let regions = [original_regions[inner], original_regions[outer], original_regions[fallback]];
            if regions[0].target != origin.ordinary[1].2 || regions[0].depth != 2 || regions[0].is_scope_fallback || regions[0].preserve_lasti
                || regions[1].target != origin.ordinary[0].2 || regions[1].depth != 0 || regions[1].is_scope_fallback || regions[1].preserve_lasti
                || !regions[2].is_scope_fallback || regions[2].depth != 0 || !regions[2].preserve_lasti
                || origin.original_fallbacks.iter().filter(|original| same_manager_region(original, &regions[2])).count() != 1
            { return false; }
            let entry_item = item_positions[&origin.entry];
            let aliases = alias_head(items, entry_item);
            origin.incoming = items.iter().filter_map(|item| match item {
                Item::Instruction(record) if matches!(record.operand(), Operand::Forward(label) | Operand::Backward(label) if aliases.contains(&label)) => Some(*record),
                _ => None,
            }).collect();
            origin.entry_head = aliases;
            origin.region_heads = Some(regions.map(|region| (
                alias_head(items, item_positions[&region.start]),
                alias_head(items, item_positions[&region.end]),
            )));
            origin.regions = Some(regions);
            true
        });
        self.exception_owner_provenance
            .copied_nested_try_manager_returns = origins;
    }
}

impl LayoutAssembler {
    pub(super) fn copied_nested_try_manager_null_owner(
        &self,
        marker: usize,
        array_start: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> bool {
        if self.python_version != crate::PythonVersion::Py314
            || tail.early_removals != 5
            || tail.insertions != 0
            || tail.late_removals != 0
            || tail.post_compaction_fusions != 0
        {
            return false;
        }
        let provenance = &self.exception_owner_provenance;
        let matches = provenance.copied_nested_try_manager_returns.iter().filter(|origin| {
            let Some(cleanup_start) = array_start.checked_add(origin.prefix.len()) else { return false; };
            let Some(original_regions) = &origin.regions else { return false; };
            let Some(region_heads) = &origin.region_heads else { return false; };
            let Some(item_positions) = unique_label_positions(&self.items) else { return false; };
            let Some(entry_item) = item_positions.get(&origin.entry).copied() else { return false; };
            let aliases = alias_head(&self.items, entry_item);
            let incoming = self.items.iter().filter_map(|item| match item {
                Item::Instruction(record) if matches!(record.operand(), Operand::Forward(label) | Operand::Backward(label) if aliases.contains(&label)) => Some(*record),
                _ => None,
            }).collect::<Vec<_>>();
            if marker != cleanup_start + 9 || positions.get(&origin.entry) != Some(&array_start)
                || aliases != origin.entry_head || incoming != origin.incoming || !null_pop(origin.inner_pop, 3) || !null_pop(origin.pop, 1)
                || origin.setups.iter().any(|(setup, target, location, head)| {
                    anchored_start(&self.items, head).is_none()
                        || provenance.setups.iter().filter(|(label, saved)| label == setup && saved == location).count() != 1
                        || provenance.ordinary_try_setups.get(setup) != Some(target)
                })
                || !provenance.source_boundaries.contains(&origin.entry) || !provenance.block_boundaries.contains(&origin.entry)
                || provenance.normal_try_copy_blocks.contains(&origin.entry) || provenance.normal_try_copy_internal_labels.contains(&origin.entry)
                || provenance.retired_normal_try_copy_labels.contains(&origin.entry)
                || origin.ordinary.iter().any(|(setup, end, target)| {
                    provenance.ordinary_try_setups.get(setup) != Some(target)
                        || [setup, end, target].iter().any(|label| positions.get(label).is_none())
                })
                || [origin.manager, origin.finalizer].iter().any(|label| positions.get(label).is_none())
                || jump_targets.iter().chain(&provenance.source_boundaries).chain(&provenance.block_boundaries)
                    .any(|label| positions.get(label).is_some_and(|position| array_start < *position && *position <= marker))
                || self.items.iter().filter(|item| **item == Item::Label(origin.entry)).count() != 1
                || *instructions[marker] != origin.marker || *instructions[marker - 1] != origin.condition
                || !instructions[array_start..cleanup_start].iter().zip(&origin.prefix).all(|(actual, expected)| {
                    let mut expected = *expected;
                    if expected.opcode == LOAD_FAST && actual.opcode == LOAD_FAST_BORROW { expected.opcode = LOAD_FAST_BORROW; }
                    **actual == expected
                })
                || !instructions[cleanup_start..cleanup_start + 7].iter().zip(origin.cleanup).enumerate().all(|(offset, (actual, mut expected))| {
                    if offset == 0 { expected.local_policy.add_hidden_exception_owner_slots(1); }
                    **actual == expected
                })
            { return false; }
            let mut load = origin.load;
            load.local_policy.add_hidden_exception_owner_slots(1);
            if load.opcode == LOAD_FAST && instructions[marker - 2].opcode == LOAD_FAST_BORROW { load.opcode = LOAD_FAST_BORROW; }
            if *instructions[marker - 2] != load { return false; }
            let Some(inner) = self.uniform_exception_owner(array_start + 1, cleanup_start, positions) else { return false; };
            let Some(outer) = self.uniform_exception_owner(cleanup_start, cleanup_start + 7, positions) else { return false; };
            let inner = self.exception_regions[inner];
            let outer = self.exception_regions[outer];
            same_original_region(&inner, &original_regions[0], &region_heads[0], &self.items, &item_positions)
                && same_original_region(&outer, &original_regions[1], &region_heads[1], &self.items, &item_positions)
                && self.uniform_exception_owner(marker, marker + 1, positions).is_some_and(|index| {
                    same_original_region(&self.exception_regions[index], &original_regions[2], &region_heads[2], &self.items, &item_positions)
                })
        }).count();
        // The original inner POP and omitted manager POP precede the seven exits;
        // the omitted outer ordinary POP follows them. The append reuses that NULL cell.
        matches == 1
    }
}
