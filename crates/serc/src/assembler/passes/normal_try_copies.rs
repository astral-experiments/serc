use std::cmp::Reverse;

use rustc_hash::{FxHashMap, FxHashSet};

use super::super::block_graph::{BasicBlockId, ItemPosition};
use super::super::exception_owners::TupleJoinLocationOrigin;
use super::super::is_unconditional_jump;
use super::super::{
    ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label, Operand,
    OptimizationPipeline, SourceLocation,
};
use super::normal_try_copy_plan::{OriginalNode, OriginalTail, PrefixSlot, plan_normal_try_copies};
use crate::target::opcodes::{JUMP_BACKWARD, JUMP_FORWARD, NOP, POP_EXCEPT};

/// Checks whether restored owners retain priority when a jump's exclusion is cleared.
///
/// Unknown ranges or priorities are rejected. Existing regions win ties, so a tied
/// region must preserve the selected owner's handler, stack depth, and `lasti` policy.
fn restored_empty_try_owner_is_unshadowed(
    owners: &[ExceptionRegion],
    regions: &[ExceptionRegion],
    offsets: &FxHashMap<Label, usize>,
    position: usize,
) -> bool {
    let priority = |region: &ExceptionRegion| match region.priority {
        ExceptionRegionPriority::Resolved { start, end } if start < end => {
            Some((region.is_scope_fallback, Reverse(start), end))
        }
        _ => None,
    };
    let mut winner = None;
    for owner in owners {
        let Some(key) = priority(owner) else {
            return false;
        };
        if winner.as_ref().is_none_or(|(best, _)| key < *best) {
            winner = Some((key, owner));
        }
    }
    let Some((owner_priority, owner)) = winner else {
        return false;
    };
    for region in regions {
        let Some((&start, &end)) = offsets.get(&region.start).zip(offsets.get(&region.end)) else {
            return false;
        };
        if start > end {
            return false;
        }
        if start <= position && position < end {
            let Some(key) = priority(region) else {
                return false;
            };
            // Existing regions precede restored owners, so they win priority ties.
            if key < owner_priority
                || (key == owner_priority
                    && (region.target != owner.target
                        || region.depth != owner.depth
                        || region.preserve_lasti != owner.preserve_lasti))
            {
                return false;
            }
        }
    }
    true
}

/// Recognizes two intact, located slots of an explicit local return.
pub(super) fn original_local_return(items: &[Item]) -> bool {
    use crate::target::opcodes::{LOAD_FAST, RETURN_VALUE};

    let head_len = items
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    matches!(&items[head_len..],
    [Item::Instruction(load), Item::Instruction(ret)]
        if load.opcode == LOAD_FAST
            && load.value_argument().is_some()
            && ret.opcode == RETURN_VALUE
            && ret.value_argument() == Some(0)
            && [load, ret].iter().all(|instruction| {
                instruction.location.line >= 0
                    && instruction.pre_fold_extra_instructions() == 0
                    && instruction.local_policy.hidden_exception_owner_slots() == 0
                    && !instruction.fusion_policy.is_converted_pop_block()
                    && !instruction.fusion_policy.is_authored_break()
            }))
}

/// Recognizes four intact original slots of an explicit two-local tuple return.
pub(super) fn original_local_tuple_return(items: &[Item]) -> Option<[bool; 4]> {
    use crate::target::opcodes::{BUILD_TUPLE, LOAD_FAST, RETURN_VALUE};

    let head_len = items
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [
        Item::Instruction(first),
        Item::Instruction(second),
        Item::Instruction(tuple),
        Item::Instruction(ret),
    ] = &items[head_len..]
    else {
        return None;
    };
    (first.opcode == LOAD_FAST
        && first.value_argument().is_some()
        && second.opcode == LOAD_FAST
        && second.value_argument().is_some()
        && tuple.opcode == BUILD_TUPLE
        && tuple.value_argument() == Some(2)
        && ret.opcode == RETURN_VALUE
        && ret.value_argument() == Some(0)
        && [first, second, tuple, ret].iter().all(|instruction| {
            instruction.location.line >= 0
                && instruction.pre_fold_extra_instructions() == 0
                && !instruction.fusion_policy.is_converted_pop_block()
                && !instruction.fusion_policy.is_authored_break()
        }))
    .then_some([true; 4])
}

/// Recognizes the four original slots of an explicit local subscript return.
fn original_local_subscript_return(items: &[Item]) -> Option<OriginalTail> {
    use crate::target::opcodes::{BINARY_OP, LOAD_FAST, RETURN_VALUE};
    use crate::target::operands::BinaryOperation;

    let head = items
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [
        Item::Instruction(value),
        Item::Instruction(index),
        Item::Instruction(subscript),
        Item::Instruction(ret),
    ] = &items[head..]
    else {
        return None;
    };
    (value.opcode == LOAD_FAST
        && value.value_argument().is_some()
        && index.opcode == LOAD_FAST
        && index.value_argument().is_some()
        && subscript.opcode == BINARY_OP
        && subscript.operand() == Operand::Value(BinaryOperation::SUBSCR.argument())
        && ret.opcode == RETURN_VALUE
        && ret.value_argument() == Some(0)
        && [value, index, subscript, ret]
            .into_iter()
            .zip([1, 2, 1, 0])
            .all(|(instruction, depth)| {
                instruction.depth_after() == Some(depth)
                    && instruction.location.line >= 0
                    && instruction.pre_fold_extra_instructions() == 0
                    && instruction.local_policy.hidden_exception_owner_slots() == 0
                    && instruction.no_location_policy
                        == super::super::ir::NoLocationPolicy::default()
                    && instruction.exception_policy() == super::super::ExceptionPolicy::default()
                    && !instruction.fusion_policy.is_converted_pop_block()
                    && !instruction.fusion_policy.is_authored_break()
            }))
    .then_some(OriginalTail::ComputedReturn { width: 4 })
}

/// Recognizes an intact, single-line global attribute or zero-argument call return.
fn original_computed_return(items: &[Item]) -> Option<OriginalTail> {
    use crate::target::opcodes::{CALL, LOAD_ATTR, LOAD_GLOBAL, RETURN_VALUE};

    let head_len = items
        .iter()
        .take_while(|item| matches!(item, Item::Label(_)))
        .count();
    let [
        Item::Instruction(load),
        Item::Instruction(value),
        Item::Instruction(ret),
    ] = &items[head_len..]
    else {
        return None;
    };
    let argument = load.value_argument()?;
    if load.opcode != LOAD_GLOBAL
        || ret.opcode != RETURN_VALUE
        || ret.value_argument() != Some(0)
        || value.depth_after() != Some(1)
        || ret.depth_after() != Some(0)
        || [load, value, ret].iter().any(|instruction| {
            instruction.location.line < 0
                || instruction.location.line != ret.location.line
                || instruction.location.end_line != ret.location.line
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.local_policy.hidden_exception_owner_slots() != 0
                || instruction.fusion_policy.is_converted_pop_block()
                || instruction.fusion_policy.is_authored_break()
        })
    {
        return None;
    }
    let width = if argument & 1 == 0
        && load.depth_after() == Some(1)
        && value.opcode == LOAD_ATTR
        && matches!(value.operand(), Operand::Value(attribute) if attribute & 1 == 0)
    {
        3
    } else if argument & 1 != 0
        && load.depth_after() == Some(2)
        && value.opcode == CALL
        && value.value_argument() == Some(0)
    {
        // The fused LOAD_GLOBAL still represents its original PUSH_NULL slot.
        4
    } else {
        return None;
    };
    Some(OriginalTail::ComputedReturn { width })
}

impl OptimizationPipeline {
    /// Detects copied/folded aliases or rewritten instruction policies in a return array.
    ///
    /// All policy fields must match a fresh instruction, including fields not used by
    /// return recognition.
    fn return_array_has_rewritten_items(&self, items: &[Item]) -> bool {
        items.iter().any(|item| match item {
            Item::Label(label) => {
                self.borrow_block_aliases.contains(label)
                    || self.folded_original_predecessor_targets.contains(label)
            }
            Item::Instruction(instruction) => {
                *instruction
                    != Instruction::new(
                        instruction.opcode,
                        instruction.operand(),
                        instruction.location,
                        instruction.depth_after(),
                    )
            }
        })
    }

    /// Schedules exact local-tuple exits through their original unprotected arrays.
    pub(in crate::assembler) fn prepare_plain_tuple_copy_plan(&mut self) {
        use crate::target::opcodes::{
            BUILD_LIST, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL, NOT_TAKEN, POP_JUMP_IF_FALSE,
            STORE_FAST, SWAP, TO_BOOL, YIELD_VALUE,
        };

        if !self.graph.features().has_forward_jump()
            || !self
                .exception_owner_provenance
                .normal_try_copy_steps
                .is_empty()
            || !self
                .exception_owner_provenance
                .normal_try_copy_blocks
                .is_empty()
        {
            return;
        }
        let graph = &self.graph;
        // Every scheduled copy must terminate at one of these two exact original arrays.
        // Most graphs with a forward jump contain neither, so avoid building their indexes
        // and authenticating every block before discovering that there is nothing to copy.
        if !graph.iter().any(|(_, block)| {
            original_local_tuple_return(block.items()).is_some()
                || original_local_subscript_return(block.items()).is_some()
        }) {
            return;
        }
        let order = graph.order();
        let positions = order
            .iter()
            .enumerate()
            .map(|(index, &block)| (block, index))
            .collect::<FxHashMap<_, _>>();
        let mut offsets = FxHashMap::default();
        let mut ranges = FxHashMap::default();
        let mut offset = 0;
        for &block in order {
            let start = offset;
            for item in graph[block].items() {
                match item {
                    Item::Label(label) => {
                        offsets.insert(*label, offset);
                    }
                    Item::Instruction(_) => offset += 1,
                }
            }
            ranges.insert(block, (start, offset));
        }
        // These records distinguish an erased conditional from the discarded
        // loop-else predecessors included in the aggregate folded-target set.
        let is_folded_condition_alias = |label: Label| {
            if self.folded_loop_exit_anchors.contains(&label) {
                return false;
            }
            let Some(target_block) = graph.label_block(label) else {
                return false;
            };
            let mut found = false;
            for &(source, target, location) in &self.folded_predecessor_sources {
                if target != label {
                    continue;
                }
                found = true;
                // compile_while records both boundaries; compile_jump_if's exclusion
                // marker records neither. The aggregate folded-target set conflates them.
                if self
                    .exception_owner_provenance
                    .source_boundaries
                    .contains(&source)
                    || self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(&source)
                {
                    return false;
                }
                let Some(source_block) = graph.label_block(source) else {
                    return false;
                };
                if positions[&source_block] >= positions[&target_block] {
                    return false;
                }
                let items = graph[source_block].items();
                let Some(source_position) = items
                    .iter()
                    .position(|item| matches!(item, Item::Label(entry) if *entry == source))
                else {
                    return false;
                };
                // The producer emits this NOP immediately after the recorded label.
                // Reject a retired marker or a label moved onto other instructions.
                let Some(Item::Instruction(marker)) = items.get(source_position + 1) else {
                    return false;
                };
                if location.line < 0
                    || *marker != Instruction::new(NOP, Operand::Value(0), location, Some(0))
                {
                    return false;
                }
            }
            found
        };

        // Exact original arrays have only head labels and no erased records.
        // Folded-prefix admission below relaxes only their instruction-slot count.
        // Reject only concrete regions touching selected arrays: coroutine scope
        // fallback and unrelated SEND cleanup regions do not split these arrays.
        let intact = |block: BasicBlockId, require_original_head: bool, folded_prefix: bool| {
            // A folded conditional predecessor does not change an intact located
            // return array. A separate original match-end alias certifies its head;
            // all four slots keep their own lines, so the removed edge cannot affect
            // no-line copying or later exit-location duplication.
            let located_match_tuple = original_local_tuple_return(graph[block].items()).is_some()
                && graph[block].items().iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.original_match_join_boundaries.contains(label)
                            && !self.folded_original_predecessor_targets.contains(label))
                })
                && graph[block].items().iter().all(|item| match item {
                    Item::Instruction(instruction) => {
                        instruction.local_policy.hidden_exception_owner_slots() == 0
                            && instruction.no_location_policy == super::super::ir::NoLocationPolicy::default()
                            && instruction.exception_policy() == super::super::ExceptionPolicy::default()
                    }
                    Item::Label(_) => true,
                });
            let mut seen_instruction = false;
            let mut original_head = false;
            for item in graph[block].items() {
                match item {
                    Item::Label(label) => {
                        if seen_instruction
                            || self.original_nop_targets.contains(label)
                            || (self.folded_original_predecessor_targets.contains(label)
                                && !(located_match_tuple && is_folded_condition_alias(*label)))
                            || self.folded_condition_exit_targets.contains(label)
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
                                .setups
                                .iter()
                                .any(|(entry, _)| entry == label)
                            || self
                                .exception_owner_provenance
                                .normal_try_exits
                                .iter()
                                .any(|(entry, _)| entry == label)
                            || self
                                .exception_regions
                                .iter()
                                .any(|region| region.target == *label)
                        {
                            return false;
                        }
                        original_head |= self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                            && !self.borrow_block_aliases.contains(label);
                    }
                    Item::Instruction(instruction) => {
                        seen_instruction = true;
                        if (!folded_prefix && instruction.pre_fold_extra_instructions() != 0)
                            || instruction.fusion_policy.is_converted_pop_block()
                            || instruction.fusion_policy.is_authored_break()
                        {
                            return false;
                        }
                    }
                }
            }
            let (start, end) = ranges[&block];
            (!require_original_head || original_head)
                && seen_instruction
                && self.exception_regions.iter().all(|region| {
                    region.is_scope_fallback
                        || match (offsets.get(&region.start), offsets.get(&region.end)) {
                            (Some(&region_start), Some(&region_end)) => {
                                end <= region_start || region_end <= start
                            }
                            _ => false,
                        }
                })
        };
        // The producer records a shared nested-if join before jump threading.
        // Reuse this exact one-slot certificate for both discovery and closure.
        let is_shared_join = |block| {
            graph[block].last_instruction().is_some_and(|jump| {
                jump.location == SourceLocation::NONE
                    && jump.no_location_policy.preserves_location()
                    && !jump.no_location_policy.is_artificial_eval_break()
                    && !jump.jump_policy.is_interruptible()
                    && jump.local_policy.hidden_exception_owner_slots() == 0
                    && jump.exception_policy() == super::super::ExceptionPolicy::default()
                    && graph[block].items().iter().any(|item| {
                        matches!(item, Item::Label(label)
                            if self.nested_if_unlocated_join_blocks.contains(label))
                    })
                    && graph[block]
                        .items()
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count()
                        == 1
            })
        };
        // A case-success jump emitted at an existing target is an original
        // unlocated array even when its nested body did not predict an empty join.
        // Keep the deferred-removal guard below: only original-empty joins own
        // the prediction consumed when their slots enter the complete copy plan.
        let is_match_success_join = |block| {
            graph[block].last_instruction().is_some_and(|jump| {
                jump.opcode == JUMP_FORWARD
                    && jump.jump_policy.is_original_match_success()
                    && jump.no_location_policy.is_artificial_eval_break()
                    && jump.local_policy.hidden_exception_owner_slots() == 0
                    && jump.exception_policy() == super::super::ExceptionPolicy::default()
                    && graph[block]
                        .items()
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count()
                        == 1
            })
        };
        // A populated match arm can lack a source boundary (for example, after
        // comprehension restoration). Four persistent, located work slots keep
        // it too large for copying before and after its direct tuple-return copy.
        // Such an arm cannot affect the interacting arrays and keeps the generic
        // copier. Preserve every original-slot and exception-region exclusion;
        // only the original-head requirement is unnecessary for this size proof.
        let is_opaque_match_source = |block| {
            let items = graph[block].items();
            let Some((Item::Instruction(jump), prefix)) = items.split_last() else {
                return false;
            };
            if !intact(block, false, false)
                || jump.opcode != JUMP_FORWARD
                || !jump.jump_policy.is_original_match_success()
                || jump.no_location_policy != super::super::ir::NoLocationPolicy::default()
                || jump.local_policy.hidden_exception_owner_slots() != 0
                || jump.exception_policy() != super::super::ExceptionPolicy::default()
            {
                return false;
            }
            let mut persistent_slots = 0;
            for item in prefix {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                if instruction.location.line < 0
                    || instruction.no_location_policy
                        != super::super::ir::NoLocationPolicy::default()
                    || instruction.local_policy.hidden_exception_owner_slots() != 0
                    || instruction.exception_policy() != super::super::ExceptionPolicy::default()
                {
                    return false;
                }
                match instruction.opcode {
                    LOAD_GLOBAL | BUILD_LIST | STORE_FAST | LOAD_CONST => persistent_slots += 1,
                    SWAP | NOT_TAKEN => {}
                    _ => return false,
                }
            }
            persistent_slots >= 4
        };
        // A direct case-tail exit with no unconditional incoming edge cannot be
        // copied into any scheduled source. Its independent copy of the immutable,
        // fully located tuple return commutes with the closed original-array plan.
        // Keep it on the generic path, including global predecessor accounting.
        let is_independent_match_tail = |block, target| {
            let Some((Item::Instruction(jump), head)) = graph[block].items().split_last() else {
                return false;
            };
            let Some(end) = jump.forward_target() else {
                return false;
            };
            if self.conditional_threading_targets.is_empty()
                || !head.iter().all(|item| {
                    matches!(item, Item::Label(label)
                    if !self.borrow_block_aliases.contains(label))
                })
                || !head.iter().any(|item| {
                    matches!(item, Item::Label(label)
                    if self.conditional_threading_targets.contains_key(label))
                })
                || !intact(block, false, false)
                || graph.label_block(end) != Some(target)
                || !self
                    .exception_owner_provenance
                    .original_match_join_boundaries
                    .contains(&end)
                || !intact(target, false, false)
                || original_local_tuple_return(graph[target].items()).is_none()
                || self.return_array_has_rewritten_items(graph[target].items())
            {
                return false;
            }
            let mut expected = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(end),
                SourceLocation::NONE,
                jump.depth_after(),
            );
            expected.jump_policy.mark_interruptible();
            *jump == expected && !graph.iter_items().any(|item| matches!(item,
                Item::Instruction(incoming) if is_unconditional_jump(incoming.opcode)
                    && matches!(incoming.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if graph.label_block(label) == Some(block))))
        };
        // These sources cannot become unconditional copy targets. Leave their
        // independent tuple copy, including borrowing and defer markers, generic.
        // In particular, a merged loop cleanup is not one native original array.
        let is_independent_case_source = |block, target| {
            use crate::target::opcodes::{
                COPY, END_ASYNC_FOR, END_FOR, FOR_ITER, MATCH_CLASS, POP_ITER, POP_JUMP_IF_NONE,
                POP_JUMP_IF_TRUE, POP_TOP, RESUME, RETURN_GENERATOR,
            };

            if self.python_version < crate::PythonVersion::Py314
                || !self.exception_regions.is_empty()
                || !intact(block, false, false)
                || !intact(target, false, false)
            {
                return false;
            }
            let Some((Item::Instruction(jump), prefix)) = graph[block].items().split_last() else {
                return false;
            };
            let Some(end) = jump.forward_target() else {
                return false;
            };
            if graph.label_block(end) != Some(target)
                || !self
                    .exception_owner_provenance
                    .original_match_join_boundaries
                    .contains(&end)
                || original_local_tuple_return(graph[target].items()).is_none()
                || self.return_array_has_rewritten_items(graph[target].items())
                || jump.location.line < 0
            {
                return false;
            }
            let mut expected_jump =
                Instruction::new(JUMP_FORWARD, Operand::Forward(end), jump.location, Some(0));
            expected_jump.jump_policy.mark_original_match_success();
            expected_jump.jump_policy.mark_interruptible();

            if let [
                head @ ..,
                Item::Instruction(end_for),
                Item::Instruction(pop_iter),
            ] = prefix
                && !head.is_empty()
                && head.iter().all(|item| {
                    matches!(item, Item::Label(label)
                    if !self.borrow_block_aliases.contains(label))
                })
            {
                let iterator_depth =
                    1 + u32::from(self.python_version == crate::PythonVersion::Py315);
                expected_jump.jump_policy.mark_borrow_empty_block();
                if jump.jump_policy.defers_redundant_removal() {
                    expected_jump.jump_policy.defer_removal();
                }
                if !intact(block, true, false)
                    || *jump != expected_jump
                    || *end_for
                        != Instruction::new(
                            END_FOR,
                            Operand::Value(0),
                            jump.location,
                            Some(iterator_depth),
                        )
                    || *pop_iter
                        != Instruction::new(POP_ITER, Operand::Value(0), jump.location, Some(0))
                    || positions[&block].checked_sub(1).is_none_or(|index| {
                        super::super::block_has_fallthrough(graph[order[index]].items())
                    })
                {
                    return false;
                }
                // This sole-entry check also excludes every unconditional input.
                let mut exhaustion = false;
                for &source in order {
                    for item in graph[source].items() {
                        let Item::Instruction(incoming) = item else {
                            continue;
                        };
                        let (Operand::Forward(label) | Operand::Backward(label)) =
                            incoming.operand()
                        else {
                            continue;
                        };
                        if graph.label_block(label) != Some(block) {
                            continue;
                        }
                        if exhaustion
                            || positions[&source] >= positions[&block]
                            || graph[source].last_instruction() != Some(incoming)
                            || *incoming
                                != Instruction::new(
                                    FOR_ITER,
                                    Operand::Forward(label),
                                    jump.location,
                                    Some(iterator_depth + 1),
                                )
                        {
                            return false;
                        }
                        exhaustion = true;
                    }
                }
                return exhaustion;
            }

            // A true fallthrough body has no addressable entry, including an
            // internal alias, so no jump can address it. Its initial marker
            // belongs to the preceding test. These exact suffixes also exclude
            // the COPY/TO_BOOL target insertion in forward jump threading.
            let [Item::Instruction(marker), work @ ..] = prefix else {
                return false;
            };
            if work.is_empty() || prefix.iter().any(|item| matches!(item, Item::Label(_))) {
                return false;
            }
            let Some(previous) = positions[&block].checked_sub(1).map(|index| order[index]) else {
                return false;
            };
            let [
                ..,
                Item::Instruction(value),
                Item::Instruction(convert),
                Item::Instruction(condition),
            ] = graph[previous].items()
            else {
                return false;
            };
            let Some(failure) = condition.forward_target() else {
                return false;
            };
            let Some(failure_block) = graph.label_block(failure) else {
                return false;
            };
            let Some(depth) = condition.depth_after() else {
                return false;
            };
            if positions[&failure_block] <= positions[&block]
                || condition.location.line < 0
                || *condition
                    != Instruction::new(
                        condition.opcode,
                        condition.operand(),
                        condition.location,
                        Some(depth),
                    )
            {
                return false;
            }
            let class_test = condition.opcode == POP_JUMP_IF_NONE
                && value.opcode == MATCH_CLASS
                && value.value_argument().is_some()
                && *value
                    == Instruction::new(
                        MATCH_CLASS,
                        value.operand(),
                        condition.location,
                        Some(depth),
                    )
                && {
                    // Literal class-keyword records can precede this COPY. They
                    // belong to the untouched test, not to the opaque body.
                    let mut expected_copy = Instruction::new(
                        COPY,
                        Operand::Value(1),
                        condition.location,
                        Some(depth + 1),
                    );
                    expected_copy.local_policy.add_hidden_exception_owner_slots(
                        convert.local_policy.hidden_exception_owner_slots(),
                    );
                    *convert == expected_copy
                };
            let truth_test = matches!(condition.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                && value.opcode == LOAD_FAST
                && value.value_argument().is_some()
                && *value
                    == Instruction::new(
                        LOAD_FAST,
                        value.operand(),
                        condition.location,
                        Some(depth + 1),
                    )
                && *convert
                    == Instruction::new(
                        TO_BOOL,
                        Operand::Value(0),
                        condition.location,
                        Some(depth + 1),
                    );
            if !class_test && !truth_test {
                return false;
            }
            let mut expected_marker = Instruction::new(
                NOT_TAKEN,
                Operand::Value(0),
                condition.location,
                Some(depth),
            );
            if truth_test {
                expected_marker
                    .set_exception_policy(super::super::ExceptionPolicy::NoneDynamicCondition);
            }
            if *marker != expected_marker {
                return false;
            }
            if jump.jump_policy.defers_redundant_removal() {
                expected_jump.jump_policy.defer_removal();
            }
            if *jump != expected_jump {
                return false;
            }
            work.iter().enumerate().all(|(index, item)| {
                let Item::Instruction(instruction) = item else {
                    return false;
                };
                if instruction.value_argument().is_none()
                    || instruction.location.line < 0
                    || super::super::ends_scope(instruction.opcode)
                    || matches!(
                        instruction.opcode,
                        NOT_TAKEN
                            | YIELD_VALUE
                            | RETURN_GENERATOR
                            | RESUME
                            | END_FOR
                            | POP_ITER
                            | END_ASYNC_FOR
                    )
                {
                    return false;
                }
                let mut expected = Instruction::new(
                    instruction.opcode,
                    instruction.operand(),
                    instruction.location,
                    instruction.depth_after(),
                );
                if instruction.opcode == LOAD_FAST {
                    expected.local_policy = super::super::ir::LocalPolicy::for_load(
                        instruction.local_policy.force_owned_load(),
                        instruction.local_policy.strict_owned_load(),
                    );
                }
                if instruction.fusion_policy.location_from_next() {
                    if instruction.opcode != POP_TOP
                        || !matches!(work.get(index + 1), Some(Item::Instruction(donor))
                            if donor.location.line >= 0 && donor.location == instruction.location)
                    {
                        return false;
                    }
                    expected.fusion_policy.mark_location_from_next();
                }
                *instruction == expected
            })
        };
        // A populated predecessor cannot itself become a small tuple exit: its
        // replaced jump remains a NOP beside the four return slots. A located
        // work prefix also excludes no-line copying. Preserve its entry and owner
        // guards while allowing recorded folds in that prefix. The plan below
        // treats its width as a lower bound and verifies it is never a copy target.
        let is_opaque_folded_source = |block| {
            let items = graph[block].items();
            let Some((Item::Instruction(jump), prefix)) = items.split_last() else {
                return false;
            };
            if !intact(block, true, true)
                || jump.opcode != JUMP_FORWARD
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
                || jump.no_location_policy != super::super::ir::NoLocationPolicy::default()
                || jump.exception_policy() != super::super::ExceptionPolicy::default()
            {
                return false;
            }
            let mut located_work = false;
            for item in prefix {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                if instruction.no_location_policy != super::super::ir::NoLocationPolicy::default()
                    || instruction.exception_policy() != super::super::ExceptionPolicy::default()
                    || (instruction.local_policy.hidden_exception_owner_slots() != 0
                        && (instruction.opcode != LOAD_CONST
                            || instruction.local_policy.hidden_exception_owner_slots()
                                != u16::from(instruction.pre_fold_extra_instructions())))
                {
                    return false;
                }
                located_work |= instruction.location.line >= 0
                    && !matches!(instruction.opcode, NOP | NOT_TAKEN);
            }
            located_work
        };
        // Unlabeled comparison bridges can join the model only as observers:
        // the complete schedule must prove that neither source nor target is copied.
        let is_comparison_observer = |block: BasicBlockId, target: BasicBlockId| {
            use crate::target::opcodes::{LOAD_SMALL_INT, POP_JUMP_IF_FALSE, POP_TOP};

            if self
                .exception_owner_provenance
                .original_comparison_fallthroughs
                .is_empty()
            {
                return false;
            }
            let [Item::Instruction(marker), Item::Instruction(bridge)] = graph[block].items()
            else {
                return false;
            };
            if !intact(block, false, false) || !intact(target, true, false) {
                return false;
            }
            let target_instructions = graph[target]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let Some((jump, prefix)) = target_instructions.split_last() else {
                return false;
            };
            // Persistent located work prevents no-line copying. Its replaced jump
            // plus the eventual four-slot return also exceeds MAX_COPY_SIZE.
            if jump.opcode != JUMP_FORWARD
                || !matches!(prefix, [value] | [value, _] if matches!(value.opcode, LOAD_CONST | LOAD_SMALL_INT))
                || (prefix.len() == 2 && prefix[1].opcode != STORE_FAST)
                || prefix.iter().any(|instruction| {
                    instruction.location.line < 0
                        || instruction.pre_fold_extra_instructions() != 0
                        || instruction.local_policy.hidden_exception_owner_slots() != 0
                        || instruction.no_location_policy
                            != super::super::ir::NoLocationPolicy::default()
                        || instruction.exception_policy()
                            != super::super::ExceptionPolicy::default()
                })
            {
                return false;
            }
            self.exception_owner_provenance.original_comparison_fallthroughs.iter().any(
                |(cleanup, end, [failure, condition, original_marker, original_bridge, pop, cleanup_jump])| {
                    if marker != original_marker || bridge != original_bridge
                        || graph.label_block(*end) != Some(target)
                        || !matches!(
                            (bridge.jump_policy.inline_small_exit(), bridge.jump_policy.retained_nop()),
                            (false, super::super::ir::RetainedJumpNop::None)
                                | (true, super::super::ir::RetainedJumpNop::Always)
                        )
                        || bridge.jump_policy.is_interruptible()
                        || marker.opcode != NOT_TAKEN
                        || condition.opcode != POP_JUMP_IF_FALSE
                        || failure.opcode != POP_JUMP_IF_FALSE
                        || failure.operand() != Operand::Forward(*cleanup)
                        || pop.opcode != POP_TOP
                        || cleanup_jump.operand() != condition.operand()
                        || [failure, condition, marker, bridge, pop, cleanup_jump].into_iter().any(|instruction| {
                            instruction.pre_fold_extra_instructions() != 0
                                || instruction.local_policy.hidden_exception_owner_slots() != 0
                                || instruction.no_location_policy != super::super::ir::NoLocationPolicy::default()
                                || instruction.exception_policy() != super::super::ExceptionPolicy::default()
                        })
                    {
                        return false;
                    }
                    let position = positions[&block];
                    let Some(previous) = position.checked_sub(1).and_then(|i| order.get(i)) else {
                        return false;
                    };
                    let Some(cleanup_block) = graph.label_block(*cleanup) else {
                        return false;
                    };
                    if graph[*previous].last_instruction() != Some(condition)
                        || order.get(position + 1) != Some(&cleanup_block)
                        || order.get(position + 2) != Some(&target)
                        || !intact(cleanup_block, false, false)
                        || !graph[cleanup_block].items().iter().filter_map(|item| match item {
                            Item::Instruction(instruction) => Some(instruction),
                            Item::Label(_) => None,
                        }).eq([pop, cleanup_jump])
                    {
                        return false;
                    }
                    // The original first comparison must still keep POP_TOP live;
                    // otherwise early cleanup removal could erase this boundary.
                    let failures = order.iter().filter(|&&source| {
                        positions[&source] < positions[previous]
                            && graph[source].last_instruction() == Some(failure)
                    }).count();
                    failures == 1
                        && graph.iter_items().filter(|item| **item == Item::Instruction(*failure)).count() == 1
                        && graph.iter_items().filter(|item| matches!(item,
                            Item::Instruction(instruction)
                                if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                                    if graph.label_block(label) == Some(cleanup_block))
                        )).count() == 1
                },
            )
        };
        let mut returns = FxHashMap::default();
        let mut match_head_returns = FxHashSet::default();
        for &block in order {
            if !intact(block, true, false) {
                // Live case-success edges establish the original match end without
                // the generic source-boundary tag. Admit only its exact tuple array;
                // seed discovery below still requires the shared-join certificate.
                if original_local_tuple_return(graph[block].items()).is_none()
                    || !intact(block, false, false)
                    || !graph[block].items().iter().any(|item| {
                        matches!(item,
                        Item::Label(label) if self.exception_owner_provenance
                            .original_match_join_boundaries.contains(label))
                    })
                    || self.return_array_has_rewritten_items(graph[block].items())
                {
                    continue;
                }
                match_head_returns.insert(block);
            }
            if let Some(tail) = original_local_tuple_return(graph[block].items())
                .map(|located| OriginalTail::LocalTupleReturn { located })
                .or_else(|| original_local_subscript_return(graph[block].items()))
            {
                returns.insert(block, tail);
            }
        }
        if returns.is_empty() {
            return;
        }
        // A shared case-success array keeps its first NOP unlocated after copying
        // a four-slot return. Authenticate two inputs that survive the early copy
        // rounds: a runtime conditional and its distinct, located store fallthrough.
        // A current predecessor count alone cannot establish their later survival.
        let mut shared_match_joins = FxHashMap::default();
        for &block in order {
            if !is_match_success_join(block)
                || !intact(block, true, false)
                || graph[block].items().iter().any(|item| {
                    matches!(item,
                    Item::Label(label) if self.borrow_block_aliases.contains(label))
                })
            {
                continue;
            }
            let jump = graph[block].last_instruction().expect("match-success jump");
            let Some(end) = jump.forward_target() else {
                continue;
            };
            let Some(target) = graph.label_block(end) else {
                continue;
            };
            if !returns.contains_key(&target)
                || original_local_tuple_return(graph[target].items()).is_none()
                || !self
                    .exception_owner_provenance
                    .original_match_join_boundaries
                    .contains(&end)
                || self.return_array_has_rewritten_items(graph[target].items())
                || jump.jump_policy.preserves_nop_after_threading()
                || jump.no_location_policy.preserves_location()
                || jump.pre_fold_extra_instructions() != 0
            {
                continue;
            }
            let Some(start) = positions[&block].checked_sub(3) else {
                continue;
            };
            let [condition_block, marker_block, body, _] = order[start..=positions[&block]] else {
                unreachable!("three preceding blocks")
            };
            if !intact(condition_block, true, false) || !intact(body, true, false) {
                continue;
            }
            let [
                ..,
                Item::Instruction(load),
                Item::Instruction(convert),
                Item::Instruction(condition),
            ] = graph[condition_block].items()
            else {
                continue;
            };
            let [Item::Instruction(marker)] = graph[marker_block].items() else {
                continue;
            };
            let Some(store) = graph[body].last_instruction() else {
                continue;
            };
            let mut expected_marker = Instruction::new(
                NOT_TAKEN,
                Operand::Value(0),
                condition.location,
                condition.depth_after(),
            );
            expected_marker
                .set_exception_policy(super::super::ExceptionPolicy::NoneDynamicCondition);
            if load.opcode != LOAD_FAST
                || load.value_argument().is_none()
                || convert.opcode != TO_BOOL
                || convert.value_argument() != Some(0)
                || condition.opcode != POP_JUMP_IF_FALSE
                || !matches!(condition.operand(), Operand::Forward(label)
                    if graph.label_block(label) == Some(block))
                || *marker != expected_marker
                || store.opcode != STORE_FAST
                || store.local_policy.may_store_null()
                || [condition_block, body].into_iter().any(|source| {
                    graph[source].items().iter().any(|item| match item {
                        Item::Label(label) => self.borrow_block_aliases.contains(label),
                        Item::Instruction(instruction) => {
                            instruction.location.line < 0
                                || *instruction
                                    != Instruction::new(
                                        instruction.opcode,
                                        instruction.operand(),
                                        instruction.location,
                                        instruction.depth_after(),
                                    )
                                || (instruction != condition
                                    && (instruction.value_argument().is_none()
                                        || super::super::ends_scope(instruction.opcode)
                                        || matches!(
                                            instruction.opcode,
                                            NOP | NOT_TAKEN | SWAP | YIELD_VALUE
                                        )))
                        }
                    })
                })
            {
                continue;
            }
            shared_match_joins.insert(block, (target, condition_block, body));
        }
        // Limit this plan to tuple tails reached by a certified original join,
        // directly or through one certified shared nested-if array. The full
        // predecessor closure below still verifies every original copy slot;
        // unrelated tuple exits in the same compilation unit keep their own path.
        returns.retain(|block, _| {
            shared_match_joins
                .values()
                .any(|(target, _, _)| target == block)
                || (!match_head_returns.contains(block) && graph.iter_items().any(|item| {
                    matches!(item, Item::Instruction(instruction)
                    if instruction.opcode == JUMP_FORWARD
                        && instruction.jump_policy.is_original_empty_join()
                        && matches!(instruction.operand(), Operand::Forward(label)
                            if graph.label_block(label).is_some_and(|target| {
                                target == *block
                                    || (intact(target, true, false)
                                        && is_shared_join(target)
                                        && graph[target].last_instruction().is_some_and(|bridge| {
                                            bridge.opcode == JUMP_FORWARD
                                                && matches!(bridge.operand(), Operand::Forward(end)
                                                    if graph.label_block(end) == Some(*block))
                                        }))
                            })))
                }))
        });
        if returns.is_empty() {
            return;
        }
        let mut selected = returns.keys().copied().collect::<FxHashSet<_>>();
        let mut sources = FxHashSet::default();
        let mut joins = FxHashSet::default();
        let mut shared_joins = FxHashSet::default();
        let mut opaque_sources = FxHashSet::default();
        let mut observer_sources = FxHashSet::default();
        // Close over every original unconditional predecessor, including direct
        // tuple entries and predecessors of ordinary source arrays. Omission of
        // even a source whose final copy fails could change a later target's size.
        loop {
            let before = selected.len();
            for &block in order {
                let Some(jump) = graph[block].last_instruction() else {
                    continue;
                };
                if !is_unconditional_jump(jump.opcode) {
                    continue;
                }
                let (Operand::Forward(label) | Operand::Backward(label)) = jump.operand() else {
                    continue;
                };
                let Some(target) = graph.label_block(label) else {
                    continue;
                };
                if !selected.contains(&target) {
                    continue;
                }
                if returns.contains_key(&target)
                    && jump.forward_target().is_some()
                    && positions[&block] < positions[&target]
                    && ((!intact(block, true, false) && is_opaque_match_source(block))
                        || is_independent_match_tail(block, target)
                        || is_independent_case_source(block, target))
                {
                    // No head is synthesized and no copy step is scheduled here.
                    // Global predecessor accounting still includes this direct edge.
                    continue;
                }
                // A partial nested if can contribute another original empty array
                // to the same closed graph. Its conditional predecessors keep the
                // original jump unlocated, but private copies inherit their own lines.
                // Require the producer's exact head, not an arbitrary unlocated jump.
                let shared_join = is_shared_join(block);
                let opaque_source = !intact(block, true, false) && is_opaque_folded_source(block);
                let observer = is_comparison_observer(block, target);
                if (!intact(block, true, false) && !opaque_source && !observer)
                    || jump.opcode != JUMP_FORWARD
                    || jump.forward_target().is_none()
                    || positions[&block] >= positions[&target]
                    || (!jump.jump_policy.inline_small_exit() && !observer)
                    || (jump.jump_policy.always_retains_nop() && !observer)
                    || jump.jump_policy.borrow_empty_block()
                    || (jump.jump_policy.defers_redundant_removal()
                        && !shared_match_joins.contains_key(&block)
                        && !(jump.jump_policy.is_original_empty_join()
                            && jump.jump_policy.preserves_nop_after_threading()))
                    || (jump.no_location_policy.preserves_location() && !shared_join)
                {
                    return;
                }
                if observer {
                    observer_sources.insert(block);
                }
                if shared_join {
                    shared_joins.insert(block);
                } else if jump.jump_policy.is_original_empty_join() || is_match_success_join(block)
                {
                    if graph[block]
                        .items()
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count()
                        != 1
                    {
                        return;
                    }
                    joins.insert(block);
                }
                if opaque_source {
                    opaque_sources.insert(block);
                }
                sources.insert(block);
                selected.insert(block);
            }
            if before == selected.len() {
                break;
            }
        }
        if joins.is_empty()
            || shared_match_joins
                .values()
                .any(|(_, condition, body)| selected.contains(condition) || selected.contains(body))
        {
            return;
        }
        // The producer also certifies a sole conditional predecessor after a
        // terminating guard. That join has a real inherited test location, so
        // its original-empty bit alone cannot authorize resetting the line.
        if graph.iter_items().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if !is_unconditional_jump(instruction.opcode)
                    && matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if graph.label_block(label).is_some_and(|target| joins.contains(&target)
                            && !shared_match_joins.contains_key(&target))))
        }) {
            return;
        }
        let mut nodes = Vec::new();
        for &block in order {
            if let Some(&tail) = returns.get(&block) {
                nodes.push(OriginalNode {
                    id: block,
                    prefix: Vec::new(),
                    tail,
                });
            } else if observer_sources.contains(&block) {
                let bridge = graph[block]
                    .last_instruction()
                    .expect("witnessed comparison bridge");
                let Some(end) = bridge.forward_target() else {
                    unreachable!()
                };
                nodes.push(OriginalNode {
                    id: block,
                    prefix: Vec::new(),
                    tail: OriginalTail::Jump {
                        target: graph.label_block(end).expect("original comparison end"),
                        located: false,
                        interruptible: false,
                    },
                });
            } else if sources.contains(&block) {
                let instructions = graph[block]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>();
                let (jump, prefix) = instructions
                    .split_last()
                    .expect("certified original source");
                let Some(label) = jump.forward_target() else {
                    unreachable!()
                };
                let target = graph.label_block(label).expect("closed original target");
                let is_join = joins.contains(&block) || shared_joins.contains(&block);
                // An already unlocated ordinary array must have the same eligibility
                // in the executor. Only certified empty joins gain that permission.
                if !is_join
                    && instructions
                        .iter()
                        .all(|instruction| instruction.location.line < 0)
                    && !jump.no_location_policy.allows_block_inlining()
                {
                    return;
                }
                nodes.push(OriginalNode {
                    id: block,
                    prefix: prefix
                        .iter()
                        .flat_map(|instruction| {
                            // CPython copies before folding LOAD_GLOBAL/PUSH_NULL.
                            // The fused operand preserves the second original slot,
                            // whose callee location is the same as LOAD_GLOBAL's.
                            let fused_push_null = instruction.opcode == LOAD_GLOBAL
                                && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0);
                            std::iter::repeat_n(
                                PrefixSlot {
                                    is_nop: instruction.opcode == NOP,
                                    located: instruction.location.line >= 0,
                                },
                                1 + usize::from(fused_push_null),
                            )
                        })
                        .collect(),
                    tail: OriginalTail::Jump {
                        target,
                        located: !is_join && jump.location.line >= 0,
                        interruptible: jump.jump_policy.is_interruptible(),
                    },
                });
            }
        }
        let Some(result) = plan_normal_try_copies(&nodes) else {
            return;
        };
        if result.steps.is_empty()
            || result.steps.iter().any(|step| {
                opaque_sources.contains(&step.target)
                    || observer_sources.contains(&step.source)
                    || observer_sources.contains(&step.target)
            })
        {
            return;
        }
        // Head-only admission above preserves every entry label. No POP_BLOCK,
        // synthetic instruction, owner hole, or alternate internal label is added.
        let labels = selected
            .iter()
            .filter(|block| !observer_sources.contains(block))
            .map(|&block| {
                (
                    block,
                    graph[block].first_label().expect("original head label"),
                )
            })
            .collect::<FxHashMap<_, _>>();
        let copy_blocks = order
            .iter()
            .copied()
            .filter(|block| sources.contains(block) && !observer_sources.contains(block))
            .map(|block| labels[&block])
            .collect();
        let copy_steps = result
            .steps
            .iter()
            .map(|step| {
                (
                    labels[&step.source],
                    labels[&step.target],
                    step.target_width,
                )
            })
            .collect();
        let ordered_joins = order
            .iter()
            .copied()
            .filter(|block| joins.contains(block) || shared_joins.contains(block))
            .collect::<Vec<_>>();
        let fallthrough_stores = order
            .windows(2)
            .filter_map(|pair| {
                if !joins.contains(&pair[1]) {
                    return None;
                }
                let predecessor = self.graph[pair[0]].first_label()?;
                let store = *self.graph[pair[0]].last_instruction()?;
                (store.opcode == STORE_FAST
                    && !store.local_policy.may_store_null()
                    && store.location.line >= 0)
                    .then_some((pair[1], (predecessor, store)))
            })
            .collect::<FxHashMap<_, _>>();
        let mut join_origins = FxHashMap::default();
        for block in ordered_joins {
            let jump = self
                .graph
                .instructions_mut(block)
                .next_back()
                .expect("original empty join");
            if jump.jump_policy.defers_redundant_removal() {
                // The complete plan now owns this certified singleton's jump vacancy.
                if shared_match_joins.contains_key(&block) {
                    // This certificate owns an ordinary deferred case-success slot,
                    // with no predicted retained NOP to transfer into the copy plan.
                    jump.jump_policy.resolve_deferred_removal();
                } else {
                    jump.jump_policy.consume_original_join_copy_prediction();
                }
            }
            let origin = fallthrough_stores
                .get(&block)
                .copied()
                .filter(|(_, store)| store.location == jump.location);
            jump.location = SourceLocation::NONE;
            jump.no_location_policy.clear_preserved_location();
            jump.no_location_policy.allow_block_inlining();
            if let Some((predecessor, store)) = origin {
                // This is only an original adjacency record. A successful static
                // store exchange must certify the donor later.
                join_origins.insert(
                    labels[&block],
                    TupleJoinLocationOrigin {
                        predecessor,
                        store,
                        jump: *jump,
                    },
                );
            }
        }
        self.exception_owner_provenance.normal_try_copy_join_origins = join_origins;
        self.exception_owner_provenance
            .normal_try_copy_join_donors
            .clear();
        self.exception_owner_provenance.normal_try_copy_blocks = copy_blocks;
        self.exception_owner_provenance.normal_try_copy_steps = copy_steps;
    }

    /// Recognizes intact compiler-generated ordinary-await handlers inside a case interval.
    fn ordinary_await_cleanup_labels(&self, blocks: &[BasicBlockId]) -> FxHashSet<Label> {
        use crate::target::opcodes::{CLEANUP_THROW, END_SEND, SEND, YIELD_VALUE};

        let mut labels = FxHashSet::default();
        for original in &self.exception_owner_provenance.ordinary_await_cleanups {
            let (yielded, cleanup) = (original.start, original.target);
            let (Some(yield_block), Some(cleanup_block)) = (
                self.graph.label_block(yielded),
                self.graph.label_block(cleanup),
            ) else {
                continue;
            };
            let (Some(yield_position), Some(cleanup_position)) = (
                blocks.iter().position(|block| *block == yield_block),
                blocks.iter().position(|block| *block == cleanup_block),
            ) else {
                continue;
            };
            let mut setups = self
                .exception_owner_provenance
                .setups
                .iter()
                .filter(|(label, _)| *label == yielded || *label == cleanup);
            if setups.next() != Some(&(yielded, None)) || setups.next().is_some() {
                continue;
            }
            let mut regions = self
                .exception_regions
                .iter()
                .filter(|region| region.target == yielded || region.target == cleanup);
            let Some(region) = regions.next() else {
                continue;
            };
            if regions.next().is_some()
                || region.start != original.start
                || region.end != original.end
                || region.target != original.target
                || region.depth != original.depth
                || region.is_scope_fallback != original.is_scope_fallback
                || region.preserve_lasti != original.preserve_lasti
                || region.is_scope_fallback
                || region.preserve_lasti
                || !matches!((region.priority, original.priority),
                    (ExceptionRegionPriority::Resolved { start, end },
                     ExceptionRegionPriority::Resolved { start: original_start, end: original_end })
                        if start == original_start && end == original_end)
            {
                continue;
            }
            let [Item::Label(yield_label), Item::Instruction(yield_value)] =
                self.graph[yield_block].items()
            else {
                continue;
            };
            let [Item::Label(cleanup_label), Item::Instruction(cleanup_throw)] =
                self.graph[cleanup_block].items()
            else {
                continue;
            };
            let Some(end_block) = blocks.get(cleanup_position + 1) else {
                continue;
            };
            let [Item::Label(end), Item::Instruction(end_send), ..] =
                self.graph[*end_block].items()
            else {
                continue;
            };
            if *yield_label != yielded
                || *cleanup_label != cleanup
                || yield_position == 0
                || yield_position + 1 >= cleanup_position
                || self.graph.label_block(region.end) != Some(blocks[yield_position + 1])
                || yield_value.opcode != YIELD_VALUE
                || yield_value.value_argument() != Some(1)
                || cleanup_throw.opcode != CLEANUP_THROW
                || cleanup_throw.value_argument() != Some(0)
                || end_send.opcode != END_SEND
                || end_send.value_argument() != Some(0)
                || yield_value.depth_after() != Some(region.depth)
                || cleanup_throw.depth_after() != Some(region.depth)
                || end_send.depth_after()
                    != region.depth.checked_sub(match self.python_version {
                        crate::PythonVersion::Py312
                        | crate::PythonVersion::Py313
                        | crate::PythonVersion::Py314 => 1,
                        crate::PythonVersion::Py315 => 2,
                    })
                || !self
                    .exception_owner_provenance
                    .source_boundaries
                    .contains(end)
                || !self.graph[blocks[yield_position - 1]]
                    .last_instruction()
                    .is_some_and(|send| {
                        send.opcode == SEND && send.operand() == Operand::Forward(*end)
                    })
                || super::super::block_has_fallthrough(
                    self.graph[blocks[cleanup_position - 1]].items(),
                )
            {
                continue;
            }
            // Exact one-label blocks reject aliases. The paired SEND is the only
            // normal input to these records or their END_SEND continuation.
            if self.graph.iter_items().filter(|item| matches!(item,
                Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if label == yielded || label == cleanup || label == *end)))
                .count() != 1
            {
                continue;
            }
            labels.extend([yielded, cleanup]);
        }
        labels
    }

    /// Recognizes an original unlocated handler cleanup before a live continuation.
    fn empty_try_cleanup_successor(&self, block: BasicBlockId) -> Option<BasicBlockId> {
        use crate::target::opcodes::{
            DELETE_FAST, DELETE_NAME, LOAD_CONST, STORE_FAST, STORE_NAME,
        };

        let graph = &self.graph;
        let items = graph[block].items();
        let head_len = items
            .iter()
            .take_while(|item| matches!(item, Item::Label(_)))
            .count();
        let intact_label = |label: &Label| {
            !self.original_nop_targets.contains(label)
                && !self.folded_original_predecessor_targets.contains(label)
                && !self.folded_condition_exit_targets.contains(label)
                && !self.borrow_block_aliases.contains(label)
                && !self
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .contains_key(label)
                && !self
                    .exception_owner_provenance
                    .omitted_nop_locations
                    .contains_key(label)
                && !self
                    .exception_owner_provenance
                    .setups
                    .iter()
                    .any(|(entry, _)| entry == label)
                && !self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == *label)
        };
        if !items[..head_len].iter().any(|item| {
            matches!(item, Item::Label(label)
            if self.exception_owner_provenance.source_boundaries.contains(label)
                && self.exception_owner_provenance.block_boundaries.contains(label))
        }) || items[..head_len].iter().any(|item| {
            matches!(item, Item::Label(label)
                if !intact_label(label))
        }) {
            return None;
        }
        let records = items[head_len..]
            .iter()
            .map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let pop = match records.as_slice() {
            [pop] if pop.opcode == POP_EXCEPT => *pop,
            [pop, load, store, delete]
                if pop.opcode == POP_EXCEPT
                    && load.opcode == LOAD_CONST
                    && ((store.opcode == STORE_FAST && delete.opcode == DELETE_FAST)
                        || (store.opcode == STORE_NAME && delete.opcode == DELETE_NAME))
                    && store.operand() == delete.operand()
                    && store.depth_after() == pop.depth_after()
                    && delete.depth_after() == pop.depth_after() =>
            {
                *pop
            }
            _ => return None,
        };
        if records.iter().any(|instruction| {
            instruction.location != SourceLocation::NONE
                || instruction.value_argument().is_none()
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.local_policy.hidden_exception_owner_slots() != 0
                || instruction.exception_policy() != super::super::ExceptionPolicy::default()
                || instruction.fusion_policy.is_converted_pop_block()
                || instruction.fusion_policy.is_authored_break()
        }) {
            return None;
        }
        let position = graph
            .order()
            .iter()
            .position(|candidate| *candidate == block)?;
        let tail = *graph.order().get(position + 1)?;
        let (Item::Instruction(jump), labels) = graph[tail].items().split_last()? else {
            return None;
        };
        let mut original_policy = super::super::ir::JumpPolicy::default();
        original_policy.prevent_threading_target();
        if labels.is_empty()
            || labels.iter().any(|item| !matches!(item, Item::Label(_)))
            || jump.opcode != JUMP_FORWARD
            || jump.location != SourceLocation::NONE
            || jump.depth_after() != pop.depth_after()
            || jump.pre_fold_extra_instructions() != 0
            || jump.local_policy.hidden_exception_owner_slots() != 0
            || jump.exception_policy() != super::super::ExceptionPolicy::default()
            || !jump.no_location_policy.allows_block_inlining()
            || jump.jump_policy != original_policy
            || labels.iter().any(|item| matches!(item, Item::Label(label)
                if !intact_label(label)
                    || self.exception_owner_provenance.source_boundaries.contains(label)
                    || self.exception_owner_provenance.block_boundaries.contains(label)
                    || self.preserved_block_boundaries.contains(label)
                    || graph.iter_items().any(|item| matches!(item, Item::Instruction(instruction)
                        if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                            if target == *label)))))
        {
            return None;
        }
        let target = jump.forward_target()?;
        graph.label_block(target)
    }

    /// Restores original empty-try owners before a live prefix or shared finalizer exit.
    ///
    /// An unlocated jump needs either the shared finalizer's enclosing owners or
    /// an intact SETUP donor that cannot expose a higher-priority exception region.
    fn restore_empty_try_jump_owners(&mut self) {
        let graph = &self.graph;
        if !self
            .exception_owner_provenance
            .normal_try_jump_owners
            .iter()
            .any(|(&(start, _), owners)| {
                !owners.is_empty()
                    && graph.label_block(start).is_some_and(|block| {
                        matches!(graph[block].items().last(), Some(Item::Instruction(jump))
                            if jump.opcode == JUMP_FORWARD
                                && jump.exception_policy()
                                    == super::super::ExceptionPolicy::EXCLUDE_ALWAYS)
                    })
            })
        {
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
        let mut selected = Vec::new();
        let mut jump_owners: Vec<_> = self
            .exception_owner_provenance
            .normal_try_jump_owners
            .iter()
            .collect();
        jump_owners.sort_by_key(|&(&(start, end), _)| (start.as_u32(), end.as_u32()));
        for (&(start, end), owners) in jump_owners {
            let (Some(&start_offset), Some(&end_offset), Some(block)) = (
                offsets.get(&start),
                offsets.get(&end),
                graph.label_block(start),
            ) else {
                continue;
            };
            let Some((Item::Instruction(jump), head)) = graph[block].items().split_last() else {
                continue;
            };
            let Some(target) = jump.forward_target() else {
                continue;
            };
            if owners.is_empty()
                || end_offset != start_offset + 1
                || instructions.get(start_offset).copied() != Some(jump)
                || head.is_empty()
                || head.iter().any(|item| !matches!(item, Item::Label(_)))
                || jump.opcode != JUMP_FORWARD
                || jump.exception_policy() != super::super::ExceptionPolicy::EXCLUDE_ALWAYS
                || jump.pre_fold_extra_instructions() != 0
                || jump.local_policy.hidden_exception_owner_slots() != 0
                || jump.fusion_policy.is_converted_pop_block()
                || jump.fusion_policy.is_authored_break()
                || jump.jump_policy.borrow_empty_block()
                || jump.jump_policy.defers_redundant_removal()
                || jump.jump_policy.retained_nop()
                    != super::super::ir::RetainedJumpNop::DirectTarget
                || self
                    .exception_owner_provenance
                    .normal_try_exits
                    .iter()
                    .filter(|(entry, original_target)| {
                        *original_target == target && head.contains(&Item::Label(*entry))
                    })
                    .count()
                    != 1
            {
                continue;
            }
            let Some(mut target_block) = graph.label_block(target) else {
                continue;
            };
            // A shared finalizer RERAISE has the same enclosing owner as the
            // original empty-body exit. Keep that owner available to exit copying.
            let mut target_instructions =
                graph[target_block]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    });
            let shared_reraise = self.python_version >= crate::PythonVersion::Py314
                && target_instructions.next().is_some_and(|instruction| {
                    instruction.opcode == crate::target::opcodes::RERAISE
                        && instruction.value_argument() == Some(0)
                        && instruction.location == SourceLocation::NONE
                        && instruction.exception_policy()
                            == super::super::ExceptionPolicy::default()
                        && instruction.pre_fold_extra_instructions() == 0
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                })
                && target_instructions.next().is_none()
                && owners.iter().all(|owner| {
                    self.exception_regions.iter().any(|region| {
                        region.target == owner.target
                            && region.depth == owner.depth
                            && region.preserve_lasti == owner.preserve_lasti
                            && offsets
                                .get(&region.start)
                                .zip(offsets.get(&region.end))
                                .zip(offsets.get(&target))
                                .is_some_and(|((start, end), target)| {
                                    start <= target && target < end
                                })
                    })
                });
            // Removed assertions can leave the normal jump unlocated until its
            // intact SETUP donates a line. Clearing its exclusion must not activate
            // an inner region that shadows the recorded original owner.
            let empty_setup_donor = self.python_version >= crate::PythonVersion::Py314
                && restored_empty_try_owner_is_unshadowed(
                    owners,
                    &self.exception_regions,
                    &offsets,
                    start_offset,
                )
                && start_offset.checked_sub(1).is_some_and(|previous| {
                    let setup = instructions[previous];
                    setup.opcode == NOP
                        && setup.location.line >= 0
                        && setup
                            .depth_after()
                            .is_some_and(|depth| jump.depth_after() == Some(depth))
                        && setup.jump_policy.preserves_explicit_nop()
                        && setup.exception_policy() == super::super::ExceptionPolicy::EXCLUDE_ALWAYS
                        && setup.pre_fold_extra_instructions() == 0
                        && setup.local_policy.hidden_exception_owner_slots() == 0
                        && offsets.iter().any(|(label, offset)| {
                            *offset == start_offset
                                && self
                                    .exception_owner_provenance
                                    .ordinary_try_setups
                                    .contains_key(label)
                                && self
                                    .exception_owner_provenance
                                    .setups
                                    .contains(&(*label, Some(setup.location)))
                        })
                });
            if jump.location.line < 0 && !shared_reraise && !empty_setup_donor {
                continue;
            }
            // A wholly unlocated handler cleanup is copied before its final jump
            // reaches the same large runtime prefix. Preserve the original source
            // owners until the ordinary copier transports the cleanup's owners.
            if let Some(continuation) = self.empty_try_cleanup_successor(target_block) {
                target_block = continuation;
            }
            let mut prefix = 0;
            for item in graph[target_block].items() {
                match item {
                    Item::Label(label) => {
                        if prefix > 0
                            || self.original_nop_targets.contains(label)
                            || self.folded_original_predecessor_targets.contains(label)
                            || self.folded_condition_exit_targets.contains(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_counts
                                .contains_key(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_locations
                                .contains_key(label)
                        {
                            break;
                        }
                    }
                    Item::Instruction(instruction) => {
                        if instruction.location.line < 0
                            || instruction.opcode == NOP
                            || super::super::ends_scope(instruction.opcode)
                            || instruction.value_argument().is_none()
                            || instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.fusion_policy.is_converted_pop_block()
                        {
                            break;
                        }
                        prefix += 1;
                        if prefix == 5 {
                            break;
                        }
                    }
                }
            }
            // CPython copies at most four original exit records; a located runtime
            // prefix also excludes unlocated-exit copying and jump-to-jump threading.
            if prefix == 5 || shared_reraise {
                selected.push((
                    start_offset,
                    start.as_u32(),
                    end.as_u32(),
                    block,
                    owners.clone(),
                ));
            }
        }
        selected.sort_by_key(|(offset, start, end, _, _)| (*offset, *start, *end));
        for (_, _, _, block, owners) in selected {
            self.graph
                .instructions_mut(block)
                .next_back()
                .expect("original normal jump")
                .set_exception_policy(super::super::ExceptionPolicy::default());
            self.exception_regions.extend(owners);
        }
    }

    /// Keeps an original unlocated discard/`POP_BLOCK`/jump array eligible for copying.
    fn prepare_unlocated_normal_try_discards(&mut self) {
        use super::super::ir::BorrowFallthrough;
        use crate::target::opcodes::{LOAD_FAST, POP_TOP, RETURN_VALUE};

        // A trailing authored Pass can be absent from the array without a head alias.
        if self.exception_owner_provenance.omitted_nop_tail.is_some() {
            return;
        }
        let order = self.graph.order();
        let position = |label| {
            let block = self.graph.label_block(label)?;
            order.iter().position(|candidate| *candidate == block)
        };
        let intact_label = |label| {
            !self.original_nop_targets.contains(&label)
                && !self.folded_original_predecessor_targets.contains(&label)
                && !self.folded_condition_exit_targets.contains(&label)
                && !self.borrow_block_aliases.contains(&label)
                && !self
                    .exception_owner_provenance
                    .fusion_boundaries
                    .contains(&label)
                && !self
                    .exception_owner_provenance
                    .continuation_boundaries
                    .contains(&label)
                && !self
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .contains_key(&label)
                && !self
                    .exception_owner_provenance
                    .omitted_nop_locations
                    .contains_key(&label)
                && !self
                    .exception_owner_provenance
                    .setups
                    .iter()
                    .any(|(entry, _)| *entry == label)
                && !self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == label)
        };
        let selected = self
            .exception_owner_provenance
            .normal_try_exits
            .iter()
            .filter_map(|&(entry, target)| {
                let cleanup_position = position(entry)?;
                let cleanup = order[cleanup_position];
                let discard = *order.get(cleanup_position.checked_sub(1)?)?;
                let target_position = position(target)?;
                let continuation = self.graph[order[target_position]].items();
                let [Item::Label(join), Item::Instruction(pop)] = self.graph[discard].items()
                else {
                    return None;
                };
                let [Item::Label(cleanup_entry), Item::Instruction(jump)] =
                    self.graph[cleanup].items()
                else {
                    return None;
                };
                let depth = pop.depth_after()?;
                let local_return = matches!(continuation,
                    [Item::Label(return_entry), Item::Instruction(load), Item::Instruction(ret)]
                        if depth == 0
                            && *return_entry == target
                            && load.opcode == LOAD_FAST
                            && load.value_argument().is_some()
                            && load.location.line >= 0
                            && ret.location.line >= 0
                            && *load == Instruction::new(
                                LOAD_FAST, load.operand(), load.location, Some(1))
                            && *ret == Instruction::new(
                                RETURN_VALUE, Operand::Value(0), ret.location, Some(0)));
                // The same three original records can leave one live iterator on
                // the stack. The continuation stays separate: either the original
                // common backedge or a located, non-exiting source block follows.
                let iterator_depth = 1 + u32::from(self.python_version == crate::PythonVersion::Py315);
                let loop_continuation = self.python_version >= crate::PythonVersion::Py314
                    && depth == iterator_depth
                    && match continuation {
                        [Item::Label(label), Item::Instruction(backedge)]
                            if *label == target && backedge.opcode == JUMP_BACKWARD =>
                        {
                            backedge.location == SourceLocation::NONE
                                && backedge.depth_after() == Some(depth)
                                && matches!(backedge.operand(), Operand::Backward(header)
                                    if position(header).is_some_and(|head| head < cleanup_position))
                                && self.conditional_threading_targets.get(&target)
                                    == Some(&backedge.location)
                                && backedge.pre_fold_extra_instructions() == 0
                                && backedge.local_policy.hidden_exception_owner_slots() == 0
                                && !backedge.fusion_policy.is_authored_break()
                                && !backedge.fusion_policy.is_converted_pop_block()
                        }
                        [Item::Label(label), Item::Instruction(first), ..]
                            if *label == target
                                && first.location.line >= 0
                                && first.opcode != NOP
                                && !is_unconditional_jump(first.opcode)
                                && !super::super::is_conditional_jump(first.opcode) =>
                        {
                            continuation[1..]
                                .iter()
                                .all(|item| matches!(item, Item::Instruction(instruction)
                                    if !super::super::ends_scope(instruction.opcode)
                                        && instruction.pre_fold_extra_instructions() == 0
                                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                                        && !instruction.fusion_policy.is_converted_pop_block()))
                        }
                        _ => false,
                    };
                let mut expected_jump = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(target),
                    SourceLocation::NONE,
                    Some(depth),
                );
                if loop_continuation
                    && jump.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
                {
                    // A terminating handler marks this original normal exit as a
                    // borrow barrier. It adds no native record; keep the barrier
                    // unchanged when the existing copier transports this suffix.
                    expected_jump
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                }
                if (!local_return && !loop_continuation)
                    || *cleanup_entry != entry
                    || cleanup_position >= target_position
                    || [*join, entry, target]
                        .iter()
                        .any(|label| !intact_label(*label))
                    || ![*join, target].iter().all(|label| {
                        self.exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                            && self
                                .exception_owner_provenance
                                .block_boundaries
                                .contains(label)
                    })
                    || self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(&entry)
                    || self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(&entry)
                    || self.preserved_block_boundaries.contains(&entry)
                    || *pop
                        != Instruction::new(
                            POP_TOP,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(depth),
                        )
                    || *jump != expected_jump
                {
                    return None;
                }
                // The cleanup is a range split inside the discard's original array.
                // An independently targeted cleanup would instead have its own copy order.
                let mut incoming = None;
                for (source, block) in self.graph.iter() {
                    for item in block.items() {
                        let Item::Instruction(instruction) = item else {
                            continue;
                        };
                        let (Operand::Forward(label) | Operand::Backward(label)) =
                            instruction.operand()
                        else {
                            continue;
                        };
                        if label == entry {
                            return None;
                        }
                        if label == *join {
                            let source_position =
                                order.iter().position(|candidate| *candidate == source)?;
                            let mut expected = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(*join),
                                instruction.location,
                                Some(depth + 1),
                            );
                            // IfExp edges already carry the split-copy policy. A
                            // plain fast-call edge is admitted only by this entire
                            // original-array and ordinary-owner certificate.
                            let plain_loop_edge = loop_continuation && *instruction == expected;
                            expected.no_location_policy.allow_split_return_exit_copy();
                            if incoming.is_some()
                                || source_position >= cleanup_position - 1
                                || (*instruction != expected && !plain_loop_edge)
                            {
                                return None;
                            }
                            incoming = Some((source_position, plain_loop_edge.then_some(source)));
                        }
                    }
                }
                let (source_position, plain_source) = incoming?;
                let mut owner = false;
                for region in &self.exception_regions {
                    let start = position(region.start)?;
                    let end = position(region.end)?;
                    if start > end || [*join, entry, target].contains(&region.target) {
                        return None;
                    }
                    if region.end == entry {
                        if owner
                            || region.is_scope_fallback
                            || region.depth != depth
                            || region.preserve_lasti
                            || !(start..end).contains(&source_position)
                            || !(start..end).contains(&(cleanup_position - 1))
                            || self
                                .exception_owner_provenance
                                .ordinary_try_setups
                                .get(&region.start)
                                != Some(&region.target)
                        {
                            return None;
                        }
                        owner = true;
                    } else if [
                        source_position,
                        cleanup_position - 1,
                        cleanup_position,
                        target_position,
                    ]
                    .iter()
                    .any(|position| (start..end).contains(position))
                        || [*join, entry, target]
                            .iter()
                            .any(|label| region.start == *label || region.end == *label)
                    {
                        return None;
                    }
                }
                owner.then_some((cleanup, plain_source))
            })
            .collect::<Vec<_>>();
        for (cleanup, plain_source) in selected {
            if let Some(source) = plain_source {
                // Copy the authenticated original range-end split with the suffix:
                // the discard stays protected, while the following jump does not.
                self.graph
                    .instructions_mut(source)
                    .next_back()
                    .expect("certified discard predecessor")
                    .no_location_policy
                    .allow_split_return_exit_copy();
            }
            // Keep the original region-end slot and ordinary copy schedule. Only the
            // original no-line-number eligibility was missing from this split array.
            self.graph
                .instructions_mut(cleanup)
                .next_back()
                .expect("certified normal try exit")
                .no_location_policy
                .allow_block_inlining();
        }
    }

    /// Restores a shared original cleanup before its successful edges are copied.
    pub(in crate::assembler) fn prepare_conditional_normal_try_joins(&mut self) {
        // A scheduled original-array plan owns this graph's copy order.
        if !self
            .exception_owner_provenance
            .normal_try_copy_steps
            .is_empty()
        {
            return;
        }
        self.prepare_unlocated_normal_try_discards();
        self.restore_empty_try_jump_owners();
        let mut selected = Vec::new();
        for &(entry, target) in &self.exception_owner_provenance.normal_try_exits {
            let (Some(block), Some(target_block)) = (
                self.graph.label_block(entry),
                self.graph.label_block(target),
            ) else {
                continue;
            };
            // A located conditional block is not a scope exit, so it cannot be copied
            // even below the small-exit limit. Larger located prefixes cannot be copied either.
            let mut target_slots = 0;
            let mut target_is_located = false;
            let mut target_is_conditional = false;
            for item in self.graph[target_block].items() {
                match item {
                    Item::Label(_) if target_slots > 0 => break,
                    Item::Label(_) => {}
                    Item::Instruction(instruction) => {
                        target_slots += 1;
                        target_is_located |=
                            instruction.location.line >= 0 && instruction.opcode != NOP;
                        if target_slots > 4 {
                            break;
                        }
                        target_is_conditional =
                            super::super::is_conditional_jump(instruction.opcode);
                        if super::super::ends_scope(instruction.opcode)
                            || instruction.jump_target().is_some()
                        {
                            break;
                        }
                    }
                }
            }
            // An intact local return is copied after the original cleanup array.
            // Keep that array's POP_BLOCK and normal JUMP until line propagation;
            // the closed constant/tuple-copy plan above still owns its own exits.
            let intact_local_return = original_local_return(self.graph[target_block].items());
            if (target_slots <= 4 && !target_is_conditional && !intact_local_return)
                || !target_is_located
            {
                continue;
            }
            let items = self.graph[block].items();
            let Some((Item::Instruction(jump), head)) = items.split_last() else {
                continue;
            };
            // A terminal condition can retain its direct normal jump while a separate
            // cold recovery array still stops borrowing. Certify that original head
            // below without clearing its borrowed-load boundary.
            let borrowed_terminal_head = jump.jump_policy.borrow_empty_block()
                && jump.jump_policy.retained_nop()
                    == super::super::ir::RetainedJumpNop::DirectTarget
                && jump.location.line >= 0
                && target_slots > 4;
            if head.is_empty()
                || head.iter().any(|item| !matches!(item, Item::Label(_)))
                || jump.opcode != JUMP_FORWARD
                || jump.operand() != Operand::Forward(target)
                || jump.pre_fold_extra_instructions() != 0
                || jump.jump_policy.always_retains_nop()
                || (jump.jump_policy.borrow_empty_block() && !borrowed_terminal_head)
                || jump.jump_policy.defers_redundant_removal()
                || jump.fusion_policy.is_authored_break()
            {
                continue;
            }
            let labels = head
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            // CPython resolves this head before folding its original incoming edges.
            // A surviving conditional is not a unique line donor when another
            // original condition or loop-else exit already folded away.
            if labels.iter().any(|label| {
                self.folded_original_predecessor_targets.contains(label)
                    || self.original_nop_targets.contains(label)
                    || self
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .contains_key(label)
                    || self
                        .exception_owner_provenance
                        .omitted_nop_locations
                        .contains_key(label)
                    || self.folded_condition_exit_targets.contains(label)
                    || self
                        .exception_owner_provenance
                        .setups
                        .iter()
                        .any(|(target, _)| target == label)
                    || self
                        .exception_regions
                        .iter()
                        .any(|region| region.target == *label)
            }) {
                continue;
            }
            let order = self.graph.order();
            let position = order
                .iter()
                .position(|candidate| *candidate == block)
                .unwrap();
            let target_position = order
                .iter()
                .position(|candidate| *candidate == target_block)
                .unwrap();
            let fallthrough_source = position
                .checked_sub(1)
                .map(|index| order[index])
                .filter(|source| super::super::block_has_fallthrough(self.graph[*source].items()));
            // A wildcard call/discard or class-pattern failure cleanup falls through
            // to this original array. Replay both captured instructions exactly;
            // unrelated fallthroughs cannot replace the conditional-origin rule below.
            let original_fallthrough =
                fallthrough_source.is_some_and(|source| {
                    self.exception_owner_provenance
                        .normal_try_fallthrough_donors
                        .get(&entry)
                        .is_some_and(|(first, pop)| {
                            if let Some(&(first_label, last_label)) = self
                                .exception_owner_provenance
                                .normal_try_class_pattern_donor_labels
                                .get(&entry)
                            {
                                let Some(previous) = position.checked_sub(2).map(|index| order[index])
                                else {
                                    return false;
                                };
                                // Replay the two separately labeled original arrays, never
                                // flattening their boundary or accepting renamed copy labels.
                                return self.graph.label_block(first_label) == Some(previous)
                                    && self.graph.label_block(last_label) == Some(source)
                                    && self.graph[previous].items()
                                        == [Item::Label(first_label), Item::Instruction(*first)]
                                    && self.graph[source].items()
                                        == [Item::Label(last_label), Item::Instruction(*pop)]
                                    && self.graph.iter_items().all(|item| {
                                        !matches!(item, Item::Instruction(instruction)
                                            if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                                                if label == last_label))
                                    })
                                    && self.exception_regions.iter().all(|region| {
                                        region.target != first_label
                                            && region.target != last_label
                                            && region.start != last_label
                                            && region.end != last_label
                                    })
                                    && self.exception_owner_provenance.setups.iter().all(
                                        |(label, _)| *label != first_label && *label != last_label,
                                    );
                            }
                            let mut instructions =
                                self.graph[source].items().iter().rev().filter_map(
                                    |item| match item {
                                        Item::Instruction(instruction) => Some(instruction),
                                        Item::Label(_) => None,
                                    },
                                );
                            instructions.next() == Some(pop) && instructions.next() == Some(first)
                        })
                });
            if position >= target_position
                || (fallthrough_source.is_some() && !original_fallthrough)
            {
                continue;
            }
            let mut success_edges = 0;
            let mut successful_sources = Vec::new();
            let mut failure_edges = 0;
            let mut conditional_source = None;
            let mut unknown_edge = false;
            for (source_block, item) in self.graph.iter().flat_map(|(block, contents)| {
                contents.items().iter().map(move |item| (block, item))
            }) {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                let (Operand::Forward(label) | Operand::Backward(label)) = instruction.operand()
                else {
                    continue;
                };
                if !labels.contains(&label) {
                    continue;
                }
                // A case ending in a conditional emits its success edge in the
                // original empty join, whether or not later threading retains its NOP.
                // That retention policy does not change the original predecessor of
                // this POP_BLOCK array. The match-success bit retires on retargeting;
                // replay only the unchanged edge and its authored match-end label.
                let original_match_success_head = instruction.opcode == JUMP_FORWARD
                    && instruction.jump_policy.is_original_match_success()
                    && instruction.no_location_policy.is_artificial_eval_break()
                    && !instruction.no_location_policy.preserves_location()
                    && instruction.pre_fold_extra_instructions() == 0
                    && instruction.local_policy.hidden_exception_owner_slots() == 0
                    && self
                        .exception_owner_provenance
                        .original_match_join_boundaries
                        .contains(&label)
                    && self.graph[source_block].items().split_last().is_some_and(
                        |(last, head)| {
                            matches!(last, Item::Instruction(last) if last == instruction)
                                && head.iter().all(|item| matches!(item, Item::Label(_)))
                                && head.iter().any(|item| matches!(item, Item::Label(label)
                                    if self.exception_owner_provenance.block_boundaries.contains(label)
                                        && !self.borrow_block_aliases.contains(label)))
                        },
                    );
                if instruction.forward_target().is_none()
                    || (instruction.jump_policy.preserves_nop_after_threading()
                        && !original_match_success_head)
                    || instruction.jump_policy.borrow_empty_block()
                    || instruction.jump_policy.always_retains_nop()
                    // An original unlocated eval-breaker head still needs separate
                    // predecessor copies. A preceding real instruction supplies its own line.
                    || (instruction.no_location_policy.is_artificial_eval_break()
                        && !original_match_success_head
                        && self.graph[source_block].items().split_last().is_some_and(|(_, head)| {
                            !head.iter().any(|item| matches!(item,
                                Item::Instruction(instruction)
                                    if instruction.location.line >= 0
                                        && !matches!(instruction.opcode, NOP | crate::target::opcodes::NOT_TAKEN)
                                        && !is_unconditional_jump(instruction.opcode)
                                        && !instruction.no_location_policy.is_artificial_eval_break()))
                        }))
                {
                    unknown_edge = true;
                } else if is_unconditional_jump(instruction.opcode) {
                    success_edges += 1;
                    successful_sources.push(source_block);
                } else if super::super::is_conditional_jump(instruction.opcode)
                    && instruction.location.line >= 0
                {
                    failure_edges += 1;
                    conditional_source = Some(source_block);
                } else {
                    unknown_edge = true;
                }
            }
            // The sole original condition must still donate this exact location.
            // Shared, folded, copied, or fallthrough entries retain the existing
            // rejection; a live success edge still belongs to the ordinary copy path.
            let borrowed_terminal_join = borrowed_terminal_head
                && success_edges == 0
                && failure_edges == 1
                && !unknown_edge
                && fallthrough_source.is_none()
                && !original_fallthrough
                && conditional_source.is_some_and(|source| {
                    order[..position].contains(&source)
                        && self.graph[source].items().iter().any(|item| {
                            matches!(item, Item::Label(label)
                                if self.exception_owner_provenance.block_boundaries.contains(label))
                        })
                        && self.graph[source]
                            .items()
                            .iter()
                            .rev()
                            .find_map(|item| match item {
                                Item::Instruction(instruction) => Some(instruction),
                                Item::Label(_) => None,
                            })
                            .is_some_and(|instruction| {
                                super::super::is_conditional_jump(instruction.opcode)
                                    && instruction.location == jump.location
                                    && instruction.pre_fold_extra_instructions() == 0
                                    && instruction.local_policy.hidden_exception_owner_slots() == 0
                                    && matches!(instruction.operand(), Operand::Forward(label)
                                        if labels.contains(&label))
                            })
                });
            if (borrowed_terminal_head || success_edges == 0) && !borrowed_terminal_join
                || failure_edges != usize::from(!original_fallthrough)
                || unknown_edge
            {
                continue;
            }
            if original_fallthrough {
                // Case-success arrays precede the final physical fallthrough donor.
                // Exclude outer bypasses and nested cleanup arrays; these cannot
                // establish the original match-success copy order.
                let Some(first_success) = order
                    .iter()
                    .position(|block| successful_sources.contains(block))
                else {
                    continue;
                };
                let ordinary_await_labels =
                    self.ordinary_await_cleanup_labels(&order[first_success..position]);
                if !successful_sources
                    .iter()
                    .all(|source| order[..position - 1].contains(source))
                    || order[first_success..position]
                        .iter()
                        .flat_map(|block| self.graph[*block].items())
                        .any(|item| matches!(item, Item::Label(label)
                            if !ordinary_await_labels.contains(label)
                                && (self.exception_regions.iter().any(|region| region.target == *label)
                                    || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label))))
                {
                    continue;
                }
            } else if intact_local_return {
                let conditional_position = order
                    .iter()
                    .position(|candidate| Some(*candidate) == conditional_source)
                    .expect("one conditional predecessor");
                // Restore only interleaved original success and failure arrays.
                // An outer condition that bypasses every successful arm retains
                // the existing direct-return folding and its original line donation.
                // Nested handler arrays already carry their own cleanup slots.
                if conditional_position >= position
                    || !order[..conditional_position]
                        .iter()
                        .any(|block| successful_sources.contains(block))
                    || !order[conditional_position + 1..position]
                        .iter()
                        .any(|block| successful_sources.contains(block))
                    || order[conditional_position + 1..position]
                        .iter()
                        .flat_map(|block| self.graph[*block].items())
                        .any(|item| matches!(item, Item::Label(label)
                            if self.exception_regions.iter().any(|region| region.target == *label)
                                || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label)))
                {
                    continue;
                }
            }
            if !selected.contains(&block) {
                selected.push(block);
            }
        }
        let mut pop_intervals = Vec::new();
        for block in selected {
            let labels = self.graph[block]
                .items()
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            // CPython copies this original unlocated cleanup before removing an
            // adjacent incoming jump. Keep that edge through our early removal pass.
            self.graph.for_each_instruction_mut(|instruction| {
                if instruction.opcode == JUMP_FORWARD
                    && let Some(label) = instruction.forward_target()
                    && labels.contains(&label)
                {
                    instruction.jump_policy.defer_removal();
                }
            });
            let jump = self
                .graph
                .instructions_mut(block)
                .next_back()
                .expect("normal exit jump");
            jump.location = SourceLocation::NONE;
            jump.no_location_policy.clear_preserved_location();
            jump.no_location_policy.allow_block_inlining();
            jump.jump_policy.prevent_threading_target();
            if jump.jump_policy.borrow_empty_block() {
                // Selection admits this barrier only for a certified terminal head.
                // Retain its late jump NOP without changing borrowed-load reachability.
                jump.jump_policy.retain_redundant_nop_after_threading();
            }
            jump.allow_unconditional_block_inlining();
            let depth = jump.depth_after();
            let offset = self.graph[block]
                .items()
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
                .unwrap();
            let mut pop =
                Instruction::synthetic(NOP, Operand::Value(0), SourceLocation::NONE, depth);
            pop.fusion_policy.mark_converted_pop_block();
            let pop_start = Label::from_u32(self.next_label);
            let pop_end = Label::from_u32(self.next_label + 1);
            self.next_label += 2;
            self.graph.insert_label(block, offset, pop_start);
            self.graph
                .insert_item(ItemPosition::new(block, offset + 1), Item::Instruction(pop));
            self.graph.insert_label(block, offset + 2, pop_end);
            pop_intervals.push((pop_start, pop_end));
            self.exception_owner_provenance
                .normal_try_copy_internal_labels
                .push(pop_end);
        }
        if pop_intervals.is_empty() {
            return;
        }
        let mut offsets = FxHashMap::default();
        let mut offset = 0;
        for item in self.graph.iter_items() {
            match item {
                Item::Label(label) => {
                    offsets.insert(*label, offset);
                }
                Item::Instruction(_) => offset += 1,
            }
        }
        pop_intervals.sort_by_key(|(start, _)| offsets[start]);
        for region in std::mem::take(&mut self.exception_regions) {
            if region.is_scope_fallback {
                self.exception_regions.push(region);
                continue;
            }
            let mut start = region.start;
            for &(pop_start, pop_end) in &pop_intervals {
                if offsets[&region.start] <= offsets[&pop_start]
                    && offsets[&pop_end] <= offsets[&region.end]
                {
                    self.exception_regions.push(ExceptionRegion {
                        start,
                        end: pop_start,
                        ..region
                    });
                    start = pop_end;
                }
            }
            self.exception_regions
                .push(ExceptionRegion { start, ..region });
        }
        self.exception_owner_provenance
            .normal_try_copy_pop_ranges
            .extend(pop_intervals);
    }
    /// Restores an omitted Pass before a separately entered ordinary cleanup head.
    fn restore_normal_try_pass_prefixes(&mut self) -> Vec<(Label, Label)> {
        let mut prefixes = Vec::new();
        for (entry, target) in self.exception_owner_provenance.normal_try_exits.clone() {
            let (Some(block), Some(target_block)) = (
                self.graph.label_block(entry),
                self.graph.label_block(target),
            ) else {
                continue;
            };
            let order = self.graph.order();
            let (Some(position), Some(target_position)) = (
                order.iter().position(|candidate| *candidate == block),
                order
                    .iter()
                    .position(|candidate| *candidate == target_block),
            ) else {
                continue;
            };
            let Some((Item::Instruction(jump), head)) = self.graph[block].items().split_last()
            else {
                continue;
            };
            if head.is_empty() || head.iter().any(|item| !matches!(item, Item::Label(_))) {
                continue;
            }
            let jump = *jump;
            let shared_unlocated_exit = jump.location == SourceLocation::NONE
                && head.iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.shared_jump_copy_location_blocks.contains(label))
                });
            if position >= target_position
                || jump.opcode != JUMP_FORWARD
                || jump.operand() != Operand::Forward(target)
                || jump.pre_fold_extra_instructions() != 0
                || jump.jump_policy.always_retains_nop()
                || (jump.no_location_policy.preserves_location() && !shared_unlocated_exit)
            {
                continue;
            }
            let labels = head
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            let Some(location) = self
                .exception_owner_provenance
                .omitted_nop_locations
                .get(&labels[0])
                .copied()
            else {
                continue;
            };
            let prefix_len = labels
                .iter()
                .take_while(|label| {
                    self.exception_owner_provenance
                        .omitted_nop_counts
                        .contains_key(*label)
                })
                .count();
            let Some(entry_offset) = labels.iter().position(|label| *label == entry) else {
                continue;
            };
            // An owner-only label can follow an earlier instruction in the native block.
            // Require real jump entries before and after the Pass to keep both arrays distinct.
            if location.line < 0
                || prefix_len == 0
                || prefix_len >= entry_offset
                || labels[..prefix_len].iter().any(|label| {
                    !self.original_nop_targets.contains(label)
                        || self.exception_owner_provenance.omitted_nop_counts.get(label) != Some(&1)
                        || self.exception_owner_provenance.omitted_nop_locations.get(label)
                            != Some(&location)
                })
                || labels[prefix_len..].iter().any(|label| {
                    self.original_nop_targets.contains(label)
                        || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                        || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                })
                || !self.graph.iter_items().any(|item| matches!(item,
                    Item::Instruction(instruction)
                        if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                            if labels[..prefix_len].contains(&label))))
                || !self.graph.iter_items().any(|item| matches!(item,
                    Item::Instruction(instruction)
                        if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                            if labels[prefix_len..entry_offset].contains(&label))))
            {
                continue;
            }
            // The compiler recorded every label present at the Pass. Later aliases
            // enter after it, so retain those aliases in a distinct fallthrough successor.
            self.graph.insert_item(
                ItemPosition::new(block, prefix_len),
                Item::Instruction(Instruction::synthetic(
                    NOP,
                    Operand::Value(0),
                    location,
                    jump.depth_after(),
                )),
            );
            let mut suffix = Vec::new();
            while self.graph[block].items().len() > prefix_len + 1 {
                suffix.push(self.graph.remove_item_at(block, prefix_len + 1));
            }
            self.graph.insert_block(position + 1, suffix);
            prefixes.extend(labels[..prefix_len].iter().map(|label| (*label, entry)));
        }
        prefixes
    }

    /// Restores original cleanup arrays for a closed ordinary-try/short-return graph.
    ///
    /// These records precede location propagation and NOP compaction in CPython. Keeping
    /// the original `POP_BLOCK` and Pass slots lets the existing copier carry their labels
    /// and exception ranges through the same ascending, once-per-round copy schedule.
    pub(in crate::assembler) fn prepare_normal_try_copy_plan(&mut self) {
        // Prefix restoration and every selected cleanup require a forward jump and
        // a recorded normal exit or discard owner that can certify a cleanup crossing.
        if !self.graph.features().has_forward_jump()
            || (self.exception_owner_provenance.normal_try_exits.is_empty()
                && self
                    .exception_owner_provenance
                    .normal_try_jump_owners
                    .is_empty())
        {
            return;
        }
        let pass_prefixes = self.restore_normal_try_pass_prefixes();
        let mut discard_joins = self.original_discard_try_exit_joins();
        let handler_joins = self.original_discard_handler_exit_joins();
        let handler_heads = handler_joins.keys().copied().collect::<FxHashSet<_>>();
        let handler_continuations = handler_joins
            .values()
            .map(|(_, target, tail)| (*target, *tail))
            .collect::<FxHashMap<_, _>>();
        discard_joins.extend(
            handler_joins
                .iter()
                .map(|(head, (split, target, _))| (*head, (*split, *target))),
        );
        if !discard_joins.is_empty() {
            // Range-only labels do not split CPython's original POP_TOP/JUMP array.
            // Attempt the existing closed schedule with those arrays intact. An
            // unrelated predecessor or unsupported tail leaves the graph unchanged.
            let original = self.graph.clone();
            let original_next_label = self.next_label;
            let discard_blocks = self
                .graph
                .order()
                .iter()
                .copied()
                .filter(|block| discard_joins.contains_key(block))
                .collect::<Vec<_>>();
            let mut internal = discard_blocks
                .iter()
                .map(|block| discard_joins[block].0)
                .collect::<Vec<_>>();
            let forbidden = self.exception_owner_provenance.source_boundaries.clone();
            if self
                .graph
                .join_continuation_blocks(&internal, &forbidden)
                .is_ok()
            {
                for block in discard_blocks {
                    self.graph
                        .instructions_mut(block)
                        .next_back()
                        .expect("original discard exit")
                        .allow_unconditional_block_inlining();
                }
                let mut handler_pop_ranges = Vec::new();
                let mut handler_owner_starts = FxHashMap::default();
                // The unnamed handler's POP_BLOCK precedes its POP_EXCEPT in this
                // same original array. Preserve that slot and its unowned interval.
                let handler_order = self
                    .graph
                    .order()
                    .iter()
                    .copied()
                    .filter(|head| handler_joins.contains_key(head))
                    .collect::<Vec<_>>();
                for head in handler_order {
                    let (split, _, _) = handler_joins[&head];
                    handler_owner_starts.insert(
                        split,
                        self.graph[head]
                            .first_label()
                            .expect("original handler discard"),
                    );
                    let offset = self.graph[head]
                        .items()
                        .iter()
                        .position(|item| *item == Item::Label(split))
                        .expect("handler end")
                        + 1;
                    let end = Label::from_u32(self.next_label);
                    self.next_label += 1;
                    let mut pop = Instruction::synthetic(
                        NOP,
                        Operand::Value(0),
                        SourceLocation::NONE,
                        Some(1),
                    );
                    pop.fusion_policy.mark_converted_pop_block();
                    self.graph
                        .insert_item(ItemPosition::new(head, offset), Item::Instruction(pop));
                    self.graph.insert_label(head, offset + 1, end);
                    internal.push(end);
                    handler_pop_ranges.push((split, end));
                }
                self.prepare_normal_try_copy_plan_with_discards(
                    &pass_prefixes,
                    &discard_joins,
                    &handler_heads,
                    &handler_continuations,
                );
                if !self
                    .exception_owner_provenance
                    .normal_try_copy_steps
                    .is_empty()
                {
                    // Rejoining this exact array puts the handler's original end
                    // inside its block. Expose the owned POP_TOP as a physical
                    // region fragment, so copying its labels carries that owner
                    // while POP_BLOCK and POP_EXCEPT remain outside the handler.
                    // Keep the original priority and all other owner fields.
                    if !handler_owner_starts.is_empty() {
                        let regions = std::mem::take(&mut self.exception_regions);
                        for region in regions {
                            if let Some(&start) = handler_owner_starts.get(&region.end)
                                && region.start != start
                            {
                                self.exception_regions.push(ExceptionRegion {
                                    end: start,
                                    ..region
                                });
                                self.exception_regions
                                    .push(ExceptionRegion { start, ..region });
                            } else {
                                self.exception_regions.push(region);
                            }
                        }
                    }
                    self.exception_owner_provenance
                        .normal_try_copy_internal_labels
                        .extend(internal);
                    self.exception_owner_provenance
                        .normal_try_copy_pop_ranges
                        .extend(handler_pop_ranges);
                    return;
                }
            }
            self.graph = original;
            self.next_label = original_next_label;
        }
        self.prepare_normal_try_copy_plan_with_discards(
            &pass_prefixes,
            &FxHashMap::default(),
            &FxHashSet::default(),
            &FxHashMap::default(),
        );
    }

    /// Checks that a discard-exit label has no borrowing, folding, or omitted-NOP provenance.
    ///
    /// Setup donors and handler entries also cannot certify an original discard exit.
    fn original_discard_label_is_intact(&self, label: Label) -> bool {
        !self.original_nop_targets.contains(&label)
            && !self.folded_original_predecessor_targets.contains(&label)
            && !self.folded_condition_exit_targets.contains(&label)
            && !self.borrow_block_aliases.contains(&label)
            && !self
                .exception_owner_provenance
                .omitted_nop_counts
                .contains_key(&label)
            && !self
                .exception_owner_provenance
                .omitted_nop_locations
                .contains_key(&label)
            && !self
                .exception_owner_provenance
                .setups
                .iter()
                .any(|(entry, _)| *entry == label)
            && !self
                .exception_regions
                .iter()
                .any(|region| region.target == label)
    }

    /// Certifies a discarded runtime expression's original direct ordinary-try exit.
    fn original_discard_try_exit_joins(&self) -> FxHashMap<BasicBlockId, (Label, BasicBlockId)> {
        use crate::target::opcodes::POP_TOP;

        if self
            .exception_owner_provenance
            .normal_try_jump_owners
            .is_empty()
        {
            return FxHashMap::default();
        }
        let graph = &self.graph;
        let order = graph.order();
        let position = |label| {
            let block = graph.label_block(label)?;
            order.iter().position(|candidate| *candidate == block)
        };
        order
            .windows(2)
            .filter_map(|pair| {
                let [Item::Label(join), Item::Instruction(pop)] = graph[pair[0]].items() else {
                    return None;
                };
                let [Item::Label(split), Item::Instruction(jump)] = graph[pair[1]].items() else {
                    return None;
                };
                let target = jump.forward_target()?;
                let target_block = graph.label_block(target)?;
                if *pop
                    != Instruction::new(POP_TOP, Operand::Value(0), SourceLocation::NONE, Some(0))
                    || *jump
                        != Instruction::new(
                            JUMP_FORWARD,
                            Operand::Forward(target),
                            SourceLocation::NONE,
                            Some(0),
                        )
                    || !self.original_discard_label_is_intact(*join)
                    || !self.original_discard_label_is_intact(*split)
                    || !self.preserved_block_boundaries.contains(join)
                    || !self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(join)
                    || !self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(join)
                    || self.preserved_block_boundaries.contains(split)
                    || self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(split)
                    || self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(split)
                    || position(*split)? >= position(target)?
                    || !self.exception_owner_provenance.normal_try_exits.iter().any(
                        |(entry, exit)| {
                            graph.label_block(*entry) == Some(target_block)
                                && super::control_flow::is_original_constant_return(graph, *exit)
                        },
                    )
                {
                    return None;
                }
                // This is the compiler's recorded one-instruction exclusion, not an
                // independently targeted cleanup or an inferred exception-range end.
                let mut owners = self
                    .exception_owner_provenance
                    .normal_try_jump_owners
                    .iter()
                    .filter(|((start, _), _)| start == split);
                let ((_, end), owner) = owners.next()?;
                let [owner] = owner.as_slice() else {
                    return None;
                };
                let following = *order.get(position(*split)? + 1)?;
                if owners.next().is_some()
                    || graph.label_block(*end) != Some(following)
                    || owner.start != *split
                    || owner.end != *end
                    || owner.is_scope_fallback
                    || owner.depth != 0
                    || owner.preserve_lasti
                {
                    return None;
                }
                let mut incoming = None;
                for (source, block) in graph.iter() {
                    for item in block.items() {
                        let Item::Instruction(instruction) = item else {
                            continue;
                        };
                        let (Operand::Forward(label) | Operand::Backward(label)) =
                            instruction.operand()
                        else {
                            continue;
                        };
                        if label == *split {
                            return None;
                        }
                        if label != *join {
                            continue;
                        }
                        let mut expected = Instruction::new(
                            JUMP_FORWARD,
                            Operand::Forward(*join),
                            SourceLocation::NONE,
                            Some(1),
                        );
                        expected.no_location_policy.allow_split_return_exit_copy();
                        if incoming.is_some() || *instruction != expected {
                            return None;
                        }
                        incoming = Some(order.iter().position(|candidate| *candidate == source)?);
                    }
                }
                let incoming = incoming?;
                let join_position = position(*join)?;
                // The true arm and shared discard have the same ordinary-try owner.
                // The false arm is the sole physical fallthrough; no loop/fallback or
                // nested cleanup is admitted by a coincident source location.
                let previous = *order.get(join_position.checked_sub(1)?)?;
                if incoming >= join_position - 1
                    || !super::super::block_has_fallthrough(graph[previous].items())
                    || !self.exception_regions.iter().any(|region| {
                        region.end == *split
                            && region.target == owner.target
                            && region.depth == 0
                            && !region.preserve_lasti
                            && !region.is_scope_fallback
                            && self
                                .exception_owner_provenance
                                .ordinary_try_setups
                                .get(&region.start)
                                == Some(&region.target)
                            && position(region.start).is_some_and(|start| start <= incoming)
                    })
                    || self.exception_regions.iter().any(|region| {
                        let (Some(start), Some(end)) =
                            (position(region.start), position(region.end))
                        else {
                            return true;
                        };
                        [incoming, join_position]
                            .iter()
                            .any(|position| (start..end).contains(position))
                            && (region.target != owner.target
                                || region.depth != 0
                                || region.preserve_lasti
                                || region.is_scope_fallback)
                    })
                {
                    return None;
                }
                Some((pair[0], (*split, target_block)))
            })
            .collect()
    }

    /// Captures the original discard/unnamed-cleanup/outer-join path before threading.
    fn original_discard_handler_exit_joins(
        &self,
    ) -> FxHashMap<BasicBlockId, (Label, BasicBlockId, BasicBlockId)> {
        use crate::target::opcodes::{LOAD_CONST, LOAD_FAST, POP_TOP, RETURN_VALUE, STORE_DEREF};

        if self.exception_owner_provenance.normal_try_exits.is_empty() {
            return FxHashMap::default();
        }
        let graph = &self.graph;
        let order = graph.order();
        let position = |label| {
            let block = graph.label_block(label)?;
            order.iter().position(|candidate| *candidate == block)
        };
        order
            .windows(2)
            .filter_map(|pair| {
                let [Item::Label(join), Item::Instruction(discard)] = graph[pair[0]].items() else {
                    return None;
                };
                let [
                    Item::Label(split),
                    Item::Instruction(pop),
                    Item::Instruction(jump),
                ] = graph[pair[1]].items()
                else {
                    return None;
                };
                let target = jump.forward_target()?;
                let target_block = graph.label_block(target)?;
                let [Item::Label(continuation), Item::Instruction(exit)] =
                    graph[target_block].items()
                else {
                    return None;
                };
                let tail = exit.forward_target()?;
                let tail_block = graph.label_block(tail)?;
                let [
                    Item::Label(tail_head),
                    Item::Instruction(load),
                    Item::Instruction(store),
                    Item::Instruction(none),
                    Item::Instruction(ret),
                ] = graph[tail_block].items()
                else {
                    return None;
                };
                if *continuation != target
                    || *tail_head != tail
                    || *discard
                        != Instruction::new(
                            POP_TOP,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(1),
                        )
                    || *pop
                        != Instruction::new(
                            POP_EXCEPT,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(0),
                        )
                    || [jump, exit].iter().any(|instruction| {
                        instruction.opcode != JUMP_FORWARD
                            || instruction.location != SourceLocation::NONE
                            || instruction.depth_after() != Some(0)
                            || instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.exception_policy()
                                != super::super::ExceptionPolicy::default()
                            || !instruction.jump_policy.inline_small_exit()
                            || instruction
                                .jump_policy
                                .retains_nop_for(instruction.operand())
                            || instruction.jump_policy.is_interruptible()
                            || instruction.no_location_policy
                                != super::super::ir::NoLocationPolicy::default()
                    })
                    || load.opcode != LOAD_FAST
                    || load.value_argument().is_none()
                    || load.depth_after() != Some(1)
                    || load.location.line < 0
                    || load.location.end_line != load.location.line
                    || store.opcode != STORE_DEREF
                    || store.value_argument().is_none()
                    || store.depth_after() != Some(0)
                    || store.location.line != load.location.line
                    || store.location.end_line != store.location.line
                    || none.opcode != LOAD_CONST
                    || none.value_argument().is_none()
                    || *none
                        != Instruction::new(
                            LOAD_CONST,
                            none.operand(),
                            SourceLocation::NONE,
                            Some(1),
                        )
                    || *ret
                        != Instruction::new(
                            RETURN_VALUE,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(0),
                        )
                    || [load, store].iter().any(|instruction| {
                        instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.exception_policy()
                                != super::super::ExceptionPolicy::default()
                            || instruction.no_location_policy
                                != super::super::ir::NoLocationPolicy::default()
                            || instruction.fusion_policy.is_converted_pop_block()
                    })
                    || [*join, *split, target, tail]
                        .iter()
                        .any(|label| !self.original_discard_label_is_intact(*label))
                    || !self.preserved_block_boundaries.contains(join)
                    || ![*join, target, tail].iter().all(|label| {
                        self.exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                            && self
                                .exception_owner_provenance
                                .block_boundaries
                                .contains(label)
                    })
                    || self.preserved_block_boundaries.contains(split)
                    || self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(split)
                    || self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(split)
                    || !(position(*split)? < position(target)?
                        && position(target)? < position(tail)?)
                {
                    return None;
                }
                let mut owners = self
                    .exception_regions
                    .iter()
                    .filter(|region| region.end == *split);
                let owner = owners.next()?;
                if owners.next().is_some()
                    || owner.is_scope_fallback
                    || owner.depth != 1
                    || !owner.preserve_lasti
                    || !self
                        .exception_owner_provenance
                        .setups
                        .contains(&(owner.start, None))
                    || !self.exception_owner_provenance.normal_try_exits.iter().any(
                        |(entry, end)| {
                            *end == target
                                && self.exception_regions.iter().any(|region| {
                                    region.end == *entry
                                        && region.target == owner.start
                                        && !region.is_scope_fallback
                                        && region.depth == 0
                                        && !region.preserve_lasti
                                        && self
                                            .exception_owner_provenance
                                            .ordinary_try_setups
                                            .get(&region.start)
                                            == Some(&owner.start)
                                })
                        },
                    )
                {
                    return None;
                }
                let mut incoming = None;
                for (source, block) in graph.iter() {
                    for item in block.items() {
                        let Item::Instruction(instruction) = item else {
                            continue;
                        };
                        let (Operand::Forward(label) | Operand::Backward(label)) =
                            instruction.operand()
                        else {
                            continue;
                        };
                        if label == *split {
                            return None;
                        }
                        if label != *join {
                            continue;
                        }
                        if incoming.is_some()
                            || instruction.opcode != JUMP_FORWARD
                            || instruction.location != SourceLocation::NONE
                            || instruction.depth_after() != Some(2)
                            || instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.exception_policy()
                                != super::super::ExceptionPolicy::default()
                            || instruction.no_location_policy
                                != super::super::ir::NoLocationPolicy::default()
                            || !instruction.jump_policy.inline_small_exit()
                            || instruction
                                .jump_policy
                                .retains_nop_for(instruction.operand())
                            || instruction.jump_policy.is_interruptible()
                        {
                            return None;
                        }
                        incoming = Some(order.iter().position(|candidate| *candidate == source)?);
                    }
                }
                let incoming = incoming?;
                let join_position = position(*join)?;
                let previous = *order.get(join_position.checked_sub(1)?)?;
                if incoming >= join_position - 1
                    || position(owner.start)? > incoming
                    || !super::super::block_has_fallthrough(graph[previous].items())
                    || self.exception_regions.iter().any(|region| {
                        let (Some(start), Some(end)) =
                            (position(region.start), position(region.end))
                        else {
                            return true;
                        };
                        let same_owner = region.start == owner.start
                            && region.end == owner.end
                            && region.target == owner.target
                            && region.depth == owner.depth
                            && region.preserve_lasti == owner.preserve_lasti
                            && region.is_scope_fallback == owner.is_scope_fallback
                            && match (region.priority, owner.priority) {
                                (
                                    ExceptionRegionPriority::OriginalRange(start, end),
                                    ExceptionRegionPriority::OriginalRange(owner_start, owner_end),
                                ) => start == owner_start && end == owner_end,
                                (
                                    ExceptionRegionPriority::Resolved { start, end },
                                    ExceptionRegionPriority::Resolved {
                                        start: owner_start,
                                        end: owner_end,
                                    },
                                ) => start == owner_start && end == owner_end,
                                _ => false,
                            };
                        ([incoming, join_position]
                            .iter()
                            .any(|position| (start..end).contains(position))
                            && !same_owner)
                            || [position(target).unwrap(), position(tail).unwrap()]
                                .iter()
                                .any(|position| (start..end).contains(position))
                    })
                {
                    return None;
                }
                Some((pair[0], (*split, target_block, tail_block)))
            })
            .collect()
    }

    fn prepare_normal_try_copy_plan_with_discards(
        &mut self,
        pass_prefixes: &[(Label, Label)],
        discard_joins: &FxHashMap<BasicBlockId, (Label, BasicBlockId)>,
        handler_discards: &FxHashSet<BasicBlockId>,
        handler_continuations: &FxHashMap<BasicBlockId, BasicBlockId>,
    ) {
        let graph = &self.graph;
        let order = graph.order();
        let positions = order
            .iter()
            .enumerate()
            .map(|(i, block)| (*block, i))
            .collect::<FxHashMap<_, _>>();
        let original_return_tail = |label| {
            if super::control_flow::is_original_constant_return(graph, label) {
                return Some(OriginalTail::ConstantReturn { located: [true; 3] });
            }
            let block = graph.label_block(label)?;
            let items = graph[block].items();
            let tail = original_local_tuple_return(items)
                .map(|located| OriginalTail::LocalTupleReturn { located })
                .or_else(|| original_local_subscript_return(items))
                .or_else(|| original_computed_return(items))
                .or_else(|| {
                    handler_continuations
                        .values()
                        .any(|tail| *tail == block)
                        .then_some(OriginalTail::CellStoreReturn {
                            located: [true, true, false, false],
                        })
                })?;
            let mut original_head = false;
            for item in items {
                let Item::Label(label) = item else {
                    break;
                };
                // The same original-head exclusions used by the plain tuple plan:
                // no erased records, cleanup entries, or exception-handler aliases.
                if self.original_nop_targets.contains(label)
                    || self.folded_original_predecessor_targets.contains(label)
                    || self.folded_condition_exit_targets.contains(label)
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
                        .setups
                        .iter()
                        .any(|(entry, _)| entry == label)
                    || self
                        .exception_owner_provenance
                        .normal_try_exits
                        .iter()
                        .any(|(entry, _)| entry == label)
                    || self
                        .exception_regions
                        .iter()
                        .any(|region| region.target == *label)
                {
                    return None;
                }
                original_head |= self
                    .exception_owner_provenance
                    .source_boundaries
                    .contains(label)
                    && !self.borrow_block_aliases.contains(label);
            }
            original_head.then_some(tail)
        };
        let mut exits = FxHashMap::default();
        let mut returns = FxHashMap::default();
        for &(entry, target) in &self.exception_owner_provenance.normal_try_exits {
            let (Some(block), Some(target_block)) =
                (graph.label_block(entry), graph.label_block(target))
            else {
                continue;
            };
            let instructions = graph[block]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let [jump] = instructions.as_slice() else {
                continue;
            };
            // The compiler keeps original shared cleanup jumps unlocated while
            // permitting their copies to inherit a predecessor's location.
            let shared_unlocated_exit = jump.location == SourceLocation::NONE
                && graph[block].items().iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.shared_jump_copy_location_blocks.contains(label))
                });
            if positions[&block] >= positions[&target_block]
                || jump.opcode != JUMP_FORWARD
                || jump.operand() != Operand::Forward(target)
                || jump.pre_fold_extra_instructions() != 0
                || jump.jump_policy.always_retains_nop()
                || (jump.no_location_policy.preserves_location() && !shared_unlocated_exit)
            {
                continue;
            }
            // A location-tagged Pass alias is part of this original array, not a second
            // independently copyable two-slot POP_BLOCK/JUMP array at the same offset.
            let mut pass_locations = graph[block]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => self
                        .exception_owner_provenance
                        .omitted_nop_locations
                        .get(label),
                    Item::Instruction(_) => None,
                })
                .copied()
                .collect::<Vec<_>>();
            pass_locations.dedup();
            if pass_locations.len() > 1 || graph[block].items().iter().any(|item| {
                matches!(item, Item::Label(label)
                    if self.exception_owner_provenance.omitted_nop_counts.get(label).is_some_and(|count| *count != 1))
            }) {
                continue;
            }
            let pass = pass_locations.into_iter().next();
            if pass.is_some() {
                let entry_offset = graph[block]
                    .items()
                    .iter()
                    .position(|item| matches!(item, Item::Label(label) if *label == entry))
                    .expect("entry label");
                if graph.iter_items().any(|item| matches!(item,
                    Item::Instruction(instruction)
                        if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                            if graph.label_block(label) == Some(block)
                                && graph[block].items().iter().position(|item|
                                    matches!(item, Item::Label(candidate) if *candidate == label))
                                    .is_none_or(|offset| offset >= entry_offset))))
                { continue; }
            }
            exits.insert(block, (entry, target_block, pass));
            if let Some(tail) = original_return_tail(target)
                && self.exception_regions.iter().all(|region| {
                    region.is_scope_fallback
                        || match (
                            graph.label_block(region.start),
                            graph.label_block(region.end),
                        ) {
                            (Some(start), Some(end)) => !(positions[&start]..positions[&end])
                                .contains(&positions[&target_block]),
                            _ => false,
                        }
                })
            {
                returns.insert(target_block, tail);
            }
        }
        // Original empty joins are ordinary one-slot arrays, not POP_BLOCK cleanups.
        // Compiler provenance excludes authored NOPs. Shared joins retain their original
        // absent line without authorizing NOP retention from conditional donors;
        // current singleton shape alone is insufficient.
        let mut joins = FxHashMap::default();
        for &block in order {
            if exits.contains_key(&block) {
                continue;
            }
            if let Some((_, target)) = discard_joins.get(&block) {
                joins.insert(block, *target);
                continue;
            }
            if let Some(target) = handler_continuations.get(&block) {
                let label = graph[*target]
                    .first_label()
                    .expect("certified assignment return");
                let Some(tail) = original_return_tail(label) else {
                    return;
                };
                joins.insert(block, *target);
                returns.insert(*target, tail);
                continue;
            }
            let instructions = graph[block]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let [jump] = instructions.as_slice() else {
                continue;
            };
            let Some(label) = jump.forward_target() else {
                continue;
            };
            let Some(target) = graph.label_block(label) else {
                continue;
            };
            if jump.opcode != JUMP_FORWARD
                || (!jump.jump_policy.is_original_empty_join()
                    && !jump.jump_policy.is_original_shared_empty_join())
                || !jump.jump_policy.inline_small_exit()
                || jump.jump_policy.always_retains_nop()
                || jump.pre_fold_extra_instructions() != 0
                || positions[&block] >= positions[&target]
                || graph[block].items().iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.original_nop_targets.contains(label)
                            || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                            || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label)
                            || self.exception_regions.iter().any(|region| region.target == *label))
                })
            {
                continue;
            }
            joins.insert(block, target);
            if let Some(tail) = original_return_tail(label)
                && self.exception_regions.iter().all(|region| {
                    region.is_scope_fallback
                        || match (
                            graph.label_block(region.start),
                            graph.label_block(region.end),
                        ) {
                            (Some(start), Some(end)) => {
                                !(positions[&start]..positions[&end]).contains(&positions[&target])
                            }
                            _ => false,
                        }
                })
            {
                returns.insert(target, tail);
            }
        }
        // Paired normal and handler exits share an original join. Located joins
        // surrender their predicted NOP only after the complete schedule is accepted.
        let mut guarded_handler_joins = FxHashMap::default();
        if self.python_version >= crate::PythonVersion::Py314 {
            for &join in order {
                let Some((Item::Instruction(jump), head)) = graph[join].items().split_last() else {
                    continue;
                };
                let Some(label) = jump.forward_target() else {
                    continue;
                };
                let Some(target) = graph.label_block(label) else {
                    continue;
                };
                let Some(tail @ OriginalTail::LocalTupleReturn { .. }) =
                    original_return_tail(label)
                else {
                    continue;
                };
                if head.is_empty()
                    || !head.iter().all(|item| matches!(item, Item::Label(_)))
                    || !head.iter().any(|item| matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.source_boundaries.contains(label)))
                    || positions[&join] >= positions[&target]
                    || self.exception_regions.iter().any(|region| {
                        !region.is_scope_fallback && match (
                            graph.label_block(region.start),
                            graph.label_block(region.end),
                        ) {
                            (Some(start), Some(end)) => {
                                [join, target].iter().any(|block|
                                    (positions[&start]..positions[&end]).contains(&positions[block]))
                            }
                            _ => true,
                        }
                    })
                    || !exits.values().any(|(_, target, pass)| *target == join && pass.is_none())
                    || !graph[join].items().iter().any(|item| matches!(item,
                        Item::Label(label) if self.exception_owner_provenance.original_handler_cleanup_join_exits.contains(label)))
                    || graph[join].items().iter().any(|item| matches!(item,
                        Item::Label(label) if self.borrow_block_aliases.contains(label)
                            || self.folded_original_predecessor_targets.contains(label)
                            || self.folded_condition_exit_targets.contains(label)
                            || self.original_nop_targets.contains(label)
                            || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                            || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label)
                            || self.exception_regions.iter().any(|region| region.target == *label)))
                {
                    continue;
                }
                let mut expected = Instruction::new(
                    JUMP_FORWARD,
                    jump.operand(),
                    jump.location,
                    jump.depth_after(),
                );
                if jump.location == SourceLocation::NONE {
                    expected.no_location_policy.preserve_location();
                } else if jump.location.line >= 0 {
                    expected.jump_policy.mark_original_empty_join();
                    expected.jump_policy.retain_redundant_nop_after_threading();
                    expected.no_location_policy.set_borrow_fallthrough(
                        super::super::ir::BorrowFallthrough::RetainedNopReachable,
                    );
                } else {
                    continue;
                }
                if *jump == expected {
                    guarded_handler_joins.insert(join, (target, tail));
                }
            }
        }
        // An unnamed handler's recorded original cleanup can connect two normal
        // try exits. Its omitted POP_BLOCK is already carried on POP_EXCEPT; keep
        // all three slots in the same closed schedule instead of threading around it.
        let mut handler_cleanup_joins = FxHashSet::default();
        let mut direct_handler_return_joins = FxHashSet::default();
        for &block in order {
            let items = graph[block].items();
            let head_len = items
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
            let [Item::Instruction(pop), Item::Instruction(jump)] = &items[head_len..] else {
                continue;
            };
            let Some(label) = jump.forward_target() else {
                continue;
            };
            let Some(target) = graph.label_block(label) else {
                continue;
            };
            if pop.opcode != POP_EXCEPT
                || pop.pre_fold_extra_instructions() != 1
                || jump.opcode != JUMP_FORWARD
                || jump.pre_fold_extra_instructions() != 0
                || !jump.jump_policy.inline_small_exit()
                || !jump.no_location_policy.allows_block_inlining()
                || jump.jump_policy.retains_nop_for(jump.operand())
                || jump.jump_policy.is_interruptible()
                || pop.depth_after() != jump.depth_after()
                || [pop, jump].iter().any(|instruction| {
                    instruction.location != SourceLocation::NONE
                        || instruction.local_policy.hidden_exception_owner_slots() != 0
                })
                || !self
                    .exception_owner_provenance
                    .original_handler_cleanup_join_exits
                    .contains(&label)
                || (!exits.contains_key(&target)
                    && !guarded_handler_joins.contains_key(&target)
                    && !matches!(returns.get(&target), Some(OriginalTail::LocalTupleReturn { .. })))
                || positions[&block] >= positions[&target]
                || !items[..head_len].iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.source_boundaries.contains(label)
                            && !self.borrow_block_aliases.contains(label))
                })
                || items[..head_len].iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.original_nop_targets.contains(label)
                            || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                            || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label)
                            || self.exception_regions.iter().any(|region| region.target == *label))
                })
            {
                continue;
            }
            handler_cleanup_joins.insert(block);
            if !exits.contains_key(&target) {
                direct_handler_return_joins.insert(block);
            }
            joins.insert(block, target);
        }
        guarded_handler_joins.retain(|join, _| {
            handler_cleanup_joins
                .iter()
                .any(|cleanup| joins.get(cleanup) == Some(join))
        });
        for &join in order {
            if let Some(&(target, tail)) = guarded_handler_joins.get(&join) {
                joins.insert(join, target);
                returns.insert(target, tail);
            }
        }
        // Named cleanup contains two original POP_BLOCK records before POP_EXCEPT.
        // Its complete source snapshot is necessary: an earlier handler arm can copy
        // this unlocated array before the cleanup itself acquires a located return.
        let mut named_cleanup_joins = FxHashSet::default();
        for &(entry, original) in &self
            .exception_owner_provenance
            .original_named_handler_cleanup_joins
        {
            let Some(block) = graph.label_block(entry) else {
                continue;
            };
            let items = graph[block].items();
            let head_len = items
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
            let Operand::Forward(label) = original[4].operand() else {
                continue;
            };
            let Some(target) = graph.label_block(label) else {
                continue;
            };
            if !items[..head_len].contains(&Item::Label(entry))
                || items[head_len..].iter().copied().ne(original.map(Item::Instruction))
                || positions[&block] >= positions[&target]
                || !original[4].jump_policy.inline_small_exit()
                || original[4].jump_policy.retains_nop_for(original[4].operand())
                || original[4].jump_policy.is_interruptible()
                || !original[4].no_location_policy.allows_block_inlining()
                || !items[..head_len].iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.source_boundaries.contains(label)
                            && !self.borrow_block_aliases.contains(label))
                })
                || items[..head_len].iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if self.original_nop_targets.contains(label)
                            || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                            || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label)
                            || self.exception_regions.iter().any(|region| region.target == *label))
                })
            {
                continue;
            }
            named_cleanup_joins.insert(block);
            joins.insert(block, target);
        }
        // A conditional handler join can precede an empty normal-finally exit.
        // Keep both original arrays: the first tuple copy grows past four slots.
        let mut guarded_finally_joins = FxHashSet::default();
        if self.python_version >= crate::PythonVersion::Py314 {
            for &join in order {
                let Some(&middle) = joins.get(&join) else {
                    continue;
                };
                let (
                    Some((Item::Instruction(first), first_head)),
                    Some((Item::Instruction(second), second_head)),
                ) = (
                    graph[join].items().split_last(),
                    graph[middle].items().split_last(),
                )
                else {
                    continue;
                };
                let Some(label) = second.forward_target() else {
                    continue;
                };
                let Some(target) = graph.label_block(label) else {
                    continue;
                };
                let Some(tail @ OriginalTail::LocalTupleReturn { .. }) =
                    original_return_tail(label)
                else {
                    continue;
                };
                let mut expected_first =
                    Instruction::new(JUMP_FORWARD, first.operand(), first.location, Some(0));
                expected_first.jump_policy.mark_original_empty_join();
                expected_first
                    .jump_policy
                    .retain_redundant_nop_after_threading();
                expected_first.no_location_policy.set_borrow_fallthrough(
                    super::super::ir::BorrowFallthrough::RetainedNopReachable,
                );
                let mut expected_second = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(label),
                    SourceLocation::NONE,
                    Some(0),
                );
                expected_second.jump_policy.mark_borrow_empty_block();
                if first.location.line < 0
                    || *first != expected_first
                    || *second != expected_second
                    || first_head.is_empty()
                    || second_head.is_empty()
                    || !first_head.iter().chain(second_head).all(|item| matches!(item, Item::Label(_)))
                    || positions[&join] >= positions[&middle]
                    || positions[&middle] >= positions[&target]
                    || !exits.values().any(|(_, target, pass)| *target == join && pass.is_none())
                    || [join, middle, target].iter().any(|block| {
                        let mut head = graph[*block].items().iter().take_while(|item| matches!(item, Item::Label(_)));
                        !head.clone().any(|item| matches!(item, Item::Label(label)
                            if self.exception_owner_provenance.source_boundaries.contains(label)
                                && self.exception_owner_provenance.block_boundaries.contains(label)))
                            || head.any(|item| matches!(item, Item::Label(label)
                                if self.borrow_block_aliases.contains(label)
                                    || self.folded_original_predecessor_targets.contains(label)
                                    || self.folded_condition_exit_targets.contains(label)
                                    || self.original_nop_targets.contains(label)
                                    || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                                    || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                                    || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label)
                                    || self.exception_regions.iter().any(|region| region.target == *label)))
                    })
                    || self.exception_regions.iter().any(|region| {
                        !region.is_scope_fallback && match (
                            graph.label_block(region.start), graph.label_block(region.end),
                        ) {
                            (Some(start), Some(end)) => [join, middle, target].iter().any(|block|
                                (positions[&start]..positions[&end]).contains(&positions[block])),
                            _ => true,
                        }
                    })
                    || graph[target].items().iter().filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    }).zip([1, 2, 1, 0]).any(|(instruction, depth)| {
                        *instruction != Instruction::new(instruction.opcode, instruction.operand(), instruction.location, Some(depth))
                    })
                {
                    continue;
                }
                guarded_finally_joins.insert(join);
                joins.insert(middle, target);
                returns.insert(target, tail);
            }
        }
        let successor = |block| {
            exits
                .get(&block)
                .map(|(_, target, _)| *target)
                .or_else(|| joins.get(&block).copied())
        };
        // Reject partial paths. Region-end coincidence, cycles and arbitrary jump heads
        // do not certify an original POP_BLOCK or an intact original return array.
        let mut entry_returns = FxHashSet::default();
        for &source in order {
            let Some(jump) = graph[source].last_instruction() else {
                continue;
            };
            let authored_break =
                jump.opcode == JUMP_BACKWARD && jump.fusion_policy.is_authored_break();
            // A real branch entry can reach the same original cleanup array without
            // a loop. Its complete source and every competing predecessor are still
            // admitted by the closed copy graph below; a cleanup's own edge alone
            // does not provide this additional entry certificate. A match-success
            // fallthrough block may have no label; its compiler-emitted edge carries
            // the same original-entry evidence without adding a CFG boundary.
            let ordinary_branch_entry = jump.opcode == JUMP_FORWARD
                && jump.jump_policy.inline_small_exit()
                && !jump.jump_policy.always_retains_nop()
                && jump.pre_fold_extra_instructions() == 0
                && !exits.contains_key(&source)
                && (jump.jump_policy.is_original_match_success()
                    || graph[source].items().iter().any(|item| {
                        matches!(item, Item::Label(label)
                            if self.exception_owner_provenance.block_boundaries.contains(label))
                    }));
            if !authored_break && !ordinary_branch_entry {
                continue;
            }
            let Some(label) = jump.forward_target() else {
                continue;
            };
            let Some(mut current) = graph.label_block(label) else {
                continue;
            };
            if positions[&source] >= positions[&current] {
                continue;
            }
            // An authored break enters the restored Pass first. Follow this one
            // certified fallthrough only to discover its closed cleanup/return path;
            // the Pass block itself never enters the whole-array copy schedule.
            if let Some((_, entry)) = pass_prefixes.iter().find(|(prefix, _)| *prefix == label) {
                // The extra fallthrough is certified only by the existing authored
                // break path; ordinary branches must enter a normal-try array itself.
                if !authored_break {
                    continue;
                }
                let Some(exit) = graph.label_block(*entry) else {
                    continue;
                };
                if order.get(positions[&current] + 1) != Some(&exit) {
                    continue;
                }
                current = exit;
            }
            let mut seen = FxHashSet::default();
            let mut crossed_cleanup = false;
            while let Some(target) = successor(current) {
                if !seen.insert(current) {
                    break;
                }
                crossed_cleanup |=
                    exits.contains_key(&current) || handler_discards.contains(&current);
                current = target;
                if returns.contains_key(&current) {
                    if crossed_cleanup {
                        entry_returns.insert(current);
                    }
                    break;
                }
            }
        }
        if entry_returns.is_empty()
            && (exits.is_empty()
                || !returns
                    .values()
                    .any(|tail| matches!(tail, OriginalTail::LocalTupleReturn { .. })))
        {
            return;
        }
        let original_jump_targets = graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(label) | Operand::Backward(label) => graph.label_block(label),
                    Operand::Value(_) | Operand::Deferred(_) => None,
                },
                Item::Label(_) => None,
            })
            .collect::<FxHashSet<_>>();
        // A private normal exit is part of its preceding runtime array. It can
        // still enter original unlocated joins whose copy rounds determine the
        // size of a later tuple exit, without becoming a standalone POP_BLOCK target.
        let private_join_schedule = entry_returns.is_empty();
        if private_join_schedule {
            let mut private_joins = Vec::new();
            for &exit in order {
                let Some(&(_, join, pass)) = exits.get(&exit) else {
                    continue;
                };
                if pass.is_some() || original_jump_targets.contains(&exit) {
                    continue;
                }
                if guarded_handler_joins.contains_key(&join)
                    || guarded_finally_joins.contains(&join)
                {
                    private_joins.push(join);
                    continue;
                }
                let items = graph[join].items();
                let Some((Item::Instruction(jump), head)) = items.split_last() else {
                    continue;
                };
                let Some(target) = jump.forward_target() else {
                    continue;
                };
                let Some(target) = graph.label_block(target) else {
                    continue;
                };
                if head.is_empty()
                    || !head.iter().all(|item| matches!(item, Item::Label(_)))
                    || !head.iter().any(|item| matches!(item, Item::Label(label)
                        if self.exception_owner_provenance.source_boundaries.contains(label)
                            && !self.borrow_block_aliases.contains(label)))
                    || head.iter().any(|item| matches!(item, Item::Label(label)
                        if self.original_nop_targets.contains(label)
                            || self.folded_original_predecessor_targets.contains(label)
                            || self.folded_condition_exit_targets.contains(label)
                            || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                            || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                            || self.exception_owner_provenance.setups.iter().any(|(entry, _)| entry == label)
                            || self.exception_regions.iter().any(|region| region.target == *label)))
                    || jump.opcode != JUMP_FORWARD
                    || jump.location != SourceLocation::NONE
                    || jump.pre_fold_extra_instructions() != 0
                    || jump.local_policy.hidden_exception_owner_slots() != 0
                    || jump.fusion_policy.is_converted_pop_block()
                    || jump.fusion_policy.is_authored_break()
                    || jump.no_location_policy != super::super::ir::NoLocationPolicy::default()
                    || jump.exception_policy() != super::super::ExceptionPolicy::default()
                    || !jump.jump_policy.inline_small_exit()
                    || jump.jump_policy.retains_nop_for(jump.operand())
                    || jump.jump_policy.defers_redundant_removal()
                    || jump.jump_policy.is_interruptible()
                    || positions[&exit] >= positions[&join]
                    || positions[&join] >= positions[&target]
                    || self.exception_regions.iter().any(|region| {
                        !region.is_scope_fallback
                            && match (graph.label_block(region.start), graph.label_block(region.end)) {
                                (Some(start), Some(end)) => (positions[&start]..positions[&end])
                                    .contains(&positions[&join]),
                                _ => true,
                            }
                    })
                {
                    continue;
                }
                private_joins.push(join);
                joins.insert(join, target);
            }
            for start in private_joins {
                let mut current = start;
                let mut seen = FxHashSet::default();
                while let Some(&target) = joins.get(&current) {
                    if !seen.insert(current) {
                        break;
                    }
                    current = target;
                    if matches!(
                        returns.get(&current),
                        Some(OriginalTail::LocalTupleReturn { .. })
                    ) {
                        entry_returns.insert(current);
                        break;
                    }
                }
            }
        }
        let successor = |block| {
            exits
                .get(&block)
                .map(|(_, target, _)| *target)
                .or_else(|| joins.get(&block).copied())
        };
        let mut selected = FxHashSet::default();
        for start in order
            .iter()
            .copied()
            .filter(|block| exits.contains_key(block))
        {
            let mut path = Vec::new();
            let mut current = start;
            while let Some(target) = successor(current) {
                if path.contains(&current) {
                    break;
                }
                path.push(current);
                current = target;
                if entry_returns.contains(&current) {
                    selected.extend(path.into_iter().filter(|block| exits.contains_key(block)));
                    break;
                }
            }
        }
        if selected.is_empty() {
            return;
        }
        // A range end alone is not an original CFG entry. Every restored cleanup
        // array must also be entered by a real original jump target at this head.
        if selected
            .iter()
            .any(|block| !original_jump_targets.contains(block))
        {
            // A private normal exit belongs to its preceding runtime array. For
            // an intact constant or local tuple tail, keep its jump as an ordinary
            // scheduled source; no predecessor can copy its omitted POP_BLOCK or
            // depend on its width. Only independently entered cleanup targets
            // receive restored arrays.
            if !entry_returns.iter().all(|block| {
                matches!(
                    returns[block],
                    OriginalTail::ConstantReturn { .. } | OriginalTail::LocalTupleReturn { .. }
                )
            })
                || selected
                    .iter()
                    .filter(|block| !original_jump_targets.contains(block))
                    .any(|block| {
                        exits[block].2.is_some()
                            || !matches!(graph[*block].items().split_last(),
                                Some((Item::Instruction(jump), head))
                                    if !head.is_empty()
                                        && head.iter().all(|item| matches!(item, Item::Label(_)))
                                        && jump.location.line >= 0
                                        && jump.local_policy.hidden_exception_owner_slots() == 0
                                        && jump.exception_policy() == super::super::ExceptionPolicy::default()
                                        && !jump.jump_policy.retains_nop_for(jump.operand())
                                        && !jump.jump_policy.is_interruptible())
                    })
            {
                return;
            }
            selected.retain(|block| original_jump_targets.contains(block));
            if selected.is_empty() && !private_join_schedule {
                return;
            }
        }
        if private_join_schedule && !selected.is_empty() {
            return;
        }
        let selected_returns = entry_returns;
        let mut targets = selected
            .union(&selected_returns)
            .copied()
            .collect::<FxHashSet<_>>();
        // Keep intermediate original targets, including joins before the first
        // cleanup. All their unconditional predecessors are checked below too.
        loop {
            let before = targets.len();
            for &join in order {
                if let Some(target) = joins.get(&join)
                    && targets.contains(target)
                {
                    targets.insert(join);
                }
            }
            if targets.len() == before {
                break;
            }
        }
        let selected_joins = targets
            .iter()
            .copied()
            .filter(|block| joins.contains_key(block))
            .collect::<FxHashSet<_>>();
        if discard_joins
            .keys()
            .any(|block| !selected_joins.contains(block))
        {
            return;
        }
        // The generic copier copies complete blocks. Reject a jump into a return
        // suffix rather than treating it as an alias of the original return head.
        if graph.iter_items().any(|item| matches!(item, Item::Instruction(instruction)
            if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                if graph.label_block(label).is_some_and(|block| selected_returns.contains(&block)
                    && graph[block].items().iter().take_while(|item| !matches!(item, Item::Label(candidate) if *candidate == label))
                        .any(|item| matches!(item, Item::Instruction(_)))))))
        { return; }
        let mut sources = FxHashSet::default();
        for &block in order {
            if let Some(jump) = graph[block].last_instruction()
                && is_unconditional_jump(jump.opcode)
                && let Some(target) = jump.forward_target()
                && graph
                    .label_block(target)
                    .is_some_and(|target| targets.contains(&target))
                && jump.jump_policy.inline_small_exit()
                && !jump.jump_policy.always_retains_nop()
                && jump.pre_fold_extra_instructions() == 0
                && (selected_joins.contains(&block)
                    // The original final case jumps across its empty failure block.
                    // Its deferred removal keeps this edge live until exit copying,
                    // even when its target is the next physical instruction.
                    || (jump.jump_policy.is_original_match_success()
                        && jump.jump_policy.defers_redundant_removal())
                    || graph.label_block(target).is_some_and(|target| {
                        // Certified join sources are protected by the pre-copy plan guard.
                        // Early jump removal skips label-only blocks. Compare its next
                        // physical instruction, not the next block arena/order position.
                        order
                            .iter()
                            .skip(positions[&block] + 1)
                            .copied()
                            .find(|candidate| {
                                graph[*candidate]
                                    .items()
                                    .iter()
                                    .any(|item| matches!(item, Item::Instruction(_)))
                            })
                            != Some(target)
                    }))
            {
                sources.insert(block);
            }
        }
        // Extending a schedule through handler cleanup also admits its incoming
        // sources. Every further unconditional predecessor must participate before
        // one of those arrays acquires a return; reject incomplete extensions.
        let extends_handler_cleanup = named_cleanup_joins
            .iter()
            .chain(&direct_handler_return_joins)
            .any(|block| selected_joins.contains(block));
        if extends_handler_cleanup
            && order.iter().any(|block| {
                graph[*block].last_instruction().is_some_and(|jump| {
                is_unconditional_jump(jump.opcode)
                    && matches!(jump.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if graph.label_block(label).is_some_and(|target|
                            sources.contains(&target) && !targets.contains(&target)))
            })
            })
        {
            return;
        }
        if !selected.is_subset(&sources) || !selected_joins.is_subset(&sources) {
            return;
        }
        // Widths of private range fragments need not reconstruct the preceding
        // runtime array: no jump can enter them. Every actual copy target remains
        // a certified singleton join or the intact four-slot tuple return.
        if private_join_schedule
            && sources.iter().any(|block| {
                !selected_joins.contains(block) && original_jump_targets.contains(block)
            })
        {
            return;
        }
        // An excluded unconditional predecessor could change a target array before this
        // schedule reaches it. Decline the whole certificate instead of mixing orders.
        if order.iter().any(|block| {
            graph[*block].last_instruction().is_some_and(|jump| {
                is_unconditional_jump(jump.opcode)
                    && matches!(jump.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if graph.label_block(label).is_some_and(|target| targets.contains(&target)))
                    && !sources.contains(block)
            })
        }) {
            return;
        }
        let mut nodes = Vec::new();
        for &block in order {
            if selected_returns.contains(&block) {
                nodes.push(OriginalNode {
                    id: block,
                    prefix: Vec::new(),
                    tail: returns[&block],
                });
            } else if sources.contains(&block) {
                let jump = graph[block].last_instruction().expect("certified jump");
                let Some(target) = jump.forward_target() else {
                    unreachable!()
                };
                let target = graph.label_block(target).expect("certified target");
                let mut prefix = Vec::new();
                let located = if let Some((_, _, pass)) =
                    exits.get(&block).filter(|_| selected.contains(&block))
                {
                    if pass.is_some() {
                        prefix.push(PrefixSlot {
                            is_nop: true,
                            located: true,
                        });
                    }
                    prefix.push(PrefixSlot {
                        is_nop: true,
                        located: false,
                    });
                    false
                } else if selected_joins.contains(&block) {
                    if discard_joins.contains_key(&block) {
                        prefix.push(PrefixSlot {
                            is_nop: false,
                            located: false,
                        });
                        if handler_discards.contains(&block) {
                            prefix.extend([
                                PrefixSlot {
                                    is_nop: true,
                                    located: false,
                                },
                                PrefixSlot {
                                    is_nop: false,
                                    located: false,
                                },
                            ]);
                        }
                    }
                    if handler_cleanup_joins.contains(&block) {
                        // The producer recorded POP_BLOCK as one hidden slot on
                        // this intact POP_EXCEPT, followed by the original jump.
                        prefix.extend([
                            PrefixSlot {
                                is_nop: true,
                                located: false,
                            },
                            PrefixSlot {
                                is_nop: false,
                                located: false,
                            },
                        ]);
                    }
                    if named_cleanup_joins.contains(&block) {
                        prefix.extend(
                            [PrefixSlot {
                                is_nop: true,
                                located: false,
                            }; 2],
                        );
                        prefix.extend(
                            [PrefixSlot {
                                is_nop: false,
                                located: false,
                            }; 4],
                        );
                    }
                    // An ordinary empty join has no prefix slots.
                    false
                } else {
                    let mut instructions = graph[block]
                        .items()
                        .iter()
                        .filter_map(|item| match item {
                            Item::Instruction(instruction) => Some(instruction),
                            Item::Label(_) => None,
                        })
                        .peekable();
                    while let Some(instruction) = instructions.next() {
                        if instructions.peek().is_none() {
                            break;
                        }
                        prefix.push(PrefixSlot {
                            is_nop: instruction.opcode == NOP,
                            located: instruction.location.line >= 0,
                        });
                    }
                    jump.location.line >= 0
                };
                nodes.push(OriginalNode {
                    id: block,
                    prefix,
                    tail: OriginalTail::Jump {
                        target,
                        located,
                        interruptible: jump.opcode == JUMP_BACKWARD
                            || jump.jump_policy.is_interruptible(),
                    },
                });
            }
        }
        let Some(result) = plan_normal_try_copies(&nodes) else {
            return;
        };
        // Restore only an explicitly recorded original jump owner. The compiler's
        // eager exclusion predicted that this edge already reached the unprotected
        // footer; the accepted schedule can instead keep or copy the protected jump.
        let mut original_offsets = FxHashMap::default();
        let mut original_instructions = Vec::new();
        for item in graph.iter_items() {
            match item {
                Item::Label(label) => {
                    original_offsets.insert(*label, original_instructions.len());
                }
                Item::Instruction(instruction) => original_instructions.push(instruction),
            }
        }
        let mut original_jump_owners = self
            .exception_owner_provenance
            .normal_try_jump_owners
            .iter()
            .filter_map(|(&(start, end), owners)| {
                let source = graph.label_block(start)?;
                let start_offset = *original_offsets.get(&start)?;
                let end_offset = *original_offsets.get(&end)?;
                let jump = graph[source].last_instruction()?;
                if !sources.contains(&source)
                    // Empty-body exits keep their exclusion unless a separate intact
                    // runtime prefix proves that their original jump survives copying.
                    || jump.exception_policy() == super::super::ExceptionPolicy::EXCLUDE_ALWAYS
                    || end_offset != start_offset + 1
                    || original_instructions.get(start_offset).copied() != Some(jump)
                    || jump.opcode != JUMP_FORWARD
                    || jump.pre_fold_extra_instructions() != 0
                {
                    return None;
                }
                Some((start_offset, start.as_u32(), end.as_u32(), owners))
            })
            .collect::<Vec<_>>();
        // Preserve original source order and then the recorded owner order for
        // each slot; map iteration must not decide exception-table tie ordering.
        original_jump_owners.sort_by_key(|(offset, start, end, _)| (*offset, *start, *end));
        let original_jump_owners = original_jump_owners
            .into_iter()
            .flat_map(|(_, _, _, owners)| owners.iter().copied())
            .collect::<Vec<_>>();
        let ordered_joins = order
            .iter()
            .copied()
            .filter(|block| selected_joins.contains(block))
            .collect::<Vec<_>>();
        let mut labels = FxHashMap::default();
        let blocks = order
            .iter()
            .copied()
            .filter(|block| sources.contains(block) || selected_returns.contains(block))
            .collect::<Vec<_>>();
        for block in blocks {
            let label = if let Some(label) = self.graph[block].first_label() {
                label
            } else {
                let label = Label::from_u32(self.next_label);
                self.next_label += 1;
                self.graph.insert_label(block, 0, label);
                label
            };
            labels.insert(block, label);
        }
        // Restore original absence only after the complete schedule was accepted.
        // The compiler's inherited final-arm line is not authored on this join.
        for block in ordered_joins {
            if named_cleanup_joins.contains(&block) {
                // Carry the two omitted POP_BLOCKs as exact original array width.
                // Their ownerless NOPs disappear after copying and need no live slots.
                self.graph
                    .instructions_mut(block)
                    .next()
                    .expect("named cleanup POP_EXCEPT")
                    .add_pre_fold_extra_instructions(2);
            }
            let jump = self
                .graph
                .instructions_mut(block)
                .next_back()
                .expect("certified original empty join");
            if (guarded_handler_joins.contains_key(&block)
                || guarded_finally_joins.contains(&block))
                && jump.jump_policy.is_original_empty_join()
            {
                jump.jump_policy.consume_original_join_copy_prediction();
            }
            jump.location = SourceLocation::NONE;
            jump.no_location_policy.clear_preserved_location();
            jump.no_location_policy.allow_block_inlining();
        }
        let mut pop_intervals = Vec::new();
        let mut selected_in_order = selected.iter().copied().collect::<Vec<_>>();
        selected_in_order.sort_by_key(|block| positions[block]);
        for block in &selected_in_order {
            let (entry, _, pass) = exits[block];
            let jump = self
                .graph
                .instructions_mut(*block)
                .next_back()
                .expect("certified normal exit");
            jump.location = SourceLocation::NONE;
            jump.no_location_policy.allow_block_inlining();
            let depth = jump.depth_after();
            let entry_offset = self.graph[*block]
                .items()
                .iter()
                .position(|item| matches!(item, Item::Label(label) if *label == entry))
                .expect("certified normal exit label");
            let mut pop =
                Instruction::synthetic(NOP, Operand::Value(0), SourceLocation::NONE, depth);
            pop.fusion_policy.mark_converted_pop_block();
            let pop_start = Label::from_u32(self.next_label);
            let pop_end = Label::from_u32(self.next_label + 1);
            self.next_label += 2;
            let jump_offset = self.graph[*block]
                .items()
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
                .expect("normal jump");
            // All original no-Pass aliases enter before POP_BLOCK. The authored Pass,
            // when present, lies before the original try_end label and keeps its owner.
            self.graph.insert_label(*block, jump_offset, pop_start);
            self.graph.insert_item(
                ItemPosition::new(*block, jump_offset + 1),
                Item::Instruction(pop),
            );
            self.graph.insert_label(*block, jump_offset + 2, pop_end);
            pop_intervals.push((pop_start, pop_end));
            if let Some(location) = pass {
                self.graph.insert_item(
                    ItemPosition::new(*block, entry_offset),
                    Item::Instruction(Instruction::synthetic(
                        NOP,
                        Operand::Value(0),
                        location,
                        depth,
                    )),
                );
            }
        }
        let mut internal_labels = Vec::new();
        for block in &selected_in_order {
            let mut after_instruction = false;
            for item in self.graph[*block].items() {
                match item {
                    Item::Instruction(_) => after_instruction = true,
                    Item::Label(label) if after_instruction => internal_labels.push(*label),
                    Item::Label(_) => {}
                }
            }
        }
        self.exception_owner_provenance
            .normal_try_copy_internal_labels = internal_labels;
        let mut offsets = FxHashMap::default();
        let mut index = 0;
        for item in self.graph.iter_items() {
            match item {
                Item::Label(label) => {
                    offsets.insert(*label, index);
                }
                Item::Instruction(_) => index += 1,
            }
        }
        self.exception_regions.extend(original_jump_owners);
        let regions = std::mem::take(&mut self.exception_regions);
        for region in regions {
            if region.is_scope_fallback {
                self.exception_regions.push(region);
                continue;
            }
            let mut start = region.start;
            for &(pop_start, pop_end) in &pop_intervals {
                if offsets[&region.start] <= offsets[&pop_start]
                    && offsets[&pop_end] <= offsets[&region.end]
                {
                    self.exception_regions.push(ExceptionRegion {
                        start,
                        end: pop_start,
                        ..region
                    });
                    start = pop_end;
                }
            }
            self.exception_regions
                .push(ExceptionRegion { start, ..region });
        }
        self.exception_owner_provenance.normal_try_copy_blocks =
            sources.iter().map(|block| labels[block]).collect();
        self.exception_owner_provenance.normal_try_copy_pop_ranges = pop_intervals;
        self.exception_owner_provenance.normal_try_copy_steps = result
            .steps
            .iter()
            .map(|step| {
                (
                    labels[&step.source],
                    labels[&step.target],
                    step.target_width,
                )
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::{Assembler, ExceptionPolicy, ExceptionRegionPriority, UnoptimizedCode};
    use crate::target::opcodes::{LOAD_CONST, POP_EXCEPT, POP_TOP, RETURN_VALUE};

    fn item(opcode: crate::target::Opcode, operand: Operand, line: i32) -> Item {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            if line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(line, line, 4, 9)
            },
            None,
        ))
    }

    fn region(start: Label, end: Label, target: Label, fallback: bool) -> ExceptionRegion {
        ExceptionRegion {
            start,
            end,
            target,
            depth: 0,
            preserve_lasti: fallback,
            is_scope_fallback: fallback,
            priority: ExceptionRegionPriority::OriginalRange(start, end),
        }
    }

    fn two_exits() -> OptimizationPipeline {
        let mut source = Instruction::new(
            JUMP_BACKWARD,
            Operand::Forward(Label::from_u32(2)),
            SourceLocation::new(11, 11, 8, 13),
            None,
        );
        source.fusion_policy.mark_authored_break();
        let mut assembler = Assembler {
            code: UnoptimizedCode::Linear(vec![
                Item::Label(Label::from_u32(0)),
                item(POP_EXCEPT, Operand::Value(0), 11),
                item(POP_TOP, Operand::Value(0), 11),
                Item::Instruction(source),
                Item::Label(Label::from_u32(1)),
                item(RETURN_VALUE, Operand::Value(0), 1),
                Item::Label(Label::from_u32(2)),
                item(JUMP_FORWARD, Operand::Forward(Label::from_u32(4)), 5),
                Item::Label(Label::from_u32(3)),
                item(RETURN_VALUE, Operand::Value(0), 2),
                Item::Label(Label::from_u32(4)),
                item(JUMP_FORWARD, Operand::Forward(Label::from_u32(6)), 3),
                Item::Label(Label::from_u32(5)),
                item(RETURN_VALUE, Operand::Value(0), 2),
                Item::Label(Label::from_u32(6)),
                item(NOP, Operand::Value(0), 20),
                item(LOAD_CONST, Operand::Value(0), 20),
                item(RETURN_VALUE, Operand::Value(0), 20),
                Item::Label(Label::from_u32(7)),
                item(RETURN_VALUE, Operand::Value(0), 21),
                Item::Label(Label::from_u32(8)),
                item(RETURN_VALUE, Operand::Value(0), 22),
                Item::Label(Label::from_u32(9)),
                item(RETURN_VALUE, Operand::Value(0), 23),
            ]),
            exception_regions: vec![
                region(
                    Label::from_u32(0),
                    Label::from_u32(2),
                    Label::from_u32(7),
                    false,
                ),
                region(
                    Label::from_u32(0),
                    Label::from_u32(4),
                    Label::from_u32(8),
                    false,
                ),
                region(
                    Label::from_u32(0),
                    Label::from_u32(7),
                    Label::from_u32(9),
                    true,
                ),
            ],
            next_label: 10,
            ..Assembler::default()
        };
        assembler.record_normal_try_exit(Label::from_u32(2), Label::from_u32(4), None);
        assembler.record_normal_try_exit(Label::from_u32(4), Label::from_u32(6), None);
        assembler.into_optimization_pipeline()
    }

    fn owners_at(
        pipeline: &OptimizationPipeline,
        wanted_block: crate::assembler::block_graph::BasicBlockId,
        ordinal: usize,
    ) -> Vec<Label> {
        let mut positions = FxHashMap::default();
        let mut index = 0;
        let mut slot = None;
        for (block, contents) in pipeline.graph.iter() {
            let mut instruction_index = 0;
            for item in contents.items() {
                match item {
                    Item::Label(label) => {
                        positions.insert(*label, index);
                    }
                    Item::Instruction(_) => {
                        if block == wanted_block && instruction_index == ordinal {
                            slot = Some(index);
                        }
                        instruction_index += 1;
                        index += 1;
                    }
                }
            }
        }
        let slot = slot.expect("unique source instruction");
        let mut owners = pipeline
            .exception_regions
            .iter()
            .filter(|region| positions[&region.start] <= slot && slot < positions[&region.end])
            .map(|region| region.target)
            .collect::<Vec<_>>();
        owners.sort_by_key(|label| label.as_u32());
        owners.dedup();
        owners
    }

    fn instructions_between(
        pipeline: &OptimizationPipeline,
        start: Label,
        end: Label,
    ) -> Vec<(
        crate::assembler::block_graph::BasicBlockId,
        usize,
        Instruction,
    )> {
        let mut result = Vec::new();
        let mut active = false;
        let mut seen_start = false;
        let mut seen_end = false;
        for (block, contents) in pipeline.graph.iter() {
            let mut ordinal = 0;
            for item in contents.items() {
                match item {
                    Item::Label(label) if *label == start => {
                        assert!(!seen_start && !seen_end);
                        active = true;
                        seen_start = true;
                    }
                    Item::Label(label) if *label == end => {
                        assert!(seen_start && !seen_end);
                        active = false;
                        seen_end = true;
                    }
                    Item::Instruction(instruction) => {
                        if active {
                            result.push((block, ordinal, *instruction));
                        }
                        ordinal += 1;
                    }
                    Item::Label(_) => {}
                }
            }
        }
        assert!(seen_start && seen_end);
        result
    }

    #[test]
    fn original_pop_holes_preserve_fallback_and_copied_jump_keeps_intermediate_owner() {
        let mut pipeline = two_exits();
        pipeline.prepare_normal_try_copy_plan();
        assert!(
            !pipeline
                .exception_owner_provenance
                .normal_try_copy_steps
                .is_empty()
        );
        for label in [Label::from_u32(2), Label::from_u32(4)] {
            let block = pipeline.graph.label_block(label).unwrap();
            let pop = *pipeline.graph[block]
                .items()
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .unwrap();
            assert!(pop.fusion_policy.is_converted_pop_block());
            assert_eq!(
                pop.exception_policy().exclusion(),
                crate::assembler::ExceptionExclusion::None
            );
            assert_eq!(owners_at(&pipeline, block, 0), [Label::from_u32(9)]);
        }
        let absence = pipeline.capture_original_location_absence();
        let copies = pipeline.duplicate_exit_blocks(&absence, &mut Vec::new());
        let first_copy = *copies.first().expect("first original cleanup copy");
        // Normalization separates labels inside a copied array. Select its exact
        // ordered range instead of assuming the original source remains one block.
        let copied = instructions_between(&pipeline, first_copy, Label::from_u32(1));
        let [
            (pop_block, pop_ordinal, pop),
            (jump_block, jump_ordinal, jump),
        ] = copied.as_slice()
        else {
            panic!("first cleanup copy must contain exactly POP_BLOCK and its intermediate jump");
        };
        assert_eq!(jump.operand(), Operand::Forward(Label::from_u32(4)));
        assert_eq!(
            owners_at(&pipeline, *jump_block, *jump_ordinal),
            [Label::from_u32(8), Label::from_u32(9)]
        );
        assert!(pop.fusion_policy.is_converted_pop_block());
        assert_eq!(
            owners_at(&pipeline, *pop_block, *pop_ordinal),
            [Label::from_u32(9)]
        );
        let source = instructions_between(&pipeline, Label::from_u32(0), first_copy);
        assert_eq!(source.len(), 3);
        let markers = source
            .iter()
            .copied()
            .filter(|(_, _, instruction)| {
                instruction.opcode == NOP
                    && instruction.location == SourceLocation::new(11, 11, 8, 13)
            })
            .collect::<Vec<_>>();
        let [(marker_block, marker_ordinal, _)] = markers.as_slice() else {
            panic!("the replaced source jump must have one original NOP marker");
        };
        assert_eq!(
            owners_at(&pipeline, *marker_block, *marker_ordinal),
            [Label::from_u32(7), Label::from_u32(8), Label::from_u32(9)]
        );
    }

    #[test]
    fn empty_setup_owner_restoration_preserves_original_priority() {
        let [start, end, handler, other, middle, missing] = [0, 1, 2, 3, 4, 5].map(Label::from_u32);
        let offsets = FxHashMap::from_iter([(start, 0), (middle, 1), (end, 2)]);
        let owner = ExceptionRegion {
            priority: ExceptionRegionPriority::Resolved { start: 10, end: 20 },
            ..region(start, end, handler, false)
        };
        let enclosing = ExceptionRegion {
            priority: ExceptionRegionPriority::Resolved { start: 5, end: 25 },
            ..owner
        };
        let inner = ExceptionRegion {
            priority: ExceptionRegionPriority::Resolved { start: 11, end: 19 },
            ..owner
        };
        let different = ExceptionRegion {
            target: other,
            ..owner
        };
        for (name, live, expected) in [
            ("same owner", owner, true),
            (
                "same owner fragment",
                ExceptionRegion {
                    start: middle,
                    ..owner
                },
                true,
            ),
            ("enclosing owner", enclosing, true),
            (
                "fallback owner",
                ExceptionRegion {
                    is_scope_fallback: true,
                    ..inner
                },
                true,
            ),
            ("inner owner at same depth", inner, false),
            (
                "inner owner ends at jump",
                ExceptionRegion {
                    end: middle,
                    ..inner
                },
                true,
            ),
            ("equal priority other target", different, false),
            (
                "equal priority other depth",
                ExceptionRegion {
                    depth: owner.depth + 1,
                    ..owner
                },
                false,
            ),
            (
                "equal priority other lasti",
                ExceptionRegion {
                    preserve_lasti: !owner.preserve_lasti,
                    ..owner
                },
                false,
            ),
            (
                "unknown physical end",
                ExceptionRegion {
                    end: missing,
                    ..owner
                },
                false,
            ),
            (
                "reversed physical range",
                ExceptionRegion {
                    start: end,
                    end: start,
                    ..owner
                },
                false,
            ),
            (
                "unresolved priority",
                region(start, end, handler, false),
                false,
            ),
        ] {
            assert_eq!(
                restored_empty_try_owner_is_unshadowed(&[owner], &[live], &offsets, 1),
                expected,
                "{name}",
            );
        }
        assert!(restored_empty_try_owner_is_unshadowed(
            &[owner, different],
            &[owner],
            &offsets,
            1,
        ));
        assert!(!restored_empty_try_owner_is_unshadowed(
            &[different, owner],
            &[owner],
            &offsets,
            1,
        ));
        assert!(!restored_empty_try_owner_is_unshadowed(
            &[region(start, end, handler, false)],
            &[],
            &offsets,
            1,
        ));
    }

    #[test]
    fn empty_setup_owner_restoration_requires_known_matching_depths() {
        for (setup_depth, jump_depth) in [
            (Some(2), Some(2)),
            (Some(2), Some(3)),
            (Some(2), None),
            (None, Some(2)),
            (None, None),
        ] {
            let [entry, start, end, target, handler] = [0, 1, 2, 3, 4].map(Label::from_u32);
            let location = SourceLocation::new(5, 8, 4, 12);
            let mut setup = Instruction::new(NOP, Operand::Value(0), location, setup_depth);
            setup.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            setup.jump_policy.preserve_explicit_nop();
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::NONE,
                jump_depth,
            );
            jump.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            jump.jump_policy.retain_nop_for_direct_target();
            let mut items = vec![
                Item::Label(entry),
                Item::Instruction(setup),
                Item::Label(start),
                Item::Instruction(jump),
                Item::Label(end),
                Item::Label(target),
            ];
            items.extend((0..5).map(|_| item(LOAD_CONST, Operand::Value(0), 9)));
            items.extend([
                item(RETURN_VALUE, Operand::Value(0), 9),
                Item::Label(handler),
                item(RETURN_VALUE, Operand::Value(0), 10),
            ]);
            let mut assembler = Assembler {
                code: UnoptimizedCode::Linear(items),
                next_label: 5,
                python_version: crate::PythonVersion::Py314,
                ..Assembler::default()
            };
            assembler.record_normal_try_exit(start, target, None);
            let mut pipeline = assembler.into_optimization_pipeline();
            let provenance = &mut pipeline.exception_owner_provenance;
            provenance.ordinary_try_setups.insert(start, handler);
            provenance.setups.push((start, Some(location)));
            let mut owner = region(start, end, handler, false);
            owner.priority = ExceptionRegionPriority::Resolved { start: 0, end: 10 };
            provenance
                .normal_try_jump_owners
                .insert((start, end), vec![owner]);
            let original = pipeline.graph.copy_items();
            pipeline.restore_empty_try_jump_owners();
            let block = pipeline.graph.label_block(start).unwrap();
            if setup_depth == Some(2) && jump_depth == Some(2) {
                assert_eq!(owners_at(&pipeline, block, 0), [handler]);
                assert_eq!(
                    pipeline.graph[block]
                        .last_instruction()
                        .unwrap()
                        .exception_policy(),
                    ExceptionPolicy::default()
                );
            } else {
                assert_eq!(
                    pipeline.graph.copy_items(),
                    original,
                    "{setup_depth:?}, {jump_depth:?}"
                );
                assert!(pipeline.exception_regions.is_empty());
            }
        }
    }

    #[test]
    fn forward_exits_without_original_cleanup_owners_leave_the_copy_plan_unchanged() {
        let mut pipeline = two_exits();
        pipeline.exception_owner_provenance.normal_try_exits.clear();
        pipeline
            .exception_owner_provenance
            .normal_try_jump_owners
            .clear();
        let original_items = pipeline.graph.copy_items();
        let original_regions = format!("{:?}", pipeline.exception_regions);
        let original_next_label = pipeline.next_label;

        // The full planner still sees the authored break, forward joins and protected
        // ranges, but none can certify a path crossing an original cleanup.
        pipeline.prepare_normal_try_copy_plan_with_discards(
            &[],
            &FxHashMap::default(),
            &FxHashSet::default(),
            &FxHashMap::default(),
        );
        pipeline.prepare_normal_try_copy_plan();

        assert_eq!(pipeline.graph.copy_items(), original_items);
        assert_eq!(
            format!("{:?}", pipeline.exception_regions),
            original_regions
        );
        assert_eq!(pipeline.next_label, original_next_label);
        assert!(
            pipeline
                .exception_owner_provenance
                .normal_try_copy_steps
                .is_empty()
        );
    }

    #[test]
    fn label_only_intermediate_block_does_not_certify_an_early_jump_barrier() {
        let mut pipeline = two_exits();
        let empty = pipeline.graph.label_block(Label::from_u32(3)).unwrap();
        pipeline
            .graph
            .retain_items(empty, |item| matches!(item, Item::Label(_)));
        let before = format!("{:?}", pipeline.graph.iter_items().collect::<Vec<_>>());
        pipeline.prepare_normal_try_copy_plan();
        assert!(
            pipeline
                .exception_owner_provenance
                .normal_try_copy_steps
                .is_empty()
        );
        assert_eq!(
            before,
            format!("{:?}", pipeline.graph.iter_items().collect::<Vec<_>>())
        );
        assert_eq!(pipeline.exception_regions.len(), 3);
    }
    #[test]
    fn conditional_normal_try_join_requires_closed_original_edges() {
        use crate::target::opcodes::POP_JUMP_IF_FALSE;
        for case in [
            "ordinary",
            "hidden_nop",
            "two_failures",
            "folded_condition",
            "folded_loop_else",
            "unlocated_failure",
            "borrowed_source",
            "small_footer",
            "target_seam",
            "scheduled",
            "artificial_success",
            "artificial_body",
            "artificial_unlocated_prefix",
            "handler_entry",
        ] {
            let entry = Label::from_u32(2);
            let end = Label::from_u32(4);
            let mut source = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(entry),
                SourceLocation::new(8, 8, 4, 9),
                None,
            );
            source.jump_policy.defer_removal();
            if case == "borrowed_source" {
                source.jump_policy.mark_borrow_empty_block();
            }
            let mut items = vec![
                Item::Label(Label::from_u32(0)),
                item(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(entry),
                    if case == "unlocated_failure" { -1 } else { 7 },
                ),
            ];
            if case == "two_failures" {
                items.push(item(POP_JUMP_IF_FALSE, Operand::Forward(entry), 7));
            }
            if case == "artificial_success" {
                items.extend([
                    item(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(1)), 7),
                    item(POP_TOP, Operand::Value(0), 8),
                    Item::Label(Label::from_u32(1)),
                ]);
            } else {
                items.extend([
                    Item::Label(Label::from_u32(1)),
                    if case == "artificial_unlocated_prefix" {
                        item(NOP, Operand::Value(0), -1)
                    } else {
                        item(POP_TOP, Operand::Value(0), 8)
                    },
                ]);
            }
            items.extend([
                Item::Instruction(source),
                Item::Label(entry),
                item(JUMP_FORWARD, Operand::Forward(end), 8),
                Item::Label(Label::from_u32(3)),
                item(RETURN_VALUE, Operand::Value(0), 9),
                Item::Label(end),
            ]);
            for index in 0..if case == "small_footer" { 1 } else { 5 } {
                items.push(item(LOAD_CONST, Operand::Value(0), 10));
                if case == "target_seam" && index == 0 {
                    items.push(Item::Label(Label::from_u32(7)));
                }
            }
            items.extend([
                item(RETURN_VALUE, Operand::Value(0), 10),
                Item::Label(Label::from_u32(5)),
                item(RETURN_VALUE, Operand::Value(0), 11),
            ]);
            let mut assembler = Assembler {
                code: UnoptimizedCode::Linear(items),
                next_label: 8,
                ..Assembler::default()
            };
            assembler.record_normal_try_exit(entry, end, None);
            if case == "folded_condition" {
                assembler.record_folded_condition_predecessor(entry);
            }
            if case == "folded_loop_else" {
                assembler.record_folded_loop_else_predecessor(entry);
            }
            if case == "hidden_nop" {
                assembler.original_nop_targets.insert(entry);
            }
            if case == "handler_entry" {
                assembler
                    .exception_regions
                    .push(region(Label::from_u32(0), end, entry, false));
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            if case == "scheduled" {
                pipeline
                    .exception_owner_provenance
                    .normal_try_copy_steps
                    .push((Label::from_u32(1), entry, 2));
            }
            if matches!(
                case,
                "artificial_success" | "artificial_body" | "artificial_unlocated_prefix"
            ) {
                pipeline.graph.for_each_instruction_mut(|instruction| {
                    if instruction.opcode == JUMP_FORWARD
                        && instruction.operand() == Operand::Forward(entry)
                    {
                        instruction.no_location_policy.mark_artificial_eval_break();
                    }
                });
            }
            pipeline.prepare_conditional_normal_try_joins();
            let block = pipeline.graph.label_block(entry).unwrap();
            let instructions = pipeline.graph[block]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                instructions.len(),
                if matches!(case, "ordinary" | "artificial_body") {
                    2
                } else {
                    1
                },
                "{case}"
            );
            if matches!(case, "ordinary" | "artificial_body") {
                assert_eq!(instructions[0].opcode, NOP);
                assert!(instructions[0].fusion_policy.is_converted_pop_block());
                assert_eq!(instructions[0].location, SourceLocation::NONE);
                assert_eq!(instructions[1].location, SourceLocation::NONE);
                assert!(instructions[1].only_allows_unconditional_inlining());
                assert_eq!(
                    pipeline
                        .exception_owner_provenance
                        .normal_try_copy_pop_ranges
                        .len(),
                    1
                );
                assert_eq!(
                    pipeline
                        .exception_owner_provenance
                        .normal_try_copy_internal_labels
                        .len(),
                    1
                );
            } else {
                assert!(
                    pipeline
                        .exception_owner_provenance
                        .normal_try_copy_pop_ranges
                        .is_empty()
                );
            }
        }
    }

    // The producer's two-comparison condition, both assignment arms, an original
    // nested-if join, and the four-slot tuple footer. Capture before appending
    // the arms, exactly when the compiler has just marked CompareEnd.
    fn comparison_observer_fixture(retained: bool) -> OptimizationPipeline {
        use crate::target::opcodes::{
            BUILD_TUPLE, COMPARE_OP, COPY, LOAD_FAST, NOT_TAKEN, POP_JUMP_IF_FALSE, STORE_FAST,
            SWAP, TO_BOOL,
        };

        let cleanup = Label::from_u32(2);
        let body = Label::from_u32(3);
        let otherwise = Label::from_u32(4);
        let join = Label::from_u32(5);
        let footer = Label::from_u32(6);
        let mut bridge = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(body),
            SourceLocation::new(10, 10, 4, 15),
            None,
        );
        if retained {
            bridge.jump_policy.retain_nop();
        } else {
            bridge.jump_policy.prevent_small_exit_inlining();
        }
        let mut assembler = Assembler {
            code: UnoptimizedCode::Linear(vec![
                Item::Label(Label::from_u32(0)),
                item(LOAD_FAST, Operand::Value(0), 10),
                item(LOAD_FAST, Operand::Value(1), 10),
                item(SWAP, Operand::Value(2), 10),
                item(COPY, Operand::Value(2), 10),
                item(COMPARE_OP, Operand::Value(2), 10),
                item(TO_BOOL, Operand::Value(0), 10),
                item(POP_JUMP_IF_FALSE, Operand::Forward(cleanup), 10),
                item(NOT_TAKEN, Operand::Value(0), 10),
                item(LOAD_FAST, Operand::Value(2), 10),
                item(COMPARE_OP, Operand::Value(2), 10),
                item(TO_BOOL, Operand::Value(0), 10),
                item(POP_JUMP_IF_FALSE, Operand::Forward(otherwise), 10),
                item(NOT_TAKEN, Operand::Value(0), 10),
                Item::Instruction(bridge),
                Item::Label(cleanup),
                item(POP_TOP, Operand::Value(0), 10),
                item(JUMP_FORWARD, Operand::Forward(otherwise), 10),
                Item::Label(body),
            ]),
            next_label: 8,
            ..Assembler::default()
        };
        assembler.record_original_comparison_fallthrough(cleanup, body);
        assert_eq!(
            assembler
                .exception_owner_provenance
                .original_comparison_fallthroughs
                .len(),
            1,
        );
        let mut join_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(footer),
            SourceLocation::NONE,
            None,
        );
        join_jump.jump_policy.mark_original_empty_join();
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            unreachable!()
        };
        items.extend([
            item(LOAD_CONST, Operand::Value(0), 11),
            item(STORE_FAST, Operand::Value(3), 11),
            item(JUMP_FORWARD, Operand::Forward(join), 11),
            Item::Label(otherwise),
            item(LOAD_CONST, Operand::Value(1), 12),
            item(STORE_FAST, Operand::Value(3), 12),
            Item::Label(join),
            Item::Instruction(join_jump),
            Item::Label(footer),
            item(LOAD_FAST, Operand::Value(4), 13),
            item(LOAD_FAST, Operand::Value(3), 13),
            item(BUILD_TUPLE, Operand::Value(2), 13),
            item(RETURN_VALUE, Operand::Value(0), 13),
        ]);
        for label in [Label::from_u32(0), cleanup, body, otherwise, join, footer] {
            assembler.record_exception_owner_block_boundary(label);
        }
        assembler.into_optimization_pipeline()
    }

    #[test]
    fn comparison_observers_require_live_exact_unlabelled_records() {
        for retained in [false, true] {
            for case in [
                "intact",
                "missing witness",
                "labelled bridge",
                "removed failure",
                "changed failure",
                "removed pop",
                "changed pop",
            ] {
                let mut pipeline = comparison_observer_fixture(retained);
                let (_, _, records) = pipeline
                    .exception_owner_provenance
                    .original_comparison_fallthroughs[0];
                let observer = pipeline
                    .graph
                    .iter()
                    .find_map(|(block, contents)| {
                        (contents.last_instruction() == Some(&records[3])).then_some(block)
                    })
                    .unwrap();
                let original_observer = pipeline.graph[observer].items().to_vec();
                match case {
                    "intact" => {}
                    "missing witness" => pipeline
                        .exception_owner_provenance
                        .original_comparison_fallthroughs
                        .clear(),
                    "labelled bridge" => pipeline.graph.insert_item(
                        ItemPosition::new(observer, 0),
                        Item::Label(Label::from_u32(7)),
                    ),
                    "removed failure" | "removed pop" => {
                        let removed = records[if case == "removed failure" { 0 } else { 4 }];
                        for block in pipeline.graph.order().to_vec() {
                            pipeline
                                .graph
                                .retain_items(block, |item| *item != Item::Instruction(removed));
                        }
                    }
                    "changed failure" | "changed pop" => {
                        let changed = records[if case == "changed failure" { 0 } else { 4 }];
                        pipeline.graph.for_each_instruction_mut(|instruction| {
                            if *instruction == changed {
                                instruction.location.line += 1;
                            }
                        });
                    }
                    _ => unreachable!(),
                }
                pipeline.prepare_plain_tuple_copy_plan();
                assert_eq!(
                    !pipeline
                        .exception_owner_provenance
                        .normal_try_copy_steps
                        .is_empty(),
                    case == "intact",
                    "{case}, retained={retained}",
                );
                if case == "intact" {
                    // Model-only participation must not relabel, erase the late
                    // marker, or change the physical bridge's location/policies.
                    assert_eq!(pipeline.graph[observer].items(), original_observer);
                    assert!(pipeline.graph[observer].first_label().is_none());
                    assert_eq!(
                        pipeline
                            .exception_owner_provenance
                            .normal_try_copy_blocks
                            .len(),
                        2
                    );
                }
            }
        }
    }

    #[test]
    fn shared_match_tuple_plan_preserves_five_slot_join_and_live_inputs() {
        use crate::target::opcodes::{
            BUILD_TUPLE, COPY, END_FOR, FOR_ITER, LOAD_FAST, MATCH_CLASS, NOT_TAKEN, POP_ITER,
            POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_TRUE, STORE_FAST, TO_BOOL,
            YIELD_VALUE,
        };

        for case in [
            "ordinary",
            "generic return head",
            "no match end",
            "borrow return",
            "hidden return",
            "borrow join",
            "hidden condition",
            "folded condition",
            "marker owner",
            "marker fusion",
            "body no store",
            "hidden body",
            "protected body",
            "retained join",
            "wide tuple",
            "independent tail",
            "independent tail generic head",
            "independent tail borrowed",
            "independent tail no producer",
            "independent tail hidden",
            "independent tail incoming",
            "independent tail retained",
            "independent tail wrong target",
            "opaque loop",
            "opaque loop 315",
            "opaque loop deferred",
            "opaque loop deferred retained",
            "opaque loop iterator span",
            "opaque loop extra entry",
            "opaque loop fallthrough",
            "opaque loop hidden cleanup",
            "opaque class",
            "opaque truth",
            "opaque owned donor",
            "opaque class labelled",
            "opaque class hidden",
            "opaque class folded",
            "opaque class owner",
            "opaque class suspension",
            "opaque class incoming",
            "opaque class region",
            "opaque class marker span",
            "opaque class marker owner",
            "opaque truth copy producer",
            "opaque external donor",
        ] {
            let mut shared = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(Label::from_u32(5)),
                SourceLocation::NONE,
                None,
            );
            shared.no_location_policy.preserve_location();
            let mut success = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(Label::from_u32(6)),
                SourceLocation::new(12, 12, 4, 9),
                None,
            );
            success.jump_policy.mark_original_match_success();
            success.jump_policy.mark_interruptible();
            success.jump_policy.defer_removal();
            success.no_location_policy.mark_artificial_eval_break();
            let mut marker = Instruction::new(
                NOT_TAKEN,
                Operand::Value(0),
                SourceLocation::new(11, 11, 4, 9),
                None,
            );
            marker.set_exception_policy(super::super::super::ExceptionPolicy::NoneDynamicCondition);
            let mut assembler = Assembler {
                code: UnoptimizedCode::Linear(vec![
                    Item::Label(Label::from_u32(0)),
                    item(LOAD_CONST, Operand::Value(0), 9),
                    item(STORE_FAST, Operand::Value(1), 9),
                    item(JUMP_FORWARD, Operand::Forward(Label::from_u32(1)), 9),
                    Item::Label(Label::from_u32(1)),
                    Item::Instruction(shared),
                    Item::Label(Label::from_u32(2)),
                    item(LOAD_FAST, Operand::Value(0), 11),
                    item(TO_BOOL, Operand::Value(0), 11),
                    item(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(5)), 11),
                    Item::Instruction(marker),
                    Item::Label(Label::from_u32(3)),
                    item(LOAD_CONST, Operand::Value(1), 12),
                    item(STORE_FAST, Operand::Value(1), 12),
                    Item::Label(Label::from_u32(5)),
                    Item::Instruction(success),
                    Item::Label(Label::from_u32(6)),
                    item(LOAD_FAST, Operand::Value(2), 13),
                    item(LOAD_FAST, Operand::Value(1), 13),
                    item(BUILD_TUPLE, Operand::Value(2), 13),
                    item(RETURN_VALUE, Operand::Value(0), 13),
                ]),
                next_label: 8,
                ..Assembler::default()
            };
            for label in [
                Label::from_u32(0),
                Label::from_u32(1),
                Label::from_u32(2),
                Label::from_u32(3),
                Label::from_u32(5),
            ] {
                assembler.record_exception_owner_block_boundary(label);
            }
            assembler
                .nested_if_unlocated_join_blocks
                .insert(Label::from_u32(1));
            assembler
                .exception_owner_provenance
                .original_match_join_boundaries
                .insert(Label::from_u32(6));
            if case == "generic return head" {
                assembler.record_exception_owner_block_boundary(Label::from_u32(6));
            }
            if case.starts_with("independent tail") {
                let mut tail = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(Label::from_u32(6)),
                    SourceLocation::NONE,
                    None,
                );
                tail.jump_policy.mark_interruptible();
                let mut none_condition = Instruction::new(
                    crate::target::opcodes::POP_JUMP_IF_NONE,
                    Operand::Forward(Label::from_u32(9)),
                    SourceLocation::new(15, 15, 4, 9),
                    None,
                );
                none_condition
                    .local_policy
                    .add_hidden_exception_owner_slots(3);
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!()
                };
                let footer = items
                    .iter()
                    .position(|item| *item == Item::Label(Label::from_u32(6)))
                    .unwrap();
                items.splice(
                    footer..footer,
                    [
                        Item::Label(Label::from_u32(8)),
                        item(LOAD_FAST, Operand::Value(0), 15),
                        Item::Instruction(none_condition),
                        item(NOT_TAKEN, Operand::Value(0), 15),
                        Item::Label(Label::from_u32(10)),
                        item(LOAD_FAST, Operand::Value(0), 16),
                        item(STORE_FAST, Operand::Value(2), 16),
                        Item::Label(Label::from_u32(9)),
                        Item::Instruction(tail),
                    ],
                );
                assembler.next_label = 12;
                assembler.mark_conditional_threading_target(
                    Label::from_u32(9),
                    SourceLocation::new(15, 15, 4, 9),
                );
                if case == "independent tail generic head" {
                    assembler.record_exception_owner_block_boundary(Label::from_u32(9));
                }
            }
            if case.starts_with("opaque") {
                let record = |opcode, operand, line, depth| {
                    Item::Instruction(Instruction::new(
                        opcode,
                        operand,
                        SourceLocation::new(line, line, 4, 9),
                        Some(depth),
                    ))
                };
                let mut tail = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(Label::from_u32(6)),
                    SourceLocation::new(16, 16, 4, 9),
                    Some(0),
                );
                tail.jump_policy.mark_original_match_success();
                tail.jump_policy.mark_interruptible();
                let extra = if case.starts_with("opaque loop") {
                    assembler.python_version = if case == "opaque loop 315" {
                        crate::PythonVersion::Py315
                    } else {
                        crate::PythonVersion::Py314
                    };
                    let depth =
                        1 + u32::from(assembler.python_version == crate::PythonVersion::Py315);
                    tail.jump_policy.mark_borrow_empty_block();
                    if case.starts_with("opaque loop deferred") {
                        tail.jump_policy.defer_removal();
                        if case.ends_with("retained") {
                            tail.jump_policy.retain_redundant_nop_after_threading();
                        }
                    }
                    assembler.record_exception_owner_block_boundary(Label::from_u32(9));
                    let mut extra = vec![
                        Item::Label(Label::from_u32(8)),
                        record(
                            FOR_ITER,
                            Operand::Forward(Label::from_u32(9)),
                            16,
                            depth + 1,
                        ),
                        Item::Label(Label::from_u32(10)),
                        record(STORE_FAST, Operand::Value(3), 16, depth),
                        record(
                            JUMP_BACKWARD,
                            Operand::Backward(Label::from_u32(8)),
                            16,
                            depth,
                        ),
                        Item::Label(Label::from_u32(9)),
                        record(END_FOR, Operand::Value(0), 16, depth),
                        record(POP_ITER, Operand::Value(0), 16, 0),
                        Item::Instruction(tail),
                    ];
                    match case {
                        "opaque loop iterator span" => {
                            let Item::Instruction(iter) = &mut extra[1] else {
                                unreachable!()
                            };
                            iter.location = SourceLocation::new(17, 17, 4, 9);
                        }
                        "opaque loop extra entry" => {
                            extra[4] = record(
                                JUMP_FORWARD,
                                Operand::Forward(Label::from_u32(9)),
                                16,
                                depth + 1,
                            );
                            extra.insert(4, record(LOAD_CONST, Operand::Value(0), 16, depth + 1));
                        }
                        "opaque loop fallthrough" => {
                            extra[4] = record(LOAD_CONST, Operand::Value(0), 16, depth + 1);
                        }
                        "opaque loop hidden cleanup" => {
                            let Item::Instruction(cleanup) = &mut extra[6] else {
                                unreachable!()
                            };
                            cleanup.local_policy.add_hidden_exception_owner_slots(1);
                        }
                        _ => {}
                    }
                    extra
                } else {
                    let truth = case.starts_with("opaque truth");
                    let copied_truth = case == "opaque truth copy producer";
                    let branch_depth = u32::from(!truth || copied_truth);
                    let mut marker = Instruction::new(
                        NOT_TAKEN,
                        Operand::Value(0),
                        SourceLocation::new(15, 15, 4, 9),
                        Some(branch_depth),
                    );
                    if truth {
                        marker.set_exception_policy(
                            super::super::super::ExceptionPolicy::NoneDynamicCondition,
                        );
                        tail.jump_policy.defer_removal();
                    }
                    let mut extra = vec![Item::Label(Label::from_u32(8))];
                    if truth {
                        extra.push(record(LOAD_FAST, Operand::Value(0), 15, 1));
                        if copied_truth {
                            extra.push(record(COPY, Operand::Value(1), 15, 2));
                        }
                        extra.extend([
                            record(TO_BOOL, Operand::Value(0), 15, branch_depth + 1),
                            record(
                                POP_JUMP_IF_TRUE,
                                Operand::Forward(Label::from_u32(9)),
                                15,
                                branch_depth,
                            ),
                        ]);
                    } else {
                        let mut copy = Instruction::new(
                            COPY,
                            Operand::Value(1),
                            SourceLocation::new(15, 15, 4, 9),
                            Some(2),
                        );
                        copy.local_policy.add_hidden_exception_owner_slots(2);
                        extra.extend([
                            record(LOAD_FAST, Operand::Value(0), 15, 1),
                            record(LOAD_FAST, Operand::Value(1), 15, 2),
                            record(LOAD_CONST, Operand::Value(0), 15, 3),
                            record(MATCH_CLASS, Operand::Value(0), 15, 1),
                            Item::Instruction(copy),
                            record(
                                POP_JUMP_IF_NONE,
                                Operand::Forward(Label::from_u32(9)),
                                15,
                                1,
                            ),
                        ]);
                    }
                    let marker_index = extra.len();
                    extra.push(Item::Instruction(marker));
                    if branch_depth != 0 {
                        extra.push(record(POP_TOP, Operand::Value(0), 15, 0));
                    }
                    let load_index = extra.len();
                    extra.extend([
                        record(LOAD_FAST, Operand::Value(0), 16, 1),
                        record(STORE_FAST, Operand::Value(2), 16, 0),
                    ]);
                    match case {
                        "opaque class hidden" | "opaque class folded" | "opaque class owner" => {
                            let Item::Instruction(work) = &mut extra[load_index] else {
                                unreachable!()
                            };
                            match case {
                                "opaque class hidden" => {
                                    work.local_policy.add_hidden_exception_owner_slots(1);
                                }
                                "opaque class folded" => work.add_pre_fold_extra_instructions(1),
                                "opaque class owner" => work.set_exception_policy(
                                    super::super::super::ExceptionPolicy::NonePresent,
                                ),
                                _ => unreachable!(),
                            }
                        }
                        "opaque class suspension" => {
                            extra.insert(
                                load_index + 1,
                                record(YIELD_VALUE, Operand::Value(0), 16, 1),
                            );
                        }
                        "opaque class marker span" => {
                            let Item::Instruction(marker) = &mut extra[marker_index] else {
                                unreachable!()
                            };
                            marker.location = SourceLocation::new(16, 16, 4, 9);
                        }
                        "opaque class marker owner" => {
                            let Item::Instruction(marker) = &mut extra[marker_index] else {
                                unreachable!()
                            };
                            marker.set_exception_policy(
                                super::super::super::ExceptionPolicy::NoneDynamicCondition,
                            );
                        }
                        "opaque owned donor" => {
                            let Item::Instruction(pop) = &mut extra[marker_index + 1] else {
                                unreachable!()
                            };
                            pop.location = SourceLocation::new(16, 16, 4, 9);
                            pop.fusion_policy.mark_location_from_next();
                            let Item::Instruction(load) = &mut extra[load_index] else {
                                unreachable!()
                            };
                            load.local_policy =
                                super::super::super::ir::LocalPolicy::for_load(true, false);
                        }
                        "opaque external donor" => {
                            extra.push(record(LOAD_FAST, Operand::Value(0), 16, 1));
                            let mut pop = Instruction::new(
                                POP_TOP,
                                Operand::Value(0),
                                SourceLocation::new(16, 16, 4, 9),
                                Some(0),
                            );
                            pop.fusion_policy.mark_location_from_next();
                            extra.push(Item::Instruction(pop));
                        }
                        _ => {}
                    }
                    if matches!(case, "opaque class labelled" | "opaque class incoming") {
                        extra.insert(marker_index, Item::Label(Label::from_u32(11)));
                    }
                    extra.extend([Item::Instruction(tail), Item::Label(Label::from_u32(9))]);
                    if branch_depth != 0 {
                        extra.push(record(POP_TOP, Operand::Value(0), 17, 0));
                    }
                    extra.extend([
                        record(LOAD_CONST, Operand::Value(0), 17, 1),
                        record(RETURN_VALUE, Operand::Value(0), 17, 0),
                    ]);
                    extra
                };
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!()
                };
                let footer = items
                    .iter()
                    .position(|item| *item == Item::Label(Label::from_u32(6)))
                    .unwrap();
                items.splice(footer..footer, extra);
                assembler.next_label = 12;
                if case == "opaque class region" {
                    assembler.exception_regions.push(region(
                        Label::from_u32(8),
                        Label::from_u32(9),
                        Label::from_u32(0),
                        true,
                    ));
                }
                if case == "opaque class incoming" {
                    items[3] = record(JUMP_FORWARD, Operand::Forward(Label::from_u32(11)), 9, 1);
                    items.insert(3, record(LOAD_CONST, Operand::Value(0), 9, 1));
                }
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            let condition = pipeline.graph.label_block(Label::from_u32(2)).unwrap();
            let body = pipeline.graph.label_block(Label::from_u32(3)).unwrap();
            let join = pipeline.graph.label_block(Label::from_u32(5)).unwrap();
            let footer = pipeline.graph.label_block(Label::from_u32(6)).unwrap();
            match case {
                "ordinary"
                | "generic return head"
                | "independent tail"
                | "independent tail generic head" => {}
                "no match end" => pipeline
                    .exception_owner_provenance
                    .original_match_join_boundaries
                    .clear(),
                "borrow return" => {
                    pipeline.borrow_block_aliases.insert(Label::from_u32(6));
                }
                "hidden return" => pipeline
                    .graph
                    .instructions_mut(footer)
                    .next()
                    .unwrap()
                    .add_pre_fold_extra_instructions(1),
                "borrow join" => {
                    pipeline
                        .graph
                        .insert_item(ItemPosition::new(join, 0), Item::Label(Label::from_u32(7)));
                    pipeline.borrow_block_aliases.insert(Label::from_u32(7));
                }
                "hidden condition" => pipeline
                    .graph
                    .instructions_mut(condition)
                    .next()
                    .unwrap()
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                "folded condition" => {
                    pipeline
                        .folded_original_predecessor_targets
                        .insert(Label::from_u32(2));
                }
                "marker owner" | "marker fusion" => {
                    pipeline.graph.for_each_instruction_mut(|instruction| {
                        if instruction.opcode == NOT_TAKEN {
                            if case == "marker owner" {
                                instruction.set_exception_policy(
                                    super::super::super::ExceptionPolicy::NonePresent,
                                );
                            } else {
                                instruction.fusion_policy.prevent_with_previous();
                            }
                        }
                    });
                }
                "body no store" => {
                    pipeline
                        .graph
                        .instructions_mut(body)
                        .next_back()
                        .unwrap()
                        .opcode = POP_TOP;
                }
                "hidden body" => pipeline
                    .graph
                    .instructions_mut(body)
                    .next()
                    .unwrap()
                    .add_pre_fold_extra_instructions(1),
                "protected body" => {
                    pipeline.exception_regions.push(region(
                        Label::from_u32(3),
                        Label::from_u32(5),
                        Label::from_u32(0),
                        false,
                    ));
                }
                "retained join" => pipeline
                    .graph
                    .instructions_mut(join)
                    .next_back()
                    .unwrap()
                    .jump_policy
                    .retain_redundant_nop_after_threading(),
                "wide tuple" => pipeline.graph.for_each_instruction_mut(|instruction| {
                    if instruction.opcode == BUILD_TUPLE {
                        instruction.set_operand(Operand::Value(3));
                    }
                }),
                "independent tail borrowed" => {
                    let tail = pipeline.graph.label_block(Label::from_u32(9)).unwrap();
                    pipeline
                        .graph
                        .insert_item(ItemPosition::new(tail, 0), Item::Label(Label::from_u32(11)));
                    pipeline.borrow_block_aliases.insert(Label::from_u32(11));
                }
                "independent tail no producer" => pipeline.conditional_threading_targets.clear(),
                "independent tail hidden"
                | "independent tail retained"
                | "independent tail wrong target" => {
                    let tail = pipeline.graph.label_block(Label::from_u32(9)).unwrap();
                    let jump = pipeline.graph.instructions_mut(tail).next_back().unwrap();
                    match case {
                        "independent tail hidden" => {
                            jump.local_policy.add_hidden_exception_owner_slots(1);
                        }
                        "independent tail retained" => {
                            jump.jump_policy.retain_redundant_nop_after_threading();
                        }
                        "independent tail wrong target" => {
                            jump.set_operand(Operand::Forward(Label::from_u32(5)));
                        }
                        _ => unreachable!(),
                    }
                }
                "independent tail incoming" => {
                    let source = pipeline.graph.label_block(Label::from_u32(0)).unwrap();
                    pipeline
                        .graph
                        .instructions_mut(source)
                        .next_back()
                        .unwrap()
                        .set_operand(Operand::Forward(Label::from_u32(9)));
                }
                _ if case.starts_with("opaque") => {}
                _ => unreachable!(),
            }
            let before = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
            let opaque_before = pipeline
                .graph
                .order()
                .iter()
                .copied()
                .skip_while(|block| {
                    pipeline.graph[*block].first_label() != Some(Label::from_u32(8))
                })
                .take_while(|block| *block != footer)
                .map(|block| (block, pipeline.graph[block].items().to_vec()))
                .collect::<Vec<_>>();
            let condition_before = pipeline.graph[condition].items().to_vec();
            let body_before = pipeline.graph[body].items().to_vec();
            let footer_before = pipeline.graph[footer].items().to_vec();
            pipeline.prepare_plain_tuple_copy_plan();
            if matches!(
                case,
                "ordinary"
                    | "generic return head"
                    | "independent tail"
                    | "independent tail generic head"
                    | "opaque loop"
                    | "opaque loop 315"
                    | "opaque loop deferred"
                    | "opaque class"
                    | "opaque truth"
                    | "opaque owned donor"
            ) {
                // The original ascending rounds see O only after it has grown to
                // NOP + four return slots. A must keep its jump to that five-slot O.
                assert_eq!(
                    pipeline.exception_owner_provenance.normal_try_copy_steps,
                    [
                        (Label::from_u32(0), Label::from_u32(1), 1),
                        (Label::from_u32(1), Label::from_u32(5), 1),
                        (Label::from_u32(5), Label::from_u32(6), 4),
                        (Label::from_u32(1), Label::from_u32(6), 4),
                    ],
                    "{case}"
                );
                assert_eq!(pipeline.graph[condition].items(), condition_before);
                assert_eq!(pipeline.graph[body].items(), body_before);
                assert_eq!(pipeline.graph[footer].items(), footer_before);
                if case.starts_with("opaque") {
                    for (block, items) in opaque_before {
                        assert_eq!(pipeline.graph[block].items(), items, "{case}");
                    }
                }
                let planned_join = pipeline.graph[join].last_instruction().unwrap();
                assert_eq!(planned_join.location, SourceLocation::NONE);
                assert!(!planned_join.jump_policy.defers_redundant_removal());
            } else {
                assert!(
                    pipeline
                        .exception_owner_provenance
                        .normal_try_copy_steps
                        .is_empty(),
                    "{case}"
                );
                assert_eq!(
                    pipeline.graph.iter_items().copied().collect::<Vec<_>>(),
                    before,
                    "{case}"
                );
            }
        }
    }

    #[test]
    fn guarded_finally_tuple_schedule_requires_intact_private_arrays() {
        use crate::target::opcodes::{
            BUILD_TUPLE, DELETE_FAST, LOAD_FAST, POP_JUMP_IF_TRUE, STORE_FAST,
        };

        for case in [
            "accepted",
            "legacy",
            "extra slot",
            "foreign alias",
            "concrete owner",
            "changed depth",
            "changed policy",
            "addressable cleanup",
            "adjacent cleanup",
            "wide tuple",
            "name return",
            "missing boundary",
        ] {
            let [entry, normal, cleanup, join, middle, tail, handler, alias] =
                [0, 1, 2, 3, 4, 5, 6, 7].map(Label::from_u32);
            let location = SourceLocation::new(10, 10, 4, 9);
            let record = |opcode, operand, depth| {
                Item::Instruction(Instruction::new(opcode, operand, location, Some(depth)))
            };
            let mut first =
                Instruction::new(JUMP_FORWARD, Operand::Forward(middle), location, Some(0));
            first.jump_policy.mark_original_empty_join();
            first.jump_policy.retain_redundant_nop_after_threading();
            first.no_location_policy.set_borrow_fallthrough(
                crate::assembler::ir::BorrowFallthrough::RetainedNopReachable,
            );
            let mut second = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(tail),
                SourceLocation::NONE,
                Some(u32::from(case == "changed depth")),
            );
            if case != "changed policy" {
                second.jump_policy.mark_borrow_empty_block();
            }
            let mut items = vec![
                Item::Label(entry),
                record(LOAD_FAST, Operand::Value(0), 1),
                record(
                    POP_JUMP_IF_TRUE,
                    Operand::Forward(if case == "addressable cleanup" {
                        cleanup
                    } else {
                        join
                    }),
                    0,
                ),
                Item::Label(normal),
                record(JUMP_FORWARD, Operand::Forward(join), 0),
                Item::Label(cleanup),
                record(POP_EXCEPT, Operand::Value(0), 0),
                record(LOAD_CONST, Operand::Value(0), 1),
                record(STORE_FAST, Operand::Value(3), 0),
                record(DELETE_FAST, Operand::Value(3), 0),
                record(JUMP_FORWARD, Operand::Forward(join), 0),
            ];
            if case != "adjacent cleanup" {
                items.extend([
                    Item::Label(handler),
                    record(LOAD_CONST, Operand::Value(0), 1),
                    record(RETURN_VALUE, Operand::Value(0), 0),
                ]);
            }
            items.extend([
                Item::Label(join),
                Item::Instruction(first),
                Item::Label(middle),
            ]);
            if case == "foreign alias" {
                items.push(Item::Label(alias));
            }
            if case == "extra slot" {
                items.push(record(NOP, Operand::Value(0), 0));
            }
            items.extend([
                Item::Instruction(second),
                Item::Label(tail),
                record(LOAD_FAST, Operand::Value(1), 1),
            ]);
            if case != "name return" {
                items.push(record(LOAD_FAST, Operand::Value(2), 2));
                if case == "wide tuple" {
                    items.push(record(LOAD_FAST, Operand::Value(3), 3));
                }
                items.push(record(
                    BUILD_TUPLE,
                    Operand::Value(if case == "wide tuple" { 3 } else { 2 }),
                    1,
                ));
            }
            items.push(record(RETURN_VALUE, Operand::Value(0), 0));
            let mut assembler = Assembler {
                code: UnoptimizedCode::Linear(items),
                python_version: if case == "legacy" {
                    crate::PythonVersion::Py313
                } else {
                    crate::PythonVersion::Py315
                },
                next_label: 8,
                ..Assembler::default()
            };
            assembler.record_normal_try_exit(normal, join, None);
            for label in [join, middle, tail] {
                if case != "missing boundary" || label != middle {
                    assembler.record_exception_owner_block_boundary(label);
                }
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            if case == "foreign alias" {
                pipeline.borrow_block_aliases.insert(alias);
            }
            if case == "concrete owner" {
                pipeline
                    .exception_regions
                    .push(region(middle, tail, handler, false));
            }
            let before = pipeline.graph.copy_items();
            pipeline.prepare_normal_try_copy_plan();
            let steps = &pipeline.exception_owner_provenance.normal_try_copy_steps;
            if case == "accepted" {
                assert_eq!(
                    *steps,
                    [
                        (normal, join, 1),
                        (cleanup, join, 1),
                        (join, middle, 1),
                        (middle, tail, 4),
                        (join, tail, 4),
                    ]
                );
            } else {
                assert!(steps.is_empty(), "{case}");
                assert_eq!(pipeline.graph.copy_items(), before, "{case}");
            }
        }
    }

    #[test]
    fn copy_model_reports_observer_participation_as_both_source_and_target() {
        // Without a persistent located prefix, an unlocated singleton is copied
        // into its predecessor and then copies the small return itself. The
        // observer verifier must never silently omit either participation kind.
        let mut nodes = (0..2)
            .map(|id| OriginalNode {
                id: BasicBlockId::from_usize(id),
                prefix: Vec::new(),
                tail: OriginalTail::Jump {
                    target: BasicBlockId::from_usize(id + 1),
                    located: false,
                    interruptible: false,
                },
            })
            .collect::<Vec<_>>();
        nodes.push(OriginalNode {
            id: BasicBlockId::from_usize(2),
            prefix: Vec::new(),
            tail: OriginalTail::LocalTupleReturn { located: [true; 4] },
        });
        let result = plan_normal_try_copies(&nodes).unwrap();
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.source.index() == 0 && step.target.index() == 1)
        );
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.source.index() == 1 && step.target.index() == 2)
        );
    }
}
