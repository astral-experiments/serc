use super::conditionals::Predicate;
use super::regions::{ConstantReturn, constant_return};
use super::{Body, Instruction};

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

    /// Walk back to the first instruction producing the predicate. A latch
    /// returning to an earlier statement in the same block belongs to an outer
    /// loop, even if it immediately precedes this conditional's other arm.
    fn condition_start(&self, ops: &[Instruction], branch: usize) -> Option<usize> {
        let mut inputs = 1usize;
        for i in (self.flow.block(branch).ops.start..branch).rev() {
            let op = ops[i];
            let (pops, pushes) =
                serc_bytecode::target::stack_counts_for(self.m.version, op.opcode, op.arg);
            inputs = inputs.checked_sub(pushes)? + pops;
            if inputs == 0 {
                return Some(i);
            }
        }
        None
    }

    pub(super) fn while_latch(
        &self,
        ops: &[Instruction],
        predicate: &Predicate,
        end: usize,
    ) -> Option<usize> {
        let (start, exit) = (predicate.suite, predicate.target);
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
        if let Some(header) = self.condition_start(ops, predicate.first)
            && self.flow.block(header).ops.start == header
            && let Some(latch) = backedge(header)
        {
            return Some(latch);
        }

        // 3.12 and 3.13 jump back to the body after a second copy of the
        // predicate. Its false edge may reach a copy of the loop's return.
        let latch = backedge(start)?;
        if latch <= start || ops[latch - 1].name != ops[predicate.branch].name {
            return None;
        }
        let dest = self.flow.jump(latch - 1)?;
        let same_exit = (dest == exit && latch + 1 == exit)
            || (dest == latch + 1
                && constant_return(ops, exit).is_some_and(|(at, value)| {
                    let stop = exit + (exit - at);
                    at == dest && stop <= end && constant_return(ops, stop) == Some((exit, value))
                }));
        same_exit.then_some(latch)
    }

    pub(super) fn while_loop(
        &mut self,
        ops: &[Instruction],
        predicate: &Predicate,
        latch: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let (start, target) = (predicate.suite, predicate.target);
        let back = self.flow.target(latch)?;
        let body_end = if back == start {
            let condition = self
                .condition_start(ops, predicate.branch)
                .ok_or("missing copied loop predicate")?;
            let width = predicate.branch - condition;
            let copy = (latch - 1)
                .checked_sub(width)
                .filter(|&n| n >= start)
                .ok_or("incomplete copied loop condition")?;
            if !super::comparisons::same_instructions(
                &ops[condition..predicate.branch],
                &ops[copy..latch - 1],
            ) {
                return Err("distinct copied loop condition".into());
            }
            copy
        } else if latch + 1 == target {
            latch
        } else {
            // An early backedge is a continue. A different path still has
            // statements between that edge and the condition's exit.
            target
        };
        let active = self.loop_exits(ops, back, start..body_end, target, end)?;
        let outer = self.loop_edges.replace(active);
        let suite = self.fragment(ops, start, body_end, level)?.suite()?;
        self.loop_edges = outer;
        self.block(&format!("while {}:", predicate.test(true)), &suite)?;
        if back == start
            && self.flow.jump(latch - 1) != Some(target)
            && self.terminal_returns.contains(&target)
            && self.terminal_returns.end == end
            && active.after.is_none_or(|after| after == target)
        {
            // The copied false exits already return None. Writing an explicit
            // post-loop return would merge their distinct return blocks.
            return Ok(end);
        }
        self.loop_else(ops, target, active, level)
    }
}
