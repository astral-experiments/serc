//! Source-local borrow barriers before a certified ordinary short await.

use rustc_hash::{FxHashMap, FxHashSet};

use super::fusion_owner_slots::same_manager_region;
use super::{
    Assembler, EmittedJump, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    Operand,
};
use crate::target::opcodes::{
    CLEANUP_THROW, END_SEND, GET_AWAITABLE, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, LOAD_CONST,
    LOAD_FAST, RESUME, SEND, YIELD_VALUE,
};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(super) struct ShortAwaitBorrowTarget {
    source: Vec<Item>,
    completed: Option<CompletedAwait>,
}

#[derive(Debug)]
struct CompletedAwait {
    items: Vec<Item>,
    prefix_len: usize,
    region: ExceptionRegion,
}

fn unique_labels(items: &[Item]) -> Option<FxHashMap<Label, usize>> {
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

fn anchored_start(items: &[Item], original: &[Item]) -> Option<usize> {
    let Item::Label(entry) = *original.first()? else {
        return None;
    };
    let index = items.iter().position(|item| *item == Item::Label(entry))?;
    if matches!(items.get(index.wrapping_sub(1)), Some(Item::Label(_))) {
        return None;
    }
    (items.get(index..index.checked_add(original.len())?)? == original).then_some(index)
}

impl Assembler {
    /// Captures a distinct normal source without widening the generic exit-size proof.
    pub(crate) fn record_noncopyable_short_await_borrow_exit(&mut self, source: EmittedJump) {
        if self.python_version != crate::PythonVersion::Py314 {
            return;
        }
        let index = source.instruction().item_index as usize;
        let items = self.linear_items();
        let mut start = index;
        while start > 0 && matches!(items[start - 1], Item::Label(_)) {
            start -= 1;
        }
        if start == index {
            return;
        }
        let original = items[start..=index].to_vec();
        self.defer_noncopyable_borrow_exit(source, false);
        self.deferred_noncopyable_borrow_exits
            .last_mut()
            .unwrap()
            .short_await = Some(Box::new(ShortAwaitBorrowTarget {
            source: original,
            completed: None,
        }));
    }

    /// Seals the exact ordinary-await emitter, including its actual None constant.
    pub(crate) fn record_short_await_borrow_target(
        &mut self,
        none: u32,
        send: Label,
        end: Label,
        yielded: Label,
        yielded_end: Label,
        cleanup: Label,
    ) {
        if !self
            .deferred_noncopyable_borrow_exits
            .iter()
            .any(|origin| origin.short_await.is_some())
        {
            return;
        }
        let Some(positions) = unique_labels(self.linear_items()) else {
            return;
        };
        let completed = self
            .deferred_noncopyable_borrow_exits
            .iter()
            .enumerate()
            .filter_map(|(index, origin)| {
                let target = origin.short_await.as_ref()?;
                if target.completed.is_some() {
                    return None;
                }
                let Operand::Forward(entry) = origin.original.operand() else {
                    return None;
                };
                let items = self.linear_items();
                let mut start = *positions.get(&entry)?;
                while start > 0 && matches!(items[start - 1], Item::Label(_)) {
                    start -= 1;
                }
                let prefix_len = items[start..]
                    .iter()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .count();
                let [
                    Item::Instruction(load),
                    Item::Instruction(awaitable),
                    Item::Instruction(constant),
                    Item::Label(actual_send),
                    Item::Instruction(send_record),
                    Item::Label(actual_yielded),
                    Item::Instruction(yield_record),
                    Item::Label(actual_yielded_end),
                    Item::Instruction(resume),
                    Item::Instruction(backedge),
                    Item::Label(actual_cleanup),
                    Item::Instruction(cleanup_record),
                    Item::Label(actual_end),
                    Item::Instruction(end_record),
                ] = &items[start + prefix_len..]
                else {
                    return None;
                };
                if (
                    *actual_send,
                    *actual_end,
                    *actual_yielded,
                    *actual_yielded_end,
                    *actual_cleanup,
                ) != (send, end, yielded, yielded_end, cleanup)
                    || !matches!(load.operand(), Operand::Value(_))
                    || load.location.line < 0
                    || awaitable.location.line < 0
                {
                    return None;
                }
                let location = awaitable.location;
                for (record, opcode, operand, depth, location) in [
                    (load, LOAD_FAST, load.operand(), 1, load.location),
                    (awaitable, GET_AWAITABLE, Operand::Value(0), 1, location),
                    (constant, LOAD_CONST, Operand::Value(none), 2, location),
                    (send_record, SEND, Operand::Forward(end), 2, location),
                    (yield_record, YIELD_VALUE, Operand::Value(1), 2, location),
                    (resume, RESUME, Operand::Value(3), 2, location),
                    (
                        backedge,
                        JUMP_BACKWARD_NO_INTERRUPT,
                        Operand::Backward(send),
                        2,
                        location,
                    ),
                    (
                        cleanup_record,
                        CLEANUP_THROW,
                        Operand::Value(0),
                        2,
                        location,
                    ),
                    (end_record, END_SEND, Operand::Value(0), 1, location),
                ] {
                    if *record != Instruction::new(opcode, operand, location, Some(depth)) {
                        return None;
                    }
                }
                let mut regions = self
                    .exception_owner_provenance
                    .ordinary_await_cleanups
                    .iter()
                    .filter(|region| region.start == yielded || region.target == cleanup);
                let region = *regions.next()?;
                if regions.next().is_some()
                    || region.start != yielded
                    || region.end != yielded_end
                    || region.target != cleanup
                    || region.depth != 2
                    || region.preserve_lasti
                    || region.is_scope_fallback
                    || !matches!(region.priority, ExceptionRegionPriority::OriginalRange(a, b)
                        if a == yielded && b == yielded_end)
                {
                    return None;
                }
                Some((
                    index,
                    CompletedAwait {
                        items: items[start..].to_vec(),
                        prefix_len,
                        region,
                    },
                ))
            })
            .collect::<Vec<_>>();
        for (index, completed) in completed {
            self.deferred_noncopyable_borrow_exits[index]
                .short_await
                .as_mut()
                .unwrap()
                .completed = Some(completed);
        }
    }

    /// Rechecks full original records and all interior entries before source-only mutation.
    pub(super) fn short_await_borrow_target_is_intact(
        &self,
        origin: &ShortAwaitBorrowTarget,
        source_index: usize,
        source: Instruction,
    ) -> bool {
        let Some(completed) = &origin.completed else {
            return false;
        };
        let items = self.linear_items();
        if unique_labels(items).is_none()
            || source.opcode != JUMP_FORWARD
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(source))
                .count()
                != 1
        {
            return false;
        }
        let Some(source_start) = anchored_start(items, &origin.source) else {
            return false;
        };
        let Some(start) = anchored_start(items, &completed.items) else {
            return false;
        };
        if source_index >= start
            || source_start + origin.source.len() != source_index + 1
            || matches!(items.get(source_index + 1), Some(Item::Instruction(_)))
        {
            return false;
        }
        let prefix = start + completed.prefix_len;
        let internal = completed.items[completed.prefix_len + 3..]
            .iter()
            .filter_map(|item| {
                if let Item::Label(label) = item {
                    Some(*label)
                } else {
                    None
                }
            })
            .collect::<FxHashSet<_>>();
        for (index, item) in items.iter().enumerate() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
                && internal.contains(&label)
                && index != prefix + 4
                && index != prefix + 9
            {
                return false;
            }
        }
        let region = &completed.region;
        if self
            .exception_regions
            .iter()
            .filter(|other| same_manager_region(region, other))
            .count()
            != 1
            || self
                .exception_owner_provenance
                .ordinary_await_cleanups
                .iter()
                .filter(|other| same_manager_region(region, other))
                .count()
                != 1
            || self.exception_regions.iter().any(|other| {
                (internal.contains(&other.target)
                    || internal.contains(&other.start)
                    || internal.contains(&other.end))
                    && !same_manager_region(region, other)
            })
        {
            return false;
        }
        let Item::Instruction(send) = items[prefix + 4] else {
            return false;
        };
        let Item::Label(end) = items[prefix + 12] else {
            return false;
        };
        let Item::Instruction(end_send) = items[prefix + 13] else {
            return false;
        };
        let mut origins = self
            .exception_owner_provenance
            .ordinary_await_ends
            .iter()
            .filter(|original| original.entry == end);
        let Some(original) = origins.next() else {
            return false;
        };
        origins.next().is_none()
            && original.send == send
            && original.end_send == end_send
            && original.cleanup == items[prefix + 10..prefix + 12]
            && original.bridge.is_none()
    }
}
