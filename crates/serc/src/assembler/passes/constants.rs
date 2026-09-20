use rustc_hash::{FxHashMap, FxHashSet};

use super::super::block_graph::ItemPosition;
use super::super::{ConstantTable, Item, Label, Operand, OptimizationPipeline, SourceLocation};
use crate::code_object::Constant;
use crate::target::Opcode;
use crate::target::opcodes::{
    BUILD_LIST, BUILD_SET, BUILD_TUPLE, CALL_INTRINSIC_1, CONTAINS_OP, GET_ITER, LIST_APPEND,
    LOAD_COMMON_CONSTANT, LOAD_CONST, LOAD_SMALL_INT, NOP, SET_ADD, UNARY_NEGATIVE,
};
use crate::target::operands::UnaryIntrinsic;
use crate::target::operands315::CommonConstant;
use crate::{CompileError, PythonVersion};

impl OptimizationPipeline {
    /// Folds 3.15's append-built constant sequences without crossing basic blocks.
    pub(super) fn fold_constant_sequences(
        &mut self,
        constants: &mut dyn ConstantTable,
    ) -> Result<Option<u32>, CompileError> {
        if self.python_version != PythonVersion::Py315 {
            return Ok(None);
        }
        let mut removed_max_depth = None;
        for block in self.graph.order().to_vec() {
            for offset in 0..self.graph[block].items().len() {
                let Item::Instruction(target) = self.graph[block].items()[offset] else {
                    continue;
                };
                let intrinsic = target.opcode == CALL_INTRINSIC_1
                    && target.operand() == Operand::Value(UnaryIntrinsic::LIST_TO_TUPLE.argument());
                let append = matches!(target.opcode, LIST_APPEND | SET_ADD)
                    && target.operand() == Operand::Value(1)
                    && matches!(
                        self.graph[block].items().get(offset + 1),
                        Some(Item::Instruction(next)) if matches!(next.opcode, GET_ITER | CONTAINS_OP)
                    );
                if !intrinsic && !append {
                    continue;
                }
                let is_set = target.opcode == SET_ADD;
                let build_opcode = if is_set { BUILD_SET } else { BUILD_LIST };
                let append_opcode = if is_set { SET_ADD } else { LIST_APPEND };
                let mut expect_append = intrinsic;
                let mut values = Vec::new();
                let mut start = None;
                for previous in (0..offset).rev() {
                    let Item::Instruction(instruction) = self.graph[block].items()[previous] else {
                        break;
                    };
                    if instruction.opcode == NOP {
                        continue;
                    }
                    if instruction.opcode == build_opcode
                        && instruction.operand() == Operand::Value(0)
                    {
                        if expect_append {
                            start = Some(previous);
                        }
                        break;
                    }
                    if expect_append {
                        if instruction.opcode != append_opcode
                            || instruction.operand() != Operand::Value(1)
                        {
                            break;
                        }
                    } else {
                        let Operand::Value(argument) = instruction.operand() else {
                            break;
                        };
                        match instruction.opcode {
                            LOAD_CONST => values.push(constants.get(argument).clone()),
                            LOAD_SMALL_INT => values.push(Constant::Int(u64::from(argument))),
                            _ => break,
                        }
                    }
                    expect_append = !expect_append;
                }
                let Some(start) = start else {
                    continue;
                };
                values.reverse();
                let index = constants.fold_sequence(values, is_set)?;
                // CPython nops the final append as well, clearing its location before
                // replacing it. The optional intrinsic instead keeps its original span.
                let end = if intrinsic { offset } else { offset + 1 };
                for previous in start..end {
                    let instruction = self
                        .graph
                        .instruction_mut(ItemPosition::new(block, previous))
                        .unwrap();
                    if instruction.opcode != NOP {
                        let depth = instruction.depth_after().unwrap_or(0);
                        removed_max_depth = Some(
                            removed_max_depth.map_or(depth, |current: u32| current.max(depth)),
                        );
                        instruction.opcode = NOP;
                        instruction.set_operand(Operand::Value(0));
                        instruction.location = SourceLocation::NONE;
                        instruction.no_location_policy.clear_preserved_location();
                    }
                }
                let target = self
                    .graph
                    .instruction_mut(ItemPosition::new(block, offset))
                    .unwrap();
                target.opcode = LOAD_CONST;
                target.set_operand(Operand::Value(index));
            }
        }
        Ok(removed_max_depth)
    }

    /// Lowers 3.15's immortal constants after value-based graph optimizations.
    pub(super) fn lower_common_constants(&mut self, constants: &mut dyn ConstantTable) {
        if self.python_version != PythonVersion::Py315 {
            return;
        }
        let mut changed = false;
        self.graph.for_each_instruction_mut(|instruction| {
            if instruction.opcode != LOAD_CONST {
                return;
            }
            let Operand::Value(index) = instruction.operand() else {
                return;
            };
            let common = match constants.get(index) {
                Constant::None => CommonConstant::NONE,
                Constant::Bool(true) => CommonConstant::TRUE,
                Constant::Bool(false) => CommonConstant::FALSE,
                Constant::String(value) if value.is_empty() => CommonConstant::EMPTY_STR,
                Constant::SignedInt(-1) => CommonConstant::MINUS_ONE,
                _ => return,
            };
            instruction.opcode = LOAD_COMMON_CONSTANT;
            instruction.set_operand(Operand::Value(common.argument()));
            changed = true;
        });
        if !changed {
            return;
        }

        // Code generation still admits these constants in source order. Only the
        // final constant table drops their unused slots; slot zero remains reserved.
        let mut used = vec![false; constants.len()];
        for item in self.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && instruction.opcode == LOAD_CONST
                && let Operand::Value(index) = instruction.operand()
            {
                used[index as usize] = true;
            }
        }
        // Earlier folding already established admission order, including values
        // first constructed by expressions whose loads were subsequently discarded.
        let order = used
            .iter()
            .enumerate()
            .filter_map(|(index, used)| used.then_some(u32::try_from(index).unwrap()))
            .collect::<Vec<_>>();
        let indices = constants.retain_used(&used, &order);
        self.graph.for_each_instruction_mut(|instruction| {
            if instruction.opcode == LOAD_CONST
                && let Operand::Value(index) = instruction.operand()
            {
                instruction.set_operand(Operand::Value(indices[index as usize].unwrap()));
            }
        });
    }

    /// Folds constant operations exposed by exit copying, preserving basic-block boundaries.
    pub(super) fn fold_copied_constants(
        &mut self,
        constants: &mut dyn ConstantTable,
        copied_entry_labels: &[Label],
    ) -> Result<(), CompileError> {
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

#[cfg(test)]
mod tests {
    use crate::code_object::Constant;
    use crate::target::opcodes::{LIST_APPEND, LOAD_COMMON_CONSTANT, SET_ADD};
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    #[test]
    fn python315_common_constants_preserve_first_slot_and_folded_containers() {
        let source =
            "a = None\nb = True\nc = False\nd = ''\ne = -1\nf = 1000\ng = (None, False, '', -1)\n";
        for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
            let module = compile_with_options(
                source,
                "constants.py",
                &CompileOptions {
                    python_version,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let arguments = module
                .bytecode()
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|word| word[0] == LOAD_COMMON_CONSTANT.code_for(python_version))
                .map(|word| word[1])
                .collect::<Vec<_>>();
            if python_version == PythonVersion::Py315 {
                assert_eq!(arguments, [7, 9, 10, 8, 11, 7]);
                assert!(matches!(
                    module.code.constants.as_slice(),
                    [Constant::None, Constant::Int(1000), Constant::Tuple(values)]
                        if matches!(values.as_slice(), [Constant::None, Constant::Bool(false), Constant::String(empty), Constant::SignedInt(-1)] if empty.is_empty())
                ));
            } else {
                assert!(arguments.is_empty());
                assert_eq!(module.code.constants.len(), 7);
            }
        }
    }

    #[test]
    fn python315_common_constant_pruning_preserves_discarded_fold_admission() {
        let module = compile_with_options(
            "()\nx = (1, 2)\nf(**kw)\n",
            "discarded_constant.py",
            &CompileOptions {
                python_version: PythonVersion::Py315,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        assert!(matches!(
            module.code.constants.as_slice(),
            [Constant::Int(1), Constant::Tuple(empty), Constant::Tuple(pair)]
                if empty.is_empty() && matches!(pair.as_slice(), [Constant::Int(1), Constant::Int(2)])
        ));
    }

    #[test]
    fn python315_large_sequence_folding_preserves_constant_admission_order() {
        let elements = std::iter::repeat_n("'same value', 10000", 16)
            .collect::<Vec<_>>()
            .join(", ");
        let source =
            format!("first = 10000\nfor item in [{elements}]:\n    pass\nlast = (20000, 20001)\n");
        for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
            let module = compile_with_options(
                &source,
                "large_sequence.py",
                &CompileOptions {
                    python_version,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let append_count = module
                .bytecode()
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|word| word[0] == LIST_APPEND.code_for(python_version))
                .count();
            if python_version == PythonVersion::Py315 {
                assert_eq!(append_count, 0);
                assert!(matches!(
                    module.code.constants.as_slice(),
                    [Constant::Int(10000), Constant::Tuple(values), Constant::Tuple(last)]
                        if values.len() == 32
                            && values.as_chunks::<2>().0.iter().all(|pair|
                                matches!(pair, [Constant::String(value), Constant::Int(10000)] if value == "same value"))
                            && matches!(last.as_slice(), [Constant::Int(20000), Constant::Int(20001)])
                ));
            } else {
                assert_eq!(append_count, 32);
            }
        }
    }

    #[test]
    fn python315_large_frozenset_uses_python_equality_and_records_provenance() {
        let elements = std::iter::repeat_n("True, 1, 1.0, -0.0, 0, 0.0, 0j", 5)
            .collect::<Vec<_>>()
            .join(", ");
        let module = compile_with_options(
            &format!("result = item in {{{elements}}}\n"),
            "large_frozenset.py",
            &CompileOptions {
                python_version: PythonVersion::Py315,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        assert!(matches!(
            module.code.constants.as_slice(),
            [Constant::Bool(true), Constant::FrozenSet(values)]
                if matches!(values.as_slice(), [Constant::Bool(true), Constant::Float(value)] if value.to_bits() == (-0.0_f64).to_bits())
        ));
        assert_eq!(module.code.stack_size, 2);
        assert_eq!(
            module
                .code
                .marshal_provenance
                .subtree_has_frozenset_constants,
            Some(true)
        );
        assert!(
            !module
                .bytecode()
                .as_chunks::<2>()
                .0
                .iter()
                .any(|word| word[0] == SET_ADD.code_for(PythonVersion::Py315))
        );
    }
}
