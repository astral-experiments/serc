use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{
    BoolOp, Expr, ExprBinOp, ExprContext, FStringPart, InterpolatedStringElement, Number, Operator,
    Stmt, UnaryOp,
};
use ruff_text_size::Ranged;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use smallvec::SmallVec;

use super::{LOAD_CONST, LOAD_SMALL_INT, NOP, STORE_NAME};
use crate::CompileError;
use crate::assembler::{Label, OperandPatchId};
use crate::target::operands::Conversion;

use super::Compiler;
use super::expression::literal_truthiness;
use super::scope::NameId;
use super::symbol::{mangle_name, to_u32};
use crate::code_object::{CodeObject, Constant};

#[derive(Debug, Default)]
pub(super) struct ConstantPool {
    values: Vec<Constant>,
    indices: Option<ConstantIndices>,
}

const CONSTANT_INDEX_THRESHOLD: usize = 12;

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
        values
            .iter()
            .position(|existing| ConstantEquality::PoolKey.equal(existing, constant))
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

        if let Some(index) = self
            .values
            .iter()
            .position(|existing| ConstantEquality::PoolKey.equal(existing, &constant))
        {
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
                code.arg_count.hash(&mut hasher);
                code.positional_only_arg_count.hash(&mut hasher);
                code.keyword_only_arg_count.hash(&mut hasher);
                code.stack_size.hash(&mut hasher);
                code.flags.hash(&mut hasher);
                code.bytecode.hash(&mut hasher);
                code.constants.len().hash(&mut hasher);
                for constant in &code.constants {
                    hash_constant(constant)?.hash(&mut hasher);
                }
                code.names.hash(&mut hasher);
                code.locals.hash(&mut hasher);
                code.local_kinds.hash(&mut hasher);
                code.filename.hash(&mut hasher);
                code.first_line_number.hash(&mut hasher);
                code.name.hash(&mut hasher);
                code.qualified_name.hash(&mut hasher);
                code.line_table.hash(&mut hasher);
                code.exception_table.hash(&mut hasher);
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
    pub(super) folded_constant_dependencies: Vec<(Constant, Vec<Constant>)>,
    pub(super) has_untracked_discarded_collection: bool,
    pub(super) deferred_names: Vec<(OperandPatchId, String)>,
    pub(super) names: Vec<String>,
    pub(super) name_ids: Vec<u32>,
    name_indices: FxHashMap<NameId, u32>,
    pub(super) interned_constant_strings: FxHashSet<String>,
    pub(super) qualified_name_constant_index: Option<u32>,
    pub(super) has_frozenset_constants: bool,
    pub(super) first_flowgraph_constant: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum DeferredConstantTarget {
    Instruction(OperandPatchId),
    DiscardedExpression(Label),
}

impl crate::assembler::ConstantTable for OutputState {
    fn get(&self, index: u32) -> &Constant {
        &self.constants.values[index as usize]
    }

    fn intern(&mut self, constant: Constant) -> Result<u32, CompileError> {
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

    fn retain_used(&mut self, used: &[bool], order: &[u32]) -> Vec<Option<u32>> {
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
                pool.values
                    .iter()
                    .position(|value| ConstantEquality::PoolKey.equal(value, constant))
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

impl Compiler<'_> {
    pub(super) fn emit_folded_constant(
        &mut self,
        expression: &Expr,
        constant: Constant,
    ) -> Result<(), CompileError> {
        if matches!(expression, Expr::Name(name) if name.id.as_str() == "__debug__") {
            return self.emit_preprocessed_constant(expression, constant);
        }
        if let Some(literal) = literal_constant(expression) {
            self.add_constant(literal)?;
        } else {
            self.record_folded_operands(expression)?;
        }
        if matches!(expression, Expr::Tuple(_))
            || matches!(expression, Expr::UnaryOp(unary) if unary.op == UnaryOp::Not)
        {
            self.emit_folded_tuple_not_nops(expression)?;
            self.assembler
                .set_location(self.source_location(expression.range()));
        }
        let hidden_owner_slots = folded_constant_owner_slots(expression);
        if let Constant::Int(value) = constant
            && let Ok(value) = u8::try_from(value)
        {
            self.emit(LOAD_SMALL_INT, u32::from(value), 1)?;
        } else {
            self.emit_deferred_constant(constant)?;
        }
        self.assembler
            .add_last_constant_fold_slots(hidden_owner_slots);
        // Literal tuple elements, including prebuilt constant slices, each remove
        // one load from this owner's block when the tuple is folded.
        if let Expr::Tuple(tuple) = expression
            && tuple.elts.iter().all(|element| {
                matches!(
                    element,
                    Expr::NoneLiteral(_)
                        | Expr::BooleanLiteral(_)
                        | Expr::NumberLiteral(_)
                        | Expr::StringLiteral(_)
                        | Expr::BytesLiteral(_)
                        | Expr::EllipsisLiteral(_)
                ) || matches!(element, Expr::Slice(slice) if constant_slice(slice).is_some())
            })
        {
            self.assembler
                .record_last_uniform_fold_owner_slots(tuple.elts.len());
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
        Ok(())
    }

    fn record_folded_operands(&mut self, expression: &Expr) -> Result<(), CompileError> {
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
                if let Expr::Slice(slice) = subscript.slice.as_ref() {
                    if let Some(constant) = constant_slice(slice) {
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
                    fold_constant(&subscript.slice),
                    Some(Constant::Int(value)) if u8::try_from(value).is_ok()
                ) {
                    self.record_folded_value(&subscript.slice)?;
                }
            }
            Expr::UnaryOp(unary) => self.record_folded_value(&unary.operand)?,
            _ => {}
        }
        let operands: Vec<_> = match expression {
            Expr::BinOp(binary) => [&*binary.left, &*binary.right]
                .into_iter()
                .filter_map(fold_constant)
                .collect(),
            Expr::UnaryOp(unary) => fold_constant(&unary.operand).into_iter().collect(),
            Expr::Subscript(subscript) => [&*subscript.value, &*subscript.slice]
                .into_iter()
                .filter_map(fold_constant)
                .collect(),
            _ => Vec::new(),
        };
        if !operands.is_empty()
            && let Some(result) = fold_constant(expression)
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
        match expression {
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.emit_folded_tuple_not_nops(element)?;
                }
            }
            Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => {
                self.assembler
                    .set_location(self.source_location(unary.operand.range()));
                self.emit(NOP, 0, 0)?;
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
        if matches!(expression, Expr::Slice(_))
            && let Some(constant) = fold_constant(expression)
        {
            self.add_constant(constant)?;
            return Ok(());
        }
        if matches!(expression, Expr::Name(name) if name.id.as_str() == "__debug__") {
            self.add_constant(Constant::Bool(true))?;
            return Ok(());
        }
        if let Expr::FString(fstring) = expression
            && fold_constant(expression).is_some()
        {
            let constant = match super::expression::fstring_components(fstring, self.context.source)
                .into_iter()
                .next()
            {
                Some(super::expression::InterpolatedStringComponent::Literal(literal)) => {
                    literal.into_constant()
                }
                None => Constant::String(String::new()),
                Some(super::expression::InterpolatedStringComponent::Interpolation(_, _)) => {
                    unreachable!("a folded f-string has no interpolated expressions")
                }
            };
            self.add_constant(constant)?;
            return Ok(());
        }

        self.record_folded_operands(expression)?;
        if let Some(constant) = fold_constant(expression)
            && !matches!(constant, Constant::Int(value) if u8::try_from(value).is_ok())
        {
            self.output.deferred_constants.push((None, constant));
        }
        Ok(())
    }

    pub(super) fn emit_preprocessed_constant(
        &mut self,
        expression: &Expr,
        constant: Constant,
    ) -> Result<(), CompileError> {
        if let Constant::Int(value) = constant
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
                self.output.names.push(name.into_owned());
                self.output.name_ids.push(name_id.0);
                entry.insert(index);
                Ok(index)
            }
        }
    }

    pub(super) fn emit_unsigned_constant(&mut self, value: u32) -> Result<(), CompileError> {
        if u8::try_from(value).is_ok() {
            self.emit(LOAD_SMALL_INT, value, 1)
        } else {
            // CPython's small-integer load table ends at 255. Larger synthesized integers
            // must be loaded from the constant table.
            let constant = self.add_constant(Constant::Int(u64::from(value)))?;
            self.emit(LOAD_CONST, constant, 1)
        }
    }

    pub(super) fn add_constant(&mut self, constant: Constant) -> Result<u32, CompileError> {
        self.context
            .record_constant_string_origins(&constant, false);
        if let Constant::Code(code) = &constant {
            self.output.has_frozenset_constants |= code
                .marshal_provenance
                .subtree_has_frozenset_constants
                .unwrap_or(true);
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
            if let Constant::Code(code) = constant {
                self.output.has_frozenset_constants |= code
                    .marshal_provenance
                    .subtree_has_frozenset_constants
                    .unwrap_or(true);
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
            if !self
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

/// Returns instruction slots removed when CPython's flow graph folds `expression`.
///
/// The count is attached to the emitted folded result so labels and other control-flow
/// boundaries naturally constrain which later boolean marker can reuse the stale slots.
pub(super) fn folded_constant_owner_slots(expression: &Expr) -> usize {
    fn direct(expression: &Expr) -> usize {
        match expression {
            Expr::BinOp(binary) => 2 + direct(&binary.left) + direct(&binary.right),
            Expr::Tuple(tuple) => tuple
                .elts
                .iter()
                .fold(tuple.elts.len(), |slots, element| slots + direct(element)),
            Expr::UnaryOp(unary) => {
                if unary.op == UnaryOp::Not
                    && matches!(unary.operand.as_ref(), Expr::Tuple(_))
                    && fold_constant(&unary.operand).is_none()
                {
                    1
                } else {
                    1 + direct(&unary.operand)
                }
            }
            Expr::Subscript(subscript) => 2 + direct(&subscript.value) + direct(&subscript.slice),
            _ => 0,
        }
    }

    direct(expression)
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
    /// Equality for CPython's constant table, including type and floating-point bit identity.
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
        Constant::Float(value) => Some((NumericReal::Float(*value), 0.0)),
        Constant::Complex { real, imag } => Some((NumericReal::Float(*real), *imag)),
        _ => None,
    }
}

fn integer_numeric_real(mut value: u64, negative: bool) -> NumericReal {
    let mut digits = Vec::new();
    while value != 0 {
        digits.push((value & 0x7fff) as u16);
        value >>= 15;
    }
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

    let mut digits = match integer_numeric_real(mantissa, negative) {
        NumericReal::Integer { digits, .. } => digits,
        NumericReal::Float(_) => unreachable!(),
    };
    let shift = u32::try_from(exponent).ok()?;
    let digit_shift = usize::try_from(shift / 15).ok()?;
    let bit_shift = shift % 15;
    if digit_shift > 0 {
        digits.splice(0..0, std::iter::repeat_n(0, digit_shift));
    }
    if bit_shift > 0 {
        let mut carry = 0_u32;
        for digit in &mut digits {
            let shifted = (u32::from(*digit) << bit_shift) | carry;
            *digit = (shifted & 0x7fff) as u16;
            carry = shifted >> 15;
        }
        if carry != 0 {
            digits.push(u16::try_from(carry).expect("base-2^15 carry fits in u16"));
        }
    }
    Some(NumericReal::Integer { negative, digits })
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
pub(super) fn literal_constant(expression: &Expr) -> Option<Constant> {
    match expression {
        Expr::NoneLiteral(_) => Some(Constant::None),
        Expr::BooleanLiteral(boolean) => Some(Constant::Bool(boolean.value)),
        Expr::EllipsisLiteral(_) => Some(Constant::Ellipsis),
        Expr::NumberLiteral(number) => Some(match &number.value {
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
        }),
        Expr::StringLiteral(string) => Some(Constant::String(string.value.to_str().to_string())),
        Expr::BytesLiteral(bytes) => Some(Constant::Bytes(bytes.value.bytes().collect())),
        _ => None,
    }
}
pub(super) enum PercentFormatPart<'a> {
    Literal(String),
    Formatted {
        expression: &'a Expr,
        conversion: Conversion,
        format_spec: Option<String>,
    },
}

pub(super) fn optimized_percent_format(
    expression: &ExprBinOp,
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

    let format = format.value.chars().collect::<Vec<_>>();
    let mut position = 0;
    let mut argument_index = 0;
    let mut parts = Vec::with_capacity(arguments.elts.len() * 2 + 1);
    while position < format.len() {
        let mut literal = String::new();
        while position < format.len() {
            if format[position] != '%' {
                literal.push(format[position]);
                position += 1;
            } else if format.get(position + 1) == Some(&'%') {
                literal.push('%');
                position += 2;
            } else {
                break;
            }
        }
        if !literal.is_empty() {
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
    format: &[char],
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
            '-' => left_justify = true,
            '+' | ' ' | '#' | '0' => {}
            _ => break character,
        }
    };

    let mut width = None;
    if character.is_ascii_digit() {
        let mut value = 0_u32;
        let mut digits = 0;
        while character.is_ascii_digit() {
            value = value * 10 + character.to_digit(10).unwrap();
            character = next()?;
            digits += 1;
            if digits >= MAX_DIGITS {
                return None;
            }
        }
        width = Some(value);
    }

    let mut precision = None;
    if character == '.' {
        character = next()?;
        let mut value = 0_u32;
        if character.is_ascii_digit() {
            let mut digits = 0;
            while character.is_ascii_digit() {
                value = value * 10 + character.to_digit(10).unwrap();
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
        's' => Conversion::STR,
        'r' => Conversion::REPR,
        'a' => Conversion::ASCII,
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

#[expect(
    clippy::cast_precision_loss,
    reason = "CPython mixed integer/float folding rounds integer operands to binary64"
)]
pub(super) fn fold_constant(expression: &Expr) -> Option<Constant> {
    if let Some(constant) = literal_constant(expression) {
        return Some(constant);
    }
    match expression {
        Expr::Name(name) if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__" => {
            Some(Constant::Bool(true))
        }
        Expr::FString(fstring) => {
            let mut value = String::new();
            for part in &fstring.value {
                match part {
                    FStringPart::Literal(literal) => value.push_str(&literal.value),
                    FStringPart::FString(fstring) => {
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
            .map(fold_constant)
            .collect::<Option<Vec<_>>>()
            .map(Constant::Tuple),
        Expr::Subscript(subscript) if subscript.ctx == ExprContext::Load => {
            fold_literal_subscript(&subscript.value, &subscript.slice)
        }
        Expr::Slice(slice) => constant_slice(slice),
        Expr::UnaryOp(unary) => match unary.op {
            UnaryOp::UAdd => match fold_constant(&unary.operand)? {
                Constant::Bool(value) => Some(Constant::Int(u64::from(value))),
                constant @ (Constant::Int(_)
                | Constant::SignedInt(_)
                | Constant::BigInt { .. }
                | Constant::Float(_)
                | Constant::Complex { .. }) => Some(constant),
                _ => None,
            },
            UnaryOp::Not => literal_truthiness(&unary.operand).map(|value| Constant::Bool(!value)),
            UnaryOp::USub => fold_constant(&unary.operand)?.negated(),
            UnaryOp::Invert => {
                let operand = fold_constant(&unary.operand)?;
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
            let left = fold_constant(&binary.left)?;
            let right = fold_constant(&binary.right)?;
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
                (Constant::Int(left), Operator::FloorDiv, Constant::Int(right)) if right != 0 => {
                    Some(Constant::Int(left / right))
                }
                (left, Operator::FloorDiv, right)
                    if matches!(left, Constant::Float(_))
                        || matches!(right, Constant::Float(_)) =>
                {
                    fold_float_floor_divide(&left, &right).map(Constant::Float)
                }
                (Constant::Int(left), Operator::Mod, Constant::Int(right)) if right != 0 => {
                    Some(Constant::Int(left % right))
                }
                (Constant::Int(left), Operator::Pow, Constant::Int(right)) => {
                    const MAX_INT_SIZE: u64 = 128;

                    if left != 0 && right > 0 {
                        let left_bits = u64::from(u64::BITS - left.leading_zeros());
                        if left_bits > MAX_INT_SIZE / right {
                            return None;
                        }
                    }
                    let right = u32::try_from(right).ok()?;
                    if let Some(value) = left.checked_pow(right) {
                        Some(Constant::Int(value))
                    } else {
                        u128::from(left)
                            .checked_pow(right)
                            .map(|value| Constant::BigInt {
                                negative: false,
                                digits: big_integer_digits(&value.to_string()),
                            })
                    }
                }
                (
                    Constant::Int(0) | Constant::Bool(false),
                    Operator::Pow,
                    Constant::BigInt {
                        negative: false, ..
                    },
                ) => Some(Constant::Int(0)),
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
                    left @ (Constant::Bool(_) | Constant::Int(_) | Constant::SignedInt(_)),
                    Operator::Pow,
                    Constant::SignedInt(right),
                ) if right < 0 => {
                    let left = match left {
                        Constant::Bool(value) => f64::from(value),
                        Constant::Int(value) => value as f64,
                        Constant::SignedInt(value) => value as f64,
                        _ => unreachable!(),
                    };
                    let result = left.powf(right as f64);
                    result.is_finite().then_some(Constant::Float(result))
                }
                (
                    left,
                    operator @ (Operator::Add
                    | Operator::Sub
                    | Operator::Mult
                    | Operator::Div
                    | Operator::Pow),
                    right,
                ) if matches!(left, Constant::Float(_))
                    && matches!(
                        right,
                        Constant::Bool(_) | Constant::Int(_) | Constant::SignedInt(_)
                    )
                    || matches!(right, Constant::Float(_))
                        && matches!(
                            left,
                            Constant::Bool(_) | Constant::Int(_) | Constant::SignedInt(_)
                        ) =>
                {
                    let to_float = |constant: Constant| match constant {
                        Constant::Bool(value) => Some(f64::from(value)),
                        Constant::Int(value) => Some(value as f64),
                        Constant::SignedInt(value) => Some(value as f64),
                        Constant::Float(value) => Some(value),
                        _ => None,
                    };
                    let left = to_float(left)?;
                    let right = to_float(right)?;
                    match operator {
                        Operator::Add => Some(Constant::Float(left + right)),
                        Operator::Sub => Some(Constant::Float(left - right)),
                        Operator::Mult => Some(Constant::Float(left * right)),
                        Operator::Div if right != 0.0 => Some(Constant::Float(left / right)),
                        Operator::Pow => {
                            let result = left.powf(right);
                            result.is_finite().then_some(Constant::Float(result))
                        }
                        Operator::Div => None,
                        _ => unreachable!(),
                    }
                }
                (Constant::Float(left), Operator::Pow, Constant::Float(right)) => {
                    let result = left.powf(right);
                    result.is_finite().then_some(Constant::Float(result))
                }
                (Constant::Bool(left), Operator::Pow, Constant::Bool(right)) => {
                    Some(Constant::Int(u64::from(left).pow(u32::from(right))))
                }
                (left, operator @ (Operator::Add | Operator::Sub | Operator::Mult), right)
                    if matches!(left, Constant::Complex { .. })
                        || matches!(right, Constant::Complex { .. }) =>
                {
                    fold_complex_arithmetic(&left, operator, &right)
                }
                (left, Operator::Div, right)
                    if matches!(left, Constant::Complex { .. })
                        || matches!(right, Constant::Complex { .. }) =>
                {
                    fold_complex_division(&left, &right)
                }
                (left, Operator::Pow, right)
                    if matches!(left, Constant::Complex { .. })
                        || matches!(right, Constant::Complex { .. }) =>
                {
                    fold_complex_integer_power(&left, &right)
                }
                (Constant::String(mut left), Operator::Add, Constant::String(right)) => {
                    left.push_str(&right);
                    Some(Constant::String(left))
                }
                (Constant::Bytes(mut left), Operator::Add, Constant::Bytes(right)) => {
                    left.extend(right);
                    Some(Constant::Bytes(left))
                }
                (Constant::Tuple(mut left), Operator::Add, Constant::Tuple(right)) => {
                    left.extend(right);
                    Some(Constant::Tuple(left))
                }
                (Constant::String(value), Operator::Mult, Constant::Int(times))
                | (Constant::Int(times), Operator::Mult, Constant::String(value)) => {
                    let length = value.chars().count();
                    let Some(max_times) = 4096_usize.checked_div(length) else {
                        return Some(Constant::String(String::new()));
                    };
                    let times = usize::try_from(times).ok()?;
                    (times <= max_times).then(|| Constant::String(value.repeat(times)))
                }
                (Constant::Bytes(value), Operator::Mult, Constant::Int(times))
                | (Constant::Int(times), Operator::Mult, Constant::Bytes(value)) => {
                    let Some(max_times) = 4096_usize.checked_div(value.len()) else {
                        return Some(Constant::Bytes(Vec::new()));
                    };
                    let times = usize::try_from(times).ok()?;
                    (times <= max_times).then(|| Constant::Bytes(value.repeat(times)))
                }
                (Constant::Tuple(value), Operator::Mult, Constant::Int(times))
                | (Constant::Int(times), Operator::Mult, Constant::Tuple(value)) => {
                    let Some(max_times) = 256_usize.checked_div(value.len()) else {
                        return Some(Constant::Tuple(Vec::new()));
                    };
                    let times = usize::try_from(times).ok()?;
                    (times <= max_times).then(|| {
                        Constant::Tuple((0..times).flat_map(|_| value.iter().cloned()).collect())
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
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

/// Match the quotient from CPython's `_float_div_mod`, including rounding and signed zero.
fn fold_float_floor_divide(left: &Constant, right: &Constant) -> Option<f64> {
    let to_float = |constant: &Constant| match constant {
        Constant::Float(value) => Some(*value),
        // Dividing an integer by one provides PyLong_AsDouble's rounding and overflow checks.
        constant => fold_integer_true_divide(constant, &Constant::Int(1)),
    };
    let left = to_float(left)?;
    let right = to_float(right)?;
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

fn fold_literal_subscript(value: &Expr, index: &Expr) -> Option<Constant> {
    fn integer(expression: &Expr) -> Option<usize> {
        match fold_constant(expression)? {
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

    fn bound(expression: Option<&Expr>, length: usize, default: usize) -> Option<usize> {
        let Some(expression) = expression else {
            return Some(default);
        };
        integer(expression).map(|value| value.min(length))
    }

    fn indices(slice: &ruff_python_ast::ExprSlice, length: usize) -> Option<Vec<usize>> {
        let lower = bound(slice.lower.as_deref(), length, 0)?;
        let upper = bound(slice.upper.as_deref(), length, length)?;
        let step = slice.step.as_deref().map_or(Some(1), integer)?;
        if step == 0 {
            return None;
        }
        Some((lower.min(upper)..upper).step_by(step).collect())
    }

    let value = fold_constant(value)?;
    if !matches!(index, Expr::Slice(_)) {
        let index = match fold_constant(index)? {
            Constant::Bool(value) => i128::from(u8::from(value)),
            Constant::Int(value) => i128::from(value),
            Constant::SignedInt(value) => i128::from(value),
            _ => return None,
        };
        let length = match &value {
            Constant::String(value) => value.chars().count(),
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
            Constant::Bytes(value) => Some(Constant::Int(u64::from(value[index]))),
            Constant::Tuple(value) => Some(value[index].clone()),
            _ => None,
        };
    }

    let Expr::Slice(slice) = index else {
        unreachable!();
    };
    if slice.step.is_some() && constant_slice(slice).is_none() {
        return None;
    }

    match value {
        Constant::String(value) => {
            let characters = value.chars().collect::<Vec<_>>();
            Some(Constant::String(
                indices(slice, characters.len())?
                    .into_iter()
                    .map(|index| characters[index])
                    .collect(),
            ))
        }
        Constant::Bytes(value) => Some(Constant::Bytes(
            indices(slice, value.len())?
                .into_iter()
                .map(|index| value[index])
                .collect(),
        )),
        Constant::Tuple(value) => Some(Constant::Tuple(
            indices(slice, value.len())?
                .into_iter()
                .map(|index| value[index].clone())
                .collect(),
        )),
        _ => None,
    }
}

pub(super) fn folded_bool_operand(expression: &Expr) -> Option<(&Expr, Constant)> {
    let Expr::BoolOp(boolean) = expression else {
        return fold_constant(expression).map(|constant| (expression, constant));
    };
    let (last, leading) = boolean.values.split_last()?;
    for value in leading {
        if matches!(value, Expr::BoolOp(_)) {
            return None;
        }
        let constant = fold_constant(value)?;
        let truthiness = constant_truthiness(&constant);
        if matches!(boolean.op, BoolOp::And) && !truthiness
            || matches!(boolean.op, BoolOp::Or) && truthiness
        {
            return Some((value, constant));
        }
    }
    if matches!(last, Expr::BoolOp(_)) {
        None
    } else {
        fold_constant(last).map(|constant| (last, constant))
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
        Constant::Float(value) => Some((*value, 0.0)),
        Constant::Complex { real, imag } => Some((*real, *imag)),
        _ => None,
    }
}

fn fold_complex_arithmetic(
    left: &Constant,
    operator: Operator,
    right: &Constant,
) -> Option<Constant> {
    let (left_real, left_imag) = constant_complex_parts(left)?;
    let (right_real, right_imag) = constant_complex_parts(right)?;
    // CPython uses mixed real/complex formulas without introducing an imaginary
    // zero for the real operand. That distinction preserves signed zeros.
    let (real, imag) = match (left, right) {
        (Constant::Complex { .. }, Constant::Complex { .. }) => match operator {
            Operator::Add => (left_real + right_real, left_imag + right_imag),
            Operator::Sub => (left_real - right_real, left_imag - right_imag),
            Operator::Mult => complex_product((left_real, left_imag), (right_real, right_imag)),
            _ => unreachable!(),
        },
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

fn complex_quotient(left: (f64, f64), right: (f64, f64)) -> Option<(f64, f64)> {
    let abs_real = right.0.abs();
    let abs_imag = right.1.abs();
    let result = if abs_real >= abs_imag {
        if abs_real == 0.0 {
            return None;
        }
        let ratio = right.1 / right.0;
        let denominator = right.0 + right.1 * ratio;
        (
            (left.0 + left.1 * ratio) / denominator,
            (left.1 - left.0 * ratio) / denominator,
        )
    } else if abs_imag >= abs_real {
        let ratio = right.0 / right.1;
        let denominator = right.0 * ratio + right.1;
        (
            (left.0 * ratio + left.1) / denominator,
            (left.1 * ratio - left.0) / denominator,
        )
    } else {
        (f64::NAN, f64::NAN)
    };
    (result.0.is_finite() && result.1.is_finite()).then_some(result)
}

fn fold_complex_division(left: &Constant, right: &Constant) -> Option<Constant> {
    let left_parts = constant_complex_parts(left)?;
    let right_parts = constant_complex_parts(right)?;
    let (real, imag) = match (left, right) {
        (Constant::Complex { .. }, Constant::Complex { .. }) => {
            complex_quotient(left_parts, right_parts)?
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
            } else {
                let ratio = right_parts.0 / right_parts.1;
                let denominator = right_parts.0 * ratio + right_parts.1;
                (
                    (left_parts.0 * ratio) / denominator,
                    (-left_parts.0) / denominator,
                )
            }
        }
        _ => return None,
    };
    (real.is_finite() && imag.is_finite()).then_some(Constant::Complex { real, imag })
}

#[expect(
    clippy::float_cmp,
    reason = "CPython checks complex exponents for exact binary64 integrality"
)]
fn fold_complex_integer_power(base: &Constant, exponent: &Constant) -> Option<Constant> {
    let base = constant_complex_parts(base)?;
    let (exponent_real, exponent_imag) = constant_complex_parts(exponent)?;
    if exponent_imag != 0.0 || exponent_real != exponent_real.floor() || exponent_real.abs() > 100.0
    {
        return None;
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
            result = complex_product(result, power);
        }
        mask <<= 1;
        power = complex_product(power, power);
    }
    if exponent < 0 {
        result = complex_quotient((1.0, 0.0), result)?;
    }
    (result.0.is_finite() && result.1.is_finite()).then_some(Constant::Complex {
        real: result.0,
        imag: result.1,
    })
}

pub(super) fn constant_truthiness(constant: &Constant) -> bool {
    match constant {
        Constant::None => false,
        Constant::Bool(value) => *value,
        Constant::Ellipsis | Constant::Slice { .. } | Constant::Code(_) => true,
        Constant::Int(value) => *value != 0,
        Constant::SignedInt(value) => *value != 0,
        Constant::BigInt { digits, .. } => !digits.is_empty(),
        Constant::Float(value) => *value != 0.0,
        Constant::Complex { real, imag } => *real != 0.0 || *imag != 0.0,
        Constant::String(value) => !value.is_empty(),
        Constant::SurrogateString(value) => !value.is_empty(),
        Constant::Bytes(value) => !value.is_empty(),
        Constant::Tuple(value) | Constant::FrozenSet(value) => !value.is_empty(),
    }
}

pub(super) fn constant_slice(slice: &ruff_python_ast::ExprSlice) -> Option<Constant> {
    fn bound(expression: Option<&Expr>) -> Option<Constant> {
        expression.map_or(Some(Constant::None), literal_constant)
    }

    Some(Constant::Slice {
        lower: Box::new(bound(slice.lower.as_deref())?),
        upper: Box::new(bound(slice.upper.as_deref())?),
        step: Box::new(bound(slice.step.as_deref())?),
    })
}

pub(super) fn two_element_slice_optimization(slice: &ruff_python_ast::ExprSlice) -> bool {
    slice.step.is_none() && constant_slice(slice).is_none()
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
                        self.visit_expr(&loop_.iter);
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
                        // CPython defers a syntactic Constant return until after unwind.
                        // Tuple, unary and binary expressions still admit their components
                        // before cleanup, even when later optimization folds their value.
                        if let Some(value) = &return_.value
                            && !is_literal_constant(value)
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
            if let Expr::Call(call) = expression {
                self.visit_expr(&call.func);
                for argument in &call.arguments.args {
                    self.visit_expr(argument);
                }
                let shape = self.compiler.call_shape(call);
                let expanded = shape.has_starred
                    || shape.has_keyword_unpack
                    || (shape.direct_method.is_none()
                        && shape.positional_count + shape.keyword_count * 2 > 30);
                if expanded {
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
                if let Some(constant) = constant_slice(slice) {
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
            if let Some(constant) = literal_constant(expression) {
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
    use super::{
        CONSTANT_INDEX_THRESHOLD, ConstantEquality, ConstantIndices, ConstantPool,
        constant_pool_fingerprint,
    };
    use crate::code_object::{CodeObject, Constant, LocalsIdentity, MarshalProvenance};

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
            locals: vec!["value".to_string()],
            local_kinds: Vec::new(),
            filename: "module.py".to_string(),
            name: "<module>".to_string(),
            qualified_name: "<module>".to_string(),
            first_line_number: 1,
            line_table: Vec::new(),
            exception_table: Vec::new(),
            marshal_provenance: MarshalProvenance::default(),
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
        let values = [Constant::Int(1), Constant::Int(2)];
        let mut indices = ConstantIndices::default();
        indices.insert(Some(0), 0);
        indices.insert(Some(0), 1);

        assert_eq!(indices.find(&values, &Constant::Int(2), Some(0)), Some(1));
        assert_eq!(indices.find(&values, &Constant::Int(3), Some(0)), None);
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
        let duplicate_index = pool.intern(duplicate).unwrap();
        assert_eq!(pool.intern(compatible).unwrap(), duplicate_index);
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
            .subtree_has_frozenset_constants = Some(true);

        assert!(ConstantEquality::PoolKey.equal(
            &Constant::Code(Box::new(plain)),
            &Constant::Code(Box::new(with_graph_provenance))
        ));
    }
}
