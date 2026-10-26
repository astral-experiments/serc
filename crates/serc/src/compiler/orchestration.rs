use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use ruff_python_ast::token::Tokens;
use ruff_python_ast::{Expr, Stmt, Suite};
use ruff_text_size::Ranged;
use ruff_text_size::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    BUILD_SET, COPY, COPY_FREE_VARS, GET_AITER, GET_ITER, LOAD_CONST, LOAD_FAST, LOAD_LOCALS,
    LOAD_SMALL_INT, MAKE_CELL, MAKE_FUNCTION, NOP, POP_TOP, RESUME, RETURN_GENERATOR, RETURN_VALUE,
    SETUP_ANNOTATIONS, STORE_DEREF,
};
use crate::assembler::{AssembledCode, Assembler, ConstantTable, SourceLocation};
use crate::code_object::{
    CodeFilename, CodeObject, Constant, LocalsIdentity, MarshalMetadataIds, MarshalProvenance,
    MetadataId, MetadataName,
};
use crate::target::code_flags::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_FUTURE_ANNOTATIONS, CO_GENERATOR, CO_HAS_DOCSTRING,
    CO_NEWLOCALS, CO_OPTIMIZED, CO_VARARGS, CO_VARKEYWORDS,
};
use crate::target::local_kinds::{
    CO_FAST_ARG_KW, CO_FAST_ARG_POS, CO_FAST_ARG_VAR, CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN,
    CO_FAST_LOCAL,
};
use crate::target::operands::{FunctionAttribute, ResumeLocation, UnaryIntrinsic};
use crate::{CompileError, PythonVersion};

use super::constant::{DeferredConstantTarget, OutputState, fold_constant, literal_constant};
use super::definition::clean_doc;
use super::expression::early_condition_truthiness;
use super::scope::{
    LoweringFunctionPlan, NameId, NameInterner, NameSet, ScopeId,
    collect_conditional_simple_annotations, collect_simple_annotations, has_annotations,
};
use super::source::{CompilationContext, OriginalScopes, implicit_return_range};
use super::statement::{
    ControlFlowState, suite_terminates, terminal_statement_uses_implicit_return_location,
};
use super::symbol::{ReraiseStackState, ResumeDepth, SymbolState, to_u32};
use super::{Compiler, Scope};

/// Orders fast locals ahead of sorted cell-only locals while reserving the later free slots.
#[inline]
fn partition_function_locals(
    plan: &LoweringFunctionPlan,
    parameter_count: usize,
    names: &NameInterner,
) -> (Vec<NameId>, usize) {
    let mut locals = Vec::with_capacity(plan.locals.len().saturating_add(plan.freevars.len()));
    if plan.cellvars.is_empty() {
        locals.extend_from_slice(&plan.locals);
        let fast_local_count = locals.len();
        return (locals, fast_local_count);
    }

    let mut cell_only_locals = Vec::new();
    for (index, &name) in plan.locals.iter().enumerate() {
        if index >= parameter_count
            && plan.cellvars.contains(&name)
            && !plan.inlined_comprehension_cellvars.contains(&name)
        {
            cell_only_locals.push(name);
        } else {
            locals.push(name);
        }
    }
    let fast_local_count = locals.len();
    cell_only_locals.sort_by(|left, right| {
        let spelling = |name| {
            plan.cell_sort_names
                .get(&name)
                .map_or_else(|| names.resolve(name), String::as_str)
        };
        spelling(*left).cmp(spelling(*right))
    });
    locals.extend(cell_only_locals);
    (locals, fast_local_count)
}

/// Proves that the surviving constants cannot be changed by interning a metadata tuple.
///
/// Every emitted name and local has an interner ID, including metadata from discarded code.
/// An absent spelling cannot alias metadata; a present spelling conservatively leaves the
/// complete tuple comparison to marshal, even when that name was never emitted.
fn metadata_alias_scan_unnecessary(code: &CodeObject, names: &NameInterner) -> bool {
    fn constants_cannot_alias(values: &[Constant], names: &NameInterner) -> bool {
        if values
            .iter()
            .all(|value| matches!(value, Constant::String(_)))
        {
            return values.iter().all(|value| {
                let Constant::String(value) = value else {
                    unreachable!("all-string tuple");
                };
                // Ordinary ASCII identifier strings are already interned independently.
                value
                    .bytes()
                    .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
                    || names.get(value).is_none()
            });
        }
        values
            .iter()
            .all(|value| constant_cannot_alias(value, names))
    }

    fn constant_cannot_alias(value: &Constant, names: &NameInterner) -> bool {
        match value {
            Constant::Tuple(values) => constants_cannot_alias(values, names),
            Constant::FrozenSet(values) => values
                .iter()
                .all(|value| constant_cannot_alias(value, names)),
            Constant::Code(code) => constants_cannot_alias(&code.constants, names),
            Constant::Slice { lower, upper, step } => [lower, upper, step]
                .into_iter()
                .all(|value| constant_cannot_alias(value, names)),
            _ => true,
        }
    }

    constants_cannot_alias(&code.constants, names)
}

#[derive(Clone, Debug)]
pub(super) struct CodeUnit {
    pub(super) scope: Scope,
    pub(super) scope_id: ScopeId,
    pub(super) generic_target_qualified_name: Option<String>,
    pub(super) child_qualified_name_parent: Option<String>,
    pub(super) class_scope_is_nested: bool,
    pub(super) module_annotation_index: u32,
    pub(super) module_annotation_indices: FxHashMap<TextRange, u32>,
    pub(super) class_conditional_annotations: VecDeque<TextRange>,
    pub(super) annotation_classdict_index: Option<u32>,
    name: String,
    pub(super) qualified_name: String,
    pub(super) private_name: Option<String>,
    pub(super) mangled_names: Option<FxHashSet<String>>,
    pub(super) free_names_are_final: bool,
    pub(super) pending_private_type_parameters: FxHashSet<NameId>,
    /// Plans use declaration-time generic-header symbol identities, while
    /// instructions look up names with the completed mangling restriction.
    pub(super) header_symbol_names_are_final: bool,
    pub(super) arg_count: u32,
    positional_only_arg_count: u32,
    keyword_only_arg_count: u32,
    pub(super) flags: u32,
    /// A synchronous function marked as a coroutine by an assertion removed under `-O`.
    pub(super) legacy_sync_coroutine: bool,
    pub(super) first_line_number: u32,
    pub(super) annotation_thunk: bool,
}

#[derive(Debug)]
struct CodeUnitSpec {
    scope: Scope,
    scope_id: ScopeId,
    name: String,
    qualified_name: String,
    private_name: Option<String>,
    arg_count: u32,
    positional_only_arg_count: u32,
    keyword_only_arg_count: u32,
    flags: u32,
    first_line_number: u32,
}

/// Records constants created by late assembler passes at their cache-admission point.
struct InterningConstantTable<'a, 'source> {
    output: &'a mut OutputState,
    context: &'a CompilationContext<'source>,
}

impl ConstantTable for InterningConstantTable<'_, '_> {
    fn get(&self, index: u32) -> &Constant {
        self.output.get(index)
    }

    fn intern(&mut self, constant: Constant) -> Result<u32, CompileError> {
        self.context
            .record_constant_string_origins(&constant, false);
        self.output.intern(constant)
    }

    fn reallocate_non_reflexive_constant(
        &mut self,
        index: u32,
        remap: &mut crate::code_object::NonReflexiveRemap,
    ) -> Result<u32, CompileError> {
        self.output.reallocate_non_reflexive_constant(index, remap)
    }

    fn fold_sequence(&mut self, values: Vec<Constant>, is_set: bool) -> Result<u32, CompileError> {
        let index = self.output.fold_sequence(values, is_set)?;
        self.context
            .record_constant_string_origins(self.output.get(index), false);
        Ok(index)
    }

    fn fold_binary_sequence(
        &mut self,
        left: Constant,
        argument: u32,
        right: Constant,
    ) -> Result<Option<u32>, CompileError> {
        let index = self.output.fold_binary_sequence(left, argument, right)?;
        if let Some(index) = index {
            if argument == crate::target::operands::BinaryOperation::MULTIPLY.argument()
                && matches!(self.output.get(index), Constant::Bytes(value) if value.is_empty())
            {
                self.context.record_empty_bytes_origin(true);
            }
            self.context
                .record_constant_string_origins(self.output.get(index), false);
        }
        Ok(index)
    }

    fn len(&self) -> usize {
        self.output.len()
    }

    fn retain_used_in_order(&mut self, used: &[bool]) -> Vec<Option<u32>> {
        self.output.retain_used_in_order(used)
    }

    fn retain_used(
        &mut self,
        used: &[bool],
        order: &[u32],
        early_booleans: &[bool],
    ) -> Vec<Option<u32>> {
        self.output.retain_used(used, order, early_booleans)
    }
}

impl<'source> Compiler<'source> {
    /// Keep each compiler in place across fallible construction, body lowering, and assembly.
    fn new(
        context: Rc<CompilationContext<'source>>,
        unit: CodeUnit,
        symbols: SymbolState,
    ) -> Box<Self> {
        let allocation = Box::new_uninit();
        let assembler = Assembler::with_python_version(context.python_version);
        Box::write(
            allocation,
            Self {
                context,
                cached_source_lines: Cell::new([None; 2]),
                unit,
                output: OutputState::default(),
                symbols,
                control: ControlFlowState::default(),
                assembler,
                native_label_plan: None,
                legacy_short_label_prefix: Default::default(),
            },
        )
    }

    /// Inherit the enclosing scope's name mangling and imported-name context.
    pub(super) fn inherit_lexical_context(&mut self, parent: &Self) {
        self.unit.private_name.clone_from(&parent.unit.private_name);
        self.unit
            .mangled_names
            .clone_from(&parent.unit.mangled_names);
        self.unit
            .pending_private_type_parameters
            .clone_from(&parent.unit.pending_private_type_parameters);
        self.unit.header_symbol_names_are_final = parent.unit.header_symbol_names_are_final;
        self.symbols
            .imported_scope_names
            .clone_from(&parent.symbols.imported_scope_names);
    }

    /// Shares lexical context while keeping discarded parent instructions in a fresh emitter.
    pub(super) fn discarded_builder(&self) -> Box<Self> {
        Self::new(
            Rc::clone(&self.context),
            self.unit.clone(),
            self.symbols.clone(),
        )
    }

    #[cfg(test)]
    pub(crate) fn module(
        filename: &str,
        source: &'source str,
        tokens: &'source Tokens,
        body: &'source Suite,
    ) -> Box<Self> {
        Self::module_with_python_version(
            filename,
            source,
            tokens,
            body,
            crate::PythonVersion::Py314,
        )
    }

    #[cfg(test)]
    pub(crate) fn module_with_python_version(
        filename: impl Into<CodeFilename>,
        source: &'source str,
        tokens: &'source Tokens,
        body: &'source Suite,
        python_version: crate::PythonVersion,
    ) -> Box<Self> {
        Self::module_with_scopes(
            filename,
            source,
            tokens,
            body,
            None,
            python_version,
            crate::NativeBuildProfile::Default,
            None,
        )
    }

    pub(crate) fn module_with_scopes(
        filename: impl Into<CodeFilename>,
        source: &'source str,
        tokens: &'source Tokens,
        body: &'source Suite,
        original_scopes: Option<OriginalScopes>,
        python_version: crate::PythonVersion,
        native_build_profile: crate::NativeBuildProfile,
        optimized: Option<&super::optimization::OptimizedSuite>,
    ) -> Box<Self> {
        let mut context = CompilationContext::with_scopes(
            filename.into(),
            source,
            tokens,
            body,
            original_scopes,
            python_version,
            native_build_profile,
        );
        if let Some(optimized) = optimized {
            context.optimized_locations.clone_from(&optimized.locations);
        }
        let context = Rc::new(context);
        let future_flags = context.module_plan().future_flags;
        let unit = CodeUnit::from_spec(CodeUnitSpec {
            scope: Scope::Module,
            scope_id: ScopeId::MODULE,
            name: "<module>".to_string(),
            qualified_name: "<module>".to_string(),
            private_name: None,
            arg_count: 0,
            positional_only_arg_count: 0,
            keyword_only_arg_count: 0,
            flags: future_flags,
            first_line_number: 1,
        });
        Self::new(context, unit, SymbolState::default())
    }

    pub(super) fn function(
        context: Rc<CompilationContext<'source>>,
        name: &str,
        qualified_name: String,
        first_line_number: u32,
        plan: &LoweringFunctionPlan,
        scope_id: ScopeId,
        arg_count: u32,
        positional_only_arg_count: u32,
        keyword_only_arg_count: u32,
        parameter_flags: u32,
    ) -> Result<Box<Self>, CompileError> {
        let parameter_count = usize::try_from(
            arg_count
                + keyword_only_arg_count
                + u32::from(parameter_flags & CO_VARARGS != 0)
                + u32::from(parameter_flags & CO_VARKEYWORDS != 0),
        )
        .unwrap_or(usize::MAX);
        let names = &context.scopes.names;
        let header_storage = plan.header_local_storage.as_ref();
        let (mut locals, fast_local_count) = if let Some(storage) = header_storage {
            let mut seen = FxHashSet::default();
            if storage.fast_locals.iter().any(|name| !seen.insert(*name))
                || storage
                    .fast_locals
                    .get(..parameter_count)
                    .is_none_or(|parameters| Some(parameters) != plan.locals.get(..parameter_count))
                || storage.cellvars.iter().any(|name| !seen.contains(name))
            {
                return Err(CompileError::Internal(
                    "invalid generic-header fast-local storage layout".to_string(),
                ));
            }
            let mut locals = storage.fast_locals.clone();
            let fast_local_count = locals.len();
            let mut cell_only_locals: Vec<_> = plan
                .cellvars
                .iter()
                .filter(|name| !locals.contains(name))
                .copied()
                .collect();
            cell_only_locals.sort_by(|left, right| {
                let spelling = |name| {
                    plan.cell_sort_names
                        .get(&name)
                        .map_or_else(|| names.resolve(name), String::as_str)
                };
                spelling(*left).cmp(spelling(*right))
            });
            locals.extend(cell_only_locals);
            (locals, fast_local_count)
        } else {
            partition_function_locals(plan, parameter_count, names)
        };
        let mut indices = FxHashMap::default();
        indices.reserve(locals.len().saturating_add(plan.local_aliases.len()));
        for (index, &name) in locals.iter().enumerate() {
            indices.insert(name, to_u32(index, "local variable count")?);
        }
        let mut temporary_indices = FxHashMap::default();
        if let Some(storage) = header_storage {
            for &(symbol, physical) in &storage.comprehension_slots {
                let index = indices.get(&physical).copied().ok_or_else(|| {
                    CompileError::Internal(format!(
                        "missing generic-header comprehension storage `{}`",
                        names.resolve(physical)
                    ))
                })?;
                if usize::try_from(index).unwrap() >= fast_local_count {
                    return Err(CompileError::Internal(format!(
                        "generic-header comprehension storage `{}` is not a fast local",
                        names.resolve(physical)
                    )));
                }
                temporary_indices.insert(symbol, index);
                temporary_indices.insert(physical, index);
            }
            // CPython adds mangled save/restore slots to its metadata tables
            // without adding the corresponding name to the symbol table. Keep
            // these slots unavailable to ordinary name and closure lookup.
            let declared_names: FxHashSet<_> = plan
                .locals
                .iter()
                .chain(plan.cellvars.iter())
                .copied()
                .collect();
            indices.retain(|name, _| declared_names.contains(name));
        }
        for &(alias, original) in &plan.local_aliases {
            let index = indices[&original];
            indices.insert(alias, index);
            temporary_indices.insert(alias, index);
            temporary_indices.insert(original, index);
        }
        let local_count = locals.len();
        let mut free_indices = FxHashMap::default();
        free_indices.reserve(plan.freevars.len().saturating_add(plan.free_aliases.len()));
        for (index, &name) in plan.freevars.iter().enumerate() {
            free_indices.insert(name, to_u32(local_count + index, "free variable count")?);
        }
        for &(alias, original) in &plan.free_aliases {
            free_indices.insert(alias, free_indices[&original]);
        }
        locals.extend(plan.freevars.iter().copied());

        let aliases: Option<Rc<[(NameId, NameId)]>> =
            (!plan.local_aliases.is_empty()).then(|| plan.local_aliases.clone().into());
        let scope_cell_names = plan
            .cellvars
            .iter()
            .copied()
            .collect::<NameSet>()
            .with_aliases(aliases.as_ref());
        let mut cell_names = scope_cell_names.clone();
        if let Some(storage) = header_storage {
            cell_names.extend(storage.cellvars.iter().copied());
        }
        let global_names = plan.globals.iter().copied().collect();
        let initialized_locals = locals
            .iter()
            .take(parameter_count)
            .copied()
            .collect::<NameSet>()
            .with_aliases(aliases.as_ref());
        let free_names = plan.freevars.clone();

        let symbols = SymbolState {
            locals,
            fast_local_count,
            cell_names: cell_names.clone(),
            free_names,
            initialized_locals,
            temporary_indices,
            hidden_names: NameSet::default().with_aliases(aliases.as_ref()),
            active_temporaries: NameSet::default().with_aliases(aliases.as_ref()),
            active_temporary_fast_cells: NameSet::default().with_aliases(aliases.as_ref()),
            active_temporary_free_cells: NameSet::default().with_aliases(aliases.as_ref()),
            owned_load_locals: NameSet::default().with_aliases(aliases.as_ref()),
            ..SymbolState::default()
        };
        let unit = CodeUnit::from_spec(CodeUnitSpec {
            scope: Scope::Function {
                indices,
                free_indices,
                cells: scope_cell_names,
                globals: global_names,
            },
            scope_id,
            name: name.to_string(),
            qualified_name,
            private_name: None,
            arg_count,
            positional_only_arg_count,
            keyword_only_arg_count,
            flags: CO_OPTIMIZED | CO_NEWLOCALS | parameter_flags,
            first_line_number,
        });
        Ok(Self::new(context, unit, symbols))
    }

    pub(super) fn class(
        context: Rc<CompilationContext<'source>>,
        name: &str,
        qualified_name: String,
        first_line_number: u32,
        scope_id: ScopeId,
        globals: FxHashSet<NameId>,
        nonlocals: FxHashSet<NameId>,
        freevars: &[NameId],
        body_free_aliases: &[(NameId, NameId)],
        comprehension_targets: &[NameId],
        comprehension_cells: &FxHashSet<NameId>,
        bound_names: FxHashSet<NameId>,
        needs_class_closure: bool,
        has_methods: bool,
        has_conditional_annotations: bool,
        future_flags: u32,
    ) -> Box<Self> {
        let mut locals = comprehension_targets.to_vec();
        let fast_local_count = locals.len();
        let mut cell_names: NameSet = comprehension_cells.iter().copied().collect();
        let temporary_indices = comprehension_targets
            .iter()
            .copied()
            .enumerate()
            .map(|(index, name)| (name, u32::try_from(index).unwrap_or(u32::MAX)))
            .collect();
        let hidden_names = comprehension_targets.iter().copied().collect();
        let names = &context.scopes.names;
        if needs_class_closure {
            let name = names
                .get("__class__")
                .expect("module analysis should intern `__class__`");
            if !locals.contains(&name) {
                locals.push(name);
            }
            cell_names.insert(name);
        }
        if has_methods {
            let name = names
                .get("__classdict__")
                .expect("module analysis should intern `__classdict__`");
            if !locals.contains(&name) {
                locals.push(name);
            }
            cell_names.insert(name);
        }
        if has_conditional_annotations {
            let name = names
                .get("__conditional_annotations__")
                .expect("module analysis should intern `__conditional_annotations__`");
            if !locals.contains(&name) {
                locals.push(name);
            }
            cell_names.insert(name);
        }
        let cell_only_comprehension_names = context
            .scopes
            .elided_comprehension_cells(scope_id)
            .iter()
            .copied()
            .filter(|name| !locals.contains(name))
            .collect::<Vec<_>>();
        locals.extend(cell_only_comprehension_names);
        locals[fast_local_count..].sort_by_key(|&name| names.resolve(name));
        let free_start = locals.len();
        let free_indices = body_free_aliases
            .iter()
            .map(|(name, physical)| {
                let index = freevars
                    .iter()
                    .position(|name| name == physical)
                    .expect("class body capture has a physical slot");
                (*name, u32::try_from(free_start + index).unwrap_or(u32::MAX))
            })
            .collect();
        locals.extend(freevars.iter().copied());
        let free_names = freevars.to_vec();
        let globals = globals.into_iter().collect();
        let nonlocals = nonlocals.into_iter().collect();
        let bound_names = bound_names.into_iter().collect();
        let symbols = SymbolState {
            locals,
            fast_local_count,
            cell_names,
            free_names,
            temporary_indices,
            hidden_names,
            ..SymbolState::default()
        };
        let mut unit = CodeUnit::from_spec(CodeUnitSpec {
            scope: Scope::Class {
                globals,
                nonlocals,
                free_indices,
                bound_names,
            },
            scope_id,
            name: name.to_string(),
            qualified_name,
            private_name: Some(name.to_string()),
            arg_count: 0,
            positional_only_arg_count: 0,
            keyword_only_arg_count: 0,
            flags: future_flags,
            first_line_number,
        });
        unit.free_names_are_final = true;
        Self::new(context, unit, symbols)
    }

    pub(crate) fn compile_module(
        mut self: Box<Self>,
        body: &Suite,
    ) -> Result<Box<CodeObject>, CompileError> {
        self.assembler.reserve_items(body.len().saturating_mul(16));
        if self.context.python_version == PythonVersion::Py312 {
            self.legacy_short_label_prefix =
                super::legacy_native_seed_bound::record(&mut self.assembler, body);
            self.native_label_plan = super::legacy_native_labels::NativeLabelPlan::module(
                body,
                self.unit.flags & CO_FUTURE_ANNOTATIONS != 0,
            );
        }
        self.symbols
            .module_globals
            .clone_from(&self.context.module_plan().module_globals);
        self.symbols
            .imported_scope_names
            .clone_from(&self.context.module_plan().imported_scope_names);
        for &name in &self.context.module_plan().comprehension_targets {
            let index = to_u32(self.symbols.locals.len(), "module local count")?;
            self.symbols.locals.push(name);
            self.symbols.temporary_indices.insert(name, index);
            self.symbols.hidden_names.insert(name);
        }
        self.symbols.cell_names.extend(
            self.context
                .module_plan()
                .comprehension_cell_names
                .iter_unordered(),
        );
        self.symbols.fast_local_count = self.symbols.locals.len();
        let simple_module_annotations = self.context.module_plan().has_simple_annotations;
        let module_annotations = self.context.module_plan().has_annotations;
        let deferred_annotations = self.has_deferred_annotations();
        let future_module_annotations =
            self.unit.flags & CO_FUTURE_ANNOTATIONS != 0 && module_annotations;
        if module_annotations && deferred_annotations {
            let annotation_name = self.required_name_id("__conditional_annotations__");
            if !self.symbols.locals.contains(&annotation_name) {
                self.symbols.locals.push(annotation_name);
            }
            self.symbols.cell_names.insert(annotation_name);
        }
        let elided_comprehension_cells = self
            .context
            .scopes
            .elided_comprehension_cells(ScopeId::MODULE);
        let synthetic_annotation_cell = self
            .context
            .scopes
            .names
            .get("__conditional_annotations__")
            .filter(|name| !elided_comprehension_cells.contains(name));
        let cell_only_comprehension_names = elided_comprehension_cells
            .iter()
            .copied()
            .filter(|name| !self.symbols.locals.contains(name))
            .collect::<Vec<_>>();
        self.symbols.locals.extend(cell_only_comprehension_names);
        // Ordinary cells precede synthetic annotation cells, even after their assertions vanish.
        self.symbols.locals[self.symbols.fast_local_count..].sort_by_key(|&name| {
            (
                Some(name) == synthetic_annotation_cell,
                self.context.scopes.names.resolve(name),
            )
        });
        self.assembler.set_location(SourceLocation::NONE);
        let comprehension_cells = self
            .symbols
            .locals
            .iter()
            .enumerate()
            .filter(|(_, name)| self.is_cell_id(**name))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in comprehension_cells {
            self.emit(MAKE_CELL, to_u32(index, "module cell index")?, 0)?;
        }
        if self.context.python_version == PythonVersion::Py315 && self.context.module_contains_yield
        {
            self.assembler
                .set_location(SourceLocation::new(1, 1, -1, -1));
            self.emit(RETURN_GENERATOR, 0, 1)?;
            self.emit(POP_TOP, 0, -1)?;
        }
        self.assembler.set_location(SourceLocation::new(0, 1, 0, 0));
        self.emit_resume(ResumeLocation::AT_FUNC_START, ResumeDepth::Zero)?;
        // Match the deferred annotation function's visitation order, including suites
        // omitted during lowering. Repeated cleanup visits require occurrence IDs,
        // so those modules retain their existing sequential registration path.
        if deferred_annotations && simple_module_annotations && !future_module_annotations {
            let mut annotations = Vec::new();
            collect_simple_annotations(body, &mut annotations);
            for (index, annotation) in annotations.into_iter().enumerate() {
                let index = to_u32(index, "module annotation index")?;
                if self
                    .unit
                    .module_annotation_indices
                    .insert(annotation.range, index)
                    .is_some()
                {
                    self.unit.module_annotation_indices.clear();
                    break;
                }
            }
        }
        let mut deferred_module_annotations = None;
        if module_annotations && !deferred_annotations {
            if let Some(statement) = body.first() {
                self.assembler
                    .set_location(self.physical_source_location(statement.range()));
            }
            self.emit(SETUP_ANNOTATIONS, 0, 0)?;
        }
        if module_annotations && deferred_annotations {
            self.name_index("__conditional_annotations__")?;
            if let Some(statement) = body.first() {
                self.assembler
                    .set_location(self.physical_source_location(statement.range()));
            }
            if !future_module_annotations {
                if simple_module_annotations {
                    self.apply_compiler_stack_effect(LOAD_CONST, 0, 1)?;
                    let instruction = self.assembler.emit_placeholder_with_depth(
                        LOAD_CONST,
                        self.control.stack.depth.cast_unsigned(),
                    );
                    deferred_module_annotations = Some((
                        instruction,
                        self.output.deferred_constants_before_return.len(),
                        body.as_slice(),
                    ));
                    self.emit(MAKE_FUNCTION, 0, 0)?;
                    self.emit_deferred_store_name("__annotate__")?;
                }
            }
            self.emit(BUILD_SET, 0, 1)?;
            self.store_name("__conditional_annotations__")?;
            if future_module_annotations {
                self.emit(SETUP_ANNOTATIONS, 0, 0)?;
            }
        }

        let has_docstring = matches!(body.first(), Some(Stmt::Expr(expression))
            if expression.value.is_string_literal_expr());
        let body = if let Some(Stmt::Expr(expression)) = body.first() {
            if let Expr::StringLiteral(string) = expression.value.as_ref() {
                let constant =
                    self.add_constant(if self.context.python_version == PythonVersion::Py312 {
                        self.string_literal_constant(string)
                    } else {
                        Constant::String(clean_doc(string.value.to_str()))
                    })?;
                // Native docstring loads use the literal expression, excluding
                // parentheses belonging to the enclosing expression statement.
                let location = self.source_location(string.range);
                self.assembler.set_location(location);
                self.emit(LOAD_CONST, constant, 1)?;
                if self.context.python_version == PythonVersion::Py312 {
                    self.assembler.set_location(SourceLocation::NONE);
                }
                self.store_name("__doc__")?;
                &body[1..]
            } else if deferred_annotations
                && module_annotations
                && (fold_constant(&expression.value).is_some()
                    || self.context.fold_constant(&expression.value).is_some())
            {
                if self.context.python_version >= PythonVersion::Py314
                    && literal_constant(&expression.value).is_none()
                {
                    // Late folds still admit their original operands before the
                    // annotation setup absorbs the discarded expression's NOP.
                    let first_fold = self.output.deferred_constants.len();
                    self.record_folded_value(&expression.value)?;
                    if self.context.python_version == PythonVersion::Py315 {
                        self.record_discarded_fold_admissions(first_fold, None);
                    }
                }
                &body[1..]
            } else {
                body.as_slice()
            }
        } else {
            body.as_slice()
        };

        let body_start = self.assembler.instruction_count();
        let add_implicit_return = !suite_terminates(body);
        self.compile_scope_suite(body, add_implicit_return, has_docstring)?;
        if let Some((instruction, constant_position, annotation_body)) = deferred_module_annotations
        {
            // CPython lowers the deferred annotation thunk after the entire module body,
            // although its setup instructions are placed in the module's initial block.
            let annotation_child = self.compile_module_annotations(annotation_body)?;
            self.output.deferred_constants_before_return.insert(
                constant_position,
                (instruction, Constant::Code(annotation_child)),
            );
        }
        let add_implicit_return =
            add_implicit_return && !self.control.statements.emitted_fallthrough_return;
        if add_implicit_return
            && body.last().is_some_and(|statement| {
                terminal_statement_uses_implicit_return_location(
                    statement,
                    self.context.python_version,
                    self.unit.flags & CO_FUTURE_ANNOTATIONS == 0,
                )
            })
        {
            self.assembler.set_location(
                self.source_location(implicit_return_range(&self, body.last().unwrap())),
            );
        } else if self.assembler.instruction_count() > body_start {
            self.assembler.set_location(SourceLocation::NONE);
        } else if let Some(statement) = body
            .last()
            .filter(|statement| !matches!(statement, Stmt::Global(_) | Stmt::Nonlocal(_)))
        {
            self.assembler
                .set_location(self.source_location(statement.range()));
        }
        self.finish_inner(add_implicit_return)
    }

    pub(super) fn compile_function_body(
        mut self: Box<Self>,
        body: &Suite,
    ) -> Result<Box<CodeObject>, CompileError> {
        self.assembler.reserve_items(body.len().saturating_mul(16));
        if self.context.python_version == PythonVersion::Py312 {
            self.legacy_short_label_prefix =
                super::legacy_native_seed_bound::record(&mut self.assembler, body);
            self.native_label_plan = super::legacy_native_labels::NativeLabelPlan::function(
                body,
                self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != 0,
            );
        }
        self.emit_function_prologue()?;
        if self.context.python_version == PythonVersion::Py312 {
            // CPython 3.12 starts a separate body block even for ordinary functions.
            // A same-line `pass` must not inherit RESUME's synthetic 0:0 span.
            let body_entry = self.assembler.label();
            self.assembler.mark(body_entry);
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::FunctionStart,
                body_entry,
            );
            self.assembler
                .record_exception_owner_block_boundary(body_entry);
        }
        if !self.has_deferred_annotations()
            && !matches!(body.first(), Some(Stmt::Expr(expression)) if expression.value.is_string_literal_expr())
        {
            self.add_constant(Constant::None)?;
        }

        let has_docstring = matches!(body.first(), Some(Stmt::Expr(expression))
            if expression.value.is_string_literal_expr());
        let body = if let Some(Stmt::Expr(expression)) = body.first() {
            if let Expr::StringLiteral(string) = expression.value.as_ref() {
                self.add_constant(if self.context.python_version == PythonVersion::Py312 {
                    self.string_literal_constant(string)
                } else {
                    Constant::String(clean_doc(string.value.to_str()))
                })?;
                if self.has_deferred_annotations() {
                    self.unit.flags |= CO_HAS_DOCSTRING;
                }
                &body[1..]
            } else {
                body.as_slice()
            }
        } else {
            body.as_slice()
        };

        let body_start = self.assembler.instruction_count();
        let add_implicit_return = !suite_terminates(body);
        self.compile_scope_suite(body, add_implicit_return, has_docstring)?;
        let add_implicit_return =
            add_implicit_return && !self.control.statements.emitted_fallthrough_return;
        // Elided statements must not hide the assignment that owns the implicit
        // return's location before static store reordering and fusion.
        let terminal_statement = body.last();
        let is_elided_statement = |statement: &Stmt| {
            matches!(statement, Stmt::Global(global) if global.names.is_empty())
                && self.context.is_elided_statement(statement)
        };
        let annotated_comprehension_store = |statement: &Stmt| {
            if self.context.python_version >= PythonVersion::Py314
                && let Stmt::AnnAssign(assignment) = statement
                && matches!(assignment.target.as_ref(), Expr::Name(_))
                && matches!(
                    assignment.value.as_deref(),
                    Some(Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_))
                )
            {
                // Function-local annotations emit no code after the result store.
                // Its span remains the return donor when comprehension restores move.
                Some(assignment.target.range())
            } else {
                None
            }
        };
        let return_location_statement = if self.context.python_version >= PythonVersion::Py314
            && terminal_statement.is_some_and(is_elided_statement)
        {
            body.iter()
                .rev()
                .find(|statement| !is_elided_statement(statement))
                .filter(|statement| {
                    matches!(statement, Stmt::Assign(_))
                        || annotated_comprehension_store(statement).is_some()
                })
                .or(terminal_statement)
        } else {
            terminal_statement
        };
        let return_range = return_location_statement.and_then(|statement| {
            annotated_comprehension_store(statement).or_else(|| {
                terminal_statement_uses_implicit_return_location(
                    statement,
                    self.context.python_version,
                    false,
                )
                .then(|| implicit_return_range(&self, statement))
            })
        });
        if add_implicit_return && let Some(range) = return_range {
            self.assembler.set_location(self.source_location(range));
        } else if self.assembler.instruction_count() > body_start {
            self.assembler.set_location(SourceLocation::NONE);
        } else if let Some(statement @ Stmt::Pass(_)) = body.last() {
            self.assembler
                .set_location(self.source_location(statement.range()));
        }
        // Native 3.12's StopIteration wrapper appends an unlocated return even
        // after a terminating source suite, separating its final source joins
        // from the handler. Keep the suite's location selection above unchanged.
        // A retained dead If join likewise needs its original nonempty footer
        // until native-label capture and unreachable-code removal finish.
        let add_implicit_return = add_implicit_return
            || (self.context.python_version == PythonVersion::Py312
                && (self.control.generators.generator_region_start.is_some()
                    || self.control.statements.retained_native_if_join)
                && !self.control.statements.emitted_fallthrough_return);
        if self.context.python_version == PythonVersion::Py312
            && add_implicit_return
            && self.assembler.location() == SourceLocation::NONE
            && terminal_statement.is_some_and(is_elided_statement)
            && let Some(Stmt::If(conditional)) = body
                .iter()
                .rev()
                .find(|statement| !is_elided_statement(statement))
            && early_condition_truthiness(&conditional.test).is_none()
            && ((self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR)
                == CO_COROUTINE
                && conditional.elif_else_clauses.is_empty()
                && matches!(conditional.body.iter().rfind(|part| !is_elided_statement(part)),
                    Some(Stmt::Expr(expression))
                        if matches!(expression.value.as_ref(), Expr::Call(_) | Expr::Await(_))))
                || (conditional.body.iter().all(is_elided_statement)
                    && matches!(conditional.elif_else_clauses.as_slice(), [clause]
                        if clause.test.is_none() && clause.body.iter().all(is_elided_statement))))
        {
            // General If lowering leaves its original end label here, including
            // discarded coroutine calls before an erased suffix. Stores retain
            // their early line donor. Keep the call footer until
            // native repeated predecessor counts decide which returns survive.
            self.assembler.record_current_original_scope_exit();
        }
        self.finish_inner(add_implicit_return)
    }

    pub(super) fn compile_class_body(
        mut self: Box<Self>,
        body: &Suite,
    ) -> Result<Box<CodeObject>, CompileError> {
        if self.context.python_version != PythonVersion::Py312 {
            self.context.begin_class_static_attributes();
        }
        self.assembler.reserve_items(body.len().saturating_mul(16));
        let has_classdict_cell = self.is_cell("__classdict__");
        let has_class_cell = self.is_cell("__class__");
        let has_conditional_annotations = self.is_cell("__conditional_annotations__");
        if has_conditional_annotations && self.unit.flags & CO_FUTURE_ANNOTATIONS == 0 {
            let mut annotations = Vec::new();
            collect_conditional_simple_annotations(body, &mut annotations);
            self.unit.class_conditional_annotations = annotations
                .into_iter()
                .map(|annotation| annotation.range)
                .collect();
        }
        self.assembler.set_location(SourceLocation::NONE);
        if !self.symbols.free_names.is_empty() {
            self.emit(
                COPY_FREE_VARS,
                to_u32(self.symbols.free_names.len(), "class free variable count")?,
                0,
            )?;
        }
        let cell_indices = self
            .symbols
            .locals
            .iter()
            .take(self.symbols.locals.len() - self.symbols.free_names.len())
            .enumerate()
            .filter(|(_, name)| self.is_cell_id(**name))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in cell_indices {
            self.emit(MAKE_CELL, to_u32(index, "class cell index")?, 0)?;
        }
        let line = i32::try_from(self.unit.first_line_number).unwrap_or(i32::MAX);
        self.assembler
            .set_location(SourceLocation::new(line, line, 0, 0));
        self.emit_resume(ResumeLocation::AT_FUNC_START, ResumeDepth::Zero)?;
        self.load_name("__name__")?;
        self.store_name("__module__")?;
        self.output.qualified_name_is_canonical = self
            .context
            .record_class_qualified_name(&self.unit.qualified_name);
        // A parser-interned dotted name can leave the class constant distinct from
        // its interned code metadata; preserve the cache admission recorded above.
        self.context.record_constant_string_origin(
            &self.unit.qualified_name,
            (self.has_deferred_annotations()
                && self.output.qualified_name_is_canonical != Some(false))
                || self.unit.qualified_name == self.unit.name,
        );
        let qualified_name =
            self.add_constant(Constant::String(self.unit.qualified_name.clone()))?;
        self.emit(LOAD_CONST, qualified_name, 1)?;
        self.output.qualified_name_constant_index = Some(qualified_name);
        self.store_name("__qualname__")?;
        if self.context.python_version != PythonVersion::Py312 {
            if self.has_deferred_annotations() && u8::try_from(self.unit.first_line_number).is_ok()
            {
                self.emit(LOAD_SMALL_INT, self.unit.first_line_number, 1)?;
            } else {
                let first_line_number =
                    self.add_constant(Constant::Int(u64::from(self.unit.first_line_number)))?;
                self.emit(LOAD_CONST, first_line_number, 1)?;
            }
            self.store_name("__firstlineno__")?;
        }
        if self
            .symbols
            .free_names
            .iter()
            .any(|name| *name == self.required_name_id(".type_params"))
        {
            self.load_name(".type_params")?;
            self.store_name("__type_params__")?;
        }
        if has_classdict_cell {
            self.emit(LOAD_LOCALS, 0, 1)?;
            let index = self
                .symbols
                .locals
                .iter()
                .position(|name| *name == self.required_name_id("__classdict__"))
                .expect("classdict cell is present");
            self.emit(STORE_DEREF, to_u32(index, "classdict cell index")?, -1)?;
        }
        if has_conditional_annotations {
            self.emit(BUILD_SET, 0, 1)?;
            let index = self.closure_index("__conditional_annotations__")?;
            self.emit(STORE_DEREF, index, -1)?;
        }
        if if self.has_deferred_annotations() {
            self.unit.flags & CO_FUTURE_ANNOTATIONS != 0 && has_annotations(body)
        } else {
            has_annotations(body)
        } {
            self.emit(SETUP_ANNOTATIONS, 0, 0)?;
        }

        let has_docstring = matches!(body.first(), Some(Stmt::Expr(expression))
            if expression.value.is_string_literal_expr());
        let body = if let Some(Stmt::Expr(expression)) = body.first() {
            if let Expr::StringLiteral(string) = expression.value.as_ref() {
                let docstring =
                    self.add_constant(if self.context.python_version == PythonVersion::Py312 {
                        self.string_literal_constant(string)
                    } else {
                        Constant::String(clean_doc(string.value.to_str()))
                    })?;
                // Native docstring loads use the literal expression, excluding
                // parentheses belonging to the enclosing expression statement.
                let location = self.source_location(string.range);
                self.assembler.set_location(location);
                self.emit(LOAD_CONST, docstring, 1)?;
                if self.context.python_version == PythonVersion::Py312 {
                    // Native leaves the artificial docstring store unlocated until
                    // after NOP removal, so a same-line trailing Pass can donate.
                    self.assembler.set_location(SourceLocation::NONE);
                }
                self.store_name("__doc__")?;
                &body[1..]
            } else {
                body.as_slice()
            }
        } else {
            body.as_slice()
        };
        let body_start = self.assembler.instruction_count();
        let mut footer_location = SourceLocation::NONE;
        self.compile_scope_suite(body, false, has_docstring)?;
        debug_assert!(self.unit.class_conditional_annotations.is_empty());
        if let Some(Stmt::Expr(expression)) = body.last()
            && (fold_constant(&expression.value).is_some()
                || self.context.fold_constant(&expression.value).is_some())
        {
            let range = if let Expr::FString(fstring) = expression.value.as_ref() {
                self.fstring_result_range(fstring)
            } else if self.context.python_version >= PythonVersion::Py314
                && matches!(expression.value.as_ref(), Expr::BinOp(_) | Expr::UnaryOp(_))
            {
                expression.value.range()
            } else {
                expression.range
            };
            self.assembler.set_location(self.source_location(range));
            self.emit(NOP, 0, 0)?;
        } else if let Some(Stmt::If(statement)) = body.last()
            && matches!(statement.test.as_ref(), Expr::EllipsisLiteral(_))
            && let Some(Stmt::Expr(expression)) = statement.body.last()
            && (fold_constant(&expression.value).is_some()
                || self.context.fold_constant(&expression.value).is_some())
        {
            if statement.elif_else_clauses.is_empty() {
                // Without an exit jump, the footer inherits the final expression's location.
                let range = if self.context.python_version >= PythonVersion::Py314
                    && matches!(expression.value.as_ref(), Expr::BinOp(_) | Expr::UnaryOp(_))
                {
                    expression.value.range()
                } else {
                    expression.range
                };
                footer_location = self.source_location(range);
            }
            // A discarded else leaves an exit jump to the originally unlocated footer.
            // Keep it unlocated until exit copying transfers the predecessor's location.
        } else if self.assembler.instruction_count() > body_start
            && let Some(location) = self.assembler.last_instruction_location()
        {
            self.assembler.set_location(location);
        } else if let Some(statement) = body.last() {
            // Declarations emit no instruction and cannot donate a new location
            // to the artificial class epilogue.
            if !matches!(statement, Stmt::Global(_) | Stmt::Nonlocal(_))
                && !matches!(
                    statement,
                    Stmt::AnnAssign(annotation) if annotation.simple && annotation.value.is_none()
                )
            {
                self.assembler
                    .set_location(self.source_location(statement.range()));
                self.emit(NOP, 0, 0)?;
            }
        }

        if self.has_deferred_annotations()
            && self.unit.flags & CO_FUTURE_ANNOTATIONS == 0
            && let Some((annotation_child, closure_names)) = self.compile_class_annotations(body)?
        {
            let line = i32::try_from(self.unit.first_line_number).unwrap_or(i32::MAX);
            self.assembler
                .set_location(SourceLocation::new(line, line, 0, 0));
            self.emit_closure_tuple(&closure_names)?;
            let annotation = self.add_constant(Constant::Code(annotation_child))?;
            self.emit(LOAD_CONST, annotation, 1)?;
            self.emit(MAKE_FUNCTION, 0, 0)?;
            self.emit_function_attribute(FunctionAttribute::CLOSURE)?;
            self.store_name("__annotate_func__")?;
        }

        self.assembler.set_location(footer_location);
        if self.context.python_version != PythonVersion::Py312 {
            let static_attributes = self.context.finish_class_static_attributes();
            self.context
                .record_static_attribute_tuple(&static_attributes);
            let static_attributes = self
                .output
                .constants
                .intern(Constant::Tuple(static_attributes))?;
            self.emit(LOAD_CONST, static_attributes, 1)?;
            self.store_name("__static_attributes__")?;
        }
        if has_classdict_cell {
            let index = self
                .symbols
                .locals
                .iter()
                .position(|name| *name == self.required_name_id("__classdict__"))
                .expect("classdict cell is present");
            self.emit(
                if self.context.python_version == PythonVersion::Py312 {
                    super::LOAD_CLOSURE
                } else {
                    LOAD_FAST
                },
                to_u32(index, "classdict cell index")?,
                1,
            )?;
            self.store_name("__classdictcell__")?;
        }
        if has_class_cell {
            let index = self
                .symbols
                .locals
                .iter()
                .position(|name| *name == self.required_name_id("__class__"))
                .expect("class cell is present");
            self.emit(
                if self.context.python_version == PythonVersion::Py312 {
                    super::LOAD_CLOSURE
                } else {
                    LOAD_FAST
                },
                to_u32(index, "class cell index")?,
                1,
            )?;
            self.emit(COPY, 1, 1)?;
            self.store_name("__classcell__")?;
            self.emit(RETURN_VALUE, 0, -1)?;
            self.finish_inner(false)
        } else {
            self.finish()
        }
    }

    pub(super) fn finish(self: Box<Self>) -> Result<Box<CodeObject>, CompileError> {
        self.finish_inner(true)
    }

    pub(super) fn finish_inner(
        mut self: Box<Self>,
        add_implicit_return: bool,
    ) -> Result<Box<CodeObject>, CompileError> {
        for (instruction, constant) in
            std::mem::take(&mut self.output.deferred_constants_before_return)
        {
            let index = self.add_constant(constant)?;
            self.assembler.patch_argument(instruction, index)?;
        }
        if self.output.has_deferred_break_none {
            let none = self.add_constant(Constant::None)?;
            self.assembler.set_deferred_none_constant_index(none);
        }
        if add_implicit_return {
            let none = self.add_constant(Constant::None)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(RETURN_VALUE, 0, -1)?;
        }
        if matches!(&self.unit.scope, Scope::Class { .. }) {
            // CPython first inlines unlocated class footers, then duplicates shared
            // exits for conditional predecessors before propagating locations.
            self.assembler.allow_last_no_location_block_inlining();
        }
        if self.output.constants.is_empty()
            && !(self.unit.name == "<lambda>" && self.unit.flags & CO_GENERATOR != 0)
        {
            self.add_constant(Constant::None)?;
        }
        if let Some(start) = self.control.generators.generator_region_start {
            self.assembler.record_exception_setup(start, false);
            let handler = self.assembler.label();
            self.assembler
                .record_legacy_native_setup_target(start, handler);
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::StopIterationHandler,
                handler,
            );
            self.assembler.mark(handler);
            let mut region_start = start;
            for (exclusion_start, exclusion_end) in
                &self.control.generators.generator_region_exclusions
            {
                self.assembler
                    .add_stopiteration_exception_region_with_depth(
                        region_start,
                        *exclusion_start,
                        handler,
                        self.control.generators.generator_region_depth,
                    );
                region_start = *exclusion_end;
            }
            self.assembler
                .add_stopiteration_exception_region_with_depth(
                    region_start,
                    handler,
                    handler,
                    self.control.generators.generator_region_depth,
                );
            self.assembler.set_location(SourceLocation::NONE);
            self.set_depth(1);
            self.emit_unary_intrinsic(UnaryIntrinsic::STOPITERATION_ERROR)?;
            self.emit_reraise(ReraiseStackState::StopIterationHandler)?;
        }

        if self.control.stack.depth != 0 {
            return Err(CompileError::Internal(format!(
                "compiler finished with stack depth {}",
                self.control.stack.depth
            )));
        }

        self.output.first_flowgraph_constant = self.output.constants.len();
        for (deferred_index, (target, constant)) in
            std::mem::take(&mut self.output.deferred_constants)
                .into_iter()
                .enumerate()
        {
            if self
                .output
                .fresh_empty_bytes_deferred
                .contains(&deferred_index)
            {
                self.context.record_empty_bytes_origin(true);
            }
            let index = self.add_constant(constant)?;
            match target {
                Some(DeferredConstantTarget::Instruction(instruction)) => {
                    self.assembler.patch_argument(instruction, index)?;
                }
                Some(DeferredConstantTarget::DiscardedExpression(label)) => {
                    self.assembler.record_discarded_fold_admission(label, index);
                }
                Some(DeferredConstantTarget::FoldedOperand(id)) => {
                    self.assembler.record_fold_admission(id, index);
                }
                None => {}
            }
        }
        for (instruction, name) in std::mem::take(&mut self.output.deferred_names) {
            let index = self.name_index(&name)?;
            self.assembler.patch_argument(instruction, index)?;
        }
        self.certify_native_source_labels();
        self.assembler.resolve_operand_patches()?;
        self.assembler.optimize_constant_pops();
        if self.has_deferred_annotations() {
            self.remove_unused_constants();
        }

        let local_kinds = self
            .symbols
            .locals
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let positional_only = usize::try_from(self.unit.positional_only_arg_count).unwrap();
                let positional = usize::try_from(self.unit.arg_count).unwrap();
                let keyword_only = usize::try_from(self.unit.keyword_only_arg_count).unwrap();
                let argument_kind = if !self.has_deferred_annotations() {
                    0
                } else if index < positional_only {
                    CO_FAST_ARG_POS
                } else if index < positional {
                    CO_FAST_ARG_POS | CO_FAST_ARG_KW
                } else if index < positional + keyword_only {
                    CO_FAST_ARG_KW
                } else if self.unit.flags & CO_VARARGS != 0 && index == positional + keyword_only {
                    CO_FAST_ARG_POS | CO_FAST_ARG_VAR
                } else if self.unit.flags & CO_VARKEYWORDS != 0
                    && index
                        == positional
                            + keyword_only
                            + usize::from(self.unit.flags & CO_VARARGS != 0)
                {
                    CO_FAST_ARG_KW | CO_FAST_ARG_VAR
                } else {
                    0
                };
                let storage_kind =
                    if index >= self.symbols.locals.len() - self.symbols.free_names.len() {
                        CO_FAST_FREE
                    } else if self.is_cell_id(*name) {
                        if index < self.symbols.fast_local_count {
                            CO_FAST_LOCAL | CO_FAST_CELL
                        } else {
                            CO_FAST_CELL
                        }
                    } else {
                        CO_FAST_LOCAL
                    };
                argument_kind
                    | storage_kind
                    | if storage_kind & CO_FAST_LOCAL != 0 && self.is_hidden_id(*name) {
                        CO_FAST_HIDDEN
                    } else {
                        0
                    }
            })
            .collect();
        let interner = &self.context.scopes.names;
        let (output_locals, local_ids): (Vec<MetadataName>, Vec<MetadataId>) = self
            .symbols
            .locals
            .iter()
            .enumerate()
            .map(|(index, name_id)| {
                let spelling = self.resolve_name(*name_id);
                if (self.unit.free_names_are_final
                    && index >= self.symbols.locals.len() - self.symbols.free_names.len()
                    || self.unit.header_symbol_names_are_final)
                    && !(self.unit.annotation_thunk && spelling == ".format")
                {
                    return (
                        MetadataName::from(interner.clone_name(*name_id)),
                        MetadataId::from(*name_id),
                    );
                }
                // The annotation argument has a separate internal identity so user names
                // and comprehension targets spelled `format` do not bind to its slot.
                if self.unit.annotation_thunk && spelling == ".format" {
                    return (
                        MetadataName::new_static("format"),
                        MetadataId::from(self.required_name_id("format")),
                    );
                }
                let name = self.mangled_name_cow(spelling);
                let id = if name == spelling {
                    *name_id
                } else {
                    self.required_name_id(&name)
                };
                (
                    MetadataName::from(interner.clone_name(id)),
                    MetadataId::from(id),
                )
            })
            .unzip();
        let output_names = self
            .output
            .name_ids
            .iter()
            .map(|id| MetadataName::from(interner.clone_name(NameId::from_u32(id.as_u32()))))
            .collect::<Vec<_>>();
        let filename_id = to_u32(interner.len(), "marshal metadata name count")?;
        let metadata_ids = MarshalMetadataIds {
            domain_len: filename_id
                .checked_add(1)
                .ok_or_else(|| CompileError::Internal("too many marshal metadata names".into()))?,
            names: std::mem::take(&mut self.output.name_ids),
            locals: local_ids,
            filename: MetadataId::from_u32(filename_id),
            name: interner.get(&self.unit.name).map(MetadataId::from),
            qualified_name: interner
                .get(&self.unit.qualified_name)
                .map(MetadataId::from),
        };
        let parameter_count = usize::try_from(
            self.unit.arg_count
                + self.unit.keyword_only_arg_count
                + u32::from(self.unit.flags & CO_VARARGS != 0)
                + u32::from(self.unit.flags & CO_VARKEYWORDS != 0),
        )
        .unwrap_or(usize::MAX);
        let mut constants = InterningConstantTable {
            output: &mut self.output,
            context: &self.context,
        };
        let AssembledCode {
            bytecode,
            line_table,
            exception_table,
            max_depth: assembled_max_depth,
            removed_max_depth,
        } = self.assembler.finish_code_with_constants(
            self.unit.first_line_number,
            self.symbols.fast_local_count,
            parameter_count,
            Some(&mut constants),
        )?;
        if exception_table.is_empty() || line_table.is_empty() {
            self.context.record_empty_bytes_origin(false);
        }
        let max_depth = if self.context.python_version <= crate::PythonVersion::Py313
            || (removed_max_depth == Some(self.control.stack.max_depth)
                && assembled_max_depth < self.control.stack.max_depth)
        {
            assembled_max_depth
        } else {
            self.control.stack.max_depth
        };
        let stack_size = if self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR)
            != 0
            && self.control.generators.generator_region_start.is_some()
        {
            max_depth.max(self.control.generators.generator_region_depth + 2)
        } else if self.context.python_version == PythonVersion::Py312 {
            max_depth
        } else {
            max_depth.max(1)
        };
        let (parser_interned_strings, canonical_constant_strings) =
            if matches!(self.unit.scope, Scope::Module) {
                CompilationContext::finish_constant_string_provenance(
                    &mut self.context,
                    self.output.has_frozenset_constants,
                )
            } else {
                (FxHashSet::default(), FxHashSet::default())
            };
        let metadata_tuple_properties =
            crate::marshal::metadata_tuple_properties(&output_names, &output_locals);
        let constants = self.output.constants.into_values();
        // The root's non-string constant tuple is recorded as unique, so its hash
        // would never be used. All-string roots can still alias metadata tuples.
        let constants_tuple_hash = if matches!(self.unit.scope, Scope::Module)
            && !constants
                .iter()
                .all(|value| matches!(value, Constant::String(_)))
        {
            None
        } else {
            crate::marshal::constants_tuple_hash(&constants)
        };
        let marshal_occurrence_bound = crate::marshal::code_occurrence_bound(
            &constants,
            output_names.len(),
            output_locals.len(),
        );
        let allocation = Box::new_uninit();
        let mut code = Box::write(
            allocation,
            CodeObject {
                arg_count: self.unit.arg_count,
                positional_only_arg_count: self.unit.positional_only_arg_count,
                keyword_only_arg_count: self.unit.keyword_only_arg_count,
                stack_size,
                flags: self.unit.flags,
                bytecode,
                constants,
                names: output_names,
                locals: output_locals,
                local_kinds,
                filename: self.context.filename.clone(),
                name: self.unit.name,
                qualified_name: self.unit.qualified_name,
                first_line_number: self.unit.first_line_number,
                line_table,
                exception_table,
                marshal_provenance: MarshalProvenance {
                    assembly_order: None,
                    interning_events: Vec::new(),
                    interned_metadata_tuples: Vec::new(),
                    locals_identity: if self.unit.annotation_thunk {
                        LocalsIdentity::Fresh
                    } else {
                        LocalsIdentity::ByValue
                    },
                    explicitly_interned_strings: self.output.interned_constant_strings,
                    parser_interned_strings,
                    generated_mangled_strings: if matches!(self.unit.scope, Scope::Module) {
                        self.context.generated_mangled_strings.clone()
                    } else {
                        FxHashSet::default()
                    },
                    canonical_constant_strings,
                    canonical_constant_strings_complete: matches!(self.unit.scope, Scope::Module),
                    qualified_name_constant_index: self.output.qualified_name_constant_index,
                    qualified_name_is_canonical: self.output.qualified_name_is_canonical,
                    uncached_percent_constant: self.context.has_uncached_percent_constant(),
                    metadata_ids,
                    metadata_name_lookup: None,
                    metadata_tuple_properties: Some(metadata_tuple_properties),
                    constants_tuple_hash,
                    constant_hashes: crate::marshal::ConstantHashes::default(),
                    marshal_occurrence_bound: Some(marshal_occurrence_bound),
                    marshal_graph_summary: None,
                    table_hashes: None,
                    subtree_has_frozenset_constants: self.output.has_frozenset_constants,
                    metadata_alias_scan_unnecessary: false,
                    fresh_empty_bytes_constant: self.context.fresh_empty_bytes_constant(),
                },
            },
        );
        code.marshal_provenance.table_hashes = Some([
            crate::marshal::bytes_hash(&code.line_table),
            crate::marshal::bytes_hash(&code.exception_table),
        ]);
        debug_assert!(
            code.names.iter().chain(&code.locals).all(|name| self
                .context
                .scopes
                .names
                .get(name.as_str())
                .is_some()),
            "emitted metadata names must be present in the module interner"
        );
        code.validate()?;
        code.marshal_provenance.assembly_order = self.context.record_code_assembly(&code);
        if matches!(self.unit.scope, Scope::Module) {
            code.marshal_provenance.constant_hashes =
                crate::marshal::constant_hashes_for_code(&code);
            code.marshal_provenance.interning_events = self.context.take_interning_events();
            code.marshal_provenance.interned_metadata_tuples =
                self.context.take_interned_metadata_tuples();
            code.marshal_provenance.metadata_alias_scan_unnecessary =
                metadata_alias_scan_unnecessary(&code, &self.context.scopes.names);
            let context = Rc::get_mut(&mut self.context)
                .expect("completed module exclusively owns its compilation context");
            code.marshal_provenance.metadata_name_lookup = Some(std::sync::Arc::new(
                std::mem::take(&mut context.scopes.names).into_marshal_lookup(),
            ));
        }
        code.marshal_provenance.marshal_graph_summary = crate::marshal::code_graph_summary(&code);
        Ok(code)
    }

    pub(super) fn emit_function_prologue(&mut self) -> Result<(), CompileError> {
        self.emit_prologue(None)
    }

    /// Acquire a generator expression's outer iterator before returning its generator.
    pub(super) fn emit_generator_expression_prologue(
        &mut self,
        is_async: bool,
        iterator_location: SourceLocation,
    ) -> Result<(), CompileError> {
        self.emit_prologue(Some((is_async, iterator_location)))
    }

    fn emit_prologue(
        &mut self,
        generator_iterator: Option<(bool, SourceLocation)>,
    ) -> Result<(), CompileError> {
        self.assembler.set_location(SourceLocation::NONE);
        if !self.symbols.free_names.is_empty() {
            self.emit(
                COPY_FREE_VARS,
                to_u32(self.symbols.free_names.len(), "free variable count")?,
                0,
            )?;
        }
        let cells: Vec<_> = self
            .symbols
            .locals
            .iter()
            .take(self.symbols.locals.len() - self.symbols.free_names.len())
            .enumerate()
            .filter(|(_, name)| self.is_cell_id(**name))
            .map(|(index, _)| index)
            .collect();
        for index in cells {
            self.emit(MAKE_CELL, to_u32(index, "cell variable index")?, 0)?;
        }
        if let Some((is_async, iterator_location)) = generator_iterator {
            // This semantic marker becomes RESUME_AT_GEN_EXPR_START during layout.
            // Keeping it distinct from the 3.14 depth bit avoids an ambiguous operand.
            self.emit(RESUME, crate::target::GENEXPR_RESUME_ARGUMENT, 0)?;
            self.assembler.set_location(iterator_location);
            self.emit(LOAD_FAST, 0, 1)?;
            if is_async {
                self.emit(GET_AITER, 0, 0)?;
            } else {
                self.emit(GET_ITER, 0, self.iterator_extra_slots())?;
            }
            self.control.generators.generator_region_depth =
                self.control.stack.depth.cast_unsigned();
        }
        if self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != 0 {
            let line = i32::try_from(self.unit.first_line_number).unwrap_or(i32::MAX);
            self.assembler
                .set_location(SourceLocation::new(line, line, -1, -1));
            // This physical +1/-1 frame-startup pair precedes logical expression lowering and
            // intentionally does not participate in the compiler's value-depth accounting.
            self.assembler.emit(RETURN_GENERATOR, 0);
            if self.context.python_version == PythonVersion::Py312 {
                self.assembler.set_location(SourceLocation::NONE);
            }
            self.assembler.emit(POP_TOP, 0);
            if self.context.python_version == PythonVersion::Py312 {
                self.assembler.preserve_last_no_location();
            }
            let start = self.assembler.label();
            self.assembler.mark(start);
            self.control.generators.generator_region_start = Some(start);
        }
        let line = i32::try_from(self.unit.first_line_number).unwrap_or(i32::MAX);
        self.assembler
            .set_location(SourceLocation::new(line, line, 0, 0));
        self.emit_resume(ResumeLocation::AT_FUNC_START, ResumeDepth::Zero)
    }
}

impl CodeUnit {
    fn from_spec(spec: CodeUnitSpec) -> Self {
        Self {
            scope: spec.scope,
            scope_id: spec.scope_id,
            generic_target_qualified_name: None,
            child_qualified_name_parent: None,
            class_scope_is_nested: false,
            module_annotation_index: 0,
            module_annotation_indices: FxHashMap::default(),
            class_conditional_annotations: VecDeque::new(),
            annotation_classdict_index: None,
            name: spec.name,
            qualified_name: spec.qualified_name,
            private_name: spec.private_name,
            mangled_names: None,
            free_names_are_final: false,
            pending_private_type_parameters: FxHashSet::default(),
            header_symbol_names_are_final: false,
            arg_count: spec.arg_count,
            positional_only_arg_count: spec.positional_only_arg_count,
            keyword_only_arg_count: spec.keyword_only_arg_count,
            flags: spec.flags,
            legacy_sync_coroutine: false,
            first_line_number: spec.first_line_number,
            annotation_thunk: false,
        }
    }
}

#[cfg(test)]
mod function_local_tests {
    use super::*;

    #[test]
    fn function_local_partition_matches_two_reference_passes() {
        fn reference(
            plan: &LoweringFunctionPlan,
            parameter_count: usize,
            names: &NameInterner,
        ) -> (Vec<NameId>, usize) {
            let mut locals: Vec<_> = plan
                .locals
                .iter()
                .enumerate()
                .filter(|(index, name)| {
                    *index < parameter_count
                        || !plan.cellvars.contains(*name)
                        || plan.inlined_comprehension_cellvars.contains(*name)
                })
                .map(|(_, name)| *name)
                .collect();
            let fast_local_count = locals.len();
            let mut cell_only_locals: Vec<_> = plan
                .locals
                .iter()
                .enumerate()
                .filter(|(index, name)| {
                    *index >= parameter_count
                        && plan.cellvars.contains(*name)
                        && !plan.inlined_comprehension_cellvars.contains(*name)
                })
                .map(|(_, name)| *name)
                .collect();
            cell_only_locals.sort_by(|left, right| {
                let spelling = |name| {
                    plan.cell_sort_names
                        .get(&name)
                        .map_or_else(|| names.resolve(name), String::as_str)
                };
                spelling(*left).cmp(spelling(*right))
            });
            locals.extend(cell_only_locals);
            (locals, fast_local_count)
        }

        let mut names = NameInterner::default();
        let local = ["z", "a", "m", "b", "t", "c"].map(|name| names.intern(name));
        let free = ["free_one", "free_two"].map(|name| names.intern(name));
        let set = |mask: u32| {
            local
                .into_iter()
                .enumerate()
                .filter_map(|(index, name)| (mask & (1 << index) != 0).then_some(name))
                .collect()
        };
        for cell_mask in 0..1 << local.len() {
            for comprehension_mask in 0..1 << local.len() {
                for parameter_count in 0..=local.len() + 1 {
                    let plan = LoweringFunctionPlan {
                        locals: local.to_vec(),
                        freevars: free.to_vec(),
                        cellvars: set(cell_mask),
                        inlined_comprehension_cellvars: set(comprehension_mask),
                        cell_sort_names: FxHashMap::from_iter([
                            (local[0], "0".to_owned()),
                            (local[1], "y".to_owned()),
                        ]),
                        ..LoweringFunctionPlan::default()
                    };
                    assert_eq!(
                        partition_function_locals(&plan, parameter_count, &names),
                        reference(&plan, parameter_count, &names),
                        "cells={cell_mask}, comprehensions={comprehension_mask}, parameters={parameter_count}"
                    );
                }
            }
        }
        assert_eq!(
            partition_function_locals(&LoweringFunctionPlan::default(), 0, &names),
            (Vec::new(), 0)
        );
    }
}
