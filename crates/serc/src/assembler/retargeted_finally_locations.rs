//! Location donation through an authenticated original manager/ordinary-try bridge.

use rustc_hash::{FxHashMap, FxHashSet};

use super::block_graph::ItemPosition;
use super::fusion_owner_slots::same_manager_region;
use super::normal_finally_locations::{
    anchored_start, array_start, labels, same_priority, unique_label_positions,
};
use super::{
    Assembler, EmittedJump, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction,
    Item, Label, Operand, OptimizationPipeline, SourceLocation,
};
use crate::target::opcodes::{
    JUMP_FORWARD, LOAD_DEREF, LOAD_FAST, NOP, NOT_TAKEN, POP_EXCEPT, POP_JUMP_IF_NONE,
    POP_JUMP_IF_NOT_NONE, POP_TOP, PUSH_EXC_INFO,
};
use crate::target::{is_scope_exit, is_unconditional_jump};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct ManagerSuppressionExit {
    target: Label,
    tail: Vec<Item>,
}

/// Original one-hop inputs and the receipt produced by that exact threading event.
#[derive(Debug)]
pub(super) struct RetargetedNormalFinallyPrefix {
    entry: Label,
    marker: usize,
    prefix: Vec<Item>,
    manager: ManagerSuppressionExit,
    bridge: Vec<Item>,
    end: Vec<Item>,
    handler: Vec<Item>,
    setup: Label,
    setup_range: (Label, Label),
    protected_aliases: Vec<Label>,
    region: ExceptionRegion,
    threaded: Option<Vec<Item>>,
}

pub(super) struct InitialBorrowMark {
    origin: usize,
    position: ItemPosition,
    original: Instruction,
    expected: Instruction,
}

fn records(items: &[Item]) -> impl Iterator<Item = &Instruction> {
    items.iter().filter_map(|item| match item {
        Item::Instruction(instruction) => Some(instruction),
        Item::Label(_) => None,
    })
}

fn head(items: &[Item], positions: &FxHashMap<Label, usize>, label: Label) -> Option<Vec<Item>> {
    let mut start = *positions.get(&label)?;
    while start > 0 && matches!(items[start - 1], Item::Label(_)) {
        start -= 1;
    }
    let mut end = start;
    while matches!(items.get(end), Some(Item::Label(_))) {
        end += 1;
    }
    matches!(items.get(end), Some(Item::Instruction(_))).then(|| items[start..=end].to_vec())
}

impl ManagerSuppressionExit {
    pub(super) fn visit_records_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        for item in &mut self.tail {
            if let Item::Instruction(instruction) = item {
                visit(instruction);
            }
        }
    }
}

impl RetargetedNormalFinallyPrefix {
    pub(super) fn visit_records_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        self.manager.visit_records_mut(&mut visit);
        for item in self
            .prefix
            .iter_mut()
            .chain(&mut self.bridge)
            .chain(&mut self.end)
            .chain(&mut self.handler)
            .chain(self.threaded.iter_mut().flatten())
        {
            if let Item::Instruction(instruction) = item {
                visit(instruction);
            }
        }
    }

    pub(super) fn resolve_priority(&mut self, positions: &FxHashMap<Label, usize>) -> bool {
        if let ExceptionRegionPriority::OriginalRange(start, end) = self.region.priority {
            let (Some(start), Some(end)) = (positions.get(&start), positions.get(&end)) else {
                return false;
            };
            self.region.priority = ExceptionRegionPriority::Resolved {
                start: *start,
                end: *end,
            };
        }
        true
    }

    fn intact(&self, items: &[Item], regions: &[ExceptionRegion]) -> Option<usize> {
        unique_label_positions(items)?;
        let start = anchored_start(items, &self.prefix)?;
        anchored_start(items, &self.end)?;
        anchored_start(items, &self.handler)?;
        if !matches!(items.get(start.checked_sub(1)?), Some(Item::Instruction(previous))
            if is_scope_exit(previous.opcode) || is_unconditional_jump(previous.opcode))
            || regions
                .iter()
                .filter(|region| same_priority(region.priority, self.region.priority))
                .count()
                != 1
            || regions
                .iter()
                .filter(|region| region.target == self.region.target)
                .count()
                != 1
            || !regions.iter().any(|region| {
                let mut original = self.region;
                if self.protected_aliases.contains(&region.start) {
                    original.start = region.start;
                }
                if self.end.contains(&Item::Label(region.end)) {
                    original.end = region.end;
                }
                same_manager_region(region, &original)
            })
        {
            return None;
        }
        Some(start)
    }
}

impl Assembler {
    /// Records the concrete, fully configured suppression exit, not its source range alone.
    pub(crate) fn record_manager_suppression_exit(
        &mut self,
        emitted: EmittedJump,
        suppressed: Label,
        handled: Label,
        target: Label,
    ) {
        if self.python_version < crate::PythonVersion::Py314 {
            return;
        }
        let jump = *self.emitted_jump_mut(emitted, "recording a manager suppression exit");
        let items = self.linear_items();
        let Some(positions) = unique_label_positions(items) else {
            return;
        };
        let Some(&start) = positions.get(&suppressed) else {
            return;
        };
        let end = emitted.instruction().item_index as usize;
        let Some(tail) = items.get(start..=end) else {
            return;
        };
        let instructions = records(tail).copied().collect::<Vec<_>>();
        if jump.opcode != JUMP_FORWARD
            || jump.operand() != Operand::Forward(target)
            || jump.location.line < 0
            || jump.depth_after() != Some(0)
            || instructions
                .iter()
                .map(|instruction| instruction.opcode)
                .collect::<Vec<_>>()
                != [POP_TOP, POP_EXCEPT, POP_TOP, POP_TOP, POP_TOP, JUMP_FORWARD]
            || instructions
                .iter()
                .map(Instruction::depth_after)
                .collect::<Vec<_>>()
                != [Some(4), Some(3), Some(2), Some(1), Some(0), Some(0)]
            || instructions.iter().any(|instruction| {
                instruction.location != jump.location
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.local_policy.hidden_exception_owner_slots() != 0
            })
            || instructions.last() != Some(&jump)
            || !tail.contains(&Item::Label(handled))
            || matches!(items.get(start.wrapping_sub(1)), Some(Item::Label(_)))
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(jump))
                .count()
                != 1
        {
            return;
        }
        self.exception_owner_provenance
            .manager_suppression_exits
            .push(ManagerSuppressionExit {
                target,
                tail: tail.to_vec(),
            });
    }

    /// Captures a source-certified nested-finally None prefix and its original one-hop route.
    pub(crate) fn record_retargeted_normal_finally_prefix(&mut self, ordinal: usize) {
        if let Some(origin) = self.retargeted_normal_finally_prefix(ordinal) {
            self.exception_owner_provenance
                .retargeted_normal_finally_prefixes
                .push(origin);
        }
    }

    fn retargeted_normal_finally_prefix(
        &self,
        ordinal: usize,
    ) -> Option<RetargetedNormalFinallyPrefix> {
        let items = self.linear_items();
        let positions = unique_label_positions(items)?;
        let index = self.instruction_item_index(ordinal)?;
        let provenance = &self.exception_owner_provenance;
        let mut original = provenance
            .original_normal_finally_arrays
            .iter()
            .filter(|origin| {
                anchored_start(items, &origin.prefix).is_some_and(|start| {
                    origin
                        .prefix
                        .iter()
                        .position(|item| matches!(item, Item::Instruction(_)))
                        .is_some_and(|offset| start + offset == index)
                })
            });
        let original = original.next().filter(|_| original.next().is_none())?;
        let (setup_start, setup_end, setup) = original.retained_setup?;
        if provenance.ordinary_try_setups.contains_key(&setup) {
            return None;
        }
        let prefix = &original.prefix;
        let instructions = records(prefix).collect::<Vec<_>>();
        let [pop, marker, load, condition, not_taken] = instructions.as_slice() else {
            return None;
        };
        if pop.location != SourceLocation::NONE
            || pop.opcode != NOP
            || pop.operand() != Operand::Value(0)
            || pop.depth_after() != Some(0)
            || !pop.fusion_policy.is_converted_pop_block()
            || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || pop.no_location_policy.preserves_location()
            || marker.opcode != NOP
            || marker.location.line < 0
            || !matches!(load.opcode, LOAD_FAST | LOAD_DEREF)
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || not_taken.opcode != NOT_TAKEN
            || not_taken.operand() != Operand::Value(0)
            || not_taken.location != condition.location
            || not_taken.depth_after() != condition.depth_after()
            || prefix.last() != Some(&Item::Instruction(**not_taken))
        {
            return None;
        }
        let mut regions = self.exception_regions.iter().filter(|region| {
            region.start == setup && !region.is_scope_fallback && !region.preserve_lasti
        });
        let region = *regions.next()?;
        if regions.next().is_some()
            || region.depth != 0
            || self
                .exception_regions
                .iter()
                .filter(|other| other.target == region.target)
                .count()
                != 1
        {
            return None;
        }
        let handler = head(items, &positions, region.target)?;
        if records(&handler).next()?.opcode != PUSH_EXC_INFO {
            return None;
        }
        let end = head(items, &positions, region.end)?;
        let Operand::Forward(branch_target) = condition.operand() else {
            return None;
        };
        if !end.contains(&Item::Label(branch_target)) {
            return None;
        }
        let marker_index = prefix
            .iter()
            .position(|item| matches!(item, Item::Instruction(_)))?;
        let setup_offset = prefix.iter().position(|item| *item == Item::Label(setup))?;
        let mut protected_start = setup_offset;
        while protected_start > 0 && matches!(prefix[protected_start - 1], Item::Label(_)) {
            protected_start -= 1;
        }
        let protected_aliases = prefix[protected_start..]
            .iter()
            .take_while(|item| matches!(item, Item::Label(_)))
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect();
        let mut candidates = provenance
            .manager_suppression_exits
            .iter()
            .filter_map(|manager| {
                let source_start = anchored_start(items, &manager.tail)?;
                let bridge_start = *positions.get(&manager.target)?;
                let mut bridge_end = bridge_start;
                while matches!(items.get(bridge_end), Some(Item::Label(_))) {
                    bridge_end += 1;
                }
                let Item::Instruction(bridge) = *items.get(bridge_end)? else {
                    return None;
                };
                let bridge_start = array_start(items, bridge_end)?;
                let bridge_items = &items[bridge_start..=bridge_end];
                if bridge.opcode != JUMP_FORWARD
                    || bridge.operand() != Operand::Forward(original.entry)
                    || bridge.location != SourceLocation::NONE
                    || bridge.depth_after() != Some(0)
                    || bridge.pre_fold_extra_instructions() != 0
                    || bridge.local_policy.hidden_exception_owner_slots() != 0
                    || items
                        .iter()
                        .filter(|item| **item == Item::Instruction(bridge))
                        .count()
                        != 1
                    || !provenance
                        .compound_try_owner_exits
                        .iter()
                        .any(|(entry, target)| {
                            *target == original.entry && bridge_items.contains(&Item::Label(*entry))
                        })
                    || manager
                        .tail
                        .last()
                        .is_none_or(|jump| items.iter().filter(|item| *item == jump).count() != 1)
                    || source_start + manager.tail.len() > bridge_start
                    || bridge_end >= index
                {
                    return None;
                }
                Some((manager.clone(), bridge_items.to_vec()))
            });
        let (manager, bridge) = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        Some(RetargetedNormalFinallyPrefix {
            entry: original.entry,
            marker: marker_index,
            prefix: prefix[..prefix.len() - 1].to_vec(),
            manager,
            bridge,
            end,
            handler,
            setup,
            setup_range: (setup_start, setup_end),
            protected_aliases,
            region,
            threaded: None,
        })
    }
}

impl OptimizationPipeline {
    /// Captures a complete donor occurrence before the initial-borrow writer changes it.
    pub(super) fn retargeted_finally_initial_borrow_candidates(&self) -> Vec<InitialBorrowMark> {
        let origins = &self
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes;
        if origins.is_empty() {
            return Vec::new();
        }
        let positions = self.graph.item_positions();
        let items = positions
            .iter()
            .map(|position| *self.graph.item(*position))
            .collect::<Vec<_>>();
        origins
            .iter()
            .enumerate()
            .filter_map(|(index, origin)| {
                if origin.threaded.is_some() {
                    return None;
                }
                origin.intact(&items, &self.exception_regions)?;
                anchored_start(&items, &origin.bridge)?;
                let tail = &origin.manager.tail;
                let start = anchored_start(&items, tail)?;
                let Item::Instruction(original) = *tail.last()? else {
                    return None;
                };
                if items
                    .iter()
                    .filter(|item| **item == Item::Instruction(original))
                    .count()
                    != 1
                {
                    return None;
                }
                let mut expected = original;
                if original.initial_borrow_jump_target().is_some() {
                    return None;
                }
                let Operand::Forward(target) = original.operand() else {
                    return None;
                };
                expected.mark_initial_borrow_jump_target(target);
                Some(InitialBorrowMark {
                    origin: index,
                    position: positions[start + tail.len() - 1],
                    original,
                    expected,
                })
            })
            .collect()
    }

    /// Accepts only the exact initial-target mark at that unchanged donor occurrence.
    pub(super) fn seal_retargeted_finally_initial_borrow_marks(
        &mut self,
        candidates: Vec<InitialBorrowMark>,
    ) {
        if candidates.is_empty() {
            return;
        }
        let positions = self.graph.item_positions();
        let mut items = positions
            .iter()
            .map(|position| *self.graph.item(*position))
            .collect::<Vec<_>>();
        for candidate in candidates {
            let Some(at) = positions
                .iter()
                .position(|position| *position == candidate.position)
            else {
                continue;
            };
            if items[at] != Item::Instruction(candidate.expected)
                || items.iter().filter(|item| **item == items[at]).count() != 1
            {
                continue;
            }
            let origin = &mut self
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[candidate.origin];
            if origin.threaded.is_some() {
                continue;
            }
            let tail = &origin.manager.tail;
            if tail.last() != Some(&Item::Instruction(candidate.original)) {
                continue;
            }
            items[at] = Item::Instruction(candidate.original);
            let intact = origin.intact(&items, &self.exception_regions).is_some()
                && anchored_start(&items, tail).is_some_and(|start| start + tail.len() - 1 == at)
                && anchored_start(&items, &origin.bridge).is_some()
                && items
                    .iter()
                    .filter(|item| **item == Item::Instruction(candidate.original))
                    .count()
                    == 1;
            items[at] = Item::Instruction(candidate.expected);
            if intact {
                *origin.manager.tail.last_mut().expect("captured donor") =
                    Item::Instruction(candidate.expected);
            }
        }
    }

    /// Binds the original bridge occurrence before the existing borrowing-head mark.
    pub(super) fn retargeted_finally_borrow_candidates(
        &self,
        heads: &[ItemPosition],
    ) -> Vec<(usize, ItemPosition, Instruction)> {
        if self
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes
            .is_empty()
        {
            return Vec::new();
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
        self.exception_owner_provenance
            .retargeted_normal_finally_prefixes
            .iter()
            .enumerate()
            .filter_map(|(index, origin)| {
                if origin.threaded.is_some() {
                    return None;
                }
                origin.intact(&items, &self.exception_regions)?;
                anchored_start(&items, &origin.manager.tail)?;
                let start = anchored_start(&items, &origin.bridge)?;
                let at = start + origin.bridge.len() - 1;
                let Item::Instruction(instruction) = items[at] else {
                    return None;
                };
                let source = origin.manager.tail.last()?;
                (heads.contains(&positioned[at].0)
                    && !instruction.jump_policy.borrow_empty_block()
                    && items.iter().filter(|item| **item == items[at]).count() == 1
                    && items.iter().filter(|item| *item == source).count() == 1)
                    .then_some((index, positioned[at].0, instruction))
            })
            .collect()
    }

    /// Transports only the full record actually changed by that borrowing-head mark.
    pub(super) fn seal_retargeted_finally_borrow_marks(
        &mut self,
        candidates: Vec<(usize, ItemPosition, Instruction)>,
    ) {
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
        let mut items = positioned.iter().map(|(_, item)| *item).collect::<Vec<_>>();
        for (index, position, original) in candidates {
            let Some(at) = positioned.iter().position(|(at, _)| *at == position) else {
                continue;
            };
            let Item::Instruction(actual) = items[at] else {
                continue;
            };
            let mut expected = original;
            expected.jump_policy.mark_borrow_empty_block();
            if actual != expected {
                continue;
            }
            let origin = &mut self
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[index];
            if origin.bridge.last() != Some(&Item::Instruction(original)) {
                continue;
            }
            let unique = items
                .iter()
                .filter(|item| **item == Item::Instruction(actual))
                .count()
                == 1;
            items[at] = Item::Instruction(original);
            let intact =
                unique
                    && origin.intact(&items, &self.exception_regions).is_some()
                    && anchored_start(&items, &origin.manager.tail).is_some()
                    && anchored_start(&items, &origin.bridge)
                        .is_some_and(|start| start + origin.bridge.len() - 1 == at)
                    && items
                        .iter()
                        .filter(|item| **item == Item::Instruction(original))
                        .count()
                        == 1
                    && origin.manager.tail.last().is_some_and(|source| {
                        items.iter().filter(|item| *item == source).count() == 1
                    });
            items[at] = Item::Instruction(actual);
            if !intact {
                continue;
            }
            *origin.bridge.last_mut().expect("captured bridge") = Item::Instruction(actual);
        }
    }

    /// Authenticates source and bridge occurrences before the real thread operation.
    pub(super) fn retargeted_finally_thread_candidates(
        &self,
    ) -> FxHashMap<usize, (usize, Instruction, Label)> {
        let items = self.graph.iter_items().copied().collect::<Vec<_>>();
        let mut result = FxHashMap::default();
        let mut ambiguous = FxHashSet::default();
        for (index, origin) in self
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes
            .iter()
            .enumerate()
        {
            let proof = (|| {
                if origin.threaded.is_some() {
                    return None;
                }
                origin.intact(&items, &self.exception_regions)?;
                let start = anchored_start(&items, &origin.manager.tail)?;
                anchored_start(&items, &origin.bridge)?;
                let Item::Instruction(source) = *origin.manager.tail.last()? else {
                    return None;
                };
                let bridge = origin.bridge.last()?;
                if items
                    .iter()
                    .filter(|item| **item == Item::Instruction(source))
                    .count()
                    != 1
                    || items.iter().filter(|item| *item == bridge).count() != 1
                    || source.operand() != Operand::Forward(origin.manager.target)
                {
                    return None;
                }
                let source_index = items[..start + origin.manager.tail.len() - 1]
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count();
                Some((source_index, (index, source, origin.entry)))
            })();
            if let Some((source, candidate)) = proof {
                if ambiguous.contains(&source) {
                    continue;
                }
                if result.insert(source, candidate).is_some() {
                    result.remove(&source);
                    ambiguous.insert(source);
                }
            }
        }
        result
    }

    /// Seals only real one-hop events, after existing owner-transfer processing is complete.
    pub(super) fn seal_retargeted_finally_threads(&mut self, receipts: Vec<(usize, Instruction)>) {
        let items = self.graph.iter_items().copied().collect::<Vec<_>>();
        if unique_label_positions(&items).is_none() {
            return;
        }
        for (index, instruction) in receipts {
            let origin = &mut self
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[index];
            let mut tail = origin.manager.tail.clone();
            *tail.last_mut().expect("captured manager exit") = Item::Instruction(instruction);
            if instruction.operand() == Operand::Forward(origin.entry)
                && anchored_start(&items, &tail).is_some()
                && items
                    .iter()
                    .filter(|item| **item == Item::Instruction(instruction))
                    .count()
                    == 1
                && origin.intact(&items, &self.exception_regions).is_some()
            {
                origin.threaded = Some(tail);
            }
        }
    }

    /// Supplies one original POP location without changing copying, ownership or retention.
    pub(super) fn propagate_retargeted_normal_finally_locations(&mut self) {
        if self
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes
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
        let reachable = self.graph.reachable_blocks(&self.exception_regions);
        let mut updates = Vec::new();
        for origin in &self
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes
        {
            let proof = (|| {
                let start = origin.intact(&items, &self.exception_regions)?;
                let tail = origin.threaded.as_ref()?;
                let source = anchored_start(&items, tail)? + tail.len() - 1;
                let handler = anchored_start(&items, &origin.handler)?;
                let end = anchored_start(&items, &origin.end)?;
                if source >= start
                    || [start, source, handler, end]
                        .iter()
                        .any(|index| !reachable[positioned[*index].0.block().index()])
                {
                    return None;
                }
                let Item::Instruction(jump) = items[source] else {
                    return None;
                };
                let provenance = &self.exception_owner_provenance;
                let setup_location = records(&origin.prefix).nth(1)?.location;
                if provenance
                    .setup_nop_ranges
                    .iter()
                    .filter(|range| **range == origin.setup_range)
                    .count()
                    != 1
                    || provenance
                        .setups
                        .iter()
                        .filter(|(label, _)| *label == origin.setup)
                        .copied()
                        .collect::<Vec<_>>()
                        != [(origin.setup, Some(setup_location))]
                    || provenance.ordinary_try_setups.contains_key(&origin.setup)
                    || jump.location.line < 0
                    || items
                        .iter()
                        .filter(|item| **item == Item::Instruction(jump))
                        .count()
                        != 1
                {
                    return None;
                }
                let head = labels(&origin.prefix[..origin.marker]).collect::<FxHashSet<_>>();
                let interior =
                    labels(&origin.prefix[origin.marker + 1..]).collect::<FxHashSet<_>>();
                if self.exception_regions.iter().any(|region| {
                    head.contains(&region.target) || interior.contains(&region.target)
                }) {
                    return None;
                }
                let mut incoming = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    if let Item::Instruction(instruction) = item
                        && let Operand::Forward(target) | Operand::Backward(target) =
                            instruction.operand()
                    {
                        if interior.contains(&target) {
                            return None;
                        }
                        if head.contains(&target) {
                            incoming.push(index);
                        }
                    }
                }
                if incoming != [source] {
                    return None;
                }
                Some((positioned[start + origin.marker].0, jump.location))
            })();
            if let Some(update) = proof {
                updates.push(update);
                #[cfg(test)]
                tests::trace("replay");
            }
        }
        for (position, location) in updates {
            self.graph
                .instruction_mut(position)
                .expect("captured normal POP")
                .location = location;
        }
    }
}
