use super::regions::Fragment;
use super::{Body, Instruction};

pub(super) struct Predicate {
    pub(super) header: Option<usize>,
    pub(super) value: String,
    pub(super) fallthrough_true: bool,
    pub(super) branch: usize,
    pub(super) suite: usize,
    pub(super) target: usize,
    pub(super) comparison_cleanup: Option<usize>,
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

fn fallthrough(ops: &[Instruction], branch: usize) -> usize {
    branch + 1 + usize::from(ops.get(branch + 1).is_some_and(|op| op.name == "NOT_TAKEN"))
}

// On identity branches this is the condition that is true on fallthrough.
fn condition(name: &str, value: String) -> String {
    match name {
        "POP_JUMP_IF_NONE" => format!("({value}) is not None"),
        "POP_JUMP_IF_NOT_NONE" => format!("({value}) is None"),
        _ => value,
    }
}

fn negate_if(value: String, invert: bool) -> String {
    if invert {
        format!("not ({value})")
    } else {
        value
    }
}

/// A sequence of tests with two remaining destinations.
struct Term {
    start: usize,
    branch: usize,
    suite: usize,
    comparison_cleanup: Option<usize>,
    yes: usize,
    no: usize,
    value: String,
}

impl Term {
    fn comparison(start: usize, chain: Predicate) -> Self {
        let (yes, no) = if chain.fallthrough_true {
            (chain.suite, chain.target)
        } else {
            (chain.target, chain.suite)
        };
        Self {
            start,
            branch: chain.branch,
            suite: chain.suite,
            comparison_cleanup: chain.comparison_cleanup,
            yes,
            no,
            value: chain.value,
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
        self.none_constant(*load)
    }

    pub(super) fn none_constant(&self, op: Instruction) -> bool {
        match op.name {
            "LOAD_CONST" | "RETURN_CONST" => self
                .item(self.code.constants, op.arg)
                .is_ok_and(|id| matches!(self.m.objects[id], super::Object::None)),
            "LOAD_COMMON_CONSTANT" => op.arg == super::CommonConstant::NONE.argument(),
            _ => false,
        }
    }

    fn term(
        &self,
        ops: &[Instruction],
        start: usize,
        branch: usize,
        value: String,
    ) -> Result<Term, String> {
        let name = ops[branch].name;
        let jump = self.flow.target(branch)?;
        let jump = self
            .clause_exits
            .map_or(jump, |copies| copies.destination(jump));
        let fall = fallthrough(ops, branch);
        let (yes, no) = if name == "POP_JUMP_IF_TRUE" {
            (jump, fall)
        } else {
            (fall, jump)
        };
        Ok(Term {
            start,
            branch,
            suite: fall,
            comparison_cleanup: None,
            value: condition(name, value),
            yes,
            no,
        })
    }

    /// Reduce adjacent tests once they share an exit. For `a or b and c`
    /// the right pair reduces first. An unfinished term belongs to a later
    /// suite and is left for the normal statement traversal.
    fn predicate(
        &mut self,
        ops: &[Instruction],
        initial: Term,
        end: usize,
        level: usize,
        header: Option<usize>,
    ) -> Result<Predicate, String> {
        let first = initial.start;
        let mut terms = vec![initial];
        for _ in 0..32 {
            let last = terms.last().unwrap();
            let start = last.suite;
            if start >= end || last.yes == last.no || !self.flow.entered_from(start, first..start) {
                break;
            }
            let mut rhs = if let Some(chain) = self.comparison_operand(ops, start, end, level) {
                Term::comparison(start, chain)
            } else if let Some((branch, value)) = self.predicate_operand(ops, start, end, level)? {
                self.term(ops, start, branch, value)?
            } else {
                break;
            };
            let branch = rhs.branch;
            while let Some(lhs) = terms.last()
                && self.flow.entered_from(rhs.start, lhs.start..rhs.start)
                && let Some((op, invert)) = self.shared_exit(lhs, &rhs, end)
            {
                self.charge(lhs.value.len() + rhs.value.len() + 16)?;
                let lhs = terms.pop().unwrap();
                if invert
                    && rhs.start == start
                    && matches!(ops[branch - 1].name, "CONTAINS_OP" | "IS_OP")
                    && !matches!(
                        ops[lhs.branch.saturating_sub(1)].name,
                        "CONTAINS_OP" | "IS_OP"
                    )
                {
                    // Negating a membership or identity test flips its opcode
                    // argument. Complement the left term to keep the original
                    // comparison and the sense of its branch.
                    let opposite = if op == "and" { "or" } else { "and" };
                    rhs.value = format!("(not ({}) {opposite} {})", lhs.value, rhs.value);
                } else {
                    let right = negate_if(rhs.value, invert);
                    rhs.value = format!("({} {op} {right})", lhs.value);
                    if invert {
                        std::mem::swap(&mut rhs.yes, &mut rhs.no);
                    }
                }
                rhs.start = lhs.start;
            }
            terms.push(rhs);
        }
        let term = terms.into_iter().next().unwrap();
        let suite = term.suite;
        let fallthrough_true = term.yes == suite;
        Ok(Predicate {
            header,
            value: term.value,
            branch: term.branch,
            suite,
            fallthrough_true,
            target: if fallthrough_true { term.no } else { term.yes },
            comparison_cleanup: term.comparison_cleanup,
        })
    }

    pub(super) fn extend_comparison(
        &mut self,
        ops: &[Instruction],
        start: usize,
        chain: Predicate,
        level: usize,
    ) -> Result<Predicate, String> {
        let header = chain.header;
        self.predicate(
            ops,
            Term::comparison(start, chain),
            ops.len(),
            level,
            header,
        )
    }

    fn shared_exit(&self, lhs: &Term, rhs: &Term, end: usize) -> Option<(&'static str, bool)> {
        let same = |a, b| {
            a == b
                || (end == self.terminal_returns.end
                    && self.terminal_returns.contains(&a)
                    && self.terminal_returns.contains(&b))
        };
        if lhs.yes == rhs.start && same(lhs.no, rhs.no) {
            Some(("and", false))
        } else if lhs.yes == rhs.start && same(lhs.no, rhs.yes) {
            Some(("and", true))
        } else if lhs.no == rhs.start && same(lhs.yes, rhs.yes) {
            Some(("or", false))
        } else if lhs.no == rhs.start && same(lhs.yes, rhs.no) {
            Some(("or", true))
        } else {
            None
        }
    }

    fn predicate_operand(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<Option<(usize, String)>, String> {
        let mut branch = self.flow.block(start).ops.end - 1;
        if branch <= start || branch >= end || !ops[branch].name.starts_with("POP_JUMP_IF_") {
            return Ok(None);
        }
        // A guarded all()/any() has an inlined loop and a fallback call. The
        // predicate consumes their shared result. The expression lifter checks
        // both paths and the identity guard before returning that operand.
        if branch >= start + 3
            && super::builtin_calls::truth_guard(&ops[branch - 3..=branch]).is_some()
            && let Some(fallback) = self.flow.jump(branch)
            && (branch + 1..end).contains(&fallback)
            && let Some(join) = self.flow.fallthrough(self.flow.block(fallback))
            && (fallback + 1..end).contains(&join)
        {
            branch = self.flow.block(join).ops.end - 1;
            if branch >= end || !ops[branch].name.starts_with("POP_JUMP_IF_") {
                return Ok(None);
            }
            self.charge(branch - start)?;
            if self.flow.blocks_in(start..branch + 1).any(|block| {
                let i = block.ops.end - 1;
                (block.ops.start > start
                    && !self.flow.entered_from(block.ops.start, start..branch + 1))
                    || (i != branch
                        && self
                            .flow
                            .jump(i)
                            .is_some_and(|to| !(start..=branch).contains(&to)))
            }) {
                return Ok(None);
            }
        }
        let stop = branch - usize::from(ops[branch - 1].name == "TO_BOOL");
        // A statement suite may also end in a test. It is an operand only
        // when it leaves a single expression and emits no statements.
        Ok(self
            .fragment(ops, start, stop, level)
            .and_then(Fragment::expression)
            .ok()
            .map(|value| (branch, value)))
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
        self.instruction_lines.as_deref().is_ok_and(|lines| {
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
        let initial = self.term(ops, i, i, value)?;
        let mut predicate = self.predicate(ops, initial, end, level, header)?;
        if let Some(cleanup) = predicate.comparison_cleanup
            && predicate.suite < cleanup
        {
            return self.comparison_suites(&ops[..end], &predicate, cleanup, level);
        }
        if self.conditional_break(ops, predicate.target, end) {
            self.block(&format!("if {}:", predicate.test(false)), "break")?;
            return Ok(predicate.suite);
        }
        predicate.target = self.local_join(ops, predicate.target, end);
        if let Some(latch) = self.enclosing_continue_latch(ops, &predicate, end) {
            predicate.suite = predicate.target;
            predicate.target = end;
            predicate.fallthrough_true = !predicate.fallthrough_true;
            return self.while_loop(ops, &predicate, &latch, end, level);
        }
        if self.guarded_iteration_tail(ops, &predicate, end) {
            let suite = self.fragment(ops, predicate.target, end, level)?.suite()?;
            self.block(&format!("if {}:", predicate.test(false)), &suite)?;
            return Ok(end);
        }
        let fallthrough_end = predicate.target;
        self.conditional_suites(ops, &predicate, fallthrough_end, end, level)
    }

    // CPython can thread a nested arm directly to the enclosing join.
    // Falling off this fragment runs the caller's jump to the same place.
    fn local_join(&self, ops: &[Instruction], destination: usize, end: usize) -> usize {
        let destination = self
            .clause_exits
            .map_or(destination, |copies| copies.destination(destination));
        if self.arm_exit == Some((end, destination))
            || (destination > end
                && ops.get(end).is_some_and(|op| op.name == "JUMP_FORWARD")
                && self.flow.jump(end) == Some(destination))
        {
            end
        } else {
            destination
        }
    }

    /// If one arm ends in a raise or return, its nested guards can skip the
    /// other arm without leaving a `JUMP_FORWARD` at the end of the first.
    fn threaded_arm_join(&self, first: std::ops::Range<usize>, limit: usize) -> Option<usize> {
        let mut join = None;
        for block in self.flow.blocks_in(first.clone()) {
            if self.flow.fallthrough(block) == Some(first.end) {
                return None;
            }
            if let Some(target) = self.flow.jump(block.ops.end - 1)
                && !first.contains(&target)
            {
                if target <= first.end || target > limit || join.is_some_and(|old| old != target) {
                    return None;
                }
                join = Some(target);
            }
        }
        join
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
            // At the end of a function each successful short-circuit edge may
            // have its own implicit return. An explicit return merges them.
            if end == self.terminal_returns.end
                && self.terminal_returns.contains(&target)
                && self.terminal_returns.len() > self.flow.block(target).ops.len()
            {
                return Ok(end);
            }
            return Ok(target);
        }
        if self.loop_edges.is_some()
            && let Some(handler) = self.flow.handler(i)
            && self
                .active_try
                .is_none_or(|active| active.target != handler.target)
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
        // A binary expression's left operand is already on the stack. When
        // CPython copies the final operation and return into both arms, join
        // the right operands so the left operand still runs before the test.
        if !self.stack.is_empty()
            && fallthrough_end >= start + 3
            && end >= target + 3
            && ops[end - 2].name == "BINARY_OP"
            && ops[end - 1].name == "RETURN_VALUE"
            && super::comparisons::same_instructions(
                &ops[fallthrough_end - 2..fallthrough_end],
                &ops[end - 2..end],
            )
        {
            let yes = self
                .fragment(ops, start, fallthrough_end - 2, level)?
                .expression()?;
            let no = self.fragment(ops, target, end - 2, level)?.expression()?;
            self.push(format!("({yes} if {test} else {no})"))?;
            return Ok(end - 2);
        }
        if fallthrough_end == target
            && target < end
            && self.stack.is_empty()
            && let Some(copy) = self
                .clause_exits
                .and_then(|exits| exits.before_arm(ops, &self.flow, target, end))
                .or_else(|| self.handler_rejoin?.restore_before(ops, &self.flow, target))
            && copy >= start
            && self.flow.blocks_covering(start..copy).all(|b| {
                [self.flow.jump(b.ops.end - 1), self.flow.fallthrough(b)]
                    .into_iter()
                    .flatten()
                    .all(|to| {
                        (start..=copy).contains(&to)
                            || self.handler_rejoin.is_some_and(|rejoin| {
                                rejoin.restore_before(ops, &self.flow, b.ops.end).is_some()
                            })
                    })
            })
        {
            let left = self.fragment(ops, start, copy, level)?.suite()?;
            let right = self.fragment(ops, target, end, level)?.suite()?;
            self.block(&format!("if {test}:"), &left)?;
            self.block("else:", &right)?;
            return Ok(end);
        }
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
            let join = self.local_join(ops, self.flow.target(tail)?, end);
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
        if fallthrough_end == target
            && target < end
            && self.stack.is_empty()
            && (matches!(
                ops[tail].name,
                "RAISE_VARARGS" | "RETURN_VALUE" | "RETURN_CONST"
            ) || self.implicit_reraise(ops, tail))
        {
            self.charge(fallthrough_end - start)?;
            if let Some(join) = self.threaded_arm_join(start..fallthrough_end, end) {
                let enclosing = self.arm_exit.replace((fallthrough_end, join));
                let suite = self.fragment(ops, start, fallthrough_end, level);
                self.arm_exit = enclosing;
                let suite = suite?.suite()?;
                let other = self.fragment(ops, target, join, level)?.suite()?;
                self.block(&format!("if {test}:"), &suite)?;
                self.block("else:", &other)?;
                return Ok(join);
            }
        }
        // The first arm of a terminal if/else can have its own copies of the
        // implicit return. A bare None exit carrying its incoming guard's
        // line belongs to that guard. Index it as the end of this arm so the
        // nested test does not emit a return that merges the original exits.
        if self.stack.is_empty()
            && self.loop_edges.is_none()
            && end == self.terminal_returns.end
            && fallthrough_end == target
            && start < tail
            && target < end
            && self.flow.handler(tail).is_none()
            && let last = self.flow.block(tail)
            && start < last.ops.start
            && self.none_return(&ops[last.ops.clone()])
            && let guard = self.flow.block(start).ops.end - 1
            && ops[guard].name.starts_with("POP_JUMP_IF_")
            && self.flow.jump(guard) == Some(last.ops.start)
            && self.instruction_lines.as_deref().is_ok_and(|lines| {
                lines[guard].is_some() && lines[guard] == lines[last.ops.start]
            })
            && self.flow.blocks_in(start..target).all(|block| {
                [
                    self.flow.jump(block.ops.end - 1),
                    self.flow.fallthrough(block),
                ]
                .into_iter()
                .flatten()
                .all(|to| (start..target).contains(&to))
            })
        {
            let outer = self.terminal_returns.clone();
            self.index_terminal_returns(&ops[..target], None);
            let left = self.fragment(ops, start, target, level);
            self.terminal_returns = outer;
            let left = left?.suite()?;
            let right = self.fragment(ops, target, end, level)?.suite()?;
            self.block(&format!("if {test}:"), &left)?;
            self.block("else:", &right)?;
            return Ok(end);
        }
        let suite = self.fragment(ops, start, fallthrough_end, level)?.suite()?;
        if self.implicit_reraise(ops, tail)
            && target < end
            && !self
                .flow
                .incoming(target)
                .any(|from| (start..fallthrough_end).contains(&from))
        {
            self.block(&format!("if {test}:"), &suite)?;
            let other = self.fragment(ops, target, end, level)?.suite()?;
            if !other.is_empty() {
                self.block("else:", &other)?;
            }
            return Ok(end);
        }
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
                    if predicate.comparison_cleanup.is_some_and(|at| {
                        // Earlier failed links return directly. A conditional
                        // expression would jump to a shared result instead.
                        super::regions::constant_return(ops, self.flow.block(at).ops.end)
                            .is_some_and(|(load, _)| load == at + 1)
                    }) || ops[tail].name == "RETURN_CONST"
                        || ops[end - 1].name == "RETURN_CONST"
                        || (target + 2 == end && ops[target].name == "LOAD_FAST_BORROW")
                        // Two unprotected iterator cleanups are separate
                        // returns. A conditional expression shares its cleanup.
                        || self.active_try.is_some_and(|h| {
                            [tail, end - 1].into_iter().all(|ret|
                                super::protected_regions::iterator_return_block(
                                    ops, &self.flow, self.flow.block(ret).ops.clone(), h.depth))
                        })
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
                    // A nested guard can target a block containing only its
                    // implicit None return. An explicit return after the outer
                    // else would merge that exit with its sibling.
                    let implicit = (end == ops.len() || end == self.terminal_returns.end)
                        && level == 0
                        && a == "None"
                        && self.none_return(&ops[self.flow.block(tail).ops.clone()])
                        && super::regions::constant_return(ops, end).is_some_and(|(at, _)| {
                            at > target
                                && self.instruction_lines.as_deref().is_ok_and(|lines| {
                                    lines[at].is_some() && lines[at] == lines[at - 1]
                                })
                        });
                    if !implicit {
                        self.line(&format!("return {a}"))?;
                    }
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
