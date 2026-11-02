//! Since 3.13, a class body's first line is also an executable constant.
//! Match class occurrences in source order, including the first decorator,
//! and compact a one-statement suite only when a later class needs the room.
use ruff_python_ast::{
    Stmt,
    visitor::{Visitor, walk_stmt},
};
use ruff_source_file::LineIndex;
use std::collections::BTreeMap;

use super::{MAX_SOURCE, Module, Object};

pub(super) fn class_candidates(m: &Module<'_>, candidates: &mut Vec<String>) -> Result<(), String> {
    let mut targets: BTreeMap<&str, Vec<usize>> = BTreeMap::default();
    for obj in &m.objects {
        if let Object::Code(code) = obj
            && code.flags & serc_bytecode::target::code_flags::CO_NEWLOCALS == 0
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
    let mut suites = Suites {
        index: LineIndex::from_source_text(source),
        classes: Vec::new(),
        inline: vec![false; lines.len()],
    };
    for stmt in module.suite() {
        suites.visit_stmt(stmt);
    }
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
