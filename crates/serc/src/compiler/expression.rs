use std::collections::BTreeSet;
use std::rc::Rc;

use ruff_python_ast::visitor::{Visitor, walk_expr};
use ruff_python_ast::{
    BoolOp, CmpOp, ConversionFlag, Expr, ExprBinOp, ExprContext, ExprIf, FStringPart, Identifier,
    InterpolatedStringElement, Keyword, Number, Operator, Pattern, Singleton, Stmt, UnaryOp,
};
use ruff_text_size::Ranged;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    BINARY_SLICE, BUILD_INTERPOLATION, BUILD_LIST, BUILD_MAP, BUILD_SET, BUILD_SLICE, BUILD_STRING,
    BUILD_TEMPLATE, BUILD_TUPLE, CALL, CALL_FUNCTION_EX, CALL_KW, CLEANUP_THROW, COMPARE_OP,
    CONTAINS_OP, COPY, DELETE_ATTR, DELETE_SUBSCR, DICT_MERGE, DICT_UPDATE, END_ASYNC_FOR,
    END_SEND, FOR_ITER, FORMAT_SIMPLE, FORMAT_WITH_SPEC, GET_AITER, GET_ANEXT, GET_AWAITABLE,
    GET_ITER, GET_LEN, GET_YIELD_FROM_ITER, IS_OP, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT,
    JUMP_FORWARD, LIST_APPEND, LIST_EXTEND, LOAD_ATTR, LOAD_COMMON_CONSTANT, LOAD_CONST,
    LOAD_DEREF, LOAD_FAST, LOAD_FAST_AND_CLEAR, LOAD_GLOBAL, LOAD_SMALL_INT, LOAD_SUPER_ATTR,
    MAKE_CELL, MAP_ADD, MATCH_CLASS, MATCH_KEYS, MATCH_MAPPING, MATCH_SEQUENCE, NOP, NOT_TAKEN,
    POP_ITER, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, POP_TOP,
    PUSH_NULL, RETURN_VALUE, SEND, SET_ADD, SET_UPDATE, STORE_ATTR, STORE_FAST, STORE_SLICE,
    STORE_SUBSCR, SWAP, TO_BOOL, UNARY_INVERT, UNARY_NEGATIVE, UNARY_NOT, UNPACK_EX,
    UNPACK_SEQUENCE,
};
use crate::assembler::{ExceptionPolicy, Label, SourceLocation};
use crate::code_object::Constant;
use crate::target::code_flags::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR, CO_METHOD, CO_NESTED, CO_VARARGS,
    CO_VARKEYWORDS,
};
use crate::target::opcodes::{BUILD_CONST_KEY_MAP, FORMAT_VALUE, KW_NAMES};
use crate::target::operands::{
    BinaryOperation, CommonConstant, ComparisonOperation, Conversion, FunctionAttribute,
    ResumeLocation, UnaryIntrinsic,
};
use crate::target::{Opcode, is_scope_exit};
use crate::{CompileError, PythonVersion};

use super::constant::{
    PercentFormatPart, explicit_surrogate_string, first_literal_constant, fold_constant,
    fold_constant_for_version, folded_bool_operand, folded_constant_owner_slots,
    is_small_literal_tuple_element, number_constant, optimized_percent_format,
    python_constant_values_equal, two_element_slice_optimization,
};
use super::scope::{
    LoweringFunctionPlan, NameId, NameSet, analyze_lambda_scope,
    collect_nested_comprehension_target_names, collect_self_iterated_nested_comprehension_targets,
    collect_target_names, comprehension_cell_names, comprehension_name_bindings,
    comprehension_scope_facts, comprehension_temporary_names, generator_cellvars,
    generator_required_names, global_comprehension_temporary_names,
    inlined_comprehension_cell_names_in_expression, legacy_class_comprehension_names,
    parameter_default_expressions,
};
use super::source::{
    debug_text_range, first_expression_instruction_range, fstring_literal_range,
    statement_execution_range, strip_expression_comments, synthetic_condition_exit_range,
};
use super::statement::{
    condition_jump_target_count, final_chained_comparison_jump, is_wildcard_pattern,
    suite_has_linear_borrow_fallthrough, suite_terminates,
    terminal_for_break_exit_inherits_predecessor, unique_condition_jump_target,
};
use super::symbol::{
    ComparisonMode, ComprehensionRestore, ReraiseStackState, ResumeDepth, expression_name, to_u32,
    unsupported,
};
use super::{
    CPYTHON_STACK_USE_GUIDELINE, Compiler, SHADOWED_SUPER_SENTINEL, SUPPORTED_FUTURE_FLAGS, Scope,
};

#[derive(Clone, Copy)]
struct ComprehensionLoop {
    restart: Label,
    iterator_depth: usize,
    comprehension_location: SourceLocation,
}

#[derive(Debug)]
struct MatchContext {
    stores: Vec<NameId>,
    fail_pop: Vec<Label>,
    // Case or OR-alternative source identity; fail-pop indices belong to this context.
    native_fail_owner: ruff_text_size::TextRange,
    direct_loop_failure: Option<Label>,
    on_top: usize,
    ownerless_markers_after_specialized_none: bool,
    eager_final_mapping_capture: bool,
    empty_failure_jumps: Vec<(usize, usize, Option<Label>)>,
    empty_success_blocks: Vec<(Label, usize)>,
}

pub(super) struct PendingInterpolatedLiteral {
    value: String,
    surrogate_value: Option<Vec<u8>>,
    range: ruff_text_size::TextRange,
}

impl PendingInterpolatedLiteral {
    fn append(
        pending: &mut Option<Self>,
        source: &str,
        value: &str,
        source_range: ruff_text_size::TextRange,
        range: ruff_text_size::TextRange,
        is_raw: bool,
    ) {
        let surrogate_value = (!is_raw && value.contains('\u{fffd}'))
            .then(|| {
                let start = usize::from(source_range.start());
                let end = usize::from(source_range.end());
                explicit_surrogate_string(value, &source[start..end])
            })
            .flatten();
        if let Some(pending) = pending {
            if let Some(surrogate_value) = surrogate_value {
                pending
                    .surrogate_value
                    .get_or_insert_with(|| pending.value.as_bytes().to_vec())
                    .extend(surrogate_value);
            } else if let Some(pending_value) = &mut pending.surrogate_value {
                pending_value.extend_from_slice(value.as_bytes());
            }
            pending.value.push_str(value);
            pending.range = ruff_text_size::TextRange::new(pending.range.start(), range.end());
        } else {
            *pending = Some(Self {
                value: value.to_string(),
                surrogate_value,
                range,
            });
        }
    }

    pub(super) fn into_constant(self) -> Constant {
        if let Some(value) = self.surrogate_value {
            Constant::SurrogateString(value)
        } else {
            Constant::String(self.value)
        }
    }

    fn load(self, compiler: &mut Compiler<'_>) -> Result<(), CompileError> {
        compiler
            .assembler
            .set_location(compiler.source_location(self.range));
        let constant = compiler.add_constant(self.into_constant())?;
        compiler.emit(LOAD_CONST, constant, 1)
    }
}

/// A merged literal or interpolation in CPython's formatted-string evaluation order.
pub(super) enum InterpolatedStringComponent<'ast> {
    Literal(PendingInterpolatedLiteral),
    Interpolation(&'ast ruff_python_ast::InterpolatedElement, bool),
}

fn flush_interpolated_literal(
    pending: &mut Option<PendingInterpolatedLiteral>,
    components: &mut Vec<InterpolatedStringComponent<'_>>,
    discard_empty: bool,
) {
    if let Some(literal) = pending.take()
        && (!discard_empty || !literal.value.is_empty())
    {
        components.push(InterpolatedStringComponent::Literal(literal));
    }
}

fn append_interpolated_elements<'ast>(
    elements: &'ast ruff_python_ast::InterpolatedStringElements,
    is_raw: bool,
    source: &str,
    pending: &mut Option<PendingInterpolatedLiteral>,
    components: &mut Vec<InterpolatedStringComponent<'ast>>,
    discard_empty: bool,
) {
    for element in elements {
        match element {
            InterpolatedStringElement::Literal(literal) => {
                PendingInterpolatedLiteral::append(
                    pending,
                    source,
                    &literal.value,
                    literal.range,
                    if discard_empty {
                        fstring_literal_range(source, literal.range, is_raw)
                    } else {
                        literal.range
                    },
                    is_raw,
                );
            }
            InterpolatedStringElement::Interpolation(interpolation) => {
                if let Some(debug) = &interpolation.debug_text {
                    PendingInterpolatedLiteral::append(
                        pending,
                        source,
                        &strip_expression_comments(debug.as_str()),
                        debug_text_range(interpolation),
                        debug_text_range(interpolation),
                        true,
                    );
                }
                flush_interpolated_literal(pending, components, discard_empty);
                components.push(InterpolatedStringComponent::Interpolation(
                    interpolation,
                    is_raw,
                ));
            }
        }
    }
}

/// Merge adjacent literal tokens and debug text before visiting interpolated expressions.
pub(super) fn fstring_components<'ast>(
    fstring: &'ast ruff_python_ast::ExprFString,
    source: &str,
) -> Vec<InterpolatedStringComponent<'ast>> {
    let mut components = Vec::new();
    let mut pending = None;
    for part in &fstring.value {
        match part {
            FStringPart::Literal(literal) => PendingInterpolatedLiteral::append(
                &mut pending,
                source,
                &literal.value,
                literal.range,
                literal.range,
                literal.flags.prefix().is_raw(),
            ),
            FStringPart::FString(fstring) => append_interpolated_elements(
                &fstring.elements,
                fstring.flags.prefix().is_raw(),
                source,
                &mut pending,
                &mut components,
                true,
            ),
        }
    }
    flush_interpolated_literal(&mut pending, &mut components, true);
    components
}

/// Normalize format-specification fragments using the same literal merging rules.
pub(super) fn format_spec_components<'ast>(
    elements: &'ast ruff_python_ast::InterpolatedStringElements,
    is_raw: bool,
    source: &str,
) -> Vec<InterpolatedStringComponent<'ast>> {
    let mut components = Vec::new();
    let mut pending = None;
    append_interpolated_elements(
        elements,
        is_raw,
        source,
        &mut pending,
        &mut components,
        false,
    );
    flush_interpolated_literal(&mut pending, &mut components, false);
    components
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollectionKind {
    List,
    Tuple,
    Set,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpecializedLowering {
    Applied,
    NotApplicable,
}

#[derive(Clone, Copy, Debug)]
enum SuperAttributeKind {
    Attribute,
    Method,
}

impl SuperAttributeKind {
    const fn method_flag(self) -> u32 {
        match self {
            Self::Attribute => 0,
            Self::Method => 1,
        }
    }

    const fn stack_effect(self) -> i32 {
        match self {
            Self::Attribute => -2,
            Self::Method => -1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ConditionMarkerPolicy {
    excluded: ExceptionPolicy,
    ordinary: ExceptionPolicy,
}

impl Compiler<'_> {
    /// Defer restoration only when CPython can exchange it with a fast-local store.
    pub(super) fn assignment_defers_async_comprehension_restore(
        &self,
        assignment: &ruff_python_ast::StmtAssign,
    ) -> bool {
        // Legacy CFG passes need the original restore/store order: 3.13 resolves
        // successor locations before static swaps, and 3.12 counts the SWAP when
        // deciding which cleanup blocks to inline. Let those passes exchange it.
        if self.context.python_version <= PythonVersion::Py313
            || !expression_defers_async_comprehension_restore(&assignment.value)
        {
            return false;
        }
        let [Expr::Name(target)] = assignment.targets.as_slice() else {
            return false;
        };
        let Scope::Function {
            indices,
            free_indices,
            cells,
            globals,
        } = &self.unit.scope
        else {
            return false;
        };
        let name = self.required_name_id(target.id.as_str());
        if !indices.contains_key(&name)
            || free_indices.contains_key(&name)
            || cells.contains(name)
            || globals.contains(name)
            || self.source_location(target.range).line
                != self.source_location(assignment.value.range()).line
        {
            return false;
        }
        let (generators, key, value) = match assignment.value.as_ref() {
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
            _ => unreachable!("only inlined asynchronous comprehensions defer restoration"),
        };
        // A store to any saved comprehension local prevents the static permutation.
        !comprehension_temporary_names(generators, key, value)
            .iter()
            .any(|saved| saved == target.id.as_str())
    }

    pub(super) fn compile_match(
        &mut self,
        statement: &ruff_python_ast::StmtMatch,
    ) -> Result<(), CompileError> {
        self.compile_match_inner(statement, false)
    }

    /// Carries a match successor only to its direct case-tail iterator break.
    fn compile_match_case_suite(
        &mut self,
        body: &[Stmt],
        match_has_following: bool,
    ) -> Result<(), CompileError> {
        let iterator_continuation = if match_has_following
            && let Some(Stmt::For(iterator)) = body.last()
            && !iterator.is_async
            && iterator.orelse.is_empty()
        {
            Some(iterator.range)
        } else {
            None
        };
        self.with_control_override(
            |control| &mut control.loops.match_case_iterator_continuation,
            iterator_continuation,
            |compiler| compiler.compile_suite(body),
        )
    }

    /// Recognizes the three original records in a direct global-attribute return.
    pub(super) fn is_direct_global_attribute_return(
        &self,
        returned: &ruff_python_ast::StmtReturn,
    ) -> bool {
        let Some(Expr::Attribute(attribute)) = returned.value.as_deref() else {
            return false;
        };
        let Expr::Name(name) = attribute.value.as_ref() else {
            return false;
        };
        if name.id.as_str() == "__debug__" || self.unit.annotation_classdict_index.is_some() {
            return false;
        }
        let name = self.required_name_id(name.id.as_str());
        !self.symbols.active_temporaries.contains(name)
            && matches!(&self.unit.scope, Scope::Function { indices, free_indices, globals, .. }
                if globals.contains(name)
                    || (!indices.contains_key(&name) && !free_indices.contains_key(&name)))
    }

    /// Admits a four-record subject-pop return whose copied entry has a fixed guard donor.
    fn can_copy_match_guard_subject_return(
        &self,
        pattern: &Pattern,
        guard: &Expr,
        body: &[Stmt],
    ) -> bool {
        self.context.python_version >= PythonVersion::Py314
            && matches!(pattern, Pattern::MatchAs(pattern)
                if pattern.pattern.is_none() && pattern.name.is_some())
            && matches!(guard, Expr::Compare(comparison)
                if comparison.ops.len() == 2
                    && !(matches!(comparison.comparators.last(), Some(Expr::NoneLiteral(_)))
                        && matches!(comparison.ops.last(), Some(CmpOp::Is | CmpOp::IsNot))))
            && matches!(body, [Stmt::Return(returned)]
                if self.is_direct_global_attribute_return(returned))
            && self.match_guard_subject_return_is_unprotected()
    }

    /// Keeps a returning case's comparison bridge eligible for original-budget exit copying.
    fn compile_match_guard(
        &mut self,
        guard: &Expr,
        body: &[Stmt],
        otherwise: Label,
    ) -> Result<(), CompileError> {
        let retained_exit = matches!(body.last(), Some(Stmt::Return(_)))
            .then(|| final_chained_comparison_jump(guard))
            .flatten();
        self.with_control_override(
            |control| &mut control.conditions.retained_chained_comparison_exit,
            retained_exit,
            |compiler| compiler.compile_jump_if(guard, false, otherwise),
        )
    }

    pub(super) fn compile_match_inner(
        &mut self,
        statement: &ruff_python_ast::StmtMatch,
        terminal: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.stack.depth;
        let match_has_following =
            !terminal && self.control.statements.current_statement_has_following;
        if self.context.python_version >= PythonVersion::Py314
            && matches!(statement.subject.as_ref(), Expr::Tuple(_))
            && let Some(constant) =
                fold_constant_for_version(&statement.subject, self.context.python_version)
        {
            self.record_folded_operands(&statement.subject)?;
            if self.output.constants.is_empty()
                && let Some(seed) = first_literal_constant(&statement.subject)
            {
                self.add_constant(seed)?;
            }
            self.assembler
                .set_location(self.source_location(statement.subject.range()));
            self.emit_deferred_constant(constant)?;
        } else {
            self.compile_expression(&statement.subject)?;
        }
        let end = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::MatchEnd(statement.range),
            end,
        );
        let has_default = statement.cases.len() > 1
            && statement
                .cases
                .last()
                .is_some_and(|case| is_wildcard_pattern(&case.pattern));
        let matched_cases = statement.cases.len() - usize::from(has_default);
        let mut failure_reaches_end = false;
        let mut specialized_none_seen = false;
        let mut or_default_success_exit = None;
        let mut aliased_or_failure_jumps = Vec::new();

        for (index, case) in statement.cases[..matched_cases].iter().enumerate() {
            let copied_subject = index + 1 != matched_cases;
            if !copied_subject
                && !has_default
                && case.guard.is_none()
                && (matches!(case.body.as_slice(), [Stmt::Pass(_)])
                    || (matches!(case.body.as_slice(), [Stmt::Continue(_)])
                        && self.control.loops.non_final_match_tail != Some(statement.range)))
                && matches!(&case.pattern, Pattern::MatchSingleton(pattern) if pattern.value == Singleton::None)
                && self.control.falls_through_to_loop_backedge()
                && self.control.generators.generator_region_start.is_none()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self.control.withs.active_with_region_exclusions.is_empty()
            {
                // A final pass or direct-tail continue has separate failure and body exits.
                // A non-final branch keeps its outer join and the ordinary failure layout.
                // Keep the authored body location on the successful None branch.
                self.add_constant(Constant::None)?;
                let success = self.assembler.label();
                self.assembler
                    .set_location(self.source_location(case.pattern.range()));
                self.emit_jump_forward(POP_JUMP_IF_NONE, success, -1)?;
                self.emit_with_exception_policy(
                    NOT_TAKEN,
                    0,
                    0,
                    ExceptionPolicy::normalized_owner(true),
                )?;
                self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
                self.assembler.mark(success);
                self.assembler
                    .set_location(self.source_location(case.body[0].range()));
                self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
                failure_reaches_end = true;
                continue;
            }
            if copied_subject {
                self.assembler
                    .set_location(self.source_location(case.pattern.range()));
                self.emit(COPY, 1, 1)?;
            }
            let failure_depth = base_depth + i32::from(copied_subject);
            let mut context = MatchContext {
                stores: Vec::new(),
                fail_pop: Vec::new(),
                native_fail_owner: case.pattern.range(),
                direct_loop_failure: (!copied_subject
                    && !has_default
                    && case.guard.is_none()
                    && matches!(&case.pattern, Pattern::MatchValue(_))
                    && self.control.loops.non_final_match_tail != Some(statement.range)
                    && self.control.falls_through_to_loop_backedge())
                .then_some(end),
                on_top: 0,
                ownerless_markers_after_specialized_none: specialized_none_seen,
                eager_final_mapping_capture: case.guard.is_some() && !copied_subject,
                empty_failure_jumps: Vec::new(),
                empty_success_blocks: Vec::new(),
            };
            self.compile_stack_pattern(&case.pattern, &mut context)?;
            specialized_none_seen = context.ownerless_markers_after_specialized_none;
            for &name in &context.stores {
                self.assembler
                    .set_location(self.source_location(case.pattern.range()));
                self.store_name_id(name)?;
            }
            let mut guard_always_false = false;
            let mut irrefutable_true_guard = false;
            if let Some(guard) = &case.guard {
                if let Some(truthiness) = early_condition_truthiness(guard) {
                    self.record_folded_value(guard)?;
                    irrefutable_true_guard = truthiness && context.fail_pop.is_empty();
                    if irrefutable_true_guard {
                        let location = self.source_location(guard.range());
                        if self
                            .assembler
                            .last_instruction_location()
                            .is_none_or(|previous| previous.line != location.line)
                        {
                            self.assembler.set_location(location);
                            self.emit(NOP, 0, 0)?;
                        }
                    }
                    if !truthiness || !context.fail_pop.is_empty() {
                        let range = if is_wildcard_pattern(&case.pattern) {
                            case.pattern.range()
                        } else {
                            guard.range()
                        };
                        let location = self.source_location(range);
                        let redundant_capture_line = truthiness
                            && !context.stores.is_empty()
                            && self.source_location(case.pattern.range()).line == location.line;
                        self.assembler.set_location(if redundant_capture_line {
                            SourceLocation::NONE
                        } else {
                            location
                        });
                        self.emit(NOP, 0, 0)?;
                        if redundant_capture_line {
                            // Native constant folding removes the guard's line after the
                            // same-line capture store. Keep its slot for original block sizing.
                            self.assembler.preserve_last_no_location();
                        }
                    }
                    if !truthiness {
                        self.ensure_match_fail_pop(&mut context, 0);
                        guard_always_false = true;
                    }
                } else {
                    self.ensure_match_fail_pop(&mut context, 0);
                    if copied_subject
                        && is_wildcard_pattern(&case.pattern)
                        && self.source_location(case.pattern.range()).line
                            != self
                                .source_location(first_expression_instruction_range(
                                    guard,
                                    self.context.python_version,
                                ))
                                .line
                    {
                        // CPython replaces the wildcard's COPY/POP_TOP pair with NOPs.
                        // Its pattern line survives when the guard begins on a later line.
                        self.assembler
                            .set_location(self.source_location(case.pattern.range()));
                        self.emit(NOP, 0, 0)?;
                    }
                    if !copied_subject
                        || self.can_copy_match_guard_subject_return(
                            &case.pattern,
                            guard,
                            &case.body,
                        )
                    {
                        // The assembler transports the original bridge donor only onto an
                        // intact private copy of the four-record subject-pop return.
                        self.compile_match_guard(guard, &case.body, context.fail_pop[0])?;
                    } else {
                        self.compile_jump_if(guard, false, context.fail_pop[0])?;
                    }
                }
            }
            if !guard_always_false {
                if copied_subject {
                    let location = if self.context.python_version <= PythonVersion::Py313 {
                        self.source_location(case.pattern.range())
                    } else {
                        case.body.first().map_or(SourceLocation::NONE, |statement| {
                            self.source_location(statement_execution_range(
                                statement,
                                self.context.python_version,
                            ))
                        })
                    };
                    self.assembler.set_location(location);
                    self.emit(POP_TOP, 0, -1)?;
                    if self.context.python_version >= PythonVersion::Py314 {
                        self.assembler.use_next_location_for_last_instruction();
                    }
                }
                let body_previous_location = self.assembler.last_instruction_location();
                let body_start = self.assembler.instruction_count();
                let tail_if = if !terminal
                    && let Some((Stmt::If(statement), leading)) = case.body.split_last()
                    && statement.elif_else_clauses.is_empty()
                    && early_condition_truthiness(&statement.test).is_none()
                    && (!suite_terminates(&statement.body)
                        || !self.control.falls_through_to_loop_backedge())
                {
                    Some((statement, leading))
                } else {
                    None
                };
                let emitted_case_exit = if let Some((statement, leading)) = tail_if {
                    let loop_tail = self.control.falls_through_to_loop_backedge();
                    self.compile_suite(leading)?;
                    self.compile_match_case_tail_if(
                        statement,
                        end,
                        loop_tail,
                        index + 1 == matched_cases && !context.fail_pop.is_empty(),
                    )?;
                    true
                } else {
                    false
                };
                let emitted_fallthrough = if emitted_case_exit {
                    false
                } else if terminal && matches!(case.body.last(), Some(Stmt::If(_) | Stmt::Match(_)))
                {
                    let ((), emitted_fallthrough) = self.with_fallthrough_tracking(|compiler| {
                        compiler.compile_suite_inner(&case.body, true)
                    })?;
                    emitted_fallthrough
                } else if !terminal
                    && index + 1 < matched_cases
                    && case.guard.is_none()
                    && matches!(&case.pattern, Pattern::MatchValue(_))
                    && let [Stmt::Try(tail)] = case.body.as_slice()
                    && self.compile_match_case_tail_try(tail, end)?
                {
                    // Handler fallthrough still needs the case's original success exit.
                    false
                } else {
                    // A non-default case appends its own success jump after a nested match.
                    // Its retained original slot prevents that match's final value failure
                    // from threading directly to the enclosing loop backedge.
                    let non_final_match_tail = if !terminal
                        && self.control.falls_through_to_loop_backedge()
                        && let Some(Stmt::Match(nested)) = case.body.last()
                        && nested.cases.last().is_some_and(|case| {
                            case.guard.is_none() && matches!(case.pattern, Pattern::MatchValue(_))
                        }) {
                        Some(nested.range)
                    } else {
                        self.control.loops.non_final_match_tail
                    };
                    // Only a direct case-tail match can thread into this case's exit.
                    // A later case or default keeps that outer exit distinct from its join.
                    let match_tail_before_case_exit = if !terminal
                        && (copied_subject || has_default)
                        && let Some(Stmt::Match(nested)) = case.body.last()
                    {
                        Some(nested.range)
                    } else {
                        None
                    };
                    self.with_control_override(
                        |control| &mut control.loops.non_final_match_tail,
                        non_final_match_tail,
                        |compiler| {
                            compiler.with_control_override(
                                |control| &mut control.statements.match_tail_before_case_exit,
                                match_tail_before_case_exit,
                                |compiler| {
                                    compiler
                                        .compile_match_case_suite(&case.body, match_has_following)
                                },
                            )
                        },
                    )?;
                    false
                };
                if self.context.python_version <= PythonVersion::Py313
                    && self.assembler.instruction_count() == body_start
                    && let [Stmt::Pass(pass)] = case.body.as_slice()
                {
                    // Protected suites can omit a final Pass in favor of their cleanup.
                    // Legacy case jumps start unlocated, so retain its original donor NOP.
                    self.assembler
                        .set_location(self.source_location(pass.range));
                    self.emit(NOP, 0, 0)?;
                }
                if !context.empty_failure_jumps.is_empty()
                    && self
                        .assembler
                        .instruction_opcode_at(self.assembler.instruction_count() - 1)
                        .is_some_and(is_scope_exit)
                {
                    // Copying the small success exit leaves the OR failure path behind an
                    // empty block during borrow analysis. Count from the success label:
                    // capture stores, stack rotations, and both subject pops also belong
                    // to the copied block.
                    for &(failure, success_start, cleanup) in &context.empty_failure_jumps {
                        if self
                            .assembler
                            .pre_fusion_instruction_count_since(success_start)
                            <= 4
                        {
                            if cleanup.is_none() {
                                self.assembler.mark_borrow_empty_block_at(failure);
                            } else if has_default
                                && index + 1 == matched_cases
                                && case.guard.is_none()
                                && cleanup.is_some_and(|label| context.fail_pop.contains(&label))
                                && self
                                    .assembler
                                    .is_small_linear_scope_exit_since(success_start)
                            {
                                aliased_or_failure_jumps.push(failure);
                            }
                        }
                    }
                }
                if !emitted_case_exit && !suite_terminates(&case.body) && !emitted_fallthrough {
                    // The final nested pattern's failure reaches this outer case exit;
                    // successful inner cases have already copied or bypassed that exit.
                    let nested_pattern_failure = matches!(case.body.last(), Some(Stmt::Match(nested))
                    if nested.cases.last().is_some_and(|case| {
                        case.guard.is_none()
                            && matches!(case.pattern, Pattern::MatchValue(_))
                    }));
                    if !terminal
                        && index + 1 == matched_cases
                        && !has_default
                        && !nested_pattern_failure
                    {
                        // This final case falls into the following code after its jump is
                        // removed; it does not retain a separate failed-pattern exit.
                        self.set_fallthrough_end_location(&case.body, body_start);
                    } else {
                        // A nested value-pattern failure reaches the outer case's exit after
                        // successful inner cases have copied it, supplying its pattern span.
                        self.set_branch_end_location(&case.body, body_start);
                    }
                    self.set_match_case_backedge_location(&case.body);
                    if self.context.python_version <= crate::PythonVersion::Py313
                        || (terminal && terminal_for_break_exit_inherits_predecessor(&case.body))
                    {
                        // Native case exits receive their span in the target CFG schedule.
                        // A terminal loop break supplies its own span after exit copying.
                        self.assembler.set_location(SourceLocation::NONE);
                    }
                    if terminal {
                        self.emit_deferred_implicit_return()?;
                        // CPython copies the implicit return onto the original case-success
                        // jump before considering copies onto an OR pattern's success edges.
                        self.assembler.add_last_inlined_jump_slot();
                    } else {
                        let original_unlocated_head = self.assembler.at_jump_target();
                        let authored_pass_exit = self.assembler.last_instruction_is_nop()
                            && !self.assembler.last_nop_repeats_previous_line()
                            && matches!(case.body.last(), Some(Stmt::Pass(_)));
                        let fallthrough_if_join = !original_unlocated_head
                            && self.following_name_return_fits_match_copy_budget()
                            && case.body.last().is_some_and(|statement| {
                                let Stmt::If(tail) = statement else {
                                    return false;
                                };
                                let Some((otherwise, branches)) =
                                    tail.elif_else_clauses.split_last()
                                else {
                                    return false;
                                };
                                early_condition_truthiness(&tail.test).is_none()
                                    && suite_terminates(&tail.body)
                                    && otherwise.test.is_none()
                                    && !suite_terminates(&otherwise.body)
                                    && branches.iter().all(|clause| {
                                        clause.test.as_ref().is_some_and(|test| {
                                            early_condition_truthiness(test).is_none()
                                        }) && suite_terminates(&clause.body)
                                    })
                            });
                        // A complete if with terminating tested arms still has an original
                        // empty join when only its else suite falls through. It needs no
                        // incoming jump to retain the slot replaced by a small return.
                        // An assertion's sole success edge enters an original empty
                        // array after its terminal raise. Its case-success jump keeps
                        // that slot when a small following return is copied into it.
                        let assertion_success_join = original_unlocated_head
                            && case.body.last().is_some_and(|statement| {
                                matches!(statement, Stmt::Assert(assertion)
                                    if early_condition_truthiness(&assertion.test).is_none()
                                        && unique_condition_jump_target(&assertion.test, true)
                                            .is_some())
                            });
                        let original_empty_join = self.assembler.at_original_match_join()
                            || fallthrough_if_join
                            || assertion_success_join;
                        // The final async-for exhaustion falls directly into the following
                        // match continuation before cold blocks move. Its surviving bridge
                        // represents that fallthrough, not an interruptible case-success jump.
                        let final_async_exhaustion = match_has_following
                            && index + 1 == matched_cases
                            && !has_default
                            && self
                                .assembler
                                .instruction_opcode_at(self.assembler.instruction_count() - 1)
                                == Some(END_ASYNC_FOR);
                        let success_exit = self
                            .emit_jump_forward_instruction_with_exception_policy(
                                JUMP_FORWARD,
                                end,
                                if final_async_exhaustion
                                    && self.context.python_version != PythonVersion::Py312
                                {
                                    // Cold ordering materializes the original fallthrough as a
                                    // new bridge; it has no original exception-owner slot.
                                    ExceptionPolicy::EXCLUDE_ALWAYS
                                } else {
                                    ExceptionPolicy::default()
                                },
                            )?;
                        self.assembler
                            .record_original_match_success(success_exit, original_empty_join);
                        if original_unlocated_head {
                            self.assembler.mark_as_artificial_eval_break(success_exit);
                            if let Some(Stmt::If(tail)) = case.body.last()
                                && !(self
                                    .control
                                    .statements
                                    .match_successor_fills_small_exit_copy_budget
                                    && match_case_has_nested_jump_predecessor(tail))
                                && early_condition_truthiness(&tail.test).is_none()
                                && (tail
                                    .elif_else_clauses
                                    .last()
                                    .is_some_and(|clause| clause.test.is_none())
                                    || (index + 1 == matched_cases
                                        && !has_default
                                        && !self.control.falls_through_to_loop_backedge()
                                        && !self.following_name_return_fits_match_copy_budget()
                                        && tail.elif_else_clauses.last().is_some_and(|clause| {
                                            clause.test.is_some()
                                                && suite_has_linear_borrow_fallthrough(&clause.body)
                                        })))
                                && tail.elif_else_clauses.iter().all(|clause| {
                                    clause.test.as_ref().is_none_or(|test| {
                                        early_condition_truthiness(test).is_none()
                                    })
                                })
                            {
                                // The case-success jump occupies the nested if's empty join.
                                // Thread incoming branches before retaining its separate line.
                                self.assembler
                                    .preserve_redundant_jump_nop_after_threading(success_exit);
                            }
                        }
                        if index + 1 == matched_cases
                            && (!context.fail_pop.is_empty()
                                || context.direct_loop_failure.is_some()
                                || assertion_success_join)
                        {
                            // CPython duplicates small exits before removing this jump across
                            // the empty final pattern-failure block. An assertion success
                            // also keeps its original jump until conditional threading.
                            self.assembler.defer_jump_removal(success_exit);
                        }
                        let subject_pop_is_optimized_away = !copied_subject
                            && context.stores.is_empty()
                            && fold_constant_for_version(
                                &statement.subject,
                                self.context.python_version,
                            )
                            .is_some();
                        let previous_instruction_covers_branch = context.fail_pop.is_empty()
                            && !subject_pop_is_optimized_away
                            && body_previous_location.is_some_and(|location| {
                                location.line == self.assembler.location().line
                            });
                        if irrefutable_true_guard {
                            self.assembler.preserve_inlined_jump_nop(success_exit);
                        } else if authored_pass_exit && case.body.len() > 1 {
                            // The case-success jump absorbed an authored trailing Pass NOP.
                            // Copying or retargeting its exit must keep that line event.
                            self.assembler.preserve_authored_pass_jump_nop(success_exit);
                        } else if self.assembler.instruction_count() == body_start + 1
                            && !previous_instruction_covers_branch
                            && !matches!(self.unit.scope, Scope::Class { .. })
                            // A pass success with original failure cleanup still owns a JUMP.
                            // At a loop tail it must thread to the backedge; retaining its
                            // removed-jump NOP would leave a forward bridge.
                            && !(case.guard.is_none()
                                && matches!(case.body.as_slice(), [Stmt::Pass(_)])
                                && self.control.falls_through_to_loop_backedge()
                                && ((has_default
                                    && matches!(case.pattern, Pattern::MatchValue(_)))
                                    || !context.fail_pop.is_empty()))
                        {
                            if authored_pass_exit
                                && self.control.statements.match_tail_before_case_exit
                                    == Some(statement.range)
                                && !copied_subject
                                && !has_default
                                && index + 1 == matched_cases
                                && case.guard.is_none()
                                && matches!(case.pattern, Pattern::MatchValue(_))
                                && context.fail_pop.len() == 1
                                && !self.control.falls_through_to_loop_backedge()
                            {
                                // This final value-pattern failure has a zero-pop join.
                                // Keep the Pass line on its success jump while allowing it
                                // to thread through an enclosing case's original exit.
                                self.assembler.preserve_authored_pass_jump_nop(success_exit);
                            } else if authored_pass_exit
                                && !copied_subject
                                && case.guard.is_none()
                                && matches!(case.body.as_slice(), [Stmt::Pass(_)])
                                && !context.fail_pop.is_empty()
                                && !self.control.falls_through_to_loop_backedge()
                                && body_start
                                    .checked_sub(1)
                                    .and_then(|index| self.assembler.instruction_opcode_at(index))
                                    == Some(UNPACK_SEQUENCE)
                            {
                                // The final refutable class pattern still has its original
                                // unpack record before this Pass success. Its jump can thread
                                // through the enclosing join while keeping the authored line.
                                self.assembler.preserve_authored_pass_jump_nop(success_exit);
                            } else if copied_subject
                                && case.guard.is_none()
                                && matches!(case.body.as_slice(), [Stmt::Pass(_)])
                                && !context.fail_pop.is_empty()
                                && !self.control.falls_through_to_loop_backedge()
                            {
                                // The subject POP keeps this success array nonempty. Thread
                                // its exit before restoring the authored Pass line's NOP.
                                self.assembler
                                    .preserve_redundant_jump_nop_after_threading(success_exit);
                            } else {
                                self.assembler.preserve_inlined_jump_nop(success_exit);
                            }
                            // CPython keeps this retained success-exit NOP on the physical
                            // fallthrough for irrefutable explicit-global captures. Refutable
                            // and local captures keep the ordinary borrowed-load barrier.
                            if context.fail_pop.is_empty()
                                && !context.stores.is_empty()
                                && matches!(
                                    &self.unit.scope,
                                    Scope::Function { globals, .. }
                                        if context.stores.iter().all(|name| globals.contains(*name))
                                )
                            {
                                self.assembler
                                    .mark_retained_nop_borrow_fallthrough(success_exit);
                            }
                        } else {
                            // A surviving case-success edge is CPython's JUMP. Cleanup
                            // predecessors threaded through it keep its eval-breaker check.
                            if !final_async_exhaustion {
                                self.assembler.mark_jump_as_interruptible(success_exit);
                            }
                            if original_unlocated_head {
                                self.assembler.inherit_nested_if_match_exit_locations(
                                    success_exit,
                                    original_unlocated_head,
                                );
                            }
                        }
                        if self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) == 0
                            && has_default
                            && index + 1 == matched_cases
                            && case.guard.is_none()
                            && matches!(case.pattern, Pattern::MatchOr(_))
                            && context.fail_pop.len() == 1
                            && !context.empty_failure_jumps.is_empty()
                        {
                            or_default_success_exit = Some(self.assembler.instruction_count() - 1);
                        }
                        if matches!(case.body.last(), Some(Stmt::For(tail))
                                if !tail.is_async && tail.orelse.is_empty())
                        {
                            // The loop's end is a separate original array after iterator
                            // cleanup. Removing its sole case-success jump leaves that
                            // array empty, so borrowing cannot visit the match continuation.
                            self.assembler
                                .mark_borrow_empty_block_if_unretained(success_exit);
                        }
                    }
                }
            } else {
                self.pre_register_suite_names(&case.body)?;
            }
            for &(success, success_start) in &context.empty_success_blocks {
                // Small exits copied onto the OR success edges remain reachable even when
                // their original shared block has become unreachable.
                let copied_exit = self
                    .assembler
                    .instruction_opcode_at(self.assembler.instruction_count() - 1)
                    .is_some_and(is_scope_exit)
                    && self
                        .assembler
                        .pre_fusion_instruction_count_since(success_start)
                        <= 4;
                if !copied_exit {
                    self.assembler.prevent_borrow_reachability(success);
                }
            }
            if !context.fail_pop.is_empty() {
                self.emit_match_fail_pop(
                    &context,
                    self.source_location(case.pattern.range()),
                    failure_depth,
                )?;
                if index + 1 != matched_cases {
                    // CPython starts every following case in a fresh CFG block. Keep the
                    // failure cleanup separate so it cannot absorb the next case as a small
                    // exit when a large OR pattern pushes its success target out of range.
                    let boundary = self.assembler.label();
                    self.assembler.preserve_block_boundary_if_new(boundary);
                    self.assembler.mark(boundary);
                }
                failure_reaches_end = true;
            } else {
                failure_reaches_end = false;
                if index + 1 != matched_cases {
                    // A constant-true guard can make a syntactically guarded,
                    // irrefutable case unconditional. The remaining cases are
                    // unreachable, but code generation still starts them with
                    // the original subject on the stack.
                    self.set_depth(failure_depth);
                }
            }
        }

        if has_default {
            let case = statement.cases.last().unwrap();
            let default_start = self.assembler.instruction_count();
            self.assembler
                .set_location(self.source_location(case.pattern.range()));
            self.emit(NOP, 0, 0)?;
            if let Some(guard) = &case.guard {
                if terminal && jump_constant_truthiness(guard) == Some(false) {
                    self.record_folded_value(guard)?;
                    self.assembler
                        .set_location(self.source_location(guard.range()));
                    let exit =
                        self.emit_jump_forward_instruction_preserving_location(JUMP_FORWARD, end)?;
                    self.assembler.preserve_inlined_jump_nop(exit);
                    // Keep the folded guard's original edge through return duplication so
                    // the copy inherits its span, rather than the wildcard NOP's span.
                    self.assembler.defer_jump_removal(exit);
                } else {
                    self.compile_match_guard(guard, &case.body, end)?;
                }
                failure_reaches_end = true;
            }
            let body_start = self.assembler.instruction_count();
            let emitted_fallthrough =
                if terminal && matches!(case.body.last(), Some(Stmt::If(_) | Stmt::Match(_))) {
                    let ((), emitted_fallthrough) = self.with_fallthrough_tracking(|compiler| {
                        compiler.compile_suite_inner(&case.body, true)
                    })?;
                    emitted_fallthrough
                } else {
                    self.compile_match_case_suite(&case.body, match_has_following)?;
                    false
                };
            if let Some(success_offset) = or_default_success_exit
                && case.guard.is_none()
                && matches!(case.body.as_slice(), [Stmt::Return(return_)]
                    if return_.value.as_deref().is_none_or(|value| matches!(value, Expr::NoneLiteral(_))))
                && self
                    .assembler
                    .pre_fusion_instruction_count_since(default_start)
                    <= 4
                && self
                    .assembler
                    .instruction_opcode_at(self.assembler.instruction_count() - 1)
                    == Some(RETURN_VALUE)
            {
                // The OR's zero-pop failure jump copies this small fallback before
                // the successful body. Its original array then becomes empty and
                // stops borrowing when the case-success jump is removed across it.
                self.assembler
                    .mark_borrow_empty_block_if_unretained_at(success_offset);
            }
            if (!terminal || self.context.python_version <= PythonVersion::Py313)
                && self.assembler.instruction_count() == body_start
                && let [Stmt::Pass(pass)] = case.body.as_slice()
                && case
                    .guard
                    .as_ref()
                    .is_none_or(|guard| early_condition_truthiness(guard) != Some(false))
            {
                // Protected suites usually transfer a trailing pass location to their exit.
                // The wildcard case falls through without a success jump, so retain its pass
                // instruction separately from the wildcard pattern's line event.
                self.assembler
                    .set_location(self.source_location(pass.range));
                self.emit(NOP, 0, 0)?;
            }
            if terminal && !suite_terminates(&case.body) && !emitted_fallthrough {
                self.set_branch_end_location(&case.body, body_start);
                if self.context.python_version <= crate::PythonVersion::Py313
                    || terminal_for_break_exit_inherits_predecessor(&case.body)
                {
                    self.assembler.set_location(SourceLocation::NONE);
                }
                // The wildcard body falls through to the original match-end block.
                // Keep its instructions outside the shared implicit-return copy budget.
                let implicit_return = self.assembler.label();
                if self.context.python_version <= crate::PythonVersion::Py313 {
                    self.assembler
                        .record_exception_owner_block_boundary(implicit_return);
                }
                self.assembler.mark(implicit_return);
                self.emit_deferred_implicit_return()?;
                failure_reaches_end = case.guard.is_some();
            }
            if !aliased_or_failure_jumps.is_empty()
                && case.guard.is_none()
                && self.assembler.has_large_linear_prefix_since(default_start)
            {
                // Fail-pop cleanup either shares this large wildcard array or
                // falls through to it. Neither target can be copied onto the
                // OR failure jump before its success array becomes empty.
                for failure in aliased_or_failure_jumps {
                    self.assembler.mark_borrow_empty_block_at(failure);
                }
            }
        }

        self.assembler.mark_original_match_join(end);
        self.set_depth(base_depth);
        if terminal && failure_reaches_end {
            self.assembler.set_location(SourceLocation::NONE);
            self.emit_deferred_implicit_return()?;
        }
        self.set_depth(base_depth);
        Ok(())
    }

    pub(super) fn compile_match_case_tail_if(
        &mut self,
        statement: &ruff_python_ast::StmtIf,
        match_end: Label,
        loop_tail: bool,
        final_case: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.stack.depth;
        let false_exit = self.assembler.label();
        // The no-else If's original End is its false edge, before the case exit.
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::IfEnd(statement.range),
            false_exit,
        );
        self.compile_jump_if(&statement.test, false, false_exit)?;
        self.mark_definitely_evaluated_locals(&statement.test);

        let body_start = self.assembler.instruction_count();
        if loop_tail
            && let Some((Stmt::If(nested), leading)) = statement.body.split_last()
            && nested.elif_else_clauses.is_empty()
            && early_condition_truthiness(&nested.test).is_none()
            && !suite_terminates(&nested.body)
        {
            self.compile_suite(leading)?;
            self.compile_match_case_tail_if(nested, match_end, true, final_case)?;
        } else {
            if loop_tail {
                self.compile_suite(&statement.body)?;
            } else {
                self.compile_match_case_tail_suite(&statement.body, false_exit)?;
            }
            self.set_branch_end_location(&statement.body, body_start);
            if !loop_tail {
                self.assembler
                    .record_comprehension_assignment_exit(false_exit);
            }
            if loop_tail {
                let original_unlocated_head = self.assembler.at_jump_target();
                let success_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, match_end)?;
                self.assembler.mark_jump_as_interruptible(success_exit);
                self.assembler
                    .inherit_nested_if_match_exit_locations(success_exit, original_unlocated_head);
            }
        }

        self.assembler.mark(false_exit);
        self.set_depth(base_depth);
        if loop_tail {
            self.assembler.set_location(
                self.source_location(synthetic_condition_exit_range(&statement.test)),
            );
            let false_case_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, match_end)?;
            self.assembler
                .prepare_no_location_block_for_inlining(false_case_exit);
            self.assembler.mark_jump_as_interruptible(false_case_exit);
        } else {
            // The body and false edge share CPython's case-exit block. Small return exits
            // replace its jump before conditional threading is considered.
            let terminating_body = suite_terminates(&statement.body);
            let unique_false_edge = terminating_body
                .then(|| unique_condition_jump_target(&statement.test, false))
                .flatten();
            // 3.12 copies the following small return before resolving lines.
            // Its original case-success jump is unlocated, so the replacement
            // NOP must disappear before a unique false edge can donate a span.
            self.assembler
                .set_location(if self.context.python_version == PythonVersion::Py312 {
                    SourceLocation::NONE
                } else {
                    unique_false_edge.map_or(SourceLocation::NONE, |test| {
                        self.source_location(test.range())
                    })
                });
            self.assembler.mark_conditional_threading_target(
                false_exit,
                self.source_location(synthetic_condition_exit_range(&statement.test)),
            );
            let original_unlocated_head = self.assembler.at_jump_target();
            let success_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, match_end)?;
            if final_case {
                // The final case's jump crosses an empty pattern-failure block. Delay its
                // removal until exit copying and retain its NOP after conditional threading.
                self.assembler
                    .preserve_redundant_jump_nop_after_threading(success_exit);
            }
            if unique_false_edge.is_some() && self.context.python_version != PythonVersion::Py313 {
                // Only the false edge reaches this original empty block. Preserve its
                // condition slot even when a containing match retargets the case exit
                // before a small return replaces the jump.
                self.assembler
                    .preserve_empty_join_inlined_jump_nop(success_exit);
            } else {
                self.assembler.mark_jump_as_interruptible(success_exit);
                self.assembler
                    .inherit_nested_if_match_exit_locations(success_exit, original_unlocated_head);
            }
        }
        Ok(())
    }

    // Share one original case-exit jump across nested fallthrough conditions. CPython
    // copies small exit blocks before threading conditions through that jump; adding
    // another original jump for each nested condition would duplicate those copies.
    fn compile_match_case_tail_suite(
        &mut self,
        body: &[Stmt],
        false_exit: Label,
    ) -> Result<(), CompileError> {
        if let Some((Stmt::If(nested), leading)) = body.split_last()
            && nested.elif_else_clauses.is_empty()
            && early_condition_truthiness(&nested.test).is_none()
            && !suite_terminates(&nested.body)
        {
            self.compile_suite(leading)?;
            self.compile_jump_if(&nested.test, false, false_exit)?;
            self.mark_definitely_evaluated_locals(&nested.test);
            self.compile_match_case_tail_suite(&nested.body, false_exit)
        } else {
            self.compile_suite(body)
        }
    }

    fn ensure_match_fail_pop(&mut self, context: &mut MatchContext, pops: usize) {
        while context.fail_pop.len() <= pops {
            let label = self.assembler.label();
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::MatchFailPop(
                    context.native_fail_owner,
                    context.fail_pop.len(),
                ),
                label,
            );
            context.fail_pop.push(label);
        }
    }

    fn jump_to_match_fail(
        &mut self,
        context: &mut MatchContext,
        opcode: Opcode,
        effect: i32,
    ) -> Result<(), CompileError> {
        self.jump_to_match_fail_with_marker_policy(
            context,
            opcode,
            effect,
            ExceptionPolicy::default(),
        )
    }

    fn jump_to_match_fail_with_marker_policy(
        &mut self,
        context: &mut MatchContext,
        opcode: Opcode,
        effect: i32,
        marker_policy: ExceptionPolicy,
    ) -> Result<(), CompileError> {
        if let Some(failure) = context.direct_loop_failure {
            debug_assert_eq!(opcode, POP_JUMP_IF_FALSE);
            debug_assert_eq!(context.on_top, 0);
            // CPython threads the final value-pattern failure through the loop exit, then
            // normalizes its backward conditional into an inverse jump and a fresh backedge.
            // Threading appends a new conditional; all three new instruction slots lack owners.
            let success = self.assembler.label();
            self.emit_jump_forward_with_exception_policy(
                POP_JUMP_IF_TRUE,
                success,
                effect,
                ExceptionPolicy::EXCLUDE_ALWAYS,
            )?;
            self.emit_with_exception_policy(NOT_TAKEN, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS)?;
            let backedge = self.emit_jump_forward_instruction_with_exception_policy(
                JUMP_FORWARD,
                failure,
                ExceptionPolicy::EXCLUDE_ALWAYS,
            )?;
            self.assembler
                .mark_normalized_conditional_backedge(backedge);
            self.assembler.mark(success);
            return Ok(());
        }
        let pops = context.on_top + context.stores.len();
        self.ensure_match_fail_pop(context, pops);
        self.emit_jump_forward(opcode, context.fail_pop[pops], effect)?;
        if matches!(
            opcode,
            POP_JUMP_IF_FALSE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE | POP_JUMP_IF_TRUE
        ) {
            // CPython appends forward-jump markers after exception owners have been labeled;
            // callers select the stale owner left by any earlier optimization.
            self.emit_with_exception_policy(NOT_TAKEN, 0, 0, marker_policy)?;
        }
        Ok(())
    }

    fn emit_match_fail_pop(
        &mut self,
        context: &MatchContext,
        location: SourceLocation,
        failure_depth: i32,
    ) -> Result<(), CompileError> {
        for pops in (1..context.fail_pop.len()).rev() {
            self.assembler.mark(context.fail_pop[pops]);
            self.set_depth(failure_depth + i32::try_from(pops).unwrap());
            self.assembler.set_location(location);
            self.emit(POP_TOP, 0, -1)?;
        }
        // Native 3.12 cleanup falls through to a separate zero-pop block.
        // Keep that original boundary even when no pattern jumps to it directly.
        if self.context.python_version == crate::PythonVersion::Py312 {
            self.assembler
                .record_exception_owner_block_boundary(context.fail_pop[0]);
        }
        self.assembler.mark(context.fail_pop[0]);
        self.set_depth(failure_depth);
        Ok(())
    }

    fn compile_stack_pattern(
        &mut self,
        pattern: &Pattern,
        context: &mut MatchContext,
    ) -> Result<(), CompileError> {
        self.assembler
            .set_location(self.source_location(pattern.range()));
        match pattern {
            Pattern::MatchValue(pattern) => {
                if let Some(constant) =
                    fold_constant_for_version(&pattern.value, self.context.python_version)
                {
                    self.emit_preprocessed_constant(&pattern.value, constant)?;
                } else {
                    self.compile_expression(&pattern.value)?;
                }
                self.assembler
                    .set_location(self.source_location(pattern.range));
                self.emit_comparison(ComparisonOperation::EQ, ComparisonMode::Boolean)?;
                self.jump_to_match_fail(context, POP_JUMP_IF_FALSE, -1)
            }
            Pattern::MatchSingleton(pattern) => {
                let constant = self.add_constant(match pattern.value {
                    Singleton::None => Constant::None,
                    Singleton::True => Constant::Bool(true),
                    Singleton::False => Constant::Bool(false),
                })?;
                // CPython folds this identity test inside protected suites as well. The
                // current exception policy tracks the removed instructions' marker owner.
                if matches!(pattern.value, Singleton::None) {
                    // CPython specializes the identity comparison and failure jump into a
                    // consuming None jump. Register the constant before the specialization so
                    // its pool ordering follows the same source traversal. The removed
                    // LOAD_CONST and IS_OP leave a protected slot for the late NOT_TAKEN marker.
                    // From 3.13, the constant pass folds these slots before swap optimization,
                    // which can reconstruct rotations into them with the None pattern's span.
                    // In 3.12, swaps run first in the same left-to-right pass, so the still-real
                    // identity test bounds the run and the capture's swap locations survive.
                    let preserve_swap_slots = self.context.python_version >= PythonVersion::Py313
                        && self.assembler.current_block_ends_with_multiple_swaps();
                    if preserve_swap_slots {
                        self.emit(NOP, 0, 0)?;
                        self.emit(NOP, 0, 0)?;
                    }
                    let condition_start = self.assembler.item_count();
                    let result = self.jump_to_match_fail_with_marker_policy(
                        context,
                        POP_JUMP_IF_NOT_NONE,
                        -1,
                        ExceptionPolicy::normalized_owner(true),
                    );
                    if !preserve_swap_slots {
                        self.assembler
                            .add_hidden_exception_owner_slots_at(condition_start, 2);
                    }
                    context.ownerless_markers_after_specialized_none = true;
                    result
                } else {
                    self.emit(LOAD_CONST, constant, 1)?;
                    self.emit(IS_OP, 0, -1)?;
                    let marker_policy = if context.ownerless_markers_after_specialized_none {
                        // Unlike value and structural patterns, a following boolean singleton
                        // does not leave another protected slot for its late marker.
                        ExceptionPolicy::normalized_owner(false)
                    } else if matches!(pattern.value, Singleton::True | Singleton::False) {
                        // Keep the authored singleton origin without choosing an owner here.
                        // The assembler must certify the original array and every later edit.
                        ExceptionPolicy::NoneUnchangedBooleanSingleton
                    } else {
                        ExceptionPolicy::default()
                    };
                    self.jump_to_match_fail_with_marker_policy(
                        context,
                        POP_JUMP_IF_FALSE,
                        -1,
                        marker_policy,
                    )
                }
            }
            Pattern::MatchAs(pattern) => {
                if let Some(inner) = &pattern.pattern {
                    if !is_wildcard_pattern(inner) {
                        context.on_top += 1;
                        self.emit(COPY, 1, 1)?;
                        self.compile_stack_pattern(inner, context)?;
                        context.on_top -= 1;
                    }
                }
                self.assembler
                    .set_location(self.source_location(pattern.range));
                self.capture_match_name(pattern.name.as_ref().map(Identifier::as_str), context)
            }
            Pattern::MatchStar(pattern) => {
                self.capture_match_name(pattern.name.as_ref().map(Identifier::as_str), context)
            }
            Pattern::MatchOr(pattern) => self.compile_stack_or_pattern(pattern, context),
            Pattern::MatchSequence(pattern) => {
                self.compile_stack_sequence_pattern(pattern, context)
            }
            Pattern::MatchMapping(pattern) => self.compile_stack_mapping_pattern(pattern, context),
            Pattern::MatchClass(pattern) => self.compile_stack_class_pattern(pattern, context),
        }
    }

    fn capture_match_name(
        &mut self,
        name: Option<&str>,
        context: &mut MatchContext,
    ) -> Result<(), CompileError> {
        let Some(name) = name else {
            return self.emit(POP_TOP, 0, -1);
        };
        let mut rotations = context.on_top + context.stores.len() + 1;
        while rotations > 1 {
            self.emit(SWAP, to_u32(rotations, "pattern capture rotation")?, 0)?;
            rotations -= 1;
        }
        context.stores.push(self.required_name_id(name));
        Ok(())
    }

    fn rotate_match_stack(&mut self, mut count: usize) -> Result<(), CompileError> {
        while count > 1 {
            self.emit(SWAP, to_u32(count, "pattern capture rotation")?, 0)?;
            count -= 1;
        }
        Ok(())
    }

    fn compile_stack_or_pattern(
        &mut self,
        pattern: &ruff_python_ast::PatternMatchOr,
        context: &mut MatchContext,
    ) -> Result<(), CompileError> {
        let entry_depth = self.control.stack.depth;
        let end = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::MatchOrEnd(pattern.range),
            end,
        );
        let mut control: Option<Vec<NameId>> = None;
        let mut empty_success = false;
        for (index, alternative) in pattern.patterns.iter().enumerate() {
            self.assembler
                .set_location(self.source_location(alternative.range()));
            self.emit(COPY, 1, 1)?;
            let mut alternative_context = MatchContext {
                stores: Vec::new(),
                fail_pop: Vec::new(),
                native_fail_owner: alternative.range(),
                direct_loop_failure: None,
                on_top: 0,
                ownerless_markers_after_specialized_none: context
                    .ownerless_markers_after_specialized_none,
                eager_final_mapping_capture: context.eager_final_mapping_capture,
                empty_failure_jumps: Vec::new(),
                empty_success_blocks: Vec::new(),
            };
            self.compile_stack_pattern(alternative, &mut alternative_context)?;
            context.ownerless_markers_after_specialized_none |=
                alternative_context.ownerless_markers_after_specialized_none;
            context
                .empty_failure_jumps
                .append(&mut alternative_context.empty_failure_jumps);
            context
                .empty_success_blocks
                .append(&mut alternative_context.empty_success_blocks);

            if index != 0 {
                let control = control.as_ref().unwrap();
                if alternative_context.stores.len() != control.len() {
                    return Err(CompileError::InvalidPattern {
                        message: "alternative patterns bind different names".to_string(),
                        range: alternative.range(),
                    });
                }
                for control_index in (0..control.len()).rev() {
                    let Some(store_index) = alternative_context
                        .stores
                        .iter()
                        .position(|name| name == &control[control_index])
                    else {
                        return Err(CompileError::InvalidPattern {
                            message: "alternative patterns bind different names".to_string(),
                            range: alternative.range(),
                        });
                    };
                    if control_index != store_index {
                        debug_assert!(store_index < control_index);
                        let rotations = store_index + 1;
                        let rotated: Vec<_> =
                            alternative_context.stores.drain(..rotations).collect();
                        alternative_context.stores.splice(
                            control_index - store_index..control_index - store_index,
                            rotated,
                        );
                        for _ in 0..rotations {
                            self.rotate_match_stack(control_index + 1)?;
                        }
                    }
                }
            }

            self.assembler
                .set_location(self.source_location(alternative.range()));
            let success_starts_block = self
                .assembler
                .instruction_opcode_at(self.assembler.instruction_count() - 1)
                == Some(NOT_TAKEN);
            let success = self.emit_jump_forward_instruction(JUMP_FORWARD, end)?;
            if index == 0
                && pattern.patterns.len() == 2
                && pattern.patterns.last().is_some_and(|last| {
                    is_wildcard_pattern(last)
                        && self.source_location(last.range()).line
                            == self.source_location(alternative.range()).line
                })
                && matches!(
                    alternative,
                    Pattern::MatchValue(_) | Pattern::MatchSingleton(_)
                )
                && success_starts_block
            {
                // The simple alternative's success jump and the wildcard's COPY/POP_TOP
                // block disappear. A structural pattern's cleanup or a separately located
                // wildcard can instead leave a nonempty predecessor of the shared body.
                empty_success = true;
            }
            if success_starts_block
                && self.source_location(alternative.range()).line
                    != self.source_location(pattern.range).line
            {
                // NOT_TAKEN is inserted after CPython removes redundant NOPs. At that
                // point this success edge starts its block, so its alternative's line
                // survives small-exit duplication before the shared pattern cleanup.
                self.assembler.preserve_inlined_jump_nop(success);
            }
            if alternative_context.fail_pop.is_empty() {
                self.set_depth(entry_depth);
            } else {
                self.emit_match_fail_pop(
                    &alternative_context,
                    self.source_location(alternative.range()),
                    entry_depth,
                )?;
            }
            if index == 0 {
                control = Some(alternative_context.stores);
            }
        }
        self.assembler
            .set_location(self.source_location(pattern.range));
        self.emit(POP_TOP, 0, -1)?;
        let pops = context.on_top + context.stores.len();
        self.ensure_match_fail_pop(context, pops);
        self.emit_jump_forward_instruction(JUMP_FORWARD, context.fail_pop[pops])?;
        // Outer failure pops also allow the success array to copy away, but
        // their failure target must remain too large to copy into this jump.
        let success_start = self.assembler.instruction_count();
        context.empty_failure_jumps.push((
            success_start - 1,
            success_start,
            (pops != 0).then_some(context.fail_pop[pops]),
        ));
        if empty_success {
            context
                .empty_success_blocks
                .push((end, self.assembler.instruction_count()));
        }
        self.assembler.mark(end);
        let control = control.unwrap();
        self.set_depth(entry_depth + i32::try_from(control.len()).unwrap());
        self.assembler
            .set_location(self.source_location(pattern.range));
        let rotations = control.len() + 1 + context.on_top + context.stores.len();
        for name in control {
            self.rotate_match_stack(rotations)?;
            if context.stores.contains(&name) {
                return Err(CompileError::InvalidPattern {
                    message: format!(
                        "multiple assignments to name '{}' in pattern",
                        self.resolve_name(name)
                    ),
                    range: pattern.range,
                });
            }
            context.stores.push(name);
        }
        self.emit(POP_TOP, 0, -1)
    }

    fn compile_stack_sequence_pattern(
        &mut self,
        pattern: &ruff_python_ast::PatternMatchSequence,
        context: &mut MatchContext,
    ) -> Result<(), CompileError> {
        let starred = pattern
            .patterns
            .iter()
            .position(|pattern| matches!(pattern, Pattern::MatchStar(_)));
        let minimum = pattern.patterns.len() - usize::from(starred.is_some());
        let only_wildcards = pattern.patterns.iter().all(|pattern| {
            matches!(pattern, Pattern::MatchAs(pattern) if pattern.pattern.is_none() && pattern.name.is_none())
                || matches!(pattern, Pattern::MatchStar(pattern) if pattern.name.is_none())
        });
        context.on_top += 1;
        self.emit(MATCH_SEQUENCE, 0, 1)?;
        // These structural tests precede CFG optimization and insert no Boolean conversion.
        // Resolve their appended markers from staged vacancies, including a fresh NULL slot.
        self.jump_to_match_fail_with_marker_policy(
            context,
            POP_JUMP_IF_FALSE,
            -1,
            ExceptionPolicy::NoneDynamicConditionStrict,
        )?;
        if starred.is_none() || pattern.patterns.len() > 1 {
            self.emit(GET_LEN, 0, 1)?;
            if self.output.constants.is_empty() {
                self.add_constant(Constant::Int(u64::try_from(minimum).unwrap_or(u64::MAX)))?;
            }
            self.emit_unsigned_constant(to_u32(minimum, "sequence pattern length")?)?;
            self.emit_comparison(
                if starred.is_some() {
                    ComparisonOperation::GE
                } else {
                    ComparisonOperation::EQ
                },
                ComparisonMode::Value,
            )?;
            self.jump_to_match_fail_with_marker_policy(
                context,
                POP_JUMP_IF_FALSE,
                -1,
                ExceptionPolicy::NoneDynamicConditionStrict,
            )?;
        }
        context.on_top -= 1;
        if only_wildcards {
            return self.emit(POP_TOP, 0, -1);
        }
        if let Some(starred) = starred
            && matches!(&pattern.patterns[starred], Pattern::MatchStar(pattern) if pattern.name.is_none())
        {
            context.on_top += 1;
            for (index, subpattern) in pattern.patterns.iter().enumerate() {
                if index == starred || is_wildcard_pattern(subpattern) {
                    continue;
                }
                self.assembler
                    .set_location(self.source_location(pattern.range));
                self.emit(COPY, 1, 1)?;
                if index < starred {
                    self.emit_unsigned_constant(to_u32(index, "sequence pattern index")?)?;
                } else {
                    self.emit(GET_LEN, 0, 1)?;
                    self.emit_unsigned_constant(to_u32(
                        pattern.patterns.len() - index,
                        "sequence pattern suffix index",
                    )?)?;
                    self.emit_binary_operation(binary_operator(Operator::Sub, false))?;
                }
                self.emit_binary_operation(BinaryOperation::SUBSCR)?;
                self.compile_stack_pattern(subpattern, context)?;
            }
            context.on_top -= 1;
            self.assembler
                .set_location(self.source_location(pattern.range));
            return self.emit(POP_TOP, 0, -1);
        }
        if let Some(starred) = starred {
            let before = to_u32(starred, "sequence pattern prefix length")?;
            let after = to_u32(
                pattern.patterns.len() - starred - 1,
                "sequence pattern suffix length",
            )?;
            self.emit(
                UNPACK_EX,
                before | (after << 8),
                i32::try_from(pattern.patterns.len()).unwrap() - 1,
            )?;
        } else {
            self.emit(
                UNPACK_SEQUENCE,
                to_u32(pattern.patterns.len(), "sequence pattern length")?,
                i32::try_from(pattern.patterns.len()).unwrap() - 1,
            )?;
        }
        context.on_top += pattern.patterns.len();
        for subpattern in &pattern.patterns {
            context.on_top -= 1;
            self.compile_stack_pattern(subpattern, context)?;
        }
        Ok(())
    }

    fn compile_stack_mapping_pattern(
        &mut self,
        pattern: &ruff_python_ast::PatternMatchMapping,
        context: &mut MatchContext,
    ) -> Result<(), CompileError> {
        context.on_top += 1;
        self.emit(MATCH_MAPPING, 0, 1)?;
        self.jump_to_match_fail_with_marker_policy(
            context,
            POP_JUMP_IF_FALSE,
            -1,
            ExceptionPolicy::NoneDynamicConditionStrict,
        )?;
        if pattern.keys.is_empty() && pattern.rest.is_none() {
            context.on_top -= 1;
            return self.emit(POP_TOP, 0, -1);
        }
        if !pattern.keys.is_empty() {
            self.emit(GET_LEN, 0, 1)?;
            self.add_constant(Constant::Int(
                u64::try_from(pattern.keys.len()).unwrap_or(u64::MAX),
            ))?;
            self.emit_unsigned_constant(to_u32(pattern.keys.len(), "mapping pattern key count")?)?;
            self.emit_comparison(ComparisonOperation::GE, ComparisonMode::Value)?;
            self.jump_to_match_fail_with_marker_policy(
                context,
                POP_JUMP_IF_FALSE,
                -1,
                ExceptionPolicy::NoneDynamicConditionStrict,
            )?;
        }
        // Legacy CFG folding retains each key's source line as a NOP before
        // building the tuple. Keep those loads until the target's folding pass.
        if self.context.python_version >= crate::PythonVersion::Py314
            && pattern.keys.len() <= 30
            && let Some(keys) = pattern
                .keys
                .iter()
                .map(|value| fold_constant_for_version(value, self.context.python_version))
                .collect::<Option<Vec<_>>>()
        {
            // Pattern literals are folded by AST optimization before code generation.
            // Reserve each scalar before the flowgraph builds their keys tuple.
            for key in &keys {
                self.add_constant(key.clone())?;
            }
            self.emit_deferred_constant(Constant::Tuple(keys))?;
        } else {
            for key in &pattern.keys {
                self.compile_expression(key)?;
            }
            self.emit_build(BUILD_TUPLE, pattern.keys.len())?;
        }
        self.emit(MATCH_KEYS, 0, 1)?;
        context.on_top += 2;
        self.emit(COPY, 1, 1)?;
        self.add_constant(Constant::None)?;
        self.jump_to_match_fail(context, POP_JUMP_IF_NONE, -1)?;
        self.emit(
            UNPACK_SEQUENCE,
            to_u32(pattern.patterns.len(), "mapping pattern value count")?,
            i32::try_from(pattern.patterns.len()).unwrap() - 1,
        )?;
        context.on_top += pattern.patterns.len();
        context.on_top -= 1;
        for (index, subpattern) in pattern.patterns.iter().enumerate() {
            context.on_top -= 1;
            if pattern.rest.is_none()
                && pattern.keys.len() == 1
                && index + 1 == pattern.patterns.len()
                && let Pattern::MatchAs(capture) = subpattern
                && let Some(name) = &capture.name
                && capture.pattern.is_none()
                && context.eager_final_mapping_capture
            {
                self.assembler
                    .set_location(self.source_location(pattern.range));
                self.store_name(name.as_str())?;
            } else if pattern.rest.is_none()
                && pattern.keys.len() == 1
                && context.stores.is_empty()
                && index + 1 == pattern.patterns.len()
                && let Pattern::MatchAs(capture) = subpattern
                && let Some(name) = &capture.name
                && let Some(inner @ Pattern::MatchClass(class)) = capture.pattern.as_deref()
                && class.arguments.patterns.is_empty()
                && class.arguments.keywords.is_empty()
            {
                self.assembler
                    .set_location(self.source_location(capture.range));
                context.on_top += 1;
                self.emit(COPY, 1, 1)?;
                self.compile_stack_pattern(inner, context)?;
                context.on_top -= 1;
                // With no pending captures, static swaps move this store ahead of the
                // mapping cleanup. Otherwise keep the original rotation and deferred stores:
                // their line boundaries can prevent the static-swap pass from moving it.
                self.assembler
                    .set_location(self.source_location(pattern.range));
                self.store_name(name.as_str())?;
            } else {
                self.compile_stack_pattern(subpattern, context)?;
            }
        }
        context.on_top -= 2;
        if let Some(rest) = &pattern.rest {
            self.assembler
                .set_location(self.source_location(pattern.range));
            self.emit(BUILD_MAP, 0, 1)?;
            self.emit(SWAP, 3, 0)?;
            self.emit(DICT_UPDATE, 2, -1)?;
            self.emit(
                UNPACK_SEQUENCE,
                to_u32(pattern.keys.len(), "mapping pattern key count")?,
                i32::try_from(pattern.keys.len()).unwrap() - 1,
            )?;
            let mut size = pattern.keys.len();
            while size > 0 {
                self.emit(COPY, to_u32(1 + size, "mapping rest copy depth")?, 1)?;
                self.emit(SWAP, 2, 0)?;
                self.emit(DELETE_SUBSCR, 0, -2)?;
                size -= 1;
            }
            self.capture_match_name(Some(rest.as_str()), context)
        } else {
            self.assembler
                .set_location(self.source_location(pattern.range));
            self.emit(POP_TOP, 0, -1)?;
            self.emit(POP_TOP, 0, -1)
        }
    }

    fn compile_stack_class_pattern(
        &mut self,
        pattern: &ruff_python_ast::PatternMatchClass,
        context: &mut MatchContext,
    ) -> Result<(), CompileError> {
        self.compile_expression(&pattern.cls)?;
        let keyword_names = pattern
            .arguments
            .keywords
            .iter()
            .map(|keyword| Constant::String(keyword.attr.as_str().to_string()))
            .collect();
        let keyword_names = self.add_interned_string_tuple(keyword_names)?;
        self.emit(LOAD_CONST, keyword_names, 1)?;
        self.emit(
            MATCH_CLASS,
            to_u32(
                pattern.arguments.patterns.len(),
                "class positional pattern count",
            )?,
            -2,
        )?;
        self.emit(COPY, 1, 1)?;
        self.add_constant(Constant::None)?;
        context.on_top += 1;
        // The specialized failure jump removes LOAD_CONST(None) and IS_OP before
        // normalized markers reuse the original instruction-array slots.
        self.assembler.add_last_hidden_exception_owner_slots(2);
        self.jump_to_match_fail(context, POP_JUMP_IF_NONE, -1)?;
        let subpattern_count = pattern.arguments.patterns.len() + pattern.arguments.keywords.len();
        self.emit(
            UNPACK_SEQUENCE,
            to_u32(subpattern_count, "class pattern count")?,
            i32::try_from(subpattern_count).unwrap() - 1,
        )?;
        context.on_top += subpattern_count;
        context.on_top -= 1;
        for subpattern in &pattern.arguments.patterns {
            context.on_top -= 1;
            if is_wildcard_pattern(subpattern) {
                self.assembler
                    .set_location(self.source_location(pattern.range));
                self.emit(POP_TOP, 0, -1)?;
            } else {
                self.compile_stack_pattern(subpattern, context)?;
            }
        }
        for keyword in &pattern.arguments.keywords {
            context.on_top -= 1;
            if is_wildcard_pattern(&keyword.pattern) {
                self.assembler
                    .set_location(self.source_location(pattern.range));
                self.emit(POP_TOP, 0, -1)?;
            } else {
                self.compile_stack_pattern(&keyword.pattern, context)?;
            }
        }
        Ok(())
    }

    /// Return the expression span after the target's comparison-negation AST fold.
    pub(super) fn expression_location(&self, expression: &Expr) -> SourceLocation {
        if self.context.python_version <= PythonVersion::Py313
            && let Some((comparison, _)) = legacy_not_comparison(expression)
        {
            return self.source_location(comparison.range);
        }
        self.source_location(expression.range())
    }

    pub(super) fn compile_expression(&mut self, expression: &Expr) -> Result<(), CompileError> {
        self.compile_expression_with_boolean_continuation(expression, None)
    }

    /// Preserve the location of the Boolean test consuming this expression's value.
    fn compile_expression_with_boolean_continuation(
        &mut self,
        expression: &Expr,
        continuation: Option<(BoolOp, SourceLocation)>,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313
            && let Some(comparison) = legacy_folded_not_comparison(expression)
        {
            return self.compile_expression_with_boolean_continuation(&comparison, continuation);
        }
        let previous = self.assembler.location();
        self.assembler
            .set_location(self.source_location(expression.range()));
        let result = self.compile_expression_inner(expression, continuation);
        self.assembler.set_location(previous);
        result
    }

    fn compile_expression_inner(
        &mut self,
        expression: &Expr,
        continuation: Option<(BoolOp, SourceLocation)>,
    ) -> Result<(), CompileError> {
        let starting_depth = self.control.stack.depth;
        if self.context.python_version >= PythonVersion::Py314
            && let Expr::BoolOp(boolean) = expression
            && let Some((origin, _)) = folded_bool_operand(expression, self.context.python_version)
            && let Some(constant) = self.source_aware_fold_constant(origin)
        {
            self.pre_register_expression_names(expression)?;
            let expression_location = self.source_location(expression.range());
            let origin_location = self.source_location(origin.range());
            if expression_location.line != origin_location.line {
                self.assembler.set_location(expression_location);
                self.emit(NOP, 0, 0)?;
            }
            self.assembler.set_location(origin_location);
            self.emit_folded_constant(origin, constant)?;
            if boolean.values.len() > 1
                && boolean
                    .values
                    .first()
                    .is_some_and(|first| first.range() == origin.range())
            {
                // A decisive first operand leaves an empty original block before
                // the join. Keep pending call inputs in the preceding borrow block,
                // but allow independent outer Boolean jumps to reach the join.
                let discarded = self.assembler.label();
                self.assembler.mark(discarded);
                self.assembler.prevent_borrow_reachability(discarded);
                let end = self.assembler.label();
                self.assembler.preserve_block_boundary(end);
                self.assembler.mark(end);
            }
            return Ok(());
        }
        if (matches!(
            expression,
            Expr::BinOp(_) | Expr::Subscript(_) | Expr::UnaryOp(_) | Expr::Tuple(_)
        ) || matches!(
            expression,
            Expr::Name(name)
                if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__"
        )) && let Some(constant) = self.source_aware_fold_constant(expression)
        {
            self.emit_folded_constant(expression, constant)?;
            return Ok(());
        }
        match expression {
            Expr::Name(name) if name.ctx == ExprContext::Load => {
                self.load_name(name.id.as_str())?;
            }
            Expr::NoneLiteral(_) => {
                let index = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, index, 1)?;
            }
            Expr::BooleanLiteral(boolean) => {
                let index = self.add_constant(Constant::Bool(boolean.value))?;
                self.emit(LOAD_CONST, index, 1)?;
            }
            Expr::EllipsisLiteral(_) => {
                let index = self.add_constant(Constant::Ellipsis)?;
                self.emit(LOAD_CONST, index, 1)?;
            }
            Expr::NumberLiteral(number) => {
                let constant = number_constant(&number.value);
                if self.context.python_version >= PythonVersion::Py314
                    && let Number::Int(value) = &number.value
                    && let Some(value) = value.as_u8()
                {
                    if self.output.constants.is_empty() {
                        self.add_constant(constant)?;
                    }
                    self.emit(LOAD_SMALL_INT, u32::from(value), 1)?;
                } else {
                    let index = self.add_constant(constant)?;
                    self.emit(LOAD_CONST, index, 1)?;
                }
            }
            Expr::StringLiteral(string) => {
                let index = self.add_constant(self.string_literal_constant(string))?;
                self.emit(LOAD_CONST, index, 1)?;
            }
            Expr::BytesLiteral(bytes) => {
                let index = self.add_constant(Constant::Bytes(bytes.value.bytes().collect()))?;
                self.emit(LOAD_CONST, index, 1)?;
            }
            Expr::BinOp(binary) => {
                if self.compile_optimized_percent_format(binary)?
                    == SpecializedLowering::NotApplicable
                {
                    self.compile_expression(&binary.left)?;
                    let exclude_optimized_tuple_guard_from_comprehension = binary.op
                        == Operator::Add
                        && matches!(binary.left.as_ref(), Expr::Name(_))
                        && matches!(binary.right.as_ref(), Expr::Call(call)
                                if matches!(optimized_generator_callable_for_target(call, self.context.python_version), Some("tuple" | "list" | "set")));
                    self.with_control_override(
                        |control| {
                            &mut control
                                .comprehensions
                                .exclude_optimized_tuple_guard_from_comprehension
                        },
                        exclude_optimized_tuple_guard_from_comprehension,
                        |compiler| compiler.compile_expression(&binary.right),
                    )?;
                    self.emit_binary_operation(binary_operator(binary.op, false))?;
                }
            }
            Expr::UnaryOp(unary) => {
                self.compile_expression(&unary.operand)?;
                match unary.op {
                    UnaryOp::Invert => self.emit(UNARY_INVERT, 0, 0)?,
                    UnaryOp::Not => {
                        self.emit(TO_BOOL, 0, 0)?;
                        self.emit(UNARY_NOT, 0, 0)?;
                    }
                    UnaryOp::UAdd => self.emit_unary_intrinsic(UnaryIntrinsic::UNARY_POSITIVE)?,
                    UnaryOp::USub => self.emit(UNARY_NEGATIVE, 0, 0)?,
                }
            }
            Expr::Lambda(lambda) => self.compile_lambda(lambda)?,
            Expr::Call(call) => {
                self.compile_call(call)?;
            }
            Expr::Attribute(attribute) if attribute.ctx == ExprContext::Load => {
                if self.compile_super_attribute(attribute, SuperAttributeKind::Attribute)?
                    == SpecializedLowering::NotApplicable
                {
                    self.compile_expression(&attribute.value)?;
                    let index = self.name_index(attribute.attr.as_str())?;
                    self.assembler
                        .set_location(self.attribute_opcode_location(attribute));
                    self.emit(LOAD_ATTR, index << 1, 0)?;
                }
            }
            Expr::List(list) if list.ctx == ExprContext::Load => {
                self.compile_collection(&list.elts, CollectionKind::List)?;
            }
            Expr::Tuple(tuple) if tuple.ctx == ExprContext::Load => {
                self.compile_collection(&tuple.elts, CollectionKind::Tuple)?;
            }
            Expr::Set(set) => {
                self.compile_collection(&set.elts, CollectionKind::Set)?;
            }
            Expr::Dict(dict) => {
                self.compile_dict(&dict.items)?;
            }
            Expr::Subscript(subscript) if subscript.ctx == ExprContext::Load => {
                self.compile_expression(&subscript.value)?;
                if let Expr::Slice(slice) = subscript.slice.as_ref() {
                    let slice_location = self.source_location(slice.range);
                    let subscript_location = self.source_location(subscript.range);
                    if self.context.python_version >= PythonVersion::Py314
                        && let Some(constant) = self.source_aware_constant_slice(slice)
                    {
                        let index = self.add_constant(constant)?;
                        self.assembler.set_location(slice_location);
                        self.emit(LOAD_CONST, index, 1)?;
                        self.assembler.set_location(subscript_location);
                        self.emit_binary_operation(BinaryOperation::SUBSCR)?;
                    } else if slice.step.is_none() {
                        self.compile_optional_slice_bound(slice.lower.as_deref(), slice_location)?;
                        self.compile_optional_slice_bound(slice.upper.as_deref(), slice_location)?;
                        self.assembler.set_location(subscript_location);
                        self.emit(BINARY_SLICE, 0, -2)?;
                    } else {
                        self.compile_expression(&subscript.slice)?;
                        self.emit_binary_operation(BinaryOperation::SUBSCR)?;
                    }
                } else {
                    self.compile_expression(&subscript.slice)?;
                    self.emit_binary_operation(BinaryOperation::SUBSCR)?;
                }
            }
            Expr::If(expression) => self.compile_if_expression(expression, continuation)?,
            Expr::BoolOp(expression) => self.compile_bool_expression(expression, continuation)?,
            Expr::Compare(expression) => self.compile_compare_expression(expression)?,
            Expr::Named(expression) => {
                self.compile_expression(&expression.value)?;
                self.emit(COPY, 1, 1)?;
                self.compile_store_target(&expression.target)?;
            }
            Expr::Slice(slice) => self.compile_slice(slice)?,
            Expr::FString(fstring) => self.compile_fstring(fstring)?,
            Expr::ListComp(comprehension) => self.compile_comprehension(
                &comprehension.generators,
                None,
                &comprehension.elt,
                BUILD_LIST,
                LIST_APPEND,
                false,
            )?,
            Expr::SetComp(comprehension) => self.compile_comprehension(
                &comprehension.generators,
                None,
                &comprehension.elt,
                BUILD_SET,
                SET_ADD,
                false,
            )?,
            Expr::DictComp(comprehension) => self.compile_comprehension(
                &comprehension.generators,
                comprehension.key.as_deref(),
                &comprehension.value,
                BUILD_MAP,
                MAP_ADD,
                false,
            )?,
            Expr::Generator(generator) => self.compile_generator_expression(generator)?,
            Expr::Yield(expression) => {
                if self.unit.flags & (CO_GENERATOR | CO_ASYNC_GENERATOR) == 0 {
                    return Err(CompileError::invalid_syntax(
                        "'yield' outside function",
                        expression.range,
                    ));
                }
                if let Some(value) = &expression.value {
                    self.compile_expression(value)?;
                } else {
                    let none = self.add_constant(Constant::None)?;
                    self.emit(LOAD_CONST, none, 1)?;
                }
                self.assembler
                    .set_location(self.source_location(expression.range));
                if self.unit.flags & CO_ASYNC_GENERATOR != 0 {
                    self.emit_unary_intrinsic(UnaryIntrinsic::ASYNC_GEN_WRAP)?;
                }
                self.emit_yield_value(0, 0)?;
                let wrapper_is_only_exception_region =
                    self.control.generators.generator_region_start.is_some()
                        && self.control.suspension_exception_depth() == 0;
                let resume_depth = if wrapper_is_only_exception_region {
                    ResumeDepth::One
                } else {
                    ResumeDepth::Zero
                };
                self.emit_resume(ResumeLocation::AFTER_YIELD, resume_depth)?;
            }
            Expr::YieldFrom(expression) => {
                if self.unit.flags & CO_GENERATOR == 0 {
                    let message = if self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR) != 0 {
                        "'yield from' inside async function"
                    } else {
                        "'yield from' outside function"
                    };
                    return Err(CompileError::invalid_syntax(message, expression.range));
                }
                if self.context.python_version == PythonVersion::Py315 {
                    self.compile_iterable_expression(&expression.value)?;
                } else {
                    self.compile_expression(&expression.value)?;
                }
                self.emit(GET_YIELD_FROM_ITER, 0, self.iterator_extra_slots())?;
                let none = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, none, 1)?;
                let send = self.assembler.label();
                let end = self.assembler.label();
                let yielded = self.assembler.label();
                let yielded_end = self.assembler.label();
                let cleanup = self.assembler.label();
                self.bind_native_source_label(
                    super::legacy_native_labels::NativeRole::YieldFromSend(expression.range),
                    send,
                );
                self.bind_native_source_label(
                    super::legacy_native_labels::NativeRole::YieldFromFail(expression.range),
                    cleanup,
                );
                self.bind_native_source_label(
                    super::legacy_native_labels::NativeRole::YieldFromExit(expression.range),
                    end,
                );
                self.assembler.mark(send);
                self.emit_jump_forward(SEND, end, 0)?;
                self.assembler.record_exception_setup(yielded, false);
                self.assembler.mark(yielded);
                self.emit_yield_value(1, 0)?;
                self.assembler.mark(yielded_end);
                let resume_depth = if self.control.generators.generator_region_start.is_none() {
                    ResumeDepth::One
                } else {
                    ResumeDepth::Zero
                };
                self.emit_resume(ResumeLocation::AFTER_YIELD_FROM, resume_depth)?;
                self.emit_jump_backward(JUMP_BACKWARD_NO_INTERRUPT, send, 0)?;
                self.assembler.mark(cleanup);
                self.assembler.add_exception_region(
                    yielded,
                    yielded_end,
                    cleanup,
                    (starting_depth + 2 + self.iterator_extra_slots()).cast_unsigned(),
                    false,
                );
                self.assembler
                    .set_location(self.source_location(expression.range));
                self.set_depth(starting_depth + 3 + self.iterator_extra_slots());
                self.emit(CLEANUP_THROW, 0, -1)?;
                self.assembler.mark(end);
                self.set_depth(starting_depth + 2 + self.iterator_extra_slots());
                self.emit(END_SEND, 0, -self.iterator_slots())?;
            }
            Expr::Await(expression) => self.compile_await(expression)?,
            Expr::TString(tstring) => self.compile_tstring(tstring)?,
            Expr::Starred(starred) => {
                self.compile_expression(&starred.value)?;
                self.emit(UNPACK_SEQUENCE, 1, 0)?;
            }
            Expr::IpyEscapeCommand(_)
            | Expr::List(_)
            | Expr::Name(_)
            | Expr::Subscript(_)
            | Expr::Tuple(_)
            | Expr::Attribute(_) => return Err(unsupported(expression_name(expression))),
        }

        let expected_depth = starting_depth
            + 1
            + if self
                .control
                .comprehensions
                .defer_async_comprehension_restore
            {
                i32::try_from(
                    self.control
                        .comprehensions
                        .pending_comprehension_restores
                        .iter()
                        .map(|(indices, _)| indices.len())
                        .sum::<usize>(),
                )
                .unwrap()
            } else {
                0
            };
        if self.control.stack.depth != expected_depth {
            return Err(CompileError::Internal(format!(
                "expression changed stack depth from {starting_depth} to {}",
                self.control.stack.depth
            )));
        }
        Ok(())
    }

    fn compile_fstring(
        &mut self,
        fstring: &ruff_python_ast::ExprFString,
    ) -> Result<(), CompileError> {
        let mut components = fstring_components(fstring, self.context.source);
        let value_count = components.len();
        if let [InterpolatedStringComponent::Literal(literal)] = components.as_mut_slice() {
            literal.range = self.fstring_result_range(fstring);
        }
        let large = value_count > 30;
        let outer_location = self.source_location(fstring.range);
        if large {
            self.assembler.set_location(outer_location);
            self.load_string_constant("")?;
            let join = self.name_index("join")?;
            self.emit(LOAD_ATTR, (join << 1) | 1, 1)?;
            self.emit(BUILD_LIST, 0, 1)?;
        }
        for component in components {
            match component {
                InterpolatedStringComponent::Literal(literal) => literal.load(self)?,
                InterpolatedStringComponent::Interpolation(interpolation, is_raw) => {
                    self.compile_interpolation(interpolation, is_raw)?;
                }
            }
            if large {
                self.assembler.set_location(outer_location);
                self.emit(LIST_APPEND, 1, -1)?;
            }
        }
        self.assembler.set_location(outer_location);
        if large {
            self.emit(CALL, 1, -2)
        } else {
            self.finish_interpolated_string(value_count, "f-string component count")
        }
    }

    fn compile_tstring(
        &mut self,
        tstring: &ruff_python_ast::ExprTString,
    ) -> Result<(), CompileError> {
        let mut strings = vec![None];
        let mut interpolations = Vec::new();
        for part in &tstring.value {
            for element in &part.elements {
                match element {
                    InterpolatedStringElement::Literal(literal) => {
                        PendingInterpolatedLiteral::append(
                            strings.last_mut().unwrap(),
                            self.context.source,
                            &literal.value,
                            literal.range,
                            literal.range,
                            part.flags.prefix().is_raw(),
                        );
                    }
                    InterpolatedStringElement::Interpolation(interpolation) => {
                        if let Some(debug) = &interpolation.debug_text {
                            PendingInterpolatedLiteral::append(
                                strings.last_mut().unwrap(),
                                self.context.source,
                                &strip_expression_comments(debug.as_str()),
                                debug_text_range(interpolation),
                                debug_text_range(interpolation),
                                true,
                            );
                        }
                        interpolations.push((interpolation, part.flags.prefix().is_raw()));
                        strings.push(None);
                    }
                }
            }
        }

        let strings = strings
            .into_iter()
            .map(|literal| {
                literal.map_or_else(
                    || Constant::String(String::new()),
                    PendingInterpolatedLiteral::into_constant,
                )
            })
            .collect::<Vec<_>>();
        for string in &strings {
            self.add_constant(string.clone())?;
        }
        self.emit_deferred_constant(Constant::Tuple(strings))?;
        for (interpolation, is_raw) in &interpolations {
            self.compile_expression(&interpolation.expression)?;
            self.assembler
                .set_location(self.source_location(interpolation.range));
            let expression_text = self.interpolation_expression_text(interpolation);
            self.load_string_constant(&expression_text)?;

            let conversion = if interpolation.debug_text.is_some()
                && interpolation.format_spec.is_none()
                && interpolation.conversion == ConversionFlag::None
            {
                ConversionFlag::Repr
            } else {
                interpolation.conversion
            };
            let mut argument = 2 | match conversion {
                ConversionFlag::None => 0,
                ConversionFlag::Str => 4,
                ConversionFlag::Repr => 8,
                ConversionFlag::Ascii => 12,
            };
            let effect = if let Some(format_spec) = &interpolation.format_spec {
                self.compile_interpolated_elements(
                    &format_spec.elements,
                    format_spec.range,
                    *is_raw,
                )?;
                argument |= 1;
                -2
            } else {
                -1
            };
            self.assembler
                .set_location(self.source_location(interpolation.range));
            self.emit(BUILD_INTERPOLATION, argument, effect)?;
        }
        self.assembler
            .set_location(self.source_location(tstring.range));
        if interpolations.is_empty() {
            self.emit_deferred_constant(Constant::Tuple(Vec::new()))?;
        } else {
            self.emit_build(BUILD_TUPLE, interpolations.len())?;
        }
        self.emit(BUILD_TEMPLATE, 0, -1)
    }

    fn interpolation_expression_text(
        &self,
        interpolation: &ruff_python_ast::InterpolatedElement,
    ) -> String {
        let start = usize::from(interpolation.range.start()) + 1;
        let expression_end = usize::from(interpolation.expression.range().end());
        // CPython truncates the stored expression text at a top-level `!=`
        // when the interpolation has a format specifier.
        if interpolation.format_spec.is_some()
            && interpolation.debug_text.is_none()
            && interpolation.conversion == ConversionFlag::None
            && let Expr::Compare(comparison) = interpolation.expression.as_ref()
            && comparison.ops.contains(&CmpOp::NotEq)
            && let Some(offset) = self.context.source[start..expression_end].find("!=")
        {
            return strip_expression_comments(
                self.context.source[start..start + offset].trim_end(),
            );
        }
        if interpolation.debug_text.is_some() {
            let range = self.debug_expression_source_range(interpolation);
            let source = &self.context.source[usize::from(range.start())..usize::from(range.end())];
            return strip_expression_comments(source).trim_end().to_string();
        }
        let interpolation_end = usize::from(interpolation.range.end()).saturating_sub(1);
        let trailing = &self.context.source[expression_end..interpolation_end];
        let delimiter = if interpolation.conversion != ConversionFlag::None {
            trailing.find('!')
        } else if interpolation.format_spec.is_some() {
            trailing.find(':')
        } else {
            None
        };
        let end = delimiter.map_or(interpolation_end, |offset| expression_end + offset);
        strip_expression_comments(&self.context.source[start..end])
            .trim_end()
            .to_string()
    }

    fn compile_interpolated_elements(
        &mut self,
        elements: &ruff_python_ast::InterpolatedStringElements,
        range: ruff_text_size::TextRange,
        is_raw: bool,
    ) -> Result<(), CompileError> {
        let components = format_spec_components(elements, is_raw, self.context.source);
        let value_count = components.len();
        for component in components {
            match component {
                InterpolatedStringComponent::Literal(mut literal) => {
                    // Python 3.12 retains adjacent empty format-spec tokens; later
                    // parsers discard them before merging the nonempty constants.
                    if self.context.python_version >= PythonVersion::Py313 {
                        literal.range =
                            fstring_literal_range(self.context.source, literal.range, is_raw);
                    }
                    literal.load(self)?;
                }
                InterpolatedStringComponent::Interpolation(interpolation, is_raw) => {
                    self.compile_interpolation(interpolation, is_raw)?;
                }
            }
        }
        let range = ruff_text_size::TextRange::new(
            ruff_text_size::TextSize::new(u32::from(range.start()).saturating_sub(1)),
            range.end(),
        );
        self.assembler.set_location(self.source_location(range));
        self.finish_interpolated_string(value_count, "format specification component count")
    }

    fn compile_interpolation(
        &mut self,
        interpolation: &ruff_python_ast::InterpolatedElement,
        is_raw: bool,
    ) -> Result<(), CompileError> {
        self.compile_expression(&interpolation.expression)?;
        self.assembler
            .set_location(self.source_location(interpolation.range));
        if self.context.python_version == PythonVersion::Py312 {
            use crate::target::operands312::FormatValue;
            let mut argument = match interpolation.conversion {
                ConversionFlag::None
                    if interpolation.debug_text.is_some()
                        && interpolation.format_spec.is_none() =>
                {
                    FormatValue::REPR.argument()
                }
                ConversionFlag::None => FormatValue::NONE.argument(),
                ConversionFlag::Str => FormatValue::STR.argument(),
                ConversionFlag::Repr => FormatValue::REPR.argument(),
                ConversionFlag::Ascii => FormatValue::ASCII.argument(),
            };
            if let Some(format_spec) = &interpolation.format_spec {
                self.compile_interpolated_elements(
                    &format_spec.elements,
                    format_spec.range,
                    is_raw,
                )?;
                argument |= FormatValue::HAVE_SPEC.argument();
            }
            self.assembler
                .set_location(self.source_location(interpolation.range));
            return self.emit(
                FORMAT_VALUE,
                argument,
                -i32::from(interpolation.format_spec.is_some()),
            );
        }
        match interpolation.conversion {
            ConversionFlag::None
                if interpolation.debug_text.is_some() && interpolation.format_spec.is_none() =>
            {
                self.emit_conversion(Conversion::REPR)?;
            }
            ConversionFlag::None => {}
            ConversionFlag::Str => self.emit_conversion(Conversion::STR)?,
            ConversionFlag::Repr => self.emit_conversion(Conversion::REPR)?,
            ConversionFlag::Ascii => self.emit_conversion(Conversion::ASCII)?,
        }
        if let Some(format_spec) = &interpolation.format_spec {
            self.compile_interpolated_elements(&format_spec.elements, format_spec.range, is_raw)?;
            self.assembler
                .set_location(self.source_location(interpolation.range));
            self.emit(FORMAT_WITH_SPEC, 0, -1)
        } else {
            self.emit(FORMAT_SIMPLE, 0, 0)
        }
    }

    fn compile_optimized_percent_format(
        &mut self,
        expression: &ExprBinOp,
    ) -> Result<SpecializedLowering, CompileError> {
        if self.context.python_version <= PythonVersion::Py313
            && self.return_value_is_constant(&expression.right)
        {
            // The legacy AST pass folds a constant RHS tuple before recognizing
            // percent formatting, so it no longer has the required Tuple AST node.
            return Ok(SpecializedLowering::NotApplicable);
        }
        let Some(parts) = optimized_percent_format(expression) else {
            return Ok(SpecializedLowering::NotApplicable);
        };

        let expression_location = self.source_location(expression.range);
        let part_count = parts.len();
        for part in parts {
            match part {
                PercentFormatPart::Literal(value) => {
                    if value == "%" {
                        self.context.record_percent_replacement();
                    }
                    // The preprocessing pass synthesizes these constants without
                    // source locations.
                    self.assembler.set_location(SourceLocation::NONE);
                    self.load_string_constant(&value)?;
                    self.assembler
                        .inherit_percent_target_location_for_last_instruction();
                }
                PercentFormatPart::Formatted {
                    expression,
                    conversion,
                    format_spec,
                } => {
                    self.compile_expression(expression)?;
                    let location = self.source_location(expression.range());
                    self.assembler.set_location(location);
                    if self.context.python_version == PythonVersion::Py312 {
                        use crate::target::operands312::FormatValue;
                        let mut argument = if conversion == Conversion::STR {
                            FormatValue::STR.argument()
                        } else if conversion == Conversion::REPR {
                            FormatValue::REPR.argument()
                        } else {
                            FormatValue::ASCII.argument()
                        };
                        let has_spec = format_spec.is_some();
                        if let Some(format_spec) = format_spec {
                            self.assembler.set_location(SourceLocation::NONE);
                            self.load_string_constant(&format_spec)?;
                            self.assembler.set_location(location);
                            argument |= FormatValue::HAVE_SPEC.argument();
                        }
                        self.emit(FORMAT_VALUE, argument, -i32::from(has_spec))?;
                        continue;
                    }
                    self.emit_conversion(conversion)?;
                    if let Some(format_spec) = format_spec {
                        self.assembler.set_location(SourceLocation::NONE);
                        self.load_string_constant(&format_spec)?;
                        self.assembler.set_location(location);
                        self.emit(FORMAT_WITH_SPEC, 0, -1)?;
                    } else {
                        self.emit(FORMAT_SIMPLE, 0, 0)?;
                    }
                }
            }
        }

        self.assembler.set_location(expression_location);
        match part_count {
            0 => self.load_string_constant("")?,
            1 => {}
            count => self.emit(
                BUILD_STRING,
                to_u32(count, "percent-format component count")?,
                1 - i32::try_from(count).unwrap(),
            )?,
        }
        Ok(SpecializedLowering::Applied)
    }

    pub(super) fn load_string_constant(&mut self, value: &str) -> Result<(), CompileError> {
        let constant = self.add_constant(Constant::String(value.to_string()))?;
        self.emit(LOAD_CONST, constant, 1)
    }

    fn finish_interpolated_string(
        &mut self,
        value_count: usize,
        description: &str,
    ) -> Result<(), CompileError> {
        match value_count {
            0 if self.context.python_version == PythonVersion::Py312 => {
                self.emit(BUILD_STRING, 0, 1)
            }
            0 => self.load_string_constant(""),
            1 => Ok(()),
            count => self.emit(
                BUILD_STRING,
                to_u32(count, description)?,
                1 - i32::try_from(count).unwrap(),
            ),
        }
    }

    fn compile_await(
        &mut self,
        expression: &ruff_python_ast::ExprAwait,
    ) -> Result<(), CompileError> {
        if self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR) == 0 {
            return Err(CompileError::invalid_syntax(
                "'await' outside function",
                expression.range,
            ));
        }

        self.compile_expression(&expression.value)?;
        self.compile_awaitable_on_stack_with_native_roles(
            0,
            Some([
                super::legacy_native_labels::NativeRole::AwaitSend(expression.range),
                super::legacy_native_labels::NativeRole::AwaitFail(expression.range),
                super::legacy_native_labels::NativeRole::AwaitExit(expression.range),
            ]),
        )
    }

    pub(super) fn compile_awaitable_on_stack_with_native_roles(
        &mut self,
        get_awaitable_argument: u32,
        native_roles: Option<[super::legacy_native_labels::NativeRole; 3]>,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.stack.depth - 1;
        let cleanup_location = self.assembler.location();
        self.emit(GET_AWAITABLE, get_awaitable_argument, 0)?;
        if self.context.python_version == PythonVersion::Py315 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;

        let send = self.assembler.label();
        let end = self.assembler.label();
        let yielded = self.assembler.label();
        let yielded_end = self.assembler.label();
        let cleanup = self.assembler.label();
        if let Some([send_role, fail_role, exit_role]) = native_roles {
            self.bind_native_source_label(send_role, send);
            self.bind_native_source_label(fail_role, cleanup);
            self.bind_native_source_label(exit_role, end);
        }
        self.assembler.mark(send);
        self.emit_jump_forward(SEND, end, 0)?;
        self.assembler.record_exception_setup(yielded, false);
        self.assembler.mark(yielded);
        self.emit_yield_value(1, 0)?;
        self.assembler.mark(yielded_end);
        self.emit_resume(ResumeLocation::AFTER_AWAIT, ResumeDepth::Zero)?;
        self.emit_jump_backward(JUMP_BACKWARD_NO_INTERRUPT, send, 0)?;

        self.assembler.mark(cleanup);
        self.assembler.add_exception_region(
            yielded,
            yielded_end,
            cleanup,
            (base_depth + 2 + self.iterator_extra_slots()).cast_unsigned(),
            false,
        );
        self.assembler.set_location(cleanup_location);
        self.set_depth(base_depth + 3 + self.iterator_extra_slots());
        self.emit(CLEANUP_THROW, 0, -1)?;
        self.assembler.record_exception_owner_block_boundary(end);
        self.assembler.mark(end);
        self.set_depth(base_depth + 2 + self.iterator_extra_slots());
        self.emit(END_SEND, 0, -self.iterator_slots())?;
        if get_awaitable_argument == 0 {
            self.assembler
                .record_ordinary_await_cleanup(yielded, cleanup);
        }
        Ok(())
    }

    pub(super) fn emit_build(&mut self, opcode: Opcode, length: usize) -> Result<(), CompileError> {
        let length = to_u32(length, "collection element count")?;
        self.emit(opcode, length, 1 - i32::try_from(length).unwrap())
    }

    fn compile_call(&mut self, call: &ruff_python_ast::ExprCall) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            return self.compile_legacy_call(call);
        }
        if let Some(callable) =
            optimized_generator_callable_for_target(call, self.context.python_version)
        {
            return self.compile_optimized_generator_call(call, callable);
        }

        let shape = self.call_shape(call);
        let direct_method = if let Some(attribute) = shape.direct_method {
            if self.compile_super_attribute(attribute, SuperAttributeKind::Method)?
                == SpecializedLowering::NotApplicable
            {
                self.compile_simple_expression_operand(&attribute.value)?;
                let index = self.name_index(attribute.attr.as_str())?;
                self.assembler
                    .set_location(self.attribute_opcode_location(attribute));
                self.emit(LOAD_ATTR, (index << 1) | 1, 1)?;
            }
            true
        } else {
            false
        };
        if !direct_method && let Some(attribute) = shape.imported_module_callable {
            self.compile_simple_expression_operand(&attribute.value)?;
            let index = self.name_index(attribute.attr.as_str())?;
            self.assembler
                .set_location(self.attribute_opcode_location(attribute));
            self.emit(LOAD_ATTR, index << 1, 0)?;
            self.assembler
                .set_location(self.source_location(call.func.range()));
            self.emit(PUSH_NULL, 0, 1)?;
        } else if !direct_method
            && self.compile_direct_global_callable(&call.func)?
                == SpecializedLowering::NotApplicable
        {
            let newly_owned_callable = if let Expr::Name(name) = call.func.as_ref()
                && call
                    .arguments
                    .args
                    .iter()
                    .chain(call.arguments.keywords.iter().map(|keyword| &keyword.value))
                    .any(expression_contains_inlined_comprehension)
            {
                let name = self.required_name_id(name.id.as_str());
                self.symbols.owned_load_locals.insert(name)
            } else {
                false
            };
            self.compile_simple_expression_operand(&call.func)?;
            if newly_owned_callable && let Expr::Name(name) = call.func.as_ref() {
                let name = self.required_name_id(name.id.as_str());
                self.symbols.owned_load_locals.remove(name);
            }
            self.assembler
                .set_location(self.source_location(call.func.range()));
            self.emit(PUSH_NULL, 0, 1)?;
        }
        let call_location = self.call_opcode_location_with_method(call, shape.direct_method);
        self.assembler.set_location(call_location);

        if !shape.has_starred
            && !shape.has_keyword_unpack
            && (direct_method || shape.positional_count + shape.keyword_count * 2 <= 30)
        {
            for argument in &call.arguments.args {
                self.compile_simple_expression_operand(argument)?;
            }
            for keyword in &call.arguments.keywords {
                self.compile_simple_expression_operand(&keyword.value)?;
            }
            let argument_count = call.arguments.args.len() + call.arguments.keywords.len();
            let argument_count = to_u32(argument_count, "call argument count")?;
            if call.arguments.keywords.is_empty() {
                return self.emit(
                    CALL,
                    argument_count,
                    -i32::try_from(argument_count).unwrap() - 1,
                );
            }

            let keyword_names = call
                .arguments
                .keywords
                .iter()
                .map(|keyword| Constant::String(keyword.arg.as_ref().unwrap().as_str().to_string()))
                .collect();
            let names = self.add_interned_string_tuple(keyword_names)?;
            if direct_method {
                let location = if let Expr::Attribute(attribute) = call.func.as_ref() {
                    self.attribute_opcode_location(attribute)
                } else {
                    self.source_location(call.func.range())
                };
                self.assembler.set_location(location);
            }
            self.emit(LOAD_CONST, names, 1)?;
            self.assembler.set_location(call_location);
            return self.emit(
                CALL_KW,
                argument_count,
                -i32::try_from(argument_count).unwrap() - 2,
            );
        }

        let mut starred_argument_pushed_null = false;
        if call.arguments.args.is_empty() {
            self.emit_deferred_constant(Constant::Tuple(Vec::new()))?;
        } else if call.arguments.args.len() == 1 {
            if let Expr::Starred(starred) = &call.arguments.args[0] {
                if call.arguments.keywords.is_empty()
                    && self.compile_direct_global_callable(&starred.value)?
                        == SpecializedLowering::Applied
                {
                    starred_argument_pushed_null = true;
                } else {
                    self.compile_simple_expression_operand(&starred.value)?;
                }
            } else {
                self.compile_collection(&call.arguments.args, CollectionKind::Tuple)?;
            }
        } else {
            self.compile_collection(&call.arguments.args, CollectionKind::Tuple)?;
        }

        if call.arguments.keywords.is_empty() && !starred_argument_pushed_null {
            self.emit(PUSH_NULL, 0, 1)?;
        } else {
            self.compile_call_keyword_dict(call_location, &call.arguments.keywords)?;
        }
        if (!shape.has_starred && !shape.has_keyword_unpack) || starred_argument_pushed_null {
            self.assembler.set_location(call_location);
        }
        self.emit(CALL_FUNCTION_EX, 0, -3)
    }

    /// Emits the 3.12/3.13 call protocols, including their optional keyword dictionary.
    fn compile_legacy_call(
        &mut self,
        call: &ruff_python_ast::ExprCall,
    ) -> Result<(), CompileError> {
        let shape = self.call_shape(call);
        let python312 = self.context.python_version == PythonVersion::Py312;
        let direct_method = if let Some(attribute) = shape.direct_method {
            if self.compile_super_attribute(attribute, SuperAttributeKind::Method)?
                == SpecializedLowering::NotApplicable
            {
                self.compile_expression(&attribute.value)?;
                let index = self.name_index(attribute.attr.as_str())?;
                self.assembler
                    .set_location(self.attribute_opcode_location(attribute));
                self.emit(LOAD_ATTR, (index << 1) | 1, 1)?;
            }
            true
        } else {
            false
        };
        if !direct_method
            && self.compile_direct_global_callable(&call.func)?
                == SpecializedLowering::NotApplicable
        {
            self.assembler
                .set_location(self.source_location(call.func.range()));
            if python312 {
                self.emit(PUSH_NULL, 0, 1)?;
            }
            if let Some(attribute) = shape.imported_module_callable {
                self.compile_expression(&attribute.value)?;
                let index = self.name_index(attribute.attr.as_str())?;
                self.assembler
                    .set_location(self.attribute_opcode_location(attribute));
                self.emit(LOAD_ATTR, index << 1, 0)?;
            } else {
                self.compile_expression(&call.func)?;
            }
            self.assembler
                .set_location(self.source_location(call.func.range()));
            if !python312 {
                self.emit(PUSH_NULL, 0, 1)?;
            }
        }
        let call_location = self.call_opcode_location_with_method(call, shape.direct_method);
        if !shape.has_starred
            && !shape.has_keyword_unpack
            && (direct_method || shape.positional_count + shape.keyword_count * 2 <= 30)
        {
            for argument in &call.arguments.args {
                self.compile_expression(argument)?;
            }
            for keyword in &call.arguments.keywords {
                self.compile_expression(&keyword.value)?;
            }
            let count = to_u32(
                call.arguments.args.len() + call.arguments.keywords.len(),
                "call argument count",
            )?;
            self.assembler.set_location(call_location);
            if call.arguments.keywords.is_empty() {
                return self.emit(CALL, count, -i32::try_from(count).unwrap() - 1);
            }
            let keys = call
                .arguments
                .keywords
                .iter()
                .map(|keyword| Constant::String(keyword.arg.as_ref().unwrap().as_str().to_owned()))
                .collect();
            let keys = self.add_interned_string_tuple(keys)?;
            if let Some(attribute) = shape.direct_method {
                self.assembler
                    .set_location(self.attribute_opcode_location(attribute));
            }
            if python312 {
                self.emit(KW_NAMES, keys, 0)?;
                self.assembler.set_location(call_location);
                self.emit(CALL, count, -i32::try_from(count).unwrap() - 1)
            } else {
                self.emit(LOAD_CONST, keys, 1)?;
                self.assembler.set_location(call_location);
                self.emit(CALL_KW, count, -i32::try_from(count).unwrap() - 2)
            }
        } else {
            self.assembler.set_location(call_location);
            match call.arguments.args.as_ref() {
                [Expr::Starred(starred)] => self.compile_expression(&starred.value)?,
                arguments if arguments.len() <= 2 && !shape.has_starred => {
                    // Synthetic argument tuples bypass the AST constant folder. Native
                    // code generation folds three or more constants; smaller tuples
                    // are admitted by the CFG pass alongside annotation/default tuples.
                    for argument in arguments {
                        self.compile_expression(argument)?;
                    }
                    self.assembler.set_location(call_location);
                    self.emit_build(BUILD_TUPLE, arguments.len())?;
                }
                arguments => self.compile_collection(arguments, CollectionKind::Tuple)?,
            }
            self.compile_call_keyword_dict(call_location, &call.arguments.keywords)?;
            self.assembler.set_location(call_location);
            let has_keywords = !call.arguments.keywords.is_empty();
            self.emit(
                CALL_FUNCTION_EX,
                u32::from(has_keywords),
                -2 - i32::from(has_keywords),
            )
        }
    }

    fn compile_optimized_generator_call(
        &mut self,
        call: &ruff_python_ast::ExprCall,
        callable: &str,
    ) -> Result<(), CompileError> {
        let (common_constant, initial_result, collect_collection) = match callable {
            "all" => (CommonConstant::BUILTIN_ALL.argument(), true, false),
            "any" => (CommonConstant::BUILTIN_ANY.argument(), false, false),
            "tuple" => (CommonConstant::BUILTIN_TUPLE.argument(), false, true),
            "list" => (
                crate::target::operands315::CommonConstant::BUILTIN_LIST.argument(),
                false,
                true,
            ),
            "set" => (
                crate::target::operands315::CommonConstant::BUILTIN_SET.argument(),
                false,
                true,
            ),
            _ => unreachable!("optimized generator call must use a supported builtin"),
        };
        let callable_location = self.source_location(call.func.range());
        let base_depth = self.control.stack.depth;
        let fallback = self.assembler.label();
        let end = self.assembler.label();

        self.compile_expression(&call.func)?;
        self.assembler.set_location(callable_location);
        self.emit(COPY, 1, 1)?;
        self.emit(LOAD_COMMON_CONSTANT, common_constant, 1)?;
        self.emit(IS_OP, 0, -1)?;
        self.emit_jump_forward(POP_JUMP_IF_FALSE, fallback, -1)?;
        let in_inlined_comprehension = !self
            .control
            .comprehensions
            .active_comprehension_region_exclusions
            .is_empty();
        let exclude_not_taken_from_comprehension = collect_collection
            && self
                .control
                .comprehensions
                .exclude_optimized_tuple_guard_from_comprehension
            && in_inlined_comprehension;
        let not_taken_exclusion_start = exclude_not_taken_from_comprehension.then(|| {
            let start = self.assembler.label();
            self.assembler.mark(start);
            start
        });
        // The optimized-call guard already belongs to the original CFG. Only its
        // NOT_TAKEN is appended after compaction and SETUP removal, so resolve the
        // marker from those original slots without Boolean-normalization insertions.
        self.emit_with_exception_policy(
            NOT_TAKEN,
            0,
            0,
            ExceptionPolicy::NoneDynamicConditionStrict,
        )?;
        if let Some(start) = not_taken_exclusion_start {
            let end = self.assembler.label();
            self.assembler.mark(end);
            for exclusions in &mut self
                .control
                .comprehensions
                .active_comprehension_region_exclusions
            {
                exclusions.push((start, end));
            }
        }
        self.emit(POP_TOP, 0, -1)?;

        if collect_collection {
            self.emit(
                if callable == "set" {
                    BUILD_SET
                } else {
                    BUILD_LIST
                },
                0,
                1,
            )?;
        }
        self.compile_expression(&call.arguments.args[0])?;

        if self.context.python_version == PythonVersion::Py315 {
            self.assembler.set_location(callable_location);
            self.emit(PUSH_NULL, 0, 1)?;
        }
        let loop_start = self.assembler.label();
        let exhausted = self.assembler.label();
        let iterator_depth = self.control.stack.depth;
        self.assembler.mark(loop_start);
        self.assembler.set_location(callable_location);
        self.emit_jump_forward(FOR_ITER, exhausted, 1)?;
        if collect_collection {
            self.emit(
                if callable == "set" {
                    SET_ADD
                } else {
                    LIST_APPEND
                },
                1 + self.iterator_slots().cast_unsigned(),
                -1,
            )?;
            self.emit_jump_backward(JUMP_BACKWARD, loop_start, 0)?;
        } else {
            self.emit(TO_BOOL, 0, 0)?;
            let matched = self.assembler.label();
            self.emit_jump_forward(
                if initial_result {
                    POP_JUMP_IF_FALSE
                } else {
                    POP_JUMP_IF_TRUE
                },
                matched,
                -1,
            )?;
            // CPython normalizes the backward conditional after labeling exception
            // targets. Its new block holds an ownerless NOT_TAKEN and backedge;
            // the original forward conditional above retains its protected owner.
            self.emit_with_exception_policy(NOT_TAKEN, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS)?;
            self.emit_jump_backward_with_exception_policy(
                JUMP_BACKWARD,
                loop_start,
                0,
                ExceptionPolicy::EXCLUDE_ALWAYS,
            )?;
            self.assembler.mark(matched);
            self.emit(POP_ITER, 0, -self.iterator_slots())?;
            let result = self.add_constant(Constant::Bool(!initial_result))?;
            self.emit(LOAD_CONST, result, 1)?;
            self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
        }

        self.assembler.mark(exhausted);
        self.set_depth(iterator_depth + 1);
        self.assembler.set_location(callable_location);
        self.emit_end_for_cleanup()?;
        self.assembler.set_location(callable_location);
        if callable == "tuple" {
            self.emit_unary_intrinsic(UnaryIntrinsic::LIST_TO_TUPLE)?;
        } else if !collect_collection {
            let result = self.add_constant(Constant::Bool(initial_result))?;
            self.emit(LOAD_CONST, result, 1)?;
        }
        let exhausted_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, end)?;
        if collect_collection && self.control.generators.copy_optimized_tuple_branch_exit {
            self.assembler
                .allow_transitive_small_exit_inlining(exhausted_exit);
        }

        self.assembler.mark(fallback);
        self.set_depth(base_depth + 1);
        self.assembler.set_location(callable_location);
        self.emit(PUSH_NULL, 0, 1)?;
        self.compile_expression(&call.arguments.args[0])?;
        self.assembler.set_location(self.call_opcode_location(call));
        self.emit(CALL, 1, -2)?;

        // Both optimized and fallback calls enter a new original CFG array here.
        // A later conditional cannot reuse slots removed from either call's block.
        self.assembler.record_exception_owner_block_boundary(end);
        self.assembler.mark(end);
        self.set_depth(base_depth + 1);
        Ok(())
    }

    /// Builds the keyword dictionary for an expanded call in source evaluation order.
    fn compile_call_keyword_dict(
        &mut self,
        call_location: SourceLocation,
        keywords: &[Keyword],
    ) -> Result<(), CompileError> {
        let mut have_dict = false;
        let mut named_start = 0;
        for (index, keyword) in keywords.iter().enumerate() {
            if keyword.arg.is_some() {
                continue;
            }
            if named_start < index {
                self.compile_keyword_map(call_location, &keywords[named_start..index])?;
                if have_dict {
                    self.emit(DICT_MERGE, 1, -1)?;
                }
                have_dict = true;
            }
            if !have_dict {
                self.emit(BUILD_MAP, 0, 1)?;
                have_dict = true;
            }
            if self.context.python_version <= PythonVersion::Py313 {
                self.compile_expression(&keyword.value)?;
            } else {
                self.compile_simple_expression_operand(&keyword.value)?;
            }
            self.assembler.set_location(call_location);
            self.emit(DICT_MERGE, 1, -1)?;
            named_start = index + 1;
        }
        if named_start < keywords.len() {
            self.compile_keyword_map(call_location, &keywords[named_start..])?;
            if have_dict {
                self.emit(DICT_MERGE, 1, -1)?;
            }
        }
        Ok(())
    }

    fn compile_keyword_map(
        &mut self,
        call_location: SourceLocation,
        keywords: &[Keyword],
    ) -> Result<(), CompileError> {
        debug_assert!(!keywords.is_empty() && keywords.iter().all(|keyword| keyword.arg.is_some()));
        if self.context.python_version <= PythonVersion::Py313 && (2..=15).contains(&keywords.len())
        {
            let mut keys = Vec::with_capacity(keywords.len());
            for keyword in keywords {
                self.compile_expression(&keyword.value)?;
                let name = keyword.arg.as_ref().unwrap().as_str();
                self.context.record_constant_string_origin(name, true);
                keys.push(Constant::String(name.to_owned()));
            }
            let keys = self.add_interned_string_tuple(keys)?;
            self.assembler.set_location(call_location);
            self.emit(LOAD_CONST, keys, 1)?;
            let count = to_u32(keywords.len(), "keyword count")?;
            self.emit(BUILD_CONST_KEY_MAP, count, -i32::try_from(count).unwrap())?;
        } else if keywords.len() * 2 > 30 {
            self.assembler.set_location(SourceLocation::NONE);
            self.emit(BUILD_MAP, 0, 1)?;
            for keyword in keywords {
                let name = keyword.arg.as_ref().unwrap();
                self.context
                    .record_constant_string_origin(name.as_str(), true);
                let key = self.add_constant(Constant::String(name.as_str().to_string()))?;
                self.assembler.set_location(call_location);
                self.emit(LOAD_CONST, key, 1)?;
                self.compile_expression(&keyword.value)?;
                self.assembler.set_location(SourceLocation::NONE);
                self.emit(MAP_ADD, 1, -2)?;
            }
        } else {
            for keyword in keywords {
                let name = keyword.arg.as_ref().unwrap();
                self.context
                    .record_constant_string_origin(name.as_str(), true);
                let key = self.add_constant(Constant::String(name.as_str().to_string()))?;
                self.assembler.set_location(call_location);
                self.emit(LOAD_CONST, key, 1)?;
                self.compile_expression(&keyword.value)?;
            }
            self.assembler.set_location(call_location);
            let count = to_u32(keywords.len(), "keyword count")?;
            self.emit(
                BUILD_MAP,
                count,
                1 - i32::try_from(keywords.len() * 2).unwrap(),
            )?;
        }
        self.assembler.set_location(call_location);
        Ok(())
    }

    pub(super) fn imported_module_attribute(
        &self,
        attribute: &ruff_python_ast::ExprAttribute,
    ) -> bool {
        let Expr::Name(name) = attribute.value.as_ref() else {
            return false;
        };
        // CPython only checks whether the name is import-originated in the module symbol table.
        self.is_imported(name.id.as_str())
    }

    fn compile_super_attribute(
        &mut self,
        attribute: &ruff_python_ast::ExprAttribute,
        kind: SuperAttributeKind,
    ) -> Result<SpecializedLowering, CompileError> {
        let Expr::Call(call) = attribute.value.as_ref() else {
            return Ok(SpecializedLowering::NotApplicable);
        };
        let Expr::Name(super_name) = call.func.as_ref() else {
            return Ok(SpecializedLowering::NotApplicable);
        };
        if super_name.id.as_str() != "super"
            || attribute.attr.as_str() == "__class__"
            || !call.arguments.keywords.is_empty()
            || self.is_imported("super")
            || self.is_imported(SHADOWED_SUPER_SENTINEL)
        {
            return Ok(SpecializedLowering::NotApplicable);
        }
        let explicit = match call.arguments.args.as_ref() {
            [] => false,
            [first, second]
                if !matches!(first, Expr::Starred(_)) && !matches!(second, Expr::Starred(_)) =>
            {
                true
            }
            _ => return Ok(SpecializedLowering::NotApplicable),
        };
        let globally_resolved = match &self.unit.scope {
            Scope::Module => true,
            Scope::Function {
                indices,
                free_indices,
                globals,
                ..
            } => {
                self.contains_name(globals, "super")
                    && self.scope_index(indices, "super").is_none()
                    && self.scope_index(free_indices, "super").is_none()
            }
            Scope::Class { bound_names, .. } => !self.contains_name(bound_names, "super"),
        };
        if !globally_resolved {
            return Ok(SpecializedLowering::NotApplicable);
        }

        if explicit {
            self.assembler
                .set_location(self.source_location(super_name.range));
            self.load_name("super")?;
            self.compile_expression(&call.arguments.args[0])?;
            self.compile_expression(&call.arguments.args[1])?;
            self.assembler
                .set_location(self.source_location(attribute.range));
            let attribute_index = self.name_index(attribute.attr.as_str())?;
            self.emit(
                LOAD_SUPER_ATTR,
                (attribute_index << 2) | 2 | kind.method_flag(),
                kind.stack_effect(),
            )?;
            let attribute_location = self.source_location(attribute.range);
            if attribute_location.line != attribute_location.end_line {
                self.assembler
                    .set_location(self.source_location(attribute.attr.range()));
                self.emit(NOP, 0, 0)?;
            } else {
                // CPython removes the same-line specialized-call NOP before appending markers.
                self.assembler.add_last_hidden_exception_owner_slots(1);
                self.assembler.record_last_same_line_super_owner_slot();
            }
            return Ok(SpecializedLowering::Applied);
        }

        if self.unit.arg_count == 0 {
            return Ok(SpecializedLowering::NotApplicable);
        }
        let (class_index, first_parameter, first_parameter_is_cell) = match &self.unit.scope {
            Scope::Function {
                indices,
                free_indices,
                cells,
                globals,
            } if self.contains_name(globals, "super")
                && self.scope_index(indices, "super").is_none() =>
            {
                let Some(class_index) = self.scope_index(free_indices, "__class__") else {
                    return Ok(SpecializedLowering::NotApplicable);
                };
                let Some(first_parameter_name) = self.symbols.locals.first().copied() else {
                    return Ok(SpecializedLowering::NotApplicable);
                };
                let first_parameter_is_cell = cells.contains(first_parameter_name);
                let Some(first_parameter) = indices.get(&first_parameter_name).copied() else {
                    return Ok(SpecializedLowering::NotApplicable);
                };
                (class_index, first_parameter, first_parameter_is_cell)
            }
            _ => return Ok(SpecializedLowering::NotApplicable),
        };

        self.assembler
            .set_location(self.source_location(super_name.range));
        let super_index = self.name_index("super")?;
        self.emit(LOAD_GLOBAL, super_index << 1, 1)?;
        self.assembler
            .set_location(self.source_location(call.range));
        self.emit(LOAD_DEREF, class_index, 1)?;
        let receiver_opcode = if first_parameter_is_cell {
            LOAD_DEREF
        } else {
            LOAD_FAST
        };
        self.emit(receiver_opcode, first_parameter, 1)?;
        self.assembler
            .set_location(self.source_location(attribute.range));
        let attribute_index = self.name_index(attribute.attr.as_str())?;
        self.emit(
            LOAD_SUPER_ATTR,
            (attribute_index << 2) | kind.method_flag(),
            kind.stack_effect(),
        )?;
        let attribute_location = self.source_location(attribute.range);
        if attribute_location.line != attribute_location.end_line {
            self.assembler
                .set_location(self.source_location(attribute.attr.range()));
            self.emit(NOP, 0, 0)?;
        } else {
            // CPython removes the same-line specialized-call NOP before appending markers.
            self.assembler.add_last_hidden_exception_owner_slots(1);
            self.assembler.record_last_same_line_super_owner_slot();
        }
        Ok(SpecializedLowering::Applied)
    }

    fn compile_direct_global_callable(
        &mut self,
        expression: &Expr,
    ) -> Result<SpecializedLowering, CompileError> {
        if self.unit.annotation_classdict_index.is_some() {
            return Ok(SpecializedLowering::NotApplicable);
        }
        let Expr::Name(name) = expression else {
            return Ok(SpecializedLowering::NotApplicable);
        };
        // AST preprocessing replaces __debug__ with a Boolean before callable lowering.
        if name.id.as_str() == "__debug__" {
            return Ok(SpecializedLowering::NotApplicable);
        }
        let name_id = self.required_name_id(name.id.as_str());
        if self.symbols.active_temporaries.contains(name_id) {
            return Ok(SpecializedLowering::NotApplicable);
        }
        let is_global = match &self.unit.scope {
            Scope::Module => self.symbols.module_globals.contains(name_id),
            Scope::Function {
                indices,
                free_indices,
                globals,
                ..
            } => {
                globals.contains(name_id)
                    || (!indices.contains_key(&name_id) && !free_indices.contains_key(&name_id))
            }
            Scope::Class {
                globals,
                free_indices,
                ..
            } => {
                globals.contains(name_id)
                    || (!self
                        .control
                        .comprehensions
                        .active_comprehension_local_names
                        .is_empty()
                        && !free_indices.contains_key(&name_id))
            }
        };
        if !is_global {
            return Ok(SpecializedLowering::NotApplicable);
        }
        self.assembler
            .set_location(self.source_location(expression.range()));
        let index = self.name_index_with_id(name.id.as_str(), name_id)?;
        self.emit(LOAD_GLOBAL, (index << 1) | 1, 2)?;
        Ok(SpecializedLowering::Applied)
    }

    fn compile_lambda(&mut self, lambda: &ruff_python_ast::ExprLambda) -> Result<(), CompileError> {
        let empty_parameters = ruff_python_ast::Parameters::default();
        let parameters = lambda.parameters.as_deref().unwrap_or(&empty_parameters);
        if parameters
            .iter()
            .any(|parameter| parameter.annotation().is_some())
        {
            return Err(unsupported("lambda parameter annotation"));
        }

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
                    .map(|default| fold_constant_for_version(default, self.context.python_version))
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
                for default in &positional_defaults {
                    self.record_folded_value(default)?;
                }
                self.assembler
                    .set_location(self.source_location(lambda.range));
                self.emit_deferred_constant(Constant::Tuple(defaults))?;
                self.assembler
                    .add_last_hidden_exception_owner_slots(hidden_owner_slots);
            } else {
                for default in &positional_defaults {
                    self.compile_expression(default)?;
                }
                if !self.has_deferred_annotations() {
                    self.assembler
                        .set_location(self.source_location(lambda.range));
                }
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
        if !keyword_defaults.is_empty() && self.context.python_version <= PythonVersion::Py313 {
            let mut keys = Vec::with_capacity(keyword_defaults.len());
            for (name, default) in &keyword_defaults {
                self.compile_expression(default)?;
                let mangled = self.mangled_name(name);
                self.context
                    .record_identifier_constant_origin(name, &mangled);
                keys.push(Constant::String(mangled));
            }
            let keys = self.add_interned_string_tuple(keys)?;
            self.emit(LOAD_CONST, keys, 1)?;
            let count = to_u32(keyword_defaults.len(), "keyword default count")?;
            self.emit(BUILD_CONST_KEY_MAP, count, -i32::try_from(count).unwrap())?;
        } else if !keyword_defaults.is_empty() {
            for (name, default) in &keyword_defaults {
                let mangled = self.mangled_name(name);
                self.context
                    .record_identifier_constant_origin(name, &mangled);
                let name = self.add_constant(Constant::String(mangled))?;
                self.emit(LOAD_CONST, name, 1)?;
                self.compile_expression(default)?;
            }
            let count = to_u32(keyword_defaults.len(), "keyword default count")?;
            self.emit(BUILD_MAP, count, 1 - i32::try_from(count).unwrap() * 2)?;
        }

        let analysis = analyze_lambda_scope(lambda);
        let freevar_names = analysis
            .required
            .iter()
            .filter(|name| self.can_provide_closure(name))
            .cloned()
            .collect::<BTreeSet<_>>();
        let globals = self.name_id_set(analysis.required.difference(&freevar_names));
        let freevars = self.name_ids(&freevar_names);
        let closure_names = freevars.clone();
        let plan = LoweringFunctionPlan {
            locals: self.name_ids(analysis.locals),
            globals,
            cellvars: self.name_id_set(analysis.cellvars),
            inlined_comprehension_cellvars: self
                .name_id_set(analysis.inlined_comprehension_cellvars),
            freevars,
            ..LoweringFunctionPlan::default()
        };

        let arg_count = to_u32(
            parameters.posonlyargs.len() + parameters.args.len(),
            "lambda argument count",
        )?;
        let positional_only = to_u32(
            parameters.posonlyargs.len(),
            "lambda positional-only argument count",
        )?;
        let keyword_only = to_u32(
            parameters.kwonlyargs.len(),
            "lambda keyword-only argument count",
        )?;
        let mut parameter_flags = (if parameters.vararg.is_some() {
            CO_VARARGS
        } else {
            0
        } | if parameters.kwarg.is_some() {
            CO_VARKEYWORDS
        } else {
            0
        } | if matches!(self.unit.scope, Scope::Function { .. })
            || !self
                .control
                .comprehensions
                .active_comprehension_local_names
                .is_empty()
        {
            CO_NESTED
        } else if matches!(self.unit.scope, Scope::Class { .. }) {
            (if self.context.python_version >= PythonVersion::Py314 {
                CO_METHOD
            } else {
                0
            }) | if self.unit.class_scope_is_nested {
                CO_NESTED
            } else {
                0
            }
        } else if !closure_names.is_empty() {
            CO_NESTED
        } else {
            0
        }) | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        if expression_contains_yield(&lambda.body) {
            parameter_flags |= CO_GENERATOR;
        }
        let qualified_name = self.child_qualified_name("<lambda>");
        let first_line_number = self.line_number(u32::from(lambda.range.start()));
        let mut child = Self::function(
            Rc::clone(&self.context),
            "<lambda>",
            qualified_name,
            first_line_number,
            &plan,
            self.unit.scope_id,
            arg_count,
            positional_only,
            keyword_only,
            parameter_flags,
        )?;
        child.unit.private_name.clone_from(&self.unit.private_name);
        child
            .unit
            .mangled_names
            .clone_from(&self.unit.mangled_names);
        child
            .symbols
            .imported_scope_names
            .clone_from(&self.symbols.imported_scope_names);
        child.emit_function_prologue()?;
        if self.context.python_version <= PythonVersion::Py313 {
            child.add_constant(Constant::None)?;
        }
        if parameter_flags & CO_GENERATOR != 0 {
            // CPython does not wrap generator lambdas in the stop-iteration
            // handler used by `def` generators and generator expressions.
            child.control.generators.generator_region_start = None;
        }
        let return_location = child.expression_location(&lambda.body);
        child.compile_tail_return_expression(&lambda.body, return_location)?;
        let child = child.finish_inner(false)?;

        if !closure_names.is_empty() {
            self.emit_closure_tuple(&closure_names)?;
        }
        let constant = self.add_constant(Constant::Code(Box::new(child)))?;
        self.emit(LOAD_CONST, constant, 1)?;
        let flags = if closure_names.is_empty() {
            0
        } else {
            FunctionAttribute::CLOSURE.argument()
        } | if keyword_defaults.is_empty() {
            0
        } else {
            FunctionAttribute::KWDEFAULTS.argument()
        } | if positional_defaults.is_empty() {
            0
        } else {
            FunctionAttribute::DEFAULTS.argument()
        };
        self.emit_make_function(flags)?;
        Ok(())
    }

    pub(super) fn compile_comprehension(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        key: Option<&Expr>,
        value: &Expr,
        build_opcode: Opcode,
        add_opcode: Opcode,
        discard_result: bool,
    ) -> Result<(), CompileError> {
        if generators.is_empty() {
            return Err(CompileError::Internal(
                "comprehension has no generators".to_string(),
            ));
        }
        let comprehension_cell_names = comprehension_cell_names(generators, key, value);
        let base_depth = self.control.stack.depth;
        let comprehension_location = self.assembler.location();
        // Legacy AST folding applies to every comprehension iterable, including
        // async clauses; ordinary async-for statements follow a different rule.
        if (generators[0].is_async && self.context.python_version >= PythonVersion::Py314)
            || self.context.python_version == PythonVersion::Py315
        {
            self.compile_expression(&generators[0].iter)?;
        } else {
            self.compile_iterable_expression(&generators[0].iter)?;
        }
        if self.context.python_version <= PythonVersion::Py314 {
            self.assembler
                .set_location(self.source_location(generators[0].iter.range()));
            self.emit_comprehension_iterator(generators[0].is_async)?;
        }
        self.assembler.set_location(comprehension_location);

        let mut temporary_names = comprehension_temporary_names(generators, key, value);
        let mut local_names = Vec::new();
        let mut self_iterated_nested_targets = Vec::new();
        for generator in generators {
            collect_target_names(&generator.target, &mut local_names);
        }
        for (index, generator) in generators.iter().enumerate() {
            // The first iterable runs before entering the inlined comprehension scope.
            if index > 0 {
                collect_self_iterated_nested_comprehension_targets(
                    &generator.iter,
                    &mut self_iterated_nested_targets,
                );
            }
            for condition in &generator.ifs {
                collect_self_iterated_nested_comprehension_targets(
                    condition,
                    &mut self_iterated_nested_targets,
                );
            }
        }
        if let Some(key) = key {
            collect_self_iterated_nested_comprehension_targets(
                key,
                &mut self_iterated_nested_targets,
            );
        }
        collect_self_iterated_nested_comprehension_targets(
            value,
            &mut self_iterated_nested_targets,
        );
        let mut active_temporary_names = temporary_names.clone();
        let legacy_class_comprehension = self.context.python_version <= PythonVersion::Py313
            && matches!(self.unit.scope, Scope::Class { .. })
            && self
                .control
                .comprehensions
                .active_comprehension_local_names
                .is_empty();
        if legacy_class_comprehension {
            let Scope::Class { free_indices, .. } = &self.unit.scope else {
                unreachable!("legacy class comprehension requires a class scope");
            };
            let mut seen = FxHashSet::default();
            temporary_names = legacy_class_comprehension_names(generators, key, value)
                .into_iter()
                .filter(|(name, requires_free)| {
                    (!requires_free
                        || (self.scope_index(free_indices, name).is_some()
                            && super::scope::legacy_class_comprehension_keeps_free_name(
                                name,
                                self.context.python_version,
                            )))
                        && seen.insert(name.clone())
                })
                .map(|(name, _)| name)
                .collect();
        }
        let global_saves = match &self.unit.scope {
            Scope::Module => {
                global_comprehension_temporary_names(generators, key, value, |_| true, None)
            }
            Scope::Function { globals, .. } if globals.iter_unordered().next().is_some() => {
                global_comprehension_temporary_names(
                    generators,
                    key,
                    value,
                    |name| globals.contains(self.required_name_id(name)),
                    self.unit.private_name.as_deref(),
                )
            }
            _ => None,
        };
        if let Some(global_saves) = global_saves {
            // Save explicit-global walrus slots in symbol order without changing their accesses.
            // Only iteration targets belong in active_temporary_names above.
            temporary_names = global_saves;
        }
        // An inner target read by its own iterable must keep the enclosing value live until the
        // inner comprehension starts and takes ownership of its save/restore.
        let direct_temporary_names = local_names.iter().cloned().collect::<FxHashSet<_>>();
        let self_iterated_nested_targets = self_iterated_nested_targets
            .into_iter()
            .collect::<FxHashSet<_>>();
        let free_scope_names = if matches!(&self.unit.scope,
            Scope::Function { free_indices, .. } if !free_indices.is_empty())
        {
            comprehension_name_bindings(generators, key, value)
                .into_iter()
                .filter_map(|(name, bound)| (!bound).then_some(name))
                .collect()
        } else {
            FxHashSet::default()
        };
        let deferred_free_targets = free_scope_names
            .iter()
            .filter(|name| {
                let name_id = self.required_name_id(name);
                !direct_temporary_names.contains(*name)
                    && (!comprehension_cell_names.contains(*name) || self.is_cell(name))
                    && matches!(&self.unit.scope, Scope::Function { free_indices, .. }
                        if free_indices.contains_key(&name_id))
                    && (!self.symbols.active_temporaries.contains(name_id)
                        || self.symbols.active_temporary_free_cells.contains(name_id))
            })
            .cloned()
            .collect::<FxHashSet<_>>();
        active_temporary_names.retain(|name| !deferred_free_targets.contains(name));
        temporary_names.retain(|name| !deferred_free_targets.contains(name));
        if !legacy_class_comprehension {
            temporary_names.retain(|name| {
                direct_temporary_names.contains(name)
                    || !self_iterated_nested_targets.contains(name)
                    || !self.is_initialized(name)
            });
        }

        let mut seen_temporaries = FxHashSet::default();
        temporary_names.retain(|name| {
            let key = if matches!(&self.unit.scope, Scope::Function { globals, .. }
                if globals.contains(self.required_name_id(name)))
            {
                self.mangled_name(name)
            } else {
                name.clone()
            };
            seen_temporaries.insert(key)
        });
        // CPython saves the hidden fast slot, but creates and accesses a captured
        // target's cell through its existing free slot while the comprehension is active.
        let mut comprehension_free_cell_indices = FxHashMap::default();
        if let Scope::Function { free_indices, .. } = &self.unit.scope {
            for name in &temporary_names {
                let name_id = self.required_name_id(name);
                if comprehension_cell_names.contains(name)
                    && self.is_cell(name)
                    && let Some(index) = free_indices.get(&name_id).copied()
                    && (!self.symbols.active_temporaries.contains(name_id)
                        || self.symbols.active_temporary_free_cells.contains(name_id))
                {
                    comprehension_free_cell_indices.insert(name_id, index);
                }
            }
        }
        let mut temporary_indices = Vec::with_capacity(temporary_names.len());
        for name in &temporary_names {
            let index = self.ensure_temporary(name)?;
            temporary_indices.push(index);
            self.emit(LOAD_FAST_AND_CLEAR, index, 1)?;
            if comprehension_cell_names.contains(name) && self.is_cell(name) {
                let cell_index = comprehension_free_cell_indices
                    .get(&self.required_name_id(name))
                    .copied()
                    .unwrap_or(index);
                self.emit(MAKE_CELL, cell_index, 0)?;
            }
        }
        let temporary_restores = temporary_names
            .iter()
            .zip(&temporary_indices)
            .map(|(name, index)| ComprehensionRestore {
                index: *index,
                name: self.required_name_id(name),
            })
            .collect::<Vec<_>>();
        if !temporary_names.is_empty() {
            self.emit(
                SWAP,
                to_u32(temporary_names.len() + 1, "comprehension local count")?,
                0,
            )?;
        }
        let local_names: NameSet = local_names
            .iter()
            .map(|name| self.required_name_id(name))
            .collect();
        let direct_temporary_ids = local_names.iter_unordered().collect::<Vec<_>>();
        let active_temporary_ids: Vec<_> = active_temporary_names
            .iter()
            .map(|name| self.required_name_id(name))
            .collect();
        let previous_active_temporary_ids: NameSet = active_temporary_ids
            .iter()
            .copied()
            .filter(|name| self.symbols.active_temporaries.contains(*name))
            .collect();
        let comprehension_cell_ids = self.name_id_set(&comprehension_cell_names);
        let previous_fast_temporary_ids: NameSet = direct_temporary_ids
            .iter()
            .copied()
            .filter(|name| self.symbols.active_temporary_fast_cells.contains(*name))
            .collect();
        let previous_free_temporary_ids: NameSet = active_temporary_ids
            .iter()
            .copied()
            .filter(|name| self.symbols.active_temporary_free_cells.contains(*name))
            .collect();
        for name in &active_temporary_ids {
            if comprehension_free_cell_indices.contains_key(name) {
                self.symbols.active_temporary_free_cells.insert(*name);
            } else {
                self.symbols.active_temporary_free_cells.remove(*name);
            }
        }
        for name in &direct_temporary_ids {
            let is_fast_local = self
                .symbols
                .temporary_indices
                .get(name)
                .and_then(|index| usize::try_from(*index).ok())
                .is_some_and(|index| index < self.symbols.fast_local_count);
            if comprehension_cell_ids.contains(name) || !is_fast_local {
                self.symbols.active_temporary_fast_cells.remove(*name);
            } else {
                self.symbols.active_temporary_fast_cells.insert(*name);
            }
        }
        self.symbols
            .active_temporaries
            .extend(active_temporary_ids.iter().copied());

        let protected_start = self.assembler.label();
        let protected_end = self.assembler.label();
        let cleanup = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ComprehensionCleanup(
                generators[0].iter.range(),
            ),
            cleanup,
        );
        let parent_cleanup = self
            .control
            .comprehensions
            .active_comprehension_cleanups
            .last()
            .copied();
        let cleanup_location = self.assembler.location();
        if !temporary_names.is_empty() {
            self.assembler
                .record_exception_setup(protected_start, false);
        }
        self.assembler.mark(protected_start);
        self.emit(build_opcode, 0, 1)?;
        self.emit(SWAP, 2, 0)?;
        let cleanup_depth =
            (base_depth + i32::try_from(temporary_names.len()).unwrap() + 1).cast_unsigned();
        // CPython's method-call optimization reads `DEF_IMPORT` from the mutable module
        // symbol table. An inlined module comprehension temporarily replaces the symbols
        // whose scopes differ after compiling its first iterable. Mirror that effective
        // import state here so nested lambdas and generator expressions inherit it, then
        // restore the exact removed names when this comprehension ends.
        let mut removed_imports = Vec::new();
        let mut removed_module_globals = Vec::new();
        let mut added_module_globals = Vec::new();
        if matches!(self.unit.scope, Scope::Module) {
            let facts = comprehension_scope_facts(generators, key, value);
            // A nested `global` declaration also marks the module symbol explicit.
            // The comprehension's own reads are implicit globals instead, except
            // for its assignment-expression targets. Save the effective outer flags
            // before the import overlay, then replace them only for this body.
            removed_module_globals = facts
                .names
                .iter()
                .filter_map(|name| {
                    let name_id = self.required_name_id(name);
                    (!facts.walrus_targets.contains(name)
                        && self.symbols.module_globals.contains(name_id))
                    .then_some(name_id)
                })
                .collect();
            // An outer inlined comprehension may have temporarily masked this
            // flag. A nested assignment expression still owns an explicit global.
            added_module_globals = facts
                .walrus_targets
                .iter()
                .map(|name| self.required_name_id(name))
                .filter(|name| !self.symbols.module_globals.contains(*name))
                .collect();
            for name in facts.names {
                let name_id = self.required_name_id(&name);
                if !self.symbols.imported_scope_names.contains(name_id)
                    || facts.walrus_targets.contains(&name)
                {
                    continue;
                }
                let is_local = local_names.contains(name_id);
                let is_free = !is_local
                    && self
                        .control
                        .comprehensions
                        .active_comprehension_local_names
                        .iter()
                        .any(|locals| locals.contains(name_id));
                if (is_local
                    && !comprehension_cell_names.contains(&name)
                    && !self.symbols.module_globals.contains(name_id))
                    || is_free
                {
                    continue;
                }
                if self.symbols.imported_scope_names.remove(name_id) {
                    removed_imports.push(name_id);
                }
            }
            for &name in &removed_module_globals {
                self.symbols.module_globals.remove(name);
            }
            self.symbols
                .module_globals
                .extend(added_module_globals.iter().copied());
        }
        let result = self.with_control_override(
            |control| &mut control.comprehensions.in_unfiltered_comprehension,
            generators.iter().all(|generator| generator.ifs.is_empty()),
            |compiler| {
                let suppressed = std::mem::take(
                    &mut compiler
                        .control
                        .comprehensions
                        .suppress_filtered_comprehension_none_owner,
                );
                let in_element =
                    std::mem::take(&mut compiler.control.comprehensions.in_comprehension_element);
                let root_if_filter = std::mem::replace(
                    &mut compiler
                        .control
                        .comprehensions
                        .filtered_comprehension_has_root_if_expression,
                    generators.len() != 1
                        || generators.iter().any(|generator| {
                            generator
                                .ifs
                                .iter()
                                .any(comprehension_filter_has_root_if_expression)
                        }),
                );
                let result = compiler.with_comprehension_context(
                    (!temporary_names.is_empty()).then_some((cleanup, cleanup_depth)),
                    local_names,
                    |compiler| {
                        compiler.compile_comprehension_generator(
                            generators, 0, key, value, add_opcode, 0,
                        )
                    },
                );
                compiler
                    .control
                    .comprehensions
                    .suppress_filtered_comprehension_none_owner = suppressed;
                compiler.control.comprehensions.in_comprehension_element = in_element;
                compiler
                    .control
                    .comprehensions
                    .filtered_comprehension_has_root_if_expression = root_if_filter;
                result
            },
        );
        self.symbols.imported_scope_names.extend(removed_imports);
        self.symbols.module_globals.extend(removed_module_globals);
        for name in added_module_globals {
            self.symbols.module_globals.remove(name);
        }
        let ((), region_exclusions) = result?;
        self.assembler.mark(protected_end);

        let restore_temporary_scopes = |compiler: &mut Self| {
            for &name in &active_temporary_ids {
                if !previous_active_temporary_ids.contains(name) {
                    compiler.symbols.active_temporaries.remove(name);
                }
                if previous_free_temporary_ids.contains(name) {
                    compiler.symbols.active_temporary_free_cells.insert(name);
                } else {
                    compiler.symbols.active_temporary_free_cells.remove(name);
                }
            }
            for &name in &direct_temporary_ids {
                if previous_fast_temporary_ids.contains(name) {
                    compiler.symbols.active_temporary_fast_cells.insert(name);
                } else {
                    compiler.symbols.active_temporary_fast_cells.remove(name);
                }
            }
        };
        if temporary_names.is_empty() {
            restore_temporary_scopes(self);
            self.set_depth(base_depth + 1);
            if discard_result {
                self.assembler
                    .set_location(self.source_location(generators.last().unwrap().iter.range()));
                self.emit(POP_TOP, 0, -1)?;
            }
            return Ok(());
        }

        let normal_cleanup = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ComprehensionEnd(generators[0].iter.range()),
            normal_cleanup,
        );
        let normal_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, normal_cleanup)?;
        if discard_result && generators.iter().any(|generator| generator.is_async) {
            self.assembler.prevent_jump_inlining(normal_exit);
        }
        if !discard_result {
            self.assembler
                .record_original_comprehension_normal_exit(normal_exit, normal_cleanup);
        }
        self.assembler.mark(cleanup);
        self.set_depth(base_depth + i32::try_from(temporary_indices.len()).unwrap() + 2);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(SWAP, 2, 0)?;
        self.emit(POP_TOP, 0, -1)?;
        self.assembler.set_location(cleanup_location);
        self.emit(
            SWAP,
            to_u32(temporary_indices.len() + 1, "comprehension local count")?,
            0,
        )?;
        for (position, index) in temporary_indices.iter().rev().enumerate() {
            if position > 0 {
                self.assembler.fusion_barrier();
            }
            self.emit(STORE_FAST, *index, -1)?;
        }
        self.emit_reraise(ReraiseStackState::ExceptionOnly)?;
        let cleanup_end = self.assembler.label();
        self.assembler.mark(cleanup_end);
        if let Some((parent, depth)) = parent_cleanup {
            self.assembler
                .add_exception_region(cleanup, cleanup_end, parent, depth, false);
        }
        self.assembler.mark(normal_cleanup);

        let deferred_normal_restore = self
            .control
            .comprehensions
            .defer_async_comprehension_restore
            && generators.iter().any(|generator| generator.is_async);
        self.set_depth(base_depth + i32::try_from(temporary_names.len()).unwrap() + 1);
        self.assembler.set_location(comprehension_location);
        if deferred_normal_restore {
            // The normal restore follows the asynchronous iterator's exception-only exit.
            // A later independent branch entry can still make its continuation reachable.
            if generators[0].is_async {
                self.assembler.prevent_borrow_reachability(normal_cleanup);
            }
            self.control
                .comprehensions
                .pending_comprehension_restores
                .push((temporary_restores, comprehension_location));
        } else {
            // Legacy targets count the original SWAP before copying small exit blocks.
            // 3.12 also needs the unlocated discard until its later line resolution.
            let discard_before_restore =
                discard_result && self.context.python_version > crate::PythonVersion::Py313;
            if discard_before_restore {
                self.emit(POP_TOP, 0, -1)?;
            } else {
                self.emit(
                    SWAP,
                    to_u32(temporary_names.len() + 1, "comprehension local count")?,
                    0,
                )?;
            }
            if discard_before_restore {
                let final_restore = temporary_restores.last();
                let restores = temporary_restores[..temporary_restores.len().saturating_sub(1)]
                    .iter()
                    .rev()
                    .chain(final_restore);
                for (position, restore) in restores.copied().enumerate() {
                    if position > 0 {
                        self.assembler.fusion_barrier();
                    }
                    let emitted = self.emit_comprehension_restore(restore)?;
                    self.assembler
                        .prevent_instruction_fusion_with_previous(emitted);
                }
            } else {
                // STORE_FAST_MAYBE_NULL restores share one original CPython block.
                // Per-instruction fusion policies below preserve their distinct opcodes
                // without splitting that block before small-exit copying.
                for restore in temporary_restores.iter().rev().copied() {
                    let emitted = self.emit_comprehension_restore(restore)?;
                    self.assembler
                        .prevent_instruction_fusion_with_previous(emitted);
                }
            }
            // CPython converts STORE_FAST_MAYBE_NULL only after inserting superinstructions.
            self.assembler.prevent_last_instruction_fusion();
            if discard_result && !discard_before_restore {
                self.assembler.set_location(SourceLocation::NONE);
                self.emit(POP_TOP, 0, -1)?;
            }
            if !discard_result {
                self.assembler
                    .record_original_comprehension_normal_restores(normal_cleanup);
            }
            if !discard_result && generators.iter().all(|generator| !generator.is_async) {
                self.assembler
                    .record_comprehension_restore_owner_prefix(normal_cleanup);
            }
        }
        restore_temporary_scopes(self);

        let mut region_start = protected_start;
        for (exclusion_start, exclusion_end) in region_exclusions {
            self.assembler.add_exception_region_with_origin(
                region_start,
                exclusion_start,
                cleanup,
                cleanup_depth,
                false,
                (protected_start, protected_end),
            );
            region_start = exclusion_end;
        }
        self.assembler.add_exception_region_with_origin(
            region_start,
            protected_end,
            cleanup,
            cleanup_depth,
            false,
            (protected_start, protected_end),
        );
        self.set_depth(
            base_depth
                + if deferred_normal_restore {
                    i32::try_from(temporary_names.len()).unwrap() + 1
                } else {
                    i32::from(!discard_result)
                },
        );
        Ok(())
    }

    fn compile_generator_expression(
        &mut self,
        generator: &ruff_python_ast::ExprGenerator,
    ) -> Result<(), CompileError> {
        if generator.generators.is_empty() {
            return Err(CompileError::Internal(
                "generator expression has no generators".to_string(),
            ));
        }
        let is_async = generator_is_async(generator);

        let mut locals = vec![".0".to_string()];
        // Captured inlined targets retain their fast-local slots, unlike cells belonging
        // only to the generator. This distinction also determines the final cell order.
        let mut inlined_cellvars = FxHashSet::default();
        for (index, comprehension) in generator.generators.iter().enumerate() {
            if index > 0 {
                // A later iterable executes before its target is stored. Inlined
                // comprehensions in that iterable therefore reserve their temporary fast
                // locals before the generator target.
                collect_nested_comprehension_target_names(&comprehension.iter, &mut locals);
                inlined_cellvars.extend(inlined_comprehension_cell_names_in_expression(
                    &comprehension.iter,
                ));
            }
            collect_target_names(&comprehension.target, &mut locals);
        }
        collect_nested_comprehension_target_names(&generator.elt, &mut locals);
        inlined_cellvars.extend(inlined_comprehension_cell_names_in_expression(
            &generator.elt,
        ));
        for (index, comprehension) in generator.generators.iter().enumerate() {
            collect_nested_comprehension_target_names(&comprehension.target, &mut locals);
            if index > 0 {
                collect_nested_comprehension_target_names(&comprehension.iter, &mut locals);
            }
            for condition in &comprehension.ifs {
                collect_nested_comprehension_target_names(condition, &mut locals);
                inlined_cellvars.extend(inlined_comprehension_cell_names_in_expression(condition));
            }
        }
        let mut seen = FxHashSet::default();
        locals.retain(|name| seen.insert(name.clone()));
        let required_names = generator_required_names(generator);
        let generator_cellvars = generator_cellvars(generator);
        let mut globals = FxHashSet::default();
        let mut freevars = BTreeSet::new();
        for name in required_names {
            if self.can_provide_closure(&name) {
                freevars.insert(name);
            } else {
                globals.insert(name);
            }
        }
        let locals = self.name_ids(locals);
        let globals = self.name_id_set(globals);
        let generator_cellvars = self.name_id_set(generator_cellvars);
        let freevars = self.name_ids(freevars);
        let closure_names = freevars.clone();
        let generator_range = self.generator_expression_range(generator);
        let plan = LoweringFunctionPlan {
            locals,
            globals,
            cellvars: generator_cellvars,
            inlined_comprehension_cellvars: self.name_id_set(inlined_cellvars),
            freevars,
            ..LoweringFunctionPlan::default()
        };
        let inlined_comprehension_parent = !self
            .control
            .comprehensions
            .active_comprehension_local_names
            .is_empty();
        let flags = if is_async {
            CO_ASYNC_GENERATOR
        } else {
            CO_GENERATOR
        } | if self.child_function_is_nested() || inlined_comprehension_parent {
            CO_NESTED
        } else {
            0
        } | if self.context.python_version >= PythonVersion::Py314
            && matches!(self.unit.scope, Scope::Class { .. })
            && !inlined_comprehension_parent
        {
            CO_METHOD
        } else {
            0
        } | (self.unit.flags & SUPPORTED_FUTURE_FLAGS);
        let mut child = Self::function(
            Rc::clone(&self.context),
            "<genexpr>",
            self.child_qualified_name("<genexpr>"),
            self.line_number(u32::from(generator_range.start())),
            &plan,
            self.unit.scope_id,
            1,
            0,
            0,
            flags,
        )?;
        child.control.generators.generator_expression = true;
        child.unit.private_name.clone_from(&self.unit.private_name);
        child
            .unit
            .mangled_names
            .clone_from(&self.unit.mangled_names);
        child
            .symbols
            .imported_scope_names
            .clone_from(&self.symbols.imported_scope_names);
        // CPython only inserts `.<locals>` for function, async-function, and lambda parents.
        child.unit.child_qualified_name_parent = Some(child.unit.qualified_name.clone());
        let generator_location = child.source_location(generator_range);
        if child.context.python_version == PythonVersion::Py315 {
            child.emit_generator_expression_prologue(
                generator.generators[0].is_async,
                child.source_location(generator.generators[0].iter.range()),
            )?;
        } else {
            child.emit_function_prologue()?;
            child.assembler.set_location(generator_location);
            child.emit_owned_fast(0, 1)?;
        }
        child.assembler.set_location(generator_location);
        child.compile_generator_expression_loop(
            &generator.generators,
            0,
            &generator.elt,
            generator_location,
        )?;
        let child = child.finish()?;

        self.assembler
            .set_location(self.source_location(generator_range));
        if !closure_names.is_empty() {
            self.emit_closure_tuple(&closure_names)?;
        }
        let code = self.add_constant(Constant::Code(Box::new(child)))?;
        self.emit(LOAD_CONST, code, 1)?;
        self.emit_make_function(if closure_names.is_empty() {
            0
        } else {
            FunctionAttribute::CLOSURE.argument()
        })?;
        if (generator.generators[0].is_async && self.context.python_version >= PythonVersion::Py314)
            || self.context.python_version == PythonVersion::Py315
        {
            self.compile_expression(&generator.generators[0].iter)?;
        } else {
            self.compile_iterable_expression(&generator.generators[0].iter)?;
        }
        if self.context.python_version <= PythonVersion::Py314 {
            self.assembler
                .set_location(self.source_location(generator.generators[0].iter.range()));
            self.emit_comprehension_iterator(generator.generators[0].is_async)?;
        }
        self.assembler
            .set_location(self.source_location(generator_range));
        self.emit(CALL, 0, -1)
    }

    fn compile_generator_expression_loop(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        index: usize,
        element: &Expr,
        generator_location: SourceLocation,
    ) -> Result<(), CompileError> {
        let generator = &generators[index];
        if generator.is_async {
            return self.compile_async_generator_expression_loop(
                generators,
                index,
                element,
                generator_location,
            );
        }
        if self.context.python_version == PythonVersion::Py313 {
            self.assembler
                .set_location(self.source_location(generator.iter.range()));
            self.emit_comprehension_iterator(false)?;
        }
        let loop_depth = self.control.stack.depth;
        let start = self.assembler.label();
        let filter_cleanup = if self.context.python_version <= PythonVersion::Py313 {
            self.assembler.label()
        } else {
            start
        };
        let cleanup = self.assembler.label();
        self.assembler.mark(start);
        self.assembler
            .set_location(self.source_location(generator.iter.range()));
        self.emit_jump_forward(FOR_ITER, cleanup, 1)?;
        self.compile_store_target(&generator.target)?;
        for condition in &generator.ifs {
            let accepted = self.assembler.label();
            self.compile_generator_filter(condition, element, accepted, filter_cleanup)?;
            self.assembler.mark(accepted);
        }
        self.compile_generator_expression_tail(
            generators,
            index,
            element,
            generator_location,
            filter_cleanup,
        )?;
        if self.context.python_version <= PythonVersion::Py313 {
            // Keep CPython's original rejection block until jump threading and
            // backward-conditional normalization run in the assembler.
            self.assembler.preserve_block_boundary(filter_cleanup);
            self.assembler.mark(filter_cleanup);
        }
        self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
        self.assembler.mark(cleanup);
        self.set_depth(loop_depth + 1);
        self.assembler
            .set_location(self.source_location(generator.iter.range()));
        self.emit_end_for_cleanup()?;
        Ok(())
    }

    fn compile_async_generator_expression_loop(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        index: usize,
        element: &Expr,
        generator_location: SourceLocation,
    ) -> Result<(), CompileError> {
        let generator = &generators[index];
        let loop_depth = self.control.stack.depth;
        let cleanup_location = generator_location;
        let start = self.assembler.label();
        let filter_cleanup = if self.context.python_version <= PythonVersion::Py313 {
            self.assembler.label()
        } else {
            start
        };
        let protected_start = self.assembler.label();
        let yielded = self.assembler.label();
        let yielded_end = self.assembler.label();
        let send = self.assembler.label();
        let send_end = self.assembler.label();
        let protected_end = self.assembler.label();
        let cleanup_throw = self.assembler.label();
        let async_cleanup = self.assembler.label();

        self.assembler.set_location(generator_location);
        self.assembler.mark(start);
        self.assembler
            .record_exception_setup(protected_start, false);
        self.assembler.mark(protected_start);
        self.emit(GET_ANEXT, 0, 1)?;
        if self.context.python_version == PythonVersion::Py315 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.assembler.mark(send);
        self.emit_jump_forward(SEND, send_end, 0)?;
        self.assembler.record_exception_setup(yielded, false);
        self.assembler.mark(yielded);
        self.emit_yield_value(1, 0)?;
        self.assembler.mark(yielded_end);
        self.emit_resume(ResumeLocation::AFTER_AWAIT, ResumeDepth::Zero)?;
        self.emit_jump_backward(JUMP_BACKWARD_NO_INTERRUPT, send, 0)?;
        self.assembler.mark(cleanup_throw);
        self.set_depth(loop_depth + 3 + self.iterator_extra_slots());
        self.emit(CLEANUP_THROW, 0, -1)?;
        let cleanup_throw_end = self.assembler.label();
        self.assembler.mark(cleanup_throw_end);
        self.control
            .generators
            .generator_region_exclusions
            .push((cleanup_throw, cleanup_throw_end));
        self.assembler.mark(send_end);
        self.set_depth(loop_depth + 2 + self.iterator_extra_slots());
        self.emit(END_SEND, 0, -self.iterator_slots())?;
        if self.context.python_version == PythonVersion::Py314 {
            // Native compaction erases the POP_BLOCK between END_SEND and target storage.
            // Its vacant record remains in the original array used by later markers.
            self.assembler.add_last_hidden_exception_owner_slots(1);
        }
        self.assembler.mark(protected_end);
        self.compile_store_target(&generator.target)?;
        if self.context.python_version == PythonVersion::Py314 {
            self.assembler
                .transfer_linear_try_pop_block_slot(protected_end);
        }

        for condition in &generator.ifs {
            let accepted = self.assembler.label();
            self.compile_generator_filter(condition, element, accepted, filter_cleanup)?;
            self.assembler.mark(accepted);
        }
        self.compile_generator_expression_tail(
            generators,
            index,
            element,
            generator_location,
            filter_cleanup,
        )?;
        if self.context.python_version <= PythonVersion::Py313 {
            // Keep CPython's original rejection block until jump threading and
            // backward-conditional normalization run in the assembler.
            self.assembler.preserve_block_boundary(filter_cleanup);
            self.assembler.mark(filter_cleanup);
        }
        self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
        self.assembler.set_location(cleanup_location);

        self.assembler.add_exception_region(
            yielded,
            yielded_end,
            cleanup_throw,
            (loop_depth + 2 + self.iterator_extra_slots()).cast_unsigned(),
            false,
        );
        self.assembler.mark(async_cleanup);
        self.set_depth(loop_depth + 1);
        self.apply_compiler_stack_effect(END_ASYNC_FOR, 0, -2)?;
        if self.context.python_version <= PythonVersion::Py313 {
            self.assembler.emit(END_ASYNC_FOR, 0);
        } else {
            self.assembler.emit_backward(END_ASYNC_FOR, send);
        }
        self.assembler.add_exception_region(
            protected_start,
            protected_end,
            async_cleanup,
            loop_depth.cast_unsigned(),
            false,
        );
        Ok(())
    }

    /// Preserves the tuple-build/unpack optimization in an elided singleton clause.
    fn compile_singleton_comprehension_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> Result<(), CompileError> {
        let previous_location = self.assembler.location();
        let targets = match target {
            Expr::Tuple(target) => Some(target.elts.as_slice()),
            Expr::List(target) => Some(target.elts.as_slice()),
            _ => None,
        };
        if let Some(targets) = targets
            && let Expr::Tuple(tuple) = value
            && targets.len() == tuple.elts.len()
            && (1..=3).contains(&targets.len())
            && !targets.iter().any(Expr::is_starred_expr)
            && !tuple.elts.iter().any(Expr::is_starred_expr)
        {
            // CPython removes BUILD_TUPLE followed by matching UNPACK_SEQUENCE
            // before folding a tuple of constants. Evaluate each value independently.
            for value in &tuple.elts {
                self.compile_expression(value)?;
            }
            // Removed tuple/unpack operations still carry line events. Leave an
            // explicit NOP when the previous line differs; the assembler can remove
            // it once the actual next target instruction has the same line.
            let tuple_location = self.source_location(value.range());
            let target_location = self.source_location(target.range());
            let mut preceding_location = self.assembler.current_block_last_instruction_location();
            let retain_tuple_nop =
                preceding_location.is_none_or(|location| location.line != tuple_location.line);
            let mut removed_slots = usize::from(!retain_tuple_nop);
            if retain_tuple_nop {
                self.assembler.set_location(tuple_location);
                self.emit(NOP, 0, 0)?;
                if tuple_location.line != target_location.line {
                    preceding_location = Some(tuple_location);
                }
            }
            self.assembler.set_location(target_location);
            if targets.len() > 1 {
                self.emit(SWAP, to_u32(targets.len(), "unpack target count")?, 0)?;
                self.assembler.add_last_constant_fold_slots(removed_slots);
                removed_slots = 0;
            } else if preceding_location
                .is_none_or(|location| location.line != target_location.line)
            {
                self.emit(NOP, 0, 0)?;
                self.assembler.add_last_constant_fold_slots(removed_slots);
                removed_slots = 0;
            } else {
                removed_slots += 1;
            }
            let target_start = self.assembler.item_count();
            for target in targets {
                self.compile_store_target(target)?;
            }
            self.assembler
                .add_constant_fold_slots_at(target_start, removed_slots);
        } else {
            self.compile_expression(value)?;
            self.compile_store_target(target)?;
        }
        self.assembler.set_location(previous_location);
        Ok(())
    }

    /// Emits the remaining clauses and yielded value inside a generator-expression loop.
    fn compile_generator_expression_tail(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        index: usize,
        element: &Expr,
        generator_location: SourceLocation,
        restart: Label,
    ) -> Result<(), CompileError> {
        if let Some(nested) = generators.get(index + 1) {
            let singleton = match &nested.iter {
                Expr::List(list) => match list.elts.as_slice() {
                    [element] if !matches!(element, Expr::Starred(_)) => Some(element),
                    _ => None,
                },
                Expr::Tuple(tuple) => match tuple.elts.as_slice() {
                    [element] if !matches!(element, Expr::Starred(_)) => Some(element),
                    _ => None,
                },
                _ => None,
            };
            if !nested.is_async
                && let Some(singleton) = singleton
                && (self.context.python_version >= PythonVersion::Py314
                    || self.source_aware_fold_constant(singleton).is_none())
            {
                // CPython treats `for value in [expression]` as an assignment. Filters on
                // that clause continue the enclosing real loop when they reject the value.
                self.compile_singleton_comprehension_assignment(&nested.target, singleton)?;
                for condition in &nested.ifs {
                    let accepted = self.assembler.label();
                    self.compile_generator_filter(condition, element, accepted, restart)?;
                    self.assembler.mark(accepted);
                }
                self.compile_generator_expression_tail(
                    generators,
                    index + 1,
                    element,
                    generator_location,
                    restart,
                )?;
            } else {
                if nested.is_async && self.context.python_version >= PythonVersion::Py314 {
                    self.compile_expression(&nested.iter)?;
                } else {
                    self.compile_iterable_expression(&nested.iter)?;
                }
                self.assembler
                    .set_location(self.source_location(nested.iter.range()));
                self.emit_comprehension_iterator(nested.is_async)?;
                self.compile_generator_expression_loop(
                    generators,
                    index + 1,
                    element,
                    generator_location,
                )?;
            }
        } else if self.context.python_version == PythonVersion::Py315
            && let Expr::Starred(starred) = element
        {
            self.compile_iterable_expression(&starred.value)?;
            self.assembler
                .set_location(self.source_location(element.range()));
            self.emit_comprehension_iterator(false)?;
            let loop_depth = self.control.stack.depth;
            let start = self.assembler.label();
            let end = self.assembler.label();
            self.assembler.mark(start);
            self.emit_jump_forward(FOR_ITER, end, 1)?;
            if self.unit.flags & CO_ASYNC_GENERATOR != 0 {
                self.emit_unary_intrinsic(UnaryIntrinsic::ASYNC_GEN_WRAP)?;
            }
            self.emit_yield_value(0, 0)?;
            self.emit_resume(ResumeLocation::AFTER_YIELD, ResumeDepth::One)?;
            self.emit(POP_TOP, 0, -1)?;
            self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
            self.assembler.mark(end);
            self.set_depth(loop_depth + 1);
            self.emit_end_for_cleanup()?;
        } else {
            self.compile_expression(element)?;
            self.assembler
                .set_location(self.source_location(element.range()));
            if self.unit.flags & CO_ASYNC_GENERATOR != 0 {
                self.emit_unary_intrinsic(UnaryIntrinsic::ASYNC_GEN_WRAP)?;
            }
            self.emit_yield_value(0, 0)?;
            self.emit_resume(ResumeLocation::AFTER_YIELD, ResumeDepth::One)?;
            self.emit(POP_TOP, 0, -1)?;
        }
        self.assembler
            .set_location(self.source_location(element.range()));
        Ok(())
    }

    fn emit_multiline_none_filter_nops(
        &mut self,
        comparison: &ruff_python_ast::ExprCompare,
        element_location: SourceLocation,
    ) -> Result<(), CompileError> {
        let left_location = self.source_location(comparison.left.range());
        let none_location = self.source_location(comparison.comparators[0].range());
        if left_location.line == none_location.line {
            let comparison_location = self.source_location(comparison.range);
            if element_location.line != comparison_location.line
                && self.assembler.at_empty_borrow_block()
                && self
                    .assembler
                    .current_block_last_instruction_location()
                    .is_none()
                && self.assembler.current_block_starts_at_source_boundary()
            {
                // None specialization leaves a comparison-line NOP at an empty original
                // join. The following filter jump carries the distinct element line.
                self.assembler.set_location(comparison_location);
                self.emit(NOP, 0, 0)?;
            }
            return Ok(());
        }

        // The filter jump carries the element's location. Preserve the two source transitions
        // that CPython emits while specializing a multiline `is [not] None` comparison before
        // that jump: first the `None` operand and then the whole comparison.
        self.assembler.set_location(none_location);
        self.emit(NOP, 0, 0)?;
        self.assembler
            .set_location(self.source_location(comparison.range));
        self.emit(NOP, 0, 0)
    }

    fn compile_generator_filter(
        &mut self,
        condition: &Expr,
        element: &Expr,
        accepted: Label,
        restart: Label,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            return self.compile_jump_if(condition, false, restart);
        }
        if let Expr::If(conditional) = condition {
            return self.compile_conditional_filter(
                conditional,
                accepted,
                |compiler, arm, accepted| {
                    compiler.compile_generator_filter(arm, element, accepted, restart)
                },
            );
        }
        if early_condition_truthiness(condition) == Some(false) {
            self.record_folded_value(condition)?;
            let condition_location = self.source_location(condition.range());
            if condition_location.line != self.source_location(element.range()).line {
                self.assembler.set_location(condition_location);
                self.emit(NOP, 0, 0)?;
            }
            let start = self.assembler.label();
            self.assembler.mark(start);
            self.assembler
                .set_location(self.source_location(element.range()));
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            let end = self.assembler.label();
            self.assembler.mark(end);
            self.record_filter_exclusion(start, end, false, false);
            return Ok(());
        }
        if early_condition_truthiness(condition) == Some(true) {
            self.record_folded_value(condition)?;
            if matches!(condition, Expr::BooleanLiteral(_)) {
                // The removed filter jump still separates the original instruction arrays.
                self.assembler
                    .record_exception_owner_block_boundary(accepted);
                self.assembler
                    .set_location(self.source_location(condition.range()));
                self.emit(NOP, 0, 0)?;
            }
            return Ok(());
        }
        if let Expr::BoolOp(boolean) = condition
            && boolean.op == BoolOp::And
        {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            for value in leading {
                let next = self.assembler.label();
                self.compile_generator_filter(value, element, next, restart)?;
                self.assembler.mark(next);
            }
            return self.compile_generator_filter(last, element, accepted, restart);
        }
        if let Expr::BoolOp(boolean) = condition
            && boolean.op == BoolOp::Or
        {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            for value in leading {
                self.compile_jump_if(value, true, accepted)?;
            }
            return self.compile_generator_filter(last, element, accepted, restart);
        }
        if let Expr::UnaryOp(unary) = condition
            && unary.op == UnaryOp::Not
        {
            return self.compile_filter_condition(
                &unary.operand,
                self.source_location(element.range()),
                (accepted, restart),
                false,
                false,
                true,
            );
        }
        if let Expr::Compare(comparison) = condition
            && comparison.ops.len() > 1
        {
            return self.compile_chained_comparison_filter(
                comparison,
                self.source_location(element.range()),
                (accepted, restart),
                false,
                false,
                false,
            );
        }
        if let Expr::Compare(comparison) = condition
            && comparison.ops.len() == 1
            && matches!(comparison.comparators[0], Expr::NoneLiteral(_))
            && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
        {
            self.compile_expression(&comparison.left)?;
            self.add_constant(Constant::None)?;
            self.emit_multiline_none_filter_nops(
                comparison,
                self.source_location(element.range()),
            )?;
            let exclusion_start = self.assembler.label();
            self.assembler.mark(exclusion_start);
            self.assembler
                .set_location(self.source_location(element.range()));
            self.emit_jump_forward(
                if comparison.ops[0] == CmpOp::Is {
                    POP_JUMP_IF_NONE
                } else {
                    POP_JUMP_IF_NOT_NONE
                },
                accepted,
                -1,
            )?;
            self.emit(NOT_TAKEN, 0, 0)?;
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            self.control
                .generators
                .generator_region_exclusions
                .push((exclusion_start, exclusion_end));
            return Ok(());
        }
        self.with_control_override(
            |control| &mut control.generators.generator_filter_expression,
            true,
            |compiler| compiler.compile_expression(condition),
        )?;
        self.assembler
            .set_location(self.source_location(condition.range()));
        self.emit(TO_BOOL, 0, 0)?;
        let exclusion_start = self.assembler.label();
        self.assembler.mark(exclusion_start);
        self.assembler
            .set_location(self.source_location(element.range()));
        self.emit_jump_forward(POP_JUMP_IF_TRUE, accepted, -1)?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        let exclusion_end = self.assembler.label();
        self.assembler.mark(exclusion_end);
        self.control
            .generators
            .generator_region_exclusions
            .push((exclusion_start, exclusion_end));
        Ok(())
    }

    fn compile_filter_condition(
        &mut self,
        condition: &Expr,
        element_location: SourceLocation,
        (accepted, restart): (Label, Label),
        include_comprehension_cleanup: bool,
        include_protected_region_exclusion: bool,
        negated: bool,
    ) -> Result<(), CompileError> {
        // CPython's codegen_jump_if preserves branch lowering through every nested NOT
        // and Boolean operand; evaluating a nested operand as a value leaves extra copies.
        if let Expr::UnaryOp(unary) = condition
            && unary.op == UnaryOp::Not
        {
            return self.compile_filter_condition(
                &unary.operand,
                element_location,
                (accepted, restart),
                include_comprehension_cleanup,
                include_protected_region_exclusion,
                !negated,
            );
        }
        if let Expr::BoolOp(boolean) = condition
            && (boolean.op == BoolOp::Or) == negated
        {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            for value in leading {
                let next = self.assembler.label();
                self.compile_filter_condition(
                    value,
                    element_location,
                    (next, restart),
                    include_comprehension_cleanup,
                    include_protected_region_exclusion,
                    negated,
                )?;
                self.assembler.mark(next);
            }
            let empty_folded_tail = jump_constant_truthiness(last)
                .is_some_and(|truthiness| truthiness != negated)
                && self.assembler.at_empty_borrow_block();
            self.compile_filter_condition(
                last,
                element_location,
                (accepted, restart),
                include_comprehension_cleanup,
                include_protected_region_exclusion,
                negated,
            )?;
            if empty_folded_tail {
                // The last Boolean operand has its own original fallthrough block.
                // Removing its folded condition stops borrowing through that block.
                self.assembler.mark_last_nop_borrow_barrier_if_removed();
            }
            return Ok(());
        }
        if let Expr::BoolOp(boolean) = condition
            && (boolean.op == BoolOp::And) == negated
        {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            for value in leading {
                self.compile_jump_if(value, !negated, accepted)?;
            }
            return self.compile_filter_condition(
                last,
                element_location,
                (accepted, restart),
                include_comprehension_cleanup,
                include_protected_region_exclusion,
                negated,
            );
        }
        self.compile_filter_operand(
            condition,
            element_location,
            (accepted, restart),
            include_comprehension_cleanup,
            include_protected_region_exclusion,
            negated,
        )
    }

    fn compile_filter_operand(
        &mut self,
        operand: &Expr,
        element_location: SourceLocation,
        (accepted, restart): (Label, Label),
        include_comprehension_cleanup: bool,
        include_protected_region_exclusion: bool,
        negated: bool,
    ) -> Result<(), CompileError> {
        // A literal reached through a negated or conditional filter still folds as a
        // branch. Only the rejected edge is normalized into an unprotected backedge.
        if let Some(truthiness) = jump_constant_truthiness(operand) {
            self.record_folded_value(operand)?;
            let operand_location = self.source_location(operand.range());
            if truthiness != negated {
                self.assembler
                    .record_exception_owner_block_boundary(accepted);
                self.assembler.set_location(operand_location);
                self.emit(NOP, 0, 0)?;
            } else {
                if operand_location.line != element_location.line {
                    self.assembler.set_location(operand_location);
                    self.emit(NOP, 0, 0)?;
                }
                let start = self.assembler.label();
                self.assembler.mark(start);
                self.assembler.set_location(element_location);
                self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
                let end = self.assembler.label();
                self.assembler.mark(end);
                self.record_filter_exclusion(
                    start,
                    end,
                    include_comprehension_cleanup,
                    include_protected_region_exclusion,
                );
            }
            return Ok(());
        }
        if let Expr::If(conditional) = operand {
            return self.compile_conditional_filter(
                conditional,
                accepted,
                |compiler, arm, accepted| {
                    compiler.compile_filter_condition(
                        arm,
                        element_location,
                        (accepted, restart),
                        include_comprehension_cleanup,
                        include_protected_region_exclusion,
                        negated,
                    )
                },
            );
        }
        if let Expr::Compare(comparison) = operand
            && comparison.ops.len() > 1
        {
            return self.compile_chained_comparison_filter(
                comparison,
                element_location,
                (accepted, restart),
                include_comprehension_cleanup,
                include_protected_region_exclusion,
                negated,
            );
        }
        if let Expr::Compare(comparison) = operand
            && comparison.ops.len() == 1
            && matches!(comparison.comparators[0], Expr::NoneLiteral(_))
            && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
        {
            self.compile_expression(&comparison.left)?;
            self.add_constant(Constant::None)?;
            self.emit_multiline_none_filter_nops(comparison, element_location)?;
            let exclusion_start = self.assembler.label();
            self.assembler.mark(exclusion_start);
            self.assembler.set_location(element_location);
            self.emit_jump_forward(
                if (comparison.ops[0] == CmpOp::Is) == negated {
                    POP_JUMP_IF_NOT_NONE
                } else {
                    POP_JUMP_IF_NONE
                },
                accepted,
                -1,
            )?;
            self.emit(NOT_TAKEN, 0, 0)?;
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            self.record_filter_exclusion(
                exclusion_start,
                exclusion_end,
                include_comprehension_cleanup,
                include_protected_region_exclusion,
            );
            return Ok(());
        }
        if !self
            .control
            .comprehensions
            .suppress_filtered_comprehension_none_owner
            && comprehension_filter_has_root_if_expression(operand)
        {
            self.with_control_override(
                |control| {
                    &mut control
                        .comprehensions
                        .suppress_filtered_comprehension_none_owner
                },
                true,
                |compiler| {
                    compiler.with_control_override(
                        |control| &mut control.generators.generator_filter_expression,
                        true,
                        |compiler| compiler.compile_expression(operand),
                    )
                },
            )?;
        } else {
            self.with_control_override(
                |control| &mut control.generators.generator_filter_expression,
                true,
                |compiler| compiler.compile_expression(operand),
            )?;
        }
        self.assembler
            .set_location(self.source_location(operand.range()));
        self.emit(TO_BOOL, 0, 0)?;
        let exclusion_start = self.assembler.label();
        self.assembler.mark(exclusion_start);
        self.assembler.set_location(element_location);
        self.emit_jump_forward(
            if negated {
                POP_JUMP_IF_FALSE
            } else {
                POP_JUMP_IF_TRUE
            },
            accepted,
            -1,
        )?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        let exclusion_end = self.assembler.label();
        self.assembler.mark(exclusion_end);
        self.record_filter_exclusion(
            exclusion_start,
            exclusion_end,
            include_comprehension_cleanup,
            include_protected_region_exclusion,
        );
        Ok(())
    }

    fn compile_chained_comparison_filter(
        &mut self,
        comparison: &ruff_python_ast::ExprCompare,
        element_location: SourceLocation,
        (accepted, restart): (Label, Label),
        include_comprehension_cleanup: bool,
        include_protected_region_exclusion: bool,
        negated: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.stack.depth;
        let cleanup = self.assembler.label();
        let residue = self.assembler.label();
        let comparison_location = self.source_location(comparison.range);
        self.compile_conditional_comparison_prefix(comparison, cleanup)?;

        let last = comparison.comparators.last().unwrap();
        let operator = *comparison.ops.last().unwrap();
        let none_jump =
            matches!(last, Expr::NoneLiteral(_)) && matches!(operator, CmpOp::Is | CmpOp::IsNot);
        let jump = if none_jump {
            self.add_constant(Constant::None)?;
            let none_location = self.source_location(last.range());
            if none_location.line != comparison_location.line {
                self.assembler.set_location(none_location);
                self.emit(NOP, 0, 0)?;
                self.assembler.set_location(comparison_location);
                self.emit(NOP, 0, 0)?;
            }
            if (operator == CmpOp::Is) == negated {
                POP_JUMP_IF_NOT_NONE
            } else {
                POP_JUMP_IF_NONE
            }
        } else {
            self.compile_comparison_rhs(operator, last)?;
            self.assembler.set_location(comparison_location);
            self.emit_boolean_comparison(operator)?;
            if negated {
                POP_JUMP_IF_FALSE
            } else {
                POP_JUMP_IF_TRUE
            }
        };
        let terminal_exclusion_start = self.assembler.label();
        self.assembler.mark(terminal_exclusion_start);
        self.assembler.set_location(element_location);
        self.emit_jump_forward(jump, residue, -1)?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        let terminal_exclusion_end = self.assembler.label();
        self.assembler.mark(terminal_exclusion_end);

        self.assembler.mark(residue);
        self.assembler.set_location(comparison_location);
        self.emit_jump_forward(JUMP_FORWARD, accepted, 0)?;

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 1);
        self.emit(POP_TOP, 0, -1)?;
        self.set_depth(base_depth);

        self.record_filter_exclusion(
            terminal_exclusion_start,
            terminal_exclusion_end,
            include_comprehension_cleanup,
            include_protected_region_exclusion,
        );
        if !negated {
            let cleanup_exclusion_start = self.assembler.label();
            self.assembler.mark(cleanup_exclusion_start);
            self.assembler.set_location(element_location);
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            let cleanup_exclusion_end = self.assembler.label();
            self.assembler.mark(cleanup_exclusion_end);
            self.record_filter_exclusion(
                cleanup_exclusion_start,
                cleanup_exclusion_end,
                include_comprehension_cleanup,
                include_protected_region_exclusion,
            );
        }
        Ok(())
    }

    fn record_filter_exclusion(
        &mut self,
        start: Label,
        end: Label,
        include_comprehension_cleanup: bool,
        include_protected_region_exclusion: bool,
    ) {
        self.control
            .generators
            .generator_region_exclusions
            .push((start, end));
        if include_comprehension_cleanup {
            for exclusions in &mut self
                .control
                .comprehensions
                .active_comprehension_region_exclusions
            {
                exclusions.push((start, end));
            }
        }
        // Backward-branch normalization creates the filter-rejection block after
        // CPython assigns exception handlers, leaving it outside every enclosing try/with
        // region. This also applies after suspension and inside copied finally bodies.
        if include_protected_region_exclusion
            && (!self
                .control
                .exceptions
                .active_exception_region_exclusions
                .is_empty()
                || !self.control.withs.active_with_region_exclusions.is_empty())
        {
            for exclusions in &mut self.control.exceptions.active_exception_region_exclusions {
                exclusions.push((start, end));
            }
            for exclusions in &mut self.control.withs.active_with_region_exclusions {
                exclusions.push((start, end));
            }
        }
    }

    /// Lowers both arms as filter branches, preserving the first arm's exit location.
    pub(super) fn compile_conditional_filter(
        &mut self,
        conditional: &ruff_python_ast::ExprIf,
        accepted: Label,
        mut compile_arm: impl FnMut(&mut Self, &Expr, Label) -> Result<(), CompileError>,
    ) -> Result<(), CompileError> {
        let otherwise = self.assembler.label();
        let body_accepted = self.assembler.label();
        self.assembler.preserve_block_boundary(otherwise);
        self.assembler.preserve_block_boundary(body_accepted);
        self.assembler.preserve_block_boundary(accepted);
        self.compile_jump_if(&conditional.test, false, otherwise)?;
        compile_arm(self, &conditional.body, body_accepted)?;
        self.assembler.mark(body_accepted);
        let shared_body_exit = self.assembler.has_multiple_jump_sources(body_accepted);
        let exit_location = if shared_body_exit {
            SourceLocation::NONE
        } else {
            let range = if comprehension_filter_truthiness(&conditional.body).is_none() {
                // Branch lowering strips transparent `not`; the operand owns this exit.
                synthetic_condition_exit_range(&conditional.body)
            } else {
                conditional.body.range()
            };
            self.source_location(range)
        };
        self.assembler.set_location(exit_location);
        let body_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, accepted)?;
        // This is the original conditional-expression join, even when its rejected
        // arm has already become a backward condition. Retain its slot when a
        // small accepted exit is copied before native line-number cleanup.
        self.assembler.preserve_direct_inlined_jump_nop(body_exit);
        if shared_body_exit {
            self.assembler.preserve_last_no_location();
        }
        self.assembler.mark(otherwise);
        compile_arm(self, &conditional.orelse, accepted)
    }

    fn compile_comprehension_filter(
        &mut self,
        condition: &Expr,
        element_location: SourceLocation,
        accepted: Label,
        restart: Label,
        include_with_region_exclusion: bool,
        include_protected_region_exclusion: bool,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            return self.compile_jump_if(condition, false, restart);
        }
        if let Expr::If(conditional) = condition {
            return self.compile_conditional_filter(
                conditional,
                accepted,
                |compiler, arm, accepted| {
                    compiler.compile_comprehension_filter(
                        arm,
                        element_location,
                        accepted,
                        restart,
                        include_with_region_exclusion,
                        include_protected_region_exclusion,
                    )
                },
            );
        }
        if comprehension_filter_truthiness(condition) == Some(false) {
            self.record_folded_value(condition)?;
            let condition_location = self.source_location(condition.range());
            if condition_location.line != element_location.line {
                self.assembler.set_location(condition_location);
                self.emit(NOP, 0, 0)?;
            }
            let start = self.assembler.label();
            self.assembler.mark(start);
            self.assembler.set_location(element_location);
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            let end = self.assembler.label();
            self.assembler.mark(end);
            self.record_filter_exclusion(start, end, true, include_protected_region_exclusion);
            return Ok(());
        }
        if comprehension_filter_truthiness(condition) == Some(true) {
            // The removed filter jump still separates the original instruction arrays.
            self.assembler
                .record_exception_owner_block_boundary(accepted);
            self.record_folded_value(condition)?;
            if matches!(condition, Expr::UnaryOp(unary) if unary.op == UnaryOp::Not) {
                self.emit_folded_tuple_not_nops(condition)?;
            } else {
                self.assembler
                    .set_location(self.source_location(condition.range()));
                self.emit(NOP, 0, 0)?;
            }
            return Ok(());
        }
        if let Expr::BoolOp(boolean) = condition
            && boolean.op == BoolOp::And
        {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            for value in leading {
                let next = self.assembler.label();
                self.compile_comprehension_filter(
                    value,
                    element_location,
                    next,
                    restart,
                    include_with_region_exclusion,
                    include_protected_region_exclusion,
                )?;
                self.assembler.mark(next);
            }
            return self.compile_comprehension_filter(
                last,
                element_location,
                accepted,
                restart,
                include_with_region_exclusion,
                include_protected_region_exclusion,
            );
        }
        if let Expr::BoolOp(boolean) = condition
            && boolean.op == BoolOp::Or
        {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            // A source-line split prevents the comprehension target load from fusing; CPython
            // keeps the leading specialized None-condition marker inside the cleanup region.
            let preserve_multiline_list_filter_none_owner = include_with_region_exclusion
                && include_protected_region_exclusion
                && matches!(self.unit.scope, Scope::Function { .. })
                && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) == 0
                && self.control.generators.generator_region_start.is_none()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self.control.withs.active_with_region_exclusions.is_empty()
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .len()
                    == 1
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .last()
                    .is_some_and(Vec::is_empty)
                && matches!(leading, [Expr::Compare(comparison)]
                    if comparison.ops.len() == 1
                        && comparison.ops[0] == CmpOp::Is
                        && matches!(comparison.left.as_ref(), Expr::Name(_) | Expr::Attribute(_))
                        && comparison.comparators.len() == 1
                        && matches!(comparison.comparators[0], Expr::NoneLiteral(_)))
                && (matches!(last, Expr::Call(call) if matches!(call.func.as_ref(), Expr::Attribute(_)))
                    || matches!(last, Expr::Compare(_)))
                && self.assembler.location().line < self.source_location(leading[0].range()).line;
            for value in leading {
                if preserve_multiline_list_filter_none_owner {
                    self.with_control_override(
                        |control| &mut control.conditions.include_scoped_none_condition_owner,
                        true,
                        |compiler| compiler.compile_jump_if(value, true, accepted),
                    )?;
                } else {
                    self.compile_jump_if(value, true, accepted)?;
                }
            }
            return self.compile_comprehension_filter(
                last,
                element_location,
                accepted,
                restart,
                include_with_region_exclusion,
                include_protected_region_exclusion,
            );
        }
        if let Expr::UnaryOp(unary) = condition
            && unary.op == UnaryOp::Not
        {
            return self.compile_filter_condition(
                &unary.operand,
                element_location,
                (accepted, restart),
                true,
                include_protected_region_exclusion,
                true,
            );
        }
        if let Expr::Compare(comparison) = condition
            && comparison.ops.len() > 1
        {
            return self.compile_chained_comparison_filter(
                comparison,
                element_location,
                (accepted, restart),
                true,
                include_protected_region_exclusion,
                false,
            );
        }
        if let Expr::Compare(comparison) = condition
            && comparison.ops.len() == 1
            && matches!(comparison.comparators[0], Expr::NoneLiteral(_))
            && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
        {
            self.compile_expression(&comparison.left)?;
            self.add_constant(Constant::None)?;
            self.emit_multiline_none_filter_nops(comparison, element_location)?;
            let exclusion_start = self.assembler.label();
            self.assembler.mark(exclusion_start);
            self.assembler.set_location(element_location);
            self.emit_jump_forward(
                if comparison.ops[0] == CmpOp::Is {
                    POP_JUMP_IF_NONE
                } else {
                    POP_JUMP_IF_NOT_NONE
                },
                accepted,
                -1,
            )?;
            self.emit(NOT_TAKEN, 0, 0)?;
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            self.record_filter_exclusion(
                exclusion_start,
                exclusion_end,
                true,
                include_protected_region_exclusion,
            );
            if include_with_region_exclusion
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
            {
                for exclusions in &mut self.control.withs.active_with_region_exclusions {
                    exclusions.push((exclusion_start, exclusion_end));
                }
            }
            return Ok(());
        }
        if !self
            .control
            .comprehensions
            .suppress_filtered_comprehension_none_owner
            && comprehension_filter_has_root_if_expression(condition)
        {
            self.with_control_override(
                |control| {
                    &mut control
                        .comprehensions
                        .suppress_filtered_comprehension_none_owner
                },
                true,
                |compiler| compiler.compile_expression(condition),
            )?;
        } else {
            self.compile_expression(condition)?;
        }
        self.assembler
            .set_location(self.source_location(condition.range()));
        self.emit(TO_BOOL, 0, 0)?;
        let exclusion_start = self.assembler.label();
        self.assembler.mark(exclusion_start);
        self.assembler.set_location(element_location);
        self.emit_jump_forward(POP_JUMP_IF_TRUE, accepted, -1)?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        let exclusion_end = self.assembler.label();
        self.assembler.mark(exclusion_end);
        self.record_filter_exclusion(
            exclusion_start,
            exclusion_end,
            true,
            include_protected_region_exclusion,
        );
        if include_with_region_exclusion
            && self
                .control
                .exceptions
                .active_exception_region_exclusions
                .is_empty()
        {
            for exclusions in &mut self.control.withs.active_with_region_exclusions {
                exclusions.push((exclusion_start, exclusion_end));
            }
        }
        Ok(())
    }

    /// Initializes an iterator and the target's optional synchronous index slot.
    fn emit_comprehension_iterator(&mut self, is_async: bool) -> Result<(), CompileError> {
        self.emit(
            if is_async { GET_AITER } else { GET_ITER },
            0,
            if is_async {
                0
            } else {
                self.iterator_extra_slots()
            },
        )
    }

    fn compile_comprehension_generator(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        index: usize,
        key: Option<&Expr>,
        value: &Expr,
        add_opcode: Opcode,
        iterator_depth: usize,
    ) -> Result<(), CompileError> {
        let generator = &generators[index];
        if generator.is_async {
            return self.compile_async_comprehension_generator(
                generators,
                index,
                key,
                value,
                add_opcode,
                iterator_depth,
            );
        }
        let comprehension_location = self.assembler.location();
        if self.context.python_version == PythonVersion::Py313
            || (index == 0 && self.context.python_version == PythonVersion::Py315)
        {
            self.assembler
                .set_location(self.source_location(generator.iter.range()));
            self.emit_comprehension_iterator(false)?;
            self.assembler.set_location(comprehension_location);
        }
        let loop_depth = self.control.stack.depth;
        let start = self.assembler.label();
        let filter_cleanup = if self.context.python_version <= PythonVersion::Py313 {
            self.assembler.label()
        } else {
            start
        };
        let cleanup = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ComprehensionStart(generator.iter.range()),
            start,
        );
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ComprehensionIfCleanup(generator.iter.range()),
            filter_cleanup,
        );
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ComprehensionAnchor(generator.iter.range()),
            cleanup,
        );
        self.assembler.mark(start);
        self.assembler
            .set_location(self.source_location(generator.iter.range()));
        self.emit_jump_forward(FOR_ITER, cleanup, 1)?;
        self.compile_store_target(&generator.target)?;

        for condition in &generator.ifs {
            let accepted = self.assembler.label();
            // A rejection inherits its loop's backedge location. Only the final loop
            // appends the dictionary item and extends that location through the value.
            let element_range = if index + 1 < generators.len() {
                key.unwrap_or(value).range()
            } else {
                key.map_or_else(
                    || value.range(),
                    |key| ruff_text_size::TextRange::new(key.range().start(), value.range().end()),
                )
            };
            self.compile_comprehension_filter(
                condition,
                self.source_location(element_range),
                accepted,
                filter_cleanup,
                add_opcode == LIST_APPEND,
                true,
            )?;
            self.assembler.mark(accepted);
        }

        self.compile_comprehension_tail(
            generators,
            index,
            key,
            value,
            add_opcode,
            ComprehensionLoop {
                restart: filter_cleanup,
                iterator_depth: iterator_depth + usize::try_from(self.iterator_slots()).unwrap(),
                comprehension_location,
            },
        )?;
        if self.context.python_version <= PythonVersion::Py313 {
            // Keep CPython's original rejection block until jump threading and
            // backward-conditional normalization run in the assembler.
            self.assembler.preserve_block_boundary(filter_cleanup);
            self.assembler.mark(filter_cleanup);
        }
        self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
        self.assembler.mark(cleanup);
        self.set_depth(loop_depth + 1);
        self.assembler
            .set_location(self.source_location(generator.iter.range()));
        self.emit_end_for_cleanup()?;
        Ok(())
    }

    /// Emits nested clauses while counting only iterators that remain on the stack.
    fn compile_comprehension_tail(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        index: usize,
        key: Option<&Expr>,
        value: &Expr,
        add_opcode: Opcode,
        loop_context: ComprehensionLoop,
    ) -> Result<(), CompileError> {
        if let Some(nested) = generators.get(index + 1) {
            let singleton = match &nested.iter {
                Expr::List(list) => match list.elts.as_slice() {
                    [element] if !matches!(element, Expr::Starred(_)) => Some(element),
                    _ => None,
                },
                Expr::Tuple(tuple) => match tuple.elts.as_slice() {
                    [element] if !matches!(element, Expr::Starred(_)) => Some(element),
                    _ => None,
                },
                _ => None,
            };
            if !nested.is_async
                && let Some(singleton) = singleton
                && (self.context.python_version >= PythonVersion::Py314
                    || self.source_aware_fold_constant(singleton).is_none())
            {
                // A singleton clause is an assignment; rejected filters continue the
                // enclosing real loop without adding an iterator or a cleanup block.
                self.compile_singleton_comprehension_assignment(&nested.target, singleton)?;
                for condition in &nested.ifs {
                    let accepted = self.assembler.label();
                    self.compile_comprehension_filter(
                        condition,
                        self.source_location(key.unwrap_or(value).range()),
                        accepted,
                        loop_context.restart,
                        add_opcode == LIST_APPEND,
                        true,
                    )?;
                    self.assembler.mark(accepted);
                }
                self.compile_comprehension_tail(
                    generators,
                    index + 1,
                    key,
                    value,
                    add_opcode,
                    loop_context,
                )?;
            } else {
                if nested.is_async && self.context.python_version >= PythonVersion::Py314 {
                    self.compile_expression(&nested.iter)?;
                } else {
                    self.compile_iterable_expression(&nested.iter)?;
                }
                self.assembler
                    .set_location(self.source_location(nested.iter.range()));
                self.emit_comprehension_iterator(nested.is_async)?;
                self.assembler
                    .set_location(loop_context.comprehension_location);
                self.compile_comprehension_generator(
                    generators,
                    index + 1,
                    key,
                    value,
                    add_opcode,
                    loop_context.iterator_depth,
                )?;
            }
            self.assembler
                .set_location(self.source_location(key.unwrap_or(value).range()));
        } else {
            let (element, add_opcode) = if self.context.python_version == PythonVersion::Py315 {
                match (add_opcode, value) {
                    (LIST_APPEND, Expr::Starred(starred)) => (starred.value.as_ref(), LIST_EXTEND),
                    (SET_ADD, Expr::Starred(starred)) => (starred.value.as_ref(), SET_UPDATE),
                    (MAP_ADD, _) if key.is_none() => (value, DICT_UPDATE),
                    _ => (value, add_opcode),
                }
            } else {
                (value, add_opcode)
            };
            self.compile_comprehension_element(key, element)?;
            let add_range = key.map_or_else(
                || value.range(),
                |key| ruff_text_size::TextRange::new(key.range().start(), value.range().end()),
            );
            self.assembler.set_location(self.source_location(add_range));
            let effect = if key.is_some() { -2 } else { -1 };
            self.emit(
                add_opcode,
                to_u32(loop_context.iterator_depth + 1, "comprehension depth")?,
                effect,
            )?;
        }
        Ok(())
    }

    fn compile_comprehension_element(
        &mut self,
        key: Option<&Expr>,
        value: &Expr,
    ) -> Result<(), CompileError> {
        let folded_conditional_key = matches!(key, Some(Expr::If(expression)) if early_condition_truthiness(&expression.test).is_some());
        let in_element = (!self.control.comprehensions.in_unfiltered_comprehension
            && key.is_none()
            && !expression_contains_inlined_comprehension(value))
        .then(|| {
            std::mem::replace(
                &mut self.control.comprehensions.in_comprehension_element,
                true,
            )
        });
        let result = self.compile_with_strict_owned_loads(folded_conditional_key, |compiler| {
            if let Some(key) = key {
                if folded_conditional_key {
                    // CPython retains the folded conditional's entry and join boundaries. They
                    // prevent fusion around the key and force both MAP_ADD operands to be owned.
                    compiler.assembler.fusion_barrier();
                }
                compiler.compile_expression(key)?;
                if folded_conditional_key {
                    compiler.assembler.fusion_barrier();
                }
            }
            compiler.compile_expression(value)
        });
        if let Some(in_element) = in_element {
            self.control.comprehensions.in_comprehension_element = in_element;
        }
        result
    }

    fn compile_async_comprehension_generator(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        index: usize,
        key: Option<&Expr>,
        value: &Expr,
        add_opcode: Opcode,
        iterator_depth: usize,
    ) -> Result<(), CompileError> {
        let generator = &generators[index];
        let cleanup_location = self.assembler.location();
        if index == 0 && self.context.python_version == PythonVersion::Py315 {
            self.assembler
                .set_location(self.source_location(generator.iter.range()));
            self.emit_comprehension_iterator(true)?;
            self.assembler.set_location(cleanup_location);
        }
        let loop_depth = self.control.stack.depth;
        let start = self.assembler.label();
        let filter_cleanup = if self.context.python_version <= PythonVersion::Py313 {
            self.assembler.label()
        } else {
            start
        };
        let protected_start = self.assembler.label();
        let yielded = self.assembler.label();
        let yielded_end = self.assembler.label();
        let send = self.assembler.label();
        let send_end = self.assembler.label();
        let protected_before_cleanup_end = self.assembler.label();
        let protected_after_cleanup_start = self.assembler.label();
        let protected_end = self.assembler.label();
        let cleanup_throw = self.assembler.label();
        let cleanup_throw_end = self.assembler.label();
        let async_cleanup = self.assembler.label();

        self.assembler.mark(start);
        self.assembler
            .record_exception_setup(protected_start, false);
        self.assembler.mark(protected_start);
        self.emit(GET_ANEXT, 0, 1)?;
        if self.context.python_version == PythonVersion::Py315 {
            self.emit(PUSH_NULL, 0, 1)?;
        }
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.assembler.mark(send);
        self.emit_jump_forward(SEND, send_end, 0)?;
        self.assembler.record_exception_setup(yielded, false);
        self.assembler.mark(yielded);
        self.emit_yield_value(1, 0)?;
        self.assembler.mark(yielded_end);
        self.emit_resume(ResumeLocation::AFTER_AWAIT, ResumeDepth::Zero)?;
        self.emit_jump_backward(JUMP_BACKWARD_NO_INTERRUPT, send, 0)?;
        self.assembler.mark(protected_before_cleanup_end);
        self.assembler.set_location(cleanup_location);
        self.assembler.mark(cleanup_throw);
        self.set_depth(loop_depth + 3 + self.iterator_extra_slots());
        self.emit(CLEANUP_THROW, 0, -1)?;
        self.assembler.mark(cleanup_throw_end);
        self.assembler.mark(protected_after_cleanup_start);
        self.assembler.mark(send_end);
        self.set_depth(loop_depth + 2 + self.iterator_extra_slots());
        self.emit(END_SEND, 0, -self.iterator_slots())?;
        self.assembler.mark(protected_end);
        self.compile_store_target(&generator.target)?;

        for condition in &generator.ifs {
            let accepted = self.assembler.label();
            let element_range = if index + 1 < generators.len() {
                key.unwrap_or(value).range()
            } else {
                key.map_or_else(
                    || value.range(),
                    |key| ruff_text_size::TextRange::new(key.range().start(), value.range().end()),
                )
            };
            self.compile_comprehension_filter(
                condition,
                self.source_location(element_range),
                accepted,
                filter_cleanup,
                add_opcode == LIST_APPEND,
                true,
            )?;
            self.assembler.mark(accepted);
        }
        let filtered_element_exclusion_start = (!generator.ifs.is_empty()).then(|| {
            let start = self.assembler.label();
            self.assembler.mark(start);
            start
        });
        self.compile_comprehension_tail(
            generators,
            index,
            key,
            value,
            add_opcode,
            ComprehensionLoop {
                restart: filter_cleanup,
                iterator_depth: iterator_depth + 1,
                comprehension_location: cleanup_location,
            },
        )?;
        if self.context.python_version <= PythonVersion::Py313 {
            // Keep CPython's original rejection block until jump threading and
            // backward-conditional normalization run in the assembler.
            self.assembler.preserve_block_boundary(filter_cleanup);
            self.assembler.mark(filter_cleanup);
        }
        self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
        if let Some(exclusion_start) = filtered_element_exclusion_start {
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            self.control
                .generators
                .generator_region_exclusions
                .push((exclusion_start, exclusion_end));
        }
        self.assembler.set_location(cleanup_location);
        self.assembler.add_exception_region(
            yielded,
            yielded_end,
            cleanup_throw,
            (loop_depth + 2 + self.iterator_extra_slots()).cast_unsigned(),
            false,
        );
        self.assembler.mark(async_cleanup);
        self.set_depth(loop_depth + 1);
        self.apply_compiler_stack_effect(END_ASYNC_FOR, 0, -2)?;
        if self.context.python_version <= PythonVersion::Py313 {
            self.assembler.emit(END_ASYNC_FOR, 0);
        } else {
            self.assembler.emit_backward(END_ASYNC_FOR, send);
        }
        let async_cleanup_end = self.assembler.label();
        self.assembler.mark(async_cleanup_end);
        self.control
            .generators
            .generator_region_exclusions
            .push((cleanup_throw, async_cleanup_end));
        self.assembler.add_exception_region(
            protected_start,
            protected_before_cleanup_end,
            async_cleanup,
            loop_depth.cast_unsigned(),
            false,
        );
        self.assembler.add_exception_region(
            protected_after_cleanup_start,
            protected_end,
            async_cleanup,
            loop_depth.cast_unsigned(),
            false,
        );
        self.assembler.add_exception_region(
            cleanup_throw,
            cleanup_throw_end,
            async_cleanup,
            loop_depth.cast_unsigned(),
            false,
        );
        Ok(())
    }

    fn ensure_temporary(&mut self, name: &str) -> Result<u32, CompileError> {
        if let Some(index) = self.temporary_index(name) {
            return Ok(index);
        }
        let name_id = self.required_name_id(name);
        let existing = match &self.unit.scope {
            Scope::Function { indices, .. } => indices.get(&name_id).copied(),
            Scope::Module | Scope::Class { .. } => None,
        };
        let index = if let Some(index) = existing {
            index
        } else {
            let index = to_u32(self.symbols.locals.len(), "comprehension local count")?;
            self.symbols.locals.push(name_id);
            if !matches!(self.unit.scope, Scope::Function { .. }) {
                self.symbols.hidden_names.insert(name_id);
            }
            index
        };
        self.symbols.temporary_indices.insert(name_id, index);
        Ok(index)
    }

    /// Records set construction before Python equality removes duplicate operands.
    fn fold_frozenset_constant(&mut self, constants: Vec<Constant>) -> Constant {
        self.output.has_frozenset_constants = true;
        let mut unique = Vec::with_capacity(constants.len());
        for constant in &constants {
            if !unique
                .iter()
                .any(|existing| python_constant_values_equal(existing, constant))
            {
                unique.push(constant.clone());
            }
        }
        let result = Constant::FrozenSet(unique);
        self.record_folded_constant_dependencies(&result, constants);
        result
    }

    /// Legacy code generation admits folded display constants before visiting later code.
    fn emit_collection_constant(&mut self, constant: Constant) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            let index = self.add_constant(constant)?;
            self.emit(LOAD_CONST, index, 1)
        } else {
            self.emit_deferred_constant(constant)
        }
    }

    fn compile_collection(
        &mut self,
        elements: &[Expr],
        kind: CollectionKind,
    ) -> Result<(), CompileError> {
        let collection_location = self.assembler.location();
        if !elements.iter().any(Expr::is_starred_expr) {
            if kind == CollectionKind::Tuple
                && let Some(constants) = elements
                    .iter()
                    .map(|element| self.source_aware_fold_constant(element))
                    .collect::<Option<Vec<_>>>()
            {
                for element in elements {
                    if self.context.python_version >= PythonVersion::Py314 {
                        self.record_folded_value(element)?;
                    }
                }
                for element in elements {
                    self.emit_folded_tuple_not_nops(element)?;
                }
                self.assembler.set_location(collection_location);
                self.emit_collection_constant(Constant::Tuple(constants))?;
                let hidden_owner_slots = folded_elements_owner_slots(
                    self.context.python_version,
                    elements,
                    elements.len(),
                );
                self.assembler
                    .add_last_constant_fold_slots(hidden_owner_slots);
                // Nested literal tuples remove their own loads and builds from this array.
                // Larger tuples use LIST_APPEND and have different slot accounting.
                if elements.len() <= 30 && elements.iter().all(is_small_literal_tuple_element) {
                    self.assembler
                        .record_last_uniform_fold_owner_slots(hidden_owner_slots);
                }
                return Ok(());
            }
            if elements.len() > 30
                && !(self.context.python_version <= PythonVersion::Py313
                    && matches!(kind, CollectionKind::List | CollectionKind::Set)
                    && elements
                        .iter()
                        .all(|element| self.source_aware_fold_constant(element).is_some()))
            {
                self.emit(kind.build_opcode_for_unpacking(), 0, 1)?;
                for element in elements {
                    self.compile_expression(element)?;
                    self.emit(kind.append_opcode(), 1, -1)?;
                }
                if kind == CollectionKind::Tuple {
                    self.emit_unary_intrinsic(UnaryIntrinsic::LIST_TO_TUPLE)?;
                }
                return Ok(());
            }
            if matches!(kind, CollectionKind::List | CollectionKind::Set)
                && elements.len() >= 3
                && let Some(constants) = elements
                    .iter()
                    .map(|element| self.source_aware_fold_constant(element))
                    .collect::<Option<Vec<_>>>()
            {
                for element in elements {
                    if self.context.python_version >= PythonVersion::Py314 {
                        self.record_folded_value(element)?;
                    }
                }
                for element in elements {
                    self.emit_folded_tuple_not_nops(element)?;
                }
                self.assembler.set_location(collection_location);
                let constant = if kind == CollectionKind::Set {
                    self.fold_frozenset_constant(constants)
                } else {
                    Constant::Tuple(constants)
                };
                self.emit(kind.build_opcode_for_unpacking(), 0, 1)?;
                self.emit_collection_constant(constant)?;
                self.emit(
                    if kind == CollectionKind::List {
                        LIST_EXTEND
                    } else {
                        SET_UPDATE
                    },
                    1,
                    -1,
                )?;
                let hidden_owner_slots = folded_elements_owner_slots(
                    self.context.python_version,
                    elements,
                    elements.len() - 2,
                );
                self.assembler
                    .add_last_constant_fold_slots(hidden_owner_slots);
                let scalar_literal = |element: &Expr| {
                    matches!(
                        element,
                        Expr::NoneLiteral(_)
                            | Expr::BooleanLiteral(_)
                            | Expr::NumberLiteral(_)
                            | Expr::StringLiteral(_)
                            | Expr::BytesLiteral(_)
                            | Expr::EllipsisLiteral(_)
                    )
                };
                // Flat literal tuples fold in this same original array. Count their
                // removed loads as well as the outer display's removed records.
                // Larger tuples use LIST_APPEND and need a different certificate.
                if elements.iter().all(|element| {
                    scalar_literal(element)
                        || matches!(element, Expr::Tuple(tuple)
                            if tuple.elts.len() <= 30
                                && tuple.elts.iter().all(scalar_literal))
                }) {
                    self.assembler
                        .record_last_uniform_fold_owner_slots(hidden_owner_slots);
                }
                return Ok(());
            }
            for element in elements {
                self.compile_expression(element)?;
            }
            return self.emit_build(kind.build_opcode(), elements.len());
        }

        // CPython applies the stack-use limit to the whole display, including starred
        // elements. Large displays build the collection before evaluating any element.
        let unpacking_start = if elements.len() > 30 {
            0
        } else {
            elements
                .iter()
                .position(Expr::is_starred_expr)
                .unwrap_or(elements.len())
        };
        if self.context.python_version >= PythonVersion::Py314
            && unpacking_start >= 3
            && let Some(constants) = elements[..unpacking_start]
                .iter()
                .map(|element| self.source_aware_fold_constant(element))
                .collect::<Option<Vec<_>>>()
        {
            for element in &elements[..unpacking_start] {
                if self.context.python_version >= PythonVersion::Py314 {
                    self.record_folded_value(element)?;
                }
            }
            for element in &elements[..unpacking_start] {
                self.emit_folded_tuple_not_nops(element)?;
            }
            self.assembler.set_location(collection_location);
            let constant = if kind == CollectionKind::Set {
                self.fold_frozenset_constant(constants)
            } else {
                Constant::Tuple(constants)
            };
            self.emit(kind.build_opcode_for_unpacking(), 0, 1)?;
            self.emit_collection_constant(constant)?;
            self.emit(kind.extend_opcode(), 1, -1)?;
            self.assembler
                .add_last_constant_fold_slots(folded_elements_owner_slots(
                    self.context.python_version,
                    &elements[..unpacking_start],
                    if elements.len() <= 30 {
                        unpacking_start.saturating_sub(2)
                    } else {
                        0
                    },
                ));
        } else {
            for element in &elements[..unpacking_start] {
                self.compile_expression(element)?;
            }
            self.emit_build(kind.build_opcode_for_unpacking(), unpacking_start)?;
        }
        for element in &elements[unpacking_start..] {
            if let Expr::Starred(starred) = element {
                self.compile_expression(&starred.value)?;
                self.emit(kind.extend_opcode(), 1, -1)?;
            } else {
                self.compile_expression(element)?;
                self.emit(kind.append_opcode(), 1, -1)?;
            }
        }
        if kind == CollectionKind::Tuple {
            self.emit_unary_intrinsic(UnaryIntrinsic::LIST_TO_TUPLE)?;
        }
        Ok(())
    }

    pub(super) fn compile_iterable_expression(
        &mut self,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        let previous = self.assembler.location();
        let expression_location = self.source_location(expression.range());
        self.assembler.set_location(expression_location);
        let result = match expression {
            Expr::Tuple(tuple)
                if self.context.python_version >= PythonVersion::Py314
                    && (tuple.elts.iter().any(Expr::is_starred_expr) || tuple.elts.len() > 30) =>
            {
                // The 3.15 flowgraph folds constant append chains before removing this
                // intrinsic, preserving the tuple's location on the folded load.
                self.compile_collection(&tuple.elts, CollectionKind::List)?;
                if self.context.python_version == PythonVersion::Py315 {
                    self.emit_unary_intrinsic(UnaryIntrinsic::LIST_TO_TUPLE)?;
                }
                Ok(())
            }
            Expr::List(list) if !list.elts.iter().any(Expr::is_starred_expr) => {
                if (self.context.python_version <= PythonVersion::Py313 || list.elts.len() <= 30)
                    && let Some(constants) = list
                        .elts
                        .iter()
                        .map(|element| self.source_aware_fold_constant(element))
                        .collect::<Option<Vec<_>>>()
                {
                    for element in &list.elts {
                        if self.context.python_version >= PythonVersion::Py314 {
                            self.record_folded_value(element)?;
                        }
                    }
                    for element in &list.elts {
                        self.emit_folded_tuple_not_nops(element)?;
                    }
                    self.assembler.set_location(expression_location);
                    self.emit_collection_constant(Constant::Tuple(constants))?;
                    self.assembler
                        .add_last_constant_fold_slots(folded_elements_owner_slots(
                            self.context.python_version,
                            &list.elts,
                            if list.elts.len() <= 30 {
                                list.elts.len()
                            } else {
                                0
                            },
                        ));
                    Ok(())
                } else {
                    self.compile_collection(
                        &list.elts,
                        if self.context.python_version >= PythonVersion::Py314
                            && list.elts.len() > 30
                        {
                            CollectionKind::List
                        } else {
                            CollectionKind::Tuple
                        },
                    )
                }
            }
            Expr::Set(set)
                if (self.context.python_version <= PythonVersion::Py313
                    || set.elts.len() <= 30)
                    && !set.elts.iter().any(Expr::is_starred_expr)
                    && set
                        .elts
                        .iter()
                        .all(|element| self.source_aware_fold_constant(element).is_some()) =>
            {
                let mut constants = Vec::with_capacity(set.elts.len());
                for element in &set.elts {
                    if self.context.python_version >= PythonVersion::Py314 {
                        self.record_folded_value(element)?;
                    }
                    constants.push(self.source_aware_fold_constant(element).unwrap());
                }
                for element in &set.elts {
                    self.emit_folded_tuple_not_nops(element)?;
                }
                self.assembler.set_location(expression_location);
                let constant = self.fold_frozenset_constant(constants);
                self.emit_collection_constant(constant)?;
                self.assembler
                    .add_last_constant_fold_slots(folded_elements_owner_slots(
                        self.context.python_version,
                        &set.elts,
                        if set.elts.len() <= 30 {
                            set.elts.len()
                        } else {
                            0
                        },
                    ));
                // This membership container folds one literal load per element
                // into its survivor, with no intervening source or cleanup record.
                if set.elts.len() >= 4
                    && set.elts.iter().all(|element| {
                        matches!(
                            element,
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
                        .record_last_uniform_fold_owner_slots(set.elts.len());
                }
                Ok(())
            }
            _ => self.compile_expression_inner(expression, None),
        };
        self.assembler.set_location(previous);
        result
    }

    fn compile_dict(&mut self, items: &[ruff_python_ast::DictItem]) -> Result<(), CompileError> {
        let mut have_dict = false;
        let mut index = 0;
        while index < items.len() {
            if items[index].key.is_none() {
                if !have_dict {
                    self.emit(BUILD_MAP, 0, 1)?;
                    have_dict = true;
                }
                self.compile_expression(&items[index].value)?;
                self.emit(DICT_UPDATE, 1, -1)?;
                index += 1;
                continue;
            }

            // CPython flushes a run after its seventeenth pair, or at an unpacking
            // boundary. Runs exceeding fifteen pairs build incrementally.
            let group_start = index;
            while index < items.len() && items[index].key.is_some() && index - group_start < 17 {
                index += 1;
            }
            let group = &items[group_start..index];
            if self.context.python_version <= PythonVersion::Py313
                && (2..=15).contains(&group.len())
                && let Some(keys) = group
                    .iter()
                    .map(|item| self.source_aware_fold_constant(item.key.as_ref().unwrap()))
                    .collect::<Option<Vec<_>>>()
            {
                for item in group {
                    self.compile_expression(&item.value)?;
                }
                let keys = self.add_constant(Constant::Tuple(keys))?;
                self.emit(LOAD_CONST, keys, 1)?;
                let count = to_u32(group.len(), "dictionary item count")?;
                self.emit(BUILD_CONST_KEY_MAP, count, -i32::try_from(count).unwrap())?;
                if have_dict {
                    self.emit(DICT_UPDATE, 1, -1)?;
                }
                have_dict = true;
                continue;
            }
            let large = group.len() > 15;
            if large {
                self.emit(BUILD_MAP, 0, 1)?;
            }
            for item in group {
                self.compile_expression(item.key.as_ref().unwrap())?;
                self.compile_expression(&item.value)?;
                if large {
                    self.emit(MAP_ADD, 1, -2)?;
                }
            }
            if !large {
                self.emit_build_map(group.len())?;
            }
            if have_dict {
                self.emit(DICT_UPDATE, 1, -1)?;
            }
            have_dict = true;
        }
        if !have_dict {
            self.emit(BUILD_MAP, 0, 1)?;
        }
        Ok(())
    }

    pub(super) fn emit_build_map(&mut self, length: usize) -> Result<(), CompileError> {
        let length = to_u32(length, "dictionary item count")?;
        self.emit(BUILD_MAP, length, 1 - i32::try_from(length).unwrap() * 2)
    }

    fn compile_slice(&mut self, slice: &ruff_python_ast::ExprSlice) -> Result<(), CompileError> {
        let slice_location = self.source_location(slice.range);
        if self.context.python_version >= PythonVersion::Py314
            && let Some(constant) = self.source_aware_constant_slice(slice)
        {
            let index = self.add_constant(constant)?;
            self.assembler.set_location(slice_location);
            return self.emit(LOAD_CONST, index, 1);
        }
        for part in [&slice.lower, &slice.upper] {
            self.compile_optional_slice_bound(part.as_deref(), slice_location)?;
        }
        let argument_count = if let Some(step) = &slice.step {
            self.compile_expression(step)?;
            3
        } else {
            2
        };
        self.emit(
            BUILD_SLICE,
            argument_count,
            1 - argument_count.cast_signed(),
        )
    }

    pub(super) fn compile_optional_slice_bound(
        &mut self,
        bound: Option<&Expr>,
        fallback_location: SourceLocation,
    ) -> Result<(), CompileError> {
        if let Some(bound) = bound {
            self.compile_expression(bound)
        } else {
            let none = self.add_constant(Constant::None)?;
            self.assembler.set_location(fallback_location);
            self.emit(LOAD_CONST, none, 1)
        }
    }

    fn preserve_folded_if_expression_entry(&mut self, truthiness: bool) {
        if !truthiness {
            // The discarded true arm and its exit leave an empty original CFG block
            // before the selected false arm. Borrow traversal stops at that block,
            // and load fusion must keep the selected arm separate from its predecessor.
            let selected_arm = self.assembler.label();
            self.assembler.mark(selected_arm);
            self.assembler.prevent_borrow_reachability(selected_arm);
        }
    }

    fn compile_if_expression(
        &mut self,
        expression: &ruff_python_ast::ExprIf,
        continuation: Option<(BoolOp, SourceLocation)>,
    ) -> Result<(), CompileError> {
        if let Some(truthiness) = early_condition_truthiness(&expression.test) {
            self.record_folded_value(&expression.test)?;
            self.pre_register_expression_names(&expression.body)?;
            self.pre_register_expression_names(&expression.orelse)?;
            self.preserve_folded_if_expression_entry(truthiness);
            let selected_entry = self.assembler.label();
            if !truthiness {
                self.pre_register_discarded_expression(&expression.body, Some(selected_entry))?;
            }
            let selected = if truthiness {
                &expression.body
            } else {
                &expression.orelse
            };
            if self.source_location(expression.test.range()).line
                != self.source_location(selected.range()).line
            {
                // Folding a multiline condition leaves its source-position marker in CPython's
                // optimized flow graph.
                self.assembler
                    .set_location(self.source_location(expression.test.range()));
                self.emit(NOP, 0, 0)?;
            }
            self.assembler
                .record_exception_owner_block_boundary(selected_entry);
            self.assembler.mark(selected_entry);
            let selected_continuation = if truthiness { None } else { continuation };
            self.compile_expression_with_boolean_continuation(selected, selected_continuation)?;
            let end = self.assembler.label();
            if truthiness {
                self.pre_register_discarded_expression(&expression.orelse, Some(end))?;
            }
            // Keep the eliminated arm's borrowed-load barrier separate from the original
            // join. The exit-copying pass can still append a small join to the selected arm.
            self.assembler.mark_last_borrow_fallthrough_barrier();
            if truthiness {
                self.assembler.set_location(SourceLocation::NONE);
                let exit = self.emit_jump_forward_instruction(JUMP_FORWARD, end)?;
                self.assembler.mark_folded_condition_exit(exit);
            } else {
                self.assembler.record_exception_owner_block_boundary(end);
            }
            self.assembler.mark(end);
            return Ok(());
        }
        let base_depth = self.control.stack.depth;
        let else_label = self.assembler.label();
        let end = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ValueIfEnd(expression.range),
            end,
        );
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ValueIfElse(expression.range),
            else_label,
        );
        // CPython appends this repeated-attribute if-expression marker after
        // compacting an ordinary try inside an iterator loop. A fresh marker slot
        // is ownerless; a concrete protected fallthrough vacancy remains owned.
        let exclude_plain_try_attribute_marker =
            self.control.generators.generator_region_start.is_none()
                && self.control.in_iterator_loop()
                && !self.control.finally.exceptional_finally_condition
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .is_empty()
                && self.control.withs.active_with_region_exclusions.is_empty()
                && self.control.finally.active_normal_finally_bodies == 0
                && self.control.finally.active_finally_end_blocks == 0
                && self.control.exceptions.active_exception_group_handlers == 0
                && !self.control.exceptions.same_line_try_body
                && !self.control.current_exception_handler_binds_name()
                && !self.control.in_exception_handler()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .len()
                    == 1
                && !self
                    .assembler
                    .fallthrough_has_concrete_dynamic_exception_owner()
                && matches!(
                    (expression.test.as_ref(), expression.body.as_ref()),
                    (Expr::Attribute(test), Expr::Attribute(body))
                        if test.attr.as_str() == body.attr.as_str()
                            && matches!(
                                (test.value.as_ref(), body.value.as_ref()),
                                (Expr::Name(test), Expr::Name(body))
                                    if test.id.as_str() == body.id.as_str()
                            )
                );
        let exclude_try_marker = self
            .control
            .conditions
            .exclude_condition_not_taken_from_exception
            || exclude_plain_try_attribute_marker
            || (self
                .control
                .current_try_handler_starts_with_optimized_boolean_generator()
                && matches!(expression.test.as_ref(), Expr::Name(_)))
            || (self
                .control
                .statements
                .current_statement_preceded_by_terminating_if
                && self.control.stack.depth == 2
                && self.unit.flags & (CO_GENERATOR | CO_ASYNC_GENERATOR) == 0
                && self.control.withs.active_with_region_exclusions.len() == 1
                && self.control.in_synchronous_with()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .is_empty()
                && self.control.finally.active_normal_finally_bodies == 0
                && self.control.finally.active_finally_end_blocks == 0
                && self.control.exceptions.active_exception_group_handlers == 0
                && is_name_rooted_if_expression_condition(&expression.test));
        let preserve_none_condition_owner = (self.control.in_synchronous_with()
            || (self
                .control
                .exceptions
                .active_exception_region_exclusions
                .is_empty()
                && !self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .is_empty()
                && !self
                    .control
                    .comprehensions
                    .suppress_filtered_comprehension_none_owner))
            && matches!(expression.test.as_ref(), Expr::Compare(comparison)
                if comparison.ops.len() == 1
                    && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
                    && matches!(comparison.comparators[0], Expr::NoneLiteral(_)));
        let preserve_generator_loop_condition_owner =
            self.unit.flags & (CO_GENERATOR | CO_ASYNC_GENERATOR) != 0
                && self.control.in_iterator_loop()
                && !self.control.generators.generator_expression
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && matches!(expression.test.as_ref(), Expr::UnaryOp(unary)
                    if unary.op == UnaryOp::Not
                        && matches!(unary.operand.as_ref(), Expr::Name(_)))
                && matches!(expression.body.as_ref(), Expr::Name(_))
                && matches!(expression.orelse.as_ref(), Expr::BinOp(_));
        let enter_filtered_comprehension_conditional = !self
            .control
            .comprehensions
            .suppress_filtered_comprehension_none_owner
            && !self.control.comprehensions.in_unfiltered_comprehension
            && !self
                .control
                .comprehensions
                .active_comprehension_region_exclusions
                .is_empty();
        let direct_filtered_comprehension = enter_filtered_comprehension_conditional
            && self.control.comprehensions.in_comprehension_element
            && !self
                .control
                .comprehensions
                .filtered_comprehension_has_root_if_expression
            && self.control.generators.generator_region_start.is_none()
            && self
                .control
                .exceptions
                .active_exception_region_exclusions
                .is_empty()
            && self.control.withs.active_with_region_exclusions.is_empty();
        let preserve_filtered_comprehension_short_circuit_none_owner = direct_filtered_comprehension
            && comprehension_condition_has_trailing_none(&expression.test);
        // An unfused comprehension target can leave the leading specialized None-condition
        // marker inside the cleanup region when the conditional short-circuits to a comparison.
        let preserve_unfiltered_comprehension_leading_none_owner =
            self.control.comprehensions.in_unfiltered_comprehension
                && self.control.generators.generator_region_start.is_none()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self.control.withs.active_with_region_exclusions.is_empty()
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .len()
                    == 1
                && matches!(expression.test.as_ref(), Expr::BoolOp(boolean)
                if boolean.op == BoolOp::Or
                    && matches!(boolean.values.as_slice(), [Expr::Compare(first), Expr::Compare(_)]
                        if first.ops.len() == 1
                            && matches!(first.ops[0], CmpOp::Is | CmpOp::IsNot)
                            && matches!(first.left.as_ref(), Expr::Name(_))
                            && first.comparators.len() == 1
                            && matches!(first.comparators[0], Expr::NoneLiteral(_))));
        let include_scoped_none_condition_owner =
            preserve_unfiltered_comprehension_leading_none_owner
                || self.control.conditions.include_scoped_none_condition_owner;
        self.with_control_override(
            |control| &mut control.conditions.include_scoped_none_condition_owner,
            include_scoped_none_condition_owner,
            |compiler| {
                compiler.with_control_override(
                    |control| {
                        &mut control
                            .conditions
                            .exclude_condition_not_taken_from_exception
                    },
                    exclude_try_marker,
                    |compiler| {
                        compiler.compile_if_expression_condition(&expression.test, else_label)
                    },
                )
            },
        )?;
        if preserve_none_condition_owner
            || preserve_generator_loop_condition_owner
            || preserve_filtered_comprehension_short_circuit_none_owner
        {
            self.assembler.override_last_not_taken_exception_owner();
        }
        self.assembler.record_short_circuit_owner_continuation();
        // The body reaches the join through an authored forward jump, so its Boolean
        // tests keep their own locations. Only the else arm falls into the outer test.
        if enter_filtered_comprehension_conditional {
            self.with_control_override(
                |control| {
                    &mut control
                        .comprehensions
                        .suppress_filtered_comprehension_none_owner
                },
                true,
                |compiler| compiler.compile_expression(&expression.body),
            )?;
        } else {
            self.compile_expression(&expression.body)?;
        }
        // CPython emits the conditional-expression join without a location. Its later
        // propagation pass inherits the body location only when the join remains in the same
        // basic block; short-circuit bodies can leave it deliberately unlocated.
        // A nested conditional leaves its own join immediately before this one, so propagate
        // that conditional's actual fallthrough leaf rather than its whole source range.
        let preserve_nested_no_location = if let Expr::If(nested) = expression.body.as_ref() {
            let fallthrough = conditional_expression_fallthrough(nested);
            let location = if self.context.python_version <= PythonVersion::Py313 {
                // Native legacy joins stay unlocated through early exit copying;
                // inheriting the else leaf now would thread away the donor owner.
                SourceLocation::NONE
            } else {
                fallthrough.map_or(SourceLocation::NONE, |fallthrough| {
                    self.source_location(fallthrough.range())
                })
            };
            self.assembler.set_location(location);
            fallthrough.is_none()
        } else {
            self.assembler.set_location(SourceLocation::NONE);
            false
        };
        let optimized_tuple_join = matches!(expression.body.as_ref(), Expr::Call(call)
            if matches!(optimized_generator_callable_for_target(call, self.context.python_version), Some("tuple" | "list" | "set")))
            && self.assembler.at_jump_target();
        let body_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, end)?;
        if let Expr::If(nested) = expression.body.as_ref()
            && early_condition_truthiness(&nested.test).is_none()
            && !preserve_nested_no_location
        {
            // The nested conditional's fallthrough enters a separate original join block.
            // Only copying its direct exit keeps that jump as a source-position NOP.
            self.assembler.preserve_direct_inlined_jump_nop(body_exit);
        } else if optimized_tuple_join {
            // Optimized and fallback calls end at a separate original array. Its
            // jump slot survives copying before the fallback supplies its line.
            self.assembler
                .preserve_empty_join_inlined_jump_nop(body_exit);
        }
        if self.control.statements.terminal_if_call_expression {
            self.assembler.prevent_jump_inlining(body_exit);
        }
        if self
            .control
            .exceptions
            .active_exception_region_exclusions
            .len()
            == 1
            && self.control.withs.active_with_region_exclusions.is_empty()
            && self
                .control
                .comprehensions
                .active_comprehension_region_exclusions
                .is_empty()
            && self.control.generators.generator_region_start.is_none()
            && self.control.finally.active_normal_finally_bodies == 0
            && self.control.finally.active_finally_end_blocks == 0
            && !self.control.in_exception_handler()
            && !self.control.statements.suppress_split_return_copy
        {
            // CPython can copy a small call/return tail onto this branch even when the
            // enclosing try splits the shared exit at an exception-region boundary.
            self.assembler.allow_split_return_copy(body_exit);
        }
        if matches!(expression.body.as_ref(), Expr::BoolOp(_)) || preserve_nested_no_location {
            self.assembler.preserve_no_location(body_exit.instruction());
        }
        if self.context.python_version == PythonVersion::Py314
            && self.control.finally.active_copied_finally_bodies == 0
            && self.control.finally.active_normal_finally_bodies == 0
            && self.control.finally.active_finally_end_blocks == 0
            && condition_jump_target_count(&expression.test, false) == 0
        {
            // A folded short-circuit outcome can discard the else array while
            // retaining evaluation of its runtime operands. Erasing this exit
            // still crosses that empty original array before the shared join.
            self.assembler.record_noncopyable_borrow_exit(body_exit);
        }
        self.assembler
            .record_exception_owner_block_boundary(else_label);
        self.assembler.mark(else_label);
        if self.context.python_version == PythonVersion::Py314
            && matches!(expression.test.as_ref(), Expr::UnaryOp(unary)
                if unary.op == UnaryOp::Not
                    && jump_constant_truthiness(&unary.operand) == Some(true))
        {
            // Negating a true constant discards the first arm and its exit array.
            // CPython's borrow traversal stops there before the selected else arm.
            self.assembler.prevent_borrow_reachability(else_label);
        }
        self.set_depth(base_depth);
        if enter_filtered_comprehension_conditional {
            self.with_control_override(
                |control| {
                    &mut control
                        .comprehensions
                        .suppress_filtered_comprehension_none_owner
                },
                true,
                |compiler| {
                    compiler.compile_expression_with_boolean_continuation(
                        &expression.orelse,
                        continuation,
                    )
                },
            )?;
        } else {
            self.compile_expression_with_boolean_continuation(&expression.orelse, continuation)?;
        }
        self.assembler.preserve_block_boundary_if_new(end);
        self.assembler.record_exception_owner_block_boundary(end);
        self.assembler.mark(end);
        self.set_depth(base_depth + 1);
        Ok(())
    }

    fn compile_if_expression_condition(
        &mut self,
        test: &Expr,
        otherwise: Label,
    ) -> Result<(), CompileError> {
        // A value branch can become a small return after the shared tail is copied.
        // Keep the comparison fallthrough eligible for that subsequent copy.
        self.with_control_override(
            |control| &mut control.conditions.retained_chained_comparison_exit,
            final_chained_comparison_jump(test),
            |compiler| compiler.compile_jump_if(test, false, otherwise),
        )
    }

    pub(super) fn compile_tail_return_expression(
        &mut self,
        expression: &Expr,
        return_location: SourceLocation,
    ) -> Result<(), CompileError> {
        fn has_only_runtime_tail_conditions(expression: &Expr) -> bool {
            let Expr::If(conditional) = expression else {
                return true;
            };
            early_condition_truthiness(&conditional.test).is_none()
                && has_only_runtime_tail_conditions(&conditional.body)
                && has_only_runtime_tail_conditions(&conditional.orelse)
        }

        // Keep each original conditional join before the shared return. Distributing
        // returns first can make an inner argument join a small exit too early.
        // Folded selectors retain their existing discarded-cache admission boundaries.
        let defer_return = matches!(expression, Expr::If(_))
            && has_only_runtime_tail_conditions(expression)
            && !expression_contains_yield(expression);
        self.compile_tail_return_expression_inner(
            expression,
            return_location,
            false,
            defer_return,
        )?;
        if defer_return {
            self.assembler.set_location(return_location);
            if self.source_location(expression.range()).line != return_location.line {
                // CPython keeps the return-line marker through early exit copying.
                // It counts toward the original block's four-instruction copy limit.
                self.emit(NOP, 0, 0)?;
            }
            self.emit(RETURN_VALUE, 0, -1)?;
        }
        Ok(())
    }

    fn compile_tail_return_expression_inner(
        &mut self,
        expression: &Expr,
        return_location: SourceLocation,
        final_fallthrough: bool,
        defer_return: bool,
    ) -> Result<(), CompileError> {
        let Expr::If(conditional) = expression else {
            self.compile_with_strict_owned_loads(
                final_fallthrough && matches!(expression, Expr::Name(_)),
                |compiler| compiler.compile_expression(expression),
            )?;
            if !defer_return {
                self.assembler.set_location(return_location);
                self.emit(RETURN_VALUE, 0, -1)?;
            }
            return Ok(());
        };

        if let Some(truthiness) = early_condition_truthiness(&conditional.test) {
            self.record_folded_value(&conditional.test)?;
            self.pre_register_expression_names(&conditional.body)?;
            self.pre_register_expression_names(&conditional.orelse)?;
            if !truthiness {
                self.pre_register_discarded_expression(&conditional.body, None)?;
            }
            self.preserve_folded_if_expression_entry(truthiness);
            let selected = if truthiness {
                &conditional.body
            } else {
                &conditional.orelse
            };
            if self.source_location(conditional.test.range()).line
                != self.source_location(selected.range()).line
            {
                self.assembler
                    .set_location(self.source_location(conditional.test.range()));
                self.emit(NOP, 0, 0)?;
            }
            self.compile_tail_return_expression_inner(
                selected,
                return_location,
                final_fallthrough,
                defer_return,
            )?;
            if truthiness {
                self.pre_register_discarded_expression(&conditional.orelse, None)?;
            }
            return Ok(());
        }

        let base_depth = self.control.stack.depth;
        let otherwise = self.assembler.label();
        let end = defer_return.then(|| self.assembler.label());
        if let Some(end) = end {
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ValueIfEnd(conditional.range),
                end,
            );
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ValueIfElse(conditional.range),
                otherwise,
            );
        }
        if defer_return {
            self.compile_if_expression_condition(&conditional.test, otherwise)?;
        } else {
            self.compile_jump_if(&conditional.test, false, otherwise)?;
        }
        self.compile_tail_return_expression_inner(
            &conditional.body,
            return_location,
            false,
            defer_return,
        )?;
        if let Some(end) = end {
            self.assembler.set_location(SourceLocation::NONE);
            self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
        }

        self.assembler.mark(otherwise);
        self.set_depth(base_depth);
        self.compile_tail_return_expression_inner(
            &conditional.orelse,
            return_location,
            true,
            defer_return,
        )?;
        if let Some(end) = end {
            self.assembler.preserve_block_boundary_if_new(end);
            self.assembler.mark(end);
            self.set_depth(base_depth + 1);
        } else {
            self.set_depth(base_depth);
        }
        Ok(())
    }

    fn compile_bool_expression(
        &mut self,
        expression: &ruff_python_ast::ExprBoolOp,
        continuation: Option<(BoolOp, SourceLocation)>,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313 {
            let end = self.assembler.label();
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ValueBoolEnd(expression.range),
                end,
            );
            let location = self.source_location(expression.range);
            let jump = if expression.op == BoolOp::And {
                POP_JUMP_IF_FALSE
            } else {
                POP_JUMP_IF_TRUE
            };
            let Some((last, leading)) = expression.values.split_last() else {
                return Err(CompileError::Internal(
                    "Boolean expression has no operands".to_owned(),
                ));
            };
            for value in leading {
                self.compile_expression(value)?;
                self.assembler.set_location(location);
                self.emit(COPY, 1, 1)?;
                if self.context.python_version == PythonVersion::Py313 {
                    self.emit(TO_BOOL, 0, 0)?;
                }
                self.emit_jump_forward(jump, end, -1)?;
                self.emit(POP_TOP, 0, -1)?;
            }
            self.compile_expression(last)?;
            self.assembler.mark(end);
            Ok(())
        } else {
            self.compile_bool_expression_at(expression, continuation, false)
        }
    }

    fn compile_bool_expression_at(
        &mut self,
        expression: &ruff_python_ast::ExprBoolOp,
        continuation: Option<(BoolOp, SourceLocation)>,
        nested_leading_group: bool,
    ) -> Result<(), CompileError> {
        // Threading a value-preserving boolean jump into an enclosing test gives it that
        // test's location. The last operand flows directly to the enclosing test, so pass
        // this continuation through trailing groups even when their operators differ.
        let location = continuation
            .filter(|(operator, _)| *operator == expression.op)
            .map_or_else(
                || self.source_location(expression.range),
                |(_, location)| location,
            );
        let mut effective = Vec::with_capacity(expression.values.len());
        let mut short_circuited = false;
        let mut folded_prefix = false;
        for (index, value) in expression.values.iter().enumerate() {
            let is_last = index + 1 == expression.values.len();
            if short_circuited {
                self.pre_register_expression_names(value)?;
                if fold_constant_for_version(value, self.context.python_version).is_some() {
                    self.record_folded_value(value)?;
                }
                continue;
            }
            if !is_last && let Some(truthiness) = early_condition_truthiness(value) {
                let decisive = matches!(expression.op, BoolOp::And) && !truthiness
                    || matches!(expression.op, BoolOp::Or) && truthiness;
                if decisive {
                    effective.push(value);
                    short_circuited = true;
                } else {
                    self.record_folded_value(value)?;
                    folded_prefix |= index == 0
                        && fold_constant_for_version(value, self.context.python_version).is_some();
                }
                continue;
            }
            effective.push(value);
        }

        let Some((last, leading)) = effective.split_last() else {
            return Err(CompileError::Internal(
                "boolean expression contains no values".to_string(),
            ));
        };
        self.assembler.set_location(location);
        if folded_prefix {
            // CPython folds the leading constant load and its boolean jump, but retains the
            // jump as a source-position NOP when the next operand starts on another line.
            let empty_folded_block = self.assembler.at_empty_borrow_block();
            self.emit(NOP, 0, 0)?;
            if empty_folded_block {
                // Removing the sole folded value test empties its original array.
                // A retained line NOP still permits borrowing through that array.
                self.assembler.mark_last_nop_borrow_barrier_if_removed();
            }
        }
        let base_depth = self.control.stack.depth;
        let end = self.assembler.label();
        let jump = match expression.op {
            BoolOp::And => POP_JUMP_IF_FALSE,
            BoolOp::Or => POP_JUMP_IF_TRUE,
        };
        let follows_inline_comprehension = std::mem::take(
            &mut self
                .control
                .generators
                .pre_suspension_inline_comprehension_restore,
        );
        let has_nested_leading_boolean = follows_inline_comprehension
            && leading.iter().any(|value| matches!(value, Expr::BoolOp(_)));

        for (operand_index, value) in leading.iter().enumerate() {
            if let Expr::BoolOp(inner) = value
                && (inner.op == expression.op
                    || folded_bool_operand(value, self.context.python_version).is_none())
            {
                self.compile_bool_expression_at(
                    inner,
                    Some((expression.op, location)),
                    inner.op == expression.op,
                )?;
            } else {
                self.compile_expression_with_boolean_continuation(
                    value,
                    Some((expression.op, location)),
                )?;
            }
            let original_location = self.source_location(expression.range);
            if original_location.line != location.line
                && self
                    .assembler
                    .current_block_last_instruction_location()
                    .is_none_or(|previous| previous.line != original_location.line)
            {
                // Threading into the outer test converts the inner jump to a NOP.
                // Retain its line event when neither neighboring instruction owns it.
                self.assembler.set_location(original_location);
                self.emit(NOP, 0, 0)?;
            }
            self.assembler.set_location(location);
            let resumes_after_yield = self.unit.flags & (CO_GENERATOR | CO_ASYNC_GENERATOR) != 0
                && self.assembler.current_block_resumes_after_yield();
            // CPython inserts `COPY`, `TO_BOOL`, and `NOT_TAKEN` after compacting the CFG. The
            // marker can only reuse a stale exception owner when all three inserted slots were
            // vacated in this block, or in a tuple handler's fallthrough label chain.
            let marker_owner_slots = self.assembler.current_block_dynamic_exception_owner_slots();
            let fallthrough_marker_reuses_exception_owner = self
                .control
                .exceptions
                .active_tuple_handler_try_bodies
                .last()
                .is_some_and(|tuple_handler| *tuple_handler)
                && self.assembler.fallthrough_dynamic_exception_owner_slots() >= 3;
            let marker_reuses_exception_owner = (marker_owner_slots >= 3
                && !(self.control.in_synchronous_with()
                    && self.control.follows_copied_nested_handler_join()))
                || fallthrough_marker_reuses_exception_owner;
            let current_block = self.assembler.current_block_token();
            let exception_type_marker_owner = self
                .control
                .exception_type_bool_marker_preserves_owner(current_block);
            let follows_sync_with_branch_join = self.control.in_synchronous_with()
                && self.control.follows_copied_nested_handler_join();
            let follows_sync_with_try_join = expression.op == BoolOp::Or
                && operand_index == 0
                && !nested_leading_group
                && self.control.in_synchronous_with()
                && (self.control.statements.current_statement_preceded_by_try
                    || (self
                        .control
                        .statements
                        .current_statement_preceded_by_terminating_if
                        && self
                            .control
                            .exceptions
                            .active_exception_region_exclusions
                            .is_empty()))
                && !marker_reuses_exception_owner;
            let follows_sync_with_nonterminal_join = expression.op == BoolOp::Or
                && operand_index == 0
                && (marker_owner_slots == 1
                    || (marker_owner_slots == 2 && expression.values.len() == 2))
                && !nested_leading_group
                && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) == 0
                && self.control.withs.active_with_region_exclusions.len() == 1
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self
                    .control
                    .statements
                    .current_statement_follows_sync_with_nonterminal_join;
            let follows_sync_with_return_join = operand_index == 0
                && (!self
                    .control
                    .statements
                    .current_statement_follows_sync_with_return_join_after_gap
                    || expression.values.len() == 2)
                && !nested_leading_group
                && self.control.in_synchronous_with()
                && self
                    .control
                    .statements
                    .current_statement_follows_sync_with_return_join
                && (self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                    || (!self
                        .control
                        .statements
                        .current_statement_follows_sync_with_return_join_after_gap
                        && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR)
                            == 0
                        && !self.control.in_synchronous_with_nested_exception_region()))
                && !marker_reuses_exception_owner;
            let follows_sync_with_terminal_boolean_value = expression.op == BoolOp::Or
                && operand_index == 0
                && !nested_leading_group
                && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) == 0
                && self.control.in_synchronous_with()
                && self.control.withs.active_with_region_exclusions.len() == 1
                && !self.control.in_exception_handler()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .is_empty()
                && !self.control.in_loop()
                && !self.control.statements.current_statement_has_following
                && !self.control.statements.inherited_statement_has_following
                && !self
                    .control
                    .statements
                    .current_statement_follows_sync_with_return_join
                && !self
                    .control
                    .statements
                    .current_statement_follows_sync_with_nonterminal_join
                && self
                    .control
                    .statements
                    .current_return_boolean_value_marker_scan
                    .as_mut()
                    .is_some_and(|scan| self.assembler.advance_boolean_value_marker_scan(scan));
            let preserve_exception_type_owner =
                exception_type_marker_owner == Some(true) && !follows_sync_with_branch_join;
            let exclude_exception_type_owner =
                exception_type_marker_owner == Some(false) && !marker_reuses_exception_owner;
            let exclude_inlined_comprehension_owner = !self
                .control
                .comprehensions
                .active_comprehension_region_exclusions
                .is_empty()
                && !marker_reuses_exception_owner
                && exception_type_marker_owner.is_none();
            let exclusion_start = self.assembler.label();
            self.assembler.mark(exclusion_start);
            self.assembler
                .record_value_jump_owner_continuation(exclusion_start);
            self.emit(COPY, 1, 1)?;
            self.emit(TO_BOOL, 0, 0)?;
            self.emit_jump_forward(jump, end, -1)?;
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);

            let exception_group_handler =
                self.control.exceptions.active_exception_group_handlers > 0;
            let normalized_owner = self.control.finally.active_normal_finally_bodies > 0
                || self.control.finally.active_finally_end_blocks > 0
                || resumes_after_yield
                || exception_group_handler;
            let suspension_epoch = self.control.generators.coroutine_suspension_epoch;
            let follows_inline_comprehension_before_suspension = follows_inline_comprehension
                && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) != 0
                && suspension_epoch == 0
                && !self.control.generators.emitted_yield_value
                && !resumes_after_yield
                && operand_index == 0
                && marker_owner_slots == 1
                && self.control.generators.generator_region_start.is_some()
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .is_empty()
                && self.control.withs.active_with_region_exclusions.is_empty()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self.control.finally.active_normal_finally_bodies == 0
                && self.control.finally.active_finally_end_blocks == 0
                && self.control.exceptions.active_exception_group_handlers == 0
                && !self.control.in_loop()
                && !nested_leading_group
                && !has_nested_leading_boolean;
            let follows_sync_with_wrapped_boolean_value = operand_index == 0
                && marker_owner_slots == 1
                && !nested_leading_group
                && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) == 0
                && self.control.withs.active_with_region_exclusions.len() == 1
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .is_empty()
                && self.control.finally.active_normal_finally_bodies == 0
                && self.control.finally.active_finally_end_blocks == 0
                && self.control.exceptions.active_exception_group_handlers == 0
                && !self.control.in_loop()
                && matches!(
                    expression.values.as_slice(),
                    [
                        Expr::Call(_),
                        Expr::List(_)
                            | Expr::Dict(_)
                            | Expr::Tuple(_)
                            | Expr::Set(_)
                            | Expr::Name(_)
                            | Expr::Attribute(_),
                    ]
                )
                && self
                    .control
                    .withs
                    .current_sync_with_boolean_value_marker_scan
                    .as_mut()
                    .is_some_and(|scan| self.assembler.advance_boolean_value_marker_scan(scan));
            let follows_marker_before_coroutine_suspension = self.unit.flags & CO_COROUTINE != 0
                && suspension_epoch == 0
                && self
                    .control
                    .comprehensions
                    .active_comprehension_region_exclusions
                    .is_empty()
                && self.control.withs.active_with_region_exclusions.is_empty()
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                && self.control.finally.active_normal_finally_bodies == 0
                && self.control.finally.active_finally_end_blocks == 0
                && self.control.exceptions.active_exception_group_handlers == 0
                && (((1..=2).contains(&marker_owner_slots)
                    && self
                        .control
                        .generators
                        .generator_region_exclusions
                        .is_empty()
                    && !self.control.in_loop()
                    && self.assembler.contains_opcode(NOT_TAKEN))
                    || self.assembler.contains_boolean_value_marker());
            let exclude_generator_region_owner = (self.control.generators.generator_expression
                || follows_marker_before_coroutine_suspension)
                && !marker_reuses_exception_owner;
            let after_coroutine_suspension =
                self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR) != 0 && suspension_epoch > 0;
            let has_fresh_with_owner = after_coroutine_suspension
                && self.control.has_with_suspension_epoch(suspension_epoch);
            let named_exception_handler = self.control.current_exception_handler_binds_name();
            let exclude_nested_handler_boolean = self.control.in_exception_handler()
                && self.control.statements.inherited_statement_has_following
                && self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) == 0
                && self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .len()
                    == 1
                && self.control.withs.active_with_region_exclusions.is_empty()
                && self.control.finally.active_normal_finally_bodies == 0;
            let exclude_after_coroutine_suspension = after_coroutine_suspension
                && !has_fresh_with_owner
                && self.control.finally.active_finally_end_blocks == 0
                && !marker_reuses_exception_owner
                && !self.control.exceptions.same_line_try_body
                && !named_exception_handler
                && !exception_group_handler
                && exception_type_marker_owner.is_none();
            let exclude_from_plain_try = (self.control.generators.generator_region_start.is_none()
                || (self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) != 0
                    && suspension_epoch == 0
                    && marker_owner_slots == 1
                    && self.optimized_generator_dispatch_is_in_normal_try()
                    && self.assembler.contains_opcode(NOT_TAKEN)))
                && self.control.active_with_count() == 0
                && self.control.finally.active_normal_finally_bodies == 0
                && self.control.finally.active_finally_end_blocks == 0
                && !self.control.exceptions.same_line_try_body
                && (!named_exception_handler || exclude_nested_handler_boolean)
                && !exception_group_handler
                && !marker_reuses_exception_owner
                && exception_type_marker_owner.is_none()
                && !self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .is_empty()
                // Exception-range labels can split one original instruction array. Three
                // vacancies leave a marker slot after COPY and TO_BOOL are inserted.
                && self.assembler.fallthrough_dynamic_exception_owner_slots() < 3;
            if nested_leading_group && exclude_from_plain_try {
                for exclusions in &mut self.control.exceptions.active_exception_region_exclusions {
                    exclusions.push((exclusion_start, exclusion_end));
                }
            }
            let exclude_from_all_owners = exclude_exception_type_owner
                || exclude_inlined_comprehension_owner
                || exclude_after_coroutine_suspension
                || exclude_from_plain_try
                || follows_inline_comprehension_before_suspension
                || follows_sync_with_wrapped_boolean_value
                || follows_sync_with_try_join
                || follows_sync_with_return_join
                || follows_sync_with_terminal_boolean_value
                || follows_sync_with_nonterminal_join
                || follows_sync_with_branch_join;
            let marker_start = (!exclude_inlined_comprehension_owner
                && (exclude_generator_region_owner
                    || exclude_after_coroutine_suspension
                    || exclude_from_plain_try))
                .then(|| {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    start
                });

            self.emit_with_exception_policy(
                NOT_TAKEN,
                0,
                0,
                if exclude_from_all_owners {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::normalized_owner(
                        normalized_owner
                            || preserve_exception_type_owner
                            || marker_reuses_exception_owner,
                    )
                },
            )?;

            self.assembler
                .defer_last_boolean_exception_owner(exclusion_start);

            if let Some(marker_start) = marker_start {
                let marker_end = self.assembler.label();
                self.assembler.mark(marker_end);
                if self.control.generators.generator_region_start.is_some()
                    && (exclude_generator_region_owner || exclude_after_coroutine_suspension)
                {
                    self.control
                        .generators
                        .generator_region_exclusions
                        .push((marker_start, marker_end));
                }
                if exclude_after_coroutine_suspension
                    && self.control.finally.active_normal_finally_bodies == 0
                {
                    let active_with_count = self.control.active_with_count();
                    for exclusions in self
                        .control
                        .withs
                        .active_with_region_exclusions
                        .iter_mut()
                        .take(active_with_count)
                    {
                        exclusions.push((marker_start, marker_end));
                    }
                }
                if exclude_from_plain_try
                    || (exclude_after_coroutine_suspension
                        && self.control.finally.active_normal_finally_bodies == 0)
                {
                    for exclusions in
                        &mut self.control.exceptions.active_exception_region_exclusions
                    {
                        exclusions.push((marker_start, marker_end));
                    }
                }
            }
            self.assembler
                .set_location(self.source_location(expression.range));
            self.emit(POP_TOP, 0, -1)?;
        }
        if let Expr::BoolOp(inner) = last
            && (inner.op == expression.op
                || folded_bool_operand(last, self.context.python_version).is_none())
        {
            self.compile_bool_expression_at(inner, continuation, false)?;
        } else {
            self.compile_expression_with_boolean_continuation(last, continuation)?;
        }
        // Later expressions start in the Boolean value's original join array.
        self.assembler.preserve_block_boundary_if_new(end);
        self.assembler.record_exception_owner_block_boundary(end);
        self.assembler.mark(end);
        self.set_depth(base_depth + 1);
        Ok(())
    }

    pub(super) fn emit_boolean_comparison(&mut self, operator: CmpOp) -> Result<(), CompileError> {
        let (opcode, argument) = comparison_operator_boolean(operator, self.context.python_version);
        self.emit(opcode, argument, -1)?;
        if matches!(opcode, IS_OP | CONTAINS_OP) {
            // CPython folds the original comparison into its following TO_BOOL slot.
            self.assembler.add_last_hidden_exception_owner_slots(1);
        }
        Ok(())
    }

    /// Emits the shared short-circuit prefix, leaving the final comparison to its caller.
    pub(super) fn compile_conditional_comparison_prefix(
        &mut self,
        comparison: &ruff_python_ast::ExprCompare,
        cleanup: Label,
    ) -> Result<(), CompileError> {
        self.compile_expression(&comparison.left)?;
        for (operator, comparator) in comparison
            .ops
            .iter()
            .zip(&comparison.comparators)
            .take(comparison.ops.len() - 1)
        {
            self.compile_expression(comparator)?;
            self.assembler
                .set_location(self.source_location(comparison.range));
            self.emit(SWAP, 2, 0)?;
            self.emit(COPY, 2, 1)?;
            self.emit_boolean_comparison(*operator)?;
            self.emit_jump_forward(POP_JUMP_IF_FALSE, cleanup, -1)?;
            self.emit(NOT_TAKEN, 0, 0)?;
        }
        Ok(())
    }

    fn compile_compare_expression(
        &mut self,
        expression: &ruff_python_ast::ExprCompare,
    ) -> Result<(), CompileError> {
        self.compile_expression(&expression.left)?;
        if expression.ops.len() == 1 {
            self.compile_comparison_rhs(expression.ops[0], &expression.comparators[0])?;
            let (opcode, argument) =
                comparison_operator(expression.ops[0], self.context.python_version);
            return self.emit(opcode, argument, -1);
        }

        let base_depth = self.control.stack.depth - 1;
        let cleanup = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ValueCompareCleanup(expression.range),
            cleanup,
        );
        for (operator, comparator) in expression
            .ops
            .iter()
            .zip(&expression.comparators)
            .take(expression.ops.len() - 1)
        {
            self.compile_expression(comparator)?;
            self.emit(SWAP, 2, 0)?;
            self.emit(COPY, 2, 1)?;
            let (opcode, argument) = comparison_operator(*operator, self.context.python_version);
            self.emit(opcode, argument, -1)?;
            self.emit(COPY, 1, 1)?;
            self.emit(TO_BOOL, 0, 0)?;
            self.emit_jump_forward(POP_JUMP_IF_FALSE, cleanup, -1)?;
            // CPython appends the chained-comparison marker after compacting the CFG. The
            // generator's stop-iteration owner survives only when this block offers a stale
            // instruction slot for the marker to reuse.
            let exclude_generator_owner = self.control.generators.generator_expression
                && self.control.generators.generator_region_start.is_some()
                && !self.control.generators.generator_filter_expression
                && !self.assembler.current_block_has_dynamic_exception_owner();
            let marker_start = exclude_generator_owner.then(|| {
                let start = self.assembler.label();
                self.assembler.mark(start);
                start
            });
            self.emit(NOT_TAKEN, 0, 0)?;
            // Value comparisons emit COPY/TO_BOOL before optimization. Generator filters
            // use conditional comparison lowering and retain their separate ownership rules.
            if !self.control.generators.generator_filter_expression {
                self.assembler
                    .defer_last_original_condition_exception_owner();
            }
            if let Some(marker_start) = marker_start {
                let marker_end = self.assembler.label();
                self.assembler.mark(marker_end);
                self.control
                    .generators
                    .generator_region_exclusions
                    .push((marker_start, marker_end));
            }
            self.emit(POP_TOP, 0, -1)?;
        }
        let operator = *expression.ops.last().unwrap();
        self.compile_comparison_rhs(operator, expression.comparators.last().unwrap())?;
        let (opcode, argument) = comparison_operator(operator, self.context.python_version);
        self.emit(opcode, argument, -1)?;
        let end = self.assembler.label();
        self.bind_native_source_label(
            super::legacy_native_labels::NativeRole::ValueCompareEnd(expression.range),
            end,
        );
        self.emit_jump_forward(JUMP_FORWARD, end, 0)?;

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 2);
        self.emit(SWAP, 2, 0)?;
        self.emit(POP_TOP, 0, -1)?;
        self.assembler.mark(end);
        self.set_depth(base_depth + 1);
        Ok(())
    }

    pub(super) fn compile_comparison_rhs(
        &mut self,
        operator: CmpOp,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        // LIST_TO_TUPLE disappears only before GET_ITER. Membership keeps the
        // intrinsic, allowing large constant tuples to fold before CONTAINS_OP.
        if matches!(operator, CmpOp::In | CmpOp::NotIn) && !matches!(expression, Expr::Tuple(_)) {
            self.compile_iterable_expression(expression)
        } else {
            self.compile_expression(expression)
        }
    }

    fn emit_condition_not_taken(
        &mut self,
        mut policy: ConditionMarkerPolicy,
    ) -> Result<(), CompileError> {
        if policy.ordinary == ExceptionPolicy::NonePresent
            && self.control.manager_entered_in_current_normal_finally()
        {
            // Each later condition in the manager body can start a fresh original array.
            // Recover its owner only when compaction left a protected reusable slot.
            policy.ordinary = ExceptionPolicy::NoneDynamicConditionStrict;
        }
        if self.control.finally.active_normal_finally_bodies > 0
            && !self.control.withs.active_with_region_exclusions.is_empty()
            && !self.control.conditions.exclude_terminal_if_not_taken
            && !self
                .control
                .conditions
                .exclude_condition_not_taken_from_exception
            && !self.control.withs.force_terminal_with_marker_ownerless
            && let Some(owns_slot) = self.assembler.current_normal_finally_condition_slot_owner(
                &self.control.withs.active_with_region_exclusions,
            )
        {
            // The loop exit's excluded NOP does not split the original array.
            // Its exact prefix distinguishes a reused call slot from a fresh truth marker.
            return self.emit_with_exception_policy(
                NOT_TAKEN,
                0,
                0,
                if owns_slot {
                    policy.ordinary.defer_condition_owner()
                } else {
                    ExceptionPolicy::NoneAbsent
                },
            );
        }
        if (self.control.conditions.allow_compacted_condition_owner
            || (self.control.finally.active_normal_finally_bodies > 0
                && !self.control.withs.active_with_region_exclusions.is_empty()))
            && self.assembler.current_block_starts_at_source_boundary()
            && self.assembler.current_block_has_dynamic_exception_owner()
        {
            // A source entry excludes vacancies from an earlier, differently protected prefix.
            // Include folds introduced by this condition, such as specializing `is None`.
            return self.emit_with_exception_policy(
                NOT_TAKEN,
                0,
                0,
                policy.ordinary.defer_condition_owner(),
            );
        }
        let terminal_try_depth = self
            .control
            .exceptions
            .terminal_try_condition_exception_exclusion_depth;
        let exclude_normal_finally_not_taken_from_with =
            self.control.finally.active_normal_finally_bodies > 0
                && !self.control.withs.active_with_region_exclusions.is_empty()
                && !self
                    .control
                    .withs
                    .include_simple_normal_finally_not_taken_from_with
                && !self.control.manager_entered_in_current_normal_finally();
        if self.control.withs.force_terminal_with_marker_ownerless {
            // A fresh terminal marker has no owner, while early compaction reuses a slot
            // in the current protected block, before any terminal-condition exclusions.
            let exception_policy = if self.assembler.current_block_has_dynamic_exception_owner() {
                policy.ordinary
            } else {
                ExceptionPolicy::EXCLUDE_ALWAYS
            };
            return self.emit_with_exception_policy(NOT_TAKEN, 0, 0, exception_policy);
        }
        if self.control.conditions.exclude_terminal_if_not_taken
            && !self
                .control
                .conditions
                .exclude_condition_not_taken_from_exception
            && terminal_try_depth.is_none()
            && !exclude_normal_finally_not_taken_from_with
            && !self.control.withs.active_with_region_exclusions.is_empty()
            && self.assembler.current_block_has_dynamic_exception_owner()
        {
            // Compaction can leave a protected slot on a terminal branch, including an elif
            // reached after another arm unwinds the manager. Retain that structural owner
            // before the terminal-condition exclusion removes it from the with region.
            self.control.conditions.exclude_terminal_if_not_taken = false;
            self.control
                .withs
                .exclude_terminal_if_not_taken_from_generator = false;
            return self.emit_with_exception_policy(
                NOT_TAKEN,
                0,
                0,
                policy.ordinary.defer_condition_owner(),
            );
        }
        if !self.control.conditions.exclude_terminal_if_not_taken
            && !self
                .control
                .conditions
                .exclude_condition_not_taken_from_exception
            && terminal_try_depth.is_none()
            && !exclude_normal_finally_not_taken_from_with
        {
            return self.emit_with_exception_policy(
                NOT_TAKEN,
                0,
                0,
                policy.ordinary.defer_condition_owner(),
            );
        }

        let exclude_from_generator = (self
            .control
            .conditions
            .exclude_condition_not_taken_from_exception
            || terminal_try_depth.is_some()
            || self
                .control
                .withs
                .exclude_terminal_if_not_taken_from_generator)
            && self.control.generators.generator_region_start.is_some();
        let exclude_from_exception = self
            .control
            .conditions
            .exclude_condition_not_taken_from_exception
            || (self.control.conditions.exclude_terminal_if_not_taken
                && !self.control.withs.active_with_region_exclusions.is_empty());
        self.control.conditions.exclude_terminal_if_not_taken = false;
        self.control
            .withs
            .exclude_terminal_if_not_taken_from_generator = false;

        let exclusion_start = self.assembler.label();
        self.assembler.mark(exclusion_start);
        // Terminal conditions can also reuse an owned original array slot.
        // Resolve vacancies after cleanup, as for ordinary condition markers.
        let exception_policy = if !exclude_normal_finally_not_taken_from_with {
            policy.excluded.defer_condition_owner()
        } else {
            // The normal finalizer's scoped marker exclusion remains authoritative.
            // Proven compaction in its original array uses the earlier owner path.
            policy.excluded
        };
        self.emit_with_exception_policy(NOT_TAKEN, 0, 0, exception_policy)?;
        let exclusion_end = self.assembler.label();
        self.assembler.mark(exclusion_end);

        if exclude_from_generator {
            self.control
                .generators
                .generator_region_exclusions
                .push((exclusion_start, exclusion_end));
        }
        for exclusions in &mut self.control.withs.active_with_region_exclusions {
            exclusions.push((exclusion_start, exclusion_end));
        }
        if exclude_from_exception
            && self
                .control
                .conditions
                .exclude_condition_not_taken_from_all_exception_regions
        {
            for exclusions in &mut self.control.exceptions.active_exception_region_exclusions {
                exclusions.push((exclusion_start, exclusion_end));
            }
        } else {
            if let Some(depth) = terminal_try_depth {
                for exclusions in self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .iter_mut()
                    .take(depth)
                {
                    exclusions.push((exclusion_start, exclusion_end));
                }
            }
            let terminal_try_includes_innermost = terminal_try_depth.is_some_and(|depth| {
                depth
                    >= self
                        .control
                        .exceptions
                        .active_exception_region_exclusions
                        .len()
            });
            if exclude_from_exception
                && !terminal_try_includes_innermost
                && let Some(exclusions) = self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .last_mut()
            {
                exclusions.push((exclusion_start, exclusion_end));
            }
        }
        Ok(())
    }

    fn emit_condition_not_taken_preserving_terminal_try_owner(
        &mut self,
        policy: ConditionMarkerPolicy,
    ) -> Result<(), CompileError> {
        self.with_control_override(
            |control| {
                &mut control
                    .exceptions
                    .terminal_try_condition_exception_exclusion_depth
            },
            None,
            |compiler| compiler.emit_condition_not_taken(policy),
        )
    }

    pub(super) fn compile_linear_assert_condition(
        &mut self,
        expression: &Expr,
        label: Label,
    ) -> Result<(), CompileError> {
        if matches!(expression, Expr::BoolOp(_) | Expr::If(_) | Expr::UnaryOp(_))
            || jump_constant_truthiness(expression).is_some()
            || self
                .control
                .is_ownerless_optimized_boolean_generator_test(expression.range())
            || matches!(
                expression,
                Expr::Compare(comparison)
                    if comparison.ops.len() != 1
                        || matches!(comparison.comparators[0], Expr::NoneLiteral(_))
                            && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
            )
        {
            return self.compile_jump_if(expression, true, label);
        }

        let comparison_is_boolean = if let Expr::Compare(comparison) = expression {
            self.compile_simple_expression_operand(&comparison.left)?;
            if matches!(comparison.ops[0], CmpOp::In | CmpOp::NotIn) {
                self.compile_comparison_rhs(comparison.ops[0], &comparison.comparators[0])?;
            } else {
                self.compile_simple_expression_operand(&comparison.comparators[0])?;
            }
            self.assembler
                .set_location(self.source_location(expression.range()));
            self.emit_boolean_comparison(comparison.ops[0])?;
            true
        } else {
            self.compile_simple_expression_operand(expression)?;
            self.assembler
                .set_location(self.source_location(expression.range()));
            self.emit(TO_BOOL, 0, 0)?;
            false
        };
        self.emit_jump_forward(POP_JUMP_IF_TRUE, label, -1)?;
        self.emit_with_exception_policy(
            NOT_TAKEN,
            0,
            0,
            ExceptionPolicy::normalized_owner(comparison_is_boolean),
        )
    }

    fn compile_simple_expression_operand(&mut self, expression: &Expr) -> Result<(), CompileError> {
        let (name, attribute) = match expression {
            Expr::Name(name)
                if name.ctx == ExprContext::Load && name.id.as_str() != "__debug__" =>
            {
                (name, None)
            }
            Expr::Attribute(attribute)
                if attribute.ctx == ExprContext::Load
                    && matches!(attribute.value.as_ref(), Expr::Name(name)
                        if name.ctx == ExprContext::Load && name.id.as_str() != "__debug__") =>
            {
                let Expr::Name(name) = attribute.value.as_ref() else {
                    unreachable!();
                };
                (name, Some(attribute))
            }
            _ => return self.compile_expression(expression),
        };

        let previous = self.assembler.location();
        self.assembler
            .set_location(self.source_location(name.range()));
        let result = (|| {
            self.load_name(name.id.as_str())?;
            if let Some(attribute) = attribute {
                let index = self.name_index(attribute.attr.as_str())?;
                self.assembler
                    .set_location(self.attribute_opcode_location(attribute));
                self.emit(LOAD_ATTR, index << 1, 0)?;
            }
            Ok(())
        })();
        self.assembler.set_location(previous);
        result
    }

    /// Lowers a runtime conditional with independently addressable arm outcomes.
    pub(super) fn compile_if_expression_jump(
        &mut self,
        conditional: &ruff_python_ast::ExprIf,
        jump_on: bool,
        body_target: Label,
        else_target: Label,
    ) -> Result<(), CompileError> {
        let otherwise = self.assembler.label();
        let end = self.assembler.label();
        if body_target == else_target {
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ConditionIfEnd(conditional.range),
                end,
            );
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ConditionIfElse(conditional.range),
                otherwise,
            );
        }
        self.compile_jump_if(&conditional.test, false, otherwise)?;
        self.assembler.record_short_circuit_owner_continuation();
        self.compile_if_expression_arm_jump(&conditional.body, jump_on, body_target)?;
        self.assembler.set_location(SourceLocation::NONE);
        let body_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, end)?;
        if self.control.conditions.retained_multiline_assert_ifexp_exit == Some(conditional.range) {
            self.assembler.preserve_inlined_jump_nop(body_exit);
        } else {
            // CPython copies the original unlocated join before small branch tails.
            self.assembler.preserve_direct_inlined_jump_nop(body_exit);
        }
        self.assembler
            .record_exception_owner_block_boundary(otherwise);
        self.assembler.mark(otherwise);
        if self
            .control
            .exceptions
            .terminal_try_condition_exception_exclusion_depth
            .is_some()
        {
            self.with_control_override(
                |control| {
                    &mut control
                        .exceptions
                        .terminal_try_condition_exception_exclusion_depth
                },
                None,
                |compiler| {
                    compiler.compile_if_expression_arm_jump(
                        &conditional.orelse,
                        jump_on,
                        else_target,
                    )
                },
            )?;
        } else {
            self.compile_if_expression_arm_jump(&conditional.orelse, jump_on, else_target)?;
        }
        if body_target == else_target
            && jump_constant_truthiness(&conditional.orelse) == Some(jump_on)
        {
            self.assembler.prevent_folded_arm_exit_inlining(
                otherwise,
                else_target,
                self.source_location(conditional.orelse.range()),
            );
        }
        self.assembler.preserve_block_boundary_if_new(end);
        self.assembler.mark(end);
        Ok(())
    }

    /// Keeps legacy constant arms conditional until after exit-block copying.
    fn compile_if_expression_arm_jump(
        &mut self,
        expression: &Expr,
        jump_on: bool,
        label: Label,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313
            && self.return_value_is_constant(expression)
        {
            // CPython copies exits before folding these instructions. Emitting
            // a jump early would copy the target's exception owner and remove
            // the constant NOP that still blocks 3.12 jump threading.
            self.compile_expression(expression)?;
            self.assembler
                .set_location(self.source_location(expression.range()));
            self.emit(TO_BOOL, 0, 0)?;
            self.emit_jump_forward(
                if jump_on {
                    POP_JUMP_IF_TRUE
                } else {
                    POP_JUMP_IF_FALSE
                },
                label,
                -1,
            )?;
            self.emit(NOT_TAKEN, 0, 0)
        } else {
            self.compile_jump_if(expression, jump_on, label)
        }
    }

    pub(super) fn compile_jump_if(
        &mut self,
        expression: &Expr,
        jump_on: bool,
        label: Label,
    ) -> Result<(), CompileError> {
        if self.context.python_version <= PythonVersion::Py313
            && let Some(comparison) = legacy_folded_not_comparison(expression)
        {
            return self.compile_jump_if(&comparison, jump_on, label);
        }
        let legacy_constant = self.context.python_version <= PythonVersion::Py313
            && self.return_value_is_constant(expression);
        if legacy_constant {
            // Keep the original constant and conditional until CFG optimization.
            // An early unconditional jump would expose its target to exit copying
            // and let incoming edges skip the constant's retained line marker.
            return self.compile_if_expression_arm_jump(expression, jump_on, label);
        }
        if let Expr::UnaryOp(unary) = expression
            && unary.op == UnaryOp::Not
            && !legacy_constant
        {
            let defer_terminal_with_exclusion = self.control.withs.terminal_with_condition
                == Some(expression.range())
                && self.control.conditions.exclude_terminal_if_not_taken
                && !self.control.withs.active_with_region_exclusions.is_empty()
                && matches!(unary.operand.as_ref(), Expr::BoolOp(boolean) if boolean.values.len() == 2);
            if defer_terminal_with_exclusion {
                return self.with_control_override(
                    |control| &mut control.withs.defer_terminal_with_exclusion_through_negation,
                    true,
                    |compiler| compiler.compile_jump_if(&unary.operand, !jump_on, label),
                );
            }
            return self.compile_jump_if(&unary.operand, !jump_on, label);
        }
        if let Expr::If(conditional) = expression
            && let Some(truthiness) = early_condition_truthiness(&conditional.test)
        {
            self.record_folded_value(&conditional.test)?;
            self.pre_register_expression_names(&conditional.body)?;
            self.pre_register_expression_names(&conditional.orelse)?;
            self.preserve_folded_if_expression_entry(truthiness);
            if truthiness
                && self.assembler.at_empty_borrow_block()
                && self.source_location(conditional.test.range()).line
                    == self.source_location(conditional.body.range()).line
            {
                // Removing this same-line selector empties its original block.
                // Preserve the separate selected arm after that borrow barrier.
                let empty_selector = self.assembler.label();
                self.assembler.mark(empty_selector);
                self.assembler.prevent_borrow_reachability(empty_selector);
                let selected_arm = self.assembler.label();
                self.assembler.preserve_block_boundary(selected_arm);
                self.assembler.mark(selected_arm);
            }
            let preserve_multiline_failure_join = truthiness
                && self.control.conditions.retained_multiline_assert_ifexp_exit
                    == Some(conditional.range);
            if preserve_multiline_failure_join
                && self.source_location(conditional.test.range()).line
                    != self.source_location(conditional.body.range()).line
            {
                // The selected arm starts on another line, retaining the folded selector marker.
                self.assembler
                    .set_location(self.source_location(conditional.test.range()));
                self.emit(NOP, 0, 0)?;
            }
            self.compile_jump_if(
                if truthiness {
                    &conditional.body
                } else {
                    &conditional.orelse
                },
                jump_on,
                label,
            )?;
            if preserve_multiline_failure_join {
                // The first arm's removed forward join keeps its multiline assertion marker.
                self.assembler
                    .set_location(self.source_location(conditional.body.range()));
                self.emit(NOP, 0, 0)?;
                self.assembler.preserve_last_nop();
            }
            return Ok(());
        }
        let constant_truthiness = if legacy_constant {
            self.source_aware_fold_constant(expression)
                .as_ref()
                .map(Constant::is_truthy)
        } else {
            jump_constant_truthiness(expression)
        };
        if let Some(truthiness) = constant_truthiness {
            let empty_folded_block = self.assembler.at_empty_borrow_block();
            self.record_folded_value(expression)?;
            self.assembler
                .set_location(self.source_location(expression.range()));
            if truthiness == jump_on {
                // The folded conditional keeps its own operand span. A preceding
                // same-line constant NOP is removed without donating its columns.
                let exit =
                    self.emit_jump_forward_instruction_preserving_location(JUMP_FORWARD, label)?;
                self.assembler.preserve_inlined_jump_nop(exit);
                self.assembler
                    .mark_folded_jump_borrow_fallthrough(exit, empty_folded_block);
                if matches!(
                    self.context.python_version,
                    PythonVersion::Py314 | PythonVersion::Py315
                ) && self.control.conditions.uncopied_assertion_success == Some(label)
                {
                    // This edge becomes unconditional only after CPython's early
                    // copy pass; a located assertion continuation stays shared.
                    self.assembler.prevent_jump_inlining(exit);
                }
            } else {
                // Native line resolution still sees this edge before constant folding.
                self.assembler.record_folded_condition_predecessor(label);
                let exclusion_start = self.assembler.label();
                self.assembler.mark(exclusion_start);
                self.emit(NOP, 0, 0)?;
                self.assembler
                    .record_folded_predecessor_source(exclusion_start, label);
                if empty_folded_block {
                    // Removing the sole folded condition empties its original array.
                    // A retained line NOP still permits borrowing through that array.
                    self.assembler.mark_last_nop_borrow_barrier_if_removed();
                }
                let exclusion_end = self.assembler.label();
                // The folded conditional still leaves its original fallthrough block.
                self.assembler
                    .record_exception_owner_block_boundary(exclusion_end);
                self.assembler.mark(exclusion_end);
                // CPython assigns the condition's handler before folding its instructions.
                // A surviving source-line NOP retains that owner.
            }
            return Ok(());
        }
        if let Expr::BoolOp(boolean) = expression {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            let operands_are_negated =
                self.control.withs.terminal_with_condition == Some(expression.range())
                    && matches!(boolean.op, BoolOp::Or)
                    && !jump_on
                    && boolean.values.iter().all(
                        |value| matches!(value, Expr::UnaryOp(unary) if unary.op == UnaryOp::Not),
                    );
            let defer_terminal_with_exclusion = leading.len() == 1
                && self.control.conditions.exclude_terminal_if_not_taken
                && !self.control.withs.active_with_region_exclusions.is_empty()
                && (operands_are_negated
                    || self
                        .control
                        .withs
                        .defer_terminal_with_exclusion_through_negation);
            let preserve_leading_protected_loop_owner =
                self.control.loops.protected_iterator_continue_condition
                    == Some(expression.range());
            let short_circuit_jumps_to_target = matches!(boolean.op, BoolOp::Or) == jump_on;
            if short_circuit_jumps_to_target {
                for (operand_index, value) in leading.iter().enumerate() {
                    if defer_terminal_with_exclusion {
                        self.with_control_override(
                            |control| &mut control.conditions.exclude_terminal_if_not_taken,
                            false,
                            |compiler| compiler.compile_jump_if(value, jump_on, label),
                        )?;
                    } else {
                        self.compile_jump_if(value, jump_on, label)?;
                    }
                    // Short-circuiting leaves the remaining operands as empty original
                    // blocks. Stop borrowing at their destination while preserving the
                    // constant NOP and any earlier edge that could alias this entry.
                    if jump_constant_truthiness(value) == Some(matches!(boolean.op, BoolOp::Or)) {
                        self.assembler.record_folded_condition_borrow_entry(label);
                    }
                    if preserve_leading_protected_loop_owner && operand_index == 0 {
                        self.assembler.preserve_last_not_taken_exception_owner();
                    }
                    self.assembler.record_short_circuit_owner_continuation();
                }
                return self.compile_jump_if(last, jump_on, label);
            }
            let end = self.assembler.label();
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ConditionBoolEnd(boolean.range),
                end,
            );
            for (operand_index, value) in leading.iter().enumerate() {
                if defer_terminal_with_exclusion {
                    self.with_control_override(
                        |control| &mut control.conditions.exclude_terminal_if_not_taken,
                        false,
                        |compiler| compiler.compile_jump_if(value, !jump_on, end),
                    )?;
                } else {
                    self.compile_jump_if(value, !jump_on, end)?;
                }
                if jump_constant_truthiness(value) == Some(matches!(boolean.op, BoolOp::Or)) {
                    self.assembler.record_folded_condition_borrow_entry(end);
                }
                if preserve_leading_protected_loop_owner && operand_index == 0 {
                    self.assembler.preserve_last_not_taken_exception_owner();
                }
                self.assembler.record_short_circuit_owner_continuation();
            }
            self.compile_jump_if(last, jump_on, label)?;
            // The final operand may already end at a pending join label. Keep
            // those labels in one block so small return copies see its body.
            self.assembler.preserve_block_boundary_if_new(end);
            self.assembler.mark(end);
            // Native codegen_jump_if marks this existing short-circuit join.
            self.assembler.record_exception_owner_block_boundary(end);
            return Ok(());
        }
        if let Expr::If(conditional) = expression {
            return self.compile_if_expression_jump(conditional, jump_on, label, label);
        }
        if let Expr::Compare(comparison) = expression
            && comparison.ops.len() > 1
        {
            let base_depth = self.control.stack.depth;
            let cleanup = self.assembler.label();
            let end = self.assembler.label();
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ConditionCompareCleanup(comparison.range),
                cleanup,
            );
            self.compile_conditional_comparison_prefix(comparison, cleanup)?;

            let final_comparator = comparison.comparators.last().unwrap();
            let final_operator = *comparison.ops.last().unwrap();
            let specialized_none_jump = matches!(final_comparator, Expr::NoneLiteral(_))
                && matches!(final_operator, CmpOp::Is | CmpOp::IsNot);
            if specialized_none_jump {
                self.add_constant(Constant::None)?;
                let none_location = self.source_location(final_comparator.range());
                if self.context.python_version >= PythonVersion::Py314
                    && none_location.line != self.source_location(comparison.range).line
                {
                    // Specializing None leaves its original operand-line cell until NOP cleanup.
                    self.assembler.set_location(none_location);
                    self.emit(NOP, 0, 0)?;
                }
            } else {
                self.compile_comparison_rhs(final_operator, final_comparator)?;
            }
            self.assembler
                .set_location(self.source_location(comparison.range));
            if specialized_none_jump {
                let jump_if_none = jump_on == (final_operator == CmpOp::Is);
                self.emit_jump_forward(
                    if jump_if_none {
                        POP_JUMP_IF_NONE
                    } else {
                        POP_JUMP_IF_NOT_NONE
                    },
                    label,
                    -1,
                )?;
            } else {
                self.emit_boolean_comparison(final_operator)?;
                self.emit_jump_forward(
                    if jump_on {
                        POP_JUMP_IF_TRUE
                    } else {
                        POP_JUMP_IF_FALSE
                    },
                    label,
                    -1,
                )?;
            }
            self.emit(NOT_TAKEN, 0, 0)?;
            if self.context.python_version <= PythonVersion::Py313 {
                self.assembler.set_location(SourceLocation::NONE);
            }
            // Native reserves End here, after operand labels. Its source plan
            // is independent of the earlier physical label allocation above.
            self.bind_native_source_label(
                super::legacy_native_labels::NativeRole::ConditionCompareEnd(comparison.range),
                end,
            );
            let comparison_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, end)?;
            if self.control.conditions.retained_chained_comparison_exit == Some(comparison.range) {
                // CPython retains this final comparison fallthrough jump as a NOP when its
                // neighboring terminal exit is copied through the optimized CFG.
                self.assembler.preserve_inlined_jump_nop(comparison_exit);
            } else if !jump_on {
                self.assembler.prevent_jump_inlining(comparison_exit);
            }

            self.assembler.mark(cleanup);
            self.set_depth(base_depth + 1);
            if self.context.python_version <= PythonVersion::Py313 {
                self.assembler
                    .set_location(self.source_location(comparison.range));
            }
            self.emit(POP_TOP, 0, -1)?;
            if !jump_on {
                if self.context.python_version <= PythonVersion::Py313 {
                    self.assembler.set_location(SourceLocation::NONE);
                }
                let cleanup_exit = self.emit_jump_forward_instruction(JUMP_FORWARD, label)?;
                if self.control.statements.in_terminal_if_branch_suite
                    && self.control.conditions.retained_chained_comparison_exit
                        == Some(comparison.range)
                {
                    // The cleanup and false edges must continue sharing a small following exit.
                    self.assembler.prevent_jump_inlining(cleanup_exit);
                }
            }

            self.assembler.mark(end);
            // Native marks this existing join after emitting the comparison cleanup.
            self.assembler.record_exception_owner_block_boundary(end);
            if self.context.python_version >= PythonVersion::Py314 && !jump_on {
                self.assembler
                    .record_original_comparison_fallthrough(cleanup, end);
            }
            self.set_depth(base_depth);
            return Ok(());
        }
        if let Expr::Compare(comparison) = expression
            && comparison.ops.len() == 1
            && matches!(comparison.comparators[0], Expr::NoneLiteral(_))
            && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
        {
            self.compile_expression(&comparison.left)?;
            self.add_constant(Constant::None)?;
            let left_location = self.source_location(comparison.left.range());
            let none_location = self.source_location(comparison.comparators[0].range());
            // CPython compacts the specialized LOAD_CONST(None) against the
            // preceding instruction in its original block, whose line need not
            // be the start of a multiline operand (for example, LOAD_ATTR).
            let left_line = self
                .assembler
                .current_block_last_instruction_location()
                .map_or(left_location.line, |location| location.line);
            if left_line != none_location.line {
                self.assembler.set_location(none_location);
                self.emit(NOP, 0, 0)?;
            }
            self.assembler
                .set_location(self.source_location(expression.range()));
            let jump_if_none = jump_on == (comparison.ops[0] == CmpOp::Is);
            self.emit_jump_forward(
                if jump_if_none {
                    POP_JUMP_IF_NONE
                } else {
                    POP_JUMP_IF_NOT_NONE
                },
                label,
                -1,
            )?;
            // Specializing `is None` removes LOAD_CONST, IS_OP, and TO_BOOL before
            // normalization appends the fallthrough marker. A multiline None literal keeps
            // one of those slots as its source-location NOP.
            self.assembler.add_last_hidden_exception_owner_slots(
                3 - usize::from(left_line != none_location.line),
            );
            if left_line == none_location.line {
                self.assembler.record_source_none_condition();
            }
            // CPython keeps this specialized fallthrough in the surrounding generator's
            // stop-iteration region. Override the generic condition exclusion only when no
            // nested control-flow region can own the marker instead.
            let preserve_generator_stop_iteration_owner =
                self.control.generators.generator_region_start.is_some()
                    && self
                        .control
                        .comprehensions
                        .active_comprehension_region_exclusions
                        .is_empty()
                    && self.control.withs.active_with_region_exclusions.is_empty()
                    && self
                        .control
                        .exceptions
                        .active_exception_region_exclusions
                        .is_empty()
                    && self.control.finally.active_normal_finally_bodies == 0;
            // CPython specializes this condition by removing LOAD_CONST(None), IS_OP, and
            // TO_BOOL before appending NOT_TAKEN. Preserve the owner only when optimization can
            // reuse a stale protected slot in the current block.
            let marker_reuses_exception_owner =
                self.assembler.current_block_has_dynamic_exception_owner();
            let preserve_sync_with_short_circuit_none_owner = self
                .control
                .current_synchronous_with_body_start_item()
                .is_some_and(|start| {
                    self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR | CO_GENERATOR) == 0
                        && !self.control.in_loop()
                        && !self.control.in_synchronous_with_nested_exception_region()
                        && self.assembler.contains_opcode_since_item(NOT_TAKEN, start)
                });
            let policy = ConditionMarkerPolicy {
                excluded: if preserve_sync_with_short_circuit_none_owner {
                    ExceptionPolicy::normalized_owner(true)
                } else {
                    ExceptionPolicy::NoneDynamicCondition
                },
                ordinary: ExceptionPolicy::normalized_owner(
                    self.control.generators.generator_region_start.is_some()
                        || self.control.conditions.include_assert_none_condition_owner
                        || self.control.conditions.include_scoped_none_condition_owner
                        || self
                            .control
                            .synchronous_with_preserves_none_condition_owner()
                        || self
                            .control
                            .finally_try_body_preserves_none_condition_owner()
                        || preserve_sync_with_short_circuit_none_owner
                        || self.control.in_first_unnamed_exception_handler()
                        || self.control.finally.active_normal_finally_bodies > 0
                        || marker_reuses_exception_owner,
                ),
            };
            return if preserve_generator_stop_iteration_owner {
                self.with_control_override(
                    |control| {
                        &mut control
                            .conditions
                            .exclude_condition_not_taken_from_exception
                    },
                    false,
                    |compiler| {
                        compiler.emit_condition_not_taken_preserving_terminal_try_owner(policy)
                    },
                )
            } else {
                self.emit_condition_not_taken_preserving_terminal_try_owner(policy)
            };
        }
        let comparison_is_boolean = if let Expr::Compare(comparison) = expression
            && comparison.ops.len() == 1
        {
            self.compile_expression(&comparison.left)?;
            self.compile_comparison_rhs(comparison.ops[0], &comparison.comparators[0])?;
            self.assembler
                .set_location(self.source_location(expression.range()));
            self.emit_boolean_comparison(comparison.ops[0])?;
            true
        } else {
            self.compile_expression(expression)?;
            false
        };
        if !comparison_is_boolean {
            self.assembler
                .set_location(self.source_location(expression.range()));
            self.emit(TO_BOOL, 0, 0)?;
        }
        self.emit_jump_forward(
            if jump_on {
                POP_JUMP_IF_TRUE
            } else {
                POP_JUMP_IF_FALSE
            },
            label,
            -1,
        )?;
        if comparison_is_boolean
            || matches!(
                expression,
                Expr::Name(_) | Expr::Attribute(_) | Expr::Call(_)
            )
        {
            // The direct codegen_jump_if path ends an original CP array. A single
            // comparison can compact its predecessor without changing this boundary.
            // Both its target and normalized backward fallthrough are original
            // boundaries, unlike analysis-only labels or copied jump targets.
            self.assembler
                .record_original_primitive_condition_boundary(label);
        }
        let comparison_marker_owner = if self.control.loops.loop_tail_while_condition {
            ExceptionPolicy::default()
        } else {
            ExceptionPolicy::normalized_owner(true)
        };
        let preserve_iterator_loop_assertion_owner = self
            .control
            .loops
            .preserve_first_iterator_loop_assertion_owner;
        // A direct attribute-call assertion can reuse a compacted protected slot in a with body.
        // Keep the block scan behind the cheap assertion and call guards.
        let preserve_sync_with_assertion_owner = self
            .control
            .conditions
            .include_assert_none_condition_owner
            && !self.control.withs.active_with_region_exclusions.is_empty()
            && matches!(expression, Expr::Call(call) if matches!(call.func.as_ref(), Expr::Attribute(_)))
            && self.assembler.current_block_has_dynamic_exception_owner();
        let policy = ConditionMarkerPolicy {
            excluded: if comparison_is_boolean {
                comparison_marker_owner
            } else {
                ExceptionPolicy::normalized_owner(false)
            },
            ordinary: if comparison_is_boolean {
                comparison_marker_owner
            } else {
                ExceptionPolicy::normalized_owner(
                    self.control.finally.active_normal_finally_bodies > 0
                        || preserve_iterator_loop_assertion_owner
                        || preserve_sync_with_assertion_owner,
                )
            },
        };
        let preserve_terminal_try_owner = comparison_is_boolean
            || (self
                .control
                .exceptions
                .terminal_try_condition_exception_exclusion_depth
                .is_some()
                && self.assembler.current_block_has_dynamic_exception_owner());
        let shifted_plain_try_marker = !comparison_is_boolean
            && self
                .control
                .current_try_marker_is_shifted_to_ownerless_slot(
                    &self.assembler,
                    expression.range(),
                );
        let exclude_post_async_with_marker = !comparison_is_boolean
            && self.control.withs.post_async_with_condition_marker == Some(expression.range())
            && !self.assembler.current_block_has_dynamic_exception_owner();
        if exclude_post_async_with_marker {
            return self.emit_condition_not_taken(ConditionMarkerPolicy {
                excluded: ExceptionPolicy::EXCLUDE_ALWAYS,
                ordinary: ExceptionPolicy::EXCLUDE_ALWAYS,
            });
        }
        let exclude_plain_try_marker = !comparison_is_boolean
            && (shifted_plain_try_marker || {
                let nested_try_ifexp_marker = self
                    .control
                    .current_try_marker_uses_concrete_owner_slots(expression.range());
                let retained_entry_setups = self
                    .control
                    .exceptions
                    .active_exception_region_exclusions
                    .len()
                    > 1
                    && self.assembler.entry_has_retained_exception_setups();
                !(if nested_try_ifexp_marker || retained_entry_setups {
                    self.assembler
                        .fallthrough_has_concrete_dynamic_exception_owner()
                } else {
                    self.assembler.fallthrough_has_dynamic_exception_owner()
                }) && self.control.generators.generator_region_start.is_none()
                    && !self.control.in_iterator_loop()
                    && !self.control.finally.exceptional_finally_condition
                    && self
                        .control
                        .comprehensions
                        .active_comprehension_region_exclusions
                        .is_empty()
                    && self.control.withs.active_with_region_exclusions.is_empty()
                    && self.control.finally.active_normal_finally_bodies == 0
                    && self.control.finally.active_finally_end_blocks == 0
                    && self.control.exceptions.active_exception_group_handlers == 0
                    && !self.control.exceptions.same_line_try_body
                    && !self.control.current_exception_handler_binds_name()
                    && !self.control.in_exception_handler()
                    && (self
                        .control
                        .exceptions
                        .active_exception_region_exclusions
                        .len()
                        == 1
                        || nested_try_ifexp_marker
                        || retained_entry_setups)
            });
        if exclude_plain_try_marker {
            // CPython appends normalized markers after exception targets are labeled. Force the
            // plain try marker ownerless only when no optimized CFG slot can carry an owner.
            self.emit_condition_not_taken(ConditionMarkerPolicy {
                excluded: ExceptionPolicy::EXCLUDE_ALWAYS,
                ordinary: ExceptionPolicy::EXCLUDE_ALWAYS,
            })
        } else if preserve_terminal_try_owner {
            self.emit_condition_not_taken_preserving_terminal_try_owner(policy)
        } else {
            self.emit_condition_not_taken(policy)
        }
    }

    pub(super) fn compile_store_target(&mut self, expression: &Expr) -> Result<(), CompileError> {
        let previous = self.assembler.location();
        self.assembler
            .set_location(self.source_location(expression.range()));
        let result = self.compile_store_target_inner(expression);
        self.assembler.set_location(previous);
        result
    }

    fn compile_store_target_inner(&mut self, expression: &Expr) -> Result<(), CompileError> {
        match expression {
            Expr::Name(name) => self.store_name(name.id.as_str()),
            Expr::Attribute(attribute) => {
                if matches!(attribute.value.as_ref(), Expr::Name(name) if name.id.as_str() == "self")
                {
                    self.context.record_static_attribute(
                        attribute.attr.as_str(),
                        matches!(self.unit.scope, Scope::Class { .. }),
                    );
                }
                self.compile_expression(&attribute.value)?;
                let index = self.name_index(attribute.attr.as_str())?;
                self.assembler
                    .set_location(self.attribute_opcode_location(attribute));
                self.emit(STORE_ATTR, index, -2)
            }
            Expr::Subscript(subscript) => {
                self.compile_expression(&subscript.value)?;
                if let Expr::Slice(slice) = subscript.slice.as_ref()
                    && two_element_slice_optimization(slice, self.context.python_version)
                {
                    let slice_location = self.source_location(slice.range);
                    self.compile_optional_slice_bound(slice.lower.as_deref(), slice_location)?;
                    self.compile_optional_slice_bound(slice.upper.as_deref(), slice_location)?;
                    self.assembler
                        .set_location(self.source_location(subscript.range));
                    self.emit(STORE_SLICE, 0, -4)
                } else {
                    self.compile_expression(&subscript.slice)?;
                    self.assembler
                        .set_location(self.source_location(subscript.range));
                    self.emit(STORE_SUBSCR, 0, -3)
                }
            }
            Expr::List(list) => self.compile_unpack_target(&list.elts),
            Expr::Tuple(tuple) => self.compile_unpack_target(&tuple.elts),
            Expr::Starred(starred) => self.compile_store_target(&starred.value),
            _ => Err(unsupported("assignment target")),
        }
    }

    fn compile_unpack_target(&mut self, elements: &[Expr]) -> Result<(), CompileError> {
        let starred = elements.iter().position(Expr::is_starred_expr);
        if let Some(starred) = starred {
            let before = to_u32(starred, "unpack target count")?;
            let after = to_u32(elements.len() - starred - 1, "unpack target count")?;
            self.emit(
                UNPACK_EX,
                before | (after << 8),
                i32::try_from(elements.len()).unwrap() - 1,
            )?;
        } else {
            let count = to_u32(elements.len(), "unpack target count")?;
            self.emit(
                UNPACK_SEQUENCE,
                count,
                i32::try_from(elements.len()).unwrap() - 1,
            )?;
        }
        for element in elements {
            self.compile_store_target(element)?;
        }
        Ok(())
    }

    pub(super) fn compile_delete_target(&mut self, expression: &Expr) -> Result<(), CompileError> {
        self.assembler
            .set_location(self.source_location(expression.range()));
        match expression {
            Expr::Name(name) => self.delete_name(name.id.as_str()),
            Expr::Attribute(attribute) => {
                self.compile_expression(&attribute.value)?;
                let index = self.name_index(attribute.attr.as_str())?;
                self.emit(DELETE_ATTR, index, -1)
            }
            Expr::Subscript(subscript) => {
                self.compile_expression(&subscript.value)?;
                self.compile_expression(&subscript.slice)?;
                self.emit(DELETE_SUBSCR, 0, -2)
            }
            Expr::List(list) => {
                for element in &list.elts {
                    self.compile_delete_target(element)?;
                }
                Ok(())
            }
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.compile_delete_target(element)?;
                }
                Ok(())
            }
            _ => Err(unsupported("deletion target")),
        }
    }
}

impl CollectionKind {
    const fn build_opcode(self) -> Opcode {
        match self {
            Self::List => BUILD_LIST,
            Self::Tuple => BUILD_TUPLE,
            Self::Set => BUILD_SET,
        }
    }

    const fn build_opcode_for_unpacking(self) -> Opcode {
        match self {
            Self::List | Self::Tuple => BUILD_LIST,
            Self::Set => BUILD_SET,
        }
    }

    const fn append_opcode(self) -> Opcode {
        match self {
            Self::List | Self::Tuple => LIST_APPEND,
            Self::Set => SET_ADD,
        }
    }

    const fn extend_opcode(self) -> Opcode {
        match self {
            Self::List | Self::Tuple => LIST_EXTEND,
            Self::Set => SET_UPDATE,
        }
    }
}

/// Mirrors the legacy AST optimizer, including the inner comparison's source range.
fn legacy_folded_not_comparison(expression: &Expr) -> Option<Expr> {
    let (comparison, operator) = legacy_not_comparison(expression)?;
    let mut comparison = comparison.clone();
    comparison.ops[0] = operator;
    Some(Expr::Compare(comparison))
}

/// Identify the single comparison and operator left by legacy `not` folding.
fn legacy_not_comparison(expression: &Expr) -> Option<(&ruff_python_ast::ExprCompare, CmpOp)> {
    let Expr::UnaryOp(_) = expression else {
        return None;
    };
    let mut operand = expression;
    let mut negated = false;
    while let Expr::UnaryOp(unary) = operand {
        if unary.op != UnaryOp::Not {
            return None;
        }
        negated = !negated;
        operand = &unary.operand;
    }
    let Expr::Compare(comparison) = operand else {
        return None;
    };
    let [operator] = comparison.ops.as_ref() else {
        return None;
    };
    let inverse = match operator {
        CmpOp::Is => CmpOp::IsNot,
        CmpOp::IsNot => CmpOp::Is,
        CmpOp::In => CmpOp::NotIn,
        CmpOp::NotIn => CmpOp::In,
        _ => return None,
    };
    Some((comparison, if negated { inverse } else { *operator }))
}

pub(super) fn expression_contains_yield(expression: &Expr) -> bool {
    #[derive(Default)]
    struct Collector {
        found: bool,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(expression, Expr::Lambda(_)) {
                return;
            }
            if matches!(expression, Expr::Yield(_) | Expr::YieldFrom(_)) {
                self.found = true;
                return;
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(expression);
    collector.found
}

fn comprehension_filter_has_root_if_expression(expression: &Expr) -> bool {
    match expression {
        Expr::If(_) => true,
        Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => {
            comprehension_filter_has_root_if_expression(&unary.operand)
        }
        Expr::BoolOp(boolean) => boolean
            .values
            .iter()
            .any(comprehension_filter_has_root_if_expression),
        _ => false,
    }
}

fn comprehension_condition_has_trailing_none(expression: &Expr) -> bool {
    match expression {
        Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => {
            comprehension_condition_has_trailing_none(&unary.operand)
        }
        Expr::BoolOp(boolean) => boolean
            .values
            .last()
            .is_some_and(comprehension_condition_has_trailing_none),
        Expr::Compare(comparison) => {
            comparison.ops.len() == 1
                && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
                && matches!(comparison.comparators[0], Expr::NoneLiteral(_))
        }
        _ => false,
    }
}

fn is_name_rooted_if_expression_condition(expression: &Expr) -> bool {
    let mut root = expression;
    while let Expr::Attribute(attribute) = root {
        root = attribute.value.as_ref();
    }
    matches!(root, Expr::Name(_))
}

fn expression_contains_inlined_comprehension(expression: &Expr) -> bool {
    #[derive(Default)]
    struct Collector {
        found: bool,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.found || matches!(expression, Expr::Lambda(_) | Expr::Generator(_)) {
                return;
            }
            if matches!(
                expression,
                Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_)
            ) {
                self.found = true;
            } else {
                walk_expr(self, expression);
            }
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(expression);
    collector.found
}

// The outermost iterable runs in the enclosing scope. Only suspension in the
// generator itself makes CPython skip its synchronous builtin-call specialization.
fn generator_is_async(generator: &ruff_python_ast::ExprGenerator) -> bool {
    generator
        .generators
        .iter()
        .enumerate()
        .any(|(index, generator)| {
            generator.is_async
                || (index > 0 && expression_suspends_current_scope(&generator.iter))
                || expression_suspends_current_scope(&generator.target)
                || generator.ifs.iter().any(expression_suspends_current_scope)
        })
        || expression_suspends_current_scope(&generator.elt)
}

pub(super) fn optimized_generator_callable(call: &ruff_python_ast::ExprCall) -> Option<&str> {
    optimized_generator_callable_for_target(call, PythonVersion::Py314)
}

pub(super) fn optimized_generator_callable_for_target(
    call: &ruff_python_ast::ExprCall,
    python_version: PythonVersion,
) -> Option<&str> {
    if python_version <= PythonVersion::Py313 {
        return None;
    }
    let Expr::Name(name) = call.func.as_ref() else {
        return None;
    };
    if !(matches!(name.id.as_str(), "all" | "any" | "tuple")
        || (python_version == PythonVersion::Py315 && matches!(name.id.as_str(), "list" | "set")))
        || !matches!(call.arguments.args.as_ref(), [Expr::Generator(generator)]
            if !generator_is_async(generator))
        || !call.arguments.keywords.is_empty()
    {
        return None;
    }
    Some(name.id.as_str())
}

pub(super) fn expression_suspends_current_scope(expression: &Expr) -> bool {
    #[derive(Default)]
    struct Collector {
        found: bool,
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.found {
                return;
            }
            match expression {
                Expr::Generator(generator) => {
                    if let Some(first) = generator.generators.first() {
                        self.visit_expr(&first.iter);
                    }
                    return;
                }
                Expr::Lambda(lambda) => {
                    if let Some(parameters) = lambda.parameters.as_deref() {
                        for default in parameter_default_expressions(parameters) {
                            self.visit_expr(default);
                        }
                    }
                    return;
                }
                _ => {}
            }
            if matches!(expression, Expr::Await(_)) {
                self.found = true;
                return;
            }
            let has_async_comprehension = match expression {
                Expr::ListComp(comprehension) => comprehension
                    .generators
                    .iter()
                    .any(|generator| generator.is_async),
                Expr::SetComp(comprehension) => comprehension
                    .generators
                    .iter()
                    .any(|generator| generator.is_async),
                Expr::DictComp(comprehension) => comprehension
                    .generators
                    .iter()
                    .any(|generator| generator.is_async),
                _ => false,
            };
            if has_async_comprehension {
                self.found = true;
                return;
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_expr(expression);
    collector.found
}
pub(super) fn expression_defers_async_comprehension_restore(expression: &Expr) -> bool {
    match expression {
        Expr::ListComp(comprehension) => comprehension
            .generators
            .iter()
            .any(|generator| generator.is_async),
        Expr::SetComp(comprehension) => comprehension
            .generators
            .iter()
            .any(|generator| generator.is_async),
        Expr::DictComp(comprehension) => comprehension
            .generators
            .iter()
            .any(|generator| generator.is_async),
        _ => false,
    }
}
pub(super) fn literal_truthiness(expression: &Expr) -> Option<bool> {
    match expression {
        Expr::Name(name) if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__" => {
            Some(true)
        }
        Expr::NoneLiteral(_) => Some(false),
        Expr::BooleanLiteral(value) => Some(value.value),
        Expr::NumberLiteral(number) => match &number.value {
            Number::Int(value) => Some(value.as_u64() != Some(0)),
            Number::Float(value) => Some(*value != 0.0),
            Number::Complex { real, imag } => Some(*real != 0.0 || *imag != 0.0),
        },
        Expr::StringLiteral(value) => Some(!value.value.is_empty()),
        Expr::BytesLiteral(value) => Some(!value.value.is_empty()),
        Expr::FString(_) => match fold_constant(expression)? {
            Constant::String(value) => Some(!value.is_empty()),
            _ => None,
        },
        Expr::Tuple(value) => Some(!value.elts.is_empty()),
        _ => None,
    }
}

fn folded_elements_owner_slots(
    python_version: PythonVersion,
    elements: &[Expr],
    structural_slots: usize,
) -> usize {
    structural_slots
        + elements
            .iter()
            .map(|element| folded_constant_owner_slots(element, python_version))
            .sum::<usize>()
}

pub(super) fn jump_constant_truthiness(expression: &Expr) -> Option<bool> {
    match expression {
        Expr::EllipsisLiteral(_) => Some(true),
        Expr::Name(name) if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__" => {
            Some(true)
        }
        Expr::NoneLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::FString(_) => literal_truthiness(expression),
        _ => None,
    }
}

pub(super) fn early_condition_truthiness(expression: &Expr) -> Option<bool> {
    if matches!(expression, Expr::Tuple(_)) {
        None
    } else {
        literal_truthiness(expression)
    }
}

// Returns the leaf that reaches a nested conditional expression's join by fallthrough.
// A short-circuit leaf has multiple predecessors, so CPython keeps the join source-less.
fn conditional_expression_fallthrough(expression: &ExprIf) -> Option<&Expr> {
    let branch = if early_condition_truthiness(&expression.test) == Some(true) {
        expression.body.as_ref()
    } else {
        expression.orelse.as_ref()
    };
    match branch {
        Expr::If(nested) => conditional_expression_fallthrough(nested),
        Expr::BoolOp(_) => None,
        _ => Some(branch),
    }
}

fn comprehension_filter_truthiness(expression: &Expr) -> Option<bool> {
    if let Expr::UnaryOp(unary) = expression
        && unary.op == UnaryOp::Not
    {
        jump_constant_truthiness(&unary.operand).map(|truthiness| !truthiness)
    } else {
        early_condition_truthiness(expression)
    }
}
pub(super) fn binary_operator(operator: Operator, inplace: bool) -> BinaryOperation {
    let operation = match operator {
        Operator::Add => BinaryOperation::ADD,
        Operator::BitAnd => BinaryOperation::AND,
        Operator::FloorDiv => BinaryOperation::FLOOR_DIVIDE,
        Operator::LShift => BinaryOperation::LSHIFT,
        Operator::MatMult => BinaryOperation::MATRIX_MULTIPLY,
        Operator::Mult => BinaryOperation::MULTIPLY,
        Operator::Mod => BinaryOperation::REMAINDER,
        Operator::BitOr => BinaryOperation::OR,
        Operator::Pow => BinaryOperation::POWER,
        Operator::RShift => BinaryOperation::RSHIFT,
        Operator::Sub => BinaryOperation::SUBTRACT,
        Operator::Div => BinaryOperation::TRUE_DIVIDE,
        Operator::BitXor => BinaryOperation::XOR,
    };
    if inplace {
        operation.inplace()
    } else {
        operation
    }
}

pub(super) fn comparison_operator(operator: CmpOp, python_version: PythonVersion) -> (Opcode, u32) {
    let (opcode, argument) = match operator {
        CmpOp::Lt => (COMPARE_OP, ComparisonOperation::LT.argument()),
        CmpOp::LtE => (COMPARE_OP, ComparisonOperation::LE.argument()),
        CmpOp::Eq => (COMPARE_OP, ComparisonOperation::EQ.argument()),
        CmpOp::NotEq => (COMPARE_OP, ComparisonOperation::NE.argument()),
        CmpOp::Gt => (COMPARE_OP, ComparisonOperation::GT.argument()),
        CmpOp::GtE => (COMPARE_OP, ComparisonOperation::GE.argument()),
        CmpOp::Is => (IS_OP, 0),
        CmpOp::IsNot => (IS_OP, 1),
        CmpOp::In => (CONTAINS_OP, 0),
        CmpOp::NotIn => (CONTAINS_OP, 1),
    };
    if opcode == COMPARE_OP && python_version == PythonVersion::Py312 {
        // Python 3.12 stores the comparison index above four mask bits; later
        // targets insert a fifth bit that requests a Boolean result.
        (opcode, ((argument >> 5) << 4) | (argument & 15))
    } else {
        (opcode, argument)
    }
}

pub(super) fn comparison_operator_boolean(
    operator: CmpOp,
    python_version: PythonVersion,
) -> (Opcode, u32) {
    let (opcode, argument) = comparison_operator(operator, python_version);
    if python_version != PythonVersion::Py312
        && matches!(
            operator,
            CmpOp::Lt | CmpOp::LtE | CmpOp::Eq | CmpOp::NotEq | CmpOp::Gt | CmpOp::GtE
        )
    {
        (opcode, ComparisonOperation::force_boolean(argument))
    } else {
        (opcode, argument)
    }
}

/// Finds earlier case arms whose nested conditional retains an original jump into the join.
fn match_case_has_nested_jump_predecessor(statement: &ruff_python_ast::StmtIf) -> bool {
    std::iter::once(statement.body.as_slice())
        .chain(
            statement
                .elif_else_clauses
                .iter()
                .filter(|clause| clause.test.is_some())
                .map(|clause| clause.body.as_slice()),
        )
        .any(|body| {
            let Some(Stmt::If(nested)) = body.last() else {
                return false;
            };
            !nested.elif_else_clauses.is_empty()
                && early_condition_truthiness(&nested.test).is_none()
                && std::iter::once(nested.body.as_slice())
                    .chain(
                        nested
                            .elif_else_clauses
                            .iter()
                            .take(nested.elif_else_clauses.len() - 1)
                            .map(|clause| clause.body.as_slice()),
                    )
                    // Cleanup and internal branches leave different original exit blocks.
                    .any(suite_has_linear_borrow_fallthrough)
        })
}
