use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_text_size::{Ranged, TextRange};

use crate::CompileError;

/// CPython's tokenizer has 100 indentation slots, including the module-level slot.
/// Count logical INDENT/DEDENT tokens so strings, comments, continuation lines and
/// indentation width do not consume extra slots. Run before semantic validation or
/// lowering can discard unreachable suites.
pub(crate) fn first_excess(tokens: &Tokens) -> Option<TextRange> {
    let mut depth = 0;
    for token in tokens {
        match token.kind() {
            TokenKind::Indent => {
                depth += 1;
                if depth == 100 {
                    return Some(token.range());
                }
            }
            TokenKind::Dedent => depth -= 1,
            _ => {}
        }
    }
    None
}

pub(crate) fn excess_error(range: TextRange) -> CompileError {
    CompileError::invalid_syntax("too many levels of indentation", range)
}
