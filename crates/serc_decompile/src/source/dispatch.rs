//! Follow failed exception matches in order and separate the successful
//! try path from the continuation shared with handled exceptions.
use std::ops::Range;

use serc_bytecode::instructions::Handler;

use super::{Body, Instruction, Object};
use crate::flow::Graph;

pub(super) struct Dispatch {
    arms: Vec<Arm>,
    success: Range<usize>,
    continuation: Range<usize>,
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
        let [none, store, delete, reraise] = ops.get(cleanup..failed)? else {
            return None;
        };
        let unwind = self.flow.handler(binding + 1)?;
        let loads_none = match none.name {
            "LOAD_CONST" => matches!(
                self.m.objects[self.item(self.code.constants, none.arg).ok()?],
                Object::None
            ),
            "LOAD_COMMON_CONSTANT" => none.arg == super::CommonConstant::NONE.argument(),
            _ => false,
        };
        ((unwind.target, unwind.depth, unwind.lasti) == (none.offset, 1, true)
            && loads_none
            && (store.name, store.arg) == (ops[binding].name, ops[binding].arg)
            && store.name.starts_with("STORE_")
            && delete.name.strip_prefix("DELETE_") == store.name.strip_prefix("STORE_")
            && delete.arg == store.arg
            && (reraise.name, reraise.arg) == ("RERAISE", 1))
            .then_some(cleanup)
    }

    pub(super) fn exception_dispatch(
        &self,
        ops: &[Instruction],
        handler: usize,
        success: usize,
        limit: usize,
    ) -> Option<Dispatch> {
        let (_, cleanup) = unwind(ops, &self.flow, handler, 1)?;
        if cleanup + 3 != limit {
            return None;
        }
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
                && (!(success..handler).contains(&dest)
                    || join.replace(dest).is_some_and(|old| old != dest))
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
        let join = join.unwrap_or(handler);
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
            if !super::regions::constant_return(ops, tail.end)
                .is_some_and(|(start, _)| start == tail.start)
            {
                continuation = tail.clone();
                arm.exit = Exit::Jump(continuation.start);
            }
        }
        Some(Dispatch {
            arms,
            success: success..join,
            continuation,
        })
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
        let pop = body.clone().rfind(|&i| ops[i].name == "POP_EXCEPT")?;
        let mut after = pop + 1;
        if stop != next {
            // The normal exit must clear the same name as the unwind.
            if !ops
                .get(after..after + 3)?
                .iter()
                .zip(&ops[stop..next])
                .all(|(a, b)| (a.name, a.arg) == (b.name, b.arg))
            {
                return None;
            }
            after += 3;
        }
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

    pub(super) fn emit_dispatch(
        &mut self,
        ops: &[Instruction],
        dispatch: Dispatch,
    ) -> Result<(), String> {
        for arm in dispatch.arms {
            let mut header = "except".to_owned();
            if let Some(kind) = arm.kind {
                header.push(' ');
                header.push_str(&self.fragment(ops, kind.start, kind.end, 0)?.expression()?);
            }
            let op = ops[arm.binding];
            if op.name != "POP_TOP" {
                let table = if matches!(op.name, "STORE_NAME" | "STORE_GLOBAL") {
                    self.code.names
                } else {
                    self.code.locals
                };
                header.push_str(" as ");
                header.push_str(self.name(table, op.arg)?);
            }
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
                if !dispatch.continuation.is_empty() || suite.is_empty() || tail != "return None\n"
                {
                    suite.push_str(&tail);
                }
            }
            header.push(':');
            self.block(&header, &suite)?;
        }
        if !dispatch.success.is_empty() {
            let suite = self
                .fragment(ops, dispatch.success.start, dispatch.success.end, 0)?
                .suite()?;
            // CPython can duplicate the implicit return in every terminal arm.
            // Spelling it explicitly can move None ahead of keyword tuples.
            let suite = if dispatch.continuation.is_empty() {
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
        self.region(
            ops,
            dispatch.continuation.start,
            dispatch.continuation.end,
            0,
        )
    }
}
