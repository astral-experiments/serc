use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hasher;

use ruff_python_ast::name::Name;
use ruff_python_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use ruff_python_ast::{Expr, ExprContext, Pattern, Stmt, StmtClassDef, StmtFunctionDef, TypeParam};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};

use crate::CompileError;
use crate::target::code_flags::{CO_FUTURE_ANNOTATIONS, CO_FUTURE_BARRY_AS_BDFL};

use super::symbol::mangle_name;
use super::{Compiler, SHADOWED_SUPER_SENTINEL, Scope};

impl Compiler<'_> {
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

const SCOPE_SEED_NAMES: [&str; 26] = [
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
];

#[derive(Debug, Default)]
pub(super) struct NameInterner {
    names: Vec<Cow<'static, str>>,
    slots: Vec<Option<NameId>>,
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
    fn seeded(seeds: &[&'static str]) -> Self {
        let mut names = Vec::with_capacity(seeds.len());
        let mut slots = vec![None; seeds.len().saturating_mul(2).max(8).next_power_of_two()];
        for &name in seeds {
            let id = NameId(u32::try_from(names.len()).expect("too many interned names"));
            let mut slot = Self::hash(name) & (slots.len() - 1);
            while slots[slot].is_some() {
                slot = (slot + 1) & (slots.len() - 1);
            }
            slots[slot] = Some(id);
            names.push(Cow::Borrowed(name));
        }
        Self { names, slots }
    }

    pub(super) fn intern(&mut self, name: &str) -> NameId {
        if let Some(id) = self.get(name) {
            return id;
        }
        self.insert_owned(name.to_string())
    }

    fn intern_owned(&mut self, name: String) -> NameId {
        if let Some(id) = self.get(name.as_str()) {
            return id;
        }
        self.insert_owned(name)
    }

    fn insert_owned(&mut self, name: String) -> NameId {
        if self.names.len() * 2 >= self.slots.len() {
            let mut slots = vec![None; self.slots.len().max(8) * 2];
            for (index, name) in self.names.iter().enumerate() {
                let mut slot = Self::hash(name.as_ref()) & (slots.len() - 1);
                while slots[slot].is_some() {
                    slot = (slot + 1) & (slots.len() - 1);
                }
                slots[slot] = Some(NameId(
                    u32::try_from(index).expect("too many interned names"),
                ));
            }
            self.slots = slots;
        }
        let id = NameId(u32::try_from(self.names.len()).expect("too many interned names"));
        let mut slot = Self::hash(&name) & (self.slots.len() - 1);
        while self.slots[slot].is_some() {
            slot = (slot + 1) & (self.slots.len() - 1);
        }
        self.slots[slot] = Some(id);
        self.names.push(Cow::Owned(name));
        id
    }

    pub(super) fn get(&self, name: &str) -> Option<NameId> {
        if self.slots.is_empty() {
            return None;
        }
        let mut slot = Self::hash(name) & (self.slots.len() - 1);
        loop {
            let id = self.slots[slot]?;
            if self.names[usize::try_from(id.0).expect("name ID should fit in usize")].as_ref()
                == name
            {
                return Some(id);
            }
            slot = (slot + 1) & (self.slots.len() - 1);
        }
    }

    fn hash(name: &str) -> usize {
        let mut hasher = FxHasher::default();
        hasher.write(name.as_bytes());
        usize::try_from(hasher.finish()).expect("FxHasher output should fit in usize")
    }

    fn intern_set(&mut self, names: impl IntoIterator<Item = String>) -> FxHashSet<NameId> {
        names
            .into_iter()
            .map(|name| self.intern_owned(name))
            .collect()
    }

    pub(super) fn resolve(&self, id: NameId) -> &str {
        self.names[usize::try_from(id.0).expect("name ID should fit in usize")].as_ref()
    }

    pub(super) fn len(&self) -> usize {
        self.names.len()
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
    pub(super) comprehension_shadowed_targets: FxHashSet<NameId>,
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
    outside_inlined_comprehension_references: FxHashSet<String>,
    outside_inlined_comprehension_requirements: FxHashSet<String>,
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
    has_inlined_comprehension: bool,
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
            if local_names.contains(&name) && !self.comprehension_shadowed_targets.contains(&name) {
                self.cellvars.insert(name);
            } else if !self.globals.contains(&name) {
                self.required.insert(name);
            }
        }
        for name in annotation_references {
            if local_names.contains(&name) && !self.comprehension_shadowed_targets.contains(&name) {
                self.cellvars.insert(name);
            } else if !self.globals.contains(&name) {
                self.required.insert(name);
            }
        }
    }

    fn build(
        names: &mut NameInterner,
        definition: &StmtFunctionDef,
        module_parent: bool,
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
            outside_inlined_comprehension_references,
            outside_inlined_comprehension_requirements,
            explicit_dunder_class_reference,
            nested_lambda_requirements: lambda_requirements,
            nested_generator_requirements: generator_requirements,
            nested_annotation_scope_requirements: annotation_scope_requirements,
            generator_named_targets,
            inlined_comprehension_cellvars,
            has_inlined_comprehension,
            ..
        } = facts;

        // Record potential local loads and actual bindings in one source-order pass. Filtering the
        // potential names after the pass preserves CPython's load-before-store local ordering
        // without walking the body once just to discover its complete binding set.
        let shadowed_nonlocal_cells = inlined_comprehension_cellvars
            .iter()
            .filter(|name| {
                nonlocals.contains(*name)
                    && outside_inlined_comprehension_references.contains(*name)
            })
            .cloned()
            .collect();
        let mut locals = LocalCollector {
            candidate_bindings: Some(FxHashMap::default()),
            shadowed_nonlocal_cells,
            globals,
            has_inlined_comprehension,
            ignore_generator_body_comprehension_targets: generator_named_targets.is_empty(),
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
        let parameter_names = locals.names.iter().cloned().collect::<FxHashSet<_>>();
        locals.collect_suite(&definition.body);
        locals.retain_bindings();
        let comprehension_fast_bindings = locals.comprehension_iteration_targets.clone();
        let comprehension_target_predecessors = locals.comprehension_target_predecessors.clone();
        let comprehension_annotation_only_predecessors =
            locals.comprehension_annotation_only_predecessors.clone();

        // Child references in a module-level function resolve globally before comprehensions
        // are inlined. A temporary target cannot turn those references into closure captures.
        // Enclosing function bindings have different cell promotion during CPython's inlining.
        let comprehension_shadowed_targets = locals
            .comprehension_targets
            .iter()
            .filter(|name| {
                (outside_inlined_comprehension_references.contains(name.as_str())
                    || module_parent
                        && (outside_inlined_comprehension_requirements.contains(name.as_str())
                            || names.get(name.as_str()).is_some_and(|name| {
                                class_requirements.contains(&name)
                                    || nested_function_requirements.contains(&name)
                                    || nested_function_annotation_references.contains(&name)
                            })))
                    && !locals.non_comprehension_bindings.contains(name.as_str())
            })
            .cloned()
            .collect::<FxHashSet<_>>();
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
                || references.contains(name.as_str())
                    && (name != "__class__" || explicit_dunder_class_reference)
                || names
                    .get(name)
                    .is_some_and(|name| class_requirements.contains(&name))
                || lambda_requirements.contains(name.as_str())
                || generator_requirements.contains(name.as_str())
                || annotation_scope_requirements.contains(name.as_str())
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
        let is_closure_local = |name: &str| {
            local_names.contains(name) && !comprehension_shadowed_targets.contains(name)
        };
        let mut cellvars = generator_named_targets
            .into_iter()
            .filter(|name| is_closure_local(name))
            .collect::<FxHashSet<_>>();
        cellvars.extend(
            class_requirements
                .iter()
                .map(|name| names.resolve(*name))
                .filter(|name| is_closure_local(name))
                .map(str::to_string),
        );
        cellvars.extend(
            lambda_requirements
                .into_iter()
                .filter(|name| is_closure_local(name)),
        );
        cellvars.extend(
            generator_requirements
                .into_iter()
                .filter(|name| is_closure_local(name)),
        );
        cellvars.extend(
            annotation_scope_requirements
                .into_iter()
                .filter(|name| is_closure_local(name)),
        );
        // A captured comprehension target still needs a cell when an unrelated outer
        // reference resolves globally. Its cell is active only inside the comprehension.
        cellvars.extend(
            inlined_comprehension_cellvars
                .iter()
                .filter(|name| comprehension_shadowed_targets.contains(name.as_str()))
                .cloned(),
        );
        let retained_local_names: FxHashSet<_> = locals.names.iter().map(Name::as_str).collect();
        let required: BTreeSet<_> = references
            .iter()
            .chain(&locals.nonlocals)
            .filter(|name| {
                (comprehension_shadowed_targets.contains(name.as_str())
                    || !retained_local_names.contains(name.as_str()))
                    && !locals.globals.contains(*name)
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
                .map(|name| names.intern(name.as_str()))
                .collect(),
            globals: names.intern_set(locals.globals),
            annotation_references: names.intern_set(annotation_collector.references),
            cellvars: names.intern_set(cellvars),
            inlined_comprehension_cellvars: names.intern_set(inlined_comprehension_cellvars),
            freevars: Vec::new(),
            annotation_freevars: Vec::new(),
            comprehension_shadowed_targets: comprehension_shadowed_targets
                .into_iter()
                .map(|name| names.intern(name.as_str()))
                .collect(),
            required: names.intern_set(required),
            type_parameter_names: names.intern_set(type_parameter_names),
        };
        plan.absorb_child_requirements(
            nested_function_requirements.iter().copied(),
            nested_function_annotation_references.iter().copied(),
        );
        let comprehension_fast_cellvars = comprehension_fast_bindings
            .iter()
            .filter_map(|name| names.get(name))
            .filter(|name| plan.cellvars.contains(name))
            .collect::<Vec<_>>();
        let comprehension_reordered_fast_locals = comprehension_fast_bindings
            .iter()
            .filter(|name| !parameter_names.contains(*name))
            .filter_map(|name| {
                let id = names.get(name)?;
                // Global and cell operations do not reserve the fast-local slot used to
                // save a later comprehension temporary with the same name.
                (plan.comprehension_shadowed_targets.contains(&id) || plan.cellvars.contains(&id))
                    .then_some(id)
            })
            .collect::<Vec<_>>();
        let comprehension_fast_predecessors = comprehension_target_predecessors
            .into_iter()
            .filter_map(|(name, predecessors)| {
                let annotation_only_predecessors =
                    comprehension_annotation_only_predecessors.get(&name);
                Some((
                    names.get(&name)?,
                    predecessors
                        .into_iter()
                        .filter(|predecessor| {
                            annotation_only_predecessors.is_none_or(|annotation_only| {
                                !annotation_only.contains(predecessor)
                            })
                        })
                        .filter_map(|predecessor| names.get(&predecessor))
                        .collect(),
                ))
            })
            .collect();
        reorder_comprehension_fast_cells(
            &mut plan.locals,
            &comprehension_reordered_fast_locals,
            &comprehension_fast_predecessors,
        );
        plan.inlined_comprehension_cellvars
            .extend(comprehension_fast_cellvars);
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
        if self.locals.contains(&name) && !self.comprehension_shadowed_targets.contains(&name) {
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
    comprehension_cells: FxHashSet<NameId>,
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
    pub(super) comprehension_cells: FxHashSet<NameId>,
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
        // CPython retains this synthetic cell under future annotations even though
        // the annotation thunk does not capture it in that mode.
        let mut conditional_annotations = Vec::new();
        collect_conditional_simple_annotations(&definition.body, &mut conditional_annotations);
        let has_conditional_annotations = !conditional_annotations.is_empty();
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
            inlined_comprehension_cellvars,
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
                    || (!locals.globals.contains(name) && !locals.seen.contains(name.as_str()))
            })
            .collect::<BTreeSet<_>>();
        required.extend(locals.nonlocals.iter().cloned());
        body_explicitly_references_dunder_class |=
            annotation_scope_requirements.contains("__class__");
        required.extend(
            annotation_scope_requirements.into_iter().filter(|name| {
                !locals.seen.contains(name.as_str()) && !locals.globals.contains(name)
            }),
        );
        let body_requires_dunder_class =
            body_explicitly_references_dunder_class && required.contains("__class__");

        required.extend(nested_lambda_requirements.iter().cloned());
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
            .map(|name| names.intern(name.as_str()))
            .collect();
        Self {
            globals: names.intern_set(locals.globals),
            nonlocals: names.intern_set(locals.nonlocals),
            bound_names: locals
                .seen
                .into_iter()
                .map(|name| names.intern(name.as_str()))
                .collect(),
            comprehension_targets,
            comprehension_cells: names.intern_set(inlined_comprehension_cellvars),
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
            comprehension_cells: self.comprehension_cells.clone(),
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
    module: ModulePlan,
    nodes: Vec<ScopeNode>,
    by_parent_and_origin: FxHashMap<(ScopeId, ScopeOrigin), ScopeId>,
}

#[derive(Debug, Default)]
pub(super) struct ModulePlan {
    pub(super) future_flags: u32,
    pub(super) module_globals: NameSet,
    pub(super) imported_scope_names: NameSet,
    pub(super) comprehension_targets: Vec<NameId>,
    pub(super) comprehension_cell_names: NameSet,
    pub(super) has_annotations: bool,
    pub(super) has_simple_annotations: bool,
    pub(super) has_yield_expressions: bool,
}

#[derive(Debug, Default)]
struct ModulePlanBuilder {
    module_globals: FxHashSet<String>,
    imported_scope_names: FxHashSet<String>,
    comprehension_cell_names: FxHashSet<String>,
    has_annotations: bool,
    has_simple_annotations: bool,
    has_yield_expressions: bool,
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
                .map(|name| {
                    names
                        .get(name.as_str())
                        .unwrap_or_else(|| panic!("module analysis did not intern `{name}`"))
                })
                .collect(),
            comprehension_cell_names: facts
                .comprehension_cell_names
                .drain()
                .map(name_id)
                .collect(),
            has_annotations: facts.has_annotations,
            has_simple_annotations: facts.has_simple_annotations,
            has_yield_expressions: facts.has_yield_expressions,
        }
    }
}

impl Default for ScopeArena {
    fn default() -> Self {
        Self {
            names: NameInterner::default(),
            module: ModulePlan::default(),
            nodes: vec![ScopeNode {
                plan: ScopePlan::Module,
                children: Vec::new(),
            }],
            by_parent_and_origin: FxHashMap::default(),
        }
    }
}

impl ScopeArena {
    pub(super) fn analyze(body: &[Stmt]) -> Self {
        let future_flags = future_feature_flags(body);
        let future_annotations = future_flags & CO_FUTURE_ANNOTATIONS != 0;
        let mut arena = Self {
            names: NameInterner::seeded(&SCOPE_SEED_NAMES),
            ..Self::default()
        };
        // The name walk also records scope definitions in preorder. Allocating their nodes first
        // and then finishing plans in reverse preserves lexical child order and closure
        // requirements without walking the statement tree a second time.
        let (facts, definitions, requires_module_bindings) =
            ModuleNameCollector::collect(&mut arena.names, body);
        let mut bindings = LocalCollector::default();
        if requires_module_bindings {
            bindings.collect_suite(body);
        }
        arena.build_scopes(definitions, future_annotations);
        arena.module = ModulePlan::build(&arena.names, future_flags, facts, bindings);
        arena
    }

    pub(super) const fn module(&self) -> &ModulePlan {
        &self.module
    }

    fn build_scopes(&mut self, definitions: Vec<ScopeBuildInput<'_>>, future_annotations: bool) {
        self.nodes.reserve(definitions.len());
        self.by_parent_and_origin.reserve(definitions.len());
        for (index, input) in definitions.iter().enumerate() {
            let (origin, plan) = match input.definition {
                ScopeDefinition::Function(definition, _) => (
                    ScopeOrigin::function(definition),
                    ScopePlan::Function(FunctionPlan::default()),
                ),
                ScopeDefinition::Class(definition) => (
                    ScopeOrigin::class(definition),
                    ScopePlan::Class(ClassPlan::default()),
                ),
            };
            let id = self.add_scope(input.parent, origin, plan);
            debug_assert_eq!(id.0, index + 1);
        }

        for (index, input) in definitions.into_iter().enumerate().rev() {
            let id = ScopeId(index + 1);
            match input.definition {
                ScopeDefinition::Function(definition, flat) => {
                    self.build_function(id, input.parent, definition, flat, future_annotations);
                }
                ScopeDefinition::Class(definition) => {
                    self.build_class(id, definition, future_annotations);
                }
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
        id: ScopeId,
        parent: ScopeId,
        definition: &StmtFunctionDef,
        flat: Option<FlatFunctionInputs>,
        future_annotations: bool,
    ) {
        // Flat functions have no closure, annotation, or evaluation-order edge cases. Their
        // bindings and references were already collected during the module-wide name walk.
        if let Some(inputs) = flat {
            let mut locals = Vec::with_capacity(inputs.names.len());
            let mut required = FxHashSet::default();
            required.reserve(inputs.names.len());
            for (name, bound) in inputs.names {
                if bound {
                    locals.push(name);
                } else {
                    required.insert(name);
                }
            }
            self.nodes[id.0].plan = ScopePlan::Function(FunctionPlan {
                locals,
                required,
                ..FunctionPlan::default()
            });
            return;
        }

        let facts = ScopeBodyFacts::for_function(&definition.body);
        let (nested_function_requirements, nested_function_annotation_references) =
            self.nested_function_requirements(id);
        let class_requirements = self.nested_class_requirements(id);
        let plan = FunctionPlan::build(
            &mut self.names,
            definition,
            parent == ScopeId::MODULE,
            future_annotations,
            facts,
            &nested_function_requirements,
            &nested_function_annotation_references,
            &class_requirements,
        );
        self.nodes[id.0].plan = ScopePlan::Function(plan);
    }

    fn build_class(&mut self, id: ScopeId, definition: &StmtClassDef, future_annotations: bool) {
        let facts = ScopeBodyFacts::for_class(&definition.body, future_annotations);
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

struct ScopeBuildInput<'ast> {
    parent: ScopeId,
    definition: ScopeDefinition<'ast>,
}

enum ScopeDefinition<'ast> {
    Function(&'ast StmtFunctionDef, Option<FlatFunctionInputs>),
    Class(&'ast StmtClassDef),
}

struct ModuleNameCollector<'a, 'ast> {
    names: &'a mut NameInterner,
    private_name: Option<String>,
    facts: ModulePlanBuilder,
    nested_scope_depth: usize,
    comprehension_depth: usize,
    generator_depth: usize,
    current_scope: ScopeId,
    in_module_header: bool,
    requires_module_bindings: bool,
    flat_function: Option<FlatFunctionInputs>,
    definitions: Vec<ScopeBuildInput<'ast>>,
}

#[derive(Debug)]
struct FlatFunctionInputs {
    // Preserve the first runtime occurrence of each name; a later store upgrades the existing
    // entry without moving it, matching CPython's load-before-store local ordering.
    names: Vec<(NameId, bool)>,
    name_positions: Option<FxHashMap<NameId, usize>>,
    complex: bool,
}

impl FlatFunctionInputs {
    const LINEAR_SEARCH_LIMIT: usize = 8;

    fn record(&mut self, name: NameId, bound: bool) {
        if self.complex {
            return;
        }

        if let Some(name_positions) = &mut self.name_positions {
            if let Some(position) = name_positions.get(&name).copied() {
                self.names[position].1 |= bound;
            } else {
                name_positions.insert(name, self.names.len());
                self.names.push((name, bound));
            }
            return;
        }

        if let Some((_, was_bound)) = self
            .names
            .iter_mut()
            .find(|(existing, _)| *existing == name)
        {
            *was_bound |= bound;
            return;
        }

        if self.names.len() < Self::LINEAR_SEARCH_LIMIT {
            self.names.push((name, bound));
            return;
        }

        let mut name_positions = FxHashMap::default();
        name_positions.reserve(self.names.len() + 1);
        name_positions.extend(
            self.names
                .iter()
                .enumerate()
                .map(|(position, (name, _))| (*name, position)),
        );
        name_positions.insert(name, self.names.len());
        self.names.push((name, bound));
        self.name_positions = Some(name_positions);
    }

    fn mark_complex(&mut self) {
        if self.complex {
            return;
        }

        self.complex = true;
        self.names.clear();
        self.name_positions = None;
    }
}

impl<'a, 'ast> ModuleNameCollector<'a, 'ast> {
    fn collect(
        names: &'a mut NameInterner,
        body: &'ast [Stmt],
    ) -> (ModulePlanBuilder, Vec<ScopeBuildInput<'ast>>, bool) {
        let mut collector = Self {
            names,
            private_name: None,
            facts: ModulePlanBuilder::default(),
            nested_scope_depth: 0,
            comprehension_depth: 0,
            generator_depth: 0,
            current_scope: ScopeId::MODULE,
            in_module_header: false,
            requires_module_bindings: false,
            flat_function: None,
            definitions: Vec::new(),
        };
        collector.collect_suite(body);
        (
            collector.facts,
            collector.definitions,
            collector.requires_module_bindings,
        )
    }

    fn intern(&mut self, name: &str) -> NameId {
        // A module binding named `super` disables the specialized zero-argument form. Be
        // conservative about nested/header occurrences so the module-local walk can be skipped
        // only when the sentinel is definitely irrelevant.
        self.requires_module_bindings |= name == "super";
        if self.private_name.is_none() {
            return self.names.intern(name);
        }

        let mangled = mangle_name(name, self.private_name.as_deref());
        let id = self.names.intern(name);
        if let Cow::Owned(mangled) = mangled {
            self.names.intern_owned(mangled);
        }
        id
    }

    fn record_flat(&mut self, name: NameId, bound: bool) {
        if let Some(flat) = &mut self.flat_function {
            flat.record(name, bound);
        }
    }

    fn record_flat_name(&mut self, name: &ruff_python_ast::ExprName) {
        let spelling = name.id.as_str();
        let id = self.intern(spelling);
        if spelling == "super" {
            self.mark_flat_complex();
        }
        self.record_flat(id, name.ctx != ExprContext::Load);
    }

    fn mark_flat_complex(&mut self) {
        if let Some(flat) = &mut self.flat_function {
            flat.mark_complex();
        }
    }

    fn collect_suite(&mut self, body: &'ast [Stmt]) {
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

impl<'ast> Visitor<'ast> for ModuleNameCollector<'_, 'ast> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        let at_module_scope = self.nested_scope_depth == 0;
        if matches!(
            statement,
            Stmt::AugAssign(_)
                | Stmt::AnnAssign(_)
                | Stmt::Try(_)
                | Stmt::Match(_)
                | Stmt::Global(_)
                | Stmt::Nonlocal(_)
                | Stmt::TypeAlias(_)
                | Stmt::ClassDef(_)
        ) {
            self.mark_flat_complex();
        }
        match statement {
            Stmt::FunctionDef(definition) => {
                self.mark_flat_complex();
                self.intern(definition.name.as_str());
                let parent = self.current_scope;
                let scope = ScopeId(self.definitions.len() + 1);
                self.definitions.push(ScopeBuildInput {
                    parent,
                    definition: ScopeDefinition::Function(definition, None),
                });
                self.current_scope = scope;
                let outer_flat = self.flat_function.take();
                let mut flat = FlatFunctionInputs {
                    names: Vec::with_capacity(8),
                    name_positions: None,
                    complex: definition.type_params.is_some()
                        || definition.returns.is_some()
                        || definition
                            .parameters
                            .iter()
                            .any(|parameter| parameter.annotation().is_some()),
                };
                for parameter in definition
                    .parameters
                    .posonlyargs
                    .iter()
                    .chain(&definition.parameters.args)
                    .chain(&definition.parameters.kwonlyargs)
                {
                    flat.record(self.intern(parameter.name().as_str()), true);
                }
                if let Some(parameter) = &definition.parameters.vararg {
                    flat.record(self.intern(parameter.name.as_str()), true);
                }
                if let Some(parameter) = &definition.parameters.kwarg {
                    flat.record(self.intern(parameter.name.as_str()), true);
                }
                self.collect_type_parameters(definition.type_params.as_deref());
                self.nested_scope_depth += 1;
                let outer_header = self.in_module_header;
                self.in_module_header |= at_module_scope;
                for decorator in &definition.decorator_list {
                    if at_module_scope {
                        self.facts.comprehension_cell_names.extend(
                            inlined_comprehension_cell_names_in_expression(&decorator.expression),
                        );
                    }
                    self.visit_decorator(decorator);
                }
                if at_module_scope {
                    for default in function_default_expressions(definition) {
                        self.facts
                            .comprehension_cell_names
                            .extend(inlined_comprehension_cell_names_in_expression(default));
                    }
                }
                if let Some(type_params) = definition.type_params.as_deref() {
                    self.visit_type_params(type_params);
                }
                self.visit_parameters(&definition.parameters);
                if let Some(returns) = definition.returns.as_deref() {
                    self.visit_annotation(returns);
                }
                self.in_module_header = outer_header;
                self.flat_function = Some(flat);
                self.collect_suite(&definition.body);
                self.nested_scope_depth -= 1;
                let flat = self
                    .flat_function
                    .take()
                    .expect("function name collection should retain its flat inputs");
                if !flat.complex {
                    let ScopeDefinition::Function(_, inputs) =
                        &mut self.definitions[scope.0 - 1].definition
                    else {
                        unreachable!("function definition changed while collecting its body");
                    };
                    *inputs = Some(flat);
                }
                self.flat_function = outer_flat;
                self.current_scope = parent;
                return;
            }
            Stmt::ClassDef(definition) => {
                self.intern(definition.name.as_str());
                let parent = self.current_scope;
                self.current_scope = ScopeId(self.definitions.len() + 1);
                self.definitions.push(ScopeBuildInput {
                    parent,
                    definition: ScopeDefinition::Class(definition),
                });
                self.nested_scope_depth += 1;
                let outer_header = self.in_module_header;
                self.in_module_header |= at_module_scope;
                for decorator in &definition.decorator_list {
                    if at_module_scope {
                        self.facts.comprehension_cell_names.extend(
                            inlined_comprehension_cell_names_in_expression(&decorator.expression),
                        );
                    }
                    self.visit_decorator(decorator);
                }
                // A generic class header executes in its own annotation scope, whose
                // private-name context is the class being defined. Decorators still
                // belong to the enclosing scope.
                let outer_private_name = self.private_name.clone();
                if definition.type_params.is_some() {
                    self.private_name = Some(definition.name.as_str().to_string());
                }
                self.collect_type_parameters(definition.type_params.as_deref());
                if let Some(type_params) = definition.type_params.as_deref() {
                    self.visit_type_params(type_params);
                }
                if let Some(arguments) = definition.arguments.as_deref() {
                    if at_module_scope && definition.type_params.is_none() {
                        for argument in &arguments.args {
                            self.facts
                                .comprehension_cell_names
                                .extend(inlined_comprehension_cell_names_in_expression(argument));
                        }
                        for keyword in &arguments.keywords {
                            self.facts.comprehension_cell_names.extend(
                                inlined_comprehension_cell_names_in_expression(&keyword.value),
                            );
                        }
                    }
                    self.visit_arguments(arguments);
                }
                self.in_module_header = outer_header;
                self.private_name = Some(definition.name.as_str().to_string());
                self.collect_suite(&definition.body);
                self.private_name = outer_private_name;
                self.nested_scope_depth -= 1;
                self.current_scope = parent;
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
                    let id = self.intern(bound_name);
                    self.record_flat(id, true);
                    if at_module_scope {
                        self.facts
                            .imported_scope_names
                            .insert(bound_name.to_string());
                    }
                }
            }
            Stmt::ImportFrom(statement) => {
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
                        let id = self.intern(bound_name);
                        self.record_flat(id, true);
                        // CPython's method-call optimization queries DEF_IMPORT on the module
                        // symbol table, where future feature names and aliases remain imports.
                        if at_module_scope {
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
                self.record_flat_name(name);
                return;
            }
            Expr::Attribute(attribute) => {
                self.intern(attribute.attr.as_str());
                if let Expr::Name(name) = attribute.value.as_ref() {
                    self.record_flat_name(name);
                } else {
                    self.visit_expr(&attribute.value);
                }
                return;
            }
            Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::IpyEscapeCommand(_) => return,
            _ => {}
        }

        self.facts.has_yield_expressions |=
            matches!(expression, Expr::Yield(_) | Expr::YieldFrom(_));
        self.requires_module_bindings |= (self.nested_scope_depth == 0 || self.in_module_header)
            && matches!(
                expression,
                Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_)
            );
        if matches!(
            expression,
            Expr::Lambda(_)
                | Expr::Generator(_)
                | Expr::ListComp(_)
                | Expr::SetComp(_)
                | Expr::DictComp(_)
                | Expr::Named(_)
        ) {
            self.mark_flat_complex();
        }
        if let Expr::Lambda(lambda) = expression {
            if let Some(parameters) = lambda.parameters.as_deref() {
                self.collect_parameters(parameters);
            }
            self.nested_scope_depth += 1;
            let outer_header = self.in_module_header;
            self.in_module_header |= self.nested_scope_depth == 1;
            walk_expr(self, expression);
            self.in_module_header = outer_header;
            self.nested_scope_depth -= 1;
            return;
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
    parameter_default_expressions(&definition.parameters)
}

fn parameter_default_expressions(
    parameters: &ruff_python_ast::Parameters,
) -> impl Iterator<Item = &Expr> {
    parameters
        .posonlyargs
        .iter()
        .chain(&parameters.args)
        .filter_map(|parameter| parameter.default.as_deref())
        .chain(
            parameters
                .kwonlyargs
                .iter()
                .filter_map(|parameter| parameter.default.as_deref()),
        )
}

fn visit_lambda_defaults<'ast>(
    visitor: &mut impl Visitor<'ast>,
    lambda: &'ast ruff_python_ast::ExprLambda,
) {
    if let Some(parameters) = lambda.parameters.as_deref() {
        for default in parameter_default_expressions(parameters) {
            visitor.visit_expr(default);
        }
    }
}

#[derive(Clone, Copy)]
enum LocalUnwind<'a> {
    Finally(&'a [Stmt]),
    Loop,
}

#[derive(Debug, Default)]
struct LocalCollector {
    names: Vec<Name>,
    seen: FxHashSet<Name>,
    comprehension_targets: Vec<Name>,
    comprehension_iteration_targets: Vec<Name>,
    seen_comprehension_iteration_targets: FxHashSet<Name>,
    comprehension_annotation_only_predecessors: FxHashMap<Name, Vec<Name>>,
    seen_comprehension_targets: FxHashSet<Name>,
    comprehension_late_bindings: FxHashSet<Name>,
    comprehension_target_predecessors: FxHashMap<Name, Vec<Name>>,
    shadowed_nonlocal_cells: FxHashSet<String>,
    non_comprehension_bindings: FxHashSet<Name>,
    candidate_bindings: Option<FxHashMap<Name, bool>>,
    annotation_only: FxHashSet<Name>,
    globals: FxHashSet<String>,
    nonlocals: FxHashSet<String>,
    has_inlined_comprehension: bool,
    ignore_generator_body_comprehension_targets: bool,
}

fn reorder_comprehension_fast_cells<T>(
    locals: &mut Vec<T>,
    fast_cells: &[T],
    target_predecessors: &FxHashMap<T, Vec<T>>,
) where
    T: Clone + Eq + std::hash::Hash,
{
    let mut placed = FxHashSet::default();
    for cell in fast_cells {
        locals.retain(|candidate| candidate != cell);
        let mut predecessors = target_predecessors
            .get(cell)
            .into_iter()
            .flatten()
            .cloned()
            .collect::<FxHashSet<_>>();
        predecessors.extend(placed.iter().cloned());
        let index = locals
            .iter()
            .rposition(|candidate| predecessors.contains(candidate))
            .map_or(0, |index| index + 1);
        locals.insert(index, cell.clone());
        placed.insert(cell.clone());
    }
}

impl LocalCollector {
    fn deferred_nonlocal_comprehension_names(
        &self,
        generators: &[ruff_python_ast::Comprehension],
        key: Option<&Expr>,
        value: &Expr,
    ) -> FxHashSet<String> {
        if self.nonlocals.is_empty() {
            return FxHashSet::default();
        }
        let captured = comprehension_cell_names(generators, key, value);
        comprehension_name_bindings(generators, key, value)
            .into_iter()
            .filter_map(|(name, bound)| {
                (!bound
                    && self.nonlocals.contains(&name)
                    && (!captured.contains(&name) || self.shadowed_nonlocal_cells.contains(&name)))
                .then_some(name)
            })
            .collect()
    }

    /// Reserve only actual scope saves when a nested comprehension runs in a first iterable.
    fn collect_nested_comprehension_saves(&self, expression: &Expr, names: &mut Vec<String>) {
        struct Collector<'a> {
            locals: &'a LocalCollector,
            names: &'a mut Vec<String>,
        }

        impl<'ast> Visitor<'ast> for Collector<'_> {
            fn visit_expr(&mut self, expression: &'ast Expr) {
                let (generators, key, value) = match expression {
                    Expr::ListComp(comprehension) => {
                        (&comprehension.generators, None, comprehension.elt.as_ref())
                    }
                    Expr::SetComp(comprehension) => {
                        (&comprehension.generators, None, comprehension.elt.as_ref())
                    }
                    Expr::DictComp(comprehension) => (
                        &comprehension.generators,
                        comprehension.key.as_deref(),
                        comprehension.value.as_ref(),
                    ),
                    Expr::Generator(generator) => {
                        if let Some(first) = generator.generators.first() {
                            self.visit_expr(&first.iter);
                        }
                        return;
                    }
                    Expr::Lambda(lambda) => {
                        visit_lambda_defaults(self, lambda);
                        return;
                    }
                    _ => {
                        walk_expr(self, expression);
                        return;
                    }
                };
                if let Some(first) = generators.first() {
                    self.visit_expr(&first.iter);
                }
                let deferred = self
                    .locals
                    .deferred_nonlocal_comprehension_names(generators, key, value);
                self.names.extend(
                    comprehension_temporary_names(generators, key, value)
                        .into_iter()
                        .filter(|name| !deferred.contains(name)),
                );
                // The first iterable was visited before this scope's saves. Walk only
                // the remaining runtime expressions so nested first iterables run once.
                for (index, generator) in generators.iter().enumerate() {
                    if index > 0 {
                        self.visit_expr(&generator.iter);
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
            }
        }

        Collector {
            locals: self,
            names,
        }
        .visit_expr(expression);
    }

    fn insert_comprehension_target(&mut self, name: Name) {
        if self.seen_comprehension_targets.insert(name.clone()) {
            self.comprehension_annotation_only_predecessors
                .insert(name.clone(), self.annotation_only.iter().cloned().collect());
            if self.non_comprehension_bindings.contains(&name) {
                self.comprehension_late_bindings.insert(name.clone());
            }
            self.comprehension_target_predecessors.insert(
                name.clone(),
                self.names
                    .iter()
                    .filter(|predecessor| *predecessor != &name)
                    .cloned()
                    .collect(),
            );
            self.comprehension_targets.push(name);
        }
    }
    fn insert_comprehension_iteration_target(&mut self, name: Name) {
        if !self.seen_comprehension_iteration_targets.contains(&name)
            && self.seen_comprehension_targets.contains(&name)
        {
            // A containing-scope walrus store can precede the first inlined save of the
            // same name. Only the latter reserves its temporary fast-local position.
            self.comprehension_target_predecessors.insert(
                name.clone(),
                self.names
                    .iter()
                    .filter(|predecessor| *predecessor != &name)
                    .cloned()
                    .collect(),
            );
        }
        self.insert_comprehension_target(name.clone());
        if self
            .seen_comprehension_iteration_targets
            .insert(name.clone())
        {
            self.comprehension_iteration_targets.push(name);
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
            candidate_bindings.insert(Name::new(name), true);
            self.names.push(Name::new(name));
            return true;
        }
        if self.seen.insert(Name::new(name)) {
            self.names.push(Name::new(name));
            true
        } else {
            false
        }
    }

    fn insert_candidate(&mut self, name: &str) {
        if self.globals.contains(name) || self.nonlocals.contains(name) {
            return;
        }
        if self
            .candidate_bindings
            .as_ref()
            .is_some_and(|candidate_bindings| candidate_bindings.contains_key(name))
        {
            self.claim_annotation_only_position(name);
            return;
        }
        let Some(candidate_bindings) = &mut self.candidate_bindings else {
            return;
        };
        candidate_bindings.insert(Name::new(name), false);
        self.names.push(Name::new(name));
    }

    fn claim_annotation_only_position(&mut self, name: &str) {
        // A no-value function annotation emits no fast-local operation in CPython. Keep its
        // symbol-table position provisional until the first runtime load or binding claims it.
        if !self.annotation_only.remove(name) {
            return;
        }
        let index = self
            .names
            .iter()
            .position(|candidate| candidate == name)
            .expect("an annotation-only local should have a candidate position");
        let name = self.names.remove(index);
        self.names.push(name);
    }

    fn retain_bindings(&mut self) {
        let Some(candidate_bindings) = self.candidate_bindings.take() else {
            return;
        };
        self.names.retain(|name| candidate_bindings[name]);
    }

    fn insert(&mut self, name: &str) {
        // A nonlocal store does not bind the hidden fast local used by a later comprehension.
        if !self.nonlocals.contains(name) {
            self.non_comprehension_bindings.insert(Name::new(name));
        }
        self.claim_annotation_only_position(name);
        self.mark_binding(name);
    }

    fn insert_annotation_only(&mut self, name: &str) {
        self.non_comprehension_bindings.insert(Name::new(name));
        if self.mark_binding(name) {
            self.annotation_only.insert(Name::new(name));
        }
    }

    fn collect_suite(&mut self, body: &[Stmt]) {
        self.collect_suite_with_unwind(body, &[]);
    }

    fn collect_suite_with_unwind<'a>(&mut self, body: &'a [Stmt], unwind: &[LocalUnwind<'a>]) {
        for statement in body {
            match statement {
                Stmt::Assign(assignment) => {
                    if self.candidate_bindings.is_none() || !self.has_inlined_comprehension {
                        self.collect_expression(&assignment.value);
                    } else {
                        let mut bindings = FxHashSet::default();
                        for target in &assignment.targets {
                            let mut names = Vec::new();
                            collect_target_names(target, &mut names);
                            bindings.extend(names.into_iter().map(Name::from));
                        }
                        self.collect_assignment_expression(&assignment.value, bindings);
                    }
                    for target in &assignment.targets {
                        self.collect_target(target);
                    }
                }
                Stmt::AugAssign(assignment) => {
                    self.collect_target(&assignment.target);
                    self.collect_expression(&assignment.value);
                }
                Stmt::AnnAssign(assignment) => {
                    if let Some(value) = &assignment.value {
                        if self.candidate_bindings.is_none() || !self.has_inlined_comprehension {
                            self.collect_expression(value);
                        } else {
                            let mut names = Vec::new();
                            collect_target_names(&assignment.target, &mut names);
                            self.collect_assignment_expression(
                                value,
                                names.into_iter().map(Name::from).collect(),
                            );
                        }
                        self.collect_target(&assignment.target);
                    } else {
                        match assignment.target.as_ref() {
                            Expr::Name(name) if assignment.simple => {
                                self.insert_annotation_only(name.id.as_str());
                            }
                            Expr::Name(_) => {}
                            target => self.collect_target(target),
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
                    let mut loop_unwind = unwind.to_vec();
                    loop_unwind.push(LocalUnwind::Loop);
                    self.collect_suite_with_unwind(&statement.body, &loop_unwind);
                    self.collect_suite_with_unwind(&statement.orelse, unwind);
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
                    self.collect_suite_with_unwind(&statement.body, unwind);
                    for clause in &statement.elif_else_clauses {
                        if let Some(test) = &clause.test {
                            self.collect_expression(test);
                        }
                        self.collect_suite_with_unwind(&clause.body, unwind);
                    }
                }
                Stmt::While(statement) => {
                    self.collect_expression(&statement.test);
                    let mut loop_unwind = unwind.to_vec();
                    loop_unwind.push(LocalUnwind::Loop);
                    self.collect_suite_with_unwind(&statement.body, &loop_unwind);
                    self.collect_suite_with_unwind(&statement.orelse, unwind);
                }
                Stmt::Try(statement) => {
                    let mut finally_unwind = unwind.to_vec();
                    if !statement.finalbody.is_empty() {
                        finally_unwind.push(LocalUnwind::Finally(&statement.finalbody));
                    }
                    self.collect_suite_with_unwind(&statement.body, &finally_unwind);
                    // CPython emits the normal else path before moving exception handlers into
                    // cold blocks, so locals first bound there precede handler-only locals.
                    self.collect_suite_with_unwind(&statement.orelse, &finally_unwind);
                    for handler in &statement.handlers {
                        if let Some(handler) = handler.as_except_handler() {
                            if let Some(exception_type) = &handler.type_ {
                                self.collect_expression(exception_type);
                            }
                            if let Some(name) = &handler.name {
                                self.insert(name.as_str());
                            }
                            self.collect_suite_with_unwind(&handler.body, &finally_unwind);
                        }
                    }
                    self.collect_suite_with_unwind(&statement.finalbody, unwind);
                }
                Stmt::With(statement) => {
                    for item in &statement.items {
                        self.collect_expression(&item.context_expr);
                        if let Some(target) = &item.optional_vars {
                            self.collect_target(target);
                        }
                    }
                    self.collect_suite_with_unwind(&statement.body, unwind);
                }
                Stmt::Match(statement) => {
                    self.collect_expression(&statement.subject);
                    for case in &statement.cases {
                        self.collect_pattern(&case.pattern);
                        if let Some(guard) = &case.guard {
                            self.collect_expression(guard);
                        }
                        self.collect_suite_with_unwind(&case.body, unwind);
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
                    self.collect_unwind(unwind, false);
                }
                Stmt::Break(_) | Stmt::Continue(_) => self.collect_unwind(unwind, true),
                _ => {}
            }
        }
    }

    fn collect_unwind(&mut self, unwind: &[LocalUnwind<'_>], loop_exit: bool) {
        // CPython allocates fast locals when it first emits their operations. Cleanup copied
        // onto an early exit therefore claims its local positions before the remaining body.
        for (index, frame) in unwind.iter().enumerate().rev() {
            match frame {
                LocalUnwind::Finally(body) => {
                    self.collect_suite_with_unwind(body, &unwind[..index]);
                }
                LocalUnwind::Loop if loop_exit => break,
                LocalUnwind::Loop => {}
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
            Expr::Attribute(attribute) => self.collect_expression(&attribute.value),
            Expr::Subscript(subscript) => {
                self.collect_expression(&subscript.value);
                self.collect_expression(&subscript.slice);
            }
            _ => {}
        }
    }

    fn collect_expression(&mut self, expression: &Expr) {
        NamedExpressionCollector {
            collector: self,
            in_inlined_comprehension: false,
            in_generator_body: false,
            collect_known_loads: true,
            named_target_after_value: false,
            known_assignment_bindings: FxHashSet::default(),
            ignored_candidate_loads: FxHashSet::default(),
        }
        .visit_expr(expression);
    }

    fn collect_assignment_expression(
        &mut self,
        expression: &Expr,
        known_assignment_bindings: FxHashSet<Name>,
    ) {
        NamedExpressionCollector {
            collector: self,
            in_inlined_comprehension: false,
            in_generator_body: false,
            collect_known_loads: true,
            named_target_after_value: false,
            known_assignment_bindings,
            ignored_candidate_loads: FxHashSet::default(),
        }
        .visit_expr(expression);
    }

    fn collect_definition_header_expression(&mut self, expression: &Expr) {
        NamedExpressionCollector {
            collector: self,
            in_inlined_comprehension: false,
            in_generator_body: false,
            collect_known_loads: true,
            named_target_after_value: true,
            known_assignment_bindings: FxHashSet::default(),
            ignored_candidate_loads: FxHashSet::default(),
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
    in_generator_body: bool,
    collect_known_loads: bool,
    named_target_after_value: bool,
    known_assignment_bindings: FxHashSet<Name>,
    ignored_candidate_loads: FxHashSet<Name>,
}

impl<'ast> Visitor<'ast> for NamedExpressionCollector<'_> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let Expr::Lambda(lambda) = expression {
            visit_lambda_defaults(self, lambda);
            return;
        }
        if let Expr::Generator(generator) = expression {
            let previous = self.in_inlined_comprehension;
            if let Some(first) = generator.generators.first() {
                self.visit_expr(&first.iter);
            }
            self.in_inlined_comprehension = false;
            let previous_loads = std::mem::replace(&mut self.collect_known_loads, false);
            let previous_generator = std::mem::replace(&mut self.in_generator_body, true);
            if !self.collector.ignore_generator_body_comprehension_targets {
                walk_expr(self, expression);
            }
            self.in_generator_body = previous_generator;
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
                    .insert_comprehension_target(Name::new(name.id.as_str()));
            }
            self.collector.collect_target(&named.target);
            return;
        }
        if self.collect_known_loads
            && let Expr::Name(name) = expression
            && name.ctx == ExprContext::Load
            && !self.ignored_candidate_loads.contains(name.id.as_str())
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
            if self.in_generator_body {
                // Walrus targets bind in the containing function, but temporary locals in
                // comprehensions evaluated by a generator belong to the generator's scope.
                walk_expr(self, expression);
                return;
            }
            if !self.in_inlined_comprehension {
                // CPython evaluates the outermost iterable before reserving the inlined targets.
                // Target names without a known outer binding are ignored during this early walk
                // so a global reference does not claim the later temporary's fast-local position.
                let mut target_names = Vec::new();
                for generator in generators {
                    collect_target_names(&generator.target, &mut target_names);
                }
                let previous_ignored = self.ignored_candidate_loads.clone();
                self.ignored_candidate_loads.extend(
                    target_names.into_iter().map(Name::from).filter(|name| {
                        !self.collector.non_comprehension_bindings.contains(name)
                            && !self.known_assignment_bindings.contains(name)
                    }),
                );
                if let Some(first) = generators.first() {
                    self.visit_expr(&first.iter);
                }
                self.ignored_candidate_loads = previous_ignored;
            }

            if let Some(first) = generators.first() {
                let mut names = Vec::new();
                if self.collector.nonlocals.is_empty() {
                    collect_nested_comprehension_target_names(&first.iter, &mut names);
                } else {
                    self.collector
                        .collect_nested_comprehension_saves(&first.iter, &mut names);
                }
                for name in names.into_iter().map(Name::from) {
                    // The first iterable executes before the enclosing comprehension binds its
                    // target. Inlined comprehensions there therefore allocate their temporary
                    // fast locals first; the later full expression walk only observes them
                    // after the enclosing targets have already been collected.
                    self.collector
                        .insert_comprehension_iteration_target(name.clone());
                    self.collector.claim_annotation_only_position(&name);
                    self.collector.mark_binding(&name);
                }
            }
            let (key, value) = match expression {
                Expr::ListComp(comprehension) => (None, comprehension.elt.as_ref()),
                Expr::SetComp(comprehension) => (None, comprehension.elt.as_ref()),
                Expr::DictComp(comprehension) => {
                    (comprehension.key.as_deref(), comprehension.value.as_ref())
                }
                _ => unreachable!(),
            };
            let deferred_nonlocal_names = self
                .collector
                .deferred_nonlocal_comprehension_names(generators, key, value);
            // Reserve current-scope saves before visiting children. An explicit nonlocal
            // reference retains its outer value until the child that actually binds it.
            let mut previous_target_names = Vec::new();
            for name in comprehension_temporary_names(generators, key, value)
                .into_iter()
                .filter(|name| !deferred_nonlocal_names.contains(name))
                .map(Name::from)
            {
                let is_new_target = !self.collector.seen_comprehension_targets.contains(&name);
                self.collector
                    .insert_comprehension_iteration_target(name.clone());
                if is_new_target
                    && self.collector.comprehension_late_bindings.contains(&name)
                    && let Some(predecessors) = self
                        .collector
                        .comprehension_target_predecessors
                        .get_mut(&name)
                {
                    predecessors.extend(previous_target_names.iter().cloned());
                }
                previous_target_names.push(name.clone());
                self.collector.claim_annotation_only_position(&name);
                self.collector.mark_binding(&name);
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
                    .insert_comprehension_target(Name::new(name.id.as_str()));
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

struct ScopeBodyFactsCollector<'ast> {
    facts: ScopeBodyFacts,
    references: FxHashSet<&'ast str>,
    outside_inlined_comprehension_references: Option<FxHashSet<String>>,
    skip_annotations: bool,
    collect_function_scope_facts: bool,
    generator_depth: usize,
    inlined_comprehension_depth: usize,
    seen_inlined_comprehension_targets: FxHashSet<String>,
    class_comprehension_targets: Vec<String>,
}

impl<'ast> ScopeBodyFactsCollector<'ast> {
    fn collect(
        body: &'ast [Stmt],
        skip_annotations: bool,
        collect_function_scope_facts: bool,
    ) -> ScopeBodyFacts {
        let mut collector = Self {
            facts: ScopeBodyFacts::default(),
            references: FxHashSet::default(),
            outside_inlined_comprehension_references: None,
            skip_annotations,
            collect_function_scope_facts,
            generator_depth: 0,
            inlined_comprehension_depth: 0,
            seen_inlined_comprehension_targets: FxHashSet::default(),
            class_comprehension_targets: Vec::new(),
        };
        for statement in body {
            collector.visit_stmt(statement);
        }
        collector.facts.references = collector
            .references
            .into_iter()
            .map(str::to_string)
            .collect();
        collector.facts.outside_inlined_comprehension_references = collector
            .outside_inlined_comprehension_references
            .unwrap_or_default();
        if collect_function_scope_facts {
            // A nonlocal declaration establishes an own-scope free symbol even if
            // every use occurs under an inlined comprehension target.
            collector
                .facts
                .outside_inlined_comprehension_references
                .extend(collector.facts.nonlocals.iter().cloned());
        }
        collector.facts
    }

    fn collect_function_header(&mut self, definition: &'ast StmtFunctionDef) {
        self.facts.has_function_definition = true;
        // Symbol analysis visits defaults before decorators, which can establish the
        // first inlined-comprehension binding of a name in the enclosing scope.
        for default in function_default_expressions(definition) {
            self.visit_expr(default);
        }
        for decorator in &definition.decorator_list {
            self.visit_expr(&decorator.expression);
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

    fn collect_class_header(&mut self, definition: &'ast StmtClassDef) {
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

    fn collect_type_alias(&mut self, statement: &'ast ruff_python_ast::StmtTypeAlias) {
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

    fn insert_reference(&mut self, name: &'ast str) {
        self.references.insert(name);
        if self.inlined_comprehension_depth == 0
            && let Some(references) = &mut self.outside_inlined_comprehension_references
        {
            references.insert(name.to_string());
        }
        if name == "super" {
            self.references.insert("__class__");
        } else if name == "__class__" {
            self.facts.explicit_dunder_class_reference = true;
        }
    }
}

impl<'ast> Visitor<'ast> for ScopeBodyFactsCollector<'ast> {
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
            Stmt::Assign(assignment) if self.collect_function_scope_facts => {
                for target in &assignment.targets {
                    self.visit_expr(target);
                }
                self.visit_expr(&assignment.value);
                return;
            }
            Stmt::AugAssign(assignment) if self.collect_function_scope_facts => {
                self.visit_expr(&assignment.target);
                self.visit_expr(&assignment.value);
                return;
            }
            Stmt::For(statement) if self.collect_function_scope_facts => {
                self.visit_expr(&statement.target);
                self.visit_expr(&statement.iter);
                self.visit_body(&statement.body);
                self.visit_body(&statement.orelse);
                return;
            }
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
        if self.collect_function_scope_facts
            && let Expr::Dict(dictionary) = expression
        {
            for item in &dictionary.items {
                if let Some(key) = &item.key {
                    self.visit_expr(key);
                }
            }
            for item in &dictionary.items {
                self.visit_expr(&item.value);
            }
            return;
        }
        if let Expr::Lambda(lambda) = expression {
            visit_lambda_defaults(self, lambda);
            let required = analyze_lambda_scope(lambda).required;
            if self.inlined_comprehension_depth == 0 {
                self.facts
                    .outside_inlined_comprehension_requirements
                    .extend(required.iter().cloned());
            }
            self.facts.nested_lambda_requirements.extend(
                required
                    .into_iter()
                    .filter(|name| !self.class_comprehension_targets.contains(name)),
            );
            return;
        }
        if self.collect_function_scope_facts
            && self.generator_depth == 0
            && let Expr::Generator(generator) = expression
        {
            let required = generator_required_names(generator);
            if self.inlined_comprehension_depth == 0 {
                self.facts
                    .outside_inlined_comprehension_requirements
                    .extend(required.iter().cloned());
            }
            self.facts.nested_generator_requirements.extend(required);
            self.facts
                .generator_named_targets
                .extend(generator_named_targets(generator));
            if let Some(first) = generator.generators.first() {
                self.visit_expr(&first.iter);
            }
            return;
        }
        if self.generator_depth == 0 {
            let comprehension = match expression {
                Expr::ListComp(comprehension) => Some((
                    comprehension.generators.as_slice(),
                    None,
                    comprehension.elt.as_ref(),
                )),
                Expr::SetComp(comprehension) => Some((
                    comprehension.generators.as_slice(),
                    None,
                    comprehension.elt.as_ref(),
                )),
                Expr::DictComp(comprehension) => Some((
                    comprehension.generators.as_slice(),
                    comprehension.key.as_deref(),
                    comprehension.value.as_ref(),
                )),
                _ => None,
            };
            if let Some((generators, key, value)) = comprehension {
                self.facts
                    .inlined_comprehension_cellvars
                    .extend(comprehension_cell_names(generators, key, value));
                if self.collect_function_scope_facts {
                    if !self.facts.has_inlined_comprehension {
                        let mut references = self
                            .references
                            .iter()
                            .copied()
                            .map(str::to_string)
                            .collect::<FxHashSet<_>>();
                        if !self.facts.explicit_dunder_class_reference {
                            references.remove("__class__");
                        }
                        self.outside_inlined_comprehension_references = Some(references);
                    }
                    self.facts.has_inlined_comprehension = true;
                    // Only the first iterable is evaluated outside the inlined comprehension's
                    // temporary scope.
                    if let Some(first) = generators.first() {
                        self.visit_expr(&first.iter);
                    }
                    if self.inlined_comprehension_depth == 0 {
                        for (name, bound) in comprehension_name_bindings(generators, key, value) {
                            if bound {
                                self.seen_inlined_comprehension_targets.insert(name);
                            } else if !self.seen_inlined_comprehension_targets.contains(&name) {
                                self.outside_inlined_comprehension_references
                                    .as_mut()
                                    .expect("inlined comprehension references are initialized")
                                    .insert(name);
                            }
                        }
                    }
                    self.inlined_comprehension_depth += 1;
                    for (index, generator) in generators.iter().enumerate() {
                        self.visit_expr(&generator.target);
                        if index > 0 {
                            self.visit_expr(&generator.iter);
                        }
                        for condition in &generator.ifs {
                            self.visit_expr(condition);
                        }
                    }
                    self.visit_expr(value);
                    if let Some(key) = key {
                        self.visit_expr(key);
                    }
                    self.inlined_comprehension_depth -= 1;
                    return;
                }
                // A class comprehension provides its own captured targets. References from
                // nested lambdas to those targets do not become class free variables.
                if let Some(first) = generators.first() {
                    self.visit_expr(&first.iter);
                }
                let outer_targets = self.class_comprehension_targets.len();
                for generator in generators {
                    collect_target_names(&generator.target, &mut self.class_comprehension_targets);
                }
                for (index, generator) in generators.iter().enumerate() {
                    if index > 0 {
                        self.visit_expr(&generator.iter);
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
                self.class_comprehension_targets.truncate(outer_targets);
                return;
            }
        }

        if let Expr::Name(name) = expression
            && name.ctx == ExprContext::Load
        {
            self.insert_reference(name.id.as_str());
        }

        let is_generator = matches!(expression, Expr::Generator(_));
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
    skip_generator_bodies: bool,
    skip_inlined_comprehension_bodies: bool,
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
        if let Expr::Lambda(lambda) = expression {
            visit_lambda_defaults(self, lambda);
            return;
        }
        let generators = match expression {
            Expr::Generator(generator) if self.skip_generator_bodies => {
                Some(generator.generators.as_slice())
            }
            Expr::ListComp(comprehension) if self.skip_inlined_comprehension_bodies => {
                Some(comprehension.generators.as_slice())
            }
            Expr::SetComp(comprehension) if self.skip_inlined_comprehension_bodies => {
                Some(comprehension.generators.as_slice())
            }
            Expr::DictComp(comprehension) if self.skip_inlined_comprehension_bodies => {
                Some(comprehension.generators.as_slice())
            }
            _ => None,
        };
        if let Some(generators) = generators {
            // Only the first iterable is evaluated in the enclosing scope.
            if let Some(first) = generators.first() {
                self.visit_expr(&first.iter);
            }
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
    let parameter_names = locals.names.iter().cloned().collect::<FxHashSet<_>>();
    locals.collect_expression(&lambda.body);

    let mut references = ReferenceCollector {
        skip_generator_bodies: true,
        ..ReferenceCollector::default()
    };
    references.visit_expr(&lambda.body);
    let mut nested_requirements = nested_lambda_required_names_in_expression(&lambda.body);
    nested_requirements.extend(nested_generator_required_names_in_expression(&lambda.body));
    let local_names = locals.names.iter().cloned().collect::<FxHashSet<_>>();
    let mut outside_references = ReferenceCollector {
        skip_generator_bodies: true,
        skip_inlined_comprehension_bodies: true,
        ..ReferenceCollector::default()
    };
    // A comprehension's first iterable can read an outer binding with its target's spelling.
    // Lambda defaults are evaluated here too; nested lambda bodies retain their own captures.
    if !locals.comprehension_targets.is_empty() {
        outside_references.visit_expr(&lambda.body);
    }
    let cellvars: FxHashSet<_> = nested_requirements
        .iter()
        .filter(|name| local_names.contains(name.as_str()))
        .cloned()
        .collect();
    let required = references
        .references
        .iter()
        .chain(&nested_requirements)
        .filter(|name| {
            !local_names.contains(name.as_str())
                || (outside_references.references.contains(name.as_str())
                    && !locals.non_comprehension_bindings.contains(name.as_str()))
        })
        .cloned()
        .collect();
    let mut inlined_comprehension_cellvars =
        inlined_comprehension_cell_names_in_expression(&lambda.body);
    let comprehension_fast_cellvars = locals
        .comprehension_iteration_targets
        .iter()
        .filter(|name| cellvars.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let comprehension_target_predecessors = locals.comprehension_target_predecessors.clone();
    let comprehension_reordered_fast_cellvars = locals
        .comprehension_iteration_targets
        .iter()
        .filter(|name| !parameter_names.contains(*name))
        .filter(|name| locals.comprehension_late_bindings.contains(*name))
        .filter(|name| cellvars.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    reorder_comprehension_fast_cells(
        &mut locals.names,
        &comprehension_reordered_fast_cellvars,
        &comprehension_target_predecessors,
    );
    inlined_comprehension_cellvars.extend(
        comprehension_fast_cellvars
            .iter()
            .map(|name| name.as_str().to_string()),
    );
    LambdaScopeAnalysis {
        locals: locals.names.into_iter().map(String::from).collect(),
        cellvars,
        inlined_comprehension_cellvars,
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
            if let Expr::Generator(generator) = expression {
                if let Some(first) = generator.generators.first() {
                    self.visit_expr(&first.iter);
                }
                return;
            }
            if let Expr::Lambda(lambda) = expression {
                visit_lambda_defaults(self, lambda);
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
                // The first iterable executes in the enclosing scope and can create its
                // own closures, independently of the generator body's requirements.
                if let Some(first) = generator.generators.first() {
                    self.visit_expr(&first.iter);
                }
                self.required.extend(generator_required_names(generator));
                return;
            }
            if let Expr::Lambda(lambda) = expression {
                visit_lambda_defaults(self, lambda);
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

pub(super) fn expression_requirements_outside_comprehensions(
    expression: &Expr,
) -> BTreeSet<String> {
    #[derive(Default)]
    struct Collector {
        required: BTreeSet<String>,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            let generators = match expression {
                Expr::ListComp(comprehension) => Some(comprehension.generators.as_slice()),
                Expr::SetComp(comprehension) => Some(comprehension.generators.as_slice()),
                Expr::DictComp(comprehension) => Some(comprehension.generators.as_slice()),
                Expr::Lambda(lambda) => {
                    visit_lambda_defaults(self, lambda);
                    self.required.extend(analyze_lambda_scope(lambda).required);
                    return;
                }
                Expr::Generator(generator) => {
                    self.required.extend(generator_required_names(generator));
                    Some(generator.generators.as_slice())
                }
                Expr::Name(name) if name.ctx == ExprContext::Load => {
                    self.required.insert(name.id.to_string());
                    return;
                }
                _ => None,
            };
            if let Some(generators) = generators {
                if let Some(first) = generators.first() {
                    self.visit_expr(&first.iter);
                }
                return;
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(expression);
    collector.required
}

pub(super) fn inlined_comprehension_local_names_in_expression(expression: &Expr) -> Vec<String> {
    let mut collector = LocalCollector {
        ignore_generator_body_comprehension_targets: true,
        ..LocalCollector::default()
    };
    collector.collect_expression(expression);
    collector
        .names
        .into_iter()
        .filter(|name| {
            collector
                .seen_comprehension_iteration_targets
                .contains(name)
        })
        .map(String::from)
        .collect()
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

/// Return saved comprehension locals in CPython's symbol insertion order.
/// Nested inlined scopes merge after the containing scope's own symbols.
pub(super) fn comprehension_temporary_names(
    generators: &[ruff_python_ast::Comprehension],
    key: Option<&Expr>,
    value: &Expr,
) -> Vec<String> {
    #[derive(Default)]
    struct Collector {
        names: Vec<String>,
        nested_names: Vec<String>,
        targets: FxHashSet<String>,
    }

    impl Collector {
        fn comprehension(
            generators: &[ruff_python_ast::Comprehension],
            key: Option<&Expr>,
            value: &Expr,
        ) -> Self {
            let mut collector = Self::default();
            for (index, generator) in generators.iter().enumerate() {
                let mut targets = Vec::new();
                collect_target_names(&generator.target, &mut targets);
                collector.targets.extend(targets);
                collector.visit_expr(&generator.target);
                if index > 0 {
                    collector.visit_expr(&generator.iter);
                }
                for condition in &generator.ifs {
                    collector.visit_expr(condition);
                }
            }
            // The symbol-table traversal visits a dict comprehension's value before its key.
            collector.visit_expr(value);
            if let Some(key) = key {
                collector.visit_expr(key);
            }
            collector.names.append(&mut collector.nested_names);
            collector
        }
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            let (generators, key, value) = match expression {
                Expr::ListComp(comprehension) => {
                    (&comprehension.generators, None, comprehension.elt.as_ref())
                }
                Expr::SetComp(comprehension) => {
                    (&comprehension.generators, None, comprehension.elt.as_ref())
                }
                Expr::DictComp(comprehension) => (
                    &comprehension.generators,
                    comprehension.key.as_deref(),
                    comprehension.value.as_ref(),
                ),
                Expr::Generator(generator) => {
                    if let Some(first) = generator.generators.first() {
                        self.visit_expr(&first.iter);
                    }
                    return;
                }
                Expr::Lambda(lambda) => {
                    visit_lambda_defaults(self, lambda);
                    return;
                }
                Expr::Name(name) => {
                    self.names.push(name.id.as_str().to_string());
                    return;
                }
                _ => {
                    walk_expr(self, expression);
                    return;
                }
            };
            if let Some(first) = generators.first() {
                self.visit_expr(&first.iter);
            }
            let child = Self::comprehension(generators, key, value);
            self.targets.extend(child.targets);
            self.nested_names.extend(child.names);
        }
    }

    let mut collector = Collector::comprehension(generators, key, value);
    collector
        .names
        .retain(|name| collector.targets.remove(name));
    collector.names
}

/// Return each inlined symbol's binding role: true for bound, false for external.
/// Own symbols precede inlined children; otherwise the first child admission wins.
pub(super) fn comprehension_name_bindings(
    generators: &[ruff_python_ast::Comprehension],
    key: Option<&Expr>,
    value: &Expr,
) -> BTreeMap<String, bool> {
    #[derive(Default)]
    struct Collector {
        symbols: BTreeMap<String, bool>,
        children: Vec<BTreeMap<String, bool>>,
        closure_references: BTreeSet<String>,
    }

    impl Collector {
        fn comprehension(
            generators: &[ruff_python_ast::Comprehension],
            key: Option<&Expr>,
            value: &Expr,
        ) -> BTreeMap<String, bool> {
            let mut collector = Self::default();
            for generator in generators {
                let mut targets = Vec::new();
                collect_target_names(&generator.target, &mut targets);
                collector
                    .symbols
                    .extend(targets.into_iter().map(|name| (name, true)));
            }
            for (index, generator) in generators.iter().enumerate() {
                collector.visit_expr(&generator.target);
                if index > 0 {
                    collector.visit_expr(&generator.iter);
                }
                for condition in &generator.ifs {
                    collector.visit_expr(condition);
                }
            }
            collector.visit_expr(value);
            if let Some(key) = key {
                collector.visit_expr(key);
            }
            for child in collector.children {
                for (name, bound) in child {
                    collector.symbols.entry(name).or_insert(bound);
                }
            }
            for name in collector.closure_references {
                collector.symbols.entry(name).or_insert(false);
            }
            collector.symbols
        }
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            let (generators, key, value) = match expression {
                Expr::ListComp(comprehension) => {
                    (&comprehension.generators, None, comprehension.elt.as_ref())
                }
                Expr::SetComp(comprehension) => {
                    (&comprehension.generators, None, comprehension.elt.as_ref())
                }
                Expr::DictComp(comprehension) => (
                    &comprehension.generators,
                    comprehension.key.as_deref(),
                    comprehension.value.as_ref(),
                ),
                Expr::Generator(generator) => {
                    if let Some(first) = generator.generators.first() {
                        self.visit_expr(&first.iter);
                    }
                    self.closure_references
                        .extend(generator_required_names(generator));
                    return;
                }
                Expr::Lambda(lambda) => {
                    visit_lambda_defaults(self, lambda);
                    self.closure_references
                        .extend(analyze_lambda_scope(lambda).required);
                    return;
                }
                Expr::Dict(dictionary) => {
                    for item in &dictionary.items {
                        if let Some(key) = &item.key {
                            self.visit_expr(key);
                        }
                    }
                    for item in &dictionary.items {
                        self.visit_expr(&item.value);
                    }
                    return;
                }
                Expr::Named(named) => {
                    if let Expr::Name(name) = named.target.as_ref() {
                        self.symbols.entry(name.id.to_string()).or_insert(false);
                    }
                    self.visit_expr(&named.value);
                    return;
                }
                Expr::Name(name) if name.ctx == ExprContext::Load => {
                    self.symbols.entry(name.id.to_string()).or_insert(false);
                    if name.id.as_str() == "super" {
                        self.symbols.entry("__class__".to_string()).or_insert(false);
                    }
                    return;
                }
                _ => {
                    walk_expr(self, expression);
                    return;
                }
            };
            if let Some(first) = generators.first() {
                self.visit_expr(&first.iter);
            }
            self.children
                .push(Self::comprehension(generators, key, value));
        }
    }

    Collector::comprehension(generators, key, value)
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
                Expr::Generator(generator) => {
                    if let Some(first) = generator.generators.first() {
                        self.visit_expr(&first.iter);
                    }
                    return;
                }
                Expr::Lambda(lambda) => {
                    visit_lambda_defaults(self, lambda);
                    return;
                }
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
pub(super) fn collect_self_iterated_nested_comprehension_targets(
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
                    let Some(references) = definitely_evaluated_references(&generator.iter) else {
                        continue;
                    };
                    let mut targets = Vec::new();
                    collect_target_names(&generator.target, &mut targets);
                    self.names
                        .extend(targets.into_iter().filter(|name| references.contains(name)));
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

pub(super) fn inlined_comprehension_cell_names_in_expression(
    expression: &Expr,
) -> FxHashSet<String> {
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
                Expr::Lambda(lambda) => {
                    visit_lambda_defaults(self, lambda);
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
            if let Expr::Lambda(lambda) = expression {
                visit_lambda_defaults(self, lambda);
                return;
            }
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
    fn collect_requirements(
        expression: &Expr,
        local_names: &mut FxHashSet<String>,
        references: &mut ReferenceCollector,
        nested_requirements: &mut BTreeSet<String>,
    ) {
        let mut names = Vec::new();
        collect_nested_comprehension_target_names(expression, &mut names);
        local_names.extend(names);
        references.visit_expr(expression);
        nested_requirements.extend(nested_expression_required_names(expression));
    }

    let mut local_names = FxHashSet::default();
    for comprehension in &generator.generators {
        let mut names = Vec::new();
        collect_target_names(&comprehension.target, &mut names);
        local_names.extend(names);
    }

    let mut references = ReferenceCollector {
        skip_generator_bodies: true,
        ..ReferenceCollector::default()
    };
    let mut nested_requirements = BTreeSet::new();
    collect_requirements(
        &generator.elt,
        &mut local_names,
        &mut references,
        &mut nested_requirements,
    );
    for (index, comprehension) in generator.generators.iter().enumerate() {
        collect_requirements(
            &comprehension.target,
            &mut local_names,
            &mut references,
            &mut nested_requirements,
        );
        // The outermost iterable is evaluated by the enclosing scope and passed
        // to the generator as its implicit `.0` argument.
        if index > 0 {
            collect_requirements(
                &comprehension.iter,
                &mut local_names,
                &mut references,
                &mut nested_requirements,
            );
        }
        for condition in &comprehension.ifs {
            collect_requirements(
                condition,
                &mut local_names,
                &mut references,
                &mut nested_requirements,
            );
        }
    }
    references
        .references
        .extend(generator_named_targets(generator));
    references.references.extend(nested_requirements);
    references
        .references
        .into_iter()
        .filter(|name| !local_names.contains(name))
        .collect()
}

pub(super) fn generator_cellvars(generator: &ruff_python_ast::ExprGenerator) -> FxHashSet<String> {
    fn extend_local_names(expression: &Expr, local_names: &mut FxHashSet<String>) {
        let mut names = Vec::new();
        collect_nested_comprehension_target_names(expression, &mut names);
        local_names.extend(names);
    }

    let mut local_names = FxHashSet::default();
    for comprehension in &generator.generators {
        let mut names = Vec::new();
        collect_target_names(&comprehension.target, &mut names);
        local_names.extend(names);
    }

    extend_local_names(&generator.elt, &mut local_names);
    let mut nested_requirements = nested_expression_required_names(&generator.elt);
    for (index, comprehension) in generator.generators.iter().enumerate() {
        extend_local_names(&comprehension.target, &mut local_names);
        if index > 0 {
            extend_local_names(&comprehension.iter, &mut local_names);
            nested_requirements.extend(nested_expression_required_names(&comprehension.iter));
        }
        for condition in &comprehension.ifs {
            extend_local_names(condition, &mut local_names);
            nested_requirements.extend(nested_expression_required_names(condition));
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

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use ruff_python_parser::parse_module;
    use rustc_hash::{FxHashMap, FxHashSet};

    use super::{
        FlatFunctionInputs, LocalCollector, ModuleNameCollector, NameId, NameInterner,
        SCOPE_SEED_NAMES, ScopeDefinition, ScopeId,
    };

    #[test]
    fn scope_seed_names_are_unique() {
        assert_eq!(
            SCOPE_SEED_NAMES.into_iter().collect::<FxHashSet<_>>().len(),
            SCOPE_SEED_NAMES.len()
        );
    }

    #[test]
    fn nested_first_iterables_reserve_each_scope_once() {
        let collector = LocalCollector {
            nonlocals: FxHashSet::from_iter(["unused".to_string()]),
            ..LocalCollector::default()
        };
        let mut source = "values".to_string();
        let mut expected = Vec::new();
        for depth in 0..32 {
            source = format!("[item_{depth} for item_{depth} in {source}]");
            expected.push(format!("item_{depth}"));
            let parsed = parse_module(&source).unwrap();
            let [ruff_python_ast::Stmt::Expr(statement)] = parsed.suite().as_slice() else {
                panic!("expected one expression statement");
            };
            let mut names = Vec::new();
            collector.collect_nested_comprehension_saves(&statement.value, &mut names);
            // Check the admissions before deduplication: duplicate visits multiply with depth.
            assert_eq!(names, expected, "first-iterable depth {}", depth + 1);
        }
    }

    #[test]
    fn copied_finally_locals_follow_unwind_order_and_loop_boundaries() {
        let cases: &[(&str, &[&str])] = &[
            (
                "try:\n    if flag:\n        return flag\n    later = flag\nfinally:\n    cleanup = flag\n",
                &["cleanup", "later"],
            ),
            (
                "try:\n    for item in items:\n        if flag:\n            break\n    later = flag\nfinally:\n    cleanup = flag\n",
                &["item", "later", "cleanup"],
            ),
            (
                "try:\n    for item in items:\n        if flag:\n            continue\n    later = flag\nfinally:\n    cleanup = flag\n",
                &["item", "later", "cleanup"],
            ),
            (
                "try:\n    for item in items:\n        if flag:\n            return flag\n    later = flag\nfinally:\n    cleanup = flag\n",
                &["item", "cleanup", "later"],
            ),
            (
                "for item in items:\n    try:\n        if flag:\n            break\n        later = flag\n    finally:\n        cleanup = flag\n",
                &["item", "cleanup", "later"],
            ),
            (
                "for item in items:\n    try:\n        if flag:\n            continue\n        later = flag\n    finally:\n        cleanup = flag\n",
                &["item", "cleanup", "later"],
            ),
            (
                "try:\n    try:\n        if flag:\n            return flag\n        later = flag\n    finally:\n        inner = flag\n        return inner\nfinally:\n    outer = flag\n",
                &["inner", "outer", "later"],
            ),
        ];
        for (source, expected) in cases {
            let parsed = parse_module(source).unwrap();
            let mut collector = LocalCollector::default();
            collector.collect_suite(parsed.suite());
            assert_eq!(collector.names, *expected, "source:\n{source}");
        }
    }

    #[test]
    fn seeded_name_interner_preserves_ids_order_and_borrowed_storage() {
        let mut names = NameInterner::seeded(&SCOPE_SEED_NAMES);

        assert_eq!(names.slots.len(), 64);
        for (index, name) in SCOPE_SEED_NAMES.into_iter().enumerate() {
            let id = NameId(u32::try_from(index).unwrap());
            assert_eq!(names.get(name), Some(id));
            assert_eq!(names.resolve(id), name);
            assert!(matches!(names.names[index], Cow::Borrowed(_)));
        }

        assert_eq!(names.intern("__user_name__"), NameId(26));
        assert!(matches!(names.names[26], Cow::Owned(_)));
        assert_eq!(names.intern("super"), NameId(25));
    }

    #[test]
    fn default_name_interner_starts_with_zero_id() {
        let mut names = NameInterner::default();

        assert_eq!(names.intern("dynamic"), NameId(0));
        assert!(matches!(names.names[0], Cow::Owned(_)));
    }

    #[test]
    fn module_name_walk_records_scope_definitions_in_preorder() {
        let source = "if condition:\n    def outer(value):\n        class Inner:\n            def method(self):\n                return value\nelse:\n    class Sibling:\n        def other(self):\n            return 1\n";
        let parsed = parse_module(source).unwrap();
        let mut names = NameInterner::seeded(&SCOPE_SEED_NAMES);

        let (_, definitions, requires_module_bindings) =
            ModuleNameCollector::collect(&mut names, parsed.suite());

        assert!(!requires_module_bindings);
        assert_eq!(
            definitions
                .iter()
                .map(|input| {
                    let (name, flat) = match &input.definition {
                        ScopeDefinition::Function(definition, flat) => {
                            (definition.name.as_str(), flat.is_some())
                        }
                        ScopeDefinition::Class(definition) => (definition.name.as_str(), false),
                    };
                    (input.parent, name, flat)
                })
                .collect::<Vec<_>>(),
            [
                (ScopeId::MODULE, "outer", false),
                (ScopeId(1), "Inner", false),
                (ScopeId(2), "method", true),
                (ScopeId::MODULE, "Sibling", false),
                (ScopeId(4), "other", true),
            ]
        );
    }

    #[test]
    fn module_binding_walk_only_runs_for_module_sensitive_inputs() {
        let cases = [
            (
                "def nested(values):\n    return [item for item in values]\n",
                false,
            ),
            ("[item for item in values]\n", true),
            (
                "@decorate([item for item in values])\ndef header():\n    pass\n",
                true,
            ),
            (
                "def header(value=[item for item in values]):\n    pass\n",
                true,
            ),
            ("(lambda value=[item for item in values]: value)()\n", true),
            ("def nested():\n    return super()\n", true),
        ];

        for (source, expected) in cases {
            let parsed = parse_module(source).unwrap();
            let mut names = NameInterner::seeded(&SCOPE_SEED_NAMES);
            let (_, _, requires_module_bindings) =
                ModuleNameCollector::collect(&mut names, parsed.suite());
            assert_eq!(requires_module_bindings, expected, "source:\n{source}");
        }
    }

    #[test]
    fn flat_function_inputs_preserve_first_occurrence_and_late_binding() {
        let mut inputs = FlatFunctionInputs {
            names: Vec::new(),
            name_positions: None,
            complex: false,
        };

        inputs.record(NameId(2), false);
        inputs.record(NameId(1), true);
        inputs.record(NameId(2), true);

        assert_eq!(inputs.names, [(NameId(2), true), (NameId(1), true)]);
        assert!(inputs.name_positions.is_none());
    }

    #[test]
    fn indexed_flat_function_inputs_preserve_first_occurrence_and_late_binding() {
        let mut inputs = FlatFunctionInputs {
            names: Vec::new(),
            name_positions: None,
            complex: false,
        };

        for name in 0..=FlatFunctionInputs::LINEAR_SEARCH_LIMIT {
            inputs.record(NameId(u32::try_from(name).unwrap()), false);
        }
        for name in (0..=FlatFunctionInputs::LINEAR_SEARCH_LIMIT).rev() {
            inputs.record(NameId(u32::try_from(name).unwrap()), true);
        }

        assert_eq!(
            inputs.names,
            (0..=FlatFunctionInputs::LINEAR_SEARCH_LIMIT)
                .map(|name| (NameId(u32::try_from(name).unwrap()), true))
                .collect::<Vec<_>>()
        );
        assert!(inputs.name_positions.is_some());
    }

    #[test]
    fn complex_function_inputs_discard_and_stop_recording_names() {
        let mut inputs = FlatFunctionInputs {
            names: vec![(NameId(1), false), (NameId(2), true)],
            name_positions: Some(FxHashMap::from_iter([(NameId(1), 0), (NameId(2), 1)])),
            complex: false,
        };

        inputs.mark_complex();
        inputs.record(NameId(3), true);
        inputs.record(NameId(1), true);

        assert!(inputs.complex);
        assert!(inputs.names.is_empty());
        assert!(inputs.name_positions.is_none());
    }
}
