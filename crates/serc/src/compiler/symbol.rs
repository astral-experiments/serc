use std::borrow::Cow;
use std::rc::Rc;

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use ruff_python_ast::{Expr, ExprContext, Pattern, Stmt};
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::expression::{InterpolatedStringComponent, format_spec_components, fstring_components};
use super::scope::collect_target_names;
use super::{
    BINARY_OP, BUILD_TUPLE, CALL_INTRINSIC_1, CALL_INTRINSIC_2, COMPARE_OP, CONVERT_VALUE,
    DELETE_DEREF, DELETE_FAST, DELETE_GLOBAL, DELETE_NAME, LOAD_COMMON_CONSTANT, LOAD_DEREF,
    LOAD_FAST, LOAD_FAST_CHECK, LOAD_FROM_DICT_OR_DEREF, LOAD_FROM_DICT_OR_GLOBALS, LOAD_GLOBAL,
    LOAD_LOCALS, LOAD_NAME, LOAD_SPECIAL, RERAISE, RESUME, SET_FUNCTION_ATTRIBUTE, STORE_DEREF,
    STORE_FAST, STORE_GLOBAL, STORE_NAME, YIELD_VALUE,
};
use crate::assembler::{EmittedInstruction, EmittedJump, ExceptionPolicy, Label, SourceLocation};
use crate::code_object::Constant;
use crate::target::code_flags::{CO_ASYNC_GENERATOR, CO_COROUTINE, CO_FUTURE_ANNOTATIONS};
use crate::target::operands::{
    BinaryIntrinsic, BinaryOperation, CommonConstant, ComparisonOperation, Conversion,
    FunctionAttribute, ResumeLocation, SpecialMethod, UnaryIntrinsic,
};
use crate::target::stack_counts_for as opcode_stack_counts;
use crate::target::{Opcode, is_unconditional_jump};
use crate::{CompileError, PythonVersion};

use super::RETURN_VALUE;

use super::constant::{
    DeferredConstantTarget, constant_slice, fold_constant, fold_constant_for_version,
    literal_constant, unique_frozenset_constants,
};
use super::scope::{NameId, NameSet, definitely_evaluated_references};
use super::source::unparse_annotation;
use super::{Compiler, Scope, comprehension_parts};

#[derive(Clone, Debug, Default)]
pub(super) struct SymbolState {
    pub(super) locals: Vec<NameId>,
    pub(super) fast_local_count: usize,
    pub(super) cell_names: NameSet,
    pub(super) free_names: Vec<NameId>,
    pub(super) temporary_indices: FxHashMap<NameId, u32>,
    pub(super) hidden_names: NameSet,
    pub(super) active_temporaries: NameSet,
    pub(super) active_temporary_fast_cells: NameSet,
    pub(super) active_temporary_free_cells: NameSet,
    pub(super) initialized_locals: NameSet,
    pub(super) owned_load_locals: NameSet,
    pub(super) imported_scope_names: NameSet,
    pub(super) type_parameter_names: NameSet,
    pub(super) module_globals: NameSet,
}

#[derive(Clone, Copy)]
enum NameMutation {
    Store,
    Delete,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ComparisonMode {
    Value,
    Boolean,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResumeDepth {
    Zero,
    One,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ComprehensionRestore {
    pub(super) index: u32,
    pub(super) name: NameId,
}

impl ResumeDepth {
    const fn is_one(self) -> bool {
        matches!(self, Self::One)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReraiseStackState {
    /// Only the exception remains, as in comprehension and exception-group cleanup.
    ExceptionOnly,
    /// The exception is being propagated from an active exception or finally handler.
    ActiveExceptionHandler,
    /// The generator or coroutine stop-iteration handler retains physical interpreter state that
    /// is not part of the compiler's logical stack.
    StopIterationHandler,
    /// CPython's standard `[exc_info, lasti, exc]` handler-cleanup sequence.
    HandlerCleanup,
    /// An unsuccessful `with` handler retains five values, or four before Python 3.14.
    WithHandler,
}

impl ReraiseStackState {
    pub(super) const fn argument_and_effect(self) -> (u32, i32) {
        match self {
            Self::ExceptionOnly => (0, -1),
            Self::ActiveExceptionHandler => (0, -2),
            Self::StopIterationHandler => (1, -1),
            Self::HandlerCleanup => (1, -3),
            Self::WithHandler => (2, -5),
        }
    }
}

/// Whether lowering this definition or a nested definition can assemble annotation locals.
fn definition_contains_annotation_scope(statement: &Stmt) -> bool {
    struct AnnotationScope(bool);
    impl<'ast> Visitor<'ast> for AnnotationScope {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            if matches!(statement, Stmt::TypeAlias(_)) {
                self.0 = true;
            } else if !self.0 {
                walk_stmt(self, statement);
            }
        }

        fn visit_annotation(&mut self, _annotation: &'ast Expr) {
            self.0 = true;
        }

        fn visit_type_params(&mut self, _params: &'ast ruff_python_ast::TypeParams) {
            self.0 = true;
        }

        fn visit_expr(&mut self, _expression: &'ast Expr) {}
    }

    let mut annotations = AnnotationScope(false);
    annotations.visit_stmt(statement);
    annotations.0
}

impl Compiler<'_> {
    fn name_id(&self, name: &str) -> Option<NameId> {
        self.context.scopes.names.get(name)
    }

    pub(super) fn required_name_id(&self, name: &str) -> NameId {
        self.name_id(name)
            .unwrap_or_else(|| panic!("module analysis did not intern `{name}`"))
    }

    /// Resolve an instruction's spelling against a canonical generic-header plan.
    ///
    /// Ordinary scopes keep their existing raw-name lookup. In a generic header,
    /// plans contain declaration-time symbol identities, while instruction lookup
    /// uses the completed type-parameter mangling restriction.
    pub(super) fn instruction_name_id(&self, name: &str, raw: NameId) -> NameId {
        if self.unit.header_symbol_names_are_final {
            self.required_name_id(&self.mangled_name_cow(name))
        } else {
            raw
        }
    }

    /// Return the symbol-table identity frozen at this expression's declaration.
    pub(super) fn symbol_analysis_name_id(&self, name: &str) -> NameId {
        if self.unit.header_symbol_names_are_final {
            self.required_name_id(&self.symbol_analysis_name(name))
        } else {
            self.required_name_id(name)
        }
    }

    pub(super) fn name_ids<I, S>(&self, names: I) -> Vec<NameId>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        names
            .into_iter()
            .map(|name| self.required_name_id(name.as_ref()))
            .collect()
    }

    pub(super) fn name_id_set<I, S>(&self, names: I) -> FxHashSet<NameId>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        names
            .into_iter()
            .map(|name| self.required_name_id(name.as_ref()))
            .collect()
    }

    fn name_strings(&self, names: impl IntoIterator<Item = NameId>) -> Vec<String> {
        let interner = &self.context.scopes.names;
        names
            .into_iter()
            .map(|name| interner.resolve(name).to_string())
            .collect()
    }

    pub(super) fn insert_lexical_name(&self, names: &mut Vec<NameId>, name: NameId) {
        self.context.scopes.names.insert_lexical(names, name);
    }

    pub(super) fn contains_name(&self, names: &NameSet, name: &str) -> bool {
        self.name_id(name)
            .is_some_and(|raw| names.contains(self.instruction_name_id(name, raw)))
    }

    pub(super) fn resolve_name(&self, name: NameId) -> &str {
        self.context.scopes.names.resolve(name)
    }

    pub(super) fn scope_index(&self, indices: &FxHashMap<NameId, u32>, name: &str) -> Option<u32> {
        self.name_id(name)
            .and_then(|raw| indices.get(&self.instruction_name_id(name, raw)).copied())
    }

    pub(super) fn is_cell(&self, name: &str) -> bool {
        self.contains_name(&self.symbols.cell_names, name)
    }

    pub(super) fn is_cell_id(&self, name: NameId) -> bool {
        self.symbols.cell_names.contains(name)
    }

    pub(super) fn is_hidden_id(&self, name: NameId) -> bool {
        self.symbols.hidden_names.contains(name)
    }

    pub(super) fn is_initialized(&self, name: &str) -> bool {
        self.contains_name(&self.symbols.initialized_locals, name)
    }

    pub(super) fn is_imported(&self, name: &str) -> bool {
        self.contains_name(&self.symbols.imported_scope_names, name)
    }

    pub(super) fn is_type_parameter_id(&self, name: NameId) -> bool {
        self.symbols.type_parameter_names.contains(name)
    }

    pub(super) fn temporary_index(&self, name: &str) -> Option<u32> {
        self.name_id(name).and_then(|raw| {
            let symbol = if self.unit.header_symbol_names_are_final {
                self.symbol_analysis_name_id(name)
            } else {
                raw
            };
            self.symbols.temporary_indices.get(&symbol).copied()
        })
    }

    fn active_temporary_index(&self, name: NameId) -> u32 {
        if self.symbols.active_temporary_free_cells.contains(name)
            && let Scope::Function { free_indices, .. } = &self.unit.scope
        {
            return free_indices[&name];
        }
        self.symbols
            .temporary_indices
            .get(&name)
            .copied()
            .expect("active temporary should have an index")
    }

    /// A saved descendant target does not turn an unrelated global use into a local.
    pub(super) fn active_temporary_binds_name(&self, name: NameId) -> bool {
        if !self.symbols.active_temporaries.contains(name) {
            return false;
        }
        if self.control.generators.generator_expression
            && matches!(&self.unit.scope, Scope::Function { globals, .. } if globals.contains(name))
        {
            return self
                .control
                .comprehensions
                .active_comprehension_local_names
                .iter()
                .any(|locals| locals.contains(name));
        }
        true
    }

    pub(super) fn load_name(&mut self, name: &str) -> Result<(), CompileError> {
        let raw_name_id = self.required_name_id(name);
        let name_id = self.instruction_name_id(name, raw_name_id);
        if self.active_temporary_binds_name(name_id) {
            let index = self.active_temporary_index(name_id);
            return self.emit(
                if self.symbols.cell_names.contains(name_id)
                    && !self.symbols.active_temporary_fast_cells.contains(name_id)
                {
                    LOAD_DEREF
                } else {
                    LOAD_FAST
                },
                index,
                1,
            );
        }
        let annotation_classdict_index = self
            .unit
            .annotation_classdict_index
            .filter(|_| !self.unit.annotation_thunk || !self.class_scope_declares_global(name_id));
        match &self.unit.scope {
            Scope::Module => {
                let index = self.name_index_with_id(name, name_id)?;
                if self.symbols.module_globals.contains(name_id) {
                    self.emit(LOAD_GLOBAL, index << 1, 1)
                } else {
                    self.emit(LOAD_NAME, index, 1)
                }
            }
            Scope::Class { free_indices, .. } => {
                let in_comprehension = !self
                    .control
                    .comprehensions
                    .active_comprehension_local_names
                    .is_empty();
                if self.class_scope_declares_global(name_id)
                    && !(in_comprehension && free_indices.contains_key(&name_id))
                {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(LOAD_GLOBAL, index << 1, 1)
                } else if let Some(index) = free_indices.get(&name_id).copied()
                    // A cell carried through for a method does not change class-local lookup.
                    && (!self.class_scope_binds_name(name) || in_comprehension)
                {
                    if in_comprehension {
                        self.emit(LOAD_DEREF, index, 1)
                    } else {
                        self.emit(LOAD_LOCALS, 0, 1)?;
                        self.emit(LOAD_FROM_DICT_OR_DEREF, index, 0)
                    }
                } else {
                    let index = self.name_index_with_id(name, name_id)?;
                    if in_comprehension {
                        // The first iterable runs in the class namespace, but the inlined
                        // comprehension body falls back to globals as its former implicit
                        // function scope would.
                        self.emit(LOAD_GLOBAL, index << 1, 1)
                    } else {
                        self.emit(LOAD_NAME, index, 1)
                    }
                }
            }
            Scope::Function {
                indices,
                free_indices,
                cells,
                globals,
            } => {
                if globals.contains(name_id) {
                    if let Some(classdict) = annotation_classdict_index
                        && name != "__classdict__"
                        && !self.class_scope_declares_global(name_id)
                    {
                        self.emit(LOAD_DEREF, classdict, 1)?;
                        let name = self.annotation_mangled_name(name);
                        let index = self.name_index(&name)?;
                        return self.emit(LOAD_FROM_DICT_OR_GLOBALS, index, 0);
                    }
                    let index = self.name_index_with_id(name, name_id)?;
                    return self.emit(LOAD_GLOBAL, index << 1, 1);
                }
                if let Some(index) = free_indices.get(&name_id).copied() {
                    // An inlined comprehension can use a temporary fast-local slot with the
                    // same spelling as a free variable. Outside the comprehension, resolve the
                    // name through the closure. Class annotations check their namespace first,
                    // even when the source name is `__classdict__` itself.
                    if let Some(classdict) = annotation_classdict_index {
                        self.emit(LOAD_DEREF, classdict, 1)?;
                        return self.emit(LOAD_FROM_DICT_OR_DEREF, index, 0);
                    }
                    return self.emit(LOAD_DEREF, index, 1);
                }
                if let Some(index) = indices.get(&name_id).copied() {
                    if cells.contains(name_id) {
                        if let Some(classdict) = annotation_classdict_index
                            && name != "__classdict__"
                        {
                            self.emit(LOAD_DEREF, classdict, 1)?;
                            self.emit(LOAD_FROM_DICT_OR_DEREF, index, 0)
                        } else {
                            self.emit(LOAD_DEREF, index, 1)
                        }
                    } else if self.symbols.initialized_locals.contains(name_id) {
                        let emitted = if self.symbols.owned_load_locals.contains(name_id) {
                            self.emit_owned_fast_instruction(index)?
                        } else {
                            self.emit_instruction(LOAD_FAST, index)?
                        };
                        if self
                            .control
                            .take_direct_comprehension_assignment_result(name_id)
                        {
                            self.assembler.mark_load_fast_as_known_initialized(emitted);
                        }
                        Ok(())
                    } else {
                        self.emit(LOAD_FAST_CHECK, index, 1)
                    }
                } else {
                    if self.unit.header_symbol_names_are_final {
                        // Code generation can mangle a later type parameter to a
                        // spelling absent from this declaration-time symbol table.
                        let index = self.name_index_with_id(name, name_id)?;
                        return self.emit(LOAD_NAME, index, 1);
                    }
                    if let Some(classdict) = annotation_classdict_index {
                        self.emit(LOAD_DEREF, classdict, 1)?;
                        let index = self.name_index_with_id(name, name_id)?;
                        return self.emit(LOAD_FROM_DICT_OR_GLOBALS, index, 0);
                    }
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(LOAD_GLOBAL, index << 1, 1)
                }
            }
        }
    }

    pub(super) fn mark_definitely_evaluated_locals(&mut self, expression: &Expr) {
        let Scope::Function { indices, .. } = &self.unit.scope else {
            return;
        };
        let Some(references) = definitely_evaluated_references(expression) else {
            return;
        };
        let names = &self.context.scopes.names;
        if !self.unit.header_symbol_names_are_final {
            self.symbols
                .initialized_locals
                .extend(references.into_iter().filter_map(|reference| {
                    names
                        .get(reference)
                        .filter(|name| indices.contains_key(name))
                }));
            return;
        }
        let initialized = references
            .into_iter()
            .filter_map(|reference| {
                names
                    .get(reference)
                    .map(|raw| self.instruction_name_id(reference, raw))
                    .filter(|name| indices.contains_key(name))
            })
            .collect::<Vec<_>>();
        self.symbols.initialized_locals.extend(initialized);
    }

    pub(super) fn store_name(&mut self, name: &str) -> Result<(), CompileError> {
        let name_id = self.required_name_id(name);
        self.mutate_name_with_id(name, name_id, NameMutation::Store)
    }

    pub(super) fn store_name_id(&mut self, name_id: NameId) -> Result<(), CompileError> {
        let context = Rc::clone(&self.context);
        let name = context.scopes.names.resolve(name_id);
        self.mutate_name_with_id(name, name_id, NameMutation::Store)
    }

    pub(super) fn delete_name(&mut self, name: &str) -> Result<(), CompileError> {
        let name_id = self.required_name_id(name);
        self.mutate_name_with_id(name, name_id, NameMutation::Delete)
    }

    pub(super) fn delete_name_id(&mut self, name_id: NameId) -> Result<(), CompileError> {
        let context = Rc::clone(&self.context);
        let name = context.scopes.names.resolve(name_id);
        self.mutate_name_with_id(name, name_id, NameMutation::Delete)
    }

    fn mutate_name_with_id(
        &mut self,
        name: &str,
        name_id: NameId,
        mutation: NameMutation,
    ) -> Result<(), CompileError> {
        let name_id = self.instruction_name_id(name, name_id);
        let (global, namespace, deref, fast, effect) = match mutation {
            NameMutation::Store => (STORE_GLOBAL, STORE_NAME, STORE_DEREF, STORE_FAST, -1),
            NameMutation::Delete => (DELETE_GLOBAL, DELETE_NAME, DELETE_DEREF, DELETE_FAST, 0),
        };
        if self.active_temporary_binds_name(name_id) {
            let index = self.active_temporary_index(name_id);
            return self.emit(
                if self.symbols.cell_names.contains(name_id)
                    && !self.symbols.active_temporary_fast_cells.contains(name_id)
                {
                    deref
                } else {
                    fast
                },
                index,
                effect,
            );
        }
        match &self.unit.scope {
            Scope::Module => {
                let index = self.name_index_with_id(name, name_id)?;
                if self.symbols.module_globals.contains(name_id) {
                    self.emit(global, index, effect)
                } else {
                    self.emit(namespace, index, effect)
                }
            }
            Scope::Class {
                nonlocals,
                free_indices,
                ..
            } => {
                if self.class_scope_declares_global(name_id) {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(global, index, effect)
                } else if nonlocals.contains(name_id) {
                    let index = free_indices.get(&name_id).copied().ok_or_else(|| {
                        CompileError::Internal(format!("missing class nonlocal variable `{name}`"))
                    })?;
                    self.emit(deref, index, effect)
                } else if let Some(index) = free_indices.get(&name_id).copied()
                    && !self.class_scope_binds_name(name)
                {
                    // Compiler-generated class assignments use the same symbol classification
                    // as explicit accesses. Only explicit class bindings force a namespace write.
                    self.emit(deref, index, effect)
                } else {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(namespace, index, effect)
                }
            }
            Scope::Function {
                indices,
                free_indices,
                cells,
                globals,
            } => {
                if globals.contains(name_id) {
                    let index = self.name_index_with_id(name, name_id)?;
                    return self.emit(global, index, effect);
                }
                // An inactive comprehension temporary does not replace a nonlocal binding.
                if let Some(index) = free_indices.get(&name_id).copied() {
                    self.emit(deref, index, effect)
                } else if let Some(index) = indices.get(&name_id).copied() {
                    match mutation {
                        NameMutation::Store => self.symbols.initialized_locals.insert(name_id),
                        NameMutation::Delete => self.symbols.initialized_locals.remove(name_id),
                    };
                    if cells.contains(name_id) {
                        self.emit(deref, index, effect)
                    } else {
                        self.emit(fast, index, effect)
                    }
                } else {
                    if self.unit.header_symbol_names_are_final {
                        let index = self.name_index_with_id(name, name_id)?;
                        return self.emit(namespace, index, effect);
                    }
                    Err(CompileError::Internal(format!(
                        "missing local variable `{name}`"
                    )))
                }
            }
        }
    }

    pub(super) fn closure_index(&self, name: &str) -> Result<u32, CompileError> {
        let name_id = self.required_name_id(name);
        self.closure_index_id(name_id)
    }

    pub(super) fn closure_index_id(&self, name: NameId) -> Result<u32, CompileError> {
        if self.symbols.active_temporaries.contains(name)
            && self.symbols.active_temporary_free_cells.contains(name)
        {
            return Ok(self.active_temporary_index(name));
        }
        if (!self.is_cell_id(name)
            || self.context.python_version <= PythonVersion::Py313
                && !self
                    .control
                    .comprehensions
                    .active_comprehension_local_names
                    .is_empty())
            && let Scope::Class { free_indices, .. } = &self.unit.scope
            && let Some(index) = free_indices.get(&name)
            && !(self.is_cell_id(name)
                && (matches!(self.resolve_name(name), "__class__" | "__classdict__")
                    || self
                        .control
                        .comprehensions
                        .active_comprehension_local_names
                        .iter()
                        .any(|locals| locals.contains(name))))
        {
            // Plain comprehension temporaries cannot supply an enclosing free cell.
            // Within a legacy comprehension, other captured names use enclosing free cells.
            // Own class cells and active comprehension target cells take precedence.
            return Ok(*index);
        }
        match &self.unit.scope {
            Scope::Module => self
                .symbols
                .temporary_indices
                .get(&name)
                .copied()
                .filter(|_| self.is_cell_id(name))
                .ok_or_else(|| {
                    CompileError::Internal(format!(
                        "module cannot provide closure variable `{}`",
                        self.resolve_name(name)
                    ))
                }),
            Scope::Class { free_indices, .. } => self.symbols.locals
                [..self.symbols.locals.len() - self.symbols.free_names.len()]
                .iter()
                .position(|local| *local == name)
                .map(|index| to_u32(index, "class closure variable index"))
                .transpose()?
                .or_else(|| free_indices.get(&name).copied())
                .ok_or_else(|| {
                    let name = self.resolve_name(name);
                    let locals = self.name_strings(self.symbols.locals.iter().copied());
                    let free_names = self.name_strings(self.symbols.free_names.iter().copied());
                    CompileError::Internal(format!(
                        "class `{}` cannot provide closure variable `{name}` (locals: {:?}; free names: {:?})",
                        self.unit.qualified_name, locals, free_names
                    ))
                }),
            Scope::Function {
                indices,
                free_indices,
                ..
            } => (if self.symbols.active_temporaries.contains(name) {
                indices.get(&name).or_else(|| free_indices.get(&name))
            } else {
                free_indices.get(&name).or_else(|| indices.get(&name))
            })
                .copied()
                .ok_or_else(|| {
                    CompileError::Internal(format!(
                        "missing closure variable `{}`",
                        self.resolve_name(name)
                    ))
                }),
        }
    }

    /// Resolve a child's declaration-time closure identity in its parent.
    ///
    /// Header descendants use declaration-time names, including passthrough
    /// captures whose spelling differs from the completed codegen name.
    pub(super) fn child_closure_index(&self, name: NameId) -> Result<u32, CompileError> {
        if self.unit.header_symbol_names_are_final {
            return self.physical_closure_index(name)?.ok_or_else(|| {
                CompileError::Internal(format!(
                    "missing generic-header closure variable `{}`",
                    self.resolve_name(name)
                ))
            });
        }
        self.closure_index_id(name)
    }

    pub(super) fn can_provide_closure(&self, name: &str) -> bool {
        let symbol_name = if self.unit.header_symbol_names_are_final {
            Cow::Owned(self.symbol_analysis_name(name))
        } else {
            Cow::Borrowed(name)
        };
        let Some(name_id) = self.name_id(&symbol_name) else {
            return false;
        };
        self.can_provide_closure_id(name_id)
    }

    pub(super) fn can_provide_closure_id(&self, name: NameId) -> bool {
        if self.control.generators.generator_expression
            && matches!(&self.unit.scope, Scope::Function { globals, .. } if globals.contains(name))
            && !self.active_temporary_binds_name(name)
        {
            return false;
        }
        match &self.unit.scope {
            Scope::Module => {
                // Module cells belong to active inlined comprehensions, not module bindings.
                self.symbols.cell_names.contains(name)
                    && self.symbols.active_temporaries.contains(name)
            }
            Scope::Class { free_indices, .. } => {
                (self.symbols.cell_names.contains(name)
                    && (!self.symbols.hidden_names.contains(name)
                        || self.symbols.active_temporaries.contains(name)))
                    || free_indices.contains_key(&name)
            }
            Scope::Function {
                indices,
                free_indices,
                cells,
                globals,
            } => {
                indices.contains_key(&name)
                    && cells.contains(name)
                    && (!globals.contains(name) || self.symbols.active_temporaries.contains(name))
                    || free_indices.contains_key(&name)
            }
        }
    }

    /// Resolve an emitted capture identity without applying the current source-name context.
    pub(super) fn physical_closure_index(&self, name: NameId) -> Result<Option<u32>, CompileError> {
        let spelling = self.resolve_name(name);
        if self.unit.header_symbol_names_are_final
            && self.unit.generic_target_qualified_name.is_some()
            && let Scope::Function { indices, cells, .. } = &self.unit.scope
            && cells.contains(name)
            && let Some(index) = indices.get(&name)
        {
            return Ok(Some(*index));
        }
        // Own synthetic class cells and active comprehension cells retain their precedence.
        for candidate in self.symbols.cell_names.iter_unordered() {
            let matches = if self.unit.header_symbol_names_are_final {
                candidate == name
            } else {
                self.mangled_name_cow(self.resolve_name(candidate)) == spelling
            };
            if matches
                && (!self.symbols.hidden_names.contains(candidate)
                    || self.symbols.active_temporaries.contains(candidate))
                && self.can_provide_closure_id(candidate)
            {
                return self.closure_index_id(candidate).map(Some);
            }
        }
        let start = self.symbols.locals.len() - self.symbols.free_names.len();
        for (index, candidate) in self.symbols.free_names.iter().enumerate() {
            let matches =
                if self.unit.free_names_are_final || self.unit.header_symbol_names_are_final {
                    *candidate == name
                } else {
                    self.mangled_name_cow(self.resolve_name(*candidate)) == spelling
                };
            if matches {
                return to_u32(start + index, "physical closure variable index").map(Some);
            }
        }
        Ok(None)
    }

    pub(super) fn emit_closure_tuple(&mut self, names: &[NameId]) -> Result<(), CompileError> {
        let indices = names
            .iter()
            .map(|name| self.child_closure_index(*name))
            .collect::<Result<Vec<_>, _>>()?;
        self.emit_closure_indices(&indices)
    }

    /// Emit a closure tuple whose lexical identities have already resolved to physical slots.
    pub(super) fn emit_closure_indices(&mut self, indices: &[u32]) -> Result<(), CompileError> {
        for &local in indices {
            if self.context.python_version == PythonVersion::Py312 {
                self.emit(crate::target::opcodes::LOAD_CLOSURE, local, 1)?;
                continue;
            }
            let emitted = self.emit_instruction(LOAD_FAST, local)?;
            // CPython lowers pseudo `LOAD_CLOSURE` instructions after checking ordinary fast
            // loads for initialization and after forming superinstructions.
            self.assembler
                .mark_load_fast_as_post_analysis_closure(emitted);
            self.assembler
                .prevent_instruction_fusion_with_previous(emitted);
        }
        self.emit_build(BUILD_TUPLE, indices.len())
    }

    pub(super) fn emit_pending_comprehension_restores(
        &mut self,
        single_name_target: bool,
    ) -> Result<(), CompileError> {
        let restores =
            std::mem::take(&mut self.control.comprehensions.pending_comprehension_restores);
        let original_tail_location = if single_name_target && restores.len() == 1 {
            self.assembler
                .last_fast_store_location()
                .filter(|(index, _)| restores[0].0.iter().all(|restore| restore.index != *index))
                .map(|(_, location)| location)
        } else {
            None
        };
        for (restores, location) in restores {
            self.assembler.set_location(location);
            let mut last_restore = None;
            // Static SWAP elimination moves the assignment before the reversed restores,
            // exchanging it with the last saved local's first restore instruction.
            let (last, preceding) = restores
                .split_last()
                .expect("pending restores should not be empty");
            let ordered_restores = preceding.iter().rev().chain(std::iter::once(last));
            for (position, restore) in ordered_restores.copied().enumerate() {
                self.assembler.fusion_barrier();
                if position > 0 {
                    self.assembler.set_location(location);
                }
                let emitted = self.emit_comprehension_restore(restore)?;
                self.assembler
                    .prevent_instruction_fusion_with_previous(emitted);
                last_restore = Some(emitted);
            }
            let last_restore = last_restore.expect("pending restores should not be empty");
            // The SWAP/STORE_FAST_MAYBE_NULL restore contracts by one CFG slot.
            self.assembler.add_last_hidden_exception_owner_slots(1);
            // CPython converts STORE_FAST_MAYBE_NULL only after inserting superinstructions.
            self.assembler.prevent_instruction_fusion(last_restore);
            if let Some(location) = original_tail_location {
                self.assembler
                    .record_original_fallthrough_location(last_restore, location);
            }
        }
        Ok(())
    }

    pub(super) fn emit_comprehension_restore(
        &mut self,
        restore: ComprehensionRestore,
    ) -> Result<EmittedInstruction, CompileError> {
        let emitted = self.emit_instruction(STORE_FAST, restore.index)?;
        // Header saves can give distinct symbols the same cell storage slot.
        // Preserve CPython's STORE_FAST_MAYBE_NULL through duplicate-store and
        // fusion optimizations, including cell saves on Python 3.13 and later.
        if usize::try_from(restore.index).is_ok_and(|index| index < self.symbols.fast_local_count)
            && (!self.symbols.cell_names.contains(restore.name)
                || self.unit.header_symbol_names_are_final
                    && self.context.python_version >= PythonVersion::Py313)
        {
            self.assembler
                .mark_store_fast_as_maybe_null_comprehension_restore(emitted);
        }
        Ok(emitted)
    }

    pub(super) fn child_function_is_nested(&self) -> bool {
        match self.unit.scope {
            Scope::Function { .. } => true,
            Scope::Class { .. } => self.unit.class_scope_is_nested,
            Scope::Module => false,
        }
    }

    fn child_qualified_name_with_global_scope(
        &self,
        name: &str,
        honor_explicit_global: bool,
    ) -> String {
        if let Some(parent) = &self.unit.child_qualified_name_parent {
            return if parent.is_empty() {
                name.to_string()
            } else {
                format!("{parent}.{name}")
            };
        }
        if self.unit.annotation_thunk
            && let Some(prefix) = self.unit.qualified_name.strip_suffix("__annotate__")
        {
            return format!("{prefix}{name}");
        }
        match &self.unit.scope {
            Scope::Module => name.to_string(),
            Scope::Class { .. }
                if honor_explicit_global
                    && self
                        .name_id(name)
                        .is_some_and(|name| self.class_scope_declares_global(name)) =>
            {
                name.to_string()
            }
            Scope::Function { globals, .. }
                if honor_explicit_global && self.contains_name(globals, name) =>
            {
                name.to_string()
            }
            Scope::Class { .. } => format!("{}.{}", self.unit.qualified_name, name),
            Scope::Function { .. } => format!("{}.<locals>.{name}", self.unit.qualified_name),
        }
    }

    pub(super) fn child_qualified_name(&self, name: &str) -> String {
        self.child_qualified_name_with_global_scope(name, true)
    }

    pub(super) fn annotation_child_qualified_name(&self, name: &str) -> String {
        self.child_qualified_name_with_global_scope(name, false)
    }

    fn annotation_mangled_name(&self, name: &str) -> String {
        if self.unit.private_name.is_some() {
            return self.mangled_name(name);
        }
        if !name.starts_with("__") || name.ends_with("__") || name.contains('.') {
            return name.to_string();
        }
        let Some(class_name) = self
            .unit
            .qualified_name
            .strip_suffix(".__annotate__")
            .and_then(|parent| parent.rsplit('.').next())
            .map(|name| name.trim_start_matches('_'))
            .filter(|name| !name.is_empty())
        else {
            return name.to_string();
        };
        format!("_{class_name}{name}")
    }

    pub(super) fn mangled_name(&self, name: &str) -> String {
        self.mangled_name_cow(name).into_owned()
    }

    /// Resolve a source name as it was seen while analyzing the header.
    ///
    /// The wrapper advances `pending_private_type_parameters` as it visits
    /// declarations. Descendant annotation scopes retain a frozen copy.
    pub(super) fn symbol_analysis_name(&self, name: &str) -> String {
        if self
            .name_id(name)
            .is_some_and(|name_id| self.unit.pending_private_type_parameters.contains(&name_id))
        {
            name.to_owned()
        } else {
            self.mangled_name(name)
        }
    }

    pub(super) fn mangled_name_cow<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self
            .unit
            .mangled_names
            .as_ref()
            .is_some_and(|names| !names.contains(name))
        {
            return Cow::Borrowed(name);
        }
        mangle_name(name, self.unit.private_name.as_deref())
    }

    pub(super) fn pre_register_suite_names(&mut self, body: &[Stmt]) -> Result<(), CompileError> {
        // CPython visits discarded branches before pruning their instructions. Constants reused
        // by surviving code keep the position reserved by that first visit.
        self.pre_register_suite_names_filtered(body, true, None)
    }

    pub(super) fn pre_register_conditionally_discarded_suite(
        &mut self,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        // CPython folds discarded conditional blocks before pruning them. Preserve
        // those construction points when exit copying reorders deferred constants.
        let first_fold = self.output.deferred_constants.len();
        self.pre_register_suite_names_with_folds(body, true, true, None, None)?;
        self.record_discarded_fold_admissions(first_fold, None);
        Ok(())
    }

    pub(super) fn pre_register_suite_names_filtered(
        &mut self,
        body: &[Stmt],
        register_literals: bool,
        first_expression: Option<&Expr>,
    ) -> Result<(), CompileError> {
        self.pre_register_suite_names_with_folds(
            body,
            register_literals,
            false,
            first_expression,
            None,
        )
    }

    pub(super) fn pre_register_discarded_expression(
        &mut self,
        expression: &Expr,
        admission: Option<Label>,
    ) -> Result<(), CompileError> {
        // Both arms are lowered before a constant condition is pruned. Literals and
        // values folded in the discarded arm keep their original cache admission order.
        let first_fold = self.output.deferred_constants.len();
        self.pre_register_suite_names_with_folds(&[], true, true, None, Some(expression))?;
        self.record_discarded_fold_admissions(first_fold, admission);
        Ok(())
    }

    pub(super) fn record_discarded_fold_admissions(
        &mut self,
        first_fold: usize,
        admission: Option<Label>,
    ) {
        if self.output.deferred_constants.len() > first_fold {
            let admission = admission.unwrap_or_else(|| {
                let label = self.assembler.label();
                self.assembler.mark(label);
                label
            });
            for (target, _) in &mut self.output.deferred_constants[first_fold..] {
                debug_assert!(target.is_none());
                *target = Some(DeferredConstantTarget::DiscardedExpression(admission));
            }
        }
    }

    fn pre_register_suite_names_with_folds(
        &mut self,
        body: &[Stmt],
        register_literals: bool,
        register_folds: bool,
        first_expression: Option<&Expr>,
        expression_only: Option<&Expr>,
    ) -> Result<(), CompileError> {
        // Run cache admissions and real child builders in source order. Each child action owns
        // its defaults, decorators, and nested scopes; the disposable parent is never assembled.
        enum SkippedAction<'ast> {
            Origins(Vec<(Constant, bool)>),
            StaticAttribute(&'ast str),
            Statement(&'ast Stmt),
            Expression(&'ast Expr),
            FirstExpression(&'ast Expr),
        }

        #[derive(Default)]
        struct UnwindExitDetector(bool);

        impl<'ast> Visitor<'ast> for UnwindExitDetector {
            fn visit_stmt(&mut self, statement: &'ast Stmt) {
                if self.0 || matches!(statement, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                    return;
                }
                if matches!(
                    statement,
                    Stmt::Return(_) | Stmt::Break(_) | Stmt::Continue(_)
                ) {
                    self.0 = true;
                } else {
                    walk_stmt(self, statement);
                }
            }
        }

        #[derive(Default)]
        struct Collector<'compiler, 'source, 'ast> {
            actions: Vec<SkippedAction<'ast>>,
            build_discarded: bool,
            names: Vec<(String, bool)>,
            constants: Vec<Constant>,
            folded_expressions: Vec<&'ast Expr>,
            has_untracked_collection: bool,
            reorder_try_suites: bool,
            register_literals: bool,
            register_folds: bool,
            string_origin_constants: Vec<(Constant, bool)>,
            record_string_origins: bool,
            private_name: Option<String>,
            inlined_targets: Vec<String>,
            future_annotations: bool,
            legacy_compiler: Option<&'compiler Compiler<'source>>,
            legacy_annotated_scope: bool,
            source: &'source str,
            python_version: PythonVersion,
        }

        impl<'ast> Collector<'_, '_, 'ast> {
            /// Discarded annotation scopes intern their locals before the code is removed.
            fn needs_discarded_definition(&self, statement: &Stmt) -> bool {
                self.record_string_origins
                    && (self.build_discarded
                        || self.python_version >= PythonVersion::Py314
                            && definition_contains_annotation_scope(statement))
            }

            fn legacy_folded_constant(&self, expression: &Expr) -> Option<Constant> {
                let compiler = self.legacy_compiler?;
                if matches!(
                    expression,
                    Expr::BinOp(_) | Expr::Subscript(_) | Expr::UnaryOp(_) | Expr::Tuple(_)
                ) {
                    compiler.source_aware_fold_constant(expression)
                } else {
                    None
                }
            }

            /// Match legacy whole-display code generation and AST iterable folding.
            fn legacy_collection_constant(
                &self,
                expression: &Expr,
                iterable: bool,
            ) -> Option<Constant> {
                let compiler = self.legacy_compiler?;
                let (elements, is_set) = match expression {
                    Expr::List(list) if list.ctx == ExprContext::Load => {
                        (list.elts.as_slice(), false)
                    }
                    Expr::Set(set) => (set.elts.as_slice(), true),
                    _ => return None,
                };
                if !iterable && elements.len() < 3 {
                    return None;
                }
                let values = elements
                    .iter()
                    .map(|element| compiler.source_aware_fold_constant(element))
                    .collect::<Option<Vec<_>>>()?;
                if is_set {
                    Some(Constant::FrozenSet(unique_frozenset_constants(
                        values,
                        std::convert::identity,
                    )))
                } else {
                    Some(Constant::Tuple(values))
                }
            }

            fn visit_iterable_expression(&mut self, expression: &'ast Expr) {
                if let Some(constant) = self.legacy_collection_constant(expression, true) {
                    // Visit the whole container before any later reuse of its members.
                    self.record_interpolated_constant(constant);
                } else {
                    self.visit_expr(expression);
                }
            }

            /// Reserve legacy constant-key tuples after their values, at each map flush.
            fn visit_legacy_dict(&mut self, items: &'ast [ruff_python_ast::DictItem]) {
                let compiler = self.legacy_compiler.unwrap();
                let mut index = 0;
                while index < items.len() {
                    if items[index].key.is_none() {
                        self.visit_expr(&items[index].value);
                        index += 1;
                        continue;
                    }
                    let start = index;
                    while index < items.len() && items[index].key.is_some() && index - start < 17 {
                        index += 1;
                    }
                    let group = &items[start..index];
                    if (2..=15).contains(&group.len())
                        && let Some(keys) = group
                            .iter()
                            .map(|item| {
                                compiler.source_aware_fold_constant(item.key.as_ref().unwrap())
                            })
                            .collect::<Option<Vec<_>>>()
                    {
                        for item in group {
                            self.visit_expr(&item.value);
                        }
                        self.record_interpolated_constant(Constant::Tuple(keys));
                    } else {
                        for item in group {
                            self.visit_expr(item.key.as_ref().unwrap());
                            self.visit_expr(&item.value);
                        }
                    }
                }
            }

            fn record_interpolated_constant(&mut self, constant: Constant) {
                if self.record_string_origins {
                    self.string_origin_constants.push((constant.clone(), false));
                }
                if self.register_literals {
                    self.constants.push(constant);
                }
            }

            fn visit_interpolated_components(
                &mut self,
                components: Vec<InterpolatedStringComponent<'ast>>,
            ) {
                if components.len() > 30 || components.is_empty() {
                    self.record_interpolated_constant(Constant::String(String::new()));
                }
                if components.len() > 30 {
                    self.names.push(("join".to_string(), true));
                }
                for component in components {
                    match component {
                        InterpolatedStringComponent::Literal(literal) => {
                            self.record_interpolated_constant(literal.into_constant());
                        }
                        InterpolatedStringComponent::Interpolation(interpolation, is_raw) => {
                            self.visit_expr(&interpolation.expression);
                            if let Some(specification) = &interpolation.format_spec {
                                self.visit_interpolated_components(format_spec_components(
                                    &specification.elements,
                                    is_raw,
                                    self.source,
                                ));
                            }
                        }
                    }
                }
            }

            fn flush_origins(&mut self) {
                if !self.string_origin_constants.is_empty() {
                    self.actions.push(SkippedAction::Origins(std::mem::take(
                        &mut self.string_origin_constants,
                    )));
                }
            }

            fn record_key_origin(&mut self, raw: &str) {
                if self.record_string_origins {
                    let mangled = mangle_name(raw, self.private_name.as_deref());
                    let interned = mangled == raw;
                    self.string_origin_constants
                        .push((Constant::String(mangled.into_owned()), interned));
                }
            }

            fn origin_collector(&self, private_name: Option<String>) -> Self {
                Self {
                    record_string_origins: true,
                    private_name,
                    future_annotations: self.future_annotations,
                    legacy_compiler: self.legacy_compiler,
                    source: self.source,
                    ..Self::default()
                }
            }

            fn record_annotation(&mut self, name: &str, annotation: &'ast Expr) {
                self.record_key_origin(name);
                if self.future_annotations {
                    self.string_origin_constants.push((
                        Constant::String(unparse_annotation(annotation, self.source)),
                        false,
                    ));
                } else {
                    self.visit_expr(annotation);
                }
            }

            /// Retain parent-scope admissions for an eager legacy function annotation.
            fn visit_legacy_annotation(&mut self, name: &str, annotation: &'ast Expr) {
                if self.register_literals {
                    self.constants.push(Constant::String(
                        mangle_name(name, self.private_name.as_deref()).into_owned(),
                    ));
                }
                if self.future_annotations {
                    if self.register_literals {
                        self.constants.push(Constant::String(unparse_annotation(
                            annotation,
                            self.source,
                        )));
                    }
                } else {
                    self.visit_expr(annotation);
                }
            }

            fn visit_scope_body(&mut self, body: &'ast [Stmt]) {
                if let Some(Stmt::Expr(statement)) = body.first()
                    && let Some(docstring @ Constant::String(_)) =
                        literal_constant(&statement.value)
                {
                    self.string_origin_constants.push((docstring, false));
                }
                self.visit_body(body);
            }
        }

        impl<'ast> Visitor<'ast> for Collector<'_, '_, 'ast> {
            fn visit_body(&mut self, body: &'ast [Stmt]) {
                let register_folds = self.register_folds;
                for statement in body {
                    self.visit_stmt(statement);
                    if super::statement::suite_terminates(std::slice::from_ref(statement)) {
                        self.register_folds = false;
                    }
                }
                self.register_folds = register_folds;
            }

            fn visit_stmt(&mut self, statement: &'ast Stmt) {
                if crate::stack_growth_required() {
                    return crate::grow_stack(|| self.visit_stmt(statement));
                }

                match statement {
                    Stmt::Expr(expression)
                        if literal_constant(&expression.value).is_some()
                            || self.legacy_folded_constant(&expression.value).is_some() =>
                    {
                        return;
                    }
                    Stmt::AugAssign(assignment) => {
                        // Augmented assignment evaluates its target but does not lower it
                        // through the ordinary attribute Store path.
                        match assignment.target.as_ref() {
                            Expr::Attribute(attribute) => {
                                self.visit_expr(&attribute.value);
                                self.names.push((attribute.attr.as_str().to_string(), true));
                            }
                            target => self.visit_expr(target),
                        }
                        self.visit_expr(&assignment.value);
                        return;
                    }
                    Stmt::AnnAssign(assignment) => {
                        let build = self.build_discarded
                            && self.legacy_compiler.is_some()
                            && self.record_string_origins;
                        if build {
                            self.flush_origins();
                            self.actions.push(SkippedAction::Statement(statement));
                            self.record_string_origins = false;
                        }
                        if let Some(value) = &assignment.value {
                            self.visit_expr(value);
                            self.visit_expr(&assignment.target);
                        } else {
                            // Even an unevaluated annotation still evaluates the
                            // owner or index of a complex assignment target.
                            match assignment.target.as_ref() {
                                Expr::Attribute(attribute) => self.visit_expr(&attribute.value),
                                Expr::Subscript(subscript) => {
                                    self.visit_expr(&subscript.value);
                                    super::walk_annotation_subscript(self, &subscript.slice);
                                }
                                _ => {}
                            }
                        }
                        if self.legacy_annotated_scope {
                            if self.future_annotations && assignment.simple {
                                self.record_interpolated_constant(Constant::String(
                                    unparse_annotation(&assignment.annotation, self.source),
                                ));
                            } else if !self.future_annotations {
                                self.visit_expr(&assignment.annotation);
                            }
                            if assignment.simple {
                                self.names.push(("__annotations__".to_string(), false));
                                if self.register_literals
                                    && let Expr::Name(name) = assignment.target.as_ref()
                                {
                                    self.constants.push(Constant::String(
                                        mangle_name(name.id.as_str(), self.private_name.as_deref())
                                            .into_owned(),
                                    ));
                                }
                            }
                        }
                        if build {
                            self.record_string_origins = true;
                            self.string_origin_constants.clear();
                        }
                        return;
                    }
                    Stmt::For(statement)
                        if self.legacy_compiler.is_some() && !statement.is_async =>
                    {
                        self.visit_iterable_expression(&statement.iter);
                        self.visit_expr(&statement.target);
                        self.visit_body(&statement.body);
                        self.visit_body(&statement.orelse);
                        return;
                    }
                    Stmt::For(statement) if statement.is_async => {
                        self.visit_expr(&statement.iter);
                        // GET_ANEXT loads None before assigning the yielded item, even when
                        // the surrounding suite is removed by later reachability analysis.
                        if self.register_literals {
                            self.constants.push(Constant::None);
                        }
                        self.visit_expr(&statement.target);
                        self.visit_body(&statement.body);
                        self.visit_body(&statement.orelse);
                        return;
                    }
                    Stmt::Try(statement) if self.reorder_try_suites && !statement.is_star => {
                        // Ordinary try codegen visits the else suite before handlers.
                        // Except-star keeps the AST order: handlers before else.
                        self.visit_body(&statement.body);
                        self.visit_body(&statement.orelse);
                        for handler in &statement.handlers {
                            self.visit_except_handler(handler);
                        }
                        self.visit_body(&statement.finalbody);
                        return;
                    }
                    Stmt::FunctionDef(definition) => {
                        let build = self.needs_discarded_definition(statement);
                        if build {
                            self.flush_origins();
                            self.actions.push(SkippedAction::Statement(statement));
                            self.record_string_origins = false;
                        }
                        for decorator in &definition.decorator_list {
                            self.visit_expr(&decorator.expression);
                        }
                        for parameter in definition
                            .parameters
                            .posonlyargs
                            .iter()
                            .chain(&definition.parameters.args)
                        {
                            if let Some(default) = &parameter.default {
                                self.visit_expr(default);
                            }
                        }
                        for parameter in &definition.parameters.kwonlyargs {
                            if let Some(default) = &parameter.default {
                                self.record_key_origin(parameter.name().as_str());
                                self.visit_expr(default);
                            }
                        }
                        if self.legacy_compiler.is_some() && definition.type_params.is_none() {
                            // Legacy non-generic signatures execute in this scope even when
                            // their surrounding instructions are later removed. Keep their
                            // names and reusable constants in the original admission order.
                            let record_string_origins = self.record_string_origins;
                            self.record_string_origins = false;
                            let parameters = &definition.parameters;
                            for parameter in parameters
                                .args
                                .iter()
                                .chain(&parameters.posonlyargs)
                                .map(|parameter| &parameter.parameter)
                                .chain(parameters.vararg.as_deref())
                                .chain(
                                    parameters
                                        .kwonlyargs
                                        .iter()
                                        .map(|parameter| &parameter.parameter),
                                )
                                .chain(parameters.kwarg.as_deref())
                            {
                                if let Some(annotation) = &parameter.annotation {
                                    self.visit_legacy_annotation(
                                        parameter.name.as_str(),
                                        annotation,
                                    );
                                }
                            }
                            if let Some(annotation) = &definition.returns {
                                self.visit_legacy_annotation("return", annotation);
                            }
                            self.record_string_origins = record_string_origins;
                        }
                        if self.record_string_origins {
                            let mut child = self.origin_collector(self.private_name.clone());
                            if let Some(parameters) = &definition.type_params {
                                child.visit_type_params(parameters);
                            }
                            let parameters = &definition.parameters;
                            for parameter in parameters
                                .args
                                .iter()
                                .chain(&parameters.posonlyargs)
                                .map(|parameter| &parameter.parameter)
                                .chain(parameters.vararg.as_deref())
                                .chain(
                                    parameters
                                        .kwonlyargs
                                        .iter()
                                        .map(|parameter| &parameter.parameter),
                                )
                                .chain(parameters.kwarg.as_deref())
                            {
                                if let Some(annotation) = &parameter.annotation {
                                    child.record_annotation(parameter.name.as_str(), annotation);
                                }
                            }
                            if let Some(annotation) = &definition.returns {
                                child.record_annotation("return", annotation);
                            }
                            child.visit_scope_body(&definition.body);
                            self.string_origin_constants
                                .extend(child.string_origin_constants);
                        }
                        if build {
                            self.record_string_origins = true;
                            self.string_origin_constants.clear();
                        }
                        self.names
                            .push((definition.name.as_str().to_string(), false));
                        return;
                    }
                    Stmt::ClassDef(definition) => {
                        let build = self.needs_discarded_definition(statement);
                        if build {
                            self.flush_origins();
                            self.actions.push(SkippedAction::Statement(statement));
                            self.record_string_origins = false;
                        }
                        for decorator in &definition.decorator_list {
                            self.visit_expr(&decorator.expression);
                        }
                        if self.record_string_origins {
                            let mut child =
                                self.origin_collector(Some(definition.name.to_string()));
                            if let Some(parameters) = &definition.type_params {
                                child.visit_type_params(parameters);
                            }
                            child.visit_scope_body(&definition.body);
                            self.string_origin_constants
                                .extend(child.string_origin_constants);
                            self.string_origin_constants
                                .push((Constant::String(definition.name.to_string()), true));
                        }
                        if self.register_literals && definition.type_params.is_none() {
                            self.constants
                                .push(Constant::String(definition.name.to_string()));
                        }
                        if definition.type_params.is_none()
                            && let Some(arguments) = definition.arguments.as_deref()
                        {
                            for argument in &arguments.args {
                                self.visit_expr(argument);
                            }
                            for keyword in &arguments.keywords {
                                self.visit_expr(&keyword.value);
                            }
                        }
                        if build {
                            self.record_string_origins = true;
                            self.string_origin_constants.clear();
                        }
                        self.names
                            .push((definition.name.as_str().to_string(), false));
                        return;
                    }
                    Stmt::With(with_statement)
                        if self.register_literals && !with_statement.is_async =>
                    {
                        walk_stmt(self, statement);
                        self.constants.push(Constant::None);
                        return;
                    }
                    Stmt::TypeAlias(alias) if self.needs_discarded_definition(statement) => {
                        self.flush_origins();
                        self.actions.push(SkippedAction::Statement(statement));
                        if let Expr::Name(name) = alias.name.as_ref() {
                            self.names.push((name.id.to_string(), false));
                        }
                        return;
                    }
                    Stmt::TypeAlias(statement) => {
                        if let Expr::Name(name) = statement.name.as_ref() {
                            self.names.push((name.id.as_str().to_string(), false));
                            if self.record_string_origins {
                                self.string_origin_constants
                                    .push((Constant::String(name.id.to_string()), true));
                                let mut child = self.origin_collector(self.private_name.clone());
                                if let Some(parameters) = &statement.type_params {
                                    child.visit_type_params(parameters);
                                }
                                child.visit_expr(&statement.value);
                                self.string_origin_constants
                                    .extend(child.string_origin_constants);
                            }
                        }
                        return;
                    }
                    Stmt::Import(import) => {
                        if self.legacy_compiler.is_some() && self.register_literals {
                            self.constants.push(Constant::Int(0));
                            self.constants.push(Constant::None);
                        } else if self.python_version == PythonVersion::Py314
                            && self.register_literals
                        {
                            // Import emits None as its from-list before the unreachable
                            // branch is removed. A later use keeps that cache position.
                            self.constants.push(Constant::None);
                        }
                        for alias in &import.names {
                            let imported = alias.name.as_str();
                            self.names.push((imported.to_string(), true));
                            if alias.asname.is_some() {
                                for component in imported.split('.').skip(1) {
                                    self.names.push((component.to_string(), true));
                                }
                            }
                            let bound = alias.asname.as_ref().map_or_else(
                                || imported.split('.').next().unwrap_or(imported),
                                |name| name.as_str(),
                            );
                            self.names.push((bound.to_string(), false));
                        }
                        return;
                    }
                    Stmt::ImportFrom(import) => {
                        let register_legacy_literals =
                            self.legacy_compiler.is_some() && self.register_literals;
                        if self.record_string_origins || register_legacy_literals {
                            let names = Constant::Tuple(
                                import
                                    .names
                                    .iter()
                                    .map(|alias| Constant::String(alias.name.to_string()))
                                    .collect(),
                            );
                            if register_legacy_literals {
                                self.constants.push(Constant::Int(u64::from(import.level)));
                                self.constants.push(names.clone());
                            }
                            if self.record_string_origins {
                                self.string_origin_constants.push((names, true));
                            }
                        }
                        let module = import.module.as_ref().map_or("", |name| name.as_str());
                        self.names.push((module.to_string(), true));
                        for alias in &import.names {
                            if alias.name.as_str() == "*" {
                                // IMPORT_STAR has no IMPORT_FROM or local binding name.
                                continue;
                            }
                            self.names.push((alias.name.as_str().to_string(), true));
                            let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                            self.names.push((bound.as_str().to_string(), false));
                        }
                        return;
                    }
                    _ => {}
                }
                walk_stmt(self, statement);
            }

            fn visit_except_handler(
                &mut self,
                except_handler: &'ast ruff_python_ast::ExceptHandler,
            ) {
                let ruff_python_ast::ExceptHandler::ExceptHandler(handler) = except_handler;
                if let Some(type_) = &handler.type_ {
                    self.visit_expr(type_);
                }
                if let Some(name) = &handler.name {
                    self.names.push((name.as_str().to_string(), false));
                }
                self.visit_body(&handler.body);
            }

            fn visit_type_param(&mut self, parameter: &'ast ruff_python_ast::TypeParam) {
                if self.record_string_origins {
                    self.string_origin_constants
                        .push((Constant::String(parameter.name().to_string()), true));
                }
                ruff_python_ast::visitor::walk_type_param(self, parameter);
            }

            fn visit_expr(&mut self, expression: &'ast Expr) {
                if crate::stack_growth_required() {
                    return crate::grow_stack(|| self.visit_expr(expression));
                }

                if let Some(constant) = self.legacy_collection_constant(expression, false) {
                    self.record_interpolated_constant(constant);
                    return;
                }
                if let Some(constant) = self.legacy_folded_constant(expression) {
                    // Legacy AST folding admits only the result, before the next child
                    // is built. Replaying operands or delaying this action changes which
                    // object a later class-qualified-name constant finds in the cache.
                    if self.record_string_origins {
                        if self.build_discarded {
                            self.flush_origins();
                            self.actions.push(SkippedAction::Expression(expression));
                        } else {
                            self.string_origin_constants.push((constant.clone(), false));
                        }
                    }
                    if self.register_literals {
                        self.constants.push(constant);
                    } else if self.register_folds {
                        self.folded_expressions.push(expression);
                    }
                    return;
                }
                let elements = match expression {
                    Expr::Tuple(tuple) => Some(tuple.elts.as_slice()),
                    Expr::List(list) => Some(list.elts.as_slice()),
                    Expr::Set(set) => Some(set.elts.as_slice()),
                    _ => None,
                };
                if elements.is_some_and(|elements| {
                    elements.iter().any(|element| {
                        fold_constant_for_version(element, self.python_version).is_none()
                    })
                }) {
                    self.has_untracked_collection = true;
                }
                if let Expr::Slice(slice) = expression
                    && self.legacy_compiler.is_some()
                {
                    // Legacy targets load each bound before building the slice. Reused
                    // bounds keep this admission even when the surrounding code is removed.
                    for bound in [&slice.lower, &slice.upper] {
                        if let Some(bound) = bound {
                            self.visit_expr(bound);
                        } else {
                            self.record_interpolated_constant(Constant::None);
                        }
                    }
                    if let Some(step) = &slice.step {
                        self.visit_expr(step);
                    }
                    return;
                }
                if let Expr::Slice(slice) = expression
                    && let Some(constant) = constant_slice(slice)
                {
                    // CPython constructs literal slices during code generation, before
                    // flowgraph folds and before constants in later discarded statements.
                    if self.record_string_origins {
                        self.string_origin_constants.push((constant.clone(), false));
                    }
                    if self.register_literals {
                        self.constants.push(constant);
                    }
                    return;
                }
                if self.register_folds
                    && literal_constant(expression).is_none()
                    && fold_constant_for_version(expression, self.python_version).is_some()
                {
                    self.folded_expressions.push(expression);
                }
                if let Expr::FString(fstring) = expression {
                    self.visit_interpolated_components(fstring_components(fstring, self.source));
                    return;
                }
                if self.record_string_origins
                    && let Some(constant) = literal_constant(expression)
                {
                    // The constant cache sees the original operands before later folding.
                    self.string_origin_constants.push((constant, false));
                }
                if self.register_literals
                    && let Some(constant) = literal_constant(expression)
                {
                    self.constants.push(constant);
                    return;
                }
                match expression {
                    Expr::Dict(dictionary) if self.legacy_compiler.is_some() => {
                        self.visit_legacy_dict(&dictionary.items);
                        return;
                    }
                    Expr::Compare(comparison) if self.legacy_compiler.is_some() => {
                        self.visit_expr(comparison.first_operand());
                        for (index, comparator) in comparison.comparators().iter().enumerate() {
                            if index + 1 == comparison.comparators().len()
                                && matches!(
                                    comparison.ops[index],
                                    ruff_python_ast::CmpOp::In | ruff_python_ast::CmpOp::NotIn
                                )
                            {
                                self.visit_iterable_expression(comparator);
                            } else {
                                self.visit_expr(comparator);
                            }
                        }
                        return;
                    }
                    Expr::Call(call) if self.record_string_origins || self.register_literals => {
                        self.visit_expr(&call.func);
                        for argument in &call.arguments.args {
                            self.visit_expr(argument);
                        }
                        let expanded = call.arguments.args.iter().any(Expr::is_starred_expr)
                            || call
                                .arguments
                                .keywords
                                .iter()
                                .any(|keyword| keyword.arg.is_none())
                            || (!matches!(call.func.as_ref(), Expr::Attribute(_))
                                && call.arguments.args.len() + call.arguments.keywords.len() * 2
                                    > 30);
                        if expanded {
                            for keyword in &call.arguments.keywords {
                                if let Some(name) = &keyword.arg {
                                    if self.register_literals {
                                        self.constants.push(Constant::String(name.to_string()));
                                    }
                                    self.string_origin_constants
                                        .push((Constant::String(name.to_string()), true));
                                }
                                self.visit_expr(&keyword.value);
                            }
                        } else {
                            for keyword in &call.arguments.keywords {
                                self.visit_expr(&keyword.value);
                            }
                            let names = Constant::Tuple(
                                call.arguments
                                    .keywords
                                    .iter()
                                    .map(|keyword| {
                                        Constant::String(keyword.arg.as_ref().unwrap().to_string())
                                    })
                                    .collect(),
                            );
                            if self.register_literals && !call.arguments.keywords.is_empty() {
                                self.constants.push(names.clone());
                            }
                            self.string_origin_constants.push((names, true));
                        }
                        return;
                    }
                    Expr::Name(name)
                        if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__" =>
                    {
                        // AST preprocessing rewrites this load before discarded code is
                        // visited, so it reserves a boolean constant rather than a name.
                        if self.register_literals {
                            self.constants.push(Constant::Bool(true));
                        }
                    }
                    Expr::Name(name) => {
                        if !self
                            .inlined_targets
                            .iter()
                            .any(|target| target == name.id.as_str())
                        {
                            self.names.push((name.id.as_str().to_string(), false));
                        }
                    }
                    Expr::Attribute(attribute) => {
                        if self.record_string_origins
                            && attribute.ctx == ExprContext::Store
                            && matches!(attribute.value.as_ref(), Expr::Name(name) if name.id.as_str() == "self")
                        {
                            self.actions
                                .push(SkippedAction::StaticAttribute(attribute.attr.as_str()));
                        }
                        self.visit_expr(&attribute.value);
                        self.names.push((attribute.attr.as_str().to_string(), true));
                        return;
                    }
                    Expr::Await(expression) if self.register_literals => {
                        self.visit_expr(&expression.value);
                        self.constants.push(Constant::None);
                        return;
                    }
                    Expr::YieldFrom(expression) if self.register_literals => {
                        // SEND starts with None, even when its block is later discarded.
                        self.visit_expr(&expression.value);
                        self.constants.push(Constant::None);
                        return;
                    }
                    Expr::Yield(expression)
                        if self.register_literals && expression.value.is_none() =>
                    {
                        self.constants.push(Constant::None);
                        return;
                    }
                    Expr::Lambda(lambda) => {
                        let build = self.build_discarded && self.record_string_origins;
                        if build {
                            self.flush_origins();
                            self.actions.push(SkippedAction::Expression(expression));
                            self.record_string_origins = false;
                        }
                        if let Some(parameters) = lambda.parameters.as_deref() {
                            for parameter in parameters.posonlyargs.iter().chain(&parameters.args) {
                                if let Some(default) = &parameter.default {
                                    self.visit_expr(default);
                                }
                            }
                            for parameter in &parameters.kwonlyargs {
                                if let Some(default) = &parameter.default {
                                    self.record_key_origin(parameter.name().as_str());
                                    self.visit_expr(default);
                                }
                            }
                        }
                        if self.record_string_origins {
                            let mut child = self.origin_collector(self.private_name.clone());
                            child.visit_expr(&lambda.body);
                            self.string_origin_constants
                                .extend(child.string_origin_constants);
                        }
                        if build {
                            self.record_string_origins = true;
                            self.string_origin_constants.clear();
                        }
                        return;
                    }
                    Expr::Generator(generator) => {
                        let build = self.build_discarded && self.record_string_origins;
                        if build {
                            self.flush_origins();
                            self.actions.push(SkippedAction::Expression(expression));
                            self.record_string_origins = false;
                        }
                        // Only the outer iterable runs in this scope; the generator body has
                        // its own name table, even when its enclosing branch is eliminated.
                        if let Some(first) = generator.generators.first() {
                            self.visit_iterable_expression(&first.iter);
                        }
                        if build {
                            self.record_string_origins = true;
                            self.string_origin_constants.clear();
                        }
                        return;
                    }
                    _ => {}
                }
                if let Some((generators, key, value)) = comprehension_parts(expression) {
                    let build = self.build_discarded && self.record_string_origins;
                    if build {
                        self.flush_origins();
                        self.actions.push(SkippedAction::Expression(expression));
                        self.record_string_origins = false;
                    }
                    if let Some(first) = generators.first() {
                        self.visit_iterable_expression(&first.iter);
                    }
                    // Hidden fast locals apply only inside this comprehension. The same
                    // spelling in a surrounding dead suite still belongs in its name table.
                    let outer_target_count = self.inlined_targets.len();
                    for generator in generators {
                        collect_target_names(&generator.target, &mut self.inlined_targets);
                    }
                    for (index, generator) in generators.iter().enumerate() {
                        if index > 0 {
                            self.visit_iterable_expression(&generator.iter);
                        }
                        self.visit_expr(&generator.target);
                        for condition in &generator.ifs {
                            self.visit_expr(condition);
                        }
                    }
                    if let Some(key) = key {
                        self.visit_expr(key);
                    }
                    self.visit_expr(value);
                    self.inlined_targets.truncate(outer_target_count);
                    if build {
                        self.record_string_origins = true;
                        self.string_origin_constants.clear();
                    }
                    return;
                }
                walk_expr(self, expression);
            }

            fn visit_pattern(&mut self, pattern: &'ast Pattern) {
                let capture = match pattern {
                    Pattern::MatchAs(pattern) => pattern.name.as_ref(),
                    Pattern::MatchStar(pattern) => pattern.name.as_ref(),
                    Pattern::MatchMapping(pattern) => pattern.rest.as_ref(),
                    _ => None,
                };
                if let Some(name) = capture {
                    self.names.push((name.as_str().to_string(), false));
                }
                walk_pattern(self, pattern);
            }
        }

        // Returns and loop exits can compile an enclosing finalizer before later
        // suites. Preserve the existing admission traversal until those copies are
        // replayed; raises do not inline finalizers during code generation.
        let mut unwind_exits = UnwindExitDetector::default();
        unwind_exits.visit_body(body);
        let mut collector = Collector {
            reorder_try_suites: !unwind_exits.0,
            register_literals,
            register_folds,
            build_discarded: self.context.requires_discarded_builders()
                || self.context.requires_empty_bytes_cache_trace,
            record_string_origins: self.context.has_tracked_string_origins()
                || self.context.requires_empty_bytes_cache_trace,
            private_name: self.unit.private_name.clone(),
            future_annotations: self.unit.flags & CO_FUTURE_ANNOTATIONS != 0,
            legacy_compiler: (!self.has_deferred_annotations()).then_some(&*self),
            legacy_annotated_scope: !self.has_deferred_annotations()
                && matches!(self.unit.scope, Scope::Module | Scope::Class { .. }),
            source: self.context.source,
            python_version: self.context.python_version,
            ..Collector::default()
        };
        if let Some(expression) = expression_only {
            collector.visit_expr(expression);
        } else if let Some(expression) = first_expression {
            // Compile this whole expression once: its first constant may come from a
            // lambda default, or from a generator child before its outer iterable.
            collector
                .actions
                .push(SkippedAction::FirstExpression(expression));
            let record_string_origins = collector.record_string_origins;
            collector.record_string_origins = false;
            collector.visit_stmt(&body[0]);
            collector.record_string_origins = record_string_origins;
            collector.visit_body(&body[1..]);
        } else {
            collector.visit_body(body);
        }
        collector.flush_origins();
        let Collector {
            actions,
            has_untracked_collection,
            constants,
            folded_expressions,
            names,
            ..
        } = collector;
        let mut builder = None;
        for action in actions {
            match action {
                SkippedAction::StaticAttribute(name) => {
                    self.context.record_static_attribute(
                        name,
                        matches!(self.unit.scope, Scope::Class { .. }),
                    );
                }
                SkippedAction::Origins(origins) => {
                    for (constant, interned) in origins {
                        self.context.record_constant_empty_bytes_origin(&constant);
                        self.context
                            .record_constant_string_origins(&constant, interned);
                    }
                }
                SkippedAction::Statement(statement) => {
                    let builder = builder.get_or_insert_with(|| self.discarded_builder());
                    match statement {
                        Stmt::FunctionDef(definition) => {
                            builder.compile_function_definition(definition)?;
                        }
                        Stmt::ClassDef(definition) => {
                            builder.compile_class_definition(definition)?;
                        }
                        Stmt::TypeAlias(alias) => builder.compile_type_alias(alias)?,
                        Stmt::AnnAssign(_) => {
                            builder.compile_suite(std::slice::from_ref(statement))?;
                        }
                        _ => unreachable!(),
                    }
                }
                SkippedAction::Expression(expression) => {
                    let builder = builder.get_or_insert_with(|| self.discarded_builder());
                    builder.compile_expression(expression)?;
                    builder.emit(super::POP_TOP, 0, -1)?;
                }
                SkippedAction::FirstExpression(expression) => {
                    let builder = builder.get_or_insert_with(|| self.discarded_builder());
                    builder.compile_expression(expression)?;
                    self.retain_discarded_first_constant(builder)?;
                    builder.emit(super::POP_TOP, 0, -1)?;
                }
            }
        }
        self.output.has_untracked_discarded_collection |= has_untracked_collection;
        for constant in constants {
            self.add_constant(constant)?;
        }
        // The first CFG reachability pass retains both edges of a constant condition.
        // Its discarded branch can therefore create a folded value before the later
        // reachability pass removes the branch; surviving uses retain that cache slot.
        for expression in folded_expressions {
            self.record_folded_value(expression)?;
        }
        for (name, force_name) in names {
            let name_id = self.required_name_id(&name);
            let is_local_or_free = self.symbols.active_temporaries.contains(name_id)
                || matches!(
                    &self.unit.scope,
                    Scope::Function {
                        indices,
                        free_indices,
                        globals,
                        ..
                    } if indices.contains_key(&name_id) && !globals.contains(name_id)
                        || free_indices.contains_key(&name_id)
                );
            if force_name || !is_local_or_free {
                self.name_index_with_id(&name, name_id)?;
            }
        }
        Ok(())
    }

    pub(super) fn pre_register_expression_names(
        &mut self,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        #[derive(Default)]
        struct Collector<'ast> {
            names: SmallVec<[(&'ast str, bool); 4]>,
        }

        impl<'ast> Visitor<'ast> for Collector<'ast> {
            fn visit_expr(&mut self, expression: &'ast Expr) {
                if crate::stack_growth_required() {
                    return crate::grow_stack(|| self.visit_expr(expression));
                }

                match expression {
                    Expr::Name(name)
                        if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__" => {}
                    Expr::Name(name) => self.names.push((name.id.as_str(), false)),
                    Expr::Attribute(attribute) => {
                        self.visit_expr(&attribute.value);
                        self.names.push((attribute.attr.as_str(), true));
                        return;
                    }
                    Expr::Lambda(lambda) => {
                        if let Some(parameters) = lambda.parameters.as_deref() {
                            for parameter in parameters
                                .posonlyargs
                                .iter()
                                .chain(&parameters.args)
                                .chain(&parameters.kwonlyargs)
                            {
                                if let Some(default) = &parameter.default {
                                    self.visit_expr(default);
                                }
                            }
                        }
                        return;
                    }
                    Expr::Generator(generator) => {
                        if let Some(first) = generator.generators.first() {
                            self.visit_expr(&first.iter);
                        }
                        return;
                    }
                    _ => {}
                }
                walk_expr(self, expression);
            }
        }

        let mut collector = Collector::default();
        collector.visit_expr(expression);
        for (name, force_name) in collector.names {
            let name_id = self.required_name_id(name);
            let is_local_or_free = self.symbols.active_temporaries.contains(name_id)
                || matches!(
                    &self.unit.scope,
                    Scope::Function {
                        indices,
                        free_indices,
                        globals,
                        ..
                    } if indices.contains_key(&name_id) && !globals.contains(name_id)
                        || free_indices.contains_key(&name_id)
                );
            if force_name || !is_local_or_free {
                self.name_index_with_id(name, name_id)?;
            }
        }
        Ok(())
    }

    pub(super) fn pre_register_expression_constants(
        &mut self,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        #[derive(Default)]
        struct Collector<'source> {
            constants: Vec<Constant>,
            legacy_fstring_source: Option<&'source str>,
        }

        impl Collector<'_> {
            fn visit_interpolated_components(
                &mut self,
                components: Vec<InterpolatedStringComponent<'_>>,
                source: &str,
            ) {
                if components.len() > 30 {
                    self.constants.push(Constant::String(String::new()));
                }
                for component in components {
                    match component {
                        InterpolatedStringComponent::Literal(literal) => {
                            self.constants.push(literal.into_constant());
                        }
                        InterpolatedStringComponent::Interpolation(interpolation, is_raw) => {
                            self.visit_expr(&interpolation.expression);
                            if let Some(specification) = &interpolation.format_spec {
                                self.visit_interpolated_components(
                                    format_spec_components(&specification.elements, is_raw, source),
                                    source,
                                );
                            }
                        }
                    }
                }
            }
        }

        impl<'ast> Visitor<'ast> for Collector<'_> {
            fn visit_expr(&mut self, expression: &'ast Expr) {
                if crate::stack_growth_required() {
                    return crate::grow_stack(|| self.visit_expr(expression));
                }

                if let Expr::FString(fstring) = expression
                    && let Some(source) = self.legacy_fstring_source
                {
                    // Ruff represents literal fragments outside Expr; the generic
                    // visitor would miss constants reused after this message is removed.
                    self.visit_interpolated_components(fstring_components(fstring, source), source);
                } else if let Some(constant) = literal_constant(expression) {
                    self.constants.push(constant);
                } else {
                    walk_expr(self, expression);
                }
            }
        }

        let mut collector = Collector {
            legacy_fstring_source: (self.context.python_version <= PythonVersion::Py313)
                .then_some(self.context.source),
            ..Collector::default()
        };
        collector.visit_expr(expression);
        for constant in collector.constants {
            self.add_constant(constant)?;
        }
        Ok(())
    }

    pub(super) fn emit(
        &mut self,
        opcode: Opcode,
        argument: u32,
        effect: i32,
    ) -> Result<(), CompileError> {
        self.emit_with_exception_policy(opcode, argument, effect, ExceptionPolicy::default())
    }

    /// Emits an instruction whose logical stack effect is fixed by target metadata.
    pub(super) fn emit_instruction(
        &mut self,
        opcode: Opcode,
        argument: u32,
    ) -> Result<EmittedInstruction, CompileError> {
        let opcode = self.emission_opcode(opcode);
        let effect =
            required_compiler_stack_effect(self.assembler.python_version(), opcode, argument)?;
        self.apply_stack_effect(effect)?;
        Ok(self.assembler.emit_with_depth(
            opcode,
            argument,
            self.control.stack.depth.cast_unsigned(),
            ExceptionPolicy::default(),
        ))
    }

    /// Emits an async-iteration branch marker before exception ownership is assigned.
    pub(super) fn emit_authored_not_taken(&mut self) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            return Ok(());
        }
        let emitted = self.emit_instruction(crate::target::opcodes::NOT_TAKEN, 0)?;
        if self.assembler.python_version() == crate::PythonVersion::Py315 {
            self.assembler.mark_authored_branch_marker(emitted);
        }
        Ok(())
    }

    pub(super) fn emit_yield_value(
        &mut self,
        argument: u32,
        effect: i32,
    ) -> Result<(), CompileError> {
        self.control.generators.emitted_yield_value = true;
        self.emit(YIELD_VALUE, argument, effect)
    }

    pub(super) fn emit_with_exception_policy(
        &mut self,
        opcode: Opcode,
        argument: u32,
        effect: i32,
        exception_policy: ExceptionPolicy,
    ) -> Result<(), CompileError> {
        if self.omits_opcode(opcode) {
            debug_assert_eq!(effect, 0);
            return Ok(());
        }
        let opcode = self.emission_opcode(opcode);
        self.apply_compiler_stack_effect(opcode, argument, effect)?;
        self.assembler.emit_value_with_depth_discard(
            opcode,
            argument,
            self.control.stack.depth.cast_unsigned(),
            exception_policy,
        );
        Ok(())
    }

    /// Identifies zero-effect markers absent from an older bytecode target.
    fn omits_opcode(&self, opcode: Opcode) -> bool {
        (self.context.python_version <= PythonVersion::Py313
            && opcode == crate::target::opcodes::NOT_TAKEN)
            || (self.context.python_version == PythonVersion::Py312
                && opcode == crate::target::opcodes::TO_BOOL)
    }

    /// Resolves equivalent instructions before validating their target stack effects.
    fn emission_opcode(&self, opcode: Opcode) -> Opcode {
        if self.omits_opcode(opcode) {
            // Emission sites retaining an instruction handle need a removable slot.
            crate::target::opcodes::NOP
        } else if self.context.python_version <= PythonVersion::Py313
            && opcode == crate::target::opcodes::POP_ITER
        {
            crate::target::opcodes::POP_TOP
        } else {
            opcode
        }
    }

    /// Emits the target's cleanup for the iterator-exhausted edge of a loop.
    pub(super) fn emit_end_for_cleanup(&mut self) -> Result<(), CompileError> {
        if self.context.python_version == PythonVersion::Py312 {
            self.emit(crate::target::opcodes::END_FOR, 0, -2)
        } else {
            self.emit(crate::target::opcodes::END_FOR, 0, -1)?;
            self.emit(crate::target::opcodes::POP_ITER, 0, -self.iterator_slots())
        }
    }

    pub(super) fn emit_reraise(&mut self, state: ReraiseStackState) -> Result<(), CompileError> {
        let (argument, mut effect) = state.argument_and_effect();
        if self.context.python_version <= PythonVersion::Py313
            && matches!(state, ReraiseStackState::WithHandler)
        {
            // Older with statements retain only the exit callable on the stack.
            effect += 1;
        }
        self.apply_stack_effect(effect)?;
        self.assembler.emit_value_with_depth_discard(
            RERAISE,
            argument,
            self.control.stack.depth.cast_unsigned(),
            ExceptionPolicy::default(),
        );
        Ok(())
    }

    pub(super) fn emit_unary_intrinsic(
        &mut self,
        intrinsic: UnaryIntrinsic,
    ) -> Result<(), CompileError> {
        self.emit(CALL_INTRINSIC_1, intrinsic.argument(), 0)
    }

    pub(super) fn emit_binary_intrinsic(
        &mut self,
        intrinsic: BinaryIntrinsic,
    ) -> Result<(), CompileError> {
        self.emit(CALL_INTRINSIC_2, intrinsic.argument(), -1)
    }

    pub(super) fn emit_function_attribute(
        &mut self,
        attribute: FunctionAttribute,
    ) -> Result<(), CompileError> {
        self.emit(SET_FUNCTION_ATTRIBUTE, attribute.argument(), -1)
    }

    pub(super) fn emit_common_constant(
        &mut self,
        constant: CommonConstant,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313
            && constant == CommonConstant::ASSERTIONERROR
        {
            return self.emit(crate::target::opcodes::LOAD_ASSERTION_ERROR, 0, 1);
        }
        self.emit(LOAD_COMMON_CONSTANT, constant.argument(), 1)
    }

    pub(super) fn emit_resume(
        &mut self,
        location: ResumeLocation,
        depth: ResumeDepth,
    ) -> Result<(), CompileError> {
        let argument = if self.context.python_version == PythonVersion::Py312 {
            location.argument()
        } else {
            location.at_depth(depth.is_one())
        };
        self.emit(RESUME, argument, 0)?;
        if location == ResumeLocation::AFTER_AWAIT
            && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR) != 0
        {
            self.control.generators.coroutine_suspension_epoch = self
                .control
                .generators
                .coroutine_suspension_epoch
                .saturating_add(1);
        }
        Ok(())
    }

    pub(super) fn emit_binary_operation(
        &mut self,
        operation: BinaryOperation,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313
            && operation == BinaryOperation::SUBSCR
        {
            return self.emit(crate::target::opcodes::BINARY_SUBSCR, 0, -1);
        }
        self.emit(BINARY_OP, operation.argument(), -1)
    }

    pub(super) fn emit_comparison(
        &mut self,
        operation: ComparisonOperation,
        mode: ComparisonMode,
    ) -> Result<(), CompileError> {
        let argument = if self.context.python_version == PythonVersion::Py312 {
            let argument = operation.argument();
            ((argument >> 5) << 4) | (argument & 15)
        } else {
            match mode {
                ComparisonMode::Value => operation.argument(),
                ComparisonMode::Boolean => operation.boolean_argument(),
            }
        };
        self.emit(COMPARE_OP, argument, -1)
    }

    pub(super) fn emit_conversion(&mut self, conversion: Conversion) -> Result<(), CompileError> {
        self.emit(CONVERT_VALUE, conversion.argument(), 0)
    }

    pub(super) fn emit_special_method(
        &mut self,
        method: SpecialMethod,
    ) -> Result<(), CompileError> {
        self.emit(LOAD_SPECIAL, method.argument(), 1)
    }

    pub(super) fn emit_owned_fast(
        &mut self,
        argument: u32,
        effect: i32,
    ) -> Result<(), CompileError> {
        self.apply_compiler_stack_effect(LOAD_FAST, argument, effect)?;
        self.assembler
            .emit_owned_fast_with_depth(argument, self.control.stack.depth.cast_unsigned());
        Ok(())
    }

    pub(super) fn emit_owned_fast_instruction(
        &mut self,
        argument: u32,
    ) -> Result<EmittedInstruction, CompileError> {
        let effect =
            required_compiler_stack_effect(self.assembler.python_version(), LOAD_FAST, argument)?;
        self.apply_stack_effect(effect)?;
        Ok(self
            .assembler
            .emit_owned_fast_with_depth(argument, self.control.stack.depth.cast_unsigned()))
    }

    pub(super) fn emit_jump_forward(
        &mut self,
        opcode: Opcode,
        label: Label,
        effect: i32,
    ) -> Result<(), CompileError> {
        self.emit_jump_forward_with_exception_policy(
            opcode,
            label,
            effect,
            ExceptionPolicy::default(),
        )
    }

    /// Emits a forward jump whose logical stack effect is fixed by target metadata.
    pub(super) fn emit_jump_forward_instruction(
        &mut self,
        opcode: Opcode,
        label: Label,
    ) -> Result<EmittedJump, CompileError> {
        self.emit_jump_forward_instruction_with_exception_policy(
            opcode,
            label,
            ExceptionPolicy::default(),
        )
    }

    /// Emits a folded condition's jump without transferring a preceding NOP's location.
    /// The removed NOP still contributes its original slots and exclusion bookkeeping.
    pub(super) fn emit_jump_forward_instruction_preserving_location(
        &mut self,
        opcode: Opcode,
        label: Label,
    ) -> Result<EmittedJump, CompileError> {
        self.emit_jump_forward_instruction_inner(
            opcode,
            label,
            ExceptionPolicy::default(),
            true,
            true,
        )
    }

    pub(super) fn emit_jump_forward_instruction_with_exception_policy(
        &mut self,
        opcode: Opcode,
        label: Label,
        exception_policy: ExceptionPolicy,
    ) -> Result<EmittedJump, CompileError> {
        self.emit_jump_forward_instruction_inner(opcode, label, exception_policy, false, true)
    }

    /// Keeps the preceding original record for location propagation and NOP removal.
    pub(super) fn emit_jump_forward_instruction_preserving_nop(
        &mut self,
        opcode: Opcode,
        label: Label,
        exception_policy: ExceptionPolicy,
    ) -> Result<EmittedJump, CompileError> {
        self.emit_jump_forward_instruction_inner(opcode, label, exception_policy, false, false)
    }

    /// Shares validated jump emission and optional NOP compaction between location policies.
    fn emit_jump_forward_instruction_inner(
        &mut self,
        opcode: Opcode,
        label: Label,
        exception_policy: ExceptionPolicy,
        preserve_location: bool,
        compact_trailing_nop: bool,
    ) -> Result<EmittedJump, CompileError> {
        let original_slots = self.prepare_jump(opcode, preserve_location, compact_trailing_nop);
        let effect = required_compiler_stack_effect(self.assembler.python_version(), opcode, 0)?;
        self.apply_stack_effect(effect)?;
        let jump = self.assembler.emit_forward_with_depth(
            opcode,
            label,
            self.control.stack.depth.cast_unsigned(),
            exception_policy,
        );
        self.assembler.add_last_original_block_slots(original_slots);
        Ok(jump)
    }

    pub(super) fn emit_jump_forward_with_exception_policy(
        &mut self,
        opcode: Opcode,
        label: Label,
        effect: i32,
        exception_policy: ExceptionPolicy,
    ) -> Result<(), CompileError> {
        let original_slots = self.prepare_jump(opcode, false, true);
        self.apply_compiler_stack_effect(opcode, 0, effect)?;
        self.assembler.emit_forward_with_depth(
            opcode,
            label,
            self.control.stack.depth.cast_unsigned(),
            exception_policy,
        );
        self.assembler.add_last_original_block_slots(original_slots);
        Ok(())
    }

    pub(super) fn emit_jump_backward(
        &mut self,
        opcode: Opcode,
        label: Label,
        effect: i32,
    ) -> Result<(), CompileError> {
        self.emit_jump_backward_with_exception_policy(
            opcode,
            label,
            effect,
            ExceptionPolicy::default(),
        )
    }

    /// Emits a backward jump whose logical stack effect is fixed by target metadata.
    pub(super) fn emit_jump_backward_instruction(
        &mut self,
        opcode: Opcode,
        label: Label,
    ) -> Result<EmittedJump, CompileError> {
        let original_slots = self.prepare_jump(opcode, false, true);
        let effect = required_compiler_stack_effect(self.assembler.python_version(), opcode, 0)?;
        self.apply_stack_effect(effect)?;
        let jump = self
            .assembler
            .emit_backward_with_depth_and_exception_policy(
                opcode,
                label,
                self.control.stack.depth.cast_unsigned(),
                ExceptionPolicy::default(),
            );
        self.assembler.add_last_original_block_slots(original_slots);
        Ok(jump)
    }

    pub(super) fn emit_jump_backward_with_exception_policy(
        &mut self,
        opcode: Opcode,
        label: Label,
        effect: i32,
        exception_policy: ExceptionPolicy,
    ) -> Result<(), CompileError> {
        let original_slots = self.prepare_jump(opcode, false, true);
        self.apply_compiler_stack_effect(opcode, 0, effect)?;
        self.assembler
            .emit_backward_with_depth_and_exception_policy(
                opcode,
                label,
                self.control.stack.depth.cast_unsigned(),
                exception_policy,
            );
        self.assembler.add_last_original_block_slots(original_slots);
        Ok(())
    }

    /// Clears pending comprehension results and optionally removes a preceding NOP
    /// for unconditional jumps on 3.14+, updating exclusions.
    /// Returns the original slots that must be attached after emitting the jump;
    /// folded conditions can retain their jump location instead of inheriting the NOP's.
    fn prepare_jump(
        &mut self,
        opcode: Opcode,
        preserve_location: bool,
        compact_trailing_nop: bool,
    ) -> usize {
        self.control.clear_direct_comprehension_assignment_results();
        let removed_nop = (compact_trailing_nop
            && self.context.python_version >= crate::PythonVersion::Py314
            && is_unconditional_jump(opcode))
        .then(|| self.take_trailing_nop_location())
        .flatten();
        if !preserve_location && let Some((location, _)) = removed_nop {
            self.assembler.set_location(location);
        }
        removed_nop.map_or(0, |(_, slots)| slots)
    }

    pub(super) fn take_trailing_nop_location(&mut self) -> Option<(SourceLocation, usize)> {
        let removed = self.assembler.take_trailing_nop_location()?;
        if let Some(exclusion) = removed.exclusion {
            self.control
                .generators
                .generator_region_exclusions
                .retain(|candidate| *candidate != exclusion);
            for exclusions in &mut self.control.withs.active_with_region_exclusions {
                exclusions.retain(|candidate| *candidate != exclusion);
            }
            for exclusions in &mut self.control.exceptions.active_exception_region_exclusions {
                exclusions.retain(|candidate| *candidate != exclusion);
            }
            for exclusions in &mut self
                .control
                .comprehensions
                .active_comprehension_region_exclusions
            {
                exclusions.retain(|candidate| *candidate != exclusion);
            }
        }
        Some((removed.location, removed.original_slots))
    }

    pub(super) fn apply_compiler_stack_effect(
        &mut self,
        opcode: Opcode,
        argument: u32,
        declared: i32,
    ) -> Result<(), CompileError> {
        // Lowering supplies a statically known effect at every call site. Keep the generated
        // metadata cross-check on every test/debug emission without paying for it in production.
        #[cfg(any(test, debug_assertions))]
        let effect = validate_compiler_stack_effect_for(
            self.assembler.python_version(),
            opcode,
            argument,
            declared,
        )?;
        #[cfg(not(any(test, debug_assertions)))]
        let effect = {
            let _ = (opcode, argument);
            declared
        };
        self.apply_stack_effect(effect)
    }

    fn apply_stack_effect(&mut self, effect: i32) -> Result<(), CompileError> {
        self.control.stack.depth += effect;
        if self.control.stack.depth < 0 {
            return Err(CompileError::Internal(
                "compiler produced a negative stack depth".to_string(),
            ));
        }
        self.control.stack.max_depth = self
            .control
            .stack
            .max_depth
            .max(self.control.stack.depth.cast_unsigned());
        Ok(())
    }

    pub(super) fn set_depth(&mut self, depth: i32) {
        debug_assert!(depth >= 0);
        self.control.stack.depth = depth;
        self.control.stack.max_depth = self.control.stack.max_depth.max(depth.cast_unsigned());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompilerStackEffect {
    Fixed(i32),
    ContextDependent,
}

/// Returns the effect on the compiler's logical value stack when it is fixed.
#[cfg(test)]
pub(super) fn compiler_stack_effect(opcode: Opcode, argument: u32) -> CompilerStackEffect {
    compiler_stack_effect_for(crate::PythonVersion::Py314, opcode, argument)
}

fn compiler_stack_effect_for(
    python_version: crate::PythonVersion,
    opcode: Opcode,
    argument: u32,
) -> CompilerStackEffect {
    match opcode {
        // CPython's physical interpreter stack retains the return value for frame teardown, but
        // the compiler has finished consuming its logical value stack.
        RETURN_VALUE => CompilerStackEffect::Fixed(-1),
        // RERAISE can discard additional logical exception-state values introduced by the
        // surrounding lowering path. Its effect is supplied by that path explicitly.
        RERAISE => CompilerStackEffect::ContextDependent,
        _ => {
            let (popped, pushed) = opcode_stack_counts(python_version, opcode, argument);
            CompilerStackEffect::Fixed(
                i32::try_from(pushed).expect("opcode push count fits in i32")
                    - i32::try_from(popped).expect("opcode pop count fits in i32"),
            )
        }
    }
}

fn required_compiler_stack_effect(
    python_version: crate::PythonVersion,
    opcode: Opcode,
    argument: u32,
) -> Result<i32, CompileError> {
    match compiler_stack_effect_for(python_version, opcode, argument) {
        CompilerStackEffect::Fixed(effect) => Ok(effect),
        CompilerStackEffect::ContextDependent => Err(CompileError::Internal(format!(
            "opcode {} with argument {argument} has a context-dependent stack effect; use its typed emitter",
            opcode.code(),
        ))),
    }
}

#[cfg(test)]
pub(super) fn validate_compiler_stack_effect(
    opcode: Opcode,
    argument: u32,
    declared: i32,
) -> Result<i32, CompileError> {
    validate_compiler_stack_effect_for(crate::PythonVersion::Py314, opcode, argument, declared)
}

#[cfg(any(test, debug_assertions))]
fn validate_compiler_stack_effect_for(
    python_version: crate::PythonVersion,
    opcode: Opcode,
    argument: u32,
    declared: i32,
) -> Result<i32, CompileError> {
    match compiler_stack_effect_for(python_version, opcode, argument) {
        CompilerStackEffect::Fixed(expected) if declared != expected => {
            Err(CompileError::Internal(format!(
                "compiler declared stack effect {declared} for opcode {} with argument {argument}, expected {expected}",
                opcode.code()
            )))
        }
        CompilerStackEffect::Fixed(expected) => Ok(expected),
        CompilerStackEffect::ContextDependent => Err(CompileError::Internal(format!(
            "compiler declared stack effect {declared} for context-dependent opcode {} with argument {argument}; use its typed emitter",
            opcode.code(),
        ))),
    }
}

#[inline]
pub(super) fn mangle_name<'a>(name: &'a str, private_name: Option<&str>) -> Cow<'a, str> {
    let Some(private_name) = private_name else {
        return Cow::Borrowed(name);
    };
    if !name.starts_with("__") {
        return Cow::Borrowed(name);
    }
    mangle_private_name(name, private_name)
}

fn mangle_private_name<'a>(name: &'a str, private_name: &str) -> Cow<'a, str> {
    if name.ends_with("__") || name.contains('.') {
        return Cow::Borrowed(name);
    }
    let class_name = private_name.trim_start_matches('_');
    if class_name.is_empty() {
        return Cow::Borrowed(name);
    }
    Cow::Owned(format!("_{class_name}{name}"))
}
pub(super) fn to_u32(value: usize, what: &str) -> Result<u32, CompileError> {
    value
        .try_into()
        .map_err(|_| CompileError::Unsupported(format!("{what} exceeds CPython's u32 limit")))
}

pub(super) fn unsupported(feature: &str) -> CompileError {
    CompileError::Unsupported(format!(
        "Python {} {feature} is not implemented yet",
        crate::CPYTHON_TARGET.version_string()
    ))
}

pub(super) fn statement_name(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::AnnAssign(_) => "annotated assignment",
        Stmt::Assert(_) => "assert statement",
        Stmt::ClassDef(_) => "class definition",
        Stmt::Delete(_) => "del statement",
        Stmt::For(_) => "for statement",
        Stmt::Global(_) => "global statement",
        Stmt::Import(_) => "import statement",
        Stmt::ImportFrom(_) => "from import statement",
        Stmt::IpyEscapeCommand(_) => "IPython escape command",
        Stmt::Match(_) => "match statement",
        Stmt::Nonlocal(_) => "nonlocal statement",
        Stmt::Raise(_) => "raise statement",
        Stmt::Try(_) => "try statement",
        Stmt::TypeAlias(_) => "type alias statement",
        Stmt::With(_) => "with statement",
        _ => "statement",
    }
}

pub(super) fn expression_name(expression: &Expr) -> &'static str {
    match expression {
        Expr::Await(_) => "await expression",
        Expr::DictComp(_) => "dictionary comprehension",
        Expr::FString(_) => "f-string",
        Expr::Generator(_) => "generator expression",
        Expr::IpyEscapeCommand(_) => "IPython escape command",
        Expr::Lambda(_) => "lambda expression",
        Expr::ListComp(_) => "list comprehension",
        Expr::Named(_) => "named expression",
        Expr::SetComp(_) => "set comprehension",
        Expr::Slice(_) => "slice expression",
        Expr::Starred(_) => "starred expression",
        Expr::TString(_) => "template string",
        Expr::Yield(_) => "yield expression",
        Expr::YieldFrom(_) => "yield from expression",
        Expr::Attribute(_) => "attribute store or delete",
        Expr::List(_) => "list store or delete",
        Expr::Name(_) => "name store or delete",
        Expr::Subscript(_) => "subscript store or delete",
        Expr::Tuple(_) => "tuple store or delete",
        _ => "expression",
    }
}

/// Conservatively detects collections whose folded origins require CFG construction.
pub(super) fn has_untracked_discarded_collection(body: &[Stmt]) -> bool {
    struct RuntimeCollection(bool);
    impl<'ast> Visitor<'ast> for RuntimeCollection {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }

            let elements = match expression {
                Expr::Tuple(tuple) => Some(tuple.elts.as_slice()),
                Expr::List(list) => Some(list.elts.as_slice()),
                Expr::Set(set) => Some(set.elts.as_slice()),
                _ => None,
            };
            self.0 |= elements.is_some_and(|elements| {
                elements
                    .iter()
                    .any(|element| fold_constant(element).is_none())
            });
            walk_expr(self, expression);
        }
    }
    let mut visitor = RuntimeCollection(false);
    for statement in body {
        visitor.visit_stmt(statement);
    }
    visitor.0
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::mangle_name;

    #[test]
    fn mangle_name_preserves_spelling_and_ownership() {
        for (name, private_name, expected, owned) in [
            ("", None, "", false),
            ("", Some("C"), "", false),
            ("name", None, "name", false),
            ("name", Some("C"), "name", false),
            ("_", Some("C"), "_", false),
            ("__", Some("C"), "__", false),
            ("__name", None, "__name", false),
            ("__name", Some("C"), "_C__name", true),
            ("__name", Some("_C"), "_C__name", true),
            ("__name", Some("___C"), "_C__name", true),
            ("__name", Some(""), "__name", false),
            ("__name", Some("___"), "__name", false),
            ("__name__", Some("C"), "__name__", false),
            ("__name_", Some("C"), "_C__name_", true),
            ("__name.part", Some("C"), "__name.part", false),
            ("__\u{e9}", Some("_\u{e9}"), "_\u{e9}__\u{e9}", true),
            (
                "\u{ff3f}\u{ff3f}name",
                Some("C"),
                "\u{ff3f}\u{ff3f}name",
                false,
            ),
            ("__name", Some("\u{ff3f}C"), "_\u{ff3f}C__name", true),
        ] {
            let actual = mangle_name(name, private_name);
            assert_eq!(actual.as_ref(), expected, "{name:?}, {private_name:?}");
            assert_eq!(
                matches!(actual, Cow::Owned(_)),
                owned,
                "{name:?}, {private_name:?}"
            );
        }
    }
}
