use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;

use ruff_python_ast::token::TokenKind;
use ruff_python_ast::visitor::Visitor;
use ruff_python_ast::{Expr, Keyword, Stmt, StmtClassDef, StmtFunctionDef, TypeParam};
use ruff_text_size::Ranged;
use rustc_hash::FxHashSet;

use super::{
    BUILD_CONST_KEY_MAP, BUILD_LIST, BUILD_MAP, BUILD_TUPLE, CALL, CALL_FUNCTION_EX, CALL_KW,
    CONTAINS_OP, COPY, DICT_MERGE, LIST_APPEND, LIST_EXTEND, LOAD_BUILD_CLASS, LOAD_CONST,
    LOAD_DEREF, LOAD_FAST, LOAD_SMALL_INT, MAP_ADD, NOT_TAKEN, POP_JUMP_IF_FALSE, PUSH_NULL,
    RAISE_VARARGS, RETURN_VALUE, STORE_SUBSCR, SWAP, UNPACK_SEQUENCE,
};
use crate::assembler::SourceLocation;
use crate::code_object::{CodeObject, Constant};
use crate::target::code_flags::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_FUTURE_ANNOTATIONS, CO_GENERATOR, CO_METHOD, CO_NESTED,
    CO_VARARGS, CO_VARKEYWORDS,
};
use crate::target::operands::{
    BinaryIntrinsic, CommonConstant, ComparisonOperation, FunctionAttribute, UnaryIntrinsic,
};
use crate::{CompileError, PythonVersion};

use super::constant::folded_constant_owner_slots;
use super::scope::{
    LoweringFunctionPlan, NameId, ReferenceCollector, annotation_nested_required_names,
    annotation_nested_required_names_with_mangler, collect_conditional_simple_annotations,
    collect_simple_annotations, expression_required_names, expression_required_names_with_mangler,
    expression_requirements_outside_comprehensions, function_annotation_expressions,
    header_expression_storage, inlined_comprehension_cell_names_in_expression,
    inlined_comprehension_local_names_in_expression, nested_expression_required_names,
};
use super::source::unparse_annotation;
use super::symbol::{ComparisonMode, to_u32, unsupported};
use super::{CPYTHON_STACK_USE_GUIDELINE, Compiler, SUPPORTED_FUTURE_FLAGS, Scope};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DefinitionDecorators {
    Compile,
    AlreadyCompiled,
}

#[derive(Clone, Copy, Debug)]
struct CompiledFunctionDefaults {
    positional: bool,
    keyword: bool,
}

impl CompiledFunctionDefaults {
    fn count(self) -> usize {
        usize::from(self.positional) + usize::from(self.keyword)
    }
}

impl Compiler<'_> {
    /// Class bindings apply to direct annotation loads; nested functions and comprehensions retain captures.
    fn class_visible_annotation_requirements(
        &self,
        expression: &Expr,
        unmangled: bool,
    ) -> BTreeSet<String> {
        let shadows = |name| {
            if unmangled {
                self.class_scope_shadows_unmangled_annotation_name(name)
            } else {
                self.class_scope_shadows_annotation_name(name)
            }
        };
        self.class_visible_annotation_requirements_with(expression, shadows)
    }

    fn class_visible_annotation_requirements_with(
        &self,
        expression: &Expr,
        shadows: impl Fn(NameId) -> bool,
    ) -> BTreeSet<String> {
        let mut required = expression_required_names(expression);
        if required
            .iter()
            .any(|name| shadows(self.required_name_id(name)))
        {
            let nested = annotation_nested_required_names(expression);
            required.retain(|name| !shadows(self.required_name_id(name)) || nested.contains(name));
        }
        required
    }

    fn type_parameter_scope_requirements(
        &self,
        type_params: &ruff_python_ast::TypeParams,
        private_class: Option<&str>,
    ) -> BTreeSet<String> {
        if let Some(private_class) = private_class {
            let mut declared = FxHashSet::default();
            let mut required = BTreeSet::new();
            for parameter in type_params {
                declared.insert(parameter.name().as_str());
                let mangle = |name: &str| {
                    if declared.contains(name) {
                        super::symbol::mangle_name(name, Some(private_class)).into_owned()
                    } else {
                        name.to_owned()
                    }
                };
                let mut collect = |expression: &Expr| {
                    let nested = annotation_nested_required_names_with_mangler(expression, &mangle)
                        .into_iter()
                        .map(|name| mangle(&name))
                        .collect::<FxHashSet<_>>();
                    for name in expression_required_names_with_mangler(expression, &mangle) {
                        let name = mangle(&name);
                        let shadows = self.class_scope_shadows_unmangled_annotation_name(
                            self.required_name_id(&name),
                        );
                        if !shadows || nested.contains(&name) {
                            required.insert(name);
                        }
                    }
                };
                if let TypeParam::TypeVar(parameter) = parameter
                    && let Some(bound) = parameter.bound.as_deref()
                {
                    collect(bound);
                }
                if let Some(default) = parameter.default() {
                    collect(default);
                }
            }
            return required;
        }
        let shadows = |name| self.class_scope_shadows_annotation_name(name);
        let mut required = BTreeSet::new();
        for parameter in type_params {
            if let TypeParam::TypeVar(parameter) = parameter
                && let Some(bound) = parameter.bound.as_deref()
            {
                required.extend(self.class_visible_annotation_requirements_with(bound, shadows));
            }
            if let Some(default) = parameter.default() {
                required.extend(self.class_visible_annotation_requirements_with(default, shadows));
            }
        }
        required
    }

    pub(super) fn compile_type_alias(
        &mut self,
        statement: &ruff_python_ast::StmtTypeAlias,
    ) -> Result<(), CompileError> {
        let Expr::Name(name) = statement.name.as_ref() else {
            return Err(CompileError::Internal(
                "type alias name is not an identifier".to_string(),
            ));
        };
        if let Some(type_params) = statement.type_params.as_deref() {
            return self.compile_generic_type_alias(
                name.id.as_str(),
                &statement.value,
                type_params,
                statement,
            );
        }

        self.assembler
            .set_location(self.source_location(statement.range));
        self.context
            .record_constant_string_origin(name.id.as_str(), true);
        self.load_string_constant(name.id.as_str())?;
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.compile_type_parameter_thunk(
            name.id.as_str(),
            &statement.value,
            statement.range,
            self.child_function_is_nested(),
            true,
        )?;
        self.emit_build(BUILD_TUPLE, 3)?;
        self.emit_unary_intrinsic(UnaryIntrinsic::TYPEALIAS)?;
        self.store_name(name.id.as_str())
    }

    fn compile_generic_type_alias(
        &mut self,
        name: &str,
        value: &Expr,
        type_params: &ruff_python_ast::TypeParams,
        statement: &ruff_python_ast::StmtTypeAlias,
    ) -> Result<(), CompileError> {
        let type_names: FxHashSet<_> = type_params
            .iter()
            .map(|parameter| self.required_name_id(parameter.name().as_str()))
            .collect();
        let locals = type_params
            .iter()
            .map(|parameter| self.required_name_id(parameter.name().as_str()))
            .collect();
        let mut requirements = self.type_parameter_scope_requirements(type_params, None);
        requirements.extend(self.class_visible_annotation_requirements(value, false));
        let requirements = self.name_ids(&requirements);
        let cellvars = requirements
            .iter()
            .filter(|name| type_names.contains(*name))
            .copied()
            .collect();
        let mut freevars = requirements
            .iter()
            .filter(|name| !type_names.contains(*name))
            .filter(|name| self.can_provide_closure_id(**name))
            .copied()
            .collect();
        if matches!(self.unit.scope, Scope::Class { .. }) {
            let classdict = self.required_name_id("__classdict__");
            self.insert_lexical_name(&mut freevars, classdict);
        }
        freevars.sort_unstable_by_key(|name| self.mangled_name(self.resolve_name(*name)));
        let plan = LoweringFunctionPlan {
            locals,
            cellvars,
            freevars,
            ..LoweringFunctionPlan::default()
        };
        let wrapper_name = format!("<generic parameters of {name}>");
        let wrapper_flags = if self.child_function_is_nested() {
            CO_NESTED
        } else {
            0
        } | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        let mut wrapper = Self::function(
            Rc::clone(&self.context),
            &wrapper_name,
            self.child_qualified_name(&wrapper_name),
            self.line_number(u32::from(statement.range.start())),
            &plan,
            self.unit.scope_id,
            0,
            0,
            0,
            wrapper_flags,
        )?;
        wrapper.inherit_lexical_context(self);
        let type_parameter_names = type_names.iter().copied().collect();
        wrapper.symbols.type_parameter_names = type_parameter_names;
        wrapper.unit.generic_target_qualified_name =
            Some(self.annotation_child_qualified_name(name));
        if matches!(self.unit.scope, Scope::Class { .. }) {
            wrapper.unit.annotation_classdict_index = Some(wrapper.closure_index("__classdict__")?);
        }
        wrapper.emit_function_prologue()?;
        wrapper
            .assembler
            .set_location(wrapper.source_location(statement.range));
        wrapper.context.record_constant_string_origin(name, true);
        wrapper.load_string_constant(name)?;
        wrapper.compile_type_parameters(type_params)?;
        wrapper.compile_type_parameter_thunk(name, value, statement.range, true, true)?;
        wrapper.emit_build(BUILD_TUPLE, 3)?;
        wrapper.emit_unary_intrinsic(UnaryIntrinsic::TYPEALIAS)?;
        wrapper.emit(RETURN_VALUE, 0, -1)?;
        let wrapper = wrapper.finish_inner(false)?;
        let wrapper_closure_names = plan.freevars;
        if self.context.python_version == PythonVersion::Py312 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        self.emit_function_with_closure(wrapper, &wrapper_closure_names)?;
        if self.context.python_version != PythonVersion::Py312 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        self.emit(CALL, 0, -1)?;
        self.store_name(name)
    }

    pub(super) fn compile_function_definition(
        &mut self,
        definition: &StmtFunctionDef,
    ) -> Result<(), CompileError> {
        if let Some(type_params) = definition.type_params.as_deref() {
            return self.compile_generic_function_definition(definition, type_params);
        }
        self.compile_plain_function_definition(definition, None, DefinitionDecorators::Compile)
    }

    fn compile_generic_function_definition(
        &mut self,
        definition: &StmtFunctionDef,
        type_params: &ruff_python_ast::TypeParams,
    ) -> Result<(), CompileError> {
        for decorator in &definition.decorator_list {
            self.compile_expression(&decorator.expression)?;
        }
        let definition_location = self.definition_location(
            definition.range,
            definition.name.range(),
            TokenKind::Def,
            definition.is_async,
        );
        if self.context.python_version == PythonVersion::Py312 {
            self.assembler.set_location(definition_location);
            self.emit(PUSH_NULL, 0, 1)?;
        }
        let defaults = self.compile_function_defaults(definition, definition_location)?;
        let default_argument_count = defaults.count();
        if default_argument_count == 2 {
            self.assembler.set_location(definition_location);
            self.emit(SWAP, 2, 0)?;
        }
        let type_names: FxHashSet<_> = type_params
            .iter()
            .map(|parameter| self.required_name_id(parameter.name().as_str()))
            .collect();
        let child_scope_id = self.function_scope_id(definition)?;
        let arena = self.scope_arena();
        let mut child_plan = arena
            .function(child_scope_id)
            .ok_or_else(|| CompileError::Internal("function scope has no plan".to_string()))?
            .clone();
        for name in child_plan.required.iter().copied().collect::<Vec<_>>() {
            if type_names.contains(&name) || self.can_provide_closure_id(name) {
                child_plan.insert_freevar(&arena.names, name);
            } else {
                child_plan.mark_global(name);
            }
        }
        let child_plan = child_plan.into_lowering_plan();

        let mut locals = vec![self.required_name_id(".defaults")];
        if defaults.keyword {
            locals.push(self.required_name_id(".kwdefaults"));
        }
        locals.extend(
            type_params
                .iter()
                .map(|parameter| self.required_name_id(parameter.name().as_str())),
        );
        let type_parameter_requirements =
            self.name_ids(self.type_parameter_scope_requirements(type_params, None));
        let wrapper_local_names: FxHashSet<_> = locals.iter().copied().collect();
        let mut wrapper_plan = LoweringFunctionPlan {
            locals,
            cellvars: type_parameter_requirements
                .iter()
                .filter(|name| type_names.contains(*name))
                .copied()
                .collect(),
            required: type_parameter_requirements
                .iter()
                .filter(|name| !wrapper_local_names.contains(*name))
                .copied()
                .collect(),
            ..LoweringFunctionPlan::default()
        };
        let nested_annotation_requirements = self.name_id_set(
            function_annotation_expressions(definition).flat_map(annotation_nested_required_names),
        );
        wrapper_plan.absorb_child_requirements(
            child_plan.required.iter().copied(),
            child_plan
                .annotation_references
                .iter()
                .copied()
                .filter(|name| {
                    self.has_deferred_annotations()
                        && (!self.class_scope_shadows_annotation_name(*name)
                            || nested_annotation_requirements.contains(name))
                }),
        );
        if !self.has_deferred_annotations() && self.unit.flags & CO_FUTURE_ANNOTATIONS == 0 {
            // Legacy signatures execute inside the generic wrapper. Only their
            // nested functions need cells; direct type-parameter loads stay fast.
            for annotation in function_annotation_expressions(definition) {
                for local in
                    self.name_ids(inlined_comprehension_local_names_in_expression(annotation))
                {
                    if !wrapper_plan.locals.contains(&local) {
                        wrapper_plan.locals.push(local);
                    }
                }
                let cells =
                    self.name_id_set(inlined_comprehension_cell_names_in_expression(annotation));
                wrapper_plan
                    .inlined_comprehension_cellvars
                    .extend(cells.iter().copied());
                wrapper_plan.cellvars.extend(cells);
                wrapper_plan.absorb_child_requirements(
                    self.name_ids(nested_expression_required_names(annotation)),
                    [],
                );
            }
            wrapper_plan.required.extend(
                child_plan
                    .annotation_references
                    .iter()
                    .copied()
                    .filter(|name| !wrapper_plan.locals.contains(name)),
            );
        }
        for name in wrapper_plan.required.iter().copied().collect::<Vec<_>>() {
            if self.can_provide_closure_id(name) {
                wrapper_plan.insert_freevar(&arena.names, name);
            } else {
                wrapper_plan.mark_global(name);
            }
        }
        if matches!(self.unit.scope, Scope::Class { .. }) {
            wrapper_plan.insert_freevar(&arena.names, self.required_name_id("__classdict__"));
        }
        let wrapper_name = format!("<generic parameters of {}>", definition.name);
        let target_qualified_name = self.child_qualified_name(definition.name.as_str());
        let wrapper_flags = if self.child_function_is_nested() {
            CO_NESTED
        } else {
            0
        } | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        let mut wrapper = Self::function(
            Rc::clone(&self.context),
            &wrapper_name,
            self.child_qualified_name(&wrapper_name),
            self.line_number(u32::from(
                definition
                    .decorator_list
                    .first()
                    .map_or(definition.range.start(), |decorator| {
                        decorator.expression.range().start()
                    }),
            )),
            &wrapper_plan,
            self.unit.scope_id,
            to_u32(
                default_argument_count,
                "generic function default argument count",
            )?,
            0,
            0,
            wrapper_flags,
        )?;
        wrapper.inherit_lexical_context(self);
        let type_parameter_names = type_names.iter().copied().collect();
        wrapper.symbols.type_parameter_names = type_parameter_names;
        if !self.has_deferred_annotations() {
            wrapper.unit.child_qualified_name_parent = Some(
                self.annotation_child_qualified_name(definition.name.as_str())
                    .rsplit_once('.')
                    .map_or("", |(parent, _)| parent)
                    .to_string(),
            );
        }
        wrapper.unit.generic_target_qualified_name = Some(target_qualified_name);
        if matches!(self.unit.scope, Scope::Class { .. }) {
            wrapper.unit.annotation_classdict_index = Some(wrapper.closure_index("__classdict__")?);
        }
        wrapper.emit_function_prologue()?;
        wrapper.compile_type_parameters(type_params)?;
        wrapper.assembler.set_location(definition_location);
        for index in 0..default_argument_count {
            wrapper.emit(
                LOAD_FAST,
                to_u32(index, "generic function default argument index")?,
                1,
            )?;
        }
        if default_argument_count > 0 {
            // CPython inserts closure loads after fusing the synthetic default
            // argument loads, so the last default does not fuse with a cell.
            wrapper.assembler.prevent_last_instruction_fusion();
        }
        wrapper.compile_plain_function_definition(
            definition,
            Some(defaults),
            DefinitionDecorators::AlreadyCompiled,
        )?;
        wrapper.emit(SWAP, 2, 0)?;
        wrapper.emit_binary_intrinsic(BinaryIntrinsic::SET_FUNCTION_TYPE_PARAMS)?;
        wrapper.emit(RETURN_VALUE, 0, -1)?;
        let wrapper = wrapper.finish_inner(false)?;
        let wrapper_closure_names = wrapper_plan.freevars;
        self.assembler.set_location(definition_location);
        self.emit_function_with_closure(wrapper, &wrapper_closure_names)?;
        if self.context.python_version == PythonVersion::Py312 {
            if default_argument_count > 0 {
                self.emit(
                    SWAP,
                    to_u32(
                        default_argument_count + 1,
                        "generic function wrapper call depth",
                    )?,
                    0,
                )?;
            }
            self.emit(
                CALL,
                to_u32(
                    default_argument_count,
                    "generic function wrapper call argument count",
                )?,
                -i32::try_from(default_argument_count).unwrap() - 1,
            )?;
        } else if default_argument_count == 0 {
            self.emit(PUSH_NULL, 0, 1)?;
            self.emit(CALL, 0, -1)?;
        } else {
            self.emit(
                SWAP,
                to_u32(
                    default_argument_count + 1,
                    "generic function wrapper call depth",
                )?,
                0,
            )?;
            self.emit(
                CALL,
                to_u32(
                    default_argument_count - 1,
                    "generic function wrapper call argument count",
                )?,
                -i32::try_from(default_argument_count).unwrap(),
            )?;
        }
        for decorator in definition.decorator_list.iter().rev() {
            self.assembler
                .set_location(self.source_location(decorator.expression.range()));
            self.emit(CALL, 0, -1)?;
        }
        self.assembler.set_location(definition_location);
        self.store_name(definition.name.as_str())
    }

    fn compile_plain_function_definition(
        &mut self,
        definition: &StmtFunctionDef,
        preloaded_defaults: Option<CompiledFunctionDefaults>,
        decorators: DefinitionDecorators,
    ) -> Result<(), CompileError> {
        let parameters = &definition.parameters;
        if decorators == DefinitionDecorators::Compile {
            for decorator in &definition.decorator_list {
                self.compile_expression(&decorator.expression)?;
            }
        }
        let definition_location = self.definition_location(
            definition.range,
            definition.name.range(),
            TokenKind::Def,
            definition.is_async,
        );

        let defaults = if let Some(defaults) = preloaded_defaults {
            defaults
        } else {
            self.compile_function_defaults(definition, definition_location)?
        };

        let (scope_id, plan) = self.resolved_function_plan(definition)?;
        let closure_names = plan.freevars.clone();
        let annotation_child = if self.has_deferred_annotations() {
            self.compile_function_annotations(definition, &plan)?
        } else {
            self.compile_eager_function_annotations(definition, definition_location)?;
            None
        };

        let arg_count = to_u32(
            parameters.posonlyargs.len() + parameters.args.len(),
            "function argument count",
        )?;
        let positional_only_arg_count = to_u32(
            parameters.posonlyargs.len(),
            "positional-only argument count",
        )?;
        let keyword_only_arg_count =
            to_u32(parameters.kwonlyargs.len(), "keyword-only argument count")?;
        let mut parameter_flags = (if parameters.vararg.is_some() {
            CO_VARARGS
        } else {
            0
        } | if parameters.kwarg.is_some() {
            CO_VARKEYWORDS
        } else {
            0
        } | if matches!(self.unit.scope, Scope::Function { .. }) {
            CO_NESTED
        } else if matches!(self.unit.scope, Scope::Class { .. }) {
            (if self.has_deferred_annotations() {
                CO_METHOD
            } else {
                0
            }) | if self.unit.class_scope_is_nested {
                CO_NESTED
            } else {
                0
            }
        } else {
            0
        }) | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        let contains_yield = self.context.function_contains_yield(definition);
        if definition.is_async && contains_yield {
            parameter_flags |= CO_ASYNC_GENERATOR;
        } else if definition.is_async {
            parameter_flags |= CO_COROUTINE;
        } else if contains_yield {
            parameter_flags |= CO_GENERATOR;
        }
        let qualified_name = self
            .unit
            .generic_target_qualified_name
            .clone()
            .unwrap_or_else(|| self.child_qualified_name(definition.name.as_str()));
        let first_line_number =
            self.line_number(u32::from(if decorators == DefinitionDecorators::Compile {
                definition
                    .decorator_list
                    .first()
                    .map_or(definition.range.start(), |decorator| {
                        decorator.expression.range().start()
                    })
            } else {
                definition.range.start()
            }));
        let mut child = Self::function(
            Rc::clone(&self.context),
            definition.name.as_str(),
            qualified_name,
            first_line_number,
            &plan,
            scope_id,
            arg_count,
            positional_only_arg_count,
            keyword_only_arg_count,
            parameter_flags,
        )?;
        child.inherit_lexical_context(self);
        let child = child.compile_function_body(&definition.body)?;

        self.assembler.set_location(definition_location);
        if let Some((annotation_child, annotation_closure_names)) = annotation_child {
            self.emit_function_with_closure(annotation_child, &annotation_closure_names)?;
        }
        if !closure_names.is_empty() {
            self.emit_closure_tuple(&closure_names)?;
        }
        let constant = self.add_constant(Constant::Code(Box::new(child)))?;
        self.emit(LOAD_CONST, constant, 1)?;
        let mut flags = 0;
        if !closure_names.is_empty() {
            flags |= FunctionAttribute::CLOSURE.argument();
        }
        if function_annotation_expressions(definition).next().is_some() {
            flags |= if self.has_deferred_annotations() {
                FunctionAttribute::ANNOTATE.argument()
            } else {
                FunctionAttribute::ANNOTATIONS.argument()
            };
        }
        if defaults.keyword {
            flags |= FunctionAttribute::KWDEFAULTS.argument();
        }
        if defaults.positional {
            flags |= FunctionAttribute::DEFAULTS.argument();
        }
        self.emit_make_function(flags)?;
        if decorators == DefinitionDecorators::Compile {
            for decorator in definition.decorator_list.iter().rev() {
                self.assembler
                    .set_location(self.source_location(decorator.expression.range()));
                self.emit(CALL, 0, -1)?;
            }
        }
        self.assembler.set_location(definition_location);
        if self.unit.generic_target_qualified_name.is_some() {
            Ok(())
        } else {
            self.store_name(definition.name.as_str())
        }
    }

    /// Create a function after placing its closure below the code object.
    pub(super) fn emit_function_with_closure(
        &mut self,
        code: CodeObject,
        closure_names: &[NameId],
    ) -> Result<(), CompileError> {
        let indices = closure_names
            .iter()
            .map(|name| self.child_closure_index(*name))
            .collect::<Result<Vec<_>, _>>()?;
        self.emit_function_with_closure_indices(code, &indices)
    }

    /// Create a function with a closure resolved across explicit lexical contexts.
    fn emit_function_with_closure_indices(
        &mut self,
        code: CodeObject,
        indices: &[u32],
    ) -> Result<(), CompileError> {
        if !indices.is_empty() {
            self.emit_closure_indices(indices)?;
        }
        let code = self.add_constant(Constant::Code(Box::new(code)))?;
        self.emit(LOAD_CONST, code, 1)?;
        self.emit_make_function(if indices.is_empty() {
            0
        } else {
            FunctionAttribute::CLOSURE.argument()
        })
    }

    fn compile_function_defaults(
        &mut self,
        definition: &StmtFunctionDef,
        definition_location: SourceLocation,
    ) -> Result<CompiledFunctionDefaults, CompileError> {
        let parameters = &definition.parameters;
        let positional_defaults: Vec<_> = parameters
            .posonlyargs
            .iter()
            .chain(&parameters.args)
            .filter_map(|parameter| parameter.default.as_deref())
            .collect();
        if !positional_defaults.is_empty() {
            let folded_defaults = if self.has_deferred_annotations()
                && positional_defaults.len() <= CPYTHON_STACK_USE_GUIDELINE
            {
                positional_defaults
                    .iter()
                    .map(|default| self.context.fold_constant(default))
                    .collect::<Option<Vec<_>>>()
            } else {
                None
            };
            if let Some(defaults) = folded_defaults {
                let mut hidden_owner_slots = positional_defaults.len();
                for default in &positional_defaults {
                    hidden_owner_slots +=
                        folded_constant_owner_slots(default, self.context.python_version);
                }
                let operand_folds =
                    self.record_folded_collection_operands(positional_defaults.iter().copied())?;
                self.assembler.set_location(definition_location);
                self.emit_deferred_constant(Constant::Tuple(defaults))?;
                self.attach_folded_operand_admissions(operand_folds);
                self.assembler
                    .add_last_hidden_exception_owner_slots(hidden_owner_slots);
                // Literal defaults are folded before CFG copy sizing, but their
                // removed loads and BUILD_TUPLE still share this original owner.
                if (4..=CPYTHON_STACK_USE_GUIDELINE).contains(&positional_defaults.len())
                    && positional_defaults.iter().all(|default| {
                        matches!(
                            default,
                            Expr::NoneLiteral(_)
                                | Expr::BooleanLiteral(_)
                                | Expr::NumberLiteral(_)
                                | Expr::StringLiteral(_)
                                | Expr::BytesLiteral(_)
                                | Expr::EllipsisLiteral(_)
                        )
                    })
                {
                    self.assembler
                        .record_last_uniform_fold_owner_slots(hidden_owner_slots);
                }
            } else {
                for default in &positional_defaults {
                    self.compile_expression(default)?;
                }
                self.assembler.set_location(definition_location);
                self.emit_build(BUILD_TUPLE, positional_defaults.len())?;
            }
        }

        let keyword_defaults: Vec<_> = parameters
            .kwonlyargs
            .iter()
            .filter_map(|parameter| {
                parameter
                    .default
                    .as_deref()
                    .map(|default| (parameter.name().as_str(), default))
            })
            .collect();
        if !keyword_defaults.is_empty() {
            let mut names = Vec::new();
            for (name, default) in &keyword_defaults {
                self.assembler.set_location(definition_location);
                let mangled = self.mangled_name(name);
                self.context
                    .record_identifier_constant_origin(name, &mangled);
                if self.has_deferred_annotations() {
                    let name = self.add_constant(Constant::String(mangled))?;
                    self.emit(LOAD_CONST, name, 1)?;
                } else {
                    names.push(Constant::String(mangled));
                }
                self.compile_expression(default)?;
            }
            let count = to_u32(keyword_defaults.len(), "keyword default count")?;
            self.assembler.set_location(definition_location);
            if self.has_deferred_annotations() {
                self.emit(BUILD_MAP, count, 1 - i32::try_from(count).unwrap() * 2)?;
            } else {
                let names = self.add_interned_string_tuple(names)?;
                self.emit(LOAD_CONST, names, 1)?;
                self.emit(BUILD_CONST_KEY_MAP, count, -i32::try_from(count).unwrap())?;
            }
        }

        Ok(CompiledFunctionDefaults {
            positional: !positional_defaults.is_empty(),
            keyword: !keyword_defaults.is_empty(),
        })
    }

    pub(super) fn compile_class_definition(
        &mut self,
        definition: &StmtClassDef,
    ) -> Result<(), CompileError> {
        if let Some(type_params) = definition.type_params.as_deref() {
            return self.compile_generic_class_definition(definition, type_params);
        }
        self.compile_plain_class_definition(definition, DefinitionDecorators::Compile)
    }

    fn compile_generic_class_definition(
        &mut self,
        definition: &StmtClassDef,
        type_params: &ruff_python_ast::TypeParams,
    ) -> Result<(), CompileError> {
        for decorator in &definition.decorator_list {
            self.compile_expression(&decorator.expression)?;
        }
        let type_names: FxHashSet<_> = type_params
            .iter()
            .map(|parameter| self.required_name_id(parameter.name().as_str()))
            .collect();
        let mut locals: Vec<_> = type_params
            .iter()
            .map(|parameter| self.required_name_id(parameter.name().as_str()))
            .collect();
        locals.push(self.required_name_id(".generic_base"));
        locals.push(self.required_name_id(".type_params"));
        let (_, class_plan) = self.class_plan(definition)?;
        let required_names = &class_plan.required;
        let type_parameter_requirements =
            self.type_parameter_scope_requirements(type_params, Some(definition.name.as_str()));
        let mut type_parameter_cell_requirements = type_parameter_requirements.clone();
        let mut wrapper_requirements = type_parameter_requirements;
        let header_mangle = |name: &str| {
            if type_params
                .iter()
                .any(|parameter| parameter.name().as_str() == name)
            {
                super::symbol::mangle_name(name, Some(definition.name.as_str())).into_owned()
            } else {
                name.to_owned()
            }
        };
        let mut header_references = ReferenceCollector::default();
        if let Some(arguments) = definition.arguments.as_deref() {
            let mut collect = |expression: &Expr| {
                header_references.visit_expr(expression);
                let nested =
                    annotation_nested_required_names_with_mangler(expression, &header_mangle)
                        .into_iter()
                        .map(|name| header_mangle(&name))
                        .collect::<BTreeSet<_>>();
                wrapper_requirements.extend(
                    expression_required_names_with_mangler(expression, &header_mangle)
                        .into_iter()
                        .map(|name| header_mangle(&name))
                        .filter(|name| {
                            !self.class_scope_shadows_unmangled_annotation_name(
                                self.required_name_id(name),
                            ) || nested.contains(name)
                        }),
                );
                type_parameter_cell_requirements.extend(nested);
            };
            for argument in &arguments.args {
                collect(argument);
            }
            for keyword in &arguments.keywords {
                collect(&keyword.value);
            }
        }
        let type_parameter_cell_requirements = self.name_id_set(type_parameter_cell_requirements);
        let wrapper_requirements = self.name_id_set(wrapper_requirements);
        let mut cellvars: FxHashSet<_> = type_names.intersection(required_names).copied().collect();
        let emitted_type_name = |name| {
            self.required_name_id(&super::symbol::mangle_name(
                self.resolve_name(name),
                Some(definition.name.as_str()),
            ))
        };
        let emitted_type_names = type_names
            .iter()
            .copied()
            .map(emitted_type_name)
            .collect::<FxHashSet<_>>();
        let local_aliases = type_names
            .iter()
            .copied()
            .filter_map(|name| {
                let emitted = emitted_type_name(name);
                (emitted != name).then_some((emitted, name))
            })
            .collect::<Vec<_>>();
        cellvars.extend(
            type_names.iter().copied().filter(|name| {
                type_parameter_cell_requirements.contains(&emitted_type_name(*name))
            }),
        );
        cellvars.extend(type_names.iter().copied().filter(|name| {
            let emitted = emitted_type_name(*name);
            required_names.contains(&emitted) || class_plan.unmangled_required.contains(&emitted)
        }));
        cellvars.insert(self.required_name_id(".type_params"));
        let mut captures = Vec::new();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "Capture records are sorted by emitted identity before closure emission."
        )]
        for &name in required_names {
            if type_names.contains(&name) || emitted_type_names.contains(&name) {
                continue;
            }
            let enclosing = class_plan
                .enclosing_names
                .get(&name)
                .copied()
                .unwrap_or(name);
            if self.can_provide_closure_id(enclosing) {
                let physical =
                    self.required_name_id(&self.mangled_name(self.resolve_name(enclosing)));
                captures.push((physical, self.closure_index_id(enclosing)?));
            }
        }
        for &name in wrapper_requirements
            .iter()
            .filter(|name| !emitted_type_names.contains(*name))
            .chain(class_plan.unmangled_required.iter())
        {
            if !emitted_type_names.contains(&name)
                && let Some(index) = self.physical_closure_index(name)?
            {
                captures.push((name, index));
            }
        }
        // Class-visible headers always receive the class namespace cell.
        if matches!(self.unit.scope, Scope::Class { .. }) {
            let classdict = self.required_name_id("__classdict__");
            captures.push((classdict, self.closure_index_id(classdict)?));
        }
        captures.sort_unstable_by(|(left, _), (right, _)| {
            self.resolve_name(*left).cmp(self.resolve_name(*right))
        });
        captures.dedup_by_key(|(name, _)| *name);
        let (freevars, wrapper_closure_indices): (Vec<_>, Vec<_>) = captures.into_iter().unzip();
        // Direct header loads honor class bindings even when a nested expression still
        // captures the same name. Type parameters keep their own fast-local or cell lookup.
        let globals = header_references
            .references
            .iter()
            .map(|name| self.required_name_id(&header_mangle(name)))
            .filter(|name| !type_names.contains(name))
            .filter(|name| !emitted_type_names.contains(name))
            .filter(|name| {
                !freevars.contains(name)
                    || self.class_scope_shadows_unmangled_annotation_name(*name)
            })
            .collect();
        let plan = LoweringFunctionPlan {
            locals,
            local_aliases,
            globals,
            cellvars,
            freevars,
            ..LoweringFunctionPlan::default()
        };
        let wrapper_name = format!("<generic parameters of {}>", definition.name);
        let target_qualified_name = self.child_qualified_name(definition.name.as_str());
        let wrapper_child_qualified_name_parent = target_qualified_name
            .rsplit_once('.')
            .map_or("", |(parent, _)| parent)
            .to_string();
        let wrapper_flags = if self.child_function_is_nested() {
            CO_NESTED
        } else {
            0
        } | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        let mut wrapper = Self::function(
            Rc::clone(&self.context),
            &wrapper_name,
            self.child_qualified_name(&wrapper_name),
            self.line_number(u32::from(
                definition
                    .decorator_list
                    .first()
                    .map_or(definition.range.start(), |decorator| {
                        decorator.expression.range().start()
                    }),
            )),
            &plan,
            self.unit.scope_id,
            0,
            0,
            0,
            wrapper_flags,
        )?;
        wrapper.unit.private_name = Some(definition.name.as_str().to_string());
        wrapper.unit.mangled_names = Some(
            type_params
                .iter()
                .map(|parameter| parameter.name().to_string())
                .collect(),
        );
        wrapper.unit.free_names_are_final = true;
        let passthrough_aliases = plan
            .freevars
            .iter()
            .copied()
            .filter(|name| {
                wrapper.mangled_name(wrapper.resolve_name(*name)) != wrapper.resolve_name(*name)
            })
            .collect::<Vec<_>>();
        if let Scope::Function { free_indices, .. } = &mut wrapper.unit.scope {
            // A declared private type parameter keeps its own source lookup while
            // a raw header capture with the same spelling occupies a separate tail slot.
            for name in passthrough_aliases {
                free_indices.remove(&name);
            }
        }
        wrapper
            .symbols
            .imported_scope_names
            .clone_from(&self.symbols.imported_scope_names);
        let type_parameter_names = type_names.iter().copied().collect();
        wrapper.symbols.type_parameter_names = type_parameter_names;
        wrapper.unit.generic_target_qualified_name = Some(target_qualified_name);
        wrapper.unit.child_qualified_name_parent = Some(wrapper_child_qualified_name_parent);
        if matches!(self.unit.scope, Scope::Class { .. }) {
            wrapper.unit.annotation_classdict_index = Some(wrapper.closure_index("__classdict__")?);
        }
        wrapper.emit_function_prologue()?;
        wrapper.compile_type_parameters(type_params)?;
        wrapper.assembler.set_location(wrapper.definition_location(
            definition.range,
            definition.name.range(),
            TokenKind::Class,
            false,
        ));
        wrapper.store_name(".type_params")?;
        // Header argument scopes see the completed type-parameter restriction.
        // Their plans use the already-canonical symbol names, while the wrapper
        // retains its existing metadata identities.
        wrapper.unit.header_symbol_names_are_final = true;
        let class_result = wrapper
            .compile_plain_class_definition(definition, DefinitionDecorators::AlreadyCompiled);
        wrapper.unit.header_symbol_names_are_final = false;
        class_result?;
        wrapper.emit(RETURN_VALUE, 0, -1)?;
        let wrapper = wrapper.finish_inner(false)?;

        let definition_location = self.definition_location(
            definition.range,
            definition.name.range(),
            TokenKind::Class,
            false,
        );
        self.assembler.set_location(definition_location);
        if self.context.python_version == PythonVersion::Py312 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        self.emit_function_with_closure_indices(wrapper, &wrapper_closure_indices)?;
        if self.context.python_version != PythonVersion::Py312 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        self.emit(CALL, 0, -1)?;
        for decorator in definition.decorator_list.iter().rev() {
            self.assembler
                .set_location(self.source_location(decorator.expression.range()));
            self.emit(CALL, 0, -1)?;
        }
        self.assembler.set_location(definition_location);
        self.store_name(definition.name.as_str())
    }

    fn compile_plain_class_definition(
        &mut self,
        definition: &StmtClassDef,
        decorators: DefinitionDecorators,
    ) -> Result<(), CompileError> {
        if decorators == DefinitionDecorators::Compile {
            for decorator in &definition.decorator_list {
                self.compile_expression(&decorator.expression)?;
            }
        }
        let is_generic = self.unit.generic_target_qualified_name.is_some();

        let (scope_id, class_plan) = self.class_plan(definition)?;
        let mut captures = Vec::new();
        let mut body_free_aliases = Vec::new();
        for name in class_plan.required.iter().copied() {
            let physical = self.required_name_id(&super::symbol::mangle_name(
                self.resolve_name(name),
                Some(definition.name.as_str()),
            ));
            let index = if is_generic {
                self.physical_closure_index(physical)?
            } else {
                let enclosing = class_plan
                    .enclosing_names
                    .get(&name)
                    .copied()
                    .unwrap_or(name);
                let can_capture = match &self.unit.scope {
                    Scope::Module => false,
                    Scope::Class { free_indices, .. } => {
                        self.symbols.cell_names.contains(enclosing)
                            || free_indices.contains_key(&enclosing)
                    }
                    Scope::Function { .. } => self.can_provide_closure_id(enclosing),
                };
                can_capture
                    .then(|| self.closure_index_id(enclosing))
                    .transpose()?
            };
            if let Some(index) = index {
                captures.push((physical, index));
                body_free_aliases.extend([(name, physical), (physical, physical)]);
            }
        }
        #[expect(
            clippy::iter_over_hash_type,
            reason = "Capture records are sorted by emitted identity before closure emission."
        )]
        for &name in &class_plan.unmangled_required {
            if let Some(index) = self.physical_closure_index(name)? {
                captures.push((name, index));
            }
        }
        if is_generic {
            let type_params = self.required_name_id(".type_params");
            captures.push((type_params, self.closure_index_id(type_params)?));
            body_free_aliases.push((type_params, type_params));
        }
        captures.sort_unstable_by(|(left, _), (right, _)| {
            self.resolve_name(*left).cmp(self.resolve_name(*right))
        });
        captures.dedup_by_key(|(name, _)| *name);
        let (freevars, closure_indices): (Vec<_>, Vec<_>) = captures.into_iter().unzip();
        let qualified_name = self
            .unit
            .generic_target_qualified_name
            .clone()
            .unwrap_or_else(|| self.child_qualified_name(definition.name.as_str()));
        let first_line_number =
            self.line_number(u32::from(if decorators == DefinitionDecorators::Compile {
                definition
                    .decorator_list
                    .first()
                    .map_or(definition.range.start(), |decorator| {
                        decorator.expression.range().start()
                    })
            } else {
                definition.range.start()
            }));
        let class_scope_is_nested = self.child_function_is_nested();
        let mut comprehension_targets = Vec::with_capacity(class_plan.comprehension_targets.len());
        for &(name, requires_free) in &class_plan.comprehension_targets {
            if (!requires_free
                || (body_free_aliases.iter().any(|(alias, _)| *alias == name)
                    && super::scope::legacy_class_comprehension_keeps_free_name(
                        self.resolve_name(name),
                        self.context.python_version,
                    )))
                && !comprehension_targets.contains(&name)
            {
                comprehension_targets.push(name);
            }
        }
        let mut child = Self::class(
            Rc::clone(&self.context),
            definition.name.as_str(),
            qualified_name,
            first_line_number,
            scope_id,
            class_plan.globals,
            class_plan.nonlocals,
            &freevars,
            &body_free_aliases,
            &comprehension_targets,
            &class_plan.comprehension_cells,
            class_plan.bound_names,
            class_plan.needs_class_closure,
            class_plan.needs_classdict,
            class_plan.has_conditional_annotations,
            self.unit.flags & SUPPORTED_FUTURE_FLAGS,
        );
        child.unit.class_scope_is_nested = class_scope_is_nested;
        child
            .symbols
            .type_parameter_names
            .clone_from(&self.symbols.type_parameter_names);
        child
            .symbols
            .imported_scope_names
            .clone_from(&self.symbols.imported_scope_names);
        let child = child.compile_class_body(&definition.body)?;

        let definition_location = self.definition_location(
            definition.range,
            definition.name.range(),
            TokenKind::Class,
            false,
        );
        self.assembler.set_location(definition_location);
        if self.context.python_version == PythonVersion::Py312 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        self.emit(LOAD_BUILD_CLASS, 0, 1)?;
        if self.context.python_version != PythonVersion::Py312 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        self.emit_function_with_closure_indices(child, &closure_indices)?;
        self.context
            .record_constant_string_origin(definition.name.as_str(), true);
        let name = self.add_constant(Constant::String(definition.name.as_str().to_string()))?;
        self.emit(LOAD_CONST, name, 1)?;
        if is_generic {
            self.load_name(".type_params")?;
            self.emit_unary_intrinsic(UnaryIntrinsic::SUBSCRIPT_GENERIC)?;
            self.store_name(".generic_base")?;
        }

        let arguments = definition.arguments.as_deref();
        let has_unpacking = arguments.is_some_and(|arguments| {
            arguments.args.iter().any(Expr::is_starred_expr)
                || arguments
                    .keywords
                    .iter()
                    .any(|keyword| keyword.arg.is_none())
        });
        let has_starred_arguments =
            arguments.is_some_and(|arguments| arguments.args.iter().any(Expr::is_starred_expr));
        let large_call = arguments.is_some_and(|arguments| {
            arguments.args.len() + arguments.keywords.len() * 2 > CPYTHON_STACK_USE_GUIDELINE
        });
        let large_positional_tuple =
            arguments.map_or(0, |arguments| arguments.args.len()) + 2 + usize::from(is_generic)
                > CPYTHON_STACK_USE_GUIDELINE;
        let argument_count = arguments.map_or(0, |arguments| {
            arguments.args.len() + arguments.keywords.len()
        }) + usize::from(is_generic);
        if has_unpacking || large_call {
            // Extended class calls include the body function and class name in their positional
            // tuple. Large tuples accumulate bases one at a time to bound stack depth.
            if large_positional_tuple {
                self.assembler.set_location(definition_location);
                self.emit_build(BUILD_LIST, 2)?;
                if let Some(arguments) = arguments {
                    for argument in &arguments.args {
                        let (argument, opcode) = if let Expr::Starred(starred) = argument {
                            (starred.value.as_ref(), LIST_EXTEND)
                        } else {
                            (argument, LIST_APPEND)
                        };
                        self.compile_expression(argument)?;
                        self.assembler.set_location(definition_location);
                        self.emit(opcode, 1, -1)?;
                    }
                }
                if is_generic {
                    self.load_name(".generic_base")?;
                    self.assembler.set_location(definition_location);
                    self.emit(LIST_APPEND, 1, -1)?;
                }
                self.emit_unary_intrinsic(UnaryIntrinsic::LIST_TO_TUPLE)?;
            } else if has_starred_arguments {
                let leading_count = arguments.map_or(0, |arguments| {
                    arguments
                        .args
                        .iter()
                        .take_while(|argument| !argument.is_starred_expr())
                        .count()
                });
                if let Some(arguments) = arguments {
                    for argument in arguments.args.iter().take(leading_count) {
                        self.compile_expression(argument)?;
                    }
                }
                self.emit_build(BUILD_LIST, leading_count + 2)?;
                for argument in arguments
                    .expect("starred class arguments require an argument list")
                    .args
                    .iter()
                    .skip(leading_count)
                {
                    if let Expr::Starred(starred) = argument {
                        self.compile_expression(&starred.value)?;
                        self.emit(LIST_EXTEND, 1, -1)?;
                    } else {
                        self.compile_expression(argument)?;
                        self.emit(LIST_APPEND, 1, -1)?;
                    }
                }
                if is_generic {
                    self.load_name(".generic_base")?;
                    self.emit(LIST_APPEND, 1, -1)?;
                }
                self.emit_unary_intrinsic(UnaryIntrinsic::LIST_TO_TUPLE)?;
            } else {
                if let Some(arguments) = arguments {
                    for argument in &arguments.args {
                        self.compile_expression(argument)?;
                    }
                }
                if is_generic {
                    self.load_name(".generic_base")?;
                }
                let positional_count =
                    arguments.map_or(0, |arguments| arguments.args.len()) + usize::from(is_generic);
                self.emit_build(BUILD_TUPLE, positional_count + 2)?;
            }
            if let Some(arguments) = arguments.filter(|arguments| !arguments.keywords.is_empty()) {
                let mut have_dict = false;
                let mut named_start = 0;
                for (index, keyword) in arguments.keywords.iter().enumerate() {
                    if keyword.arg.is_some() {
                        continue;
                    }
                    if named_start < index {
                        self.compile_class_keyword_map(
                            &arguments.keywords[named_start..index],
                            definition_location,
                        )?;
                        if have_dict {
                            self.emit(DICT_MERGE, 1, -1)?;
                        }
                        have_dict = true;
                    }
                    if !have_dict {
                        self.emit(BUILD_MAP, 0, 1)?;
                        have_dict = true;
                    }
                    self.compile_expression(&keyword.value)?;
                    self.emit(DICT_MERGE, 1, -1)?;
                    named_start = index + 1;
                }
                if named_start < arguments.keywords.len() {
                    self.compile_class_keyword_map(
                        &arguments.keywords[named_start..],
                        definition_location,
                    )?;
                    if have_dict {
                        self.emit(DICT_MERGE, 1, -1)?;
                    }
                }
            } else if self.has_deferred_annotations() {
                self.emit(PUSH_NULL, 0, 1)?;
            }
            self.assembler.set_location(definition_location);
            if self.has_deferred_annotations() {
                self.emit(CALL_FUNCTION_EX, 0, -3)?;
            } else {
                let has_keywords =
                    arguments.is_some_and(|arguments| !arguments.keywords.is_empty());
                self.emit(
                    CALL_FUNCTION_EX,
                    u32::from(has_keywords),
                    -2 - i32::from(has_keywords),
                )?;
            }
        } else {
            if let Some(arguments) = arguments {
                for argument in &arguments.args {
                    self.compile_expression(argument)?;
                }
            }
            if is_generic {
                self.load_name(".generic_base")?;
            }
            if let Some(arguments) = arguments {
                for keyword in &arguments.keywords {
                    self.compile_expression(&keyword.value)?;
                }
            }
            self.assembler.set_location(definition_location);
            if let Some(arguments) = arguments.filter(|arguments| !arguments.keywords.is_empty()) {
                let keyword_names = arguments
                    .keywords
                    .iter()
                    .map(|keyword| {
                        Constant::String(keyword.arg.as_ref().unwrap().as_str().to_string())
                    })
                    .collect();
                let names = self.add_interned_string_tuple(keyword_names)?;
                let count = to_u32(argument_count + 2, "class argument count")?;
                if self.context.python_version == PythonVersion::Py312 {
                    self.emit(super::KW_NAMES, names, 0)?;
                    self.emit(CALL, count, -i32::try_from(argument_count).unwrap() - 3)?;
                } else {
                    self.emit(LOAD_CONST, names, 1)?;
                    self.emit(CALL_KW, count, -i32::try_from(argument_count).unwrap() - 4)?;
                }
            } else {
                let count = to_u32(argument_count + 2, "class argument count")?;
                self.emit(CALL, count, -i32::try_from(argument_count).unwrap() - 3)?;
            }
        }
        if decorators == DefinitionDecorators::Compile {
            for decorator in definition.decorator_list.iter().rev() {
                self.assembler
                    .set_location(self.source_location(decorator.expression.range()));
                self.emit(CALL, 0, -1)?;
            }
        }
        self.assembler.set_location(definition_location);
        if self.unit.generic_target_qualified_name.is_some() {
            Ok(())
        } else {
            self.store_name(definition.name.as_str())
        }
    }

    fn compile_class_keyword_map(
        &mut self,
        keywords: &[Keyword],
        definition_location: SourceLocation,
    ) -> Result<(), CompileError> {
        debug_assert!(!keywords.is_empty() && keywords.iter().all(|keyword| keyword.arg.is_some()));
        let large_map = keywords.len() * 2 > CPYTHON_STACK_USE_GUIDELINE;
        if !self.has_deferred_annotations() && !large_map && keywords.len() > 1 {
            for keyword in keywords {
                self.compile_expression(&keyword.value)?;
            }
            self.assembler.set_location(definition_location);
            let names = keywords
                .iter()
                .map(|keyword| Constant::String(keyword.arg.as_ref().unwrap().as_str().to_string()))
                .collect();
            let names = self.add_interned_string_tuple(names)?;
            self.emit(LOAD_CONST, names, 1)?;
            let count = to_u32(keywords.len(), "class keyword count")?;
            return self.emit(BUILD_CONST_KEY_MAP, count, -i32::try_from(count).unwrap());
        }
        if large_map {
            self.assembler.set_location(SourceLocation::NONE);
            self.emit(BUILD_MAP, 0, 1)?;
        }
        for keyword in keywords {
            let name = keyword.arg.as_ref().unwrap();
            self.context
                .record_constant_string_origin(name.as_str(), true);
            let key = self.add_constant(Constant::String(name.as_str().to_string()))?;
            if large_map {
                self.assembler.set_location(definition_location);
            }
            self.emit(LOAD_CONST, key, 1)?;
            self.compile_expression(&keyword.value)?;
            if large_map {
                self.assembler.set_location(SourceLocation::NONE);
                self.emit(MAP_ADD, 1, -2)?;
            }
        }
        self.assembler.set_location(definition_location);
        if large_map {
            Ok(())
        } else {
            self.emit_build_map(keywords.len())
        }
    }

    fn compile_type_parameters(
        &mut self,
        type_params: &ruff_python_ast::TypeParams,
    ) -> Result<(), CompileError> {
        // During symbol analysis, a generic class adds each declared private name to its
        // mangling restriction before visiting that parameter's bound. An earlier bound sees
        // a later private spelling as a global, although code generation uses the completed
        // restriction and emits its mangled spelling.
        if self.unit.mangled_names.is_some() {
            self.unit.pending_private_type_parameters = type_params
                .iter()
                .map(|parameter| parameter.name().as_str())
                .filter(|name| self.mangled_name(name) != *name)
                .map(|name| self.required_name_id(name))
                .collect();
        }
        for parameter in type_params {
            let name = parameter.name().as_str();
            self.unit
                .pending_private_type_parameters
                .remove(&self.required_name_id(name));
            self.assembler
                .set_location(self.source_location(parameter.range()));
            self.context.record_constant_string_origin(name, true);
            self.load_string_constant(name)?;
            match parameter {
                TypeParam::TypeVar(parameter) => {
                    if let Some(bound) = &parameter.bound {
                        let setup_range = if self.context.python_version == PythonVersion::Py312 {
                            parameter.range
                        } else {
                            bound.range()
                        };
                        self.compile_type_parameter_thunk(name, bound, setup_range, true, false)?;
                        self.assembler
                            .set_location(self.source_location(parameter.range()));
                        let intrinsic = if matches!(bound.as_ref(), Expr::Tuple(_)) {
                            BinaryIntrinsic::TYPEVAR_WITH_CONSTRAINTS
                        } else {
                            BinaryIntrinsic::TYPEVAR_WITH_BOUND
                        };
                        self.emit_binary_intrinsic(intrinsic)?;
                    } else {
                        self.emit_unary_intrinsic(UnaryIntrinsic::TYPEVAR)?;
                    }
                }
                TypeParam::ParamSpec(_) => self.emit_unary_intrinsic(UnaryIntrinsic::PARAMSPEC)?,
                TypeParam::TypeVarTuple(_) => {
                    self.emit_unary_intrinsic(UnaryIntrinsic::TYPEVARTUPLE)?;
                }
            }
            if let Some(default) = parameter.default() {
                self.compile_type_parameter_thunk(name, default, default.range(), true, false)?;
                self.assembler
                    .set_location(self.source_location(parameter.range()));
                self.emit_binary_intrinsic(BinaryIntrinsic::SET_TYPEPARAM_DEFAULT)?;
            }
            self.emit(COPY, 1, 1)?;
            self.store_name(name)?;
        }
        if let Some(first) = type_params.first() {
            self.assembler
                .set_location(self.source_location(first.range()));
        }
        self.emit_build(BUILD_TUPLE, type_params.len())
    }

    fn compile_type_parameter_thunk(
        &mut self,
        name: &str,
        expression: &Expr,
        setup_range: ruff_text_size::TextRange,
        nested: bool,
        type_alias: bool,
    ) -> Result<(), CompileError> {
        let mut references = ReferenceCollector::default();
        references.visit_expr(expression);
        let generic_class_header =
            self.unit.mangled_names.is_some() && self.unit.generic_target_qualified_name.is_some();
        let symbol_name = |name: &str| {
            if generic_class_header {
                self.symbol_analysis_name(name)
            } else {
                name.to_owned()
            }
        };
        let can_see_class_scope = matches!(self.unit.scope, Scope::Class { .. })
            || self.unit.annotation_classdict_index.is_some();
        let mut local_names = if self.has_deferred_annotations() {
            vec![".format".to_string()]
        } else {
            Vec::new()
        };
        let inlined_cellvars = if can_see_class_scope {
            FxHashSet::default()
        } else {
            // The first iterable runs before its comprehension reserves fast locals. Reuse
            // the function-scope collector so nested iterables retain that allocation order.
            local_names.extend(inlined_comprehension_local_names_in_expression(expression));
            inlined_comprehension_cell_names_in_expression(expression)
        };
        let mut seen = FxHashSet::default();
        local_names.retain(|name| seen.insert(name.clone()));
        let mut locals = self.name_ids(local_names.iter().map(|name| symbol_name(name)));
        if generic_class_header {
            let mut seen = FxHashSet::default();
            locals.retain(|name| seen.insert(*name));
        }
        let required = if generic_class_header {
            expression_required_names_with_mangler(expression, &symbol_name)
        } else {
            expression_required_names(expression)
        };
        let required_names = self.name_ids(required.iter().map(|name| symbol_name(name)));
        let nested_names = if generic_class_header {
            annotation_nested_required_names_with_mangler(expression, &symbol_name)
        } else {
            annotation_nested_required_names(expression)
        };
        let nested_requirements =
            self.name_id_set(nested_names.iter().map(|name| symbol_name(name)));
        let outside_requirements = self.name_id_set(
            expression_requirements_outside_comprehensions(expression)
                .iter()
                .map(|name| symbol_name(name)),
        );
        let mut freevars: Vec<_> = required_names
            .iter()
            .filter(|name| !locals.contains(name) || outside_requirements.contains(name))
            .filter(|name| {
                if generic_class_header {
                    self.physical_closure_index(**name).ok().flatten().is_some()
                } else {
                    !self.unit.pending_private_type_parameters.contains(*name)
                        && (self.is_type_parameter_id(**name)
                            || self.can_provide_closure_id(**name))
                }
            })
            .filter(|name| {
                !(if generic_class_header {
                    self.class_scope_shadows_unmangled_annotation_name(**name)
                } else {
                    self.class_scope_shadows_annotation_name(**name)
                }) || nested_requirements.contains(*name)
            })
            .copied()
            .collect();
        if generic_class_header {
            freevars.sort_unstable_by(|left, right| {
                self.resolve_name(*left).cmp(self.resolve_name(*right))
            });
            freevars.dedup();
        }
        if can_see_class_scope {
            let classdict = self.required_name_id("__classdict__");
            self.insert_lexical_name(&mut freevars, classdict);
        }
        if !generic_class_header {
            freevars.sort_unstable_by_key(|name| self.mangled_name(self.resolve_name(*name)));
        }
        let freevar_set: FxHashSet<_> = freevars.iter().copied().collect();
        let globals = references
            .references
            .iter()
            .map(|name| self.required_name_id(&symbol_name(name)))
            .filter(|name| {
                !freevar_set.contains(name)
                    || if generic_class_header {
                        self.class_scope_shadows_unmangled_annotation_name(*name)
                    } else {
                        self.class_scope_shadows_annotation_name(*name)
                    }
            })
            .collect();
        let cellvars = self.name_id_set(inlined_cellvars.iter().map(|name| symbol_name(name)));
        let header_local_storage = (generic_class_header
            && !can_see_class_scope
            && !self.unit.pending_private_type_parameters.is_empty())
        .then(|| {
            let prefix = if self.has_deferred_annotations() {
                vec![self.required_name_id(".format")]
            } else {
                Vec::new()
            };
            header_expression_storage(
                &ruff_python_ast::Parameters::default(),
                expression,
                &symbol_name,
                &|name| self.mangled_name(name),
                &self.context.scopes.names,
                &prefix,
            )
        });
        let plan = LoweringFunctionPlan {
            header_local_storage,
            locals,
            globals,
            freevars,
            inlined_comprehension_cellvars: cellvars.clone(),
            cellvars,
            ..LoweringFunctionPlan::default()
        };
        let closure_names = plan.freevars.clone();
        // Definitions inside an annotation scope are qualified against that
        // scope's parent. Generic-parameter wrappers are themselves annotation
        // scopes even though this compiler represents them as functions.
        let child_qualified_name_parent = if self.unit.generic_target_qualified_name.is_some() {
            self.unit.qualified_name.clone()
        } else {
            match self.unit.scope {
                Scope::Module => String::new(),
                Scope::Class { .. } => self.unit.qualified_name.clone(),
                Scope::Function { .. } => format!("{}.<locals>", self.unit.qualified_name),
            }
        };
        let mut child = Self::function(
            Rc::clone(&self.context),
            name,
            if self.unit.generic_target_qualified_name.is_some() {
                self.unit.qualified_name.rsplit_once('.').map_or_else(
                    || name.to_string(),
                    |(parent, _)| format!("{parent}.{name}"),
                )
            } else {
                self.annotation_child_qualified_name(name)
            },
            self.line_number(u32::from(
                if self.context.python_version == PythonVersion::Py312 && !type_alias {
                    expression.range().start()
                } else {
                    setup_range.start()
                },
            )),
            &plan,
            self.unit.scope_id,
            u32::from(self.has_deferred_annotations()),
            u32::from(self.has_deferred_annotations()),
            0,
            (if nested { CO_NESTED } else { 0 }) | (self.unit.flags & SUPPORTED_FUTURE_FLAGS),
        )?;
        child.inherit_lexical_context(self);
        if generic_class_header {
            child.unit.header_symbol_names_are_final = true;
            child.unit.free_names_are_final = true;
        }
        // Before 3.14, type alias values use ordinary function scopes, so
        // definitions within them include the alias and `<locals>`.
        if !type_alias || self.has_deferred_annotations() {
            child.unit.child_qualified_name_parent = Some(child_qualified_name_parent);
        }
        if can_see_class_scope {
            child.unit.annotation_classdict_index = Some(child.closure_index("__classdict__")?);
        }
        child.emit_function_prologue()?;
        if type_alias && !self.has_deferred_annotations() {
            child.add_constant(Constant::None)?;
        }
        child
            .assembler
            .set_location(child.source_location(setup_range));
        if self.has_deferred_annotations() {
            child.emit_annotation_format_guard(".format")?;
        }
        if let Expr::Starred(starred) = expression {
            child.compile_expression(&starred.value)?;
            child
                .assembler
                .set_location(child.source_location(setup_range));
            child.emit(UNPACK_SEQUENCE, 1, 0)?;
        } else {
            child.compile_expression(expression)?;
        }
        child
            .assembler
            .set_location(child.source_location(setup_range));
        child.emit(RETURN_VALUE, 0, -1)?;
        let child = child.finish_inner(false)?;

        self.assembler
            .set_location(self.source_location(setup_range));
        let mut flags = 0;
        if self.has_deferred_annotations() {
            let defaults = self.add_constant(Constant::Tuple(vec![Constant::Int(1)]))?;
            self.emit(LOAD_CONST, defaults, 1)?;
            flags |= FunctionAttribute::DEFAULTS.argument();
        }
        if !closure_names.is_empty() {
            if generic_class_header {
                let indices = closure_names
                    .iter()
                    .map(|name| {
                        self.physical_closure_index(*name)?.ok_or_else(|| {
                            CompileError::Internal(format!(
                                "missing header closure variable `{}`",
                                self.resolve_name(*name)
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.emit_closure_indices(&indices)?;
            } else {
                self.emit_closure_tuple(&closure_names)?;
            }
        }
        let code = self.add_constant(Constant::Code(Box::new(child)))?;
        self.emit(LOAD_CONST, code, 1)?;
        if !closure_names.is_empty() {
            flags |= FunctionAttribute::CLOSURE.argument();
        }
        self.emit_make_function(flags)
    }

    fn emit_annotation_format_guard(&mut self, parameter: &str) -> Result<(), CompileError> {
        self.load_name(parameter)?;
        self.add_constant(Constant::Int(2))?;
        self.emit(LOAD_SMALL_INT, 2, 1)?;
        self.emit_comparison(ComparisonOperation::GT, ComparisonMode::Value)?;
        let supported = self.assembler.label();
        self.emit_jump_forward(POP_JUMP_IF_FALSE, supported, -1)?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_common_constant(CommonConstant::NOTIMPLEMENTEDERROR)?;
        self.emit(RAISE_VARARGS, 1, -1)?;
        self.assembler.mark(supported);
        self.set_depth(0);
        Ok(())
    }

    /// Before Python 3.14, annotations are evaluated in the defining scope and
    /// passed to `MAKE_FUNCTION` as alternating name/value entries.
    fn compile_eager_function_annotations(
        &mut self,
        definition: &StmtFunctionDef,
        location: SourceLocation,
    ) -> Result<(), CompileError> {
        let annotations = function_annotations(definition, self.context.python_version);
        self.compile_function_annotation_entries(&annotations, location)?;
        if !annotations.is_empty() {
            self.assembler.set_location(location);
            self.emit_build(BUILD_TUPLE, annotations.len() * 2)?;
        }
        Ok(())
    }

    fn compile_function_annotation_entries(
        &mut self,
        annotations: &[(&str, &Expr)],
        location: SourceLocation,
    ) -> Result<(), CompileError> {
        for (name, annotation) in annotations {
            self.assembler.set_location(location);
            let mangled = self.mangled_name(name);
            self.context
                .record_identifier_constant_origin(name, &mangled);
            self.load_string_constant(&mangled)?;
            if self.unit.flags & CO_FUTURE_ANNOTATIONS != 0 {
                self.assembler
                    .set_location(self.source_location(annotation.range()));
                self.load_string_constant(&unparse_annotation(annotation, self.context.source))?;
            } else if let Expr::Starred(starred) = annotation {
                self.compile_expression(&starred.value)?;
                self.assembler.set_location(location);
                self.emit(UNPACK_SEQUENCE, 1, 0)?;
            } else {
                self.compile_expression(annotation)?;
            }
        }
        Ok(())
    }

    fn compile_function_annotations(
        &self,
        definition: &StmtFunctionDef,
        function_plan: &LoweringFunctionPlan,
    ) -> Result<Option<(CodeObject, Vec<NameId>)>, CompileError> {
        let annotations = function_annotations(definition, self.context.python_version);
        if annotations.is_empty() {
            return Ok(None);
        }

        let nested_requirements = self.name_id_set(
            annotations
                .iter()
                .flat_map(|(_, annotation)| annotation_nested_required_names(annotation)),
        );
        let mut freevars = function_plan.annotation_freevars.clone();
        freevars.retain(|name| {
            !self.class_scope_shadows_annotation_name(*name) || nested_requirements.contains(name)
        });
        if self.unit.flags & CO_FUTURE_ANNOTATIONS == 0 {
            for name in function_plan
                .annotation_references
                .iter()
                .filter(|name| {
                    self.is_type_parameter_id(**name)
                        && (!self.class_scope_shadows_annotation_name(**name)
                            || nested_requirements.contains(*name))
                })
                .copied()
            {
                self.insert_lexical_name(&mut freevars, name);
            }
        }
        let can_see_class_scope = self.unit.flags & CO_FUTURE_ANNOTATIONS == 0
            && (matches!(self.unit.scope, Scope::Class { .. })
                || self.unit.annotation_classdict_index.is_some());
        if can_see_class_scope {
            let classdict = self.required_name_id("__classdict__");
            self.insert_lexical_name(&mut freevars, classdict);
        }
        // Private captures sort by their emitted spelling, alongside synthetic class cells.
        freevars.sort_unstable_by_key(|name| self.mangled_name(self.resolve_name(*name)));
        let mut local_names = vec![".format".to_string()];
        let mut references = ReferenceCollector::default();
        let mut inlined_cellvars = FxHashSet::default();
        if self.unit.flags & CO_FUTURE_ANNOTATIONS == 0 && !can_see_class_scope {
            // Reserve comprehension locals before assigning the annotation thunk's free slots.
            for (_, annotation) in &annotations {
                if !freevars.is_empty() {
                    references.visit_expr(annotation);
                }
                local_names.extend(inlined_comprehension_local_names_in_expression(annotation));
                inlined_cellvars.extend(inlined_comprehension_cell_names_in_expression(annotation));
            }
        }
        let mut seen = FxHashSet::default();
        local_names.retain(|name| seen.insert(name.clone()));
        let mut annotation_globals: FxHashSet<_> = references
            .references
            .iter()
            .map(|name| self.required_name_id(name))
            .filter(|name| !freevars.contains(name))
            .collect();
        annotation_globals.extend(
            function_plan
                .annotation_references
                .iter()
                .copied()
                .filter(|name| self.class_scope_shadows_annotation_name(*name)),
        );
        let cellvars = self.name_id_set(inlined_cellvars);
        let plan = LoweringFunctionPlan {
            locals: self.name_ids(&local_names),
            freevars,
            inlined_comprehension_cellvars: cellvars.clone(),
            cellvars,
            ..LoweringFunctionPlan::default()
        };
        let parameter_flags = (if self.child_function_is_nested() {
            CO_NESTED
        } else {
            0
        }) | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        let annotation_qualified_name = if self.context.python_version == PythonVersion::Py315 {
            if self.unit.generic_target_qualified_name.is_some() {
                // A generic wrapper is an annotation scope. CPython skips it,
                // including an early return when its parent is the module.
                self.unit.qualified_name.rsplit_once('.').map_or_else(
                    || "__annotate__".to_string(),
                    |(parent, _)| format!("{parent}.{}.__annotate__", definition.name),
                )
            } else {
                format!(
                    "{}.__annotate__",
                    self.annotation_child_qualified_name(definition.name.as_str())
                )
            }
        } else if self.unit.generic_target_qualified_name.is_some() {
            // A global definition resets its own name, but its annotation scope still
            // belongs to the generic wrapper's enclosing scope.
            self.unit.qualified_name.rsplit_once('.').map_or_else(
                || "__annotate__".to_string(),
                |(parent, _)| format!("{parent}.__annotate__"),
            )
        } else {
            self.child_qualified_name("__annotate__")
        };
        let mut child = Self::function(
            Rc::clone(&self.context),
            "__annotate__",
            annotation_qualified_name,
            self.line_number(u32::from(definition.name.range().start())),
            &plan,
            self.unit.scope_id,
            1,
            1,
            0,
            parameter_flags,
        )?;
        if self.context.python_version == PythonVersion::Py315
            || self.unit.generic_target_qualified_name.is_some()
        {
            // Lambdas and generators inside annotations are qualified against
            // the enclosing scope, independently of the annotation owner's name.
            child.unit.child_qualified_name_parent =
                Some(if self.unit.generic_target_qualified_name.is_some() {
                    self.unit.qualified_name.clone()
                } else {
                    self.annotation_child_qualified_name("__annotate__")
                        .strip_suffix(".__annotate__")
                        .unwrap_or("")
                        .to_string()
                });
        }
        child.inherit_lexical_context(self);
        child.unit.annotation_thunk = true;
        if can_see_class_scope {
            child.unit.annotation_classdict_index = Some(child.closure_index("__classdict__")?);
        }
        child.emit_function_prologue()?;
        let location = self.definition_location(
            definition.range,
            definition.name.range(),
            TokenKind::Def,
            definition.is_async,
        );
        child.assembler.set_location(location);

        child.emit_annotation_format_guard(".format")?;
        // The format guard reads its synthetic parameter; annotation expressions use their own globals.
        if let Scope::Function { globals, .. } = &mut child.unit.scope {
            globals.extend(annotation_globals);
        }

        child.compile_function_annotation_entries(&annotations, location)?;
        child.assembler.set_location(location);
        let count = to_u32(annotations.len(), "annotation count")?;
        child.emit(BUILD_MAP, count, 1 - i32::try_from(count).unwrap() * 2)?;
        child.emit(RETURN_VALUE, 0, -1)?;
        let child = child.finish_inner(false)?;
        Ok(Some((child, plan.freevars)))
    }

    pub(super) fn compile_class_annotations(
        &self,
        body: &[Stmt],
    ) -> Result<Option<(CodeObject, Vec<NameId>)>, CompileError> {
        let mut annotations = Vec::new();
        collect_simple_annotations(body, &mut annotations);
        if annotations.is_empty() {
            return Ok(None);
        }
        let mut conditional_annotations = Vec::new();
        collect_conditional_simple_annotations(body, &mut conditional_annotations);
        let mut conditional_annotations = VecDeque::from(conditional_annotations);

        // Annotation thunks must pass enclosing cells through to nested lambdas
        // and generators, even when the annotation itself never loads the name.
        let mut references = FxHashSet::default();
        for annotation in &annotations {
            references.extend(expression_required_names(&annotation.annotation));
        }
        let nested_requirements = self.name_id_set(
            annotations
                .iter()
                .flat_map(|annotation| annotation_nested_required_names(&annotation.annotation)),
        );
        let Scope::Class { free_indices, .. } = &self.unit.scope else {
            unreachable!("class annotations are compiled in a class scope");
        };
        let available_freevars: FxHashSet<_> = free_indices.keys().copied().collect();
        let reference_ids = self.name_id_set(&references);
        let mut freevars = Vec::new();
        for name in reference_ids
            .iter()
            .filter(|name| {
                available_freevars.contains(*name)
                    && (!self.class_scope_shadows_annotation_name(**name)
                        || nested_requirements.contains(*name))
            })
            .copied()
        {
            self.insert_lexical_name(&mut freevars, name);
        }
        let dunder_class = self.required_name_id("__class__");
        if reference_ids.contains(&dunder_class)
            && self.is_cell_id(dunder_class)
            && (!self.class_scope_shadows_annotation_name(dunder_class)
                || nested_requirements.contains(&dunder_class))
        {
            self.insert_lexical_name(&mut freevars, dunder_class);
        }
        let classdict = self.required_name_id("__classdict__");
        self.insert_lexical_name(&mut freevars, classdict);
        // Preserve closure lookup IDs while matching the emitted private-name order.
        freevars.sort_unstable_by_key(|name| self.mangled_name(self.resolve_name(*name)));
        if !conditional_annotations.is_empty() {
            let conditional = self.required_name_id("__conditional_annotations__");
            freevars.push(conditional);
        }
        let freevar_set: FxHashSet<_> = freevars.iter().copied().collect();
        let globals = reference_ids
            .into_iter()
            .filter(|name| {
                !freevar_set.contains(name) || self.class_scope_shadows_annotation_name(*name)
            })
            .collect();
        let closure_names = freevars.clone();
        let plan = LoweringFunctionPlan {
            locals: vec![self.required_name_id(".format")],
            globals,
            freevars,
            ..LoweringFunctionPlan::default()
        };
        let parameter_flags =
            (if self.child_function_is_nested() || !self.symbols.free_names.is_empty() {
                CO_NESTED
            } else {
                0
            }) | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        let mut child = Self::function(
            Rc::clone(&self.context),
            "__annotate__",
            self.child_qualified_name("__annotate__"),
            self.unit.first_line_number,
            &plan,
            self.unit.scope_id,
            1,
            1,
            0,
            parameter_flags,
        )?;
        child.inherit_lexical_context(self);
        child.unit.annotation_thunk = true;
        child.unit.annotation_classdict_index = Some(child.closure_index("__classdict__")?);
        child.emit_function_prologue()?;
        let line = i32::try_from(self.unit.first_line_number).unwrap_or(i32::MAX);
        let setup_location = SourceLocation::new(line, line, 0, 0);
        child.assembler.set_location(setup_location);
        child.emit_annotation_format_guard(".format")?;
        child.emit(BUILD_MAP, 0, 1)?;

        let mut conditional_index = 0_u32;
        for annotation in annotations {
            let annotation_conditional = conditional_annotations
                .front()
                .is_some_and(|candidate| std::ptr::eq(*candidate, annotation));
            let skip = if annotation_conditional {
                conditional_annotations.pop_front();
                child
                    .assembler
                    .set_location(child.source_location(annotation.range));
                child.emit_unsigned_constant(conditional_index)?;
                conditional_index += 1;
                let conditional = child.closure_index("__conditional_annotations__")?;
                child.emit(LOAD_DEREF, conditional, 1)?;
                child.emit(CONTAINS_OP, 0, -1)?;
                let skip = child.assembler.label();
                child.emit_jump_forward(POP_JUMP_IF_FALSE, skip, -1)?;
                child.emit(NOT_TAKEN, 0, 0)?;
                Some(skip)
            } else {
                None
            };
            let Expr::Name(target) = annotation.target.as_ref() else {
                return Err(unsupported("simple annotation target"));
            };
            child
                .assembler
                .set_location(child.source_location(annotation.annotation.range()));
            child.compile_expression(&annotation.annotation)?;
            child
                .assembler
                .set_location(child.source_location(annotation.range));
            child.emit(COPY, 2, 1)?;
            let mangled = child.mangled_name(target.id.as_str());
            child
                .context
                .record_identifier_constant_origin(target.id.as_str(), &mangled);
            child.load_string_constant(&mangled)?;
            child.assembler.set_location(setup_location);
            child.emit(STORE_SUBSCR, 0, -3)?;
            if let Some(skip) = skip {
                child.assembler.mark(skip);
                child.set_depth(1);
            }
        }
        debug_assert!(conditional_annotations.is_empty());
        child.assembler.set_location(setup_location);
        child.emit(RETURN_VALUE, 0, -1)?;
        let child = child.finish_inner(false)?;
        Ok(Some((child, closure_names)))
    }

    pub(super) fn compile_module_annotations(
        &self,
        body: &[Stmt],
    ) -> Result<CodeObject, CompileError> {
        let mut annotations = Vec::new();
        collect_simple_annotations(body, &mut annotations);
        let first = annotations.first().ok_or_else(|| {
            CompileError::Internal("module annotation thunk has no annotations".to_string())
        })?;
        let mut local_names = vec![".format".to_string()];
        let mut inlined_cellvars = FxHashSet::default();
        for annotation in &annotations {
            local_names.extend(inlined_comprehension_local_names_in_expression(
                &annotation.annotation,
            ));
            inlined_cellvars.extend(inlined_comprehension_cell_names_in_expression(
                &annotation.annotation,
            ));
        }
        let mut seen = FxHashSet::default();
        local_names.retain(|name| seen.insert(name.clone()));
        let cellvars = self.name_id_set(inlined_cellvars);
        let plan = LoweringFunctionPlan {
            locals: self.name_ids(&local_names),
            inlined_comprehension_cellvars: cellvars.clone(),
            cellvars,
            ..LoweringFunctionPlan::default()
        };
        let mut child = Self::function(
            Rc::clone(&self.context),
            "__annotate__",
            "__annotate__".to_string(),
            self.line_number(u32::from(
                body.first().map_or(first.range, Ranged::range).start(),
            )),
            &plan,
            self.unit.scope_id,
            1,
            1,
            0,
            self.unit.flags & SUPPORTED_FUTURE_FLAGS,
        )?;
        child.inherit_lexical_context(self);
        child.unit.annotation_thunk = true;
        child.emit_function_prologue()?;
        let setup_location =
            self.physical_source_location(body.first().map_or(first.range, Ranged::range));
        child.assembler.set_location(setup_location);
        child.emit_annotation_format_guard(".format")?;
        child.emit(BUILD_MAP, 0, 1)?;

        for (index, annotation) in annotations.iter().enumerate() {
            let Expr::Name(target) = annotation.target.as_ref() else {
                return Err(unsupported("simple annotation target"));
            };
            let location = self.source_location(annotation.range);
            child.assembler.set_location(location);
            let index = to_u32(index, "module annotation index")?;
            child.emit_unsigned_constant(index)?;
            child.load_name("__conditional_annotations__")?;
            child.emit(CONTAINS_OP, 0, -1)?;
            let skip = child.assembler.label();
            child.emit_jump_forward(POP_JUMP_IF_FALSE, skip, -1)?;
            child.emit(NOT_TAKEN, 0, 0)?;
            child.compile_expression(&annotation.annotation)?;
            child.assembler.set_location(location);
            child.emit(COPY, 2, 1)?;
            child
                .context
                .record_constant_string_origin(target.id.as_str(), true);
            child.load_string_constant(target.id.as_str())?;
            child.assembler.set_location(setup_location);
            child.emit(STORE_SUBSCR, 0, -3)?;
            child.assembler.mark(skip);
            child.set_depth(1);
        }
        child.assembler.set_location(setup_location);
        child.emit(RETURN_VALUE, 0, -1)?;
        child.finish_inner(false)
    }
}

/// Collect function annotations in the target's code-generation order.
fn function_annotations(
    definition: &StmtFunctionDef,
    python_version: PythonVersion,
) -> Vec<(&str, &Expr)> {
    let parameters = &definition.parameters;
    let mut annotations = Vec::new();
    let positional_groups = if python_version == PythonVersion::Py315 {
        [&parameters.posonlyargs, &parameters.args]
    } else {
        [&parameters.args, &parameters.posonlyargs]
    };
    for parameter in positional_groups.into_iter().flatten() {
        if let Some(annotation) = parameter.parameter.annotation.as_deref() {
            annotations.push((parameter.name().as_str(), annotation));
        }
    }
    if let Some(parameter) = &parameters.vararg
        && let Some(annotation) = parameter.annotation.as_deref()
    {
        annotations.push((parameter.name.as_str(), annotation));
    }
    for parameter in &parameters.kwonlyargs {
        if let Some(annotation) = parameter.parameter.annotation.as_deref() {
            annotations.push((parameter.name().as_str(), annotation));
        }
    }
    if let Some(parameter) = &parameters.kwarg
        && let Some(annotation) = parameter.annotation.as_deref()
    {
        annotations.push((parameter.name.as_str(), annotation));
    }
    if let Some(annotation) = definition.returns.as_deref() {
        annotations.push(("return", annotation));
    }
    annotations
}

pub(super) fn clean_doc(docstring: &str) -> String {
    let expanded = if docstring.contains('\t') {
        let mut expanded = String::with_capacity(docstring.len());
        let mut column = 0_usize;
        for character in docstring.chars() {
            match character {
                '\t' => {
                    let spaces = 8 - column % 8;
                    expanded.extend(std::iter::repeat_n(' ', spaces));
                    column += spaces;
                }
                '\n' | '\r' => {
                    expanded.push(character);
                    column = 0;
                }
                _ => {
                    expanded.push(character);
                    column += 1;
                }
            }
        }
        std::borrow::Cow::Owned(expanded)
    } else {
        std::borrow::Cow::Borrowed(docstring)
    };

    let mut lines = expanded.split_inclusive('\n');
    let margin = lines
        .clone()
        .skip(1)
        .filter_map(|line| {
            let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
            line.as_bytes()
                .get(indentation)
                .is_some_and(|byte| *byte != b'\n')
                .then_some(indentation)
        })
        .min()
        .unwrap_or(0);

    let Some(first) = lines.next() else {
        return expanded.into_owned();
    };
    let mut cleaned = String::with_capacity(expanded.len());
    cleaned.push_str(first.trim_start_matches(' '));
    for line in lines {
        let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
        cleaned.push_str(&line[indentation.min(margin)..]);
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use crate::code_object::{CodeObject, Constant};
    use crate::target::local_kinds::CO_FAST_FREE;
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    fn find_code<'a>(code: &'a CodeObject, name: &str) -> Option<&'a CodeObject> {
        if code.name == name {
            return Some(code);
        }
        code.constants.iter().find_map(|constant| match constant {
            Constant::Code(child) => find_code(child, name),
            _ => None,
        })
    }

    #[test]
    fn nested_generic_class_private_bounds_capture_their_own_parameters() {
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for (binding, captures_parameter) in [
                ("__T = int", true),
                ("global __T", true),
                ("_Inner__T = int", false),
                ("global _Inner__T", false),
            ] {
                let source =
                    format!("class Outer:\n    {binding}\n    class Inner[__T: __T]: pass\n");
                let module = compile_with_options(
                    &source,
                    "nested_generic_class_private.py",
                    &CompileOptions {
                        python_version,
                        ..CompileOptions::default()
                    },
                )
                .unwrap();
                let bound = find_code(&module.code, "__T").unwrap();
                let has_parameter = bound
                    .locals
                    .iter()
                    .zip(&bound.local_kinds)
                    .any(|(name, kind)| name == "_Inner__T" && kind & CO_FAST_FREE != 0);
                assert_eq!(
                    has_parameter, captures_parameter,
                    "{python_version:?}: {binding}"
                );
            }
        }
    }

    #[test]
    fn legacy_type_aliases_qualify_nested_lambdas() {
        for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
            for source in ["type Alias = lambda: None", "type Alias[T] = lambda: T"] {
                let module = compile_with_options(
                    source,
                    "type_alias.py",
                    &CompileOptions {
                        python_version,
                        ..CompileOptions::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    find_code(&module.code, "<lambda>").unwrap().qualified_name,
                    "Alias.<locals>.<lambda>",
                    "{python_version:?}: {source}",
                );
            }
        }
    }

    #[test]
    fn annotation_parameter_order_matches_selected_python_version() {
        let source =
            "def f(p:int, /, q:str, *args:float, kw:bool, **kwargs:bytes) -> object:\n pass\n";
        for (python_version, expected) in [
            (
                PythonVersion::Py314,
                ["q", "p", "args", "kw", "kwargs", "return"],
            ),
            (
                PythonVersion::Py315,
                ["p", "q", "args", "kw", "kwargs", "return"],
            ),
        ] {
            let module = compile_with_options(
                source,
                "annotations.py",
                &CompileOptions {
                    python_version,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let annotations = find_code(&module.code, "__annotate__").unwrap();
            let names = annotations
                .constants
                .iter()
                .filter_map(|constant| match constant {
                    Constant::String(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(names, expected);
        }
    }

    #[test]
    fn python315_annotation_qualified_names_preserve_enclosing_scope() {
        for (source, annotation, lambda) in [
            (
                "def f(x: (lambda: None)): pass",
                "f.__annotate__",
                "<lambda>",
            ),
            (
                "def outer():\n def f(x: (lambda: None)): pass",
                "outer.<locals>.f.__annotate__",
                "outer.<locals>.<lambda>",
            ),
            (
                "def f[T](x: (lambda: None)): pass",
                "__annotate__",
                "<generic parameters of f>.<lambda>",
            ),
            (
                "def outer():\n global f\n def f[T](x: (lambda: None)): pass",
                "outer.<locals>.f.__annotate__",
                "outer.<locals>.<generic parameters of f>.<lambda>",
            ),
            (
                "class C:\n def f[T](x: (lambda: None)): pass",
                "C.f.__annotate__",
                "C.<generic parameters of f>.<lambda>",
            ),
        ] {
            let module = compile_with_options(
                source,
                "annotations.py",
                &CompileOptions {
                    python_version: PythonVersion::Py315,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            assert_eq!(
                find_code(&module.code, "__annotate__")
                    .unwrap()
                    .qualified_name,
                annotation,
                "{source}",
            );
            assert_eq!(
                find_code(&module.code, "<lambda>").unwrap().qualified_name,
                lambda,
                "{source}",
            );
        }
    }
}
