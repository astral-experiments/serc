use super::ir::EmissionStreamId;
use super::{
    Assembler, CALL, CLEANUP_THROW, CompletedAwaitTrySetups, END_SEND, ExceptionRegion,
    ExceptionRegionPriority, GET_AWAITABLE, Instruction, Item, JUMP_BACKWARD_NO_INTERRUPT,
    LOAD_CONST, Label, Operand, POP_TOP, RESUME, SEND, SWAP, SourceLocation, UnoptimizedCode,
    fusion_owner_slots, normal_finally_locations,
};
use crate::target::opcodes::YIELD_VALUE;
use crate::target::operands::ResumeLocation;

#[cfg(test)]
mod tests;

/// Original source entries before a return starts emitting its unwind.
pub(crate) struct HeldAwaitSource {
    stream: EmissionStreamId,
    setups: CompletedAwaitTrySetups,
    loop_start: usize,
    entries: Vec<(usize, Vec<Item>)>,
}

/// The completed event emitted by one actual async-manager return unwind.
pub(crate) struct CompletedManagerAwait {
    stream: EmissionStreamId,
    start: usize,
    items: Vec<Item>,
    occurrences: Vec<(Instruction, Vec<usize>)>,
    cleanup: ExceptionRegion,
}

fn occurrences(items: &[Item], record: Instruction) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(at, item)| (*item == Item::Instruction(record)).then_some(at))
        .collect()
}

fn unique_label(items: &[Item], label: Label) -> Option<usize> {
    let mut found = items
        .iter()
        .enumerate()
        .filter_map(|(at, item)| (*item == Item::Label(label)).then_some(at));
    let at = found.next()?;
    found.next().is_none().then_some(at)
}

fn source_entry(items: &[Item], at: usize) -> Option<(usize, Vec<Item>)> {
    if at > items.len() {
        return None;
    }
    let mut start = at;
    let mut end = at;
    while start > 0 && matches!(items[start - 1], Item::Label(_)) {
        start -= 1;
    }
    while matches!(items.get(end), Some(Item::Label(_))) {
        end += 1;
    }
    let finish = if matches!(items.get(end), Some(Item::Instruction(_))) {
        end + 1
    } else {
        end
    };
    let span = items.get(start..finish)?;
    if start == end
        || span.iter().any(|item| {
            matches!(item, Item::Label(label)
        if unique_label(items, *label).is_none())
        })
    {
        return None;
    }
    Some((start, span.to_vec()))
}

impl Assembler {
    /// Anchors the original while and optional manager entries, not their final widths.
    pub(crate) fn capture_held_await_source(
        &self,
        ordinary: &[(Label, Label)],
        loop_start: usize,
        manager_start: Option<usize>,
    ) -> Option<HeldAwaitSource> {
        if !matches!((ordinary.len(), manager_start), (3, None) | (2, Some(_))) {
            return None;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        let setups = self.capture_ordinary_try_setup_origins(ordinary)?;
        let positions = ordinary
            .iter()
            .map(|(label, _)| unique_label(items, *label))
            .collect::<Option<Vec<_>>>()?;
        if !(positions[0] < loop_start
            && loop_start < positions[1]
            && positions.windows(2).all(|pair| pair[0] < pair[1]))
            || manager_start.is_some_and(|at| at <= positions[1])
        {
            return None;
        }
        let mut entries = vec![source_entry(items, loop_start)?];
        if let Some(at) = manager_start {
            entries.push(source_entry(items, at)?);
        }
        Some(HeldAwaitSource {
            stream: self.emission_stream,
            setups,
            loop_start,
            entries,
        })
    }

    fn held_await_source_unchanged(&self, source: &HeldAwaitSource) -> bool {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return false;
        };
        let pairs = source
            .setups
            .origins
            .iter()
            .map(|origin| (origin.0, origin.1))
            .collect::<Vec<_>>();
        source.stream == self.emission_stream
            && self.capture_ordinary_try_setup_origins(&pairs).as_ref() == Some(&source.setups)
            && source.entries.iter().all(|(at, original)| {
                let Some(end) = at.checked_add(original.len()) else {
                    return false;
                };
                items.get(*at..end) == Some(original.as_slice())
                    && (*at == 0 || !matches!(items[*at - 1], Item::Label(_)))
                    && (!matches!(original.last(), Some(Item::Label(_)))
                        || !matches!(items.get(end), Some(Item::Label(_))))
                    && original.iter().all(|item| {
                        !matches!(item, Item::Label(label)
                        if unique_label(items, *label).is_none())
                    })
            })
            && unique_label(items, pairs[0].0).is_some_and(|at| at < source.loop_start)
            && unique_label(items, pairs[1].0).is_some_and(|at| source.loop_start < at)
    }

    /// The three-body inner-while route is separate from the retained-loop two-body route.
    pub(crate) fn completed_inner_while_await_has_frontier(
        &self,
        aliases: [Label; 4],
        source: &HeldAwaitSource,
    ) -> bool {
        source.setups.origins.len() == 3
            && source.entries.len() == 1
            && self.held_await_source_unchanged(source)
            && self.completed_ordinary_await_has_frontier(aliases, &source.setups)
    }

    /// Captures only the complete event just emitted by the async-manager return emitter.
    pub(crate) fn capture_completed_manager_await(
        &self,
        start: usize,
        entry: Label,
        location: SourceLocation,
        none: u32,
        depth: u32,
    ) -> Option<CompletedManagerAwait> {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        let span = items.get(start..)?;
        let [
            Item::Label(begin),
            Item::Instruction(swap3),
            Item::Instruction(swap2),
            Item::Instruction(none1),
            Item::Instruction(none2),
            Item::Instruction(none3),
            Item::Instruction(call),
            Item::Instruction(awaitable),
            Item::Instruction(initial),
            Item::Label(sending),
            Item::Instruction(send),
            Item::Label(yielded),
            Item::Instruction(yield_value),
            Item::Label(yield_end),
            Item::Instruction(resume),
            Item::Instruction(back),
            Item::Label(cleanup),
            Item::Instruction(cleanup_throw),
            Item::Label(completed),
            Item::Instruction(end_send),
            Item::Instruction(pop),
        ] = span
        else {
            return None;
        };
        let values = [
            (swap3, SWAP, 3, 3),
            (swap2, SWAP, 2, 3),
            (none1, LOAD_CONST, none, 4),
            (none2, LOAD_CONST, none, 5),
            (none3, LOAD_CONST, none, 6),
            (call, CALL, 3, 2),
            (awaitable, GET_AWAITABLE, 2, 2),
            (initial, LOAD_CONST, none, 3),
            (yield_value, YIELD_VALUE, 1, 3),
            (resume, RESUME, ResumeLocation::AFTER_AWAIT.argument(), 3),
            (cleanup_throw, CLEANUP_THROW, 0, 3),
            (end_send, END_SEND, 0, 2),
            (pop, POP_TOP, 0, 1),
        ];
        if *begin != entry
            || location.line < 0
            || depth != 3
            || values.iter().any(|(record, opcode, argument, depth)| {
                **record != Instruction::value_with_depth(*opcode, *argument, location, *depth)
            })
            || *send != Instruction::new(SEND, Operand::Forward(*completed), location, Some(3))
            || *back
                != Instruction::new(
                    JUMP_BACKWARD_NO_INTERRUPT,
                    Operand::Backward(*sending),
                    location,
                    Some(3),
                )
            || span.iter().any(|item| match item {
                Item::Label(label) => unique_label(items, *label).is_none(),
                Item::Instruction(record) => record.location != location,
            })
        {
            return None;
        }
        let mut regions = self.exception_regions.iter().filter(|region| {
            region.start == *yielded && region.end == *yield_end && region.target == *cleanup
        });
        let region = *regions.next()?;
        if regions.next().is_some()
            || region.depth != depth
            || region.preserve_lasti
            || region.is_scope_fallback
            || !matches!(region.priority, ExceptionRegionPriority::OriginalRange(a, b)
                if a == *yielded && b == *yield_end)
            || !self.manager_await_region_unchanged(&region)
            || !self.manager_await_roles_unchanged(*yielded, *completed)
        {
            return None;
        }
        let mut saved = Vec::new();
        for item in span {
            if let Item::Instruction(record) = item
                && !saved.iter().any(|(old, _)| old == record)
            {
                saved.push((*record, occurrences(items, *record)));
            }
        }
        Some(CompletedManagerAwait {
            stream: self.emission_stream,
            start,
            items: span.to_vec(),
            occurrences: saved,
            cleanup: region,
        })
    }

    fn manager_await_region_unchanged(&self, saved: &ExceptionRegion) -> bool {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return false;
        };
        let (Some(begin), Some(end), Some(_)) = (
            unique_label(items, saved.start),
            unique_label(items, saved.end),
            unique_label(items, saved.target),
        ) else {
            return false;
        };
        if begin >= end
            || self.exception_regions.iter().any(|region| {
                [region.start, region.end, region.target]
                    .iter()
                    .any(|label| unique_label(items, *label).is_none())
            })
        {
            return false;
        }
        let mut relevant = self.exception_regions.iter().filter(|region| {
            region.start == saved.start
                || region.target == saved.target
                || normal_finally_locations::same_priority(region.priority, saved.priority)
                || matches!((unique_label(items, region.start), unique_label(items, region.end)),
                    (Some(a), Some(b)) if a < end && begin < b)
        });
        relevant
            .next()
            .is_some_and(|region| fusion_owner_slots::same_manager_region(region, saved))
            && relevant.next().is_none()
    }

    fn manager_await_roles_unchanged(&self, yielded: Label, completed: Label) -> bool {
        let mut setups = self
            .exception_owner_provenance
            .setups
            .iter()
            .filter(|(label, _)| *label == yielded);
        setups.next() == Some(&(yielded, None))
            && setups.next().is_none()
            && self
                .exception_owner_provenance
                .block_boundaries
                .contains(&completed)
            && !self
                .exception_owner_provenance
                .ordinary_await_ends
                .iter()
                .any(|origin| origin.entry == completed)
    }

    /// Revalidates the genuine event immediately before its two ordinary POPs.
    pub(crate) fn completed_manager_await_has_frontier(
        &self,
        origin: &CompletedManagerAwait,
        aliases: [Label; 2],
        source: &HeldAwaitSource,
    ) -> bool {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return false;
        };
        let Some(end) = origin.start.checked_add(origin.items.len()) else {
            return false;
        };
        let (Some(Item::Label(yielded)), Some(Item::Label(completed))) =
            (origin.items.get(11), origin.items.get(18))
        else {
            return false;
        };
        origin.stream == self.emission_stream
            && source.setups.origins.len() == 2 && source.entries.len() == 2
            && self.held_await_source_unchanged(source)
            && source.entries.iter().all(|(at, entry)| at.checked_add(entry.len()).is_some_and(|end| end <= origin.start))
            && items.get(origin.start..end) == Some(origin.items.as_slice())
            && items.get(end..) == Some(aliases.map(Item::Label).as_slice())
            && origin.items.iter().chain(items.get(end..).unwrap_or_default()).all(|item|
                !matches!(item, Item::Label(label) if unique_label(items, *label).is_none()))
            && origin.occurrences.iter().all(|(record, original)| occurrences(items, *record) == *original)
            && self.manager_await_region_unchanged(&origin.cleanup)
            && self.manager_await_roles_unchanged(*yielded, *completed)
    }
}
