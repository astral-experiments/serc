//! A branch of a handler can restore the prior exception and jump directly
//! to the continuation, leaving its sibling branch out of line.
use super::{Graph, Instruction};

#[derive(Clone, Copy)]
pub(in crate::source) struct HandlerRejoin {
    next: usize,
    unwind: usize,
    depth: usize,
}

impl HandlerRejoin {
    pub(in crate::source) fn new(
        ops: &[Instruction],
        flow: &Graph,
        binding: usize,
        next: usize,
    ) -> Option<Self> {
        let handler = flow.handler(binding)?;
        (ops[binding].name == "POP_TOP" && handler.lasti && handler.depth > 0).then_some(Self {
            next,
            unwind: handler.target,
            depth: handler.depth,
        })
    }

    /// Return the start of a complete exit block owned by this handler.
    pub(in crate::source) fn restore_before(
        self,
        ops: &[Instruction],
        flow: &Graph,
        end: usize,
    ) -> Option<usize> {
        let pop = end.checked_sub(2)?;
        let jump = pop + 1;
        let block = flow.block(pop);
        (ops[pop].name == "POP_EXCEPT"
            && matches!(
                ops[jump].name,
                "JUMP_FORWARD" | "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
            )
            && block.ops == (pop..end)
            && block.height == Some(self.depth)
            && flow.jump(jump) == Some(self.next)
            && flow.handler(pop).is_none()
            && flow.incoming(pop).all(|from| {
                flow.handler(from)
                    .is_some_and(|h| h.lasti && h.depth == self.depth && h.target == self.unwind)
            }))
        .then_some(pop)
    }
}
