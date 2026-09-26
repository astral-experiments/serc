use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_text_size::Ranged;

use crate::CompileError;

/// Ruff can parse `yield, value` as a tuple containing a valueless yield.
/// CPython requires the yield to be parenthesized before that tuple comma.
pub(crate) fn validate(source: &str, tokens: &Tokens) -> Result<(), CompileError> {
    if !source.contains("yield") {
        return Ok(());
    }
    let mut bare_yield = false;
    for token in tokens {
        match token.kind() {
            TokenKind::Comment | TokenKind::NonLogicalNewline => {}
            TokenKind::Comma if bare_yield => {
                return Err(CompileError::invalid_syntax(
                    "invalid syntax",
                    token.range(),
                ));
            }
            kind => bare_yield = kind == TokenKind::Yield,
        }
    }
    Ok(())
}
