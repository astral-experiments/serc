use rustc_hash::{FxHashMap, FxHashSet};

use super::block_graph::{BlockGraph, ItemPosition};
use super::exception_owners::ExceptionOwnerProvenance;
use super::ir::BorrowCleanupCall;
use super::{
    Assembler, BorrowFallthrough, EmittedJump, ExceptionRegion, Instruction, Item, Label, Operand,
    UnoptimizedCode,
};
use crate::target::is_conditional_jump;
use crate::target::opcodes::{JUMP_FORWARD, RAISE_VARARGS};

#[cfg(test)]
mod tests;

/// An original synchronous compound-try exit, not a future finalizer POP record.
#[derive(Clone, Debug)]
pub(super) struct Source {
    entry: Label,
    target: Label,
    head: Vec<Item>,
    boundaries: Vec<(bool, bool)>,
    jump: Instruction,
    guard: Instruction,
    raised: Instruction,
    normally_reachable: bool,
}

pub(super) struct Context<'a> {
    pub(super) provenance: &'a ExceptionOwnerProvenance,
    pub(super) regions: &'a [ExceptionRegion],
    pub(super) aliases: &'a FxHashSet<Label>,
    pub(super) preserved: &'a FxHashSet<Label>,
    pub(super) original_nops: &'a FxHashSet<Label>,
    pub(super) unreachable: &'a FxHashSet<Label>,
    pub(super) finally_entries: &'a FxHashMap<Label, Option<BorrowCleanupCall>>,
    pub(super) finally_sources: Option<&'a FxHashMap<Label, bool>>,
}

fn labels(head: &[Item]) -> impl Iterator<Item = Label> + '_ {
    head.iter().map(|item| match item {
        Item::Label(label) => *label,
        Item::Instruction(_) => unreachable!(),
    })
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

fn unique(items: &[Item], record: Instruction) -> bool {
    items
        .iter()
        .filter(|item| **item == Item::Instruction(record))
        .count()
        == 1
}

fn head_start(items: &[Item], mut index: usize) -> usize {
    while index > 0 && matches!(items[index - 1], Item::Label(_)) {
        index -= 1;
    }
    index
}

impl Context<'_> {
    fn clean(&self, label: Label) -> bool {
        let p = self.provenance;
        !self.aliases.contains(&label)
            && !self.preserved.contains(&label)
            && !self.original_nops.contains(&label)
            && !self.unreachable.contains(&label)
            && !self.finally_entries.contains_key(&label)
            && !p.borrow_only_block_boundaries.contains(&label)
            && !p.omitted_nop_counts.contains_key(&label)
            && !p.omitted_nop_locations.contains_key(&label)
            && !p.fusion_boundaries.contains(&label)
            && !p.continuation_boundaries.contains(&label)
            && !p.value_jump_continuation_boundaries.contains(&label)
            && !p.setup_nop_boundaries.contains(&label)
            && !p.setups.iter().any(|(setup, _)| *setup == label)
            && !p.ordinary_try_setups.contains_key(&label)
            && !p.named_handler_cleanup_targets.contains(&label)
            && !p
                .setup_nop_ranges
                .iter()
                .any(|(start, end)| *start == label || *end == label)
            && !p.original_fallthrough_setups.contains(&label)
            && !p.return_region_reentries.contains(&label)
            && !p.loop_finally_region_boundaries.contains(&label)
            && !p.normal_try_copy_blocks.contains(&label)
            && !p.normal_try_copy_inline_labels.contains(&label)
            && !p.normal_try_copy_internal_labels.contains(&label)
            && !p.normal_try_copy_jump_nop_successors.contains(&label)
            && !p.retired_normal_try_copy_labels.contains(&label)
            && !self.regions.iter().any(|region| region.target == label)
    }

    fn boundaries(&self, label: Label) -> (bool, bool) {
        (
            self.provenance.source_boundaries.contains(&label),
            self.provenance.block_boundaries.contains(&label),
        )
    }

    fn registry_matches(&self, source: &Source) -> bool {
        let mut origins = self
            .provenance
            .compound_try_owner_exits
            .iter()
            .filter(|(entry, _)| labels(&source.head).any(|label| label == *entry));
        origins.next() == Some(&(source.entry, source.target)) && origins.next().is_none()
    }
}

impl Source {
    fn matches_items(&self, items: &[Item], index: usize, context: &Context<'_>) -> bool {
        let Some(positions) = positions(items) else {
            return false;
        };
        let Some(start) = index.checked_sub(self.head.len()) else {
            return false;
        };
        if head_start(items, index) != start
            || items.get(start..index) != Some(&self.head)
            || items.get(index) != Some(&Item::Instruction(self.jump))
            || !matches!(items.get(start.wrapping_sub(1)), Some(Item::Instruction(record)) if *record == self.raised)
            || labels(&self.head).any(|label| label == self.target)
            || positions
                .get(&self.target)
                .is_some_and(|target| *target <= index)
            || !context.registry_matches(self)
            || !labels(&self.head)
                .zip(&self.boundaries)
                .all(|(label, old)| context.clean(label) && context.boundaries(label) == *old)
            || context
                .regions
                .iter()
                .any(|region| region.target == self.target)
            || positions.get(&self.target).is_some_and(|&target| {
                items[head_start(items, target)..]
                    .iter()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .any(|item| {
                        matches!(item, Item::Label(label)
                            if context.regions.iter().any(|region| region.target == *label))
                    })
            })
            || ![self.jump, self.guard, self.raised]
                .into_iter()
                .all(|record| unique(items, record))
        {
            return false;
        }
        let mut incoming = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Instruction(record)
                    if matches!(record.operand(),
                Operand::Forward(label) | Operand::Backward(label)
                    if labels(&self.head).any(|entry| entry == label)) =>
                {
                    Some((index, *record))
                }
                _ => None,
            });
        incoming
            .next()
            .is_some_and(|(index, record)| index < start && record == self.guard)
            && incoming.next().is_none()
    }

    pub(super) fn matches(
        &self,
        graph: &BlockGraph,
        position: ItemPosition,
        jump: Instruction,
        context: &Context<'_>,
    ) -> bool {
        // This original-source traversal is populated after emission. Its complete
        // reachable head must agree with the captured statement-level expectation.
        let Some(sources) = context.finally_sources else {
            return false;
        };
        if !self.normally_reachable
            || !labels(&self.head)
                .all(|label| sources.get(&label) == Some(&self.normally_reachable))
            || self.jump != jump
            || graph[position.block()].items().len() != self.head.len() + 1
            || position.offset() != self.head.len()
            || graph[position.block()].items()[..position.offset()] != self.head
        {
            return false;
        }
        let items = graph.copy_items();
        let Some(positions) = positions(&items) else {
            return false;
        };
        let Some(start) = labels(&self.head)
            .next()
            .and_then(|label| positions.get(&label))
        else {
            return false;
        };
        positions.contains_key(&self.target)
            && self.matches_items(&items, start + self.head.len(), context)
    }
}

impl Assembler {
    /// Records a unique synchronous source for the existing late ownerless-NOP writer.
    pub(crate) fn record_synchronous_compound_try_nop(
        &mut self,
        entry: Label,
        target: Label,
        emitted: EmittedJump,
        normally_reachable: bool,
    ) {
        if !normally_reachable
            || self.python_version < crate::PythonVersion::Py314
            || emitted.instruction().stream != self.emission_stream
        {
            return;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return;
        };
        let Ok(index) = usize::try_from(emitted.instruction().item_index) else {
            return;
        };
        if index + 1 != items.len() {
            return;
        }
        let Some(Item::Instruction(jump)) = items.get(index) else {
            return;
        };
        let mut expected = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            jump.location,
            Some(0),
        );
        expected.jump_policy.prevent_small_exit_inlining();
        expected.jump_policy.retain_nop_for_direct_target();
        expected
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        if *jump != expected || jump.location.line < 0 {
            return;
        }
        let start = head_start(items, index);
        let head = &items[start..index];
        let Some(Item::Instruction(raised)) = items.get(start.wrapping_sub(1)) else {
            return;
        };
        if head.is_empty()
            || !head.contains(&Item::Label(entry))
            || *raised
                != Instruction::new(RAISE_VARARGS, raised.operand(), raised.location, Some(0))
            || !matches!(raised.operand(), Operand::Value(0..=2))
            || raised.location.line < 0
        {
            return;
        }
        let incoming = items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record)
                    if matches!(record.operand(),
                Operand::Forward(label) | Operand::Backward(label)
                    if labels(head).any(|entry| entry == label)) =>
                {
                    Some(*record)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [guard] = incoming.as_slice() else {
            return;
        };
        if !is_conditional_jump(guard.opcode)
            || !matches!(guard.operand(), Operand::Forward(_))
            || *guard != Instruction::new(guard.opcode, guard.operand(), jump.location, Some(0))
        {
            return;
        }
        let context = Context {
            provenance: &self.exception_owner_provenance,
            regions: &self.exception_regions,
            aliases: &self.borrow_block_aliases,
            preserved: &self.preserved_block_boundaries,
            original_nops: &self.original_nop_targets,
            unreachable: &self.borrow_unreachable_blocks,
            finally_entries: &self.borrow_finally_entries,
            finally_sources: None,
        };
        let origin = Source {
            entry,
            target,
            head: head.to_vec(),
            boundaries: labels(head)
                .map(|label| context.boundaries(label))
                .collect(),
            jump: *jump,
            guard: *guard,
            raised: *raised,
            normally_reachable,
        };
        let Operand::Forward(guard_entry) = guard.operand() else {
            return;
        };
        if context.boundaries(guard_entry) == (true, true)
            && origin.matches_items(items, index, &context)
        {
            self.exception_owner_provenance
                .synchronous_compound_try_nops
                .push(origin);
        }
    }
}
