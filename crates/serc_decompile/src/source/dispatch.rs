//! Follow failed exception matches in order and separate the successful
//! try path from the continuation shared with handled exceptions.
use std::ops::Range;

use serc_bytecode::instructions::Handler;

use super::protected_regions::ExitCopies;
use super::{Body, Instruction};
use crate::flow::Graph;

pub(super) struct Dispatch {
    arms: Vec<Arm>,
    success: Range<usize>,
    continuation: Range<usize>,
    retry: bool,
}

pub(super) struct Arm {
    kind: Option<Range<usize>>,
    pub(super) binding: usize,
    body: Range<usize>,
    exit: Exit,
    copies: Option<ExitCopies>,
}

enum Exit {
    Raise,
    Jump(usize),
    Return(Range<usize>),
    ReturnValue,
}

impl Arm {
    pub(super) fn returns(&self) -> bool {
        matches!(self.exit, Exit::Return(_) | Exit::ReturnValue)
    }
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
        let reraise = ops.get(cleanup + 3)?;
        if (reraise.name, reraise.arg) != ("RERAISE", 1) {
            return None;
        }
        self.cleared_binding(ops, cleanup)
    }

    pub(super) fn cleared_binding(
        &self,
        ops: &[Instruction],
        cleanup: usize,
    ) -> Option<Instruction> {
        let [none, store, delete] = ops.get(cleanup..cleanup + 3)? else {
            return None;
        };
        let name = store.name.strip_prefix("STORE_")?;
        (matches!(none.name, "LOAD_CONST" | "LOAD_COMMON_CONSTANT")
            && self.none_constant(*none)
            && delete.name.strip_prefix("DELETE_") == Some(name)
            && delete.arg == store.arg)
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
        let (mut arms, join) = self.handler_arms(ops, handler + 1, cleanup)?;
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
        if matches!(arms.as_slice(), [arm] if matches!(arm.exit, Exit::ReturnValue)) {
            // The handler evaluated its own return with an active exception.
            // Only the successful try can reach the remaining statements.
            return Some(Dispatch {
                arms,
                success: success..success,
                continuation: success..handler,
                retry: false,
            });
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

    fn handler_arms(
        &self,
        ops: &[Instruction],
        mut cursor: usize,
        cleanup: usize,
    ) -> Option<(Vec<Arm>, Option<usize>)> {
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
            let (mut body, exit) = self.handler_body(ops, binding, next)?;
            // A guard in a matching arm can jump to its own copy of POP_EXCEPT,
            // binding cleanup, and constant return. Both exits end the arm.
            let copies = if let Exit::Return(tail) = &exit
                && super::regions::constant_return(ops, tail.end)
                    .is_some_and(|(load, _)| load == tail.start)
            {
                ExitCopies::handler(ops, &self.flow, body.clone(), tail.end)
            } else {
                None
            };
            if let Some(copies) = copies {
                body.end = copies.first;
            }
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
                copies,
            });
            cursor = next;
        }
        if arms.is_empty()
            || !(cursor == cleanup
                || (cursor + 1 == cleanup && (ops[cursor].name, ops[cursor].arg) == ("RERAISE", 0)))
        {
            return None;
        }
        Some((arms, join))
    }

    pub(super) fn following_arms(
        &self,
        ops: &[Instruction],
        start: usize,
        cleanup: usize,
    ) -> Option<(Vec<Arm>, Option<usize>)> {
        let (arms, join) = self.handler_arms(ops, start, cleanup)?;
        arms.iter()
            .all(|arm| match &arm.exit {
                // An expression evaluated after POP_EXCEPT belongs to a
                // continuation outside the handler. Constants can return here.
                Exit::Return(tail) => super::regions::constant_return(ops, tail.end)
                    .is_some_and(|(load, _)| load == tail.start),
                Exit::Raise | Exit::ReturnValue | Exit::Jump(_) => true,
            })
            .then_some((arms, join))
    }

    /// The try already returned its pending value. Each matched handler may
    /// raise, evaluate its own return, or restore the previous exception and
    /// reach a shared continuation copied after its `POP_EXCEPT`.
    pub(super) fn returning_handlers(
        &mut self,
        ops: &[Instruction],
        handler: usize,
    ) -> Result<(), String> {
        let (_, cleanup) = unwind(ops, &self.flow, handler, 1).ok_or("missing handler unwind")?;
        let (mut arms, join) = self
            .handler_arms(ops, handler + 1, cleanup)
            .ok_or("unsupported returning exception dispatch")?;
        if join.is_some() {
            return Err("unsupported exception continuation".into());
        }
        let tails: Vec<_> = arms
            .iter()
            .filter_map(|arm| match &arm.exit {
                Exit::Return(tail) => Some(&ops[tail.clone()]),
                _ => None,
            })
            .collect();
        let copied = tails.len() > 1
            && tails
                .windows(2)
                .all(|pair| super::comparisons::same_instructions(pair[0], pair[1]));
        let mut continuation = None;
        for arm in &mut arms {
            if let Exit::Return(tail) = &arm.exit
                && (copied
                    || super::regions::constant_return(ops, tail.end)
                        .is_none_or(|(load, _)| load != tail.start))
            {
                // This expression runs after the exception binding is cleared.
                let source = self.fragment(ops, tail.start, tail.end, 0)?.suite()?;
                if continuation.as_ref().is_some_and(|prior| prior != &source) {
                    return Err("distinct handler continuations".into());
                }
                continuation = Some(source);
                arm.exit = Exit::Jump(tail.start);
            }
            self.emit_arm(ops, arm, 0, true)?;
        }
        if let Some(continuation) = continuation {
            for line in continuation.lines() {
                self.line(line)?;
            }
        }
        Ok(())
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
        keyword: &str,
        kind: Option<Range<usize>>,
        binding: usize,
        level: usize,
    ) -> Result<String, String> {
        let mut header = keyword.to_owned();
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

    pub(super) fn emit_arm(
        &mut self,
        ops: &[Instruction],
        arm: &Arm,
        level: usize,
        keep_return: bool,
    ) -> Result<(), String> {
        let header = self.except_header(ops, "except", arm.kind.clone(), arm.binding, level)?;
        let fragment = self.protected_fragment(ops, arm.body.clone(), level, arm.copies)?;
        let mut suite = if matches!(arm.exit, Exit::ReturnValue) {
            let (suite, returns) = fragment.pending_return()?;
            if !returns {
                return Err("missing exception return value".into());
            }
            suite
        } else {
            fragment.suite()?
        };
        if let Exit::Return(tail) = &arm.exit {
            let tail = self.fragment(ops, tail.start, tail.end, level)?.suite()?;
            if keep_return || suite.is_empty() || tail != "return None\n" {
                suite.push_str(&tail);
            }
        }
        self.block(&header, &suite)?;
        Ok(())
    }

    pub(super) fn emit_dispatch(
        &mut self,
        ops: &[Instruction],
        dispatch: &Dispatch,
        try_source_start: usize,
    ) -> Result<(), String> {
        for arm in &dispatch.arms {
            self.emit_arm(
                ops,
                arm,
                0,
                dispatch.retry || !dispatch.continuation.is_empty(),
            )?;
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
