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

    pub(super) fn while_loop(
        &mut self,
        ops: &[Instruction],
        predicate: &Predicate,
        test: &str,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let (i, start, target) = (predicate.branch, predicate.suite, predicate.target);
        let tail = target - 1;
        let back = self.flow.target(tail)?;
        let body_end = if back <= i {
            tail
        } else if back == start && tail > start && ops[tail - 1].jump == ops[i].jump {
            // CPython 3.12 duplicates the condition at the bottom. Match
            // its complete operand sequence before removing the copy.
            let mut before = i;
            let mut copy = tail - 1;
            let mut inputs = 1usize;
            while inputs > 0 {
                if before == 0 || copy <= start {
                    return Err("incomplete copied loop condition".into());
                }
                before -= 1;
                copy -= 1;
                let op = ops[before];
                if op.opcode != ops[copy].opcode || op.arg != ops[copy].arg {
                    return Err("distinct copied loop condition".into());
                }
                let (pops, pushes) =
                    serc_bytecode::target::stack_counts_for(self.m.version, op.opcode, op.arg);
                inputs = inputs
                    .checked_sub(pushes)
                    .ok_or("shared operand in copied loop condition")?
                    + pops;
            }
            copy
        } else {
            return Err("unsupported loop condition".into());
        };
        let active = self.loop_exits(ops, back, start..body_end, target, end)?;
        let outer = self.loop_edges.replace(active);
        let suite = self.fragment(ops, start, body_end, level)?.suite()?;
        self.loop_edges = outer;
        self.block(&format!("while {test}:"), &suite)?;
        self.loop_else(ops, target, active, level)
    }
}
