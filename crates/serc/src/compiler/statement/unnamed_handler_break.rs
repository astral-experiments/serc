use super::{
    Compiler, ControlFlowState, ExceptionHandlerContext, FinallyExitContext, IteratorCleanup,
    LoopContext,
};
use crate::PythonVersion;
use crate::target::code_flags::{CO_ASYNC_GENERATOR, CO_COROUTINE, CO_GENERATOR};
use ruff_python_ast::{CmpOp, Expr, Stmt};
use ruff_text_size::Ranged;

#[derive(Clone, Copy, PartialEq)]
pub(super) struct UnnamedHandlerBreakFrontier<'ast> {
    loop_context: LoopContext,
    handler: ExceptionHandlerContext,
    finalizer: FinallyExitContext<'ast>,
}

impl<'ast> UnnamedHandlerBreakFrontier<'ast> {
    pub(super) fn capture(
        control: &ControlFlowState<'ast>,
        version: PythonVersion,
        flags: u32,
    ) -> Option<Self> {
        let [loop_context] = control.loops.contexts.as_slice() else {
            return None;
        };
        let [handler] = control.exceptions.active_exception_handlers.as_slice() else {
            return None;
        };
        let [finalizer] = control.finally.active_loop_finally_contexts.as_slice() else {
            return None;
        };
        if version != PythonVersion::Py314
            || !matches!(
                flags & (CO_GENERATOR | CO_COROUTINE | CO_ASYNC_GENERATOR),
                CO_COROUTINE | CO_ASYNC_GENERATOR
            )
            || loop_context.iterator_cleanup != IteratorCleanup::None
            || loop_context.with_depth != 0
            || loop_context.finally_end_depth != 0
            || loop_context.exception_region_depth != 0
            || handler.name.is_some()
            || handler.loop_depth != 1
            || handler.with_depth != 0
            || handler.try_body_depth != 0
            || handler.finally_end_depth != 0
            || handler.exception_region_depth != 1
            || finalizer.handler_depth != 0
            || finalizer.with_depth != 0
            || finalizer.loop_depth != 1
            || finalizer.finally_end_depth != 0
            || finalizer.try_body_depth != 0
            || finalizer.exception_region_depth != 0
            || finalizer.depth != 0
            || !control.withs.active_with_exits.is_empty()
            || !control.exceptions.active_try_marker_slots.is_empty()
            || control.exceptions.active_exception_group_handlers != 0
            || control.exceptions.active_exception_region_exclusions.len() != 2
            || control.finally.active_finally_try_bodies != 1
            || control.finally.active_finally_end_blocks != 0
            || control.finally.active_normal_finally_bodies != 0
            || control.finally.active_copied_finally_bodies != 0
            || control.finally.active_overriding_finally_returns != 0
            || !control.finally.active_pass_finally_locations.is_empty()
            || !control.finally.preserved_return_value_contexts.is_empty()
            || control.finally.copied_return_finally_preserved_values != 0
            || !matches!(finalizer.body.first(), Some(Stmt::If(guard))
                if guard.elif_else_clauses.is_empty()
                    && matches!(guard.test.as_ref(), Expr::Compare(comparison)
                        if matches!(comparison.first_operand(), Expr::Name(_))
                            && matches!(comparison.ops.as_ref(), [CmpOp::Is | CmpOp::IsNot])
                            && matches!(comparison.comparators(), [Expr::NoneLiteral(_)])))
        {
            return None;
        }
        Some(Self {
            loop_context: *loop_context,
            handler: *handler,
            finalizer: *finalizer,
        })
    }

    pub(super) fn record(self, compiler: &mut Compiler<'ast>, start: usize) {
        if Self::capture(
            &compiler.control,
            compiler.context.python_version,
            compiler.unit.flags,
        ) != Some(self)
            || !std::ptr::eq(
                compiler.control.finally.active_loop_finally_contexts[0]
                    .body
                    .as_ptr(),
                self.finalizer.body.as_ptr(),
            )
        {
            return;
        }
        let Stmt::If(guard) = &self.finalizer.body[0] else {
            unreachable!();
        };
        let Expr::Compare(comparison) = guard.test.as_ref() else {
            unreachable!();
        };
        compiler.assembler.record_unnamed_handler_break_none(
            start,
            [
                self.loop_context.continue_label,
                self.loop_context.break_label,
                self.handler.body_cleanup,
                self.handler.common_cleanup,
                self.finalizer.handler,
            ],
            compiler.source_location(comparison.first_operand().range()),
            compiler.source_location(comparison.range),
        );
    }
}
