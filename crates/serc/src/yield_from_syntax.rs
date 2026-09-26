use ruff_python_ast::token::TokenKind;
use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{Expr, ModModule, Stmt};
use ruff_python_parser::{ParseError, ParseErrorType, Parsed};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::{grow_stack, stack_growth_required};

/// Find the diagnostic location when Ruff rejects a tuple as a `yield from` operand.
pub(crate) fn invalid_tuple_diagnostic(
    parsed: &Parsed<ModModule>,
) -> Option<(&'static str, TextRange)> {
    struct Finder<'a> {
        errors: &'a [ParseError],
        tokens: &'a [ruff_python_ast::token::Token],
        found: Option<(&'static str, TextRange)>,
    }

    impl<'a> Visitor<'a> for Finder<'_> {
        fn visit_stmt(&mut self, statement: &'a Stmt) {
            if self.found.is_none() {
                if stack_growth_required() {
                    return grow_stack(|| self.visit_stmt(statement));
                }
                walk_stmt(self, statement);
            }
        }

        fn visit_expr(&mut self, expression: &'a Expr) {
            if self.found.is_some() {
                return;
            }
            if stack_growth_required() {
                return grow_stack(|| self.visit_expr(expression));
            }
            if let Expr::YieldFrom(yield_from) = expression {
                if let Expr::Tuple(tuple) = yield_from.value.as_ref() {
                    if !tuple.parenthesized
                        && self.errors.iter().any(|error| match error.error {
                            ParseErrorType::UnparenthesizedTupleExpression => {
                                error.location == tuple.range
                            }
                            ParseErrorType::InvalidStarredExpressionUsage => {
                                tuple.elts.first().is_some_and(|first| {
                                    matches!(first, Expr::Starred(_))
                                        && first.range() == error.location
                                })
                            }
                            _ => false,
                        })
                        && self.errors.iter().all(|error| match error.error {
                            ParseErrorType::UnparenthesizedTupleExpression => {
                                error.location == tuple.range
                            }
                            ParseErrorType::InvalidStarredExpressionUsage => tuple
                                .elts
                                .binary_search_by_key(&error.location.start(), |element| {
                                    element.start()
                                })
                                .is_ok_and(|index| {
                                    let element = &tuple.elts[index];
                                    matches!(element, Expr::Starred(_))
                                        && element.range() == error.location
                                }),
                            _ => false,
                        })
                    {
                        if let Some(first) = tuple.elts.first() {
                            self.found = if matches!(first, Expr::Starred(_)) {
                                if tuple.start() < first.start() {
                                    Some(("cannot use starred expression here", first.range()))
                                } else {
                                    Some((
                                        "invalid syntax",
                                        TextRange::at(first.start(), TextSize::from(1)),
                                    ))
                                }
                            } else {
                                self.tokens
                                    .iter()
                                    .find(|token| {
                                        token.kind() == TokenKind::Comma
                                            && token.start() >= first.end()
                                            && token.end() <= tuple.end()
                                    })
                                    .map(|token| ("invalid syntax", token.range()))
                            };
                        }
                    }
                }
            }
            if self.found.is_none() {
                walk_expr(self, expression);
            }
        }
    }

    if !parsed.unsupported_syntax_errors().is_empty()
        || !parsed.errors().iter().any(|error| {
            matches!(
                error.error,
                ParseErrorType::UnparenthesizedTupleExpression
                    | ParseErrorType::InvalidStarredExpressionUsage
            )
        })
    {
        return None;
    }

    let mut finder = Finder {
        errors: parsed.errors(),
        tokens: parsed.tokens(),
        found: None,
    };
    finder.visit_body(parsed.suite());
    finder.found
}
