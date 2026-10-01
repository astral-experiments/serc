//! The original iterator-return array that carries a copied finalizer SETUP.

use super::{
    CO_ASYNC_GENERATOR, CO_GENERATOR, Compiler, ComprehensionState, ConditionState,
    ControlFlowState, ExceptionState, Expr, FinallyExitContext, FinallyState, GeneratorState,
    IteratorCleanup, Label, LoopState, PreservedReturnValueContext, Scope, SourceLocation,
    StatementState, Stmt, TextRange, WithState,
};

#[derive(PartialEq)]
struct State<'ast> {
    comprehensions_inactive: bool,
    withs: WithState,
    exceptions: ExceptionState,
    finally: FinallyState<'ast>,
    bodies: Vec<(*const Stmt, usize)>,
    conditions: ConditionState,
    loops: LoopState,
    generators: GeneratorState,
    statements: Statements,
}

#[derive(PartialEq)]
struct Statements {
    context: super::StatementContext,
    ranges: [Option<TextRange>; 4],
    flags: [bool; 6],
    scan: Option<crate::assembler::BooleanValueMarkerScan>,
}

impl Statements {
    fn capture(state: &StatementState) -> Self {
        Self {
            context: state.context,
            ranges: [
                state.match_tail_before_case_exit,
                state.while_before_conditional_join,
                state.retained_branch_pass,
                state.terminal_assignment_return_boundary,
            ],
            flags: [
                state.in_terminal_if_branch_suite,
                state.terminal_if_branch_has_siblings,
                state.split_terminal_assert_success,
                state.terminal_if_call_expression,
                state.suppress_split_return_copy,
                state.emitted_fallthrough_return,
            ],
            scan: state.current_return_boolean_value_marker_scan,
        }
    }
}

fn inactive_comprehensions(state: &ComprehensionState) -> bool {
    !state.defer_async_comprehension_restore
        && state.pending_comprehension_restores.is_empty()
        && state.active_comprehension_cleanups.is_empty()
        && state.active_comprehension_local_names.is_empty()
        && state.active_comprehension_region_exclusions.is_empty()
        && !state.in_unfiltered_comprehension
        && !state.suppress_filtered_comprehension_none_owner
        && !state.in_comprehension_element
        && !state.filtered_comprehension_has_root_if_expression
        && state.direct_comprehension_assignment_results.is_empty()
        && !state.exclude_optimized_tuple_guard_from_comprehension
}

impl<'ast> State<'ast> {
    fn capture(control: &ControlFlowState<'ast>) -> Self {
        Self {
            comprehensions_inactive: inactive_comprehensions(&control.comprehensions),
            withs: control.withs.clone(),
            exceptions: control.exceptions.clone(),
            finally: control.finally.clone(),
            bodies: control
                .finally
                .active_return_finally_contexts
                .iter()
                .chain(&control.finally.active_loop_finally_contexts)
                .map(|context| (context.body.as_ptr(), context.body.len()))
                .collect(),
            conditions: control.conditions.clone(),
            loops: control.loops.clone(),
            generators: control.generators.clone(),
            statements: Statements::capture(&control.statements),
        }
    }

    fn truncate_for_copy(&mut self) {
        self.loops.contexts.clear();
        self.exceptions.active_exception_region_exclusions.clear();
        self.finally.active_loop_finally_contexts.clear();
        self.bodies.clear();
        self.finally.active_finally_try_bodies = 0;
        self.finally.active_copied_finally_bodies = 1;
        self.finally.copied_return_finally_preserved_values = 1;
        self.finally
            .preserved_return_value_contexts
            .push(PreservedReturnValueContext {
                loops: 0,
                finally_scopes: 0,
                handlers: 0,
                managers: 0,
            });
    }
}

pub(super) struct RetainedSetupTruthFrontier<'ast> {
    state: State<'ast>,
    finalizer: FinallyExitContext<'ast>,
    body: (*const Stmt, usize),
    start: crate::assembler::RetainedSetupTruthStart,
    local: u32,
    condition: SourceLocation,
    setup: SourceLocation,
    returned: SourceLocation,
}

impl<'ast> RetainedSetupTruthFrontier<'ast> {
    pub(super) fn before_copy(
        mut self,
        control: &ControlFlowState<'ast>,
        finalizer: &FinallyExitContext<'ast>,
        starts: &[Option<Label>],
        general: Label,
        iterator: (Label, Label),
    ) -> Option<Self> {
        let [Some(collector)] = starts else {
            return None;
        };
        if self.state != State::capture(control)
            || self.finalizer
                != (FinallyExitContext {
                    body: &[],
                    ..*finalizer
                })
            || self.body != (finalizer.body.as_ptr(), finalizer.body.len())
            || control.stack.depth != 1
            || [*collector, general, iterator.0, iterator.1]
                .iter()
                .enumerate()
                .any(|(index, label)| {
                    [*collector, general, iterator.0, iterator.1][..index].contains(label)
                })
        {
            return None;
        }
        self.start.bind_unwind(general, iterator, *collector);
        self.state.truncate_for_copy();
        Some(self)
    }

    pub(super) fn copy_unchanged(&self, control: &ControlFlowState<'ast>) -> bool {
        self.state == State::capture(control) && control.stack.depth == 1
    }

    pub(super) fn record(self, compiler: &mut Compiler<'ast>, pop: usize) {
        compiler.assembler.record_retained_setup_truth(
            self.start,
            pop,
            self.local,
            self.condition,
            self.setup,
            self.returned,
        );
    }
}

impl<'ast> Compiler<'ast> {
    pub(super) fn retained_setup_truth_frontier(
        &self,
        finalizer: &FinallyExitContext<'ast>,
        returned: &ruff_python_ast::StmtReturn,
        return_item: usize,
    ) -> Option<RetainedSetupTruthFrontier<'ast>> {
        if self.context.python_version != crate::PythonVersion::Py314
            || self.unit.flags & (CO_GENERATOR | CO_ASYNC_GENERATOR) != 0
        {
            return None;
        }
        let Expr::Name(value) = returned.value.as_deref()? else {
            return None;
        };
        let [Stmt::Try(cleanup)] = finalizer.body else {
            return None;
        };
        let Some(Stmt::If(guard)) = cleanup.body.first() else {
            return None;
        };
        let Expr::Name(condition) = guard.test.as_ref() else {
            return None;
        };
        let Scope::Function {
            indices,
            free_indices,
            cells,
            globals,
        } = &self.unit.scope
        else {
            return None;
        };
        let local = |name: &str| {
            (self.is_initialized(name)
                && self.scope_index(free_indices, name).is_none()
                && !self.contains_name(cells, name)
                && !self.contains_name(globals, name))
            .then(|| self.scope_index(indices, name))
            .flatten()
        };
        let returned_local = local(value.id.as_str())?;
        let local = local(condition.id.as_str())?;
        let control = &self.control;
        let finally = &control.finally;
        let [original] = finally.active_loop_finally_contexts.as_slice() else {
            return None;
        };
        let [iterator] = control.loops.contexts.as_slice() else {
            return None;
        };
        if cleanup.is_star
            || !cleanup.handlers.is_empty()
            || !cleanup.orelse.is_empty()
            || cleanup.finalbody.is_empty()
            || !guard.elif_else_clauses.is_empty()
            || !std::ptr::eq(original.body, finalizer.body)
            || *original != *finalizer
            || !finalizer.source_range.contains_range(returned.range)
            || finalizer.enclosing_finally_try_depth != 0
            || finalizer.pass_finally_depth != 0
            || finalizer.loop_depth != 0
            || finalizer.with_depth != 0
            || finalizer.handler_depth != 0
            || finalizer.finally_end_depth != 0
            || finalizer.try_body_depth != 0
            || finalizer.exception_region_depth != 0
            || finalizer.suspension_exception_depth != 0
            || finalizer.loop_context_depth != 0
            || !finalizer.copy_on_return
            || finalizer.loop_exit_outer_return_depth.is_some()
            || finalizer.depth != 0
            || finalizer.preserves_conditional_exit_owner
            || iterator.iterator_cleanup != IteratorCleanup::Sync
            || iterator.with_depth != 0
            || iterator.finally_end_depth != 0
            || iterator.exception_region_depth != 1
            || !control.withs.active_with_exits.is_empty()
            || !control.withs.active_with_region_exclusions.is_empty()
            || !control.exceptions.ordinary_try_bodies.is_empty()
            || !control.exceptions.active_try_marker_slots.is_empty()
            || !control.exceptions.active_exception_handlers.is_empty()
            || control.exceptions.active_exception_group_handlers != 0
            || control.exceptions.active_exception_region_exclusions.len() != 1
            || !inactive_comprehensions(&control.comprehensions)
            || !finally.active_return_finally_contexts.is_empty()
            || finally.active_copied_finally_bodies != 0
            || finally.active_normal_finally_bodies != 0
            || finally.active_finally_end_blocks != 0
            || finally.active_finally_try_bodies != 1
            || finally.copied_return_finally_preserved_values != 0
            || !finally.preserved_return_value_contexts.is_empty()
            || finally.copied_return_finally_loop_depth.is_some()
            || finally.copy_return_finally_condition_exit
            || finally.copies_terminating_finally_returns
            || finally.copied_finally_tail.is_some()
            || finally.preserve_next_finally_exit_nop
            || finally.capture_next_finally_manager_pop
            || finally.captured_finally_manager_pop.is_some()
            || finally.pending_name_return_borrow_exit.is_some()
            || finally.active_overriding_finally_returns != 0
            || !finally.active_pass_finally_locations.is_empty()
            || !finally.finally_end_region_indices.is_empty()
            || finally.exceptional_finally_continuation_depths.is_some()
            || finally.exceptional_finally_condition
            || finally.active_conditional_cleanup_finally_try_bodies != 0
            || finally.protected_try_loop_depths.as_slice() != [(1, 0)]
            || finally.innermost_finally_exception_exclusion_depth != Some(1)
            || control.stack.depth != 2
        {
            return None;
        }
        let start = self.assembler.capture_retained_setup_truth_start(
            return_item,
            returned_local,
            self.source_location(value.range),
        )?;
        Some(RetainedSetupTruthFrontier {
            state: State::capture(control),
            finalizer: FinallyExitContext {
                body: &[],
                ..*finalizer
            },
            body: (finalizer.body.as_ptr(), finalizer.body.len()),
            start,
            local,
            condition: self.source_location(condition.range),
            setup: self.source_location(cleanup.range),
            returned: self.source_location(returned.range),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_text_size::TextSize;

    fn finalizer(body: &[Stmt]) -> FinallyExitContext<'_> {
        FinallyExitContext {
            source_range: TextRange::new(TextSize::new(0), TextSize::new(1000)),
            body,
            enclosing_finally_try_depth: 0,
            pass_finally_depth: 0,
            loop_depth: 0,
            with_depth: 0,
            handler_depth: 0,
            finally_end_depth: 0,
            try_body_depth: 0,
            exception_region_depth: 0,
            suspension_exception_depth: 0,
            loop_context_depth: 0,
            copy_on_return: true,
            loop_exit_outer_return_depth: None,
            handler: Label::from_u32(1),
            depth: 0,
            preserves_conditional_exit_owner: false,
        }
    }

    #[test]
    fn retained_setup_context_requires_body_identity_not_structural_equality() {
        let first = ruff_python_parser::parse_module("cleanup()\n").unwrap();
        let second = ruff_python_parser::parse_module("cleanup()\n").unwrap();
        for returning in [false, true] {
            let mut control = ControlFlowState::default();
            let contexts = if returning {
                &mut control.finally.active_return_finally_contexts
            } else {
                &mut control.finally.active_loop_finally_contexts
            };
            contexts.push(finalizer(first.suite()));
            let captured = State::capture(&control);
            assert!(captured == State::capture(&control));
            let contexts = if returning {
                &mut control.finally.active_return_finally_contexts
            } else {
                &mut control.finally.active_loop_finally_contexts
            };
            contexts[0].body = second.suite();
            assert_eq!(captured.finally, control.finally);
            assert!(captured != State::capture(&control));
        }
    }

    #[test]
    fn retained_setup_context_truncates_only_the_expected_copy_state() {
        let parsed = ruff_python_parser::parse_module("cleanup()\n").unwrap();
        let changes: &[fn(&mut ControlFlowState<'_>)] = &[
            |control| {
                control.generators.coroutine_suspension_epoch += 1;
            },
            |control| {
                control
                    .exceptions
                    .active_exception_region_exclusions
                    .push(Vec::new());
            },
            |control| {
                control.finally.active_normal_finally_bodies = 1;
            },
            |control| {
                control.finally.active_copied_finally_bodies = 2;
            },
            |control| {
                control.finally.copied_return_finally_preserved_values = 2;
            },
            |control| {
                control.finally.preserved_return_value_contexts[0].handlers = 1;
            },
            |control| {
                control.finally.normal_finally_continuation_depths = None;
            },
            |control| {
                control.finally.protected_try_loop_depths.clear();
            },
            |control| {
                control.conditions.include_scoped_none_condition_owner = true;
            },
            |control| {
                control.statements.suppress_split_return_copy = true;
            },
            |control| {
                control.comprehensions.in_comprehension_element = true;
            },
        ];
        for (index, change) in changes.iter().enumerate() {
            let mut control = ControlFlowState::default();
            control
                .finally
                .active_loop_finally_contexts
                .push(finalizer(parsed.suite()));
            control.finally.active_finally_try_bodies = 1;
            control.finally.protected_try_loop_depths.push((1, 0));
            control.finally.normal_finally_continuation_depths = Some((0, 0, 0));
            control
                .exceptions
                .active_exception_region_exclusions
                .push(Vec::new());
            let mut expected = State::capture(&control);
            expected.truncate_for_copy();
            control.finally.active_loop_finally_contexts.clear();
            control
                .exceptions
                .active_exception_region_exclusions
                .clear();
            control.finally.active_finally_try_bodies = 0;
            control.finally.active_copied_finally_bodies = 1;
            control.finally.copied_return_finally_preserved_values = 1;
            control
                .finally
                .preserved_return_value_contexts
                .push(PreservedReturnValueContext {
                    loops: 0,
                    finally_scopes: 0,
                    handlers: 0,
                    managers: 0,
                });
            assert!(expected == State::capture(&control));
            change(&mut control);
            assert!(expected != State::capture(&control), "mutation {index}");
        }
    }
}
