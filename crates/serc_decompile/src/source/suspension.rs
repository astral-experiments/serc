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
            return self.suspended_managers(bc);
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
            self.handlers = super::protected_regions::suspended(bc, &self.flow);
            ops.iter()
                .rposition(|op| matches!(op.name, "RETURN_VALUE" | "RETURN_CONST"))
                .ok_or("generator has no return")?
                + 1
        };
        if self.depth > 0 && !self.class {
            self.index_terminal_returns(&ops[..end], stop_handler);
        }
        self.region(ops, 0, end, 0)
    }

    fn suspended_managers(&mut self, bc: &Bytecode) -> Result<(), String> {
        let ops = &bc.ops;
        let index = |offset| {
            ops.binary_search_by_key(&offset, |op| op.offset)
                .map_err(|_| "invalid suspended manager boundary")
        };
        // The normal path precedes the out-of-line throw and manager handlers.
        let normal_end = bc
            .handlers
            .iter()
            .filter_map(|h| index(h.target).ok())
            .filter(|&i| {
                matches!(ops[i].name, "CLEANUP_THROW" | "PUSH_EXC_INFO")
                    || (ops[i].name, ops[i].arg)
                        == (
                            "CALL_INTRINSIC_1",
                            UnaryIntrinsic::STOPITERATION_ERROR.argument(),
                        )
            })
            .min()
            .ok_or("missing suspended manager cleanup")?;
        let mut cursor = 0;
        let mut ranges = bc.handlers.iter();
        while let Some(entry) = ranges.next() {
            let start = index(entry.start)?;
            if start >= normal_end {
                break;
            }
            let handler = index(entry.target)?;
            if start < cursor
                || !entry.lasti
                || ops[handler].name != "PUSH_EXC_INFO"
                || ops
                    .get(handler + 1)
                    .is_none_or(|op| op.name != "WITH_EXCEPT_START")
            {
                continue;
            }
            let asynchronous = ops
                .get(handler + 2)
                .is_some_and(|op| (op.name, op.arg) == ("GET_AWAITABLE", 2));
            let protocol_end = if asynchronous {
                ops[cursor..start]
                    .iter()
                    .rposition(|op| (op.name, op.arg) == ("GET_AWAITABLE", 1))
                    .map(|n| cursor + n)
                    .ok_or("missing awaited manager entry")?
            } else {
                start
            };
            let enter = cursor
                + super::exceptions::manager_entry(&ops[cursor..protocol_end], asynchronous)?;
            self.region(ops, cursor, enter, 0)?;
            let manager = self.expr()?;

            // Await temporarily protects the yield with a CLEANUP_THROW handler.
            // Its resume must return to exactly the same manager at the same depth.
            let mut end = entry.end;
            while let [suspend, resume, ..] = ranges.as_slice()
                && suspend.start == end
                && suspend.end == resume.start
                && (resume.target, resume.depth, resume.lasti)
                    == (entry.target, entry.depth, entry.lasti)
                && ops[index(suspend.start)?].name == "YIELD_VALUE"
                && ops[index(suspend.target)?].name == "CLEANUP_THROW"
            {
                end = resume.end;
                ranges.nth(1);
            }
            let end = index(end)?;
            if end > normal_end || start >= end {
                return Err("unsupported suspended manager extent".into());
            }
            let (item, body, seed) = self.manager_item(manager, ops, start, end)?;
            let (suite, returns) = self.seeded(ops, body, end, 0, seed)?.pending_return()?;
            let header = if asynchronous { "async with" } else { "with" };
            self.block(&format!("{header} {item}:"), &suite)?;

            let call = find(ops, end, normal_end, "CALL")?;
            let close = if asynchronous {
                if ops
                    .get(call + 1)
                    .is_none_or(|op| (op.name, op.arg) != ("GET_AWAITABLE", 2))
                {
                    return Err("missing awaited manager exit".into());
                }
                find(ops, call + 2, normal_end, "END_SEND")?
            } else {
                call
            };
            if ops.get(close + 1).is_none_or(|op| op.name != "POP_TOP") {
                return Err("unsupported suspended manager exit".into());
            }
            cursor = close + 2;
            if returns {
                if ops.get(cursor).is_none_or(|op| op.name != "RETURN_VALUE") {
                    return Err("unsupported return through suspended manager".into());
                }
                return Ok(());
            }
        }
        self.region(ops, cursor, normal_end, 0)
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
