use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_text_size::Ranged;

use crate::CompileError;

/// Match the default decimal conversion limit of the pinned CPython interpreters.
/// Validate tokens before lowering, including literals in unreachable statements.
pub(crate) fn validate(source: &str, tokens: &Tokens) -> Result<(), CompileError> {
    const MAX_DIGITS: usize = 4300;
    for token in tokens {
        if token.kind() != TokenKind::Int {
            continue;
        }
        let literal = &source[token.range()];
        // A valid integer starting with zero is either a power-of-two-base literal,
        // which is exempt, or all zeros, which CPython converts via its C-integer
        // fast path without invoking the limited decimal conversion.
        if literal.len() <= MAX_DIGITS || literal.starts_with('0') {
            continue;
        }
        let digits = literal.bytes().filter(u8::is_ascii_digit).count();
        if digits > MAX_DIGITS {
            return Err(CompileError::invalid_syntax(
                format!(
                    "Exceeds the limit ({MAX_DIGITS} digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit - Consider hexadecimal for huge integer literals to avoid decimal conversion limits."
                ),
                token.range(),
            ));
        }
    }
    Ok(())
}
