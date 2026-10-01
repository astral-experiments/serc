//! Original NULL cells in an unnamed handler's direct BREAK finalizer.

#[cfg(test)]
mod tests;

use super::{ExceptionOwnerTail, OwnerLabelPositions};
use crate::assembler::block_graph::{BasicBlockId, BlockGraph};
use crate::assembler::{
    Assembler, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    LayoutAssembler, Operand, SourceLocation,
};
use crate::target::opcodes::{
    LOAD_DEREF, LOAD_FAST, LOAD_FAST_BORROW, LOAD_GLOBAL, NOP, NOT_TAKEN, POP_EXCEPT,
    POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE,
};
use rustc_hash::{FxHashMap, FxHashSet};

fn same_region(a: ExceptionRegion, b: ExceptionRegion) -> bool {
    let priority = match (a.priority, b.priority) {
        (
            ExceptionRegionPriority::OriginalRange(a, b),
            ExceptionRegionPriority::OriginalRange(c, d),
        ) => a == c && b == d,
        (
            ExceptionRegionPriority::Resolved { start: a, end: b },
            ExceptionRegionPriority::Resolved { start: c, end: d },
        ) => a == c && b == d,
        _ => false,
    };
    a.start == b.start
        && a.end == b.end
        && a.target == b.target
        && priority
        && a.depth == b.depth
        && a.preserve_lasti == b.preserve_lasti
        && a.is_scope_fallback == b.is_scope_fallback
}

#[derive(Debug)]
pub(in crate::assembler) struct UnnamedHandlerBreak {
    entry: Label,
    identities: [Label; 5],
    original_aliases: Vec<(Label, usize)>,
    aliases: Vec<(Label, usize)>,
    incoming: Vec<Instruction>,
    records: [Instruction; 5],
    fallback_producers: Vec<ExceptionRegion>,
    owners: Vec<ExceptionRegion>,
    condition_transported: bool,
    cold_table: Option<Vec<ExceptionRegion>>,
    marker_head: Option<Label>,
}

impl UnnamedHandlerBreak {
    pub(super) fn visit_instructions(&mut self, visit: &mut impl FnMut(&mut Instruction)) {
        for record in self.records.iter_mut().chain(&mut self.incoming) {
            visit(record);
        }
    }

    pub(in crate::assembler) fn record_fallback(&mut self, region: ExceptionRegion) {
        self.fallback_producers.push(region);
    }

    fn matches_records(&self, records: &[&Instruction]) -> bool {
        let [pop, load, condition, marker] = records else {
            return false;
        };
        let mut expected_pop = self.records[1];
        expected_pop
            .local_policy
            .add_hidden_exception_owner_slots(1);
        let mut expected_load = self.records[2];
        if expected_load.opcode == LOAD_FAST && load.opcode == LOAD_FAST_BORROW {
            expected_load.opcode = LOAD_FAST_BORROW;
        }
        **pop == expected_pop
            && **load == expected_load
            && **condition == self.records[3]
            && **marker == self.records[4]
    }

    /// Receipt issued before the exit-copy writer labels this original marker block.
    pub(in crate::assembler) fn prepend_marker_head(
        &mut self,
        graph: &BlockGraph,
        block: BasicBlockId,
        fresh: Label,
    ) {
        if self.marker_head.is_some()
            || self.cold_table.is_some()
            || graph[block].first_label().is_some()
            || graph.label_block(fresh).is_some()
        {
            return;
        }
        let items = graph.copy_items();
        if items.iter().any(|item| matches!(item, Item::Instruction(record) if matches!(record.operand(), Operand::Forward(label) | Operand::Backward(label) if label == fresh))) { return; }
        if projected_aliases(&items, self.entry, 5, 0).as_ref() != Some(&self.original_aliases)
            || incoming(
                &items,
                &self.aliases.iter().map(|(label, _)| *label).collect(),
            ) != self.incoming
        {
            return;
        }
        let Some(head) = alias_head(&items, self.entry) else {
            return;
        };
        let records = items[head..]
            .iter()
            .enumerate()
            .filter_map(|(at, item)| match item {
                Item::Instruction(record) => Some((head + at, record)),
                Item::Label(_) => None,
            })
            .take(5)
            .collect::<Vec<_>>();
        if records.len() != 5
            || !records
                .iter()
                .zip(&self.records)
                .all(|((_, current), original)| *current == original)
        {
            return;
        }
        let Some(position) = graph
            .order()
            .iter()
            .position(|candidate| *candidate == block)
        else {
            return;
        };
        let preceding = graph.order()[..position]
            .iter()
            .map(|block| graph[*block].items().len())
            .sum::<usize>();
        if preceding != records[4].0
            || graph[block].items().first() != Some(&Item::Instruction(self.records[4]))
        {
            return;
        }
        self.aliases.push((fresh, 3));
        self.marker_head = Some(fresh);
    }

    pub(in crate::assembler) fn retarget_condition(
        &mut self,
        receipts: &[(Instruction, Instruction)],
        items: &[Item],
    ) -> bool {
        let pairs = receipts
            .iter()
            .filter(|(before, _)| *before == self.records[3])
            .collect::<Vec<_>>();
        let [(before, after)] = pairs.as_slice() else {
            return pairs.is_empty();
        };
        if self.condition_transported || !matches!(after.operand(), Operand::Forward(_)) {
            return false;
        }
        let mut expected = *before;
        expected.retarget(after.operand());
        if expected != *after
            || items
                .iter()
                .filter(|item| matches!(item, Item::Instruction(record) if record == after))
                .count()
                != 1
        {
            return false;
        }
        self.records[3] = *after;
        self.condition_transported = true;
        true
    }

    pub(in crate::assembler) fn resolve_priorities(
        &mut self,
        positions: &FxHashMap<Label, usize>,
    ) -> bool {
        for region in &mut self.owners {
            if let ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
                let (Some(&start), Some(&end)) = (positions.get(&start), positions.get(&end))
                else {
                    return false;
                };
                region.priority = ExceptionRegionPriority::Resolved { start, end };
            }
        }
        self.owners.len() == 2
    }

    pub(in crate::assembler) fn rebind_cold(
        &mut self,
        before: &[ExceptionRegion],
        retargets: &[(ExceptionRegion, ExceptionRegion)],
        current: &[ExceptionRegion],
    ) -> bool {
        if self.cold_table.is_some()
            || self.owners.len() != 2
            || retargets.len() != current.len()
            || !retargets
                .iter()
                .zip(current)
                .all(|((_, rebuilt), actual)| same_region(*rebuilt, *actual))
        {
            return false;
        }
        let mut last_source = None;
        for (source, rebuilt) in retargets {
            let indices = before
                .iter()
                .enumerate()
                .filter_map(|(index, region)| same_region(*region, *source).then_some(index))
                .collect::<Vec<_>>();
            let [index] = indices.as_slice() else {
                return false;
            };
            if last_source.is_some_and(|last| last > *index) {
                return false;
            }
            last_source = Some(*index);
            let mut shape = *rebuilt;
            shape.start = source.start;
            shape.end = source.end;
            if !same_region(shape, *source) {
                return false;
            }
        }
        let mut owners = Vec::new();
        for original in &self.owners {
            let replacements = retargets
                .iter()
                .filter(|(source, _)| same_region(*source, *original));
            let mut found = false;
            for (_, rebuilt) in replacements {
                let mut shape = *rebuilt;
                shape.start = original.start;
                shape.end = original.end;
                if !same_region(shape, *original) {
                    return false;
                }
                owners.push(*rebuilt);
                found = true;
            }
            if !found {
                return false;
            }
        }
        self.owners = owners;
        self.cold_table = Some(current.to_vec());
        true
    }
}

fn incoming(items: &[Item], labels: &FxHashSet<Label>) -> Vec<Instruction> {
    items.iter().filter_map(|item| match item {
        Item::Instruction(record) if matches!(record.operand(), Operand::Forward(label) | Operand::Backward(label) if labels.contains(&label)) => Some(*record),
        _ => None,
    }).collect()
}

fn alias_head(items: &[Item], entry: Label) -> Option<usize> {
    let entry = items.iter().position(|item| *item == Item::Label(entry))?;
    Some(
        items[..entry]
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .map_or(0, |at| at + 1),
    )
}

fn live_aliases(items: &[Item], entry: Label) -> Option<Vec<(Label, usize)>> {
    projected_aliases(items, entry, 4, 0)
}

fn projected_aliases(
    items: &[Item],
    entry: Label,
    count: usize,
    removed: usize,
) -> Option<Vec<(Label, usize)>> {
    let mut aliases = Vec::new();
    let mut records: usize = 0;
    for item in &items[alias_head(items, entry)?..] {
        match item {
            Item::Label(label) => aliases.push((*label, records.saturating_sub(removed))),
            Item::Instruction(_) => records += 1,
        }
        if records == count {
            return Some(aliases);
        }
    }
    None
}

impl Assembler {
    pub(crate) fn record_unnamed_handler_break_none(
        &mut self,
        start: usize,
        identities: [Label; 5],
        load_location: SourceLocation,
        condition_location: SourceLocation,
    ) {
        if self.python_version != crate::PythonVersion::Py314 {
            return;
        }
        let items = self.linear_items();
        let Some((entry_item, entry)) = items.get(..start).and_then(|prefix| {
            prefix
                .iter()
                .enumerate()
                .rev()
                .find_map(|(at, item)| match item {
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
                        Some((at, *label))
                    }
                    _ => None,
                })
        }) else {
            return;
        };
        if items[entry_item..start]
            .iter()
            .any(|item| matches!(item, Item::Instruction(_)))
        {
            return;
        }
        let mut records = Vec::new();
        let mut aliases = Vec::new();
        let mut original_aliases = Vec::new();
        let Some(head) = alias_head(items, entry) else {
            return;
        };
        for item in &items[head..] {
            match item {
                Item::Label(label) => {
                    original_aliases.push((*label, records.len()));
                    aliases.push((*label, records.len().saturating_sub(1)));
                }
                Item::Instruction(record) => records.push(*record),
            }
            if records.len() == 5 {
                break;
            }
        }
        let Ok(records): Result<[Instruction; 5], _> = records.try_into() else {
            return;
        };
        let [authored, pop, load, condition, marker] = records;
        let mut expected_nop = Instruction::new(NOP, Operand::Value(0), authored.location, Some(1));
        expected_nop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        let expected_pop =
            Instruction::new(POP_EXCEPT, Operand::Value(0), authored.location, Some(0));
        let mut expected_load =
            Instruction::new(load.opcode, load.operand(), load_location, Some(1));
        if load.opcode == LOAD_FAST {
            expected_load.local_policy = crate::assembler::ir::LocalPolicy::for_load(
                load.local_policy.force_owned_load(),
                false,
            );
        }
        let mut expected_condition = Instruction::new(
            condition.opcode,
            condition.operand(),
            condition_location,
            Some(0),
        );
        expected_condition
            .local_policy
            .add_hidden_exception_owner_slots(3);
        let mut expected_marker =
            Instruction::new(NOT_TAKEN, Operand::Value(0), condition_location, Some(0));
        expected_marker.set_exception_policy(ExceptionPolicy::NonePresent);
        if authored != expected_nop
            || authored.location.line < 0
            || pop != expected_pop
            || load != expected_load
            || !matches!(load.opcode, LOAD_FAST | LOAD_GLOBAL | LOAD_DEREF)
            || matches!((load.opcode, load.operand()), (LOAD_GLOBAL, Operand::Value(arg)) if arg & 1 != 0)
            || !matches!(load.operand(), Operand::Value(_))
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition != expected_condition
            || marker != expected_marker
            || aliases.iter().any(|(label, offset)| {
                *offset > 0
                    && (self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(label)
                        || self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label))
            })
            || aliases
                .iter()
                .map(|(label, _)| *label)
                .collect::<FxHashSet<_>>()
                .len()
                != aliases.len()
            || aliases.iter().any(|(label, _)| {
                self.exception_owner_provenance
                    .normal_try_copy_blocks
                    .contains(label)
                    || self
                        .exception_owner_provenance
                        .retired_normal_try_copy_labels
                        .contains(label)
                    || self
                        .exception_owner_provenance
                        .normal_try_copy_internal_labels
                        .contains(label)
            })
            || identities[2] != identities[3]
            || identities.iter().collect::<FxHashSet<_>>().len() != 4
        {
            return;
        }
        let labels = aliases.iter().map(|(label, _)| *label).collect();
        let incoming = incoming(items, &labels);
        self.exception_owner_provenance
            .unnamed_handler_breaks
            .push(UnnamedHandlerBreak {
                entry,
                identities,
                original_aliases,
                aliases,
                incoming,
                records,
                fallback_producers: Vec::new(),
                owners: Vec::new(),
                condition_transported: false,
                cold_table: None,
                marker_head: None,
            });
    }

    pub(in crate::assembler) fn bind_unnamed_handler_break_owners(&mut self) {
        let crate::assembler::UnoptimizedCode::Linear(items) = &self.code else {
            return;
        };
        let mut positions = FxHashMap::default();
        let mut index = 0;
        for item in items {
            match item {
                Item::Label(label) => {
                    if positions.insert(*label, index).is_some() {
                        return;
                    }
                }
                Item::Instruction(_) => index += 1,
            }
        }
        self.exception_owner_provenance
            .unnamed_handler_breaks
            .retain_mut(|origin| {
                let Some(&start) = positions.get(&origin.entry) else {
                    return false;
                };
                if incoming(
                    items,
                    &origin.aliases.iter().map(|(label, _)| *label).collect(),
                ) != origin.incoming
                {
                    return false;
                }
                let covers = |region: &ExceptionRegion, at| {
                    positions
                        .get(&region.start)
                        .zip(positions.get(&region.end))
                        .is_some_and(|(a, b)| *a <= at && at < *b)
                };
                let cleanup = self
                    .exception_regions
                    .iter()
                    .filter(|region| {
                        !region.is_scope_fallback
                            && !region.preserve_lasti
                            && region.depth == 0
                            && region.target == origin.identities[4]
                            && covers(region, start + 1)
                    })
                    .copied()
                    .collect::<Vec<_>>();
                let fallback = origin
                    .fallback_producers
                    .iter()
                    .filter(|region| {
                        region.is_scope_fallback
                            && region.preserve_lasti
                            && region.depth == 0
                            && covers(region, start + 2)
                            && covers(region, start + 4)
                    })
                    .copied()
                    .collect::<Vec<_>>();
                let ([cleanup], [fallback]) = (cleanup.as_slice(), fallback.as_slice()) else {
                    return false;
                };
                origin.owners = vec![*cleanup, *fallback];
                true
            });
    }
}

impl LayoutAssembler {
    pub(super) fn unnamed_handler_break_null_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> bool {
        if self.python_version != crate::PythonVersion::Py314
            || marker != array + 3
            || tail.early_removals != 4
            || tail.insertions != 0
            || tail.late_removals != 0
            || tail.post_compaction_fusions != 0
        {
            return false;
        }
        let provenance = &self.exception_owner_provenance;
        provenance
            .unnamed_handler_breaks
            .iter()
            .filter(|origin| {
                let Some(table) = &origin.cold_table else {
                    return false;
                };
                let Operand::Forward(condition_target) = origin.records[3].operand() else {
                    return false;
                };
                if positions.get(&origin.entry) != Some(&array)
                    || live_aliases(&self.items, origin.entry).as_ref() != Some(&origin.aliases)
                    || positions
                        .get(&condition_target)
                        .is_none_or(|at| array <= *at && *at <= marker)
                    || table.len() != self.exception_regions.len()
                    || !table
                        .iter()
                        .zip(&self.exception_regions)
                        .all(|(before, after)| same_region(*before, *after))
                    || !provenance.source_boundaries.contains(&origin.entry)
                    || !provenance.block_boundaries.contains(&origin.entry)
                    || origin.identities.iter().any(|label| {
                        positions.get(label).is_none()
                            || self
                                .items
                                .iter()
                                .filter(|item| **item == Item::Label(*label))
                                .count()
                                != 1
                    })
                    || origin.aliases.iter().any(|(label, offset)| {
                        positions.get(label) != Some(&(array + offset))
                            || self
                                .items
                                .iter()
                                .filter(|item| **item == Item::Label(*label))
                                .count()
                                != 1
                            || provenance.normal_try_copy_blocks.contains(label)
                            || provenance.retired_normal_try_copy_labels.contains(label)
                            || provenance.normal_try_copy_internal_labels.contains(label)
                    })
                    || jump_targets
                        .iter()
                        .chain(&provenance.source_boundaries)
                        .chain(&provenance.block_boundaries)
                        .any(|label| {
                            positions
                                .get(label)
                                .is_some_and(|at| array < *at && *at <= marker)
                        })
                    || incoming(
                        &self.items,
                        &origin.aliases.iter().map(|(label, _)| *label).collect(),
                    ) != origin.incoming
                {
                    return false;
                }
                if !origin.matches_records(&instructions[array..=marker]) {
                    return false;
                }
                let (Some(cleanup), Some(fallback)) = (
                    self.uniform_exception_owner(array, array + 1, positions),
                    self.uniform_exception_owner(array + 1, marker + 1, positions),
                ) else {
                    return false;
                };
                let cleanup = self.exception_regions[cleanup];
                let fallback = self.exception_regions[fallback];
                !cleanup.is_scope_fallback
                    && !cleanup.preserve_lasti
                    && cleanup.depth == 0
                    && cleanup.target == origin.identities[4]
                    && fallback.is_scope_fallback
                    && fallback.preserve_lasti
                    && fallback.depth == 0
                    && [cleanup, fallback].iter().all(|region| {
                        origin
                            .owners
                            .iter()
                            .any(|expected| same_region(*expected, *region))
                            && self
                                .exception_regions
                                .iter()
                                .filter(|other| same_region(**other, *region))
                                .count()
                                == 1
                    })
            })
            .count()
            == 1
    }
}
