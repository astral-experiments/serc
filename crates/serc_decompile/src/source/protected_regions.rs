//! Recover protected suites at their instruction boundaries, including tries
//! inside loops and conditional arms, and suspended suites split across a yield.
use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};
use serc_bytecode::instructions::Handler;

use super::{Body, Bytecode, Instruction};
use crate::flow::Graph;
mod bound_parent;
mod finally;
mod groups;
mod handler_rejoin;
mod handler_returns;
mod loop_exits;
mod retry_finally;

pub(super) use bound_parent::BoundParent;
use finally::{break_marker, finally_region, returning_finally_loop};
use groups::{HandlerGroup, groups};
pub(super) use handler_rejoin::HandlerRejoin;
pub(super) use handler_returns::returning_handler_child;
use handler_returns::{enclosing_exception, nested_handler_return};

pub(super) use loop_exits::iterator_return_block;
use loop_exits::{guarded_return_end, iterator_success_latch};

pub(super) type Handlers = FxHashMap<usize, Protected>;

enum Clause {
    Except {
        kind: Option<Range<usize>>,
        binding: usize,
        success: Range<usize>,
        following: Vec<super::dispatch::Arm>,
        finalizer: Option<Range<usize>>,
        retry: bool,
    },
    Finally {
        deferred_return: Range<usize>,
        terminal_unwind: Option<(usize, super::finalizers::CopyExit)>,
    },
}

pub(super) struct Protected {
    return_exits: Vec<super::regions::ReturnExit>,
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

impl Protected {
    /// In a combined try/except/finally the first unwind enters the finalizer.
    /// Its own unwind is the one that propagates to the enclosing suite.
    fn unwind(&self, ops: &[Instruction], flow: &Graph, entry: Handler) -> Option<usize> {
        let index = |offset| ops.binary_search_by_key(&offset, |op| op.offset).ok();
        let (_, unwind) =
            super::dispatch::unwind(ops, flow, index(entry.target)?, entry.depth + 1)?;
        if let Clause::Except {
            finalizer: Some(normal),
            ..
        } = &self.clause
        {
            let outer = flow.handler(unwind)?;
            if outer.lasti
                || outer.depth != entry.depth
                || outer.start != ops[unwind].offset
                || outer.end != ops.get(unwind + 3)?.offset
                || normal.end != self.next
            {
                return None;
            }
            let (_, last) =
                super::dispatch::unwind(ops, flow, index(outer.target)?, outer.depth + 1)?;
            Some(last)
        } else {
            Some(unwind)
        }
    }

    fn normal_finalizer(&self) -> Option<&Range<usize>> {
        match &self.clause {
            Clause::Finally { .. } => Some(&self.handler),
            Clause::Except { finalizer, .. } => finalizer.as_ref(),
        }
    }
}

/// Consecutive copies of the same straight-line clause exit. A predicate can
/// enter any copy at its first instruction, including past the lifted suite.
#[derive(Clone, Copy)]
pub(super) struct ExitCopies {
    pub(super) first: usize,
    end: usize,
    width: usize,
}

impl ExitCopies {
    pub(super) fn suffix(
        ops: &[Instruction],
        flow: &Graph,
        body: Range<usize>,
        width: usize,
    ) -> Option<Self> {
        let mut first = body.end.checked_sub(width)?;
        if width == 0 || first < body.start {
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

    pub(super) fn handler(
        ops: &[Instruction],
        flow: &Graph,
        body: Range<usize>,
        end: usize,
    ) -> Option<Self> {
        let width = end.checked_sub(body.end)?;
        Self::suffix(ops, flow, body.start..end, width).or_else(|| {
            let block = flow.block(body.end);
            (ops[body.end].name == "POP_EXCEPT"
                && block.ops == (body.end..end)
                && block.height == Some(1)
                && flow.handler(body.end).is_none())
            .then_some(Self {
                first: body.end,
                end,
                width,
            })
        })
    }

    /// The first arm of a terminal if/else can end with a separate copy of
    /// the final handler cleanup, with the second arm's statements in between.
    pub(super) fn before_arm(
        self,
        ops: &[Instruction],
        flow: &Graph,
        target: usize,
        end: usize,
    ) -> Option<usize> {
        let copy = target.checked_sub(self.width)?;
        (end == self.first
            && flow.block(copy).ops == (copy..target)
            && flow.handler(copy).is_none()
            && flow.block(copy).height == Some(1)
            && ops[copy].name == "POP_EXCEPT"
            && super::comparisons::same_instructions(
                &ops[copy..target],
                &ops[end..end + self.width],
            ))
        .then_some(copy)
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

pub(super) fn collect(bc: &Bytecode, owner: &Body<'_, '_, '_>) -> Handlers {
    let mut result = Handlers::default();
    for group in groups(bc, &owner.flow) {
        if let Some((start, suite)) = region(&bc.ops, owner, &group, &mut result) {
            result.insert(start, suite);
        }
    }
    result
}

/// Include normal continuations reachable solely through an except clause.
/// A coroutine can also lack a return instruction to delimit its body.
pub(super) fn handled_end(bc: &Bytecode, owner: &Body<'_, '_, '_>) -> Option<usize> {
    let mut starts = vec![0];
    for group in groups(bc, &owner.flow) {
        let entry = group.entries.first()?;
        let start = bc
            .ops
            .binary_search_by_key(&entry.start, |op| op.offset)
            .ok()?;
        // An await that never reaches a return still has its delegated
        // throw handler. The normal SEND edge already visits its resumption.
        if group.entries.len() == 1 && owner.delegated_cleanup(&bc.ops, start).is_some() {
            continue;
        }
        let suite = owner.handlers.get(&start).or_else(|| {
            let child = group.child?;
            if group.entries.len() == 1 {
                owner.handlers.get(&child).filter(|p| {
                    matches!(
                        &p.clause,
                        Clause::Except {
                            finalizer: Some(_),
                            ..
                        }
                    )
                })
            } else {
                owner.handlers.get(&(child - 1))
            }
        })?;
        if bc.ops.get(suite.end)?.offset == entry.target && suite.next > suite.end {
            bc.ops.get(suite.next)?;
            starts.push(suite.next);
        }
    }
    Some(owner.flow.normal_end(starts))
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
    let groups = groups(bc, flow);
    let split = groups.last()?.entries.len() > 1;
    let single_dispatch = groups.len() == 1;
    let unguarded = single_dispatch
        && !split
        && !bc
            .ops
            .iter()
            .take_while(|op| op.offset < first.start)
            .any(|op| op.jump.is_some_and(|target| target > first.start))
        && !bc
            .ops
            .iter()
            .take_while(|op| op.offset < first.target)
            .any(|op| {
                op.offset >= first.end
                    && op.name == "JUMP_BACKWARD"
                    && op.jump.is_some_and(|target| target < first.start)
            });
    let mut result = Handlers::default();
    let mut unwinds = FxHashSet::default();
    let mut starts = vec![0];
    for group in groups {
        let dispatch = bc
            .ops
            .binary_search_by_key(&group.entries[0].target, |op| op.offset)
            .ok()?;
        let h = flow.handler(dispatch)?;
        let (start, mut suite) = region(&bc.ops, owner, &group, &mut result)?;
        // A standalone try may have a continuation copied into its terminal
        // arms. Let the complete dispatch compare the copies before placing it.
        if unguarded
            && matches!(&suite.clause, Clause::Except { following, .. }
                if following.iter().any(super::dispatch::Arm::returns))
        {
            return None;
        }
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
            let exceptional_finally = matches!(suite.clause, Clause::Finally { .. })
                && (suite.handler.start > dispatch
                    || break_marker(&bc.ops, flow, start..suite.end));
            let multiple =
                matches!(&suite.clause, Clause::Except { following, .. } if !following.is_empty());
            if !guarded_return && !exceptional_finally && !multiple {
                return None;
            }
        }
        if let Clause::Except {
            binding, following, ..
        } = &suite.clause
        {
            for bind in std::iter::once(*binding).chain(following.iter().map(|arm| arm.binding)) {
                if bc.ops[bind].name != "POP_TOP" {
                    let cleanup = owner.binding_unwind(&bc.ops, bind)?;
                    unwinds.insert((cleanup.target, cleanup.depth));
                }
            }
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
    group: &HandlerGroup,
    inner: &mut Handlers,
) -> Option<(usize, Protected)> {
    let entries = &group.entries;
    let flow = &owner.flow;
    let index = |offset| ops.binary_search_by_key(&offset, |op| op.offset).ok();
    let first = entries[0];
    let start = index(first.start)?;
    let dispatch = index(first.target)?;
    if ops[dispatch].name != "PUSH_EXC_INFO" || start >= dispatch {
        return None;
    }

    let (unwind, cleanup) = super::dispatch::unwind(ops, flow, dispatch, first.depth + 1)?;
    let finalizer = unwind.start == ops[dispatch].offset
        && ops[dispatch + 1].name != "POP_TOP"
        && ops[dispatch + 1].name != "BUILD_LIST"
        && super::dispatch::typed_handler(flow, ops, dispatch + 1).is_none()
        && super::finalizers::owns_cleanup(ops, owner, dispatch..cleanup, unwind);
    if finalizer
        && group.entries.len() == 1
        && let Some(child) = group.child
        && let Some(suite) = inner.get(&child)
        && let Some(normal) = finally::after_except(ops, owner, suite, child, dispatch, cleanup)
    {
        let mut suite = inner.remove(&child)?;
        if let Clause::Except { finalizer, .. } = &mut suite.clause {
            suite.next = normal.end;
            *finalizer = Some(normal);
        }
        return Some((child, suite));
    }
    if finalizer
        && let Some(region) = returning_finally_loop(ops, owner, entries, start, dispatch, cleanup)
    {
        return Some((start, region));
    }
    let (start, end) = if let Some(child) = group.child
        && let Some(suite) = inner.get(&child)
        && groups::encloses_finalizer(ops, flow, entries, start, child, suite)
    {
        // The same outer try catches a raise from the inner suite and either
        // copy of its finally. Nest at the child's tracing NOP so the inner
        // suite keeps its own placement at the first protected instruction.
        (child - 1, suite.next)
    } else {
        (start, protected_end(ops, flow, entries, inner)?)
    };
    if end > dispatch {
        return None;
    }
    let placed = finalizer
        .then(|| finally_region(ops, owner, start..end, dispatch, cleanup))
        .flatten()
        .or_else(|| retry_finally::recover(ops, owner, start..end, dispatch, cleanup, inner));
    if let Some(mut finally) = placed {
        finally::nested_return(ops, flow, start..end, &mut finally, inner);
        return Some((start, finally));
    }
    let mut following = Vec::new();
    let mut following_join = None;
    let (kind, binding, stop) = if ops.get(dispatch + 1)?.name == "POP_TOP" {
        (None, dispatch + 1, cleanup)
    } else {
        let (check, pop, failed) = super::dispatch::typed_handler(flow, ops, dispatch + 1)?;
        if failed + 1 != cleanup || (ops[failed].name, ops[failed].arg) != ("RERAISE", 0) {
            // Any later matches that fall through must rejoin the same
            // iterator as the first arm. Check that after placing its else.
            (following, following_join) = owner.following_arms(ops, failed, cleanup, inner)?;
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
    // The iterator's successful cleanup can leave a tracing NOP between the
    // end of protection and the join shared by the break and the handler.
    let end = if ops[end].name == "NOP"
        && flow.jump(stop - 1) == Some(end + 1)
        && flow.incoming(end + 1).any(|i| {
            (start..end).contains(&i)
                && loop_exits::break_over_cleanup(ops, flow, i, first) == Some(end + 1)
        }) {
        end + 1
    } else {
        end
    };
    // An inline handler rejoins just past its unwind. When the try returns,
    // that edge comes from POP_EXCEPT in the handler. Include RETURN_VALUE
    // in the try suite before resuming at the jump target.
    // A successful try returns immediately. An exception in the try can
    // reach a separate raise *after* POP_EXCEPT, with the caller's exception
    // restored. It must not run under the matching except.
    let after_handler = if end + 1 == dispatch
        && ops[end].name == "RETURN_VALUE"
        && kind.is_some()
        && following.is_empty()
        && ops[stop - 1].name == "RAISE_VARARGS"
        && let Some((pop, after)) = Body::handler_pop(ops, binding, stop)
        && after < stop
        && flow.block(pop).ops == (pop..stop)
        && flow.handler(pop).is_none_or(|h| h.target != unwind.target)
    {
        Some((pop, after))
    } else {
        None
    };
    let after_cleanup = cleanup + 3;
    let mut success = end..end;
    let parent = (first.depth == 1)
        .then(|| BoundParent::at_cleanup(owner, ops, cleanup))
        .flatten()
        .filter(|parent| parent.success(owner, ops, end..dispatch).is_some());
    let (end, mut next) = match ops.get(end..dispatch)? {
        _ if parent.is_some() => (end, dispatch),
        _ if let Some((_, after)) = after_handler => (dispatch, after),
        // An out-of-line handler can jump backwards past a try-else. Only
        // normal completion of the protected suite enters that else, which
        // may itself contain another placed try.
        _ if matches!(
            ops[stop - 1].name,
            "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
        ) && let Some(join) = flow.jump(stop - 1)
            && (end + 1..dispatch).contains(&join)
            && flow.block(join).height == Some(first.depth)
            && Body::handler_pop(ops, binding, stop)
                .is_some_and(|(pop, after)| body <= pop && after == stop - 1)
            && flow.blocks_in(end..join).all(|block| {
                [flow.jump(block.ops.end - 1), flow.fallthrough(block)]
                    .into_iter()
                    .flatten()
                    .all(|to| (end..=join).contains(&to))
            })
            && following_join.is_none_or(|to| to == join) =>
        {
            if join == end + 1
                && ops[end].name == "RETURN_VALUE"
                && flow.block(end).height == Some(first.depth + 1)
            {
                // The protected expression produced a pending return (often
                // an await). Only the handler falls through to the join.
                (join, join)
            } else {
                success = end..join;
                (end, join)
            }
        }
        [swap, pop, ret]
            if (swap.name, swap.arg) == ("SWAP", 2)
                && pop.name == "POP_EXCEPT"
                && ret.name == "RETURN_VALUE"
                && first.depth == 1
                && enclosing_exception(ops, flow, end, cleanup)
                && flow.handler(end + 1).is_none() =>
        {
            // Returning the inner try's value also exits an enclosing except.
            // The value was evaluated with that exception still active.
            (dispatch, dispatch)
        }
        [.., jump]
            if jump.name == "JUMP_FORWARD"
                && flow.jump(dispatch - 1) == Some(after_cleanup)
                && (end + 1 == dispatch || flow.block(end).ops == (end..dispatch)) =>
        {
            // A straight-line else runs outside the inner protection. Both
            // the successful try and a handled exception skip the unwind.
            success = end..dispatch - 1;
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
    let falls_off = owner.none_return(&ops[next..flow.block(next).ops.end]);
    if falls_off {
        for arm in &mut following {
            arm.fall_out_of_parent();
        }
    }
    let mut retry = false;
    let mut handler_tail = None;
    let mut handler_exits = None;
    let handler = match ops.get(body..stop)? {
        _ if let Some((pop, _)) = after_handler => body..pop,
        _ if let Some((child_end, tail)) =
            returning_handler_child(ops, owner, body..stop, unwind, inner) =>
        {
            handler_tail = tail.filter(|_| !falls_off);
            body..child_end
        }
        _ if !bound
            && let Some((pop, ret)) =
                parent.and_then(|parent| parent.handler_exit(owner, ops, stop))
            && pop >= body =>
        {
            handler_tail = ret.map(|at| at..stop);
            body..pop
        }
        _ if !bound
            && end == dispatch
            && first.depth == 1
            && let Some((expression_end, tail)) =
                nested_handler_return(ops, flow, body..stop, cleanup) =>
        {
            handler_tail = Some(tail..stop);
            body..expression_end
        }
        [.., raise] if raise.name == "RAISE_VARARGS" => body..stop,
        [.., jump]
            if matches!(
                jump.name,
                "JUMP_FORWARD" | "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
            ) && let Some((pop, after)) = Body::handler_pop(ops, binding, stop)
                && after == stop - 1
                && let Some(to) = flow.jump(stop - 1) =>
        {
            // A while loop's normal latch and handler can both jump straight
            // to the header. Falling out of this try reaches the same latch.
            let shared_latch = ops[next].name == "JUMP_BACKWARD"
                && to < start
                && flow.jump(next) == Some(to)
                && flow.block(to).height == Some(first.depth);
            if following.is_empty()
                && (next == end || flow.jump(end) == Some(next))
                && success.is_empty()
                && let Some(after_break) =
                    loop_exits::retry_break(ops, flow, start..end, to, after_cleanup)
            {
                retry = true;
                next = after_break;
            } else if to < start
                && ops[to..start].iter().all(|op| op.name == "NOP")
                && flow.block(to).height == Some(first.depth)
                && end < dispatch
                && ops[dispatch - 1].name == "JUMP_BACKWARD"
                && flow
                    .jump(dispatch - 1)
                    .is_some_and(|latch| latch == to || (latch == to + 1 && latch < start))
            {
                // A successful iteration has work between the try and the
                // latch. The handler jumps over it to the while header.
                handler_tail = Some(stop - 1..stop);
            } else if to != next
                && Some(to) != for_header
                && !shared_latch
                && parent.is_none_or(|parent| parent.exit(owner, ops, to).is_none())
            {
                // The handler skips the rest of this iteration. Only a jump
                // to the enclosing iterator at its saved depth is a continue.
                if ops[to].name != "FOR_ITER"
                    || to >= start
                    || flow.jump(to).is_none_or(|exit| end >= exit)
                    || flow.for_iter_depth(to) != Some(first.depth)
                {
                    return None;
                }
                // An explicit continue can have the same edges as a handler
                // falling through before an else. Its POP_EXCEPT has its own
                // source line, even when the handler did work before it.
                if body < pop
                    && owner.instruction_lines.as_deref().is_ok_and(|lines| {
                        lines[pop].is_none_or(|line| lines[body..pop].contains(&Some(line)))
                    })
                    && let Some(latch) = iterator_success_latch(owner, ops, start..next, to)
                {
                    success = next..latch;
                    next = latch;
                } else {
                    handler_tail = Some(stop - 1..stop);
                }
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
                    && (owner.depth == 0 && owner.none_return(&ops[ret..normal.ops.end])
                        || flow.incoming(next).any(|source| {
                            source > next
                                && flow.jump(source) == Some(next)
                                && flow
                                    .handler(source)
                                    .is_some_and(|h| h.target == first.target)
                        }))
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
    let try_exits = loop_exits::copied_latch(ops, flow, start..end, next);
    if try_exits.is_some() {
        next += 1;
    }
    if let Some(header) = following_join
        && parent.is_none_or(|parent| parent.exit(owner, ops, header).is_none())
        && (ops[header].name != "FOR_ITER"
            || flow.for_iter_depth(header) != Some(first.depth)
            || ops[next].name != "JUMP_BACKWARD"
            || flow.jump(next) != Some(header))
    {
        return None;
    }
    Some((
        start,
        Protected {
            return_exits: Vec::new(),
            end,
            next,
            clause: Clause::Except {
                kind,
                binding,
                success,
                following,
                finalizer: None,
                retry,
            },
            try_exits,
            handler_exits,
            handler,
            handler_tail,
        },
    ))
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
            && let Some(normal) = suite.normal_finalizer()
            && let Some(h) = flow.handler(child)
            && let Some(unwind) = suite.unwind(ops, flow, h)
            && let Some(parent) = flow.handler(unwind)
            && parent.start == ops[unwind].offset
            && (parent.target, parent.depth, parent.lasti)
                == (first.target, first.depth, first.lasti)
            && index(parent.end)? == unwind + 3
            && (at == unwind
                || at == suite.next
                || (normal.start == at
                    && normal.end == suite.next
                    && suite.next <= index(entry.end)?))
        {
            cleanups.insert(unwind, unwind + 3);
            end = suite.next;
            if at == normal.start && normal.end == end {
                // Protection can resume on the normal finalizer copy and
                // continue into further statements in the enclosing try.
                end = index(entry.end)?;
                continue;
            }
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
        if end + 1 == at && loop_exits::break_over_cleanup(ops, flow, end, first).is_some() {
            end = index(entry.end)?;
            continue;
        }
        if end < at
            && (iterator_return_block(ops, flow, end..at, first.depth)
                || (first.depth == 0
                    && flow.block(end).ops == (end..at)
                    && flow.block(end).height == Some(0)
                    && flow.handler(end).is_none()
                    && super::regions::constant_return(ops, at).is_some_and(|(load, _)| {
                        load == end + usize::from(ops[end].name == "NOT_TAKEN")
                    })))
            && flow.incoming(at).any(|i| {
                i < end
                    && flow.jump(i) == Some(at)
                    && ops[i].name.starts_with("POP_JUMP_IF_")
                    && flow.handler(i).is_some_and(|h| {
                        (h.target, h.depth, h.lasti) == (first.target, first.depth, false)
                    })
            })
        {
            end = index(entry.end)?;
            continue;
        }
        if super::loops::branch_gap(ops, flow, end..at, first) {
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
        // A throw inside an out-of-line handler can resume END_SEND by
        // fallthrough. Its cleanup already belongs to this protected span.
        if (at..end).contains(&cleanup)
            && cleanup + 1 < end
            && ops[cleanup + 1].name == "END_SEND"
            && flow.jump(at.checked_sub(2)?) == Some(cleanup + 1)
        {
            continue;
        }
        let resume = flow.jump(cleanup + 1)?;
        if !(at..end).contains(&resume) || ops[resume].name != "END_SEND" {
            return None;
        }
        cleanups.insert(cleanup, cleanup + 1);
    }
    cleanups.is_empty().then_some(end)
}

impl Body<'_, '_, '_> {
    /// A loop may walk over a whole protected suite whose dispatcher lives
    /// after the latch. The suite lifter owns that exceptional path.
    pub(super) fn loop_contains_handler(
        &self,
        ops: &[Instruction],
        entry: Handler,
        body: Range<usize>,
    ) -> bool {
        let Ok(start) = ops.binary_search_by_key(&entry.start, |op| op.offset) else {
            return false;
        };
        !entry.lasti
            && body.contains(&start)
            && self.handlers.get(&start).is_some_and(|suite| {
                ops.get(suite.end).is_some_and(|op| op.offset == entry.end)
                    && start < suite.next
                    && suite.next <= body.end
            })
    }

    /// An out-of-line handler may jump back into the normal layout after
    /// restoring the saved exception. Attribute that edge to its placed try.
    pub(super) fn handler_jumps_from(
        &self,
        ops: &[Instruction],
        body: Range<usize>,
        source: usize,
        target: usize,
    ) -> bool {
        ops.get(source)
            .is_some_and(|op| matches!(op.name, "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"))
            && self.flow.jump(source) == Some(target)
            && self.handlers.iter().any(|(&start, suite)| {
                let Clause::Except { binding, .. } = suite.clause else {
                    return false;
                };
                body.contains(&start)
                    && start < suite.end
                    && suite.end <= target
                    && suite.next == target
                    && suite.handler.end + 1 + 3 * usize::from(ops[binding].name != "POP_TOP")
                        == source
                    && Self::handler_pop(ops, binding, source + 1)
                        == Some((suite.handler.end, source))
            })
    }

    /// A nested try can own the rest of a manager's body. On an exception
    /// its final unwind must transfer control to this manager's exit.
    pub(super) fn managed_child_end(
        &self,
        ops: &[Instruction],
        mut start: usize,
        manager: Handler,
    ) -> Option<usize> {
        while ops.get(start)?.name == "NOP" {
            start += 1;
        }
        let child = self.handlers.get(&start)?;
        let entry = self.flow.handler(start)?;
        let unwind = child.unwind(ops, &self.flow, entry)?;
        let parent = self.flow.handler(unwind)?;
        (!entry.lasti
            && entry.depth == manager.depth
            && child.next > start
            && parent.start == ops[unwind].offset
            && parent.end == ops.get(unwind + 3)?.offset
            && (parent.target, parent.depth, parent.lasti)
                == (manager.target, manager.depth, manager.lasti))
            .then_some(child.next)
    }

    pub(super) fn protected_fragment(
        &mut self,
        ops: &[Instruction],
        body: Range<usize>,
        level: usize,
        exits: Option<ExitCopies>,
        rejoin: Option<HandlerRejoin>,
    ) -> Result<super::regions::Fragment, String> {
        let outer = std::mem::replace(&mut self.clause_exits, exits);
        let outer_rejoin = std::mem::replace(&mut self.handler_rejoin, rejoin);
        let fragment = self.fragment(ops, body.start, body.end, level);
        self.handler_rejoin = outer_rejoin;
        self.clause_exits = outer;
        fragment
    }

    pub(super) fn protected_region(
        &mut self,
        ops: &[Instruction],
        start: usize,
        limit: usize,
        level: usize,
        entry: &mut Protected,
    ) -> Result<Option<usize>, String> {
        if entry.next > limit || !self.stack.is_empty() {
            return Ok(None);
        }
        let outer = std::mem::replace(&mut self.active_try, self.flow.handler(start));
        std::mem::swap(&mut self.return_exits, &mut entry.return_exits);
        let suite = self.protected_fragment(ops, start..entry.end, level, entry.try_exits, None);
        std::mem::swap(&mut self.return_exits, &mut entry.return_exits);
        self.active_try = outer;
        let (mut suite, returns) = suite?.pending_return()?;
        let mut next = entry.next;
        let terminal_unwind = match entry.clause {
            Clause::Finally {
                terminal_unwind, ..
            } => terminal_unwind,
            Clause::Except { .. } => None,
        };
        let consumes_return = terminal_unwind
            .is_some_and(|(_, exit)| exit == super::finalizers::CopyExit::PendingReturn);
        if consumes_return && !returns {
            return Err("missing return value across finally".into());
        }
        let carried = returns
            && !consumes_return
            && next == limit
            && matches!(
                entry.clause,
                Clause::Finally { .. }
                    | Clause::Except {
                        finalizer: Some(_),
                        ..
                    }
            );
        if returns && !carried && !consumes_return {
            if next >= limit || ops[next].name != "RETURN_VALUE" {
                return Err("unsupported return through suspended handler".into());
            }
            next += 1;
        }
        let header = match &entry.clause {
            Clause::Except { kind, binding, .. } => {
                self.except_header(ops, "except", kind.clone(), *binding, level)?
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
        let outer = std::mem::replace(
            &mut self.finally_unwind,
            terminal_unwind.map(|(target, _)| target),
        );
        let rejoin = match &entry.clause {
            Clause::Except {
                binding, success, ..
            } if success.is_empty() && entry.handler_tail.is_none() => {
                HandlerRejoin::new(ops, self, *binding, entry.next)
            }
            _ => None,
        };
        let handler = self.protected_fragment(
            ops,
            entry.handler.clone(),
            level,
            entry.handler_exits,
            rejoin,
        );
        self.finally_unwind = outer;
        let handler = handler?;
        let handler = if let Some(tail) = &entry.handler_tail {
            handler.resume(self, ops, tail.clone(), level)?
        } else {
            handler.suite()?
        };
        let retry = matches!(entry.clause, Clause::Except { retry: true, .. });
        let try_start = self.lines.len();
        if retry {
            suite.push_str("break\n");
        }
        self.try_block(&suite, start > 0 && ops[start - 1].name != "NOP")?;
        self.block(&header, &handler)?;
        if let Clause::Except {
            success,
            following,
            finalizer,
            ..
        } = &entry.clause
        {
            for arm in following {
                self.emit_arm(ops, arm, level, true)?;
            }
            if !success.is_empty() {
                let suite = self
                    .fragment(ops, success.start, success.end, level)?
                    .suite()?;
                self.block("else:", &suite)?;
            }
            if let Some(normal) = finalizer {
                let suite = self
                    .fragment(ops, normal.start, normal.end, level)?
                    .suite()?;
                self.block("finally:", &suite)?;
            }
        }
        if retry {
            let body = self.lines.split_off(try_start);
            self.block("while True:", &body)?;
        }
        if terminal_unwind.is_some()
            && !consumes_return
            && entry.next > entry.end
            && (level > 0 || next < limit)
        {
            self.line("return")?;
        }
        if carried {
            self.stack.push(super::Value::PendingReturn);
        }
        // An implicit None return inherits the protected statement's line.
        // Spelling it after the try can merge distinct binding cleanups on 3.12.
        if level == 0
            && next > 0
            && next < limit
            && self.none_return(&ops[next..limit])
            && self
                .instruction_lines
                .as_deref()
                .is_ok_and(|lines| lines[next].is_some() && lines[next] == lines[next - 1])
        {
            next = limit;
        }
        Ok(Some(next))
    }
}
