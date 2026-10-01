use super::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR, Compiler, Expr, Ranged, Scope, SourceLocation,
    Stmt, condition_has_only_runtime_tests, expression_contains_inline_branch,
    expression_suspends_current_scope, suite_any_statement, suite_terminates,
};

fn plain(expression: &Expr) -> bool {
    !expression_contains_inline_branch(expression) && !expression_suspends_current_scope(expression)
}

fn ordinary_await(statement: &Stmt) -> Option<&Expr> {
    let Stmt::Expr(expression) = statement else {
        return None;
    };
    let Expr::Await(awaited) = expression.value.as_ref() else {
        return None;
    };
    (matches!(awaited.value.as_ref(), Expr::Call(_)) && plain(&awaited.value))
        .then_some(&expression.value)
}

fn ordinary_suite(body: &[Stmt], recovery: bool) -> bool {
    body.iter().all(|statement| match statement {
        Stmt::Assign(assign) => {
            assign
                .targets
                .iter()
                .all(|target| matches!(target, Expr::Name(_)))
                && plain(&assign.value)
        }
        Stmt::Expr(expression) => {
            ordinary_await(statement).is_some()
                || (matches!(expression.value.as_ref(), Expr::Call(_)) && plain(&expression.value))
        }
        Stmt::Assert(assertion) => {
            plain(&assertion.test) && assertion.msg.as_deref().is_none_or(plain)
        }
        Stmt::Raise(raise) => {
            raise.exc.as_deref().is_none_or(plain) && raise.cause.as_deref().is_none_or(plain)
        }
        Stmt::Pass(_) => true,
        Stmt::If(branch) => {
            condition_has_only_runtime_tests(&branch.test)
                && plain(&branch.test)
                && ordinary_suite(&branch.body, recovery)
                && branch.elif_else_clauses.iter().all(|clause| {
                    clause
                        .test
                        .as_ref()
                        .is_none_or(|test| condition_has_only_runtime_tests(test) && plain(test))
                        && ordinary_suite(&clause.body, recovery)
                })
        }
        Stmt::Try(statement) if recovery => {
            !statement.is_star
                && statement.orelse.is_empty()
                && statement.finalbody.is_empty()
                && !statement.handlers.is_empty()
                && ordinary_suite(&statement.body, false)
                && statement.handlers.iter().all(|handler| {
                    handler.as_except_handler().is_some_and(|handler| {
                        handler.name.is_none()
                            && matches!(handler.type_.as_deref(), Some(Expr::Name(_)))
                            && ordinary_suite(&handler.body, false)
                    })
                })
        }
        _ => false,
    })
}

fn call_finalizer(body: &[Stmt]) -> bool {
    !body.is_empty()
        && body.iter().all(|statement| match statement {
            Stmt::Expr(expression) => {
                matches!(expression.value.as_ref(), Expr::Call(_)) && plain(&expression.value)
            }
            Stmt::If(branch) => {
                condition_has_only_runtime_tests(&branch.test)
                    && plain(&branch.test)
                    && call_finalizer(&branch.body)
                    && branch.elif_else_clauses.iter().all(|clause| {
                        clause.test.as_ref().is_none_or(|test| {
                            condition_has_only_runtime_tests(test) && plain(test)
                        }) && call_finalizer(&clause.body)
                    })
            }
            _ => false,
        })
}

fn completing_handler(body: &[Stmt]) -> bool {
    ordinary_suite(body, false)
        && !suite_any_statement(body, |statement| {
            !matches!(statement, Stmt::Assign(_) | Stmt::If(_))
                && !matches!(statement, Stmt::Expr(expression) if matches!(expression.value.as_ref(), Expr::Call(_)))
        })
}

impl Compiler<'_> {
    pub(super) fn retained_normal_finally_entry_source(
        &self,
        statement: &ruff_python_ast::StmtTry,
    ) -> Option<(SourceLocation, SourceLocation)> {
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
            || !ordinary_suite(&statement.body, false)
        {
            return None;
        }
        let [handler] = statement.handlers.as_slice() else {
            return None;
        };
        let handler = handler.as_except_handler()?;
        if handler.name.is_none()
            || !matches!(handler.type_.as_deref(), Some(Expr::Name(_)))
            || suite_terminates(&handler.body)
            || !ordinary_suite(&handler.body, true)
        {
            return None;
        }
        let Stmt::If(branch) = statement.body.last()? else {
            return None;
        };
        let [clause] = branch.elif_else_clauses.as_slice() else {
            return None;
        };
        if clause.test.is_some() {
            return None;
        }
        let first = ordinary_await(branch.body.last()?)?;
        let last = ordinary_await(clause.body.last()?)?;
        let [Stmt::Try(cleanup)] = statement.finalbody.as_slice() else {
            return None;
        };
        let [action] = cleanup.body.as_slice() else {
            return None;
        };
        let [handler] = cleanup.handlers.as_slice() else {
            return None;
        };
        let handler = handler.as_except_handler()?;
        if cleanup.is_star
            || !cleanup.orelse.is_empty()
            || cleanup.finalbody.is_empty()
            || ordinary_await(action).is_none()
            || handler.name.is_none()
            || !matches!(handler.type_.as_deref(), Some(Expr::Name(_)))
            || !completing_handler(&handler.body)
            || !call_finalizer(&cleanup.finalbody)
        {
            return None;
        }
        Some((
            self.source_location(first.range()),
            self.source_location(last.range()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_normal_entry_requires_a_completing_handler_shape() {
        for source in [
            "error = exc\nrecord(error)\n",
            "if error is None:\n    error = exc\nrecord(error)\n",
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(completing_handler(&parsed.syntax().body));
        }
        for source in [
            "assert True\nrecord(error)\n",
            "assert False\nrecord(error)\n",
            "assert condition\nrecord(error)\n",
            "if condition:\n    assert True\nelse:\n    record(error)\n",
            "if condition:\n    record(error)\nelse:\n    assert False\n",
            "if condition:\n    record(error)\nelse:\n    assert condition\n",
            "pass\n",
            "raise\n",
            "await record(error)\n",
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(!completing_handler(&parsed.syntax().body), "{source}");
        }
    }

    #[test]
    fn retained_normal_entry_requires_an_actual_call_finalizer() {
        for source in ["", "pass\n", "result = None\n", "if condition:\n    pass\n"] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(!call_finalizer(&parsed.syntax().body), "{source}");
        }
        for source in [
            "finish(result)\n",
            "if condition:\n    finish(result)\n",
            "if condition:\n    finish(result)\nelse:\n    record(result)\n",
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(call_finalizer(&parsed.syntax().body), "{source}");
        }
    }
}
