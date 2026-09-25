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
use super::scope::{
    LoweringFunctionPlan, NameId, NameSet, ScopeId, collect_conditional_simple_annotations,
    collect_simple_annotations, has_annotations,
};
use super::source::{CompilationContext, implicit_return_range};
use super::statement::{
    ControlFlowState, suite_terminates, terminal_statement_uses_implicit_return_location,
};
use super::symbol::{ReraiseStackState, ResumeDepth, SymbolState, to_u32};
use super::{Compiler, Scope};

/// Proves that the surviving constants cannot be changed by interning a metadata tuple.
///
/// Ordinary ASCII identifier strings are already interned independently. For every other string
/// in an all-string constant tuple, conservatively check for an equal name or local anywhere in
/// this module, including recorded metadata from discarded code. Equality of the complete tuple
/// is left to marshal when any string can match.
fn metadata_alias_scan_unnecessary(code: &CodeObject) -> bool {
    fn collect_candidates<'a>(values: &'a [Constant], candidates: &mut FxHashSet<&'a str>) {
        if values
            .iter()
            .all(|value| matches!(value, Constant::String(_)))
        {
            for value in values {
                let Constant::String(value) = value else {
                    unreachable!("all-string tuple");
                };
                if value
                    .bytes()
                    .any(|byte| byte != b'_' && !byte.is_ascii_alphanumeric())
                {
                    candidates.insert(value);
                }
            }
            return;
        }

        for value in values {
            collect_nested_candidates(value, candidates);
        }
    }

    fn collect_nested_candidates<'a>(value: &'a Constant, candidates: &mut FxHashSet<&'a str>) {
        match value {
            Constant::Tuple(values) => collect_candidates(values, candidates),
            Constant::FrozenSet(values) => {
                for value in values {
                    collect_nested_candidates(value, candidates);
                }
            }
            Constant::Code(code) => collect_candidates(&code.constants, candidates),
            Constant::Slice { lower, upper, step } => {
                for value in [lower.as_ref(), upper.as_ref(), step.as_ref()] {
                    collect_nested_candidates(value, candidates);
                }
            }
            _ => {}
        }
    }

    fn metadata_matches(code: &CodeObject, candidates: &FxHashSet<&str>) -> bool {
        if code
            .names
            .iter()
            .chain(&code.locals)
            .map(MetadataName::as_str)
            .chain(
                code.marshal_provenance
                    .interned_metadata_tuples
                    .iter()
                    .flatten()
                    .map(String::as_str),
            )
            .any(|value| candidates.contains(value))
        {
            return true;
        }
        code.constants
            .iter()
            .any(|value| nested_metadata_matches(value, candidates))
    }

    fn nested_metadata_matches(value: &Constant, candidates: &FxHashSet<&str>) -> bool {
        match value {
            Constant::Code(code) => metadata_matches(code, candidates),
            Constant::Tuple(values) | Constant::FrozenSet(values) => values
                .iter()
                .any(|value| nested_metadata_matches(value, candidates)),
            Constant::Slice { lower, upper, step } => [lower, upper, step]
                .into_iter()
                .any(|value| nested_metadata_matches(value, candidates)),
            _ => false,
        }
    }

    let mut candidates = FxHashSet::default();
    collect_candidates(&code.constants, &mut candidates);
    candidates.is_empty() || !metadata_matches(code, &candidates)
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
        let assembler = Assembler::with_python_version(context.python_version);
        Box::new(Self {
            context,
            cached_source_lines: Cell::new([None; 2]),
            unit,
            output: OutputState::default(),
            symbols,
            control: ControlFlowState::default(),
            assembler,
            native_label_plan: None,
        })
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
        Self::module_with_profile(
            filename,
            source,
            tokens,
            body,
            python_version,
            crate::NativeBuildProfile::Default,
        )
    }

    pub(crate) fn module_with_profile(
        filename: impl Into<CodeFilename>,
        source: &'source str,
        tokens: &'source Tokens,
        body: &'source Suite,
        python_version: crate::PythonVersion,
        native_build_profile: crate::NativeBuildProfile,
    ) -> Box<Self> {
        let context = Rc::new(CompilationContext::with_profile(
            filename.into(),
            source,
            tokens,
            body,
            python_version,
            native_build_profile,
        ));
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
        let mut locals: Vec<_> = if let Some(storage) = header_storage {
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
            storage.fast_locals.clone()
        } else {
            plan.locals
                .iter()
                .enumerate()
                .filter(|(index, name)| {
                    *index < parameter_count
                        || !plan.cellvars.contains(*name)
                        || plan.inlined_comprehension_cellvars.contains(*name)
                })
                .map(|(_, name)| *name)
                .collect()
        };
        let fast_local_count = locals.len();
        let mut cell_only_locals: Vec<_> = if header_storage.is_some() {
            plan.cellvars
                .iter()
                .filter(|name| !locals.contains(name))
                .copied()
                .collect()
        } else {
            plan.locals
                .iter()
                .enumerate()
                .filter(|(index, name)| {
                    *index >= parameter_count
                        && plan.cellvars.contains(*name)
                        && !plan.inlined_comprehension_cellvars.contains(*name)
                })
                .map(|(_, name)| *name)
                .collect()
        };
        cell_only_locals.sort_by(|left, right| {
            let spelling = |name| {
                plan.cell_sort_names
                    .get(&name)
                    .map_or_else(|| names.resolve(name), String::as_str)
            };
            spelling(*left).cmp(spelling(*right))
        });
        locals.extend(cell_only_locals);
        let mut indices = locals
            .iter()
            .enumerate()
            .map(|(index, name)| Ok((*name, to_u32(index, "local variable count")?)))
            .collect::<Result<FxHashMap<_, _>, CompileError>>()?;
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
        let mut free_indices = plan
            .freevars
            .iter()
            .enumerate()
            .map(|(index, name)| Ok((*name, to_u32(local_count + index, "free variable count")?)))
            .collect::<Result<FxHashMap<_, _>, CompileError>>()?;
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
    ) -> Result<CodeObject, CompileError> {
        self.assembler.reserve_items(body.len().saturating_mul(16));
        if self.context.python_version == PythonVersion::Py312 {
            if super::legacy_native_seed_bound::certifies(body) {
                self.assembler.certify_legacy_uniform_jump_seed();
            }
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
        let simple_module_annotations = self.context.module_plan().has_simple_annotations;
        let module_annotations = self.context.module_plan().has_annotations;
        let deferred_annotations = self.has_deferred_annotations();
        let future_module_annotations =
            self.unit.flags & CO_FUTURE_ANNOTATIONS != 0 && module_annotations;
        if module_annotations && deferred_annotations {
            let annotation_cell =
                to_u32(self.symbols.locals.len(), "module annotation cell index")?;
            let annotation_name = self.required_name_id("__conditional_annotations__");
            self.symbols.locals.push(annotation_name);
            self.symbols.cell_names.insert(annotation_name);
            self.assembler.set_location(SourceLocation::NONE);
            self.emit(MAKE_CELL, annotation_cell, 0)?;
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
                    .set_location(self.source_location(statement.range()));
            }
            self.emit(SETUP_ANNOTATIONS, 0, 0)?;
        }
        if module_annotations && deferred_annotations {
            self.name_index("__conditional_annotations__")?;
            if let Some(statement) = body.first() {
                self.assembler
                    .set_location(self.source_location(statement.range()));
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
                (instruction, Constant::Code(Box::new(annotation_child))),
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
    ) -> Result<CodeObject, CompileError> {
        self.assembler.reserve_items(body.len().saturating_mul(16));
        if self.context.python_version == PythonVersion::Py312 {
            if super::legacy_native_seed_bound::certifies(body) {
                self.assembler.certify_legacy_uniform_jump_seed();
            }
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
        if add_implicit_return
            && body.last().is_some_and(|statement| {
                terminal_statement_uses_implicit_return_location(
                    statement,
                    self.context.python_version,
                    false,
                )
            })
        {
            self.assembler.set_location(
                self.source_location(implicit_return_range(&self, body.last().unwrap())),
            );
        } else if self.assembler.instruction_count() > body_start {
            self.assembler.set_location(SourceLocation::NONE);
        } else if let Some(statement @ Stmt::Pass(_)) = body.last() {
            self.assembler
                .set_location(self.source_location(statement.range()));
        }
        // Native 3.12's StopIteration wrapper appends an unlocated return even
        // after a terminating source suite, separating its final source joins
        // from the handler. Keep the suite's location selection above unchanged.
        let add_implicit_return = add_implicit_return
            || (self.context.python_version == PythonVersion::Py312
                && self.control.generators.generator_region_start.is_some()
                && !self.control.statements.emitted_fallthrough_return);
        self.finish_inner(add_implicit_return)
    }

    pub(super) fn compile_class_body(
        mut self: Box<Self>,
        body: &Suite,
    ) -> Result<CodeObject, CompileError> {
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
        self.context.record_constant_string_origin(
            &self.unit.qualified_name,
            self.has_deferred_annotations() || self.unit.qualified_name == self.unit.name,
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
            let annotation = self.add_constant(Constant::Code(Box::new(annotation_child)))?;
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

    pub(super) fn finish(self: Box<Self>) -> Result<CodeObject, CompileError> {
        self.finish_inner(true)
    }

    pub(super) fn finish_inner(
        mut self: Box<Self>,
        add_implicit_return: bool,
    ) -> Result<CodeObject, CompileError> {
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
                    return (MetadataName::new(spelling), MetadataId::from(*name_id));
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
                    MetadataId::from(*name_id)
                } else {
                    MetadataId::from(self.required_name_id(&name))
                };
                (MetadataName::from(name), id)
            })
            .unzip();
        let interner = &self.context.scopes.names;
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
            crate::marshal::metadata_tuple_properties(&self.output.names, &output_locals);
        let mut code = CodeObject {
            arg_count: self.unit.arg_count,
            positional_only_arg_count: self.unit.positional_only_arg_count,
            keyword_only_arg_count: self.unit.keyword_only_arg_count,
            stack_size,
            flags: self.unit.flags,
            bytecode,
            constants: self.output.constants.into_values(),
            names: self.output.names,
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
                qualified_name_constant_index: self.output.qualified_name_constant_index,
                qualified_name_is_canonical: self.output.qualified_name_is_canonical,
                uncached_percent_constant: self.context.has_uncached_percent_constant(),
                metadata_ids,
                metadata_tuple_properties: Some(metadata_tuple_properties),
                subtree_has_frozenset_constants: self.output.has_frozenset_constants,
                metadata_alias_scan_unnecessary: false,
                fresh_empty_bytes_constant: self.context.fresh_empty_bytes_constant(),
            },
        };
        code.validate()?;
        code.marshal_provenance.assembly_order = self.context.record_code_assembly(&code);
        if matches!(self.unit.scope, Scope::Module) {
            code.marshal_provenance.interning_events = self.context.take_interning_events();
            code.marshal_provenance.interned_metadata_tuples =
                self.context.take_interned_metadata_tuples();
            code.marshal_provenance.metadata_alias_scan_unnecessary =
                metadata_alias_scan_unnecessary(&code);
        }
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
            first_line_number: spec.first_line_number,
            annotation_thunk: false,
        }
    }
}
