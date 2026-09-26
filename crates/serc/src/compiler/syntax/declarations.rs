//! Checks the CPython symbol-table rules for `global` and `nonlocal` declarations.

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Pattern, Stmt};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;

use crate::{CompileError, PythonVersion};

use super::mangle_name;

pub(super) fn validate(body: &[Stmt], version: PythonVersion) -> Result<(), CompileError> {
    let future_annotations = body.iter().any(|statement| {
        matches!(statement, Stmt::ImportFrom(import) if import.level == 0
            && import.module.as_deref() == Some("__future__")
            && import.names.iter().any(|alias| alias.name.as_str() == "annotations"))
    });
    let mut validator = Validator {
        scopes: vec![Scope::new(ScopeKind::Module, None)],
        current: 0,
        private_name: None,
        version,
        future_annotations,
        error: None,
        stop_pattern_debug: None,
        stop_node: None,
        finding_stop: false,
        stopped: false,
    };
    validator.visit_body(body);
    if let Some(error) = validator.error {
        return Err(error);
    }
    validator.resolve()
}

/// Check symbol-collection declaration errors encountered before this pattern write.
/// Binding resolution happens after symbol collection, so it must not precede the write.
pub(super) fn immediate_before_pattern_debug(
    body: &[Stmt],
    version: PythonVersion,
    range: TextRange,
) -> Option<CompileError> {
    let future_annotations = body.iter().any(|statement| {
        matches!(statement, Stmt::ImportFrom(import) if import.level == 0
            && import.module.as_deref() == Some("__future__")
            && import.names.iter().any(|alias| alias.name.as_str() == "annotations"))
    });
    let mut validator = Validator {
        scopes: vec![Scope::new(ScopeKind::Module, None)],
        current: 0,
        private_name: None,
        version,
        future_annotations,
        error: None,
        stop_pattern_debug: Some(range),
        stop_node: None,
        finding_stop: false,
        stopped: false,
    };
    validator.visit_body(body);
    validator.error
}

/// Check declarations encountered before an earlier symbol-table error, without resolving
/// nonlocal bindings that CPython examines after collecting the entire symbol table.
pub(super) fn immediate_before_node(
    body: &[Stmt],
    version: PythonVersion,
    range: TextRange,
) -> Option<CompileError> {
    let future_annotations = body.iter().any(|statement| {
        matches!(statement, Stmt::ImportFrom(import) if import.level == 0
            && import.module.as_deref() == Some("__future__")
            && import.names.iter().any(|alias| alias.name.as_str() == "annotations"))
    });
    let new_validator = |finding_stop| Validator {
        scopes: vec![Scope::new(ScopeKind::Module, None)],
        current: 0,
        private_name: None,
        version,
        future_annotations,
        error: None,
        stop_pattern_debug: None,
        stop_node: Some(range),
        finding_stop,
        stopped: false,
    };
    // Some reported ranges refer to identifiers rather than visited statements or expressions.
    // Do not consider a declaration after such an unmatched range to precede the error.
    let mut finder = new_validator(true);
    finder.visit_body(body);
    if !finder.stopped {
        return None;
    }
    let mut validator = new_validator(false);
    validator.visit_body(body);
    validator.error
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ScopeKind {
    Module,
    Function,
    Class,
    Comprehension,
    Annotation,
    TypeParameters,
}

struct Scope {
    kind: ScopeKind,
    parent: Option<usize>,
    symbols: FxHashMap<String, Symbol>,
    // Keep declaration order stable independently of hash-map iteration and repeated statements.
    declarations: Vec<String>,
}

impl Scope {
    fn new(kind: ScopeKind, parent: Option<usize>) -> Self {
        Self {
            kind,
            parent,
            symbols: FxHashMap::default(),
            declarations: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Symbol {
    used: bool,
    assigned: bool,
    parameter: bool,
    annotated: bool,
    imported: bool,
    type_parameter: bool,
    global: bool,
    nonlocal: bool,
    first_declaration: Option<Declaration>,
}

struct Declaration {
    spelling: String,
    range: TextRange,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DeclarationKind {
    Global,
    Nonlocal,
}

impl DeclarationKind {
    fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Nonlocal => "nonlocal",
        }
    }
}

#[derive(Clone, Copy)]
enum NameUse {
    Read,
    Assign,
    Parameter,
    Import,
    TypeParameter,
}

struct Validator<'a> {
    scopes: Vec<Scope>,
    current: usize,
    private_name: Option<&'a str>,
    version: PythonVersion,
    future_annotations: bool,
    error: Option<CompileError>,
    stop_pattern_debug: Option<TextRange>,
    stop_node: Option<TextRange>,
    finding_stop: bool,
    stopped: bool,
}

impl<'a> Validator<'a> {
    fn name(&self, spelling: &str) -> String {
        mangle_name(spelling, self.private_name).into_owned()
    }

    fn record(&mut self, scope: usize, spelling: &str, usage: NameUse) {
        if self.stopped || self.error.is_some() {
            return;
        }
        let name = self.name(spelling);
        let symbol = self.scopes[scope].symbols.entry(name).or_default();
        match usage {
            NameUse::Read => symbol.used = true,
            NameUse::Assign => symbol.assigned = true,
            NameUse::Parameter => symbol.parameter = true,
            NameUse::Import => symbol.imported = true,
            NameUse::TypeParameter => symbol.type_parameter = true,
        }
    }

    fn enter(&mut self, kind: ScopeKind, visit: impl FnOnce(&mut Self)) {
        let parent = self.current;
        self.current = self.scopes.len();
        self.scopes.push(Scope::new(kind, Some(parent)));
        visit(self);
        self.current = parent;
    }

    fn reject(&mut self, message: String, range: TextRange) {
        if self.error.is_none() && !self.finding_stop {
            self.error = Some(CompileError::invalid_syntax(message, range));
        }
    }

    fn declaration(&mut self, name: &ast::Identifier, range: TextRange, kind: DeclarationKind) {
        let spelling = name.as_str();
        let canonical = self.name(spelling);
        let scope = &mut self.scopes[self.current];
        let symbol = scope.symbols.entry(canonical.clone()).or_default();
        let kind_name = kind.name();
        let error = if symbol.parameter {
            Some(format!("name '{spelling}' is parameter and {kind_name}"))
        } else if symbol.used {
            Some(format!(
                "name '{spelling}' is used prior to {kind_name} declaration"
            ))
        } else if symbol.annotated {
            Some(format!("annotated name '{spelling}' can't be {kind_name}"))
        } else if symbol.assigned {
            Some(format!(
                "name '{spelling}' is assigned to before {kind_name} declaration"
            ))
        } else {
            None
        };
        if symbol.first_declaration.is_none() {
            symbol.first_declaration = Some(Declaration {
                spelling: spelling.to_owned(),
                range,
            });
            scope.declarations.push(canonical);
        }
        match kind {
            DeclarationKind::Global => symbol.global = true,
            DeclarationKind::Nonlocal => symbol.nonlocal = true,
        }
        if let Some(error) = error {
            self.reject(error, range);
        }
    }

    fn annotated_name(&mut self, name: &ast::ExprName, range: TextRange) {
        let spelling = name.id.as_str();
        let canonical = self.name(spelling);
        let scope = &mut self.scopes[self.current];
        let symbol = scope.symbols.entry(canonical).or_default();
        let error = if scope.kind != ScopeKind::Module && symbol.global {
            Some(format!("annotated name '{spelling}' can't be global"))
        } else if symbol.nonlocal {
            Some(format!("annotated name '{spelling}' can't be nonlocal"))
        } else {
            None
        };
        symbol.annotated = true;
        symbol.assigned = true;
        if let Some(error) = error {
            self.reject(error, range);
        }
    }

    fn resolve(&self) -> Result<(), CompileError> {
        for scope in &self.scopes {
            for name in &scope.declarations {
                let symbol = &scope.symbols[name];
                let declaration = symbol.first_declaration.as_ref().unwrap();
                let spelling = &declaration.spelling;
                let error = if symbol.global && symbol.nonlocal {
                    Some(format!("name '{spelling}' is nonlocal and global"))
                } else if !symbol.nonlocal {
                    None
                } else if scope.kind == ScopeKind::Module {
                    Some("nonlocal declaration not allowed at module level".to_owned())
                } else {
                    self.nonlocal_error(scope.parent, name, spelling)
                };
                if let Some(error) = error {
                    return Err(CompileError::invalid_syntax(error, declaration.range));
                }
            }
        }
        Ok(())
    }

    fn nonlocal_error(
        &self,
        mut parent: Option<usize>,
        name: &str,
        spelling: &str,
    ) -> Option<String> {
        while let Some(index) = parent {
            let scope = &self.scopes[index];
            match scope.kind {
                ScopeKind::Module => break,
                ScopeKind::Class if name == "__class__" => return None,
                ScopeKind::Function | ScopeKind::TypeParameters => {
                    if let Some(symbol) = scope.symbols.get(name) {
                        if symbol.global {
                            break;
                        }
                        if symbol.type_parameter {
                            return Some(format!(
                                "nonlocal binding not allowed for type parameter '{spelling}'"
                            ));
                        }
                        if !symbol.nonlocal
                            && (symbol.assigned || symbol.parameter || symbol.imported)
                        {
                            return None;
                        }
                    }
                }
                ScopeKind::Class | ScopeKind::Comprehension | ScopeKind::Annotation => {}
            }
            parent = scope.parent;
        }
        Some(format!("no binding for nonlocal '{spelling}' found"))
    }

    fn defaults(&mut self, parameters: &'a ast::Parameters) {
        for default in parameters
            .iter_non_variadic_params()
            .filter_map(|parameter| parameter.default.as_deref())
        {
            self.visit_expr(default);
        }
    }

    fn parameters(&mut self, parameters: &'a ast::Parameters) {
        for parameter in parameters {
            self.record(self.current, parameter.name().as_str(), NameUse::Parameter);
        }
    }

    fn function_annotations(&mut self, function: &'a ast::StmtFunctionDef) {
        for parameter in &function.parameters {
            if let Some(annotation) = &parameter.as_parameter().annotation {
                self.visit_annotation(annotation);
            }
        }
        if let Some(returns) = &function.returns {
            self.visit_annotation(returns);
        }
    }

    fn type_parameters(&mut self, parameters: &'a ast::TypeParams) {
        for parameter in parameters {
            self.record(
                self.current,
                parameter.name().as_str(),
                NameUse::TypeParameter,
            );
            self.visit_type_param(parameter);
        }
    }

    fn comprehension(
        &mut self,
        generators: &'a [ast::Comprehension],
        result: impl FnOnce(&mut Self),
    ) {
        let Some((first, rest)) = generators.split_first() else {
            return;
        };
        self.visit_expr(&first.iter);
        self.enter(ScopeKind::Comprehension, |visitor| {
            visitor.visit_expr(&first.target);
            for condition in &first.ifs {
                visitor.visit_expr(condition);
            }
            for generator in rest {
                visitor.visit_comprehension(generator);
            }
            result(visitor);
        });
    }
}

impl<'a> Visitor<'a> for Validator<'a> {
    fn visit_stmt(&mut self, statement: &'a Stmt) {
        if self.error.is_some() || self.stopped {
            return;
        }
        if self.stop_node == Some(statement.range()) {
            self.stopped = true;
            return;
        }
        match statement {
            Stmt::Global(statement) => {
                for name in &statement.names {
                    self.declaration(name, statement.range, DeclarationKind::Global);
                }
            }
            Stmt::Nonlocal(statement) => {
                for name in &statement.names {
                    self.declaration(name, statement.range, DeclarationKind::Nonlocal);
                }
            }
            Stmt::FunctionDef(function) => {
                self.record(self.current, function.name.as_str(), NameUse::Assign);
                self.defaults(&function.parameters);
                for decorator in &function.decorator_list {
                    self.visit_decorator(decorator);
                }
                let visit_function = |visitor: &mut Self| {
                    visitor.function_annotations(function);
                    visitor.enter(ScopeKind::Function, |visitor| {
                        visitor.parameters(&function.parameters);
                        visitor.visit_body(&function.body);
                    });
                };
                if let Some(type_params) = &function.type_params {
                    self.enter(ScopeKind::TypeParameters, |visitor| {
                        visitor.type_parameters(type_params);
                        visit_function(visitor);
                    });
                } else {
                    visit_function(self);
                }
            }
            Stmt::ClassDef(class) => {
                self.record(self.current, class.name.as_str(), NameUse::Assign);
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(type_params) = &class.type_params {
                    let private_name = self.private_name.replace(class.name.as_str());
                    self.enter(ScopeKind::TypeParameters, |visitor| {
                        visitor.type_parameters(type_params);
                        if let Some(arguments) = &class.arguments {
                            visitor.visit_arguments(arguments);
                        }
                        visitor.enter(ScopeKind::Class, |visitor| visitor.visit_body(&class.body));
                    });
                    self.private_name = private_name;
                } else {
                    if let Some(arguments) = &class.arguments {
                        self.visit_arguments(arguments);
                    }
                    let private_name = self.private_name.replace(class.name.as_str());
                    self.enter(ScopeKind::Class, |visitor| visitor.visit_body(&class.body));
                    self.private_name = private_name;
                }
            }
            Stmt::TypeAlias(alias) => {
                self.visit_expr(&alias.name);
                let visit_alias = |visitor: &mut Self| {
                    visitor.enter(ScopeKind::Annotation, |visitor| {
                        visitor.visit_expr(&alias.value);
                    });
                };
                if let Some(parameters) = &alias.type_params {
                    self.enter(ScopeKind::TypeParameters, |visitor| {
                        visitor.type_parameters(parameters);
                        visit_alias(visitor);
                    });
                } else {
                    visit_alias(self);
                }
            }
            Stmt::AnnAssign(statement) => {
                if let Expr::Name(name) = statement.target.as_ref() {
                    if statement.simple {
                        self.annotated_name(name, statement.range);
                    } else if statement.value.is_some() {
                        self.visit_expr(&statement.target);
                    }
                } else {
                    self.visit_expr(&statement.target);
                }
                self.visit_annotation(&statement.annotation);
                if let Some(value) = &statement.value {
                    self.visit_expr(value);
                }
            }
            _ => walk_stmt(self, statement),
        }
    }

    fn visit_annotation(&mut self, expression: &'a Expr) {
        if self.future_annotations {
            return;
        }
        if self.version >= PythonVersion::Py314 {
            self.enter(ScopeKind::Annotation, |visitor| {
                visitor.visit_expr(expression);
            });
        } else {
            self.visit_expr(expression);
        }
    }

    fn visit_expr(&mut self, expression: &'a Expr) {
        if self.error.is_some() || self.stopped {
            return;
        }
        if self.stop_node == Some(expression.range()) {
            self.stopped = true;
            return;
        }
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_expr(expression));
        }

        match expression {
            Expr::Name(name) => {
                let usage = if name.ctx.is_load() {
                    NameUse::Read
                } else {
                    NameUse::Assign
                };
                self.record(self.current, name.id.as_str(), usage);
                if name.ctx.is_load()
                    && name.id.as_str() == "super"
                    && matches!(
                        self.scopes[self.current].kind,
                        ScopeKind::Function | ScopeKind::Comprehension
                    )
                {
                    self.record(self.current, "__class__", NameUse::Read);
                }
            }
            Expr::Named(named) => {
                self.visit_expr(&named.value);
                if let Expr::Name(name) = named.target.as_ref() {
                    let mut owner = self.current;
                    while self.scopes[owner].kind == ScopeKind::Comprehension {
                        owner = self.scopes[owner].parent.unwrap();
                    }
                    self.record(owner, name.id.as_str(), NameUse::Assign);
                } else {
                    self.visit_expr(&named.target);
                }
            }
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.defaults(parameters);
                }
                self.enter(ScopeKind::Function, |visitor| {
                    if let Some(parameters) = &lambda.parameters {
                        visitor.parameters(parameters);
                    }
                    visitor.visit_expr(&lambda.body);
                });
            }
            Expr::ListComp(comp) => {
                self.comprehension(&comp.generators, |visitor| visitor.visit_expr(&comp.elt));
            }
            Expr::SetComp(comp) => {
                self.comprehension(&comp.generators, |visitor| visitor.visit_expr(&comp.elt));
            }
            Expr::Generator(comp) => {
                self.comprehension(&comp.generators, |visitor| visitor.visit_expr(&comp.elt));
            }
            Expr::DictComp(comp) => self.comprehension(&comp.generators, |visitor| {
                if let Some(key) = &comp.key {
                    visitor.visit_expr(key);
                }
                visitor.visit_expr(&comp.value);
            }),
            _ => walk_expr(self, expression),
        }
    }

    fn visit_alias(&mut self, alias: &'a ast::Alias) {
        if self.stop_node == Some(alias.range) {
            self.stopped = true;
            return;
        }
        let name = alias.asname.as_ref().map_or_else(
            || alias.name.as_str().split('.').next().unwrap(),
            ast::Identifier::as_str,
        );
        if name != "*" {
            self.record(self.current, name, NameUse::Import);
        }
    }

    fn visit_except_handler(&mut self, handler: &'a ast::ExceptHandler) {
        let ast::ExceptHandler::ExceptHandler(handler) = handler;
        if let Some(exception) = &handler.type_ {
            self.visit_expr(exception);
        }
        if let Some(name) = &handler.name {
            self.record(self.current, name.as_str(), NameUse::Assign);
        }
        self.visit_body(&handler.body);
    }

    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        if self.error.is_some() || self.stopped {
            return;
        }
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_pattern(pattern));
        }
        if self.stop_pattern_debug.is_some()
            && let Pattern::MatchClass(class) = pattern
        {
            self.visit_expr(&class.cls);
            for child in &class.arguments.patterns {
                self.visit_pattern(child);
            }
            for keyword in &class.arguments.keywords {
                if keyword.attr.as_str() == "__debug__"
                    && self.stop_pattern_debug == Some(keyword.pattern.range())
                {
                    self.stopped = true;
                    return;
                }
            }
            for keyword in &class.arguments.keywords {
                self.visit_pattern(&keyword.pattern);
            }
            return;
        }
        walk_pattern(self, pattern);
        let name = match pattern {
            Pattern::MatchAs(pattern) => pattern.name.as_ref(),
            Pattern::MatchStar(pattern) => pattern.name.as_ref(),
            Pattern::MatchMapping(pattern) => pattern.rest.as_ref(),
            _ => None,
        };
        if let Some(name) = name {
            if name.as_str() == "__debug__" && self.stop_pattern_debug == Some(pattern.range()) {
                self.stopped = true;
                return;
            }
            self.record(self.current, name.as_str(), NameUse::Assign);
        }
    }
}
