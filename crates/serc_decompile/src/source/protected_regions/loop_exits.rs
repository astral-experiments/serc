//! The normal and exceptional exits of a try can both rejoin an enclosing
//! iterator or while loop. Protection boundaries separate the return value
//! and the handler's saved exception from the loop's own stack slots.
use super::{Body, ExitCopies, FxHashSet, Graph, Instruction, Range};

/// The last guard of a try may skip to another copy of the while latch.
pub(super) fn copied_latch(
    ops: &[Instruction],
    flow: &Graph,
    protected: Range<usize>,
    next: usize,
) -> Option<ExitCopies> {
    let end = protected.end;
    let header = flow.jump(end)?;
    (next == end
        && ops[end].name == "JUMP_BACKWARD"
        && header < protected.start
        && ops.get(end + 1)?.name == "JUMP_BACKWARD"
        && flow.jump(end + 1) == Some(header)
        && flow.entered_from(end + 1, protected)
        && flow.handler(end) == flow.handler(end + 1))
    .then_some(ExitCopies {
        first: end,
        end: end + 2,
        width: 1,
    })
}

/// Only the successful try enters this tail. The handled paths rejoin the
/// iterator, while the normal path runs the else suite before the same latch.
pub(super) fn iterator_success_latch(
    owner: &Body<'_, '_, '_>,
    ops: &[Instruction],
    protected: Range<usize>,
    header: usize,
) -> Option<usize> {
    let flow = &owner.flow;
    let latch = flow.jump(header)?.checked_sub(1)?;
    let success = protected.end..latch;
    if success.is_empty()
        || ops[latch].name != "JUMP_BACKWARD"
        || flow.jump(latch) != Some(header)
        || flow.block(success.start).ops.start != success.start
    {
        return None;
    }
    // An out-of-line CLEANUP_THROW can re-enter the else at its own END_SEND.
    let throws: FxHashSet<_> = success
        .clone()
        .filter(|&i| ops[i].name == "YIELD_VALUE")
        .filter_map(|i| owner.delegated_cleanup(ops, i).map(|cleanup| cleanup + 1))
        .collect();
    for block in flow.blocks_covering(success.clone()) {
        let at = block.ops.start;
        if !flow.incoming(at).all(|source| {
            success.contains(&source)
                || throws.contains(&source)
                || at == success.start && protected.contains(&source)
        }) {
            return None;
        }
        if [flow.jump(block.ops.end - 1), flow.fallthrough(block)]
            .into_iter()
            .flatten()
            .any(|to| !(success.start..=latch).contains(&to) && to != header)
        {
            return None;
        }
    }
    Some(latch)
}

/// A guarded return evaluates its value under protection but returns it
/// outside the table. In a for loop the same block first drops the iterator.
/// The other arm skips the whole epilogue and continues after the try.
pub(super) fn guarded_return_end(
    ops: &[Instruction],
    flow: &Graph,
    suite: Range<usize>,
    dispatch: usize,
    depth: usize,
) -> Option<usize> {
    let block = flow.block(suite.end);
    let next = block.ops.end;
    if block.ops.start != suite.end || next >= dispatch {
        return None;
    }
    let skipped = flow.incoming(next).any(|i| {
        suite.contains(&i) && ops[i].name.starts_with("POP_JUMP_IF_") && flow.jump(i) == Some(next)
    });
    if depth == 0 {
        return (skipped
            && block.height == Some(1)
            && flow.handler(suite.end).is_none()
            && matches!(&ops[block.ops.clone()], [ret] if ret.name == "RETURN_VALUE"))
        .then_some(next);
    }
    let header = |i: usize| {
        i < suite.start && ops[i].name == "FOR_ITER" && flow.for_iter_depth(i) == Some(depth)
    };
    // When both arms return, the handler owns the only edge back to the
    // iterator. Otherwise the skipped return must lead to this loop's latch.
    (iterator_return_block(ops, flow, block.ops.clone(), depth)
        && ((ops[next].name == "END_FOR" && flow.incoming(next).any(header))
            || (skipped && flow.jump(next).is_some_and(header))))
    .then_some(next)
}

pub(in crate::source) fn iterator_return_block(
    ops: &[Instruction],
    flow: &Graph,
    span: Range<usize>,
    depth: usize,
) -> bool {
    depth > 0
        && flow.block(span.start).ops == span
        && flow.block(span.start).height == Some(depth + 1)
        && flow.handler(span.start).is_none()
        && matches!(&ops[span], [swap, pops @ .., ret] if (swap.name, swap.arg as usize) == ("SWAP", depth + 1)
            && pops.len() == depth && pops.iter().all(|op| op.name == "POP_TOP") && ret.name == "RETURN_VALUE")
}
