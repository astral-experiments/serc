use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::transformer::{Transformer, walk_expr as transform_expr};
use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{
    Expr, ExprContext, FStringPartRef, InterpolatedStringElement, Operator, Stmt, StmtFunctionDef,
    StringLiteralFlags, StringLiteralValue, Suite,
};
use ruff_python_codegen::{Generator, Indentation, Mode as CodegenMode};
use ruff_source_file::{LineEnding, LineIndex};
use ruff_text_size::{Ranged, TextLen, TextRange, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::assembler::SourceLocation;
use crate::code_object::{
    CodeFilename, CodeInterningSummary, CodeObject, CompilationInterningEvent, Constant,
};

use super::constant::{
    constant_slice_with_literals, explicit_surrogate_string, fold_constant,
    fold_constant_for_version, fold_constant_with_profile, is_literal_constant, literal_constant,
};
use super::scope::{ModulePlan, NameId, ScopeArena};
use super::statement::function_body_contains_yield;
use super::symbol::mangle_name;
use super::{CachedSourceLine, Compiler};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StringConstantOrigin {
    #[default]
    Unrecorded,
    Uninterned,
    Interned,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum EmptyBytesOrigin {
    #[default]
    Unrecorded,
    Singleton,
    Fresh,
}

#[derive(Debug, Default)]
struct ConstantOrigins {
    all_strings: bool,
    interned_qualified_names: FxHashSet<String>,
    strings: FxHashMap<String, bool>,
    string_tuples: FxHashSet<Vec<String>>,
}

impl ConstantOrigins {
    fn tracks(&self, value: &str) -> bool {
        self.all_strings || value.contains('.') || value == "%"
    }

    fn record_constant(&mut self, constant: &Constant) {
        match constant {
            Constant::String(value) if self.tracks(value) => {
                self.strings.entry(value.clone()).or_insert(false);
            }
            Constant::Tuple(values) => {
                // Metadata tuples enter the native cache without admitting their members.
                // Reusing one as a literal must not create scalar cache entries afterwards.
                if let Some(strings) = self.string_tuple(values)
                    && !self.string_tuples.insert(strings)
                {
                    return;
                }
                for value in values {
                    self.record_constant(value);
                }
            }
            Constant::FrozenSet(values) => {
                for value in values {
                    self.record_constant(value);
                }
            }
            // Nested code objects recorded their admissions while compiling their bodies.
            _ => {}
        }
    }

    fn string_tuple(&self, values: &[Constant]) -> Option<Vec<String>> {
        if !self.all_strings
            && !values
                .iter()
                .any(|value| matches!(value, Constant::String(value) if self.tracks(value)))
        {
            return None;
        }
        values
            .iter()
            .map(|value| match value {
                Constant::String(value) => Some(value.clone()),
                _ => None,
            })
            .collect()
    }

    fn record_code_assembly(&mut self, code: &CodeObject) {
        if code.names.iter().any(|value| self.tracks(value)) {
            self.string_tuples
                .insert(code.names.iter().map(ToString::to_string).collect());
        }
        if let Some(strings) = self.string_tuple(&code.constants) {
            self.string_tuples.insert(strings);
        }
        if code.marshal_provenance.locals_identity == crate::code_object::LocalsIdentity::ByValue
            && code.locals.iter().any(|value| self.tracks(value))
        {
            self.string_tuples
                .insert(code.locals.iter().map(ToString::to_string).collect());
        }
    }
}

#[derive(Debug)]
pub(super) struct CompilationContext<'source> {
    pub(super) python_version: crate::PythonVersion,
    pub(super) native_build_profile: crate::NativeBuildProfile,
    pub(super) filename: CodeFilename,
    pub(super) source: &'source str,
    pub(super) tokens: &'source Tokens,
    line_index: LineIndex,
    pub(super) optimized_locations: FxHashMap<TextRange, super::optimization::OptimizedLocation>,
    pub(super) scopes: ScopeArena,
    original_generator_functions: Option<FxHashSet<TextRange>>,
    pub(super) module_contains_yield: bool,
    finally_bodies: FxHashMap<TextRange, &'source [Stmt]>,
    constant_string_origins: FxHashMap<Cow<'source, str>, Cell<StringConstantOrigin>>,
    // Class qualified-name and synthesized character identity depend on the first cache admission,
    // even when that constant or the defining class is later removed as unreachable.
    constant_origins: Option<RefCell<ConstantOrigins>>,
    // Discarded code can intern an equal literal tuple without interning standalone literals.
    interned_metadata_tuples: Option<RefCell<Vec<Vec<String>>>>,
    static_attribute_names: FxHashSet<String>,
    active_class_static_attributes: RefCell<Vec<BTreeSet<String>>>,
    cached_static_attribute_tuples: RefCell<FxHashSet<Vec<String>>>,
    pub(super) generated_mangled_strings: FxHashSet<String>,
    // Generated names can first become canonical when an otherwise discarded child assembles.
    // Restricted generic-class identifiers also require real child lowering for key origins.
    pub(super) requires_interning_trace: bool,
    pub(super) requires_empty_bytes_cache_trace: bool,
    empty_bytes_origin: Cell<EmptyBytesOrigin>,
    interning_events: RefCell<Vec<CompilationInterningEvent>>,
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
    pub(super) fn fold_constant(&self, expression: &Expr) -> Option<Constant> {
        if self.native_build_profile == crate::NativeBuildProfile::Default {
            return fold_constant_for_version(expression, self.python_version);
        }
        fold_constant_with_profile(
            expression,
            self.python_version,
            self.native_build_profile,
            &literal_constant,
            true,
        )
    }

    #[cfg(test)]
    pub(super) fn new(
        filename: &str,
        source: &'source str,
        tokens: &'source Tokens,
        body: &'source Suite,
    ) -> Self {
        Self::with_python_version(
            filename.into(),
            source,
            tokens,
            body,
            crate::PythonVersion::Py314,
        )
    }

    #[cfg(test)]
    pub(super) fn with_python_version(
        filename: CodeFilename,
        source: &'source str,
        tokens: &'source Tokens,
        body: &'source Suite,
        python_version: crate::PythonVersion,
    ) -> Self {
        Self::with_profile(
            filename,
            source,
            tokens,
            body,
            python_version,
            crate::NativeBuildProfile::Default,
        )
    }

    pub(super) fn with_profile(
        filename: CodeFilename,
        source: &'source str,
        tokens: &'source Tokens,
        body: &'source Suite,
        python_version: crate::PythonVersion,
        native_build_profile: crate::NativeBuildProfile,
    ) -> Self {
        let name_capacity = if body.is_empty() {
            0
        } else {
            body.len().max(tokens.len() / 20).clamp(32, 4096)
        };
        let mut source_analysis = SourceAnalysis::new(python_version, name_capacity);
        let scopes = ScopeArena::analyze_with_source(body, python_version, &mut source_analysis);
        let mut generated_mangled_strings = FxHashSet::default();
        let mut requires_interning_trace = false;
        let mut static_attribute_names = FxHashSet::default();
        let mut has_percent_replacement = false;
        let mut finally_bodies = FxHashMap::default();
        let (constant_string_origins, requires_empty_bytes_cache_trace) = source_analysis.finish(
            &mut generated_mangled_strings,
            &mut requires_interning_trace,
            &mut static_attribute_names,
            &mut has_percent_replacement,
            &mut finally_bodies,
        );
        let legacy = python_version <= crate::PythonVersion::Py313;
        let has_relevant_class = if legacy {
            scopes.has_class_scopes()
        } else {
            scopes.has_nested_class_scopes()
        };
        let constant_origins = (has_relevant_class || has_percent_replacement).then(|| {
            RefCell::new(ConstantOrigins {
                all_strings: legacy,
                ..ConstantOrigins::default()
            })
        });
        let interned_metadata_tuples = constant_string_origins
            .keys()
            .any(|name| !name.is_ascii())
            .then(RefCell::default);
        Self {
            python_version,
            native_build_profile,
            filename,
            source,
            tokens,
            line_index: LineIndex::from_source_text(source),
            optimized_locations: FxHashMap::default(),
            scopes,
            original_generator_functions: None,
            module_contains_yield: false,
            finally_bodies,
            constant_string_origins,
            constant_origins,
            interned_metadata_tuples,
            static_attribute_names,
            active_class_static_attributes: RefCell::default(),
            cached_static_attribute_tuples: RefCell::new(FxHashSet::default()),
            generated_mangled_strings,
            requires_interning_trace,
            requires_empty_bytes_cache_trace,
            empty_bytes_origin: Cell::default(),
            interning_events: RefCell::new(Vec::new()),
        }
    }

    /// Preserve symbol-table and suspension facts established before assertion removal.
    pub(super) fn preserve_unoptimized_scopes(&mut self, body: &Suite) {
        struct GeneratorCollector {
            ranges: FxHashSet<TextRange>,
            eager_annotations: bool,
        }

        impl<'ast> Visitor<'ast> for GeneratorCollector {
            fn visit_stmt(&mut self, statement: &'ast Stmt) {
                if let Stmt::FunctionDef(function) = statement
                    && function_body_contains_yield(&function.body, self.eager_annotations)
                {
                    self.ranges.insert(function.range);
                }
                walk_stmt(self, statement);
            }

            fn visit_expr(&mut self, expression: &'ast Expr) {
                if crate::stack_growth_required() {
                    return crate::grow_stack(|| self.visit_expr(expression));
                }
                walk_expr(self, expression);
            }
        }

        let mut original_scopes =
            ScopeArena::analyze_with_optimized_assertions(body, self.python_version);
        original_scopes.retain_lowered_comprehension_locals(&self.scopes);
        self.scopes = original_scopes;
        let eager_annotations = self.python_version <= crate::PythonVersion::Py313
            && self.module_plan().future_flags & crate::target::code_flags::CO_FUTURE_ANNOTATIONS
                == 0;
        self.module_contains_yield = function_body_contains_yield(body, eager_annotations);
        let mut collector = GeneratorCollector {
            ranges: FxHashSet::default(),
            eager_annotations,
        };
        collector.visit_body(body);
        self.original_generator_functions = Some(collector.ranges);
    }

    /// Use the original function body when optimization has removed a suspension expression.
    pub(super) fn function_contains_yield(&self, definition: &StmtFunctionDef) -> bool {
        if let Some(generators) = &self.original_generator_functions {
            return generators.contains(&definition.range);
        }
        self.module_plan().may_contain_yield(&definition.body)
            && function_body_contains_yield(
                &definition.body,
                self.python_version <= crate::PythonVersion::Py313
                    && self.module_plan().future_flags
                        & crate::target::code_flags::CO_FUTURE_ANNOTATIONS
                        == 0,
            )
    }

    /// Recover the original AST suite even when lowering revisits a copied exit.
    pub(super) fn finally_body(&self, statement: &ruff_python_ast::StmtTry) -> &'source [Stmt] {
        self.finally_bodies[&statement.range]
    }

    pub(super) const fn module_plan(&self) -> &ModulePlan {
        self.scopes.module()
    }

    /// Preserve the first empty object admitted to CPython's compilation-wide constant cache.
    pub(super) fn record_empty_bytes_origin(&self, fresh: bool) {
        if self.requires_empty_bytes_cache_trace
            && self.empty_bytes_origin.get() == EmptyBytesOrigin::Unrecorded
        {
            self.empty_bytes_origin.set(if fresh {
                EmptyBytesOrigin::Fresh
            } else {
                EmptyBytesOrigin::Singleton
            });
        }
    }

    pub(super) fn fresh_empty_bytes_constant(&self) -> bool {
        self.empty_bytes_origin.get() == EmptyBytesOrigin::Fresh
    }

    pub(super) fn record_constant_empty_bytes_origin(&self, constant: &Constant) {
        if !self.requires_empty_bytes_cache_trace
            || self.empty_bytes_origin.get() != EmptyBytesOrigin::Unrecorded
        {
            return;
        }
        match constant {
            Constant::Bytes(value) if value.is_empty() => self.record_empty_bytes_origin(false),
            Constant::Tuple(values) | Constant::FrozenSet(values) => {
                for value in values {
                    self.record_constant_empty_bytes_origin(value);
                }
            }
            _ => {}
        }
    }

    /// Records the first object admitted to the compilation-wide constant cache for an
    /// parser-interned name used as a constant. Later equal constants reuse that object's identity,
    /// even when the first load is removed by control-flow optimization.
    pub(super) fn record_constant_string_origin(&self, value: &str, interned: bool) {
        if let Some(origin) = self.constant_string_origins.get(value)
            && origin.get() == StringConstantOrigin::Unrecorded
        {
            origin.set(if interned {
                StringConstantOrigin::Interned
            } else {
                StringConstantOrigin::Uninterned
            });
        }
    }

    pub(super) fn constant_string_origin_is_interned(&self, value: &str) -> bool {
        self.constant_string_origins
            .get(value)
            .is_some_and(|origin| origin.get() == StringConstantOrigin::Interned)
    }

    pub(super) fn record_identifier_constant_origin(&self, raw: &str, mangled: &str) {
        // `_Py_Mangle` returns a fresh string when the spelling changes, even if an equal
        // identifier was already interned by the parser.
        self.record_constant_string_origin(mangled, raw == mangled);
    }

    pub(super) fn record_constant_string_origins(&self, constant: &Constant, interned: bool) {
        if let Some(origins) = &self.constant_origins {
            origins.borrow_mut().record_constant(constant);
        }
        self.record_interned_constant_string_origins(constant, interned);
    }

    fn record_interned_constant_string_origins(&self, constant: &Constant, interned: bool) {
        if self.constant_string_origins.is_empty() {
            return;
        }
        match constant {
            Constant::String(value) => self.record_constant_string_origin(value, interned),
            Constant::Tuple(values) | Constant::FrozenSet(values) => {
                if matches!(constant, Constant::Tuple(_)) {
                    if self.record_static_attribute_tuple_cache(values.iter().map(|value| {
                        if let Constant::String(value) = value {
                            Some(value.as_str())
                        } else {
                            None
                        }
                    })) {
                        return;
                    }
                    self.record_cached_string_tuple(values);
                }
                for value in values {
                    self.record_interned_constant_string_origins(value, interned);
                }
            }
            // A nested code object's constants were recorded while compiling its body.
            _ => {}
        }
    }

    /// Tracks whole-tuple hits that can suppress a static attribute's scalar admission.
    fn record_static_attribute_tuple_cache<'a>(
        &self,
        values: impl Iterator<Item = Option<&'a str>> + Clone,
    ) -> bool {
        if self.static_attribute_names.is_empty()
            || !values
                .clone()
                .any(|value| value.is_some_and(|value| self.static_attribute_names.contains(value)))
        {
            return false;
        }
        let Some(values) = values
            .map(|value| value.map(str::to_string))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        !self
            .cached_static_attribute_tuples
            .borrow_mut()
            .insert(values)
    }

    pub(super) fn begin_class_static_attributes(&self) {
        self.active_class_static_attributes
            .borrow_mut()
            .push(BTreeSet::new());
    }

    /// CPython searches the enclosing compiler units, excluding the current unit.
    /// A store in a nested class body therefore belongs to its outer class.
    pub(super) fn record_static_attribute(&self, name: &str, current_unit_is_class: bool) {
        let mut classes = self.active_class_static_attributes.borrow_mut();
        if let Some(index) = classes
            .len()
            .checked_sub(1 + usize::from(current_unit_is_class))
        {
            classes[index].insert(name.to_string());
        }
    }

    pub(super) fn finish_class_static_attributes(&self) -> Vec<Constant> {
        self.active_class_static_attributes
            .borrow_mut()
            .pop()
            .expect("class static attributes were initialized")
            .into_iter()
            .map(Constant::String)
            .collect()
    }

    /// Static attributes enter the recursive cache as raw parser-interned identifiers.
    pub(super) fn record_static_attribute_tuple(&self, values: &[Constant]) {
        let cached = self.record_static_attribute_tuple_cache(values.iter().map(|value| {
            if let Constant::String(value) = value {
                Some(value.as_str())
            } else {
                None
            }
        }));
        if !cached {
            for value in values {
                if let Constant::String(value) = value {
                    self.record_constant_string_origin(value, true);
                }
            }
        }
        self.record_cached_string_tuple(values);
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
            self.record_interning_event(CompilationInterningEvent::CachedTuple(values));
        }
    }

    fn record_interning_event(&self, event: CompilationInterningEvent) -> usize {
        let mut events = self.interning_events.borrow_mut();
        let order = events.len();
        events.push(event);
        order
    }

    pub(super) fn record_code_assembly(&self, code: &CodeObject) -> Option<usize> {
        if let Some(tuples) = &self.interned_metadata_tuples {
            let mut tuples = tuples.borrow_mut();
            if code.names.iter().any(|name| !name.is_ascii()) {
                tuples.push(code.names.iter().map(ToString::to_string).collect());
            }
            if code.marshal_provenance.locals_identity
                == crate::code_object::LocalsIdentity::ByValue
                && code.locals.iter().any(|name| !name.is_ascii())
            {
                tuples.push(code.locals.iter().map(ToString::to_string).collect());
            }
        }
        if let Some(origins) = &self.constant_origins {
            let mut origins = origins.borrow_mut();
            origins.record_code_assembly(code);
            if self.python_version != crate::PythonVersion::Py312
                && origins.tracks(&code.qualified_name)
            {
                origins
                    .interned_qualified_names
                    .insert(code.qualified_name.clone());
            }
        }
        // Assembly merges these tuples without recursively admitting their members.
        self.record_static_attribute_tuple_cache(
            code.names.iter().map(|value| Some(value.as_str())),
        );
        self.record_static_attribute_tuple_cache(code.constants.iter().map(|value| {
            if let Constant::String(value) = value {
                Some(value.as_str())
            } else {
                None
            }
        }));
        if code.marshal_provenance.locals_identity == crate::code_object::LocalsIdentity::ByValue {
            self.record_static_attribute_tuple_cache(
                code.locals.iter().map(|value| Some(value.as_str())),
            );
        }
        if !self.requires_interning_trace {
            return None;
        }
        Some(
            self.record_interning_event(CompilationInterningEvent::CodeAssembly(
                CodeInterningSummary::from_code(code),
            )),
        )
    }

    pub(super) fn take_interning_events(&self) -> Vec<CompilationInterningEvent> {
        self.interning_events.take()
    }

    pub(super) fn take_interned_metadata_tuples(&self) -> Vec<Vec<String>> {
        self.interned_metadata_tuples
            .as_ref()
            .map_or_else(Vec::new, RefCell::take)
    }

    pub(super) fn has_tracked_string_origins(&self) -> bool {
        !self.constant_string_origins.is_empty() || self.tracks_constant_origins()
    }

    pub(super) fn tracks_constant_origins(&self) -> bool {
        self.constant_origins.is_some()
    }

    pub(super) fn requires_discarded_builders(&self) -> bool {
        self.requires_interning_trace
            || self.tracks_constant_origins()
            || !self.active_class_static_attributes.borrow().is_empty()
            || self.interned_metadata_tuples.is_some()
    }

    /// Records the class body's actual qualified-name constant admission.
    ///
    /// Python 3.12 aliases metadata only for the winning class definition. Python
    /// 3.13 and newer intern that winner, so later equal class names can share its identity.
    pub(super) fn record_class_qualified_name(&self, value: &str) -> Option<bool> {
        let origins = self.constant_origins.as_ref()?;
        let mut origins = origins.borrow_mut();
        if !origins.tracks(value) {
            return None;
        }
        let metadata_already_interned = self.filename.as_utf8() == Some(value)
            || origins.interned_qualified_names.contains(value);
        match origins.strings.entry(value.to_string()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                // A dotted import interns its complete spelling while parsing. In 3.13,
                // code construction reuses that object for the qualified-name metadata,
                // leaving the class's newly cached scalar constant distinct.
                let canonical = self.python_version == crate::PythonVersion::Py312
                    || (!self.constant_string_origins.contains_key(value)
                        && !metadata_already_interned);
                entry.insert(canonical);
                Some(canonical)
            }
            std::collections::hash_map::Entry::Occupied(entry) => {
                Some(self.python_version != crate::PythonVersion::Py312 && *entry.get())
            }
        }
    }

    /// Records the fresh one-character result of replacing `%%` with `%` during AST preprocessing.
    /// `PyUnicode_Replace` allocates this result directly instead of using the Latin-1 singleton.
    pub(super) fn record_percent_replacement(&self) {
        if let Some(origins) = &self.constant_origins {
            origins
                .borrow_mut()
                .strings
                .entry("%".to_string())
                .or_insert(true);
        }
    }

    pub(super) fn has_uncached_percent_constant(&self) -> bool {
        self.constant_origins
            .as_ref()
            .is_some_and(|origins| origins.borrow().strings.get("%") == Some(&true))
    }

    /// Takes the completed module's string keys after all child compilers have been consumed.
    /// Only frozenset reconstruction needs the full set of parser-interned identifiers.
    pub(super) fn finish_constant_string_provenance(
        context: &mut Rc<Self>,
        has_frozenset_constants: bool,
    ) -> (FxHashSet<String>, FxHashSet<String>) {
        let context = Rc::get_mut(context)
            .expect("completed module should exclusively own its compilation context");
        let origins = std::mem::take(&mut context.constant_string_origins);
        let mut identifiers = FxHashSet::default();
        if has_frozenset_constants {
            identifiers.reserve(origins.len());
        }
        let mut canonical = FxHashSet::default();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "Both provenance sets are only queried for membership."
        )]
        for (value, origin) in origins {
            if has_frozenset_constants
                && !context.generated_mangled_strings.contains(value.as_ref())
            {
                if origin.into_inner() == StringConstantOrigin::Interned {
                    canonical.insert(value.as_ref().to_string());
                }
                identifiers.insert(value.into_owned());
            } else if origin.into_inner() == StringConstantOrigin::Interned {
                canonical.insert(value.into_owned());
            }
        }
        (identifiers, canonical)
    }
}

/// Tests for the ASCII dunder substring without constructing a generic string searcher.
fn has_double_underscore(name: &str) -> bool {
    name.as_bytes().windows(2).any(|pair| pair == b"__")
}

/// Whether folding can carry a bytes literal to a result, possibly through tuple indexing.
fn can_fold_bytes(expression: &Expr) -> bool {
    if crate::stack_growth_required() {
        return crate::grow_stack(|| can_fold_bytes(expression));
    }
    match expression {
        Expr::BytesLiteral(_) => true,
        Expr::BinOp(binary) if matches!(binary.op, Operator::Add | Operator::Mult) => {
            can_fold_bytes(&binary.left) || can_fold_bytes(&binary.right)
        }
        Expr::Tuple(tuple) => tuple.elts.iter().any(can_fold_bytes),
        Expr::Subscript(subscript) => can_fold_bytes(&subscript.value),
        _ => false,
    }
}

#[derive(Default)]
pub(super) struct SourceAnalysis<'ast> {
    python_version: crate::PythonVersion,
    finally_bodies: FxHashMap<TextRange, &'ast [Stmt]>,
    origins: FxHashMap<Cow<'ast, str>, Cell<StringConstantOrigin>>,
    // Source-only identifiers still enter `origins` directly, in their original visitation order.
    expression_origin_names: Vec<ExpressionOriginNames>,
    private_name: Option<String>,
    mangled_names: Option<FxHashSet<String>>,
    generated_mangled_strings: FxHashSet<String>,
    raw_private_identifiers: FxHashSet<&'ast str>,
    restricted_private_identifier: bool,
    static_attribute_names: FxHashSet<String>,
    has_percent_replacement: bool,
    requires_empty_bytes_cache_trace: bool,
}

#[derive(Clone, Copy, Default)]
struct ExpressionOriginNames {
    seen: u64,
    // Whether the raw spelling contains `__`; only these need context-dependent work on reuse.
    private: u64,
}

pub(super) struct SourceClassState {
    private_name: Option<String>,
    mangled_names: Option<FxHashSet<String>>,
}

impl<'ast> SourceAnalysis<'ast> {
    pub(super) fn new(python_version: crate::PythonVersion, name_capacity: usize) -> Self {
        Self {
            python_version,
            origins: FxHashMap::with_capacity_and_hasher(name_capacity, rustc_hash::FxBuildHasher),
            ..Self::default()
        }
    }

    fn mangled_name<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self
            .mangled_names
            .as_ref()
            .is_some_and(|names| !names.contains(name))
        {
            Cow::Borrowed(name)
        } else {
            mangle_name(name, self.private_name.as_deref())
        }
    }

    fn record_name(&mut self, name: &'ast str) {
        self.record_raw_name(name);
        let mangled = self.mangled_name(name);
        if mangled != name {
            let mangled = mangled.into_owned();
            self.generated_mangled_strings.insert(mangled.clone());
            self.origins.entry(Cow::Owned(mangled)).or_default();
        }
    }

    fn record_raw_name(&mut self, name: &'ast str) {
        self.origins.entry(Cow::Borrowed(name)).or_default();
        self.record_raw_name_effects(name);
    }

    #[inline]
    fn record_raw_name_with_id(&mut self, name: &'ast str, name_id: NameId) {
        let index = name_id.index();
        let word_index = index / u64::BITS as usize;
        if word_index >= self.expression_origin_names.len() {
            self.expression_origin_names
                .resize(word_index + 1, ExpressionOriginNames::default());
        }
        let word = &mut self.expression_origin_names[word_index];
        let bit = 1 << (index % u64::BITS as usize);
        let private = if word.seen & bit == 0 {
            word.seen |= bit;
            self.origins.entry(Cow::Borrowed(name)).or_default();
            let private = has_double_underscore(name);
            if private {
                word.private |= bit;
            }
            private
        } else {
            word.private & bit != 0
        };
        if private {
            self.record_private_name_effects(name);
        }
    }

    fn record_raw_name_effects(&mut self, name: &'ast str) {
        if has_double_underscore(name) {
            self.record_private_name_effects(name);
        }
    }

    fn record_private_name_effects(&mut self, name: &'ast str) {
        self.restricted_private_identifier |= self.mangled_names.is_some();
        self.raw_private_identifiers.insert(name);
        let mangled = self.mangled_name(name);
        if mangled != name {
            self.generated_mangled_strings.insert(mangled.into_owned());
        }
    }

    pub(super) fn observe_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::Try(statement) = statement
            && !statement.finalbody.is_empty()
        {
            self.finally_bodies
                .insert(statement.range, &statement.finalbody);
        }
        match statement {
            Stmt::AnnAssign(assignment) => {
                if let Expr::Name(target) = assignment.target.as_ref() {
                    self.record_name(target.id.as_str());
                }
            }
            Stmt::FunctionDef(definition) => {
                self.record_raw_name(definition.name.as_str());
                if definition.returns.is_some() {
                    self.origins.entry(Cow::Borrowed("return")).or_default();
                }
            }
            Stmt::ClassDef(definition) => {
                self.record_raw_name(definition.name.as_str());
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
                self.origins.entry(Cow::Borrowed("case")).or_default();
            }
            Stmt::TypeAlias(alias) => {
                if let Expr::Name(name) = alias.name.as_ref() {
                    self.origins
                        .entry(Cow::Borrowed(name.id.as_str()))
                        .or_default();
                }
            }
            _ => {}
        }
    }

    pub(super) fn enter_class_header(
        &mut self,
        definition: &'ast ruff_python_ast::StmtClassDef,
    ) -> SourceClassState {
        let previous = SourceClassState {
            private_name: self.private_name.clone(),
            mangled_names: self.mangled_names.clone(),
        };
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
        }
        previous
    }

    pub(super) fn enter_class_body(&mut self, definition: &ruff_python_ast::StmtClassDef) {
        self.private_name = Some(definition.name.to_string());
        self.mangled_names = None;
    }

    pub(super) fn leave_class(&mut self, previous: SourceClassState) {
        self.private_name = previous.private_name;
        self.mangled_names = previous.mangled_names;
    }

    pub(super) fn observe_parameters(&mut self, parameters: &'ast ruff_python_ast::Parameters) {
        for parameter in &parameters.kwonlyargs {
            if parameter.default.is_some() {
                self.record_name(parameter.name().as_str());
            }
        }
    }

    pub(super) fn observe_parameter(&mut self, parameter: &'ast ruff_python_ast::Parameter) {
        self.record_raw_name(parameter.name.as_str());
        if parameter.annotation.is_some() {
            self.record_name(parameter.name.as_str());
        }
    }

    pub(super) fn observe_expr(&mut self, expression: &'ast Expr) {
        match expression {
            Expr::Name(name) => {
                self.record_raw_name(name.id.as_str());
            }
            Expr::BinOp(binary) if binary.op == Operator::Mod => {
                if let Expr::StringLiteral(format) = binary.left.as_ref() {
                    self.has_percent_replacement |= format.value.to_str().contains("%%");
                }
            }
            Expr::BinOp(binary)
                if binary.op == Operator::Mult
                    && !self.requires_empty_bytes_cache_trace
                    && self.python_version >= crate::PythonVersion::Py314
                    && can_fold_bytes(expression) =>
            {
                self.requires_empty_bytes_cache_trace = matches!(
                    fold_constant_for_version(expression, self.python_version),
                    Some(Constant::Bytes(value)) if value.is_empty()
                );
            }
            Expr::Attribute(attribute) => {
                self.record_raw_name(attribute.attr.as_str());
                self.record_attribute_effects(attribute);
            }
            _ => {}
        }
    }

    #[inline]
    pub(super) fn observe_name_with_id(
        &mut self,
        name: &'ast ruff_python_ast::ExprName,
        id: NameId,
    ) {
        self.record_raw_name_with_id(name.id.as_str(), id);
    }

    #[inline]
    pub(super) fn observe_attribute_with_id(
        &mut self,
        attribute: &'ast ruff_python_ast::ExprAttribute,
        id: NameId,
    ) {
        self.record_raw_name_with_id(attribute.attr.as_str(), id);
        self.record_attribute_effects(attribute);
    }

    fn record_attribute_effects(&mut self, attribute: &ruff_python_ast::ExprAttribute) {
        // Collect a conservative cache filter in this existing traversal.
        // Actual class ownership is determined by the lowering Store path.
        if attribute.ctx == ExprContext::Store
            && matches!(attribute.value.as_ref(), Expr::Name(name) if name.id.as_str() == "self")
        {
            self.static_attribute_names
                .insert(attribute.attr.to_string());
        }
    }

    pub(super) fn observe_except_handler(&mut self, handler: &'ast ruff_python_ast::ExceptHandler) {
        let ruff_python_ast::ExceptHandler::ExceptHandler(handler) = handler;
        if let Some(name) = &handler.name {
            self.record_raw_name(name.as_str());
        }
    }

    pub(super) fn observe_pattern(&mut self, pattern: &'ast ruff_python_ast::Pattern) {
        let capture = match pattern {
            ruff_python_ast::Pattern::MatchAs(pattern) => pattern.name.as_ref(),
            ruff_python_ast::Pattern::MatchStar(pattern) => pattern.name.as_ref(),
            ruff_python_ast::Pattern::MatchMapping(pattern) => pattern.rest.as_ref(),
            _ => None,
        };
        if let Some(name) = capture {
            self.record_raw_name(name.as_str());
        }
    }

    pub(super) fn observe_type_param(&mut self, parameter: &'ast ruff_python_ast::TypeParam) {
        let name = parameter.name().as_str();
        if let Some(names) = &mut self.mangled_names {
            names.insert(name.to_string());
        }
        self.record_raw_name(name);
    }

    pub(super) fn observe_keyword(&mut self, keyword: &'ast ruff_python_ast::Keyword) {
        if let Some(name) = &keyword.arg {
            self.record_raw_name(name.as_str());
        }
    }

    pub(super) fn observe_alias(&mut self, alias: &'ast ruff_python_ast::Alias) {
        self.origins
            .entry(Cow::Borrowed(alias.name.as_str()))
            .or_default();
        for component in alias.name.as_str().split('.') {
            self.record_raw_name(component);
        }
        if let Some(name) = &alias.asname {
            self.record_raw_name(name.as_str());
        }
    }

    pub(super) fn observe_pattern_keyword(
        &mut self,
        keyword: &'ast ruff_python_ast::PatternKeyword,
    ) {
        self.record_raw_name(keyword.attr.as_str());
    }

    fn finish(
        mut self,
        generated_mangled_strings: &mut FxHashSet<String>,
        requires_interning_trace: &mut bool,
        static_attribute_names: &mut FxHashSet<String>,
        has_percent_replacement: &mut bool,
        finally_bodies: &mut FxHashMap<TextRange, &'ast [Stmt]>,
    ) -> (FxHashMap<Cow<'ast, str>, Cell<StringConstantOrigin>>, bool) {
        self.generated_mangled_strings
            .retain(|name| !self.raw_private_identifiers.contains(name.as_str()));
        *requires_interning_trace =
            self.restricted_private_identifier || !self.generated_mangled_strings.is_empty();
        *generated_mangled_strings = self.generated_mangled_strings;
        *static_attribute_names = self.static_attribute_names;
        *has_percent_replacement = self.has_percent_replacement;
        *finally_bodies = self.finally_bodies;
        (self.origins, self.requires_empty_bytes_cache_trace)
    }
}

#[cfg(test)]
impl<'ast> Visitor<'ast> for SourceAnalysis<'ast> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        self.observe_stmt(statement);
        if let Stmt::ClassDef(definition) = statement {
            for decorator in &definition.decorator_list {
                self.visit_decorator(decorator);
            }
            let previous = self.enter_class_header(definition);
            if let Some(type_params) = &definition.type_params {
                self.visit_type_params(type_params);
            }
            if let Some(arguments) = &definition.arguments {
                self.visit_arguments(arguments);
            }
            self.enter_class_body(definition);
            self.visit_body(&definition.body);
            self.leave_class(previous);
            return;
        }
        walk_stmt(self, statement);
    }

    fn visit_parameters(&mut self, parameters: &'ast ruff_python_ast::Parameters) {
        self.observe_parameters(parameters);
        ruff_python_ast::visitor::walk_parameters(self, parameters);
    }

    fn visit_parameter(&mut self, parameter: &'ast ruff_python_ast::Parameter) {
        self.observe_parameter(parameter);
        if let Some(annotation) = &parameter.annotation {
            self.visit_expr(annotation);
        }
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_expr(expression));
        }

        self.observe_expr(expression);
        match expression {
            Expr::Name(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::IpyEscapeCommand(_) => {}
            Expr::Attribute(attribute) => self.visit_expr(&attribute.value),
            _ => walk_expr(self, expression),
        }
    }

    fn visit_except_handler(&mut self, handler: &'ast ruff_python_ast::ExceptHandler) {
        self.observe_except_handler(handler);
        let ruff_python_ast::ExceptHandler::ExceptHandler(handler) = handler;
        if let Some(type_) = &handler.type_ {
            self.visit_expr(type_);
        }
        self.visit_body(&handler.body);
    }

    fn visit_pattern(&mut self, pattern: &'ast ruff_python_ast::Pattern) {
        self.observe_pattern(pattern);
        ruff_python_ast::visitor::walk_pattern(self, pattern);
    }

    fn visit_type_param(&mut self, parameter: &'ast ruff_python_ast::TypeParam) {
        self.observe_type_param(parameter);
        ruff_python_ast::visitor::walk_type_param(self, parameter);
    }

    fn visit_keyword(&mut self, keyword: &'ast ruff_python_ast::Keyword) {
        self.observe_keyword(keyword);
        self.visit_expr(&keyword.value);
    }

    fn visit_alias(&mut self, alias: &'ast ruff_python_ast::Alias) {
        self.observe_alias(alias);
    }

    fn visit_pattern_keyword(&mut self, keyword: &'ast ruff_python_ast::PatternKeyword) {
        self.observe_pattern_keyword(keyword);
        self.visit_pattern(&keyword.pattern);
    }
}

#[cfg(test)]
fn analyze_source<'ast>(
    body: &'ast Suite,
    python_version: crate::PythonVersion,
    name_capacity: usize,
    generated_mangled_strings: &mut FxHashSet<String>,
    requires_interning_trace: &mut bool,
    static_attribute_names: &mut FxHashSet<String>,
    has_percent_replacement: &mut bool,
    finally_bodies: &mut FxHashMap<TextRange, &'ast [Stmt]>,
) -> (FxHashMap<Cow<'ast, str>, Cell<StringConstantOrigin>>, bool) {
    let mut visitor = SourceAnalysis::new(
        python_version,
        if body.is_empty() { 0 } else { name_capacity },
    );
    visitor.visit_body(body);
    visitor.finish(
        generated_mangled_strings,
        requires_interning_trace,
        static_attribute_names,
        has_percent_replacement,
        finally_bodies,
    )
}

impl Compiler<'_> {
    pub(super) fn line_number(&self, offset: u32) -> u32 {
        self.source_line(self.source_offset(offset)).number
    }

    pub(super) fn source_location(&self, range: ruff_text_size::TextRange) -> SourceLocation {
        let optimized = self.context.optimized_locations.get(&range);
        if matches!(
            optimized,
            Some(super::optimization::OptimizedLocation::Elided)
        ) {
            return SourceLocation::NONE;
        }
        let location = self.physical_source_location(range);
        if matches!(
            optimized,
            Some(super::optimization::OptimizedLocation::SyntheticPass)
        ) {
            return SourceLocation::new(
                location.line,
                location.line,
                location.column,
                location.column.saturating_add(4),
            );
        }
        location
    }

    /// Locate setup instructions that precede the elision of their opening statement.
    pub(super) fn physical_source_location(
        &self,
        range: ruff_text_size::TextRange,
    ) -> SourceLocation {
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
        // The parser supplies token-aligned ranges for the definition and its name.
        let token_start = || {
            let tokens = self.context.tokens.in_range(ruff_text_size::TextRange::new(
                range.start(),
                name_range.start(),
            ));
            tokens
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
                })
        };
        let prefix: &[u8] = match (keyword, is_async) {
            (TokenKind::Def, false) => b"def ",
            (TokenKind::Def, true) => b"async def ",
            (TokenKind::Class, false) => b"class ",
            _ => &[],
        };
        let name_start = usize::from(name_range.start());
        let fast_start = name_start.checked_sub(prefix.len()).and_then(|start| {
            let preceding = self.context.source.as_bytes().get(start.wrapping_sub(1));
            (start >= usize::from(range.start())
                && !prefix.is_empty()
                && (start == usize::from(range.start())
                    || preceding.is_some_and(u8::is_ascii_whitespace))
                && self.context.source.as_bytes().get(start..name_start) == Some(prefix))
            .then(|| ruff_text_size::TextSize::try_from(start).unwrap())
        });
        let start = fast_start.unwrap_or_else(token_start);
        let (line, column) = self.source_position(u32::from(start));
        let (end_line, end_column) = self.definition_end_position(range);
        SourceLocation::new(line, end_line, column, end_column)
    }

    fn definition_end_position(&self, range: ruff_text_size::TextRange) -> (i32, i32) {
        let token_end = || {
            self.context
                .tokens
                .after(range.end())
                .first()
                .filter(|token| token.kind() == TokenKind::Semi)
                .map_or(range.end(), ruff_text_size::Ranged::end)
        };
        let tail = self
            .context
            .source
            .as_bytes()
            .get(usize::from(range.end())..);
        let fast_end = tail.and_then(|tail| {
            let blanks = tail
                .iter()
                .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0c'))
                .count();
            match tail.get(blanks) {
                None | Some(b'\n' | b'\r' | b'#') => Some(range.end()),
                Some(b';') => {
                    Some(range.end() + ruff_text_size::TextSize::try_from(blanks + 1).unwrap())
                }
                _ => None,
            }
        });
        let end = fast_end.unwrap_or_else(token_end);
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

    #[inline]
    fn source_line(&self, offset: TextSize) -> CachedSourceLine {
        let [recent, previous] = self.cached_source_lines.get();
        if let Some(line) = recent
            && line.contains(offset)
        {
            return line;
        }
        if let Some(line) = previous
            && line.contains(offset)
        {
            self.cached_source_lines.set([previous, recent]);
            return line;
        }

        self.uncached_source_line(offset, recent)
    }

    #[inline(never)]
    fn uncached_source_line(
        &self,
        offset: TextSize,
        recent: Option<CachedSourceLine>,
    ) -> CachedSourceLine {
        let line_starts = self.context.line_index.line_starts();
        let adjacent = recent.and_then(|line| {
            let current = usize::try_from(line.number).ok()?.checked_sub(1)?;
            if offset >= line.end && !line.is_last {
                let next = current + 1;
                if line_starts.get(next + 1).is_none_or(|end| offset < *end) {
                    return Some(next);
                }
            } else if offset < line.start {
                let previous = current.checked_sub(1)?;
                if offset >= line_starts[previous] {
                    return Some(previous);
                }
            }
            None
        });
        let line_index = adjacent
            .unwrap_or_else(|| self.context.line_index.line_index(offset).to_zero_indexed());
        let start = line_starts[line_index];
        let end = line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or_else(|| self.source_offset(u32::MAX));
        let cached = CachedSourceLine {
            number: u32::try_from(line_index + 1).unwrap_or(u32::MAX),
            start,
            end,
            is_last: line_index + 1 == line_starts.len(),
        };
        // A multiline range visits its start and end, then often returns to the start while
        // lowering its children. Retain both ends rather than replacing one with the other.
        self.cached_source_lines.set([Some(cached), recent]);
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
    #[inline]
    pub(super) fn source_aware_fold_constant(&self, expression: &Expr) -> Option<Constant> {
        match expression {
            Expr::StringLiteral(string) => Some(self.string_literal_constant(string)),
            Expr::NoneLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BytesLiteral(_) => literal_constant(expression),
            Expr::Name(name) => (name.ctx == ExprContext::Load && name.id.as_str() == "__debug__")
                .then_some(Constant::Bool(true)),
            Expr::FString(_)
            | Expr::Tuple(_)
            | Expr::Subscript(_)
            | Expr::Slice(_)
            | Expr::UnaryOp(_)
            | Expr::BinOp(_) => self.source_aware_fold_compound(expression),
            // Runtime-only expressions cannot fold, even when all their descendants are literals.
            _ => None,
        }
    }

    fn source_aware_fold_compound(&self, expression: &Expr) -> Option<Constant> {
        struct RequiresRuntimeConstruction(bool);

        impl<'a> Visitor<'a> for RequiresRuntimeConstruction {
            fn visit_expr(&mut self, expression: &'a Expr) {
                if crate::stack_growth_required() {
                    return crate::grow_stack(|| self.visit_expr(expression));
                }

                if matches!(expression, Expr::Slice(_) | Expr::FString(_)) {
                    self.0 = true;
                } else if !self.0 {
                    walk_expr(self, expression);
                }
            }
        }

        let literal = |expression: &Expr| match expression {
            Expr::UnaryOp(unary)
                if !self.has_deferred_annotations()
                    && unary.op == ruff_python_ast::UnaryOp::Invert =>
            {
                if let Some(Constant::Bool(value)) = fold_constant(&unary.operand) {
                    return Some(Constant::SignedInt(-i64::from(value) - 1));
                }
                None
            }
            Expr::UnaryOp(unary)
                if !self.has_deferred_annotations()
                    && unary.op == ruff_python_ast::UnaryOp::Not
                    && matches!(
                        unary.operand.as_ref(),
                        Expr::UnaryOp(_) | Expr::BinOp(_) | Expr::Subscript(_)
                    ) =>
            {
                // The legacy AST optimizer applies `not` after recursively folding its operand.
                self.source_aware_fold_constant(&unary.operand)
                    .map(|constant| Constant::Bool(!constant.is_truthy()))
            }

            Expr::StringLiteral(string) => Some(self.string_literal_constant(string)),
            Expr::FString(fstring) => {
                // Preserve the existing literal-only f-string folding gate.
                fold_constant(expression)?;
                match super::expression::fstring_components(fstring, self.context.source)
                    .into_iter()
                    .next()
                {
                    Some(super::expression::InterpolatedStringComponent::Literal(literal)) => {
                        Some(literal.into_constant())
                    }
                    None => Some(Constant::String(String::new())),
                    Some(super::expression::InterpolatedStringComponent::Interpolation(_, _)) => {
                        unreachable!("a folded f-string has no interpolated expressions")
                    }
                }
            }
            _ => literal_constant(expression),
        };
        let modern = self.context.python_version >= crate::PythonVersion::Py314;
        let constant = fold_constant_with_profile(
            expression,
            self.context.python_version,
            self.context.native_build_profile,
            &literal,
            modern,
        )?;

        // Modern folding already rejects `not <tuple>` recursively. The legacy AST optimizer
        // leaves slices and joined strings at runtime even where generic folding accepts them.
        if modern {
            return Some(constant);
        }
        let mut runtime = RequiresRuntimeConstruction(false);
        runtime.visit_expr(expression);
        (!runtime.0).then_some(constant)
    }

    pub(super) fn source_aware_constant_slice(
        &self,
        slice: &ruff_python_ast::ExprSlice,
    ) -> Option<Constant> {
        constant_slice_with_literals(slice, &|expression| match expression {
            Expr::StringLiteral(string) => Some(self.string_literal_constant(string)),
            _ => literal_constant(expression),
        })
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
        &self,
        fstring: &ruff_python_ast::ExprFString,
    ) -> ruff_text_size::TextRange {
        let mut component_count = 0;
        let mut pending_literal = false;
        let mut only_interpolation = None;
        for part in &fstring.value {
            match part {
                FStringPartRef::Literal(literal) => {
                    pending_literal |= !literal.value.is_empty();
                }
                FStringPartRef::FString(fstring) => {
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
                fstring_literal_span(fstring, self.context.source)
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
            return self.source_location(call.range());
        };
        let attribute_location = self.source_location(attribute.range);
        if attribute_location.line == attribute_location.end_line {
            return self.source_location(call.range());
        }
        self.call_opcode_location_with_method(call, self.call_shape(call).direct_method)
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
                    call.range().end(),
                ));
            }
        }
        self.source_location(call.range())
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

/// Exclude a final empty native f-string token from an otherwise nonempty literal.
pub(super) fn fstring_literal_range(
    source: &str,
    range: ruff_text_size::TextRange,
    is_raw: bool,
) -> ruff_text_size::TextRange {
    if is_raw {
        return range;
    }
    let mut text = &source[usize::from(range.start())..usize::from(range.end())];
    while let Some(prefix) = text
        .strip_suffix("\\\r\n")
        .or_else(|| text.strip_suffix("\\\n"))
        .or_else(|| text.strip_suffix("\\\r"))
    {
        text = prefix;
    }
    // In a valid literal, a final brace ends either a doubled brace or a named Unicode
    // escape. CPython's lexer ends FSTRING_MIDDLE there. The parser drops a following
    // token made entirely of line continuations, whereas Ruff includes it in this range.
    // Without that token boundary, the continuation belongs to the nonempty token.
    if matches!(text.as_bytes().last(), Some(b'{' | b'}')) {
        ruff_text_size::TextRange::at(range.start(), text.text_len())
    } else {
        range
    }
}

fn fstring_literal_span(
    fstring: &ruff_python_ast::ExprFString,
    source: &str,
) -> Option<ruff_text_size::TextRange> {
    let mut first = None;
    let mut last = None;
    let mut include = |range: ruff_text_size::TextRange| {
        first.get_or_insert(range.start());
        last = Some(range.end());
    };
    for part in &fstring.value {
        match part {
            FStringPartRef::Literal(literal) => include(literal.range),
            FStringPartRef::FString(fstring) => {
                for element in &fstring.elements {
                    match element {
                        InterpolatedStringElement::Literal(literal) => {
                            include(fstring_literal_range(
                                source,
                                literal.range,
                                fstring.flags.prefix().is_raw(),
                            ));
                        }
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
            Expr::FString(fstring) => compiler.fstring_result_range(fstring),
            expression
                if is_literal_constant(expression)
                    || (compiler.context.python_version <= crate::PythonVersion::Py313
                        && compiler.return_value_is_constant(expression)) =>
            {
                statement.range()
            }
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

pub(super) fn statement_execution_range(
    statement: &Stmt,
    python_version: crate::PythonVersion,
) -> ruff_text_size::TextRange {
    match statement {
        Stmt::Assign(statement) => {
            first_expression_instruction_range(&statement.value, python_version)
        }
        Stmt::AnnAssign(statement) => statement
            .value
            .as_deref()
            .map_or(statement.range, |expression| {
                first_expression_instruction_range(expression, python_version)
            }),
        Stmt::AugAssign(statement) => {
            first_expression_instruction_range(&statement.target, python_version)
        }
        Stmt::Expr(statement) => {
            first_expression_instruction_range(&statement.value, python_version)
        }
        Stmt::If(statement) => first_expression_instruction_range(&statement.test, python_version),
        Stmt::Return(statement) => statement
            .value
            .as_deref()
            .map_or(statement.range, |expression| {
                first_expression_instruction_range(expression, python_version)
            }),
        _ => statement.range(),
    }
}

pub(super) fn first_expression_instruction_range(
    expression: &Expr,
    python_version: crate::PythonVersion,
) -> ruff_text_size::TextRange {
    if crate::stack_growth_required() {
        return crate::grow_stack(|| {
            first_expression_instruction_range(expression, python_version)
        });
    }

    if matches!(
        expression,
        Expr::BinOp(_) | Expr::Name(_) | Expr::UnaryOp(_) | Expr::Tuple(_)
    ) && fold_constant_for_version(expression, python_version).is_some()
    {
        return expression.range();
    }
    match expression {
        Expr::Attribute(attribute) => {
            first_expression_instruction_range(&attribute.value, python_version)
        }
        Expr::BinOp(binary) => first_expression_instruction_range(&binary.left, python_version),
        Expr::BoolOp(boolean) => boolean
            .values
            .first()
            .map_or(expression.range(), |expression| {
                first_expression_instruction_range(expression, python_version)
            }),
        Expr::Call(call) => first_expression_instruction_range(&call.func, python_version),
        Expr::Compare(compare) => {
            first_expression_instruction_range(compare.first_operand(), python_version)
        }
        Expr::If(conditional) => {
            first_expression_instruction_range(&conditional.test, python_version)
        }
        Expr::Named(named) => first_expression_instruction_range(&named.value, python_version),
        Expr::Subscript(subscript) => {
            first_expression_instruction_range(&subscript.value, python_version)
        }
        Expr::UnaryOp(unary) => first_expression_instruction_range(&unary.operand, python_version),
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
    if let Some(annotation) = unparse_simple_annotation(expression, source) {
        return annotation;
    }
    let mut depth = crate::stack::ExpressionDepth::default();
    depth.visit_expr(expression);
    crate::stack::with_depth(depth.maximum(), || {
        unparse_annotation_inner(expression, source)
    })
}

/// Render common expressions without cloning or recursively unparsing the AST.
fn unparse_simple_annotation(expression: &Expr, source: &str) -> Option<String> {
    enum Part<'a> {
        Expression(&'a Expr, u8),
        Text(&'a str),
    }

    // These precedence values match Ruff's AST unparser.
    const SUBSCRIPT: u8 = 13;
    const COMMA: u8 = 21;
    const MAX: u8 = 63;

    if !crate::stack::is_simple_annotation(expression) {
        return None;
    }
    let mut pending = vec![Part::Expression(expression, 0)];
    let mut result = String::new();
    while let Some(part) = pending.pop() {
        match part {
            Part::Text(text) => result.push_str(text),
            Part::Expression(Expr::Name(name), _) => result.push_str(name.id.as_str()),
            Part::Expression(Expr::NoneLiteral(_), _) => result.push_str("None"),
            Part::Expression(Expr::BooleanLiteral(boolean), _) => {
                result.push_str(if boolean.value { "True" } else { "False" });
            }
            Part::Expression(Expr::EllipsisLiteral(_), _) => result.push_str("..."),
            Part::Expression(literal @ (Expr::NumberLiteral(_) | Expr::StringLiteral(_)), _) => {
                // The existing leaf renderer preserves source-dependent string escapes
                // and integer spellings without traversing the surrounding deep tree.
                result.push_str(&unparse_annotation_inner(literal, source));
            }
            Part::Expression(expression @ Expr::Attribute(attribute), _) => {
                if matches!(
                    attribute.value.as_ref(),
                    Expr::NumberLiteral(ruff_python_ast::ExprNumberLiteral {
                        value: ruff_python_ast::Number::Int(_),
                        ..
                    })
                ) {
                    result.push_str(&unparse_annotation_inner(expression, source));
                } else {
                    pending.push(Part::Text(attribute.attr.as_str()));
                    pending.push(Part::Text("."));
                    pending.push(Part::Expression(&attribute.value, MAX));
                }
            }
            Part::Expression(Expr::Subscript(subscript), _) => {
                pending.push(Part::Text("]"));
                pending.push(Part::Expression(&subscript.slice, SUBSCRIPT));
                pending.push(Part::Text("["));
                pending.push(Part::Expression(&subscript.value, MAX));
            }
            Part::Expression(Expr::Call(call), _) => {
                pending.push(Part::Text(")"));
                for (index, argument) in call.arguments.args.iter().enumerate().rev() {
                    pending.push(Part::Expression(argument, COMMA));
                    if index > 0 {
                        pending.push(Part::Text(", "));
                    }
                }
                pending.push(Part::Text("("));
                pending.push(Part::Expression(&call.func, MAX));
            }
            Part::Expression(Expr::BinOp(binary), level) => {
                let (operator, precedence) = match binary.op {
                    Operator::BitOr => (" | ", 39),
                    Operator::BitXor => (" ^ ", 41),
                    Operator::BitAnd => (" & ", 43),
                    Operator::LShift => (" << ", 45),
                    Operator::RShift => (" >> ", 45),
                    Operator::Add => (" + ", 47),
                    Operator::Sub => (" - ", 47),
                    Operator::Mult => (" * ", 49),
                    Operator::MatMult => (" @ ", 49),
                    Operator::Div => (" / ", 49),
                    Operator::Mod => (" % ", 49),
                    Operator::FloorDiv => (" // ", 49),
                    Operator::Pow => (" ** ", 55),
                };
                let right_associative = binary.op == Operator::Pow;
                let group = level > precedence;
                if group {
                    result.push('(');
                    pending.push(Part::Text(")"));
                }
                pending.push(Part::Expression(
                    &binary.right,
                    precedence + u8::from(!right_associative),
                ));
                pending.push(Part::Text(operator));
                pending.push(Part::Expression(
                    &binary.left,
                    precedence + u8::from(right_associative),
                ));
            }
            Part::Expression(Expr::UnaryOp(unary), level) => {
                let (operator, precedence) = match unary.op {
                    ruff_python_ast::UnaryOp::Not => ("not ", 35),
                    ruff_python_ast::UnaryOp::Invert => ("~", 53),
                    ruff_python_ast::UnaryOp::UAdd => ("+", 53),
                    ruff_python_ast::UnaryOp::USub => ("-", 53),
                };
                if level > precedence {
                    result.push('(');
                    pending.push(Part::Text(")"));
                }
                pending.push(Part::Expression(&unary.operand, precedence));
                pending.push(Part::Text(operator));
            }
            Part::Expression(_, _) => unreachable!("simple annotation was checked above"),
        }
    }
    Some(result)
}

fn unparse_annotation_inner(expression: &Expr, source: &str) -> String {
    #[derive(Default)]
    struct AnnotationStrings<'ast> {
        needs_normalization: bool,
        strings: Vec<&'ast ruff_python_ast::ExprStringLiteral>,
        fstrings: Vec<&'ast ruff_python_ast::ExprFString>,
        integer_attribute_values: Vec<&'ast ruff_python_ast::ExprNumberLiteral>,
        large_integer_values: Vec<&'ast ruff_python_ast::ExprNumberLiteral>,
        unpacked_keywords: Vec<&'ast ruff_python_ast::Keyword>,
        templates: Vec<&'ast ruff_python_ast::ExprTString>,
    }

    impl<'ast> Visitor<'ast> for AnnotationStrings<'ast> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }

            match expression {
                Expr::StringLiteral(string) => {
                    self.needs_normalization = true;
                    self.strings.push(string);
                }
                Expr::FString(string) => {
                    self.needs_normalization = true;
                    self.fstrings.push(string);
                }
                Expr::TString(string) => {
                    self.needs_normalization = true;
                    self.templates.push(string);
                }
                Expr::Attribute(attribute) => {
                    if let Expr::NumberLiteral(number) = attribute.value.as_ref()
                        && matches!(number.value, ruff_python_ast::Number::Int(_))
                    {
                        self.integer_attribute_values.push(number);
                    }
                    walk_expr(self, expression);
                }
                Expr::NumberLiteral(number) => {
                    if matches!(&number.value, ruff_python_ast::Number::Int(integer) if integer.as_u64().is_none())
                    {
                        self.large_integer_values.push(number);
                    }
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
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }

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
                    let end = call.range().end();
                    for keyword in &mut call.arguments.keywords {
                        keyword.range = ruff_text_size::TextRange::empty(end);
                    }
                }
            }
        }
    }

    if let Expr::StringLiteral(string) = expression {
        return annotation_string_literal_repr(string, source).0;
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
            .strings
            .iter()
            .zip(original_strings.strings)
            .filter(|(_, original)| original.value.to_str().contains('\u{fffd}'))
            .filter_map(|(generated, original)| {
                let (repr, has_surrogates) = annotation_string_literal_repr(original, source);
                if has_surrogates {
                    Some((std::ops::Range::<usize>::from(generated.range), repr))
                } else {
                    None
                }
            }),
    );
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
    // Ruff renders an integer attribute as `(1).real`; CPython uses `1 .real`.
    let mut integer_attribute_replacements = Vec::new();
    let mut integer_attribute_ranges = FxHashSet::default();
    for number in generated_strings.integer_attribute_values {
        let opening = usize::from(number.start()).saturating_sub(1);
        let closing = usize::from(number.end());
        if parenthesis_pairs.contains(&(opening, closing)) {
            let mut value = if let ruff_python_ast::Number::Int(integer) = &number.value
                && integer.as_u64().is_none()
            {
                let token = integer.to_string();
                if annotation_is_nondecimal_integer(&token) {
                    annotation_integer_to_decimal(&token)
                } else {
                    token
                }
            } else {
                annotation[usize::from(number.start())..closing].to_owned()
            };
            value.push(' ');
            integer_attribute_replacements.push((opening..closing + 1, value));
            integer_attribute_ranges.insert(number.range);
        }
    }
    // Ruff retains the original base for large integers; CPython emits decimal.
    let mut large_integer_replacements = Vec::new();
    for number in generated_strings.large_integer_values {
        if integer_attribute_ranges.contains(&number.range) {
            continue;
        }
        if let ruff_python_ast::Number::Int(integer) = &number.value {
            let token = integer.to_string();
            if annotation_is_nondecimal_integer(&token) {
                large_integer_replacements.push((
                    std::ops::Range::<usize>::from(number.range),
                    annotation_integer_to_decimal(&token),
                ));
            }
        }
    }
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
    let has_integer_edits =
        !integer_attribute_replacements.is_empty() || !large_integer_replacements.is_empty();
    replacements.extend(integer_attribute_replacements);
    replacements.extend(large_integer_replacements);
    let non_overlapping = if has_integer_edits {
        replacements.sort_unstable_by_key(|(range, _)| range.start);
        replacements
            .windows(2)
            .all(|pair| pair[0].0.end <= pair[1].0.start)
    } else {
        false
    };
    if non_overlapping {
        // Apply disjoint edits in one pass: deleting one parenthesis at a time would
        // repeatedly shift the remaining string for wide annotations.
        let mut corrected = String::with_capacity(annotation.len());
        let mut previous = 0;
        for (range, replacement) in replacements {
            corrected.push_str(&annotation[previous..range.start]);
            corrected.push_str(&replacement);
            previous = range.end;
        }
        corrected.push_str(&annotation[previous..]);
        annotation = corrected;
    } else {
        replacements.sort_unstable_by_key(|(range, _)| std::cmp::Reverse(range.start));
        for (range, replacement) in replacements {
            annotation.replace_range(range, &replacement);
        }
    }
    if matches!(expression, Expr::Tuple(tuple) if !tuple.elts.is_empty()) {
        annotation.insert(0, '(');
        annotation.push(')');
    }
    annotation
}

fn annotation_is_nondecimal_integer(token: &str) -> bool {
    ["0x", "0X", "0o", "0O", "0b", "0B"]
        .iter()
        .any(|prefix| token.starts_with(*prefix))
}

/// Format a nondecimal integer token using the decimal spelling CPython unparses.
fn annotation_integer_to_decimal(token: &str) -> String {
    use std::fmt::Write as _;

    const DECIMAL_BASE: u64 = 1_000_000_000;
    let mut decimal = vec![0_u32];
    for limb in super::constant::big_integer_digits(token).into_iter().rev() {
        let mut carry = u64::from(limb);
        for part in &mut decimal {
            let value = u64::from(*part) * (1 << 15) + carry;
            *part = u32::try_from(value % DECIMAL_BASE).expect("decimal limb fits in u32");
            carry = value / DECIMAL_BASE;
        }
        if carry != 0 {
            decimal.push(u32::try_from(carry).expect("decimal carry fits in u32"));
        }
    }
    let mut output = decimal
        .pop()
        .expect("the decimal representation is nonempty")
        .to_string();
    for part in decimal.into_iter().rev() {
        write!(output, "{part:09}").expect("writing to a string cannot fail");
    }
    output
}

/// These expressions cannot contain any of the constructs corrected after Ruff generates them.
fn annotation_needs_no_unparse_corrections(expression: &Expr) -> bool {
    if crate::stack_growth_required() {
        return crate::grow_stack(|| annotation_needs_no_unparse_corrections(expression));
    }
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
    let mut body = AnnotationStringBody::default();
    for part in &string.value {
        match part {
            FStringPartRef::Literal(literal) => {
                body.push_literal(
                    &literal.value,
                    source,
                    literal.range,
                    literal.flags.prefix().is_raw(),
                    true,
                );
            }
            FStringPartRef::FString(part) => {
                append_annotation_interpolations(
                    &mut body,
                    &part.elements,
                    source,
                    None,
                    part.flags.prefix().is_raw(),
                );
            }
        }
    }
    format!("f{}", body.repr())
}

/// Template interpolations retain their authored text, including grouping parentheses.
fn unparse_annotation_tstring(string: &ruff_python_ast::ExprTString, source: &str) -> String {
    let parsed = ruff_python_parser::parse_unchecked(
        &source[std::ops::Range::<usize>::from(string.range)],
        ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::ParenthesizedExpression),
    );
    let mut body = AnnotationStringBody::default();
    for part in &string.value {
        append_annotation_interpolations(
            &mut body,
            &part.elements,
            source,
            Some((parsed.tokens(), string.start())),
            part.flags.prefix().is_raw(),
        );
    }
    format!("t{}", body.repr())
}

fn append_annotation_interpolations(
    body: &mut AnnotationStringBody,
    elements: &[InterpolatedStringElement],
    source: &str,
    template_tokens: Option<(&Tokens, TextSize)>,
    is_raw: bool,
) {
    for element in elements {
        match element {
            InterpolatedStringElement::Literal(literal) => {
                body.push_literal(&literal.value, source, literal.range, is_raw, true);
            }
            InterpolatedStringElement::Interpolation(interpolation) => {
                if let Some(debug_text) = &interpolation.debug_text {
                    body.push_braced(&annotation_interpolation_text(debug_text.as_str()));
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
                    append_annotation_interpolations(body, &spec.elements, source, None, is_raw);
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

/// Keeps source-only surrogate identities until Python's string repr is emitted.
#[derive(Default)]
struct AnnotationStringBody {
    value: String,
    surrogates: Vec<(usize, u16)>,
}

impl AnnotationStringBody {
    fn push(&mut self, character: char) {
        self.value.push(character);
    }

    fn push_str(&mut self, value: &str) {
        self.value.push_str(value);
    }

    fn push_braced(&mut self, value: &str) {
        for character in value.chars() {
            self.push(character);
            if matches!(character, '{' | '}') {
                self.push(character);
            }
        }
    }

    fn push_literal(
        &mut self,
        value: &str,
        source: &str,
        range: TextRange,
        is_raw: bool,
        escape_braces: bool,
    ) {
        let restored = (!is_raw && value.contains('\u{fffd}'))
            .then(|| {
                explicit_surrogate_string(value, &source[std::ops::Range::<usize>::from(range)])
            })
            .flatten();
        // Ruff's replacement character and a surrogate-pass UTF-8 sequence are both
        // three bytes, so their offsets in the restored and decoded values agree.
        for (offset, character) in value.char_indices() {
            if character == '\u{fffd}'
                && let Some([0xed, second @ 0xa0..=0xbf, third @ 0x80..=0xbf]) = restored
                    .as_deref()
                    .and_then(|bytes| bytes.get(offset..offset + 3))
            {
                let surrogate = 0xd000 | (u16::from(second & 0x3f) << 6) | u16::from(third & 0x3f);
                self.surrogates.push((self.value.len(), surrogate));
            }
            self.push(character);
            if escape_braces && matches!(character, '{' | '}') {
                self.push(character);
            }
        }
    }

    fn repr(&self) -> String {
        let repr = python_string_repr(&self.value);
        if self.surrogates.is_empty() {
            return repr;
        }
        // U+FFFD is printable in Python repr, so its occurrences remain in order
        // after quote selection and escaping. Replace only source surrogate entries.
        let mut surrogates = self.surrogates.iter().copied().peekable();
        let mut replacements = self
            .value
            .char_indices()
            .filter(|(_, character)| *character == '\u{fffd}')
            .map(|(offset, _)| {
                if surrogates.peek().is_some_and(|(index, _)| *index == offset) {
                    surrogates.next().map(|(_, surrogate)| surrogate)
                } else {
                    None
                }
            });
        let mut output = String::with_capacity(repr.len());
        for character in repr.chars() {
            if character == '\u{fffd}'
                && let Some(Some(surrogate)) = replacements.next()
            {
                use std::fmt::Write;
                write!(output, "\\u{surrogate:04x}").expect("writing to a string succeeds");
            } else {
                output.push(character);
            }
        }
        output
    }
}

fn annotation_string_literal_repr(
    string: &ruff_python_ast::ExprStringLiteral,
    source: &str,
) -> (String, bool) {
    let (mut repr, has_surrogates) = if string.value.to_str().contains('\u{fffd}') {
        let mut body = AnnotationStringBody::default();
        for literal in &string.value {
            body.push_literal(
                literal.as_str(),
                source,
                literal.range,
                literal.flags.prefix().is_raw(),
                false,
            );
        }
        (body.repr(), !body.surrogates.is_empty())
    } else {
        (python_string_repr(string.value.to_str()), false)
    };
    if string.value.is_unicode()
        && string.value.iter().next().is_some_and(|literal| {
            source.as_bytes().get(usize::from(literal.range.start())) == Some(&b'u')
        })
    {
        repr.insert(0, 'u');
    }
    (repr, has_surrogates)
}

fn python_string_repr(value: &str) -> String {
    ruff_python_literal::escape::UnicodeEscape::new_repr(value)
        .str_repr(ruff_python_ast::str::TripleQuotes::No)
        .to_string()
        .expect("Python string representation length fits in usize")
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    #[test]
    fn source_aware_atomic_folding_preserves_literal_values_and_surrogates() {
        use crate::PythonVersion::{Py312, Py313, Py314, Py315};

        for (source, expected) in [
            ("None", Constant::None),
            ("False", Constant::Bool(false)),
            ("...", Constant::Ellipsis),
            ("42", Constant::Int(42)),
            ("-2**2", Constant::SignedInt(-4)),
            ("(-2)**2", Constant::Int(4)),
            ("'a' 'b'", Constant::String("ab".into())),
            ("b'a' b'\\xff'", Constant::Bytes(vec![b'a', 0xff])),
            (
                "'a' '\\ud800'",
                Constant::SurrogateString(vec![b'a', 0xed, 0xa0, 0x80]),
            ),
            ("r'\\ud800'", Constant::String(r"\ud800".into())),
            ("'\\ufffd'", Constant::String("\u{fffd}".into())),
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            let [Stmt::Expr(statement)] = parsed.suite().as_slice() else {
                unreachable!("test cases are expressions");
            };
            for version in [Py312, Py313, Py314, Py315] {
                let compiler = Compiler::module_with_python_version(
                    "fold.py",
                    source,
                    parsed.tokens(),
                    parsed.suite(),
                    version,
                );
                let actual = compiler
                    .source_aware_fold_constant(&statement.value)
                    .expect("literal should fold");
                assert_eq!(
                    std::mem::discriminant(&actual),
                    std::mem::discriminant(&expected),
                    "{version:?} {source}: {actual:?}",
                );
                assert!(
                    crate::compiler::constant::python_constant_values_equal(&actual, &expected),
                    "{version:?} {source}: {actual:?}",
                );
            }
        }
    }

    #[test]
    fn source_aware_folding_keeps_target_runtime_expressions() {
        use crate::PythonVersion::{Py312, Py313, Py314, Py315};

        for (source, legacy, modern) in [
            ("__debug__", true, true),
            ("name", false, false),
            ("(1, 2).real", false, false),
            ("[1, 2]", false, false),
            ("{1, 2}", false, false),
            ("{'a': 1}", false, false),
            ("1 == 2", false, false),
            ("1 and 2", false, false),
            ("1 if 2 else 3", false, false),
            ("lambda: 1", false, false),
            ("[x for x in (1, 2)]", false, false),
            ("'abc'[1]", true, true),
            ("('abc',)[0]", true, true),
            ("'abc'[1:]", false, true),
            ("('abc'[1:],)", false, true),
            ("f'abc'", false, true),
            ("(f'abc',)", false, true),
            ("f'{value}'", false, false),
            ("not f'abc'", false, true),
            ("not f'{(not ())}'", false, false),
            ("not ()", true, false),
            ("((not ()),)", true, false),
            ("('a', 'b')[not ()]", true, false),
            ("('a', 'b')[(not (),)[0]]", true, false),
            ("1 + (not ())", true, false),
            ("~True", true, false),
            ("'\\ud800' + 'x'", true, true),
            ("helper(not (), f'abc', 'abc'[1:])", false, false),
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            let [Stmt::Expr(statement)] = parsed.suite().as_slice() else {
                unreachable!("test cases are expressions");
            };
            for (version, expected) in [
                (Py312, legacy),
                (Py313, legacy),
                (Py314, modern),
                (Py315, modern),
            ] {
                let compiler = Compiler::module_with_python_version(
                    "fold.py",
                    source,
                    parsed.tokens(),
                    parsed.suite(),
                    version,
                );
                let actual = compiler.source_aware_fold_constant(&statement.value);
                assert_eq!(
                    actual.is_some(),
                    expected,
                    "{version:?} {source}: {actual:?}"
                );
            }
        }
    }

    #[test]
    fn empty_bytes_cache_trace_only_folds_possible_byte_sequences() {
        use crate::PythonVersion::{Py313, Py314, Py315};

        for (source, possible_bytes, requires_trace) in [
            ("b'abc' * 0", true, true),
            ("False * (b'a' + b'b')", true, true),
            ("(b'abc'[1:] + b'd') * 0", true, true),
            ("((1,), (b'a',))[1][0] * 0", true, true),
            ("((0,) + (b'a',))[1] * 0", true, true),
            ("((b'a',) * 2)[1] * 0", true, true),
            ("b'' * -1", true, true),
            ("b'a' * 2", true, false),
            ("(b'a',) * 0", true, false),
            ("(1, b'a')[0] * 0", true, false),
            ("b'a'[0] * 0", true, false),
            ("42 * (12 + 13)", false, false),
            ("'abc' * 0", false, false),
            ("f'{b\"a\"}' * 0", false, false),
            ("method(b'abc') * 0", false, false),
            ("(1, 2)[b'abc'[0]] * 0", false, false),
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            let [Stmt::Expr(statement)] = parsed.suite().as_slice() else {
                unreachable!("test cases are expressions");
            };
            assert_eq!(can_fold_bytes(&statement.value), possible_bytes, "{source}");
            for version in [Py313, Py314, Py315] {
                let context = CompilationContext::with_python_version(
                    "bytes.py".into(),
                    source,
                    parsed.tokens(),
                    parsed.suite(),
                    version,
                );
                assert_eq!(
                    context.requires_empty_bytes_cache_trace,
                    requires_trace && version >= Py314,
                    "{version:?} {source}",
                );
            }
        }
    }

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
            "class Café:\n    def f(self, __parameter: int):\n        self.__secret\n        self.__already\n        self._Café__already\n        self.élan\n        self.ends_\n",
        )
        .unwrap();
        let mut generated = FxHashSet::default();
        let mut requires_trace = false;
        let (origins, _) = analyze_source(
            parsed.suite(),
            crate::PythonVersion::Py314,
            0,
            &mut generated,
            &mut requires_trace,
            &mut FxHashSet::default(),
            &mut false,
            &mut FxHashMap::default(),
        );
        for raw in [
            "Café",
            "__secret",
            "__already",
            "__parameter",
            "_Café__already",
            "élan",
            "ends_",
        ] {
            let (key, _) = origins.get_key_value(raw).expect("raw name is recorded");
            assert!(matches!(key, Cow::Borrowed(_)), "{raw:?}");
        }
        let (key, _) = origins.get_key_value("_Café__parameter").unwrap();
        assert!(matches!(key, Cow::Owned(_)));
        assert!(generated.contains("_Café__secret"));
        assert!(generated.contains("_Café__parameter"));
        assert!(!generated.contains("_Café__already"));
        assert!(requires_trace);
    }

    #[test]
    fn source_analysis_visits_names_inside_attributes_and_interpolated_literals() {
        let source = r#"
self.attribute = receiver.inner.outer
value = ("ordinary" " adjacent", b"literal", 3, True, None, ...)
formatted = f"text {inside.member}"
templated = t"text {template.member}"
percent = "%%" % operand
"#;
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let mut attributes = FxHashSet::default();
        let mut percent = false;
        let (origins, empty_bytes) = analyze_source(
            parsed.suite(),
            crate::PythonVersion::Py314,
            32,
            &mut FxHashSet::default(),
            &mut false,
            &mut attributes,
            &mut percent,
            &mut FxHashMap::default(),
        );
        for name in [
            "self",
            "attribute",
            "receiver",
            "inner",
            "outer",
            "inside",
            "member",
            "template",
            "operand",
        ] {
            assert!(origins.contains_key(name), "{name}");
        }
        for literal in ["ordinary adjacent", "literal", "text "] {
            assert!(!origins.contains_key(literal), "{literal}");
        }
        assert_eq!(attributes, FxHashSet::from_iter(["attribute".to_owned()]));
        assert!(percent);
        assert!(!empty_bytes);
    }

    #[test]
    fn combined_source_and_scope_walk_matches_independent_source_walk() {
        use crate::PythonVersion::{Py312, Py313, Py314, Py315};

        let source = r#"
_Before__x
_Before__x.attr
__shared
prefix__suffix
import alpha.beta as __module
from __origin.inner import __member as __alias
@decorate(__outside)
class __Outer[__T: __bound = __default](factory(__T, __external, __key=__value)):
    __field: __type = __value
    @decorate(__enclosing)
    class __Inner[__U: __T](factory(__U, __other)):
        def method[__V](self, __argument: __T, *, __option=__fallback) -> __result:
            nonlocal __field
            global __global
            self.__attribute = __receiver.__middle.__leaf
            self.__attribute += __increment
            try:
                __operation()
            except __Error as __caught:
                __handler(__caught)
            finally:
                __cleanup()
            match __subject:
                case __Point(__pattern=__capture, value=[*__rest]):
                    pass
                case {"key": __entry, **__remaining}:
                    pass
            type __Alias[__W] = tuple[__W, __V]
            return lambda __lambda=__fallback: __lambda.__attribute
        __ordinary = (__item.__element for __item in __iterable if __item)
try:
    text = "%%" % __operand
    empty = b"value" * 0
except __RootError as __root_caught:
    __root_handler(__root_caught)
finally:
    __root_cleanup()
def flat():
    __flat_receiver.__flat_attribute += __flat_increment
    try:
        __flat_operation()
    except __FlatError as __flat_caught:
        __flat_handler(__flat_caught)
    return __flat_receiver.__flat_inner.__flat_leaf
class Before:
    __x: object
    self.__x
class After:
    __x: object
    self.__x
_After__x
_After__x.attr
class RepeatOuter[__T: (__shared, prefix__suffix)]:
    self.__shared = __shared
    class RepeatInner[__T: (__shared, prefix__suffix)]:
        self.__shared = __shared
        self.__shared
    self.__shared
"#;
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        for version in [Py312, Py313, Py314, Py315] {
            let mut reference = SourceAnalysis::new(version, 0);
            reference.visit_body(parsed.suite());
            let mut combined = SourceAnalysis::new(version, 0);
            ScopeArena::analyze_with_source(parsed.suite(), version, &mut combined);

            let origin_spellings = |analysis: &SourceAnalysis<'_>| {
                analysis
                    .origins
                    .iter()
                    .map(|(name, origin)| {
                        (
                            name.to_string(),
                            (matches!(name, Cow::Owned(_)), origin.get()),
                        )
                    })
                    .collect::<std::collections::BTreeMap<_, _>>()
            };
            assert_eq!(origin_spellings(&combined), origin_spellings(&reference));
            for (spelling, owned) in [("_Before__x", false), ("_After__x", true)] {
                let (name, _) = combined.origins.get_key_value(spelling).unwrap();
                assert_eq!(matches!(name, Cow::Owned(_)), owned, "{spelling}");
            }
            for generated in ["_RepeatOuter__shared", "_RepeatInner__shared"] {
                assert!(combined.generated_mangled_strings.contains(generated));
            }
            assert!(combined.static_attribute_names.contains("__shared"));
            assert_eq!(combined.finally_bodies, reference.finally_bodies);
            assert_eq!(
                combined.generated_mangled_strings,
                reference.generated_mangled_strings
            );
            assert_eq!(
                combined.raw_private_identifiers,
                reference.raw_private_identifiers
            );
            assert_eq!(
                combined.restricted_private_identifier,
                reference.restricted_private_identifier
            );
            assert_eq!(
                combined.static_attribute_names,
                reference.static_attribute_names
            );
            assert_eq!(
                combined.has_percent_replacement,
                reference.has_percent_replacement
            );
            assert_eq!(
                combined.requires_empty_bytes_cache_trace,
                reference.requires_empty_bytes_cache_trace
            );
        }
    }

    #[test]
    fn repeated_expression_name_still_records_restricted_private_context() {
        let source = "prefix__suffix\nclass Restricted[T: prefix__suffix]:\n    pass\n";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let mut combined = SourceAnalysis::new(crate::PythonVersion::Py314, 0);
        ScopeArena::analyze_with_source(parsed.suite(), crate::PythonVersion::Py314, &mut combined);
        let mut reference = SourceAnalysis::new(crate::PythonVersion::Py314, 0);
        reference.visit_body(parsed.suite());
        assert!(combined.restricted_private_identifier);
        assert_eq!(
            combined.restricted_private_identifier,
            reference.restricted_private_identifier
        );
        assert!(combined.raw_private_identifiers.contains("prefix__suffix"));
    }

    #[test]
    fn expression_name_index_caches_private_spelling_across_id_words_and_contexts() {
        let mut source = String::new();
        for index in 0..140 {
            writeln!(source, "plain_{index}.member").unwrap();
        }
        source.push_str(
            "__private\nprefix__suffix\nclass First[T: prefix__suffix]:\n    self.__private\nclass Second:\n    self.__private\nplain_139.member\n",
        );
        let parsed = ruff_python_parser::parse_module(&source).unwrap();
        let mut analysis = SourceAnalysis::new(crate::PythonVersion::Py314, 0);
        let arena = ScopeArena::analyze_with_source(
            parsed.suite(),
            crate::PythonVersion::Py314,
            &mut analysis,
        );
        let cached = |name: &str| {
            let id = arena.names.get(name).unwrap().index();
            let word = &analysis.expression_origin_names[id / 64];
            let bit = 1 << (id % 64);
            (word.seen & bit != 0, word.private & bit != 0)
        };
        for index in 0..140 {
            assert_eq!(cached(&format!("plain_{index}")), (true, false));
        }
        assert_eq!(cached("member"), (true, false));
        assert_eq!(cached("__private"), (true, true));
        assert_eq!(cached("prefix__suffix"), (true, true));
        assert!(analysis.restricted_private_identifier);
        assert_eq!(
            analysis.generated_mangled_strings,
            FxHashSet::from_iter(["_First__private".to_owned(), "_Second__private".to_owned()])
        );
    }

    #[test]
    fn finishing_module_string_provenance_moves_exclusive_keys() {
        let tokens = Tokens::new(Vec::new());
        let body = Suite::new();
        let mut context = CompilationContext::new("example.py", "", &tokens, &body);
        context.constant_string_origins = [
            ("unused", StringConstantOrigin::Unrecorded),
            ("not_interned", StringConstantOrigin::Uninterned),
            ("canonical", StringConstantOrigin::Interned),
            ("mangled_unused", StringConstantOrigin::Unrecorded),
            ("mangled_not_interned", StringConstantOrigin::Uninterned),
            ("mangled_canonical", StringConstantOrigin::Interned),
        ]
        .into_iter()
        .map(|(name, state)| (Cow::Owned(name.to_string()), Cell::new(state)))
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
        context.record_interning_event(CompilationInterningEvent::CachedTuple(vec![
            "earlier".to_string(),
        ]));
        let mut context = Rc::new(context);
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

        let moved = CompilationContext::finish_constant_string_provenance(&mut context, true);
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
        assert_eq!(context.interning_events.borrow().len(), 1);
        assert_eq!(
            context.record_interning_event(CompilationInterningEvent::CachedTuple(Vec::new())),
            1
        );
    }

    #[test]
    fn finishing_without_frozensets_keeps_only_canonical_provenance() {
        let tokens = Tokens::new(Vec::new());
        let body = Suite::new();
        let mut context = CompilationContext::new("example.py", "", &tokens, &body);
        context.constant_string_origins = [
            (Cow::Borrowed("unused"), StringConstantOrigin::Unrecorded),
            (
                Cow::Borrowed("not_interned"),
                StringConstantOrigin::Uninterned,
            ),
            (Cow::Borrowed("canonical"), StringConstantOrigin::Interned),
            (
                Cow::Owned("mangled_canonical".to_string()),
                StringConstantOrigin::Interned,
            ),
        ]
        .into_iter()
        .map(|(name, state)| (name, Cell::new(state)))
        .collect();
        context
            .generated_mangled_strings
            .insert("mangled_canonical".to_string());
        let mut context = Rc::new(context);
        let (identifiers, canonical) =
            CompilationContext::finish_constant_string_provenance(&mut context, false);
        assert!(identifiers.is_empty());
        assert_eq!(
            canonical,
            ["canonical".to_string(), "mangled_canonical".to_string()]
                .into_iter()
                .collect()
        );
    }
}
