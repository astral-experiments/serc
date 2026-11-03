//! A try inside a named except arm can return through the enclosing binding.
//! Match every normal copy of its cleanup against the exceptional one.
use super::{Body, Handler, Instruction, Range};
use crate::source::{
    comparisons::same_instructions, dispatch, exceptions::boundary, regions::constant_return,
};

#[derive(Clone, Copy)]
pub(in crate::source) struct BoundParent {
    unwind: Handler,
    clear: usize,
    child_cleanup: usize,
}

impl BoundParent {
    pub(in crate::source) fn at_cleanup(
        owner: &Body<'_, '_, '_>,
        ops: &[Instruction],
        cleanup: usize,
    ) -> Option<Self> {
        let flow = &owner.flow;
        let unwind = flow.handler(cleanup)?;
        let clear = boundary(ops, unwind.target).ok()?;
        owner.cleared_name(ops, clear)?;
        dispatch::unwind(ops, flow, clear, 1)?;
        (unwind.lasti && unwind.depth == 1 && clear >= cleanup + 3).then_some(Self {
            unwind,
            clear,
            child_cleanup: cleanup,
        })
    }

    pub(in crate::source) fn child_unwind(
        owner: &Body<'_, '_, '_>,
        ops: &[Instruction],
        binding: usize,
    ) -> Option<Handler> {
        let first = binding + 1;
        let start = first + usize::from(ops.get(first)?.name == "NOP");
        let entry = ops.get(start)?;
        let h = owner.flow.handler(start)?;
        if h.lasti || h.depth != 1 || h.start != entry.offset {
            return None;
        }
        let dispatch = boundary(ops, h.target).ok()?;
        let (_, cleanup) = dispatch::unwind(ops, &owner.flow, dispatch, 2)?;
        Some(Self::at_cleanup(owner, ops, cleanup)?.unwind)
    }

    /// The outer `POP_EXCEPT` clears its binding and returns `None`. A child
    /// which takes this exit can instead fall out of the enclosing handler.
    pub(super) fn exit(
        self,
        owner: &Body<'_, '_, '_>,
        ops: &[Instruction],
        pop: usize,
    ) -> Option<Range<usize>> {
        let block = owner.flow.block(pop);
        let load = pop + 4;
        (ops[pop].name == "POP_EXCEPT"
            && block.ops.start == pop
            && block.height == Some(1)
            && owner.flow.handler(pop).is_none()
            && same_instructions(ops.get(pop + 1..load)?, &ops[self.clear..self.clear + 3])
            && owner.none_return(ops.get(load..block.ops.end)?))
        .then_some(load..block.ops.end)
    }

    pub(super) fn success(
        self,
        owner: &Body<'_, '_, '_>,
        ops: &[Instruction],
        span: Range<usize>,
    ) -> Option<Range<usize>> {
        match ops.get(span.clone())? {
            [jump] if jump.name == "JUMP_FORWARD" => owner
                .flow
                .jump(span.start)
                .and_then(|pop| self.exit(owner, ops, pop)),
            _ if !span.is_empty() => self
                .exit(owner, ops, span.start)
                .filter(|tail| tail.end == span.end),
            _ => None,
        }
    }

    /// The child restores its own exception immediately before its parent's.
    pub(in crate::source) fn handler_exit(
        self,
        owner: &Body<'_, '_, '_>,
        ops: &[Instruction],
        end: usize,
    ) -> Option<(usize, Option<usize>)> {
        let (start, inner_pop, ret) = if let Some((load, _)) = constant_return(ops, end) {
            let pop = load.checked_sub(5)?;
            if self.exit(owner, ops, pop + 1)?.end != end {
                return None;
            }
            (pop, pop, None)
        } else {
            let start = end.checked_sub(8)?;
            let [first, _, second, pop, clear @ .., ret] = ops.get(start..end)? else {
                return None;
            };
            let outer = owner.flow.handler(self.clear)?;
            if (first.name, first.arg, second.name, second.arg) != ("SWAP", 2, "SWAP", 2)
                || pop.name != "POP_EXCEPT"
                || ret.name != "RETURN_VALUE"
                || owner.flow.handler(start)?.target != ops[self.child_cleanup].offset
                || owner
                    .flow
                    .handler(start + 2)
                    .is_none_or(|h| (h.target, h.depth, h.lasti) != (outer.target, 1, true))
                || owner.flow.handler(start + 3).is_some()
                || !same_instructions(clear, &ops[self.clear..self.clear + 3])
            {
                return None;
            }
            (start, start + 1, Some(end - 1))
        };
        let h = owner.flow.handler(inner_pop)?;
        (ops[inner_pop].name == "POP_EXCEPT"
            && h.target == self.unwind.target
            && h.lasti
            && h.depth == self.unwind.depth)
            .then_some((start, ret))
    }

    pub(super) fn owns_tail(
        self,
        owner: &Body<'_, '_, '_>,
        ops: &[Instruction],
        mut at: usize,
        end: usize,
    ) -> bool {
        if end != self.clear {
            return false;
        }
        while at < end {
            let Some(tail) = self.exit(owner, ops, at) else {
                return false;
            };
            at = tail.end;
        }
        at == end
    }
}
