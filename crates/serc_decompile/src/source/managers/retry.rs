//! A manager can finish with a retry loop whose only exit is exceptional.
//! Recover its outer handler even when a nested try interrupts the body.
use super::{Body, Handler, Instruction, body_end, boundary, is_manager_handler, manager_entry};
use crate::source::{comprehensions, dispatch, partial::GapKind, regions::Fragment};
use rustc_hash::FxHashSet;
use std::ops::Range;

fn control_only(ops: &[Instruction]) -> bool {
    ops.iter().all(|op| {
        matches!(op.name, "NOP" | "NOT_TAKEN")
            || serc_bytecode::target::is_unconditional_jump(op.opcode)
            || op.name.starts_with("POP_JUMP_IF_")
    })
}

impl Body<'_, '_, '_> {
    pub(super) fn managed_retry(
        &mut self,
        ops: &[Instruction],
        end: usize,
        manager: Handler,
        level: usize,
    ) -> Result<Option<String>, String> {
        if ops.get(end).is_none_or(|op| op.name != "NOP") {
            return Ok(None);
        }
        let Some(start) = (end..ops.len())
            .take_while(|&i| i == end || ops[i - 1].name == "NOP")
            .find(|&i| self.flow.handler(i).is_some_and(|h| !h.lasti))
        else {
            return Ok(None);
        };
        let entry = self.flow.handler(start).unwrap();
        let target = boundary(ops, entry.target)?;
        let Some((_, unwind)) = dispatch::unwind(ops, &self.flow, target, entry.depth + 1) else {
            return Ok(None);
        };
        let owns =
            |h: Handler| (h.target, h.depth, h.lasti) == (manager.target, manager.depth, true);
        if start >= target
            || entry.start != ops[start].offset
            || entry.depth != manager.depth
            || ops[target].name != "PUSH_EXC_INFO"
            || !self.flow.handler(unwind).is_some_and(owns)
        {
            return Ok(None);
        }
        let Some((arm, jump)) = self.retry_arm(ops, target, unwind) else {
            return Ok(None);
        };
        if !self.flow.handler(jump).is_some_and(owns) {
            return Ok(None);
        }
        let mut header = self.flow.target(jump)?;
        // 3.12 and 3.13 can thread the handler through the normal latch.
        if ops[header].name == "JUMP_BACKWARD" && self.flow.handler(header).is_some_and(owns) {
            header = self.flow.target(header)?;
        }
        if !(end.saturating_sub(1)..start).contains(&header)
            || !ops[header..start].iter().all(|op| op.name == "NOP")
            || self.flow.block(header).height != Some(manager.depth)
            || !self.flow.incoming(header).any(|i| {
                (start..target).contains(&i)
                    && ops[i].name == "JUMP_BACKWARD"
                    && self.flow.handler(i).is_some_and(owns)
            })
            || !self.retry_stays_in_manager(ops, header..target, manager)?
        {
            return Ok(None);
        }
        let cut = self.retry_prefix(ops, start..target, entry, header, manager);

        let complete = ops[cut].name == "JUMP_BACKWARD"
            && self.flow.jump(cut) == Some(header)
            && self.flow.handler(cut).is_some_and(owns)
            && self.flow.normal_end([start]) == cut + 1;
        if cut == start || !complete && !self.work.partial {
            return Ok(None);
        }
        // We render this handler below. Nested protected suites still use
        // their usual entries while the body is being lifted.
        let placed = self.handlers.remove(&start);
        let outer = self.active_try.replace(entry);
        let outer_loop = self.loop_edges.replace(crate::source::loops::Loop::retry(
            header,
            cut,
            self.loop_edges,
        ));
        let prefix = self.fragment(ops, start, cut, level);
        self.loop_edges = outer_loop;
        self.active_try = outer;
        if let Some(placed) = placed {
            self.handlers.insert(start, placed);
        }
        let Ok(mut body) = prefix.and_then(Fragment::suite) else {
            return Ok(None);
        };
        let pos = self.lines.len();
        if self.emit_arm(ops, &arm, level, true).is_err() {
            self.lines.truncate(pos);
            return Ok(None);
        }
        let handler = self.lines.split_off(pos);
        if !complete {
            self.gap(
                "unplaced control flow in retry loop",
                ops,
                cut..target,
                GapKind::ProtectedTail,
            )?;
            body.push_str(&self.lines.split_off(pos));
        }
        self.block("try:", &body)?;
        let iteration = self.lines.split_off(pos);
        self.block("while True:", &(iteration + &handler))?;
        Ok(Some(self.lines.split_off(pos)))
    }

    /// Follow both normal and exceptional paths. A break into the manager's
    /// normal exit or another statement after the loop invalidates this shape.
    fn retry_stays_in_manager(
        &mut self,
        ops: &[Instruction],
        span: Range<usize>,
        manager: Handler,
    ) -> Result<bool, String> {
        let mut seen = FxHashSet::default();
        let mut work = vec![span.start];
        while let Some(at) = work.pop() {
            if !span.contains(&at) {
                return Ok(false);
            }
            if !seen.insert(at) {
                continue;
            }
            self.charge(1)?;
            let block = self.flow.block(at);
            let last = block.ops.end - 1;
            let h = self.flow.handler(at);
            if let Some(h) = h {
                if h.depth < manager.depth {
                    return Ok(false);
                }
                if h.target != manager.target && h.target != ops[span.end].offset {
                    work.push(boundary(ops, h.target)?);
                }
            }
            // Outside the try and its nested handlers only tracing, tests and
            // transfers may run. In particular, a normal __exit__ is a call.
            if h.is_none_or(|h| h.target == manager.target)
                && !control_only(&ops[block.ops.clone()])
            {
                return Ok(false);
            }
            work.extend(
                [self.flow.jump(last), self.flow.fallthrough(block)]
                    .into_iter()
                    .flatten(),
            );
        }
        Ok(true)
    }

    /// A prefix ends on an empty operand stack with every normal branch
    /// accounted for, including backedges into a comprehension or inner loop.
    fn retry_prefix(
        &self,
        ops: &[Instruction],
        span: Range<usize>,
        owner: Handler,
        header: usize,
        manager: Handler,
    ) -> usize {
        let mut cut = span.start;
        let mut required = span.start;
        let mut child_end = span.start;
        let mut managed_until = span.start;
        for block in self.flow.blocks_covering(span.clone()) {
            let at = block.ops.start;
            if at < managed_until {
                continue;
            }
            let last = block.ops.end - 1;
            if self.flow.handler(at).is_some_and(|h| {
                (h.target, h.depth, h.lasti) == (manager.target, manager.depth, true)
            }) && control_only(&ops[block.ops.clone()])
                && (self.normal_join(ops, last) == header
                    || ops[last].name == "NOP"
                        && self
                            .flow
                            .fallthrough(block)
                            .is_some_and(|to| self.normal_join(ops, to) == header))
            {
                if required <= at && self.flow.jump(last) == Some(header) {
                    cut = at;
                }
                continue;
            }
            if at >= child_end {
                if let Some(h) = self.flow.handler(at) {
                    if (h.target, h.depth, h.lasti) != (owner.target, owner.depth, false)
                        && comprehensions::restore_under(ops, &self.flow, h, owner).is_none()
                    {
                        if let Some(next) = self.manager_in_retry(ops, at, h, owner, span.end) {
                            managed_until = next;
                            required = required.max(next);
                            continue;
                        }
                        let Some(next) = self.enclosed_try_end(ops, at, owner) else {
                            break;
                        };
                        child_end = next;
                        required = required.max(next);
                    }
                } else if !control_only(&ops[block.ops.clone()]) {
                    break;
                }
            }
            if let Some(to) = self.flow.jump(last) {
                if to < span.start {
                    break;
                }
                required = required.max(to);
            }
            for from in self.flow.incoming(at) {
                if from > at && self.handler_jumps_from(ops, span.start..at, from, at) {
                    continue;
                }
                required = required.max(from + 1);
            }
            let end = block.ops.end;
            if end < span.end && end >= required && self.flow.block(end).height == Some(owner.depth)
            {
                cut = end;
            }
        }
        cut
    }

    /// The manager lifter owns this item through its normal exit. Suppression
    /// must reach the same continuation, possibly through a second loop latch.
    fn manager_in_retry(
        &self,
        ops: &[Instruction],
        at: usize,
        entry: Handler,
        owner: Handler,
        limit: usize,
    ) -> Option<usize> {
        if !entry.lasti
            || entry.start != ops[at].offset
            || entry.depth <= owner.depth
            || !is_manager_handler(ops, &entry).ok()?
        {
            return None;
        }
        let enter = manager_entry(&ops[..at], false).ok()?;
        if !self.flow.entered_from(at, enter..at) {
            return None;
        }
        let end = body_end(ops, &self.flow, entry).ok()?;
        let (exit, _) = self
            .manager_body_tail(ops, end, entry, Some(false), limit)
            .ok()?;
        let next = self.manager_exit(ops, exit..limit, false, false)?;
        let suppressed = self.after_with_suppression(ops, &entry).ok()?;
        let parent = self.flow.handler(exit)?;
        (next < limit
            && suppressed < ops.len()
            && (parent.target, parent.depth, parent.lasti)
                == (owner.target, owner.depth, owner.lasti)
            && self.normal_join(ops, next) == self.normal_join(ops, suppressed))
        .then_some(next)
    }

    /// A suppression jump and a normal exit can share a second jump back to
    /// the loop. Compare their destinations without following a whole cycle.
    fn normal_join(&self, ops: &[Instruction], mut at: usize) -> usize {
        for _ in 0..2 {
            if !serc_bytecode::target::is_unconditional_jump(ops[at].opcode) {
                break;
            }
            let Some(to) = self.flow.jump(at) else { break };
            at = to;
        }
        at
    }
}
