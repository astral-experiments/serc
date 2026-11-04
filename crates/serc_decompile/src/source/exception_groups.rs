//! Follow each except* match. Normal completion and a new exception in the
//! handler both continue to the next match with the remaining subgroup.
use std::ops::Range;

use serc_bytecode::target::operands::BinaryIntrinsic;

use super::{Body, Instruction, dispatch, regions};
use crate::flow::Graph;

struct Arm {
    kind: Range<usize>,
    binding: usize,
    body: Range<usize>,
}

struct Group {
    arms: Vec<Arm>,
    success: Range<usize>,
    continuation: Range<usize>,
}

// CPython can copy LIST_APPEND 1 into each predecessor of PREP_RERAISE_STAR.
// Older targets can leave a jump to another jump instead.
pub(super) fn next_match(
    ops: &[Instruction],
    flow: &Graph,
    mut at: usize,
    limit: usize,
) -> Option<(usize, bool)> {
    let mut appended = false;
    while at < limit {
        match (ops[at].name, ops[at].arg) {
            ("JUMP_FORWARD", _) => at = flow.jump(at).filter(|&next| next > at)?,
            ("LIST_APPEND", 1) if !appended => {
                appended = true;
                at += 1;
            }
            _ => return Some((at, appended)),
        }
    }
    None
}

impl Body<'_, '_, '_> {
    fn group_clear(&self, ops: &[Instruction], at: usize, binding: Instruction) -> Option<usize> {
        if binding.name == "POP_TOP" {
            return Some(at);
        }
        let store = self.cleared_binding(ops, at)?;
        ((store.name, store.arg) == (binding.name, binding.arg)).then_some(at + 3)
    }

    fn group_arm(
        &self,
        ops: &[Instruction],
        kind: usize,
        cleanup: usize,
    ) -> Option<(Arm, usize, bool)> {
        let flow = &self.flow;
        let branch = flow.block(kind).ops.end.checked_sub(1)?;
        let check = branch.checked_sub(2)?;
        if check <= kind
            || ops[check].name != "CHECK_EG_MATCH"
            || (ops[check + 1].name, ops[check + 1].arg) != ("COPY", 1)
            || ops[branch].name != "POP_JUMP_IF_NONE"
        {
            return None;
        }
        let miss = flow.jump(branch)?;
        let binding = branch + 1 + usize::from(ops.get(branch + 1)?.name == "NOT_TAKEN");
        if binding >= miss || miss >= cleanup || ops[miss].name != "POP_TOP" {
            return None;
        }
        let destination = next_match(ops, flow, miss + 1, cleanup)?;
        let start = binding + 1;
        let mut end = start;
        let mut raises = false;
        if let Some(h) = flow.handler(start).filter(|h| h.depth == 4 && h.lasti) {
            end = ops.binary_search_by_key(&h.end, |op| op.offset).ok()?;
            let error = ops.binary_search_by_key(&h.target, |op| op.offset).ok()?;
            // 3.15 can exclude a fallthrough monitoring marker from this
            // range. Both neighboring intervals must unwind to the same arm.
            while end + 1 < error
                && ops[end].name == "NOT_TAKEN"
                && flow.handler(end).is_none()
                && let Some(resumed) = flow.handler(end + 1)
                && (resumed.target, resumed.depth, resumed.lasti) == (h.target, h.depth, h.lasti)
            {
                end = ops
                    .binary_search_by_key(&resumed.end, |op| op.offset)
                    .ok()?;
            }
            if !(start < end && end <= error && error < miss) {
                return None;
            }
            let append = self.group_clear(ops, error, ops[binding])?;
            if !matches!(ops.get(append..append+2)?, [list,pop] if (list.name,list.arg) == ("LIST_APPEND",3) && pop.name == "POP_TOP")
                || next_match(ops, flow, append + 2, cleanup)? != destination
            {
                return None;
            }
            raises = end == error && ops[end - 1].name == "RAISE_VARARGS";
        }
        if !raises {
            let next = self.group_clear(ops, end, ops[binding])?;
            if next_match(ops, flow, next, cleanup)? != destination {
                return None;
            }
        }
        Some((
            Arm {
                kind: kind..check,
                binding,
                body: start..end,
            },
            destination.0,
            destination.1,
        ))
    }

    fn group_dispatch(
        &self,
        ops: &[Instruction],
        normal: Range<usize>,
        pending: bool,
    ) -> Option<Group> {
        let dispatch = normal.end;
        let (_, cleanup) = dispatch::unwind(ops, &self.flow, dispatch, 1)?;
        if !matches!(ops.get(dispatch+1..dispatch+3)?, [list,copy] if (list.name,list.arg) == ("BUILD_LIST",0)
            && (copy.name,copy.arg) == ("COPY",2))
        {
            return None;
        }
        let mut arms = Vec::new();
        let mut at = dispatch + 3;
        loop {
            let (arm, next, last) = self.group_arm(ops, at, cleanup)?;
            if next <= at {
                return None;
            }
            arms.push(arm);
            at = next;
            if last {
                break;
            }
        }
        // PREP_RERAISE_STAR merges new exceptions with the unmatched remainder.
        // Only the None result reaches the successful continuation.
        if !matches!(ops.get(at..at+3)?, [prep,copy,test] if (prep.name,prep.arg) == ("CALL_INTRINSIC_2",BinaryIntrinsic::PREP_RERAISE_STAR.argument())
            && (copy.name,copy.arg) == ("COPY",1) && test.name == "POP_JUMP_IF_NOT_NONE")
        {
            return None;
        }
        let unmatched = self.flow.jump(at + 2)?;
        let pop = at + 3 + usize::from(ops.get(at + 3)?.name == "NOT_TAKEN");
        if !matches!(ops.get(pop..pop+2)?, [discard, restore] if discard.name == "POP_TOP" && restore.name == "POP_EXCEPT")
            || !matches!(ops.get(unmatched..cleanup)?, [swap,restore,raise] if (swap.name,swap.arg) == ("SWAP",2)
                && restore.name == "POP_EXCEPT" && (raise.name,raise.arg) == ("RERAISE",0))
        {
            return None;
        }
        let handled = pop + 2..unmatched;
        let mut success = if pending {
            if !matches!(&ops[normal.clone()], [ret] if ret.name == "RETURN_VALUE") {
                return None;
            }
            dispatch..dispatch
        } else {
            normal.clone()
        };
        let continuation = if matches!(ops.get(handled.clone())?, [jump] if jump.name.starts_with("JUMP_BACKWARD"))
        {
            let join = self.flow.jump(handled.start)?;
            if pending || !normal.contains(&join) {
                return None;
            }
            success.end = join;
            join..dispatch
        } else {
            let (load, value) = regions::constant_return(ops, unmatched)?;
            if load != handled.start {
                return None;
            }
            if let Some((start, normal_value)) = regions::constant_return(ops, dispatch)
                && success.contains(&start)
                && normal_value == value
            {
                success.end = start;
            }
            handled
        };
        Some(Group {
            arms,
            success,
            continuation,
        })
    }

    pub(super) fn exception_group(
        &mut self,
        ops: &[Instruction],
        normal: Range<usize>,
        pending: bool,
    ) -> Result<bool, String> {
        let Some(group) = self.group_dispatch(ops, normal, pending) else {
            return Ok(false);
        };
        for arm in group.arms {
            let header = self.except_header(ops, "except*", Some(arm.kind), arm.binding, 0)?;
            let body = self
                .fragment(ops, arm.body.start, arm.body.end, 0)?
                .suite()?;
            self.block(&header, &body)?;
        }
        if !group.success.is_empty() {
            let body = self
                .fragment(ops, group.success.start, group.success.end, 0)?
                .suite()?;
            self.block("else:", &body)?;
        }
        self.region(ops, group.continuation.start, group.continuation.end, 0)?;
        Ok(true)
    }
}
