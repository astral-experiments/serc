//! An async iterator exhausts through the exception edge of `GET_ANEXT`.
//! The handler and its else suite can follow the normal return of the scope.
use super::{Body, Instruction};

impl Body<'_, '_, '_> {
    pub(in crate::source) fn async_loop(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let header = start + 1;
        let handler = self
            .flow
            .handler(header)
            .ok_or("missing async-for handler")?;
        let cleanup = super::super::exceptions::boundary(ops, handler.target)?;
        let send =
            header + 2 + usize::from(ops.get(header + 1).is_some_and(|op| op.name == "PUSH_NULL"));
        let received = self.flow.target(send)?;
        if handler.lasti
            || ops.get(cleanup).is_none_or(|op| op.name != "END_ASYNC_FOR")
            || ops.get(send).is_none_or(|op| op.name != "SEND")
            || ops.get(received).is_none_or(|op| op.name != "END_SEND")
            || self.flow.handler(received).is_none_or(|h| {
                (h.target, h.depth, h.lasti) != (handler.target, handler.depth, false)
            })
        {
            return Err("unsupported async-for dispatch".into());
        }
        let bind = received
            + 1
            + usize::from(
                ops.get(received + 1)
                    .is_some_and(|op| op.name == "NOT_TAKEN"),
            );
        let latch = self
            .flow
            .incoming(header)
            .rev()
            .find(|&i| (bind..end.min(cleanup)).contains(&i) && ops[i].name == "JUMP_BACKWARD")
            .ok_or("missing async-for latch")?;
        let iterable = self.expr()?;
        let binding = self.binding(ops, bind, latch)?;
        let active = self.loop_exits(ops, header, binding.next..latch, latch + 1, end)?;
        let outer = self.loop_edges.replace(active);
        let body = self.seeded(ops, binding.next, latch, level, binding.seed);
        self.loop_edges = outer;
        self.block(
            &format!("async for {} in {iterable}:", binding.target),
            &body?.suite()?,
        )?;

        let exhausted = cleanup + 1;
        if let Some(after) = active.after
            && after < exhausted
        {
            // Breaks jump past the latch to the continuation. Exhaustion
            // runs the out-of-line else, then jumps back to that same point.
            let mut i = exhausted;
            while i < ops.len() {
                self.charge(1)?;
                let block = self.flow.block(i);
                let last = block.ops.end - 1;
                if matches!(
                    ops[last].name,
                    "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
                ) && self.flow.jump(last) == Some(after)
                {
                    if exhausted < last {
                        let suite = self.fragment(ops, exhausted, last, level)?.suite()?;
                        self.block("else:", &suite)?;
                    }
                    return Ok(after);
                }
                if self.flow.jump(last).is_some() || self.flow.fallthrough(block) != Some(last + 1)
                {
                    break;
                }
                i = last + 1;
            }
            return Err("unsupported async-for exhaustion path".into());
        }
        self.loop_else(ops, exhausted, active, level)
    }
}
