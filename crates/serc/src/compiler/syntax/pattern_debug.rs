//! Match-pattern validation in the order used by CPython for reserved names.

use ruff_python_ast::visitor::{Visitor, walk_pattern, walk_stmt};
use ruff_python_ast::{self as ast, Pattern, Stmt};
use ruff_python_parser::semantic_errors::{
    SemanticSyntaxContext, SemanticSyntaxError, SemanticSyntaxErrorKind,
};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::CompileError;

use super::Validator;
use super::pattern_numeric::LargeNumericKeys;

#[derive(Default)]
struct PatternNames<'ast> {
    ordered: Vec<&'ast str>,
    names: FxHashSet<&'ast str>,
}

impl<'ast> PatternNames<'ast> {
    fn insert(&mut self, name: &'ast str) -> bool {
        if self.names.insert(name) {
            self.ordered.push(name);
            true
        } else {
            false
        }
    }
}

/// Use parsed identifiers: Python normalizes their spelling before validating names.
pub(super) fn has_debug_pattern(statement: &ast::StmtMatch) -> bool {
    struct Finder(bool);
    impl<'ast> Visitor<'ast> for Finder {
        fn visit_pattern(&mut self, pattern: &'ast Pattern) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_pattern(pattern));
            }
            if self.0 {
                return;
            }
            self.0 = match pattern {
                Pattern::MatchClass(class) => class
                    .arguments
                    .keywords
                    .iter()
                    .any(|keyword| keyword.attr.as_str() == "__debug__"),
                Pattern::MatchAs(pattern) => pattern
                    .name
                    .as_ref()
                    .is_some_and(|name| name == "__debug__"),
                Pattern::MatchStar(pattern) => pattern
                    .name
                    .as_ref()
                    .is_some_and(|name| name == "__debug__"),
                Pattern::MatchMapping(pattern) => pattern
                    .rest
                    .as_ref()
                    .is_some_and(|name| name == "__debug__"),
                _ => false,
            };
            if !self.0 {
                walk_pattern(self, pattern);
            }
        }
        fn visit_expr(&mut self, _expression: &'ast ast::Expr) {}
    }
    let mut finder = Finder(false);
    for case in &statement.cases {
        finder.visit_pattern(&case.pattern);
    }
    finder.0
}

pub(super) fn body_has_debug_pattern(body: &[Stmt]) -> bool {
    struct Finder(bool);
    impl<'ast> Visitor<'ast> for Finder {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            if self.0 {
                return;
            }
            if let Stmt::Match(statement) = statement
                && has_debug_pattern(statement)
            {
                self.0 = true;
            } else {
                walk_stmt(self, statement);
            }
        }
        fn visit_expr(&mut self, expression: &'ast ast::Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            ruff_python_ast::visitor::walk_expr(self, expression);
        }
        fn visit_pattern(&mut self, pattern: &'ast Pattern) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_pattern(pattern));
            }
            walk_pattern(self, pattern);
        }
    }
    let mut finder = Finder(false);
    finder.visit_body(body);
    finder.0
}

impl<'ast, const PRUNE_STATEMENTS: bool, const FUTURE_ONLY: bool>
    Validator<'_, 'ast, PRUNE_STATEMENTS, FUTURE_ONLY>
{
    pub(super) fn visit_debug_match(
        &mut self,
        statement: &'ast Stmt,
        matched: &'ast ast::StmtMatch,
    ) {
        // Retain the shared checker's state and diagnostics while postponing its eager pattern
        // checks until the corresponding native pattern event is reached.
        *self.collected_match_errors.borrow_mut() = Some(Vec::new());
        if self.statement_needs_checker(statement) {
            self.check_statement(statement);
        }
        let errors = self
            .collected_match_errors
            .borrow_mut()
            .take()
            .unwrap_or_default();
        if let Some(error) = errors.iter().find(|error| {
            !matches!(
                error.kind,
                SemanticSyntaxErrorKind::IrrefutableCasePattern(_)
                    | SemanticSyntaxErrorKind::MultipleCaseAssignment(_)
                    | SemanticSyntaxErrorKind::MultipleStarredNamesInSequencePattern
                    | SemanticSyntaxErrorKind::DuplicateMatchKey(_)
                    | SemanticSyntaxErrorKind::DuplicateMatchClassAttribute(_)
                    | SemanticSyntaxErrorKind::DifferentMatchPatternBindings
                    | SemanticSyntaxErrorKind::WriteToDebug(_)
            )
        }) {
            // Do not discard a new checker diagnostic if the pinned checker changes.
            self.report_semantic_error(error.clone());
            if self.error.borrow().is_some() {
                return;
            }
        }
        let duplicate_keys = errors
            .iter()
            .filter_map(|error| {
                matches!(error.kind, SemanticSyntaxErrorKind::DuplicateMatchKey(_))
                    .then_some((error.range, error))
            })
            .collect();
        self.visit_expr(&matched.subject);
        for (index, case) in matched.cases.iter().enumerate() {
            if self.error.borrow().is_some() {
                break;
            }
            if self.pre_lowering_only {
                self.visit_symbol_pattern(&case.pattern);
            } else {
                let mut names = PatternNames::default();
                self.visit_codegen_pattern(
                    &case.pattern,
                    case.guard.is_some() || index + 1 == matched.cases.len(),
                    &mut names,
                    &duplicate_keys,
                );
            }
            if self.error.borrow().is_some() {
                break;
            }
            if let Some(guard) = &case.guard {
                self.visit_expr(guard);
            }
            if self.error.borrow().is_some() {
                break;
            }
            self.visit_body(&case.body);
        }
    }

    fn visit_symbol_pattern(&mut self, pattern: &'ast Pattern) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_symbol_pattern(pattern));
        }
        if self.error.borrow().is_some() {
            return;
        }
        match pattern {
            Pattern::MatchValue(value) => self.visit_expr(&value.value),
            Pattern::MatchSingleton(_) => {}
            Pattern::MatchSequence(sequence) => {
                for child in &sequence.patterns {
                    self.visit_symbol_pattern(child);
                }
            }
            Pattern::MatchMapping(mapping) => {
                for key in &mapping.keys {
                    self.visit_expr(key);
                }
                for child in &mapping.patterns {
                    self.visit_symbol_pattern(child);
                }
                if let Some(name) = &mapping.rest {
                    self.check_symbol_pattern_name(name.as_str(), pattern.range());
                }
            }
            Pattern::MatchClass(class) => {
                self.visit_expr(&class.cls);
                for child in &class.arguments.patterns {
                    self.visit_symbol_pattern(child);
                }
                for keyword in &class.arguments.keywords {
                    self.check_symbol_pattern_name(keyword.attr.as_str(), keyword.pattern.range());
                }
                for keyword in &class.arguments.keywords {
                    self.visit_symbol_pattern(&keyword.pattern);
                }
            }
            Pattern::MatchStar(star) => {
                if let Some(name) = &star.name {
                    self.check_symbol_pattern_name(name.as_str(), pattern.range());
                }
            }
            Pattern::MatchAs(as_pattern) => {
                if let Some(inner) = &as_pattern.pattern {
                    self.visit_symbol_pattern(inner);
                }
                if let Some(name) = &as_pattern.name {
                    self.check_symbol_pattern_name(name.as_str(), pattern.range());
                }
            }
            Pattern::MatchOr(or_pattern) => {
                for child in &or_pattern.patterns {
                    self.visit_symbol_pattern(child);
                }
            }
        }
    }

    fn check_symbol_pattern_name(&mut self, name: &str, range: TextRange) {
        let previously_reported = self.error.borrow().is_some();
        self.check_debug_write(name, ast::ExprContext::Store, range);
        if name == "__debug__" && !previously_reported && self.error.borrow().is_some() {
            self.symbol_pattern_debug_range = Some(range);
        }
    }

    fn pattern_error(&mut self, message: impl Into<String>, range: TextRange) {
        if self.error.borrow().is_none() && self.bare_starred.is_none() {
            self.lowering_pattern_error = true;
            *self.error.borrow_mut() = Some(CompileError::InvalidPattern {
                message: message.into(),
                range,
            });
        }
    }

    fn store_pattern_name(
        &mut self,
        name: &'ast ast::Identifier,
        range: TextRange,
        names: &mut PatternNames<'ast>,
    ) {
        if self.python_version < ast::PythonVersion::PY314 {
            self.check_debug_write(name.as_str(), ast::ExprContext::Store, range);
        }
        if self.error.borrow().is_some() {
            return;
        }
        if !names.insert(name.as_str()) {
            self.pattern_error(
                format!(
                    "multiple assignments to name '{}' in pattern",
                    name.as_str()
                ),
                range,
            );
        }
    }

    fn visit_codegen_pattern(
        &mut self,
        pattern: &'ast Pattern,
        allow_irrefutable: bool,
        names: &mut PatternNames<'ast>,
        errors: &FxHashMap<TextRange, &SemanticSyntaxError>,
    ) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| {
                self.visit_codegen_pattern(pattern, allow_irrefutable, names, errors);
            });
        }
        if self.error.borrow().is_some() {
            return;
        }
        match pattern {
            Pattern::MatchValue(value) => self.visit_expr(&value.value),
            Pattern::MatchSingleton(_) => {}
            Pattern::MatchSequence(sequence) => {
                if sequence
                    .patterns
                    .iter()
                    .filter(|child| child.is_match_star())
                    .count()
                    > 1
                {
                    self.pattern_error(
                        "multiple starred names in sequence pattern",
                        pattern.range(),
                    );
                    return;
                }
                for child in &sequence.patterns {
                    self.visit_codegen_pattern(child, true, names, errors);
                }
            }
            Pattern::MatchMapping(mapping) => {
                let mut numeric_keys = LargeNumericKeys::default();
                for key in &mapping.keys {
                    if let Some(error) = errors.get(&key.range()) {
                        let message = super::mapping_diagnostic::diagnostic(
                            self.source,
                            key,
                            pattern.range(),
                            self.python_version,
                        )
                        .map_or_else(|| error.to_string(), |(message, _)| message);
                        self.pattern_error(message, pattern.range());
                        return;
                    }
                    if numeric_keys.duplicate(key, self.python_version) {
                        let message = super::mapping_diagnostic::diagnostic(
                            self.source,
                            key,
                            pattern.range(),
                            self.python_version,
                        )
                        .map_or_else(
                            || {
                                format!(
                                    "mapping pattern checks duplicate key `{}`",
                                    &self.source[key.range()]
                                )
                            },
                            |(message, _)| message,
                        );
                        self.pattern_error(message, pattern.range());
                        return;
                    }
                    self.visit_expr(key);
                    if self.error.borrow().is_some() {
                        return;
                    }
                }
                for child in &mapping.patterns {
                    self.visit_codegen_pattern(child, true, names, errors);
                }
                if let Some(name) = &mapping.rest {
                    self.store_pattern_name(name, pattern.range(), names);
                }
            }
            Pattern::MatchClass(class) => {
                let mut next = FxHashMap::default();
                let mut invalid = None;
                for keyword in class.arguments.keywords.iter().rev() {
                    let name = keyword.attr.as_str();
                    let repeated = next.insert(name, keyword.pattern.range());
                    if self.python_version < ast::PythonVersion::PY314 && name == "__debug__" {
                        invalid = Some((name, keyword.pattern.range(), true));
                    } else if let Some(range) = repeated {
                        invalid = Some((name, range, false));
                    }
                }
                if let Some((name, range, debug)) = invalid {
                    if debug {
                        self.check_debug_write(name, ast::ExprContext::Store, range);
                    } else {
                        self.pattern_error(
                            format!("attribute name repeated in class pattern: {name}"),
                            range,
                        );
                    }
                    return;
                }
                self.visit_expr(&class.cls);
                for child in &class.arguments.patterns {
                    self.visit_codegen_pattern(child, true, names, errors);
                }
                for keyword in &class.arguments.keywords {
                    self.visit_codegen_pattern(&keyword.pattern, true, names, errors);
                }
            }
            Pattern::MatchStar(star) => {
                if let Some(name) = &star.name {
                    self.store_pattern_name(name, pattern.range(), names);
                }
            }
            Pattern::MatchAs(as_pattern) => {
                if let Some(inner) = &as_pattern.pattern {
                    self.visit_codegen_pattern(inner, allow_irrefutable, names, errors);
                } else if !allow_irrefutable {
                    let message = as_pattern.name.as_ref().map_or_else(
                        || "wildcard makes remaining patterns unreachable".to_owned(),
                        |name| {
                            format!(
                                "name capture '{}' makes remaining patterns unreachable",
                                name.as_str()
                            )
                        },
                    );
                    self.pattern_error(message, pattern.range());
                }
                if let Some(name) = &as_pattern.name {
                    self.store_pattern_name(name, pattern.range(), names);
                }
            }
            Pattern::MatchOr(or_pattern) => {
                let mut control: Option<PatternNames<'ast>> = None;
                for (index, arm) in or_pattern.patterns.iter().enumerate() {
                    let mut current = PatternNames::default();
                    self.visit_codegen_pattern(
                        arm,
                        allow_irrefutable && index + 1 == or_pattern.patterns.len(),
                        &mut current,
                        errors,
                    );
                    if self.error.borrow().is_some() {
                        return;
                    }
                    if let Some(control) = &control {
                        if control.names != current.names {
                            self.pattern_error(
                                "alternative patterns bind different names",
                                pattern.range(),
                            );
                            return;
                        }
                    } else {
                        control = Some(current);
                    }
                }
                for name in control.unwrap_or_default().ordered {
                    if !names.insert(name) {
                        self.pattern_error(
                            format!("multiple assignments to name '{name}' in pattern"),
                            pattern.range(),
                        );
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;
    use ruff_text_size::{TextRange, TextSize};

    use crate::{CompileError, PythonVersion};

    use super::super::validate;

    #[test]
    fn reserved_names_follow_native_pattern_and_symbol_order() {
        let debug = "cannot assign to __debug__";
        let duplicate = "attribute name repeated in class pattern: a";
        let cases = [
            (
                "match value:\n case C(__debug__, __debug__=other): pass\n",
                [
                    (debug, "other", false),
                    (debug, "other", false),
                    (debug, "__debug__", false),
                    (debug, "__debug__", false),
                ],
            ),
            (
                "match value:\n case C(__debug__=v, a=1, a=2): pass\n",
                [(debug, "v", true); 4],
            ),
            (
                "match value:\n case C(a=1, a=2, __debug__=v): pass\n",
                [
                    (duplicate, "2", false),
                    (duplicate, "2", false),
                    (debug, "v", true),
                    (debug, "v", true),
                ],
            ),
            (
                "match value:\n case C(D(__debug__=inner), __debug__=outer): pass\n",
                [
                    (debug, "outer", false),
                    (debug, "outer", false),
                    (debug, "inner", false),
                    (debug, "inner", false),
                ],
            ),
            (
                "match value:\n case [C(__debug__=v), *a, *b]: pass\n",
                [
                    (
                        "multiple starred names in sequence pattern",
                        "[C(__debug__=v), *a, *b]",
                        false,
                    ),
                    (
                        "multiple starred names in sequence pattern",
                        "[C(__debug__=v), *a, *b]",
                        false,
                    ),
                    (debug, "v", true),
                    (debug, "v", true),
                ],
            ),
            (
                "match value:\n case C(__debug__=v) if f(x=1, x=2): pass\n",
                [(debug, "v", true); 4],
            ),
            (
                "match value:\n case 1: break\n case C(__debug__=v): pass\n",
                [
                    ("'break' outside loop", "break", false),
                    ("'break' outside loop", "break", false),
                    (debug, "v", true),
                    (debug, "v", true),
                ],
            ),
            (
                "match value:\n case [*__debug__]: pass\n",
                [(debug, "*__debug__", false); 4],
            ),
            (
                "match value:\n case {**__debug__}: pass\n",
                [(debug, "{**__debug__}", false); 4],
            ),
            (
                "match value:\n case C() as __debug__: pass\n",
                [(debug, "C() as __debug__", false); 4],
            ),
            (
                "match value:\n case C(__ｄｅｂｕｇ__=x): pass\n",
                [(debug, "x", true); 4],
            ),
            (
                "match value:\n case C(__debug__=x): pass\nfrom __future__ import annotations\n",
                [(debug, "x", true); 4],
            ),
            (
                "x=1\nglobal x\nmatch value:\n case C(__debug__=z): pass\n",
                [(
                    "name 'x' is assigned to before global declaration",
                    "global x",
                    false,
                ); 4],
            ),
            (
                "match value:\n case C(__debug__=z): pass\nx=1\nglobal x\n",
                [
                    (
                        "name 'x' is assigned to before global declaration",
                        "global x",
                        false,
                    ),
                    (
                        "name 'x' is assigned to before global declaration",
                        "global x",
                        false,
                    ),
                    (debug, "z", false),
                    (debug, "z", false),
                ],
            ),
            (
                "def f():\n nonlocal x\nmatch value:\n case C(__debug__=z): pass\n",
                [
                    ("no binding for nonlocal 'x' found", "nonlocal x", false),
                    ("no binding for nonlocal 'x' found", "nonlocal x", false),
                    (debug, "z", false),
                    (debug, "z", false),
                ],
            ),
            (
                "match value:\n case [x, (C(x) | D(x)), E(__debug__=y)]: pass\n",
                [
                    (
                        "multiple assignments to name 'x' in pattern",
                        "C(x) | D(x)",
                        false,
                    ),
                    (
                        "multiple assignments to name 'x' in pattern",
                        "C(x) | D(x)",
                        false,
                    ),
                    (debug, "y", true),
                    (debug, "y", true),
                ],
            ),
            (
                "match value:\n case [C(x) | D(y), E(__debug__=v)]: pass\n",
                [
                    (
                        "alternative patterns bind different names",
                        "C(x) | D(y)",
                        false,
                    ),
                    (
                        "alternative patterns bind different names",
                        "C(x) | D(y)",
                        false,
                    ),
                    (debug, "v", true),
                    (debug, "v", true),
                ],
            ),
        ];
        let targets = [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ];
        for (source, expected) in cases {
            let parsed = parse_module(source).unwrap();
            for (version, (message, fragment, last)) in targets.into_iter().zip(expected) {
                let position = if last {
                    source.rfind(fragment)
                } else {
                    source.find(fragment)
                }
                .unwrap();
                let range = TextRange::new(
                    TextSize::try_from(position).unwrap(),
                    TextSize::try_from(position + fragment.len()).unwrap(),
                );
                let actual = validate(source, &parsed.syntax().body, version).unwrap_err();
                match actual {
                    CompileError::InvalidPattern {
                        message: actual,
                        range: actual_range,
                    }
                    | CompileError::InvalidSyntax {
                        message: actual,
                        range: actual_range,
                    } => {
                        assert_eq!(
                            (actual.as_str(), actual_range),
                            (message, range),
                            "{version}, {source}"
                        );
                    }
                    other => panic!("unexpected {version} diagnostic for {source}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn large_integer_and_float_keys_precede_legacy_debug_writes() {
        let source = "match value:\n case {18446744073709551616: _, 18446744073709551616.0: C(__debug__=x)}: pass\n";
        let parsed = parse_module(source).unwrap();
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            let error = validate(source, &parsed.syntax().body, version).unwrap_err();
            let CompileError::InvalidPattern { message, range } = error else {
                panic!("expected a duplicate key for {version}: {error:?}");
            };
            assert!(message.starts_with("mapping pattern checks duplicate key"));
            let start = source.find('{').unwrap();
            let end = source.find('}').unwrap() + 1;
            assert_eq!(
                range,
                TextRange::new(
                    TextSize::try_from(start).unwrap(),
                    TextSize::try_from(end).unwrap()
                )
            );
        }
    }
}
