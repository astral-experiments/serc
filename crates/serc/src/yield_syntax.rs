use ruff_python_ast::token::{Token, TokenKind, Tokens};
use ruff_python_ast::{BoolOp, Expr};
use ruff_python_parser::{Mode, ParseOptions};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::{CompileError, PythonVersion};

enum Context {
    Literal { kind: &'static str, start: usize },
    Field { kind: &'static str, start: usize },
    FormatSpec(&'static str),
    Delimiter,
}

/// Ruff can parse `yield, value` as a tuple containing a valueless yield.
/// CPython requires the yield to be parenthesized before that tuple comma.
pub(crate) fn validate(
    source: &str,
    tokens: &Tokens,
    python_version: PythonVersion,
) -> Result<(), CompileError> {
    if !source.contains("yield") {
        return Ok(());
    }
    let mut bare_yield = false;
    let invalid_comma = tokens.iter().enumerate().find_map(|(index, token)| {
        let kind = token.kind();
        if kind.is_trivia() {
            return None;
        }
        let invalid = bare_yield && kind == TokenKind::Comma;
        bare_yield = kind == TokenKind::Yield;
        invalid.then_some(index)
    });
    let Some(comma) = invalid_comma else {
        return Ok(());
    };
    // Valid programs need neither a context stack nor replacement-field prefix parsing.
    Err(bare_yield_error(source, tokens, comma, python_version))
}

/// Recover the first invalid pair's context only after the allocation-free token scan rejects it.
fn bare_yield_error(
    source: &str,
    tokens: &Tokens,
    comma: usize,
    python_version: PythonVersion,
) -> CompileError {
    let mut contexts = Vec::new();
    for (index, token) in tokens.iter().enumerate().take(comma) {
        match token.kind() {
            TokenKind::FStringStart => contexts.push(Context::Literal {
                kind: "f",
                start: index,
            }),
            TokenKind::TStringStart => contexts.push(Context::Literal {
                kind: "t",
                start: index,
            }),
            TokenKind::Lbrace => {
                let context = match contexts.last() {
                    Some(Context::Literal { kind, .. } | Context::FormatSpec(kind)) => {
                        Context::Field {
                            kind,
                            start: index + 1,
                        }
                    }
                    _ => Context::Delimiter,
                };
                contexts.push(context);
            }
            TokenKind::Lpar | TokenKind::Lsqb => contexts.push(Context::Delimiter),
            TokenKind::Colon => {
                if let Some(context) = contexts.last_mut()
                    && let Context::Field { kind, .. } = *context
                {
                    *context = Context::FormatSpec(kind);
                }
            }
            TokenKind::Rpar
            | TokenKind::Rsqb
            | TokenKind::Rbrace
            | TokenKind::FStringEnd
            | TokenKind::TStringEnd => {
                contexts.pop();
            }
            _ => {}
        }
    }
    // Before 3.14, t/tr is a name followed by an ordinary string. Its contents
    // cannot introduce a replacement field; diagnose its surrounding context.
    let unsupported_template = if python_version < PythonVersion::Py314 {
        contexts.iter().enumerate().find_map(|(position, context)| {
            if let Context::Literal { kind: "t", start } = context {
                Some((position, *start))
            } else {
                None
            }
        })
    } else {
        None
    };
    let (active_contexts, end) = unsupported_template
        .map_or((contexts.as_slice(), comma), |(position, start)| {
            (&contexts[..position], start)
        });
    if let Some((kind, start)) = active_contexts.iter().rev().find_map(|context| {
        if let Context::Field { kind, start } = context {
            Some((*kind, *start))
        } else {
            None
        }
    }) {
        return interpolation_error(
            source,
            &tokens[start..=end],
            kind,
            unsupported_template.is_some(),
        );
    }
    let range = unsupported_template.map_or_else(
        || tokens[comma].range(),
        |(_, start)| {
            TextRange::at(
                template_opening_quote(source, &tokens[start]),
                TextSize::new(1),
            )
        },
    );
    CompileError::invalid_syntax("invalid syntax", range)
}

/// Locate the quote following the t/tr name used by targets without template-string syntax.
fn template_opening_quote(source: &str, template: &Token) -> TextSize {
    let offset = source[template.range()]
        .find(['\'', '"'])
        .expect("a template opener contains a quote");
    template.start() + TextSize::try_from(offset).expect("token offset fits in TextSize")
}

/// Follow the native invalid replacement-field alternatives on this otherwise accepted token path.
fn interpolation_error(
    source: &str,
    tokens: &[Token],
    kind: &str,
    unsupported_template: bool,
) -> CompileError {
    let template_quote = unsupported_template.then(|| {
        let template = tokens.last().expect("the template opener is present");
        template_opening_quote(source, template)
    });
    if let Some(quote) = template_quote
        && let Some(error) = adjacent_template_error(source, tokens, quote)
    {
        return error;
    }
    let mut depth = 0;
    let mut boundaries = Vec::new();
    for token in tokens {
        if token.kind().is_trivia() {
            continue;
        }
        // An annotated_rhs can only finish outside nested delimiters and string literals.
        if depth == 0 {
            boundaries.push(token.range());
            if token.kind() == TokenKind::TStringStart
                && let Some(quote) = template_quote
            {
                boundaries.push(TextRange::at(quote, TextSize::new(1)));
            }
        }
        match token.kind() {
            TokenKind::Lpar
            | TokenKind::Lsqb
            | TokenKind::Lbrace
            | TokenKind::FStringStart
            | TokenKind::TStringStart => depth += 1,
            TokenKind::Rpar
            | TokenKind::Rsqb
            | TokenKind::Rbrace
            | TokenKind::FStringEnd
            | TokenKind::TStringEnd => depth -= 1,
            _ => {}
        }
    }
    let first = boundaries[0];
    // CPython first tries !annotated_rhs, then annotated_rhs followed by a bad field delimiter.
    // Trying complete prefixes in reverse recovers the latter rule's backtracking boundary.
    // Reparse only this invalid field, with its implicit line joining and yield/star context.
    for next in boundaries.iter().skip(1).rev() {
        let prefix = &source[first.start().to_usize()..next.start().to_usize()];
        let expression = format!("f\"{{{prefix}\n}}\"");
        if ruff_python_parser::parse_unchecked(&expression, ParseOptions::from(Mode::Module))
            .errors()
            .is_empty()
        {
            return CompileError::invalid_syntax(
                format!("{kind}-string: expecting '=', or '!', or ':', or '}}'"),
                *next,
            );
        }
    }
    CompileError::invalid_syntax(
        format!("{kind}-string: expecting a valid expression after '{{'"),
        first,
    )
}

/// Match the legacy `invalid_expression` disjunction/adjacent-expression rule before field fallback.
fn adjacent_template_error(
    source: &str,
    tokens: &[Token],
    quote: TextSize,
) -> Option<CompileError> {
    for token in tokens {
        if token.kind().is_trivia() || token.kind() == TokenKind::TStringStart {
            continue;
        }
        // Native explicitly excludes NAME STRING and soft keywords from this diagnostic.
        if matches!(&source[token.range()], "_" | "case" | "match" | "type") {
            continue;
        }
        let range = TextRange::new(token.start(), quote);
        // A disjunction is an `or` chain. Requiring the appended operand at the root
        // distinguishes bare yield/if/lambda/tuple forms from parenthesized atoms.
        let probe = format!("({}\n or __serc_probe__)", &source[range]);
        let Ok(parsed) = ruff_python_parser::parse_expression(&probe) else {
            continue;
        };
        let Expr::BoolOp(boolean) = parsed.expr() else {
            continue;
        };
        if boolean.op != BoolOp::Or
            || !matches!(
                boolean.values.last(),
                Some(Expr::Name(name)) if name.id.as_str() == "__serc_probe__"
            )
        {
            continue;
        }
        return Some(CompileError::invalid_syntax(
            "invalid syntax. Perhaps you forgot a comma?",
            range,
        ));
    }
    None
}
