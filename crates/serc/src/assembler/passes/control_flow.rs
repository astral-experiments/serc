use rustc_hash::{FxHashMap, FxHashSet};

use super::super::{
    COMPARE_OP, CONTAINS_OP, COPY, ExceptionExclusion, IS_OP, Instruction, Item, JUMP_BACKWARD,
    JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, NOP, NOT_TAKEN, Operand, POP_JUMP_IF_FALSE,
    POP_JUMP_IF_TRUE, POP_TOP, STORE_FAST, TO_BOOL, UNARY_NOT, ends_scope, is_unconditional_jump,
};
use crate::assembler::OptimizationPipeline;
use crate::target::Opcode;
use crate::target::operands::ComparisonOperation;

impl OptimizationPipeline {
    /// Replaces an overwritten local store with a stack pop.
    ///
    /// CPython applies this peephole before superinstruction fusion when two
    /// adjacent stores target the same local on the same traced line.
    pub(in crate::assembler) fn optimize_redundant_store_fast(&mut self) {
        let graph = &mut self.graph;
        let order = graph.order().to_vec();
        for block in order {
            let items = graph[block].items_mut();
            for index in 0..items.len().saturating_sub(1) {
                let [Item::Instruction(first), Item::Instruction(second)] =
                    &mut items[index..index + 2]
                else {
                    continue;
                };
                if first.opcode == STORE_FAST
                    && second.opcode == STORE_FAST
                    && matches!(
                        (first.operand, second.operand),
                        (Operand::Value(first), Operand::Value(second)) if first == second
                    )
                    && first.location.line == second.location.line
                {
                    first.opcode = POP_TOP;
                    first.operand = Operand::Value(0);
                }
            }
        }
    }

    pub(in crate::assembler) fn remove_unreachable_instructions(&mut self) {
        let graph = &mut self.graph;
        let reachable = graph.reachable_blocks(&self.exception_regions);
        let order = graph.order().to_vec();
        let mut empty = FxHashSet::default();
        for block in order {
            if reachable[block.index()] {
                continue;
            }
            graph[block].items_mut().retain(|item| match item {
                Item::Label(_) => true,
                Item::Instruction(_) => false,
            });
            if graph[block].items().is_empty() {
                empty.insert(block);
            }
        }
        if !empty.is_empty() {
            graph.retain_order(|block| !empty.contains(&block));
        }
    }

    #[cfg(any(test, debug_assertions))]
    pub(in crate::assembler) fn reachable_items(&self) -> Vec<bool> {
        self.graph.item_reachability(&self.exception_regions)
    }

    pub(in crate::assembler) fn optimize_boolean_conversions(&mut self) {
        let graph = &mut self.graph;
        let order = graph.order().to_vec();
        for block in order {
            let items = graph[block].items_mut();
            for index in 0..items.len().saturating_sub(1) {
                let [Item::Instruction(current), Item::Instruction(next)] =
                    &mut items[index..index + 2]
                else {
                    continue;
                };
                match (current.opcode, next.opcode) {
                    (TO_BOOL, TO_BOOL) => {
                        current.opcode = NOP;
                        current.operand = Operand::Value(0);
                        continue;
                    }
                    (UNARY_NOT, TO_BOOL) => {
                        current.opcode = NOP;
                        current.operand = Operand::Value(0);
                        next.opcode = UNARY_NOT;
                        next.operand = Operand::Value(0);
                        continue;
                    }
                    (UNARY_NOT, UNARY_NOT) => {
                        current.opcode = NOP;
                        current.operand = Operand::Value(0);
                        next.opcode = NOP;
                        next.operand = Operand::Value(0);
                        continue;
                    }
                    (CONTAINS_OP | IS_OP, UNARY_NOT) => {
                        let Operand::Value(argument) = current.operand else {
                            continue;
                        };
                        let opcode = current.opcode;
                        current.opcode = NOP;
                        current.operand = Operand::Value(0);
                        next.opcode = opcode;
                        next.operand = Operand::Value(argument ^ 1);
                        continue;
                    }
                    _ => {}
                }
                if next.opcode != TO_BOOL {
                    continue;
                }
                let Operand::Value(argument) = current.operand else {
                    continue;
                };
                let replacement = match current.opcode {
                    COMPARE_OP => Some((COMPARE_OP, ComparisonOperation::force_boolean(argument))),
                    CONTAINS_OP => Some((CONTAINS_OP, argument)),
                    IS_OP => Some((IS_OP, argument)),
                    _ => None,
                };
                let Some((opcode, argument)) = replacement else {
                    continue;
                };
                current.opcode = NOP;
                current.operand = Operand::Value(0);
                next.opcode = opcode;
                next.operand = Operand::Value(argument);
            }
        }
    }

    pub(in crate::assembler) fn thread_forward_jumps(&mut self) {
        #[derive(Clone, Copy)]
        struct ConditionalTarget {
            opcode: Opcode,
            operand: Operand,
            fallthrough_index: usize,
        }

        fn is_value_preserving_jump(items: &[Item], index: usize) -> bool {
            let Item::Instruction(instruction) = items[index] else {
                return false;
            };
            if !matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                || !matches!(instruction.operand, Operand::Forward(_))
            {
                return false;
            }
            let mut previous = items[..index].iter().rev().filter_map(|item| {
                let Item::Instruction(instruction) = item else {
                    return None;
                };
                (instruction.opcode != NOP).then_some(instruction)
            });
            let Some(to_bool) = previous.next() else {
                return false;
            };
            let Some(copy) = previous.next() else {
                return false;
            };
            to_bool.opcode == TO_BOOL
                && copy.opcode == COPY
                && matches!(copy.operand, Operand::Value(1))
        }

        // CPython threads the value-preserving jumps used by boolean
        // expressions through a nested boolean test. If both tests jump on
        // the same truth value, they share the final target. Otherwise the
        // inner jump skips the redundant outer test and enters its POP_TOP
        // fallthrough path directly.
        let graph = &self.graph;
        let mut positions = graph.item_positions();
        let mut items = positions
            .iter()
            .map(|position| *graph.item(*position))
            .collect::<Vec<_>>();
        let mut conditional_targets = FxHashMap::default();
        for (index, item) in items.iter().enumerate() {
            let Item::Label(label) = item else {
                continue;
            };
            let following = items[index + 1..]
                .iter()
                .enumerate()
                .filter_map(|(offset, item)| {
                    let Item::Instruction(instruction) = item else {
                        return None;
                    };
                    (instruction.opcode != NOP).then_some((index + 1 + offset, *instruction))
                })
                .take(5)
                .collect::<Vec<_>>();
            let [
                (_, copy),
                (_, to_bool),
                (_, jump),
                (_, not_taken),
                (fallthrough_index, pop),
            ] = following.as_slice()
            else {
                continue;
            };
            if copy.opcode != COPY
                || !matches!(copy.operand, Operand::Value(1))
                || to_bool.opcode != TO_BOOL
                || !matches!(jump.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                || !matches!(jump.operand, Operand::Forward(_))
                || not_taken.opcode != NOT_TAKEN
                || pop.opcode != POP_TOP
            {
                continue;
            }
            conditional_targets.insert(
                *label,
                ConditionalTarget {
                    opcode: jump.opcode,
                    operand: jump.operand,
                    fallthrough_index: *fallthrough_index,
                },
            );
        }

        let mut fallthrough_indices = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let Item::Instruction(instruction) = item else {
                    return None;
                };
                if !is_value_preserving_jump(&items, index) {
                    return None;
                }
                let Operand::Forward(target) = instruction.operand else {
                    return None;
                };
                let target = conditional_targets.get(&target)?;
                (instruction.opcode != target.opcode).then_some(target.fallthrough_index)
            })
            .collect::<Vec<_>>();
        fallthrough_indices.sort_unstable();
        fallthrough_indices.dedup();
        let fallthrough_labels = fallthrough_indices
            .iter()
            .map(|index| (*index, self.label()))
            .collect::<FxHashMap<_, _>>();
        {
            let graph = &mut self.graph;
            for index in fallthrough_indices.into_iter().rev() {
                graph.insert_item(positions[index], Item::Label(fallthrough_labels[&index]));
            }
        }

        let graph = &self.graph;
        positions = graph.item_positions();
        items = positions
            .iter()
            .map(|position| *graph.item(*position))
            .collect();
        let value_preserving_jumps = (0..items.len())
            .filter(|index| is_value_preserving_jump(&items, *index))
            .collect::<FxHashSet<_>>();
        let graph = &mut self.graph;
        for (index, position) in positions.iter().copied().enumerate() {
            let Item::Instruction(instruction) = graph.item_mut(position) else {
                continue;
            };
            if !value_preserving_jumps.contains(&index) {
                continue;
            }
            if !matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE) {
                continue;
            }
            let Operand::Forward(mut target) = instruction.operand else {
                continue;
            };
            for _ in 0..conditional_targets.len() {
                let Some(next) = conditional_targets.get(&target) else {
                    break;
                };
                if instruction.opcode == next.opcode {
                    instruction.retarget(next.operand);
                    let Operand::Forward(next_target) = next.operand else {
                        break;
                    };
                    if next_target == target {
                        break;
                    }
                    target = next_target;
                } else {
                    let Some(fallthrough) = fallthrough_labels.get(&next.fallthrough_index) else {
                        break;
                    };
                    instruction.retarget(Operand::Forward(*fallthrough));
                    break;
                }
            }
        }

        let graph = &self.graph;
        items = graph.iter_items().copied().collect();
        let mut jump_targets = FxHashMap::default();
        for (index, item) in items.iter().enumerate() {
            let Item::Label(label) = item else {
                continue;
            };
            let Some(Item::Instruction(instruction)) = items[index + 1..]
                .iter()
                .find(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            // A jump retained as a source-position NOP remains a CFG boundary.
            if instruction.jump_policy.allows_threading_target()
                && !instruction.jump_policy.always_retains_nop()
                && is_unconditional_jump(instruction.opcode)
                && let operand @ (Operand::Forward(_) | Operand::Backward(_)) = instruction.operand
            {
                jump_targets.insert(*label, (operand, instruction.opcode));
            }
        }

        let graph = &mut self.graph;
        let order = graph.order().to_vec();
        for block in order {
            for item in graph[block].items_mut() {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                if instruction.opcode != JUMP_FORWARD
                    || instruction.jump_policy.always_retains_nop()
                {
                    continue;
                }
                let Operand::Forward(mut target) = instruction.operand else {
                    continue;
                };
                let mut operand = instruction.operand;
                let mut opcode = instruction.opcode;
                for _ in 0..jump_targets.len() {
                    let Some((next_operand, next_opcode)) = jump_targets.get(&target).copied()
                    else {
                        break;
                    };
                    let next = match next_operand {
                        Operand::Forward(next) | Operand::Backward(next) => next,
                        Operand::Value(_) => unreachable!(),
                    };
                    if next == target {
                        break;
                    }
                    operand = next_operand;
                    if next_opcode == JUMP_BACKWARD {
                        opcode = JUMP_BACKWARD;
                    } else if next_opcode == JUMP_BACKWARD_NO_INTERRUPT {
                        opcode = JUMP_BACKWARD_NO_INTERRUPT;
                    }
                    target = next;
                }
                instruction.retarget(operand);
                instruction.opcode = opcode;
            }
        }
    }

    pub(in crate::assembler) fn remove_redundant_forward_jumps(&mut self) {
        let graph = &mut self.graph;
        if !graph.iter_items().any(|item| {
            matches!(
                item,
                Item::Instruction(instruction)
                    if instruction.opcode == JUMP_FORWARD
                        && matches!(instruction.operand, Operand::Forward(_))
            )
        }) {
            return;
        }
        let mut items = graph
            .iter()
            .flat_map(|(block, contents)| {
                contents
                    .items()
                    .iter()
                    .copied()
                    .map(move |item| (block, item))
            })
            .collect::<Vec<_>>();
        let mut index = 0;
        while index < items.len() {
            let Item::Instruction(instruction) = items[index].1 else {
                index += 1;
                continue;
            };
            let Operand::Forward(target) = instruction.operand else {
                index += 1;
                continue;
            };
            if instruction.opcode != JUMP_FORWARD {
                index += 1;
                continue;
            }
            if instruction.jump_policy.defers_redundant_removal() {
                let Item::Instruction(instruction) = &mut items[index].1 else {
                    unreachable!();
                };
                instruction.jump_policy.resolve_deferred_removal();
                index += 1;
                continue;
            }
            let target_is_next = items[index + 1..]
                .iter()
                .map(|(_, item)| item)
                .take_while(|item| matches!(item, Item::Label(_)))
                .any(|item| matches!(item, Item::Label(label) if *label == target));
            if !target_is_next {
                index += 1;
                continue;
            }
            let previous = items[..index]
                .iter()
                .rposition(|(_, item)| matches!(item, Item::Instruction(_)));
            let previous_line = previous.map(|previous| {
                let Item::Instruction(instruction) = items[previous].1 else {
                    unreachable!();
                };
                instruction.location.line
            });
            let next = items[index + 1..]
                .iter()
                .position(|(_, item)| matches!(item, Item::Instruction(_)))
                .map(|next| index + 1 + next);
            let next_line = next.map(|next| {
                let Item::Instruction(instruction) = items[next].1 else {
                    unreachable!();
                };
                instruction.location.line
            });
            if !instruction.jump_policy.always_retains_nop()
                && (instruction.location.line < 0
                    || previous_line == Some(instruction.location.line)
                    || next_line == Some(instruction.location.line))
            {
                items.remove(index);
                if instruction.exception_policy.exclusion == ExceptionExclusion::IfExtended
                    && let Some(previous) = previous
                {
                    let Item::Instruction(previous) = &mut items[previous].1 else {
                        unreachable!();
                    };
                    previous
                        .exception_policy
                        .exclude(ExceptionExclusion::IfExtended);
                }
                if instruction.location.line >= 0
                    && let Some(next) = next.map(|next| next - 1)
                    && let Item::Instruction(next) = &mut items[next].1
                    && next.location.line < 0
                    && !next.no_location_policy.preserve
                    && !next.no_location_policy.allow_block_inlining
                {
                    next.location = instruction.location;
                }
            } else {
                let Item::Instruction(instruction) = &mut items[index].1 else {
                    unreachable!();
                };
                instruction.opcode = NOP;
                instruction.operand = Operand::Value(0);
                index += 1;
            }
        }

        let order = graph.order().to_vec();
        for block in order {
            graph[block].items_mut().clear();
        }
        for (block, item) in items {
            graph[block].items_mut().push(item);
        }
    }

    pub(in crate::assembler) fn propagate_locations_within_blocks(&mut self) {
        let graph = &mut self.graph;
        let order = graph.order().to_vec();
        for block in order {
            let mut previous = None;
            let mut block_has_instruction = false;
            for item in graph[block].items_mut() {
                match item {
                    Item::Label(_) if block_has_instruction => {
                        previous = None;
                        block_has_instruction = false;
                    }
                    Item::Label(_) => {}
                    Item::Instruction(instruction) => {
                        block_has_instruction = true;
                        if instruction.location.line < 0 && !instruction.no_location_policy.preserve
                        {
                            if let Some(location) = previous {
                                instruction.location = location;
                            }
                        } else {
                            previous = Some(instruction.location);
                        }
                        if !matches!(instruction.operand, Operand::Value(_))
                            || ends_scope(instruction.opcode)
                        {
                            previous = None;
                            block_has_instruction = false;
                        }
                    }
                }
            }
        }
    }

    pub(in crate::assembler) fn remove_redundant_nops(&mut self) {
        let graph = &self.graph;
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
            let remove = instruction.opcode == NOP
                && (instruction.location.line < 0
                    || (instruction.converted_pop_block
                        && previous.is_some_and(|previous: Instruction| {
                            previous.location.line == instruction.location.line
                                && !(ends_scope(previous.opcode)
                                    || is_unconditional_jump(previous.opcode))
                        }))
                    || instructions
                        .get(index + 1)
                        .is_some_and(|(_, next)| next.location.line == instruction.location.line));
            if remove {
                redundant.push(position);
            } else {
                previous = Some(instruction);
            }
        }

        let graph = &mut self.graph;
        for position in redundant.into_iter().rev() {
            graph.remove_item(position);
        }
    }
}
