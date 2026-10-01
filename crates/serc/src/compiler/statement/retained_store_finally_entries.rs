use super::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR, Compiler, Expr, Ranged, Scope, SourceLocation,
    Stmt, Visitor, condition_has_only_runtime_tests, expression_contains_inline_branch,
    expression_suspends_current_scope, suite_contains_yield_with_headers,
    suite_has_linear_borrow_fallthrough, walk_expr,
};

fn unproved_expression(expression: &Expr) -> bool {
    #[derive(Default)]
    struct Detector(bool);
    impl<'ast> Visitor<'ast> for Detector {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            if self.0 {
                return;
            }
            if matches!(
                expression,
                Expr::Named(_)
                    | Expr::Lambda(_)
                    | Expr::ListComp(_)
                    | Expr::SetComp(_)
                    | Expr::DictComp(_)
                    | Expr::Generator(_)
                    | Expr::Await(_)
                    | Expr::Yield(_)
                    | Expr::YieldFrom(_)
            ) {
                self.0 = true;
            } else {
                walk_expr(self, expression);
            }
        }
    }
    let mut detector = Detector::default();
    detector.visit_expr(expression);
    detector.0
}

fn ordinary_work(statement: &Stmt) -> bool {
    (match statement {
        Stmt::Assign(assignment) => {
            !unproved_expression(&assignment.value)
                && assignment
                    .targets
                    .iter()
                    .all(|target| !unproved_expression(target))
        }
        Stmt::Expr(expression) => {
            matches!(expression.value.as_ref(), Expr::Call(_))
                && !unproved_expression(&expression.value)
        }
        _ => false,
    }) && suite_has_linear_borrow_fallthrough(std::slice::from_ref(statement))
        && !suite_contains_yield_with_headers(std::slice::from_ref(statement), true, false)
}

fn manager_work(body: &[Stmt]) -> bool {
    !body.is_empty()
        && !suite_contains_yield_with_headers(body, true, false)
        && body.iter().all(|statement| match statement {
            Stmt::Assign(assignment) => {
                !expression_suspends_current_scope(&assignment.value)
                    && !unproved_expression(&assignment.value)
                    && assignment.targets.iter().all(|target| {
                        !expression_suspends_current_scope(target) && !unproved_expression(target)
                    })
            }
            Stmt::Expr(expression) => {
                matches!(expression.value.as_ref(), Expr::Call(_))
                    && !expression_suspends_current_scope(&expression.value)
                    && !unproved_expression(&expression.value)
            }
            Stmt::If(branch) => {
                condition_has_only_runtime_tests(&branch.test)
                    && !expression_suspends_current_scope(&branch.test)
                    && !unproved_expression(&branch.test)
                    && manager_work(&branch.body)
                    && branch.elif_else_clauses.iter().all(|clause| {
                        clause.test.as_ref().is_none_or(|test| {
                            condition_has_only_runtime_tests(test)
                                && !expression_suspends_current_scope(test)
                                && !unproved_expression(test)
                        }) && manager_work(&clause.body)
                    })
            }
            _ => false,
        })
}

impl Compiler<'_> {
    pub(super) fn retained_store_finally_entry_source(
        &self,
        statement: &ruff_python_ast::StmtTry,
    ) -> Option<(SourceLocation, SourceLocation)> {
        let control = &self.control;
        if self.context.python_version != crate::PythonVersion::Py314
            || !matches!(self.unit.scope, Scope::Function { .. })
            || self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != 0
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
            || suite_contains_yield_with_headers(&statement.body, true, false)
        {
            return None;
        }
        let (Stmt::Assign(assignment), prefix) = statement.body.split_last()? else {
            return None;
        };
        let [Expr::Name(target)] = assignment.targets.as_slice() else {
            return None;
        };
        let Expr::BoolOp(value) = assignment.value.as_ref() else {
            return None;
        };
        if !prefix.iter().all(ordinary_work)
            || value.values.len() != 2
            || value.values.iter().any(|part| {
                self.context.fold_constant(part).is_some()
                    || expression_contains_inline_branch(part)
                    || expression_suspends_current_scope(part)
                    || unproved_expression(part)
            })
        {
            return None;
        }
        let [handler] = statement.handlers.as_slice() else {
            return None;
        };
        let handler = handler.as_except_handler()?;
        if handler.name.is_none()
            || !matches!(handler.type_.as_deref(), Some(Expr::Name(_)))
            || !handler.body.last().is_some_and(ordinary_work)
            || !handler
                .body
                .iter()
                .any(|statement| matches!(statement, Stmt::If(_)))
            || !handler.body.iter().all(|statement| {
                if let Stmt::If(guard) = statement {
                    guard.elif_else_clauses.is_empty()
                        && condition_has_only_runtime_tests(&guard.test)
                        && !expression_suspends_current_scope(&guard.test)
                        && !unproved_expression(&guard.test)
                        && matches!(guard.body.as_slice(), [Stmt::Raise(raise)]
                            if raise.exc.is_none() && raise.cause.is_none())
                } else {
                    ordinary_work(statement)
                }
            })
        {
            return None;
        }
        let (Stmt::With(manager), suffix) = statement.finalbody.split_first()? else {
            return None;
        };
        let [item] = manager.items.as_slice() else {
            return None;
        };
        if manager.is_async
            || item.optional_vars.is_some()
            || !matches!(&item.context_expr, Expr::Attribute(attribute)
                if matches!(attribute.value.as_ref(), Expr::Name(_)))
            || !manager_work(&manager.body)
            || !manager
                .body
                .iter()
                .any(|statement| matches!(statement, Stmt::Expr(_)))
            || !manager_work(suffix)
        {
            return None;
        }
        Some((
            self.source_location(target.range),
            self.source_location(item.context_expr.range()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{manager_work, ordinary_work};

    #[test]
    fn retained_store_work_excludes_unproved_control_flow_and_expression_scopes() {
        for source in ["value = prepare()", "report(value)", "obj.value = left"] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(ordinary_work(&parsed.syntax().body[0]), "{source}");
            assert!(manager_work(&parsed.syntax().body), "{source}");
        }
        for source in [
            "assert False",
            "assert True",
            "assert value",
            "pass",
            "value",
            "value = (result := prepare())",
            "value = [part for part in parts]",
            "value = (part for part in parts)",
            "value = lambda: prepare()",
            "value = await prepare()",
            "value = (yield prepare())",
            "report(result := prepare())",
            "report([part for part in parts])",
            "obj[(result := key)] = value",
            "return value",
            "raise Error()",
            "with manager:\n    report(value)",
            "for value in parts:\n    report(value)",
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(!ordinary_work(&parsed.syntax().body[0]), "{source}");
            assert!(!manager_work(&parsed.syntax().body), "{source}");
        }
        for source in [
            "obj.value = left and right",
            "if value:\n    report(value)",
            "if value:\n    obj.value = left and right\nelse:\n    report(value)",
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(!ordinary_work(&parsed.syntax().body[0]), "{source}");
            assert!(manager_work(&parsed.syntax().body), "{source}");
        }
        for source in [
            "if False:\n    report(value)",
            "if True:\n    report(value)",
            "if value:\n    assert value",
            "if value:\n    pass",
            "if value:\n    report(value)\nelse:\n    raise Error()",
            "if (result := value):\n    report(value)",
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            assert!(!manager_work(&parsed.syntax().body), "{source}");
        }
    }
}
