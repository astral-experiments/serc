#[cfg(any(test, debug_assertions))]
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use super::block_graph::BlockGraph;
use super::exception_owners::OwnerLabelPositions;
use super::fusion_owner_slots::FusionOwnerCertificates;
use super::layout::{backward_argument, label_position};
use super::{
    AssembledCode, Assembler, ConstantTable, Instruction, Item, Label, LayoutAssembler, NOP,
    Operand, OptimizationPipeline, SourceLocation, UnoptimizedCode, extended_arg_count,
};
#[cfg(any(test, debug_assertions))]
use super::{AssemblerStage, ExceptionRegion};
use crate::CompileError;

mod blocks;
mod constants;
mod control_flow;
mod legacy;
mod legacy_constants;
mod linear;
mod locals;
mod normal_try_copies;
mod normal_try_copy_plan;
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
                label.as_u32()
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
                    id.as_u32()
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
                target.as_u32()
            )));
        };
        if is_forward && target_index <= index {
            return Err(invariant_error(format!(
                "instruction {index} has a forward operand to label {} at {target_index}",
                target.as_u32()
            )));
        }
        if !is_forward && target_index >= index {
            return Err(invariant_error(format!(
                "instruction {index} has a backward operand to label {} at {target_index}",
                target.as_u32()
            )));
        }
    }

    for (index, region) in exception_regions.iter().enumerate() {
        let position = |label: Label, role: &str| {
            label_positions.get(&label).copied().ok_or_else(|| {
                invariant_error(format!(
                    "exception region {index} has an unbound {role} label {}",
                    label.as_u32()
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
        #[cfg(test)]
        super::snapshots::record(stage, items, &self.exception_regions);
        validate_structure(items, &self.exception_regions, reachable.as_deref(), stage)
    }
}

#[cfg(any(test, debug_assertions))]
impl OptimizationPipeline {
    fn validate_structure(&self, stage: AssemblerStage) -> Result<(), CompileError> {
        self.graph
            .validate()
            .map_err(|message| invariant_error(stage, &message))?;
        let items = self.graph.copy_items();
        let reachable = stage
            .requires_full_reachability()
            .then(|| self.reachable_items());
        #[cfg(test)]
        super::snapshots::record(stage, &items, &self.exception_regions);
        validate_structure(&items, &self.exception_regions, reachable.as_deref(), stage)
    }
}

#[cfg(any(test, debug_assertions))]
impl LayoutAssembler {
    fn validate_structure(&self, stage: AssemblerStage) -> Result<(), CompileError> {
        #[cfg(test)]
        super::snapshots::record(stage, &self.items, &self.exception_regions);
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
        self.prepare_legacy_yield_arguments();
        self.resolve_folded_condition_borrow_entries();
        // Keep ordinary blocks with more locals on the graph path, which also preserves
        // initialization for locals above CPython's 64-bit dataflow mask.
        if self.python_version == crate::PythonVersion::Py314
            && self
                .exception_owner_provenance
                .module_annotation_nop_pairs
                .is_empty()
            && self
                .exception_owner_provenance
                .module_annotation_jump_pairs
                .is_empty()
            && !(self.early_boolean_admissions_certified && self.has_early_boolean_admissions)
            && (self.has_only_linear_assertion_side_exits()
                || self.has_only_linear_constant_return()
                || (local_count <= 64
                    && (self.has_only_linear_instructions(local_count)
                        || self.has_only_linear_terminal_guards())))
        {
            return self
                .optimize_linear_assertion_side_exits(
                    local_count,
                    parameter_count,
                    constants.as_deref(),
                )?
                .finish(first_line_number);
        }
        self.into_optimization_pipeline()
            .optimize(first_line_number, local_count, parameter_count, constants)?
            .finish(first_line_number)
    }

    pub(in crate::assembler) fn into_optimization_pipeline(mut self) -> OptimizationPipeline {
        self.prune_unused_loop_finally_region_boundaries();
        self.resolve_folded_condition_borrow_entries();
        let mut exception_owner_provenance = self.exception_owner_provenance.into_box();
        debug_assert_eq!(exception_owner_provenance.original_pass_nops.len(), 0);
        exception_owner_provenance.original_pass_nops = self.original_pass_nops;
        let (mut graph, initial_reachable) = match self.code {
            UnoptimizedCode::Linear(items) => (
                BlockGraph::from_items(items, &rustc_hash::FxHashSet::default()),
                None,
            ),
            UnoptimizedCode::Prepared {
                mut graph,
                mut reachable,
            } => {
                if !self.preserved_block_boundaries.is_empty() {
                    graph.remove_initial_preserved_boundaries(Some(&mut reachable));
                }
                (graph, Some(reachable))
            }
        };
        if self.python_version <= crate::PythonVersion::Py313
            && let Some(index) = self.deferred_none_constant_index
        {
            // Legacy targets prune constants after CFG optimization. Resolve the
            // already-admitted None before LOAD_CONST can fold into RETURN_CONST.
            graph.for_each_instruction_mut(|instruction| {
                if instruction.opcode == crate::target::opcodes::LOAD_CONST
                    && instruction.operand() == Operand::Value(u32::MAX)
                {
                    instruction.set_operand(Operand::Value(index));
                }
            });
        }
        for (entry, original) in self.folded_arm_exit_origins {
            let Some(target) = original.forward_target() else {
                continue;
            };
            let (Some(source), Some(target)) =
                (graph.label_block(entry), graph.label_block(target))
            else {
                continue;
            };
            if !graph[source]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(*instruction),
                    Item::Label(_) => None,
                })
                .eq(std::iter::once(original))
            {
                continue;
            }
            // A folded successful arm can thread through an original unlocated
            // branch exit. The remaining runtime None-test then owns that exit's line.
            // Keep return arrays and every protected or multiply entered join separate.
            let folded_join = (|| {
                use crate::target::opcodes::{
                    JUMP_FORWARD, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, RAISE_VARARGS,
                };

                if !self.exception_regions.is_empty() {
                    return None;
                }
                let (Item::Instruction(bridge), labels) = graph[target].items().split_last()?
                else {
                    return None;
                };
                let end = bridge.forward_target()?;
                let end = graph.label_block(end)?;
                let mut unlocated = super::ir::NoLocationPolicy::default();
                unlocated.preserve_location();
                if labels.is_empty()
                    || !labels.iter().all(|item| {
                        matches!(item, Item::Label(label)
                        if exception_owner_provenance.block_boundaries.contains(label)
                            && !self.original_nop_targets.contains(label))
                    })
                    || bridge.opcode != JUMP_FORWARD
                    || bridge.location != SourceLocation::NONE
                    || bridge.no_location_policy != unlocated
                    || bridge.jump_policy != super::ir::JumpPolicy::default()
                    || bridge.exception_policy() != super::ExceptionPolicy::default()
                    || bridge.pre_fold_extra_instructions() != 0
                    || bridge.local_policy.hidden_exception_owner_slots() != 0
                    || bridge.depth_after() != original.depth_after()
                    || original.exception_policy() != super::ExceptionPolicy::default()
                {
                    return None;
                }
                let order = graph.order();
                let source_position = order.iter().position(|block| *block == source)?;
                let target_position = order.iter().position(|block| *block == target)?;
                let end_position = order.iter().position(|block| *block == end)?;
                if !(source_position < target_position && target_position < end_position)
                    || graph[*order.get(target_position.checked_sub(1)?)?]
                        .last_instruction()?
                        .opcode
                        != RAISE_VARARGS
                {
                    return None;
                }
                let mut incoming = graph
                    .iter()
                    .flat_map(|(block, array)| {
                        array.items().iter().filter_map(move |item| match item {
                            Item::Instruction(instruction) => Some((block, *instruction)),
                            Item::Label(_) => None,
                        })
                    })
                    .filter(|(_, instruction)| {
                        matches!(instruction.operand(),
                    Operand::Forward(label) | Operand::Backward(label)
                        if graph.label_block(label) == Some(target))
                    });
                let first = incoming.next()?;
                let second = incoming.next()?;
                if incoming.next().is_some() {
                    return None;
                }
                let (condition_block, condition) = if first == (source, original) {
                    second
                } else if second == (source, original) {
                    first
                } else {
                    return None;
                };
                if !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                    || condition.forward_target().is_none()
                    || condition.location.line < 0
                    || condition.location.line != original.location.line
                    || condition.depth_after() != original.depth_after()
                    || condition.pre_fold_extra_instructions() != 0
                    // Same-line None specialization removes LOAD_CONST, IS_OP, and
                    // TO_BOOL. Admit those three slots only on the captured source record.
                    || (condition.local_policy.hidden_exception_owner_slots() != 0
                        && (condition.local_policy.hidden_exception_owner_slots() != 3
                            || !exception_owner_provenance.source_none_conditions
                                .iter().any(|origin| origin.instruction == condition)))
                    || condition.exception_policy() != super::ExceptionPolicy::default()
                    || graph[condition_block].last_instruction() != Some(&condition)
                {
                    return None;
                }
                Some(bridge.operand())
            })();
            if let Some(destination) = folded_join {
                graph
                    .instructions_mut(source)
                    .next()
                    .expect("exact folded arm")
                    .retarget(destination);
                graph
                    .instructions_mut(target)
                    .next()
                    .expect("exact original join")
                    .no_location_policy
                    .clear_preserved_location();
                continue;
            }
            let mut instructions = graph[target]
                .items()
                .iter()
                .filter_map(Item::as_instruction);
            let (Some(first), Some(second)) = (instructions.next(), instructions.next()) else {
                continue;
            };
            let (marker, load, ret) = if first.opcode == NOP {
                let Some(ret) = instructions.next() else {
                    continue;
                };
                (Some(first), second, ret)
            } else {
                (None, first, second)
            };
            if instructions.next().is_some()
                // A literal return emits this plain marker at the expression's
                // original location before loading its constant. It is not an
                // arbitrary NOP or a location donated to an implicit return.
                || marker.is_some_and(|marker| {
                    load.opcode != crate::target::opcodes::LOAD_CONST
                        || load.location != ret.location
                        || load.depth_after() != Some(1)
                        || ret.depth_after() != Some(0)
                        || *marker
                            != Instruction::new(NOP, Operand::Value(0), load.location, Some(0))
                })
                || !matches!(
                    load.opcode,
                    crate::target::opcodes::LOAD_FAST | crate::target::opcodes::LOAD_CONST
                )
                || ret.opcode != crate::target::opcodes::RETURN_VALUE
                || load.location.line < 0
                || ret.location.line < 0
                || [load, ret].iter().any(|instruction| {
                    instruction.pre_fold_extra_instructions() != 0
                        || instruction.local_policy.hidden_exception_owner_slots() != 0
                })
            {
                continue;
            }
            // CPython's earlier small-exit pass still sees the arm's original
            // conditional jump. An implicit unlocated return instead takes the
            // separate exit-duplication path and must retain its existing policy.
            graph
                .instructions_mut(source)
                .next()
                .expect("exact original folded arm")
                .jump_policy
                .prevent_small_exit_inlining();
        }
        if !self.exception_regions.is_empty()
            || !exception_owner_provenance.normal_try_jump_owners.is_empty()
            || !exception_owner_provenance
                .ordinary_await_cleanups
                .is_empty()
        {
            // Original setup order expresses nesting even when copied cleanup code makes an
            // inner range longer than its parent, or exclusions split the parent into fragments.
            let mut label_positions = OwnerLabelPositions::new(graph.item_count());
            for (index, item) in graph.iter_items().enumerate() {
                if let Item::Label(label) = item {
                    label_positions.insert(*label, index);
                }
            }
            // Removed source labels cannot certify an intact original await.
            exception_owner_provenance
                .ordinary_await_cleanups
                .retain(|region| {
                    label_positions.get(&region.start).is_some()
                        && label_positions.get(&region.end).is_some()
                        && label_positions.get(&region.target).is_some()
                });
            for region in self
                .exception_regions
                .iter_mut()
                .chain(
                    exception_owner_provenance
                        .normal_try_jump_owners
                        .values_mut()
                        .flatten(),
                )
                .chain(
                    exception_owner_provenance
                        .ordinary_await_cleanups
                        .iter_mut(),
                )
            {
                if let super::ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
                    region.priority = super::ExceptionRegionPriority::Resolved {
                        start: label_positions[&start],
                        end: label_positions[&end],
                    };
                }
            }
        }
        #[cfg(test)]
        assert!(
            graph.features().has_jump()
                || !graph.iter_items().any(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if instruction.jump_target().is_some())
                })
        );
        exception_owner_provenance.bind_normal_try_borrow_exits(
            &graph,
            &self.preserved_block_boundaries,
            &self.exception_regions,
            &self.folded_loop_exit_anchors,
        );
        if graph.features().has_jump() {
            for item in graph.iter_items() {
                if let Item::Instruction(instruction) = item
                    && let Some(label) = instruction.jump_target()
                {
                    exception_owner_provenance.block_boundaries.insert(label);
                }
            }
        }
        // Later CFG passes can restore protected jump owners but only from this map.
        if (!self.exception_regions.is_empty()
            || !exception_owner_provenance.normal_try_jump_owners.is_empty())
            && (graph.features().has_forward_conditional()
                || !exception_owner_provenance
                    .original_value_jump_prefixes
                    .is_empty())
        {
            exception_owner_provenance.capture_uncompacted_conditional_arrays(
                &graph.copy_items(),
                self.python_version == crate::PythonVersion::Py314,
            );
        }
        Self::restore_legacy_return_pop_blocks(&mut graph, &self.legacy_return_pop_blocks);
        OptimizationPipeline {
            python_version: self.python_version,
            legacy_uniform_jump_seed: self.legacy_uniform_jump_seed,
            graph,
            folded_original_predecessor_targets: self
                .folded_away_condition_targets
                .union(&self.folded_away_loop_else_targets)
                .copied()
                .collect(),
            folded_predecessor_sources: self.folded_predecessor_sources,
            folded_loop_exit_anchors: self.folded_loop_exit_anchors,
            discarded_fold_admissions: self.discarded_fold_admissions,
            folded_constant_admissions: self.folded_constant_admissions,
            early_boolean_admissions_certified: self.early_boolean_admissions_certified,
            has_early_boolean_admissions: self.has_early_boolean_admissions,
            initial_reachable,
            retired_exception_handler_targets: rustc_hash::FxHashSet::default(),
            next_label: self.next_label,
            legacy_native_labels: self.legacy_native_labels,
            exception_regions: self.exception_regions,
            exception_owner_provenance,
            preserved_block_boundaries: self.preserved_block_boundaries,
            borrow_unreachable_blocks: self.borrow_unreachable_blocks,
            borrow_omitted_for_backedges: self.borrow_omitted_for_backedges,
            borrow_jump_entry_labels: self.borrow_jump_entry_labels,
            borrow_pre_copy_jump_entries: rustc_hash::FxHashMap::default(),
            borrow_threaded_jump_targets: rustc_hash::FxHashMap::default(),
            borrow_threaded_jump_entries: rustc_hash::FxHashMap::default(),
            borrow_authored_finally_break_origins:
                crate::assembler::ir::AuthoredFinallyBreakOrigins::default(),
            borrow_authored_finally_break_entries: rustc_hash::FxHashMap::default(),
            borrow_loop_exit_blocks: self.borrow_loop_exit_blocks,
            borrow_finally_entries: self.borrow_finally_entries,
            borrow_finally_jump_sources: rustc_hash::FxHashMap::default(),
            borrow_block_aliases: self.borrow_block_aliases,
            conditional_threading_targets: self.conditional_threading_targets,
            comprehension_assignment_exits: self.comprehension_assignment_exits,
            match_join_store_origins: self.match_join_store_origins,
            original_nop_targets: self.original_nop_targets,
            original_scope_exit_labels: self.original_scope_exit_labels,
            original_unlocated_cleanup_targets: self.original_unlocated_cleanup_targets,
            original_unlocated_discard_targets: self.original_unlocated_discard_targets,
            copied_finally_iterator_break_origins: self.copied_finally_iterator_break_origins,
            copied_finally_conjunction_break_origins: self.copied_finally_conjunction_break_origins,
            shared_finally_iterator_break_origins: self.shared_finally_iterator_break_origins,
            original_shared_unlocated_jump_entries: rustc_hash::FxHashMap::default(),
            original_jump_head_targets: rustc_hash::FxHashSet::default(),
            deferred_shared_jump_blocks: self.deferred_shared_jump_blocks,
            retained_pop_block_origins: self.retained_pop_block_origins,
            shared_jump_copy_location_blocks: self.shared_jump_copy_location_blocks,
            nested_if_unlocated_join_blocks: self.nested_if_unlocated_join_blocks,
            folded_condition_exit_targets: self.folded_condition_exit_targets,
            has_assertion_side_exit_candidate: self.has_assertion_side_exit_candidate,
            has_synthesized_percent_target_locations: self.has_synthesized_percent_target_locations,
            has_next_location: self.has_next_location,
        }
    }
}

impl OptimizationPipeline {
    fn label(&mut self) -> Label {
        let label = Label::from_u32(self.next_label);
        self.next_label += 1;
        label
    }

    fn optimize(
        mut self,
        first_line_number: u32,
        local_count: usize,
        parameter_count: usize,
        mut constants: Option<&mut dyn ConstantTable>,
    ) -> Result<LayoutAssembler, CompileError> {
        if self.python_version <= crate::PythonVersion::Py313 {
            return self.optimize_legacy(
                first_line_number,
                local_count,
                parameter_count,
                constants,
            );
        }
        let local_nop_donors = self.capture_local_nop_donors();
        let reversed_store_exits = self.capture_reversed_store_exits();
        self.preserve_shared_original_manager_pop_locations();
        self.record_shared_unlocated_jump_entries();
        let mut original_owners = FusionOwnerCertificates::capture_original_owners(&self);
        let retained_finally_origins = self.capture_retained_finally_origins();
        let setup_finally_origins = self.record_borrow_cleanup_calls_with_setups();
        // Retain immediately after any pass that edits fusion-view inputs. A skipped pass or
        // one that reports no edits can reuse the checkpoint without hiding an invalidation.
        original_owners.retain_pipeline(&self);
        if self.record_borrow_empty_jump_heads() {
            original_owners.retain_pipeline(&self);
        }
        if !self.borrow_jump_entry_labels.is_empty() {
            // Filtering this separate borrowing index changes no fusion-view input.
            self.borrow_jump_entry_labels = Self::nonempty_original_borrow_jump_entries(
                &self.graph,
                &self.exception_owner_provenance.block_boundaries,
                &self.borrow_jump_entry_labels,
            );
        }
        if !self.borrow_unreachable_blocks.is_empty() {
            let routes = Self::mark_initial_borrow_jump_entries(
                &mut self.graph,
                &self.exception_owner_provenance.block_boundaries,
            );
            if self.borrow_jump_entry_labels.is_empty() {
                // A direct break can bypass an earlier empty array, but must not
                // reopen that array through an originally threaded cleanup route.
                self.graph.for_each_instruction_mut(|instruction| {
                    if !instruction.fusion_policy.is_authored_break()
                        || instruction
                            .initial_borrow_jump_target()
                            .is_some_and(|target| routes.contains_key(&target))
                    {
                        instruction.clear_initial_borrow_jump_target();
                    }
                });
            } else {
                self.borrow_threaded_jump_targets = routes;
            }
            original_owners.retain_pipeline(&self);
        }
        self.record_original_jump_heads();
        self.capture_normal_try_break_pop_owners();
        original_owners.retain_pipeline(&self);
        self.prepare_original_conditional_jump_targets();
        original_owners.retain_pipeline(&self);
        self.preserve_folded_pop_block_locations();
        let assertion_side_exits = self.has_only_assertion_side_exits();
        let mut removed_max_depth = if assertion_side_exits {
            None
        } else {
            self.remove_unreachable_instructions()
        };
        original_owners.retain_pipeline(&self);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveUnreachableInitial)?;
        self.prepare_normal_try_copy_plan();
        self.prepare_plain_tuple_copy_plan();
        self.prepare_conditional_normal_try_joins();
        original_owners.retain_pipeline(&self);
        let original_location_absence = if assertion_side_exits {
            rustc_hash::FxHashSet::default()
        } else {
            self.capture_original_location_absence()
        };
        let synthesized_predecessor_locations = if self.has_synthesized_percent_target_locations {
            self.resolve_synthesized_percent_locations();
            original_owners.retain_pipeline(&self);
            let locations = self.defer_synthesized_predecessor_locations();
            original_owners.retain_pipeline(&self);
            locations
        } else {
            Vec::new()
        };
        self.optimize_boolean_conversions(constants.as_deref());
        original_owners.retain_pipeline(&self);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeBooleanConversions)?;
        let threaded_normal_try_joins = self.capture_threaded_normal_try_joins();
        let threaded_jumps = !assertion_side_exits && self.thread_forward_jumps();
        self.borrow_threaded_jump_targets.clear();
        original_owners.retain_pipeline(&self);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::ThreadForwardJumps)?;
        if threaded_jumps {
            if let Some(depth) = self.remove_unreachable_instructions() {
                removed_max_depth =
                    Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
            }
            original_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveUnreachableAfterThreading)?;
        if !assertion_side_exits {
            self.remove_redundant_forward_jumps(false);
            original_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantForwardJumpsEarly)?;
        if self.optimize_redundant_store_fast() {
            original_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeRedundantStoreFast)?;
        let copied_comprehension_swaps = self.capture_copied_comprehension_swaps();
        let (swap_blocks, capture_cleanup_swaps, swaps_changed) = self.optimize_swap_runs();
        if swaps_changed {
            original_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::OptimizeSwapRuns)?;
        let mut static_swap_store_gaps =
            self.apply_static_swaps((swap_blocks, capture_cleanup_swaps));
        original_owners.retain_pipeline(&self);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::ApplyStaticSwaps)?;
        let mut copied_exit_owner_retargets = Vec::new();
        let mut copied_exit_region_retargets = Vec::new();
        let copied_entry_labels = if assertion_side_exits {
            self.propagate_assertion_exit_locations();
            Vec::new()
        } else {
            self.duplicate_exit_blocks_with_store_origins(
                &original_location_absence,
                (
                    &mut copied_exit_owner_retargets,
                    &mut copied_exit_region_retargets,
                ),
                &reversed_store_exits,
                &threaded_normal_try_joins,
                &mut static_swap_store_gaps,
                constants
                    .as_mut()
                    .map(|constants| &mut **constants as &mut dyn ConstantTable),
            )?
        };
        self.replay_copied_comprehension_swaps(copied_comprehension_swaps);
        self.borrow_threaded_jump_entries.clear();
        self.borrow_authored_finally_break_origins =
            crate::assembler::ir::AuthoredFinallyBreakOrigins::default();
        // Also clear scratch origins on assertion-only and duplicate-pass early-return paths.
        if !self.borrow_unreachable_blocks.is_empty() {
            Self::clear_initial_borrow_jump_entries(&mut self.graph);
        }
        #[cfg(test)]
        assert!(self.graph.iter_items().all(|item| {
            !matches!(item, Item::Instruction(instruction)
                if instruction.initial_borrow_jump_target().is_some())
        }));
        original_owners.retain_after_exit_copy(
            &self,
            &copied_exit_owner_retargets,
            &copied_exit_region_retargets,
        );
        // Exit duplication changes this surviving conditional's target, not its
        // original array or the normal-finally comparison cells recorded within it.
        for (before, after) in &copied_exit_owner_retargets {
            if self
                .exception_owner_provenance
                .copied_handler_cleanups
                .iter()
                .any(|origin| origin.following_iterator)
                && self
                    .exception_owner_provenance
                    .source_none_conditions
                    .iter()
                    .filter(|origin| origin.instruction == *before)
                    .count()
                    == 1
                && self
                    .graph
                    .iter_items()
                    .filter(|item| matches!(item, Item::Instruction(record) if record == after))
                    .count()
                    == 1
            {
                self.exception_owner_provenance
                    .handler_iterator_none_retargets
                    .push((*before, *after));
            }
            let primitive_origins = &mut self
                .exception_owner_provenance
                .original_primitive_condition_boundaries;
            if primitive_origins
                .iter()
                .filter(|origin| **origin == *before)
                .count()
                == 1
                && self
                    .graph
                    .iter_items()
                    .filter(|item| matches!(item, Item::Instruction(record) if record == after))
                    .count()
                    == 1
            {
                for origin in primitive_origins {
                    if *origin == *before {
                        *origin = *after;
                    }
                }
                // The named-break proof authenticates this same primitive origin
                // independently of its cleanup array. Keep both witnesses in sync.
                for origin in &mut self
                    .exception_owner_provenance
                    .copied_named_break_truth_prefixes
                {
                    if origin.original_condition == *before {
                        origin.original_condition = *after;
                    }
                }
            }
            for prefix in &mut self
                .exception_owner_provenance
                .normal_finally_truth_prefixes
            {
                if prefix[3] == *before {
                    prefix[3] = *after;
                }
            }
            for prefix in self
                .exception_owner_provenance
                .copied_shared_break_truth_prefixes
                .iter_mut()
                .chain(
                    self.exception_owner_provenance
                        .copied_named_break_truth_prefixes
                        .iter_mut()
                        .map(|origin| &mut origin.items),
                )
            {
                for item in prefix {
                    if let Item::Instruction(condition) = item
                        && condition == before
                    {
                        // Preserve the actual copy pass's complete before/after record;
                        // the primitive witness is transported separately above.
                        *condition = *after;
                    }
                }
            }
            for prefix in &mut self
                .exception_owner_provenance
                .copied_conjunction_break_prefixes
            {
                for item in prefix {
                    if let Item::Instruction(condition) = item
                        && condition == before
                    {
                        // Only an actual cleanup copy may transport this full record.
                        // The owner resolver still requires literal prefix equality.
                        *condition = *after;
                    }
                }
            }
            for origin in &mut self
                .exception_owner_provenance
                .uncompacted_conditional_arrays
            {
                for item in &mut origin.prefix {
                    if let Item::Instruction(condition) = item
                        && condition == before
                    {
                        // Transport only the recorded retarget; final replay still
                        // requires equality of every original array record.
                        *condition = *after;
                    }
                }
            }
            for (_, _, condition) in self
                .exception_owner_provenance
                .normal_finally_none_prefixes
                .iter_mut()
                .chain(&mut self.exception_owner_provenance.copied_finally_none_prefixes)
            {
                if condition == before {
                    *condition = *after;
                }
            }
            for origin in &mut self.exception_owner_provenance.copied_return_none_prefixes {
                if origin.condition == *before {
                    origin.condition = *after;
                }
            }
            for (_, _, records) in &mut self
                .exception_owner_provenance
                .copied_bare_handler_none_prefixes
            {
                if records[8] == *before {
                    records[8] = *after;
                }
            }
            for records in &mut self
                .exception_owner_provenance
                .copied_iterator_attribute_none_prefixes
            {
                if records[3] == *before {
                    records[3] = *after;
                }
            }
            for (_, records) in &mut self
                .exception_owner_provenance
                .copied_handler_return_none_prefixes
            {
                if records[5] == *before {
                    records[5] = *after;
                }
            }
            for origin in &mut self
                .exception_owner_provenance
                .saved_named_handler_none_prefixes
            {
                for item in &mut origin.items {
                    if let Item::Instruction(condition) = item
                        && condition == before
                    {
                        *condition = *after;
                    }
                }
            }
            for origin in &mut self
                .exception_owner_provenance
                .continued_handler_try_prefixes
            {
                for item in &mut origin.items {
                    if let Item::Instruction(condition) = item
                        && condition == before
                    {
                        // This pair comes from the actual surviving conditional's
                        // target duplication, with every other field unchanged.
                        *condition = *after;
                    }
                }
            }
            for origin in &mut self.exception_owner_provenance.copied_return_truth_prefixes {
                if origin.condition == *before {
                    origin.condition = *after;
                }
            }
            for origin in &mut self.exception_owner_provenance.copied_return_call_prefixes {
                if origin.condition == *before {
                    origin.condition = *after;
                }
            }
            for origin in &mut self
                .exception_owner_provenance
                .copied_return_attribute_truth_prefixes
            {
                if origin.condition == *before {
                    origin.condition = *after;
                }
            }
            for origin in &mut self
                .exception_owner_provenance
                .copied_manager_return_compactions
            {
                if origin.condition == *before {
                    origin.condition = *after;
                }
            }
            for origin in &mut self
                .exception_owner_provenance
                .copied_manager_return_none_prefixes
            {
                if origin.condition == *before {
                    origin.condition = *after;
                }
            }
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::DuplicateExitBlocks)?;
        // Native optimize_load_const walks the copied graph before the later folds and
        // conditional threading. Freeze instruction-owned admissions at that same boundary.
        let early_booleans = self.capture_early_boolean_admissions();
        // CPython removes unreachable blocks after its per-block optimizations, including
        // small-exit copying followed by conditional threading, before checking local loads.
        // A terminating match arm can leave its original case jump newly unreachable here.
        let threaded_original = self.thread_original_conditional_jump_targets();
        original_owners.retain_pipeline(&self);
        if threaded_original {
            if let Some(depth) = self.remove_unreachable_instructions() {
                removed_max_depth =
                    Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
            }
            original_owners.retain_pipeline(&self);
        }
        if let Some(constants) = constants {
            let sequence_depth = if self.python_version == crate::PythonVersion::Py315 {
                self.fold_constant_sequences(constants)?
            } else {
                None
            };
            if let Some(depth) = sequence_depth {
                removed_max_depth =
                    Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
                original_owners.retain_pipeline(&self);
            }
            if sequence_depth.is_some()
                || !copied_entry_labels.is_empty()
                || early_booleans.is_some()
                || !self.folded_constant_admissions.is_empty()
            {
                self.fold_copied_constants(
                    constants,
                    &copied_entry_labels,
                    early_booleans.as_deref().unwrap_or_default(),
                )?;
                original_owners.retain_pipeline(&self);
            }
            if self.python_version == crate::PythonVersion::Py315 {
                self.lower_common_constants(constants);
                original_owners.retain_pipeline(&self);
            }
        }
        if let Some(depth) = self.remove_redundant_copy_pop_pairs() {
            removed_max_depth = Some(removed_max_depth.map_or(depth, |current| current.max(depth)));
        }
        original_owners.retain_pipeline(&self);
        if assertion_side_exits {
            self.add_checks_for_assertion_side_exits(local_count, parameter_count);
        } else {
            self.add_checks_for_uninitialized_loads(
                local_count,
                parameter_count,
                &local_nop_donors,
            );
        }
        original_owners.retain_pipeline(&self);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::AddChecksForUninitializedLoads)?;
        if self.compact_static_swap_store_gaps(static_swap_store_gaps) {
            original_owners.retain_pipeline(&self);
        }
        let before_fusion = FusionOwnerCertificates::snapshot(&self);
        let fusion_changed = self.fuse_superinstructions();
        let mut fusion_owners = original_owners.after_fusion(before_fusion, &self, fusion_changed);
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::FuseSuperinstructions)?;
        let omitted_borrow_markers = if !assertion_side_exits {
            let mut owner_retargets = Vec::new();
            let omitted = self.push_cold_blocks_to_end(
                fusion_owners
                    .has_manager_setup_certificates()
                    .then_some(&mut owner_retargets),
            );
            fusion_owners.retain_after_cold_regions(&self, &owner_retargets);
            omitted
        } else {
            rustc_hash::FxHashSet::default()
        };
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::PushColdBlocksToEnd)?;
        if !assertion_side_exits && self.remove_redundant_checked_loads() {
            fusion_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantCheckedLoads)?;
        let mut locations_changed = self.resolve_folded_sequence_locations();
        locations_changed |= self.propagate_locations_within_blocks();
        // Assertion-only paths skip cold-block retention and still need this checkpoint.
        if assertion_side_exits || locations_changed {
            fusion_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::PropagateLocationsWithinBlocks)?;
        if self.remove_redundant_swaps_before_pops() {
            fusion_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantSwapsBeforePops)?;
        let nop_cleanup = self.remove_redundant_nops();
        if nop_cleanup.may_have_changed {
            fusion_owners.retain_pipeline(&self);
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantNops)?;
        if !assertion_side_exits {
            if nop_cleanup.has_forward_jump {
                self.remove_redundant_forward_jumps(true);
                fusion_owners.retain_pipeline(&self);
            }
            // Manager exits can expose a redundant POP_BLOCK after their jump disappears.
            if self.remove_redundant_with_exit_nops() {
                fusion_owners.retain_pipeline(&self);
            }
        }
        #[cfg(any(test, debug_assertions))]
        self.validate_structure(AssemblerStage::RemoveRedundantForwardJumpsLate)?;
        if self.normalize_backward_conditionals()? {
            fusion_owners.retain_pipeline(&self);
        }
        let mut retained_finally_entries = self.retained_finally_entries(&retained_finally_origins);
        let module_annotation_borrow_markers =
            control_flow::surviving_module_annotation_borrow_markers(
                &self.graph,
                &self.exception_owner_provenance.module_annotation_jump_nops,
            );
        let features = self.graph.features();
        let mut items = self.graph.into_items();
        if !omitted_borrow_markers.is_empty() {
            // Analysis-only boundaries must not obstruct physical jump or NOP cleanup.
            let mut marked = Vec::with_capacity(items.len() + omitted_borrow_markers.len());
            let mut remaining = omitted_borrow_markers.clone();
            for item in items {
                let marker_label = match item {
                    Item::Label(label) if omitted_borrow_markers.contains(&label) => Some(label),
                    _ => None,
                };
                marked.push(item);
                if let Some(label) = marker_label {
                    if !remaining.remove(&label) {
                        return Err(CompileError::Internal(
                            "deferred borrow marker label was duplicated".to_owned(),
                        ));
                    }
                    let mut marker =
                        Instruction::synthetic(NOP, Operand::Value(0), SourceLocation::NONE, None);
                    marker.jump_policy.mark_borrow_empty_block();
                    marked.push(Item::Instruction(marker));
                }
            }
            if !remaining.is_empty() {
                return Err(CompileError::Internal(
                    "deferred borrow marker label was removed".to_owned(),
                ));
            }
            items = marked;
        }
        let mut setup_finally_entries: Vec<_> = Self::retain_setup_finally_origins(
            &items,
            &setup_finally_origins,
            &mut self.borrow_finally_entries,
            &mut self.borrow_block_aliases,
        )
        .into_iter()
        .collect();
        setup_finally_entries.sort_unstable_by_key(|(source, _)| source.as_u32());
        for (source, target) in setup_finally_entries {
            retained_finally_entries.entry(source).or_insert(target);
        }
        if features.has_fast_load() {
            // These entries split only borrowed-reference analysis. They do not
            // change earlier CFG copying, NOP removal or exception ownership.
            let borrow_boundaries = if self
                .exception_owner_provenance
                .borrow_only_block_boundaries
                .is_empty()
            {
                std::borrow::Cow::Borrowed(&self.preserved_block_boundaries)
            } else {
                let mut boundaries = self.preserved_block_boundaries.clone();
                boundaries.extend(
                    self.exception_owner_provenance
                        .borrow_only_block_boundaries
                        .iter()
                        .copied(),
                );
                std::borrow::Cow::Owned(boundaries)
            };
            let mut break_loads_before = Vec::new();
            for (prefix_index, prefix) in self
                .exception_owner_provenance
                .copied_shared_break_truth_prefixes
                .iter()
                .chain(
                    self.exception_owner_provenance
                        .copied_named_break_truth_prefixes
                        .iter()
                        .map(|origin| &origin.items),
                )
                .enumerate()
            {
                for (record_index, item) in prefix.iter().enumerate() {
                    let Item::Instruction(record) = item else {
                        continue;
                    };
                    if record.opcode != crate::target::opcodes::LOAD_FAST {
                        continue;
                    }
                    let Some(anchor) =
                        prefix[..record_index]
                            .iter()
                            .rev()
                            .find_map(|item| match item {
                                Item::Label(label) => Some(*label),
                                Item::Instruction(_) => None,
                            })
                    else {
                        continue;
                    };
                    let mut anchors = items
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| **item == Item::Label(anchor));
                    let Some((anchor_index, _)) = anchors.next() else {
                        continue;
                    };
                    if anchors.next().is_some() {
                        continue;
                    }
                    let Some((offset, before)) = items[anchor_index + 1..]
                        .iter()
                        .enumerate()
                        .find_map(|(offset, item)| match item {
                            Item::Instruction(before) => Some((offset, *before)),
                            Item::Label(_) => None,
                        })
                    else {
                        continue;
                    };
                    if before == *record {
                        break_loads_before.push((
                            prefix_index,
                            record_index,
                            anchor_index + 1 + offset,
                            before,
                        ));
                    }
                }
            }
            Self::optimize_load_fast_with_retained_entries(
                self.python_version,
                &mut items,
                &self.exception_regions,
                locals::BorrowEntryLabels {
                    unreachable: &self.borrow_unreachable_blocks,
                    loop_exits: &self.borrow_loop_exit_blocks,
                    finally_entries: &self.borrow_finally_entries,
                    finally_jump_sources: &self.borrow_finally_jump_sources,
                    jump_entries: &self.borrow_jump_entry_labels,
                    original_boundaries: &self.exception_owner_provenance.block_boundaries,
                    pre_copy_jump_entries: &self.borrow_pre_copy_jump_entries,
                    authored_finally_break_entries: &self.borrow_authored_finally_break_entries,
                },
                &borrow_boundaries,
                &self.borrow_block_aliases,
                assertion_side_exits,
                &retained_finally_entries,
            );
            for (prefix_index, record_index, index, before) in break_loads_before {
                let mut expected = before;
                expected.opcode = crate::target::opcodes::LOAD_FAST_BORROW;
                if items[index] != Item::Instruction(expected) {
                    continue;
                }
                // The unique captured range alias identified this physical load before
                // analysis; transport only its actual conversion at that same slot.
                let prefix = self
                    .exception_owner_provenance
                    .copied_shared_break_truth_prefixes
                    .iter_mut()
                    .chain(
                        self.exception_owner_provenance
                            .copied_named_break_truth_prefixes
                            .iter_mut()
                            .map(|origin| &mut origin.items),
                    )
                    .nth(prefix_index)
                    .expect("captured break prefix is retained during borrowing");
                if prefix[record_index] == Item::Instruction(before) {
                    prefix[record_index] = Item::Instruction(expected);
                }
            }
        }
        if features.has_borrow_empty_block() || !omitted_borrow_markers.is_empty() {
            control_flow::retire_module_annotation_borrow_markers(
                &mut items,
                &module_annotation_borrow_markers,
            );
            Self::remove_borrow_empty_block_markers(&mut items);
        }
        #[cfg(any(test, debug_assertions))]
        validate_structure(
            &items,
            &self.exception_regions,
            None,
            AssemblerStage::OptimizeLoadFast,
        )?;
        Self::resolve_deferred_percent_locations(
            &mut items,
            self.has_synthesized_percent_target_locations,
            &synthesized_predecessor_locations,
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
        self.exception_owner_provenance.fused_manager_owners = fusion_owners.finalize(
            &mut items,
            &self.exception_regions,
            &self.exception_owner_provenance,
            &self.preserved_block_boundaries,
        );
        Ok(LayoutAssembler {
            python_version: self.python_version,
            legacy_uniform_jump_seed: self.legacy_uniform_jump_seed,
            items,
            exception_regions: self.exception_regions,
            exception_owner_provenance: self.exception_owner_provenance.into(),
            preserved_block_boundaries: self.preserved_block_boundaries,
            removed_max_depth,
            next_label: self.next_label,
            has_next_location: self.has_next_location,
        })
    }
}

/// Symbolic labels and jump targets collected alongside the first physical layout pass.
#[derive(Default)]
struct InitialSymbolicJumps {
    labels: SmallVec<[(Label, u32); 32]>,
    targets: SmallVec<[Label; 16]>,
}

impl InitialSymbolicJumps {
    fn record_label(&mut self, label: Label, instruction_index: usize) {
        self.labels.push((
            label,
            u32::try_from(instruction_index).expect("final symbolic instruction index fits u32"),
        ));
    }

    fn finish(self, label_count: usize) -> Option<(Vec<Option<u32>>, bool)> {
        if self.targets.is_empty() {
            return None;
        }
        let mut indices = vec![None; label_count];
        for (label, index) in self.labels {
            indices[label.index()] = Some(index);
        }
        let can_be_unextended = self.targets.into_iter().all(|target| {
            label_position(&indices, target).is_some_and(|index| u8::try_from(index).is_ok())
        });
        Some((indices, can_be_unextended))
    }
}

impl LayoutAssembler {
    /// Computes wire operands without changing semantic instruction provenance.
    #[inline]
    pub(in crate::assembler) fn encoded_operand(
        &self,
        instruction: &Instruction,
    ) -> Result<Operand, CompileError> {
        use crate::target::opcodes::{GET_YIELD_FROM_ITER, RESUME};

        if self.python_version == crate::PythonVersion::Py314
            || !matches!(instruction.opcode, RESUME | GET_YIELD_FROM_ITER)
        {
            Ok(instruction.operand())
        } else {
            self.encoded_version_specific_operand(instruction)
        }
    }

    fn encoded_version_specific_operand(
        &self,
        instruction: &Instruction,
    ) -> Result<Operand, CompileError> {
        use crate::target::GENEXPR_RESUME_ARGUMENT;
        use crate::target::opcodes::{GET_YIELD_FROM_ITER, RESUME};
        use crate::target::operands::ResumeLocation;
        use crate::target::operands315::{IteratorMode, ResumeLocation as ResumeLocation315};

        let operand = instruction.operand();
        if self.python_version != crate::PythonVersion::Py315 {
            if instruction.opcode == RESUME
                && let Operand::Value(argument) = operand
            {
                let argument = if argument == GENEXPR_RESUME_ARGUMENT {
                    0
                } else if self.python_version == crate::PythonVersion::Py312 {
                    argument & 3
                } else {
                    argument
                };
                return Ok(Operand::Value(argument));
            }
            return Ok(operand);
        }
        let Operand::Value(argument) = operand else {
            return Ok(operand);
        };
        let encoded = match instruction.opcode {
            GET_YIELD_FROM_ITER => IteratorMode::YIELD_FROM_CORO_CHECK.argument(),
            RESUME if argument == GENEXPR_RESUME_ARGUMENT => {
                ResumeLocation315::AT_GEN_EXPR_START.argument()
            }
            RESUME => {
                let depth_flag = ResumeLocation::AT_FUNC_START.at_depth(true);
                let location = argument & !depth_flag;
                let target = [
                    (
                        ResumeLocation::AT_FUNC_START,
                        ResumeLocation315::AT_FUNC_START,
                    ),
                    (ResumeLocation::AFTER_YIELD, ResumeLocation315::AFTER_YIELD),
                    (
                        ResumeLocation::AFTER_YIELD_FROM,
                        ResumeLocation315::AFTER_YIELD_FROM,
                    ),
                    (ResumeLocation::AFTER_AWAIT, ResumeLocation315::AFTER_AWAIT),
                ]
                .into_iter()
                .find_map(|(semantic, target)| (semantic.argument() == location).then_some(target))
                .ok_or_else(|| CompileError::Internal("unknown semantic RESUME location".into()))?;
                target.at_depth(argument & depth_flag != 0)
            }
            _ => return Ok(operand),
        };
        Ok(Operand::Value(encoded))
    }

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
        if matches!(
            self.python_version,
            crate::PythonVersion::Py314 | crate::PythonVersion::Py315
        ) && let Some((bytecode, line_table, max_depth)) =
            self.encode_small_final_artifacts(&mut labels, first_line_number)?
        {
            #[cfg(any(test, debug_assertions))]
            self.validate_structure(AssemblerStage::FinalLayout)?;
            let exception_table = if self.exception_regions.is_empty() {
                Vec::new()
            } else {
                self.unextended_exception_table(&labels)?
            };
            return Ok(AssembledCode {
                bytecode,
                line_table,
                exception_table,
                max_depth,
                removed_max_depth: self.removed_max_depth,
            });
        }
        let mut instruction_count = 0;
        let mut code_units = 0;
        let mut can_be_unextended = true;
        let mut symbolic_jumps =
            (self.items.len() > usize::from(u8::MAX)).then(InitialSymbolicJumps::default);
        for item in &self.items {
            match *item {
                Item::Instruction(instruction) => {
                    instruction_count += 1;
                    code_units += 1 + u32::from(instruction.opcode.caches_for(self.python_version));
                    can_be_unextended &= match self.encoded_operand(&instruction)? {
                        Operand::Value(argument) => u8::try_from(argument).is_ok(),
                        Operand::Forward(target) | Operand::Backward(target) => {
                            if let Some(symbolic_jumps) = &mut symbolic_jumps {
                                symbolic_jumps.targets.push(target);
                            }
                            true
                        }
                        Operand::Deferred(_) => false,
                    };
                }
                Item::Label(label) => {
                    labels[label.index()] = Some(code_units);
                    if let Some(symbolic_jumps) = &mut symbolic_jumps {
                        symbolic_jumps.record_label(label, instruction_count);
                    }
                }
            }
        }

        let uniform_jump_widths = self.python_version == crate::PythonVersion::Py312
            && self.legacy_uniform_jump_seed
            // Exact nonzero provenance contradicts a universal zero-width
            // proof. Fail closed to the existing exact/index path for the whole
            // object, without changing either certificate or any instruction.
            && self.items.iter().all(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(_) | Operand::Backward(_) => instruction
                        .legacy_initial_jump_width()
                        .is_none_or(|width| width == 0),
                    Operand::Value(_) | Operand::Deferred(_) => true,
                },
                Item::Label(_) => true,
            });
        let native_jump_widths = !uniform_jump_widths
            && self.python_version == crate::PythonVersion::Py312
            && self.items.iter().all(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(_) | Operand::Backward(_) => {
                        instruction.legacy_initial_jump_width().is_some()
                    }
                    Operand::Value(_) | Operand::Deferred(_) => true,
                },
                Item::Label(_) => true,
            });
        if native_jump_widths {
            can_be_unextended &= self.items.iter().all(|item| match item {
                Item::Instruction(instruction) => instruction
                    .legacy_initial_jump_width()
                    .is_none_or(|width| width == 0),
                Item::Label(_) => true,
            });
        }
        // 3.12 seeds from native symbolic operands; 3.13+ uses instruction indices.
        // Uncertified 3.12 code objects retain the existing compatibility path.
        let initial_jump_arguments = if !uniform_jump_widths
            && !native_jump_widths
            && instruction_count > usize::from(u8::MAX)
        {
            symbolic_jumps
                .expect("a scope with more than 255 instructions has more than 255 items")
                .finish(labels.len())
                .map(|(indices, jumps_can_be_unextended)| {
                    can_be_unextended &= jumps_can_be_unextended;
                    indices
                })
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
                self.unextended_exception_table(&labels)?
            };
            return Ok(AssembledCode {
                bytecode,
                line_table,
                exception_table,
                max_depth,
                removed_max_depth: self.removed_max_depth,
            });
        }

        // Cold relocation reads the original END_SEND label when the captured
        // suspension and its bridge survive. Its width is independent of any
        // labels allocated later for empty or duplicated blocks.
        let mut short_await_bridges = rustc_hash::FxHashMap::default();
        if !native_jump_widths
            && !uniform_jump_widths
            && self
                .exception_owner_provenance
                .legacy_source_jump_labels_fit
        {
            short_await_bridges.extend(
                self.exception_owner_provenance
                    .ordinary_await_ends
                    .iter()
                    .filter_map(|origin| origin.bridge),
            );
        }
        let mut expected_bridge = None;
        let mut extended_args = Vec::with_capacity(instruction_count);
        let mut resolved_arguments = Vec::with_capacity(instruction_count);
        for item in &self.items {
            let instruction = match item {
                Item::Instruction(instruction) => instruction,
                Item::Label(label) => {
                    if let Some(bridge) = short_await_bridges.get(label) {
                        expected_bridge = Some(bridge);
                    }
                    continue;
                }
            };
            let short_await_bridge = expected_bridge.take() == Some(instruction);
            let argument = match self.encoded_operand(instruction)? {
                Operand::Value(argument) => argument,
                Operand::Forward(label) | Operand::Backward(label) => initial_jump_arguments
                    .as_ref()
                    .and_then(|indices| label_position(indices, label))
                    .unwrap_or(0),
                Operand::Deferred(_) => 0,
            };
            let width = if instruction.jump_target().is_some() && uniform_jump_widths {
                // This consistent whole-object proof covers every jump; no
                // instruction or capture is mutated by accepting it.
                0
            } else if native_jump_widths && instruction.jump_target().is_some() {
                instruction
                    .legacy_initial_jump_width()
                    .expect("all native jumps are certified")
            } else if short_await_bridge
                || (self
                    .exception_owner_provenance
                    .legacy_source_jump_labels_fit
                    && matches!(
                        instruction.opcode,
                        crate::target::opcodes::SEND
                            | crate::target::opcodes::JUMP_BACKWARD_NO_INTERRUPT
                    ))
            {
                // Python 3.12 emits SEND and JUMP_NO_INTERRUPT only in its
                // yield-from loop. Their END_SEND and SEND targets are nonempty
                // source blocks, so empty-block repair cannot replace the operands.
                0
            } else {
                extended_arg_count(argument)
            };
            extended_args.push(width);
            resolved_arguments.push(argument);
        }

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
                let jump_base = opcode_position
                    + 1
                    + u32::from(instruction.opcode.caches_for(self.python_version));
                position = jump_base;
                let argument = match instruction.operand() {
                    Operand::Value(_) => {
                        // Value operands and their encoded widths do not depend on layout.
                        instruction_index += 1;
                        continue;
                    }
                    Operand::Deferred(id) => {
                        return Err(CompileError::Internal(format!(
                            "unresolved operand patch {} reached final layout",
                            id.as_u32()
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
                        backward_argument(self.python_version, instruction, jump_base, target)?
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

#[cfg(test)]
mod uniform_jump_seed_tests {
    use super::{Assembler, Instruction, Item, Label, LayoutAssembler, Operand, SourceLocation};
    use crate::PythonVersion;
    use crate::assembler::exception_owners::LazyExceptionOwnerProvenance;
    use crate::target::opcodes::{
        EXTENDED_ARG, JUMP_BACKWARD, LOAD_CONST, NOP, POP_TOP, RETURN_VALUE,
    };

    fn boundary_layout(
        version: PythonVersion,
        uniform: bool,
        exact: Option<u32>,
        wide_value: bool,
    ) -> LayoutAssembler {
        let instruction = |opcode, argument| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                SourceLocation::NONE,
                None,
            ))
        };
        let mut items = vec![
            instruction(LOAD_CONST, if wide_value { 0x1234 } else { 1 }),
            instruction(POP_TOP, 0),
        ];
        // The physical target index exceeds255, but a distinct source proof may
        // certify that all native symbolic operands start without EXTENDED_ARG.
        items.extend((0..300).map(|_| instruction(NOP, 0)));
        let target = Label::from_u32(0);
        items.push(Item::Label(target));
        items.extend((0..254).map(|_| instruction(NOP, 0)));
        let mut jump = Instruction::new(
            JUMP_BACKWARD,
            Operand::Backward(target),
            SourceLocation::NONE,
            None,
        );
        if let Some(argument) = exact {
            jump.set_legacy_initial_jump_argument(argument);
        }
        items.push(Item::Instruction(jump));
        LayoutAssembler {
            python_version: version,
            legacy_uniform_jump_seed: uniform,
            items,
            exception_regions: Vec::new(),
            exception_owner_provenance: LazyExceptionOwnerProvenance::default(),
            preserved_block_boundaries: rustc_hash::FxHashSet::default(),
            removed_max_depth: None,
            next_label: 1,
            has_next_location: false,
        }
    }

    #[test]
    fn fused_symbolic_scan_preserves_target_index_width_boundaries() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for (uniform, exact) in [(false, None), (true, None), (false, Some(0))] {
                for target_index in [254, 255, 256] {
                    let mut layout = boundary_layout(version, uniform, exact, false);
                    let instruction = |opcode, operand| {
                        Item::Instruction(Instruction::new(
                            opcode,
                            operand,
                            SourceLocation::NONE,
                            None,
                        ))
                    };
                    let target = Label::from_u32(0);
                    let caches = usize::from(JUMP_BACKWARD.caches_for(version));
                    let mut jump = Instruction::new(
                        JUMP_BACKWARD,
                        Operand::Backward(target),
                        SourceLocation::NONE,
                        None,
                    );
                    if let Some(exact) = exact {
                        jump.set_legacy_initial_jump_argument(exact);
                    }
                    layout.items =
                        std::iter::repeat_n(instruction(NOP, Operand::Value(0)), target_index)
                            .chain([Item::Label(target)])
                            .chain(std::iter::repeat_n(
                                instruction(NOP, Operand::Value(0)),
                                254 - caches,
                            ))
                            .chain([Item::Instruction(jump)])
                            .collect();
                    let code = layout.finish(1).unwrap().bytecode;
                    let jump_end = code.len() - 2 * caches;
                    let unextended = target_index <= 255
                        || (version == PythonVersion::Py312 && (uniform || exact.is_some()));
                    assert_eq!(
                        &code[jump_end - 4..jump_end],
                        if unextended {
                            [
                                NOP.code_for(version),
                                0,
                                JUMP_BACKWARD.code_for(version),
                                255,
                            ]
                        } else {
                            [
                                EXTENDED_ARG.code_for(version),
                                1,
                                JUMP_BACKWARD.code_for(version),
                                0,
                            ]
                        },
                        "{version:?}, uniform={uniform}, exact={exact:?}, target={target_index}",
                    );
                }
            }
        }
    }

    #[test]
    fn large_jump_free_layout_ignores_sparse_labels_on_both_width_paths() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for argument in [1, 0x1234] {
                let fixture = |labels: bool| {
                    let mut layout = boundary_layout(version, false, None, false);
                    let instruction = |opcode, operand| {
                        Item::Instruction(Instruction::new(
                            opcode,
                            Operand::Value(operand),
                            SourceLocation::new(1, 1, 0, 1),
                            None,
                        ))
                    };
                    layout.items.clear();
                    for index in 0..=310 {
                        if labels && matches!(index, 0 | 254 | 255 | 256 | 310) {
                            layout.items.push(Item::Label(Label::from_usize(index * 2)));
                        }
                        if index == 0 {
                            layout.items.push(instruction(LOAD_CONST, argument));
                        } else if index == 310 {
                            layout.items.push(instruction(RETURN_VALUE, 0));
                        } else {
                            layout.items.push(instruction(NOP, 0));
                        }
                    }
                    layout.next_label = if labels { 621 } else { 0 };
                    layout.finish(1).unwrap()
                };
                let sparse = fixture(true);
                let bare = fixture(false);
                assert_eq!(sparse.bytecode, bare.bytecode);
                assert_eq!(sparse.line_table, bare.line_table);
                assert_eq!(sparse.exception_table, bare.exception_table);
                assert_eq!(sparse.max_depth, bare.max_depth);
                assert_eq!(sparse.removed_max_depth, bare.removed_max_depth);
            }
        }
    }

    #[test]
    fn general_layout_preserves_target_specific_value_operands() {
        use crate::PythonVersion::{Py312, Py313, Py314, Py315};
        use crate::target::GENEXPR_RESUME_ARGUMENT;
        use crate::target::opcodes::{GET_YIELD_FROM_ITER, RESUME};
        use crate::target::operands::ResumeLocation;

        let depth = ResumeLocation::AFTER_YIELD.at_depth(true);
        for (version, expected_depth, expected_genexpr, expected_iter) in [
            (Py312, 1, 0, 0),
            (Py313, depth, 0, 0),
            (Py314, depth, GENEXPR_RESUME_ARGUMENT, 0),
            (Py315, 9, 4, 3),
        ] {
            let original = boundary_layout(version, false, None, true)
                .finish(1)
                .unwrap();
            for (opcode, argument, expected) in [
                (RESUME, depth, expected_depth),
                (RESUME, GENEXPR_RESUME_ARGUMENT, expected_genexpr),
                (GET_YIELD_FROM_ITER, 0, expected_iter),
            ] {
                let mut layout = boundary_layout(version, false, None, true);
                layout.items.insert(
                    0,
                    Item::Instruction(Instruction::new(
                        opcode,
                        Operand::Value(argument),
                        SourceLocation::NONE,
                        None,
                    )),
                );
                let mut bytecode = Vec::new();
                if expected == 0x100 {
                    bytecode.extend([EXTENDED_ARG.code_for(version), 1]);
                }
                bytecode.extend([
                    opcode.code_for(version),
                    u8::try_from(expected & 0xff).unwrap(),
                ]);
                bytecode.resize(
                    bytecode.len() + 2 * usize::from(opcode.caches_for(version)),
                    0,
                );
                bytecode.extend_from_slice(&original.bytecode);
                assert_eq!(layout.finish(1).unwrap().bytecode, bytecode);
            }
        }
    }

    #[test]
    fn general_layout_preserves_operand_error_order() {
        use crate::assembler::OperandPatchId;
        use crate::target::opcodes::{JUMP_FORWARD, RESUME};

        let deferred = (LOAD_CONST, Operand::Deferred(OperandPatchId::from_u32(7)));
        let unbound = (JUMP_FORWARD, Operand::Forward(Label::from_u32(1)));
        let invalid_resume = (RESUME, Operand::Value(99));
        let deferred_error = "unresolved operand patch 7 reached final layout";
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for (prefix, expected) in [
                ([deferred, unbound], deferred_error),
                ([unbound, deferred], "unbound forward jump label"),
                (
                    [deferred, invalid_resume],
                    if version == PythonVersion::Py315 {
                        "unknown semantic RESUME location"
                    } else {
                        deferred_error
                    },
                ),
            ] {
                let mut layout = boundary_layout(version, false, None, true);
                layout.items.splice(
                    0..0,
                    prefix.map(|(opcode, operand)| {
                        Item::Instruction(Instruction::new(
                            opcode,
                            operand,
                            SourceLocation::NONE,
                            None,
                        ))
                    }),
                );
                assert_eq!(
                    layout.finish(1).err().unwrap(),
                    crate::CompileError::Internal(expected.to_owned())
                );
            }
        }
    }

    #[test]
    fn uniform_certificate_selects_native_boundary_and_preserves_value_widths() {
        let version = PythonVersion::Py312;
        for wide_value in [false, true] {
            for exact in [None, Some(0)] {
                let code = boundary_layout(version, true, exact, wide_value)
                    .finish(1)
                    .unwrap()
                    .bytecode;
                assert!(code.ends_with(&[JUMP_BACKWARD.code_for(version), 255]));
                if wide_value {
                    assert_eq!(
                        &code[..4],
                        &[
                            EXTENDED_ARG.code_for(version),
                            0x12,
                            LOAD_CONST.code_for(version),
                            0x34
                        ]
                    );
                } else {
                    assert_eq!(&code[..2], &[LOAD_CONST.code_for(version), 1]);
                }
            }
            // An existing exact nonzero width contradicts the global bound.
            // Preserve the native extended fixed point instead of overriding it.
            let code = boundary_layout(version, true, Some(256), wide_value)
                .finish(1)
                .unwrap()
                .bytecode;
            assert!(code.ends_with(&[
                EXTENDED_ARG.code_for(version),
                1,
                JUMP_BACKWARD.code_for(version),
                0
            ]));
        }
        for exact in [None, Some(256)] {
            let code = boundary_layout(version, false, exact, true)
                .finish(1)
                .unwrap()
                .bytecode;
            assert!(code.ends_with(&[
                EXTENDED_ARG.code_for(version),
                1,
                JUMP_BACKWARD.code_for(version),
                0
            ]));
        }
        for version in [
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let without = boundary_layout(version, false, None, true)
                .finish(1)
                .unwrap()
                .bytecode;
            let with = boundary_layout(version, true, Some(0), true)
                .finish(1)
                .unwrap()
                .bytecode;
            assert_eq!(with, without);
        }
    }

    #[test]
    fn uniform_certificate_survives_exact_provenance_cancellation_and_pipeline() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let mut assembler = Assembler::with_python_version(version);
            assembler.certify_legacy_uniform_jump_seed();
            if version == PythonVersion::Py312 {
                assembler.invalidate_legacy_native_labels();
            }
            let expected = version == PythonVersion::Py312;
            assert_eq!(assembler.legacy_uniform_jump_seed, expected);
            assembler.emit(LOAD_CONST, 0);
            assembler.emit(RETURN_VALUE, 0);
            let pipeline = assembler.into_optimization_pipeline();
            assert_eq!(pipeline.legacy_uniform_jump_seed, expected);
            let layout = pipeline.optimize(1, 0, 0, None).unwrap();
            assert_eq!(layout.legacy_uniform_jump_seed, expected);
        }
    }
}
