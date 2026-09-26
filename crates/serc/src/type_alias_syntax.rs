use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::{Visitor, walk_stmt};
use ruff_python_ast::{Expr, Operator, Stmt, TypeParam};
use ruff_python_parser::{ParseError, UnsupportedSyntaxError};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::{CompileError, PythonVersion};

/// Replay the invalid-assignment alternative reached while retrying a type alias.
/// A valid alias does not take this path: CPython only retries the grammar after
/// another syntax error, or when its target version rejects the default itself.
pub(crate) fn before_error(
    source: &str,
    suite: &[Stmt],
    tokens: &Tokens,
    errors: &[ParseError],
    unsupported: &[UnsupportedSyntaxError],
    version: PythonVersion,
    error: &CompileError,
) -> Option<CompileError> {
    if !source.contains("type")
        || crate::yield_syntax::tokenizer_error_takes_priority(source, tokens, errors)
    {
        return None;
    }
    let limit = error.range()?.start();
    let mut visitor = AliasVisitor {
        source,
        tokens,
        unsupported,
        version,
        limit,
        result: None,
    };
    visitor.visit_body(suite);
    visitor.result.map(|range| {
        CompileError::invalid_syntax(
            "invalid syntax. Maybe you meant '==' or ':=' instead of '='?",
            range,
        )
    })
}

struct AliasVisitor<'a> {
    source: &'a str,
    tokens: &'a Tokens,
    unsupported: &'a [UnsupportedSyntaxError],
    version: PythonVersion,
    limit: TextSize,
    result: Option<TextRange>,
}

impl<'a> Visitor<'a> for AliasVisitor<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_stmt(stmt));
        }
        if stmt.start() > self.limit {
            return;
        }
        if let Stmt::TypeAlias(alias) = stmt
            && let Some(params) = &alias.type_params
        {
            for param in &params.type_params {
                let param = match param {
                    TypeParam::TypeVar(param) => param,
                    // A previous starred default can trigger its own invalid
                    // assignment diagnostic before any following alias.
                    TypeParam::TypeVarTuple(param) if param.default.is_some() => {
                        self.limit = self.limit.min(param.start());
                        break;
                    }
                    TypeParam::TypeVarTuple(_) => continue,
                    // The invalid argument alternative cannot advance past a
                    // parameter specification to later parameters in this alias.
                    TypeParam::ParamSpec(param) => {
                        // Python 3.12 rejects this default before it reaches a
                        // later alias, even when its error is reported later.
                        if self.version == PythonVersion::Py312 && param.default.is_some() {
                            self.limit = self.limit.min(param.start());
                        }
                        break;
                    }
                };
                if param.name.start() > self.limit {
                    break;
                }
                if param.bound.is_some() && param.default.is_some() {
                    break;
                }
                if param.default.is_none() {
                    continue;
                }
                // Preserve an unrelated earlier unsupported construct.
                if self
                    .unsupported
                    .iter()
                    .any(|error| error.range.start() < param.name.start())
                {
                    continue;
                }
                let Some(end) = bitwise_prefix_end(self.source, self.tokens, param.range()) else {
                    break;
                };
                let range = TextRange::new(
                    param.name.start(),
                    native_error_end(self.source, param.name.start(), end),
                );
                if self
                    .result
                    .is_none_or(|earlier| range.start() < earlier.start())
                {
                    self.result = Some(range);
                }
            }
        }
        walk_stmt(self, stmt);
    }

    // Expressions cannot contain alias statements.
    fn visit_expr(&mut self, _expr: &'a Expr) {}
}

/// CPython converts both columns against the diagnostic's first source line,
/// even when the expression ends on a later line. Convert its resulting
/// character column back into a byte offset on the ending line.
fn native_error_end(source: &str, start: TextSize, end: TextSize) -> TextSize {
    let start = start.to_usize();
    let end = end.to_usize();
    let start_line = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let end_line = source[..end].rfind('\n').map_or(0, |index| index + 1);
    if start_line == end_line {
        return TextSize::try_from(end).unwrap();
    }
    let first_line = source[start_line..].split('\n').next().unwrap();
    let last_line = source[end_line..].split('\n').next().unwrap();
    // The native API counts a one-based byte offset, decodes partial UTF-8
    // using replacement, and includes the terminator if past the line end.
    let column = end - end_line + 1;
    let prefix = &first_line.as_bytes()[..column.min(first_line.len())];
    let characters =
        String::from_utf8_lossy(prefix).chars().count() + usize::from(column > first_line.len());
    let target = characters.saturating_sub(1);
    let offset = last_line
        .char_indices()
        .nth(target)
        .map_or(last_line.len(), |(index, _)| index);
    TextSize::try_from(end_line + offset).unwrap()
}

/// Find a default's first top-level bitwise-or expression, including parens.
fn bitwise_prefix_end(source: &str, tokens: &Tokens, param: TextRange) -> Option<TextSize> {
    let mut tokens = tokens
        .in_range(param)
        .iter()
        .filter(|token| !token.kind().is_trivia());
    tokens.find(|token| token.kind() == TokenKind::Equal)?;
    let first = tokens.next()?;
    let mut depth = 0usize;
    let mut end = first.start();
    for token in std::iter::once(first).chain(tokens) {
        let kind = token.kind();
        if depth == 0 && ends_bitwise_or(kind) {
            // The native invalid-named-expression alternative excludes these.
            if matches!(kind, TokenKind::Equal | TokenKind::ColonEqual) {
                return None;
            }
            break;
        }
        match kind {
            TokenKind::Lpar
            | TokenKind::Lsqb
            | TokenKind::Lbrace
            | TokenKind::FStringStart
            | TokenKind::TStringStart => depth += 1,
            TokenKind::Rpar
            | TokenKind::Rsqb
            | TokenKind::Rbrace
            | TokenKind::FStringEnd
            | TokenKind::TStringEnd => depth = depth.checked_sub(1)?,
            _ => {}
        }
        end = token.end();
    }
    if end == first.start() || depth != 0 {
        return None;
    }
    let probe = format!(
        "({}\n | __serc_probe__)",
        &source[TextRange::new(first.start(), end)]
    );
    // The temporary tree may contain a very deep expression. Use the same
    // guarded drop as the module being compiled.
    let parsed = ruff_python_parser::parse_unchecked(
        &probe,
        ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::Module),
    )
    .try_into_module()?;
    let parsed = crate::ParsedModule(Some(parsed));
    let parsed = parsed.0.as_ref()?;
    if !parsed.errors().is_empty() {
        return None;
    }
    let Stmt::Expr(statement) = parsed.suite().first()? else {
        return None;
    };
    let Expr::BinOp(binary) = &*statement.value else {
        return None;
    };
    if binary.op != Operator::BitOr
        || !matches!(&*binary.right, Expr::Name(name) if name.id.as_str() == "__serc_probe__")
    {
        return None;
    }
    // The native error uses the expression node's end, which excludes
    // redundant parentheses but includes delimiters belonging to tuple or
    // generator expressions. The probe added one leading parenthesis.
    Some(first.start() + binary.left.end() - TextSize::new(1))
}

fn ends_bitwise_or(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::If
            | TokenKind::In
            | TokenKind::Is
            | TokenKind::Less
            | TokenKind::Greater
            | TokenKind::LessEqual
            | TokenKind::GreaterEqual
            | TokenKind::EqEqual
            | TokenKind::NotEqual
            | TokenKind::Equal
            | TokenKind::ColonEqual
    )
}
