//! Python 3.12 does not permit defaults on parameter specifications.

use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::{Visitor, walk_stmt};
use ruff_python_ast::{self as ast, Stmt};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::CompileError;

/// Find the first unsupported `ParamSpec` default in a successfully parsed module.
pub(crate) fn validate(body: &[Stmt], tokens: &Tokens) -> Option<CompileError> {
    let mut visitor = Defaults {
        tokens,
        first: None,
    };
    visitor.visit_body(body);
    visitor
        .first
        .map(|(message, range)| CompileError::invalid_syntax(message, range))
}

struct Defaults<'a> {
    tokens: &'a Tokens,
    first: Option<(&'static str, TextRange)>,
}

impl<'ast> Visitor<'ast> for Defaults<'_> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        let (params, function) = match statement {
            Stmt::TypeAlias(alias) => (alias.type_params.as_deref(), false),
            Stmt::ClassDef(class) => (class.type_params.as_deref(), false),
            Stmt::FunctionDef(function) => (function.type_params.as_deref(), true),
            _ => (None, false),
        };
        if let Some(params) = params {
            for parameter in params {
                let ast::TypeParam::ParamSpec(parameter) = parameter else {
                    continue;
                };
                if parameter.default.is_none() {
                    continue;
                }
                let equal = self
                    .tokens
                    .in_range(parameter.range())
                    .iter()
                    .find(|token| {
                        token.start() >= parameter.name.end() && token.kind() == TokenKind::Equal
                    });
                if let Some(equal) = equal {
                    // In a function header CPython backtracks and reports the opening
                    // bracket, while aliases and classes report the equals sign.
                    let (message, range) = if function {
                        (
                            "expected '('",
                            TextRange::at(params.start(), TextSize::new(1)),
                        )
                    } else {
                        ("invalid syntax", equal.range())
                    };
                    if self
                        .first
                        .is_none_or(|(_, first)| range.start() < first.start())
                    {
                        self.first = Some((message, range));
                    }
                }
            }
        }
        walk_stmt(self, statement);
    }

    // Python declarations are statements; expressions cannot contain nested declarations.
    fn visit_expr(&mut self, _expression: &'ast ast::Expr) {}

    fn visit_pattern(&mut self, _pattern: &'ast ast::Pattern) {}
}
