use super::regions::Fragment;
use super::{Body, Instruction};

pub(super) struct Predicate {
    pub(super) header: Option<usize>,
    pub(super) value: String,
    pub(super) fallthrough_true: bool,
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
    /// CPython may give each short-circuit edge a distinct copy of the same
    /// implicit function exit. Only a suffix of unprotected, empty-stack
    /// return blocks is interchangeable with falling off the end of the def.
    /// A generator can have its implicit `StopIteration` handler on those exits.
    pub(super) fn index_terminal_returns(
        &mut self,
        ops: &[Instruction],
        stop_handler: Option<usize>,
    ) {
        let mut start = ops.len();
        while start > 0 {
            let block = self.flow.block(start - 1);
            if block.height != Some(0)
                || self
                    .flow
                    .handler(block.ops.start)
                    .is_some_and(|h| !(Some(h.target) == stop_handler && h.depth == 0 && h.lasti))
                || !self.none_return(&ops[block.ops.clone()])
            {
                break;
            }
            start = block.ops.start;
        }
        self.terminal_returns = start..ops.len();
    }

    pub(super) fn none_return(&self, ops: &[Instruction]) -> bool {
        let load = match ops {
            [ret] if ret.name == "RETURN_CONST" => ret,
            // On targets with RETURN_CONST, a separate load and return
            // can be the None arm of a conditional expression.
            [load, ret]
                if ret.name == "RETURN_VALUE" && self.m.version >= super::PythonVersion::Py314 =>
            {
                load
            }
            _ => return false,
        };
        match load.name {
            "LOAD_CONST" | "RETURN_CONST" => self
                .item(self.code.constants, load.arg)
                .is_ok_and(|id| matches!(self.m.objects[id], super::Object::None)),
            "LOAD_COMMON_CONSTANT" => load.arg == super::CommonConstant::NONE.argument(),
            _ => false,
        }
    }

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
        header: Option<usize>,
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
        // For either identity branch, use the test that is true on fallthrough.
        let condition = |i: usize, value: String| match ops[i].name {
            "POP_JUMP_IF_NONE" => format!("({value}) is not None"),
            "POP_JUMP_IF_NOT_NONE" => format!("({value}) is None"),
            _ => value,
        };
        let mut value = condition(first, value);
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
                || !matches!(
                    ops[next].name,
                    "POP_JUMP_IF_FALSE"
                        | "POP_JUMP_IF_TRUE"
                        | "POP_JUMP_IF_NONE"
                        | "POP_JUMP_IF_NOT_NONE"
                )
            {
                break;
            }
            let (b_yes, b_no) = arms(next, self.flow.target(next)?);
            let same_exit = |a, b| {
                a == b
                    || (end == self.terminal_returns.end
                        && self.terminal_returns.contains(&a)
                        && self.terminal_returns.contains(&b))
            };
            let (operator, invert) = if yes == start && same_exit(no, b_no) {
                ("and", false)
            } else if yes == start && same_exit(no, b_yes) {
                ("and", true)
            } else if no == start && same_exit(yes, b_yes) {
                ("or", false)
            } else if no == start && same_exit(yes, b_no) {
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
            let rhs = condition(next, rhs);
            let rhs = if invert { format!("not ({rhs})") } else { rhs };
            self.charge(value.len() + rhs.len() + 8)?;
            value = format!("({value} {operator} {rhs})");
            (yes, no) = if invert { (b_no, b_yes) } else { (b_yes, b_no) };
            branch = next;
        }
        let start = fall(branch);
        Ok(Predicate {
            header,
            value,
            branch,
            suite: start,
            target: if yes == start { no } else { yes },
            fallthrough_true: yes == start,
        })
    }

    // A shared return can be copied to both arms. Distinct source lines
    // distinguish an authored early return, unless the compiler assigned an
    // implicit None return the line of the preceding statement. The caller
    // requires that preceding statement to belong to the early arm.
    fn distinct_constant_returns(&self, ops: &[Instruction], left: usize, right: usize) -> bool {
        let Some((a, av)) = super::regions::constant_return(ops, left) else {
            return false;
        };
        let Some((b, bv)) = super::regions::constant_return(ops, right) else {
            return false;
        };
        if av != bv || a == 0 {
            return false;
        }
        self.instruction_lines(ops).is_ok_and(|lines| {
            matches!((lines[a - 1], lines[a], lines[b]),
                (Some(prev), Some(early), Some(late)) if prev < early && early < late)
        })
    }

    pub(super) fn conditional(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
        level: usize,
        header: Option<usize>,
    ) -> Result<usize, String> {
        let value = self.expr()?;
        let mut predicate = self.predicate(ops, i, value, end, level, header)?;
        if let Some(latch) = self.enclosing_continue_latch(ops, &predicate, end) {
            predicate.suite = predicate.target;
            predicate.target = end;
            predicate.fallthrough_true = !predicate.fallthrough_true;
            return self.while_loop(ops, &predicate, &latch, end, level);
        }
        let fallthrough_end = predicate.target;
        self.conditional_suites(ops, &predicate, fallthrough_end, end, level)
    }

    pub(super) fn conditional_suites(
        &mut self,
        ops: &[Instruction],
        predicate: &Predicate,
        fallthrough_end: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let (i, start, target) = (predicate.branch, predicate.suite, predicate.target);
        if target <= i || target > end || fallthrough_end < start || fallthrough_end > target {
            return Err("unsupported conditional destination".into());
        }
        let tail = fallthrough_end - 1;
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
            return self.finally_break(ops, predicate, handler, level, end);
        }
        let test = predicate.test(true);
        if self.loop_transfer(ops, tail).is_some() {
            let suite = self.fragment(ops, start, fallthrough_end, level)?.suite()?;
            self.block(&format!("if {test}:"), &suite)?;
            return Ok(target);
        }
        if let Some(latch) = self.while_latch(ops, predicate, end) {
            return self.while_loop(ops, predicate, &latch, end, level);
        }
        if self.flow.join(i) == Some(target) {
            // Both paths reach the taken target. Only the fallthrough path
            // contributes a suite, even if it ends in a jump to the join.
            let stop = if self.flow.jump(tail) == Some(target) && ops[tail].name == "JUMP_FORWARD" {
                tail
            } else {
                fallthrough_end
            };
            let suite = self.fragment(ops, start, stop, level)?.suite()?;
            self.block(&format!("if {test}:"), &suite)?;
            return Ok(target);
        }
        if ops[tail].name == "JUMP_FORWARD" {
            // With a returning or looping arm there may be no common forward
            // postdominator. The other arm still jumps over its alternate.
            let destination = self.flow.target(tail)?;
            // CPython can send both arms of a nested conditional directly
            // to the enclosing join. The caller owns the jump at `end`.
            let join = if destination > end
                && ops.get(end).is_some_and(|op| op.name == "JUMP_FORWARD")
                && self.flow.jump(end) == Some(destination)
            {
                end
            } else {
                destination
            };
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
        // An enclosing guard can trim later copies of the very same exit.
        // Its nested suite still reaches the end of the function on fallthrough.
        if self.loop_edges.is_none()
            && (self.terminal_returns.start..=self.terminal_returns.end).contains(&end)
        {
            // A guard can return immediately, leaving the rest of the function
            // on its other edge. Keep it separate from any branches in the tail.
            if end == self.terminal_returns.end && self.none_return(&ops[start..fallthrough_end])
                && !self.terminal_returns.contains(&target)
                // A conditional expression can return an owned alternate
                // local where a separate return statement borrows it.
                && !(target + 2 == end && ops[target].name == "LOAD_FAST"
                    && self.m.version >= super::PythonVersion::Py314)
            {
                self.block(&format!("if {test}:"), "return")?;
                return Ok(target);
            }
            // At function exit CPython may duplicate the implicit None return
            // into both arms. Hoisting an explicit return creates a shared
            // successor when the input has two distinct terminal blocks.
            if self.terminal_returns.contains(&target) {
                let width = self.flow.block(target).ops.len();
                let copy = fallthrough_end.checked_sub(width).filter(|&n| {
                    n > start
                        && self.none_return(&ops[n..fallthrough_end])
                        && self.flow.block(n).ops.start < n
                });
                if copy.is_some() || self.terminal_returns.len() > width {
                    let suite = self
                        .fragment(ops, start, copy.unwrap_or(fallthrough_end), level)?
                        .suite()?;
                    self.block(&format!("if {test}:"), &suite)?;
                    return Ok(end);
                }
            }
        }
        let suite = self.fragment(ops, start, fallthrough_end, level)?.suite()?;
        if matches!(ops[tail].name, "RETURN_CONST" | "RETURN_VALUE")
            && end > target
            && (matches!(ops[end - 1].name, "RETURN_CONST" | "RETURN_VALUE")
                // A module or class return has no source statement. Its
                // other arm must remain under an else or the raise runs on
                // import even when the first arm completed successfully.
                || ((self.depth == 0 || self.class)
                    && matches!(ops[end - 1].name, "RAISE_VARARGS" | "RERAISE")))
            // A physical return at the end of the suite can belong to a
            // nested guard. Its other edge still enters the shared tail.
            && !self.flow.incoming(target).any(|from| (start..fallthrough_end).contains(&from))
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
                    // A conditional expression loads its constant arms before
                    // RETURN_VALUE. RETURN_CONST on 3.12/3.13 instead belongs
                    // to a separate return, as does a borrowed alternate local
                    // on the newer targets.
                    if ops[tail].name == "RETURN_CONST"
                        || ops[end - 1].name == "RETURN_CONST"
                        || (target + 2 == end && ops[target].name == "LOAD_FAST_BORROW")
                    {
                        self.block(&format!("if {test}:"), &suite)?;
                        self.line(&format!("return {b}"))?;
                        return Ok(end);
                    }
                    // An enclosing return can reuse this expression as its true
                    // arm. Python otherwise associates the outer test to the right.
                    self.line(&format!("return ({a} if {test} else {b})"))?;
                    return Ok(end);
                }
                if a == b {
                    // An early constant return can guard a loop with a
                    // different return inside it, followed by the same
                    // constant at exhaustion. Keep the guard's own exit.
                    if (!left.is_empty()
                        && self.distinct_constant_returns(ops, fallthrough_end, end))
                        || (left.is_empty()
                            && super::regions::constant_return(ops, fallthrough_end)
                                .is_some_and(|(at, _)| at == start)
                            && self.flow.blocks_in(target..end).any(|block| {
                                block.ops.end < end
                                    && matches!(
                                        ops[block.ops.end - 1].name,
                                        "RETURN_CONST" | "RETURN_VALUE"
                                    )
                            }))
                    {
                        self.block(&format!("if {test}:"), &suite)?;
                        // A jump out of an else suite can survive as a tracing
                        // NOP before the copied return. Keep the alternate
                        // inside the conditional and its continuation outside.
                        if !right.is_empty()
                            && super::regions::constant_return(ops, end)
                                .is_some_and(|(at, _)| at > target && ops[at - 1].name == "NOP")
                        {
                            self.block("else:", &right)?;
                            self.line(&format!("return {b}"))?;
                        } else {
                            self.line(other.trim_end_matches('\n'))?;
                        }
                        return Ok(end);
                    }
                    self.block(&format!("if {test}:"), &left)?;
                    if !right.is_empty() {
                        self.block("else:", &right)?;
                    }
                    self.line(&format!("return {a}"))?;
                    return Ok(end);
                }
                // A bare constant return exits the function. The remaining
                // statements can stay at their original indentation, including
                // another guard followed by its own return.
                if start == tail && ops[tail].name == "RETURN_CONST" {
                    self.block(&format!("if {test}:"), &suite)?;
                    self.line(other.trim_end_matches('\n'))?;
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
