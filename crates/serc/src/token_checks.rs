use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_text_size::{Ranged, TextRange};

/// Facts needed for CPython-specific diagnostics after Ruff has parsed a module.
#[derive(Default)]
pub(crate) struct ParsedTokenChecks {
    pub(crate) deep_delimiters: bool,
    pub(crate) has_template: bool,
    pub(crate) excess_indent: Option<TextRange>,
    pub(crate) has_yield: bool,
    pub(crate) longest_integer: usize,
}

impl ParsedTokenChecks {
    /// Find candidates in one pass. The diagnostic routines still choose the first error.
    pub(crate) fn collect(tokens: &Tokens) -> Self {
        let mut checks = Self::default();
        let mut delimiters = 0_usize;
        let mut indent = 0_i32;
        for token in tokens {
            match token.kind() {
                TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => {
                    delimiters += 1;
                    checks.deep_delimiters |= delimiters > crate::delimiter_nesting::MAX_NESTING;
                }
                TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => {
                    delimiters = delimiters.saturating_sub(1);
                }
                TokenKind::Indent => {
                    indent += 1;
                    if indent == crate::indentation::MAX_DEPTH && checks.excess_indent.is_none() {
                        checks.excess_indent = Some(token.range());
                    }
                }
                TokenKind::Dedent => indent -= 1,
                TokenKind::TStringStart => checks.has_template = true,
                TokenKind::Yield => checks.has_yield = true,
                TokenKind::Int => {
                    checks.longest_integer =
                        checks.longest_integer.max(usize::from(token.range().len()))
                }
                _ => {}
            }
        }
        checks
    }
}
