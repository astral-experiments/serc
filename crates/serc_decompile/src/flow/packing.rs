//! Compare handler ranges in units of individual operations. CPython can
//! fuse adjacent local loads differently when their source lines change.
use serc_bytecode::PythonVersion;
use serc_bytecode::instructions::{Bytecode, Handler};
use serc_bytecode::target::{Opcode, opcodes};

fn paired_load(opcode: Opcode) -> Option<Opcode> {
    match opcode {
        opcodes::LOAD_FAST_LOAD_FAST => Some(opcodes::LOAD_FAST),
        opcodes::LOAD_FAST_BORROW_LOAD_FAST_BORROW => Some(opcodes::LOAD_FAST_BORROW),
        _ => None,
    }
}

struct Unpacked {
    code: Bytecode,
    // Logical index of each physical instruction, followed by the exclusive end.
    starts: Vec<usize>,
}

impl Unpacked {
    fn decode(version: PythonVersion, bytes: &[u8], table: &[u8]) -> Option<Self> {
        let code = Bytecode::decode(version, bytes, table).ok()?;
        let mut starts = Vec::with_capacity(code.ops.len() + 1);
        let mut index = 0;
        for op in &code.ops {
            starts.push(index);
            index += 1;
            if paired_load(op.opcode).is_some() {
                u8::try_from(op.arg).ok()?;
                index += 1;
            }
        }
        starts.push(index);
        Some(Self { code, starts })
    }

    fn logical(&self, offset: usize) -> usize {
        // The decoder validates every jump, handler and exclusive end boundary.
        self.starts[self.code.ops.partition_point(|op| op.offset < offset)]
    }

    fn instructions(&self) -> impl Iterator<Item = (Opcode, usize)> {
        self.code.ops.iter().flat_map(|op| {
            let arg = op.arg as usize;
            let (first, second) = if let Some(load) = paired_load(op.opcode) {
                ((load, arg >> 4), Some((load, arg & 15)))
            } else {
                let arg = op.jump.map_or(arg, |offset| self.logical(offset));
                ((op.opcode, arg), None)
            };
            std::iter::once(first).chain(second)
        })
    }

    fn handlers(&self) -> impl Iterator<Item = Handler> {
        self.code.handlers.iter().map(|h| Handler {
            start: self.logical(h.start),
            end: self.logical(h.end),
            target: self.logical(h.target),
            ..*h
        })
    }
}

pub(crate) fn same_packed_flow(
    version: PythonVersion,
    original: &[u8],
    original_table: &[u8],
    compiled: &[u8],
    compiled_table: &[u8],
) -> bool {
    if version == PythonVersion::Py312 || original_table.is_empty() || compiled_table.is_empty() {
        return false;
    }
    let (Some(left), Some(right)) = (
        Unpacked::decode(version, original, original_table),
        Unpacked::decode(version, compiled, compiled_table),
    ) else {
        return false;
    };
    left.starts != right.starts
        && left.starts.last() == right.starts.last()
        && left.code.handlers.len() == right.code.handlers.len()
        && left.handlers().eq(right.handlers())
        && left.instructions().eq(right.instructions())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocated_edges_must_match_at_individual_load_boundaries() {
        let version = PythonVersion::Py314;
        let jump = opcodes::JUMP_FORWARD.code_for(version);
        let pair = opcodes::LOAD_FAST_BORROW_LOAD_FAST_BORROW.code_for(version);
        let load = opcodes::LOAD_FAST_BORROW.code_for(version);
        let ret = opcodes::RETURN_VALUE.code_for(version);
        let packed = [jump, 1, pair, 0x12, ret, 0];
        let split = [jump, 2, load, 1, load, 2, ret, 0];
        // The range runs from the loads to the end of the stream.
        let matches = |ops: &[u8], table: &[u8]| {
            same_packed_flow(version, &packed, &[0x81, 2, 2, 3], ops, table)
        };
        let table = [0x81, 3, 3, 3];
        assert!(matches(&split, &table));
        for changed in [
            [0x82, 2, 3, 3], // second load starts protection
            [0x81, 2, 3, 3], // return loses protection
            [0x81, 3, 2, 3], // handler starts on second load
            [0x81, 3, 3, 1], // lower stack depth
            [0x81, 3, 3, 2], // lasti is no longer pushed
        ] {
            assert!(!matches(&split, &changed));
        }
        for (index, value) in [
            (1, 1), // jump lands on the second load
            (2, opcodes::LOAD_FAST.code_for(version)),
            (4, opcodes::LOAD_FAST_CHECK.code_for(version)),
            (5, 1), // second local differs
        ] {
            let mut changed = split;
            changed[index] = value;
            assert!(!matches(&changed, &table));
        }
    }
}
