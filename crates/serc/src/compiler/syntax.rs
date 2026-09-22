//! Source validation runs before lowering can discard unreachable statements.

use std::cell::RefCell;

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_python_parser::semantic_errors::{
    AwaitOutsideAsyncFunctionKind, InvalidExpressionKind, LazyImportContext, SemanticSyntaxChecker,
    SemanticSyntaxContext, SemanticSyntaxError, SemanticSyntaxErrorKind,
};
use ruff_text_size::{Ranged, TextRange};

use crate::{CompileError, PythonVersion};

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
    in_try: bool,
}

impl<'ast> Scope<'ast> {
    fn new(kind: ScopeKind<'ast>) -> Self {
        Self {
            kind,
            loop_depth: 0,
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
                    self.visit_type_params(params);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
                self.with_scope(ScopeKind::Class, |visitor| visitor.visit_body(&class.body));
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
            Stmt::Try(_) => {
                let previous = std::mem::replace(&mut self.scope_mut().in_try, true);
                walk_stmt(self, statement);
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
