//! Follow failed exception matches in order and separate the successful
//! try path from the continuation shared with handled exceptions.
use std::ops::Range;

use serc_bytecode::instructions::Handler;

use super::{Body, Instruction};
use crate::flow::Graph;

pub(super) struct Dispatch {
    arms: Vec<Arm>,
    success: Range<usize>,
    continuation: Range<usize>,
    retry: bool,
}

struct Arm {
    kind: Option<Range<usize>>,
    binding: usize,
    body: Range<usize>,
    exit: Exit,
}

enum Exit {
    Raise,
    Jump(usize),
    Return(Range<usize>),
    ReturnValue,
}

/// The false edge of the match selects the next clause or the final reraise.
pub(super) fn typed_handler(
    flow: &Graph,
    ops: &[Instruction],
    at: usize,
) -> Option<(usize, usize, usize)> {
    ops.get(at)?;
    let branch = flow.block(at).ops.end.checked_sub(1)?;
    let check = branch.checked_sub(1)?;
    let next = flow.jump(branch)?;
    let bind = branch + 1 + usize::from(ops.get(branch + 1)?.name == "NOT_TAKEN");
    (check > at
        && ops[check].name == "CHECK_EXC_MATCH"
        && ops[branch].name == "POP_JUMP_IF_FALSE"
        && bind < next)
        .then_some((check, bind, next))
}

pub(super) fn unwind(
    ops: &[Instruction],
    flow: &Graph,
    dispatch: usize,
    depth: usize,
) -> Option<(Handler, usize)> {
    let h = flow.handler(dispatch)?;
    let target = ops.binary_search_by_key(&h.target, |op| op.offset).ok()?;
    (h.lasti
        && h.depth == depth
        && target > dispatch
        && matches!(ops.get(target..target + 3)?,
        [copy, pop, raise] if (copy.name, copy.arg) == ("COPY", 3) && pop.name == "POP_EXCEPT"
            && (raise.name, raise.arg) == ("RERAISE", 1)))
    .then_some((h, target))
}

impl Body<'_, '_, '_> {
    /// A bound exception has an extra unwind edge which deletes the binding.
    pub(super) fn exception_name_cleanup(
        &self,
        ops: &[Instruction],
        binding: usize,
        failed: usize,
    ) -> Option<usize> {
        let cleanup = failed.checked_sub(4)?;
        let unwind = self.flow.handler(binding + 1)?;
        let store = self.cleared_name(ops, cleanup)?;
        ((unwind.target, unwind.depth, unwind.lasti) == (ops[cleanup].offset, 1, true)
            && (store.name, store.arg) == (ops[binding].name, ops[binding].arg))
            .then_some(cleanup)
    }

    fn cleared_name(&self, ops: &[Instruction], cleanup: usize) -> Option<Instruction> {
        let [none, store, delete, reraise] = ops.get(cleanup..cleanup + 4)? else {
            return None;
        };
        let name = store.name.strip_prefix("STORE_")?;
        (matches!(none.name, "LOAD_CONST" | "LOAD_COMMON_CONSTANT")
            && self.none_constant(*none)
            && delete.name.strip_prefix("DELETE_") == Some(name)
            && delete.arg == store.arg
            && (reraise.name, reraise.arg) == ("RERAISE", 1))
            .then_some(*store)
    }

    /// A value returned from an except suite stays on the stack while CPython
    /// restores the active exception and clears the optional binding.
    pub(super) fn handler_value_return(
        &self,
        ops: &[Instruction],
        swap: usize,
        end: usize,
    ) -> Option<usize> {
        let pop = swap + 1;
        if self.stack.len() != 1 || ops[swap].arg != 2 || ops.get(pop)?.name != "POP_EXCEPT" {
            return None;
        }
        let (outer, _) = unwind(ops, &self.flow, swap, 1)?;
        let mut ret = pop + 1;
        if ops.get(ret)?.name != "RETURN_VALUE" {
            let h = self.flow.handler(swap.checked_sub(1)?)?;
            let cleanup = ops.binary_search_by_key(&h.target, |op| op.offset).ok()?;
            self.cleared_name(ops, cleanup)?;
            if !h.lasti
                || h.depth != outer.depth
                || self.flow.handler(cleanup)?.target != outer.target
                || !super::comparisons::same_instructions(
                    ops.get(ret..ret + 3)?,
                    &ops[cleanup..cleanup + 3],
                )
            {
                return None;
            }
            ret += 3;
        }
        (ret < end && ops[ret].name == "RETURN_VALUE").then_some(ret)
    }

    /// After dropping the iterator a return also restores the active exception
    /// and, for a bound handler, clears its name exactly as on unwind.
    pub(super) fn handler_constant_return(
        &self,
        ops: &[Instruction],
        protected: usize,
        pop: usize,
        end: usize,
    ) -> Option<usize> {
        if pop >= end || ops[pop].name != "POP_EXCEPT" || self.flow.handler(pop).is_some() {
            return None;
        }
        let stop = self.flow.block(pop).ops.end;
        let (load, _) = super::regions::constant_return(ops, stop)?;
        if stop > end || load <= pop {
            return None;
        }
        if load == pop + 1 {
            unwind(ops, &self.flow, protected, 1)?;
        } else {
            let h = self.flow.handler(protected)?;
            let cleanup = ops.binary_search_by_key(&h.target, |op| op.offset).ok()?;
            self.cleared_name(ops, cleanup)?;
            if !h.lasti
                || h.depth != 1
                || !super::comparisons::same_instructions(
                    &ops[pop + 1..load],
                    &ops[cleanup..cleanup + 3],
                )
            {
                return None;
            }
        }
        Some(load)
    }

    pub(super) fn exception_dispatch(
        &self,
        bc: &super::Bytecode,
        handler: usize,
        start: usize,
        success: usize,
        limit: usize,
    ) -> Option<Dispatch> {
        let ops = &bc.ops;
        let (_, cleanup) = unwind(ops, &self.flow, handler, 1)?;
        let after = cleanup + 3;
        // When the try raises or returns, only a matching handler can reach
        // the code after its unwind. A constant return lies just outside
        // the protected interval. Bound handlers clear the exception name first.
        let handled_tail = if after == limit {
            None
        } else {
            let raises = success == handler && ops[handler - 1].name == "RAISE_VARARGS";
            let returns = super::regions::constant_return(ops, handler)
                .is_some_and(|(start, _)| start == success);
            if after >= limit || !(raises || returns) {
                return None;
            }
            let rest = bc
                .handlers
                .partition_point(|entry| entry.start < ops[after].offset);
            let (remaining, end) =
                super::comprehensions::protection(bc, &bc.handlers[rest..]).ok()?;
            if !remaining.is_empty() {
                return None;
            }
            Some(after..self.flow.normal_end([after]).min(end).min(limit))
        };
        let mut cursor = handler + 1;
        let mut arms = Vec::new();
        let mut join = None;
        while cursor < cleanup && ops[cursor].name != "RERAISE" {
            let (kind, binding, next) = if ops[cursor].name == "POP_TOP" {
                (None, cursor, cleanup)
            } else {
                let (check, binding, next) = typed_handler(&self.flow, ops, cursor)?;
                if next >= cleanup {
                    return None;
                }
                (Some(cursor..check), binding, next)
            };
            let (body, exit) = self.handler_body(ops, binding, next)?;
            if let Exit::Jump(dest) = exit
                && join.replace(dest).is_some_and(|old| old != dest)
            {
                return None;
            }
            arms.push(Arm {
                kind,
                binding,
                body,
                exit,
            });
            cursor = next;
        }
        if arms.is_empty()
            || !(cursor == cleanup
                || (cursor + 1 == cleanup && (ops[cursor].name, ops[cursor].arg) == ("RERAISE", 0)))
        {
            return None;
        }
        // A retrying handler jumps to the NOP at the try header. Older
        // targets take that backedge through a jump after the unwind.
        let retry_header = match (&handled_tail, join) {
            (None, dest) => dest,
            (Some(tail), Some(dest))
                if tail.start == dest && tail.len() == 1 && ops[dest].name == "JUMP_BACKWARD" =>
            {
                self.flow.jump(dest)
            }
            _ => None,
        };
        if self.none_return(&ops[success..handler])
            && retry_header.is_some_and(|header| {
                header < start && ops[header..start].iter().all(|op| op.name == "NOP")
            })
        {
            return Some(Dispatch {
                arms,
                success: success..handler,
                continuation: handler..handler,
                retry: true,
            });
        }
        if let Some(tail) = handled_tail {
            return (join == Some(tail.start)).then_some(Dispatch {
                arms,
                success: success..handler,
                continuation: tail,
                retry: false,
            });
        }
        if join.is_some_and(|dest| !(success..handler).contains(&dest)) {
            return None;
        }
        if let [arm] = arms.as_slice()
            && matches!(arm.exit, Exit::Raise)
            && success < handler
        {
            // A conditional raise at the end of an else suite can jump to a
            // tracing NOP before the next statement. Moving the guard outside
            // the else drops that NOP and shifts the out-of-line handler.
            let branch = self.flow.block(success).ops.end - 1;
            let next = self.flow.jump(branch)?;
            if ops[branch].name.starts_with("POP_JUMP_IF_")
                && (branch + 1..handler).contains(&next)
                && ops[next - 1].name == "RAISE_VARARGS"
                && ops[next].name == "NOP"
            {
                return Some(Dispatch {
                    arms,
                    success: success..next,
                    continuation: next..handler,
                    retry: false,
                });
            }
        }
        let mut join = join.unwrap_or(handler);
        let mut continuation = join..handler;
        if let [arm] = arms.as_mut_slice() {
            // A lone empty handler may return a constant, or fall through to
            // a separate return after a successful else has exited. In the
            // latter case the expression runs after POP_EXCEPT has restored
            // the previous exception.
            let Exit::Return(tail) = &arm.exit else {
                return None;
            };
            if !arm.body.is_empty() || arm.kind.is_none() || success >= handler {
                return None;
            }
            let normal = &ops[success..handler];
            let handled = &ops[tail.clone()];
            if normal.len() == handled.len()
                && normal
                    .iter()
                    .zip(handled)
                    .all(|(a, b)| a.jump.is_none() && (a.opcode, a.arg) == (b.opcode, b.arg))
            {
                // CPython copied one straight-line continuation onto both
                // exits. It executes after the active exception is restored.
                join = success;
                continuation = success..handler;
                arm.exit = Exit::Jump(success);
            } else if super::regions::constant_return(ops, tail.end)
                .is_none_or(|(start, _)| start != tail.start)
            {
                continuation = tail.clone();
                arm.exit = Exit::Jump(continuation.start);
            }
        }
        Some(Dispatch {
            arms,
            success: success..join,
            continuation,
            retry: false,
        })
    }

    /// Locate `POP_EXCEPT` and the first instruction after normal binding cleanup.
    pub(super) fn handler_pop(
        ops: &[Instruction],
        binding: usize,
        stop: usize,
    ) -> Option<(usize, usize)> {
        let pop = (binding + 1..stop).rfind(|&i| ops[i].name == "POP_EXCEPT")?;
        let mut after = pop + 1;
        if ops[binding].name != "POP_TOP" {
            // The normal exit must clear the same name as the unwind.
            if !ops
                .get(after..after + 3)?
                .iter()
                .zip(ops.get(stop..stop + 3)?)
                .all(|(a, b)| (a.name, a.arg) == (b.name, b.arg))
            {
                return None;
            }
            after += 3;
        }
        Some((pop, after))
    }

    fn handler_body(
        &self,
        ops: &[Instruction],
        binding: usize,
        next: usize,
    ) -> Option<(Range<usize>, Exit)> {
        let stop = if ops[binding].name == "POP_TOP" {
            next
        } else {
            self.exception_name_cleanup(ops, binding, next)?
        };
        let body = binding + 1..stop;
        if ops.get(body.clone())?.last()?.name == "RAISE_VARARGS" {
            return Some((body, Exit::Raise));
        }
        let (pop, after) = Self::handler_pop(ops, binding, stop)?;
        let exit = stop.checked_sub(1)?;
        if after > exit {
            return None;
        }
        let outcome = match ops[exit].name {
            "JUMP_FORWARD" | "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT" if after == exit => {
                Exit::Jump(self.flow.jump(exit)?)
            }
            "RETURN_VALUE"
                if after == exit
                    && pop > body.start
                    && (ops[pop - 1].name, ops[pop - 1].arg) == ("SWAP", 2) =>
            {
                return Some((body.start..pop - 1, Exit::ReturnValue));
            }
            "RETURN_VALUE" | "RETURN_CONST" => Exit::Return(after..stop),
            _ => return None,
        };
        Some((body.start..pop, outcome))
    }

    pub(super) fn except_header(
        &mut self,
        ops: &[Instruction],
        kind: Option<Range<usize>>,
        binding: usize,
        level: usize,
    ) -> Result<String, String> {
        let mut header = "except".to_owned();
        if let Some(kind) = kind {
            header.push(' ');
            header.push_str(
                &self
                    .fragment(ops, kind.start, kind.end, level)?
                    .expression()?,
            );
        }
        let op = ops[binding];
        if op.name != "POP_TOP" {
            let table = if matches!(op.name, "STORE_NAME" | "STORE_GLOBAL") {
                self.code.names
            } else {
                self.code.locals
            };
            header.push_str(" as ");
            header.push_str(self.name(table, op.arg)?);
        }
        header.push(':');
        Ok(header)
    }

    pub(super) fn emit_dispatch(
        &mut self,
        ops: &[Instruction],
        dispatch: Dispatch,
        try_source_start: usize,
    ) -> Result<(), String> {
        for arm in dispatch.arms {
            let header = self.except_header(ops, arm.kind, arm.binding, 0)?;
            let fragment = self.fragment(ops, arm.body.start, arm.body.end, 0)?;
            let mut suite = if matches!(arm.exit, Exit::ReturnValue) {
                let (suite, returns) = fragment.pending_return()?;
                if !returns {
                    return Err("missing exception return value".into());
                }
                suite
            } else {
                fragment.suite()?
            };
            if let Exit::Return(tail) = arm.exit {
                let tail = self.fragment(ops, tail.start, tail.end, 0)?.suite()?;
                if dispatch.retry
                    || !dispatch.continuation.is_empty()
                    || suite.is_empty()
                    || tail != "return None\n"
                {
                    suite.push_str(&tail);
                }
            }
            self.block(&header, &suite)?;
        }
        if !dispatch.success.is_empty() {
            let suite = self
                .fragment(ops, dispatch.success.start, dispatch.success.end, 0)?
                .suite()?;
            // CPython can duplicate the implicit return in every terminal arm.
            // Spelling it explicitly can move None ahead of keyword tuples.
            let suite = if dispatch.continuation.is_empty() && !dispatch.retry {
                suite
                    .strip_suffix("return None\n")
                    .filter(|s| s.is_empty() || s.ends_with('\n'))
                    .unwrap_or(&suite)
            } else {
                &suite
            };
            if !suite.is_empty() {
                self.block("else:", suite)?;
            }
        }
        if dispatch.retry {
            let suite = self.lines.split_off(try_source_start);
            return self.block("while True:", &suite);
        }
        self.region(
            ops,
            dispatch.continuation.start,
            dispatch.continuation.end,
            0,
        )
    }
}
