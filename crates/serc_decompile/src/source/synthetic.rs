//! CPython indexes names before discarding dead instructions. A missing run
//! of operands can therefore survive between two names used by live code.
use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};
use serc_bytecode::target::{code_flags, local_kinds};

use super::{Body, Code, Id, Instruction, Module, Object, identifier};

#[derive(Default)]
pub(super) struct Stub<'a> {
    names: Vec<&'a str>,
    locals: Vec<&'a str>,
    guard: Option<String>,
    generator: bool,
}

// A tracing NOP can stand in for a dead suite. At the end of a scope the
// NOP can be folded into the return, leaving no instruction to attach to.
pub(super) type Stubs<'a> = BTreeMap<Option<usize>, Stub<'a>>;

pub(crate) type Hints = FxHashMap<Id, Hint>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Ordinary,
    DeadSuites,
    LiveGuards,
    LexicalBindings,
}

pub(crate) struct Hint {
    names: Vec<usize>,
    locals: Vec<usize>,
    captures: Vec<usize>,
    guard: Option<Guard>,
    generator: bool,
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
        let kinds = m.bytes(code.local_kinds)?;
        let mut locals = Vec::new();
        let mut captures = Vec::new();
        for i in missing(code.locals, recovered.locals)? {
            match kinds.get(i) {
                Some(&local_kinds::CO_FAST_FREE) => captures.push(i),
                Some(kind)
                    if kind & (local_kinds::CO_FAST_LOCAL | local_kinds::CO_FAST_CELL) != 0
                        && kind & local_kinds::CO_FAST_HIDDEN == 0 =>
                {
                    locals.push(i);
                }
                _ => {}
            }
        }
        let generator_flags = code_flags::CO_GENERATOR | code_flags::CO_ASYNC_GENERATOR;
        let generator = code.flags & generator_flags != 0 && recovered.flags & generator_flags == 0;
        let needs_suite =
            !names.is_empty() || !locals.is_empty() || !captures.is_empty() || generator;
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
            && needs_suite
            && a.first()
                .is_some_and(|&i| matches!(m.objects[i], Object::None))
        {
            guard = Some(Guard {
                literal: "None".into(),
                truthy: false,
            });
        }
        if needs_suite || guard.is_some() {
            hints.insert(
                id,
                Hint {
                    names,
                    locals,
                    captures,
                    guard,
                    generator,
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

#[derive(PartialEq, Eq)]
struct Guard {
    literal: String,
    truthy: bool,
}

// Only literals whose truth value is known without executing Python.
fn guard(m: &Module<'_>, id: Option<Id>, budget: &mut usize) -> Option<Guard> {
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
    Some(Guard { literal, truthy })
}

impl Hint {
    pub(super) fn live_guard(&self, ops: &[Instruction]) -> Option<String> {
        let guard = self.guard.as_ref()?;
        if !self.names.is_empty()
            || !self.locals.is_empty()
            || !self.captures.is_empty()
            || self.generator
            || ops.iter().any(|op| op.name == "NOP")
        {
            return None;
        }
        Some(if guard.truthy {
            guard.literal.clone()
        } else {
            format!("not {}", guard.literal)
        })
    }

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
            local_writes(op, |i| {
                if let Some(slot) = bindings.get_mut(i)
                    && *slot == 0
                {
                    *slot = op.offset;
                }
            });
            if op.name == "NOP" {
                nops.push(op.offset);
            }
        }

        let mut stubs = Stubs::new();
        place_names(m, names, &self.names, &uses, &nops, |site, name| {
            stubs.entry(site).or_default().names.push(name);
        })?;
        place_names(m, locals, &self.locals, &bindings, &nops, |site, name| {
            stubs.entry(site).or_default().locals.push(name);
        })?;
        if self.guard.is_some() || self.generator || !self.captures.is_empty() {
            let stub = stubs.entry(nops.first().copied()).or_default();
            stub.guard = self.guard.as_ref().map(|g| {
                if g.truthy {
                    format!("{} and False", g.literal)
                } else {
                    g.literal.clone()
                }
            });
            stub.generator = self.generator;
            for &i in &self.captures {
                stub.names.push(identifier(m.text(locals[i])?)?);
            }
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

fn local_writes(op: &Instruction, mut bind: impl FnMut(usize)) {
    let arg = op.arg as usize;
    match op.name {
        "STORE_FAST" | "DELETE_FAST" | "STORE_DEREF" | "DELETE_DEREF" | "LOAD_FAST_AND_CLEAR" => {
            bind(arg);
        }
        "STORE_FAST_STORE_FAST" => {
            bind(arg >> 4);
            bind(arg & 15);
        }
        "STORE_FAST_LOAD_FAST" => bind(arg >> 4),
        _ => {}
    }
}

impl Body<'_, '_, '_> {
    // A bare function-local annotation can leave a local or cell slot without
    // a single store. In particular, an inner nonlocal cannot even compile
    // without a lexical binding in its enclosing function.
    pub(super) fn lexical_bindings(&mut self, ops: &[Instruction]) -> Result<(), String> {
        let code = self.code;
        if code.flags & code_flags::CO_OPTIMIZED == 0 || self.m.text(code.name)?.starts_with('<') {
            return Ok(());
        }
        let names = self.m.tuple(code.locals)?;
        let kinds = self.m.bytes(code.local_kinds)?;
        let parameters = code.args
            + code.kw_only
            + usize::from(code.flags & code_flags::CO_VARARGS != 0)
            + usize::from(code.flags & code_flags::CO_VARKEYWORDS != 0);
        if names.len() != kinds.len() || parameters > names.len() {
            return Err("invalid parameter layout".into());
        }
        self.charge(names.len() + ops.len())?;
        let mut written = vec![false; names.len()];
        for op in ops
            .iter()
            .filter(|op| !self.omitted_cell_stores.contains(&op.offset))
        {
            local_writes(op, |i| {
                if let Some(slot) = written.get_mut(i) {
                    *slot = true;
                }
            });
        }
        let mut missing = Vec::new();
        for i in parameters..names.len() {
            if !written[i]
                && kinds[i] & (local_kinds::CO_FAST_LOCAL | local_kinds::CO_FAST_CELL) != 0
                && kinds[i] & (local_kinds::CO_FAST_FREE | local_kinds::CO_FAST_HIDDEN) == 0
            {
                missing.push(identifier(self.m.text(names[i])?)?);
            }
        }
        for chunk in missing.chunks(128) {
            self.line(&format!(
                "if False: {} = None  # serc: synthetic dead code",
                chunk.join(" = ")
            ))?;
            self.work.emitted = true;
        }
        Ok(())
    }

    // Keep the live statement on its original line. Putting it on a new line
    // can cause CPython to insert a tracing NOP before it.
    pub(super) fn guard_statement(&mut self, guard: &str) -> Result<(), String> {
        use ruff_python_ast::Stmt;
        self.charge(self.lines.len())?;
        let parsed = ruff_python_parser::parse_module(&self.lines)
            .map_err(|_| "unsupported live guard body")?;
        let range = parsed
            .suite()
            .iter()
            .find_map(|stmt| match stmt {
                Stmt::Assign(s) => Some(s.range),
                Stmt::AugAssign(s) => Some(s.range),
                Stmt::Expr(s) if !s.value.is_string_literal_expr() => Some(s.range),
                Stmt::Raise(s) => Some(s.range),
                Stmt::Return(s) => Some(s.range),
                _ => None,
            })
            .ok_or("missing live guard statement")?;
        let start = range.start().to_usize();
        let end = range.end().to_usize();
        if self.lines[start..end].contains('\n') || !self.lines[end..].starts_with('\n') {
            return Err("unsupported multiline live guard".into());
        }
        let prefix = format!("if {guard}: ");
        let marker = "  # serc: synthetic live guard";
        self.charge(prefix.len() + marker.len())?;
        if self.lines.len() + prefix.len() + marker.len() > super::MAX_SOURCE {
            return Err("source size limit exceeded".into());
        }
        self.lines.insert_str(end, marker);
        self.lines.insert_str(start, &prefix);
        self.work.emitted = true;
        Ok(())
    }

    pub(super) fn emit_stub(&mut self, site: Option<usize>) -> Result<bool, String> {
        let Some(stub) = self.stubs.remove(&site) else {
            return Ok(false);
        };
        // Limit the size of a synthetic assignment or tuple before assembling
        // a line or asking the compiler to parse it.
        if stub.locals.len() + stub.names.len() > 128 {
            return Err("synthetic statement operand limit exceeded".into());
        }
        let mut value = stub.names.join(", ");
        if stub.generator {
            value = if value.is_empty() {
                "(yield)".into()
            } else {
                format!("(yield {value})")
            };
        } else if value.is_empty() {
            value = if stub.locals.is_empty() {
                "pass"
            } else {
                "None"
            }
            .into();
        }
        // An implicit None return can carry the last dead suite's line.
        // Let that suite end the function instead of adding another NOP.
        if site.is_none()
            && let Some(prefix) = self
                .lines
                .strip_suffix("return\n")
                .or_else(|| self.lines.strip_suffix("return None\n"))
            && (prefix.is_empty() || prefix.ends_with('\n'))
        {
            self.lines.truncate(prefix.len());
        }
        let mut out = format!("if {}: ", stub.guard.as_deref().unwrap_or("False"));
        for local in stub.locals {
            out.push_str(local);
            out.push_str(" = ");
        }
        out.push_str(&value);
        out.push_str("  # serc: synthetic dead code");
        self.line(&out)?;
        self.work.emitted |= self.work.mode == Mode::DeadSuites;
        Ok(true)
    }
}
