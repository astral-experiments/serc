//! Match a retained definition to its recompiled exception table before
//! attaching a warning. The original cache does not record its optimize level.
use std::collections::BTreeMap;

use ruff_python_ast::statement_visitor::{StatementVisitor, walk_stmt};
use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{Stmt, token::Tokens};
use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;
use rustc_hash::{FxHashMap, FxHashSet};
use serc_bytecode::OptimizationLevel;
use serc_bytecode::marshal::reader::read;
use serc_bytecode::target::code_flags;

use super::{Candidate, Id, MAX_WORK, Module, Object, identifier};

pub(super) const WARNING: &str =
    "  # serc: exception table differs after recompilation (see original blocks)";

fn children<'a>(
    m: &'a Module<'_>,
    id: Id,
    budget: &mut usize,
) -> Option<BTreeMap<&'a str, Vec<Id>>> {
    let constants = m.tuple(m.code(id).ok()?.constants).ok()?;
    *budget = budget.checked_sub(1 + constants.len())?;
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for &child in constants {
        if let Object::Code(code) = &m.objects[child] {
            let name = m.text(code.qualname).ok()?;
            *budget = budget.checked_sub(name.len())?;
            groups.entry(name).or_default().push(child);
        }
    }
    Some(groups)
}

/// Pair children within their parent in constant-table order. If a repeated
/// definition was dropped, its remaining namesakes cannot be paired reliably.
fn differences(m: &Module<'_>, candidate: &Module<'_>) -> Option<FxHashMap<Id, usize>> {
    let mut pending = vec![(m.root, candidate.root)];
    let mut seen = FxHashSet::default();
    let mut different = FxHashMap::default();
    let mut budget = MAX_WORK;
    while let Some((id, other)) = pending.pop() {
        if !seen.insert((id, other)) {
            continue;
        }
        let code = m.code(id).ok()?;
        let recovered = candidate.code(other).ok()?;
        let left = m.bytes(code.exceptions).ok()?;
        let right = candidate.bytes(recovered.exceptions).ok()?;
        budget = budget.checked_sub(1 + left.len() + right.len())?;
        if left != right
            && code.flags & code_flags::CO_OPTIMIZED != 0
            && identifier(m.text(code.name).ok()?).is_ok()
        {
            different.insert(id, recovered.first_line);
        }
        let mut peers = children(candidate, other, &mut budget)?;
        for (name, ids) in children(m, id, &mut budget)? {
            if let Some(others) = peers.remove(name)
                && ids.len() == others.len()
            {
                pending.extend(ids.into_iter().zip(others));
            }
        }
    }
    Some(different)
}

struct Headers<'a> {
    locations: FxHashMap<(usize, &'a str), Id>,
    lines: LineIndex,
    tokens: &'a Tokens,
    edits: Vec<(usize, Id)>,
}

impl<'a> StatementVisitor<'a> for Headers<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if let Stmt::FunctionDef(def) = stmt {
            let start = def
                .decorator_list
                .first()
                .map_or(def.start(), Ranged::start);
            let line = self.lines.line_index(start).get();
            if let Some(&id) = self.locations.get(&(line, def.name.as_str()))
                && let Some(newline) = self
                    .tokens
                    .after(def.name.end())
                    .iter()
                    .find(|token| token.kind() == TokenKind::Newline)
            {
                // A logical newline follows the complete header, including
                // multiline defaults, or the complete suite of a one-liner.
                self.edits.push((newline.start().to_usize(), id));
            }
        }
        walk_stmt(self, stmt);
    }
}

pub(super) fn annotate(
    m: &Module<'_>,
    candidate: &mut Candidate,
    mut compile: impl FnMut(&str, OptimizationLevel) -> Option<Vec<u8>>,
) -> Vec<Id> {
    let explicit: FxHashSet<_> = candidate.unplaced.iter().map(|gap| gap.scope).collect();
    let mut remaining: Option<FxHashMap<Id, usize>> = None;
    for level in [
        OptimizationLevel::Zero,
        OptimizationLevel::One,
        OptimizationLevel::Two,
    ] {
        let Some(bytes) = compile(&candidate.source, level) else {
            return Vec::new();
        };
        let Ok(other) = read(&bytes) else {
            return Vec::new();
        };
        if m.version != other.version {
            return Vec::new();
        }
        let Some(mut differences) = differences(m, &other) else {
            return Vec::new();
        };
        differences.retain(|id, _| !explicit.contains(id));
        if let Some(remaining) = &mut remaining {
            remaining.retain(|id, line| differences.get(id) == Some(line));
        } else {
            remaining = Some(differences);
        }
        if remaining.as_ref().is_none_or(FxHashMap::is_empty) {
            return Vec::new();
        }
    }
    let Some(remaining) = remaining else {
        return Vec::new();
    };
    let Ok(parsed) = ruff_python_parser::parse_module(&candidate.source) else {
        return Vec::new();
    };
    let mut headers = Headers {
        locations: remaining
            .into_iter()
            .filter_map(|(id, line)| Some(((line, m.text(m.code(id).ok()?.name).ok()?), id)))
            .collect(),
        lines: LineIndex::from_source_text(&candidate.source),
        tokens: parsed.tokens(),
        edits: Vec::new(),
    };
    headers.visit_body(parsed.suite());
    let mut source =
        String::with_capacity(candidate.source.len() + headers.edits.len() * WARNING.len());
    let mut start = 0;
    let mut warned = Vec::new();
    for (end, id) in headers.edits {
        source.push_str(&candidate.source[start..end]);
        source.push_str(WARNING);
        warned.push(id);
        start = end;
    }
    source.push_str(&candidate.source[start..]);
    candidate.source = source;
    warned
}
