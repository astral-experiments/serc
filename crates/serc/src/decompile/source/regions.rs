//! Structure a slice of the instruction graph. Each arm gets a fresh operand
//! stack; values that survive a join must be expressible as a Python expression.
use std::fmt::Write as _;

use super::{Body, Instruction, Value};

pub(super) struct Fragment {
    lines: String,
    stack: Vec<Value>,
}

impl Fragment {
    pub(super) fn expression(mut self) -> Result<String, String> {
        if self.lines.is_empty()
            && self.stack.len() == 1
            && let Some(Value::Expr(value)) = self.stack.pop()
        {
            Ok(value)
        } else {
            Err("region is not a single expression".into())
        }
    }
    pub(super) fn pending_return(mut self) -> Result<(String, bool), String> {
        match self.stack.pop() {
            None => Ok((self.lines, false)),
            Some(Value::Expr(value)) if self.stack.is_empty() => {
                writeln!(self.lines, "return {value}").unwrap();
                Ok((self.lines, true))
            }
            _ => Err("unsupported protected stack outcome".into()),
        }
    }
    pub(super) fn suite(self) -> Result<String, String> {
        if self.stack.is_empty() {
            Ok(self.lines)
        } else {
            Err("unresolved stack value at statement boundary".into())
        }
    }
}

fn destination(ops: &[Instruction], i: usize) -> Result<usize, String> {
    let offset = ops[i].jump.ok_or("missing jump destination")?;
    ops.binary_search_by_key(&offset, |op| op.offset)
        .map_err(|_| "invalid jump destination".into())
}

impl Body<'_, '_, '_> {
    pub(super) fn fragment(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<Fragment, String> {
        let outer_stack = std::mem::take(&mut self.stack);
        let outer_lines = std::mem::take(&mut self.lines);
        self.region(ops, start, end, level + 1)?;
        Ok(Fragment {
            lines: std::mem::replace(&mut self.lines, outer_lines),
            stack: std::mem::replace(&mut self.stack, outer_stack),
        })
    }

    pub(super) fn block(&mut self, header: &str, suite: &str) -> Result<(), String> {
        self.line(header)?;
        if suite.is_empty() {
            self.line("    pass")?;
        }
        for line in suite.lines() {
            self.line(&format!("    {line}"))?;
        }
        Ok(())
    }

    pub(super) fn region(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<(), String> {
        if level > 32 || end > ops.len() || start > end {
            return Err("unsupported region bounds or nesting".into());
        }
        let mut i = start;
        while i < end {
            let op = ops[i];
            let next = match op.name {
                "JUMP_BACKWARD" | "JUMP_FORWARD" if self.loop_transfer(ops, i).is_some() => {
                    self.line(self.loop_transfer(ops, i).unwrap())?;
                    Ok(i + 1)
                }
                "POP_TOP" | "POP_ITER"
                    if self.stack.is_empty()
                        && i + 1 < end
                        && (self.loop_transfer(ops, i + 1) == Some("break")
                            || (ops[i + 1].name == "POP_TOP"
                                && i + 2 < end
                                && self.loop_transfer(ops, i + 2) == Some("break"))) =>
                {
                    Ok(i + 1)
                }
                "COPY" if op.arg == 1 => self.boolean(ops, i, end, level),
                "GET_ITER" if ops.get(i + 1).is_some_and(|next| next.name == "FOR_ITER") => {
                    self.for_loop(ops, i + 1, end, level)
                }
                "POP_JUMP_IF_FALSE"
                | "POP_JUMP_IF_TRUE"
                | "POP_JUMP_IF_NONE"
                | "POP_JUMP_IF_NOT_NONE" => self.conditional(ops, i, end, level),
                "TO_BOOL"
                    if ops
                        .get(i + 1)
                        .is_some_and(|op| op.name.starts_with("POP_JUMP_IF_")) =>
                {
                    Ok(i + 1)
                }
                _ => self.step(op).map(|()| i + 1),
            }
            .map_err(|err| format!("byte {} ({}): {err}", op.offset, op.name))?;
            if next <= i || next > end {
                return Err(format!("branch escapes region at byte {}", op.offset));
            }
            i = next;
        }
        Ok(())
    }

    fn boolean(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let branch = i + 1 + usize::from(ops.get(i + 1).is_some_and(|op| op.name == "TO_BOOL"));
        let operator = match ops.get(branch).map(|op| op.name) {
            Some("POP_JUMP_IF_FALSE") => "and",
            Some("POP_JUMP_IF_TRUE") => "or",
            _ => return Err("unsupported copied value".into()),
        };
        let discard =
            branch + 1 + usize::from(ops.get(branch + 1).is_some_and(|op| op.name == "NOT_TAKEN"));
        let target = destination(ops, branch)?;
        if ops.get(discard).is_none_or(|op| op.name != "POP_TOP")
            || target > end
            || target <= discard + 1
        {
            return Err("unsupported short-circuit shape".into());
        }
        let lhs = self.expr()?;
        let rhs = self
            .fragment(ops, discard + 1, target, level)?
            .expression()?;
        self.push(format!("({lhs} {operator} {rhs})"))?;
        Ok(target)
    }

    fn loop_transfer(&self, ops: &[Instruction], i: usize) -> Option<&'static str> {
        let (header, after) = self.loop_edges?;
        if !matches!(ops[i].name, "JUMP_BACKWARD" | "JUMP_FORWARD") {
            return None;
        }
        let dest = destination(ops, i).ok()?;
        if dest == header {
            Some("continue")
        } else if Some(dest) == after {
            Some("break")
        } else {
            None
        }
    }

    fn for_loop(
        &mut self,
        ops: &[Instruction],
        header: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let exit = destination(ops, header)?;
        if exit > end
            || exit <= header + 2
            || ops[exit].name != "END_FOR"
            || ops[exit - 1].jump != Some(ops[header].offset)
        {
            return Err("unsupported for-loop exit".into());
        }
        let store = ops[header + 1];
        let table = match store.name {
            "STORE_NAME" => self.code.names,
            "STORE_FAST" => self.code.locals,
            _ => return Err("unsupported for-loop binding".into()),
        };
        let name = self.name(table, store.arg)?.to_owned();
        let iterable = self.expr()?;
        let after_cleanup = exit
            + 1
            + usize::from(
                ops.get(exit + 1)
                    .is_some_and(|op| matches!(op.name, "POP_ITER" | "POP_TOP")),
            );
        let mut after_else = None;
        for index in header + 2..exit - 1 {
            if ops[index].name == "JUMP_FORWARD" {
                let dest = destination(ops, index)?;
                if dest >= after_cleanup && dest <= end {
                    if after_else.is_some_and(|other| other != dest) {
                        return Err("multiple for-loop exit targets".into());
                    }
                    after_else = Some(dest);
                }
            }
        }
        let outer = self.loop_edges.replace((header, after_else));
        let suite = self.fragment(ops, header + 2, exit - 1, level)?.suite()?;
        self.loop_edges = outer;
        self.block(&format!("for {name} in {iterable}:"), &suite)?;
        if let Some(after) = after_else
            && after > after_cleanup
        {
            let suite = self.fragment(ops, after_cleanup, after, level)?.suite()?;
            self.block("else:", &suite)?;
            Ok(after)
        } else {
            Ok(after_cleanup)
        }
    }

    fn conditional(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let target = destination(ops, i)?;
        if target <= i || target > end {
            return Err("unsupported conditional destination".into());
        }
        let value = self.expr()?;
        let test = match ops[i].name {
            "POP_JUMP_IF_FALSE" => value,
            "POP_JUMP_IF_TRUE" => format!("not ({value})"),
            "POP_JUMP_IF_NONE" => format!("({value}) is not None"),
            _ => format!("({value}) is None"),
        };
        let start = i + 1 + usize::from(ops.get(i + 1).is_some_and(|op| op.name == "NOT_TAKEN"));
        let tail = target - 1;
        if self.loop_transfer(ops, tail).is_some() {
            let suite = self.fragment(ops, start, target, level)?.suite()?;
            self.block(&format!("if {test}:"), &suite)?;
            return Ok(target);
        }
        if ops[tail].name == "JUMP_BACKWARD" {
            let back = destination(ops, tail)?;
            let body_end = if back <= i {
                tail
            } else if back == start && tail > start && ops[tail - 1].jump == ops[i].jump {
                // CPython 3.12 duplicates the condition at the bottom. Match
                // its complete operand sequence before removing the copy.
                let mut before = i;
                let mut copy = tail - 1;
                while copy > start
                    && before > 0
                    && ops[before - 1].opcode == ops[copy - 1].opcode
                    && ops[before - 1].arg == ops[copy - 1].arg
                {
                    before -= 1;
                    copy -= 1;
                }
                copy
            } else {
                return Err("unsupported loop condition".into());
            };
            let suite = self.fragment(ops, start, body_end, level)?.suite()?;
            self.block(&format!("while {test}:"), &suite)?;
            return Ok(target);
        }
        if ops[tail].name == "JUMP_FORWARD" {
            let join = destination(ops, tail)?;
            if join <= target || join > end {
                return Err("unsupported conditional join".into());
            }
            let yes = self.fragment(ops, start, tail, level)?;
            let no = self.fragment(ops, target, join, level)?;
            if yes.stack.is_empty() && no.stack.is_empty() {
                self.block(&format!("if {test}:"), &yes.suite()?)?;
                self.block("else:", &no.suite()?)?;
            } else {
                self.push(format!(
                    "({} if {test} else {})",
                    yes.expression()?,
                    no.expression()?
                ))?;
            }
            return Ok(join);
        }
        let suite = self.fragment(ops, start, target, level)?.suite()?;
        if matches!(ops[tail].name, "RETURN_CONST" | "RETURN_VALUE")
            && end > target
            && matches!(ops[end - 1].name, "RETURN_CONST" | "RETURN_VALUE")
        {
            let other = self.fragment(ops, target, end, level)?.suite()?;
            let split_return = |s: &str| -> Option<(String, String)> {
                let (before, last) = s
                    .trim_end_matches('\n')
                    .rsplit_once('\n')
                    .unwrap_or(("", s.trim_end_matches('\n')));
                Some((before.to_owned(), last.strip_prefix("return ")?.to_owned()))
            };
            if let (Some((left, a)), Some((right, b))) =
                (split_return(&suite), split_return(&other))
            {
                if left.is_empty() && right.is_empty() {
                    self.line(&format!("return {a} if {test} else {b}"))?;
                    return Ok(end);
                }
                if a == b {
                    self.block(&format!("if {test}:"), &left)?;
                    if !right.is_empty() {
                        self.block("else:", &right)?;
                    }
                    self.line(&format!("return {a}"))?;
                    return Ok(end);
                }
            }
            self.block(&format!("if {test}:"), &suite)?;
            self.block("else:", &other)?;
            return Ok(end);
        }
        self.block(&format!("if {test}:"), &suite)?;
        Ok(target)
    }
}
