//! Target-specific diagnostics for recoverable parser errors.

use ruff_python_ast::token::{Tokens, parenthesized_range};
use ruff_python_ast::visitor::{
    Visitor, walk_arguments, walk_comprehension, walk_decorator, walk_elif_else_clause, walk_expr,
    walk_match_case, walk_pattern, walk_stmt,
};
use ruff_python_ast::{
    AnyNodeRef, Arguments, Comprehension, Decorator, ElifElseClause, Expr, MatchCase, Pattern, Stmt,
};
use ruff_text_size::{Ranged, TextRange};

/// Find an attribute target where CPython's grammar checks named expressions.
pub(crate) fn is_attribute_assignment_target(
    body: &[Stmt],
    tokens: &Tokens,
    range: TextRange,
) -> bool {
    struct Finder<'ast, 'tokens> {
        range: TextRange,
        tokens: &'tokens Tokens,
        parent: Option<AnyNodeRef<'ast>>,
        found: bool,
    }

    impl Finder<'_, '_> {
        fn allows_named_expression(&self, expression: &Expr) -> bool {
            let Some(parent) = self.parent else {
                return false;
            };
            if parenthesized_range(expression.into(), parent, self.tokens).is_some() {
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
                AnyNodeRef::ExprTuple(tuple) => tuple.parenthesized,
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
            let previous = self.parent.replace(statement.into());
            walk_stmt(self, statement);
            self.parent = previous;
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
                && self.allows_named_expression(expression)
            {
                self.found = true;
                return;
            }
            let previous = self.parent.replace(expression.into());
            walk_expr(self, expression);
            self.parent = previous;
        }

        fn visit_pattern(&mut self, pattern: &'ast Pattern) {
            if self.found || !pattern.range().contains_range(self.range) {
                return;
            }
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_pattern(pattern));
            }
            let previous = self.parent.replace(pattern.into());
            walk_pattern(self, pattern);
            self.parent = previous;
        }

        fn visit_arguments(&mut self, arguments: &'ast Arguments) {
            let previous = self.parent.replace(arguments.into());
            walk_arguments(self, arguments);
            self.parent = previous;
        }

        fn visit_comprehension(&mut self, comprehension: &'ast Comprehension) {
            let previous = self.parent.replace(comprehension.into());
            walk_comprehension(self, comprehension);
            self.parent = previous;
        }

        fn visit_decorator(&mut self, decorator: &'ast Decorator) {
            let previous = self.parent.replace(decorator.into());
            walk_decorator(self, decorator);
            self.parent = previous;
        }

        fn visit_match_case(&mut self, case: &'ast MatchCase) {
            let previous = self.parent.replace(case.into());
            walk_match_case(self, case);
            self.parent = previous;
        }

        fn visit_elif_else_clause(&mut self, clause: &'ast ElifElseClause) {
            let previous = self.parent.replace(clause.into());
            walk_elif_else_clause(self, clause);
            self.parent = previous;
        }
    }

    let mut finder = Finder {
        range,
        tokens,
        parent: None,
        found: false,
    };
    finder.visit_body(body);
    finder.found
}
