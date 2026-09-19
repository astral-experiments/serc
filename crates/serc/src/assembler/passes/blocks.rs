use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    ExceptionExclusion, ExceptionPolicy, ExceptionRegion, Instruction, Item, JUMP_BACKWARD,
    JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, Label, Operand, OptimizationPipeline, SourceLocation,
    block_has_fallthrough, block_jump_target, cleanup_block_creation_order, ends_scope,
};
use crate::assembler::block_graph::{BasicBlockId, BlockGraph, ItemPosition};
use crate::assembler::exception_owners::ExceptionOwnerProvenance;
use crate::target::opcodes::{
    BUILD_TUPLE, CALL, CALL_KW, DELETE_FAST, END_ASYNC_FOR, END_SEND, FOR_ITER, LOAD_CONST,
    LOAD_FAST, LOAD_GLOBAL, NOP, NOT_TAKEN, POP_EXCEPT, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE,
    POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, POP_TOP, RAISE_VARARGS, RERAISE, RETURN_VALUE,
    STORE_ATTR, STORE_FAST, SWAP,
};
use crate::target::{is_conditional_jump, is_unconditional_jump};

/// Recognizes an ordinary jump that retains CPython's interrupt check before layout.
fn has_jump_eval_check(instruction: &Instruction) -> bool {
    instruction.opcode == JUMP_BACKWARD
        || (is_unconditional_jump(instruction.opcode) && instruction.jump_policy.is_interruptible())
}

/// Propagates lines through copied records in the same original instruction array.
fn propagate_planned_array_locations(
    graph: &mut BlockGraph,
    order: &[BasicBlockId],
    planned_blocks: &FxHashSet<BasicBlockId>,
    internal_labels: &FxHashSet<Label>,
) {
    for block in order
        .iter()
        .copied()
        .filter(|block| planned_blocks.contains(block))
    {
        let mut previous = None;
        let updates = graph[block]
            .items()
            .iter()
            .enumerate()
            .filter_map(|(offset, item)| match item {
                Item::Label(label) => {
                    if !internal_labels.contains(label) {
                        previous = None;
                    }
                    None
                }
                Item::Instruction(instruction) => {
                    if instruction.location.line >= 0
                        || instruction.no_location_policy.preserves_location()
                    {
                        previous = Some(instruction.location);
                        None
                    } else {
                        previous.map(|location| (ItemPosition::new(block, offset), location))
                    }
                }
            })
            .collect::<Vec<_>>();
        for (position, location) in updates {
            graph
                .instruction_mut(position)
                .expect("planned array instruction")
                .location = location;
        }
    }
}

const WARM_BLOCK: u8 = 1;
const COLD_BLOCK: u8 = 2;

/// Resolves a virtual interruptible jump when cold-block ordering does not run.
///
/// Item positions distinguish labels before and after a jump within the same block.
fn normalize_unmoved_interruptible_jumps(graph: &mut BlockGraph) {
    if !graph.features().has_forward_jump() {
        return;
    }
    let items = graph
        .order()
        .iter()
        .copied()
        .flat_map(|block| {
            graph[block]
                .items()
                .iter()
                .enumerate()
                .map(move |(offset, item)| (ItemPosition::new(block, offset), *item))
        })
        .collect::<Vec<_>>();
    let labels = items
        .iter()
        .enumerate()
        .filter_map(|(index, (_, item))| match item {
            Item::Label(label) => Some((*label, index)),
            Item::Instruction(_) => None,
        })
        .collect::<FxHashMap<_, _>>();
    for (index, (position, item)) in items.iter().enumerate() {
        let Item::Instruction(instruction) = item else {
            continue;
        };
        if instruction.opcode != JUMP_BACKWARD {
            continue;
        }
        let Operand::Forward(target) = instruction.operand() else {
            continue;
        };
        let Some(&target_position) = labels.get(&target) else {
            continue;
        };
        let instruction = graph
            .instruction_mut(*position)
            .expect("original virtual jump position");
        if target_position <= index {
            instruction.retarget(Operand::Backward(target));
        } else {
            instruction.opcode = JUMP_FORWARD;
        }
    }
}

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

/// Keeps a surviving original shared synthetic forward-jump array source-less after exit copying.
/// Authored breaks retain deferred cleanup donors that can resolve after later jump threading.
fn preserve_original_shared_jump_location(
    graph: &mut BlockGraph,
    target: BasicBlockId,
    predecessors: usize,
    original_location_absence: &FxHashSet<Label>,
) -> bool {
    if predecessors <= 1
        || !original_entry_has_no_location_donor(graph[target].items(), original_location_absence)
    {
        return false;
    }
    let mut instructions = graph[target].items().iter().filter_map(|item| match item {
        Item::Instruction(instruction) => Some(instruction),
        Item::Label(_) => None,
    });
    let Some(jump) = instructions.next() else {
        return false;
    };
    if instructions.next().is_some()
        || jump.opcode != JUMP_FORWARD
        || !matches!(jump.operand(), Operand::Forward(_))
        || jump.location.line >= 0
        || !matches!(
            jump.jump_policy.retained_nop(),
            super::super::ir::RetainedJumpNop::DirectTarget
        )
        || jump.jump_policy.is_interruptible()
        || jump.fusion_policy.is_authored_break()
        || jump.fusion_policy.location_from_next()
        || jump.no_location_policy.percent_target_location != 0
    {
        return false;
    }
    // Copies have new labels, and an original array that became private has one
    // predecessor. Both retain their own donor; only the still-shared original
    // has the certified absence, which later location writers must also respect.
    graph
        .instructions_mut(target)
        .next()
        .expect("certified original forward jump")
        .no_location_policy
        .preserve_location();
    true
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

/// Certifies a local raise without copied return/finally unwind records.
///
/// One original converted `POP_BLOCK` may precede the two-record raise. Authored
/// or folded-away NOPs do not certify that original array's size.
fn block_is_local_raise(items: &[Item], original_nop_targets: &FxHashSet<Label>) -> bool {
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
    let Some(mut first) = instructions.next() else {
        return false;
    };
    if first.opcode == NOP {
        if first.operand() != Operand::Value(0)
            || first.location != SourceLocation::NONE
            || !first.fusion_policy.is_converted_pop_block()
            || first.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || first.pre_fold_extra_instructions() != 0
            || first.local_policy.hidden_exception_owner_slots() != 0
        {
            return false;
        }
        let Some(local) = instructions.next() else {
            return false;
        };
        first = local;
    }
    first.opcode == LOAD_FAST
        && first.pre_fold_extra_instructions() == 0
        && first.local_policy.hidden_exception_owner_slots() == 0
        && instructions.next().is_some_and(|raise| {
            raise.opcode == RAISE_VARARGS
                && raise.operand() == Operand::Value(1)
                && raise.pre_fold_extra_instructions() == 0
                && raise.local_policy.hidden_exception_owner_slots() == 0
        })
        && instructions.next().is_none()
}

/// Recognizes a named-handler cleanup followed by a short, located value return.
fn named_cleanup_has_short_return(graph: &BlockGraph, target: BasicBlockId) -> bool {
    let cleanup = graph[target]
        .items()
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    let [pop, value, store, delete, jump] = cleanup.as_slice() else {
        return false;
    };
    if pop.opcode != POP_EXCEPT
        || value.opcode != LOAD_CONST
        || store.opcode != STORE_FAST
        || delete.opcode != DELETE_FAST
        || store.operand() != delete.operand()
        || jump.opcode != JUMP_FORWARD
        || cleanup.iter().any(|instruction| {
            instruction.pre_fold_extra_instructions() != 0
                || instruction.local_policy.hidden_exception_owner_slots() != 0
        })
    {
        return false;
    }
    let Operand::Forward(label) = jump.operand() else {
        return false;
    };
    let Some(successor) = graph.label_block(label) else {
        return false;
    };
    let mut returned = graph[successor]
        .items()
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        });
    let Some(mut value) = returned.next() else {
        return false;
    };
    if value.opcode == NOP {
        if value.pre_fold_extra_instructions() != 0
            || value.local_policy.hidden_exception_owner_slots() != 0
        {
            return false;
        }
        let Some(next) = returned.next() else {
            return false;
        };
        value = next;
    }
    let Some(exit) = returned.next() else {
        return false;
    };
    matches!(value.opcode, LOAD_FAST | LOAD_CONST)
        && exit.opcode == RETURN_VALUE
        && returned.next().is_none()
        && [value, exit].iter().all(|instruction| {
            instruction.location.line >= 0
                && instruction.pre_fold_extra_instructions() == 0
                && instruction.local_policy.hidden_exception_owner_slots() == 0
        })
}

// Every restored POP_BLOCK is outside ordinary handlers but inside the scope fallback.
// Reapply its exact one-slot hole after all source and copied regions have been formed.
fn restore_normal_try_pop_holes(
    graph: &BlockGraph,
    regions: &mut Vec<ExceptionRegion>,
    ranges: &[(Label, Label)],
) {
    if ranges.is_empty() {
        return;
    }
    let mut offsets = FxHashMap::default();
    let mut instructions = Vec::new();
    for item in graph.iter_items() {
        match item {
            Item::Label(label) => {
                offsets.insert(*label, instructions.len());
            }
            Item::Instruction(instruction) => instructions.push(instruction),
        }
    }
    let mut holes = Vec::new();
    for &(start, end) in ranges {
        let (first, last) = match (offsets.get(&start), offsets.get(&end)) {
            (Some(&first), Some(&last)) => (first, last),
            // An entirely discarded original/copy has no remaining slot or range.
            (None, None) => continue,
            _ => panic!("original cleanup copy lost one POP_BLOCK boundary"),
        };
        // Dead originals can retain their labels for exception bookkeeping.
        if first == last {
            continue;
        }
        assert_eq!(last, first + 1, "original POP_BLOCK hole changed width");
        let pop = instructions[first];
        assert!(
            pop.opcode == NOP && pop.fusion_policy.is_converted_pop_block(),
            "original POP_BLOCK hole no longer contains its converted record"
        );
        holes.push((first, last, start, end));
    }
    holes.sort_by_key(|&(first, last, _, _)| (first, last));
    assert!(
        holes.windows(2).all(|pair| pair[0].1 <= pair[1].0),
        "original POP_BLOCK holes overlap"
    );
    let original_regions = std::mem::take(regions);
    for region in original_regions {
        if region.is_scope_fallback {
            regions.push(region);
            continue;
        }
        let first = offsets[&region.start];
        let last = offsets[&region.end];
        let mut start = region.start;
        for &(lo, hi, pop_start, pop_end) in &holes {
            if lo < last && first < hi {
                assert!(
                    first <= lo && hi <= last,
                    "region partly intersects original POP_BLOCK"
                );
                regions.push(ExceptionRegion {
                    start,
                    end: pop_start,
                    ..region
                });
                start = pop_end;
            }
        }
        regions.push(ExceptionRegion { start, ..region });
    }
}

/// Identifies a range reentry that adds no original SETUP or POP record.
fn is_metadata_range_entry(
    label: Label,
    exception_regions: &[ExceptionRegion],
    original_setups: &[(Label, Option<SourceLocation>)],
) -> bool {
    exception_regions.iter().any(|region| region.start == label)
        && exception_regions
            .iter()
            .all(|region| region.end != label && region.target != label)
        && original_setups.iter().all(|(setup, _)| *setup != label)
}

/// Finds original SETUP records already represented by a retained NOP in this target chain.
fn represented_setup_nops(
    graph: &BlockGraph,
    blocks: &[BasicBlockId],
    provenance: &ExceptionOwnerProvenance,
) -> FxHashSet<Label> {
    if blocks.len() <= 1
        || provenance.setup_nop_ranges.is_empty()
        || !blocks.iter().any(|block| {
            graph[*block].items().iter().any(
                |item| matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP),
            )
        })
    {
        return FxHashSet::default();
    }
    let mut positions = FxHashMap::default();
    let mut instructions = Vec::new();
    for item in blocks.iter().flat_map(|block| graph[*block].items()) {
        match item {
            Item::Label(label) => {
                positions.insert(*label, instructions.len());
            }
            Item::Instruction(instruction) => instructions.push(*instruction),
        }
    }
    provenance
        .setups
        .iter()
        .filter_map(|&(setup, location)| {
            let location = location?;
            let &index = positions.get(&setup)?;
            let nop_index = index.checked_sub(1)?;
            let nop = instructions.get(nop_index)?;
            let mut expected =
                Instruction::new(NOP, Operand::Value(0), location, Some(nop.depth_after()?));
            expected.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            if location.line < 0
                || !provenance.ordinary_try_setups.contains_key(&setup)
                || *nop != expected
                || provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| positions.get(label) == Some(&index))
                    .count()
                    != 1
                || provenance
                    .setup_nop_ranges
                    .iter()
                    .filter(|(start, end)| {
                        positions.get(start) == Some(&nop_index)
                            && positions.get(end) == Some(&index)
                    })
                    .count()
                    != 1
            {
                return None;
            }
            Some(setup)
        })
        .collect()
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
    retired_exception_handler_targets: &FxHashSet<Label>,
    original_nop_targets: &FxHashSet<Label>,
    provenance: &ExceptionOwnerProvenance,
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
        .filter(|region| !retired_exception_handler_targets.contains(&region.target))
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
            && ((target_has_no_location
                && (ends_scope(target_last.opcode) || has_jump_eval_check(target_last)))
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
                || block_is_local_raise(graph[target].items(), original_nop_targets)
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
            && block_jump_target(graph[order[target_chain_end]].items()).is_none()
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
        let represented_setups = represented_setup_nops(graph, target_blocks, provenance);
        let mut at_target_entry = true;
        let target_pre_fusion_size = target_blocks
            .iter()
            .flat_map(|block| graph[*block].items())
            .map(|item| match item {
                Item::Instruction(instruction) => {
                    at_target_entry = false;
                    let fused_push_null = instruction.opcode == LOAD_GLOBAL
                        && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0);
                    1 + usize::from(fused_push_null)
                        + usize::from(instruction.pre_fold_extra_instructions())
                }
                Item::Label(label) => usize::from(
                    target_chain_end > target_position
                        && exception_split_labels.contains(label)
                        && !provenance.return_region_reentries.contains(label)
                        && !represented_setups.contains(label)
                        && !(at_target_entry
                            && is_metadata_range_entry(
                                *label,
                                exception_regions,
                                &provenance.setups,
                            )),
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
                && (block_is_bare_raise(graph[target].items(), original_nop_targets)
                    || block_is_local_raise(graph[target].items(), original_nop_targets)))
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
            && ((target_has_no_location
                && (ends_scope(target_last.opcode)
                    || graph[target_exit]
                        .last_instruction()
                        .is_some_and(has_jump_eval_check)))
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

// A preserved owner boundary can split an original label-only prefix from its POP_BLOCK.
// Follow only that captured prefix, after copying has fixed the reachable predecessors.
fn restore_retained_pop_block_locations(
    graph: &mut BlockGraph,
    exception_regions: &[ExceptionRegion],
    preserved_boundaries: &FxHashSet<Label>,
    origins: &FxHashMap<Label, Vec<Label>>,
) {
    if origins.is_empty() {
        return;
    }
    let reachable = graph.reachable_blocks(exception_regions);
    let order = graph.order().to_vec();
    let mut incoming = vec![(0usize, None); reachable.len()];
    if let Some(entry) = order.first() {
        incoming[entry.index()].0 = 1;
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
            incoming[target.index()].0 += 1;
            incoming[target.index()].1 = Some(source);
        }
    }
    let handlers = exception_regions
        .iter()
        .filter_map(|region| graph.label_block(region.target))
        .collect::<FxHashSet<_>>();
    let mut ordered_origins = origins.iter().collect::<Vec<_>>();
    ordered_origins.sort_unstable_by_key(|(entry, _)| entry.0);
    for (entry, prefix) in ordered_origins {
        let Some(target) = graph.label_block(*entry) else {
            continue;
        };
        if !reachable[target.index()]
            || !preserved_boundaries.contains(entry)
            || !graph[target]
                .items()
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .any(|item| matches!(item, Item::Label(label) if label == entry))
        {
            continue;
        }
        let Some(first) = graph[target].items().iter().find_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        }) else {
            continue;
        };
        if first.opcode != NOP
            || !first.fusion_policy.is_converted_pop_block()
            || !first.jump_policy.preserves_explicit_nop()
            || first.jump_policy.borrow_empty_block()
            || first.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || first.location != SourceLocation::NONE
            || first.no_location_policy.preserves_location()
        {
            continue;
        }
        let mut block = target;
        let mut visited = FxHashSet::default();
        let mut crossed_empty = false;
        let location = loop {
            if !visited.insert(block) || handlers.contains(&block) {
                break None;
            }
            let (1, Some(source)) = incoming[block.index()] else {
                break None;
            };
            if handlers.contains(&source) {
                break None;
            }
            if let Some(instruction) = graph[source].last_instruction() {
                break (crossed_empty
                    && is_unconditional_jump(instruction.opcode)
                    && block_jump_target(graph[source].items())
                        .and_then(|label| graph.label_block(label))
                        == Some(block)
                    && instruction.location.line >= 0)
                    .then_some(instruction.location);
            }
            if !graph[source]
                .items()
                .iter()
                .all(|item| matches!(item, Item::Label(label) if prefix.contains(label)))
            {
                break None;
            }
            crossed_empty = true;
            block = source;
        };
        if let Some(location) = location
            && let Some(first) = graph.instructions_mut(target).next()
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

/// Certifies the original normal jump immediately following a recorded `POP_BLOCK`.
fn source_has_original_pop_jump<'a>(
    graph: &'a BlockGraph,
    source: BasicBlockId,
    ranges: &[(Label, Label)],
    provenance: &super::super::exception_owners::ExceptionOwnerProvenance,
    preserved_boundaries: &FxHashSet<Label>,
    exception_regions: &[ExceptionRegion],
    pending_items: impl Iterator<Item = &'a Item>,
) -> bool {
    let Some((Item::Instruction(jump), prefix)) = graph[source].items().split_last() else {
        return false;
    };
    if !is_unconditional_jump(jump.opcode)
        || !matches!(jump.operand(), Operand::Forward(_))
        || jump.pre_fold_extra_instructions() != 0
        || jump.local_policy.hidden_exception_owner_slots() != 0
    {
        return false;
    }
    let Some(pop_offset) = prefix
        .iter()
        .rposition(|item| matches!(item, Item::Instruction(_)))
    else {
        return false;
    };
    let Item::Instruction(pop) = prefix[pop_offset] else {
        unreachable!();
    };
    if pop.opcode != NOP
        || !pop.fusion_policy.is_converted_pop_block()
        || pop.pre_fold_extra_instructions() != 0
        || pop.local_policy.hidden_exception_owner_slots() != 0
    {
        return false;
    }
    let Some(&(_, end)) = ranges.iter().find(|(start, end)| {
        if provenance.retired_normal_try_copy_labels.contains(start)
            || provenance.retired_normal_try_copy_labels.contains(end)
        {
            return false;
        }
        let start = prefix
            .iter()
            .position(|item| matches!(item, Item::Label(label) if label == start));
        let end = prefix
            .iter()
            .position(|item| matches!(item, Item::Label(label) if label == end));
        matches!((start, end), (Some(start), Some(end))
            if start < pop_offset
                && pop_offset < end
                && prefix[start..pop_offset].iter().all(|item| matches!(item, Item::Label(_))))
    }) else {
        return false;
    };
    let continuation_labels = prefix[pop_offset + 1..]
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) => Some(*label),
            Item::Instruction(_) => None,
        })
        .collect::<FxHashSet<_>>();
    if continuation_labels.iter().any(|label| {
        provenance.retired_normal_try_copy_labels.contains(label)
            || (*label != end
                && !provenance.normal_try_copy_internal_labels.contains(label)
                && !provenance.normal_try_copy_inline_labels.contains(label))
            || preserved_boundaries.contains(label)
            || exception_regions
                .iter()
                .any(|region| region.target == *label)
            || provenance.setups.iter().any(|(entry, _)| entry == label)
    }) {
        return false;
    }
    // A genuine internal endpoint is not an independently entered jump array.
    // Include copies waiting to be appended so their edges cannot hide an entry.
    !graph.iter_items().chain(pending_items).any(|item| {
        matches!(item, Item::Instruction(instruction)
            if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                if continuation_labels.contains(&label)))
    })
}

// The original pass performs these location repairs even when it creates no copies, so the fast
// path must preserve them before returning.

/// Propagates a resolved `POP_BLOCK` line into its original following jump slot.
fn propagate_normal_try_pop_continuations(
    graph: &mut BlockGraph,
    ranges: &[(Label, Label)],
    forbidden_entries: &FxHashSet<Label>,
) {
    let mut offsets = FxHashMap::default();
    let mut instructions = Vec::new();
    let mut incoming = forbidden_entries.clone();
    for (block, contents) in graph.iter() {
        for (offset, item) in contents.items().iter().enumerate() {
            match item {
                Item::Label(label) => {
                    offsets.insert(*label, instructions.len());
                }
                Item::Instruction(instruction) => {
                    if let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                    {
                        incoming.insert(label);
                    }
                    instructions.push((ItemPosition::new(block, offset), *instruction));
                }
            }
        }
    }
    let mut updates = Vec::new();
    for &(start, end) in ranges {
        let (Some(&start), Some(&end)) = (offsets.get(&start), offsets.get(&end)) else {
            continue;
        };
        if end != start + 1 {
            continue;
        }
        let (_, pop) = instructions[start];
        let Some(&(position, next)) = instructions.get(end) else {
            continue;
        };
        if pop.opcode != NOP
            || !pop.fusion_policy.is_converted_pop_block()
            || pop.location.line < 0
            || !(is_unconditional_jump(next.opcode) || next.opcode == NOP)
            || next.location.line >= 0
            || next.no_location_policy.preserves_location()
            || offsets
                .iter()
                .any(|(label, offset)| *offset == end && incoming.contains(label))
        {
            continue;
        }
        updates.push((position, pop.location));
    }
    for (position, location) in updates {
        graph
            .instruction_mut(position)
            .expect("original try continuation")
            .location = location;
    }
}

/// Replays only the recorded SWAP-to-NOP conversion and complete store exchange.
fn static_swap_tail_location(
    source: &[Item],
    [swap, first, last]: [Instruction; 3],
) -> Option<SourceLocation> {
    let mut removed_swap = swap;
    removed_swap.opcode = NOP;
    removed_swap.set_operand(Operand::Value(0));
    (source
        == [
            Item::Instruction(removed_swap),
            Item::Instruction(last),
            Item::Instruction(first),
        ])
    .then_some(last.location)
}

/// Replays the original store donor only across the exact static-SWAP permutation.
fn comprehension_assignment_exit_location(
    source: &[Item],
    target: &[Item],
    origins: &FxHashMap<Label, (Vec<Label>, [Instruction; 3])>,
) -> Option<SourceLocation> {
    let entry_len = target
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let jump = match &target[entry_len..] {
        [Item::Instruction(jump)] if entry_len == 1 => jump,
        [Item::Instruction(pop), Item::Instruction(jump)]
            if pop.opcode == POP_EXCEPT && pop.location.line < 0 =>
        {
            jump
        }
        _ => return None,
    };
    if jump.opcode != JUMP_FORWARD || jump.location.line >= 0 {
        return None;
    }
    let (entries, instructions) = target[..entry_len].iter().find_map(|item| match item {
        Item::Label(exit) => origins.get(exit),
        Item::Instruction(_) => None,
    })?;
    if source.len() != entries.len() + 3 {
        return None;
    }
    let (labels, tail) = source.split_at(entries.len());
    if !labels
        .iter()
        .copied()
        .eq(entries.iter().copied().map(Item::Label))
    {
        return None;
    }
    static_swap_tail_location(tail, *instructions)
}

/// Original triple-store fallthroughs whose unnamed handler cleanup is shared.
pub(in crate::assembler) struct TripleStoreHandlerExits {
    original_labels: FxHashSet<Label>,
    entries: FxHashMap<Label, TripleStoreHandlerExit>,
}

struct TripleStoreHandlerExit {
    swapped_source: Vec<Item>,
    cleanup: Vec<Item>,
    donor: SourceLocation,
}

impl TripleStoreHandlerExits {
    /// Replays only the captured SWAP-to-NOP conversion and complete store exchange.
    fn location(&self, source: &[Item], target: &[Item]) -> Option<SourceLocation> {
        let origin = target
            .iter()
            .take_while(|item| matches!(item, Item::Label(_)))
            .find_map(|item| match item {
                Item::Label(label) => self.entries.get(label),
                Item::Instruction(_) => None,
            })?;
        if target != origin.cleanup.as_slice() || !source.ends_with(&origin.swapped_source) {
            return None;
        }
        // Exit copying can prepend fresh aliases, but cannot replace an original
        // boundary, insert an instruction, or alter any captured source record.
        let aliases = &source[..source.len() - origin.swapped_source.len()];
        aliases
            .iter()
            .all(|item| {
                matches!(item,
            Item::Label(label) if !self.original_labels.contains(label))
            })
            .then_some(origin.donor)
    }
}

/// Predicts the surviving donor of a reversed store pair at the end of an array.
fn fused_store_tail_location(source: &[Item]) -> Option<SourceLocation> {
    let [.., Item::Instruction(first), Item::Instruction(second)] = source else {
        return None;
    };
    if first.opcode != STORE_FAST || second.opcode != STORE_FAST {
        return None;
    }
    let fused = super::locals::fuse_pair(first, second)?;
    if fused.location.line < 0 || fused.location.column <= second.location.column {
        return None;
    }
    let start = source
        .iter()
        .rposition(|item| !matches!(item, Item::Instruction(instruction) if instruction.opcode == STORE_FAST))
        .map_or(0, |offset| offset + 1);
    let stores = &source[start..];
    if stores
        .iter()
        .any(|item| matches!(item, Item::Instruction(instruction) if instruction.location.line < 0))
    {
        return None;
    }
    // No preceding non-store can fuse with this run. Its resolved locations also
    // cannot change through the fuser's forward-location donation. Walk the same
    // non-overlapping pairs: an odd run can leave the final store unfused.
    let tail = stores.len() - 2;
    let mut offset = 0;
    while offset < tail {
        offset += match (&stores[offset], &stores[offset + 1]) {
            (Item::Instruction(first), Item::Instruction(second))
                if super::locals::fuse_pair(first, second).is_some() =>
            {
                2
            }
            _ => 1,
        };
    }
    (offset == tail).then_some(fused.location)
}

/// Carries a named-handler cleanup's line across its exception-end split.
fn named_handler_cleanup_exit_location(source: &[Item], target: &[Item]) -> Option<SourceLocation> {
    let source_entry_len = source
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [
        Item::Instruction(pop),
        Item::Instruction(load),
        Item::Instruction(store),
        Item::Instruction(delete),
    ] = &source[source_entry_len..]
    else {
        return None;
    };
    let target_entry_len = target
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let target_body = &target[target_entry_len..];
    let supported_exit = match target_body {
        [Item::Instruction(jump)] => {
            is_unconditional_jump(jump.opcode)
                && jump.location.line < 0
                && jump.no_location_policy.allows_block_inlining()
                && !jump.no_location_policy.preserves_location()
        }
        [
            Item::Instruction(outer_pop),
            clearing @ ..,
            Item::Instruction(none),
            Item::Instruction(ret),
        ] => {
            outer_pop.opcode == POP_EXCEPT
                && none.opcode == LOAD_CONST
                && none.operand() == load.operand()
                && ret.opcode == RETURN_VALUE
                && match clearing {
                    [] => true,
                    [
                        Item::Instruction(clear),
                        Item::Instruction(store),
                        Item::Instruction(delete),
                    ] => {
                        clear.opcode == LOAD_CONST
                            && clear.operand() == load.operand()
                            && store.opcode == STORE_FAST
                            && delete.opcode == DELETE_FAST
                            && store.operand() == delete.operand()
                    }
                    _ => false,
                }
                && target_body.iter().all(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if instruction.location.line < 0
                            && !instruction.no_location_policy.preserves_location())
                })
        }
        _ => false,
    };
    (pop.opcode == POP_EXCEPT
        && pop.location.line >= 0
        && load.opcode == LOAD_CONST
        && store.opcode == STORE_FAST
        && delete.opcode == DELETE_FAST
        && store.operand() == delete.operand()
        && [load, store, delete].iter().all(|instruction| {
            instruction.location.line < 0 && !instruction.no_location_policy.preserves_location()
        })
        && target_entry_len > 0
        && supported_exit)
        .then_some(pop.location)
}

/// Resolves only an original match cleanup whose complete fallthrough array still matches.
fn match_join_store_exit_location(
    source: &[Item],
    target: &[Item],
    origins: &FxHashMap<Label, Vec<Item>>,
) -> Option<SourceLocation> {
    let entry_len = target
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [Item::Instruction(pop), Item::Instruction(jump)] = &target[entry_len..] else {
        return None;
    };
    if pop.opcode != POP_EXCEPT
        || pop.location.line >= 0
        || jump.opcode != JUMP_FORWARD
        || jump.location.line >= 0
    {
        return None;
    }
    let original = target[..entry_len].iter().find_map(|item| match item {
        Item::Label(label) => origins.get(label),
        Item::Instruction(_) => None,
    })?;
    let [
        prefix @ ..,
        Item::Instruction(swap),
        Item::Instruction(first),
        Item::Instruction(last),
    ] = original.as_slice()
    else {
        return None;
    };
    if source.len() != original.len() || source[..prefix.len()] != *prefix {
        return None;
    }
    static_swap_tail_location(&source[prefix.len()..], [*swap, *first, *last])
}

fn propagate_exit_block_locations(
    graph: &mut BlockGraph,
    order: &[BasicBlockId],
    analysis: &ExitBlockAnalysis,
    exception_regions: &[ExceptionRegion],
    original_boundaries: &FxHashSet<Label>,
    original_location_absence: &FxHashSet<Label>,
    comprehension_assignment_exits: &FxHashMap<Label, (Vec<Label>, [Instruction; 3])>,
    preserved_boundaries: &FxHashSet<Label>,
    source_boundaries: &FxHashSet<Label>,
    setups: &[(Label, Option<SourceLocation>)],
) {
    for target in order.iter().copied() {
        if analysis.exception_handler_blocks[target.index()] {
            continue;
        }
        if preserve_original_shared_jump_location(
            graph,
            target,
            analysis.predecessors[target.index()],
            original_location_absence,
        ) {
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
            if let Some(location) = comprehension_assignment_exit_location(
                graph[source].items(),
                graph[target].items(),
                comprehension_assignment_exits,
            ) {
                return Some(location);
            }
            if analysis.predecessors[target.index()] == 1
                && analysis.positions[source.index()] + 1 == target_position
                && block_has_fallthrough(graph[source].items())
                && let Some(location) = named_handler_cleanup_exit_location(
                    graph[source].items(),
                    graph[target].items(),
                )
                && graph[target]
                    .items()
                    .iter()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .all(|item| {
                        let Item::Label(label) = item else {
                            unreachable!();
                        };
                        exception_regions.iter().any(|region| region.end == *label)
                            && !preserved_boundaries.contains(label)
                            && !source_boundaries.contains(label)
                            && !original_boundaries.contains(label)
                            && !setups.iter().any(|(setup, _)| setup == label)
                    })
            {
                // The inner POP_EXCEPT has its sole predecessor's line already.
                // Its unlocated name-clearing tail receives that line later.
                return Some(location);
            }
            let last = graph[source].last_instruction()?;
            // A saved return crosses the artificial SWAP before an unnamed handler's
            // two-instruction exit. Propagate the same original block's finalizer line
            // before the later SWAP optimization removes that last donor boundary.
            if analysis.predecessors[target.index()] == 1
                && last.opcode == SWAP
                && matches!(last.operand(), Operand::Value(2))
                && last.location.line < 0
                && !last.no_location_policy.preserves_location()
            {
                let mut exit = graph[target].items().iter().filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                });
                if exit.next() == Some(POP_EXCEPT)
                    && exit.next() == Some(RETURN_VALUE)
                    && exit.next().is_none()
                {
                    let previous = graph[source]
                        .items()
                        .iter()
                        .rev()
                        .filter_map(|item| match item {
                            Item::Instruction(instruction) => Some(instruction),
                            Item::Label(_) => None,
                        })
                        .nth(1);
                    if let Some(previous) = previous
                        && matches!(previous.opcode, POP_TOP | STORE_ATTR | DELETE_FAST)
                        && previous.location.line >= 0
                    {
                        return Some(previous.location);
                    }
                }
            }
            Some(last.location)
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
    /// Resolves synthesized percent-format literals before their location donors are optimized.
    ///
    /// CPython's early line-number propagation gives this leading constant the sole
    /// predecessor's condition location. Serc retains the eventual percent-expression
    /// location on some intervening jumps, so recover the earlier location from the jump's
    /// preceding instruction or, for a split jump block, its sole predecessor.
    pub(in crate::assembler) fn resolve_synthesized_percent_locations(&mut self) {
        if !self.has_synthesized_percent_target_locations {
            return;
        }

        let order = self.graph.order().to_vec();
        // CPython propagates locations before fusing adjacent local loads. Retain the
        // second load's location for a following literal before fusion removes that donor.
        for block in order.iter().copied() {
            let mut previous = SourceLocation::NONE;
            for instruction in self.graph.instructions_mut(block) {
                if instruction.location.line >= 0 {
                    previous = instruction.location;
                } else if instruction.no_location_policy.percent_target_location == u8::MAX
                    && previous.line >= 0
                {
                    instruction.location = previous;
                    instruction.no_location_policy.percent_target_location = 0;
                }
            }
        }
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

    /// Reverses backward conditionals around a fresh interruptible backedge.
    pub(in crate::assembler) fn normalize_backward_conditionals(
        &mut self,
    ) -> Result<(), crate::CompileError> {
        let mut position = 0;
        while position < self.graph.order().len() {
            let block = self.graph.block_at(position);
            let Some(instruction) = self.graph[block].last_instruction().copied() else {
                position += 1;
                continue;
            };
            let Operand::Backward(target) = instruction.operand() else {
                position += 1;
                continue;
            };
            let reversed = match instruction.opcode {
                POP_JUMP_IF_FALSE => POP_JUMP_IF_TRUE,
                POP_JUMP_IF_TRUE => POP_JUMP_IF_FALSE,
                POP_JUMP_IF_NONE => POP_JUMP_IF_NOT_NONE,
                POP_JUMP_IF_NOT_NONE => POP_JUMP_IF_NONE,
                _ => {
                    position += 1;
                    continue;
                }
            };
            // Code generation already emitted the forward form's marker. CPython
            // instead creates it in the new backedge block at this late stage.
            let marker = self.graph.order()[position + 1..].iter().find_map(|block| {
                self.graph[*block].items().iter().enumerate().find_map(
                    |(offset, item)| match item {
                        Item::Instruction(marker) => {
                            Some((*block, ItemPosition::new(*block, offset), marker.opcode))
                        }
                        Item::Label(_) => None,
                    },
                )
            });
            let Some((_marker_block, marker, NOT_TAKEN)) = marker else {
                return Err(crate::CompileError::Internal(
                    "backward conditional has no fallthrough marker".to_owned(),
                ));
            };
            // Remove the marker before inserting a successor label can shift its offset.
            self.graph.remove_item(marker);
            let successor = self.graph.block_at(position + 1);
            let fallthrough =
                if let Some(Item::Label(label)) = self.graph[successor].items().first() {
                    *label
                } else {
                    let label = self.label();
                    self.graph.insert_label(successor, 0, label);
                    label
                };
            let fresh_entry = self.label();
            let conditional = self.graph.instructions_mut(block).next_back().unwrap();
            conditional.opcode = reversed;
            conditional.retarget(Operand::Forward(fallthrough));
            let mut marker = Instruction::synthetic(
                NOT_TAKEN,
                Operand::Value(0),
                instruction.location,
                instruction.depth_after(),
            );
            let mut backedge = Instruction::synthetic(
                JUMP_BACKWARD,
                Operand::Backward(target),
                instruction.location,
                instruction.depth_after(),
            );
            // The new CPython instruction array starts with null exception owners.
            marker.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
            backedge.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
            self.preserved_block_boundaries.insert(fresh_entry);
            self.graph.insert_block(
                position + 1,
                vec![
                    Item::Label(fresh_entry),
                    Item::Instruction(marker),
                    Item::Instruction(backedge),
                ],
            );
            position += 2;
        }
        Ok(())
    }

    pub(in crate::assembler) fn push_cold_blocks_to_end(
        &mut self,
        mut owner_retargets: Option<&mut Vec<(ExceptionRegion, ExceptionRegion)>>,
    ) -> FxHashSet<Label> {
        if self.exception_regions.is_empty() {
            normalize_unmoved_interruptible_jumps(&mut self.graph);
            return FxHashSet::default();
        }
        let graph = &mut self.graph;
        if graph.len() < 2 {
            normalize_unmoved_interruptible_jumps(graph);
            return FxHashSet::default();
        }

        // Constant-true while lowering aliases CPython's empty condition anchor
        // with the shared loop exit. Restore that original array before cold
        // relocation: its synthetic bridge cannot inherit a line from the earlier
        // break across the empty anchor. Ordinary nonempty cleanup bridges retain
        // their existing location propagation.
        let loop_anchor_targets = self
            .folded_predecessor_sources
            .iter()
            .filter_map(|&(source, target, location)| {
                if !self.folded_loop_exit_anchors.contains(&target) {
                    return None;
                }
                let source = graph.label_block(source)?;
                let marker = graph[source].items().iter().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })?;
                // A removed marker can leave a label alias on unrelated live code.
                (marker.opcode == NOP && marker.location == location)
                    .then(|| graph.label_block(target))
                    .flatten()
            })
            .collect::<FxHashSet<_>>();
        let loop_anchor_positions = graph
            .order()
            .iter()
            .enumerate()
            .filter_map(|(position, block)| loop_anchor_targets.contains(block).then_some(position))
            .collect::<Vec<_>>();
        for position in loop_anchor_positions.into_iter().rev() {
            let anchor = Label(self.next_label);
            self.next_label += 1;
            graph.insert_block(position, vec![Item::Label(anchor)]);
        }

        // Loop-tail lowering can omit the original array containing the final backedge.
        // CPython keeps that array after its instructions disappear. A surviving FOR_ITER
        // supplies provenance only when its cleanup target is an original recorded label.
        let omitted_targets = graph
            .iter()
            .filter_map(|(_, block)| {
                let last = block.last_instruction()?;
                let Operand::Forward(target) = last.operand() else {
                    return None;
                };
                (last.opcode == FOR_ITER && self.borrow_omitted_for_backedges.contains(&target))
                    .then_some(target)
            })
            .collect::<FxHashSet<_>>();
        let cleanup_positions = graph
            .order()
            .iter()
            .enumerate()
            .filter_map(|(position, block)| {
                graph[*block]
                    .items()
                    .iter()
                    .any(|item| matches!(item, Item::Label(label) if omitted_targets.contains(label)))
                    .then_some(position)
            })
            .collect::<Vec<_>>();
        let mut omitted_backedges = FxHashSet::default();
        let mut omitted_markers = FxHashSet::default();
        for position in cleanup_positions.into_iter().rev() {
            let label = Label(self.next_label);
            self.next_label += 1;
            // Keep the array empty while physical jumps and locations are finalized.
            let block = graph.insert_block(position, vec![Item::Label(label)]);
            omitted_backedges.insert(block);
            omitted_markers.insert(label);
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
            normalize_unmoved_interruptible_jumps(graph);
            return omitted_markers;
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
            let empty_while_anchor = self
                .exception_owner_provenance
                .empty_while_break_anchors
                .iter()
                .any(|&(anchor, end, original)| {
                    if !graph[next]
                        .items()
                        .starts_with(&[Item::Label(anchor), Item::Label(end)])
                    {
                        return false;
                    }
                    let mut marker = original;
                    marker.opcode = NOP;
                    marker.set_operand(Operand::Value(0));
                    graph[current].last_instruction() == Some(&marker)
                });
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
            let mut bridge = Instruction::synthetic(
                JUMP_BACKWARD_NO_INTERRUPT,
                Operand::Backward(target),
                location,
                None,
            );
            bridge.exclude_from_exception_table(exception_exclusion);
            if empty_while_anchor {
                // CPython's empty original anchor separates the authored break NOP
                // from this new cold bridge. It cannot donate its line across that
                // array, and must survive the later equal-next-line NOP removal.
                bridge.location = SourceLocation::NONE;
                bridge.no_location_policy.preserve_location();
            }
            for origin in &mut self.exception_owner_provenance.async_with_setups {
                if origin.bridge.is_some()
                    || graph[current].items() != origin.cleanup.as_slice()
                    || graph[next].items() != origin.prefix.as_slice()
                    || graph.order().get(position + 2).is_none_or(|after| {
                        graph[*after].items().first() != Some(&Item::Label(origin.setup))
                    })
                {
                    continue;
                }
                let mut incoming = graph.iter().flat_map(|(_, block)| block.items()).filter_map(|item| match item {
                    Item::Instruction(instruction)
                        if matches!(instruction.operand(), Operand::Forward(entry) | Operand::Backward(entry)
                            if origin.prefix.contains(&Item::Label(entry)) || entry == origin.setup) => Some(instruction),
                    _ => None,
                });
                if incoming.next() == Some(&origin.send) && incoming.next().is_none() {
                    origin.bridge = Some((label, bridge));
                }
            }
            for origin in &mut self.exception_owner_provenance.ordinary_await_ends {
                if origin.bridge.is_some()
                    || graph[current].items() != origin.cleanup.as_slice()
                    || !graph[next].items().starts_with(&[
                        Item::Label(origin.entry),
                        Item::Instruction(origin.end_send),
                    ])
                {
                    continue;
                }
                let mut incoming = graph.iter().flat_map(|(_, block)| block.items()).filter_map(|item| match item {
                    Item::Instruction(instruction)
                        if matches!(instruction.operand(), Operand::Forward(entry) | Operand::Backward(entry)
                            if graph.label_block(entry) == Some(next)) => Some(instruction),
                    _ => None,
                });
                if incoming.next() == Some(&origin.send) && incoming.next().is_none() {
                    origin.bridge = Some((label, bridge));
                }
            }
            let block = graph.insert_block(
                position + 1,
                vec![Item::Label(label), Item::Instruction(bridge)],
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
                    || (!expected_cold
                        && (temperatures[block.index()] == WARM_BLOCK
                            || (temperatures[block.index()] == 0
                                && omitted_backedges.contains(&block))))
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
        // A reversed pair identifies its second exit as the outer cleanup. It does not
        // authorize moving the inner comprehension's continuation before its handler.
        let nested_async_cleanup_exits = async_iteration_exits
            .windows(2)
            .filter_map(|blocks| {
                (cleanup_block_creation_order(graph[*blocks[0]].items())
                    > cleanup_block_creation_order(graph[*blocks[1]].items()))
                .then_some(*blocks[1])
            })
            .collect::<FxHashSet<_>>();
        // Inlined comprehensions emit their successful restore after the exceptional restore.
        // CPython puts a terminal restore before the handlers. Move a direct local restore or
        // the outer restore of nested async loops back to END_ASYNC_FOR's fallthrough.
        let terminal_restore = (0..order.len().saturating_sub(1)).find_map(|position| {
            // A conditional fallthrough starts ordinary code after the loop. Moving its
            // target here would change that fallthrough and can reverse another conditional.
            if !graph[order[position + 1]]
                .last_instruction()
                .is_some_and(|instruction| is_unconditional_jump(instruction.opcode))
            {
                return None;
            }
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
            // Cold-block ordering preserves the relative order of warm blocks. A
            // backward edge to a warm loop header is not a deferred terminal restore,
            // even when the header's conditional fallthrough ends with a return.
            if start <= position && temperatures[target.index()] == WARM_BLOCK {
                return None;
            }
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
                // The outer exit must itself restore an inlined comprehension. An
                // ordinary async-for's terminal finally suite stays after its handlers.
                nested_async_cleanup_exits.contains(&order[position])
                    && self
                        .exception_owner_provenance
                        .original_comprehension_normal_exits
                        .iter()
                        .any(|(entry, jump)| {
                            graph.label_block(*entry) == Some(target)
                                && graph[order[position + 1]].last_instruction() == Some(jump)
                        })
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
                let rebuilt = ExceptionRegion {
                    start,
                    end,
                    target: region.target,
                    depth: region.depth,
                    preserve_lasti: region.preserve_lasti,
                    is_scope_fallback: region.is_scope_fallback,
                    priority: region.priority,
                };
                if let Some(retargets) = owner_retargets.as_mut() {
                    retargets.push((region, rebuilt));
                }
                rebuilt_regions.push(rebuilt);
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
        omitted_markers
    }

    /// Keeps a normal cleanup head unlocated when folding removed an original predecessor.
    ///
    /// CPython resolves lines before folding constant condition edges. Walk the original
    /// folded paths from a surviving, reachable source marker while their instructions
    /// still exist, crossing only original unlocated jump bridges at live joins. A folded
    /// target alone does not prove reachability: lowering can emit loops after terminal arms.
    /// Only a located target that cannot be copied before line resolution certifies
    /// that the converted `POP_BLOCK` had no sole predecessor from which to inherit a line.
    pub(in crate::assembler) fn preserve_folded_pop_block_locations(&mut self) {
        if self.folded_predecessor_sources.is_empty() {
            return;
        }
        let reachable = self.graph.reachable_blocks(&self.exception_regions);
        let order = self.graph.order();
        let positions = order
            .iter()
            .enumerate()
            .map(|(index, block)| (*block, index))
            .collect::<FxHashMap<_, _>>();
        let mut pending = self
            .folded_predecessor_sources
            .iter()
            .filter_map(|&(source, target, location)| {
                let source = self.graph.label_block(source)?;
                if !reachable[source.index()] {
                    return None;
                }
                let marker = self.graph[source]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })?;
                // A removed source marker can leave its label aliased to later live code.
                // Such an alias no longer certifies the original condition's reachability.
                if marker.opcode != NOP || marker.location != location {
                    return None;
                }
                self.graph.label_block(target)
            })
            .collect::<Vec<_>>();
        let mut visited = FxHashSet::default();
        let mut joins = Vec::new();
        while let Some(block) = pending.pop() {
            if !visited.insert(block) {
                continue;
            }
            if reachable[block.index()] {
                // CPython copies an unlocated jump-only join onto each incoming
                // path before resolving lines. Carry the folded predecessor through
                // that original bridge so the cleanup head remains shared.
                let items = self.graph[block].items();
                let mut instructions = items.iter().filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                });
                if let Some(jump) = instructions.next()
                    && instructions.next().is_none()
                    && jump.opcode == JUMP_FORWARD
                    && jump.location == SourceLocation::NONE
                    && jump.pre_fold_extra_instructions() == 0
                    && jump.local_policy.hidden_exception_owner_slots() == 0
                    && jump.jump_policy.allows_threading_target()
                    && !jump.jump_policy.retains_nop_for(jump.operand())
                    && !jump.jump_policy.preserves_nop_after_threading()
                    && !jump.jump_policy.is_interruptible()
                    && !jump.no_location_policy.preserves_location()
                    && items.iter().any(|item| {
                        matches!(item, Item::Label(label)
                            if self.exception_owner_provenance.block_boundaries.contains(label))
                    })
                    && items.iter().all(|item| match item {
                        Item::Label(label) => {
                            !self.original_nop_targets.contains(label)
                                && !self
                                    .exception_owner_provenance
                                    .omitted_nop_counts
                                    .contains_key(label)
                                && !self
                                    .exception_owner_provenance
                                    .omitted_nop_locations
                                    .contains_key(label)
                        }
                        Item::Instruction(_) => true,
                    })
                    && let Operand::Forward(target) = jump.operand()
                    && let Some(target) = self.graph.label_block(target)
                    && positions[&block] < positions[&target]
                {
                    pending.push(target);
                } else {
                    joins.push(block);
                }
                continue;
            }
            if block_has_fallthrough(self.graph[block].items())
                && let Some(next) = order.get(positions[&block] + 1)
            {
                pending.push(*next);
            }
            if let Some(target) = block_jump_target(self.graph[block].items())
                && let Some(target) = self.graph.label_block(target)
            {
                pending.push(target);
            }
        }
        for block in joins {
            let instructions = self.graph[block]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let Some(first) = instructions.first() else {
                continue;
            };
            if first.opcode != NOP
                || !first.fusion_policy.is_converted_pop_block()
                || !matches!(
                    first.exception_policy(),
                    ExceptionPolicy::AlwaysUnchanged | ExceptionPolicy::AlwaysAbsent
                )
                || first.location != SourceLocation::NONE
                || first.no_location_policy.preserves_location()
                || first.pre_fold_extra_instructions() != 0
                || first.local_policy.hidden_exception_owner_slots() != 0
                || !instructions
                    .iter()
                    .any(|instruction| instruction.location.line >= 0)
                || (instructions.len() <= 4
                    && instructions.last().is_some_and(|instruction| {
                        ends_scope(instruction.opcode) || is_unconditional_jump(instruction.opcode)
                    }))
            {
                continue;
            }
            self.graph
                .instructions_mut(block)
                .next()
                .expect("certified converted POP_BLOCK head")
                .no_location_policy
                .preserve_location();
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

    /// Captures the original donor before static swaps reverse a three-store tail.
    pub(in crate::assembler) fn capture_triple_store_handler_exits(
        &self,
    ) -> TripleStoreHandlerExits {
        let mut entries = FxHashMap::default();
        for pair in self.graph.order().windows(2) {
            let source = self.graph[pair[0]].items();
            let target = self.graph[pair[1]].items();
            let Some(label) = self.graph[pair[1]].first_label() else {
                continue;
            };
            if !self
                .exception_owner_provenance
                .source_boundaries
                .contains(&label)
            {
                continue;
            }
            let entry_len = target
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
            let [Item::Instruction(pop), Item::Instruction(jump)] = &target[entry_len..] else {
                continue;
            };
            if pop.opcode != POP_EXCEPT
                || pop.location != SourceLocation::NONE
                || jump.opcode != JUMP_FORWARD
                || jump.location != SourceLocation::NONE
                || !matches!(jump.operand(), Operand::Forward(_))
            {
                continue;
            }
            let [
                ..,
                Item::Instruction(swap),
                Item::Instruction(first),
                Item::Instruction(middle),
                Item::Instruction(last),
            ] = source
            else {
                continue;
            };
            if swap.opcode != SWAP
                || swap.operand() != Operand::Value(3)
                || [first, middle, last].iter().any(|store| {
                    store.opcode != STORE_FAST
                        || store.location.line < 0
                        || store.location.line != swap.location.line
                })
            {
                continue;
            }
            let mut swapped_source = source.to_vec();
            let start = swapped_source.len() - 4;
            let mut removed_swap = *swap;
            removed_swap.opcode = NOP;
            removed_swap.set_operand(Operand::Value(0));
            swapped_source[start..].copy_from_slice(&[
                Item::Instruction(removed_swap),
                Item::Instruction(*last),
                Item::Instruction(*middle),
                Item::Instruction(*first),
            ]);
            entries.insert(
                label,
                TripleStoreHandlerExit {
                    swapped_source,
                    cleanup: target.to_vec(),
                    donor: last.location,
                },
            );
        }
        let original_labels = if entries.is_empty() {
            FxHashSet::default()
        } else {
            self.graph
                .iter_items()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect()
        };
        TripleStoreHandlerExits {
            original_labels,
            entries,
        }
    }

    #[cfg(test)]
    pub(in crate::assembler) fn duplicate_exit_blocks(
        &mut self,
        original_location_absence: &FxHashSet<Label>,
        owner_retargets: &mut Vec<(Instruction, Instruction)>,
    ) -> Vec<Label> {
        let origins = self.capture_triple_store_handler_exits();
        self.duplicate_exit_blocks_with_store_origins(
            original_location_absence,
            owner_retargets,
            &origins,
        )
    }

    pub(in crate::assembler) fn duplicate_exit_blocks_with_store_origins(
        &mut self,
        original_location_absence: &FxHashSet<Label>,
        owner_retargets: &mut Vec<(Instruction, Instruction)>,
        triple_store_handler_exits: &TripleStoreHandlerExits,
    ) -> Vec<Label> {
        let graph = &mut self.graph;
        if graph.is_normalized_jump_free_single_block(&self.preserved_block_boundaries) {
            return Vec::new();
        }

        if !graph.is_normalized(&self.preserved_block_boundaries) {
            graph.normalize_blocks(&self.preserved_block_boundaries);
        }
        let active_internal_labels = self
            .exception_owner_provenance
            .normal_try_copy_internal_labels
            .iter()
            .copied()
            .filter(|label| {
                !self
                    .exception_owner_provenance
                    .retired_normal_try_copy_labels
                    .contains(label)
            })
            .collect::<Vec<_>>();
        if !active_internal_labels.is_empty() {
            let mut forbidden_entries = self.preserved_block_boundaries.clone();
            forbidden_entries.extend(self.exception_regions.iter().map(|region| region.target));
            forbidden_entries.extend(
                self.exception_owner_provenance
                    .setups
                    .iter()
                    .map(|(label, _)| *label),
            );
            forbidden_entries.extend(active_internal_labels.iter().copied().filter(|label| {
                self.exception_owner_provenance
                    .synchronous_with_setups
                    .contains(label)
            }));
            graph
                .join_continuation_blocks(&active_internal_labels, &forbidden_entries)
                .expect("certified original cleanup array continuations");
        }
        if graph.len() < 2 {
            return Vec::new();
        }

        let order = graph.order().to_vec();
        let analysis = exit_block_analysis(
            graph,
            &order,
            &self.exception_regions,
            &self.retired_exception_handler_targets,
            &self.original_nop_targets,
            &self.exception_owner_provenance,
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
            self.borrow_authored_finally_break_entries =
                Self::take_pre_copy_authored_finally_break_entries(
                    graph,
                    &self.exception_owner_provenance.block_boundaries,
                    &mut self.borrow_authored_finally_break_origins,
                );
            if !self.borrow_jump_entry_labels.is_empty() {
                self.borrow_pre_copy_jump_entries =
                    Self::take_pre_copy_borrow_entries_with_threaded(
                        graph,
                        &self.exception_owner_provenance.block_boundaries,
                        &mut self.borrow_threaded_jump_entries,
                    );
            }
            propagate_exit_block_locations(
                graph,
                &order,
                &analysis,
                &self.exception_regions,
                &self.exception_owner_provenance.block_boundaries,
                original_location_absence,
                &self.comprehension_assignment_exits,
                &self.preserved_block_boundaries,
                &self.exception_owner_provenance.source_boundaries,
                &self.exception_owner_provenance.setups,
            );
            let heads = shared_pop_block_heads
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>();
            restore_shared_pop_block_locations(graph, &self.exception_regions, &heads);
            restore_retained_pop_block_locations(
                graph,
                &self.exception_regions,
                &self.preserved_block_boundaries,
                &self.retained_pop_block_origins,
            );
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
        self.borrow_authored_finally_break_entries =
            Self::take_pre_copy_authored_finally_break_entries(
                graph,
                &self.exception_owner_provenance.block_boundaries,
                &mut self.borrow_authored_finally_break_origins,
            );
        if !self.borrow_jump_entry_labels.is_empty() {
            // These are still the pre-copy blocks, including labels just assigned to
            // implicit entries. Every subsequently copied block receives fresh labels.
            self.borrow_pre_copy_jump_entries = Self::take_pre_copy_borrow_entries_with_threaded(
                graph,
                &self.exception_owner_provenance.block_boundaries,
                &mut self.borrow_threaded_jump_entries,
            );
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

        // Snapshot the original keyword-return seam before any exit is copied. A cleanup
        // elsewhere in the unit must not split two owned loads in this unprotected array.
        // Keep the complete item lists so a rewritten source, target, or alias cannot reuse it.
        let original_keyword_return_seams = order
            .iter()
            .filter_map(|&source| {
                let source_items = graph[source].items();
                let [.., Item::Instruction(first), Item::Instruction(jump)] = source_items else {
                    return None;
                };
                let Operand::Forward(label) = jump.operand() else {
                    return None;
                };
                let target = graph.label_block(label)?;
                let target_items = graph[target].items();
                let first_instruction = target_items
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))?;
                let [
                    Item::Instruction(second),
                    Item::Instruction(names),
                    Item::Instruction(call),
                    Item::Instruction(ret),
                ] = &target_items[first_instruction..]
                else {
                    return None;
                };
                (jump.opcode == JUMP_FORWARD
                    && first.opcode == LOAD_FAST
                    && second.opcode == LOAD_FAST
                    && names.opcode == LOAD_CONST
                    && call.opcode == CALL_KW
                    && ret.opcode == RETURN_VALUE
                    && first.pre_fold_extra_instructions() == 0
                    && jump.pre_fold_extra_instructions() == 0
                    && first.exception_policy() == ExceptionPolicy::default()
                    && [second, names, call, ret].iter().all(|instruction| {
                        instruction.pre_fold_extra_instructions() == 0
                            && instruction.exception_policy() == ExceptionPolicy::default()
                            && matches!(instruction.operand(), Operand::Value(_))
                    })
                    && super::locals::fuse_pair(first, second).is_some()
                    && self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(&label)
                    && !exception_boundary_blocks[source.index()]
                    && !exception_boundary_blocks[target.index()]
                    && region_memberships
                        .iter()
                        .enumerate()
                        .all(|(index, membership)| {
                            let source_owned = membership.contains(&positions[source.index()]);
                            let target_owned = membership.contains(&positions[target.index()]);
                            if self.exception_regions[index].is_scope_fallback {
                                source_owned == target_owned
                            } else {
                                !source_owned && !target_owned
                            }
                        }))
                .then(|| {
                    (
                        source,
                        (target, source_items.to_vec(), target_items.to_vec()),
                    )
                })
            })
            .collect::<FxHashMap<_, _>>();

        let exception_split_labels = self
            .exception_regions
            .iter()
            .flat_map(|region| [region.start, region.end])
            .collect::<FxHashSet<_>>();
        // A protected expression exit can be one original array split at POP_BLOCK.
        // Snapshot its complete source, expression tail and return before any copy.
        let original_protected_return_seams = order
            .iter()
            .filter_map(|&source| {
                let source_items = graph[source].items();
                let [
                    Item::Label(_),
                    Item::Instruction(first),
                    Item::Instruction(jump),
                ] = source_items
                else {
                    return None;
                };
                let Operand::Forward(entry) = jump.operand() else {
                    return None;
                };
                let target = graph.label_block(entry)?;
                let target_items = graph[target].items();
                let [
                    Item::Label(target_entry),
                    Item::Instruction(second),
                    Item::Instruction(consume),
                ] = target_items
                else {
                    return None;
                };
                let return_block = *order.get(positions[target.index()] + 1)?;
                let return_items = graph[return_block].items();
                let [Item::Label(end), Item::Instruction(ret)] = return_items else {
                    return None;
                };
                let mut owners = region_memberships
                    .iter()
                    .enumerate()
                    .filter(|(_, membership)| {
                        membership.contains(&positions[source.index()])
                            || membership.contains(&positions[target.index()])
                    });
                let (region_index, membership) = owners.next()?;
                let region = self.exception_regions[region_index];
                (owners.next().is_none()
                    && !region.is_scope_fallback
                    && region.depth == 0
                    && !region.preserve_lasti
                    && membership.contains(&positions[source.index()])
                    && membership.contains(&positions[target.index()])
                    && region.end == *end
                    && !membership.contains(&positions[return_block.index()])
                    && !exception_boundary_blocks[source.index()]
                    && !exception_boundary_blocks[target.index()]
                    && predecessors[target.index()] == 2
                    && predecessors[return_block.index()] == 1
                    && positions[source.index()] < positions[target.index()]
                    && *target_entry == entry
                    && exception_split_labels.contains(end)
                    && jump.opcode == JUMP_FORWARD
                    && jump.location.line < 0
                    && first.opcode == LOAD_FAST
                    && second.opcode == LOAD_FAST
                    && matches!(consume.opcode, BUILD_TUPLE | CALL)
                    && ret.opcode == RETURN_VALUE
                    && first.location.line >= 0
                    && super::locals::fuse_pair(first, second).is_some()
                    && [first, jump, second, consume, ret]
                        .iter()
                        .all(|instruction| {
                            instruction.pre_fold_extra_instructions() == 0
                                && instruction.local_policy.hidden_exception_owner_slots() == 0
                                && instruction.exception_policy() == ExceptionPolicy::default()
                        })
                    && [second, consume, ret]
                        .iter()
                        .all(|instruction| matches!(instruction.operand(), Operand::Value(_))))
                .then(|| {
                    (
                        source,
                        (
                            target,
                            return_block,
                            region_index,
                            source_items.to_vec(),
                            target_items.to_vec(),
                            return_items.to_vec(),
                        ),
                    )
                })
            })
            .collect::<FxHashMap<_, _>>();

        // Keep locations already present on original return records distinct from
        // locations inherited by synthetic cleanup copies during this pass.
        let mut original_return_locations = FxHashMap::default();
        let mut return_labels = Vec::new();
        for item in graph.iter_items() {
            match item {
                Item::Label(label) => return_labels.push(*label),
                Item::Instruction(instruction) => {
                    if instruction.opcode == RETURN_VALUE
                        && instruction.location.line >= 0
                        && !instruction
                            .no_location_policy
                            .copy_inherits_shared_predecessor()
                    {
                        for label in &return_labels {
                            original_return_locations.insert(*label, instruction.location);
                        }
                    }
                    return_labels.clear();
                }
            }
        }
        let mut exit_without_unique_predecessor = vec![false; positions.len()];
        for block in order.iter().copied() {
            let unlocated_scope_exit = graph[block].items().iter().all(|item| {
                        !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                    })
                    && graph[block]
                        .last_instruction()
                        .is_some_and(|instruction| {
                            ends_scope(instruction.opcode) || has_jump_eval_check(instruction)
                        });
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
        let mut planned_inline_labels = active_internal_labels
            .iter()
            .copied()
            .collect::<FxHashSet<_>>();
        let mut copied_entry_labels = Vec::new();
        let mut copied_exit_or_eval_entry_labels = FxHashSet::default();
        let mut retained_unlocated_join_blocks = FxHashSet::default();
        let mut target_copies = FxHashMap::<_, Vec<Vec<Item>>>::default();
        let mut copied_split_eval_starts = FxHashMap::default();
        let mut copied_region_exclusions =
            vec![Vec::<(Label, Label)>::new(); self.exception_regions.len()];
        let mut copied_exception_regions = Vec::new();
        let mut original_pop_ranges = self
            .exception_owner_provenance
            .normal_try_copy_pop_ranges
            .clone();
        // The closed return-copy schedule owns its original slots and location repairs.
        // These ordinary cleanup arrays were restored only when that schedule was absent.
        let generic_original_arrays = self
            .exception_owner_provenance
            .normal_try_copy_steps
            .is_empty()
            && !original_pop_ranges.is_empty();
        // A nested condition can share its original case-success array with the
        // body (or another condition). Copying the certified normal-try cleanup
        // replaces that array's jump before conditional predecessors are visited.
        // Retain the original shared-array classification across that exact copy.
        let shared_match_cleanup_heads = order
            .iter()
            .copied()
            .filter_map(|source| {
                if !generic_original_arrays || predecessors[source.index()] <= 1 {
                    return None;
                }
                let source_items = graph[source].items();
                let (Item::Instruction(jump), head) = source_items.split_last()? else {
                    return None;
                };
                if head.is_empty()
                    || !head.iter().all(|item| matches!(item, Item::Label(_)))
                    || jump.opcode != JUMP_FORWARD
                    || jump.pre_fold_extra_instructions() != 0
                    || jump.local_policy.hidden_exception_owner_slots() != 0
                    || jump.no_location_policy.preserves_location()
                    || jump.exception_policy() != ExceptionPolicy::default()
                {
                    return None;
                }
                let Operand::Forward(label) = jump.operand() else {
                    return None;
                };
                if !self
                    .exception_owner_provenance
                    .original_match_join_boundaries
                    .contains(&label)
                {
                    return None;
                }
                let target = graph.label_block(label)?;
                let target_items = graph[target].items();
                let [
                    labels @ ..,
                    Item::Label(start),
                    Item::Instruction(pop),
                    Item::Label(end),
                    Item::Instruction(exit),
                ] = target_items
                else {
                    return None;
                };
                if !labels.iter().all(|item| matches!(item, Item::Label(_)))
                    || !labels.contains(&Item::Label(label))
                    || !original_pop_ranges.contains(&(*start, *end))
                    || [start, end].into_iter().any(|label| {
                        self.exception_owner_provenance
                            .retired_normal_try_copy_labels
                            .contains(label)
                            || self
                                .exception_owner_provenance
                                .normal_try_copy_inline_labels
                                .contains(label)
                    })
                    || pop.opcode != NOP
                    || pop.operand() != Operand::Value(0)
                    || !pop.fusion_policy.is_converted_pop_block()
                    || exit.opcode != JUMP_FORWARD
                    || !matches!(exit.operand(), Operand::Forward(_))
                    || !exit.only_allows_unconditional_inlining()
                    || !exit.no_location_policy.allows_block_inlining()
                    || exit.jump_policy.allows_threading_target()
                    || jump.depth_after() != pop.depth_after()
                    || pop.depth_after() != exit.depth_after()
                    || [pop, exit].iter().any(|instruction| {
                        instruction.location != SourceLocation::NONE
                            || instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                    })
                {
                    return None;
                }
                Some((
                    source,
                    (target, source_items.to_vec(), target_items.to_vec()),
                ))
            })
            .collect::<FxHashMap<_, _>>();
        let mut shared_match_cleanup_copies = FxHashMap::<BasicBlockId, Vec<Item>>::default();
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
        let retired_labels = &self
            .exception_owner_provenance
            .retired_normal_try_copy_labels;
        let planned_blocks = self
            .exception_owner_provenance
            .normal_try_copy_blocks
            .iter()
            .filter(|label| !retired_labels.contains(*label))
            .filter_map(|label| graph.label_block(*label))
            .collect::<FxHashSet<_>>();
        let planned_steps = self
            .exception_owner_provenance
            .normal_try_copy_steps
            .iter()
            .filter(|(source, _, _)| !retired_labels.contains(source))
            .map(|(source, target, width)| {
                (
                    graph.label_block(*source).expect("certified copy source"),
                    graph.label_block(*target).expect("certified copy target"),
                    *width,
                )
            })
            .collect::<Vec<_>>();
        let mut original_join_marker_donors = Vec::new();
        let mut planned_cursor = 0;
        let mut source_cursor = 0;
        loop {
            let planned_target = planned_steps
                .get(planned_cursor)
                .map(|(_, target, _)| *target);
            let planned_width = planned_steps
                .get(planned_cursor)
                .map(|(_, _, width)| *width);
            let selected_source = planned_steps
                .get(planned_cursor)
                .map(|(source, _, _)| source)
                .or_else(|| sources.get(source_cursor));
            let Some(&source) = selected_source else {
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
                        .is_some_and(|instruction| {
                            ends_scope(instruction.opcode) || has_jump_eval_check(instruction)
                        });
                    exit_without_unique_predecessor[block.index()] |=
                        remaining_predecessors[block.index()] > 1 && unlocated_scope_exit;
                }
                propagate_planned_array_locations(
                    graph,
                    &order,
                    &planned_blocks,
                    &planned_inline_labels,
                );
                sources.append(&mut conditional_sources);
                continue;
            };
            if planned_target.is_some() {
                planned_cursor += 1;
            } else {
                source_cursor += 1;
                if planned_blocks.contains(&source) {
                    continue;
                }
            }
            let planned_copy = planned_target.is_some();
            let Some(target_label) = block_jump_target(graph[source].items()) else {
                continue;
            };
            let Some(target) = graph.label_block(target_label) else {
                continue;
            };
            if let Some(expected) = planned_target {
                assert_eq!(target, expected, "original cleanup copy target changed");
            }
            let source_position = positions[source.index()];
            let target_position = positions[target.index()];
            let source_has_fallthrough = block_has_fallthrough(graph[source].items());
            let preserves_shared_match_exit = shared_match_cleanup_heads.get(&source).is_some_and(
                |(original_target, source_items, target_items)| {
                    *original_target == target
                        && graph[source].items() == source_items
                        && graph[target].items() == target_items
                },
            );
            let copy_shared_match_exit = source_has_fallthrough
                && remaining_predecessors[target.index()] > 1
                && shared_match_cleanup_copies
                    .get(&target)
                    .is_some_and(|items| graph[target].items() == items);
            let mut target_chain_end = target_position;
            while target_chain_end + 1 < order.len()
                && block_has_fallthrough(graph[order[target_chain_end]].items())
                // Exception-table splits can divide one original block, but a
                // conditional jump always ends it and must not be copied with its fallthrough.
                && block_jump_target(graph[order[target_chain_end]].items()).is_none()
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
            let represented_setups =
                represented_setup_nops(graph, target_blocks, &self.exception_owner_provenance);
            let mut at_target_entry = true;
            let target_pre_fusion_size = target_blocks
                .iter()
                .flat_map(|block| graph[*block].items())
                .map(|item| match item {
                    Item::Instruction(instruction) => {
                        at_target_entry = false;
                        let fused_push_null = instruction.opcode == LOAD_GLOBAL
                            && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0);
                        1 + usize::from(fused_push_null)
                            + usize::from(instruction.pre_fold_extra_instructions())
                    }
                    Item::Label(label) => usize::from(
                        target_chain_end > target_position
                            && exception_split_labels.contains(label)
                            && !self
                                .exception_owner_provenance
                                .return_region_reentries
                                .contains(label)
                            && !represented_setups.contains(label)
                            && !(at_target_entry
                                && is_metadata_range_entry(
                                    *label,
                                    &self.exception_regions,
                                    &self.exception_owner_provenance.setups,
                                )),
                    ),
                })
                .sum::<usize>();
            if let Some(width) = planned_width {
                assert_eq!(
                    target_chain_end, target_position,
                    "original cleanup target split"
                );
                assert_eq!(
                    target_pre_fusion_size, width,
                    "original cleanup array changed before copy"
                );
            }
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
            let source_unlocated_inlining = graph[source]
                .last_instruction()
                .is_some_and(|instruction| instruction.no_location_policy.allows_block_inlining());
            let target_is_small_exit = target_pre_fusion_size <= 4
                && target_terminal_opcode.is_some_and(|opcode| {
                    ends_scope(opcode)
                        // SETUP_FINALLY and POP_BLOCK do not split the original CPython block.
                        && (target_chain_end == target_position
                            || matches!(opcode, RAISE_VARARGS | RETURN_VALUE))
                });
            let source_allows_small_exit_inlining = (target_chain_end == target_position
                && (block_is_bare_raise(graph[target].items(), &self.original_nop_targets)
                    || block_is_local_raise(graph[target].items(), &self.original_nop_targets)))
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
            let target_has_unlocated_jump_eval_check = target_has_no_location
                && graph[target_exit]
                    .last_instruction()
                    .is_some_and(has_jump_eval_check);
            let target_has_copied_jump_eval_check = target_has_unlocated_jump_eval_check
                && target_blocks.iter().any(|block| {
                    graph[*block].items().iter().any(|item| {
                        matches!(item, Item::Label(label) if copied_exit_or_eval_entry_labels.contains(label))
                    })
                });
            let shared_original_loop_discard = remaining_predecessors[target.index()] > 1
                && target_chain_end == target_position
                && target_has_no_location
                && !target_allows_no_location_block_inlining
                && !target_has_artificial_eval_break
                && !target_has_copied_jump_eval_check
                && graph[target]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.opcode),
                        Item::Label(_) => None,
                    })
                    .eq([POP_TOP, JUMP_BACKWARD]);
            if shared_original_loop_discard {
                // Loop-tail lowering appends the enclosing if's backedge to this discard.
                // The original discard falls through to that separate join, so it is not
                // an exit block eligible for copying or predecessor-location propagation.
                for instruction in graph.instructions_mut(target) {
                    instruction.no_location_policy.preserve_location();
                }
                continue;
            }
            if source_has_fallthrough
                && !copy_shared_match_exit
                && !target_has_copied_jump_eval_check
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
            if !planned_copy
                && inline_no_location_block
                && target_chain_end == target_position
                && graph[source].last_instruction().is_some_and(|jump| {
                    jump.opcode == JUMP_FORWARD
                        // Unlocated cleanup continuations retain their original copy order
                        // so removing one predecessor can still locate the shared cleanup.
                        && jump.location.line >= 0
                        // Retained authored line slots keep their original copy order.
                        && !jump.jump_policy.retains_nop_for(jump.operand())
                        && jump.pre_fold_extra_instructions() > 0
                        && jump.local_policy.hidden_exception_owner_slots() == 0
                })
                && named_cleanup_has_short_return(graph, target)
                // Ordinary try exits have a separately recorded POP_BLOCK/copy order.
                // Their retained entry labels must not use the branch-join rule.
                && !self
                    .exception_owner_provenance
                    .normal_try_exits
                    .iter()
                    .any(|(entry, _)| graph.label_block(*entry) == Some(source))
            {
                // Threading bypassed an original join before this cleanup. Its return is
                // appended before that earlier edge can copy the expanded named cleanup.
                // Direct predecessors still copy here; the existing queue revisits this
                // edge after the cleanup's own return copy and checks its new size/location.
                continue;
            }
            if planned_copy {
                assert!(
                    inline_small_exit || inline_no_location_block,
                    "certified cleanup copy lost eligibility"
                );
            }
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
                    || target_has_unlocated_jump_eval_check
                    || target_is_no_location_no_fallthrough)
                    || remaining_predecessors[target.index()] <= 1)
            {
                continue;
            }

            // Replacing this source jump ends its original join provenance. Copied target
            // labels are renamed below and therefore cannot inherit that provenance either.
            if !self.deferred_shared_jump_blocks.is_empty()
                || !self.shared_jump_copy_location_blocks.is_empty()
                || !self.retained_pop_block_origins.is_empty()
                || !self
                    .exception_owner_provenance
                    .retained_normal_pop_blocks
                    .is_empty()
            {
                for item in graph[source].items() {
                    if let Item::Label(label) = item {
                        self.deferred_shared_jump_blocks.remove(label);
                        self.shared_jump_copy_location_blocks.remove(label);
                        self.retained_pop_block_origins.remove(label);
                        self.exception_owner_provenance
                            .retained_normal_pop_blocks
                            .retain(|_, origin| {
                                !origin.before.contains(label) && !origin.after.contains(label)
                            });
                    }
                }
            }

            let source_location = if planned_copy {
                graph[source]
                    .last_instruction()
                    .expect("planned terminal jump")
                    .location
            } else {
                graph[source]
                    .items()
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) if instruction.location.line >= 0 => {
                            Some(instruction.location)
                        }
                        Item::Instruction(_) | Item::Label(_) => None,
                    })
                    .unwrap_or(SourceLocation::NONE)
            };
            let keyword_return_seam = original_keyword_return_seams.get(&source).filter(
                |(original_target, source_items, target_items)| {
                    inline_small_exit
                        && target_chain_end == target_position
                        && target_pre_fusion_size == 4
                        && *original_target == target
                        && graph[source].items() == source_items
                        && graph[target].items() == target_items
                },
            );
            let protected_return_seam = original_protected_return_seams.get(&source).filter(
                |(
                    original_target,
                    original_return,
                    _,
                    source_items,
                    target_items,
                    return_items,
                )| {
                    inline_small_exit
                        && !planned_copy
                        && target_pre_fusion_size == 4
                        && target_blocks == [*original_target, *original_return]
                        && *original_target == target
                        && graph[source].items() == source_items
                        && graph[target].items() == target_items
                        && graph[*original_return].items() == return_items
                },
            );
            let mut copied = target_blocks
                .iter()
                .flat_map(|block| graph[*block].items().iter().copied())
                .map(|item| match item {
                    Item::Instruction(instruction) => {
                        Item::Instruction(instruction.for_exit_copy())
                    }
                    Item::Label(_) => item,
                })
                .collect::<Vec<_>>();
            // A shared jump keeps its original no-location policy through NOP removal.
            // Its copy has a new predecessor and can inherit that predecessor's location.
            // Read the original labels before they are renamed below.
            if !self.shared_jump_copy_location_blocks.is_empty() {
                let mut shared_jump = false;
                let mut original_pop = false;
                for item in &mut copied {
                    match item {
                        Item::Label(label) => {
                            shared_jump |= self.shared_jump_copy_location_blocks.contains(label);
                            original_pop |= original_pop_ranges
                                .iter()
                                .any(|(start, _)| *start == *label);
                        }
                        Item::Instruction(instruction) => {
                            if shared_jump && is_unconditional_jump(instruction.opcode) {
                                instruction.no_location_policy.clear_preserved_location();
                            }
                            // The planned original array restores its POP_BLOCK before the
                            // shared JUMP. Carry this head only across that exact restored slot.
                            shared_jump &= planned_copy
                                && original_pop
                                && instruction.opcode == NOP
                                && instruction.fusion_policy.is_converted_pop_block();
                            original_pop = false;
                        }
                    }
                }
            }
            if inline_no_location_block
                && graph[source].last_instruction().is_some_and(|instruction| {
                    instruction.opcode == JUMP_BACKWARD
                        || instruction.jump_policy.is_interruptible()
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
                if matches!(
                    instruction.jump_policy.retained_nop(),
                    super::super::ir::RetainedJumpNop::None
                ) {
                    instruction.jump_policy.mark_interruptible();
                } else {
                    // The copied target still owns its retained jump NOP. Carry the
                    // original JUMP kind in the opcode until cold ordering sets direction.
                    instruction.opcode = JUMP_BACKWARD;
                }
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
            // A short keyword-call raise is also a scope exit in CPython's
            // original array. Keep the same empty-join provenance and location
            // guards as returns; admit only its three unchanged records here.
            let inline_unprotected_keyword_raise = inline_small_exit
                && target_chain_end == target_position
                && target_pre_fusion_size == 3
                && graph[target]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.opcode),
                        Item::Label(_) => None,
                    })
                    .eq([LOAD_CONST, CALL_KW, RAISE_VARARGS])
                && graph[target]
                    .last_instruction()
                    .is_some_and(|instruction| instruction.operand() == Operand::Value(1))
                && region_memberships
                    .iter()
                    .enumerate()
                    .all(|(index, membership)| {
                        self.exception_regions[index].is_scope_fallback
                            || (!membership.contains(&source_position)
                                && !membership.contains(&target_position))
                    });
            // Keep an isolated unlocated join's replaced-jump NOP until its
            // remaining fallthrough predecessor supplies a location. Earlier jump
            // threading may already have removed its other incoming edges.
            // Exception bookkeeping can split a block without creating an original
            // CFG boundary, so only retain markers on recorded original blocks.
            let preserve_unlocated_join_nop = (inline_unprotected_return
                || inline_unprotected_keyword_raise)
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
                && region_memberships
                    .iter()
                    .enumerate()
                    .all(|(index, membership)| {
                        let source_owned = membership.contains(&source_position);
                        let target_owned = membership.contains(&target_position);
                        // The scope fallback does not split this original empty join.
                        // Its replaced jump keeps the same owner as the copied return.
                        if self.exception_regions[index].is_scope_fallback {
                            source_owned == target_owned
                        } else {
                            !source_owned && !target_owned
                        }
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
            // CPython copies small exits before threading jumps. The compiler's
            // original-empty-join certificate survives retargeting even when the
            // direct-edge line policy is cleared. Current block width alone cannot
            // prove this: earlier folding may have removed original instructions.
            let preserve_threaded_join_nop = (inline_unprotected_return
                || inline_unprotected_keyword_raise)
                && target_chain_end == target_position
                && source_location.line >= 0
                && graph[source].items().iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.block_boundaries.contains(label))
                })
                && graph[source]
                    .items()
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count()
                    == 1
                && graph[source].last_instruction().is_some_and(|instruction| {
                    instruction.jump_policy.is_original_empty_join()
                        && !instruction.no_location_policy.preserves_location()
                        && instruction.exception_policy() == ExceptionPolicy::default()
                });
            // Match-success jumps are originally unlocated. A located replacement
            // may be retained only after their sole surviving fallthrough has supplied
            // the donor; copying it into another predecessor would carry the wrong line.
            let preserve_single_fallthrough_match_nop = !planned_copy
                && inline_unprotected_return
                && target_chain_end == target_position
                && (super::control_flow::is_original_constant_return(graph, target_label)
                    || super::normal_try_copies::original_local_return(graph[target].items()))
                && source_location.line >= 0
                && remaining_predecessors[source.index()] == 1
                && graph[source].items().iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.original_match_join_boundaries.contains(label)
                            && !self.borrow_block_aliases.contains(label)
                            && graph.label_block(*label) == Some(source))
                })
                && graph[source]
                    .items()
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count()
                    == 1
                && graph[source].last_instruction().is_some_and(|instruction| {
                    instruction.opcode == JUMP_FORWARD
                        && instruction.jump_policy.is_original_match_success()
                        && instruction.pre_fold_extra_instructions() == 0
                        && !instruction.no_location_policy.preserves_location()
                        && instruction.exception_policy() == ExceptionPolicy::default()
                })
                && jump_sources[source.index()].iter().all(|predecessor| {
                    drop_blocks[predecessor.index()]
                        || block_jump_target(graph[*predecessor].items())
                            .and_then(|label| graph.label_block(label))
                            != Some(source)
                })
                && source_position.checked_sub(1).is_some_and(|position| {
                    let previous = order[position];
                    !drop_blocks[previous.index()]
                        && !inline_copies.contains_key(&previous)
                        && !target_copies.contains_key(&previous)
                        && block_has_fallthrough(graph[previous].items())
                        && graph[previous]
                            .last_instruction()
                            .is_some_and(|instruction| instruction.location == source_location)
                });
            // A shared match end can still be copied into an earlier pass arm.
            // Keep its original jump slot unlocated through every copy round; only
            // its exact original successor seam receives the final call's location.
            let deferred_match_donor = (!planned_copy
                && inline_unprotected_return
                && target_chain_end == target_position
                && super::control_flow::is_original_constant_return(graph, target_label)
                && self.exception_regions.is_empty()
                && graph[source]
                    .items()
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count()
                    == 1)
                .then(|| {
                    graph[source].items().iter().find_map(|item| {
                        let Item::Label(entry) = item else {
                            return None;
                        };
                        let (expected, donor) = self
                            .exception_owner_provenance
                            .shared_match_return_donors
                            .get(entry)?;
                        let previous = *order.get(source_position.checked_sub(1)?)?;
                        (graph.label_block(*entry) == Some(source)
                            && !self.borrow_block_aliases.contains(entry)
                            && !drop_blocks[previous.index()]
                            && !inline_copies.contains_key(&previous)
                            && !target_copies.contains_key(&previous)
                            && block_has_fallthrough(graph[previous].items())
                            && graph[previous].last_instruction() == Some(donor)
                            && graph[source].last_instruction() == Some(expected))
                        .then_some(donor.location)
                    })
                })
                .flatten();
            let preserve_inlined_jump_nop = planned_copy
                || deferred_match_donor.is_some()
                || (inline_small_exit
                    && (preserve_unlocated_join_nop
                        || preserve_threaded_join_nop
                        || preserve_single_fallthrough_match_nop
                        || (source_location.line >= 0
                            && graph[source].last_instruction().is_some_and(|instruction| {
                                instruction
                                    .jump_policy
                                    .retains_nop_for(instruction.operand())
                            }))));
            let copied_label = Label(self.next_label);
            self.next_label += 1;
            if !self.borrow_finally_jump_sources.is_empty() {
                // A copied terminal jump has no independent original source certificate.
                self.borrow_finally_jump_sources.insert(copied_label, false);
            }
            let mut replaced_entry_label = false;
            let mut copied_labels = FxHashMap::default();
            let mut copied_jump_nop_successors = Vec::new();
            let mut copied_previous_is_nop = false;
            for item in &mut copied {
                if let Item::Instruction(instruction) = item {
                    copied_previous_is_nop = instruction.opcode == NOP;
                }
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
                    // A target beginning at this label does not include the source NOP.
                    // Inherit its certificate only when that NOP is physically copied.
                    if copied_previous_is_nop
                        && self
                            .exception_owner_provenance
                            .normal_try_copy_jump_nop_successors
                            .contains(&original)
                    {
                        copied_jump_nop_successors.push(*label);
                    }
                }
            }
            self.exception_owner_provenance
                .normal_try_copy_jump_nop_successors
                .extend(copied_jump_nop_successors);
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
            if planned_copy {
                // These freshly renamed labels are inside the array appended after the
                // source jump's NOP. External region aliases below are not physical copies.
                // Other copy kinds do not inherit this original-array certificate.
                self.exception_owner_provenance
                    .normal_try_copy_inline_labels
                    .extend(copied.iter().filter_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    }));
            }
            // Conditional copies can also enter an expanded original cleanup array.
            // Project its POP_BLOCK holes through physical labels before external exception
            // region ends add synthetic aliases that do not copy those original endpoints.
            let copied_pop_ranges = original_pop_ranges
                .iter()
                .filter_map(|(start, end)| {
                    match (copied_labels.get(start), copied_labels.get(end)) {
                        (Some(&start), Some(&end)) => Some((start, end)),
                        (None, None) => None,
                        _ => panic!("original cleanup copy split a POP_BLOCK range"),
                    }
                })
                .collect::<Vec<_>>();
            let generic_original_source_copy = generic_original_arrays
                && !source_has_fallthrough
                && inline_unprotected_return
                && target_chain_end == target_position
                && super::normal_try_copies::original_local_return(graph[target].items())
                && source_has_original_pop_jump(
                    graph,
                    source,
                    &original_pop_ranges,
                    &self.exception_owner_provenance,
                    &self.preserved_block_boundaries,
                    &self.exception_regions,
                    inline_copies
                        .values()
                        .chain(target_copies.values())
                        .flatten()
                        .flatten(),
                );
            let generic_original_copy = generic_original_arrays
                && !source_has_fallthrough
                && (!copied_pop_ranges.is_empty() || generic_original_source_copy);
            if generic_original_copy {
                // Only labels physically copied from the original array certify adjacency.
                // The exception-end aliases added below have no original instruction slot.
                self.exception_owner_provenance
                    .normal_try_copy_inline_labels
                    .extend(copied.iter().filter_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    }));
            }
            // A return inlined earlier in this pass can already contain holes in its
            // enclosing handlers. A conditional copy preserves those original instruction
            // owners too. Only physically copied endpoints certify a complete hole; the
            // synthetic exception-end aliases below do not.
            let mut inherited_conditional_exclusions = Vec::new();
            if source_has_fallthrough {
                let copied_positions = copied
                    .iter()
                    .enumerate()
                    .filter_map(|(position, item)| match item {
                        Item::Label(label) => Some((*label, position)),
                        Item::Instruction(_) => None,
                    })
                    .collect::<FxHashMap<_, _>>();
                for (index, exclusions) in copied_region_exclusions.iter().enumerate() {
                    let membership = &region_memberships[index];
                    if !membership.contains(&target_position)
                        || !membership.contains(&target_chain_end)
                    {
                        continue;
                    }
                    for (start, end) in exclusions {
                        if let (Some(&start), Some(&end)) =
                            (copied_labels.get(start), copied_labels.get(end))
                            && copied_positions[&start] < copied_positions[&end]
                        {
                            inherited_conditional_exclusions.push((index, start, end));
                        }
                    }
                }
            }
            let mut protected_fragment_origins = FxHashSet::default();
            // The aliases below close copied regions without copying handler code.
            // Only labels copied with instructions can redirect an exception target.
            let copied_handler_labels = copied_labels.clone();
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
            // Renaming a region-only re-entry must not create an original unwind slot.
            let reentries = &mut self.exception_owner_provenance.return_region_reentries;
            let mut ordered_copied_labels = copied_labels.iter().collect::<Vec<_>>();
            ordered_copied_labels.sort_unstable_by_key(|(original, _)| original.0);
            for (original, replacement) in ordered_copied_labels {
                if reentries.contains(original) {
                    reentries.insert(*replacement);
                }
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
                    let end = *copied_labels.get(&region.end)?;
                    protected_fragment_origins.insert(index);
                    Some(ExceptionRegion {
                        start,
                        end,
                        target: copied_handler_labels
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
            // CPython labels the original return before copying it. A previous
            // cleanup copy can leave a hole in the destination's scope fallback;
            // preserve the complete return target's own fallback inside that hole.
            // The intact original return certificate excludes cleanup POP_BLOCKs and
            // internal exception boundaries, whose owners must remain independent.
            if planned_copy
                && target_chain_end == target_position
                && (super::control_flow::is_original_constant_return(graph, target_label)
                    || super::normal_try_copies::original_local_tuple_return(graph[target].items())
                        .is_some())
            {
                let fallbacks = self
                    .exception_regions
                    .iter()
                    .enumerate()
                    .filter(|(index, region)| {
                        region.is_scope_fallback
                            && region_memberships[*index].contains(&target_position)
                            && !protected_fragment_origins.contains(index)
                    })
                    .map(|(_, region)| *region)
                    .collect::<Vec<_>>();
                if !fallbacks.is_empty() {
                    let copied_end = Label(self.next_label);
                    self.next_label += 1;
                    copied.push(Item::Label(copied_end));
                    copied_exception_regions.extend(fallbacks.into_iter().map(|region| {
                        ExceptionRegion {
                            start: copied_label,
                            end: copied_end,
                            ..region
                        }
                    }));
                }
            }
            // A copied fragment only replaces the holes for its own original region.
            // Other enclosing regions still need the copied exclusions even when this
            // cleanup also introduces a separate protected fragment.
            for (index, start, end) in inherited_conditional_exclusions {
                if !protected_fragment_origins.contains(&index) {
                    copied_region_exclusions[index].push((start, end));
                }
            }
            if planned_copy || generic_original_copy {
                // Keep the original jump slot until the ordinary NOP-removal pass.
                // Its existing successor label provides identity without another IR item.
                self.exception_owner_provenance
                    .normal_try_copy_jump_nop_successors
                    .insert(copied_label);
                planned_inline_labels.extend(copied_labels.values().copied());
                let position = graph[source]
                    .items()
                    .iter()
                    .rposition(|item| matches!(item, Item::Instruction(_)))
                    .expect("planned source jump");
                let original = *graph[source]
                    .last_instruction()
                    .expect("planned source jump");
                let original_donor = if planned_copy {
                    graph[source]
                        .first_label()
                        .and_then(|label| {
                            self.exception_owner_provenance
                                .normal_try_copy_join_donors
                                .remove(&label)
                        })
                        .filter(|(record, _)| *record == original)
                        .map(|(_, location)| location)
                } else {
                    None
                };
                let mut marker = Instruction::synthetic(
                    NOP,
                    Operand::Value(0),
                    original.location,
                    original.depth_after(),
                );
                marker.set_exception_policy(original.exception_policy());
                // This marker still occupies the original shared jump slot. Its missing
                // location remains independent of the cleanup copied after it.
                if generic_original_copy && original.no_location_policy.preserves_location() {
                    marker.no_location_policy.preserve_location();
                }
                if !planned_copy {
                    marker.add_pre_fold_extra_instructions(original.pre_fold_extra_instructions());
                }
                if let Some(location) = original_donor {
                    // Keep every copy round unlocated. The unique copied successor
                    // identifies this original marker after the schedule finishes.
                    original_join_marker_donors.push((source, copied_label, marker, location));
                }
                graph.insert_item(
                    ItemPosition::new(source, position),
                    Item::Instruction(marker),
                );
            } else if let Some(location) = deferred_match_donor {
                let position = graph[source]
                    .items()
                    .iter()
                    .rposition(|item| matches!(item, Item::Instruction(_)))
                    .expect("original match jump");
                let original = *graph[source]
                    .last_instruction()
                    .expect("original match jump");
                let mut marker = Instruction::synthetic(
                    NOP,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    original.depth_after(),
                );
                marker.set_exception_policy(original.exception_policy());
                original_join_marker_donors.push((source, copied_label, marker, location));
                graph.insert_item(
                    ItemPosition::new(source, position),
                    Item::Instruction(marker),
                );
            } else if preserve_inlined_jump_nop {
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
                && !planned_copy
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
                    u8::from(!preserve_inlined_jump_nop && !generic_original_source_copy)
                        .saturating_add(jump_extra),
                );
            }
            if let Some(instruction) = copied.iter_mut().find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            }) && instruction
                .no_location_policy
                .copy_inherits_shared_predecessor()
            {
                // The original shared cleanup can stay unlocated while its copies inherit
                // their own predecessor, as CPython resolves each copied exit separately.
                instruction.no_location_policy.clear_preserved_location();
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
                        || copy_shared_match_exit
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
            let copied_return_locations = copied_labels
                .iter()
                .filter_map(|(original, copied)| {
                    original_return_locations
                        .get(original)
                        .map(|location| (*copied, *location))
                })
                .collect::<FxHashMap<_, _>>();
            if inherit_copied_locations && !planned_copy {
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
                let mut original_return_location = None;
                let mut exception_split_entry = false;
                for item in &mut copied {
                    if let Item::Label(label) = item
                        && let Some(location) = copied_return_locations.get(label)
                    {
                        original_return_location = Some(*location);
                    }
                    match item {
                        Item::Label(label) if copied_exception_split_labels.contains(label) => {
                            block_needs_location = true;
                            exception_split_entry = true;
                        }
                        Item::Label(label) if copied_inline_entry_labels.contains(label) => {
                            inline_entry = true;
                        }
                        Item::Instruction(instruction) => {
                            if block_needs_location {
                                // A synthetic exception split does not erase the original
                                // located RETURN that follows the copied expression.
                                if !exception_split_entry
                                    || instruction.opcode != RETURN_VALUE
                                    || original_return_location != Some(instruction.location)
                                    || copy_conditional_split_return
                                    || target_has_artificial_eval_break
                                {
                                    instruction.location = source_location;
                                }
                                block_needs_location = false;
                            } else if inline_entry
                                && instruction.location.line < 0
                                && !instruction.no_location_policy.preserves_location()
                            {
                                instruction.location = inline_location;
                            }
                            inline_entry = false;
                            original_return_location = None;
                            exception_split_entry = false;
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
            if copy_shared_match_exit {
                // This exact array contains only the former case jump's NOP and
                // its certified POP_BLOCK/JUMP copy. Its earlier donor belongs to
                // another predecessor; every copied slot inherits this condition.
                for item in &mut copied {
                    if let Item::Instruction(instruction) = item {
                        instruction.location = source_location;
                    }
                }
            }
            original_return_locations.extend(copied_return_locations);
            if inline_small_exit || inline_no_location_block {
                if planned_copy || (inline_no_location_block && target_chain_end > target_position)
                {
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
                        // Scheduled copies require physically copied endpoints; synthetic
                        // region-end aliases do not identify an original owner hole.
                        let exclusion_labels = if planned_copy {
                            &copied_handler_labels
                        } else {
                            &copied_labels
                        };
                        let inherited = exclusions
                            .iter()
                            .filter_map(|(start, end)| {
                                let start = *exclusion_labels.get(start)?;
                                let end = *exclusion_labels.get(end)?;
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
                    if !planned_copy && let Some(position) = trailing_nop_position {
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
            if inline_no_location_block
                && target_pre_fusion_size == 1
                && target_terminal_opcode == Some(JUMP_BACKWARD)
                && let Some(original) = graph[source].last_instruction().filter(|instruction| {
                    instruction.jump_policy.is_normalized_conditional_backedge()
                })
            {
                // This edge already represents CPython's late backward-conditional expansion.
                // Replacing its bridge with the original backedge must retain the fresh slot
                // owner policy; ordinary predecessors still copy the original owned jump.
                for item in &mut copied {
                    if let Item::Instruction(instruction) = item {
                        instruction.set_exception_policy(original.exception_policy());
                        instruction
                            .jump_policy
                            .mark_normalized_conditional_backedge();
                    }
                }
            }
            // Later rounds can copy arrays containing an earlier cloned POP_BLOCK.
            original_pop_ranges.extend(copied_pop_ranges);
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
                    predecessor_copy.extend(
                        transitive_copy
                            .iter()
                            .copied()
                            .map(Instruction::for_exit_copy)
                            .map(Item::Instruction),
                    );
                    let jump_position = graph[predecessor]
                        .items()
                        .iter()
                        .rposition(|item| matches!(item, Item::Instruction(_)))
                        .expect("a transitive exit-copy predecessor must end in a jump");
                    // A copied exit cannot inherit its predecessor's original POP prefix.
                    self.exception_owner_provenance
                        .copied_return_pop_prefixes
                        .retain(|origin| graph.label_block(origin.entry) != Some(predecessor));
                    self.exception_owner_provenance
                        .copied_return_call_prefixes
                        .retain(|origin| graph.label_block(origin.entry) != Some(predecessor));
                    self.exception_owner_provenance
                        .copied_return_attribute_truth_prefixes
                        .retain(|origin| graph.label_block(origin.entry) != Some(predecessor));
                    graph.remove_item_at(predecessor, jump_position);
                    inline_copies
                        .entry(predecessor)
                        .or_default()
                        .push(predecessor_copy);
                    remaining_predecessors[source.index()] -= 1;
                }
            }
            if inline_small_exit || inline_no_location_block {
                if inline_small_exit
                    && !target_has_no_location
                    && copied.iter().any(|item| {
                        matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                    })
                {
                    // Exit duplication classifies the block after unconditional inlining.
                    // A located exit replaces the old unlocated eval-check jump.
                    exit_without_unique_predecessor[source.index()] = false;
                }
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
                    // Replacing the original jump can append equal records from another array.
                    self.exception_owner_provenance
                        .copied_return_pop_prefixes
                        .retain(|origin| graph.label_block(origin.entry) != Some(source));
                    self.exception_owner_provenance
                        .copied_return_call_prefixes
                        .retain(|origin| graph.label_block(origin.entry) != Some(source));
                    self.exception_owner_provenance
                        .copied_return_attribute_truth_prefixes
                        .retain(|origin| graph.label_block(origin.entry) != Some(source));
                    graph.remove_item_at(source, position);
                }
            } else if let Some(instruction) = graph.instructions_mut(source).next_back() {
                let original = *instruction;
                instruction.retarget(Operand::Forward(copied_label));
                if is_conditional_jump(original.opcode) {
                    // Copying the target leaves this original conditional array intact.
                    owner_retargets.push((original, *instruction));
                }
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
            // A newly copied, unprotected straight-line exit has no native block boundary at
            // this seam. Admit its local pair through the same predicate used by the fuser;
            // the following operation does not determine whether CPython combines the loads.
            // Keep all existing copying paths unchanged, and decline any exception-bearing
            // unit or copy with another label or branch that could observe the entry offset.
            let inline_unprotected_local_pair = inline_small_exit
                && self.exception_regions.is_empty()
                && copied_exception_regions.is_empty()
                && copied_region_exclusions.iter().all(Vec::is_empty)
                && self.borrow_finally_jump_sources.is_empty()
                && !self.preserved_block_boundaries.contains(&copied_label)
                && !self
                    .exception_owner_provenance
                    .block_boundaries
                    .contains(&copied_label)
                && copied.split_first().is_some_and(|(entry, tail)| {
                    matches!(entry, Item::Label(label) if *label == copied_label)
                        && tail.iter().all(|item| {
                            matches!(item, Item::Instruction(instruction)
                                if matches!(instruction.operand(), Operand::Value(_)))
                        })
                        && graph[source].last_instruction().is_some_and(|first| {
                            let Some(Item::Instruction(second)) = tail.first() else {
                                return false;
                            };
                            first.opcode == LOAD_FAST
                                && second.opcode == LOAD_FAST
                                && first.exception_policy() == ExceptionPolicy::default()
                                && second.exception_policy() == ExceptionPolicy::default()
                                && super::locals::fuse_pair(first, second).is_some()
                        })
                });
            // Only the copied entry is artificial. Keep its original target and require
            // the source to have lost exactly its jump, with the four copied records intact.
            let inline_keyword_return_pair =
                keyword_return_seam.is_some_and(|(_, source_items, target_items)| {
                    inline_unprotected_return
                        && !planned_copy
                        && !preserve_inlined_jump_nop
                        && graph[source].items() == &source_items[..source_items.len() - 1]
                        && graph[target].items() == target_items
                        && copied
                            .iter()
                            .skip_while(|item| matches!(item, Item::Label(_)))
                            .copied()
                            .eq(target_items
                                .iter()
                                .skip_while(|item| matches!(item, Item::Label(_)))
                                .enumerate()
                                .map(|(index, item)| match item {
                                    Item::Instruction(instruction) => {
                                        let mut expected = instruction.for_exit_copy();
                                        if index == 0 {
                                            // The existing copy pass carries the removed
                                            // source jump as one pre-fold slot on this load.
                                            expected.add_pre_fold_extra_instructions(1);
                                        }
                                        Item::Instruction(expected)
                                    }
                                    Item::Label(_) => *item,
                                }))
                });
            let inline_protected_return_pair = protected_return_seam.is_some_and(
                |(_, original_return, region_index, source_items, target_items, return_items)| {
                    let [
                        Item::Label(entry),
                        Item::Instruction(second),
                        Item::Instruction(consume),
                        Item::Label(end),
                        Item::Instruction(ret),
                        Item::Label(exclusion_end),
                    ] = copied.as_slice()
                    else {
                        return false;
                    };
                    let [Item::Label(original_end), Item::Instruction(original_ret)] =
                        return_items.as_slice()
                    else {
                        return false;
                    };
                    let [
                        Item::Label(_),
                        Item::Instruction(original_second),
                        Item::Instruction(original_consume),
                    ] = target_items.as_slice()
                    else {
                        return false;
                    };
                    let region = self.exception_regions[*region_index];
                    let mut fragments = copied_exception_regions
                        .iter()
                        .filter(|fragment| fragment.start == *entry);
                    let fragment_matches = fragments.next().is_some_and(|fragment| {
                        fragment.end == *end
                            && fragment.target == region.target
                            && fragment.depth == region.depth
                            && fragment.preserve_lasti == region.preserve_lasti
                            && fragment.is_scope_fallback == region.is_scope_fallback
                    }) && fragments.next().is_none();
                    !preserve_inlined_jump_nop
                        && *entry == copied_label
                        && copied_labels.get(original_end) == Some(end)
                        && *entry != *end
                        && *entry != *exclusion_end
                        && *end != *exclusion_end
                        && graph[source].items() == &source_items[..source_items.len() - 1]
                        && graph[target].items() == target_items
                        && graph[*original_return].items() == return_items
                        && *second == original_second.for_exit_copy()
                        && *consume == original_consume.for_exit_copy()
                        && *ret == original_ret.for_exit_copy()
                        && protected_fragment_origins.contains(region_index)
                        && fragment_matches
                        && !copied_region_exclusions[*region_index].is_empty()
                        && copied_region_exclusions[*region_index]
                            .iter()
                            .all(|range| *range == (*end, *exclusion_end))
                        && !self.preserved_block_boundaries.contains(entry)
                        && !self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(entry)
                },
            );
            if inline_protected_return_pair {
                // Keep the fresh region entry, moving it over only the exact first load
                // already owned by that same region. Its end/POP_BLOCK and return stay put.
                // The normal copy scheduling and later fuser remain unchanged.
                copied.remove(0);
                graph.insert_label(source, 1, copied_label);
            }
            if inline_borrowed_tuple_exit
                || inline_unprotected_local_pair
                || inline_keyword_return_pair
            {
                let first_instruction = copied
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))
                    .unwrap_or(copied.len());
                copied.drain(..first_instruction);
                graph.append_to_block(source, copied);
            } else if inline_no_location_block
                || inline_unprotected_return
                || (inline_small_exit && target_terminal_opcode == Some(RERAISE))
                || planned_copy
            {
                let copied_jump_target =
                    block_jump_target(&copied).and_then(|label| graph.label_block(label));
                graph.append_to_block(source, copied);
                if preserves_shared_match_exit && generic_original_copy {
                    // Recheck the complete expanded array for each conditional copy;
                    // another inlining, retarget or label edit retires this admission.
                    shared_match_cleanup_copies.insert(source, graph[source].items().to_vec());
                    exit_without_unique_predecessor[source.index()] =
                        remaining_predecessors[source.index()] > 1;
                }
                // Deferred copies receive this classification before conditional duplication.
                // Keep it when a reraising exit is appended during the fixed-point scan.
                if inline_unprotected_return
                    || (inline_small_exit && target_terminal_opcode == Some(RERAISE))
                {
                    let unlocated_scope_exit = graph[source].items().iter().all(|item| {
                        !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                    }) && graph[source]
                        .last_instruction()
                        .is_some_and(|instruction| {
                            ends_scope(instruction.opcode) || has_jump_eval_check(instruction)
                        });
                    exit_without_unique_predecessor[source.index()] |=
                        remaining_predecessors[source.index()] > 1 && unlocated_scope_exit;
                    if unlocated_scope_exit
                        && inline_small_exit
                        && target_terminal_opcode == Some(RERAISE)
                        && source_unlocated_inlining
                    {
                        // Preserve the array's inlining permission after replacing its jump.
                        // The removed jump's unconditional-only restriction does not apply to
                        // the new scope exit: shared unlocated exits also need conditional copies.
                        // Keep any restriction already present on the copied target itself.
                        graph
                            .instructions_mut(source)
                            .next_back()
                            .unwrap()
                            .no_location_policy
                            .allow_block_inlining();
                    }
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
                if !planned_copy {
                    sources.push(source);
                }
                // A completed return or reraising cleanup also makes earlier predecessors
                // eligible for a small exit copy. Revisit them before conditional duplication.
                for predecessor in jump_sources[source.index()].iter().copied() {
                    if !planned_copy
                        && positions[predecessor.index()] < source_position
                        && block_jump_target(graph[predecessor].items())
                            .and_then(|label| graph.label_block(label))
                            == Some(source)
                    {
                        sources.push(predecessor);
                    }
                }
            } else if source_has_fallthrough {
                // Conditional threading follows exit copying in CPython. A copied original
                // case jump retains the same threading certificate as its source label.
                for item in target_blocks.iter().flat_map(|block| graph[*block].items()) {
                    if let Item::Label(original) = item
                        && let Some(&location) = self.conditional_threading_targets.get(original)
                        && let Some(replacement) = copied_labels.get(original)
                    {
                        self.conditional_threading_targets
                            .insert(*replacement, location);
                    }
                }
                if let Some(location) = graph[target].items().iter().find_map(|item| match item {
                    Item::Label(label) => self.conditional_threading_targets.get(label).copied(),
                    Item::Instruction(_) => None,
                }) {
                    self.conditional_threading_targets
                        .insert(copied_label, location);
                }
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
                // A copied entry must not combine one alias's ordinary-jump head with
                // another alias's common-loop location certificate. Both facts must come
                // from the same original label before the copy can thread as a backedge.
                if copied.iter().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                }) == Some(JUMP_BACKWARD)
                    && self.original_jump_head_targets.contains(&copied_label)
                    && let Some(&location) = self.conditional_threading_targets.get(&copied_label)
                    && !graph[target].items().iter().any(|item| {
                        matches!(item, Item::Label(original)
                            if self.original_jump_head_targets.contains(original)
                                && self.conditional_threading_targets.get(original) == Some(&location))
                    })
                {
                    self.original_jump_head_targets.remove(&copied_label);
                }
                // Exception-region labels can split one original cleanup array. Insert its
                // copies after the whole array so the original still reaches its own jump.
                if target != target_exit && target_has_unlocated_jump_eval_check {
                    copied_split_eval_starts.insert(target_exit, target_position);
                }
                target_copies.entry(target_exit).or_default().push(copied);
            } else {
                inline_copies.entry(source).or_default().push(copied);
            }
            remaining_predecessors[target.index()] -= 1;
            if remaining_predecessors[target.index()] == 0 {
                drop_blocks[target.index()] = true;
            }
        }

        // Location restoration must follow every copy round: locating a join's NOP
        // earlier can change the eligibility of a later original-array copy. Copies
        // receive new labels, so this seam certifies only the original jump marker.
        for (source, successor, mut expected, location) in original_join_marker_donors {
            let items = graph[source].items();
            let Some(successor_position) = items
                .iter()
                .position(|item| matches!(item, Item::Label(label) if *label == successor))
            else {
                continue;
            };
            let Some(position) = items[..successor_position]
                .iter()
                .rposition(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            let Some(marker) = graph.instruction_mut(ItemPosition::new(source, position)) else {
                continue;
            };
            // The ordinary location writer may have visited this exact marker while
            // finishing unconditional copies. Every other record field must agree.
            expected.location = marker.location;
            if *marker == expected {
                marker.location = location;
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
            let label_positions = order
                .iter()
                .flat_map(|block| {
                    graph[*block]
                        .items()
                        .iter()
                        .chain(inline_copies.get(block).into_iter().flatten().flatten())
                        .chain(
                            target_copies
                                .get(block)
                                .into_iter()
                                .flatten()
                                .rev()
                                .flatten(),
                        )
                })
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
                    // Sort and merge complete holes inside the original protected region.
                    // Later copies can extend an earlier hole; applying discovery order
                    // would incorrectly protect part of that copied exit again.
                    let Some((&region_start, &region_end)) = label_positions
                        .get(&region.start)
                        .zip(label_positions.get(&region.end))
                    else {
                        continue;
                    };
                    let Some(ranges) = exclusions
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
                    exclusions.sort_by_key(|(start, _)| label_positions[start]);
                    let mut merged = Vec::<(Label, Label)>::with_capacity(exclusions.len());
                    for &(start, end) in exclusions.iter() {
                        if let Some((_, previous_end)) = merged.last_mut()
                            && label_positions[&start] < label_positions[previous_end]
                        {
                            if label_positions[&end] > label_positions[previous_end] {
                                *previous_end = end;
                            }
                        } else {
                            merged.push((start, end));
                        }
                    }
                    *exclusions = merged;
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
            if preserve_original_shared_jump_location(
                graph,
                target,
                remaining_predecessors[target.index()],
                original_location_absence,
            ) {
                continue;
            }
            if planned_blocks.contains(&target)
                && remaining_predecessors[target.index()] > 1
                && original_entry_has_no_location_donor(
                    graph[target].items(),
                    original_location_absence,
                )
                && has_shared_cleanup_entry(
                    graph,
                    &order,
                    positions[target.index()],
                    jump_sources[target.index()]
                        .iter()
                        .filter(|source| {
                            block_has_fallthrough(graph[**source].items())
                                && block_jump_target(graph[**source].items())
                                    .and_then(|label| graph.label_block(label))
                                    == Some(target)
                        })
                        .count(),
                    remaining_predecessors[target.index()],
                )
            {
                // Conditional edges and fallthrough still share this original array.
                // Its original entry had no line donor before jump threading either.
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
                && shared_match_cleanup_copies
                    .get(&target)
                    .is_some_and(|items| {
                        graph[target].items() == items
                            && items.iter().all(|item| {
                                !matches!(item,
                            Item::Instruction(instruction)
                                if instruction.no_location_policy.preserves_location())
                            })
                    })
            {
                let mut live_sources =
                    jump_sources[target.index()]
                        .iter()
                        .copied()
                        .filter(|source| {
                            !drop_blocks[source.index()]
                                && block_jump_target(graph[*source].items())
                                    .and_then(|label| graph.label_block(label))
                                    == Some(target)
                        });
                if let Some(source) = live_sources.next()
                    && live_sources.next().is_none()
                    && let Some(instruction) = graph[source].last_instruction()
                    && is_conditional_jump(instruction.opcode)
                    && instruction.location.line >= 0
                {
                    // Other conditions received copies. The exact surviving original
                    // array now belongs to this sole condition, including its inlined
                    // POP_BLOCK and jump whose previous donor came from another edge.
                    let location = instruction.location;
                    for instruction in graph.instructions_mut(target) {
                        instruction.location = location;
                    }
                    continue;
                }
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
            if remaining_predecessors[target.index()] == 1
                && predecessors[target.index()] > 1
                && let Some(instruction) = graph.instructions_mut(target).next()
                && instruction
                    .no_location_policy
                    .copy_inherits_shared_predecessor()
            {
                // Duplicating another incoming edge also gives the original cleanup a
                // unique predecessor; it follows the same location rule as the copies.
                instruction.no_location_policy.clear_preserved_location();
            }
            if remaining_predecessors[target.index()] == 1
                && predecessors[target.index()] > 1
                && graph[target]
                    .items()
                    .iter()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .any(|item| {
                        matches!(item, Item::Label(label) if self.shared_jump_copy_location_blocks.contains(label))
                    })
                && let Some(instruction) = graph.instructions_mut(target).next()
                && has_jump_eval_check(instruction)
            {
                // A shared original jump becomes a private eval-check exit after its other
                // predecessors receive copies. Resolve its remaining predecessor's line too.
                instruction.no_location_policy.clear_preserved_location();
            }
            let copied_exit_needs_location = remaining_predecessors[target.index()] == 1
                && (predecessors[target.index()] > 1
                    || retained_unlocated_join_blocks.contains(&target)
                    // Scheduled copies restore the original leading jump NOP even
                    // when its conditional predecessor was already unique.
                    || planned_blocks.contains(&target))
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
                if remaining_predecessors[target.index()] == 1
                    && predecessors[target.index()] > 1
                    && positions[source.index()] + 1 == target_position
                    && block_has_fallthrough(graph[source].items())
                    && let Some(location) = match_join_store_exit_location(
                        graph[source].items(),
                        graph[target].items(),
                        &self.match_join_store_origins,
                    )
                {
                    return Some(location);
                }
                if let Some(location) = comprehension_assignment_exit_location(
                    graph[source].items(),
                    graph[target].items(),
                    &self.comprehension_assignment_exits,
                ) {
                    return Some(location);
                }
                if remaining_predecessors[target.index()] == 1
                    && positions[source.index()] + 1 == target_position
                    && block_has_fallthrough(graph[source].items())
                    && let Some(location) = named_handler_cleanup_exit_location(
                        graph[source].items(),
                        graph[target].items(),
                    )
                    && graph[target]
                        .items()
                        .iter()
                        .take_while(|item| matches!(item, Item::Label(_)))
                        .all(|item| {
                            let Item::Label(label) = item else {
                                unreachable!();
                            };
                            self.exception_regions
                                .iter()
                                .any(|region| region.end == *label)
                                && !self.preserved_block_boundaries.contains(label)
                                && !self
                                    .exception_owner_provenance
                                    .source_boundaries
                                    .contains(label)
                                && !self
                                    .exception_owner_provenance
                                    .block_boundaries
                                    .contains(label)
                                && !self
                                    .exception_owner_provenance
                                    .setups
                                    .iter()
                                    .any(|(setup, _)| setup == label)
                        })
                {
                    // The POP_EXCEPT donor is already resolved. Its name-clearing tail
                    // receives that line later, after this original exit is visited.
                    return Some(location);
                }
                if let Some(&start) = copied_split_eval_starts.get(&target)
                    && remaining_predecessors[target.index()] == 1
                    && positions[source.index()] + 1 == target_position
                {
                    // The original cleanup's prefix now has its predecessor's location,
                    // but within-block propagation runs later. Cross only the table splits
                    // already validated when copying this same unlocated eval-check array.
                    return order[start..target_position]
                        .iter()
                        .flat_map(|block| graph[*block].items())
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
                }
                if remaining_predecessors[target.index()] == 1
                    && predecessors[target.index()] > 1
                    && !planned_blocks.contains(&target)
                    && positions[source.index()] + 1 == target_position
                    && block_has_fallthrough(graph[source].items())
                    && let Some(location) = triple_store_handler_exits
                        .location(graph[source].items(), graph[target].items())
                        .or_else(|| fused_store_tail_location(graph[source].items()))
                {
                    // The captured triple keeps its original last-store donor across
                    // static swaps. Other reversed pairs use their surviving fused line.
                    return Some(location);
                }
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

        // A unique remaining predecessor can locate the first restored POP or
        // replaced jump only after copying. Carry that donation through its
        // original array before NOP compaction, just as for the copied arrays.
        propagate_planned_array_locations(graph, &order, &planned_blocks, &planned_inline_labels);

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
                    if let Item::Label(label) = item {
                        self.exception_owner_provenance
                            .retire_async_with_setup(*label);
                        self.exception_owner_provenance
                            .copied_return_pop_prefixes
                            .retain(|origin| origin.entry != *label);
                        self.exception_owner_provenance
                            .copied_return_call_prefixes
                            .retain(|origin| origin.entry != *label);
                        self.exception_owner_provenance
                            .copied_return_attribute_truth_prefixes
                            .retain(|origin| origin.entry != *label);
                        // Labels retained without their array cannot certify a later seam.
                        self.exception_owner_provenance
                            .normal_try_copy_inline_labels
                            .remove(label);
                        if let Some(mut admissions) = self.discarded_fold_admissions.remove(label) {
                            discarded_admissions.append(&mut admissions);
                        }
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
        restore_retained_pop_block_locations(
            graph,
            &self.exception_regions,
            &self.preserved_block_boundaries,
            &self.retained_pop_block_origins,
        );
        restore_normal_try_pop_holes(graph, &mut self.exception_regions, &original_pop_ranges);
        if generic_original_arrays {
            let handler_entries = self
                .exception_regions
                .iter()
                .map(|region| region.target)
                .chain(
                    self.exception_owner_provenance
                        .setups
                        .iter()
                        .map(|(label, _)| *label),
                )
                .collect();
            propagate_normal_try_pop_continuations(graph, &original_pop_ranges, &handler_entries);
        }
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

    fn retained_head() -> Item {
        let Item::Instruction(mut instruction) = head() else {
            unreachable!()
        };
        instruction.jump_policy.preserve_explicit_nop();
        instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        Item::Instruction(instruction)
    }

    fn check_retained(
        items: Vec<Item>,
        regions: &[ExceptionRegion],
        entry: Label,
        prefix: Vec<Label>,
        expected_line: Option<i32>,
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
        if let Some(line) = expected_line {
            let target = expected.label_block(entry).unwrap();
            expected.instructions_mut(target).next().unwrap().location =
                SourceLocation::new(line, line, 12, 17);
        }
        restore_retained_pop_block_locations(
            &mut actual,
            regions,
            &boundaries,
            &[(entry, prefix)].into_iter().collect(),
        );
        assert_eq!(
            format!("{:?}", actual.iter_items().collect::<Vec<_>>()),
            format!("{:?}", expected.iter_items().collect::<Vec<_>>()),
        );
    }

    #[test]
    fn normal_try_pop_continuations_require_a_live_original_slot() {
        for (empty, entered) in [(false, false), (true, false), (false, true)] {
            let start = Label(1);
            let end = Label(2);
            let target = Label(3);
            let Item::Instruction(mut pop) = head() else {
                unreachable!();
            };
            pop.location = SourceLocation::new(7, 7, 12, 17);
            let mut items = Vec::new();
            if !empty {
                items.push(Item::Label(start));
            }
            items.push(Item::Instruction(pop));
            if empty {
                items.push(Item::Label(start));
            }
            items.extend([
                Item::Label(end),
                jump(target),
                Item::Label(target),
                located(9),
            ]);
            let mut graph = BlockGraph::from_items(items, &FxHashSet::default());
            let forbidden = if entered {
                [end].into_iter().collect()
            } else {
                FxHashSet::default()
            };
            propagate_normal_try_pop_continuations(&mut graph, &[(start, end)], &forbidden);
            let jump = graph
                .iter_items()
                .find_map(|item| match item {
                    Item::Instruction(instruction) if instruction.opcode == JUMP_FORWARD => {
                        Some(instruction)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(jump.location.line, if !empty && !entered { 7 } else { -1 });
        }
    }

    #[test]
    fn retained_pop_block_uses_only_its_original_empty_prefix() {
        for empty_count in [1, 3] {
            for omit_origin in [false, true] {
                let entry = Label(empty_count + 1);
                let mut items = vec![
                    Item::Label(Label(0)),
                    instruction(JUMP_FORWARD, Operand::Forward(Label(1)), 8),
                ];
                items.extend((1..=empty_count).map(|index| Item::Label(Label(index))));
                items.extend([
                    Item::Label(entry),
                    retained_head(),
                    located(20),
                    instruction(RETURN_VALUE, Operand::Value(0), 20),
                ]);
                let prefix = ((if omit_origin { 2 } else { 1 })..=empty_count + 1)
                    .map(Label)
                    .collect();
                check_retained(items, &[], entry, prefix, (!omit_origin).then_some(8));
            }
        }
    }

    #[test]
    fn retained_pop_block_rejects_a_shared_empty_predecessor() {
        let empty = Label(2);
        let entry = Label(3);
        check_retained(
            vec![
                Item::Label(Label(0)),
                instruction(LOAD_FAST, Operand::Value(0), 1),
                instruction(POP_JUMP_IF_FALSE, Operand::Forward(empty), 1),
                Item::Label(Label(1)),
                instruction(JUMP_FORWARD, Operand::Forward(empty), 8),
                Item::Label(empty),
                Item::Label(entry),
                retained_head(),
                located(20),
                instruction(RETURN_VALUE, Operand::Value(0), 20),
            ],
            &[],
            entry,
            vec![empty, entry],
            None,
        );
    }

    #[test]
    fn retained_pop_block_rejects_handler_and_unlocated_donors() {
        for handler in [false, true] {
            let empty = Label(1);
            let entry = Label(2);
            let end = Label(3);
            let regions = if handler {
                vec![region(Label(0), end, empty)]
            } else {
                Vec::new()
            };
            check_retained(
                vec![
                    Item::Label(Label(0)),
                    located(8),
                    instruction(
                        JUMP_FORWARD,
                        Operand::Forward(empty),
                        if handler { 8 } else { -1 },
                    ),
                    Item::Label(empty),
                    Item::Label(entry),
                    retained_head(),
                    located(20),
                    instruction(RETURN_VALUE, Operand::Value(0), 20),
                    Item::Label(end),
                ],
                &regions,
                entry,
                vec![empty, entry],
                None,
            );
        }
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
