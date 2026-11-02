//! Recover synchronous comprehension clauses from their nested iterator loops.
use std::fmt::Write as _;

use super::{Body, Instruction, Value};

pub(super) fn is_collection_insert(op: &Instruction) -> bool {
    op.arg > 1
        && matches!(
            op.name,
            "LIST_APPEND" | "SET_ADD" | "MAP_ADD" | "LIST_EXTEND" | "SET_UPDATE" | "DICT_UPDATE"
        )
}

fn loop_exit(ops: &[Instruction], header: usize, end: usize) -> Result<usize, String> {
    let exit = ops[header]
        .jump
        .and_then(|target| ops.binary_search_by_key(&target, |op| op.offset).ok())
        .ok_or("invalid comprehension exit")?;
    if exit <= header + 1
        || exit >= end
        || ops[exit].name != "END_FOR"
        || (ops[exit - 1].name, ops[exit - 1].jump) != ("JUMP_BACKWARD", Some(ops[header].offset))
    {
        return Err("unsupported comprehension backedge".into());
    }
    Ok(exit)
}

/// If the outer backedge follows an inner iterator's cleanup, that iterator
/// supplies the next `for` clause. An iterator in the element is consumed before
/// the result is appended and does not share this cleanup.
fn inner_header(ops: &[Instruction], start: usize, back: usize) -> Result<Option<usize>, String> {
    let mut tail = back;
    if tail > start && matches!(ops[tail - 1].name, "POP_ITER" | "POP_TOP") {
        tail -= 1;
    }
    if tail <= start || ops[tail - 1].name != "END_FOR" {
        return Ok(None);
    }
    let end = ops[tail - 1].offset;
    ops[start..tail - 1]
        .iter()
        .position(|op| op.name == "FOR_ITER" && op.jump == Some(end))
        .map(|n| Some(start + n))
        .ok_or_else(|| "missing inner comprehension header".into())
}

fn clause_boundary(
    ops: &[Instruction],
    start: usize,
    back: usize,
    child: Option<usize>,
) -> Result<usize, String> {
    Ok(if let Some(child) = child {
        // 3.13 can emit two GET_ITERs for an inner clause.
        let mut get_iter = child;
        while get_iter > start && ops[get_iter - 1].name == "GET_ITER" {
            get_iter -= 1;
        }
        if get_iter == child {
            return Err("missing inner comprehension iterable".into());
        }
        get_iter
    } else {
        match &ops[start..back] {
            [.., yield_op, resume, pop]
                if (yield_op.name, resume.name, pop.name)
                    == ("YIELD_VALUE", "RESUME", "POP_TOP") =>
            {
                back - 3
            }
            [.., append] if is_collection_insert(append) => back - 1,
            _ => return Err("missing comprehension result".into()),
        }
    })
}

impl Body<'_, '_, '_> {
    pub(super) fn comprehension(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let mut iterable = self.expr()?;
        let outer = ops[start..end]
            .iter()
            .position(|op| op.name == "FOR_ITER")
            .map(|n| start + n)
            .ok_or("missing comprehension header")?;
        let outer_exit = loop_exit(ops, outer, end)?;
        let mut header = outer;
        let mut exit = outer_exit;
        let mut clauses = String::new();
        for _ in 0..32 {
            let back = exit - 1;
            let binding = self.binding(ops, header + 1, back)?;
            write!(clauses, " for {} in ({iterable})", binding.target).unwrap();
            self.charge(back - binding.next)?;
            let child = inner_header(ops, binding.next, back)?;
            let boundary = clause_boundary(ops, binding.next, back, child)?;
            let mut element = binding.next;
            let mut seed = binding.seed;
            for test in binding.next..boundary {
                let branch = ops[test];
                if !branch.name.starts_with("POP_JUMP_IF_") {
                    continue;
                }
                // A failed filter goes back to the current iterator. Other
                // branches here belong to the iterable or element expression.
                let skip = test + 1 + usize::from(ops[test + 1].name == "NOT_TAKEN");
                if skip + 1 > boundary
                    || ops[skip].name != "JUMP_BACKWARD"
                    || ops[skip].jump != Some(ops[header].offset)
                    || branch.jump != Some(ops[skip + 1].offset)
                {
                    continue;
                }
                let filter = self.comprehension_filter(
                    ops,
                    element..=test,
                    ops[skip + 1].offset,
                    level,
                    std::mem::take(&mut seed),
                )?;
                write!(clauses, " if {filter}").unwrap();
                element = skip + 1;
            }
            let body = self.seeded(ops, element, boundary, level, seed)?;
            if let Some(child) = child {
                iterable = body.expression()?;
                exit = loop_exit(ops, child, back)?;
                // 3.15 expands a starred generator element with an anonymous
                // inner loop that yields each item directly, without binding it.
                if self.m.version == serc_bytecode::PythonVersion::Py315
                    && self.m.text(self.code.name)? == "<genexpr>"
                    && let [item, resume, pop] = &ops[child + 1..exit - 1]
                    && (item.name, resume.name, pop.name) == ("YIELD_VALUE", "RESUME", "POP_TOP")
                {
                    self.push(format!("(*({iterable}){clauses})"))?;
                    return Ok(end);
                }
                header = child;
                continue;
            }
            let op = ops[boundary].name;
            let result = match op {
                "MAP_ADD" => {
                    let (key, value) = body.key_value()?;
                    format!("{{{key}: {value}{clauses}}}")
                }
                "LIST_APPEND" => format!("[{}{clauses}]", body.expression()?),
                "SET_ADD" => format!("{{{}{clauses}}}", body.expression()?),
                "LIST_EXTEND" => format!("[*({}){clauses}]", body.expression()?),
                "SET_UPDATE" => format!("{{*({}){clauses}}}", body.expression()?),
                "DICT_UPDATE" => format!("{{**({}){clauses}}}", body.expression()?),
                "YIELD_VALUE" => format!("({}{clauses})", body.expression()?),
                _ => unreachable!(),
            };
            self.push(result)?;
            if op == "YIELD_VALUE" {
                return Ok(end);
            }
            return self.restore_comprehension(ops, start, outer, outer_exit, end);
        }
        Err("comprehension clause nesting limit exceeded".into())
    }

    /// Alternatives in an `or` filter all jump to the same accepted path.
    /// The last comparison falls through to the iterator backedge on failure.
    fn comprehension_filter(
        &mut self,
        ops: &[Instruction],
        tests: std::ops::RangeInclusive<usize>,
        accepted: usize,
        level: usize,
        mut seed: Vec<Value>,
    ) -> Result<String, String> {
        let mut start = *tests.start();
        let mut alternatives = Vec::new();
        for test in tests {
            let op = ops[test];
            if op.jump != Some(accepted) || !op.name.starts_with("POP_JUMP_IF_") {
                continue;
            }
            let end = test - usize::from(test > start && ops[test - 1].name == "TO_BOOL");
            let value = self
                .seeded(ops, start, end, level, std::mem::take(&mut seed))?
                .expression()?;
            alternatives.push(match op.name {
                "POP_JUMP_IF_TRUE" => value,
                "POP_JUMP_IF_FALSE" => format!("not ({value})"),
                "POP_JUMP_IF_NONE" => format!("({value}) is None"),
                "POP_JUMP_IF_NOT_NONE" => format!("({value}) is not None"),
                _ => return Err("unsupported comprehension filter".into()),
            });
            start = test + 1 + usize::from(ops[test + 1].name == "NOT_TAKEN");
        }
        Ok(alternatives.join(" or "))
    }

    fn restore_comprehension(
        &mut self,
        ops: &[Instruction],
        start: usize,
        header: usize,
        exit: usize,
        end: usize,
    ) -> Result<usize, String> {
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
