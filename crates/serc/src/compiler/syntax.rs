//! Source validation runs before lowering can discard unreachable statements.

use std::borrow::Cow;
use std::cell::RefCell;

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_parameters, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_python_parser::semantic_errors::{
    AwaitOutsideAsyncFunctionKind, InvalidExpressionKind, LazyImportContext, SemanticSyntaxChecker,
    SemanticSyntaxContext, SemanticSyntaxError, SemanticSyntaxErrorKind,
};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{CompileError, PythonVersion};

use super::constant::explicit_surrogate_string;
use super::statement::suite_contains_yield_in_function_scope;
use super::symbol::mangle_name;

/// Validate AST-based syntax restrictions using Ruff's shared checker.
pub(crate) fn validate(
    source: &str,
    body: &[Stmt],
    python_version: PythonVersion,
) -> Result<(), CompileError> {
    validate_with_statement_pruning::<true>(source, body, python_version)
}

fn validate_with_statement_pruning<const PRUNE_STATEMENTS: bool>(
    source: &str,
    body: &[Stmt],
    python_version: PythonVersion,
) -> Result<(), CompileError> {
    let mut validator = new_validator::<PRUNE_STATEMENTS, false>(source, body, python_version);
    validator.visit_scope_body(body);
    let error = validator.error.into_inner();
    if matches!(&error, Some(CompileError::InvalidDocstring { .. })) {
        // Native validates future directives before cleaning docstrings. Revisit only this rare
        // error path so an invalid directive anywhere retains its higher diagnostic priority.
        let mut validator = new_validator::<false, true>(source, body, python_version);
        validator.visit_body(body);
        if let Some(future_error) = validator.error.into_inner() {
            return Err(future_error);
        }
    }
    error.map_or(Ok(()), Err)
}

fn new_validator<'source, 'ast, const PRUNE_STATEMENTS: bool, const FUTURE_ONLY: bool>(
    source: &'source str,
    body: &'ast [Stmt],
    python_version: PythonVersion,
) -> Validator<'source, 'ast, PRUNE_STATEMENTS, FUTURE_ONLY> {
    Validator {
        checker: SemanticSyntaxChecker::new(),
        checker_prefix: CheckerPrefix::Initial,
        error: RefCell::new(None),
        source,
        body,
        python_version: match python_version {
            PythonVersion::Py312 => ast::PythonVersion::PY312,
            PythonVersion::Py313 => ast::PythonVersion::PY313,
            PythonVersion::Py314 => ast::PythonVersion::PY314,
            PythonVersion::Py315 => ast::PythonVersion::PY315,
        },
        future_annotations: body.iter().any(|statement| {
            matches!(statement, Stmt::ImportFrom(import)
                if import.level == 0 && import.module.as_deref() == Some("__future__")
                    && import.names.iter().any(|alias| alias.name.as_str() == "annotations"))
        }),
        private_name: None,
        mangled_names: None,
        scopes: vec![Scope::new(ScopeKind::Module)],
    }
}

#[cfg(test)]
fn validate_without_statement_pruning(
    source: &str,
    body: &[Stmt],
    python_version: PythonVersion,
) -> Result<(), CompileError> {
    validate_with_statement_pruning::<false>(source, body, python_version)
}

/// Returns the UTF-8 failure CPython encounters before it starts cleaning a docstring.
fn surrogate_docstring_error(source: &str, string: &ast::ExprStringLiteral) -> Option<String> {
    if !string
        .value
        .iter()
        .any(|part| !part.flags.prefix().is_raw() && part.as_str().contains('\u{fffd}'))
    {
        return None;
    }

    let mut value = Vec::new();
    for part in &string.value {
        if !part.flags.prefix().is_raw()
            && let Some(restored) = explicit_surrogate_string(part.as_str(), &source[part.range])
        {
            value.extend(restored);
        } else {
            value.extend_from_slice(part.as_str().as_bytes());
        }
    }

    // The restored value is UTF-8 with surrogate code points retained. Only leading bytes
    // contribute a character position, and CPython reports the first contiguous surrogate run.
    let mut first = None;
    let mut end = 0;
    for (position, (offset, _)) in value
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte & 0xc0 != 0x80)
        .enumerate()
    {
        if let [0xed, middle @ 0xa0..=0xbf, last @ 0x80..=0xbf, ..] = value[offset..] {
            let codepoint = 0xd000 | (u16::from(middle & 0x3f) << 6) | u16::from(last & 0x3f);
            first.get_or_insert((position, codepoint));
            end = position + 1;
        } else if first.is_some() {
            break;
        }
    }
    let (start, codepoint) = first?;
    let characters = if end == start + 1 {
        format!("character '\\u{codepoint:04x}' in position {start}")
    } else {
        format!("characters in position {start}-{}", end - 1)
    };
    Some(format!(
        "'utf-8' codec can't encode {characters}: surrogates not allowed"
    ))
}

#[derive(Clone, Copy)]
enum CheckerPrefix {
    Initial,
    Open,
    Closed,
}

#[derive(Clone, Copy)]
enum ScopeKind<'ast> {
    Module,
    Class,
    Function {
        is_async: bool,
        is_async_generator: bool,
        parameters: Option<&'ast ast::Parameters>,
    },
    Comprehension {
        generator: bool,
        is_async: bool,
    },
}

struct Scope<'ast> {
    kind: ScopeKind<'ast>,
    loop_depth: usize,
    except_star_loop_depth: Option<usize>,
    in_try: bool,
}

impl<'ast> Scope<'ast> {
    fn new(kind: ScopeKind<'ast>) -> Self {
        Self {
            kind,
            loop_depth: 0,
            except_star_loop_depth: None,
            in_try: false,
        }
    }
}

struct Validator<'source, 'ast, const PRUNE_STATEMENTS: bool, const FUTURE_ONLY: bool> {
    checker: SemanticSyntaxChecker,
    checker_prefix: CheckerPrefix,
    error: RefCell<Option<CompileError>>,
    source: &'source str,
    body: &'ast [Stmt],
    python_version: ast::PythonVersion,
    future_annotations: bool,
    private_name: Option<&'ast str>,
    mangled_names: Option<FxHashSet<&'ast str>>,
    scopes: Vec<Scope<'ast>>,
}

impl<'ast, const PRUNE_STATEMENTS: bool, const FUTURE_ONLY: bool>
    Validator<'_, 'ast, PRUNE_STATEMENTS, FUTURE_ONLY>
{
    fn visit_scope_body(&mut self, body: &'ast [Stmt]) {
        if self.error.borrow().is_none()
            && self.python_version >= ast::PythonVersion::PY313
            && let Some(Stmt::Expr(statement)) = body.first()
            && let Expr::StringLiteral(string) = statement.value.as_ref()
            && let Some(message) = surrogate_docstring_error(self.source, string)
        {
            *self.error.borrow_mut() = Some(CompileError::InvalidDocstring {
                message,
                range: string.range,
            });
        }
        self.visit_body(body);
    }

    fn scope(&self) -> &Scope<'ast> {
        self.scopes
            .last()
            .expect("validation always has a module scope")
    }

    fn scope_mut(&mut self) -> &mut Scope<'ast> {
        self.scopes
            .last_mut()
            .expect("validation always has a module scope")
    }

    fn reject_except_star_transfer(&self, statement: &Stmt) {
        let mut error = self.error.borrow_mut();
        if error.is_none() {
            *error = Some(CompileError::invalid_syntax(
                "'break', 'continue' and 'return' cannot appear in an except* block",
                statement.range(),
            ));
        }
    }

    /// Ruff's annotation walk also visits independent lambda and lazy generator bodies.
    /// Their expressions are checked again below in the scope where they actually execute.
    fn escapes_annotation_scope(&self, range: TextRange, kind: &InvalidExpressionKind) -> bool {
        struct Finder {
            range: TextRange,
            is_await: bool,
            found: bool,
        }
        impl<'ast> Visitor<'ast> for Finder {
            fn visit_stmt(&mut self, statement: &'ast Stmt) {
                if !self.found && statement.range().contains_range(self.range) {
                    walk_stmt(self, statement);
                }
            }
            fn visit_expr(&mut self, expression: &'ast Expr) {
                if self.found || !expression.range().contains_range(self.range) {
                    return;
                }
                self.found = match expression {
                    Expr::Lambda(lambda) => lambda.body.range().contains_range(self.range),
                    Expr::Generator(generator) if self.is_await => {
                        // Only the first iterable executes eagerly in the annotation scope.
                        generator
                            .generators
                            .first()
                            .is_some_and(|first| !first.iter.range().contains_range(self.range))
                    }
                    _ => false,
                };
                if !self.found {
                    walk_expr(self, expression);
                }
            }
        }
        let mut finder = Finder {
            range,
            is_await: matches!(kind, InvalidExpressionKind::Await),
            found: false,
        };
        finder.visit_body(self.body);
        finder.found
    }

    fn with_scope(&mut self, kind: ScopeKind<'ast>, visit: impl FnOnce(&mut Self)) {
        self.scopes.push(Scope::new(kind));
        visit(self);
        self.scopes.pop();
    }

    fn with_private_context(
        &mut self,
        private_name: &'ast str,
        mangled_names: Option<FxHashSet<&'ast str>>,
        visit: impl FnOnce(&mut Self),
    ) {
        let previous_private = self.private_name.replace(private_name);
        let previous_mangled = std::mem::replace(&mut self.mangled_names, mangled_names);
        visit(self);
        self.private_name = previous_private;
        self.mangled_names = previous_mangled;
    }

    fn validate_private_parameters(&self, parameters: &'ast ast::Parameters) {
        let Some(private_name) = self.private_name else {
            return;
        };
        // This check only reports differently spelled parameters that mangling makes equal.
        // Ruff already checks duplicate spellings, including when none of them can be mangled.
        if self.error.borrow().is_some()
            || parameters.len() < 2
            || private_name.trim_start_matches('_').is_empty()
            || !parameters.iter().any(|parameter| {
                let spelling = parameter.name().as_str();
                spelling.starts_with("__")
                    && !spelling.ends_with("__")
                    && !spelling.contains('.')
                    && self
                        .mangled_names
                        .as_ref()
                        .is_none_or(|names| names.contains(spelling))
            })
        {
            return;
        }
        let mut seen = FxHashMap::default();
        // Symbol analysis visits keyword-only parameters before either variadic parameter.
        for parameter in parameters
            .iter_non_variadic_params()
            .map(|parameter| &parameter.parameter)
            .chain(parameters.vararg.as_deref())
            .chain(parameters.kwarg.as_deref())
        {
            let spelling = parameter.name.as_str();
            let canonical = if self
                .mangled_names
                .as_ref()
                .is_some_and(|names| !names.contains(spelling))
            {
                Cow::Borrowed(spelling)
            } else {
                mangle_name(spelling, self.private_name)
            };
            if let Some(previous) = seen.insert(canonical, spelling)
                && previous != spelling
            {
                let kind = if self.python_version >= ast::PythonVersion::PY315 {
                    "parameter"
                } else {
                    "argument"
                };
                *self.error.borrow_mut() = Some(CompileError::invalid_syntax(
                    format!("duplicate {kind} '{spelling}' in function definition"),
                    parameter.name.range(),
                ));
                return;
            }
        }
    }

    fn with_checker(&mut self, visit: impl FnOnce(&mut SemanticSyntaxChecker, &Self)) {
        let mut checker = std::mem::take(&mut self.checker);
        visit(&mut checker, self);
        self.checker = checker;
    }

    fn check_statement(&mut self, statement: &Stmt) {
        // Relative imports of a module named __future__ are ordinary imports. The shared
        // checker identifies directives by their module spelling, so hide that spelling here.
        if let Stmt::ImportFrom(import) = statement
            && import.level > 0
            && import.module.as_deref() == Some("__future__")
        {
            let mut import = import.clone();
            import.module = None;
            self.with_checker(|checker, context| {
                checker.visit_stmt(&Stmt::ImportFrom(import), context);
            });
        } else {
            self.with_checker(|checker, context| checker.visit_stmt(statement, context));
        }
    }

    fn statement_needs_checker(&mut self, statement: &Stmt) -> bool {
        if !PRUNE_STATEMENTS {
            return true;
        }

        // Ruff updates its docstring and future-import boundaries on statement entry. Keep every
        // call until both are closed; after that its remaining statement state never changes.
        if !matches!(self.checker_prefix, CheckerPrefix::Closed) {
            let keeps_open = match statement {
                Stmt::Expr(statement) => {
                    matches!(self.checker_prefix, CheckerPrefix::Initial)
                        && statement.value.is_string_literal_expr()
                }
                Stmt::ImportFrom(import) => {
                    import.level == 0
                        && !import.is_lazy
                        && import.module.as_deref() == Some("__future__")
                }
                _ => false,
            };
            self.checker_prefix = if keeps_open {
                CheckerPrefix::Open
            } else {
                CheckerPrefix::Closed
            };
            return true;
        }

        // The pinned checker reports errors only for these statement kinds. Parent checks for
        // starred expressions apply to direct children; other child errors are still visited.
        match statement {
            Stmt::FunctionDef(_)
            | Stmt::ClassDef(_)
            | Stmt::TypeAlias(_)
            | Stmt::AnnAssign(_)
            | Stmt::Match(_)
            | Stmt::Try(_)
            | Stmt::Import(_)
            | Stmt::ImportFrom(_)
            | Stmt::Global(_)
            | Stmt::Nonlocal(_) => true,
            Stmt::Assign(statement) => {
                statement.value.is_starred_expr()
                    || matches!(statement.targets.as_slice(), [Expr::Starred(_)])
            }
            Stmt::Return(statement) => {
                !self.in_function_scope()
                    || statement
                        .value
                        .as_deref()
                        .is_some_and(Expr::is_starred_expr)
            }
            Stmt::For(statement) => {
                statement.is_async
                    || statement.target.is_starred_expr()
                    || statement.iter.is_starred_expr()
            }
            Stmt::With(statement) => statement.is_async,
            Stmt::Break(_) | Stmt::Continue(_) => !self.in_loop_context(),
            Stmt::Delete(_)
            | Stmt::AugAssign(_)
            | Stmt::While(_)
            | Stmt::If(_)
            | Stmt::Raise(_)
            | Stmt::Assert(_)
            | Stmt::Expr(_)
            | Stmt::Pass(_)
            | Stmt::IpyEscapeCommand(_) => false,
        }
    }

    fn comprehension(
        &mut self,
        generators: &'ast [ast::Comprehension],
        generator: bool,
        visit_result: impl FnOnce(&mut Self),
    ) {
        let Some((first, rest)) = generators.split_first() else {
            return;
        };
        // The first iterable executes in the enclosing scope, even for a lazy generator.
        self.visit_expr(&first.iter);
        self.with_scope(
            ScopeKind::Comprehension {
                generator,
                is_async: generators.iter().any(|clause| clause.is_async),
            },
            |visitor| {
                visitor.visit_expr(&first.target);
                for condition in &first.ifs {
                    visitor.visit_expr(condition);
                }
                for clause in rest {
                    visitor.visit_comprehension(clause);
                }
                visit_result(visitor);
            },
        );
    }
}

impl<'ast, const PRUNE_STATEMENTS: bool, const FUTURE_ONLY: bool> Visitor<'ast>
    for Validator<'_, 'ast, PRUNE_STATEMENTS, FUTURE_ONLY>
{
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if self.error.borrow().is_some() {
            return;
        }
        if FUTURE_ONLY {
            self.check_statement(statement);
            walk_stmt(self, statement);
            return;
        }
        // This precedes return-value checks and except* transfer checks, as in CPython.
        if let Stmt::Return(statement) = statement
            && statement.value.is_some()
            && matches!(
                self.scope().kind,
                ScopeKind::Function {
                    is_async_generator: true,
                    ..
                }
            )
        {
            self.report_semantic_error(SemanticSyntaxError {
                kind: SemanticSyntaxErrorKind::ReturnInGenerator,
                range: statement.range,
                python_version: self.python_version,
            });
            return;
        }
        // A loop contained within the handler can be exited without leaving except*.
        // Independent function and class scopes start without the enclosing boundary.
        if matches!(statement, Stmt::Break(_) | Stmt::Continue(_))
            && self.scope().except_star_loop_depth == Some(self.scope().loop_depth)
        {
            self.reject_except_star_transfer(statement);
            return;
        }
        if self.statement_needs_checker(statement) {
            self.check_statement(statement);
        }
        match statement {
            Stmt::FunctionDef(function) => {
                for decorator in &function.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(params) = &function.type_params {
                    self.visit_type_params(params);
                }
                self.visit_parameters(&function.parameters);
                if let Some(returns) = &function.returns {
                    self.visit_annotation(returns);
                }
                self.with_scope(
                    ScopeKind::Function {
                        is_async: function.is_async,
                        // Ruff's return checker skips eager nested headers and defaults.
                        is_async_generator: function.is_async
                            && suite_contains_yield_in_function_scope(
                                &function.body,
                                !self.future_annotations
                                    && self.python_version < ast::PythonVersion::PY314,
                            ),
                        parameters: Some(&function.parameters),
                    },
                    |visitor| visitor.visit_scope_body(&function.body),
                );
            }
            Stmt::ClassDef(class) => {
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(params) = &class.type_params {
                    self.with_private_context(
                        class.name.as_str(),
                        Some(FxHashSet::default()),
                        |visitor| {
                            for parameter in params.iter() {
                                // Generic class headers only mangle type parameters already
                                // declared when this bound or default is visited.
                                visitor
                                    .mangled_names
                                    .as_mut()
                                    .expect("generic headers have restricted name mangling")
                                    .insert(parameter.name().as_str());
                                visitor.visit_type_param(parameter);
                            }
                            if let Some(arguments) = &class.arguments {
                                visitor.visit_arguments(arguments);
                            }
                            visitor.mangled_names = None;
                            visitor.with_scope(ScopeKind::Class, |visitor| {
                                visitor.visit_scope_body(&class.body);
                            });
                        },
                    );
                } else {
                    if let Some(arguments) = &class.arguments {
                        self.visit_arguments(arguments);
                    }
                    self.with_private_context(class.name.as_str(), None, |visitor| {
                        visitor.with_scope(ScopeKind::Class, |visitor| {
                            visitor.visit_scope_body(&class.body);
                        });
                    });
                }
            }
            Stmt::For(statement) => {
                self.visit_expr(&statement.iter);
                self.visit_expr(&statement.target);
                self.scope_mut().loop_depth += 1;
                self.visit_body(&statement.body);
                self.scope_mut().loop_depth -= 1;
                self.visit_body(&statement.orelse);
            }
            Stmt::While(statement) => {
                self.visit_expr(&statement.test);
                self.scope_mut().loop_depth += 1;
                self.visit_body(&statement.body);
                self.scope_mut().loop_depth -= 1;
                self.visit_body(&statement.orelse);
            }
            Stmt::Return(_) => {
                // CPython validates the returned expression before unwinding the handler.
                walk_stmt(self, statement);
                if self.scope().except_star_loop_depth.is_some() {
                    self.reject_except_star_transfer(statement);
                }
            }
            Stmt::Try(try_statement) => {
                let previous = std::mem::replace(&mut self.scope_mut().in_try, true);
                if try_statement.is_star {
                    self.visit_body(&try_statement.body);
                    for ast::ExceptHandler::ExceptHandler(handler) in &try_statement.handlers {
                        if let Some(type_) = &handler.type_ {
                            self.visit_expr(type_);
                        }
                        let scope = self.scope_mut();
                        let previous = scope.except_star_loop_depth.replace(scope.loop_depth);
                        self.visit_body(&handler.body);
                        self.scope_mut().except_star_loop_depth = previous;
                    }
                    self.visit_body(&try_statement.orelse);
                    self.visit_body(&try_statement.finalbody);
                } else {
                    walk_stmt(self, statement);
                }
                self.scope_mut().in_try = previous;
            }
            _ => walk_stmt(self, statement),
        }
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if FUTURE_ONLY {
            return;
        }
        // This adapter does not supply declarations to Ruff's global-order check. A bare name
        // therefore only needs checking when it assigns to or deletes `__debug__`. Ordinary
        // literals and attribute wrappers cannot report errors or update validation state.
        match expression {
            Expr::Name(name)
                if name.id.as_str() != "__debug__"
                    || !(name.ctx.is_store() || name.ctx.is_del()) =>
            {
                return;
            }
            Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_) => return,
            Expr::Attribute(attribute) => {
                let mut base = attribute.value.as_ref();
                while let Expr::Attribute(attribute) = base {
                    base = attribute.value.as_ref();
                }
                self.visit_expr(base);
                return;
            }
            _ => {}
        }
        if self.error.borrow().is_some() {
            return;
        }
        let check_expression = match expression {
            // These are the only remaining variants checked by the pinned Ruff visitor. Its
            // tuple/list check applies only to assignment targets; a lambda without parameters
            // has no identifiers or duplicates to check. All other expressions still recurse.
            Expr::Name(_)
            | Expr::ListComp(_)
            | Expr::SetComp(_)
            | Expr::DictComp(_)
            | Expr::Generator(_)
            | Expr::Yield(_)
            | Expr::YieldFrom(_)
            | Expr::Await(_) => true,
            Expr::Tuple(tuple) => tuple.ctx.is_store(),
            Expr::List(list) => list.ctx.is_store(),
            Expr::Lambda(lambda) => lambda.parameters.is_some(),
            Expr::BoolOp(_)
            | Expr::Named(_)
            | Expr::BinOp(_)
            | Expr::UnaryOp(_)
            | Expr::If(_)
            | Expr::Dict(_)
            | Expr::Set(_)
            | Expr::Compare(_)
            | Expr::Call(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::FString(_)
            | Expr::TString(_)
            | Expr::Attribute(_)
            | Expr::Subscript(_)
            | Expr::Starred(_)
            | Expr::Slice(_)
            | Expr::IpyEscapeCommand(_) => false,
        };
        if check_expression {
            self.with_checker(|checker, context| checker.visit_expr(expression, context));
        }
        match expression {
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.visit_parameters(parameters);
                }
                self.with_scope(
                    ScopeKind::Function {
                        is_async: false,
                        is_async_generator: false,
                        parameters: lambda.parameters.as_deref(),
                    },
                    |visitor| visitor.visit_expr(&lambda.body),
                );
            }
            Expr::ListComp(ast::ExprListComp {
                elt, generators, ..
            })
            | Expr::SetComp(ast::ExprSetComp {
                elt, generators, ..
            }) => {
                self.comprehension(generators, false, |visitor| visitor.visit_expr(elt));
            }
            Expr::Generator(ast::ExprGenerator {
                elt, generators, ..
            }) => {
                self.comprehension(generators, true, |visitor| visitor.visit_expr(elt));
            }
            Expr::DictComp(ast::ExprDictComp {
                key,
                value,
                generators,
                ..
            }) => {
                self.comprehension(generators, false, |visitor| {
                    if let Some(key) = key {
                        visitor.visit_expr(key);
                    }
                    visitor.visit_expr(value);
                });
            }
            _ => walk_expr(self, expression),
        }
    }

    fn visit_parameters(&mut self, parameters: &'ast ast::Parameters) {
        if FUTURE_ONLY {
            return;
        }
        self.validate_private_parameters(parameters);
        walk_parameters(self, parameters);
    }
}

impl<const PRUNE_STATEMENTS: bool, const FUTURE_ONLY: bool> SemanticSyntaxContext
    for Validator<'_, '_, PRUNE_STATEMENTS, FUTURE_ONLY>
{
    fn future_annotations_or_stub(&self) -> bool {
        self.future_annotations
    }
    fn python_version(&self) -> ast::PythonVersion {
        self.python_version
    }
    fn source(&self) -> &str {
        self.source
    }

    fn lazy_import_context(&self) -> Option<LazyImportContext> {
        match self.scope().kind {
            ScopeKind::Function { .. } => Some(LazyImportContext::Function),
            ScopeKind::Class => Some(LazyImportContext::Class),
            _ => self
                .scope()
                .in_try
                .then_some(LazyImportContext::TryExceptBlocks),
        }
    }

    // Declaration ordering and enclosing-name resolution remain outside this AST-only
    // adapter, as in ty. These hooks do not claim to validate those restrictions.
    fn global(&self, _name: &str) -> Option<TextRange> {
        None
    }
    fn has_nonlocal_binding(&self, _name: &str) -> bool {
        true
    }

    fn in_async_context(&self) -> bool {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| match scope.kind {
                ScopeKind::Function { is_async, .. } => Some(is_async),
                ScopeKind::Module | ScopeKind::Class => Some(false),
                ScopeKind::Comprehension { .. } => None,
            })
            .unwrap_or(false)
    }

    fn in_await_allowed_context(&self) -> bool {
        self.in_async_context() || self.in_generator_context() || self.in_function_scope()
    }

    fn in_yield_allowed_context(&self) -> bool {
        self.in_function_scope()
    }

    fn in_sync_comprehension(&self) -> bool {
        for scope in self.scopes.iter().rev() {
            match scope.kind {
                ScopeKind::Comprehension {
                    is_async: false, ..
                } => return true,
                ScopeKind::Comprehension { is_async: true, .. } => {}
                _ => break,
            }
        }
        false
    }

    fn in_class_body_comprehension(&self) -> bool {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| match scope.kind {
                ScopeKind::Comprehension { .. } => None,
                ScopeKind::Class => Some(true),
                _ => Some(false),
            })
            .unwrap_or(false)
    }

    fn in_module_scope(&self) -> bool {
        matches!(self.scope().kind, ScopeKind::Module)
    }
    fn in_function_scope(&self) -> bool {
        matches!(self.scope().kind, ScopeKind::Function { .. })
    }
    fn in_generator_context(&self) -> bool {
        for scope in self.scopes.iter().rev() {
            match scope.kind {
                ScopeKind::Comprehension {
                    generator: true, ..
                } => return true,
                ScopeKind::Comprehension {
                    generator: false, ..
                } => {}
                _ => break,
            }
        }
        false
    }
    fn in_notebook(&self) -> bool {
        false
    }
    fn in_loop_context(&self) -> bool {
        self.scope().loop_depth > 0
    }
    fn is_bound_parameter(&self, name: &str) -> bool {
        matches!(self.scope().kind, ScopeKind::Function { parameters: Some(parameters), .. } if parameters.includes(name))
    }

    fn report_semantic_error(&self, error: SemanticSyntaxError) {
        use SemanticSyntaxErrorKind as Kind;
        if FUTURE_ONLY
            && !matches!(
                &error.kind,
                Kind::FutureFeatureNotDefined(_) | Kind::LateFutureImport | Kind::LazyFutureImport
            )
        {
            return;
        }
        if let Kind::InvalidExpression(kind, _) = &error.kind
            && self.escapes_annotation_scope(error.range, kind)
        {
            return;
        }
        let mut reported = self.error.borrow_mut();
        if reported.is_some() {
            return;
        }
        // Preserve the existing public diagnostics for checks previously performed in lowering.
        let message = match &error.kind {
            Kind::BreakOutsideLoop => "'break' outside loop".to_owned(),
            Kind::ContinueOutsideLoop => "'continue' not properly in loop".to_owned(),
            Kind::ReturnOutsideFunction => "'return' outside function".to_owned(),
            Kind::YieldOutsideFunction(kind) => format!("'{kind}' outside function"),
            Kind::YieldFromInAsyncFunction => "'yield from' inside async function".to_owned(),
            Kind::AwaitOutsideAsyncFunction(AwaitOutsideAsyncFunctionKind::Await) => {
                "'await' outside function".to_owned()
            }
            Kind::AwaitOutsideAsyncFunction(AwaitOutsideAsyncFunctionKind::AsyncFor) => {
                "'async for' outside async function".to_owned()
            }
            Kind::AwaitOutsideAsyncFunction(AwaitOutsideAsyncFunctionKind::AsyncWith) => {
                "'async with' outside async function".to_owned()
            }
            Kind::MultipleCaseAssignment(name) => {
                format!("multiple assignments to name '{name}' in pattern")
            }
            _ => error.to_string(),
        };
        *reported = Some(
            if matches!(
                error.kind,
                Kind::DifferentMatchPatternBindings
                    | Kind::MultipleCaseAssignment(_)
                    | Kind::MultipleStarredNamesInSequencePattern
                    | Kind::IrrefutableCasePattern(_)
                    | Kind::DuplicateMatchKey(_)
                    | Kind::DuplicateMatchClassAttribute(_)
            ) {
                CompileError::InvalidPattern {
                    message,
                    range: error.range,
                }
            } else {
                CompileError::invalid_syntax(message, error.range)
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::{validate, validate_without_statement_pruning};
    use crate::{CompileError, PythonVersion};

    const TARGETS: [PythonVersion; 4] = [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ];

    #[test]
    fn leaf_expressions_keep_debug_and_interpolated_expression_checks() {
        let valid = "ordinary = 1\ntext = 'left' 'right'\nraw = b'left' b'right'\nvalues = (ordinary, True, False, None, ..., 1.2, 3j, __debug__)\ndel ordinary\n";
        let parsed = parse_module(valid).unwrap();
        for python_version in TARGETS {
            validate(valid, &parsed.syntax().body, python_version).unwrap();
        }

        for (source, message) in [
            ("__debug__ = 1", "__debug__"),
            ("del __debug__", "__debug__"),
            ("(ordinary, __debug__) = (1, 2)", "__debug__"),
            ("value = f'{(__debug__ := 1)}'", "__debug__"),
            ("value = f'{(yield ordinary)}'", "yield"),
        ] {
            let parsed = parse_module(source).unwrap();
            for python_version in TARGETS {
                let error = validate(source, &parsed.syntax().body, python_version).unwrap_err();
                assert!(
                    error.to_string().contains(message),
                    "{python_version:?}: {source}: {error}",
                );
            }
        }
    }

    #[test]
    fn skipped_expression_wrappers_still_check_descendants_and_assignment_targets() {
        let valid = "left, right = (), ()\nvalues = (*left, *right), [*left, *right]\ndel (left, right)\nempty = lambda: __debug__\n";
        let parsed = parse_module(valid).unwrap();
        for python_version in TARGETS {
            validate(valid, &parsed.syntax().body, python_version).unwrap();
        }

        for (source, message) in [
            ("value = consume((yield item))", "yield"),
            ("value = (yield item).attribute", "yield"),
            ("value = not (other or 1 + (yield item))", "yield"),
            ("value = values[0:(yield item)]", "yield"),
            ("value = {'key': {item, (yield item)}}", "yield"),
            (
                "value = (bound := (yield item)) if flag else other",
                "yield",
            ),
            ("value = [(yield item) == other]", "yield"),
            ("value = consume((await item))", "await"),
            ("(*left, *right) = values", "starred"),
            ("[*left, *right] = values", "starred"),
            (
                "value = consume([item for item in (bound := values)])",
                "assignment expression",
            ),
        ] {
            let parsed = parse_module(source).unwrap();
            for python_version in TARGETS {
                let error = validate(source, &parsed.syntax().body, python_version).unwrap_err();
                assert!(
                    error.to_string().to_ascii_lowercase().contains(message),
                    "{python_version:?}: {source}: {error}",
                );
            }
        }
    }

    #[test]
    fn attribute_chains_keep_the_first_error_at_the_terminal_expression() {
        let valid = "normal.first.second.third = 1\ndel normal.first.second.third\nvalue = __debug__.first.second.third\nvalue = (lambda: 1).first.second.third\n";
        let parsed = parse_module(valid).unwrap();
        for python_version in TARGETS {
            validate(valid, &parsed.syntax().body, python_version).unwrap();
        }

        for (source, message, expected) in [
            (
                "value = ((yield first).a.b.c, (await second).d.e.f)",
                "'yield' outside function",
                "yield first",
            ),
            (
                "value = ((await first).a.b.c, (yield second).d.e.f)",
                "'await' outside function",
                "await first",
            ),
            (
                "value = ((__debug__ := first).a.b.c, (yield second).d.e.f)",
                "cannot assign to `__debug__`",
                "__debug__",
            ),
            (
                "value = ((yield first).a.b.c, (__debug__ := second).d.e.f)",
                "'yield' outside function",
                "yield first",
            ),
        ] {
            let parsed = parse_module(source).unwrap();
            for python_version in TARGETS {
                let error = validate(source, &parsed.syntax().body, python_version).unwrap_err();
                let CompileError::InvalidSyntax {
                    message: actual,
                    range,
                } = error
                else {
                    panic!("unexpected error for {source}: {error}");
                };
                assert_eq!(actual, message, "{python_version:?}: {source}");
                assert_eq!(
                    &source[usize::from(range.start())..usize::from(range.end())],
                    expected,
                    "{python_version:?}: {source}",
                );
            }
        }
    }

    #[test]
    fn pruned_statement_checks_match_the_standalone_checker_for_every_target() {
        for source in [
            "",
            "'module docstring'\nfrom __future__ import annotations\nfrom __future__ import division\nvalue = 1",
            "from __future__ import annotations\nfrom __future__ import division\nvalue = 1",
            "'first string'\n'second string'\nfrom __future__ import annotations",
            "from __future__ import annotations\n'not a docstring'\nfrom __future__ import division",
            "from .__future__ import annotations\nfrom __future__ import division",
            "'doc'\nfrom .__future__ import annotations\nfrom __future__ import division",
            "pass\nfrom __future__ import annotations",
            "if flag:\n 'nested string'\n from __future__ import annotations",
            "def ordinary():\n 'function docstring'\n from __future__ import annotations",
            "class Ordinary:\n 'class docstring'\n from __future__ import annotations",
            "lazy from __future__ import annotations\nfrom __future__ import division",
            "value = 0\nordinary = other\nordinary += 1\ndel ordinary\nassert True\nif flag: pass\nwhile False: pass",
            "value = 0\nordinary = (yield first)\n(__debug__ := second)",
            "value = 0\n__debug__ = other\n(yield second)",
            "value = 0\n*ordinary = other\n(yield second)",
            "value = 0\nordinary = *other\n(yield second)",
            "value = 0\n(*left, *right) = other\n(yield second)",
            "value = 0\nordinary += (yield first)\n(__debug__ := second)",
            "value = 0\ndel __debug__\n(yield second)",
            "value = 0\nassert (yield first)\n(__debug__ := second)",
            "value = 0\nif (yield first): pass\n(__debug__ := second)",
            "value = 0\nwhile (yield first): pass\n(__debug__ := second)",
            "value = 0\nraise (yield first)\n(__debug__ := second)",
            "value = 0\nwith context as __debug__: pass\n(yield second)",
            "value = 0\nasync with context: pass\n(yield second)",
            "value = 0\nfor __debug__ in other: pass\n(yield second)",
            "value = 0\nfor *ordinary in other: pass\n(yield second)",
            "value = 0\nfor ordinary in *other: pass\n(yield second)",
            "value = 0\nasync for ordinary in other: pass\n(yield second)",
            "value = 0\nreturn (yield first)\n(__debug__ := second)",
            "value = 0\nbreak\n(__debug__ := second)",
            "value = 0\ncontinue\n(__debug__ := second)",
            "def ordinary():\n for name in values:\n  if name: continue\n  break\n with context: pass\n return 1",
            "def ordinary():\n return *values\n (__debug__ := second)",
            "def ordinary():\n return (__debug__ := first)\n (__debug__ := second)",
            "def ordinary():\n while flag:\n  pass\n else:\n  break",
            "def ordinary():\n try: pass\n except* Error:\n  for value in values: break\n  return (__debug__ := first)",
            "async def ordinary():\n yield 1\n return (await first)",
            "def ordinary(value):\n pass\n global value\n (__debug__ := second)",
            "value = 0\ntry: pass\nexcept Error as __debug__: pass\n(yield second)",
        ] {
            let parsed = parse_module(source).unwrap_or_else(|error| panic!("{source}: {error}"));
            for python_version in TARGETS {
                let pruned = validate(source, &parsed.syntax().body, python_version);
                let reference = validate_without_statement_pruning(
                    source,
                    &parsed.syntax().body,
                    python_version,
                );
                assert_eq!(
                    format!("{pruned:?}"),
                    format!("{reference:?}"),
                    "{python_version:?}: {source}",
                );
            }
        }
    }

    #[test]
    fn non_mangling_parameters_keep_ordinary_duplicate_checks() {
        for source in [
            "class C:\n def f(__private): pass",
            "class C:\n def f(__dunder__, _C__dunder__, ordinary): pass",
            "class C:\n value = lambda __dunder__, _C__dunder__: None",
            "class ___:\n def f(__private, ____private, ordinary): pass",
            "class C[T]((lambda __private, _C__private: object)(0, 0)): pass",
        ] {
            let parsed = parse_module(source).unwrap();
            for python_version in TARGETS {
                validate(source, &parsed.syntax().body, python_version)
                    .unwrap_or_else(|error| panic!("{python_version:?}: {source}: {error}"));
            }
        }

        for source in [
            "class C:\n def f(ordinary, ordinary): pass",
            "class C:\n value = lambda ordinary, ordinary: None",
            "class C:\n def f(__dunder__, __dunder__): pass",
            "class ___:\n def f(__private, __private): pass",
        ] {
            let parsed = parse_module(source).unwrap();
            for python_version in TARGETS {
                let error = validate(source, &parsed.syntax().body, python_version).unwrap_err();
                assert!(
                    error.to_string().to_ascii_lowercase().contains("duplicate"),
                    "{python_version:?}: {source}: {error}",
                );
            }
        }
    }

    #[test]
    fn private_parameter_aliases_are_rejected_in_their_lexical_class() {
        for (source, duplicate) in [
            ("class C:\n def f(__x, _C__x): pass", "_C__x"),
            ("class C:\n def f(_C__x, __x): pass", "__x"),
            ("class C:\n def f(__x, /, _C__x): pass", "_C__x"),
            ("class C:\n def f(__x, *, _C__x): pass", "_C__x"),
            ("class C:\n def f(*__x, _C__x): pass", "__x"),
            ("class C:\n def f(*__x, **_C__x): pass", "_C__x"),
            ("class C:\n f = lambda __x, _C__x: None", "_C__x"),
            ("class C:\n def f():\n  def g(__x, _C__x): pass", "_C__x"),
            ("class C:\n def f[T](__x, _C__x): pass", "_C__x"),
            ("class __C:\n def f(__x, _C__x): pass", "_C__x"),
            ("class C:\n class D:\n  def f(__x, _D__x): pass", "_D__x"),
            (
                "class C:\n class D: pass\n def f(__x, _C__x): pass",
                "_C__x",
            ),
            (
                "class C:\n class D((lambda __x, _C__x: object)(0, 0)): pass",
                "_C__x",
            ),
            (
                "class C:\n @(lambda __x, _C__x: object)\n class D[T]: pass",
                "_C__x",
            ),
            (
                "class C[__x]((lambda __x, _C__x: object)(0, 0)): pass",
                "_C__x",
            ),
            (
                "class C[__x, T: (lambda __x, _C__x: object)]: pass",
                "_C__x",
            ),
        ] {
            let parsed = parse_module(source).unwrap();
            for python_version in TARGETS {
                let error = validate(source, &parsed.syntax().body, python_version).unwrap_err();
                let CompileError::InvalidSyntax { message, range } = error else {
                    panic!("unexpected error for {source}: {error}");
                };
                let kind = if python_version == PythonVersion::Py315 {
                    "parameter"
                } else {
                    "argument"
                };
                assert_eq!(
                    message,
                    format!("duplicate {kind} '{duplicate}' in function definition"),
                    "{python_version:?}: {source}",
                );
                assert_eq!(
                    &source[usize::from(range.start())..usize::from(range.end())],
                    duplicate,
                );
            }
        }
    }

    #[test]
    fn parameter_mangling_respects_class_headers_and_context_restoration() {
        for source in [
            "def f(__x, _C__x): pass",
            "class C: pass\ndef f(__x, _C__x): pass",
            "class ___:\n def f(__x, ____x): pass",
            "class C:\n class D:\n  def f(__x, _C__x): pass",
            "@(lambda __x, _C__x: object)\nclass C[T]: pass",
            "class C((lambda __x, _C__x: object)(0, 0)): pass",
            "class C[T]((lambda __x, _C__x: object)(0, 0)): pass",
            "class C[T: (lambda __x, _C__x: object), __x]: pass",
            "class C[__x: (lambda __y, _C__y: object)]: pass",
        ] {
            let parsed = parse_module(source).unwrap();
            for python_version in TARGETS {
                validate(source, &parsed.syntax().body, python_version)
                    .unwrap_or_else(|error| panic!("{python_version:?}: {source}: {error}"));
            }
        }
    }
}
