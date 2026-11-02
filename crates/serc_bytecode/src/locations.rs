//! Decode instruction line numbers from CPython's location table. Each entry
//! covers one to eight two-byte code units, including inline caches.
use crate::instructions::Instruction;

/// Return the optional source line at each decoded instruction's entry point.
/// An omitted table provides no hints. Invalid tables cannot locate a handler.
pub fn instruction_lines(
    table: &[u8],
    first: usize,
    byte_len: usize,
    ops: &[Instruction],
) -> Result<Vec<Option<usize>>, String> {
    let mut lines = vec![None; ops.len()];
    if table.is_empty() {
        return Ok(lines);
    }
    let mut table = table;
    let mut line = i64::from(i32::try_from(first).map_err(|_| "invalid first line")?);
    let mut end = 0usize;
    let mut next = 0;
    while let Some((&header, rest)) = table.split_first() {
        table = rest;
        if header & 128 == 0 {
            return Err("missing location entry marker".into());
        }
        end = end
            .checked_add(2 * (usize::from(header & 7) + 1))
            .ok_or("location offset overflow")?;
        if end > byte_len {
            return Err("location extends beyond bytecode".into());
        }
        let kind = header >> 3 & 15;
        match kind {
            0..=9 => {
                payload(&mut table)?;
            }
            10..=12 => {
                line += i64::from(kind - 10);
                payload(&mut table)?;
                payload(&mut table)?;
            }
            13 | 14 => {
                let value = varint(&mut table)?;
                let delta = i64::from(value >> 1);
                line += if value & 1 != 0 { -delta } else { delta };
                if kind == 14 {
                    for _ in 0..3 {
                        varint(&mut table)?;
                    }
                }
            }
            _ => {}
        }
        if !(0..=i64::from(i32::MAX)).contains(&line) {
            return Err("invalid location line".into());
        }
        while next < ops.len() && ops[next].offset < end {
            lines[next] = if kind == 15 {
                None
            } else {
                usize::try_from(line).ok()
            };
            next += 1;
        }
    }
    if next != ops.len() || end != byte_len {
        return Err("truncated location table".into());
    }
    Ok(lines)
}

fn payload(table: &mut &[u8]) -> Result<u8, String> {
    let (&byte, rest) = table.split_first().ok_or("truncated location entry")?;
    if byte & 128 != 0 {
        return Err("unexpected location entry marker".into());
    }
    *table = rest;
    Ok(byte)
}

fn varint(table: &mut &[u8]) -> Result<u32, String> {
    let mut value = 0_u32;
    for shift in (0..32).step_by(6) {
        let byte = payload(table)?;
        let part = u32::from(byte & 63);
        if part > u32::MAX >> shift {
            return Err("location varint overflow".into());
        }
        value |= part << shift;
        if byte & 64 == 0 {
            return Ok(value);
        }
    }
    Err("location varint overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_runs_and_varints() {
        for table in [
            &[0][..],
            &[0xf0, 0xc0],
            &[0xf0, 64, 64, 64, 64, 64, 63],
            &[0xe8, 3],
            &[0xf9],
        ] {
            assert!(instruction_lines(table, 0, 2, &[]).is_err(), "{table:?}");
        }
    }
}
