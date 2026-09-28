use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::super::{
    ConstantTable, ExceptionExclusion, ExceptionPolicy, ExceptionRegion, Instruction, Item,
    JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, Label, Operand, OptimizationPipeline,
    SourceLocation, block_has_fallthrough, block_jump_target, cleanup_block_creation_order,
    ends_scope,
};
use crate::assembler::block_graph::{BasicBlockId, BlockGraph, ItemPosition};
use crate::assembler::exception_owners::ExceptionOwnerProvenance;
use crate::target::opcodes::{
    BUILD_TUPLE, CALL, CALL_KW, DELETE_FAST, END_ASYNC_FOR, END_SEND, FOR_ITER, LOAD_ATTR,
    LOAD_CONST, LOAD_DEREF, LOAD_FAST, LOAD_GLOBAL, LOAD_SMALL_INT, NOP, NOT_TAKEN, POP_EXCEPT,
    POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, POP_TOP,
    RAISE_VARARGS, RERAISE, RETURN_VALUE, STORE_ATTR, STORE_FAST, SWAP,
};
use crate::target::{is_conditional_jump, is_unconditional_jump};

/// Recognizes a located attribute assignment with exactly four original copy slots.
fn is_four_slot_attribute_store_jump(items: &[Item]) -> bool {
    let Some(first) = items
        .iter()
        .position(|item| matches!(item, Item::Instruction(_)))
    else {
        return false;
    };
    matches!(
        &items[first..],
        [Item::Instruction(value), Item::Instruction(object), Item::Instruction(store), Item::Instruction(jump)]
            if matches!(value.opcode, LOAD_CONST | LOAD_SMALL_INT)
                && object.opcode == LOAD_FAST
                && store.opcode == STORE_ATTR
                && jump.opcode == JUMP_FORWARD
                && [value, object, store, jump].iter().all(|instruction| {
                    instruction.location.line >= 0
                        && instruction.pre_fold_extra_instructions() == 0
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                })
    )
}

/// Certifies the original comparison bridge and its four-record subject-pop return.
fn copied_match_guard_subject_donor(
    source: &[Item],
    target: &[Item],
    predecessor: &Instruction,
    origins: &[(Label, Label, [Instruction; 6])],
) -> Option<(SourceLocation, [Instruction; 4])> {
    let target_start = target
        .iter()
        .position(|item| matches!(item, Item::Instruction(_)))?;
    let [
        Item::Instruction(pop),
        Item::Instruction(global),
        Item::Instruction(attribute),
        Item::Instruction(returned),
    ] = &target[target_start..]
    else {
        return None;
    };
    let mut expected_pop = Instruction::new(POP_TOP, Operand::Value(0), pop.location, Some(0));
    expected_pop.fusion_policy.mark_location_from_next();
    if *pop != expected_pop
        || global.opcode != LOAD_GLOBAL
        || !matches!(global.operand(), Operand::Value(argument) if argument & 1 == 0)
        || attribute.opcode != LOAD_ATTR
        || !matches!(attribute.operand(), Operand::Value(argument) if argument & 1 == 0)
        || returned.opcode != RETURN_VALUE
        || returned.operand() != Operand::Value(0)
        || [global, attribute, returned].iter().any(|instruction| {
            instruction.location.line < 0
                || **instruction
                    != Instruction::new(
                        instruction.opcode,
                        instruction.operand(),
                        instruction.location,
                        Some(u32::from(instruction.opcode != RETURN_VALUE)),
                    )
        })
    {
        return None;
    }
    let source_start = source
        .iter()
        .position(|item| matches!(item, Item::Instruction(_)))?;
    let [Item::Instruction(marker), Item::Instruction(bridge)] = &source[source_start..] else {
        return None;
    };
    if bridge.location.line < 0
        || !bridge.jump_policy.inline_small_exit()
        || bridge.jump_policy.retained_nop() != super::super::ir::RetainedJumpNop::Always
        || bridge.location != predecessor.location
        || marker.location != bridge.location
        || bridge.depth_after() != Some(1)
        || marker.depth_after() != Some(1)
        || predecessor.depth_after() != Some(1)
    {
        return None;
    }
    origins
        .iter()
        .any(|(_, end, records)| {
            records[1] == *predecessor
                && records[2] == *marker
                && records[3] == *bridge
                && bridge.operand() == Operand::Forward(*end)
                && target[..target_start].contains(&Item::Label(*end))
        })
        .then_some((bridge.location, [*pop, *global, *attribute, *returned]))
}

/// Retains exactly the incoming-edge information used by original-line propagation.
#[derive(Clone, Copy)]
enum LocationPredecessors {
    None,
    One(usize),
    Multiple,
}

impl LocationPredecessors {
    fn add(&mut self, predecessor: usize) {
        *self = match *self {
            Self::None => Self::One(predecessor),
            Self::One(_) | Self::Multiple => Self::Multiple,
        };
    }
}

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
    if !graph.features().has_forward_jump()
        || !graph.iter_items().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if instruction.opcode == JUMP_BACKWARD
                    && matches!(instruction.operand(), Operand::Forward(_)))
        })
    {
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

// Dispose of a shared cleanup's stack values, optionally preserving a pending value.
// Deferred source locations must resolve through their own writers, which do not all
// honor PRESERVE.
fn is_unlocated_value_cleanup_prefix(items: &[Item], python_version: crate::PythonVersion) -> bool {
    let mut instructions = items.iter().filter_map(|item| match item {
        Item::Instruction(instruction) => Some(instruction),
        Item::Label(_) => None,
    });
    let Some(first) = instructions.next() else {
        return false;
    };
    let slots = match (first.opcode, first.operand()) {
        (SWAP, Operand::Value(2)) => 1,
        // Python 3.15 keeps an iterator index beside the iterator itself.
        (SWAP, Operand::Value(3)) if python_version == crate::PythonVersion::Py315 => 2,
        // A literal return has no pending value to preserve across those two pops.
        (POP_TOP, Operand::Value(0)) if python_version == crate::PythonVersion::Py315 => 1,
        _ => return false,
    };
    let unlocated = |instruction: &Instruction| {
        instruction.location.line < 0
            && !instruction.fusion_policy.location_from_next()
            && instruction.no_location_policy.percent_target_location == 0
    };
    if !unlocated(first) {
        return false;
    }
    for _ in 0..slots {
        let Some(discard) = instructions.next() else {
            return false;
        };
        if discard.opcode != POP_TOP || !unlocated(discard) {
            return false;
        }
    }
    instructions.next().is_none()
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
    if !original_nop_targets.is_empty()
        && items
            .iter()
            .any(|item| matches!(item, Item::Label(label) if original_nop_targets.contains(label)))
    {
        return false;
    }
    let prefix_labels = items
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    if let [
        Item::Instruction(value),
        Item::Instruction(receiver),
        Item::Instruction(store),
        Item::Instruction(raise),
    ] = &items[prefix_labels..]
    {
        // A literal attribute store followed by a bare raise still has exactly four
        // original slots. The raise uses the active handler instead of copying finally
        // cleanup, so an enclosing try's return-copy barrier does not apply.
        return value.opcode == LOAD_CONST
            && matches!(receiver.opcode, LOAD_FAST | LOAD_DEREF)
            && store.opcode == STORE_ATTR
            && raise.opcode == RAISE_VARARGS
            && raise.operand() == Operand::Value(0)
            && [value, receiver, store, raise].iter().all(|instruction| {
                matches!(instruction.operand(), Operand::Value(_))
                    && instruction.location.line >= 0
                    && instruction.pre_fold_extra_instructions() == 0
                    && instruction.local_policy.hidden_exception_owner_slots() == 0
                    && instruction.exception_policy() == ExceptionPolicy::default()
            });
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
    if !original_nop_targets.is_empty()
        && items
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

/// Authenticates a shared unlocated jump after exactly one original jump was threaded.
fn is_shared_threaded_handler_jump(
    items: &[Item],
    shared_jumps: &FxHashSet<Label>,
    original_nops: &FxHashSet<Label>,
    provenance: &ExceptionOwnerProvenance,
) -> bool {
    let entry_len = items
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [Item::Instruction(jump)] = &items[entry_len..] else {
        return false;
    };
    items[..entry_len]
        .iter()
        .any(|item| matches!(item, Item::Label(label) if shared_jumps.contains(label)))
        && items[..entry_len].iter().all(|item| {
            matches!(item, Item::Label(label)
                if !original_nops.contains(label)
                    && !provenance.omitted_nop_counts.contains_key(label)
                    && !provenance.omitted_nop_locations.contains_key(label))
        })
        && jump.opcode == JUMP_FORWARD
        && jump.location == SourceLocation::NONE
        && jump.no_location_policy.preserves_location()
        && !jump.no_location_policy.is_artificial_eval_break()
        && !jump.jump_policy.is_interruptible()
        && jump.pre_fold_extra_instructions() == 1
        && jump.local_policy.hidden_exception_owner_slots() == 0
        && jump.exception_policy() == ExceptionPolicy::default()
}

/// Recognizes a recorded unnamed cleanup whose original `POP_BLOCK` survives in its width.
fn unnamed_cleanup_has_short_return(
    graph: &BlockGraph,
    target: BasicBlockId,
    recorded_exits: &FxHashSet<Label>,
) -> bool {
    let items = graph[target].items();
    let entry_len = items
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [Item::Instruction(pop), Item::Instruction(jump)] = &items[entry_len..] else {
        return false;
    };
    if pop.opcode != POP_EXCEPT
        || pop.pre_fold_extra_instructions() != 1
        || jump.opcode != JUMP_FORWARD
        || jump.pre_fold_extra_instructions() != 0
        || [pop, jump].iter().any(|instruction| {
            instruction.location != SourceLocation::NONE
                || instruction.local_policy.hidden_exception_owner_slots() != 0
        })
    {
        return false;
    }
    let Operand::Forward(label) = jump.operand() else {
        return false;
    };
    if !recorded_exits.contains(&label) {
        return false;
    }
    let Some(successor) = graph.label_block(label) else {
        return false;
    };
    let returned = graph[successor].items();
    // An explicit same-line constant return retains its authored NOP during exit
    // copying. Its complete original array makes this cleanup larger still.
    if super::control_flow::is_original_constant_return(graph, label) {
        return returned.iter().all(|item| {
            !matches!(item, Item::Instruction(instruction)
                if instruction.local_policy.hidden_exception_owner_slots() != 0)
        });
    }
    let entry_len = returned
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [Item::Instruction(value), Item::Instruction(exit)] = &returned[entry_len..] else {
        return false;
    };
    // A bare global load occupies one original slot. The low-bit NULL form
    // also represents PUSH_NULL and cannot use this two-record certificate.
    (matches!(value.opcode, LOAD_FAST | LOAD_CONST)
        || (value.opcode == LOAD_GLOBAL
            && matches!(value.operand(), Operand::Value(argument) if argument & 1 == 0)))
        && exit.opcode == RETURN_VALUE
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

#[derive(Default)]
struct RepresentedSetupNops {
    setups: FxHashSet<Label>,
    labels: FxHashSet<Label>,
}

/// Finds retained SETUP records and their exact metadata-only NOP boundaries.
fn represented_setup_nops(
    graph: &BlockGraph,
    blocks: &[BasicBlockId],
    provenance: &ExceptionOwnerProvenance,
) -> RepresentedSetupNops {
    if blocks.len() <= 1
        || provenance.setup_nop_ranges.is_empty()
        || !blocks.iter().any(|block| {
            graph[*block].items().iter().any(
                |item| matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP),
            )
        })
    {
        return RepresentedSetupNops::default();
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
            {
                return None;
            }
            let mut ranges = provenance.setup_nop_ranges.iter().filter(|(start, end)| {
                positions.get(start) == Some(&nop_index) && positions.get(end) == Some(&index)
            });
            let &(start, end) = ranges.next()?;
            if ranges.next().is_some() {
                return None;
            }
            Some((setup, start, end))
        })
        .fold(
            RepresentedSetupNops::default(),
            |mut represented, (setup, start, end)| {
                represented.setups.insert(setup);
                // The retained NOP already contributes the original SETUP slot. Its
                // registered exclusion boundaries do not add another original record.
                represented.labels.extend([setup, start, end]);
                represented
            },
        )
}

/// Reconstructs one original exit array across exception-table-only splits.
fn metadata_split_exit_chain<'a>(
    graph: &BlockGraph,
    order: &'a [BasicBlockId],
    start: usize,
    predecessors: &[usize],
    exception_split_labels: &FxHashSet<Label>,
) -> &'a [BasicBlockId] {
    let mut end = start;
    while end + 1 < order.len()
        && block_has_fallthrough(graph[order[end]].items())
        // A conditional jump ends the original array even when its fallthrough
        // also starts an exception-table range.
        && block_jump_target(graph[order[end]].items()).is_none()
        && predecessors[order[end + 1].index()] == 1
        && graph[order[end + 1]].items().iter().any(
            |item| matches!(item, Item::Label(label) if exception_split_labels.contains(label)),
        )
    {
        end += 1;
    }
    &order[start..=end]
}

struct ExitBlockAnalysis {
    positions: Vec<usize>,
    predecessors: Vec<usize>,
    jump_predecessors: Vec<Option<BasicBlockId>>,
    conditional_predecessors: Vec<usize>,
    exception_handler_blocks: Vec<bool>,
    requires_copy: bool,
}

#[derive(Clone, Copy, Default)]
struct ExitTargetAnalysis {
    inline_small_exit: bool,
    implicit_small_raise: bool,
    copy_conditional_split_return: bool,
    no_location_exit: bool,
    shared_exit: bool,
}

/// Tests source and terminal prerequisites before inspecting a target's complete location data.
#[inline]
fn source_may_copy_exit(
    graph: &BlockGraph,
    order: &[BasicBlockId],
    positions: &[usize],
    predecessors: &[usize],
    exception_regions: &[ExceptionRegion],
    original_nop_targets: &FxHashSet<Label>,
    source: BasicBlockId,
) -> bool {
    let Some(source_last) = graph[source].last_instruction() else {
        return false;
    };
    let (Operand::Forward(label) | Operand::Backward(label)) = source_last.operand() else {
        return false;
    };
    let Some(target) = graph.label_block(label) else {
        return false;
    };
    let Some(target_last) = graph[target].last_instruction() else {
        return false;
    };
    let source_falls =
        !ends_scope(source_last.opcode) && !is_unconditional_jump(source_last.opcode);
    let shared = predecessors[target.index()] > 1;

    if (!source_falls && source_last.jump_policy.inline_small_exit())
        || (source_falls && shared && source_last.no_location_policy.copies_split_return_exit())
        || (shared && target_last.no_location_policy.is_artificial_eval_break())
    {
        return true;
    }
    if !source_falls
        && target_last.opcode == RAISE_VARARGS
        && (block_is_bare_raise(graph[target].items(), original_nop_targets)
            || block_is_local_raise(graph[target].items(), original_nop_targets))
    {
        return true;
    }

    let target_falls =
        !ends_scope(target_last.opcode) && !is_unconditional_jump(target_last.opcode);
    let shared_unlocated =
        shared && (ends_scope(target_last.opcode) || has_jump_eval_check(target_last));
    let inline_unlocated = !target_falls
        && target_last.no_location_policy.allows_block_inlining()
        && (!source_falls || shared);
    let can_split = !exception_regions.is_empty() && target_falls;
    if !(shared_unlocated || inline_unlocated || can_split)
        || graph[target].items().iter().any(
            |item| matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0),
        )
    {
        return false;
    }
    shared_unlocated
        || inline_unlocated
        || order
            .get(positions[target.index()] + 1)
            .is_some_and(|next| {
                predecessors[next.index()] == 1
                    && graph[*next].items().iter().any(|item| {
                        matches!(item, Item::Label(label) if exception_regions.iter().any(|region| {
                            region.start == *label || region.end == *label
                        }))
                    })
            })
}

#[cfg(test)]
fn source_may_copy_exit_original(
    graph: &BlockGraph,
    order: &[BasicBlockId],
    positions: &[usize],
    predecessors: &[usize],
    exception_regions: &[ExceptionRegion],
    original_nop_targets: &FxHashSet<Label>,
    source: BasicBlockId,
) -> bool {
    let Some(target) =
        block_jump_target(graph[source].items()).and_then(|label| graph.label_block(label))
    else {
        return false;
    };
    let Some(target_last) = graph[target].last_instruction() else {
        return false;
    };
    let target_has_no_location = graph[target].items().iter().all(
        |item| !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0),
    );
    let source_has_fallthrough = block_has_fallthrough(graph[source].items());
    let shared_exit = predecessors[target.index()] > 1
        && ((target_has_no_location
            && (ends_scope(target_last.opcode) || has_jump_eval_check(target_last)))
            || target_last.no_location_policy.is_artificial_eval_break());
    let no_location_exit = target_has_no_location
        && !block_has_fallthrough(graph[target].items())
        && target_last.no_location_policy.allows_block_inlining()
        && (!source_has_fallthrough || predecessors[target.index()] > 1);
    let split_no_location_exit = !exception_regions.is_empty()
        && target_has_no_location
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
        && ((target_last.opcode == RAISE_VARARGS
            && (block_is_bare_raise(graph[target].items(), original_nop_targets)
                || block_is_local_raise(graph[target].items(), original_nop_targets)))
            || graph[source]
                .last_instruction()
                .is_some_and(|instruction| instruction.jump_policy.inline_small_exit()));
    let conditional_split_return_source = source_has_fallthrough
        && predecessors[target.index()] > 1
        && graph[source]
            .last_instruction()
            .is_some_and(|instruction| instruction.no_location_policy.copies_split_return_exit());
    shared_exit
        || no_location_exit
        || split_no_location_exit
        || small_exit_source
        || conditional_split_return_source
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
        positions[block.index()] = position;
        let (has_fallthrough, jump) = graph[block].successors();
        if has_fallthrough && let Some(next) = order.get(position + 1) {
            predecessors[next.index()] += 1;
        }
        if let Some(target) = jump.and_then(|label| graph.label_block(label)) {
            predecessors[target.index()] += 1;
            conditional_predecessors[target.index()] += usize::from(has_fallthrough);
            if jump_predecessors[target.index()].is_none() {
                jump_predecessors[target.index()] = Some(block);
            }
        }
    }

    // This cheap scan is a superset of the exact copy conditions. A false result is sufficient
    // to skip the exception-boundary and target-chain analysis.
    let possible_copy = order.iter().copied().any(|source| {
        source_may_copy_exit(
            graph,
            order,
            &positions,
            &predecessors,
            exception_regions,
            original_nop_targets,
            source,
        )
    });
    #[cfg(test)]
    assert_eq!(
        possible_copy,
        order.iter().copied().any(|source| {
            source_may_copy_exit_original(
                graph,
                order,
                &positions,
                &predecessors,
                exception_regions,
                original_nop_targets,
                source,
            )
        })
    );
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
    let mut target_analyses = vec![None; vector_len];
    for source in order.iter().copied() {
        let Some(source_last) = graph[source].last_instruction() else {
            continue;
        };
        let (Operand::Forward(label) | Operand::Backward(label)) = source_last.operand() else {
            continue;
        };
        let Some(target) = graph.label_block(label) else {
            continue;
        };
        let analysis = target_analyses[target.index()].get_or_insert_with(|| {
            let target_position = positions[target.index()];
            let target_blocks = metadata_split_exit_chain(
                graph,
                order,
                target_position,
                &predecessors,
                &exception_split_labels,
            );
            let target_chain_end = target_position + target_blocks.len() - 1;
            let target_exit = *target_blocks.last().expect("target chain is nonempty");
            let Some(target_last) = graph[target_exit].last_instruction() else {
                return ExitTargetAnalysis::default();
            };
            let target_has_no_location = target_blocks.iter().all(|block| {
                graph[*block].items().iter().all(|item| {
                    !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                })
            });
            if !ends_scope(target_last.opcode)
                && !target_has_no_location
                && !target_last.no_location_policy.is_artificial_eval_break()
            {
                return ExitTargetAnalysis::default();
            }
            let target_is_small_exit = if ends_scope(target_last.opcode)
                && (target_chain_end == target_position
                    || matches!(target_last.opcode, RAISE_VARARGS | RETURN_VALUE))
            {
                let represented_setups = represented_setup_nops(graph, target_blocks, provenance);
                let mut at_target_entry = true;
                target_blocks
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
                                && !represented_setups.labels.contains(label)
                                && !(at_target_entry
                                    && is_metadata_range_entry(
                                        *label,
                                        exception_regions,
                                        &provenance.setups,
                                    )),
                        ),
                    })
                    .try_fold(0, |size, width| {
                        let size = size + width;
                        if size >= 5 { Err(size) } else { Ok(size) }
                    })
                    .unwrap_or_else(std::convert::identity)
                    <= 4
            } else {
                false
            };
            ExitTargetAnalysis {
                inline_small_exit: target_is_small_exit
                    && !exception_handler_blocks[target.index()]
                    && !target_blocks.iter().any(|block| {
                        graph[*block].items().iter().any(|item| {
                            matches!(item, Item::Instruction(instruction) if instruction.opcode == END_SEND)
                        })
                    }),
                implicit_small_raise: target_is_small_exit
                    && target_chain_end == target_position
                    && target_last.opcode == RAISE_VARARGS
                    && (block_is_bare_raise(graph[target].items(), original_nop_targets)
                        || block_is_local_raise(graph[target].items(), original_nop_targets)),
                copy_conditional_split_return: target_is_small_exit
                    && predecessors[target.index()] > 1
                    && !exception_handler_blocks[target.index()],
                no_location_exit: target_has_no_location
                    && !block_has_fallthrough(graph[target_exit].items())
                    && target_last.no_location_policy.allows_block_inlining(),
                shared_exit: predecessors[target.index()] > 1
                    && ((target_has_no_location
                        && (ends_scope(target_last.opcode) || has_jump_eval_check(target_last)))
                        || target_last.no_location_policy.is_artificial_eval_break()),
            }
        });
        let source_copies_split_return = source_last.no_location_policy.copies_split_return_exit();
        let source_has_fallthrough =
            !ends_scope(source_last.opcode) && !is_unconditional_jump(source_last.opcode);
        let inline_small_exit = !source_has_fallthrough
            && analysis.inline_small_exit
            && (analysis.implicit_small_raise || source_last.jump_policy.inline_small_exit());
        let copy_conditional_split_return = source_has_fallthrough
            && source_copies_split_return
            && analysis.copy_conditional_split_return;
        if inline_small_exit
            || copy_conditional_split_return
            || (!source_has_fallthrough && analysis.no_location_exit)
            || (predecessors[target.index()] > 1
                && (analysis.shared_exit || analysis.no_location_exit))
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
    ordered_origins.sort_unstable_by_key(|(entry, _)| entry.as_u32());
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

/// Recognizes a retained, located SETUP record in its exact original one-slot range.
/// Its exclusion labels can survive after the surrounding exception range disappears.
fn block_is_original_setup_nop(
    graph: &BlockGraph,
    order: &[BasicBlockId],
    positions: &[usize],
    block: BasicBlockId,
    provenance: &ExceptionOwnerProvenance,
) -> bool {
    let mut instructions = graph[block].items().iter().filter_map(|item| match item {
        Item::Instruction(instruction) => Some(instruction),
        Item::Label(_) => None,
    });
    let Some(nop) = instructions.next() else {
        return false;
    };
    if nop.opcode != NOP || nop.location.line < 0 || instructions.next().is_some() {
        return false;
    }
    let position = positions[block.index()];
    let Some(&next) = order.get(position + 1) else {
        return false;
    };
    let setups = represented_setup_nops(graph, &order[position..=position + 1], provenance);
    graph[next]
        .items()
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .any(|item| matches!(item, Item::Label(label) if setups.setups.contains(label)))
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
    setup_nop_provenance: Option<&ExceptionOwnerProvenance>,
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
                        return setup_nop_provenance.is_some_and(|provenance| {
                            block_is_original_setup_nop(graph, order, positions, next, provenance)
                        });
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
    instructions: &[Instruction],
) -> Option<SourceLocation> {
    let [swap, first, middle @ .., last] = instructions else {
        return None;
    };
    let mut removed_swap = *swap;
    removed_swap.opcode = NOP;
    removed_swap.set_operand(Operand::Value(0));
    source
        .iter()
        .copied()
        .eq([removed_swap, *last]
            .into_iter()
            .chain(middle.iter().copied())
            .chain([*first])
            .map(Item::Instruction))
        .then_some(last.location)
}

/// Replays the original store donor only across the exact static-SWAP permutation.
fn comprehension_assignment_exit_location(
    source: &[Item],
    target: &[Item],
    origins: &FxHashMap<Label, (Vec<Label>, Vec<Instruction>)>,
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
    if !matches!(jump.opcode, JUMP_FORWARD | JUMP_BACKWARD) || jump.location.line >= 0 {
        return None;
    }
    let (entries, instructions) = target[..entry_len].iter().find_map(|item| match item {
        Item::Label(exit) => origins.get(exit),
        Item::Instruction(_) => None,
    })?;
    if source.len() != entries.len() + instructions.len() {
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
    static_swap_tail_location(tail, instructions)
}

/// Original normal-try exits threaded through one shared, unlocated branch join.
pub(in crate::assembler) struct ThreadedNormalTryJoins {
    entries: FxHashMap<Label, (Vec<Item>, Vec<Item>)>,
    recoveries: FxHashMap<Label, NamedRecoveryCopy>,
}

struct NamedRecoveryCopy {
    normal_entry: Label,
    prefix: Label,
    split: Label,
    cleanup: Label,
    items: Vec<Item>,
    cleanup_items: Vec<Item>,
    sources: FxHashMap<Label, Vec<Item>>,
}

impl ThreadedNormalTryJoins {
    /// Accepts only the captured source after one threading step and its unchanged target.
    fn matches(&self, graph: &BlockGraph, entry: Label, source: &[Item], target: &[Item]) -> bool {
        self.entries
            .get(&entry)
            .is_some_and(|(expected_source, expected_target)| {
                source == expected_source.as_slice() && target == expected_target.as_slice()
            })
            && self.recoveries.values().any(|recovery| {
                if recovery.normal_entry != entry {
                    return false;
                }
                let (Some(prefix), Some(tail)) = (
                    graph.label_block(recovery.prefix),
                    graph.label_block(recovery.split),
                ) else {
                    return false;
                };
                graph[prefix]
                    .items()
                    .iter()
                    .chain(graph[tail].items())
                    .eq(&recovery.items)
                    && recovery.sources.iter().any(|(label, items)| {
                        graph
                            .label_block(*label)
                            .is_some_and(|source| graph[source].items() == items)
                    })
            })
    }
}

/// Original reversed-store fallthroughs into shared cleanup or loop exits.
pub(in crate::assembler) struct ReversedStoreExits {
    original_labels: FxHashSet<Label>,
    entries: FxHashMap<Label, ReversedStoreExit>,
}

struct ReversedStoreExit {
    swapped_source: Vec<Item>,
    target: Vec<Item>,
    donor: SourceLocation,
}

impl ReversedStoreExits {
    /// Replays only the captured SWAP-to-NOP conversion and complete store exchange.
    fn location(&self, source: &[Item], target: &[Item]) -> Option<SourceLocation> {
        let origin = target
            .iter()
            .take_while(|item| matches!(item, Item::Label(_)))
            .find_map(|item| match item {
                Item::Label(label) => self.entries.get(label),
                Item::Instruction(_) => None,
            })?;
        if target != origin.target.as_slice() || !source.ends_with(&origin.swapped_source) {
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
    static_swap_tail_location(&source[prefix.len()..], &[*swap, *first, *last])
}

fn propagate_exit_block_locations(
    graph: &mut BlockGraph,
    order: &[BasicBlockId],
    analysis: &ExitBlockAnalysis,
    exception_regions: &[ExceptionRegion],
    original_boundaries: &FxHashSet<Label>,
    original_location_absence: &FxHashSet<Label>,
    comprehension_assignment_exits: &FxHashMap<Label, (Vec<Label>, Vec<Instruction>)>,
    preserved_boundaries: &FxHashSet<Label>,
    source_boundaries: &FxHashSet<Label>,
    setups: &[(Label, Option<SourceLocation>)],
    python_version: crate::PythonVersion,
    provenance: &ExceptionOwnerProvenance,
) {
    #[cfg(test)]
    let mut eager = graph.clone();

    propagate_exit_block_locations_inner::<true>(
        graph,
        order,
        analysis,
        exception_regions,
        original_boundaries,
        original_location_absence,
        comprehension_assignment_exits,
        preserved_boundaries,
        source_boundaries,
        setups,
        python_version,
        provenance,
    );

    #[cfg(test)]
    {
        propagate_exit_block_locations_inner::<false>(
            &mut eager,
            order,
            analysis,
            exception_regions,
            original_boundaries,
            original_location_absence,
            comprehension_assignment_exits,
            preserved_boundaries,
            source_boundaries,
            setups,
            python_version,
            provenance,
        );
        assert_eq!(graph.copy_items(), eager.copy_items());
    }
}

/// Resolves only source-less first instructions; later located records are never changed.
fn propagate_exit_block_locations_inner<const SKIP_LOCATED: bool>(
    graph: &mut BlockGraph,
    order: &[BasicBlockId],
    analysis: &ExitBlockAnalysis,
    exception_regions: &[ExceptionRegion],
    original_boundaries: &FxHashSet<Label>,
    original_location_absence: &FxHashSet<Label>,
    comprehension_assignment_exits: &FxHashMap<Label, (Vec<Label>, Vec<Instruction>)>,
    preserved_boundaries: &FxHashSet<Label>,
    source_boundaries: &FxHashSet<Label>,
    setups: &[(Label, Option<SourceLocation>)],
    python_version: crate::PythonVersion,
    provenance: &ExceptionOwnerProvenance,
) {
    for target in order.iter().copied() {
        if analysis.exception_handler_blocks[target.index()] {
            continue;
        }
        if SKIP_LOCATED
            && graph[target]
                .items()
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .is_none_or(|first| first.location.line >= 0)
        {
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
        let literal_cleanup = python_version == crate::PythonVersion::Py315
            && graph[target].items().iter().find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.opcode),
                Item::Label(_) => None,
            }) == Some(POP_TOP);
        let preserve_shared_cleanup = analysis.predecessors[target.index()] > 1
            && is_unlocated_value_cleanup_prefix(graph[target].items(), python_version)
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
                literal_cleanup.then_some(provenance),
                target,
            );
        let preserve_literal_cleanup = literal_cleanup && preserve_shared_cleanup;
        // A shared discard or conditional cleanup join has no unique source. Exit copies
        // acquire their branch's location separately when the continuation can be copied.
        if analysis.predecessors[target.index()] > 1
            && !preserve_literal_cleanup
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
            && ((first.opcode == POP_EXCEPT
                && graph[target].items().iter().any(|item| {
                    matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                }))
                || (first.opcode == NOP && first.fusion_policy.is_converted_pop_block()))
            && first.location.line < 0
            && !first.no_location_policy.preserves_location()
            && let Some(location) = analysis.jump_predecessors[target.index()].and_then(|source| {
                graph[source]
                    .last_instruction()
                    .map(|instruction| instruction.location)
            })
            && let Some(first) = graph.instructions_mut(target).next()
        {
            // A sole jump locates the artificial cleanup entry even when later
            // instructions in the same block already have authored locations.
            first.location = location;
            continue;
        }
        if !graph[target].items().iter().all(|item| {
            !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
        }) {
            continue;
        }
        if preserve_shared_cleanup {
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

type ExitCopyRetargets<'a> = (
    &'a mut Vec<(Instruction, Instruction)>,
    &'a mut Vec<(ExceptionRegion, ExceptionRegion)>,
);

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

        // CPython propagates the SETUP span across the original fallthrough into
        // the protected body before converting SETUP to NOP and compacting it.
        // Resolve this exact seam early so equal-next-line compaction agrees.
        let setup_prefix_locations = order
            .windows(2)
            .filter_map(|pair| {
                let [source, target] = *pair else {
                    unreachable!()
                };
                let (Item::Instruction(marker), source_labels) =
                    self.graph[source].items().split_last()?
                else {
                    return None;
                };
                let [
                    Item::Label(end),
                    Item::Label(setup),
                    Item::Instruction(prefix),
                    ..,
                ] = self.graph[target].items()
                else {
                    return None;
                };
                if predecessors.get(&target).map(|(count, _)| *count) != Some(1)
                    || marker.opcode != NOP
                    || marker.location.line < 0
                    || marker.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
                    || !source_labels
                        .iter()
                        .all(|item| matches!(item, Item::Label(_)))
                    || !self.exception_owner_provenance.setup_nop_ranges.iter().any(
                        |(start, finish)| {
                            finish == end && source_labels.contains(&Item::Label(*start))
                        },
                    )
                    || !self
                        .exception_owner_provenance
                        .setups
                        .contains(&(*setup, Some(marker.location)))
                    || !self
                        .exception_regions
                        .iter()
                        .any(|region| region.start == *setup && !region.is_scope_fallback)
                    || prefix.opcode != LOAD_CONST
                    || prefix.location != SourceLocation::NONE
                    || prefix.no_location_policy.percent_target_location != u8::MAX
                    || prefix.exception_policy() != ExceptionPolicy::default()
                    || marker
                        .depth_after()
                        .zip(prefix.depth_after())
                        .is_none_or(|(before, after)| before.checked_add(1) != Some(after))
                    || [marker, prefix].iter().any(|instruction| {
                        instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                            || !matches!(instruction.operand(), Operand::Value(_))
                    })
                {
                    return None;
                }
                Some((target, marker.location))
            })
            .collect::<Vec<_>>();
        for (target, location) in setup_prefix_locations {
            let prefix = self
                .graph
                .instructions_mut(target)
                .next()
                .expect("certified prefix");
            prefix.location = location;
            prefix.no_location_policy.percent_target_location = 0;
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

    /// Defers synthesized percent literals that begin a sole predecessor's target.
    ///
    /// CPython's early line-number propagation follows both fallthrough and jump edges.
    /// Retain the last located instruction from the sole predecessor before optimization
    /// removes the jump or places an exception handler between the two blocks.
    pub(in crate::assembler) fn defer_synthesized_predecessor_locations(
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
        for (position, source) in order.iter().copied().enumerate() {
            let fallthrough = block_has_fallthrough(self.graph[source].items())
                .then(|| order.get(position + 1).copied())
                .flatten();
            let jump_target = block_jump_target(self.graph[source].items())
                .and_then(|label| self.graph.label_block(label));
            let Some(location) =
                self.graph[source]
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
            let authored_nop_predecessor =
                self.graph[source]
                    .last_instruction()
                    .is_some_and(|instruction| {
                        instruction.opcode == NOP
                            && instruction.location.line >= 0
                            && !instruction.fusion_policy.is_converted_pop_block()
                            && instruction.exception_policy() == ExceptionPolicy::default()
                    });
            for target in fallthrough.into_iter().chain(jump_target) {
                if predecessors[target.index()] != 1 {
                    continue;
                }
                let Some(first) = self.graph.instructions_mut(target).next() else {
                    continue;
                };
                if first.location.line >= 0
                    || first.no_location_policy.percent_target_location != u8::MAX
                {
                    continue;
                }
                if authored_nop_predecessor && fallthrough == Some(target) {
                    // Native line propagation gives the synthesized literal its preceding
                    // authored NOP's span before equal-next-line NOP compaction.
                    first.location = location;
                    first.no_location_policy.percent_target_location = 0;
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
        }
        locations
    }

    /// Applies locations selected before CFG optimization without affecting opcode transforms.
    pub(in crate::assembler) fn resolve_deferred_percent_locations(
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
    ) -> Result<bool, crate::CompileError> {
        if !self.graph.features().has_pop_conditional() {
            return Ok(false);
        }
        let has_branch_markers = self.python_version >= crate::PythonVersion::Py314;
        let mut changed = false;
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
            if has_branch_markers {
                // Code generation already emitted the forward form's marker. CPython
                // instead creates it in the new backedge block at this late stage.
                let marker = self.graph.order()[position + 1..].iter().find_map(|block| {
                    self.graph[*block]
                        .items()
                        .iter()
                        .enumerate()
                        .find_map(|(offset, item)| match item {
                            Item::Instruction(marker) => {
                                Some((*block, ItemPosition::new(*block, offset), marker.opcode))
                            }
                            Item::Label(_) => None,
                        })
                });
                let Some((_marker_block, marker, NOT_TAKEN)) = marker else {
                    return Err(crate::CompileError::Internal(
                        "backward conditional has no fallthrough marker".to_owned(),
                    ));
                };
                // Remove the marker before inserting a successor label can shift its offset.
                self.graph.remove_item(marker);
            }
            changed = true;
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
            let mut items = vec![Item::Label(fresh_entry)];
            // Python 3.12 and 3.13 insert only the backedge, without a branch marker.
            if has_branch_markers {
                let mut marker = Instruction::synthetic(
                    NOT_TAKEN,
                    Operand::Value(0),
                    instruction.location,
                    instruction.depth_after(),
                );
                // The new CPython instruction array starts with null exception owners.
                marker.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
                items.push(Item::Instruction(marker));
            }
            let mut backedge = Instruction::synthetic(
                JUMP_BACKWARD,
                Operand::Backward(target),
                instruction.location,
                instruction.depth_after(),
            );
            backedge.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
            if let Some(native) = self.legacy_native_labels.as_mut()
                && native.is_certified()
            {
                if let Some(argument) = native.target_label(&self.graph, target) {
                    backedge.set_legacy_initial_jump_argument(argument);
                } else {
                    native.invalidate();
                }
            }
            items.push(Item::Instruction(backedge));
            self.preserved_block_boundaries.insert(fresh_entry);
            self.graph.insert_block(position + 1, items);
            position += 2;
        }
        Ok(changed)
    }

    pub(in crate::assembler) fn push_cold_blocks_to_end(
        &mut self,
        mut owner_retargets: Option<&mut Vec<(ExceptionRegion, ExceptionRegion)>>,
    ) -> FxHashSet<Label> {
        if self.exception_regions.is_empty() {
            normalize_unmoved_interruptible_jumps(&mut self.graph);
            return FxHashSet::default();
        }
        // Range-only labels can split our physical graph without starting a
        // native 3.13 instruction array. Keep that distinction when the label's
        // empty block becomes the predecessor of a newly inserted cold bridge.
        let legacy_arrays = (self.python_version == crate::PythonVersion::Py313)
            .then(|| self.legacy_instruction_arrays());
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
            .filter_map(|(position, block)| {
                // 3.12 erases this empty false-condition array before cold relocation.
                (self.python_version != crate::PythonVersion::Py312
                    && loop_anchor_targets.contains(block))
                .then_some(position)
            })
            .collect::<Vec<_>>();
        for position in loop_anchor_positions.into_iter().rev() {
            let anchor = Label::from_u32(self.next_label);
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
            let label = Label::from_u32(self.next_label);
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
                let label = Label::from_u32(self.next_label);
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

        // Erased handlers retain their labels but have lost their handler status.
        // Those labels must not seed cold paths through otherwise empty arrays.
        let mut stack = self
            .exception_regions
            .iter()
            .filter(|region| {
                !self
                    .retired_exception_handler_targets
                    .contains(&region.target)
            })
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

        if self.python_version >= crate::PythonVersion::Py314 {
            for pair in order.windows(2) {
                let [current, next] = *pair else {
                    unreachable!()
                };
                if temperatures[current.index()] != COLD_BLOCK
                    || temperatures[next.index()] != WARM_BLOCK
                {
                    continue;
                }
                let mut instructions =
                    graph[current]
                        .items()
                        .iter()
                        .enumerate()
                        .rev()
                        .filter_map(|(offset, item)| match item {
                            Item::Instruction(instruction) => Some((offset, instruction)),
                            Item::Label(_) => None,
                        });
                let Some((offset, jump)) = instructions.next() else {
                    continue;
                };
                let Some((_, previous)) = instructions.next() else {
                    continue;
                };
                if jump.opcode == JUMP_FORWARD
                    && matches!(jump.operand(), Operand::Forward(target)
                        if graph.label_block(target) == Some(next))
                    && *jump
                        == Instruction::new(
                            JUMP_FORWARD,
                            jump.operand(),
                            jump.location,
                            jump.depth_after(),
                        )
                    && jump.location.line == previous.location.line
                    && graph[next].items().iter().find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.location.line >= 0),
                        Item::Label(_) => None,
                    }) == Some(true)
                {
                    // Exit copying can leave a deferred adjacent branch jump here.
                    // CPython removes it before cold relocation creates an ownerless
                    // bridge with the final instruction's post-SWAP location.
                    graph.remove_item(ItemPosition::new(current, offset));
                }
            }
        }

        let explicit_jumps = (0..order.len().saturating_sub(1))
            .filter(|position| {
                temperatures[order[*position].index()] == COLD_BLOCK
                    && block_has_fallthrough(graph[order[*position]].items())
                    && temperatures[order[*position + 1].index()] == WARM_BLOCK
            })
            .collect::<Vec<_>>();
        if let Some(native) = self.legacy_native_labels.as_mut()
            && native.is_certified()
        {
            let mut next_native = native.next_cfg_label(graph);
            for &position in &explicit_jumps {
                native.allocate_for_block(order[position + 1], &mut next_native);
            }
        }
        for position in explicit_jumps.into_iter().rev() {
            let next = graph.block_at(position + 1);
            let target = if let Some(label) = graph[next].first_label() {
                label
            } else {
                let label = Label::from_u32(self.next_label);
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
            let (mut location, exception_exclusion) = graph[current].last_instruction().map_or(
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
            if graph[current].last_instruction().is_none()
                && let Some(arrays) = &legacy_arrays
                && let Some(array) = arrays.get(&current)
            {
                // Native propagation sees the last instruction before an unused
                // range label in this same array. An actual original empty array
                // still stops inheritance, as does any intervening jump target.
                if let Some(previous) = order[..position]
                    .iter()
                    .rev()
                    .take_while(|block| arrays.get(*block) == Some(array))
                    .find_map(|block| graph[*block].last_instruction())
                {
                    location = previous.location;
                }
            }
            let label = Label::from_u32(self.next_label);
            self.next_label += 1;
            let mut bridge = Instruction::synthetic(
                JUMP_BACKWARD_NO_INTERRUPT,
                Operand::Backward(target),
                location,
                None,
            );
            bridge.exclude_from_exception_table(exception_exclusion);
            if self.python_version == crate::PythonVersion::Py312 {
                // 3.12 creates an ordinary jump with no location and leaves its
                // new block's predecessor count at zero, preventing line inheritance.
                bridge.opcode = JUMP_BACKWARD;
                bridge.location = SourceLocation::NONE;
                bridge.no_location_policy.preserve_location();
                if let Some(native) = self.legacy_native_labels.as_ref()
                    && let Some(argument) = native.block_label(next)
                {
                    bridge.set_legacy_initial_jump_argument(argument);
                }
            }
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
                    let label = Label::from_u32(self.next_label);
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
                    // 3.12's ordinary JUMP retains its eval check after cold relocation.
                    // Explicit SEND backedges already use JUMP_BACKWARD_NO_INTERRUPT.
                    instruction.opcode = if self.python_version == crate::PythonVersion::Py312
                        || instruction.fusion_policy.is_authored_break()
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
        let original_arrays = (self.python_version == crate::PythonVersion::Py313)
            .then(|| self.legacy_instruction_arrays());
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
                    // Legacy early copying also crosses a normal try exit whose
                    // later direct-copy policy retains a NOP. The original folded
                    // path still reaches its successor before line resolution.
                    && (!jump.jump_policy.retains_nop_for(jump.operand())
                        || (self.python_version == crate::PythonVersion::Py313
                            && jump.jump_policy.retained_nop()
                                == super::super::ir::RetainedJumpNop::DirectTarget
                            && self.exception_owner_provenance.normal_try_exits.iter().any(
                                |&(entry, target)| {
                                    jump.operand() == Operand::Forward(target)
                                        && items.iter().any(|item| {
                                            matches!(item, Item::Label(label) if *label == entry)
                                        })
                                },
                            )))
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
            let order = self.graph.order();
            // Exception-range endpoints split legacy POP_BLOCK from its located
            // exit call without starting a new native instruction array. Test
            // the complete array for early-copy eligibility and source lines.
            let start = positions[&block];
            let mut end = start + 1;
            if let Some(arrays) = &original_arrays {
                while end < order.len() && arrays[&order[end]] == arrays[&block] {
                    end += 1;
                }
            }
            let instructions = order[start..end]
                .iter()
                .flat_map(|block| self.graph[*block].items())
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
        // A shared unlocated entry seeds each certified chain. Sharing requires a
        // jump target, and any omitted NOP on its aliases is already a location donor.
        if !self.graph.features().has_jump() {
            return FxHashSet::default();
        }
        let mut saw_instruction = false;
        let mut pending_label = false;
        let mut pending_original_nop = false;
        if !self.graph.iter_items().any(|item| match item {
            Item::Label(label) => {
                pending_label = true;
                pending_original_nop |= self.original_nop_targets.contains(label);
                false
            }
            Item::Instruction(instruction) => {
                let candidate = saw_instruction
                    && pending_label
                    && !pending_original_nop
                    && instruction.location.line < 0
                    && instruction.pre_fold_extra_instructions() == 0;
                saw_instruction = true;
                pending_label = false;
                pending_original_nop = false;
                candidate
            }
        }) {
            return FxHashSet::default();
        }
        if !self.has_shared_unlocated_original_entry() {
            return FxHashSet::default();
        }
        self.capture_original_location_absence_unchecked()
    }

    /// Determines whether any chain can start without building per-instruction state.
    fn has_shared_unlocated_original_entry(&self) -> bool {
        let mut candidate_labels = FxHashMap::<Label, usize>::default();
        let mut candidate_has_predecessor = Vec::new();
        let mut pending_labels = smallvec::SmallVec::<[Label; 4]>::new();
        let mut saw_instruction = false;
        let mut previous_falls_through = false;
        for item in self.graph.iter_items() {
            match item {
                Item::Label(label) => pending_labels.push(*label),
                Item::Instruction(instruction) => {
                    if saw_instruction
                        && !pending_labels.is_empty()
                        && instruction.location.line < 0
                        && instruction.pre_fold_extra_instructions() == 0
                        && pending_labels
                            .iter()
                            .all(|label| !self.original_nop_targets.contains(label))
                    {
                        let candidate = candidate_has_predecessor.len();
                        candidate_has_predecessor.push(previous_falls_through);
                        for label in pending_labels.drain(..) {
                            candidate_labels.insert(label, candidate);
                        }
                    } else {
                        pending_labels.clear();
                    }
                    saw_instruction = true;
                    previous_falls_through = !ends_scope(instruction.opcode)
                        && !is_unconditional_jump(instruction.opcode);
                }
            }
        }
        for item in self.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
                && let Some(&candidate) = candidate_labels.get(&label)
            {
                if candidate_has_predecessor[candidate] {
                    return true;
                }
                candidate_has_predecessor[candidate] = true;
            }
        }
        false
    }

    #[cfg(test)]
    pub(in crate::assembler) fn capture_original_location_absence_unoptimized_for_test(
        &self,
    ) -> FxHashSet<Label> {
        self.capture_original_location_absence_unchecked()
    }

    fn capture_original_location_absence_unchecked(&self) -> FxHashSet<Label> {
        let mut instructions = Vec::new();
        let mut label_entries = vec![None; usize::try_from(self.next_label).unwrap()];
        let mut pending_labels = Vec::new();
        let mut possible_source = Vec::new();
        for (_, block) in self.graph.iter() {
            instructions.reserve(block.items().len());
            possible_source.reserve(block.items().len());
            for item in block.items() {
                match item {
                    Item::Label(label) => pending_labels.push(*label),
                    Item::Instruction(instruction) => {
                        let entry = instructions.len();
                        let mut located = instruction.location.line >= 0
                            || instruction.pre_fold_extra_instructions() > 0;
                        for label in pending_labels.drain(..) {
                            located |= self.original_nop_targets.contains(&label);
                            if label.index() >= label_entries.len() {
                                label_entries.resize(label.index() + 1, None);
                            }
                            label_entries[label.index()] = Some(entry);
                        }
                        instructions.push(instruction);
                        possible_source.push(located);
                    }
                }
            }
        }
        let mut predecessors = vec![LocationPredecessors::None; instructions.len()];
        for (index, instruction) in instructions.iter().enumerate() {
            if index > 0 {
                let previous = instructions[index - 1];
                if !ends_scope(previous.opcode) && !is_unconditional_jump(previous.opcode) {
                    predecessors[index].add(index - 1);
                }
            }
            if let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
                && let Some(target) = label_entries.get(label.index()).copied().flatten()
            {
                predecessors[target].add(index);
            }
        }
        // An entry with no predecessor, a cycle, or hidden original instructions is unknown.
        // Follow sole-predecessor chains transitively; shared unlocated entries cannot inherit
        // a predecessor's line. Memoization bounds the traversal to the original graph size.
        let mut absence = vec![None; instructions.len()];
        let mut visited = vec![usize::MAX; instructions.len()];
        let mut path = Vec::new();
        // Only labeled entries can contribute to the result.
        for start in label_entries.iter().copied().flatten() {
            path.clear();
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
                match predecessors[entry] {
                    LocationPredecessors::One(previous) => entry = previous,
                    LocationPredecessors::None => break false,
                    LocationPredecessors::Multiple => break true,
                }
            };
            absence[start] = Some(absent);
            for &entry in &path {
                absence[entry] = Some(absent);
            }
        }
        label_entries
            .into_iter()
            .enumerate()
            .filter_map(|(label, entry)| {
                entry
                    .filter(|entry| absence[*entry] == Some(true))
                    .map(|_| Label::from_u32(u32::try_from(label).unwrap()))
            })
            .collect()
    }

    /// Captures a sole break through an ordinary try's original normal-exit array.
    pub(in crate::assembler) fn capture_normal_try_break_pop_owners(&mut self) {
        if self.exception_owner_provenance.normal_try_exits.is_empty() {
            return;
        }
        let graph = &self.graph;
        let items = graph.copy_items();
        let labels = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let mut incoming = FxHashMap::<BasicBlockId, Vec<(BasicBlockId, Instruction)>>::default();
        let mut fallthroughs = FxHashSet::default();
        for (position, &block) in graph.order().iter().enumerate() {
            if block_has_fallthrough(graph[block].items())
                && let Some(next) = graph.order().get(position + 1)
            {
                fallthroughs.insert(*next);
            }
            for item in graph[block].items() {
                if let Item::Instruction(jump) = item
                    && let Operand::Forward(target) | Operand::Backward(target) = jump.operand()
                    && let Some(target) = graph.label_block(target)
                {
                    incoming.entry(target).or_default().push((block, *jump));
                }
            }
        }
        let exception_targets = self
            .exception_regions
            .iter()
            .filter_map(|region| graph.label_block(region.target))
            .collect::<FxHashSet<_>>();
        for &(entry, target) in &self.exception_owner_provenance.normal_try_exits {
            let (Some(source), Some(destination)) =
                (graph.label_block(entry), graph.label_block(target))
            else {
                continue;
            };
            let Some((Item::Instruction(jump), source_head)) = graph[source].items().split_last()
            else {
                continue;
            };
            let Some([(donor_block, donor)]) = incoming.get(&source).map(Vec::as_slice) else {
                continue;
            };
            // An unreachable original handler cleanup may still have a syntactic
            // jump to the finalizer. It is not another normal predecessor when it
            // has no entry, incoming jump, physical fallthrough, or exception edge.
            let destination_incoming = incoming
                .get(&destination)
                .into_iter()
                .flatten()
                .copied()
                .filter(|(block, _)| {
                    graph.order().first() == Some(block)
                        || incoming.contains_key(block)
                        || fallthroughs.contains(block)
                        || exception_targets.contains(block)
                })
                .collect::<Vec<_>>();
            if source == destination
                || fallthroughs.contains(&source)
                || fallthroughs.contains(&destination)
                || !matches!(destination_incoming.as_slice(),
                    [(block, original)] if *block == source && original == jump)
                || source_head.is_empty()
                || source_head
                    .iter()
                    .any(|item| !matches!(item, Item::Label(_)))
                || jump.opcode != JUMP_FORWARD
                || jump.operand() != Operand::Forward(target)
                || jump.location.line < 0
                || jump.exception_policy() != ExceptionPolicy::default()
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
                || jump.fusion_policy.is_authored_break()
                || donor.opcode != JUMP_FORWARD
                || !donor.fusion_policy.is_authored_break()
                || donor.location != jump.location
                || donor.depth_after() != jump.depth_after()
                || donor.exception_policy() != ExceptionPolicy::default()
                || donor.pre_fold_extra_instructions() != 1
                || donor.local_policy.hidden_exception_owner_slots() != 0
                || graph[*donor_block].last_instruction() != Some(donor)
            {
                continue;
            }
            let source_labels = source_head
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            let Some(source_position) = graph.order().iter().position(|block| *block == source)
            else {
                continue;
            };
            let Some(handler) = graph
                .order()
                .get(source_position + 1)
                .and_then(|block| graph[*block].first_label())
            else {
                continue;
            };
            // The inner handler ends at this source array. Only an enclosing
            // ordinary region can own the jump after its original POP_BLOCK.
            if !self.exception_regions.iter().any(|region| {
                source_labels.contains(&region.end)
                    && region.target == handler
                    && !region.is_scope_fallback
                    && !region.preserve_lasti
                    && Some(region.depth) == jump.depth_after()
            }) {
                continue;
            }
            let Some((Item::Instruction(exit), prefix)) = graph[destination].items().split_last()
            else {
                continue;
            };
            let Operand::Forward(exit_target) = exit.operand() else {
                continue;
            };
            let marker_at = prefix
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)));
            let Some(marker_at) = marker_at else {
                continue;
            };
            let Item::Instruction(pop) = prefix[marker_at] else {
                unreachable!();
            };
            if prefix.first() != Some(&Item::Label(target))
                || exit.opcode != JUMP_FORWARD
                || exit.exception_policy() != ExceptionPolicy::default()
                || exit.depth_after() != jump.depth_after()
                || exit.pre_fold_extra_instructions() != 0
                || exit.local_policy.hidden_exception_owner_slots() != 0
                || pop.opcode != NOP
                || pop.operand() != Operand::Value(0)
                || pop.location != SourceLocation::NONE
                || !pop.fusion_policy.is_converted_pop_block()
                || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
                || pop.depth_after() != jump.depth_after()
                || pop.pre_fold_extra_instructions() != 0
                || pop.local_policy.hidden_exception_owner_slots() != 0
                || pop.no_location_policy.preserves_location()
                || !matches!(prefix.get(marker_at + 1), Some(Item::Instruction(next))
                    if next.location.line >= 0 && next.location.line != jump.location.line)
                || prefix[marker_at + 1..].iter().any(|item| match item {
                    Item::Label(_) => true,
                    Item::Instruction(record) => {
                        !matches!(record.operand(), Operand::Value(_))
                            || record.opcode.has_jump()
                            || ends_scope(record.opcode)
                            || matches!(
                                record.opcode,
                                NOP | NOT_TAKEN
                                    | crate::target::opcodes::SEND
                                    | crate::target::opcodes::YIELD_VALUE
                                    | crate::target::opcodes::RESUME
                            )
                            || record.exception_policy() != ExceptionPolicy::default()
                            || record.local_policy.hidden_exception_owner_slots() != 0
                            || record.pre_fold_extra_instructions() != 0
                    }
                })
            {
                continue;
            }
            let source_at = labels[&entry] + source_head.len()
                - source_head
                    .iter()
                    .position(|item| *item == Item::Label(entry))
                    .unwrap();
            let mut owners = self.exception_regions.iter().filter(|region| {
                labels
                    .get(&region.start)
                    .is_some_and(|start| *start <= source_at)
                    && labels.get(&region.end).is_some_and(|end| source_at < *end)
            });
            let Some(region) = owners.next() else {
                continue;
            };
            if owners.next().is_some()
                || region.is_scope_fallback
                || region.preserve_lasti
                || Some(region.depth) != jump.depth_after()
                || !prefix[..marker_at].contains(&Item::Label(region.end))
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == target)
            {
                continue;
            }
            self.exception_owner_provenance
                .normal_try_break_pop_owners
                .push(crate::assembler::exception_owners::NormalTryBreakPopOwner {
                    source_labels,
                    handler,
                    entry: target,
                    prefix: prefix.to_vec(),
                    exit: exit_target,
                    location: jump.location,
                    region: *region,
                });
        }
    }

    /// Captures normal exits that bypass a separate branch join before named cleanup copying.
    pub(in crate::assembler) fn capture_threaded_normal_try_joins(&self) -> ThreadedNormalTryJoins {
        let mut entries = FxHashMap::default();
        let mut recoveries = FxHashMap::default();
        if self.exception_owner_provenance.normal_try_exits.is_empty() {
            return ThreadedNormalTryJoins {
                entries,
                recoveries,
            };
        }
        let graph = &self.graph;
        let mut original_predecessors = None;
        for &(entry, join) in &self.exception_owner_provenance.normal_try_exits {
            let (Some(source), Some(join_block)) =
                (graph.label_block(entry), graph.label_block(join))
            else {
                continue;
            };
            let Some((Item::Instruction(jump), head)) = graph[source].items().split_last() else {
                continue;
            };
            let Some((Item::Instruction(join_jump), join_head)) =
                graph[join_block].items().split_last()
            else {
                continue;
            };
            let Operand::Forward(target) = join_jump.operand() else {
                continue;
            };
            let Some(target_block) = graph.label_block(target) else {
                continue;
            };
            if source == join_block
                || target_block == source
                || target_block == join_block
                || jump.opcode != JUMP_FORWARD
                || jump.operand() != Operand::Forward(join)
                || jump.location.line < 0
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
                || head.is_empty()
                || head.iter().any(|item| !matches!(item, Item::Label(_)))
                || join_jump.opcode != JUMP_FORWARD
                || join_jump.location != SourceLocation::NONE
                || join_jump.pre_fold_extra_instructions() != 0
                || join_jump.local_policy.hidden_exception_owner_slots() != 0
                || join_jump.depth_after() != jump.depth_after()
                || !join_jump.jump_policy.allows_threading_target()
                || join_jump.jump_policy.retains_nop_for(join_jump.operand())
                || join_jump.jump_policy.is_interruptible()
                || join_head.is_empty()
                || join_head.iter().any(|item| match item {
                    Item::Instruction(_) => true,
                    Item::Label(label) => {
                        self.original_nop_targets.contains(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_counts
                                .contains_key(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_locations
                                .contains_key(label)
                            || self
                                .exception_owner_provenance
                                .normal_try_exits
                                .iter()
                                .any(|(normal_entry, _)| normal_entry == label)
                            || self
                                .exception_owner_provenance
                                .setups
                                .iter()
                                .any(|(setup, _)| setup == label)
                    }
                })
                || !named_cleanup_has_short_return(graph, target_block)
            {
                continue;
            }
            // Keep one normal exit per original join so recovery certificates cannot
            // depend on the iteration order of independently entered normal exits.
            if self.exception_owner_provenance.normal_try_exits.iter().any(
                |&(other_entry, other_join)| {
                    other_entry != entry && graph.label_block(other_join) == Some(join_block)
                },
            ) {
                continue;
            }
            // Build this index once, only after finding an intact named return cleanup.
            let (reachable, predecessors, positions, explicit_entries, located_sources) =
                original_predecessors.get_or_insert_with(|| {
                    let reachable = graph.reachable_blocks(&self.exception_regions);
                    let mut predecessors = FxHashMap::<Label, Vec<BasicBlockId>>::default();
                    let mut located_sources =
                        FxHashMap::<BasicBlockId, Vec<BasicBlockId>>::default();
                    for &block in graph.order() {
                        if reachable[block.index()]
                            && let Some(incoming) = graph[block].last_instruction()
                            && incoming.opcode == JUMP_FORWARD
                            && incoming.pre_fold_extra_instructions() == 0
                            && incoming.local_policy.hidden_exception_owner_slots() == 0
                            && let Operand::Forward(target) = incoming.operand()
                        {
                            if incoming.location == SourceLocation::NONE {
                                predecessors.entry(target).or_default().push(block);
                            } else if incoming.location.line >= 0
                                && !incoming.jump_policy.retains_nop_for(incoming.operand())
                                && !incoming.jump_policy.is_interruptible()
                                && let Some(target) = graph.label_block(target)
                            {
                                located_sources.entry(target).or_default().push(block);
                            }
                        }
                    }
                    let positions = graph
                        .order()
                        .iter()
                        .enumerate()
                        .map(|(position, &block)| (block, position))
                        .collect::<FxHashMap<_, _>>();
                    let explicit_entries = graph
                        .iter_items()
                        .filter_map(|item| match item {
                            Item::Instruction(instruction) => match instruction.operand() {
                                Operand::Forward(label) | Operand::Backward(label) => {
                                    graph.label_block(label)
                                }
                                _ => None,
                            },
                            Item::Label(_) => None,
                        })
                        .chain(
                            self.exception_regions
                                .iter()
                                .filter_map(|region| graph.label_block(region.target)),
                        )
                        .collect::<FxHashSet<_>>();
                    (
                        reachable,
                        predecessors,
                        positions,
                        explicit_entries,
                        located_sources,
                    )
                });
            if !reachable[source.index()]
                || !predecessors.get(&join).is_some_and(|incoming| {
                    incoming.iter().any(|&other| {
                        other != source
                            && graph[other].last_instruction().is_some_and(|incoming| {
                                incoming.depth_after() == jump.depth_after()
                            })
                    })
                })
            {
                continue;
            }
            // The recovery half must be one original named cleanup split immediately
            // before its unlocated jump. Do not promote an independently entered tail.
            let mut captured_recovery = false;
            let returned = block_jump_target(graph[target_block].items())
                .and_then(|label| graph.label_block(label))
                .expect("captured named cleanup return");
            if join_jump.jump_policy.borrow_empty_block()
                && positions[&join_block] < positions[&target_block]
                && positions[&target_block] < positions[&returned]
            {
                for &other in predecessors.get(&join).into_iter().flatten() {
                    let Some(previous) = positions[&other].checked_sub(1) else {
                        continue;
                    };
                    let prefix = graph.order()[previous];
                    let [Item::Label(split), Item::Instruction(tail)] = graph[other].items() else {
                        continue;
                    };
                    let Some(Item::Label(prefix_label)) = graph[prefix].items().first() else {
                        continue;
                    };
                    let mut cleanup = graph[prefix].items().iter().filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    });
                    let (Some(pop), Some(value), Some(store), Some(delete)) = (
                        cleanup.next(),
                        cleanup.next(),
                        cleanup.next(),
                        cleanup.next(),
                    ) else {
                        continue;
                    };
                    let prefix_head = graph[prefix]
                        .items()
                        .iter()
                        .take_while(|item| matches!(item, Item::Label(_)))
                        .count();
                    if cleanup.next().is_some()
                        || graph[prefix].items()[prefix_head..]
                            .iter()
                            .any(|item| matches!(item, Item::Label(_)))
                        || other == source
                        || positions[&source] >= positions[&prefix]
                        || positions[&other] >= positions[&join_block]
                        || !reachable[prefix.index()]
                        || explicit_entries.contains(&other)
                        || !block_has_fallthrough(graph[prefix].items())
                        || block_jump_target(graph[prefix].items()).is_some()
                        || pop.opcode != POP_EXCEPT
                        || value.opcode != LOAD_CONST
                        || store.opcode != STORE_FAST
                        || delete.opcode != DELETE_FAST
                        || store.operand() != delete.operand()
                        || tail.depth_after() != jump.depth_after()
                        || tail.jump_policy.retains_nop_for(tail.operand())
                        || tail.jump_policy.is_interruptible()
                        || tail.no_location_policy.is_artificial_eval_break()
                        || [pop, value, store, delete].iter().any(|instruction| {
                            instruction.location != SourceLocation::NONE
                                || instruction.no_location_policy.is_artificial_eval_break()
                                || instruction.pre_fold_extra_instructions() != 0
                                || instruction.local_policy.hidden_exception_owner_slots() != 0
                        })
                        || graph[prefix]
                            .items()
                            .iter()
                            .chain(graph[other].items())
                            .any(|item| {
                                let Item::Label(label) = item else {
                                    return false;
                                };
                                self.original_nop_targets.contains(label)
                                    || self
                                        .exception_owner_provenance
                                        .omitted_nop_counts
                                        .contains_key(label)
                                    || self
                                        .exception_owner_provenance
                                        .omitted_nop_locations
                                        .contains_key(label)
                            })
                        || !self.exception_regions.iter().any(|region| {
                            region.end == *split && graph.label_block(region.start) == Some(prefix)
                        })
                    {
                        continue;
                    }
                    let sources = located_sources
                        .get(&prefix)
                        .into_iter()
                        .flatten()
                        .filter_map(|&recovery_source| {
                            if positions[&source] >= positions[&recovery_source]
                                || positions[&recovery_source] >= positions[&prefix]
                            {
                                return None;
                            }
                            let items = graph[recovery_source].items();
                            // Early expansion must be invisible to incoming copies. These
                            // four located work slots survive removal of the terminal jump;
                            // any later exit array therefore exceeds the small-copy limit.
                            let persistent_work = items.iter().filter(|item| {
                                let Item::Instruction(instruction) = item else {
                                    return false;
                                };
                                instruction.location.line >= 0
                                    && instruction.no_location_policy
                                        == super::super::ir::NoLocationPolicy::default()
                                    && instruction.opcode != NOP
                                    && !is_unconditional_jump(instruction.opcode)
                                    && !is_conditional_jump(instruction.opcode)
                                    && !ends_scope(instruction.opcode)
                            }).take(4).count();
                            if persistent_work < 4
                                || items.iter().any(|item| {
                                    matches!(item, Item::Instruction(instruction)
                                        if instruction.no_location_policy.is_artificial_eval_break())
                                })
                            {
                                return None;
                            }
                            let label = graph[recovery_source].first_label()?;
                            Some((label, items.to_vec()))
                        })
                        .collect::<FxHashMap<_, _>>();
                    if sources.is_empty() {
                        continue;
                    }
                    let mut items = graph[prefix].items().to_vec();
                    items.extend_from_slice(graph[other].items());
                    let Some(Item::Instruction(expected_tail)) = items.last_mut() else {
                        unreachable!("captured recovery jump");
                    };
                    expected_tail.retarget(Operand::Forward(target));
                    expected_tail.add_pre_fold_extra_instructions(1);
                    expected_tail
                        .no_location_policy
                        .set_borrow_fallthrough(super::super::ir::BorrowFallthrough::Barrier);
                    recoveries.insert(
                        *prefix_label,
                        NamedRecoveryCopy {
                            normal_entry: entry,
                            prefix: *prefix_label,
                            split: *split,
                            cleanup: target,
                            items,
                            cleanup_items: graph[target_block].items().to_vec(),
                            sources,
                        },
                    );
                    captured_recovery = true;
                }
            }
            if captured_recovery {
                let mut expected_source = graph[source].items().to_vec();
                let Some(Item::Instruction(expected_jump)) = expected_source.last_mut() else {
                    unreachable!("captured normal try jump");
                };
                expected_jump.retarget(Operand::Forward(target));
                expected_jump.add_pre_fold_extra_instructions(1);
                entries.insert(
                    entry,
                    (expected_source, graph[target_block].items().to_vec()),
                );
            }
        }
        ThreadedNormalTryJoins {
            entries,
            recoveries,
        }
    }

    /// Captures the original donor before static swaps reverse a certified store tail.
    pub(in crate::assembler) fn capture_reversed_store_exits(&self) -> ReversedStoreExits {
        let mut entries = FxHashMap::default();
        for pair in self.graph.order().windows(2) {
            let source = self.graph[pair[0]].items();
            let target = self.graph[pair[1]].items();
            let Some(label) = self.graph[pair[1]].first_label() else {
                continue;
            };
            let entry_len = target
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
            let original_handler_exit = matches!(&target[entry_len..],
                [Item::Instruction(pop), Item::Instruction(jump)]
                    if pop.opcode == POP_EXCEPT
                        && pop.location == SourceLocation::NONE
                        && jump.opcode == JUMP_FORWARD
                        && jump.location == SourceLocation::NONE
                        && matches!(jump.operand(), Operand::Forward(_)))
                && self
                    .exception_owner_provenance
                    .source_boundaries
                    .contains(&label);
            // Native resolves this shared backedge before reversing the stores. Keep
            // the original last-store donor only while both arrays survive unchanged.
            let original_loop_exit = matches!(&target[entry_len..],
                [Item::Instruction(jump)]
                    if jump.opcode == JUMP_BACKWARD
                        && jump.location == SourceLocation::NONE
                        && matches!(jump.operand(), Operand::Backward(_)));
            if !original_handler_exit && !original_loop_exit {
                continue;
            }
            if let Some([swap, first, last]) = super::super::constant_pair_store_tail(source) {
                let mut swapped_source = source.to_vec();
                let start = swapped_source.len() - 3;
                let mut removed_swap = swap;
                removed_swap.opcode = NOP;
                removed_swap.set_operand(Operand::Value(0));
                swapped_source[start..].copy_from_slice(&[
                    Item::Instruction(removed_swap),
                    Item::Instruction(last),
                    Item::Instruction(first),
                ]);
                entries.insert(
                    label,
                    ReversedStoreExit {
                        swapped_source,
                        target: target.to_vec(),
                        donor: last.location,
                    },
                );
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
                ReversedStoreExit {
                    swapped_source,
                    target: target.to_vec(),
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
        ReversedStoreExits {
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
        let origins = self.capture_reversed_store_exits();
        let threaded_normal_try_joins = self.capture_threaded_normal_try_joins();
        self.duplicate_exit_blocks_with_store_origins(
            original_location_absence,
            (owner_retargets, &mut Vec::new()),
            &origins,
            &threaded_normal_try_joins,
            &mut [],
            None,
        )
        .expect("test constant-free copy does not allocate constants")
    }

    pub(in crate::assembler) fn duplicate_exit_blocks_with_store_origins(
        &mut self,
        original_location_absence: &FxHashSet<Label>,
        retargets: ExitCopyRetargets<'_>,
        reversed_store_exits: &ReversedStoreExits,
        threaded_normal_try_joins: &ThreadedNormalTryJoins,
        static_swap_store_gaps: &mut [super::swaps::StaticSwapStoreGap],
        mut constants: Option<&mut dyn ConstantTable>,
    ) -> Result<Vec<Label>, crate::CompileError> {
        let (owner_retargets, region_retargets) = retargets;
        let graph = &mut self.graph;
        if graph.is_normalized_jump_free_single_block(&self.preserved_block_boundaries) {
            return Ok(Vec::new());
        }

        graph.normalize_blocks_if_needed(&self.preserved_block_boundaries);
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
            return Ok(Vec::new());
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
                (first.opcode == NOP
                    && first.fusion_policy.is_converted_pop_block()
                    && first.location == SourceLocation::NONE
                    && !first.no_location_policy.preserves_location()
                    && !first.jump_policy.preserves_explicit_nop()
                    && !first.jump_policy.borrow_empty_block()
                    && instructions.next().is_none())
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
                self.python_version,
                &self.exception_owner_provenance,
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
            return Ok(Vec::new());
        }
        let ExitBlockAnalysis {
            positions,
            predecessors,
            jump_predecessors: _,
            conditional_predecessors: _,
            exception_handler_blocks,
            requires_copy: _,
        } = analysis;
        // Bind each captured marker to its complete original prefix before the
        // copier labels implicit block entries. Transport only that new label;
        // final owner replay still requires every item to match literally.
        let conjunction_markers = if self
            .exception_owner_provenance
            .copied_conjunction_break_prefixes
            .is_empty()
        {
            Vec::new()
        } else {
            let original_items = graph.copy_items();
            self.exception_owner_provenance
                .copied_conjunction_break_prefixes
                .iter()
                .enumerate()
                .filter_map(|(index, prefix)| {
                    if !matches!(prefix.last(), Some(Item::Instruction(marker))
                        if marker.opcode == NOT_TAKEN)
                    {
                        return None;
                    }
                    let mut matches = original_items
                        .windows(prefix.len())
                        .enumerate()
                        .filter(|(_, items)| *items == prefix.as_slice());
                    let (start, _) = matches.next()?;
                    matches
                        .next()
                        .is_none()
                        .then_some((index, start + prefix.len() - 1))
                })
                .collect::<Vec<_>>()
        };
        let shared_break_markers = if self
            .exception_owner_provenance
            .copied_shared_break_truth_prefixes
            .is_empty()
        {
            Vec::new()
        } else {
            let original_items = graph.copy_items();
            self.exception_owner_provenance.copied_shared_break_truth_prefixes.iter().enumerate()
                .filter_map(|(index, prefix)| {
                    if !matches!(prefix.last(), Some(Item::Instruction(marker)) if marker.opcode == NOT_TAKEN) {
                        return None;
                    }
                    let mut matches = original_items.windows(prefix.len()).enumerate()
                        .filter(|(_, items)| *items == prefix.as_slice());
                    let (start, _) = matches.next()?;
                    matches.next().is_none().then_some((index, start + prefix.len() - 1))
                }).collect::<Vec<_>>()
        };
        let mut original_position = 0;
        for block in order.iter().copied() {
            let original_len = graph[block].items().len();
            if graph[block].first_label().is_none() {
                let label = Label::from_u32(self.next_label);
                self.next_label += 1;
                for gap in &mut *static_swap_store_gaps {
                    gap.prepend_copy_entry(graph, block, label);
                }
                graph.insert_label(block, 0, label);
                for &(index, marker_position) in &shared_break_markers {
                    if marker_position == original_position {
                        let prefix = &mut self
                            .exception_owner_provenance
                            .copied_shared_break_truth_prefixes[index];
                        prefix.insert(prefix.len() - 1, Item::Label(label));
                    }
                }
                for &(index, marker_position) in &conjunction_markers {
                    if marker_position == original_position {
                        let prefix = &mut self
                            .exception_owner_provenance
                            .copied_conjunction_break_prefixes[index];
                        prefix.insert(prefix.len() - 1, Item::Label(label));
                    }
                }
            }
            original_position += original_len;
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
        // Region endpoints can share a basic block after earlier simplification.
        // Keep their instruction positions as well as their block membership so
        // a nonempty range inside one block survives exit copying.
        let mut region_label_offsets = FxHashMap::default();
        let mut region_instruction_count = 0usize;
        if !self.exception_regions.is_empty() {
            for &block in &order {
                for item in graph[block].items() {
                    match item {
                        Item::Label(label) => {
                            region_label_offsets.insert(*label, region_instruction_count);
                        }
                        Item::Instruction(_) => region_instruction_count += 1,
                    }
                }
            }
        }
        let empty_regions = self
            .exception_regions
            .iter()
            .map(|region| {
                let start = region_label_offsets
                    .get(&region.start)
                    .copied()
                    .unwrap_or(0);
                let end = region_label_offsets
                    .get(&region.end)
                    .copied()
                    .unwrap_or(region_instruction_count);
                start >= end
            })
            .collect::<Vec<_>>();
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
        let order_if = |condition| if condition { order.as_slice() } else { &[] };
        let keyword_order = order_if(!self.exception_owner_provenance.block_boundaries.is_empty());
        let original_keyword_return_seams = keyword_order
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
        let protected_order = order_if(!self.exception_regions.is_empty());
        let original_protected_return_seams = protected_order
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

        // A chained comparison reaches its protected value arm through a separate
        // NOT_TAKEN/jump array. Preserve the complete original arrays before copying
        // the unowned return; only that one earlier predecessor may revisit the copy.
        let comparison_order = order_if(
            !self
                .exception_owner_provenance
                .ordinary_try_setups
                .is_empty(),
        );
        let original_comparison_return_seams = comparison_order
            .iter()
            .filter_map(|&source| {
                let source_position = positions[source.index()];
                let source_items = graph[source].items();
                let [
                    Item::Label(entry),
                    Item::Instruction(load),
                    Item::Instruction(jump),
                ] = source_items
                else {
                    return None;
                };
                let Operand::Forward(return_entry) = jump.operand() else {
                    return None;
                };
                let return_block = graph.label_block(return_entry)?;
                let return_items = graph[return_block].items();
                let [
                    Item::Label(return_label),
                    Item::Label(end),
                    Item::Instruction(ret),
                ] = return_items
                else {
                    return None;
                };
                let predecessor = *order.get(source_position.checked_sub(2)?)?;
                let discard = *order.get(source_position.checked_sub(1)?)?;
                let alternative = *order.get(source_position + 1)?;
                let condition = *order.get(source_position.checked_sub(3)?)?;
                let [
                    Item::Label(_),
                    Item::Instruction(not_taken),
                    Item::Instruction(success),
                ] = graph[predecessor].items()
                else {
                    return None;
                };
                let [
                    Item::Label(_),
                    Item::Instruction(pop),
                    Item::Instruction(failure),
                ] = graph[discard].items()
                else {
                    return None;
                };
                let [Item::Label(alternative_entry), Item::Instruction(value)] =
                    graph[alternative].items()
                else {
                    return None;
                };
                let condition_jump = graph[condition].last_instruction()?;
                let owned_blocks = [condition, predecessor, discard, source, alternative];
                let mut owners = region_memberships
                    .iter()
                    .enumerate()
                    .filter(|(_, membership)| {
                        owned_blocks
                            .iter()
                            .any(|block| membership.contains(&positions[block.index()]))
                    });
                let (region_index, membership) = owners.next()?;
                let region = self.exception_regions[region_index];
                (owners.next().is_none()
                    && owned_blocks
                        .iter()
                        .all(|block| membership.contains(&positions[block.index()]))
                    && region_memberships
                        .iter()
                        .all(|membership| !membership.contains(&positions[return_block.index()]))
                    && !region.is_scope_fallback
                    && region.depth == 0
                    && !region.preserve_lasti
                    && self
                        .exception_owner_provenance
                        .ordinary_try_setups
                        .get(&region.start)
                        == Some(&region.target)
                    && region.end == *end
                    && positions[return_block.index()] == source_position + 2
                    && *return_label == return_entry
                    && predecessors[source.index()] == 1
                    && predecessors[predecessor.index()] == 1
                    && predecessors[return_block.index()] == 2
                    && [predecessor, discard, source, alternative]
                        .iter()
                        .all(|block| !exception_boundary_blocks[block.index()])
                    && self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(entry)
                    && jump.opcode == JUMP_FORWARD
                    && jump.location == SourceLocation::NONE
                    && jump.no_location_policy.copies_split_return_exit()
                    && !jump.jump_policy.retains_nop_for(jump.operand())
                    && !has_jump_eval_check(jump)
                    && load.opcode == LOAD_FAST
                    && load.depth_after() == Some(1)
                    && load.location.line >= 0
                    && matches!(load.operand(), Operand::Value(_))
                    && ret.opcode == RETURN_VALUE
                    && ret.depth_after() == Some(0)
                    && ret.location.line == load.location.line
                    && ret.operand() == Operand::Value(0)
                    && not_taken.opcode == NOT_TAKEN
                    && not_taken.depth_after() == Some(0)
                    && success.opcode == JUMP_FORWARD
                    && success.operand() == Operand::Forward(*entry)
                    && success.jump_policy.retains_nop_for(success.operand())
                    && !has_jump_eval_check(success)
                    && pop.opcode == POP_TOP
                    && failure.opcode == JUMP_FORWARD
                    && failure.operand() == Operand::Forward(*alternative_entry)
                    && condition_jump.opcode == POP_JUMP_IF_FALSE
                    && condition_jump.operand() == Operand::Forward(*alternative_entry)
                    && condition_jump.location == not_taken.location
                    && success.location == not_taken.location
                    && pop.location == not_taken.location
                    && failure.location == not_taken.location
                    && not_taken.location.line == load.location.line
                    && value.depth_after() == Some(1)
                    && matches!(value.operand(), Operand::Value(_))
                    && block_has_fallthrough(graph[alternative].items())
                    && [load, jump, ret, not_taken, success, pop, failure, value]
                        .iter()
                        .all(|instruction| {
                            instruction.pre_fold_extra_instructions() == 0
                                && instruction.local_policy.hidden_exception_owner_slots() == 0
                                && instruction.exception_policy() == ExceptionPolicy::default()
                        }))
                .then(|| {
                    (
                        source,
                        (
                            predecessor,
                            return_block,
                            region_index,
                            owned_blocks
                                .into_iter()
                                .chain([return_block])
                                .map(|block| (block, graph[block].items().to_vec()))
                                .collect::<Vec<_>>(),
                        ),
                    )
                })
            })
            .collect::<FxHashMap<_, _>>();
        let mut comparison_return_revisits =
            FxHashMap::<_, (BasicBlockId, Vec<Item>, Vec<Item>, usize, (Label, Label))>::default();

        // Keep locations already present on original return records distinct from
        // locations inherited by synthetic cleanup copies during this pass.
        let mut original_return_locations = FxHashMap::default();
        let mut return_labels = Vec::new();
        // An original located return only prevents inheritance across an exception split.
        // Without such a split, no copied instruction can consult these locations.
        let return_order = if exception_split_labels.is_empty() {
            &[]
        } else {
            order.as_slice()
        };
        for item in return_order.iter().flat_map(|block| graph[*block].items()) {
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
            if predecessors[block.index()] <= 1 {
                continue;
            }
            // The 3.15 iterator cleanup can exceed the small-copy limit. Classify
            // its complete original array, as the early copy analysis does, even
            // when an outer try splits the cleanup from RETURN_VALUE.
            let exit_blocks = if self.python_version == crate::PythonVersion::Py315 {
                metadata_split_exit_chain(
                    graph,
                    &order,
                    positions[block.index()],
                    &predecessors,
                    &exception_split_labels,
                )
            } else {
                std::slice::from_ref(&block)
            };
            let unlocated_scope_exit = graph[*exit_blocks.last().expect("nonempty exit array")]
                .last_instruction()
                .is_some_and(|instruction| {
                    ends_scope(instruction.opcode) || has_jump_eval_check(instruction)
                }) && exit_blocks.iter().all(|block| {
                graph[*block].items().iter().all(|item| {
                    !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                })
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
                unlocated_scope_exit || artificial_eval_break;
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
        let shared_match_order = order_if(
            generic_original_arrays
                && !self
                    .exception_owner_provenance
                    .original_match_join_boundaries
                    .is_empty(),
        );
        let shared_match_cleanup_heads = shared_match_order
            .iter()
            .copied()
            .filter_map(|source| {
                if predecessors[source.index()] <= 1 {
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
        let mut jump_sources = vec![SmallVec::<[BasicBlockId; 2]>::new(); positions.len()];
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
        // This token is consumed on the immediately following source visit only.
        let mut recovery_revisit = None::<(BasicBlockId, BasicBlockId, Vec<Item>, Vec<Item>)>;
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
            let recovery_visit = recovery_revisit.take();
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
            let completes_named_recovery = recovery_visit.is_some_and(
                |(expected_source, expected_target, source_items, target_items)| {
                    source == expected_source
                        && target == expected_target
                        && graph[source].items() == source_items
                        && graph[target].items() == target_items
                },
            );
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
            let target_blocks = metadata_split_exit_chain(
                graph,
                &order,
                target_position,
                &predecessors,
                &exception_split_labels,
            );
            let target_chain_end = target_position + target_blocks.len() - 1;
            let target_exit = *target_blocks.last().expect("target chain is nonempty");
            let target_terminal_opcode = target_blocks
                .iter()
                .rev()
                .flat_map(|block| graph[*block].items().iter().rev())
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                });
            let target_has_no_location = target_blocks.iter().all(|block| {
                graph[*block].items().iter().all(|item| {
                    !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
                })
            });
            // Only scope exits, wholly unlocated targets, or already-classified shared exits
            // can produce a copy. The earlier loop-discard repair is wholly unlocated too.
            // Scheduled copies still run every certification assertion below.
            if !planned_copy
                && !target_terminal_opcode.is_some_and(ends_scope)
                && !target_has_no_location
                && !exit_without_unique_predecessor[target.index()]
            {
                continue;
            }
            let represented_setups =
                represented_setup_nops(graph, target_blocks, &self.exception_owner_provenance);
            let mut at_target_entry = true;
            // Ordinary checks only distinguish widths up to four. Scheduled copies must
            // retain their exact width so their original-array assertion remains meaningful.
            let size_limit = if planned_copy { usize::MAX } else { 5 };
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
                            && !represented_setups.labels.contains(label)
                            && !(at_target_entry
                                && is_metadata_range_entry(
                                    *label,
                                    &self.exception_regions,
                                    &self.exception_owner_provenance.setups,
                                )),
                    ),
                })
                .try_fold(0, |size, width| {
                    let size = size + width;
                    if size >= size_limit { Err(size) } else { Ok(size) }
                })
                .unwrap_or_else(std::convert::identity);
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
                && target_terminal_opcode == Some(RAISE_VARARGS)
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
            // RAISE_VARARGS also exits the scope. Append unprotected copies now so
            // another small-exit round can see the expanded predecessor, just as
            // CPython's basicblock_inline_small_or_no_lineno_blocks does.
            let inline_unprotected_raise = inline_small_exit
                && target_terminal_opcode == Some(RAISE_VARARGS)
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
            let shared_unnamed_cleanup_source = !planned_copy
                && inline_no_location_block
                && target_chain_end == target_position
                && source_position < target_position
                && !self.shared_jump_copy_location_blocks.is_empty()
                && is_shared_threaded_handler_jump(
                    graph[source].items(),
                    &self.shared_jump_copy_location_blocks,
                    &self.original_nop_targets,
                    &self.exception_owner_provenance,
                )
                && unnamed_cleanup_has_short_return(
                    graph,
                    target,
                    &self
                        .exception_owner_provenance
                        .original_handler_cleanup_join_exits,
                );
            if !planned_copy
                && inline_no_location_block
                && target_chain_end == target_position
                && graph[source].last_instruction().is_some_and(|jump| {
                    jump.opcode == JUMP_FORWARD
                        // A recorded shared jump waits for its unnamed cleanup's return.
                        // Other unlocated cleanup continuations retain their copy order.
                        && (jump.location.line >= 0 || shared_unnamed_cleanup_source)
                        // Retained authored line slots keep their original copy order.
                        && !jump.jump_policy.retains_nop_for(jump.operand())
                        && jump.pre_fold_extra_instructions() > 0
                        && jump.local_policy.hidden_exception_owner_slots() == 0
                })
                && ((named_cleanup_has_short_return(graph, target)
                    && !completes_named_recovery
                    // Direct ordinary try exits retain their separate POP_BLOCK/copy order.
                    // A certified original branch join still postpones the normal try edge
                    // when threading has bypassed that join on its way to this cleanup.
                    && !self
                        .exception_owner_provenance
                        .normal_try_exits
                        .iter()
                        .any(|(entry, _)| {
                            graph.label_block(*entry) == Some(source)
                                && !threaded_normal_try_joins.matches(
                                    graph,
                                    *entry,
                                    graph[source].items(),
                                    graph[target].items(),
                                )
                        }))
                    || (graph[source]
                        .last_instruction()
                        .is_some_and(|jump| jump.pre_fold_extra_instructions() == 1)
                        && unnamed_cleanup_has_short_return(
                            graph,
                            target,
                            &self
                                .exception_owner_provenance
                                .original_handler_cleanup_join_exits,
                        )))
            {
                // Threading bypassed an original join before this cleanup. Its return is
                // appended before that earlier edge can copy the expanded named cleanup.
                // Direct predecessors still copy here; the existing queue revisits this
                // edge after the cleanup's own return copy and checks its new size/location.
                // Unnamed cleanup also retains its original POP_BLOCK, so its expanded
                // POP_BLOCK/POP_EXCEPT/replaced-jump/value/return array has at least five slots.
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
            // A two-cell iterator return split at an outer POP_BLOCK is cleanup,
            // not a large footer whose copy order follows cold-block relocation.
            let split_iterator_return = self.python_version == crate::PythonVersion::Py315
                && target_chain_end > target_position
                && target_has_no_location
                && target_blocks
                    .iter()
                    .flat_map(|block| graph[*block].items())
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => {
                            Some((instruction.opcode, instruction.operand()))
                        }
                        Item::Label(_) => None,
                    })
                    .eq([
                        (SWAP, Operand::Value(3)),
                        (POP_TOP, Operand::Value(0)),
                        (POP_TOP, Operand::Value(0)),
                        (RETURN_VALUE, Operand::Value(0)),
                    ]);
            if exit_without_unique_predecessor[target.index()]
                && !self.exception_regions.is_empty()
                && !split_iterator_return
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
                // Without exception regions there are no cold blocks to reorder, so ordinary
                // class predecessors retain native copying order. Exception cleanup exits are
                // also copied before relocation, even when clearing a name makes them large.
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
                        Item::Instruction(instruction)
                            if instruction.fusion_policy.location_from_next()
                                && instruction.location == SourceLocation::NONE
                                && instruction.no_location_policy.preserves_location() =>
                        {
                            // An explicitly unlocated NEXT_LOCATION pop stops the earlier
                            // pattern from donating to an erased body's copied loop exit.
                            Some(SourceLocation::NONE)
                        }
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
            let comparison_return_seam = original_comparison_return_seams.get(&source).filter(
                |(_, original_return, _, arrays)| {
                    inline_small_exit
                        && !planned_copy
                        && target_pre_fusion_size == 1
                        && target_blocks == [*original_return]
                        && target == *original_return
                        && arrays
                            .iter()
                            .all(|(block, items)| graph[*block].items() == items)
                },
            );
            let comparison_return_revisit = comparison_return_revisits.get(&source).filter(
                |(original_target, source_items, target_items, region_index, hole)| {
                    inline_small_exit
                        && !planned_copy
                        && target_chain_end == target_position
                        && target == *original_target
                        && graph[source].items() == source_items
                        && graph[target].items() == target_items
                        && copied_region_exclusions[*region_index].contains(hole)
                },
            );
            let recovery_copy = (!planned_copy
                && inline_no_location_block
                && !source_has_fallthrough
                && source_location.line >= 0)
                .then(|| {
                    graph[target].items().iter().find_map(|item| {
                        let Item::Label(label) = item else {
                            return None;
                        };
                        let recovery = threaded_normal_try_joins.recoveries.get(label)?;
                        let cleanup = graph.label_block(recovery.cleanup)?;
                        let source_label = graph[source].first_label()?;
                        let expected_source = recovery.sources.get(&source_label)?;
                        (graph[source].items() == expected_source
                            && target_blocks
                                .iter()
                                .flat_map(|block| graph[*block].items())
                                .eq(&recovery.items)
                            && graph[cleanup].items() == recovery.cleanup_items)
                            .then_some((recovery, cleanup))
                    })
                })
                .flatten();
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
            if inline_small_exit && let Some(constants) = constants.as_deref_mut() {
                // Native small-exit copying precedes arithmetic folding. Other
                // copies reuse already allocated constants; code objects always do.
                let mut remap = crate::code_object::NonReflexiveRemap::default();
                for item in &mut copied {
                    if let Item::Instruction(instruction) = item
                        && instruction.opcode == LOAD_CONST
                        && let Operand::Value(index) = instruction.operand()
                    {
                        let index =
                            constants.reallocate_non_reflexive_constant(index, &mut remap)?;
                        instruction.set_operand(Operand::Value(index));
                    }
                }
            }
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
                        // Retained Pass NOPs share their tag with interruptible
                        // jumps. The original match-success certificate still
                        // proves this authored edge was JUMP before copying.
                        || (instruction.opcode == JUMP_FORWARD
                            && instruction.jump_policy.is_original_match_success()
                            && instruction.exception_policy() == ExceptionPolicy::default())
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
            // Threading can bypass the empty nested join after an original pass
            // array reached by a conditional edge. A populated fallthrough instead
            // keeps borrowing into the copied return after its donated line NOP.
            let copied_return_crosses_empty_join = preserve_threaded_join_nop
                && inline_unprotected_return
                && super::normal_try_copies::original_local_return(graph[target].items())
                && self.exception_regions.is_empty()
                && remaining_predecessors[source.index()] == 1
                && jump_sources[source.index()].iter().any(|predecessor| {
                    !drop_blocks[predecessor.index()]
                        && graph[*predecessor].last_instruction().is_some_and(|jump| {
                            is_conditional_jump(jump.opcode)
                                && matches!(jump.operand(), Operand::Forward(label)
                                    if graph.label_block(label) == Some(source))
                        })
                })
                && graph[source]
                    .items()
                    .iter()
                    .filter(|item| {
                        matches!(item, Item::Label(label)
                            if self.exception_owner_provenance.block_boundaries.contains(label))
                    })
                    .count()
                    == 2
                && graph[source].last_instruction().is_some_and(|jump| {
                    // Both direct and threaded copies need the original NOP
                    // array. A donated conditional-line slot alone also describes
                    // populated paths whose copied return remains reachable.
                    graph[source].items().iter().any(|item| {
                        matches!(item, Item::Label(label)
                            if self.original_nop_targets.contains(label))
                    }) && ((jump.pre_fold_extra_instructions() == 1
                        && !jump.jump_policy.retains_nop_for(jump.operand()))
                        || (jump.pre_fold_extra_instructions() == 0
                            && jump.jump_policy.retained_nop()
                                == super::super::ir::RetainedJumpNop::DirectTarget))
                        && jump.local_policy.hidden_exception_owner_slots() == 0
                        && !jump.fusion_policy.is_authored_break()
                        && !jump.jump_policy.borrow_empty_block()
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
            let copied_label = Label::from_u32(self.next_label);
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
                        *label = Label::from_u32(self.next_label);
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
                let copied_end = Label::from_u32(self.next_label);
                self.next_label += 1;
                copied.push(Item::Label(copied_end));
                copied_labels.insert(original, copied_end);
            }
            // Renaming a region-only re-entry must not create an original unwind slot.
            let reentries = &mut self.exception_owner_provenance.return_region_reentries;
            let mut ordered_copied_labels = copied_labels.iter().collect::<Vec<_>>();
            ordered_copied_labels.sort_unstable_by_key(|(original, _)| original.as_u32());
            for (original, replacement) in ordered_copied_labels {
                if reentries.contains(original) {
                    reentries.insert(*replacement);
                }
            }
            // Alias labels at one instruction boundary do not reopen an excluded owner.
            // Compare copied holes by instruction position so a second copy can retain a
            // complete exclusion even when its region starts before the hole's first alias.
            let mut copied_instruction_positions = FxHashMap::default();
            if !inherited_conditional_exclusions.is_empty() {
                let mut position = 0;
                for item in &copied {
                    match item {
                        Item::Label(label) => {
                            copied_instruction_positions.insert(*label, position);
                        }
                        Item::Instruction(_) => position += 1,
                    }
                }
            }
            // An exception split can end inside the copied cleanup even when its region began in
            // the predecessor's handler body. Preserve that leading intersection in the copy;
            // inner exclusions still divide the successive POP_EXCEPT instructions below.
            copied_exception_regions.extend(self.exception_regions.iter().enumerate().filter_map(
                |(index, region)| {
                    // Empty or inverted ranges protect no instructions. In particular,
                    // a copied start alias must not resurrect such a region across the
                    // copied cleanup when its original end lies outside the array.
                    if empty_regions[index] {
                        return None;
                    }
                    let start = copied_labels.get(&region.start).copied().or_else(|| {
                        (target_chain_end > target_position
                            && region_memberships[index].contains(&positions[target.index()]))
                        .then_some(copied_label)
                    })?;
                    let end = *copied_labels.get(&region.end)?;
                    if inherited_conditional_exclusions.iter().any(
                        |(origin, hole_start, hole_end)| {
                            *origin == index
                                && matches!(
                                    (
                                        copied_instruction_positions.get(hole_start),
                                        copied_instruction_positions.get(&start),
                                        copied_instruction_positions.get(&end),
                                        copied_instruction_positions.get(hole_end),
                                    ),
                                    (Some(hole_start), Some(start), Some(end), Some(hole_end))
                                        if hole_start <= start && end <= hole_end
                                )
                        },
                    ) {
                        return None;
                    }
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
                    let copied_end = Label::from_u32(self.next_label);
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
                    original_join_marker_donors.push((
                        source,
                        copied_label,
                        marker,
                        location,
                        true,
                    ));
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
                original_join_marker_donors.push((source, copied_label, marker, location, false));
                graph.insert_item(
                    ItemPosition::new(source, position),
                    Item::Instruction(marker),
                );
            } else if preserve_inlined_jump_nop {
                let position = copied
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))
                    .unwrap_or(copied.len());
                if self.python_version >= crate::PythonVersion::Py314
                    && inline_small_exit
                    && !planned_copy
                    && target_pre_fusion_size == 4
                    && target_chain_end == target_position
                    && inline_unprotected_return
                    && let Some(predecessor) = source_position
                        .checked_sub(1)
                        .and_then(|position| order.get(position))
                        .and_then(|block| graph[*block].last_instruction())
                    && let Some((donor, original)) = copied_match_guard_subject_donor(
                        graph[source].items(),
                        graph[target].items(),
                        predecessor,
                        &self
                            .exception_owner_provenance
                            .original_comparison_fallthroughs,
                    )
                    && let [
                        Item::Instruction(pop),
                        Item::Instruction(global),
                        Item::Instruction(attribute),
                        Item::Instruction(returned),
                    ] = &mut copied[position..]
                    && [*pop, *global, *attribute, *returned] == original
                {
                    // Native NOP cleanup transfers the replaced bridge's guard span to
                    // POP_TOP(NEXT_LOCATION). For this intact private copy the intervening
                    // passes cannot observe the projection: no internal edges, local loads,
                    // folds or owner state exist. Keep the physical NOP below, so repeated
                    // exit copying still sees all five original records. The source POP
                    // remains NEXT_LOCATION and resolves against its own return expression.
                    pop.location = donor;
                    pop.fusion_policy.clear_location_from_next();
                }
                let mut marker =
                    Instruction::synthetic(NOP, Operand::Value(0), source_location, None);
                if copied_return_crosses_empty_join {
                    marker
                        .no_location_policy
                        .set_borrow_fallthrough(super::super::ir::BorrowFallthrough::Barrier);
                }
                copied.insert(position, Item::Instruction(marker));
            }
            let inline_reraising_attribute_store = matches!(
                self.python_version,
                crate::PythonVersion::Py314 | crate::PythonVersion::Py315
            ) && inline_small_exit
                && target_terminal_opcode == Some(RERAISE)
                && target_pre_fusion_size == 1
                && target_chain_end == target_position
                && graph[target].last_instruction().is_some_and(|instruction| {
                    instruction.location.line < 0 && instruction.operand() == Operand::Value(0)
                })
                && is_four_slot_attribute_store_jump(graph[source].items());
            let inline_reraising_iterator_break = self.python_version
                == crate::PythonVersion::Py315
                && inline_small_exit
                && target_terminal_opcode == Some(RERAISE)
                && target_pre_fusion_size == 1
                && target_chain_end == target_position
                && graph[source]
                    .last_instruction()
                    .is_some_and(|jump| jump.fusion_policy.is_authored_break())
                && graph[source]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.opcode),
                        Item::Label(_) => None,
                    })
                    .eq([NOP, POP_TOP, POP_TOP, JUMP_FORWARD]);
            if (inline_unprotected_return
                || inline_unprotected_raise
                || inline_reraising_iterator_break
                || inline_reraising_attribute_store)
                && !planned_copy
                && let Some(instruction) = copied.iter_mut().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
            {
                // CPython retains the replaced jump and any jumps bypassed by our earlier
                // threading as NOPs until after the repeated small-exit copy pass. A 3.15
                // iterator break's marker and two POPs, or a located three-operation
                // attribute store, then make its copied RERAISE five original slots,
                // preventing another small-exit copy.
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
            let copied_return_locations = if original_return_locations.is_empty() {
                FxHashMap::default()
            } else {
                copied_labels
                    .iter()
                    .filter_map(|(original, copied)| {
                        original_return_locations
                            .get(original)
                            .map(|location| (*copied, *location))
                    })
                    .collect::<FxHashMap<_, _>>()
            };
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
            let inline_exit_copy =
                inline_small_exit || inline_no_location_block || split_iterator_return;
            // A large shared exit can also be copied after an unconditional source.
            // Its original target owners survive even though this is not inlining.
            if inline_exit_copy || !source_has_fallthrough {
                if planned_copy
                    || (inline_no_location_block && target_chain_end > target_position)
                    || comparison_return_revisit.is_some()
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
                    if inline_exit_copy
                        && !planned_copy
                        && let Some(position) = trailing_nop_position
                    {
                        whole_copy_exclusion_start = Label::from_u32(self.next_label);
                        self.next_label += 1;
                        graph.insert_label(source, position, whole_copy_exclusion_start);
                    }
                    let copied_end = Label::from_u32(self.next_label);
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
                    let copied_label = Label::from_u32(self.next_label);
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
            let inline_comparison_return =
                comparison_return_seam.is_some_and(|(_, original_return, region_index, arrays)| {
                    let [
                        Item::Label(entry),
                        Item::Label(end),
                        Item::Instruction(ret),
                        Item::Label(exclusion_end),
                    ] = copied.as_slice()
                    else {
                        return false;
                    };
                    let [
                        Item::Label(original_entry),
                        Item::Label(original_end),
                        Item::Instruction(original_ret),
                    ] = graph[*original_return].items()
                    else {
                        return false;
                    };
                    let original_source =
                        arrays.iter().find(|(block, _)| *block == source).unwrap();
                    !preserve_inlined_jump_nop
                        && graph[source].items()
                            == &original_source.1[..original_source.1.len() - 1]
                        && *entry == copied_label
                        && copied_labels.get(original_entry) == Some(entry)
                        && copied_labels.get(original_end) == Some(end)
                        && *entry != *end
                        && *end != *exclusion_end
                        && *entry != *exclusion_end
                        && *ret == original_ret.for_exit_copy()
                        && protected_fragment_origins.is_empty()
                        && copied_region_exclusions[*region_index]
                            .contains(&(*entry, *exclusion_end))
                });
            if inline_comparison_return {
                let (predecessor, _, region_index, arrays) = comparison_return_seam.unwrap();
                let original_predecessor = arrays
                    .iter()
                    .find(|(block, _)| block == predecessor)
                    .unwrap();
                let Item::Instruction(ret) = &mut copied[2] else {
                    unreachable!("certified comparison return");
                };
                // The removed original jump is still a slot during repeated exit copying.
                ret.add_pre_fold_extra_instructions(1);
                let Item::Label(exclusion_end) = copied[3] else {
                    unreachable!("certified return exclusion end");
                };
                let mut expected_target = graph[source].items().to_vec();
                expected_target.extend_from_slice(&copied);
                comparison_return_revisits.insert(
                    *predecessor,
                    (
                        source,
                        original_predecessor.1.clone(),
                        expected_target,
                        *region_index,
                        (copied_label, exclusion_end),
                    ),
                );
                // A later copy can drop the arm but its exclusion endpoints still close
                // the original owner. Retain those labels just like original region ends.
                exception_boundary_blocks[source.index()] = true;
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
                || inline_unprotected_raise
                || inline_comparison_return
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
                    if let Some((recovery, cleanup)) = recovery_copy
                        && copied_handler_labels.contains_key(&recovery.split)
                        && graph[cleanup].items() == recovery.cleanup_items
                        && block_jump_target(graph[source].items()) == Some(recovery.cleanup)
                    {
                        // The erased join captured this outer cleanup before its return
                        // grew. Reproduce that saved array for this original recovery
                        // source only; the normal exit still sees the later grown target.
                        recovery_revisit = Some((
                            source,
                            cleanup,
                            graph[source].items().to_vec(),
                            graph[cleanup].items().to_vec(),
                        ));
                        sources.insert(source_cursor, source);
                    } else {
                        sources.push(source);
                    }
                }
                // A completed scope exit also makes earlier predecessors eligible for a
                // small exit copy. Revisit them before conditional duplication.
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
        for (source, successor, mut expected, location, store_donor) in original_join_marker_donors
        {
            // A surviving shared original cannot inherit this fallthrough store's line.
            if store_donor && remaining_predecessors[source.index()] != 1 {
                continue;
            }
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
                let rebuilt_start = self.exception_regions.len();
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
                // Record the exact source of every split before copied ranges are appended.
                region_retargets.extend(
                    self.exception_regions[rebuilt_start..]
                        .iter()
                        .copied()
                        .map(|rebuilt| (region, rebuilt)),
                );
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

        #[cfg(test)]
        let mut eager_location_graph = graph.clone();
        let location_targets = order.iter().copied().map(|target| (true, target));
        #[cfg(test)]
        let location_targets =
            location_targets.chain(order.iter().copied().map(|target| (false, target)));
        for (skip_located, target) in location_targets {
            #[cfg(test)]
            let graph = if skip_located {
                &mut *graph
            } else {
                &mut eager_location_graph
            };
            if exception_handler_blocks[target.index()] {
                continue;
            }
            if skip_located
                && graph[target]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })
                    .is_some_and(|first| {
                        first.location.line >= 0
                            && !planned_blocks.contains(&target)
                            && !shared_match_cleanup_copies.contains_key(&target)
                            && (remaining_predecessors[target.index()] != 1
                                || predecessors[target.index()] <= 1
                                || (!first.no_location_policy.copy_inherits_shared_predecessor()
                                    && (!has_jump_eval_check(first)
                                        || self.shared_jump_copy_location_blocks.is_empty())))
                    })
            {
                // Located heads can only change through the explicitly tracked repairs above.
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
            let literal_cleanup = self.python_version == crate::PythonVersion::Py315
                && graph[target].items().iter().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                }) == Some(POP_TOP);
            let preserve_shared_cleanup = remaining_predecessors[target.index()] > 1
                && is_unlocated_value_cleanup_prefix(graph[target].items(), self.python_version)
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
                    literal_cleanup.then_some(&self.exception_owner_provenance),
                    target,
                );
            let preserve_literal_cleanup = literal_cleanup && preserve_shared_cleanup;
            if planned_blocks.contains(&target)
                && remaining_predecessors[target.index()] > 1
                && !preserve_literal_cleanup
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
                && !preserve_literal_cleanup
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
                if remaining_predecessors[target.index()] == 1
                    && let Some(Item::Instruction(first)) = graph[target]
                        .items()
                        .iter()
                        .find(|item| matches!(item, Item::Instruction(_)))
                    && first.opcode == POP_EXCEPT
                    && first.location.line < 0
                    && !first.no_location_policy.preserves_location()
                    && let Some(location) = jump_sources[target.index()]
                        .iter()
                        .copied()
                        .filter(|source| !drop_blocks[source.index()])
                        .find_map(|source| {
                            block_jump_target(graph[source].items())
                                .and_then(|label| graph.label_block(label))
                                .filter(|block| *block == target)
                                .and_then(|_| graph[source].last_instruction())
                                .filter(|instruction| instruction.location.line >= 0)
                                .map(|instruction| instruction.location)
                        })
                    && let Some(first) = graph.instructions_mut(target).next()
                {
                    // Only the artificial prefix needs a line from its sole live jump.
                    // Keep copied arrays on their existing internal-entry propagation path.
                    first.location = location;
                }
                continue;
            }
            if preserve_shared_cleanup {
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
                    && let Some(location) = reversed_store_exits
                        .location(graph[source].items(), graph[target].items())
                        .or_else(|| fused_store_tail_location(graph[source].items()))
                {
                    // Captured store tails keep their original last-store donor across
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
        #[cfg(test)]
        assert_eq!(graph.copy_items(), eager_location_graph.copy_items());

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

        // Copying and splitting protected regions can introduce boundaries in an
        // original block after the initial boundary snapshot. Retain those labels
        // even when every instruction in the original block has been copied away.
        for block in self
            .exception_regions
            .iter()
            .flat_map(|region| [region.start, region.end, region.target])
            .filter_map(|label| graph.label_block(label))
        {
            exception_boundary_blocks[block.index()] = true;
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
        Ok(copied_entry_labels)
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

    #[test]
    fn exit_location_propagation_skips_only_located_or_missing_first_instructions() {
        let entry = Label::from_u32(0);
        let target = Label::from_u32(1);
        let alias = Label::from_u32(2);
        let unlocated_load = instruction(LOAD_CONST, Operand::Value(0), -1);
        for version in [
            crate::PythonVersion::Py312,
            crate::PythonVersion::Py313,
            crate::PythonVersion::Py314,
            crate::PythonVersion::Py315,
        ] {
            for first in [
                None,
                Some(located(19)),
                Some(unlocated_load),
                Some(head()),
                Some(instruction(POP_TOP, Operand::Value(0), -1)),
                Some(instruction(POP_TOP, Operand::Value(0), 19)),
                Some(instruction(POP_EXCEPT, Operand::Value(0), -1)),
            ] {
                for tail in [
                    None,
                    Some(instruction(RETURN_VALUE, Operand::Value(0), -1)),
                    Some(instruction(RETURN_VALUE, Operand::Value(0), 29)),
                ] {
                    let items = [
                        Item::Label(entry),
                        located(7),
                        instruction(JUMP_FORWARD, Operand::Forward(target), 7),
                        Item::Label(target),
                        Item::Label(alias),
                    ]
                    .into_iter()
                    .chain(first)
                    .chain(tail)
                    .collect();
                    let empty = FxHashSet::default();
                    let provenance = ExceptionOwnerProvenance::default();
                    let mut graph = BlockGraph::from_items(items, &empty);
                    let order = graph.order().to_vec();
                    let analysis =
                        exit_block_analysis(&graph, &order, &[], &empty, &empty, &provenance);
                    propagate_exit_block_locations(
                        &mut graph,
                        &order,
                        &analysis,
                        &[],
                        &empty,
                        &empty,
                        &FxHashMap::default(),
                        &empty,
                        &empty,
                        &[],
                        version,
                        &provenance,
                    );

                    let target_block = graph.label_block(target).unwrap();
                    let actual = graph.instructions_mut(target_block).next();
                    if matches!(first, Some(Item::Instruction(instruction)) if instruction.location.line == 19)
                    {
                        assert_eq!(actual.unwrap().location.line, 19);
                    } else if first == Some(unlocated_load)
                        && tail.is_none_or(|item| matches!(item, Item::Instruction(instruction) if instruction.location.line < 0))
                    {
                        assert_eq!(actual.unwrap().location.line, 7);
                    } else if first.is_none() && tail.is_none() {
                        assert!(actual.is_none());
                    }
                }
            }
        }
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
            let start = Label::from_u32(1);
            let end = Label::from_u32(2);
            let target = Label::from_u32(3);
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
                let entry = Label::from_u32(empty_count + 1);
                let mut items = vec![
                    Item::Label(Label::from_u32(0)),
                    instruction(JUMP_FORWARD, Operand::Forward(Label::from_u32(1)), 8),
                ];
                items.extend((1..=empty_count).map(|index| Item::Label(Label::from_u32(index))));
                items.extend([
                    Item::Label(entry),
                    retained_head(),
                    located(20),
                    instruction(RETURN_VALUE, Operand::Value(0), 20),
                ]);
                let prefix = ((if omit_origin { 2 } else { 1 })..=empty_count + 1)
                    .map(Label::from_u32)
                    .collect();
                check_retained(items, &[], entry, prefix, (!omit_origin).then_some(8));
            }
        }
    }

    #[test]
    fn retained_pop_block_rejects_a_shared_empty_predecessor() {
        let empty = Label::from_u32(2);
        let entry = Label::from_u32(3);
        check_retained(
            vec![
                Item::Label(Label::from_u32(0)),
                instruction(LOAD_FAST, Operand::Value(0), 1),
                instruction(POP_JUMP_IF_FALSE, Operand::Forward(empty), 1),
                Item::Label(Label::from_u32(1)),
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
            let empty = Label::from_u32(1);
            let entry = Label::from_u32(2);
            let end = Label::from_u32(3);
            let regions = if handler {
                vec![region(Label::from_u32(0), end, empty)]
            } else {
                Vec::new()
            };
            check_retained(
                vec![
                    Item::Label(Label::from_u32(0)),
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
            let entry = Label::from_u32(0);
            let left = Label::from_u32(1);
            let right = Label::from_u32(2);
            let parent = Label::from_u32(3);
            let child = Label::from_u32(4);
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
        let dead = Label::from_u32(1);
        let target = Label::from_u32(2);
        check(
            vec![
                Item::Label(Label::from_u32(0)),
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
        let other = Label::from_u32(1);
        let target = Label::from_u32(2);
        // Both edges are present after all retained-target revivals and deferred
        // copies have been materialized. Neither may be discounted as an old copy.
        check(
            vec![
                Item::Label(Label::from_u32(0)),
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
        let target = Label::from_u32(1);
        check(
            vec![
                Item::Label(Label::from_u32(0)),
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
        let entry = Label::from_u32(0);
        let dead_start = Label::from_u32(1);
        let dead_end = Label::from_u32(2);
        let handler = Label::from_u32(3);
        let target = Label::from_u32(4);
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
        let target = Label::from_u32(1);
        let Item::Instruction(mut last) = jump(target) else {
            unreachable!()
        };
        last.no_location_policy.preserve_location();
        check(
            vec![
                Item::Label(Label::from_u32(0)),
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
        let entry = Label::from_u32(0);
        let end = Label::from_u32(1);
        let target = Label::from_u32(2);
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
        let target = Label::from_u32(1);
        let source = Label::from_u32(2);
        check(
            vec![
                Item::Label(Label::from_u32(0)),
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
        let first = Label::from_u32(1);
        let second = Label::from_u32(2);
        for heads in [[first, second], [second, first]] {
            check(
                vec![
                    Item::Label(Label::from_u32(0)),
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

#[cfg(test)]
mod percent_prefix_setup_tests {
    use super::*;
    use crate::assembler::{Assembler, ExceptionRegionPriority};

    #[test]
    fn setup_prefix_requires_original_seam_and_untouched_prefix() {
        for mutation in [
            "none", "range", "setup", "span", "prefix", "depth", "unknown", "owner", "incoming",
        ] {
            let [start, end, setup, stop, handler] =
                std::array::from_fn(|i| Label::from_u32(u32::try_from(i).unwrap()));
            let location = SourceLocation::new(2, 5, 4, 12);
            let mut marker = Instruction::new(
                NOP,
                Operand::Value(0),
                location,
                (mutation != "unknown").then_some(0),
            );
            marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            let mut prefix = Instruction::new(
                LOAD_CONST,
                Operand::Value(0),
                SourceLocation::NONE,
                (mutation != "unknown").then_some(if mutation == "depth" { 2 } else { 1 }),
            );
            prefix.no_location_policy.percent_target_location =
                if mutation == "prefix" { 0 } else { u8::MAX };
            if mutation == "owner" {
                prefix.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            }
            let ret = Item::Instruction(Instruction::new(
                RETURN_VALUE,
                Operand::Value(0),
                SourceLocation::new(3, 3, 8, 20),
                Some(0),
            ));
            let mut items = vec![
                Item::Label(start),
                Item::Instruction(marker),
                Item::Label(end),
                Item::Label(setup),
                Item::Instruction(prefix),
                Item::Label(stop),
                ret,
                Item::Label(handler),
            ];
            if mutation == "incoming" {
                items.push(Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(setup),
                    location,
                    Some(0),
                )));
            }
            items.push(ret);
            let mut assembler = Assembler::from_linear_items(items);
            assembler.next_label = 5;
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.has_synthesized_percent_target_locations = true;
            if mutation != "range" {
                pipeline
                    .exception_owner_provenance
                    .setup_nop_ranges
                    .push((start, end));
            }
            if mutation != "setup" {
                pipeline.exception_owner_provenance.setups.push((
                    setup,
                    Some(if mutation == "span" {
                        SourceLocation::new(2, 5, 5, 12)
                    } else {
                        location
                    }),
                ));
            }
            pipeline.exception_regions.push(ExceptionRegion {
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(setup, stop),
                start: setup,
                end: stop,
                target: handler,
                depth: 0,
                preserve_lasti: false,
            });
            pipeline.resolve_synthesized_percent_locations();
            let block = pipeline.graph.label_block(setup).unwrap();
            let actual = pipeline.graph.instructions_mut(block).next().unwrap();
            assert_eq!(
                actual.location,
                if mutation == "none" {
                    location
                } else {
                    SourceLocation::NONE
                },
                "{mutation}"
            );
        }
    }
}

#[cfg(test)]
mod copied_match_guard_subject_tests {
    use super::*;

    #[test]
    fn donor_requires_original_guard_bridge_and_exact_four_record_return() {
        let guard = SourceLocation::new(3, 3, 20, 40);
        let body = SourceLocation::new(4, 4, 12, 30);
        let cleanup = Label::from_u32(1);
        let end = Label::from_u32(2);
        let condition = Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(Label::from_u32(3)),
            guard,
            Some(1),
        );
        let marker = Instruction::new(NOT_TAKEN, Operand::Value(0), guard, Some(1));
        let mut bridge = Instruction::new(JUMP_FORWARD, Operand::Forward(end), guard, Some(1));
        bridge.jump_policy.retain_nop();
        let failure =
            Instruction::new(POP_JUMP_IF_FALSE, Operand::Forward(cleanup), guard, Some(2));
        let cleanup_pop = Instruction::new(POP_TOP, Operand::Value(0), guard, Some(1));
        let cleanup_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(Label::from_u32(3)),
            guard,
            Some(1),
        );
        let mut subject = Instruction::new(POP_TOP, Operand::Value(0), body, Some(0));
        subject.fusion_policy.mark_location_from_next();
        for mutation in 0..12 {
            let mut source = vec![
                Item::Label(Label::from_u32(0)),
                Item::Instruction(marker),
                Item::Instruction(bridge),
            ];
            let mut target = vec![
                Item::Label(end),
                Item::Instruction(subject),
                Item::Instruction(Instruction::new(
                    LOAD_GLOBAL,
                    Operand::Value(0),
                    body,
                    Some(1),
                )),
                Item::Instruction(Instruction::new(
                    LOAD_ATTR,
                    Operand::Value(0),
                    body,
                    Some(1),
                )),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    body,
                    Some(0),
                )),
            ];
            let mut predecessor = condition;
            let mut origins = vec![(
                cleanup,
                end,
                [
                    failure,
                    condition,
                    marker,
                    bridge,
                    cleanup_pop,
                    cleanup_jump,
                ],
            )];
            match mutation {
                1 => origins.clear(),
                2 => predecessor.location = body,
                3 => target[0] = Item::Label(Label::from_u32(99)),
                4 => target.insert(3, Item::Label(Label::from_u32(99))),
                5 => target.insert(
                    3,
                    Item::Instruction(Instruction::new(NOP, Operand::Value(0), body, Some(1))),
                ),
                6 => {
                    let mut changed = subject;
                    changed.fusion_policy.clear_location_from_next();
                    target[1] = Item::Instruction(changed);
                }
                7 => {
                    let mut changed = subject;
                    changed.local_policy.add_hidden_exception_owner_slots(1);
                    target[1] = Item::Instruction(changed);
                }
                8 => {
                    target[2] = Item::Instruction(Instruction::new(
                        LOAD_GLOBAL,
                        Operand::Value(1),
                        body,
                        Some(1),
                    ));
                }
                9 => {
                    target[3] = Item::Instruction(Instruction::new(
                        LOAD_ATTR,
                        Operand::Value(1),
                        body,
                        Some(1),
                    ));
                }
                10 => source.insert(2, Item::Label(Label::from_u32(99))),
                11 => {
                    let mut changed = bridge;
                    changed.jump_policy.prevent_small_exit_inlining();
                    source[2] = Item::Instruction(changed);
                }
                _ => {}
            }
            let donor = copied_match_guard_subject_donor(&source, &target, &predecessor, &origins);
            assert_eq!(donor.is_some(), mutation == 0, "mutation {mutation}");
            if let Some((location, records)) = donor {
                assert_eq!(location, guard);
                assert_eq!(records[0], subject);
                assert!(records[0].fusion_policy.location_from_next());
            }
        }
    }
}

#[cfg(test)]
mod represented_setup_nop_tests {
    use super::*;
    use crate::assembler::ExceptionRegionPriority;

    fn setup_nop(location: SourceLocation) -> Instruction {
        let mut nop = Instruction::new(NOP, Operand::Value(0), location, Some(0));
        nop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        nop
    }

    // The producer emits distinct NOP boundaries and setup labels. Each nested
    // NOP splits its surrounding exception regions, including an empty prefix.
    fn nested_raise(count: u32) -> (BlockGraph, ExceptionOwnerProvenance, Vec<ExceptionRegion>) {
        let mut items = vec![
            Item::Label(Label::from_u32(0)),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(Label::from_u32(10)),
                SourceLocation::new(2, 2, 4, 8),
                Some(0),
            )),
        ];
        let mut provenance = ExceptionOwnerProvenance::default();
        for index in 0..count {
            let start = Label::from_u32(10 + 3 * index);
            let end = Label::from_u32(11 + 3 * index);
            let setup = Label::from_u32(12 + 3 * index);
            let line = i32::try_from(index + 4).unwrap();
            let location = SourceLocation::new(line, line + 4, 4, 16);
            items.extend([
                Item::Label(start),
                Item::Instruction(setup_nop(location)),
                Item::Label(end),
                Item::Label(setup),
            ]);
            provenance.setups.push((setup, Some(location)));
            provenance
                .ordinary_try_setups
                .insert(setup, Label::from_u32(200 + index));
            provenance.setup_nop_ranges.push((start, end));
        }
        let raised = SourceLocation::new(9, 9, 8, 23);
        items.extend([
            Item::Instruction(Instruction::new(
                LOAD_GLOBAL,
                Operand::Value(0),
                raised,
                Some(1),
            )),
            Item::Instruction(Instruction::new(
                RAISE_VARARGS,
                Operand::Value(1),
                raised,
                Some(0),
            )),
            Item::Label(Label::from_u32(100)),
        ]);
        let mut regions = Vec::new();
        for index in 0..count {
            let setup = Label::from_u32(12 + 3 * index);
            let handler = Label::from_u32(200 + index);
            let mut start = setup;
            for nested in index + 1..count {
                regions.push(ExceptionRegion {
                    priority: ExceptionRegionPriority::OriginalRange(setup, Label::from_u32(100)),
                    is_scope_fallback: false,
                    start,
                    end: Label::from_u32(10 + 3 * nested),
                    target: handler,
                    depth: 0,
                    preserve_lasti: false,
                });
                start = Label::from_u32(11 + 3 * nested);
            }
            regions.push(ExceptionRegion {
                priority: ExceptionRegionPriority::OriginalRange(setup, Label::from_u32(100)),
                is_scope_fallback: false,
                start,
                end: Label::from_u32(100),
                target: handler,
                depth: 0,
                preserve_lasti: false,
            });
            items.extend([
                Item::Label(handler),
                Item::Instruction(Instruction::new(
                    RERAISE,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    Some(0),
                )),
            ]);
        }
        (
            BlockGraph::from_items(items, &FxHashSet::default()),
            provenance,
            regions,
        )
    }

    #[test]
    fn source_preflight_matches_original_across_policies_and_graph_shapes() {
        let source_label = Label::from_u32(0);
        let other_label = Label::from_u32(1);
        let target_label = Label::from_u32(2);
        let boundary = Label::from_u32(3);
        let end = Label::from_u32(4);
        let unknown = Label::from_u32(99);
        let located = SourceLocation::new(3, 3, 0, 3);
        let mut source_count = 0;
        let mut accepted = 0;
        for source_opcode in [JUMP_FORWARD, POP_JUMP_IF_FALSE, NOP, RETURN_VALUE] {
            for source_policy in 0..4 {
                for target_opcode in [NOP, RETURN_VALUE, RAISE_VARARGS, JUMP_BACKWARD] {
                    for target_policy in 0..4 {
                        for target_shape in 0..4 {
                            for structure in 0..6 {
                                let operand = match structure {
                                    4 => Operand::Value(0),
                                    5 => Operand::Forward(unknown),
                                    _ => Operand::Forward(target_label),
                                };
                                let mut source =
                                    Instruction::new(source_opcode, operand, located, None);
                                if source_policy & 1 != 0 {
                                    source.jump_policy.prevent_small_exit_inlining();
                                }
                                if source_policy & 2 != 0 {
                                    source.no_location_policy.allow_split_return_exit_copy();
                                }
                                let mut target = Instruction::new(
                                    target_opcode,
                                    if target_opcode == JUMP_BACKWARD {
                                        Operand::Backward(source_label)
                                    } else {
                                        Operand::Value(0)
                                    },
                                    if target_shape == 1 {
                                        located
                                    } else {
                                        SourceLocation::NONE
                                    },
                                    None,
                                );
                                if target_policy & 1 != 0 {
                                    target.no_location_policy.allow_block_inlining();
                                }
                                if target_policy & 2 != 0 {
                                    target.no_location_policy.mark_artificial_eval_break();
                                }
                                let mut items =
                                    vec![Item::Label(source_label), Item::Instruction(source)];
                                if structure != 3 {
                                    items.extend([
                                        Item::Label(other_label),
                                        Item::Instruction(Instruction::new(
                                            if structure == 1 {
                                                JUMP_FORWARD
                                            } else {
                                                RETURN_VALUE
                                            },
                                            if structure == 1 {
                                                Operand::Forward(target_label)
                                            } else {
                                                Operand::Value(0)
                                            },
                                            located,
                                            None,
                                        )),
                                    ]);
                                }
                                if target_shape == 3 {
                                    items.extend([
                                        Item::Label(boundary),
                                        Item::Instruction(Instruction::new(
                                            RETURN_VALUE,
                                            Operand::Value(0),
                                            located,
                                            None,
                                        )),
                                        Item::Label(end),
                                        Item::Label(target_label),
                                    ]);
                                } else {
                                    items.push(Item::Label(target_label));
                                    if target_shape == 2 {
                                        items.push(Item::Instruction(Instruction::new(
                                            LOAD_FAST,
                                            Operand::Value(0),
                                            located,
                                            None,
                                        )));
                                    }
                                    items.extend([
                                        Item::Instruction(target),
                                        Item::Label(boundary),
                                        Item::Instruction(Instruction::new(
                                            RETURN_VALUE,
                                            Operand::Value(0),
                                            located,
                                            None,
                                        )),
                                        Item::Label(end),
                                    ]);
                                }
                                let regions = if structure == 2 || structure == 3 {
                                    vec![ExceptionRegion {
                                        start: boundary,
                                        end,
                                        target: end,
                                        depth: 0,
                                        preserve_lasti: false,
                                        priority: ExceptionRegionPriority::OriginalRange(
                                            boundary, end,
                                        ),
                                        is_scope_fallback: false,
                                    }]
                                } else {
                                    Vec::new()
                                };
                                let nops = if target_policy == 3 {
                                    FxHashSet::from_iter([target_label])
                                } else {
                                    FxHashSet::default()
                                };
                                let graph = BlockGraph::from_items(items, &FxHashSet::default());
                                let analysis = exit_block_analysis(
                                    &graph,
                                    graph.order(),
                                    &regions,
                                    &FxHashSet::default(),
                                    &nops,
                                    &ExceptionOwnerProvenance::default(),
                                );
                                for &source in graph.order() {
                                    let result = source_may_copy_exit(
                                        &graph,
                                        graph.order(),
                                        &analysis.positions,
                                        &analysis.predecessors,
                                        &regions,
                                        &nops,
                                        source,
                                    );
                                    let original = source_may_copy_exit_original(
                                        &graph,
                                        graph.order(),
                                        &analysis.positions,
                                        &analysis.predecessors,
                                        &regions,
                                        &nops,
                                        source,
                                    );
                                    assert_eq!(
                                        result, original,
                                        "{source_opcode:?} {source_policy} {target_opcode:?} {target_policy} {target_shape} {structure} {source:?}"
                                    );
                                    source_count += 1;
                                    accepted += usize::from(result);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(accepted > 0 && accepted < source_count);
    }

    #[test]
    fn nested_setup_boundaries_preserve_four_slot_exit_limit() {
        for count in [2, 3] {
            let (graph, provenance, regions) = nested_raise(count);
            let analysis = exit_block_analysis(
                &graph,
                graph.order(),
                &regions,
                &FxHashSet::default(),
                &FxHashSet::default(),
                &provenance,
            );
            assert_eq!(analysis.requires_copy, count == 2, "{count} setup records");
        }
    }

    #[test]
    fn shared_exit_target_keeps_each_predecessors_copy_policy() {
        for copy_source in [None, Some(0), Some(1)] {
            let target = Label::from_u32(3);
            let location = SourceLocation::new(2, 2, 0, 5);
            let mut items = Vec::new();
            for source in 0..2 {
                items.push(Item::Label(Label::from_u32(source)));
                let mut jump = Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(target),
                    location,
                    Some(0),
                );
                if copy_source == Some(source) {
                    jump.no_location_policy.allow_split_return_exit_copy();
                }
                items.push(Item::Instruction(jump));
            }
            items.extend([
                Item::Label(target),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    location,
                    Some(0),
                )),
            ]);
            let graph = BlockGraph::from_items(items, &FxHashSet::default());
            let analysis = exit_block_analysis(
                &graph,
                graph.order(),
                &[],
                &FxHashSet::default(),
                &FxHashSet::default(),
                &ExceptionOwnerProvenance::default(),
            );
            assert_eq!(analysis.requires_copy, copy_source.is_some());
        }
    }

    #[test]
    fn exit_copy_preserves_located_loop_target_while_inlining_separate_raise() {
        for width in [1, 4, 20] {
            let entry = Label::from_u32(0);
            let second = Label::from_u32(1);
            let raise_source = Label::from_u32(2);
            let continuation = Label::from_u32(3);
            let raised = Label::from_u32(4);
            let location = SourceLocation::new(2, 2, 0, 5);
            let located = |opcode, operand| {
                Item::Instruction(Instruction::new(opcode, operand, location, None))
            };
            let mut items = vec![
                Item::Label(entry),
                located(POP_JUMP_IF_FALSE, Operand::Forward(continuation)),
                Item::Label(second),
                located(POP_JUMP_IF_FALSE, Operand::Forward(continuation)),
                Item::Label(raise_source),
                located(JUMP_FORWARD, Operand::Forward(raised)),
            ];
            let mut original_continuation = vec![Item::Label(continuation)];
            original_continuation.extend((0..width).map(|_| located(NOP, Operand::Value(0))));
            original_continuation.push(located(
                JUMP_BACKWARD_NO_INTERRUPT,
                Operand::Backward(entry),
            ));
            items.extend_from_slice(&original_continuation);
            items.extend([
                Item::Label(raised),
                located(RAISE_VARARGS, Operand::Value(0)),
            ]);
            let mut assembler = crate::assembler::Assembler::from_linear_items(items);
            assembler.next_label = 5;
            let mut pipeline = assembler.into_optimization_pipeline();
            let analysis = exit_block_analysis(
                &pipeline.graph,
                pipeline.graph.order(),
                &[],
                &FxHashSet::default(),
                &FxHashSet::default(),
                &pipeline.exception_owner_provenance,
            );
            assert!(analysis.requires_copy, "width {width}");

            let absence = pipeline.capture_original_location_absence();
            pipeline.duplicate_exit_blocks(&absence, &mut Vec::new());
            let continuation_block = pipeline.graph.label_block(continuation).unwrap();
            assert_eq!(
                pipeline.graph[continuation_block].items(),
                original_continuation,
                "width {width}"
            );
            let raise_source_block = pipeline.graph.label_block(raise_source).unwrap();
            assert_eq!(
                pipeline.graph[raise_source_block]
                    .last_instruction()
                    .unwrap()
                    .opcode,
                RAISE_VARARGS,
                "width {width}"
            );
        }
    }

    #[test]
    fn exit_copy_counts_folded_width_through_four_original_slots() {
        for extra_width in [0, 1, 2, 3, 200] {
            let source = Label::from_u32(0);
            let target = Label::from_u32(1);
            let location = SourceLocation::new(2, 2, 0, 5);
            let mut value = Instruction::new(LOAD_FAST, Operand::Value(0), location, Some(1));
            value.add_pre_fold_extra_instructions(extra_width);
            let items = vec![
                Item::Label(source),
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(target),
                    location,
                    Some(0),
                )),
                Item::Label(target),
                Item::Instruction(value),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    location,
                    Some(0),
                )),
            ];
            let mut assembler = crate::assembler::Assembler::from_linear_items(items);
            assembler.next_label = 2;
            let mut pipeline = assembler.into_optimization_pipeline();
            let analysis = exit_block_analysis(
                &pipeline.graph,
                pipeline.graph.order(),
                &[],
                &FxHashSet::default(),
                &FxHashSet::default(),
                &pipeline.exception_owner_provenance,
            );
            assert_eq!(analysis.requires_copy, extra_width <= 2);

            let absence = pipeline.capture_original_location_absence();
            pipeline.duplicate_exit_blocks(&absence, &mut Vec::new());
            let source = pipeline.graph.label_block(source).unwrap();
            let last = pipeline.graph[source].last_instruction().unwrap();
            assert_eq!(last.opcode == RETURN_VALUE, extra_width <= 2);
            assert_eq!(last.opcode == JUMP_FORWARD, extra_width > 2);
        }
    }

    #[test]
    fn boundaries_require_unique_full_setup_nop_certificate() {
        for mutation in [
            "none",
            "missing_range",
            "duplicate_range",
            "duplicate_setup",
            "not_ordinary",
            "location",
            "policy",
            "operand",
            "prefold",
        ] {
            let (mut graph, mut provenance, _) = nested_raise(2);
            let setup = Label::from_u32(15);
            let range = (Label::from_u32(13), Label::from_u32(14));
            match mutation {
                "missing_range" => provenance
                    .setup_nop_ranges
                    .retain(|candidate| *candidate != range),
                "duplicate_range" => provenance.setup_nop_ranges.push(range),
                "duplicate_setup" => provenance.setups.push(provenance.setups[1]),
                "not_ordinary" => {
                    provenance.ordinary_try_setups.remove(&setup);
                }
                "location" | "policy" | "operand" | "prefold" => {
                    let block = graph.label_block(range.0).unwrap();
                    let nop = graph.instructions_mut(block).next().unwrap();
                    match mutation {
                        "location" => nop.location.column += 1,
                        "policy" => {
                            *nop = Instruction::new(NOP, Operand::Value(0), nop.location, Some(0));
                        }
                        "operand" => {
                            *nop = Instruction::new(NOP, Operand::Value(1), nop.location, Some(0));
                            nop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                        }
                        "prefold" => nop.add_pre_fold_extra_instructions(1),
                        _ => unreachable!(),
                    }
                }
                _ => {}
            }
            let represented = represented_setup_nops(&graph, graph.order(), &provenance);
            for label in [setup, range.0, range.1] {
                assert_eq!(
                    represented.labels.contains(&label),
                    mutation == "none",
                    "{mutation}: {label:?}"
                );
            }
            assert_eq!(
                represented.setups.contains(&setup),
                mutation == "none",
                "{mutation}"
            );
        }
    }

    #[test]
    fn boundary_alias_does_not_certify_a_different_original_setup() {
        let first = SourceLocation::new(4, 7, 4, 20);
        let second = SourceLocation::new(8, 11, 4, 20);
        for first_is_certified in [false, true] {
            let items = vec![
                Item::Label(Label::from_u32(0)),
                Item::Instruction(setup_nop(first)),
                // This label is both an uncertified original setup and the exact
                // start of a different, certified NOP's range.
                Item::Label(Label::from_u32(1)),
                Item::Label(Label::from_u32(9)),
                Item::Instruction(setup_nop(second)),
                Item::Label(Label::from_u32(2)),
                Item::Label(Label::from_u32(3)),
                Item::Instruction(Instruction::new(
                    RAISE_VARARGS,
                    Operand::Value(0),
                    second,
                    Some(0),
                )),
            ];
            let graph = BlockGraph::from_items(items, &[Label::from_u32(1)].into_iter().collect());
            let mut provenance = ExceptionOwnerProvenance {
                setups: vec![
                    (
                        Label::from_u32(1),
                        Some(if first_is_certified { first } else { second }),
                    ),
                    (Label::from_u32(3), Some(second)),
                ],
                ..ExceptionOwnerProvenance::default()
            };
            provenance
                .ordinary_try_setups
                .insert(Label::from_u32(1), Label::from_u32(20));
            provenance
                .ordinary_try_setups
                .insert(Label::from_u32(3), Label::from_u32(21));
            provenance
                .setup_nop_ranges
                .push((Label::from_u32(1), Label::from_u32(2)));
            if first_is_certified {
                provenance
                    .setup_nop_ranges
                    .push((Label::from_u32(0), Label::from_u32(1)));
            }
            let represented = represented_setup_nops(&graph, graph.order(), &provenance);
            assert!(represented.labels.contains(&Label::from_u32(1)));
            assert!(represented.setups.contains(&Label::from_u32(3)));
            assert!(!represented.labels.contains(&Label::from_u32(9)));
            assert_eq!(
                represented.setups.contains(&Label::from_u32(1)),
                first_is_certified
            );
            let mut positions = vec![0; graph.len()];
            for (position, block) in graph.order().iter().enumerate() {
                positions[block.index()] = position;
            }
            assert_eq!(
                block_is_original_setup_nop(
                    &graph,
                    graph.order(),
                    &positions,
                    graph.order()[0],
                    &provenance
                ),
                first_is_certified
            );
        }
    }
}
