use super::{
    CmpOp, Compiler, ConditionState, ControlFlowState, ExceptionState, Expr, FinallyExitContext,
    FinallyState, IteratorCleanup, Label, LoopState, Ranged, Scope, Stmt, TextRange, WithState,
    copied_continue_expression_is_linear,
};
use crate::assembler::HeldAwaitSource;
use crate::target::code_flags::{CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnwindKind {
    OrdinaryAwait,
    ManagerAwait,
}

/// Complete source state, with only the intentional return-stack truncation normalized.
#[derive(PartialEq)]
struct SourceFrontier<'ast> {
    finalizer: FinallyExitContext<'ast>,
    body_identity: (*const Stmt, usize),
    finally: FinallyState<'ast>,
    loops: LoopState,
    exceptions: ExceptionState,
    withs: WithState,
    conditions: ConditionState,
}

pub(super) struct HeldAwaitOrdinaryPopFrontier<'ast> {
    kind: UnwindKind,
    returned: TextRange,
    original: SourceFrontier<'ast>,
    pub(super) source: HeldAwaitSource,
}

impl<'ast> SourceFrontier<'ast> {
    fn capture(
        control: &ControlFlowState<'ast>,
        returned: TextRange,
        finalizer: &FinallyExitContext<'ast>,
        kind: UnwindKind,
    ) -> Option<Self> {
        let [loop_finalizer] = control.finally.active_loop_finally_contexts.as_slice() else {
            return None;
        };
        let [inner_loop] = control.loops.contexts.as_slice() else {
            return None;
        };
        let ordinary_count = match kind {
            UnwindKind::OrdinaryAwait => 3,
            UnwindKind::ManagerAwait => 2,
        };
        let finalizer_record = FinallyExitContext {
            body: &[],
            ..*finalizer
        };
        if finalizer_record
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
            || inner_loop.iterator_cleanup != IteratorCleanup::None
            || inner_loop.has_orelse
            || inner_loop.with_depth != 0
            || inner_loop.finally_end_depth != 0
            || inner_loop.exception_region_depth != 2
            || control.exceptions.ordinary_try_bodies.len() != ordinary_count
            || control
                .exceptions
                .ordinary_try_bodies
                .iter()
                .enumerate()
                .any(|(index, body)| body.depth != 0 || body.exclusion_index != index + 1)
            || control.exceptions.active_try_marker_slots.len() != ordinary_count
            || control.exceptions.active_exception_region_exclusions.len() != ordinary_count + 1
            || !control.exceptions.active_exception_handlers.is_empty()
            || control.exceptions.active_exception_group_handlers != 0
            || !control
                .comprehensions
                .active_comprehension_region_exclusions
                .is_empty()
            || control.finally.active_finally_try_bodies != 1
            || control.finally.protected_try_loop_depths.as_slice() != [(1, 0)]
            || control.finally.innermost_finally_exception_exclusion_depth != Some(1)
            || control.finally.active_normal_finally_bodies != 0
            || control.finally.active_copied_finally_bodies != 0
            || control.finally.active_finally_end_blocks != 0
            || control.finally.active_overriding_finally_returns != 0
            || !control.finally.active_pass_finally_locations.is_empty()
            || !control.finally.preserved_return_value_contexts.is_empty()
            || control.finally.copied_return_finally_preserved_values != 0
            || control.finally.copied_return_finally_loop_depth.is_some()
            || control.finally.copy_return_finally_condition_exit
            || control.finally.copies_terminating_finally_returns
        {
            return None;
        }
        match kind {
            UnwindKind::OrdinaryAwait => {
                if !control.withs.active_with_exits.is_empty()
                    || !control.withs.active_with_region_exclusions.is_empty()
                {
                    return None;
                }
            }
            UnwindKind::ManagerAwait => {
                let [manager] = control.withs.active_with_exits.as_slice() else {
                    return None;
                };
                if !manager.is_async
                    || manager.loop_depth != 1
                    || manager.exception_region_depth != 3
                    || manager.finally_end_depth != 0
                    || manager.normal_finally_depth != 0
                    || manager.normal_exit != super::WithBodyNormalExit::Terminated
                    || manager.terminal_condition_preceded_by_finally
                    || manager.preserves_none_condition_owner
                    || control.withs.active_with_region_exclusions.len() != 1
                {
                    return None;
                }
            }
        }
        let mut labels = vec![
            finalizer.handler,
            inner_loop.continue_label,
            inner_loop.break_label,
        ];
        for body in &control.exceptions.ordinary_try_bodies {
            labels.extend([body.setup, body.original_end, body.target]);
        }
        labels.extend(
            control
                .withs
                .active_with_exits
                .iter()
                .map(|manager| manager.handler),
        );
        if labels
            .iter()
            .enumerate()
            .any(|(at, label)| labels[..at].contains(label))
        {
            return None;
        }
        let mut finally = control.finally.clone();
        finally.active_return_finally_contexts.clear();
        for context in &mut finally.active_loop_finally_contexts {
            context.body = &[];
        }
        Some(Self {
            finalizer: finalizer_record,
            body_identity: (finalizer.body.as_ptr(), finalizer.body.len()),
            finally,
            loops: control.loops.clone(),
            exceptions: control.exceptions.clone(),
            withs: control.withs.clone(),
            conditions: control.conditions.clone(),
        })
    }
}

impl<'ast> HeldAwaitOrdinaryPopFrontier<'ast> {
    pub(super) fn is_manager(&self) -> bool {
        self.kind == UnwindKind::ManagerAwait
    }

    fn unchanged(
        &self,
        control: &ControlFlowState<'ast>,
        returned: TextRange,
        finalizer: &FinallyExitContext<'ast>,
    ) -> bool {
        returned == self.returned
            && control.finally.active_return_finally_contexts.is_empty()
            && SourceFrontier::capture(control, returned, finalizer, self.kind).as_ref()
                == Some(&self.original)
    }

    pub(super) fn ordinary_aliases(
        &self,
        control: &ControlFlowState<'ast>,
        returned: TextRange,
        finalizer: &FinallyExitContext<'ast>,
        boundaries: [Label; 3],
        starts: &[Option<Label>],
    ) -> Option<[Label; 4]> {
        let [Some(a), Some(b), Some(c), Some(d)] = starts else {
            return None;
        };
        if self.kind != UnwindKind::OrdinaryAwait
            || a != b
            || b != c
            || c != d
            || !self.unchanged(control, returned, finalizer)
        {
            return None;
        }
        let aliases = [boundaries[0], boundaries[1], boundaries[2], *a];
        (!aliases
            .iter()
            .enumerate()
            .any(|(at, label)| aliases[..at].contains(label)))
        .then_some(aliases)
    }

    pub(super) fn manager_aliases(
        &self,
        control: &ControlFlowState<'ast>,
        returned: TextRange,
        finalizer: &FinallyExitContext<'ast>,
        general: Label,
        starts: &[Option<Label>],
    ) -> Option<[Label; 2]> {
        let [Some(a), Some(b), Some(c)] = starts else {
            return None;
        };
        (self.kind == UnwindKind::ManagerAwait
            && a == b
            && b == c
            && *a != general
            && self.unchanged(control, returned, finalizer))
        .then_some([*a, general])
    }
}

impl<'ast> Compiler<'ast> {
    /// Selects distinct ordinary-await and manager-await producers before return emission.
    pub(super) fn capture_held_await_ordinary_pops(
        &self,
        statement: &ruff_python_ast::StmtReturn,
        preserve_tos: bool,
        overridden: bool,
    ) -> Option<HeldAwaitOrdinaryPopFrontier<'ast>> {
        if self.context.python_version != crate::PythonVersion::Py314
            || self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != CO_COROUTINE
            || !preserve_tos
            || overridden
        {
            return None;
        }
        let [finalizer] = self
            .control
            .finally
            .active_return_finally_contexts
            .as_slice()
        else {
            return None;
        };
        let kind = match self.control.withs.active_with_exits.as_slice() {
            [] => UnwindKind::OrdinaryAwait,
            [_] => UnwindKind::ManagerAwait,
            _ => return None,
        };
        let linear_await = matches!(statement.value.as_deref(), Some(Expr::Await(awaited))
            if matches!(awaited.value.as_ref(), Expr::Call(_))
                && copied_continue_expression_is_linear(&awaited.value));
        let initialized_fast = |name: &str| {
            self.is_initialized(name)
                && matches!(&self.unit.scope, Scope::Function { indices, free_indices, cells, globals }
                if self.scope_index(indices, name).is_some() && self.scope_index(free_indices, name).is_none()
                    && !self.contains_name(cells, name) && !self.contains_name(globals, name))
        };
        if !(linear_await
            || (kind == UnwindKind::ManagerAwait
                && matches!(statement.value.as_deref(), Some(Expr::Name(name)) if initialized_fast(name.id.as_str()))))
        {
            return None;
        }
        let Some(Stmt::If(guard)) = finalizer.body.first() else {
            return None;
        };
        if !guard.elif_else_clauses.is_empty()
            || !matches!(guard.test.as_ref(), Expr::Compare(comparison)
                if matches!(comparison.first_operand(), Expr::Name(name) if initialized_fast(name.id.as_str()))
                    && matches!(comparison.ops.as_ref(), [CmpOp::Is | CmpOp::IsNot])
                    && matches!(comparison.comparators(), [Expr::NoneLiteral(_)]))
        {
            return None;
        }
        let original = SourceFrontier::capture(&self.control, statement.range(), finalizer, kind)?;
        let source = self.assembler.capture_held_await_source(
            &original
                .exceptions
                .ordinary_try_bodies
                .iter()
                .map(|body| (body.setup, body.target))
                .collect::<Vec<_>>(),
            original.loops.contexts[0].body_start_item,
            original
                .withs
                .active_with_exits
                .first()
                .map(|manager| manager.body_start_item),
        )?;
        Some(HeldAwaitOrdinaryPopFrontier {
            kind,
            returned: statement.range(),
            original,
            source,
        })
    }
}
