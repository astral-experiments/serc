//! A candidate must reproduce the executable fields of every nested code
//! object. Source positions describe the lost original spelling.
use crate::marshal::reader::{Module, Object};

pub(super) fn equivalent(original: &Module<'_>, recovered: &Module<'_>) -> bool {
    let mut pending = vec![(original.root, recovered.root)];
    let mut seen = rustc_hash::FxHashSet::default();
    let mut aliases_a = rustc_hash::FxHashMap::default();
    let mut aliases_b = rustc_hash::FxHashMap::default();
    let mut budget = 2_000_000usize;
    while let Some((i, j)) = pending.pop() {
        if budget == 0 {
            return false;
        }
        budget -= 1;
        if !seen.insert((i, j)) {
            continue;
        }
        if seen.len() > 1_000_000 {
            return false;
        }
        // For identity-bearing constants a shared reference cannot become
        // two independently reconstructed objects, or vice versa.
        if matches!(original.objects[i], Object::Text { interned: false, .. } | Object::Tuple(_) | Object::Int(257.. | ..=-6) | Object::Bytes(_) | Object::Float(_) | Object::Complex(..) | Object::Long { .. } | Object::Code(_) | Object::FrozenSet(_) | Object::FrozenDict(_) | Object::Slice(_))
            && (aliases_a.insert(i, j).is_some_and(|old| old != j)
                || aliases_b.insert(j, i).is_some_and(|old| old != i))
        {
            return false;
        }
        let ok = match (&original.objects[i], &recovered.objects[j]) {
            (Object::Code(x), Object::Code(y)) => {
                pending.extend([
                    (x.bytecode, y.bytecode),
                    (x.constants, y.constants),
                    (x.names, y.names),
                    (x.locals, y.locals),
                    (x.local_kinds, y.local_kinds),
                    (x.name, y.name),
                    (x.qualname, y.qualname),
                    (x.exceptions, y.exceptions),
                ]);
                (x.args, x.pos_only, x.kw_only, x.flags, x.stack_size)
                    == (y.args, y.pos_only, y.kw_only, y.flags, y.stack_size)
            }
            (Object::Tuple(x), Object::Tuple(y)) | (Object::FrozenSet(x), Object::FrozenSet(y)) => {
                if x.len() != y.len() || x.len() + pending.len() > 1_000_000 {
                    return false;
                }
                pending.extend(x.iter().copied().zip(y.iter().copied()));
                true
            }
            (Object::Slice(x), Object::Slice(y)) => {
                pending.extend(x.iter().copied().zip(y.iter().copied()));
                true
            }
            (Object::FrozenDict(x), Object::FrozenDict(y)) => {
                if x.len() != y.len() || x.len() * 2 + pending.len() > 1_000_000 {
                    return false;
                }
                for ((ka, va), (kb, vb)) in x.iter().zip(y) {
                    pending.extend([(*ka, *kb), (*va, *vb)]);
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
            return false;
        }
    }
    true
}
