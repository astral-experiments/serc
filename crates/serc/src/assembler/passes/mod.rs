#[cfg(any(test, debug_assertions))]
use rustc_hash::FxHashMap;

use super::block_graph::BlockGraph;
use super::layout::label_position;
use super::{
    AssembledCode, Assembler, Item, Label, LayoutAssembler, Operand, OptimizationPipeline,
    UnoptimizedCode, extended_arg_count,
};
#[cfg(any(test, debug_assertions))]
use super::{AssemblerStage, ExceptionRegion};
use crate::CompileError;

mod blocks;
mod control_flow;
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
        let (target, is_forward) = match instruction.operand {
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

    pub(crate) fn finish_code(
        mut self,
        first_line_number: u32,
        local_count: usize,
        parameter_count: usize,
    ) -> Result<AssembledCode, CompileError> {
        self.resolve_operand_patches()?;
        self.into_optimization_pipeline()
            .optimize(local_count, parameter_count)?
            .finish(first_line_number)
    }

    pub(in crate::assembler) fn into_optimization_pipeline(self) -> OptimizationPipeline {
        let graph = match self.code {
            UnoptimizedCode::Linear(items) => {
                BlockGraph::from_items(items, &rustc_hash::FxHashSet::default())
            }
            UnoptimizedCode::Prepared(mut graph) => {
                graph.remove_initial_preserved_boundaries();
                graph
            }
        };
        OptimizationPipeline {
            graph,
            next_label: self.next_label,
            exception_regions: self.exception_regions,
            preserved_block_boundaries: self.preserved_block_boundaries,
            borrow_unreachable_blocks: self.borrow_unreachable_blocks,
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
    ) -> Result<LayoutAssembler, CompileError> {
        let mut removed_max_depth = self.remove_unreachable_instructions();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveUnreachableInitial)?;
        self.optimize_boolean_conversions();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeBooleanConversions)?;
        self.thread_forward_jumps();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::ThreadForwardJumps)?;
        if let Some(depth) = self.remove_unreachable_instructions() {
            removed_max_depth = Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveUnreachableAfterThreading)?;
        self.remove_redundant_forward_jumps();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantForwardJumpsEarly)?;
        self.optimize_redundant_store_fast();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeRedundantStoreFast)?;
        self.optimize_swap_runs();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeSwapRuns)?;
        self.apply_static_swaps();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::ApplyStaticSwaps)?;
        self.duplicate_exit_blocks();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::DuplicateExitBlocks)?;
        self.add_checks_for_uninitialized_loads(local_count, parameter_count);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::AddChecksForUninitializedLoads)?;
        self.fuse_superinstructions();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::FuseSuperinstructions)?;
        self.push_cold_blocks_to_end();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::PushColdBlocksToEnd)?;
        self.remove_redundant_checked_loads();
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
        self.remove_redundant_forward_jumps();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantForwardJumpsLate)?;
        self.optimize_load_fast();
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeLoadFast)?;
        Ok(LayoutAssembler {
            items: self.graph.into_items(),
            exception_regions: self.exception_regions,
            preserved_block_boundaries: self.preserved_block_boundaries,
            removed_max_depth,
            next_label: self.next_label,
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
            if instruction.location_from_next {
                instruction.location = next_location.unwrap_or(super::SourceLocation::NONE);
                instruction.location_from_next = false;
            }
            next_location = Some(instruction.location);
        }
    }

    fn finish(mut self, first_line_number: u32) -> Result<AssembledCode, CompileError> {
        self.resolve_next_locations();
        let instruction_count = self
            .items
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        let mut extended_args = vec![0_u8; instruction_count];
        let mut resolved_arguments = vec![0_u32; instruction_count];
        let mut labels = vec![None; usize::try_from(self.next_label).unwrap()];

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
                let argument = match instruction.operand {
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
                        jump_base.checked_sub(target).ok_or_else(|| {
                            CompileError::Internal("backward jump points forward".to_string())
                        })?
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
                let line_table = self.line_table(&extended_args, first_line_number);
                let exception_table = self.exception_table(&extended_args, &labels)?;
                let max_depth = self
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => instruction.depth_after,
                        Item::Label(_) => None,
                    })
                    .max()
                    .unwrap_or(0);
                return Ok(AssembledCode {
                    bytecode: self.encode(&extended_args, &resolved_arguments, code_units),
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
