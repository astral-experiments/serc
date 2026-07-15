use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::Arc;

use ruff_python_ast::token::Tokens;
use ruff_python_ast::{Expr, Stmt, Suite};
use ruff_text_size::Ranged;
use ruff_text_size::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    BUILD_SET, COPY, COPY_FREE_VARS, LOAD_CONST, LOAD_FAST, LOAD_LOCALS, LOAD_SMALL_INT, MAKE_CELL,
    MAKE_FUNCTION, NOP, POP_TOP, RETURN_GENERATOR, RETURN_VALUE, SETUP_ANNOTATIONS, STORE_DEREF,
};
use crate::CompileError;
use crate::assembler::{AssembledCode, Assembler, SourceLocation};
use crate::code_object::{CodeObject, Constant, LocalsIdentity, MarshalProvenance};
use crate::target::code_flags::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_FUTURE_ANNOTATIONS, CO_GENERATOR, CO_HAS_DOCSTRING,
    CO_NEWLOCALS, CO_OPTIMIZED, CO_VARARGS, CO_VARKEYWORDS,
};
use crate::target::local_kinds::{
    CO_FAST_ARG_KW, CO_FAST_ARG_POS, CO_FAST_ARG_VAR, CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN,
    CO_FAST_LOCAL,
};
use crate::target::operands::{FunctionAttribute, ResumeLocation, UnaryIntrinsic};

use super::constant::{OutputState, fold_constant};
use super::definition::clean_doc;
use super::scope::{
    LoweringFunctionPlan, NameId, NameSet, ScopeId, class_static_attributes,
    collect_conditional_simple_annotations, has_simple_annotations,
};
use super::source::{CompilationContext, implicit_return_range};
use super::statement::{
    ControlFlowState, statement_uses_implicit_return_location, suite_terminates,
};
use super::symbol::{ReraiseStackState, ResumeDepth, SymbolState, to_u32};
use super::{Compiler, Scope};

#[derive(Debug)]
pub(super) struct CodeUnit {
    pub(super) scope: Scope,
    pub(super) scope_id: ScopeId,
    pub(super) generic_target_qualified_name: Option<String>,
    pub(super) child_qualified_name_parent: Option<String>,
    pub(super) class_scope_is_nested: bool,
    pub(super) module_annotation_index: u32,
    pub(super) class_conditional_annotations: VecDeque<TextRange>,
    pub(super) annotation_classdict_index: Option<u32>,
    name: String,
    pub(super) qualified_name: String,
    pub(super) private_name: Option<String>,
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

impl<'source> Compiler<'source> {
    fn new(
        context: Arc<CompilationContext<'source>>,
        unit: CodeUnit,
        symbols: SymbolState,
    ) -> Self {
        Self {
            context,
            cached_source_line: Cell::new(None),
            unit,
            output: OutputState::default(),
            symbols,
            control: ControlFlowState::default(),
            assembler: Assembler::default(),
        }
    }

    pub(crate) fn module(
        filename: &str,
        source: &'source str,
        tokens: &'source Tokens,
        body: &Suite,
    ) -> Self {
        let context = Arc::new(CompilationContext::new(filename, source, tokens, body));
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
        context: Arc<CompilationContext<'source>>,
        name: &str,
        qualified_name: String,
        first_line_number: u32,
        plan: &LoweringFunctionPlan,
        scope_id: ScopeId,
        arg_count: u32,
        positional_only_arg_count: u32,
        keyword_only_arg_count: u32,
        parameter_flags: u32,
    ) -> Result<Self, CompileError> {
        let parameter_count = usize::try_from(
            arg_count
                + keyword_only_arg_count
                + u32::from(parameter_flags & CO_VARARGS != 0)
                + u32::from(parameter_flags & CO_VARKEYWORDS != 0),
        )
        .unwrap_or(usize::MAX);
        let names = &context.scopes.names;
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
        cell_only_locals.sort_by(|left, right| names.resolve(*left).cmp(names.resolve(*right)));
        locals.extend(cell_only_locals);
        let indices = locals
            .iter()
            .enumerate()
            .map(|(index, name)| Ok((*name, to_u32(index, "local variable count")?)))
            .collect::<Result<FxHashMap<_, _>, CompileError>>()?;
        let local_count = locals.len();
        let free_indices = plan
            .freevars
            .iter()
            .enumerate()
            .map(|(index, name)| Ok((*name, to_u32(local_count + index, "free variable count")?)))
            .collect::<Result<FxHashMap<_, _>, CompileError>>()?;
        locals.extend(plan.freevars.iter().copied());

        let cell_names: NameSet = plan.cellvars.iter().copied().collect();
        let global_names = plan.globals.iter().copied().collect();
        let initialized_locals = locals.iter().take(parameter_count).copied().collect();
        let free_names = plan.freevars.clone();

        let symbols = SymbolState {
            locals,
            fast_local_count,
            cell_names: cell_names.clone(),
            free_names,
            initialized_locals,
            ..SymbolState::default()
        };
        let unit = CodeUnit::from_spec(CodeUnitSpec {
            scope: Scope::Function {
                indices,
                free_indices,
                cells: cell_names,
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
        context: Arc<CompilationContext<'source>>,
        name: &str,
        qualified_name: String,
        first_line_number: u32,
        scope_id: ScopeId,
        globals: FxHashSet<NameId>,
        nonlocals: FxHashSet<NameId>,
        freevars: &[NameId],
        comprehension_targets: &[NameId],
        bound_names: FxHashSet<NameId>,
        needs_class_closure: bool,
        has_methods: bool,
        has_conditional_annotations: bool,
        future_flags: u32,
    ) -> Self {
        let mut locals = comprehension_targets.to_vec();
        let fast_local_count = locals.len();
        let mut cell_names = NameSet::default();
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
        let free_indices = freevars
            .iter()
            .enumerate()
            .map(|(index, name)| (*name, u32::try_from(free_start + index).unwrap_or(u32::MAX)))
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
        let unit = CodeUnit::from_spec(CodeUnitSpec {
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
        Self::new(context, unit, symbols)
    }

    pub(crate) fn compile_module(mut self, body: &Suite) -> Result<CodeObject, CompileError> {
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
        let future_module_annotations =
            self.unit.flags & CO_FUTURE_ANNOTATIONS != 0 && module_annotations;
        if module_annotations {
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
        if module_annotations {
            self.name_index("__conditional_annotations__")?;
            if let Some(statement) = body.first() {
                self.assembler
                    .set_location(self.source_location(statement.range()));
            }
            if !future_module_annotations {
                if simple_module_annotations {
                    let annotation_child = self.compile_module_annotations(body)?;
                    self.emit_deferred_constant_before_return(Constant::Code(Box::new(
                        annotation_child,
                    )))?;
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

        let body = if let Some(Stmt::Expr(expression)) = body.first() {
            if let Expr::StringLiteral(string) = expression.value.as_ref() {
                let constant =
                    self.add_constant(Constant::String(clean_doc(string.value.to_str())))?;
                self.assembler
                    .set_location(self.source_location(expression.range));
                self.emit(LOAD_CONST, constant, 1)?;
                self.store_name("__doc__")?;
                &body[1..]
            } else if module_annotations && fold_constant(&expression.value).is_some() {
                &body[1..]
            } else {
                body.as_slice()
            }
        } else {
            body.as_slice()
        };

        let body_start = self.assembler.instruction_count();
        let add_implicit_return = !suite_terminates(body);
        self.compile_suite_inner(body, add_implicit_return)?;
        let add_implicit_return =
            add_implicit_return && !self.control.statements.emitted_fallthrough_return;
        if add_implicit_return
            && body
                .last()
                .is_some_and(statement_uses_implicit_return_location)
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
        mut self,
        body: &Suite,
    ) -> Result<CodeObject, CompileError> {
        self.emit_function_prologue()?;

        let body = if let Some(Stmt::Expr(expression)) = body.first() {
            if let Expr::StringLiteral(string) = expression.value.as_ref() {
                self.add_constant(Constant::String(clean_doc(string.value.to_str())))?;
                self.unit.flags |= CO_HAS_DOCSTRING;
                &body[1..]
            } else {
                body.as_slice()
            }
        } else {
            body.as_slice()
        };

        let body_start = self.assembler.instruction_count();
        let add_implicit_return = !suite_terminates(body);
        self.compile_suite_inner(body, add_implicit_return)?;
        let add_implicit_return =
            add_implicit_return && !self.control.statements.emitted_fallthrough_return;
        if add_implicit_return
            && body
                .last()
                .is_some_and(statement_uses_implicit_return_location)
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
        self.finish_inner(add_implicit_return)
    }

    pub(super) fn compile_class_body(mut self, body: &Suite) -> Result<CodeObject, CompileError> {
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
        let qualified_name =
            self.add_constant(Constant::String(self.unit.qualified_name.clone()))?;
        self.emit(LOAD_CONST, qualified_name, 1)?;
        self.output.qualified_name_constant_index = Some(qualified_name);
        self.store_name("__qualname__")?;
        if u8::try_from(self.unit.first_line_number).is_ok() {
            self.emit(LOAD_SMALL_INT, self.unit.first_line_number, 1)?;
        } else {
            let first_line_number =
                self.add_constant(Constant::Int(u64::from(self.unit.first_line_number)))?;
            self.emit(LOAD_CONST, first_line_number, 1)?;
        }
        self.store_name("__firstlineno__")?;
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
        if self.unit.flags & CO_FUTURE_ANNOTATIONS != 0 && has_simple_annotations(body) {
            self.emit(SETUP_ANNOTATIONS, 0, 0)?;
        }

        let body = if let Some(Stmt::Expr(expression)) = body.first() {
            if let Expr::StringLiteral(string) = expression.value.as_ref() {
                let docstring =
                    self.add_constant(Constant::String(clean_doc(string.value.to_str())))?;
                self.assembler
                    .set_location(self.source_location(expression.range));
                self.emit(LOAD_CONST, docstring, 1)?;
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
        self.compile_suite(body)?;
        debug_assert!(self.unit.class_conditional_annotations.is_empty());
        if let Some(Stmt::Expr(expression)) = body.last()
            && fold_constant(&expression.value).is_some()
        {
            let range = if let Expr::FString(fstring) = expression.value.as_ref() {
                Self::fstring_result_range(fstring)
            } else {
                expression.range
            };
            self.assembler.set_location(self.source_location(range));
            self.emit(NOP, 0, 0)?;
        } else if let Some(Stmt::If(statement)) = body.last()
            && matches!(statement.test.as_ref(), Expr::EllipsisLiteral(_))
            && let Some(Stmt::Expr(expression)) = statement.body.last()
            && fold_constant(&expression.value).is_some()
        {
            // CPython transfers the final folded expression's location to the class footer.
            footer_location = self.source_location(expression.range);
        } else if self.assembler.instruction_count() > body_start
            && let Some(location) = self.assembler.last_instruction_location()
        {
            self.assembler.set_location(location);
        } else if let Some(statement) = body.last() {
            if !matches!(
                statement,
                Stmt::AnnAssign(annotation) if annotation.simple && annotation.value.is_none()
            ) {
                self.assembler
                    .set_location(self.source_location(statement.range()));
                self.emit(NOP, 0, 0)?;
            }
        }

        if self.unit.flags & CO_FUTURE_ANNOTATIONS == 0
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
        let static_attributes = self.add_constant(Constant::Tuple(
            class_static_attributes(body)
                .into_iter()
                .map(Constant::String)
                .collect(),
        ))?;
        self.emit(LOAD_CONST, static_attributes, 1)?;
        self.store_name("__static_attributes__")?;
        if has_classdict_cell {
            let index = self
                .symbols
                .locals
                .iter()
                .position(|name| *name == self.required_name_id("__classdict__"))
                .expect("classdict cell is present");
            self.emit(LOAD_FAST, to_u32(index, "classdict cell index")?, 1)?;
            self.store_name("__classdictcell__")?;
        }
        if has_class_cell {
            let index = self
                .symbols
                .locals
                .iter()
                .position(|name| *name == self.required_name_id("__class__"))
                .expect("class cell is present");
            self.emit(LOAD_FAST, to_u32(index, "class cell index")?, 1)?;
            self.emit(COPY, 1, 1)?;
            self.store_name("__classcell__")?;
            self.emit(RETURN_VALUE, 0, -1)?;
            self.finish_inner(false)
        } else {
            self.finish()
        }
    }

    pub(super) fn finish(self) -> Result<CodeObject, CompileError> {
        self.finish_inner(true)
    }

    pub(super) fn finish_inner(
        mut self,
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
        if self.output.constants.is_empty()
            && !(self.unit.name == "<lambda>" && self.unit.flags & CO_GENERATOR != 0)
        {
            self.add_constant(Constant::None)?;
        }
        if let Some(start) = self.control.generators.generator_region_start {
            let handler = self.assembler.label();
            self.assembler.mark(handler);
            let mut region_start = start;
            for (exclusion_start, exclusion_end) in
                &self.control.generators.generator_region_exclusions
            {
                self.assembler.add_exception_region(
                    region_start,
                    *exclusion_start,
                    handler,
                    0,
                    true,
                );
                region_start = *exclusion_end;
            }
            self.assembler
                .add_exception_region(region_start, handler, handler, 0, true);
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

        for (instruction, constant) in std::mem::take(&mut self.output.deferred_constants) {
            let index = self.add_constant(constant)?;
            if let Some(instruction) = instruction {
                self.assembler.patch_argument(instruction, index)?;
            }
        }
        for (instruction, name) in std::mem::take(&mut self.output.deferred_names) {
            let index = self.name_index(&name)?;
            self.assembler.patch_argument(instruction, index)?;
        }
        self.assembler.resolve_operand_patches()?;
        self.assembler.optimize_constant_pops();
        self.remove_unused_constants();

        let local_kinds = self
            .symbols
            .locals
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let positional_only = usize::try_from(self.unit.positional_only_arg_count).unwrap();
                let positional = usize::try_from(self.unit.arg_count).unwrap();
                let keyword_only = usize::try_from(self.unit.keyword_only_arg_count).unwrap();
                let argument_kind = if index < positional_only {
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
                    | if self.is_hidden_id(*name) {
                        CO_FAST_HIDDEN
                    } else {
                        0
                    }
            })
            .collect();
        let output_locals = self
            .symbols
            .locals
            .iter()
            .map(|name| self.mangled_name(self.resolve_name(*name)))
            .collect();
        let parameter_count = usize::try_from(
            self.unit.arg_count
                + self.unit.keyword_only_arg_count
                + u32::from(self.unit.flags & CO_VARARGS != 0)
                + u32::from(self.unit.flags & CO_VARKEYWORDS != 0),
        )
        .unwrap_or(usize::MAX);
        let AssembledCode {
            bytecode,
            line_table,
            exception_table,
            max_depth: assembled_max_depth,
            removed_max_depth,
        } = self.assembler.finish_code(
            self.unit.first_line_number,
            self.symbols.fast_local_count,
            parameter_count,
        )?;
        let max_depth = if removed_max_depth == Some(self.control.stack.max_depth)
            && assembled_max_depth < self.control.stack.max_depth
        {
            assembled_max_depth
        } else {
            self.control.stack.max_depth
        };
        let stack_size = if self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR)
            != 0
            && self.control.generators.generator_region_start.is_some()
        {
            max_depth.max(2)
        } else {
            max_depth.max(1)
        };
        let code = CodeObject {
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
                locals_identity: if self.unit.annotation_thunk {
                    LocalsIdentity::Fresh
                } else {
                    LocalsIdentity::ByValue
                },
                explicitly_interned_strings: self.output.interned_constant_strings,
                qualified_name_constant_index: self.output.qualified_name_constant_index,
            },
        };
        code.validate()?;
        Ok(code)
    }

    pub(super) fn emit_function_prologue(&mut self) -> Result<(), CompileError> {
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
        if self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != 0 {
            let line = i32::try_from(self.unit.first_line_number).unwrap_or(i32::MAX);
            self.assembler
                .set_location(SourceLocation::new(line, line, -1, -1));
            // This physical +1/-1 frame-startup pair precedes logical expression lowering and
            // intentionally does not participate in the compiler's value-depth accounting.
            self.assembler.emit(RETURN_GENERATOR, 0);
            self.assembler.emit(POP_TOP, 0);
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
            class_conditional_annotations: VecDeque::new(),
            annotation_classdict_index: None,
            name: spec.name,
            qualified_name: spec.qualified_name,
            private_name: spec.private_name,
            arg_count: spec.arg_count,
            positional_only_arg_count: spec.positional_only_arg_count,
            keyword_only_arg_count: spec.keyword_only_arg_count,
            flags: spec.flags,
            first_line_number: spec.first_line_number,
            annotation_thunk: false,
        }
    }
}
