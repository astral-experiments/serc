use ruff_python_ast::token::{TokenFlags, TokenKind};
use ruff_python_parser::{Mode, ParseErrorType, ParseOptions, lexer};
use ruff_text_size::TextRange;

use crate::{CompileError, PythonVersion, indentation, yield_syntax};

const MAX_NESTING: usize = 200;

/// CPython's tokenizer rejects the 201st simultaneously open delimiter. Replacement
/// fields in interpolated strings share the same nesting limit as ordinary expressions.
pub(crate) fn validate(
    source: &str,
    options: &ParseOptions,
    version: PythonVersion,
) -> Result<(), CompileError> {
    validate_with_fast_check::<true>(source, options, version)
}

/// Validates nesting with an optional byte-scan proof; tests disable it to check lexer parity.
fn validate_with_fast_check<const FAST: bool>(
    source: &str,
    options: &ParseOptions,
    version: PythonVersion,
) -> Result<(), CompileError> {
    // Most files cannot exceed the limit even if every opening character were a
    // delimiter. Avoid lexing them twice, especially during batch compilation.
    if opening_count_is_bounded(source.as_bytes(), MAX_NESTING) {
        return Ok(());
    }

    if FAST && nesting_is_bounded(source) {
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
                        match earlier_indentation_error(source, options, range, version) {
                            EarlierIndentation::Error(error) => return Err(error),
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

#[inline]
fn opening_count_is_bounded(bytes: &[u8], maximum: usize) -> bool {
    bytes
        .iter()
        .filter(|byte| matches!(byte, b'(' | b'[' | b'{'))
        .take(maximum + 1)
        .count()
        <= maximum
}

/// Certifies the nesting limit for ordinary strings and comments without lexing identifiers.
/// Unrecognized suffixes use the full lexer unless even their raw openings fit the limit.
fn nesting_is_bounded(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut closing = [0_u8; MAX_NESTING];
    let mut depth = 0;
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'(' | b'[' | b'{' => {
                if depth == MAX_NESTING {
                    return false;
                }
                closing[depth] = match byte {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                depth += 1;
            }
            b')' | b']' | b'}' => {
                if depth == 0 || closing[depth - 1] != byte {
                    // The full lexer stops at the first unmatched delimiter too.
                    return true;
                }
                depth -= 1;
            }
            b'#' => {
                while bytes
                    .get(index)
                    .is_some_and(|byte| !matches!(byte, b'\n' | b'\r'))
                {
                    index += 1;
                }
                continue;
            }
            b'\'' | b'"' => {
                if index > 0
                    && (bytes[index - 1].is_ascii_alphanumeric()
                        || bytes[index - 1] == b'_'
                        || !bytes[index - 1].is_ascii())
                {
                    // Every remaining lexer opening needs a source opening byte,
                    // just as in the whole-file guard. This also bounds interpolation.
                    return opening_count_is_bounded(&bytes[index..], MAX_NESTING - depth);
                }
                let triple =
                    bytes.get(index + 1) == Some(&byte) && bytes.get(index + 2) == Some(&byte);
                let width = if triple { 3 } else { 1 };
                index += width;
                while let Some(&character) = bytes.get(index) {
                    if character == b'\\' {
                        // A CRLF escape is one continued line, like a lone CR or LF.
                        index += if bytes.get(index + 1) == Some(&b'\r')
                            && bytes.get(index + 2) == Some(&b'\n')
                        {
                            3
                        } else {
                            2
                        };
                    } else if character == byte
                        && (!triple
                            || (bytes.get(index + 1) == Some(&byte)
                                && bytes.get(index + 2) == Some(&byte)))
                    {
                        index += width;
                        break;
                    } else if !triple && matches!(character, b'\n' | b'\r') {
                        // An unclosed ordinary string stops the nesting validator.
                        return true;
                    } else {
                        index += 1;
                    }
                }
                continue;
            }
            b'\\' => {
                return opening_count_is_bounded(&bytes[index..], MAX_NESTING - depth);
            }
            _ => {}
        }
        index += 1;
    }
    true
}

enum EarlierIndentation {
    Error(CompileError),
    Defer,
    None,
}

fn earlier_indentation_error(
    source: &str,
    options: &ParseOptions,
    range: TextRange,
    version: PythonVersion,
) -> EarlierIndentation {
    let prefix = &source[..range.start().to_usize()];
    let parsed = ruff_python_parser::parse_unchecked(prefix, options.clone());
    let parse_error = parsed.errors().first();
    if let Some(indent) = indentation::first_excess(parsed.tokens()) {
        return if parse_error.is_none_or(|error| indent.start() <= error.location.start()) {
            EarlierIndentation::Error(
                yield_syntax::before_indentation_error(source, options, indent, version)
                    .unwrap_or_else(|| indentation::excess_error(indent)),
            )
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

pub(crate) fn invalid_number_suffix(source: &str, range: TextRange) -> bool {
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

#[cfg(test)]
mod tests {
    use super::{nesting_is_bounded, validate_with_fast_check};
    use crate::PythonVersion;
    use ruff_python_parser::{Mode, ParseOptions};

    #[test]
    fn bounded_scan_preserves_lexer_diagnostics() {
        let options = ParseOptions::from(Mode::Module);
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for depth in [0, 1, 199, 200, 201, 202] {
                let expression = format!("{}0{}", "(".repeat(depth), ")".repeat(depth));
                for prefix in [
                    "",
                    "# ([(\n",
                    "value = ",
                    "if True:\n    ",
                    "0xg\n",
                    "yield, 0\n",
                    "value = 'unterminated\n",
                    "value = '''unterminated\n",
                    "value = \"escaped\\\"([({})])\"\n",
                    "value = '\\\n'\n",
                    "value = '\\\r\ncontinued'\n",
                    "value = '''([{\\\"\n}])'''\n",
                    "value = r'([{}])'\n",
                    "value = f'{(0)}'\n",
                    "value = t'{(0)}'\n",
                    "value = rf'{(0)}'\n",
                    "value = \"\"\"([{\n}])\"\"\"\n",
                    ")\n",
                    "[)\n",
                    "value = 1 \\\n  + 2\n",
                ] {
                    for source in [
                        format!("{prefix}{expression}\n{}\n", "# ()".repeat(201)),
                        format!("{}\n{prefix}{expression}\n", "# ()".repeat(201)),
                    ] {
                        assert_eq!(
                            validate_with_fast_check::<true>(&source, &options, version),
                            validate_with_fast_check::<false>(&source, &options, version),
                            "{version:?}: {source:?}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn bounded_scan_requires_fallback_for_unbounded_suffixes_and_excess_nesting() {
        for source in [
            "f'{value}'",
            "t'{value}'",
            "r'value'",
            "rf'{value}'",
            "x \\\n + y",
        ] {
            assert!(nesting_is_bounded(source), "{source:?}");
            let source = format!("{source}{}", "(".repeat(201));
            assert!(!nesting_is_bounded(&source), "{source:?}");
        }
        assert!(nesting_is_bounded(&format!(
            "{}0{}",
            "(".repeat(200),
            ")".repeat(200)
        )));
        assert!(!nesting_is_bounded(&format!(
            "{}0{}",
            "(".repeat(201),
            ")".repeat(201)
        )));
        assert!(nesting_is_bounded(&format!("{}\n0", "# (".repeat(300))));
        assert!(nesting_is_bounded(&format!("'{}'", "([{)]}".repeat(300))));
    }
}
