//! Place nested exception suites in dependency order.
use super::{Bytecode, Graph, Handler};
use rustc_hash::FxHashMap;

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
            [normal, exceptional] if first.start < normal.start && normal.end <= first.target => {
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
    let children: FxHashMap<_, _> = groups
        .values()
        .filter_map(|group| enclosing(group))
        .collect();
    let mut ordered: Vec<_> = groups
        .into_iter()
        .map(|(target, entries)| HandlerGroup {
            entries,
            child: children.get(&target).copied(),
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
