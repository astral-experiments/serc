//! A retained SETUP cell reused by the copied-finalizer truth marker.

use super::block_graph::{BasicBlockId, BlockGraph, ItemPosition};
use super::exception_owners::ExceptionOwnerProvenance;
use super::fusion_owner_slots::same_manager_region;
use super::ir::EmissionStreamId;
use super::normal_finally_locations::{anchored_start, unique_label_positions};
use super::{
    Assembler, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    LayoutAssembler, Operand, OptimizationPipeline, SourceLocation, UnoptimizedCode,
};
use crate::target::opcodes::{
    CALL, CALL_KW, JUMP_FORWARD, LOAD_ATTR, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL, NOP, NOT_TAKEN,
    POP_JUMP_IF_FALSE, POP_TOP, PUSH_NULL, STORE_FAST, SWAP, TO_BOOL,
};
use rustc_hash::{FxHashMap, FxHashSet};

mod cold;
#[cfg(test)]
mod tests;
pub(super) use cold::RetainedSetupColdRewrite;
mod collapse;
pub(super) use collapse::RetainedSetupCollapse;

#[derive(Debug)]
pub(crate) struct RetainedSetupTruthStart {
    stream: EmissionStreamId,
    start: usize,
    prefix: Vec<Item>,
    predecessor: Vec<Item>,
    join: Label,
    exit: Instruction,
    unwind: Option<(Label, Label, Label, Label)>,
}

impl RetainedSetupTruthStart {
    pub(crate) fn bind_unwind(
        &mut self,
        general: Label,
        iterator: (Label, Label),
        collector: Label,
    ) {
        self.unwind = Some((general, iterator.0, iterator.1, collector));
    }
}

#[derive(Debug)]
pub(super) struct RetainedSetupTruth {
    source: RetainedSetupTruthStart,
    original: Vec<Item>,
    current: Vec<Item>,
    current_predecessor: Vec<Item>,
    current_exit: Instruction,
    marker_head: Option<Label>,
    pop: usize,
    setup: (Label, Label, Label),
    handler: Label,
    setup_regions: Vec<ExceptionRegion>,
    target_head: Vec<Item>,
    target_collapse: Option<TargetCollapse>,
    setup_location: SourceLocation,
    regions: Option<Vec<ExceptionRegion>>,
    removed: bool,
    borrow_validated: bool,
    graph_validated: bool,
    roles: Option<Vec<LabelRoles>>,
}

#[derive(Debug, PartialEq, Eq)]
struct LabelRoles {
    label: Label,
    flags: Vec<bool>,
    internal_copy_count: usize,
    setup: Vec<Option<SourceLocation>>,
    ordinary_setup: Option<Label>,
    omitted: Option<usize>,
    normal_exits: Vec<(Label, Label)>,
}

impl LabelRoles {
    fn capture(provenance: &ExceptionOwnerProvenance, label: Label) -> Self {
        Self {
            label,
            flags: [
                &provenance.block_boundaries,
                &provenance.borrow_only_block_boundaries,
                &provenance.source_boundaries,
                &provenance.continuation_boundaries,
                &provenance.value_jump_continuation_boundaries,
                &provenance.return_region_reentries,
                &provenance.loop_finally_region_boundaries,
                &provenance.fusion_boundaries,
                &provenance.setup_nop_boundaries,
                &provenance.original_fallthrough_setups,
                &provenance.named_handler_cleanup_targets,
                &provenance.normal_try_copy_blocks,
                &provenance.retired_normal_try_copy_labels,
                &provenance.normal_try_copy_inline_labels,
                &provenance.protected_prefix_boundaries,
            ]
            .iter()
            .map(|set| set.contains(&label))
            .collect(),
            internal_copy_count: provenance
                .normal_try_copy_internal_labels
                .iter()
                .filter(|entry| **entry == label)
                .count(),
            setup: provenance
                .setups
                .iter()
                .filter(|(entry, _)| *entry == label)
                .map(|(_, value)| *value)
                .collect(),
            ordinary_setup: provenance.ordinary_try_setups.get(&label).copied(),
            omitted: provenance.omitted_nop_counts.get(&label).copied(),
            normal_exits: provenance
                .normal_try_exits
                .iter()
                .filter(|(source, target)| *source == label || *target == label)
                .copied()
                .collect(),
        }
    }
}

#[derive(Debug)]
struct TargetCollapse {
    source: Label,
    target: Label,
    original: [Item; 2],
    removed: bool,
}

fn instruction(item: &Item) -> Option<Instruction> {
    if let Item::Instruction(value) = item {
        Some(*value)
    } else {
        None
    }
}

fn label(item: &Item) -> Option<Label> {
    if let Item::Label(value) = item {
        Some(*value)
    } else {
        None
    }
}

fn head(items: &[Item], at: usize) -> Option<Vec<Item>> {
    let mut start = at;
    while start > 0 && matches!(items[start - 1], Item::Label(_)) {
        start -= 1;
    }
    let mut end = start;
    while matches!(items.get(end), Some(Item::Label(_))) {
        end += 1;
    }
    (end > start).then(|| items[start..end].to_vec())
}

fn same_regions(left: &[ExceptionRegion], right: &[ExceptionRegion]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| same_manager_region(a, b))
}

fn original_setup_regions(
    regions: &[ExceptionRegion],
    positions: &FxHashMap<Label, usize>,
    entry: Label,
    guard_end: usize,
) -> Option<Vec<ExceptionRegion>> {
    let starts = regions
        .iter()
        .filter(|region| region.start == entry)
        .collect::<Vec<_>>();
    let [first] = starts.as_slice() else {
        return None;
    };
    let ExceptionRegionPriority::OriginalRange(start, end) = first.priority else {
        return None;
    };
    if start != entry || first.depth != 1 || first.preserve_lasti || first.is_scope_fallback {
        return None;
    }
    let origin_start = *positions.get(&start)?;
    let origin_end = *positions.get(&end)?;
    let handler = *positions.get(&first.target)?;
    if origin_start >= guard_end || guard_end >= origin_end || origin_end > handler {
        return None;
    }
    let mut captured = Vec::new();
    for region in regions {
        let same_priority = matches!(region.priority,
            ExceptionRegionPriority::OriginalRange(a, b) if a == start && b == end);
        if region.target != first.target && !same_priority {
            continue;
        }
        if !same_priority
            || region.target != first.target
            || region.depth != first.depth
            || region.preserve_lasti != first.preserve_lasti
            || region.is_scope_fallback != first.is_scope_fallback
        {
            return None;
        }
        let current_start = *positions.get(&region.start)?;
        let current_end = *positions.get(&region.end)?;
        if current_start < origin_start || current_start >= current_end || current_end > origin_end
        {
            return None;
        }
        if let Some(previous) = captured.last() {
            let previous: &ExceptionRegion = previous;
            if *positions.get(&previous.end)? >= current_start {
                return None;
            }
        }
        captured.push(*region);
    }
    if !matches!(captured.len(), 1 | 2)
        || captured.first()?.start != start
        || captured.last()?.end != end
        || *positions.get(&captured.first()?.end)? <= guard_end
    {
        return None;
    }
    Some(captured)
}

impl Assembler {
    /// Captures the real authored value join before the iterator return is unwound.
    pub(crate) fn capture_retained_setup_truth_start(
        &self,
        returned: usize,
        local: u32,
        location: SourceLocation,
    ) -> Option<RetainedSetupTruthStart> {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        if self.python_version != crate::PythonVersion::Py314
            || items.get(returned)
                != Some(&Item::Instruction(Instruction::value_with_depth(
                    LOAD_FAST, local, location, 2,
                )))
        {
            return None;
        }
        let positions = unique_label_positions(items)?;
        let candidates = self
            .exception_owner_provenance
            .original_if_expression_joins
            .iter()
            .filter_map(|(join, exit)| {
                let start = *positions.get(join)?;
                (start < returned
                    && items[start + 1..=returned]
                        .iter()
                        .all(|item| matches!(item, Item::Instruction(_))))
                .then_some((*join, *exit, start))
            })
            .collect::<Vec<_>>();
        let [(join, exit, start)] = candidates.as_slice() else {
            return None;
        };
        let (join, exit, start) = (*join, *exit, *start);
        if head(items, start)?.as_slice() != [Item::Label(join)] {
            return None;
        }
        let incoming = items.iter().enumerate().filter_map(|(index, item)| {
            let value = instruction(item)?;
            matches!(value.operand(), Operand::Forward(target) | Operand::Backward(target) if target == join)
                .then_some((index, value))
        }).collect::<Vec<_>>();
        let [(predecessor_end, actual)] = incoming.as_slice() else {
            return None;
        };
        if *actual != exit
            || exit.opcode != JUMP_FORWARD
            || exit.operand() != Operand::Forward(join)
            || *predecessor_end >= start
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(exit))
                .count()
                != 1
        {
            return None;
        }
        let predecessor_start = items[..*predecessor_end]
            .iter()
            .rposition(|item| matches!(item, Item::Label(_)))?;
        let predecessor_head = head(items, predecessor_start)?;
        let predecessor_start = predecessor_start + 1 - predecessor_head.len();
        let prefix = items[start..=returned].to_vec();
        let mut fusions = 0;
        let mut previous = None;
        for item in &prefix[1..] {
            let value = instruction(item)?;
            let Operand::Value(argument) = value.operand() else {
                return None;
            };
            if !matches!(
                value.opcode,
                LOAD_FAST
                    | LOAD_CONST
                    | LOAD_GLOBAL
                    | LOAD_ATTR
                    | CALL
                    | CALL_KW
                    | STORE_FAST
                    | POP_TOP
                    | PUSH_NULL
            ) || value
                != Instruction::new(
                    value.opcode,
                    value.operand(),
                    value.location,
                    value.depth_after(),
                )
                || value.location.line < 0
                || (value.opcode == LOAD_ATTR && argument & 1 != 0)
                || (value.opcode == PUSH_NULL && (argument != 0 || previous == Some(LOAD_GLOBAL)))
            {
                return None;
            }
            fusions += usize::from(value.opcode == LOAD_GLOBAL && argument & 1 != 0);
            previous = Some(value.opcode);
        }
        if fusions != 3 {
            return None;
        }
        Some(RetainedSetupTruthStart {
            stream: self.emission_stream,
            start,
            prefix,
            predecessor: items[predecessor_start..=*predecessor_end].to_vec(),
            join,
            exit,
            unwind: None,
        })
    }

    /// Binds the actual copied POP, retained SETUP and first original truth condition.
    pub(crate) fn record_retained_setup_truth(
        &mut self,
        source: RetainedSetupTruthStart,
        pop: usize,
        local: u32,
        condition: SourceLocation,
        setup_location: SourceLocation,
        returned: SourceLocation,
    ) {
        let origin =
            self.retained_setup_truth(source, pop, local, condition, setup_location, returned);
        if let Some(origin) = origin {
            self.exception_owner_provenance
                .retained_setup_truths
                .push(origin);
        }
    }

    fn retained_setup_truth(
        &self,
        source: RetainedSetupTruthStart,
        pop: usize,
        local: u32,
        condition: SourceLocation,
        setup_location: SourceLocation,
        returned: SourceLocation,
    ) -> Option<RetainedSetupTruth> {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        let positions = unique_label_positions(items)?;
        let (general, iterator_start, iterator_end, collector) = source.unwind?;
        let after_prefix = source.start + source.prefix.len();
        if source.stream != self.emission_stream
            || self.python_version != crate::PythonVersion::Py314
            || items.get(source.start..after_prefix)? != source.prefix
            || pop != after_prefix + 6
            || items.get(after_prefix..pop)?
                != [
                    Item::Label(general),
                    Item::Label(iterator_start),
                    Item::Instruction(Instruction::value_with_depth(SWAP, 2, returned, 2)),
                    Item::Instruction(Instruction::value_with_depth(POP_TOP, 0, returned, 1)),
                    Item::Label(iterator_end),
                    Item::Label(collector),
                ]
            || anchored_start(items, &source.predecessor).is_none()
        {
            return None;
        }
        let mut expected_pop = Instruction::value_with_depth(NOP, 0, returned, 1);
        expected_pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        expected_pop.fusion_policy.mark_converted_pop_block();
        let mut expected_setup = Instruction::value_with_depth(NOP, 0, setup_location, 1);
        expected_setup.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        let setup_start = label(items.get(pop + 1)?)?;
        let setup_end = label(items.get(pop + 3)?)?;
        let entry = label(items.get(pop + 4)?)?;
        let branch = instruction(items.get(pop + 7)?)?;
        let Operand::Forward(target) = branch.operand() else {
            return None;
        };
        let mut marker = Instruction::value_with_depth(NOT_TAKEN, 0, condition, 1);
        marker.set_exception_policy(ExceptionPolicy::NoneDynamicCondition);
        let expected = [
            Item::Instruction(expected_pop),
            Item::Label(setup_start),
            Item::Instruction(expected_setup),
            Item::Label(setup_end),
            Item::Label(entry),
            Item::Instruction(Instruction::value_with_depth(
                LOAD_FAST, local, condition, 2,
            )),
            Item::Instruction(Instruction::value_with_depth(TO_BOOL, 0, condition, 2)),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(target),
                condition,
                Some(1),
            )),
            Item::Instruction(marker),
        ];
        if items.get(pop..pop + expected.len())? != expected
            || self
                .exception_owner_provenance
                .setup_nop_ranges
                .iter()
                .filter(|(start, end)| *start == setup_start || *end == setup_end)
                .copied()
                .collect::<Vec<_>>()
                != [(setup_start, setup_end)]
            || self
                .exception_owner_provenance
                .setups
                .iter()
                .filter(|(label, _)| *label == entry)
                .copied()
                .collect::<Vec<_>>()
                != [(entry, Some(setup_location))]
        {
            return None;
        }
        let setup_regions =
            original_setup_regions(&self.exception_regions, &positions, entry, pop + 8)?;
        let handler = setup_regions[0].target;
        let original = items[source.start..pop + 9].to_vec();
        if items
            .iter()
            .filter(|item| **item == Item::Instruction(expected_pop))
            .count()
            != 1
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(expected_setup))
                .count()
                != 1
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(marker))
                .count()
                != 1
        {
            return None;
        }
        let mut physical = Vec::new();
        let mut compacted = Vec::new();
        for (index, item) in original.iter().enumerate() {
            let Some(value) = instruction(item) else {
                continue;
            };
            if value.opcode == NOT_TAKEN {
                break;
            }
            physical.push(index);
            if value.opcode == LOAD_GLOBAL
                && matches!(value.operand(), Operand::Value(argument) if argument & 1 != 0)
            {
                physical.push(index);
            }
            if index != pop - source.start {
                compacted.push(index);
            }
        }
        // The three real global fusions and one erased POP leave exactly the old SETUP cell.
        if physical.get(compacted.len()) != Some(&(pop + 2 - source.start)) {
            return None;
        }
        let target_head = head(items, *positions.get(&target)?)?;
        let collapse_candidates = self
            .exception_owner_provenance
            .normal_try_exits
            .iter()
            .filter(|(_, target)| target_head.contains(&Item::Label(*target)))
            .collect::<Vec<_>>();
        let target_collapse = if let [(source, target)] = collapse_candidates.as_slice() {
            let at = *positions.get(source)?;
            let jump = instruction(items.get(at + 1)?)?;
            if head(items, at)?.as_slice() != [Item::Label(*source)]
                || !matches!(items.get(at + 2), Some(Item::Label(_)))
                || at >= *positions.get(target)?
                || jump
                    != Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(*target),
                        jump.location,
                        Some(1),
                    )
                || jump.location.line < 0
                || items
                    .iter()
                    .filter(|item| **item == Item::Instruction(jump))
                    .count()
                    != 1
                || self.exception_regions.iter().any(|region| {
                    region.target == *source || target_head.contains(&Item::Label(region.target))
                })
            {
                return None;
            }
            Some(TargetCollapse {
                source: *source,
                target: *target,
                original: [Item::Label(*source), Item::Instruction(jump)],
                removed: false,
            })
        } else if collapse_candidates.is_empty() {
            None
        } else {
            return None;
        };
        let mut origin = RetainedSetupTruth {
            pop: pop - source.start,
            current_predecessor: source.predecessor.clone(),
            current_exit: source.exit,
            marker_head: None,
            source,
            current: original.clone(),
            original,
            setup: (setup_start, setup_end, entry),
            handler,
            setup_regions,
            target_head,
            target_collapse,
            setup_location,
            regions: None,
            removed: false,
            borrow_validated: false,
            graph_validated: false,
            roles: None,
        };
        let sealed = origin.seal(self);
        sealed.then_some(origin)
    }

    pub(super) fn seal_retained_setup_truths(&mut self) {
        if !matches!(self.code, UnoptimizedCode::Linear(_)) {
            return;
        }
        let mut origins =
            std::mem::take(&mut self.exception_owner_provenance.retained_setup_truths);
        origins.retain_mut(|origin| origin.seal(self));
        for origin in &mut origins {
            origin.finish_regions(&self.exception_regions);
            let labels = origin
                .current
                .iter()
                .chain(&origin.current_predecessor)
                .chain(&origin.target_head)
                .chain(
                    origin
                        .target_collapse
                        .iter()
                        .flat_map(|collapse| &collapse.original),
                )
                .filter_map(label)
                .collect::<FxHashSet<_>>();
            let mut labels = labels.into_iter().collect::<Vec<_>>();
            labels.sort_unstable_by_key(|label| label.as_u32());
            origin.roles = Some(
                labels
                    .into_iter()
                    .map(|label| LabelRoles::capture(&self.exception_owner_provenance, label))
                    .collect(),
            );
        }
        self.exception_owner_provenance.retained_setup_truths = origins;
    }
}

impl RetainedSetupTruth {
    pub(super) fn before_borrow(
        origins: &mut Vec<Self>,
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
    ) -> Vec<(Label, usize, Instruction)> {
        let mut selected = Vec::new();
        origins.retain(|origin| {
            if !origin.graph_validated
                || !origin.records(items)
                || !origin.roles_match(provenance, regions)
                || !origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, regions))
            {
                return false;
            }
            let Some(start) = anchored_start(items, &origin.current) else {
                return false;
            };
            let offset = origin.current.len() - 5;
            let Some(before) = instruction(&origin.current[offset]) else {
                return false;
            };
            if before.opcode != LOAD_FAST
                || origin.current[offset + 3]
                    != Item::Label(origin.marker_head.expect("validated marker head"))
            {
                return false;
            }
            selected.push((origin.source.join, start + offset, before));
            true
        });
        selected
    }

    pub(super) fn after_borrow(
        origins: &mut Vec<Self>,
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        selected: &[(Label, usize, Instruction)],
    ) {
        origins.retain_mut(|origin| {
            let matches = selected
                .iter()
                .filter(|(join, _, _)| *join == origin.source.join)
                .collect::<Vec<_>>();
            let [(_, at, before)] = matches.as_slice() else {
                return false;
            };
            let Some(after) = items.get(*at).and_then(instruction) else {
                return false;
            };
            let mut expected = *before;
            expected.opcode = crate::target::opcodes::LOAD_FAST_BORROW;
            let offset = origin.current.len() - 5;
            if origin.current[offset] != Item::Instruction(*before)
                || (after != *before && after != expected)
            {
                return false;
            }
            origin.current[offset] = Item::Instruction(after);
            origin.borrow_validated = origin.records(items)
                && origin.roles_match(provenance, regions)
                && origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, regions));
            origin.borrow_validated
        });
    }

    fn roles_match(
        &self,
        provenance: &ExceptionOwnerProvenance,
        regions: &[ExceptionRegion],
    ) -> bool {
        self.roles.as_ref().is_some_and(|roles| {
            roles
                .iter()
                .all(|role| *role == LabelRoles::capture(provenance, role.label))
        }) && !regions.iter().any(|region| {
            self.target_head.contains(&Item::Label(region.target))
                || self
                    .target_collapse
                    .as_ref()
                    .is_some_and(|collapse| region.target == collapse.source)
        })
    }
    fn records(&self, items: &[Item]) -> bool {
        let Some(positions) = unique_label_positions(items) else {
            return false;
        };
        let Some(start) = anchored_start(items, &self.current) else {
            return false;
        };
        let Some(predecessor) = anchored_start(items, &self.current_predecessor) else {
            return false;
        };
        let predecessor_head = self
            .source
            .predecessor
            .iter()
            .take_while(|item| matches!(item, Item::Label(_)))
            .copied()
            .collect::<Vec<_>>();
        let setup_record = self.current[self.pop + 2 - usize::from(self.removed)];
        let marker = *self.current.last().expect("captured truth marker");
        if head(items, start).as_deref() != Some(&[Item::Label(self.source.join)])
            || head(items, predecessor).as_deref() != Some(predecessor_head.as_slice())
            || items.iter().filter(|item| **item == setup_record).count() != 1
            || items.iter().filter(|item| **item == marker).count() != 1
            || (!self.removed
                && items
                    .iter()
                    .filter(|item| **item == self.current[self.pop])
                    .count()
                    != 1)
            || anchored_start(items, &self.target_head).is_none()
            || self
                .target_head
                .first()
                .and_then(label)
                .and_then(|label| positions.get(&label))
                .and_then(|at| head(items, *at))
                .as_deref()
                != Some(self.target_head.as_slice())
        {
            return false;
        }
        if let Some(collapse) = &self.target_collapse {
            if !collapse.removed {
                let Some(at) = anchored_start(items, &collapse.original) else {
                    return false;
                };
                if head(items, at).as_deref() != Some(&[Item::Label(collapse.source)])
                    || !matches!(items.get(at + 2), Some(Item::Label(_)))
                    || items
                        .iter()
                        .filter(|item| **item == collapse.original[1])
                        .count()
                        != 1
                {
                    return false;
                }
            }
        }
        let labels = self
            .current
            .iter()
            .filter_map(label)
            .collect::<FxHashSet<_>>();
        let incoming = items.iter().filter_map(instruction).filter(|value|
            matches!(value.operand(), Operand::Forward(target) | Operand::Backward(target) if labels.contains(&target))).collect::<Vec<_>>();
        incoming == [self.current_exit]
    }

    pub(super) fn prepend_marker_head(
        &mut self,
        graph: &BlockGraph,
        block: BasicBlockId,
        fresh: Label,
    ) {
        if self.marker_head.is_some() || graph[block].first_label().is_some() {
            return;
        }
        let items = graph.copy_items();
        if !self.records(&items) || items.contains(&Item::Label(fresh)) {
            return;
        }
        let Some(start) = anchored_start(&items, &self.current) else {
            return;
        };
        let Some(block_start) = graph
            .iter()
            .scan(0, |offset, (id, data)| {
                let start = *offset;
                *offset += data.items().len();
                Some((id, start))
            })
            .find_map(|(id, at)| (id == block).then_some(at))
        else {
            return;
        };
        if block_start != start + self.current.len() - 1
            || graph[block].items().first() != self.current.last()
        {
            return;
        }
        self.current
            .insert(self.current.len() - 1, Item::Label(fresh));
        self.marker_head = Some(fresh);
    }

    fn seal(&mut self, assembler: &Assembler) -> bool {
        if self.regions.is_some() {
            return true;
        }
        let items = assembler.linear_items();
        if self.source.stream != assembler.emission_stream
            || !self.records(items)
            || items.get(self.source.start..self.source.start + self.original.len())
                != Some(self.original.as_slice())
        {
            return false;
        }
        let Some(positions) = unique_label_positions(items) else {
            return false;
        };
        let Some(current_regions) = original_setup_regions(
            &assembler.exception_regions,
            &positions,
            self.setup.2,
            self.source.start + self.original.len() - 1,
        ) else {
            return false;
        };
        if !same_regions(&self.setup_regions, &current_regions) {
            return false;
        }
        // Regions are captured after the complete function has been emitted.
        true
    }

    pub(super) fn finish_regions(&mut self, regions: &[ExceptionRegion]) {
        if self.regions.is_none() {
            self.regions = Some(regions.to_vec());
        }
    }

    pub(super) fn resolve_priorities(&mut self, positions: &FxHashMap<Label, usize>) -> bool {
        let Some(regions) = &mut self.regions else {
            return false;
        };
        for region in regions {
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

    pub(super) fn visit_records_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        visit(&mut self.source.exit);
        for item in self
            .source
            .prefix
            .iter_mut()
            .chain(&mut self.source.predecessor)
            .chain(&mut self.original)
            .chain(&mut self.current)
            .chain(&mut self.current_predecessor)
        {
            if let Item::Instruction(value) = item {
                visit(value);
            }
        }
        visit(&mut self.current_exit);
        if let Some(collapse) = &mut self.target_collapse {
            if let Item::Instruction(value) = &mut collapse.original[1] {
                visit(value);
            }
        }
    }

    pub(super) fn remap_constants(
        &mut self,
        map: &[Option<u32>],
        deferred_none: Option<u32>,
    ) -> bool {
        let mut valid = true;
        self.visit_records_mut(|value| {
            if super::uses_constant_index(value.opcode) {
                let Operand::Value(index) = value.operand() else {
                    valid = false;
                    return;
                };
                let index = if index == u32::MAX {
                    deferred_none
                } else {
                    Some(index)
                };
                if let Some(Some(new)) = index.and_then(|index| map.get(index as usize)) {
                    value.set_operand(Operand::Value(*new));
                } else {
                    valid = false;
                }
            }
        });
        valid
    }
}

impl OptimizationPipeline {
    pub(super) fn validate_retained_setup_truth_graph(&mut self) {
        if self
            .exception_owner_provenance
            .retained_setup_truths
            .is_empty()
        {
            return;
        }
        let items = self.graph.copy_items();
        let live = self.graph.reachable_blocks(&self.exception_regions);
        let mut origins =
            std::mem::take(&mut self.exception_owner_provenance.retained_setup_truths);
        origins.retain_mut(|origin| {
            origin.graph_validated = origin.removed
                && origin.marker_head.is_some()
                && origin.records(&items)
                && origin.roles_match(&self.exception_owner_provenance, &self.exception_regions)
                && origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, &self.exception_regions))
                && origin
                    .current
                    .iter()
                    .chain(&origin.current_predecessor)
                    .chain(&origin.target_head)
                    .filter_map(label)
                    .all(|label| {
                        self.graph
                            .label_block(label)
                            .is_some_and(|block| live.get(block.index()) == Some(&true))
                    });
            origin.graph_validated
        });
        self.exception_owner_provenance.retained_setup_truths = origins;
    }
    pub(super) fn before_retained_setup_truth_locations(&mut self) -> Vec<(Label, ItemPosition)> {
        if self
            .exception_owner_provenance
            .retained_setup_truths
            .is_empty()
        {
            return Vec::new();
        }
        let items = self.graph.copy_items();
        let positions = self
            .graph
            .iter()
            .flat_map(|(block, data)| {
                (0..data.items().len()).map(move |offset| ItemPosition::new(block, offset))
            })
            .collect::<Vec<_>>();
        let mut captured = Vec::new();
        self.exception_owner_provenance
            .retained_setup_truths
            .retain(|origin| {
                if !origin.records(&items) || origin.current_exit.location.line >= 0 {
                    return false;
                }
                let Some(start) = anchored_start(&items, &origin.current_predecessor) else {
                    return false;
                };
                captured.push((
                    origin.source.join,
                    positions[start + origin.current_predecessor.len() - 1],
                ));
                true
            });
        captured
    }

    pub(super) fn after_retained_setup_truth_locations(
        &mut self,
        captured: &[(Label, ItemPosition)],
        changes: &[(ItemPosition, Instruction, Instruction)],
    ) {
        if self
            .exception_owner_provenance
            .retained_setup_truths
            .is_empty()
        {
            return;
        }
        let items = self.graph.copy_items();
        self.exception_owner_provenance
            .retained_setup_truths
            .retain_mut(|origin| {
                let matching = captured
                    .iter()
                    .filter(|(join, _)| *join == origin.source.join)
                    .collect::<Vec<_>>();
                let [(_, position)] = matching.as_slice() else {
                    return false;
                };
                let matching = changes
                    .iter()
                    .filter(|(at, _, _)| at == position)
                    .collect::<Vec<_>>();
                let [(at, before, after)] = matching.as_slice() else {
                    return false;
                };
                let mut expected = *before;
                expected.location = after.location;
                if *before != origin.current_exit
                    || after.location.line < 0
                    || *after != expected
                    || self.graph.item(*at) != &Item::Instruction(*after)
                {
                    return false;
                }
                let Some(Item::Instruction(value)) = origin.current_predecessor.last_mut() else {
                    return false;
                };
                if *value != *before {
                    return false;
                }
                *value = *after;
                origin.current_exit = *after;
                origin.records(&items)
            });
    }

    pub(super) fn transport_retained_setup_truth_pop(
        &mut self,
        removed: &[ItemPosition],
        donations: &[(ItemPosition, u16)],
    ) {
        if self
            .exception_owner_provenance
            .retained_setup_truths
            .is_empty()
        {
            return;
        }
        let items = self.graph.iter_items().copied().collect::<Vec<_>>();
        let positions = self
            .graph
            .iter()
            .flat_map(|(block, data)| {
                (0..data.items().len()).map(move |offset| ItemPosition::new(block, offset))
            })
            .collect::<Vec<_>>();
        self.exception_owner_provenance
            .retained_setup_truths
            .retain_mut(|origin| {
                if origin.removed {
                    return origin.records(&items);
                }
                if !origin.records(&items) {
                    return false;
                }
                let Some(start) = anchored_start(&items, &origin.current) else {
                    return false;
                };
                let pop = positions[start + origin.pop];
                let setup = positions[start + origin.pop + 2];
                if !removed.contains(&pop) {
                    return true;
                }
                if donations
                    .iter()
                    .filter(|(position, _)| *position == setup)
                    .copied()
                    .collect::<Vec<_>>()
                    != [(setup, 1)]
                    || removed.iter().any(|position| {
                        positions[start..start + origin.current.len()].contains(position)
                            && *position != pop
                    })
                {
                    return false;
                }
                let Item::Instruction(value) = &mut origin.current[origin.pop + 2] else {
                    return false;
                };
                value.local_policy.add_hidden_exception_owner_slots(1);
                origin.current.remove(origin.pop);
                origin.removed = true;
                true
            });
    }
}

impl LayoutAssembler {
    pub(super) fn retained_setup_truth_null_markers(&self) -> FxHashSet<usize> {
        let mut result = FxHashSet::default();
        for origin in &self.exception_owner_provenance.retained_setup_truths {
            if !origin.removed
                || !origin.borrow_validated
                || !origin.graph_validated
                || !origin.records(&self.items)
                || !origin.roles_match(&self.exception_owner_provenance, &self.exception_regions)
                || !origin
                    .regions
                    .as_ref()
                    .is_some_and(|regions| same_regions(regions, &self.exception_regions))
                || self
                    .exception_owner_provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| *label == origin.setup.2)
                    .copied()
                    .collect::<Vec<_>>()
                    != [(origin.setup.2, Some(origin.setup_location))]
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == origin.source.join)
                || !self.exception_regions.iter().any(|region| {
                    region.target == origin.handler && region.depth == 1 && !region.preserve_lasti
                })
            {
                continue;
            }
            if let Some(start) = anchored_start(&self.items, &origin.current) {
                let marker = start + origin.current.len() - 1;
                result.insert(
                    self.items[..marker]
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count(),
                );
            }
        }
        result
    }
}
