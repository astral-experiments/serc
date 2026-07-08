use std::borrow::Cow;
use std::collections::BTreeSet;

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use ruff_python_ast::{Expr, ExprContext, Pattern, Stmt, StmtClassDef, StmtFunctionDef, TypeParam};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::CompileError;
use crate::target::code_flags::{CO_FUTURE_ANNOTATIONS, CO_FUTURE_BARRY_AS_BDFL};

use super::symbol::mangle_name;
use super::{Compiler, SHADOWED_SUPER_SENTINEL, Scope};

impl Compiler {
    pub(super) fn scope_arena(&self) -> &ScopeArena {
        &self.context.scopes
    }

    pub(super) fn function_scope_id(
        &self,
        definition: &StmtFunctionDef,
    ) -> Result<ScopeId, CompileError> {
        self.scope_arena()
            .child(self.unit.scope_id, ScopeOrigin::function(definition))
            .ok_or_else(|| {
                CompileError::Internal(format!(
                    "missing scope plan for nested function `{}`",
                    definition.name
                ))
            })
    }

    fn class_scope_id(&self, definition: &StmtClassDef) -> Result<ScopeId, CompileError> {
        self.scope_arena()
            .child(self.unit.scope_id, ScopeOrigin::class(definition))
            .ok_or_else(|| {
                CompileError::Internal(format!(
                    "missing scope plan for nested class `{}`",
                    definition.name
                ))
            })
    }

    pub(super) fn resolved_function_plan(
        &self,
        definition: &StmtFunctionDef,
    ) -> Result<(ScopeId, LoweringFunctionPlan), CompileError> {
        let scope_id = self.function_scope_id(definition)?;
        let arena = self.scope_arena();
        let mut plan = arena
            .function(scope_id)
            .ok_or_else(|| CompileError::Internal("function scope has no plan".to_string()))?
            .clone();
        let required = plan.required.iter().copied().collect::<Vec<_>>();
        for name in required {
            let captures_name = match &self.unit.scope {
                Scope::Class { free_indices, .. } => {
                    arena.names.get("__class__") == Some(name) || free_indices.contains_key(&name)
                }
                Scope::Module | Scope::Function { .. } => self.can_provide_closure_id(name),
            };
            if captures_name {
                plan.insert_freevar(&arena.names, name);
            } else {
                plan.mark_global(name);
            }
        }
        for name in plan
            .annotation_references
            .iter()
            .copied()
            .collect::<Vec<_>>()
        {
            let captures_name = match &self.unit.scope {
                // Method annotation thunks are created by the class body and can capture the
                // class body's free variables, but not arbitrary class-local bindings.
                Scope::Class { free_indices, .. } => free_indices.contains_key(&name),
                Scope::Module | Scope::Function { .. } => self.can_provide_closure_id(name),
            };
            if captures_name {
                plan.insert_annotation_freevar(&arena.names, name);
            }
        }
        Ok((scope_id, plan.into_lowering_plan()))
    }

    pub(super) fn class_plan(
        &self,
        definition: &StmtClassDef,
    ) -> Result<(ScopeId, LoweringClassPlan), CompileError> {
        let scope_id = self.class_scope_id(definition)?;
        let arena = self.scope_arena();
        let plan = arena
            .class(scope_id)
            .ok_or_else(|| CompileError::Internal("class scope has no plan".to_string()))?
            .lowering_plan();
        Ok((scope_id, plan))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct NameId(pub(super) u32);

#[derive(Debug, Default)]
pub(super) struct NameInterner {
    names: Vec<String>,
    by_name: FxHashMap<String, NameId>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct NameSet {
    words: Vec<u64>,
}

impl NameSet {
    pub(super) fn contains(&self, name: NameId) -> bool {
        let index = usize::try_from(name.0).expect("name ID should fit in usize");
        self.words
            .get(index / u64::BITS as usize)
            .is_some_and(|word| word & (1 << (index % u64::BITS as usize)) != 0)
    }

    pub(super) fn insert(&mut self, name: NameId) -> bool {
        let index = usize::try_from(name.0).expect("name ID should fit in usize");
        let word_index = index / u64::BITS as usize;
        self.words.resize(self.words.len().max(word_index + 1), 0);
        let bit = 1 << (index % u64::BITS as usize);
        let inserted = self.words[word_index] & bit == 0;
        self.words[word_index] |= bit;
        inserted
    }

    pub(super) fn remove(&mut self, name: NameId) -> bool {
        let index = usize::try_from(name.0).expect("name ID should fit in usize");
        let Some(word) = self.words.get_mut(index / u64::BITS as usize) else {
            return false;
        };
        let bit = 1 << (index % u64::BITS as usize);
        let removed = *word & bit != 0;
        *word &= !bit;
        removed
    }

    pub(super) fn iter_unordered(&self) -> impl Iterator<Item = NameId> + '_ {
        self.words
            .iter()
            .enumerate()
            .flat_map(|(word_index, word)| {
                (0..u64::BITS)
                    .filter(move |bit| word & (1 << bit) != 0)
                    .map(move |bit| {
                        NameId(
                            u32::try_from(word_index * u64::BITS as usize + bit as usize)
                                .expect("name ID should fit in u32"),
                        )
                    })
            })
    }
}

impl Extend<NameId> for NameSet {
    fn extend<T: IntoIterator<Item = NameId>>(&mut self, iter: T) {
        for name in iter {
            self.insert(name);
        }
    }
}

impl FromIterator<NameId> for NameSet {
    fn from_iter<T: IntoIterator<Item = NameId>>(iter: T) -> Self {
        let mut names = Self::default();
        names.extend(iter);
        names
    }
}

impl NameInterner {
    pub(super) fn intern(&mut self, name: &str) -> NameId {
        if let Some(id) = self.by_name.get(name) {
            return *id;
        }
        self.insert_owned(name.to_string())
    }

    fn intern_owned(&mut self, name: String) -> NameId {
        if let Some(id) = self.by_name.get(name.as_str()) {
            return *id;
        }
        self.insert_owned(name)
    }

    fn insert_owned(&mut self, name: String) -> NameId {
        let id = NameId(u32::try_from(self.names.len()).expect("too many interned names"));
        self.names.push(name.clone());
        self.by_name.insert(name, id);
        id
    }

    pub(super) fn get(&self, name: &str) -> Option<NameId> {
        self.by_name.get(name).copied()
    }

    fn intern_set(&mut self, names: impl IntoIterator<Item = String>) -> FxHashSet<NameId> {
        names
            .into_iter()
            .map(|name| self.intern_owned(name))
            .collect()
    }

    pub(super) fn resolve(&self, id: NameId) -> &str {
        &self.names[usize::try_from(id.0).expect("name ID should fit in usize")]
    }

    pub(super) fn insert_lexical(&self, names: &mut Vec<NameId>, id: NameId) {
        let spelling = self.resolve(id);
        match names.binary_search_by(|candidate| self.resolve(*candidate).cmp(spelling)) {
            Ok(_) => {}
            Err(index) => names.insert(index, id),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct FunctionPlan {
    pub(super) locals: Vec<NameId>,
    pub(super) globals: FxHashSet<NameId>,
    pub(super) annotation_references: FxHashSet<NameId>,
    pub(super) cellvars: FxHashSet<NameId>,
    pub(super) inlined_comprehension_cellvars: FxHashSet<NameId>,
    pub(super) freevars: Vec<NameId>,
    pub(super) annotation_freevars: Vec<NameId>,
    pub(super) required: FxHashSet<NameId>,
    pub(super) type_parameter_names: FxHashSet<NameId>,
}

// These facts share the same current-scope traversal boundary. Named function and class bodies
// belong to their own `ScopeArena` nodes; only their headers contribute facts to the parent.
#[derive(Debug, Default)]
pub(super) struct ScopeBodyFacts {
    pub(super) globals: FxHashSet<String>,
    pub(super) nonlocals: FxHashSet<String>,
    pub(super) references: FxHashSet<String>,
    explicit_dunder_class_reference: bool,
    pub(super) nested_lambda_requirements: BTreeSet<String>,
    pub(super) nested_generator_requirements: BTreeSet<String>,
    pub(super) nested_annotation_scope_requirements: BTreeSet<String>,
    pub(super) generator_named_targets: FxHashSet<String>,
    pub(super) inlined_comprehension_cellvars: FxHashSet<String>,
    pub(super) has_function_definition: bool,
    pub(super) has_generic_definition: bool,
    pub(super) has_type_alias: bool,
    pub(super) has_simple_annotations: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct LoweringFunctionPlan {
    pub(super) locals: Vec<NameId>,
    pub(super) globals: FxHashSet<NameId>,
    pub(super) annotation_references: FxHashSet<NameId>,
    pub(super) cellvars: FxHashSet<NameId>,
    pub(super) inlined_comprehension_cellvars: FxHashSet<NameId>,
    pub(super) freevars: Vec<NameId>,
    pub(super) annotation_freevars: Vec<NameId>,
    pub(super) required: FxHashSet<NameId>,
}

impl LoweringFunctionPlan {
    pub(super) fn absorb_child_requirements(
        &mut self,
        required_names: impl IntoIterator<Item = NameId>,
        annotation_references: impl IntoIterator<Item = NameId>,
    ) {
        let local_names: FxHashSet<_> = self.locals.iter().copied().collect();
        for name in required_names {
            if local_names.contains(&name) {
                self.cellvars.insert(name);
            } else if !self.globals.contains(&name) {
                self.required.insert(name);
            }
        }
        for name in annotation_references {
            if local_names.contains(&name) {
                self.cellvars.insert(name);
            } else if !self.globals.contains(&name) {
                self.required.insert(name);
            }
        }
    }

    pub(super) fn insert_freevar(&mut self, names: &NameInterner, name: NameId) {
        names.insert_lexical(&mut self.freevars, name);
    }

    pub(super) fn mark_global(&mut self, name: NameId) {
        self.annotation_freevars
            .retain(|candidate| *candidate != name);
        if self.locals.contains(&name) {
            return;
        }
        self.freevars.retain(|candidate| *candidate != name);
        self.globals.insert(name);
        self.required.remove(&name);
    }
}

impl FunctionPlan {
    fn absorb_child_requirements(
        &mut self,
        required_names: impl IntoIterator<Item = NameId>,
        annotation_references: impl IntoIterator<Item = NameId>,
    ) {
        let local_names: FxHashSet<_> = self.locals.iter().copied().collect();
        for name in required_names {
            if local_names.contains(&name) {
                self.cellvars.insert(name);
            } else if !self.globals.contains(&name) {
                self.required.insert(name);
            }
        }
        for name in annotation_references {
            if local_names.contains(&name) {
                self.cellvars.insert(name);
            } else if !self.globals.contains(&name) {
                self.required.insert(name);
            }
        }
    }

    fn build(
        names: &mut NameInterner,
        definition: &StmtFunctionDef,
        future_annotations: bool,
        facts: ScopeBodyFacts,
        nested_function_requirements: &FxHashSet<NameId>,
        nested_function_annotation_references: &FxHashSet<NameId>,
        class_requirements: &FxHashSet<NameId>,
    ) -> Self {
        let ScopeBodyFacts {
            globals,
            nonlocals,
            mut references,
            explicit_dunder_class_reference,
            nested_lambda_requirements: lambda_requirements,
            nested_generator_requirements: generator_requirements,
            nested_annotation_scope_requirements: annotation_scope_requirements,
            generator_named_targets,
            inlined_comprehension_cellvars,
            ..
        } = facts;

        // Record potential local loads and actual bindings in one source-order pass. Filtering the
        // potential names after the pass preserves CPython's load-before-store local ordering
        // without walking the body once just to discover its complete binding set.
        let mut locals = LocalCollector {
            candidate_bindings: Some(FxHashMap::default()),
            globals,
            nonlocals,
            ..LocalCollector::default()
        };
        for parameter in definition
            .parameters
            .posonlyargs
            .iter()
            .chain(&definition.parameters.args)
            .chain(&definition.parameters.kwonlyargs)
        {
            locals.insert(parameter.name().as_str());
        }
        if let Some(parameter) = &definition.parameters.vararg {
            locals.insert(parameter.name.as_str());
        }
        if let Some(parameter) = &definition.parameters.kwarg {
            locals.insert(parameter.name.as_str());
        }
        locals.collect_suite(&definition.body);
        locals.retain_bindings();

        let mut annotation_collector = ReferenceCollector::default();
        if !future_annotations {
            for parameter in definition
                .parameters
                .posonlyargs
                .iter()
                .chain(&definition.parameters.args)
            {
                if let Some(annotation) = parameter.parameter.annotation.as_deref() {
                    annotation_collector.visit_expr(annotation);
                }
            }
            if let Some(parameter) = &definition.parameters.vararg
                && let Some(annotation) = parameter.annotation.as_deref()
            {
                annotation_collector.visit_expr(annotation);
            }
            for parameter in &definition.parameters.kwonlyargs {
                if let Some(annotation) = parameter.parameter.annotation.as_deref() {
                    annotation_collector.visit_expr(annotation);
                }
            }
            if let Some(parameter) = &definition.parameters.kwarg
                && let Some(annotation) = parameter.annotation.as_deref()
            {
                annotation_collector.visit_expr(annotation);
            }
            if let Some(annotation) = definition.returns.as_deref() {
                annotation_collector.visit_expr(annotation);
            }
        }

        if locals.annotation_only.contains("__class__") && !explicit_dunder_class_reference {
            references.remove("__class__");
        }
        let local_names: FxHashSet<_> = locals.names.iter().cloned().collect();
        locals.names.retain(|name| {
            !locals.annotation_only.contains(name)
                || references.contains(name)
                    && (name != "__class__" || explicit_dunder_class_reference)
                || names
                    .get(name)
                    .is_some_and(|name| class_requirements.contains(&name))
                || lambda_requirements.contains(name)
                || generator_requirements.contains(name)
                || annotation_scope_requirements.contains(name)
                || names
                    .get(name)
                    .is_some_and(|name| nested_function_requirements.contains(&name))
        });
        references.extend(
            class_requirements
                .iter()
                .map(|name| names.resolve(*name).to_string()),
        );
        references.extend(lambda_requirements.iter().cloned());
        references.extend(generator_requirements.iter().cloned());
        references.extend(annotation_scope_requirements.iter().cloned());
        let mut cellvars = generator_named_targets
            .into_iter()
            .filter(|name| local_names.contains(name))
            .collect::<FxHashSet<_>>();
        cellvars.extend(
            class_requirements
                .iter()
                .map(|name| names.resolve(*name))
                .filter(|name| local_names.contains(*name))
                .map(str::to_string),
        );
        cellvars.extend(
            lambda_requirements
                .into_iter()
                .filter(|name| local_names.contains(name)),
        );
        cellvars.extend(
            generator_requirements
                .into_iter()
                .filter(|name| local_names.contains(name)),
        );
        cellvars.extend(
            annotation_scope_requirements
                .into_iter()
                .filter(|name| local_names.contains(name)),
        );
        let retained_local_names: FxHashSet<_> = locals.names.iter().map(String::as_str).collect();
        let required: BTreeSet<_> = references
            .iter()
            .chain(&locals.nonlocals)
            .filter(|name| {
                !retained_local_names.contains(name.as_str()) && !locals.globals.contains(*name)
            })
            .cloned()
            .collect();
        let type_parameter_names = definition
            .type_params
            .iter()
            .flat_map(|parameters| parameters.iter())
            .map(|parameter| parameter.name().as_str().to_string())
            .collect::<Vec<_>>();
        let mut plan = Self {
            locals: locals
                .names
                .into_iter()
                .map(|name| names.intern_owned(name))
                .collect(),
            globals: names.intern_set(locals.globals),
            annotation_references: names.intern_set(annotation_collector.references),
            cellvars: names.intern_set(cellvars),
            inlined_comprehension_cellvars: names.intern_set(inlined_comprehension_cellvars),
            freevars: Vec::new(),
            annotation_freevars: Vec::new(),
            required: names.intern_set(required),
            type_parameter_names: names.intern_set(type_parameter_names),
        };
        plan.absorb_child_requirements(
            nested_function_requirements.iter().copied(),
            nested_function_annotation_references.iter().copied(),
        );
        plan
    }

    pub(super) fn insert_freevar(&mut self, names: &NameInterner, name: NameId) {
        names.insert_lexical(&mut self.freevars, name);
    }

    fn insert_annotation_freevar(&mut self, names: &NameInterner, name: NameId) {
        names.insert_lexical(&mut self.annotation_freevars, name);
    }

    pub(super) fn mark_global(&mut self, name: NameId) {
        self.annotation_freevars
            .retain(|candidate| *candidate != name);
        if self.locals.contains(&name) {
            return;
        }
        self.freevars.retain(|candidate| *candidate != name);
        self.globals.insert(name);
        self.required.remove(&name);
    }

    pub(super) fn into_lowering_plan(self) -> LoweringFunctionPlan {
        LoweringFunctionPlan {
            locals: self.locals,
            globals: self.globals,
            annotation_references: self.annotation_references,
            cellvars: self.cellvars,
            inlined_comprehension_cellvars: self.inlined_comprehension_cellvars,
            freevars: self.freevars,
            annotation_freevars: self.annotation_freevars,
            required: self.required,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ClassPlan {
    globals: FxHashSet<NameId>,
    nonlocals: FxHashSet<NameId>,
    bound_names: FxHashSet<NameId>,
    comprehension_targets: Vec<NameId>,
    required: FxHashSet<NameId>,
    needs_class_closure: bool,
    needs_classdict: bool,
    has_conditional_annotations: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct LoweringClassPlan {
    pub(super) globals: FxHashSet<NameId>,
    pub(super) nonlocals: FxHashSet<NameId>,
    pub(super) bound_names: FxHashSet<NameId>,
    pub(super) comprehension_targets: Vec<NameId>,
    pub(super) required: FxHashSet<NameId>,
    pub(super) needs_class_closure: bool,
    pub(super) needs_classdict: bool,
    pub(super) has_conditional_annotations: bool,
}

impl ClassPlan {
    fn build(
        names: &mut NameInterner,
        definition: &StmtClassDef,
        future_annotations: bool,
        facts: ScopeBodyFacts,
        method_requirements: &FxHashSet<NameId>,
        method_annotation_references: &FxHashSet<NameId>,
        nested_class_requirements: &FxHashSet<NameId>,
        child_needs_class_closure: bool,
    ) -> Self {
        let has_conditional_annotations = if future_annotations {
            false
        } else {
            let mut annotations = Vec::new();
            collect_conditional_simple_annotations(&definition.body, &mut annotations);
            !annotations.is_empty()
        };
        let ScopeBodyFacts {
            globals,
            nonlocals,
            references,
            explicit_dunder_class_reference,
            nested_lambda_requirements,
            nested_annotation_scope_requirements: annotation_scope_requirements,
            has_function_definition,
            has_generic_definition,
            has_type_alias,
            has_simple_annotations,
            ..
        } = facts;
        let mut locals = LocalCollector {
            globals,
            nonlocals,
            ignore_generator_body_comprehension_targets: true,
            ..LocalCollector::default()
        };
        locals.collect_suite(&definition.body);

        let mut body_explicitly_references_dunder_class = explicit_dunder_class_reference;
        let mut required = references
            .into_iter()
            .filter(|name| {
                locals.nonlocals.contains(name)
                    || (!locals.globals.contains(name) && !locals.seen.contains(name))
            })
            .collect::<BTreeSet<_>>();
        required.extend(locals.nonlocals.iter().cloned());
        body_explicitly_references_dunder_class |=
            annotation_scope_requirements.contains("__class__");
        required.extend(
            annotation_scope_requirements
                .into_iter()
                .filter(|name| !locals.seen.contains(name) && !locals.globals.contains(name)),
        );
        let body_requires_dunder_class =
            body_explicitly_references_dunder_class && required.contains("__class__");

        required.extend(
            method_requirements
                .iter()
                .map(|name| names.resolve(*name).to_string()),
        );
        if !future_annotations {
            required.extend(
                method_annotation_references
                    .iter()
                    .map(|name| names.resolve(*name))
                    .filter(|name| !locals.seen.contains(*name) && !locals.globals.contains(*name))
                    .map(str::to_string),
            );
        }
        required.extend(
            nested_class_requirements
                .iter()
                .map(|name| names.resolve(*name).to_string()),
        );
        if !body_requires_dunder_class && !locals.nonlocals.contains("__class__") {
            required.remove("__class__");
        }
        required.remove("__classdict__");
        required.remove("__conditional_annotations__");

        let needs_class_closure =
            child_needs_class_closure || nested_lambda_requirements.contains("__class__");
        let needs_classdict = has_type_alias
            || has_generic_definition
            || !future_annotations && (has_function_definition || has_simple_annotations);
        let comprehension_targets = std::mem::take(&mut locals.comprehension_targets)
            .into_iter()
            .map(|name| names.intern_owned(name))
            .collect();
        Self {
            globals: names.intern_set(locals.globals),
            nonlocals: names.intern_set(locals.nonlocals),
            bound_names: names.intern_set(locals.seen),
            comprehension_targets,
            required: names.intern_set(required),
            needs_class_closure,
            needs_classdict,
            has_conditional_annotations,
        }
    }

    fn lowering_plan(&self) -> LoweringClassPlan {
        LoweringClassPlan {
            globals: self.globals.clone(),
            nonlocals: self.nonlocals.clone(),
            bound_names: self.bound_names.clone(),
            comprehension_targets: self.comprehension_targets.clone(),
            required: self.required.clone(),
            needs_class_closure: self.needs_class_closure,
            needs_classdict: self.needs_classdict,
            has_conditional_annotations: self.has_conditional_annotations,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ScopeId(usize);

impl ScopeId {
    pub(super) const MODULE: Self = Self(0);
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScopeOrigin {
    Function(u32, u32),
    Class(u32, u32),
}

impl ScopeOrigin {
    fn function(definition: &StmtFunctionDef) -> Self {
        Self::Function(
            u32::from(definition.range.start()),
            u32::from(definition.range.end()),
        )
    }

    fn class(definition: &StmtClassDef) -> Self {
        Self::Class(
            u32::from(definition.range.start()),
            u32::from(definition.range.end()),
        )
    }
}

#[derive(Debug)]
enum ScopePlan {
    Module,
    Function(FunctionPlan),
    Class(ClassPlan),
}

#[derive(Debug)]
struct ScopeNode {
    plan: ScopePlan,
    children: Vec<ScopeId>,
}

#[derive(Debug)]
pub(super) struct ScopeArena {
    pub(super) names: NameInterner,
    nodes: Vec<ScopeNode>,
    by_parent_and_origin: FxHashMap<(ScopeId, ScopeOrigin), ScopeId>,
}

#[derive(Debug)]
pub(super) struct ModulePlan {
    pub(super) future_flags: u32,
    pub(super) module_globals: NameSet,
    pub(super) imported_scope_names: NameSet,
    pub(super) comprehension_targets: Vec<NameId>,
    pub(super) comprehension_cell_names: NameSet,
    pub(super) has_annotations: bool,
    pub(super) has_simple_annotations: bool,
}

#[derive(Debug, Default)]
struct ModulePlanBuilder {
    module_globals: FxHashSet<String>,
    imported_scope_names: FxHashSet<String>,
    comprehension_cell_names: FxHashSet<String>,
    has_annotations: bool,
    has_simple_annotations: bool,
}

impl ModulePlan {
    fn build(
        names: &NameInterner,
        future_flags: u32,
        mut facts: ModulePlanBuilder,
        bindings: LocalCollector,
    ) -> Self {
        let name_id = |name: String| {
            names
                .get(&name)
                .unwrap_or_else(|| panic!("module analysis did not intern `{name}`"))
        };
        let mut imported_scope_names: NameSet =
            facts.imported_scope_names.drain().map(&name_id).collect();
        if bindings.seen.contains("super") {
            imported_scope_names.insert(
                names
                    .get(SHADOWED_SUPER_SENTINEL)
                    .expect("module analysis should intern the shadowed-super sentinel"),
            );
        }
        Self {
            future_flags,
            module_globals: facts.module_globals.drain().map(&name_id).collect(),
            imported_scope_names,
            comprehension_targets: bindings
                .comprehension_targets
                .into_iter()
                .map(&name_id)
                .collect(),
            comprehension_cell_names: facts
                .comprehension_cell_names
                .drain()
                .map(name_id)
                .collect(),
            has_annotations: facts.has_annotations,
            has_simple_annotations: facts.has_simple_annotations,
        }
    }
}

impl Default for ScopeArena {
    fn default() -> Self {
        Self {
            names: NameInterner::default(),
            nodes: vec![ScopeNode {
                plan: ScopePlan::Module,
                children: Vec::new(),
            }],
            by_parent_and_origin: FxHashMap::default(),
        }
    }
}

impl ScopeArena {
    pub(super) fn analyze(body: &[Stmt]) -> (Self, ModulePlan) {
        let future_flags = future_feature_flags(body);
        let future_annotations = future_flags & CO_FUTURE_ANNOTATIONS != 0;
        let mut arena = Self::default();
        for name in [
            SHADOWED_SUPER_SENTINEL,
            "",
            ".0",
            ".defaults",
            ".format",
            ".generic_base",
            ".kwdefaults",
            ".type_params",
            "__class__",
            "__classdict__",
            "__classcell__",
            "__classdictcell__",
            "__conditional_annotations__",
            "__annotate__",
            "__annotate_func__",
            "__annotations__",
            "__doc__",
            "__firstlineno__",
            "__module__",
            "__name__",
            "__qualname__",
            "__static_attributes__",
            "__type_params__",
            "format",
            "join",
            "super",
        ] {
            arena.names.intern(name);
        }
        // Unordered module facts share the existing name walk. The separate local collector is
        // retained only for its CPython-sensitive comprehension-target ordering.
        let facts = ModuleNameCollector::collect(&mut arena.names, body);
        let mut bindings = LocalCollector::default();
        bindings.collect_suite(body);
        arena.build_suite(ScopeId::MODULE, body, future_annotations);
        let module = ModulePlan::build(&arena.names, future_flags, facts, bindings);
        (arena, module)
    }

    fn build_suite(&mut self, parent: ScopeId, body: &[Stmt], future_annotations: bool) {
        for statement in body {
            match statement {
                Stmt::FunctionDef(definition) => {
                    self.build_function(parent, definition, future_annotations);
                }
                Stmt::ClassDef(definition) => {
                    self.build_class(parent, definition, future_annotations);
                }
                Stmt::If(statement) => {
                    self.build_suite(parent, &statement.body, future_annotations);
                    for clause in &statement.elif_else_clauses {
                        self.build_suite(parent, &clause.body, future_annotations);
                    }
                }
                Stmt::For(statement) => {
                    self.build_suite(parent, &statement.body, future_annotations);
                    self.build_suite(parent, &statement.orelse, future_annotations);
                }
                Stmt::While(statement) => {
                    self.build_suite(parent, &statement.body, future_annotations);
                    self.build_suite(parent, &statement.orelse, future_annotations);
                }
                Stmt::Try(statement) => {
                    self.build_suite(parent, &statement.body, future_annotations);
                    for handler in &statement.handlers {
                        if let Some(handler) = handler.as_except_handler() {
                            self.build_suite(parent, &handler.body, future_annotations);
                        }
                    }
                    self.build_suite(parent, &statement.orelse, future_annotations);
                    self.build_suite(parent, &statement.finalbody, future_annotations);
                }
                Stmt::With(statement) => {
                    self.build_suite(parent, &statement.body, future_annotations);
                }
                Stmt::Match(statement) => {
                    for case in &statement.cases {
                        self.build_suite(parent, &case.body, future_annotations);
                    }
                }
                _ => {}
            }
        }
    }

    fn add_scope(&mut self, parent: ScopeId, origin: ScopeOrigin, plan: ScopePlan) -> ScopeId {
        let id = ScopeId(self.nodes.len());
        self.nodes.push(ScopeNode {
            plan,
            children: Vec::new(),
        });
        self.nodes[parent.0].children.push(id);
        self.by_parent_and_origin.insert((parent, origin), id);
        id
    }

    fn build_function(
        &mut self,
        parent: ScopeId,
        definition: &StmtFunctionDef,
        future_annotations: bool,
    ) {
        let facts = ScopeBodyFacts::for_function(&definition.body);
        let id = self.add_scope(
            parent,
            ScopeOrigin::function(definition),
            ScopePlan::Function(FunctionPlan::default()),
        );
        self.build_suite(id, &definition.body, future_annotations);
        let (nested_function_requirements, nested_function_annotation_references) =
            self.nested_function_requirements(id);
        let class_requirements = self.nested_class_requirements(id);
        let plan = FunctionPlan::build(
            &mut self.names,
            definition,
            future_annotations,
            facts,
            &nested_function_requirements,
            &nested_function_annotation_references,
            &class_requirements,
        );
        self.nodes[id.0].plan = ScopePlan::Function(plan);
    }

    fn build_class(
        &mut self,
        parent: ScopeId,
        definition: &StmtClassDef,
        future_annotations: bool,
    ) {
        let facts = ScopeBodyFacts::for_class(&definition.body, future_annotations);
        let id = self.add_scope(
            parent,
            ScopeOrigin::class(definition),
            ScopePlan::Class(ClassPlan::default()),
        );
        self.build_suite(id, &definition.body, future_annotations);
        let (method_requirements, method_annotation_references) =
            self.nested_function_requirements(id);
        let nested_class_requirements = self.nested_class_requirements(id);
        let dunder_class = self.names.get("__class__");
        let child_needs_class_closure = self.nodes[id.0].children.iter().any(|child| {
            dunder_class.is_some_and(|dunder_class| match &self.nodes[child.0].plan {
                ScopePlan::Function(plan) => plan.required.contains(&dunder_class),
                ScopePlan::Class(plan) => plan.required.contains(&dunder_class),
                ScopePlan::Module => false,
            })
        });
        let plan = ClassPlan::build(
            &mut self.names,
            definition,
            future_annotations,
            facts,
            &method_requirements,
            &method_annotation_references,
            &nested_class_requirements,
            child_needs_class_closure,
        );
        self.nodes[id.0].plan = ScopePlan::Class(plan);
    }

    fn nested_function_requirements(
        &self,
        parent: ScopeId,
    ) -> (FxHashSet<NameId>, FxHashSet<NameId>) {
        let mut required = FxHashSet::default();
        let mut annotation_references = FxHashSet::default();
        for child in &self.nodes[parent.0].children {
            let ScopePlan::Function(plan) = &self.nodes[child.0].plan else {
                continue;
            };
            required.extend(
                plan.required
                    .iter()
                    .filter(|name| !plan.type_parameter_names.contains(*name))
                    .copied(),
            );
            annotation_references.extend(
                plan.annotation_references
                    .iter()
                    .filter(|name| !plan.type_parameter_names.contains(*name))
                    .copied(),
            );
        }
        (required, annotation_references)
    }

    fn nested_class_requirements(&self, parent: ScopeId) -> FxHashSet<NameId> {
        self.nodes[parent.0]
            .children
            .iter()
            .filter_map(|child| match &self.nodes[child.0].plan {
                ScopePlan::Class(plan) => Some(&plan.required),
                ScopePlan::Module | ScopePlan::Function(_) => None,
            })
            .flatten()
            .copied()
            .collect()
    }

    fn child(&self, parent: ScopeId, origin: ScopeOrigin) -> Option<ScopeId> {
        self.by_parent_and_origin.get(&(parent, origin)).copied()
    }

    pub(super) fn function(&self, id: ScopeId) -> Option<&FunctionPlan> {
        match &self.nodes.get(id.0)?.plan {
            ScopePlan::Function(plan) => Some(plan),
            ScopePlan::Module | ScopePlan::Class(_) => None,
        }
    }

    fn class(&self, id: ScopeId) -> Option<&ClassPlan> {
        match &self.nodes.get(id.0)?.plan {
            ScopePlan::Class(plan) => Some(plan),
            ScopePlan::Module | ScopePlan::Function(_) => None,
        }
    }
}

struct ModuleNameCollector<'a> {
    names: &'a mut NameInterner,
    private_name: Option<String>,
    facts: ModulePlanBuilder,
    nested_scope_depth: usize,
    comprehension_depth: usize,
    generator_depth: usize,
}

impl<'a> ModuleNameCollector<'a> {
    fn collect(names: &'a mut NameInterner, body: &[Stmt]) -> ModulePlanBuilder {
        let mut collector = Self {
            names,
            private_name: None,
            facts: ModulePlanBuilder::default(),
            nested_scope_depth: 0,
            comprehension_depth: 0,
            generator_depth: 0,
        };
        collector.collect_suite(body);
        collector.facts
    }

    fn intern(&mut self, name: &str) {
        let mangled = mangle_name(name, self.private_name.as_deref());
        self.names.intern(name);
        if let Cow::Owned(mangled) = mangled {
            self.names.intern_owned(mangled);
        }
    }

    fn collect_suite(&mut self, body: &[Stmt]) {
        for statement in body {
            self.visit_stmt(statement);
        }
    }

    fn collect_parameters(&mut self, parameters: &ruff_python_ast::Parameters) {
        for parameter in parameters
            .posonlyargs
            .iter()
            .chain(&parameters.args)
            .chain(&parameters.kwonlyargs)
        {
            self.intern(parameter.name().as_str());
        }
        if let Some(parameter) = &parameters.vararg {
            self.intern(parameter.name.as_str());
        }
        if let Some(parameter) = &parameters.kwarg {
            self.intern(parameter.name.as_str());
        }
    }

    fn collect_type_parameters(&mut self, parameters: Option<&ruff_python_ast::TypeParams>) {
        if let Some(parameters) = parameters {
            for parameter in parameters {
                self.intern(parameter.name().as_str());
            }
        }
    }
}

impl<'ast> Visitor<'ast> for ModuleNameCollector<'_> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        let at_module_scope = self.nested_scope_depth == 0;
        match statement {
            Stmt::FunctionDef(definition) => {
                self.intern(definition.name.as_str());
                self.collect_parameters(&definition.parameters);
                self.collect_type_parameters(definition.type_params.as_deref());
                self.nested_scope_depth += 1;
                walk_stmt(self, statement);
                self.nested_scope_depth -= 1;
                return;
            }
            Stmt::ClassDef(definition) => {
                self.intern(definition.name.as_str());
                self.collect_type_parameters(definition.type_params.as_deref());
                self.nested_scope_depth += 1;
                for decorator in &definition.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(type_params) = definition.type_params.as_deref() {
                    self.visit_type_params(type_params);
                }
                if let Some(arguments) = definition.arguments.as_deref() {
                    self.visit_arguments(arguments);
                }
                let outer_private_name = self
                    .private_name
                    .replace(definition.name.as_str().to_string());
                self.collect_suite(&definition.body);
                self.private_name = outer_private_name;
                self.nested_scope_depth -= 1;
                return;
            }
            Stmt::TypeAlias(statement) => {
                self.collect_type_parameters(statement.type_params.as_deref());
            }
            Stmt::AnnAssign(annotation) if at_module_scope => {
                self.facts.has_annotations = true;
                self.facts.has_simple_annotations |= annotation.simple;
            }
            Stmt::Global(statement) => {
                for name in &statement.names {
                    self.intern(name.as_str());
                    self.facts.module_globals.insert(name.as_str().to_string());
                }
            }
            Stmt::Nonlocal(statement) => {
                for name in &statement.names {
                    self.intern(name.as_str());
                }
            }
            Stmt::Import(statement) => {
                for alias in &statement.names {
                    let module = alias.name.as_str();
                    self.intern(module);
                    for part in module.split('.').skip(1) {
                        self.intern(part);
                    }
                    let bound_name = alias.asname.as_ref().map_or_else(
                        || alias.name.as_str().split('.').next().unwrap_or(""),
                        |name| name.as_str(),
                    );
                    self.intern(bound_name);
                    if at_module_scope {
                        self.facts
                            .imported_scope_names
                            .insert(bound_name.to_string());
                    }
                }
            }
            Stmt::ImportFrom(statement) => {
                let is_future = statement
                    .module
                    .as_ref()
                    .is_some_and(|module| module.as_str() == "__future__");
                self.intern(
                    statement
                        .module
                        .as_ref()
                        .map_or("", |module| module.as_str()),
                );
                for alias in &statement.names {
                    if alias.name.as_str() != "*" {
                        self.intern(alias.name.as_str());
                        let bound_name = alias
                            .asname
                            .as_ref()
                            .map_or(alias.name.as_str(), |name| name.as_str());
                        self.intern(bound_name);
                        if at_module_scope && !is_future {
                            self.facts
                                .imported_scope_names
                                .insert(bound_name.to_string());
                        }
                    }
                }
            }
            Stmt::Try(statement) => {
                for handler in &statement.handlers {
                    if let Some(handler) = handler.as_except_handler()
                        && let Some(name) = &handler.name
                    {
                        self.intern(name.as_str());
                    }
                }
            }
            _ => {}
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        match expression {
            Expr::Name(name) => {
                self.intern(name.id.as_str());
            }
            Expr::Attribute(attribute) => {
                self.intern(attribute.attr.as_str());
            }
            Expr::Lambda(lambda) => {
                if let Some(parameters) = lambda.parameters.as_deref() {
                    self.collect_parameters(parameters);
                }
                self.nested_scope_depth += 1;
                walk_expr(self, expression);
                self.nested_scope_depth -= 1;
                return;
            }
            _ => {}
        }

        if self.nested_scope_depth == 0 && self.generator_depth == 0 {
            match expression {
                Expr::ListComp(comprehension) => {
                    self.facts
                        .comprehension_cell_names
                        .extend(comprehension_cell_names(
                            &comprehension.generators,
                            None,
                            &comprehension.elt,
                        ));
                }
                Expr::SetComp(comprehension) => {
                    self.facts
                        .comprehension_cell_names
                        .extend(comprehension_cell_names(
                            &comprehension.generators,
                            None,
                            &comprehension.elt,
                        ));
                }
                Expr::DictComp(comprehension) => {
                    self.facts
                        .comprehension_cell_names
                        .extend(comprehension_cell_names(
                            &comprehension.generators,
                            comprehension.key.as_deref(),
                            &comprehension.value,
                        ));
                }
                _ => {}
            }
        }
        if self.nested_scope_depth == 0
            && self.comprehension_depth > 0
            && let Expr::Named(named) = expression
            && let Expr::Name(name) = named.target.as_ref()
        {
            self.facts
                .module_globals
                .insert(name.id.as_str().to_string());
        }

        let is_generator = matches!(expression, Expr::Generator(_));
        let is_comprehension = is_generator
            || matches!(
                expression,
                Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_)
            );
        if is_comprehension {
            self.comprehension_depth += 1;
            self.generator_depth += usize::from(is_generator);
            walk_expr(self, expression);
            self.generator_depth -= usize::from(is_generator);
            self.comprehension_depth -= 1;
            return;
        }
        walk_expr(self, expression);
    }

    fn visit_pattern(&mut self, pattern: &'ast Pattern) {
        let name = match pattern {
            Pattern::MatchAs(pattern) => pattern.name.as_ref(),
            Pattern::MatchStar(pattern) => pattern.name.as_ref(),
            Pattern::MatchMapping(pattern) => pattern.rest.as_ref(),
            _ => None,
        };
        if let Some(name) = name {
            self.intern(name.as_str());
        }
        walk_pattern(self, pattern);
    }
}

fn function_default_expressions(definition: &StmtFunctionDef) -> impl Iterator<Item = &Expr> {
    definition
        .parameters
        .posonlyargs
        .iter()
        .chain(&definition.parameters.args)
        .filter_map(|parameter| parameter.default.as_deref())
        .chain(
            definition
                .parameters
                .kwonlyargs
                .iter()
                .filter_map(|parameter| parameter.default.as_deref()),
        )
}
#[derive(Debug, Default)]
struct LocalCollector {
    names: Vec<String>,
    seen: FxHashSet<String>,
    comprehension_targets: Vec<String>,
    seen_comprehension_targets: FxHashSet<String>,
    candidate_bindings: Option<FxHashMap<String, bool>>,
    annotation_only: FxHashSet<String>,
    globals: FxHashSet<String>,
    nonlocals: FxHashSet<String>,
    ignore_generator_body_comprehension_targets: bool,
}

impl LocalCollector {
    fn insert_comprehension_target(&mut self, name: String) {
        if self.seen_comprehension_targets.insert(name.clone()) {
            self.comprehension_targets.push(name);
        }
    }

    fn mark_binding(&mut self, name: &str) -> bool {
        if self.globals.contains(name) || self.nonlocals.contains(name) {
            return false;
        }
        if let Some(candidate_bindings) = &mut self.candidate_bindings {
            if let Some(binding) = candidate_bindings.get_mut(name) {
                *binding = true;
                return false;
            }
            candidate_bindings.insert(name.to_string(), true);
            self.names.push(name.to_string());
            return true;
        }
        if self.seen.insert(name.to_string()) {
            self.names.push(name.to_string());
            true
        } else {
            false
        }
    }

    fn insert_candidate(&mut self, name: &str) {
        if self.globals.contains(name) || self.nonlocals.contains(name) {
            return;
        }
        let Some(candidate_bindings) = &mut self.candidate_bindings else {
            return;
        };
        if candidate_bindings.contains_key(name) {
            return;
        }
        candidate_bindings.insert(name.to_string(), false);
        self.names.push(name.to_string());
    }

    fn retain_bindings(&mut self) {
        let Some(candidate_bindings) = self.candidate_bindings.take() else {
            return;
        };
        self.names.retain(|name| candidate_bindings[name]);
    }

    fn insert(&mut self, name: &str) {
        self.annotation_only.remove(name);
        self.mark_binding(name);
    }

    fn insert_annotation_only(&mut self, name: &str) {
        if self.mark_binding(name) {
            self.annotation_only.insert(name.to_string());
        }
    }

    fn collect_suite(&mut self, body: &[Stmt]) {
        for statement in body {
            match statement {
                Stmt::Assign(assignment) => {
                    let mut references = ReferenceCollector::default();
                    references.visit_expr(&assignment.value);
                    for target in &assignment.targets {
                        let mut names = Vec::new();
                        collect_target_names(target, &mut names);
                        for name in names {
                            if references.references.contains(name.as_str()) {
                                self.insert(&name);
                            }
                        }
                    }
                    self.collect_expression(&assignment.value);
                    for target in &assignment.targets {
                        self.collect_target(target);
                    }
                }
                Stmt::AugAssign(assignment) => {
                    self.collect_expression(&assignment.value);
                    if let Expr::Name(name) = assignment.target.as_ref() {
                        self.insert(name.id.as_str());
                    }
                }
                Stmt::AnnAssign(assignment) => {
                    if let Some(value) = &assignment.value {
                        self.collect_expression(value);
                    }
                    if (assignment.simple || assignment.value.is_some())
                        && let Expr::Name(name) = assignment.target.as_ref()
                    {
                        if assignment.value.is_some() {
                            self.insert(name.id.as_str());
                        } else {
                            self.insert_annotation_only(name.id.as_str());
                        }
                    }
                }
                Stmt::FunctionDef(definition) => {
                    for decorator in &definition.decorator_list {
                        self.collect_definition_header_expression(&decorator.expression);
                    }
                    for default in function_default_expressions(definition) {
                        self.collect_definition_header_expression(default);
                    }
                    self.insert(definition.name.as_str());
                }
                Stmt::ClassDef(definition) => {
                    for decorator in &definition.decorator_list {
                        self.collect_definition_header_expression(&decorator.expression);
                    }
                    if definition.type_params.is_none()
                        && let Some(arguments) = definition.arguments.as_deref()
                    {
                        for argument in &arguments.args {
                            self.collect_definition_header_expression(argument);
                        }
                        for keyword in &arguments.keywords {
                            self.collect_definition_header_expression(&keyword.value);
                        }
                    }
                    self.insert(definition.name.as_str());
                }
                Stmt::Delete(statement) => {
                    for target in &statement.targets {
                        self.collect_target(target);
                    }
                }
                Stmt::TypeAlias(statement) => {
                    if let Expr::Name(name) = statement.name.as_ref() {
                        self.insert(name.id.as_str());
                    }
                    self.collect_expression(&statement.value);
                }
                Stmt::For(statement) => {
                    self.collect_expression(&statement.iter);
                    self.collect_target(&statement.target);
                    self.collect_suite(&statement.body);
                    self.collect_suite(&statement.orelse);
                }
                Stmt::Import(statement) => {
                    for alias in &statement.names {
                        self.insert(alias.asname.as_ref().map_or_else(
                            || alias.name.as_str().split('.').next().unwrap_or(""),
                            |name| name.as_str(),
                        ));
                    }
                }
                Stmt::ImportFrom(statement) => {
                    for alias in &statement.names {
                        if alias.name.as_str() != "*" {
                            self.insert(
                                alias
                                    .asname
                                    .as_ref()
                                    .map_or(alias.name.as_str(), |name| name.as_str()),
                            );
                        }
                    }
                }
                Stmt::If(statement) => {
                    self.collect_expression(&statement.test);
                    self.collect_suite(&statement.body);
                    for clause in &statement.elif_else_clauses {
                        if let Some(test) = &clause.test {
                            self.collect_expression(test);
                        }
                        self.collect_suite(&clause.body);
                    }
                }
                Stmt::While(statement) => {
                    self.collect_expression(&statement.test);
                    self.collect_suite(&statement.body);
                    self.collect_suite(&statement.orelse);
                }
                Stmt::Try(statement) => {
                    self.collect_suite(&statement.body);
                    for handler in &statement.handlers {
                        if let Some(handler) = handler.as_except_handler() {
                            if let Some(name) = &handler.name {
                                self.insert(name.as_str());
                            }
                            if let Some(exception_type) = &handler.type_ {
                                self.collect_expression(exception_type);
                            }
                            self.collect_suite(&handler.body);
                        }
                    }
                    self.collect_suite(&statement.orelse);
                    self.collect_suite(&statement.finalbody);
                }
                Stmt::With(statement) => {
                    for item in &statement.items {
                        self.collect_expression(&item.context_expr);
                        if let Some(target) = &item.optional_vars {
                            self.collect_target(target);
                        }
                    }
                    self.collect_suite(&statement.body);
                }
                Stmt::Match(statement) => {
                    self.collect_expression(&statement.subject);
                    for case in &statement.cases {
                        self.collect_pattern(&case.pattern);
                        if let Some(guard) = &case.guard {
                            self.collect_expression(guard);
                        }
                        self.collect_suite(&case.body);
                    }
                }
                Stmt::Assert(statement) => {
                    self.collect_expression(&statement.test);
                    if let Some(message) = &statement.msg {
                        self.collect_expression(message);
                    }
                }
                Stmt::Expr(statement) => self.collect_expression(&statement.value),
                Stmt::Raise(statement) => {
                    if let Some(exception) = &statement.exc {
                        self.collect_expression(exception);
                    }
                    if let Some(cause) = &statement.cause {
                        self.collect_expression(cause);
                    }
                }
                Stmt::Return(statement) => {
                    if let Some(value) = &statement.value {
                        self.collect_expression(value);
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_target(&mut self, target: &Expr) {
        match target {
            Expr::Name(name) => self.insert(name.id.as_str()),
            Expr::List(list) => {
                for element in &list.elts {
                    self.collect_target(element);
                }
            }
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.collect_target(element);
                }
            }
            Expr::Starred(starred) => self.collect_target(&starred.value),
            _ => {}
        }
    }

    fn collect_expression(&mut self, expression: &Expr) {
        NamedExpressionCollector {
            collector: self,
            in_inlined_comprehension: false,
            collect_known_loads: true,
            named_target_after_value: false,
        }
        .visit_expr(expression);
    }

    fn collect_definition_header_expression(&mut self, expression: &Expr) {
        NamedExpressionCollector {
            collector: self,
            in_inlined_comprehension: false,
            collect_known_loads: true,
            named_target_after_value: true,
        }
        .visit_expr(expression);
    }

    fn collect_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::MatchValue(pattern) => self.collect_expression(&pattern.value),
            Pattern::MatchSingleton(_) => {}
            Pattern::MatchSequence(pattern) => {
                for pattern in &pattern.patterns {
                    self.collect_pattern(pattern);
                }
            }
            Pattern::MatchMapping(pattern) => {
                for key in &pattern.keys {
                    self.collect_expression(key);
                }
                for pattern in &pattern.patterns {
                    self.collect_pattern(pattern);
                }
                if let Some(rest) = &pattern.rest {
                    self.insert(rest.as_str());
                }
            }
            Pattern::MatchClass(pattern) => {
                self.collect_expression(&pattern.cls);
                for argument in &pattern.arguments.patterns {
                    self.collect_pattern(argument);
                }
                for keyword in &pattern.arguments.keywords {
                    self.collect_pattern(&keyword.pattern);
                }
            }
            Pattern::MatchStar(pattern) => {
                if let Some(name) = &pattern.name {
                    self.insert(name.as_str());
                }
            }
            Pattern::MatchAs(pattern) => {
                if let Some(inner) = &pattern.pattern {
                    self.collect_pattern(inner);
                }
                if let Some(name) = &pattern.name {
                    self.insert(name.as_str());
                }
            }
            Pattern::MatchOr(pattern) => {
                for pattern in &pattern.patterns {
                    self.collect_pattern(pattern);
                }
            }
        }
    }
}

struct NamedExpressionCollector<'a> {
    collector: &'a mut LocalCollector,
    in_inlined_comprehension: bool,
    collect_known_loads: bool,
    named_target_after_value: bool,
}

impl<'ast> Visitor<'ast> for NamedExpressionCollector<'_> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if matches!(expression, Expr::Lambda(_)) {
            return;
        }
        if matches!(expression, Expr::Generator(_)) {
            let previous = std::mem::replace(&mut self.in_inlined_comprehension, false);
            let previous_loads = std::mem::replace(&mut self.collect_known_loads, false);
            if self.collector.ignore_generator_body_comprehension_targets {
                let Expr::Generator(generator) = expression else {
                    unreachable!();
                };
                if let Some(first) = generator.generators.first() {
                    self.visit_expr(&first.iter);
                }
            } else {
                walk_expr(self, expression);
            }
            self.collect_known_loads = previous_loads;
            self.in_inlined_comprehension = previous;
            return;
        }
        if self.named_target_after_value
            && let Expr::Named(named) = expression
        {
            self.visit_expr(&named.value);
            if self.in_inlined_comprehension
                && let Expr::Name(name) = named.target.as_ref()
            {
                self.collector
                    .insert_comprehension_target(name.id.as_str().to_string());
            }
            self.collector.collect_target(&named.target);
            return;
        }
        if self.collect_known_loads
            && let Expr::Name(name) = expression
            && name.ctx == ExprContext::Load
        {
            self.collector.insert_candidate(name.id.as_str());
        }
        let generators = match expression {
            Expr::ListComp(comprehension) => Some(comprehension.generators.as_slice()),
            Expr::SetComp(comprehension) => Some(comprehension.generators.as_slice()),
            Expr::DictComp(comprehension) => Some(comprehension.generators.as_slice()),
            _ => None,
        };
        if let Some(generators) = generators {
            if let Some(first) = generators.first() {
                let mut names = Vec::new();
                collect_nested_comprehension_target_names(&first.iter, &mut names);
                for name in names {
                    self.collector.insert_comprehension_target(name);
                }
            }
            for generator in generators {
                let mut names = Vec::new();
                collect_target_names(&generator.target, &mut names);
                for name in names {
                    self.collector.insert_comprehension_target(name);
                }
                self.collector.collect_target(&generator.target);
            }
            let previous = std::mem::replace(&mut self.in_inlined_comprehension, true);
            walk_expr(self, expression);
            self.in_inlined_comprehension = previous;
            return;
        }
        if let Expr::Named(named) = expression {
            // CPython allocates locals referenced by the value before the walrus target.
            walk_expr(self, expression);

            if self.in_inlined_comprehension
                && let Expr::Name(name) = named.target.as_ref()
            {
                self.collector
                    .insert_comprehension_target(name.id.as_str().to_string());
            }
            self.collector.collect_target(&named.target);
            return;
        }
        walk_expr(self, expression);
    }
}

impl ScopeBodyFacts {
    pub(super) fn for_function(body: &[Stmt]) -> Self {
        ScopeBodyFactsCollector::collect(body, true, true)
    }

    pub(super) fn for_class(body: &[Stmt], future_annotations: bool) -> Self {
        ScopeBodyFactsCollector::collect(body, future_annotations, false)
    }
}

struct ScopeBodyFactsCollector {
    facts: ScopeBodyFacts,
    skip_annotations: bool,
    collect_function_scope_facts: bool,
    generator_depth: usize,
}

impl ScopeBodyFactsCollector {
    fn collect(
        body: &[Stmt],
        skip_annotations: bool,
        collect_function_scope_facts: bool,
    ) -> ScopeBodyFacts {
        let mut collector = Self {
            facts: ScopeBodyFacts::default(),
            skip_annotations,
            collect_function_scope_facts,
            generator_depth: 0,
        };
        for statement in body {
            collector.visit_stmt(statement);
        }
        collector.facts
    }

    fn collect_function_header(&mut self, definition: &StmtFunctionDef) {
        self.facts.has_function_definition = true;
        // A nested function's decorators execute in the current scope and may be that scope's
        // only reference to an enclosing binding.
        for decorator in &definition.decorator_list {
            self.visit_expr(&decorator.expression);
        }
        for default in function_default_expressions(definition) {
            self.visit_expr(default);
        }
        let Some(type_params) = definition.type_params.as_deref() else {
            return;
        };
        self.facts.has_generic_definition = true;
        let type_names: BTreeSet<_> = type_params
            .iter()
            .map(|parameter| parameter.name().as_str().to_string())
            .collect();
        self.facts.nested_annotation_scope_requirements.extend(
            type_parameter_required_names(type_params)
                .difference(&type_names)
                .cloned(),
        );
    }

    fn collect_class_header(&mut self, definition: &StmtClassDef) {
        for decorator in &definition.decorator_list {
            self.visit_expr(&decorator.expression);
        }
        let Some(type_params) = definition.type_params.as_deref() else {
            // Plain class arguments execute directly in the current scope. Generic class
            // arguments instead execute in the synthetic type-parameter scope handled below.
            if let Some(arguments) = definition.arguments.as_deref() {
                for argument in &arguments.args {
                    self.visit_expr(argument);
                }
                for keyword in &arguments.keywords {
                    self.visit_expr(&keyword.value);
                }
            }
            return;
        };
        self.facts.has_generic_definition = true;
        let type_names: BTreeSet<_> = type_params
            .iter()
            .map(|parameter| parameter.name().as_str().to_string())
            .collect();
        let mut required = type_parameter_required_names(type_params);
        if let Some(arguments) = definition.arguments.as_deref() {
            for argument in &arguments.args {
                required.extend(expression_required_names(argument));
            }
            for keyword in &arguments.keywords {
                required.extend(expression_required_names(&keyword.value));
            }
        }
        self.facts
            .nested_annotation_scope_requirements
            .extend(required.difference(&type_names).cloned());
    }

    fn collect_type_alias(&mut self, statement: &ruff_python_ast::StmtTypeAlias) {
        self.facts.has_type_alias = true;
        let mut required = expression_required_names(&statement.value);
        if let Some(type_params) = statement.type_params.as_deref() {
            let type_names: BTreeSet<_> = type_params
                .iter()
                .map(|parameter| parameter.name().as_str().to_string())
                .collect();
            required.extend(type_parameter_required_names(type_params));
            self.facts
                .nested_annotation_scope_requirements
                .extend(required.difference(&type_names).cloned());
        } else {
            self.facts
                .nested_annotation_scope_requirements
                .extend(required);
        }
    }

    fn insert_reference(&mut self, name: &str) {
        self.facts.references.insert(name.to_string());
        if name == "super" {
            self.facts.references.insert("__class__".to_string());
        } else if name == "__class__" {
            self.facts.explicit_dunder_class_reference = true;
        }
    }
}

impl<'ast> Visitor<'ast> for ScopeBodyFactsCollector {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        match statement {
            Stmt::FunctionDef(definition) => {
                // The child body is represented by a separate `ScopeArena` node and plan.
                self.collect_function_header(definition);
                return;
            }
            Stmt::ClassDef(definition) => {
                // As above, only the class header is evaluated in the current scope.
                self.collect_class_header(definition);
                return;
            }
            Stmt::TypeAlias(statement) => self.collect_type_alias(statement),
            Stmt::AnnAssign(annotation) => {
                self.facts.has_simple_annotations |= annotation.simple;
                if self.skip_annotations {
                    self.visit_expr(&annotation.target);
                    if let Some(value) = &annotation.value {
                        self.visit_expr(value);
                    }
                    return;
                }
            }
            Stmt::Global(statement) => self
                .facts
                .globals
                .extend(statement.names.iter().map(|name| name.as_str().to_string())),
            Stmt::Nonlocal(statement) => self
                .facts
                .nonlocals
                .extend(statement.names.iter().map(|name| name.as_str().to_string())),
            _ => {}
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let Expr::Lambda(lambda) = expression {
            self.facts
                .nested_lambda_requirements
                .extend(analyze_lambda_scope(lambda).required);
            return;
        }
        if let Expr::Name(name) = expression
            && name.ctx == ExprContext::Load
        {
            self.insert_reference(name.id.as_str());
        }

        let is_generator = matches!(expression, Expr::Generator(_));
        if self.collect_function_scope_facts && self.generator_depth == 0 {
            match expression {
                Expr::Generator(generator) => {
                    self.facts
                        .nested_generator_requirements
                        .extend(generator_required_names(generator));
                    self.facts
                        .generator_named_targets
                        .extend(generator_named_targets(generator));
                }
                Expr::ListComp(comprehension) => {
                    self.facts
                        .inlined_comprehension_cellvars
                        .extend(comprehension_cell_names(
                            &comprehension.generators,
                            None,
                            &comprehension.elt,
                        ));
                }
                Expr::SetComp(comprehension) => {
                    self.facts
                        .inlined_comprehension_cellvars
                        .extend(comprehension_cell_names(
                            &comprehension.generators,
                            None,
                            &comprehension.elt,
                        ));
                }
                Expr::DictComp(comprehension) => {
                    self.facts
                        .inlined_comprehension_cellvars
                        .extend(comprehension_cell_names(
                            &comprehension.generators,
                            comprehension.key.as_deref(),
                            &comprehension.value,
                        ));
                }
                _ => {}
            }
        }

        if is_generator {
            self.generator_depth += 1;
            walk_expr(self, expression);
            self.generator_depth -= 1;
        } else {
            walk_expr(self, expression);
        }
    }
}

#[derive(Default)]
pub(super) struct ReferenceCollector {
    pub(super) references: FxHashSet<String>,
    skip_annotations: bool,
    explicit_dunder_class_reference: bool,
}

impl<'ast> Visitor<'ast> for ReferenceCollector {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if matches!(statement, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
            return;
        }
        if self.skip_annotations
            && let Stmt::AnnAssign(annotation) = statement
        {
            self.visit_expr(&annotation.target);
            if let Some(value) = &annotation.value {
                self.visit_expr(value);
            }
            return;
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if matches!(expression, Expr::Lambda(_)) {
            return;
        }
        if let Expr::Name(name) = expression {
            if name.ctx == ExprContext::Load {
                self.references.insert(name.id.as_str().to_string());
                if name.id.as_str() == "super" {
                    self.references.insert("__class__".to_string());
                } else if name.id.as_str() == "__class__" {
                    self.explicit_dunder_class_reference = true;
                }
            }
        }
        walk_expr(self, expression);
    }
}

pub(super) fn definitely_evaluated_references(expression: &Expr) -> Option<FxHashSet<String>> {
    #[derive(Default)]
    struct ConditionalEvaluationDetector(bool);

    impl<'ast> Visitor<'ast> for ConditionalEvaluationDetector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(
                expression,
                Expr::BoolOp(_)
                    | Expr::If(_)
                    | Expr::Lambda(_)
                    | Expr::Generator(_)
                    | Expr::ListComp(_)
                    | Expr::SetComp(_)
                    | Expr::DictComp(_)
            ) || matches!(expression, Expr::Compare(comparison) if comparison.ops.len() > 1)
            {
                self.0 = true;
                return;
            }
            walk_expr(self, expression);
        }
    }

    let mut detector = ConditionalEvaluationDetector::default();
    detector.visit_expr(expression);
    if detector.0 {
        return None;
    }
    let mut references = ReferenceCollector::default();
    references.visit_expr(expression);
    Some(references.references)
}

pub(super) struct LambdaScopeAnalysis {
    pub(super) locals: Vec<String>,
    pub(super) cellvars: FxHashSet<String>,
    pub(super) inlined_comprehension_cellvars: FxHashSet<String>,
    pub(super) required: BTreeSet<String>,
}

pub(super) fn analyze_lambda_scope(lambda: &ruff_python_ast::ExprLambda) -> LambdaScopeAnalysis {
    let empty_parameters = ruff_python_ast::Parameters::default();
    let parameters = lambda.parameters.as_deref().unwrap_or(&empty_parameters);
    let mut locals = LocalCollector::default();
    for parameter in parameters
        .posonlyargs
        .iter()
        .chain(&parameters.args)
        .chain(&parameters.kwonlyargs)
    {
        locals.insert(parameter.name().as_str());
    }
    if let Some(parameter) = &parameters.vararg {
        locals.insert(parameter.name.as_str());
    }
    if let Some(parameter) = &parameters.kwarg {
        locals.insert(parameter.name.as_str());
    }
    locals.collect_expression(&lambda.body);

    let mut references = ReferenceCollector::default();
    references.visit_expr(&lambda.body);
    let mut nested_requirements = nested_lambda_required_names_in_expression(&lambda.body);
    nested_requirements.extend(nested_generator_required_names_in_expression(&lambda.body));
    let local_names = locals.names.iter().cloned().collect::<FxHashSet<_>>();
    let cellvars = nested_requirements
        .iter()
        .filter(|name| local_names.contains(*name))
        .cloned()
        .collect();
    let required = references
        .references
        .iter()
        .chain(&nested_requirements)
        .filter(|name| !local_names.contains(*name))
        .cloned()
        .collect();
    LambdaScopeAnalysis {
        locals: locals.names,
        cellvars,
        inlined_comprehension_cellvars: inlined_comprehension_cell_names_in_expression(
            &lambda.body,
        ),
        required,
    }
}

fn nested_lambda_required_names_in_expression(expression: &Expr) -> BTreeSet<String> {
    #[derive(Default)]
    struct Collector {
        required: BTreeSet<String>,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let Expr::Lambda(lambda) = expression {
                self.required.extend(analyze_lambda_scope(lambda).required);
                return;
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(expression);
    collector.required
}

fn nested_generator_required_names_in_expression(expression: &Expr) -> BTreeSet<String> {
    #[derive(Default)]
    struct Collector {
        required: BTreeSet<String>,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let Expr::Generator(generator) = expression {
                self.required.extend(generator_required_names(generator));
                return;
            }
            if matches!(expression, Expr::Lambda(_)) {
                return;
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(expression);
    collector.required
}

pub(super) fn expression_required_names(expression: &Expr) -> BTreeSet<String> {
    let mut references = ReferenceCollector::default();
    references.visit_expr(expression);
    let mut required = references.references.into_iter().collect::<BTreeSet<_>>();
    required.extend(nested_expression_required_names(expression));
    required
}

pub(super) fn nested_expression_required_names(expression: &Expr) -> BTreeSet<String> {
    let mut required = nested_lambda_required_names_in_expression(expression);
    required.extend(nested_generator_required_names_in_expression(expression));
    required
}

pub(super) fn type_parameter_required_names(
    type_params: &ruff_python_ast::TypeParams,
) -> BTreeSet<String> {
    let mut required = BTreeSet::new();
    for parameter in type_params {
        if let TypeParam::TypeVar(parameter) = parameter
            && let Some(bound) = &parameter.bound
        {
            required.extend(expression_required_names(bound));
        }
        if let Some(default) = parameter.default() {
            required.extend(expression_required_names(default));
        }
    }
    required
}

pub(super) fn class_static_attributes(body: &[Stmt]) -> Vec<String> {
    #[derive(Default)]
    struct Collector {
        attributes: BTreeSet<String>,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            // A nested class becomes the nearest enclosing class compiler for
            // any `self.attr` stores in its body.
            if matches!(statement, Stmt::ClassDef(_)) {
                return;
            }
            // CPython's augmented-assignment lowering and an annotation without
            // a value never pass the attribute through the ordinary Store path.
            if matches!(statement, Stmt::AugAssign(_))
                || matches!(statement, Stmt::AnnAssign(annotation) if annotation.value.is_none())
            {
                return;
            }
            walk_stmt(self, statement);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let Expr::Attribute(attribute) = expression
                && attribute.ctx == ExprContext::Store
                && matches!(
                    attribute.value.as_ref(),
                    Expr::Name(name) if name.id.as_str() == "self"
                )
            {
                self.attributes.insert(attribute.attr.as_str().to_string());
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    for statement in body {
        collector.visit_stmt(statement);
    }
    collector.attributes.into_iter().collect()
}

pub(super) fn collect_target_names(target: &Expr, names: &mut Vec<String>) {
    match target {
        Expr::Name(name) => names.push(name.id.as_str().to_string()),
        Expr::List(list) => {
            for element in &list.elts {
                collect_target_names(element, names);
            }
        }
        Expr::Tuple(tuple) => {
            for element in &tuple.elts {
                collect_target_names(element, names);
            }
        }
        Expr::Starred(starred) => collect_target_names(&starred.value, names),
        _ => {}
    }
}

pub(super) fn collect_nested_comprehension_target_names(
    expression: &Expr,
    names: &mut Vec<String>,
) {
    struct Collector<'a> {
        names: &'a mut Vec<String>,
    }

    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            let generators = match expression {
                Expr::ListComp(comprehension) => Some(comprehension.generators.as_slice()),
                Expr::SetComp(comprehension) => Some(comprehension.generators.as_slice()),
                Expr::DictComp(comprehension) => Some(comprehension.generators.as_slice()),
                Expr::Generator(_) | Expr::Lambda(_) => return,
                _ => None,
            };
            if let Some(generators) = generators {
                for generator in generators {
                    collect_target_names(&generator.target, self.names);
                }
            }
            walk_expr(self, expression);
        }
    }

    Collector { names }.visit_expr(expression);
}

pub(super) fn comprehension_cell_names(
    generators: &[ruff_python_ast::Comprehension],
    key: Option<&Expr>,
    value: &Expr,
) -> FxHashSet<String> {
    fn extend_requirements(expression: &Expr, required: &mut BTreeSet<String>) {
        required.extend(nested_lambda_required_names_in_expression(expression));
        required.extend(nested_generator_required_names_in_expression(expression));
    }

    let mut targets = Vec::new();
    let mut required = BTreeSet::new();
    for (index, generator) in generators.iter().enumerate() {
        collect_target_names(&generator.target, &mut targets);
        if index > 0 {
            collect_nested_comprehension_target_names(&generator.iter, &mut targets);
            extend_requirements(&generator.iter, &mut required);
        }
        for condition in &generator.ifs {
            collect_nested_comprehension_target_names(condition, &mut targets);
            extend_requirements(condition, &mut required);
        }
    }
    if let Some(key) = key {
        collect_nested_comprehension_target_names(key, &mut targets);
        extend_requirements(key, &mut required);
    }
    collect_nested_comprehension_target_names(value, &mut targets);
    extend_requirements(value, &mut required);

    let targets = targets.into_iter().collect::<FxHashSet<_>>();
    required
        .into_iter()
        .filter(|name| targets.contains(name))
        .collect()
}

fn inlined_comprehension_cell_names_in_expression(expression: &Expr) -> FxHashSet<String> {
    #[derive(Default)]
    struct Collector {
        names: FxHashSet<String>,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            match expression {
                Expr::ListComp(comprehension) => self.names.extend(comprehension_cell_names(
                    &comprehension.generators,
                    None,
                    &comprehension.elt,
                )),
                Expr::SetComp(comprehension) => self.names.extend(comprehension_cell_names(
                    &comprehension.generators,
                    None,
                    &comprehension.elt,
                )),
                Expr::DictComp(comprehension) => self.names.extend(comprehension_cell_names(
                    &comprehension.generators,
                    comprehension.key.as_deref(),
                    &comprehension.value,
                )),
                Expr::Lambda(_) | Expr::Generator(_) => return,
                _ => {}
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(expression);
    collector.names
}

pub(super) fn collect_named_expression_target_names(expression: &Expr, names: &mut Vec<String>) {
    struct Collector<'a> {
        names: &'a mut Vec<String>,
    }

    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let Expr::Named(named) = expression
                && let Expr::Name(name) = named.target.as_ref()
            {
                self.names.push(name.id.as_str().to_string());
            }
            if matches!(expression, Expr::Generator(_) | Expr::Lambda(_)) {
                return;
            }
            walk_expr(self, expression);
        }
    }

    Collector { names }.visit_expr(expression);
}

pub(super) fn generator_named_targets(
    generator: &ruff_python_ast::ExprGenerator,
) -> FxHashSet<String> {
    #[derive(Default)]
    struct Collector {
        names: FxHashSet<String>,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let Expr::Named(named) = expression
                && let Expr::Name(name) = named.target.as_ref()
            {
                self.names.insert(name.id.as_str().to_string());
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(&generator.elt);
    for comprehension in &generator.generators {
        collector.visit_expr(&comprehension.iter);
        for condition in &comprehension.ifs {
            collector.visit_expr(condition);
        }
    }
    collector.names
}

pub(super) fn generator_required_names(
    generator: &ruff_python_ast::ExprGenerator,
) -> BTreeSet<String> {
    let mut local_names = FxHashSet::default();
    for comprehension in &generator.generators {
        let mut names = Vec::new();
        collect_target_names(&comprehension.target, &mut names);
        local_names.extend(names);
    }

    let mut references = ReferenceCollector::default();
    references.visit_expr(&generator.elt);
    for (index, comprehension) in generator.generators.iter().enumerate() {
        // The outermost iterable is evaluated by the enclosing scope and passed
        // to the generator as its implicit `.0` argument.
        if index > 0 {
            references.visit_expr(&comprehension.iter);
        }
        for condition in &comprehension.ifs {
            references.visit_expr(condition);
        }
    }
    references
        .references
        .extend(generator_named_targets(generator));
    references
        .references
        .into_iter()
        .filter(|name| !local_names.contains(name))
        .collect()
}

pub(super) fn generator_cellvars(generator: &ruff_python_ast::ExprGenerator) -> FxHashSet<String> {
    let mut local_names = FxHashSet::default();
    for comprehension in &generator.generators {
        let mut names = Vec::new();
        collect_target_names(&comprehension.target, &mut names);
        local_names.extend(names);
    }

    let mut nested_requirements = nested_lambda_required_names_in_expression(&generator.elt);
    for (index, comprehension) in generator.generators.iter().enumerate() {
        if index > 0 {
            nested_requirements.extend(nested_lambda_required_names_in_expression(
                &comprehension.iter,
            ));
        }
        for condition in &comprehension.ifs {
            nested_requirements.extend(nested_lambda_required_names_in_expression(condition));
        }
    }
    nested_requirements
        .into_iter()
        .filter(|name| local_names.contains(name))
        .collect()
}
fn future_feature_flags(body: &[Stmt]) -> u32 {
    body.iter()
        .filter_map(|statement| {
            let Stmt::ImportFrom(import) = statement else {
                return None;
            };
            import
                .module
                .as_ref()
                .is_some_and(|module| module.as_str() == "__future__")
                .then_some(import)
        })
        .flat_map(|import| &import.names)
        .fold(0, |flags, alias| {
            flags
                | match alias.name.as_str() {
                    "barry_as_FLUFL" => CO_FUTURE_BARRY_AS_BDFL,
                    "annotations" => CO_FUTURE_ANNOTATIONS,
                    _ => 0,
                }
        })
}

pub(super) fn has_simple_annotations(body: &[Stmt]) -> bool {
    let mut annotations = Vec::new();
    collect_simple_annotations(body, &mut annotations);
    !annotations.is_empty()
}

pub(super) fn collect_conditional_simple_annotations<'a>(
    body: &'a [Stmt],
    annotations: &mut Vec<&'a ruff_python_ast::StmtAnnAssign>,
) {
    for statement in body {
        match statement {
            Stmt::If(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                for clause in &statement.elif_else_clauses {
                    collect_simple_annotations(&clause.body, annotations);
                }
            }
            Stmt::For(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                collect_simple_annotations(&statement.orelse, annotations);
            }
            Stmt::While(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                collect_simple_annotations(&statement.orelse, annotations);
            }
            Stmt::Try(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                collect_simple_annotations(&statement.orelse, annotations);
                for handler in &statement.handlers {
                    if let Some(handler) = handler.as_except_handler() {
                        collect_simple_annotations(&handler.body, annotations);
                    }
                }
                collect_simple_annotations(&statement.finalbody, annotations);
                // Try/finally lowers the final body once for the normal path and once for the
                // exceptional path, so both copies conditionally register their annotations.
                collect_simple_annotations(&statement.finalbody, annotations);
            }
            Stmt::With(statement) => collect_simple_annotations(&statement.body, annotations),
            Stmt::Match(statement) => {
                for case in &statement.cases {
                    collect_simple_annotations(&case.body, annotations);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn collect_simple_annotations<'a>(
    body: &'a [Stmt],
    annotations: &mut Vec<&'a ruff_python_ast::StmtAnnAssign>,
) {
    for statement in body {
        match statement {
            Stmt::AnnAssign(annotation) if annotation.simple => annotations.push(annotation),
            Stmt::If(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                for clause in &statement.elif_else_clauses {
                    collect_simple_annotations(&clause.body, annotations);
                }
            }
            Stmt::For(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                collect_simple_annotations(&statement.orelse, annotations);
            }
            Stmt::While(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                collect_simple_annotations(&statement.orelse, annotations);
            }
            Stmt::Try(statement) => {
                collect_simple_annotations(&statement.body, annotations);
                collect_simple_annotations(&statement.orelse, annotations);
                for handler in &statement.handlers {
                    if let Some(handler) = handler.as_except_handler() {
                        collect_simple_annotations(&handler.body, annotations);
                    }
                }
                collect_simple_annotations(&statement.finalbody, annotations);
                // Try/finally lowers the final body once for the normal path and once for the
                // exceptional path. CPython records deferred annotations during both copies.
                collect_simple_annotations(&statement.finalbody, annotations);
            }
            Stmt::With(statement) => {
                collect_simple_annotations(&statement.body, annotations);
            }
            Stmt::Match(statement) => {
                for case in &statement.cases {
                    collect_simple_annotations(&case.body, annotations);
                }
            }
            _ => {}
        }
    }
}
