//! CPython indexes names before discarding dead instructions. A missing run
//! of operands can therefore survive between two names used by live code.
use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};
use serc_bytecode::target::local_kinds;

use super::{Body, Code, Id, Instruction, Module, Object, identifier};

#[derive(Default)]
pub(super) struct Stub<'a> {
    names: Vec<&'a str>,
    locals: Vec<&'a str>,
    guard: Option<String>,
}

// A tracing NOP can stand in for a dead suite. At the end of a scope the
// NOP can be folded into the return, leaving no instruction to attach to.
pub(super) type Stubs<'a> = BTreeMap<Option<usize>, Stub<'a>>;

pub(crate) type Hints = FxHashMap<Id, Hint>;

pub(crate) struct Hint {
    names: Vec<usize>,
    locals: Vec<usize>,
    guard: Option<String>,
}

// Compare the first compiled candidate with the input before constructing
// any dead suites. In particular, unused parameters and compiler-generated
// names in an otherwise identical sibling scope need no placeholders.
pub(crate) fn differences(m: &Module<'_>, candidate: &Module<'_>) -> Result<Hints, String> {
    let mut hints = Hints::default();
    let mut pending = vec![(m.root, candidate.root)];
    let mut visited = FxHashSet::default();
    let mut budget = super::MAX_WORK;
    while let Some((id, other)) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let code = m.code(id)?;
        let recovered = candidate.code(other)?;
        if m.text(code.qualname)? != candidate.text(recovered.qualname)? {
            continue;
        }
        let mut missing = |a, b| -> Result<Vec<usize>, String> {
            let a = m.tuple(a)?;
            let b = candidate.tuple(b)?;
            let mut found = FxHashSet::default();
            for &item in b {
                let name = candidate.text(item)?;
                budget = budget
                    .checked_sub(name.len() + 1)
                    .ok_or("synthetic metadata limit exceeded")?;
                found.insert(name);
            }
            let mut absent = Vec::new();
            for (i, &item) in a.iter().enumerate() {
                let name = m.text(item)?;
                budget = budget
                    .checked_sub(name.len() + 1)
                    .ok_or("synthetic metadata limit exceeded")?;
                if !found.contains(name) {
                    absent.push(i);
                }
            }
            Ok(absent)
        };
        let names = missing(code.names, recovered.names)?;
        let mut locals = missing(code.locals, recovered.locals)?;
        let kinds = m.bytes(code.local_kinds)?;
        locals.retain(|&i| kinds.get(i) == Some(&local_kinds::CO_FAST_LOCAL));
        let a = m.tuple(code.constants)?;
        let b = candidate.tuple(recovered.constants)?;
        budget = budget
            .checked_sub(a.len() + b.len())
            .ok_or("synthetic metadata limit exceeded")?;
        let first = guard(m, a.first().copied(), &mut budget);
        let mut guard = if first != guard(candidate, b.first().copied(), &mut budget) {
            first
        } else {
            None
        };
        if guard.is_none()
            && (!names.is_empty() || !locals.is_empty())
            && a.first()
                .is_some_and(|&i| matches!(m.objects[i], Object::None))
        {
            guard = Some("None".into());
        }
        if !names.is_empty() || !locals.is_empty() || guard.is_some() {
            hints.insert(
                id,
                Hint {
                    names,
                    locals,
                    guard,
                },
            );
        }
        // A definition removed altogether has no recoverable body. Pair only
        // corresponding child scopes, including repeated definitions of a name.
        pending.extend(
            a.iter()
                .copied()
                .filter(|&i| matches!(m.objects[i], Object::Code(_)))
                .zip(
                    b.iter()
                        .copied()
                        .filter(|&i| matches!(candidate.objects[i], Object::Code(_))),
                ),
        );
    }
    Ok(hints)
}

// Use only side-effect-free scalar literals. For a truthy retained constant
// the right operand keeps the guard false.
fn guard(m: &Module<'_>, id: Option<Id>, budget: &mut usize) -> Option<String> {
    let id = id?;
    let truthy = match &m.objects[id] {
        Object::None => false,
        Object::Bool(v) => *v,
        Object::Int(v) => *v != 0,
        Object::Float(bits) => f64::from_bits(*bits) != 0.0,
        Object::Text { bytes, .. } | Object::Bytes(bytes) if bytes.len() <= 1024 => {
            !bytes.is_empty()
        }
        Object::Tuple(items) if items.is_empty() => false,
        _ => return None,
    };
    let literal = super::literal(m, id, 0, budget).ok()?;
    Some(if truthy {
        format!("{literal} and False")
    } else {
        literal
    })
}

impl Hint {
    pub(super) fn place<'a>(
        &self,
        m: &'a Module<'_>,
        code: &Code,
        ops: &[Instruction],
    ) -> Result<Stubs<'a>, String> {
        let names = m.tuple(code.names)?;
        let locals = m.tuple(code.locals)?;
        let mut uses = vec![0; names.len()];
        let mut bindings = vec![0; locals.len()];
        let mut nops = Vec::new();
        for op in ops {
            let arg = op.arg as usize;
            let index = match op.name {
                "LOAD_GLOBAL" | "LOAD_ATTR" => Some(arg >> 1),
                "LOAD_SUPER_ATTR" => Some(arg >> 2),
                "LOAD_NAME"
                | "STORE_NAME"
                | "DELETE_NAME"
                | "STORE_GLOBAL"
                | "DELETE_GLOBAL"
                | "LOAD_FROM_DICT_OR_GLOBALS"
                | "STORE_ATTR"
                | "DELETE_ATTR"
                | "IMPORT_NAME"
                | "IMPORT_FROM" => Some(arg),
                _ => None,
            };
            if let Some(slot) = index.and_then(|i| uses.get_mut(i))
                && *slot == 0
            {
                *slot = op.offset;
            }
            let mut bind = |i: usize| {
                if let Some(slot) = bindings.get_mut(i)
                    && *slot == 0
                {
                    *slot = op.offset;
                }
            };
            match op.name {
                "STORE_FAST" | "DELETE_FAST" | "LOAD_FAST_AND_CLEAR" => bind(arg),
                "STORE_FAST_STORE_FAST" => {
                    bind(arg >> 4);
                    bind(arg & 15);
                }
                "STORE_FAST_LOAD_FAST" => bind(arg >> 4),
                "NOP" => nops.push(op.offset),
                _ => {}
            }
        }

        let mut stubs = Stubs::new();
        place_names(m, names, &self.names, &uses, &nops, |site, name| {
            stubs.entry(site).or_default().names.push(name);
        })?;
        place_names(m, locals, &self.locals, &bindings, &nops, |site, name| {
            stubs.entry(site).or_default().locals.push(name);
        })?;
        if let Some(guard) = &self.guard {
            stubs.entry(nops.first().copied()).or_default().guard = Some(guard.clone());
        }
        Ok(stubs)
    }
}

// Place each run of missing indices between its surviving neighbors. Use
// the last tracing NOP before the next operand, or the tail of the scope
// when the final unused names have no NOP of their own.
fn place_names<'a>(
    m: &'a Module<'_>,
    table: &[Id],
    absent: &[usize],
    offsets: &[usize],
    nops: &[usize],
    mut add: impl FnMut(Option<usize>, &'a str),
) -> Result<(), String> {
    for run in absent.chunk_by(|a, b| *b == a + 1) {
        let first = run[0];
        let end = first + run.len();
        let lower = first.checked_sub(1).map_or(0, |prev| offsets[prev]);
        let available = &nops[nops.partition_point(|&n| n < lower)..];
        let site = if let Some(&next) = offsets.get(end) {
            available[..available.partition_point(|&n| n < next)].last()
        } else {
            available.first()
        };
        for &id in &table[first..end] {
            add(site.copied(), identifier(m.text(id)?)?);
        }
    }
    Ok(())
}

impl Body<'_, '_, '_> {
    pub(super) fn emit_stub(&mut self, site: Option<usize>) -> Result<bool, String> {
        let Some(stub) = self.stubs.remove(&site) else {
            return Ok(false);
        };
        // Limit the size of a synthetic assignment or tuple before assembling
        // a line or asking the compiler to parse it.
        if stub.locals.len() + stub.names.len() > 128 {
            return Err("synthetic statement operand limit exceeded".into());
        }
        let rhs = if stub.locals.is_empty() {
            "pass"
        } else {
            "None"
        };
        let mut out = format!("if {}: ", stub.guard.as_deref().unwrap_or("False"));
        for local in stub.locals {
            out.push_str(local);
            out.push_str(" = ");
        }
        if stub.names.is_empty() {
            out.push_str(rhs);
        } else {
            out.push_str(&stub.names.join(", "));
        }
        out.push_str("  # serc: synthetic dead code");
        self.line(&out)?;
        self.work.emitted_stub = true;
        Ok(true)
    }
}
