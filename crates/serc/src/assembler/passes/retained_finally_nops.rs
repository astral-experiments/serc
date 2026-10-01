use rustc_hash::{FxHashMap, FxHashSet};

use super::super::block_graph::{BlockGraph, ItemPosition};
use super::super::exception_owners::ExceptionOwnerProvenance;
use super::super::ir::BorrowCleanupCall;
use super::super::ir::ExceptionRegionPriority;
use super::super::{
    BorrowFallthrough, ExceptionPolicy, ExceptionRegion, Instruction, Item, Label, Operand,
    OptimizationPipeline, SourceLocation, ends_scope,
};
use crate::PythonVersion;
use crate::target::opcodes::{
    CALL, CALL_KW, GET_AWAITABLE, JUMP_BACKWARD, JUMP_FORWARD, LOAD_CONST, NOP, NOT_TAKEN, POP_TOP,
    RETURN_VALUE, SEND, YIELD_VALUE,
};

#[cfg(test)]
mod tests;

/// Original compound exits whose own physical NOP may reach a populated finalizer.
#[derive(Default)]
pub(super) struct RetainedFinallyNops {
    origins: Vec<Origin>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RegionStage {
    Threading,
    ExitCopy,
    Cold,
    Complete,
}

struct Origin {
    source: Label,
    source_head: Vec<Item>,
    jump: Instruction,
    target: Label,
    target_head: Vec<Item>,
    target_prefix: Vec<Instruction>,
    pop: Instruction,
    removed_pop: bool,
    before_cleanup: Option<Vec<Item>>,
    replayed_prefix: Option<Vec<Instruction>>,
    target_fence: Vec<Item>,
    regions: Vec<ExceptionRegion>,
    region_roles: Vec<RegionRole>,
    region_stage: RegionStage,
    region_pending: bool,
    head_metadata: Vec<(Label, HeadMetadata)>,
    receipt: Option<Instruction>,
    live: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegionRole {
    Endpoint {
        ends_source: bool,
        ends_target: bool,
        protects_source: bool,
    },
    ScopeFallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinallyEntry {
    Missing,
    Present(Option<BorrowCleanupCall>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeadMetadata {
    block: bool,
    source: bool,
    preserved: bool,
    unreachable: bool,
    alias: bool,
    original_nop: bool,
    jump_entry: bool,
    finally_entry: FinallyEntry,
    finally_jump_source: Option<bool>,
    omitted_count: Option<usize>,
    omitted_location: Option<SourceLocation>,
    forbidden: bool,
}

pub(super) struct HeadView<'a> {
    pub(super) provenance: &'a ExceptionOwnerProvenance,
    pub(super) preserved: &'a FxHashSet<Label>,
    pub(super) unreachable: &'a FxHashSet<Label>,
    pub(super) aliases: &'a FxHashSet<Label>,
    pub(super) original_nops: &'a FxHashSet<Label>,
    pub(super) jump_entries: &'a FxHashSet<Label>,
    pub(super) finally_entries: &'a FxHashMap<Label, Option<BorrowCleanupCall>>,
    pub(super) finally_jump_sources: &'a FxHashMap<Label, bool>,
}

impl<'a> HeadView<'a> {
    fn pipeline(pipeline: &'a OptimizationPipeline) -> Self {
        Self {
            provenance: &pipeline.exception_owner_provenance,
            preserved: &pipeline.preserved_block_boundaries,
            unreachable: &pipeline.borrow_unreachable_blocks,
            aliases: &pipeline.borrow_block_aliases,
            original_nops: &pipeline.original_nop_targets,
            jump_entries: &pipeline.borrow_jump_entry_labels,
            finally_entries: &pipeline.borrow_finally_entries,
            finally_jump_sources: &pipeline.borrow_finally_jump_sources,
        }
    }

    fn get(&self, label: Label) -> HeadMetadata {
        HeadMetadata {
            block: self.provenance.block_boundaries.contains(&label),
            source: self.provenance.source_boundaries.contains(&label),
            preserved: self.preserved.contains(&label),
            unreachable: self.unreachable.contains(&label),
            alias: self.aliases.contains(&label),
            original_nop: self.original_nops.contains(&label),
            jump_entry: self.jump_entries.contains(&label),
            finally_entry: self
                .finally_entries
                .get(&label)
                .copied()
                .map_or(FinallyEntry::Missing, FinallyEntry::Present),
            finally_jump_source: self.finally_jump_sources.get(&label).copied(),
            omitted_count: self.provenance.omitted_nop_counts.get(&label).copied(),
            omitted_location: self.provenance.omitted_nop_locations.get(&label).copied(),
            forbidden: forbidden_head(self.provenance, label),
        }
    }
}

fn positions(items: &[Item]) -> Option<FxHashMap<Label, usize>> {
    let mut positions = FxHashMap::default();
    for (index, item) in items.iter().enumerate() {
        if let Item::Label(label) = item
            && positions.insert(*label, index).is_some()
        {
            return None;
        }
    }
    Some(positions)
}

fn labels(head: &[Item]) -> impl Iterator<Item = Label> + '_ {
    head.iter().map(|item| match item {
        Item::Label(label) => *label,
        Item::Instruction(_) => unreachable!(),
    })
}

fn same_region(left: &ExceptionRegion, right: &ExceptionRegion) -> bool {
    left.start == right.start
        && left.end == right.end
        && left.target == right.target
        && left.depth == right.depth
        && left.preserve_lasti == right.preserve_lasti
        && left.is_scope_fallback == right.is_scope_fallback
        && match (left.priority, right.priority) {
            (
                ExceptionRegionPriority::OriginalRange(a, b),
                ExceptionRegionPriority::OriginalRange(c, d),
            ) => a == c && b == d,
            (
                ExceptionRegionPriority::Resolved { start: a, end: b },
                ExceptionRegionPriority::Resolved { start: c, end: d },
            ) => a == c && b == d,
            _ => false,
        }
}

fn same_head(items: &[Item], start: usize, head: &[Item]) -> bool {
    items.get(start..start + head.len()) == Some(head)
        && !start
            .checked_sub(1)
            .is_some_and(|index| matches!(items[index], Item::Label(_)))
        && matches!(items.get(start + head.len()), Some(Item::Instruction(_)))
}

fn forbidden_head(provenance: &ExceptionOwnerProvenance, label: Label) -> bool {
    provenance.fusion_boundaries.contains(&label)
        || provenance.continuation_boundaries.contains(&label)
        || provenance
            .value_jump_continuation_boundaries
            .contains(&label)
        || provenance.setup_nop_boundaries.contains(&label)
        || provenance.borrow_only_block_boundaries.contains(&label)
        || provenance.normal_try_copy_inline_labels.contains(&label)
        || provenance.normal_try_copy_internal_labels.contains(&label)
        || provenance
            .normal_try_copy_jump_nop_successors
            .contains(&label)
        || provenance.retired_normal_try_copy_labels.contains(&label)
}

fn relevant_regions(
    items: &[Item],
    regions: &[ExceptionRegion],
    source_head: &[Item],
    target_head: &[Item],
    target_length: usize,
) -> Option<Vec<ExceptionRegion>> {
    let positions = positions(items)?;
    let source_start = *positions.get(&labels(source_head).next()?)?;
    let source_end = source_start + source_head.len() + 1;
    let target_start = *positions.get(&labels(target_head).next()?)?;
    let target_end = target_start + target_length;
    let heads = labels(source_head)
        .chain(labels(target_head))
        .collect::<FxHashSet<_>>();
    let mut relevant = Vec::new();
    for region in regions {
        positions.get(&region.target)?;
        let (Some(&start), Some(&end)) = (positions.get(&region.start), positions.get(&region.end))
        else {
            return None;
        };
        let low = start.min(end);
        let high = start.max(end);
        if heads.contains(&region.start)
            || heads.contains(&region.end)
            || heads.contains(&region.target)
            || (low < source_end && high > source_start)
            || (low < target_end && high > target_start)
        {
            relevant.push(*region);
        }
    }
    Some(relevant)
}

fn regions_match(original: &[ExceptionRegion], current: &[ExceptionRegion]) -> bool {
    original.len() == current.len()
        && original
            .iter()
            .zip(current)
            .all(|(left, right)| same_region(left, right))
}

fn head_end_region_roles(
    items: &[Item],
    regions: &[ExceptionRegion],
    source_head: &[Item],
    target_head: &[Item],
    target_length: usize,
) -> Option<Vec<RegionRole>> {
    let positions = positions(items)?;
    let source = labels(source_head)
        .next()
        .and_then(|label| positions.get(&label))
        .copied()?;
    let target = labels(target_head)
        .next()
        .and_then(|label| positions.get(&label))
        .copied()?;
    let source_instruction = source + source_head.len();
    let target_instruction = target + target_head.len();
    regions
        .iter()
        .map(|region| {
            let start = *positions.get(&region.start)?;
            let end = *positions.get(&region.end)?;
            let handler = *positions.get(&region.target)?;
            if region.is_scope_fallback {
                return (region.depth == 0
                    && region.preserve_lasti
                    && start < source
                    && source_instruction < target
                    && target + target_length <= end
                    && end <= handler)
                    .then_some(RegionRole::ScopeFallback);
            }
            (start <= end
                && (source_head.contains(&Item::Label(region.end))
                    || target_head.contains(&Item::Label(region.end)))
                && !source_head.contains(&Item::Label(region.start))
                && !target_head.contains(&Item::Label(region.start))
                && !source_head.contains(&Item::Label(region.target))
                && !target_head.contains(&Item::Label(region.target))
                && !(start < target + target_length && end > target_instruction))
                .then_some(RegionRole::Endpoint {
                    ends_source: source_head.contains(&Item::Label(region.end)),
                    ends_target: target_head.contains(&Item::Label(region.end)),
                    protects_source: start <= source_instruction && source_instruction < end,
                })
        })
        .collect()
}

impl Origin {
    fn original_spans_match(&self, pipeline: &OptimizationPipeline, items: &[Item]) -> bool {
        let Some(positions) = positions(items) else {
            return false;
        };
        let Some(source_block) = pipeline.graph.label_block(self.source) else {
            return false;
        };
        let Some(target_block) = pipeline.graph.label_block(self.target) else {
            return false;
        };
        let source = pipeline.graph[source_block].items();
        let target = pipeline.graph[target_block].items();
        let Some((Item::Instruction(jump), head)) = source.split_last() else {
            return false;
        };
        let mut resolved_jump = self.jump;
        resolved_jump.jump_policy.resolve_deferred_removal();
        if head != self.source_head
            || (*jump != self.jump && *jump != resolved_jump)
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(*jump))
                .count()
                != 1
            || positions[&self.source] >= positions[&self.target]
            || !same_head(
                items,
                positions[&labels(&self.source_head).next().unwrap()],
                &self.source_head,
            )
            || !same_head(target, 0, &self.target_head)
            || target.get(self.target_head.len()) != Some(&Item::Instruction(self.pop))
            || !self
                .head_metadata
                .iter()
                .all(|(label, metadata)| HeadView::pipeline(pipeline).get(*label) == *metadata)
        {
            return false;
        }
        let provenance = &pipeline.exception_owner_provenance;
        if provenance
            .compound_try_owner_exits
            .iter()
            .filter(|tuple| **tuple == (self.source, self.target))
            .count()
            != 1
            || provenance
                .compound_try_owner_exits
                .iter()
                .filter(|(entry, destination)| {
                    self.source_head.contains(&Item::Label(*entry))
                        || self.target_head.contains(&Item::Label(*destination))
                })
                .count()
                != 1
        {
            return false;
        }
        let suffix = &target[self.target_head.len() + 1..];
        super::locals::retained_borrow_prefix_length(&self.target_prefix, suffix, true)
            .is_some_and(|length| suffix.get(length..) == Some(self.target_fence.as_slice()))
    }
}

impl RetainedFinallyNops {
    pub(super) fn capture(pipeline: &OptimizationPipeline) -> Self {
        let mut result = Self::default();
        if pipeline.python_version != PythonVersion::Py314 {
            return result;
        }
        let items = pipeline.graph.copy_items();
        let Some(positions) = positions(&items) else {
            return result;
        };
        for (_, block) in pipeline.graph.iter() {
            let Some((Item::Instruction(jump), source_head)) = block.items().split_last() else {
                continue;
            };
            if source_head.is_empty()
                || !source_head
                    .iter()
                    .all(|item| matches!(item, Item::Label(_)))
            {
                continue;
            }
            let Operand::Forward(target) = jump.operand() else {
                continue;
            };
            let Some(depth) = jump.depth_after() else {
                continue;
            };
            let mut expected = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                jump.location,
                Some(depth),
            );
            expected.jump_policy.mark_original_empty_join();
            expected.jump_policy.prevent_small_exit_inlining();
            expected.jump_policy.retain_redundant_nop_after_threading();
            expected
                .no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
            if *jump != expected
                || jump.location.line < 0
                || items
                    .iter()
                    .filter(|item| **item == Item::Instruction(*jump))
                    .count()
                    != 1
            {
                continue;
            }
            let Some(Item::Label(source)) = source_head.last() else {
                continue;
            };
            let provenance = &pipeline.exception_owner_provenance;
            if provenance
                .compound_try_owner_exits
                .iter()
                .filter(|origin| **origin == (*source, target))
                .count()
                != 1
                || labels(source_head)
                    .filter(|label| provenance.block_boundaries.contains(label))
                    .count()
                    != 1
                || labels(source_head)
                    .filter(|label| provenance.source_boundaries.contains(label))
                    .count()
                    != 1
                || !labels(source_head).any(|label| {
                    provenance.block_boundaries.contains(&label)
                        && provenance.source_boundaries.contains(&label)
                })
                || labels(source_head).any(|label| {
                    pipeline.preserved_block_boundaries.contains(&label)
                        || pipeline.borrow_unreachable_blocks.contains(&label)
                        || pipeline.borrow_block_aliases.contains(&label)
                        || pipeline.original_nop_targets.contains(&label)
                        || provenance.omitted_nop_counts.contains_key(&label)
                        || provenance.omitted_nop_locations.contains_key(&label)
                        || forbidden_head(provenance, label)
                        || pipeline
                            .exception_regions
                            .iter()
                            .any(|region| region.target == label)
                })
            {
                continue;
            }
            let Some(target_block) = pipeline.graph.label_block(target) else {
                continue;
            };
            let target_items = pipeline.graph[target_block].items();
            let target_head_length = target_items
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
            let (target_head, records) = target_items.split_at(target_head_length);
            if target_head.is_empty()
                || !target_head.contains(&Item::Label(target))
                || positions[source] >= positions[&target]
                || !same_head(
                    &items,
                    positions[&labels(source_head).next().unwrap()],
                    source_head,
                )
                || !same_head(
                    &items,
                    positions[&labels(target_head).next().unwrap()],
                    target_head,
                )
                || !provenance.block_boundaries.contains(&target)
                || labels(target_head)
                    .filter(|label| provenance.block_boundaries.contains(label))
                    .count()
                    != 1
                || !labels(target_head)
                    .any(|label| pipeline.borrow_finally_entries.contains_key(&label))
                || labels(target_head).any(|label| {
                    pipeline.preserved_block_boundaries.contains(&label)
                        || pipeline.borrow_block_aliases.contains(&label)
                        || pipeline.original_nop_targets.contains(&label)
                        || provenance.omitted_nop_counts.contains_key(&label)
                        || provenance.omitted_nop_locations.contains_key(&label)
                        || forbidden_head(provenance, label)
                        || (pipeline.borrow_unreachable_blocks.contains(&label)
                            && (!pipeline.borrow_finally_entries.contains_key(&label)
                                || !pipeline.borrow_jump_entry_labels.contains(&label)))
                        || pipeline
                            .exception_regions
                            .iter()
                            .any(|region| region.target == label)
                })
            {
                continue;
            }
            let Some(Item::Instruction(pop)) = records.first() else {
                continue;
            };
            let mut expected_pop =
                Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(depth));
            expected_pop.fusion_policy.mark_converted_pop_block();
            expected_pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            if pop.exception_policy() == ExceptionPolicy::AlwaysAbsent {
                expected_pop.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
            }
            if *pop != expected_pop {
                continue;
            }
            let Some(last_pop) = records.iter().rposition(|item| {
                matches!(item,
                Item::Instruction(instruction) if instruction.opcode == POP_TOP)
            }) else {
                continue;
            };
            if last_pop == 0 || last_pop + 1 == records.len() {
                continue;
            }
            let mut prefix = Vec::new();
            for item in &records[1..=last_pop] {
                let Item::Instruction(instruction) = item else {
                    break;
                };
                if !matches!(instruction.operand(), Operand::Value(_))
                    || matches!(
                        instruction.opcode,
                        NOP | NOT_TAKEN | GET_AWAITABLE | SEND | YIELD_VALUE
                    )
                    || ends_scope(instruction.opcode)
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.local_policy.hidden_exception_owner_slots() != 0
                    || instruction.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::Reachable
                    || instruction.jump_policy.borrow_empty_block()
                {
                    break;
                }
                prefix.push(*instruction);
            }
            if prefix.len() != last_pop
                || !prefix
                    .iter()
                    .any(|record| matches!(record.opcode, CALL | CALL_KW))
            {
                continue;
            }
            let fence = &records[last_pop + 1..];
            let valid_fence = match fence {
                [Item::Instruction(jump)] => {
                    matches!(jump.opcode, JUMP_FORWARD | JUMP_BACKWARD)
                        && matches!(jump.operand(), Operand::Forward(_) | Operand::Backward(_))
                        && jump.depth_after() == Some(depth)
                }
                [Item::Instruction(load), Item::Instruction(ret)] => {
                    let location = prefix.last().unwrap().location;
                    let mut expected_return =
                        Instruction::new(RETURN_VALUE, Operand::Value(0), location, Some(depth));
                    expected_return.add_pre_fold_extra_instructions(1);
                    matches!(load.operand(), Operand::Value(_))
                        && *load
                            == Instruction::new(
                                LOAD_CONST,
                                load.operand(),
                                location,
                                Some(depth + 1),
                            )
                        && *ret == expected_return
                }
                _ => false,
            };
            if !valid_fence || fence.iter().any(|item| matches!(item, Item::Instruction(record)
                if (record.opcode != RETURN_VALUE && record.pre_fold_extra_instructions() != 0) || record.local_policy.hidden_exception_owner_slots() != 0
                    || record.jump_policy.borrow_empty_block() || record.fusion_policy.is_converted_pop_block()))
                || provenance.compound_try_owner_exits.iter().filter(|(entry, destination)|
                    source_head.contains(&Item::Label(*entry)) || target_head.contains(&Item::Label(*destination))).count() != 1
            { continue; }
            let Some(regions) = relevant_regions(
                &items,
                &pipeline.exception_regions,
                source_head,
                target_head,
                target_items.len(),
            ) else {
                continue;
            };
            if regions.is_empty()
                || regions
                    .iter()
                    .filter(|region| region.is_scope_fallback)
                    .count()
                    > 1
                || regions
                    .iter()
                    .any(|region| region.is_scope_fallback && region.end != region.target)
            {
                continue;
            }
            let Some(region_roles) = head_end_region_roles(
                &items,
                &regions,
                source_head,
                target_head,
                target_items.len(),
            ) else {
                continue;
            };
            result.origins.push(Origin {
                source: *source,
                source_head: source_head.to_vec(),
                jump: *jump,
                target,
                target_head: target_head.to_vec(),
                target_prefix: prefix,
                pop: *pop,
                removed_pop: false,
                before_cleanup: None,
                replayed_prefix: None,
                target_fence: fence.to_vec(),
                regions,
                region_roles,
                region_stage: RegionStage::Threading,
                region_pending: false,
                head_metadata: labels(source_head)
                    .chain(labels(target_head))
                    .map(|label| (label, HeadView::pipeline(pipeline).get(label)))
                    .collect(),
                receipt: None,
                live: true,
            });
        }
        result
    }

    pub(super) fn before_region_rewrite(
        &mut self,
        pipeline: &OptimizationPipeline,
        stage: RegionStage,
    ) -> bool {
        if self.origins.is_empty() {
            return false;
        }
        let items = pipeline.graph.copy_items();
        for origin in &mut self.origins {
            origin.live &= !origin.region_pending
                && origin.region_stage == stage
                && origin.original_spans_match(pipeline, &items)
                && origin.regions.iter().all(|original| {
                    pipeline
                        .exception_regions
                        .iter()
                        .filter(|region| same_region(original, region))
                        .count()
                        == 1
                })
                && pipeline
                    .graph
                    .label_block(origin.target)
                    .is_some_and(|block| {
                        relevant_regions(
                            &items,
                            &pipeline.exception_regions,
                            &origin.source_head,
                            &origin.target_head,
                            pipeline.graph[block].items().len(),
                        )
                        .is_some_and(|regions| {
                            regions_match(&origin.regions, &regions)
                                && head_end_region_roles(
                                    &items,
                                    &regions,
                                    &origin.source_head,
                                    &origin.target_head,
                                    pipeline.graph[block].items().len(),
                                )
                                .as_ref()
                                    == Some(&origin.region_roles)
                        })
                    });
            origin.region_pending = origin.live;
        }
        self.origins.iter().any(|origin| origin.live)
    }

    pub(super) fn after_region_rewrite(
        &mut self,
        pipeline: &OptimizationPipeline,
        retargets: &[(ExceptionRegion, ExceptionRegion)],
        stage: RegionStage,
    ) {
        if self.origins.is_empty() {
            return;
        }
        let items = pipeline.graph.copy_items();
        for origin in &mut self.origins {
            if !std::mem::take(&mut origin.region_pending) || origin.region_stage != stage {
                origin.live = false;
                continue;
            }
            origin.region_stage = match stage {
                RegionStage::Threading => RegionStage::ExitCopy,
                RegionStage::ExitCopy => RegionStage::Cold,
                RegionStage::Cold => RegionStage::Complete,
                RegionStage::Complete => {
                    origin.live = false;
                    continue;
                }
            };
            if !origin.original_spans_match(pipeline, &items) {
                origin.live = false;
                continue;
            }
            let Some(block) = pipeline.graph.label_block(origin.target) else {
                origin.live = false;
                continue;
            };
            let length = pipeline.graph[block].items().len();
            let mut transferred = Vec::new();
            let mut previous_output = None;
            for (original, role) in origin.regions.iter().zip(&origin.region_roles) {
                let mut rebuilt_group = Vec::new();
                let group = retargets
                    .iter()
                    .filter(|(before, _)| same_region(original, before))
                    .map(|(_, after)| *after)
                    .collect::<Vec<_>>();
                if group.is_empty() {
                    let matches = pipeline
                        .exception_regions
                        .iter()
                        .enumerate()
                        .filter(|(_, region)| same_region(original, region))
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    if matches.len() != 1
                        || previous_output.is_some_and(|index| index >= matches[0])
                    {
                        origin.live = false;
                        break;
                    }
                    previous_output = Some(matches[0]);
                    rebuilt_group.push(*original);
                } else {
                    for rebuilt in group {
                        let mut shape = rebuilt;
                        shape.start = original.start;
                        shape.end = original.end;
                        let matches = pipeline
                            .exception_regions
                            .iter()
                            .enumerate()
                            .filter(|(_, region)| same_region(&rebuilt, region))
                            .map(|(index, _)| index)
                            .collect::<Vec<_>>();
                        if !same_region(&shape, original)
                            || matches.len() != 1
                            || previous_output.is_some_and(|index| index >= matches[0])
                        {
                            origin.live = false;
                            break;
                        }
                        previous_output = Some(matches[0]);
                        rebuilt_group.push(rebuilt);
                    }
                }
                let Some(related) = relevant_regions(
                    &items,
                    &rebuilt_group,
                    &origin.source_head,
                    &origin.target_head,
                    length,
                ) else {
                    origin.live = false;
                    break;
                };
                if related.len() != 1
                    || head_end_region_roles(
                        &items,
                        &related,
                        &origin.source_head,
                        &origin.target_head,
                        length,
                    )
                    .as_deref()
                        != Some(std::slice::from_ref(role))
                {
                    origin.live = false;
                    break;
                }
                transferred.extend(related);
            }
            let expected = relevant_regions(
                &items,
                &transferred,
                &origin.source_head,
                &origin.target_head,
                length,
            );
            let actual = relevant_regions(
                &items,
                &pipeline.exception_regions,
                &origin.source_head,
                &origin.target_head,
                length,
            );
            match (expected, actual) {
                (Some(expected), Some(actual))
                    if origin.live
                        && regions_match(&expected, &actual)
                        && head_end_region_roles(
                            &items,
                            &actual,
                            &origin.source_head,
                            &origin.target_head,
                            length,
                        )
                        .as_ref()
                            == Some(&origin.region_roles) =>
                {
                    origin.regions = expected;
                }
                _ => origin.live = false,
            }
        }
    }

    /// Keeps the omitted leading POP tied to its real NOP cleanup, not a later guess.
    pub(super) fn observe_pop_cleanup(&mut self, pipeline: &OptimizationPipeline, after: bool) {
        for origin in &mut self.origins {
            let Some(block) = pipeline.graph.label_block(origin.target) else {
                origin.live = false;
                continue;
            };
            let items = pipeline.graph[block].items();
            let index = origin.target_head.len();
            if !after {
                let length = items.get(index + 1..).and_then(|suffix| {
                    super::locals::retained_borrow_prefix_length(
                        &origin.target_prefix,
                        suffix,
                        true,
                    )
                });
                origin.live &= origin.before_cleanup.is_none()
                    && same_head(items, 0, &origin.target_head)
                    && items.get(index) == Some(&Item::Instruction(origin.pop))
                    && length.is_some_and(|length| {
                        items.get(index + 1 + length..) == Some(origin.target_fence.as_slice())
                    });
                if origin.live {
                    origin.before_cleanup = Some(items.to_vec());
                }
            } else {
                let Some(mut expected) = origin.before_cleanup.take() else {
                    origin.live = false;
                    continue;
                };
                expected.remove(index);
                let Item::Instruction(first) = &mut expected[index] else {
                    origin.live = false;
                    continue;
                };
                if first.local_policy.hidden_exception_owner_slots() != 0 {
                    origin.live = false;
                    continue;
                }
                first.local_policy.add_hidden_exception_owner_slots(1);
                origin.live &= items == expected;
                origin.removed_pop = origin.live;
                if origin.live {
                    let prefix_end = expected.len() - origin.target_fence.len();
                    origin.replayed_prefix = Some(
                        expected[index..prefix_end]
                            .iter()
                            .map(|item| match item {
                                Item::Instruction(record) => *record,
                                Item::Label(_) => unreachable!(),
                            })
                            .collect(),
                    );
                }
            }
        }
    }

    /// Seals only an observed rewrite after the existing pass's final owner mutation.
    pub(super) fn record_rewrite(
        &mut self,
        graph: &BlockGraph,
        position: ItemPosition,
        before: Instruction,
        after: Instruction,
    ) {
        if self.origins.is_empty() {
            return;
        }
        let items = graph.copy_items();
        let Some(positions) = positions(&items) else {
            return;
        };
        for origin in &mut self.origins {
            let mut expected = origin.jump;
            expected.jump_policy.resolve_deferred_removal();
            if before != expected {
                continue;
            }
            expected.opcode = NOP;
            expected.set_operand(Operand::Value(0));
            let Some(start) = labels(&origin.source_head)
                .next()
                .and_then(|label| positions.get(&label))
                .copied()
            else {
                continue;
            };
            if origin.receipt.is_some()
                || after != expected
                || graph.item(position) != &Item::Instruction(after)
                || !same_head(&items, start, &origin.source_head)
                || items.get(start + origin.source_head.len()) != Some(&Item::Instruction(after))
                || items
                    .iter()
                    .filter(|item| **item == Item::Instruction(after))
                    .count()
                    != 1
                || items.contains(&Item::Instruction(before))
            {
                origin.live = false;
                continue;
            }
            origin.receipt = Some(after);
        }
    }

    pub(super) fn revalidate(&mut self, pipeline: &OptimizationPipeline) {
        if self.origins.is_empty() {
            return;
        }
        let reachable = pipeline.graph.reachable_blocks(&pipeline.exception_regions);
        let items = pipeline.graph.copy_items();
        for origin in &mut self.origins {
            origin.live &= origin.removed_pop
                && origin.region_stage == RegionStage::Complete
                && !origin.region_pending
                && pipeline
                    .graph
                    .label_block(origin.source)
                    .is_some_and(|block| reachable.get(block.index()).copied() == Some(true))
                && pipeline
                    .graph
                    .label_block(origin.target)
                    .is_some_and(|block| reachable.get(block.index()).copied() == Some(true))
                && pipeline
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .iter()
                    .filter(|(entry, destination)| {
                        origin.source_head.contains(&Item::Label(*entry))
                            || origin.target_head.contains(&Item::Label(*destination))
                    })
                    .count()
                    == 1
                && origin
                    .head_metadata
                    .iter()
                    .all(|(label, metadata)| HeadView::pipeline(pipeline).get(*label) == *metadata)
                && pipeline
                    .graph
                    .label_block(origin.target)
                    .is_some_and(|block| {
                        relevant_regions(
                            &items,
                            &pipeline.exception_regions,
                            &origin.source_head,
                            &origin.target_head,
                            pipeline.graph[block].items().len(),
                        )
                        .is_some_and(|regions| {
                            regions_match(&origin.regions, &regions)
                                && head_end_region_roles(
                                    &items,
                                    &regions,
                                    &origin.source_head,
                                    &origin.target_head,
                                    pipeline.graph[block].items().len(),
                                )
                                .as_ref()
                                    == Some(&origin.region_roles)
                        })
                    })
                && labels(&origin.source_head).all(|label| {
                    !pipeline.borrow_unreachable_blocks.contains(&label)
                        && !pipeline.preserved_block_boundaries.contains(&label)
                        && !pipeline.borrow_block_aliases.contains(&label)
                });
        }
    }

    /// Replays after analysis-only markers have been inserted into the final items.
    pub(super) fn entries(
        &self,
        items: &[Item],
        regions: &[ExceptionRegion],
        head_view: &HeadView<'_>,
    ) -> FxHashMap<Label, Label> {
        if self.origins.is_empty() {
            return FxHashMap::default();
        }
        let provenance = head_view.provenance;
        let mut result = FxHashMap::default();
        let Some(positions) = positions(items) else {
            return result;
        };
        for origin in &self.origins {
            let Some(nop) = origin.receipt.filter(|_| origin.live) else {
                continue;
            };
            let Some(start) = labels(&origin.source_head)
                .next()
                .and_then(|label| positions.get(&label))
                .copied()
            else {
                continue;
            };
            if !same_head(items, start, &origin.source_head)
                || items.get(start + origin.source_head.len()) != Some(&Item::Instruction(nop))
                || items
                    .iter()
                    .filter(|item| **item == Item::Instruction(nop))
                    .count()
                    != 1
                || provenance
                    .compound_try_owner_exits
                    .iter()
                    .filter(|tuple| **tuple == (origin.source, origin.target))
                    .count()
                    != 1
            {
                continue;
            }
            let target_start = start + origin.source_head.len() + 1;
            if !same_head(items, target_start, &origin.target_head) {
                continue;
            }
            let prefix_start = target_start + origin.target_head.len();
            let Some(prefix) = &origin.replayed_prefix else {
                continue;
            };
            let Some(length) =
                super::locals::retained_borrow_prefix_length(prefix, &items[prefix_start..], true)
            else {
                continue;
            };
            let end = prefix_start + length;
            if items.get(end..end + origin.target_fence.len())
                != Some(origin.target_fence.as_slice())
            {
                continue;
            }
            if items
                .get(end + origin.target_fence.len())
                .is_some_and(|item| !matches!(item, Item::Label(_)))
            {
                continue;
            }
            if !origin
                .head_metadata
                .iter()
                .all(|(label, metadata)| head_view.get(*label) == *metadata)
                || !relevant_regions(
                    items,
                    regions,
                    &origin.source_head,
                    &origin.target_head,
                    origin.target_head.len() + length + origin.target_fence.len(),
                )
                .is_some_and(|regions| {
                    regions_match(&origin.regions, &regions)
                        && head_end_region_roles(
                            items,
                            &regions,
                            &origin.source_head,
                            &origin.target_head,
                            origin.target_head.len() + length + origin.target_fence.len(),
                        )
                        .as_ref()
                            == Some(&origin.region_roles)
                })
            {
                continue;
            }
            result.insert(origin.source, origin.target);
        }
        result
    }
}
