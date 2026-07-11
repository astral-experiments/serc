use std::borrow::Cow;
use std::sync::Arc;

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use ruff_python_ast::{Expr, Pattern, Stmt};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    BINARY_OP, BUILD_TUPLE, CALL_INTRINSIC_1, CALL_INTRINSIC_2, COMPARE_OP, CONVERT_VALUE,
    DELETE_DEREF, DELETE_FAST, DELETE_GLOBAL, DELETE_NAME, LOAD_COMMON_CONSTANT, LOAD_DEREF,
    LOAD_FAST, LOAD_FAST_CHECK, LOAD_FROM_DICT_OR_DEREF, LOAD_FROM_DICT_OR_GLOBALS, LOAD_GLOBAL,
    LOAD_LOCALS, LOAD_NAME, LOAD_SPECIAL, RERAISE, RESUME, RETURN_VALUE, SET_FUNCTION_ATTRIBUTE,
    STORE_DEREF, STORE_FAST, STORE_GLOBAL, STORE_NAME,
};
use crate::CompileError;
use crate::assembler::{ExceptionPolicy, Label, SourceLocation};
use crate::code_object::Constant;
use crate::target::code_flags::{CO_ASYNC_GENERATOR, CO_COROUTINE};
use crate::target::operands::{
    BinaryIntrinsic, BinaryOperation, CommonConstant, ComparisonOperation, Conversion,
    FunctionAttribute, ResumeLocation, SpecialMethod, UnaryIntrinsic,
};
use crate::target::{
    Opcode, is_unconditional_jump, num_popped as opcode_num_popped, num_pushed as opcode_num_pushed,
};

use super::constant::literal_constant;
use super::scope::{NameId, NameSet, definitely_evaluated_references};
use super::{Compiler, Scope};

#[derive(Debug, Default)]
pub(super) struct SymbolState {
    pub(super) locals: Vec<NameId>,
    pub(super) fast_local_count: usize,
    pub(super) cell_names: NameSet,
    pub(super) free_names: Vec<NameId>,
    pub(super) temporary_indices: FxHashMap<NameId, u32>,
    pub(super) hidden_names: NameSet,
    pub(super) active_temporaries: NameSet,
    pub(super) active_temporary_fast_cells: NameSet,
    pub(super) initialized_locals: NameSet,
    pub(super) owned_load_locals: NameSet,
    pub(super) imported_scope_names: NameSet,
    pub(super) type_parameter_names: NameSet,
    pub(super) module_globals: NameSet,
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
    /// An unsuccessful synchronous or asynchronous `with` handler retains five logical values.
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

impl Compiler {
    fn name_id(&self, name: &str) -> Option<NameId> {
        self.context.scopes.names.get(name)
    }

    pub(super) fn required_name_id(&self, name: &str) -> NameId {
        self.name_id(name)
            .unwrap_or_else(|| panic!("module analysis did not intern `{name}`"))
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
        self.name_id(name).is_some_and(|name| names.contains(name))
    }

    pub(super) fn resolve_name(&self, name: NameId) -> &str {
        self.context.scopes.names.resolve(name)
    }

    pub(super) fn scope_index(&self, indices: &FxHashMap<NameId, u32>, name: &str) -> Option<u32> {
        self.name_id(name)
            .and_then(|name| indices.get(&name).copied())
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
        self.name_id(name)
            .and_then(|name| self.symbols.temporary_indices.get(&name).copied())
    }

    pub(super) fn load_name(&mut self, name: &str) -> Result<(), CompileError> {
        let name_id = self.required_name_id(name);
        if self.symbols.active_temporaries.contains(name_id) {
            let index = self
                .symbols
                .temporary_indices
                .get(&name_id)
                .copied()
                .expect("active temporary should have an index");
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
        let annotation_classdict_index = self.unit.annotation_classdict_index;
        match &self.unit.scope {
            Scope::Module => {
                let index = self.name_index_with_id(name, name_id)?;
                if self.symbols.module_globals.contains(name_id) {
                    self.emit(LOAD_GLOBAL, index << 1, 1)
                } else {
                    self.emit(LOAD_NAME, index, 1)
                }
            }
            Scope::Class {
                globals,
                free_indices,
                ..
            } => {
                if globals.contains(name_id) {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(LOAD_GLOBAL, index << 1, 1)
                } else if let Some(index) = free_indices.get(&name_id).copied() {
                    if self.control.active_comprehension_cleanups.is_empty() {
                        self.emit(LOAD_LOCALS, 0, 1)?;
                        self.emit(LOAD_FROM_DICT_OR_DEREF, index, 0)
                    } else {
                        self.emit(LOAD_DEREF, index, 1)
                    }
                } else {
                    let index = self.name_index_with_id(name, name_id)?;
                    if self.control.active_comprehension_cleanups.is_empty() {
                        self.emit(LOAD_NAME, index, 1)
                    } else {
                        // The first iterable runs in the class namespace, but the inlined
                        // comprehension body retains the globals-only lookup semantics of its
                        // former implicit function scope.
                        self.emit(LOAD_GLOBAL, index << 1, 1)
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
                    {
                        self.emit(LOAD_DEREF, classdict, 1)?;
                        let name = self.annotation_mangled_name(name);
                        let index = self.name_index(&name)?;
                        return self.emit(LOAD_FROM_DICT_OR_GLOBALS, index, 0);
                    }
                    let index = self.name_index_with_id(name, name_id)?;
                    return self.emit(LOAD_GLOBAL, index << 1, 1);
                }
                if indices.contains_key(&name_id)
                    && let Some(index) = free_indices.get(&name_id).copied()
                {
                    // An inlined comprehension can use a temporary fast-local slot with the
                    // same spelling as a free variable. Outside the comprehension, resolve the
                    // name through the closure.
                    if let Some(classdict) = annotation_classdict_index
                        && name != "__classdict__"
                    {
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
                        if self.symbols.owned_load_locals.contains(name_id) {
                            self.emit_owned_fast(index, 1)?;
                        } else {
                            self.emit(LOAD_FAST, index, 1)?;
                        }
                        if self
                            .control
                            .take_direct_comprehension_assignment_result(name_id)
                        {
                            self.assembler.mark_last_load_fast_as_known_initialized();
                        }
                        Ok(())
                    } else {
                        self.emit(LOAD_FAST_CHECK, index, 1)
                    }
                } else if let Some(index) = free_indices.get(&name_id).copied() {
                    if let Some(classdict) = annotation_classdict_index
                        && name != "__classdict__"
                    {
                        self.emit(LOAD_DEREF, classdict, 1)?;
                        self.emit(LOAD_FROM_DICT_OR_DEREF, index, 0)
                    } else {
                        self.emit(LOAD_DEREF, index, 1)
                    }
                } else {
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
        let Some(references) = definitely_evaluated_references(expression) else {
            return;
        };
        let Scope::Function { indices, .. } = &self.unit.scope else {
            return;
        };
        let references = references
            .into_iter()
            .filter_map(|name| {
                self.name_id(&name)
                    .filter(|name| indices.contains_key(name))
            })
            .collect::<Vec<_>>();
        for name in references {
            self.symbols.initialized_locals.insert(name);
        }
    }

    pub(super) fn store_name(&mut self, name: &str) -> Result<(), CompileError> {
        let name_id = self.required_name_id(name);
        self.store_name_with_id(name, name_id)
    }

    pub(super) fn store_name_id(&mut self, name_id: NameId) -> Result<(), CompileError> {
        let context = Arc::clone(&self.context);
        let name = context.scopes.names.resolve(name_id);
        self.store_name_with_id(name, name_id)
    }

    fn store_name_with_id(&mut self, name: &str, name_id: NameId) -> Result<(), CompileError> {
        if self.symbols.active_temporaries.contains(name_id) {
            let index = self
                .symbols
                .temporary_indices
                .get(&name_id)
                .copied()
                .expect("active temporary should have an index");
            return self.emit(
                if self.symbols.cell_names.contains(name_id)
                    && !self.symbols.active_temporary_fast_cells.contains(name_id)
                {
                    STORE_DEREF
                } else {
                    STORE_FAST
                },
                index,
                -1,
            );
        }
        match &self.unit.scope {
            Scope::Module => {
                let index = self.name_index_with_id(name, name_id)?;
                if self.symbols.module_globals.contains(name_id) {
                    self.emit(STORE_GLOBAL, index, -1)
                } else {
                    self.emit(STORE_NAME, index, -1)
                }
            }
            Scope::Class {
                globals,
                nonlocals,
                free_indices,
                ..
            } => {
                if globals.contains(name_id) {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(STORE_GLOBAL, index, -1)
                } else if nonlocals.contains(name_id) {
                    let index = free_indices.get(&name_id).copied().ok_or_else(|| {
                        CompileError::Internal(format!("missing class nonlocal variable `{name}`"))
                    })?;
                    self.emit(STORE_DEREF, index, -1)
                } else {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(STORE_NAME, index, -1)
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
                    return self.emit(STORE_GLOBAL, index, -1);
                }
                if let Some(index) = indices.get(&name_id).copied() {
                    self.symbols.initialized_locals.insert(name_id);
                    if cells.contains(name_id) {
                        self.emit(STORE_DEREF, index, -1)
                    } else {
                        self.emit(STORE_FAST, index, -1)
                    }
                } else if let Some(index) = free_indices.get(&name_id).copied() {
                    self.emit(STORE_DEREF, index, -1)
                } else {
                    Err(CompileError::Internal(format!(
                        "missing local variable `{name}`"
                    )))
                }
            }
        }
    }

    pub(super) fn delete_name(&mut self, name: &str) -> Result<(), CompileError> {
        let name_id = self.required_name_id(name);
        self.delete_name_with_id(name, name_id)
    }

    pub(super) fn delete_name_id(&mut self, name_id: NameId) -> Result<(), CompileError> {
        let context = Arc::clone(&self.context);
        let name = context.scopes.names.resolve(name_id);
        self.delete_name_with_id(name, name_id)
    }

    fn delete_name_with_id(&mut self, name: &str, name_id: NameId) -> Result<(), CompileError> {
        if self.symbols.active_temporaries.contains(name_id) {
            let index = self
                .symbols
                .temporary_indices
                .get(&name_id)
                .copied()
                .expect("active temporary should have an index");
            return self.emit(
                if self.symbols.cell_names.contains(name_id)
                    && !self.symbols.active_temporary_fast_cells.contains(name_id)
                {
                    DELETE_DEREF
                } else {
                    DELETE_FAST
                },
                index,
                0,
            );
        }
        match &self.unit.scope {
            Scope::Module => {
                let index = self.name_index_with_id(name, name_id)?;
                if self.symbols.module_globals.contains(name_id) {
                    self.emit(DELETE_GLOBAL, index, 0)
                } else {
                    self.emit(DELETE_NAME, index, 0)
                }
            }
            Scope::Class {
                globals,
                nonlocals,
                free_indices,
                ..
            } => {
                if globals.contains(name_id) {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(DELETE_GLOBAL, index, 0)
                } else if nonlocals.contains(name_id) {
                    let index = free_indices.get(&name_id).copied().ok_or_else(|| {
                        CompileError::Internal(format!("missing class nonlocal variable `{name}`"))
                    })?;
                    self.emit(DELETE_DEREF, index, 0)
                } else {
                    let index = self.name_index_with_id(name, name_id)?;
                    self.emit(DELETE_NAME, index, 0)
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
                    return self.emit(DELETE_GLOBAL, index, 0);
                }
                if let Some(index) = indices.get(&name_id).copied() {
                    self.symbols.initialized_locals.remove(name_id);
                    if cells.contains(name_id) {
                        self.emit(DELETE_DEREF, index, 0)
                    } else {
                        self.emit(DELETE_FAST, index, 0)
                    }
                } else if let Some(index) = free_indices.get(&name_id).copied() {
                    self.emit(DELETE_DEREF, index, 0)
                } else {
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

    fn closure_index_id(&self, name: NameId) -> Result<u32, CompileError> {
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
            Scope::Class { .. } => self
                .symbols
                .locals
                .iter()
                .position(|local| *local == name)
                .map(|index| to_u32(index, "class closure variable index"))
                .transpose()?
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
            } => indices
                .get(&name)
                .or_else(|| free_indices.get(&name))
                .copied()
                .ok_or_else(|| {
                    CompileError::Internal(format!(
                        "missing closure variable `{}`",
                        self.resolve_name(name)
                    ))
                }),
        }
    }

    pub(super) fn can_provide_closure(&self, name: &str) -> bool {
        let Some(name_id) = self.name_id(name) else {
            return false;
        };
        self.can_provide_closure_id(name_id)
    }

    pub(super) fn can_provide_closure_id(&self, name: NameId) -> bool {
        match &self.unit.scope {
            Scope::Module => self.symbols.cell_names.contains(name),
            Scope::Class { free_indices, .. } => {
                self.symbols.cell_names.contains(name) || free_indices.contains_key(&name)
            }
            Scope::Function {
                indices,
                free_indices,
                cells,
                ..
            } => {
                indices.contains_key(&name) && cells.contains(name)
                    || free_indices.contains_key(&name)
            }
        }
    }

    pub(super) fn emit_closure_tuple(&mut self, names: &[NameId]) -> Result<(), CompileError> {
        for name in names {
            let local = self.closure_index_id(*name)?;
            self.emit(LOAD_FAST, local, 1)?;
            // CPython lowers pseudo `LOAD_CLOSURE` instructions after checking ordinary fast
            // loads for initialization and after forming superinstructions.
            self.assembler
                .mark_last_load_fast_as_post_analysis_closure();
            self.assembler
                .prevent_last_instruction_fusion_with_previous();
        }
        self.emit_build(BUILD_TUPLE, names.len())
    }

    pub(super) fn emit_pending_comprehension_restores(&mut self) -> Result<(), CompileError> {
        let restores = std::mem::take(&mut self.control.pending_comprehension_restores);
        for (restores, location) in restores {
            self.assembler.set_location(location);
            for (position, restore) in restores.iter().copied().enumerate() {
                self.assembler.fusion_barrier();
                if position > 0 {
                    self.assembler.set_location(location);
                }
                self.emit_comprehension_restore(restore)?;
                self.assembler
                    .prevent_last_instruction_fusion_with_previous();
            }
            // The SWAP/STORE_FAST_MAYBE_NULL restore contracts by one CFG slot.
            self.assembler.add_last_hidden_exception_owner_slots(1);
            // CPython converts STORE_FAST_MAYBE_NULL only after inserting superinstructions.
            self.assembler.prevent_last_instruction_fusion();
        }
        Ok(())
    }

    pub(super) fn emit_comprehension_restore(
        &mut self,
        restore: ComprehensionRestore,
    ) -> Result<(), CompileError> {
        self.emit(STORE_FAST, restore.index, -1)?;
        if usize::try_from(restore.index).is_ok_and(|index| index < self.symbols.fast_local_count)
            && !self.symbols.cell_names.contains(restore.name)
        {
            self.assembler
                .mark_last_store_fast_as_maybe_null_comprehension_restore();
        }
        Ok(())
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
            Scope::Class { .. } => format!("{}.{}", self.unit.qualified_name, name),
            Scope::Function { globals, .. }
                if honor_explicit_global && self.contains_name(globals, name) =>
            {
                name.to_string()
            }
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
        mangle_name(name, self.unit.private_name.as_deref()).into_owned()
    }

    pub(super) fn pre_register_suite_names(&mut self, body: &[Stmt]) -> Result<(), CompileError> {
        #[derive(Default)]
        struct Collector {
            names: Vec<(String, bool)>,
        }

        impl<'ast> Visitor<'ast> for Collector {
            fn visit_stmt(&mut self, statement: &'ast Stmt) {
                match statement {
                    Stmt::AnnAssign(assignment) => {
                        if let Some(value) = &assignment.value {
                            self.visit_expr(value);
                            self.visit_expr(&assignment.target);
                        }
                        return;
                    }
                    Stmt::FunctionDef(definition) => {
                        for decorator in &definition.decorator_list {
                            self.visit_expr(&decorator.expression);
                        }
                        for parameter in definition
                            .parameters
                            .posonlyargs
                            .iter()
                            .chain(&definition.parameters.args)
                            .chain(&definition.parameters.kwonlyargs)
                        {
                            if let Some(default) = &parameter.default {
                                self.visit_expr(default);
                            }
                        }
                        self.names
                            .push((definition.name.as_str().to_string(), false));
                        return;
                    }
                    Stmt::ClassDef(definition) => {
                        for decorator in &definition.decorator_list {
                            self.visit_expr(&decorator.expression);
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
                        self.names
                            .push((definition.name.as_str().to_string(), false));
                        return;
                    }
                    Stmt::TypeAlias(statement) => {
                        if let Expr::Name(name) = statement.name.as_ref() {
                            self.names.push((name.id.as_str().to_string(), false));
                        }
                        return;
                    }
                    Stmt::Import(import) => {
                        for alias in &import.names {
                            let imported = alias.name.as_str();
                            self.names.push((imported.to_string(), true));
                            let bound = alias.asname.as_ref().map_or_else(
                                || imported.split('.').next().unwrap_or(imported),
                                |name| name.as_str(),
                            );
                            self.names.push((bound.to_string(), false));
                        }
                        return;
                    }
                    Stmt::ImportFrom(import) => {
                        if let Some(module) = &import.module {
                            self.names.push((module.as_str().to_string(), true));
                        }
                        for alias in &import.names {
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
                for statement in &handler.body {
                    self.visit_stmt(statement);
                }
            }

            fn visit_expr(&mut self, expression: &'ast Expr) {
                match expression {
                    Expr::Name(name) => self.names.push((name.id.as_str().to_string(), false)),
                    Expr::Attribute(attribute) => {
                        self.visit_expr(&attribute.value);
                        self.names.push((attribute.attr.as_str().to_string(), true));
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
                    _ => {}
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

        let mut collector = Collector::default();
        for statement in body {
            collector.visit_stmt(statement);
        }
        for (name, force_name) in collector.names {
            let name_id = self.required_name_id(&name);
            let is_local_or_free = matches!(
                &self.unit.scope,
                Scope::Function {
                    indices,
                    free_indices,
                    globals,
                    ..
                } if indices.contains_key(&name_id) && !globals.contains(name_id)
                    || free_indices.contains_key(&name_id)
            ) || self.temporary_index(&name).is_some();
            if force_name || !is_local_or_free {
                self.name_index(&name)?;
            }
        }
        Ok(())
    }

    pub(super) fn pre_register_expression_names(
        &mut self,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        #[derive(Default)]
        struct Collector {
            names: Vec<(String, bool)>,
        }

        impl<'ast> Visitor<'ast> for Collector {
            fn visit_expr(&mut self, expression: &'ast Expr) {
                match expression {
                    Expr::Name(name) => self.names.push((name.id.as_str().to_string(), false)),
                    Expr::Attribute(attribute) => {
                        self.visit_expr(&attribute.value);
                        self.names.push((attribute.attr.as_str().to_string(), true));
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
            let name_id = self.required_name_id(&name);
            let is_local_or_free = matches!(
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
                self.name_index(&name)?;
            }
        }
        Ok(())
    }

    pub(super) fn pre_register_expression_constants(
        &mut self,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        #[derive(Default)]
        struct Collector {
            constants: Vec<Constant>,
        }

        impl<'ast> Visitor<'ast> for Collector {
            fn visit_expr(&mut self, expression: &'ast Expr) {
                if let Some(constant) = literal_constant(expression) {
                    self.constants.push(constant);
                } else {
                    walk_expr(self, expression);
                }
            }
        }

        let mut collector = Collector::default();
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

    pub(super) fn emit_with_exception_policy(
        &mut self,
        opcode: Opcode,
        argument: u32,
        effect: i32,
        exception_policy: ExceptionPolicy,
    ) -> Result<(), CompileError> {
        self.apply_compiler_stack_effect(opcode, argument, effect)?;
        self.assembler.emit_with_depth(
            opcode,
            argument,
            self.control.depth.cast_unsigned(),
            exception_policy,
        );
        Ok(())
    }

    pub(super) fn emit_reraise(&mut self, state: ReraiseStackState) -> Result<(), CompileError> {
        let (argument, effect) = state.argument_and_effect();
        self.apply_stack_effect(effect)?;
        self.assembler.emit_with_depth(
            RERAISE,
            argument,
            self.control.depth.cast_unsigned(),
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
        self.emit(LOAD_COMMON_CONSTANT, constant.argument(), 1)
    }

    pub(super) fn emit_resume(
        &mut self,
        location: ResumeLocation,
        depth: ResumeDepth,
    ) -> Result<(), CompileError> {
        self.emit(RESUME, location.at_depth(depth.is_one()), 0)?;
        if location == ResumeLocation::AFTER_AWAIT
            && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR) != 0
        {
            self.control.coroutine_suspension_epoch =
                self.control.coroutine_suspension_epoch.saturating_add(1);
        }
        Ok(())
    }

    pub(super) fn emit_binary_operation(
        &mut self,
        operation: BinaryOperation,
    ) -> Result<(), CompileError> {
        self.emit(BINARY_OP, operation.argument(), -1)
    }

    pub(super) fn emit_comparison(
        &mut self,
        operation: ComparisonOperation,
        mode: ComparisonMode,
    ) -> Result<(), CompileError> {
        let argument = match mode {
            ComparisonMode::Value => operation.argument(),
            ComparisonMode::Boolean => operation.boolean_argument(),
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
            .emit_owned_fast_with_depth(argument, self.control.depth.cast_unsigned());
        Ok(())
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

    pub(super) fn emit_jump_forward_with_exception_policy(
        &mut self,
        opcode: Opcode,
        label: Label,
        effect: i32,
        exception_policy: ExceptionPolicy,
    ) -> Result<(), CompileError> {
        if is_unconditional_jump(opcode)
            && let Some(location) = self.take_trailing_nop_location()
        {
            self.assembler.set_location(location);
        }
        self.apply_compiler_stack_effect(opcode, 0, effect)?;
        self.assembler.emit_forward_with_depth(
            opcode,
            label,
            self.control.depth.cast_unsigned(),
            exception_policy,
        );
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

    pub(super) fn emit_jump_backward_with_exception_policy(
        &mut self,
        opcode: Opcode,
        label: Label,
        effect: i32,
        exception_policy: ExceptionPolicy,
    ) -> Result<(), CompileError> {
        if is_unconditional_jump(opcode)
            && let Some(location) = self.take_trailing_nop_location()
        {
            self.assembler.set_location(location);
        }
        self.apply_compiler_stack_effect(opcode, 0, effect)?;
        self.assembler
            .emit_backward_with_depth_and_exception_policy(
                opcode,
                label,
                self.control.depth.cast_unsigned(),
                exception_policy,
            );
        Ok(())
    }

    pub(super) fn take_trailing_nop_location(&mut self) -> Option<SourceLocation> {
        let (location, exclusion) = self.assembler.take_trailing_nop_location()?;
        if let Some(exclusion) = exclusion {
            self.control
                .generator_region_exclusions
                .retain(|candidate| *candidate != exclusion);
            for exclusions in &mut self.control.active_with_region_exclusions {
                exclusions.retain(|candidate| *candidate != exclusion);
            }
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.retain(|candidate| *candidate != exclusion);
            }
            for exclusions in &mut self.control.active_comprehension_region_exclusions {
                exclusions.retain(|candidate| *candidate != exclusion);
            }
        }
        Some(location)
    }

    pub(super) fn apply_compiler_stack_effect(
        &mut self,
        opcode: Opcode,
        argument: u32,
        declared: i32,
    ) -> Result<(), CompileError> {
        let effect = validate_compiler_stack_effect(opcode, argument, declared)?;
        self.apply_stack_effect(effect)
    }

    fn apply_stack_effect(&mut self, effect: i32) -> Result<(), CompileError> {
        self.control.depth += effect;
        if self.control.depth < 0 {
            return Err(CompileError::Internal(
                "compiler produced a negative stack depth".to_string(),
            ));
        }
        self.control.max_depth = self
            .control
            .max_depth
            .max(self.control.depth.cast_unsigned());
        Ok(())
    }

    pub(super) fn set_depth(&mut self, depth: i32) {
        debug_assert!(depth >= 0);
        self.control.depth = depth;
        self.control.max_depth = self.control.max_depth.max(depth.cast_unsigned());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompilerStackEffect {
    Fixed(i32),
    ContextDependent,
}

/// Returns the effect on the compiler's logical value stack when it is fixed.
pub(super) fn compiler_stack_effect(opcode: Opcode, argument: u32) -> CompilerStackEffect {
    match opcode {
        // CPython's physical interpreter stack retains the return value for frame teardown, but
        // the compiler has finished consuming its logical value stack.
        RETURN_VALUE => CompilerStackEffect::Fixed(-1),
        // RERAISE can discard additional logical exception-state values introduced by the
        // surrounding lowering path. Its effect is supplied by that path explicitly.
        RERAISE => CompilerStackEffect::ContextDependent,
        _ => CompilerStackEffect::Fixed(
            i32::try_from(opcode_num_pushed(opcode, argument))
                .expect("opcode push count fits in i32")
                - i32::try_from(opcode_num_popped(opcode, argument))
                    .expect("opcode pop count fits in i32"),
        ),
    }
}

pub(super) fn validate_compiler_stack_effect(
    opcode: Opcode,
    argument: u32,
    declared: i32,
) -> Result<i32, CompileError> {
    match compiler_stack_effect(opcode, argument) {
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

pub(super) fn mangle_name<'a>(name: &'a str, private_name: Option<&str>) -> Cow<'a, str> {
    if !name.starts_with("__") || name.ends_with("__") || name.contains('.') {
        return Cow::Borrowed(name);
    }
    let Some(class_name) = private_name
        .map(|name| name.trim_start_matches('_'))
        .filter(|name| !name.is_empty())
    else {
        return Cow::Borrowed(name);
    };
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
