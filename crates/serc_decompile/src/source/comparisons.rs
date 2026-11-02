//! Each non-final comparison keeps its right operand for the following link.
//! A false comparison discards that saved operand and bypasses the remaining
//! expressions. Value and predicate contexts have different cleanup blocks.
use std::fmt::Write as _;

use super::{Body, Instruction, PythonVersion, conditionals::Predicate};

pub(super) fn operator(op: Instruction, version: PythonVersion) -> Option<&'static str> {
    match op.name {
        "IS_OP" => ["is", "is not"].get(op.arg as usize).copied(),
        "CONTAINS_OP" => ["in", "not in"].get(op.arg as usize).copied(),
        "COMPARE_OP" => {
            let shift = if version == PythonVersion::Py312 {
                4
            } else {
                5
            };
            ["<", "<=", "==", "!=", ">", ">="]
                .get((op.arg >> shift) as usize)
                .copied()
        }
        _ => None,
    }
}

pub(super) fn is_link(ops: &[Instruction]) -> bool {
    matches!(ops, [swap, copy, cmp, ..] if (swap.name, swap.arg) == ("SWAP", 2)
        && (copy.name, copy.arg) == ("COPY", 2) && matches!(cmp.name, "COMPARE_OP" | "IS_OP" | "CONTAINS_OP"))
}

fn after_branch(ops: &[Instruction], branch: usize) -> usize {
    branch + 1 + usize::from(ops.get(branch + 1).is_some_and(|op| op.name == "NOT_TAKEN"))
}

fn guard(ops: &[Instruction], link: usize) -> Option<(usize, bool)> {
    let mut at = link + 3;
    let copied = ops
        .get(at)
        .is_some_and(|op| (op.name, op.arg) == ("COPY", 1));
    at += usize::from(copied);
    at += usize::from(ops.get(at).is_some_and(|op| op.name == "TO_BOOL"));
    (ops.get(at)?.name == "POP_JUMP_IF_FALSE").then_some((at, copied))
}

fn same_instructions(left: &[Instruction], right: &[Instruction]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.opcode == b.opcode && a.arg == b.arg && a.jump == b.jump)
}

impl Body<'_, '_, '_> {
    pub(super) fn comparison_chain(
        &mut self,
        ops: &[Instruction],
        first: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let ops = &ops[..end];
        let (branch, value) = guard(ops, first).ok_or("unsupported comparison guard")?;
        let cleanup = self.flow.target(branch)?;
        if cleanup <= branch || cleanup >= end || !self.flow.entered_from(cleanup, first..cleanup) {
            return Err("invalid comparison cleanup".into());
        }
        let right = self.expr()?;
        let left = self.expr()?;
        let cmp = operator(ops[first + 2], self.m.version).ok_or("unknown comparison")?;
        let mut chain = format!("({left} {cmp} {right}");
        let mut branch = branch;
        loop {
            let mut operand = after_branch(ops, branch);
            if value {
                if ops.get(operand).is_none_or(|op| op.name != "POP_TOP") {
                    return Err("missing comparison truth discard".into());
                }
                operand += 1;
            }
            let mut next_link = None;
            for next in operand..cleanup {
                self.charge(1)?;
                if is_link(&ops[next..cleanup])
                    && let Some((guard, copied)) = guard(ops, next)
                    && copied == value
                    && self.flow.jump(guard) == Some(cleanup)
                {
                    next_link = Some((next, guard));
                    break;
                }
            }
            if let Some((next, next_branch)) = next_link {
                let rhs = self.fragment(ops, operand, next, level)?.expression()?;
                let cmp = operator(ops[next + 2], self.m.version).ok_or("unknown comparison")?;
                write!(chain, " {cmp} {rhs}").unwrap();
                branch = next_branch;
                continue;
            }
            if value {
                let (tail, join) = self.value_continuation(ops, cleanup)?;
                let last = tail
                    .checked_sub(1)
                    .filter(|&n| n >= operand)
                    .ok_or("missing final comparison")?;
                self.finish_comparison(ops, operand, last, &mut chain, level)?;
                self.push(chain)?;
                return Ok(join);
            }

            return self.comparison_predicate(ops, operand, cleanup, chain, level);
        }
    }

    fn value_continuation(
        &self,
        ops: &[Instruction],
        cleanup: usize,
    ) -> Result<(usize, usize), String> {
        let end = ops.len();
        let [swap, discard, ..] = &ops[cleanup..] else {
            return Err("incomplete comparison cleanup".into());
        };
        if (swap.name, swap.arg) != ("SWAP", 2) || discard.name != "POP_TOP" {
            return Err("unsupported comparison value cleanup".into());
        }
        let join = cleanup + 2;
        let tail = if ops[cleanup - 1].name == "JUMP_FORWARD"
            && self.flow.jump(cleanup - 1) == Some(join)
        {
            cleanup - 1
        } else {
            // CPython may duplicate a short terminal block on both paths.
            let block_end = self.flow.block(cleanup).ops.end;
            if block_end > end
                || block_end <= join
                || !matches!(ops[block_end - 1].name, "RETURN_VALUE" | "RETURN_CONST")
            {
                return Err("unsupported comparison continuation".into());
            }
            let start = cleanup
                .checked_sub(block_end - join)
                .ok_or("invalid comparison continuation")?;
            if !same_instructions(&ops[start..cleanup], &ops[join..block_end]) {
                return Err("unequal comparison continuations".into());
            }
            start
        };
        Ok((tail, join))
    }

    fn comparison_predicate(
        &mut self,
        ops: &[Instruction],
        operand: usize,
        cleanup: usize,
        mut chain: String,
        level: usize,
    ) -> Result<usize, String> {
        let end = ops.len();
        if ops[cleanup].name != "POP_TOP" {
            return Err("unsupported comparison predicate cleanup".into());
        }
        let last = (operand..cleanup)
            .find(|&i| {
                operator(ops[i], self.m.version).is_some()
                    && ops
                        .get(i + 1)
                        .is_some_and(|op| op.name == "POP_JUMP_IF_FALSE")
            })
            .ok_or("missing final comparison branch")?;
        self.finish_comparison(ops, operand, last, &mut chain, level)?;
        let branch = last + 1;
        let target = self.flow.target(branch)?;
        let mut suite = after_branch(ops, branch);
        let discarded = cleanup + 1;
        if target >= end
            || discarded >= end
            || !(target == discarded
                || self.flow.jump(discarded) == Some(target)
                || same_instructions(
                    &ops[discarded..self.flow.block(discarded).ops.end.min(end)],
                    &ops[target..self.flow.block(target).ops.end.min(end)],
                ))
        {
            return Err("comparison failures have different destinations".into());
        }
        if suite < end && ops[suite].name == "JUMP_FORWARD" {
            suite = self.flow.target(suite)?;
        }
        if suite < cleanup && matches!(ops[cleanup - 1].name, "RETURN_CONST" | "RETURN_VALUE") {
            let body = self.fragment(ops, suite, cleanup, level)?.suite()?;
            self.block(&format!("if {chain}:"), &body)?;
            return Ok(target);
        }
        let suite_end = if suite < cleanup { cleanup } else { target };
        self.conditional_suites(
            ops,
            &Predicate {
                value: chain,
                fallthrough_true: true,
                branch,
                suite,
                target,
            },
            suite_end,
            end,
            level,
        )
    }

    fn finish_comparison(
        &mut self,
        ops: &[Instruction],
        operand: usize,
        last: usize,
        chain: &mut String,
        level: usize,
    ) -> Result<(), String> {
        let cmp = operator(ops[last], self.m.version).ok_or("missing final comparison operator")?;
        let rhs = self.fragment(ops, operand, last, level)?.expression()?;
        write!(chain, " {cmp} {rhs})").unwrap();
        Ok(())
    }
}
