use std::fmt::Write as _;

use super::{Body, Bytecode, Instruction, Value};
use crate::target::code_flags;

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
            let binding = match ops[start].name {
                "STORE_FAST" => format!(" as {}", self.name(self.code.locals, ops[start].arg)?),
                "POP_TOP" => String::new(),
                _ => return Err("unsupported async with target".into()),
            };
            let suite = self.fragment(ops, start + 1, end, 0)?.suite()?;
            self.block(&format!("async with {cm}{binding}:"), &suite)?;
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
        if ops[bind].name != "STORE_FAST" {
            return Err("unsupported async for binding".into());
        }
        let name = self.name(self.code.locals, ops[bind].arg)?.to_owned();
        let back = find(ops, bind + 1, last, "JUMP_BACKWARD")?;
        let suite = self.fragment(ops, bind + 1, back, level)?.suite()?;
        self.block(&format!("async for {name} in {iterable}:"), &suite)?;
        // 3.15 has a separate END_ASYNC_FOR cleanup operand.
        Ok(last + 1)
    }

    pub(super) fn comprehension(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let iterable = self.expr()?;
        let header = find(ops, start, end, "FOR_ITER")?;
        let exit = ops[header]
            .jump
            .and_then(|offset| ops.binary_search_by_key(&offset, |op| op.offset).ok())
            .ok_or("invalid comprehension exit")?;
        let append = (header + 2..exit)
            .find(|&i| {
                matches!(
                    ops[i].name,
                    "LIST_APPEND" | "SET_ADD" | "MAP_ADD" | "YIELD_VALUE"
                )
            })
            .ok_or("missing comprehension result")?;
        let bind = ops[header + 1];
        let mut seed = Vec::new();
        let index = match bind.name {
            "STORE_FAST" => bind.arg,
            "STORE_FAST_LOAD_FAST" => {
                seed.push(Value::Expr(
                    self.name(self.code.locals, bind.arg & 15)?.to_owned(),
                ));
                bind.arg >> 4
            }
            _ => return Err("unsupported comprehension target".into()),
        };
        let name = self.name(self.code.locals, index)?.to_owned();
        let mut element = header + 2;
        let mut filters = String::new();
        while let Some(test) = ops[element..append]
            .iter()
            .position(|op| op.name.starts_with("POP_JUMP_IF_"))
            .map(|n| element + n)
        {
            let predicate_end =
                test - usize::from(test > element && ops[test - 1].name == "TO_BOOL");
            let pred = self
                .seeded(
                    ops,
                    element,
                    predicate_end,
                    level,
                    std::mem::take(&mut seed),
                )?
                .expression()?;
            let forward = ops[test]
                .jump
                .and_then(|offset| ops.binary_search_by_key(&offset, |op| op.offset).ok())
                .ok_or("invalid comprehension filter")?;
            if forward <= test || forward > append || ops[test].name != "POP_JUMP_IF_TRUE" {
                return Err("unsupported comprehension filter".into());
            }
            write!(filters, " if {pred}").unwrap();
            element = forward;
        }
        let mut values = self.seeded(ops, element, append, level, seed)?.stack;
        let Some(Value::Expr(last)) = values.pop() else {
            return Err("missing comprehension expression".into());
        };
        let item = if ops[append].name == "MAP_ADD" {
            let Some(Value::Expr(key)) = values.pop() else {
                return Err("missing comprehension key".into());
            };
            format!("{key}: {last}")
        } else {
            last
        };
        let (left, right) = if ops[append].name == "LIST_APPEND" {
            ("[", "]")
        } else if ops[append].name == "YIELD_VALUE" {
            ("(", ")")
        } else {
            ("{", "}")
        };
        self.push(format!(
            "{left}{item} for {name} in {iterable}{filters}{right}"
        ))?;
        if ops[append].name == "YIELD_VALUE" {
            return Ok(end);
        }
        let restore = find(ops, exit, end, "STORE_FAST")?;
        Ok(restore + 1)
    }
}
