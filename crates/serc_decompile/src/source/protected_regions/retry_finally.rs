//! Match a finally containing a retry loop whose exception dispatcher is
//! inline on unwind but placed after the return on the normal path.
use super::{Body, Clause, Handlers, Instruction, Protected, Range};
use crate::source::{
    dispatch,
    finalizers::{self, CopyExit},
};

pub(super) fn recover(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    suite: Range<usize>,
    dispatch: usize,
    cleanup: usize,
    inner: &Handlers,
) -> Option<Protected> {
    let flow = &owner.flow;
    let reraise = cleanup.checked_sub(1)?;
    if (ops[reraise].name, ops[reraise].arg) != ("RERAISE", 0)
        || suite.end >= dispatch
        || ops[dispatch + 1].name == "POP_TOP"
        || dispatch::typed_handler(flow, ops, dispatch + 1).is_some()
    {
        return None;
    }
    let unwind = flow.handler(dispatch)?;
    let right = flow
        .blocks_covering(dispatch + 1..reraise)
        .map(|b| b.ops.start)
        .find(|at| inner.contains_key(at))?;
    if right <= dispatch
        || !flow.blocks_covering(dispatch..right).all(|b| {
            flow.handler(b.ops.start).is_some_and(|h| {
                (h.target, h.depth, h.lasti) == (unwind.target, unwind.depth, true)
            }) || (flow.handler(b.ops.start).is_none()
                && ops[b.ops.clone()]
                    .iter()
                    .all(|op| op.name == "NOP" || op.name == "NOT_TAKEN"))
        })
    {
        return None;
    }
    let compare = |a: Range<usize>, b| owner.finally_copy_end(ops, a, b, CopyExit::Fallthrough);
    let normal = suite.end;
    let left = compare(normal..dispatch, dispatch + 1..right)?;
    let regular = inner.get(&left)?;
    let exceptional = inner.get(&right)?;
    let parts = |at, p: &Protected| {
        let Clause::Except {
            kind: Some(kind),
            binding,
            success,
            following,
            retry: true,
            finalizer: None,
        } = &p.clause
        else {
            return None;
        };
        (following.is_empty() && success.is_empty() && p.handler_tail.is_none()).then(|| {
            [
                at..p.end,
                kind.clone(),
                *binding..*binding + 1,
                p.handler.clone(),
            ]
        })
    };
    for (a, b) in parts(left, regular)?
        .into_iter()
        .zip(parts(right, exceptional)?)
    {
        if compare(a.clone(), b)? != a.end {
            return None;
        }
    }
    // The dispatcher restores the saved exception before an unmatched or
    // failing retry handler unwinds into the rest of the exceptional finally.
    let h = flow.handler(right)?;
    let target = ops.binary_search_by_key(&h.target, |op| op.offset).ok()?;
    let (_, child_unwind) = dispatch::unwind(ops, flow, target, h.depth + 1)?;
    if exceptional.next > reraise
        || regular.next >= dispatch
        || flow
            .handler(child_unwind)
            .is_none_or(|p| (p.target, p.depth, p.lasti) != (unwind.target, unwind.depth, true))
    {
        return None;
    }
    let next = if finalizers::owns_cleanup(ops, owner, exceptional.next..cleanup, unwind) {
        compare(regular.next..dispatch, exceptional.next..reraise)?
    } else {
        guarded_raise(
            ops,
            owner,
            regular.next..dispatch,
            exceptional.next..reraise,
            inner,
        )?
    };
    Some(Protected {
        return_exits: Vec::new(),
        end: normal,
        next,
        clause: Clause::Finally {
            deferred_return: next..next,
            terminal_unwind: None,
        },
        handler: normal..next,
        try_exits: None,
        handler_exits: None,
        handler_tail: None,
    })
}

/// A retry loop can save an exception and later raise it under a finally
/// that deletes the reference. Its unwind sits inline on the exceptional
/// path and out of line on the normal path.
fn guarded_raise(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    normal: Range<usize>,
    exceptional: Range<usize>,
    inner: &Handlers,
) -> Option<usize> {
    let flow = &owner.flow;
    let arm = |tail: Range<usize>| {
        let test = flow.block(tail.start).ops.end.checked_sub(1)?;
        let join = flow.jump(test)?;
        let start = (test + 1..join).find(|&i| !matches!(ops[i].name, "NOP" | "NOT_TAKEN"))?;
        let child = inner.get(&start)?;
        (test < tail.end
            && join <= tail.end
            && ops[test].name.starts_with("POP_JUMP_IF_")
            && matches!(
                child.clause,
                Clause::Finally {
                    terminal_unwind: None,
                    ..
                }
            )
            && child.end == child.next
            && child.next <= join
            && ops[child.end - 1].name == "RAISE_VARARGS")
            .then_some((test, start, join, child))
    };
    let (a, left, next, regular) = arm(normal.clone())?;
    let (b, right, reraise, raised) = arm(exceptional.clone())?;
    let parent = flow.handler(reraise)?;
    let h = flow.handler(right)?;
    let dispatch = ops.binary_search_by_key(&h.target, |op| op.offset).ok()?;
    let (_, unwind) = dispatch::unwind(ops, flow, dispatch, h.depth + 1)?;
    if ops[a].name != ops[b].name
        || regular.next != next
        || reraise != exceptional.end
        || raised.next != dispatch
        || unwind + 3 != reraise
        || flow
            .handler(unwind)
            .is_none_or(|u| (u.target, u.lasti, u.depth) != (parent.target, true, parent.depth))
    {
        return None;
    }
    for (x, y) in [
        (normal.start..a, exceptional.start..b),
        (a + 1..left, b + 1..right),
        (left..regular.end, right..raised.end),
        (regular.handler.clone(), raised.handler.clone()),
    ] {
        if owner.finally_copy_end(ops, x.clone(), y, CopyExit::Fallthrough) != Some(x.end) {
            return None;
        }
    }
    Some(next)
}
