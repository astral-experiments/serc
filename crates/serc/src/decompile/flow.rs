//! Stack heights at instruction boundaries. Branch targets use the compiler's
//! logical edges through `END_FOR` and `END_SEND`, including their cleanup effects.
use super::instructions::Bytecode;
use crate::PythonVersion;
use crate::target::{is_scope_exit_for, is_unconditional_jump, opcodes, stack_counts_for};

pub(super) fn stack_heights(
    code: &Bytecode,
    version: PythonVersion,
    stack_size: usize,
) -> Result<Vec<Option<usize>>, String> {
    let mut heights = vec![None; code.ops.len()];
    let mut pending = vec![(0, 0)];
    if code
        .handlers
        .windows(2)
        .any(|pair| pair[0].end > pair[1].start)
    {
        return Err("overlapping or unsorted exception ranges".into());
    }
    let index = |offset| {
        code.ops
            .binary_search_by_key(&offset, |op| op.offset)
            .map_err(|_| format!("missing instruction at byte {offset}"))
    };
    while let Some((i, height)) = pending.pop() {
        let op = code
            .ops
            .get(i)
            .ok_or("control falls off the instruction stream")?;
        if let Some(known) = heights[i] {
            if height != known {
                return Err(format!("inconsistent stack heights at byte {}", op.offset));
            }
            continue;
        }
        if height > stack_size {
            return Err(format!("stack exceeds declared size at byte {}", op.offset));
        }
        heights[i] = Some(height);
        if let Some(h) = code
            .handlers
            .get(code.handlers.partition_point(|h| h.end <= op.offset))
            && h.start <= op.offset
        {
            // The interpreter restores the recorded depth, optionally pushes
            // the saved instruction offset, then pushes the exception.
            let handler_height = h
                .depth
                .checked_add(1 + usize::from(h.lasti))
                .ok_or("exception stack overflow")?;
            pending.push((index(h.target)?, handler_height));
        }
        let (pop, push) = if op.opcode == opcodes::RETURN_GENERATOR {
            // Resuming a newly created generator pushes its initial send value.
            (0, 1)
        } else {
            stack_counts_for(version, op.opcode, op.arg)
        };
        let next = height
            .checked_sub(pop)
            .and_then(|n| n.checked_add(push))
            .ok_or_else(|| format!("stack underflow at {} byte {}", op.name, op.offset))?;
        // END_ASYNC_FOR points back to the SEND that supplied the
        // exception. Execution falls through on successful cleanup.
        if let Some(target) = op.jump
            && op.opcode != opcodes::END_ASYNC_FOR
        {
            pending.push((index(target)?, next));
        }
        if !is_scope_exit_for(version, op.opcode) && !is_unconditional_jump(op.opcode) {
            pending.push((i + 1, next));
        }
    }
    Ok(heights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::reader::{Object, read};
    use crate::{CompileOptions, compile_with_options};

    #[test]
    fn joins_backedges_and_handlers() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let code = compile_with_options("def f(xs):\n for x in xs:\n  try:\n   if x: use(x)\n  except ValueError: break\n return None\n", "flow.py", &CompileOptions { python_version: version, ..CompileOptions::default() }).unwrap();
            let bytes = code.to_timestamp_pyc(0, 0);
            let m = read(&bytes).unwrap();
            for item in &m.objects {
                if let Object::Code(code) = item {
                    let bc = Bytecode::decode(
                        version,
                        m.bytes(code.bytecode).unwrap(),
                        m.bytes(code.exceptions).unwrap(),
                    )
                    .unwrap();
                    let heights = stack_heights(&bc, version, code.stack_size).unwrap();
                    for handler in &bc.handlers {
                        let i = bc
                            .ops
                            .binary_search_by_key(&handler.target, |op| op.offset)
                            .unwrap();
                        assert_eq!(
                            heights[i],
                            Some(handler.depth + 1 + usize::from(handler.lasti))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn underflow_is_an_error() {
        let version = PythonVersion::Py314;
        let bc =
            Bytecode::decode(version, &[opcodes::RETURN_VALUE.code_for(version), 0], &[]).unwrap();
        assert!(
            stack_heights(&bc, version, 1)
                .unwrap_err()
                .contains("underflow")
        );
    }
}
