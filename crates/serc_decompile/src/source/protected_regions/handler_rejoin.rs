//! A branch of a handler can restore the prior exception and jump directly
//! to the continuation, leaving its sibling branch out of line.
use super::{Body, Graph, Instruction};

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
