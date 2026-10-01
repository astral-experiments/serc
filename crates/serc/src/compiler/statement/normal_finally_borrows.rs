use super::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR, Compiler, Constant, Expr, Ranged, Scope, Stmt,
    condition_has_linear_runtime_leaves, condition_has_only_runtime_tests,
    expression_contains_inline_branch, expression_suspends_current_scope, suite_any_statement,
    suite_contains_abrupt_control_flow, suite_contains_yield, suite_has_linear_borrow_fallthrough,
    suite_has_normal_fallthrough,
};
use crate::PythonVersion;
use crate::target::opcodes::{CALL, CALL_KW};

// The guard's false edge retains the cold handler cleanup. An unguarded terminal
// recovery can instead leave that empty array on the normal finally fallthrough.
fn ends_in_guarded_recovery(body: &[Stmt]) -> bool {
    let Some(Stmt::If(guard)) = body.last() else {
        return false;
    };
    let runtime = |test: &Expr| {
        condition_has_only_runtime_tests(test)
            && condition_has_linear_runtime_leaves(test)
            && !expression_suspends_current_scope(test)
    };
    if !runtime(&guard.test)
        || guard
            .elif_else_clauses
            .iter()
            .any(|clause| clause.test.as_ref().is_some_and(|test| !runtime(test)))
    {
        return false;
    }
    if let Some(clause) = guard.elif_else_clauses.last() {
        return clause.test.is_none() && ends_in_guarded_recovery(&clause.body);
    }
    let Some((Stmt::Try(recovery), prefix)) = guard.body.split_last() else {
        return false;
    };
    !recovery.is_star
        && recovery.orelse.is_empty()
        && recovery.finalbody.is_empty()
        && (prefix.is_empty() || suite_has_linear_borrow_fallthrough(prefix))
        && suite_has_linear_borrow_fallthrough(&recovery.body)
        && matches!(recovery.handlers.as_slice(), [handler]
        if handler.as_except_handler().is_some_and(|handler| {
            handler.name.is_some()
                && handler.type_.as_ref().is_some_and(|ty| {
                    !expression_contains_inline_branch(ty)
                        && !expression_suspends_current_scope(ty)
                })
                && suite_has_linear_borrow_fallthrough(&handler.body)
        }))
}

impl Compiler<'_> {
    pub(super) fn normal_finally_has_original_store_source(
        &self,
        statement: &ruff_python_ast::StmtTry,
    ) -> bool {
        let control = &self.control;
        if self.context.python_version != PythonVersion::Py314
            || self.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != 0
            || statement.is_star
            || !statement.orelse.is_empty()
            || !control.loops.contexts.is_empty()
            || !control.withs.active_with_exits.is_empty()
            || !control.withs.active_with_region_exclusions.is_empty()
            || !control.exceptions.active_exception_handlers.is_empty()
            || control.exceptions.active_exception_group_handlers != 0
            || control.exceptions.active_exception_region_exclusions.len() != 1
            || control.finally.active_finally_try_bodies != 1
            || control.finally.active_normal_finally_bodies != 0
            || control.finally.active_copied_finally_bodies != 0
            || control.finally.active_finally_end_blocks != 0
            || control.finally.active_overriding_finally_returns != 0
            || !self.suite_has_nonempty_borrow_entry(&statement.finalbody)
        {
            return false;
        }
        let [owner] = control.finally.active_return_finally_contexts.as_slice() else {
            return false;
        };
        let body = self.context.finally_body(statement);
        if owner.source_range != statement.range
            || owner.body.as_ptr() != body.as_ptr()
            || owner.body.len() != body.len()
        {
            return false;
        }
        let linear_handler = |handler: &ruff_python_ast::ExceptHandlerExceptHandler| {
            handler.type_.as_deref().is_some_and(|ty| {
                !expression_contains_inline_branch(ty) && !expression_suspends_current_scope(ty)
            }) && handler.name.is_some()
                && !suite_contains_abrupt_control_flow(&handler.body)
                && !suite_contains_yield(&handler.body)
                && suite_has_normal_fallthrough(&handler.body)
        };
        let [handler] = statement.handlers.as_slice() else {
            return false;
        };
        if !handler.as_except_handler().is_some_and(|handler| {
            linear_handler(handler) && ends_in_guarded_recovery(&handler.body)
        }) {
            return false;
        }
        let [Stmt::Try(cleanup)] = statement.finalbody.as_slice() else {
            return false;
        };
        let [Stmt::Expr(action)] = cleanup.body.as_slice() else {
            return false;
        };
        let [recovery] = cleanup.handlers.as_slice() else {
            return false;
        };
        if cleanup.is_star
            || !cleanup.orelse.is_empty()
            || !cleanup.finalbody.is_empty()
            || !matches!(action.value.as_ref(), Expr::Call(call) if matches!(call.func.as_ref(), Expr::Attribute(_)))
            || expression_contains_inline_branch(&action.value)
            || expression_suspends_current_scope(&action.value)
            || !recovery.as_except_handler().is_some_and(|handler| {
                linear_handler(handler) && suite_has_linear_borrow_fallthrough(&handler.body)
            })
        {
            return false;
        }
        let Some((Stmt::Assign(assignment), prefix)) = statement.body.split_last() else {
            return false;
        };
        let Some((Stmt::Expr(discard), prefix)) = prefix.split_last() else {
            return false;
        };
        let Expr::Call(call) = discard.value.as_ref() else {
            return false;
        };
        let [Expr::Name(target)] = assignment.targets.as_slice() else {
            return false;
        };
        if !matches!(assignment.value.as_ref(), Expr::NumberLiteral(_))
            || !suite_has_normal_fallthrough(prefix)
            || suite_any_statement(prefix, |part| {
                matches!(part, Stmt::Return(_) | Stmt::Break(_) | Stmt::Continue(_))
            })
            || suite_contains_yield(prefix)
            || expression_contains_inline_branch(&discard.value)
            || expression_suspends_current_scope(&discard.value)
            || call
                .arguments
                .args
                .iter()
                .any(|arg| matches!(arg, Expr::Starred(_)))
            || call
                .arguments
                .keywords
                .iter()
                .any(|keyword| keyword.arg.is_none())
        {
            return false;
        }
        let Some(Constant::Int(value)) = self.context.fold_constant(&assignment.value) else {
            return false;
        };
        let Ok(value) = u8::try_from(value) else {
            return false;
        };
        let Scope::Function { indices, cells, .. } = &self.unit.scope else {
            return false;
        };
        let name = self.required_name_id(target.id.as_str());
        let Some(&local) = indices.get(&name) else {
            return false;
        };
        if cells.contains(name) {
            return false;
        }
        let Ok(count) = u32::try_from(call.arguments.args.len() + call.arguments.keywords.len())
        else {
            return false;
        };
        self.assembler.current_original_call_store_source(
            (
                if call.arguments.keywords.is_empty() {
                    CALL
                } else {
                    CALL_KW
                },
                count,
                self.source_location(call.range()),
            ),
            (value, self.source_location(assignment.value.range())),
            (local, self.source_location(target.range)),
        )
    }
}
