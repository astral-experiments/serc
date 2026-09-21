use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    Instruction, Item, NOP, NOT_TAKEN, Operand, POP_TOP, STORE_FAST, SWAP, UNPACK_EX,
    UNPACK_SEQUENCE, capture_cleanup_rotation, ends_scope, single_capture_cleanup_vacancies,
};
use crate::assembler::OptimizationPipeline;
use crate::assembler::block_graph::{BlockGraph, ItemPosition};

impl OptimizationPipeline {
    pub(in crate::assembler) fn remove_redundant_swaps_before_pops(&mut self) -> bool {
        let graph = &self.graph;
        if !graph.features().has_swap() {
            return false;
        }
        let instructions = graph
            .item_positions()
            .into_iter()
            .filter_map(|position| {
                let Item::Instruction(instruction) = *graph.item(position) else {
                    return None;
                };
                Some((position, instruction))
            })
            .collect::<Vec<_>>();
        let mut previous = None;
        let mut redundant = Vec::new();
        for (index, (position, instruction)) in instructions.iter().copied().enumerate() {
            let remove = if instruction.opcode == SWAP
                && let Operand::Value(depth) = instruction.operand()
            {
                let pop_count = usize::try_from(depth).unwrap_or(usize::MAX);
                previous.is_some_and(|previous: Instruction| previous.opcode == SWAP)
                    && instructions[index + 1..]
                        .get(..pop_count)
                        .is_some_and(|following| {
                            // CPython cannot fold a rotation across distinct line events.
                            let line = following
                                .first()
                                .map_or(-1, |(_, instruction)| instruction.location.line);
                            following.iter().all(|(_, instruction)| {
                                instruction.opcode == POP_TOP
                                    && (line < 0 || instruction.location.line == line)
                            })
                        })
            } else {
                false
            };
            if remove {
                redundant.push(position);
            } else {
                previous = Some(instruction);
            }
        }

        let graph = &mut self.graph;
        let changed = !redundant.is_empty();
        for position in redundant.into_iter().rev() {
            graph.remove_item(position);
        }
        changed
    }

    /// Replaces each run of stack swaps with the shortest equivalent sequence.
    ///
    /// This is the first half of CPython's `swaptimize` pass. Reconstructing the
    /// permutation from the end of the run guarantees that the replacement fits
    /// in the original instruction slots. Reports whether any original slot changed.
    pub(in crate::assembler) fn optimize_swap_runs(
        &mut self,
    ) -> (Vec<Vec<ItemPosition>>, FxHashSet<ItemPosition>, bool) {
        const VISITED: usize = usize::MAX;

        let graph = &self.graph;
        if !graph.features().has_swap() {
            return (Vec::new(), FxHashSet::default(), false);
        }
        let mut block_labels = FxHashSet::default();
        for item in graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
            {
                block_labels.insert(label);
            }
        }
        for region in &self.exception_regions {
            block_labels.extend([region.start, region.end, region.target]);
        }
        block_labels.extend(self.preserved_block_boundaries.iter().copied());
        let blocks = graph.partition_positions(&block_labels, |instruction| {
            !matches!(instruction.operand(), Operand::Value(_)) || ends_scope(instruction.opcode)
        });
        // Admit cleanups from their complete original partition before any swap run is
        // rewritten. A newly simplified suffix cannot certify an emitted owner slot.
        let mut capture_cleanup_swaps = FxHashSet::default();
        for block in &blocks {
            for rotation in block.windows(6) {
                if !matches!(graph.item(rotation[0]), Item::Instruction(prefix)
                    if matches!(prefix.opcode, UNPACK_SEQUENCE | POP_TOP))
                {
                    continue;
                }
                let items = std::array::from_fn::<_, 6, _>(|index| *graph.item(rotation[index]));
                if single_capture_cleanup_vacancies(&items).is_some() {
                    capture_cleanup_swaps.extend([rotation[1], rotation[2]]);
                }
            }
            if !block.iter().any(|position| {
                matches!(graph.item(*position), Item::Instruction(instruction)
                    if (instruction.opcode == UNPACK_SEQUENCE
                        && matches!(instruction.operand(), Operand::Value(0 | 1)))
                        || matches!(instruction.opcode, UNPACK_EX | POP_TOP | NOT_TAKEN))
            }) {
                continue;
            }
            let items = block
                .iter()
                .map(|position| *graph.item(*position))
                .collect::<Vec<_>>();
            for index in 0..items.len() {
                if let Some(rotation) = capture_cleanup_rotation(&items[index..]) {
                    capture_cleanup_swaps.extend(
                        block[index + 1..index + 1 + rotation.swap_count]
                            .iter()
                            .copied(),
                    );
                }
            }
        }
        let graph = &mut self.graph;
        let mut changed = false;
        for block in &blocks {
            let instruction_positions = block
                .iter()
                .copied()
                .filter(|position| matches!(graph.item(*position), Item::Instruction(_)))
                .collect::<Vec<_>>();
            let mut position = 0;
            while position < instruction_positions.len() {
                let Item::Instruction(first) = *graph.item(instruction_positions[position]) else {
                    unreachable!();
                };
                if first.opcode != SWAP {
                    position += 1;
                    continue;
                }
                let Operand::Value(first_depth) = first.operand() else {
                    unreachable!();
                };
                let mut depth = usize::try_from(first_depth).unwrap();
                let mut run_end = position + 1;
                let mut multiple_swaps = false;
                while run_end < instruction_positions.len() {
                    let Item::Instruction(instruction) =
                        *graph.item(instruction_positions[run_end])
                    else {
                        unreachable!();
                    };
                    if instruction.opcode == SWAP {
                        let Operand::Value(argument) = instruction.operand() else {
                            unreachable!();
                        };
                        depth = depth.max(usize::try_from(argument).unwrap());
                        multiple_swaps = true;
                    } else if instruction.opcode != NOP {
                        break;
                    }
                    run_end += 1;
                }
                if !multiple_swaps {
                    position += 1;
                    continue;
                }

                let mut stack = (0..depth).collect::<Vec<_>>();
                for instruction_position in &instruction_positions[position..run_end] {
                    let Item::Instruction(instruction) = *graph.item(*instruction_position) else {
                        unreachable!();
                    };
                    if instruction.opcode == SWAP {
                        let Operand::Value(argument) = instruction.operand() else {
                            unreachable!();
                        };
                        stack.swap(0, usize::try_from(argument).unwrap() - 1);
                    }
                }

                let mut current = run_end;
                for index in 0..depth {
                    if stack[index] == VISITED || stack[index] == index {
                        continue;
                    }
                    let mut stack_index = index;
                    loop {
                        if stack_index != 0 {
                            current -= 1;
                            let Some(instruction) =
                                graph.instruction_mut(instruction_positions[current])
                            else {
                                unreachable!();
                            };
                            let argument =
                                Operand::Value(u32::try_from(stack_index + 1).unwrap_or(u32::MAX));
                            changed |=
                                instruction.opcode != SWAP || instruction.operand() != argument;
                            instruction.opcode = SWAP;
                            instruction.set_operand(argument);
                        }
                        if stack[stack_index] == VISITED {
                            debug_assert_eq!(stack_index, index);
                            break;
                        }
                        let next = stack[stack_index];
                        stack[stack_index] = VISITED;
                        stack_index = next;
                    }
                }
                while current > position {
                    current -= 1;
                    let Some(instruction) = graph.instruction_mut(instruction_positions[current])
                    else {
                        unreachable!();
                    };
                    changed |=
                        instruction.opcode != NOP || instruction.operand() != Operand::Value(0);
                    instruction.opcode = NOP;
                    instruction.set_operand(Operand::Value(0));
                }
                position = run_end;
            }
        }
        (blocks, capture_cleanup_swaps, changed)
    }

    /// Applies stack swaps statically by reordering local stores and pops.
    ///
    /// This is the `apply_static_swaps` half of CPython's `swaptimize` pass and
    /// must run before adjacent local stores are fused. Reverse traversal applies
    /// consecutive swaps from right to left, matching CPython's per-run handoff.
    pub(in crate::assembler) fn apply_static_swaps(
        &mut self,
        blocks: (Vec<Vec<ItemPosition>>, FxHashSet<ItemPosition>),
    ) {
        fn swappable(instruction: &Instruction) -> bool {
            matches!(instruction.opcode, POP_TOP | STORE_FAST)
        }

        fn stored_local(instruction: &Instruction) -> Option<u32> {
            (instruction.opcode == STORE_FAST).then(|| match instruction.operand() {
                Operand::Value(argument) => argument,
                Operand::Deferred(_) | Operand::Forward(_) | Operand::Backward(_) => {
                    unreachable!()
                }
            })
        }

        fn next_swappable(
            graph: &BlockGraph,
            positions: &[ItemPosition],
            mut index: usize,
            end: usize,
            line: Option<i32>,
        ) -> Option<usize> {
            while index < end {
                let Item::Instruction(instruction) = *graph.item(positions[index]) else {
                    index += 1;
                    continue;
                };
                if line.is_some_and(|line| instruction.location.line != line) {
                    return None;
                }
                if instruction.opcode == NOP {
                    index += 1;
                    continue;
                }
                return swappable(&instruction).then_some(index);
            }
            None
        }

        let (blocks, capture_cleanup_swaps) = blocks;
        let join_origins =
            std::mem::take(&mut self.exception_owner_provenance.normal_try_copy_join_origins);
        let retired_labels = &self
            .exception_owner_provenance
            .retired_normal_try_copy_labels;
        let graph = &mut self.graph;
        // Resolve original labels only within this pass. A saved item offset could
        // otherwise name a different store after earlier graph rewrites.
        let join_stores = graph
            .order()
            .windows(2)
            .filter_map(|pair| {
                let label = graph[pair[1]].first_label()?;
                if retired_labels.contains(&label) {
                    return None;
                }
                let origin = *join_origins.get(&label)?;
                if retired_labels.contains(&origin.predecessor)
                    || graph[pair[0]].first_label() != Some(origin.predecessor)
                    || graph[pair[0]].last_instruction() != Some(&origin.store)
                    || !matches!(graph[pair[1]].items(), [Item::Label(head), Item::Instruction(jump)]
                        if *head == label && *jump == origin.jump)
                {
                    return None;
                }
                let offset = graph[pair[0]]
                    .items()
                    .iter()
                    .rposition(|item| matches!(item, Item::Instruction(_)))?;
                Some((ItemPosition::new(pair[0], offset), (label, origin)))
            })
            .collect::<FxHashMap<_, _>>();
        for positions in blocks {
            let mut index = positions.len();
            while index > 0 {
                index -= 1;
                let Item::Instruction(swap) = *graph.item(positions[index]) else {
                    continue;
                };
                if swap.opcode != SWAP {
                    continue;
                }
                let Operand::Value(depth) = swap.operand() else {
                    unreachable!();
                };
                let Some(first) =
                    next_swappable(graph, &positions, index + 1, positions.len(), None)
                else {
                    continue;
                };
                let Item::Instruction(first_instruction) = *graph.item(positions[first]) else {
                    unreachable!();
                };
                let line = (first_instruction.location.line >= 0)
                    .then_some(first_instruction.location.line);
                let mut last = first;
                let mut valid = true;
                for _ in 1..depth {
                    let Some(next) =
                        next_swappable(graph, &positions, last + 1, positions.len(), line)
                    else {
                        valid = false;
                        break;
                    };
                    last = next;
                }
                if !valid {
                    continue;
                }
                let Item::Instruction(first_instruction) = *graph.item(positions[first]) else {
                    unreachable!();
                };
                let Item::Instruction(last_instruction) = *graph.item(positions[last]) else {
                    unreachable!();
                };
                let first_store = stored_local(&first_instruction);
                let last_store = stored_local(&last_instruction);
                if first_store.is_none()
                    && last_store.is_none()
                    && !capture_cleanup_swaps.contains(&positions[index])
                {
                    // A complete conditional-fallthrough capture rotation proves both pop
                    // records are identical and its compacted-slot count is known at emission.
                    let rotation = index
                        .checked_sub(2)
                        .and_then(|start| positions.get(start..index + 4))
                        .map(|positions| {
                            std::array::from_fn::<_, 6, _>(|index| *graph.item(positions[index]))
                        });
                    if !rotation.as_ref().is_some_and(|rotation| {
                        matches!(rotation[0], Item::Instruction(marker) if marker.opcode == NOT_TAKEN)
                            && single_capture_cleanup_vacancies(rotation).is_some()
                    }) {
                        continue;
                    }
                }
                if first_store.is_some() || last_store.is_some() {
                    if first_store == last_store
                        || positions[first + 1..last].iter().any(|position| {
                            let Item::Instruction(instruction) = graph.item(*position) else {
                                return false;
                            };
                            stored_local(instruction).is_some_and(|local| {
                                Some(local) == first_store || Some(local) == last_store
                            })
                        })
                    {
                        continue;
                    }
                }

                // 3.12 resolves lines after static swaps, so the exchanged restore
                // supplies the successor's location instead of the original store.
                if self.python_version >= crate::PythonVersion::Py313
                    && first_instruction.local_policy.may_store_null()
                    && last_instruction.opcode == STORE_FAST
                    && !last_instruction.local_policy.may_store_null()
                {
                    if let Some((label, origin)) = join_stores.get(&positions[last])
                        && origin.store == last_instruction
                        && graph.label_block(*label).is_some_and(|block| {
                            matches!(graph[block].items(), [Item::Label(head), Item::Instruction(jump)]
                                if *head == *label && *jump == origin.jump)
                        })
                    {
                        // The original assignment store really moves in this exchange.
                        // Its adjacent selected join stays unlocated through copying;
                        // only the original replacement NOP receives this donor later.
                        self.exception_owner_provenance
                            .normal_try_copy_join_donors
                            .insert(*label, (origin.jump, last_instruction.location));
                    }
                    // Native resolves unlocated successors before exchanging the target
                    // store with a comprehension restore. Retain that original origin.
                    for position in &positions[last + 1..] {
                        let Some(next) = graph.instruction_mut(*position) else {
                            break;
                        };
                        if next.location.line >= 0 || next.no_location_policy.preserves_location() {
                            break;
                        }
                        next.location = last_instruction.location;
                        if !matches!(next.operand(), Operand::Value(_)) || ends_scope(next.opcode) {
                            break;
                        }
                    }
                }

                let Some(swap) = graph.instruction_mut(positions[index]) else {
                    unreachable!();
                };
                swap.opcode = NOP;
                swap.set_operand(Operand::Value(0));
                *graph
                    .instruction_mut(positions[first])
                    .expect("swappable item is an instruction") = last_instruction;
                *graph
                    .instruction_mut(positions[last])
                    .expect("swappable item is an instruction") = first_instruction;
            }
        }
    }
}
