//! Recover context manager entries, protected bodies, and suppression exits.
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
        limit: usize,
    ) -> Result<(), String> {
        let ops = &bc.ops;
        let mut ranges = handlers.iter();
        let mut cursor = 0;
        let mut normal_end = self.flow.normal_end([0]).min(limit);
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
            || self.flow.block(target).height != Some(2 + pops)
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
        let mut managers = Vec::new();
        let mut previous_body = None;
        let mut seed = Vec::new();
        let (start, mut end) = loop {
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
            managers.push((asynchronous, item));
            seed = loaded;

            // Each additional item starts where the enclosing protected range
            // ends. The enclosing handler resumes protection during inner cleanup.
            if let Some(inner) = self.flow.handler(end)
                && inner.start == ops[end].offset
                && inner.depth > entry.depth
                && inner.lasti
                && is_manager_handler(ops, &inner)?
            {
                previous_body = Some(body_start);
                entry = inner;
            } else {
                break (body_start, end);
            }
        };
        // An early return splits the protected body around its normal
        // __exit__ call. The next arm is still inside the same manager.
        while let Some(next) = self.managed_child_end(ops, end, entry) {
            if next > *normal_end {
                return Err("nested try escapes manager body".into());
            }
            end = if let Some(resume) = self.flow.handler(next)
                && resume.start == ops[next].offset
                && (resume.target, resume.depth, resume.lasti)
                    == (entry.target, entry.depth, entry.lasti)
            {
                body_end(ops, &self.flow, resume)?
            } else {
                next
            };
        }
        let mut exits = Vec::new();
        if let [(asynchronous, _)] = managers.as_slice() {
            while let Some((exit, next)) =
                self.manager_return_exit(ops, end..*normal_end, entry, *asynchronous)
            {
                exits.push(exit);
                end = body_end(ops, &self.flow, next)?;
            }
        }
        let outer = std::mem::replace(&mut self.return_exits, exits);
        let body = self.seeded(ops, start, end, level, seed);
        self.return_exits = outer;
        let (body, returns) = body?.pending_return()?;
        // Adjacent managers of the same kind share a header. A switch
        // between with and async with requires a nested suite.
        let mut indent = String::new();
        for group in managers.chunk_by(|a, b| a.0 == b.0) {
            let header = if group[0].0 { "async with" } else { "with" };
            let items = group
                .iter()
                .map(|(_, item)| item.as_str())
                .collect::<Vec<_>>();
            self.line(&format!("{indent}{header} {}:", items.join(", ")))?;
            indent.push_str("    ");
        }
        if body.is_empty() {
            self.line(&format!("{indent}pass"))?;
        }
        for line in body.lines() {
            self.line(&format!("{indent}{line}"))?;
        }

        let mut next = end;
        for (asynchronous, _) in managers.iter().rev() {
            next = self
                .manager_exit(ops, next..*normal_end, *asynchronous, returns)
                .ok_or("unsupported manager exit")?;
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
