//! Prepare the code-generation AST for Python's explicit optimization levels.

use std::cell::RefCell;

use ruff_python_ast::visitor::Visitor;
use ruff_python_ast::visitor::transformer::{Transformer, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, Expr, ExprContext, Stmt, Suite};
use ruff_text_size::TextRange;
use rustc_hash::FxHashMap;

use crate::{OptimizationLevel, PythonVersion};

use super::scope::future_feature_flags;
use crate::target::code_flags::CO_FUTURE_ANNOTATIONS;

/// Keep source ranges while applying the transformations that precede code generation.
pub(crate) fn prepare(
    body: &Suite,
    level: OptimizationLevel,
    python_version: PythonVersion,
) -> OptimizedSuite {
    let optimizer = Optimizer {
        strip_docstrings: level == OptimizationLevel::Two,
        future_annotations: future_feature_flags(body) & CO_FUTURE_ANNOTATIONS != 0,
        python_version,
        locations: RefCell::default(),
    };
    let mut depth = crate::stack::ExpressionDepth::default();
    depth.visit_body(body);
    crate::stack::with_depth(depth.maximum(), || {
        let mut body = body.clone();
        optimizer.scope_body(&mut body);
        optimizer.visit_body(&mut body);
        OptimizedSuite {
            body,
            locations: optimizer.locations.into_inner(),
        }
    })
}

pub(crate) struct OptimizedSuite {
    pub(crate) body: Suite,
    pub(super) locations: FxHashMap<TextRange, OptimizedLocation>,
}

impl Drop for OptimizedSuite {
    fn drop(&mut self) {
        struct ClearExpressions;

        impl Transformer for ClearExpressions {
            fn visit_expr(&self, expression: &mut Expr) {
                if crate::stack_growth_required() {
                    return crate::grow_stack(|| self.visit_expr(expression));
                }
                walk_expr(self, expression);
                // Clear children before replacing their parent so destruction cannot recurse
                // through a deeply nested expression after the compilation stack has unwound.
                *expression = Expr::NoneLiteral(ast::ExprNoneLiteral::default());
            }
        }

        ClearExpressions.visit_body(&mut self.body);
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
        match statement {
            Stmt::Assert(assertion) => {
                self.locations
                    .borrow_mut()
                    .insert(assertion.range, OptimizedLocation::Elided);
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
