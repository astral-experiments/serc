//! The original POP cells between completed manager exits and a copied None test.

#[cfg(test)]
mod tests;

use crate::target::operands::ResumeLocation;
use rustc_hash::{FxHashMap, FxHashSet};

use super::block_graph::{BasicBlockId, BlockGraph, ItemPosition};
use super::fusion_owner_slots::same_manager_region;
use super::ir::EmissionStreamId;
use super::normal_finally_locations::{anchored_start, same_priority, unique_label_positions};
use super::{
    Assembler, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    LayoutAssembler, Operand, OptimizationPipeline, SourceLocation, UnoptimizedCode,
    uses_constant_index,
};
use crate::target::opcodes::{
    CALL, CLEANUP_THROW, END_SEND, GET_AWAITABLE, JUMP_BACKWARD_NO_INTERRUPT, LOAD_CONST,
    LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_NONE, POP_TOP, RESUME, SEND, SWAP, YIELD_VALUE,
};

/// An indexed receipt issued by the real preserved-value manager-unwind emitter.
#[derive(Debug)]
pub(crate) struct CopiedFinallyManagerEvent {
    stream: EmissionStreamId,
    start: usize,
    items: Vec<Item>,
    none: u32,
    cleanup: Option<ExceptionRegion>,
    bridge: Option<(Label, Instruction)>,
    entry_head: Vec<Item>,
}

impl CopiedFinallyManagerEvent {
    pub(crate) fn starts_at(&self, entry: Label) -> bool {
        self.items.first() == Some(&Item::Label(entry))
    }

    fn anchored_items(&self, end: usize) -> Vec<Item> {
        self.entry_head
            .iter()
            .chain(&self.items[1..end])
            .copied()
            .collect()
    }
}

/// Original records remain separate from the final array after its real POP is removed.
#[derive(Debug)]
pub(super) struct CopiedAsyncFinallyOwner {
    stream: EmissionStreamId,
    events: Vec<CopiedFinallyManagerEvent>,
    original: Vec<Item>,
    surviving: Vec<Item>,
    original_start: usize,
    setup: (Label, Label),
    setup_head: Vec<Item>,
    ordinary: (Label, Label, Label),
    regions: Option<Vec<ExceptionRegion>>,
    target: Label,
    target_head: Vec<Item>,
    pop_removed: bool,
    pop_offset: usize,
    marker_label: Option<Label>,
    graph_heads: Vec<(Label, BasicBlockId)>,
}

fn label(item: &Item) -> Option<Label> {
    match item {
        Item::Label(label) => Some(*label),
        Item::Instruction(_) => None,
    }
}

fn record(item: &Item) -> Option<Instruction> {
    match item {
        Item::Instruction(instruction) => Some(*instruction),
        Item::Label(_) => None,
    }
}

fn head(items: &[Item], position: usize) -> Option<Vec<Item>> {
    let mut start = position;
    while start > 0 && matches!(items[start - 1], Item::Label(_)) {
        start -= 1;
    }
    let mut end = start;
    while matches!(items.get(end), Some(Item::Label(_))) {
        end += 1;
    }
    (start < end).then(|| items[start..end].to_vec())
}

fn complete_head_start(items: &[Item], expected: &[Item]) -> Option<usize> {
    let start = anchored_start(items, expected)?;
    (head(items, start)?.as_slice() == expected).then_some(start)
}

fn graph_items(graph: &BlockGraph) -> Vec<Item> {
    graph
        .iter()
        .flat_map(|(_, block)| block.items())
        .copied()
        .collect()
}

fn same_regions(left: &[ExceptionRegion], right: &[ExceptionRegion]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_manager_region(left, right))
}

fn bound_regions(items: &[Item], regions: &[ExceptionRegion]) -> bool {
    unique_label_positions(items).is_some_and(|positions| {
        regions.iter().all(|region| {
            [region.start, region.end, region.target]
                .iter()
                .all(|label| positions.contains_key(label))
        })
    })
}

impl CopiedAsyncFinallyOwner {
    fn records_unchanged(&self, items: &[Item], cold: bool) -> bool {
        if unique_label_positions(items).is_none()
            || anchored_start(
                items,
                if self.pop_removed {
                    &self.surviving
                } else {
                    &self.original
                },
            )
            .is_none()
            || complete_head_start(items, &self.setup_head).is_none()
            || complete_head_start(items, &self.target_head).is_none()
        {
            return false;
        }
        self.events.iter().all(|event| {
            if !cold || event.cleanup.is_none() {
                return event.bridge.is_none()
                    && anchored_start(items, &event.anchored_items(event.items.len())).is_some();
            }
            let Some((bridge, instruction)) = event.bridge else {
                return false;
            };
            // These are the emitter's six original arrays, not arbitrary matching records.
            anchored_start(items, &event.anchored_items(9)).is_some()
                && [9..11, 11..13, 13..16, 18..21]
                    .into_iter()
                    .all(|range| anchored_start(items, &event.items[range]).is_some())
                && anchored_start(
                    items,
                    &[
                        event.items[16],
                        event.items[17],
                        Item::Label(bridge),
                        Item::Instruction(instruction),
                    ],
                )
                .is_some()
        })
    }

    fn graph_unchanged(&self, graph: &BlockGraph, regions: &[ExceptionRegion], cold: bool) -> bool {
        let items = graph_items(graph);
        if !bound_regions(&items, regions)
            || !self.records_unchanged(&items, cold)
            || self
                .graph_heads
                .iter()
                .any(|(label, block)| graph.label_block(*label) != Some(*block))
        {
            return false;
        }
        let reachable = graph.reachable_blocks(regions);
        self.events
            .iter()
            .flat_map(|event| event.items.iter().chain(&event.entry_head))
            .chain(&self.original)
            .chain(&self.setup_head)
            .chain(&self.target_head)
            .filter_map(label)
            .all(|label| {
                graph
                    .label_block(label)
                    .is_some_and(|block| reachable[block.index()])
            })
    }

    fn bind_graph_heads(&mut self, graph: &BlockGraph) -> bool {
        if self
            .graph_heads
            .iter()
            .any(|(label, block)| graph.label_block(*label) != Some(*block))
        {
            return false;
        }
        let mut seen = self
            .graph_heads
            .iter()
            .map(|(label, _)| *label)
            .collect::<FxHashSet<_>>();
        for label in self
            .events
            .iter()
            .flat_map(|event| event.items.iter().chain(&event.entry_head))
            .chain(&self.original)
            .chain(&self.setup_head)
            .chain(&self.target_head)
            .filter_map(label)
        {
            if seen.insert(label) {
                let Some(block) = graph.label_block(label) else {
                    return false;
                };
                self.graph_heads.push((label, block));
            }
        }
        true
    }

    /// Bind the fresh head only at the existing unlabeled synthetic marker writer.
    pub(super) fn prepend_marker_head(
        &mut self,
        graph: &BlockGraph,
        block: BasicBlockId,
        fresh: Label,
    ) {
        if self.marker_label.is_some() || graph[block].first_label().is_some() {
            return;
        }
        let items = graph_items(graph);
        if unique_label_positions(&items).is_none() {
            return;
        }
        let Some(start) = anchored_start(&items, &self.original) else {
            return;
        };
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
        if preceding != start + self.original.len() - 1
            || graph[block].items().first() != self.original.last()
            || graph.label_block(fresh).is_some()
        {
            return;
        }
        self.original
            .insert(self.original.len() - 1, Item::Label(fresh));
        self.surviving
            .insert(self.surviving.len() - 1, Item::Label(fresh));
        self.marker_label = Some(fresh);
        self.graph_heads.push((fresh, block));
    }

    /// Receipt for the real cold cleanup bridge, before its new block is inserted.
    pub(super) fn record_cold_bridge(
        &mut self,
        graph: &BlockGraph,
        source: BasicBlockId,
        target: BasicBlockId,
        fresh: Label,
        bridge: Instruction,
    ) {
        let items = graph_items(graph);
        if unique_label_positions(&items).is_none() || graph.label_block(fresh).is_some() {
            return;
        }
        for event in &mut self.events {
            if event.cleanup.is_none()
                || event.bridge.is_some()
                || graph[source].items() != &event.items[16..18]
                || !graph[target].items().starts_with(&event.items[18..21])
                || anchored_start(&items, &event.anchored_items(event.items.len())).is_none()
            {
                continue;
            }
            let Some(entry) = label(&event.items[18]) else {
                continue;
            };
            let Some(cleanup) = record(&event.items[17]) else {
                continue;
            };
            let expected = Instruction::synthetic(
                JUMP_BACKWARD_NO_INTERRUPT,
                Operand::Backward(entry),
                cleanup.location,
                None,
            );
            if bridge != expected {
                continue;
            }
            let mut incoming = items.iter().filter_map(record).filter(|instruction| {
                matches!(instruction.operand(), Operand::Forward(entry) | Operand::Backward(entry)
                    if graph.label_block(entry) == Some(target))
            });
            if incoming.next() != record(&event.items[10]) || incoming.next().is_some() {
                continue;
            }
            event.bridge = Some((fresh, bridge));
        }
    }
}

/// One invocation of the existing indexed threaded-owner writer.
pub(super) struct CopiedFinallyThreadRewrite {
    items: Vec<Item>,
    regions: Vec<ExceptionRegion>,
    pairs: Vec<(usize, usize)>,
    events: Vec<(usize, usize, Label, Label)>,
}

impl CopiedFinallyThreadRewrite {
    pub(super) fn capture(
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        origins: &mut Vec<CopiedAsyncFinallyOwner>,
        pairs: &[(usize, usize)],
    ) -> Option<Self> {
        if origins.is_empty() {
            return None;
        }
        let items = graph_items(graph);
        let instructions = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| record(item).map(|_| index))
            .collect::<Vec<_>>();
        origins.retain_mut(|origin| {
            if !origin
                .regions
                .as_ref()
                .is_some_and(|old| same_regions(old, regions))
                || !origin.graph_unchanged(graph, regions, false)
                || !origin.bind_graph_heads(graph)
            {
                return false;
            }
            let mut protected = Vec::new();
            let events = origin
                .events
                .iter()
                .map(|event| event.anchored_items(event.items.len()))
                .collect::<Vec<_>>();
            for span in events.iter().map(Vec::as_slice).chain([
                origin.original.as_slice(),
                origin.setup_head.as_slice(),
                origin.target_head.as_slice(),
            ]) {
                let Some(start) = anchored_start(&items, span) else {
                    return false;
                };
                protected.push(start..start + span.len());
            }
            pairs.iter().all(|(source, target)| {
                [source, target].iter().all(|index| {
                    instructions
                        .get(**index)
                        .is_some_and(|index| protected.iter().all(|range| !range.contains(index)))
                })
            })
        });
        (!origins.is_empty()).then(|| Self {
            items,
            regions: regions.to_vec(),
            pairs: pairs.to_vec(),
            events: Vec::new(),
        })
    }

    pub(super) fn emitted(&mut self, source: usize, target: usize, start: Label, end: Label) {
        self.events.push((source, target, start, end));
    }

    pub(super) fn finish(
        self,
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        lineage: &[usize],
        origins: &mut Vec<CopiedAsyncFinallyOwner>,
    ) {
        let valid = (|| {
            if self
                .events
                .iter()
                .map(|(source, target, _, _)| (*source, *target))
                .collect::<Vec<_>>()
                != self.pairs.iter().rev().copied().collect::<Vec<_>>()
            {
                return None;
            }
            let positions = unique_label_positions(&self.items)?;
            let instruction_items = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| record(item).map(|_| index))
                .collect::<Vec<_>>();
            let mut expected_items = self.items.clone();
            let mut fresh = FxHashSet::default();
            let mut last_source = usize::MAX;
            let mut exclusions = vec![Vec::new(); self.regions.len()];
            let mut copied = Vec::new();
            for &(source, target, start, end) in &self.events {
                if source >= last_source
                    || positions.contains_key(&start)
                    || positions.contains_key(&end)
                    || !fresh.insert(start)
                    || !fresh.insert(end)
                {
                    return None;
                }
                last_source = source;
                let source_item = *instruction_items.get(source)?;
                let target_item = *instruction_items.get(target)?;
                expected_items.insert(source_item + 1, Item::Label(end));
                expected_items.insert(source_item, Item::Label(start));
                for (index, region) in self.regions.iter().enumerate() {
                    let range = *positions.get(&region.start)?..*positions.get(&region.end)?;
                    if range.start > range.end {
                        return None;
                    }
                    match (range.contains(&source_item), range.contains(&target_item)) {
                        (true, false) => exclusions[index].push((start, end)),
                        (false, true) => copied.push((
                            index,
                            ExceptionRegion {
                                start,
                                end,
                                ..*region
                            },
                        )),
                        _ => {}
                    }
                }
            }
            if expected_items != graph_items(graph) {
                return None;
            }
            let mut expected = Vec::new();
            for (index, (region, exclusions)) in self.regions.iter().zip(exclusions).enumerate() {
                let mut start = region.start;
                for (excluded_start, excluded_end) in exclusions.into_iter().rev() {
                    expected.push((
                        index,
                        ExceptionRegion {
                            start,
                            end: excluded_start,
                            ..*region
                        },
                    ));
                    start = excluded_end;
                }
                expected.push((index, ExceptionRegion { start, ..*region }));
            }
            expected.extend(copied);
            if expected.len() != regions.len()
                || lineage.len() != regions.len()
                || !expected.iter().zip(regions).zip(lineage).all(
                    |(((index, expected), actual), emitted)| {
                        index == emitted && same_manager_region(expected, actual)
                    },
                )
            {
                return None;
            }
            Some(())
        })()
        .is_some();
        origins.retain_mut(|origin| {
            if !valid
                || !origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, &self.regions))
                || !origin.graph_unchanged(graph, regions, false)
            {
                return false;
            }
            origin.regions = Some(regions.to_vec());
            true
        });
    }
}

/// Indexed original-block membership for one cold-region reconstruction.
pub(super) struct CopiedFinallyColdRewrite {
    regions: Vec<ExceptionRegion>,
    order: Vec<BasicBlockId>,
    heads: FxHashMap<BasicBlockId, Label>,
    ranges: Vec<std::ops::Range<usize>>,
    bridges: Vec<(BasicBlockId, Label, Instruction)>,
    outputs: Vec<usize>,
    groups: Vec<(usize, usize)>,
}

/// Block IDs may change only while the existing normalizer preserves every Item.
pub(super) struct CopiedFinallyNormalization(Vec<Item>);

impl CopiedFinallyNormalization {
    pub(super) fn capture(
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        origins: &mut Vec<CopiedAsyncFinallyOwner>,
    ) -> Option<Self> {
        if origins.is_empty() {
            return None;
        }
        origins.retain_mut(|origin| {
            origin
                .regions
                .as_ref()
                .is_some_and(|old| same_regions(old, regions))
                && origin.graph_unchanged(graph, regions, false)
                && origin.bind_graph_heads(graph)
        });
        (!origins.is_empty()).then(|| Self(graph_items(graph)))
    }

    pub(super) fn finish(self, graph: &BlockGraph, origins: &mut Vec<CopiedAsyncFinallyOwner>) {
        let items = graph_items(graph);
        if items != self.0 || unique_label_positions(&items).is_none() {
            origins.clear();
            return;
        }
        origins.retain_mut(|origin| {
            for (label, block) in &mut origin.graph_heads {
                let Some(new) = graph.label_block(*label) else {
                    return false;
                };
                *block = new;
            }
            true
        });
    }
}

impl CopiedFinallyColdRewrite {
    pub(super) fn capture(
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        origins: &mut Vec<CopiedAsyncFinallyOwner>,
    ) -> Option<Self> {
        if origins.is_empty() {
            return None;
        }
        origins.retain_mut(|origin| {
            origin
                .regions
                .as_ref()
                .is_some_and(|old| same_regions(old, regions))
                && origin.graph_unchanged(graph, regions, false)
                && origin.bind_graph_heads(graph)
        });
        if origins.is_empty() {
            return None;
        }
        let captured = (|| {
            let items = graph_items(graph);
            unique_label_positions(&items)?;
            let order = graph.order().to_vec();
            let positions = order
                .iter()
                .enumerate()
                .map(|(index, block)| (*block, index))
                .collect::<FxHashMap<_, _>>();
            let heads = order
                .iter()
                .map(|block| Some((*block, graph[*block].first_label()?)))
                .collect::<Option<FxHashMap<_, _>>>()?;
            let ranges = regions
                .iter()
                .map(|region| {
                    let start = *positions.get(&graph.label_block(region.start)?)?;
                    let end = *positions.get(&graph.label_block(region.end)?)?;
                    (start <= end).then_some(start..end)
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Self {
                regions: regions.to_vec(),
                order,
                heads,
                ranges,
                bridges: Vec::new(),
                outputs: Vec::new(),
                groups: Vec::new(),
            })
        })();
        if captured.is_none() {
            origins.clear();
        }
        captured
    }

    pub(super) fn bridge(&mut self, block: BasicBlockId, label: Label, instruction: Instruction) {
        self.bridges.push((block, label, instruction));
    }

    pub(super) fn output(&mut self, original_index: usize) {
        self.outputs.push(original_index);
    }

    pub(super) fn group(&mut self, original_index: usize, emitted: usize) {
        self.groups.push((original_index, emitted));
    }

    pub(super) fn finish(
        self,
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        final_label: Option<Label>,
        origins: &mut Vec<CopiedAsyncFinallyOwner>,
    ) {
        let valid = (|| {
            if !bound_regions(&graph_items(graph), regions) {
                return None;
            }
            let mut fresh = FxHashSet::default();
            for (block, label, instruction) in &self.bridges {
                if self.heads.contains_key(block)
                    || !fresh.insert(*block)
                    || graph[*block].items()
                        != [Item::Label(*label), Item::Instruction(*instruction)]
                {
                    return None;
                }
            }
            let mut order = graph.order();
            if let Some(label) = final_label {
                let (last, preceding) = order.split_last()?;
                if graph[*last].items() != [Item::Label(label)]
                    || self.heads.contains_key(last)
                    || fresh.contains(last)
                {
                    return None;
                }
                order = preceding;
            }
            if order
                .iter()
                .any(|block| !self.heads.contains_key(block) && !fresh.contains(block))
            {
                return None;
            }
            if self.heads.iter().any(|(block, label)| {
                order.contains(block) && graph[*block].first_label() != Some(*label)
            }) {
                return None;
            }
            let positions = self
                .order
                .iter()
                .enumerate()
                .map(|(position, block)| (*block, position))
                .collect::<FxHashMap<_, _>>();
            let mut expected = Vec::new();
            if self.groups.len() != self.regions.len() {
                return None;
            }
            for (original_index, (region, range)) in
                self.regions.iter().zip(&self.ranges).enumerate()
            {
                let member = |block: &BasicBlockId| {
                    positions
                        .get(block)
                        .is_some_and(|position| range.contains(position))
                };
                let before = expected.len();
                let mut position = 0;
                while position < order.len() {
                    while position < order.len() && !member(&order[position]) {
                        position += 1;
                    }
                    if position == order.len() {
                        break;
                    }
                    let start = graph[order[position]].first_label()?;
                    while position < order.len() && member(&order[position]) {
                        position += 1;
                    }
                    let end = if position == order.len() {
                        final_label?
                    } else {
                        graph[order[position]].first_label()?
                    };
                    expected.push((
                        original_index,
                        ExceptionRegion {
                            start,
                            end,
                            ..*region
                        },
                    ));
                }
                let count = expected.len() - before;
                if self.groups[original_index] != (original_index, count)
                    || (count == 0 && !range.is_empty())
                {
                    return None;
                }
            }
            if expected.len() != regions.len()
                || self.outputs.len() != regions.len()
                || !expected.iter().zip(regions).zip(&self.outputs).all(
                    |(((index, expected), actual), emitted)| {
                        index == emitted && same_manager_region(expected, actual)
                    },
                )
            {
                return None;
            }
            Some(())
        })()
        .is_some();
        origins.retain_mut(|origin| {
            if !valid {
                return false;
            }
            let empty_disjoint =
                self.groups
                    .iter()
                    .filter(|(_, count)| *count == 0)
                    .all(|(index, _)| {
                        let region = &self.regions[*index];
                        let protected = origin
                            .events
                            .iter()
                            .flat_map(|event| event.items.iter().chain(&event.entry_head))
                            .chain(&origin.original)
                            .chain(&origin.setup_head)
                            .chain(&origin.target_head)
                            .filter_map(label)
                            .filter_map(|label| graph.label_block(label))
                            .collect::<FxHashSet<_>>();
                        [region.start, region.end, region.target]
                            .iter()
                            .all(|label| {
                                graph
                                    .label_block(*label)
                                    .is_some_and(|block| !protected.contains(&block))
                            })
                            && !origin
                                .events
                                .iter()
                                .filter_map(|event| event.cleanup)
                                .any(|cleanup| same_priority(cleanup.priority, region.priority))
                            && region.target != origin.ordinary.2
                    });
            if !empty_disjoint
                || !origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, &self.regions))
                || !origin.graph_unchanged(graph, regions, true)
            {
                return false;
            }
            origin.regions = Some(regions.to_vec());
            true
        });
    }
}

impl Assembler {
    /// Authenticate canonical records immediately after this emitter adds None and exits.
    pub(crate) fn capture_copied_finally_manager_event(
        &self,
        start: usize,
        entry: Label,
        none: u32,
        asynchronous: bool,
        location: SourceLocation,
        constants: &impl super::ConstantTable,
    ) -> Option<CopiedFinallyManagerEvent> {
        if !matches!(self.code, UnoptimizedCode::Linear(_))
            || self.python_version != crate::PythonVersion::Py314
            || none as usize >= constants.len()
            || !matches!(constants.get(none), crate::code_object::Constant::None)
        {
            return None;
        }
        let items = self.linear_items();
        unique_label_positions(items)?;
        let emitted = items.get(start..)?;
        let first = record(emitted.get(1)?)?;
        let depth = first.depth_after()?;
        let value = |opcode, operand, depth| {
            Item::Instruction(Instruction::value_with_depth(
                opcode, operand, location, depth,
            ))
        };
        let common = [
            Item::Label(entry),
            value(SWAP, 3, depth),
            value(SWAP, 2, depth),
            value(LOAD_CONST, none, depth.checked_add(1)?),
            value(LOAD_CONST, none, depth.checked_add(2)?),
            value(LOAD_CONST, none, depth.checked_add(3)?),
            value(CALL, 3, depth.checked_sub(1)?),
        ];
        if !emitted.starts_with(&common) {
            return None;
        }
        let cleanup = if asynchronous {
            if emitted.len() != 21 {
                return None;
            }
            let send = label(&emitted[9])?;
            let yielded = label(&emitted[11])?;
            let resumed = label(&emitted[13])?;
            let cleanup = label(&emitted[16])?;
            let end = label(&emitted[18])?;
            let tail = [
                value(GET_AWAITABLE, 2, depth.checked_sub(1)?),
                value(LOAD_CONST, none, depth),
                Item::Label(send),
                Item::Instruction(Instruction::new(
                    SEND,
                    Operand::Forward(end),
                    location,
                    Some(depth),
                )),
                Item::Label(yielded),
                value(YIELD_VALUE, 1, depth),
                Item::Label(resumed),
                value(RESUME, ResumeLocation::AFTER_AWAIT.argument(), depth),
                Item::Instruction(Instruction::new(
                    JUMP_BACKWARD_NO_INTERRUPT,
                    Operand::Backward(send),
                    location,
                    Some(depth),
                )),
                Item::Label(cleanup),
                value(CLEANUP_THROW, 0, depth),
                Item::Label(end),
                value(END_SEND, 0, depth.checked_sub(1)?),
                value(POP_TOP, 0, depth.checked_sub(2)?),
            ];
            if emitted[7..] != tail {
                return None;
            }
            let mut regions = self
                .exception_regions
                .iter()
                .filter(|region| region.start == yielded);
            let region = *regions.next()?;
            if regions.next().is_some()
                || region.end != resumed
                || region.target != cleanup
                || region.depth != depth
                || region.preserve_lasti
                || region.is_scope_fallback
                || !matches!(region.priority, ExceptionRegionPriority::OriginalRange(a, b) if a == yielded && b == resumed)
            {
                return None;
            }
            Some(region)
        } else {
            if emitted.len() != 8 || emitted[7] != value(POP_TOP, 0, depth.checked_sub(2)?) {
                return None;
            }
            None
        };
        Some(CopiedFinallyManagerEvent {
            stream: self.emission_stream,
            start,
            items: emitted.to_vec(),
            none,
            cleanup,
            bridge: None,
            entry_head: head(items, start)?,
        })
    }

    /// Bind the real omitted manager/ordinary POP decisions to the already emitted finally POP.
    pub(crate) fn record_copied_async_finally_owner(
        &mut self,
        events: Vec<CopiedFinallyManagerEvent>,
        ordinary: (Label, Label, Label),
        aliases: [Label; 2],
        pop: usize,
        local: u32,
        local_location: SourceLocation,
        condition_location: SourceLocation,
    ) {
        let origin = self.copied_async_finally_owner(
            events,
            ordinary,
            aliases,
            pop,
            local,
            local_location,
            condition_location,
        );
        if let Some(origin) = origin {
            self.exception_owner_provenance
                .copied_async_finally_owners
                .push(origin);
        }
    }

    fn copied_async_finally_owner(
        &self,
        events: Vec<CopiedFinallyManagerEvent>,
        ordinary: (Label, Label, Label),
        aliases: [Label; 2],
        pop: usize,
        local: u32,
        local_location: SourceLocation,
        condition_location: SourceLocation,
    ) -> Option<CopiedAsyncFinallyOwner> {
        if !matches!(self.code, UnoptimizedCode::Linear(_))
            || self.python_version != crate::PythonVersion::Py314
            || !(2..=3).contains(&events.len())
        {
            return None;
        }
        let items = self.linear_items();
        let positions = unique_label_positions(items)?;
        for (index, event) in events.iter().enumerate() {
            if event.stream != self.emission_stream
                || items.get(event.start..event.start.checked_add(event.items.len())?)?
                    != event.items
                || event.cleanup.is_some() != (index + 1 < events.len())
                || event.none != events[0].none
                || record(event.items.get(1)?)?.depth_after()
                    != Some(u32::try_from(2 * (events.len() - index) + 1).ok()?)
                || (index > 0
                    && events[index - 1].start + events[index - 1].items.len() != event.start)
            {
                return None;
            }
        }
        let synchronous = events.last()?;
        let completed = &events[events.len() - 2];
        let start = completed.start + 18;
        let after_sync = synchronous.start + synchronous.items.len();
        let pop_item = self.instruction_item_index(pop)?;
        if pop_item != after_sync + 2
            || aliases[0] == aliases[1]
            || items.get(after_sync..pop_item)?
                != [Item::Label(aliases[0]), Item::Label(aliases[1])]
        {
            return None;
        }
        let mut expected_pop = Instruction::value_with_depth(NOP, 0, SourceLocation::NONE, 1);
        expected_pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        expected_pop.fusion_policy.mark_converted_pop_block();
        let loaded = Instruction::value_with_depth(LOAD_FAST, local, local_location, 2);
        let branch = record(items.get(pop_item + 2)?)?;
        let Operand::Forward(target) = branch.operand() else {
            return None;
        };
        let mut expected_branch = Instruction::new(
            POP_JUMP_IF_NONE,
            Operand::Forward(target),
            condition_location,
            Some(1),
        );
        expected_branch
            .local_policy
            .add_hidden_exception_owner_slots(3);
        let mut marker = Instruction::value_with_depth(NOT_TAKEN, 0, condition_location, 1);
        marker.set_exception_policy(ExceptionPolicy::NonePresent);
        if items.get(pop_item..pop_item + 4)?
            != [
                Item::Instruction(expected_pop),
                Item::Instruction(loaded),
                Item::Instruction(expected_branch),
                Item::Instruction(marker),
            ]
        {
            return None;
        }
        let setup_position = *positions.get(&ordinary.0)?;
        let setup_head = head(items, setup_position)?;
        if self
            .exception_owner_provenance
            .setups
            .iter()
            .filter(|(entry, _)| *entry == ordinary.0)
            .copied()
            .collect::<Vec<_>>()
            != [(ordinary.0, None)]
            || self
                .exception_owner_provenance
                .ordinary_try_setups
                .get(&ordinary.0)
                != Some(&ordinary.2)
            || setup_position >= start
        {
            return None;
        }
        let target_head = head(items, *positions.get(&target)?)?;
        let original = items[start..pop_item + 4].to_vec();
        let mut surviving = original.clone();
        surviving.remove(pop_item - start);
        Some(CopiedAsyncFinallyOwner {
            stream: self.emission_stream,
            events,
            original,
            surviving,
            original_start: start,
            setup: (ordinary.0, ordinary.2),
            setup_head,
            ordinary,
            regions: None,
            target,
            target_head,
            pop_removed: false,
            pop_offset: pop_item - start,
            marker_label: None,
            graph_heads: Vec::new(),
        })
    }

    /// Finish region identity before either Linear or Prepared code enters optimization.
    pub(super) fn seal_copied_async_finally_owners(&mut self) {
        if !matches!(self.code, UnoptimizedCode::Linear(_)) {
            return;
        }
        let mut origins =
            std::mem::take(&mut self.exception_owner_provenance.copied_async_finally_owners);
        origins.retain_mut(|origin| origin.seal(self));
        self.exception_owner_provenance.copied_async_finally_owners = origins;
    }
}

impl CopiedAsyncFinallyOwner {
    fn seal(&mut self, assembler: &Assembler) -> bool {
        if self.regions.is_some() {
            return true;
        }
        let items = assembler.linear_items();
        let Some(positions) = unique_label_positions(items) else {
            return false;
        };
        if self.stream != assembler.emission_stream
            || items.get(self.original_start..self.original_start + self.original.len())
                != Some(self.original.as_slice())
            || complete_head_start(items, &self.setup_head).is_none()
            || complete_head_start(items, &self.target_head).is_none()
            || assembler
                .exception_owner_provenance
                .setups
                .iter()
                .filter(|(entry, _)| *entry == self.setup.0)
                .copied()
                .collect::<Vec<_>>()
                != [(self.setup.0, None)]
            || assembler
                .exception_owner_provenance
                .ordinary_try_setups
                .get(&self.setup.0)
                != Some(&self.setup.1)
        {
            return false;
        }
        let ordinary = assembler
            .exception_regions
            .iter()
            .filter(|region| {
                matches!(region.priority, ExceptionRegionPriority::OriginalRange(start, end)
                if start == self.ordinary.0 && end == self.ordinary.1)
            })
            .collect::<Vec<_>>();
        let authored = assembler
            .exception_owner_provenance
            .ordinary_try_region_origins
            .iter()
            .filter(|(setup, _, _, _)| *setup == self.ordinary.0)
            .collect::<Vec<_>>();
        if ordinary.is_empty()
            || ordinary.len() != authored.len()
            || !ordinary.iter().zip(authored).all(|(region, original)| {
                (self.ordinary.0, region.start, region.end, region.target) == *original
                    && region.target == self.ordinary.2
                    && region.depth == 0
                    && !region.preserve_lasti
                    && !region.is_scope_fallback
            })
        {
            return false;
        }
        for event in &self.events {
            if items.get(event.start..event.start + event.items.len())
                != Some(event.items.as_slice())
                || anchored_start(items, &event.anchored_items(event.items.len())).is_none()
            {
                return false;
            }
            if let Some(cleanup) = event.cleanup {
                let matching = assembler
                    .exception_regions
                    .iter()
                    .filter(|region| same_priority(region.priority, cleanup.priority))
                    .collect::<Vec<_>>();
                if !matches!(matching.as_slice(), [region] if same_manager_region(region, &cleanup))
                {
                    return false;
                }
            }
        }
        if assembler.exception_regions.iter().any(|region| {
            [region.start, region.end, region.target]
                .iter()
                .any(|label| !positions.contains_key(label))
        }) {
            return false;
        }
        self.regions = Some(assembler.exception_regions.clone());
        true
    }

    pub(super) fn visit_records_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        for item in self
            .events
            .iter_mut()
            .flat_map(|event| &mut event.items)
            .chain(&mut self.original)
            .chain(&mut self.surviving)
        {
            if let Item::Instruction(instruction) = item {
                visit(instruction);
            }
        }
    }

    pub(super) fn remap_constants(
        &mut self,
        map: &[Option<u32>],
        deferred_none: Option<u32>,
    ) -> bool {
        let mut valid = true;
        self.visit_records_mut(|instruction| {
            if uses_constant_index(instruction.opcode) {
                let Operand::Value(index) = instruction.operand() else {
                    valid = false;
                    return;
                };
                let index = if index == u32::MAX {
                    deferred_none
                } else {
                    Some(index)
                };
                if let Some(Some(new)) = index.and_then(|index| map.get(index as usize)) {
                    instruction.set_operand(Operand::Value(*new));
                } else {
                    valid = false;
                }
            }
        });
        for event in &mut self.events {
            if let Some(Some(new)) = map.get(event.none as usize) {
                event.none = *new;
            } else {
                valid = false;
            }
        }
        valid
    }

    pub(super) fn resolve_priorities(&mut self, positions: &FxHashMap<Label, usize>) -> bool {
        let Some(regions) = self.regions.as_mut() else {
            return false;
        };
        for region in regions.iter_mut().chain(
            self.events
                .iter_mut()
                .filter_map(|event| event.cleanup.as_mut()),
        ) {
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
}

impl LayoutAssembler {
    pub(super) fn copied_async_finally_null_markers(&self) -> FxHashSet<usize> {
        let mut result = FxHashSet::default();
        if self
            .exception_owner_provenance
            .copied_async_finally_owners
            .is_empty()
        {
            return result;
        }
        let Some(positions) = unique_label_positions(&self.items) else {
            return result;
        };
        for origin in &self.exception_owner_provenance.copied_async_finally_owners {
            let proof = (|| {
                if !origin.pop_removed || !origin.records_unchanged(&self.items, true) {
                    return None;
                }
                let start = anchored_start(&self.items, &origin.surviving)?;
                complete_head_start(&self.items, &origin.setup_head)?;
                complete_head_start(&self.items, &origin.target_head)?;
                let regions = origin.regions.as_ref()?;
                if regions.len() != self.exception_regions.len()
                    || !regions
                        .iter()
                        .zip(&self.exception_regions)
                        .all(|(a, b)| same_manager_region(a, b))
                    || !positions.contains_key(&origin.target)
                    || self
                        .exception_owner_provenance
                        .ordinary_try_setups
                        .get(&origin.setup.0)
                        != Some(&origin.setup.1)
                    || self
                        .exception_owner_provenance
                        .setups
                        .iter()
                        .filter(|(entry, _)| *entry == origin.setup.0)
                        .copied()
                        .collect::<Vec<_>>()
                        != [(origin.setup.0, None)]
                {
                    return None;
                }
                let marker = start + origin.surviving.len() - 1;
                Some(
                    self.items[..marker]
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count(),
                )
            })();
            if let Some(marker) = proof {
                result.insert(marker);
            }
        }
        result
    }
}

impl OptimizationPipeline {
    /// Keep graph occurrence and reachability identity until the graph becomes linear.
    pub(super) fn validate_copied_async_finally_graph(&mut self) {
        self.exception_owner_provenance
            .copied_async_finally_owners
            .retain(|origin| {
                origin.pop_removed
                    && origin
                        .regions
                        .as_ref()
                        .is_some_and(|old| same_regions(old, &self.exception_regions))
                    && origin.graph_unchanged(&self.graph, &self.exception_regions, true)
            });
    }

    /// Record only the real removal and one-cell donation selected by the existing NOP pass.
    pub(super) fn transport_copied_async_finally_pop(
        &mut self,
        removed: &[ItemPosition],
        donations: &[(ItemPosition, u16)],
    ) {
        if self
            .exception_owner_provenance
            .copied_async_finally_owners
            .is_empty()
        {
            return;
        }
        let positioned = self
            .graph
            .iter()
            .flat_map(|(block, contents)| {
                contents
                    .items()
                    .iter()
                    .enumerate()
                    .map(move |(offset, item)| (ItemPosition::new(block, offset), *item))
            })
            .collect::<Vec<_>>();
        let items = positioned.iter().map(|(_, item)| *item).collect::<Vec<_>>();
        if unique_label_positions(&items).is_none() {
            self.exception_owner_provenance
                .copied_async_finally_owners
                .clear();
            return;
        }
        let reachable = self.graph.reachable_blocks(&self.exception_regions);
        for origin in &mut self.exception_owner_provenance.copied_async_finally_owners {
            if origin.pop_removed {
                continue;
            }
            let Some(start) = anchored_start(&items, &origin.original) else {
                continue;
            };
            let pop = start + origin.pop_offset;
            let load = pop + 1;
            if removed
                .iter()
                .filter(|position| **position == positioned[pop].0)
                .count()
                != 1
                || donations
                    .iter()
                    .filter(|(position, _)| *position == positioned[load].0)
                    .copied()
                    .collect::<Vec<_>>()
                    != [(positioned[load].0, 1)]
                || positioned[start..start + origin.original.len()]
                    .iter()
                    .enumerate()
                    .any(|(offset, (position, _))| {
                        !reachable[position.block().index()]
                            || (start + offset != pop && removed.contains(position))
                    })
            {
                continue;
            }
            let Item::Instruction(before) = items[load] else {
                continue;
            };
            let mut after = before;
            after.local_policy.add_hidden_exception_owner_slots(1);
            let surviving_load = origin.pop_offset;
            if origin.surviving[surviving_load] != Item::Instruction(before) {
                continue;
            }
            origin.surviving[surviving_load] = Item::Instruction(after);
            origin.pop_removed = true;
        }
    }
}
