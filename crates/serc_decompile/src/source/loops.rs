use std::fmt::Write as _;

use super::conditionals::Predicate;
use super::regions::constant_return;
use super::{Body, Instruction, Value};

pub(super) struct WhileLatch {
    header: usize,
    body_end: usize,
    implicit_return: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Loop {
    header: usize,
    body_end: usize,
    after: Option<usize>,
    // Index of the return operand CPython copied into a break arm.
    copied_return: Option<usize>,
}

fn local_return(ops: &[Instruction], end: usize) -> Option<usize> {
    let [.., load, ret] = ops.get(..end)? else {
        return None;
    };
    (matches!(load.name, "LOAD_FAST" | "LOAD_FAST_BORROW") && ret.name == "RETURN_VALUE")
        .then_some(end - 2)
}

impl Body<'_, '_, '_> {
    fn copied_return(
        &self,
        ops: &[Instruction],
        header: usize,
        end: usize,
        expected: usize,
    ) -> Option<usize> {
        let at = if let Some((at, _)) = constant_return(ops, end) {
            at
        } else {
            let at = local_return(ops, end)?;
            let block = self.flow.block(at);
            let slots = self.iterator_slots(ops, header)?;
            // The entire arm discards exactly this iterator before reading
            // the same local as the continuation. A return from a nested
            // iterator would have additional slots to unwind.
            if block.ops.end != end
                || at - block.ops.start != slots
                || block.height != Some(slots)
                || self.flow.handler(at).is_some()
                || ops[block.ops.start..at]
                    .iter()
                    .any(|op| op.name != "POP_TOP")
            {
                return None;
            }
            at
        };
        (ops[at].opcode == ops[expected].opcode && ops[at].arg == ops[expected].arg).then_some(at)
    }

    /// A straight-line `while True` body can end in a conditional exit
    /// followed by an unconditional backedge. Older targets jump over the
    /// backedge on the exit path and leave the leading NOP outside the loop.
    pub(super) fn bottom_tested_loop(
        &mut self,
        ops: &[Instruction],
        nop: usize,
        end: usize,
        level: usize,
    ) -> Result<Option<usize>, String> {
        if nop + 1 >= end {
            return Ok(None);
        }
        let block = self.flow.block(nop + 1);
        let header = block.ops.start;
        // A protected predicate can end just before its conditional jump.
        // The table boundary splits the block without changing normal flow.
        let branch = if block.ops.end < end
            && ops[block.ops.end].name.starts_with("POP_JUMP_IF_")
            && self.flow.fallthrough(block) == Some(block.ops.end)
            && self.flow.entered_from(block.ops.end, block.ops.clone())
        {
            block.ops.end
        } else {
            block.ops.end - 1
        };
        if header < nop
            || (header > nop && self.handlers.contains_key(&header))
            || branch + 1 >= end
        {
            return Ok(None);
        }
        let fall = branch + 1 + usize::from(ops[branch + 1].name == "NOT_TAKEN");
        if fall >= end
            || self.flow.jump(branch) != Some(fall + 1)
            || !self.flow.entered_from(branch + 1, branch..branch + 1)
        {
            return Ok(None);
        }
        let (jump, exits_on_jump) = match ops[fall].name {
            "JUMP_BACKWARD" => (fall, true),
            "JUMP_FORWARD"
                if fall + 1 < end
                    && self.flow.jump(fall) == Some(fall + 2)
                    && self.flow.entered_from(fall + 1, branch..branch + 1) =>
            {
                (fall + 1, false)
            }
            _ => return Ok(None),
        };
        if ops[jump].name != "JUMP_BACKWARD" || self.flow.jump(jump) != Some(header) {
            return Ok(None);
        }
        let mut body = self.fragment(ops, nop + 1, branch, level)?;
        let value = body
            .stack
            .pop()
            .and_then(Value::expression_text)
            .ok_or("missing loop predicate")?;
        let test = match (ops[branch].name, exits_on_jump) {
            ("POP_JUMP_IF_TRUE", true) | ("POP_JUMP_IF_FALSE", false) => value,
            ("POP_JUMP_IF_FALSE", true) | ("POP_JUMP_IF_TRUE", false) => format!("not ({value})"),
            ("POP_JUMP_IF_NONE", true) | ("POP_JUMP_IF_NOT_NONE", false) => {
                format!("({value}) is None")
            }
            ("POP_JUMP_IF_NOT_NONE", true) | ("POP_JUMP_IF_NONE", false) => {
                format!("({value}) is not None")
            }
            _ => return Err("unsupported loop predicate".into()),
        };
        let mut suite = body.suite()?;
        writeln!(suite, "if {test}:\n    break").unwrap();
        self.block("while True:", &suite)?;
        Ok(Some(jump + 1))
    }

    pub(super) fn loop_transfer(&self, ops: &[Instruction], i: usize) -> Option<&'static str> {
        let active = self.loop_edges?;
        if !matches!(ops[i].name, "JUMP_BACKWARD" | "JUMP_FORWARD") {
            return None;
        }
        let dest = self.flow.jump(i)?;
        if dest == active.header {
            Some("continue")
        } else if Some(dest) == active.after {
            Some("break")
        } else {
            None
        }
    }

    /// At the end of an except suite's loop, a failed test can go straight
    /// back to `FOR_ITER` outside the exception table. An explicit continue
    /// inside the handler would protect that jump. Guard the remaining body
    /// instead so the iterator advances implicitly.
    pub(super) fn guarded_iteration_tail(
        &self,
        ops: &[Instruction],
        predicate: &Predicate,
        end: usize,
    ) -> bool {
        let Some(active) = self.loop_edges else {
            return false;
        };
        active.body_end == end
            && ops[active.header].name == "FOR_ITER"
            && self.flow.handler(active.header).is_some_and(|h| h.lasti)
            && predicate.suite + 1 == predicate.target
            && predicate.target < end
            && self.loop_transfer(ops, predicate.suite) == Some("continue")
            && self.flow.handler(predicate.suite).is_none()
            && self
                .flow
                .entered_from(predicate.target, predicate.branch..predicate.target)
    }

    /// Recover a constant-true loop whose paths return, raise, or come back
    /// to its header. Older targets leave the header NOP just before the
    /// ordinary backedge target, but an explicit continue can still visit it.
    pub(super) fn unconditional_loop(
        &mut self,
        ops: &[Instruction],
        nop: usize,
        end: usize,
        level: usize,
    ) -> Result<Option<usize>, String> {
        let header = nop + usize::from(self.m.version < super::PythonVersion::Py314);
        if header >= end || self.flow.block(header).ops.start != header {
            return Ok(None);
        }
        let Some(latch) = self.flow.incoming(header).rev().find(|&i| {
            (nop + 1..end).contains(&i)
                && ops[i].name == "JUMP_BACKWARD"
                && self.flow.jump(i) == Some(header)
        }) else {
            return Ok(None);
        };
        let after = latch + 1;
        let stop = super::suspension::stop_iteration_handler(ops);
        let mut return_arm = None;
        let mut exits_to_after = false;
        for block in self.flow.blocks_in(header..after) {
            let start = block.ops.start;
            let last = block.ops.end - 1;
            exits_to_after |= self.flow.jump(last) == Some(after);
            // A break followed by a return can be threaded into one arm,
            // leaving its tracing NOP before the copied load and return.
            if after == end
                && ops[start].name == "NOP"
                && block.height == self.flow.block(header).height
                && (local_return(ops, block.ops.end) == Some(start + 1)
                    || constant_return(ops, block.ops.end).is_some_and(|(at, _)| at == start + 1))
            {
                return_arm = Some(start + 1);
            }
            let protected = self.flow.handler(start).is_some_and(|h| {
                let generator_exit = Some(h.target) == stop && h.depth == 0 && h.lasti;
                !generator_exit && self.delegated_cleanup(ops, start).is_none()
            });
            if protected
                || (start != header
                    && !self.flow.entered_from(start, nop..after)
                    && !self.delegation_rejoins_loop(ops, start, nop..after))
                || self.flow.jump(last).is_some_and(|to| {
                    !((nop..after).contains(&to)
                        || (to == after
                            && (ops[last].name == "JUMP_FORWARD"
                                || ops[last].name.starts_with("POP_JUMP_IF_"))))
                })
            {
                return Ok(None);
            }
        }
        let copied_return = return_arm.filter(|_| !exits_to_after);
        let outer = self.loop_edges.replace(Loop {
            header: nop,
            body_end: latch,
            after: Some(after),
            copied_return,
        });
        let body = self.fragment(ops, nop + 1, latch, level);
        self.loop_edges = outer;
        self.block("while True:", &body?.suite()?)?;
        if let Some(at) = copied_return {
            self.region(ops, at, self.flow.block(at).ops.end, level)?;
        }
        Ok(Some(after))
    }

    /// The out-of-line `CLEANUP_THROW` rejoins the same `END_SEND` as the
    /// in-loop `SEND`. Only accept that edge from outside the loop.
    fn delegation_rejoins_loop(
        &self,
        ops: &[Instruction],
        start: usize,
        body: std::ops::Range<usize>,
    ) -> bool {
        if ops[start].name != "END_SEND" {
            return false;
        }
        let Some(send) = self
            .flow
            .incoming(start)
            .find(|&i| body.contains(&i) && ops[i].name == "SEND")
        else {
            return false;
        };
        let Some(cleanup) = self.delegated_cleanup(ops, send + 1) else {
            return false;
        };
        self.flow
            .incoming(start)
            .all(|i| body.contains(&i) || i == cleanup + 1)
    }

    pub(super) fn conditional_break(
        &self,
        ops: &[Instruction],
        destination: usize,
        end: usize,
    ) -> bool {
        self.loop_edges.is_some_and(|active| {
            ops[active.header].name == "NOP"
                && active.body_end == end
                && active.after == Some(destination)
        })
    }

    pub(super) fn folded_break(&self, ops: &[Instruction], i: usize, end: usize) -> bool {
        let Some(active) = self.loop_edges else {
            return false;
        };
        let Some(expected) = active.copied_return else {
            return false;
        };
        let stop = self.flow.block(i).ops.end;
        if ops[active.header].name == "NOP" {
            return i + 1 == expected && ops[i].name == "NOP" && stop <= end;
        }
        stop <= end
            && self
                .copied_return(ops, active.header, stop, expected)
                .is_some_and(|start| {
                    start > i
                    // A single iterator uses up to two slots, plus a tracing
                    // NOP. Leave any earlier user statements in the suite.
                    && start - i <= 3
                    && ops[i..start]
                        .iter()
                        .all(|op| matches!(op.name, "NOP" | "POP_ITER" | "POP_TOP"))
                })
    }

    fn iterator_slots(&self, ops: &[Instruction], header: usize) -> Option<usize> {
        let iter = header.checked_sub(1)?;
        if ops[header].name != "FOR_ITER" || ops[iter].name != "GET_ITER" {
            return None;
        }
        let (_, slots) = serc_bytecode::target::stack_counts_for(
            self.m.version,
            ops[iter].opcode,
            ops[iter].arg,
        );
        let depth = self.flow.for_iter_depth(header)?;
        // A loop in an except clause also carries the saved exception.
        (depth == slots
            || (depth == slots + 1
                && self
                    .flow
                    .handler(header)
                    .is_some_and(|h| h.lasti && h.depth == 1)))
        .then_some(slots)
    }

    /// A return inside a for suite discards the iterator that the source
    /// operand stack leaves implicit. A computed return value is moved
    /// underneath it first. Only consume slots owned by this iterator.
    pub(super) fn iterator_return(
        &self,
        ops: &[Instruction],
        i: usize,
        end: usize,
    ) -> Option<usize> {
        let slots = self.iterator_slots(ops, self.loop_edges?.header)?;
        let computed = match ops[i].name {
            "SWAP" if ops[i].arg as usize == slots + 1 && self.stack.len() == 1 => true,
            "POP_TOP" if self.stack.is_empty() => false,
            _ => return None,
        };
        let pop = i + usize::from(computed);
        let next = pop + slots;
        let stop = self.flow.block(i).ops.end;
        if next > stop || stop > end || ops[pop..next].iter().any(|op| op.name != "POP_TOP") {
            return None;
        }
        if !computed && next == stop {
            return self.handler_constant_return(ops, i, next, end);
        }
        let returns = if computed {
            next + 1 == stop && ops[next].name == "RETURN_VALUE"
        } else {
            constant_return(ops, stop).is_some_and(|(load, _)| load == next)
        };
        returns.then_some(next)
    }

    fn loop_exits(
        &self,
        ops: &[Instruction],
        header: usize,
        body: std::ops::Range<usize>,
        exhausted: usize,
        end: usize,
    ) -> Result<Loop, String> {
        let mut active = Loop {
            header,
            body_end: body.end,
            after: None,
            copied_return: None,
        };
        let continuation = (exhausted < end)
            .then(|| self.flow.block(exhausted).ops.end)
            .filter(|&stop| stop <= end)
            .and_then(|stop| constant_return(ops, stop))
            .map(|(start, _)| start)
            .or_else(|| {
                // The exhaustion path may contain another loop before the
                // shared return. Only fold a local load after iterator cleanup.
                local_return(ops, end).filter(|&at| {
                    ops[header].name == "FOR_ITER"
                        && self.flow.block(at).ops.end == end
                        && self.flow.handler(at).is_none()
                })
            })
            .filter(|&start| start >= exhausted);
        for block in self.flow.blocks_in(body) {
            let last = block.ops.end - 1;
            let after = if ops[last].name == "JUMP_FORWARD" {
                self.flow
                    .jump(last)
                    .filter(|&dest| dest >= exhausted && dest <= end)
            } else if let Some(after) = continuation
                && let Some(copy) = self.copied_return(ops, header, block.ops.end, after)
                && copy > block.ops.start
                && matches!(ops[copy - 1].name, "NOP" | "POP_ITER" | "POP_TOP")
            {
                active.copied_return = Some(after);
                Some(after)
            } else {
                None
            };
            if let Some(after) = after {
                if active.after.is_some_and(|other| other != after) {
                    return Err("multiple loop exit targets".into());
                }
                active.after = Some(after);
            }
        }
        Ok(active)
    }

    fn loop_else(
        &mut self,
        ops: &[Instruction],
        exhausted: usize,
        active: Loop,
        level: usize,
    ) -> Result<usize, String> {
        if let Some(after) = active.after
            && after > exhausted
        {
            let suite = self.fragment(ops, exhausted, after, level)?.suite()?;
            self.block("else:", &suite)?;
            Ok(after)
        } else {
            Ok(exhausted)
        }
    }

    pub(super) fn for_loop(
        &mut self,
        ops: &[Instruction],
        header: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let exit = self.flow.target(header)?;
        if exit > end || exit <= header + 2 || ops[exit].name != "END_FOR" {
            return Err("unsupported for-loop exit".into());
        }

        let iterable = self.expr()?;
        let after_cleanup = exit
            + 1
            + usize::from(
                ops.get(exit + 1)
                    .is_some_and(|op| matches!(op.name, "POP_ITER" | "POP_TOP")),
            );
        // A loop whose body always exits has no latch. Its last transfer
        // belongs to the body and must be emitted.
        let body_end = if self.flow.jump(exit - 1) == Some(header) {
            exit - 1
        } else if matches!(
            ops[exit - 1].name,
            "RETURN_VALUE" | "RETURN_CONST" | "RAISE_VARARGS" | "JUMP_FORWARD"
        ) || (ops[exit - 1].name == "JUMP_BACKWARD"
            && self
                .flow
                .jump(exit - 1)
                .is_some_and(|to| (header + 1..exit).contains(&to))
            && self
                .flow
                .incoming(header)
                .any(|from| (header + 1..exit).contains(&from)))
        {
            // A nested loop may own the last jump while an earlier arm
            // advances the iterator. Keep both transfers in the body.
            exit
        } else {
            return Err("unsupported for-loop latch".into());
        };
        let binding = self.binding(ops, header + 1, body_end)?;
        let active = self.loop_exits(ops, header, binding.next..body_end, after_cleanup, end)?;
        let outer = self.loop_edges.replace(active);
        let suite = self
            .seeded(ops, binding.next, body_end, level, binding.seed)?
            .suite()?;
        self.loop_edges = outer;
        self.block(&format!("for {} in {iterable}:", binding.target), &suite)?;
        self.loop_else(ops, after_cleanup, active, level)
    }

    pub(super) fn while_latch(
        &self,
        ops: &[Instruction],
        predicate: &Predicate,
        end: usize,
    ) -> Option<WhileLatch> {
        let (header, start, exit) = (predicate.header?, predicate.suite, predicate.target);
        if start >= exit {
            return None;
        }
        let backedge = |target| {
            self.flow.incoming(target).rev().find(|&i| {
                (start..exit).contains(&i)
                    && ops[i].name == "JUMP_BACKWARD"
                    && self.flow.jump(i) == Some(target)
            })
        };
        // 3.12 and 3.13 may duplicate the entire condition at the bottom,
        // even when a continue in the body jumps to the original condition.
        if let Some(jump) = backedge(start)
            && let Some(latch) = self.copied_while_latch(ops, predicate, header, jump, end)
        {
            return Some(latch);
        }
        // A condition can span multiple blocks, e.g. the fast and fallback
        // paths of any() over a generator. The backedge must re-enter the
        // expression where it began with an empty operand stack.
        if self.flow.block(header).ops.start != header {
            return None;
        }
        let jump = backedge(header)?;
        Some(WhileLatch {
            header,
            implicit_return: false,
            // An early continue can precede other statements in the loop.
            body_end: if jump + 1 == exit { jump } else { exit },
        })
    }

    /// At the end of a for suite CPython can jump into the inner while body,
    /// with a copy of the outer continue on each false edge.
    pub(super) fn enclosing_continue_latch(
        &self,
        ops: &[Instruction],
        predicate: &Predicate,
        end: usize,
    ) -> Option<WhileLatch> {
        let body = predicate.target;
        let header = predicate.header?;
        let test = end.checked_sub(2)?;
        if body != predicate.suite + 1
            || end >= ops.len()
            || body > test
            || self.loop_transfer(ops, predicate.suite) != Some("continue")
            || ops[end - 1].name != "JUMP_BACKWARD"
        {
            return None;
        }
        if self.flow.jump(end - 1) == Some(header)
            && self.flow.block(header).ops.start == header
            && end == self.loop_edges?.body_end
        {
            return Some(WhileLatch {
                header,
                body_end: end - 1,
                implicit_return: false,
            });
        }
        if self.loop_transfer(ops, end) != Some("continue")
            || self.flow.jump(end - 1) != Some(body)
            || self.flow.jump(test) != Some(end)
            || !matches!(
                (ops[predicate.branch].name, ops[test].name),
                ("POP_JUMP_IF_TRUE", "POP_JUMP_IF_FALSE")
                    | ("POP_JUMP_IF_FALSE", "POP_JUMP_IF_TRUE")
                    | ("POP_JUMP_IF_NONE", "POP_JUMP_IF_NOT_NONE")
                    | ("POP_JUMP_IF_NOT_NONE", "POP_JUMP_IF_NONE")
            )
        {
            return None;
        }
        Some(WhileLatch {
            header,
            body_end: self.predicate_copy_start(ops, header, predicate.branch, body, test)?,
            implicit_return: false,
        })
    }

    fn copied_while_latch(
        &self,
        ops: &[Instruction],
        predicate: &Predicate,
        header: usize,
        jump: usize,
        end: usize,
    ) -> Option<WhileLatch> {
        let (branch, exit) = (predicate.branch, predicate.target);
        let test = jump.checked_sub(1)?;
        if ops[test].name != ops[branch].name || self.flow.jump(test)? != jump + 1 {
            return None;
        }
        let implicit_return = jump + 1 != exit
            && end == self.terminal_returns.end
            && self.terminal_returns.contains(&(jump + 1))
            && self.terminal_returns.contains(&exit);
        if jump + 1 != exit && !implicit_return {
            // Both tests can exit via separate copies of a constant return.
            let (at, value) = constant_return(ops, exit)?;
            let stop = exit + (exit - at);
            if at != jump + 1 || stop > end || constant_return(ops, stop) != Some((exit, value)) {
                return None;
            }
        }
        Some(WhileLatch {
            header,
            body_end: self.predicate_copy_start(ops, header, branch, predicate.suite, test)?,
            implicit_return,
        })
    }

    fn predicate_copy_start(
        &self,
        ops: &[Instruction],
        header: usize,
        branch: usize,
        body: usize,
        test: usize,
    ) -> Option<usize> {
        let copy = test.checked_sub(branch - header)?;
        if copy < body
            || !(header..branch).zip(copy..test).all(|(a, b)| {
                if ops[a].opcode != ops[b].opcode {
                    return false;
                }
                match (self.flow.jump(a), self.flow.jump(b)) {
                    // A copied SEND, for example, jumps to its own END_SEND.
                    (Some(x), Some(y)) if (header..=branch).contains(&x) => {
                        y == copy + (x - header)
                    }
                    (Some(x), Some(y)) => {
                        x == y
                            || (self.terminal_returns.contains(&x)
                                && self.terminal_returns.contains(&y))
                    }
                    (None, None) => ops[a].arg == ops[b].arg,
                    _ => false,
                }
            })
        {
            return None;
        }
        Some(copy)
    }

    pub(super) fn while_loop(
        &mut self,
        ops: &[Instruction],
        predicate: &Predicate,
        latch: &WhileLatch,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let (start, target) = (predicate.suite, predicate.target);
        let active = self.loop_exits(ops, latch.header, start..latch.body_end, target, end)?;
        let falls_out = latch.body_end == target
            && self
                .flow
                .incoming(target)
                .any(|from| (start..target).contains(&from));
        if falls_out && active.after.is_some_and(|after| after > target) {
            return Err("loop falls into its else suite".into());
        }
        let outer = self.loop_edges.replace(active);
        let mut suite = self.fragment(ops, start, latch.body_end, level)?.suite()?;
        self.loop_edges = outer;
        if falls_out {
            // The other body paths jumped back to the header. Falling off the
            // recovered suite would repeat the loop on its exit path as well.
            suite.push_str("break\n");
        }
        self.block(&format!("while {}:", predicate.test(true)), &suite)?;
        if latch.implicit_return && active.after.is_none_or(|after| after == target) {
            // The copied false exits already return None. Writing an explicit
            // post-loop return would merge their distinct return blocks.
            return Ok(end);
        }
        self.loop_else(ops, target, active, level)
    }
}
