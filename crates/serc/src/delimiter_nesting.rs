use ruff_python_ast::token::{TokenFlags, TokenKind};
use ruff_python_parser::{Mode, ParseErrorType, ParseOptions, lexer};
use ruff_text_size::TextRange;

use crate::{CompileError, PythonVersion, indentation, yield_syntax};

const MAX_NESTING: usize = 200;

/// Reuse a successfully parsed token stream to prove the nesting limit was not exceeded.
/// Malformed input and legacy template strings retain the independent tokenizer's
/// diagnostic precedence.
pub(crate) fn validate_parsed(
    source: &str,
    options: &ParseOptions,
    version: PythonVersion,
    parsed: &ruff_python_parser::Parsed<ruff_python_ast::ModModule>,
) -> Result<(), CompileError> {
    if parsed.errors().is_empty()
        && (version >= PythonVersion::Py314
            || !parsed
                .tokens()
                .iter()
                .any(|token| token.kind() == TokenKind::TStringStart))
    {
        if parsed.tokens().len() <= MAX_NESTING {
            return Ok(());
        }
        let mut depth = 0_usize;
        let exceeds_limit = parsed.tokens().iter().any(|token| {
            match token.kind() {
                TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => depth += 1,
                TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            depth > MAX_NESTING
        });
        if !exceeds_limit {
            return Ok(());
        }
    }
    validate(source, options, version)
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

/// Uses Ruff only for one prefixed string, preserving the surrounding delimiter budget.
fn shallow_prefixed_string_end(source: &str, quote: usize, outer_depth: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut start = quote;
    while start > 0 && bytes[start - 1].is_ascii_alphabetic() {
        start -= 1;
        if quote - start > 2 {
            return None;
        }
    }
    if start == quote
        || (start > 0
            && (bytes[start - 1].is_ascii_alphanumeric()
                || bytes[start - 1] == b'_'
                || !bytes[start - 1].is_ascii()))
    {
        return None;
    }

    let mut tokens = lexer::lex(&source[start..], Mode::Module);
    match tokens.next_token() {
        TokenKind::String if !tokens.current_flags().contains(TokenFlags::UNCLOSED_STRING) => {
            return Some(start + tokens.current_range().end().to_usize());
        }
        TokenKind::FStringStart => {}
        _ => return None,
    }

    let mut stack = [TokenKind::EndOfFile; MAX_NESTING];
    let mut depth = 0;
    let mut strings = 1;
    loop {
        let token = tokens.next_token();
        match token {
            TokenKind::FStringStart => strings += 1,
            TokenKind::FStringEnd => {
                strings -= 1;
                if strings == 0 {
                    return (depth == 0).then(|| start + tokens.current_range().end().to_usize());
                }
            }
            TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => {
                if outer_depth + depth == MAX_NESTING {
                    return None;
                }
                stack[depth] = token;
                depth += 1;
            }
            TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => {
                if depth == 0
                    || !matches!(
                        (stack[depth - 1], token),
                        (TokenKind::Lpar, TokenKind::Rpar)
                            | (TokenKind::Lsqb, TokenKind::Rsqb)
                            | (TokenKind::Lbrace, TokenKind::Rbrace)
                    )
                {
                    return None;
                }
                depth -= 1;
            }
            TokenKind::String if tokens.current_flags().contains(TokenFlags::UNCLOSED_STRING) => {
                return None;
            }
            TokenKind::TStringStart | TokenKind::Unknown | TokenKind::EndOfFile => return None,
            _ => {}
        }
    }
}

/// Certifies the nesting limit for ordinary strings and comments without lexing identifiers.
/// Unrecognized suffixes use the full lexer unless even their raw openings fit the limit.
fn proves_shallow_nesting(source: &str) -> bool {
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
                    if let Some(end) = shallow_prefixed_string_end(source, index, depth) {
                        index = end;
                        continue;
                    }
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

/// CPython's tokenizer rejects the 201st simultaneously open delimiter. Replacement
/// fields in interpolated strings share the same nesting limit as ordinary expressions.
pub(crate) fn validate(
    source: &str,
    options: &ParseOptions,
    version: PythonVersion,
) -> Result<(), CompileError> {
    validate_with_fast_scan::<true>(source, options, version)
}

/// Validates nesting with optional fast paths; tests disable them to check lexer parity.
fn validate_with_fast_scan<const FAST_SCAN: bool>(
    source: &str,
    options: &ParseOptions,
    version: PythonVersion,
) -> Result<(), CompileError> {
    // Most files cannot exceed the limit even if every opening character were a
    // delimiter. Avoid lexing them twice, especially during batch compilation.
    if opening_count_is_bounded(source.as_bytes(), MAX_NESTING) {
        return Ok(());
    }

    if FAST_SCAN && proves_shallow_nesting(source) {
        return Ok(());
    }

    validate_tokens(source, options, version, !FAST_SCAN)
}

fn validate_tokens(
    source: &str,
    options: &ParseOptions,
    version: PythonVersion,
    mask_legacy_templates: bool,
) -> Result<(), CompileError> {
    let legacy = matches!(version, PythonVersion::Py312 | PythonVersion::Py313);
    // Ruff recognizes template-string prefixes in every target version. CPython
    // 3.12 and 3.13 instead tokenize them as an identifier followed by an opaque
    // ordinary string. Replace only the prefix character, keeping every byte
    // offset and all delimiter, quote, and escape characters unchanged.
    let adjusted;
    let lexer_source = if legacy && mask_legacy_templates {
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
            TokenKind::TStringStart if legacy && !mask_legacy_templates => {
                // Legacy quotes can expose delimiters hidden inside a template string.
                return validate_tokens(source, options, version, true);
            }
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
    use super::{MAX_NESTING, proves_shallow_nesting, validate_parsed, validate_with_fast_scan};
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
                            validate_with_fast_scan::<true>(&source, &options, version),
                            validate_with_fast_scan::<false>(&source, &options, version),
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
            assert!(proves_shallow_nesting(source), "{source:?}");
            let source = format!("{source}{}", "(".repeat(201));
            assert!(!proves_shallow_nesting(&source), "{source:?}");
        }
        assert!(proves_shallow_nesting(&format!(
            "{}0{}",
            "(".repeat(200),
            ")".repeat(200)
        )));
        assert!(!proves_shallow_nesting(&format!(
            "{}0{}",
            "(".repeat(201),
            ")".repeat(201)
        )));
        assert!(proves_shallow_nesting(&format!("{}\n0", "# (".repeat(300))));
        assert!(proves_shallow_nesting(&format!(
            "'{}'",
            "([{)]}".repeat(300)
        )));
    }

    #[test]
    fn parsed_nesting_proof_preserves_tokenizer_diagnostics() {
        let deep = format!("{}0{}", "(".repeat(201), ")".repeat(201));
        for source in [
            "x = f'{(value)}'".to_owned(),
            "x = t'{(value)}'".to_owned(),
            "x = f'{f\"{value}\"}'".to_owned(),
            "x = '\"([{' # (".to_owned(),
            format!("x = {deep}"),
            format!("x = f'{{{deep}}}'"),
            format!("x = t'{{{deep}}}'"),
            format!("x = '\n{deep}"),
            format!("return +\nx = {deep}"),
            format!(" x = 1\nx = {deep}"),
            format!("x = 1invalid\ny = {deep}"),
            format!("x = 0b12\ny = {deep}"),
            format!("x = $\ny = {deep}"),
        ] {
            for (version, target) in [
                (PythonVersion::Py312, ruff_python_ast::PythonVersion::PY312),
                (PythonVersion::Py313, ruff_python_ast::PythonVersion::PY313),
                (PythonVersion::Py314, ruff_python_ast::PythonVersion::PY314),
                (PythonVersion::Py315, ruff_python_ast::PythonVersion::PY315),
            ] {
                let options = ParseOptions::from(Mode::Module).with_target_version(target);
                let parsed = ruff_python_parser::parse_unchecked(&source, options.clone())
                    .try_into_module()
                    .unwrap();
                let expected = validate_with_fast_scan::<false>(&source, &options, version);
                assert_eq!(
                    expected,
                    validate_parsed(&source, &options, version, &parsed),
                    "{version:?}: {source}"
                );
                if let Err(expected) = expected {
                    let options = crate::CompileOptions {
                        python_version: version,
                        ..crate::CompileOptions::default()
                    };
                    assert_eq!(
                        crate::compile_with_options(&source, "nesting.py", &options).unwrap_err(),
                        expected,
                        "{version:?}: {source}"
                    );
                }
            }
        }
    }

    #[test]
    fn lazy_template_masking_preserves_legacy_diagnostics() {
        let padding = "value = (1)\n".repeat(MAX_NESTING + 1);
        let deep = format!(
            "{}0{}",
            "(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        for (version, target) in [
            (PythonVersion::Py312, ruff_python_ast::PythonVersion::PY312),
            (PythonVersion::Py313, ruff_python_ast::PythonVersion::PY313),
        ] {
            let options = ParseOptions::from(Mode::Module).with_target_version(target);
            for prefix in [
                "t", "T", "tr", "Tr", "tR", "TR", "rt", "rT", "Rt", "RT", "bt", "tb", "ft", "tf",
                "ut", "tu", "ttr", "aT", "not", "await", "import", "assert", "except", "test",
            ] {
                for quote in ["'", "\"", "'''", "\"\"\""] {
                    let literal = format!("{prefix}{quote}text {{value}}{quote}");
                    let exposed = format!(
                        "{prefix}{quote}{{{quote}{}{quote}}}{quote}",
                        "(".repeat(MAX_NESTING + 1),
                    );
                    for source in [
                        format!("{literal}\n{padding}"),
                        format!("{literal}\n{deep}"),
                        format!("{padding}{literal}\n{deep}"),
                        format!("{exposed}\n"),
                        format!("{exposed}\n{padding}"),
                        format!("def f():\n    {literal}\n    {deep}"),
                        format!("\u{03bb}{literal}\n{deep}"),
                    ] {
                        assert_eq!(
                            validate_with_fast_scan::<true>(&source, &options, version),
                            validate_with_fast_scan::<false>(&source, &options, version),
                            "{version}: {source:?}",
                        );
                        let parsed = ruff_python_parser::parse_unchecked(&source, options.clone())
                            .try_into_module()
                            .unwrap();
                        assert_eq!(
                            validate_parsed(&source, &options, version, &parsed),
                            validate_with_fast_scan::<false>(&source, &options, version),
                            "{version}: {source:?}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn shallow_scan_agrees_with_lexer_at_nesting_and_string_boundaries() {
        let options = ParseOptions::from(Mode::Module);
        let prelude = "value = (1, 2)\n".repeat(MAX_NESTING + 1);
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for newline in ["\n", "\r", "\r\n"] {
                let source = format!(
                    "# ignored opening brackets ({newline}{}1{}",
                    "(".repeat(MAX_NESTING + 1),
                    ")".repeat(MAX_NESTING + 1),
                );
                assert_eq!(
                    validate_with_fast_scan::<true>(&source, &options, version),
                    validate_with_fast_scan::<false>(&source, &options, version),
                    "{version}: comment ending in {newline:?}",
                );
            }
            for string in [
                "'plain (['",
                "r'escaped\\\'([quote'",
                "b\"escaped\\\" ([quote\"",
                "Br'([{}])'",
                "rB'([{}])'",
                "u'([{}])'",
                "'''triple\n([\"quoted\"'''",
                "\"\"\"triple\n(['quoted'\"\"\"",
                "f'item {i + 1}'",
                "fr'item {i + 1}'",
                "rf'item {i + 1}'",
                "F'item {i + 1}'",
                "t'item {i + 1}'",
                "f'{{literal}} {i:{width}}'",
                "f'{\"(quoted)\"}'",
                "f'{f\"{i}\"}'",
                "f'{mapping['key']}'",
                "f'{f'{value}'}'",
                "fr'{values[0]:{width}.{precision}f}'",
                "f'{(lambda: 0)()}'",
                "f'{t'{value}'}'",
                "ff'{value}'",
                "\u{03bb}f'{value}'",
                "f'''{i\n# } ( [\n}'''",
                "'unterminated",
                "f'{unterminated'",
                "'''unterminated",
            ] {
                for depth in [0, MAX_NESTING - 1, MAX_NESTING, MAX_NESTING + 1] {
                    let source = format!(
                        "{prelude}{}{}{}\n",
                        "(".repeat(depth),
                        string,
                        ")".repeat(depth)
                    );
                    assert_eq!(
                        validate_with_fast_scan::<true>(&source, &options, version),
                        validate_with_fast_scan::<false>(&source, &options, version),
                        "{version}: {string:?} at depth {depth}",
                    );
                }
            }
        }
        assert!(proves_shallow_nesting(&prelude));
        assert!(proves_shallow_nesting("x = f\"Item {i+1}\"\n"));
        for string in [
            "r'([{}])'",
            "Br'([{}])'",
            "u'([{}])'",
            "f'{mapping['key']}'",
            "f'{f'{value}'}'",
            "fr'{values[0]:{width}.{precision}f}'",
        ] {
            assert!(
                proves_shallow_nesting(&format!("{prelude}{string}\n{prelude}")),
                "{string:?}",
            );
        }
        assert!(proves_shallow_nesting(&format!(
            "# {}\nx = '{}'\n",
            "(".repeat(1000),
            "[".repeat(1000),
        )));
        assert!(!proves_shallow_nesting(&"(".repeat(MAX_NESTING + 1)));
    }
}
