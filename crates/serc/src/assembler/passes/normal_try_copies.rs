use rustc_hash::{FxHashMap, FxHashSet};

use super::super::block_graph::{BasicBlockId, ItemPosition};
use super::super::exception_owners::TupleJoinLocationOrigin;
use super::super::is_unconditional_jump;
use super::super::{
    ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label, Operand,
    OptimizationPipeline, SourceLocation,
};
use super::normal_try_copy_plan::{OriginalNode, OriginalTail, PrefixSlot, plan_normal_try_copies};
use crate::target::opcodes::{JUMP_BACKWARD, JUMP_FORWARD, NOP};

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
            && matches!(load.operand(), Operand::Value(_))
            && ret.opcode == RETURN_VALUE
            && ret.operand() == Operand::Value(0)
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
        && matches!(first.operand(), Operand::Value(_))
        && second.opcode == LOAD_FAST
        && matches!(second.operand(), Operand::Value(_))
        && tuple.opcode == BUILD_TUPLE
        && tuple.operand() == Operand::Value(2)
        && ret.opcode == RETURN_VALUE
        && ret.operand() == Operand::Value(0)
        && [first, second, tuple, ret].iter().all(|instruction| {
            instruction.location.line >= 0
                && instruction.pre_fold_extra_instructions() == 0
                && !instruction.fusion_policy.is_converted_pop_block()
                && !instruction.fusion_policy.is_authored_break()
        }))
    .then_some([true; 4])
}

impl OptimizationPipeline {
    /// Schedules exact local-tuple exits through their original unprotected arrays.
    pub(in crate::assembler) fn prepare_plain_tuple_copy_plan(&mut self) {
        use crate::target::opcodes::{LOAD_GLOBAL, STORE_FAST};

        if !self
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
        // A complete original array has only head labels and no erased records.
        // Reject only concrete regions touching selected arrays: coroutine scope
        // fallback and unrelated SEND cleanup regions do not split these arrays.
        let intact = |block| {
            let mut seen_instruction = false;
            let mut original_head = false;
            for item in graph[block].items() {
                match item {
                    Item::Label(label) => {
                        if seen_instruction
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
                        if instruction.pre_fold_extra_instructions() != 0
                            || instruction.fusion_policy.is_converted_pop_block()
                            || instruction.fusion_policy.is_authored_break()
                        {
                            return false;
                        }
                    }
                }
            }
            let (start, end) = ranges[&block];
            original_head
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
        let mut returns = FxHashMap::default();
        for &block in order {
            if !intact(block) {
                continue;
            }
            if let Some(located) = original_local_tuple_return(graph[block].items()) {
                returns.insert(block, located);
            }
        }
        // Limit this plan to tuple tails reached by a certified original join;
        // unrelated tuple exits in the same compilation unit keep their own path.
        returns.retain(|block, _| {
            graph.iter_items().any(|item| {
                matches!(item, Item::Instruction(instruction)
                if instruction.opcode == JUMP_FORWARD
                    && instruction.jump_policy.is_original_empty_join()
                    && matches!(instruction.operand(), Operand::Forward(label)
                        if graph.label_block(label) == Some(*block)))
            })
        });
        if returns.is_empty() {
            return;
        }
        let mut selected = returns.keys().copied().collect::<FxHashSet<_>>();
        let mut sources = FxHashSet::default();
        let mut joins = FxHashSet::default();
        let mut shared_joins = FxHashSet::default();
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
                // A partial nested if can contribute another original empty array
                // to the same closed graph. Its conditional predecessors keep the
                // original jump unlocated, but private copies inherit their own lines.
                // Require the producer's exact head, not an arbitrary unlocated jump.
                let shared_join = jump.location == SourceLocation::NONE
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
                        == 1;
                if !intact(block)
                    || jump.opcode != JUMP_FORWARD
                    || !matches!(jump.operand(), Operand::Forward(_))
                    || positions[&block] >= positions[&target]
                    || !jump.jump_policy.inline_small_exit()
                    || jump.jump_policy.always_retains_nop()
                    || jump.jump_policy.borrow_empty_block()
                    || (jump.jump_policy.defers_redundant_removal()
                        && !(jump.jump_policy.is_original_empty_join()
                            && jump.jump_policy.preserves_nop_after_threading()))
                    || (jump.no_location_policy.preserves_location() && !shared_join)
                {
                    return;
                }
                if shared_join {
                    shared_joins.insert(block);
                } else if jump.jump_policy.is_original_empty_join() {
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
                sources.insert(block);
                selected.insert(block);
            }
            if before == selected.len() {
                break;
            }
        }
        if joins.is_empty() {
            return;
        }
        // The producer also certifies a sole conditional predecessor after a
        // terminating guard. That join has a real inherited test location, so
        // its original-empty bit alone cannot authorize resetting the line.
        if graph.iter_items().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if !is_unconditional_jump(instruction.opcode)
                    && matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if graph.label_block(label).is_some_and(|target| joins.contains(&target))))
        }) {
            return;
        }
        let mut nodes = Vec::new();
        for &block in order {
            if let Some(&located) = returns.get(&block) {
                nodes.push(OriginalNode {
                    id: block.index(),
                    prefix: Vec::new(),
                    tail: OriginalTail::LocalTupleReturn { located },
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
                let Operand::Forward(label) = jump.operand() else {
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
                    id: block.index(),
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
                        target: target.index(),
                        located: !is_join && jump.location.line >= 0,
                        interruptible: jump.jump_policy.is_interruptible(),
                    },
                });
            }
        }
        let Some(result) = plan_normal_try_copies(&nodes) else {
            return;
        };
        if result.steps.is_empty() {
            return;
        }
        // Head-only admission above preserves every entry label. No POP_BLOCK,
        // synthetic instruction, owner hole, or alternate internal label is added.
        let labels = selected
            .iter()
            .map(|&block| {
                (
                    block.index(),
                    graph[block].first_label().expect("original head label"),
                )
            })
            .collect::<FxHashMap<_, _>>();
        let copy_blocks = order
            .iter()
            .copied()
            .filter(|block| sources.contains(block))
            .map(|block| labels[&block.index()])
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
                jump.jump_policy.consume_original_join_copy_prediction();
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
                // comprehension-restore exchange must certify the donor later.
                join_origins.insert(
                    labels[&block.index()],
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
                || yield_value.operand() != Operand::Value(1)
                || cleanup_throw.opcode != CLEANUP_THROW
                || cleanup_throw.operand() != Operand::Value(0)
                || end_send.opcode != END_SEND
                || end_send.operand() != Operand::Value(0)
                || yield_value.depth_after() != Some(region.depth)
                || cleanup_throw.depth_after() != Some(region.depth)
                || end_send.depth_after() != region.depth.checked_sub(1)
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

    /// Restores the owner of an empty try's unchanged jump into a large live prefix.
    fn restore_uncopied_empty_try_jump_owners(&mut self) {
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
        jump_owners.sort_by_key(|&(&(start, end), _)| (start.0, end.0));
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
            let Operand::Forward(target) = jump.operand() else {
                continue;
            };
            if owners.is_empty()
                || end_offset != start_offset + 1
                || instructions.get(start_offset).copied() != Some(jump)
                || head.is_empty()
                || head.iter().any(|item| !matches!(item, Item::Label(_)))
                || jump.opcode != JUMP_FORWARD
                || jump.location.line < 0
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
            let Some(target_block) = graph.label_block(target) else {
                continue;
            };
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
                            || !matches!(instruction.operand(), Operand::Value(_))
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
            if prefix == 5 {
                selected.push((start_offset, start.0, end.0, block, owners.clone()));
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
        self.restore_uncopied_empty_try_jump_owners();
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
                            || matches!(
                                instruction.operand(),
                                Operand::Forward(_) | Operand::Backward(_)
                            )
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
                if !matches!(instruction.operand(), Operand::Forward(_))
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
                    && let Operand::Forward(label) = instruction.operand()
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
            let pop_start = Label(self.next_label);
            let pop_end = Label(self.next_label + 1);
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

    /// Restores original cleanup arrays for a closed ordinary-try/constant-or-local-tuple-return graph.
    ///
    /// These records precede location propagation and NOP compaction in CPython. Keeping
    /// the original `POP_BLOCK` and Pass slots lets the existing copier carry their labels
    /// and exception ranges through the same ascending, once-per-round copy schedule.
    pub(in crate::assembler) fn prepare_normal_try_copy_plan(&mut self) {
        let pass_prefixes = self.restore_normal_try_pass_prefixes();
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
            let located = original_local_tuple_return(items)?;
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
            original_head.then_some(OriginalTail::LocalTupleReturn { located })
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
        let mut empty_joins = FxHashMap::default();
        for &block in order {
            if exits.contains_key(&block) {
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
            let Operand::Forward(label) = jump.operand() else {
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
            empty_joins.insert(block, target);
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
        let successor = |block| {
            exits
                .get(&block)
                .map(|(_, target, _)| *target)
                .or_else(|| empty_joins.get(&block).copied())
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
            let Operand::Forward(label) = jump.operand() else {
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
                crossed_cleanup |= exits.contains_key(&current);
                current = target;
                if returns.contains_key(&current) {
                    if crossed_cleanup {
                        entry_returns.insert(current);
                    }
                    break;
                }
            }
        }
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
        if selected.iter().any(|block| !graph.iter_items().any(|item| matches!(item,
            Item::Instruction(instruction)
                if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if graph.label_block(label) == Some(*block)))))
        { return; }
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
                if let Some(target) = empty_joins.get(&join)
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
            .filter(|block| empty_joins.contains_key(block))
            .collect::<FxHashSet<_>>();
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
                && let Operand::Forward(target) = jump.operand()
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
        if !selected.is_subset(&sources) || !selected_joins.is_subset(&sources) {
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
                    id: block.index(),
                    prefix: Vec::new(),
                    tail: returns[&block],
                });
            } else if sources.contains(&block) {
                let jump = graph[block].last_instruction().expect("certified jump");
                let Operand::Forward(target) = jump.operand() else {
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
                    // No POP_BLOCK belongs to an ordinary original empty join.
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
                    id: block.index(),
                    prefix,
                    tail: OriginalTail::Jump {
                        target: target.index(),
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
                Some((start_offset, start.0, end.0, owners))
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
                let label = Label(self.next_label);
                self.next_label += 1;
                self.graph.insert_label(block, 0, label);
                label
            };
            labels.insert(block.index(), label);
        }
        // Restore original absence only after the complete schedule was accepted.
        // The compiler's inherited final-arm line is not authored on this join.
        for block in ordered_joins {
            let jump = self
                .graph
                .instructions_mut(block)
                .next_back()
                .expect("certified original empty join");
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
            let pop_start = Label(self.next_label);
            let pop_end = Label(self.next_label + 1);
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
            sources.iter().map(|block| labels[&block.index()]).collect();
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
    use crate::assembler::{Assembler, ExceptionRegionPriority, UnoptimizedCode};
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
            Operand::Forward(Label(2)),
            SourceLocation::new(11, 11, 8, 13),
            None,
        );
        source.fusion_policy.mark_authored_break();
        let mut assembler = Assembler {
            code: UnoptimizedCode::Linear(vec![
                Item::Label(Label(0)),
                item(POP_EXCEPT, Operand::Value(0), 11),
                item(POP_TOP, Operand::Value(0), 11),
                Item::Instruction(source),
                Item::Label(Label(1)),
                item(RETURN_VALUE, Operand::Value(0), 1),
                Item::Label(Label(2)),
                item(JUMP_FORWARD, Operand::Forward(Label(4)), 5),
                Item::Label(Label(3)),
                item(RETURN_VALUE, Operand::Value(0), 2),
                Item::Label(Label(4)),
                item(JUMP_FORWARD, Operand::Forward(Label(6)), 3),
                Item::Label(Label(5)),
                item(RETURN_VALUE, Operand::Value(0), 2),
                Item::Label(Label(6)),
                item(NOP, Operand::Value(0), 20),
                item(LOAD_CONST, Operand::Value(0), 20),
                item(RETURN_VALUE, Operand::Value(0), 20),
                Item::Label(Label(7)),
                item(RETURN_VALUE, Operand::Value(0), 21),
                Item::Label(Label(8)),
                item(RETURN_VALUE, Operand::Value(0), 22),
                Item::Label(Label(9)),
                item(RETURN_VALUE, Operand::Value(0), 23),
            ]),
            exception_regions: vec![
                region(Label(0), Label(2), Label(7), false),
                region(Label(0), Label(4), Label(8), false),
                region(Label(0), Label(7), Label(9), true),
            ],
            next_label: 10,
            ..Assembler::default()
        };
        assembler.record_normal_try_exit(Label(2), Label(4), None);
        assembler.record_normal_try_exit(Label(4), Label(6), None);
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
        owners.sort_by_key(|label| label.0);
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
        for label in [Label(2), Label(4)] {
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
            assert_eq!(owners_at(&pipeline, block, 0), [Label(9)]);
        }
        let absence = pipeline.capture_original_location_absence();
        let copies = pipeline.duplicate_exit_blocks(&absence, &mut Vec::new());
        let first_copy = *copies.first().expect("first original cleanup copy");
        // Normalization separates labels inside a copied array. Select its exact
        // ordered range instead of assuming the original source remains one block.
        let copied = instructions_between(&pipeline, first_copy, Label(1));
        let [
            (pop_block, pop_ordinal, pop),
            (jump_block, jump_ordinal, jump),
        ] = copied.as_slice()
        else {
            panic!("first cleanup copy must contain exactly POP_BLOCK and its intermediate jump");
        };
        assert_eq!(jump.operand(), Operand::Forward(Label(4)));
        assert_eq!(
            owners_at(&pipeline, *jump_block, *jump_ordinal),
            [Label(8), Label(9)]
        );
        assert!(pop.fusion_policy.is_converted_pop_block());
        assert_eq!(owners_at(&pipeline, *pop_block, *pop_ordinal), [Label(9)]);
        let source = instructions_between(&pipeline, Label(0), first_copy);
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
            [Label(7), Label(8), Label(9)]
        );
    }
    #[test]
    fn label_only_intermediate_block_does_not_certify_an_early_jump_barrier() {
        let mut pipeline = two_exits();
        let empty = pipeline.graph.label_block(Label(3)).unwrap();
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
            let entry = Label(2);
            let end = Label(4);
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
                Item::Label(Label(0)),
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
                    item(POP_JUMP_IF_FALSE, Operand::Forward(Label(1)), 7),
                    item(POP_TOP, Operand::Value(0), 8),
                    Item::Label(Label(1)),
                ]);
            } else {
                items.extend([
                    Item::Label(Label(1)),
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
                Item::Label(Label(3)),
                item(RETURN_VALUE, Operand::Value(0), 9),
                Item::Label(end),
            ]);
            for index in 0..if case == "small_footer" { 1 } else { 5 } {
                items.push(item(LOAD_CONST, Operand::Value(0), 10));
                if case == "target_seam" && index == 0 {
                    items.push(Item::Label(Label(7)));
                }
            }
            items.extend([
                item(RETURN_VALUE, Operand::Value(0), 10),
                Item::Label(Label(5)),
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
                    .push(region(Label(0), end, entry, false));
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            if case == "scheduled" {
                pipeline
                    .exception_owner_provenance
                    .normal_try_copy_steps
                    .push((Label(1), entry, 2));
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
}
