use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{BoolOp, CmpOp, Expr, Pattern, Stmt, UnaryOp};
use ruff_text_size::{Ranged, TextRange};

use super::{
    BINARY_SLICE, BUILD_LIST, BUILD_MAP, BUILD_SET, BUILD_TUPLE, CALL, CHECK_EG_MATCH,
    CHECK_EXC_MATCH, CLEANUP_THROW, COPY, END_ASYNC_FOR, END_FOR, END_SEND, FOR_ITER, GET_AITER,
    GET_ANEXT, GET_ITER, IMPORT_FROM, IMPORT_NAME, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT,
    JUMP_FORWARD, LIST_APPEND, LOAD_ATTR, LOAD_CONST, LOAD_DEREF, LOAD_SMALL_INT, MAP_ADD, NOP,
    NOT_TAKEN, POP_EXCEPT, POP_ITER, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE,
    POP_JUMP_IF_TRUE, POP_TOP, PUSH_EXC_INFO, RAISE_VARARGS, RETURN_VALUE, SEND, SET_ADD,
    STORE_ATTR, STORE_SLICE, STORE_SUBSCR, SWAP, TO_BOOL, WITH_EXCEPT_START, YIELD_VALUE,
};
use crate::CompileError;
use crate::assembler::{ExceptionPolicy, Label, SourceLocation};
use crate::code_object::Constant;
use crate::target::code_flags::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_FUTURE_ANNOTATIONS, CO_GENERATOR,
};
use crate::target::operands::{
    BinaryIntrinsic, BinaryOperation, CommonConstant, ResumeLocation, SpecialMethod, UnaryIntrinsic,
};

use super::constant::{
    first_literal_constant, first_suite_literal_constant, fold_constant, is_literal_constant,
    literal_constant, optimized_percent_format, two_element_slice_optimization,
};
use super::expression::{
    binary_operator, comparison_operator, comparison_operator_boolean, early_condition_truthiness,
    expression_defers_async_comprehension_restore, jump_constant_truthiness,
    optimized_generator_callable,
};
use super::scope::{NameId, NameSet, ReferenceCollector, collect_target_names};
use super::source::{
    condition_exit_range, implicit_return_range, synthetic_condition_exit_range,
    unary_not_condition_exit_range, unparse_annotation,
};
use super::symbol::{
    ComprehensionRestore, ReraiseStackState, ResumeDepth, statement_name, to_u32, unsupported,
};
use super::{Compiler, Scope};

#[derive(Clone, Copy, Debug)]
struct LoopContext {
    body_start: usize,
    continue_label: Label,
    break_label: Label,
    iterator_cleanup: IteratorCleanup,
    protect_boolean_return_cleanup: bool,
    prevent_break_exit_inlining: bool,
    with_depth: usize,
    finally_end_depth: usize,
    exception_region_depth: usize,
    break_target_keeps_exception_owner: bool,
    break_policy: LoopBreakPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoopBreakPolicy {
    Return,
    Jump,
    PreserveJumpExit,
}

impl LoopBreakPolicy {
    const fn returns(self) -> bool {
        matches!(self, Self::Return)
    }

    const fn preserves_jump_exit(self) -> bool {
        matches!(self, Self::PreserveJumpExit)
    }
}

#[derive(Clone, Copy, Debug)]
struct ExceptionHandlerContext {
    name: Option<NameId>,
    first: bool,
    loop_depth: usize,
    unwinds_before_sync_with: bool,
    leading_statement: Option<TextRange>,
    trailing_statement: Option<TextRange>,
    ownerless_optimized_boolean_generator_test: Option<TextRange>,
    protect_outer_return_pop: bool,
    common_cleanup: Label,
    common_cleanup_depth: u32,
}

#[derive(Debug)]
struct ReturnFinallyContext {
    body: Vec<Stmt>,
    loop_depth: usize,
    handler: Label,
    depth: u32,
    preserves_conditional_exit_owner: bool,
}

#[derive(Clone, Copy, Debug)]
struct ExceptionTypeBoolMarkerState {
    first_handler: bool,
    initial_block: usize,
}

impl ExceptionTypeBoolMarkerState {
    fn new(first_handler: bool, initial_block: usize) -> Self {
        Self {
            first_handler,
            initial_block,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TryMarkerSlotContext {
    ownerless_prefix_slots: usize,
    protected_start_instruction: usize,
    ownerless_direct_condition: Option<TextRange>,
    handler_starts_with_optimized_boolean_generator: bool,
}

#[derive(Clone, Copy, Debug)]
struct WithExitContext {
    location: SourceLocation,
    is_async: bool,
    exception_region_depth: usize,
    loop_depth: usize,
    preserves_none_condition_owner: bool,
}

#[derive(Clone, Copy, Debug)]
enum WithBodyExitNoop {
    Located(TextRange),
    LocatedConvertedPopBlock(TextRange),
    ConvertedPopBlock,
}

type RegionExclusions = Vec<(Label, Label)>;
type RegionExclusionStack = Vec<RegionExclusions>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IteratorCleanup {
    None,
    Sync,
    Async,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoopBackedge {
    Emitted,
    Required,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LoopTailMode {
    non_final_branch: bool,
    try_body_normal_exit: bool,
    propagate_try_body_normal_exit: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum NestedIfExitLocation<'a> {
    Located(&'a Expr),
    Unlocated,
}

#[derive(Clone, Copy, Debug)]
struct TryCompileOptions {
    terminal: bool,
    emit_statement_nop: bool,
    exclude_terminal_body_not_taken: bool,
    loop_tail: Option<(Label, bool)>,
}

impl TryCompileOptions {
    const STANDARD: Self = Self {
        terminal: false,
        emit_statement_nop: true,
        exclude_terminal_body_not_taken: false,
        loop_tail: None,
    };
    const TERMINAL: Self = Self {
        terminal: true,
        ..Self::STANDARD
    };
    const LEGACY_LOOP_TAIL: Self = Self {
        exclude_terminal_body_not_taken: true,
        ..Self::STANDARD
    };
    const NESTED_HANDLER: Self = Self {
        terminal: false,
        emit_statement_nop: false,
        exclude_terminal_body_not_taken: false,
        loop_tail: None,
    };

    const fn loop_tail(restart: Label, split_short_circuit: bool) -> Self {
        Self {
            loop_tail: Some((restart, split_short_circuit)),
            ..Self::STANDARD
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ControlFlowState {
    pub(super) defer_async_comprehension_restore: bool,
    pub(super) reorder_async_comprehension_cleanup_throw: bool,
    pub(super) pending_comprehension_restores: Vec<(Vec<ComprehensionRestore>, SourceLocation)>,
    pub(super) active_comprehension_cleanups: Vec<(Label, u32)>,
    pub(super) active_comprehension_local_names: Vec<NameSet>,
    pub(super) active_comprehension_region_exclusions: Vec<Vec<(Label, Label)>>,
    active_with_exits: Vec<WithExitContext>,
    pub(super) active_with_suspension_epochs: Vec<usize>,
    pub(super) active_with_region_exclusions: Vec<Vec<(Label, Label)>>,
    active_terminal_withs: usize,
    // CPython retains the empty end block when an async for ends a protected suite.
    emit_protected_async_for_end_nop: bool,
    // CPython retains the normal finally exit when it directly ends a synchronous with body.
    // This is consumed by the selected outer try before compiling any nested try statements.
    preserve_next_finally_exit_nop: bool,
    // The complete final if whose fallthrough retains a converted POP_BLOCK before a terminal
    // try-else suite. The range keeps nested and preceding if statements from consuming it.
    retained_try_body_if_exit: Option<ruff_text_size::TextRange>,
    pub(super) active_exception_region_exclusions: Vec<Vec<(Label, Label)>>,
    // Whether each active ordinary try body has only pass handlers. The innermost entry controls
    // normalized markers whose reused slots depend on that try's handler CFG.
    pub(super) active_pass_only_try_bodies: Vec<bool>,
    // Whether each active ordinary try has a tuple-typed handler whose CFG can leave stale
    // fallthrough exception-owner slots for a top-level Boolean value marker.
    pub(super) active_tuple_handler_try_bodies: Vec<bool>,
    active_try_marker_slots: Vec<Option<TryMarkerSlotContext>>,
    active_exception_handlers: Vec<ExceptionHandlerContext>,
    // Exception-group handlers retain their body marker owner after coroutine suspension.
    pub(super) active_exception_group_handlers: usize,
    active_return_finally_contexts: Vec<ReturnFinallyContext>,
    // A sole copied finally condition can duplicate the preserved return onto its false edge.
    copy_return_finally_condition_exit: bool,
    // CPython's normalized `NOT_TAKEN` can reuse a normal-finally CFG slot that still owns the
    // coroutine stop-iteration handler.
    pub(super) active_normal_finally_bodies: usize,
    pub(super) active_finally_end_blocks: usize,
    // Set while compiling a condition introduced by exceptional-finally lowering.
    pub(super) exceptional_finally_condition: bool,
    active_conditional_cleanup_finally_try_bodies: usize,
    active_finally_try_bodies: usize,
    active_pass_finally_locations: Vec<SourceLocation>,
    active_overriding_finally_returns: usize,
    preserve_finally_break_exit_loop_range: Option<ruff_text_size::TextRange>,
    // The final synchronous-with `if` test whose converted POP_BLOCK exclusion can be
    // routed across a short-circuit condition without affecting nested body expressions.
    pub(super) terminal_with_condition: Option<ruff_text_size::TextRange>,
    pub(super) exclude_terminal_if_not_taken: bool,
    // A leading `not` around a two-operand Boolean condition routes the synchronous-with
    // terminal marker exclusion to the inner condition's final operand.
    pub(super) defer_terminal_with_exclusion_through_negation: bool,
    // The first single-line two-operand condition of a protected iterator-loop continue.
    // CPython keeps its leading marker in the synchronous-with owner.
    pub(super) protected_iterator_continue_condition: Option<ruff_text_size::TextRange>,
    pub(super) exclude_condition_not_taken_from_exception: bool,
    pub(super) include_assert_none_condition_owner: bool,
    pub(super) include_scoped_none_condition_owner: bool,
    // The final chained comparison whose fallthrough jump survives small-exit inlining as a NOP.
    pub(super) retained_chained_comparison_exit: Option<TextRange>,
    pub(super) exclude_condition_not_taken_from_all_exception_regions: bool,
    // CPython normalizes a loop-tail while condition through a fresh backward-jump block. Its
    // `NOT_TAKEN` slot therefore cannot inherit the compacted condition block's stale owner.
    pub(super) loop_tail_while_condition: bool,
    // A final nested while can reach an enclosing loop backedge through ordinary fallthrough.
    // CPython threads that exit before jump normalization, which likewise moves `NOT_TAKEN`
    // into a fresh backward-jump block instead of reusing the condition block's stale slot.
    suite_fallthrough_to_loop_backedge: bool,
    // The final source statement compiled before a split loop-tail `if`, if any.
    synthesized_loop_tail_predecessor: Option<TextRange>,
    exclude_loop_tail_not_taken_from_control_flow_regions: bool,
    exception_type_bool_marker_state: Option<ExceptionTypeBoolMarkerState>,
    implicit_return_exception_exclusion_depth: Option<usize>,
    pub(super) terminal_try_condition_exception_exclusion_depth: Option<usize>,
    unwind_exception_handlers_for_implicit_return: bool,
    prevent_try_exit_inlining: bool,
    pub(super) coroutine_suspension_epoch: usize,
    pub(super) generator_expression: bool,
    pub(super) same_line_try_body: bool,
    // Whether the statement currently entered by `compile_suite` has a sibling successor.
    pub(super) current_statement_has_following: bool,
    direct_comprehension_assignment_results: NameSet,
    // Whether that successor is another try statement. A loop break that targets this sibling
    // remains inside the current protected region instead of exiting through its normal edge.
    current_statement_followed_by_try: bool,
    branch_join_successor: Option<TextRange>,
    // Whether the current statement is nested directly in a terminal if branch suite.
    pub(super) in_terminal_if_branch_suite: bool,
    copied_nested_handler_join_successor: bool,
    // Whether that successor is a direct loop break.
    current_statement_followed_by_break: bool,
    // Whether the current expression is the complete value of an ordinary assignment and is an
    // optimized all/any/tuple generator call.
    pub(super) direct_optimized_generator_assignment: bool,
    // Whether an optimized tuple call is the right operand of a name addition in an inlined
    // comprehension. CPython excludes its dispatch marker from the comprehension's restore range.
    pub(super) exclude_optimized_tuple_guard_from_comprehension: bool,
    // Whether an optimized tuple call is the assignment in the first branch of an `if` whose
    // join is immediately followed by a local return.
    pub(super) copy_optimized_tuple_branch_exit: bool,
    // Whether the current expression is being lowered as one atom of a Boolean condition.
    pub(super) optimized_generator_boolean_condition: bool,
    // Whether that successor is a direct return of one local. CPython copies this tiny exit
    // before removing a constant-loop break edge, so its borrowed-load pass still visits it.
    current_statement_followed_by_name_return: bool,
    // Whether that successor is a direct literal return. CPython can copy this small exit onto
    // the normal edge of a preceding try/else before retaining the edge as a source NOP.
    current_statement_followed_by_literal_return: bool,
    // Whether that successor is a direct folded tuple return. CPython keeps loop breaks targeting
    // this constant exit instead of copying the tuple-return block onto each break edge.
    current_statement_followed_by_tuple_return: bool,
    // Whether that successor exits the function by returning or raising. CPython copies these
    // exit blocks before removing assertion-branch jumps, including jumps from later clauses.
    current_statement_followed_by_return_or_raise: bool,
    // A direct nested try whose normal edge copies a following return across the enclosing
    // protected branch.
    copied_split_return_try: Option<TextRange>,
    // A final try-else `if` whose retained normal-exit marker moves to its first nested join.
    relocated_try_else_exit: Option<TextRange>,
    // The direct final synchronous `with` in an active non-final try body, if any.
    direct_final_sync_with_try_body: Option<ruff_text_size::TextRange>,
    // The direct final synchronous `with` with a normal body fallthrough in a handler-form try
    // nested in an active try/finally protected body.
    direct_final_sync_with_protected_handler_try_body: Option<ruff_text_size::TextRange>,
    // Direct statements in the innermost normal try body. Nested branch suites must not inherit
    // CPython's temporary block ownership for a sibling statement in that body.
    direct_try_body_statements: Vec<TextRange>,
    // A nested try under a normal body with an else suite exits there instead of directly
    // crossing the enclosing handler boundary.
    direct_try_body_has_else: bool,
    // A direct final assignment branch whose copied normal exit crosses to a following statement
    // outside the try.
    direct_try_assignment_branch_exit: Option<TextRange>,
    // Whether code follows the enclosing try statement in its current suite.
    direct_try_body_has_successor: bool,
    // Normal exits from a trailing if chain cross the innermost handler boundary.
    direct_try_if_exit: bool,
    // A one-return try/handler nested directly in a synchronous with unwinds that try before
    // running the surrounding manager exit.
    direct_return_handler_try_inside_sync_with: bool,
    // Conditions nested within a fully terminating synchronous-with return chain retain the
    // manager's normalized exception owner.
    sync_with_direct_return_chain: bool,
    // A direct assignment-only if/else whose assignment or loop-continue handler leaves the
    // successful branch exit outside the try owner.
    ownerless_handler_assignment_branch: Option<TextRange>,
    pub(super) generator_region_start: Option<Label>,
    pub(super) generator_region_exclusions: Vec<(Label, Label)>,
    pub(super) emitted_fallthrough_return: bool,
    loops: Vec<LoopContext>,
    pub(super) depth: i32,
    pub(super) max_depth: u32,
}

impl ControlFlowState {
    pub(super) fn in_first_unnamed_exception_handler(&self) -> bool {
        self.active_exception_handlers
            .last()
            .is_some_and(|handler| handler.first && handler.name.is_none())
    }

    pub(super) fn current_exception_handler_binds_name(&self) -> bool {
        self.active_exception_handlers
            .last()
            .is_some_and(|handler| handler.name.is_some())
    }

    pub(super) fn in_exception_handler(&self) -> bool {
        !self.active_exception_handlers.is_empty()
    }

    pub(super) fn is_ownerless_optimized_boolean_generator_test(&self, range: TextRange) -> bool {
        self.active_exception_handlers
            .last()
            .is_some_and(|handler| {
                handler.ownerless_optimized_boolean_generator_test == Some(range)
            })
    }

    pub(super) fn in_synchronous_with(&self) -> bool {
        self.active_with_exits
            .last()
            .is_some_and(|context| !context.is_async)
    }

    pub(super) fn synchronous_with_preserves_none_condition_owner(&self) -> bool {
        self.active_with_exits
            .last()
            .is_some_and(|context| !context.is_async && context.preserves_none_condition_owner)
    }

    pub(super) fn finally_try_body_preserves_none_condition_owner(&self) -> bool {
        self.active_conditional_cleanup_finally_try_bodies > 0
    }

    pub(super) fn follows_copied_nested_handler_join(&self) -> bool {
        self.copied_nested_handler_join_successor
    }

    pub(super) fn take_direct_comprehension_assignment_result(&mut self, name: NameId) -> bool {
        self.direct_comprehension_assignment_results.remove(name)
    }

    fn is_direct_trailing_handler_statement(&self, range: TextRange) -> bool {
        self.active_exception_handlers
            .last()
            .is_some_and(|handler| handler.trailing_statement == Some(range))
    }

    pub(super) fn in_iterator_loop(&self) -> bool {
        self.loops
            .last()
            .is_some_and(|context| context.iterator_cleanup != IteratorCleanup::None)
    }

    pub(super) fn in_while_loop(&self) -> bool {
        self.loops
            .last()
            .is_some_and(|context| context.iterator_cleanup == IteratorCleanup::None)
    }

    pub(super) fn current_try_marker_is_shifted_to_ownerless_slot(
        &self,
        current_instruction: usize,
        condition: TextRange,
    ) -> bool {
        self.active_try_marker_slots
            .last()
            .copied()
            .flatten()
            .is_some_and(|context| {
                context.ownerless_direct_condition == Some(condition)
                    || context.ownerless_prefix_slots
                        >= current_instruction.saturating_sub(context.protected_start_instruction)
            })
    }

    pub(super) fn current_try_handler_starts_with_optimized_boolean_generator(&self) -> bool {
        self.active_try_marker_slots
            .last()
            .copied()
            .flatten()
            .is_some_and(|context| context.handler_starts_with_optimized_boolean_generator)
    }

    fn in_exception_protected_iterator_loop(&self) -> bool {
        self.loops.last().is_some_and(|context| {
            context.iterator_cleanup != IteratorCleanup::None
                && context.exception_region_depth == self.active_exception_region_exclusions.len()
        })
    }

    fn protects_boolean_return_iterator_cleanup(&self) -> bool {
        self.loops.last().is_some_and(|context| {
            context.protect_boolean_return_cleanup
                && context.exception_region_depth == self.active_exception_region_exclusions.len()
        })
    }

    /// Returns whether a direct Boolean exception-type marker should retain its owner.
    ///
    /// CPython appends normalized markers without initializing the reused CFG instruction slot.
    /// The first marker in a first handler can therefore inherit the common-cleanup owner. The
    /// corresponding slot is fresh for later handlers and after another marker or block boundary.
    pub(super) fn exception_type_bool_marker_preserves_owner(
        &self,
        current_block: usize,
    ) -> Option<bool> {
        self.exception_type_bool_marker_state
            .as_ref()
            .and_then(|state| (state.initial_block == current_block).then_some(state.first_handler))
    }
}

impl Compiler {
    fn loop_context(
        &self,
        continue_label: Label,
        break_label: Label,
        iterator_cleanup: IteratorCleanup,
        break_policy: LoopBreakPolicy,
    ) -> LoopContext {
        LoopContext {
            body_start: self.assembler.instruction_count(),
            continue_label,
            break_label,
            iterator_cleanup,
            protect_boolean_return_cleanup: false,
            prevent_break_exit_inlining: iterator_cleanup != IteratorCleanup::None
                && self.control.current_statement_followed_by_tuple_return,
            with_depth: self.control.active_with_exits.len(),
            finally_end_depth: self.control.active_finally_end_blocks,
            exception_region_depth: self.control.active_exception_region_exclusions.len(),
            break_target_keeps_exception_owner: self.control.current_statement_followed_by_try,
            break_policy,
        }
    }

    fn with_try_body_context<T>(
        &mut self,
        statement: &ruff_python_ast::StmtTry,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let try_line = self.source_location(statement.range).line;
        let same_line = statement
            .body
            .first()
            .is_some_and(|body| self.source_location(body.range()).line == try_line);
        let final_sync_with_try_body =
            statement.body.last().and_then(|statement| match statement {
                Stmt::With(statement) if !statement.is_async => Some(statement.range),
                _ => None,
            });
        let final_sync_with_body_falls_through = statement.body.last().is_some_and(|statement| {
            matches!(
                statement,
                Stmt::With(statement)
                    if !statement.is_async && !suite_terminates(&statement.body)
            )
        });
        let direct_final_sync_with_try_body = (self.control.current_statement_has_following
            && statement.orelse.is_empty())
        .then_some(final_sync_with_try_body)
        .flatten();
        let direct_final_sync_with_protected_handler_try_body = (statement.orelse.is_empty()
            && !statement.handlers.is_empty()
            && self.control.active_finally_try_bodies > 0
            && final_sync_with_body_falls_through
            && self.control.generator_region_start.is_none())
        .then_some(final_sync_with_try_body)
        .flatten();
        let direct_try_body_statements = statement.body.iter().map(Ranged::range).collect();
        let direct_try_body_has_else = !statement.orelse.is_empty();
        let direct_try_body_has_single_raise_handler = matches!(
            statement.handlers.as_slice(),
            [handler]
                if handler.as_except_handler().is_some_and(|handler| {
                    matches!(handler.body.as_slice(), [Stmt::Raise(_)])
                })
        );
        let direct_try_assignment_branch_exit = if self.control.current_statement_has_following
            && statement.orelse.is_empty()
            && statement.finalbody.is_empty()
            && !statement.handlers.is_empty()
        {
            match statement.body.last() {
                Some(Stmt::If(branch))
                    if (is_direct_try_assignment_branch(branch)
                        && (statement.body.len() == 1
                            || (direct_try_body_has_single_raise_handler
                                && is_membership_assignment_branch(branch))))
                        || is_leading_pair_assignment_branch(branch) =>
                {
                    Some(branch.range)
                }
                _ => None,
            }
        } else {
            None
        };
        let direct_try_body_has_successor = self.control.current_statement_has_following;
        self.with_control_override(
            |control| &mut control.same_line_try_body,
            same_line,
            |compiler| {
                compiler.with_control_override(
                    |control| &mut control.direct_try_body_has_else,
                    direct_try_body_has_else,
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.direct_try_assignment_branch_exit,
                            direct_try_assignment_branch_exit,
                            |compiler| {
                                compiler.with_control_override(
                                    |control| &mut control.direct_try_body_has_successor,
                                    direct_try_body_has_successor,
                                    |compiler| {
                                        compiler.with_control_override(
                                            |control| &mut control.direct_try_if_exit,
                                            false,
                                            |compiler| {
                                                compiler.with_control_override(
                                                    |control| {
                                                        &mut control.direct_try_body_statements
                                                    },
                                                    direct_try_body_statements,
                                                    |compiler| {
                                                        compiler.with_control_override(
                                                            |control| {
                                                                &mut control
                                                                    .direct_final_sync_with_protected_handler_try_body
                                                            },
                                                            direct_final_sync_with_protected_handler_try_body,
                                                            |compiler| {
                                                                compiler.with_control_override(
                                                                    |control| {
                                                                        &mut control
                                                                            .direct_final_sync_with_try_body
                                                                    },
                                                                    direct_final_sync_with_try_body,
                                                                    compile,
                                                                )
                                                            },
                                                        )
                                                    },
                                                )
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    },
                )
            },
        )
    }

    fn with_loop_context<T>(
        &mut self,
        context: LoopContext,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let previous_depth = self.control.loops.len();
        self.control.loops.push(context);
        let result = compile(self);
        debug_assert!(
            result.is_err() || self.control.loops.len() == previous_depth + 1,
            "loop contexts should be balanced"
        );
        self.control.loops.truncate(previous_depth);
        result
    }

    fn loop_break_policy(&self, range: ruff_text_size::TextRange) -> LoopBreakPolicy {
        if self.control.preserve_finally_break_exit_loop_range == Some(range) {
            LoopBreakPolicy::PreserveJumpExit
        } else {
            LoopBreakPolicy::Jump
        }
    }

    fn with_exception_handler<T>(
        &mut self,
        context: ExceptionHandlerContext,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<(T, Vec<(Label, Label)>), CompileError> {
        let ((value, _context), exclusions) = self.with_control_stack(
            |control| &mut control.active_exception_region_exclusions,
            Vec::new(),
            |compiler| {
                compiler.with_control_stack(
                    |control| &mut control.active_exception_handlers,
                    context,
                    compile,
                )
            },
        )?;
        Ok((value, exclusions))
    }

    pub(super) fn with_comprehension_context<T>(
        &mut self,
        cleanup: (Label, u32),
        local_names: NameSet,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<(T, Vec<(Label, Label)>), CompileError> {
        let (((value, exclusions), _cleanup), _local_names) = self.with_control_stack(
            |control| &mut control.active_comprehension_local_names,
            local_names,
            |compiler| {
                compiler.with_control_stack(
                    |control| &mut control.active_comprehension_cleanups,
                    cleanup,
                    |compiler| {
                        compiler.with_control_stack(
                            |control| &mut control.active_comprehension_region_exclusions,
                            Vec::new(),
                            compile,
                        )
                    },
                )
            },
        )?;
        Ok((value, exclusions))
    }

    fn with_control_stack<T, R>(
        &mut self,
        select: for<'a> fn(&'a mut ControlFlowState) -> &'a mut Vec<T>,
        value: T,
        compile: impl FnOnce(&mut Self) -> Result<R, CompileError>,
    ) -> Result<(R, T), CompileError> {
        let previous_depth = select(&mut self.control).len();
        select(&mut self.control).push(value);
        let result = compile(self);
        let entries = select(&mut self.control);
        debug_assert!(
            result.is_err() || entries.len() == previous_depth + 1,
            "scoped control-flow stacks should be balanced"
        );
        entries.truncate(previous_depth + 1);
        let value = entries
            .pop()
            .expect("scoped control-flow stack retains its entry");
        result.map(|result| (result, value))
    }

    fn with_control_counter<R>(
        &mut self,
        select: for<'a> fn(&'a mut ControlFlowState) -> &'a mut usize,
        compile: impl FnOnce(&mut Self) -> Result<R, CompileError>,
    ) -> Result<R, CompileError> {
        let previous = *select(&mut self.control);
        *select(&mut self.control) = previous + 1;
        let result = compile(self);
        *select(&mut self.control) = previous;
        result
    }

    fn with_finally_protected_context<T>(
        &mut self,
        pass_location: Option<SourceLocation>,
        overriding_return: bool,
        preserve_break_exit_loop_range: Option<ruff_text_size::TextRange>,
        preserves_none_condition_owner: bool,
        return_context: Option<ReturnFinallyContext>,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<(T, Vec<(Label, Label)>), CompileError> {
        let exclusion_depth = self.control.active_exception_region_exclusions.len();
        let pass_depth = self.control.active_pass_finally_locations.len();
        let return_depth = self.control.active_return_finally_contexts.len();
        let has_pass_location = pass_location.is_some();
        let has_return_context = return_context.is_some();
        self.control
            .active_exception_region_exclusions
            .push(Vec::new());
        if let Some(location) = pass_location {
            self.control.active_pass_finally_locations.push(location);
        }
        if let Some(context) = return_context {
            self.control.active_return_finally_contexts.push(context);
        }
        let previous_overriding_returns = self.control.active_overriding_finally_returns;
        self.control.active_overriding_finally_returns += usize::from(overriding_return);
        let previous_break_exit_loop_range = std::mem::replace(
            &mut self.control.preserve_finally_break_exit_loop_range,
            preserve_break_exit_loop_range,
        );
        let previous_try_bodies = self.control.active_finally_try_bodies;
        self.control.active_finally_try_bodies = previous_try_bodies + 1;
        let previous_conditional_cleanup_bodies =
            self.control.active_conditional_cleanup_finally_try_bodies;
        self.control.active_conditional_cleanup_finally_try_bodies =
            previous_conditional_cleanup_bodies + usize::from(preserves_none_condition_owner);

        let result = compile(self);

        self.control.active_conditional_cleanup_finally_try_bodies =
            previous_conditional_cleanup_bodies;
        self.control.active_finally_try_bodies = previous_try_bodies;
        self.control.preserve_finally_break_exit_loop_range = previous_break_exit_loop_range;
        self.control.active_overriding_finally_returns = previous_overriding_returns;
        debug_assert!(
            result.is_err()
                || self.control.active_return_finally_contexts.len()
                    == return_depth + usize::from(has_return_context),
            "return-finally contexts should be balanced"
        );
        self.control
            .active_return_finally_contexts
            .truncate(return_depth);
        debug_assert!(
            result.is_err()
                || self.control.active_pass_finally_locations.len()
                    == pass_depth + usize::from(has_pass_location),
            "pass-finally locations should be balanced"
        );
        self.control
            .active_pass_finally_locations
            .truncate(pass_depth);
        debug_assert!(
            result.is_err()
                || self.control.active_exception_region_exclusions.len() == exclusion_depth + 1,
            "finally exception exclusions should be balanced"
        );
        self.control
            .active_exception_region_exclusions
            .truncate(exclusion_depth + 1);
        let exclusions = self
            .control
            .active_exception_region_exclusions
            .pop()
            .expect("finally protected region retains its exclusion collector");
        result.map(|value| (value, exclusions))
    }

    fn with_region_exclusion_collector<T>(
        &mut self,
        select: for<'a> fn(&'a mut ControlFlowState) -> &'a mut RegionExclusionStack,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<(T, RegionExclusions), CompileError> {
        let (value, exclusions) = self.with_control_stack(select, Vec::new(), compile)?;
        Ok((value, exclusions))
    }

    fn with_with_context<T>(
        &mut self,
        context: WithExitContext,
        terminal: bool,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let previous_depth = self.control.active_with_exits.len();
        let previous_terminal_withs = self.control.active_terminal_withs;
        self.control.active_with_exits.push(context);
        self.control
            .active_with_suspension_epochs
            .push(self.control.coroutine_suspension_epoch);
        self.control.active_terminal_withs += usize::from(terminal);
        let result = compile(self);
        self.control.active_terminal_withs = previous_terminal_withs;
        debug_assert!(
            result.is_err() || self.control.active_with_exits.len() == previous_depth + 1,
            "with contexts should be balanced"
        );
        debug_assert!(
            result.is_err()
                || self.control.active_with_suspension_epochs.len() == previous_depth + 1,
            "with suspension epochs should be balanced"
        );
        self.control.active_with_exits.truncate(previous_depth);
        self.control
            .active_with_suspension_epochs
            .truncate(previous_depth);
        result
    }

    pub(super) fn with_control_override<T, R>(
        &mut self,
        select: for<'a> fn(&'a mut ControlFlowState) -> &'a mut T,
        value: T,
        compile: impl FnOnce(&mut Self) -> Result<R, CompileError>,
    ) -> Result<R, CompileError> {
        let previous = std::mem::replace(select(&mut self.control), value);
        let result = compile(self);
        *select(&mut self.control) = previous;
        result
    }

    pub(super) fn with_fallthrough_tracking<T>(
        &mut self,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<(T, bool), CompileError> {
        let previous = std::mem::replace(&mut self.control.emitted_fallthrough_return, false);
        let result = compile(self);
        let emitted = std::mem::replace(&mut self.control.emitted_fallthrough_return, previous);
        result.map(|value| (value, emitted))
    }

    pub(super) fn optimized_generator_dispatch_is_in_normal_try(&self) -> bool {
        self.control.active_exception_handlers.is_empty()
            && self.control.active_finally_try_bodies == 0
    }

    pub(super) fn compile_suite(&mut self, body: &[Stmt]) -> Result<(), CompileError> {
        self.compile_suite_inner(body, false)
    }

    pub(super) fn compile_suite_inner(
        &mut self,
        body: &[Stmt],
        final_expression_becomes_return: bool,
    ) -> Result<(), CompileError> {
        for (index, statement) in body.iter().enumerate() {
            let is_final = index + 1 == body.len();
            if is_final && final_expression_becomes_return {
                match statement {
                    Stmt::While(statement)
                        if statement.orelse.is_empty()
                            && !suite_contains_loop_break(&statement.body)
                            && matches!(statement.test.as_ref(), Expr::BoolOp(boolean) if boolean.op == BoolOp::And && boolean.values.len() > 1) =>
                    {
                        self.compile_terminal_while_and(statement)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::While(statement)
                        if statement.orelse.is_empty()
                            && suite_contains_loop_break(&statement.body) =>
                    {
                        self.compile_terminal_while_break(statement)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::For(statement)
                        if !statement.is_async
                            && statement.orelse.is_empty()
                            && matches!(statement.body.as_slice(), [Stmt::Break(_)]) =>
                    {
                        self.compile_terminal_for_break(statement)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::If(statement) => {
                        self.compile_terminal_if(statement)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::Expr(expression) if matches!(expression.value.as_ref(), Expr::If(_)) => {
                        let Expr::If(expression) = expression.value.as_ref() else {
                            unreachable!();
                        };
                        self.compile_terminal_if_expression(expression)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::Expr(expression) if matches!(expression.value.as_ref(), Expr::Tuple(tuple) if matches!(tuple.elts.last(), Some(Expr::If(_)))) =>
                    {
                        let Expr::Tuple(tuple) = expression.value.as_ref() else {
                            unreachable!();
                        };
                        self.compile_terminal_tuple_if_expression(tuple)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::Assign(assignment)
                        if matches!(assignment.value.as_ref(), Expr::If(_))
                            && Self::terminal_assignment_if_expression_requires_specialization(
                                assignment,
                            ) =>
                    {
                        let Expr::If(expression) = assignment.value.as_ref() else {
                            unreachable!();
                        };
                        self.compile_terminal_assignment_if_expression(assignment, expression)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::With(statement) if !statement.is_async => {
                        self.compile_with_statement(statement, true)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::With(statement) => {
                        self.compile_async_with_items(&statement.items, &statement.body, true)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::Try(statement) => {
                        let previous = self.assembler.location();
                        self.assembler
                            .set_location(self.source_location(statement.range));
                        let result = self.compile_try_inner(statement, TryCompileOptions::TERMINAL);
                        self.assembler.set_location(previous);
                        result?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::Match(statement) => {
                        self.compile_match_inner(statement, true)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::Expr(expression)
                        if matches!(expression.value.as_ref(), Expr::BoolOp(_)) =>
                    {
                        let Expr::BoolOp(expression) = expression.value.as_ref() else {
                            unreachable!();
                        };
                        self.compile_terminal_bool_expression(expression)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    Stmt::Expr(expression) if matches!(expression.value.as_ref(), Expr::Compare(compare) if compare.ops.len() > 1) =>
                    {
                        let Expr::Compare(expression) = expression.value.as_ref() else {
                            unreachable!();
                        };
                        self.compile_terminal_compare_expression(expression)?;
                        self.control.emitted_fallthrough_return = true;
                        continue;
                    }
                    _ => {}
                }
            }
            if let Stmt::Pass(statement) = statement
                // CPython retains pass markers in the exceptional copy of a multi-statement
                // finally body, even though ordinary protected suites omit them.
                && (self.control.active_exception_region_exclusions.is_empty()
                    || self.control.active_finally_end_blocks > 0)
                && (self.control.active_with_region_exclusions.is_empty() || body.len() > 1)
            {
                self.assembler
                    .set_location(self.source_location(statement.range));
                let noop_start = self.assembler.label();
                self.assembler.mark(noop_start);
                self.emit(NOP, 0, 0)?;
                let noop_end = self.assembler.label();
                self.assembler.mark(noop_end);
                if is_final {
                    for exclusions in &mut self.control.active_with_region_exclusions {
                        exclusions.push((noop_start, noop_end));
                    }
                }
            } else if let Stmt::Expr(expression) = statement
                && fold_constant(&expression.value).is_some()
            {
                if literal_constant(&expression.value).is_none() {
                    self.record_folded_value(&expression.value)?;
                }
                let range = if let Expr::FString(fstring) = expression.value.as_ref() {
                    Self::fstring_result_range(fstring)
                } else {
                    statement.range()
                };
                self.assembler.set_location(self.source_location(range));
                if index + 1 < body.len()
                    || (!final_expression_becomes_return
                        && !matches!(self.unit.scope, Scope::Class { .. }))
                {
                    let noop_start = self.assembler.label();
                    self.assembler.mark(noop_start);
                    self.emit(NOP, 0, 0)?;
                    let noop_end = self.assembler.label();
                    self.assembler.mark(noop_end);
                    let final_statement = index + 1 == body.len();
                    if final_statement && self.control.generator_region_start.is_some() {
                        self.control
                            .generator_region_exclusions
                            .push((noop_start, noop_end));
                    }
                    if final_statement {
                        for exclusions in &mut self.control.active_with_region_exclusions {
                            exclusions.push((noop_start, noop_end));
                        }
                        for exclusions in &mut self.control.active_exception_region_exclusions {
                            exclusions.push((noop_start, noop_end));
                        }
                    }
                }
            } else {
                // CPython drops this end NOP when async-with is the innermost protected owner;
                // an intervening exception region still retains it.
                let innermost_async_with =
                    self.control
                        .active_with_exits
                        .last()
                        .is_some_and(|context| {
                            context.is_async
                                && self.control.active_exception_region_exclusions.len()
                                    == context.exception_region_depth
                        });
                let emit_protected_async_for_end_nop = is_final
                    && matches!(statement, Stmt::For(statement) if statement.is_async)
                    && !innermost_async_with
                    && (!self.control.active_with_region_exclusions.is_empty()
                        || !self.control.active_exception_region_exclusions.is_empty());
                let transparent_loop_tail = matches!(statement, Stmt::While(statement) if statement.orelse.is_empty())
                    || matches!(statement, Stmt::If(statement)
                            if statement.elif_else_clauses.is_empty()
                                || matches!(statement.elif_else_clauses.as_slice(), [clause]
                                    if clause.test.is_none()))
                    || matches!(statement, Stmt::Expr(expression)
                        if matches!(expression.value.as_ref(), Expr::BoolOp(_)));
                let suite_fallthrough_to_loop_backedge =
                    self.control.suite_fallthrough_to_loop_backedge
                        && is_final
                        && transparent_loop_tail;
                let followed_by_break = matches!(body.get(index + 1), Some(Stmt::Break(_)));
                let followed_by_name_return = matches!(
                    body.get(index + 1),
                    Some(Stmt::Return(statement))
                        if matches!(statement.value.as_deref(), Some(Expr::Name(_)))
                );
                let followed_by_literal_return = matches!(
                    body.get(index + 1),
                    Some(Stmt::Return(statement))
                        if statement.value.as_deref().is_none_or(is_literal_constant)
                );
                let followed_by_tuple_return = matches!(
                    body.get(index + 1),
                    Some(Stmt::Return(statement))
                        if matches!(
                            statement.value.as_deref(),
                            Some(value @ Expr::Tuple(_)) if fold_constant(value).is_some()
                        )
                );
                let followed_by_return_or_raise =
                    matches!(body.get(index + 1), Some(Stmt::Return(_) | Stmt::Raise(_)));
                let followed_by_try = matches!(body.get(index + 1), Some(Stmt::Try(_)));
                let previous_statement = body[..index].last();
                // CPython leaves an empty join block after a sibling `if`; its borrowed-load
                // traversal does not cross that block to reach a folded assertion's failure
                // path, so the assertion message remains owned.
                let constant_false_assert_after_if = matches!(
                    (previous_statement, statement),
                    (Some(Stmt::If(_)), Stmt::Assert(assertion))
                        if early_condition_truthiness(&assertion.test) == Some(false)
                );
                let preceded_by_branch_join = previous_statement.is_some_and(|statement| {
                    matches!(statement, Stmt::If(previous) if if_normal_exit_count(previous) > 1)
                });
                let preceded_by_copied_nested_handler_join = previous_statement.is_some_and(
                    |statement| matches!(statement, Stmt::If(previous) if if_contains_copied_nested_handler_exit(previous)),
                );
                let direct_comprehension_assignment_results = if index == 0 {
                    self.control.direct_comprehension_assignment_results.clone()
                } else if matches!(statement, Stmt::If(_) | Stmt::Try(_))
                    && let Some(Stmt::Assign(assignment)) = body.get(index - 1)
                    && matches!(
                        assignment.value.as_ref(),
                        Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_)
                    )
                {
                    let mut target_names = Vec::new();
                    for target in &assignment.targets {
                        collect_target_names(target, &mut target_names);
                    }
                    self.name_id_set(target_names).into_iter().collect()
                } else {
                    NameSet::default()
                };
                let previous_direct_comprehension_assignment_results = std::mem::replace(
                    &mut self.control.direct_comprehension_assignment_results,
                    direct_comprehension_assignment_results,
                );
                let previous_copied_nested_handler_join_successor = std::mem::replace(
                    &mut self.control.copied_nested_handler_join_successor,
                    preceded_by_copied_nested_handler_join,
                );
                let result = self.with_control_override(
                    |control| &mut control.emit_protected_async_for_end_nop,
                    emit_protected_async_for_end_nop,
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.current_statement_has_following,
                            !is_final,
                            |compiler| {
                                compiler.with_control_override(
                                    |control| &mut control.current_statement_followed_by_break,
                                    followed_by_break,
                                    |compiler| {
                                compiler.with_control_override(
                                    |control| {
                                        &mut control.current_statement_followed_by_name_return
                                    },
                                    followed_by_name_return,
                                    |compiler| {
                                        compiler.with_control_override(
                                            |control| {
                                                &mut control
                                                    .current_statement_followed_by_literal_return
                                            },
                                            followed_by_literal_return,
                                            |compiler| {
                                                compiler.with_control_override(
                                                    |control| {
                                                        &mut control.current_statement_followed_by_tuple_return
                                                    },
                                                    followed_by_tuple_return,
                                                    |compiler| {
                                                compiler.with_control_override(
                                                    |control| {
                                                        &mut control
                                                    .current_statement_followed_by_return_or_raise
                                                    },
                                                    followed_by_return_or_raise,
                                                    |compiler| {
                                                        compiler.with_control_override(
                                                            |control| {
                                                                &mut control
                                                                    .current_statement_followed_by_try
                                                            },
                                                            followed_by_try,
                                                            |compiler| {
                                                                compiler.with_control_override(
                                                                    |control| {
                                                                        &mut control
                                                                            .suite_fallthrough_to_loop_backedge
                                                                    },
                                                                    suite_fallthrough_to_loop_backedge,
                                                                    |compiler| {
                                                                        compiler.with_control_override(
                                                                            |control| {
                                                                                &mut control
                                                                                    .branch_join_successor
                                                                            },
                                                                            preceded_by_branch_join
                                                                                .then_some(statement.range()),
                                                                            |compiler| {
                                                                                compiler.compile_with_strict_owned_loads(
                                                                                    constant_false_assert_after_if,
                                                                                    |compiler| compiler.compile_statement(statement),
                                                                                )
                                                                            },
                                                                        )
                                                                    },
                                                                )
                                                            },
                                                        )
                                                    },
                                                )
                                                    },
                                                )
                                            },
                                        )
                                    },
                                )
                                    },
                                )
                            },
                        )
                    },
                );
                self.control.copied_nested_handler_join_successor =
                    previous_copied_nested_handler_join_successor;
                self.control.direct_comprehension_assignment_results =
                    previous_direct_comprehension_assignment_results;
                result?;
            }
            if statement_terminates(statement) {
                let unreachable = &body[index + 1..];
                // CPython visits unreachable statements before the flow graph removes them.
                // Its constant compaction always retains slot zero, so the first literal in
                // an unreachable tail survives when no reachable constant preceded it.
                if self.output.constants.is_empty()
                    && let Some(constant) = first_suite_literal_constant(unreachable)
                {
                    self.add_constant(constant)?;
                }
                self.pre_register_suite_names(unreachable)?;
                break;
            }
        }
        Ok(())
    }

    fn emit_implicit_return(&mut self) -> Result<(), CompileError> {
        if self.control.unwind_exception_handlers_for_implicit_return {
            // Terminal nested suites return while the enclosing handler is still active.
            return self.emit_deferred_implicit_return();
        }
        let exclusion_start = self.begin_implicit_return_exception_exclusion(false);
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.emit(RETURN_VALUE, 0, -1)?;
        self.finish_implicit_return_exception_exclusion(exclusion_start, false);
        Ok(())
    }

    fn mark_terminal_assignment_implicit_return_boundary(&mut self, body: &[Stmt]) {
        if matches!(body.last(), Some(Stmt::Assign(_))) {
            // CPython emits the function's implicit return after compiling the source suite, so
            // a terminal assignment and that return begin in separate basic blocks. Keep the
            // boundary so a preceding handler exit does not see their combined four instructions
            // as a small exit block and copy both onto the cold edge.
            let implicit_return = self.assembler.label();
            self.assembler.mark(implicit_return);
        }
    }

    pub(super) fn emit_deferred_implicit_return(&mut self) -> Result<(), CompileError> {
        let initialized_locals = self
            .control
            .unwind_exception_handlers_for_implicit_return
            .then(|| self.symbols.initialized_locals.clone());
        let exclude_from_all_exception_regions =
            self.control.unwind_exception_handlers_for_implicit_return;
        let exclusion_start =
            self.begin_implicit_return_exception_exclusion(exclude_from_all_exception_regions);
        if self.control.unwind_exception_handlers_for_implicit_return {
            for index in (0..self.control.active_exception_handlers.len()).rev() {
                let name = self.control.active_exception_handlers[index].name;
                self.emit(POP_EXCEPT, 0, -1)?;
                if let Some(name) = name {
                    self.emit_deferred_constant_before_return(Constant::None)?;
                    self.store_name_id(name)?;
                    self.delete_name_id(name)?;
                }
            }
        }
        self.emit_deferred_constant_before_return(Constant::None)?;
        self.emit(RETURN_VALUE, 0, -1)?;
        self.finish_implicit_return_exception_exclusion(
            exclusion_start,
            exclude_from_all_exception_regions,
        );
        if let Some(initialized_locals) = initialized_locals {
            // The cleanup edge returned, so its deletes do not affect subsequently emitted cold
            // handler blocks.
            self.symbols.initialized_locals = initialized_locals;
        }
        Ok(())
    }

    fn begin_implicit_return_exception_exclusion(
        &mut self,
        exclude_from_all_exception_regions: bool,
    ) -> Option<Label> {
        (!self.control.active_exception_region_exclusions.is_empty()
            && (exclude_from_all_exception_regions
                || self
                    .control
                    .implicit_return_exception_exclusion_depth
                    .is_some()))
        .then(|| {
            let start = self.assembler.label();
            self.assembler.mark(start);
            start
        })
    }

    fn finish_implicit_return_exception_exclusion(
        &mut self,
        exclusion_start: Option<Label>,
        exclude_from_all_exception_regions: bool,
    ) {
        let Some(exclusion_start) = exclusion_start else {
            return;
        };
        let exclusion_end = self.assembler.label();
        self.assembler.mark(exclusion_end);
        if exclude_from_all_exception_regions {
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((exclusion_start, exclusion_end));
            }
        } else if let Some(depth) = self.control.implicit_return_exception_exclusion_depth {
            for exclusions in self
                .control
                .active_exception_region_exclusions
                .iter_mut()
                .take(depth)
            {
                exclusions.push((exclusion_start, exclusion_end));
            }
        }
    }

    fn terminal_try_condition_exception_exclusion_depth(&self) -> Option<usize> {
        self.control
            .implicit_return_exception_exclusion_depth
            .filter(|depth| self.control.generator_region_start.is_none() || *depth > 1)
    }

    fn terminal_try_condition_includes_generator_region(&self, branch_index: usize) -> bool {
        branch_index > 0
            || (self.unit.flags & CO_COROUTINE != 0
                && self.control.generator_region_start.is_some()
                && self.assembler.contains_opcode(YIELD_VALUE))
    }

    fn with_terminal_try_condition_exception_exclusion<R>(
        &mut self,
        include_generator: bool,
        compile: impl FnOnce(&mut Self) -> Result<R, CompileError>,
    ) -> Result<R, CompileError> {
        let depth = if include_generator {
            self.control.implicit_return_exception_exclusion_depth
        } else {
            self.terminal_try_condition_exception_exclusion_depth()
        };
        self.with_control_override(
            |control| &mut control.terminal_try_condition_exception_exclusion_depth,
            depth,
            compile,
        )
    }

    fn emit_terminal_try_condition_not_taken(&mut self) -> Result<(), CompileError> {
        let exclusion = self
            .terminal_try_condition_exception_exclusion_depth()
            .map(|depth| {
                let start = self.assembler.label();
                self.assembler.mark(start);
                (start, depth)
            });
        self.emit(NOT_TAKEN, 0, 0)?;
        if let Some((exclusion_start, depth)) = exclusion {
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            if self.control.generator_region_start.is_some() {
                self.control
                    .generator_region_exclusions
                    .push((exclusion_start, exclusion_end));
            }
            for exclusions in self
                .control
                .active_exception_region_exclusions
                .iter_mut()
                .take(depth)
            {
                exclusions.push((exclusion_start, exclusion_end));
            }
        }
        Ok(())
    }

    fn compile_terminal_while_break(
        &mut self,
        statement: &ruff_python_ast::StmtWhile,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let start = self.assembler.label();
        let condition_false = self.assembler.label();
        self.assembler.mark(start);
        if early_condition_truthiness(&statement.test) == Some(true) {
            if self.output.constants.is_empty()
                && let Some(constant) = fold_constant(&statement.test)
            {
                self.add_constant(constant)?;
            }
            self.assembler
                .set_location(self.source_location(statement.test.range()));
            self.emit(NOP, 0, 0)?;
        } else {
            self.with_terminal_try_condition_exception_exclusion(false, |compiler| {
                compiler.compile_jump_if(&statement.test, false, condition_false)
            })?;
        }
        if self.output.constants.is_empty()
            && let Some(constant) = first_suite_literal_constant(&statement.body)
        {
            self.add_constant(constant)?;
        }
        let context = self.loop_context(
            start,
            condition_false,
            IteratorCleanup::None,
            LoopBreakPolicy::Return,
        );
        let normalize_final_condition = matches!(
            statement.body.last(),
            Some(Stmt::If(tail))
                if tail.elif_else_clauses.is_empty()
                    && !suite_contains_loop_break(&tail.body)
        );
        if normalize_final_condition {
            // CPython normalizes a backward conditional jump by reversing it around an explicit
            // backedge. The terminal-while path still needs that layout for a simple final `if`.
            let body_start = self.assembler.instruction_count();
            let tail_jumps = self.with_loop_context(context, |compiler| {
                compiler.compile_while_tail_suite(&statement.body, start)
            })?;
            if tail_jumps == LoopBackedge::Required && !suite_terminates(&statement.body) {
                self.emit_while_backedge(&statement.body, body_start, start)?;
            }
        } else {
            self.with_loop_context(context, |compiler| {
                compiler.with_control_override(
                    |control| &mut control.suite_fallthrough_to_loop_backedge,
                    true,
                    |compiler| compiler.compile_suite(&statement.body),
                )
            })?;
            if !suite_terminates(&statement.body) {
                if let Some(location) = self.assembler.last_instruction_location() {
                    self.assembler.set_location(location);
                }
                self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
            }
        }

        self.assembler.mark(condition_false);
        self.set_depth(base_depth);
        self.assembler
            .set_location(self.source_location(condition_exit_range(&statement.test)));
        self.emit_implicit_return()?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_terminal_while_and(
        &mut self,
        statement: &ruff_python_ast::StmtWhile,
    ) -> Result<(), CompileError> {
        let Expr::BoolOp(condition) = statement.test.as_ref() else {
            unreachable!("terminal while-and requires a boolean condition");
        };
        let base_depth = self.control.depth;
        let start = self.assembler.label();
        self.assembler.mark(start);
        let exits = condition
            .values
            .iter()
            .map(|value| {
                let exit = self.assembler.label();
                self.compile_jump_if(value, false, exit)?;
                Ok((value, exit))
            })
            .collect::<Result<Vec<_>, CompileError>>()?;

        let context = self.loop_context(
            start,
            exits[0].1,
            IteratorCleanup::None,
            LoopBreakPolicy::Return,
        );
        let body_start = self.assembler.instruction_count();
        self.with_loop_context(context, |compiler| {
            compiler.with_control_override(
                |control| &mut control.suite_fallthrough_to_loop_backedge,
                true,
                |compiler| compiler.compile_suite(&statement.body),
            )
        })?;
        if !suite_terminates(&statement.body) {
            self.emit_while_backedge(&statement.body, body_start, start)?;
        }

        for (value, exit) in exits.into_iter().rev() {
            self.assembler.mark(exit);
            self.set_depth(base_depth);
            self.assembler
                .set_location(self.source_location(condition_exit_range(value)));
            self.emit_implicit_return()?;
        }
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_terminal_for_break(
        &mut self,
        statement: &ruff_python_ast::StmtFor,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let cleanup = self.assembler.label();
        self.compile_expression(&statement.iter)?;
        self.assembler
            .set_location(self.source_location(statement.iter.range()));
        self.emit(GET_ITER, 0, 0)?;
        self.emit_jump_forward(FOR_ITER, cleanup, 1)?;
        self.compile_store_target(&statement.target)?;
        self.assembler
            .set_location(self.source_location(statement.body[0].range()));
        self.emit(POP_TOP, 0, -1)?;
        self.emit_implicit_return()?;

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 2);
        self.assembler
            .set_location(self.source_location(statement.iter.range()));
        self.emit(END_FOR, 0, -1)?;
        self.emit(POP_ITER, 0, -1)?;
        self.emit_implicit_return()?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_terminal_if(
        &mut self,
        statement: &ruff_python_ast::StmtIf,
    ) -> Result<(), CompileError> {
        if statement.elif_else_clauses.is_empty()
            && let Expr::Compare(comparison) = statement.test.as_ref()
            && comparison.ops.len() > 1
        {
            return self.compile_terminal_if_chained_compare(statement, comparison);
        }
        if let Some(truthiness) = early_condition_truthiness(&statement.test)
            && statement.elif_else_clauses.len() <= 1
            && statement
                .elif_else_clauses
                .first()
                .is_none_or(|clause| clause.test.is_none())
        {
            if let Some(constant) = fold_constant(&statement.test)
                && (self.output.constants.is_empty() || matches!(constant, Constant::None))
            {
                self.add_constant(constant)?;
            }
            if !truthiness && statement.elif_else_clauses.is_empty() {
                self.pre_register_suite_names(&statement.body)?;
                self.assembler
                    .set_location(self.source_location(statement.test.range()));
                self.emit_deferred_implicit_return()?;
                return Ok(());
            }
            self.assembler
                .set_location(self.source_location(statement.test.range()));
            self.emit(NOP, 0, 0)?;
            let body = if truthiness {
                &statement.body
            } else if let Some(clause) = statement.elif_else_clauses.first() {
                self.pre_register_suite_names(&statement.body)?;
                &clause.body
            } else {
                self.pre_register_suite_names(&statement.body)?;
                self.emit_deferred_implicit_return()?;
                return Ok(());
            };
            let emitted_fallthrough = if matches!(body.last(), Some(Stmt::If(_))) {
                let ((), emitted_fallthrough) = self.with_fallthrough_tracking(|compiler| {
                    compiler.compile_suite_inner(body, true)
                })?;
                emitted_fallthrough
            } else {
                self.compile_suite(body)?;
                false
            };
            if truthiness {
                for clause in &statement.elif_else_clauses {
                    self.pre_register_suite_names(&clause.body)?;
                }
            }
            if !emitted_fallthrough {
                if let Some(location) = self.assembler.last_instruction_location() {
                    self.assembler.set_location(location);
                }
                self.emit_deferred_implicit_return()?;
            }
            return Ok(());
        }

        let base_depth = self.control.depth;
        let mut branches: Vec<(Option<&Expr>, &[Stmt])> =
            vec![(Some(&statement.test), statement.body.as_slice())];
        branches.extend(
            statement
                .elif_else_clauses
                .iter()
                .map(|clause| (clause.test.as_ref(), clause.body.as_slice())),
        );
        let has_else = branches.last().is_some_and(|(test, _)| test.is_none());

        for (branch_index, (test, body)) in branches.into_iter().enumerate() {
            if let Some(test) = test {
                let next = self.assembler.label();
                let exclude_not_taken = self.control.exclude_terminal_if_not_taken
                    || (!self.control.active_with_region_exclusions.is_empty()
                        && branch_index > 0
                        && suite_terminates(body));
                let exclude_from_generator = self.unit.flags & CO_COROUTINE != 0
                    && self.control.generator_region_start.is_some()
                    && self.assembler.contains_opcode(YIELD_VALUE)
                    && self.control.active_with_region_exclusions.is_empty()
                    && self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_normal_finally_bodies == 0;
                self.with_control_override(
                    |control| &mut control.exclude_terminal_if_not_taken,
                    exclude_not_taken,
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.exclude_condition_not_taken_from_exception,
                            exclude_from_generator,
                            |compiler| {
                                compiler.with_terminal_try_condition_exception_exclusion(
                                    compiler.terminal_try_condition_includes_generator_region(
                                        branch_index,
                                    ),
                                    |compiler| compiler.compile_jump_if(test, false, next),
                                )
                            },
                        )
                    },
                )?;
                self.mark_definitely_evaluated_locals(test);
                let body_start = self.assembler.instruction_count();
                let ((), emitted_fallthrough) = self.with_control_override(
                    |control| &mut control.in_terminal_if_branch_suite,
                    true,
                    |compiler| {
                        compiler.with_fallthrough_tracking(|compiler| {
                            compiler.compile_suite_inner(body, true)
                        })
                    },
                )?;
                if !suite_terminates(body) && !emitted_fallthrough {
                    if let Some(range) = suite_assert_success_fallthrough_range(body) {
                        self.assembler.set_location(self.source_location(range));
                    } else if let Some(range) = suite_unary_not_fallthrough_range(body) {
                        self.assembler.set_location(self.source_location(range));
                    } else if let Some(statement) = body
                        .last()
                        .filter(|statement| statement_uses_implicit_return_location(statement))
                    {
                        self.assembler.set_location(
                            self.source_location(implicit_return_range(self, statement)),
                        );
                    } else if self.assembler.instruction_count() > body_start
                        && let Some(location) = self.assembler.last_instruction_location()
                    {
                        self.assembler.set_location(location);
                    } else if let Some(statement) = body.last() {
                        self.assembler
                            .set_location(self.source_location(statement.range()));
                    } else {
                        self.assembler
                            .set_location(self.source_location(condition_fallthrough_range(test)));
                    }
                    self.mark_terminal_assignment_implicit_return_boundary(body);
                    self.emit_deferred_implicit_return()?;
                }
                self.assembler.mark(next);
                self.set_depth(base_depth);
                self.assembler
                    .set_location(self.source_location(condition_fallthrough_range(test)));
            } else {
                let body_start = self.assembler.instruction_count();
                let ((), emitted_fallthrough) = self.compile_with_strict_owned_loads(
                    suite_is_constant_false_assertion(body),
                    |compiler| {
                        compiler.with_fallthrough_tracking(|compiler| {
                            compiler.compile_suite_inner(body, true)
                        })
                    },
                )?;
                if !suite_terminates(body) && !emitted_fallthrough {
                    if let Some(range) = suite_assert_success_fallthrough_range(body) {
                        self.assembler.set_location(self.source_location(range));
                    } else if let Some(range) = suite_unary_not_fallthrough_range(body) {
                        self.assembler.set_location(self.source_location(range));
                    } else if let Some(statement) = body
                        .last()
                        .filter(|statement| statement_uses_implicit_return_location(statement))
                    {
                        self.assembler.set_location(
                            self.source_location(implicit_return_range(self, statement)),
                        );
                    } else if self.assembler.instruction_count() > body_start
                        && let Some(location) = self.assembler.last_instruction_location()
                    {
                        self.assembler.set_location(location);
                    } else if let Some(statement) = body.last() {
                        self.assembler
                            .set_location(self.source_location(statement.range()));
                    }
                    self.mark_terminal_assignment_implicit_return_boundary(body);
                    self.emit_deferred_implicit_return()?;
                }
            }
        }
        if !has_else {
            self.assembler.set_location(SourceLocation::NONE);
            self.emit_deferred_implicit_return()?;
        }
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_terminal_if_chained_compare(
        &mut self,
        statement: &ruff_python_ast::StmtIf,
        comparison: &ruff_python_ast::ExprCompare,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let cleanup = self.assembler.label();
        let body = self.assembler.label();
        let condition_false = self.assembler.label();
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
            let (opcode, argument) = comparison_operator_boolean(*operator);
            self.emit(opcode, argument, -1)?;
            self.emit_jump_forward(POP_JUMP_IF_FALSE, cleanup, -1)?;
            self.emit(NOT_TAKEN, 0, 0)?;
        }
        self.compile_expression(comparison.comparators.last().unwrap())?;
        self.assembler
            .set_location(self.source_location(comparison.range));
        let (opcode, argument) = comparison_operator_boolean(*comparison.ops.last().unwrap());
        self.emit(opcode, argument, -1)?;
        self.emit_jump_forward(POP_JUMP_IF_FALSE, condition_false, -1)?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_jump_forward(JUMP_FORWARD, body, 0)?;
        self.assembler.prevent_last_jump_inlining();

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 1);
        self.assembler
            .set_location(self.source_location(comparison.range));
        self.emit(POP_TOP, 0, -1)?;
        self.emit_deferred_implicit_return()?;

        self.assembler.mark(body);
        self.set_depth(base_depth);
        self.compile_suite(&statement.body)?;
        if let Some(last) = statement.body.last() {
            self.assembler
                .set_location(self.source_location(last.range()));
        }
        self.emit_deferred_implicit_return()?;

        self.assembler.mark(condition_false);
        self.set_depth(base_depth);
        self.assembler
            .set_location(self.source_location(comparison.range));
        self.emit_deferred_implicit_return()?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_terminating_if_chained_compare_with_else(
        &mut self,
        statement: &ruff_python_ast::StmtIf,
        comparison: &ruff_python_ast::ExprCompare,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let cleanup = self.assembler.label();
        let body = self.assembler.label();
        let otherwise = self.assembler.label();
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
            let (opcode, argument) = comparison_operator_boolean(*operator);
            self.emit(opcode, argument, -1)?;
            self.emit_jump_forward(POP_JUMP_IF_FALSE, cleanup, -1)?;
            self.emit(NOT_TAKEN, 0, 0)?;
        }

        let final_comparator = comparison.comparators.last().unwrap();
        let final_operator = *comparison.ops.last().unwrap();
        let specialized_none_jump = matches!(final_comparator, Expr::NoneLiteral(_))
            && matches!(final_operator, CmpOp::Is | CmpOp::IsNot);
        if specialized_none_jump {
            self.add_constant(Constant::None)?;
        } else {
            self.compile_expression(final_comparator)?;
        }
        self.assembler
            .set_location(self.source_location(comparison.range));
        if specialized_none_jump {
            self.emit_jump_forward(
                if final_operator == CmpOp::Is {
                    POP_JUMP_IF_NOT_NONE
                } else {
                    POP_JUMP_IF_NONE
                },
                otherwise,
                -1,
            )?;
        } else {
            let (opcode, argument) = comparison_operator_boolean(final_operator);
            self.emit(opcode, argument, -1)?;
            self.emit_jump_forward(POP_JUMP_IF_FALSE, otherwise, -1)?;
        }
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit(NOP, 0, 0)?;
        self.assembler.preserve_last_nop();

        self.assembler.mark(body);
        self.set_depth(base_depth);
        self.compile_suite(&statement.body)?;

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 1);
        self.assembler
            .set_location(self.source_location(comparison.range));
        self.emit(POP_TOP, 0, -1)?;

        self.assembler.mark(otherwise);
        self.set_depth(base_depth);
        let [clause] = statement.elif_else_clauses.as_slice() else {
            unreachable!("terminal chained comparison specialization requires one else clause");
        };
        self.compile_suite(&clause.body)?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_terminal_if_expression(
        &mut self,
        expression: &ruff_python_ast::ExprIf,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let otherwise = self.assembler.label();
        self.with_terminal_try_condition_exception_exclusion(false, |compiler| {
            compiler.compile_jump_if(&expression.test, false, otherwise)
        })?;
        self.compile_expression(&expression.body)?;
        self.assembler
            .set_location(self.source_location(expression.body.range()));
        self.emit(POP_TOP, 0, -1)?;
        self.emit_implicit_return()?;

        self.assembler.mark(otherwise);
        self.set_depth(base_depth);
        self.compile_expression(&expression.orelse)?;
        // CPython's conditional-expression join is a jump target. Its redundant-pair
        // optimizer therefore does not remove a constant else value with this POP_TOP.
        let end = self.assembler.label();
        self.assembler.mark(end);
        self.assembler
            .set_location(self.source_location(expression.orelse.range()));
        self.emit(POP_TOP, 0, -1)?;
        self.emit_implicit_return()?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_terminal_tuple_if_expression(
        &mut self,
        tuple: &ruff_python_ast::ExprTuple,
    ) -> Result<(), CompileError> {
        let Some((Expr::If(expression), leading)) = tuple.elts.split_last() else {
            return Err(CompileError::Internal(
                "terminal tuple does not end in a conditional expression".to_string(),
            ));
        };
        let base_depth = self.control.depth;
        for element in leading {
            self.compile_expression(element)?;
        }
        let branch_depth = self.control.depth;
        let otherwise = self.assembler.label();
        self.compile_jump_if(&expression.test, false, otherwise)?;
        self.compile_expression(&expression.body)?;
        self.assembler
            .set_location(self.source_location(tuple.range));
        self.emit_build(BUILD_TUPLE, tuple.elts.len())?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit_implicit_return()?;

        self.assembler.mark(otherwise);
        self.set_depth(branch_depth);
        self.compile_expression(&expression.orelse)?;
        self.assembler
            .set_location(self.source_location(tuple.range));
        self.emit_build(BUILD_TUPLE, tuple.elts.len())?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit_implicit_return()?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn terminal_assignment_if_expression_requires_specialization(
        assignment: &ruff_python_ast::StmtAssign,
    ) -> bool {
        // CPython only copies terminal exit blocks with at most four pre-fusion instructions.
        // Compound targets therefore need the ordinary joined lowering so the assembler can
        // apply that threshold. Keep the direct forms specialized to preserve nested-conditional
        // behavior and the load fusion before a simple attribute store.
        match assignment.targets.as_slice() {
            [Expr::Name(_)] => true,
            [Expr::Attribute(attribute)] => {
                matches!(attribute.value.as_ref(), Expr::Name(_))
            }
            _ => false,
        }
    }

    fn compile_terminal_assignment_if_expression(
        &mut self,
        assignment: &ruff_python_ast::StmtAssign,
        expression: &ruff_python_ast::ExprIf,
    ) -> Result<(), CompileError> {
        self.compile_terminal_assignment_if_expression_inner(assignment, expression, false)
    }

    fn compile_terminal_assignment_if_expression_inner(
        &mut self,
        assignment: &ruff_python_ast::StmtAssign,
        expression: &ruff_python_ast::ExprIf,
        preserve_fallthrough_join_nop: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let otherwise = self.assembler.label();
        self.compile_jump_if(&expression.test, false, otherwise)?;
        self.pre_register_expression_names(&expression.body)?;
        self.pre_register_expression_names(&expression.orelse)?;
        if let Expr::If(body) = expression.body.as_ref() {
            // CPython copies the terminal assignment and return into the nested branches. The
            // nested expression's fallthrough join survives that copy as a source-position NOP.
            self.compile_terminal_assignment_if_expression_inner(assignment, body, true)?;
        } else {
            self.compile_expression(&expression.body)?;
            self.compile_assignment_targets(&assignment.targets)?;
            self.assembler
                .set_location(self.source_location(assignment.targets[0].range()));
            self.emit_deferred_implicit_return()?;
        }

        self.assembler.mark(otherwise);
        self.set_depth(base_depth);
        let direct_else_name = matches!(expression.orelse.as_ref(), Expr::Name(_));
        self.compile_with_strict_owned_loads(direct_else_name, |compiler| {
            compiler.compile_expression(&expression.orelse)
        })?;
        if direct_else_name {
            self.assembler.prevent_last_instruction_fusion();
        }
        if preserve_fallthrough_join_nop {
            self.assembler
                .set_location(self.source_location(expression.orelse.range()));
            self.emit(NOP, 0, 0)?;
        }
        self.compile_assignment_targets(&assignment.targets)?;
        self.assembler
            .set_location(self.source_location(assignment.targets[0].range()));
        self.emit_deferred_implicit_return()?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_assignment_targets(&mut self, targets: &[Expr]) -> Result<(), CompileError> {
        for (index, target) in targets.iter().enumerate() {
            if index + 1 < targets.len() {
                self.emit(COPY, 1, 1)?;
            }
            self.compile_store_target(target)?;
        }
        Ok(())
    }

    fn compile_terminal_bool_expression(
        &mut self,
        expression: &ruff_python_ast::ExprBoolOp,
    ) -> Result<(), CompileError> {
        let Some((last, leading)) = expression.values.split_last() else {
            return Err(CompileError::Internal(
                "boolean expression contains no values".to_string(),
            ));
        };
        self.assembler
            .set_location(self.source_location(expression.range));
        let base_depth = self.control.depth;
        let jump = match expression.op {
            BoolOp::And => POP_JUMP_IF_FALSE,
            BoolOp::Or => POP_JUMP_IF_TRUE,
        };

        let mut short_circuits = Vec::with_capacity(leading.len());
        for value in leading {
            let short_circuit = self.assembler.label();
            let continue_outer = self.assembler.label();
            short_circuits.push(short_circuit);
            if let Expr::BoolOp(inner) = value {
                self.compile_bool_expression_short_to(
                    inner,
                    if inner.op == expression.op {
                        short_circuit
                    } else {
                        continue_outer
                    },
                )?;
            } else {
                self.compile_expression(value)?;
            }
            self.assembler
                .set_location(self.source_location(expression.range));
            let exclusion_start = self.assembler.label();
            self.assembler.mark(exclusion_start);
            self.emit(COPY, 1, 1)?;
            self.emit(TO_BOOL, 0, 0)?;
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            self.emit_jump_forward(jump, short_circuit, -1)?;
            self.emit_terminal_try_condition_not_taken()?;
            self.assembler.mark(continue_outer);
            self.emit(POP_TOP, 0, -1)?;
        }

        if let Expr::BoolOp(last) = last {
            self.compile_terminal_bool_expression(last)?;
        } else {
            self.compile_expression(last)?;
            let result = self.assembler.label();
            self.assembler.preserve_block_boundary(result);
            self.assembler.mark(result);
            self.assembler
                .set_location(self.source_location(last.range()));
            self.emit(POP_TOP, 0, -1)?;
            self.emit_implicit_return()?;
        }

        for short_circuit in short_circuits.into_iter().rev() {
            self.assembler.mark(short_circuit);
            self.set_depth(base_depth + 1);
            self.assembler
                .set_location(self.source_location(expression.range));
            self.emit(POP_TOP, 0, -1)?;
            self.emit_implicit_return()?;
        }
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_bool_expression_short_to(
        &mut self,
        expression: &ruff_python_ast::ExprBoolOp,
        short_circuit: Label,
    ) -> Result<(), CompileError> {
        let Some((last, leading)) = expression.values.split_last() else {
            return Err(CompileError::Internal(
                "boolean expression contains no values".to_string(),
            ));
        };
        self.assembler
            .set_location(self.source_location(expression.range));
        let jump = match expression.op {
            BoolOp::And => POP_JUMP_IF_FALSE,
            BoolOp::Or => POP_JUMP_IF_TRUE,
        };
        for value in leading {
            self.compile_expression(value)?;
            self.emit(COPY, 1, 1)?;
            self.emit(TO_BOOL, 0, 0)?;
            self.emit_jump_forward(jump, short_circuit, -1)?;
            self.emit_terminal_try_condition_not_taken()?;
            self.emit(POP_TOP, 0, -1)?;
        }
        self.compile_expression(last)
    }

    fn compile_terminal_compare_expression(
        &mut self,
        expression: &ruff_python_ast::ExprCompare,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let cleanup = self.assembler.label();
        self.assembler
            .set_location(self.source_location(expression.range));
        self.compile_expression(&expression.left)?;
        for (operator, comparator) in expression
            .ops
            .iter()
            .zip(&expression.comparators)
            .take(expression.ops.len() - 1)
        {
            self.compile_expression(comparator)?;
            self.emit(SWAP, 2, 0)?;
            self.emit(COPY, 2, 1)?;
            let (opcode, argument) = comparison_operator(*operator);
            self.emit(opcode, argument, -1)?;
            self.emit(COPY, 1, 1)?;
            self.emit(TO_BOOL, 0, 0)?;
            self.emit_jump_forward(POP_JUMP_IF_FALSE, cleanup, -1)?;
            self.emit(NOT_TAKEN, 0, 0)?;
            self.emit(POP_TOP, 0, -1)?;
        }
        self.compile_expression(expression.comparators.last().unwrap())?;
        let (opcode, argument) = comparison_operator(*expression.ops.last().unwrap());
        self.emit(opcode, argument, -1)?;
        self.assembler
            .set_location(self.source_location(expression.range));
        self.emit(POP_TOP, 0, -1)?;
        self.emit_implicit_return()?;

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 2);
        self.emit(SWAP, 2, 0)?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit_implicit_return()?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        let previous = self.assembler.location();
        self.assembler
            .set_location(self.source_location(statement.range()));
        let result = self.compile_statement_inner(statement);
        self.assembler.set_location(previous);
        result
    }

    fn compile_statement_inner(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        let starting_depth = self.control.depth;
        match statement {
            Stmt::Assign(assignment) => {
                let unpacked = if let [target] = assignment.targets.as_slice()
                    && let Expr::Tuple(value) = assignment.value.as_ref()
                {
                    let targets = match target {
                        Expr::Tuple(target) => Some(target.elts.as_slice()),
                        Expr::List(target) => Some(target.elts.as_slice()),
                        _ => None,
                    };
                    targets.filter(|targets| {
                        targets.len() == value.elts.len()
                            && targets.len() <= 3
                            && !targets.iter().any(Expr::is_starred_expr)
                            && !value.elts.iter().any(Expr::is_starred_expr)
                    })
                } else {
                    None
                };
                if let Some(targets) = unpacked {
                    let Expr::Tuple(value) = assignment.value.as_ref() else {
                        unreachable!();
                    };
                    for value in &value.elts {
                        self.compile_expression(value)?;
                    }
                    if targets.len() > 1 {
                        self.assembler
                            .set_location(self.source_location(assignment.targets[0].range()));
                        self.emit(SWAP, to_u32(targets.len(), "unpack target count")?, 0)?;
                    }
                    for target in targets {
                        self.compile_store_target(target)?;
                    }
                } else {
                    let defer_restore =
                        expression_defers_async_comprehension_restore(&assignment.value);
                    let direct_optimized_generator_assignment = matches!(
                        assignment.value.as_ref(),
                        Expr::Call(call) if optimized_generator_callable(call).is_some()
                    ) && self
                        .control
                        .direct_try_body_statements
                        .contains(&statement.range());
                    self.with_control_override(
                        |control| &mut control.defer_async_comprehension_restore,
                        defer_restore,
                        |compiler| {
                            compiler.with_control_override(
                                |control| &mut control.direct_optimized_generator_assignment,
                                direct_optimized_generator_assignment,
                                |compiler| {
                                    let newly_owned = if assignment
                                        .targets
                                        .iter()
                                        .any(|target| matches!(target, Expr::Name(_)))
                                        && let Expr::Name(name) = assignment.value.as_ref()
                                    {
                                        let name = compiler.required_name_id(name.id.as_str());
                                        compiler.symbols.owned_load_locals.insert(name)
                                    } else {
                                        false
                                    };
                                    compiler.compile_expression(&assignment.value)?;
                                    if newly_owned
                                        && let Expr::Name(name) = assignment.value.as_ref()
                                    {
                                        let name = compiler.required_name_id(name.id.as_str());
                                        compiler.symbols.owned_load_locals.remove(name);
                                    }
                                    Ok(())
                                },
                            )
                        },
                    )?;
                    for (index, target) in assignment.targets.iter().enumerate() {
                        if index + 1 < assignment.targets.len() {
                            self.emit(COPY, 1, 1)?;
                        }
                        self.compile_store_target(target)?;
                    }
                    if defer_restore {
                        for target in &assignment.targets {
                            let mut names = Vec::new();
                            collect_target_names(target, &mut names);
                            let names = names
                                .into_iter()
                                .map(|name| self.required_name_id(&name))
                                .collect::<Vec<_>>();
                            self.symbols.owned_load_locals.extend(names);
                        }
                    }
                    self.emit_pending_comprehension_restores()?;
                }
            }
            Stmt::AugAssign(assignment) => {
                self.compile_augmented_assignment(assignment)?;
            }
            Stmt::AnnAssign(assignment) => self.compile_annotated_assignment(assignment)?,
            Stmt::Expr(expression) => {
                let discarded_comprehension = match expression.value.as_ref() {
                    Expr::ListComp(comprehension) => {
                        self.compile_comprehension(
                            &comprehension.generators,
                            None,
                            &comprehension.elt,
                            BUILD_LIST,
                            LIST_APPEND,
                            true,
                        )?;
                        true
                    }
                    Expr::SetComp(comprehension) => {
                        self.compile_comprehension(
                            &comprehension.generators,
                            None,
                            &comprehension.elt,
                            BUILD_SET,
                            SET_ADD,
                            true,
                        )?;
                        true
                    }
                    Expr::DictComp(comprehension) => {
                        self.compile_comprehension(
                            &comprehension.generators,
                            comprehension.key.as_deref(),
                            &comprehension.value,
                            BUILD_MAP,
                            MAP_ADD,
                            true,
                        )?;
                        true
                    }
                    _ => {
                        self.compile_expression(&expression.value)?;
                        false
                    }
                };
                if discarded_comprehension {
                    return Ok(());
                }
                self.mark_definitely_evaluated_locals(&expression.value);
                let preserve_no_location = matches!(
                    expression.value.as_ref(),
                    Expr::BoolOp(_) | Expr::If(_)
                ) || matches!(expression.value.as_ref(), Expr::Compare(compare) if compare.ops.len() > 1);
                let optimized_generator_call = matches!(
                    expression.value.as_ref(),
                    Expr::Call(call) if optimized_generator_callable(call).is_some()
                );
                if preserve_no_location || optimized_generator_call {
                    self.assembler.set_location(SourceLocation::NONE);
                } else {
                    let discard_range = match expression.value.as_ref() {
                        Expr::Named(named) => named.target.range(),
                        Expr::Attribute(attribute) => self.attribute_opcode_range(attribute),
                        Expr::Call(call) => self.call_opcode_range(call),
                        Expr::FString(fstring) => Self::fstring_result_range(fstring),
                        expression => expression.range(),
                    };
                    if matches!(expression.value.as_ref(), Expr::BinOp(binary) if optimized_percent_format(binary).is_some())
                        && let Some(location) = self.assembler.last_instruction_location()
                    {
                        self.assembler.set_location(location);
                    } else {
                        self.assembler
                            .set_location(self.source_location(discard_range));
                    }
                }
                let loop_tail_boolean = self.control.suite_fallthrough_to_loop_backedge
                    && !self.control.current_statement_has_following
                    && matches!(expression.value.as_ref(), Expr::BoolOp(_));
                if loop_tail_boolean
                    && let Expr::BoolOp(boolean) = expression.value.as_ref()
                    && let Some(last) = boolean.values.last()
                {
                    self.assembler
                        .set_location(self.source_location(last.range()));
                }
                self.emit(POP_TOP, 0, -1)?;
                if preserve_no_location && !loop_tail_boolean {
                    self.assembler.preserve_last_no_location();
                }
            }
            Stmt::Pass(_) => {}
            Stmt::If(statement) => self.compile_if(statement)?,
            Stmt::While(statement) => self.compile_while(statement)?,
            Stmt::For(statement) if statement.is_async => self.compile_async_for(statement)?,
            Stmt::For(statement) => self.compile_for(statement)?,
            Stmt::Break(_) => {
                // Loop-control cleanup runs after CPython unwinds the active exception block.
                let unreachable_depth = self.control.depth;
                let context = self
                    .control
                    .loops
                    .last()
                    .copied()
                    .ok_or_else(|| CompileError::Internal("break outside loop".to_string()))?;
                let is_first_emitted_body_instruction =
                    self.assembler.instruction_count() == context.body_start;
                self.emit(NOP, 0, 0)?;
                let marker_end = self.assembler.instruction_count();
                let finally_end_unwind_start =
                    self.emit_finally_end_unwind(context.finally_end_depth, false)?;
                let exclusion_start = if self.control.active_exception_region_exclusions.len()
                    == context.exception_region_depth
                {
                    None
                } else if finally_end_unwind_start.is_some() {
                    finally_end_unwind_start
                } else {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    Some(start)
                };
                self.emit_loop_control_exception_unwind(self.control.loops.len())?;
                let with_unwind_starts = self.emit_active_with_unwind(context.with_depth, false)?;
                match context.iterator_cleanup {
                    IteratorCleanup::None => {}
                    IteratorCleanup::Sync => self.emit(POP_TOP, 0, -1)?,
                    IteratorCleanup::Async => self.emit(POP_TOP, 0, -1)?,
                }
                if context.break_policy.returns() {
                    self.emit_implicit_return()?;
                } else {
                    let jump_exclusion_start =
                        (self.control.active_exception_region_exclusions.len()
                            == context.exception_region_depth
                            && context.exception_region_depth > 0
                            && context.finally_end_depth == 0
                            && !context.break_target_keeps_exception_owner
                            && break_jump_needs_exception_region_exclusion(
                                self.control.active_finally_try_bodies,
                                context.iterator_cleanup,
                                self.unit.flags,
                            )
                            && self.control.active_exception_handlers.is_empty())
                        .then(|| {
                            let start = self.assembler.label();
                            self.assembler.mark(start);
                            start
                        });
                    let has_no_emitted_cleanup = self.assembler.instruction_count() == marker_end;
                    self.emit_jump_forward(JUMP_FORWARD, context.break_label, 0)?;
                    self.assembler.mark_last_jump_as_authored_break();
                    if context.prevent_break_exit_inlining {
                        self.assembler.prevent_last_jump_inlining();
                    } else if context.break_policy.preserves_jump_exit() {
                        // CPython inlines small exit blocks before removing a jump to the next
                        // block. Keep this terminal protected-loop break through that pass so
                        // the following finally return is copied onto the edge.
                        self.assembler.defer_last_jump_removal();
                        self.assembler.preserve_last_inlined_jump_nop();
                    } else if has_no_emitted_cleanup {
                        // CPython keeps the break marker when it replaces a jump with a small
                        // inlined exit block. A break that is the first emitted body instruction
                        // also owns the line event after early direct-jump removal; later
                        // same-line body instructions do not. Cleanup edges retain neither NOP.
                        if is_first_emitted_body_instruction {
                            self.assembler.preserve_last_inlined_jump_nop();
                        } else {
                            self.assembler.preserve_last_direct_inlined_jump_nop();
                        }
                    }
                    if let Some(jump_exclusion_start) = jump_exclusion_start {
                        let jump_exclusion_end = self.assembler.label();
                        self.assembler.mark(jump_exclusion_end);
                        for exclusions in &mut self.control.active_exception_region_exclusions {
                            exclusions.push((jump_exclusion_start, jump_exclusion_end));
                        }
                    }
                }
                if exclusion_start.is_some() || !with_unwind_starts.is_empty() {
                    let exclusion_end = self.assembler.label();
                    self.assembler.mark(exclusion_end);
                    if let Some(exclusion_start) = exclusion_start {
                        for exclusions in self
                            .control
                            .active_exception_region_exclusions
                            .iter_mut()
                            .skip(context.exception_region_depth)
                        {
                            exclusions.push((exclusion_start, exclusion_end));
                        }
                    }
                    for (index, unwind_start) in with_unwind_starts {
                        self.control.active_with_region_exclusions[index]
                            .push((unwind_start, exclusion_end));
                    }
                }
                self.set_depth(unreachable_depth);
            }
            Stmt::Continue(_) => {
                // Loop-control cleanup runs after CPython unwinds the active exception block.
                let context =
                    self.control.loops.last().copied().ok_or_else(|| {
                        CompileError::Internal("continue outside loop".to_string())
                    })?;
                self.emit(NOP, 0, 0)?;
                let finally_end_unwind_start =
                    self.emit_finally_end_unwind(context.finally_end_depth, false)?;
                let exclusion_start = if self.control.active_exception_region_exclusions.len()
                    == context.exception_region_depth
                {
                    None
                } else if finally_end_unwind_start.is_some() {
                    finally_end_unwind_start
                } else {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    Some(start)
                };
                self.emit_loop_control_exception_unwind(self.control.loops.len())?;
                let with_unwind_starts = self.emit_active_with_unwind(context.with_depth, false)?;
                self.emit_jump_backward(JUMP_BACKWARD, context.continue_label, 0)?;
                // CPython emits a temporary NOP before loop control, so an incoming jump sees
                // the NOP instead of threading through this backward jump.
                self.assembler.prevent_last_jump_threading_target();
                if exclusion_start.is_some() || !with_unwind_starts.is_empty() {
                    let exclusion_end = self.assembler.label();
                    self.assembler.mark(exclusion_end);
                    if let Some(exclusion_start) = exclusion_start {
                        for exclusions in self
                            .control
                            .active_exception_region_exclusions
                            .iter_mut()
                            .skip(context.exception_region_depth)
                        {
                            exclusions.push((exclusion_start, exclusion_end));
                        }
                    }
                    for (index, unwind_start) in with_unwind_starts {
                        self.control.active_with_region_exclusions[index]
                            .push((unwind_start, exclusion_end));
                    }
                }
                self.set_depth(starting_depth);
            }
            Stmt::Return(statement) => {
                if !matches!(self.unit.scope, Scope::Function { .. }) {
                    return Err(unsupported("return outside a function"));
                }
                let return_is_overridden = self.control.active_overriding_finally_returns > 0;
                let preserve_tos = statement
                    .value
                    .as_deref()
                    .is_some_and(|value| !is_literal_constant(value));
                let unwinds_pass_finally = !self.control.active_pass_finally_locations.is_empty();
                let overriding_unwind_start = if return_is_overridden
                    && !preserve_tos
                    && !self.control.active_exception_region_exclusions.is_empty()
                {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    Some(start)
                } else {
                    None
                };
                let supported_nested_finally_return = matches!(
                    statement.value.as_deref(),
                    Some(Expr::Name(_) | Expr::BooleanLiteral(_) | Expr::NoneLiteral(_))
                );
                let nested_call_finally_return = supported_nested_finally_return
                    && self
                        .control
                        .active_return_finally_contexts
                        .last()
                        .is_some_and(|context| {
                            matches!(context.body.as_slice(), [Stmt::Expr(expression)]
                            if matches!(expression.value.as_ref(), Expr::Call(_)))
                        })
                    && (self.control.active_exception_region_exclusions.len()
                        > self.control.active_return_finally_contexts.len()
                        || !self.control.active_exception_handlers.is_empty());
                let return_finally_unwind_start = (overriding_unwind_start.is_none()
                    && !preserve_tos
                    && nested_call_finally_return
                    && self.control.active_exception_handlers.is_empty())
                .then(|| {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    start
                });

                if !preserve_tos {
                    let range = statement
                        .value
                        .as_deref()
                        .map_or(statement.range, Ranged::range);
                    self.assembler.set_location(self.source_location(range));
                    self.emit(NOP, 0, 0)?;
                } else if let Some(value) = &statement.value {
                    self.compile_expression(value)?;
                    let location = self.source_location(statement.range);
                    // CPython converts a synchronous `with` POP_BLOCK following a multiline
                    // `and` return into a NOP carrying the whole return statement's range.
                    // Mark it as converted so ordinary redundant markers can still disappear.
                    if location.line != location.end_line
                        && matches!(value.as_ref(), Expr::BoolOp(boolean) if boolean.op == BoolOp::And)
                        && self
                            .control
                            .active_with_exits
                            .last()
                            .is_some_and(|context| !context.is_async)
                    {
                        self.assembler.set_location(location);
                        self.emit(NOP, 0, 0)?;
                        self.assembler.mark_last_as_converted_pop_block();
                    }
                }
                let pass_finally_edge_start = unwinds_pass_finally
                    .then(|| {
                        let start = self.assembler.label();
                        self.assembler
                            .mark_before_trailing_converted_pop_block(start)
                            .then_some(start)
                    })
                    .flatten();
                let nested_finally_region_unwind_start = (self.control.active_finally_end_blocks
                    > 0
                    && self.control.active_exception_region_exclusions.len()
                        > self.control.active_finally_end_blocks)
                    .then(|| {
                        let start = self.assembler.label();
                        self.assembler.mark(start);
                        (self.control.active_finally_end_blocks, start)
                    });
                let finally_end_unwind_start = self.emit_finally_end_unwind(0, preserve_tos)?;
                // A result returned from a loop that was already protected by this try keeps the
                // iterator cleanup protected. For a Boolean literal, start the exclusion before
                // loading the returned constant; a call already has its value on the stack.
                let delay_boolean_iterator_exception_unwind_start =
                    matches!(statement.value.as_deref(), Some(Expr::BooleanLiteral(_)))
                        && self.control.protects_boolean_return_iterator_cleanup();
                let delay_iterator_exception_unwind_start = overriding_unwind_start.is_none()
                    && !self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_with_exits.is_empty()
                    && !unwinds_pass_finally
                    && self.control.active_exception_handlers.is_empty()
                    && self.control.active_return_finally_contexts.is_empty()
                    && (delay_boolean_iterator_exception_unwind_start
                        || (matches!(statement.value.as_deref(), Some(Expr::Call(_)))
                            && self.control.in_exception_protected_iterator_loop()));
                let delay_exception_unwind_start = delay_iterator_exception_unwind_start
                    || (overriding_unwind_start.is_none()
                        && !self.control.active_exception_region_exclusions.is_empty()
                        && self.control.active_with_exits.is_empty()
                        && !unwinds_pass_finally
                        && preserve_tos
                        && !self.control.active_exception_handlers.is_empty());
                let mut exception_unwind_start = if overriding_unwind_start.is_some() {
                    overriding_unwind_start
                } else if return_finally_unwind_start.is_some() {
                    return_finally_unwind_start
                } else if finally_end_unwind_start.is_some() {
                    finally_end_unwind_start
                } else if !self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_with_exits.is_empty()
                    && !unwinds_pass_finally
                    && !delay_exception_unwind_start
                {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    Some(start)
                } else {
                    None
                };
                let unwinds_handler_before_sync_with = !self.control.active_with_exits.is_empty()
                    && !self.control.active_exception_handlers.is_empty()
                    && self
                        .control
                        .active_exception_handlers
                        .iter()
                        .all(|handler| handler.unwinds_before_sync_with);
                let handler_count = if !return_is_overridden
                    && (self.control.active_with_exits.is_empty()
                        || unwinds_handler_before_sync_with)
                    && !unwinds_pass_finally
                {
                    self.control.active_exception_handlers.len()
                } else {
                    0
                };
                let mut next_loop = self.control.loops.len();
                for handler_index in (0..handler_count).rev() {
                    let (loop_depth, name, common_cleanup, common_cleanup_depth) = {
                        let handler = &self.control.active_exception_handlers[handler_index];
                        (
                            handler.loop_depth,
                            handler.name,
                            handler.common_cleanup,
                            handler.common_cleanup_depth,
                        )
                    };
                    self.emit_loop_iterator_cleanup(loop_depth, next_loop, preserve_tos)?;
                    if preserve_tos {
                        let common_cleanup_start = (name.is_some()
                            || (handler_count == 1
                                && self.control.active_exception_region_exclusions.len() > 1
                                && self
                                    .control
                                    .direct_try_body_statements
                                    .contains(&statement.range)))
                        .then(|| {
                            let start = self.assembler.label();
                            self.assembler.mark(start);
                            start
                        });
                        self.emit(SWAP, 2, 0)?;
                        if let Some(common_cleanup_start) = common_cleanup_start {
                            // A named-handler return always assigns this swap to the common cleanup.
                            // A direct value return crossing a nested try in a bare handler needs
                            // the same short region so the inner try stops owning the stack before `SWAP`.
                            let common_cleanup_end = self.assembler.label();
                            self.assembler.mark(common_cleanup_end);
                            self.assembler.add_exception_region(
                                common_cleanup_start,
                                common_cleanup_end,
                                common_cleanup,
                                common_cleanup_depth,
                                true,
                            );
                        }
                    }
                    if (delay_exception_unwind_start || unwinds_handler_before_sync_with)
                        && exception_unwind_start.is_none()
                    {
                        let start = self.assembler.label();
                        self.assembler.mark(start);
                        exception_unwind_start = Some(start);
                    }
                    let protects_outer_handler = handler_index > 0
                        && self.control.active_exception_handlers[handler_index]
                            .protect_outer_return_pop;
                    let protected_pop_except = (protects_outer_handler
                        || (!return_is_overridden
                            && !self.control.active_return_finally_contexts.is_empty()))
                    .then(|| {
                        let start = self.assembler.label();
                        self.assembler.mark(start);
                        start
                    });
                    self.emit(POP_EXCEPT, 0, -1)?;
                    if let Some(start) = protected_pop_except {
                        let end = self.assembler.label();
                        self.assembler.mark(end);
                        if let Some(handler) = handler_index
                            .checked_sub(1)
                            .map(|index| &self.control.active_exception_handlers[index])
                        {
                            self.assembler.add_exception_region(
                                start,
                                end,
                                handler.common_cleanup,
                                handler.common_cleanup_depth,
                                true,
                            );
                        } else {
                            for context in &self.control.active_return_finally_contexts {
                                self.assembler.add_exception_region(
                                    start,
                                    end,
                                    context.handler,
                                    context.depth,
                                    false,
                                );
                            }
                        }
                    }
                    if let Some(name) = name {
                        let none = self.add_constant(Constant::None)?;
                        self.emit(LOAD_CONST, none, 1)?;
                        self.store_name_id(name)?;
                        self.delete_name_id(name)?;
                    }
                    next_loop = loop_depth;
                }
                let return_finally_contexts = if !return_is_overridden {
                    std::mem::take(&mut self.control.active_return_finally_contexts)
                } else {
                    Vec::new()
                };
                let unwinds_return_finally = nested_call_finally_return;
                for (context_index, finally_context) in
                    return_finally_contexts.iter().enumerate().rev()
                {
                    let iterator_cleanup_start = self.assembler.label();
                    self.assembler.mark(iterator_cleanup_start);
                    self.emit_loop_iterator_cleanup(
                        finally_context.loop_depth,
                        next_loop,
                        preserve_tos,
                    )?;
                    let iterator_cleanup_end = self.assembler.label();
                    self.assembler.mark(iterator_cleanup_end);
                    if preserve_tos
                        && nested_call_finally_return
                        && self.control.active_exception_handlers.is_empty()
                        && matches!(statement.value.as_deref(), Some(Expr::Name(_)))
                        && finally_context.loop_depth < next_loop
                    {
                        self.assembler.add_exception_region(
                            iterator_cleanup_start,
                            iterator_cleanup_end,
                            finally_context.handler,
                            finally_context.depth,
                            false,
                        );
                    }
                    // A copied inner finally remains protected by the next outer finally. The
                    // outermost copy runs after every protected region has been unwound.
                    let copied_finally_start = if context_index > 0 {
                        let start = self.assembler.label();
                        self.assembler.mark(start);
                        Some(start)
                    } else {
                        None
                    };
                    let previous_copy_return_finally_condition_exit =
                        self.control.copy_return_finally_condition_exit;
                    self.control.copy_return_finally_condition_exit =
                        matches!(statement.value.as_deref(), Some(Expr::Name(_)))
                            && finally_context.preserves_conditional_exit_owner;
                    let initialized_locals = self.symbols.initialized_locals.clone();
                    let result = if context_index > 0 {
                        self.with_region_exclusion_collector(
                            |control| &mut control.active_exception_region_exclusions,
                            |compiler| compiler.compile_suite(&finally_context.body),
                        )
                    } else {
                        self.compile_suite(&finally_context.body)
                            .map(|()| ((), Vec::new()))
                    };
                    self.control.copy_return_finally_condition_exit =
                        previous_copy_return_finally_condition_exit;
                    self.symbols.initialized_locals = initialized_locals;
                    let ((), copied_finally_exclusions) = result?;
                    if let (Some(copied_finally_start), Some(outer_context)) = (
                        copied_finally_start,
                        context_index
                            .checked_sub(1)
                            .and_then(|index| return_finally_contexts.get(index)),
                    ) {
                        let copied_finally_end = self.assembler.label();
                        self.assembler.mark(copied_finally_end);
                        self.add_exception_regions_with_exclusions(
                            copied_finally_start,
                            copied_finally_end,
                            &copied_finally_exclusions,
                            outer_context.handler,
                            outer_context.depth,
                            false,
                        );
                    }
                    if let Some(location) = self.assembler.last_instruction_location() {
                        self.assembler.set_location(location);
                    }
                    next_loop = finally_context.loop_depth;
                }
                if !return_finally_contexts.is_empty() {
                    self.control.active_return_finally_contexts = return_finally_contexts;
                }
                self.emit_loop_iterator_cleanup(0, next_loop, preserve_tos)?;
                if delay_boolean_iterator_exception_unwind_start
                    && delay_iterator_exception_unwind_start
                    && exception_unwind_start.is_none()
                {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    exception_unwind_start = Some(start);
                }
                if self.control.direct_return_handler_try_inside_sync_with
                    && exception_unwind_start.is_none()
                {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    exception_unwind_start = Some(start);
                }
                let with_unwind_starts = self.emit_active_with_unwind(0, preserve_tos)?;
                let finally_unwind_start = if unwinds_pass_finally {
                    let start = if let Some(start) = pass_finally_edge_start {
                        start
                    } else {
                        let start = self.assembler.label();
                        self.assembler.mark(start);
                        start
                    };
                    self.emit_pass_finally_nops()?;
                    self.assembler.set_location(SourceLocation::NONE);
                    Some(start)
                } else {
                    None
                };
                // A literal returned through the direct final `with` in a normal try body is
                // reloaded after manager cleanup. CPython excludes that reload and return from
                // the surrounding try's handler.
                if !preserve_tos
                    && !return_is_overridden
                    && !with_unwind_starts.is_empty()
                    && self.control.terminal_with_condition.is_some()
                    && self.control.direct_final_sync_with_try_body.is_some()
                    && exception_unwind_start.is_none()
                    && finally_unwind_start.is_none()
                    && !self.control.active_exception_region_exclusions.is_empty()
                {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    exception_unwind_start = Some(start);
                }
                if !preserve_tos && !return_is_overridden {
                    if let Some(value) = &statement.value {
                        let return_location = self.source_location(statement.range);
                        let return_spans_lines = return_location.line != return_location.end_line;
                        if unwinds_return_finally
                            || !with_unwind_starts.is_empty()
                            || finally_unwind_start.is_some()
                        {
                            let constant = fold_constant(value).ok_or_else(|| {
                                CompileError::Internal(
                                    "literal return value did not fold to a constant".to_string(),
                                )
                            })?;
                            self.emit_preprocessed_constant(value, constant)?;
                        } else if return_spans_lines {
                            let value_location = self.source_location(value.range());
                            // Parentheses outside the literal's line span preserve the return
                            // statement location on RETURN_VALUE. Otherwise, both the constant
                            // and return use the literal's range.
                            let location = if value_location.line == return_location.line
                                && value_location.end_line == return_location.end_line
                            {
                                value_location
                            } else {
                                return_location
                            };
                            self.assembler.set_location(location);
                            let constant = fold_constant(value).ok_or_else(|| {
                                CompileError::Internal(
                                    "literal return value did not fold to a constant".to_string(),
                                )
                            })?;
                            self.emit_preprocessed_constant(value, constant)?;
                        } else {
                            self.assembler
                                .set_location(self.source_location(value.range()));
                            self.compile_expression(value)?;
                        }
                    } else {
                        let none = self.add_constant(Constant::None)?;
                        self.emit(LOAD_CONST, none, 1)?;
                    }
                }
                let return_exclusion_start = if exception_unwind_start.is_none()
                    && finally_unwind_start.is_none()
                    && !self.control.active_exception_region_exclusions.is_empty()
                {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    Some(start)
                } else {
                    None
                };
                if return_is_overridden {
                    if preserve_tos {
                        self.assembler
                            .set_location(self.source_location(statement.range));
                        self.emit(POP_TOP, 0, -1)?;
                    }
                } else {
                    self.emit(RETURN_VALUE, 0, -1)?;
                }
                if exception_unwind_start.is_some() || return_exclusion_start.is_some() {
                    let return_exclusion_end = self.assembler.label();
                    self.assembler.mark(return_exclusion_end);
                    let exclusion_start = exception_unwind_start
                        .or(return_exclusion_start)
                        .expect("return exclusion has a start");
                    for (index, exclusions) in self
                        .control
                        .active_exception_region_exclusions
                        .iter_mut()
                        .enumerate()
                    {
                        let exclusion_start = nested_finally_region_unwind_start
                            .filter(|(cutoff, _)| index >= *cutoff)
                            .map_or(exclusion_start, |(_, start)| start);
                        exclusions.push((exclusion_start, return_exclusion_end));
                    }
                }
                if !with_unwind_starts.is_empty() || finally_unwind_start.is_some() {
                    let unwind_end = self.assembler.label();
                    self.assembler.mark(unwind_end);
                    for (index, unwind_start) in with_unwind_starts {
                        self.control.active_with_region_exclusions[index]
                            .push((unwind_start, unwind_end));
                    }
                    if let Some(unwind_start) = finally_unwind_start {
                        for exclusions in &mut self.control.active_exception_region_exclusions {
                            exclusions.push((unwind_start, unwind_end));
                        }
                    }
                }
                self.set_depth(starting_depth);
            }
            Stmt::FunctionDef(definition) => self.compile_function_definition(definition)?,
            Stmt::ClassDef(definition) => self.compile_class_definition(definition)?,
            Stmt::Delete(statement) => {
                for target in &statement.targets {
                    self.compile_delete_target(target)?;
                }
            }
            Stmt::Assert(statement) => self.compile_assert(statement)?,
            Stmt::Raise(statement) => self.compile_raise(statement)?,
            Stmt::Import(statement) => self.compile_import(statement)?,
            Stmt::ImportFrom(statement) => self.compile_from_import(statement)?,
            Stmt::Global(_) | Stmt::Nonlocal(_) => {}
            Stmt::Try(statement) => self.compile_try(statement)?,
            Stmt::Match(statement) => self.compile_match(statement)?,
            Stmt::TypeAlias(statement) => self.compile_type_alias(statement)?,
            Stmt::With(statement) if statement.is_async => self.compile_async_with(statement)?,
            Stmt::With(statement) => self.compile_with(statement)?,
            Stmt::IpyEscapeCommand(_) => {
                return Err(unsupported(statement_name(statement)));
            }
        }

        if self.control.depth != starting_depth {
            return Err(CompileError::Internal(format!(
                "statement changed stack depth from {starting_depth} to {}",
                self.control.depth
            )));
        }
        Ok(())
    }

    fn compile_if(&mut self, statement: &ruff_python_ast::StmtIf) -> Result<(), CompileError> {
        if let Expr::Compare(comparison) = statement.test.as_ref()
            && comparison.ops.len() > 1
            && suite_terminates(&statement.body)
            && matches!(statement.elif_else_clauses.as_slice(), [clause]
                if clause.test.is_none() && suite_terminates(&clause.body))
        {
            return self.compile_terminating_if_chained_compare_with_else(statement, comparison);
        }
        if let Some(truthiness) = early_condition_truthiness(&statement.test)
            && statement.elif_else_clauses.len() <= 1
            && statement
                .elif_else_clauses
                .first()
                .is_none_or(|clause| clause.test.is_none())
        {
            if let Some(constant) = fold_constant(&statement.test) {
                self.add_constant(constant)?;
            }
            self.assembler
                .set_location(self.source_location(statement.test.range()));
            self.emit(NOP, 0, 0)?;
            if truthiness {
                let body_start = self.assembler.instruction_count();
                self.compile_suite(&statement.body)?;
                if self.assembler.instruction_count() == body_start
                    && let Some(Stmt::Pass(statement)) = statement.body.last()
                {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                    self.emit(NOP, 0, 0)?;
                }
                for clause in &statement.elif_else_clauses {
                    self.pre_register_suite_names(&clause.body)?;
                }
            } else {
                self.pre_register_suite_names(&statement.body)?;
                self.assembler.disable_load_fast_borrowing();
                if let Some(clause) = statement.elif_else_clauses.first() {
                    let body_start = self.assembler.instruction_count();
                    self.compile_suite(&clause.body)?;
                    if self.assembler.instruction_count() == body_start
                        && let Some(Stmt::Pass(statement)) = clause.body.last()
                    {
                        self.assembler
                            .set_location(self.source_location(statement.range()));
                        self.emit(NOP, 0, 0)?;
                    }
                }
            }
            return Ok(());
        }

        // The normal joins of a final direct `if` leave this try body. Trailing nested `if`
        // statements share that exit after jump threading, unless another statement follows.
        let direct_try_if_exit = (self
            .control
            .direct_try_body_statements
            .last()
            .is_some_and(|direct| *direct == statement.range)
            || (self.control.direct_try_if_exit && !self.control.current_statement_has_following))
            && matches!(
                statement.elif_else_clauses.as_slice(),
                [clause] if clause.test.is_none()
            )
            && !matches!(
                statement.test.as_ref(),
                Expr::Compare(comparison)
                    if comparison.ops.len() == 1
                        && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
                        && matches!(comparison.comparators[0], Expr::NoneLiteral(_))
            )
            && (!self.control.in_iterator_loop() || self.control.direct_try_body_has_successor)
            && !self.control.in_exception_handler()
            && !matches!(statement.body.as_slice(), [Stmt::Pass(_)])
            && !statement
                .elif_else_clauses
                .iter()
                .any(|clause| matches!(clause.body.as_slice(), [Stmt::Pass(_)]))
            && !self.control.direct_try_body_has_else
            && self.control.active_finally_try_bodies == 0;
        let base_depth = self.control.depth;
        let direct_try_body_exit = direct_try_if_exit
            || self.control.direct_try_assignment_branch_exit == Some(statement.range);
        let end = self.assembler.label();
        let mut branches: Vec<(Option<&Expr>, &[Stmt])> =
            vec![(Some(&statement.test), statement.body.as_slice())];
        branches.extend(
            statement
                .elif_else_clauses
                .iter()
                .map(|clause| (clause.test.as_ref(), clause.body.as_slice())),
        );

        let branch_count = branches.len();
        let has_terminating_else = branches
            .last()
            .is_some_and(|(test, body)| test.is_none() && suite_terminates(body));
        // The remaining branches still compile for symbol and constant-table side effects, but
        // their exception handlers do not contribute to CPython's stack size.
        let first_branch_is_always_true = jump_constant_truthiness(&statement.test) == Some(true);
        let mut first_branch_max_depth = None;
        for (branch_index, (test, body)) in branches.into_iter().enumerate() {
            if let Some(test) = test {
                let next = self.assembler.label();
                let negated_method_call_condition = matches!(test, Expr::UnaryOp(unary)
                    if unary.op == UnaryOp::Not
                        && matches!(unary.operand.as_ref(), Expr::Call(call)
                            if matches!(call.func.as_ref(), Expr::Attribute(_))));
                let direct_sync_with_iterator =
                    self.control
                        .active_with_exits
                        .last()
                        .is_some_and(|context| {
                            !context.is_async
                                && context.loop_depth.checked_add(1)
                                    == Some(self.control.loops.len())
                        });
                let preserve_sync_with_continue_owner = branch_index == 0
                    && self.control.in_iterator_loop()
                    && direct_sync_with_iterator
                    && ((negated_method_call_condition
                        && matches!(body, [Stmt::Continue(_)])
                        && self.control.loops.last().is_some_and(|context| {
                            self.assembler.instruction_count() == context.body_start
                        }))
                        || (self.control.current_statement_has_following
                            && matches!(test, Expr::Name(_) | Expr::Call(_))
                            && matches!(body.last(), Some(Stmt::Continue(_)))
                            && self.control.loops.last().is_some_and(|context| {
                                !self
                                    .assembler
                                    .contains_opcode_since(NOT_TAKEN, context.body_start)
                            })));
                let preserve_sync_with_none_owner = branch_index == 0
                    && !suite_terminates(body)
                    && matches!(statement.elif_else_clauses.as_slice(), [clause]
                        if clause.test.is_none() && !suite_terminates(&clause.body))
                    && matches!(test, Expr::Compare(comparison)
                        if comparison.ops.len() == 1
                            && comparison.ops[0] == CmpOp::Is
                            && matches!(comparison.comparators[0], Expr::NoneLiteral(_)))
                    && !self.control.active_with_region_exclusions.is_empty()
                    && self
                        .control
                        .active_with_exits
                        .last()
                        .is_some_and(|context| !context.is_async);

                let preserve_sync_with_terminating_call_owner = branch_index == 0
                    && statement.elif_else_clauses.len() > 1
                    && matches!(test, Expr::Call(_))
                    && suite_terminates(body)
                    && self.control.loops.is_empty()
                    && self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_with_region_exclusions.len() == 1
                    && self.control.active_with_exits.len() == 1
                    && self
                        .control
                        .active_with_exits
                        .last()
                        .is_some_and(|context| !context.is_async);
                let preserve_sync_with_direct_return_chain_owner =
                    self.control.sync_with_direct_return_chain
                        || (!self.control.current_statement_has_following
                            && self.control.loops.is_empty()
                            && self.control.active_exception_region_exclusions.is_empty()
                            && self.control.active_with_region_exclusions.len() == 1
                            && self.control.active_with_exits.len() == 1
                            && self
                                .control
                                .active_with_exits
                                .last()
                                .is_some_and(|context| !context.is_async)
                            && suite_terminates_in_direct_return_handler_chain(&statement.body)
                            && statement
                                .elif_else_clauses
                                .last()
                                .is_some_and(|clause| clause.test.is_none())
                            && statement.elif_else_clauses.iter().all(|clause| {
                                suite_terminates_in_direct_return_handler_chain(&clause.body)
                            })
                            && (suite_contains_direct_return_handler_try(&statement.body)
                                || statement.elif_else_clauses.iter().any(|clause| {
                                    suite_contains_direct_return_handler_try(&clause.body)
                                })));
                let preserve_generator_with_conditional_body_owner = branch_index == 0
                    && self.unit.flags & (CO_GENERATOR | CO_ASYNC_GENERATOR) != 0
                    && self.control.in_synchronous_with()
                    && self.assembler.contains_opcode(YIELD_VALUE)
                    && matches!(
                        body,
                        [Stmt::Assign(assignment), Stmt::Raise(_)]
                            if matches!(assignment.value.as_ref(), Expr::If(conditional)
                                if matches!(conditional.test.as_ref(), Expr::Compare(comparison)
                                    if comparison.ops.len() == 1
                                        && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
                                        && matches!(comparison.comparators[0], Expr::NoneLiteral(_))))
                    );
                let exclude_normal_finally_short_circuit = branch_index == 0
                    && self.control.active_normal_finally_bodies > 0
                    && self.control.in_synchronous_with()
                    && !self.control.current_statement_has_following
                    && matches!(body, [Stmt::Expr(_)])
                    && matches!(test, Expr::BoolOp(boolean)
                        if boolean.op == BoolOp::And
                            && matches!(boolean.values.as_slice(),
                                [Expr::Compare(_), Expr::Name(_)]));
                let exclude_not_taken = self.control.exclude_terminal_if_not_taken
                    || exclude_normal_finally_short_circuit
                    || (!self.control.active_with_region_exclusions.is_empty()
                        && branch_index > 0
                        && suite_terminates(body));
                let preserve_first_unnamed_handler_loop_owner = branch_index == 0
                    && self.control.synthesized_loop_tail_predecessor == Some(statement.range)
                    && self.control.in_iterator_loop()
                    && self.control.in_first_unnamed_exception_handler();
                let protected_iterator_continue_condition = (branch_index == 0
                    && matches!(body, [Stmt::Continue(_)])
                    && self.control.in_iterator_loop()
                    && !self.control.active_with_region_exclusions.is_empty()
                    && self
                        .control
                        .active_with_exits
                        .last()
                        .is_some_and(|context| {
                            !context.is_async && context.location.line == context.location.end_line
                        })
                    && {
                        let location = self.source_location(test.range());
                        location.line == location.end_line
                    }
                    && matches!(test, Expr::BoolOp(boolean) if boolean.values.len() == 2)
                    && self.control.loops.last().is_some_and(|context| {
                        !self
                            .assembler
                            .contains_opcode_since(NOT_TAKEN, context.body_start)
                    }))
                .then(|| test.range());
                let retain_folded_test_in_protected_region = branch_index == 0
                    && early_condition_truthiness(test) == Some(true)
                    && (!self.control.active_with_region_exclusions.is_empty()
                        || !self.control.active_exception_region_exclusions.is_empty()
                        || self.control.generator_region_start.is_some());
                // An earlier or active handler can leave independently mismatched cold blocks
                // whose offsets currently balance CPython's table. Iterator-loop exits already
                // have the stable copied layout needed to retain this otherwise-missing NOP.
                let stable_terminal_exit_layout = self.control.in_iterator_loop()
                    || (self.control.active_exception_region_exclusions.is_empty()
                        && !self.assembler.contains_opcode(PUSH_EXC_INFO));
                let retained_chained_comparison_exit =
                    (matches!(body, [Stmt::Return(_) | Stmt::Raise(_)])
                        && stable_terminal_exit_layout)
                        .then(|| {
                            final_chained_comparison_jump(test).or_else(|| {
                                (self.control.current_statement_has_following
                                    && matches!(test, Expr::Compare(comparison)
                                        if comparison.ops.len() > 1
                                            && !matches!(comparison.comparators.last(), Some(Expr::NoneLiteral(_)))))
                                .then(|| test.range())
                            })
                        })
                        .flatten();
                let include_scoped_none_condition_owner = branch_count == 1
                    && branch_index == 0
                    && matches!(self.unit.scope, Scope::Function { .. })
                    && ((!self.control.current_statement_has_following
                        && self
                            .control
                            .direct_try_body_statements
                            .contains(&statement.range)
                        && self.control.active_pass_only_try_bodies.last() == Some(&true)
                        && suite_terminates(body)
                        && matches!(test, Expr::BoolOp(_)))
                        || (self.control.current_statement_has_following
                            && self.control.in_iterator_loop()
                            && self.control.generator_region_start.is_none()
                            && self.control.active_with_region_exclusions.is_empty()
                            && !self.control.active_exception_region_exclusions.is_empty()
                            && self.control.active_normal_finally_bodies == 0
                            && !self.control.in_exception_handler()
                            && is_simple_none_default_assignment(test, body))
                        || (self.control.current_statement_has_following
                            && !self.control.in_iterator_loop()
                            && self.control.generator_region_start.is_none()
                            && self.control.active_with_region_exclusions.is_empty()
                            && self.control.active_exception_region_exclusions.len() == 1
                            && self.control.active_normal_finally_bodies == 0
                            && !self.control.in_exception_handler()
                            && is_call_none_default_assignment(test, body)));
                let exclude_later_handler_leading_marker = branch_index == 0
                    && !is_specialized_none_condition(test)
                    && is_later_bare_raise_fallback_if(statement)
                    && self
                        .control
                        .active_exception_handlers
                        .last()
                        .is_some_and(|handler| {
                            !handler.first
                                && handler.name.is_none()
                                && handler.leading_statement == Some(statement.range)
                        });
                let previous_forced_borrowed_loads = exclude_normal_finally_short_circuit
                    .then(|| self.assembler.set_forced_borrowed_loads(true));
                let compile_condition = self.with_control_override(
                    |control| &mut control.protected_iterator_continue_condition,
                    protected_iterator_continue_condition,
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.retained_chained_comparison_exit,
                            retained_chained_comparison_exit,
                            |compiler| {
                                compiler.with_control_override(
                                    |control| &mut control.exclude_terminal_if_not_taken,
                                    exclude_not_taken,
                                    |compiler| {
                                        compiler.with_control_override(
                                            |control| &mut control.include_scoped_none_condition_owner,
                                            include_scoped_none_condition_owner,
                                            |compiler| {
                                                let exclude_condition = compiler.control.exclude_condition_not_taken_from_exception
                                                    || exclude_later_handler_leading_marker;
                                                compiler.with_control_override(
                                                    |control| &mut control.exclude_condition_not_taken_from_exception,
                                                    exclude_condition,
                                                    |compiler| {
                                                        compiler.with_control_override(
                                                            |control| &mut control.defer_terminal_with_exclusion_through_negation,
                                                            exclude_normal_finally_short_circuit,
                                                            |compiler| {
                                                        compiler.with_terminal_try_condition_exception_exclusion(
                                                            compiler.terminal_try_condition_includes_generator_region(
                                                                branch_index,
                                                            ),
                                                            |compiler| compiler.compile_jump_if(test, false, next),
                                                        )
                                                            },
                                                        )
                                                    },
                                                )
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    },
                );
                if let Some(previous) = previous_forced_borrowed_loads {
                    self.assembler.set_forced_borrowed_loads(previous);
                }
                compile_condition?;
                if branch_index == 0 && self.control.copy_return_finally_condition_exit {
                    self.assembler.allow_last_condition_split_return_copy();
                }

                if preserve_sync_with_continue_owner {
                    // CPython keeps the protected CFG owner when normalization reuses a removed
                    // loop-control block slot.
                    self.assembler.preserve_last_not_taken_exception_owner();
                }
                if preserve_first_unnamed_handler_loop_owner {
                    // The condition immediately before a synthesized loop-tail branch inside
                    // the first bare handler reuses the handler block's protected slot. An
                    // ordinary non-final condition and the final loop-control condition each
                    // start in fresh blocks.
                    self.assembler.preserve_last_not_taken_exception_owner();
                }
                if preserve_sync_with_none_owner {
                    // A complete non-exiting branch keeps its specialized `is None` marker in
                    // the surrounding synchronous-with region even after an earlier normalized
                    // marker consumes the with setup's stale owner slot.
                    self.assembler.preserve_last_not_taken_exception_owner();
                }
                if preserve_sync_with_terminating_call_owner {
                    // The removed branch edge leaves its manager owner on this marker.
                    self.assembler.preserve_last_not_taken_exception_owner();
                }

                if preserve_sync_with_direct_return_chain_owner {
                    // This one-return branch chain retains the surrounding manager's owner on
                    // each normalized condition marker.
                    self.assembler.preserve_last_not_taken_exception_owner();
                }
                if preserve_generator_with_conditional_body_owner {
                    // A post-yield marker needs the innermost owner even when an overlapping
                    // outer inclusion already covers its final instruction range.
                    self.assembler.override_last_not_taken_exception_owner();
                }
                if retain_folded_test_in_protected_region {
                    // CPython keeps the first folded branch marker inside the surrounding
                    // protected region instead of treating it as an artificial condition NOP.
                    if self.control.generator_region_start.is_some() {
                        self.control.generator_region_exclusions.pop();
                    }
                    for exclusions in &mut self.control.active_with_region_exclusions {
                        exclusions.pop();
                    }
                    for exclusions in &mut self.control.active_exception_region_exclusions {
                        exclusions.pop();
                    }
                }
                let copy_optimized_tuple_branch_exit = branch_index == 0
                    && self.control.current_statement_followed_by_name_return
                    && self.control.generator_region_start.is_none()
                    && self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_with_region_exclusions.is_empty()
                    && self
                        .control
                        .active_comprehension_region_exclusions
                        .is_empty()
                    && matches!(statement.elif_else_clauses.as_slice(), [clause] if clause.test.is_none())
                    && matches!(body, [Stmt::Assign(assignment)]
                        if matches!(assignment.targets.as_slice(), [Expr::Name(_)])
                            && matches!(assignment.value.as_ref(), Expr::Call(call)
                                if optimized_generator_callable(call) == Some("tuple")));
                self.control.direct_comprehension_assignment_results = NameSet::default();
                self.mark_definitely_evaluated_locals(test);
                let body_start = self.assembler.instruction_count();
                let copied_split_return_try =
                    (self.control.current_statement_followed_by_return_or_raise
                        && !self.control.in_exception_handler()
                        && !self.control.active_exception_region_exclusions.is_empty()
                        && self
                            .control
                            .direct_try_body_statements
                            .contains(&statement.range))
                    .then(|| match body.last() {
                        Some(Stmt::Try(nested))
                            if nested.orelse.is_empty()
                                && nested.finalbody.is_empty()
                                && !nested.handlers.is_empty() =>
                        {
                            Some(nested.range)
                        }
                        _ => None,
                    })
                    .flatten();
                self.with_control_override(
                    |control| &mut control.copy_optimized_tuple_branch_exit,
                    copy_optimized_tuple_branch_exit,
                    |compiler| {
                        compiler.compile_with_strict_owned_loads(
                            matches!(test, Expr::If(_)),
                            |compiler| {
                                compiler.with_control_override(
                                    |control| &mut control.sync_with_direct_return_chain,
                                    preserve_sync_with_direct_return_chain_owner,
                                    |compiler| {
                                        compiler.with_control_override(
                                            |control| &mut control.copied_split_return_try,
                                            copied_split_return_try,
                                            |compiler| {
                                                compiler.with_control_override(
                                                    |control| &mut control.direct_try_if_exit,
                                                    direct_try_if_exit,
                                                    |compiler| compiler.compile_suite(body),
                                                )
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    },
                )?;
                let pass_only_body = matches!(body, [Stmt::Pass(_)]);
                if self.assembler.instruction_count() == body_start
                    && let Some(statement) = body.last()
                {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                    self.emit(NOP, 0, 0)?;
                }
                if branch_index == 0 && first_branch_is_always_true {
                    first_branch_max_depth = Some(self.control.max_depth);
                }
                if !suite_terminates(body) && branch_index + 1 < branch_count {
                    let direct_try_body_exit = direct_try_body_exit
                        || self.control.ownerless_handler_assignment_branch
                            == Some(statement.range);
                    let nested_if_exit = body.last().and_then(|statement| match statement {
                        Stmt::If(statement) => nested_if_exit_location(
                            statement,
                            self.control.current_statement_has_following,
                        ),
                        _ => None,
                    });
                    let nested_if_ends_in_break = matches!(
                        body.last(),
                        Some(Stmt::If(statement))
                            if matches!(statement.body.last(), Some(Stmt::Break(_)))
                    );
                    if nested_if_ends_in_break && has_terminating_else {
                        let Some(Stmt::If(statement)) = body.last() else {
                            unreachable!();
                        };
                        self.assembler.set_location(
                            self.source_location(synthetic_condition_exit_range(&statement.test)),
                        );
                    } else if let Some(NestedIfExitLocation::Located(test)) = nested_if_exit {
                        self.assembler
                            .set_location(self.source_location(condition_fallthrough_range(test)));
                    } else if matches!(nested_if_exit, Some(NestedIfExitLocation::Unlocated)) {
                        self.assembler.set_location(SourceLocation::NONE);
                    // When both arms end in plain loops, CPython threads this branch exit from
                    // the first loop's false edge and retains that condition's source location.
                    } else if let Some(Stmt::While(while_statement)) = body.last()
                        && while_statement.orelse.is_empty()
                        && statement.elif_else_clauses.last().is_some_and(|clause| {
                            clause.test.is_none()
                                && matches!(
                                    clause.body.last(),
                                    Some(Stmt::While(else_while)) if else_while.orelse.is_empty()
                                )
                        })
                    {
                        self.assembler.set_location(
                            self.source_location(condition_fallthrough_range(
                                &while_statement.test,
                            )),
                        );
                    } else {
                        self.set_branch_end_location(body, body_start);
                    }
                    let direct_try_body_exit_start = direct_try_body_exit.then(|| {
                        let start = self.assembler.label();
                        self.assembler.mark(start);
                        start
                    });
                    self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
                    if let Some(start) = direct_try_body_exit_start {
                        let exclusion_end = self.assembler.label();
                        self.assembler.mark(exclusion_end);
                        self.control
                            .active_exception_region_exclusions
                            .last_mut()
                            .expect("direct try-body branch exit requires an active try")
                            .push((start, exclusion_end));
                    }
                    // CPython retains the first assertion branch's source jump when it copies a
                    // nonterminal join onto that edge. A directly following return or raise
                    // copies the function-exit block onto later clauses too. Without a copied
                    // function exit, terminating `else` blocks take a different CFG path and
                    // drop the jump instead.
                    let retain_assertion_exit = ((branch_index == 0 && !has_terminating_else)
                        || self.control.current_statement_followed_by_return_or_raise)
                        && matches!(body.last(), Some(Stmt::Assert(_)));
                    if nested_if_ends_in_break && has_terminating_else {
                        self.assembler.preserve_last_inlined_jump_nop();
                    } else if matches!(nested_if_exit, Some(NestedIfExitLocation::Unlocated)) {
                        self.assembler.preserve_last_no_location();
                    } else if pass_only_body && is_none_pass_assignment_branch(statement) {
                        self.assembler
                            .preserve_last_redundant_jump_nop_after_threading();
                    } else if pass_only_body || retain_assertion_exit {
                        self.assembler.preserve_last_direct_inlined_jump_nop();
                    } else if let Some(Stmt::If(nested)) = body.last() {
                        // CPython emits this jump in the nested if's empty join block. When
                        // small-exit inlining replaces it with a NOP, no preceding instruction
                        // in that block can make the NOP redundant.
                        let relocate_try_else_exit = branch_index == 0
                            && self.control.relocated_try_else_exit == Some(statement.range);
                        if relocate_try_else_exit
                            || (branch_index == 0
                                && body.len() == 1
                                && statement.elif_else_clauses.len() >= 2
                                && statement
                                    .elif_else_clauses
                                    .last()
                                    .is_some_and(|clause| clause.test.is_none())
                                && nested.elif_else_clauses.len() >= 2
                                && nested
                                    .elif_else_clauses
                                    .last()
                                    .is_some_and(|clause| clause.test.is_none()))
                        {
                            self.assembler
                                .preserve_last_redundant_jump_nop_after_threading();
                        } else {
                            self.assembler.preserve_last_direct_inlined_jump_nop();
                        }
                    }
                }
                if self.control.active_finally_end_blocks > 0
                    && branch_count == 1
                    && branch_index == 0
                    && statement.elif_else_clauses.is_empty()
                    && !self.control.current_statement_has_following
                    && !suite_terminates(body)
                {
                    if !matches!(body.last(), Some(Stmt::If(_))) {
                        self.set_branch_end_location(body, body_start);
                        self.emit_reraise(ReraiseStackState::ActiveExceptionHandler)?;
                    }
                    self.assembler.mark(next);
                    self.set_depth(base_depth);
                    self.assembler
                        .set_location(self.source_location(synthetic_condition_exit_range(test)));
                    self.emit_reraise(ReraiseStackState::ActiveExceptionHandler)?;
                    continue;
                }
                self.assembler.mark(next);
                self.set_depth(base_depth);
            } else {
                let body_start = self.assembler.instruction_count();
                // CPython emits a constant-false assertion in a final `else` suite in a
                // separate failure block that its borrowed-load traversal never visits.
                self.compile_with_strict_owned_loads(
                    suite_is_constant_false_assertion(body),
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.direct_try_if_exit,
                            direct_try_if_exit,
                            |compiler| compiler.compile_suite(body),
                        )
                    },
                )?;
                if branch_index + 1 == branch_count
                    && self.assembler.instruction_count() == body_start
                    && let Some(statement) = body.last()
                {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                    self.emit(NOP, 0, 0)?;
                }
            }
        }

        if self.control.retained_try_body_if_exit == Some(statement.range) {
            let exit_start = self.assembler.label();
            self.assembler.mark(exit_start);
            if let Some(location) = self.assembler.last_instruction_location() {
                self.assembler.set_location(location);
            }
            self.emit(NOP, 0, 0)?;
            self.assembler.mark_last_as_converted_pop_block();
            self.assembler.preserve_last_nop();
            let exit_end = self.assembler.label();
            self.assembler.mark(exit_end);
            self.control
                .active_exception_region_exclusions
                .last_mut()
                .expect("retained try-body if exit requires an active try")
                .push((exit_start, exit_end));
        }
        self.assembler.mark(end);
        self.set_depth(base_depth);
        if let Some(max_depth) = first_branch_max_depth {
            self.control.max_depth = max_depth;
        }
        Ok(())
    }

    pub(super) fn set_branch_end_location(&mut self, body: &[Stmt], body_start: usize) {
        if let Some(Stmt::If(statement)) = body.last()
            && let Some(test) = terminating_if_fallthrough_test(statement)
        {
            self.assembler
                .set_location(self.source_location(condition_fallthrough_range(test)));
        } else if let Some(range) = suite_assert_success_fallthrough_range(body) {
            self.assembler.set_location(self.source_location(range));
        } else if let Some(range) = suite_unary_not_fallthrough_range(body) {
            self.assembler.set_location(self.source_location(range));
        } else if self.assembler.instruction_count() > body_start
            && let Some(location) = self.assembler.last_instruction_location()
        {
            self.assembler.set_location(location);
        } else if let Some(statement) = body.last() {
            self.assembler
                .set_location(self.source_location(statement.range()));
        }
    }

    pub(super) fn compile_with_strict_owned_loads<T>(
        &mut self,
        strict: bool,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let previous = strict.then(|| self.assembler.set_strict_owned_loads(true));
        let result = compile(self);
        if let Some(previous) = previous {
            self.assembler.set_strict_owned_loads(previous);
        }
        result
    }

    fn compile_loop_tail_suite(
        &mut self,
        body: &[Stmt],
        restart: Label,
        split_short_circuit: bool,
    ) -> Result<LoopBackedge, CompileError> {
        self.compile_loop_tail_suite_inner(
            body,
            restart,
            split_short_circuit,
            LoopTailMode::default(),
        )
    }

    fn compile_non_final_branch_loop_tail_suite(
        &mut self,
        body: &[Stmt],
        restart: Label,
        try_body_normal_exit: bool,
    ) -> Result<LoopBackedge, CompileError> {
        self.compile_loop_tail_suite_inner(
            body,
            restart,
            false,
            LoopTailMode {
                non_final_branch: true,
                try_body_normal_exit,
                propagate_try_body_normal_exit: try_body_normal_exit,
            },
        )
    }

    fn compile_loop_tail_suite_inner(
        &mut self,
        body: &[Stmt],
        restart: Label,
        split_short_circuit: bool,
        mode: LoopTailMode,
    ) -> Result<LoopBackedge, CompileError> {
        match body.split_last() {
            Some((Stmt::If(statement), leading)) => {
                self.with_control_override(
                    |control| &mut control.synthesized_loop_tail_predecessor,
                    leading.last().map(Ranged::range),
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.suite_fallthrough_to_loop_backedge,
                            false,
                            |compiler| compiler.compile_suite(leading),
                        )
                    },
                )?;
                self.compile_loop_tail_if(statement, restart, split_short_circuit, mode, leading)?;
                Ok(LoopBackedge::Emitted)
            }
            Some((Stmt::While(statement), leading))
                if statement.orelse.is_empty()
                    && early_condition_truthiness(&statement.test).is_none()
                    && !suite_contains_loop_break(&statement.body)
                    && !matches!(
                        statement.body.last(),
                        Some(Stmt::Try(statement)) if try_requires_loop_tail_inlining(statement)
                    ) =>
            {
                self.with_control_override(
                    |control| &mut control.suite_fallthrough_to_loop_backedge,
                    false,
                    |compiler| compiler.compile_suite(leading),
                )?;
                self.compile_loop_tail_while(statement, restart, mode.non_final_branch)?;
                Ok(LoopBackedge::Emitted)
            }
            Some((Stmt::Assert(statement), leading))
                if early_condition_truthiness(&statement.test).is_none() =>
            {
                self.with_control_override(
                    |control| &mut control.suite_fallthrough_to_loop_backedge,
                    false,
                    |compiler| compiler.compile_suite(leading),
                )?;
                self.compile_loop_tail_assert(statement, restart, mode.non_final_branch)?;
                Ok(LoopBackedge::Emitted)
            }
            Some((Stmt::Try(statement), leading))
                if !mode.non_final_branch
                    && self.control.generator_region_start.is_none()
                    && try_requires_loop_tail_inlining(statement) =>
            {
                self.with_control_override(
                    |control| &mut control.suite_fallthrough_to_loop_backedge,
                    false,
                    |compiler| compiler.compile_suite(leading),
                )?;
                self.compile_try_inner(
                    statement,
                    TryCompileOptions::loop_tail(restart, split_short_circuit),
                )?;
                Ok(LoopBackedge::Required)
            }
            _ => {
                self.with_control_override(
                    |control| &mut control.suite_fallthrough_to_loop_backedge,
                    true,
                    |compiler| compiler.compile_suite(body),
                )?;
                Ok(LoopBackedge::Required)
            }
        }
    }

    fn add_control_flow_region_exclusion(&mut self, start: Label, end: Label) {
        if self.control.generator_region_start.is_some() {
            self.control.generator_region_exclusions.push((start, end));
        }
        for exclusions in &mut self.control.active_with_region_exclusions {
            exclusions.push((start, end));
        }
        for exclusions in &mut self.control.active_exception_region_exclusions {
            exclusions.push((start, end));
        }
    }

    fn has_active_control_flow_region(&self) -> bool {
        self.control.generator_region_start.is_some()
            || !self.control.active_with_region_exclusions.is_empty()
            || !self.control.active_exception_region_exclusions.is_empty()
    }

    fn emit_active_with_unwind(
        &mut self,
        from_depth: usize,
        preserve_tos: bool,
    ) -> Result<Vec<(usize, Label)>, CompileError> {
        let context_count = self.control.active_with_exits.len();
        let mut unwind_starts = Vec::with_capacity(context_count - from_depth);
        for index in (from_depth..context_count).rev() {
            let context = self.control.active_with_exits[index];
            let start = self.assembler.label();
            self.assembler.mark(start);
            unwind_starts.push((index, start));
            self.assembler.set_location(context.location);
            if preserve_tos {
                self.emit(SWAP, 3, 0)?;
                self.emit(SWAP, 2, 0)?;
            }
            let none = self.add_constant(Constant::None)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(CALL, 3, -4)?;
            if context.is_async {
                self.compile_awaitable_on_stack(2)?;
            }
            self.emit(POP_TOP, 0, -1)?;
        }
        Ok(unwind_starts)
    }

    fn emit_loop_iterator_cleanup(
        &mut self,
        from_depth: usize,
        to_depth: usize,
        preserve_tos: bool,
    ) -> Result<(), CompileError> {
        for index in (from_depth..to_depth).rev() {
            match self.control.loops[index].iterator_cleanup {
                IteratorCleanup::None => {}
                IteratorCleanup::Sync | IteratorCleanup::Async => {
                    if preserve_tos {
                        self.emit(SWAP, 2, 0)?;
                    }
                    self.emit(POP_TOP, 0, -1)?;
                }
            }
        }
        Ok(())
    }

    fn emit_pass_finally_nops(&mut self) -> Result<(), CompileError> {
        for index in (0..self.control.active_pass_finally_locations.len()).rev() {
            let location = self.control.active_pass_finally_locations[index];
            self.assembler.set_location(location);
            self.emit(NOP, 0, 0)?;
        }
        Ok(())
    }

    fn emit_loop_control_exception_unwind(
        &mut self,
        loop_depth: usize,
    ) -> Result<(), CompileError> {
        for index in (0..self.control.active_exception_handlers.len()).rev() {
            let (handler_loop_depth, name) = {
                let handler = &self.control.active_exception_handlers[index];
                (handler.loop_depth, handler.name)
            };
            if handler_loop_depth < loop_depth {
                break;
            }
            self.emit(POP_EXCEPT, 0, -1)?;
            if let Some(name) = name {
                let none = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, none, 1)?;
                self.store_name_id(name)?;
                self.delete_name_id(name)?;
            }
        }
        Ok(())
    }

    fn emit_finally_end_unwind(
        &mut self,
        from_depth: usize,
        preserve_tos: bool,
    ) -> Result<Option<Label>, CompileError> {
        let mut unwind_start = None;
        for _ in from_depth..self.control.active_finally_end_blocks {
            if preserve_tos {
                self.emit(SWAP, 2, 0)?;
            }
            self.emit(POP_TOP, 0, -1)?;
            if preserve_tos {
                self.emit(SWAP, 2, 0)?;
            }
            if unwind_start.is_none() {
                let start = self.assembler.label();
                self.assembler.mark(start);
                unwind_start = Some(start);
            }
            self.emit(POP_EXCEPT, 0, -1)?;
        }
        Ok(unwind_start)
    }

    fn compile_loop_tail_while(
        &mut self,
        statement: &ruff_python_ast::StmtWhile,
        restart: Label,
        non_final_branch: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        if non_final_branch && !self.has_active_control_flow_region() {
            // CPython leaves an unprotected inner loop's body as the fallthrough block and
            // targets the enclosing backedge when its condition is false.
            let start = self.assembler.label();
            let exit = self.assembler.label();
            self.assembler.mark(start);
            self.with_control_override(
                |control| &mut control.loop_tail_while_condition,
                true,
                |compiler| compiler.compile_jump_if(&statement.test, false, exit),
            )?;

            let context =
                self.loop_context(start, restart, IteratorCleanup::None, LoopBreakPolicy::Jump);
            let body_start = self.assembler.instruction_count();
            let tail_jumps = self.with_loop_context(context, |compiler| {
                compiler.compile_while_tail_suite(&statement.body, start)
            })?;
            if tail_jumps == LoopBackedge::Required && !suite_terminates(&statement.body) {
                self.emit_while_backedge(&statement.body, body_start, start)?;
            }

            self.assembler.mark(exit);
            self.set_depth(base_depth);
            self.assembler
                .set_location(self.source_location(condition_fallthrough_range(&statement.test)));
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            return Ok(());
        }

        let start = self.assembler.label();
        let body_label = self.assembler.label();
        let control_region = self
            .has_active_control_flow_region()
            .then(|| (self.assembler.label(), self.assembler.label()));

        self.assembler.mark(start);
        self.with_control_override(
            |control| &mut control.loop_tail_while_condition,
            true,
            |compiler| compiler.compile_jump_if(&statement.test, true, body_label),
        )?;
        if let Some((control_start, _)) = control_region {
            self.assembler
                .mark_before_trailing_instructions(control_start, 2);
        }
        self.assembler
            .set_location(self.source_location(condition_fallthrough_range(&statement.test)));
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        if let Some((control_start, control_end)) = control_region {
            self.assembler.mark(control_end);
            self.add_control_flow_region_exclusion(control_start, control_end);
        }

        self.assembler.mark(body_label);
        self.set_depth(base_depth);
        let context =
            self.loop_context(start, restart, IteratorCleanup::None, LoopBreakPolicy::Jump);
        let body_start = self.assembler.instruction_count();
        let tail_jumps = self.with_loop_context(context, |compiler| {
            compiler.with_control_override(
                |control| &mut control.exclude_loop_tail_not_taken_from_control_flow_regions,
                control_region.is_some(),
                |compiler| compiler.compile_while_tail_suite(&statement.body, start),
            )
        })?;
        if tail_jumps == LoopBackedge::Required && !suite_terminates(&statement.body) {
            self.emit_while_backedge(&statement.body, body_start, start)?;
        }
        Ok(())
    }

    fn compile_while_tail_suite(
        &mut self,
        body: &[Stmt],
        restart: Label,
    ) -> Result<LoopBackedge, CompileError> {
        if self.control.generator_region_start.is_some()
            && let Some((Stmt::Try(statement), leading)) = body.split_last()
            && try_requires_legacy_loop_tail_exclusion(statement)
        {
            self.with_control_override(
                |control| &mut control.suite_fallthrough_to_loop_backedge,
                false,
                |compiler| compiler.compile_suite(leading),
            )?;
            self.compile_try_inner(statement, TryCompileOptions::LEGACY_LOOP_TAIL)?;
            Ok(LoopBackedge::Required)
        } else {
            self.compile_loop_tail_suite(body, restart, true)
        }
    }

    fn emit_while_backedge(
        &mut self,
        body: &[Stmt],
        body_start: usize,
        restart: Label,
    ) -> Result<(), CompileError> {
        let inline_try_tail = body.last().is_some_and(|statement| match statement {
            Stmt::Try(statement) if self.control.generator_region_start.is_some() => {
                try_requires_legacy_loop_tail_exclusion(statement)
            }
            Stmt::Try(statement) => {
                try_requires_loop_tail_inlining(statement)
                    || try_normal_exit_ends_in_break_only_loop(statement)
            }
            _ => false,
        });
        if !inline_try_tail {
            self.set_branch_end_location(body, body_start);
        }
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        if inline_try_tail {
            self.assembler.prepare_last_no_location_block_for_inlining();
        }
        if body
            .last()
            .is_some_and(try_has_conditional_shared_loop_exit)
        {
            // CPython copies this shared backedge onto each conditional fallthrough.
            self.assembler.mark_last_as_artificial_eval_break();
        }
        Ok(())
    }

    fn compile_loop_tail_assert(
        &mut self,
        statement: &ruff_python_ast::StmtAssert,
        restart: Label,
        non_final_branch: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        if non_final_branch
            && !self.has_active_control_flow_region()
            && condition_jump_target_count(&statement.test, false) == 1
            && condition_jump_target_count(&statement.test, true) == 1
        {
            // In a non-final branch, successful assertion evaluation exits the enclosing branch.
            // CPython jumps over the fallthrough raise block to the loop backedge.
            let success = self.assembler.label();
            self.compile_jump_if(&statement.test, true, success)?;
            self.assembler
                .set_location(self.source_location(statement.range));
            self.emit_common_constant(CommonConstant::ASSERTIONERROR)?;
            if let Some(message) = &statement.msg {
                self.compile_expression(message)?;
                self.assembler
                    .set_location(self.source_location(statement.range));
                self.emit(CALL, 0, -1)?;
            }
            self.assembler
                .set_location(self.source_location(statement.test.range()));
            self.emit(RAISE_VARARGS, 1, -1)?;

            self.assembler.mark(success);
            self.set_depth(base_depth);
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
            return Ok(());
        }

        let failure = self.assembler.label();
        self.compile_loop_tail_condition(&statement.test, false, failure, restart)?;

        self.assembler.mark(failure);
        self.set_depth(base_depth);
        self.assembler
            .set_location(self.source_location(statement.range));
        self.emit_common_constant(CommonConstant::ASSERTIONERROR)?;
        if let Some(message) = &statement.msg {
            self.compile_expression(message)?;
            self.assembler
                .set_location(self.source_location(statement.range));
            self.emit(CALL, 0, -1)?;
        }
        self.assembler
            .set_location(self.source_location(statement.test.range()));
        self.emit(RAISE_VARARGS, 1, -1)?;
        Ok(())
    }

    fn emit_loop_tail_backedge(
        &mut self,
        restart: Label,
        exclude_from_innermost_exception: bool,
    ) -> Result<(), CompileError> {
        let exclusion_start = exclude_from_innermost_exception.then(|| {
            let start = self.assembler.label();
            self.assembler.mark(start);
            start
        });
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        if let Some(exclusion_start) = exclusion_start {
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            self.control
                .active_exception_region_exclusions
                .last_mut()
                .expect("try-body loop tail requires an active exception region")
                .push((exclusion_start, exclusion_end));
        }
        Ok(())
    }

    fn compile_loop_tail_if(
        &mut self,
        statement: &ruff_python_ast::StmtIf,
        restart: Label,
        split_short_circuit: bool,
        mode: LoopTailMode,
        leading: &[Stmt],
    ) -> Result<(), CompileError> {
        let LoopTailMode {
            non_final_branch,
            try_body_normal_exit,
            propagate_try_body_normal_exit,
        } = mode;
        let base_depth = self.control.depth;
        let first_branch_reuses_protected_owner = matches!(
            (leading, statement.test.as_ref()),
            ([Stmt::Assign(assignment)], Expr::Name(test))
                if matches!(assignment.targets.as_slice(), [Expr::Name(target)]
                    if target.id.as_str() == test.id.as_str())
        );
        if statement.elif_else_clauses.is_empty() {
            let protected_try_normal_exit = try_body_normal_exit
                && self.control.generator_region_start.is_none()
                && self.control.active_with_region_exclusions.is_empty()
                && !self.control.active_exception_region_exclusions.is_empty();
            // CPython keeps a non-final branch's nested condition body before the fallback
            // backedge even when that body exits the enclosing loop or function.
            if non_final_branch
                && !split_short_circuit
                && (!self.has_active_control_flow_region() || protected_try_normal_exit)
                && condition_jump_target_count(&statement.test, false) == 1
                && (condition_jump_target_count(&statement.test, true) == 1
                    || (statement.body.len() == 1
                        && matches!(statement.body.first(), Some(Stmt::Assign(_)))))
            {
                let body = statement.body.as_slice();
                let fallback = self.assembler.label();
                self.with_control_override(
                    |control| &mut control.exclude_condition_not_taken_from_exception,
                    protected_try_normal_exit,
                    |compiler| compiler.compile_jump_if(&statement.test, false, fallback),
                )?;
                let body_start = self.assembler.instruction_count();
                let nested_tail = self.compile_with_strict_owned_loads(
                    matches!(statement.test.as_ref(), Expr::If(_)),
                    |compiler| {
                        compiler.compile_non_final_branch_loop_tail_suite(
                            body,
                            restart,
                            propagate_try_body_normal_exit,
                        )
                    },
                )?;
                if nested_tail == LoopBackedge::Required && !suite_terminates(body) {
                    self.set_branch_end_location(body, body_start);
                    self.emit_loop_tail_backedge(restart, protected_try_normal_exit)?;
                }

                self.assembler.mark(fallback);
                self.set_depth(base_depth);
                self.assembler.set_location(
                    self.source_location(condition_fallthrough_range(&statement.test)),
                );
                self.emit_loop_tail_backedge(restart, protected_try_normal_exit)?;
                return Ok(());
            }
            if !try_body_normal_exit {
                if self.control.generator_region_start.is_none()
                    && self.control.protects_boolean_return_iterator_cleanup()
                    && let Expr::UnaryOp(unary) = statement.test.as_ref()
                    && unary.op == UnaryOp::Not
                    && let Expr::Compare(comparison) = unary.operand.as_ref()
                    && comparison.ops.len() > 1
                {
                    return self.compile_loop_tail_chained_compare(
                        statement,
                        comparison,
                        restart,
                        split_short_circuit,
                        true,
                    );
                }
                if self.control.generator_region_start.is_none()
                    && let Expr::Compare(comparison) = statement.test.as_ref()
                    && comparison.ops.len() > 1
                {
                    return self.compile_loop_tail_chained_compare(
                        statement,
                        comparison,
                        restart,
                        split_short_circuit,
                        false,
                    );
                }
                let body = statement.body.as_slice();
                let body_label = self.assembler.label();
                self.compile_loop_tail_condition(&statement.test, true, body_label, restart)?;
                self.assembler.mark(body_label);
                self.set_depth(base_depth);
                let body_start = self.assembler.instruction_count();
                let nested_tail = self.compile_with_strict_owned_loads(
                    matches!(statement.test.as_ref(), Expr::If(_)),
                    |compiler| compiler.compile_loop_tail_suite(body, restart, split_short_circuit),
                )?;
                if nested_tail == LoopBackedge::Required && !suite_terminates(body) {
                    self.set_branch_end_location(body, body_start);
                    self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
                }
                return Ok(());
            }
        }
        let mut branches: Vec<(Option<&Expr>, &[Stmt])> =
            vec![(Some(&statement.test), statement.body.as_slice())];
        branches.extend(
            statement
                .elif_else_clauses
                .iter()
                .map(|clause| (clause.test.as_ref(), clause.body.as_slice())),
        );
        let branch_count = branches.len();
        let has_else = branches.last().is_some_and(|(test, _)| test.is_none());

        for (branch_index, (test, body)) in branches.into_iter().enumerate() {
            if !try_body_normal_exit && !has_else && branch_index + 1 == branch_count {
                let test = test.expect("last branch without else has a test");
                if non_final_branch
                    && !self.has_active_control_flow_region()
                    && condition_jump_target_count(test, false) == 1
                    && condition_jump_target_count(test, true) == 1
                {
                    // CPython retains source order for a final `elif` nested in a non-final
                    // loop-tail branch: the body falls through and the false edge reaches the
                    // enclosing backedge directly.
                    let fallback = self.assembler.label();
                    self.compile_jump_if(test, false, fallback)?;
                    let body_start = self.assembler.instruction_count();
                    let nested_tail = self.compile_with_strict_owned_loads(
                        matches!(test, Expr::If(_)),
                        |compiler| {
                            compiler.compile_non_final_branch_loop_tail_suite(body, restart, false)
                        },
                    )?;
                    if nested_tail == LoopBackedge::Required && !suite_terminates(body) {
                        self.set_branch_end_location(body, body_start);
                        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
                    }

                    self.assembler.mark(fallback);
                    self.set_depth(base_depth);
                    self.assembler
                        .set_location(self.source_location(synthetic_condition_exit_range(test)));
                    self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
                    return Ok(());
                }
                let body_label = self.assembler.label();
                // Chained comparisons retain a separate cleanup join before the loop edge.
                // Keep their generic Boolean join until that loop-tail layout is modeled;
                // ordinary short-circuit operands normalize directly against the backedge.
                if split_short_circuit && !expression_contains_chained_comparison(test) {
                    self.compile_loop_tail_condition(test, true, body_label, restart)?;
                } else {
                    self.compile_loop_tail_condition_atom(test, true, body_label, restart)?;
                }
                self.assembler.mark(body_label);
                self.set_depth(base_depth);
                let body_start = self.assembler.instruction_count();
                let nested_tail = self
                    .compile_with_strict_owned_loads(matches!(test, Expr::If(_)), |compiler| {
                        compiler.compile_loop_tail_suite(body, restart, split_short_circuit)
                    })?;
                if nested_tail == LoopBackedge::Required && !suite_terminates(body) {
                    self.set_branch_end_location(body, body_start);
                    self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
                }
                return Ok(());
            }
            let next = test.map(|_| self.assembler.label());
            if let (Some(test), Some(next)) = (test, next) {
                let has_outer_exception_region = try_body_normal_exit
                    && self.control.active_exception_region_exclusions.len() > 1;
                let protected_nested_branch = try_body_normal_exit && non_final_branch;
                let exclude_from_control_flow_regions = has_outer_exception_region
                    || protected_nested_branch
                    || (!try_body_normal_exit
                        && (!self.control.active_exception_region_exclusions.is_empty()
                            || self
                                .control
                                .exclude_loop_tail_not_taken_from_control_flow_regions));
                let exclude_from_all_exception_regions = has_outer_exception_region
                    || (!try_body_normal_exit
                        && !self.control.active_exception_region_exclusions.is_empty());
                self.with_control_override(
                    |control| &mut control.exclude_condition_not_taken_from_exception,
                    exclude_from_control_flow_regions,
                    |compiler| {
                        compiler.with_control_override(
                            |control| {
                                &mut control.exclude_condition_not_taken_from_all_exception_regions
                            },
                            exclude_from_all_exception_regions,
                            |compiler| compiler.compile_jump_if(test, false, next),
                        )
                    },
                )?;
                if branch_index == 0
                    && !try_body_normal_exit
                    && !non_final_branch
                    && first_branch_reuses_protected_owner
                    && self.control.in_iterator_loop()
                    && !self.control.active_with_region_exclusions.is_empty()
                    && self
                        .control
                        .active_with_exits
                        .last()
                        .is_some_and(|context| {
                            !context.is_async
                                && context.loop_depth.checked_add(1)
                                    == Some(self.control.loops.len())
                        })
                {
                    // CPython reuses the protected owner on the first branch fallthrough of a
                    // multi-branch loop tail. Later branch markers start in fresh CFG blocks.
                    self.assembler.preserve_last_not_taken_exception_owner();
                }
                if branch_index == 0
                    && has_else
                    && !try_body_normal_exit
                    && is_flattening_yield_branch(statement)
                    && !non_final_branch
                    && matches!(self.control.loops.last(), Some(context)
                        if context.iterator_cleanup == IteratorCleanup::Sync)
                    && self.control.generator_region_start.is_some()
                    && !self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_with_region_exclusions.is_empty()
                {
                    // A generator's first multi-branch loop-tail marker reuses the surrounding
                    // try owner after the normal loop-exit edge is copied through the handler.
                    self.assembler.preserve_last_not_taken_exception_owner();
                }
            }
            // A non-final branch still targets the following `elif`; its nested Boolean joins
            // therefore remain forward instead of splitting directly around the loop edge.
            let nested_tail = self.compile_with_strict_owned_loads(
                test.is_some_and(|test| matches!(test, Expr::If(_))),
                |compiler| {
                    compiler.compile_loop_tail_suite_inner(
                        body,
                        restart,
                        false,
                        LoopTailMode {
                            non_final_branch: test.is_some() || non_final_branch,
                            try_body_normal_exit: propagate_try_body_normal_exit,
                            propagate_try_body_normal_exit,
                        },
                    )
                },
            )?;
            if nested_tail == LoopBackedge::Required && !suite_terminates(body) {
                if let Some(location) = self.assembler.last_instruction_location() {
                    self.assembler.set_location(location);
                } else if let Some(statement) = body.last() {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                }
                self.emit_loop_tail_backedge(restart, try_body_normal_exit)?;
            }
            if let Some(next) = next {
                self.assembler.mark(next);
                self.set_depth(base_depth);
            }
        }
        if !has_else {
            let fallthrough_test = if try_body_normal_exit {
                statement
                    .elif_else_clauses
                    .iter()
                    .rev()
                    .find_map(|clause| clause.test.as_ref())
                    .unwrap_or(statement.test.as_ref())
            } else {
                statement.test.as_ref()
            };
            self.assembler.set_location(
                self.source_location(synthetic_condition_exit_range(fallthrough_test)),
            );
            self.emit_loop_tail_backedge(restart, try_body_normal_exit)?;
        }
        Ok(())
    }

    fn compile_loop_tail_condition(
        &mut self,
        expression: &Expr,
        jump_on: bool,
        success: Label,
        restart: Label,
    ) -> Result<(), CompileError> {
        if let Expr::UnaryOp(unary) = expression
            && unary.op == UnaryOp::Not
        {
            return self.compile_loop_tail_condition(&unary.operand, !jump_on, success, restart);
        }
        if let Expr::BoolOp(boolean) = expression {
            let Some((last, leading)) = boolean.values.split_last() else {
                return Err(CompileError::Internal(
                    "boolean expression contains no values".to_string(),
                ));
            };
            let short_circuit_value = boolean.op == BoolOp::Or;
            if short_circuit_value == jump_on {
                for value in leading {
                    self.compile_jump_if(value, jump_on, success)?;
                }
            } else {
                for value in leading {
                    let next = self.assembler.label();
                    self.compile_loop_tail_condition(value, !short_circuit_value, next, restart)?;
                    self.assembler.mark(next);
                }
            }
            return self.compile_loop_tail_condition(last, jump_on, success, restart);
        }
        self.compile_loop_tail_condition_atom(expression, jump_on, success, restart)
    }

    fn compile_loop_tail_condition_atom(
        &mut self,
        expression: &Expr,
        jump_on: bool,
        success: Label,
        restart: Label,
    ) -> Result<(), CompileError> {
        let control_region = (early_condition_truthiness(expression).is_none()
            && self.has_active_control_flow_region())
        .then(|| (self.assembler.label(), self.assembler.label()));
        let generator_exclusion = (control_region.is_none()
            && self.control.generator_region_start.is_some())
        .then(|| self.assembler.label());
        self.compile_jump_if(expression, jump_on, success)?;
        if let Some((control_start, _)) = control_region {
            self.assembler
                .mark_before_trailing_instructions(control_start, 2);
        }
        if let Some(exclusion_start) = generator_exclusion {
            self.assembler
                .mark_before_trailing_instructions(exclusion_start, 2);
        }
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        if let Some((control_start, control_end)) = control_region {
            self.assembler.mark(control_end);
            self.add_control_flow_region_exclusion(control_start, control_end);
        }
        if let Some(exclusion_start) = generator_exclusion {
            self.control
                .generator_region_exclusions
                .push((exclusion_start, success));
        }
        Ok(())
    }

    fn compile_loop_tail_chained_compare(
        &mut self,
        statement: &ruff_python_ast::StmtIf,
        comparison: &ruff_python_ast::ExprCompare,
        restart: Label,
        split_short_circuit: bool,
        negated: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let cleanup = self.assembler.label();
        let success = self.assembler.label();
        let body = self.assembler.label();

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
            let (opcode, argument) = comparison_operator_boolean(*operator);
            self.emit(opcode, argument, -1)?;
            self.emit_jump_forward(POP_JUMP_IF_FALSE, cleanup, -1)?;
            self.emit(NOT_TAKEN, 0, 0)?;
        }

        self.compile_expression(comparison.comparators.last().unwrap())?;
        self.assembler
            .set_location(self.source_location(comparison.range));
        let (opcode, argument) = comparison_operator_boolean(*comparison.ops.last().unwrap());
        self.emit(opcode, argument, -1)?;
        self.emit_jump_forward(
            if negated {
                POP_JUMP_IF_FALSE
            } else {
                POP_JUMP_IF_TRUE
            },
            success,
            -1,
        )?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;

        self.assembler.mark(success);
        self.set_depth(base_depth);
        self.assembler
            .set_location(self.source_location(comparison.range));
        self.emit_jump_forward(JUMP_FORWARD, body, 0)?;
        if negated {
            self.assembler.preserve_last_inlined_jump_nop();
        }

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 1);
        self.assembler
            .set_location(self.source_location(comparison.range));
        self.emit(POP_TOP, 0, -1)?;
        if negated {
            self.emit_jump_forward(JUMP_FORWARD, body, 0)?;
        } else {
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        }

        self.assembler.mark(body);
        self.set_depth(base_depth);
        let body_start = self.assembler.instruction_count();
        let nested_tail =
            self.compile_loop_tail_suite(&statement.body, restart, split_short_circuit)?;
        if nested_tail == LoopBackedge::Required && !suite_terminates(&statement.body) {
            self.set_branch_end_location(&statement.body, body_start);
            self.emit_jump_backward(JUMP_BACKWARD, restart, 0)?;
        }
        Ok(())
    }

    fn compile_while(
        &mut self,
        statement: &ruff_python_ast::StmtWhile,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        if let Some(truthiness) = early_condition_truthiness(&statement.test) {
            // CPython interns a folded loop test before compiling the body, even though the
            // optimized condition emits no load. Unused nonzero constants are pruned later;
            // if the value is used in the body, this preserves its source-order slot.
            if let Some(constant) = fold_constant(&statement.test) {
                self.add_constant(constant)?;
            }
            if !truthiness {
                self.pre_register_suite_names(&statement.body)?;
                // The eliminated loop body leaves a CFG boundary that CPython's borrowed-load
                // optimizer does not traverse into the following block.
                self.assembler.set_strict_owned_loads(true);
                self.assembler
                    .set_location(self.source_location(statement.test.range()));
                self.emit(NOP, 0, 0)?;
                self.compile_suite(&statement.orelse)?;
                return Ok(());
            }

            let start = self.assembler.label();
            let end = self.assembler.label();
            self.assembler.mark(start);
            self.assembler
                .set_location(self.source_location(statement.test.range()));
            self.emit(NOP, 0, 0)?;
            let context = self.loop_context(
                start,
                end,
                IteratorCleanup::None,
                self.loop_break_policy(statement.range),
            );
            let body_start = self.assembler.instruction_count();
            let tail_jumps = self.with_loop_context(context, |compiler| {
                compiler.compile_while_tail_suite(&statement.body, start)
            })?;
            if tail_jumps == LoopBackedge::Required && !suite_terminates(&statement.body) {
                self.emit_while_backedge(&statement.body, body_start, start)?;
            }
            self.pre_register_suite_names(&statement.orelse)?;
            self.assembler.mark(end);
            let copied_direct_return_exit = self.control.current_statement_followed_by_name_return
                && self.control.active_exception_region_exclusions.is_empty()
                && self.control.active_with_region_exclusions.is_empty()
                && suite_ends_in_simple_conditional_break(&statement.body);
            let coroutine_terminal_break_exit = self.control.generator_region_start.is_some()
                && suite_contains_loop_continue(&statement.body)
                && statement.body.last().is_some_and(|statement| {
                    suite_contains_loop_break(std::slice::from_ref(statement))
                });
            if suite_contains_loop_break(&statement.body)
                && !copied_direct_return_exit
                && (!self.control.active_exception_region_exclusions.is_empty()
                    || !self.control.active_with_region_exclusions.is_empty()
                    || coroutine_terminal_break_exit
                    || !suite_contains_borrow_reachable_conditional_loop_break(&statement.body))
            {
                // A constant-true loop can reach its exit only through `break`. CPython removes
                // an unconditional break jump before borrowed-load analysis but does not
                // reconnect the large exit block. A conditional break retains an explicit exit
                // edge when it is non-tail, when its cleanup crosses a protected region, or when
                // it shares a final `if` with an explicit fallthrough branch. A surrounding
                // control-flow region can still rewrite that edge during cleanup. A coroutine
                // stop handler with a continue edge also keeps a terminal break exit disconnected
                // from the following block.
                self.assembler.prevent_borrow_reachability(end);
            }
            self.set_depth(base_depth);
            if statement.orelse.is_empty() {
                self.assembler
                    .set_location(self.source_location(statement.test.range()));
            }
            return Ok(());
        }
        let start = self.assembler.label();
        let else_label = self.assembler.label();
        let end = self.assembler.label();

        self.assembler.mark(start);
        let exclude_not_taken = !self.control.active_exception_region_exclusions.is_empty()
            || self.control.active_terminal_withs > 0
            || self.control.generator_region_start.is_some();
        let condition_fallthrough_to_loop_backedge =
            self.control.suite_fallthrough_to_loop_backedge;
        self.with_control_override(
            |control| &mut control.exclude_condition_not_taken_from_exception,
            exclude_not_taken,
            |compiler| {
                compiler.with_control_override(
                    |control| &mut control.loop_tail_while_condition,
                    condition_fallthrough_to_loop_backedge,
                    |compiler| compiler.compile_jump_if(&statement.test, false, else_label),
                )
            },
        )?;
        let context = self.loop_context(
            start,
            end,
            IteratorCleanup::None,
            self.loop_break_policy(statement.range),
        );
        let body_start = self.assembler.instruction_count();
        let tail_jumps = self.with_loop_context(context, |compiler| {
            compiler.compile_while_tail_suite(&statement.body, start)
        })?;
        if tail_jumps == LoopBackedge::Required && !suite_terminates(&statement.body) {
            self.emit_while_backedge(&statement.body, body_start, start)?;
        }

        self.assembler.mark(else_label);
        self.set_depth(base_depth);
        // CPython keeps the converted protected-region POP_BLOCK marker only when the while
        // condition contributes one edge to the normal exit.
        if statement.orelse.is_empty()
            && !self.control.current_statement_has_following
            && (!self.control.active_with_region_exclusions.is_empty()
                || (!self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_finally_try_bodies == 0
                    && !matches!(statement.test.as_ref(), Expr::BoolOp(_))
                    && !suite_contains_nested_loop_raise(&statement.body)
                    && self
                        .control
                        .direct_try_body_statements
                        .last()
                        .is_some_and(|range| *range == statement.range)))
            && condition_jump_target_count(&statement.test, false) == 1
        {
            let exclusion_start = self.assembler.label();
            self.assembler.mark(exclusion_start);
            self.assembler
                .set_location(self.source_location(condition_exit_range(&statement.test)));
            self.emit(NOP, 0, 0)?;
            let exclusion_end = self.assembler.label();
            self.assembler.mark(exclusion_end);
            if self.control.generator_region_start.is_some() {
                self.control
                    .generator_region_exclusions
                    .push((exclusion_start, exclusion_end));
            }
            for exclusions in &mut self.control.active_with_region_exclusions {
                exclusions.push((exclusion_start, exclusion_end));
            }
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((exclusion_start, exclusion_end));
            }
        }
        self.compile_suite(&statement.orelse)?;
        self.assembler.mark(end);
        self.set_depth(base_depth);
        if statement.orelse.is_empty() {
            self.assembler
                .set_location(self.source_location(condition_exit_range(&statement.test)));
        }
        Ok(())
    }

    fn compile_for(&mut self, statement: &ruff_python_ast::StmtFor) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let has_break = suite_contains_loop_break(&statement.body);
        let start = self.assembler.label();
        let cleanup = self.assembler.label();
        let end = self.assembler.label();
        let mut newly_initialized_targets = Vec::new();
        collect_target_names(&statement.target, &mut newly_initialized_targets);
        newly_initialized_targets.retain(|name| !self.is_initialized(name));

        self.compile_iterable_expression(&statement.iter)?;
        self.assembler
            .set_location(self.source_location(statement.iter.range()));
        self.emit(GET_ITER, 0, 0)?;
        self.assembler.mark(start);
        self.emit_jump_forward(FOR_ITER, cleanup, 1)?;
        self.compile_store_target(&statement.target)?;

        let mut context = self.loop_context(
            start,
            end,
            IteratorCleanup::Sync,
            self.loop_break_policy(statement.range),
        );
        context.protect_boolean_return_cleanup = matches!(
            statement.body.as_slice(),
            [Stmt::If(branch)]
                if branch.elif_else_clauses.is_empty()
                    && matches!(
                        branch.body.as_slice(),
                        [Stmt::Return(return_statement)]
                            if matches!(
                                return_statement.value.as_deref(),
                                Some(Expr::BooleanLiteral(_))
                            )
                    )
        );
        let body_start = self.assembler.instruction_count();
        let tail_jumps = self.with_loop_context(context, |compiler| {
            compiler.compile_loop_tail_suite(&statement.body, start, true)
        })?;
        if tail_jumps == LoopBackedge::Required && !suite_terminates(&statement.body) {
            if self.assembler.instruction_count() > body_start
                && let Some(location) = self.assembler.last_instruction_location()
            {
                self.assembler.set_location(location);
            } else if let Some(statement) = statement.body.last() {
                self.assembler
                    .set_location(self.source_location(statement.range()));
            }
            let loop_tail_boolean = matches!(statement.body.last(), Some(Stmt::Expr(expression))
                if matches!(expression.value.as_ref(), Expr::BoolOp(_)));
            if loop_tail_boolean {
                self.assembler.set_location(SourceLocation::NONE);
            }
            self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
            if loop_tail_boolean {
                // CPython duplicates this unlocated loop exit onto every short-circuit edge.
                self.assembler.mark_last_as_artificial_eval_break();
            }
            if statement
                .body
                .last()
                .is_some_and(try_has_conditional_shared_loop_exit)
            {
                self.assembler.mark_last_as_artificial_eval_break();
            }
        }

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 2);
        self.assembler
            .set_location(self.source_location(statement.iter.range()));
        self.emit(END_FOR, 0, -1)?;
        self.emit(POP_ITER, 0, -1)?;
        for name in &newly_initialized_targets {
            let name = self.required_name_id(name);
            self.symbols.initialized_locals.remove(name);
        }
        let orelse_start = self.assembler.instruction_count();
        self.compile_suite(&statement.orelse)?;
        if self.assembler.instruction_count() == orelse_start
            && let [Stmt::Pass(statement)] = statement.orelse.as_slice()
            && self.control.active_exception_region_exclusions.is_empty()
            && !self.control.active_with_region_exclusions.is_empty()
        {
            let noop_start = self.assembler.label();
            self.assembler.mark(noop_start);
            self.assembler
                .set_location(self.source_location(statement.range));
            self.emit(NOP, 0, 0)?;
            let noop_end = self.assembler.label();
            self.assembler.mark(noop_end);
            for exclusions in &mut self.control.active_with_region_exclusions {
                exclusions.push((noop_start, noop_end));
            }
        }
        if has_break {
            self.assembler.mark(end);
        }
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_async_for(
        &mut self,
        statement: &ruff_python_ast::StmtFor,
    ) -> Result<(), CompileError> {
        if self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR) == 0 {
            return Err(unsupported("async for outside coroutine code"));
        }

        let base_depth = self.control.depth;
        let has_break = suite_contains_loop_break(&statement.body);
        let start = self.assembler.label();
        let protected_start = self.assembler.label();
        let yielded = self.assembler.label();
        let yielded_end = self.assembler.label();
        let send = self.assembler.label();
        let send_end = self.assembler.label();
        let protected_end = self.assembler.label();
        let cleanup_throw = self.assembler.label();
        let async_cleanup = self.assembler.label();
        let end = self.assembler.label();
        let mut newly_initialized_targets = Vec::new();
        collect_target_names(&statement.target, &mut newly_initialized_targets);
        newly_initialized_targets.retain(|name| !self.is_initialized(name));

        self.compile_expression(&statement.iter)?;
        let iterator_location = self.source_location(statement.iter.range());
        self.assembler.set_location(iterator_location);
        self.emit(GET_AITER, 0, 0)?;
        self.assembler
            .set_location(self.source_location(statement.range));
        self.assembler.mark(start);
        self.assembler.mark(protected_start);
        self.emit(GET_ANEXT, 0, 1)?;
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.assembler.mark(send);
        self.emit_jump_forward(SEND, send_end, 0)?;
        self.assembler.mark(yielded);
        self.emit(YIELD_VALUE, 1, 0)?;
        self.assembler.mark(yielded_end);
        self.emit_resume(ResumeLocation::AFTER_AWAIT, ResumeDepth::Zero)?;
        self.emit_jump_backward(JUMP_BACKWARD_NO_INTERRUPT, send, 0)?;
        self.assembler.mark(send_end);
        self.set_depth(base_depth + 3);
        self.emit(END_SEND, 0, -1)?;
        self.assembler.mark(protected_end);
        self.emit(NOT_TAKEN, 0, 0)?;
        self.compile_store_target(&statement.target)?;

        let context = self.loop_context(
            start,
            end,
            IteratorCleanup::Async,
            self.loop_break_policy(statement.range),
        );
        let body_start = self.assembler.instruction_count();
        let tail_jumps = self.with_loop_context(context, |compiler| {
            compiler.compile_loop_tail_suite(&statement.body, start, true)
        })?;
        if tail_jumps == LoopBackedge::Required && !suite_terminates(&statement.body) {
            if self.assembler.instruction_count() > body_start
                && let Some(location) = self.assembler.last_instruction_location()
            {
                self.assembler.set_location(location);
            } else if let Some(statement) = statement.body.last() {
                self.assembler
                    .set_location(self.source_location(statement.range()));
            }
            self.emit_jump_backward(JUMP_BACKWARD, start, 0)?;
        }

        self.assembler.mark(cleanup_throw);
        self.set_depth(base_depth + 4);
        self.assembler
            .set_location(self.source_location(statement.range));
        self.emit(CLEANUP_THROW, 0, -1)?;
        let cleanup_throw_end = self.assembler.label();
        self.assembler.mark(cleanup_throw_end);
        self.emit_jump_backward(JUMP_BACKWARD_NO_INTERRUPT, send_end, 0)?;
        self.assembler.add_exception_region(
            yielded,
            yielded_end,
            cleanup_throw,
            (base_depth + 3).cast_unsigned(),
            false,
        );

        self.assembler.mark(async_cleanup);
        self.control
            .generator_region_exclusions
            .push((cleanup_throw, async_cleanup));
        for exclusions in &mut self.control.active_with_region_exclusions {
            exclusions.push((cleanup_throw_end, async_cleanup));
        }
        for exclusions in &mut self.control.active_exception_region_exclusions {
            exclusions.push((cleanup_throw_end, async_cleanup));
        }
        self.set_depth(base_depth + 2);
        self.assembler.set_location(iterator_location);
        self.apply_compiler_stack_effect(END_ASYNC_FOR, 0, -2)?;
        self.assembler.emit_backward(END_ASYNC_FOR, send_end);
        self.assembler.add_exception_region(
            protected_start,
            protected_end,
            async_cleanup,
            (base_depth + 1).cast_unsigned(),
            false,
        );
        self.assembler.add_exception_region(
            cleanup_throw,
            cleanup_throw_end,
            async_cleanup,
            (base_depth + 1).cast_unsigned(),
            false,
        );
        for name in &newly_initialized_targets {
            let name = self.required_name_id(name);
            self.symbols.initialized_locals.remove(name);
        }
        self.compile_suite(&statement.orelse)?;
        if has_break {
            self.assembler.mark(end);
            if self.control.active_exception_region_exclusions.is_empty()
                && !suite_contains_borrow_reachable_conditional_loop_break(&statement.body)
            {
                // The normal exhaustion edge reaches this block only through END_ASYNC_FOR's
                // exception handler. A simple or tail break has its direct jump removed before
                // borrowed-load traversal, leaving the following block unreachable.
                self.assembler.prevent_borrow_reachability(end);
            }
        }
        if self.control.emit_protected_async_for_end_nop
            && statement.orelse.is_empty()
            && !has_break
        {
            let noop_start = self.assembler.label();
            self.assembler.mark(noop_start);
            self.assembler
                .set_location(self.source_location(statement.iter.range()));
            self.emit(NOP, 0, 0)?;
            let noop_end = self.assembler.label();
            self.assembler.mark(noop_end);
            // CPython's coroutine stop handler excludes both the synthetic end block and the
            // preceding END_ASYNC_FOR, while the surrounding protected region still owns the
            // END_ASYNC_FOR itself.
            self.control
                .generator_region_exclusions
                .push((async_cleanup, noop_end));
            for exclusions in &mut self.control.active_with_region_exclusions {
                exclusions.push((noop_start, noop_end));
            }
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((noop_start, noop_end));
            }
        }
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_augmented_assignment(
        &mut self,
        assignment: &ruff_python_ast::StmtAugAssign,
    ) -> Result<(), CompileError> {
        match assignment.target.as_ref() {
            Expr::Name(target) => {
                let target_location = self.source_location(target.range);
                self.assembler.set_location(target_location);
                self.load_name(target.id.as_str())?;
                self.compile_expression(&assignment.value)?;
                self.assembler
                    .set_location(self.source_location(assignment.range));
                self.emit_binary_operation(binary_operator(assignment.op, true))?;
                self.assembler.set_location(target_location);
                self.store_name(target.id.as_str())?;
            }
            Expr::Attribute(target) => {
                self.compile_expression(&target.value)?;
                self.assembler
                    .set_location(self.source_location(target.range));
                self.emit(COPY, 1, 1)?;
                let index = self.name_index(target.attr.as_str())?;
                let attribute_location = self.source_location(self.attribute_opcode_range(target));
                self.assembler.set_location(attribute_location);
                self.emit(LOAD_ATTR, index << 1, 0)?;
                self.compile_expression(&assignment.value)?;
                self.assembler
                    .set_location(self.source_location(assignment.range));
                self.emit_binary_operation(binary_operator(assignment.op, true))?;
                self.assembler.set_location(attribute_location);
                self.emit(SWAP, 2, 0)?;
                self.emit(STORE_ATTR, index, -2)?;
            }
            Expr::Subscript(target) => {
                self.compile_expression(&target.value)?;
                let optimized_slice = if let Expr::Slice(slice) = target.slice.as_ref()
                    && two_element_slice_optimization(slice)
                {
                    let slice_location = self.source_location(slice.range);
                    self.compile_optional_slice_bound(slice.lower.as_deref(), slice_location)?;
                    self.compile_optional_slice_bound(slice.upper.as_deref(), slice_location)?;
                    true
                } else {
                    self.compile_expression(&target.slice)?;
                    false
                };
                let target_location = self.source_location(target.range);
                self.assembler.set_location(target_location);
                if optimized_slice {
                    self.emit(COPY, 3, 1)?;
                    self.emit(COPY, 3, 1)?;
                    self.emit(COPY, 3, 1)?;
                    self.emit(BINARY_SLICE, 0, -2)?;
                } else {
                    self.emit(COPY, 2, 1)?;
                    self.emit(COPY, 2, 1)?;
                    self.emit_binary_operation(BinaryOperation::SUBSCR)?;
                }
                self.compile_expression(&assignment.value)?;
                self.assembler
                    .set_location(self.source_location(assignment.range));
                self.emit_binary_operation(binary_operator(assignment.op, true))?;
                self.assembler.set_location(target_location);
                if optimized_slice {
                    self.emit(SWAP, 4, 0)?;
                    self.emit(SWAP, 3, 0)?;
                    self.emit(SWAP, 2, 0)?;
                    self.emit(STORE_SLICE, 0, -4)?;
                } else {
                    self.emit(SWAP, 3, 0)?;
                    self.emit(SWAP, 2, 0)?;
                    self.emit(STORE_SUBSCR, 0, -3)?;
                }
            }
            _ => return Err(unsupported("augmented assignment target")),
        }
        Ok(())
    }

    fn compile_annotated_assignment(
        &mut self,
        assignment: &ruff_python_ast::StmtAnnAssign,
    ) -> Result<(), CompileError> {
        if assignment.simple
            && matches!(self.unit.scope, Scope::Class { .. })
            && self.unit.flags & CO_FUTURE_ANNOTATIONS == 0
        {
            if let Some(value) = &assignment.value {
                self.compile_expression(value)?;
                self.compile_store_target(&assignment.target)?;
            }
            if self
                .unit
                .class_conditional_annotations
                .front()
                .is_some_and(|range| *range == assignment.range)
            {
                self.unit.class_conditional_annotations.pop_front();
                let index = self.unit.module_annotation_index;
                self.unit.module_annotation_index += 1;
                self.assembler
                    .set_location(self.source_location(assignment.range));
                let cell = self.closure_index("__conditional_annotations__")?;
                self.emit(LOAD_DEREF, cell, 1)?;
                self.emit(LOAD_SMALL_INT, index, 1)?;
                self.emit(SET_ADD, 1, -1)?;
                self.emit(POP_TOP, 0, -1)?;
            }
            return Ok(());
        }
        if assignment.simple
            && (matches!(self.unit.scope, Scope::Module)
                || matches!(self.unit.scope, Scope::Class { .. })
                    && self.unit.flags & CO_FUTURE_ANNOTATIONS != 0)
        {
            if let Some(value) = &assignment.value {
                self.compile_expression(value)?;
                self.compile_store_target(&assignment.target)?;
            }
            let Expr::Name(target) = assignment.target.as_ref() else {
                return Err(unsupported("simple annotation target"));
            };
            if self.unit.flags & CO_FUTURE_ANNOTATIONS != 0 {
                self.assembler
                    .set_location(self.source_location(assignment.annotation.range()));
                let annotation = unparse_annotation(&assignment.annotation);
                self.load_string_constant(&annotation)?;
                self.assembler
                    .set_location(self.source_location(assignment.range));
                self.load_name("__annotations__")?;
                let target = self.mangled_name(target.id.as_str());
                self.load_string_constant(&target)?;
                self.emit(STORE_SUBSCR, 0, -3)?;
            } else {
                let index = self.unit.module_annotation_index;
                self.unit.module_annotation_index += 1;
                if self.output.constants.is_empty() {
                    self.add_constant(Constant::Int(u64::from(index)))?;
                }
                self.assembler
                    .set_location(self.source_location(assignment.range));
                self.load_name("__conditional_annotations__")?;
                self.emit(LOAD_SMALL_INT, index, 1)?;
                self.emit(SET_ADD, 1, -1)?;
                self.emit(POP_TOP, 0, -1)?;
            }
            return Ok(());
        }
        if let Some(value) = &assignment.value {
            self.compile_expression(value)?;
            self.compile_store_target(&assignment.target)?;
            return Ok(());
        }

        match assignment.target.as_ref() {
            Expr::Name(_) => {}
            Expr::Attribute(attribute) => {
                self.compile_expression(&attribute.value)?;
                self.assembler
                    .set_location(self.source_location(attribute.value.range()));
                self.emit(POP_TOP, 0, -1)?;
            }
            Expr::Subscript(subscript) => {
                self.compile_expression(&subscript.value)?;
                self.assembler
                    .set_location(self.source_location(subscript.value.range()));
                self.emit(POP_TOP, 0, -1)?;
                self.compile_discarded_annotation_slice(&subscript.slice)?;
            }
            _ => return Err(unsupported("annotated assignment target")),
        }
        Ok(())
    }

    fn compile_discarded_annotation_slice(
        &mut self,
        expression: &Expr,
    ) -> Result<(), CompileError> {
        match expression {
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.compile_discarded_annotation_slice(element)?;
                }
            }
            Expr::Slice(slice) => {
                for element in [
                    slice.lower.as_deref(),
                    slice.upper.as_deref(),
                    slice.step.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    self.compile_discarded_annotation_slice(element)?;
                }
            }
            _ if fold_constant(expression).is_some() => {
                if let Some(constant) = first_literal_constant(expression) {
                    self.add_constant(constant)?;
                }
            }
            _ => {
                self.compile_expression(expression)?;
                self.assembler
                    .set_location(self.source_location(expression.range()));
                self.emit(POP_TOP, 0, -1)?;
            }
        }
        Ok(())
    }

    fn compile_assert(
        &mut self,
        statement: &ruff_python_ast::StmtAssert,
    ) -> Result<(), CompileError> {
        if early_condition_truthiness(&statement.test) == Some(true) {
            if let Some(constant) = fold_constant(&statement.test) {
                self.add_constant(constant)?;
            }
            if let Some(message) = &statement.msg {
                self.pre_register_expression_names(message)?;
                self.pre_register_expression_constants(message)?;
            }
            self.assembler
                .set_location(self.source_location(statement.test.range()));
            return self.emit(NOP, 0, 0);
        }
        let base_depth = self.control.depth;
        let end = self.assembler.label();
        let include_none_owner = !self.control.active_with_region_exclusions.is_empty();
        let retained_chained_comparison_exit = match statement.test.as_ref() {
            Expr::Compare(comparison) if comparison.ops.len() > 1 => Some(comparison.range),
            _ => None,
        };
        self.with_control_override(
            |control| &mut control.include_assert_none_condition_owner,
            include_none_owner,
            |compiler| {
                compiler.with_control_override(
                    |control| &mut control.retained_chained_comparison_exit,
                    retained_chained_comparison_exit,
                    |compiler| compiler.compile_jump_if(&statement.test, true, end),
                )
            },
        )?;
        self.assembler
            .set_location(self.source_location(statement.range));
        self.emit_common_constant(CommonConstant::ASSERTIONERROR)?;
        if let Some(message) = &statement.msg {
            self.compile_expression(message)?;
            self.assembler
                .set_location(self.source_location(statement.range));
            self.emit(CALL, 0, -1)?;
        }
        self.assembler
            .set_location(self.source_location(statement.test.range()));
        self.emit(RAISE_VARARGS, 1, -1)?;
        self.assembler.mark(end);
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_raise(
        &mut self,
        statement: &ruff_python_ast::StmtRaise,
    ) -> Result<(), CompileError> {
        let mut argument_count = 0;
        if let Some(exception) = &statement.exc {
            self.compile_expression(exception)?;
            argument_count += 1;
            if let Some(cause) = &statement.cause {
                self.compile_expression(cause)?;
                argument_count += 1;
            }
        }
        self.emit(RAISE_VARARGS, argument_count, -argument_count.cast_signed())
    }

    fn compile_try(&mut self, statement: &ruff_python_ast::StmtTry) -> Result<(), CompileError> {
        self.compile_try_inner(statement, TryCompileOptions::STANDARD)
    }

    fn compile_try_inner(
        &mut self,
        statement: &ruff_python_ast::StmtTry,
        options: TryCompileOptions,
    ) -> Result<(), CompileError> {
        if !statement.finalbody.is_empty() {
            return self.compile_try_finally(statement, options.terminal);
        }
        self.compile_try_except(statement, options)
    }

    fn compile_try_except(
        &mut self,
        statement: &ruff_python_ast::StmtTry,
        options: TryCompileOptions,
    ) -> Result<(), CompileError> {
        let TryCompileOptions {
            terminal,
            emit_statement_nop,
            exclude_terminal_body_not_taken,
            loop_tail,
        } = options;
        if statement.is_star {
            return self.compile_try_star_except(statement, terminal, emit_statement_nop);
        }
        if statement.handlers.is_empty() {
            return Err(unsupported("try statement without handlers"));
        }

        let base_depth = self.control.depth;
        let direct_return_handler_try_inside_sync_with =
            self.control.active_exception_region_exclusions.is_empty()
                && self.control.active_with_region_exclusions.len() == 1
                && self.control.active_with_exits.len() == 1
                && self.control.loops.is_empty()
                && self.control.active_finally_try_bodies == 0
                && self
                    .control
                    .active_with_exits
                    .last()
                    .is_some_and(|context| !context.is_async)
                && (is_direct_return_handler_try(statement)
                    || is_direct_return_raise_handler_try(statement));
        let nested_in_final_outer_try_statement =
            !self.control.active_exception_region_exclusions.is_empty()
                && self
                    .control
                    .direct_try_body_statements
                    .last()
                    .is_some_and(|outer| {
                        outer.start() <= statement.range.start()
                            && statement.range.end() <= outer.end()
                    })
                && !self.control.direct_try_body_has_else
                && self.control.active_finally_try_bodies == 0
                && !self.control.in_iterator_loop()
                && try_has_named_pair_exception_handler(statement);
        let try_start = self.assembler.label();
        let try_end = self.assembler.label();
        let handler_start = self.assembler.label();
        let common_cleanup = self.assembler.label();
        let end = self.assembler.label();
        let body_ends_with_break_loop = statement.orelse.is_empty()
            && matches!(
                statement.body.last(),
                Some(Stmt::For(statement))
                    if statement.orelse.is_empty()
                        && suite_contains_loop_break(&statement.body)
            );
        let handlers_terminate = statement.handlers.iter().all(|handler| {
            handler
                .as_except_handler()
                .is_some_and(|handler| suite_terminates(&handler.body))
        });
        let pass_only_handlers = statement.handlers.iter().all(|handler| {
            handler
                .as_except_handler()
                .is_some_and(|handler| matches!(handler.body.as_slice(), [Stmt::Pass(_)]))
        });
        // CPython folds every successful arm of a trailing nested conversion chain directly onto
        // the iterator backedge. Restrict that layout to recursive one-call try/except/else arms
        // so ordinary nested handlers keep their shared joins.
        let nested_loop_conversion_try = self.control.in_iterator_loop()
            && !self.control.current_statement_has_following
            && is_loop_conversion_try(statement)
            && (!self.control.active_exception_handlers.is_empty()
                || statement.handlers.iter().any(|handler| {
                    handler
                        .as_except_handler()
                        .is_some_and(|handler| matches!(handler.body.as_slice(), [Stmt::Try(_)]))
                }));
        let has_tuple_handler = statement.handlers.iter().any(|handler| {
            handler
                .as_except_handler()
                .is_some_and(|handler| matches!(handler.type_.as_deref(), Some(Expr::Tuple(_))))
        });
        let trailing_fallthrough_handler_try = self
            .control
            .active_exception_handlers
            .last()
            .is_some_and(|handler| handler.name.is_none())
            && self
                .control
                .is_direct_trailing_handler_statement(statement.range)
            && (try_has_fallthrough_body_and_handler(statement) || nested_loop_conversion_try);
        let propagates_protected_loop_tail_mode =
            try_propagates_protected_loop_tail_mode(statement);
        let nested_handler_tail_if_with_terminal_handlers = handlers_terminate
            && self.control.in_exception_handler()
            && statement.orelse.is_empty()
            && matches!(
                statement.body.as_slice(),
                [Stmt::If(tail)]
                    if tail.elif_else_clauses.is_empty()
                        && matches!(tail.test.as_ref(), Expr::UnaryOp(unary) if unary.op == UnaryOp::Not)
                        && matches!(tail.body.as_slice(), [Stmt::Expr(expression)] if matches!(expression.value.as_ref(), Expr::Call(_)))
            )
            && matches!(
                statement.handlers.as_slice(),
                [handler]
                    if handler.as_except_handler().is_some_and(|handler| {
                        matches!(handler.body.as_slice(), [Stmt::Return(statement)]
                            if matches!(statement.value.as_deref(), Some(Expr::BooleanLiteral(_))))
                    })
            );
        let normal_exit_suite = if statement.orelse.is_empty() {
            statement.body.as_slice()
        } else {
            statement.orelse.as_slice()
        };
        let relocated_try_else_exit = self
            .control
            .current_statement_followed_by_name_return
            .then(|| match statement.orelse.last() {
                Some(Stmt::If(tail)) if try_else_exit_moves_to_first_nested_join(tail) => {
                    Some(tail.range)
                }
                _ => None,
            })
            .flatten();
        let retained_try_body_if_exit = (suite_terminates(&statement.orelse)
            && matches!(
                statement.body.last(),
                Some(Stmt::If(statement))
                    if early_condition_truthiness(&statement.test).is_none()
                        && matches!(statement.body.as_slice(), [Stmt::Expr(_)])
                        && matches!(statement.elif_else_clauses.as_slice(), [clause]
                            if clause.test.is_none()
                                && matches!(clause.body.as_slice(), [Stmt::Expr(_)]))
            ))
        .then(|| statement.body.last().expect("matched final if").range());
        // A direct nested try with a terminating handler leaves the outer try body followed by
        // an empty normal-exit block after CPython reorders the cold handler blocks.
        let handler_contains_terminal_nested_try = statement.handlers.iter().any(|handler| {
            handler.as_except_handler().is_some_and(|handler| {
                handler.body.iter().any(|statement| {
                    let Stmt::Try(statement) = statement else {
                        return false;
                    };
                    statement.handlers.iter().any(|handler| {
                        handler
                            .as_except_handler()
                            .is_some_and(|handler| suite_terminates(&handler.body))
                    })
                })
            })
        });
        let handler_contains_nested_handler_cold_join = !handlers_terminate
            && statement.handlers.iter().any(|handler| {
                handler
                    .as_except_handler()
                    .is_some_and(|handler| suite_contains_nested_handler_cold_join(&handler.body))
            });
        let normal_exit_starts_in_empty_block = self.control.current_statement_has_following
            && (suite_ends_in_empty_try_exit_block(normal_exit_suite)
                || handler_contains_terminal_nested_try
                || handler_contains_nested_handler_cold_join);
        let copy_break_only_loop_exit = self.control.current_statement_has_following
            && suite_ends_in_nested_break_only_loop(normal_exit_suite);
        // CPython can leave an empty physical block after a final with or nested try. Borrowed-
        // load traversal stops there, so a following suite cannot reach this try join through
        // that normal-exit edge.
        if self.control.current_statement_has_following
            && ((statement.orelse.is_empty()
                && matches!(statement.body.last(), Some(Stmt::With(_))))
                || (matches!(normal_exit_suite.last(), Some(Stmt::Try(_)))
                    && suite_ends_in_empty_try_exit_block(normal_exit_suite)))
        {
            self.assembler.prevent_borrow_reachability(end);
        }

        if emit_statement_nop {
            let statement_nop_start = self.assembler.label();
            self.assembler.mark(statement_nop_start);
            self.assembler
                .set_location(self.source_location_including_trailing_semicolon(statement.range));
            self.emit(NOP, 0, 0)?;
            let statement_nop_end = self.assembler.label();
            self.assembler.mark(statement_nop_end);
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((statement_nop_start, statement_nop_end));
            }
            for exclusions in &mut self.control.active_with_region_exclusions {
                exclusions.push((statement_nop_start, statement_nop_end));
            }
            if self.control.generator_region_start.is_some() {
                self.control
                    .generator_region_exclusions
                    .push((statement_nop_start, statement_nop_end));
            }
        }
        // A first direct condition after a multi-edge branch join starts in a fresh ownerless
        // block. Separately, enough vacancies before an assignment-led iterator-loop try can
        // shift a later normalized marker before the protected region's setup boundary.
        let ownerless_direct_condition = self
            .control
            .branch_join_successor
            .is_some_and(|successor| successor == statement.range)
            .then(|| match statement.body.first() {
                Some(Stmt::If(statement)) => Some(statement.test.range()),
                _ => None,
            })
            .flatten();
        let track_shifted_marker_slots = matches!(statement.body.first(), Some(Stmt::Assign(_)));
        let try_marker_slots = (self.control.generator_region_start.is_none()
            && self.control.active_exception_region_exclusions.is_empty()
            && self.control.active_with_region_exclusions.is_empty()
            && self
                .control
                .active_comprehension_region_exclusions
                .is_empty()
            && self.control.in_iterator_loop()
            && (track_shifted_marker_slots || ownerless_direct_condition.is_some()))
        .then(|| TryMarkerSlotContext {
            ownerless_prefix_slots: if track_shifted_marker_slots {
                self.assembler.fallthrough_dynamic_exception_owner_slots()
            } else {
                0
            },
            protected_start_instruction: self.assembler.instruction_count(),
            ownerless_direct_condition,
            handler_starts_with_optimized_boolean_generator:
                try_handler_starts_with_optimized_boolean_generator(statement),
        });
        self.assembler.mark(try_start);
        self.control
            .active_exception_region_exclusions
            .push(Vec::new());
        self.control
            .active_tuple_handler_try_bodies
            .push(has_tuple_handler);
        self.control
            .active_pass_only_try_bodies
            .push(pass_only_handlers);
        self.control.active_try_marker_slots.push(try_marker_slots);
        let direct_continue =
            statement.orelse.is_empty() && matches!(statement.body.as_slice(), [Stmt::Continue(_)]);
        let direct_with_exit = self
            .control
            .active_with_exits
            .last()
            .is_some_and(|context| context.loop_depth == self.control.loops.len());
        let retained_assertion_exit = (self.control.current_statement_has_following
            || !statement.orelse.is_empty()
            || direct_with_exit)
            && matches!(
                statement.body.last(),
                Some(Stmt::Assert(assertion))
                    if condition_jump_target_count(&assertion.test, true) == 1
            );
        let unlocated_conditional_exit = statement.orelse.is_empty()
            && self.control.in_iterator_loop()
            && self.control.synthesized_loop_tail_predecessor == Some(statement.range)
            && matches!(
                statement.body.last(),
                Some(Stmt::If(nested))
                    if terminating_if_fallthrough_test(nested).is_some_and(|test| {
                        condition_jump_target_count(test, false) > 1
                    })
            );
        let include_try_none_condition_owner =
            self.control.include_assert_none_condition_owner || unlocated_conditional_exit;
        let ownerless_handler_assignment_branch = if statement.orelse.is_empty()
            && let [Stmt::If(branch)] = statement.body.as_slice()
            && is_assignment_pair_branch(branch)
            && matches!(statement.handlers.as_slice(), [handler]
            if handler.as_except_handler().is_some_and(|handler| {
                (matches!(handler.body.as_slice(), [Stmt::Assign(_)])
                    && assignment_pair_branch_has_prefix(branch))
                    || (self.control.in_iterator_loop()
                        && matches!(handler.body.as_slice(), [Stmt::Continue(_)]))
            })) {
            Some(branch.range)
        } else {
            None
        };
        let mut normal_return_emitted = false;
        let body_noop_location = if direct_continue {
            self.assembler.mark(try_end);
            self.compile_statement(&statement.body[0])?;
            None
        } else {
            let body_instruction_count = self.assembler.instruction_count();
            if statement.body.len() > 1
                && let Some(Stmt::Pass(pass)) = statement.body.first()
            {
                self.assembler
                    .set_location(self.source_location(pass.range));
                self.emit(NOP, 0, 0)?;
            }
            if exclude_terminal_body_not_taken
                && let Some((last @ Stmt::If(_), leading)) = statement.body.split_last()
            {
                self.compile_suite(leading)?;
                self.with_control_override(
                    |control| &mut control.exclude_terminal_if_not_taken,
                    true,
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.exclude_condition_not_taken_from_exception,
                            true,
                            |compiler| compiler.compile_statement(last),
                        )
                    },
                )?;
            } else if let Some((restart, split_short_circuit)) = loop_tail
                && let Some((Stmt::If(statement), leading)) = statement.body.split_last()
            {
                self.compile_suite(leading)?;
                self.compile_loop_tail_if(
                    statement,
                    restart,
                    split_short_circuit,
                    LoopTailMode {
                        non_final_branch: false,
                        try_body_normal_exit: true,
                        propagate_try_body_normal_exit: propagates_protected_loop_tail_mode,
                    },
                    leading,
                )?;
            } else if terminal
                && statement.orelse.is_empty()
                && statement
                    .body
                    .last()
                    .is_some_and(terminal_try_distributes_implicit_return)
            {
                let exception_region_depth = self.control.active_exception_region_exclusions.len();
                let ((), emitted) = self.with_try_body_context(statement, |compiler| {
                    compiler.with_control_override(
                        |control| &mut control.implicit_return_exception_exclusion_depth,
                        Some(exception_region_depth),
                        |compiler| {
                            compiler.with_fallthrough_tracking(|compiler| {
                                compiler.compile_suite_inner(&statement.body, true)
                            })
                        },
                    )
                })?;
                normal_return_emitted = emitted;
            } else if nested_handler_tail_if_with_terminal_handlers
                && let Some((last @ Stmt::If(_), leading)) = statement.body.split_last()
            {
                self.with_try_body_context(statement, |compiler| {
                    compiler.compile_suite(leading)?;
                    compiler.with_control_override(
                        |control| &mut control.exclude_condition_not_taken_from_exception,
                        true,
                        |compiler| {
                            compiler.with_control_override(
                                |control| {
                                    &mut control
                                        .exclude_condition_not_taken_from_all_exception_regions
                                },
                                true,
                                |compiler| compiler.compile_statement(last),
                            )
                        },
                    )
                })?;
            } else {
                self.with_try_body_context(statement, |compiler| {
                    compiler.with_control_override(
                        |control| &mut control.direct_return_handler_try_inside_sync_with,
                        direct_return_handler_try_inside_sync_with,
                        |compiler| {
                            compiler.with_control_override(
                                |control| &mut control.include_assert_none_condition_owner,
                                include_try_none_condition_owner,
                                |compiler| {
                                    compiler.with_control_override(
                                        |control| &mut control.retained_try_body_if_exit,
                                        retained_try_body_if_exit,
                                        |compiler| {
                                            compiler.with_control_override(
                                                |control| {
                                                    &mut control.ownerless_handler_assignment_branch
                                                },
                                                ownerless_handler_assignment_branch,
                                                |compiler| compiler.compile_suite(&statement.body),
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                })?;
            }
            let location = (self.assembler.instruction_count() == body_instruction_count)
                .then(|| {
                    statement
                        .body
                        .last()
                        .map(|statement| self.source_location(statement.range()))
                })
                .flatten();
            if retained_assertion_exit {
                self.assembler.mark_before_trailing_labels(try_end);
            } else {
                self.assembler.mark(try_end);
            }
            if let Some(Stmt::Assert(assertion)) = statement.body.last()
                && retained_assertion_exit
            {
                // CPython's POP_BLOCK follows the successful assertion edge. The jump targets
                // that instruction directly, so it survives conversion as a located NOP.
                let exit_test = unique_condition_jump_target(&assertion.test, true)
                    .expect("retained assertion exit has a unique successful edge");
                self.assembler
                    .set_location(self.source_location(exit_test.range()));
                self.emit(NOP, 0, 0)?;
                self.assembler.mark_last_as_converted_pop_block();
                self.assembler.preserve_last_nop();
                if early_condition_truthiness(&assertion.test) == Some(true) {
                    self.assembler.mark_last_borrow_fallthrough_barrier();
                }
            }
            if !statement.orelse.is_empty()
                && let Some(location) = location
            {
                // If `try` and its pass body share a line, CPython retains the enclosing
                // statement location on the synthetic body marker.
                let location = self
                    .assembler
                    .last_instruction_location()
                    .filter(|previous| previous.line == location.line)
                    .unwrap_or(location);
                self.assembler.set_location(location);
                // The no-op body does not inherit an enclosing finally's exception owner.
                let exception_policy = if self.control.prevent_try_exit_inlining
                    && self.control.active_exception_region_exclusions.len() > 1
                {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::default()
                };
                self.emit_with_exception_policy(NOP, 0, 0, exception_policy)?;
            }
            location
        };
        let _try_marker_slots = self
            .control
            .active_try_marker_slots
            .pop()
            .expect("active try region has marker slot context");
        let _pass_only_try_body = self
            .control
            .active_pass_only_try_bodies
            .pop()
            .expect("active try region has pass-only handler context");
        let _tuple_handler_try_body = self
            .control
            .active_tuple_handler_try_bodies
            .pop()
            .expect("active try region has tuple-handler context");
        let try_exclusions = self
            .control
            .active_exception_region_exclusions
            .pop()
            .expect("active try region has exclusion collector");
        let orelse_instruction_count = self.assembler.instruction_count();
        self.with_control_override(
            |control| &mut control.relocated_try_else_exit,
            relocated_try_else_exit,
            |compiler| {
                if terminal
                    && statement
                        .orelse
                        .last()
                        .is_some_and(terminal_try_distributes_implicit_return)
                {
                    let ((), emitted) = compiler.with_fallthrough_tracking(|compiler| {
                        compiler.compile_suite_inner(&statement.orelse, true)
                    })?;
                    normal_return_emitted = emitted;
                    Ok(())
                } else {
                    compiler.compile_suite(&statement.orelse)
                }
            },
        )?;
        let orelse_noop_location = (self.assembler.instruction_count() == orelse_instruction_count)
            .then(|| {
                statement
                    .orelse
                    .last()
                    .map(|statement| self.source_location(statement.range()))
            })
            .flatten();
        if direct_continue {
        } else if terminal {
            if let Some(location) = orelse_noop_location {
                self.assembler.set_location(location);
            } else if matches!(statement.body.last(), Some(Stmt::Try(_))) {
                self.assembler.set_location(SourceLocation::NONE);
            } else if statement.orelse.is_empty()
                && let Some(location) = body_noop_location
            {
                self.assembler.set_location(location);
            } else if statement.orelse.is_empty()
                && let Some(range) = suite_assert_success_fallthrough_range(&statement.body)
            {
                self.assembler.set_location(self.source_location(range));
            } else if let Some(location) = self.assembler.last_instruction_location() {
                self.assembler.set_location(location);
            }
            if !normal_return_emitted {
                self.emit_deferred_implicit_return()?;
            }
        } else {
            let following_try_threads_conditional_exit =
                self.control.current_statement_followed_by_try
                    && statement.orelse.is_empty()
                    && matches!(statement.body.last(), Some(Stmt::If(nested))
                    if terminating_if_fallthrough_test(nested).is_some()
                        && matches!(nested.body.as_slice(), [Stmt::If(_)]))
                    && direct_try_body_exit_location(&statement.body).is_none();
            let direct_exit_location = (statement.orelse.is_empty() && !unlocated_conditional_exit)
                .then(|| direct_try_body_exit_location(&statement.body))
                .flatten();
            let unlocated_direct_exit_before_break =
                self.control.current_statement_followed_by_break
                    && statement.orelse.is_empty()
                    && matches!(
                        statement.body.last(),
                        Some(Stmt::If(branch))
                            if terminating_if_fallthrough_test(branch).is_some()
                                && condition_jump_target_count(&branch.test, false) > 1
                    );
            if unlocated_direct_exit_before_break {
                // A following break remains a separate CFG exit, so the converted `POP_BLOCK`
                // does not inherit the terminal branch's source range.
                self.assembler.set_location(SourceLocation::NONE);
            } else if let Some(location) = orelse_noop_location {
                self.assembler.set_location(location);
            } else if let Some(location) = direct_exit_location {
                self.assembler
                    .set_location(self.source_location(location.range()));
            } else if unlocated_conditional_exit {
                self.assembler.set_location(SourceLocation::NONE);
            } else if following_try_threads_conditional_exit {
                // The following try's statement marker is the normal-edge join. CPython drops
                // the preceding try's synthetic condition-exit location while threading that
                // edge, so it must not survive as a distinct source NOP.
                self.assembler.set_location(SourceLocation::NONE);
            } else if self.control.current_statement_followed_by_literal_return
                && let Some(Stmt::If(nested)) = statement.orelse.last()
                && nested.elif_else_clauses.is_empty()
                && matches!(nested.body.as_slice(), [Stmt::Return(_)])
                && let Some(test) = terminating_if_fallthrough_test(nested)
            {
                // Exit duplication copies the following literal return onto this normal edge;
                // CPython then retains the edge as a NOP carrying the condition's exit range.
                self.assembler
                    .set_location(self.source_location(synthetic_condition_exit_range(test)));
            } else if statement.orelse.is_empty()
                && let Some(Stmt::If(nested)) = statement.body.last()
                && let Some(test) = terminating_if_fallthrough_test(nested)
            {
                self.assembler
                    .set_location(self.source_location(condition_fallthrough_range(test)));
            } else if statement.orelse.is_empty()
                && let Some(range) = suite_unary_not_fallthrough_range(&statement.body)
            {
                self.assembler.set_location(self.source_location(range));
            } else if statement.orelse.is_empty()
                && let Some(location) = body_noop_location
            {
                self.assembler.set_location(location);
            } else if pass_only_handlers
                && matches!(self.unit.scope, Scope::Function { .. })
                && !statement.orelse.is_empty()
                && suite_retained_try_exit_has_normal_fallthrough(&statement.orelse)
                && let Some(Stmt::If(nested)) = statement.orelse.last()
                && let Some(test) = terminating_if_fallthrough_test(nested)
                && matches!(test, Expr::Name(_))
            {
                self.assembler
                    .set_location(self.source_location(condition_fallthrough_range(test)));
            } else if let Some(location) = self.assembler.last_instruction_location() {
                self.assembler.set_location(location);
            }
            if nested_handler_tail_if_with_terminal_handlers {
                self.assembler.set_location(SourceLocation::NONE);
            }
            let body_ends_with_overridden_return = self.control.active_overriding_finally_returns
                > 0
                && matches!(statement.body.last(), Some(Stmt::Return(_)));
            let exit_exclusion_start =
                if !self.control.active_exception_region_exclusions.is_empty()
                    && (body_ends_with_overridden_return
                        || (body_noop_location.is_some()
                            && (self.control.active_overriding_finally_returns > 0
                                || (pass_only_handlers
                                    && !self.control.prevent_try_exit_inlining))))
                {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    Some(start)
                } else {
                    None
                };
            // A pass-only nested try leaves an ownerless exit NOP unless the enclosing region
            // is the synthetic protected body of try/finally.
            let exclude_nested_normal_exit = nested_in_final_outer_try_statement
                && self.control.active_pass_finally_locations.is_empty();
            let trailing_handler_exit_exclusion_start =
                trailing_fallthrough_handler_try.then(|| {
                    let start = self.assembler.label();
                    self.assembler.mark(start);
                    start
                });
            let exception_policy = if exclude_nested_normal_exit
                || (body_noop_location.is_some()
                    && statement.orelse.is_empty()
                    && !self.control.active_exception_region_exclusions.is_empty()
                    && self.control.active_finally_try_bodies == 0)
            {
                ExceptionPolicy::EXCLUDE_ALWAYS
            } else {
                ExceptionPolicy::default()
            };
            self.emit_jump_forward_with_exception_policy(JUMP_FORWARD, end, 0, exception_policy)?;
            if self.control.copied_split_return_try == Some(statement.range) {
                self.assembler.allow_last_split_return_copy();
            }
            if following_try_threads_conditional_exit {
                self.assembler.prevent_borrow_reachability(end);
                self.assembler.preserve_last_no_location();
            }
            if let Some(start) = trailing_handler_exit_exclusion_start {
                let exclusion_end = self.assembler.label();
                self.assembler.mark(exclusion_end);
                self.control
                    .active_exception_region_exclusions
                    .last_mut()
                    .expect("trailing handler try exit requires an outer handler")
                    .push((start, exclusion_end));
            }
            if copy_break_only_loop_exit {
                // Copy the following try exit directly onto the only reachable loop break.
                self.assembler.prepare_last_no_location_block_for_inlining();
            }
            if unlocated_direct_exit_before_break {
                self.assembler.preserve_last_no_location();
            }
            if unlocated_conditional_exit {
                self.assembler.prevent_borrow_reachability(end);
                self.assembler.preserve_last_no_location();
            }
            if nested_handler_tail_if_with_terminal_handlers {
                self.assembler.preserve_last_no_location();
            }
            if normal_exit_starts_in_empty_block {
                self.assembler.mark_last_borrow_empty_block();
            }
            if handlers_terminate && !self.control.current_statement_followed_by_name_return {
                // CPython removes this normal-exit edge before borrowed-load traversal. Keep
                // the barrier on the source edge so a shared target can still be reached from
                // an enclosing branch or loop. A direct following name return is copied onto
                // the normal edge first, so its load remains reachable and borrowed.
                self.assembler.mark_last_borrow_fallthrough_barrier();
            }
            if !statement.orelse.is_empty()
                && suite_ends_in_retained_try_exit_join(&statement.orelse)
                && relocated_try_else_exit.is_none()
            {
                self.assembler
                    .preserve_last_redundant_jump_nop_after_threading();
                if pass_only_handlers
                    && matches!(self.unit.scope, Scope::Function { .. })
                    && suite_retained_try_exit_has_normal_fallthrough(&statement.orelse)
                {
                    self.assembler.mark_last_retained_nop_borrow_fallthrough();
                }
            }
            if direct_exit_location.is_some() {
                self.assembler.preserve_last_direct_inlined_jump_nop();
            }
            if statement.orelse.is_empty() && retained_assertion_exit {
                self.assembler.prevent_last_jump_threading_target();
            }
            if statement.orelse.is_empty()
                && suite_ends_in_retained_try_body_exit_join(&statement.body)
            {
                self.assembler
                    .preserve_last_redundant_jump_nop_after_threading();
            }
            if body_ends_with_break_loop {
                // CPython's `POP_BLOCK` initially prevents the loop break from threading
                // through this normal try exit. Once `POP_BLOCK` disappears, the exit jump
                // is retained as the loop cleanup's source-position NOP.
                self.assembler
                    .preserve_last_redundant_jump_nop_after_threading();
            }
            if statement.orelse.is_empty() && body_noop_location.is_some() {
                // Keep the pass line when direct exit duplication replaces this jump.
                self.assembler.preserve_last_direct_inlined_jump_nop();
            }
            if let Some(exit_exclusion_start) = exit_exclusion_start {
                let exit_exclusion_end = self.assembler.label();
                self.assembler.mark(exit_exclusion_end);
                for exclusions in &mut self.control.active_exception_region_exclusions {
                    exclusions.push((exit_exclusion_start, exit_exclusion_end));
                }
            }
            if self.control.prevent_try_exit_inlining {
                self.assembler.prevent_last_jump_inlining();
            }
        }
        self.add_exception_regions_with_exclusions(
            try_start,
            try_end,
            &try_exclusions,
            handler_start,
            base_depth.cast_unsigned(),
            false,
        );

        self.assembler.mark(handler_start);
        self.set_depth(base_depth + 1);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(PUSH_EXC_INFO, 0, 1)?;
        let mut dispatch_start = handler_start;
        let mut final_handler_location = SourceLocation::NONE;

        for (handler_index, handler) in statement.handlers.iter().enumerate() {
            let handler = handler.as_except_handler().ok_or_else(|| {
                CompileError::Internal("invalid exception handler node".to_string())
            })?;
            let next_handler = self.assembler.label();
            let handler_location = self.source_location_including_trailing_semicolon(handler.range);
            final_handler_location = handler_location;
            self.set_depth(base_depth + 2);
            let mut not_taken_exclusion = None;
            if let Some(exception_type) = &handler.type_ {
                let mut references = ReferenceCollector::default();
                references.visit_expr(exception_type);
                let reference_names = references
                    .references
                    .into_iter()
                    .map(|name| self.required_name_id(&name))
                    .collect::<Vec<_>>();
                let newly_owned: Vec<_> = reference_names
                    .into_iter()
                    .filter(|name| self.symbols.owned_load_locals.insert(*name))
                    .collect();
                let bool_marker_state =
                    matches!(exception_type.as_ref(), Expr::BoolOp(_)).then(|| {
                        ExceptionTypeBoolMarkerState::new(
                            handler_index == 0,
                            self.assembler.current_block_token(),
                        )
                    });
                self.with_control_override(
                    |control| &mut control.exception_type_bool_marker_state,
                    bool_marker_state,
                    |compiler| compiler.compile_expression(exception_type),
                )?;
                for name in newly_owned {
                    self.symbols.owned_load_locals.remove(name);
                }
                let exclude_not_taken_from_dispatch = handler_index > 0
                    || matches!(exception_type.as_ref(), Expr::BoolOp(_) | Expr::If(_));
                // CPython normalizes these jumps after exception targets are labeled. A later
                // handler or conditional exception type appends the marker into a fresh CFG
                // slot, so it cannot inherit an outer generator, with, or finally owner.
                let ownerless_not_taken = handler_index > 0
                    || matches!(exception_type.as_ref(), Expr::BoolOp(_) | Expr::If(_));
                self.assembler.set_location(handler_location);
                self.emit(CHECK_EXC_MATCH, 0, 0)?;
                self.emit_jump_forward(POP_JUMP_IF_FALSE, next_handler, -1)?;
                let exclusion_start = self.assembler.label();
                self.assembler.mark(exclusion_start);
                self.emit_with_exception_policy(
                    NOT_TAKEN,
                    0,
                    0,
                    if ownerless_not_taken {
                        ExceptionPolicy::EXCLUDE_ALWAYS
                    } else {
                        ExceptionPolicy::default()
                    },
                )?;
                let exclusion_end = self.assembler.label();
                self.assembler.mark(exclusion_end);
                if exclude_not_taken_from_dispatch {
                    for exclusions in &mut self.control.active_exception_region_exclusions {
                        exclusions.push((exclusion_start, exclusion_end));
                    }
                    not_taken_exclusion = Some((exclusion_start, exclusion_end));
                }
            } else if handler_index + 1 != statement.handlers.len() {
                return Err(CompileError::Internal(
                    "bare except handler must be last".to_string(),
                ));
            }

            self.assembler.set_location(handler_location);
            if let Some(name) = &handler.name {
                self.store_name(name.as_str())?;
            } else {
                self.emit(POP_TOP, 0, -1)?;
            }

            let body_start = self.assembler.label();
            let body_end = self.assembler.label();
            let name_cleanup = self.assembler.label();
            self.assembler.mark(body_start);
            if matches!(handler.body.as_slice(), [Stmt::Continue(_)]) {
                self.assembler.mark(body_end);
                self.assembler.add_exception_region(
                    dispatch_start,
                    body_end,
                    common_cleanup,
                    (base_depth + 1).cast_unsigned(),
                    true,
                );
                self.assembler
                    .set_location(self.source_location(handler.body[0].range()));
                self.emit(POP_EXCEPT, 0, -1)?;
                if let Some(name) = &handler.name {
                    let none = self.add_constant(Constant::None)?;
                    self.emit(LOAD_CONST, none, 1)?;
                    self.store_name(name.as_str())?;
                    self.delete_name(name.as_str())?;
                }
                let continue_label = self
                    .control
                    .loops
                    .last()
                    .map(|context| context.continue_label)
                    .ok_or_else(|| CompileError::Internal("continue outside loop".to_string()))?;
                self.emit_jump_backward(JUMP_BACKWARD, continue_label, 0)?;
                if let Some(name) = &handler.name {
                    self.assembler.add_exception_region(
                        body_start,
                        body_end,
                        name_cleanup,
                        (base_depth + 1).cast_unsigned(),
                        true,
                    );
                    self.assembler.mark(name_cleanup);
                    self.set_depth(base_depth + 3);
                    self.assembler.set_location(SourceLocation::NONE);
                    let none = self.add_constant(Constant::None)?;
                    self.emit(LOAD_CONST, none, 1)?;
                    self.store_name(name.as_str())?;
                    self.delete_name(name.as_str())?;
                    self.emit_reraise(ReraiseStackState::HandlerCleanup)?;
                }
                self.assembler.mark(next_handler);
                dispatch_start = if handler.name.is_some() {
                    name_cleanup
                } else {
                    next_handler
                };
                continue;
            }
            if handler.name.is_none() && matches!(handler.body.as_slice(), [Stmt::Break(_)]) {
                self.assembler.mark(body_end);
                self.assembler.add_exception_region(
                    dispatch_start,
                    body_end,
                    common_cleanup,
                    (base_depth + 1).cast_unsigned(),
                    true,
                );
                self.assembler
                    .set_location(self.source_location(handler.body[0].range()));
                self.emit(POP_EXCEPT, 0, -1)?;
                let unreachable_depth = self.control.depth;
                let context = self
                    .control
                    .loops
                    .last()
                    .copied()
                    .ok_or_else(|| CompileError::Internal("break outside loop".to_string()))?;
                match context.iterator_cleanup {
                    IteratorCleanup::None => {}
                    IteratorCleanup::Sync | IteratorCleanup::Async => {
                        self.emit(POP_TOP, 0, -1)?;
                    }
                }
                if context.break_policy.returns() {
                    self.emit_implicit_return()?;
                } else {
                    self.emit_jump_forward(JUMP_BACKWARD, context.break_label, 0)?;
                    self.assembler.mark_last_jump_as_authored_break();
                }
                self.set_depth(unreachable_depth);
                self.assembler.mark(next_handler);
                dispatch_start = next_handler;
                continue;
            }
            if handler.name.is_none()
                && !self.control.active_pass_finally_locations.is_empty()
                && let [Stmt::Return(return_statement)] = handler.body.as_slice()
                && return_statement
                    .value
                    .as_deref()
                    .is_none_or(|value| matches!(value, Expr::NoneLiteral(_)))
            {
                self.assembler.mark(body_end);
                self.assembler.add_exception_region(
                    dispatch_start,
                    body_end,
                    common_cleanup,
                    (base_depth + 1).cast_unsigned(),
                    true,
                );
                let return_range = return_statement
                    .value
                    .as_deref()
                    .map_or(return_statement.range, Ranged::range);
                self.assembler
                    .set_location(self.source_location(return_range));
                self.emit(POP_EXCEPT, 0, -1)?;
                let unwind_start = self.assembler.label();
                self.assembler.mark(unwind_start);
                self.emit_pass_finally_nops()?;
                self.assembler.set_location(SourceLocation::NONE);
                let none = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, none, 1)?;
                self.emit(RETURN_VALUE, 0, -1)?;
                let unwind_end = self.assembler.label();
                self.assembler.mark(unwind_end);
                for exclusions in &mut self.control.active_exception_region_exclusions {
                    exclusions.push((unwind_start, unwind_end));
                }
                self.assembler.mark(next_handler);
                dispatch_start = next_handler;
                continue;
            }
            let handler_body_start = self.assembler.instruction_count();
            let newly_owned_handler_locals = {
                let initialized_locals = &self.symbols.initialized_locals;
                let owned_load_locals = &mut self.symbols.owned_load_locals;
                initialized_locals
                    .iter_unordered()
                    .filter(|name| owned_load_locals.insert(*name))
                    .collect::<Vec<_>>()
            };
            let handler_name = handler
                .name
                .as_ref()
                .map(|name| self.required_name_id(name.as_str()));
            let previously_owned =
                handler_name.is_some_and(|name| !self.symbols.owned_load_locals.insert(name));
            let terminal_handler_if = if terminal
                && let Some(name) = &handler.name
                && let [Stmt::If(statement)] = handler.body.as_slice()
                && terminal_exception_handler_if_supported(statement)
            {
                Some((statement, name.as_str()))
            } else {
                None
            };
            // CPython terminal-compiles a final nested try while the outer handler fblock is
            // still active, so each implicit return carries the outer handler cleanup.
            let terminal_handler_try = terminal
                && terminal_handler_if.is_none()
                && matches!(handler.body.last(), Some(Stmt::Try(_)));
            let mut terminal_handler_try_return = false;
            let (terminal_handler_exclusions, mut handler_region_exclusions) =
                if let Some((statement, name)) = terminal_handler_if {
                    (
                        Some(self.compile_terminal_exception_handler_if(
                            statement,
                            name,
                            base_depth + 1,
                        )?),
                        Vec::new(),
                    )
                } else {
                    let strict_owned_loads = matches!(
                        handler.body.last(),
                        Some(Stmt::Break(_) | Stmt::Continue(_))
                    );
                    let context = ExceptionHandlerContext {
                        name: handler_name,
                        first: handler_index == 0,
                        loop_depth: self.control.loops.len(),
                        unwinds_before_sync_with: direct_return_handler_try_inside_sync_with,
                        leading_statement: handler.body.first().map(Ranged::range),
                        trailing_statement: handler.body.last().map(Ranged::range),
                        common_cleanup,
                        ownerless_optimized_boolean_generator_test:
                            handler_ownerless_optimized_boolean_generator_test(&handler.body),
                        protect_outer_return_pop: nested_handler_tail_if_with_terminal_handlers
                            || (handler_name.is_none()
                                && (suite_terminates(&statement.body)
                                    || self.control.current_statement_has_following)
                                && matches!(handler.body.as_slice(), [Stmt::Return(_)])
                                && matches!(
                                    self.control.active_exception_handlers.as_slice(),
                                    [outer] if outer.name.is_none()
                                )),
                        common_cleanup_depth: (base_depth + 1).cast_unsigned(),
                    };
                    let ((), exclusions) = self.with_exception_handler(context, |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.unwind_exception_handlers_for_implicit_return,
                            terminal_handler_try,
                            |compiler| {
                                if terminal_handler_try {
                                    let ((), emitted_fallthrough) = compiler
                                        .with_fallthrough_tracking(|compiler| {
                                            compiler.compile_with_strict_owned_loads(
                                                strict_owned_loads,
                                                |compiler| {
                                                    compiler
                                                        .compile_suite_inner(&handler.body, true)
                                                },
                                            )
                                        })?;
                                    terminal_handler_try_return = emitted_fallthrough;
                                    Ok(())
                                } else {
                                    compiler.compile_with_strict_owned_loads(
                                        strict_owned_loads,
                                        |compiler| compiler.compile_suite(&handler.body),
                                    )
                                }
                            },
                        )
                    })?;
                    (None, exclusions)
                };
            if let Some(exclusions) = &terminal_handler_exclusions {
                handler_region_exclusions.extend_from_slice(exclusions);
            }
            if let Some(name) = handler_name
                && !previously_owned
            {
                self.symbols.owned_load_locals.remove(name);
            }
            for name in newly_owned_handler_locals {
                self.symbols.owned_load_locals.remove(name);
            }
            let handler_body_has_instructions =
                self.assembler.instruction_count() > handler_body_start;
            // CPython allocates the normal exit before an explicit `else`, so collecting this
            // cold class-body handler puts the named cleanup before the bare-raise arm.
            let handler_ends_with_preordered_named_cleanup = matches!(
                handler.body.last(),
                Some(Stmt::If(tail))
                    if handler.name.is_some()
                        && matches!(self.unit.scope, Scope::Class { .. })
                        && handler.body.len() == 1
                        && !suite_terminates(&tail.body)
                        && tail.elif_else_clauses.last().is_some_and(|clause| {
                            clause.test.is_none()
                                && matches!(
                                    clause.body.as_slice(),
                                    [Stmt::Raise(statement)] if statement.exc.is_none()
                                )
                        })
            );
            let direct_nested_loop_handler_backedge = nested_loop_conversion_try;
            let copy_nested_loop_handler_backedge = direct_nested_loop_handler_backedge
                && matches!(
                    handler.body.as_slice(),
                    [Stmt::Try(nested)] if is_loop_conversion_try(nested)
                );
            let handler_ends_with_inlineable_cleanup = handler_ends_with_preordered_named_cleanup
                || handler.body.last().is_some_and(|tail| match tail {
                    Stmt::Try(nested) => {
                        handler.name.is_some()
                            // Fallback imports feed the outer handler cleanup from the successful
                            // import and from the nested handler. CPython leaves that cleanup
                            // unlocated so its exit-copy pass can inline one copy onto each edge.
                            || (handler.body.len() == 1
                                && suite_is_import_assignment(&statement.body)
                                && nested.orelse.is_empty()
                                && nested.finalbody.is_empty()
                                && suite_is_import_assignment(&nested.body)
                                && !suite_terminates(&nested.body)
                                && nested.handlers.iter().any(|nested_handler| {
                                    nested_handler
                                        .as_except_handler()
                                        .is_some_and(|nested_handler| {
                                            nested_handler.name.is_some()
                                                && !suite_terminates(&nested_handler.body)
                                                && matches!(
                                                    nested_handler.body.last(),
                                                    Some(Stmt::Assign(_))
                                                )
                                        })
                                }))
                            // A trailing nested try with fallthrough from both its body and a
                            // handler feeds the outer bare-handler cleanup from multiple edges.
                            // Leave that cleanup unlocated so CPython's repeated exit-copy pass
                            // can inline it onto each nested normal exit.
                            || try_has_fallthrough_body_and_handler(nested)
                            || (statement.body.len() == 1
                                && !suite_terminates(&statement.body)
                                && nested.finalbody.is_empty()
                                && nested.orelse.is_empty()
                                && nested.handlers.iter().any(|nested_handler| {
                                    let Some(nested_handler) = nested_handler.as_except_handler()
                                    else {
                                        return false;
                                    };
                                    match nested_handler.body.last() {
                                        Some(Stmt::Try(_)) => true,
                                        Some(Stmt::Raise(statement)) => matches!(
                                            statement.cause.as_deref(),
                                            Some(Expr::NoneLiteral(_))
                                        ),
                                        _ => false,
                                    }
                                }))
                    }
                    Stmt::With(statement) => {
                        handler.name.is_some() && !suite_terminates(&statement.body)
                    }
                    Stmt::If(tail) => {
                        (handler.name.is_none()
                            && tail
                                .elif_else_clauses
                                .last()
                                .is_some_and(|clause| clause.test.is_none())
                            && if_normal_exit_count(tail) >= 2
                            && tail.body.len() == 1
                            && tail
                                .elif_else_clauses
                                .iter()
                                .all(|clause| clause.body.len() == 1)
                            && (handler.body.len() > 1 || suite_terminates(&tail.body)))
                            // A terminating alternative leaves the nested try's normal exit as
                            // the sole path to a named-handler cleanup. CPython copies the
                            // cleanup onto that exit before placing the cold terminating arm.
                            || (handler.name.is_some()
                                && tail.elif_else_clauses.last().is_some_and(|clause| {
                                    clause.test.is_none() && suite_terminates(&clause.body)
                                })
                                && matches!(
                                    tail.body.last(),
                                    Some(Stmt::Try(nested))
                                        if !nested.orelse.is_empty()
                                            && nested.handlers.iter().all(|handler| {
                                                handler.as_except_handler().is_some_and(|handler| {
                                                    suite_terminates(&handler.body)
                                                })
                                            })
                                ))
                    }
                    _ => false,
                });
            // CPython keeps this simple conditional handler cleanup source-less before copying
            // its exit, rather than inheriting the call location from the taken branch.
            let handler_ends_with_unlocated_if_cleanup =
                self.control.current_statement_has_following
                    && matches!(
                        handler.body.last(),
                        Some(Stmt::If(tail))
                            if handler.body.len() == 1
                                && tail.elif_else_clauses.is_empty()
                                && (matches!(tail.test.as_ref(), Expr::Compare(_))
                                    || early_condition_truthiness(&tail.test).is_none())
                                && matches!(
                                    tail.body.as_slice(),
                                    [Stmt::Expr(expression)]
                                        if matches!(expression.value.as_ref(), Expr::Call(_))
                                )
                    );
            let handler_ends_with_preserved_unlocated_cleanup = matches!(
                handler.body.last(),
                Some(Stmt::If(tail))
                    if handler.name.is_none()
                        && handler.body.len() > 1
                        && !self.control.active_with_exits.is_empty()
                        && tail.elif_else_clauses.is_empty()
                        && matches!(
                            tail.body.as_slice(),
                            [Stmt::Expr(expression)]
                                if matches!(expression.value.as_ref(), Expr::Call(_))
                        )
            );
            let handler_ends_with_unlocated_bare_raise_branch_cleanup = handler.name.is_none()
                && handler.body.len() > 1
                && matches!(
                    handler.body.last(),
                    Some(Stmt::If(tail))
                        if tail.elif_else_clauses.is_empty()
                            && matches!(
                                tail.test.as_ref(),
                                Expr::UnaryOp(unary)
                                    if unary.op == UnaryOp::Not
                                        && matches!(
                                            unary.operand.as_ref(),
                                            Expr::BoolOp(boolean) if boolean.op == BoolOp::Or
                                        )
                            )
                            && matches!(
                                tail.body.as_slice(),
                                [Stmt::Raise(statement)]
                                    if statement.exc.is_none() && statement.cause.is_none()
                            )
                );
            let handler_ends_with_unlocated_cleanup = copy_nested_loop_handler_backedge
                || handler_ends_with_inlineable_cleanup
                || handler_ends_with_unlocated_if_cleanup
                || handler_ends_with_preserved_unlocated_cleanup;
            let handler_ends_with_nonterminal_with = matches!(
                handler.body.last(),
                Some(Stmt::With(statement)) if !suite_terminates(&statement.body)
            );
            let inlineable_bare_handler_if_with_multiline_join =
                handler_ends_with_inlineable_cleanup
                    && handler.name.is_none()
                    && handler.body.len() > 1
                    && matches!(
                        handler.body.last(),
                        Some(Stmt::If(tail))
                            if matches!(
                                tail.body.as_slice(),
                                [Stmt::Assign(assignment)]
                                    if matches!(assignment.value.as_ref(), Expr::BoolOp(_))
                                        && {
                                            let location = self
                                                .source_location(assignment.value.range());
                                            location.line != location.end_line
                                        }
                            )
                    );
            self.assembler.mark(body_end);
            if handler.name.is_some() {
                self.add_exception_regions_with_exclusions(
                    body_start,
                    body_end,
                    &handler_region_exclusions,
                    name_cleanup,
                    (base_depth + 1).cast_unsigned(),
                    true,
                );
            }
            let mut dispatch_exclusions = Vec::new();
            if let Some(exclusion) = not_taken_exclusion {
                dispatch_exclusions.push(exclusion);
            }
            dispatch_exclusions.extend_from_slice(&handler_region_exclusions);
            self.add_exception_regions_with_exclusions(
                dispatch_start,
                body_end,
                &dispatch_exclusions,
                common_cleanup,
                (base_depth + 1).cast_unsigned(),
                true,
            );
            if terminal_handler_exclusions.is_none() && !terminal_handler_try_return {
                // CPython propagates a handler's sole condition-fallthrough edge onto the
                // normal cleanup block, even when code follows the enclosing try statement.
                if handler_ends_with_unlocated_bare_raise_branch_cleanup {
                    self.assembler.set_location(SourceLocation::NONE);
                } else if let Some(Stmt::If(statement)) = handler.body.last()
                    && let Some(test) = terminating_if_fallthrough_test(statement)
                    && let Some(test) = unique_condition_jump_target(test, false)
                {
                    self.assembler
                        .set_location(self.source_location(test.range()));
                } else if let Some(range) = suite_unary_not_fallthrough_range(&handler.body) {
                    self.assembler.set_location(self.source_location(range));
                } else if !handler_body_has_instructions
                    && let Some(statement) = handler.body.last()
                {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                // CPython leaves cleanup after these direct trailing compound joins unlocated so
                // every incoming edge can inline its own copy with the predecessor's line.
                } else if handler_ends_with_unlocated_cleanup {
                    self.assembler.set_location(SourceLocation::NONE);
                } else if let Some(location) = self.assembler.last_instruction_location() {
                    self.assembler.set_location(location);
                }
                self.emit(POP_EXCEPT, 0, -1)?;
                if inlineable_bare_handler_if_with_multiline_join {
                    // CPython first keeps this cleanup unlocated so exits from both arms can be
                    // copied. A multiline boolean assignment gives its joined store a distinct
                    // source location, which the optimizer propagates onto the cleanup copy even
                    // though that store block has multiple predecessors.
                    self.assembler
                        .make_last_no_location_copy_inherit_shared_predecessor();
                }
                if handler_ends_with_preserved_unlocated_cleanup
                    || handler_ends_with_unlocated_bare_raise_branch_cleanup
                    || handler_ends_with_unlocated_if_cleanup
                {
                    self.assembler.preserve_last_no_location();
                }
                if let Some(name) = &handler.name {
                    let none = self.add_constant(Constant::None)?;
                    self.emit(LOAD_CONST, none, 1)?;
                    self.store_name(name.as_str())?;
                    self.delete_name(name.as_str())?;
                }
                // Each copied arm leaves the current handler, then unwinds enclosing handlers at
                // the same loop depth before taking the backedge. Exception exclusions begin at
                // each successive POP_EXCEPT so the earlier pops retain their outer owners.
                if direct_nested_loop_handler_backedge {
                    let context = self.control.loops.last().copied().ok_or_else(|| {
                        CompileError::Internal(
                            "nested conversion handler outside iterator loop".to_string(),
                        )
                    })?;
                    let loop_depth = self.control.loops.len();
                    let mut unwind_exclusions = Vec::new();
                    for index in (0..self.control.active_exception_handlers.len()).rev() {
                        let (handler_loop_depth, name) = {
                            let handler = &self.control.active_exception_handlers[index];
                            (handler.loop_depth, handler.name)
                        };
                        if handler_loop_depth < loop_depth {
                            break;
                        }
                        let exclusion_start = self.assembler.label();
                        self.assembler.mark(exclusion_start);
                        unwind_exclusions.push((index, exclusion_start));
                        self.emit(POP_EXCEPT, 0, -1)?;
                        if let Some(name) = name {
                            let none = self.add_constant(Constant::None)?;
                            self.emit(LOAD_CONST, none, 1)?;
                            self.store_name_id(name)?;
                            self.delete_name_id(name)?;
                        }
                    }
                    self.emit_jump_backward(JUMP_BACKWARD, context.continue_label, 0)?;
                    if copy_nested_loop_handler_backedge {
                        self.assembler.prepare_last_no_location_block_for_inlining();
                    }
                    let unwind_end = self.assembler.label();
                    self.assembler.mark(unwind_end);
                    for (index, exclusion_start) in unwind_exclusions {
                        self.control.active_exception_region_exclusions[index]
                            .push((exclusion_start, unwind_end));
                    }
                } else if terminal {
                    self.emit_deferred_implicit_return()?;
                } else {
                    let protected_loop_value_return = self.control.in_iterator_loop()
                        && self.control.active_finally_try_bodies > 0
                        && matches!(
                            statement.body.last(),
                            Some(Stmt::Return(statement))
                                if matches!(statement.value.as_deref(), Some(Expr::Name(_)))
                        );
                    let following_expression_exit_keeps_owner =
                        self.control.current_statement_has_following
                            && matches!(statement.body.as_slice(), [Stmt::Expr(_)]);

                    // A pass-only finally keeps these extended handler exits in its protected
                    // region; executable finally bodies use CPython's stale-owner exclusion.
                    let trailing_handler_exit_exclusion_start = trailing_fallthrough_handler_try
                        .then(|| {
                            let start = self.assembler.label();
                            self.assembler.mark(start);
                            start
                        });
                    let exception_policy = if handler_body_has_instructions
                        && nested_in_final_outer_try_statement
                        && self.control.active_pass_finally_locations.is_empty()
                        && !protected_loop_value_return
                    {
                        ExceptionPolicy::EXCLUDE_ALWAYS
                    } else if !handler_body_has_instructions
                        && !self.control.active_exception_region_exclusions.is_empty()
                        && self.control.active_pass_finally_locations.is_empty()
                        && !protected_loop_value_return
                        && !following_expression_exit_keeps_owner
                    {
                        ExceptionPolicy::EXCLUDE_IF_EXTENDED
                    } else {
                        ExceptionPolicy::default()
                    };
                    self.emit_jump_forward_with_exception_policy(
                        JUMP_FORWARD,
                        end,
                        0,
                        exception_policy,
                    )?;
                    if let Some(start) = trailing_handler_exit_exclusion_start {
                        let exclusion_end = self.assembler.label();
                        self.assembler.mark(exclusion_end);
                        self.control
                            .active_exception_region_exclusions
                            .last_mut()
                            .expect("trailing handler try exit requires an outer handler")
                            .push((start, exclusion_end));
                    }
                    if handler_ends_with_inlineable_cleanup {
                        // The cleanup is larger than a small exit, but is still eligible because
                        // it has no line number and cannot fall through.
                        self.assembler.prepare_last_no_location_block_for_inlining();
                        if handler_ends_with_nonterminal_with {
                            self.assembler.make_last_no_location_copy_interruptible();
                        }
                    }
                    let return_is_overridden = self.control.active_overriding_finally_returns > 0
                        && matches!(handler.body.last(), Some(Stmt::Return(_)));
                    if self.control.prevent_try_exit_inlining && !return_is_overridden {
                        self.assembler.prevent_last_jump_inlining();
                    }
                }
            }

            if let Some(name) = &handler.name {
                self.assembler.mark(name_cleanup);
                self.set_depth(base_depth + 3);
                self.assembler.set_location(SourceLocation::NONE);
                let none = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, none, 1)?;
                self.store_name(name.as_str())?;
                self.delete_name(name.as_str())?;
                self.emit_reraise(ReraiseStackState::HandlerCleanup)?;
            }

            self.assembler.mark(next_handler);
            dispatch_start = if handler.name.is_some() {
                name_cleanup
            } else {
                next_handler
            };
        }

        if statement.handlers.last().is_some_and(|handler| {
            handler
                .as_except_handler()
                .is_some_and(|handler| handler.type_.is_some())
        }) {
            self.set_depth(base_depth + 2);
            self.assembler.set_location(final_handler_location);
            self.emit_reraise(ReraiseStackState::ActiveExceptionHandler)?;
            let dispatch_end = self.assembler.label();
            self.assembler.mark(dispatch_end);
            self.assembler.add_exception_region(
                dispatch_start,
                dispatch_end,
                common_cleanup,
                (base_depth + 1).cast_unsigned(),
                true,
            );
        }

        self.assembler.mark(common_cleanup);
        self.set_depth(base_depth + 3);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(COPY, 3, 1)?;
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_reraise(ReraiseStackState::HandlerCleanup)?;

        self.assembler.mark(end);
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_try_star_except(
        &mut self,
        statement: &ruff_python_ast::StmtTry,
        terminal: bool,
        emit_statement_nop: bool,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let try_start = self.assembler.label();
        let try_end = self.assembler.label();
        let handler_start = self.assembler.label();
        let orelse = self.assembler.label();
        let end = self.assembler.label();
        let common_cleanup = self.assembler.label();
        let reraise_star = self.assembler.label();
        let reraise = self.assembler.label();

        if emit_statement_nop {
            let statement_nop_start = self.assembler.label();
            self.assembler.mark(statement_nop_start);
            self.assembler
                .set_location(self.source_location_including_trailing_semicolon(statement.range));
            self.emit(NOP, 0, 0)?;
            let statement_nop_end = self.assembler.label();
            self.assembler.mark(statement_nop_end);
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((statement_nop_start, statement_nop_end));
            }
        }
        self.assembler.mark(try_start);
        self.control
            .active_exception_region_exclusions
            .push(Vec::new());
        let body_instruction_start = self.assembler.instruction_count();
        self.with_try_body_context(statement, |compiler| {
            compiler.compile_suite(&statement.body)
        })?;
        let body_noop_location = (self.assembler.instruction_count() == body_instruction_start)
            .then(|| {
                statement
                    .body
                    .last()
                    .map(|statement| self.source_location(statement.range()))
            })
            .flatten();
        let body_fallthrough_location =
            body_noop_location.or_else(|| self.assembler.last_instruction_location());
        let exit_exclusion_start = if body_noop_location.is_some()
            && statement.orelse.is_empty()
            && !self.control.prevent_try_exit_inlining
            && !self.control.active_exception_region_exclusions.is_empty()
        {
            let start = self.assembler.label();
            self.assembler.mark(start);
            Some(start)
        } else {
            None
        };
        if !terminal && let Some(location) = body_noop_location {
            self.assembler.set_location(location);
            self.emit(NOP, 0, 0)?;
        }
        self.assembler.mark(try_end);
        let try_exclusions = self
            .control
            .active_exception_region_exclusions
            .pop()
            .expect("active try-star region has exclusion collector");
        self.assembler.set_location(if terminal {
            body_fallthrough_location.unwrap_or(SourceLocation::NONE)
        } else {
            SourceLocation::NONE
        });
        self.emit_jump_forward(JUMP_FORWARD, orelse, 0)?;
        if let Some(exit_exclusion_start) = exit_exclusion_start {
            let exit_exclusion_end = self.assembler.label();
            self.assembler.mark(exit_exclusion_end);
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((exit_exclusion_start, exit_exclusion_end));
            }
        }
        self.add_exception_regions_with_exclusions(
            try_start,
            try_end,
            &try_exclusions,
            handler_start,
            base_depth.cast_unsigned(),
            false,
        );

        self.assembler.mark(handler_start);
        self.set_depth(base_depth + 1);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(PUSH_EXC_INFO, 0, 1)?;
        let handler_region_start = handler_start;
        let mut handler_region_exclusions = Vec::new();

        for (handler_index, handler) in statement.handlers.iter().enumerate() {
            let handler = handler.as_except_handler().ok_or_else(|| {
                CompileError::Internal("invalid exception-group handler node".to_string())
            })?;
            let handler_location = self.source_location_including_trailing_semicolon(handler.range);
            let next_handler = self.assembler.label();
            let handler_error = self.assembler.label();
            let no_match = self.assembler.label();
            let body_cleanup = self.assembler.label();

            self.set_depth(base_depth + 2);
            if handler_index == 0 {
                self.assembler.set_location(handler_location);
                self.emit(BUILD_LIST, 0, 1)?;
                self.emit(COPY, 2, 1)?;
            }
            let exception_type = handler.type_.as_ref().ok_or_else(|| {
                CompileError::Internal("exception-group handler has no type".to_string())
            })?;
            let bool_marker_state = matches!(exception_type.as_ref(), Expr::BoolOp(_)).then(|| {
                ExceptionTypeBoolMarkerState::new(
                    handler_index == 0,
                    self.assembler.current_block_token(),
                )
            });
            self.with_control_override(
                |control| &mut control.exception_type_bool_marker_state,
                bool_marker_state,
                |compiler| compiler.compile_expression(exception_type),
            )?;
            self.assembler.set_location(handler_location);
            self.emit(CHECK_EG_MATCH, 0, 0)?;
            self.emit(COPY, 1, 1)?;
            self.emit_jump_forward(POP_JUMP_IF_NONE, no_match, -1)?;
            let ownerless_not_taken = handler_index > 0
                || matches!(exception_type.as_ref(), Expr::BoolOp(_) | Expr::If(_));
            let not_taken_exclusion_start = ownerless_not_taken.then(|| {
                let start = self.assembler.label();
                self.assembler.mark(start);
                start
            });
            self.emit_with_exception_policy(
                NOT_TAKEN,
                0,
                0,
                if ownerless_not_taken {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::default()
                },
            )?;
            if let Some(start) = not_taken_exclusion_start {
                let end = self.assembler.label();
                self.assembler.mark(end);
                handler_region_exclusions.push((start, end));
                // The synthetic second-handler marker belongs to neither the group handler nor
                // an enclosing finally region.
                for exclusions in &mut self.control.active_exception_region_exclusions {
                    exclusions.push((start, end));
                }
            }

            if let Some(name) = &handler.name {
                self.store_name(name.as_str())?;
            } else {
                self.emit(POP_TOP, 0, -1)?;
            }
            let body_start = self.assembler.label();
            self.assembler.mark(body_start);
            let body_instruction_start = self.assembler.instruction_count();
            self.with_control_counter(
                |control| &mut control.active_exception_group_handlers,
                |compiler| compiler.compile_suite(&handler.body),
            )?;
            let body_has_instructions = self.assembler.instruction_count() > body_instruction_start;
            let body_end = self.assembler.label();
            self.assembler.mark(body_end);
            if let Some(name) = &handler.name {
                if body_has_instructions
                    && let Some(location) = self.assembler.last_instruction_location()
                {
                    self.assembler.set_location(location);
                } else if let Some(statement) = handler.body.last() {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                }
                let none = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, none, 1)?;
                self.store_name(name.as_str())?;
                self.delete_name(name.as_str())?;
            }
            let is_last = handler_index + 1 == statement.handlers.len();
            if is_last {
                if body_has_instructions
                    && let Some(location) = self.assembler.last_instruction_location()
                {
                    self.assembler.set_location(location);
                } else if let Some(statement) = handler.body.last() {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                }
                self.emit(LIST_APPEND, 1, -1)?;
                self.emit_jump_forward(JUMP_FORWARD, reraise_star, 0)?;
            } else {
                if body_has_instructions
                    && let Some(location) = self.assembler.last_instruction_location()
                {
                    self.assembler.set_location(location);
                } else if let Some(statement) = handler.body.last() {
                    self.assembler
                        .set_location(self.source_location(statement.range()));
                }
                self.emit_jump_forward(JUMP_FORWARD, next_handler, 0)?;
            }

            self.assembler.add_exception_region(
                body_start,
                body_end,
                body_cleanup,
                (base_depth + 4).cast_unsigned(),
                true,
            );
            self.assembler.mark(body_cleanup);
            self.set_depth(base_depth + 6);
            self.assembler.set_location(SourceLocation::NONE);
            if let Some(name) = &handler.name {
                let none = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, none, 1)?;
                self.store_name(name.as_str())?;
                self.delete_name(name.as_str())?;
            }
            self.emit(LIST_APPEND, 3, -1)?;
            self.emit(POP_TOP, 0, -1)?;
            if is_last {
                self.emit(LIST_APPEND, 1, -1)?;
                self.emit_jump_forward(JUMP_FORWARD, reraise_star, 0)?;
            } else {
                self.emit_jump_forward(JUMP_FORWARD, handler_error, 0)?;
            }

            if !is_last {
                self.assembler.mark(next_handler);
                self.set_depth(base_depth + 4);
                self.emit(NOP, 0, 0)?;
                self.emit_jump_forward(JUMP_FORWARD, handler_error, 0)?;
            }

            self.assembler.mark(no_match);
            self.set_depth(base_depth + 5);
            self.assembler.set_location(handler_location);
            self.emit(POP_TOP, 0, -1)?;

            self.set_depth(base_depth + 4);
            if is_last {
                self.assembler.set_location(SourceLocation::NONE);
                self.emit(LIST_APPEND, 1, -1)?;
                self.emit_jump_forward(JUMP_FORWARD, reraise_star, 0)?;
            } else {
                self.assembler.mark(handler_error);
            }
        }

        self.assembler.mark(reraise_star);
        self.set_depth(base_depth + 3);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit_binary_intrinsic(BinaryIntrinsic::PREP_RERAISE_STAR)?;
        self.emit(COPY, 1, 1)?;
        self.emit_jump_forward(POP_JUMP_IF_NOT_NONE, reraise, -1)?;
        let exclusion_start = self.assembler.label();
        self.assembler.mark(exclusion_start);
        self.emit(NOT_TAKEN, 0, 0)?;
        let exclusion_end = self.assembler.label();
        self.assembler.mark(exclusion_end);
        handler_region_exclusions.push((exclusion_start, exclusion_end));
        for exclusions in &mut self.control.active_exception_region_exclusions {
            exclusions.push((exclusion_start, exclusion_end));
        }
        self.emit(POP_TOP, 0, -1)?;
        let handler_region_end = self.assembler.label();
        self.assembler.mark(handler_region_end);
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_jump_forward(JUMP_FORWARD, end, 0)?;

        self.assembler.mark(reraise);
        self.set_depth(base_depth + 2);
        self.emit(SWAP, 2, 0)?;
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_reraise(ReraiseStackState::ExceptionOnly)?;
        self.add_exception_regions_with_exclusions(
            handler_region_start,
            handler_region_end,
            &handler_region_exclusions,
            common_cleanup,
            (base_depth + 1).cast_unsigned(),
            true,
        );

        self.assembler.mark(common_cleanup);
        self.set_depth(base_depth + 3);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(COPY, 3, 1)?;
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_reraise(ReraiseStackState::HandlerCleanup)?;

        self.assembler.mark(orelse);
        self.set_depth(base_depth);
        let orelse_instruction_start = self.assembler.instruction_count();
        self.compile_suite(&statement.orelse)?;
        // A protected try-star else suite retains its last pass marker.
        if self.assembler.instruction_count() == orelse_instruction_start
            && let Some(Stmt::Pass(statement)) = statement.orelse.last()
        {
            self.assembler
                .set_location(self.source_location(statement.range));
            self.emit(NOP, 0, 0)?;
        }
        self.assembler.mark(end);
        self.set_depth(base_depth);
        if terminal {
            if let Some(location) = self.assembler.last_instruction_location() {
                self.assembler.set_location(location);
            }
            self.emit_implicit_return()?;
        }
        Ok(())
    }

    fn compile_terminal_exception_handler_if(
        &mut self,
        statement: &ruff_python_ast::StmtIf,
        name: &str,
        handler_depth: i32,
    ) -> Result<Vec<(Label, Label)>, CompileError> {
        let mut branches: Vec<(Option<&Expr>, &[Stmt])> =
            vec![(Some(&statement.test), statement.body.as_slice())];
        branches.extend(
            statement
                .elif_else_clauses
                .iter()
                .map(|clause| (clause.test.as_ref(), clause.body.as_slice())),
        );
        let has_else = branches.last().is_some_and(|(test, _)| test.is_none());
        let branch_count = branches.len();
        let mut exclusions = Vec::new();
        let mut last_test: &Expr = statement.test.as_ref();

        for (branch_index, (test, body)) in branches.into_iter().enumerate() {
            let next = if let Some(test) = test {
                last_test = test;
                let next = self.assembler.label();
                if branch_index > 0 && branch_index + 1 == branch_count && !has_else {
                    let ((), branch_exclusions) = self.with_region_exclusion_collector(
                        |control| &mut control.active_with_region_exclusions,
                        |compiler| {
                            compiler.with_control_override(
                                |control| &mut control.exclude_terminal_if_not_taken,
                                true,
                                |compiler| compiler.compile_jump_if(test, false, next),
                            )
                        },
                    )?;
                    exclusions.extend(branch_exclusions);
                } else {
                    self.compile_jump_if(test, false, next)?;
                }
                Some(next)
            } else {
                None
            };

            let action_location = if matches!(body, [Stmt::Return(statement)] if statement.value.is_none())
            {
                self.source_location(body[0].range())
            } else {
                let instruction_count = self.assembler.instruction_count();
                self.compile_suite(body)?;
                if let Some(range) = suite_unary_not_fallthrough_range(body) {
                    self.source_location(range)
                } else if self.assembler.instruction_count() > instruction_count {
                    self.assembler
                        .last_instruction_location()
                        .unwrap_or_else(|| {
                            self.source_location(synthetic_condition_exit_range(last_test))
                        })
                } else {
                    body.last().map_or_else(
                        || self.source_location(synthetic_condition_exit_range(last_test)),
                        |statement| self.source_location(statement.range()),
                    )
                }
            };
            exclusions.push(self.emit_terminal_exception_handler_action(
                name,
                handler_depth,
                action_location,
            )?);

            if let Some(next) = next {
                self.assembler.mark(next);
                self.set_depth(handler_depth);
            }
        }
        if !has_else {
            exclusions.push(self.emit_terminal_exception_handler_action(
                name,
                handler_depth,
                self.source_location(synthetic_condition_exit_range(last_test)),
            )?);
        }
        Ok(exclusions)
    }

    fn emit_terminal_exception_handler_action(
        &mut self,
        name: &str,
        handler_depth: i32,
        location: SourceLocation,
    ) -> Result<(Label, Label), CompileError> {
        let start = self.assembler.label();
        self.assembler.mark(start);
        self.assembler.set_location(location);
        self.emit(POP_EXCEPT, 0, -1)?;
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.store_name(name)?;
        self.delete_name(name)?;
        self.emit_implicit_return()?;
        let end = self.assembler.label();
        self.assembler.mark(end);
        self.set_depth(handler_depth);
        Ok((start, end))
    }

    fn add_exception_regions_with_exclusions(
        &mut self,
        start: Label,
        end: Label,
        exclusions: &[(Label, Label)],
        target: Label,
        depth: u32,
        preserve_lasti: bool,
    ) {
        let mut region_start = start;
        for (exclusion_start, exclusion_end) in exclusions {
            self.assembler.add_exception_region(
                region_start,
                *exclusion_start,
                target,
                depth,
                preserve_lasti,
            );
            region_start = *exclusion_end;
        }
        self.assembler
            .add_exception_region(region_start, end, target, depth, preserve_lasti);
    }

    fn compile_try_finally(
        &mut self,
        statement: &ruff_python_ast::StmtTry,
        terminal: bool,
    ) -> Result<(), CompileError> {
        let preserve_exit_nop = std::mem::take(&mut self.control.preserve_next_finally_exit_nop);

        if statement.handlers.is_empty()
            && statement.orelse.is_empty()
            && matches!(statement.body.as_slice(), [Stmt::Continue(_)])
            && matches!(statement.finalbody.as_slice(), [Stmt::Break(_)])
        {
            return self.compile_try_finally_continue_break(statement);
        }
        let base_depth = self.control.depth;
        let protected_start = self.assembler.label();
        let protected_end = self.assembler.label();
        let finally_handler = self.assembler.label();
        let cleanup = self.assembler.label();
        let end = self.assembler.label();
        let fallthrough_handlers_reach_simple_finally =
            statement.handlers.iter().all(|handler| {
                handler.as_except_handler().is_some_and(|handler| {
                    handler.type_.is_some()
                        && handler.body.iter().all(|statement| {
                            !matches!(
                                statement,
                                Stmt::Assert(_)
                                    | Stmt::Break(_)
                                    | Stmt::Continue(_)
                                    | Stmt::For(_)
                                    | Stmt::If(_)
                                    | Stmt::Match(_)
                                    | Stmt::Raise(_)
                                    | Stmt::Return(_)
                                    | Stmt::Try(_)
                                    | Stmt::While(_)
                                    | Stmt::With(_)
                            )
                        })
                })
            }) && matches!(statement.finalbody.as_slice(), [Stmt::Expr(_)])
                && !matches!(
                    statement.body.last(),
                    Some(Stmt::If(_) | Stmt::Try(_) | Stmt::While(_) | Stmt::With(_))
                );
        if !statement.handlers.is_empty() && !fallthrough_handlers_reach_simple_finally {
            // A terminating handler leaves an empty CPython CFG block before the normal finally
            // body. Bare or compound handlers and compound try-body exits can leave the same
            // boundary. Linear typed handlers keep a single-expression body reachable.
            self.assembler.prevent_borrow_reachability(protected_end);
        }
        if !statement.handlers.is_empty()
            && matches!(statement.finalbody.first(), Some(Stmt::Try(_)))
        {
            // Keep handler exits targeting the nested finally body instead of the removed
            // normal-exit NOP that precedes it.
            self.assembler.preserve_block_boundary(protected_end);
        }

        let statement_nop_start = self.assembler.label();
        self.assembler.mark(statement_nop_start);
        self.emit(NOP, 0, 0)?;
        let statement_nop_end = self.assembler.label();
        self.assembler.mark(statement_nop_end);
        for exclusions in &mut self.control.active_exception_region_exclusions {
            exclusions.push((statement_nop_start, statement_nop_end));
        }
        for exclusions in &mut self.control.active_with_region_exclusions {
            exclusions.push((statement_nop_start, statement_nop_end));
        }
        if self.control.generator_region_start.is_some() {
            self.control
                .generator_region_exclusions
                .push((statement_nop_start, statement_nop_end));
        }
        self.assembler.mark(protected_start);
        let pass_finally_location = match statement.finalbody.as_slice() {
            [Stmt::Pass(statement)] => Some(self.source_location(statement.range)),
            _ => None,
        };
        let overriding_return = matches!(statement.finalbody.last(), Some(Stmt::Return(_)));
        let preserve_break_exit_loop_range = if overriding_return {
            match statement.body.last() {
                Some(Stmt::While(loop_statement))
                    if loop_statement.orelse.is_empty()
                        && suite_contains_loop_break(&loop_statement.body) =>
                {
                    Some(loop_statement.range)
                }
                Some(Stmt::For(loop_statement))
                    if loop_statement.orelse.is_empty()
                        && suite_contains_loop_break(&loop_statement.body) =>
                {
                    Some(loop_statement.range)
                }
                _ => self.control.preserve_finally_break_exit_loop_range,
            }
        } else {
            self.control.preserve_finally_break_exit_loop_range
        };
        let copy_finally_on_return = pass_finally_location.is_none()
            && !suite_terminates(&statement.finalbody)
            && self.control.active_exception_handlers.is_empty();
        let return_context = copy_finally_on_return.then(|| ReturnFinallyContext {
            body: statement.finalbody.to_vec(),
            loop_depth: self.control.loops.len(),
            handler: finally_handler,
            depth: base_depth.cast_unsigned(),
            preserves_conditional_exit_owner: statement.handlers.is_empty()
                && finally_suite_preserves_none_condition_owner(&statement.finalbody),
        });
        let (protected_has_instructions, protected_exclusions) = self
            .with_finally_protected_context(
                pass_finally_location,
                overriding_return,
                preserve_break_exit_loop_range,
                finally_suite_preserves_none_condition_owner(&statement.finalbody),
                return_context,
                |compiler| {
                    if statement.handlers.is_empty() {
                        let instruction_count = compiler.assembler.instruction_count();
                        if statement.body.len() > 1
                            && let Some(Stmt::Pass(pass)) = statement.body.first()
                        {
                            compiler
                                .assembler
                                .set_location(compiler.source_location(pass.range));
                            compiler.emit(NOP, 0, 0)?;
                        }
                        compiler.with_try_body_context(statement, |compiler| {
                            compiler.compile_suite(&statement.body)
                        })?;
                        let has_instructions =
                            compiler.assembler.instruction_count() > instruction_count;
                        if !has_instructions {
                            if let Some(last) = statement.body.last() {
                                compiler
                                    .assembler
                                    .set_location(compiler.source_location(last.range()));
                            }
                            // A no-op protected body does not acquire an enclosing try's handler.
                            let exception_policy =
                                if compiler.control.active_exception_region_exclusions.len() > 1 {
                                    ExceptionPolicy::EXCLUDE_ALWAYS
                                } else {
                                    ExceptionPolicy::default()
                                };
                            compiler.emit_with_exception_policy(NOP, 0, 0, exception_policy)?;
                        }
                        Ok(has_instructions)
                    } else {
                        compiler.with_control_override(
                            |control| &mut control.prevent_try_exit_inlining,
                            true,
                            |compiler| {
                                compiler.compile_try_except(
                                    statement,
                                    TryCompileOptions::NESTED_HANDLER,
                                )
                            },
                        )?;
                        Ok(true)
                    }
                },
            )?;
        self.assembler.mark(protected_end);
        if terminal
            && pass_finally_location.is_some()
            && !statement.handlers.is_empty()
            && suite_terminates(&statement.body)
            && statement.handlers.iter().any(|handler| {
                handler
                    .as_except_handler()
                    .is_some_and(|handler| !suite_terminates(&handler.body))
            })
            && let Some(location) = statement.handlers.iter().rev().find_map(|handler| {
                handler
                    .as_except_handler()
                    .and_then(|handler| handler.body.last())
                    .map(|statement| self.source_location(statement.range()))
            })
        {
            let exit_nop_start = self.assembler.label();
            self.assembler.mark(exit_nop_start);
            self.assembler.set_location(location);
            self.emit(NOP, 0, 0)?;
            let exit_nop_end = self.assembler.label();
            self.assembler.mark(exit_nop_end);
            if self.control.generator_region_start.is_some() {
                self.control
                    .generator_region_exclusions
                    .push((exit_nop_start, exit_nop_end));
            }
        }
        // `POP_BLOCK` is converted to a NOP before CPython's CFG optimizer runs. It normally
        // disappears, but can retain the sole predecessor's line when a small finally exit is
        // copied onto another edge.
        let normal_finally_pop_block_exclusion_start =
            (!self.control.active_with_region_exclusions.is_empty()
                && !self.control.current_statement_has_following
                && matches!(statement.finalbody.as_slice(), [Stmt::If(_)]))
            .then(|| {
                let start = self.assembler.label();
                self.assembler.mark(start);
                start
            });
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(NOP, 0, 0)?;
        if let Some(start) = normal_finally_pop_block_exclusion_start {
            let end = self.assembler.label();
            self.assembler.mark(end);
            for exclusions in &mut self.control.active_with_region_exclusions {
                exclusions.push((start, end));
            }
        }
        let noop_finally_body = statement.finalbody.iter().all(|statement| {
            matches!(statement, Stmt::Pass(_))
                || matches!(statement, Stmt::Expr(expression) if fold_constant(&expression.value).is_some())
        });
        // A handler-form try leaves its converted POP_BLOCK unlocated when the normal finally
        // body has no executable instructions. Otherwise, predecessor propagation can retain it.
        if statement.handlers.is_empty() || !noop_finally_body {
            self.assembler.mark_last_as_converted_pop_block();
        } else {
            self.assembler.preserve_last_no_location();
        }
        let ((), finalbody_emitted_fallthrough) = self.with_fallthrough_tracking(|compiler| {
            if let Some(location) = pass_finally_location {
                compiler.assembler.set_location(location);
                if compiler
                    .control
                    .active_exception_region_exclusions
                    .is_empty()
                {
                    compiler.emit(NOP, 0, 0)?;
                } else {
                    // The normal finally copy is outside an enclosing try's exception ownership.
                    // Mark both the pass and its exit because CFG optimization can keep either NOP.
                    let pass_start = compiler.assembler.label();
                    compiler.assembler.mark(pass_start);
                    compiler.emit_with_exception_policy(
                        NOP,
                        0,
                        0,
                        ExceptionPolicy::EXCLUDE_ALWAYS,
                    )?;
                    let pass_end = compiler.assembler.label();
                    compiler.assembler.mark(pass_end);
                    for exclusions in &mut compiler.control.active_exception_region_exclusions {
                        exclusions.push((pass_start, pass_end));
                    }
                }
            } else {
                compiler.with_control_counter(
                    |control| &mut control.active_normal_finally_bodies,
                    |compiler| compiler.compile_suite_inner(&statement.finalbody, terminal),
                )?;
            }
            Ok(())
        })?;
        if terminal && !finalbody_emitted_fallthrough && !suite_terminates(&statement.finalbody) {
            if let Some(range) = suite_unary_not_fallthrough_range(&statement.finalbody) {
                self.assembler.set_location(self.source_location(range));
            } else if let Some(statement) = statement
                .finalbody
                .last()
                .filter(|statement| statement_uses_implicit_return_location(statement))
            {
                self.assembler
                    .set_location(self.source_location(implicit_return_range(self, statement)));
            } else if let Some(location) = self.assembler.last_instruction_location() {
                self.assembler.set_location(location);
            }
            self.emit_implicit_return()?;
        } else if !terminal {
            if let Some(location) = self.assembler.last_instruction_location() {
                self.assembler.set_location(location);
            }
            let exception_policy = if pass_finally_location.is_some()
                && !self.control.active_exception_region_exclusions.is_empty()
            {
                ExceptionPolicy::EXCLUDE_ALWAYS
            } else {
                ExceptionPolicy::default()
            };
            let exit_exclusion_start = preserve_exit_nop.then(|| {
                let start = self.assembler.label();
                self.assembler.mark(start);
                start
            });
            self.emit_jump_forward_with_exception_policy(JUMP_FORWARD, end, 0, exception_policy)?;
            if preserve_exit_nop {
                self.assembler
                    .preserve_last_redundant_jump_nop_after_threading();
            }
            if let Some(exit_exclusion_start) = exit_exclusion_start {
                let exit_exclusion_end = self.assembler.label();
                self.assembler.mark(exit_exclusion_end);
                for exclusions in &mut self.control.active_with_region_exclusions {
                    exclusions.push((exit_exclusion_start, exit_exclusion_end));
                }
                for exclusions in &mut self.control.active_exception_region_exclusions {
                    exclusions.push((exit_exclusion_start, exit_exclusion_end));
                }
                if self.control.generator_region_start.is_some() {
                    self.control
                        .generator_region_exclusions
                        .push((exit_exclusion_start, exit_exclusion_end));
                }
            }
        }
        if protected_has_instructions {
            self.add_exception_regions_with_exclusions(
                protected_start,
                protected_end,
                &protected_exclusions,
                finally_handler,
                base_depth.cast_unsigned(),
                false,
            );
        } else {
            self.assembler.add_exception_region(
                protected_end,
                protected_end,
                finally_handler,
                base_depth.cast_unsigned(),
                false,
            );
        }

        self.assembler.mark(finally_handler);
        self.set_depth(base_depth + 1);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(PUSH_EXC_INFO, 0, 1)?;
        let handler_start = finally_handler;
        self.control
            .active_exception_region_exclusions
            .push(Vec::new());
        let mut explicit_handler_end = None;
        if let Some((Stmt::Return(return_statement), leading)) = statement.finalbody.split_last() {
            self.compile_suite(leading)?;
            explicit_handler_end =
                Some(self.compile_return_from_exception_handler(return_statement)?);
        } else if let [Stmt::If(final_if)] = statement.finalbody.as_slice()
            && final_if.elif_else_clauses.is_empty()
        {
            let condition_false = if self.control.in_synchronous_with()
                && matches!(final_if.body.as_slice(), [Stmt::Expr(_)])
                && let Expr::BoolOp(boolean) = final_if.test.as_ref()
                && boolean.op == BoolOp::And
                && let [first, second] = boolean.values.as_slice()
                && matches!((first, second), (Expr::Compare(_), Expr::Name(_)))
            {
                let first_false = self.assembler.label();
                let second_false = self.assembler.label();
                self.with_control_override(
                    |control| &mut control.exceptional_finally_condition,
                    true,
                    |compiler| {
                        compiler.compile_jump_if(first, false, first_false)?;
                        compiler.compile_jump_if(second, false, second_false)
                    },
                )?;
                vec![(first_false, first), (second_false, second)]
            } else {
                let condition_false = self.assembler.label();
                self.with_control_override(
                    |control| &mut control.exceptional_finally_condition,
                    true,
                    |compiler| compiler.compile_jump_if(&final_if.test, false, condition_false),
                )?;
                vec![(condition_false, final_if.test.as_ref())]
            };
            self.with_control_counter(
                |control| &mut control.active_finally_end_blocks,
                |compiler| compiler.compile_suite(&final_if.body),
            )?;
            if !suite_terminates(&final_if.body) {
                if let Some(location) = self.assembler.last_instruction_location() {
                    self.assembler.set_location(location);
                }
                self.emit_reraise(ReraiseStackState::ActiveExceptionHandler)?;
            }
            for (condition_false, test) in condition_false.into_iter().rev() {
                self.assembler.mark(condition_false);
                self.set_depth(base_depth + 2);
                self.assembler
                    .set_location(self.source_location(synthetic_condition_exit_range(test)));
                self.emit_reraise(ReraiseStackState::ActiveExceptionHandler)?;
            }
        } else {
            if let Some(location) = pass_finally_location {
                self.assembler.set_location(location);
                self.emit(NOP, 0, 0)?;
            } else {
                self.with_control_counter(
                    |control| &mut control.active_finally_end_blocks,
                    |compiler| compiler.compile_suite(&statement.finalbody),
                )?;
            }
            if !suite_terminates(&statement.finalbody) {
                if let Some(range) = suite_unary_not_fallthrough_range(&statement.finalbody) {
                    self.assembler.set_location(self.source_location(range));
                } else if let Some(statement) = statement
                    .finalbody
                    .last()
                    .filter(|statement| statement_uses_implicit_return_location(statement))
                {
                    self.assembler
                        .set_location(self.source_location(implicit_return_range(self, statement)));
                } else if let Some(location) = self.assembler.last_instruction_location() {
                    self.assembler.set_location(location);
                }
                self.emit_reraise(ReraiseStackState::ActiveExceptionHandler)?;
            }
        }
        let handler_end = if let Some(handler_end) = explicit_handler_end {
            handler_end
        } else {
            let handler_end = self.assembler.label();
            self.assembler.mark(handler_end);
            handler_end
        };
        let handler_exclusions = self
            .control
            .active_exception_region_exclusions
            .pop()
            .expect("active finally handler has exclusion collector");
        self.add_exception_regions_with_exclusions(
            handler_start,
            handler_end,
            &handler_exclusions,
            cleanup,
            (base_depth + 1).cast_unsigned(),
            true,
        );

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 3);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(COPY, 3, 1)?;
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_reraise(ReraiseStackState::HandlerCleanup)?;
        self.assembler.mark(end);
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_try_finally_continue_break(
        &mut self,
        statement: &ruff_python_ast::StmtTry,
    ) -> Result<(), CompileError> {
        let base_depth = self.control.depth;
        let context = self
            .control
            .loops
            .last()
            .copied()
            .ok_or_else(|| CompileError::Internal("break outside loop".to_string()))?;
        let continue_statement = &statement.body[0];
        let break_statement = &statement.finalbody[0];
        let handler = self.assembler.label();
        let handler_end = self.assembler.label();
        let cleanup = self.assembler.label();
        let empty_protected = self.assembler.label();

        self.assembler
            .set_location(self.source_location(statement.range));
        self.emit(NOP, 0, 0)?;
        self.assembler.mark(empty_protected);
        self.assembler.add_exception_region(
            empty_protected,
            empty_protected,
            handler,
            base_depth.cast_unsigned(),
            false,
        );
        self.assembler
            .set_location(self.source_location(continue_statement.range()));
        self.emit(NOP, 0, 0)?;
        self.assembler
            .set_location(self.source_location(break_statement.range()));
        match context.iterator_cleanup {
            IteratorCleanup::None => {}
            IteratorCleanup::Sync | IteratorCleanup::Async => self.emit(POP_TOP, 0, -1)?,
        }
        if context.break_policy.returns() {
            self.emit_implicit_return()?;
        } else {
            self.emit_jump_forward(JUMP_FORWARD, context.break_label, 0)?;
            self.assembler.mark_last_jump_as_authored_break();
        }

        self.assembler.mark(handler);
        self.set_depth(base_depth + 1);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(PUSH_EXC_INFO, 0, 1)?;
        self.assembler
            .set_location(self.source_location(break_statement.range()));
        self.emit(POP_TOP, 0, -1)?;
        self.assembler.mark(handler_end);
        self.emit(POP_EXCEPT, 0, -1)?;
        match context.iterator_cleanup {
            IteratorCleanup::None => {}
            IteratorCleanup::Sync | IteratorCleanup::Async => self.emit(POP_TOP, 0, -1)?,
        }
        if context.break_policy.returns() {
            self.emit_implicit_return()?;
        } else {
            self.emit_jump_forward(JUMP_FORWARD, context.break_label, 0)?;
            self.assembler.mark_last_jump_as_authored_break();
        }
        self.assembler.add_exception_region(
            handler,
            handler_end,
            cleanup,
            (base_depth + 1).cast_unsigned(),
            true,
        );

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 3);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(COPY, 3, 1)?;
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_reraise(ReraiseStackState::HandlerCleanup)?;
        self.set_depth(base_depth);
        Ok(())
    }

    fn compile_return_from_exception_handler(
        &mut self,
        statement: &ruff_python_ast::StmtReturn,
    ) -> Result<Label, CompileError> {
        let preserve_value = statement
            .value
            .as_deref()
            .is_some_and(|value| !is_literal_constant(value));
        if preserve_value {
            self.compile_expression(
                statement
                    .value
                    .as_deref()
                    .expect("preserved return has a value"),
            )?;
            self.emit(SWAP, 2, 0)?;
        } else {
            let range = statement
                .value
                .as_deref()
                .map_or(statement.range, Ranged::range);
            self.assembler.set_location(self.source_location(range));
        }

        self.emit(POP_TOP, 0, -1)?;
        if preserve_value {
            self.emit(SWAP, 2, 0)?;
        }
        let handler_end = self.assembler.label();
        self.assembler.mark(handler_end);
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_loop_iterator_cleanup(0, self.control.loops.len(), preserve_value)?;

        let pass_finally_unwind_start = (!self.control.active_pass_finally_locations.is_empty())
            .then(|| {
                let start = self.assembler.label();
                self.assembler.mark(start);
                start
            });
        self.emit_pass_finally_nops()?;

        if !preserve_value {
            if let Some(value) = &statement.value {
                self.compile_expression(value)?;
            } else {
                let none = self.add_constant(Constant::None)?;
                self.emit(LOAD_CONST, none, 1)?;
            }
        }
        self.emit(RETURN_VALUE, 0, -1)?;
        if let Some(pass_finally_unwind_start) = pass_finally_unwind_start {
            let unwind_end = self.assembler.label();
            self.assembler.mark(unwind_end);
            let outer_region_count = self
                .control
                .active_exception_region_exclusions
                .len()
                .saturating_sub(1);
            for exclusions in self
                .control
                .active_exception_region_exclusions
                .iter_mut()
                .take(outer_region_count)
            {
                exclusions.push((pass_finally_unwind_start, unwind_end));
            }
        }
        Ok(handler_end)
    }

    fn compile_with(&mut self, statement: &ruff_python_ast::StmtWith) -> Result<(), CompileError> {
        self.compile_with_statement(statement, false)
    }

    fn compile_with_statement(
        &mut self,
        statement: &ruff_python_ast::StmtWith,
        terminal: bool,
    ) -> Result<(), CompileError> {
        // A direct try/except omits its owner from the suppressed handler exit. A sole synthetic
        // try/finally owner remains, while a handler-form try nested in its protected body omits
        // only that nested owner and remains covered by the outer finally.
        let exclude_handler_exit = (self.control.active_finally_try_bodies == 0
            && self.control.direct_final_sync_with_try_body == Some(statement.range))
            || self
                .control
                .direct_final_sync_with_protected_handler_try_body
                == Some(statement.range);
        self.compile_with_items(
            &statement.items,
            &statement.body,
            terminal,
            exclude_handler_exit,
        )
    }

    fn compile_with_body(
        &mut self,
        remaining: &[ruff_python_ast::WithItem],
        body: &[Stmt],
    ) -> Result<Option<WithBodyExitNoop>, CompileError> {
        let mut body_noop = None;
        if remaining.is_empty() {
            let body_start = self.assembler.instruction_count();
            let body_previous_location = self.assembler.last_instruction_location();
            let terminal_if = matches!(
                body.last(),
                Some(Stmt::If(statement))
                    if statement.elif_else_clauses.is_empty()
                        && suite_terminates(&statement.body)
            );
            let terminal_branching = matches!(
                body.last(),
                Some(Stmt::If(statement))
                    if statement
                        .elif_else_clauses
                        .iter()
                        .any(|clause| clause.test.is_some())
                        && (statement
                            .elif_else_clauses
                            .last()
                            .is_some_and(|clause| clause.test.is_some())
                            || suite_terminates(&statement.body)
                            || statement
                                .elif_else_clauses
                                .iter()
                                .any(|clause| suite_terminates(&clause.body))
                            || !std::iter::once(statement.body.as_slice())
                                .chain(
                                    statement
                                        .elif_else_clauses
                                        .iter()
                                        .map(|clause| clause.body.as_slice()),
                                )
                                .all(|body| matches!(body, [Stmt::Assign(_)])))
            );
            if terminal_if || terminal_branching {
                let (last, leading) = body.split_last().expect("with body has a final if");
                let Stmt::If(statement) = last else {
                    unreachable!("terminal with condition must be an if statement");
                };
                self.compile_suite(leading)?;
                self.with_control_override(
                    |control| &mut control.terminal_with_condition,
                    Some(statement.test.range()),
                    |compiler| {
                        compiler.with_control_override(
                            |control| &mut control.exclude_terminal_if_not_taken,
                            true,
                            |compiler| compiler.compile_statement(last),
                        )
                    },
                )?;
            } else if let Some((last @ Stmt::Try(statement), leading)) = body.split_last()
                && statement.handlers.is_empty()
                && statement.orelse.is_empty()
                && !suite_terminates(&statement.body)
                && !suite_terminates(&statement.finalbody)
                && !suite_terminates(leading)
                && statement.finalbody.iter().any(|statement| {
                    !matches!(statement, Stmt::Pass(_))
                        && !matches!(
                            statement,
                            Stmt::Expr(expression) if fold_constant(&expression.value).is_some()
                        )
                })
            {
                self.compile_suite(leading)?;
                self.with_control_override(
                    |control| &mut control.preserve_next_finally_exit_nop,
                    true,
                    |compiler| compiler.compile_statement(last),
                )?;
            } else if !matches!(body, [Stmt::Pass(_)]) {
                // Keep direct pass-only bodies on the with compiler's fallback path. CPython
                // retains that NOP inside the protected range, unlike a terminal pass in a
                // nested suite such as a loop `else`.
                self.compile_suite(body)?;
            }
            let trailing_nop_is_covered = body_previous_location.is_some_and(|previous| {
                previous.line >= 0
                    && self
                        .assembler
                        .last_instruction_location()
                        .is_some_and(|last| last.line == previous.line)
            }) && self.take_trailing_nop_location().is_some();
            if !trailing_nop_is_covered
                && self.assembler.instruction_count() == body_start
                && let Some(statement) = body.last()
                // A same-line `as` target store already carries the pass line event.
                && body_previous_location.is_none_or(|previous| {
                    previous.line != self.source_location(statement.range()).line
                })
            {
                body_noop = Some(WithBodyExitNoop::Located(statement.range()));
            } else if terminal_if
                && let Some(Stmt::If(statement)) = body.last()
                && (condition_jump_target_count(&statement.test, false) == 1
                    || self.control.direct_final_sync_with_try_body.is_none())
            {
                // On the direct normal edge of a surrounding try, CPython drops a converted
                // `POP_BLOCK` marker fed by multiple condition fallthroughs. A sole predecessor
                // still supplies the marker's visible location.
                body_noop = Some(WithBodyExitNoop::Located(condition_fallthrough_range(
                    &statement.test,
                )));
            } else if let Some(Stmt::Assert(statement)) = body.last() {
                if condition_jump_target_count(&statement.test, true) == 1 {
                    // CPython converts the with block's trailing `POP_BLOCK` to an unlocated
                    // NOP. Its sole successful assert predecessor supplies the visible line.
                    body_noop = Some(WithBodyExitNoop::ConvertedPopBlock);
                }
            }
        } else {
            self.compile_with_items(remaining, body, false, false)?;
        }
        Ok(body_noop)
    }

    fn compile_with_items(
        &mut self,
        items: &[ruff_python_ast::WithItem],
        body: &[Stmt],
        terminal: bool,
        exclude_handler_exit: bool,
    ) -> Result<(), CompileError> {
        let Some((item, remaining)) = items.split_first() else {
            return self.compile_suite(body);
        };
        let base_depth = self.control.depth;
        let protected_start = self.assembler.label();
        let protected_end = self.assembler.label();
        let handler = self.assembler.label();
        let suppress = self.assembler.label();
        let handler_end = self.assembler.label();
        let cleanup = self.assembler.label();
        let end = self.assembler.label();
        self.control.max_depth = self.control.max_depth.max((base_depth + 7).cast_unsigned());
        let newly_initialized_targets =
            item.optional_vars
                .as_deref()
                .map_or_else(Vec::new, |target| {
                    let mut names = Vec::new();
                    collect_target_names(target, &mut names);
                    names.retain(|name| !self.is_initialized(name));
                    names
                });

        self.compile_expression(&item.context_expr)?;
        self.assembler
            .set_location(self.source_location(item.context_expr.range()));
        self.emit(COPY, 1, 1)?;
        self.emit_special_method(SpecialMethod::__EXIT__)?;
        self.emit(SWAP, 2, 0)?;
        self.emit(SWAP, 3, 0)?;
        self.emit_special_method(SpecialMethod::__ENTER__)?;
        self.emit(CALL, 0, -1)?;
        self.assembler.mark(protected_start);
        if let Some(target) = &item.optional_vars {
            self.compile_store_target(target)?;
        } else {
            self.emit(POP_TOP, 0, -1)?;
        }
        let context = WithExitContext {
            location: self.source_location(item.context_expr.range()),
            is_async: false,
            exception_region_depth: self.control.active_exception_region_exclusions.len(),
            loop_depth: self.control.loops.len(),
            preserves_none_condition_owner: remaining.is_empty()
                && with_suite_has_fallthrough_handler_try(body),
        };
        let (body_noop, region_exclusions) = self.with_region_exclusion_collector(
            |control| &mut control.active_with_region_exclusions,
            |compiler| {
                compiler.with_with_context(context, terminal, |compiler| {
                    compiler.compile_with_body(remaining, body)
                })
            },
        )?;
        self.assembler.mark(protected_end);
        if !remaining.is_empty() && suite_terminates(body) {
            // A nested manager can suppress the terminal body's exception. CPython keeps the
            // nested item's line marker before running this outer manager's normal exit.
            self.assembler
                .set_location(self.source_location(remaining[0].context_expr.range()));
            self.emit(NOP, 0, 0)?;
        }
        let body_noop = body_noop.or_else(|| {
            remaining
                .is_empty()
                .then(|| with_body_handler_exit_location(body))
                .flatten()
                .map(WithBodyExitNoop::LocatedConvertedPopBlock)
                .or_else(|| {
                    (!self.assembler.last_instruction_is_nop()
                        && with_body_ends_in_retained_pop_block(body))
                    .then_some(WithBodyExitNoop::ConvertedPopBlock)
                })
        });
        if let Some(body_noop) = body_noop {
            let noop_start = self.assembler.label();
            self.assembler.mark(noop_start);
            let (location, converted_pop_block) = match body_noop {
                WithBodyExitNoop::Located(range) => (self.source_location(range), false),
                WithBodyExitNoop::LocatedConvertedPopBlock(range) => {
                    (self.source_location(range), true)
                }
                WithBodyExitNoop::ConvertedPopBlock => (SourceLocation::NONE, true),
            };
            self.assembler.set_location(location);
            self.emit(NOP, 0, 0)?;
            if converted_pop_block {
                self.assembler.mark_last_as_converted_pop_block();
            }
            let noop_end = self.assembler.label();
            self.assembler.mark(noop_end);
            self.control
                .generator_region_exclusions
                .push((noop_start, noop_end));
            for exclusions in &mut self.control.active_with_region_exclusions {
                exclusions.push((noop_start, noop_end));
            }
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((noop_start, noop_end));
            }
        }

        self.assembler
            .set_location(self.source_location(item.context_expr.range()));
        let none = self.add_constant(Constant::None)?;
        if !suite_terminates(body) || !remaining.is_empty() {
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(CALL, 3, -4)?;
            self.emit(POP_TOP, 0, -1)?;
            if terminal {
                self.emit_implicit_return()?;
            } else {
                self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
                if self.control.active_finally_try_bodies > 0 {
                    self.assembler.prevent_last_jump_inlining();
                }
            }
        }
        let mut region_start = protected_start;
        for (exclusion_start, exclusion_end) in region_exclusions {
            self.assembler.add_exception_region(
                region_start,
                exclusion_start,
                handler,
                (base_depth + 2).cast_unsigned(),
                true,
            );
            region_start = exclusion_end;
        }
        self.assembler.add_exception_region(
            region_start,
            protected_end,
            handler,
            (base_depth + 2).cast_unsigned(),
            true,
        );

        self.assembler.mark(handler);
        self.set_depth(base_depth + 4);
        self.assembler
            .set_location(self.source_location(item.context_expr.range()));
        self.emit(PUSH_EXC_INFO, 0, 1)?;
        self.emit(WITH_EXCEPT_START, 0, 1)?;
        self.emit(TO_BOOL, 0, 0)?;
        self.emit_jump_forward(POP_JUMP_IF_TRUE, suppress, -1)?;
        self.emit(NOT_TAKEN, 0, 0)?;
        self.emit_reraise(ReraiseStackState::WithHandler)?;
        self.assembler.mark(suppress);
        self.set_depth(base_depth + 5);
        self.emit(POP_TOP, 0, -1)?;
        self.assembler.mark(handler_end);
        self.assembler.add_exception_region(
            handler,
            handler_end,
            cleanup,
            (base_depth + 4).cast_unsigned(),
            true,
        );
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit(POP_TOP, 0, -1)?;
        if terminal {
            self.emit_implicit_return()?;
        } else {
            let outer_exception_exit_start = exclude_handler_exit.then(|| {
                let start = self.assembler.label();
                self.assembler.mark(start);
                start
            });
            self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
            if self.control.active_finally_try_bodies > 0 {
                self.assembler.prevent_last_jump_inlining();
            }
            if let Some(start) = outer_exception_exit_start {
                let exclusion_end = self.assembler.label();
                self.assembler.mark(exclusion_end);
                // Only the direct try omits this owner; an enclosing try still covers the exit.
                if let Some(exclusions) = self.control.active_exception_region_exclusions.last_mut()
                {
                    exclusions.push((start, exclusion_end));
                }
            }
        }

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 3);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(COPY, 3, 1)?;
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_reraise(ReraiseStackState::HandlerCleanup)?;
        self.assembler.mark(end);
        self.set_depth(base_depth);
        for name in newly_initialized_targets {
            let name = self.required_name_id(&name);
            self.symbols.initialized_locals.remove(name);
        }
        Ok(())
    }

    fn compile_async_with(
        &mut self,
        statement: &ruff_python_ast::StmtWith,
    ) -> Result<(), CompileError> {
        if self.unit.flags & (CO_COROUTINE | CO_ASYNC_GENERATOR) == 0 {
            return Err(unsupported("async with outside coroutine code"));
        }
        self.compile_async_with_items(&statement.items, &statement.body, false)
    }

    fn compile_async_with_items(
        &mut self,
        items: &[ruff_python_ast::WithItem],
        body: &[Stmt],
        terminal: bool,
    ) -> Result<(), CompileError> {
        let Some((item, remaining)) = items.split_first() else {
            return self.compile_suite(body);
        };
        let base_depth = self.control.depth;
        let protected_start = self.assembler.label();
        let protected_end = self.assembler.label();
        let handler = self.assembler.label();
        let suppress = self.assembler.label();
        let handler_end = self.assembler.label();
        let cleanup = self.assembler.label();
        let end = self.assembler.label();
        let newly_initialized_targets =
            item.optional_vars
                .as_deref()
                .map_or_else(Vec::new, |target| {
                    let mut names = Vec::new();
                    collect_target_names(target, &mut names);
                    names.retain(|name| !self.is_initialized(name));
                    names
                });

        self.compile_expression(&item.context_expr)?;
        self.assembler
            .set_location(self.source_location(item.context_expr.range()));
        self.emit(COPY, 1, 1)?;
        self.emit_special_method(SpecialMethod::__AEXIT__)?;
        self.emit(SWAP, 2, 0)?;
        self.emit(SWAP, 3, 0)?;
        self.emit_special_method(SpecialMethod::__AENTER__)?;
        self.emit(CALL, 0, -1)?;
        self.compile_awaitable_on_stack(1)?;
        self.assembler.mark(protected_start);
        let context = WithExitContext {
            location: self.source_location(item.context_expr.range()),
            is_async: true,
            exception_region_depth: self.control.active_exception_region_exclusions.len(),
            loop_depth: self.control.loops.len(),
            preserves_none_condition_owner: false,
        };
        let outer_exception_owns_exit = context.exception_region_depth > 0
        // Without the converted end NOP, CPython gives the async-with exit back to the
        // surrounding exception region instead of the coroutine stop handler.
            && remaining.is_empty()
            && matches!(
                body.last(),
                Some(Stmt::For(statement))
                    if statement.is_async
                        && statement.orelse.is_empty()
                        && !suite_contains_loop_break(&statement.body)
            );

        let ((), mut region_exclusions) = self.with_region_exclusion_collector(
            |control| &mut control.active_with_region_exclusions,
            |compiler| {
                if let Some(target) = &item.optional_vars {
                    compiler.compile_store_target(target)?;
                } else {
                    compiler.emit(POP_TOP, 0, -1)?;
                }
                compiler.with_with_context(context, false, |compiler| {
                    let body_start = compiler.assembler.instruction_count();
                    if remaining.is_empty() {
                        if !matches!(body, [Stmt::Pass(_)]) {
                            compiler.compile_suite(body)?;
                        }
                    } else {
                        compiler.compile_async_with_items(remaining, body, false)?;
                    }
                    if remaining.is_empty()
                        && compiler.assembler.instruction_count() == body_start
                        && let Some(statement) = body.last()
                    {
                        compiler
                            .assembler
                            .set_location(compiler.source_location(statement.range()));
                        compiler.emit(NOP, 0, 0)?;
                    }
                    Ok(())
                })
            },
        )?;
        if remaining.is_empty()
            && matches!(body, [Stmt::Expr(expression)] if fold_constant(&expression.value).is_some())
        {
            region_exclusions.clear();
        }
        self.assembler.mark(protected_end);
        if outer_exception_owns_exit {
            self.control
                .generator_region_exclusions
                .push((protected_end, end));
        }

        self.assembler
            .set_location(self.source_location(item.context_expr.range()));
        let none = self.add_constant(Constant::None)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.emit(LOAD_CONST, none, 1)?;
        self.emit(CALL, 3, -4)?;
        self.compile_awaitable_on_stack(2)?;
        self.emit(POP_TOP, 0, -1)?;
        if terminal {
            self.emit_implicit_return()?;
        } else {
            self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
        }
        let mut region_start = protected_start;
        for (exclusion_start, exclusion_end) in region_exclusions {
            self.assembler.add_exception_region(
                region_start,
                exclusion_start,
                handler,
                (base_depth + 2).cast_unsigned(),
                true,
            );
            region_start = exclusion_end;
        }
        self.assembler.add_exception_region(
            region_start,
            protected_end,
            handler,
            (base_depth + 2).cast_unsigned(),
            true,
        );

        self.assembler.mark(handler);
        self.set_depth(base_depth + 4);
        self.assembler
            .set_location(self.source_location(item.context_expr.range()));
        self.emit(PUSH_EXC_INFO, 0, 1)?;
        self.emit(WITH_EXCEPT_START, 0, 1)?;
        self.compile_awaitable_on_stack(2)?;
        self.emit(TO_BOOL, 0, 0)?;
        self.emit_jump_forward(POP_JUMP_IF_TRUE, suppress, -1)?;
        let not_taken_start = self.assembler.label();
        self.assembler.mark(not_taken_start);
        self.emit(NOT_TAKEN, 0, 0)?;
        let not_taken_end = self.assembler.label();
        self.assembler.mark(not_taken_end);
        if !outer_exception_owns_exit {
            self.control
                .generator_region_exclusions
                .push((not_taken_start, not_taken_end));
        }
        for exclusions in &mut self.control.active_with_region_exclusions {
            exclusions.push((not_taken_start, not_taken_end));
        }
        if outer_exception_owns_exit {
            for exclusions in &mut self.control.active_exception_region_exclusions {
                exclusions.push((not_taken_start, not_taken_end));
            }
        }
        self.emit_reraise(ReraiseStackState::WithHandler)?;
        self.assembler.mark(suppress);
        self.set_depth(base_depth + 5);
        self.emit(POP_TOP, 0, -1)?;
        self.assembler.mark(handler_end);
        self.assembler.add_exception_region(
            handler,
            not_taken_start,
            cleanup,
            (base_depth + 4).cast_unsigned(),
            true,
        );
        self.assembler.add_exception_region(
            not_taken_end,
            handler_end,
            cleanup,
            (base_depth + 4).cast_unsigned(),
            true,
        );
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit(POP_TOP, 0, -1)?;
        self.emit(POP_TOP, 0, -1)?;
        if terminal {
            self.emit_implicit_return()?;
        } else {
            self.emit_jump_forward(JUMP_FORWARD, end, 0)?;
        }

        self.assembler.mark(cleanup);
        self.set_depth(base_depth + 3);
        self.assembler.set_location(SourceLocation::NONE);
        self.emit(COPY, 3, 1)?;
        self.emit(POP_EXCEPT, 0, -1)?;
        self.emit_reraise(ReraiseStackState::HandlerCleanup)?;
        self.assembler.mark(end);
        self.set_depth(base_depth);
        for name in newly_initialized_targets {
            let name = self.required_name_id(&name);
            self.symbols.initialized_locals.remove(name);
        }
        Ok(())
    }

    fn compile_import(
        &mut self,
        statement: &ruff_python_ast::StmtImport,
    ) -> Result<(), CompileError> {
        for alias in &statement.names {
            if self.output.constants.is_empty() {
                self.add_constant(Constant::Int(0))?;
            }
            self.emit(LOAD_SMALL_INT, 0, 1)?;
            let none = self.add_constant(Constant::None)?;
            self.emit(LOAD_CONST, none, 1)?;
            let module = alias.name.as_str();
            let module_index = self.name_index(module)?;
            self.emit(IMPORT_NAME, module_index, -1)?;

            if let Some(as_name) = &alias.asname {
                self.compile_import_as(module, as_name.as_str())?;
            } else {
                let bound_name = module.split('.').next().unwrap_or(module);
                self.store_name(bound_name)?;
            }
        }
        Ok(())
    }

    fn compile_import_as(&mut self, module: &str, as_name: &str) -> Result<(), CompileError> {
        let mut parts = module.split('.');
        let _ = parts.next();
        let remaining: Vec<_> = parts.collect();
        if remaining.is_empty() {
            return self.store_name(as_name);
        }

        for (position, part) in remaining.iter().enumerate() {
            let index = self.name_index(part)?;
            self.emit(IMPORT_FROM, index, 1)?;
            if position + 1 < remaining.len() {
                self.emit(SWAP, 2, 0)?;
                self.emit(POP_TOP, 0, -1)?;
            }
        }
        self.store_name(as_name)?;
        self.emit(POP_TOP, 0, -1)
    }

    fn compile_from_import(
        &mut self,
        statement: &ruff_python_ast::StmtImportFrom,
    ) -> Result<(), CompileError> {
        if self.output.constants.is_empty() {
            self.add_constant(Constant::Int(u64::from(statement.level)))?;
        }
        self.emit(LOAD_SMALL_INT, statement.level, 1)?;
        let imported_names = statement
            .names
            .iter()
            .map(|alias| Constant::String(alias.name.as_str().to_string()))
            .collect();
        let star_import = statement.names.len() == 1 && statement.names[0].name.as_str() == "*";
        let names = if star_import {
            self.add_constant(Constant::Tuple(imported_names))?
        } else {
            // `IMPORT_FROM` also places every imported name in `co_names`; CPython reuses those
            // interned string identities in the fromlist tuple, including non-ASCII names.
            self.add_interned_string_tuple(imported_names)?
        };
        self.emit(LOAD_CONST, names, 1)?;
        let module = statement.module.as_ref().map_or("", |name| name.as_str());
        let module_index = self.name_index(module)?;
        self.emit(IMPORT_NAME, module_index, -1)?;

        if star_import {
            self.emit_unary_intrinsic(UnaryIntrinsic::IMPORT_STAR)?;
            self.emit(POP_TOP, 0, -1)?;
            return Ok(());
        }

        for alias in &statement.names {
            let index = self.name_index(alias.name.as_str())?;
            self.emit(IMPORT_FROM, index, 1)?;
            let bound_name = alias
                .asname
                .as_ref()
                .map_or(alias.name.as_str(), |name| name.as_str());
            self.store_name(bound_name)?;
        }
        self.emit(POP_TOP, 0, -1)
    }
}

// Mirrors the edges that `compile_jump_if` emits directly to its caller-provided target. The
// converted with-body `POP_BLOCK` only retains a location when that target has one predecessor.
fn condition_jump_target_count(expression: &Expr, jump_on: bool) -> usize {
    if let Expr::UnaryOp(unary) = expression
        && unary.op == UnaryOp::Not
    {
        return condition_jump_target_count(&unary.operand, !jump_on);
    }
    if let Expr::If(conditional) = expression
        && let Some(truthiness) = early_condition_truthiness(&conditional.test)
    {
        return condition_jump_target_count(
            if truthiness {
                &conditional.body
            } else {
                &conditional.orelse
            },
            jump_on,
        );
    }
    if let Some(truthiness) = jump_constant_truthiness(expression) {
        return usize::from(truthiness == jump_on);
    }
    if let Expr::BoolOp(boolean) = expression {
        let Some((last, leading)) = boolean.values.split_last() else {
            return 0;
        };
        let short_circuit_jumps_to_target = matches!(boolean.op, BoolOp::Or) == jump_on;
        if short_circuit_jumps_to_target {
            return leading
                .iter()
                .chain(std::iter::once(last))
                .map(|value| condition_jump_target_count(value, jump_on))
                .sum();
        }
        return condition_jump_target_count(last, jump_on);
    }
    if let Expr::If(conditional) = expression {
        return condition_jump_target_count(&conditional.body, jump_on)
            + condition_jump_target_count(&conditional.orelse, jump_on);
    }
    if let Expr::Compare(comparison) = expression
        && comparison.ops.len() > 1
        && !jump_on
    {
        return 2;
    }
    1
}

// Returns the expression that contributes the sole edge to a condition's target.
fn unique_condition_jump_target(expression: &Expr, jump_on: bool) -> Option<&Expr> {
    if condition_jump_target_count(expression, jump_on) != 1 {
        return None;
    }

    if let Expr::UnaryOp(unary) = expression
        && unary.op == UnaryOp::Not
    {
        return unique_condition_jump_target(&unary.operand, !jump_on);
    }
    if let Expr::If(conditional) = expression
        && let Some(truthiness) = early_condition_truthiness(&conditional.test)
    {
        return unique_condition_jump_target(
            if truthiness {
                &conditional.body
            } else {
                &conditional.orelse
            },
            jump_on,
        );
    }
    if let Expr::BoolOp(boolean) = expression {
        let (last, leading) = boolean.values.split_last()?;
        let short_circuit_jumps_to_target = matches!(boolean.op, BoolOp::Or) == jump_on;
        if short_circuit_jumps_to_target {
            return leading
                .iter()
                .chain(std::iter::once(last))
                .find(|value| condition_jump_target_count(value, jump_on) == 1)
                .and_then(|value| unique_condition_jump_target(value, jump_on));
        }
        return unique_condition_jump_target(last, jump_on);
    }
    if let Expr::If(conditional) = expression {
        let branch = if condition_jump_target_count(&conditional.body, jump_on) == 1 {
            &conditional.body
        } else {
            &conditional.orelse
        };
        return unique_condition_jump_target(branch, jump_on);
    }
    Some(expression)
}

fn condition_fallthrough_range(expression: &Expr) -> TextRange {
    // A sole false edge carries its contributing operand's range through CPython's CFG;
    // multi-edge joins retain the whole synthetic condition range.
    unique_condition_jump_target(expression, false)
        .map_or_else(|| synthetic_condition_exit_range(expression), Ranged::range)
}

fn expression_contains_chained_comparison(expression: &Expr) -> bool {
    #[derive(Default)]
    struct Detector {
        found: bool,
    }

    impl<'ast> Visitor<'ast> for Detector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(expression, Expr::Compare(comparison) if comparison.ops.len() > 1) {
                self.found = true;
            } else if !self.found {
                walk_expr(self, expression);
            }
        }
    }

    let mut detector = Detector::default();
    detector.visit_expr(expression);
    detector.found
}

// Returns the final chained comparison that `compile_jump_if` lowers for this condition.
// Only this final comparison can be adjacent to a copied terminal exit after CFG optimization.
fn final_chained_comparison_jump(expression: &Expr) -> Option<TextRange> {
    if let Expr::UnaryOp(unary) = expression
        && unary.op == UnaryOp::Not
    {
        return final_chained_comparison_jump(&unary.operand);
    }
    if let Expr::If(conditional) = expression
        && let Some(truthiness) = early_condition_truthiness(&conditional.test)
    {
        return final_chained_comparison_jump(if truthiness {
            &conditional.body
        } else {
            &conditional.orelse
        });
    }
    if let Expr::BoolOp(boolean) = expression {
        return boolean
            .values
            .last()
            .and_then(final_chained_comparison_jump);
    }
    match expression {
        Expr::Compare(comparison)
            if comparison.ops.len() > 1
                && !comparison
                    .ops
                    .iter()
                    .any(|operator| matches!(operator, CmpOp::Is | CmpOp::IsNot)) =>
        {
            Some(comparison.range)
        }
        _ => None,
    }
}

pub(super) fn suite_terminates(body: &[Stmt]) -> bool {
    body.iter().any(statement_terminates)
}
fn is_direct_return_handler_try(statement: &ruff_python_ast::StmtTry) -> bool {
    !statement.is_star
        && statement.orelse.is_empty()
        && statement.finalbody.is_empty()
        && matches!(statement.body.as_slice(), [Stmt::Return(_)])
        && matches!(
            statement.handlers.as_slice(),
            [handler]
                if handler.as_except_handler().is_some_and(|handler| {
                    matches!(handler.body.as_slice(), [Stmt::Return(_)])
                })
        )
}

fn is_direct_return_raise_handler_try(statement: &ruff_python_ast::StmtTry) -> bool {
    !statement.is_star
        && statement.orelse.is_empty()
        && statement.finalbody.is_empty()
        && matches!(statement.body.as_slice(), [Stmt::Return(_)])
        && matches!(
            statement.handlers.as_slice(),
            [handler]
                if handler.as_except_handler().is_some_and(|handler| {
                    matches!(
                        handler.type_.as_deref(),
                        Some(Expr::Name(name)) if name.id.as_str() == "Exception"
                    ) && matches!(handler.body.as_slice(), [Stmt::Raise(_)])
                })
        )
}
fn suite_terminates_in_direct_return_handler_chain(body: &[Stmt]) -> bool {
    let [statement] = body else {
        return false;
    };
    match statement {
        Stmt::Return(_) => true,
        Stmt::If(statement) => {
            suite_terminates_in_direct_return_handler_chain(&statement.body)
                && !statement.elif_else_clauses.is_empty()
                && statement
                    .elif_else_clauses
                    .iter()
                    .all(|clause| suite_terminates_in_direct_return_handler_chain(&clause.body))
                && statement
                    .elif_else_clauses
                    .last()
                    .is_some_and(|clause| clause.test.is_none())
        }
        Stmt::Try(statement) => is_direct_return_handler_try(statement),
        _ => false,
    }
}
fn suite_contains_direct_return_handler_try(body: &[Stmt]) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::Try(statement) => is_direct_return_handler_try(statement),
        Stmt::If(statement) => {
            suite_contains_direct_return_handler_try(&statement.body)
                || statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| suite_contains_direct_return_handler_try(&clause.body))
        }
        _ => false,
    })
}

fn suite_is_constant_false_assertion(body: &[Stmt]) -> bool {
    body.len() == 1
        && matches!(
            body.first(),
            Some(Stmt::Assert(assertion))
                if jump_constant_truthiness(&assertion.test) == Some(false)
        )
}

fn if_contains_copied_nested_handler_exit(statement: &ruff_python_ast::StmtIf) -> bool {
    std::iter::once(statement.body.as_slice())
        .chain(
            statement
                .elif_else_clauses
                .iter()
                .map(|clause| clause.body.as_slice()),
        )
        .flatten()
        .any(|statement| {
            matches!(
                statement,
                Stmt::Try(outer)
                    if outer.handlers.iter().any(|handler| {
                        handler.as_except_handler().is_some_and(|handler| {
                            matches!(
                                handler.body.last(),
                                Some(Stmt::Try(nested))
                                    if try_has_fallthrough_body_and_handler(nested)
                            )
                        })
                    })
            )
        })
}

fn try_has_fallthrough_body_and_handler(statement: &ruff_python_ast::StmtTry) -> bool {
    statement.orelse.is_empty()
        && statement.finalbody.is_empty()
        && matches!(statement.body.as_slice(), [Stmt::Assign(_)])
        && matches!(
            statement.handlers.as_slice(),
            [handler]
                if handler.as_except_handler().is_some_and(|handler| {
                    handler.name.is_none()
                        && matches!(
                            handler.body.as_slice(),
                            [Stmt::Expr(_) | Stmt::Assign(_)]
                        )
                })
        )
}

fn is_call_expression_suite(body: &[Stmt]) -> bool {
    matches!(
        body,
        [Stmt::Expr(statement)] if matches!(statement.value.as_ref(), Expr::Call(_))
    )
}

fn is_loop_conversion_try(statement: &ruff_python_ast::StmtTry) -> bool {
    !statement.is_star
        && statement.finalbody.is_empty()
        && is_call_expression_suite(&statement.body)
        && is_call_expression_suite(&statement.orelse)
        && matches!(
            statement.handlers.as_slice(),
            [handler]
                if handler.as_except_handler().is_some_and(|handler| {
                    handler.name.is_none()
                        && matches!(
                            handler.type_.as_deref(),
                            Some(Expr::Tuple(tuple)) if tuple.elts.len() >= 2
                        )
                        && match handler.body.as_slice() {
                            [Stmt::Expr(statement)] => {
                                matches!(statement.value.as_ref(), Expr::Call(_))
                            }
                            [Stmt::Try(statement)] => is_loop_conversion_try(statement),
                            _ => false,
                        }
                })
        )
}

fn try_normal_exit_predecessors(statement: &ruff_python_ast::StmtTry) -> usize {
    let normal_body =
        usize::from(!suite_terminates(&statement.body) && !suite_terminates(&statement.orelse));
    normal_body
        + statement
            .handlers
            .iter()
            .filter(|handler| {
                handler
                    .as_except_handler()
                    .is_some_and(|handler| !suite_terminates(&handler.body))
            })
            .count()
}

// A converted `POP_BLOCK` after a final try normally inherits its location from a sole jump
// predecessor. If a direct return or raise terminates the try body and the only normal predecessor
// instead comes from an exception handler, CPython propagates that handler's fallthrough location
// onto the marker.
fn with_body_handler_exit_location(body: &[Stmt]) -> Option<TextRange> {
    let Some(Stmt::Try(statement)) = body.last() else {
        return None;
    };
    if statement.is_star
        || !statement.finalbody.is_empty()
        || !suite_terminates(&statement.body)
        || !statement
            .body
            .iter()
            .any(|statement| matches!(statement, Stmt::Return(_) | Stmt::Raise(_)))
    {
        return None;
    }

    let mut fallthrough_handlers = statement
        .handlers
        .iter()
        .filter_map(ruff_python_ast::ExceptHandler::as_except_handler)
        .filter(|handler| !suite_terminates(&handler.body));
    let handler = fallthrough_handlers.next()?;
    if fallthrough_handlers.next().is_some() {
        return None;
    }

    match handler.body.last()? {
        Stmt::Assert(statement) => Some(statement.test.range()),
        Stmt::If(statement) => {
            terminating_if_fallthrough_test(statement).map(synthetic_condition_exit_range)
        }
        statement @ (Stmt::Expr(_) | Stmt::Pass(_) | Stmt::Assign(_)) => Some(statement.range()),
        _ => None,
    }
}

// CPython converts the POP_BLOCK after every synchronous with body into an unowned NOP.
// When a final structured statement leaves one normal predecessor, location propagation makes
// that NOP observable before the manager's normal exit.
fn with_body_ends_in_retained_pop_block(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Try(statement)) => {
            statement.finalbody.is_empty()
                && !suite_terminates(&statement.body)
                && !matches!(statement.body.last(), Some(Stmt::If(_)))
                && try_normal_exit_predecessors(statement) == 1
        }
        Some(Stmt::With(statement)) => {
            matches!(
                statement.body.last(),
                Some(Stmt::Return(_) | Stmt::Raise(_))
            )
        }
        _ => false,
    }
}

fn suite_ends_in_retained_try_exit_join(body: &[Stmt]) -> bool {
    let Some(Stmt::If(statement)) = body.last() else {
        return false;
    };
    if let Some(truthiness) = early_condition_truthiness(&statement.test)
        && statement.elif_else_clauses.len() <= 1
        && statement
            .elif_else_clauses
            .first()
            .is_none_or(|clause| clause.test.is_none())
    {
        let selected = if truthiness {
            statement.body.as_slice()
        } else {
            statement
                .elif_else_clauses
                .first()
                .map_or(&[] as &[Stmt], |clause| clause.body.as_slice())
        };
        return suite_ends_in_retained_try_exit_join(selected);
    }
    !statement.elif_else_clauses.is_empty() || terminating_if_fallthrough_test(statement).is_some()
}
// A direct following name return is copied through every normal edge of this try-else chain.
// CPython retains the first nested join's source jump instead of the try's final normal exit.
fn try_else_exit_moves_to_first_nested_join(statement: &ruff_python_ast::StmtIf) -> bool {
    early_condition_truthiness(&statement.test).is_none()
        && statement
            .elif_else_clauses
            .last()
            .is_some_and(|clause| clause.test.is_some())
        && matches!(
            statement.body.last(),
            Some(Stmt::If(nested))
                if !suite_terminates(&nested.body)
                    && nested.elif_else_clauses.last().is_some_and(|clause| {
                        clause.test.is_none() && !suite_terminates(&clause.body)
                    })
        )
}

// A retained try-else exit NOP can still represent a live normal edge. CPython's borrowed-load
// traversal follows that edge when at least one branch falls through the final conditional.
fn suite_retained_try_exit_has_normal_fallthrough(body: &[Stmt]) -> bool {
    let Some(Stmt::If(statement)) = body.last() else {
        return false;
    };
    if let Some(truthiness) = early_condition_truthiness(&statement.test)
        && statement.elif_else_clauses.len() <= 1
        && statement
            .elif_else_clauses
            .first()
            .is_none_or(|clause| clause.test.is_none())
    {
        let selected = if truthiness {
            statement.body.as_slice()
        } else {
            statement
                .elif_else_clauses
                .first()
                .map_or(&[] as &[Stmt], |clause| clause.body.as_slice())
        };
        return suite_retained_try_exit_has_normal_fallthrough(selected);
    }
    if_normal_exit_count(statement) > 0
        && std::iter::once(statement.body.as_slice())
            .chain(
                statement
                    .elif_else_clauses
                    .iter()
                    .map(|clause| clause.body.as_slice()),
            )
            .any(|body| matches!(body.last(), Some(Stmt::Return(_) | Stmt::Raise(_))))
}

// A terminal final branch leaves the try body's normal exit with one direct predecessor. If
// that exit later copies a small return or raise, CPython retains the original jump's location.
fn direct_try_body_exit_location(body: &[Stmt]) -> Option<&Expr> {
    let Some(Stmt::If(statement)) = body.last() else {
        return None;
    };
    if suite_contains_loop_break(&statement.body)
        || statement
            .elif_else_clauses
            .iter()
            .any(|clause| suite_contains_loop_break(&clause.body))
    {
        return None;
    }
    let test = terminating_if_fallthrough_test(statement)?;
    (early_condition_truthiness(&statement.test).is_none())
        .then(|| unique_condition_jump_target(test, false))
        .flatten()
}

// CPython builds a normal try body with different temporary block edges than its `else` suite,
// so the body's final if join survives redundant-jump removal under narrower conditions. Keep
// this predicate separate from `suite_ends_in_retained_try_exit_join`.
fn suite_ends_in_retained_try_body_exit_join(body: &[Stmt]) -> bool {
    let Some(Stmt::If(statement)) = body.last() else {
        return false;
    };
    if let Some(truthiness) = early_condition_truthiness(&statement.test) {
        if truthiness {
            return suite_ends_in_retained_try_body_exit_join(&statement.body);
        }
        for clause in &statement.elif_else_clauses {
            let Some(test) = &clause.test else {
                return !suite_terminates(&clause.body);
            };
            match early_condition_truthiness(test) {
                Some(false) => {}
                Some(true) => {
                    return suite_ends_in_retained_try_body_exit_join(&clause.body);
                }
                None => {
                    return statement
                        .elif_else_clauses
                        .last()
                        .is_some_and(|last| last.test.is_none() && !suite_terminates(&last.body));
                }
            }
        }
        return false;
    }
    statement
        .elif_else_clauses
        .last()
        .is_some_and(|clause| clause.test.is_none() && !suite_terminates(&clause.body))
}

fn if_normal_exit_count(statement: &ruff_python_ast::StmtIf) -> usize {
    usize::from(!suite_terminates(&statement.body))
        + statement
            .elif_else_clauses
            .iter()
            .filter(|clause| !suite_terminates(&clause.body))
            .count()
        + usize::from(
            statement
                .elif_else_clauses
                .last()
                .is_none_or(|clause| clause.test.is_some()),
        )
}

fn is_direct_try_assignment_branch(statement: &ruff_python_ast::StmtIf) -> bool {
    matches!(statement.body.as_slice(), [Stmt::Assign(_)])
        && matches!(
            statement.elif_else_clauses.as_slice(),
            [clause]
                if clause.test.is_none()
                    && matches!(
                        clause.body.as_slice(),
                        [Stmt::Assign(_)] | [Stmt::Assign(_), Stmt::Assign(_)]
                    )
        )
}

fn is_simple_none_default_assignment(test: &Expr, body: &[Stmt]) -> bool {
    let Expr::Compare(comparison) = test else {
        return false;
    };
    if comparison.ops.len() != 1
        || comparison.ops[0] != CmpOp::Is
        || !matches!(comparison.comparators.as_ref(), [Expr::NoneLiteral(_)])
    {
        return false;
    }
    let Expr::Name(name) = comparison.left.as_ref() else {
        return false;
    };
    matches!(
        body,
        [Stmt::Assign(assignment)]
            if matches!(assignment.targets.as_slice(), [Expr::Name(target)] if target.id == name.id)
                && is_literal_constant(&assignment.value)
    )
}

fn is_call_none_default_assignment(test: &Expr, body: &[Stmt]) -> bool {
    let Expr::Compare(comparison) = test else {
        return false;
    };
    if comparison.ops.len() != 1
        || comparison.ops[0] != CmpOp::Is
        || !matches!(comparison.comparators.as_ref(), [Expr::NoneLiteral(_)])
    {
        return false;
    }
    let Expr::Name(name) = comparison.left.as_ref() else {
        return false;
    };
    matches!(
        body,
        [Stmt::Assign(assignment)]
            if matches!(assignment.targets.as_slice(), [Expr::Name(target)] if target.id == name.id)
                && matches!(assignment.value.as_ref(), Expr::Call(_))
    )
}

fn is_membership_assignment_branch(statement: &ruff_python_ast::StmtIf) -> bool {
    is_direct_try_assignment_branch(statement)
        && matches!(
            statement.test.as_ref(),
            Expr::Compare(comparison)
                if comparison.ops.len() == 1 && comparison.ops[0] == CmpOp::In
        )
}

fn is_flattening_yield_branch(statement: &ruff_python_ast::StmtIf) -> bool {
    matches!(
        statement.test.as_ref(),
        Expr::Call(call)
            if matches!(
                call.func.as_ref(),
                Expr::Name(name) if name.id.as_str() == "hasattr"
            )
    ) && matches!(
        statement.body.as_slice(),
        [Stmt::Expr(expression)]
            if matches!(expression.value.as_ref(), Expr::YieldFrom(_))
    ) && matches!(
        statement.elif_else_clauses.as_slice(),
        [clause]
            if clause.test.is_none()
                && matches!(
                    clause.body.as_slice(),
                    [Stmt::Expr(expression)]
                        if matches!(expression.value.as_ref(), Expr::Yield(_))
                )
    )
}
fn is_specialized_none_condition(test: &Expr) -> bool {
    matches!(
        test,
        Expr::Compare(comparison)
            if comparison.ops.len() == 1
                && matches!(comparison.comparators[0], Expr::NoneLiteral(_))
                && matches!(comparison.ops[0], CmpOp::Is | CmpOp::IsNot)
    )
}

fn is_later_bare_raise_fallback_if(statement: &ruff_python_ast::StmtIf) -> bool {
    matches!(statement.test.as_ref(), Expr::Attribute(attribute)
        if attribute.attr.as_str() == "shape")
        && matches!(
            statement.body.as_slice(),
            [Stmt::Raise(statement)] if statement.exc.is_none()
        )
        && matches!(
            statement.elif_else_clauses.as_slice(),
            [first, second]
                if first.test.is_some()
                    && second.test.is_none()
                    && matches!(first.body.as_slice(), [Stmt::Assign(_)])
                    && matches!(second.body.as_slice(), [Stmt::Assign(_)])
        )
}

fn is_none_pass_assignment_branch(statement: &ruff_python_ast::StmtIf) -> bool {
    is_specialized_none_condition(&statement.test)
        && matches!(statement.body.as_slice(), [Stmt::Pass(_)])
        && matches!(
            statement.elif_else_clauses.as_slice(),
            [clause]
                if clause.test.is_none()
                    && matches!(
                        clause.body.as_slice(),
                        [Stmt::Assign(assignment)]
                            if matches!(assignment.targets.as_slice(), [Expr::Attribute(_)])
                                && matches!(assignment.value.as_ref(), Expr::NoneLiteral(_))
                    )
        )
}
fn is_leading_pair_assignment_branch(statement: &ruff_python_ast::StmtIf) -> bool {
    matches!(
        statement.test.as_ref(),
        Expr::Call(call)
            if matches!(call.func.as_ref(), Expr::Name(name) if name.id.as_str() == "issubclass")
    ) && matches!(
        statement.body.as_slice(),
        [Stmt::Assign(first), Stmt::Assign(second)]
            if matches!(first.value.as_ref(), Expr::Call(_))
                && matches!(second.value.as_ref(), Expr::NoneLiteral(_))
    ) && matches!(
        statement.elif_else_clauses.as_slice(),
        [clause]
            if clause.test.is_none()
                && matches!(clause.body.as_slice(), [Stmt::Assign(fallback)]
                    if matches!(fallback.value.as_ref(), Expr::Call(_)))
    )
}

fn is_assignment_pair_branch(statement: &ruff_python_ast::StmtIf) -> bool {
    matches!(statement.body.last(), Some(Stmt::Assign(_)))
        && matches!(statement.elif_else_clauses.as_slice(), [clause]
            if clause.test.is_none()
                && matches!(clause.body.last(), Some(Stmt::Assign(_))))
}

fn assignment_pair_branch_has_prefix(statement: &ruff_python_ast::StmtIf) -> bool {
    statement.body.len() > 1
        || matches!(statement.elif_else_clauses.as_slice(), [clause]
            if clause.body.len() > 1)
}

fn try_requires_legacy_loop_tail_exclusion(statement: &ruff_python_ast::StmtTry) -> bool {
    !statement.is_star
        && statement.finalbody.is_empty()
        && statement.orelse.is_empty()
        && matches!(
            statement.body.last(),
            Some(Stmt::If(statement))
                if statement.elif_else_clauses.is_empty()
                    && !suite_terminates(&statement.body)
        )
}

fn try_requires_loop_tail_inlining(statement: &ruff_python_ast::StmtTry) -> bool {
    !statement.is_star
        && statement.finalbody.is_empty()
        && statement.orelse.is_empty()
        && matches!(
            statement.body.last(),
            Some(Stmt::If(statement)) if if_normal_exit_count(statement) > 1
        )
}

fn try_normal_exit_ends_in_break_only_loop(statement: &ruff_python_ast::StmtTry) -> bool {
    !statement.is_star
        && statement.finalbody.is_empty()
        && has_direct_break_handler(statement)
        && matches!(
            statement.orelse.last(),
            Some(Stmt::While(loop_statement))
                if is_break_only_loop(loop_statement)
                    && loop_statement
                        .body
                        .last()
                        .is_some_and(try_has_conditional_shared_loop_exit)
        )
}

fn has_direct_break_handler(statement: &ruff_python_ast::StmtTry) -> bool {
    statement.handlers.iter().any(|handler| {
        handler
            .as_except_handler()
            .is_some_and(|handler| matches!(handler.body.as_slice(), [Stmt::Break(_)]))
    })
}

fn is_break_only_loop(statement: &ruff_python_ast::StmtWhile) -> bool {
    statement.orelse.is_empty()
        && early_condition_truthiness(&statement.test) == Some(true)
        && suite_contains_loop_break(&statement.body)
}

fn suite_ends_in_nested_break_only_loop(body: &[Stmt]) -> bool {
    matches!(
        body.last(),
        Some(Stmt::While(statement))
            if is_break_only_loop(statement)
                && matches!(
                    statement.body.last(),
                    Some(Stmt::Try(statement))
                        if try_normal_exit_ends_in_break_only_loop(statement)
                )
    )
}

pub(super) fn try_propagates_protected_loop_tail_mode(
    statement: &ruff_python_ast::StmtTry,
) -> bool {
    // CPython carries the protected source-order layout through nested loop-tail branches when
    // every handler contains only assignments before a final raise. Other handler shapes retain
    // the legacy exception-owner layout.
    !statement.handlers.is_empty()
        && statement.handlers.iter().all(|handler| {
            handler.as_except_handler().is_some_and(|handler| {
                let Some((Stmt::Raise(_), leading)) = handler.body.split_last() else {
                    return false;
                };
                leading
                    .iter()
                    .all(|statement| matches!(statement, Stmt::Assign(_)))
            })
        })
}

// A raise inside an inner loop adds an independent protected exit. Keep that exception-region
// layout separate from the normal final-while exit marker.
fn suite_contains_nested_loop_raise(body: &[Stmt]) -> bool {
    #[derive(Default)]
    struct NestedLoopRaiseDetector {
        found: bool,
        loop_depth: usize,
    }

    impl<'ast> Visitor<'ast> for NestedLoopRaiseDetector {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if self.found {
                return;
            }
            match statement {
                Stmt::Raise(_) if self.loop_depth > 0 => {
                    self.found = true;
                    return;
                }
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
                Stmt::For(_) | Stmt::While(_) => {
                    self.loop_depth += 1;
                    walk_stmt(self, statement);
                    self.loop_depth -= 1;
                    return;
                }
                _ => {}
            }
            walk_stmt(self, statement);
        }
    }

    let mut detector = NestedLoopRaiseDetector::default();
    for statement in body {
        detector.visit_stmt(statement);
    }
    detector.found
}

pub(super) fn suite_contains_yield(body: &[Stmt]) -> bool {
    #[derive(Default)]
    struct YieldCollector {
        found: bool,
    }

    impl<'ast> Visitor<'ast> for YieldCollector {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            match statement {
                Stmt::FunctionDef(_) => return,
                Stmt::ClassDef(definition) => {
                    for decorator in &definition.decorator_list {
                        self.visit_expr(&decorator.expression);
                    }
                    if let Some(arguments) = definition.arguments.as_deref() {
                        for argument in &arguments.args {
                            self.visit_expr(argument);
                        }
                        for keyword in &arguments.keywords {
                            self.visit_expr(&keyword.value);
                        }
                    }
                    return;
                }
                _ => {}
            }
            walk_stmt(self, statement);
        }

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

    let mut collector = YieldCollector::default();
    for statement in body {
        collector.visit_stmt(statement);
    }
    collector.found
}

// CPython's cold-block ordering disconnects the normal outer exit when a nested try/except
// ends a handler-side suite and its own handler is more than straight-line assignment or pass.
// Simple assignment/pass handlers stay threaded into the following borrow-reachable block.

fn suite_contains_nested_handler_cold_join(body: &[Stmt]) -> bool {
    #[derive(Default)]
    struct Detector {
        found: bool,
    }

    impl<'ast> Visitor<'ast> for Detector {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if matches!(statement, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                return;
            }
            if !self.found {
                walk_stmt(self, statement);
            }
        }

        fn visit_body(&mut self, body: &'ast [Stmt]) {
            if matches!(
                body.last(),
                Some(Stmt::Try(statement))
                    if statement.handlers.iter().any(|handler| {
                        handler.as_except_handler().is_some_and(|handler| {
                            handler
                                .body
                                .iter()
                                .any(|statement| !matches!(statement, Stmt::Assign(_) | Stmt::Pass(_)))
                        })
                    })
            ) {
                self.found = true;
                return;
            }
            for statement in body {
                self.visit_stmt(statement);
                if self.found {
                    return;
                }
            }
        }
    }

    let mut detector = Detector::default();
    detector.visit_body(body);
    detector.found
}

// A final nested `try`, or a fallthrough `if` without an `else`, leaves CPython's normal
// try-exit block empty after redundant jumps are removed.
fn suite_ends_in_empty_try_exit_block(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Try(_)) => !body[..body.len() - 1].iter().any(|statement| {
            let Stmt::If(statement) = statement else {
                return false;
            };
            let has_terminal_branch = suite_terminates(&statement.body)
                || statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| suite_terminates(&clause.body));
            let all_branches_terminate = suite_terminates(&statement.body)
                && !statement.elif_else_clauses.is_empty()
                && statement
                    .elif_else_clauses
                    .iter()
                    .all(|clause| suite_terminates(&clause.body))
                && statement
                    .elif_else_clauses
                    .last()
                    .is_some_and(|clause| clause.test.is_none());
            has_terminal_branch && !all_branches_terminate
        }),
        Some(Stmt::If(statement)) => {
            statement.elif_else_clauses.is_empty()
                && !suite_terminates(&statement.body)
                && !statement
                    .body
                    .last()
                    .is_some_and(statement_threads_through_empty_join)
        }
        _ => false,
    }
}

// CPython threads these nested compound-statement exits through a following empty `if` join.
// That gives borrowed-load traversal another route to the suite after the enclosing `try`.
fn statement_threads_through_empty_join(statement: &Stmt) -> bool {
    match statement {
        Stmt::Try(_) | Stmt::With(_) => true,
        Stmt::If(statement) => {
            !statement.elif_else_clauses.is_empty()
                && std::iter::once(statement.body.as_slice())
                    .chain(
                        statement
                            .elif_else_clauses
                            .iter()
                            .take(statement.elif_else_clauses.len() - 1)
                            .map(|clause| clause.body.as_slice()),
                    )
                    .any(|body| !suite_terminates(body))
        }
        Stmt::For(statement) => suite_contains_loop_break(&statement.body),
        Stmt::While(statement) => suite_contains_loop_break(&statement.body),
        Stmt::Match(statement) => statement
            .cases
            .iter()
            .take(statement.cases.len().saturating_sub(1))
            .any(|case| !suite_terminates(&case.body)),
        _ => false,
    }
}

fn statement_terminates(statement: &Stmt) -> bool {
    match statement {
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return(_) | Stmt::Raise(_) => true,
        Stmt::While(statement) => {
            early_condition_truthiness(&statement.test) == Some(true)
                && !suite_contains_loop_break(&statement.body)
        }
        Stmt::For(statement) => {
            !suite_contains_loop_break(&statement.body) && suite_terminates(&statement.orelse)
        }
        Stmt::If(statement) => {
            suite_terminates(&statement.body)
                && !statement.elif_else_clauses.is_empty()
                && statement
                    .elif_else_clauses
                    .iter()
                    .all(|clause| suite_terminates(&clause.body))
                && statement
                    .elif_else_clauses
                    .last()
                    .is_some_and(|clause| clause.test.is_none())
        }
        Stmt::Try(statement) => suite_terminates(&statement.finalbody),
        _ => false,
    }
}

pub(super) fn nested_if_exit_location(
    statement: &ruff_python_ast::StmtIf,
    allow_recursive_fallthrough: bool,
) -> Option<NestedIfExitLocation<'_>> {
    if early_condition_truthiness(&statement.test).is_some() {
        return None;
    }
    if allow_recursive_fallthrough
        && !suite_terminates(&statement.body)
        && statement
            .elif_else_clauses
            .iter()
            .all(|clause| !suite_terminates(&clause.body))
        && let Some(clause) = statement.elif_else_clauses.last()
        && clause.test.is_none()
        && matches!(
            clause.body.last(),
            Some(Stmt::If(tail))
                if matches!(
                    nested_if_exit_location(tail, allow_recursive_fallthrough),
                    Some(NestedIfExitLocation::Unlocated)
                )
        )
    {
        return Some(NestedIfExitLocation::Unlocated);
    }
    let (test, body) = if let Some(clause) = statement.elif_else_clauses.last() {
        if clause.test.is_none() {
            if !matches!(
                statement.elif_else_clauses.as_slice(),
                [first, second]
                    if matches!(first.test.as_ref(), Some(Expr::Call(_)))
                        && second.test.is_none()
            ) {
                return None;
            }

            return clause.body.last().and_then(|statement| match statement {
                Stmt::If(nested)
                    if matches!(nested.test.as_ref(), Expr::Name(_))
                        && matches!(nested.body.last(), Some(Stmt::If(_)))
                        && matches!(
                            nested_if_exit_location(nested, allow_recursive_fallthrough),
                            Some(NestedIfExitLocation::Unlocated)
                        ) =>
                {
                    Some(NestedIfExitLocation::Unlocated)
                }
                _ => None,
            });
        }
        (clause.test.as_ref().unwrap(), clause.body.as_slice())
    } else {
        (statement.test.as_ref(), statement.body.as_slice())
    };
    if early_condition_truthiness(test).is_some() {
        return None;
    }
    Some(
        if suite_terminates(body)
            || matches!(body.last(), Some(Stmt::Try(statement)) if statement.finalbody.is_empty())
        {
            NestedIfExitLocation::Located(test)
        } else {
            NestedIfExitLocation::Unlocated
        },
    )
}

fn suite_unary_not_fallthrough_range(body: &[Stmt]) -> Option<TextRange> {
    match body.last()? {
        Stmt::Assert(statement) if early_condition_truthiness(&statement.test).is_none() => {
            unary_not_condition_exit_range(&statement.test)
        }
        Stmt::If(statement) => {
            terminating_if_fallthrough_test(statement).and_then(unary_not_condition_exit_range)
        }
        _ => None,
    }
}

fn suite_assert_success_fallthrough_range(body: &[Stmt]) -> Option<TextRange> {
    let Some(Stmt::Assert(statement)) = body.last() else {
        return None;
    };
    // Only a unique success edge can supply one stable location to a shared branch exit.
    unique_condition_jump_target(&statement.test, true).map(Ranged::range)
}

fn terminating_if_fallthrough_test(statement: &ruff_python_ast::StmtIf) -> Option<&Expr> {
    if !suite_terminates(&statement.body) {
        return None;
    }
    let mut final_test = statement.test.as_ref();
    for clause in &statement.elif_else_clauses {
        let test = clause.test.as_ref()?;
        if !suite_terminates(&clause.body) {
            return None;
        }
        final_test = test;
    }
    Some(final_test)
}

fn terminal_exception_handler_if_supported(statement: &ruff_python_ast::StmtIf) -> bool {
    std::iter::once(statement.body.as_slice())
        .chain(
            statement
                .elif_else_clauses
                .iter()
                .map(|clause| clause.body.as_slice()),
        )
        .all(|body| {
            !suite_terminates(body)
                || matches!(body, [Stmt::Return(statement)] if statement.value.is_none())
        })
}

fn terminal_try_distributes_implicit_return(statement: &Stmt) -> bool {
    match statement {
        Stmt::If(_) | Stmt::Match(_) | Stmt::While(_) => true,
        Stmt::Expr(statement) => match statement.value.as_ref() {
            Expr::If(_) => true,
            Expr::BoolOp(boolean) => terminal_try_bool_expression_supported(boolean),
            Expr::Compare(comparison) => comparison.ops.len() > 1,
            _ => false,
        },
        _ => false,
    }
}

fn terminal_try_bool_expression_supported(expression: &ruff_python_ast::ExprBoolOp) -> bool {
    if expression.values.last().is_some_and(|value| match value {
        Expr::If(_) => true,
        Expr::Compare(comparison) => comparison.ops.len() > 1,
        _ => false,
    }) {
        return false;
    }
    expression.values.iter().all(|value| match value {
        Expr::BoolOp(nested) => terminal_try_bool_expression_supported(nested),
        _ => jump_constant_truthiness(value).is_none(),
    })
}

pub(super) fn statement_uses_implicit_return_location(statement: &Stmt) -> bool {
    matches!(statement, Stmt::Assign(assignment) if assignment.targets.len() > 1 || matches!(assignment.targets.last(), Some(Expr::List(_) | Expr::Tuple(_))))
        || matches!(statement, Stmt::Assign(assignment) if matches!(assignment.value.as_ref(), Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_)))
        || matches!(statement, Stmt::Expr(expression) if fold_constant(&expression.value).is_some())
        || matches!(
            statement,
            Stmt::AnnAssign(assignment)
                if assignment.value.is_none()
                    && matches!(assignment.target.as_ref(), Expr::Subscript(_))
        )
        || matches!(statement, Stmt::While(statement) if statement.orelse.is_empty())
        || matches!(
            statement,
            Stmt::For(statement)
                if statement.orelse.is_empty()
                    && !suite_contains_loop_break(&statement.body)
        )
}
pub(super) fn is_wildcard_pattern(pattern: &Pattern) -> bool {
    matches!(
        pattern,
        Pattern::MatchAs(pattern) if pattern.pattern.is_none() && pattern.name.is_none()
    )
}

fn suite_contains_loop_break(body: &[Stmt]) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::Break(_) => true,
        Stmt::If(statement) => {
            suite_contains_loop_break(&statement.body)
                || statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| suite_contains_loop_break(&clause.body))
        }
        Stmt::Try(statement) => {
            suite_contains_loop_break(&statement.body)
                || statement
                    .handlers
                    .iter()
                    .filter_map(ruff_python_ast::ExceptHandler::as_except_handler)
                    .any(|handler| suite_contains_loop_break(&handler.body))
                || suite_contains_loop_break(&statement.orelse)
                || suite_contains_loop_break(&statement.finalbody)
        }
        Stmt::With(statement) => suite_contains_loop_break(&statement.body),
        Stmt::Match(statement) => statement
            .cases
            .iter()
            .any(|case| suite_contains_loop_break(&case.body)),
        Stmt::For(statement) => suite_contains_loop_break(&statement.orelse),
        Stmt::While(statement) => suite_contains_loop_break(&statement.orelse),
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => false,
        _ => false,
    })
}

fn try_handler_starts_with_optimized_boolean_generator(
    statement: &ruff_python_ast::StmtTry,
) -> bool {
    statement.handlers.iter().any(|handler| {
        handler.as_except_handler().is_some_and(|handler| {
            handler_ownerless_optimized_boolean_generator_test(&handler.body).is_some()
        })
    })
}

fn handler_ownerless_optimized_boolean_generator_test(body: &[Stmt]) -> Option<TextRange> {
    let Some(Stmt::If(statement)) = body.first() else {
        return None;
    };
    let Expr::Call(call) = statement.test.as_ref() else {
        return None;
    };
    (matches!(optimized_generator_callable(call), Some("all" | "any"))
        && suite_contains_nested_handler_cold_join(&statement.body))
    .then(|| statement.test.range())
}
fn suite_contains_loop_continue(body: &[Stmt]) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::Continue(_) => true,
        Stmt::If(statement) => {
            suite_contains_loop_continue(&statement.body)
                || statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| suite_contains_loop_continue(&clause.body))
        }
        Stmt::Try(statement) => {
            suite_contains_loop_continue(&statement.body)
                || statement
                    .handlers
                    .iter()
                    .filter_map(ruff_python_ast::ExceptHandler::as_except_handler)
                    .any(|handler| suite_contains_loop_continue(&handler.body))
                || suite_contains_loop_continue(&statement.orelse)
                || suite_contains_loop_continue(&statement.finalbody)
        }
        Stmt::With(statement) => suite_contains_loop_continue(&statement.body),
        Stmt::Match(statement) => statement
            .cases
            .iter()
            .any(|case| suite_contains_loop_continue(&case.body)),
        Stmt::For(statement) => suite_contains_loop_continue(&statement.orelse),
        Stmt::While(statement) => suite_contains_loop_continue(&statement.orelse),
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => false,
        _ => false,
    })
}

fn suite_contains_borrow_reachable_conditional_loop_break(body: &[Stmt]) -> bool {
    fn has_reachable_explicit_fallthrough(statement: &ruff_python_ast::StmtIf) -> bool {
        for clause in &statement.elif_else_clauses {
            let falls_through = !suite_terminates(&clause.body);
            let Some(test) = &clause.test else {
                return falls_through;
            };
            match early_condition_truthiness(test) {
                Some(false) => {}
                Some(true) => return falls_through,
                None if falls_through => return true,
                None => {}
            }
        }
        false
    }

    fn contains(body: &[Stmt], conditional: bool) -> bool {
        for statement in body {
            let found = match statement {
                Stmt::Break(_) => conditional,
                Stmt::If(statement) => {
                    let conditional =
                        conditional || early_condition_truthiness(&statement.test).is_none();
                    contains(&statement.body, conditional)
                        || statement.elif_else_clauses.iter().any(|clause| {
                            let conditional = conditional
                                || clause
                                    .test
                                    .as_ref()
                                    .is_some_and(|test| early_condition_truthiness(test).is_none());
                            contains(&clause.body, conditional)
                        })
                }
                Stmt::Try(statement)
                    if !statement.is_star
                        && statement.orelse.is_empty()
                        && statement.finalbody.is_empty() =>
                {
                    if contains(&statement.body, conditional) {
                        true
                    } else {
                        // CPython's borrowed-load traversal can stop at the normal exit of a
                        // protected region, so a later break does not prove that the loop exit
                        // is visited.
                        return false;
                    }
                }
                Stmt::Try(_) => return false,
                Stmt::With(statement) if !statement.is_async => {
                    if contains(&statement.body, conditional) {
                        true
                    } else {
                        return false;
                    }
                }
                Stmt::With(_) => return false,
                Stmt::Match(_) | Stmt::For(_) | Stmt::While(_) => false,
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => false,
                _ => false,
            };
            if found {
                return true;
            }
        }
        false
    }

    let Some((last, leading)) = body.split_last() else {
        return false;
    };
    if suite_contains_loop_break(std::slice::from_ref(last))
        && suite_contains_loop_continue(leading)
    {
        return false;
    }
    if contains(leading, false) {
        return true;
    }

    match last {
        // Cleanup preserves the conditional break edge even when the protected statement is the
        // loop tail.
        Stmt::Try(statement)
            if !statement.is_star
                && statement.orelse.is_empty()
                && statement.finalbody.is_empty() =>
        {
            contains(&statement.body, false)
        }
        // A simple final conditional break is disconnected, but CPython retains the shared exit
        // when another branch of the final `if` can fall through.
        Stmt::If(statement)
            if has_reachable_explicit_fallthrough(statement)
                && !statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| suite_contains_loop_break(&clause.body)) =>
        {
            let conditional = early_condition_truthiness(&statement.test).is_none();
            contains(&statement.body, conditional)
        }
        _ => false,
    }
}

fn break_jump_needs_exception_region_exclusion(
    active_finally_try_bodies: usize,
    iterator_cleanup: IteratorCleanup,
    flags: u32,
) -> bool {
    active_finally_try_bodies == 0
        || iterator_cleanup == IteratorCleanup::None
        || flags & CO_GENERATOR == 0
}

fn suite_ends_in_simple_conditional_break(body: &[Stmt]) -> bool {
    matches!(
        body.last(),
        Some(Stmt::If(statement))
            if statement.elif_else_clauses.is_empty()
                && matches!(statement.body.as_slice(), [Stmt::Break(_)])
    )
}

fn suite_is_import_assignment(body: &[Stmt]) -> bool {
    matches!(
        body,
        [first, Stmt::Assign(_)]
            if matches!(first, Stmt::Import(_) | Stmt::ImportFrom(_))
    )
}

fn with_suite_has_fallthrough_handler_try(body: &[Stmt]) -> bool {
    body.iter().any(|statement| {
        let Stmt::Try(statement) = statement else {
            return false;
        };
        !statement.is_star
            && statement.orelse.is_empty()
            && statement.finalbody.is_empty()
            && !statement.handlers.is_empty()
            && !suite_terminates(&statement.body)
            && statement.handlers.iter().all(|handler| {
                handler
                    .as_except_handler()
                    .is_some_and(|handler| !suite_terminates(&handler.body))
            })
    })
}

fn finally_suite_preserves_none_condition_owner(body: &[Stmt]) -> bool {
    matches!(
        body,
        [Stmt::If(statement)]
            if statement.elif_else_clauses.is_empty()
                && matches!(statement.test.as_ref(), Expr::Name(_))
                && matches!(
                    statement.body.as_slice(),
                    [Stmt::Expr(expression)]
                        if matches!(expression.value.as_ref(), Expr::Call(_))
                )
    )
}

fn try_has_named_pair_exception_handler(statement: &ruff_python_ast::StmtTry) -> bool {
    matches!(
        statement.handlers.as_slice(),
        [handler]
            if handler.as_except_handler().is_some_and(|handler| {
                handler.name.is_some()
                    && matches!(
                        handler.type_.as_deref(),
                        Some(Expr::Name(name)) if name.id.as_str() == "Exception"
                    )
                    && matches!(handler.body.as_slice(), [Stmt::Expr(_), Stmt::Expr(_)])
            })
    )
}

fn try_has_conditional_shared_loop_exit(statement: &Stmt) -> bool {
    let Stmt::Try(statement) = statement else {
        return false;
    };
    let normal_exit = if !statement.finalbody.is_empty() {
        statement.finalbody.as_slice()
    } else if !statement.orelse.is_empty() {
        statement.orelse.as_slice()
    } else {
        return false;
    };
    if suite_terminates(&statement.body) {
        return false;
    }
    let Some(Stmt::If(final_if)) = normal_exit.last() else {
        return false;
    };
    if !final_if.elif_else_clauses.is_empty()
        || early_condition_truthiness(&final_if.test).is_some()
        || suite_terminates(&final_if.body)
    {
        return false;
    }
    let direct_expression =
        normal_exit.len() == 1 && matches!(final_if.body.as_slice(), [Stmt::Expr(_)]);
    let nested_condition = normal_exit.len() > 1
        && !statement.is_star
        && has_direct_break_handler(statement)
        && matches!(
            final_if.body.last(),
            Some(Stmt::If(statement))
                if statement.elif_else_clauses.is_empty()
                    && early_condition_truthiness(&statement.test).is_none()
                    && !suite_terminates(&statement.body)
        );
    direct_expression || nested_condition
}

#[cfg(test)]
mod tests {
    use super::{IteratorCleanup, break_jump_needs_exception_region_exclusion};
    use crate::target::code_flags::{CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR};

    #[test]
    fn finally_iterator_break_exclusion_is_synchronous_generator_only() {
        assert!(!break_jump_needs_exception_region_exclusion(
            1,
            IteratorCleanup::Sync,
            CO_GENERATOR,
        ));
        for flags in [0, CO_COROUTINE, CO_ASYNC_GENERATOR] {
            assert!(break_jump_needs_exception_region_exclusion(
                1,
                IteratorCleanup::Sync,
                flags,
            ));
        }
        assert!(break_jump_needs_exception_region_exclusion(
            0,
            IteratorCleanup::Sync,
            CO_GENERATOR,
        ));
        assert!(break_jump_needs_exception_region_exclusion(
            1,
            IteratorCleanup::None,
            CO_GENERATOR,
        ));
    }
}
