use std::cmp::Reverse;

use rustc_hash::{FxHashMap, FxHashSet};

use super::super::block_graph::{BasicBlockId, BlockGraph, ItemPosition};
use super::super::{
    Assembler, ConstantTable, ExceptionPolicy, ExceptionRegionPriority, Instruction, Item, Label,
    LayoutAssembler, Operand, OptimizationPipeline, SourceLocation, block_has_fallthrough,
    block_jump_target,
};
use crate::target::opcodes::{
    CALL_INTRINSIC_1, COMPARE_OP, CONTAINS_OP, COPY, COPY_FREE_VARS, IS_OP, JUMP_BACKWARD,
    JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, LOAD_CLOSURE, LOAD_CONST, LOAD_FAST,
    LOAD_FAST_LOAD_FAST, LOAD_GLOBAL, MAKE_CELL, NOP, POP_TOP, PUSH_NULL, RERAISE, RESUME,
    RETURN_GENERATOR, RETURN_VALUE, STORE_FAST, STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST,
    TO_BOOL, UNARY_NOT, YIELD_VALUE,
};
use crate::target::{is_conditional_jump_for, is_unconditional_jump};
use crate::{CompileError, PythonVersion};

// Original priorities are indices into a Vec of IR items, so usize::MAX cannot
// name an original instruction. Reserve it for exact legacy copied owners:
// their selected donor handler takes precedence over destination nesting.
const LEGACY_COPY_PRIORITY_START: usize = usize::MAX;

impl Assembler {
    /// Restores erased return `POP_BLOCK` records until each legacy target's native removal phase.
    pub(super) fn restore_legacy_return_pop_blocks(
        graph: &mut BlockGraph,
        boundaries: &FxHashMap<Label, usize>,
    ) {
        if boundaries.is_empty() {
            return;
        }
        let mut insertions = Vec::new();
        let mut depth = None;
        for &block in graph.order() {
            for (offset, item) in graph[block].items().iter().enumerate() {
                match item {
                    Item::Instruction(instruction) => depth = instruction.depth_after(),
                    Item::Label(label) => {
                        if let Some(&count) = boundaries.get(label) {
                            let mut marker = Instruction::synthetic(
                                NOP,
                                Operand::Value(0),
                                SourceLocation::NONE,
                                depth,
                            );
                            marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                            marker.fusion_policy.mark_converted_pop_block();
                            insertions.push((ItemPosition::new(block, offset + 1), marker, count));
                        }
                    }
                }
            }
        }
        for (position, marker, count) in insertions.into_iter().rev() {
            for _ in 0..count {
                graph.insert_item(position, Item::Instruction(marker));
            }
        }
    }

    /// Captures 3.12's exception-stack operand before CFG copying changes protected ranges.
    pub(super) fn prepare_legacy_yield_arguments(&mut self) {
        if self.python_version != PythonVersion::Py312 {
            return;
        }
        let labels = self
            .linear_items()
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        // CPython installs one outer SETUP_CLEANUP for the entire generator or
        // coroutine before labeling YIELD_VALUE. Later owner exclusions can split,
        // overlap, or omit that fallback range without changing its stack slot.
        let scope_depth = usize::from(
            self.exception_regions
                .iter()
                .any(|region| region.is_scope_fallback),
        );
        let regions = self
            .exception_regions
            .iter()
            .filter(|region| !region.is_scope_fallback)
            .filter_map(|region| {
                let start = *labels.get(&region.start)?;
                let end = *labels.get(&region.end)?;
                let priority = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*labels.get(&start)?, *labels.get(&end)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                Some((start, end, (priority, region.target)))
            })
            .collect::<Vec<_>>();
        for (index, item) in self.linear_items_mut().iter_mut().enumerate() {
            if let Item::Instruction(instruction) = item
                && instruction.opcode == YIELD_VALUE
            {
                let owners = regions
                    .iter()
                    .filter(|(start, end, _)| *start <= index && index < *end)
                    // Copied finalizers can add overlapping ranges with distinct
                    // origins for the same handler. They represent one active SETUP.
                    .map(|(_, _, (_, target))| *target)
                    .collect::<FxHashSet<_>>();
                instruction.set_operand(Operand::Value(
                    u32::try_from(owners.len() + scope_depth)
                        .expect("exception nesting fits in an operand"),
                ));
            }
        }
    }
}

impl OptimizationPipeline {
    /// Runs the pre-3.14 CFG schedule before preparing target-specific instruction layout.
    pub(super) fn optimize_legacy(
        mut self,
        first_line_number: u32,
        local_count: usize,
        parameter_count: usize,
        mut constants: Option<&mut dyn ConstantTable>,
    ) -> Result<LayoutAssembler, CompileError> {
        let original_arrays = self.legacy_instruction_arrays();
        self.initialize_legacy_native_jump_seeds(&original_arrays);
        self.reject_unmodeled_empty_native_jump_targets();
        self.preserve_legacy_setup_nops();
        if self.python_version == PythonVersion::Py313 {
            // Capture folded paths through original manager cleanup and dead tails
            // before pruning erases their successor jumps. Native resolves the
            // shared POP_BLOCK before folding those constant-loop predecessors.
            self.preserve_folded_pop_block_locations();
        }
        self.inline_legacy_exit_blocks(&original_arrays, true);
        if self.python_version == PythonVersion::Py313 {
            self.remove_unreachable_instructions();
            let predecessors = self.legacy_predecessors_before_folding(&original_arrays);
            self.resolve_legacy_locations(predecessors, &original_arrays);
        }
        if let Some(constants) = constants.as_mut() {
            self.optimize_legacy_load_constants(&mut **constants)?;
            self.optimize_legacy_constant_tuples(&mut **constants)?;
        }
        self.optimize_legacy_adjacent_instructions();
        self.reject_unmodeled_empty_native_jump_targets();
        self.thread_legacy_jumps();
        let (swap_blocks, capture_cleanup_swaps, _) = self.optimize_swap_runs();
        let _ = self.apply_static_swaps((swap_blocks, capture_cleanup_swaps));
        self.remove_legacy_nops_and_pairs(&original_arrays);
        self.reject_unmodeled_empty_native_jump_targets();
        if self.python_version == PythonVersion::Py312 {
            self.inline_legacy_exit_blocks(&original_arrays, false);
        }
        let repeated_backedges = if self.python_version == PythonVersion::Py312 {
            self.legacy_repeated_backedges(&original_arrays)
        } else {
            Vec::new()
        };
        self.remove_unreachable_instructions();
        loop {
            let before = self.graph.copy_items();
            self.remove_legacy_nops(&original_arrays, false);
            // Native redundant jumps leave a located NOP for the next cleanup.
            self.remove_legacy_redundant_jumps();
            if self.python_version == PythonVersion::Py312 || self.graph.copy_items() == before {
                break;
            }
        }
        if let Some(constants) = constants {
            self.prune_legacy_constants(constants);
        }
        self.add_legacy_checks_for_uninitialized_loads(
            local_count,
            parameter_count,
            &original_arrays,
        );
        if self.python_version == PythonVersion::Py313 {
            for block in self.graph.order().to_vec() {
                self.graph
                    .fuse_adjacent_instructions(block, fuse_legacy_fast_pair);
            }
            self.remove_legacy_nops(&original_arrays, false);
        }
        // Copied instructions retain their donor's handler, even when the copy lies
        // inside another protected range. Cold placement rebuilds ordinary regions
        // by whole block; restore the exact copied spans after that reconstruction.
        let copied_owners = self
            .exception_regions
            .iter()
            .copied()
            .filter(|region| {
                matches!(
                    region.priority,
                    ExceptionRegionPriority::Resolved {
                        start: LEGACY_COPY_PRIORITY_START,
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        // Native 3.12 cold relocation leaves new bridges with zero predecessors;
        // 3.13 explicitly assigns one so the preceding cold block donates its span.
        let blocks_before_cold_relocation =
            self.graph.order().iter().copied().collect::<FxHashSet<_>>();
        self.reject_unmodeled_empty_native_jump_targets();
        self.push_cold_blocks_to_end(None);
        if !copied_owners.is_empty() {
            self.exception_regions.retain(|region| {
                !matches!(
                    region.priority,
                    ExceptionRegionPriority::Resolved {
                        start: LEGACY_COPY_PRIORITY_START,
                        ..
                    }
                )
            });
            self.exception_regions
                .extend(copied_owners.into_iter().filter(|region| {
                    self.graph.label_block(region.start).is_some()
                        && self.graph.label_block(region.end).is_some()
                        && self.graph.label_block(region.target).is_some()
                }));
        }
        // Moving cold handlers can leave a jump to the immediately following block.
        // 3.12 retains its NOP through line resolution; 3.13 compacts it immediately.
        if self.python_version == PythonVersion::Py312 {
            self.remove_legacy_redundant_jumps();
        } else {
            self.remove_legacy_nops_and_jumps(&original_arrays, false);
        }
        // Relocation also drops unreachable physical blocks. Recount surviving
        // edges so stale predecessors cannot trigger extra exit copies, retaining
        // zero counts on newly inserted bridges only for the native 3.12 phase.
        let mut predecessors = self.legacy_predecessors();
        if self.python_version == PythonVersion::Py312 {
            predecessors.retain(|block, _| blocks_before_cold_relocation.contains(block));
            if !repeated_backedges.is_empty() {
                let surviving_blocks = self.graph.order().iter().copied().collect::<FxHashSet<_>>();
                for (block, original) in repeated_backedges {
                    let mut instructions = self.graph[block].items().iter().filter_map(|item| {
                        if let Item::Instruction(instruction) = item {
                            Some(instruction)
                        } else {
                            None
                        }
                    });
                    if surviving_blocks.contains(&block)
                        && let Some(instruction) = instructions.next()
                        && instructions.next().is_none()
                        && *instruction == original
                        && predecessors.get(&block) == Some(&1)
                    {
                        predecessors.insert(block, 2);
                    }
                }
            }
        }
        self.resolve_legacy_locations(predecessors, &original_arrays);
        if self.python_version == PythonVersion::Py312 {
            self.guarantee_legacy_exit_line_numbers(first_line_number);
        }
        // LOAD_CLOSURE remains a distinct pseudo-op until after the owned-local
        // fusion pass in 3.13; 3.12 still has a physical LOAD_CLOSURE instruction.
        if self.python_version == PythonVersion::Py313 {
            self.graph.for_each_instruction_mut(|instruction| {
                if instruction.opcode == LOAD_CLOSURE {
                    instruction.opcode = LOAD_FAST;
                }
            });
            self.remove_legacy_nops_and_jumps(&original_arrays, true);
            self.reject_unmodeled_empty_native_jump_targets();
        } else {
            // Native 3.12 does not eliminate empty blocks after ConvertPseudoOps:
            // their original jump operands survive into ResolveJumps. Reject
            // preexisting unsupported empty targets before this final conversion.
            self.reject_unmodeled_empty_native_jump_targets();
            self.remove_legacy_nops(&original_arrays, true);
        }
        self.normalize_legacy_jump_directions();
        self.normalize_backward_conditionals()?;
        if !self
            .legacy_native_labels
            .as_ref()
            .is_some_and(|native| native.is_certified())
        {
            self.graph
                .for_each_instruction_mut(Instruction::clear_legacy_initial_jump_argument);
            self.exception_owner_provenance
                .visit_legacy_seed_captures_mut(Instruction::clear_legacy_initial_jump_argument);
        }
        Ok(LayoutAssembler {
            python_version: self.python_version,
            legacy_uniform_jump_seed: self.legacy_uniform_jump_seed,
            items: self.graph.into_items(),
            exception_regions: self.exception_regions,
            exception_owner_provenance: self.exception_owner_provenance,
            preserved_block_boundaries: self.preserved_block_boundaries,
            removed_max_depth: None,
            next_label: self.next_label,
            has_next_location: self.has_next_location,
        })
    }

    /// Freezes native jump operands before any pointer-only CFG retargeting.
    fn initialize_legacy_native_jump_seeds(
        &mut self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
    ) {
        let Some(native) = self.legacy_native_labels.as_mut() else {
            return;
        };
        native.initialize_blocks(&self.graph, original_arrays);
        if !native.is_certified() {
            return;
        }
        let mut seeds = Vec::new();
        for block in self.graph.order().iter().copied() {
            for (offset, item) in self.graph[block].items().iter().enumerate() {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                let (Operand::Forward(target) | Operand::Backward(target)) = instruction.operand()
                else {
                    continue;
                };
                let Some(seed) = native.target_label(&self.graph, target) else {
                    native.invalidate();
                    return;
                };
                seeds.push((ItemPosition::new(block, offset), seed));
            }
        }
        // Mixed prefixes contain real non-jumps, but typed jump witnesses must
        // retain their producer's operand shape before generic seed preflight.
        if !self
            .exception_owner_provenance
            .legacy_seed_capture_shapes_valid()
        {
            native.invalidate();
            return;
        }
        let mut unresolved_capture = false;
        self.exception_owner_provenance
            .visit_legacy_seed_captures_mut(|instruction| {
                let (Operand::Forward(target) | Operand::Backward(target)) = instruction.operand()
                else {
                    return;
                };
                if native.target_label(&self.graph, target).is_none() {
                    unresolved_capture = true;
                }
            });
        if unresolved_capture {
            native.invalidate();
            return;
        }
        // Every target is resolved before the first write. Graph targets, native
        // labels and captured operands stay unchanged between preflight and commit.
        // Resolve each record directly so hash-map traversal order is immaterial.
        self.exception_owner_provenance
            .visit_legacy_seed_captures_mut(|instruction| {
                let (Operand::Forward(target) | Operand::Backward(target)) = instruction.operand()
                else {
                    return;
                };
                let seed = native
                    .target_label(&self.graph, target)
                    .expect("preflight resolved the unchanged captured jump target");
                instruction.set_legacy_initial_jump_argument(seed);
            });
        for (position, seed) in seeds {
            self.graph
                .instruction_mut(position)
                .unwrap()
                .set_legacy_initial_jump_argument(seed);
        }
    }

    /// Rejects unsupported native empty-block operand rewrites for the whole scope.
    fn reject_unmodeled_empty_native_jump_targets(&mut self) {
        let Some(native) = self.legacy_native_labels.as_mut() else {
            return;
        };
        if native.is_certified()
            && self.graph.iter_items().any(|item| {
                let Item::Instruction(instruction) = item else {
                    return false;
                };
                let (Operand::Forward(target) | Operand::Backward(target)) = instruction.operand()
                else {
                    return false;
                };
                self.graph
                    .label_block(target)
                    .is_none_or(|target| !native.target_is_nonempty(&self.graph, target))
            })
        {
            native.invalidate();
        }
    }

    /// Keeps certified SETUP records present until native pseudo-op conversion.
    fn preserve_legacy_setup_nops(&mut self) {
        let mut labels = FxHashMap::default();
        let mut instructions = Vec::new();
        for block in self.graph.order().iter().copied() {
            for (offset, item) in self.graph[block].items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        labels.insert(*label, instructions.len());
                    }
                    Item::Instruction(_) => instructions.push(ItemPosition::new(block, offset)),
                }
            }
        }
        for (start, end) in &self.exception_owner_provenance.setup_nop_ranges {
            let (Some(&start), Some(&end)) = (labels.get(start), labels.get(end)) else {
                continue;
            };
            if end == start + 1
                && let Some(&position) = instructions.get(start)
            {
                let instruction = self.graph.instruction_mut(position).unwrap();
                if instruction.opcode == NOP {
                    instruction.jump_policy.preserve_explicit_nop();
                }
            }
        }
    }

    /// Captures original CPython instruction arrays before folding removes their jumps.
    ///
    /// Source and preserved CFG entries, jump targets and exception handlers start arrays.
    /// Exception-range endpoints alone do not; they only split Serc's physical blocks.
    pub(super) fn legacy_instruction_arrays(&self) -> FxHashMap<BasicBlockId, usize> {
        let mut boundaries = self.preserved_block_boundaries.clone();
        boundaries.extend(&self.exception_owner_provenance.block_boundaries);
        boundaries.extend(&self.exception_owner_provenance.source_boundaries);
        if self.python_version == PythonVersion::Py312 {
            // 3.12 preserves every codegen label, including SETUP body entries.
            // 3.13 starts arrays only at explicit jump/SETUP targets instead.
            boundaries.extend(
                self.exception_owner_provenance
                    .setups
                    .iter()
                    .map(|(entry, _)| *entry),
            );
        }
        boundaries.extend(self.exception_regions.iter().map(|region| region.target));
        boundaries.extend(self.graph.iter_items().filter_map(|item| match item {
            Item::Instruction(instruction) => match instruction.operand() {
                Operand::Forward(target) | Operand::Backward(target) => Some(target),
                Operand::Value(_) | Operand::Deferred(_) => None,
            },
            Item::Label(_) => None,
        }));

        let mut arrays = FxHashMap::default();
        let mut array = 0;
        for block in self.graph.order().iter().copied() {
            if self.graph[block]
                .items()
                .iter()
                .any(|item| matches!(item, Item::Label(label) if boundaries.contains(label)))
            {
                array += 1;
            }
            arrays.insert(block, array);
            if self.graph[block]
                .last_instruction()
                .is_some_and(|instruction| {
                    matches!(
                        instruction.operand(),
                        Operand::Forward(_) | Operand::Backward(_)
                    ) || crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                })
            {
                array += 1;
            }
        }
        arrays
    }

    /// Copies native four-slot exit blocks, plus 3.13's unlocated terminal blocks.
    fn inline_legacy_exit_blocks(
        &mut self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
        before_folding: bool,
    ) {
        if before_folding {
            self.freeze_legacy_omitted_copy_slots();
        }
        loop {
            let mut changed = false;
            let order = self.graph.order().to_vec();
            for block in order.iter().copied() {
                let Some(jump) = self.graph[block].last_instruction().copied() else {
                    continue;
                };
                if !is_unconditional_jump(jump.opcode) {
                    continue;
                }
                let Some(target) = block_jump_target(self.graph[block].items())
                    .and_then(|label| self.graph.label_block(label))
                else {
                    continue;
                };
                if target == block {
                    continue;
                }
                // Range endpoints split physical blocks without starting another
                // native instruction array. Include the cleanup/return fragments
                // after such endpoints, but stop at original source/CFG entries.
                let target_index = order
                    .iter()
                    .position(|candidate| *candidate == target)
                    .unwrap();
                let target_array = original_arrays[&target];
                let targets = order[target_index..]
                    .iter()
                    .copied()
                    .take_while(|target| original_arrays.get(target) == Some(&target_array))
                    .collect::<Vec<_>>();
                if targets.contains(&block) {
                    // Metadata splits do not make a native self-loop eligible for copying.
                    continue;
                }
                let instructions = targets
                    .iter()
                    .flat_map(|target| self.graph[*target].items())
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(*instruction),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>();
                let Some(last) = instructions.last() else {
                    continue;
                };
                // Direct global calls already combine LOAD_GLOBAL and PUSH_NULL
                // here. Both native versions count them separately before folding;
                // only 3.12 revisits exits after their fusion and NOP compaction.
                let instruction_slots = instructions
                    .iter()
                    .map(|instruction| {
                        1 + usize::from(
                            before_folding
                                && instruction.opcode == LOAD_GLOBAL
                                && matches!(instruction.operand(), Operand::Value(arg) if arg & 1 != 0),
                        )
                    })
                    .sum::<usize>();
                let instruction_slots = instruction_slots
                    + if before_folding {
                        targets
                            .iter()
                            .map(|target| {
                                self.legacy_omitted_copy_slots(*target)
                                    .iter()
                                    .sum::<usize>()
                            })
                            .sum::<usize>()
                    } else {
                        0
                    };
                let small_exit = crate::target::is_scope_exit_for(self.python_version, last.opcode)
                    && instruction_slots <= 4;
                let unlocated_terminal = self.python_version == PythonVersion::Py313
                    && instructions
                        .iter()
                        .all(|instruction| instruction.location.line < 0)
                    && !block_has_fallthrough(self.graph[*targets.last().unwrap()].items());
                if !small_exit && !unlocated_terminal {
                    continue;
                }
                legacy_nop(self.graph.instructions_mut(block).next_back().unwrap());
                for target in targets {
                    for mut item in self.copy_legacy_block_items(target, block) {
                        if let Item::Instruction(instruction) = &mut item
                            && unlocated_terminal
                            && is_unconditional_jump(instruction.opcode)
                            && legacy_jump_has_eval_check(&jump)
                        {
                            // Native JUMP preserves its eval check when its unlocated
                            // successor is copied, including a forward successor.
                            instruction.jump_policy.mark_interruptible_after_threading();
                            if instruction.opcode == JUMP_BACKWARD_NO_INTERRUPT {
                                instruction.opcode = JUMP_BACKWARD;
                            }
                        }
                        let offset = self.graph[block].items().len();
                        self.graph
                            .insert_item(ItemPosition::new(block, offset), item);
                    }
                }
                changed = true;
            }
            if self.python_version == PythonVersion::Py312 || !changed {
                break;
            }
        }
    }

    /// Copies the exception target already assigned to each native instruction.
    ///
    /// CPython labels handlers before copying exit blocks. A copied `POP_EXCEPT`
    /// or return must therefore keep its donor's owner, rather than inherit the
    /// protected region surrounding the jump that it replaces.
    fn copy_legacy_block_items(
        &mut self,
        target: BasicBlockId,
        destination: BasicBlockId,
    ) -> Vec<Item> {
        if self.exception_regions.is_empty() {
            let copied = self.graph[target]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(Item::Instruction(*instruction)),
                    Item::Label(_) => None,
                })
                .collect();
            return self.copy_legacy_omitted_slots(target, copied);
        }
        let positions = self
            .graph
            .iter_items()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let regions = self
            .exception_regions
            .iter()
            .enumerate()
            .filter_map(|(index, region)| {
                let start = *positions.get(&region.start)?;
                let end = *positions.get(&region.end)?;
                let (priority_start, priority_end) = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*positions.get(&start)?, *positions.get(&end)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                Some((
                    start,
                    end,
                    (
                        region.is_scope_fallback,
                        Reverse(priority_start),
                        priority_end,
                        index,
                    ),
                    *region,
                ))
            })
            .collect::<Vec<_>>();
        // Appending inserts before the following block's labels: ranges ending
        // at that boundary still cover the copy, while ranges starting there do not.
        let mut destination_end = 0;
        for block in self.graph.order() {
            destination_end += self.graph[*block].items().len();
            if *block == destination {
                break;
            }
        }
        let destination_owner = regions
            .iter()
            .filter(|(begin, end, _, _)| *begin < destination_end && destination_end <= *end)
            .min_by_key(|(_, _, priority, _)| *priority)
            .map(|(_, _, _, region)| (region.target, region.depth, region.preserve_lasti));
        let mut start = 0;
        for block in self.graph.order() {
            if *block == target {
                break;
            }
            start += self.graph[*block].items().len();
        }
        let instructions = self.graph[target]
            .items()
            .iter()
            .enumerate()
            .filter_map(|(offset, item)| {
                let Item::Instruction(instruction) = item else {
                    return None;
                };
                let owner = if instruction.exception_policy().exclusion()
                    == super::super::ExceptionExclusion::Always
                {
                    None
                } else {
                    regions
                        .iter()
                        .filter(|(begin, end, _, _)| {
                            *begin <= start + offset && start + offset < *end
                        })
                        .min_by_key(|(_, _, priority, _)| *priority)
                        .map(|(_, _, _, region)| *region)
                };
                Some((*instruction, owner))
            })
            .collect::<Vec<_>>();
        let mut copied = Vec::with_capacity(instructions.len());
        let mut index = 0;
        while index < instructions.len() {
            let owner = instructions[index].1;
            let key = owner.map(|region| (region.target, region.depth, region.preserve_lasti));
            let end = (index + 1..instructions.len())
                .find(|next| {
                    instructions[*next]
                        .1
                        .map(|region| (region.target, region.depth, region.preserve_lasti))
                        != key
                })
                .unwrap_or(instructions.len());
            // Avoid adding a boundary when the existing destination region
            // already supplies the exact donor owner.
            let copied_owner = owner.filter(|_| key != destination_owner);
            let copy_start = copied_owner.map(|_| self.label());
            if let Some(start) = copy_start {
                copied.push(Item::Label(start));
            }
            for (mut instruction, _) in instructions[index..end].iter().copied() {
                if owner.is_none() {
                    instruction.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
                }
                copied.push(Item::Instruction(instruction));
            }
            if let Some(mut region) = copied_owner {
                region.start = copy_start.expect("owned copy has a start label");
                region.end = self.label();
                // This is an exact donor instruction owner, including an outer
                // scope handler copied into an otherwise more deeply nested try.
                region.is_scope_fallback = false;
                region.priority = ExceptionRegionPriority::Resolved {
                    start: LEGACY_COPY_PRIORITY_START,
                    end: 0,
                };
                copied.push(Item::Label(region.end));
                self.exception_regions.push(region);
            }
            index = end;
        }
        self.copy_legacy_omitted_slots(target, copied)
    }

    /// Freezes omitted records before copied jumps become same-span `NOP`s.
    fn freeze_legacy_omitted_copy_slots(&mut self) {
        let mut certified = FxHashMap::default();
        for &block in self.graph.order() {
            let mut slots = self.initial_legacy_omitted_copy_slots(block).into_iter();
            let mut entry = None;
            for item in self.graph[block].items() {
                match item {
                    Item::Label(label) => {
                        entry.get_or_insert(*label);
                    }
                    Item::Instruction(_) => {
                        let count = slots.next().expect("one slot count per instruction");
                        if count != 0 {
                            certified.insert(
                                entry.expect("omitted records have an original label"),
                                count,
                            );
                        }
                        entry = None;
                    }
                }
            }
        }
        self.exception_owner_provenance.legacy_early_copy_slots = certified;
    }

    /// Reads frozen costs without mistaking a rewritten jump for an authored Pass.
    fn legacy_omitted_copy_slots(&self, block: BasicBlockId) -> Vec<usize> {
        let mut slots = Vec::new();
        let mut pending = 0;
        for item in self.graph[block].items() {
            match item {
                Item::Label(label) => {
                    pending += self
                        .exception_owner_provenance
                        .legacy_early_copy_slots
                        .get(label)
                        .copied()
                        .unwrap_or(0);
                }
                Item::Instruction(_) => {
                    slots.push(pending);
                    pending = 0;
                }
            }
        }
        slots
    }

    /// Counts initial omitted native records, coalescing aliases and materialized `NOP`s.
    fn initial_legacy_omitted_copy_slots(&self, block: BasicBlockId) -> Vec<usize> {
        let mut slots = Vec::new();
        let mut omitted_nops = 0;
        let mut omitted_location = None;
        let mut certified = 0;
        for item in self.graph[block].items() {
            match item {
                Item::Label(label) => {
                    if let Some(&count) = self
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .get(label)
                        && count >= omitted_nops
                    {
                        omitted_nops = count;
                        omitted_location = self
                            .exception_owner_provenance
                            .omitted_nop_locations
                            .get(label)
                            .copied();
                    }
                    certified += self
                        .exception_owner_provenance
                        .legacy_early_copy_slots
                        .get(label)
                        .copied()
                        .unwrap_or(0);
                }
                Item::Instruction(instruction) => {
                    // Some try/else producers materialize an omitted body NOP
                    // later. Its original location certifies the slot already present.
                    let represented = usize::from(
                        instruction.opcode == NOP
                            && !instruction.fusion_policy.is_converted_pop_block()
                            && Some(instruction.location) == omitted_location,
                    );
                    slots.push(omitted_nops.saturating_sub(represented) + certified);
                    omitted_nops = 0;
                    omitted_location = None;
                    certified = 0;
                }
            }
        }
        slots
    }

    /// Carries exact omitted-slot costs with an early exit copy for transitive copying.
    fn copy_legacy_omitted_slots(&mut self, target: BasicBlockId, copied: Vec<Item>) -> Vec<Item> {
        let slots = self.legacy_omitted_copy_slots(target);
        if slots.iter().all(|count| *count == 0) {
            return copied;
        }
        let mut slots = slots.into_iter();
        let mut result = Vec::with_capacity(copied.len());
        for item in copied {
            if matches!(item, Item::Instruction(_)) {
                let count = slots.next().expect("copy preserves source instructions");
                if count != 0 {
                    let entry = self.label();
                    self.exception_owner_provenance
                        .legacy_early_copy_slots
                        .insert(entry, count);
                    result.push(Item::Label(entry));
                }
            }
            result.push(item);
        }
        result
    }

    fn optimize_legacy_adjacent_instructions(&mut self) {
        let version = self.python_version;
        self.graph
            .for_each_adjacent_instruction_pair_mut(|first, second| {
                if version == PythonVersion::Py312 {
                    if first.opcode == PUSH_NULL
                        && second.opcode == LOAD_GLOBAL
                        && let Operand::Value(argument) = second.operand()
                        && argument & 1 == 0
                    {
                        legacy_nop(first);
                        second.set_operand(Operand::Value(argument | 1));
                    }
                    return;
                }
                if first.opcode == LOAD_GLOBAL
                    && second.opcode == PUSH_NULL
                    && let Operand::Value(argument) = first.operand()
                    && argument & 1 == 0
                {
                    first.set_operand(Operand::Value(argument | 1));
                    legacy_nop(second);
                } else if second.opcode == TO_BOOL {
                    match first.opcode {
                        COMPARE_OP | CONTAINS_OP | IS_OP | UNARY_NOT => {
                            let opcode = first.opcode;
                            let operand = if opcode == COMPARE_OP {
                                let Operand::Value(argument) = first.operand() else {
                                    unreachable!("comparison has a value operand")
                                };
                                Operand::Value(argument | 16)
                            } else {
                                first.operand()
                            };
                            legacy_nop(first);
                            second.opcode = opcode;
                            second.set_operand(operand);
                        }
                        TO_BOOL => legacy_nop(first),
                        _ => {}
                    }
                } else if first.opcode == UNARY_NOT && second.opcode == UNARY_NOT {
                    legacy_nop(first);
                    legacy_nop(second);
                }
            });
        if version == PythonVersion::Py313 {
            self.optimize_redundant_store_fast();
        }
    }

    fn thread_legacy_jumps(&mut self) {
        for block in self.graph.order().to_vec() {
            for _ in 0..self.graph.len() {
                let Some(jump) = self.graph[block].last_instruction().copied() else {
                    break;
                };
                let unconditional = is_unconditional_jump(jump.opcode);
                if (!unconditional && !is_conditional_jump_for(self.python_version, jump.opcode))
                    || (self.python_version == PythonVersion::Py312
                        && jump.opcode == JUMP_BACKWARD_NO_INTERRUPT)
                {
                    break;
                }
                let Some(target) = block_jump_target(self.graph[block].items())
                    .and_then(|label| self.graph.label_block(label))
                else {
                    break;
                };
                let Some(bridge) = self.graph[target]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(*instruction),
                        Item::Label(_) => None,
                    })
                else {
                    break;
                };
                // Authored break/continue blocks begin with a temporary native NOP.
                // Its provenance must still block threading after emission elides it.
                // Inlineable unlocated joins use the same flag for a later modern
                // copy phase; they contain no native NOP and remain threadable here.
                if !is_unconditional_jump(bridge.opcode)
                    || (!bridge.jump_policy.allows_threading_target()
                        && !bridge.no_location_policy.allows_block_inlining())
                    // 3.13 does not thread conditional jumps through JUMP_NO_INTERRUPT.
                    // A forward instance shares its wire opcode with ordinary JUMP.
                    || (self.python_version == PythonVersion::Py313
                        && !unconditional
                        && !legacy_jump_has_eval_check(&bridge))
                    || (bridge.opcode == JUMP_BACKWARD_NO_INTERRUPT
                        && self.python_version == PythonVersion::Py312)
                    || bridge.operand() == jump.operand()
                    || (self.python_version == PythonVersion::Py312
                        && bridge.location.line >= 0
                        && bridge.location.line != jump.location.line)
                {
                    break;
                }
                // Threading through JUMP_NO_INTERRUPT must retain an existing eval check.
                let opcode = if unconditional && bridge.opcode != JUMP_BACKWARD_NO_INTERRUPT {
                    bridge.opcode
                } else {
                    jump.opcode
                };
                let instruction = self.graph.instructions_mut(block).next_back().unwrap();
                if self.python_version == PythonVersion::Py312 {
                    instruction.opcode = opcode;
                    instruction.retarget(bridge.operand());
                } else {
                    // 3.13 retains the source line as a NOP and appends a jump carrying
                    // the bridge's location, leaving later NOP cleanup to merge lines.
                    legacy_nop(instruction);
                    let mut threaded = Instruction::synthetic(
                        opcode,
                        bridge.operand(),
                        bridge.location,
                        jump.depth_after(),
                    );
                    if unconditional
                        && (legacy_jump_has_eval_check(&jump)
                            || legacy_jump_has_eval_check(&bridge))
                    {
                        threaded.jump_policy.mark_interruptible();
                    }
                    threaded.set_exception_policy(super::super::ExceptionPolicy::AlwaysAbsent);
                    let offset = self.graph[block].items().len();
                    self.graph.insert_item(
                        ItemPosition::new(block, offset),
                        Item::Instruction(threaded),
                    );
                }
            }
        }
    }

    fn normalize_legacy_jump_directions(&mut self) {
        let labels = self
            .graph
            .iter_items()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let mut index = 0;
        for block in self.graph.order().to_vec() {
            for item in self.graph.iter_items_mut(block) {
                if let super::super::block_graph::ItemMut::Instruction(instruction) = item
                    && let Operand::Forward(target) | Operand::Backward(target) =
                        instruction.operand()
                {
                    let backward = labels[&target] <= index;
                    instruction.retarget(if backward {
                        Operand::Backward(target)
                    } else {
                        Operand::Forward(target)
                    });
                    if is_unconditional_jump(instruction.opcode) {
                        instruction.opcode = if backward {
                            let interruptible = if self.python_version == PythonVersion::Py313 {
                                legacy_jump_has_eval_check(instruction)
                            } else {
                                instruction.opcode != JUMP_BACKWARD_NO_INTERRUPT
                            };
                            if interruptible {
                                JUMP_BACKWARD
                            } else {
                                JUMP_BACKWARD_NO_INTERRUPT
                            }
                        } else {
                            JUMP_FORWARD
                        };
                    }
                }
                index += 1;
            }
        }
    }

    /// Replaces jumps to the next physical instruction with line-preserving NOPs.
    fn remove_legacy_redundant_jumps(&mut self) -> bool {
        let mut candidate = None;
        let mut redundant = Vec::new();
        for block in self.graph.order().iter().copied() {
            for (offset, item) in self.graph[block].items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        if let Some((position, target)) = candidate
                            && target == *label
                        {
                            redundant.push(position);
                            candidate = None;
                        }
                    }
                    Item::Instruction(instruction) => {
                        candidate = None;
                        if is_unconditional_jump(instruction.opcode)
                            && let Operand::Forward(target) | Operand::Backward(target) =
                                instruction.operand()
                        {
                            candidate = Some((ItemPosition::new(block, offset), target));
                        }
                    }
                }
            }
        }
        let changed = !redundant.is_empty();
        for position in redundant {
            legacy_nop(self.graph.instruction_mut(position).unwrap());
        }
        changed
    }

    fn remove_legacy_nops_and_jumps(
        &mut self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
        convert_pseudo_ops: bool,
    ) {
        loop {
            let removed_nops = self.remove_legacy_nops(original_arrays, convert_pseudo_ops);
            let removed_jumps = self.remove_legacy_redundant_jumps();
            if !removed_nops && !removed_jumps {
                break;
            }
        }
    }

    fn remove_legacy_nops_and_pairs(&mut self, original_arrays: &FxHashMap<BasicBlockId, usize>) {
        loop {
            self.remove_legacy_nops(original_arrays, false);
            let mut changed = false;
            let mut previous = None;
            for block in self.graph.order().to_vec() {
                for offset in 0..self.graph[block].items().len() {
                    let Item::Instruction(instruction) = self.graph[block].items()[offset] else {
                        previous = None;
                        continue;
                    };
                    let position = ItemPosition::new(block, offset);
                    if instruction.opcode == POP_TOP
                        && let Some(previous_position) = previous
                    {
                        let predecessor = *self.graph.instruction_mut(previous_position).unwrap();
                        if predecessor.opcode == LOAD_CONST
                            || (predecessor.opcode == COPY
                                && predecessor.operand() == Operand::Value(1))
                        {
                            // Preserve both spans until the next NOP cleanup decides
                            // which records are required for line tracing.
                            legacy_nop(self.graph.instruction_mut(previous_position).unwrap());
                            legacy_nop(self.graph.instruction_mut(position).unwrap());
                            changed = true;
                        }
                    }
                    previous = Some(position);
                }
                if !block_has_fallthrough(self.graph[block].items())
                    || self.graph[block]
                        .last_instruction()
                        .is_some_and(|instruction| {
                            instruction.opcode.has_jump_for(self.python_version)
                        })
                {
                    previous = None;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn remove_legacy_nops(
        &mut self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
        convert_pseudo_ops: bool,
    ) -> bool {
        let preserve_pop_blocks =
            self.python_version == PythonVersion::Py312 && !convert_pseudo_ops;
        let mut changed = false;
        let mut previous_array = None;
        let mut previous_line = -1;
        let order = self.graph.order().to_vec();
        for (block_index, block) in order.iter().copied().enumerate() {
            let array = original_arrays.get(&block).copied();
            // Copies inserted after the snapshot have their own native arrays.
            if array.is_none() || array != previous_array {
                previous_line = -1;
            }
            previous_array = array;
            let positions = self.graph[block]
                .items()
                .iter()
                .enumerate()
                .filter_map(|(offset, item)| match item {
                    Item::Instruction(_) => Some(ItemPosition::new(block, offset)),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let mut removed = Vec::new();
            for (index, position) in positions.iter().copied().enumerate() {
                let instruction = *self.graph.instruction_mut(position).unwrap();
                // SETUP records, and 3.12 POP_BLOCK, stay real records until
                // pseudo-op conversion. Their locations still precede later NOPs.
                if instruction.opcode == NOP
                    && (convert_pseudo_ops || !instruction.jump_policy.preserves_explicit_nop())
                    && !(preserve_pop_blocks && instruction.fusion_policy.is_converted_pop_block())
                {
                    let mut remove =
                        instruction.location.line < 0 || instruction.location.line == previous_line;
                    if !remove {
                        // Metadata labels split physical blocks without ending the native
                        // instruction array. A NOP can still donate its location to the
                        // next original instruction across those fragments.
                        let next = positions.get(index + 1).copied().or_else(|| {
                            array?;
                            order[block_index + 1..]
                                .iter()
                                .take_while(|next| original_arrays.get(next).copied() == array)
                                .find_map(|next| {
                                    self.graph[*next]
                                        .items()
                                        .iter()
                                        .position(|item| matches!(item, Item::Instruction(_)))
                                        .map(|offset| ItemPosition::new(*next, offset))
                                })
                        });
                        if let Some(next) = next {
                            let next = self.graph.instruction_mut(next).unwrap();
                            if next.location.line < 0 {
                                next.location = instruction.location;
                                remove = true;
                            } else {
                                remove = next.location.line == instruction.location.line;
                            }
                        } else {
                            let next = order[block_index + 1..].iter().find_map(|block| {
                                self.graph[*block]
                                    .items()
                                    .iter()
                                    .find_map(|item| match item {
                                        Item::Instruction(next)
                                            if next.opcode != NOP
                                                || next.location.line >= 0
                                                || (!convert_pseudo_ops
                                                    && next
                                                        .jump_policy
                                                        .preserves_explicit_nop())
                                                || (preserve_pop_blocks
                                                    && next
                                                        .fusion_policy
                                                        .is_converted_pop_block()) =>
                                        {
                                            Some(next.location.line)
                                        }
                                        _ => None,
                                    })
                            });
                            remove = next == Some(instruction.location.line);
                        }
                    }
                    if remove {
                        removed.push(position);
                        continue;
                    }
                }
                previous_line = instruction.location.line;
            }
            changed |= !removed.is_empty();
            self.graph.remove_ordered_instructions(&removed);
            if self.graph[block]
                .last_instruction()
                .is_some_and(|instruction| {
                    matches!(
                        instruction.operand(),
                        Operand::Forward(_) | Operand::Backward(_)
                    ) || crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                })
            {
                previous_array = None;
            }
        }
        changed
    }

    /// Restores original constant-loop false edges for 3.13's first line pass.
    fn legacy_predecessors_before_folding(
        &self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
    ) -> FxHashMap<BasicBlockId, usize> {
        let mut predecessors = self.legacy_predecessors();
        if self.folded_predecessor_sources.is_empty() {
            return predecessors;
        }
        let order = self.graph.order();
        let positions = order
            .iter()
            .enumerate()
            .map(|(position, block)| (*block, position))
            .collect::<FxHashMap<_, _>>();
        let mut restored = FxHashSet::default();
        for &(source, target, location) in &self.folded_predecessor_sources {
            if !self.folded_loop_exit_anchors.contains(&target) {
                continue;
            }
            let (Some(source), Some(mut target)) = (
                self.graph.label_block(source),
                self.graph.label_block(target),
            ) else {
                continue;
            };
            let Some(marker) = self.graph[source]
                .items()
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
            else {
                continue;
            };
            if marker.opcode != NOP || marker.location != location {
                continue;
            }
            // Range-only labels can fragment one original target array. Cross
            // only those aliases: pruning dead source must not create a path
            // through an unrelated original block or exception handler.
            while self.graph[target].last_instruction().is_none() {
                let Some(next) = order.get(positions[&target] + 1) else {
                    break;
                };
                if original_arrays.get(&target).is_none()
                    || original_arrays.get(&target) != original_arrays.get(next)
                {
                    break;
                }
                target = *next;
            }
            if self.graph[target]
                .items()
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .is_some_and(|first| {
                    (first.opcode == NOP && first.fusion_policy.is_converted_pop_block())
                    // Ordinary try lowering keeps POP_BLOCK as a hidden copy
                    // slot. Its surviving unlocated exit jump has the same
                    // original constant-loop predecessor before line resolution.
                    || (is_unconditional_jump(first.opcode)
                        && first.location == SourceLocation::NONE)
                })
                && restored.insert((source, target))
            {
                *predecessors.entry(target).or_default() += 1;
            }
        }
        predecessors
    }

    /// Recovers repeated visits to an unlocated 3.12 backedge during CFG marking.
    fn legacy_repeated_backedges(
        &self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
    ) -> Vec<(BasicBlockId, Instruction)> {
        let full_order = self.graph.order();
        // Avoid building provenance maps when no conditional can queue its
        // fallthrough twice, the only shape this analysis can repair.
        if !full_order.windows(2).any(|pair| {
            block_has_fallthrough(self.graph[pair[0]].items())
                && block_jump_target(self.graph[pair[0]].items())
                    .and_then(|label| self.graph.label_block(label))
                    == Some(pair[1])
        }) {
            return Vec::new();
        }
        if !self.exception_regions.is_empty()
            && let Some(backedges) = self.legacy_repeated_backedges_with_setups(original_arrays)
        {
            return backedges;
        }
        let mut prefix_len = full_order.len();
        if !self.exception_regions.is_empty() || !self.exception_owner_provenance.setups.is_empty()
        {
            // Native 3.12 also traverses SETUP targets. Only simulate the prefix
            // before every original handler target when each setup is mapped
            // to its authored target and no later jump can reenter the prefix.
            // Stop at an empty or split native instruction array because the
            // physical Serc blocks no longer establish the native traversal.
            let setups = &self.exception_owner_provenance.setups;
            let captured = &self.exception_owner_provenance.legacy_native_setup_targets;
            let targets = captured.iter().copied().collect::<FxHashMap<_, _>>();
            let setup_entries = setups
                .iter()
                .map(|(entry, _)| *entry)
                .collect::<FxHashSet<_>>();
            if self.exception_regions.is_empty()
                || setups.is_empty()
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.is_scope_fallback)
                || targets.len() != captured.len()
                || setup_entries.len() != setups.len()
                || targets.len() != setups.len()
                || setups.iter().any(|(entry, _)| !targets.contains_key(entry))
            {
                return Vec::new();
            }
            let positions = full_order
                .iter()
                .enumerate()
                .map(|(i, &block)| (block, i))
                .collect::<FxHashMap<_, _>>();
            for target in targets
                .values()
                .copied()
                .chain(self.exception_regions.iter().map(|region| region.target))
            {
                let Some(handler) = self
                    .graph
                    .label_block(target)
                    .and_then(|block| positions.get(&block).copied())
                else {
                    return Vec::new();
                };
                prefix_len = prefix_len.min(handler);
            }
            let mut prefix_arrays = FxHashSet::default();
            for (index, &block) in full_order[..prefix_len].iter().enumerate() {
                if self.graph[block].last_instruction().is_none()
                    || !original_arrays
                        .get(&block)
                        .is_some_and(|array| prefix_arrays.insert(*array))
                {
                    prefix_len = index;
                    break;
                }
            }
            if full_order[prefix_len..].iter().any(|block| {
                self.graph[*block].items().iter().any(|item| {
                    let Item::Instruction(instruction) = item else {
                        return false;
                    };
                    let (Operand::Forward(target) | Operand::Backward(target)) =
                        instruction.operand()
                    else {
                        return false;
                    };
                    self.graph
                        .label_block(target)
                        .and_then(|block| positions.get(&block))
                        .is_none_or(|position| *position < prefix_len)
                })
            }) {
                return Vec::new();
            }
        }
        let order = &full_order[..prefix_len];
        let Some(&entry) = order.first() else {
            return Vec::new();
        };
        // A conditional that targets its fallthrough queues the same not-yet-
        // visited block twice in 3.12's depth-first reachability traversal.
        if !order.windows(2).any(|pair| {
            block_has_fallthrough(self.graph[pair[0]].items())
                && block_jump_target(self.graph[pair[0]].items())
                    .and_then(|label| self.graph.label_block(label))
                    == Some(pair[1])
        }) {
            return Vec::new();
        }
        let mut arrays = FxHashSet::default();
        for &block in order {
            if self.graph[block].last_instruction().is_none()
                || !original_arrays
                    .get(&block)
                    .is_some_and(|array| arrays.insert(*array))
            {
                return Vec::new();
            }
        }
        let positions = order
            .iter()
            .enumerate()
            .map(|(index, &block)| (block, index))
            .collect::<FxHashMap<_, _>>();
        let mut visited = FxHashSet::default();
        let mut visits = FxHashMap::<BasicBlockId, usize>::default();
        let mut pending = vec![entry];
        let mut repeated_successors = FxHashSet::default();
        while let Some(block) = pending.pop() {
            let Some(&position) = positions.get(&block) else {
                return Vec::new();
            };
            let repeated = !visited.insert(block);
            let count = visits.entry(block).or_default();
            *count += 1;
            if *count > 2 {
                return Vec::new();
            }
            let fallthrough = block_has_fallthrough(self.graph[block].items())
                .then(|| order.get(position + 1).copied())
                .flatten();
            let target = block_jump_target(self.graph[block].items())
                .and_then(|label| self.graph.label_block(label))
                .filter(|target| positions.contains_key(target));
            for successor in fallthrough.into_iter().chain(target) {
                if repeated {
                    repeated_successors.insert(successor);
                }
                if !visited.contains(&successor) {
                    pending.push(successor);
                }
            }
        }
        order
            .iter()
            .copied()
            .filter(|block| repeated_successors.contains(block))
            .filter_map(|block| {
                let mut instructions = self.graph[block].items().iter().filter_map(|item| {
                    if let Item::Instruction(instruction) = item {
                        Some(instruction)
                    } else {
                        None
                    }
                });
                let instruction = instructions.next()?;
                if instructions.next().is_none()
                    && (instruction.opcode == JUMP_BACKWARD
                        || instruction.opcode == JUMP_BACKWARD_NO_INTERRUPT)
                    && instruction.location == SourceLocation::NONE
                {
                    Some((block, *instruction))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Simulates 3.12's reachability walk over original instruction arrays.
    ///
    /// Exception range boundaries can split one native array into several Serc
    /// blocks.  Merge those fragments in a temporary graph, then restore the
    /// authored SETUP edges that do not appear in the emitted instructions.
    /// Return `None` when the retained provenance cannot establish this graph.
    fn legacy_repeated_backedges_with_setups(
        &self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
    ) -> Option<Vec<(BasicBlockId, Instruction)>> {
        let setups = &self.exception_owner_provenance.setups;
        let captured = &self.exception_owner_provenance.legacy_native_setup_targets;
        let targets = captured.iter().copied().collect::<FxHashMap<_, _>>();
        let entries = setups
            .iter()
            .map(|(entry, _)| *entry)
            .collect::<FxHashSet<_>>();
        let handlers = targets.values().copied().collect::<FxHashSet<_>>();
        // Native 3.12 inserts one implicit generator SETUP before the entry RESUME.
        let fallback_handlers = self
            .exception_regions
            .iter()
            .filter(|region| region.is_scope_fallback)
            .map(|region| region.target)
            .collect::<FxHashSet<_>>();
        if fallback_handlers.len() > 1 {
            return None;
        }
        let wrapper = if let Some(&handler) = fallback_handlers.iter().next() {
            let mut matches = captured.iter().filter(|(_, target)| *target == handler);
            let (entry, _) = *matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            Some(entry)
        } else {
            None
        };
        if setups.is_empty()
            || targets.len() != captured.len()
            || entries.len() != setups.len()
            || targets.len() != setups.len()
            || setups.iter().any(|(entry, _)| !targets.contains_key(entry))
            || self
                .exception_regions
                .iter()
                .any(|region| !handlers.contains(&region.target))
        {
            return None;
        }

        let mut groups: Vec<Vec<BasicBlockId>> = Vec::new();
        let mut seen_arrays = FxHashSet::default();
        let mut previous = None;
        for &block in self.graph.order() {
            let array = *original_arrays.get(&block)?;
            if previous != Some(array) {
                if !seen_arrays.insert(array) {
                    return None;
                }
                groups.push(Vec::new());
                previous = Some(array);
            }
            let index = groups.len() - 1;
            groups[index].push(block);
        }
        if groups.is_empty() {
            return Some(Vec::new());
        }
        // Native removes initially empty arrays before this traversal, but an
        // array emptied by folding can still participate. The retained array
        // identity alone does not distinguish those cases.
        if groups.iter().any(|fragments| {
            fragments
                .iter()
                .all(|block| self.graph[*block].last_instruction().is_none())
        }) {
            return None;
        }

        // A branch into the middle of a group would require a native array
        // boundary that the captured array identities do not establish.
        let mut entry_labels = FxHashMap::default();
        for (index, fragments) in groups.iter().enumerate() {
            for item in fragments
                .iter()
                .flat_map(|block| self.graph[*block].items())
            {
                match item {
                    Item::Label(label) => {
                        entry_labels.insert(*label, index);
                    }
                    Item::Instruction(_) => break,
                }
            }
        }
        let label_group = |label| entry_labels.get(&label).copied();
        let handler_groups = handlers
            .iter()
            .map(|&label| label_group(label))
            .collect::<Option<FxHashSet<_>>>()?;
        if let Some(entry) = wrapper {
            let handler = targets[&entry];
            let entry_group = label_group(entry)?;
            let handler_group = label_group(handler)?;
            // Serc emits the startup instructions early, whereas native 3.12
            // inserts them only after reachability. The original SETUP and
            // RESUME must therefore map to the immediately following array.
            if entry_group != 1
                || handler_groups.contains(&0)
                || handler_groups.contains(&entry_group)
                || !self.exception_regions.iter().any(|region| {
                    region.is_scope_fallback && region.start == entry && region.target == handler
                })
            {
                return None;
            }
            let instructions = |index: usize| {
                groups[index]
                    .iter()
                    .flat_map(|block| self.graph[*block].items())
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>()
            };
            let prefix = instructions(0);
            let [before @ .., generator, pop] = prefix.as_slice() else {
                return None;
            };
            if generator.opcode != RETURN_GENERATOR
                || pop.opcode != POP_TOP
                || generator.operand() != Operand::Value(0)
                || pop.operand() != Operand::Value(0)
                || before.iter().enumerate().any(|(index, instruction)| {
                    instruction.opcode != MAKE_CELL
                        && !(index == 0 && instruction.opcode == COPY_FREE_VARS)
                })
                || !instructions(entry_group)
                    .first()
                    .is_some_and(|instruction| {
                        instruction.opcode == RESUME && instruction.operand() == Operand::Value(0)
                    })
            {
                return None;
            }
            let cleanup = instructions(handler_group);
            if !matches!(cleanup.as_slice(), [call, reraise]
                if call.opcode == CALL_INTRINSIC_1 && call.operand() == Operand::Value(3)
                    && reraise.opcode == RERAISE && reraise.operand() == Operand::Value(1))
                || self.graph.iter_items().any(|item| {
                    let Item::Instruction(instruction) = item else { return false; };
                    match instruction.operand() {
                        Operand::Forward(label) | Operand::Backward(label) => {
                            matches!(label_group(label), Some(group) if group == 0 || group == entry_group || group == handler_group)
                        }
                        _ => false,
                    }
                })
            {
                return None;
            }
        }
        let mut setup_edges = vec![Vec::new(); groups.len()];
        let mut entry_groups = FxHashSet::default();
        for &(entry, _) in setups {
            let position = label_group(entry)?;
            if !entry_groups.insert(position) {
                return None;
            }
            let target = label_group(targets[&entry])?;
            // The handler's cleanup SETUP is its first instruction. A body
            // SETUP instead precedes its entry label in the preceding array.
            let owner = if Some(entry) == wrapper || handler_groups.contains(&position) {
                position
            } else {
                let owner = position.checked_sub(1)?;
                let last = groups[owner]
                    .iter()
                    .rev()
                    .find_map(|block| self.graph[*block].last_instruction());
                if last.is_some_and(|instruction| {
                    is_unconditional_jump(instruction.opcode)
                        || matches!(
                            instruction.operand(),
                            Operand::Forward(_) | Operand::Backward(_)
                        )
                        || crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                }) {
                    return None;
                }
                owner
            };
            if Some(entry) == wrapper {
                setup_edges[owner].insert(0, target);
            } else {
                setup_edges[owner].push(target);
            }
        }

        let mut edges = vec![Vec::new(); groups.len()];
        for (index, fragments) in groups.iter().enumerate() {
            let instructions = fragments
                .iter()
                .flat_map(|block| self.graph[*block].items())
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            if instructions
                .iter()
                .take(instructions.len().saturating_sub(1))
                .any(|instruction| {
                    matches!(
                        instruction.operand(),
                        Operand::Forward(_) | Operand::Backward(_)
                    ) || crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                })
            {
                return None;
            }
            let falls_through = instructions.last().is_none_or(|instruction| {
                !is_unconditional_jump(instruction.opcode)
                    && !crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
            });
            if falls_through && index + 1 < groups.len() {
                edges[index].push(index + 1);
            }
            // CPython queues fallthrough first, then scans SETUP and jump
            // instructions in array order. SETUP in a handler precedes its
            // terminal jump; a body SETUP belongs to a non-branching array.
            edges[index].extend(setup_edges[index].iter().copied());
            if let Some(instruction) = instructions.last() {
                match instruction.operand() {
                    Operand::Forward(label) | Operand::Backward(label) => {
                        edges[index].push(label_group(label)?);
                    }
                    Operand::Value(_) | Operand::Deferred(_) => {}
                }
            }
        }

        let mut visited = vec![false; groups.len()];
        let mut repeated_successors = vec![false; groups.len()];
        let mut pending = vec![0];
        // Duplicate pending entries can multiply through a graph. Decline an
        // unbounded traversal rather than slowing compilation pathologically.
        let mut budget = groups.len().saturating_mul(16).max(64);
        while let Some(index) = pending.pop() {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            let repeated = visited[index];
            visited[index] = true;
            for &successor in &edges[index] {
                repeated_successors[successor] |= repeated;
                if !visited[successor] {
                    pending.push(successor);
                }
            }
        }

        let mut backedges = Vec::new();
        for (index, fragments) in groups.iter().enumerate() {
            if !repeated_successors[index] {
                continue;
            }
            let mut instructions = fragments.iter().flat_map(|block| {
                self.graph[*block]
                    .items()
                    .iter()
                    .filter_map(move |item| match item {
                        Item::Instruction(instruction) => Some((*block, instruction)),
                        Item::Label(_) => None,
                    })
            });
            let Some((block, instruction)) = instructions.next() else {
                continue;
            };
            if instructions.next().is_none()
                && matches!(
                    instruction.opcode,
                    JUMP_BACKWARD | JUMP_BACKWARD_NO_INTERRUPT
                )
                && instruction.location == SourceLocation::NONE
            {
                backedges.push((block, *instruction));
            }
        }
        Some(backedges)
    }

    fn legacy_predecessors(&self) -> FxHashMap<BasicBlockId, usize> {
        let mut predecessors = FxHashMap::default();
        let reachable = self.graph.reachable_blocks(&self.exception_regions);
        for (index, block) in self.graph.order().iter().copied().enumerate() {
            // Erasing dead instructions can leave an unreachable label-only block.
            // Its apparent fallthrough is not a native predecessor of the next exit.
            if self.graph[block].last_instruction().is_none() && !reachable[block.index()] {
                continue;
            }
            if block_has_fallthrough(self.graph[block].items())
                && let Some(next) = self.graph.order().get(index + 1)
            {
                *predecessors.entry(*next).or_default() += 1;
            }
            if let Some(target) = block_jump_target(self.graph[block].items())
                .and_then(|label| self.graph.label_block(label))
            {
                *predecessors.entry(target).or_default() += 1;
            }
        }
        predecessors
    }

    /// Gives wholly unlocated 3.12 return blocks a line after native location propagation.
    fn guarantee_legacy_exit_line_numbers(&mut self, first_line_number: u32) {
        let mut line = i32::try_from(first_line_number).unwrap_or(i32::MAX);
        let arrays = self.legacy_instruction_arrays();
        let order = self.graph.order().to_vec();
        for blocks in order.chunk_by(|left, right| arrays[left] == arrays[right]) {
            let Some(last) = blocks
                .iter()
                .rev()
                .find_map(|block| self.graph[*block].last_instruction().copied())
            else {
                continue;
            };
            if last.location.line >= 0 {
                line = last.location.line;
            } else if last.opcode == RETURN_VALUE {
                // Exception-range endpoints can split this native return block
                // into physical fragments. Fill the entire original array, while
                // preserving CPython's unknown columns and end line.
                let mut first_instruction = true;
                for &block in blocks {
                    for instruction in self.graph.instructions_mut(block) {
                        // A later cold predecessor can locate the leading POP_BLOCK
                        // after this return was visited. The pinned release compiler
                        // still overwrites its line with the exit fallback below.
                        debug_assert!(
                            instruction.location.line < 0
                                || (first_instruction
                                    && instruction.opcode == NOP
                                    && instruction.fusion_policy.is_converted_pop_block())
                        );
                        first_instruction = false;
                        instruction.location.line = line;
                    }
                }
            }
        }
    }

    fn resolve_legacy_locations(
        &mut self,
        mut predecessors: FxHashMap<BasicBlockId, usize>,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
    ) {
        let mut next_native_label = self
            .legacy_native_labels
            .as_ref()
            .filter(|native| native.is_certified())
            .map(|native| native.next_cfg_label(&self.graph));
        for block in self.graph.order().to_vec() {
            let Some(jump) = self.graph[block].last_instruction().copied() else {
                continue;
            };
            let Some(target) = block_jump_target(self.graph[block].items())
                .and_then(|label| self.graph.label_block(label))
            else {
                continue;
            };
            if predecessors.get(&target).copied().unwrap_or(0) <= 1 {
                continue;
            }
            // Exception-range endpoints can split the original cleanup array.
            // Native duplicates the complete unlocated exit, including its return
            // or eval-check jump, rather than just the first physical fragment.
            let target_index = self
                .graph
                .order()
                .iter()
                .position(|block| *block == target)
                .unwrap();
            let target_array = original_arrays.get(&target).copied();
            let targets = self.graph.order()[target_index..]
                .iter()
                .copied()
                .take_while(|block| {
                    *block == target
                        || (target_array.is_some()
                            && original_arrays.get(block).copied() == target_array)
                })
                .collect::<Vec<_>>();
            let last_target = *targets.last().unwrap();
            let instructions = targets
                .iter()
                .flat_map(|target| self.graph[*target].items())
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(*instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            if !instructions.last().is_some_and(|instruction| {
                crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                    // 3.13 also copies unlocated eval-check blocks. A shared
                    // backedge must inherit each condition's complete span before
                    // jump threading removes the original conditional record.
                    || (self.python_version == PythonVersion::Py313
                        && legacy_jump_has_eval_check(instruction))
            }) || instructions
                .iter()
                .any(|instruction| instruction.location.line >= 0)
            {
                continue;
            }
            let entry = self.label();
            let mut copied = targets
                .iter()
                .flat_map(|target| self.copy_legacy_block_items(*target, last_target))
                .collect::<Vec<_>>();
            let Some(Item::Instruction(first)) = copied
                .iter_mut()
                .find(|item| matches!(item, Item::Instruction(_)))
            else {
                unreachable!("eligible legacy exit contains an instruction");
            };
            first.location = jump.location;
            let index = self
                .graph
                .order()
                .iter()
                .position(|id| *id == last_target)
                .unwrap()
                + 1;
            let copy = self.graph.insert_block(
                index,
                std::iter::once(Item::Label(entry)).chain(copied).collect(),
            );
            if let (Some(native), Some(next)) = (
                self.legacy_native_labels.as_mut(),
                next_native_label.as_mut(),
            ) {
                native.allocate_for_block(copy, next);
            }
            self.graph
                .instructions_mut(block)
                .next_back()
                .unwrap()
                .retarget(Operand::Forward(entry));
            *predecessors.get_mut(&target).unwrap() -= 1;
            predecessors.insert(copy, 1);
        }
        let order = self.graph.order().to_vec();
        for (index, block) in order.iter().copied().enumerate() {
            let mut location = SourceLocation::NONE;
            let mut follows_generator_prefix = false;
            for instruction in self.graph.instructions_mut(block) {
                // 3.12 inserts this startup POP_TOP after CFG line propagation. It
                // retains NO_LOCATION even though RETURN_GENERATOR has a source line.
                let inserted_generator_pop = self.python_version == PythonVersion::Py312
                    && follows_generator_prefix
                    && instruction.opcode == POP_TOP;
                if instruction.location.line < 0 {
                    // Native propagation ignores modern preservation flags except
                    // certified shared POP_BLOCKs and later prefix insertion.
                    if !inserted_generator_pop
                        && !legacy_pop_block_keeps_no_location(self.python_version, instruction)
                    {
                        instruction.location = location;
                    }
                } else {
                    location = instruction.location;
                }
                follows_generator_prefix = instruction.opcode == RETURN_GENERATOR;
            }
            let fallthrough = block_has_fallthrough(self.graph[block].items())
                .then(|| order.get(index + 1).copied())
                .flatten();
            let target = block_jump_target(self.graph[block].items())
                .and_then(|label| self.graph.label_block(label));
            for mut successor in fallthrough.into_iter().chain(target) {
                // Native 3.12 removes empty blocks before resolving lines. Erased
                // branches can leave label-only fragments between a donor and
                // its real successor; they must not swallow that unique edge.
                while self.python_version == PythonVersion::Py312
                    && predecessors.get(&successor) == Some(&1)
                    && self.graph[successor].last_instruction().is_none()
                {
                    let Some(next) = order
                        .iter()
                        .position(|block| *block == successor)
                        .and_then(|index| order.get(index + 1))
                    else {
                        break;
                    };
                    successor = *next;
                }
                if predecessors.get(&successor) == Some(&1)
                    && let Some(first) = self.graph.instructions_mut(successor).next()
                    && first.location.line < 0
                    && !legacy_pop_block_keeps_no_location(self.python_version, first)
                {
                    first.location = location;
                }
            }
        }
    }
}

/// Keeps a shared `POP_BLOCK` unlocated while 3.13 still has its folded predecessor.
///
/// The compiler certifies joins whose constant-test false edge has already been
/// elided. Native 3.13 resolves locations before folding that edge; 3.12 folds
/// first, so its remaining predecessor may donate a location normally.
fn legacy_pop_block_keeps_no_location(version: PythonVersion, instruction: &Instruction) -> bool {
    version == PythonVersion::Py313
        && instruction.opcode == NOP
        && instruction.fusion_policy.is_converted_pop_block()
        && instruction.no_location_policy.preserves_location()
}

/// Recovers native 3.13's direction-independent JUMP pseudo-op.
///
/// Forward `JUMP` and `JUMP_NO_INTERRUPT` share a wire opcode. Authored breaks
/// carry the ordinary kind in their emission certificate; copied and threaded
/// jumps retain it through the interruptible policy bit.
fn legacy_jump_has_eval_check(instruction: &Instruction) -> bool {
    is_unconditional_jump(instruction.opcode)
        && (instruction.opcode == JUMP_BACKWARD
            || instruction.jump_policy.is_interruptible()
            || instruction.fusion_policy.is_authored_break())
}

fn legacy_nop(instruction: &mut Instruction) {
    instruction.opcode = NOP;
    instruction.set_operand(Operand::Value(0));
}

fn fuse_legacy_fast_pair(first: &Instruction, second: &Instruction) -> Option<Instruction> {
    // CPython still represents these records as LOAD_CLOSURE and
    // STORE_FAST_MAYBE_NULL when it forms 3.13's owned-local superinstructions.
    if first.fusion_policy.prevents_with_next()
        || second.fusion_policy.prevents_with_previous()
        || first.local_policy.is_post_analysis_closure_load()
        || second.local_policy.is_post_analysis_closure_load()
        || first.local_policy.may_store_null()
        || second.local_policy.may_store_null()
    {
        return None;
    }
    let opcode = match (first.opcode, second.opcode) {
        (LOAD_FAST, LOAD_FAST) => LOAD_FAST_LOAD_FAST,
        (STORE_FAST, LOAD_FAST) => STORE_FAST_LOAD_FAST,
        (STORE_FAST, STORE_FAST) => STORE_FAST_STORE_FAST,
        _ => return None,
    };
    let (Operand::Value(first_argument), Operand::Value(second_argument)) =
        (first.operand(), second.operand())
    else {
        return None;
    };
    if first_argument >= 16
        || second_argument >= 16
        || (first.location.line >= 0
            && second.location.line >= 0
            && first.location.line != second.location.line)
    {
        return None;
    }
    Some(Instruction::fused(
        opcode,
        Operand::Value((first_argument << 4) | second_argument),
        *first,
        *second,
    ))
}

#[cfg(test)]
mod tests {
    // Add inside assembler/passes/legacy.rs tests. No oracle or source-path dependency.
    #[test]
    fn native_jump_seeds_preserve_exact_captures_and_reject_partial_updates() {
        use crate::assembler::exception_owners::{
            AsyncWithSetup, OrdinaryAwaitEnd, SourceNoneCondition,
        };
        use crate::target::opcodes::{END_SEND, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, SEND};

        let cases = [(0, 254), (0, 255), (0, 300), (1, 300), (2, 300), (3, 300)];
        for (unknown_capture, native_label) in cases {
            let start = Label::from_u32(0);
            let first_body = Label::from_u32(1);
            let second_body = Label::from_u32(2);
            let end = Label::from_u32(3);
            let normal_end = Label::from_u32(4);
            let location = SourceLocation::new(2, 2, 4, 12);
            let primitive =
                Instruction::new(POP_JUMP_IF_FALSE, Operand::Forward(end), location, Some(0));
            let none = Instruction::new(POP_JUMP_IF_NONE, Operand::Forward(end), location, Some(0));
            let send = Instruction::new(SEND, Operand::Forward(end), location, Some(1));
            let end_send = Instruction::new(END_SEND, Operand::Value(0), location, Some(0));
            let normal_exit = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(normal_end),
                location,
                Some(0),
            );
            let mut assembler = Assembler::from_linear_items(vec![
                Item::Label(start),
                Item::Instruction(primitive),
                Item::Label(first_body),
                Item::Instruction(none),
                Item::Label(second_body),
                Item::Instruction(send),
                Item::Label(end),
                Item::Instruction(end_send),
                Item::Instruction(normal_exit),
                Item::Label(normal_end),
                Item::Instruction(Instruction::new(
                    RETURN_CONST,
                    Operand::Value(0),
                    location,
                    Some(0),
                )),
            ]);
            assembler.python_version = PythonVersion::Py312;
            assembler.next_label = 5;
            assembler.bind_legacy_native_label(start, 1);
            assembler.bind_legacy_native_label(end, native_label);
            assembler.bind_legacy_native_label(normal_end, native_label + 1);
            assembler.certify_legacy_native_labels();
            assembler
                .exception_owner_provenance
                .original_primitive_condition_boundaries
                .push(primitive);
            let mut captured_none = none;
            if unknown_capture == 1 {
                captured_none.retarget(Operand::Forward(Label::from_u32(900)));
            }
            assembler
                .exception_owner_provenance
                .source_none_conditions
                .push(SourceNoneCondition {
                    entry: first_body,
                    labels: vec![first_body],
                    instruction: captured_none,
                    entry_prefix: None,
                });
            assembler
                .exception_owner_provenance
                .ordinary_await_ends
                .push(OrdinaryAwaitEnd {
                    send,
                    entry: end,
                    end_send,
                    cleanup: Vec::new(),
                    bridge: None,
                });
            let mut manager_send = send;
            if unknown_capture == 2 {
                manager_send.retarget(Operand::Forward(Label::from_u32(901)));
            }
            assembler
                .exception_owner_provenance
                .async_with_setups
                .push(AsyncWithSetup {
                    setup: end,
                    send: manager_send,
                    prefix: vec![Item::Label(end), Item::Instruction(end_send)],
                    cleanup: Vec::new(),
                    bridge: None,
                });
            let mut captured_normal_exit = normal_exit;
            if unknown_capture == 3 {
                captured_normal_exit.retarget(Operand::Forward(Label::from_u32(902)));
            }
            assembler
                .exception_owner_provenance
                .original_comprehension_normal_exits
                .push((normal_end, captured_normal_exit));
            let mut pipeline = assembler.into_optimization_pipeline();
            let original_arrays = pipeline.legacy_instruction_arrays();
            pipeline.initialize_legacy_native_jump_seeds(&original_arrays);
            let expected = (unknown_capture == 0).then_some(u8::from(native_label > 255));
            let normal_expected =
                (unknown_capture == 0).then_some(u8::from(native_label + 1 > 255));
            let live = |opcode| {
                pipeline
                    .graph
                    .iter_items()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) if instruction.opcode == opcode => {
                            Some(*instruction)
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            for opcode in [POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, SEND] {
                assert_eq!(live(opcode).legacy_initial_jump_width(), expected);
            }
            assert_eq!(
                live(JUMP_FORWARD).legacy_initial_jump_width(),
                normal_expected
            );
            let provenance = &pipeline.exception_owner_provenance;
            assert_eq!(
                provenance.original_comprehension_normal_exits[0]
                    .1
                    .legacy_initial_jump_width(),
                normal_expected
            );
            if unknown_capture != 3 {
                assert_eq!(
                    provenance.original_comprehension_normal_exits[0].1,
                    live(JUMP_FORWARD)
                );
            }
            assert_eq!(
                provenance.original_primitive_condition_boundaries[0],
                live(POP_JUMP_IF_FALSE)
            );
            assert_eq!(provenance.ordinary_await_ends[0].send, live(SEND));
            assert_eq!(
                provenance.source_none_conditions[0]
                    .instruction
                    .legacy_initial_jump_width(),
                expected
            );
            if unknown_capture != 1 {
                assert_eq!(
                    provenance.source_none_conditions[0].instruction,
                    live(POP_JUMP_IF_NONE)
                );
            }
            assert_eq!(
                provenance.async_with_setups[0]
                    .send
                    .legacy_initial_jump_width(),
                expected
            );
            if unknown_capture != 2 {
                assert_eq!(provenance.async_with_setups[0].send, live(SEND));
            }
            assert_eq!(
                pipeline
                    .legacy_native_labels
                    .as_ref()
                    .unwrap()
                    .is_certified(),
                unknown_capture == 0
            );
        }
    }

    use super::*;
    use crate::assembler::{ExceptionPolicy, Label};
    use crate::target::opcodes::{
        LOAD_CONST, POP_JUMP_IF_FALSE, RERAISE, RETURN_CONST, RETURN_VALUE,
    };

    #[test]
    fn legacy_high_locals_respect_native_block_boundaries() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        // CPython 3.12.14 resets high locals at the initial while test and SETUP
        // continuation. 3.13.15 retains initialization across these entries.
        // The straight-line control must keep unchecked loads on both targets.
        let parameters = (0..64)
            .map(|index| format!("a{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let cases: &[(PythonVersion, &str, &[u8])] = &[
            (
                PythonVersion::Py312,
                "    value = next_value()\n    while value == 0:\n        value = next_value()\n    return value\n",
                &[
                    151, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 125, 64, 127,
                    64, 100, 1, 107, 40, 0, 0, 114, 16, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0,
                    0, 0, 0, 0, 0, 125, 64, 124, 64, 100, 1, 107, 40, 0, 0, 114, 1, 140, 16, 127,
                    64, 83, 0,
                ],
            ),
            (
                PythonVersion::Py312,
                "    for value in values:\n        try:\n            consume(value)\n        except Exception:\n            pass\n",
                &[
                    151, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 68, 0, 93, 14, 0, 0, 125, 64, 9, 0,
                    116, 3, 0, 0, 0, 0, 0, 0, 0, 0, 127, 64, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 140,
                    16, 4, 0, 121, 0, 35, 0, 116, 4, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 3, 1, 0,
                    89, 0, 140, 29, 119, 0, 120, 3, 89, 0, 119, 1,
                ],
            ),
            (
                PythonVersion::Py312,
                "    value = next_value()\n    consume(value)\n    return value\n",
                &[
                    151, 0, 116, 1, 0, 0, 0, 0, 0, 0, 0, 0, 171, 0, 0, 0, 0, 0, 0, 0, 125, 64, 116,
                    3, 0, 0, 0, 0, 0, 0, 0, 0, 124, 64, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 124, 64,
                    83, 0,
                ],
            ),
            (
                PythonVersion::Py313,
                "    value = next_value()\n    while value == 0:\n        value = next_value()\n    return value\n",
                &[
                    149, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 110, 64, 85,
                    64, 83, 1, 58, 88, 0, 0, 97, 18, 0, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0,
                    0, 0, 0, 0, 0, 110, 64, 85, 64, 83, 1, 58, 88, 0, 0, 97, 2, 0, 0, 77, 18, 0, 0,
                    87, 64, 36, 0,
                ],
            ),
            (
                PythonVersion::Py313,
                "    for value in values:\n        try:\n            consume(value)\n        except Exception:\n            pass\n",
                &[
                    149, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 19, 0, 72, 15, 0, 0, 110, 64, 30, 0, 91,
                    3, 0, 0, 0, 0, 0, 0, 0, 0, 85, 64, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 77, 17, 0,
                    0, 11, 0, 32, 0, 103, 0, 33, 0, 91, 4, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 4, 0,
                    0, 32, 0, 31, 0, 77, 33, 0, 0, 102, 0, 61, 3, 31, 0, 102, 1,
                ],
            ),
            (
                PythonVersion::Py313,
                "    value = next_value()\n    consume(value)\n    return value\n",
                &[
                    149, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 110, 64, 91, 3,
                    0, 0, 0, 0, 0, 0, 0, 0, 85, 64, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 85, 64, 36, 0,
                ],
            ),
        ];
        for &(python_version, body, expected) in cases {
            let source = format!("def f({parameters}):\n{body}");
            let module = compile_with_options(
                &source,
                "high_locals.py",
                &CompileOptions {
                    python_version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "f" => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.bytecode, expected, "{python_version:?}: {body}");
        }
    }

    #[test]
    fn legacy_exit_copies_keep_donor_exception_owners() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        // Exact tables from pinned CPython 3.12.14 / 3.13.15. These independent
        // public cases have matching code bytes but used to retain the caller's
        // handler on copied returns and POP_EXCEPT/name-clearing instructions.
        let cases: &[(PythonVersion, &str, &[u8])] = &[
            (
                PythonVersion::Py312,
                "def terminal_try_manager(manager, work, Error, handle):\n    try:\n        with manager:\n            work()\n    except Error:\n        handle()\n",
                &[
                    130, 2, 33, 0, 132, 8, 21, 3, 140, 8, 33, 0, 149, 5, 30, 7, 154, 3, 33, 0, 158,
                    3, 33, 0, 161, 12, 48, 3, 175, 1, 48, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                "def terminal_try_manager(manager, work, Error, handle):\n    try:\n        with manager:\n            work()\n    except Error:\n        handle()\n",
                &[
                    130, 2, 38, 0, 132, 8, 21, 3, 140, 8, 38, 0, 149, 10, 35, 7, 159, 3, 38, 0,
                    163, 3, 38, 0, 166, 13, 54, 3, 181, 1, 54, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                "def named_handler_reraise(work, Error, handled, finish):\n    try:\n        work()\n    except Error as error:\n        if handled(error):\n            pass\n        else:\n            raise\n    finish()\n",
                &[
                    130, 7, 17, 0, 145, 6, 47, 3, 151, 13, 42, 3, 169, 1, 42, 3, 170, 5, 47, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                "def manager_nested_handler_finally(manager, work, Error, handle, cleanup, finish):\n    with manager:\n        try:\n            work()\n        except Error as error:\n            try:\n                handle(error)\n            finally:\n                cleanup()\n    finish()\n",
                &[
                    131, 1, 65, 11, 3, 133, 7, 28, 2, 156, 6, 65, 8, 5, 163, 8, 55, 4, 171, 7, 65,
                    3, 5, 178, 5, 65, 11, 3, 183, 9, 65, 0, 7, 193, 0, 3, 65, 3, 5, 193, 3, 5, 65,
                    8, 5, 193, 8, 3, 65, 11, 3, 193, 11, 10, 65, 25, 7,
                ],
            ),
        ];
        for &(version, source, expected) in cases {
            let module = compile_with_options(
                source,
                "legacy_copy_owners.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                function.exception_table, expected,
                "{version:?}: {}",
                function.name
            );
        }
    }

    #[test]
    fn legacy_generator_break_resolves_none_before_constant_optimization() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let sources = [
            "def f(xs):\n    for x in xs:\n        yield x\n        break\n",
            "def f():\n    while True:\n        value = yield 7\n        if value:\n            break\n",
            "def f(xs):\n    for x in xs:\n        yield x\n        if x:\n            break\n        yield 7\n",
            "def f(xs):\n    \"A public generator docstring.\"\n    for x in xs:\n        yield x\n        break\n",
        ];
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for (index, source) in sources.iter().enumerate() {
                let module = compile_with_options(
                    source,
                    "generator_break.py",
                    &CompileOptions {
                        python_version: version,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .code;
                let function = module
                    .constants
                    .iter()
                    .find_map(|constant| match constant {
                        Constant::Code(code) if code.name == "f" => Some(code),
                        _ => None,
                    })
                    .unwrap();
                // Both pinned native compilers preserve admission order. A docstring
                // occupies slot zero, so the break must use the actual None index.
                let none_index = u8::from(index == 3);
                if index == 1 {
                    // Native constant-true loops retain the original backedge after
                    // the break return, without synthesizing an ownerless edge.
                    let expected: &[u8] = if version == PythonVersion::Py312 {
                        &[
                            75, 0, 1, 0, 151, 0, 9, 0, 100, 1, 150, 1, 151, 1, 125, 0, 124, 0, 114,
                            1, 121, 0, 140, 8, 173, 3, 119, 1,
                        ]
                    } else {
                        &[
                            35, 0, 32, 0, 149, 0, 30, 0, 83, 1, 118, 0, 149, 5, 110, 0, 85, 0, 40,
                            0, 0, 0, 0, 0, 0, 0, 97, 1, 0, 0, 103, 0, 77, 14, 0, 0, 55, 3, 102, 1,
                        ]
                    };
                    assert_eq!(function.bytecode, expected, "{version:?}: {source}");
                }
                assert!(
                    match index {
                        0 => matches!(function.constants.as_slice(), [Constant::None]),
                        3 => matches!(
                            function.constants.as_slice(),
                            [Constant::String(doc), Constant::None]
                                if doc == "A public generator docstring."
                        ),
                        _ => matches!(
                            function.constants.as_slice(),
                            [Constant::None, Constant::Int(7)]
                        ),
                    },
                    "{version:?}: {source}: {:?}",
                    function.constants
                );
                assert!(
                    function
                        .bytecode
                        .as_chunks::<2>()
                        .0
                        .contains(&[RETURN_CONST.code_for(version), none_index]),
                    "{version:?}: {source}"
                );
            }
        }
    }

    #[test]
    fn legacy_conditional_continue_retains_the_authored_jump_block() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(xs, a, b):\n    for x in xs:\n        if a or b:\n            continue\n        yield x\n";
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            let module = compile_with_options(
                source,
                "cfg.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "f" => Some(code),
                    _ => None,
                })
                .unwrap();
            // Captured from pinned CPython 3.12.14 and 3.13.15. The continue's
            // temporary NOP prevents the first conditional from being threaded
            // into a duplicated backedge with a gap in the generator handler.
            let (bytecode, exception_table): (&[u8], &[u8]) = match version {
                PythonVersion::Py312 => (
                    &[
                        75, 0, 1, 0, 151, 0, 124, 0, 68, 0, 93, 11, 0, 0, 125, 3, 124, 1, 115, 2,
                        124, 2, 114, 1, 140, 8, 124, 3, 150, 1, 151, 1, 1, 0, 140, 13, 4, 0, 121,
                        0, 173, 3, 119, 1,
                    ],
                    &[130, 18, 20, 1],
                ),
                PythonVersion::Py313 => (
                    &[
                        35, 0, 32, 0, 149, 0, 85, 0, 19, 0, 72, 23, 0, 0, 110, 3, 85, 1, 40, 0, 0,
                        0, 0, 0, 0, 0, 100, 7, 0, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0,
                        77, 19, 0, 0, 85, 3, 118, 0, 149, 5, 32, 0, 77, 25, 0, 0, 11, 0, 32, 0,
                        103, 0, 55, 3, 102, 1,
                    ],
                    &[130, 31, 33, 1],
                ),
                _ => unreachable!(),
            };
            assert_eq!(function.bytecode, bytecode, "{version:?}");
            assert_eq!(function.exception_table, exception_table, "{version:?}");
        }
    }

    #[test]
    fn legacy_implicit_backedge_remains_threadable() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(xs):\n for x in xs:\n  if x: break\n else: return 1\n return 2\n";
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            let module = compile_with_options(
                source,
                "cfg.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "f" => Some(code),
                    _ => None,
                })
                .unwrap();
            // Unlike an authored continue, the loop's implicit unlocated jump
            // has no leading NOP. Native threading exposes a backward conditional
            // that final normalization reverses before the break cleanup.
            let expected: &[u8] = if version == PythonVersion::Py312 {
                &[
                    151, 0, 124, 0, 68, 0, 93, 6, 0, 0, 125, 1, 124, 1, 115, 1, 140, 6, 1, 0, 121,
                    2, 4, 0, 121, 1,
                ]
            } else {
                &[
                    149, 0, 85, 0, 19, 0, 72, 12, 0, 0, 110, 1, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0,
                    100, 2, 0, 0, 77, 12, 0, 0, 32, 0, 103, 2, 11, 0, 32, 0, 103, 1,
                ]
            };
            assert_eq!(function.bytecode, expected, "{version:?}");
        }
    }

    #[test]
    fn python_312_try_body_pop_block_prevents_early_conditional_threading() {
        use crate::{CompileOptions, compile_with_options};

        let source = "try:\n    first = acquire()\n    second = prepare(first)\n    if (second / \"item\").ready():\n        consume(second)\nexcept Failure:\n    recover()\nfinish()\n";
        let code = compile_with_options(
            source,
            "terminal_try_condition.py",
            &CompileOptions {
                python_version: PythonVersion::Py312,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        // The shared unlocated POP_BLOCK keeps both normal predecessors until
        // native location propagation. It then disappears without a line event.
        // These boundaries are captured from pinned CPython 3.12.14.
        assert_eq!(code.bytecode.len(), 140);
        assert_eq!(code.bytecode[70..72], [114, 8]);
        assert_eq!(
            code.exception_table,
            [130, 42, 52, 0, 180, 12, 65, 3, 3, 193, 2, 1, 65, 3, 3]
        );
    }

    #[test]
    fn python_313_conditional_retains_no_interrupt_join() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(a, b, c):\n    if a:\n        if b:\n            raise ValueError\n    elif c:\n        raise TypeError\n    result(a)\n";
        let module = compile_with_options(
            source,
            "cfg.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        // CPython 3.13.15 keeps the inner false branch targeting the separate
        // JUMP_NO_INTERRUPT join, even after assigning that jump a source line.
        assert_eq!(
            function.bytecode,
            [
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 14, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0,
                0, 97, 6, 0, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 101, 1, 79, 13, 85, 2, 40, 0, 0, 0,
                0, 0, 0, 0, 97, 6, 0, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 101, 1, 91, 5, 0, 0, 0, 0,
                0, 0, 0, 0, 85, 0, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0,
            ]
        );
    }

    #[test]
    fn python_313_break_copy_retains_its_eval_check() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(values):\n    try:\n        work()\n    except Error as error:\n        for value in values:\n            break\n    finish()\n";
        let module = compile_with_options(
            source,
            "cfg.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        // Pinned CPython 3.13.15 copies the named-handler cleanup onto the
        // authored break. That copy keeps JUMP's eval check, while normal
        // exhaustion retains the cleanup's original JUMP_NO_INTERRUPT.
        assert_eq!(
            function.bytecode,
            [
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 91,
                5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 2, 0,
                0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 24, 0, 0, 110, 1, 85, 0, 19, 0, 72, 8, 0, 0, 110, 2,
                32, 0, 31, 0, 83, 0, 110, 1, 65, 1, 77, 33, 0, 0, 11, 0, 32, 0, 31, 0, 83, 0, 110,
                1, 65, 1, 78, 40, 83, 0, 110, 1, 65, 1, 102, 1, 102, 0, 61, 3, 31, 0, 102, 1,
            ]
        );
        assert_eq!(
            function.exception_table,
            [
                130, 10, 23, 0, 151, 10, 57, 3, 161, 6, 52, 3, 173, 2, 52, 3, 180, 5, 57, 3
            ]
        );
    }

    #[test]
    fn python_313_finally_join_keeps_the_unfolded_loop_predecessor() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f():\n    try:\n        while True:\n            work()\n    except Error as error:\n        recover(error)\n    finally:\n        finish()\n";
        let module = compile_with_options(
            source,
            "cfg.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        // Pinned CPython 3.13.15 removes the unlocated normal POP_BLOCK.
        // The recovery's line must not create a NOP before the finally call.
        assert_eq!(
            function.bytecode,
            [
                149, 0, 30, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32,
                0, 77, 12, 0, 0, 33, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 21, 0, 0, 110, 0,
                91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 85, 0, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 31, 0, 83, 0,
                110, 0, 65, 0, 79, 8, 83, 0, 110, 0, 65, 0, 102, 1, 102, 0, 61, 3, 31, 0, 102, 1,
                91, 7, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91,
                7, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 102, 0, 61, 3, 31, 0,
                102, 1,
            ]
        );
        assert_eq!(
            function.exception_table,
            [
                130, 13, 15, 0, 143, 10, 46, 3, 153, 11, 41, 3, 164, 5, 60, 0, 169, 5, 46, 3, 174,
                3, 60, 0, 188, 12, 65, 8, 3
            ]
        );
    }

    #[test]
    fn python_313_match_condition_threads_through_ordinary_case_exit() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(value, check):\n    match value:\n        case 1:\n            if check:\n                raise Error\n        case _:\n            raise Other\n    finish()\n";
        let module = compile_with_options(
            source,
            "cfg.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        // Pinned CPython 3.13.15: the false condition threads through the
        // case's ordinary JUMP directly to the continuation after the match.
        assert_eq!(
            function.bytecode,
            [
                149, 0, 85, 0, 83, 1, 58, 88, 0, 0, 97, 13, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0,
                97, 13, 0, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 101, 1, 30, 0, 91, 2, 0, 0, 0, 0, 0,
                0, 0, 0, 101, 1, 91, 5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0,
                103, 0,
            ]
        );
    }

    #[test]
    fn legacy_setup_nop_keeps_its_span_until_pseudo_conversion() {
        let start = Label::from_u32(0);
        let end = Label::from_u32(1);
        let setup_location = SourceLocation::new(2, 3, 1, 16);
        let return_location = SourceLocation::new(2, 2, 6, 16);
        let mut assembler = Assembler::from_linear_items(vec![
            Item::Label(start),
            Item::Instruction(Instruction::new(
                NOP,
                Operand::Value(0),
                setup_location,
                None,
            )),
            Item::Label(end),
            Item::Instruction(Instruction::new(
                NOP,
                Operand::Value(0),
                return_location,
                None,
            )),
            Item::Instruction(Instruction::new(
                RETURN_CONST,
                Operand::Value(0),
                SourceLocation::new(3, 3, 10, 16),
                None,
            )),
        ]);
        assembler.python_version = PythonVersion::Py313;
        assembler.next_label = 2;
        assembler.record_setup_nop_boundaries(start, end);
        let mut pipeline = assembler.into_optimization_pipeline();
        let original_arrays = pipeline.legacy_instruction_arrays();
        pipeline.preserve_legacy_setup_nops();
        pipeline.remove_legacy_nops(&original_arrays, false);
        pipeline.remove_legacy_nops(&original_arrays, true);
        let locations = pipeline
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) if instruction.opcode == NOP => {
                    Some(instruction.location)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(locations, [setup_location]);
        pipeline.graph.validate().unwrap();
    }

    #[test]
    fn legacy_small_exit_copying_includes_raising_blocks() {
        for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
            for opcode in [crate::target::opcodes::RAISE_VARARGS, RERAISE] {
                let target = Label::from_u32(0);
                let location = SourceLocation::new(2, 2, 0, 5);
                let exit = Instruction::new(opcode, Operand::Value(0), location, Some(0));
                let mut assembler = Assembler::from_linear_items(vec![
                    Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(target),
                        location,
                        Some(0),
                    )),
                    Item::Label(target),
                    Item::Instruction(exit),
                ]);
                assembler.python_version = python_version;
                assembler.next_label = 1;
                let mut pipeline = assembler.into_optimization_pipeline();
                let original_arrays = pipeline.legacy_instruction_arrays();
                pipeline.inline_legacy_exit_blocks(&original_arrays, true);
                let first = pipeline.graph.block_at(0);
                assert_eq!(pipeline.graph[first].last_instruction(), Some(&exit));
                pipeline.graph.validate().unwrap();
            }
        }
    }

    #[test]
    fn legacy_exit_copies_cross_only_metadata_boundaries() {
        use crate::target::opcodes::CALL;

        for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
            for source_boundary in [false, true] {
                let target = Label::from_u32(0);
                let range_end = Label::from_u32(1);
                let handler = Label::from_u32(2);
                let location = SourceLocation::new(2, 2, 0, 5);
                let mut assembler = Assembler::from_linear_items(vec![
                    Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(target),
                        location,
                        None,
                    )),
                    Item::Label(target),
                    Item::Instruction(Instruction::new(CALL, Operand::Value(1), location, None)),
                    Item::Label(range_end),
                    Item::Instruction(Instruction::new(
                        RETURN_VALUE,
                        Operand::Value(0),
                        location,
                        None,
                    )),
                    Item::Label(handler),
                    Item::Instruction(Instruction::new(RERAISE, Operand::Value(0), location, None)),
                ]);
                assembler.python_version = python_version;
                assembler.next_label = 3;
                assembler.add_exception_region(target, range_end, handler, 0, false);
                if source_boundary {
                    assembler.record_exception_owner_block_boundary(range_end);
                }
                let mut pipeline = assembler.into_optimization_pipeline();
                let original_arrays = pipeline.legacy_instruction_arrays();
                pipeline.inline_legacy_exit_blocks(&original_arrays, true);
                let first = pipeline.graph.block_at(0);
                let instructions = pipeline.graph[first]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>();
                if source_boundary {
                    assert_eq!(instructions.len(), 1);
                    assert_eq!(instructions[0].opcode, JUMP_FORWARD);
                } else {
                    assert_eq!(
                        instructions
                            .iter()
                            .map(|instruction| instruction.opcode)
                            .collect::<Vec<_>>(),
                        [NOP, CALL, RETURN_VALUE],
                    );
                    assert_eq!(
                        instructions[2].exception_policy(),
                        ExceptionPolicy::AlwaysAbsent,
                    );
                    assert!(pipeline.exception_regions.iter().any(|region| {
                        region.target == handler
                            && pipeline.graph.label_block(region.start) == Some(first)
                            && pipeline.graph.label_block(region.end) == Some(first)
                    }));
                }
                pipeline.graph.validate().unwrap();
            }
        }
    }

    #[test]
    fn legacy_exit_copy_budget_uses_original_slots_before_folding() {
        use crate::target::opcodes::CALL;

        for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
            for fused_null in [false, true] {
                let target = Label::from_u32(0);
                let location = SourceLocation::new(2, 2, 0, 5);
                let load = Instruction::new(
                    LOAD_GLOBAL,
                    Operand::Value(u32::from(fused_null)),
                    location,
                    None,
                );
                let mut assembler = Assembler::from_linear_items(vec![
                    Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(target),
                        location,
                        None,
                    )),
                    Item::Label(target),
                    Item::Instruction(load),
                    Item::Instruction(Instruction::new(
                        LOAD_FAST,
                        Operand::Value(0),
                        location,
                        None,
                    )),
                    Item::Instruction(Instruction::new(CALL, Operand::Value(1), location, None)),
                    Item::Instruction(Instruction::new(
                        RETURN_VALUE,
                        Operand::Value(0),
                        location,
                        None,
                    )),
                ]);
                assembler.python_version = python_version;
                assembler.next_label = 1;
                let mut pipeline = assembler.into_optimization_pipeline();
                let original_arrays = pipeline.legacy_instruction_arrays();
                pipeline.inline_legacy_exit_blocks(&original_arrays, true);
                let first = pipeline.graph.block_at(0);
                assert_eq!(
                    pipeline.graph[first].last_instruction().unwrap().opcode,
                    if fused_null {
                        JUMP_FORWARD
                    } else {
                        RETURN_VALUE
                    },
                );
                if python_version == PythonVersion::Py312 {
                    pipeline.inline_legacy_exit_blocks(&original_arrays, false);
                    assert_eq!(
                        pipeline.graph[first].last_instruction().unwrap().opcode,
                        RETURN_VALUE,
                    );
                }
                pipeline.graph.validate().unwrap();
            }
        }
    }

    #[test]
    fn legacy_nops_respect_original_arrays_across_exception_range_ends() {
        for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
            for (source_boundary, removed_jump) in [
                (false, None),
                (true, None),
                (false, Some(JUMP_FORWARD)),
                (false, Some(POP_JUMP_IF_FALSE)),
            ] {
                let start = Label::from_u32(0);
                let range_end = Label::from_u32(1);
                let handler = Label::from_u32(2);
                let location = SourceLocation::new(2, 2, 0, 1);
                let mut marker = Instruction::new(NOP, Operand::Value(0), location, None);
                marker.fusion_policy.mark_converted_pop_block();
                marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                let mut items = vec![
                    Item::Label(start),
                    Item::Instruction(Instruction::new(
                        LOAD_FAST,
                        Operand::Value(0),
                        location,
                        None,
                    )),
                ];
                if let Some(opcode) = removed_jump {
                    items.push(Item::Instruction(Instruction::new(
                        opcode,
                        Operand::Forward(range_end),
                        location,
                        None,
                    )));
                }
                items.extend([
                    Item::Label(range_end),
                    Item::Instruction(marker),
                    Item::Instruction(Instruction::new(
                        LOAD_CONST,
                        Operand::Value(0),
                        SourceLocation::new(3, 3, 0, 1),
                        None,
                    )),
                    Item::Instruction(Instruction::new(
                        RETURN_VALUE,
                        Operand::Value(0),
                        SourceLocation::new(3, 3, 0, 1),
                        None,
                    )),
                    Item::Label(handler),
                    Item::Instruction(Instruction::new(
                        RERAISE,
                        Operand::Value(0),
                        SourceLocation::NONE,
                        None,
                    )),
                ]);
                let mut assembler = Assembler::from_linear_items(items);
                assembler.python_version = python_version;
                assembler.next_label = 3;
                assembler.add_exception_region(start, range_end, handler, 0, false);
                if source_boundary {
                    assembler.record_exception_owner_block_boundary(range_end);
                }
                let mut pipeline = assembler.into_optimization_pipeline();
                let original_arrays = pipeline.legacy_instruction_arrays();
                if removed_jump.is_some() {
                    let block = pipeline.graph.label_block(start).unwrap();
                    legacy_nop(pipeline.graph.instructions_mut(block).next_back().unwrap());
                }
                pipeline.remove_legacy_nops(&original_arrays, true);
                assert_eq!(
                    pipeline
                        .graph
                        .iter_items()
                        .any(|item| *item == Item::Instruction(marker)),
                    source_boundary || removed_jump.is_some(),
                );
                pipeline.graph.validate().unwrap();
            }
        }
    }

    #[test]
    fn legacy_constant_if_expression_arms_fold_after_exit_copying() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def conditional_true(manager, values, choose, check):\n    with manager:\n        for value in values:\n            assert check(value) if choose(value) else True\n";
        // Exact pinned native code and tables from the independent public reducer.
        let cases: &[(PythonVersion, &[u8], &[u8])] = &[
            (
                PythonVersion::Py312,
                &[
                    151, 0, 124, 0, 53, 0, 1, 0, 124, 1, 68, 0, 93, 21, 0, 0, 125, 4, 2, 0, 124, 2,
                    124, 4, 171, 1, 0, 0, 0, 0, 0, 0, 114, 11, 2, 0, 124, 3, 124, 4, 171, 1, 0, 0,
                    0, 0, 0, 0, 114, 1, 140, 20, 74, 0, 130, 1, 140, 23, 4, 0, 9, 0, 100, 0, 100,
                    0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 49, 0, 115, 1, 119,
                    2, 1, 0, 89, 0, 1, 0, 1, 0, 121, 0, 120, 3, 89, 0, 119, 1,
                ],
                &[131, 22, 40, 3, 154, 4, 40, 3, 168, 5, 49, 7],
            ),
            (
                PythonVersion::Py313,
                &[
                    149, 0, 85, 0, 2, 0, 32, 0, 85, 1, 19, 0, 72, 33, 0, 0, 110, 4, 85, 2, 34, 0,
                    85, 4, 53, 1, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 17, 0, 0, 85, 3,
                    34, 0, 85, 4, 53, 1, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0,
                    77, 31, 0, 0, 23, 0, 101, 1, 77, 35, 0, 0, 11, 0, 32, 0, 83, 0, 83, 0, 83, 0,
                    53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0,
                    100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 103, 0, 61, 3, 31, 0, 102, 1,
                ],
                &[
                    131, 30, 52, 3, 165, 2, 52, 3, 169, 2, 52, 3, 180, 10, 65, 2, 7,
                ],
            ),
        ];
        for &(version, bytecode, table) in cases {
            let module = compile_with_options(
                source,
                "legacy_constant_arms.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.bytecode, bytecode, "{version:?}");
            assert_eq!(function.exception_table, table, "{version:?}");
        }
    }

    #[test]
    fn legacy_with_suppression_jump_keeps_version_specific_owner() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def ordinary_try(manager, work, Error, handle, finish):\n    try:\n        with manager:\n            work()\n    except Error:\n        handle()\n    finish()\n";
        // Pinned CPython 3.12.14 keeps the enclosing handler on the suppression
        // JUMP_BACKWARD. CPython 3.13.15 copies the ownerless outer-try continuation.
        let tables: &[(PythonVersion, &[u8])] = &[
            (
                PythonVersion::Py312,
                &[
                    130, 2, 40, 0, 132, 8, 28, 3, 140, 8, 40, 0, 156, 5, 37, 7, 161, 7, 40, 0, 168,
                    12, 55, 3, 182, 1, 55, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                &[
                    130, 2, 45, 0, 132, 8, 28, 3, 140, 8, 45, 0, 156, 10, 42, 7, 166, 3, 45, 0,
                    170, 3, 45, 0, 173, 13, 61, 3, 188, 1, 61, 3,
                ],
            ),
        ];
        for &(version, expected) in tables {
            let module = compile_with_options(
                source,
                "legacy_with_suppression.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.exception_table, expected, "{version:?}");
        }
    }

    #[test]
    fn python313_final_literal_counts_toward_exit_copy_limit() {
        use crate::{CompileOptions, compile_with_options};

        let source = "if condition:\n    with manager():\n        work()\n'literal 0'\n'literal 1'\n'literal 2'\n";
        let module = compile_with_options(
            source,
            "module_literal_copy_budget.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        // Pinned CPython 3.13.15: three literal NOPs plus LOAD_CONST/RETURN_VALUE
        // exceed the four-slot copy limit, keeping the manager's footer shared.
        assert_eq!(
            module.bytecode,
            [
                149, 0, 92, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 23, 0, 0, 92, 1, 34, 0, 53, 0, 0, 0, 0,
                0, 0, 0, 2, 0, 32, 0, 92, 2, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 83, 0, 83, 0,
                83, 0, 53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 30, 0, 30, 0, 103, 0, 33, 0, 44, 0, 40, 0,
                0, 0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 78, 17, 61, 3,
                31, 0, 102, 1,
            ]
        );
        assert_eq!(
            module.line_table,
            [
                240, 3, 1, 1, 1, 222, 3, 12, 217, 9, 16, 141, 25, 217, 8, 12, 140, 6, 247, 3, 0,
                10, 19, 224, 0, 11, 216, 0, 11, 216, 0, 11, 247, 9, 0, 10, 19, 141, 25, 250,
            ]
        );
        assert_eq!(module.exception_table, [143, 8, 34, 3, 162, 10, 48, 7]);
    }

    #[test]
    fn legacy_return_pop_blocks_count_toward_exit_copy_limit() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def iterator_try_with(items, manager, work, Error):\n    for item in items:\n        try:\n            with manager:\n                work(item)\n            return item\n        except Error:\n            pass\n";
        // Exact pinned native bytes: the return array has five original slots,
        // including POP_BLOCK, so manager suppression cannot copy its four wire ops.
        let cases: &[(PythonVersion, &[u8], &[u8])] = &[
            (
                PythonVersion::Py312,
                &[
                    151, 0, 124, 0, 68, 0, 93, 25, 0, 0, 125, 4, 9, 0, 124, 1, 53, 0, 1, 0, 2, 0,
                    124, 2, 124, 4, 171, 1, 0, 0, 0, 0, 0, 0, 1, 0, 100, 0, 100, 0, 100, 0, 171, 2,
                    0, 0, 0, 0, 0, 0, 1, 0, 124, 4, 99, 2, 1, 0, 83, 0, 4, 0, 121, 0, 35, 0, 49, 0,
                    115, 1, 119, 2, 1, 0, 89, 0, 1, 0, 1, 0, 140, 15, 120, 3, 89, 0, 119, 1, 35, 0,
                    124, 3, 36, 0, 114, 3, 1, 0, 89, 0, 140, 48, 119, 0, 120, 3, 89, 0, 119, 1,
                ],
                &[
                    135, 2, 44, 2, 137, 9, 32, 5, 146, 9, 44, 2, 160, 5, 41, 9, 165, 7, 44, 2, 172,
                    5, 52, 5, 179, 1, 52, 5,
                ],
            ),
            (
                PythonVersion::Py313,
                &[
                    149, 0, 85, 0, 19, 0, 72, 25, 0, 0, 110, 4, 30, 0, 85, 1, 2, 0, 32, 0, 85, 2,
                    34, 0, 85, 4, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 83, 0, 83, 0, 83, 0, 53, 2, 0, 0,
                    0, 0, 0, 0, 32, 0, 85, 4, 115, 2, 32, 0, 36, 0, 11, 0, 32, 0, 103, 0, 33, 0,
                    44, 0, 40, 0, 0, 0, 0, 0, 0, 0, 100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32,
                    0, 78, 21, 61, 3, 31, 0, 102, 1, 33, 0, 85, 3, 7, 0, 97, 4, 0, 0, 32, 0, 31, 0,
                    77, 56, 0, 0, 102, 0, 61, 3, 31, 0, 102, 1,
                ],
                &[
                    135, 2, 50, 2, 137, 9, 33, 5, 146, 9, 50, 2, 161, 10, 47, 9, 171, 7, 50, 2,
                    178, 6, 60, 5, 187, 1, 60, 5,
                ],
            ),
        ];
        for &(version, bytecode, table) in cases {
            let module = compile_with_options(
                source,
                "legacy_return_copy_budget.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.bytecode, bytecode, "{version:?}");
            assert_eq!(function.exception_table, table, "{version:?}");
        }
    }

    #[test]
    fn legacy_discarded_comprehensions_keep_original_restore_slots() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        // Pinned native bytes: 3.12 keeps SWAP until late line propagation;
        // 3.13 counts it before early exit copying, then eliminates it later.
        let cases: &[(PythonVersion, &str, &[u8], &[u8])] = &[
            (
                PythonVersion::Py312,
                "def discarded(items):\n    [item for item in items]\n    return items\n",
                &[
                    151, 0, 124, 0, 68, 0, 143, 1, 99, 2, 103, 0, 99, 2, 93, 4, 0, 0, 125, 1, 124,
                    1, 145, 2, 140, 6, 4, 0, 99, 2, 125, 1, 1, 0, 124, 0, 83, 0, 99, 2, 1, 0, 99,
                    2, 125, 1, 119, 0,
                ],
                &[133, 9, 19, 4],
            ),
            (
                PythonVersion::Py313,
                "async def discarded(items):\n    [item async for item in items]\n",
                &[
                    35, 0, 32, 0, 149, 0, 85, 0, 16, 0, 86, 1, 115, 2, 47, 0, 115, 2, 18, 0, 83, 0,
                    104, 3, 0, 0, 118, 1, 149, 3, 78, 5, 12, 0, 111, 17, 80, 2, 77, 12, 0, 0, 8, 0,
                    78, 7, 10, 0, 79, 5, 115, 2, 32, 0, 115, 2, 110, 1, 102, 0, 32, 0, 110, 1, 103,
                    0, 55, 3, 102, 1,
                ],
                &[
                    130, 5, 33, 1, 135, 2, 25, 4, 137, 4, 23, 6, 141, 1, 21, 10, 142, 3, 23, 6,
                    145, 4, 25, 4, 149, 1, 23, 6, 151, 1, 25, 4, 152, 9, 33, 1,
                ],
            ),
        ];
        for &(version, source, bytecode, table) in cases {
            let module = compile_with_options(
                source,
                "legacy_comprehension_discard.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.bytecode, bytecode, "{version:?}");
            assert_eq!(function.exception_table, table, "{version:?}");
        }
    }

    #[test]
    fn python_312_authored_exits_keep_the_enclosing_try_owner() {
        use crate::{CompileOptions, compile_with_options};

        // Pinned native tables for an in-try break and an empty nested try.
        let cases: &[(&str, &[u8])] = &[
            (
                "try:\n    for item in items:\n        consume(item)\n        break\nexcept Error:\n    recover()\nfinish()\n",
                &[130, 16, 26, 0, 154, 12, 41, 3, 168, 1, 41, 3],
            ),
            (
                "try:\n    try:\n        pass\n    except (TypeError, RuntimeError):\n        pass\nexcept ValueError:\n    pass\nfinish()\n",
                &[
                    131, 1, 25, 0, 140, 7, 22, 3, 147, 2, 25, 0, 149, 1, 22, 3, 150, 3, 25, 0, 153,
                    5, 33, 3, 160, 1, 33, 3,
                ],
            ),
        ];
        for &(source, table) in cases {
            let module = compile_with_options(
                source,
                "legacy_enclosing_owner.py",
                &CompileOptions {
                    python_version: PythonVersion::Py312,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            assert_eq!(module.exception_table, table, "{source}");
        }
    }

    #[test]
    fn python_313_folded_guard_keeps_original_finally_predecessor() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def folded_guard(flag):\n    while flag:\n        try:\n            while ():\n                if 3:\n                    break\n        finally:\n            return\n";
        let module = compile_with_options(
            source,
            "legacy_folded_finally_owner.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) => Some(code),
                _ => None,
            })
            .unwrap();
        // Pinned native 3.13 bytes: the inner guard's original false edge keeps
        // the shared POP_BLOCK unlocated until the constant condition folds.
        assert_eq!(
            function.bytecode,
            [
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 3, 0, 0, 30, 0, 30, 0, 103, 0, 103, 0,
                33, 0, 32, 0, 31, 0, 103, 0, 61, 3, 31, 0, 102, 1,
            ]
        );
        assert_eq!(function.exception_table, [137, 1, 12, 0, 140, 2, 16, 3]);
    }

    #[test]
    fn legacy_exception_joins_match_native_blocks_and_locations() {
        use crate::{CompileOptions, compile_with_options};

        // Pinned CPython 3.12.14/3.13.15. Shared except-star joins stay
        // unlocated, SETUP bodies stop exit copying, and authored passes survive.
        type NativeCase<'a> = (PythonVersion, &'a str, &'a [u8], &'a [u8], &'a [u8]);
        let cases: &[NativeCase<'_>] = &[
            (
                PythonVersion::Py312,
                "try:\n    value = 1\nexcept* Error:\n    value = 2\nfinish()\n",
                &[
                    151, 0, 9, 0, 100, 0, 90, 0, 2, 0, 101, 2, 171, 0, 0, 0, 0, 0, 0, 0, 1, 0, 121,
                    2, 35, 0, 103, 0, 120, 2, 101, 1, 37, 0, 120, 1, 129, 8, 1, 0, 100, 1, 90, 0,
                    110, 3, 145, 3, 1, 0, 110, 2, 110, 1, 1, 0, 145, 1, 174, 1, 120, 1, 128, 3, 1,
                    0, 89, 0, 140, 31, 99, 2, 89, 0, 119, 0, 120, 3, 89, 0, 119, 1,
                ],
                &[130, 2, 12, 0, 140, 8, 38, 3, 148, 2, 23, 9, 150, 11, 38, 3],
                &[
                    240, 3, 1, 1, 1, 240, 2, 3, 1, 14, 216, 12, 13, 128, 69, 241, 6, 0, 1, 7, 133,
                    8, 248, 241, 5, 1, 1, 14, 136, 5, 243, 0, 1, 1, 14, 216, 12, 13, 129, 69, 250,
                    128, 69, 240, 3, 1, 1, 14, 255, 252,
                ],
            ),
            (
                PythonVersion::Py312,
                "try:\n    pass\nexcept Error:\n    pass\n\ntry:\n    pass\nexcept OtherError:\n    pass\n",
                &[
                    151, 0, 9, 0, 9, 0, 9, 0, 121, 0, 35, 0, 101, 0, 36, 0, 114, 3, 1, 0, 89, 0,
                    140, 9, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 101, 1, 36, 0, 114, 3, 1, 0, 89,
                    0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1,
                ],
                &[133, 5, 13, 3, 140, 1, 13, 3, 144, 5, 24, 3, 151, 1, 24, 3],
                &[
                    240, 3, 1, 1, 1, 240, 2, 3, 1, 9, 216, 4, 8, 240, 8, 3, 1, 9, 216, 4, 8, 248,
                    240, 9, 0, 8, 13, 242, 0, 1, 1, 9, 217, 4, 8, 240, 3, 1, 1, 9, 251, 240, 10, 0,
                    8, 18, 242, 0, 1, 1, 9, 217, 4, 8, 240, 3, 1, 1, 9, 250,
                ],
            ),
            (
                PythonVersion::Py313,
                "try:\n    value = 1\nexcept* Error:\n    value = 2\nfinish()\n",
                &[
                    149, 0, 30, 0, 83, 0, 114, 0, 92, 2, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0,
                    103, 2, 33, 0, 47, 0, 61, 2, 92, 1, 6, 0, 61, 1, 98, 9, 0, 0, 32, 0, 83, 1,
                    114, 0, 80, 1, 79, 6, 80, 3, 32, 0, 80, 1, 79, 2, 32, 0, 80, 1, 56, 1, 61, 1,
                    99, 3, 0, 0, 32, 0, 31, 0, 78, 34, 115, 2, 31, 0, 102, 0, 61, 3, 31, 0, 102, 1,
                ],
                &[130, 2, 12, 0, 140, 9, 41, 3, 149, 2, 25, 9, 151, 13, 41, 3],
                &[
                    240, 3, 1, 1, 1, 240, 2, 3, 1, 14, 216, 12, 13, 128, 69, 241, 6, 0, 1, 7, 133,
                    8, 248, 241, 5, 1, 1, 14, 136, 5, 244, 0, 1, 1, 14, 216, 12, 13, 130, 69, 251,
                    241, 3, 1, 1, 14, 255, 252,
                ],
            ),
            (
                PythonVersion::Py313,
                "try:\n    pass\nexcept Error:\n    pass\n\ntry:\n    pass\nexcept OtherError:\n    pass\n",
                &[
                    149, 0, 30, 0, 30, 0, 30, 0, 103, 0, 33, 0, 92, 0, 7, 0, 97, 3, 0, 0, 32, 0,
                    31, 0, 78, 10, 102, 0, 61, 3, 31, 0, 102, 1, 33, 0, 92, 1, 7, 0, 97, 3, 0, 0,
                    32, 0, 31, 0, 103, 0, 102, 0, 61, 3, 31, 0, 102, 1,
                ],
                &[133, 6, 14, 3, 141, 1, 14, 3, 145, 6, 26, 3, 153, 1, 26, 3],
                &[
                    240, 3, 1, 1, 1, 240, 2, 3, 1, 9, 216, 4, 8, 240, 8, 3, 1, 9, 216, 4, 8, 248,
                    240, 9, 0, 8, 13, 243, 0, 1, 1, 9, 217, 4, 8, 240, 3, 1, 1, 9, 251, 240, 10, 0,
                    8, 18, 243, 0, 1, 1, 9, 217, 4, 8, 240, 3, 1, 1, 9, 250,
                ],
            ),
            (
                PythonVersion::Py313,
                "try:\n    value = 1\n    pass\nexcept Error:\n    pass\nfinish()\n",
                &[
                    149, 0, 30, 0, 83, 0, 114, 0, 30, 0, 92, 2, 34, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32,
                    0, 103, 1, 33, 0, 92, 1, 7, 0, 97, 3, 0, 0, 32, 0, 31, 0, 78, 16, 102, 0, 61,
                    3, 31, 0, 102, 1,
                ],
                &[130, 2, 13, 0, 141, 6, 22, 3, 149, 1, 22, 3],
                &[
                    240, 3, 1, 1, 1, 240, 2, 4, 1, 9, 216, 12, 13, 128, 69, 216, 4, 8, 241, 6, 0,
                    1, 7, 133, 8, 248, 240, 5, 0, 8, 13, 243, 0, 1, 1, 9, 217, 4, 8, 240, 3, 1, 1,
                    9, 250,
                ],
            ),
        ];
        for &(version, source, bytecode, exceptions, lines) in cases {
            let module = compile_with_options(
                source,
                "legacy_exception_joins.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            assert_eq!(module.bytecode, bytecode, "{version:?}: {source}");
            assert_eq!(module.exception_table, exceptions, "{version:?}: {source}");
            assert_eq!(module.line_table, lines, "{version:?}: {source}");
        }
    }

    #[test]
    fn python_312_noop_finally_retains_ownerless_original_pop_block() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        // Pinned CPython 3.12.14: the else Pass's location moves to the
        // copied original POP_BLOCK, which has no exception handler.
        let module = compile_with_options(
            "def f():\n    try: pass\n    except ZeroDivisionError: pass\n    else: pass\n    finally: pass\n",
            "noop_finally_owner.py",
            &CompileOptions {
                python_version: PythonVersion::Py312,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            function.bytecode,
            [
                151, 0, 9, 0, 9, 0, 121, 0, 35, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 114, 3,
                1, 0, 89, 0, 121, 0, 119, 0, 120, 3, 89, 0, 119, 1, 35, 0, 119, 0, 120, 3, 89, 0,
                119, 1
            ]
        );
        assert_eq!(
            function.exception_table,
            [
                132, 9, 16, 3, 141, 1, 19, 0, 143, 1, 16, 3, 144, 3, 19, 0, 147, 2, 21, 3
            ]
        );
        assert_eq!(
            function.line_table,
            [
                128, 0, 216, 9, 13, 224, 10, 14, 216, 13, 17, 248, 244, 5, 0, 12, 29, 210, 4, 34,
                152, 100, 224, 13, 17, 240, 5, 0, 5, 35, 251, 224, 13, 17, 250
            ]
        );
    }

    #[test]
    fn legacy_handler_while_keeps_shared_cleanup_unlocated() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        // Pinned CPython 3.12.14 and 3.13.15: both the entry test and the
        // repeated loop test reach the handler cleanup, so neither donates its span.
        let source = "def f(flag):\n    try:\n        work()\n    except Exception as error:\n        while check():\n            work()\n    finish()\n";
        for (version, lines) in [
            (
                PythonVersion::Py312,
                &[
                    128, 0, 240, 2, 4, 5, 19, 220, 8, 12, 140, 6, 244, 8, 0, 5, 11, 133, 72, 248,
                    244, 7, 0, 12, 21, 242, 0, 2, 5, 19, 220, 14, 19, 140, 103, 220, 12, 16, 140,
                    70, 244, 3, 0, 15, 20, 141, 103, 251, 228, 4, 10, 133, 72, 251, 240, 7, 2, 5,
                    19, 250,
                ][..],
            ),
            (
                PythonVersion::Py313,
                &[
                    128, 0, 240, 2, 4, 5, 19, 220, 8, 12, 140, 6, 244, 8, 0, 5, 11, 133, 72, 248,
                    244, 7, 0, 12, 21, 243, 0, 2, 5, 19, 220, 14, 19, 143, 103, 137, 103, 220, 12,
                    16, 140, 70, 244, 3, 0, 15, 20, 143, 103, 139, 103, 255, 248, 240, 3, 2, 5, 19,
                    250,
                ][..],
            ),
        ] {
            let module = compile_with_options(
                source,
                "handler_loop.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "f" => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.line_table, lines, "{version:?}");
        }
    }
}

#[cfg(test)]
mod finally_capture_seed_tests {
    use super::*;
    use crate::assembler::Assembler;
    use crate::assembler::exception_owners::OriginalNormalFinallyArray;
    use crate::target::opcodes::{POP_JUMP_IF_FALSE, RETURN_CONST};

    #[test]
    fn finalizer_prefix_and_incoming_seeds_are_atomic_and_preserve_equality() {
        for native_entry in [12, 256] {
            for invalid in 0..4 {
                let start = Label::from_u32(0);
                let entry = Label::from_u32(1);
                let end = Label::from_u32(2);
                let incoming = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(entry),
                    SourceLocation::NONE,
                    Some(0),
                );
                let condition = Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(end),
                    SourceLocation::NONE,
                    Some(0),
                );
                let mut assembler = Assembler::from_linear_items(vec![
                    Item::Label(start),
                    Item::Instruction(incoming),
                    Item::Label(entry),
                    Item::Instruction(condition),
                    Item::Label(end),
                    Item::Instruction(Instruction::new(
                        RETURN_CONST,
                        Operand::Value(0),
                        SourceLocation::NONE,
                        Some(0),
                    )),
                ]);
                assembler.python_version = PythonVersion::Py312;
                assembler.next_label = 3;
                assembler.bind_legacy_native_label(start, 1);
                assembler.bind_legacy_native_label(entry, native_entry);
                assembler.bind_legacy_native_label(end, native_entry + 1);
                assembler.certify_legacy_native_labels();
                let mut captured_condition = condition;
                if invalid == 1 {
                    captured_condition.retarget(Operand::Forward(Label::from_u32(900)));
                }
                let mut captured_incoming = incoming;
                if invalid == 2 {
                    captured_incoming.set_operand(Operand::Value(0));
                }
                assembler
                    .exception_owner_provenance
                    .original_normal_finally_arrays
                    .push(OriginalNormalFinallyArray {
                        entry,
                        prefix: vec![Item::Label(entry), Item::Instruction(captured_condition)],
                        incoming: vec![captured_incoming],
                        retained_setup: None,
                        borrow_join: None,
                    });
                if invalid == 3 {
                    let mut malformed = condition;
                    malformed.set_operand(Operand::Value(0));
                    assembler
                        .exception_owner_provenance
                        .original_primitive_condition_boundaries
                        .push(malformed);
                }
                let mut pipeline = assembler.into_optimization_pipeline();
                let arrays = pipeline.legacy_instruction_arrays();
                pipeline.initialize_legacy_native_jump_seeds(&arrays);
                assert_eq!(
                    pipeline
                        .legacy_native_labels
                        .as_ref()
                        .unwrap()
                        .is_certified(),
                    invalid == 0
                );
                let live = pipeline
                    .graph
                    .iter_items()
                    .filter_map(|item| {
                        if let Item::Instruction(instruction) = item {
                            Some(*instruction)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                let expected = (invalid == 0).then_some(u8::from(native_entry > 255));
                assert_eq!(live[0].legacy_initial_jump_width(), expected);
                assert_eq!(live[1].legacy_initial_jump_width(), expected);
                let origin = &pipeline
                    .exception_owner_provenance
                    .original_normal_finally_arrays[0];
                assert_eq!(origin.incoming[0].legacy_initial_jump_width(), expected);
                let Item::Instruction(captured) = origin.prefix[1] else {
                    panic!()
                };
                assert_eq!(captured.legacy_initial_jump_width(), expected);
                if invalid == 0 {
                    assert_eq!(origin.incoming[0], live[0]);
                    assert_eq!(captured, live[1]);
                }
                // A formerly seeded jump can become a non-jump before cancellation.
                pipeline
                    .exception_owner_provenance
                    .visit_legacy_seed_captures_mut(|instruction| {
                        instruction.set_operand(Operand::Forward(entry));
                        instruction.set_legacy_initial_jump_argument(native_entry);
                        instruction.opcode = NOP;
                        instruction.set_operand(Operand::Value(0));
                        assert!(instruction.legacy_initial_jump_width().is_some());
                    });
                pipeline
                    .exception_owner_provenance
                    .visit_legacy_seed_captures_mut(
                        Instruction::clear_legacy_initial_jump_argument,
                    );
                pipeline
                    .exception_owner_provenance
                    .visit_legacy_seed_captures_mut(|instruction| {
                        assert_eq!(instruction.legacy_initial_jump_width(), None);
                    });
            }
        }
    }
}

#[cfg(test)]
mod protected_repeated_backedge_tests {
    use super::*;
    use crate::assembler::Assembler;
    use crate::target::opcodes::{POP_JUMP_IF_FALSE, RERAISE};

    #[test]
    fn generator_backedge_requires_original_wrapper_and_prologue() {
        for case in 0..9 {
            let label = Label::from_u32;
            let (
                entry,
                body,
                handler,
                failed,
                cleanup,
                loop_head,
                second,
                backedge,
                exit,
                handler_success,
                wrapper,
                stop_handler,
            ) = (
                label(0),
                label(1),
                label(2),
                label(3),
                label(4),
                label(5),
                label(6),
                label(7),
                label(8),
                label(9),
                label(10),
                label(11),
            );
            let normal = |opcode, argument| {
                Instruction::new(
                    opcode,
                    Operand::Value(argument),
                    SourceLocation::NONE,
                    Some(0),
                )
            };
            let branch = |target| {
                Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(target),
                    SourceLocation::NONE,
                    Some(0),
                )
            };
            let jump = |target| {
                Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(target),
                    SourceLocation::NONE,
                    Some(0),
                )
            };
            let mut items = Vec::new();
            if case == 7 {
                items.extend([
                    Item::Instruction(normal(COPY_FREE_VARS, 1)),
                    Item::Instruction(normal(MAKE_CELL, 0)),
                ]);
            }
            items.extend([
                Item::Instruction(normal(if case == 3 { NOP } else { RETURN_GENERATOR }, 0)),
                Item::Instruction(normal(POP_TOP, 0)),
                Item::Label(wrapper),
                Item::Instruction(normal(if case == 4 { NOP } else { RESUME }, 0)),
                Item::Label(entry),
                Item::Instruction(normal(NOP, 0)),
                Item::Label(body),
                Item::Instruction(normal(NOP, 0)),
                Item::Instruction(jump(loop_head)),
                Item::Label(handler),
                Item::Instruction(branch(failed)),
                Item::Label(handler_success),
                Item::Instruction(jump(loop_head)),
                Item::Label(failed),
                Item::Instruction(normal(RERAISE, 0)),
                Item::Label(cleanup),
                Item::Instruction(normal(RERAISE, 0)),
                Item::Label(loop_head),
                Item::Instruction(branch(second)),
                Item::Label(second),
                Item::Instruction(Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Backward(loop_head),
                    SourceLocation::NONE,
                    Some(0),
                )),
                Item::Label(backedge),
                Item::Instruction(Instruction::new(
                    JUMP_BACKWARD,
                    Operand::Backward(loop_head),
                    SourceLocation::NONE,
                    Some(0),
                )),
                Item::Label(exit),
                Item::Instruction(if case == 5 {
                    jump(stop_handler)
                } else {
                    normal(RETURN_VALUE, 0)
                }),
                Item::Label(stop_handler),
                Item::Instruction(normal(CALL_INTRINSIC_1, if case == 2 { 5 } else { 3 })),
                Item::Instruction(normal(RERAISE, 1)),
            ]);
            let mut assembler = Assembler::from_linear_items(items);
            assembler.python_version = PythonVersion::Py312;
            assembler.next_label = 12;
            assembler.add_exception_region(body, handler, handler, 0, false);
            assembler.add_exception_region(handler, failed, cleanup, 1, true);
            assembler.add_stopiteration_exception_region_with_depth(
                if case == 8 { entry } else { wrapper },
                stop_handler,
                stop_handler,
                0,
            );
            assembler.record_exception_setup(body, false);
            assembler.record_legacy_native_setup_target(body, handler);
            assembler.record_exception_setup(handler, false);
            assembler.record_legacy_native_setup_target(handler, cleanup);
            assembler.record_exception_setup(wrapper, false);
            if case != 1 {
                assembler.record_legacy_native_setup_target(wrapper, stop_handler);
            }
            if case == 6 {
                assembler.record_legacy_native_setup_target(wrapper, stop_handler);
            }
            let pipeline = assembler.into_optimization_pipeline();
            let arrays = pipeline.legacy_instruction_arrays();
            let result = pipeline.legacy_repeated_backedges_with_setups(&arrays);
            if case == 0 || case == 7 {
                assert_eq!(
                    result.unwrap()[0].0,
                    pipeline.graph.label_block(backedge).unwrap(),
                    "case {case}"
                );
            } else {
                assert!(result.is_none(), "case {case}");
            }
        }
    }

    #[test]
    fn repeated_visits_follow_body_and_handler_setup_edges() {
        for case in 0..5 {
            let entry = Label::from_u32(0);
            let body = Label::from_u32(1);
            let handler = Label::from_u32(2);
            let failed = Label::from_u32(3);
            let cleanup = Label::from_u32(4);
            let loop_head = Label::from_u32(5);
            let second = Label::from_u32(6);
            let backedge = Label::from_u32(7);
            let exit = Label::from_u32(8);
            let handler_success = Label::from_u32(9);
            let normal =
                |opcode| Instruction::new(opcode, Operand::Value(0), SourceLocation::NONE, Some(0));
            let branch = |target| {
                Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(target),
                    SourceLocation::NONE,
                    Some(0),
                )
            };
            let jump = |target| {
                Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(target),
                    SourceLocation::NONE,
                    Some(0),
                )
            };
            let mut assembler = Assembler::from_linear_items(vec![
                Item::Label(entry),
                Item::Instruction(normal(NOP)),
                Item::Label(body),
                Item::Instruction(normal(NOP)),
                Item::Instruction(jump(loop_head)),
                Item::Label(handler),
                Item::Instruction(branch(failed)),
                Item::Label(handler_success),
                Item::Instruction(jump(loop_head)),
                Item::Label(failed),
                Item::Instruction(normal(RERAISE)),
                Item::Label(cleanup),
                Item::Instruction(normal(RERAISE)),
                Item::Label(loop_head),
                Item::Instruction(branch(if case == 1 { exit } else { second })),
                Item::Label(second),
                Item::Instruction(Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Backward(loop_head),
                    SourceLocation::NONE,
                    Some(0),
                )),
                Item::Label(backedge),
                Item::Instruction(Instruction::new(
                    JUMP_BACKWARD,
                    Operand::Backward(loop_head),
                    SourceLocation::NONE,
                    Some(0),
                )),
                Item::Label(exit),
                Item::Instruction(normal(RETURN_VALUE)),
            ]);
            assembler.python_version = PythonVersion::Py312;
            assembler.next_label = 10;
            assembler.add_exception_region(body, handler, handler, 0, false);
            assembler.add_exception_region(handler, failed, cleanup, 1, true);
            assembler.record_exception_setup(body, false);
            assembler.record_legacy_native_setup_target(body, handler);
            assembler.record_exception_setup(handler, false);
            if case != 2 {
                assembler.record_legacy_native_setup_target(handler, cleanup);
            }
            if case == 3 {
                assembler.record_legacy_native_setup_target(handler, cleanup);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            let arrays = pipeline.legacy_instruction_arrays();
            if case == 4 {
                let block = pipeline.graph.label_block(entry).unwrap();
                let instruction = pipeline.graph[block]
                    .items()
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))
                    .unwrap();
                pipeline.graph.remove_item_at(block, instruction);
            }
            let found = pipeline.legacy_repeated_backedges_with_setups(&arrays);
            match case {
                0 => assert_eq!(
                    found.unwrap()[0].0,
                    pipeline.graph.label_block(backedge).unwrap()
                ),
                1 => assert!(found.unwrap().is_empty()),
                _ => assert!(found.is_none()),
            }
        }
    }

    #[test]
    fn protected_prefix_requires_authored_targets_and_no_reentry() {
        for case in 0..7 {
            let start = Label::from_u32(0);
            let second = Label::from_u32(1);
            let backedge = Label::from_u32(2);
            let end = Label::from_u32(3);
            let handler = Label::from_u32(4);
            let branch =
                |target| Instruction::new(POP_JUMP_IF_FALSE, target, SourceLocation::NONE, Some(0));
            let jump = |target| {
                Instruction::new(
                    JUMP_BACKWARD,
                    Operand::Backward(target),
                    SourceLocation::NONE,
                    Some(0),
                )
            };
            let mut assembler = Assembler::from_linear_items(vec![
                Item::Label(start),
                Item::Instruction(branch(Operand::Forward(second))),
                Item::Label(second),
                Item::Instruction(branch(Operand::Backward(start))),
                Item::Label(backedge),
                Item::Instruction(jump(start)),
                Item::Label(end),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    Some(0),
                )),
                Item::Label(handler),
                Item::Instruction(if case == 5 {
                    jump(start)
                } else {
                    Instruction::new(RERAISE, Operand::Value(0), SourceLocation::NONE, Some(0))
                }),
            ]);
            assembler.python_version = PythonVersion::Py312;
            assembler.next_label = 5;
            assembler.add_exception_region(start, end, handler, 0, false);
            assembler.record_exception_setup(start, false);
            if case != 1 {
                let target = match case {
                    2 => Label::from_u32(900),
                    4 => second,
                    _ => handler,
                };
                assembler.record_legacy_native_setup_target(start, target);
            }
            if case == 3 {
                assembler.record_legacy_native_setup_target(start, handler);
            }
            if case == 6 {
                assembler.record_exception_setup(start, false);
            }
            let pipeline = assembler.into_optimization_pipeline();
            let arrays = pipeline.legacy_instruction_arrays();
            let found = pipeline.legacy_repeated_backedges(&arrays);
            assert_eq!(found.len(), usize::from(case == 0), "case {case}");
            if case == 0 {
                assert_eq!(found[0].0, pipeline.graph.label_block(backedge).unwrap());
            }
        }
    }
}
