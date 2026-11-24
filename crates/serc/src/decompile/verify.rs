//! A candidate must reproduce the executable fields of every nested code
//! object. Source positions describe the lost original spelling.
use crate::marshal::reader::{Module, Object};

pub(super) fn compare(original: &Module<'_>, recovered: &Module<'_>) -> Result<(), String> {
    let mut pending = vec![(original.root, recovered.root, original.root, "module")];
    let mut seen = rustc_hash::FxHashSet::default();
    let mut aliases_a = rustc_hash::FxHashMap::default();
    let mut aliases_b = rustc_hash::FxHashMap::default();
    let mut budget = 2_000_000usize;
    while let Some((i, j, scope, field)) = pending.pop() {
        if budget == 0 {
            return Err("verification work limit exceeded".into());
        }
        budget -= 1;
        if !seen.insert((i, j)) {
            continue;
        }
        if seen.len() > 1_000_000 {
            return Err("verification work limit exceeded".into());
        }
        let mismatch = |issue: &str| {
            let name = original
                .code(scope)
                .and_then(|code| original.text(code.qualname))
                .unwrap_or("<module>");
            format!("{name}: reconstructed {field} {issue}")
        };
        // For identity-bearing constants a shared reference cannot become
        // two independently reconstructed objects, or vice versa.
        if matches!(original.objects[i], Object::Text { interned: false, .. } | Object::Tuple(_) | Object::Int(257.. | ..=-6) | Object::Bytes(_) | Object::Float(_) | Object::Complex(..) | Object::Long { .. } | Object::Code(_) | Object::FrozenSet(_) | Object::FrozenDict(_) | Object::Slice(_))
            && (aliases_a.insert(i, j).is_some_and(|old| old != j)
                || aliases_b.insert(j, i).is_some_and(|old| old != i))
        {
            return Err(mismatch("has different reference sharing"));
        }
        let ok = match (&original.objects[i], &recovered.objects[j]) {
            (Object::Code(x), Object::Code(y)) => {
                pending.extend([
                    (x.bytecode, y.bytecode, i, "co_code"),
                    (x.constants, y.constants, i, "co_consts"),
                    (x.names, y.names, i, "co_names"),
                    (x.locals, y.locals, i, "locals-plus names"),
                    (x.local_kinds, y.local_kinds, i, "locals-plus kinds"),
                    (x.name, y.name, i, "co_name"),
                    (x.qualname, y.qualname, i, "co_qualname"),
                    (x.exceptions, y.exceptions, i, "co_exceptiontable"),
                ]);
                if (x.args, x.pos_only, x.kw_only, x.flags, x.stack_size)
                    != (y.args, y.pos_only, y.kw_only, y.flags, y.stack_size)
                {
                    return Err(format!(
                        "{}: reconstructed signature, flags or stack size differs",
                        original.text(x.qualname)?
                    ));
                }
                true
            }
            (Object::Tuple(x), Object::Tuple(y)) | (Object::FrozenSet(x), Object::FrozenSet(y)) => {
                if x.len() != y.len() || x.len() + pending.len() > 1_000_000 {
                    return Err(mismatch(
                        "has a different length or exceeds the comparison limit",
                    ));
                }
                pending.extend(x.iter().zip(y).map(|(&a, &b)| (a, b, scope, field)));
                true
            }
            (Object::Slice(x), Object::Slice(y)) => {
                pending.extend(x.iter().zip(y).map(|(&a, &b)| (a, b, scope, field)));
                true
            }
            (Object::FrozenDict(x), Object::FrozenDict(y)) => {
                if x.len() != y.len() || x.len() * 2 + pending.len() > 1_000_000 {
                    return Err(mismatch(
                        "has a different length or exceeds the comparison limit",
                    ));
                }
                for ((ka, va), (kb, vb)) in x.iter().zip(y) {
                    pending.extend([(*ka, *kb, scope, field), (*va, *vb, scope, field)]);
                }
                true
            }
            (
                Object::Text {
                    bytes: x,
                    interned: intern_x,
                },
                Object::Text {
                    bytes: y,
                    interned: intern_y,
                },
            ) => x == y && intern_x == intern_y,
            (Object::Bytes(x), Object::Bytes(y)) if x != y && field == "co_code" => {
                let offset = x
                    .iter()
                    .zip(*y)
                    .position(|(a, b)| a != b)
                    .unwrap_or(x.len().min(y.len()));
                return Err(mismatch(&format!("differs at byte {offset}")));
            }
            (Object::Bytes(x), Object::Bytes(y)) => x == y,
            (Object::None, Object::None) | (Object::Ellipsis, Object::Ellipsis) => true,
            (Object::Int(x), Object::Int(y)) => x == y,
            (Object::Bool(x), Object::Bool(y)) => x == y,
            (Object::Float(x), Object::Float(y)) => x == y,
            (Object::Complex(r, i), Object::Complex(s, j)) => (r, i) == (s, j),
            (
                Object::Long {
                    negative: n,
                    digits: x,
                },
                Object::Long {
                    negative: m,
                    digits: y,
                },
            ) => n == m && x == y,
            _ => false,
        };
        if !ok {
            return Err(mismatch("differs"));
        }
    }
    Ok(())
}
