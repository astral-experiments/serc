use ruff_python_ast::Expr;
use ruff_python_ast::token::{Token, TokenKind, Tokens};
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
    let mut contexts = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind() {
            TokenKind::Comment | TokenKind::NonLogicalNewline => continue,
            TokenKind::Comma if bare_yield => {
                // Before 3.14, t/tr is a name followed by an ordinary string. Its contents
                // cannot introduce a replacement field; diagnose its surrounding context.
                let unsupported_template = (python_version < PythonVersion::Py314)
                    .then(|| {
                        contexts.iter().enumerate().find_map(|(position, context)| {
                            if let Context::Literal { kind: "t", start } = context {
                                Some((position, *start))
                            } else {
                                None
                            }
                        })
                    })
                    .flatten();
                let (active_contexts, end) = unsupported_template
                    .map_or((contexts.as_slice(), index), |(position, start)| {
                        (&contexts[..position], start)
                    });
                if let Some((kind, start)) = active_contexts.iter().rev().find_map(|context| {
                    if let Context::Field { kind, start } = context {
                        Some((*kind, *start))
                    } else {
                        None
                    }
                }) {
                    return Err(interpolation_error(
                        source,
                        &tokens[start..=end],
                        kind,
                        unsupported_template.is_some(),
                    ));
                }
                return Err(CompileError::invalid_syntax(
                    "invalid syntax",
                    token.range(),
                ));
            }
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
        bare_yield = token.kind() == TokenKind::Yield;
    }
    Ok(())
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
        let offset = source[template.range()]
            .find(['\'', '"'])
            .expect("a template opener contains a quote");
        template.start() + TextSize::try_from(offset).expect("token offset fits in TextSize")
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

/// Match the legacy invalid_expression disjunction/adjacent-expression rule before field fallback.
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
        let Ok(parsed) = ruff_python_parser::parse_parenthesized_expression_range(source, range)
        else {
            continue;
        };
        // annotated_rhs also permits tuples, conditionals, and lambdas, but this invalid
        // grammar alternative specifically requires a disjunction before the string.
        if matches!(
            parsed.expr(),
            Expr::Tuple(_) | Expr::If(_) | Expr::Lambda(_)
        ) {
            continue;
        }
        return Some(CompileError::invalid_syntax(
            "invalid syntax. Perhaps you forgot a comma?",
            range,
        ));
    }
    None
}
