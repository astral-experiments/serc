//! The normal edges use CPython's logical targets through `END_FOR` and `END_SEND`.
//! Protection boundaries split blocks so an exceptional edge always restores
//! the same handler stack throughout its block.
use std::ops::Range;

use serc_bytecode::PythonVersion;
use serc_bytecode::instructions::{Bytecode, Handler};
use serc_bytecode::target::{is_scope_exit_for, is_unconditional_jump, opcodes, stack_counts_for};

pub(super) struct Block {
    pub(super) ops: Range<usize>,
    jump: Option<usize>,
    fallthrough: Option<usize>,
    handler: Option<(usize, Handler)>,
    predecessors: Vec<usize>,
    height: Option<usize>,
}

pub(super) struct Graph {
    blocks: Vec<Block>,
    owner: Vec<usize>,
    // Nearest common continuation on forward normal paths. Returns, raises
    // and backedges end a forward region at a shared virtual exit.
    forward_postdom: Vec<usize>,
}

impl Graph {
    pub(super) fn build(
        code: &Bytecode,
        version: PythonVersion,
        stack_size: usize,
    ) -> Result<Self, String> {
        if code
            .handlers
            .windows(2)
            .any(|pair| pair[0].end > pair[1].start)
        {
            return Err("overlapping or unsorted exception ranges".into());
        }
        let len = code.ops.len();
        let index = |offset| {
            code.ops
                .binary_search_by_key(&offset, |op| op.offset)
                .map_err(|_| format!("missing instruction at byte {offset}"))
        };
        let mut starts = vec![false; len + 1];
        starts[0] = true;
        starts[len] = true;
        for (i, op) in code.ops.iter().enumerate() {
            if let Some(target) = op.jump
                && op.opcode != opcodes::END_ASYNC_FOR
            {
                starts[index(target)?] = true;
                starts[i + 1] = true;
            } else if is_scope_exit_for(version, op.opcode) {
                starts[i + 1] = true;
            }
        }
        for h in &code.handlers {
            starts[index(h.start)?] = true;
            starts[index(h.target)?] = true;
            // An interval may end one past the instruction stream.
            starts[code.ops.partition_point(|op| op.offset < h.end)] = true;
        }

        let mut owner = vec![0; len];
        let mut blocks = Vec::new();
        let mut start = 0;
        for end in (1..=len).filter(|&i| starts[i]) {
            owner[start..end].fill(blocks.len());
            blocks.push(Block {
                ops: start..end,
                jump: None,
                fallthrough: None,
                handler: None,
                predecessors: Vec::new(),
                height: None,
            });
            start = end;
        }
        let mut handlers = code.handlers.iter().peekable();
        for block in &mut blocks {
            let op = code.ops[block.ops.end - 1];
            let start = code.ops[block.ops.start].offset;
            while handlers.peek().is_some_and(|h| h.end <= start) {
                handlers.next();
            }
            if let Some(&h) = handlers.peek()
                && h.start <= start
            {
                block.handler = Some((owner[index(h.target)?], *h));
            }
            if let Some(target) = op.jump
                && op.opcode != opcodes::END_ASYNC_FOR
            {
                block.jump = Some(owner[index(target)?]);
            }
            if !is_scope_exit_for(version, op.opcode) && !is_unconditional_jump(op.opcode) {
                // owner.len() is a sentinel for a reachable malformed fallthrough.
                block.fallthrough = Some(owner.get(block.ops.end).copied().unwrap_or(len));
            }
        }

        for id in 0..blocks.len() {
            for target in [blocks[id].jump, blocks[id].fallthrough]
                .into_iter()
                .flatten()
            {
                if let Some(successor) = blocks.get_mut(target) {
                    successor.predecessors.push(id);
                }
            }
        }
        check_stack_heights(&mut blocks, code, version, stack_size)?;
        let forward_postdom = forward_postdominators(&blocks)?;
        Ok(Self {
            blocks,
            owner,
            forward_postdom,
        })
    }

    pub(super) fn block(&self, i: usize) -> &Block {
        &self.blocks[self.owner[i]]
    }

    pub(super) fn jump(&self, i: usize) -> Option<usize> {
        let b = &self.blocks[*self.owner.get(i)?];
        (b.ops.end == i + 1)
            .then_some(b.jump)
            .flatten()
            .map(|id| self.blocks[id].ops.start)
    }

    pub(super) fn target(&self, i: usize) -> Result<usize, String> {
        self.jump(i)
            .ok_or_else(|| "missing jump destination".into())
    }

    pub(super) fn handler(&self, i: usize) -> Option<Handler> {
        self.block(i).handler.map(|(_, h)| h)
    }

    pub(super) fn join(&self, i: usize) -> Option<usize> {
        self.blocks
            .get(self.forward_postdom[self.owner[i]])
            .map(|b| b.ops.start)
    }

    /// A folded predicate must have no side entrances from outside the test.
    pub(super) fn entered_from(&self, start: usize, predicate: Range<usize>) -> bool {
        // Predecessors were inserted in layout order, so checking both ends
        // proves that every incoming edge originated inside this range.
        let predecessors = &self.block(start).predecessors;
        [predecessors.first(), predecessors.last()]
            .into_iter()
            .flatten()
            .all(|&id| predicate.contains(&(self.blocks[id].ops.end - 1)))
    }

    pub(super) fn blocks_in(&self, range: Range<usize>) -> impl Iterator<Item = &Block> {
        let first = self.owner[range.start];
        self.blocks[first..]
            .iter()
            .take_while(move |b| !range.is_empty() && b.ops.start < range.end)
            .filter(|b| b.height.is_some())
    }
}

fn check_stack_heights(
    blocks: &mut [Block],
    code: &Bytecode,
    version: PythonVersion,
    stack_size: usize,
) -> Result<(), String> {
    let mut pending = vec![(0, 0)];
    while let Some((id, mut height)) = pending.pop() {
        let block = blocks
            .get_mut(id)
            .ok_or("control falls off the instruction stream")?;
        let offset = code.ops[block.ops.start].offset;
        if let Some(known) = block.height {
            if height != known {
                return Err(format!("inconsistent stack heights at byte {offset}"));
            }
            continue;
        }
        block.height = Some(height);
        for op in &code.ops[block.ops.clone()] {
            if height > stack_size {
                return Err(format!("stack exceeds declared size at byte {}", op.offset));
            }
            let (pop, push) = if op.opcode == opcodes::RETURN_GENERATOR {
                (0, 1)
            } else {
                stack_counts_for(version, op.opcode, op.arg)
            };
            height = height
                .checked_sub(pop)
                .and_then(|n| n.checked_add(push))
                .ok_or_else(|| format!("stack underflow at {} byte {}", op.name, op.offset))?;
        }
        if let Some((target, h)) = block.handler {
            let restored = h
                .depth
                .checked_add(1 + usize::from(h.lasti))
                .ok_or("exception stack overflow")?;
            pending.push((target, restored));
        }
        for successor in [block.jump, block.fallthrough].into_iter().flatten() {
            pending.push((successor, height));
        }
    }
    Ok(())
}

fn forward_postdominators(blocks: &[Block]) -> Result<Vec<usize>, String> {
    let exit = blocks.len();
    let mut parents = vec![exit; exit];
    // Bound intersections on hostile graphs with many long, crossing branches.
    let mut budget = exit * 64;
    for (id, block) in blocks.iter().enumerate().rev() {
        let forward = |next: usize| if next > id && next < exit { next } else { exit };
        let mut successors = [block.jump, block.fallthrough]
            .into_iter()
            .flatten()
            .map(forward);
        let mut common = successors.next().unwrap_or(exit);
        if let Some(mut other) = successors.next() {
            while common != other {
                budget = budget
                    .checked_sub(1)
                    .ok_or("control-flow analysis work limit exceeded")?;
                if common < other {
                    common = parents[common];
                } else {
                    other = parents[other];
                }
            }
        }
        parents[id] = common;
    }
    Ok(parents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serc_bytecode::marshal::reader::{Object, read};
    use std::process::Command;

    #[test]
    fn joins_backedges_and_handlers() {
        let source = "def f(xs):\n for x in xs:\n  try:\n   if x: use(x)\n  except ValueError: break\n return None\ndef diamond(x):\n if x: a()\n else: b()\n c()\n d()\n e()\n return x\n";
        for (var, version) in [
            ("SERC_TEST_PYTHON_312", PythonVersion::Py312),
            ("SERC_TEST_PYTHON_313", PythonVersion::Py313),
            ("SERC_TEST_PYTHON", PythonVersion::Py314),
            ("SERC_TEST_PYTHON_315", PythonVersion::Py315),
        ] {
            let python = std::env::var(var).unwrap_or_else(|_| format!("python{version}"));
            let compiled = Command::new(python)
                .args(["-I", "-S", "-c", "import importlib.util, marshal, platform, sys\nassert platform.python_version() == sys.argv[2]\nsys.stdout.buffer.write(importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(compile(sys.argv[1], 'flow.py', 'exec')))"])
                .arg(source)
                .arg(version.target().tag.trim_start_matches('v'))
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let m = read(&compiled.stdout).unwrap();
            assert_eq!(m.version, version);
            for item in &m.objects {
                if let Object::Code(code) = item {
                    let bc = Bytecode::decode(
                        version,
                        m.bytes(code.bytecode).unwrap(),
                        m.bytes(code.exceptions).unwrap(),
                    )
                    .unwrap();
                    let graph = Graph::build(&bc, version, code.stack_size).unwrap();
                    // Compare every decoded jump with its graph edge, including
                    // backward edges. Interior instructions have no jump edge.
                    for (i, op) in bc.ops.iter().enumerate() {
                        assert_eq!(graph.jump(i).map(|t| bc.ops[t].offset), op.jump);
                    }
                    if m.text(code.name).unwrap() == "diamond" {
                        let test = bc
                            .ops
                            .iter()
                            .position(|op| op.name == "POP_JUMP_IF_FALSE")
                            .unwrap();
                        let alternate = graph.target(test).unwrap();
                        let join = graph.target(alternate - 1).unwrap();
                        assert_eq!(graph.join(test), Some(join));
                        assert_eq!(graph.block(join).predecessors.len(), 2);
                        assert_ne!(graph.block(test).fallthrough, graph.block(test).jump);
                    }
                    for handler in &bc.handlers {
                        let i = bc
                            .ops
                            .binary_search_by_key(&handler.target, |op| op.offset)
                            .unwrap();
                        for (j, op) in bc
                            .ops
                            .iter()
                            .enumerate()
                            .filter(|(_, op)| (handler.start..handler.end).contains(&op.offset))
                        {
                            let block = graph.block(j);
                            assert!(bc.ops[block.ops.start].offset >= handler.start);
                            assert!(bc.ops[block.ops.end - 1].offset < handler.end);
                            assert_eq!(graph.handler(j), Some(*handler), "byte {}", op.offset);
                        }
                        assert_eq!(
                            graph.block(i).height,
                            Some(handler.depth + 1 + usize::from(handler.lasti))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn inconsistent_heights_at_a_join_are_rejected() {
        // Either jump to the NOP with an empty stack or push a second constant
        // before reaching it. Both paths are individually free of underflows.
        let ops = [
            (opcodes::LOAD_CONST, None),
            (opcodes::POP_JUMP_IF_TRUE, Some(8)),
            (opcodes::LOAD_CONST, None),
            (opcodes::JUMP_FORWARD, Some(8)),
            (opcodes::NOP, None),
            (opcodes::LOAD_CONST, None),
            (opcodes::RETURN_VALUE, None),
        ]
        .into_iter()
        .enumerate()
        .map(
            |(i, (opcode, jump))| serc_bytecode::instructions::Instruction {
                offset: i * 2,
                name: "test",
                opcode,
                arg: 0,
                jump,
            },
        )
        .collect();
        let graph = Graph::build(
            &Bytecode {
                ops,
                handlers: Vec::new(),
            },
            PythonVersion::Py314,
            2,
        );
        assert_eq!(
            graph.err().as_deref(),
            Some("inconsistent stack heights at byte 8")
        );
    }

    #[test]
    fn underflow_is_an_error() {
        let version = PythonVersion::Py314;
        let bc =
            Bytecode::decode(version, &[opcodes::RETURN_VALUE.code_for(version), 0], &[]).unwrap();
        assert!(
            Graph::build(&bc, version, 1)
                .err()
                .unwrap()
                .contains("underflow")
        );
    }
}
