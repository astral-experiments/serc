use crate::PythonVersion;
use crate::target::{Opcode, opcodes};

/// Offsets are bytes, measured from the start of `co_code`. The first prefix is
/// the instruction's entry point; only the opcode and its caches contribute
/// to the base of a relative jump.
#[derive(Clone, Copy, Debug)]
pub(super) struct Instruction {
    pub offset: usize,
    pub opcode: Opcode,
    pub name: &'static str,
    pub arg: u32,
    pub jump: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Handler {
    pub start: usize,
    pub end: usize,
    pub target: usize,
    pub depth: usize,
    pub lasti: bool,
}

pub(super) struct Bytecode {
    pub ops: Vec<Instruction>,
    pub handlers: Vec<Handler>,
}

impl Bytecode {
    pub(super) fn decode(
        version: PythonVersion,
        bytes: &[u8],
        exceptions: &[u8],
    ) -> Result<Self, String> {
        let mut ops = Vec::new();
        let mut cursor = 0;
        while cursor < bytes.len() {
            let offset = cursor;
            if ops.len() >= 1_000_000 {
                return Err(format!("instruction limit exceeded at byte {offset}"));
            }
            let mut arg = 0;
            let mut prefixes = 0;
            let (opcode, name) = loop {
                let [raw, low] = bytes
                    .get(cursor..cursor + 2)
                    .ok_or_else(|| format!("truncated instruction at byte {cursor}"))?
                else {
                    unreachable!()
                };
                let (opcode, name) = Opcode::from_wire(version, *raw)
                    .ok_or_else(|| format!("unknown opcode {raw} at byte {cursor}"))?;
                cursor += 2;
                arg = (arg << 8) | u32::from(*low);
                if opcode != opcodes::EXTENDED_ARG {
                    break (opcode, name);
                }
                prefixes += 1;
                if prefixes > 3 {
                    return Err(format!("too many EXTENDED_ARG prefixes at byte {offset}"));
                }
            };
            if arg > i32::MAX.cast_unsigned() {
                return Err(format!("invalid signed operand at byte {offset}"));
            }
            if opcode == opcodes::CACHE {
                return Err(format!("unexpected CACHE at byte {offset}"));
            }
            let end = cursor + usize::from(opcode.caches_for(version)) * 2;
            if !bytes
                .get(cursor..end)
                .is_some_and(|cache| cache.iter().all(|&b| b == 0))
            {
                return Err(format!(
                    "invalid or truncated {name} cache at byte {cursor}"
                ));
            }
            cursor = end;
            let jump = if opcode.has_jump_for(version) {
                let distance = usize::try_from(arg)
                    .ok()
                    .and_then(|n| n.checked_mul(2))
                    .ok_or("jump operand overflow")?;
                let dest = if matches!(
                    opcode,
                    opcodes::JUMP_BACKWARD
                        | opcodes::JUMP_BACKWARD_NO_INTERRUPT
                        | opcodes::END_ASYNC_FOR
                ) {
                    end.checked_sub(distance)
                } else {
                    end.checked_add(distance)
                };
                Some(dest.ok_or_else(|| format!("invalid jump at byte {offset}"))?)
            } else {
                None
            };
            ops.push(Instruction {
                offset,
                opcode,
                name,
                arg,
                jump,
            });
        }
        let boundary = |offset| ops.binary_search_by_key(&offset, |op| op.offset).is_ok();
        for op in &ops {
            if op.jump.is_some_and(|offset| !boundary(offset)) {
                return Err(format!(
                    "invalid {} destination at byte {}",
                    op.name, op.offset
                ));
            }
        }
        let mut table = exceptions;
        let mut handlers = Vec::new();
        while !table.is_empty() {
            let start = varint(&mut table, true)?;
            let size = varint(&mut table, false)?;
            let target = varint(&mut table, false)?;
            let packed = varint(&mut table, false)?;
            let to_bytes = |value: usize| value.checked_mul(2).ok_or("exception offset overflow");
            let start = to_bytes(start)?;
            let end = start
                .checked_add(to_bytes(size)?)
                .ok_or("exception range overflow")?;
            let target = to_bytes(target)?;
            if size == 0
                || !boundary(start)
                || !boundary(target)
                || (end != bytes.len() && !boundary(end))
            {
                return Err(format!("invalid exception range at byte {start}"));
            }
            if handlers.len() >= 1_000_000 {
                return Err("exception table limit exceeded".into());
            }
            handlers.push(Handler {
                start,
                end,
                target,
                depth: packed >> 1,
                lasti: packed & 1 != 0,
            });
        }
        if ops.is_empty() {
            return Err("empty instruction stream".into());
        }
        Ok(Self { ops, handlers })
    }
}

fn varint(table: &mut &[u8], mut start: bool) -> Result<usize, String> {
    let mut value = 0usize;
    loop {
        let (&byte, rest) = table.split_first().ok_or("truncated exception table")?;
        *table = rest;
        if (byte & 0x80 != 0) != start {
            return Err("invalid exception entry marker".into());
        }
        start = false;
        value = value.checked_mul(64).ok_or("exception varint overflow")? | usize::from(byte & 63);
        if byte & 64 == 0 {
            return Ok(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::reader::read;
    use std::process::Command;

    // Both the bytes and the expected disassembly come from the pinned interpreter.
    #[test]
    fn matches_dis_on_all_targets() {
        let script = r"
import dis, importlib.util, marshal, platform, sys
co = compile('def f(xs):\n for x in xs:\n  try: use(x)\n  except ValueError: break\n', 'test.py', 'exec')
f = next(c for c in co.co_consts if hasattr(c, 'co_code'))
print(platform.python_version())
print((importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(co)).hex())
for i in dis.get_instructions(f):
 print(i.offset, i.opname, i.arg or 0, i.argval if i.opcode in dis.hasjrel else '-')
for e in dis.Bytecode(f).exception_entries:
 print(e.start, e.end, e.target, e.depth, int(e.lasti))
";
        for (var, version) in [
            ("SERC_TEST_PYTHON_312", PythonVersion::Py312),
            ("SERC_TEST_PYTHON_313", PythonVersion::Py313),
            ("SERC_TEST_PYTHON", PythonVersion::Py314),
            ("SERC_TEST_PYTHON_315", PythonVersion::Py315),
        ] {
            let path = std::env::var(var).unwrap_or_else(|_| format!("python{version}"));
            let output = Command::new(&path)
                .args(["-I", "-S", "-c", script])
                .output()
                .unwrap_or_else(|e| {
                    panic!("failed to start {path}: {e} (set {var} to the pinned interpreter)")
                });
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = String::from_utf8(output.stdout).unwrap();
            let mut lines = output.lines();
            assert_eq!(lines.next(), version.target().tag.strip_prefix('v'));
            let hex = lines.next().unwrap();
            let pyc: Vec<_> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let module = read(&pyc).unwrap();
            assert_eq!(module.version, version);
            let root = module.code(module.root).unwrap();
            let f = module
                .tuple(root.constants)
                .unwrap()
                .iter()
                .find_map(|id| module.code(*id).ok())
                .unwrap();
            let decoded = Bytecode::decode(
                version,
                module.bytes(f.bytecode).unwrap(),
                module.bytes(f.exceptions).unwrap(),
            )
            .unwrap();
            let instructions = decoded.ops.iter().map(|op| {
                format!(
                    "{} {} {} {}",
                    op.offset,
                    op.name,
                    op.arg,
                    op.jump.map_or_else(|| "-".to_owned(), |n| n.to_string())
                )
            });
            let entries = decoded.handlers.iter().map(|h| {
                format!(
                    "{} {} {} {} {}",
                    h.start,
                    h.end,
                    h.target,
                    h.depth,
                    u8::from(h.lasti)
                )
            });
            assert_eq!(
                instructions.chain(entries).collect::<Vec<_>>(),
                lines.collect::<Vec<_>>(),
                "{version}"
            );
        }
    }

    #[test]
    fn prefixes_and_invalid_destinations() {
        let v = PythonVersion::Py314;
        let bytes = [
            opcodes::EXTENDED_ARG.code_for(v),
            1,
            opcodes::LOAD_CONST.code_for(v),
            3,
        ];
        assert_eq!(Bytecode::decode(v, &bytes, &[]).unwrap().ops[0].arg, 259);
        let bytes = [opcodes::JUMP_BACKWARD.code_for(v), 255];
        assert!(Bytecode::decode(v, &bytes, &[]).is_err());
        assert!(Bytecode::decode(v, &[opcodes::RESUME.code_for(v), 0], &[128]).is_err());
    }
}
