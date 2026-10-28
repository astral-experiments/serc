use std::{borrow::Cow, sync::OnceLock};

use ruff_python_ast::token::TokenKind;
use ruff_python_ast::visitor::transformer::{Transformer, walk_stmt};
use ruff_python_ast::{Expr, ExprContext, ModModule, Stmt};
use ruff_python_parser::{ParseError, ParseErrorType, ParseOptions, Parsed};
use ruff_text_size::{Ranged, TextSize};
use rustc_hash::FxHashSet;

/// Recover the diagnostics and missing bare-raise operands of the registry parser.
pub(crate) fn recover<'a>(
    source: &str,
    options: &ParseOptions,
    parsed: &'a Parsed<ModModule>,
) -> (Cow<'a, [ParseError]>, BareRaises) {
    // A packaged dependency resolves to the registry release. The Git backport
    // already handles both constructs, including its own malformed-input diagnostics.
    static NEEDS_RECOVERY: OnceLock<bool> = OnceLock::new();

    let mut raises = BareRaises::default();
    let candidates = parsed.errors().iter().any(|error| match error.error {
        ParseErrorType::ExpectedExpression => &source[error.location] == ";",
        ParseErrorType::SimpleAndCompoundStatementOnSameLine => &source[error.location] == "match",
        _ => false,
    });
    if !candidates
        || !NEEDS_RECOVERY
            .get_or_init(|| ruff_python_parser::parse_module("raise; match = value\n").is_err())
    {
        return (Cow::Borrowed(parsed.errors()), raises);
    }
    let mut errors = match_errors(source, options, parsed);
    if !errors.is_empty() {
        let tokens = parsed.tokens();
        errors.to_mut().retain(|error| {
            if error.error == ParseErrorType::ExpectedExpression && &source[error.location] == ";" {
                let index = tokens.partition_point(|token| token.start() < error.location.start());
                if let Some(token) = tokens.get(index)
                    && token.kind() == TokenKind::Semi
                    && token.range() == error.location
                    && let Some(previous) =
                        tokens[..index].iter().rev().find(|t| !t.kind().is_trivia())
                    && previous.kind() == TokenKind::Raise
                {
                    raises.0.insert(previous.end());
                    return false;
                }
            }
            true
        });
    }
    (errors, raises)
}

#[derive(Default)]
pub(crate) struct BareRaises(FxHashSet<TextSize>);

impl BareRaises {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Transformer for BareRaises {
    fn visit_stmt(&self, statement: &mut Stmt) {
        if let Stmt::Raise(raised) = statement
            && let Some(Expr::Name(name)) = raised.exc.as_deref()
            && name.ctx == ExprContext::Invalid
            && name.id.is_empty()
            && name.range.is_empty()
            && self.0.contains(&name.start())
        {
            raised.exc = None;
        } else {
            walk_stmt(self, statement);
        }
    }

    fn visit_expr(&self, _expression: &mut Expr) {}
}

/// Recover genuine parser diagnostics hidden by `match` used as an identifier.
fn match_errors<'a>(
    source: &str,
    options: &ParseOptions,
    parsed: &'a Parsed<ModModule>,
) -> Cow<'a, [ParseError]> {
    let mut view: Option<String> = None;
    for error in parsed.errors() {
        let range = error.location;
        if error.error == ParseErrorType::SimpleAndCompoundStatementOnSameLine
            && &source[range] == "match"
            && matches!(parsed.tokens().in_range(range), [token]
                if token.kind() == TokenKind::Identifier && token.range() == range)
        {
            view.get_or_insert_with(|| source.to_owned())
                .replace_range(range.start().to_usize()..range.end().to_usize(), "Match");
        }
    }
    let Some(view) = view else {
        return Cow::Borrowed(parsed.errors());
    };

    // Ruff suppresses later errors at the same start offset. Parsing the
    // identifier's five bytes unambiguously can recover an invalid target.
    let reparsed = super::ParsedModule::new(
        ruff_python_parser::parse_unchecked(&view, options.clone())
            .try_into_module()
            .expect("parser returned a module"),
    );
    Cow::Owned(
        reparsed
            .parsed
            .as_ref()
            .expect("module is present")
            .errors()
            .to_vec(),
    )
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::ParseErrorType;

    use crate::{CompileError, compile};

    #[test]
    fn semicolons_preserve_assignment_and_compound_statement_errors() {
        for (source, kind, highlight) in [
            (
                "x; match(value) = 1\n",
                ParseErrorType::InvalidAssignmentTarget,
                "match(value)",
            ),
            (
                "x; match (value):\n    case _: pass\n",
                ParseErrorType::SimpleAndCompoundStatementOnSameLine,
                "match",
            ),
        ] {
            let CompileError::Parse(error) = compile(source, "example.py").unwrap_err() else {
                panic!("expected a parser diagnostic for {source:?}");
            };
            assert_eq!(error.error, kind);
            assert_eq!(&source[error.location], highlight);
        }
    }
}
