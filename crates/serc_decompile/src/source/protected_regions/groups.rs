//! Place nested exception suites in dependency order.
use super::{Bytecode, Clause, Graph, Handler, Instruction, Protected};
use crate::source::comprehensions;
use rustc_hash::{FxHashMap, FxHashSet};

pub(super) struct HandlerGroup {
    pub(super) entries: Vec<Handler>,
    // Some enclosing tries protect only a child's cleanup copies. For a
    // combined except/finally, only the dispatcher's unwind is protected.
    pub(super) child: Option<usize>,
}

pub(super) fn groups(bc: &Bytecode, flow: &Graph) -> Vec<HandlerGroup> {
    let mut groups = FxHashMap::<_, Vec<_>>::default();
    for entry in &bc.handlers {
        if !entry.lasti {
            groups.entry(entry.target).or_default().push(*entry);
        }
    }
    let ops = &bc.ops;
    groups.retain(|_, entries| comprehensions::restore_range(ops, entries[0]).is_none());
    let index = |offset| ops.binary_search_by_key(&offset, |op| op.offset).ok();
    let enclosing = |entries: &[Handler]| {
        let first = entries[0];
        let child = index(first.start)?;
        let dispatch = index(first.target)?;
        if ops[dispatch].name != "PUSH_EXC_INFO" {
            return None;
        }
        let (_, unwind) = crate::source::dispatch::unwind(ops, flow, dispatch, first.depth + 1)?;
        let outer = flow.handler(unwind)?;
        let exceptional = match groups.get(&outer.target)?.as_slice() {
            [normal, exceptional]
                if normal.start >= first.target
                    && normal.end <= ops[unwind].offset
                    && ops[index(normal.start)?].name == "POP_EXCEPT"
                    && ops[index(normal.end)?.checked_sub(1)?].jump == Some(first.end) =>
            {
                exceptional
            }
            [normal, .., exceptional]
                if first.start < normal.start && normal.end <= first.target =>
            {
                exceptional
            }
            [exceptional] => exceptional,
            _ => return None,
        };
        (ops[child.checked_sub(1)?].name == "NOP"
            && !outer.lasti
            && outer.depth == first.depth
            && *exceptional == outer
            && outer.start == ops[unwind].offset
            && outer.end == ops.get(unwind + 3)?.offset)
            .then_some((outer.target, child))
    };
    let mut children = FxHashMap::default();
    for (target, child) in groups.values().filter_map(|group| enclosing(group)) {
        // A shared unwind alone cannot distinguish multiple possible owners.
        children
            .entry(target)
            .and_modify(|candidate| *candidate = None)
            .or_insert(Some(child));
    }
    let mut ordered: Vec<_> = groups
        .into_iter()
        .map(|(target, entries)| HandlerGroup {
            entries,
            child: children.get(&target).copied().flatten(),
        })
        .collect();
    // Recover inner suites before resolving their enclosing unwind edges.
    ordered.sort_unstable_by_key(|group| {
        std::cmp::Reverse(
            group
                .child
                .map_or(group.entries[0].start, |child| ops[child - 1].offset),
        )
    });
    ordered
}

/// Check both copies of a child's finalizer before placing its enclosing try.
/// Inner handlers may interrupt the normal copy and rejoin it out of line.
pub(super) fn encloses_finalizer(
    ops: &[Instruction],
    flow: &Graph,
    entries: &[Handler],
    start: usize,
    child: usize,
    suite: &Protected,
) -> bool {
    let [first, .., unwind] = entries else {
        return false;
    };
    if !matches!(suite.clause, Clause::Finally { .. })
        || suite.handler.start != start
        || (suite.end != start && !(suite.end + 1 == start && ops[suite.end].name == "NOP"))
        || (suite.next != suite.handler.end
            && super::finally::return_end(ops, suite.handler.end) != Some(suite.next))
        || !flow.entered_from(start, child..start)
        || entries.iter().any(|h| h.depth != first.depth)
    {
        return false;
    }
    if entries.len() == 2 {
        return ops
            .get(suite.handler.end)
            .is_some_and(|op| op.offset == first.end);
    }
    let Some(child_entry) = flow.handler(child) else {
        return false;
    };
    let mut pending = vec![start];
    let mut visited = FxHashSet::default();
    let mut routed = FxHashSet::default();
    while let Some(at) = pending.pop() {
        if at == suite.handler.end || !visited.insert(at) {
            continue;
        }
        if at < start || ops[at].offset >= child_entry.target {
            return false;
        }
        let block = flow.block(at);
        if let Some(h) = flow.handler(at) {
            if h.target == first.target {
                if h.lasti || h.depth != first.depth || h.start == unwind.start {
                    return false;
                }
                routed.insert(h.start);
            } else if let Ok(target) = ops.binary_search_by_key(&h.target, |op| op.offset) {
                pending.push(target);
            } else {
                return false;
            }
        } else if !ops[block.ops.clone()].iter().all(|op| {
            matches!(
                op.name,
                "NOP"
                    | "NOT_TAKEN"
                    | "JUMP_BACKWARD"
                    | "JUMP_BACKWARD_NO_INTERRUPT"
                    | "JUMP_FORWARD"
            )
        }) {
            return false;
        }
        let last = block.ops.end - 1;
        let edges = [flow.jump(last), flow.fallthrough(block)];
        if edges == [None, None] && !matches!(ops[last].name, "RAISE_VARARGS" | "RERAISE") {
            return false;
        }
        pending.extend(edges.into_iter().flatten());
    }
    // Every table range belongs to this cleanup or the child's exceptional
    // unwind. An unrelated arm routed to the same dispatcher is not nested.
    entries[..entries.len() - 1]
        .iter()
        .all(|entry| routed.contains(&entry.start))
}
