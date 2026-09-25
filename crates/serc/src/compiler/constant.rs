use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use ruff_python_ast::{
    BoolOp, Expr, ExprBinOp, ExprContext, ExprStringLiteral, ExprUnaryOp, FStringPartRef,
    InterpolatedStringElement, Number, Operator, Pattern, Singleton, Stmt, UnaryOp,
};
use ruff_text_size::Ranged;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use smallvec::SmallVec;

use super::{LOAD_CONST, LOAD_SMALL_INT, NOP, STORE_NAME};
use crate::assembler::{FoldAdmissionId, Label, OperandPatchId};
use crate::target::operands::Conversion;
use crate::{CompileError, PythonVersion};

use super::Compiler;
use super::expression::{jump_constant_truthiness, literal_truthiness};
use super::scope::NameId;
use super::symbol::{mangle_name, to_u32};
use crate::code_object::{CodeObject, Constant, MetadataId, MetadataName, NonReflexiveRemap};

#[derive(Debug, Default)]
pub(super) struct ConstantPool {
    values: Vec<Constant>,
    indices: Option<ConstantIndices>,
}

const CONSTANT_INDEX_THRESHOLD: usize = 12;

#[cfg(test)]
thread_local! {
    static FORCE_GENERIC_CONSTANT_ADMISSION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Pool keys never match different constant variants, including numerically equal types.
#[inline]
fn find_pool_constant(values: &[Constant], constant: &Constant) -> Option<usize> {
    let kind = std::mem::discriminant(constant);
    values.iter().position(|existing| {
        std::mem::discriminant(existing) == kind
            && ConstantEquality::PoolKey.equal(existing, constant)
    })
}

#[derive(Debug, Default)]
struct ConstantIndices {
    primary: FxHashMap<u64, u32>,
    collisions: FxHashMap<u64, Vec<u32>>,
    /// Constants containing duplicate frozenset members cannot safely use a rich fingerprint
    /// because pool equality conservatively ignores member multiplicity. The compiler does not
    /// emit such values, but retaining a fallback preserves the existing behavior for
    /// hand-constructed constants.
    fallback: Vec<u32>,
}

impl ConstantIndices {
    fn insert(&mut self, fingerprint: Option<u64>, index: u32) {
        if let Some(fingerprint) = fingerprint {
            match self.primary.entry(fingerprint) {
                Entry::Occupied(_) => {
                    self.collisions.entry(fingerprint).or_default().push(index);
                }
                Entry::Vacant(entry) => {
                    entry.insert(index);
                }
            }
        } else {
            self.fallback.push(index);
        }
    }

    fn find(
        &self,
        values: &[Constant],
        constant: &Constant,
        fingerprint: Option<u64>,
    ) -> Option<u32> {
        let matches = |index: &u32| {
            let index = usize::try_from(*index).expect("constant index fits in usize");
            ConstantEquality::PoolKey.equal(&values[index], constant)
        };
        if let Some(fingerprint) = fingerprint {
            if self.fallback.is_empty() {
                let primary = *self.primary.get(&fingerprint)?;
                if matches(&primary) {
                    return Some(primary);
                }
                // The pool inserts indices in order, so the first matching collision is earliest.
                return self
                    .collisions
                    .get(&fingerprint)?
                    .iter()
                    .copied()
                    .find(matches);
            }
            return self
                .primary
                .get(&fingerprint)
                .into_iter()
                .chain(self.collisions.get(&fingerprint).into_iter().flatten())
                .chain(&self.fallback)
                .filter(|index| matches(index))
                .copied()
                .min();
        }
        find_pool_constant(values, constant)
            .map(|index| u32::try_from(index).expect("constant index fits in u32"))
    }
}

impl ConstantPool {
    pub(super) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.values.len()
    }

    pub(super) fn intern(&mut self, constant: Constant) -> Result<u32, CompileError> {
        if matches!(&constant, Constant::None)
            && matches!(self.values.first(), Some(Constant::None))
        {
            return Ok(0);
        }
        if self.indices.is_none() && self.values.len() >= CONSTANT_INDEX_THRESHOLD {
            let mut indices = ConstantIndices::default();
            indices.primary.reserve(self.values.len().saturating_mul(2));
            for (index, value) in self.values.iter().enumerate() {
                indices.insert(
                    constant_pool_fingerprint(value),
                    u32::try_from(index).expect("constant index fits in u32"),
                );
            }
            self.indices = Some(indices);
        }

        if let Some(indices) = &mut self.indices {
            let fingerprint = constant_pool_fingerprint(&constant);
            if let Some(index) = indices.find(&self.values, &constant, fingerprint) {
                return Ok(index);
            }
            let index = to_u32(self.values.len(), "constant count")?;
            indices.insert(fingerprint, index);
            self.values.push(constant);
            return Ok(index);
        }

        if let Some(index) = find_pool_constant(&self.values, &constant) {
            return to_u32(index, "constant index");
        }
        let index = to_u32(self.values.len(), "constant count")?;
        self.values.push(constant);
        Ok(index)
    }

    pub(super) fn retain_used(&mut self, used: &[bool]) -> Vec<Option<u32>> {
        self.indices = None;
        let mut index_map = vec![None; self.values.len()];
        let mut old_index = 0;
        let mut new_index = 0;
        self.values.retain(|_| {
            let retain = old_index == 0 || used.get(old_index).copied().unwrap_or(false);
            if retain {
                index_map[old_index] = Some(new_index);
                new_index += 1;
            }
            old_index += 1;
            retain
        });
        index_map
    }

    pub(super) fn into_values(self) -> Vec<Constant> {
        self.values
    }
}

fn constant_pool_fingerprint(constant: &Constant) -> Option<u64> {
    fn hash_constant(constant: &Constant) -> Option<u64> {
        let mut hasher = FxHasher::default();
        std::mem::discriminant(constant).hash(&mut hasher);
        match constant {
            Constant::None | Constant::Ellipsis => {}
            Constant::Bool(value) => value.hash(&mut hasher),
            Constant::Int(value) => value.hash(&mut hasher),
            Constant::SignedInt(value) => value.hash(&mut hasher),
            Constant::BigInt { negative, digits } => {
                negative.hash(&mut hasher);
                digits.hash(&mut hasher);
            }
            Constant::NonReflexive(number) => number.key().hash(&mut hasher),
            Constant::Float(value) => value.to_bits().hash(&mut hasher),
            Constant::Complex { real, imag } => {
                real.to_bits().hash(&mut hasher);
                imag.to_bits().hash(&mut hasher);
            }
            Constant::String(value) => value.hash(&mut hasher),
            Constant::SurrogateString(value) | Constant::Bytes(value) => {
                value.hash(&mut hasher);
            }
            Constant::Tuple(values) => {
                values.len().hash(&mut hasher);
                for value in values {
                    hash_constant(value)?.hash(&mut hasher);
                }
            }
            Constant::FrozenSet(values) => {
                let mut members = SmallVec::<[(u64, &Constant); 4]>::with_capacity(values.len());
                for value in values {
                    members.push((hash_constant(value)?, value));
                }
                members.sort_unstable_by_key(|(fingerprint, _)| *fingerprint);
                for group in members.chunk_by(|left, right| left.0 == right.0) {
                    for (offset, (_, left)) in group.iter().enumerate() {
                        if group[offset + 1..].iter().any(|(_, right)| {
                            ConstantEquality::PoolKey.equal(left, right)
                                || ConstantEquality::PoolKey.equal(right, left)
                        }) {
                            return None;
                        }
                    }
                }
                values.len().hash(&mut hasher);
                for (fingerprint, _) in members {
                    fingerprint.hash(&mut hasher);
                }
            }
            Constant::Slice { lower, upper, step } => {
                hash_constant(lower)?.hash(&mut hasher);
                hash_constant(upper)?.hash(&mut hasher);
                hash_constant(step)?.hash(&mut hasher);
            }
            Constant::Code(code) => {
                // Source identity distinguishes nearly all code objects in a pool. Hash only
                // inexpensive equality fields, leaving the full body comparison to collisions
                // rather than rehashing nested code every time a parent enters its own pool.
                code.first_line_number.hash(&mut hasher);
                code.name.hash(&mut hasher);
                code.qualified_name.hash(&mut hasher);
                code.arg_count.hash(&mut hasher);
                code.positional_only_arg_count.hash(&mut hasher);
                code.keyword_only_arg_count.hash(&mut hasher);
                code.stack_size.hash(&mut hasher);
                code.flags.hash(&mut hasher);
                code.bytecode.len().hash(&mut hasher);
                code.constants.len().hash(&mut hasher);
                std::mem::discriminant(&code.marshal_provenance.locals_identity).hash(&mut hasher);
            }
        }
        Some(hasher.finish())
    }

    hash_constant(constant)
}

#[derive(Debug, Default)]
pub(super) struct OutputState {
    pub(super) constants: ConstantPool,
    pub(super) has_deferred_break_none: bool,
    pub(super) deferred_constants_before_return: Vec<(OperandPatchId, Constant)>,
    pub(super) deferred_constants: Vec<(Option<DeferredConstantTarget>, Constant)>,
    pub(super) fresh_empty_bytes_deferred: FxHashSet<usize>,
    pub(super) folded_constant_dependencies: Vec<(Constant, Vec<Constant>)>,
    pub(super) has_untracked_discarded_collection: bool,
    pub(super) deferred_names: Vec<(OperandPatchId, String)>,
    pub(super) names: Vec<MetadataName>,
    pub(super) name_ids: Vec<MetadataId>,
    name_indices: FxHashMap<NameId, u32>,
    pub(super) interned_constant_strings: FxHashSet<String>,
    pub(super) qualified_name_constant_index: Option<u32>,
    pub(super) qualified_name_is_canonical: Option<bool>,
    pub(super) has_frozenset_constants: bool,
    pub(super) first_flowgraph_constant: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum DeferredConstantTarget {
    Instruction(OperandPatchId),
    DiscardedExpression(Label),
    FoldedOperand(FoldAdmissionId),
}

impl crate::assembler::ConstantTable for OutputState {
    fn get(&self, index: u32) -> &Constant {
        &self.constants.values[index as usize]
    }

    fn intern(&mut self, constant: Constant) -> Result<u32, CompileError> {
        self.constants.intern(constant)
    }

    fn reallocate_non_reflexive_constant(
        &mut self,
        index: u32,
        remap: &mut NonReflexiveRemap,
    ) -> Result<u32, CompileError> {
        let constant = &self.constants.values[index as usize];
        if !constant.has_non_reflexive_number() {
            return Ok(index);
        }
        let constant = constant.clone();
        let constant = self.reallocate_non_reflexive_value(constant, remap);
        self.constants.intern(constant)
    }

    fn fold_sequence(&mut self, values: Vec<Constant>, is_set: bool) -> Result<u32, CompileError> {
        let result = if is_set {
            self.has_frozenset_constants = true;
            let mut unique = Vec::with_capacity(values.len());
            for constant in &values {
                if !unique
                    .iter()
                    .any(|existing| python_constant_values_equal(existing, constant))
                {
                    unique.push(constant.clone());
                }
            }
            Constant::FrozenSet(unique)
        } else {
            Constant::Tuple(values.clone())
        };
        self.record_folded_constant_dependencies(&result, values);
        self.constants.intern(result)
    }

    fn len(&self) -> usize {
        self.constants.len()
    }

    fn retain_used_in_order(&mut self, used: &[bool]) -> Vec<Option<u32>> {
        let indices = self.constants.retain_used(used);
        self.qualified_name_constant_index = self
            .qualified_name_constant_index
            .and_then(|index| indices[index as usize]);
        self.first_flowgraph_constant = indices
            .iter()
            .take(self.first_flowgraph_constant)
            .flatten()
            .count();
        indices
    }

    fn retain_used(
        &mut self,
        used: &[bool],
        order: &[u32],
        early_booleans: &[bool],
    ) -> Vec<Option<u32>> {
        fn append_folded_dependencies(
            constant: &Constant,
            pool: &ConstantPool,
            dependencies: &[(Constant, Vec<Constant>)],
            first_deferred: usize,
            used: &[bool],
            seen: &mut [bool],
            order: &mut Vec<u32>,
        ) {
            if let Some((_, operands)) = dependencies
                .iter()
                .find(|(result, _)| ConstantEquality::PoolKey.equal(result, constant))
            {
                for operand in operands {
                    append_folded_dependencies(
                        operand,
                        pool,
                        dependencies,
                        first_deferred,
                        used,
                        seen,
                        order,
                    );
                }
            }
            if let Constant::Tuple(elements) = constant {
                for element in elements {
                    append_folded_dependencies(
                        element,
                        pool,
                        dependencies,
                        first_deferred,
                        used,
                        seen,
                        order,
                    );
                }
            }
            let index = if let Some(indices) = &pool.indices {
                indices
                    .find(&pool.values, constant, constant_pool_fingerprint(constant))
                    .map(|index| index as usize)
            } else {
                find_pool_constant(&pool.values, constant)
            };
            if let Some(index) = index
                && index >= first_deferred
                && used[index]
                && !seen[index]
            {
                seen[index] = true;
                order.push(u32::try_from(index).expect("constant index fits in u32"));
            }
        }

        if self.has_untracked_discarded_collection {
            // Copying a discarded runtime collection can create constants absent from
            // the source-fold reservations, so their relative admissions remain unknown.
            self.folded_constant_dependencies
                .retain(|(result, _)| !matches!(result, Constant::FrozenSet(_)));
        }
        // Folded values are created after their operands, even when their first surviving
        // load precedes the operands' later loads. Preserve those dependencies while
        // allowing independent constants to follow the copied block order.
        let mut construction_order = Vec::with_capacity(order.len());
        let mut seen = vec![false; self.constants.len()];
        // CPython completes every block's early TO_BOOL pass before creating any
        // unary, arithmetic, or tuple-fold results. Code-generation literals still
        // precede these admissions, and unused intermediates are filtered normally.
        for value in early_booleans {
            if let Some(index) = self.constants.values.iter().position(
                |constant| matches!(constant, Constant::Bool(boolean) if boolean == value),
            ) && index >= self.first_flowgraph_constant
                && used[index]
                && !seen[index]
            {
                seen[index] = true;
                construction_order.push(u32::try_from(index).expect("constant index fits in u32"));
            }
        }
        for index in order {
            if seen[*index as usize] {
                continue;
            }
            append_folded_dependencies(
                &self.constants.values[*index as usize],
                &self.constants,
                &self.folded_constant_dependencies,
                self.first_flowgraph_constant,
                used,
                &mut seen,
                &mut construction_order,
            );
            if !seen[*index as usize] {
                seen[*index as usize] = true;
                construction_order.push(*index);
            }
        }
        // Code generation reserves literals first; flowgraph-created constants then follow
        // basic-block order. Late exit folding must interleave with front-folded constants.
        let mut values = std::mem::take(&mut self.constants.values)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        self.constants.indices = None;
        let mut index_map = vec![None; values.len()];
        for index in (0..self.first_flowgraph_constant)
            .filter(|index| *index == 0 || used[*index])
            .chain(
                construction_order
                    .iter()
                    .map(|index| *index as usize)
                    .filter(|index| *index >= self.first_flowgraph_constant),
            )
        {
            if let Some(value) = values[index].take() {
                index_map[index] = Some(
                    u32::try_from(self.constants.values.len()).expect("constant index fits in u32"),
                );
                self.constants.values.push(value);
            }
        }
        self.qualified_name_constant_index = self
            .qualified_name_constant_index
            .and_then(|index| index_map[index as usize]);
        self.first_flowgraph_constant = index_map
            .iter()
            .take(self.first_flowgraph_constant)
            .flatten()
            .count();
        index_map
    }
}

/// Certifies owners retained by ordinary value lowering and dynamic branch emission.
///
/// Literal tests, discarded expressions, and compound control constructs can remove an
/// original `TO_BOOL` owner before the assembler sees it. Keep their existing ordering
/// until those native lifetimes have an instruction-level representation as well.
pub(super) fn can_order_early_boolean_admissions(body: &[Stmt]) -> bool {
    struct Owners {
        certified: bool,
    }

    impl Owners {
        fn suite(&mut self, body: &[Stmt]) {
            for (index, statement) in body.iter().enumerate() {
                if !self.certified {
                    return;
                }
                if matches!(statement, Stmt::Return(_)) && index + 1 != body.len() {
                    self.certified = false;
                    return;
                }
                self.visit_stmt(statement);
            }
        }

        fn dynamic_test(expression: &Expr) -> bool {
            match expression {
                Expr::Name(name) => name.id.as_str() != "__debug__",
                Expr::Attribute(_) | Expr::Call(_) | Expr::Compare(_) => true,
                Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => {
                    Self::dynamic_test(&unary.operand)
                }
                _ => false,
            }
        }
    }

    impl<'ast> Visitor<'ast> for Owners {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if !self.certified {
                return;
            }
            match statement {
                Stmt::Assign(_) | Stmt::Return(_) | Stmt::Pass(_) => walk_stmt(self, statement),
                // Keep existing tuple-not exclusions as modern sequence folding expands.
                Stmt::Expr(expression)
                    if fold_constant(&expression.value).is_none()
                        && fold_constant_for_version(&expression.value, PythonVersion::Py314)
                            .is_none() =>
                {
                    walk_stmt(self, statement);
                }
                Stmt::If(conditional) => {
                    self.certified &= Self::dynamic_test(&conditional.test);
                    self.visit_expr(&conditional.test);
                    self.suite(&conditional.body);
                    for clause in &conditional.elif_else_clauses {
                        if !self.certified {
                            return;
                        }
                        if let Some(test) = &clause.test {
                            self.certified &= Self::dynamic_test(test);
                            self.visit_expr(test);
                        }
                        self.suite(&clause.body);
                    }
                }
                _ => self.certified = false,
            }
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if !self.certified {
                return;
            }
            match expression {
                Expr::UnaryOp(unary)
                    if unary.op == UnaryOp::Not
                        && literal_constant(&unary.operand).is_some()
                        && fold_constant(expression).is_none() =>
                {
                    // An early native Boolean with no folded owner cannot contribute an
                    // instruction witness to the copied graph.
                    self.certified = false;
                }
                Expr::Name(_)
                | Expr::NoneLiteral(_)
                | Expr::BooleanLiteral(_)
                | Expr::NumberLiteral(_)
                | Expr::StringLiteral(_)
                | Expr::BytesLiteral(_)
                | Expr::EllipsisLiteral(_)
                | Expr::Attribute(_)
                | Expr::Call(_)
                | Expr::Compare(_)
                | Expr::UnaryOp(_)
                | Expr::BinOp(_)
                | Expr::Tuple(_)
                | Expr::List(_)
                | Expr::Set(_)
                | Expr::Dict(_)
                | Expr::Subscript(_)
                | Expr::Slice(_)
                | Expr::Starred(_) => walk_expr(self, expression),
                _ => self.certified = false,
            }
        }
    }

    let mut owners = Owners { certified: true };
    owners.suite(body);
    owners.certified
}

/// Records only `TO_BOOL` operands already loaded as constants before late folding.
fn collect_early_boolean_admissions(expression: &Expr, admissions: &mut Vec<bool>) {
    struct Collector<'a>(&'a mut Vec<bool>);
    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            walk_expr(self, expression);
            if let Expr::UnaryOp(unary) = expression
                && unary.op == UnaryOp::Not
                && let Some(value) = literal_constant(&unary.operand).or_else(|| {
                    matches!(unary.operand.as_ref(), Expr::Name(name) if name.id.as_str() == "__debug__")
                        .then_some(Constant::Bool(true))
                })
            {
                let value = value.is_truthy();
                if !self.0.contains(&value) {
                    self.0.push(value);
                }
            }
        }
    }
    Collector(admissions).visit_expr(expression);
}

impl Compiler<'_> {
    pub(super) fn emit_folded_constant(
        &mut self,
        expression: &Expr,
        constant: Constant,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313
            || matches!(expression, Expr::Name(name) if name.id.as_str() == "__debug__")
        {
            // Older targets fold the AST before code generation, so only the result is
            // admitted here. Recording operands would change the first constant and its order.
            return self.emit_preprocessed_constant(expression, constant);
        }
        let first_operand_fold = self.output.deferred_constants.len();
        if let Some(literal) = literal_constant(expression) {
            self.add_constant(literal)?;
        } else {
            self.record_folded_operands(expression)?;
        }
        let operand_folds = first_operand_fold..self.output.deferred_constants.len();
        let double_ellipsis = double_negated_ellipsis(expression);
        if let Some(inner) = double_ellipsis {
            // The flowgraph keeps the folded load in the inner `not` slot and leaves
            // the outer operation's line marker after it.
            self.assembler
                .set_location(self.source_location(inner.operand.range()));
            self.emit(NOP, 0, 0)?;
            self.assembler
                .set_location(self.source_location(inner.range));
        } else if matches!(expression, Expr::Tuple(_))
            || matches!(expression, Expr::UnaryOp(unary) if unary.op == UnaryOp::Not)
        {
            self.emit_folded_tuple_not_nops(expression)?;
            self.assembler
                .set_location(self.source_location(expression.range()));
        }
        let hidden_owner_slots =
            folded_constant_owner_slots(expression, self.context.python_version);
        if self.context.python_version >= PythonVersion::Py314
            && let Constant::Int(value) = constant
            && let Ok(value) = u8::try_from(value)
        {
            self.emit(LOAD_SMALL_INT, u32::from(value), 1)?;
        } else {
            self.record_fresh_empty_bytes_fold(expression, &constant);
            self.emit_deferred_constant(constant)?;
        }
        if !operand_folds.is_empty()
            && contains_negative_empty_repetition(expression, self.context.python_version)
        {
            // An empty result can reuse a literal while its negative count is newly
            // admitted. Keep that occurrence's operands before later folded results,
            // without making the pooled empty literal depend on every later count.
            self.attach_folded_operand_admissions(Some(operand_folds));
        }
        if self.assembler.early_boolean_admissions_certified() {
            let mut admissions = Vec::new();
            collect_early_boolean_admissions(expression, &mut admissions);
            self.assembler
                .record_last_early_boolean_admissions(&admissions);
        }
        self.assembler
            .add_last_constant_fold_slots(hidden_owner_slots);
        // Recursive literal tuples and prebuilt constant slices fold within the
        // same original array, so every removed record has the survivor's owner.
        if let Expr::Tuple(tuple) = expression
            && tuple.elts.iter().all(|element| {
                is_small_literal_tuple_element(element)
                    || matches!(element, Expr::Slice(slice) if constant_slice(slice).is_some())
            })
        {
            self.assembler
                .record_last_uniform_fold_owner_slots(hidden_owner_slots);
        } else if matches!(expression, Expr::UnaryOp(unary)
            if unary.op == UnaryOp::USub
                && matches!(unary.operand.as_ref(), Expr::NumberLiteral(_)))
        {
            // Numeric negation removes its adjacent, equally owned literal load.
            self.assembler.record_last_uniform_fold_owner_slots(1);
        } else if matches!(expression, Expr::BinOp(_))
            && is_literal_binary_tree(expression)
            && self.source_location(expression.range()).line
                == self.source_location(expression.range()).end_line
        {
            // Each folded binary operation removes two adjacent literal loads. A single
            // source line keeps all nested operands in the same owner without line NOPs.
            self.assembler
                .record_last_uniform_fold_owner_slots(hidden_owner_slots);
        }
        if let Some(inner) = double_ellipsis
            && self.source_location(expression.range()).line
                != self.source_location(inner.range).line
        {
            self.assembler
                .set_location(self.source_location(expression.range()));
            self.emit(NOP, 0, 0)?;
        }
        Ok(())
    }

    /// Captures removed operands for a collection synthesized from constant expressions.
    pub(super) fn record_folded_collection_operands<'ast>(
        &mut self,
        expressions: impl Iterator<Item = &'ast Expr>,
    ) -> Result<Option<std::ops::Range<usize>>, CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            return Ok(None);
        }
        let first = self.output.deferred_constants.len();
        let mut needs_admissions = false;
        for expression in expressions {
            self.record_folded_value(expression)?;
            needs_admissions |=
                contains_negative_empty_repetition(expression, self.context.python_version);
        }
        let operands = first..self.output.deferred_constants.len();
        Ok((needs_admissions && !operands.is_empty()).then_some(operands))
    }

    /// Attaches one expression's removed operands to its surviving instruction.
    pub(super) fn attach_folded_operand_admissions(
        &mut self,
        operands: Option<std::ops::Range<usize>>,
    ) {
        let Some(operands) = operands else { return };
        let admission = self.assembler.reserve_last_fold_admissions();
        for (target, _) in &mut self.output.deferred_constants[operands] {
            debug_assert!(target.is_none());
            *target = Some(DeferredConstantTarget::FoldedOperand(admission));
        }
    }

    pub(super) fn record_folded_operands(&mut self, expression: &Expr) -> Result<(), CompileError> {
        match expression {
            Expr::BinOp(binary) => {
                self.record_folded_value(&binary.left)?;
                self.record_folded_value(&binary.right)?;
            }
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.record_folded_value(element)?;
                }
            }
            Expr::Subscript(subscript) => {
                self.record_folded_value(&subscript.value)?;
                if self.context.python_version <= PythonVersion::Py313 {
                    self.record_folded_value(&subscript.slice)?;
                } else if let Expr::Slice(slice) = subscript.slice.as_ref() {
                    if let Some(constant) = self.source_aware_constant_slice(slice) {
                        self.add_constant(constant)?;
                    } else {
                        for bound in [slice.lower.as_deref(), slice.upper.as_deref()]
                            .into_iter()
                            .flatten()
                        {
                            self.record_folded_value(bound)?;
                        }
                    }
                } else if !matches!(
                    fold_constant_for_version(&subscript.slice, self.context.python_version),
                    Some(Constant::Int(value)) if u8::try_from(value).is_ok()
                ) {
                    self.record_folded_value(&subscript.slice)?;
                }
            }
            Expr::UnaryOp(unary) => self.record_folded_value(&unary.operand)?,
            _ => {}
        }
        if self.context.python_version <= PythonVersion::Py313 {
            return Ok(());
        }
        let operands: Vec<_> = match expression {
            Expr::BinOp(binary) => [&*binary.left, &*binary.right]
                .into_iter()
                .filter_map(|value| self.source_aware_fold_constant(value))
                .collect(),
            Expr::UnaryOp(unary) => self
                .source_aware_fold_constant(&unary.operand)
                .into_iter()
                .collect(),
            Expr::Subscript(subscript) => [&*subscript.value, &*subscript.slice]
                .into_iter()
                .filter_map(|value| self.source_aware_fold_constant(value))
                .collect(),
            _ => Vec::new(),
        };
        if !operands.is_empty()
            && let Some(result) = self.source_aware_fold_constant(expression)
        {
            self.record_folded_constant_dependencies(&result, operands);
        }
        Ok(())
    }

    /// Retains the operands used for a constant's first flowgraph construction.
    pub(super) fn record_folded_constant_dependencies(
        &mut self,
        result: &Constant,
        operands: Vec<Constant>,
    ) {
        self.output
            .record_folded_constant_dependencies(result, operands);
    }
}

impl OutputState {
    /// Keeps each fold result and its construction dependencies in one allocation event.
    fn reallocate_non_reflexive_value(
        &mut self,
        mut constant: Constant,
        remap: &mut NonReflexiveRemap,
    ) -> Constant {
        fn collect(
            constant: &Constant,
            dependencies: &[(Constant, Vec<Constant>)],
            seen: &mut Vec<Constant>,
            collected: &mut Vec<(Constant, Vec<Constant>)>,
        ) {
            if seen
                .iter()
                .any(|value| ConstantEquality::PoolKey.equal(value, constant))
            {
                return;
            }
            seen.push(constant.clone());
            if let Some((result, operands)) = dependencies
                .iter()
                .find(|(result, _)| ConstantEquality::PoolKey.equal(result, constant))
            {
                for operand in operands {
                    collect(operand, dependencies, seen, collected);
                }
                if result.has_non_reflexive_number() {
                    collected.push((result.clone(), operands.clone()));
                }
            }
            if let Constant::Tuple(values) | Constant::FrozenSet(values) = constant {
                for value in values {
                    collect(value, dependencies, seen, collected);
                }
            }
        }
        let mut dependencies = Vec::new();
        collect(
            &constant,
            &self.folded_constant_dependencies,
            &mut Vec::new(),
            &mut dependencies,
        );
        constant.reallocate_non_reflexive_numbers(remap);
        for (result, operands) in &mut dependencies {
            result.reallocate_non_reflexive_numbers(remap);
            for operand in operands {
                operand.reallocate_non_reflexive_numbers(remap);
            }
        }
        self.folded_constant_dependencies.extend(dependencies);
        constant
    }

    /// Preserves operand admissions for containers created by late graph folding.
    fn record_folded_constant_dependencies(
        &mut self,
        result: &Constant,
        mut operands: Vec<Constant>,
    ) {
        if matches!(result, Constant::FrozenSet(_)) {
            // Cached operands retain their earlier construction order, including folds
            // from discarded branches preceding this set display.
            operands.sort_by_cached_key(|operand| {
                self.constants
                    .values
                    .iter()
                    .chain(self.deferred_constants.iter().map(|(_, value)| value))
                    .position(|value| ConstantEquality::PoolKey.equal(value, operand))
                    .unwrap_or(usize::MAX)
            });
        }
        let already_reserved = self
            .constants
            .values
            .iter()
            .chain(self.deferred_constants.iter().map(|(_, constant)| constant))
            .any(|constant| ConstantEquality::PoolKey.equal(constant, result));
        if !already_reserved {
            self.folded_constant_dependencies
                .push((result.clone(), operands));
        }
    }
}

impl Compiler<'_> {
    /// Emits operand-load residue retained when CPython's flowgraph folds a tuple containing
    /// constant boolean negations.
    pub(super) fn emit_folded_tuple_not_nops(
        &mut self,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            return Ok(());
        }
        match expression {
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.emit_folded_tuple_not_nops(element)?;
                }
            }
            Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => {
                let double_ellipsis = double_negated_ellipsis(expression).is_some();
                if double_ellipsis {
                    // A later tuple fold also removes the inner surviving load, while
                    // both negation locations still precede the tuple's own load.
                    self.emit_folded_tuple_not_nops(&unary.operand)?;
                    if self.source_location(expression.range()).line
                        != self.source_location(unary.operand.range()).line
                    {
                        self.assembler
                            .set_location(self.source_location(unary.operand.range()));
                        self.emit(NOP, 0, 0)?;
                    }
                    self.assembler
                        .set_location(self.source_location(expression.range()));
                    self.emit(NOP, 0, 0)?;
                } else {
                    self.assembler
                        .set_location(self.source_location(unary.operand.range()));
                    self.emit(NOP, 0, 0)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn record_folded_value(&mut self, expression: &Expr) -> Result<(), CompileError> {
        let literal = match expression {
            Expr::StringLiteral(string) => Some(self.string_literal_constant(string)),
            _ => literal_constant(expression),
        };
        if let Some(constant) = literal {
            self.add_constant(constant)?;
            return Ok(());
        }
        if self.context.python_version >= PythonVersion::Py314
            && matches!(expression, Expr::Slice(_))
            && let Some(constant) = self.source_aware_fold_constant(expression)
        {
            self.add_constant(constant)?;
            return Ok(());
        }
        if matches!(expression, Expr::Name(name) if name.id.as_str() == "__debug__") {
            self.add_constant(Constant::Bool(true))?;
            return Ok(());
        }
        if matches!(expression, Expr::FString(_))
            && let Some(constant) = self.source_aware_fold_constant(expression)
        {
            self.add_constant(constant)?;
            return Ok(());
        }

        if self.context.python_version <= PythonVersion::Py313 {
            if let Expr::Slice(slice) = expression {
                // Marshal v4 cannot encode slices: code generation loads their bounds.
                for bound in [&slice.lower, &slice.upper] {
                    if let Some(bound) = bound {
                        self.record_folded_value(bound)?;
                    } else {
                        self.add_constant(Constant::None)?;
                    }
                }
                if let Some(step) = &slice.step {
                    self.record_folded_value(step)?;
                }
            } else if let Some(constant) = self.source_aware_fold_constant(expression) {
                self.add_constant(constant)?;
            } else {
                self.record_folded_operands(expression)?;
            }
            return Ok(());
        }

        self.record_folded_operands(expression)?;
        if let Some(constant) = self.source_aware_fold_constant(expression)
            && !matches!(constant, Constant::Int(value) if u8::try_from(value).is_ok())
        {
            self.record_fresh_empty_bytes_fold(expression, &constant);
            self.output.deferred_constants.push((None, constant));
        }
        Ok(())
    }

    pub(super) fn emit_preprocessed_constant(
        &mut self,
        expression: &Expr,
        constant: Constant,
    ) -> Result<(), CompileError> {
        if self.context.python_version >= PythonVersion::Py314
            && let Constant::Int(value) = constant
            && let Ok(value) = u8::try_from(value)
        {
            if self.output.constants.is_empty()
                && let Some(seed) = first_literal_constant(expression)
            {
                self.add_constant(seed)?;
            }
            self.emit(LOAD_SMALL_INT, u32::from(value), 1)
        } else {
            let index = self.add_constant(constant)?;
            self.emit(LOAD_CONST, index, 1)
        }
    }

    pub(super) fn name_index(&mut self, name: &str) -> Result<u32, CompileError> {
        let name_id = self.required_name_id(name);
        self.name_index_with_id(name, name_id)
    }

    pub(super) fn name_index_with_id(
        &mut self,
        name: &str,
        name_id: NameId,
    ) -> Result<u32, CompileError> {
        let name = if self
            .unit
            .mangled_names
            .as_ref()
            .is_some_and(|names| !names.contains(name))
        {
            Cow::Borrowed(name)
        } else {
            mangle_name(name, self.unit.private_name.as_deref())
        };
        let name_id = match &name {
            Cow::Borrowed(_) => name_id,
            Cow::Owned(name) => self.required_name_id(name),
        };
        match self.output.name_indices.entry(name_id) {
            Entry::Occupied(entry) => Ok(*entry.get()),
            Entry::Vacant(entry) => {
                let index = to_u32(self.output.names.len(), "name count")?;
                self.output.names.push(MetadataName::from(name));
                self.output.name_ids.push(MetadataId::from(name_id));
                entry.insert(index);
                Ok(index)
            }
        }
    }

    pub(super) fn emit_unsigned_constant(&mut self, value: u32) -> Result<(), CompileError> {
        if self.context.python_version >= PythonVersion::Py314 && u8::try_from(value).is_ok() {
            self.emit(LOAD_SMALL_INT, value, 1)
        } else {
            // Older targets load every integer from the constant table; newer targets
            // use that table only beyond LOAD_SMALL_INT's 255 limit.
            let constant = self.add_constant(Constant::Int(u64::from(value)))?;
            self.emit(LOAD_CONST, constant, 1)
        }
    }

    pub(super) fn add_constant(&mut self, constant: Constant) -> Result<u32, CompileError> {
        #[cfg(test)]
        if FORCE_GENERIC_CONSTANT_ADMISSION.get() {
            return self.add_constant_generic(constant);
        }

        match &constant {
            Constant::None
            | Constant::Bool(_)
            | Constant::Ellipsis
            | Constant::Int(_)
            | Constant::SignedInt(_)
            | Constant::BigInt { .. }
            | Constant::Float(_)
            | Constant::Complex { .. }
            | Constant::SurrogateString(_) => {
                // These leaves carry no numeric allocation or native-cache provenance.
            }
            Constant::String(_) => self
                .context
                .record_constant_string_origins(&constant, false),
            Constant::Bytes(_) => self.context.record_constant_empty_bytes_origin(&constant),
            Constant::Code(code) => {
                // Nested constants were already admitted while compiling their own code.
                self.output.has_frozenset_constants |=
                    code.marshal_provenance.subtree_has_frozenset_constants;
            }
            Constant::NonReflexive(_)
            | Constant::Tuple(_)
            | Constant::FrozenSet(_)
            | Constant::Slice { .. } => return self.add_constant_generic(constant),
        }
        self.output.constants.intern(constant)
    }

    fn add_constant_generic(&mut self, constant: Constant) -> Result<u32, CompileError> {
        let constant = if self.context.python_version >= PythonVersion::Py314
            && constant.has_source_non_reflexive_number()
        {
            self.output
                .reallocate_non_reflexive_value(constant, &mut NonReflexiveRemap::default())
        } else {
            constant
        };
        self.context.record_constant_empty_bytes_origin(&constant);
        self.context
            .record_constant_string_origins(&constant, false);
        self.output.has_frozenset_constants |= matches!(constant, Constant::FrozenSet(_));
        if let Constant::Code(code) = &constant {
            self.output.has_frozenset_constants |=
                code.marshal_provenance.subtree_has_frozenset_constants;
        }
        self.output.constants.intern(constant)
    }

    pub(super) fn retain_discarded_first_constant(
        &mut self,
        discarded: &Self,
    ) -> Result<(), CompileError> {
        debug_assert!(self.output.constants.is_empty());
        if let Some(constant) = discarded.output.constants.values.first() {
            // The child builder already recorded cache admissions. Copy only the parent's
            // retained slot, without replaying those compilation-wide chronology events.
            self.output.has_frozenset_constants |= matches!(constant, Constant::FrozenSet(_));
            if let Constant::Code(code) = constant {
                self.output.has_frozenset_constants |=
                    code.marshal_provenance.subtree_has_frozenset_constants;
            }
            self.output.constants.intern(constant.clone())?;
            self.output
                .interned_constant_strings
                .extend(discarded.output.interned_constant_strings.iter().cloned());
        }
        Ok(())
    }

    pub(super) fn add_interned_string_tuple(
        &mut self,
        values: Vec<Constant>,
    ) -> Result<u32, CompileError> {
        for value in values.iter().filter_map(|value| match value {
            Constant::String(value) => Some(value),
            _ => None,
        }) {
            self.context.record_constant_string_origin(value, true);
            // Recursive constant-cache insertion can replace a parser-interned name
            // with an earlier equal literal. Preserve the winning object's identity.
            if self.context.constant_string_origin_is_interned(value)
                && !self
                    .output
                    .interned_constant_strings
                    .contains(value.as_str())
            {
                self.output.interned_constant_strings.insert(value.clone());
            }
        }
        self.add_constant(Constant::Tuple(values))
    }

    pub(super) fn remove_unused_constants(&mut self) {
        let deferred_none = self.assembler.deferred_none_constant_index();
        if self.output.constants.len() <= 1 && deferred_none.is_none() {
            return;
        }
        let mut used = self
            .assembler
            .used_constant_indices(self.output.constants.len());
        if let Some(index) = deferred_none {
            used[usize::try_from(index).expect("constant index should fit in usize")] = true;
        }
        if deferred_none.is_none() && used.iter().skip(1).all(|used| *used) {
            return;
        }
        let index_map = self.output.constants.retain_used(&used);
        self.output.first_flowgraph_constant = index_map
            .iter()
            .take(self.output.first_flowgraph_constant)
            .flatten()
            .count();
        self.assembler.remap_constant_indices(&index_map);
        self.output.qualified_name_constant_index =
            self.output.qualified_name_constant_index.and_then(|index| {
                let index = usize::try_from(index).expect("constant index always fits in usize");
                index_map[index]
            });
    }

    fn record_fresh_empty_bytes_fold(&mut self, expression: &Expr, constant: &Constant) {
        if matches!(constant, Constant::Bytes(value) if value.is_empty())
            && matches!(expression, Expr::BinOp(binary) if binary.op == Operator::Mult)
        {
            self.output
                .fresh_empty_bytes_deferred
                .insert(self.output.deferred_constants.len());
        }
    }

    pub(super) fn emit_deferred_constant(
        &mut self,
        constant: Constant,
    ) -> Result<(), CompileError> {
        self.apply_compiler_stack_effect(LOAD_CONST, 0, 1)?;
        let instruction = self
            .assembler
            .emit_placeholder_with_depth(LOAD_CONST, self.control.stack.depth.cast_unsigned());
        self.output.deferred_constants.push((
            Some(DeferredConstantTarget::Instruction(instruction)),
            constant,
        ));
        Ok(())
    }

    pub(super) fn emit_deferred_constant_before_return(
        &mut self,
        constant: Constant,
    ) -> Result<(), CompileError> {
        self.apply_compiler_stack_effect(LOAD_CONST, 0, 1)?;
        let instruction = self
            .assembler
            .emit_placeholder_with_depth(LOAD_CONST, self.control.stack.depth.cast_unsigned());
        self.output
            .deferred_constants_before_return
            .push((instruction, constant));
        Ok(())
    }

    pub(super) fn emit_deferred_store_name(&mut self, name: &str) -> Result<(), CompileError> {
        self.apply_compiler_stack_effect(STORE_NAME, 0, -1)?;
        let instruction = self
            .assembler
            .emit_placeholder_with_depth(STORE_NAME, self.control.stack.depth.cast_unsigned());
        self.output
            .deferred_names
            .push((instruction, name.to_string()));
        Ok(())
    }
}

/// Returns whether a binary-expression tree contains only primitive literal leaves.
fn is_literal_binary_tree(expression: &Expr) -> bool {
    match expression {
        Expr::BinOp(binary) => {
            is_literal_binary_tree(&binary.left) && is_literal_binary_tree(&binary.right)
        }
        _ => is_literal_constant(expression),
    }
}

/// Identifies literal tuple trees whose loads and builds share one original CFG array.
///
/// Tuples above the stack-use limit use list append operations instead of `BUILD_TUPLE`.
/// Nonliteral folds may retain line NOPs or contain control-flow boundaries.
pub(super) fn is_small_literal_tuple_element(expression: &Expr) -> bool {
    match expression {
        Expr::Tuple(tuple) => {
            tuple.elts.len() <= 30 && tuple.elts.iter().all(is_small_literal_tuple_element)
        }
        _ => is_literal_constant(expression),
    }
}

/// Returns instruction slots removed when CPython's flow graph folds `expression`.
///
/// The count is attached to the emitted folded result so labels and other control-flow
/// boundaries naturally constrain which later boolean marker can reuse the stale slots.
pub(super) fn folded_constant_owner_slots(
    expression: &Expr,
    python_version: PythonVersion,
) -> usize {
    fn direct(expression: &Expr, python_version: PythonVersion) -> usize {
        match expression {
            Expr::BinOp(binary) => {
                2 + direct(&binary.left, python_version) + direct(&binary.right, python_version)
            }
            Expr::Tuple(tuple) => tuple.elts.iter().fold(tuple.elts.len(), |slots, element| {
                slots + direct(element, python_version)
            }),
            Expr::UnaryOp(unary) => {
                if unary.op == UnaryOp::Not
                    && matches!(unary.operand.as_ref(), Expr::Tuple(_))
                    && fold_constant_for_version(&unary.operand, python_version).is_none()
                {
                    1
                } else {
                    1 + direct(&unary.operand, python_version)
                }
            }
            Expr::Subscript(subscript) => {
                2 + direct(&subscript.value, python_version)
                    + direct(&subscript.slice, python_version)
            }
            _ => 0,
        }
    }

    direct(expression, python_version)
}

pub(super) fn explicit_surrogate_string(value: &str, source: &str) -> Option<Vec<u8>> {
    let source = source.as_bytes();
    // Ruff stores invalid Unicode escapes as replacement characters. Retain their source order
    // relative to real replacement characters so the marshal layer can restore surrogate-pass
    // UTF-8 without changing a neighboring U+FFFD.
    let replacement_character = [0xef, 0xbf, 0xbd];
    let mut replacements = Vec::new();
    let mut index = 0;
    while index < source.len() {
        if source[index..].starts_with(&replacement_character) {
            replacements.push(None);
            index += replacement_character.len();
            continue;
        }
        if source[index] != b'\\' {
            index += 1;
            continue;
        }
        let preceding_backslashes = source[..index]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if preceding_backslashes % 2 == 1 {
            index += 1;
            continue;
        }
        if source[index..].starts_with(b"\\N{")
            && let Some(end) = source[index + 3..].iter().position(|byte| *byte == b'}')
        {
            let end = index + 3 + end;
            if source[index + 3..end].eq_ignore_ascii_case(b"REPLACEMENT CHARACTER") {
                replacements.push(None);
            }
            index = end + 1;
            continue;
        }
        let (digit_count, escape_len) = match source.get(index + 1) {
            Some(b'u') => (4, 6),
            Some(b'U') => (8, 10),
            _ => {
                index += 1;
                continue;
            }
        };
        let Some(end) = index
            .checked_add(escape_len)
            .filter(|end| *end <= source.len())
        else {
            index += 1;
            continue;
        };
        let Some(codepoint) = std::str::from_utf8(&source[index + 2..index + 2 + digit_count])
            .ok()
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        else {
            index += 1;
            continue;
        };
        if (0xd800..=0xdfff).contains(&codepoint) {
            replacements.push(Some(
                u16::try_from(codepoint).expect("surrogate code point fits in u16"),
            ));
        } else if codepoint == 0xfffd {
            replacements.push(None);
        }
        index = end;
    }
    if !replacements.iter().any(Option::is_some) {
        return None;
    }

    let mut bytes = value.as_bytes().to_vec();
    let mut search_start = 0;
    let mut replaced = false;
    for surrogate in replacements {
        let Some(offset) = bytes[search_start..]
            .windows(replacement_character.len())
            .position(|window| window == replacement_character)
            .map(|offset| search_start + offset)
        else {
            continue;
        };
        let Some(surrogate) = surrogate else {
            search_start = offset + replacement_character.len();
            continue;
        };
        let surrogate_bytes = [
            0xe0 | (surrogate >> 12) as u8,
            0x80 | ((surrogate >> 6) & 0x3f) as u8,
            0x80 | (surrogate & 0x3f) as u8,
        ];
        bytes.splice(
            offset..offset + replacement_character.len(),
            surrogate_bytes,
        );
        search_start = offset + surrogate_bytes.len();
        replaced = true;
    }
    replaced.then_some(bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConstantEquality {
    /// Constant-table equality, including type, signed zeros, and NaN allocation identity.
    PoolKey,
    /// Python value equality used while folding set-like literals and membership tests.
    PythonValue,
}

impl ConstantEquality {
    #[expect(
        clippy::float_cmp,
        reason = "CPython constant deduplication uses exact numeric equality"
    )]
    fn equal(self, left: &Constant, right: &Constant) -> bool {
        if let (Constant::NonReflexive(left), Constant::NonReflexive(right)) = (left, right) {
            return left.key() == right.key();
        }
        if matches!(self, Self::PythonValue)
            && let (Some((left_real, left_imag)), Some((right_real, right_imag))) =
                (numeric_parts(left), numeric_parts(right))
        {
            return left_imag == right_imag && numeric_reals_equal(&left_real, &right_real);
        }

        match (left, right) {
            (Constant::None, Constant::None) | (Constant::Ellipsis, Constant::Ellipsis) => true,
            (Constant::Bool(left), Constant::Bool(right)) => left == right,
            (Constant::Int(left), Constant::Int(right)) => left == right,
            (Constant::SignedInt(left), Constant::SignedInt(right)) => left == right,
            (
                Constant::BigInt {
                    negative: left_negative,
                    digits: left_digits,
                },
                Constant::BigInt {
                    negative: right_negative,
                    digits: right_digits,
                },
            ) => left_negative == right_negative && left_digits == right_digits,
            (Constant::Float(left), Constant::Float(right)) => left.to_bits() == right.to_bits(),
            (
                Constant::Complex {
                    real: left_real,
                    imag: left_imag,
                },
                Constant::Complex {
                    real: right_real,
                    imag: right_imag,
                },
            ) => {
                left_real.to_bits() == right_real.to_bits()
                    && left_imag.to_bits() == right_imag.to_bits()
            }
            (Constant::String(left), Constant::String(right)) => left == right,
            (Constant::SurrogateString(left), Constant::SurrogateString(right)) => left == right,
            (Constant::Bytes(left), Constant::Bytes(right)) => left == right,
            (Constant::Tuple(left), Constant::Tuple(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| self.equal(left, right))
            }
            (Constant::FrozenSet(left), Constant::FrozenSet(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .all(|left| right.iter().any(|right| self.equal(left, right)))
            }
            (
                Constant::Slice {
                    lower: left_lower,
                    upper: left_upper,
                    step: left_step,
                },
                Constant::Slice {
                    lower: right_lower,
                    upper: right_upper,
                    step: right_step,
                },
            ) => {
                // Python-value comparison is recursive only for tuple and frozenset constants.
                // Slice objects retain the constant-pool identity of their members.
                Self::PoolKey.equal(left_lower, right_lower)
                    && Self::PoolKey.equal(left_upper, right_upper)
                    && Self::PoolKey.equal(left_step, right_step)
            }
            (Constant::Code(left), Constant::Code(right)) => code_objects_equal(left, right),
            (_, _) => false,
        }
    }
}

#[derive(Debug)]
enum NumericReal {
    Integer { negative: bool, digits: Vec<u16> },
    Float(f64),
}

pub(super) fn python_constant_values_equal(left: &Constant, right: &Constant) -> bool {
    ConstantEquality::PythonValue.equal(left, right)
}

fn numeric_parts(value: &Constant) -> Option<(NumericReal, f64)> {
    match value {
        Constant::Bool(value) => Some((integer_numeric_real(u64::from(*value), false), 0.0)),
        Constant::Int(value) => Some((integer_numeric_real(*value, false), 0.0)),
        Constant::SignedInt(value) => Some((
            integer_numeric_real(value.unsigned_abs(), value.is_negative()),
            0.0,
        )),
        Constant::BigInt { negative, digits } => Some((
            NumericReal::Integer {
                negative: *negative,
                digits: normalized_digits(digits.clone()),
            },
            0.0,
        )),
        Constant::NonReflexive(number) => numeric_parts(&number.value()),
        Constant::Float(value) => Some((NumericReal::Float(*value), 0.0)),
        Constant::Complex { real, imag } => Some((NumericReal::Float(*real), *imag)),
        _ => None,
    }
}

fn integer_numeric_real(value: u64, negative: bool) -> NumericReal {
    let digits = integer_digits(value);
    NumericReal::Integer {
        negative: negative && !digits.is_empty(),
        digits,
    }
}

fn normalized_digits(mut digits: Vec<u16>) -> Vec<u16> {
    while digits.last() == Some(&0) {
        digits.pop();
    }
    digits
}

#[expect(
    clippy::float_cmp,
    reason = "CPython constant deduplication uses exact numeric equality"
)]
fn numeric_reals_equal(left: &NumericReal, right: &NumericReal) -> bool {
    match (left, right) {
        (NumericReal::Float(left), NumericReal::Float(right)) => left == right,
        (
            NumericReal::Integer {
                negative: left_negative,
                digits: left_digits,
            },
            NumericReal::Integer {
                negative: right_negative,
                digits: right_digits,
            },
        ) => left_negative == right_negative && left_digits == right_digits,
        (integer @ NumericReal::Integer { .. }, NumericReal::Float(float))
        | (NumericReal::Float(float), integer @ NumericReal::Integer { .. }) => {
            float_integer_numeric_real(*float)
                .is_some_and(|float_integer| numeric_reals_equal(integer, &float_integer))
        }
    }
}

fn float_integer_numeric_real(value: f64) -> Option<NumericReal> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some(integer_numeric_real(0, false));
    }

    let bits = value.to_bits();
    let negative = bits >> 63 != 0;
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    let (mut mantissa, exponent) = if exponent_bits == 0 {
        (fraction, -1074)
    } else {
        ((1_u64 << 52) | fraction, exponent_bits - 1023 - 52)
    };

    if exponent < 0 {
        let shift = u32::try_from(-exponent).ok()?;
        if shift >= 64 || mantissa & ((1_u64 << shift) - 1) != 0 {
            return None;
        }
        mantissa >>= shift;
        return Some(integer_numeric_real(mantissa, negative));
    }

    Some(NumericReal::Integer {
        negative,
        digits: left_shift_magnitude(&integer_digits(mantissa), usize::try_from(exponent).ok()?),
    })
}

fn code_objects_equal(left: &CodeObject, right: &CodeObject) -> bool {
    left.first_line_number == right.first_line_number
        && left.name == right.name
        && left.qualified_name == right.qualified_name
        && left.arg_count == right.arg_count
        && left.positional_only_arg_count == right.positional_only_arg_count
        && left.keyword_only_arg_count == right.keyword_only_arg_count
        && left.stack_size == right.stack_size
        && left.flags == right.flags
        && left.bytecode == right.bytecode
        && left.constants.len() == right.constants.len()
        && left
            .constants
            .iter()
            .zip(&right.constants)
            .all(|(left, right)| ConstantEquality::PoolKey.equal(left, right))
        && left.names == right.names
        && left.locals == right.locals
        && left.local_kinds == right.local_kinds
        && left.filename == right.filename
        && left.line_table == right.line_table
        && left.exception_table == right.exception_table
        && left
            .marshal_provenance
            .equal_for_constant_pool(&right.marshal_provenance)
}

/// Preserves numeric literal representation for both folding and direct emission.
/// Integers use `Int` when they fit in `u64`, otherwise unsigned `BigInt` digits;
/// float and complex components are copied without changing their bits.
///
/// ```python
/// values = (42, 18446744073709551616, 1.5, 2j)
/// ```
pub(super) fn number_constant(number: &Number) -> Constant {
    match number {
        Number::Int(value) => value.as_u64().map_or_else(
            || Constant::BigInt {
                negative: false,
                digits: big_integer_digits(&value.to_string()),
            },
            Constant::Int,
        ),
        Number::Float(value) => Constant::Float(*value),
        Number::Complex { real, imag } => Constant::Complex {
            real: *real,
            imag: *imag,
        },
    }
}

pub(super) fn literal_constant(expression: &Expr) -> Option<Constant> {
    match expression {
        Expr::NoneLiteral(_) => Some(Constant::None),
        Expr::BooleanLiteral(boolean) => Some(Constant::Bool(boolean.value)),
        Expr::EllipsisLiteral(_) => Some(Constant::Ellipsis),
        Expr::NumberLiteral(number) => Some(number_constant(&number.value)),
        Expr::StringLiteral(string) => Some(Constant::String(string.value.to_str().to_string())),
        Expr::BytesLiteral(bytes) => Some(Constant::Bytes(bytes.value.bytes().collect())),
        _ => None,
    }
}
pub(super) enum PercentFormatPart<'a> {
    Literal(Constant),
    Formatted {
        expression: &'a Expr,
        conversion: Conversion,
        format_spec: Option<String>,
    },
}

pub(super) fn optimized_percent_format(
    expression: &ExprBinOp,
    decode_literal: impl FnOnce(&ExprStringLiteral) -> Constant,
) -> Option<Vec<PercentFormatPart<'_>>> {
    if expression.op != Operator::Mod {
        return None;
    }
    let Expr::StringLiteral(format) = expression.left.as_ref() else {
        return None;
    };
    let Expr::Tuple(arguments) = expression.right.as_ref() else {
        return None;
    };
    if arguments
        .elts
        .iter()
        .any(|argument| matches!(argument, Expr::Starred(_)))
    {
        return None;
    }

    // Percent-format syntax is ASCII, so parsing bytes preserves surrogate-pass UTF-8 in
    // literal fragments without changing the supported conversions or width/precision limits.
    let format = match decode_literal(format) {
        Constant::String(value) => value.into_bytes(),
        Constant::SurrogateString(value) => value,
        _ => unreachable!("string literal decoding returns a string constant"),
    };
    let mut position = 0;
    let mut argument_index = 0;
    let mut parts = Vec::with_capacity(arguments.elts.len() * 2 + 1);
    while position < format.len() {
        let mut literal = Vec::new();
        while position < format.len() {
            if format[position] != b'%' {
                literal.push(format[position]);
                position += 1;
            } else if format.get(position + 1) == Some(&b'%') {
                literal.push(b'%');
                position += 2;
            } else {
                break;
            }
        }
        if !literal.is_empty() {
            let literal = String::from_utf8(literal).map_or_else(
                |error| Constant::SurrogateString(error.into_bytes()),
                Constant::String,
            );
            parts.push(PercentFormatPart::Literal(literal));
        }
        if position == format.len() {
            break;
        }

        let argument = arguments.elts.get(argument_index)?;
        argument_index += 1;
        position += 1;
        let (conversion, format_spec) = parse_percent_format(&format, &mut position)?;
        parts.push(PercentFormatPart::Formatted {
            expression: argument,
            conversion,
            format_spec,
        });
    }
    (argument_index == arguments.elts.len()).then_some(parts)
}

fn parse_percent_format(
    format: &[u8],
    position: &mut usize,
) -> Option<(Conversion, Option<String>)> {
    const MAX_DIGITS: usize = 3;

    let mut next = || {
        let character = format.get(*position).copied()?;
        *position += 1;
        Some(character)
    };

    let mut left_justify = false;
    let mut character = loop {
        let character = next()?;
        match character {
            b'-' => left_justify = true,
            b'+' | b' ' | b'#' | b'0' => {}
            _ => break character,
        }
    };

    let mut width = None;
    if character.is_ascii_digit() {
        let mut value = 0_u32;
        let mut digits = 0;
        while character.is_ascii_digit() {
            value = value * 10 + u32::from(character - b'0');
            character = next()?;
            digits += 1;
            if digits >= MAX_DIGITS {
                return None;
            }
        }
        width = Some(value);
    }

    let mut precision = None;
    if character == b'.' {
        character = next()?;
        let mut value = 0_u32;
        if character.is_ascii_digit() {
            let mut digits = 0;
            while character.is_ascii_digit() {
                value = value * 10 + u32::from(character - b'0');
                character = next()?;
                digits += 1;
                if digits >= MAX_DIGITS {
                    return None;
                }
            }
        }
        precision = Some(value);
    }

    let conversion = match character {
        b's' => Conversion::STR,
        b'r' => Conversion::REPR,
        b'a' => Conversion::ASCII,
        _ => return None,
    };
    let mut format_spec = String::new();
    if !left_justify && width.is_some_and(|width| width > 0) {
        format_spec.push('>');
    }
    if let Some(width) = width {
        use std::fmt::Write;
        write!(format_spec, "{width}").unwrap();
    }
    if let Some(precision) = precision {
        use std::fmt::Write;
        write!(format_spec, ".{precision}").unwrap();
    }

    Some((conversion, (!format_spec.is_empty()).then_some(format_spec)))
}

pub(super) fn fold_constant(expression: &Expr) -> Option<Constant> {
    fold_constant_with_literals(expression, PythonVersion::Py314, &literal_constant, false)
}

/// Preserves neutral legacy folding while enabling modern sequence-count folds.
pub(super) fn fold_constant_for_version(
    expression: &Expr,
    python_version: PythonVersion,
) -> Option<Constant> {
    if python_version <= PythonVersion::Py313 {
        return fold_constant(expression);
    }
    fold_constant_with_literals(expression, python_version, &literal_constant, true)
}

/// Counts the consecutive negations applied directly to an Ellipsis literal.
fn negated_ellipsis_depth(mut expression: &Expr) -> Option<usize> {
    let mut depth = 0;
    while let Expr::UnaryOp(unary) = expression {
        if unary.op != UnaryOp::Not {
            return None;
        }
        depth += 1;
        expression = &unary.operand;
    }
    matches!(expression, Expr::EllipsisLiteral(_)).then_some(depth)
}

/// Finds the inner operation when exactly two negations are applied to Ellipsis.
pub(super) fn double_negated_ellipsis(expression: &Expr) -> Option<&ExprUnaryOp> {
    if let Expr::UnaryOp(outer) = expression
        && outer.op == UnaryOp::Not
        && let Expr::UnaryOp(inner) = outer.operand.as_ref()
        && inner.op == UnaryOp::Not
        && matches!(inner.operand.as_ref(), Expr::EllipsisLiteral(_))
    {
        Some(inner)
    } else {
        None
    }
}

/// Finds the one or two Ellipsis negations folded by the modern instruction pass.
pub(super) fn instruction_folded_ellipsis_depth(expression: &Expr) -> Option<usize> {
    if let Expr::UnaryOp(unary) = expression
        && unary.op == UnaryOp::Not
        && matches!(unary.operand.as_ref(), Expr::EllipsisLiteral(_))
    {
        Some(1)
    } else {
        double_negated_ellipsis(expression).map(|_| 2)
    }
}

/// Folds the same expression shapes using source-aware literal values when supplied.
pub(super) fn fold_constant_with_literals(
    expression: &Expr,
    python_version: PythonVersion,
    literal: &impl Fn(&Expr) -> Option<Constant>,
    boolean_sequence_repetition: bool,
) -> Option<Constant> {
    if let Some(constant) = literal(expression) {
        return Some(constant);
    }
    let result = match expression {
        Expr::Name(name) if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__" => {
            Some(Constant::Bool(true))
        }
        Expr::FString(fstring) => {
            let mut value = String::new();
            for part in &fstring.value {
                match part {
                    FStringPartRef::Literal(literal) => value.push_str(&literal.value),
                    FStringPartRef::FString(fstring) => {
                        for element in &fstring.elements {
                            let InterpolatedStringElement::Literal(literal) = element else {
                                return None;
                            };
                            value.push_str(&literal.value);
                        }
                    }
                }
            }
            Some(Constant::String(value))
        }
        Expr::Tuple(tuple) if tuple.ctx == ExprContext::Load => tuple
            .elts
            .iter()
            .map(|value| {
                fold_constant_with_literals(
                    value,
                    python_version,
                    literal,
                    boolean_sequence_repetition,
                )
            })
            .collect::<Option<Vec<_>>>()
            .map(Constant::Tuple),
        Expr::Subscript(subscript) if subscript.ctx == ExprContext::Load => fold_literal_subscript(
            &subscript.value,
            &subscript.slice,
            python_version,
            literal,
            boolean_sequence_repetition,
        ),
        Expr::Slice(slice) => constant_slice_with_literals(slice, literal),
        Expr::UnaryOp(unary) => match unary.op {
            UnaryOp::UAdd => match fold_constant_with_literals(
                &unary.operand,
                python_version,
                literal,
                boolean_sequence_repetition,
            )? {
                Constant::Bool(value) => Some(Constant::Int(u64::from(value))),
                constant @ (Constant::Int(_)
                | Constant::SignedInt(_)
                | Constant::BigInt { .. }
                | Constant::Float(_)
                | Constant::Complex { .. }
                | Constant::NonReflexive(_)) => Some(constant),
                _ => None,
            },
            // Modern CPython folds BUILD_TUPLE after its LOAD_CONST/TO_BOOL sweep.
            // The remaining TO_BOOL blocks UNARY_NOT folding, including when Boolean
            // repetition made the tuple's elements constant in that same later pass.
            UnaryOp::Not
                if boolean_sequence_repetition
                    && matches!(unary.operand.as_ref(), Expr::Tuple(_)) =>
            {
                None
            }
            UnaryOp::Not => {
                // This flag also distinguishes modern instruction folding from the
                // neutral legacy AST folding used by `fold_constant`.
                let ellipsis_depth = if boolean_sequence_repetition {
                    instruction_folded_ellipsis_depth(expression)
                } else {
                    negated_ellipsis_depth(expression)
                };
                if let Some(depth) = ellipsis_depth {
                    // Modern flowgraph optimization has already visited the initial
                    // conversion when it removes the second negation.
                    return Some(Constant::Bool(depth.is_multiple_of(2)));
                }
                if matches!(unary.operand.as_ref(), Expr::Tuple(_)) {
                    fold_constant_with_literals(
                        &unary.operand,
                        python_version,
                        literal,
                        boolean_sequence_repetition,
                    )?;
                }
                literal_truthiness(&unary.operand).map(|value| Constant::Bool(!value))
            }
            UnaryOp::USub => fold_constant_with_literals(
                &unary.operand,
                python_version,
                literal,
                boolean_sequence_repetition,
            )?
            .into_numeric_value()
            .negated(),
            UnaryOp::Invert => {
                let operand = fold_constant_with_literals(
                    &unary.operand,
                    python_version,
                    literal,
                    boolean_sequence_repetition,
                )?;
                // CPython leaves boolean inversion at runtime for its deprecation warning.
                if matches!(operand, Constant::Bool(_)) {
                    return None;
                }
                let value = folded_integer(&operand)?;
                // Python integer inversion is exactly -(value + 1).
                let digits = if value.negative {
                    subtract_magnitudes(&value.digits, &[1])
                } else {
                    add_magnitudes(&value.digits, &[1])
                };
                Some(integer_constant(!value.negative, digits))
            }
        },
        Expr::BinOp(binary) => {
            let left = fold_constant_with_literals(
                &binary.left,
                python_version,
                literal,
                boolean_sequence_repetition,
            )?
            .into_numeric_value();
            let right = fold_constant_with_literals(
                &binary.right,
                python_version,
                literal,
                boolean_sequence_repetition,
            )?
            .into_numeric_value();
            match (left, binary.op, right) {
                (left, operator @ (Operator::Add | Operator::Sub | Operator::Mult), right)
                    if is_integer_constant(&left) && is_integer_constant(&right) =>
                {
                    fold_integer_arithmetic(&left, operator, &right)
                }
                (left, Operator::Div, right)
                    if is_integer_constant(&left) && is_integer_constant(&right) =>
                {
                    fold_integer_true_divide(&left, &right).map(Constant::Float)
                }
                (
                    Constant::Int(left),
                    operator @ (Operator::FloorDiv | Operator::Mod),
                    Constant::Int(right),
                ) if right != 0 => Some(Constant::Int(if operator == Operator::FloorDiv {
                    left / right
                } else {
                    left % right
                })),
                (left, operator @ (Operator::FloorDiv | Operator::Mod), right)
                    if is_integer_constant(&left) && is_integer_constant(&right) =>
                {
                    fold_integer_divmod(&left, operator, &right)
                }
                (left, Operator::FloorDiv, right)
                    if matches!(left, Constant::Float(_))
                        || matches!(right, Constant::Float(_)) =>
                {
                    fold_float_floor_divide(&left, &right).map(Constant::Float)
                }
                (left, Operator::Mod, right)
                    if matches!(left, Constant::Float(_))
                        || matches!(right, Constant::Float(_)) =>
                {
                    fold_float_remainder(&left, &right).map(Constant::Float)
                }
                (left, Operator::Pow, right)
                    if is_integer_constant(&left) && is_integer_constant(&right) =>
                {
                    fold_integer_power(&left, &right)
                }
                (left, operator @ (Operator::LShift | Operator::RShift), right)
                    if is_integer_constant(&left) && is_integer_constant(&right) =>
                {
                    fold_integer_shift(&left, operator, &right)
                }
                (
                    left,
                    operator @ (Operator::BitAnd | Operator::BitOr | Operator::BitXor),
                    right,
                ) if is_integer_constant(&left) && is_integer_constant(&right) => {
                    fold_integer_bitwise(&left, operator, &right)
                }
                (Constant::Float(left), Operator::Add, Constant::Float(right)) => {
                    Some(Constant::Float(left + right))
                }
                (Constant::Float(left), Operator::Sub, Constant::Float(right)) => {
                    Some(Constant::Float(left - right))
                }
                (Constant::Float(left), Operator::Mult, Constant::Float(right)) => {
                    Some(Constant::Float(left * right))
                }
                (Constant::Float(left), Operator::Div, Constant::Float(right)) if right != 0.0 => {
                    Some(Constant::Float(left / right))
                }
                (
                    left,
                    operator @ (Operator::Add
                    | Operator::Sub
                    | Operator::Mult
                    | Operator::Div
                    | Operator::Pow),
                    right,
                ) if matches!(left, Constant::Float(_)) && is_integer_constant(&right)
                    || matches!(right, Constant::Float(_)) && is_integer_constant(&left) =>
                {
                    let left = constant_as_float(&left)?;
                    let right = constant_as_float(&right)?;
                    match operator {
                        Operator::Add => Some(Constant::Float(left + right)),
                        Operator::Sub => Some(Constant::Float(left - right)),
                        Operator::Mult => Some(Constant::Float(left * right)),
                        Operator::Div if right != 0.0 => Some(Constant::Float(left / right)),
                        Operator::Pow => fold_float_power(left, right),
                        Operator::Div => None,
                        _ => unreachable!(),
                    }
                }
                (Constant::Float(left), Operator::Pow, Constant::Float(right)) => {
                    fold_float_power(left, right)
                }
                (left, operator @ (Operator::Add | Operator::Sub | Operator::Mult), right)
                    if matches!(left, Constant::Complex { .. })
                        || matches!(right, Constant::Complex { .. }) =>
                {
                    fold_complex_arithmetic(&left, operator, &right, python_version)
                }
                (left, Operator::Div, right)
                    if matches!(left, Constant::Complex { .. })
                        || matches!(right, Constant::Complex { .. }) =>
                {
                    fold_complex_division(&left, &right, python_version)
                }
                (left, Operator::Pow, right)
                    if matches!(left, Constant::Complex { .. })
                        || matches!(right, Constant::Complex { .. }) =>
                {
                    fold_complex_power(&left, &right, python_version)
                }
                (Constant::String(mut left), Operator::Add, Constant::String(right)) => {
                    left.push_str(&right);
                    Some(Constant::String(left))
                }
                (Constant::SurrogateString(mut left), Operator::Add, Constant::String(right)) => {
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
                (Constant::Bytes(mut left), Operator::Add, Constant::Bytes(right)) => {
                    left.extend(right);
                    Some(Constant::Bytes(left))
                }
                (Constant::Tuple(mut left), Operator::Add, Constant::Tuple(right)) => {
                    left.extend(right);
                    Some(Constant::Tuple(left))
                }
                (Constant::String(value), Operator::Mult, times)
                | (times, Operator::Mult, Constant::String(value)) => {
                    let times = sequence_repeat_count(
                        &times,
                        boolean_sequence_repetition,
                        value.is_empty(),
                    )?;
                    let length = value.chars().count();
                    let Some(max_times) = 4096_usize.checked_div(length) else {
                        return Some(Constant::String(String::new()));
                    };
                    let times = usize::try_from(times).ok()?;
                    (times <= max_times).then(|| Constant::String(value.repeat(times)))
                }
                (Constant::SurrogateString(value), Operator::Mult, times)
                | (times, Operator::Mult, Constant::SurrogateString(value)) => {
                    let times = sequence_repeat_count(
                        &times,
                        boolean_sequence_repetition,
                        value.is_empty(),
                    )?;
                    let length = value.iter().filter(|byte| **byte & 0xc0 != 0x80).count();
                    let Some(max_times) = 4096_usize.checked_div(length) else {
                        return Some(Constant::String(String::new()));
                    };
                    let times = usize::try_from(times).ok()?;
                    (times <= max_times).then(|| surrogate_string_constant(value.repeat(times)))
                }
                (Constant::Bytes(value), Operator::Mult, times)
                | (times, Operator::Mult, Constant::Bytes(value)) => {
                    let times = sequence_repeat_count(
                        &times,
                        boolean_sequence_repetition,
                        value.is_empty(),
                    )?;
                    let Some(max_times) = 4096_usize.checked_div(value.len()) else {
                        return Some(Constant::Bytes(Vec::new()));
                    };
                    let times = usize::try_from(times).ok()?;
                    (times <= max_times).then(|| Constant::Bytes(value.repeat(times)))
                }
                (Constant::Tuple(value), Operator::Mult, times)
                | (times, Operator::Mult, Constant::Tuple(value)) => {
                    let times = sequence_repeat_count(
                        &times,
                        boolean_sequence_repetition,
                        value.is_empty(),
                    )?;
                    let Some(max_times) = 256_usize.checked_div(value.len()) else {
                        return Some(Constant::Tuple(Vec::new()));
                    };
                    let times = usize::try_from(times).ok()?;
                    if times > max_times {
                        return None;
                    }
                    if let Some(budget) = 1024_usize.checked_div(times) {
                        check_tuple_complexity(&value, budget)?;
                    }
                    Some(Constant::Tuple(
                        (0..times).flat_map(|_| value.iter().cloned()).collect(),
                    ))
                }
                _ => None,
            }
        }
        _ => None,
    };
    let origin = (u64::from(expression.range().start().to_u32()) << 32)
        | u64::from(expression.range().end().to_u32());
    result.map(|constant| constant.with_non_reflexive_origin(origin))
}

fn sequence_repeat_count(
    value: &Constant,
    boolean_sequence_repetition: bool,
    empty: bool,
) -> Option<u64> {
    match value {
        Constant::Bool(value) if boolean_sequence_repetition => Some(u64::from(*value)),
        // The pinned targets convert counts to 64-bit Py_ssize_t even for empty sequences.
        Constant::Int(value) if i64::try_from(*value).is_ok() => Some(*value),
        Constant::SignedInt(_) if empty => Some(0),
        _ => None,
    }
}

/// Identifies the newly folded repetitions whose reused result hides operand admissions.
fn contains_negative_empty_repetition(expression: &Expr, python_version: PythonVersion) -> bool {
    struct Finder {
        python_version: PythonVersion,
        found: bool,
    }
    impl<'ast> Visitor<'ast> for Finder {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.found {
                return;
            }
            if let Expr::BinOp(binary) = expression
                && binary.op == Operator::Mult
            {
                let left = fold_constant_for_version(&binary.left, self.python_version);
                let right = fold_constant_for_version(&binary.right, self.python_version);
                let sequence = match (&left, &right) {
                    (Some(Constant::SignedInt(_)), Some(sequence))
                    | (Some(sequence), Some(Constant::SignedInt(_))) => Some(sequence),
                    _ => None,
                };
                self.found = match sequence {
                    Some(Constant::String(value)) => value.is_empty(),
                    Some(Constant::Bytes(value) | Constant::SurrogateString(value)) => {
                        value.is_empty()
                    }
                    Some(Constant::Tuple(value)) => value.is_empty(),
                    _ => false,
                };
            }
            if !self.found {
                walk_expr(self, expression);
            }
        }
    }
    let mut finder = Finder {
        python_version,
        found: false,
    };
    finder.visit_expr(expression);
    finder.found
}

// CPython also limits recursively contained tuple items when folding repetition.
fn check_tuple_complexity(values: &[Constant], budget: usize) -> Option<usize> {
    let budget = budget.checked_sub(values.len())?;
    values.iter().try_fold(budget, |budget, value| match value {
        Constant::Tuple(values) => check_tuple_complexity(values, budget),
        _ => Some(budget),
    })
}

fn fold_integer_bitwise(left: &Constant, operator: Operator, right: &Constant) -> Option<Constant> {
    if let (Constant::Bool(left), Constant::Bool(right)) = (left, right) {
        return Some(Constant::Bool(match operator {
            Operator::BitAnd => left & right,
            Operator::BitOr => left | right,
            Operator::BitXor => left ^ right,
            _ => unreachable!(),
        }));
    }
    let mut left = folded_integer(left)?;
    let mut right = folded_integer(right)?;
    let negative = match operator {
        Operator::BitAnd => left.negative && right.negative,
        Operator::BitOr => left.negative || right.negative,
        Operator::BitXor => left.negative != right.negative,
        _ => unreachable!(),
    };

    // A negative integer is the infinite bitwise complement of (its magnitude minus one).
    // Keep that finite magnitude and use the sign mask to extend each operand's high bits.
    if left.negative {
        left.digits = subtract_magnitudes(&left.digits, &[1]);
    }
    if right.negative {
        right.digits = subtract_magnitudes(&right.digits, &[1]);
    }
    let left_mask = if left.negative { 0x7fff } else { 0 };
    let right_mask = if right.negative { 0x7fff } else { 0 };
    let result_mask = if negative { 0x7fff } else { 0 };
    let length = left.digits.len().max(right.digits.len());
    let mut digits = Vec::with_capacity(length);
    for index in 0..length {
        let left = left.digits.get(index).copied().unwrap_or(0) ^ left_mask;
        let right = right.digits.get(index).copied().unwrap_or(0) ^ right_mask;
        let digit = match operator {
            Operator::BitAnd => left & right,
            Operator::BitOr => left | right,
            Operator::BitXor => left ^ right,
            _ => unreachable!(),
        };
        digits.push(digit ^ result_mask);
    }
    if negative {
        digits = add_magnitudes(&digits, &[1]);
    }
    Some(integer_constant(negative, digits))
}

struct FoldedInteger {
    negative: bool,
    digits: Vec<u16>,
}

fn is_integer_constant(value: &Constant) -> bool {
    matches!(
        value,
        Constant::Bool(_) | Constant::Int(_) | Constant::SignedInt(_) | Constant::BigInt { .. }
    )
}

fn folded_integer(value: &Constant) -> Option<FoldedInteger> {
    let (negative, digits) = match value {
        Constant::Bool(value) => (false, integer_digits(u64::from(*value))),
        Constant::Int(value) => (false, integer_digits(*value)),
        Constant::SignedInt(value) => (value.is_negative(), integer_digits(value.unsigned_abs())),
        Constant::BigInt { negative, digits } => (*negative, normalized_digits(digits.clone())),
        _ => return None,
    };
    Some(FoldedInteger {
        negative: negative && !digits.is_empty(),
        digits,
    })
}

fn integer_digits(mut value: u64) -> Vec<u16> {
    let mut digits = Vec::new();
    while value != 0 {
        digits.push((value & 0x7fff) as u16);
        value >>= 15;
    }
    digits
}

#[expect(
    clippy::cast_precision_loss,
    reason = "CPython mixed float folding rounds integer operands to binary64"
)]
fn constant_as_float(constant: &Constant) -> Option<f64> {
    match constant {
        Constant::Bool(value) => Some(f64::from(*value)),
        Constant::Int(value) => Some(*value as f64),
        Constant::SignedInt(value) => Some(*value as f64),
        Constant::NonReflexive(number) => constant_as_float(&number.value()),
        Constant::Float(value) => Some(*value),
        // Dividing an integer by one provides PyLong_AsDouble's rounding and overflow checks.
        constant => fold_integer_true_divide(constant, &Constant::Int(1)),
    }
}

#[expect(
    clippy::float_cmp,
    reason = "CPython checks real exponents for exact binary64 integrality"
)]
fn fold_float_power(base: f64, exponent: f64) -> Option<Constant> {
    // float_pow returns the selected NaN operand directly, preserving its sign and payload.
    if exponent == 0.0 {
        return Some(Constant::Float(1.0));
    }
    if base.is_nan() {
        return Some(Constant::Float(base));
    }
    if exponent.is_nan() {
        return Some(Constant::Float(if base == 1.0 { 1.0 } else { exponent }));
    }
    if base.is_finite() && exponent.is_finite() && base < 0.0 && exponent != exponent.floor() {
        // float_pow delegates negative fractional powers to complex_pow. Follow
        // _Py_c_pow with positive imaginary zeros for both real operands.
        let magnitude = base.hypot(0.0).powf(exponent);
        let phase = 0.0_f64.atan2(base) * exponent;
        let real = magnitude * phase.cos();
        let imag = magnitude * phase.sin();
        return (real.is_finite() && imag.is_finite()).then_some(Constant::Complex { real, imag });
    }
    // CPython handles nonfinite inputs before calling libm: their infinite or
    // NaN results are successful powers, unlike overflow from finite operands.
    // Keep zero to a finite negative power as a runtime ZeroDivisionError.
    if base == 0.0 && exponent.is_finite() && exponent < 0.0 {
        return None;
    }
    let result = base.powf(exponent);
    (result.is_finite() || !base.is_finite() || !exponent.is_finite())
        .then_some(Constant::Float(result))
}

/// Match CPython's `float_rem`: a nonzero remainder and zero both take the divisor's sign.
fn fold_float_remainder(left: &Constant, right: &Constant) -> Option<f64> {
    let left = constant_as_float(left)?;
    let right = constant_as_float(right)?;
    if right == 0.0 {
        return None;
    }
    let mut remainder = left % right;
    if remainder != 0.0 {
        if (right < 0.0) != (remainder < 0.0) {
            remainder += right;
        }
    } else {
        remainder = 0.0_f64.copysign(right);
    }
    Some(remainder)
}

/// Match the quotient from CPython's `_float_div_mod`, including rounding and signed zero.
fn fold_float_floor_divide(left: &Constant, right: &Constant) -> Option<f64> {
    let left = constant_as_float(left)?;
    let right = constant_as_float(right)?;
    if right == 0.0 {
        return None;
    }

    let remainder = left % right;
    let mut quotient = (left - remainder) / right;
    if remainder != 0.0 && (right < 0.0) != (remainder < 0.0) {
        quotient -= 1.0;
    }
    if quotient != 0.0 {
        let mut result = quotient.floor();
        if quotient - result > 0.5 {
            result += 1.0;
        }
        Some(result)
    } else {
        Some(0.0_f64.copysign(left / right))
    }
}

/// Round an exact integer ratio to binary64, as CPython's `long_true_divide` does.
/// Quantizing subnormal results directly avoids rounding twice near underflow.
fn fold_integer_true_divide(left: &Constant, right: &Constant) -> Option<f64> {
    let left = folded_integer(left)?;
    let right = folded_integer(right)?;
    if right.digits.is_empty() {
        return None;
    }
    let sign = u64::from(left.negative != right.negative) << 63;
    let left_bits = magnitude_bit_length(&left.digits);
    let right_bits = magnitude_bit_length(&right.digits);
    if left_bits == 0 || right_bits.saturating_sub(left_bits) > 1075 {
        return Some(f64::from_bits(sign));
    }
    if left_bits.saturating_sub(right_bits) > 1024 {
        return None;
    }

    // The bit-length difference is at most one above floor(log2(left / right)).
    let difference = if left_bits >= right_bits {
        i32::try_from(left_bits - right_bits).expect("bounded exponent difference")
    } else {
        -i32::try_from(right_bits - left_bits).expect("bounded exponent difference")
    };
    let comparison = if difference >= 0 {
        compare_magnitudes(
            &left.digits,
            &left_shift_magnitude(&right.digits, difference.unsigned_abs() as usize),
        )
    } else {
        compare_magnitudes(
            &left_shift_magnitude(&left.digits, difference.unsigned_abs() as usize),
            &right.digits,
        )
    };
    let exponent = difference - i32::from(comparison.is_lt());
    let quantum = (exponent - 52).max(-1074);
    let (numerator, denominator) = if quantum < 0 {
        (
            left_shift_magnitude(&left.digits, quantum.unsigned_abs() as usize),
            right.digits,
        )
    } else {
        (
            left.digits,
            left_shift_magnitude(&right.digits, quantum.unsigned_abs() as usize),
        )
    };

    // Scaling leaves at most 53 quotient bits, even for arbitrarily large inputs.
    // Keep the full remainder so ties and values immediately beside ties differ.
    let mut remainder = numerator;
    let mut quotient = 0_u64;
    if let Some(top_bit) =
        magnitude_bit_length(&remainder).checked_sub(magnitude_bit_length(&denominator))
    {
        debug_assert!(top_bit < 64);
        for bit in (0..=top_bit).rev() {
            let divisor = left_shift_magnitude(&denominator, bit);
            if !compare_magnitudes(&remainder, &divisor).is_lt() {
                remainder = subtract_magnitudes(&remainder, &divisor);
                quotient |= 1 << bit;
            }
        }
    }
    let half = compare_magnitudes(&add_magnitudes(&remainder, &remainder), &denominator);
    if half.is_gt() || (half.is_eq() && quotient & 1 != 0) {
        quotient += 1;
    }
    if quotient == 0 {
        return Some(f64::from_bits(sign));
    }

    // Construct the already-rounded float without another floating-point rounding.
    let bits = u64::BITS - quotient.leading_zeros();
    let exponent = quantum + i32::try_from(bits).expect("u64 bit length fits in i32") - 1;
    if exponent > 1023 {
        return None;
    }
    let magnitude = if exponent < -1022 {
        quotient << (quantum + 1074)
    } else {
        let significand = if bits > 53 {
            quotient >> (bits - 53)
        } else {
            quotient << (53 - bits)
        };
        (u64::try_from(exponent + 1023).expect("normal exponent is positive") << 52)
            | (significand & ((1 << 52) - 1))
    };
    Some(f64::from_bits(sign | magnitude))
}

fn fold_integer_arithmetic(
    left: &Constant,
    operator: Operator,
    right: &Constant,
) -> Option<Constant> {
    const MAX_INT_SIZE: usize = 128;

    let left = folded_integer(left)?;
    let right = folded_integer(right)?;
    let result = match operator {
        Operator::Add => add_folded_integers(&left, &right),
        Operator::Sub => {
            let right = FoldedInteger {
                negative: !right.negative && !right.digits.is_empty(),
                digits: right.digits,
            };
            add_folded_integers(&left, &right)
        }
        Operator::Mult => {
            if !left.digits.is_empty()
                && !right.digits.is_empty()
                && magnitude_bit_length(&left.digits) + magnitude_bit_length(&right.digits)
                    > MAX_INT_SIZE
            {
                return None;
            }
            FoldedInteger {
                negative: left.negative != right.negative,
                digits: multiply_magnitudes(&left.digits, &right.digits),
            }
        }
        _ => unreachable!(),
    };
    Some(integer_constant(result.negative, result.digits))
}

/// Apply CPython's constant-power bit budget to the magnitude, independently of its sign.
fn fold_integer_power(left: &Constant, right: &Constant) -> Option<Constant> {
    const MAX_INT_SIZE: usize = 128;

    let exponent = folded_integer(right)?;
    if exponent.negative {
        return fold_float_power(constant_as_float(left)?, constant_as_float(right)?);
    }
    let left = folded_integer(left)?;
    if exponent.digits.is_empty() {
        return Some(Constant::Int(1));
    }
    if left.digits.is_empty() {
        return Some(Constant::Int(0));
    }
    let mut exponent = magnitude_to_usize(&exponent.digits)?;
    if magnitude_bit_length(&left.digits) > MAX_INT_SIZE / exponent {
        return None;
    }
    let negative = left.negative && exponent & 1 != 0;
    let mut power = left.digits;
    let mut result = vec![1];
    while exponent != 0 {
        if exponent & 1 != 0 {
            result = multiply_magnitudes(&result, &power);
        }
        exponent >>= 1;
        if exponent != 0 {
            power = multiply_magnitudes(&power, &power);
        }
    }
    Some(integer_constant(negative, result))
}

/// Divide integer magnitudes, then use CPython's floor quotient and divisor-signed remainder.
fn fold_integer_divmod(left: &Constant, operator: Operator, right: &Constant) -> Option<Constant> {
    let left = folded_integer(left)?;
    let right = folded_integer(right)?;
    if right.digits.is_empty() {
        return None;
    }
    let mut remainder = left.digits;
    let mut quotient = Vec::new();
    if let Some(top_bit) =
        magnitude_bit_length(&remainder).checked_sub(magnitude_bit_length(&right.digits))
    {
        quotient.resize(top_bit / 15 + 1, 0);
        for bit in (0..=top_bit).rev() {
            let divisor = left_shift_magnitude(&right.digits, bit);
            if !compare_magnitudes(&remainder, &divisor).is_lt() {
                remainder = subtract_magnitudes(&remainder, &divisor);
                quotient[bit / 15] |= 1 << (bit % 15);
            }
        }
        quotient = normalized_digits(quotient);
    }
    let negative = left.negative != right.negative;
    if negative && !remainder.is_empty() {
        quotient = add_magnitudes(&quotient, &[1]);
        remainder = subtract_magnitudes(&right.digits, &remainder);
    }
    Some(match operator {
        Operator::FloorDiv => integer_constant(negative, quotient),
        Operator::Mod => integer_constant(right.negative, remainder),
        _ => unreachable!(),
    })
}

fn add_folded_integers(left: &FoldedInteger, right: &FoldedInteger) -> FoldedInteger {
    if left.negative == right.negative {
        FoldedInteger {
            negative: left.negative,
            digits: add_magnitudes(&left.digits, &right.digits),
        }
    } else {
        match compare_magnitudes(&left.digits, &right.digits) {
            std::cmp::Ordering::Less => FoldedInteger {
                negative: right.negative,
                digits: subtract_magnitudes(&right.digits, &left.digits),
            },
            std::cmp::Ordering::Equal => FoldedInteger {
                negative: false,
                digits: Vec::new(),
            },
            std::cmp::Ordering::Greater => FoldedInteger {
                negative: left.negative,
                digits: subtract_magnitudes(&left.digits, &right.digits),
            },
        }
    }
}

fn compare_magnitudes(left: &[u16], right: &[u16]) -> std::cmp::Ordering {
    left.len()
        .cmp(&right.len())
        .then_with(|| left.iter().rev().cmp(right.iter().rev()))
}

fn add_magnitudes(left: &[u16], right: &[u16]) -> Vec<u16> {
    let mut digits = Vec::with_capacity(left.len().max(right.len()) + 1);
    let mut carry = 0_u32;
    for index in 0..left.len().max(right.len()) {
        let value = u32::from(left.get(index).copied().unwrap_or(0))
            + u32::from(right.get(index).copied().unwrap_or(0))
            + carry;
        digits.push((value & 0x7fff) as u16);
        carry = value >> 15;
    }
    if carry != 0 {
        digits.push(u16::try_from(carry).expect("base-2^15 carry fits in u16"));
    }
    digits
}

fn subtract_magnitudes(left: &[u16], right: &[u16]) -> Vec<u16> {
    debug_assert_ne!(compare_magnitudes(left, right), std::cmp::Ordering::Less);
    let mut digits = Vec::with_capacity(left.len());
    let mut borrow = 0_i32;
    for (index, left) in left.iter().copied().enumerate() {
        let mut value =
            i32::from(left) - i32::from(right.get(index).copied().unwrap_or(0)) - borrow;
        if value < 0 {
            value += 1 << 15;
            borrow = 1;
        } else {
            borrow = 0;
        }
        digits.push(u16::try_from(value).expect("base-2^15 subtraction digit fits in u16"));
    }
    debug_assert_eq!(borrow, 0);
    normalized_digits(digits)
}

fn multiply_magnitudes(left: &[u16], right: &[u16]) -> Vec<u16> {
    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }
    let mut digits = vec![0_u16; left.len() + right.len()];
    for (left_index, left) in left.iter().copied().enumerate() {
        let mut carry = 0_u32;
        for (right_index, right) in right.iter().copied().enumerate() {
            let index = left_index + right_index;
            let value = u32::from(digits[index]) + u32::from(left) * u32::from(right) + carry;
            digits[index] = (value & 0x7fff) as u16;
            carry = value >> 15;
        }
        let mut index = left_index + right.len();
        while carry != 0 {
            if index == digits.len() {
                digits.push(0);
            }
            let value = u32::from(digits[index]) + carry;
            digits[index] = (value & 0x7fff) as u16;
            carry = value >> 15;
            index += 1;
        }
    }
    normalized_digits(digits)
}

fn magnitude_bit_length(digits: &[u16]) -> usize {
    digits.last().map_or(0, |last| {
        (digits.len() - 1) * 15 + (u16::BITS - last.leading_zeros()) as usize
    })
}

fn fold_integer_shift(left: &Constant, operator: Operator, right: &Constant) -> Option<Constant> {
    const MAX_INT_SIZE: usize = 128;

    let left = folded_integer(left)?;
    let right = folded_integer(right)?;
    if right.negative {
        return None;
    }
    if left.digits.is_empty() {
        return Some(Constant::Int(0));
    }

    let right = magnitude_to_usize(&right.digits);
    match operator {
        Operator::LShift => {
            let right = right?;
            if right != 0
                && (right > MAX_INT_SIZE
                    || magnitude_bit_length(&left.digits) > MAX_INT_SIZE - right)
            {
                return None;
            }
            Some(integer_constant(
                left.negative,
                left_shift_magnitude(&left.digits, right),
            ))
        }
        Operator::RShift => {
            let Some(right) = right else {
                return Some(if left.negative {
                    Constant::SignedInt(-1)
                } else {
                    Constant::Int(0)
                });
            };
            let bit_length = magnitude_bit_length(&left.digits);
            if right >= bit_length {
                return Some(if left.negative {
                    Constant::SignedInt(-1)
                } else {
                    Constant::Int(0)
                });
            }
            let (mut digits, discarded) = right_shift_magnitude(&left.digits, right);
            if left.negative && discarded {
                digits = add_magnitudes(&digits, &[1]);
            }
            Some(integer_constant(left.negative, digits))
        }
        _ => unreachable!(),
    }
}

fn magnitude_to_usize(digits: &[u16]) -> Option<usize> {
    let mut value = 0_usize;
    for digit in digits.iter().rev() {
        value = value
            .checked_mul(1 << 15)?
            .checked_add(usize::from(*digit))?;
    }
    Some(value)
}

fn left_shift_magnitude(digits: &[u16], shift: usize) -> Vec<u16> {
    let digit_shift = shift / 15;
    let bit_shift = shift % 15;
    let mut output = vec![0_u16; digit_shift];
    let mut carry = 0_u32;
    for digit in digits {
        let value = (u32::from(*digit) << bit_shift) | carry;
        output.push((value & 0x7fff) as u16);
        carry = value >> 15;
    }
    if carry != 0 {
        output.push(u16::try_from(carry).expect("base-2^15 shift carry fits in u16"));
    }
    output
}

fn right_shift_magnitude(digits: &[u16], shift: usize) -> (Vec<u16>, bool) {
    let digit_shift = shift / 15;
    let bit_shift = shift % 15;
    let discarded = digits[..digit_shift].iter().any(|digit| *digit != 0)
        || bit_shift != 0 && digits[digit_shift] & ((1_u16 << bit_shift) - 1) != 0;
    let mut output = Vec::with_capacity(digits.len() - digit_shift);
    for index in digit_shift..digits.len() {
        let lower = u32::from(digits[index]) >> bit_shift;
        let upper = if bit_shift == 0 {
            0
        } else {
            u32::from(digits.get(index + 1).copied().unwrap_or(0)) << (15 - bit_shift)
        };
        output.push(((lower | upper) & 0x7fff) as u16);
    }
    (normalized_digits(output), discarded)
}

fn integer_constant(negative: bool, digits: Vec<u16>) -> Constant {
    let digits = normalized_digits(digits);
    if digits.is_empty() {
        return Constant::Int(0);
    }
    if digits.len() <= 5 {
        let value = digits
            .iter()
            .enumerate()
            .fold(0_u128, |value, (index, digit)| {
                value | (u128::from(*digit) << (index * 15))
            });
        if !negative {
            if let Ok(value) = u64::try_from(value) {
                return Constant::Int(value);
            }
        } else if value < u128::from(i64::MIN.unsigned_abs()) {
            let value = i64::try_from(value).expect("magnitude below 2^63 fits in i64");
            return Constant::SignedInt(-value);
        } else if value == u128::from(i64::MIN.unsigned_abs()) {
            return Constant::SignedInt(i64::MIN);
        }
    }
    Constant::BigInt { negative, digits }
}

fn fold_literal_subscript(
    value: &Expr,
    index: &Expr,
    python_version: PythonVersion,
    literal: &impl Fn(&Expr) -> Option<Constant>,
    boolean_sequence_repetition: bool,
) -> Option<Constant> {
    fn component(
        expression: &Expr,
        default: usize,
        boolean_sequence_repetition: bool,
    ) -> Option<usize> {
        match fold_constant_with_literals(
            expression,
            PythonVersion::Py314,
            &literal_constant,
            boolean_sequence_repetition,
        )? {
            // Computed None bounds miss CPython's early constant-slice construction.
            Constant::None if matches!(expression, Expr::NoneLiteral(_)) => Some(default),
            Constant::Bool(value) => Some(usize::from(value)),
            Constant::Int(value) => usize::try_from(value).ok(),
            Constant::SignedInt(value) if value >= 0 => usize::try_from(value).ok(),
            Constant::SignedInt(_) => None,
            Constant::BigInt {
                negative: false, ..
            } => Some(usize::MAX),
            Constant::BigInt { negative: true, .. } => None,
            _ => None,
        }
    }

    fn bound(
        expression: Option<&Expr>,
        length: usize,
        default: usize,
        boolean_sequence_repetition: bool,
    ) -> Option<usize> {
        let Some(expression) = expression else {
            return Some(default);
        };
        component(expression, default, boolean_sequence_repetition).map(|value| value.min(length))
    }

    fn indices(
        slice: &ruff_python_ast::ExprSlice,
        length: usize,
        boolean_sequence_repetition: bool,
    ) -> Option<Vec<usize>> {
        let lower = bound(
            slice.lower.as_deref(),
            length,
            0,
            boolean_sequence_repetition,
        )?;
        let upper = bound(
            slice.upper.as_deref(),
            length,
            length,
            boolean_sequence_repetition,
        )?;
        let step = slice.step.as_deref().map_or(Some(1), |expression| {
            component(expression, 1, boolean_sequence_repetition)
        })?;
        if step == 0 {
            return None;
        }
        Some((lower.min(upper)..upper).step_by(step).collect())
    }

    let value =
        fold_constant_with_literals(value, python_version, literal, boolean_sequence_repetition)?;
    if !matches!(index, Expr::Slice(_)) {
        let index = if python_version <= PythonVersion::Py313 {
            // The legacy AST optimizer sees source-aware folds such as `~True` in indices.
            fold_constant_with_literals(index, python_version, literal, boolean_sequence_repetition)
        } else {
            fold_constant_with_literals(
                index,
                PythonVersion::Py314,
                &literal_constant,
                boolean_sequence_repetition,
            )
        };
        let index = match index? {
            Constant::Bool(value) => i128::from(u8::from(value)),
            Constant::Int(value) => i128::from(value),
            Constant::SignedInt(value) => i128::from(value),
            _ => return None,
        };
        let length = match &value {
            Constant::String(value) => value.chars().count(),
            Constant::SurrogateString(value) => {
                value.iter().filter(|byte| **byte & 0xc0 != 0x80).count()
            }
            Constant::Bytes(value) => value.len(),
            Constant::Tuple(value) => value.len(),
            _ => return None,
        };
        let index = if index < 0 {
            i128::try_from(length).ok()?.checked_add(index)?
        } else {
            index
        };
        let index = usize::try_from(index)
            .ok()
            .filter(|index| *index < length)?;
        return match value {
            Constant::String(value) => value
                .chars()
                .nth(index)
                .map(|character| Constant::String(character.to_string())),
            Constant::SurrogateString(value) => Some(surrogate_string_constant(
                surrogate_characters(&value)[index].to_vec(),
            )),
            Constant::Bytes(value) => Some(Constant::Int(u64::from(value[index]))),
            Constant::Tuple(value) => Some(value[index].clone()),
            _ => None,
        };
    }

    let Expr::Slice(slice) = index else {
        unreachable!();
    };
    // CPython constructs constant slices before folding their bound expressions.
    constant_slice(slice)?;

    match value {
        Constant::String(value) => {
            let characters = value.chars().collect::<Vec<_>>();
            Some(Constant::String(
                indices(slice, characters.len(), boolean_sequence_repetition)?
                    .into_iter()
                    .map(|index| characters[index])
                    .collect(),
            ))
        }
        Constant::SurrogateString(value) => {
            let characters = surrogate_characters(&value);
            Some(surrogate_string_constant(
                indices(slice, characters.len(), boolean_sequence_repetition)?
                    .into_iter()
                    .flat_map(|index| characters[index].iter().copied())
                    .collect(),
            ))
        }
        Constant::Bytes(value) => Some(Constant::Bytes(
            indices(slice, value.len(), boolean_sequence_repetition)?
                .into_iter()
                .map(|index| value[index])
                .collect(),
        )),
        Constant::Tuple(value) => Some(Constant::Tuple(
            indices(slice, value.len(), boolean_sequence_repetition)?
                .into_iter()
                .map(|index| value[index].clone())
                .collect(),
        )),
        _ => None,
    }
}

/// Splits surrogate-pass UTF-8 at code-point boundaries, including lone surrogates.
fn surrogate_characters(value: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<_> = value
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte & 0xc0 != 0x80).then_some(index))
        .collect();
    starts.push(value.len());
    starts
        .windows(2)
        .map(|pair| &value[pair[0]..pair[1]])
        .collect()
}

/// Recovers ordinary string identity when a fold removes every surrogate code point.
fn surrogate_string_constant(value: Vec<u8>) -> Constant {
    match String::from_utf8(value) {
        Ok(value) => Constant::String(value),
        Err(error) => Constant::SurrogateString(error.into_bytes()),
    }
}

pub(super) fn folded_bool_operand(
    expression: &Expr,
    python_version: PythonVersion,
) -> Option<(&Expr, Constant)> {
    let Expr::BoolOp(boolean) = expression else {
        return fold_constant_for_version(expression, python_version)
            .map(|constant| (expression, constant));
    };
    let (last, leading) = boolean.values.split_last()?;
    for value in leading {
        if matches!(value, Expr::BoolOp(_)) {
            return None;
        }
        let constant = fold_constant_for_version(value, python_version)?;
        let truthiness = if python_version <= PythonVersion::Py313 {
            constant.is_truthy()
        } else {
            // The early LOAD_CONST pass precedes arithmetic and tuple folding. Its
            // Boolean shortcuts cannot consume values produced by those later rewrites.
            jump_constant_truthiness(value)?
        };
        if matches!(boolean.op, BoolOp::And) && !truthiness
            || matches!(boolean.op, BoolOp::Or) && truthiness
        {
            return Some((value, constant));
        }
    }
    if matches!(last, Expr::BoolOp(_)) {
        None
    } else {
        fold_constant_for_version(last, python_version).map(|constant| (last, constant))
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "CPython complex folding intentionally rounds integer operands to binary64"
)]
fn constant_complex_parts(constant: &Constant) -> Option<(f64, f64)> {
    match constant {
        Constant::Bool(value) => Some((f64::from(*value), 0.0)),
        Constant::Int(value) => Some((*value as f64, 0.0)),
        Constant::SignedInt(value) => Some((*value as f64, 0.0)),
        Constant::BigInt { .. } => {
            fold_integer_true_divide(constant, &Constant::Int(1)).map(|real| (real, 0.0))
        }
        Constant::NonReflexive(number) => constant_complex_parts(&number.value()),
        Constant::Float(value) => Some((*value, 0.0)),
        Constant::Complex { real, imag } => Some((*real, *imag)),
        _ => None,
    }
}

fn fold_complex_arithmetic(
    left: &Constant,
    operator: Operator,
    right: &Constant,
    python_version: PythonVersion,
) -> Option<Constant> {
    let (left_real, left_imag) = constant_complex_parts(left)?;
    let (right_real, right_imag) = constant_complex_parts(right)?;
    // Before 3.14, CPython converts real operands to complex with imaginary +0.0.
    // The newer mixed formulas preserve different signed zeros and NaN components.
    let (real, imag) = match (left, right) {
        _ if python_version <= PythonVersion::Py313
            || matches!(
                (left, right),
                (Constant::Complex { .. }, Constant::Complex { .. })
            ) =>
        {
            match operator {
                Operator::Add => (left_real + right_real, left_imag + right_imag),
                Operator::Sub => (left_real - right_real, left_imag - right_imag),
                // Apple's legacy CPython builds contract these C expressions into FMADD.
                // Negate the rounded second product, preserving generated NaN signs as well
                // as finite cancellation; 3.14+ uses separate intermediate statements.
                Operator::Mult
                    if cfg!(all(target_os = "macos", target_arch = "aarch64"))
                        && python_version <= PythonVersion::Py313 =>
                {
                    (
                        left_real.mul_add(right_real, -(left_imag * right_imag)),
                        left_real.mul_add(right_imag, left_imag * right_real),
                    )
                }
                Operator::Mult if python_version >= PythonVersion::Py314 => {
                    versioned_complex_product(
                        (left_real, left_imag),
                        (right_real, right_imag),
                        python_version,
                    )
                }
                Operator::Mult => complex_product((left_real, left_imag), (right_real, right_imag)),
                _ => unreachable!(),
            }
        }
        (Constant::Complex { .. }, _) => match operator {
            Operator::Add => (left_real + right_real, left_imag),
            Operator::Sub => (left_real - right_real, left_imag),
            Operator::Mult => (left_real * right_real, left_imag * right_real),
            _ => unreachable!(),
        },
        (_, Constant::Complex { .. }) => match operator {
            Operator::Add => (right_real + left_real, right_imag),
            Operator::Sub => (left_real - right_real, -right_imag),
            Operator::Mult => (right_real * left_real, right_imag * left_real),
            _ => unreachable!(),
        },
        _ => return None,
    };
    Some(Constant::Complex { real, imag })
}

fn complex_product(left: (f64, f64), right: (f64, f64)) -> (f64, f64) {
    (
        left.0 * right.0 - left.1 * right.1,
        left.0 * right.1 + left.1 * right.0,
    )
}

/// Recovers infinities in modern CPython's direct and integer-power complex products.
fn versioned_complex_product(
    (mut a, mut b): (f64, f64),
    (mut c, mut d): (f64, f64),
    python_version: PythonVersion,
) -> (f64, f64) {
    let (ac, bd, ad, bc) = (a * c, b * d, a * d, b * c);
    let result = (ac - bd, ad + bc);
    if python_version <= PythonVersion::Py313 || !result.0.is_nan() || !result.1.is_nan() {
        return result;
    }

    let zero_nan = |value: f64| {
        if value.is_nan() {
            0.0_f64.copysign(value)
        } else {
            value
        }
    };
    let boxed = |value: f64| f64::from(value.is_infinite()).copysign(value);
    let mut recalculate = false;
    if a.is_infinite() || b.is_infinite() {
        (a, b, c, d) = (boxed(a), boxed(b), zero_nan(c), zero_nan(d));
        recalculate = true;
    }
    if c.is_infinite() || d.is_infinite() {
        (a, b, c, d) = (zero_nan(a), zero_nan(b), boxed(c), boxed(d));
        recalculate = true;
    }
    if !recalculate && [ac, bd, ad, bc].into_iter().any(f64::is_infinite) {
        (a, b, c, d) = (zero_nan(a), zero_nan(b), zero_nan(c), zero_nan(d));
        recalculate = true;
    }
    if recalculate {
        (
            f64::INFINITY * (a * c - b * d),
            f64::INFINITY * (a * d + b * c),
        )
    } else {
        result
    }
}

/// Recover signed zeros as modern CPython's `_Py_c_quot` does for a reciprocal.
fn complex_power_reciprocal(
    divisor: (f64, f64),
    python_version: PythonVersion,
) -> Option<(f64, f64)> {
    let result = complex_quotient((1.0, 0.0), divisor, false)?;
    if python_version >= PythonVersion::Py314
        && result.0.is_nan()
        && result.1.is_nan()
        && (divisor.0.is_infinite() || divisor.1.is_infinite())
    {
        let x = f64::from(divisor.0.is_infinite()).copysign(divisor.0);
        let y = f64::from(divisor.1.is_infinite()).copysign(divisor.1);
        Some((0.0 * (x + 0.0 * y), 0.0 * (0.0 * x - y)))
    } else {
        Some(result)
    }
}

fn complex_quotient(left: (f64, f64), right: (f64, f64), contract: bool) -> Option<(f64, f64)> {
    let abs_real = right.0.abs();
    let abs_imag = right.1.abs();
    let result = if abs_real >= abs_imag {
        if abs_real == 0.0 {
            return None;
        }
        let ratio = right.1 / right.0;
        if contract {
            let denominator = right.1.mul_add(ratio, right.0);
            return Some((
                left.1.mul_add(ratio, left.0) / denominator,
                (-left.0).mul_add(ratio, left.1) / denominator,
            ));
        }
        let denominator = right.0 + right.1 * ratio;
        (
            (left.0 + left.1 * ratio) / denominator,
            (left.1 - left.0 * ratio) / denominator,
        )
    } else if abs_imag >= abs_real {
        let ratio = right.0 / right.1;
        if contract {
            let denominator = right.0.mul_add(ratio, right.1);
            return Some((
                left.0.mul_add(ratio, left.1) / denominator,
                left.1.mul_add(ratio, -left.0) / denominator,
            ));
        }
        let denominator = right.0 * ratio + right.1;
        (
            (left.0 * ratio + left.1) / denominator,
            (left.1 * ratio - left.0) / denominator,
        )
    } else {
        (f64::NAN, f64::NAN)
    };
    Some(result)
}

fn fold_complex_division(
    left: &Constant,
    right: &Constant,
    python_version: PythonVersion,
) -> Option<Constant> {
    let left_parts = constant_complex_parts(left)?;
    let right_parts = constant_complex_parts(right)?;
    // Apple's CPython builds contract the general quotient expressions, including
    // the negation of a NaN addend in the imaginary numerator.
    let contract = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let (mut real, mut imag) = match (left, right) {
        _ if python_version <= PythonVersion::Py313 => {
            // Legacy CPython converts real operands to complex with imaginary +0.0.
            // Its quotient formula can produce different signed zeros from the mixed
            // real/complex division introduced in 3.14.
            complex_quotient(left_parts, right_parts, contract)?
        }
        (Constant::Complex { .. }, Constant::Complex { .. }) => {
            complex_quotient(left_parts, right_parts, contract)?
        }
        (Constant::Complex { .. }, _) => {
            if right_parts.0 == 0.0 {
                return None;
            }
            (left_parts.0 / right_parts.0, left_parts.1 / right_parts.0)
        }
        (_, Constant::Complex { .. }) => {
            let abs_real = right_parts.0.abs();
            let abs_imag = right_parts.1.abs();
            if abs_real >= abs_imag {
                if abs_real == 0.0 {
                    return None;
                }
                let ratio = right_parts.1 / right_parts.0;
                let denominator = right_parts.0 + right_parts.1 * ratio;
                (
                    left_parts.0 / denominator,
                    (-left_parts.0 * ratio) / denominator,
                )
            } else if abs_imag >= abs_real {
                let ratio = right_parts.0 / right_parts.1;
                let denominator = right_parts.0 * ratio + right_parts.1;
                (
                    (left_parts.0 * ratio) / denominator,
                    (-left_parts.0) / denominator,
                )
            } else {
                (f64::NAN, f64::NAN)
            }
        }
        _ => return None,
    };
    if python_version >= PythonVersion::Py314 && real.is_nan() && imag.is_nan() {
        // Modern CPython recovers infinities and signed zeros when the quotient
        // formula produces two NaNs. Legacy targets retain the original NaNs.
        if matches!(left, Constant::Complex { .. })
            && matches!(right, Constant::Complex { .. })
            && (left_parts.0.is_infinite() || left_parts.1.is_infinite())
            && right_parts.0.is_finite()
            && right_parts.1.is_finite()
        {
            let x = f64::from(left_parts.0.is_infinite()).copysign(left_parts.0);
            let y = f64::from(left_parts.1.is_infinite()).copysign(left_parts.1);
            real = f64::INFINITY * (x * right_parts.0 + y * right_parts.1);
            imag = f64::INFINITY * (y * right_parts.0 - x * right_parts.1);
        } else if matches!(right, Constant::Complex { .. })
            && (right_parts.0.is_infinite() || right_parts.1.is_infinite())
            && left_parts.0.is_finite()
            && left_parts.1.is_finite()
        {
            let x = f64::from(right_parts.0.is_infinite()).copysign(right_parts.0);
            let y = f64::from(right_parts.1.is_infinite()).copysign(right_parts.1);
            if matches!(left, Constant::Complex { .. }) {
                real = 0.0 * (left_parts.0 * x + left_parts.1 * y);
                imag = 0.0 * (left_parts.1 * x - left_parts.0 * y);
            } else {
                real = 0.0 * (left_parts.0 * x);
                imag = 0.0 * (-left_parts.0 * y);
            }
        }
    }
    Some(Constant::Complex { real, imag })
}

#[expect(
    clippy::float_cmp,
    reason = "CPython checks complex exponents for exact binary64 integrality"
)]
fn fold_complex_power(
    base: &Constant,
    exponent: &Constant,
    python_version: PythonVersion,
) -> Option<Constant> {
    let base = constant_complex_parts(base)?;
    let (exponent_real, exponent_imag) = constant_complex_parts(exponent)?;
    if exponent_imag != 0.0 || exponent_real != exponent_real.floor() || exponent_real.abs() > 100.0
    {
        // Match _Py_c_pow, preserving its operation order and signed zeros.
        if base.0 == 0.0 && base.1 == 0.0 {
            if exponent_imag != 0.0 || exponent_real < 0.0 {
                return None;
            }
            return Some(Constant::Complex {
                real: 0.0,
                imag: 0.0,
            });
        }
        let absolute = base.0.hypot(base.1);
        let mut magnitude = absolute.powf(exponent_real);
        let angle = base.1.atan2(base.0);
        let mut phase = angle * exponent_real;
        if exponent_imag != 0.0 {
            magnitude *= (-angle * exponent_imag).exp();
            phase += exponent_imag * absolute.ln();
        }
        // macOS libm returns NaNs for sin/cos of infinity without setting errno;
        // other supported platforms set EDOM, so CPython leaves the power unfolded.
        // _Py_ADJUST_ERANGE2 only retains overflow for infinite outputs.
        if phase.is_infinite() && !cfg!(target_os = "macos") {
            return None;
        }
        let component = |value: f64| {
            // The pinned Linux x86-64 build keeps the libm result in the destination
            // of the final multiplication, which preserves its NaN when both are NaN.
            if cfg!(all(target_os = "linux", target_arch = "x86_64"))
                && magnitude.is_nan()
                && value.is_nan()
            {
                value
            } else {
                magnitude * value
            }
        };
        let real = component(phase.cos());
        let imag = component(phase.sin());
        return (!real.is_infinite() && !imag.is_infinite())
            .then_some(Constant::Complex { real, imag });
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the exact integral exponent is bounded to CPython's [-100, 100] fast path"
    )]
    let exponent = exponent_real as i64;
    let mut result = (1.0, 0.0);
    let mut power = base;
    let magnitude = exponent.unsigned_abs();
    let mut mask = 1_u64;
    while mask > 0 && magnitude >= mask {
        if magnitude & mask != 0 {
            result = versioned_complex_product(result, power, python_version);
        }
        mask <<= 1;
        power = versioned_complex_product(power, power, python_version);
    }
    if exponent < 0 {
        result = complex_power_reciprocal(result, python_version)?;
    }
    (!result.0.is_infinite() && !result.1.is_infinite()).then_some(Constant::Complex {
        real: result.0,
        imag: result.1,
    })
}

pub(super) fn constant_slice(slice: &ruff_python_ast::ExprSlice) -> Option<Constant> {
    constant_slice_with_literals(slice, &literal_constant)
}

pub(super) fn constant_slice_with_literals(
    slice: &ruff_python_ast::ExprSlice,
    literal: &impl Fn(&Expr) -> Option<Constant>,
) -> Option<Constant> {
    let bound = |expression: Option<&Expr>| {
        expression.map_or(Some(Constant::None), |expression| {
            // CPython admits only literal bounds to this early slice optimization.
            literal_constant(expression)?;
            literal(expression)
        })
    };

    Some(Constant::Slice {
        lower: Box::new(bound(slice.lower.as_deref())?),
        upper: Box::new(bound(slice.upper.as_deref())?),
        step: Box::new(bound(slice.step.as_deref())?),
    })
}

pub(super) fn two_element_slice_optimization(
    slice: &ruff_python_ast::ExprSlice,
    python_version: PythonVersion,
) -> bool {
    slice.step.is_none()
        && (python_version <= PythonVersion::Py313 || constant_slice(slice).is_none())
}

pub(super) fn first_literal_constant(expression: &Expr) -> Option<Constant> {
    if let Some(constant) = literal_constant(expression) {
        return Some(constant);
    }
    match expression {
        Expr::BinOp(binary) => {
            first_literal_constant(&binary.left).or_else(|| first_literal_constant(&binary.right))
        }
        Expr::UnaryOp(unary) => first_literal_constant(&unary.operand),
        Expr::Tuple(tuple) => tuple.elts.iter().find_map(first_literal_constant),
        _ => None,
    }
}

pub(super) enum FirstConstant {
    Literal(Constant),
    NestedScope,
}

pub(super) fn first_suite_literal_constant(
    compiler: &Compiler<'_>,
    body: &[Stmt],
) -> Option<Constant> {
    match first_constant(compiler, body, None, false, false) {
        Some(FirstConstant::Literal(constant)) => Some(constant),
        Some(FirstConstant::NestedScope) | None => None,
    }
}

pub(super) fn first_suite_constant(
    compiler: &Compiler<'_>,
    body: &[Stmt],
) -> Option<FirstConstant> {
    // A live generator expression admits its code object before evaluating its iterable.
    // Do not reserve a later literal ahead of that allocation.
    first_constant(compiler, body, None, true, true)
}

pub(super) fn first_expression_constant(
    compiler: &Compiler<'_>,
    expression: &Expr,
) -> Option<FirstConstant> {
    first_constant(compiler, &[], Some(expression), true, false)
}

fn first_constant(
    compiler: &Compiler<'_>,
    body: &[Stmt],
    expression: Option<&Expr>,
    stop_at_generators: bool,
    include_implicit_constants: bool,
) -> Option<FirstConstant> {
    #[derive(Clone, Copy)]
    enum Frame<'ast> {
        ImplicitNone,
        Finally(&'ast [Stmt]),
        Loop,
    }

    struct Collector<'compiler, 'source, 'ast> {
        compiler: &'compiler Compiler<'source>,
        constant: Option<Constant>,
        reached_nested_scope: bool,
        stop_at_generators: bool,
        include_implicit_constants: bool,
        frames: Vec<Frame<'ast>>,
    }

    impl<'ast> Collector<'_, '_, 'ast> {
        fn is_complete(&self) -> bool {
            self.constant.is_some() || self.reached_nested_scope
        }

        fn visit_unwind(&mut self, stop_at_loop: bool) {
            if self.is_complete()
                || self.frames.is_empty()
                || (stop_at_loop && matches!(self.frames.last(), Some(Frame::Loop)))
            {
                return;
            }
            let frame = self.frames.pop().unwrap();
            match frame {
                Frame::ImplicitNone => self.constant = Some(Constant::None),
                Frame::Finally(body) => self.visit_body(body),
                Frame::Loop => {}
            }
            // A jump inside this finalizer sees only the surrounding frames, including
            // loops that have not already been unwound by an enclosing return.
            self.visit_unwind(stop_at_loop);
            self.frames.push(frame);
        }

        fn visit_annotation_subscript(&mut self, expression: &'ast Expr) {
            match expression {
                Expr::Slice(slice) => {
                    // Annotation-only targets evaluate explicit bounds without building a slice.
                    for bound in [&slice.lower, &slice.upper, &slice.step]
                        .into_iter()
                        .flatten()
                    {
                        self.visit_expr(bound);
                    }
                }
                Expr::Tuple(tuple) => {
                    for element in &tuple.elts {
                        self.visit_annotation_subscript(element);
                    }
                }
                _ => self.visit_expr(expression),
            }
        }

        /// Older code generation builds an entire constant display before visiting its items.
        fn legacy_collection_constant(
            &self,
            elements: &[Expr],
            is_set: bool,
            minimum_size: usize,
        ) -> Option<Constant> {
            if elements.len() < minimum_size {
                return None;
            }
            let constants = elements
                .iter()
                .map(|element| self.compiler.source_aware_fold_constant(element))
                .collect::<Option<Vec<_>>>()?;
            if is_set {
                let mut unique = Vec::with_capacity(constants.len());
                for constant in constants {
                    if !unique
                        .iter()
                        .any(|existing| python_constant_values_equal(existing, &constant))
                    {
                        unique.push(constant);
                    }
                }
                Some(Constant::FrozenSet(unique))
            } else {
                Some(Constant::Tuple(constants))
            }
        }

        fn visit_iterable_expression(&mut self, expression: &'ast Expr) {
            if self.is_complete() {
                return;
            }
            if self.compiler.context.python_version <= PythonVersion::Py313 {
                let constant = match expression {
                    Expr::List(list) if list.ctx == ExprContext::Load => {
                        self.legacy_collection_constant(&list.elts, false, 0)
                    }
                    Expr::Set(set) => self.legacy_collection_constant(&set.elts, true, 0),
                    _ => None,
                };
                if constant.is_some() {
                    self.constant = constant;
                    return;
                }
            }
            self.visit_expr(expression);
        }

        fn visit_legacy_dict(&mut self, items: &'ast [ruff_python_ast::DictItem]) {
            let mut index = 0;
            while index < items.len() && !self.is_complete() {
                if items[index].key.is_none() {
                    self.visit_expr(&items[index].value);
                    index += 1;
                    continue;
                }
                let start = index;
                // A mapping flushes after its seventeenth pair or before unpacking.
                while index < items.len() && items[index].key.is_some() && index - start < 17 {
                    index += 1;
                }
                let group = &items[start..index];
                if (2..=15).contains(&group.len())
                    && let Some(keys) = group
                        .iter()
                        .map(|item| {
                            self.compiler
                                .source_aware_fold_constant(item.key.as_ref().unwrap())
                        })
                        .collect::<Option<Vec<_>>>()
                {
                    for item in group {
                        self.visit_expr(&item.value);
                    }
                    if !self.is_complete() {
                        self.constant = Some(Constant::Tuple(keys));
                    }
                } else {
                    for item in group {
                        self.visit_expr(item.key.as_ref().unwrap());
                        self.visit_expr(&item.value);
                    }
                }
            }
        }

        fn visit_legacy_keywords(&mut self, keywords: &'ast [ruff_python_ast::Keyword]) {
            let mut index = 0;
            while index < keywords.len() && !self.is_complete() {
                if keywords[index].arg.is_none() {
                    self.visit_expr(&keywords[index].value);
                    index += 1;
                    continue;
                }
                let start = index;
                while index < keywords.len() && keywords[index].arg.is_some() {
                    index += 1;
                }
                let group = &keywords[start..index];
                if (2..=15).contains(&group.len()) {
                    for keyword in group {
                        self.visit_expr(&keyword.value);
                    }
                    if !self.is_complete() {
                        self.constant = Some(Constant::Tuple(
                            group
                                .iter()
                                .map(|keyword| {
                                    Constant::String(keyword.arg.as_ref().unwrap().to_string())
                                })
                                .collect(),
                        ));
                    }
                } else {
                    self.constant =
                        Some(Constant::String(group[0].arg.as_ref().unwrap().to_string()));
                }
            }
        }

        fn visit_interpolated_components(
            &mut self,
            components: Vec<super::expression::InterpolatedStringComponent<'ast>>,
        ) {
            use super::expression::{InterpolatedStringComponent, format_spec_components};

            if components.len() > 30 || components.is_empty() {
                self.constant = Some(Constant::String(String::new()));
                return;
            }
            for component in components {
                if self.is_complete() {
                    break;
                }
                match component {
                    InterpolatedStringComponent::Literal(literal) => {
                        self.constant = Some(literal.into_constant());
                    }
                    InterpolatedStringComponent::Interpolation(interpolation, is_raw) => {
                        self.visit_expr(&interpolation.expression);
                        if !self.is_complete()
                            && let Some(specification) = &interpolation.format_spec
                        {
                            self.visit_interpolated_components(format_spec_components(
                                &specification.elements,
                                is_raw,
                                self.compiler.context.source,
                            ));
                        }
                    }
                }
            }
        }
    }

    impl<'ast> Visitor<'ast> for Collector<'_, '_, 'ast> {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if self.is_complete() {
                return;
            }
            if (self.compiler.context.python_version <= PythonVersion::Py313
                || self.include_implicit_constants)
                && matches!(statement, Stmt::Expr(expression)
                    if self.compiler.return_value_is_constant(&expression.value))
            {
                // A discarded constant expression does not reserve the first
                // code-generation constant when searching a terminal loop body.
                return;
            }
            if matches!(statement, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                // This statement allocates a code object in the enclosing pool. Leave
                // that allocation to code generation instead of reserving a later literal.
                self.reached_nested_scope = true;
                return;
            }
            if self.include_implicit_constants {
                match statement {
                    Stmt::With(manager) => {
                        for item in &manager.items {
                            self.visit_expr(&item.context_expr);
                            if manager.is_async && !self.is_complete() {
                                self.constant = Some(Constant::None);
                            }
                            if let Some(target) = &item.optional_vars {
                                self.visit_expr(target);
                            }
                        }
                        self.frames.push(Frame::ImplicitNone);
                        self.visit_body(&manager.body);
                        self.frames.pop();
                        if !self.is_complete() {
                            self.constant = Some(Constant::None);
                        }
                        return;
                    }
                    Stmt::Break(_) | Stmt::Continue(_) => {
                        // Finalizers run before any enclosing manager's exit constants.
                        // A loop jump unwinds only frames entered inside its own loop.
                        self.visit_unwind(true);
                        return;
                    }
                    Stmt::For(loop_) => {
                        if loop_.is_async {
                            self.visit_expr(&loop_.iter);
                        } else {
                            self.visit_iterable_expression(&loop_.iter);
                        }
                        if loop_.is_async && !self.is_complete() {
                            self.constant = Some(Constant::None);
                        }
                        self.frames.push(Frame::Loop);
                        self.visit_expr(&loop_.target);
                        self.visit_body(&loop_.body);
                        self.frames.pop();
                        self.visit_body(&loop_.orelse);
                        return;
                    }
                    Stmt::While(loop_) => {
                        self.visit_expr(&loop_.test);
                        self.frames.push(Frame::Loop);
                        self.visit_body(&loop_.body);
                        self.frames.pop();
                        self.visit_body(&loop_.orelse);
                        return;
                    }
                    Stmt::Return(return_) => {
                        // CPython defers AST constants until after unwind, including values
                        // folded before code generation by the older targets.
                        if let Some(value) = &return_.value
                            && !self.compiler.return_value_is_constant(value)
                        {
                            self.visit_expr(value);
                        }
                        self.visit_unwind(false);
                        if !self.is_complete() {
                            if let Some(value) = &return_.value {
                                self.visit_expr(value);
                            } else {
                                self.constant = Some(Constant::None);
                            }
                        }
                        return;
                    }
                    Stmt::Try(try_) => {
                        if !try_.finalbody.is_empty() {
                            self.frames.push(Frame::Finally(&try_.finalbody));
                        }
                        self.visit_body(&try_.body);
                        // Ordinary except emits else before its handlers; except* emits
                        // else after them. This is constant admission order, not AST order.
                        if !try_.is_star {
                            self.visit_body(&try_.orelse);
                        }
                        for handler in &try_.handlers {
                            let handler = handler.as_except_handler().unwrap();
                            if let Some(type_) = &handler.type_ {
                                self.visit_expr(type_);
                            }
                            if handler.name.is_some() {
                                self.frames.push(Frame::ImplicitNone);
                            }
                            self.visit_body(&handler.body);
                            if handler.name.is_some() {
                                self.frames.pop();
                                if !self.is_complete() {
                                    self.constant = Some(Constant::None);
                                }
                            }
                        }
                        if try_.is_star {
                            self.visit_body(&try_.orelse);
                        }
                        if !try_.finalbody.is_empty() {
                            self.frames.pop();
                            self.visit_body(&try_.finalbody);
                        }
                        return;
                    }
                    _ => {}
                }
            }
            match statement {
                Stmt::For(loop_)
                    if self.compiler.context.python_version <= PythonVersion::Py313
                        && !loop_.is_async =>
                {
                    self.visit_iterable_expression(&loop_.iter);
                    self.visit_expr(&loop_.target);
                    self.visit_body(&loop_.body);
                    self.visit_body(&loop_.orelse);
                    return;
                }
                Stmt::AnnAssign(assignment) => {
                    if let Some(value) = &assignment.value {
                        self.visit_expr(value);
                        self.visit_expr(&assignment.target);
                    } else {
                        match assignment.target.as_ref() {
                            Expr::Subscript(subscript) => {
                                self.visit_expr(&subscript.value);
                                self.visit_annotation_subscript(&subscript.slice);
                            }
                            Expr::Attribute(attribute) => self.visit_expr(&attribute.value),
                            _ => {}
                        }
                    }
                    if self.compiler.context.python_version <= PythonVersion::Py313
                        && matches!(
                            self.compiler.unit.scope,
                            super::Scope::Module | super::Scope::Class { .. }
                        )
                        && !self.is_complete()
                    {
                        // Legacy module/class annotations execute after their target.
                        // Their first constant survives dead-code compaction at slot zero.
                        if self.compiler.unit.flags
                            & crate::target::code_flags::CO_FUTURE_ANNOTATIONS
                            != 0
                        {
                            if assignment.simple {
                                self.constant =
                                    Some(Constant::String(super::source::unparse_annotation(
                                        &assignment.annotation,
                                        self.compiler.context.source,
                                    )));
                            }
                        } else {
                            self.visit_expr(&assignment.annotation);
                        }
                        if assignment.simple
                            && !self.is_complete()
                            && let Expr::Name(name) = assignment.target.as_ref()
                        {
                            self.constant = Some(Constant::String(
                                self.compiler.mangled_name(name.id.as_str()),
                            ));
                        }
                    }
                    return;
                }
                Stmt::Import(_) => {
                    self.constant = Some(Constant::Int(0));
                    return;
                }
                Stmt::ImportFrom(import) => {
                    self.constant = Some(Constant::Int(u64::from(import.level)));
                    return;
                }
                _ => {}
            }
            walk_stmt(self, statement);
        }

        fn visit_pattern(&mut self, pattern: &'ast Pattern) {
            if self.is_complete() {
                return;
            }
            if self.include_implicit_constants
                && self.compiler.context.python_version >= PythonVersion::Py314
            {
                match pattern {
                    Pattern::MatchSequence(sequence) => {
                        let starred = sequence
                            .patterns
                            .iter()
                            .any(|pattern| matches!(pattern, Pattern::MatchStar(_)));
                        if !starred || sequence.patterns.len() > 1 {
                            let length = sequence.patterns.len() - usize::from(starred);
                            self.constant =
                                Some(Constant::Int(u64::try_from(length).unwrap_or(u64::MAX)));
                            return;
                        }
                    }
                    Pattern::MatchMapping(mapping) if !mapping.keys.is_empty() => {
                        self.constant = Some(Constant::Int(
                            u64::try_from(mapping.keys.len()).unwrap_or(u64::MAX),
                        ));
                        return;
                    }
                    Pattern::MatchMapping(mapping) if mapping.rest.is_some() => {
                        self.constant = Some(Constant::Tuple(Vec::new()));
                        return;
                    }
                    Pattern::MatchClass(class) => {
                        self.visit_expr(&class.cls);
                        if !self.is_complete() {
                            self.constant = Some(Constant::Tuple(
                                class
                                    .arguments
                                    .keywords
                                    .iter()
                                    .map(|keyword| Constant::String(keyword.attr.to_string()))
                                    .collect(),
                            ));
                        }
                        return;
                    }
                    Pattern::MatchSingleton(singleton) => {
                        self.constant = Some(match singleton.value {
                            Singleton::None => Constant::None,
                            Singleton::True => Constant::Bool(true),
                            Singleton::False => Constant::Bool(false),
                        });
                        return;
                    }
                    _ => {}
                }
            }
            walk_pattern(self, pattern);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.is_complete() {
                return;
            }
            if let Expr::FString(fstring) = expression {
                self.visit_interpolated_components(super::expression::fstring_components(
                    fstring,
                    self.compiler.context.source,
                ));
                return;
            }
            if matches!(expression, Expr::Lambda(_))
                || self.stop_at_generators && matches!(expression, Expr::Generator(_))
            {
                self.reached_nested_scope = true;
                return;
            }
            if matches!(expression, Expr::Name(name) if name.id.as_str() == "__debug__") {
                self.constant = Some(Constant::Bool(true));
                return;
            }
            if self.compiler.context.python_version <= PythonVersion::Py313 {
                match expression {
                    Expr::List(list) if list.ctx == ExprContext::Load => {
                        if let Some(constant) =
                            self.legacy_collection_constant(&list.elts, false, 3)
                        {
                            self.constant = Some(constant);
                            return;
                        }
                    }
                    Expr::Set(set) => {
                        if let Some(constant) = self.legacy_collection_constant(&set.elts, true, 3)
                        {
                            self.constant = Some(constant);
                            return;
                        }
                    }
                    Expr::Compare(comparison) => {
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
                    Expr::Dict(dict) => {
                        self.visit_legacy_dict(&dict.items);
                        return;
                    }
                    _ => {}
                }
            }
            if self.compiler.context.python_version <= PythonVersion::Py313 {
                let comprehension = match expression {
                    Expr::ListComp(comprehension) => Some((
                        &comprehension.generators[..],
                        None,
                        comprehension.elt.as_ref(),
                    )),
                    Expr::SetComp(comprehension) => Some((
                        &comprehension.generators[..],
                        None,
                        comprehension.elt.as_ref(),
                    )),
                    Expr::DictComp(comprehension) => Some((
                        &comprehension.generators[..],
                        comprehension.key.as_deref(),
                        comprehension.value.as_ref(),
                    )),
                    _ => None,
                };
                if let Some((generators, key, value)) = comprehension {
                    for generator in generators {
                        self.visit_iterable_expression(&generator.iter);
                        if generator.is_async && !self.is_complete() {
                            self.constant = Some(Constant::None);
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
                    return;
                }
            }
            if let Expr::Call(call) = expression {
                self.visit_expr(&call.func);
                let shape = self.compiler.call_shape(call);
                let expanded = shape.has_starred
                    || shape.has_keyword_unpack
                    || (shape.direct_method.is_none()
                        && shape.positional_count + shape.keyword_count * 2 > 30);
                if self.compiler.context.python_version <= PythonVersion::Py313
                    && expanded
                    && !self.is_complete()
                    && let Some(constant) =
                        self.legacy_collection_constant(&call.arguments.args, false, 3)
                {
                    self.constant = Some(constant);
                } else {
                    for argument in &call.arguments.args {
                        self.visit_expr(argument);
                    }
                }
                if expanded && self.compiler.context.python_version <= PythonVersion::Py313 {
                    self.visit_legacy_keywords(&call.arguments.keywords);
                } else if expanded {
                    for keyword in &call.arguments.keywords {
                        if !self.is_complete()
                            && let Some(name) = &keyword.arg
                        {
                            self.constant = Some(Constant::String(name.to_string()));
                        }
                        self.visit_expr(&keyword.value);
                    }
                } else {
                    for keyword in &call.arguments.keywords {
                        self.visit_expr(&keyword.value);
                    }
                    if !self.is_complete() && !call.arguments.keywords.is_empty() {
                        self.constant = Some(Constant::Tuple(
                            call.arguments
                                .keywords
                                .iter()
                                .map(|keyword| {
                                    Constant::String(keyword.arg.as_ref().unwrap().to_string())
                                })
                                .collect(),
                        ));
                    }
                }
                return;
            }
            if let Expr::Slice(slice) = expression {
                if self.compiler.context.python_version >= PythonVersion::Py314
                    && let Some(constant) = self.compiler.source_aware_constant_slice(slice)
                {
                    self.constant = Some(constant);
                } else {
                    // Slice lowering admits an implicit None for each omitted bound.
                    for bound in [&slice.lower, &slice.upper] {
                        if let Some(bound) = bound {
                            self.visit_expr(bound);
                        } else if !self.is_complete() {
                            self.constant = Some(Constant::None);
                        }
                    }
                    if let Some(step) = &slice.step {
                        self.visit_expr(step);
                    }
                }
                return;
            }
            if let Expr::Await(await_expression) = expression {
                self.visit_expr(&await_expression.value);
                if !self.is_complete() {
                    // The first SEND loads None after evaluating the awaited expression.
                    self.constant = Some(Constant::None);
                }
                return;
            }
            if let Expr::YieldFrom(yield_from) = expression {
                self.visit_expr(&yield_from.value);
                if !self.is_complete() {
                    self.constant = Some(Constant::None);
                }
                return;
            }
            if matches!(expression, Expr::Yield(yield_) if yield_.value.is_none()) {
                self.constant = Some(Constant::None);
                return;
            }
            let constant = if self.compiler.context.python_version <= PythonVersion::Py313 {
                self.compiler.source_aware_fold_constant(expression)
            } else {
                literal_constant(expression)
            };
            if let Some(constant) = constant {
                self.constant = Some(constant);
            } else {
                walk_expr(self, expression);
            }
        }
    }

    let mut collector = Collector {
        compiler,
        constant: None,
        reached_nested_scope: false,
        stop_at_generators,
        include_implicit_constants,
        frames: Vec::new(),
    };
    if let Some(expression) = expression {
        collector.visit_expr(expression);
    } else {
        for statement in body {
            collector.visit_stmt(statement);
            if collector.is_complete() {
                break;
            }
        }
    }
    collector.constant.map(FirstConstant::Literal).or_else(|| {
        collector
            .reached_nested_scope
            .then_some(FirstConstant::NestedScope)
    })
}
pub(super) fn is_literal_constant(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::NoneLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
    )
}
pub(super) fn big_integer_digits(token: &str) -> Vec<u16> {
    let token: String = token
        .chars()
        .filter(|character| *character != '_')
        .collect();
    let (digits, radix) = if let Some(digits) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        (digits, 16)
    } else if let Some(digits) = token
        .strip_prefix("0o")
        .or_else(|| token.strip_prefix("0O"))
    {
        (digits, 8)
    } else if let Some(digits) = token
        .strip_prefix("0b")
        .or_else(|| token.strip_prefix("0B"))
    {
        (digits, 2)
    } else {
        (token.as_str(), 10)
    };

    let mut value = vec![0_u16];
    for digit in digits.chars().filter_map(|digit| digit.to_digit(radix)) {
        let mut carry = digit;
        for limb in &mut value {
            let next = u32::from(*limb) * radix + carry;
            *limb = (next & 0x7fff) as u16;
            carry = next >> 15;
        }
        while carry != 0 {
            value.push((carry & 0x7fff) as u16);
            carry >>= 15;
        }
    }
    while value.len() > 1 && value.last() == Some(&0) {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::Operator;

    use super::{
        CONSTANT_INDEX_THRESHOLD, ConstantEquality, ConstantIndices, ConstantPool,
        FORCE_GENERIC_CONSTANT_ADMISSION, constant_pool_fingerprint, fold_complex_arithmetic,
    };
    use crate::code_object::{
        CodeObject, Constant, LocalsIdentity, MarshalMetadataIds, MarshalProvenance, MetadataId,
    };
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    #[test]
    fn complex_multiplication_recovery_follows_the_target() {
        let real_infinity = Constant::Complex {
            real: f64::INFINITY,
            imag: f64::NAN,
        };
        let finite = Constant::Complex {
            real: 1.0,
            imag: 1.0,
        };
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for (left, right) in [(&real_infinity, &finite), (&finite, &real_infinity)] {
                let Some(Constant::Complex { real, imag }) =
                    fold_complex_arithmetic(left, Operator::Mult, right, python_version)
                else {
                    panic!("complex multiplication should be folded");
                };
                if python_version >= PythonVersion::Py314 {
                    assert_eq!(real.to_bits(), f64::INFINITY.to_bits());
                    assert_eq!(imag.to_bits(), f64::INFINITY.to_bits());
                } else {
                    assert!(real.is_nan());
                    assert!(imag.is_nan());
                }
            }

            if python_version >= PythonVersion::Py314 {
                for (left, right) in [
                    (&real_infinity, &Constant::Float(1.0)),
                    (&Constant::Float(1.0), &real_infinity),
                ] {
                    let Some(Constant::Complex { real, imag }) =
                        fold_complex_arithmetic(left, Operator::Mult, right, python_version)
                    else {
                        panic!("mixed complex multiplication should be folded");
                    };
                    assert_eq!(real.to_bits(), f64::INFINITY.to_bits());
                    assert!(imag.is_nan());
                }
            }
        }
    }

    fn code_object(constants: Vec<Constant>) -> CodeObject {
        CodeObject {
            arg_count: 0,
            positional_only_arg_count: 0,
            keyword_only_arg_count: 0,
            stack_size: 1,
            flags: 0,
            bytecode: Vec::new(),
            constants,
            names: Vec::new(),
            locals: vec!["value".into()],
            local_kinds: Vec::new(),
            filename: "module.py".into(),
            name: "<module>".to_string(),
            qualified_name: "<module>".to_string(),
            first_line_number: 1,
            line_table: Vec::new(),
            exception_table: Vec::new(),
            marshal_provenance: MarshalProvenance {
                metadata_ids: MarshalMetadataIds {
                    domain_len: 2,
                    locals: vec![MetadataId::from_u32(0)],
                    filename: MetadataId::from_u32(1),
                    ..MarshalMetadataIds::default()
                },
                subtree_has_frozenset_constants: true,
                ..MarshalProvenance::default()
            },
        }
    }

    #[test]
    fn equality_domains_distinguish_pool_keys_from_python_values() {
        let boolean = Constant::Tuple(vec![Constant::Bool(true), Constant::Float(-0.0)]);
        let integer = Constant::Tuple(vec![Constant::Int(1), Constant::Float(0.0)]);

        assert!(!ConstantEquality::PoolKey.equal(&boolean, &integer));
        assert!(ConstantEquality::PythonValue.equal(&boolean, &integer));
    }

    #[test]
    fn constant_pool_owns_deduplication_and_retention() {
        let mut pool = ConstantPool::default();
        assert_eq!(pool.intern(Constant::None).unwrap(), 0);
        assert_eq!(pool.intern(Constant::None).unwrap(), 0);
        assert_eq!(pool.intern(Constant::Int(1)).unwrap(), 1);
        assert_eq!(
            pool.intern(Constant::String("kept".to_string())).unwrap(),
            2
        );

        let index_map = pool.retain_used(&[false, false, true]);

        assert_eq!(index_map, [Some(0), None, Some(1)]);
        assert!(matches!(
            pool.into_values().as_slice(),
            [Constant::None, Constant::String(value)] if value == "kept"
        ));
    }

    #[test]
    fn direct_scalar_admission_matches_generic_provenance_and_serialization() {
        struct ResetGenericAdmission(bool);

        impl Drop for ResetGenericAdmission {
            fn drop(&mut self) {
                FORCE_GENERIC_CONSTANT_ADMISSION.set(self.0);
            }
        }

        let sources = [
            r"
values = (None, False, True, ..., 300, -300, 123456789012345678901234567890,
          1.25, -0.0, 2.5j, 'normal', 'one.two', 'méthode', '\ud800', b'', b'value')
def outer(argument):
    def inner():
        return argument, 12.5, '\ud800', b''
    return inner, (3.5, 0j, 'one.two')
",
            r#"
class Outer:
    """Outer.Inner"""
    dotted = 'Outer.Inner'
    def method(self):
        self.méthode = '%'
        def inner(value):
            return value in {'set', 'cache'}, ('méthode', '%')
        return inner
before = b'' * 0
after = b''
values = (1e1000 % 1, (1e1000 % 1, 2j), b'kept')
"#,
            r"
def outer(argument):
    if False:
        class Discarded:
            value = ('one.two', '%', 'café')
            def inner(self, value):
                return value in {'set', 'member'}
    return argument, ('one.two', '%', 'café'), b''
",
        ];
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            for source in sources {
                let fast = compile_with_options(source, "constant_admission.py", &options).unwrap();
                let generic = {
                    let _reset =
                        ResetGenericAdmission(FORCE_GENERIC_CONSTANT_ADMISSION.replace(true));
                    compile_with_options(source, "constant_admission.py", &options).unwrap()
                };
                assert_eq!(
                    format!("{:?}", fast.code),
                    format!("{:?}", generic.code),
                    "{python_version:?}: {source}"
                );
                assert_eq!(fast.marshal(), generic.marshal(), "{python_version:?}");
                let source_size = u32::try_from(source.len()).unwrap();
                assert_eq!(
                    fast.to_timestamp_pyc(17, source_size),
                    generic.to_timestamp_pyc(17, source_size),
                    "{python_version:?}"
                );
            }
        }
    }

    #[test]
    fn reserved_none_does_not_build_the_constant_index() {
        let mut pool = ConstantPool::default();
        pool.intern(Constant::None).unwrap();
        for value in 1..CONSTANT_INDEX_THRESHOLD {
            pool.intern(Constant::Int(u64::try_from(value).unwrap()))
                .unwrap();
        }

        assert!(pool.indices.is_none());
        assert_eq!(pool.intern(Constant::None).unwrap(), 0);
        assert!(pool.indices.is_none());
        assert_eq!(pool.intern(Constant::Int(1)).unwrap(), 1);
        assert!(pool.indices.is_some());
        assert_eq!(pool.intern(Constant::None).unwrap(), 0);

        let mut unreserved = ConstantPool::default();
        unreserved
            .intern(Constant::String("docstring".to_string()))
            .unwrap();
        assert_eq!(unreserved.intern(Constant::None).unwrap(), 1);
        assert_eq!(unreserved.intern(Constant::None).unwrap(), 1);
    }

    #[test]
    fn small_constant_pool_preserves_numeric_types_and_string_kinds() {
        let mut pool = ConstantPool::default();
        let values = [
            Constant::Bool(false),
            Constant::Int(0),
            Constant::SignedInt(0),
            Constant::Float(0.0),
            Constant::Float(-0.0),
            Constant::String(String::new()),
            Constant::SurrogateString(Vec::new()),
            Constant::Bytes(Vec::new()),
        ];
        for _ in 0..2 {
            for (index, value) in values.iter().enumerate() {
                assert_eq!(
                    pool.intern(value.clone()).unwrap(),
                    u32::try_from(index).unwrap()
                );
            }
        }
        assert!(pool.indices.is_none());
    }

    #[test]
    fn indexed_constant_pool_preserves_identity_collisions_and_retention() {
        let mut pool = ConstantPool::default();
        for value in 100..108 {
            assert_eq!(
                pool.intern(Constant::Int(value)).unwrap(),
                u32::try_from(value - 100).unwrap()
            );
        }

        assert_eq!(pool.intern(Constant::Bool(true)).unwrap(), 8);
        assert_eq!(pool.intern(Constant::Int(1)).unwrap(), 9);
        assert_eq!(pool.intern(Constant::SignedInt(1)).unwrap(), 10);
        assert_eq!(pool.intern(Constant::Float(0.0)).unwrap(), 11);
        assert_eq!(pool.intern(Constant::Float(-0.0)).unwrap(), 12);
        assert_eq!(
            pool.intern(Constant::Float(f64::from_bits(0x7ff8_0000_0000_0001)))
                .unwrap(),
            13
        );
        assert_eq!(
            pool.intern(Constant::Float(f64::from_bits(0x7ff8_0000_0000_0002)))
                .unwrap(),
            14
        );

        let nested = Constant::Tuple(vec![Constant::FrozenSet(vec![
            Constant::String("first".to_string()),
            Constant::String("second".to_string()),
        ])]);
        let reordered = Constant::Tuple(vec![Constant::FrozenSet(vec![
            Constant::String("second".to_string()),
            Constant::String("first".to_string()),
        ])]);
        assert_eq!(pool.intern(nested).unwrap(), 15);
        assert_eq!(pool.intern(reordered).unwrap(), 15);

        let plain = code_object(vec![Constant::String("méthode".to_string())]);
        let mut with_intern_provenance = plain.clone();
        with_intern_provenance
            .marshal_provenance
            .explicitly_interned_strings
            .insert("méthode".to_string());
        let mut different = plain.clone();
        different.bytecode.push(1);
        let mut fresh = plain.clone();
        fresh.marshal_provenance.locals_identity = LocalsIdentity::Fresh;

        assert_eq!(pool.intern(Constant::Code(Box::new(plain))).unwrap(), 16);
        assert_eq!(
            pool.intern(Constant::Code(Box::new(with_intern_provenance)))
                .unwrap(),
            16
        );
        assert_eq!(
            pool.intern(Constant::Code(Box::new(different.clone())))
                .unwrap(),
            17
        );
        assert_eq!(
            pool.intern(Constant::Code(Box::new(different))).unwrap(),
            17
        );
        assert_eq!(pool.intern(Constant::Code(Box::new(fresh))).unwrap(), 18);

        let mut used = vec![true; 19];
        used[1] = false;
        let index_map = pool.retain_used(&used);
        assert_eq!(index_map[0], Some(0));
        assert_eq!(index_map[1], None);
        assert_eq!(index_map[2], Some(1));
        assert_eq!(pool.intern(Constant::Int(100)).unwrap(), 0);
        assert_eq!(pool.intern(Constant::Int(102)).unwrap(), 1);
        assert_eq!(pool.intern(Constant::Int(101)).unwrap(), 18);
    }

    #[test]
    fn frozenset_fingerprints_are_order_independent_and_value_sensitive() {
        let first = Constant::FrozenSet(vec![Constant::Int(1), Constant::Int(2)]);
        let reordered = Constant::FrozenSet(vec![Constant::Int(2), Constant::Int(1)]);
        let different = Constant::FrozenSet(vec![Constant::Int(3), Constant::Int(4)]);

        assert_eq!(
            constant_pool_fingerprint(&first),
            constant_pool_fingerprint(&reordered)
        );
        assert_ne!(
            constant_pool_fingerprint(&first),
            constant_pool_fingerprint(&different)
        );
    }

    #[test]
    fn constant_indices_resolve_fingerprint_collisions() {
        let values = [Constant::Int(1), Constant::Int(2), Constant::Int(2)];
        let mut indices = ConstantIndices::default();
        indices.insert(Some(0), 0);
        indices.insert(Some(0), 1);
        indices.insert(Some(0), 2);

        assert_eq!(indices.find(&values, &Constant::Int(1), Some(0)), Some(0));
        assert_eq!(indices.find(&values, &Constant::Int(2), Some(0)), Some(1));
        assert_eq!(indices.find(&values, &Constant::Int(3), Some(0)), None);
        assert_eq!(indices.find(&values, &Constant::Int(1), Some(1)), None);
    }

    #[test]
    fn constant_indices_preserve_earliest_fallback_or_fingerprint_match() {
        let values = [
            Constant::Int(2),
            Constant::Int(1),
            Constant::Int(2),
            Constant::Int(1),
        ];
        let mut indices = ConstantIndices::default();
        indices.insert(None, 0);
        indices.insert(Some(0), 1);
        indices.insert(Some(0), 2);
        indices.insert(None, 3);

        assert_eq!(indices.find(&values, &Constant::Int(1), Some(0)), Some(1));
        assert_eq!(indices.find(&values, &Constant::Int(2), Some(0)), Some(0));
        assert_eq!(indices.find(&values, &Constant::Int(2), Some(1)), Some(0));
        assert_eq!(indices.find(&values, &Constant::Int(1), None), Some(1));
    }

    #[test]
    fn indexed_code_constants_compare_bodies_with_matching_source_identities() {
        let mut pool = ConstantPool::default();
        for value in 0..CONSTANT_INDEX_THRESHOLD {
            pool.intern(Constant::Int(u64::try_from(value).unwrap()))
                .unwrap();
        }

        let mut original = code_object(vec![Constant::Int(1)]);
        original.bytecode = vec![1, 2];
        let mut other_constants = original.clone();
        other_constants.constants[0] = Constant::Int(2);
        let mut other_bytecode = original.clone();
        other_bytecode.bytecode[1] = 3;
        let mut other_line_table = original.clone();
        other_line_table.line_table.push(4);

        let original = Constant::Code(Box::new(original));
        let original_hash = constant_pool_fingerprint(&original);
        let original_index = pool.intern(original.clone()).unwrap();
        for (offset, other) in [other_constants, other_bytecode, other_line_table]
            .into_iter()
            .enumerate()
        {
            let other = Constant::Code(Box::new(other));
            assert_eq!(constant_pool_fingerprint(&other), original_hash);
            let index = original_index + u32::try_from(offset).unwrap() + 1;
            assert_eq!(pool.intern(other.clone()).unwrap(), index);
            assert_eq!(pool.intern(other).unwrap(), index);
            assert_eq!(pool.intern(original.clone()).unwrap(), original_index);
        }
    }

    #[test]
    fn indexed_constant_pool_preserves_duplicate_frozenset_fallback() {
        let mut pool = ConstantPool::default();
        for value in 0..CONSTANT_INDEX_THRESHOLD {
            pool.intern(Constant::Int(u64::try_from(value).unwrap()))
                .unwrap();
        }

        let duplicate = Constant::FrozenSet(vec![Constant::Int(1), Constant::Int(1)]);
        let compatible = Constant::FrozenSet(vec![Constant::Int(1), Constant::Int(2)]);
        assert_eq!(constant_pool_fingerprint(&duplicate), None);
        let duplicate_index = pool.intern(duplicate.clone()).unwrap();
        assert_eq!(pool.intern(compatible.clone()).unwrap(), duplicate_index);

        let duplicate_code = Constant::Code(Box::new(code_object(vec![duplicate])));
        let compatible_code = Constant::Code(Box::new(code_object(vec![compatible])));
        assert!(constant_pool_fingerprint(&duplicate_code).is_some());
        assert_eq!(
            constant_pool_fingerprint(&duplicate_code),
            constant_pool_fingerprint(&compatible_code)
        );
        let duplicate_code_index = pool.intern(duplicate_code).unwrap();
        assert_eq!(pool.intern(compatible_code).unwrap(), duplicate_code_index);
    }

    #[test]
    fn code_constant_equality_includes_locals_identity() {
        let by_value = code_object(Vec::new());
        let mut fresh = by_value.clone();
        fresh.marshal_provenance.locals_identity = LocalsIdentity::Fresh;

        assert!(!ConstantEquality::PoolKey.equal(
            &Constant::Code(Box::new(by_value)),
            &Constant::Code(Box::new(fresh))
        ));
    }

    #[test]
    fn code_constant_equality_ignores_graph_only_marshal_provenance() {
        let plain = code_object(vec![Constant::String("méthode".to_string())]);
        let mut with_graph_provenance = plain.clone();
        with_graph_provenance
            .marshal_provenance
            .explicitly_interned_strings
            .insert("méthode".to_string());
        with_graph_provenance
            .marshal_provenance
            .qualified_name_constant_index = Some(0);
        with_graph_provenance
            .marshal_provenance
            .subtree_has_frozenset_constants = false;

        assert!(ConstantEquality::PoolKey.equal(
            &Constant::Code(Box::new(plain)),
            &Constant::Code(Box::new(with_graph_provenance))
        ));
    }
}
