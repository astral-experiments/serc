//! Preserve source lines that affect instructions or executable constants.
use ruff_python_ast::{
    Expr, Keyword, MatchCase, Stmt, WithItem,
    token::TokenKind,
    visitor::{Visitor, walk_expr, walk_match_case, walk_stmt},
};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange, TextSize};
use std::collections::BTreeMap;

use super::{MAX_SOURCE, Module, Object};
use serc_bytecode::target::code_flags::CO_NEWLOCALS;

/// The lifter groups operands until precedence is known. For best-effort
/// source, remove redundant grouping without reformatting diagnostic comments
/// or changing physical lines.
pub(super) fn readable_primaries(source: String) -> String {
    if ![")(", ").", ")["]
        .iter()
        .any(|suffix| source.contains(suffix))
    {
        return source;
    }
    let Ok(parsed) = ruff_python_parser::parse_module(&source) else {
        return source;
    };
    let mut visitor = PrimaryGroups {
        source: &source,
        parens: Vec::new(),
    };
    visitor.visit_body(parsed.suite());
    if visitor.parens.is_empty() {
        return source;
    }
    visitor.parens.sort_unstable();
    let mut result = String::with_capacity(source.len() - visitor.parens.len());
    let mut from = 0;
    for at in visitor.parens {
        result.push_str(&source[from..at]);
        from = at + 1;
    }
    result.push_str(&source[from..]);
    result
}

struct PrimaryGroups<'a> {
    source: &'a str,
    parens: Vec<usize>,
}

impl<'a> Visitor<'a> for PrimaryGroups<'_> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        let primary = match expr {
            Expr::Call(call) => Some(call.func.as_ref()),
            Expr::Attribute(attr) => Some(attr.value.as_ref()),
            Expr::Subscript(index) => Some(index.value.as_ref()),
            // Debug fields can include their original spelling in the value.
            Expr::FString(_) | Expr::TString(_) => return,
            _ => None,
        };
        if let Some(
            base @ (Expr::Name(_) | Expr::Call(_) | Expr::Attribute(_) | Expr::Subscript(_)),
        ) = primary
        {
            let open = expr.start().to_usize();
            let close = base.end().to_usize();
            if self.source.get(open..base.start().to_usize()) == Some("(")
                && self.source.as_bytes().get(close) == Some(&b')')
                && !self.source[base.range()].contains(['\r', '\n'])
                && self.source[..open]
                    .chars()
                    .next_back()
                    .is_none_or(|c| c.is_ascii_punctuation() && c != '_' || c.is_whitespace())
            {
                self.parens.extend([open, close]);
            }
        }
        walk_expr(self, expr);
    }
}

#[derive(Default)]
struct SplitItems<'a> {
    keywords: Vec<&'a [Keyword]>,
    managers: Vec<&'a [WithItem]>,
    case_suites: Vec<TextRange>,
}

impl<'a> Visitor<'a> for SplitItems<'a> {
    fn visit_match_case(&mut self, case: &'a MatchCase) {
        if case.guard.is_none()
            && let [body] = case.body.as_slice()
        {
            self.case_suites
                .push(TextRange::new(case.pattern.end(), body.start()));
        }
        walk_match_case(self, case);
    }

    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if let Stmt::With(manager) = stmt
            && manager.items.len() > 1
        {
            self.managers.push(&manager.items);
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Call(call) = expr
            && call.arguments.keywords.len() > 1
        {
            self.keywords.push(&call.arguments.keywords);
        }
        walk_expr(self, expr);
    }
}

/// Preserve line boundaries that affect packed loads or tracing instructions.
pub(super) fn item_lines(raw: &str, suite: &[Stmt], formatted: &str) -> Option<String> {
    if !raw.contains(",\n") && !raw.contains("case _: ") {
        return None;
    }
    let mut before = SplitItems::default();
    before.visit_body(suite);
    let parsed = ruff_python_parser::parse_module(formatted).ok()?;
    let mut after = SplitItems::default();
    after.visit_body(parsed.suite());
    if before.keywords.len() != after.keywords.len()
        || before.managers.len() != after.managers.len()
        || before.case_suites.len() != after.case_suites.len()
    {
        return None;
    }
    let mut edits = Vec::new();
    for (a, b) in before.keywords.into_iter().zip(after.keywords) {
        if a.len() != b.len()
            || !a
                .iter()
                .zip(b)
                .all(|(x, y)| x.arg.as_deref() == y.arg.as_deref())
        {
            return None;
        }
        edits.extend(
            split_offsets(raw, a, formatted, b).map(|at| (at - TextSize::new(1), at, "\n")),
        );
    }
    for (a, b) in before.managers.into_iter().zip(after.managers) {
        if a.len() != b.len() {
            return None;
        }
        edits.extend(
            split_offsets(raw, a, formatted, b).map(|at| (at - TextSize::new(1), at, " \\\n")),
        );
    }
    // Moving a one-line wildcard arm to a new line introduces a tracing NOP.
    for (a, b) in before.case_suites.into_iter().zip(after.case_suites) {
        if &raw[a] == ": " && formatted[b].starts_with(":\n") {
            edits.push((b.start(), b.end(), ": "));
        }
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_unstable();
    let lines = LineIndex::from_source_text(formatted);
    let mut result = String::new();
    let mut from = 0;
    for (start, end, replacement) in edits {
        let at = start.to_usize();
        result.push_str(formatted.get(from..at)?);
        result.push_str(replacement);
        if replacement.ends_with('\n') {
            let line = lines
                .line_start(lines.line_index(start), formatted)
                .to_usize();
            let indent = formatted[line..at]
                .bytes()
                .take_while(|&b| b == b' ')
                .count();
            result.extend(std::iter::repeat_n(' ', indent + 4));
        }
        from = end.to_usize();
        if result.len() > MAX_SOURCE {
            return None;
        }
    }
    result.push_str(&formatted[from..]);
    (result.len() <= MAX_SOURCE).then_some(result)
}

fn split_offsets<'a, T: Ranged>(
    raw: &'a str,
    before: &'a [T],
    formatted: &'a str,
    after: &'a [T],
) -> impl Iterator<Item = TextSize> + 'a {
    before
        .windows(2)
        .zip(after.windows(2))
        .filter_map(|(a, b)| {
            let separator = &raw[a[0].end().to_usize()..a[1].start().to_usize()];
            let offset = b[1].start();
            (separator.contains('\n') && formatted[..offset.to_usize()].ends_with(", "))
                .then_some(offset)
        })
}

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

#[cfg(test)]
mod tests {
    use super::readable_primaries;

    #[test]
    fn removes_only_redundant_groups_around_primaries() {
        let source = "((log).append)((items)[0])  # (log).append\n\
                      (left + right).read()\n\
                      if(flag).value: pass\n\
                      ((log)\n .append)(1)\n\
                      debug = f'{(log).name=}'\n";
        assert_eq!(
            readable_primaries(source.into()),
            "log.append(items[0])  # (log).append\n\
             (left + right).read()\n\
             if(flag).value: pass\n\
             (log\n .append)(1)\n\
             debug = f'{(log).name=}'\n"
        );
    }
}
