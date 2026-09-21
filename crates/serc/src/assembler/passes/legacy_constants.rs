use super::super::block_graph::{BlockGraph, ItemPosition};
use super::super::{ConstantTable, Instruction, Item, Operand, OptimizationPipeline};
use crate::code_object::Constant;
use crate::target::opcodes::{
    BUILD_TUPLE, COPY, IS_OP, JUMP_BACKWARD, JUMP_FORWARD, KW_NAMES, LOAD_CONST, NOP,
    POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, RETURN_CONST,
    RETURN_VALUE, SWAP, TO_BOOL, UNPACK_SEQUENCE,
};
use crate::{CompileError, PythonVersion};

impl OptimizationPipeline {
    /// Applies the constant-load cases from CPython 3.12 and 3.13's CFG optimizer.
    pub(super) fn optimize_legacy_load_constants(
        &mut self,
        constants: &mut dyn ConstantTable,
    ) -> Result<(), CompileError> {
        if !matches!(
            self.python_version,
            PythonVersion::Py312 | PythonVersion::Py313
        ) {
            return Ok(());
        }
        for block in self.graph.order().to_vec() {
            let positions = self.graph[block]
                .items()
                .iter()
                .enumerate()
                .filter_map(|(offset, item)| {
                    matches!(item, Item::Instruction(_)).then_some(ItemPosition::new(block, offset))
                })
                .collect::<Vec<_>>();
            let mut constant = None;
            let mut index = 0;
            while index < positions.len() {
                let position = positions[index];
                let instruction = *self.graph.instruction_mut(position).unwrap();
                if instruction.opcode == LOAD_CONST {
                    constant = match instruction.operand() {
                        Operand::Value(argument) => Some(argument),
                        _ => None,
                    };
                } else if instruction.opcode != COPY || instruction.operand() != Operand::Value(1) {
                    constant = None;
                }
                let Some(argument) = constant else {
                    // 3.12 folds tuples in this same forward scan. In 3.13 they
                    // run in a separate pass, after every block's constant loads.
                    if self.python_version == PythonVersion::Py312
                        && instruction.opcode == BUILD_TUPLE
                    {
                        fold_legacy_tuple(&mut self.graph, &positions, index, constants)?;
                    }
                    index += 1;
                    continue;
                };
                let Some(&next_position) = positions.get(index + 1) else {
                    break;
                };
                let next = *self.graph.instruction_mut(next_position).unwrap();
                match next.opcode {
                    POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE => {
                        let truth = legacy_constant_truthiness(constants.get(argument));
                        legacy_nop(self.graph.instruction_mut(position).unwrap());
                        let jump = self.graph.instruction_mut(next_position).unwrap();
                        if truth == (next.opcode == POP_JUMP_IF_TRUE) {
                            jump.opcode = if matches!(jump.operand(), Operand::Backward(_)) {
                                JUMP_BACKWARD
                            } else {
                                JUMP_FORWARD
                            };
                            if self.python_version == PythonVersion::Py313 {
                                // Taken constant conditions become native JUMP,
                                // even while its target still lies forward.
                                jump.jump_policy.mark_interruptible_after_threading();
                            }
                        } else {
                            legacy_nop(jump);
                        }
                    }
                    IS_OP if matches!(constants.get(argument), Constant::None) => {
                        let mut jump_index = index + 2;
                        if let Some(&jump_position) = positions.get(jump_index) {
                            let jump = self.graph.instruction_mut(jump_position).unwrap();
                            if self.python_version == PythonVersion::Py313 && jump.opcode == TO_BOOL
                            {
                                // IS_OP already produces a bool, even when the
                                // following instruction is not a conditional jump.
                                legacy_nop(jump);
                                jump_index += 1;
                            }
                        }
                        if let Some(&jump_position) = positions.get(jump_index) {
                            let jump = self.graph.instruction_mut(jump_position).unwrap();
                            if matches!(jump.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE) {
                                let invert = next.operand() != Operand::Value(0);
                                jump.opcode = if invert ^ (jump.opcode == POP_JUMP_IF_FALSE) {
                                    POP_JUMP_IF_NOT_NONE
                                } else {
                                    POP_JUMP_IF_NONE
                                };
                                legacy_nop(self.graph.instruction_mut(position).unwrap());
                                legacy_nop(self.graph.instruction_mut(next_position).unwrap());
                            }
                        }
                    }
                    RETURN_VALUE => {
                        legacy_nop(self.graph.instruction_mut(position).unwrap());
                        let return_instruction = self.graph.instruction_mut(next_position).unwrap();
                        return_instruction.opcode = RETURN_CONST;
                        return_instruction.set_operand(Operand::Value(argument));
                        index += 1;
                    }
                    TO_BOOL if self.python_version == PythonVersion::Py313 => {
                        let truth = legacy_constant_truthiness(constants.get(argument));
                        let argument = constants.intern(Constant::Bool(truth))?;
                        legacy_nop(self.graph.instruction_mut(position).unwrap());
                        let conversion = self.graph.instruction_mut(next_position).unwrap();
                        conversion.opcode = LOAD_CONST;
                        conversion.set_operand(Operand::Value(argument));
                    }
                    _ => {}
                }
                index += 1;
            }
        }
        Ok(())
    }

    /// Folds 3.13 tuples after constant loads; 3.12 folds them in the preceding scan.
    pub(super) fn optimize_legacy_constant_tuples(
        &mut self,
        constants: &mut dyn ConstantTable,
    ) -> Result<(), CompileError> {
        if self.python_version != PythonVersion::Py313 {
            return Ok(());
        }
        for block in self.graph.order().to_vec() {
            let positions = self.graph[block]
                .items()
                .iter()
                .enumerate()
                .filter_map(|(offset, item)| {
                    matches!(item, Item::Instruction(_)).then_some(ItemPosition::new(block, offset))
                })
                .collect::<Vec<_>>();
            for index in 0..positions.len() {
                fold_legacy_tuple(&mut self.graph, &positions, index, constants)?;
            }
        }
        Ok(())
    }

    /// Removes unused legacy constants without changing their admission order.
    pub(super) fn prune_legacy_constants(&mut self, constants: &mut dyn ConstantTable) {
        if !matches!(
            self.python_version,
            PythonVersion::Py312 | PythonVersion::Py313
        ) {
            return;
        }
        let mut used = vec![false; constants.len()];
        if let Some(first) = used.first_mut() {
            // Slot zero can hold a docstring even when no instruction loads it.
            *first = true;
        }
        for item in self.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && matches!(instruction.opcode, LOAD_CONST | KW_NAMES | RETURN_CONST)
                && let Operand::Value(index) = instruction.operand()
            {
                used[index as usize] = true;
            }
        }
        let indices = constants.retain_used_in_order(&used);
        self.graph.for_each_instruction_mut(|instruction| {
            if matches!(instruction.opcode, LOAD_CONST | KW_NAMES | RETURN_CONST)
                && let Operand::Value(index) = instruction.operand()
            {
                instruction.set_operand(Operand::Value(indices[index as usize].unwrap()));
            }
        });
    }
}

/// Keeps the original location for CPython's later redundant-NOP cleanup.
fn legacy_nop(instruction: &mut Instruction) {
    instruction.opcode = NOP;
    instruction.set_operand(Operand::Value(0));
}

fn legacy_constant_truthiness(constant: &Constant) -> bool {
    match constant {
        Constant::None => false,
        Constant::Bool(value) => *value,
        Constant::Ellipsis | Constant::Slice { .. } | Constant::Code(_) => true,
        Constant::Int(value) => *value != 0,
        Constant::SignedInt(value) => *value != 0,
        Constant::BigInt { digits, .. } => !digits.is_empty(),
        Constant::Float(value) => *value != 0.0,
        Constant::Complex { real, imag } => *real != 0.0 || *imag != 0.0,
        Constant::String(value) => !value.is_empty(),
        Constant::SurrogateString(value) | Constant::Bytes(value) => !value.is_empty(),
        Constant::Tuple(value) | Constant::FrozenSet(value) => !value.is_empty(),
    }
}

/// Folds only adjacent loads; NOPs still occupy native CFG instruction slots.
fn fold_legacy_tuple(
    graph: &mut BlockGraph,
    positions: &[ItemPosition],
    index: usize,
    constants: &mut dyn ConstantTable,
) -> Result<(), CompileError> {
    let position = positions[index];
    let instruction = *graph.instruction_mut(position).unwrap();
    if instruction.opcode != BUILD_TUPLE {
        return Ok(());
    }
    let Operand::Value(count) = instruction.operand() else {
        return Ok(());
    };
    if let Some(&next_position) = positions.get(index + 1) {
        let next = graph.instruction_mut(next_position).unwrap();
        if next.opcode == UNPACK_SEQUENCE
            && next.operand() == Operand::Value(count)
            && matches!(count, 1..=3)
        {
            if count == 1 {
                legacy_nop(next);
            } else {
                next.opcode = SWAP;
            }
            legacy_nop(graph.instruction_mut(position).unwrap());
            return Ok(());
        }
    }
    let Some(start) = index.checked_sub(count as usize) else {
        return Ok(());
    };
    let mut values = Vec::with_capacity(count as usize);
    for &load_position in &positions[start..index] {
        let load = graph.instruction_mut(load_position).unwrap();
        if load.opcode != LOAD_CONST {
            return Ok(());
        }
        let Operand::Value(argument) = load.operand() else {
            return Ok(());
        };
        values.push(constants.get(argument).clone());
    }
    let argument = constants.fold_sequence(values, false)?;
    for &load_position in &positions[start..index] {
        legacy_nop(graph.instruction_mut(load_position).unwrap());
    }
    let tuple = graph.instruction_mut(position).unwrap();
    tuple.opcode = LOAD_CONST;
    tuple.set_operand(Operand::Value(argument));
    Ok(())
}
