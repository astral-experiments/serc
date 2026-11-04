//! A branch of a handler can restore the prior exception and jump directly
//! to the continuation, leaving its sibling branch out of line.
use super::{Body, Clause, Graph, Handler, Handlers, Instruction};
use std::ops::Range;

/// A retry at the tail of an except can succeed and jump straight to the
/// original try's continuation. On failure its own except raises or reraises
/// through the outer unwind. Only the retry reaches the saved-exception pop.
pub(super) fn after_retry(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    arm: Range<usize>,
    join: usize,
    unwind: Handler,
    inner: &Handlers,
) -> Option<usize> {
    let flow = &owner.flow;
    let mut exits = flow.incoming(join).filter(|i| arm.contains(i));
    let jump = exits.next()?;
    let pop = jump.checked_sub(1)?;
    if exits.next().is_some()
        || flow.jump(jump) != Some(join)
        || ops[pop].name != "POP_EXCEPT"
        || flow.block(pop).ops != (pop..jump + 1)
        || flow.block(pop).height != Some(unwind.depth)
    {
        return None;
    }
    let protected = flow.handler(pop.checked_sub(1)?)?;
    let start = ops
        .binary_search_by_key(&protected.start, |op| op.offset)
        .ok()?;
    let child = inner.get(&start)?;
    let cleanup = child.unwind(ops, flow, protected)?;
    let parent = flow.handler(cleanup)?;
    if !arm.contains(&start)
        || protected.lasti
        || protected.depth != unwind.depth
        || child.end != pop
        || child.next != pop
        || cleanup + 3 != arm.end
        || !flow.incoming(pop).all(|i| (start..pop).contains(&i))
        || !matches!(&child.clause, Clause::Except { following, finalizer: None, .. } if following.is_empty())
        || ops[child.handler.end.checked_sub(1)?].name != "RAISE_VARARGS"
        || (parent.target, parent.depth, parent.lasti) != (unwind.target, unwind.depth, true)
    {
        return None;
    }
    flow.blocks_covering(arm.start..start)
        .all(|b| {
            let span = b.ops.start.max(arm.start)..b.ops.end.min(start);
            (ops[span.clone()]
                .iter()
                .all(|op| matches!(op.name, "NOP" | "NOT_TAKEN"))
                || flow.handler(span.start).is_some_and(|h| {
                    (h.target, h.depth, h.lasti) == (unwind.target, unwind.depth, true)
                }))
                && [flow.jump(b.ops.end - 1), flow.fallthrough(b)]
                    .into_iter()
                    .flatten()
                    .all(|to| (arm.start..=start).contains(&to))
        })
        .then_some(pop)
}

#[derive(Clone, Copy)]
pub(in crate::source) struct HandlerRejoin {
    next: usize,
    unwind: usize,
    depth: usize,
    clear: Option<usize>,
}

impl HandlerRejoin {
    pub(in crate::source) fn new(
        ops: &[Instruction],
        owner: &Body<'_, '_, '_>,
        binding: usize,
        next: usize,
    ) -> Option<Self> {
        let (handler, clear) = if ops[binding].name == "POP_TOP" {
            (owner.flow.handler(binding)?, None)
        } else {
            let h = owner.binding_unwind(ops, binding)?;
            let clear = ops.binary_search_by_key(&h.target, |op| op.offset).ok()?;
            let store = owner.cleared_name(ops, clear)?;
            if (store.name, store.arg) != (ops[binding].name, ops[binding].arg) {
                return None;
            }
            (h, Some(clear))
        };
        (handler.lasti && handler.depth > 0).then_some(Self {
            next,
            unwind: handler.target,
            depth: handler.depth,
            clear,
        })
    }

    /// Return the start of a complete exit block owned by this handler.
    pub(in crate::source) fn restore_before(
        self,
        ops: &[Instruction],
        flow: &Graph,
        end: usize,
    ) -> Option<usize> {
        let jump = end.checked_sub(1)?;
        if flow.jump(jump)? != self.next {
            return None;
        }
        self.restored_pop(ops, flow, jump)
    }

    /// A named arm clears the binding before it continues the enclosing loop.
    pub(in crate::source) fn named_continue(
        self,
        ops: &[Instruction],
        flow: &Graph,
        pop: usize,
        depth: usize,
    ) -> Option<usize> {
        self.clear?;
        let jump = pop + 4;
        (self.depth == depth && self.restored_pop(ops, flow, jump)? == pop).then_some(jump)
    }

    fn restored_pop(self, ops: &[Instruction], flow: &Graph, jump: usize) -> Option<usize> {
        let pop = jump.checked_sub(if self.clear.is_some() { 4 } else { 1 })?;
        ops.get(jump)?;
        let block = flow.block(pop);
        (ops[pop].name == "POP_EXCEPT"
            && matches!(
                ops[jump].name,
                "JUMP_FORWARD" | "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
            )
            && block.ops.end == jump + 1
            && (block.ops.start == pop
                || (block.ops.start + 1 == pop && ops[block.ops.start].name == "NOT_TAKEN"))
            && block.height == Some(self.depth)
            && self.clear.is_none_or(|at| {
                crate::source::comparisons::same_instructions(&ops[pop + 1..jump], &ops[at..at + 3])
            })
            && flow
                .handler(pop)
                .is_none_or(|h| h.target != self.unwind && self.clear.is_some())
            && flow.incoming(pop).all(|from| {
                flow.handler(from)
                    .is_some_and(|h| h.lasti && h.depth == self.depth && h.target == self.unwind)
            }))
        .then_some(pop)
    }
}
