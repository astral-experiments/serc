//! Since 3.13, a class body's first line is also an executable constant.
//! Match class occurrences in source order, including the first decorator,
//! and compact a one-statement suite only when a later class needs the room.
use ruff_python_ast::{
    Stmt,
    token::TokenKind,
    visitor::{Visitor, walk_stmt},
};
use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;
use std::collections::BTreeMap;

use super::{MAX_SOURCE, Module, Object};
use serc_bytecode::target::code_flags::CO_NEWLOCALS;

pub(super) fn class_candidates(m: &Module<'_>, candidates: &mut Vec<String>) -> Result<(), String> {
    let mut targets: BTreeMap<&str, Vec<usize>> = BTreeMap::default();
    for obj in &m.objects {
        if let Object::Code(code) = obj
            && code.flags & CO_NEWLOCALS == 0
            && !m.text(code.name)?.starts_with('<')
        {
            targets
                .entry(m.text(code.name)?)
                .or_default()
                .push(code.first_line);
        }
    }
    if targets.is_empty() {
        return Ok(());
    }
    for lines in targets.values_mut() {
        lines.sort_unstable();
    }
    for i in 0..candidates.len() {
        if let Some(layout) = class_layout(&candidates[i], &targets)
            && !candidates.contains(&layout)
        {
            candidates.push(layout);
        }
    }
    Ok(())
}

struct Suites<'a> {
    index: LineIndex,
    classes: Vec<(usize, &'a str)>,
    inline: Vec<bool>,
}

impl<'a> Suites<'a> {
    fn collect(source: &str, stmts: &'a [Stmt]) -> Self {
        let mut suites = Self {
            index: LineIndex::from_source_text(source),
            classes: Vec::new(),
            inline: vec![false; source.lines().count()],
        };
        for stmt in stmts {
            suites.visit_stmt(stmt);
        }
        suites
    }
}

impl<'a> Visitor<'a> for Suites<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        let body = match stmt {
            Stmt::ClassDef(class) => {
                let start = self.index.line_index(class.range.start()).to_zero_indexed();
                self.classes.push((start, class.name.as_str()));
                Some(class.body.as_slice())
            }
            Stmt::FunctionDef(def) => Some(def.body.as_slice()),
            _ => None,
        };
        if let Some([child]) = body {
            let range = match child {
                Stmt::Pass(s) => Some(s.range),
                Stmt::Return(s) => Some(s.range),
                Stmt::Expr(s) => Some(s.range),
                Stmt::Assign(s) => Some(s.range),
                Stmt::Raise(s) => Some(s.range),
                _ => None,
            };
            if let Some(range) = range {
                let start = self.index.line_index(range.start());
                if self.index.line_index(range.end()) == start {
                    self.inline[start.to_zero_indexed()] = true;
                }
            }
        }
        walk_stmt(self, stmt);
    }
}

fn class_layout(source: &str, targets: &BTreeMap<&str, Vec<usize>>) -> Option<String> {
    let module = ruff_python_parser::parse_module(source).ok()?;
    let lines: Vec<_> = source.lines().collect();
    let suites = Suites::collect(source, module.suite());
    let mut offsets: BTreeMap<&str, usize> = BTreeMap::default();
    let mut result = String::new();
    let mut start = 0;
    let mut lineno = 1;
    for (at, name) in suites.classes {
        let nth = offsets.entry(name).or_default();
        let target = *targets.get(name)?.get(*nth)?;
        *nth += 1;
        let mut excess = (lineno + at - start).saturating_sub(target);
        for i in start..at {
            if excess > 0 && suites.inline[i] && i > start && lines[i - 1].ends_with(':') {
                result.pop();
                result.push(' ');
                result.push_str(lines[i].trim_start());
                excess -= 1;
            } else {
                result.push_str(lines[i]);
                lineno += 1;
            }
            result.push('\n');
        }
        if excess > 0 {
            return None;
        }
        let gap = target.checked_sub(lineno)?;
        if result.len().checked_add(gap)? > MAX_SOURCE {
            return None;
        }
        result.extend(std::iter::repeat_n('\n', gap));
        lineno = target;
        start = at;
    }
    for line in &lines[start..] {
        result.push_str(line);
        result.push('\n');
    }
    (result.len() <= MAX_SOURCE && result != source).then_some(result)
}

/// Align placed class definitions in a partial module. The recompiled header
/// line includes decorators, nested scopes and the best-effort preamble.
pub(super) fn partial(m: &Module<'_>, candidate: &Module<'_>, source: &str) -> Option<String> {
    if candidate.version != m.version {
        return None;
    }
    let mut targets = BTreeMap::new();
    super::paired::visit(m, candidate, |id, other, _| {
        let original = m.code(id).ok()?;
        let recovered = candidate.code(other).ok()?;
        let name = m.text(original.name).ok()?;
        if original.flags & CO_NEWLOCALS == 0
            && recovered.flags & CO_NEWLOCALS == 0
            && !name.starts_with('<')
            && candidate.text(recovered.name).ok()? == name
        {
            targets.insert((recovered.first_line, name), original.first_line);
        }
        Some(())
    })?;
    if targets.is_empty() {
        return None;
    }
    let parsed = ruff_python_parser::parse_module(source).ok()?;
    let suites = Suites::collect(source, parsed.suite());
    let sites = suites
        .classes
        .into_iter()
        .filter_map(|(index, name)| Some((index + 1, *targets.get(&(index + 1, name))?)))
        .collect::<Vec<_>>();
    // Inserting before a class that already has the right line would move it.
    let start = sites
        .iter()
        .rposition(|(line, wanted)| line == wanted)
        .map_or(0, |i| i + 1);
    // Remove standalone blank lines between suites only when needed to
    // preserve a class's line. Newlines inside strings belong to the token.
    let blanks: Vec<_> = parsed
        .tokens()
        .windows(2)
        .filter(|t| {
            matches!(
                t[0].kind(),
                TokenKind::Newline | TokenKind::NonLogicalNewline
            ) && t[1].kind() == TokenKind::NonLogicalNewline
                && t[0].end() == t[1].start()
                && &source[t[1].range()] == "\n"
        })
        .map(|t| t[1].start().to_usize())
        .collect();
    let mut blanks = blanks.as_slice();
    let mut result = String::new();
    let mut copied = 0;
    let mut shift = 0isize;
    for (i, &(line, wanted)) in sites.iter().enumerate() {
        let at = suites.index.line_starts()[line - 1].to_usize();
        let adjusted = line.checked_add_signed(shift)?;
        let (before, rest) = blanks.split_at(blanks.partition_point(|&blank| blank < at));
        blanks = rest;
        let remove = adjusted.saturating_sub(wanted).min(before.len());
        for &blank in &before[before.len() - remove..] {
            result.push_str(&source[copied..blank]);
            copied = blank + 1;
        }
        shift -= isize::try_from(remove).ok()?;
        result.push_str(&source[copied..at]);
        copied = at;
        // Before the last already aligned class, only replace lines we
        // removed. Adding more would shift that class past its original site.
        let target = if i < start { wanted.min(line) } else { wanted };
        let gap = target.saturating_sub(adjusted - remove);
        if (gap <= 1024 || i < start) && source.len().checked_add_signed(shift)? + gap <= MAX_SOURCE
        {
            result.extend(std::iter::repeat_n('\n', gap));
            shift += isize::try_from(gap).ok()?;
        }
    }
    result.push_str(&source[copied..]);
    (result != source).then_some(result)
}
