use rustc_hash::{FxHashMap, FxHashSet};

use super::block_graph::{BlockGraph, ItemPosition};
use super::ir::BorrowCleanupCall;
use super::{
    Assembler, BorrowFallthrough, EmittedJump, ExceptionPolicy, Instruction, Item, Label, Operand,
    OptimizationPipeline, SourceLocation, UnoptimizedCode,
};
use crate::target::opcodes::{
    JUMP_FORWARD, LOAD_CONST, NOP, POP_JUMP_IF_NONE, POP_TOP, RAISE_VARARGS, RETURN_VALUE,
};

#[cfg(test)]
mod tests;

/// A sole normal guard exit whose separate original POP owns its physical line event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Source {
    head: Vec<Item>,
    jump: Instruction,
    condition: Instruction,
    raised: Instruction,
    target: Label,
    destination: Option<Destination>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Destination {
    head: Vec<Item>,
    prefix: Vec<Item>,
    original_pops: Vec<PopOccurrence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PopOccurrence {
    head: Vec<Item>,
    record: Instruction,
}

pub(super) struct BorrowRoute {
    index: usize,
    origins: usize,
    condition_index: usize,
    emitted: EmittedJump,
    source: Source,
}

impl Source {
    fn matches_emitted(&self, assembler: &Assembler, emitted: EmittedJump) -> bool {
        let UnoptimizedCode::Linear(items) = &assembler.code else {
            return false;
        };
        let Ok(index) = usize::try_from(emitted.instruction().item_index) else {
            return false;
        };
        let Some(positions) = positions(items) else {
            return false;
        };
        let Operand::Forward(entry) = self.condition.operand() else {
            return false;
        };
        emitted.instruction().stream == assembler.emission_stream
            && self.destination.is_none()
            && index + 1 == items.len()
            && head_index(items, &positions, &self.head) == Some(index)
            && items.get(index) == Some(&Item::Instruction(self.jump))
            && index
                .checked_sub(self.head.len() + 1)
                .and_then(|previous| items.get(previous))
                == Some(&Item::Instruction(self.raised))
            && [self.jump, self.condition, self.raised]
                .into_iter()
                .all(|record| unique(items, record))
            && incoming_source_is_original(items, self)
            && assembler
                .exception_owner_provenance
                .source_boundaries
                .contains(&entry)
            && assembler
                .exception_owner_provenance
                .block_boundaries
                .contains(&entry)
            && !labels(&self.head).any(|label| {
                assembler.borrow_block_aliases.contains(&label)
                    || assembler.original_nop_targets.contains(&label)
                    || assembler.preserved_block_boundaries.contains(&label)
                    || assembler
                        .exception_regions
                        .iter()
                        .any(|region| region.target == label)
            })
    }

    fn visit_records_mut(&mut self, visit: &mut impl FnMut(&mut Instruction)) {
        visit(&mut self.jump);
        visit(&mut self.condition);
        visit(&mut self.raised);
        if let Some(target) = &mut self.destination {
            for original in &mut target.original_pops {
                visit(&mut original.record);
            }
            for item in &mut target.prefix {
                if let Item::Instruction(record) = item {
                    visit(record);
                }
            }
        }
    }

    pub(super) fn remap_constants(
        &mut self,
        indices: &[Option<u32>],
        deferred_none: Option<u32>,
    ) -> bool {
        let mut valid = true;
        self.visit_records_mut(&mut |record| {
            if record.opcode != LOAD_CONST {
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
            if let Some(Some(mapped)) = index.and_then(|index| indices.get(index as usize)) {
                record.set_operand(Operand::Value(*mapped));
            } else {
                valid = false;
            }
        });
        valid
    }
}

fn positions(items: &[Item]) -> Option<FxHashMap<Label, usize>> {
    let mut result = FxHashMap::default();
    for (index, item) in items.iter().enumerate() {
        if let Item::Label(label) = item
            && result.insert(*label, index).is_some()
        {
            return None;
        }
    }
    Some(result)
}

fn labels(head: &[Item]) -> impl Iterator<Item = Label> + '_ {
    head.iter().map(|item| match item {
        Item::Label(label) => *label,
        Item::Instruction(_) => unreachable!(),
    })
}

fn complete_head(items: &[Item], index: usize) -> &[Item] {
    let mut start = index;
    while start > 0 && matches!(items[start - 1], Item::Label(_)) {
        start -= 1;
    }
    &items[start..index]
}

fn head_index(items: &[Item], positions: &FxHashMap<Label, usize>, head: &[Item]) -> Option<usize> {
    let start = *positions.get(&labels(head).next()?)?;
    let end = start.checked_add(head.len())?;
    (items.get(start..end) == Some(head)
        && complete_head(items, end) == head
        && matches!(items.get(end), Some(Item::Instruction(_))))
    .then_some(end)
}

fn unique(items: &[Item], record: Instruction) -> bool {
    items
        .iter()
        .filter(|item| **item == Item::Instruction(record))
        .count()
        == 1
}

fn unique_prefix(items: &[Item], prefix: &[Item]) -> bool {
    !prefix.is_empty()
        && items
            .windows(prefix.len())
            .filter(|window| *window == prefix)
            .count()
            == 1
}

fn incoming_source_is_original(items: &[Item], source: &Source) -> bool {
    let mut incoming = items.iter().filter_map(|item| match item {
        Item::Instruction(record)
            if matches!(record.operand(),
            Operand::Forward(label) | Operand::Backward(label)
                if labels(&source.head).any(|entry| entry == label)) =>
        {
            Some(*record)
        }
        _ => None,
    });
    incoming.next() == Some(source.condition) && incoming.next().is_none()
}

fn original_pop_census(items: &[Item], pop: Instruction) -> Option<Vec<PopOccurrence>> {
    positions(items)?;
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| **item == Item::Instruction(pop))
        .map(|(index, _)| {
            let head = complete_head(items, index);
            (!head.is_empty()).then(|| PopOccurrence {
                head: head.to_vec(),
                record: pop,
            })
        })
        .collect()
}

fn pop_census_valid(items: &[Item], destination: &Destination) -> bool {
    let Some(original) = destination.original_pops.first() else {
        return false;
    };
    let Some(current) = original_pop_census(items, original.record) else {
        return false;
    };
    current
        .iter()
        .all(|record| destination.original_pops.contains(record))
}

impl Assembler {
    pub(super) fn capture_guarded_loop_borrow_route(
        &self,
        emitted: EmittedJump,
    ) -> Option<BorrowRoute> {
        let mut origins = self
            .exception_owner_provenance
            .guarded_loop_nops
            .iter()
            .enumerate()
            .filter(|(_, source)| {
                source.jump.jump_policy.allows_borrow_threading_target()
                    && source.matches_emitted(self, emitted)
            });
        let (index, source) = origins.next()?;
        let condition_index = self
            .linear_items()
            .iter()
            .position(|item| *item == Item::Instruction(source.condition))?;
        origins.next().is_none().then(|| BorrowRoute {
            index,
            origins: self.exception_owner_provenance.guarded_loop_nops.len(),
            condition_index,
            emitted,
            source: source.clone(),
        })
    }

    pub(super) fn seal_guarded_loop_borrow_route(&mut self, original: Option<BorrowRoute>) {
        let Some(original) = original else {
            return;
        };
        if self.exception_owner_provenance.guarded_loop_nops.len() != original.origins
            || self
                .exception_owner_provenance
                .guarded_loop_nops
                .get(original.index)
                != Some(&original.source)
            || self.linear_items().get(original.condition_index)
                != Some(&Item::Instruction(original.source.condition))
        {
            return;
        }
        let mut expected = original.source;
        expected.jump.jump_policy.prevent_borrow_threading_target();
        if expected.matches_emitted(self, original.emitted) {
            self.exception_owner_provenance.guarded_loop_nops[original.index] = expected;
        }
    }

    /// Captures only the borrowed-analysis form of a terminal normal-finally guard.
    pub(crate) fn record_guarded_loop_borrow_nop(&mut self, emitted: EmittedJump) {
        if self.python_version < crate::PythonVersion::Py314 {
            return;
        }
        if emitted.instruction().stream != self.emission_stream {
            return;
        }
        let Ok(index) = usize::try_from(emitted.instruction().item_index) else {
            return;
        };
        let UnoptimizedCode::Linear(items) = &self.code else {
            return;
        };
        if index + 1 != items.len() {
            return;
        }
        let Some(Item::Instruction(jump)) = items.get(index) else {
            return;
        };
        let Operand::Forward(target) = jump.operand() else {
            return;
        };
        let mut expected = Instruction::new(JUMP_FORWARD, jump.operand(), jump.location, Some(0));
        expected.jump_policy.mark_original_empty_join();
        expected.jump_policy.retain_redundant_nop_after_threading();
        expected.jump_policy.mark_borrow_empty_block();
        expected
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
        if *jump != expected || jump.location.line < 0 || !unique(items, *jump) {
            return;
        }
        let head = complete_head(items, index);
        if head.is_empty() || positions(items).is_none() {
            return;
        }
        let Some(previous) = index.checked_sub(head.len() + 1) else {
            return;
        };
        let Some(Item::Instruction(raised)) = items.get(previous) else {
            return;
        };
        let expected_raise =
            Instruction::new(RAISE_VARARGS, raised.operand(), raised.location, Some(0));
        if *raised != expected_raise
            || !matches!(raised.operand(), Operand::Value(0..=2))
            || raised.location.line < 0
            || !unique(items, *raised)
        {
            return;
        }
        let incoming = items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record)
                    if matches!(record.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if labels(head).any(|entry| entry == label)) =>
                {
                    Some(*record)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [condition] = incoming.as_slice() else {
            return;
        };
        let Operand::Forward(entry) = condition.operand() else {
            return;
        };
        let mut expected_condition = Instruction::new(
            POP_JUMP_IF_NONE,
            condition.operand(),
            jump.location,
            Some(0),
        );
        expected_condition
            .local_policy
            .add_hidden_exception_owner_slots(3);
        if *condition != expected_condition
            || condition.location != jump.location
            || condition.depth_after() != Some(0)
            || !unique(items, *condition)
            || !self
                .exception_owner_provenance
                .source_boundaries
                .contains(&entry)
            || !self
                .exception_owner_provenance
                .block_boundaries
                .contains(&entry)
            || labels(head).any(|label| {
                self.borrow_block_aliases.contains(&label)
                    || self.original_nop_targets.contains(&label)
                    || self.preserved_block_boundaries.contains(&label)
            })
        {
            return;
        }
        let origin = Source {
            head: head.to_vec(),
            jump: *jump,
            condition: *condition,
            raised: *raised,
            target,
            destination: None,
        };
        if self
            .exception_regions
            .iter()
            .any(|region| labels(head).any(|label| region.target == label))
        {
            return;
        }
        self.exception_owner_provenance
            .guarded_loop_nops
            .push(origin);
    }

    /// Binds the real normal-finally POP and completed linear call prefix at emission.
    pub(crate) fn record_guarded_loop_target_pop(&mut self, ordinal: usize) {
        if self.exception_owner_provenance.guarded_loop_nops.is_empty() {
            return;
        }
        if !matches!(self.code, UnoptimizedCode::Linear(_)) {
            return;
        }
        let Some(index) = self.instruction_item_index(ordinal) else {
            return;
        };
        let items = self.linear_items();
        let Some(Item::Instruction(pop)) = items.get(index) else {
            return;
        };
        let mut expected = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
        expected.fusion_policy.mark_converted_pop_block();
        expected.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        if *pop != expected || positions(items).is_none() {
            return;
        }
        let head = complete_head(items, index);
        let prefix = &items[index..];
        if head.is_empty()
            || prefix.len() <= 4
            || !unique_prefix(items, prefix)
            || !matches!(prefix.last(), Some(Item::Instruction(last)) if last.opcode == POP_TOP)
            || prefix.iter().any(|item| {
                !matches!(item, Item::Instruction(record)
                if matches!(record.operand(), Operand::Value(_)))
            })
        {
            return;
        }
        let Some(original_pops) = original_pop_census(items, *pop) else {
            return;
        };
        let target = Destination {
            head: head.to_vec(),
            prefix: prefix.to_vec(),
            original_pops,
        };
        for source in &mut self.exception_owner_provenance.guarded_loop_nops {
            if source.destination.is_none()
                && labels(&target.head).any(|label| label == source.target)
                && self
                    .exception_owner_provenance
                    .borrow_only_block_boundaries
                    .contains(&source.target)
            {
                source.destination = Some(target.clone());
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FinallyEntry {
    Missing,
    Present(Option<BorrowCleanupCall>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HeadMetadata {
    source: bool,
    block: bool,
    preserved: bool,
    alias: bool,
    unreachable: bool,
    original_nop: bool,
    jump_entry: bool,
    borrow_boundary: bool,
    finally_entry: FinallyEntry,
    finally_source: Option<bool>,
    omitted: Option<usize>,
    omitted_location: Option<SourceLocation>,
    forbidden: bool,
}

fn metadata(pipeline: &OptimizationPipeline, label: Label) -> HeadMetadata {
    let p = &pipeline.exception_owner_provenance;
    HeadMetadata {
        source: p.source_boundaries.contains(&label),
        block: p.block_boundaries.contains(&label),
        preserved: pipeline.preserved_block_boundaries.contains(&label),
        alias: pipeline.borrow_block_aliases.contains(&label),
        unreachable: pipeline.borrow_unreachable_blocks.contains(&label),
        original_nop: pipeline.original_nop_targets.contains(&label),
        jump_entry: pipeline.borrow_jump_entry_labels.contains(&label),
        borrow_boundary: p.borrow_only_block_boundaries.contains(&label),
        finally_entry: pipeline
            .borrow_finally_entries
            .get(&label)
            .copied()
            .map_or(FinallyEntry::Missing, FinallyEntry::Present),
        finally_source: pipeline.borrow_finally_jump_sources.get(&label).copied(),
        omitted: p.omitted_nop_counts.get(&label).copied(),
        omitted_location: p.omitted_nop_locations.get(&label).copied(),
        forbidden: p.fusion_boundaries.contains(&label)
            || p.continuation_boundaries.contains(&label)
            || p.value_jump_continuation_boundaries.contains(&label)
            || p.setup_nop_boundaries.contains(&label)
            || p.normal_try_copy_inline_labels.contains(&label)
            || p.normal_try_copy_internal_labels.contains(&label)
            || p.normal_try_copy_jump_nop_successors.contains(&label)
            || p.retired_normal_try_copy_labels.contains(&label),
    }
}

struct Origin {
    source: Source,
    current: Instruction,
    metadata: Vec<(Label, HeadMetadata)>,
    deferred: bool,
    donation_pending: bool,
    donated: bool,
    rewritten: bool,
    live: bool,
}

/// Pipeline-local receipts; none of these records authorize an earlier physical edit.
#[derive(Default)]
pub(super) struct GuardedLoopNops {
    origins: Vec<Origin>,
}

impl GuardedLoopNops {
    pub(super) fn revalidate_constants(&mut self, pipeline: &OptimizationPipeline) {
        if self.origins.is_empty() {
            return;
        }
        let items = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
        let Some(positions) = positions(&items) else {
            self.origins.clear();
            return;
        };
        for origin in &mut self.origins {
            if !origin.live {
                continue;
            }
            origin.live = origin.donated
                && (|| {
                    let destination = origin.source.destination.as_ref()?;
                    let source = head_index(&items, &positions, &origin.source.head)?;
                    let target = head_index(&items, &positions, &destination.head)?;
                    let Item::Instruction(pop) = destination.prefix[0] else {
                        return None;
                    };
                    (items[source] == Item::Instruction(origin.current)
                        && unique(&items, origin.current)
                        && unique(&items, pop)
                        && items.get(target..target + destination.prefix.len())
                            == Some(&destination.prefix)
                        && unique_prefix(&items, &destination.prefix))
                    .then_some(())
                })()
                .is_some();
        }
    }

    pub(super) fn lower_constants(
        &mut self,
        count: usize,
        lower: impl Fn(&mut Instruction) -> bool,
    ) {
        for origin in &mut self.origins {
            if !origin.live {
                continue;
            }
            for item in &mut origin.source.destination.as_mut().unwrap().prefix {
                let Item::Instruction(record) = item else {
                    unreachable!()
                };
                if record.opcode == LOAD_CONST
                    && !matches!(record.operand(), Operand::Value(index) if (index as usize) < count)
                {
                    origin.live = false;
                } else {
                    lower(record);
                }
            }
        }
    }

    pub(super) fn remap_constants(&mut self, indices: &[Option<u32>]) {
        for origin in &mut self.origins {
            if !origin.live {
                continue;
            }
            for item in &mut origin.source.destination.as_mut().unwrap().prefix {
                let Item::Instruction(record) = item else {
                    unreachable!()
                };
                if record.opcode == LOAD_CONST {
                    let Operand::Value(index) = record.operand() else {
                        origin.live = false;
                        continue;
                    };
                    if let Some(Some(mapped)) = indices.get(index as usize) {
                        record.set_operand(Operand::Value(*mapped));
                    } else {
                        origin.live = false;
                    }
                }
            }
        }
    }

    pub(super) fn capture(pipeline: &OptimizationPipeline) -> Self {
        if pipeline.python_version < crate::PythonVersion::Py314
            || pipeline
                .exception_owner_provenance
                .guarded_loop_nops
                .is_empty()
        {
            return Self::default();
        }
        let items = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
        let Some(positions) = positions(&items) else {
            return Self::default();
        };
        let origins = pipeline.exception_owner_provenance.guarded_loop_nops.iter().filter_map(|source| {
            let destination = source.destination.as_ref()?;
            if pipeline.exception_owner_provenance.guarded_loop_nops.iter()
                .filter(|other| other.head == source.head).count() != 1
                || pipeline.exception_regions.iter().any(|region|
                    labels(&source.head).chain(labels(&destination.head)).any(|label| label == region.target))
                || !incoming_source_is_original(&items, source)
            { return None; }
            let start = head_index(&items, &positions, &source.head)?;
            let end = head_index(&items, &positions, &destination.head)?;
            if start >= end || items.get(start) != Some(&Item::Instruction(source.jump))
                || !matches!(items.get(start + 1), Some(Item::Label(_)))
                || items.get(end..end + destination.prefix.len()) != Some(&destination.prefix)
                || !unique(&items, source.jump) || !unique(&items, source.condition)
                || !unique(&items, source.raised) || !unique_prefix(&items, &destination.prefix)
                || !pop_census_valid(&items, destination)
            { return None; }
            let metadata = labels(&source.head).chain(labels(&destination.head))
                .map(|label| (label, metadata(pipeline, label))).collect::<Vec<_>>();
            if metadata.iter().any(|(label, value)| value.borrow_boundary != (*label == source.target)
                || value.forbidden || value.omitted.is_some()
                || value.omitted_location.is_some() || value.alias || value.original_nop || value.preserved)
            { return None; }
            let mut source = source.clone();
            let length = items[end..].iter().take_while(|item| matches!(item, Item::Instruction(_))).count();
            let full_array = &items[end..end + length];
            if full_array.iter().any(|item| !matches!(item, Item::Instruction(record)
                if matches!(record.operand(), Operand::Value(_))))
                || !unique_prefix(&items, full_array)
                || !matches!(full_array.last(), Some(Item::Instruction(last)) if last.opcode == RETURN_VALUE)
            { return None; }
            source.destination.as_mut()?.prefix = full_array.to_vec();
            let current = source.jump;
            Some(Origin { source, current, metadata,
                deferred: false, donation_pending: false, donated: false, rewritten: false, live: true })
        }).collect();
        Self { origins }
    }

    pub(super) fn record_defer(
        &mut self,
        graph: &BlockGraph,
        position: ItemPosition,
        before: Instruction,
        after: Instruction,
    ) {
        for origin in &mut self.origins {
            if !origin.live || origin.current != before {
                continue;
            }
            let mut expected = before;
            expected.jump_policy.resolve_deferred_removal();
            origin.live = !origin.deferred
                && expected == after
                && actual_source(graph, position, origin, after);
            if origin.live {
                origin.current = after;
                origin.deferred = true;
            }
        }
    }

    pub(super) fn observe_donation(&mut self, pipeline: &OptimizationPipeline, after: bool) {
        if self.origins.is_empty() {
            return;
        }
        let items = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
        let Some(positions) = positions(&items) else {
            self.origins.clear();
            return;
        };
        for origin in &mut self.origins {
            if !origin.live {
                continue;
            }
            let destination = origin.source.destination.as_mut().unwrap();
            let Some(index) = head_index(&items, &positions, &destination.head) else {
                origin.live = false;
                continue;
            };
            let mut expected = destination.prefix.clone();
            if after {
                let Item::Instruction(pop) = &mut expected[0] else {
                    unreachable!()
                };
                pop.location = origin.source.jump.location;
            }
            origin.live = (!after || origin.donation_pending)
                && origin.deferred
                && items.get(index..index + expected.len()) == Some(&expected)
                && unique_prefix(&items, &expected)
                && pop_census_valid(&items, destination)
                && (!after || matches!(expected[0], Item::Instruction(pop) if unique(&items, pop)));
            if origin.live {
                destination.prefix = expected;
                origin.donation_pending = !after;
                origin.donated = after;
            }
        }
    }

    pub(super) fn record_nop(
        &mut self,
        graph: &BlockGraph,
        position: ItemPosition,
        before: Instruction,
        after: Instruction,
    ) {
        for origin in &mut self.origins {
            if !origin.live || origin.current != before {
                continue;
            }
            let mut expected = before;
            expected.opcode = NOP;
            expected.set_operand(Operand::Value(0));
            origin.live = origin.deferred
                && origin.donated
                && !origin.rewritten
                && expected == after
                && actual_source(graph, position, origin, after);
            if origin.live {
                origin.current = after;
                origin.rewritten = true;
            }
        }
    }

    pub(super) fn revalidate(&mut self, pipeline: &OptimizationPipeline) {
        if self.origins.is_empty() {
            return;
        }
        let items = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
        let reachable = pipeline.graph.reachable_blocks(&pipeline.exception_regions);
        for origin in &mut self.origins {
            origin.live &= origin.rewritten
                && origin.donated
                && origin
                    .metadata
                    .iter()
                    .all(|(label, expected)| metadata(pipeline, *label) == *expected)
                && !pipeline.exception_regions.iter().any(|region| {
                    origin
                        .metadata
                        .iter()
                        .any(|(label, _)| *label == region.target)
                })
                && surviving_pair(&items, origin).is_some_and(|(index, _)| {
                    let mut offset = 0;
                    for block_index in 0..pipeline.graph.len() {
                        let block = pipeline.graph.block_at(block_index);
                        let length = pipeline.graph[block].items().len();
                        if index < offset + length {
                            return reachable[block.index()];
                        }
                        offset += length;
                    }
                    false
                });
        }
    }

    pub(super) fn mark_before_borrow(
        &mut self,
        items: &mut [Item],
        aliases: &FxHashSet<Label>,
        finally_entries: &FxHashMap<Label, Option<BorrowCleanupCall>>,
    ) {
        for origin in &mut self.origins {
            if !origin.live
                || origin.metadata.iter().any(|(label, old)| {
                    aliases.contains(label) != old.alias
                        || finally_entries
                            .get(label)
                            .copied()
                            .map_or(FinallyEntry::Missing, FinallyEntry::Present)
                            != old.finally_entry
                })
            {
                continue;
            }
            let Some((index, _)) = surviving_pair(items, origin) else {
                continue;
            };
            let Item::Instruction(marker) = &mut items[index] else {
                unreachable!()
            };
            marker
                .no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved);
            origin.live = false;
        }
    }
}

fn actual_source(
    graph: &BlockGraph,
    position: ItemPosition,
    origin: &Origin,
    after: Instruction,
) -> bool {
    let items = graph.iter_items().copied().collect::<Vec<_>>();
    let Some(positions) = positions(&items) else {
        return false;
    };
    head_index(&items, &positions, &origin.source.head).is_some_and(|index| {
        items[index] == Item::Instruction(after)
            && graph.item(position) == &items[index]
            && complete_head(graph[position.block()].items(), position.offset())
                == origin.source.head
            && unique(&items, after)
    })
}

fn surviving_pair(items: &[Item], origin: &Origin) -> Option<(usize, usize)> {
    let positions = positions(items)?;
    let source = head_index(items, &positions, &origin.source.head)?;
    let destination = origin.source.destination.as_ref()?;
    let target = head_index(items, &positions, &destination.head)?;
    (source + 1 + destination.head.len() == target
        && items[source] == Item::Instruction(origin.current)
        && incoming_source_is_original(items, &origin.source)
        && origin.current.jump_policy.borrow_empty_block()
        && origin.current.no_location_policy.borrow_fallthrough()
            == BorrowFallthrough::RetainedNopReachable
        && items.get(target..target + destination.prefix.len()) == Some(&destination.prefix)
        && items
            .get(target + destination.prefix.len())
            .is_none_or(|item| matches!(item, Item::Label(_)))
        && unique(items, origin.current)
        && unique_prefix(items, &destination.prefix))
    .then_some(())?;
    let Item::Instruction(pop) = destination.prefix[0] else {
        return None;
    };
    unique(items, pop).then_some((source, target))
}
