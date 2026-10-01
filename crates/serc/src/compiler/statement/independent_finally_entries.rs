use super::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR, Compiler, Expr, Ranged, Scope, SourceLocation,
    Stmt, condition_has_only_runtime_tests, expression_contains_inline_branch,
    expression_suspends_current_scope, suite_contains_abrupt_control_flow,
    suite_contains_yield_with_headers,
};

fn plain_await(statement: &Stmt) -> Option<&Expr> {
    let Stmt::Expr(expression) = statement else {
        return None;
    };
    let Expr::Await(awaited) = expression.value.as_ref() else {
        return None;
    };
    (matches!(awaited.value.as_ref(), Expr::Call(_))
        && !expression_contains_inline_branch(&awaited.value)
        && !expression_suspends_current_scope(&awaited.value))
    .then_some(&expression.value)
}

impl Compiler<'_> {
    pub(super) fn independent_finally_entry_source(
        &self,
        statement: &ruff_python_ast::StmtTry,
    ) -> Option<(SourceLocation, SourceLocation, SourceLocation)> {
        let control = &self.control;
        if self.context.python_version != crate::PythonVersion::Py314
            || !matches!(self.unit.scope, Scope::Function { .. })
            || self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != CO_COROUTINE
            || statement.is_star
            || !statement.orelse.is_empty()
            || !control.loops.contexts.is_empty()
            || !control.withs.active_with_exits.is_empty()
            || !control.withs.active_with_region_exclusions.is_empty()
            || !control.exceptions.active_exception_handlers.is_empty()
            || control.exceptions.active_exception_group_handlers != 0
            || !control
                .exceptions
                .active_exception_region_exclusions
                .is_empty()
            || control.finally.active_finally_try_bodies != 0
            || control.finally.active_normal_finally_bodies != 0
            || control.finally.active_copied_finally_bodies != 0
            || control.finally.active_finally_end_blocks != 0
            || control.finally.active_overriding_finally_returns != 0
            || !control
                .comprehensions
                .active_comprehension_cleanups
                .is_empty()
            || !control
                .comprehensions
                .active_comprehension_local_names
                .is_empty()
            || suite_contains_abrupt_control_flow(&statement.body)
            || suite_contains_yield_with_headers(&statement.body, true, false)
        {
            return None;
        }
        let [handler] = statement.handlers.as_slice() else {
            return None;
        };
        let handler = handler.as_except_handler()?;
        let (Stmt::Raise(raise), prefix) = handler.body.split_last()? else {
            return None;
        };
        if handler.name.is_some()
            || !matches!(handler.type_.as_deref(), Some(Expr::Name(_)))
            || raise.exc.is_some()
            || raise.cause.is_some()
            || suite_contains_abrupt_control_flow(prefix)
            || suite_contains_yield_with_headers(prefix, true, false)
        {
            return None;
        }
        let Stmt::If(branch) = statement.body.last()? else {
            return None;
        };
        let last = branch.elif_else_clauses.last()?;
        let runtime = |test: &Expr| {
            condition_has_only_runtime_tests(test)
                && !expression_contains_inline_branch(test)
                && !expression_suspends_current_scope(test)
        };
        if !runtime(&branch.test)
            || !branch
                .elif_else_clauses
                .iter()
                .all(|clause| clause.test.as_ref().is_some_and(runtime))
        {
            return None;
        }
        let awaited = plain_await(branch.body.last()?)?;
        let Stmt::Expr(fallthrough) = last.body.last()? else {
            return None;
        };
        if !matches!(fallthrough.value.as_ref(), Expr::Call(_))
            || expression_contains_inline_branch(&fallthrough.value)
            || expression_suspends_current_scope(&fallthrough.value)
        {
            return None;
        }
        let [Stmt::Try(cleanup)] = statement.finalbody.as_slice() else {
            return None;
        };
        let [action] = cleanup.body.as_slice() else {
            return None;
        };
        if cleanup.is_star
            || cleanup.handlers.is_empty()
            || !cleanup.orelse.is_empty()
            || !cleanup.finalbody.is_empty()
            || plain_await(action).is_none()
        {
            return None;
        }
        Some((
            self.source_location(awaited.range()),
            self.source_location(last.test.as_ref()?.range()),
            self.source_location(fallthrough.value.range()),
        ))
    }
}
