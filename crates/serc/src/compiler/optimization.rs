//! Prepare the code-generation AST for Python's explicit optimization levels.

use std::cell::RefCell;

use ruff_python_ast::visitor::Visitor;
use ruff_python_ast::visitor::transformer::{Transformer, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, Expr, ExprContext, Stmt, Suite};
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustc_hash::FxHashMap;

use crate::{OptimizationLevel, PythonVersion};

use super::scope::future_feature_flags;
use crate::target::code_flags::CO_FUTURE_ANNOTATIONS;

/// Transform an owned tree after preserving its unoptimized binding facts.
pub(crate) fn prepare(
    body: Suite,
    level: OptimizationLevel,
    python_version: PythonVersion,
) -> OptimizedSuite {
    let mut optimized = OptimizedSuite {
        body,
        locations: FxHashMap::default(),
    };
    let optimizer = Optimizer {
        strip_docstrings: level == OptimizationLevel::Two,
        future_annotations: future_feature_flags(&optimized.body) & CO_FUTURE_ANNOTATIONS != 0,
        python_version,
        locations: RefCell::default(),
    };
    optimizer.scope_body(&mut optimized.body);
    optimizer.visit_body(&mut optimized.body);
    optimized.locations = optimizer.locations.into_inner();
    optimized
}

#[cfg(test)]
thread_local! {
    pub(crate) static FORCE_CLONED_AST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn cloned_body_for_test(body: &Suite) -> Option<Suite> {
    if !FORCE_CLONED_AST.get() {
        return None;
    }
    let mut depth = crate::stack::ExpressionDepth::default();
    depth.visit_body(body);
    crate::stack::with_depth(depth.maximum(), || Some(body.clone()))
}

/// Keep the original tree when none of the preparation transforms can apply.
pub(crate) fn requires_optimization(body: &Suite, level: OptimizationLevel) -> bool {
    struct Changes {
        strip_docstrings: bool,
        found: bool,
    }

    impl Changes {
        fn scope_body(&mut self, body: &[Stmt]) {
            self.found |= self.strip_docstrings
                && matches!(body.first(), Some(Stmt::Expr(statement))
                    if statement.value.is_string_literal_expr());
        }
    }

    impl<'ast> Visitor<'ast> for Changes {
        fn visit_body(&mut self, body: &'ast [Stmt]) {
            for statement in body {
                if self.found {
                    break;
                }
                self.visit_stmt(statement);
            }
        }

        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if self.found {
                return;
            }
            match statement {
                Stmt::Assert(_) => self.found = true,
                Stmt::FunctionDef(function) => self.scope_body(&function.body),
                Stmt::ClassDef(class) => self.scope_body(&class.body),
                _ => {}
            }
            if !self.found {
                ruff_python_ast::visitor::walk_stmt(self, statement);
            }
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.found {
                return;
            }
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            if matches!(expression, Expr::Name(name)
                if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__")
            {
                self.found = true;
            } else {
                ruff_python_ast::visitor::walk_expr(self, expression);
            }
        }
    }

    if level == OptimizationLevel::Zero {
        return false;
    }

    let mut changes = Changes {
        strip_docstrings: level == OptimizationLevel::Two,
        found: false,
    };
    changes.scope_body(body);
    changes.visit_body(body);
    changes.found
}

pub(crate) struct OptimizedSuite {
    pub(crate) body: Suite,
    pub(super) locations: FxHashMap<TextRange, OptimizedLocation>,
}

impl Drop for OptimizedSuite {
    fn drop(&mut self) {
        let shallow_drop = stacker::remaining_stack()
            .is_some_and(|remaining| remaining >= crate::STACK_SEGMENT_SIZE);
        ClearExpressions { shallow_drop }.visit_body(&mut self.body);
    }
}

struct ClearExpressions {
    shallow_drop: bool,
}

impl Transformer for ClearExpressions {
    fn visit_expr(&self, expression: &mut Expr) {
        // Leaves cannot recurse during destruction, so their parent can drop them.
        if matches!(
            expression,
            Expr::Name(_)
                | Expr::NumberLiteral(_)
                | Expr::StringLiteral(_)
                | Expr::BytesLiteral(_)
                | Expr::BooleanLiteral(_)
                | Expr::NoneLiteral(_)
                | Expr::EllipsisLiteral(_)
        ) {
            return;
        }
        // Optimization never deepens expressions, so short source ranges retain the
        // parsed tree's shallow-drop bound when a full stack segment is available.
        if self.shallow_drop && expression.range().len() <= TextSize::new(256) {
            return;
        }
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_expr(expression));
        }
        walk_expr(self, expression);
        // Clear children before replacing their parent so destruction cannot recurse
        // through a deeply nested expression after the compilation stack has unwound.
        *expression = Expr::NoneLiteral(ast::ExprNoneLiteral::default());
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum OptimizedLocation {
    Elided,
    SyntheticPass,
}

struct Optimizer {
    strip_docstrings: bool,
    future_annotations: bool,
    python_version: PythonVersion,
    locations: RefCell<FxHashMap<TextRange, OptimizedLocation>>,
}

impl Optimizer {
    fn scope_body(&self, body: &mut Suite) {
        let only_statement = body.len() == 1;
        if self.strip_docstrings
            && let Some(statement @ Stmt::Expr(_)) = body.first_mut()
            && let Stmt::Expr(expression) = statement
            && expression.value.is_string_literal_expr()
        {
            *statement = if self.python_version == PythonVersion::Py312
                || self.python_version >= PythonVersion::Py314 && only_statement
            {
                // Python 3.12 retains removed docstrings as located NOPs.
                // Modern targets synthesize a four-column `pass` when the AST suite empties.
                if self.python_version >= PythonVersion::Py314 {
                    self.locations
                        .borrow_mut()
                        .insert(expression.range, OptimizedLocation::SyntheticPass);
                }
                Stmt::Pass(ast::StmtPass {
                    node_index: ast::AtomicNodeIndex::default(),
                    range: expression.range,
                })
            } else {
                self.locations
                    .borrow_mut()
                    .insert(expression.range, OptimizedLocation::Elided);
                elided_statement(expression.range)
            };
            if self.python_version >= PythonVersion::Py314 && !only_statement {
                body.remove(0);
            }
            if self.python_version >= PythonVersion::Py314
                && let Some(Stmt::Expr(expression)) = body.first_mut()
                && let Expr::StringLiteral(string) = expression.value.as_ref()
            {
                // The native AST optimizer removes the docstring and wraps an exposed string
                // in JoinedStr, preserving its constant admission without creating a new docstring.
                // Keep the literal tokens intact so surrogate escapes and raw prefixes survive.
                let mut parts: Vec<_> = string
                    .value
                    .iter()
                    .cloned()
                    .map(ast::FStringPart::Literal)
                    .collect();
                parts.push(ast::FStringPart::FString(ast::FString {
                    node_index: ast::AtomicNodeIndex::default(),
                    range: string.range,
                    elements: Vec::new().into(),
                    flags: ast::FStringFlags::empty(),
                }));
                *expression.value = Expr::FString(ast::ExprFString {
                    node_index: ast::AtomicNodeIndex::default(),
                    range: string.range,
                    value: ast::FStringValue::concatenated(parts),
                });
            }
        }
    }
}

fn elided_statement(range: ruff_text_size::TextRange) -> Stmt {
    // An empty declaration emits no instructions. Keeping a statement boundary avoids
    // promoting a following string to a docstring or deleting an empty finally suite.
    // Scope analysis still uses the original AST, before these synthetic declarations.
    Stmt::Global(ast::StmtGlobal {
        node_index: ast::AtomicNodeIndex::default(),
        range,
        names: Vec::new(),
    })
}

impl Transformer for Optimizer {
    fn visit_stmt(&self, statement: &mut Stmt) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_stmt(statement));
        }
        match statement {
            Stmt::Assert(assertion) => {
                self.locations
                    .borrow_mut()
                    .insert(assertion.range, OptimizedLocation::Elided);
                // Erased assertions can also contain deeply nested expressions.
                let clear = ClearExpressions {
                    shallow_drop: false,
                };
                clear.visit_expr(&mut assertion.test);
                if let Some(message) = assertion.msg.as_deref_mut() {
                    clear.visit_expr(message);
                }
                *statement = elided_statement(assertion.range);
                return;
            }
            Stmt::FunctionDef(function) => self.scope_body(&mut function.body),
            Stmt::ClassDef(class) => self.scope_body(&mut class.body),
            _ => {}
        }
        walk_stmt(self, statement);
    }

    fn visit_annotation(&self, expression: &mut Expr) {
        // Postponed annotations retain their authored spelling, including __debug__.
        if !self.future_annotations {
            self.visit_expr(expression);
        }
    }

    fn visit_expr(&self, expression: &mut Expr) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_expr(expression));
        }
        if let Expr::Name(name) = expression
            && name.ctx == ExprContext::Load
            && name.id.as_str() == "__debug__"
        {
            *expression = Expr::BooleanLiteral(ast::ExprBooleanLiteral {
                node_index: ast::AtomicNodeIndex::default(),
                range: name.range,
                value: false,
            });
        } else {
            walk_expr(self, expression);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_owned_matches_cloned(source: &str, options: &crate::CompileOptions) {
        struct ResetClone(bool);

        impl Drop for ResetClone {
            fn drop(&mut self) {
                FORCE_CLONED_AST.set(self.0);
            }
        }

        let owned = crate::compile_with_options(source, "owned_optimization.py", options);
        let cloned = {
            let _reset = ResetClone(FORCE_CLONED_AST.replace(true));
            crate::compile_with_options(source, "owned_optimization.py", options)
        };
        match (owned, cloned) {
            (Ok(owned), Ok(cloned)) => {
                assert_eq!(owned.marshal(), cloned.marshal(), "{options:?}: {source}");
            }
            (owned, cloned) => {
                assert_eq!(
                    format!("{owned:?}"),
                    format!("{cloned:?}"),
                    "{options:?}: {source}"
                );
            }
        }
    }

    #[test]
    fn owned_optimization_matches_cloned_trees_for_all_targets() {
        let sources = [
            "value = 1\n",
            "'docstring'\n'not a docstring'\nvalue = __debug__\n",
            "assert hidden\ndef f(value):\n    return value\n",
            "def f():\n    assert (value := 1)\n    return value\n",
            "def f(value):\n    assert (yield value)\n",
            "def f(value):\n    assert (yield from value)\n",
            "async def f():\n    assert await work()\n",
            "def f():\n    assert await work()\n",
            "class C:\n    'docstring'\n    __field = 1\n    def method(self):\n        assert hidden\n        return [self.__field for item in values]\n",
            "class C:\n    assert [lambda: item for item in values]\n    values = [lambda: item for item in values]\n",
            "from __future__ import annotations\nclass C:\n    assert hidden\n    field: [x for x in values]\n",
            "def outer(value):\n    assert hidden\n    class C:\n        def method(self):\n            return value, __class__\n    return C\n",
            "assert caf\u{e9}\nclass C:\n    __field = ''\n    def method(self):\n        return '\u{e9}', self.__field\n",
            "assert package.submodule\nimport package.submodule\n",
            "def f():\n    try:\n        work()\n    finally:\n        assert hidden\n",
            "value = __\u{ff44}\u{ff45}\u{ff42}\u{ff55}\u{ff47}__\n",
            "value = (__debug__, f'{(__debug__, 1)}')\n",
        ];
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for optimization_level in [
                OptimizationLevel::Zero,
                OptimizationLevel::One,
                OptimizationLevel::Two,
            ] {
                let options = crate::CompileOptions {
                    python_version,
                    optimization_level,
                    ..crate::CompileOptions::default()
                };
                for source in sources {
                    assert_owned_matches_cloned(source, &options);
                }
            }
        }
    }

    #[test]
    fn owned_optimization_drops_deep_trees_on_varied_stacks() {
        for stack_size in [128 * 1024, 2 * crate::STACK_SEGMENT_SIZE] {
            std::thread::Builder::new()
                .stack_size(stack_size)
                .spawn(|| {
                    let chain = "value + ".repeat(512);
                    let source = format!(
                        "assert {chain}value, {chain}'message'\n\
                         value = {chain}(value, 0, 'value', b'value', True, None, ...)\n\
                         value = f'{{{chain}value}}'\n"
                    );
                    for python_version in [
                        PythonVersion::Py312,
                        PythonVersion::Py313,
                        PythonVersion::Py314,
                        PythonVersion::Py315,
                    ] {
                        let options = crate::CompileOptions {
                            python_version,
                            optimization_level: OptimizationLevel::Two,
                            ..crate::CompileOptions::default()
                        };
                        assert_owned_matches_cloned(&source, &options);
                    }
                })
                .unwrap()
                .join()
                .unwrap();
        }
    }

    #[test]
    fn shallow_drop_preserves_short_optimized_expressions() {
        for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
            for source in [
                "(__debug__, f'{__debug__}')\n",
                "f'{(__debug__, 1)}'\n",
                "'docstring'\n'exposed string'\n",
            ] {
                let body = ruff_python_parser::parse_module(source)
                    .unwrap()
                    .into_suite();
                let mut optimized = prepare(body, OptimizationLevel::Two, python_version);
                let Stmt::Expr(statement) = &mut optimized.body[0] else {
                    panic!("expected an expression");
                };
                let expression = statement.value.as_mut();
                let range = expression.range();
                assert!(range.len() <= TextSize::new(256));
                ClearExpressions { shallow_drop: true }.visit_expr(expression);
                assert!(matches!(expression, Expr::Tuple(_) | Expr::FString(_)));
                assert_eq!(expression.range(), range);
                ClearExpressions {
                    shallow_drop: false,
                }
                .visit_expr(expression);
                assert!(expression.is_none_literal_expr());
            }
        }
    }

    #[test]
    fn preparation_skips_only_unchanged_trees() {
        let cases = [
            ("value = 1\n", false, false),
            ("def f(x):\n    return x\n", false, false),
            ("class C:\n    value: int = 1\n", false, false),
            ("value = 1\n'not a docstring'\n", false, false),
            ("value = '__debug__'\n", false, false),
            ("'module docstring'\nvalue = 1\n", false, true),
            ("def f():\n    'function docstring'\n", false, true),
            ("class C:\n    'class docstring'\n", false, true),
            ("assert value\n", true, true),
            ("def f():\n    assert value\n", true, true),
            ("value = __debug__\n", true, true),
            ("value = f'{__debug__}'\n", true, true),
            ("value = lambda: __debug__\n", true, true),
            ("value = [__debug__ for _ in items]\n", true, true),
            (
                "value = __\u{ff44}\u{ff45}\u{ff42}\u{ff55}\u{ff47}__\n",
                true,
                true,
            ),
        ];
        for (source, one, two) in cases {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            for (level, expected) in [
                (OptimizationLevel::Zero, false),
                (OptimizationLevel::One, one),
                (OptimizationLevel::Two, two),
            ] {
                assert_eq!(
                    requires_optimization(parsed.suite(), level),
                    expected,
                    "{source:?}, {level:?}"
                );
            }
        }
    }
}
