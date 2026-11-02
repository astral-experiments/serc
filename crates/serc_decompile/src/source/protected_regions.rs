//! Recover protected suites at their instruction boundaries, including tries
//! inside loops and conditional arms, and suspended suites split across a yield.
use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};
use serc_bytecode::instructions::Handler;

use super::{Body, Bytecode, Instruction};
use crate::flow::Graph;

pub(super) type Handlers = FxHashMap<usize, Protected>;

enum Clause {
    Except(Option<Range<usize>>),
    Finally { deferred_return: Range<usize> },
}

pub(super) struct Protected {
    end: usize,
    next: usize,
    clause: Clause,
    handler: Range<usize>,
}

fn groups(bc: &Bytecode) -> FxHashMap<usize, Vec<Handler>> {
    let mut groups = FxHashMap::<_, Vec<_>>::default();
    for entry in &bc.handlers {
        if !entry.lasti {
            groups.entry(entry.target).or_default().push(*entry);
        }
    }
    groups
}

pub(super) fn suspended(bc: &Bytecode, flow: &Graph) -> Handlers {
    groups(bc)
        .into_values()
        .filter_map(|entries| region(&bc.ops, flow, &entries))
        .collect()
}

/// Use region recovery for ordinary scopes when a branch encloses the first
/// try, or there are multiple protected entries. Every user handler must have
/// a known placement before the normal traversal can skip its out-of-line code.
pub(super) fn guarded(bc: &Bytecode, flow: &Graph) -> Option<Handlers> {
    let first = bc.handlers.first()?;
    if first.lasti {
        return None;
    }
    let groups = groups(bc);
    let split = groups.get(&first.target)?.len() > 1;
    if groups.len() == 1
        && !split
        && !bc
            .ops
            .iter()
            .take_while(|op| op.offset < first.start)
            .any(|op| op.jump.is_some_and(|target| target > first.start))
    {
        return None;
    }
    let mut result = Handlers::default();
    let mut unwinds = FxHashSet::default();
    for entries in groups.into_values() {
        let dispatch = bc
            .ops
            .binary_search_by_key(&entries[0].target, |op| op.offset)
            .ok()?;
        let h = flow.handler(dispatch)?;
        let (start, suite) = region(&bc.ops, flow, &entries)?;
        unwinds.insert((h.target, h.depth));
        result.insert(start, suite);
    }
    bc.handlers
        .iter()
        .all(|h| !h.lasti || unwinds.contains(&(h.target, h.depth)))
        .then_some(result)
}

fn region(ops: &[Instruction], flow: &Graph, entries: &[Handler]) -> Option<(usize, Protected)> {
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
    let (unwind, cleanup) = super::dispatch::unwind(ops, flow, dispatch, first.depth + 1)?;
    if unwind.start == ops[dispatch].offset
        && unwind.end == ops[cleanup].offset
        && let Some(finally) = finally_region(ops, flow, end, dispatch, cleanup)
    {
        return Some((start, finally));
    }
    let (kind, body, stop) = if ops.get(dispatch + 1)?.name == "POP_TOP" {
        (None, dispatch + 2, cleanup)
    } else {
        let (check, pop, failed) = super::dispatch::typed_handler(flow, ops, dispatch + 1)?;
        if ops[pop].name != "POP_TOP"
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
        _ => {
            // A handler can fall through to the same constant return as the
            // successful try. CPython copies it instead of jumping backwards.
            let (load, value) = super::regions::constant_return(ops, stop)?;
            let width = stop - load;
            if load <= body
                || ops[load - 1].name != "POP_EXCEPT"
                || super::regions::constant_return(ops, next + width) != Some((next, value))
            {
                return None;
            }
            body..load - 1
        }
    };
    Some((
        start,
        Protected {
            end,
            next,
            clause: Clause::Except(kind),
            handler,
        },
    ))
}

/// Match the normal copy to the exceptional one, up to its final reraise.
/// CPython can borrow a local on the normal path but load it owned on unwind.
fn finally_region(
    ops: &[Instruction],
    flow: &Graph,
    end: usize,
    dispatch: usize,
    cleanup: usize,
) -> Option<Protected> {
    let reraise = cleanup.checked_sub(1)?;
    let exceptional = dispatch + 1..reraise;
    if exceptional.is_empty() || (ops[reraise].name, ops[reraise].arg) != ("RERAISE", 0) {
        return None;
    }
    let deferred = ops[end].name == "NOP";
    let start = end + usize::from(deferred);
    let normal = start..start + exceptional.len();
    if normal.end > dispatch
        || !normal.clone().zip(exceptional.clone()).all(|(a, b)| {
            let load = |op: Instruction| match op.name {
                "LOAD_FAST_BORROW" => "LOAD_FAST",
                "LOAD_FAST_BORROW_LOAD_FAST_BORROW" => "LOAD_FAST_LOAD_FAST",
                name => name,
            };
            if load(ops[a]) != load(ops[b]) {
                return false;
            }
            if ops[a].jump.is_some() {
                // Every copied jump must reach the same point within its copy,
                // including the fallthrough immediately following the cleanup.
                matches!((flow.jump(a), flow.jump(b)), (Some(x), Some(y))
                if (normal.start..=normal.end).contains(&x)
                && (exceptional.start..=exceptional.end).contains(&y)
                && x - normal.start == y - exceptional.start)
            } else {
                ops[a].arg == ops[b].arg
            }
        })
    {
        return None;
    }
    // A return of a constant can leave a tracing NOP at its original site,
    // with the constant materialized only after the normal cleanup.
    let mut next = normal.end;
    if deferred {
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
        end,
        next,
        clause: Clause::Finally {
            deferred_return: normal.end..next,
        },
        handler: normal,
    })
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
        // A monitoring marker or a loop latch can sit outside the exception
        // table between two parts of the same try. For the latch, require the
        // preceding test to exit to the resumed span and the backedge to land
        // inside this protection.
        if at == end + 1
            && end > 0
            && flow.handler(end).is_none()
            && ops[end - 1].name.starts_with("POP_JUMP_IF_")
            && (ops[end].name == "NOT_TAKEN"
                || (ops[end].name == "JUMP_BACKWARD"
                    && flow.jump(end - 1) == Some(at)
                    && flow
                        .jump(end)
                        .and_then(|i| flow.handler(i))
                        .is_some_and(|h| {
                            h.target == first.target && h.depth == first.depth && !h.lasti
                        })))
        {
            end = index(entry.end)?;
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
    pub(super) fn protected_region(
        &mut self,
        ops: &[Instruction],
        start: usize,
        limit: usize,
        level: usize,
        entry: &Protected,
    ) -> Result<Option<usize>, String> {
        if entry.next > limit || !self.stack.is_empty() {
            return Ok(None);
        }
        let (mut suite, returns) = self
            .fragment(ops, start, entry.end, level)?
            .pending_return()?;
        let mut next = entry.next;
        if returns {
            if next >= limit || ops[next].name != "RETURN_VALUE" {
                return Err("unsupported return through suspended handler".into());
            }
            next += 1;
        }
        let header = match &entry.clause {
            Clause::Except(Some(kind)) => {
                let expr = self
                    .fragment(ops, kind.start, kind.end, level)?
                    .expression()?;
                format!("except {expr}:")
            }
            Clause::Except(None) => "except:".into(),
            Clause::Finally { deferred_return } => {
                if !deferred_return.is_empty() {
                    suite.push_str(
                        &self
                            .fragment(ops, deferred_return.start, deferred_return.end, level)?
                            .suite()?,
                    );
                }
                "finally:".into()
            }
        };
        let handler = self
            .fragment(ops, entry.handler.start, entry.handler.end, level)?
            .suite()?;
        self.try_block(&suite, start > 0 && ops[start - 1].name != "NOP")?;
        self.block(&header, &handler)?;
        Ok(Some(next))
    }
}
