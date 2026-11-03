use super::{Body, Bytecode, Instruction};
use serc_bytecode::target::{code_flags, operands::UnaryIntrinsic};

fn find(ops: &[Instruction], start: usize, end: usize, name: &str) -> Result<usize, String> {
    ops.get(start..end)
        .and_then(|slice| slice.iter().position(|op| op.name == name))
        .map(|n| start + n)
        .ok_or_else(|| format!("missing {name} in suspension or comprehension"))
}

impl Body<'_, '_, '_> {
    pub(super) fn suspended(&self) -> bool {
        self.code.flags
            & (code_flags::CO_GENERATOR | code_flags::CO_COROUTINE | code_flags::CO_ASYNC_GENERATOR)
            != 0
    }

    pub(super) fn suspension_scope(&mut self, bc: &Bytecode) -> Result<(), String> {
        let ops = &bc.ops;
        if ops.iter().any(|op| op.name == "WITH_EXCEPT_START") {
            self.handlers = super::protected_regions::suspended(bc, self);
            if !self.handlers.is_empty() {
                let end = self.flow.normal_end([0]);
                return self.region(ops, 0, end, 0);
            }
            return self.with_statement(bc, &bc.handlers, ops.len());
        }
        // A suspended body can end with a raise, even when another path
        // returns. The StopIteration handler marks the end of the normal CFG.
        let stop_handler = match ops.as_slice() {
            [.., handler, reraise]
                if (handler.name, handler.arg)
                    == (
                        "CALL_INTRINSIC_1",
                        UnaryIntrinsic::STOPITERATION_ERROR.argument(),
                    )
                    && (reraise.name, reraise.arg) == ("RERAISE", 1) =>
            {
                Some(handler.offset)
            }
            _ => None,
        };
        let end = if stop_handler.is_some()
            && !bc.handlers.is_empty()
            && bc
                .handlers
                .iter()
                .all(|h| Some(h.target) == stop_handler && h.depth == 0 && h.lasti)
        {
            ops.len() - 2
        } else {
            self.handlers = super::protected_regions::suspended(bc, self);
            ops.iter()
                .rposition(|op| matches!(op.name, "RETURN_VALUE" | "RETURN_CONST"))
                .map(|last| last + 1)
                .or_else(|| {
                    super::protected_regions::returnless_end(bc, &self.flow, &self.handlers)
                })
                .ok_or("unsupported returnless suspension scope")?
        };
        if self.depth > 0 && !self.class {
            self.index_terminal_returns(&ops[..end], stop_handler);
        }
        self.region(ops, 0, end, 0)
    }

    pub(super) fn delegated(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
    ) -> Result<usize, String> {
        let last = find(ops, start + 1, end, "END_SEND")?;
        let expr = self.expr()?;
        let kind = if ops[start].name == "GET_AWAITABLE" {
            "await"
        } else {
            "yield from"
        };
        self.push(format!("({kind} {expr})"))?;
        Ok(last + 1)
    }

    pub(super) fn async_loop(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let iterable = self.expr()?;
        let last = find(ops, start + 1, end, "END_ASYNC_FOR")?;
        let bind = find(ops, start + 1, last, "END_SEND")? + 1;
        let bind = bind + usize::from(ops[bind].name == "NOT_TAKEN");
        let binding = self.binding(ops, bind, last)?;
        let back = find(ops, binding.next, last, "JUMP_BACKWARD")?;
        let suite = self
            .seeded(ops, binding.next, back, level, binding.seed)?
            .suite()?;
        self.block(
            &format!("async for {} in {iterable}:", binding.target),
            &suite,
        )?;
        // 3.15 has a separate END_ASYNC_FOR cleanup operand.
        Ok(last + 1)
    }
}
