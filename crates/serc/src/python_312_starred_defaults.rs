//! Python 3.12 parses starred type-parameter defaults differently from later versions.

use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::{Visitor, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Pattern, Stmt};
use ruff_python_parser::Parsed;
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::CompileError;

/// Find a 3.12 parser diagnostic before the later semantic checks run.
pub(crate) fn validate(parsed: &Parsed<ast::ModModule>) -> Option<CompileError> {
    // Most modules cannot contain the `*name =` sequence. Skip the extra statement
    // walk for those modules; comments and non-logical newlines can occur within it.
    let mut previous = None;
    let mut before_previous = None;
    let mut possible_default = false;
    for token in parsed.tokens() {
        let kind = token.kind();
        if matches!(kind, TokenKind::Comment | TokenKind::NonLogicalNewline) {
            continue;
        }
        if kind == TokenKind::Equal && before_previous == Some(TokenKind::Star) {
            possible_default = true;
            break;
        }
        before_previous = previous;
        previous = Some(kind);
    }
    if !possible_default {
        return None;
    }

    let mut visitor = Defaults {
        tokens: parsed.tokens(),
        first_unsupported: parsed
            .unsupported_syntax_errors()
            .iter()
            .map(|error| error.range.start())
            .min(),
        first: None,
    };
    visitor.visit_body(parsed.suite());
    visitor
        .first
        .map(|(range, message)| CompileError::invalid_syntax(message, range))
}

struct Defaults<'a> {
    tokens: &'a Tokens,
    first_unsupported: Option<TextSize>,
    first: Option<(TextRange, &'static str)>,
}

impl Defaults<'_> {
    fn check(&mut self, params: &ast::TypeParams, kind: Declaration) {
        for parameter in params {
            let ast::TypeParam::TypeVarTuple(parameter) = parameter else {
                continue;
            };
            let Some(default) = &parameter.default else {
                continue;
            };
            if parameter.name.end() > default.start() {
                continue;
            }
            let equals = self
                .tokens
                .in_range(TextRange::new(parameter.name.end(), default.start()))
                .iter()
                .find(|token| token.kind() == TokenKind::Equal);
            let Some(equals) = equals else {
                continue;
            };

            let (range, message) = match kind {
                Declaration::Function => (
                    TextRange::at(params.start(), TextSize::from(1)),
                    "expected '('",
                ),
                Declaration::Class => (equals.range(), "invalid syntax"),
                Declaration::Alias if default.is_starred_expr() => {
                    (equals.range(), "invalid syntax")
                }
                Declaration::Alias => (
                    TextRange::new(parameter.start(), default.end()),
                    "cannot assign to iterable argument unpacking",
                ),
            };
            if self
                .first_unsupported
                .is_some_and(|start| start < range.start())
            {
                continue;
            }
            if self
                .first
                .is_none_or(|(prior, _)| range.start() < prior.start())
            {
                self.first = Some((range, message));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Declaration {
    Alias,
    Function,
    Class,
}

impl<'ast> Visitor<'ast> for Defaults<'_> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_stmt(statement));
        }
        let (params, kind) = match statement {
            Stmt::TypeAlias(alias) => (alias.type_params.as_deref(), Declaration::Alias),
            Stmt::FunctionDef(function) => (function.type_params.as_deref(), Declaration::Function),
            Stmt::ClassDef(class) => (class.type_params.as_deref(), Declaration::Class),
            _ => (None, Declaration::Alias),
        };
        if let Some(params) = params {
            self.check(params, kind);
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, _: &'ast Expr) {}

    fn visit_pattern(&mut self, _: &'ast Pattern) {}
}
