//! Recover protected suites at their instruction boundaries, including tries
//! inside loops and conditional arms, and suspended suites split across a yield.
use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};
use serc_bytecode::instructions::Handler;

use super::{Body, Bytecode, Instruction};
use crate::flow::Graph;

pub(super) type Handlers = FxHashMap<usize, Protected>;

enum Clause {
    Except {
        kind: Option<Range<usize>>,
        binding: usize,
        success: Range<usize>,
    },
    Finally {
        deferred_return: Range<usize>,
        terminal_unwind: Option<usize>,
    },
}

pub(super) struct Protected {
    end: usize,
    next: usize,
    clause: Clause,
    handler: Range<usize>,
    try_exits: Option<ExitCopies>,
    handler_exits: Option<ExitCopies>,
    // Resume after POP_EXCEPT for a loop transfer. A computed return keeps
    // its value across the pop and the following iterator cleanup.
    handler_tail: Option<Range<usize>>,
}

/// Consecutive copies of the same straight-line clause exit. A predicate can
/// enter any copy at its first instruction, including past the lifted suite.
#[derive(Clone, Copy)]
pub(super) struct ExitCopies {
    first: usize,
    end: usize,
    width: usize,
}

impl ExitCopies {
    fn suffix(ops: &[Instruction], flow: &Graph, body: Range<usize>, width: usize) -> Option<Self> {
        let mut first = body.end.checked_sub(width)?;
        if first < body.start {
            return None;
        }
        let exit = &ops[first..body.end];
        while first - body.start >= width
            && super::comparisons::same_instructions(&ops[first - width..first], exit)
        {
            first -= width;
        }
        // Each additional copy is an unprotected exit entered from this
        // clause. A jump from another suite still needs its own destination.
        (body.end - first > width
            && flow
                .blocks_covering(first..body.end)
                .all(|b| flow.handler(b.ops.start).is_none())
            && (first + width..body.end).step_by(width).all(|at| {
                flow.block(at).ops.start == at && flow.entered_from(at, body.start..first)
            }))
        .then_some(Self {
            first,
            end: body.end,
            width,
        })
    }

    pub(super) fn destination(self, target: usize) -> usize {
        if (self.first..self.end).contains(&target)
            && (target - self.first).is_multiple_of(self.width)
        {
            self.first
        } else {
            target
        }
    }
}

fn groups(bc: &Bytecode) -> Vec<Vec<Handler>> {
    let mut groups = FxHashMap::<_, Vec<_>>::default();
    for entry in &bc.handlers {
        if !entry.lasti {
            groups.entry(entry.target).or_default().push(*entry);
        }
    }
    // Recover inner suites before resolving their enclosing unwind edges.
    let mut ordered: Vec<_> = groups.into_values().collect();
    ordered.sort_unstable_by_key(|entries| std::cmp::Reverse(entries[0].start));
    ordered
}

pub(super) fn suspended(bc: &Bytecode, owner: &Body<'_, '_, '_>) -> Handlers {
    let mut result = Handlers::default();
    for entries in groups(bc) {
        if let Some((start, suite)) = region(&bc.ops, owner, &entries, &result) {
            result.insert(start, suite);
        }
    }
    result
}

/// If a coroutine only raises, there is no return instruction to delimit
/// its body. Include continuations reachable solely through an except clause.
pub(super) fn returnless_end(bc: &Bytecode, flow: &Graph, handlers: &Handlers) -> Option<usize> {
    let mut starts = vec![0];
    for group in groups(bc) {
        let entry = group.first()?;
        let start = bc
            .ops
            .binary_search_by_key(&entry.start, |op| op.offset)
            .ok()?;
        let suite = handlers.get(&start)?;
        if bc.ops.get(suite.end)?.offset == entry.target && suite.next > suite.end {
            bc.ops.get(suite.next)?;
            starts.push(suite.next);
        }
    }
    Some(flow.normal_end(starts))
}

/// Use region recovery for ordinary scopes when a branch encloses the first
/// try, or there are multiple protected entries. Every user handler must have
/// a known placement before the normal traversal can skip its out-of-line code.
pub(super) fn guarded(bc: &Bytecode, owner: &Body<'_, '_, '_>) -> Option<(Handlers, usize)> {
    let flow = &owner.flow;
    let first = bc.handlers.first()?;
    if first.lasti {
        return None;
    }
    let groups = groups(bc);
    let split = groups.last()?.len() > 1;
    let single_dispatch = groups.len() == 1;
    let unguarded = single_dispatch
        && !split
        && !bc
            .ops
            .iter()
            .take_while(|op| op.offset < first.start)
            .any(|op| op.jump.is_some_and(|target| target > first.start));
    let mut result = Handlers::default();
    let mut unwinds = FxHashSet::default();
    let mut starts = vec![0];
    for entries in groups {
        let dispatch = bc
            .ops
            .binary_search_by_key(&entries[0].target, |op| op.offset)
            .ok()?;
        let h = flow.handler(dispatch)?;
        let (start, mut suite) = region(&bc.ops, owner, &entries, &result)?;
        if suite.end == dispatch && suite.next > dispatch {
            // The try returned. Only the handled exception reaches this
            // continuation, so an ordinary walk from offset zero misses it.
            starts.push(suite.next);
        } else if single_dispatch
            && first.depth == 0
            && suite.next == suite.end
            && let Some((last, _)) = super::regions::constant_return(&bc.ops, dispatch)
            && owner.none_return(&bc.ops[last..dispatch])
            && let Some(copies) =
                ExitCopies::suffix(&bc.ops, flow, start..dispatch, dispatch - last)
            && copies.first == suite.end
            && flow.unprotected_suite(suite.end..dispatch)
            && flow.normal_end([0]) == dispatch
        {
            // The final guards have copies of the implicit return outside
            // the protected suite. Its successful paths both leave the scope.
            suite.try_exits = Some(copies);
            suite.next = dispatch;
        } else if unguarded && suite.handler_exits.is_none() {
            // The linear handler walk ends at the first POP_EXCEPT. A
            // computed return inside a conditional leaves another live arm.
            let guarded_return = bc.ops[suite.handler.clone()]
                .windows(2)
                .any(|ops| (ops[0].name, ops[0].arg) == ("SWAP", 2) && ops[1].name == "POP_EXCEPT");
            let exceptional_finally =
                matches!(suite.clause, Clause::Finally { .. }) && suite.handler.start > dispatch;
            if !guarded_return && !exceptional_finally {
                return None;
            }
        }
        if let Clause::Except { binding, .. } = suite.clause
            && bc.ops[binding].name != "POP_TOP"
        {
            let name_cleanup = flow.handler(binding + 1)?;
            unwinds.insert((name_cleanup.target, name_cleanup.depth));
        }
        unwinds.insert((h.target, h.depth));
        result.insert(start, suite);
    }
    bc.handlers
        .iter()
        .all(|h| !h.lasti || unwinds.contains(&(h.target, h.depth)))
        .then(|| (result, flow.normal_end(starts)))
}

fn region(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    entries: &[Handler],
    inner: &Handlers,
) -> Option<(usize, Protected)> {
    let flow = &owner.flow;
    let index = |offset| ops.binary_search_by_key(&offset, |op| op.offset).ok();
    let first = entries[0];
    let start = index(first.start)?;
    let dispatch = index(first.target)?;
    if ops[dispatch].name != "PUSH_EXC_INFO" || start >= dispatch {
        return None;
    }

    let end = protected_end(ops, flow, entries, inner)?;
    if end > dispatch {
        return None;
    }
    let (unwind, cleanup) = super::dispatch::unwind(ops, flow, dispatch, first.depth + 1)?;
    if unwind.start == ops[dispatch].offset
        && super::finalizers::owns_cleanup(ops, flow, dispatch..cleanup, unwind)
        && ops[dispatch + 1].name != "POP_TOP"
        && ops[dispatch + 1].name != "BUILD_LIST"
        && super::dispatch::typed_handler(flow, ops, dispatch + 1).is_none()
        && let Some(finally) = finally_region(ops, owner, start..end, dispatch, cleanup)
    {
        return Some((start, finally));
    }
    let (kind, binding, stop) = if ops.get(dispatch + 1)?.name == "POP_TOP" {
        (None, dispatch + 1, cleanup)
    } else {
        let (check, pop, failed) = super::dispatch::typed_handler(flow, ops, dispatch + 1)?;
        if failed + 1 != cleanup || (ops[failed].name, ops[failed].arg) != ("RERAISE", 0) {
            return None;
        }
        let stop = if ops[pop].name == "POP_TOP" {
            failed
        } else {
            owner.exception_name_cleanup(ops, pop, failed)?
        };
        (Some(dispatch + 1..check), pop, stop)
    };
    let body = binding + 1;
    let bound = ops[binding].name != "POP_TOP";
    let end = guarded_return_end(ops, flow, start..end, dispatch, first.depth).unwrap_or(end);
    // An inline handler rejoins just past its unwind. When the try returns,
    // that edge comes from POP_EXCEPT in the handler. Include RETURN_VALUE
    // in the try suite before resuming at the jump target.
    let after_cleanup = cleanup + 3;
    let (end, mut next) = match ops.get(end..dispatch)? {
        [jump] if jump.name == "JUMP_FORWARD" && flow.jump(end) == Some(after_cleanup) => {
            (end, after_cleanup)
        }
        [ret]
            if ret.name == "RETURN_VALUE"
                && matches!(ops.get(body..stop)?, [.., pop, jump]
                if pop.name == "POP_EXCEPT" && jump.name == "JUMP_FORWARD")
                && after_cleanup < ops.len()
                && flow.jump(stop - 1) == Some(after_cleanup) =>
        {
            (dispatch, after_cleanup)
        }
        _ => (end, end),
    };
    // At the tail of a for loop, a handler can rejoin the iterator directly.
    // The normal path either has a latch or always raises before END_FOR.
    let for_header = match ops[end].name {
        "END_FOR" => flow.incoming(end).find(|&i| ops[i].name == "FOR_ITER"),
        "JUMP_BACKWARD" => flow.jump(end).filter(|&i| ops[i].name == "FOR_ITER"),
        _ => None,
    }
    .filter(|&i| i < start && flow.for_iter_depth(i) == Some(first.depth));
    let mut success = next..next;
    let mut handler_tail = None;
    let mut handler_exits = None;
    let handler = match ops.get(body..stop)? {
        [.., raise] if raise.name == "RAISE_VARARGS" => body..stop,
        [.., jump]
            if matches!(
                jump.name,
                "JUMP_FORWARD" | "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
            ) && let Some((pop, after)) = owner.handler_pop(ops, binding, stop)
                && after == stop - 1
                && let Some(to) = flow.jump(stop - 1) =>
        {
            if to != next && Some(to) != for_header {
                // The handler skips the rest of this iteration. Only a jump
                // to the enclosing iterator at its saved depth is a continue.
                if ops[to].name != "FOR_ITER"
                    || to >= start
                    || flow.jump(to).is_none_or(|exit| end >= exit)
                    || flow.for_iter_depth(to) != Some(first.depth)
                {
                    return None;
                }
                handler_tail = Some(stop - 1..stop);
            }
            body..pop
        }
        [.., ret]
            if !bound
                && for_header.is_some()
                && matches!(ret.name, "RETURN_VALUE" | "RETURN_CONST")
                && let Some(pop) = (body..stop).rfind(|&i| ops[i].name == "POP_EXCEPT")
                && matches!(ops[pop + 1].name, "SWAP" | "POP_TOP") =>
        {
            let computed = ops[pop + 1].name == "SWAP";
            if computed && (pop == body || (ops[pop - 1].name, ops[pop - 1].arg) != ("SWAP", 2)) {
                return None;
            }
            handler_tail = Some(pop + 1..stop);
            body..pop - usize::from(computed)
        }
        _ => {
            // A handler can fall through to the same constant return as the
            // successful try. CPython copies it instead of jumping backwards.
            let (load, value) = super::regions::constant_return(ops, stop)?;
            let width = stop - load;
            let clear = if bound { 3 } else { 0 };
            let pop = load.checked_sub(clear + 1)?;
            if pop < body
                || ops[pop].name != "POP_EXCEPT"
                || (bound
                    && !super::comparisons::same_instructions(
                        &ops[pop + 1..load],
                        &ops[stop..stop + clear],
                    ))
            {
                return None;
            }
            if super::regions::constant_return(ops, next + width) != Some((next, value)) {
                let normal = flow.block(next);
                let normal_return = super::regions::constant_return(ops, normal.ops.end);
                // A suppressed with exception jumps back into the successful
                // try path. Protection on that jump distinguishes an else
                // suite from a statement after an explicitly returning handler.
                if let Some((ret, normal_value)) = normal_return
                    && normal_value == value
                    && next < ret
                    && normal.ops.end <= dispatch
                    && flow.incoming(next).any(|source| {
                        source > next
                            && flow.jump(source) == Some(next)
                            && flow
                                .handler(source)
                                .is_some_and(|h| h.target == first.target)
                    })
                {
                    success = next..ret;
                    next = ret;
                } else {
                    handler_tail = Some(load..stop);
                }
            }
            handler_exits = ExitCopies::suffix(ops, flow, body..stop, stop - pop);
            body..handler_exits.map_or(pop, |copies| copies.first)
        }
    };
    Some((
        start,
        Protected {
            end,
            next,
            clause: Clause::Except {
                kind,
                binding,
                success,
            },
            try_exits: None,
            handler_exits,
            handler,
            handler_tail,
        },
    ))
}

/// A guarded return evaluates its value under protection but returns it
/// outside the table. In a for loop the same block first drops the iterator.
/// The other arm skips the whole epilogue and continues after the try.
fn guarded_return_end(
    ops: &[Instruction],
    flow: &Graph,
    suite: Range<usize>,
    dispatch: usize,
    depth: usize,
) -> Option<usize> {
    let block = flow.block(suite.end);
    let next = block.ops.end;
    if block.ops.start != suite.end
        || next >= dispatch
        || flow.handler(suite.end).is_some()
        || block.height != Some(depth + 1)
        || !flow
            .incoming(next)
            .any(|i| suite.contains(&i) && ops[i].name.starts_with("POP_JUMP_IF_"))
    {
        return None;
    }
    let (ret, cleanup) = ops[block.ops.clone()].split_last()?;
    if ret.name != "RETURN_VALUE" {
        return None;
    }
    match cleanup {
        [] if depth == 0 => Some(next),
        [swap, pops @ ..]
            if depth > 0
                && swap.name == "SWAP"
                && swap.arg as usize == depth + 1
                && pops.len() == depth
                && pops.iter().all(|op| op.name == "POP_TOP")
                && flow.jump(next).is_some_and(|header| {
                    ops[header].name == "FOR_ITER" && flow.for_iter_depth(header) == Some(depth)
                }) =>
        {
            Some(next)
        }
        _ => None,
    }
}

/// Match the normal copy to the exceptional one, up to its final reraise.
fn finally_region(
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
    let deferred = ops[end].name == "NOP";
    let start = end + usize::from(deferred);
    let normal = start..owner.finally_copy_end(ops, start..dispatch, copy.clone(), terminal)?;
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

fn protected_end(
    ops: &[Instruction],
    flow: &Graph,
    entries: &[Handler],
    inner: &Handlers,
) -> Option<usize> {
    let index = |offset| ops.binary_search_by_key(&offset, |op| op.offset).ok();
    let first = entries[0];
    // A delegated yield temporarily routes throws to CLEANUP_THROW. On success
    // it rejoins END_SEND. If it throws again, the user's handler gets control.
    let mut end = index(first.end)?;
    let mut cleanups = FxHashMap::default();
    for entry in &entries[1..] {
        let at = index(entry.start)?;
        if (entry.depth, entry.lasti) != (first.depth, first.lasti) {
            return None;
        }
        if super::managers::bridges_try(ops, flow, end..at, first, &mut cleanups).is_some() {
            end = index(entry.end)?;
            continue;
        }
        // The enclosing range can stop at a nested try and resume on its
        // exceptional unwind. Only stitch a child whose cleanup the table
        // routes back to this exact handler.
        let mut child = end;
        while child < at && matches!(ops[child].name, "NOP" | "NOT_TAKEN") {
            child += 1;
        }
        if let Some(suite) = inner.get(&child)
            && matches!(suite.clause, Clause::Finally { .. })
            && let Some(h) = flow.handler(child)
            && let Some((_, unwind)) =
                super::dispatch::unwind(ops, flow, index(h.target)?, h.depth + 1)
            && let Some(parent) = flow.handler(unwind)
            && parent.start == ops[unwind].offset
            && (parent.target, parent.depth, parent.lasti)
                == (first.target, first.depth, first.lasti)
            && index(parent.end)? == unwind + 3
            && (at == unwind || at == suite.next)
        {
            cleanups.insert(unwind, unwind + 3);
            end = suite.next;
            if at == end {
                end = index(entry.end)?;
                continue;
            }
        }
        if let Some(expected_end) = cleanups.remove(&at) {
            if index(entry.end)? != expected_end {
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
        cleanups.insert(cleanup, cleanup + 1);
    }
    cleanups.is_empty().then_some(end)
}

impl Body<'_, '_, '_> {
    fn protected_fragment(
        &mut self,
        ops: &[Instruction],
        body: Range<usize>,
        level: usize,
        exits: Option<ExitCopies>,
    ) -> Result<super::regions::Fragment, String> {
        let outer = std::mem::replace(&mut self.clause_exits, exits);
        let fragment = self.fragment(ops, body.start, body.end, level);
        self.clause_exits = outer;
        fragment
    }

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
            .protected_fragment(ops, start..entry.end, level, entry.try_exits)?
            .pending_return()?;
        let mut next = entry.next;
        if returns {
            if next >= limit || ops[next].name != "RETURN_VALUE" {
                return Err("unsupported return through suspended handler".into());
            }
            next += 1;
        }
        let header = match &entry.clause {
            Clause::Except { kind, binding, .. } => {
                self.except_header(ops, kind.clone(), *binding, level)?
            }
            Clause::Finally {
                deferred_return, ..
            } => {
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
        let terminal_unwind = match &entry.clause {
            Clause::Finally {
                terminal_unwind, ..
            } => *terminal_unwind,
            Clause::Except { .. } => None,
        };
        let outer = std::mem::replace(&mut self.finally_unwind, terminal_unwind);
        let handler =
            self.protected_fragment(ops, entry.handler.clone(), level, entry.handler_exits);
        self.finally_unwind = outer;
        let handler = handler?;
        let handler = if let Some(tail) = &entry.handler_tail {
            handler.resume(self, ops, tail.clone(), level)?
        } else {
            handler.suite()?
        };
        self.try_block(&suite, start > 0 && ops[start - 1].name != "NOP")?;
        self.block(&header, &handler)?;
        if let Clause::Except { success, .. } = &entry.clause
            && !success.is_empty()
        {
            let suite = self
                .fragment(ops, success.start, success.end, level)?
                .suite()?;
            self.block("else:", &suite)?;
        }
        if terminal_unwind.is_some() && entry.next > entry.end && (level > 0 || next < limit) {
            self.line("return")?;
        }
        Ok(Some(next))
    }
}
