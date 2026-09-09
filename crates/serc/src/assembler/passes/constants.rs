use rustc_hash::{FxHashMap, FxHashSet};

use super::super::block_graph::ItemPosition;
use super::super::{ConstantTable, Item, Label, Operand, OptimizationPipeline, SourceLocation};
use crate::CompileError;
use crate::code_object::Constant;
use crate::target::Opcode;
use crate::target::opcodes::{BUILD_TUPLE, LOAD_CONST, LOAD_SMALL_INT, NOP, UNARY_NEGATIVE};

impl OptimizationPipeline {
    /// Folds constant operations exposed by exit copying, preserving basic-block boundaries.
    pub(super) fn fold_copied_constants(
        &mut self,
        constants: &mut dyn ConstantTable,
        copied_entry_labels: &[Label],
    ) -> Result<(), CompileError> {
        if copied_entry_labels.is_empty() {
            return Ok(());
        }
        let copied_entry_labels = copied_entry_labels
            .iter()
            .copied()
            .collect::<FxHashSet<_>>();
        let mut loads = Vec::<(ItemPosition, Opcode, u32)>::new();
        let mut source_loads = 0;
        let mut copied_jump_marker = None;
        let mut folded_operand_admissions = FxHashMap::default();
        for block in self.graph.order().to_vec() {
            // Exit copies keep a label for location and exception-table bookkeeping. CPython
            // appends their instructions to the predecessor before its tuple-folding pass.
            if !self.graph[block]
                .first_label()
                .is_some_and(|label| copied_entry_labels.contains(&label))
            {
                loads.clear();
                source_loads = 0;
                copied_jump_marker = None;
            }
            for offset in 0..self.graph[block].items().len() {
                let instruction = match self.graph[block].items()[offset] {
                    Item::Label(label) => {
                        if copied_entry_labels.contains(&label) {
                            source_loads = loads.len();
                            copied_jump_marker = loads.last().map(|(position, _, _)| {
                                (
                                    *position,
                                    self.graph.instruction_mut(*position).unwrap().location,
                                )
                            });
                        }
                        continue;
                    }
                    Item::Instruction(instruction) => instruction,
                };
                let position = ItemPosition::new(block, offset);
                let Operand::Value(argument) = instruction.operand() else {
                    loads.clear();
                    source_loads = 0;
                    continue;
                };
                match instruction.opcode {
                    LOAD_CONST | LOAD_SMALL_INT => {
                        loads.push((position, instruction.opcode, argument));
                    }
                    NOP => {}
                    UNARY_NEGATIVE if !loads.is_empty() && loads.len() <= source_loads => {
                        let (operand_position, opcode, argument) = *loads.last().unwrap();
                        let operand = if opcode == LOAD_CONST {
                            constants.get(argument).clone()
                        } else {
                            Constant::Int(u64::from(argument))
                        };
                        let Some(value) = operand.negated() else {
                            loads.clear();
                            source_loads = 0;
                            continue;
                        };
                        let (opcode, argument) = if let Constant::Int(value @ 0..=255) = value {
                            (
                                LOAD_SMALL_INT,
                                u32::try_from(value).expect("small integer fits in u32"),
                            )
                        } else {
                            (LOAD_CONST, constants.intern(value)?)
                        };
                        if let Some((_, LOAD_CONST, index)) = loads.last() {
                            // A removed load still records the operand's earlier construction
                            // if that value survives elsewhere, including small-int results.
                            folded_operand_admissions.insert(operand_position, *index);
                        }
                        let load = self.graph.instruction_mut(operand_position).unwrap();
                        load.opcode = NOP;
                        load.set_operand(Operand::Value(0));
                        load.location = SourceLocation::NONE;
                        load.no_location_policy.preserve_location();
                        if let Some((marker_position, marker_location)) = copied_jump_marker {
                            // The copied-away jump precedes the entire unary chain. An inner
                            // fold on its line can hide the marker until an outer fold moves
                            // the surviving load to another line, so retain its original span.
                            let marker = self.graph.instruction_mut(marker_position).unwrap();
                            marker.location = if marker_location.line == instruction.location.line {
                                SourceLocation::NONE
                            } else {
                                marker_location
                            };
                            marker.no_location_policy.preserve_location();
                        }
                        let unary = self.graph.instruction_mut(position).unwrap();
                        unary.opcode = opcode;
                        unary.set_operand(Operand::Value(argument));
                        loads.pop();
                        loads.push((position, opcode, argument));
                        source_loads = loads.len();
                    }
                    BUILD_TUPLE
                        if argument <= 30
                            && argument as usize <= loads.len()
                            && loads.len() - (argument as usize) < source_loads =>
                    {
                        let first = loads.len() - argument as usize;
                        let values = loads[first..]
                            .iter()
                            .map(|(_, opcode, argument)| {
                                if *opcode == LOAD_CONST {
                                    constants.get(*argument).clone()
                                } else {
                                    Constant::Int(u64::from(*argument))
                                }
                            })
                            .collect();
                        let index = constants.intern(Constant::Tuple(values))?;
                        for (position, _, _) in loads.drain(first..) {
                            let load = self.graph.instruction_mut(position).unwrap();
                            load.opcode = NOP;
                            load.set_operand(Operand::Value(0));
                            load.location = SourceLocation::NONE;
                            load.no_location_policy.preserve_location();
                        }
                        if let Some((marker_position, marker_location)) = copied_jump_marker {
                            // Folding can consume a unary result, but the replaced join's
                            // marker still belongs to its original source operand.
                            let marker = self.graph.instruction_mut(marker_position).unwrap();
                            marker.location = if marker_location.line == instruction.location.line {
                                SourceLocation::NONE
                            } else {
                                marker_location
                            };
                            marker.no_location_policy.preserve_location();
                        }
                        let build = self.graph.instruction_mut(position).unwrap();
                        build.opcode = LOAD_CONST;
                        build.set_operand(Operand::Value(index));
                        loads.push((position, LOAD_CONST, index));
                        source_loads = loads.len();
                    }
                    _ => {
                        loads.clear();
                        source_loads = 0;
                    }
                }
            }
        }
        // Exit copying can reorder the first use of a tuple already folded by code generation,
        // even when it exposes no new tuple to fold here. CPython creates those constants while
        // walking the copied graph, so restore that order for the deferred tuples.
        let mut used = vec![false; constants.len()];
        let mut order = Vec::new();
        for (position, item) in self.graph.order().iter().flat_map(|block| {
            self.graph[*block]
                .items()
                .iter()
                .enumerate()
                .map(move |(offset, item)| (ItemPosition::new(*block, offset), item))
        }) {
            if let Some(index) = folded_operand_admissions.get(&position) {
                order.push(*index);
            }
            if let Item::Label(label) = item
                && let Some(admissions) = self.discarded_fold_admissions.get(label)
            {
                order.extend(admissions);
            }
            if let Item::Instruction(instruction) = item
                && instruction.opcode == LOAD_CONST
                && let Operand::Value(index) = instruction.operand()
            {
                used[index as usize] = true;
                order.push(index);
            }
        }
        order.retain(|index| used[*index as usize]);
        let indices = constants.retain_used(&used, &order);
        self.graph.for_each_instruction_mut(|instruction| {
            if instruction.opcode == LOAD_CONST
                && let Operand::Value(index) = instruction.operand()
            {
                instruction.set_operand(Operand::Value(indices[index as usize].unwrap()));
            }
        });
        Ok(())
    }
}
