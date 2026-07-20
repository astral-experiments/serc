use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    ExceptionExclusion, ExceptionRegion, Instruction, Item, JUMP_BACKWARD,
    JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, Label, Operand, OptimizationPipeline, SourceLocation,
    block_has_fallthrough, block_jump_target, cleanup_block_creation_order, ends_scope,
};
use crate::assembler::block_graph::{BasicBlockId, BlockGraph};
use crate::target::is_conditional_jump;
use crate::target::opcodes::{
    BUILD_TUPLE, CALL, CLEANUP_THROW, END_ASYNC_FOR, END_SEND, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL,
    NOP, RAISE_VARARGS, RETURN_VALUE, STORE_ATTR, STORE_FAST,
};

const WARM_BLOCK: u8 = 1;
const COLD_BLOCK: u8 = 2;

struct ExitBlockAnalysis {
    positions: Vec<usize>,
    predecessors: Vec<usize>,
    jump_predecessors: Vec<Option<BasicBlockId>>,
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
            && graph[source]
                .last_instruction()
                .is_some_and(|instruction| instruction.jump_policy.inline_small_exit());
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
        let source_retains_direct_exit_nop = graph[source]
            .last_instruction()
            .is_some_and(|instruction| instruction.jump_policy.retains_direct_target_nop());
        let source_copies_split_return = graph[source]
            .last_instruction()
            .is_some_and(|instruction| instruction.no_location_policy.copies_split_return_exit());
        let target_is_small_exit = target_pre_fusion_size <= 4
            && target_terminal_opcode.is_some_and(|opcode| {
                ends_scope(opcode)
                    && (target_chain_end == target_position
                        || opcode == RAISE_VARARGS
                        || (opcode == RETURN_VALUE
                            && (source_retains_direct_exit_nop || source_copies_split_return)))
            });
        let source_has_fallthrough = block_has_fallthrough(graph[source].items());
        let inline_small_exit = !source_has_fallthrough
            && target_is_small_exit
            && graph[source]
                .last_instruction()
                .is_some_and(|instruction| instruction.jump_policy.inline_small_exit())
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
                exception_handler_blocks,
                requires_copy: true,
            };
        }
    }

    ExitBlockAnalysis {
        positions,
        predecessors,
        jump_predecessors,
        exception_handler_blocks,
        requires_copy: false,
    }
}

// The original pass performs these location repairs even when it creates no copies, so the fast
// path must preserve them before returning.
fn propagate_exit_block_locations(
    graph: &mut BlockGraph,
    order: &[BasicBlockId],
    analysis: &ExitBlockAnalysis,
) {
    for target in order.iter().copied() {
        if analysis.exception_handler_blocks[target.index()] {
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
                    let exclusion = if instruction.exception_policy.exclusion()
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
                        instruction.exception_policy.exclude(exception_exclusion);
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

        // `codegen_add_yield_from` creates the `CLEANUP_THROW` block before it resumes
        // compiling the expression that follows the await. Our structured compiler creates the
        // same labels up front, but emits async-iterator cleanup blocks while unwinding nested
        // generators. Restore CPython's creation order when the cold blocks are collected.
        let cleanup_throw_positions = order
            .iter()
            .enumerate()
            .filter_map(|(position, block)| {
                (position + 1 < order.len()
                    && block_jump_target(graph[order[position + 1]].items()).is_some()
                    && graph[*block].items().iter().any(|item| {
                        matches!(item, Item::Instruction(instruction) if instruction.opcode == CLEANUP_THROW)
                    }))
                .then_some(position)
            })
            .collect::<Vec<_>>();
        if cleanup_throw_positions.windows(2).any(|positions| {
            cleanup_block_creation_order(graph[order[positions[0]]].items())
                > cleanup_block_creation_order(graph[order[positions[1]]].items())
        }) {
            let mut cleanup_throw_blocks = vec![false; temperatures.len()];
            for position in &cleanup_throw_positions {
                cleanup_throw_blocks[order[*position].index()] = true;
            }
            // Move each cleanup and its continuation as a pair. Intervening cold handler blocks
            // retain their relative position, as they do in CPython's block list.
            loop {
                let ordered_cleanup_blocks = order
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| cleanup_throw_blocks[block.index()])
                    .collect::<Vec<_>>();
                let Some(inversion) = ordered_cleanup_blocks.windows(2).find(|blocks_in_order| {
                    cleanup_block_creation_order(graph[*blocks_in_order[0].1].items())
                        > cleanup_block_creation_order(graph[*blocks_in_order[1].1].items())
                }) else {
                    break;
                };
                let insertion_position = inversion[0].0;
                let group_position = inversion[1].0;
                let group = order
                    .drain(group_position..group_position + 2)
                    .collect::<Vec<_>>();
                order.splice(insertion_position..insertion_position, group);
            }

            // The outer inlined comprehension's successful restore was compiled after its
            // exceptional restore in our unwind order. CPython compiled it before the nested
            // handlers, so collecting those handlers also makes the terminal restore the
            // fallthrough from `END_ASYNC_FOR` and eliminates the synthetic forward jump.
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
                (exits_scope == Some(true)).then_some((position + 2, start, end))
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
                if instruction.opcode == JUMP_FORWARD {
                    instruction.opcode = if instruction.fusion_policy.is_authored_break() {
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

    pub(in crate::assembler) fn duplicate_exit_blocks(&mut self) {
        let graph = &mut self.graph;
        if graph.is_normalized_jump_free_single_block(&self.preserved_block_boundaries) {
            return;
        }

        if !graph.is_normalized(&self.preserved_block_boundaries) {
            graph.normalize_blocks(&self.preserved_block_boundaries);
        }
        if graph.len() < 2 {
            return;
        }

        let order = graph.order().to_vec();
        let analysis = exit_block_analysis(graph, &order, &self.exception_regions);
        if !analysis.requires_copy {
            propagate_exit_block_locations(graph, &order, &analysis);
            return;
        }
        let ExitBlockAnalysis {
            positions,
            predecessors,
            jump_predecessors: _,
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
            let artificial_eval_break =
                graph[block].last_instruction().is_some_and(|instruction| {
                    instruction.no_location_policy.is_artificial_eval_break()
                });
            exit_without_unique_predecessor[block.index()] =
                predecessors[block.index()] > 1 && (unlocated_scope_exit || artificial_eval_break);
        }

        let mut inline_copies = FxHashMap::<_, Vec<Vec<Item>>>::default();
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
        let mut sources = order.clone();
        let mut source_cursor = 0;
        while let Some(&source) = sources.get(source_cursor) {
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
            let source_retains_direct_exit_nop = graph[source]
                .last_instruction()
                .is_some_and(|instruction| instruction.jump_policy.retains_direct_target_nop());
            let source_copies_split_return =
                graph[source].last_instruction().is_some_and(|instruction| {
                    instruction.no_location_policy.copies_split_return_exit()
                });
            let target_is_small_exit = target_pre_fusion_size <= 4
                && target_terminal_opcode.is_some_and(|opcode| {
                    ends_scope(opcode)
                        // An exception-boundary split before a raise represents CPython's
                        // `SETUP_FINALLY` inside one basic block. Split returns normally keep
                        // their boundary, but direct-target NOP retention proves that this
                        // concrete return is copied onto the source edge.
                        && (target_chain_end == target_position
                            || opcode == RAISE_VARARGS
                            || (opcode == RETURN_VALUE
                                && (source_retains_direct_exit_nop || source_copies_split_return)))
                });
            let source_allows_small_exit_inlining = graph[source]
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
            let target_is_no_location_no_fallthrough = target_has_no_location
                && target_allows_no_location_block_inlining
                && !block_has_fallthrough(graph[target_exit].items());
            let inline_small_exit = !source_has_fallthrough
                && target_is_small_exit
                && source_allows_small_exit_inlining
                && !target_contains_end_send
                && !exception_handler_blocks[target.index()];
            let inline_no_location_block =
                !source_has_fallthrough && target_is_no_location_no_fallthrough;
            let copy_conditional_split_return = source_has_fallthrough
                && source_copies_split_return
                && target_is_small_exit
                && remaining_predecessors[target.index()] > 1
                && !exception_handler_blocks[target.index()];
            if exit_without_unique_predecessor[target.index()]
                && target_pre_fusion_size > 4
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
                continue;
            }
            if !(inline_small_exit || inline_no_location_block || copy_conditional_split_return)
                && (!(exit_without_unique_predecessor[target.index()]
                    || target_is_no_location_no_fallthrough)
                    || remaining_predecessors[target.index()] <= 1)
            {
                continue;
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
            let preserve_inlined_jump_nop = inline_small_exit
                && source_location.line >= 0
                && graph[source].last_instruction().is_some_and(|instruction| {
                    instruction
                        .jump_policy
                        .retains_nop_for(instruction.operand())
                });
            let copied_label = Label(self.next_label);
            self.next_label += 1;
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
                        || copy_conditional_split_return)
                        && (!inline_no_location_block
                            || predecessors[source.index()] <= 1
                            || inherits_shared_predecessor)
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
                let mut block_needs_location = true;
                for item in &mut copied {
                    match item {
                        Item::Label(label) if copied_exception_split_labels.contains(label) => {
                            block_needs_location = true;
                        }
                        Item::Instruction(instruction) if block_needs_location => {
                            instruction.location = source_location;
                            block_needs_location = false;
                        }
                        Item::Instruction(_) | Item::Label(_) => {}
                    }
                }
            }
            if inline_small_exit || inline_no_location_block {
                let coroutine_owner_targets = self
                    .exception_regions
                    .iter()
                    .zip(&region_memberships)
                    .filter(|(region, membership)| {
                        region.depth == 0
                            && region.preserve_lasti
                            && membership.contains(&positions[target.index()])
                    })
                    .map(|(region, _)| region.target)
                    .collect::<FxHashSet<_>>();
                let entirely_excluded_regions = region_memberships
                    .iter()
                    .enumerate()
                    .filter_map(|(index, membership)| {
                        let region = &self.exception_regions[index];
                        (membership.contains(&positions[source.index()])
                            && !membership.contains(&positions[target.index()])
                            // Generator-region exclusions can split one logical coroutine owner.
                            // Keep the copied exit protected when the target is in its sibling.
                            && !(region.depth == 0
                                && region.preserve_lasti
                                && coroutine_owner_targets.contains(&region.target)))
                        .then_some(index)
                    })
                    .collect::<Vec<_>>();
                // CPython carries an internal protected-region end into an exit copy when the
                // return entry still has its preserved boundary alias. A single-label shared
                // return is reached only after branch threading, so its region holes belong to
                // the individual branch copies rather than this target-chain copy.
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
                        (target_has_preserved_boundary_alias
                            && membership.contains(&positions[source.index()])
                            && membership.contains(&positions[target.index()]))
                        .then(|| copied_labels.get(&region.end).copied())
                        .flatten()
                        .map(|start| (index, start))
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
                && region_memberships.iter().all(|membership| {
                    !membership.contains(&positions[source.index()])
                        && !membership.contains(&positions[target.index()])
                });
            if inline_borrowed_tuple_exit {
                copied.retain(|item| !matches!(item, Item::Label(label) if *label == copied_label));
                graph.append_to_block(source, copied);
            } else if inline_no_location_block {
                let copied_jump_target =
                    block_jump_target(&copied).and_then(|label| graph.label_block(label));
                graph.append_to_block(source, copied);
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
                // CPython repeats no-line block inlining to a fixed point. The copied block can
                // itself end in a jump to another synthetic cleanup block, so revisit this source
                // before considering its earlier jump predecessors.
                sources.push(source);
                // The copied jump can turn this source into another eligible unlocated block.
                // Revisit earlier jump predecessors until the chain reaches a fixed point.
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
                    });
                    start = exclusion_end;
                }
                self.exception_regions.push(ExceptionRegion {
                    start,
                    end: region.end,
                    target: region.target,
                    depth: region.depth,
                    preserve_lasti: region.preserve_lasti,
                });
            }
        }
        self.exception_regions.extend(copied_exception_regions);

        for target in order.iter().copied() {
            if exception_handler_blocks[target.index()] {
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
            if !graph[target].items().iter().all(|item| {
                !matches!(item, Item::Instruction(instruction) if instruction.location.line >= 0)
            }) {
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

        let mut final_order = Vec::new();
        for block in order {
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
        }
        graph
            .replace_order_and_normalize(final_order, &self.preserved_block_boundaries)
            .expect("exit-block duplication produced an invalid emission order");
    }
}
