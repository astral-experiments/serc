use super::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR, Compiler, ConditionState,
    ExceptionHandlerContext, ExceptionState, Expr, FinallyExitContext, FinallyState, Label,
    LoopState, NOP, NameId, POP_TOP, Ranged, Scope, Stmt, WithState,
    copied_continue_expression_is_linear,
};
use crate::assembler::NamedFinallyReturnPrefix;
use crate::target::Opcode;
use crate::target::opcodes::DELETE_ATTR;

pub(super) struct NamedFinallyReturnFrontier<'ast> {
    finalizer: FinallyExitContext<'ast>,
    handler: ExceptionHandlerContext,
    exceptions: ExceptionState,
    finally: FinallyState<'ast>,
    withs: WithState,
    loops: LoopState,
    conditions: ConditionState,
    finalizer_exit: Label,
    prefix: NamedFinallyReturnPrefix,
    local: u32,
    cleanup: Option<NamedHandlerCleanup>,
}

struct NamedHandlerCleanup {
    none: u32,
    handler_pop: Label,
    finalizer_exit: Label,
}

impl NamedHandlerCleanup {
    fn outer_pop(&self, starts: &[Option<Label>]) -> Option<Label> {
        match starts {
            [Some(outer), Some(handler), Some(exit)]
                if *handler == self.handler_pop && *exit == self.finalizer_exit =>
            {
                Some(*outer)
            }
            _ => None,
        }
    }
}

fn same_finalizer(left: &FinallyExitContext<'_>, right: &FinallyExitContext<'_>) -> bool {
    std::ptr::eq(left.body, right.body) && left == right
}

fn donor_opcode(body: &[Stmt]) -> Option<Opcode> {
    match body {
        [Stmt::Pass(_)] => Some(NOP),
        [Stmt::Expr(expression)]
            if matches!(expression.value.as_ref(), Expr::Call(_))
                && copied_continue_expression_is_linear(&expression.value) =>
        {
            Some(POP_TOP)
        }
        [Stmt::Delete(statement)]
            if matches!(statement.targets.as_slice(), [Expr::Attribute(attribute)]
            if copied_continue_expression_is_linear(&attribute.value)) =>
        {
            Some(DELETE_ATTR)
        }
        _ => None,
    }
}

impl Compiler<'_> {
    fn named_finally_return_local(&self, name: NameId) -> Option<u32> {
        let Scope::Function {
            indices,
            globals,
            cells,
            free_indices,
        } = &self.unit.scope
        else {
            return None;
        };
        let symbol = self.instruction_name_id(self.resolve_name(name), name);
        if globals.contains(symbol)
            || cells.contains(symbol)
            || free_indices.contains_key(&symbol)
            || self.symbols.temporary_indices.contains_key(&symbol)
            || self.symbols.active_temporaries.contains(symbol)
            || self.symbols.active_temporary_fast_cells.contains(symbol)
            || self.symbols.active_temporary_free_cells.contains(symbol)
        {
            return None;
        }
        indices.get(&symbol).copied()
    }
}

impl<'ast> NamedFinallyReturnFrontier<'ast> {
    pub(super) fn capture(
        compiler: &Compiler<'ast>,
        finalizer: FinallyExitContext<'ast>,
        starts: &[Option<Label>],
    ) -> Option<Self> {
        let control = &compiler.control;
        let [handler] = control.exceptions.active_exception_handlers.as_slice() else {
            return None;
        };
        let [None, None, Some(finalizer_exit)] = starts else {
            return None;
        };
        let finally = &control.finally;
        let opcode = donor_opcode(finalizer.body)?;
        let pass_locations = if opcode == NOP {
            vec![compiler.source_location(finalizer.body[0].range())]
        } else {
            Vec::new()
        };
        if compiler.context.python_version < crate::PythonVersion::Py314
            || compiler.unit.flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR) != 0
            || finalizer.depth != 1
            || finalizer.handler_depth != 1
            || finalizer.loop_depth != 0
            || finalizer.with_depth != 0
            || finalizer.try_body_depth != 1
            || finalizer.exception_region_depth != 2
            || finalizer.enclosing_finally_try_depth != 0
            || finalizer.pass_finally_depth != 0
            || finalizer.finally_end_depth != 0
            || finalizer.loop_context_depth != 0
            || finalizer.suspension_exception_depth != 2
            || !finalizer.copy_on_return
            || finalizer.loop_exit_outer_return_depth.is_some()
            || finalizer.preserves_conditional_exit_owner
            || handler.loop_depth != 0
            || handler.with_depth != 0
            || handler.try_body_depth != 1
            || handler.finally_end_depth != 0
            || handler.exception_region_depth != 1
            || handler.common_cleanup_depth != 1
            || !matches!(
                control.exceptions.active_try_marker_slots.as_slice(),
                [None]
            )
            || control.exceptions.active_exception_region_exclusions.len() != 3
            || control.exceptions.active_exception_group_handlers != 0
            || !control.withs.active_with_exits.is_empty()
            || !control.withs.active_with_region_exclusions.is_empty()
            || !control.loops.contexts.is_empty()
            || !control
                .comprehensions
                .active_comprehension_cleanups
                .is_empty()
            || !control
                .comprehensions
                .active_comprehension_local_names
                .is_empty()
            || !control
                .comprehensions
                .active_comprehension_region_exclusions
                .is_empty()
            || finally.active_normal_finally_bodies != 0
            || finally.active_copied_finally_bodies != 0
            || finally.active_finally_end_blocks != 0
            || finally.active_overriding_finally_returns != 0
            || finally.active_finally_try_bodies != 1
            || !matches!(finally.active_loop_finally_contexts.as_slice(), [context]
                if same_finalizer(context, &finalizer))
            || finally.copied_return_finally_preserved_values != 0
            || !finally.preserved_return_value_contexts.is_empty()
            || !finally.active_return_finally_contexts.is_empty()
            || finally.active_pass_finally_locations != pass_locations
            || control.stack.depth != 2
        {
            return None;
        }
        let local = compiler.named_finally_return_local(handler.name?)?;
        let prefix = compiler.assembler.named_finally_return_prefix(opcode)?;
        Some(Self {
            finalizer,
            handler: *handler,
            exceptions: control.exceptions.clone(),
            finally: finally.clone(),
            withs: control.withs.clone(),
            loops: control.loops.clone(),
            conditions: control.conditions.clone(),
            finalizer_exit: *finalizer_exit,
            prefix,
            local,
            cleanup: None,
        })
    }

    fn unchanged(&self, compiler: &Compiler<'ast>, restored: bool) -> bool {
        let mut finally = self.finally.clone();
        if restored {
            finally.active_return_finally_contexts = vec![self.finalizer];
        }
        compiler.control.exceptions == self.exceptions
            && compiler.control.finally == finally
            && matches!(compiler.control.finally.active_loop_finally_contexts.as_slice(), [context]
                if same_finalizer(context, &self.finalizer))
            && compiler.control.withs == self.withs
            && compiler.control.loops == self.loops
            && compiler.control.conditions == self.conditions
            && self
                .handler
                .name
                .and_then(|name| compiler.named_finally_return_local(name))
                == Some(self.local)
    }

    /// Binds the actual constant admission while the same handler owns name clearing.
    pub(super) fn bind_none(
        &mut self,
        compiler: &Compiler<'ast>,
        handler: ExceptionHandlerContext,
        finalizers: &[FinallyExitContext<'ast>],
        starts: &[Option<Label>],
        none: u32,
    ) -> bool {
        let [None, Some(handler_pop), Some(exit)] = starts else {
            return false;
        };
        if self.cleanup.is_some()
            || handler != self.handler
            || !self.unchanged(compiler, false)
            || !matches!(finalizers, [context] if same_finalizer(context, &self.finalizer))
            || *exit != self.finalizer_exit
        {
            return false;
        }
        self.cleanup = Some(NamedHandlerCleanup {
            none,
            handler_pop: *handler_pop,
            finalizer_exit: *exit,
        });
        true
    }

    pub(super) fn record(self, compiler: &mut Compiler<'ast>, starts: &[Option<Label>]) {
        let Some(cleanup) = &self.cleanup else {
            return;
        };
        let Some(outer_pop) = cleanup.outer_pop(starts) else {
            return;
        };
        if compiler.control.stack.depth != 0
            || !self.unchanged(compiler, true)
            || !matches!(compiler.control.finally.active_return_finally_contexts.as_slice(), [context]
                if same_finalizer(context, &self.finalizer))
        {
            return;
        }
        compiler.assembler.record_named_finally_return(
            &self.prefix,
            outer_pop,
            cleanup.handler_pop,
            self.handler.common_cleanup,
            self.local,
            cleanup.none,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_python_parser::parse_module;

    #[test]
    fn named_finally_return_cleanup_keeps_the_real_handler_pop_slot() {
        let label = Label::from_u32;
        let cleanup = NamedHandlerCleanup {
            none: 3,
            handler_pop: label(4),
            finalizer_exit: label(5),
        };
        assert_eq!(
            cleanup.outer_pop(&[Some(label(1)), Some(label(4)), Some(label(5))]),
            Some(label(1))
        );
        for slots in [
            vec![None, Some(label(4)), Some(label(5))],
            vec![Some(label(1)), None, Some(label(5))],
            vec![Some(label(1)), Some(label(4)), None],
            vec![Some(label(1)), Some(label(6)), Some(label(5))],
            vec![Some(label(1)), Some(label(4)), Some(label(6))],
            vec![Some(label(1)), Some(label(4))],
            vec![Some(label(1)), Some(label(4)), Some(label(5)), None],
        ] {
            assert!(cleanup.outer_pop(&slots).is_none(), "{slots:?}");
        }
    }

    #[test]
    fn named_finally_return_context_requires_the_same_authored_body() {
        let first = parse_module("pass\n").unwrap();
        let other = parse_module("pass\n").unwrap();
        let context = FinallyExitContext {
            source_range: first.suite()[0].range(),
            body: first.suite(),
            enclosing_finally_try_depth: 0,
            pass_finally_depth: 0,
            loop_depth: 0,
            with_depth: 0,
            handler_depth: 1,
            finally_end_depth: 0,
            try_body_depth: 1,
            exception_region_depth: 2,
            suspension_exception_depth: 2,
            loop_context_depth: 0,
            copy_on_return: true,
            loop_exit_outer_return_depth: None,
            handler: Label::from_u32(4),
            depth: 1,
            preserves_conditional_exit_owner: false,
        };
        assert!(same_finalizer(&context, &context));
        let foreign = FinallyExitContext {
            body: other.suite(),
            ..context
        };
        assert_eq!(context, foreign);
        assert!(!same_finalizer(&context, &foreign));
        for mutation in 0..15 {
            let mut changed = context;
            match mutation {
                0 => changed.enclosing_finally_try_depth += 1,
                1 => changed.pass_finally_depth += 1,
                2 => changed.loop_depth += 1,
                3 => changed.with_depth += 1,
                4 => changed.handler_depth += 1,
                5 => changed.finally_end_depth += 1,
                6 => changed.try_body_depth += 1,
                7 => changed.exception_region_depth += 1,
                8 => changed.suspension_exception_depth += 1,
                9 => changed.loop_context_depth += 1,
                10 => changed.copy_on_return = false,
                11 => changed.loop_exit_outer_return_depth = Some(0),
                12 => changed.handler = Label::from_u32(5),
                13 => changed.depth += 1,
                _ => changed.preserves_conditional_exit_owner = true,
            }
            assert!(!same_finalizer(&context, &changed), "mutation {mutation}");
        }
    }

    fn admitted(body: &str) -> bool {
        let mut source = String::from("async def sample(value, flag):\n");
        for line in body.lines() {
            source.push_str("    ");
            source.push_str(line);
            source.push('\n');
        }
        let parsed = parse_module(&source).unwrap();
        let Stmt::FunctionDef(function) = &parsed.suite()[0] else {
            unreachable!()
        };
        donor_opcode(&function.body).is_some()
    }

    #[test]
    fn named_finally_return_donors_are_single_linear_authored_statements() {
        for body in [
            "pass",
            "cleanup(value)",
            "cleanup(value, flag=flag)",
            "cleanup((value := flag))",
            "cleanup(lambda: (yield value))",
            "del value.attribute",
            "del value.child.attribute",
            "del factory(value).attribute",
        ] {
            assert!(admitted(body), "{body}");
        }
        for body in [
            "pass\npass",
            "cleanup(value)\ncleanup(flag)",
            "value = cleanup(flag)",
            "del value",
            "del value[flag]",
            "del value.a, value.b",
            "if flag:\n    cleanup(value)",
            "flag and cleanup(value)",
            "cleanup(value if flag else flag)",
            "cleanup(value and flag)",
            "await cleanup(value)",
            "cleanup(await value)",
            "cleanup((yield value))",
            "cleanup(lambda item=(await value): item)",
            "cleanup(lambda item=(yield value): item)",
            "del (await value).attribute",
            "del (yield value).attribute",
            "del (value if flag else flag).attribute",
            "return value",
            "raise Error()",
        ] {
            assert!(!admitted(body), "{body}");
        }
    }
}
