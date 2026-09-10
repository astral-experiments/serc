use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    ExceptionExclusion, ExceptionPolicy, ExceptionRegion, Instruction, Item, JUMP_BACKWARD,
    JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, Label, Operand, OptimizationPipeline, SourceLocation,
    block_has_fallthrough, block_jump_target, cleanup_block_creation_order, ends_scope,
};
use crate::assembler::block_graph::{BasicBlockId, BlockGraph, ItemPosition};
use crate::target::opcodes::{
    BUILD_TUPLE, CALL, END_ASYNC_FOR, END_SEND, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL, NOP,
    POP_EXCEPT, POP_TOP, RAISE_VARARGS, RETURN_VALUE, STORE_ATTR, STORE_FAST, SWAP,
};
use crate::target::{is_conditional_jump, is_unconditional_jump};

const WARM_BLOCK: u8 = 1;
const COLD_BLOCK: u8 = 2;

// Preserve a pending value while discarding the item below it. Deferred source locations
// must resolve through their own writers, which do not all honor PRESERVE.
fn is_unlocated_value_cleanup_prefix(items: &[Item]) -> bool {
    let mut instructions = items.iter().filter_map(|item| match item {
        Item::Instruction(instruction) => Some(instruction),
        Item::Label(_) => None,
    });
    let Some(first) = instructions.next() else {
        return false;
    };
    let Some(second) = instructions.next() else {
        return false;
    };
    first.opcode == SWAP
        && first.operand() == Operand::Value(2)
        && second.opcode == POP_TOP
        && instructions.next().is_none()
        && [first, second].iter().all(|instruction| {
            instruction.location.line < 0
                && !instruction.fusion_policy.location_from_next()
                && instruction.no_location_policy.percent_target_location == 0
        })
}

// Labels created by later copies have no certificate. Every original alias at an entry must
// exclude an earlier location donor before the artificial-prefix repair can suppress a seed.
fn original_entry_has_no_location_donor(items: &[Item], absent: &FxHashSet<Label>) -> bool {
    let mut has_label = false;
    for item in items {
        let Item::Label(label) = item else {
            break;
        };
        if !absent.contains(label) {
            return false;
        }
        has_label = true;
    }
    has_label
}

// A bare raise has no copied finally cleanup: it propagates through the active exception
// handler. Restrictions on try/with exits therefore do not increase this exit's original size.
// An omitted authored NOP means the optimized singleton does not certify the original size.
fn block_is_bare_raise(items: &[Item], original_nop_targets: &FxHashSet<Label>) -> bool {
    if items
        .iter()
        .any(|item| matches!(item, Item::Label(label) if original_nop_targets.contains(label)))
    {
        return false;
    }
    let mut instructions = items.iter().filter_map(|item| match item {
        Item::Instruction(instruction) => Some(instruction),
        Item::Label(_) => None,
    });
    instructions.next().is_some_and(|instruction| {
        instruction.opcode == RAISE_VARARGS
            && instruction.operand() == Operand::Value(0)
            && instruction.pre_fold_extra_instructions() == 0
    }) && instructions.next().is_none()
}

struct ExitBlockAnalysis {
    positions: Vec<usize>,
    predecessors: Vec<usize>,
    jump_predecessors: Vec<Option<BasicBlockId>>,
    conditional_predecessors: Vec<usize>,
    exception_handler_blocks: Vec<bool>,
    requires_copy: bool,
}

// Most multi-block code objects reach this pass without an eligible exit copy. Returning an
// analysis proves that the copy-producing loop below cannot run while retaining the location
// propagation performed at the end of the full pass.
fn exit_block_analysis(
    graph: &BlockGraph,
    order: &[BasicBlockId],
    exception_regions: &[ExceptionRegion],
    original_nop_targets: &FxHashSet<Label>,
) -> ExitBlockAnalysis {
    let vector_len = order
        .iter()
        .map(|block| block.index())
        .max()
        .expect("exit-block analysis requires at least one block")
        + 1;
    let mut positions = vec![usize::MAX; vector_len];
    for (position, block) in order.iter().copied().enumerate() {
        positions[block.index()] = position;
    }

    let mut exception_handler_blocks = vec![false; vector_len];
    for target in exception_regions
        .iter()
        .filter_map(|region| graph.label_block(region.target))
    {
        exception_handler_blocks[target.index()] = true;
    }
    let mut predecessors = vec![0_usize; vector_len];
    let mut jump_predecessors = vec![None; vector_len];
    let mut conditional_predecessors = vec![0; vector_len];
    predecessors[order[0].index()] = 1;
    for (position, block) in order.iter().copied().enumerate() {
        if block_has_fallthrough(graph[block].items())
            && let Some(next) = order.get(position + 1)
        {
            predecessors[next.index()] += 1;
        }
        if let Some(target) =
            block_jump_target(graph[block].items()).and_then(|label| graph.label_block(label))
        {
            predecessors[target.index()] += 1;
            conditional_predecessors[target.index()] +=
                usize::from(block_has_fallthrough(graph[block].items()));
            if jump_predecessors[target.index()].is_none() {
                jump_predecessors[target.index()] = Some(block);
            }
        }
    }

    // This cheap scan is a superset of the exact copy conditions. A false result is sufficient
    // to skip the exception-boundary and target-chain analysis.
    let possible_copy = order.iter().copied().any(|source| {
        let Some(target) =
            block_jump_target(graph[source].items()).and_then(|label| graph.label_block(label))
        else {
            return false;
        };
        let Some(target_last) = graph[target].last_instruction() else {
            return false;
        };
        let target_has_no_location = graph[target].items().iter().all(|item| {
            !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
        });
        let source_has_fallthrough = block_has_fallthrough(graph[source].items());
        let shared_exit = predecessors[target.index()] > 1
            && ((target_has_no_location && ends_scope(target_last.opcode))
                || target_last.no_location_policy.is_artificial_eval_break());
        let no_location_exit = target_has_no_location
            && !block_has_fallthrough(graph[target].items())
            && target_last.no_location_policy.allows_block_inlining()
            && (!source_has_fallthrough || predecessors[target.index()] > 1);
        let split_no_location_exit = target_has_no_location
            && block_has_fallthrough(graph[target].items())
            && order
                .get(positions[target.index()] + 1)
                .is_some_and(|next| {
                    predecessors[next.index()] == 1
                        && graph[*next].items().iter().any(|item| {
                            matches!(item, Item::Label(label) if exception_regions.iter().any(
                                |region| {
                                    region.start == *label || region.end == *label
                                }
                            ))
                        })
                });
        let small_exit_source = !source_has_fallthrough
            && (block_is_bare_raise(graph[target].items(), original_nop_targets)
                || graph[source]
                    .last_instruction()
                    .is_some_and(|instruction| instruction.jump_policy.inline_small_exit()));
        let conditional_split_return_source = source_has_fallthrough
            && predecessors[target.index()] > 1
            && graph[source].last_instruction().is_some_and(|instruction| {
                instruction.no_location_policy.copies_split_return_exit()
            });
        shared_exit
            || no_location_exit
            || split_no_location_exit
            || small_exit_source
            || conditional_split_return_source
    });
    if !possible_copy {
        return ExitBlockAnalysis {
            positions,
            predecessors,
            jump_predecessors,
            conditional_predecessors,
            exception_handler_blocks,
            requires_copy: false,
        };
    }

    // Prove that the remaining candidates are false positives. These predicates must remain a
    // conservative mirror of every copy-producing condition in `duplicate_exit_blocks`; adding a
    // new copy path there requires extending this proof before it is safe to retain the early
    // return.
    let exception_split_labels = exception_regions
        .iter()
        .flat_map(|region| [region.start, region.end])
        .collect::<FxHashSet<_>>();
    for source in order.iter().copied() {
        let Some(target) =
            block_jump_target(graph[source].items()).and_then(|label| graph.label_block(label))
        else {
            continue;
        };
        let target_position = positions[target.index()];
        let mut target_chain_end = target_position;
        while target_chain_end + 1 < order.len()
            && block_has_fallthrough(graph[order[target_chain_end]].items())
            && predecessors[order[target_chain_end + 1].index()] == 1
            && graph[order[target_chain_end + 1]].items().iter().any(
                |item| matches!(item, Item::Label(label) if exception_split_labels.contains(label)),
            )
        {
            target_chain_end += 1;
        }
        let target_blocks = &order[target_position..=target_chain_end];
        let target_exit = *target_blocks.last().expect("target chain is nonempty");
        let Some(target_last) = graph[target_exit].last_instruction() else {
            continue;
        };
        let target_pre_fusion_size = target_blocks
            .iter()
            .flat_map(|block| graph[*block].items())
            .map(|item| match item {
                Item::Instruction(instruction) => {
                    let fused_push_null = instruction.opcode == LOAD_GLOBAL
                        && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0);
                    1 + usize::from(fused_push_null)
                        + usize::from(instruction.pre_fold_extra_instructions())
                }
                Item::Label(label) => usize::from(
                    target_chain_end > target_position
                        && exception_split_labels.contains(label),
                ),
            })
            .sum::<usize>();
        let target_terminal_opcode = target_blocks
            .iter()
            .rev()
            .flat_map(|block| graph[*block].items().iter().rev())
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.opcode),
                Item::Label(_) => None,
            });
        let source_copies_split_return = graph[source]
            .last_instruction()
            .is_some_and(|instruction| instruction.no_location_policy.copies_split_return_exit());
        let target_is_small_exit = target_pre_fusion_size <= 4
            && target_terminal_opcode.is_some_and(|opcode| {
                ends_scope(opcode)
                    && (target_chain_end == target_position
                        || matches!(opcode, RAISE_VARARGS | RETURN_VALUE))
            });
        let source_has_fallthrough = block_has_fallthrough(graph[source].items());
        let inline_small_exit = !source_has_fallthrough
            && target_is_small_exit
            && ((target_chain_end == target_position
                && block_is_bare_raise(graph[target].items(), original_nop_targets))
                || graph[source]
                    .last_instruction()
                    .is_some_and(|instruction| instruction.jump_policy.inline_small_exit()))
            && !target_blocks.iter().any(|block| {
                graph[*block].items().iter().any(|item| {
                    matches!(item, Item::Instruction(instruction) if instruction.opcode == END_SEND)
                })
            })
            && !exception_handler_blocks[target.index()];
        let copy_conditional_split_return = source_has_fallthrough
            && source_copies_split_return
            && target_is_small_exit
            && predecessors[target.index()] > 1
            && !exception_handler_blocks[target.index()];
        let target_has_no_location = target_blocks.iter().all(|block| {
            graph[*block].items().iter().all(|item| {
                !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
            })
        });
        let no_location_exit = target_has_no_location
            && !block_has_fallthrough(graph[target_exit].items())
            && target_last.no_location_policy.allows_block_inlining();
        let shared_exit = predecessors[target.index()] > 1
            && ((target_has_no_location && ends_scope(target_last.opcode))
                || target_last.no_location_policy.is_artificial_eval_break());
        if inline_small_exit
            || copy_conditional_split_return
            || (!source_has_fallthrough && no_location_exit)
            || (predecessors[target.index()] > 1 && (shared_exit || no_location_exit))
        {
            return ExitBlockAnalysis {
                positions,
                predecessors,
                jump_predecessors,
                conditional_predecessors,
                exception_handler_blocks,
                requires_copy: true,
            };
        }
    }

    ExitBlockAnalysis {
        positions,
        predecessors,
        jump_predecessors,
        conditional_predecessors,
        exception_handler_blocks,
        requires_copy: false,
    }
}

// Exception boundaries and delayed inlining can leave a source-less cleanup split across
// ordinary jumps. Classify its destination before deciding whether a conditional edge needs
// a private exit copy; an ordinary continuation with source positions is still a shared join.
fn no_location_chain_reaches_exit(
    graph: &BlockGraph,
    order: &[BasicBlockId],
    positions: &[usize],
    predecessors: &[usize],
    exception_split_labels: &FxHashSet<Label>,
    mut block: BasicBlockId,
) -> bool {
    let mut visited = FxHashSet::default();
    while visited.insert(block) {
        if graph[block].items().iter().any(
            |item| matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0),
        ) {
            return false;
        }
        if let Some(last) = graph[block].last_instruction() {
            if ends_scope(last.opcode)
                || last.opcode == JUMP_BACKWARD
                || last.no_location_policy.is_artificial_eval_break()
            {
                return true;
            }
            if is_unconditional_jump(last.opcode) {
                let Some(target) = block_jump_target(graph[block].items())
                    .and_then(|label| graph.label_block(label))
                else {
                    return false;
                };
                block = target;
                continue;
            }
            if block_jump_target(graph[block].items()).is_some() {
                return false;
            }
        }
        let Some(next) = order.get(positions[block.index()] + 1) else {
            return false;
        };
        if predecessors[next.index()] != 1
            || !graph[*next].items().iter().any(
                |item| matches!(item, Item::Label(label) if exception_split_labels.contains(label)),
            )
        {
            return false;
        }
        block = *next;
    }
    false
}

// CPython copies unlocated cleanup blocks into unconditional predecessors before
// propagating locations. Conditional edges and the physical fallthrough remain as joins.
fn has_shared_cleanup_entry(
    graph: &BlockGraph,
    order: &[BasicBlockId],
    position: usize,
    conditional_predecessors: usize,
    remaining_predecessors: usize,
) -> bool {
    let fallthrough = position > 0 && block_has_fallthrough(graph[order[position - 1]].items());
    let surviving_predecessors = conditional_predecessors + usize::from(fallthrough);
    // Until every unconditional predecessor has received its own cleanup copy, it can
    // still require a different source location on the shared instructions.
    surviving_predecessors > 1 && surviving_predecessors == remaining_predecessors
}

// A shared original POP_BLOCK has no unique source line. Resolve this after copies and
// revivals: a now-unique entry must retain its predecessor's location instead.
fn restore_shared_pop_block_locations(
    graph: &mut BlockGraph,
    exception_regions: &[ExceptionRegion],
    heads: &[Label],
) {
    if heads.is_empty() {
        return;
    }
    let reachable = graph.reachable_blocks(exception_regions);
    let mut incoming = vec![0usize; reachable.len()];
    let order = graph.order().to_vec();
    if let Some(entry) = order.first() {
        incoming[entry.index()] = 1;
    }
    for (position, source) in order.iter().copied().enumerate() {
        if !reachable[source.index()] {
            continue;
        }
        let fallthrough = block_has_fallthrough(graph[source].items())
            .then(|| order.get(position + 1).copied())
            .flatten();
        let jump =
            block_jump_target(graph[source].items()).and_then(|label| graph.label_block(label));
        for target in fallthrough.into_iter().chain(jump) {
            incoming[target.index()] += 1;
        }
    }
    let handlers = exception_regions
        .iter()
        .filter_map(|region| graph.label_block(region.target))
        .collect::<FxHashSet<_>>();
    for label in heads {
        let Some(target) = graph.label_block(*label) else {
            continue;
        };
        if !reachable[target.index()]
            || incoming[target.index()] <= 1
            || handlers.contains(&target)
            || !graph[target]
                .items()
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .any(|item| matches!(item, Item::Label(current) if current == label))
        {
            continue;
        }
        if let Some(first) = graph.instructions_mut(target).next()
            && first.opcode == NOP
            && first.fusion_policy.is_converted_pop_block()
            && !first.no_location_policy.preserves_location()
            && !first.jump_policy.preserves_explicit_nop()
            && !first.jump_policy.borrow_empty_block()
        {
            first.location = SourceLocation::NONE;
        }
    }
}

// A copied cleanup can leave its dropped original in the inlining predecessor count.
// CPython recounts reachable edges before propagating lines. Repair only the unresolved
// converted POP_BLOCK heads affected by such a copy, after all copies and revivals exist.
fn restore_copied_pop_block_locations(
    graph: &mut BlockGraph,
    exception_regions: &[ExceptionRegion],
    heads: &[Label],
) {
    if heads.is_empty() {
        return;
    }
    let reachable = graph.reachable_blocks(exception_regions);
    let mut incoming = vec![(0usize, None); reachable.len()];
    let order = graph.order().to_vec();
    if let Some(entry) = order.first() {
        incoming[entry.index()].0 = 1;
    }
    for (position, source) in order.iter().copied().enumerate() {
        if !reachable[source.index()] {
            continue;
        }
        let location = graph[source]
            .items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction)
                    if instruction.location.line >= 0
                        || instruction.no_location_policy.preserves_location() =>
                {
                    Some(instruction.location)
                }
                _ => None,
            });
        let fallthrough = block_has_fallthrough(graph[source].items())
            .then(|| order.get(position + 1).copied())
            .flatten();
        let jump =
            block_jump_target(graph[source].items()).and_then(|label| graph.label_block(label));
        for target in fallthrough.into_iter().chain(jump) {
            incoming[target.index()].0 += 1;
            incoming[target.index()].1 = location;
        }
    }
    let handlers = exception_regions
        .iter()
        .filter_map(|region| graph.label_block(region.target))
        .collect::<FxHashSet<_>>();
    for label in heads {
        let Some(target) = graph.label_block(*label) else {
            continue;
        };
        if !reachable[target.index()] || handlers.contains(&target) {
            continue;
        }
        let (1, Some(location)) = incoming[target.index()] else {
            continue;
        };
        if location.line >= 0
            && let Some(first) = graph.instructions_mut(target).next()
            && first.opcode == NOP
            && first.fusion_policy.is_converted_pop_block()
            && first.location.line < 0
            && !first.no_location_policy.preserves_location()
        {
            first.location = location;
        }
    }
}

// A shared head's temporary location must not seed a now-unique copied head.
// Clear surviving shared originals before collecting predecessor locations.
fn restore_pop_block_head_locations(
    graph: &mut BlockGraph,
    exception_regions: &[ExceptionRegion],
    shared_heads: &[Label],
    copied_heads: &[Label],
) {
    restore_shared_pop_block_locations(graph, exception_regions, shared_heads);
    restore_copied_pop_block_locations(graph, exception_regions, copied_heads);
}

// Exception accounting can split an original block before a located cleanup suffix.
// A shared artificial prefix has no unique location donor in that original block.
fn unlocated_prefix_has_located_continuation(
    graph: &BlockGraph,
    order: &[BasicBlockId],
    positions: &[usize],
    predecessors: &[usize],
    exception_regions: &[ExceptionRegion],
    original_boundaries: &FxHashSet<Label>,
    mut block: BasicBlockId,
) -> bool {
    loop {
        if !block_has_fallthrough(graph[block].items())
            || graph[block].items().iter().any(|item| {
                matches!(item, Item::Instruction(instruction)
                if ends_scope(instruction.opcode)
                    || matches!(
                        instruction.operand(),
                        Operand::Forward(_) | Operand::Backward(_)
                    ))
            })
        {
            return false;
        }
        let Some(&next) = order.get(positions[block.index()] + 1) else {
            return false;
        };
        if predecessors[next.index()] != 1 {
            return false;
        }
        let mut bookkeeping_boundary = false;
        for item in graph[next].items() {
            match item {
                Item::Label(label) => {
                    if original_boundaries.contains(label) {
                        return false;
                    }
                    bookkeeping_boundary |= exception_regions
                        .iter()
                        .any(|region| region.start == *label || region.end == *label);
                }
                Item::Instruction(instruction) => {
                    if !bookkeeping_boundary {
                        return false;
                    }
                    if instruction.location.line >= 0 {
                        return true;
                    }
                    if ends_scope(instruction.opcode)
                        || matches!(
                            instruction.operand(),
                            Operand::Forward(_) | Operand::Backward(_)
                        )
                    {
                        return false;
                    }
                }
            }
        }
        block = next;
    }
}

// The original pass performs these location repairs even when it creates no copies, so the fast
// path must preserve them before returning.
fn propagate_exit_block_locations(
    graph: &mut BlockGraph,
    order: &[BasicBlockId],
    analysis: &ExitBlockAnalysis,
    exception_regions: &[ExceptionRegion],
    original_boundaries: &FxHashSet<Label>,
    original_location_absence: &FxHashSet<Label>,
) {
    for target in order.iter().copied() {
        if analysis.exception_handler_blocks[target.index()] {
            continue;
        }
        // A shared discard or conditional cleanup join has no unique source. Exit copies
        // acquire their branch's location separately when the continuation can be copied.
        if analysis.predecessors[target.index()] > 1
            && matches!(
                graph[target].items().iter().find(|item| matches!(item, Item::Instruction(_))),
                Some(Item::Instruction(instruction))
                    if instruction.opcode == POP_TOP
                        || (instruction.opcode == POP_EXCEPT
                            && has_shared_cleanup_entry(
                                graph, order, analysis.positions[target.index()],
                                analysis.conditional_predecessors[target.index()],
                                analysis.predecessors[target.index()],
                            ))
            )
        {
            continue;
        }
        if analysis.predecessors[target.index()] == 1
            && let Some(Item::Instruction(first)) = graph[target]
                .items()
                .iter()
                .find(|item| matches!(item, Item::Instruction(_)))
            && first.opcode == NOP
            && first.fusion_policy.is_converted_pop_block()
            && first.location.line < 0
            && !first.no_location_policy.preserves_location()
            && let Some(location) = analysis.jump_predecessors[target.index()].and_then(|source| {
                graph[source]
                    .last_instruction()
                    .map(|instruction| instruction.location)
            })
            && let Some(first) = graph.instructions_mut(target).next()
        {
            first.location = location;
            continue;
        }
        if !graph[target].items().iter().all(|item| {
            !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
        }) {
            continue;
        }
        if analysis.predecessors[target.index()] > 1
            && is_unlocated_value_cleanup_prefix(graph[target].items())
            && original_entry_has_no_location_donor(
                graph[target].items(),
                original_location_absence,
            )
            && unlocated_prefix_has_located_continuation(
                graph,
                order,
                &analysis.positions,
                &analysis.predecessors,
                exception_regions,
                original_boundaries,
                target,
            )
        {
            // Copying is complete. Keep the proven unlocated prefix through later writers.
            for instruction in graph.instructions_mut(target) {
                instruction.no_location_policy.preserve_location();
            }
            continue;
        }
        let target_position = analysis.positions[target.index()];
        let predecessor = if target_position > 0
            && block_has_fallthrough(graph[order[target_position - 1]].items())
        {
            Some(order[target_position - 1])
        } else {
            analysis.jump_predecessors[target.index()]
        };
        let Some(location) = predecessor.and_then(|source| {
            graph[source]
                .last_instruction()
                .map(|instruction| instruction.location)
        }) else {
            continue;
        };
        if let Some(instruction) = graph.instructions_mut(target).next()
            && !instruction.no_location_policy.preserves_location()
        {
            instruction.location = location;
        }
    }
}

impl OptimizationPipeline {
    /// Resolves synthesized percent-format literals that begin a conditional target block.
    ///
    /// CPython's early line-number propagation gives this leading constant the sole
    /// predecessor's condition location. Serc retains the eventual percent-expression
    /// location on some intervening jumps, so recover the earlier location from the jump's
    /// preceding instruction or, for a split jump block, its sole predecessor.
    pub(in crate::assembler) fn resolve_synthesized_conditional_target_locations(&mut self) {
        if !self.has_synthesized_percent_target_locations {
            return;
        }

        let order = self.graph.order().to_vec();
        let mut predecessors = FxHashMap::<BasicBlockId, (usize, SourceLocation)>::default();
        if let Some(entry) = order.first().copied() {
            predecessors.insert(entry, (1, SourceLocation::NONE));
        }
        for blocks in order.windows(2) {
            let [source, target] = blocks else {
                unreachable!();
            };
            if block_has_fallthrough(self.graph[*source].items()) {
                let location = self.graph[*source]
                    .items()
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) if instruction.location.line >= 0 => {
                            Some(instruction.location)
                        }
                        Item::Instruction(_) | Item::Label(_) => None,
                    })
                    .unwrap_or(SourceLocation::NONE);
                let entry = predecessors
                    .entry(*target)
                    .or_insert((0, SourceLocation::NONE));
                entry.0 += 1;
                entry.1 = location;
            }
        }
        let mut conditional_targets =
            FxHashMap::<BasicBlockId, (BasicBlockId, SourceLocation, SourceLocation)>::default();
        for source in order.iter().copied() {
            let mut previous_location = SourceLocation::NONE;
            for item in self.graph[source].items() {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                let predecessor_location = previous_location;
                if instruction.location.line >= 0 {
                    previous_location = instruction.location;
                }
                let (Operand::Forward(label) | Operand::Backward(label)) = instruction.operand()
                else {
                    continue;
                };
                let Some(target) = self.graph.label_block(label) else {
                    continue;
                };
                let predecessor = predecessors
                    .entry(target)
                    .or_insert((0, SourceLocation::NONE));
                predecessor.0 += 1;
                predecessor.1 = instruction.location;
                if is_conditional_jump(instruction.opcode) {
                    conditional_targets
                        .insert(target, (source, instruction.location, predecessor_location));
                }
            }
        }

        for target in order {
            let Some(&(source, jump_location, predecessor_location)) =
                conditional_targets.get(&target)
            else {
                continue;
            };
            if predecessors.get(&target).map(|(count, _)| *count) != Some(1) {
                continue;
            }
            let Some(first) = self.graph.instructions_mut(target).next() else {
                continue;
            };
            if first.no_location_policy.percent_target_location != u8::MAX {
                continue;
            }
            if first.location.line < 0 {
                let mut location = jump_location;
                if predecessor_location.line < 0
                    && let Some((1, source_predecessor_location)) = predecessors.get(&source)
                    && source_predecessor_location.line >= 0
                {
                    location = *source_predecessor_location;
                }
                first.location = location;
            }
            first.no_location_policy.percent_target_location = 0;
        }
    }

    /// Defers synthesized percent literals that begin a sole fallthrough target.
    ///
    /// CPython's early line-number propagation gives the target's leading unlocated
    /// instruction the last located instruction from its only predecessor. Handler and
    /// try-else body labels create this shape before later CFG optimization.
    pub(in crate::assembler) fn defer_synthesized_fallthrough_locations(
        &mut self,
    ) -> Vec<SourceLocation> {
        if !self.has_synthesized_percent_target_locations {
            return Vec::new();
        }
        let order = self.graph.order().to_vec();
        let Some(maximum_block) = order.iter().map(|block| block.index()).max() else {
            return Vec::new();
        };
        let mut predecessors = vec![0_usize; maximum_block + 1];
        predecessors[order[0].index()] = 1;
        for (position, source) in order.iter().copied().enumerate() {
            if block_has_fallthrough(self.graph[source].items())
                && let Some(target) = order.get(position + 1)
            {
                predecessors[target.index()] += 1;
            }
            if let Some(target) = block_jump_target(self.graph[source].items())
                .and_then(|label| self.graph.label_block(label))
            {
                predecessors[target.index()] += 1;
            }
        }

        let mut locations = Vec::new();
        for blocks in order.windows(2) {
            let [source, target] = blocks else {
                unreachable!();
            };
            if predecessors[target.index()] != 1
                || !block_has_fallthrough(self.graph[*source].items())
            {
                continue;
            }
            let Some(location) =
                self.graph[*source]
                    .items()
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) if instruction.location.line >= 0 => {
                            Some(instruction.location)
                        }
                        Item::Instruction(_) | Item::Label(_) => None,
                    })
            else {
                continue;
            };
            let Some(first) = self.graph.instructions_mut(*target).next() else {
                continue;
            };
            if first.location.line >= 0
                || first.no_location_policy.percent_target_location != u8::MAX
            {
                continue;
            }
            let Ok(id) = u8::try_from(locations.len() + 1) else {
                continue;
            };
            // Keep the all-ones value as the unresolved marker. If a single code object
            // contains more candidates than the compact side table can represent, leaving
            // the remainder unlocated preserves the compiler's existing behavior.
            if id == u8::MAX {
                continue;
            }
            locations.push(location);
            first.no_location_policy.percent_target_location = id;
        }
        locations
    }

    /// Applies locations selected before CFG optimization without affecting opcode transforms.
    pub(in crate::assembler) fn resolve_deferred_fallthrough_locations(
        items: &mut [Item],
        has_synthesized_percent_target_locations: bool,
        locations: &[SourceLocation],
    ) {
        if !has_synthesized_percent_target_locations {
            return;
        }

        for item in items {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            let id = instruction.no_location_policy.percent_target_location;
            instruction.no_location_policy.percent_target_location = 0;
            if id == 0 || id == u8::MAX || instruction.location.line >= 0 {
                continue;
            }
            if let Some(location) = locations.get(usize::from(id - 1)) {
                instruction.location = *location;
            }
        }
    }

    pub(in crate::assembler) fn push_cold_blocks_to_end(&mut self) {
        if self.exception_regions.is_empty() {
            return;
        }
        let graph = &mut self.graph;
        if graph.len() < 2 {
            return;
        }

        let order = graph.order().to_vec();
        let block_slot_count = order
            .iter()
            .map(|block| block.index())
            .max()
            .map_or(0, |index| index + 1);
        let mut block_labels = vec![None; block_slot_count];
        let mut positions = vec![usize::MAX; block_slot_count];
        for (position, block) in order.iter().copied().enumerate() {
            let label = if let Some(label) = graph[block].first_label() {
                label
            } else {
                let label = Label(self.next_label);
                self.next_label += 1;
                graph.insert_label(block, 0, label);
                label
            };
            block_labels[block.index()] = Some(label);
            positions[block.index()] = position;
        }
        let region_ranges = self
            .exception_regions
            .iter()
            .map(|region| {
                let start = graph
                    .label_block(region.start)
                    .and_then(|block| positions.get(block.index()).copied())
                    .filter(|position| *position != usize::MAX)
                    .unwrap_or(0);
                let end = graph
                    .label_block(region.end)
                    .and_then(|block| positions.get(block.index()).copied())
                    .filter(|position| *position != usize::MAX)
                    .unwrap_or(order.len());
                if start <= end { (start, end) } else { (0, 0) }
            })
            .collect::<Vec<_>>();

        let mut temperatures = vec![0_u8; block_slot_count];
        let mut stack = vec![order[0]];
        while let Some(block) = stack.pop() {
            let slot = block.index();
            if temperatures[slot] == WARM_BLOCK {
                continue;
            }
            temperatures[slot] = WARM_BLOCK;
            let position = positions[slot];
            if block_has_fallthrough(graph[block].items())
                && let Some(next) = order.get(position + 1)
            {
                stack.push(*next);
            }
            if let Some(target) = block_jump_target(graph[block].items())
                && let Some(target) = graph.label_block(target)
            {
                stack.push(target);
            }
        }

        let mut stack = self
            .exception_regions
            .iter()
            .filter_map(|region| graph.label_block(region.target))
            .filter(|block| temperatures[block.index()] != WARM_BLOCK)
            .collect::<Vec<_>>();
        while let Some(block) = stack.pop() {
            let slot = block.index();
            if temperatures[slot] != 0 {
                continue;
            }
            temperatures[slot] = COLD_BLOCK;
            let position = positions[slot];
            if block_has_fallthrough(graph[block].items())
                && let Some(next) = order.get(position + 1)
            {
                stack.push(*next);
            }
            if let Some(target) = block_jump_target(graph[block].items())
                && let Some(target) = graph.label_block(target)
            {
                stack.push(target);
            }
        }
        if !temperatures.contains(&COLD_BLOCK) {
            return;
        }

        let explicit_jumps = (0..order.len().saturating_sub(1))
            .filter(|position| {
                temperatures[order[*position].index()] == COLD_BLOCK
                    && block_has_fallthrough(graph[order[*position]].items())
                    && temperatures[order[*position + 1].index()] == WARM_BLOCK
            })
            .collect::<Vec<_>>();
        for position in explicit_jumps.into_iter().rev() {
            let next = graph.block_at(position + 1);
            let target = if let Some(label) = graph[next].first_label() {
                label
            } else {
                let label = Label(self.next_label);
                self.next_label += 1;
                graph.insert_label(next, 0, label);
                block_labels[next.index()] = Some(label);
                label
            };
            let current = graph.block_at(position);
            let (location, exception_exclusion) = graph[current].last_instruction().map_or(
                (SourceLocation::NONE, ExceptionExclusion::None),
                |instruction| {
                    let exclusion = if instruction.exception_policy().exclusion()
                        == ExceptionExclusion::IfExtended
                    {
                        ExceptionExclusion::IfExtended
                    } else {
                        ExceptionExclusion::None
                    };
                    (instruction.location, exclusion)
                },
            );
            let label = Label(self.next_label);
            self.next_label += 1;
            let block = graph.insert_block(
                position + 1,
                vec![
                    Item::Label(label),
                    Item::Instruction({
                        let mut instruction = Instruction::synthetic(
                            JUMP_BACKWARD_NO_INTERRUPT,
                            Operand::Backward(target),
                            location,
                            None,
                        );
                        instruction.exclude_from_exception_table(exception_exclusion);
                        instruction
                    }),
                ],
            );
            let index = block.index();
            block_labels.resize(index + 1, None);
            positions.resize(index + 1, usize::MAX);
            temperatures.resize(index + 1, 0);
            block_labels[index] = Some(label);
            temperatures[index] = COLD_BLOCK;
        }

        let mut order = Vec::with_capacity(graph.len());
        for expected_cold in [false, true] {
            for block in graph.order().iter().copied() {
                if (expected_cold && temperatures[block.index()] == COLD_BLOCK)
                    || (!expected_cold && temperatures[block.index()] == WARM_BLOCK)
                {
                    order.push(block);
                }
            }
        }

        let async_iteration_exits = order.iter().filter(|block| {
            graph[**block].items().iter().any(|item| {
                matches!(item, Item::Instruction(instruction) if instruction.opcode == END_ASYNC_FOR)
            })
        }).collect::<Vec<_>>();
        let nested_async_cleanup = async_iteration_exits.windows(2).any(|blocks| {
            cleanup_block_creation_order(graph[*blocks[0]].items())
                > cleanup_block_creation_order(graph[*blocks[1]].items())
        });
        // Inlined comprehensions emit their successful restore after the exceptional restore.
        // CPython puts a terminal restore before the handlers. Move a direct local restore or
        // the outer restore of nested async loops back to END_ASYNC_FOR's fallthrough.
        let terminal_restore = (0..order.len().saturating_sub(1)).find_map(|position| {
            let target = graph[order[position]]
                .items()
                .iter()
                .any(|item| {
                    matches!(item, Item::Instruction(instruction) if instruction.opcode == END_ASYNC_FOR)
                })
                .then(|| block_jump_target(graph[order[position + 1]].items()))
                .flatten()
                .and_then(|target| graph.label_block(target))?;
            let start = order.iter().position(|block| *block == target)?;
            let mut end = start;
            while end + 1 < order.len()
                && block_has_fallthrough(graph[order[end]].items())
            {
                end += 1;
            }
            let exits_scope = graph[order[end]]
                .last_instruction()
                .map(|instruction| ends_scope(instruction.opcode));
            let restores_locals = graph[target]
                .items()
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                })
                == Some(SWAP)
                && order[start..=end].iter().all(|block| {
                    graph[*block].items().iter().all(|item| {
                        !matches!(item, Item::Instruction(instruction)
                            if !matches!(instruction.opcode, SWAP | STORE_FAST | RETURN_VALUE))
                    })
                });
            // CPython copies exits with at most four CFG instructions. A protected return
            // still contains POP_BLOCK at that stage, represented here by its region boundary.
            let restore_size = order[start..=end]
                .iter()
                .map(|block| {
                    graph[*block].items().iter().filter(|item| matches!(item, Item::Instruction(_))).count()
                })
                .sum::<usize>()
                + order[start + 1..=end].iter().filter(|block| {
                    self.exception_regions.iter().any(|region| {
                        !region.is_scope_fallback && graph.label_block(region.end) == Some(**block)
                    })
                }).count();
            let relocates_restore = if restores_locals {
                restore_size <= 4
            } else {
                nested_async_cleanup
            };
            (exits_scope == Some(true) && relocates_restore)
                .then_some((position + 2, start, end))
        });
        if let Some((insertion_position, start, end)) = terminal_restore {
            let terminal_restore = order.drain(start..=end).collect::<Vec<_>>();
            let insertion_position = if insertion_position > end {
                insertion_position - terminal_restore.len()
            } else {
                insertion_position
            };
            order.splice(insertion_position..insertion_position, terminal_restore);
        }

        let mut final_label = None;
        let mut rebuilt_regions = Vec::new();
        let original_regions = std::mem::take(&mut self.exception_regions);
        for (region, (original_start, original_end)) in
            original_regions.into_iter().zip(region_ranges)
        {
            let is_member = |block: BasicBlockId| {
                positions
                    .get(block.index())
                    .copied()
                    .is_some_and(|position| original_start <= position && position < original_end)
            };
            let mut index = 0;
            while index < order.len() {
                while index < order.len() && !is_member(order[index]) {
                    index += 1;
                }
                if index == order.len() {
                    break;
                }
                let start = block_labels[order[index].index()]
                    .expect("cold-block pass should label every ordered block");
                while index < order.len() && is_member(order[index]) {
                    index += 1;
                }
                let end = if index < order.len() {
                    block_labels[order[index].index()]
                        .expect("cold-block pass should label every ordered block")
                } else if let Some(label) = final_label {
                    label
                } else {
                    let label = Label(self.next_label);
                    self.next_label += 1;
                    final_label = Some(label);
                    label
                };
                rebuilt_regions.push(ExceptionRegion {
                    start,
                    end,
                    target: region.target,
                    depth: region.depth,
                    preserve_lasti: region.preserve_lasti,
                    is_scope_fallback: region.is_scope_fallback,
                    priority: region.priority,
                });
            }
        }
        self.exception_regions = rebuilt_regions;

        positions.resize(temperatures.len(), usize::MAX);
        positions.fill(usize::MAX);
        for (position, block) in order.iter().copied().enumerate() {
            positions[block.index()] = position;
        }
        for (source_position, source) in order.iter().copied().enumerate() {
            let Some(target) = block_jump_target(graph[source].items()) else {
                continue;
            };
            let Some(target_block) = graph.label_block(target) else {
                continue;
            };
            let Some(target_position) = positions
                .get(target_block.index())
                .copied()
                .filter(|position| *position != usize::MAX)
            else {
                continue;
            };
            let instruction = graph
                .instructions_mut(source)
                .next_back()
                .expect("a block jump target must have a terminal instruction");
            if target_position <= source_position {
                instruction.retarget(Operand::Backward(target));
                if instruction.opcode == JUMP_FORWARD || instruction.jump_policy.is_interruptible()
                {
                    instruction.opcode = if instruction.fusion_policy.is_authored_break()
                        || instruction.jump_policy.is_interruptible()
                    {
                        JUMP_BACKWARD
                    } else {
                        JUMP_BACKWARD_NO_INTERRUPT
                    };
                }
            } else {
                instruction.retarget(Operand::Forward(target));
                if matches!(
                    instruction.opcode,
                    JUMP_BACKWARD | JUMP_BACKWARD_NO_INTERRUPT
                ) {
                    instruction.opcode = JUMP_FORWARD;
                }
            }
        }
        graph
            .replace_order(order)
            .expect("cold-block pass produced an invalid emission order");
        if let Some(final_label) = final_label {
            graph.push_block(vec![Item::Label(final_label)]);
        }
    }

    // CPython resolves locations before removing redundant jump blocks. A later shared entry
    // can therefore retain a line inherited while it still had one predecessor. Capture the
    // original relation before threading, including omitted NOP and folded-slot evidence.
    pub(in crate::assembler) fn capture_original_location_absence(&self) -> FxHashSet<Label> {
        let mut instructions = Vec::new();
        let mut label_entries = FxHashMap::default();
        let mut pending_labels = Vec::new();
        let mut possible_source = Vec::new();
        for item in self.graph.iter_items() {
            match item {
                Item::Label(label) => pending_labels.push(*label),
                Item::Instruction(instruction) => {
                    let entry = instructions.len();
                    let mut located = instruction.location.line >= 0
                        || instruction.pre_fold_extra_instructions() > 0;
                    for label in pending_labels.drain(..) {
                        located |= self.original_nop_targets.contains(&label);
                        label_entries.insert(label, entry);
                    }
                    instructions.push(instruction);
                    possible_source.push(located);
                }
            }
        }
        let mut predecessors = vec![Vec::new(); instructions.len()];
        for (index, instruction) in instructions.iter().enumerate() {
            if index > 0 {
                let previous = instructions[index - 1];
                if !ends_scope(previous.opcode) && !is_unconditional_jump(previous.opcode) {
                    predecessors[index].push(index - 1);
                }
            }
            if let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
                && let Some(&target) = label_entries.get(&label)
            {
                predecessors[target].push(index);
            }
        }
        // An entry with no predecessor, a cycle, or hidden original instructions is unknown.
        // Follow sole-predecessor chains transitively; shared unlocated entries cannot inherit
        // a predecessor's line. Memoization bounds the traversal to the original graph size.
        let mut absence = vec![None; instructions.len()];
        let mut visited = vec![usize::MAX; instructions.len()];
        for start in 0..instructions.len() {
            let mut path = Vec::new();
            let mut entry = start;
            let absent = loop {
                if let Some(absent) = absence[entry] {
                    break absent;
                }
                if entry == 0 || possible_source[entry] || visited[entry] == start {
                    break false;
                }
                visited[entry] = start;
                path.push(entry);
                match predecessors[entry].as_slice() {
                    [previous] => entry = *previous,
                    incoming => break incoming.len() > 1,
                }
            };
            absence[start] = Some(absent);
            for entry in path {
                absence[entry] = Some(absent);
            }
        }
        label_entries
            .into_iter()
            .filter_map(|(label, entry)| (absence[entry] == Some(true)).then_some(label))
            .collect()
    }

    pub(in crate::assembler) fn duplicate_exit_blocks(
        &mut self,
        original_location_absence: &FxHashSet<Label>,
    ) -> Vec<Label> {
        let graph = &mut self.graph;
        if graph.is_normalized_jump_free_single_block(&self.preserved_block_boundaries) {
            return Vec::new();
        }

        if !graph.is_normalized(&self.preserved_block_boundaries) {
            graph.normalize_blocks(&self.preserved_block_boundaries);
        }
        if graph.len() < 2 {
            return Vec::new();
        }

        let order = graph.order().to_vec();
        let analysis = exit_block_analysis(
            graph,
            &order,
            &self.exception_regions,
            &self.original_nop_targets,
        );
        let shared_pop_block_heads = order
            .iter()
            .copied()
            .filter_map(|target| {
                if analysis.predecessors[target.index()] <= 1
                    || analysis.exception_handler_blocks[target.index()]
                {
                    return None;
                }
                let Item::Label(label) = graph[target].items().first()? else {
                    return None;
                };
                let mut instructions = graph[target].items().iter().filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                });
                let first = instructions.next()?;
                (instructions.next().is_none()
                    && first.opcode == NOP
                    && first.fusion_policy.is_converted_pop_block()
                    && first.location == SourceLocation::NONE
                    && !first.no_location_policy.preserves_location()
                    && !first.jump_policy.preserves_explicit_nop()
                    && !first.jump_policy.borrow_empty_block())
                .then_some((target, *label))
            })
            .collect::<Vec<_>>();
        if !analysis.requires_copy {
            propagate_exit_block_locations(
                graph,
                &order,
                &analysis,
                &self.exception_regions,
                &self.exception_owner_provenance.block_boundaries,
                original_location_absence,
            );
            let heads = shared_pop_block_heads
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>();
            restore_shared_pop_block_locations(graph, &self.exception_regions, &heads);
            return Vec::new();
        }
        let ExitBlockAnalysis {
            positions,
            predecessors,
            jump_predecessors: _,
            conditional_predecessors: _,
            exception_handler_blocks,
            requires_copy: _,
        } = analysis;
        for block in order.iter().copied() {
            if graph[block].first_label().is_none() {
                let label = Label(self.next_label);
                self.next_label += 1;
                graph.insert_label(block, 0, label);
            }
        }
        let mut exception_boundary_blocks = vec![false; positions.len()];
        for block in self
            .exception_regions
            .iter()
            .flat_map(|region| [region.start, region.end, region.target])
            .filter_map(|label| graph.label_block(label))
        {
            exception_boundary_blocks[block.index()] = true;
        }
        let region_memberships = self
            .exception_regions
            .iter()
            .map(|region| {
                let start = graph
                    .label_block(region.start)
                    .map(|block| positions[block.index()])
                    .unwrap_or(0);
                let end = graph
                    .label_block(region.end)
                    .map(|block| positions[block.index()])
                    .unwrap_or(order.len());
                if start <= end { start..end } else { 0..0 }
            })
            .collect::<Vec<_>>();

        let exception_split_labels = self
            .exception_regions
            .iter()
            .flat_map(|region| [region.start, region.end])
            .collect::<FxHashSet<_>>();
        let mut exit_without_unique_predecessor = vec![false; positions.len()];
        for block in order.iter().copied() {
            let unlocated_scope_exit = graph[block].items().iter().all(|item| {
                        !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                    })
                    && graph[block]
                        .last_instruction()
                        .is_some_and(|instruction| ends_scope(instruction.opcode));
            let mut original_unlocated_discard = false;
            let artificial_eval_break =
                graph[block].last_instruction().is_some_and(|instruction| {
                    instruction.no_location_policy.is_artificial_eval_break()
                }) && graph[block].items().iter().all(|item| match item {
                    Item::Label(label) => {
                        original_unlocated_discard |=
                            self.original_unlocated_discard_targets.contains(label);
                        true
                    }
                    Item::Instruction(instruction) => {
                        let unlocated_discard = std::mem::take(&mut original_unlocated_discard)
                            && instruction.opcode == POP_TOP;
                        // An early-folded Boolean may retain its fallback on the original
                        // discard. Other located instructions, including END_SEND, stay shared.
                        instruction.location.line < 0
                            || instruction.no_location_policy.is_artificial_eval_break()
                            || unlocated_discard
                    }
                });
            exit_without_unique_predecessor[block.index()] =
                predecessors[block.index()] > 1 && (unlocated_scope_exit || artificial_eval_break);
        }

        let mut inline_copies = FxHashMap::<_, Vec<Vec<Item>>>::default();
        let mut copied_entry_labels = Vec::new();
        let mut copied_exit_or_eval_entry_labels = FxHashSet::default();
        let mut retained_unlocated_join_blocks = FxHashSet::default();
        let mut target_copies = FxHashMap::<_, Vec<Vec<Item>>>::default();
        let mut copied_region_exclusions =
            vec![Vec::<(Label, Label)>::new(); self.exception_regions.len()];
        let mut copied_exception_regions = Vec::new();
        let mut drop_blocks = vec![false; positions.len()];
        let mut remaining_predecessors = predecessors.clone();
        let mut jump_sources = vec![Vec::<BasicBlockId>::new(); positions.len()];
        for source in order.iter().copied() {
            if let Some(target) =
                block_jump_target(graph[source].items()).and_then(|label| graph.label_block(label))
            {
                jump_sources[target.index()].push(source);
            }
        }
        let (mut conditional_sources, mut sources): (Vec<_>, Vec<_>) = order
            .iter()
            .copied()
            .partition(|source| block_has_fallthrough(graph[*source].items()));
        let mut source_cursor = 0;
        loop {
            let Some(&source) = sources.get(source_cursor) else {
                if conditional_sources.is_empty() {
                    break;
                }
                // CPython finishes unconditional inlining before duplicating shared exits
                // for conditional predecessors while resolving source locations.
                for block in order.iter().copied() {
                    let Some(copies) = inline_copies.remove(&block) else {
                        continue;
                    };
                    graph.append_to_block(block, copies.into_iter().flatten());
                    let unlocated_scope_exit = graph[block].items().iter().all(|item| {
                        !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                    }) && graph[block]
                        .last_instruction()
                        .is_some_and(|instruction| ends_scope(instruction.opcode));
                    exit_without_unique_predecessor[block.index()] |=
                        remaining_predecessors[block.index()] > 1 && unlocated_scope_exit;
                }
                sources.append(&mut conditional_sources);
                continue;
            };
            source_cursor += 1;
            let Some(target_label) = block_jump_target(graph[source].items()) else {
                continue;
            };
            let Some(target) = graph.label_block(target_label) else {
                continue;
            };
            let source_position = positions[source.index()];
            let target_position = positions[target.index()];
            let source_has_fallthrough = block_has_fallthrough(graph[source].items());
            let mut target_chain_end = target_position;
            while target_chain_end + 1 < order.len()
                && block_has_fallthrough(graph[order[target_chain_end]].items())
                && predecessors[order[target_chain_end + 1].index()] == 1
                && graph[order[target_chain_end + 1]]
                    .items()
                    .iter()
                    .any(|item| {
                        matches!(item, Item::Label(label) if exception_split_labels.contains(label))
                    })
            {
                target_chain_end += 1;
            }
            let target_blocks = &order[target_position..=target_chain_end];
            let target_exit = *target_blocks.last().expect("target chain is nonempty");
            let target_pre_fusion_size = target_blocks
                .iter()
                .flat_map(|block| graph[*block].items())
                .map(|item| match item {
                    Item::Instruction(instruction) => {
                        let fused_push_null = instruction.opcode == LOAD_GLOBAL
                            && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0);
                        1 + usize::from(fused_push_null)
                            + usize::from(instruction.pre_fold_extra_instructions())
                    }
                    Item::Label(label) => usize::from(
                        target_chain_end > target_position
                            && exception_split_labels.contains(label),
                    ),
                })
                .sum::<usize>();
            let target_terminal_opcode = target_blocks
                .iter()
                .rev()
                .flat_map(|block| graph[*block].items().iter().rev())
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                });
            let source_copies_split_return =
                graph[source].last_instruction().is_some_and(|instruction| {
                    instruction.no_location_policy.copies_split_return_exit()
                });
            let target_is_small_exit = target_pre_fusion_size <= 4
                && target_terminal_opcode.is_some_and(|opcode| {
                    ends_scope(opcode)
                        // SETUP_FINALLY and POP_BLOCK do not split the original CPython block.
                        && (target_chain_end == target_position
                            || matches!(opcode, RAISE_VARARGS | RETURN_VALUE))
                });
            let source_allows_small_exit_inlining = (target_chain_end == target_position
                && block_is_bare_raise(graph[target].items(), &self.original_nop_targets))
                || graph[source]
                    .last_instruction()
                    .is_some_and(|instruction| instruction.jump_policy.inline_small_exit());
            let target_contains_end_send = target_blocks.iter().any(|block| {
                graph[*block].items().iter().any(|item| {
                    matches!(item, Item::Instruction(instruction) if instruction.opcode == END_SEND)
                })
            });
            let target_has_no_location = target_blocks.iter().all(|block| {
                graph[*block].items().iter().all(|item| {
                    !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                })
            });
            let target_has_artificial_eval_break = target_blocks.iter().any(|block| {
                graph[*block].items().iter().any(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if instruction.no_location_policy.is_artificial_eval_break())
                })
            });
            let target_allows_no_location_block_inlining = graph[target_exit]
                .last_instruction()
                .is_some_and(|instruction| instruction.no_location_policy.allows_block_inlining());
            if source_has_fallthrough
                && graph[target_exit]
                    .last_instruction()
                    .is_some_and(Instruction::only_allows_unconditional_inlining)
            {
                // Early unconditional copying does not imply permission to duplicate this
                // shared exit while resolving a conditional predecessor's locations.
                continue;
            }
            let target_is_no_location_no_fallthrough = target_has_no_location
                && target_allows_no_location_block_inlining
                && !block_has_fallthrough(graph[target_exit].items())
                && (!source_has_fallthrough
                    || no_location_chain_reaches_exit(
                        graph,
                        &order,
                        &positions,
                        &predecessors,
                        &exception_split_labels,
                        target,
                    )
                    || target_has_artificial_eval_break);
            let inline_small_exit = !source_has_fallthrough
                && target_is_small_exit
                && source_allows_small_exit_inlining
                && !target_contains_end_send
                && !exception_handler_blocks[target.index()];
            // Concrete exception regions can split original POP_BLOCK slots across labels.
            // Revisit return copies directly when no such boundary affects either block.
            let inline_unprotected_return = inline_small_exit
                && target_terminal_opcode == Some(RETURN_VALUE)
                && region_memberships
                    .iter()
                    .enumerate()
                    .all(|(index, membership)| {
                        self.exception_regions[index].is_scope_fallback
                            || (!membership.contains(&source_position)
                                && !membership.contains(&target_position))
                    });
            let inline_no_location_block =
                !source_has_fallthrough && target_is_no_location_no_fallthrough;
            let copy_conditional_split_return = source_has_fallthrough
                && source_copies_split_return
                && target_is_small_exit
                && remaining_predecessors[target.index()] > 1
                && !exception_handler_blocks[target.index()];
            if exit_without_unique_predecessor[target.index()]
                && target_pre_fusion_size > 4
                && !target_blocks.iter().any(|block| {
                    graph[*block].items().iter().any(|item| {
                        matches!(item, Item::Instruction(instruction) if instruction.opcode == POP_EXCEPT)
                    })
                })
                && remaining_predecessors[target.index()] > 1
                && jump_sources[target.index()]
                    .iter()
                    .copied()
                    .any(|predecessor| {
                        positions[predecessor.index()] > source_position
                            && block_jump_target(graph[predecessor].items())
                                .and_then(|label| graph.label_block(label))
                                == Some(target)
                    })
            {
                // CPython's second exit-duplication pass runs after cold blocks move. For a
                // large class footer shared by warm and cold jumps, that makes the cold, later
                // predecessor receive the copy while the warm predecessor keeps the original.
                // Exception cleanup exits are copied before that relocation and retain the
                // original predecessor order, even when clearing a name makes the exit large.
                continue;
            }
            // An omitted authored NOP entry is distinct from a later aliased jump head.
            // CPython still sees that located NOP block while cloning artificial exits.
            if self.original_nop_targets.contains(&target_label)
                && target_has_artificial_eval_break
                && !(inline_small_exit || inline_no_location_block || copy_conditional_split_return)
            {
                continue;
            }
            if !(inline_small_exit || inline_no_location_block || copy_conditional_split_return)
                && (!(exit_without_unique_predecessor[target.index()]
                    || target_is_no_location_no_fallthrough)
                    || remaining_predecessors[target.index()] <= 1)
            {
                continue;
            }

            // Replacing this source jump ends its original join provenance. Copied target
            // labels are renamed below and therefore cannot inherit that provenance either.
            if !self.deferred_shared_jump_blocks.is_empty()
                || !self.shared_jump_copy_location_blocks.is_empty()
            {
                for item in graph[source].items() {
                    if let Item::Label(label) = item {
                        self.deferred_shared_jump_blocks.remove(label);
                        self.shared_jump_copy_location_blocks.remove(label);
                    }
                }
            }

            let source_location = graph[source]
                .items()
                .iter()
                .rev()
                .find_map(|item| match item {
                    Item::Instruction(instruction) if instruction.location.line >= 0 => {
                        Some(instruction.location)
                    }
                    Item::Instruction(_) | Item::Label(_) => None,
                })
                .unwrap_or(SourceLocation::NONE);
            let mut copied = target_blocks
                .iter()
                .flat_map(|block| graph[*block].items().iter().copied())
                .collect::<Vec<_>>();
            // A shared jump keeps its original no-location policy through NOP removal.
            // Its copy has a new predecessor and can inherit that predecessor's location.
            // Read the original labels before they are renamed below.
            if !self.shared_jump_copy_location_blocks.is_empty() {
                let mut shared_jump = false;
                for item in &mut copied {
                    match item {
                        Item::Label(label) => {
                            shared_jump |= self.shared_jump_copy_location_blocks.contains(label);
                        }
                        Item::Instruction(instruction) => {
                            if shared_jump && is_unconditional_jump(instruction.opcode) {
                                instruction.no_location_policy.clear_preserved_location();
                            }
                            shared_jump = false;
                        }
                    }
                }
            }
            if inline_no_location_block
                && graph[source].last_instruction().is_some_and(|instruction| {
                    instruction.jump_policy.is_interruptible()
                        || instruction.fusion_policy.is_authored_break()
                })
                && let Some(instruction) = copied.iter_mut().rev().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                && is_unconditional_jump(instruction.opcode)
            {
                // CPython preserves the removed JUMP's eval-breaker check when replacing it
                // with an unlocated cleanup block whose terminal jump has no such check.
                instruction.jump_policy.mark_interruptible();
            }
            if inline_no_location_block
                && let Some(instruction) = copied.iter_mut().rev().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                && instruction
                    .no_location_policy
                    .first_copy_is_interruptible_backward()
                && remaining_predecessors[target.index()] == predecessors[target.index()]
                && instruction.opcode == JUMP_FORWARD
            {
                // CPython's copied normal exit checks the eval breaker if cold-block ordering
                // turns it backward. The original synthetic cleanup edge remains no-interrupt.
                instruction.opcode = JUMP_BACKWARD;
            }
            // Keep an isolated unlocated join's replaced-jump NOP until its
            // remaining fallthrough predecessor supplies a location. Earlier jump
            // threading may already have removed its other incoming edges.
            // Exception bookkeeping can split a block without creating an original
            // CFG boundary, so only retain markers on recorded original blocks.
            let preserve_unlocated_join_nop = inline_unprotected_return
                && target_chain_end == target_position
                && source_location.line < 0
                && graph[source].items().iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.block_boundaries.contains(label))
                })
                && remaining_predecessors[source.index()] == 1
                && graph[source]
                    .items()
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count()
                    == 1
                && graph[source].last_instruction().is_some_and(|instruction| {
                    !instruction.no_location_policy.preserves_location()
                        && instruction.exception_policy() == ExceptionPolicy::default()
                })
                && region_memberships.iter().all(|membership| {
                    !membership.contains(&source_position) && !membership.contains(&target_position)
                })
                && source_position.checked_sub(1).is_some_and(|position| {
                    let previous = order[position];
                    !drop_blocks[previous.index()]
                        && !inline_copies.contains_key(&previous)
                        && !target_copies.contains_key(&previous)
                        && block_has_fallthrough(graph[previous].items())
                        && graph[previous]
                            .last_instruction()
                            .is_some_and(|instruction| instruction.location.line >= 0)
                });
            if preserve_unlocated_join_nop {
                retained_unlocated_join_blocks.insert(source);
            }
            let preserve_inlined_jump_nop = inline_small_exit
                && (preserve_unlocated_join_nop
                    || (source_location.line >= 0
                        && graph[source].last_instruction().is_some_and(|instruction| {
                            instruction
                                .jump_policy
                                .retains_nop_for(instruction.operand())
                        })));
            let copied_label = Label(self.next_label);
            self.next_label += 1;
            if !self.borrow_finally_jump_sources.is_empty() {
                // A copied terminal jump has no independent original source certificate.
                self.borrow_finally_jump_sources.insert(copied_label, false);
            }
            let mut replaced_entry_label = false;
            let mut copied_labels = FxHashMap::default();
            for item in &mut copied {
                if let Item::Label(label) = item {
                    let original = *label;
                    if !replaced_entry_label {
                        *label = copied_label;
                        replaced_entry_label = true;
                    } else {
                        *label = Label(self.next_label);
                        self.next_label += 1;
                    }
                    copied_labels.insert(original, *label);
                }
            }
            if !replaced_entry_label {
                copied.insert(0, Item::Label(copied_label));
            }
            // A previously inlined exit can contain a jump to one of its own labels.
            // Keep that edge within this copy when the shared exit is duplicated again.
            for item in &mut copied {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                match instruction.operand() {
                    Operand::Forward(target) => {
                        if let Some(target) = copied_labels.get(&target) {
                            instruction.retarget(Operand::Forward(*target));
                        }
                    }
                    Operand::Backward(target) => {
                        if let Some(target) = copied_labels.get(&target) {
                            instruction.retarget(Operand::Backward(*target));
                        }
                    }
                    Operand::Value(_) | Operand::Deferred(_) => {}
                }
            }
            let trailing_region_ends = self
                .exception_regions
                .iter()
                .filter(|region| {
                    copied_labels.contains_key(&region.start)
                        && !copied_labels.contains_key(&region.end)
                })
                .map(|region| region.end)
                .collect::<Vec<_>>();
            for original in trailing_region_ends {
                let copied_end = Label(self.next_label);
                self.next_label += 1;
                copied.push(Item::Label(copied_end));
                copied_labels.insert(original, copied_end);
            }
            // An exception split can end inside the copied cleanup even when its region began in
            // the predecessor's handler body. Preserve that leading intersection in the copy;
            // inner exclusions still divide the successive POP_EXCEPT instructions below.
            copied_exception_regions.extend(self.exception_regions.iter().enumerate().filter_map(
                |(index, region)| {
                    let start = copied_labels.get(&region.start).copied().or_else(|| {
                        (target_chain_end > target_position
                            && region_memberships[index].contains(&positions[target.index()]))
                        .then_some(copied_label)
                    })?;
                    Some(ExceptionRegion {
                        start,
                        end: *copied_labels.get(&region.end)?,
                        target: copied_labels
                            .get(&region.target)
                            .copied()
                            .unwrap_or(region.target),
                        depth: region.depth,
                        preserve_lasti: region.preserve_lasti,
                        is_scope_fallback: region.is_scope_fallback,
                        priority: region.priority,
                    })
                },
            ));
            if preserve_inlined_jump_nop {
                let position = copied
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))
                    .unwrap_or(copied.len());
                copied.insert(
                    position,
                    Item::Instruction(Instruction::synthetic(
                        NOP,
                        Operand::Value(0),
                        source_location,
                        None,
                    )),
                );
            }
            if inline_unprotected_return
                && let Some(instruction) = copied.iter_mut().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
            {
                // CPython retains the replaced jump and any jumps bypassed by our earlier
                // threading as NOPs until after the repeated small-exit copy pass.
                let jump_extra = graph[source]
                    .last_instruction()
                    .map_or(0, Instruction::pre_fold_extra_instructions);
                instruction.add_pre_fold_extra_instructions(
                    u8::from(!preserve_inlined_jump_nop).saturating_add(jump_extra),
                );
            }
            let inherit_copied_locations = copied
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .is_some_and(|instruction| {
                    let inherits_shared_predecessor = instruction
                        .no_location_policy
                        .copy_inherits_shared_predecessor();
                    (target_has_no_location
                        || target_has_artificial_eval_break
                        || (inline_small_exit && instruction.location.line < 0)
                        || copy_conditional_split_return)
                        && (!inline_no_location_block
                            || predecessors[source.index()] <= 1
                            || inherits_shared_predecessor
                            // A removed authored NOP still owns this copy, even when several
                            // conditions target the NOP's original block.
                            || (source_location.line >= 0
                                && graph[source].items().iter().any(|item| {
                                    matches!(item, Item::Label(label)
                                        if self.original_nop_targets.contains(label))
                                }))
                            // A located instruction inside the predecessor owns the copied
                            // exit even when earlier branches join at that instruction.
                            || graph[source].items().iter().any(|item| {
                                matches!(item, Item::Instruction(instruction)
                                    if instruction.location.line >= 0
                                        && !is_unconditional_jump(instruction.opcode))
                            }))
                        && !instruction.no_location_policy.preserves_location()
                });
            if inherit_copied_locations {
                // Exception boundaries split the IR into blocks after CPython has already treated
                // the synthetic cleanup as one source-less exit. Every copied split therefore
                // inherits the same predecessor location as the entry block.
                let copied_exception_split_labels = exception_split_labels
                    .iter()
                    .filter_map(|label| copied_labels.get(label).copied())
                    .collect::<FxHashSet<_>>();
                let copied_inline_entry_labels = copied_exit_or_eval_entry_labels
                    .iter()
                    .filter_map(|label| copied_labels.get(label).copied())
                    .collect::<FxHashSet<_>>();
                let mut block_needs_location = true;
                let mut inline_entry = false;
                let mut inline_location = source_location;
                for item in &mut copied {
                    match item {
                        Item::Label(label) if copied_exception_split_labels.contains(label) => {
                            block_needs_location = true;
                        }
                        Item::Label(label) if copied_inline_entry_labels.contains(label) => {
                            inline_entry = true;
                        }
                        Item::Instruction(instruction) => {
                            if block_needs_location {
                                instruction.location = source_location;
                                block_needs_location = false;
                            } else if inline_entry
                                && instruction.location.line < 0
                                && !instruction.no_location_policy.preserves_location()
                            {
                                instruction.location = inline_location;
                            }
                            inline_entry = false;
                            if instruction.location.line >= 0
                                || instruction.no_location_policy.preserves_location()
                            {
                                inline_location = instruction.location;
                            }
                        }
                        Item::Label(_) => {}
                    }
                }
            }
            if inline_small_exit || inline_no_location_block {
                if inline_no_location_block && target_chain_end > target_position {
                    let copied_positions = copied
                        .iter()
                        .enumerate()
                        .filter_map(|(position, item)| match item {
                            Item::Label(label) => Some((*label, position)),
                            Item::Instruction(_) => None,
                        })
                        .collect::<FxHashMap<_, _>>();
                    for (index, exclusions) in copied_region_exclusions.iter_mut().enumerate() {
                        let region = &self.exception_regions[index];
                        let membership = &region_memberships[index];
                        if region.is_scope_fallback
                            || !membership.contains(&source_position)
                            || !membership.contains(&target_position)
                            || !membership.contains(&target_chain_end)
                        {
                            continue;
                        }
                        // A previously expanded cleanup can contain holes in this enclosing
                        // owner. Copy both boundaries with the instructions they delimit.
                        let inherited = exclusions
                            .iter()
                            .filter_map(|(start, end)| {
                                let start = *copied_labels.get(start)?;
                                let end = *copied_labels.get(end)?;
                                (copied_positions[&start] < copied_positions[&end])
                                    .then_some((start, end))
                            })
                            .collect::<Vec<_>>();
                        exclusions.extend(inherited);
                    }
                }
                let target_region_owners = self
                    .exception_regions
                    .iter()
                    .zip(&region_memberships)
                    .filter(|(_, membership)| membership.contains(&positions[target.index()]))
                    .map(|(region, _)| (region.target, region.depth, region.preserve_lasti))
                    .collect::<FxHashSet<_>>();
                let entirely_excluded_regions = region_memberships
                    .iter()
                    .enumerate()
                    .filter_map(|(index, membership)| {
                        let region = &self.exception_regions[index];
                        (membership.contains(&positions[source.index()])
                            && !membership.contains(&positions[target.index()])
                            // Cleanup exclusions can split one logical exception owner.
                            // Keep the copied exit protected when the target is in its sibling.
                            && !target_region_owners.contains(&(
                                region.target,
                                region.depth,
                                region.preserve_lasti,
                            )))
                        .then_some(index)
                    })
                    .collect::<Vec<_>>();
                // Copies retain the original target's protected-region end even when both
                // entry blocks initially belong to the same enclosing handler.
                let target_has_preserved_boundary_alias = graph[target]
                    .items()
                    .iter()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .count()
                    > 1;
                let split_excluded_regions = self
                    .exception_regions
                    .iter()
                    .enumerate()
                    .filter_map(|(index, region)| {
                        let membership = &region_memberships[index];
                        if !membership.contains(&positions[source.index()]) {
                            return None;
                        }
                        let end = if membership.contains(&positions[target.index()]) {
                            (target_has_preserved_boundary_alias
                                || (target_terminal_opcode == Some(RETURN_VALUE)
                                    && target_chain_end > target_position))
                                .then_some(region.end)
                        } else {
                            // A sibling can cover only the beginning of a copied exit.
                            // End the inherited source fragment at that sibling's boundary.
                            self.exception_regions
                                .iter()
                                .zip(&region_memberships)
                                .filter(|(sibling, membership)| {
                                    sibling.target == region.target
                                        && sibling.depth == region.depth
                                        && sibling.preserve_lasti == region.preserve_lasti
                                        && membership.contains(&positions[target.index()])
                                })
                                .max_by_key(|(_, membership)| membership.end)
                                .map(|(sibling, _)| sibling.end)
                        }?;
                        copied_labels.get(&end).copied().map(|start| (index, start))
                    })
                    .collect::<Vec<_>>();
                let split_return_exclusions = self
                    .exception_regions
                    .iter()
                    .enumerate()
                    .filter_map(|(index, region)| {
                        (source_copies_split_return
                            && region_memberships[index].contains(&positions[source.index()])
                            && region_memberships[index].contains(&positions[target.index()]))
                        .then(|| copied_labels.get(&region.end).copied())
                        .flatten()
                        .map(|start| (index, start))
                    })
                    .collect::<Vec<_>>();
                if !entirely_excluded_regions.is_empty()
                    || !split_excluded_regions.is_empty()
                    || !split_return_exclusions.is_empty()
                {
                    let mut whole_copy_exclusion_start = copied_label;
                    let trailing_nop_position = graph[source]
                        .items()
                        .iter()
                        .enumerate()
                        .rev()
                        .filter_map(|(position, item)| match item {
                            Item::Instruction(instruction) => Some((position, instruction.opcode)),
                            Item::Label(_) => None,
                        })
                        .nth(1)
                        .filter(|(_, opcode)| *opcode == NOP)
                        .map(|(position, _)| position);
                    if let Some(position) = trailing_nop_position {
                        whole_copy_exclusion_start = Label(self.next_label);
                        self.next_label += 1;
                        graph.insert_label(source, position, whole_copy_exclusion_start);
                    }
                    let copied_end = Label(self.next_label);
                    self.next_label += 1;
                    copied.push(Item::Label(copied_end));
                    for index in entirely_excluded_regions {
                        copied_region_exclusions[index]
                            .push((whole_copy_exclusion_start, copied_end));
                    }
                    for (index, exclusion_start) in split_excluded_regions {
                        copied_region_exclusions[index].push((exclusion_start, copied_end));
                    }
                    for (index, start) in split_return_exclusions {
                        copied_region_exclusions[index].push((start, copied_end));
                    }
                }
            }
            // CPython copies a direct local return through the first branch before revisiting an
            // optimized tuple guard in that branch. The guard is already earlier in block order,
            // so honor the compiler's narrow marker when this source is exactly STORE/LOAD/RETURN.
            let transitive_copy = if inline_small_exit {
                let source_jump_position = graph[source]
                    .items()
                    .iter()
                    .rposition(|item| matches!(item, Item::Instruction(_)))
                    .expect("an exit-copy source must end in a jump");
                let mut source_instructions = graph[source].items()[..source_jump_position]
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    });
                let mut copied_instructions = copied.iter().filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                });
                match (
                    source_instructions.next(),
                    source_instructions.next(),
                    copied_instructions.next(),
                    copied_instructions.next(),
                    copied_instructions.next(),
                ) {
                    (Some(store), None, Some(load), Some(return_value), None)
                        if store.opcode == STORE_FAST
                            && load.opcode == LOAD_FAST
                            && return_value.opcode == RETURN_VALUE
                            && store.operand() == load.operand() =>
                    {
                        Some([*store, *load, *return_value])
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(transitive_copy) = transitive_copy {
                let transitive_predecessors = jump_sources[source.index()]
                    .iter()
                    .copied()
                    .filter(|predecessor| {
                        positions[predecessor.index()] < source_position
                            && block_jump_target(graph[*predecessor].items())
                                .and_then(|label| graph.label_block(label))
                                == Some(source)
                            && graph[*predecessor]
                                .last_instruction()
                                .is_some_and(|instruction| {
                                    instruction.fusion_policy.transitive_small_exit_inlining()
                                })
                            && region_memberships.iter().all(|membership| {
                                membership.contains(&positions[predecessor.index()])
                                    == membership.contains(&positions[source.index()])
                                    && membership.contains(&positions[source.index()])
                                        == membership.contains(&positions[target.index()])
                            })
                    })
                    .collect::<Vec<_>>();
                for predecessor in transitive_predecessors {
                    let copied_label = Label(self.next_label);
                    self.next_label += 1;
                    let mut predecessor_copy = vec![Item::Label(copied_label)];
                    predecessor_copy.extend(transitive_copy.iter().copied().map(Item::Instruction));
                    let jump_position = graph[predecessor]
                        .items()
                        .iter()
                        .rposition(|item| matches!(item, Item::Instruction(_)))
                        .expect("a transitive exit-copy predecessor must end in a jump");
                    graph.remove_item_at(predecessor, jump_position);
                    inline_copies
                        .entry(predecessor)
                        .or_default()
                        .push(predecessor_copy);
                    remaining_predecessors[source.index()] -= 1;
                }
            }
            if inline_small_exit || inline_no_location_block {
                // Inlining replaces the original ordinary jump, even when the copied block
                // also starts with a jump. That replacement cannot thread its predecessors.
                self.conditional_threading_targets
                    .retain(|label, _| graph.label_block(*label) != Some(source));
                self.original_jump_head_targets
                    .retain(|label| graph.label_block(*label) != Some(source));
                copied_entry_labels.push(copied_label);
                // CPython resolves locations across inlined scope exits and eval-check blocks.
                // Other copied continuations can still represent a shared, unlocated join.
                if target_terminal_opcode
                    .is_some_and(|opcode| ends_scope(opcode) || opcode == JUMP_BACKWARD)
                    || target_has_artificial_eval_break
                {
                    copied_exit_or_eval_entry_labels.insert(copied_label);
                }
                if let Some(position) = graph[source]
                    .items()
                    .iter()
                    .rposition(|item| matches!(item, Item::Instruction(_)))
                {
                    graph.remove_item_at(source, position);
                }
            } else if let Some(instruction) = graph.instructions_mut(source).next_back() {
                instruction.retarget(Operand::Forward(copied_label));
            }
            // CPython appends a small, unprotected exit to its predecessor before inserting
            // superinstructions. A tuple/call or attribute store followed by a direct name
            // return can expose two local loads. Keeping the copied entry label would split them
            // and fuse the following pair instead.
            let inline_borrowed_tuple_exit = inline_small_exit
                && graph[source]
                    .last_instruction()
                    .is_some_and(|instruction| instruction.opcode == LOAD_FAST)
                && copied
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.opcode),
                        Item::Label(_) => None,
                    })
                    .is_some_and(|opcode| opcode == LOAD_FAST)
                && (copied.iter().any(|item| {
                    matches!(
                        item,
                        Item::Instruction(instruction)
                            if matches!(instruction.opcode, BUILD_TUPLE | CALL)
                    )
                }) || matches!(
                    copied.as_slice(),
                    [.., Item::Instruction(store), Item::Instruction(load), Item::Instruction(return_value)]
                        if store.opcode == STORE_ATTR
                            && load.opcode == LOAD_FAST
                            && return_value.opcode == RETURN_VALUE
                ))
                && region_memberships
                    .iter()
                    .enumerate()
                    .all(|(index, membership)| {
                        self.exception_regions[index].is_scope_fallback
                            || (!membership.contains(&positions[source.index()])
                                && !membership.contains(&positions[target.index()]))
                    });
            if inline_borrowed_tuple_exit {
                let first_instruction = copied
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))
                    .unwrap_or(copied.len());
                copied.drain(..first_instruction);
                graph.append_to_block(source, copied);
            } else if inline_no_location_block || inline_unprotected_return {
                let copied_jump_target =
                    block_jump_target(&copied).and_then(|label| graph.label_block(label));
                graph.append_to_block(source, copied);
                if inline_unprotected_return {
                    let unlocated_scope_exit = graph[source].items().iter().all(|item| {
                        !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                    }) && graph[source]
                        .last_instruction()
                        .is_some_and(|instruction| ends_scope(instruction.opcode));
                    exit_without_unique_predecessor[source.index()] |=
                        remaining_predecessors[source.index()] > 1 && unlocated_scope_exit;
                }
                if let Some(copied_jump_target) = copied_jump_target {
                    remaining_predecessors[copied_jump_target.index()] += 1;
                    drop_blocks[copied_jump_target.index()] = false;
                    let copied_jump_sources = &mut jump_sources[copied_jump_target.index()];
                    if let Err(position) = copied_jump_sources
                        .binary_search_by_key(&source_position, |predecessor| {
                            positions[predecessor.index()]
                        })
                    {
                        copied_jump_sources.insert(position, source);
                    }
                }
                // CPython repeats block inlining to a fixed point. A copied cleanup can end
                // in another jump, so revisit this source before its earlier predecessors.
                sources.push(source);
                // A completed return also makes earlier predecessors eligible for a small
                // exit copy. Revisit them before beginning conditional exit duplication.
                for predecessor in jump_sources[source.index()].iter().copied() {
                    if positions[predecessor.index()] < source_position
                        && block_jump_target(graph[predecessor].items())
                            .and_then(|label| graph.label_block(label))
                            == Some(source)
                    {
                        sources.push(predecessor);
                    }
                }
            } else if source_has_fallthrough {
                for item in target_blocks.iter().flat_map(|block| graph[*block].items()) {
                    if let Item::Label(original) = item
                        && self.original_jump_head_targets.contains(original)
                        && let Some(replacement) = copied_labels.get(original)
                    {
                        self.original_jump_head_targets.insert(*replacement);
                    }
                }
                if graph[target].items().iter().any(|item| {
                    matches!(item, Item::Label(label) if self.original_jump_head_targets.contains(label))
                }) {
                    self.original_jump_head_targets.insert(copied_label);
                }
                target_copies.entry(target).or_default().push(copied);
            } else {
                inline_copies.entry(source).or_default().push(copied);
            }
            remaining_predecessors[target.index()] -= 1;
            if remaining_predecessors[target.index()] == 0 {
                drop_blocks[target.index()] = true;
            }
        }

        if copied_region_exclusions
            .iter()
            .any(|exclusions| !exclusions.is_empty())
        {
            // Fixed-point no-location inlining can revisit an earlier source after copying a
            // later cleanup block. Apply the resulting holes in final emission order; treating
            // discovery order as source order can extend an enclosing handler across the next
            // copied cleanup in a nested unwind chain.
            let label_positions = graph
                .iter_items()
                .enumerate()
                .filter_map(|(position, item)| match item {
                    Item::Label(label) => Some((*label, position)),
                    Item::Instruction(_) => None,
                })
                .collect::<FxHashMap<_, _>>();
            let named_handler_cleanup_regions = self
                .exception_regions
                .iter()
                .map(|region| {
                    graph
                        .label_block(region.target)
                        .and_then(|target| {
                            let mut after_target = false;
                            graph[target].items().iter().find_map(|item| match item {
                                Item::Label(label) => {
                                    after_target |= *label == region.target;
                                    None
                                }
                                Item::Instruction(instruction) if after_target => Some(instruction),
                                Item::Instruction(_) => None,
                            })
                        })
                        .is_some_and(|instruction| instruction.opcode == LOAD_CONST)
                })
                .collect::<Vec<_>>();
            let named_handler_region_keys = self
                .exception_regions
                .iter()
                .zip(&named_handler_cleanup_regions)
                .filter_map(|(region, named_cleanup)| {
                    named_cleanup.then_some((
                        region.start,
                        region.end,
                        region.depth,
                        region.preserve_lasti,
                    ))
                })
                .collect::<FxHashSet<_>>();
            for (index, (region, exclusions)) in self
                .exception_regions
                .iter()
                .zip(&mut copied_region_exclusions)
                .enumerate()
            {
                let paired_named_handler_region = named_handler_region_keys.contains(&(
                    region.start,
                    region.end,
                    region.depth,
                    region.preserve_lasti,
                ));
                if region.preserve_lasti
                    && (named_handler_cleanup_regions[index] || paired_named_handler_region)
                {
                    exclusions.sort_by_key(|(start, _)| {
                        label_positions.get(start).copied().unwrap_or(usize::MAX)
                    });
                } else if !region.is_scope_fallback && exclusions.len() > 1 {
                    // Revisited predecessors can add excluded ranges out of emission order.
                    // Reorder only disjoint holes whose complete ranges are already present
                    // inside the original protected region.
                    let Some((&region_start, &region_end)) = label_positions
                        .get(&region.start)
                        .zip(label_positions.get(&region.end))
                    else {
                        continue;
                    };
                    let Some(mut ranges) = exclusions
                        .iter()
                        .map(|(start, end)| {
                            Some((*label_positions.get(start)?, *label_positions.get(end)?))
                        })
                        .collect::<Option<Vec<_>>>()
                    else {
                        continue;
                    };
                    if ranges.iter().any(|&(start, end)| {
                        start < region_start || start >= end || end > region_end
                    }) {
                        continue;
                    }
                    ranges.sort_unstable();
                    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
                        continue;
                    }
                    exclusions.sort_by_key(|(start, _)| label_positions[start]);
                }
            }
            let original_regions = std::mem::take(&mut self.exception_regions);
            for (region, exclusions) in original_regions.into_iter().zip(copied_region_exclusions) {
                let mut start = region.start;
                for (exclusion_start, exclusion_end) in exclusions {
                    self.exception_regions.push(ExceptionRegion {
                        start,
                        end: exclusion_start,
                        target: region.target,
                        depth: region.depth,
                        preserve_lasti: region.preserve_lasti,
                        is_scope_fallback: region.is_scope_fallback,
                        priority: region.priority,
                    });
                    start = exclusion_end;
                }
                self.exception_regions.push(ExceptionRegion {
                    start,
                    end: region.end,
                    target: region.target,
                    depth: region.depth,
                    preserve_lasti: region.preserve_lasti,
                    is_scope_fallback: region.is_scope_fallback,
                    priority: region.priority,
                });
            }
        }
        self.exception_regions.extend(copied_exception_regions);

        let copied_pop_block_heads = order
            .iter()
            .copied()
            .filter_map(|target| {
                if exception_handler_blocks[target.index()]
                    || remaining_predecessors[target.index()] <= 1
                    || !jump_sources[target.index()].iter().any(|source| {
                        drop_blocks[source.index()]
                            && block_jump_target(graph[*source].items())
                                .and_then(|label| graph.label_block(label))
                                == Some(target)
                    })
                {
                    return None;
                }
                let first = graph[target].items().iter().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })?;
                (first.opcode == NOP
                    && first.fusion_policy.is_converted_pop_block()
                    && first.location.line < 0
                    && !first.no_location_policy.preserves_location())
                .then(|| graph[target].first_label())
                .flatten()
            })
            .collect::<Vec<_>>();

        for target in order.iter().copied() {
            if exception_handler_blocks[target.index()] {
                continue;
            }
            if remaining_predecessors[target.index()] > 1
                && matches!(
                    graph[target].items().iter().find(|item| matches!(item, Item::Instruction(_))),
                    Some(Item::Instruction(instruction))
                        if instruction.opcode == POP_TOP
                            || (instruction.opcode == POP_EXCEPT
                                && has_shared_cleanup_entry(
                                    graph, &order, positions[target.index()],
                                    jump_sources[target.index()].iter().filter(|source| {
                                        block_has_fallthrough(graph[**source].items())
                                            && block_jump_target(graph[**source].items())
                                                .and_then(|label| graph.label_block(label))
                                                == Some(target)
                                    }).count(),
                                    remaining_predecessors[target.index()],
                                ))
                )
            {
                continue;
            }
            if remaining_predecessors[target.index()] == 1
                && let Some(Item::Instruction(first)) = graph[target]
                    .items()
                    .iter()
                    .find(|item| matches!(item, Item::Instruction(_)))
                && first.opcode == NOP
                && first.fusion_policy.is_converted_pop_block()
                && first.location.line < 0
                && !first.no_location_policy.preserves_location()
                && let Some(location) = jump_sources[target.index()].iter().find_map(|source| {
                    block_jump_target(graph[*source].items())
                        .and_then(|label| graph.label_block(label))
                        .filter(|block| *block == target)
                        .and_then(|_| {
                            graph[*source]
                                .last_instruction()
                                .map(|instruction| instruction.location)
                        })
                })
                && let Some(first) = graph.instructions_mut(target).next()
            {
                // A converted `POP_BLOCK` inherits the line of its sole jump predecessor.
                // This remains observable when another predecessor copied the small exit.
                first.location = location;
                continue;
            }
            let copied_exit_needs_location = remaining_predecessors[target.index()] == 1
                && (predecessors[target.index()] > 1
                    || retained_unlocated_join_blocks.contains(&target))
                && graph[target].items().iter().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.location.line < 0),
                    Item::Label(_) => None,
                }) == Some(true);
            if !copied_exit_needs_location && !graph[target].items().iter().all(|item| {
                !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
            }) {
                continue;
            }
            if remaining_predecessors[target.index()] > 1
                && is_unlocated_value_cleanup_prefix(graph[target].items())
                && original_entry_has_no_location_donor(
                    graph[target].items(),
                    original_location_absence,
                )
                && unlocated_prefix_has_located_continuation(
                    graph,
                    &order,
                    &positions,
                    &remaining_predecessors,
                    &self.exception_regions,
                    &self.exception_owner_provenance.block_boundaries,
                    target,
                )
            {
                // Copying is complete. Keep the proven unlocated prefix through later writers.
                for instruction in graph.instructions_mut(target) {
                    instruction.no_location_policy.preserve_location();
                }
                continue;
            }
            let target_position = positions[target.index()];
            let predecessor = if target_position > 0
                && block_has_fallthrough(graph[order[target_position - 1]].items())
            {
                Some(order[target_position - 1])
            } else {
                jump_sources[target.index()].iter().copied().find(|source| {
                    block_jump_target(graph[*source].items())
                        .and_then(|label| graph.label_block(label))
                        == Some(target)
                })
            };
            let Some(location) = predecessor.and_then(|source| {
                graph[source]
                    .last_instruction()
                    .map(|instruction| instruction.location)
            }) else {
                continue;
            };
            if let Some(instruction) = graph.instructions_mut(target).next()
                && !instruction.no_location_policy.preserves_location()
            {
                instruction.location = location;
            }
            // An exit already inlined into this shared block belongs to the same original
            // block when its remaining predecessor supplies a location. Keep its labels for
            // exception accounting while propagating through those copied entry boundaries.
            let mut inline_entry = false;
            let mut inline_location = location;
            let inline_entries = graph[target]
                .items()
                .iter()
                .enumerate()
                .filter_map(|(offset, item)| match item {
                    Item::Label(label) => {
                        inline_entry |= copied_exit_or_eval_entry_labels.contains(label);
                        None
                    }
                    Item::Instruction(instruction) => {
                        if instruction.location.line >= 0
                            || instruction.no_location_policy.preserves_location()
                        {
                            inline_location = instruction.location;
                        }
                        std::mem::take(&mut inline_entry)
                            .then_some((ItemPosition::new(target, offset), inline_location))
                    }
                })
                .collect::<Vec<_>>();
            for (position, inline_location) in inline_entries {
                let instruction = graph.instruction_mut(position).unwrap();
                if instruction.location.line < 0
                    && !instruction.no_location_policy.preserves_location()
                {
                    instruction.location = inline_location;
                }
            }
        }

        let jump_target = |item: &Item| match item {
            Item::Instruction(instruction) => match instruction.operand() {
                Operand::Forward(target) | Operand::Backward(target) => graph.label_block(target),
                Operand::Value(_) | Operand::Deferred(_) => None,
            },
            Item::Label(_) => None,
        };
        let mut retained_jump_targets = order
            .iter()
            .copied()
            .filter(|block| !drop_blocks[block.index()])
            .flat_map(|block| graph[block].items())
            .chain(
                inline_copies
                    .values()
                    .chain(target_copies.values())
                    .flatten()
                    .flatten(),
            )
            .filter_map(jump_target)
            .collect::<Vec<_>>();
        // A copied exit can retain an edge to a block outside the copied chain. Keep that
        // original target even when all of its original predecessors were themselves copied.
        while let Some(target) = retained_jump_targets.pop() {
            if !std::mem::replace(&mut drop_blocks[target.index()], false) {
                continue;
            }
            retained_jump_targets.extend(graph[target].items().iter().filter_map(
                |item| match item {
                    Item::Instruction(instruction) => match instruction.operand() {
                        Operand::Forward(target) | Operand::Backward(target) => {
                            graph.label_block(target)
                        }
                        Operand::Value(_) | Operand::Deferred(_) => None,
                    },
                    Item::Label(_) => None,
                },
            ));
        }

        // Dropped originals may retain labels for exception accounting. They cannot
        // certify the identity of the next surviving NOP after normalization.
        let shared_pop_block_heads = shared_pop_block_heads
            .into_iter()
            .filter_map(|(block, label)| (!drop_blocks[block.index()]).then_some(label))
            .collect::<Vec<_>>();
        let mut final_order = Vec::new();
        let mut discarded_admissions = Vec::new();
        for block in order {
            let first_added = final_order.len();
            if drop_blocks[block.index()] {
                for item in graph[block].items() {
                    if let Item::Label(label) = item
                        && let Some(mut admissions) = self.discarded_fold_admissions.remove(label)
                    {
                        discarded_admissions.append(&mut admissions);
                    }
                }
            }
            if !drop_blocks[block.index()] {
                final_order.push(block);
            } else if exception_boundary_blocks[block.index()] {
                graph.retain_items(block, |item| matches!(item, Item::Label(_)));
                final_order.push(block);
            }
            for copied in inline_copies.remove(&block).unwrap_or_default() {
                final_order.extend(
                    graph.allocate_blocks_from_items(copied, &self.preserved_block_boundaries),
                );
            }
            for copied in target_copies
                .remove(&block)
                .unwrap_or_default()
                .into_iter()
                .rev()
            {
                final_order.extend(
                    graph.allocate_blocks_from_items(copied, &self.preserved_block_boundaries),
                );
            }
            // Removing a copied exit does not erase earlier folds in an omitted arm.
            // Carry those admissions forward once in the surviving emission order.
            if !discarded_admissions.is_empty()
                && let Some(label) = final_order[first_added..]
                    .iter()
                    .flat_map(|block| graph[*block].items())
                    .find_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    })
            {
                if let Some(mut admissions) = self.discarded_fold_admissions.remove(&label) {
                    discarded_admissions.append(&mut admissions);
                }
                self.discarded_fold_admissions
                    .insert(label, std::mem::take(&mut discarded_admissions));
            }
        }
        graph
            .replace_order_and_normalize(final_order, &self.preserved_block_boundaries)
            .expect("exit-block duplication produced an invalid emission order");
        restore_pop_block_head_locations(
            graph,
            &self.exception_regions,
            &shared_pop_block_heads,
            &copied_pop_block_heads,
        );
        copied_entry_labels
    }
}

#[cfg(test)]
mod copied_pop_block_location_tests {
    use super::*;
    use crate::assembler::ExceptionRegionPriority;
    use crate::assembler::ir::FusionPolicy;
    use crate::target::opcodes::POP_JUMP_IF_FALSE;

    fn instruction(opcode: crate::target::Opcode, operand: Operand, line: i32) -> Item {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            if line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(line, line, 12, 17)
            },
            None,
        ))
    }

    fn located(line: i32) -> Item {
        instruction(LOAD_CONST, Operand::Value(0), line)
    }

    fn jump(target: Label) -> Item {
        instruction(JUMP_FORWARD, Operand::Forward(target), -1)
    }

    fn head() -> Item {
        let Item::Instruction(mut instruction) = instruction(NOP, Operand::Value(0), -1) else {
            unreachable!()
        };
        instruction.fusion_policy.mark_converted_pop_block();
        Item::Instruction(instruction)
    }

    fn region(start: Label, end: Label, target: Label) -> ExceptionRegion {
        ExceptionRegion {
            priority: ExceptionRegionPriority::OriginalRange(start, end),
            is_scope_fallback: false,
            start,
            end,
            target,
            depth: 0,
            preserve_lasti: false,
        }
    }

    // These are normalized graph snapshots at the location fallback boundary.
    // Full-pass compiler tests separately exercise how exit copying creates them.
    fn check(
        items: Vec<Item>,
        regions: &[ExceptionRegion],
        heads: &[Label],
        changed: &[(Label, i32)],
    ) {
        let boundaries = items
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashSet<_>>();
        let mut actual = BlockGraph::from_items(items.clone(), &boundaries);
        let mut expected = BlockGraph::from_items(items, &boundaries);
        for &(label, line) in changed {
            let block = expected.label_block(label).unwrap();
            expected.instructions_mut(block).next().unwrap().location =
                SourceLocation::new(line, line, 12, 17);
        }
        restore_copied_pop_block_locations(&mut actual, regions, heads);
        // Check all labels, ordering, operands, and instruction policies as well as
        // the complete four-part locations; only the listed head locations may change.
        assert_eq!(
            format!("{:?}", actual.iter_items().collect::<Vec<_>>()),
            format!("{:?}", expected.iter_items().collect::<Vec<_>>())
        );
    }

    #[test]
    fn shared_parent_cannot_seed_a_unique_copied_head() {
        for shared_parent in [false, true] {
            let entry = Label(0);
            let left = Label(1);
            let right = Label(2);
            let parent = Label(3);
            let child = Label(4);
            let mut parent_head = head();
            if let Item::Instruction(instruction) = &mut parent_head {
                instruction.location = SourceLocation::new(8, 8, 12, 17);
            }
            let mut items = vec![Item::Label(entry)];
            if shared_parent {
                items.push(instruction(LOAD_FAST, Operand::Value(0), 1));
                items.push(instruction(POP_JUMP_IF_FALSE, Operand::Forward(right), 1));
            } else {
                items.push(jump(left));
            }
            items.extend([
                Item::Label(left),
                instruction(NOP, Operand::Value(0), 8),
                instruction(JUMP_FORWARD, Operand::Forward(parent), 8),
                Item::Label(right),
                instruction(NOP, Operand::Value(0), 9),
                instruction(JUMP_FORWARD, Operand::Forward(parent), 9),
                Item::Label(parent),
                parent_head,
                Item::Label(child),
                head(),
                located(20),
                instruction(RETURN_VALUE, Operand::Value(0), 20),
            ]);
            let boundaries = [entry, left, right, parent, child]
                .into_iter()
                .collect::<FxHashSet<_>>();
            let mut actual = BlockGraph::from_items(items.clone(), &boundaries);
            let mut expected = BlockGraph::from_items(items, &boundaries);
            // The shared original's temporary seed must disappear before the
            // child's unique-predecessor lookup. A now-unique original retains
            // its seed and can still pass it to the child.
            let changed = if shared_parent { parent } else { child };
            let block = expected.label_block(changed).unwrap();
            expected.instructions_mut(block).next().unwrap().location = if shared_parent {
                SourceLocation::NONE
            } else {
                SourceLocation::new(8, 8, 12, 17)
            };
            restore_pop_block_head_locations(&mut actual, &[], &[parent], &[child]);
            assert_eq!(
                format!("{:?}", actual.iter_items().collect::<Vec<_>>()),
                format!("{:?}", expected.iter_items().collect::<Vec<_>>()),
                "shared_parent={shared_parent}",
            );
        }
    }

    #[test]
    fn copied_jump_uses_last_located_instruction_and_ignores_dead_predecessor() {
        let dead = Label(1);
        let target = Label(2);
        check(
            vec![
                Item::Label(Label(0)),
                located(8),
                instruction(POP_EXCEPT, Operand::Value(0), 8),
                jump(target),
                Item::Label(dead),
                located(99),
                jump(target),
                Item::Label(target),
                head(),
                located(20),
                instruction(RETURN_VALUE, Operand::Value(0), 20),
            ],
            &[],
            &[target],
            &[(target, 8)],
        );
    }

    #[test]
    fn live_external_edges_from_retained_or_materialized_copies_keep_join_shared() {
        let other = Label(1);
        let target = Label(2);
        // Both edges are present after all retained-target revivals and deferred
        // copies have been materialized. Neither may be discounted as an old copy.
        check(
            vec![
                Item::Label(Label(0)),
                located(1),
                instruction(POP_JUMP_IF_FALSE, Operand::Forward(other), 1),
                located(8),
                jump(target),
                Item::Label(other),
                located(9),
                jump(target),
                Item::Label(target),
                head(),
                located(20),
                instruction(RETURN_VALUE, Operand::Value(0), 20),
            ],
            &[],
            &[target],
            &[],
        );
    }

    #[test]
    fn conditional_jump_and_fallthrough_to_same_block_count_as_two_edges() {
        let target = Label(1);
        check(
            vec![
                Item::Label(Label(0)),
                located(8),
                instruction(POP_JUMP_IF_FALSE, Operand::Forward(target), 8),
                Item::Label(target),
                head(),
                located(20),
                instruction(RETURN_VALUE, Operand::Value(0), 20),
            ],
            &[],
            &[target],
            &[],
        );
    }

    #[test]
    fn handler_predecessor_counts_only_when_its_protected_body_is_reachable() {
        let entry = Label(0);
        let dead_start = Label(1);
        let dead_end = Label(2);
        let handler = Label(3);
        let target = Label(4);
        for live_handler in [false, true] {
            let changed = [(target, 8)];
            check(
                vec![
                    Item::Label(entry),
                    located(8),
                    jump(target),
                    Item::Label(dead_start),
                    located(77),
                    Item::Label(dead_end),
                    instruction(RETURN_VALUE, Operand::Value(0), 77),
                    Item::Label(handler),
                    located(9),
                    jump(target),
                    Item::Label(target),
                    head(),
                    located(20),
                    instruction(RETURN_VALUE, Operand::Value(0), 20),
                ],
                &[region(
                    if live_handler { entry } else { dead_start },
                    dead_end,
                    handler,
                )],
                &[target],
                if live_handler { &[] } else { &changed },
            );
        }
    }

    #[test]
    fn preserved_unlocated_source_boundary_blocks_location_inheritance() {
        let target = Label(1);
        let Item::Instruction(mut last) = jump(target) else {
            unreachable!()
        };
        last.no_location_policy.preserve_location();
        check(
            vec![
                Item::Label(Label(0)),
                located(8),
                Item::Instruction(last),
                Item::Label(target),
                head(),
                located(20),
                instruction(RETURN_VALUE, Operand::Value(0), 20),
            ],
            &[],
            &[target],
            &[],
        );
    }

    #[test]
    fn only_selected_unlocated_converted_nonhandler_heads_can_change() {
        let entry = Label(0);
        let end = Label(1);
        let target = Label(2);
        let selected = [target];
        for case in [
            "located",
            "preserved",
            "ordinary_nop",
            "handler",
            "unselected",
            "unreachable",
        ] {
            let Item::Instruction(mut first) = head() else {
                unreachable!()
            };
            if case == "located" {
                first.location = SourceLocation::new(23, 23, 12, 17);
            } else if case == "preserved" {
                first.no_location_policy.preserve_location();
            } else if case == "ordinary_nop" {
                first.fusion_policy = FusionPolicy::default();
            }
            let regions = if case == "handler" {
                vec![region(entry, end, target)]
            } else {
                vec![]
            };
            check(
                vec![
                    Item::Label(entry),
                    located(8),
                    if case == "unreachable" {
                        instruction(RETURN_VALUE, Operand::Value(0), 8)
                    } else {
                        jump(target)
                    },
                    Item::Label(end),
                    instruction(RETURN_VALUE, Operand::Value(0), 9),
                    Item::Label(target),
                    Item::Instruction(first),
                    located(20),
                    instruction(RETURN_VALUE, Operand::Value(0), 20),
                ],
                &regions,
                if case == "unselected" { &[] } else { &selected },
                &[],
            );
        }
    }

    #[test]
    fn backward_sole_predecessor_has_a_location_but_entry_seed_keeps_loop_shared() {
        let target = Label(1);
        let source = Label(2);
        check(
            vec![
                Item::Label(Label(0)),
                jump(source),
                Item::Label(target),
                head(),
                instruction(RETURN_VALUE, Operand::Value(0), 20),
                Item::Label(source),
                located(8),
                instruction(JUMP_BACKWARD_NO_INTERRUPT, Operand::Backward(target), -1),
            ],
            &[],
            &[target],
            &[(target, 8)],
        );
        check(
            vec![
                Item::Label(target),
                head(),
                located(8),
                instruction(JUMP_BACKWARD_NO_INTERRUPT, Operand::Backward(target), -1),
            ],
            &[],
            &[target],
            &[],
        );
    }

    #[test]
    fn unresolved_chain_does_not_depend_on_candidate_head_order() {
        let first = Label(1);
        let second = Label(2);
        for heads in [[first, second], [second, first]] {
            check(
                vec![
                    Item::Label(Label(0)),
                    located(8),
                    jump(first),
                    Item::Label(first),
                    head(),
                    jump(second),
                    Item::Label(second),
                    head(),
                    located(20),
                    instruction(RETURN_VALUE, Operand::Value(0), 20),
                ],
                &[],
                &heads,
                &[(first, 8)],
            );
        }
    }
}
