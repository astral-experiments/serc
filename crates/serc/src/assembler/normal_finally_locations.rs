//! Original normal-finally locations while an awaited nested SETUP is still distinct.

use rustc_hash::{FxHashMap, FxHashSet};

use super::block_graph::ItemPosition;
use super::fusion_owner_slots::same_manager_region;
use super::{
    Assembler, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    Operand, OptimizationPipeline, SourceLocation, uses_constant_index,
};
use crate::target::opcodes::{
    END_SEND, GET_AWAITABLE, JUMP_FORWARD, LOAD_CONST, NOP, POP_TOP, PUSH_EXC_INFO, PUSH_NULL, SEND,
};
use crate::target::{is_scope_exit, is_unconditional_jump};

#[cfg(test)]
pub(super) mod tests;

/// A normal POP, retained ordinary SETUP, and unchanged original awaited prefix.
#[derive(Debug)]
pub(super) struct AwaitedNormalFinallyPrefix {
    entry: Label,
    marker: usize,
    prefix: Vec<Item>,
    sources: Vec<Vec<Item>>,
    completion: Vec<Item>,
    handler: Vec<Item>,
    setup_range: (Label, Label),
    protected_aliases: Vec<Label>,
    region: ExceptionRegion,
}

pub(super) fn labels(items: &[Item]) -> impl Iterator<Item = Label> + '_ {
    items.iter().filter_map(|item| match item {
        Item::Label(label) => Some(*label),
        Item::Instruction(_) => None,
    })
}

pub(super) fn unique_label_positions(items: &[Item]) -> Option<FxHashMap<Label, usize>> {
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

pub(super) fn anchored_start(items: &[Item], original: &[Item]) -> Option<usize> {
    let Item::Label(entry) = *original.first()? else {
        return None;
    };
    let start = items.iter().position(|item| *item == Item::Label(entry))?;
    if matches!(items.get(start.wrapping_sub(1)), Some(Item::Label(_))) {
        return None;
    }
    (items.get(start..start.checked_add(original.len())?)? == original).then_some(start)
}

pub(super) fn array_start(items: &[Item], mut index: usize) -> Option<usize> {
    while index > 0 && matches!(items[index - 1], Item::Instruction(_)) {
        index -= 1;
    }
    let end = index;
    while index > 0 && matches!(items[index - 1], Item::Label(_)) {
        index -= 1;
    }
    (index < end).then_some(index)
}

pub(super) fn same_priority(left: ExceptionRegionPriority, right: ExceptionRegionPriority) -> bool {
    match (left, right) {
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

impl AwaitedNormalFinallyPrefix {
    pub(super) fn matches_original(&self, items: &[Item], regions: &[ExceptionRegion]) -> bool {
        std::iter::once(&self.prefix)
            .chain(&self.sources)
            .chain([&self.completion, &self.handler])
            .all(|original| anchored_start(items, original).is_some())
            && {
                let mut matching = regions.iter().filter(|region| {
                    region.target == self.region.target
                        || same_priority(region.priority, self.region.priority)
                });
                matching
                    .next()
                    .is_some_and(|region| same_manager_region(region, &self.region))
                    && matching.next().is_none()
            }
    }

    pub(super) fn visit_records_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        for item in self
            .prefix
            .iter_mut()
            .chain(self.sources.iter_mut().flatten())
            .chain(&mut self.completion)
            .chain(&mut self.handler)
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
            if !uses_constant_index(instruction.opcode) {
                return;
            }
            let Operand::Value(index) = instruction.operand() else {
                valid = false;
                return;
            };
            let index = if index == u32::MAX {
                deferred_none
            } else {
                Some(index)
            };
            if let Some(Some(remapped)) = index.and_then(|index| map.get(index as usize)) {
                instruction.set_operand(Operand::Value(*remapped));
            } else {
                valid = false;
            }
        });
        valid
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
}

impl Assembler {
    /// Captures a source-certified ordinary awaited first suite, without changing owner seeds.
    pub(crate) fn record_awaited_normal_finally_prefix(&mut self, ordinal: usize) {
        let Some(origin) = self.awaited_normal_finally_prefix(ordinal) else {
            return;
        };
        self.exception_owner_provenance
            .awaited_normal_finally_prefixes
            .push(origin);
    }

    pub(super) fn awaited_normal_finally_prefix(
        &self,
        ordinal: usize,
    ) -> Option<AwaitedNormalFinallyPrefix> {
        if self.python_version < crate::PythonVersion::Py314 {
            return None;
        }
        let items = self.linear_items();
        let positions = unique_label_positions(items)?;
        let index = self.instruction_item_index(ordinal)?;
        let Item::Instruction(marker) = items[index] else {
            return None;
        };
        if marker.opcode != NOP
            || marker.operand() != Operand::Value(0)
            || marker.location != SourceLocation::NONE
            || !marker.fusion_policy.is_converted_pop_block()
            || marker.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || marker.no_location_policy.preserves_location()
        {
            return None;
        }
        let mut begin = index;
        while begin > 0 && matches!(items[begin - 1], Item::Label(_)) {
            begin -= 1;
        }
        let Item::Label(entry) = *items.get(begin)? else {
            return None;
        };
        let provenance = &self.exception_owner_provenance;
        if !provenance.source_boundaries.contains(&entry)
            || !provenance.block_boundaries.contains(&entry)
            || !matches!(items.get(begin.checked_sub(1)?), Some(Item::Instruction(previous))
                if is_scope_exit(previous.opcode))
        {
            return None;
        }
        let mut setup_index = index + 1;
        while matches!(items.get(setup_index), Some(Item::Label(_))) {
            setup_index += 1;
        }
        let Item::Instruction(setup_marker) = *items.get(setup_index)? else {
            return None;
        };
        if setup_marker.opcode != NOP || setup_marker.location.line < 0 {
            return None;
        }
        let mut body = setup_index + 1;
        while matches!(items.get(body), Some(Item::Label(_))) {
            body += 1;
        }
        let mut setups = provenance
            .setup_nop_ranges
            .iter()
            .filter_map(|&(start, end)| {
                if !items[index + 1..setup_index].contains(&Item::Label(start))
                    || !items[setup_index + 1..body].contains(&Item::Label(end))
                {
                    return None;
                }
                let mut matching = provenance.setups.iter().filter(|(label, location)| {
                    *location == Some(setup_marker.location)
                        && items[setup_index + 1..body].contains(&Item::Label(*label))
                });
                let &(setup, _) = matching.next()?;
                matching.next().is_none().then_some(((start, end), setup))
            });
        let (setup_range, setup) = setups.next()?;
        if setups.next().is_some() {
            return None;
        }
        let handler = *provenance.ordinary_try_setups.get(&setup)?;
        let mut regions = self
            .exception_regions
            .iter()
            .filter(|region| region.target == handler);
        let region = *regions.next()?;
        if regions.next().is_some()
            || region.start != setup
            || region.is_scope_fallback
            || region.preserve_lasti
            || Some(region.depth) != marker.depth_after()
            || !matches!(region.priority, ExceptionRegionPriority::OriginalRange(start, end)
                if start == region.start && end == region.end)
        {
            return None;
        }
        if self
            .exception_regions
            .iter()
            .filter(|other| same_priority(other.priority, region.priority))
            .count()
            != 1
        {
            return None;
        }
        let mut send_index = body;
        while let Some(Item::Instruction(instruction)) = items.get(send_index) {
            if !matches!(instruction.operand(), Operand::Value(_))
                || is_scope_exit(instruction.opcode)
                || instruction.opcode == NOP
            {
                return None;
            }
            send_index += 1;
        }
        let send_labels = send_index;
        while matches!(items.get(send_index), Some(Item::Label(_))) {
            send_index += 1;
        }
        let Item::Instruction(send) = *items.get(send_index)? else {
            return None;
        };
        if send_labels == send_index || send.opcode != SEND || send.location.line < 0 {
            return None;
        }
        let (awaitable, none) = match &items[body..send_labels] {
            [
                ..,
                Item::Instruction(awaitable),
                Item::Instruction(null),
                Item::Instruction(none),
            ] if self.python_version == crate::PythonVersion::Py315
                && null.opcode == PUSH_NULL
                && null.operand() == Operand::Value(0)
                && null.location == awaitable.location =>
            {
                (awaitable, none)
            }
            [.., Item::Instruction(awaitable), Item::Instruction(none)]
                if self.python_version == crate::PythonVersion::Py314 =>
            {
                (awaitable, none)
            }
            _ => return None,
        };
        if awaitable.opcode != GET_AWAITABLE
            || awaitable.operand() != Operand::Value(0)
            || none.opcode != LOAD_CONST
        {
            return None;
        }
        let mut awaits = provenance
            .ordinary_await_ends
            .iter()
            .filter(|origin| origin.send == send);
        let awaited = awaits.next()?;
        if awaits.next().is_some() || send.operand() != Operand::Forward(awaited.entry) {
            return None;
        }
        let completion_start = *positions.get(&awaited.entry)?;
        let completion_end = *positions.get(&region.end)?;
        let completion = items.get(completion_start..=completion_end)?;
        if !matches!(completion,
            [Item::Label(_), Item::Instruction(end_send), Item::Instruction(pop), Item::Label(_)]
                if *end_send == awaited.end_send && end_send.opcode == END_SEND && pop.opcode == POP_TOP)
        {
            return None;
        }
        let handler_start = *positions.get(&handler)?;
        let mut handler_end = handler_start + 1;
        while matches!(items.get(handler_end), Some(Item::Label(_))) {
            handler_end += 1;
        }
        if !matches!(items.get(handler_end), Some(Item::Instruction(instruction))
            if instruction.opcode == PUSH_EXC_INFO)
        {
            return None;
        }
        let prefix = &items[begin..=send_index];
        let head = labels(&items[begin..index]).collect::<FxHashSet<_>>();
        let interior = labels(&items[index + 1..send_labels]).collect::<FxHashSet<_>>();
        let mut sources = Vec::new();
        for (position, item) in items.iter().enumerate() {
            let Item::Instruction(jump) = item else {
                continue;
            };
            let (Operand::Forward(target) | Operand::Backward(target)) = jump.operand() else {
                continue;
            };
            if interior.contains(&target) {
                return None;
            }
            if !head.contains(&target) {
                continue;
            }
            if jump.opcode != JUMP_FORWARD
                || jump.operand() != Operand::Forward(entry)
                || items
                    .iter()
                    .filter(|item| **item == Item::Instruction(*jump))
                    .count()
                    != 1
            {
                return None;
            }
            let start = array_start(items, position)?;
            sources.push(items[start..=position].to_vec());
        }
        if sources.len() < 2
            || self
                .exception_regions
                .iter()
                .any(|region| head.contains(&region.target) || interior.contains(&region.target))
        {
            return None;
        }
        Some(AwaitedNormalFinallyPrefix {
            entry,
            marker: index - begin,
            prefix: prefix.to_vec(),
            sources,
            completion: completion.to_vec(),
            handler: items[handler_start..=handler_end].to_vec(),
            setup_range,
            protected_aliases: labels(&items[setup_index + 1..body]).collect(),
            region,
        })
    }
}

impl OptimizationPipeline {
    /// Replays native line donation before the ordinary marker cleanup, never retaining a NOP.
    pub(super) fn propagate_awaited_normal_finally_entry_locations(&mut self) {
        if self
            .exception_owner_provenance
            .awaited_normal_finally_prefixes
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
        let Some(positions) = unique_label_positions(&items) else {
            return;
        };
        let reachable = self.graph.reachable_blocks(&self.exception_regions);
        let mut updates = Vec::new();
        for origin in &self
            .exception_owner_provenance
            .awaited_normal_finally_prefixes
        {
            let proof = (|| {
                let start = anchored_start(&items, &origin.prefix)?;
                let completion = anchored_start(&items, &origin.completion)?;
                let handler = anchored_start(&items, &origin.handler)?;
                let marker = start + origin.marker;
                if [start, marker, completion, handler]
                    .iter()
                    .any(|index| !reachable[positioned[*index].0.block().index()])
                    || !matches!(items.get(start.checked_sub(1)?), Some(Item::Instruction(previous))
                    if is_scope_exit(previous.opcode) || is_unconditional_jump(previous.opcode))
                {
                    return None;
                }
                let provenance = &self.exception_owner_provenance;
                let setup_location =
                    origin.prefix[origin.marker + 1..]
                        .iter()
                        .find_map(|item| match item {
                            Item::Instruction(marker) => Some(marker.location),
                            Item::Label(_) => None,
                        })?;
                if provenance
                    .setup_nop_ranges
                    .iter()
                    .filter(|range| **range == origin.setup_range)
                    .count()
                    != 1
                    || provenance.ordinary_try_setups.get(&origin.region.start)
                        != Some(&origin.region.target)
                    || provenance
                        .setups
                        .iter()
                        .filter(|(label, _)| *label == origin.region.start)
                        .copied()
                        .collect::<Vec<_>>()
                        != [(origin.region.start, Some(setup_location))]
                    || self
                        .exception_regions
                        .iter()
                        .filter(|region| same_priority(region.priority, origin.region.priority))
                        .count()
                        != 1
                    || self
                        .exception_regions
                        .iter()
                        .filter(|region| region.target == origin.region.target)
                        .count()
                        != 1
                    || !self.exception_regions.iter().any(|region| {
                        let mut original = origin.region;
                        if origin.protected_aliases.contains(&region.start) {
                            original.start = region.start;
                        }
                        same_manager_region(region, &original)
                    })
                {
                    return None;
                }
                let head = labels(&origin.prefix[..origin.marker]).collect::<FxHashSet<_>>();
                let send_start = origin.prefix.iter().rposition(|item| matches!(item, Item::Instruction(instruction) if instruction.opcode != SEND))? + 1;
                let interior =
                    labels(&origin.prefix[origin.marker + 1..send_start]).collect::<FxHashSet<_>>();
                if self.exception_regions.iter().any(|region| {
                    head.contains(&region.target) || interior.contains(&region.target)
                }) {
                    return None;
                }
                let mut live = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    let Item::Instruction(jump) = item else {
                        continue;
                    };
                    let (Operand::Forward(target) | Operand::Backward(target)) = jump.operand()
                    else {
                        continue;
                    };
                    if interior.contains(&target) {
                        return None;
                    }
                    if !head.contains(&target) {
                        continue;
                    }
                    let mut originals = origin
                        .sources
                        .iter()
                        .filter(|source| source.last() == Some(item));
                    let source = originals.next()?;
                    let source_start = anchored_start(&items, source)?;
                    if originals.next().is_some()
                        || source_start + source.len() - 1 != index
                        || items.iter().filter(|candidate| *candidate == item).count() != 1
                        || jump.operand() != Operand::Forward(origin.entry)
                    {
                        return None;
                    }
                    if reachable[positioned[index].0.block().index()] {
                        live.push(*jump);
                    }
                }
                let [donor] = live.as_slice() else {
                    return None;
                };
                if donor.location.line < 0 {
                    return None;
                }
                // Every endpoint still names its original anchored record, not an alias
                // created by a moved record with an identical replacement elsewhere.
                if [origin.region.start, origin.region.end, origin.region.target]
                    .iter()
                    .any(|label| !positions.contains_key(label))
                {
                    return None;
                }
                Some((positioned[marker].0, donor.location))
            })();
            if let Some(update) = proof {
                updates.push(update);
            }
        }
        for (position, location) in updates {
            self.graph
                .instruction_mut(position)
                .expect("captured POP marker")
                .location = location;
        }
    }
}
