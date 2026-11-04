//! Match a finally suite's normal copy against its out-of-line exception path.
use std::ops::Range;

use serc_bytecode::instructions::Handler;
use serc_bytecode::target::operands315::ImportMode;

use super::{Body, Instruction};

/// Monitoring markers can split a single finalizer's unwind range.
pub(super) fn owns_cleanup(
    ops: &[Instruction],
    owner: &Body<'_, '_, '_>,
    suite: Range<usize>,
    unwind: Handler,
) -> bool {
    let flow = &owner.flow;
    unwind.end == ops[suite.end].offset
        || flow
            .blocks_covering(suite)
            .all(|b| match flow.handler(b.ops.start) {
                Some(h) => {
                    (h.target, h.depth, h.lasti) == (unwind.target, unwind.depth, unwind.lasti)
                        || owner
                            .delegated_cleanup(ops, b.ops.start)
                            .and_then(|at| flow.handler(at))
                            .is_some_and(|parent| {
                                (parent.target, parent.depth, parent.lasti)
                                    == (unwind.target, unwind.depth, unwind.lasti)
                            })
                }
                None => ops[b.ops.clone()].iter().all(|op| op.name == "NOT_TAKEN"),
            })
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CopyExit {
    Fallthrough,
    ReturnNone,
    PendingReturn,
}

struct Step {
    index: usize,
    first: bool,
    name: &'static str,
    arg: u32,
}

/// A source-level local access can be checked on unwind, borrowed on success,
/// and fused with a neighboring load or store on either path.
fn steps(ops: &[Instruction], indices: Range<usize>) -> impl Iterator<Item = Step> + '_ {
    indices.flat_map(move |index| {
        let op = ops[index];
        let (first, second) = match op.name {
            "LOAD_FAST_LOAD_FAST" | "LOAD_FAST_BORROW_LOAD_FAST_BORROW" => {
                ("LOAD_FAST", Some("LOAD_FAST"))
            }
            "STORE_FAST_LOAD_FAST" => ("STORE_FAST", Some("LOAD_FAST")),
            "STORE_FAST_STORE_FAST" => ("STORE_FAST", Some("STORE_FAST")),
            "LOAD_FAST_BORROW" | "LOAD_FAST_CHECK" => ("LOAD_FAST", None),
            name => (name, None),
        };
        std::iter::once(Step {
            index,
            first: true,
            name: first,
            arg: if second.is_some() {
                op.arg >> 4
            } else {
                op.arg
            },
        })
        .chain(second.map(move |name| Step {
            index,
            first: false,
            name,
            arg: op.arg & 15,
        }))
    })
}

impl Body<'_, '_, '_> {
    pub(super) fn implicit_reraise(&self, ops: &[Instruction], i: usize) -> bool {
        (ops[i].name, ops[i].arg) == ("RERAISE", 0)
            && self.finally_unwind.is_some_and(|target| {
                self.flow
                    .handler(i)
                    .is_some_and(|h| h.target == target && h.lasti)
            })
    }

    /// Return the first normal instruction after the copy. Corresponding jumps
    /// must still land at corresponding instruction boundaries after expansion.
    pub(super) fn finally_copy_end(
        &self,
        ops: &[Instruction],
        normal: Range<usize>,
        exceptional: Range<usize>,
        exit: CopyExit,
    ) -> Option<usize> {
        let branching = ops[exceptional.clone()].iter().any(|op| op.jump.is_some());
        let mut boundaries = Vec::new();
        let mut jumps = Vec::new();
        // A delegated throw may resume END_SEND inline on one path and
        // through a separate cleanup block on the other.
        let logical = |step: &Step| {
            !(step.name == "CLEANUP_THROW"
                && step
                    .index
                    .checked_sub(3)
                    .and_then(|y| self.delegated_cleanup(ops, y))
                    == Some(step.index)
                && ops
                    .get(step.index + 1)
                    .is_some_and(|op| op.name == "END_SEND"))
        };
        let mut left = steps(ops, normal.clone()).filter(logical);
        for b in steps(ops, exceptional.clone()).filter(logical) {
            let a = left.next()?;
            if branching && a.first && b.first {
                boundaries.push((a.index, b.index));
            }
            // At function exit the same arm returns None on success and resumes
            // the active exception on unwind. Explicit source returns still
            // have to match on both paths.
            if exit != CopyExit::Fallthrough && (b.name, b.arg) == ("RERAISE", 0) {
                if exit == CopyExit::PendingReturn {
                    if a.name != "RETURN_VALUE" {
                        return None;
                    }
                    continue;
                }
                let width = if a.name == "RETURN_CONST" { 1 } else { 2 };
                if !a.first
                    || a.index + width > normal.end
                    || !self.none_return(&ops[a.index..a.index + width])
                {
                    return None;
                }
                if width == 2 {
                    left.next()?;
                }
                continue;
            }
            if a.name != b.name {
                return None;
            }
            if ops[b.index].jump.is_some() {
                jumps.push((self.flow.jump(a.index)?, self.flow.jump(b.index)?));
            } else if a.arg != b.arg {
                match a.name {
                    "YIELD_VALUE"
                        if self.m.version == super::PythonVersion::Py312
                            && self.delegated_cleanup(ops, a.index).is_some()
                            && self.delegated_cleanup(ops, b.index).is_some() => {}
                    // A module-level import is eager inside an exception
                    // handler, even when its normal copy uses the default.
                    "IMPORT_NAME"
                        if self.depth == 0
                            && self.m.version == super::PythonVersion::Py315
                            && ImportMode::DEFAULT.encode_name(a.arg >> 2) == a.arg
                            && ImportMode::EAGER.encode_name(a.arg >> 2) == b.arg => {}
                    _ => return None,
                }
            }
        }
        let end = match left.next() {
            None => normal.end,
            Some(step) if step.first => step.index,
            Some(_) => return None,
        };
        if jumps.is_empty() {
            return Some(end);
        }
        boundaries.push((end, exceptional.end));
        jumps
            .iter()
            .all(|jump| boundaries.binary_search(jump).is_ok())
            .then_some(end)
    }
}
