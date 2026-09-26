//! Detect warning-bearing successful token streams without pretending lexical order is parse order.

use ruff_python_ast::StringFlags;
use ruff_python_ast::token::{TokenKind, Tokens};
#[cfg(test)]
use ruff_python_ast::visitor::{Visitor, walk_stmt};
#[cfg(test)]
use ruff_python_ast::{Expr, Stmt};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::{
    CompilationUnicodeParsing, CompilationWarning, CompilationWarningPhase, PythonVersion,
};

pub(super) struct ParserWarnings {
    pub(super) warnings: Vec<CompilationWarning>,
    pub(super) complete: bool,
    pub(super) unicode: CompilationUnicodeParsing,
}

/// Recognize the single ordinary-literal case exactly. More involved tokenizer/parser
/// interleaving remains unknown until its native invocation sequence can be established.
pub(super) fn analyze(
    source: &str,
    tokens: &Tokens,
    version: PythonVersion,
    relative_lazy_warning: bool,
) -> ParserWarnings {
    let mut warnings = Vec::new();
    let mut unicode = CompilationUnicodeParsing::default();
    let mut complete = !relative_lazy_warning;
    for token in tokens {
        if token.kind() == TokenKind::Identifier && !source[token.range()].is_ascii() {
            unicode.identifiers = true;
        }
        if matches!(
            token.kind(),
            TokenKind::Int | TokenKind::Float | TokenKind::Complex
        ) {
            let tail = &source[usize::from(token.end())..];
            if ["and", "else", "for", "if", "in", "is", "or", "not"]
                .iter()
                .any(|keyword| tail.starts_with(keyword))
            {
                complete = false;
            }
            continue;
        }
        if !matches!(
            token.kind(),
            TokenKind::String | TokenKind::FStringMiddle | TokenKind::TStringMiddle
        ) {
            continue;
        }
        let flags = token.unwrap_string_flags();
        if flags.is_raw_string() {
            continue;
        }
        let ordinary = token.kind() == TokenKind::String;
        let range = if ordinary {
            TextRange::new(
                token.start() + flags.opener_len(),
                token.end() - flags.closer_len(),
            )
        } else {
            token.range()
        };
        let text = &source[range];
        unicode.named_escapes |= !flags.is_byte_string() && has_named_escape(text.as_bytes());
        if let Some((offset, length, octal)) =
            first_invalid_escape(text.as_bytes(), flags.is_byte_string())
        {
            let escape = &text[offset..offset + length];
            let descriptor = CompilationWarning {
                ordinal: 0,
                phase: CompilationWarningPhase::Parsing,
                kind: if octal {
                    "invalid_octal_escape"
                } else {
                    "invalid_escape"
                },
                category: "SyntaxWarning",
                message: escape_message(escape, octal, version),
                range: TextRange::at(
                    range.start() + TextSize::try_from(offset).unwrap(),
                    TextSize::try_from(length).unwrap(),
                ),
            };
            warnings.push(descriptor);
            complete &= ordinary && length > 1;
        }
        // The tokenizer emits a brace warning even when the backslash ends the middle token.
        if !ordinary
            && text
                .as_bytes()
                .iter()
                .rev()
                .take_while(|&&byte| byte == b'\\')
                .count()
                % 2
                == 1
            && source
                .as_bytes()
                .get(usize::from(range.end()))
                .is_some_and(|byte| matches!(byte, b'{' | b'}'))
        {
            complete = false;
        }
    }
    // Pegen can fetch warning-bearing tokens while speculating, and f/t-string middle
    // literals are decoded after their replacement expressions. A lexical sort is not a plan.
    complete &= warnings.len() <= 1;
    ParserWarnings {
        warnings,
        complete,
        unicode,
    }
}

/// Test raw escape spelling rather than decoded literal contents. Backslashes consume
/// the following character, so an escaped backslash cannot start a named-character lookup.
fn has_named_escape(text: &[u8]) -> bool {
    let mut bytes = text.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' && bytes.next() == Some(b'N') {
            return true;
        }
    }
    false
}

/// The parser compares normalized module names when warning about relative lazy imports.
#[cfg(test)]
fn has_relative_lazy_warning(source: &str, body: &[Stmt]) -> bool {
    struct RelativeLazy<'a> {
        source: &'a str,
        warned: bool,
    }
    impl<'ast> Visitor<'ast> for RelativeLazy<'_> {
        fn visit_expr(&mut self, _expression: &'ast Expr) {}
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if let Stmt::ImportFrom(import) = statement
                && import.level > 0
                && !import.is_lazy
                && let Some(module) = &import.module
                && module.as_str() == "lazy"
                && self
                    .source
                    .as_bytes()
                    .get(usize::from(module.start()).saturating_sub(1))
                    != Some(&b'.')
            {
                self.warned = true;
            }
            walk_stmt(self, statement);
        }
    }
    let mut relative = RelativeLazy {
        source,
        warned: false,
    };
    for statement in body {
        relative.visit_stmt(statement);
    }
    relative.warned
}

fn escape_message(escape: &str, octal: bool, version: PythonVersion) -> String {
    let kind = if octal { "octal escape" } else { "escape" };
    if version <= PythonVersion::Py313 {
        format!("invalid {kind} sequence '{escape}'")
    } else {
        format!(
            "\"{escape}\" is an invalid {kind} sequence. Such sequences will not work in the future. Did you mean \"\\{escape}\"? A raw string is also an option."
        )
    }
}

/// Return the first invalid escape, retaining exactly the octal digits consumed by the decoder.
fn first_invalid_escape(text: &[u8], bytes_literal: bool) -> Option<(usize, usize, bool)> {
    let mut index = 0;
    while index < text.len() {
        if text[index] != b'\\' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        let &escaped = text.get(index)?;
        index += 1;
        match escaped {
            b'\\' | b'\'' | b'"' | b'\n' | b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' => {}
            b'0'..=b'7' => {
                let mut value = u16::from(escaped - b'0');
                for _ in 0..2 {
                    if let Some(&digit @ b'0'..=b'7') = text.get(index) {
                        value = value * 8 + u16::from(digit - b'0');
                        index += 1;
                    } else {
                        break;
                    }
                }
                if value > 255 {
                    return Some((start, index - start, true));
                }
            }
            b'\r' => {
                if text.get(index) == Some(&b'\n') {
                    index += 1;
                }
            }
            b'x' => index = (index + 2).min(text.len()),
            b'u' if !bytes_literal => index = (index + 4).min(text.len()),
            b'U' if !bytes_literal => index = (index + 8).min(text.len()),
            b'N' if !bytes_literal => {
                while index < text.len() && text[index] != b'}' {
                    index += 1;
                }
                index = (index + 1).min(text.len());
            }
            _ if escaped.is_ascii() => return Some((start, 2, false)),
            // Native Unicode decoding rewrites non-ASCII characters before decoding escapes.
            // Their exact displayed warning spelling is not represented by a raw byte slice.
            _ => return Some((start, 1, false)),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(
        source: &str,
        tokens: &Tokens,
        body: &[Stmt],
        version: PythonVersion,
    ) -> ParserWarnings {
        super::analyze(
            source,
            tokens,
            version,
            version == PythonVersion::Py315 && has_relative_lazy_warning(source, body),
        )
    }

    #[test]
    fn valid_escape_prefixes_do_not_hide_later_invalid_escapes() {
        assert_eq!(
            first_invalid_escape(br"\n\x41\u0061\N{LATIN CAPITAL LETTER A}\z", false),
            Some((38, 2, false))
        );
        assert_eq!(first_invalid_escape(br"\\z", false), None);
        assert_eq!(first_invalid_escape(br"\377", false), None);
        assert_eq!(first_invalid_escape(br"\400", false), Some((0, 4, true)));
        assert_eq!(first_invalid_escape(br"\u0041", true), Some((0, 2, false)));
    }

    #[test]
    fn a_late_parser_warning_is_distinct_from_codegen_position() {
        let source = "def f():\n    pass\ntext = '\\z'\n";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let plan = analyze(
            source,
            parsed.tokens(),
            parsed.suite(),
            PythonVersion::Py314,
        );
        assert!(plan.complete);
        assert_eq!(plan.warnings.len(), 1);
        assert_eq!(plan.warnings[0].phase, CompilationWarningPhase::Parsing);
        assert_eq!(&source[plan.warnings[0].range], "\\z");
    }

    #[test]
    fn raw_literals_are_quiet_and_multiple_warning_order_is_not_assumed() {
        let source = "x = r'\\z'\ny = '\\q'\nz = '\\s'\n";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let plan = analyze(
            source,
            parsed.tokens(),
            parsed.suite(),
            PythonVersion::Py313,
        );
        assert!(!plan.complete);
        assert_eq!(plan.warnings.len(), 2);
    }

    #[test]
    fn relative_lazy_warnings_use_normalized_module_names() {
        for (source, complete) in [
            ("from .lazy import item\n", true),
            ("from . lazy import item\n", false),
            ("from . ｌａｚｙ import item\n", false),
            ("value = item. lazy\n", true),
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert_eq!(
                analyze(
                    source,
                    parsed.tokens(),
                    parsed.suite(),
                    PythonVersion::Py315
                )
                .complete,
                complete,
                "{source}"
            );
            assert!(
                analyze(
                    source,
                    parsed.tokens(),
                    parsed.suite(),
                    PythonVersion::Py314
                )
                .complete
            );
        }
        assert_eq!(
            first_invalid_escape(b"\\\r\n\\z", false),
            Some((3, 2, false))
        );
    }

    #[test]
    fn unicode_lookups_use_raw_identifier_and_literal_spelling() {
        for (source, identifiers, named_escapes) in [
            ("\u{212a} = 1\n", true, false),
            ("item.слово = 1\n", true, false),
            ("def f(слово): pass\n", true, false),
            ("# слово\ntext = 'слово'\n", false, false),
            (r"text = '\N{LATIN CAPITAL LETTER A}'", false, true),
            (r"text = '\\N{LATIN CAPITAL LETTER A}'", false, false),
            (r"text = r'\N{LATIN CAPITAL LETTER A}'", false, false),
            (r"text = b'\N{LATIN CAPITAL LETTER A}'", false, false),
            (r"text = f'\N{LATIN CAPITAL LETTER A}{value}'", false, true),
            (r"text = t'\N{LATIN CAPITAL LETTER A}{value}'", false, true),
            (r"text = f'{value:\N{LATIN CAPITAL LETTER A}}'", false, true),
            (r"text = rf'{слово}'", true, false),
            (r#"text = rf"{'\N{LATIN CAPITAL LETTER A}'}""#, false, true),
            (
                r"text = rf'\N{{LATIN CAPITAL LETTER A}}{value}'",
                false,
                false,
            ),
            (r"text = '\u005cN{LATIN CAPITAL LETTER A}'", false, false),
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            let plan = analyze(
                source,
                parsed.tokens(),
                parsed.suite(),
                PythonVersion::Py314,
            );
            assert_eq!(
                plan.unicode.normalizes_identifiers(),
                identifiers,
                "{source}"
            );
            assert_eq!(
                plan.unicode.decodes_named_escapes(),
                named_escapes,
                "{source}"
            );
        }
    }
}
