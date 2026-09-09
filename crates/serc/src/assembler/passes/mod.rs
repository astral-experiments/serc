#[cfg(any(test, debug_assertions))]
use rustc_hash::FxHashMap;

use super::block_graph::BlockGraph;
use super::layout::{backward_argument, label_position};
use super::{
    AssembledCode, Assembler, ConstantTable, Item, Label, LayoutAssembler, Operand,
    OptimizationPipeline, UnoptimizedCode, extended_arg_count,
};
#[cfg(any(test, debug_assertions))]
use super::{AssemblerStage, ExceptionRegion};
use crate::CompileError;

mod blocks;
mod constants;
mod control_flow;
mod linear;
mod locals;
mod swaps;

#[cfg(any(test, debug_assertions))]
fn validate_structure(
    items: &[Item],
    exception_regions: &[ExceptionRegion],
    reachable: Option<&[bool]>,
    stage: AssemblerStage,
) -> Result<(), CompileError> {
    let invariant_error = |message: String| {
        CompileError::Internal(format!(
            "assembler invariant failed after {}: {message}",
            stage.name()
        ))
    };

    let mut label_positions = FxHashMap::default();
    for (index, item) in items.iter().enumerate() {
        let Item::Label(label) = item else {
            continue;
        };
        if label_positions.insert(*label, index).is_some() {
            return Err(invariant_error(format!(
                "label {} is bound more than once",
                label.0
            )));
        }
    }

    for (index, item) in items.iter().enumerate() {
        let Item::Instruction(instruction) = item else {
            continue;
        };
        let (target, is_forward) = match instruction.operand() {
            Operand::Value(_) => continue,
            Operand::Deferred(id) => {
                return Err(invariant_error(format!(
                    "instruction {index} has unresolved operand patch {}",
                    id.0
                )));
            }
            Operand::Forward(target) => (target, true),
            Operand::Backward(target) => (target, false),
        };
        if !instruction.opcode.has_jump() {
            return Err(invariant_error(format!(
                "instruction {index} uses a label operand with non-jump opcode {}",
                instruction.opcode.code()
            )));
        }
        let Some(target_index) = label_positions.get(&target).copied() else {
            return Err(invariant_error(format!(
                "instruction {index} targets unbound label {}",
                target.0
            )));
        };
        if is_forward && target_index <= index {
            return Err(invariant_error(format!(
                "instruction {index} has a forward operand to label {} at {target_index}",
                target.0
            )));
        }
        if !is_forward && target_index >= index {
            return Err(invariant_error(format!(
                "instruction {index} has a backward operand to label {} at {target_index}",
                target.0
            )));
        }
    }

    for (index, region) in exception_regions.iter().enumerate() {
        let position = |label: Label, role: &str| {
            label_positions.get(&label).copied().ok_or_else(|| {
                invariant_error(format!(
                    "exception region {index} has an unbound {role} label {}",
                    label.0
                ))
            })
        };
        let start = position(region.start, "start")?;
        let end = position(region.end, "end")?;
        position(region.target, "target")?;
        if stage.requires_ordered_exception_regions() && start > end {
            return Err(invariant_error(format!(
                "exception region {index} starts at {start} after it ends at {end}"
            )));
        }
    }

    if let Some(reachable) = reachable {
        if let Some(index) = items.iter().enumerate().find_map(|(index, item)| {
            matches!(item, Item::Instruction(_))
                .then_some(index)
                .filter(|index| !reachable[*index])
        }) {
            return Err(invariant_error(format!(
                "instruction {index} remains unreachable"
            )));
        }
    }

    Ok(())
}

#[cfg(any(test, debug_assertions))]
fn invariant_error(stage: AssemblerStage, message: &str) -> CompileError {
    CompileError::Internal(format!(
        "assembler invariant failed after {}: {message}",
        stage.name()
    ))
}

#[cfg(test)]
impl Assembler {
    pub(in crate::assembler) fn validate_structure(
        &self,
        stage: AssemblerStage,
    ) -> Result<(), CompileError> {
        let items = self.linear_items();
        let reachable = stage.requires_full_reachability().then(|| {
            BlockGraph::from_items(items.to_vec(), &self.preserved_block_boundaries)
                .item_reachability(&self.exception_regions)
        });
        validate_structure(items, &self.exception_regions, reachable.as_deref(), stage)
    }
}

#[cfg(any(test, debug_assertions))]
impl OptimizationPipeline {
    fn validate_structure(&self, stage: AssemblerStage) -> Result<(), CompileError> {
        self.graph
            .validate()
            .map_err(|message| invariant_error(stage, &message))?;
        let items = self.graph.iter_items().copied().collect::<Vec<_>>();
        let reachable = stage
            .requires_full_reachability()
            .then(|| self.reachable_items());
        validate_structure(&items, &self.exception_regions, reachable.as_deref(), stage)
    }
}

#[cfg(any(test, debug_assertions))]
impl LayoutAssembler {
    fn validate_structure(&self, stage: AssemblerStage) -> Result<(), CompileError> {
        validate_structure(&self.items, &self.exception_regions, None, stage)
    }
}

impl Assembler {
    #[cfg(test)]
    pub(crate) fn finish(self) -> Result<Vec<u8>, CompileError> {
        self.finish_code(1, 0, 0)
            .map(|assembled| assembled.bytecode)
    }

    #[cfg(test)]
    pub(crate) fn finish_code(
        self,
        first_line_number: u32,
        local_count: usize,
        parameter_count: usize,
    ) -> Result<AssembledCode, CompileError> {
        self.finish_code_with_constants(first_line_number, local_count, parameter_count, None)
    }

    pub(crate) fn finish_code_with_constants(
        mut self,
        first_line_number: u32,
        local_count: usize,
        parameter_count: usize,
        constants: Option<&mut dyn ConstantTable>,
    ) -> Result<AssembledCode, CompileError> {
        self.resolve_operand_patches()?;
        if self.has_only_linear_assertion_side_exits() {
            return self
                .optimize_linear_assertion_side_exits(local_count, parameter_count)?
                .finish(first_line_number);
        }
        self.into_optimization_pipeline()
            .optimize(local_count, parameter_count, constants)?
            .finish(first_line_number)
    }

    pub(in crate::assembler) fn into_optimization_pipeline(mut self) -> OptimizationPipeline {
        let (graph, initial_reachable) = match self.code {
            UnoptimizedCode::Linear(items) => (
                BlockGraph::from_items(items, &rustc_hash::FxHashSet::default()),
                None,
            ),
            UnoptimizedCode::Prepared {
                mut graph,
                mut reachable,
            } => {
                graph.remove_initial_preserved_boundaries(Some(&mut reachable));
                (graph, Some(reachable))
            }
        };
        if !self.exception_regions.is_empty() {
            // Original setup order expresses nesting even when copied cleanup code makes an
            // inner range longer than its parent, or exclusions split the parent into fragments.
            let label_positions = graph
                .iter_items()
                .enumerate()
                .filter_map(|(index, item)| match item {
                    Item::Label(label) => Some((*label, index)),
                    Item::Instruction(_) => None,
                })
                .collect::<rustc_hash::FxHashMap<_, _>>();
            for region in &mut self.exception_regions {
                if let super::ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
                    region.priority = super::ExceptionRegionPriority::Resolved {
                        start: label_positions[&start],
                        end: label_positions[&end],
                    };
                }
            }
        }
        for item in graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
            {
                self.exception_owner_provenance
                    .block_boundaries
                    .insert(label);
            }
        }
        OptimizationPipeline {
            graph,
            discarded_fold_admissions: self.discarded_fold_admissions,
            initial_reachable,
            next_label: self.next_label,
            exception_regions: self.exception_regions,
            exception_owner_provenance: self.exception_owner_provenance,
            preserved_block_boundaries: self.preserved_block_boundaries,
            borrow_unreachable_blocks: self.borrow_unreachable_blocks,
            borrow_loop_exit_blocks: self.borrow_loop_exit_blocks,
            borrow_block_aliases: self.borrow_block_aliases,
            conditional_threading_targets: self.conditional_threading_targets,
            original_nop_targets: self.original_nop_targets,
            original_unlocated_cleanup_targets: self.original_unlocated_cleanup_targets,
            original_unlocated_discard_targets: self.original_unlocated_discard_targets,
            original_jump_head_targets: rustc_hash::FxHashSet::default(),
            deferred_shared_jump_blocks: self.deferred_shared_jump_blocks,
            shared_jump_copy_location_blocks: self.shared_jump_copy_location_blocks,
            folded_condition_exit_targets: self.folded_condition_exit_targets,
            has_assertion_side_exit_candidate: self.has_assertion_side_exit_candidate,
            has_synthesized_percent_target_locations: self.has_synthesized_percent_target_locations,
            has_next_location: self.has_next_location,
        }
    }
}

impl OptimizationPipeline {
    fn label(&mut self) -> Label {
        let label = Label(self.next_label);
        self.next_label += 1;
        label
    }

    fn optimize(
        mut self,
        local_count: usize,
        parameter_count: usize,
        constants: Option<&mut dyn ConstantTable>,
    ) -> Result<LayoutAssembler, CompileError> {
        self.record_original_jump_heads();
        self.prepare_original_conditional_jump_targets();
        let assertion_side_exits = self.has_only_assertion_side_exits();
        let mut removed_max_depth = if assertion_side_exits {
            None
        } else {
            self.remove_unreachable_instructions()
        };
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveUnreachableInitial)?;
        let original_location_absence = self.capture_original_location_absence();
        self.resolve_synthesized_conditional_target_locations();
        let synthesized_fallthrough_locations = self.defer_synthesized_fallthrough_locations();
        self.optimize_boolean_conversions();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeBooleanConversions)?;
        let threaded_jumps = !assertion_side_exits && self.thread_forward_jumps();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::ThreadForwardJumps)?;
        if threaded_jumps {
            if let Some(depth) = self.remove_unreachable_instructions() {
                removed_max_depth =
                    Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
            }
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveUnreachableAfterThreading)?;
        if !assertion_side_exits {
            self.remove_redundant_forward_jumps(false);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantForwardJumpsEarly)?;
        self.optimize_redundant_store_fast();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeRedundantStoreFast)?;
        let swap_blocks = self.optimize_swap_runs();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeSwapRuns)?;
        self.apply_static_swaps(swap_blocks);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::ApplyStaticSwaps)?;
        let copied_entry_labels = if assertion_side_exits {
            self.propagate_assertion_exit_locations();
            Vec::new()
        } else {
            self.duplicate_exit_blocks(&original_location_absence)
        };
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::DuplicateExitBlocks)?;
        // CPython removes unreachable blocks after its per-block optimizations, including
        // small-exit copying followed by conditional threading, before checking local loads.
        // A terminating match arm can leave its original case jump newly unreachable here.
        if self.thread_original_conditional_jump_targets()
            && let Some(depth) = self.remove_unreachable_instructions()
        {
            removed_max_depth = Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
        }
        if let Some(constants) = constants {
            self.fold_copied_constants(constants, &copied_entry_labels)?;
        }
        if let Some(depth) = self.remove_redundant_copy_pop_pairs() {
            removed_max_depth = Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
        }
        if assertion_side_exits {
            self.add_checks_for_assertion_side_exits(local_count, parameter_count);
        } else {
            self.add_checks_for_uninitialized_loads(local_count, parameter_count);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::AddChecksForUninitializedLoads)?;
        self.fuse_superinstructions();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::FuseSuperinstructions)?;
        if !assertion_side_exits {
            self.push_cold_blocks_to_end();
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::PushColdBlocksToEnd)?;
        if !assertion_side_exits {
            self.remove_redundant_checked_loads();
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantCheckedLoads)?;
        self.propagate_locations_within_blocks();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::PropagateLocationsWithinBlocks)?;
        self.remove_redundant_swaps_before_pops();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantSwapsBeforePops)?;
        self.remove_redundant_nops();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantNops)?;
        if !assertion_side_exits {
            self.remove_redundant_forward_jumps(true);
            // Manager exits can expose a redundant POP_BLOCK after their jump disappears.
            self.remove_redundant_with_exit_nops();
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantForwardJumpsLate)?;
        let features = self.graph.features();
        let mut items = self.graph.into_items();
        if features.has_fast_load() {
            Self::optimize_load_fast(
                &mut items,
                &self.exception_regions,
                &self.borrow_unreachable_blocks,
                &self.borrow_loop_exit_blocks,
                &self.preserved_block_boundaries,
                &self.borrow_block_aliases,
                assertion_side_exits,
            );
        }
        if features.has_borrow_empty_block() {
            Self::remove_borrow_empty_block_markers(&mut items);
        }
        #[cfg(any(test, debug_assertions))]
        validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::OptimizeLoadFast,
        )?;
        Self::resolve_deferred_fallthrough_locations(
            &mut items,
            self.has_synthesized_percent_target_locations,
            &synthesized_fallthrough_locations,
        );
        if !self.deferred_shared_jump_blocks.is_empty() {
            let mut shared_jump = false;
            for item in &mut items {
                match item {
                    Item::Label(label) => {
                        shared_jump |= self.deferred_shared_jump_blocks.contains(label);
                    }
                    Item::Instruction(instruction) => {
                        if shared_jump && crate::target::is_unconditional_jump(instruction.opcode) {
                            instruction.location = super::SourceLocation::NONE;
                        }
                        shared_jump = false;
                    }
                }
            }
        }
        Ok(LayoutAssembler {
            items,
            exception_regions: self.exception_regions,
            exception_owner_provenance: self.exception_owner_provenance,
            preserved_block_boundaries: self.preserved_block_boundaries,
            removed_max_depth,
            next_label: self.next_label,
            has_next_location: self.has_next_location,
        })
    }
}

impl LayoutAssembler {
    /// Resolves CPython-style `NEXT_LOCATION` markers after CFG optimization.
    fn resolve_next_locations(&mut self) {
        let mut next_location = None;
        for item in self.items.iter_mut().rev() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            if instruction.fusion_policy.location_from_next() {
                instruction.location = next_location.unwrap_or(super::SourceLocation::NONE);
                instruction.fusion_policy.clear_location_from_next();
            }
            next_location = Some(instruction.location);
        }
    }

    fn finish(mut self, first_line_number: u32) -> Result<AssembledCode, CompileError> {
        if self.has_next_location {
            self.resolve_next_locations();
        }
        let mut labels = vec![None; usize::try_from(self.next_label).unwrap()];
        let mut instruction_count = 0;
        let mut code_units = 0;
        let mut can_be_unextended = true;
        for item in &self.items {
            match *item {
                Item::Instruction(instruction) => {
                    instruction_count += 1;
                    code_units += 1 + u32::from(instruction.opcode.caches());
                    can_be_unextended &= match instruction.operand() {
                        Operand::Value(argument) => u8::try_from(argument).is_ok(),
                        Operand::Forward(_) | Operand::Backward(_) => true,
                        Operand::Deferred(_) => false,
                    };
                }
                Item::Label(label) => {
                    labels[usize::try_from(label.0).unwrap()] = Some(code_units);
                }
            }
        }

        // CPython initially stores instruction indices in jump operands. Their widths
        // seed offset resolution, including self-dependent backward jumps at 255/256.
        let initial_jump_arguments = if instruction_count > usize::from(u8::MAX) {
            let mut indices = vec![None; labels.len()];
            let mut index = 0_u32;
            for item in &self.items {
                match *item {
                    Item::Instruction(_) => index += 1,
                    Item::Label(label) => indices[usize::try_from(label.0).unwrap()] = Some(index),
                }
            }
            can_be_unextended &= self.items.iter().all(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(label) | Operand::Backward(label) => {
                        label_position(&indices, label)
                            .is_some_and(|index| u8::try_from(index).is_ok())
                    }
                    Operand::Value(_) | Operand::Deferred(_) => true,
                },
                Item::Label(_) => true,
            });
            Some(indices)
        } else {
            None
        };

        if can_be_unextended
            && let Some((bytecode, line_table, max_depth)) =
                self.encode_unextended_final_artifacts(&labels, code_units, first_line_number)?
        {
            #[cfg(any(test, debug_assertions))]
            self.validate_structure(AssemblerStage::FinalLayout)?;
            let exception_table = if self.exception_regions.is_empty() {
                Vec::new()
            } else {
                self.exception_table(&vec![0; instruction_count], &labels)?
            };
            return Ok(AssembledCode {
                bytecode,
                line_table,
                exception_table,
                max_depth,
                removed_max_depth: self.removed_max_depth,
            });
        }

        let mut extended_args = self
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => {
                    Some(extended_arg_count(match instruction.operand() {
                        Operand::Value(argument) => argument,
                        Operand::Forward(label) | Operand::Backward(label) => {
                            initial_jump_arguments
                                .as_ref()
                                .and_then(|indices| label_position(indices, label))
                                .unwrap_or(0)
                        }
                        Operand::Deferred(_) => 0,
                    }))
                }
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let mut resolved_arguments = vec![0_u32; instruction_count];

        for _ in 0..8 {
            let code_units = self.label_positions(&extended_args, &mut labels);
            let mut changed = false;
            let mut position = 0;
            let mut instruction_index = 0;

            for item in &self.items {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                let opcode_position = position + u32::from(extended_args[instruction_index]);
                let jump_base = opcode_position + 1 + u32::from(instruction.opcode.caches());
                position = jump_base;
                let argument = match instruction.operand() {
                    Operand::Value(argument) => argument,
                    Operand::Deferred(id) => {
                        return Err(CompileError::Internal(format!(
                            "unresolved operand patch {} reached final layout",
                            id.0
                        )));
                    }
                    Operand::Forward(label) => {
                        let target = label_position(&labels, label).ok_or_else(|| {
                            CompileError::Internal("unbound forward jump label".to_string())
                        })?;
                        target.checked_sub(jump_base).ok_or_else(|| {
                            CompileError::Internal("forward jump points backward".to_string())
                        })?
                    }
                    Operand::Backward(label) => {
                        let target = label_position(&labels, label).ok_or_else(|| {
                            CompileError::Internal("unbound backward jump label".to_string())
                        })?;
                        backward_argument(instruction, jump_base, target)?
                    }
                };
                resolved_arguments[instruction_index] = argument;

                let required = extended_arg_count(argument);
                if required != extended_args[instruction_index] {
                    extended_args[instruction_index] = required;
                    changed = true;
                }
                instruction_index += 1;
            }

            if !changed {
                #[cfg(any(test, debug_assertions))]
                self.validate_structure(AssemblerStage::FinalLayout)?;
                let (bytecode, line_table, max_depth) = self.encode_final_artifacts(
                    &extended_args,
                    &resolved_arguments,
                    code_units,
                    first_line_number,
                );
                let exception_table = self.exception_table(&extended_args, &labels)?;
                return Ok(AssembledCode {
                    bytecode,
                    line_table,
                    exception_table,
                    max_depth,
                    removed_max_depth: self.removed_max_depth,
                });
            }
        }

        Err(CompileError::Internal(
            "jump layout did not converge".to_string(),
        ))
    }
}
