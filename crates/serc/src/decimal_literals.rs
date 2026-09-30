use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{Expr, Stmt};
use ruff_python_parser::{UnsupportedSyntaxError, UnsupportedSyntaxErrorKind};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::CompileError;

/// Apply the selected decimal conversion limit of the pinned CPython interpreters.
/// Validate tokens before lowering, including literals in unreachable statements.
pub(crate) fn validate(
    source: &str,
    tokens: &Tokens,
    suite: &[Stmt],
    unsupported: &[UnsupportedSyntaxError],
    max_digits: u32,
) -> Result<(), CompileError> {
    if max_digits == 0 || source.len() <= max_digits as usize {
        return Ok(());
    }
    let max_digits = max_digits as usize;
    for token in tokens {
        if token.kind() != TokenKind::Int {
            continue;
        }
        let literal = &source[token.range()];
        // A valid integer starting with zero is either a power-of-two-base literal,
        // which is exempt, or all zeros, which CPython converts via its C-integer
        // fast path without invoking the limited decimal conversion.
        if literal.len() <= max_digits || literal.starts_with('0') {
            continue;
        }
        let digits = literal.bytes().filter(u8::is_ascii_digit).count();
        if digits > max_digits {
            if let Some(error) = unsupported.first() {
                // These errors are diagnosed after parsing their whole expression,
                // so a decimal literal inside it takes precedence.
                let position = match error.kind {
                    UnsupportedSyntaxErrorKind::UnpackingInComprehension(_) => {
                        comprehension_end(suite, error.range).unwrap_or(error.range.start())
                    }
                    UnsupportedSyntaxErrorKind::UnparenthesizedExceptionTypes => error.range.end(),
                    _ => error.range.start(),
                };
                if position <= token.start() {
                    return Err(CompileError::invalid_syntax(error.to_string(), error.range));
                }
            }
            return Err(CompileError::invalid_syntax(
                format!(
                    "Exceeds the limit ({max_digits} digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit - Consider hexadecimal for huge integer literals to avoid decimal conversion limits."
                ),
                token.range(),
            ));
        }
    }
    Ok(())
}

/// Find the comprehension whose element caused an unsupported unpacking error.
/// CPython parses the entire comprehension before rejecting its unpacking.
fn comprehension_end(suite: &[Stmt], range: TextRange) -> Option<TextSize> {
    struct Finder {
        range: TextRange,
        end: Option<TextSize>,
    }
    impl<'ast> Visitor<'ast> for Finder {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            if statement.range().contains_range(self.range) {
                walk_stmt(self, statement);
            }
        }
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            if !expression.range().contains_range(self.range) {
                return;
            }
            if matches!(
                expression,
                Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_)
            ) {
                self.end = Some(expression.end());
            }
            // A nested matching comprehension is closer to the offending element.
            walk_expr(self, expression);
        }
    }
    let mut finder = Finder { range, end: None };
    finder.visit_body(suite);
    finder.end
}
