//! Source certificates for the completed-async / sync-manager copied-finally array.

use super::with_unwind::WithUnwind;
use super::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR, CmpOp, Compiler, ConditionState,
    ControlFlowState, ExceptionState, Expr, FinallyExitContext, Label, LoopState, Ranged, Scope,
    SourceLocation, Stmt, TextRange, WithState,
};

#[derive(PartialEq)]
pub(super) struct CopiedAsyncFinallyFrontier<'ast> {
    finalizer: FinallyExitContext<'ast>,
    body: (*const Stmt, usize),
    returned: TextRange,
    withs: WithState,
    exceptions: ExceptionState,
    conditions: ConditionState,
    loops: LoopState,
    end_recorders: Vec<super::AsyncForEndPopRecorder>,
    generators: super::GeneratorState,
    local: u32,
    local_location: SourceLocation,
    condition_location: SourceLocation,
}

impl<'ast> CopiedAsyncFinallyFrontier<'ast> {
    fn capture(
        control: &ControlFlowState<'ast>,
        finalizer: &FinallyExitContext<'ast>,
        returned: TextRange,
        local: u32,
        local_location: SourceLocation,
        condition_location: SourceLocation,
    ) -> Option<Self> {
        let snapshot = FinallyExitContext {
            body: &[],
            ..*finalizer
        };
        let [loop_finalizer] = control.finally.active_loop_finally_contexts.as_slice() else {
            return None;
        };
        let [ordinary] = control.exceptions.ordinary_try_bodies.as_slice() else {
            return None;
        };
        let finally = &control.finally;
        if !finally.active_return_finally_contexts.is_empty()
            || snapshot
                != (FinallyExitContext {
                    body: &[],
                    ..*loop_finalizer
                })
            || !std::ptr::eq(finalizer.body, loop_finalizer.body)
            || !finalizer.source_range.contains_range(returned)
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
            || ordinary.depth != 0
            || ordinary.exclusion_index != 1
            || control.exceptions.active_try_marker_slots.as_slice() != [None]
            || control.exceptions.active_exception_region_exclusions.len() != 2
            || !control.exceptions.active_exception_handlers.is_empty()
            || control.exceptions.active_exception_group_handlers != 0
            || !control
                .comprehensions
                .active_comprehension_region_exclusions
                .is_empty()
            || !control.loops.contexts.is_empty()
            || finally.copies_terminating_finally_returns
            || finally.copied_finally_tail.is_some()
            || finally.preserve_next_finally_exit_nop
            || finally.capture_next_finally_manager_pop
            || finally.captured_finally_manager_pop.is_some()
            || finally.pending_name_return_borrow_exit.is_some()
            || finally.active_copied_finally_bodies != 0
            || finally.copy_return_finally_condition_exit
            || finally.copied_return_finally_preserved_values != 0
            || finally.copied_return_finally_loop_depth.is_some()
            || !finally.preserved_return_value_contexts.is_empty()
            || finally.active_normal_finally_bodies != 0
            || finally.active_finally_end_blocks != 0
            || !finally.finally_end_region_indices.is_empty()
            || finally.normal_finally_continuation_depths.is_some()
            || finally.exceptional_finally_continuation_depths.is_some()
            || finally.exceptional_finally_condition
            || finally.active_conditional_cleanup_finally_try_bodies != 0
            || finally.active_finally_try_bodies != 1
            || finally.protected_try_loop_depths.as_slice() != [(1, 0)]
            || finally.innermost_finally_exception_exclusion_depth != Some(1)
            || !finally.active_pass_finally_locations.is_empty()
            || finally.active_overriding_finally_returns != 0
            || finally.preserve_finally_break_exit_loop_range.is_some()
            || finally.borrow_reachable_finally_loop_range.is_some()
            || finally.protected_finally_break_owner
            || finally
                .async_for_end_pop_recorders
                .iter()
                .any(|recorder| recorder.origin.is_some())
        {
            return None;
        }
        let managers = &control.withs.active_with_exits;
        if !(2..=3).contains(&managers.len())
            || control.withs.active_with_region_exclusions.len() != managers.len()
            || managers.iter().enumerate().any(|(index, manager)| {
                manager.is_async != (index > 0)
                    || manager.exception_region_depth != 2
                    || manager.loop_depth != 0
                    || manager.finally_end_depth != 0
                    || manager.normal_finally_depth != 0
                    || manager.preserves_none_condition_owner
                    || manager.terminal_condition_preceded_by_finally
            })
        {
            return None;
        }
        let mut labels = vec![
            ordinary.setup,
            ordinary.original_end,
            ordinary.target,
            finalizer.handler,
        ];
        labels.extend(managers.iter().map(|manager| manager.handler));
        if labels
            .iter()
            .enumerate()
            .any(|(index, label)| labels[..index].contains(label))
        {
            return None;
        }
        Some(Self {
            finalizer: snapshot,
            body: (finalizer.body.as_ptr(), finalizer.body.len()),
            returned,
            withs: control.withs.clone(),
            exceptions: control.exceptions.clone(),
            conditions: control.conditions.clone(),
            loops: control.loops.clone(),
            end_recorders: finally.async_for_end_pop_recorders.clone(),
            generators: control.generators.clone(),
            local,
            local_location,
            condition_location,
        })
    }

    pub(super) fn unchanged(
        &self,
        control: &ControlFlowState<'ast>,
        finalizer: &FinallyExitContext<'ast>,
    ) -> bool {
        let Some(mut current) = Self::capture(
            control,
            finalizer,
            self.returned,
            self.local,
            self.local_location,
            self.condition_location,
        ) else {
            return false;
        };
        let mut expected_generators = self.generators.clone();
        let Some(epoch) = expected_generators
            .coroutine_suspension_epoch
            .checked_add(self.withs.active_with_exits.len() - 1)
        else {
            return false;
        };
        expected_generators.coroutine_suspension_epoch = epoch;
        let generators_match = current.generators == expected_generators;
        current.generators.clone_from(&self.generators);
        generators_match && current == *self && control.stack.depth == 1
    }

    pub(super) fn aliases(
        &self,
        control: &ControlFlowState<'ast>,
        finalizer: &FinallyExitContext<'ast>,
        unwinds: &[WithUnwind],
        events: &[crate::assembler::CopiedFinallyManagerEvent],
        starts: &[Option<Label>],
        general: Label,
    ) -> Option<[Label; 2]> {
        let [Some(first), Some(second)] = starts else {
            return None;
        };
        if first != second
            || *first == general
            || !self.unchanged(control, finalizer)
            || unwinds.len() != self.withs.active_with_exits.len()
            || events.len() != unwinds.len()
            || unwinds.iter().enumerate().any(|(order, unwind)| {
                let index = self.withs.active_with_exits.len() - order - 1;
                !unwind.matches_original(index, &self.withs)
                    || !events[order].starts_at(unwind.start)
            })
        {
            return None;
        }
        Some([*first, general])
    }

    pub(super) fn copy_context_unchanged(&self, control: &ControlFlowState<'ast>) -> bool {
        let mut withs = self.withs.clone();
        withs.active_with_exits.clear();
        let mut exceptions = self.exceptions.clone();
        exceptions.active_exception_region_exclusions.clear();
        let finally = &control.finally;
        let mut generators = self.generators.clone();
        let Some(epoch) = generators
            .coroutine_suspension_epoch
            .checked_add(self.withs.active_with_exits.len() - 1)
        else {
            return false;
        };
        generators.coroutine_suspension_epoch = epoch;
        control.withs == withs
            && control.exceptions == exceptions
            && control.conditions == self.conditions
            && control.loops == self.loops
            && finally.active_return_finally_contexts.is_empty()
            && finally.active_loop_finally_contexts.is_empty()
            && finally.active_finally_try_bodies == 0
            && finally.active_copied_finally_bodies == 1
            && finally.copied_return_finally_preserved_values == 1
            && matches!(finally.preserved_return_value_contexts.as_slice(), [context]
                if context.loops == 0 && context.finally_scopes == 0 && context.handlers == 0 && context.managers == 0)
            && !finally.copy_return_finally_condition_exit
            && finally.active_normal_finally_bodies == 0
            && finally.active_finally_end_blocks == 0
            && finally.active_overriding_finally_returns == 0
            && finally.active_pass_finally_locations.is_empty()
            && finally.async_for_end_pop_recorders == self.end_recorders
            && !finally.copies_terminating_finally_returns
            && finally.copied_finally_tail.is_none()
            && !finally.preserve_next_finally_exit_nop
            && !finally.capture_next_finally_manager_pop
            && finally.captured_finally_manager_pop.is_none()
            && finally.pending_name_return_borrow_exit.is_none()
            && finally.copied_return_finally_loop_depth.is_none()
            && finally.finally_end_region_indices.is_empty()
            && finally.normal_finally_continuation_depths.is_none()
            && finally.exceptional_finally_continuation_depths.is_none()
            && !finally.exceptional_finally_condition
            && finally.active_conditional_cleanup_finally_try_bodies == 0
            && finally.protected_try_loop_depths.as_slice() == [(1, 0)]
            && finally.innermost_finally_exception_exclusion_depth == Some(1)
            && finally.preserve_finally_break_exit_loop_range.is_none()
            && finally.borrow_reachable_finally_loop_range.is_none()
            && !finally.protected_finally_break_owner
            && control.generators == generators
            && control.stack.depth == 1
    }

    pub(super) fn record(
        self,
        compiler: &mut Compiler<'ast>,
        events: Vec<crate::assembler::CopiedFinallyManagerEvent>,
        aliases: [Label; 2],
        pop: usize,
    ) {
        let ordinary = self.exceptions.ordinary_try_bodies[0];
        compiler.assembler.record_copied_async_finally_owner(
            events,
            (ordinary.setup, ordinary.original_end, ordinary.target),
            aliases,
            pop,
            self.local,
            self.local_location,
            self.condition_location,
        );
    }
}

impl<'ast> Compiler<'ast> {
    pub(super) fn copied_async_finally_frontier(
        &self,
        finalizer: &FinallyExitContext<'ast>,
        returned: TextRange,
    ) -> Option<CopiedAsyncFinallyFrontier<'ast>> {
        if self.context.python_version != crate::PythonVersion::Py314
            || self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != CO_COROUTINE
        {
            return None;
        }
        let Some(Stmt::If(guard)) = finalizer.body.first() else {
            return None;
        };
        let Expr::Compare(comparison) = guard.test.as_ref() else {
            return None;
        };
        let Expr::Name(name) = comparison.first_operand() else {
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
        if !guard.elif_else_clauses.is_empty()
            || comparison.ops.as_ref() != [CmpOp::IsNot]
            || !matches!(comparison.comparators(), [Expr::NoneLiteral(_)])
            || !self.is_initialized(name.id.as_str())
            || self.scope_index(free_indices, name.id.as_str()).is_some()
            || self.contains_name(cells, name.id.as_str())
            || self.contains_name(globals, name.id.as_str())
        {
            return None;
        }
        CopiedAsyncFinallyFrontier::capture(
            &self.control,
            finalizer,
            returned,
            self.scope_index(indices, name.id.as_str())?,
            self.source_location(name.range),
            self.source_location(guard.test.range()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::statement::{
        OrdinaryTryBodyContext, PreservedReturnValueContext, WithBodyNormalExit, WithExitContext,
    };
    use ruff_text_size::TextSize;

    fn label(index: u32) -> Label {
        Label::from_u32(index)
    }
    fn range(start: u32, end: u32) -> TextRange {
        TextRange::new(TextSize::new(start), TextSize::new(end))
    }

    fn fixture(body: &[Stmt], managers: usize) -> (ControlFlowState<'_>, FinallyExitContext<'_>) {
        let finalizer = FinallyExitContext {
            source_range: range(0, 1000),
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
            handler: label(1),
            depth: 0,
            preserves_conditional_exit_owner: false,
        };
        let mut control = ControlFlowState::default();
        control.finally.active_loop_finally_contexts.push(finalizer);
        control.finally.active_finally_try_bodies = 1;
        control.finally.protected_try_loop_depths.push((1, 0));
        control.finally.innermost_finally_exception_exclusion_depth = Some(1);
        control
            .exceptions
            .ordinary_try_bodies
            .push(OrdinaryTryBodyContext {
                setup: label(2),
                original_end: label(3),
                target: label(4),
                depth: 0,
                exclusion_index: 1,
            });
        control.exceptions.active_try_marker_slots.push(None);
        control.exceptions.active_exception_region_exclusions =
            vec![vec![(label(5), label(6))], vec![(label(7), label(8))]];
        control.generators.coroutine_suspension_epoch = 7;
        control.generators.emitted_yield_value = true;
        for index in 0..managers {
            let label_index = u32::try_from(index).unwrap();
            control.withs.active_with_exits.push(WithExitContext {
                source_range: range(100 + label_index, 200 + label_index),
                location: SourceLocation::new(3, 3, 4, 8),
                handler: label(20 + label_index),
                is_async: index > 0,
                suspension_epoch: index,
                body_start_item: 10 + index,
                terminal_statement_range: (index + 1 == managers).then_some(range(30, 40)),
                normal_exit: WithBodyNormalExit::Unknown,
                terminal_condition_preceded_by_finally: false,
                exception_region_depth: 2,
                finally_end_depth: 0,
                normal_finally_depth: 0,
                loop_depth: 0,
                preserves_none_condition_owner: false,
            });
            control
                .withs
                .active_with_region_exclusions
                .push(vec![(label(30), label(31))]);
        }
        (control, finalizer)
    }

    fn capture<'a>(
        control: &ControlFlowState<'a>,
        finalizer: &FinallyExitContext<'a>,
    ) -> CopiedAsyncFinallyFrontier<'a> {
        CopiedAsyncFinallyFrontier::capture(
            control,
            finalizer,
            range(30, 40),
            0,
            SourceLocation::new(10, 10, 3, 8),
            SourceLocation::new(10, 10, 3, 20),
        )
        .unwrap()
    }

    #[test]
    fn copied_async_finally_binds_full_source_and_actual_suspension_progress() {
        let parsed =
            ruff_python_parser::parse_module("if value is not None:\n    finish(value)\n").unwrap();
        let changes: &[fn(&mut ControlFlowState<'_>)] = &[
            |c| c.withs.active_with_exits[0].body_start_item += 1,
            |c| c.withs.active_with_exits[1].suspension_epoch += 1,
            |c| c.withs.active_with_exits[0].source_range = range(1, 2),
            |c| c.withs.active_with_exits[0].normal_exit = WithBodyNormalExit::Terminated,
            |c| c.withs.active_with_region_exclusions[0].clear(),
            |c| c.exceptions.active_exception_region_exclusions.swap(0, 1),
            |c| c.exceptions.ordinary_try_bodies[0].exclusion_index = 0,
            |c| c.exceptions.ordinary_try_bodies[0].original_end = label(99),
            |c| c.exceptions.active_try_marker_slots.clear(),
            |c| c.exceptions.prevent_try_exit_inlining = !c.exceptions.prevent_try_exit_inlining,
            |c| c.conditions.include_scoped_none_condition_owner = true,
            |c| c.loops.loop_tail_while_condition = true,
            |c| c.generators.coroutine_suspension_epoch += 1,
            |c| {
                c.generators
                    .generator_region_exclusions
                    .push((label(98), label(99)));
            },
            |c| c.finally.active_normal_finally_bodies = 1,
            |c| c.finally.protected_try_loop_depths.clear(),
        ];
        for managers in [2, 3] {
            for (index, change) in changes.iter().enumerate() {
                let (mut control, finalizer) = fixture(parsed.suite(), managers);
                let proof = capture(&control, &finalizer);
                control.generators.coroutine_suspension_epoch += managers - 1;
                control.stack.depth = 1;
                assert!(proof.unchanged(&control, &finalizer));
                change(&mut control);
                assert!(!proof.unchanged(&control, &finalizer), "mutation {index}");
            }
        }
    }

    #[test]
    fn copied_async_finally_requires_exact_intentional_truncation() {
        let parsed =
            ruff_python_parser::parse_module("if value is not None:\n    finish(value)\n").unwrap();
        let changes: &[fn(&mut ControlFlowState<'_>)] = &[
            |c| c.withs.active_with_region_exclusions.clear(),
            |c| {
                c.exceptions
                    .active_exception_region_exclusions
                    .push(Vec::new());
            },
            |c| c.exceptions.ordinary_try_bodies.clear(),
            |c| c.finally.preserved_return_value_contexts[0].managers = 1,
            |c| c.finally.copied_return_finally_preserved_values += 1,
            |c| c.finally.innermost_finally_exception_exclusion_depth = None,
            |c| c.finally.normal_finally_continuation_depths = Some((0, 0, 0)),
            |c| c.finally.protected_try_loop_depths[0].1 = 1,
            |c| c.generators.coroutine_suspension_epoch += 1,
            |c| c.stack.depth += 1,
        ];
        for change in changes {
            let (mut control, finalizer) = fixture(parsed.suite(), 3);
            let proof = capture(&control, &finalizer);
            control.generators.coroutine_suspension_epoch += 2;
            control.stack.depth = 1;
            control.withs.active_with_exits.clear();
            control
                .exceptions
                .active_exception_region_exclusions
                .clear();
            control.finally.active_loop_finally_contexts.clear();
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
            assert!(proof.copy_context_unchanged(&control));
            change(&mut control);
            assert!(!proof.copy_context_unchanged(&control));
        }
    }
}
