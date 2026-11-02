//! Match each scope to its recompiled control flow before
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

const PREFIX: &str = "  # serc: ";
const SUFFIX: &str = " after recompilation (see original blocks)";

#[derive(Clone, Copy)]
pub(super) struct Difference {
    // The module has no definition line to annotate.
    line: Option<usize>,
    exceptions: bool,
    flow: bool,
}

impl Difference {
    fn confirmed(self) -> bool {
        self.exceptions || self.flow
    }

    pub(super) fn description(self) -> &'static str {
        match (self.exceptions, self.flow) {
            (true, true) => "exception table and normal control flow differ",
            (true, false) => "exception table differs",
            (false, true) => "normal control flow differs",
            (false, false) => unreachable!("only confirmed differences are annotated"),
        }
    }

    pub(super) fn annotation_len(self) -> usize {
        if self.line.is_some() {
            PREFIX.len() + self.description().len() + SUFFIX.len()
        } else {
            0
        }
    }

    pub(super) fn module_note(self) -> String {
        format!("# serc: module {}{SUFFIX}", self.description())
    }
}

pub(super) type Warnings = BTreeMap<Id, Difference>;

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
fn differences(
    m: &Module<'_>,
    candidate: &Module<'_>,
    explicit: &FxHashSet<Id>,
) -> Option<Warnings> {
    let mut pending = vec![(m.root, candidate.root)];
    let mut seen = FxHashSet::default();
    let mut different = Warnings::default();
    let mut budget = MAX_WORK;
    let mut function_flow_budget = MAX_WORK;
    let mut module_flow_budget = MAX_WORK;
    while let Some((id, other)) = pending.pop() {
        if !seen.insert((id, other)) {
            continue;
        }
        let code = m.code(id).ok()?;
        let recovered = candidate.code(other).ok()?;
        let left = m.bytes(code.exceptions).ok()?;
        let right = candidate.bytes(recovered.exceptions).ok()?;
        budget = budget.checked_sub(1 + left.len() + right.len())?;
        if id == m.root
            || (!explicit.contains(&id)
                && code.flags & code_flags::CO_OPTIMIZED != 0
                && identifier(m.text(code.name).ok()?).is_ok())
        {
            let original = m.bytes(code.bytecode).ok()?;
            let compiled = candidate.bytes(recovered.bytecode).ok()?;
            // Reserve a separate budget so graph comparison cannot displace
            // an otherwise reportable exception-table mismatch.
            let flow_budget = if id == m.root {
                &mut module_flow_budget
            } else {
                &mut function_flow_budget
            };
            let flow = if let Some(rest) = flow_budget.checked_sub(original.len() + compiled.len())
            {
                *flow_budget = rest;
                original != compiled
                    && crate::flow::same_normal_flow(m.version, original, compiled) == Ok(false)
            } else {
                false
            };
            let difference = Difference {
                line: (id != m.root).then_some(recovered.first_line),
                exceptions: left != right,
                flow,
            };
            if difference.confirmed() {
                different.insert(id, difference);
            }
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
) -> Warnings {
    let explicit: FxHashSet<_> = candidate.unplaced.iter().map(|gap| gap.scope).collect();
    let mut remaining: Option<Warnings> = None;
    for level in [
        OptimizationLevel::Zero,
        OptimizationLevel::One,
        OptimizationLevel::Two,
    ] {
        let Some(bytes) = compile(&candidate.source, level) else {
            return Warnings::default();
        };
        let Ok(other) = read(&bytes) else {
            return Warnings::default();
        };
        if m.version != other.version {
            return Warnings::default();
        }
        let Some(differences) = differences(m, &other, &explicit) else {
            return Warnings::default();
        };
        if let Some(remaining) = &mut remaining {
            remaining.retain(|id, previous| {
                let Some(current) = differences.get(id) else {
                    return false;
                };
                previous.exceptions &= current.exceptions;
                previous.flow &= current.flow;
                previous.line == current.line && previous.confirmed()
            });
        } else {
            remaining = Some(differences);
        }
        if remaining.as_ref().is_none_or(Warnings::is_empty) {
            return Warnings::default();
        }
    }
    let Some(remaining) = remaining else {
        return Warnings::default();
    };
    let Ok(parsed) = ruff_python_parser::parse_module(&candidate.source) else {
        return Warnings::default();
    };
    let mut headers = Headers {
        locations: remaining
            .iter()
            .filter_map(|(&id, diff)| Some(((diff.line?, m.text(m.code(id).ok()?.name).ok()?), id)))
            .collect(),
        lines: LineIndex::from_source_text(&candidate.source),
        tokens: parsed.tokens(),
        edits: Vec::new(),
    };
    headers.visit_body(parsed.suite());
    let mut source = String::with_capacity(
        candidate.source.len()
            + remaining
                .values()
                .map(|d| d.annotation_len())
                .sum::<usize>(),
    );
    let mut start = 0;
    let mut warned = Warnings::default();
    if let Some(&difference) = remaining.get(&m.root) {
        warned.insert(m.root, difference);
    }
    for (end, id) in headers.edits {
        source.push_str(&candidate.source[start..end]);
        let difference = remaining[&id];
        source.push_str(PREFIX);
        source.push_str(difference.description());
        source.push_str(SUFFIX);
        warned.insert(id, difference);
        start = end;
    }
    source.push_str(&candidate.source[start..]);
    candidate.source = source;
    warned
}
