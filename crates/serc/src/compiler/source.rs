use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::transformer::{Transformer, walk_expr as transform_expr};
use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{
    Expr, ExprContext, FStringPart, InterpolatedStringElement, Operator, Stmt, StringLiteralFlags,
    StringLiteralValue, Suite,
};
use ruff_python_codegen::{Generator, Indentation, Mode as CodegenMode};
use ruff_source_file::{LineEnding, LineIndex};
use ruff_text_size::{Ranged, TextLen, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::assembler::SourceLocation;
use crate::code_object::{
    CodeInterningSummary, CodeObject, CompilationInterningEvent, CompilationInterningEventKind,
    Constant,
};

use super::constant::{explicit_surrogate_string, fold_constant};
use super::scope::{ModulePlan, ScopeArena};
use super::symbol::mangle_name;
use super::{CachedSourceLine, Compiler};

#[derive(Debug)]
pub(super) struct CompilationContext<'source> {
    pub(super) python_version: crate::PythonVersion,
    pub(super) filename: String,
    pub(super) source: &'source str,
    pub(super) tokens: &'source Tokens,
    line_index: LineIndex,
    pub(super) scopes: ScopeArena,
    lowering_profile: LoweringProfile,
    constant_string_origins: FxHashMap<String, AtomicU8>,
    pub(super) generated_mangled_strings: FxHashSet<String>,
    // Generated names can first become canonical when an otherwise discarded child assembles.
    // Restricted generic-class identifiers also require real child lowering for key origins.
    pub(super) requires_interning_trace: bool,
    next_interning_order: AtomicUsize,
    interning_events: Mutex<Vec<CompilationInterningEvent>>,
}

/// Selects which target-specific layout rules may run while lowering a parsed module.
///
/// Ruff accepts some forward syntax that the pinned CPython target rejects. Keeping that case as
/// an explicit profile prevents a source feature from acting as an undocumented global switch
/// throughout the compiler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoweringProfile {
    ExactTarget,
    ForwardSyntax,
}

impl LoweringProfile {
    fn for_suite(body: &Suite, python_version: crate::PythonVersion) -> Self {
        if python_version == crate::PythonVersion::Py314 && suite_has_lazy_imports(body) {
            Self::ForwardSyntax
        } else {
            Self::ExactTarget
        }
    }

    const fn target_specific_rules_enabled(self) -> bool {
        matches!(self, Self::ExactTarget)
    }
}

pub(super) struct CallShape<'a> {
    pub(super) positional_count: usize,
    pub(super) keyword_count: usize,
    pub(super) has_starred: bool,
    pub(super) has_keyword_unpack: bool,
    pub(super) direct_method: Option<&'a ruff_python_ast::ExprAttribute>,
    pub(super) imported_module_callable: Option<&'a ruff_python_ast::ExprAttribute>,
}

impl<'source> CompilationContext<'source> {
    #[cfg(test)]
    pub(super) fn new(
        filename: &str,
        source: &'source str,
        tokens: &'source Tokens,
        body: &Suite,
    ) -> Self {
        Self::with_python_version(filename, source, tokens, body, crate::PythonVersion::Py314)
    }

    pub(super) fn with_python_version(
        filename: &str,
        source: &'source str,
        tokens: &'source Tokens,
        body: &Suite,
        python_version: crate::PythonVersion,
    ) -> Self {
        let scopes = ScopeArena::analyze(body);
        let mut generated_mangled_strings = FxHashSet::default();
        let mut requires_interning_trace = false;
        let constant_string_origins = parser_interned_strings(
            body,
            &mut generated_mangled_strings,
            &mut requires_interning_trace,
        );
        Self {
            python_version,
            filename: filename.to_string(),
            source,
            tokens,
            line_index: LineIndex::from_source_text(source),
            scopes,
            lowering_profile: LoweringProfile::for_suite(body, python_version),
            constant_string_origins,
            generated_mangled_strings,
            requires_interning_trace,
            next_interning_order: AtomicUsize::new(0),
            interning_events: Mutex::new(Vec::new()),
        }
    }

    pub(super) const fn target_specific_rules_enabled(&self) -> bool {
        self.lowering_profile.target_specific_rules_enabled()
    }

    pub(super) const fn module_plan(&self) -> &ModulePlan {
        self.scopes.module()
    }

    /// Records the first object admitted to the compilation-wide constant cache for an
    /// parser-interned name used as a constant. Later equal constants reuse that object's identity,
    /// even when the first load is removed by control-flow optimization.
    pub(super) fn record_constant_string_origin(&self, value: &str, interned: bool) {
        if let Some(origin) = self.constant_string_origins.get(value) {
            let _ = origin.compare_exchange(
                0,
                if interned { 2 } else { 1 },
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }

    pub(super) fn record_identifier_constant_origin(&self, raw: &str, mangled: &str) {
        // `_Py_Mangle` returns a fresh string when the spelling changes, even if an equal
        // identifier was already interned by the parser.
        self.record_constant_string_origin(mangled, raw == mangled);
    }

    pub(super) fn record_constant_string_origins(&self, constant: &Constant, interned: bool) {
        if self.constant_string_origins.is_empty() {
            return;
        }
        match constant {
            Constant::String(value) => self.record_constant_string_origin(value, interned),
            Constant::Tuple(values) | Constant::FrozenSet(values) => {
                if matches!(constant, Constant::Tuple(_)) {
                    self.record_cached_string_tuple(values);
                }
                for value in values {
                    self.record_constant_string_origins(value, interned);
                }
            }
            // A nested code object's constants were recorded while compiling its body.
            _ => {}
        }
    }

    pub(super) fn record_cached_string_tuple(&self, values: &[Constant]) {
        if self.generated_mangled_strings.is_empty() {
            return;
        }
        let Some(values) = values
            .iter()
            .map(|value| match value {
                Constant::String(value) => Some(value.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        if values
            .iter()
            .any(|value| self.generated_mangled_strings.contains(value))
        {
            self.record_interning_event(CompilationInterningEventKind::CachedTuple(values));
        }
    }

    fn record_interning_event(&self, kind: CompilationInterningEventKind) -> usize {
        let order = self.next_interning_order.fetch_add(1, Ordering::Relaxed);
        self.interning_events
            .lock()
            .unwrap()
            .push(CompilationInterningEvent { order, kind });
        order
    }

    pub(super) fn record_code_assembly(&self, code: &CodeObject) -> Option<usize> {
        if !self.requires_interning_trace {
            return None;
        }
        Some(
            self.record_interning_event(CompilationInterningEventKind::CodeAssembly(
                CodeInterningSummary::from_code(code),
            )),
        )
    }

    pub(super) fn interning_events(&self) -> Vec<CompilationInterningEvent> {
        self.interning_events.lock().unwrap().clone()
    }

    pub(super) fn has_tracked_string_origins(&self) -> bool {
        !self.constant_string_origins.is_empty()
    }

    /// Takes the completed module's string keys when no child still shares the context.
    pub(super) fn finish_constant_string_provenance(
        context: &mut Arc<Self>,
    ) -> (FxHashSet<String>, FxHashSet<String>) {
        let Some(context) = Arc::get_mut(context) else {
            return context.constant_string_provenance();
        };
        let origins = std::mem::take(&mut context.constant_string_origins);
        let mut identifiers = FxHashSet::default();
        identifiers.reserve(origins.len());
        let mut canonical = FxHashSet::default();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "Both provenance sets are only queried for membership."
        )]
        for (value, origin) in origins {
            let generated = context.generated_mangled_strings.contains(&value);
            if origin.into_inner() == 2 {
                if generated {
                    canonical.insert(value);
                } else {
                    canonical.insert(value.clone());
                    identifiers.insert(value);
                }
            } else if !generated {
                identifiers.insert(value);
            }
        }
        (identifiers, canonical)
    }

    fn constant_string_provenance(&self) -> (FxHashSet<String>, FxHashSet<String>) {
        let identifiers = self
            .constant_string_origins
            .keys()
            .filter(|value| !self.generated_mangled_strings.contains(*value))
            .cloned()
            .collect();
        let canonical = self
            .constant_string_origins
            .iter()
            .filter(|(_, origin)| origin.load(Ordering::Relaxed) == 2)
            .map(|(value, _)| value.clone())
            .collect();
        (identifiers, canonical)
    }
}

/// Tests for the ASCII dunder substring without constructing a generic string searcher.
fn has_double_underscore(name: &str) -> bool {
    name.as_bytes().windows(2).any(|pair| pair == b"__")
}

fn parser_interned_strings(
    body: &Suite,
    generated_mangled_strings: &mut FxHashSet<String>,
    requires_interning_trace: &mut bool,
) -> FxHashMap<String, AtomicU8> {
    #[derive(Default)]
    struct ConstantIdentifiers {
        origins: FxHashMap<String, AtomicU8>,
        private_name: Option<String>,
        mangled_names: Option<FxHashSet<String>>,
        generated_mangled_strings: FxHashSet<String>,
        raw_private_identifiers: FxHashSet<String>,
        restricted_private_identifier: bool,
    }

    impl ConstantIdentifiers {
        fn mangled_name<'a>(&self, name: &'a str) -> std::borrow::Cow<'a, str> {
            if self
                .mangled_names
                .as_ref()
                .is_some_and(|names| !names.contains(name))
            {
                std::borrow::Cow::Borrowed(name)
            } else {
                mangle_name(name, self.private_name.as_deref())
            }
        }

        fn record_name(&mut self, name: &str) {
            self.record_raw_name(name);
            let mangled = self.mangled_name(name);
            if mangled != name {
                let mangled = mangled.into_owned();
                self.generated_mangled_strings.insert(mangled.clone());
                self.origins.entry(mangled).or_default();
            }
        }

        fn record_raw_name(&mut self, name: &str) {
            if !self.origins.contains_key(name) {
                self.origins.insert(name.to_string(), AtomicU8::default());
            }
            if has_double_underscore(name) {
                self.restricted_private_identifier |= self.mangled_names.is_some();
                self.raw_private_identifiers.insert(name.to_string());
                let mangled = self.mangled_name(name);
                if mangled != name {
                    self.generated_mangled_strings.insert(mangled.into_owned());
                }
            }
        }
    }

    impl<'a> Visitor<'a> for ConstantIdentifiers {
        fn visit_stmt(&mut self, statement: &'a Stmt) {
            match statement {
                Stmt::AnnAssign(assignment) => {
                    if let Expr::Name(target) = assignment.target.as_ref() {
                        self.record_name(target.id.as_str());
                    }
                }
                Stmt::FunctionDef(definition) => {
                    self.record_raw_name(definition.name.as_str());
                    if definition.returns.is_some() {
                        self.origins.entry("return".to_string()).or_default();
                    }
                }
                Stmt::ClassDef(definition) => {
                    self.record_raw_name(definition.name.as_str());
                    for decorator in &definition.decorator_list {
                        self.visit_decorator(decorator);
                    }
                    let previous = self.private_name.clone();
                    let previous_mangled_names = self.mangled_names.clone();
                    if let Some(type_params) = &definition.type_params {
                        self.private_name = Some(definition.name.to_string());
                        // `_Py_MaybeMangle` restricts generic-class annotation scopes to
                        // declared type parameters. The class body has ordinary mangling.
                        self.mangled_names = Some(
                            type_params
                                .iter()
                                .map(|parameter| parameter.name().to_string())
                                .collect(),
                        );
                        self.visit_type_params(type_params);
                    }
                    if let Some(arguments) = &definition.arguments {
                        self.visit_arguments(arguments);
                    }
                    self.private_name = Some(definition.name.to_string());
                    self.mangled_names = None;
                    self.visit_body(&definition.body);
                    self.private_name = previous;
                    self.mangled_names = previous_mangled_names;
                    return;
                }
                Stmt::ImportFrom(import) => {
                    if let Some(module) = &import.module {
                        for component in module.as_str().split('.') {
                            self.record_raw_name(component);
                        }
                    }
                }
                Stmt::Global(declaration) => {
                    for name in &declaration.names {
                        self.record_raw_name(name.as_str());
                    }
                }
                Stmt::Nonlocal(declaration) => {
                    for name in &declaration.names {
                        self.record_raw_name(name.as_str());
                    }
                }
                Stmt::Match(_) => {
                    // The parser interns soft keywords through its identifier-token path.
                    self.origins.entry("case".to_string()).or_default();
                }
                Stmt::TypeAlias(alias) => {
                    if let Expr::Name(name) = alias.name.as_ref() {
                        self.origins.entry(name.id.to_string()).or_default();
                    }
                }
                _ => {}
            }
            walk_stmt(self, statement);
        }

        fn visit_parameters(&mut self, parameters: &'a ruff_python_ast::Parameters) {
            for parameter in &parameters.kwonlyargs {
                if parameter.default.is_some() {
                    self.record_name(parameter.name().as_str());
                }
            }
            ruff_python_ast::visitor::walk_parameters(self, parameters);
        }

        fn visit_parameter(&mut self, parameter: &'a ruff_python_ast::Parameter) {
            self.record_raw_name(parameter.name.as_str());
            if let Some(annotation) = &parameter.annotation {
                self.record_name(parameter.name.as_str());
                self.visit_expr(annotation);
            }
        }

        fn visit_expr(&mut self, expression: &'a Expr) {
            match expression {
                Expr::Name(name) => self.record_raw_name(name.id.as_str()),
                Expr::Attribute(attribute) => self.record_raw_name(attribute.attr.as_str()),
                _ => {}
            }
            walk_expr(self, expression);
        }

        fn visit_except_handler(&mut self, handler: &'a ruff_python_ast::ExceptHandler) {
            let ruff_python_ast::ExceptHandler::ExceptHandler(handler) = handler;
            if let Some(name) = &handler.name {
                self.record_raw_name(name.as_str());
            }
            if let Some(type_) = &handler.type_ {
                self.visit_expr(type_);
            }
            self.visit_body(&handler.body);
        }

        fn visit_pattern(&mut self, pattern: &'a ruff_python_ast::Pattern) {
            let capture = match pattern {
                ruff_python_ast::Pattern::MatchAs(pattern) => pattern.name.as_ref(),
                ruff_python_ast::Pattern::MatchStar(pattern) => pattern.name.as_ref(),
                ruff_python_ast::Pattern::MatchMapping(pattern) => pattern.rest.as_ref(),
                _ => None,
            };
            if let Some(name) = capture {
                self.record_raw_name(name.as_str());
            }
            ruff_python_ast::visitor::walk_pattern(self, pattern);
        }

        fn visit_type_param(&mut self, parameter: &'a ruff_python_ast::TypeParam) {
            let name = parameter.name().as_str();
            if let Some(names) = &mut self.mangled_names {
                names.insert(name.to_string());
            }
            self.record_raw_name(name);
            ruff_python_ast::visitor::walk_type_param(self, parameter);
        }

        fn visit_keyword(&mut self, keyword: &'a ruff_python_ast::Keyword) {
            if let Some(name) = &keyword.arg {
                self.record_raw_name(name.as_str());
            }
            self.visit_expr(&keyword.value);
        }

        fn visit_alias(&mut self, alias: &'a ruff_python_ast::Alias) {
            self.origins.entry(alias.name.to_string()).or_default();
            for component in alias.name.as_str().split('.') {
                self.record_raw_name(component);
            }
            if let Some(name) = &alias.asname {
                self.record_raw_name(name.as_str());
            }
        }

        fn visit_pattern_keyword(&mut self, keyword: &'a ruff_python_ast::PatternKeyword) {
            self.record_raw_name(keyword.attr.as_str());
            self.visit_pattern(&keyword.pattern);
        }
    }

    let mut visitor = ConstantIdentifiers::default();
    for statement in body {
        visitor.visit_stmt(statement);
    }
    visitor
        .generated_mangled_strings
        .retain(|name| !visitor.raw_private_identifiers.contains(name));
    *requires_interning_trace =
        visitor.restricted_private_identifier || !visitor.generated_mangled_strings.is_empty();
    *generated_mangled_strings = visitor.generated_mangled_strings;
    visitor.origins
}

fn suite_has_lazy_imports(body: &Suite) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::Import(import) => import.is_lazy,
        Stmt::ImportFrom(import) => import.is_lazy,
        Stmt::If(statement) => {
            suite_has_lazy_imports(&statement.body)
                || statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| suite_has_lazy_imports(&clause.body))
        }
        Stmt::For(statement) => {
            suite_has_lazy_imports(&statement.body) || suite_has_lazy_imports(&statement.orelse)
        }
        Stmt::While(statement) => {
            suite_has_lazy_imports(&statement.body) || suite_has_lazy_imports(&statement.orelse)
        }
        Stmt::With(statement) => suite_has_lazy_imports(&statement.body),
        Stmt::Match(statement) => statement
            .cases
            .iter()
            .any(|case| suite_has_lazy_imports(&case.body)),
        _ => false,
    })
}

impl Compiler<'_> {
    /// Apply 3.15's lazy-import scope checks before lowering can discard dead suites.
    pub(crate) fn validate_lazy_imports(
        body: &Suite,
        python_version: crate::PythonVersion,
    ) -> Result<(), crate::CompileError> {
        if python_version != crate::PythonVersion::Py315 {
            return Ok(());
        }

        #[derive(Default)]
        struct Validator {
            scope: Option<&'static str>,
            in_try: bool,
            error: Option<crate::CompileError>,
        }

        impl Validator {
            fn validate(&mut self, statement: &Stmt, import_type: &str) {
                let context = if self.in_try {
                    Some("try/except blocks")
                } else {
                    self.scope
                };
                if let Some(context) = context {
                    self.error = Some(crate::CompileError::invalid_syntax(
                        format!("lazy {import_type} not allowed inside {context}"),
                        statement.range(),
                    ));
                }
            }
        }

        impl<'a> Visitor<'a> for Validator {
            fn visit_expr(&mut self, _expression: &'a Expr) {
                // Import statements cannot occur inside expressions.
            }

            fn visit_stmt(&mut self, statement: &'a Stmt) {
                if self.error.is_some() {
                    return;
                }
                match statement {
                    Stmt::Import(import) if import.is_lazy => self.validate(statement, "import"),
                    Stmt::ImportFrom(import) if import.is_lazy => {
                        if import.level == 0
                            && import
                                .module
                                .as_ref()
                                .is_some_and(|module| module.as_str() == "__future__")
                        {
                            self.error = Some(crate::CompileError::invalid_syntax(
                                "lazy from __future__ import is not allowed",
                                statement.range(),
                            ));
                            return;
                        }
                        self.validate(statement, "from ... import");
                        if self.error.is_none()
                            && import.names.iter().any(|alias| alias.name.as_str() == "*")
                        {
                            self.error = Some(crate::CompileError::invalid_syntax(
                                "lazy from ... import * is not allowed",
                                statement.range(),
                            ));
                        }
                    }
                    Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                        let previous_scope = self.scope;
                        let previous_try = self.in_try;
                        self.scope = Some(if matches!(statement, Stmt::FunctionDef(_)) {
                            "functions"
                        } else {
                            "classes"
                        });
                        self.in_try = false;
                        walk_stmt(self, statement);
                        self.scope = previous_scope;
                        self.in_try = previous_try;
                    }
                    Stmt::Try(_) => {
                        let previous = self.in_try;
                        self.in_try = true;
                        walk_stmt(self, statement);
                        self.in_try = previous;
                    }
                    _ => walk_stmt(self, statement),
                }
            }
        }

        let mut validator = Validator::default();
        for statement in body {
            validator.visit_stmt(statement);
        }
        validator.error.map_or(Ok(()), Err)
    }

    pub(super) fn line_number(&self, offset: u32) -> u32 {
        self.source_line(self.source_offset(offset)).number
    }

    pub(super) fn source_location(&self, range: ruff_text_size::TextRange) -> SourceLocation {
        let start = self.source_offset(u32::from(range.start()));
        let end = self.source_offset(u32::from(range.end()));
        let source_line = self.source_line(start);
        let line = i32::try_from(source_line.number).unwrap_or(i32::MAX);
        let column = i32::try_from(u32::from(start - source_line.start)).unwrap_or(i32::MAX);

        if source_line.contains(end) {
            let end_column = i32::try_from(u32::from(end - source_line.start)).unwrap_or(i32::MAX);
            return SourceLocation::new(line, line, column, end_column);
        }

        let (end_line, end_column) = self.source_position(u32::from(end));
        SourceLocation::new(line, end_line, column, end_column)
    }

    pub(super) fn source_location_including_trailing_semicolon(
        &self,
        range: ruff_text_size::TextRange,
    ) -> SourceLocation {
        let (line, column) = self.source_position(u32::from(range.start()));
        let (end_line, end_column) = self.definition_end_position(range);
        SourceLocation::new(line, end_line, column, end_column)
    }

    pub(super) fn definition_location(
        &self,
        range: ruff_text_size::TextRange,
        name_range: ruff_text_size::TextRange,
        keyword: TokenKind,
        is_async: bool,
    ) -> SourceLocation {
        let tokens = self.context.tokens.in_range(ruff_text_size::TextRange::new(
            range.start(),
            name_range.start(),
        ));
        let start = tokens
            .iter()
            .enumerate()
            .rfind(|(_, token)| token.kind() == keyword)
            .map_or(range.start(), |(keyword_index, keyword)| {
                if is_async {
                    tokens[..keyword_index]
                        .iter()
                        .rfind(|token| token.kind() == TokenKind::Async)
                        .map_or(keyword.start(), ruff_text_size::Ranged::start)
                } else {
                    keyword.start()
                }
            });
        let (line, column) = self.source_position(u32::from(start));
        let (end_line, end_column) = self.definition_end_position(range);
        SourceLocation::new(line, end_line, column, end_column)
    }

    fn definition_end_position(&self, range: ruff_text_size::TextRange) -> (i32, i32) {
        let end = self
            .context
            .tokens
            .after(range.end())
            .first()
            .filter(|token| token.kind() == TokenKind::Semi)
            .map_or(range.end(), ruff_text_size::Ranged::end);
        self.source_position(u32::from(end))
    }

    pub(super) fn source_position(&self, offset: u32) -> (i32, i32) {
        let offset = self.source_offset(offset);
        let line = self.source_line(offset);
        (
            i32::try_from(line.number).unwrap_or(i32::MAX),
            i32::try_from(u32::from(offset - line.start)).unwrap_or(i32::MAX),
        )
    }

    fn source_line(&self, offset: TextSize) -> CachedSourceLine {
        if let Some(line) = self.cached_source_line.get()
            && line.contains(offset)
        {
            return line;
        }

        let line = self.context.line_index.line_index(offset);
        let line_index = line.to_zero_indexed();
        let line_starts = self.context.line_index.line_starts();
        let start = line_starts[line_index];
        let end = line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or_else(|| self.source_offset(u32::MAX));
        let cached = CachedSourceLine {
            number: u32::try_from(line.get()).unwrap_or(u32::MAX),
            start,
            end,
            is_last: line_index + 1 == line_starts.len(),
        };
        self.cached_source_line.set(Some(cached));
        cached
    }

    fn source_offset(&self, offset: u32) -> TextSize {
        TextSize::new(offset.min(u32::try_from(self.context.source.len()).unwrap_or(u32::MAX)))
    }

    fn source_text(&self, range: ruff_text_size::TextRange) -> &str {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        &self.context.source[start..end]
    }

    pub(super) fn string_literal_constant(
        &self,
        string: &ruff_python_ast::ExprStringLiteral,
    ) -> Constant {
        if string
            .value
            .iter()
            .all(|part| !part.as_str().contains('\u{fffd}'))
        {
            return Constant::String(string.value.to_str().to_string());
        }

        let mut bytes = Vec::new();
        let mut has_surrogates = false;
        for part in &string.value {
            if part.flags.prefix().is_raw() {
                bytes.extend_from_slice(part.as_str().as_bytes());
                continue;
            }
            if let Some(part_bytes) =
                explicit_surrogate_string(part.as_str(), self.source_text(part.range))
            {
                bytes.extend(part_bytes);
                has_surrogates = true;
            } else {
                bytes.extend_from_slice(part.as_str().as_bytes());
            }
        }
        if has_surrogates {
            Constant::SurrogateString(bytes)
        } else {
            Constant::String(string.value.to_str().to_string())
        }
    }

    /// Folds constants while preserving surrogate escapes that Ruff represents as U+FFFD.
    pub(super) fn source_aware_fold_constant(&self, expression: &Expr) -> Option<Constant> {
        match expression {
            Expr::StringLiteral(string) => Some(self.string_literal_constant(string)),
            Expr::Tuple(tuple) if tuple.ctx == ExprContext::Load => tuple
                .elts
                .iter()
                .map(|element| self.source_aware_fold_constant(element))
                .collect::<Option<Vec<_>>>()
                .map(Constant::Tuple),
            Expr::BinOp(binary) if matches!(binary.op, Operator::Add | Operator::Mult) => {
                let folded = fold_constant(expression)?;
                if !matches!(&folded, Constant::String(value) if value.contains('\u{fffd}')) {
                    return Some(folded);
                }
                let left = self.source_aware_fold_constant(&binary.left)?;
                let right = self.source_aware_fold_constant(&binary.right)?;
                match (left, binary.op, right) {
                    (
                        Constant::SurrogateString(mut left),
                        Operator::Add,
                        Constant::String(right),
                    ) => {
                        left.extend_from_slice(right.as_bytes());
                        Some(Constant::SurrogateString(left))
                    }
                    (Constant::String(left), Operator::Add, Constant::SurrogateString(right)) => {
                        let mut bytes = left.into_bytes();
                        bytes.extend(right);
                        Some(Constant::SurrogateString(bytes))
                    }
                    (
                        Constant::SurrogateString(mut left),
                        Operator::Add,
                        Constant::SurrogateString(right),
                    ) => {
                        left.extend(right);
                        Some(Constant::SurrogateString(left))
                    }
                    (Constant::SurrogateString(value), Operator::Mult, Constant::Int(times))
                    | (Constant::Int(times), Operator::Mult, Constant::SurrogateString(value)) => {
                        let length = value.iter().filter(|byte| **byte & 0xc0 != 0x80).count();
                        let max_times = 4096_usize.checked_div(length).unwrap_or(usize::MAX);
                        let times = usize::try_from(times).ok()?;
                        (times <= max_times).then(|| Constant::SurrogateString(value.repeat(times)))
                    }
                    _ => Some(folded),
                }
            }
            _ => fold_constant(expression),
        }
    }

    pub(super) fn generator_expression_range(
        &self,
        generator: &ruff_python_ast::ExprGenerator,
    ) -> ruff_text_size::TextRange {
        if generator.parenthesized {
            return generator.range;
        }

        ruff_python_ast::token::parentheses_iterator(generator.into(), None, self.context.tokens)
            .next()
            .unwrap_or(generator.range)
    }

    pub(super) fn debug_expression_source_range(
        &self,
        interpolation: &ruff_python_ast::InterpolatedElement,
    ) -> ruff_text_size::TextRange {
        assert!(
            interpolation.debug_text.is_some(),
            "debug expression range requires debug text"
        );
        let tokens = self.context.tokens.in_range(interpolation.range);
        let opening = tokens
            .first()
            .filter(|token| token.kind() == TokenKind::Lbrace)
            .expect("interpolation should start with an opening brace");
        let marker = tokens
            .iter()
            .find(|token| {
                token.start() >= interpolation.expression.range().end()
                    && token.kind() == TokenKind::Equal
            })
            .expect("debug interpolation should contain an equals token");
        ruff_text_size::TextRange::new(opening.end(), marker.start())
    }

    pub(super) fn fstring_result_range(
        fstring: &ruff_python_ast::ExprFString,
    ) -> ruff_text_size::TextRange {
        let mut component_count = 0;
        let mut pending_literal = false;
        let mut only_interpolation = None;
        for part in &fstring.value {
            match part {
                FStringPart::Literal(literal) => {
                    pending_literal |= !literal.value.is_empty();
                }
                FStringPart::FString(fstring) => {
                    for element in &fstring.elements {
                        match element {
                            InterpolatedStringElement::Literal(literal) => {
                                pending_literal |= !literal.value.is_empty();
                            }
                            InterpolatedStringElement::Interpolation(interpolation) => {
                                if interpolation.debug_text.is_some() {
                                    pending_literal = true;
                                }
                                if std::mem::take(&mut pending_literal) {
                                    component_count += 1;
                                }
                                component_count += 1;
                                only_interpolation = Some(interpolation.range);
                            }
                        }
                    }
                }
            }
        }
        if pending_literal {
            component_count += 1;
            only_interpolation = None;
        }
        if component_count == 1 {
            only_interpolation.unwrap_or_else(|| {
                fstring_literal_span(fstring)
                    .unwrap_or_else(|| interpolated_string_content_range(fstring.into()))
            })
        } else {
            fstring.range
        }
    }

    pub(super) fn attribute_opcode_range(
        &self,
        attribute: &ruff_python_ast::ExprAttribute,
    ) -> ruff_text_size::TextRange {
        let location = self.source_location(attribute.range);
        if location.line == location.end_line {
            attribute.range
        } else {
            attribute.attr.range()
        }
    }

    pub(super) fn attribute_opcode_location(
        &self,
        attribute: &ruff_python_ast::ExprAttribute,
    ) -> SourceLocation {
        let location = self.source_location(attribute.range);
        if location.line == location.end_line {
            location
        } else {
            self.source_location(attribute.attr.range())
        }
    }

    pub(super) fn call_opcode_location(&self, call: &ruff_python_ast::ExprCall) -> SourceLocation {
        let Expr::Attribute(attribute) = call.func.as_ref() else {
            return self.source_location(call.range);
        };
        let attribute_location = self.source_location(attribute.range);
        if attribute_location.line == attribute_location.end_line {
            return self.source_location(call.range);
        }
        if self.call_shape(call).direct_method.is_some() {
            self.source_location(ruff_text_size::TextRange::new(
                attribute.attr.range().start(),
                call.range.end(),
            ))
        } else {
            self.source_location(call.range)
        }
    }

    pub(super) fn call_opcode_location_with_method(
        &self,
        call: &ruff_python_ast::ExprCall,
        direct_method: Option<&ruff_python_ast::ExprAttribute>,
    ) -> SourceLocation {
        if let Some(attribute) = direct_method {
            let attribute_location = self.source_location(attribute.range);
            if attribute_location.line != attribute_location.end_line {
                return self.source_location(ruff_text_size::TextRange::new(
                    attribute.attr.range().start(),
                    call.range.end(),
                ));
            }
        }
        self.source_location(call.range)
    }

    pub(super) fn call_shape<'a>(&self, call: &'a ruff_python_ast::ExprCall) -> CallShape<'a> {
        let positional_count = call.arguments.args.len();
        let keyword_count = call.arguments.keywords.len();
        let has_starred = call.arguments.args.iter().any(Expr::is_starred_expr);
        let has_keyword_unpack = call
            .arguments
            .keywords
            .iter()
            .any(|keyword| keyword.arg.is_none());
        let (direct_method, imported_module_callable) = if !has_starred
            && !has_keyword_unpack
            && positional_count + keyword_count + usize::from(keyword_count > 0) < 30
            && let Expr::Attribute(attribute) = call.func.as_ref()
        {
            if self.imported_module_attribute(attribute) {
                (None, Some(attribute))
            } else {
                (Some(attribute), None)
            }
        } else {
            (None, None)
        };
        CallShape {
            positional_count,
            keyword_count,
            has_starred,
            has_keyword_unpack,
            direct_method,
            imported_module_callable,
        }
    }
}

pub(super) fn interpolated_string_content_range(
    string: ruff_python_ast::StringLike<'_>,
) -> ruff_text_size::TextRange {
    debug_assert!(string.is_interpolated_string());
    let mut parts = string.parts();
    let Some(first) = parts.next() else {
        return string.range();
    };
    let last = parts.next_back().unwrap_or(first);
    ruff_text_size::TextRange::new(first.content_range().start(), last.content_range().end())
}

pub(super) fn debug_text_range(
    interpolation: &ruff_python_ast::InterpolatedElement,
) -> ruff_text_size::TextRange {
    let debug = interpolation
        .debug_text
        .as_ref()
        .expect("debug text range requires debug text");
    ruff_text_size::TextRange::at(
        interpolation.range.start() + "{".text_len(),
        debug.as_str().text_len(),
    )
}

fn fstring_literal_span(
    fstring: &ruff_python_ast::ExprFString,
) -> Option<ruff_text_size::TextRange> {
    let mut first = None;
    let mut last = None;
    let mut include = |range: ruff_text_size::TextRange| {
        first.get_or_insert(range.start());
        last = Some(range.end());
    };
    for part in &fstring.value {
        match part {
            FStringPart::Literal(literal) => include(literal.range),
            FStringPart::FString(fstring) => {
                for element in &fstring.elements {
                    match element {
                        InterpolatedStringElement::Literal(literal) => include(literal.range),
                        InterpolatedStringElement::Interpolation(_) => return None,
                    }
                }
            }
        }
    }
    Some(ruff_text_size::TextRange::new(first?, last?))
}

/// Removes expression-level Python comments while preserving every other byte,
/// as CPython does for the source text stored by a t-string interpolation.
pub(super) fn strip_expression_comments(source: &str) -> String {
    let parsed = ruff_python_parser::parse_unchecked(
        source,
        ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::ParenthesizedExpression),
    );
    let mut output = String::with_capacity(source.len());
    let mut copied = 0;
    // Ruff tokenizes comments inside nested interpolated strings. They are part of the nested
    // string's source from the outer expression's perspective, so preserve them here.
    let mut interpolated_string_depth = 0_u32;
    for token in parsed.tokens() {
        match token.kind() {
            TokenKind::FStringStart | TokenKind::TStringStart => {
                interpolated_string_depth += 1;
            }
            TokenKind::FStringEnd | TokenKind::TStringEnd => {
                interpolated_string_depth = interpolated_string_depth.saturating_sub(1);
            }
            TokenKind::Comment if interpolated_string_depth == 0 => {
                let start = usize::from(token.start());
                let end = usize::from(token.end());
                output.push_str(&source[copied..start]);
                copied = end;
            }
            _ => {}
        }
    }
    output.push_str(&source[copied..]);
    output
}
pub(super) fn implicit_return_range(
    compiler: &Compiler<'_>,
    statement: &Stmt,
) -> ruff_text_size::TextRange {
    match statement {
        Stmt::Assign(assignment) => assignment
            .targets
            .last()
            .map_or(statement.range(), |target| {
                last_store_target_range(compiler, target)
            }),
        Stmt::Expr(expression) => match expression.value.as_ref() {
            Expr::FString(fstring) => Compiler::fstring_result_range(fstring),
            expression => expression.range(),
        },
        Stmt::AnnAssign(assignment) => {
            let Expr::Subscript(subscript) = assignment.target.as_ref() else {
                return statement.range();
            };
            last_discarded_annotation_slice_range(&subscript.slice)
        }
        Stmt::While(statement) => super::statement::while_condition_exit_range(&statement.test),
        Stmt::For(statement) => statement.iter.range(),
        _ => statement.range(),
    }
}

fn condition_exit_expression(mut expression: &Expr) -> &Expr {
    while let Expr::UnaryOp(unary) = expression
        && unary.op == ruff_python_ast::UnaryOp::Not
    {
        expression = &unary.operand;
    }
    expression
}

pub(super) fn condition_exit_range(expression: &Expr) -> ruff_text_size::TextRange {
    condition_exit_expression(expression).range()
}

pub(super) fn unary_not_condition_exit_range(
    expression: &Expr,
) -> Option<ruff_text_size::TextRange> {
    let Expr::UnaryOp(unary) = expression else {
        return None;
    };
    if unary.op != ruff_python_ast::UnaryOp::Not {
        return None;
    }

    let operand = condition_exit_expression(&unary.operand);
    if matches!(operand, Expr::BoolOp(_) | Expr::If(_)) {
        return None;
    }
    Some(operand.range())
}

pub(super) fn synthetic_condition_exit_range(expression: &Expr) -> ruff_text_size::TextRange {
    // `codegen_jump_if` strips `not`, so direct operands own synthetic exits. Boolean and
    // conditional operands choose an edge-specific child; preserve their previous whole-condition
    // owner until the caller retains enough jump-polarity information to select that child.
    unary_not_condition_exit_range(expression).unwrap_or(expression.range())
}

fn last_store_target_range(
    compiler: &Compiler<'_>,
    expression: &Expr,
) -> ruff_text_size::TextRange {
    match expression {
        Expr::List(list) => list.elts.last().map_or(expression.range(), |target| {
            last_store_target_range(compiler, target)
        }),
        Expr::Tuple(tuple) => tuple.elts.last().map_or(expression.range(), |target| {
            last_store_target_range(compiler, target)
        }),
        Expr::Starred(starred) => last_store_target_range(compiler, &starred.value),
        Expr::Attribute(attribute) => compiler.attribute_opcode_range(attribute),
        _ => expression.range(),
    }
}

pub(super) fn statement_execution_range(statement: &Stmt) -> ruff_text_size::TextRange {
    match statement {
        Stmt::Assign(statement) => first_expression_instruction_range(&statement.value),
        Stmt::AnnAssign(statement) => statement
            .value
            .as_deref()
            .map_or(statement.range, first_expression_instruction_range),
        Stmt::AugAssign(statement) => first_expression_instruction_range(&statement.target),
        Stmt::Expr(statement) => first_expression_instruction_range(&statement.value),
        Stmt::If(statement) => first_expression_instruction_range(&statement.test),
        Stmt::Return(statement) => statement
            .value
            .as_deref()
            .map_or(statement.range, first_expression_instruction_range),
        _ => statement.range(),
    }
}

pub(super) fn first_expression_instruction_range(expression: &Expr) -> ruff_text_size::TextRange {
    if matches!(
        expression,
        Expr::BinOp(_) | Expr::Name(_) | Expr::UnaryOp(_) | Expr::Tuple(_)
    ) && fold_constant(expression).is_some()
    {
        return expression.range();
    }
    match expression {
        Expr::Attribute(attribute) => first_expression_instruction_range(&attribute.value),
        Expr::BinOp(binary) => first_expression_instruction_range(&binary.left),
        Expr::BoolOp(boolean) => boolean
            .values
            .first()
            .map_or(expression.range(), first_expression_instruction_range),
        Expr::Call(call) => first_expression_instruction_range(&call.func),
        Expr::Compare(compare) => first_expression_instruction_range(&compare.left),
        Expr::If(conditional) => first_expression_instruction_range(&conditional.test),
        Expr::Named(named) => first_expression_instruction_range(&named.value),
        Expr::Subscript(subscript) => first_expression_instruction_range(&subscript.value),
        Expr::UnaryOp(unary) => first_expression_instruction_range(&unary.operand),
        _ => expression.range(),
    }
}

fn last_discarded_annotation_slice_range(expression: &Expr) -> ruff_text_size::TextRange {
    match expression {
        Expr::Tuple(tuple) => tuple
            .elts
            .last()
            .map_or(expression.range(), last_discarded_annotation_slice_range),
        Expr::Slice(slice) => slice
            .step
            .as_deref()
            .or(slice.upper.as_deref())
            .or(slice.lower.as_deref())
            .map_or(expression.range(), last_discarded_annotation_slice_range),
        _ => expression.range(),
    }
}
pub(super) fn unparse_annotation(expression: &Expr, source: &str) -> String {
    #[derive(Default)]
    struct AnnotationStrings<'ast> {
        needs_normalization: bool,
        fstrings: Vec<&'ast ruff_python_ast::ExprFString>,
        unpacked_keywords: Vec<&'ast ruff_python_ast::Keyword>,
        templates: Vec<&'ast ruff_python_ast::ExprTString>,
    }

    impl<'ast> Visitor<'ast> for AnnotationStrings<'ast> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            match expression {
                Expr::StringLiteral(_) => self.needs_normalization = true,
                Expr::FString(string) => {
                    self.needs_normalization = true;
                    self.fstrings.push(string);
                }
                Expr::TString(string) => {
                    self.needs_normalization = true;
                    self.templates.push(string);
                }
                Expr::Call(call) => {
                    self.needs_normalization |= call
                        .arguments
                        .args
                        .last()
                        .zip(call.arguments.keywords.first())
                        .is_some_and(|(argument, keyword)| argument.start() > keyword.start());
                    walk_expr(self, expression);
                }
                _ => walk_expr(self, expression),
            }
        }

        fn visit_keyword(&mut self, keyword: &'ast ruff_python_ast::Keyword) {
            if keyword.arg.is_none() {
                self.unpacked_keywords.push(keyword);
            }
            self.visit_expr(&keyword.value);
        }
    }

    struct AnnotationStringNormalizer<'source> {
        source: &'source str,
    }

    impl Transformer for AnnotationStringNormalizer<'_> {
        fn visit_expr(&self, expression: &mut Expr) {
            if let Expr::StringLiteral(string) = expression {
                let mut literal = string
                    .value
                    .iter()
                    .next()
                    .expect("string literals contain at least one part")
                    .clone();
                literal.value = string.value.to_str().into();
                literal.flags = if string.value.is_unicode()
                    && self
                        .source
                        .as_bytes()
                        .get(usize::from(literal.range.start()))
                        == Some(&b'u')
                {
                    StringLiteralFlags::empty().with_prefix(literal.flags.prefix())
                } else {
                    StringLiteralFlags::empty()
                };
                string.value = StringLiteralValue::single(literal);
            } else if let Expr::FString(string) = expression {
                // The surrounding expression generator only needs an atomic string node.
                // Its f-string rendering cannot represent CPython's whole-body repr, so
                // replace this empty placeholder after generating the surrounding expression.
                string.value = ruff_python_ast::FStringValue::single(ruff_python_ast::FString {
                    range: string.range,
                    node_index: string.node_index.clone(),
                    elements: Vec::new().into(),
                    flags: ruff_python_ast::FStringFlags::empty(),
                });
            } else if let Expr::TString(string) = expression {
                string.value = ruff_python_ast::TStringValue::single(ruff_python_ast::TString {
                    range: string.range,
                    node_index: string.node_index.clone(),
                    elements: Vec::new().into(),
                    flags: ruff_python_ast::TStringFlags::empty(),
                });
            } else {
                transform_expr(self, expression);
                if let Expr::Call(call) = expression {
                    // The code generator merges arguments and keywords by their source
                    // ranges, while CPython's annotation unparser prints the AST argument
                    // list before its keyword list. These ranges only affect this clone's
                    // formatting order; the expressions retain their original source ranges.
                    let end = call.range.end();
                    for keyword in &mut call.arguments.keywords {
                        keyword.range = ruff_text_size::TextRange::empty(end);
                    }
                }
            }
        }
    }

    if let Expr::StringLiteral(string) = expression {
        let mut annotation = python_string_repr(string.value.to_str());
        if string.value.is_unicode()
            && string.value.iter().next().is_some_and(|literal| {
                source.as_bytes().get(usize::from(literal.range.start())) == Some(&b'u')
            })
        {
            annotation.insert(0, 'u');
        }
        return annotation;
    }

    if let Expr::FString(string) = expression {
        return unparse_annotation_fstring(string, source);
    }
    if let Expr::TString(string) = expression {
        return unparse_annotation_tstring(string, source);
    }

    if annotation_needs_no_unparse_corrections(expression) {
        let indentation = Indentation::default();
        return Generator::new(&indentation, LineEnding::Lf)
            .with_mode(CodegenMode::AstUnparse)
            .expr(expression);
    }

    let mut original_strings = AnnotationStrings::default();
    original_strings.visit_expr(expression);
    let normalized = original_strings.needs_normalization.then(|| {
        let mut expression = expression.clone();
        AnnotationStringNormalizer { source }.visit_expr(&mut expression);
        expression
    });
    let indentation = Indentation::default();
    let mut annotation = Generator::new(&indentation, LineEnding::Lf)
        .with_mode(CodegenMode::AstUnparse)
        .expr(normalized.as_ref().unwrap_or(expression));
    let parsed = ruff_python_parser::parse_unchecked(
        &annotation,
        ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::ParenthesizedExpression),
    );
    let mut generated_strings = AnnotationStrings::default();
    if let ruff_python_ast::Mod::Expression(module) = parsed.syntax() {
        generated_strings.visit_expr(&module.body);
    }
    let mut replacements = generated_strings
        .fstrings
        .iter()
        .zip(original_strings.fstrings)
        .map(|(generated, original)| {
            (
                std::ops::Range::<usize>::from(generated.range),
                unparse_annotation_fstring(original, source),
            )
        })
        .collect::<Vec<_>>();
    replacements.extend(
        generated_strings
            .templates
            .iter()
            .zip(original_strings.templates)
            .map(|(generated, original)| {
                (
                    std::ops::Range::<usize>::from(generated.range),
                    unparse_annotation_tstring(original, source),
                )
            }),
    );

    let mut stack = Vec::<(TokenKind, usize)>::new();
    let mut parenthesis_pairs = FxHashSet::default();
    let mut bracket_pairs = Vec::new();
    for token in parsed.tokens() {
        match token.kind() {
            opening @ (TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace) => {
                stack.push((opening, usize::from(token.start())));
            }
            closing @ (TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace) => {
                let expected = match closing {
                    TokenKind::Rpar => TokenKind::Lpar,
                    TokenKind::Rsqb => TokenKind::Lsqb,
                    TokenKind::Rbrace => TokenKind::Lbrace,
                    _ => unreachable!(),
                };
                if let Some((opening, opening_index)) = stack.pop()
                    && opening == expected
                {
                    let closing_index = usize::from(token.start());
                    if opening == TokenKind::Lpar {
                        parenthesis_pairs.insert((opening_index, closing_index));
                    } else if opening == TokenKind::Lsqb {
                        bracket_pairs.push((opening_index, closing_index));
                    }
                }
            }
            _ => {}
        }
    }
    let mut removals = bracket_pairs
        .into_iter()
        .filter_map(|(opening, closing)| {
            let is_subscript = annotation.as_bytes()[..opening]
                .iter()
                .rev()
                .find(|byte| !byte.is_ascii_whitespace())
                .is_some_and(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'_' | b')' | b']' | b'}' | b'\'' | b'"')
                });
            (is_subscript
                && closing.saturating_sub(opening) > 3
                && parenthesis_pairs.contains(&(opening + 1, closing.saturating_sub(1))))
            .then_some([opening + 1, closing - 1])
        })
        .flatten()
        .collect::<Vec<_>>();
    // CPython accepts a conditional expression after **; the generator uses atom
    // precedence and adds parentheses around these lower-precedence expressions.
    for keyword in generated_strings.unpacked_keywords {
        if matches!(
            keyword.value,
            Expr::BoolOp(_)
                | Expr::BinOp(_)
                | Expr::UnaryOp(_)
                | Expr::Lambda(_)
                | Expr::If(_)
                | Expr::Compare(_)
                | Expr::Await(_)
        ) {
            let opening = usize::from(keyword.start()) + 2;
            let closing = usize::from(keyword.end()).saturating_sub(1);
            if parenthesis_pairs.contains(&(opening, closing)) {
                removals.extend([opening, closing]);
            }
        }
    }
    // CPython writes a space after lambda only when it has positional parameters.
    for (previous, current) in parsed.tokens().iter().zip(parsed.tokens().iter().skip(1)) {
        if previous.kind() == TokenKind::Lambda
            && matches!(current.kind(), TokenKind::Star | TokenKind::DoubleStar)
        {
            removals.extend(usize::from(previous.end())..usize::from(current.start()));
        }
    }
    removals.retain(|index| !replacements.iter().any(|(range, _)| range.contains(index)));
    replacements.extend(
        removals
            .into_iter()
            .map(|index| (index..index + 1, String::new())),
    );
    replacements.sort_unstable_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, replacement) in replacements {
        annotation.replace_range(range, &replacement);
    }
    if matches!(expression, Expr::Tuple(tuple) if !tuple.elts.is_empty()) {
        annotation.insert(0, '(');
        annotation.push(')');
    }
    annotation
}

/// These expressions cannot contain any of the constructs corrected after Ruff generates them.
fn annotation_needs_no_unparse_corrections(expression: &Expr) -> bool {
    match expression {
        Expr::Name(_) | Expr::NoneLiteral(_) => true,
        Expr::Attribute(attribute) => annotation_needs_no_unparse_corrections(&attribute.value),
        Expr::BinOp(binary) if binary.op == Operator::BitOr => {
            annotation_needs_no_unparse_corrections(&binary.left)
                && annotation_needs_no_unparse_corrections(&binary.right)
        }
        _ => false,
    }
}

/// CPython builds a complete `JoinedStr` body before applying string repr to it.
fn unparse_annotation_fstring(string: &ruff_python_ast::ExprFString, source: &str) -> String {
    let mut body = String::new();
    for part in &string.value {
        match part {
            FStringPart::Literal(literal) => {
                append_annotation_string_literal(&mut body, &literal.value);
            }
            FStringPart::FString(part) => {
                append_annotation_interpolations(&mut body, &part.elements, source, None);
            }
        }
    }
    format!("f{}", python_string_repr(&body))
}

/// Template interpolations retain their authored text, including grouping parentheses.
fn unparse_annotation_tstring(string: &ruff_python_ast::ExprTString, source: &str) -> String {
    let parsed = ruff_python_parser::parse_unchecked(
        &source[std::ops::Range::<usize>::from(string.range)],
        ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::ParenthesizedExpression),
    );
    let mut body = String::new();
    for part in &string.value {
        append_annotation_interpolations(
            &mut body,
            &part.elements,
            source,
            Some((parsed.tokens(), string.start())),
        );
    }
    format!("t{}", python_string_repr(&body))
}

fn append_annotation_string_literal(body: &mut String, value: &str) {
    body.push_str(&value.replace('{', "{{").replace('}', "}}"));
}

fn append_annotation_interpolations(
    body: &mut String,
    elements: &[InterpolatedStringElement],
    source: &str,
    template_tokens: Option<(&Tokens, TextSize)>,
) {
    for element in elements {
        match element {
            InterpolatedStringElement::Literal(literal) => {
                append_annotation_string_literal(body, &literal.value);
            }
            InterpolatedStringElement::Interpolation(interpolation) => {
                if let Some(debug_text) = &interpolation.debug_text {
                    append_annotation_string_literal(
                        body,
                        &annotation_interpolation_text(debug_text.as_str()),
                    );
                }
                let value = if let Some((tokens, offset)) = template_tokens {
                    let end = if interpolation.conversion.is_none() {
                        interpolation
                            .format_spec
                            .as_deref()
                            .map_or(interpolation.end(), Ranged::start)
                            - TextSize::new(1)
                    } else {
                        tokens
                            .iter()
                            .find(|token| {
                                token.kind() == TokenKind::Exclamation
                                    && token.start() + offset >= interpolation.expression.end()
                            })
                            .expect("template conversion has an exclamation token")
                            .start()
                            + offset
                    };
                    annotation_interpolation_text(
                        &source[usize::from(interpolation.start()) + 1..usize::from(end)],
                    )
                    .trim_end_matches(|character: char| {
                        character.is_whitespace() || character == '='
                    })
                    .to_owned()
                } else {
                    let mut value = unparse_annotation(&interpolation.expression, source);
                    if matches!(
                        interpolation.expression.as_ref(),
                        Expr::Lambda(_) | Expr::If(_)
                    ) {
                        value.insert(0, '(');
                        value.push(')');
                    }
                    value
                };
                body.push('{');
                if value.starts_with('{') {
                    body.push(' ');
                }
                body.push_str(&value);
                let conversion = interpolation.conversion.to_char().or_else(|| {
                    (interpolation.debug_text.is_some() && interpolation.format_spec.is_none())
                        .then_some('r')
                });
                if let Some(conversion) = conversion {
                    body.push('!');
                    body.push(conversion);
                }
                if let Some(spec) = &interpolation.format_spec {
                    body.push(':');
                    // CPython represents template format specs with FormattedValue nodes.
                    append_annotation_interpolations(body, &spec.elements, source, None);
                }
                body.push('}');
            }
        }
    }
}

/// Matches the pinned tokenizer's comment removal from interpolation metadata.
fn annotation_interpolation_text(value: &str) -> String {
    let mut quote = None;
    let mut characters = value.chars();
    let mut has_comment = false;
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                characters.next();
            }
            '\'' | '"' => {
                if quote == Some(character) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(character);
                }
            }
            '#' if quote.is_none() => {
                has_comment = true;
                break;
            }
            _ => {}
        }
    }
    if !has_comment {
        return value.to_owned();
    }

    // CPython's second pass handles quotes without skipping escaped characters.
    let mut result = String::with_capacity(value.len());
    let mut quote = None;
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match character {
            '\'' | '"' => {
                if quote == Some(character) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(character);
                }
                result.push(character);
            }
            '#' if quote.is_none() => {
                if characters.any(|character| character == '\n') {
                    result.push('\n');
                }
            }
            _ => result.push(character),
        }
    }
    result
}

fn python_string_repr(value: &str) -> String {
    ruff_python_literal::escape::UnicodeEscape::new_repr(value)
        .str_repr(ruff_python_ast::str::TripleQuotes::No)
        .to_string()
        .expect("Python string representation length fits in usize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python315_lazy_import_checks_precede_dead_code_elimination() {
        let options = crate::CompileOptions {
            python_version: crate::PythonVersion::Py315,
            ..crate::CompileOptions::default()
        };
        for (source, message) in [
            (
                "try:\n lazy import m\nexcept Error:\n pass",
                "lazy import not allowed inside try/except blocks",
            ),
            (
                "try:\n pass\nexcept Error:\n lazy from m import x as y",
                "lazy from ... import not allowed inside try/except blocks",
            ),
            (
                "try:\n pass\nexcept Error:\n pass\nelse:\n lazy import m",
                "lazy import not allowed inside try/except blocks",
            ),
            (
                "try:\n pass\nfinally:\n lazy import m",
                "lazy import not allowed inside try/except blocks",
            ),
            (
                "try:\n pass\nexcept* Error:\n lazy import m",
                "lazy import not allowed inside try/except blocks",
            ),
            (
                "if False:\n try:\n  lazy import m\n except Error:\n  pass",
                "lazy import not allowed inside try/except blocks",
            ),
            (
                "try:\n def f():\n  lazy import m\nexcept Error:\n pass",
                "lazy import not allowed inside functions",
            ),
            (
                "try:\n class C:\n  lazy import m\nexcept Error:\n pass",
                "lazy import not allowed inside classes",
            ),
            (
                "if False:\n def f():\n  lazy import m",
                "lazy import not allowed inside functions",
            ),
            (
                "if False:\n lazy from m import *",
                "lazy from ... import * is not allowed",
            ),
        ] {
            let error = crate::compile_with_options(source, "lazy.py", &options).unwrap_err();
            assert!(
                matches!(error, crate::CompileError::InvalidSyntax { message: actual, .. } if actual == message),
                "{source}"
            );
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(
                Compiler::validate_lazy_imports(parsed.suite(), crate::PythonVersion::Py314)
                    .is_ok()
            );
        }
    }

    #[test]
    fn python315_lazy_import_scope_allows_module_suites_and_relative_aliases() {
        let options = crate::CompileOptions {
            python_version: crate::PythonVersion::Py315,
            ..crate::CompileOptions::default()
        };
        for source in [
            "lazy import m as n",
            "lazy from . import x as y",
            "lazy from .__future__ import annotations",
            "if False:\n lazy import m",
            "with context:\n lazy import m",
            "for item in items:\n lazy import m",
            "try:\n pass\nexcept Error:\n pass\nlazy import m",
        ] {
            assert!(
                crate::compile_with_options(source, "lazy.py", &options).is_ok(),
                "{source}"
            );
        }
        for source in [
            "lazy from __future__ import annotations",
            "if False:\n lazy from __future__ import annotations",
        ] {
            assert!(
                crate::compile_with_options(source, "lazy.py", &options).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn raw_identifier_dunder_matches_adjacent_ascii_underscores() {
        for (name, expected) in [
            ("", false),
            ("_", false),
            ("plain", false),
            ("under_score", false),
            ("_on_both_sides_", false),
            ("__", true),
            ("___", true),
            ("__private", true),
            ("_Café__already", true),
            ("trailing__", true),
            ("_é_", false),
            ("名__字", true),
            ("＿_", false),
            ("_＿", false),
            ("＿＿", false),
        ] {
            assert_eq!(has_double_underscore(name), expected, "{name:?}");
        }

        let alphabet = ["", "_", "a", "é", "名", "＿"];
        for first in alphabet {
            for second in alphabet {
                for third in alphabet {
                    for fourth in alphabet {
                        let name = [first, second, third, fourth].concat();
                        assert_eq!(
                            has_double_underscore(&name),
                            name.contains("__"),
                            "{name:?}"
                        );
                    }
                }
            }
        }
        let mut name = "letters_and_é".repeat(64);
        assert!(!has_double_underscore(&name));
        name.push_str("__end");
        assert!(has_double_underscore(&name));
    }

    #[test]
    fn parser_dunders_preserve_raw_and_generated_mangled_identities() {
        let parsed = ruff_python_parser::parse_module(
            "class Café:\n    def f(self):\n        self.__secret\n        self.__already\n        self._Café__already\n        self.élan\n        self.ends_\n",
        )
        .unwrap();
        let mut generated = FxHashSet::default();
        let mut requires_trace = false;
        let origins = parser_interned_strings(parsed.suite(), &mut generated, &mut requires_trace);
        for raw in [
            "Café",
            "__secret",
            "__already",
            "_Café__already",
            "élan",
            "ends_",
        ] {
            assert!(origins.contains_key(raw), "{raw:?}");
        }
        assert!(generated.contains("_Café__secret"));
        assert!(!generated.contains("_Café__already"));
        assert!(requires_trace);
    }

    #[test]
    fn finishing_module_string_provenance_moves_exclusive_keys_and_preserves_shared_context() {
        let tokens = Tokens::new(Vec::new());
        let mut context = CompilationContext::new("example.py", "", &tokens, &Suite::new());
        context.constant_string_origins = [
            ("unused", 0),
            ("not_interned", 1),
            ("canonical", 2),
            ("mangled_unused", 0),
            ("mangled_not_interned", 1),
            ("mangled_canonical", 2),
        ]
        .into_iter()
        .map(|(name, state)| (name.to_string(), AtomicU8::new(state)))
        .collect();
        context.generated_mangled_strings = [
            "mangled_unused",
            "mangled_not_interned",
            "mangled_canonical",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        context.requires_interning_trace = true;
        context.record_interning_event(CompilationInterningEventKind::CachedTuple(vec![
            "earlier".to_string(),
        ]));
        let mut context = Arc::new(context);
        let input_key_address = |name: &str| {
            context
                .constant_string_origins
                .get_key_value(name)
                .unwrap()
                .0
                .as_ptr()
        };
        let unused_address = input_key_address("unused");
        let canonical_address = input_key_address("canonical");
        let mangled_canonical_address = input_key_address("mangled_canonical");
        let expected: (FxHashSet<String>, FxHashSet<String>) = (
            ["unused", "not_interned", "canonical"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            ["canonical", "mangled_canonical"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        );

        let shared = Arc::clone(&context);
        let copied = CompilationContext::finish_constant_string_provenance(&mut context);
        assert_eq!(copied, expected);
        assert_eq!(shared.constant_string_origins.len(), 6);
        drop(shared);

        let moved = CompilationContext::finish_constant_string_provenance(&mut context);
        assert_eq!(moved, expected);
        assert_eq!(moved.0.get("unused").unwrap().as_ptr(), unused_address);
        assert_eq!(
            moved.0.get("canonical").unwrap().as_ptr(),
            canonical_address
        );
        assert_eq!(
            moved.1.get("mangled_canonical").unwrap().as_ptr(),
            mangled_canonical_address
        );
        assert!(context.constant_string_origins.is_empty());
        assert_eq!(context.generated_mangled_strings.len(), 3);
        assert!(context.requires_interning_trace);
        assert_eq!(context.interning_events().len(), 1);
        assert_eq!(
            context.record_interning_event(CompilationInterningEventKind::CachedTuple(Vec::new())),
            1
        );
    }
}
