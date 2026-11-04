//! Identify the exceptional finalizer and its copies on the normal paths.
use super::{Body, Clause, Graph, Handler, Instruction, Protected, Range};
use crate::source::{PythonVersion, finalizers::CopyExit, regions::ReturnExit};

/// In a loop with no fallthrough every return runs its own normal copy of
/// the finalizer. The exceptional copy supplies the shared finally suite.
pub(super) fn returning_finally_loop(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    entries: &[Handler],
    start: usize,
    dispatch: usize,
    cleanup: usize,
) -> Option<Protected> {
    let flow = &owner.flow;
    if ops[start].name != "NOP"
        || entries.len() < 2
        || (ops[cleanup - 1].name, ops[cleanup - 1].arg) != ("RERAISE", 0)
    {
        return None;
    }
    let header = start + usize::from(owner.m.version < PythonVersion::Py314);
    let latch = flow.incoming(header).rev().find(|&i| {
        (header..dispatch).contains(&i)
            && ops[i].name == "JUMP_BACKWARD"
            && flow.jump(i) == Some(header)
    })?;
    let end = latch + 1;
    let first = entries[0];
    let split = ops[dispatch + 1..cleanup - 1]
        .iter()
        .any(|op| (op.name, op.arg) == ("RERAISE", 0));
    let handler = dispatch + 1..if split { cleanup } else { cleanup - 1 };
    let exit = if split {
        CopyExit::PendingReturn
    } else {
        CopyExit::Fallthrough
    };
    let mut exits = Vec::new();
    let mut at = start;
    while at < end {
        let block = flow.block(at);
        if block.ops.start < start
            || block.ops.end > end
            || [flow.jump(block.ops.end - 1), flow.fallthrough(block)]
                .into_iter()
                .flatten()
                .any(|to| !(start..end).contains(&to))
        {
            return None;
        }
        if flow
            .handler(at)
            .is_some_and(|h| (h.target, h.depth, h.lasti) == (first.target, first.depth, false))
        {
            if matches!(ops[block.ops.end - 1].name, "RETURN_VALUE" | "RETURN_CONST") {
                return None;
            }
            at = block.ops.end;
        } else if owner.delegated_cleanup(ops, at).is_some()
            || (ops[at].name == "NOT_TAKEN"
                && at > start
                && ops[at - 1].name.starts_with("POP_JUMP_IF_"))
        {
            at += 1;
        } else {
            let matched = owner.finally_copy_end(ops, at..end, handler.clone(), exit)?;
            let ret = matched.checked_sub(usize::from(split))?;
            if ret >= end
                || ops.get(ret)?.name != "RETURN_VALUE"
                || flow.block(at).height != Some(first.depth + 1)
            {
                return None;
            }
            // A branch into the middle of a copy could bypass part of the
            // cleanup. Only the suspension protocol may re-enter END_SEND.
            let enclosing = flow.handler(cleanup).map(|h| (h.target, h.depth, h.lasti));
            if flow.blocks_in(at..ret + 1).any(|b| {
                flow.handler(b.ops.start).is_some_and(|h| {
                    Some((h.target, h.depth, h.lasti)) != enclosing
                        && owner.delegated_cleanup(ops, b.ops.start).is_none()
                })
            }) {
                return None;
            }
            if !flow.blocks_in(at..ret + 1).all(|b| {
                b.ops.start == at
                    || flow.entered_from(b.ops.start, at..ret + 1)
                    || owner.delegation_rejoins_loop(ops, b.ops.start, at..ret + 1)
            }) {
                return None;
            }
            exits.push(ReturnExit {
                start: at,
                ret,
                pending: true,
            });
            at = ret + 1;
        }
    }
    if exits.is_empty() {
        return None;
    }
    Some(Protected {
        return_exits: exits,
        end,
        next: end,
        clause: Clause::Finally {
            deferred_return: end..end,
            terminal_unwind: split.then_some(ops[cleanup].offset),
        },
        handler,
        try_exits: None,
        handler_exits: None,
        handler_tail: None,
    })
}

/// Match the normal copy to the exceptional one, up to its final reraise.
pub(super) fn finally_region(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    suite: Range<usize>,
    dispatch: usize,
    cleanup: usize,
) -> Option<Protected> {
    let flow = &owner.flow;
    let reraise = cleanup.checked_sub(1)?;
    let exceptional = dispatch + 1..reraise;
    if exceptional.is_empty() || (ops[reraise].name, ops[reraise].arg) != ("RERAISE", 0) {
        return None;
    }
    let terminal = ops[exceptional.clone()]
        .iter()
        .any(|op| (op.name, op.arg) == ("RERAISE", 0));
    let copy = if terminal {
        dispatch + 1..cleanup
    } else {
        exceptional
    };
    let end = suite.end;
    // If every normal path raises, CPython emits only the exceptional copy.
    if ops[end - 1].name == "RAISE_VARARGS"
        && flow.blocks_in(suite.clone()).all(|block| {
            let last = block.ops.end - 1;
            let jump = flow.jump(last);
            let fallthrough = flow.fallthrough(block);
            [jump, fallthrough]
                .into_iter()
                .flatten()
                .all(|dest| suite.contains(&dest))
                && (jump.is_some() || fallthrough.is_some() || ops[last].name == "RAISE_VARARGS")
        })
    {
        return Some(Protected {
            return_exits: Vec::new(),
            end,
            next: end,
            clause: Clause::Finally {
                deferred_return: end..end,
                terminal_unwind: terminal.then_some(ops[cleanup].offset),
            },
            handler: copy,
            try_exits: None,
            handler_exits: None,
            handler_tail: None,
        });
    }
    let marker = ops[end].name == "NOP";
    let deferred = marker && !break_marker(ops, flow, suite);
    let start = end + usize::from(marker);
    let exit = if terminal {
        CopyExit::ReturnNone
    } else {
        CopyExit::Fallthrough
    };
    let normal = start..owner.finally_copy_end(ops, start..dispatch, copy.clone(), exit)?;
    // A return of a constant can leave a tracing NOP at its original site,
    // with the constant materialized only after the normal cleanup.
    let mut next = normal.end;
    if deferred && !terminal {
        next += match ops.get(next..)? {
            [ret, ..] if ret.name == "RETURN_CONST" => 1,
            [load, ret, ..]
                if matches!(
                    load.name,
                    "LOAD_CONST" | "LOAD_COMMON_CONSTANT" | "LOAD_SMALL_INT"
                ) && ret.name == "RETURN_VALUE" =>
            {
                2
            }
            _ => return None,
        };
    }
    Some(Protected {
        return_exits: Vec::new(),
        end,
        next,
        clause: Clause::Finally {
            deferred_return: normal.end..next,
            terminal_unwind: terminal.then_some(ops[cleanup].offset),
        },
        handler: if terminal { copy } else { normal },
        try_exits: None,
        handler_exits: None,
        handler_tail: None,
    })
}

/// A break can jump to a tracing NOP before the finalizer. Its operand
/// stack is empty there and execution continues after cleanup.
pub(super) fn break_marker(ops: &[Instruction], flow: &Graph, suite: Range<usize>) -> bool {
    ops[suite.end].name == "NOP"
        && flow.incoming(suite.end).any(|i| {
            suite.contains(&i) && ops[i].name == "JUMP_FORWARD" && flow.jump(i) == Some(suite.end)
        })
}
