use super::conditionals::Predicate;
use super::regions::{ConstantReturn, constant_return};
use super::{Body, Instruction};

pub(super) struct WhileLatch {
    header: usize,
    body_end: usize,
    implicit_return: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Loop {
    header: usize,
    after: Option<usize>,
    // CPython may copy the continuation's constant return into a break arm.
    copied_return: Option<ConstantReturn>,
}

impl Body<'_, '_, '_> {
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

    pub(super) fn folded_break(&self, ops: &[Instruction], i: usize, end: usize) -> bool {
        let Some(expected) = self.loop_edges.and_then(|active| active.copied_return) else {
            return false;
        };
        let stop = self.flow.block(i).ops.end;
        stop <= end
            && constant_return(ops, stop).is_some_and(|(start, value)| {
                start > i
                    // A single iterator uses up to two slots, plus a tracing
                    // NOP. Leave any earlier user statements in the suite.
                    && start - i <= 3
                    && value == expected
                    && ops[i..start]
                        .iter()
                        .all(|op| matches!(op.name, "NOP" | "POP_ITER" | "POP_TOP"))
            })
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
            after: None,
            copied_return: None,
        };
        let continuation = (exhausted < end)
            .then(|| self.flow.block(exhausted).ops.end)
            .filter(|&stop| stop <= end)
            .and_then(|stop| constant_return(ops, stop))
            .filter(|(start, _)| *start >= exhausted);
        for block in self.flow.blocks_in(body) {
            let last = block.ops.end - 1;
            let after = if ops[last].name == "JUMP_FORWARD" {
                self.flow
                    .jump(last)
                    .filter(|&dest| dest >= exhausted && dest <= end)
            } else if let Some((after, expected)) = continuation
                && let Some((copy, value)) = constant_return(ops, block.ops.end)
                && value == expected
                && copy > block.ops.start
                && matches!(ops[copy - 1].name, "NOP" | "POP_ITER" | "POP_TOP")
            {
                active.copied_return = Some(expected);
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
        ) {
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
        let copy = test.checked_sub(branch - header)?;
        if copy < predicate.suite
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
        Some(WhileLatch {
            header,
            body_end: copy,
            implicit_return,
        })
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
