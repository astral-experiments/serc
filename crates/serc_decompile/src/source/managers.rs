//! Recover context manager entries, protected bodies, and suppression exits.
mod retry;
use super::regions::ReturnExit;
use super::{Body, Bytecode, Instruction, Value, exceptions::boundary};
use crate::flow::Graph;
use rustc_hash::FxHashMap;
use serc_bytecode::instructions::Handler;
use serc_bytecode::target::operands::SpecialMethod;
use std::ops::Range;

pub(super) fn body_end(ops: &[Instruction], flow: &Graph, entry: Handler) -> Result<usize, String> {
    let mut end = boundary(ops, entry.end)?;
    while let Some((at, resume)) =
        (end + 1..ops.len().min(end + 4)).find_map(|at| flow.handler(at).map(|resume| (at, resume)))
        && resume.start == ops[at].offset
        && (resume.target, resume.depth, resume.lasti) == (entry.target, entry.depth, entry.lasti)
    {
        if !super::loops::branch_gap(ops, flow, end..at, entry) {
            let Some(suspend) = flow.handler(end) else {
                break;
            };
            if at != end + 1
                || ops[end].name != "YIELD_VALUE"
                || ops[at].name != "RESUME"
                || suspend.start != ops[end].offset
                || suspend.end != ops[at].offset
                || suspend.lasti
                || ops[boundary(ops, suspend.target)?].name != "CLEANUP_THROW"
            {
                break;
            }
        }
        end = boundary(ops, resume.end)?;
    }
    Ok(end)
}

/// A surrounding try resumes protection for the normal manager exit and
/// again when suppression or a failed exit restores the exception stack.
pub(super) fn bridges_try(
    ops: &[Instruction],
    flow: &Graph,
    gap: Range<usize>,
    parent: Handler,
    cleanups: &mut FxHashMap<usize, usize>,
) -> Option<()> {
    let manager = flow.handler(gap.start)?;
    if !manager.lasti
        || manager.depth <= parent.depth
        || manager.start != ops[gap.start].offset
        || !is_manager_handler(ops, &manager).ok()?
    {
        return None;
    }
    // A completed for loop can leave an unprotected tracing NOP before
    // the enclosing try takes ownership of the manager's normal exit.
    let end = body_end(ops, flow, manager).ok()?;
    if !ops.get(end..gap.end)?.iter().all(|op| op.name == "NOP")
        || !flow
            .blocks_covering(end..gap.end)
            .all(|b| flow.handler(b.ops.start).is_none())
    {
        return None;
    }
    let dispatch = boundary(ops, manager.target).ok()?;
    let (_, unwind) = super::dispatch::unwind(ops, flow, dispatch, manager.depth + 2)?;
    let mut owned = Vec::new();
    for block in flow.blocks_covering(dispatch..unwind + 3) {
        let Some(h) = flow.handler(block.ops.start) else {
            continue;
        };
        if (h.target, h.depth, h.lasti) != (parent.target, parent.depth, parent.lasti)
            || h.start != ops[block.ops.start].offset
        {
            continue;
        }
        let end = boundary(ops, h.end).ok()?;
        if end > unwind + 3
            || !(block.ops.start..end).all(|i| match ops[i].name {
                "COPY" => ops[i].arg == 3,
                "RERAISE" => ops[i].arg == 1,
                "POP_TOP" | "POP_EXCEPT" => true,
                "JUMP_FORWARD" | "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT" => flow
                    .jump(i)
                    .is_some_and(|dest| dest >= gap.end && !(dispatch..unwind + 3).contains(&dest)),
                _ => false,
            })
        {
            return None;
        }
        owned.push((block.ops.start, end));
    }
    cleanups.extend(owned);
    Some(())
}

fn is_manager_handler(ops: &[Instruction], entry: &Handler) -> Result<bool, String> {
    let target = boundary(ops, entry.target)?;
    Ok(ops[target].name == "PUSH_EXC_INFO"
        && ops
            .get(target + 1)
            .is_some_and(|op| op.name == "WITH_EXCEPT_START"))
}

/// Locate the implicit manager protocol immediately before a protected binding.
fn manager_entry(ops: &[Instruction], asynchronous: bool) -> Result<usize, String> {
    let (before_with, enter_method, exit_method) = if asynchronous {
        (
            "BEFORE_ASYNC_WITH",
            SpecialMethod::__AENTER__,
            SpecialMethod::__AEXIT__,
        )
    } else {
        (
            "BEFORE_WITH",
            SpecialMethod::__ENTER__,
            SpecialMethod::__EXIT__,
        )
    };
    match ops {
        [.., before] if before.name == before_with => Ok(ops.len() - 1),
        [.., copy, exit, swap2, swap3, enter, call]
            if (copy.name, copy.arg) == ("COPY", 1)
                && (exit.name, exit.arg) == ("LOAD_SPECIAL", exit_method.argument())
                && (swap2.name, swap2.arg) == ("SWAP", 2)
                && (swap3.name, swap3.arg) == ("SWAP", 3)
                && (enter.name, enter.arg) == ("LOAD_SPECIAL", enter_method.argument())
                && (call.name, call.arg) == ("CALL", 0) =>
        {
            Ok(ops.len() - 6)
        }
        _ => Err("unsupported manager entry".into()),
    }
}

impl Body<'_, '_, '_> {
    fn manager_item(
        &mut self,
        manager: String,
        ops: &[Instruction],
        start: usize,
        end: usize,
    ) -> Result<(String, usize, Vec<Value>), String> {
        if ops.get(start).is_some_and(|op| op.name == "POP_TOP") {
            return Ok((manager, start + 1, Vec::new()));
        }
        let binding = self.binding(ops, start, end)?;
        Ok((
            format!("{manager} as {}", binding.target),
            binding.next,
            binding.seed,
        ))
    }

    pub(super) fn with_statement(
        &mut self,
        bc: &Bytecode,
        handlers: &[Handler],
        start: usize,
        limit: usize,
    ) -> Result<(), String> {
        let ops = &bc.ops;
        let mut ranges = handlers.iter();
        let mut cursor = start;
        let mut normal_end = self.flow.iteration_end(ops, [cursor]).min(limit);
        let mut depth = None;
        while cursor < normal_end {
            // Skip ranges that resume outer protection during the previous
            // group's cleanup. Only a subsequent independent manager at the
            // same depth starts another statement.
            let Some(entry) = ranges.find(|range| range.start >= ops[cursor].offset) else {
                break;
            };
            if boundary(ops, entry.start)? >= normal_end {
                break;
            }
            if !entry.lasti || !is_manager_handler(ops, entry)? {
                if self.suspended() {
                    continue;
                }
                if start > 0 {
                    // The prefix proved protection only as far as this with.
                    // A later independent try still needs its own placement.
                    let stop = boundary(ops, entry.start)?;
                    self.region(ops, cursor, stop, 0)?;
                    if self.stack.is_empty() {
                        self.partial_start = stop;
                    }
                    return Err(format!("unplaced exception region at byte {}", entry.start));
                }
                break;
            }
            if *depth.get_or_insert(entry.depth) != entry.depth {
                break;
            }
            // A guard or loop can skip the entire with statement. Let its
            // enclosing region place the manager on the edge that enters it.
            let binding = boundary(ops, entry.start)?;
            if ops[cursor..binding]
                .iter()
                .any(|op| op.jump.is_some_and(|target| target >= entry.start))
            {
                break;
            }
            if let Some(next) = self.manager_group(ops, cursor, *entry, 0, &mut normal_end)? {
                cursor = next;
            } else {
                // A return expression can raise before the return takes effect.
                // Suppression by the outermost manager resumes after the with.
                let next = self.after_with_suppression(ops, entry)?;
                if next <= cursor || next >= limit {
                    return Err("unsupported continuation after with".into());
                }
                self.charge(ops.len())?;
                normal_end = self.flow.normal_end([next]).min(limit);
                if self.none_return(&ops[next..normal_end]) {
                    return Ok(());
                }
                cursor = next;
            }
        }
        self.region(ops, cursor, normal_end, 0)
    }

    /// At the end of a guarded prefix, hand an unconditional synchronous
    /// entry to the same walker used for a leading with statement.
    pub(super) fn manager_after_prefix(
        &self,
        ops: &[Instruction],
        binding: usize,
    ) -> Option<usize> {
        let entry = self.flow.handler(binding)?;
        let enter = manager_entry(&ops[..binding], false).ok()?;
        (entry.lasti
            && entry.start == ops[binding].offset
            && is_manager_handler(ops, &entry).ok()?
            && self.flow.entered_from(binding, enter..binding)
            && !ops[..enter]
                .iter()
                .any(|op| op.jump.is_some_and(|to| to >= ops[enter].offset)))
        .then_some(enter)
    }

    /// A manager inside a conditional or protected suite starts at the
    /// protocol opcode, after the surrounding expression supplied its value.
    pub(super) fn inline_manager(
        &mut self,
        ops: &[Instruction],
        enter: usize,
        end: usize,
        level: usize,
    ) -> Result<Option<usize>, String> {
        let width = match ops[enter].name {
            "BEFORE_WITH" | "BEFORE_ASYNC_WITH" => 1,
            "COPY"
                if ops
                    .get(enter + 1)
                    .is_some_and(|op| op.name == "LOAD_SPECIAL") =>
            {
                6
            }
            _ => return Ok(None),
        };
        let after = enter + width;
        let Some(protocol) = ops.get(enter..after) else {
            return Ok(None);
        };
        let asynchronous = manager_entry(protocol, true).is_ok();
        if !asynchronous && manager_entry(protocol, false).is_err() {
            return Ok(None);
        }
        let binding = if asynchronous {
            self.manager_await_end(ops, after, 1)? + 1
        } else {
            after
        };
        if binding >= end {
            return Err("manager binding escapes its suite".into());
        }
        let entry = self
            .flow
            .handler(binding)
            .ok_or("missing manager protection")?;
        if !entry.lasti || !is_manager_handler(ops, &entry)? || entry.start != ops[binding].offset {
            return Err("invalid manager protection".into());
        }
        let mut normal_end = end;
        if let Some(next) = self.manager_group(ops, enter, entry, level, &mut normal_end)? {
            return Ok(Some(next));
        }
        let next = self.after_with_suppression(ops, &entry)?;
        if next <= enter || next >= ops.len() {
            return Err("unsupported continuation after with".into());
        }
        if next > end {
            // CPython can copy a terminal block to the suppression
            // path. Emit that return after the with, where it runs
            // only if the pending return expression raised.
            let stop = self.flow.block(next).ops.end;
            if matches!(ops[stop - 1].name, "RETURN_CONST" | "RETURN_VALUE")
                && self
                    .flow
                    .handler(next)
                    .is_none_or(|h| self.suspended() && h.depth == 0 && h.lasti)
            {
                self.region(ops, next, stop, level + 1)?;
                return Ok(Some(end));
            }
        }
        Ok(Some(next))
    }

    fn after_with_suppression(
        &self,
        ops: &[Instruction],
        entry: &Handler,
    ) -> Result<usize, String> {
        let handler = boundary(ops, entry.target)?;
        let test = if ops
            .get(handler + 2)
            .is_some_and(|op| op.name == "GET_AWAITABLE")
        {
            self.manager_await_end(ops, handler + 2, 2)?
        } else {
            handler
        };
        let branch = self.flow.block(test).ops.end - 1;
        if ops[branch].name != "POP_JUMP_IF_TRUE" {
            return Err("unsupported with suppression test".into());
        }
        let target = self.flow.target(branch)?;
        let [pop, restore, rest @ ..] = &ops[target..] else {
            return Err("missing with suppression cleanup".into());
        };
        // The cleanup discards the active exception, restores the previous one,
        // and pops the saved instruction offset and manager protocol operands.
        let pops = rest
            .iter()
            .take(4)
            .take_while(|op| op.name == "POP_TOP")
            .count();
        if pop.name != "POP_TOP"
            || restore.name != "POP_EXCEPT"
            || self.flow.block(target).height != Some(entry.depth + 3)
            || !(2..=3).contains(&pops)
        {
            return Err("unsupported with suppression cleanup".into());
        }
        let next = target + 2 + pops;
        if ops.get(next).is_some_and(|op| op.name == "JUMP_FORWARD") {
            self.flow.target(next)
        } else {
            Ok(next)
        }
    }

    /// A nested with can interrupt its parent's protected range. Normal exit
    /// and suppression must reach the same continuation under the outer with.
    fn nested_manager_resume(
        &self,
        ops: &[Instruction],
        binding: usize,
        parent: Handler,
    ) -> Option<(Handler, usize)> {
        let op = ops.get(binding)?;
        let inner = self.flow.handler(binding)?;
        if inner.start != op.offset
            || !inner.lasti
            || inner.depth <= parent.depth
            || !is_manager_handler(ops, &inner).ok()?
        {
            return None;
        }
        let after = self.after_with_suppression(ops, &inner).ok()?;
        let join = if matches!(
            ops.get(after)?.name,
            "JUMP_BACKWARD" | "JUMP_BACKWARD_NO_INTERRUPT"
        ) {
            self.flow.jump(after)?
        } else {
            after
        };
        if join <= binding || join >= ops.len() {
            return None;
        }
        let resume = self.flow.handler(join - 1)?;
        let cleanup = boundary(ops, resume.start).ok()?;
        (cleanup > binding
            && (resume.target, resume.depth, resume.lasti)
                == (parent.target, parent.depth, parent.lasti)
            && self.manager_exit(ops, cleanup..join, false, false) == Some(join))
        .then_some((resume, join))
    }

    fn manager_await_end(
        &self,
        ops: &[Instruction],
        at: usize,
        kind: u32,
    ) -> Result<usize, String> {
        if ops
            .get(at)
            .is_none_or(|op| (op.name, op.arg) != ("GET_AWAITABLE", kind))
        {
            return Err("missing awaited manager exit".into());
        }
        let send = at + 2 + usize::from(ops.get(at + 1).is_some_and(|op| op.name == "PUSH_NULL"));
        self.flow
            .jump(send)
            .filter(|&end| ops[send].name == "SEND" && end > send && ops[end].name == "END_SEND")
            .ok_or_else(|| "unsupported awaited manager exit".into())
    }

    /// Recover one with statement, including adjacent items in its header.
    /// Return the normal continuation, or None for a return through the managers.
    fn manager_group(
        &mut self,
        ops: &[Instruction],
        cursor: usize,
        mut entry: Handler,
        level: usize,
        normal_end: &mut usize,
    ) -> Result<Option<usize>, String> {
        let awaited = |entry: &Handler| -> Result<bool, String> {
            let handler = boundary(ops, entry.target)?;
            Ok(ops
                .get(handler + 2)
                .is_some_and(|op| (op.name, op.arg) == ("GET_AWAITABLE", 2)))
        };
        let scope_end = *normal_end;
        let mut managers = Vec::new();
        let mut previous_body = None;
        let mut seed = Vec::new();
        let (start, end) = loop {
            let asynchronous = awaited(&entry)?;
            let start = boundary(ops, entry.start)?;
            // A delegated yield briefly transfers protection to CLEANUP_THROW.
            // Its resume belongs to the same manager at the same stack depth.
            let end = body_end(ops, &self.flow, entry)?;
            let handler = boundary(ops, entry.target)?;
            *normal_end = (*normal_end).min(handler);
            if start >= end || end > *normal_end {
                return Err("unsupported manager layout".into());
            }
            let before = previous_body.unwrap_or(cursor);
            let protocol_end = if asynchronous {
                before
                    + ops[before..start]
                        .iter()
                        .rposition(|op| (op.name, op.arg) == ("GET_AWAITABLE", 1))
                        .ok_or("missing awaited manager entry")?
            } else {
                start
            };
            let enter = before + manager_entry(&ops[before..protocol_end], asynchronous)?;
            let manager = if let Some(previous) = previous_body {
                self.seeded(ops, previous, enter, level, std::mem::take(&mut seed))?
                    .expression()?
            } else {
                self.region(ops, cursor, enter, level)?;
                self.expr()?
            };
            let (item, body_start, loaded) = self.manager_item(manager, ops, start, end)?;
            managers.push((asynchronous, item, entry));
            seed = loaded;

            // Each additional item starts where the enclosing protected range
            // ends. The enclosing handler resumes protection during inner cleanup.
            if let Some(inner) = self.flow.handler(end)
                && inner.start == ops[end].offset
                && inner.depth > entry.depth
                && inner.lasti
                && is_manager_handler(ops, &inner)?
            {
                // A later with can have ordinary statements before its entry
                // or after its exit. Keep them inside the outer suite.
                if let Some((resume, join)) = self.nested_manager_resume(ops, end, entry)
                    && (join < boundary(ops, resume.end)?
                        || seed.is_empty()
                            && body_start < end
                            && self
                                .empty_stack_after(
                                    ops,
                                    body_start..self.flow.block(body_start).ops.end.min(end),
                                    body_start,
                                    0,
                                )?
                                .is_some())
                {
                    break (body_start, end);
                }
                previous_body = Some(body_start);
                entry = inner;
            } else {
                break (body_start, end);
            }
        };
        let single = (managers.len() == 1).then(|| managers[0].0);
        let (end, exits) = self.manager_body_tail(ops, end, entry, single, *normal_end)?;
        let outer = std::mem::replace(&mut self.return_exits, exits);
        let return_exit = self.work.partial
            && managers
                .iter()
                .rev()
                .try_fold(end, |at, (asynchronous, _, _)| {
                    self.manager_exit(ops, at..*normal_end, *asynchronous, true)
                })
                .and_then(|at| ops.get(at))
                .is_some_and(|op| op.name == "RETURN_VALUE");
        let outer_protection = self.active_try.replace(entry);
        let body = self.suite_fragment(ops, start..end, level, seed, return_exit);
        self.active_try = outer_protection;
        self.return_exits = outer;
        let (mut body, returns) = body?.pending_return()?;
        let retry = if returns {
            None
        } else {
            self.managed_retry(ops, end, entry, level)?
        };
        let terminal_loop = retry.is_some();
        if let Some(retry) = retry {
            body.push_str(&retry);
        }
        // Adjacent managers of the same kind share a header. A switch
        // between with and async with requires a nested suite.
        let mut indent = String::new();
        for group in managers.chunk_by(|a, b| a.0 == b.0) {
            let header = if group[0].0 { "async with" } else { "with" };
            let mut items = group[0].1.clone();
            let mut multiline = false;
            for pair in group.windows(2) {
                let outer = pair[0].2;
                let inner = pair[1].2;
                // In 3.12, exiting managers written on different lines can
                // leave a tracing NOP between their two exit calls.
                let split = self.m.version == super::PythonVersion::Py312
                    && self.instruction_lines.as_deref().is_ok_and(|lines| {
                        let line = |entry: Handler| lines[boundary(ops, entry.start).ok()?];
                        line(outer).zip(line(inner)).is_some_and(|(a, b)| a != b)
                    })
                    && self
                        .after_with_suppression(ops, &inner)
                        .ok()
                        .and_then(|at| ops.get(at))
                        .is_some_and(|op| op.name == "NOP");
                items.push(',');
                if split {
                    items.push('\n');
                    items.push_str(&indent);
                    items.push_str("    ");
                    multiline = true;
                } else {
                    items.push(' ');
                }
                items.push_str(&pair[1].1);
            }
            if multiline {
                self.line(&format!("{indent}{header} ({items}):"))?;
            } else {
                self.line(&format!("{indent}{header} {items}:"))?;
            }
            indent.push_str("    ");
        }
        if body.is_empty() {
            self.line(&format!("{indent}pass"))?;
        }
        for line in body.lines() {
            self.line(&format!("{indent}{line}"))?;
        }
        if terminal_loop {
            return Ok(None);
        }

        // A manager whose body raises on every path has no normal exit
        // call. Suppression still resumes through its exceptional cleanup.
        if !returns
            && managers.len() == 1
            && ops.get(end).is_some_and(|op| op.offset == entry.target)
            && end > start
            && ops[end - 1].name == "RAISE_VARARGS"
            && self.flow.blocks_covering(start..end).all(|b| {
                [self.flow.jump(b.ops.end - 1), self.flow.fallthrough(b)]
                    .into_iter()
                    .flatten()
                    .all(|to| (start..end).contains(&to))
            })
        {
            return Ok(None);
        }
        let mut next = end;
        for (i, (asynchronous, _, manager)) in managers.iter().enumerate().rev() {
            next = self
                .manager_exit(ops, next..*normal_end, *asynchronous, returns)
                .ok_or_else(|| format!("unsupported manager exit at byte {}", ops[next].offset))?;
            if i > 0
                && !returns
                && ops.get(next).is_some_and(|op| op.name == "JUMP_FORWARD")
                && let Some(join) = self.flow.jump(next)
                && self.after_with_suppression(ops, manager)? == join
            {
                // Both successful exit and suppression skip the inner
                // dispatcher before running the enclosing manager's exit.
                *normal_end = scope_end.min(boundary(ops, managers[i - 1].2.target)?);
                if join >= *normal_end {
                    return Err("nested manager exit escapes its suite".into());
                }
                next = join;
            }
        }
        if returns {
            if ops.get(next).is_none_or(|op| op.name != "RETURN_VALUE") {
                return Err("unsupported return through with".into());
            }
            Ok(None)
        } else {
            Ok(Some(next))
        }
    }

    fn manager_body_tail(
        &self,
        ops: &[Instruction],
        mut end: usize,
        entry: Handler,
        single: Option<bool>,
        limit: usize,
    ) -> Result<(usize, Vec<ReturnExit>), String> {
        // An early return splits the protected body around its normal
        // __exit__ call. The next arm is still inside the same manager.
        let mut exits = Vec::new();
        loop {
            let previous = end;
            if let Some((resume, _)) = self.nested_manager_resume(ops, end, entry) {
                end = body_end(ops, &self.flow, resume)?;
            } else if let Some(next) = self
                .enclosed_try_end(ops, end, entry)
                .or_else(|| self.managed_async_iteration_end(ops, end, entry))
                .or_else(|| {
                    super::comprehensions::protected_body(ops, &self.flow, end, entry)
                        .map(|(next, _)| next)
                })
            {
                // The normal copy of a finally may already be protected by
                // this manager when the next ordinary statement is reached.
                if next >= ops.len() {
                    return Err("nested suite escapes manager body".into());
                }
                end = if let Some(resume) = self.flow.handler(next)
                    && (resume.target, resume.depth, resume.lasti)
                        == (entry.target, entry.depth, entry.lasti)
                {
                    body_end(ops, &self.flow, resume)?
                } else {
                    next
                };
            } else if let Some(asynchronous) = single
                && let Some((exit, next)) =
                    self.manager_return_exit(ops, end..limit, entry, asynchronous)
            {
                exits.push(exit);
                end = body_end(ops, &self.flow, next)?;
            } else {
                break;
            }
            if end <= previous || end > limit {
                return Err("nested suite escapes manager body".into());
            }
        }
        Ok((end, exits))
    }

    /// Match the implicit __exit__(None, None, None). Only a pending
    /// return value needs to be moved below the call's operands.
    fn manager_exit(
        &self,
        ops: &[Instruction],
        span: Range<usize>,
        asynchronous: bool,
        returns: bool,
    ) -> Option<usize> {
        let mut tail = ops.get(span.clone())?;
        while let [op, rest @ ..] = tail
            && op.name == "NOP"
        {
            tail = rest;
        }
        let modern = self.m.version >= super::PythonVersion::Py314;
        if returns {
            if modern {
                let [swap, rest @ ..] = tail else { return None };
                if (swap.name, swap.arg) != ("SWAP", 3) {
                    return None;
                }
                tail = rest;
            }
            let [swap, rest @ ..] = tail else { return None };
            if (swap.name, swap.arg) != ("SWAP", 2) {
                return None;
            }
            tail = rest;
        }
        let [a, b, c, call, rest @ ..] = tail else {
            return None;
        };
        if ![a, b, c].into_iter().all(|op| {
            matches!(op.name, "LOAD_CONST" | "LOAD_COMMON_CONSTANT") && self.none_constant(*op)
        }) || (call.name, call.arg) != ("CALL", if modern { 3 } else { 2 })
        {
            return None;
        }
        let after = span.end - rest.len();
        let pop = if asynchronous {
            self.manager_await_end(ops, after, 2).ok()? + 1
        } else {
            after
        };
        (pop < span.end && ops[pop].name == "POP_TOP").then_some(pop + 1)
    }

    /// A continue in an except arm first restores the active exception, then
    /// calls the enclosing manager's normal exit before reentering the loop.
    pub(super) fn manager_continue(
        &self,
        ops: &[Instruction],
        binding: usize,
        stop: usize,
        unwind: usize,
    ) -> Option<(usize, usize)> {
        let (pop, exit) = Self::handler_pop(ops, binding, stop)?;
        let manager = self.flow.handler(pop)?;
        let parent = self.flow.handler(unwind)?;
        if !manager.lasti
            || !is_manager_handler(ops, &manager).ok()?
            || (parent.target, parent.depth, parent.lasti) != (manager.target, manager.depth, true)
        {
            return None;
        }
        let jump = self.manager_exit(ops, exit..stop, false, false)?;
        if jump + 1 != stop || !serc_bytecode::target::is_unconditional_jump(ops[jump].opcode) {
            return None;
        }
        let to = self.flow.jump(jump)?;
        let latch = if ops[to].name == "JUMP_BACKWARD" {
            to
        } else {
            jump
        };
        let header = self.flow.jump(latch)?;
        let slots = if self.m.version >= super::PythonVersion::Py314 {
            2
        } else {
            1
        };
        (ops[header].offset < manager.start
            && self.flow.block(header).height == Some(manager.depth.checked_sub(slots)?))
        .then_some((pop, latch))
    }

    fn manager_return_exit(
        &self,
        ops: &[Instruction],
        span: Range<usize>,
        entry: Handler,
        asynchronous: bool,
    ) -> Option<(ReturnExit, Handler)> {
        let (pending, ret) = [true, false].into_iter().find_map(|pending| {
            self.manager_exit(ops, span.clone(), asynchronous, pending)
                .map(|ret| (pending, ret))
        })?;
        let next = match ops.get(ret..span.end)? {
            [op, ..]
                if pending && op.name == "RETURN_VALUE"
                    || !pending && op.name == "RETURN_CONST" =>
            {
                ret + 1
            }
            [load, ret_op, ..]
                if !pending
                    && matches!(
                        load.name,
                        "LOAD_CONST" | "LOAD_COMMON_CONSTANT" | "LOAD_SMALL_INT"
                    )
                    && ret_op.name == "RETURN_VALUE" =>
            {
                ret + 2
            }
            _ => return None,
        };
        if next >= span.end {
            return None;
        }
        let h = self.flow.handler(next)?;
        (ops[next].offset == h.start
            && (h.target, h.depth, h.lasti) == (entry.target, entry.depth, entry.lasti))
            .then_some((
                ReturnExit {
                    start: span.start,
                    ret,
                    pending,
                },
                h,
            ))
    }
}
