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

use super::statement::suite_contains_yield_in_function_scope;
use super::symbol::mangle_name;

/// Validate AST-based syntax restrictions using Ruff's shared checker.
pub(crate) fn validate(
    source: &str,
    body: &[Stmt],
    python_version: PythonVersion,
) -> Result<(), CompileError> {
    let mut validator = Validator {
        checker: SemanticSyntaxChecker::new(),
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
    };
    validator.visit_body(body);
    validator.error.into_inner().map_or(Ok(()), Err)
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

struct Validator<'source, 'ast> {
    checker: SemanticSyntaxChecker,
    error: RefCell<Option<CompileError>>,
    source: &'source str,
    body: &'ast [Stmt],
    python_version: ast::PythonVersion,
    future_annotations: bool,
    private_name: Option<&'ast str>,
    mangled_names: Option<FxHashSet<&'ast str>>,
    scopes: Vec<Scope<'ast>>,
}

impl<'ast> Validator<'_, 'ast> {
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
        if self.private_name.is_none() || self.error.borrow().is_some() {
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

impl<'ast> Visitor<'ast> for Validator<'_, 'ast> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if self.error.borrow().is_some() {
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
                    |visitor| visitor.visit_body(&function.body),
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
                                visitor.visit_body(&class.body);
                            });
                        },
                    );
                } else {
                    if let Some(arguments) = &class.arguments {
                        self.visit_arguments(arguments);
                    }
                    self.with_private_context(class.name.as_str(), None, |visitor| {
                        visitor.with_scope(ScopeKind::Class, |visitor| {
                            visitor.visit_body(&class.body);
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
        if self.error.borrow().is_some() {
            return;
        }
        self.with_checker(|checker, context| checker.visit_expr(expression, context));
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
        self.validate_private_parameters(parameters);
        walk_parameters(self, parameters);
    }
}

impl SemanticSyntaxContext for Validator<'_, '_> {
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

    use super::validate;
    use crate::{CompileError, PythonVersion};

    const TARGETS: [PythonVersion; 4] = [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ];

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
