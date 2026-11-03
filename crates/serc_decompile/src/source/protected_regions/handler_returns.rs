//! Return paths which must unwind both the current and the enclosing except.
use super::{Body, BoundParent, Clause, Graph, Handler, Handlers, Instruction, Range};
use crate::source::{dispatch, regions};

pub(super) fn enclosing_exception(
    ops: &[Instruction],
    flow: &Graph,
    at: usize,
    child_cleanup: usize,
) -> bool {
    dispatch::unwind(ops, flow, at, 1).is_some_and(|(outer, _)| {
        flow.handler(child_cleanup)
            .is_some_and(|h| (h.target, h.depth, h.lasti) == (outer.target, outer.depth, true))
    })
}

/// Both handlers must restore their saved exception before the return. A
/// computed value stays on the stack across both pops; a constant loads after.
pub(super) fn nested_handler_return(
    ops: &[Instruction],
    flow: &Graph,
    body: Range<usize>,
    cleanup: usize,
) -> Option<(usize, usize)> {
    let (expression_end, first, second, tail) =
        if let Some((load, _)) = regions::constant_return(ops, body.end) {
            (load.checked_sub(2)?, load - 2, load - 1, load)
        } else {
            let start = body.end.checked_sub(5)?;
            let [swap, _, again, _, ret] = ops.get(start..body.end)? else {
                return None;
            };
            if (swap.name, swap.arg, again.name, again.arg, ret.name)
                != ("SWAP", 2, "SWAP", 2, "RETURN_VALUE")
                || flow.handler(start)?.target != ops[cleanup].offset
            {
                return None;
            }
            (start, start + 1, start + 3, start + 4)
        };
    (expression_end >= body.start
        && ops[first].name == "POP_EXCEPT"
        && ops[second].name == "POP_EXCEPT"
        && enclosing_exception(ops, flow, first, cleanup)
        && flow.handler(second).is_none())
    .then_some((expression_end, tail))
}

/// A nested try/except can terminate a matching arm. Any prefix runs with
/// the outer exception active. Neither out-of-line dispatch belongs in the
/// arm's text.
pub(in crate::source) fn returning_handler_child(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    arm: Range<usize>,
    outer: Handler,
    inner: &Handlers,
) -> Option<(usize, Option<Range<usize>>)> {
    let flow = &owner.flow;
    let mut start = arm.start;
    while start < arm.end && !inner.contains_key(&start) {
        let block = flow.block(start);
        let next = block.ops.end.min(arm.end);
        if !ops[start..next].iter().all(|op| op.name == "NOP")
            && (flow
                .handler(start)
                .is_none_or(|h| (h.target, h.depth, h.lasti) != (outer.target, outer.depth, true))
                || flow.jump(next - 1).is_some()
                || flow.fallthrough(block) != Some(next))
        {
            return None;
        }
        start = next;
    }
    if start >= arm.end {
        return None;
    }
    let child = inner.get(&start)?;
    let h = flow.handler(start)?;
    let dispatch = child.next;
    if ops.get(dispatch)?.offset != h.target || h.lasti || h.depth != outer.depth {
        return None;
    }
    let (_, cleanup) = dispatch::unwind(ops, flow, dispatch, h.depth + 1)?;
    if matches!(child.clause, Clause::Except { .. })
        && let Some(bound) = BoundParent::at_cleanup(owner, ops, cleanup)
        && let Some(tail) = bound.success(owner, ops, child.end..dispatch)
        && bound.owns_tail(owner, ops, cleanup + 3, arm.end)
        && flow
            .handler(arm.end)
            .is_some_and(|h| h.lasti && h.target == outer.target && h.depth == outer.depth)
    {
        return Some((dispatch, Some(tail)));
    }
    let parent = flow.handler(cleanup)?;
    let terminal = match child.clause {
        Clause::Except { .. } => child.end == dispatch,
        Clause::Finally { .. } => {
            child.handler.end == dispatch && ops[dispatch - 1].name == "RAISE_VARARGS"
        }
    };
    (terminal
        && cleanup + 3 == arm.end
        && parent.lasti
        && parent.depth == outer.depth
        && parent.target == outer.target
        && parent.end > ops[arm.end].offset
        && (ops[arm.end].name, ops[arm.end].arg) == ("RERAISE", 0))
        .then_some((dispatch, None))
}
