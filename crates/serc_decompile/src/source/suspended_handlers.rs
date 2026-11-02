//! User exception handlers in suspended scopes. Exception targets are out of
//! line, even when the corresponding try is inside a loop or another handler.
use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};
use serc_bytecode::instructions::Handler;

use super::{Body, Bytecode, Instruction};
use crate::flow::Graph;

pub(super) type Handlers = FxHashMap<usize, Except>;

pub(super) struct Except {
    end: usize,
    next: usize,
    kind: Option<Range<usize>>,
    handler: Range<usize>,
}

pub(super) fn regions(bc: &Bytecode, flow: &Graph) -> Handlers {
    let mut groups = FxHashMap::<_, Vec<_>>::default();
    for entry in &bc.handlers {
        if !entry.lasti {
            groups.entry(entry.target).or_default().push(*entry);
        }
    }
    groups
        .into_values()
        .filter_map(|entries| region(&bc.ops, flow, &entries))
        .collect()
}

fn region(ops: &[Instruction], flow: &Graph, entries: &[Handler]) -> Option<(usize, Except)> {
    let index = |offset| ops.binary_search_by_key(&offset, |op| op.offset).ok();
    let first = entries[0];
    let start = index(first.start)?;
    let dispatch = index(first.target)?;
    if ops[dispatch].name != "PUSH_EXC_INFO" || start >= dispatch {
        return None;
    }

    let end = protected_end(ops, flow, entries)?;
    if end > dispatch {
        return None;
    }
    let unwind = flow.handler(dispatch)?;
    let cleanup = index(unwind.target)?;
    if !unwind.lasti
        || unwind.depth != first.depth + 1
        || !matches!(ops.get(cleanup..cleanup + 3)?, [copy, pop, raise]
            if (copy.name, copy.arg) == ("COPY", 3) && pop.name == "POP_EXCEPT" && (raise.name, raise.arg) == ("RERAISE", 1))
    {
        return None;
    }

    let (kind, body, stop) = if ops.get(dispatch + 1)?.name == "POP_TOP" {
        (None, dispatch + 2, cleanup)
    } else {
        let jump = flow.block(dispatch + 1).ops.end.checked_sub(1)?;
        let check = jump.checked_sub(1)?;
        let failed = flow.jump(jump)?;
        let pop = jump + 1 + usize::from(ops.get(jump + 1)?.name == "NOT_TAKEN");
        if check <= dispatch + 1
            || ops[check].name != "CHECK_EXC_MATCH"
            || ops[jump].name != "POP_JUMP_IF_FALSE"
            || ops.get(pop)?.name != "POP_TOP"
            || failed + 1 != cleanup
            || (ops[failed].name, ops[failed].arg) != ("RERAISE", 0)
        {
            return None;
        }
        (Some(dispatch + 1..check), pop + 1, failed)
    };
    // Inline handlers have a jump on the normal path that skips exactly their
    // dispatch and unwind. Out-of-line handlers rejoin the normal continuation.
    let next = if end + 1 == dispatch
        && ops[end].name == "JUMP_FORWARD"
        && flow.jump(end) == Some(cleanup + 3)
    {
        cleanup + 3
    } else {
        end
    };
    let handler = match ops.get(body..stop)? {
        [.., raise] if raise.name == "RAISE_VARARGS" => body..stop,
        [.., pop, jump]
            if pop.name == "POP_EXCEPT"
                && matches!(
                    jump.name,
                    "JUMP_FORWARD" | "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
                )
                && flow.jump(stop - 1) == Some(next) =>
        {
            body..stop - 2
        }
        _ => return None,
    };
    Some((
        start,
        Except {
            end,
            next,
            kind,
            handler,
        },
    ))
}

fn protected_end(ops: &[Instruction], flow: &Graph, entries: &[Handler]) -> Option<usize> {
    let index = |offset| ops.binary_search_by_key(&offset, |op| op.offset).ok();
    let first = entries[0];
    // A delegated yield temporarily routes throws to CLEANUP_THROW. On success
    // it rejoins END_SEND. If it throws again, the user's handler gets control.
    let mut end = index(first.end)?;
    let mut cleanups = FxHashSet::default();
    for entry in &entries[1..] {
        let at = index(entry.start)?;
        if (entry.depth, entry.lasti) != (first.depth, first.lasti) {
            return None;
        }
        if ops[at].name == "CLEANUP_THROW" {
            if index(entry.end)? != at + 1 || !cleanups.remove(&at) {
                return None;
            }
            continue;
        }
        let throw = flow.handler(end)?;
        let cleanup = index(throw.target)?;
        if at != end + 1
            || ops[end].name != "YIELD_VALUE"
            || ops[at].name != "RESUME"
            || ops[cleanup].name != "CLEANUP_THROW"
            || throw.start != ops[end].offset
            || throw.end != entry.start
            || throw.lasti
        {
            return None;
        }
        end = index(entry.end)?;
        let resume = flow.jump(cleanup + 1)?;
        if !(at..end).contains(&resume) || ops[resume].name != "END_SEND" {
            return None;
        }
        cleanups.insert(cleanup);
    }
    cleanups.is_empty().then_some(end)
}

impl Body<'_, '_, '_> {
    pub(super) fn suspended_handler(
        &mut self,
        ops: &[Instruction],
        start: usize,
        limit: usize,
        level: usize,
        entry: &Except,
    ) -> Result<Option<usize>, String> {
        if entry.next > limit || !self.stack.is_empty() {
            return Ok(None);
        }
        let (suite, returns) = self
            .fragment(ops, start, entry.end, level)?
            .pending_return()?;
        let mut next = entry.next;
        if returns {
            if next != entry.end || next >= limit || ops[next].name != "RETURN_VALUE" {
                return Err("unsupported return through suspended handler".into());
            }
            next += 1;
        }
        let header = if let Some(kind) = &entry.kind {
            let expr = self
                .fragment(ops, kind.start, kind.end, level)?
                .expression()?;
            format!("except {expr}:")
        } else {
            "except:".into()
        };
        let handler = self
            .fragment(ops, entry.handler.start, entry.handler.end, level)?
            .suite()?;
        self.try_block(&suite, start > 0 && ops[start - 1].name != "NOP")?;
        self.block(&header, &handler)?;
        Ok(Some(next))
    }
}
