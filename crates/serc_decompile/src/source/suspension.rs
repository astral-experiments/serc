use std::fmt::Write as _;

use super::{Body, Bytecode, Instruction, Value};
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
        let binding = self.binding(ops, header + 1, append)?;
        let name = binding.target;
        let mut seed = binding.seed;
        let mut element = binding.next;
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
        // Each saved local is restored in reverse order after the iterator cleanup.
        let mut saved: Vec<_> = ops[start..header]
            .iter()
            .filter(|op| op.name == "LOAD_FAST_AND_CLEAR")
            .map(|op| op.arg)
            .collect();
        let mut restore = exit + 1;
        if restore < end
            && matches!(ops[restore].name, "POP_ITER" | "POP_TOP")
            && self.m.version >= serc_bytecode::PythonVersion::Py313
        {
            restore += 1;
        }
        if restore >= end || saved.is_empty() {
            return Err("missing comprehension local restore".into());
        }
        match ops[restore].name {
            "SWAP" if ops[restore].arg as usize == saved.len() + 1 => {}
            // The optimizer can move the result store or discard before the
            // restores. The entry SWAP then leaves the last saved local deepest.
            "STORE_FAST" | "POP_TOP" => {
                self.step(ops[restore])?;
                saved.rotate_right(1);
            }
            _ => return Err("unsupported comprehension result cleanup".into()),
        }
        restore += 1;
        for local in saved.into_iter().rev() {
            if restore >= end || (ops[restore].name, ops[restore].arg) != ("STORE_FAST", local) {
                return Err("unsupported comprehension local restore".into());
            }
            restore += 1;
        }
        Ok(restore)
    }
}
