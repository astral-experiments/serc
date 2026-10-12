use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    Instruction, Item, NOP, NOT_TAKEN, Operand, POP_TOP, STORE_FAST, SWAP, SourceLocation,
    UNPACK_EX, UNPACK_SEQUENCE, capture_cleanup_rotation, ends_scope,
    single_capture_cleanup_vacancies,
};
use crate::assembler::block_graph::{BasicBlockId, BlockGraph, ItemPosition};
use crate::assembler::{Label, OptimizationPipeline};

/// Pass-local positions from the existing swap partition; labels still delimit source arrays.
pub(in crate::assembler) struct SwapBlock {
    positions: Vec<ItemPosition>,
    rewritten_nops: FxHashSet<ItemPosition>,
    original_donors: FxHashMap<ItemPosition, Instruction>,
    composed_capture_swaps: FxHashSet<ItemPosition>,
}

/// Certifies complete capture rotations separated by mapping cleanup pops.
fn composed_capture_cleanup(items: &[Item]) -> Option<Vec<usize>> {
    let [Item::Instruction(prefix), Item::Instruction(first), ..] = items else {
        return None;
    };
    let mut depth = first.value_argument()?;
    if !matches!(
        (prefix.opcode, prefix.value_argument()),
        (UNPACK_SEQUENCE, Some(1)) | (UNPACK_EX, Some(0))
    ) || first.opcode != SWAP
        || depth < 4
        || prefix.depth_after() != Some(depth)
        || prefix.location.line < 0
    {
        return None;
    }
    let mut index = 1;
    let mut swaps = Vec::new();
    let mut rotations = 0;
    let mut consumer_line = None;
    loop {
        let Item::Instruction(first_swap) = items.get(index)? else {
            return None;
        };
        for distance in (2..=depth).rev() {
            let Item::Instruction(swap) = items.get(index)? else {
                return None;
            };
            if swap.opcode != SWAP
                || swap.value_argument() != Some(distance)
                || swap.location != first_swap.location
                || swap.depth_after() != Some(depth)
            {
                return None;
            }
            swaps.push(index);
            index += 1;
        }
        rotations += 1;
        let mut pops = 0;
        let mut pop_line = None;
        while let Some(Item::Instruction(pop)) = items.get(index)
            && pop.opcode == POP_TOP
        {
            let line = *pop_line.get_or_insert(pop.location.line);
            if pop.value_argument() != Some(0) || pop.location.line != line {
                return None;
            }
            depth = depth.checked_sub(1)?;
            if pop.depth_after() != Some(depth) {
                return None;
            }
            pops += 1;
            index += 1;
        }
        consumer_line = pop_line.or(consumer_line);
        if !matches!(items.get(index), Some(Item::Instruction(next)) if next.opcode == SWAP) {
            break;
        }
        if pops < 2 || depth < 2 {
            return None;
        }
    }
    if rotations < 2 || depth < 2 {
        return None;
    }
    let mut stores = FxHashSet::default();
    for remaining in (0..depth).rev() {
        let Item::Instruction(store) = items.get(index)? else {
            return None;
        };
        if store.opcode != STORE_FAST
            || store.local_policy.may_store_null()
            || store.depth_after() != Some(remaining)
            || Some(store.location.line) != consumer_line
            || !stores.insert(store.value_argument()?)
        {
            return None;
        }
        index += 1;
    }
    if !matches!(items.get(index), Some(Item::Instruction(next))
        if next.location.line >= 0 && Some(next.location.line) != consumer_line)
    {
        return None;
    }
    for item in &items[..index] {
        let Item::Instruction(instruction) = item else {
            return None;
        };
        if instruction.location.line < 0
            || instruction.no_location_policy != super::super::ir::NoLocationPolicy::default()
            || instruction.fusion_policy != super::super::ir::FusionPolicy::default()
            || instruction.jump_policy != super::super::ir::JumpPolicy::default()
            || instruction.local_policy != super::super::ir::LocalPolicy::default()
            || instruction.exception_policy() != super::super::ExceptionPolicy::NoneUnchanged
            || instruction.pre_fold_extra_instructions() != 0
        {
            return None;
        }
    }
    Some(swaps)
}

fn transport_capture_pop_donors(
    donors: &mut FxHashMap<ItemPosition, Instruction>,
    certified: bool,
    (first, first_instruction): (ItemPosition, Instruction),
    (last, last_instruction): (ItemPosition, Instruction),
) {
    if certified
        && first != last
        && first_instruction.opcode == POP_TOP
        && last_instruction.opcode == POP_TOP
        && first_instruction.location.line == last_instruction.location.line
        && donors.get(&first) == Some(&first_instruction)
        && donors.get(&last) == Some(&last_instruction)
    {
        donors.insert(first, last_instruction);
        donors.insert(last, first_instruction);
    }
}

/// Retires a rewritten swap's line only when a surviving record in the same array has it.
fn clear_redundant_swap_nop_locations(
    graph: &mut BlockGraph,
    positions: &[ItemPosition],
    rewritten: &FxHashSet<ItemPosition>,
    original_donors: &FxHashMap<ItemPosition, Instruction>,
) {
    if rewritten.is_empty() {
        return;
    }
    let mut previous_line = None;
    for &position in positions {
        let Some(instruction) = graph.instruction_mut(position) else {
            previous_line = None;
            continue;
        };
        if instruction.opcode == NOP {
            if rewritten.contains(&position)
                && instruction.location.line >= 0
                && previous_line == Some(instruction.location.line)
            {
                // CPython compacts these slots after both swap phases, before later
                // copying and fusion. A later donor rewrite cannot revive this NOP.
                // Keep the original slot for owner accounting, but never restore its line.
                instruction.location = SourceLocation::NONE;
                instruction.no_location_policy.preserve_location();
            } else {
                previous_line = None;
            }
        } else {
            // Capture unpack/discard records precede their rotations in the native
            // array. Stores and other donors can move across later copy seams.
            previous_line = (matches!(instruction.opcode, UNPACK_SEQUENCE | POP_TOP)
                && original_donors.get(&position) == Some(instruction))
            .then_some(instruction.location.line);
        }
    }
}

/// Rewritten swap slots that native NOP compaction removes before local-pair fusion.
pub(in crate::assembler) struct StaticSwapStoreGap {
    block: BasicBlockId,
    prefix: Vec<Item>,
    gap: std::ops::Range<usize>,
}

impl StaticSwapStoreGap {
    /// Transport only the entry label inserted by the existing exit-copy preparation.
    pub(in crate::assembler) fn prepend_copy_entry(
        &mut self,
        graph: &BlockGraph,
        block: BasicBlockId,
        label: Label,
    ) {
        if self.block == block && graph[block].items().starts_with(&self.prefix) {
            self.prefix.insert(0, Item::Label(label));
            self.gap.start += 1;
            self.gap.end += 1;
        }
    }
}

fn capture_static_swap_store_gaps(
    graph: &BlockGraph,
    positions: &[ItemPosition],
    rewritten: &FxHashSet<ItemPosition>,
    gaps: &mut Vec<StaticSwapStoreGap>,
) {
    if rewritten.is_empty() {
        return;
    }
    for (index, &position) in positions.iter().enumerate() {
        let Item::Instruction(first) = graph.item(position) else {
            continue;
        };
        if first.opcode != STORE_FAST {
            continue;
        }
        let mut end = position.offset() + 1;
        for &next in &positions[index + 1..] {
            if next.block() != position.block() || next.offset() != end {
                break;
            }
            let Item::Instruction(instruction) = graph.item(next) else {
                break;
            };
            if rewritten.contains(&next)
                && instruction.opcode == NOP
                && (instruction.location.line < 0
                    || instruction.location.line == first.location.line)
            {
                end += 1;
                continue;
            }
            if end > position.offset() + 1
                && instruction.opcode == STORE_FAST
                && super::locals::fuse_pair(first, instruction).is_some()
            {
                gaps.push(StaticSwapStoreGap {
                    block: position.block(),
                    prefix: graph[position.block()].items()[..=end].to_vec(),
                    gap: position.offset() + 1..end,
                });
            }
            break;
        }
    }
}

impl OptimizationPipeline {
    /// Keeps original slot counts through copying, then compacts only unchanged swap gaps.
    pub(in crate::assembler) fn compact_static_swap_store_gaps(
        &mut self,
        gaps: Vec<StaticSwapStoreGap>,
    ) -> bool {
        let mut changed = false;
        // Later gaps precede earlier ones here, so their removal cannot shift a
        // still-pending prefix. Retired or changed source arrays lose admission.
        for gap in gaps.into_iter().rev() {
            if !self.graph.order().contains(&gap.block)
                || !self.graph[gap.block].items().starts_with(&gap.prefix)
            {
                continue;
            }
            for offset in gap.gap.rev() {
                self.graph.remove_item_at(gap.block, offset);
            }
            changed = true;
        }
        changed
    }

    pub(in crate::assembler) fn remove_redundant_swaps_before_pops(&mut self) -> bool {
        let graph = &self.graph;
        if !graph.features().has_swap() {
            return false;
        }
        let mut instructions = graph.order().iter().copied().flat_map(|block| {
            graph[block]
                .items()
                .iter()
                .enumerate()
                .filter_map(move |(offset, item)| match item {
                    Item::Label(_) => None,
                    Item::Instruction(instruction) => {
                        Some((ItemPosition::new(block, offset), instruction))
                    }
                })
        });
        let mut previous_was_swap = false;
        let mut redundant = Vec::new();
        let mut pop_location_swaps = Vec::new();
        while let Some((position, instruction)) = instructions.next() {
            // A removed swap still follows a swap, so it leaves this state unchanged.
            let previous_was_swap =
                std::mem::replace(&mut previous_was_swap, instruction.opcode == SWAP);
            if !previous_was_swap || instruction.opcode != SWAP {
                continue;
            }
            let Some(depth) = instruction.value_argument() else {
                continue;
            };
            if depth == 0 {
                redundant.push(position);
                continue;
            }

            let mut following = instructions.clone();
            let Some((first, first_instruction)) = following.next() else {
                continue;
            };
            if first_instruction.opcode != POP_TOP {
                continue;
            }
            let line = first_instruction.location.line;
            let mut last = first;
            let mut pops = 1;
            // CPython cannot fold a rotation across distinct line events. An unlocated
            // first pop permits later locations, just as in the static swap pass.
            for (position, instruction) in
                following.take(usize::try_from(depth - 1).unwrap_or(usize::MAX))
            {
                if instruction.opcode != POP_TOP || (line >= 0 && instruction.location.line != line)
                {
                    break;
                }
                last = position;
                pops += 1;
            }
            if pops == depth {
                redundant.push(position);
                pop_location_swaps.push((first, last));
            }
        }
        #[cfg(test)]
        {
            let (expected_redundant, expected_swaps) = tests::redundant_swap_discoveries(graph);
            assert_eq!(redundant, expected_redundant);
            assert_eq!(pop_location_swaps, expected_swaps);
        }

        let graph = &mut self.graph;
        let changed = !redundant.is_empty();
        // Native static swapping exchanges the pop records from right to left.
        // These identical opcodes keep their slot-derived depth and owner metadata,
        // but their source spans must follow the removed stack permutation.
        for (first, last) in pop_location_swaps.into_iter().rev() {
            let first_location = graph.instruction_mut(first).unwrap().location;
            let last_location = graph.instruction_mut(last).unwrap().location;
            graph.instruction_mut(first).unwrap().location = last_location;
            graph.instruction_mut(last).unwrap().location = first_location;
        }
        graph.remove_ordered_instructions(&redundant);
        changed
    }

    /// Replaces each run of stack swaps with the shortest equivalent sequence.
    ///
    /// This is the first half of CPython's `swaptimize` pass. Reconstructing the
    /// permutation from the end of the run guarantees that the replacement fits
    /// in the original instruction slots. Reports whether any original slot changed.
    pub(in crate::assembler) fn optimize_swap_runs(
        &mut self,
    ) -> (Vec<SwapBlock>, FxHashSet<ItemPosition>, bool) {
        const VISITED: usize = usize::MAX;

        let graph = &self.graph;
        if !graph.features().has_swap() {
            return (Vec::new(), FxHashSet::default(), false);
        }
        let mut block_labels = FxHashSet::default();
        for item in graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Some(label) = instruction.jump_target()
            {
                block_labels.insert(label);
            }
        }
        for region in &self.exception_regions {
            block_labels.extend([region.start, region.end, region.target]);
        }
        block_labels.extend(self.preserved_block_boundaries.iter().copied());
        if self.python_version <= crate::PythonVersion::Py313 {
            // Exit copying can remove the last jump into an original block. Native
            // swaptimize still cannot move stores across that block's entry.
            block_labels.extend(&self.exception_owner_provenance.block_boundaries);
        }
        // Both swap phases operate within partitions containing a swap.
        let blocks = graph.matching_partition_positions(
            &block_labels,
            |instruction| instruction.value_argument().is_none() || ends_scope(instruction.opcode),
            |instruction| instruction.opcode == SWAP,
        );
        // Admit cleanups from their complete original partition before any swap run is
        // rewritten. A newly simplified suffix cannot certify an emitted owner slot.
        let mut capture_cleanup_swaps = FxHashSet::default();
        let mut composed_capture_swaps = FxHashSet::default();
        let mut capture_cleanup_donors = FxHashMap::default();
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
                    if matches!(instruction.opcode, UNPACK_SEQUENCE | UNPACK_EX | POP_TOP | NOT_TAKEN))
            }) {
                continue;
            }
            let items = block
                .iter()
                .map(|position| *graph.item(*position))
                .collect::<Vec<_>>();
            for index in 0..items.len() {
                // No owner-slot prediction changes are needed in an unprotected array.
                if self.exception_regions.is_empty()
                    && let Some(swaps) = composed_capture_cleanup(&items[index..])
                {
                    for offset in swaps {
                        let position = block[index + offset];
                        capture_cleanup_swaps.insert(position);
                        composed_capture_swaps.insert(position);
                    }
                }
                if let Some(rotation) = capture_cleanup_rotation(&items[index..]) {
                    if let Some((offset, donor)) = rotation.projected_pop_donor {
                        // Static swaps rotate this original POP into the following donor slot.
                        capture_cleanup_donors.insert(block[index + offset], donor);
                    }
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
        let mut rewritten_blocks = Vec::with_capacity(blocks.len());
        for block in blocks {
            let mut rewritten = FxHashSet::default();
            // Only SWAP-started runs are rewritten; retain the full partition for donor tracking.
            let start = block.iter().position(|position| {
                matches!(graph.item(*position), Item::Instruction(instruction) if instruction.opcode == SWAP)
            }).expect("matching partition contains a swap");
            let instruction_positions = block[start..]
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
                let Some(first_depth) = first.value_argument() else {
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
                        let Some(argument) = instruction.value_argument() else {
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
                        let Some(argument) = instruction.value_argument() else {
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
                    changed |= instruction.opcode != NOP || instruction.value_argument() != Some(0);
                    if self.python_version >= crate::PythonVersion::Py314
                        && instruction.opcode == SWAP
                    {
                        rewritten.insert(instruction_positions[current]);
                    }
                    instruction.opcode = NOP;
                    instruction.set_operand(Operand::Value(0));
                }
                position = run_end;
            }
            // Rotation simplification only writes SWAP/NOP slots. Snapshot the
            // untouched capture donors before static swaps can reorder any POP_TOP.
            let mut original_donors = if self.python_version >= crate::PythonVersion::Py314
                && (!rewritten.is_empty() || instruction_positions.iter().any(|&position| {
                    matches!(graph.item(position), Item::Instruction(instruction) if instruction.opcode == SWAP)
                }))
            {
                block.iter().filter_map(|&position| match graph.item(position) {
                    Item::Instruction(instruction)
                        if matches!(instruction.opcode, UNPACK_SEQUENCE | POP_TOP) =>
                    {
                        Some((position, *instruction))
                    }
                    _ => None,
                }).collect()
            } else {
                FxHashMap::default()
            };
            original_donors.extend(block.iter().filter_map(|position| {
                capture_cleanup_donors
                    .get(position)
                    .map(|donor| (*position, *donor))
            }));
            rewritten_blocks.push(SwapBlock {
                composed_capture_swaps: block
                    .iter()
                    .copied()
                    .filter(|position| composed_capture_swaps.contains(position))
                    .collect(),
                positions: block,
                rewritten_nops: rewritten,
                original_donors,
            });
        }
        (rewritten_blocks, capture_cleanup_swaps, changed)
    }

    /// Applies stack swaps statically by reordering local stores and pops.
    ///
    /// This is the `apply_static_swaps` half of CPython's `swaptimize` pass and
    /// must run before adjacent local stores are fused. Reverse traversal applies
    /// consecutive swaps from right to left, matching CPython's per-run handoff.
    pub(in crate::assembler) fn apply_static_swaps(
        &mut self,
        blocks: (Vec<SwapBlock>, FxHashSet<ItemPosition>),
    ) -> Vec<StaticSwapStoreGap> {
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
        let mut store_gaps = Vec::new();
        let join_origins =
            std::mem::take(&mut self.exception_owner_provenance.normal_try_copy_join_origins);
        if blocks.is_empty() {
            return store_gaps;
        }
        let retired_labels = &self
            .exception_owner_provenance
            .retired_normal_try_copy_labels;
        let graph = &mut self.graph;
        // Resolve original labels only within this pass. A saved item offset could
        // otherwise name a different store after earlier graph rewrites.
        let join_stores = if join_origins.is_empty() {
            FxHashMap::default()
        } else {
            graph
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
                .collect()
        };
        for SwapBlock {
            positions,
            rewritten_nops: mut rewritten,
            mut original_donors,
            composed_capture_swaps,
        } in blocks
        {
            let mut index = positions.len();
            while index > 0 {
                index -= 1;
                let Item::Instruction(swap) = *graph.item(positions[index]) else {
                    continue;
                };
                if swap.opcode != SWAP {
                    continue;
                }
                let Some(depth) = swap.value_argument() else {
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
                if self.python_version >= crate::PythonVersion::Py314
                    && first_store.is_none()
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

                // 3.12 resolves lines after static swaps, so the exchanged store
                // supplies the successor's location instead of the original store.
                if self.python_version >= crate::PythonVersion::Py313
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
                }
                if self.python_version >= crate::PythonVersion::Py313
                    && first_instruction.local_policy.may_store_null()
                    && last_instruction.opcode == STORE_FAST
                    && !last_instruction.local_policy.may_store_null()
                {
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
                        if next.value_argument().is_none() || ends_scope(next.opcode) {
                            break;
                        }
                    }
                }

                let Some(swap) = graph.instruction_mut(positions[index]) else {
                    unreachable!();
                };
                swap.opcode = NOP;
                swap.set_operand(Operand::Value(0));
                if self.python_version >= crate::PythonVersion::Py314 {
                    rewritten.insert(positions[index]);
                }
                transport_capture_pop_donors(
                    &mut original_donors,
                    composed_capture_swaps.contains(&positions[index]),
                    (positions[first], first_instruction),
                    (positions[last], last_instruction),
                );
                *graph
                    .instruction_mut(positions[first])
                    .expect("swappable item is an instruction") = last_instruction;
                *graph
                    .instruction_mut(positions[last])
                    .expect("swappable item is an instruction") = first_instruction;
            }
            clear_redundant_swap_nop_locations(graph, &positions, &rewritten, &original_donors);
            capture_static_swap_store_gaps(graph, &positions, &rewritten, &mut store_gaps);
        }
        store_gaps
    }
}

/// The original restore and small exit, before their copying makes a static swap possible.
pub(in crate::assembler) struct CopiedComprehensionSwap {
    entry: Label,
    source: Vec<Item>,
    target: Vec<Item>,
    restores: Vec<Instruction>,
    tail: [Instruction; 3],
    original_labels: FxHashSet<Label>,
}

impl OptimizationPipeline {
    /// Records only an unchanged comprehension restore jumping to a three-cell local exit.
    pub(in crate::assembler) fn capture_copied_comprehension_swaps(
        &self,
    ) -> Vec<CopiedComprehensionSwap> {
        use crate::target::opcodes::{JUMP_FORWARD, LOAD_CONST, LOAD_FAST, RETURN_VALUE};

        if self
            .exception_owner_provenance
            .original_comprehension_normal_restores
            .is_empty()
        {
            return Vec::new();
        }
        let graph = &self.graph;
        let mut offsets = FxHashMap::default();
        let mut offset = 0;
        for item in graph.iter_items() {
            match item {
                Item::Label(label) => {
                    offsets.insert(*label, offset);
                }
                Item::Instruction(_) => offset += 1,
            }
        }
        let protected = |start: usize, end: usize| {
            self.exception_regions.iter().any(|region| {
                !region.is_scope_fallback
                    && offsets[&region.start] < end
                    && start < offsets[&region.end]
            })
        };
        let mut records = Vec::new();
        for (_, block) in graph.iter() {
            let source = block.items();
            let Some((entry, restores)) = source.iter().find_map(|item| match item {
                Item::Label(entry) => self
                    .exception_owner_provenance
                    .original_comprehension_normal_restores
                    .get(entry)
                    .map(|restores| (*entry, restores)),
                Item::Instruction(_) => None,
            }) else {
                continue;
            };
            let labels = source
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
            let [swap, stores @ ..] = restores.as_slice() else {
                continue;
            };
            let Some(Item::Instruction(jump)) = source.last() else {
                continue;
            };
            let Some(target) = jump.forward_target() else {
                continue;
            };
            if stores.is_empty()
                || swap.opcode != SWAP
                || swap.operand() != Operand::Value(u32::try_from(restores.len()).unwrap_or(0))
                || source.len() != labels + restores.len() + 1
                || !source[labels..source.len() - 1]
                    .iter()
                    .copied()
                    .eq(restores.iter().copied().map(Item::Instruction))
                || jump.opcode != JUMP_FORWARD
                || !jump.jump_policy.inline_small_exit()
                || jump.location != SourceLocation::NONE
                || protected(offsets[&entry], offsets[&entry] + restores.len() + 1)
            {
                continue;
            }
            let Some(target_block) = graph.label_block(target) else {
                continue;
            };
            let target_items = graph[target_block].items();
            let target_labels = target_items
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .count();
            let [
                Item::Instruction(store),
                Item::Instruction(load),
                Item::Instruction(ret),
            ] = &target_items[target_labels..]
            else {
                continue;
            };
            if store.opcode != STORE_FAST
                || store.local_policy.may_store_null()
                || !matches!(load.opcode, LOAD_FAST | LOAD_CONST)
                || ret.opcode != RETURN_VALUE
                || store.location.line < 0
                || protected(offsets[&target], offsets[&target] + 3)
                || stores.iter().any(|restore| {
                    restore.opcode != STORE_FAST
                        || !restore.local_policy.may_store_null()
                        || restore.location.line != store.location.line
                })
                || restores
                    .iter()
                    .chain([store, load, ret, jump])
                    .any(|instruction| {
                        instruction.pre_fold_extra_instructions() != 0
                            || instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.exception_policy()
                                != super::super::ExceptionPolicy::default()
                    })
            {
                continue;
            }
            let mut locals = FxHashSet::default();
            if !stores.iter().chain([store]).all(|instruction| {
                matches!(instruction.operand(), Operand::Value(local) if locals.insert(local))
            }) { continue; }
            records.push(CopiedComprehensionSwap {
                entry,
                source: source.to_vec(),
                target: target_items.to_vec(),
                restores: restores.clone(),
                tail: [*store, *load, *ret],
                original_labels: offsets.keys().copied().collect(),
            });
        }
        records
    }

    /// Replays the native static swap only after the copier produced the captured exit verbatim.
    pub(in crate::assembler) fn replay_copied_comprehension_swaps(
        &mut self,
        records: Vec<CopiedComprehensionSwap>,
    ) {
        for record in records {
            let Some(block) = self.graph.label_block(record.entry) else {
                continue;
            };
            let Some(index) = self
                .graph
                .order()
                .iter()
                .position(|candidate| *candidate == block)
            else {
                continue;
            };
            let Some(&copy_block) = self.graph.order().get(index + 1) else {
                continue;
            };
            let source = self.graph[block].items();
            // The actual copy removes only the original jump and inserts one fresh entry.
            if source != &record.source[..record.source.len() - 1] {
                continue;
            }
            let copy = self.graph[copy_block].items();
            let [Item::Label(entry), tail @ ..] = copy else {
                continue;
            };
            let mut expected_tail = record.tail;
            expected_tail[0].add_pre_fold_extra_instructions(1);
            if record.original_labels.contains(entry)
                || !tail.iter().copied().eq(expected_tail.into_iter().map(Item::Instruction))
                || self.graph.iter_items().any(|item| matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label) if label == *entry)))
                || self.exception_regions.iter().any(|region| [region.start, region.end, region.target].contains(entry))
                || !self.graph.iter().any(|(_, block)| block.items() == record.target)
            { continue; }
            let start = source.len() - record.restores.len();
            let mut first = record.restores[1];
            // The replaced jump remains at the copied entry when stores exchange.
            first.add_pre_fold_extra_instructions(1);
            let mut swap = record.restores[0];
            swap.opcode = NOP;
            swap.set_operand(Operand::Value(0));
            *self
                .graph
                .instruction_mut(ItemPosition::new(block, start))
                .expect("captured swap") = swap;
            *self
                .graph
                .instruction_mut(ItemPosition::new(block, start + 1))
                .expect("captured restore") = record.tail[0];
            *self
                .graph
                .instruction_mut(ItemPosition::new(copy_block, 1))
                .expect("copied store") = first;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::exception_owners::TupleJoinLocationOrigin;
    use crate::assembler::{Assembler, Label};
    use crate::target::opcodes::JUMP_FORWARD;

    #[test]
    fn swap_runs_preserve_leading_items_and_original_donors() {
        for prefix_length in [0, 1, 16, 257] {
            let mut pipeline = Assembler::default().into_optimization_pipeline();
            let location = SourceLocation::new(5, 5, 0, 1);
            let mut items = vec![Item::Label(pipeline.label())];
            items.extend((0..prefix_length).map(|_| {
                Item::Instruction(Instruction::new(NOP, Operand::Value(0), location, None))
            }));
            items.push(Item::Label(pipeline.label()));
            let donor = Instruction::new(POP_TOP, Operand::Value(0), location, None);
            items.push(Item::Instruction(donor));
            let start = items.len();
            for (opcode, argument) in [(SWAP, 2), (NOP, 0), (SWAP, 2), (STORE_FAST, 0)] {
                items.push(Item::Instruction(Instruction::new(
                    opcode,
                    Operand::Value(argument),
                    location,
                    None,
                )));
            }
            pipeline.graph = BlockGraph::from_items(items.clone(), &FxHashSet::default());
            let (blocks, capture_swaps, changed) = pipeline.optimize_swap_runs();
            assert!(changed);
            assert!(capture_swaps.is_empty());
            let [block] = blocks.as_slice() else {
                panic!("one logical swap partition");
            };
            assert_eq!(block.positions, pipeline.graph.item_positions());
            assert_eq!(
                block.original_donors.get(&block.positions[start - 1]),
                Some(&donor)
            );
            assert_eq!(block.rewritten_nops.len(), 2);
            let actual = pipeline.graph.copy_items();
            assert_eq!(&actual[..start], &items[..start]);
            assert_eq!(actual.last(), items.last());
            for item in &actual[start..start + 3] {
                let Item::Instruction(instruction) = item else {
                    unreachable!();
                };
                assert_eq!(instruction.opcode, NOP);
                assert_eq!(instruction.value_argument(), Some(0));
                assert_eq!(instruction.location, location);
            }
        }
    }

    fn composed_capture_items() -> Vec<Item> {
        [
            (UNPACK_SEQUENCE, 1, 5),
            (SWAP, 5, 5),
            (SWAP, 4, 5),
            (SWAP, 3, 5),
            (SWAP, 2, 5),
            (POP_TOP, 0, 4),
            (POP_TOP, 0, 3),
            (SWAP, 3, 3),
            (SWAP, 2, 3),
            (STORE_FAST, 0, 2),
            (STORE_FAST, 1, 1),
            (STORE_FAST, 2, 0),
            (NOP, 0, 0),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (opcode, argument, depth))| {
            let line = if index == 12 { 4 } else { 3 };
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                SourceLocation::new(line, line, 0, 1),
                Some(depth),
            ))
        })
        .collect()
    }

    #[test]
    fn composed_capture_cleanup_requires_complete_original_arrays() {
        assert_eq!(
            composed_capture_cleanup(&composed_capture_items()),
            Some(vec![1, 2, 3, 4, 7, 8])
        );
        for mutation in 0..9 {
            let mut items = composed_capture_items();
            match mutation {
                0 => items.insert(6, Item::Label(Label::from_u32(0))),
                1 => items.truncate(11),
                2 => items[10] = items[9],
                3 => items[2] = items[3],
                4 => items[7] = items[1],
                5 => items[12] = items[5],
                6 => items[0] = items[5],
                7 => items[5] = items[9],
                8 => items[6] = items[12],
                _ => unreachable!(),
            }
            assert!(composed_capture_cleanup(&items).is_none(), "{mutation}");
        }
        for index in 0..12 {
            for policy in 0..8 {
                let mut items = composed_capture_items();
                let Item::Instruction(instruction) = &mut items[index] else {
                    unreachable!()
                };
                match policy {
                    0 => instruction.location = SourceLocation::NONE,
                    1 => instruction.no_location_policy.preserve_location(),
                    2 => instruction.fusion_policy.prevent_with_previous(),
                    3 => instruction.jump_policy.retain_nop(),
                    4 => instruction.local_policy.add_hidden_exception_owner_slots(1),
                    5 => instruction
                        .set_exception_policy(super::super::super::ExceptionPolicy::EXCLUDE_ALWAYS),
                    6 => instruction.add_pre_fold_extra_instructions(1),
                    7 => {
                        *instruction = Instruction::new(
                            instruction.opcode,
                            instruction.operand(),
                            instruction.location,
                            None,
                        );
                    }
                    _ => unreachable!(),
                }
                assert!(
                    composed_capture_cleanup(&items).is_none(),
                    "{index}/{policy}"
                );
            }
        }
    }

    #[test]
    fn composed_capture_cleanup_requires_single_result_unpack() {
        for (opcode, argument, accepted) in [
            (UNPACK_SEQUENCE, 1, true),
            (UNPACK_EX, 0, true),
            (UNPACK_EX, 1, false),
            (UNPACK_EX, 256, false),
            (UNPACK_EX, 257, false),
            (UNPACK_SEQUENCE, 0, false),
            (POP_TOP, 0, false),
        ] {
            let mut items = composed_capture_items();
            let Item::Instruction(prefix) = &mut items[0] else {
                unreachable!()
            };
            *prefix = Instruction::new(
                opcode,
                Operand::Value(argument),
                prefix.location,
                prefix.depth_after(),
            );
            assert_eq!(
                composed_capture_cleanup(&items).is_some(),
                accepted,
                "{opcode:?}/{argument}"
            );
        }
    }

    #[test]
    fn composed_capture_cleanup_keeps_lines_within_each_pop_group() {
        let items = [
            (UNPACK_SEQUENCE, 1, 8),
            (SWAP, 8, 8),
            (SWAP, 7, 8),
            (SWAP, 6, 8),
            (SWAP, 5, 8),
            (SWAP, 4, 8),
            (SWAP, 3, 8),
            (SWAP, 2, 8),
            (POP_TOP, 0, 7),
            (POP_TOP, 0, 6),
            (POP_TOP, 0, 5),
            (POP_TOP, 0, 4),
            (SWAP, 4, 4),
            (SWAP, 3, 4),
            (SWAP, 2, 4),
            (POP_TOP, 0, 3),
            (POP_TOP, 0, 2),
            (STORE_FAST, 0, 1),
            (STORE_FAST, 1, 0),
            (NOP, 0, 0),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (opcode, argument, depth))| {
            let line = match index {
                0..15 => 5,
                15..19 => 3,
                _ => 6,
            };
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                SourceLocation::new(line, line, 0, 1),
                Some(depth),
            ))
        })
        .collect::<Vec<_>>();
        assert_eq!(
            composed_capture_cleanup(&items),
            Some(vec![1, 2, 3, 4, 5, 6, 7, 12, 13, 14])
        );
        for (index, line) in [(9, 3), (16, 5), (18, 5), (19, 3)] {
            let mut changed = items.clone();
            let Item::Instruction(instruction) = &mut changed[index] else {
                unreachable!()
            };
            instruction.location = SourceLocation::new(line, line, 0, 1);
            assert!(composed_capture_cleanup(&changed).is_none(), "{index}");
        }
    }

    pub(super) fn redundant_swap_discoveries(
        graph: &BlockGraph,
    ) -> (Vec<ItemPosition>, Vec<(ItemPosition, ItemPosition)>) {
        let positions = graph.item_positions();
        let mut instructions = Vec::with_capacity(positions.len());
        instructions.extend(positions.into_iter().filter_map(|position| {
            let Item::Instruction(instruction) = *graph.item(position) else {
                return None;
            };
            Some((position, instruction))
        }));
        let mut previous = None;
        let mut redundant = Vec::new();
        let mut pop_location_swaps = Vec::new();
        for (index, (position, instruction)) in instructions.iter().copied().enumerate() {
            let removable_pops = if instruction.opcode == SWAP
                && let Operand::Value(depth) = instruction.operand()
                && previous.is_some_and(|previous: Instruction| previous.opcode == SWAP)
            {
                instructions[index + 1..]
                    .get(..usize::try_from(depth).unwrap_or(usize::MAX))
                    .filter(|following| {
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
                None
            };
            if let Some(pops) = removable_pops {
                redundant.push(position);
                if let Some((first, last)) = pops.first().zip(pops.last()) {
                    pop_location_swaps.push((first.0, last.0));
                }
            } else {
                previous = Some(instruction);
            }
        }

        (redundant, pop_location_swaps)
    }

    #[test]
    fn streamed_redundant_swaps_match_snapshot_with_labels_lines_and_nop_breaks() {
        let instruction = |opcode, argument, line, column| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                SourceLocation::new(line, line, column, column + 1),
                None,
            ))
        };
        for first_depth in [0, 1, 2, 3, u32::MAX] {
            for second_depth in [0, 1, 2, 3, u32::MAX] {
                for (first_pop_line, last_pop_line) in [(4, 4), (4, 5), (-1, 5), (4, -1)] {
                    for separator in [None, Some(NOP), Some(POP_TOP)] {
                        let mut items = vec![
                            instruction(SWAP, 2, 4, 0),
                            Item::Label(Label::from_u32(0)),
                            instruction(SWAP, first_depth, 4, 1),
                        ];
                        if let Some(separator) = separator {
                            items.push(instruction(separator, 0, 4, 2));
                        }
                        items.extend([
                            Item::Label(Label::from_u32(1)),
                            instruction(SWAP, second_depth, 4, 3),
                            instruction(POP_TOP, 0, first_pop_line, 4),
                            Item::Label(Label::from_u32(2)),
                            instruction(POP_TOP, 0, last_pop_line, 5),
                            instruction(POP_TOP, 0, last_pop_line, 6),
                            instruction(STORE_FAST, 0, 4, 7),
                        ]);
                        let mut assembler = Assembler::from_linear_items(items);
                        assembler.next_label = 3;
                        let mut pipeline = assembler.into_optimization_pipeline();
                        let (redundant, swaps) = redundant_swap_discoveries(&pipeline.graph);
                        let mut expected = pipeline.graph.clone();
                        for (first, last) in swaps.into_iter().rev() {
                            let first_location = expected.instruction_mut(first).unwrap().location;
                            let last_location = expected.instruction_mut(last).unwrap().location;
                            expected.instruction_mut(first).unwrap().location = last_location;
                            expected.instruction_mut(last).unwrap().location = first_location;
                        }
                        expected.remove_ordered_instructions(&redundant);
                        assert_eq!(
                            pipeline.remove_redundant_swaps_before_pops(),
                            !redundant.is_empty()
                        );
                        assert_eq!(pipeline.graph.copy_items(), expected.copy_items());
                    }
                }
            }
        }
    }

    #[test]
    fn no_static_swaps_still_retire_join_origins() {
        let mut pipeline = Assembler::default().into_optimization_pipeline();
        let predecessor = pipeline.label();
        let join = pipeline.label();
        let target = pipeline.label();
        let store = Instruction::new(STORE_FAST, Operand::Value(0), SourceLocation::NONE, None);
        let jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        );
        pipeline
            .exception_owner_provenance
            .normal_try_copy_join_origins
            .insert(
                join,
                TupleJoinLocationOrigin {
                    predecessor,
                    store,
                    jump,
                },
            );

        let (blocks, capture_swaps, changed) = pipeline.optimize_swap_runs();
        assert!(blocks.is_empty());
        assert!(!changed);
        assert!(
            pipeline
                .apply_static_swaps((blocks, capture_swaps))
                .is_empty()
        );
        assert!(
            pipeline
                .exception_owner_provenance
                .normal_try_copy_join_origins
                .is_empty()
        );
        assert!(
            pipeline
                .exception_owner_provenance
                .normal_try_copy_join_donors
                .is_empty()
        );
    }

    #[test]
    fn rewritten_swap_line_rejects_a_replaced_donor() {
        let donor = Instruction::new(
            POP_TOP,
            Operand::Value(0),
            SourceLocation::new(5, 5, 0, 1),
            None,
        );
        let nop = Instruction::new(
            NOP,
            Operand::Value(0),
            SourceLocation::new(5, 5, 2, 3),
            None,
        );
        let mut graph = BlockGraph::from_items(
            vec![Item::Instruction(donor), Item::Instruction(nop)],
            &FxHashSet::default(),
        );
        let positions = graph.item_positions();
        let original_donors = FxHashMap::from_iter([(positions[0], donor)]);
        graph.instruction_mut(positions[0]).unwrap().location.column = 1;
        clear_redundant_swap_nop_locations(
            &mut graph,
            &positions,
            &FxHashSet::from_iter([positions[1]]),
            &original_donors,
        );
        assert_eq!(*graph.instruction_mut(positions[1]).unwrap(), nop);
    }

    #[test]
    fn capture_pop_donors_follow_only_certified_exact_exchanges() {
        for mutation in 0..9 {
            let first = Instruction::new(
                POP_TOP,
                Operand::Value(0),
                SourceLocation::new(5, 5, 0, 1),
                Some(3),
            );
            let mut last = Instruction::new(
                POP_TOP,
                Operand::Value(0),
                SourceLocation::new(5, 5, 2, 3),
                Some(2),
            );
            let graph = BlockGraph::from_items(
                vec![Item::Instruction(first), Item::Instruction(last)],
                &FxHashSet::default(),
            );
            let positions = graph.item_positions();
            let mut donors = FxHashMap::from_iter([(positions[0], first), (positions[1], last)]);
            let mut certified = true;
            let mut last_position = positions[1];
            match mutation {
                0 => {}
                1 => certified = false,
                2 => {
                    donors.remove(&positions[0]);
                }
                3 => {
                    donors.remove(&positions[1]);
                }
                4 => donors.get_mut(&positions[0]).unwrap().location.column += 1,
                5 => donors.get_mut(&positions[1]).unwrap().location.column += 1,
                6 => last_position = positions[0],
                7 => {
                    last.opcode = STORE_FAST;
                    donors.insert(positions[1], last);
                }
                8 => {
                    last.location.line = 6;
                    donors.insert(positions[1], last);
                }
                _ => unreachable!(),
            }
            let before = donors.clone();
            transport_capture_pop_donors(
                &mut donors,
                certified,
                (positions[0], first),
                (last_position, last),
            );
            if mutation == 0 {
                assert_eq!(donors[&positions[0]], last);
                assert_eq!(donors[&positions[1]], first);
            } else {
                assert_eq!(donors, before, "{mutation}");
            }
        }
    }

    #[test]
    fn rewritten_swap_lines_require_a_same_array_surviving_donor() {
        for (donor, donor_line, boundary, rewritten, removed) in [
            (POP_TOP, 5, false, true, true),
            (UNPACK_SEQUENCE, 5, false, true, true),
            (POP_TOP, 4, false, true, false),
            (POP_TOP, 5, true, true, false),
            (POP_TOP, 5, false, false, false),
            (NOP, 5, false, true, false),
            (SWAP, 5, false, true, false),
            (NOT_TAKEN, 5, false, true, false),
            (STORE_FAST, 5, false, true, false),
        ] {
            let instruction = |opcode, line| {
                Item::Instruction(Instruction::new(
                    opcode,
                    Operand::Value(0),
                    SourceLocation::new(line, line, 0, 1),
                    None,
                ))
            };
            let mut items = vec![instruction(donor, donor_line)];
            if boundary {
                items.push(Item::Label(Label::from_u32(0)));
            }
            items.push(instruction(NOP, 5));
            let mut graph = BlockGraph::from_items(items, &FxHashSet::default());
            let positions = graph.item_positions();
            let position = *positions.last().unwrap();
            let rewritten = if rewritten {
                FxHashSet::from_iter([position])
            } else {
                FxHashSet::default()
            };
            let original_donors = FxHashMap::from_iter([(
                positions[0],
                *graph.instruction_mut(positions[0]).unwrap(),
            )]);
            clear_redundant_swap_nop_locations(
                &mut graph,
                &positions,
                &rewritten,
                &original_donors,
            );
            let instruction = graph.instruction_mut(position).unwrap();
            assert_eq!(instruction.location.line, if removed { -1 } else { 5 });
            assert_eq!(instruction.no_location_policy.preserves_location(), removed);
            assert_eq!(instruction.opcode, NOP);
        }
    }
}
