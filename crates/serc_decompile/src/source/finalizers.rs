//! Match a finally suite's normal copy against its out-of-line exception path.
use std::ops::Range;

use serc_bytecode::instructions::Handler;

use super::Instruction;
use crate::flow::Graph;

/// Monitoring markers can split a single finalizer's unwind range.
pub(super) fn owns_cleanup(
    ops: &[Instruction],
    flow: &Graph,
    suite: Range<usize>,
    unwind: Handler,
) -> bool {
    unwind.end == ops[suite.end].offset
        || flow
            .blocks_covering(suite)
            .all(|b| match flow.handler(b.ops.start) {
                Some(h) => {
                    (h.target, h.depth, h.lasti) == (unwind.target, unwind.depth, unwind.lasti)
                }
                None => ops[b.ops.clone()].iter().all(|op| op.name == "NOT_TAKEN"),
            })
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

/// Return the first normal instruction after the copy. Corresponding jumps
/// must still land at corresponding instruction boundaries after expansion.
pub(super) fn copy_end(
    ops: &[Instruction],
    flow: &Graph,
    normal: Range<usize>,
    exceptional: Range<usize>,
) -> Option<usize> {
    let branching = ops[exceptional.clone()].iter().any(|op| op.jump.is_some());
    let mut boundaries = Vec::new();
    let mut jumps = Vec::new();
    let mut left = steps(ops, normal.clone());
    for b in steps(ops, exceptional.clone()) {
        let a = left.next()?;
        if a.name != b.name {
            return None;
        }
        if branching && a.first && b.first {
            boundaries.push((a.index, b.index));
        }
        if ops[b.index].jump.is_some() {
            jumps.push((flow.jump(a.index)?, flow.jump(b.index)?));
        } else if a.arg != b.arg {
            return None;
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
