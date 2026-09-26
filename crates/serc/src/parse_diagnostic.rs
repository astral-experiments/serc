//! Target-specific diagnostics for recoverable parser errors.

use ruff_python_ast::token::{Tokens, parenthesized_range};
use ruff_python_ast::visitor::{
    Visitor, walk_arguments, walk_comprehension, walk_decorator, walk_elif_else_clause, walk_expr,
    walk_match_case, walk_pattern, walk_stmt,
};
use ruff_python_ast::{
    AnyNodeRef, Arguments, Comprehension, Decorator, ElifElseClause, Expr, MatchCase, Pattern, Stmt,
};
use ruff_python_parser::ParseError;
use ruff_text_size::{Ranged, TextRange};

/// Find an attribute target where CPython's grammar checks named expressions.
pub(crate) fn is_attribute_assignment_target(
    body: &[Stmt],
    tokens: &Tokens,
    range: TextRange,
    later_errors: &[ParseError],
) -> bool {
    struct Finder<'ast, 'tokens> {
        range: TextRange,
        tokens: &'tokens Tokens,
        parent: Option<AnyNodeRef<'ast>>,
        grandparent: Option<AnyNodeRef<'ast>>,
        later_errors: &'tokens [ParseError],
        found: bool,
    }

    impl<'ast> Finder<'ast, '_> {
        fn with_parent(&mut self, parent: AnyNodeRef<'ast>, visit: impl FnOnce(&mut Self)) {
            let previous = self.parent.replace(parent);
            let previous_grandparent = std::mem::replace(&mut self.grandparent, previous);
            visit(self);
            self.parent = previous;
            self.grandparent = previous_grandparent;
        }

        fn allows_named_expression(&self, expression: &Expr) -> bool {
            let Some(parent) = self.parent else {
                return false;
            };
            let parentheses = parenthesized_range(expression.into(), parent, self.tokens);
            if parentheses.is_some() {
                return true;
            }
            match parent {
                AnyNodeRef::StmtExpr(statement) => {
                    std::ptr::eq(expression, statement.value.as_ref())
                }
                AnyNodeRef::StmtIf(statement) => std::ptr::eq(expression, statement.test.as_ref()),
                AnyNodeRef::StmtWhile(statement) => {
                    std::ptr::eq(expression, statement.test.as_ref())
                }
                AnyNodeRef::StmtMatch(statement) => {
                    std::ptr::eq(expression, statement.subject.as_ref())
                }
                AnyNodeRef::ElifElseClause(clause) => clause
                    .test
                    .as_ref()
                    .is_some_and(|test| std::ptr::eq(expression, test)),
                AnyNodeRef::MatchCase(case) => case
                    .guard
                    .as_deref()
                    .is_some_and(|guard| std::ptr::eq(expression, guard)),
                AnyNodeRef::Decorator(decorator) => std::ptr::eq(expression, &decorator.expression),
                AnyNodeRef::ExprList(_) | AnyNodeRef::ExprSet(_) => true,
                AnyNodeRef::ExprTuple(tuple) => {
                    tuple.parenthesized
                        || match self.grandparent {
                            Some(AnyNodeRef::StmtMatch(statement)) => {
                                matches!(statement.subject.as_ref(), Expr::Tuple(subject) if std::ptr::eq(subject, tuple))
                            }
                            Some(AnyNodeRef::ExprSubscript(subscript)) => {
                                matches!(subscript.slice.as_ref(), Expr::Tuple(slice) if std::ptr::eq(slice, tuple))
                            }
                            Some(AnyNodeRef::StmtExpr(statement)) => {
                                matches!(statement.value.as_ref(), Expr::Tuple(value) if std::ptr::eq(value, tuple))
                            }
                            _ => false,
                        }
                }
                AnyNodeRef::ExprListComp(comprehension) => {
                    std::ptr::eq(expression, comprehension.elt.as_ref())
                }
                AnyNodeRef::ExprSetComp(comprehension) => {
                    std::ptr::eq(expression, comprehension.elt.as_ref())
                }
                AnyNodeRef::ExprSubscript(subscript) => {
                    std::ptr::eq(expression, subscript.slice.as_ref())
                }
                _ => false,
            }
        }
    }

    impl<'ast> Visitor<'ast> for Finder<'ast, '_> {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if self.found || !statement.range().contains_range(self.range) {
                return;
            }
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            self.with_parent(statement.into(), |finder| walk_stmt(finder, statement));
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.found || !expression.range().contains_range(self.range) {
                return;
            }
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            if let Expr::Named(named) = expression
                && named.target.range() == self.range
                && named.target.is_attribute_expr()
                && !self
                    .later_errors
                    .iter()
                    .any(|error| named.value.range().contains_range(error.location))
                && self.allows_named_expression(expression)
            {
                self.found = true;
                return;
            }
            self.with_parent(expression.into(), |finder| walk_expr(finder, expression));
        }

        fn visit_pattern(&mut self, pattern: &'ast Pattern) {
            if self.found || !pattern.range().contains_range(self.range) {
                return;
            }
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_pattern(pattern));
            }
            self.with_parent(pattern.into(), |finder| walk_pattern(finder, pattern));
        }

        fn visit_arguments(&mut self, arguments: &'ast Arguments) {
            self.with_parent(arguments.into(), |finder| walk_arguments(finder, arguments));
        }

        fn visit_comprehension(&mut self, comprehension: &'ast Comprehension) {
            self.with_parent(comprehension.into(), |finder| {
                walk_comprehension(finder, comprehension);
            });
        }

        fn visit_decorator(&mut self, decorator: &'ast Decorator) {
            self.with_parent(decorator.into(), |finder| walk_decorator(finder, decorator));
        }

        fn visit_match_case(&mut self, case: &'ast MatchCase) {
            self.with_parent(case.into(), |finder| walk_match_case(finder, case));
        }

        fn visit_elif_else_clause(&mut self, clause: &'ast ElifElseClause) {
            self.with_parent(clause.into(), |finder| {
                walk_elif_else_clause(finder, clause)
            });
        }
    }

    let mut finder = Finder {
        range,
        tokens,
        parent: None,
        grandparent: None,
        later_errors,
        found: false,
    };
    finder.visit_body(body);
    finder.found
}
