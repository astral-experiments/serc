//! Recover context manager entries, protected bodies, and suppression exits.
use super::{Body, Bytecode, Instruction, Value, exceptions::boundary};
use crate::flow::Graph;
use rustc_hash::FxHashMap;
use serc_bytecode::instructions::Handler;
use serc_bytecode::target::operands::SpecialMethod;
use std::ops::Range;

pub(super) fn body_end(ops: &[Instruction], flow: &Graph, entry: Handler) -> Result<usize, String> {
    let mut end = boundary(ops, entry.end)?;
    while ops[end].name == "YIELD_VALUE"
        && let Some(suspend) = flow.handler(end)
        && suspend.start == ops[end].offset
        && !suspend.lasti
        && ops[boundary(ops, suspend.target)?].name == "CLEANUP_THROW"
        && ops.get(end + 1).is_some_and(|op| op.name == "RESUME")
        && let Some(resume) = flow.handler(end + 1)
        && suspend.end == ops[end + 1].offset
        && resume.start == suspend.end
        && (resume.target, resume.depth, resume.lasti) == (entry.target, entry.depth, entry.lasti)
    {
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
        let next = match self.manager_group(ops, enter, entry, level, &mut normal_end)? {
            Some(next) => next,
            None => {
                let next = self.after_with_suppression(ops, &entry)?;
                if next <= enter || next >= ops.len() {
                    return Err("unsupported continuation after with".into());
                }
                if next > end {
                    self.charge(ops.len())?;
                    if self.none_return(&ops[next..self.flow.normal_end([next])]) {
                        return Ok(Some(end));
                    }
                }
                next
            }
        };
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
        let asynchronous = awaited(&entry)?;
        let mut managers = Vec::new();
        let mut previous_body = None;
        let mut seed = Vec::new();
        let (start, end) = loop {
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
            managers.push(item);
            seed = loaded;

            // Each additional item starts where the enclosing protected range
            // ends. The enclosing handler resumes protection during inner cleanup.
            if let Some(inner) = self.flow.handler(end)
                && inner.start == ops[end].offset
                && inner.depth > entry.depth
                && inner.lasti
                && is_manager_handler(ops, &inner)?
                && awaited(&inner)? == asynchronous
            {
                previous_body = Some(body_start);
                entry = inner;
            } else {
                break (body_start, end);
            }
        };
        let (body, returns) = self
            .seeded(ops, start, end, level, seed)?
            .pending_return()?;
        let header = if asynchronous { "async with" } else { "with" };
        self.block(&format!("{header} {}:", managers.join(", ")), &body)?;

        let mut next = end;
        for _ in &managers {
            let exit = ops[next..*normal_end]
                .iter()
                .position(|op| op.name == "CALL")
                .map(|n| next + n)
                .ok_or("unsupported manager exit")?;
            let exit = if asynchronous {
                self.manager_await_end(ops, exit + 1, 2)?
            } else {
                exit
            };
            if exit + 1 >= *normal_end || ops[exit + 1].name != "POP_TOP" {
                return Err("unsupported manager result".into());
            }
            next = exit + 2;
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
}
