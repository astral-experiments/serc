use super::super::block_graph::BasicBlockId;
use super::super::{Item, Operand, OptimizationPipeline, ends_scope, opcode_stack_effect};
use crate::CompileError;
use crate::target::is_unconditional_jump;
use crate::target::opcodes::END_ASYNC_FOR;

#[derive(Clone, Copy)]
struct ExceptionEdge {
    start: usize,
    end: usize,
    target: BasicBlockId,
    depth: i64,
}

#[derive(Clone, Copy)]
struct BlockState {
    position: usize,
    start_depth: i64,
}

const UNREACHED_DEPTH: i64 = i64::MIN;

impl OptimizationPipeline {
    /// Calculates CPython's physical value-stack size from the final reachable CFG.
    ///
    /// CPython performs the equivalent traversal after CFG optimization. Serc's exception
    /// regions replace CPython's `SETUP_*` pseudo-instructions, so their targets are explicit
    /// exceptional edges whose entry depth includes the exception and optional `lasti` values.
    pub(in crate::assembler) fn calculate_stack_size(&self) -> Result<u32, CompileError> {
        let graph = &self.graph;
        let Some(entry) = graph.order().first().copied() else {
            return Ok(0);
        };
        let arena_len = graph
            .order()
            .iter()
            .fold(0, |len, block| len.max(block.index() + 1));
        let mut block_states = vec![
            BlockState {
                position: usize::MAX,
                start_depth: UNREACHED_DEPTH,
            };
            arena_len
        ];
        for (position, block) in graph.order().iter().copied().enumerate() {
            block_states[block.index()].position = position;
        }
        let exception_edges = self
            .exception_regions
            .iter()
            .enumerate()
            .map(|(index, region)| {
                let block = |label, role| {
                    graph.label_block(label).ok_or_else(|| {
                        stack_error(format!(
                            "exception region {index} has an unbound {role} label {}",
                            label.0
                        ))
                    })
                };
                let start_block = block(region.start, "start")?;
                let end_block = block(region.end, "end")?;
                let start = block_states[start_block.index()].position;
                let end = block_states[end_block.index()].position;
                if start > end {
                    return Err(stack_error(format!(
                        "exception region {index} starts after it ends"
                    )));
                }
                Ok(ExceptionEdge {
                    start,
                    end,
                    target: block(region.target, "target")?,
                    depth: i64::from(region.depth) + 1 + i64::from(region.preserve_lasti),
                })
            })
            .collect::<Result<Vec<_>, CompileError>>()?;
        let mut pending = Vec::with_capacity(graph.len());
        push_start_depth(&mut block_states, &mut pending, entry, 0)?;

        let mut maximum = 0_i64;
        while let Some(block) = pending.pop() {
            let position = block_states[block.index()].position;
            // A real exception can enter the innermost handler from any reachable instruction in
            // the final half-open exception-table range.
            for edge in exception_edges
                .iter()
                .filter(|edge| edge.start <= position && position < edge.end)
            {
                push_start_depth(&mut block_states, &mut pending, edge.target, edge.depth)?;
            }

            let mut depth = block_states[block.index()].start_depth;
            let mut has_fallthrough = true;
            for item in graph[block].items() {
                let instruction = match item {
                    Item::Label(label) => {
                        // CPython calculates stack depth before converting its `SETUP_*` pseudo-
                        // instructions. Match the retained edge only when this exact emitted label
                        // is reached at its original depth. This excludes setup labels left behind
                        // by folded branches. If source and target collapsed into one final block,
                        // no distinct handler instructions survived for the edge to reach.
                        for entry in self.exception_handler_entries.iter().filter(|entry| {
                            entry.source == *label && i64::from(entry.source_depth) == depth
                        }) {
                            let Some(target) = graph.label_block(entry.target) else {
                                continue;
                            };
                            if target == block {
                                continue;
                            }
                            push_start_depth(
                                &mut block_states,
                                &mut pending,
                                target,
                                i64::from(entry.target_depth),
                            )?;
                        }
                        continue;
                    }
                    Item::Instruction(instruction) => instruction,
                };
                maximum = maximum.max(depth);
                let (argument, jump_target) = match instruction.operand {
                    Operand::Value(argument) => (argument, None),
                    Operand::Forward(label) | Operand::Backward(label) => (0, Some(label)),
                };
                depth = depth
                    .checked_add(opcode_stack_effect(instruction.opcode, argument))
                    .ok_or_else(|| stack_error("stack depth overflow"))?;
                if depth < 0 {
                    return Err(stack_error(format!(
                        "stack underflow at opcode {}",
                        instruction.opcode.code()
                    )));
                }

                // CPython explicitly excludes END_ASYNC_FOR's cleanup label from the jump-edge
                // traversal; its physical continuation is the ordinary fallthrough edge.
                let ends_block = if let Some(label) = jump_target {
                    if instruction.opcode != END_ASYNC_FOR {
                        let target = graph.label_block(label).ok_or_else(|| {
                            stack_error(format!("jump targets unbound label {}", label.0))
                        })?;
                        push_start_depth(&mut block_states, &mut pending, target, depth)?;
                    }
                    is_unconditional_jump(instruction.opcode)
                } else {
                    ends_scope(instruction.opcode)
                };
                if ends_block {
                    has_fallthrough = false;
                    break;
                }
            }

            if has_fallthrough && let Some(next) = graph.order().get(position + 1) {
                push_start_depth(&mut block_states, &mut pending, *next, depth)?;
            }
        }

        // CPython code objects reserve at least one value-stack slot, including scopes whose
        // reachable instructions have a maximum physical depth of zero (for example, `raise`).
        u32::try_from(maximum.max(1)).map_err(|_| stack_error("stack size exceeds u32"))
    }
}

fn push_start_depth(
    block_states: &mut [BlockState],
    pending: &mut Vec<BasicBlockId>,
    block: BasicBlockId,
    depth: i64,
) -> Result<(), CompileError> {
    let start_depth = &mut block_states[block.index()].start_depth;
    match *start_depth {
        current if current != UNREACHED_DEPTH && current != depth => Err(stack_error(format!(
            "inconsistent stack depth at block {}: {current} != {depth}",
            block.index()
        ))),
        UNREACHED_DEPTH => {
            *start_depth = depth;
            pending.push(block);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn stack_error(message: impl Into<String>) -> CompileError {
    CompileError::Internal(format!(
        "invalid final control-flow stack: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::block_graph::BlockGraph;
    use crate::assembler::{
        ExceptionHandlerEntry, ExceptionRegion, Instruction, Label, SourceLocation,
    };
    use crate::target::opcodes::{
        JUMP_FORWARD, LOAD_CONST, POP_JUMP_IF_FALSE, POP_TOP, RETURN_VALUE,
    };

    fn instruction(opcode: crate::target::Opcode, operand: Operand) -> Item {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            SourceLocation::NONE,
            None,
        ))
    }

    fn pipeline(items: Vec<Item>) -> OptimizationPipeline {
        OptimizationPipeline {
            graph: BlockGraph::from_items(items, &rustc_hash::FxHashSet::default()),
            next_label: 0,
            exception_regions: Vec::new(),
            exception_handler_entries: Vec::new(),
            preserved_block_boundaries: rustc_hash::FxHashSet::default(),
            borrow_unreachable_blocks: rustc_hash::FxHashSet::default(),
        }
    }

    #[test]
    fn rejects_stack_underflow() {
        let pipeline = pipeline(vec![instruction(POP_TOP, Operand::Value(0))]);

        let error = pipeline.calculate_stack_size().unwrap_err();

        assert!(error.to_string().contains("stack underflow"));
    }

    #[test]
    fn reserves_cpython_minimum_stack_slot() {
        let pipeline = pipeline(vec![instruction(
            crate::target::opcodes::NOP,
            Operand::Value(0),
        )]);

        assert_eq!(pipeline.calculate_stack_size().unwrap(), 1);
    }

    #[test]
    fn rejects_inconsistent_block_entry_depths() {
        let target = Label(0);
        let pipeline = pipeline(vec![
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(POP_JUMP_IF_FALSE, Operand::Forward(target)),
            instruction(LOAD_CONST, Operand::Value(1)),
            instruction(JUMP_FORWARD, Operand::Forward(target)),
            Item::Label(target),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        let error = pipeline.calculate_stack_size().unwrap_err();

        assert!(error.to_string().contains("inconsistent stack depth"));
    }

    #[test]
    fn includes_exception_handler_entry_values() {
        let start = Label(0);
        let end = Label(1);
        let handler = Label(2);
        let exit = Label(3);
        let mut pipeline = pipeline(vec![
            Item::Label(start),
            instruction(LOAD_CONST, Operand::Value(0)),
            Item::Label(end),
            instruction(JUMP_FORWARD, Operand::Forward(exit)),
            Item::Label(handler),
            instruction(POP_TOP, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(exit),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        pipeline.exception_regions.push(ExceptionRegion {
            start,
            end,
            target: handler,
            depth: 0,
            preserve_lasti: true,
        });

        assert_eq!(pipeline.calculate_stack_size().unwrap(), 2);
    }

    #[test]
    fn retains_setup_edge_after_exception_region_disappears() {
        let source = Label(0);
        let handler = Label(1);
        let exit = Label(2);
        let mut pipeline = pipeline(vec![
            Item::Label(source),
            instruction(crate::target::opcodes::NOP, Operand::Value(0)),
            instruction(JUMP_FORWARD, Operand::Forward(exit)),
            Item::Label(handler),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(exit),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        pipeline
            .exception_handler_entries
            .push(ExceptionHandlerEntry {
                source,
                target: handler,
                source_depth: 0,
                target_depth: 1,
            });

        assert_eq!(pipeline.calculate_stack_size().unwrap(), 2);
    }

    #[test]
    fn ignores_setup_edge_from_an_eliminated_path() {
        let source = Label(0);
        let handler = Label(1);
        let exit = Label(2);
        let mut pipeline = pipeline(vec![
            Item::Label(source),
            instruction(crate::target::opcodes::NOP, Operand::Value(0)),
            instruction(JUMP_FORWARD, Operand::Forward(exit)),
            Item::Label(handler),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(exit),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        pipeline
            .exception_handler_entries
            .push(ExceptionHandlerEntry {
                source,
                target: handler,
                source_depth: 1,
                target_depth: 10,
            });

        assert_eq!(pipeline.calculate_stack_size().unwrap(), 1);
    }

    #[test]
    fn ignores_setup_edge_collapsed_into_its_source_block() {
        let source = Label(0);
        let mut pipeline = pipeline(vec![
            Item::Label(source),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        pipeline
            .exception_handler_entries
            .push(ExceptionHandlerEntry {
                source,
                target: source,
                source_depth: 0,
                target_depth: 4,
            });

        assert_eq!(pipeline.calculate_stack_size().unwrap(), 1);
    }

    #[test]
    fn ignores_unreachable_high_stack_depths() {
        let exit = Label(0);
        let mut items = vec![
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(JUMP_FORWARD, Operand::Forward(exit)),
        ];
        items.extend((0..100).map(|_| instruction(LOAD_CONST, Operand::Value(0))));
        items.extend([
            Item::Label(exit),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        let pipeline = pipeline(items);

        assert_eq!(pipeline.calculate_stack_size().unwrap(), 1);
    }

    #[test]
    fn handles_sparse_block_ids() {
        let exit = Label(0);
        let mut pipeline = pipeline(vec![
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(JUMP_FORWARD, Operand::Forward(exit)),
            instruction(LOAD_CONST, Operand::Value(1)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(exit),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        let unreachable = pipeline.graph.order()[1];
        pipeline.graph.retain_order(|block| block != unreachable);

        assert_eq!(pipeline.calculate_stack_size().unwrap(), 1);
    }
}
