use super::regions::Fragment;
use super::{Body, Instruction};

pub(super) struct Predicate {
    value: String,
    fallthrough_true: bool,
    pub(super) branch: usize,
    pub(super) suite: usize,
    pub(super) target: usize,
}

impl Predicate {
    pub(super) fn test(&self, fallthrough: bool) -> String {
        if self.fallthrough_true == fallthrough {
            self.value.clone()
        } else {
            format!("not ({})", self.value)
        }
    }
}

impl Body<'_, '_, '_> {
    /// Fold consecutive predicate blocks that share a true or false successor.
    /// A and B only compose when short-circuiting A reaches the same successor
    /// as one outcome of B. Keep the physical fallthrough for suite recovery.
    fn predicate(
        &mut self,
        ops: &[Instruction],
        first: usize,
        value: String,
        end: usize,
        level: usize,
    ) -> Result<Predicate, String> {
        let fall =
            |i: usize| i + 1 + usize::from(ops.get(i + 1).is_some_and(|op| op.name == "NOT_TAKEN"));
        let arms = |i: usize, jump: usize| {
            if ops[i].name == "POP_JUMP_IF_TRUE" {
                (jump, fall(i))
            } else {
                (fall(i), jump)
            }
        };
        let value = match ops[first].name {
            "POP_JUMP_IF_NONE" => format!("({value}) is not None"),
            "POP_JUMP_IF_NOT_NONE" => format!("({value}) is None"),
            _ => value,
        };
        let mut value = value;
        let mut branch = first;
        let (mut yes, mut no) = arms(branch, self.flow.target(branch)?);
        for _ in 0..32 {
            let start = fall(branch);
            if start >= end || yes == no || !self.flow.entered_from(start, first..start) {
                break;
            }
            let next = self.flow.block(start).ops.end - 1;
            if next >= end
                || next == start
                || !matches!(ops[next].name, "POP_JUMP_IF_FALSE" | "POP_JUMP_IF_TRUE")
            {
                break;
            }
            let (b_yes, b_no) = arms(next, self.flow.target(next)?);
            let (operator, invert) = if yes == start && no == b_no {
                ("and", false)
            } else if yes == start && no == b_yes {
                ("and", true)
            } else if no == start && yes == b_yes {
                ("or", false)
            } else if no == start && yes == b_no {
                ("or", true)
            } else {
                break;
            };
            // A suite containing statements can also end in a conditional.
            // Only combine blocks that produce a single predicate value.
            let stop = next - usize::from(ops[next - 1].name == "TO_BOOL");
            let Ok(rhs) = self
                .fragment(ops, start, stop, level)
                .and_then(Fragment::expression)
            else {
                break;
            };
            let rhs = if invert { format!("not ({rhs})") } else { rhs };
            self.charge(value.len() + rhs.len() + 8)?;
            value = format!("({value} {operator} {rhs})");
            (yes, no) = if invert { (b_no, b_yes) } else { (b_yes, b_no) };
            branch = next;
        }
        let start = fall(branch);
        Ok(Predicate {
            value,
            branch,
            suite: start,
            target: if yes == start { no } else { yes },
            fallthrough_true: yes == start,
        })
    }

    pub(super) fn conditional(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let value = self.expr()?;
        let predicate = self.predicate(ops, i, value, end, level)?;
        let (i, start, target) = (predicate.branch, predicate.suite, predicate.target);
        if target <= i || target > end {
            return Err("unsupported conditional destination".into());
        }
        let tail = target - 1;
        if start < tail
            && ops[tail].name == "RAISE_VARARGS"
            && ops[tail].arg == 1
            && (ops[start].name == "LOAD_ASSERTION_ERROR"
                || (ops[start].name == "LOAD_COMMON_CONSTANT"
                    && ops[start].arg
                        == serc_bytecode::target::operands315::CommonConstant::ASSERTIONERROR
                            .argument()))
        {
            let test = predicate.test(false);
            let msg = if start + 1 == tail {
                String::new()
            } else if ops[tail - 1].name == "CALL" {
                format!(
                    ", {}",
                    self.fragment(ops, start + 1, tail - 1, level)?
                        .expression()?
                )
            } else {
                return Err("unsupported assertion argument".into());
            };
            self.line(&format!("assert {test}{msg}"))?;
            return Ok(target);
        }
        if self.loop_edges.is_some()
            && let Some(handler) = self.flow.handler(i)
            && !handler.lasti
            && handler.depth > 0
            && ops
                .get(i + 1..=start)
                .is_some_and(|after| after.iter().any(|op| op.offset == handler.end))
            && target < end
        {
            return self.finally_break(ops, &predicate, handler, level, end);
        }
        let test = predicate.test(true);
        if self.loop_transfer(ops, tail).is_some() {
            let suite = self.fragment(ops, start, target, level)?.suite()?;
            self.block(&format!("if {test}:"), &suite)?;
            return Ok(target);
        }
        if ops[tail].name == "JUMP_BACKWARD" {
            return self.while_loop(ops, &predicate, &test, end, level);
        }
        if self.flow.join(i) == Some(target) {
            // Both paths reach the taken target. Only the fallthrough path
            // contributes a suite, even if it ends in a jump to the join.
            let stop = if self.flow.jump(tail) == Some(target) && ops[tail].name == "JUMP_FORWARD" {
                tail
            } else {
                target
            };
            let suite = self.fragment(ops, start, stop, level)?.suite()?;
            self.block(&format!("if {test}:"), &suite)?;
            return Ok(target);
        }
        if ops[tail].name == "JUMP_FORWARD" {
            // With a returning or looping arm there may be no common forward
            // postdominator. The other arm still jumps over its alternate.
            let join = self.flow.target(tail)?;
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
