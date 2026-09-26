use ruff_python_ast::token::{TokenFlags, TokenKind};
use ruff_python_parser::{Mode, ParseErrorType, ParseOptions, lexer};
use ruff_text_size::TextRange;

use crate::{CompileError, PythonVersion, indentation};

/// CPython's tokenizer rejects the 201st simultaneously open delimiter. Replacement
/// fields in interpolated strings share the same nesting limit as ordinary expressions.
pub(crate) fn validate(
    source: &str,
    options: &ParseOptions,
    version: PythonVersion,
) -> Result<(), CompileError> {
    const MAX_NESTING: usize = 200;

    // Most files cannot exceed the limit even if every opening character were a
    // delimiter. Avoid lexing them twice, especially during batch compilation.
    if source
        .bytes()
        .filter(|byte| matches!(byte, b'(' | b'[' | b'{'))
        .take(MAX_NESTING + 1)
        .count()
        <= MAX_NESTING
    {
        return Ok(());
    }

    // Ruff recognizes template-string prefixes in every target version. CPython
    // 3.12 and 3.13 instead tokenize them as an identifier followed by an opaque
    // ordinary string. Replace only the prefix character, keeping every byte
    // offset and all delimiter, quote, and escape characters unchanged.
    let adjusted;
    let lexer_source = if matches!(version, PythonVersion::Py312 | PythonVersion::Py313) {
        let mut bytes = source.as_bytes().to_vec();
        for index in 0..bytes.len() {
            if matches!(bytes[index], b't' | b'T')
                && (matches!(bytes.get(index + 1), Some(b'\'' | b'"'))
                    || (matches!(bytes.get(index + 1), Some(b'r' | b'R'))
                        && matches!(bytes.get(index + 2), Some(b'\'' | b'"'))))
            {
                bytes[index] = b'z';
            }
        }
        adjusted = String::from_utf8(bytes).expect("replacing ASCII preserves UTF-8");
        adjusted.as_str()
    } else {
        source
    };
    let mut lexer = lexer::lex(lexer_source, Mode::Module);
    let mut stack = [TokenKind::EndOfFile; MAX_NESTING];
    let mut depth = 0;
    let mut saw_indent = false;
    loop {
        let token = lexer.next_token();
        match token {
            TokenKind::EndOfFile => return Ok(()),
            TokenKind::Indent => saw_indent = true,
            TokenKind::Int | TokenKind::Float | TokenKind::Complex
                if invalid_number_suffix(source, lexer.current_range()) =>
            {
                // Ruff may split an invalid number into multiple tokens, while
                // CPython reports the malformed literal during tokenization.
                return Ok(());
            }
            TokenKind::String if lexer.current_flags().contains(TokenFlags::UNCLOSED_STRING) => {
                return Ok(());
            }
            TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => {
                if depth == MAX_NESTING {
                    let range = lexer.current_range();
                    if saw_indent {
                        match earlier_indentation_error(source, options, range) {
                            EarlierIndentation::Excess(range) => {
                                return Err(indentation::excess_error(range));
                            }
                            EarlierIndentation::Defer => return Ok(()),
                            EarlierIndentation::None => {}
                        }
                    }
                    return Err(CompileError::invalid_syntax(
                        "too many nested parentheses",
                        range,
                    ));
                }
                stack[depth] = token;
                depth += 1;
            }
            TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => {
                let matches = depth > 0
                    && matches!(
                        (stack[depth - 1], token),
                        (TokenKind::Lpar, TokenKind::Rpar)
                            | (TokenKind::Lsqb, TokenKind::Rsqb)
                            | (TokenKind::Lbrace, TokenKind::Rbrace)
                    );
                if !matches {
                    // CPython reports an unmatched delimiter as soon as it is lexed.
                    return Ok(());
                }
                depth -= 1;
            }
            // These printable ASCII characters become invalid grammar tokens in
            // CPython, not tokenizer failures; a later nesting error takes priority.
            // Other lexer errors (including bad strings and indentation) precede it.
            TokenKind::Unknown
                if !matches!(
                    &source[lexer.current_range().start().to_usize()
                        ..lexer.current_range().end().to_usize()],
                    "`" | "?" | "$"
                ) =>
            {
                return Ok(());
            }
            _ => {}
        }
    }
}

enum EarlierIndentation {
    Excess(TextRange),
    Defer,
    None,
}

fn earlier_indentation_error(
    source: &str,
    options: &ParseOptions,
    range: TextRange,
) -> EarlierIndentation {
    let prefix = &source[..range.start().to_usize()];
    let parsed = ruff_python_parser::parse_unchecked(prefix, options.clone());
    let parse_error = parsed.errors().first();
    if let Some(indent) = indentation::first_excess(parsed.tokens()) {
        return if parse_error.is_none_or(|error| indent.start() <= error.location.start()) {
            EarlierIndentation::Excess(indent)
        } else {
            EarlierIndentation::Defer
        };
    }
    if parse_error.is_some_and(|error| {
        matches!(error.error, ParseErrorType::UnexpectedIndentation)
            && error.location.start() < range.start()
    }) {
        EarlierIndentation::Defer
    } else {
        EarlierIndentation::None
    }
}

fn invalid_number_suffix(source: &str, range: TextRange) -> bool {
    let literal = &source[range.start().to_usize()..range.end().to_usize()];
    let suffix = &source[range.end().to_usize()..];
    let Some(first) = suffix.bytes().next() else {
        return false;
    };
    if first.is_ascii_digit()
        && (literal.starts_with("0b")
            || literal.starts_with("0B")
            || literal.starts_with("0o")
            || literal.starts_with("0O"))
    {
        return true;
    }
    if !first.is_ascii_alphabetic() && first != b'_' {
        return false;
    }
    // CPython permits these adjacent keywords with a SyntaxWarning. Its
    // tokenizer handles `if`, `in`, and `is` with two-character lookahead;
    // the other keywords must end before the next identifier character.
    if ["if", "in", "is"]
        .iter()
        .any(|word| suffix.starts_with(word))
    {
        return false;
    }
    !["and", "else", "for", "not", "or"].iter().any(|word| {
        suffix.strip_prefix(word).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|c| c.is_ascii() && !c.is_ascii_alphanumeric() && c != '_')
        })
    })
}
