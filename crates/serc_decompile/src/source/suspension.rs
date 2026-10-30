use super::{Body, Bytecode, Instruction};
use serc_bytecode::target::code_flags;

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
        if let Some(with) = ops.iter().position(|op| op.name == "WITH_EXCEPT_START")
            && let Some(handler) = with.checked_sub(1)
            && let Some(h) = bc.handlers.iter().find(|h| h.target == ops[handler].offset)
        {
            let enter = ops
                .iter()
                .position(|op| op.name == "BEFORE_ASYNC_WITH" || op.name == "COPY")
                .ok_or("missing async manager entry")?;
            self.region(ops, 0, enter, 0)?;
            let cm = self.expr()?;
            let idx = |offset| {
                ops.binary_search_by_key(&offset, |op| op.offset)
                    .map_err(|_| "invalid async handler")
            };
            let start = idx(h.start)?;
            let end = idx(h.end)?;
            let (item, body_start, seed) = self.manager_item(cm, ops, start, end)?;
            let suite = self.seeded(ops, body_start, end, 0, seed)?.suite()?;
            self.block(&format!("async with {item}:"), &suite)?;
            let close = find(ops, end, handler, "END_SEND")?;
            let ret = ops[close + 1..handler]
                .iter()
                .position(|op| matches!(op.name, "RETURN_VALUE" | "RETURN_CONST"))
                .map(|n| close + 1 + n)
                .ok_or("unsupported async manager continuation")?;
            return self.region(ops, close + 2, ret + 1, 0);
        }
        let stop = ops
            .iter()
            .rposition(|op| matches!(op.name, "RETURN_VALUE" | "RETURN_CONST"))
            .ok_or("generator has no return")?;
        self.region(ops, 0, stop + 1, 0)
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
