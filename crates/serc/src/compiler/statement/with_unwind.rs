use super::super::legacy_native_labels::WithAwait;
use super::{
    CALL, Compiler, Constant, LOAD_CONST, Label, POP_TOP, SWAP, WithExitContext, WithState,
};
use crate::CompileError;
use crate::assembler::{Assembler, CompletedManagerAwait, HeldAwaitSource};
use ruff_text_size::TextRange;

/// A manager exit and the state needed to close its protected region after copied cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WithUnwind {
    pub(super) index: usize,
    pub(super) start: Label,
    exception_region_depth: usize,
    exclusion_count: usize,
}

/// The one manager exit actually emitted for this preserved return.
pub(super) struct CompletedManagerUnwind {
    returned: TextRange,
    withs: WithState,
    unwind: WithUnwind,
    completion: CompletedManagerAwait,
}

struct RecordedWithUnwind {
    unwinds: Vec<WithUnwind>,
    completion: Option<CompletedManagerUnwind>,
    copied_finally: Option<Vec<crate::assembler::CopiedFinallyManagerEvent>>,
}

impl CompletedManagerUnwind {
    pub(super) fn has_frontier(
        &self,
        withs: &WithState,
        returned: TextRange,
        unwinds: &[WithUnwind],
        assembler: &Assembler,
        aliases: [Label; 2],
        source: &HeldAwaitSource,
    ) -> bool {
        self.returned == returned
            && &self.withs == withs
            && unwinds == [self.unwind]
            && assembler.completed_manager_await_has_frontier(&self.completion, aliases, source)
    }
}

impl WithUnwind {
    #[cfg(test)]
    pub(super) fn test_origin(
        index: usize,
        start: Label,
        exception_region_depth: usize,
        exclusion_count: usize,
    ) -> Self {
        Self {
            index,
            start,
            exception_region_depth,
            exclusion_count,
        }
    }

    pub(super) fn matches_original(self, index: usize, state: &WithState) -> bool {
        self.index == index
            && state.active_with_exits.get(index).is_some_and(|context| {
                context.exception_region_depth == self.exception_region_depth
            })
            && state
                .active_with_region_exclusions
                .get(index)
                .is_some_and(|exclusions| exclusions.len() == self.exclusion_count)
    }
    pub(super) fn exits_exception_region(self, index: usize) -> bool {
        index >= self.exception_region_depth
    }

    /// Preserve the first cleanup boundary for each enclosing exception region.
    pub(super) fn record_exception_starts(self, starts: &mut [Option<Label>]) {
        for start in &mut starts[self.exception_region_depth..] {
            start.get_or_insert(self.start);
        }
    }

    /// Save the boundary and exclusion length before copied cleanup adds narrower ranges.
    pub(super) fn record_exception_starts_with_checkpoints(
        self,
        starts: &mut [Option<(Label, usize)>],
        exclusions: &[Vec<(Label, Label)>],
    ) {
        for (index, start) in starts
            .iter_mut()
            .enumerate()
            .skip(self.exception_region_depth)
        {
            start.get_or_insert((self.start, exclusions[index].len()));
        }
    }
}

impl WithState {
    fn begin_unwind(&self, index: usize, start: Label) -> WithUnwind {
        WithUnwind {
            index,
            start,
            exception_region_depth: self.active_with_exits[index].exception_region_depth,
            exclusion_count: self.active_with_region_exclusions[index].len(),
        }
    }

    pub(super) fn finish_unwinds(&mut self, unwinds: Vec<WithUnwind>, end: Label) {
        for unwind in unwinds {
            let exclusions = &mut self.active_with_region_exclusions[unwind.index];
            // Copied cleanup can append narrower exclusions after this manager has exited.
            // The complete unwind exclusion subsumes those ranges.
            exclusions.truncate(unwind.exclusion_count);
            exclusions.push((unwind.start, end));
        }
    }
}

impl Compiler<'_> {
    pub(super) fn with_with_context<T>(
        &mut self,
        mut context: WithExitContext,
        terminal: bool,
        compile: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let previous_depth = self.control.withs.active_with_exits.len();
        let previous_terminal_withs = self.control.withs.active_terminal_withs;
        // The `as` target is compiled before the context becomes active and can itself suspend.
        // Capture the epoch at the same push boundary as the former parallel epoch stack.
        context.suspension_epoch = self.control.generators.coroutine_suspension_epoch;
        self.control.withs.active_with_exits.push(context);
        self.control.withs.active_terminal_withs += usize::from(terminal);
        let result = compile(self);
        self.control.withs.active_terminal_withs = previous_terminal_withs;
        debug_assert!(
            result.is_err() || self.control.withs.active_with_exits.len() == previous_depth + 1,
            "with contexts should be balanced"
        );
        self.control
            .withs
            .active_with_exits
            .truncate(previous_depth);
        result
    }

    /// Python 3.14 replaced the bound exit callable with a callable/self pair.
    pub(super) fn with_exit_stack_slots(&self) -> i32 {
        if self.context.python_version <= crate::PythonVersion::Py313 {
            1
        } else {
            2
        }
    }

    pub(super) fn emit_with_exit_call(&mut self) -> Result<(), CompileError> {
        let slots = self.with_exit_stack_slots();
        self.emit(CALL, (slots + 1).cast_unsigned(), -slots - 2)
    }

    pub(super) fn emit_with_unwind(
        &mut self,
        from_depth: usize,
        to_depth: usize,
        preserve_tos: bool,
        origin: super::super::legacy_native_labels::WithAwait,
    ) -> Result<Vec<WithUnwind>, CompileError> {
        self.emit_with_unwind_inner(from_depth, to_depth, preserve_tos, origin, false, false)
            .map(|recorded| recorded.unwinds)
    }

    /// Retains a receipt from the real emitter without changing its instructions or policies.
    pub(super) fn emit_with_unwind_and_completion(
        &mut self,
        from_depth: usize,
        to_depth: usize,
        preserve_tos: bool,
        origin: WithAwait,
    ) -> Result<(Vec<WithUnwind>, Option<CompletedManagerUnwind>), CompileError> {
        self.emit_with_unwind_inner(from_depth, to_depth, preserve_tos, origin, true, false)
            .map(|recorded| (recorded.unwinds, recorded.completion))
    }

    pub(super) fn emit_with_unwind_recorded(
        &mut self,
        from_depth: usize,
        to_depth: usize,
        preserve_tos: bool,
        origin: WithAwait,
        capture: bool,
    ) -> Result<
        (
            Vec<WithUnwind>,
            Option<Vec<crate::assembler::CopiedFinallyManagerEvent>>,
        ),
        CompileError,
    > {
        self.emit_with_unwind_inner(from_depth, to_depth, preserve_tos, origin, false, capture)
            .map(|recorded| (recorded.unwinds, recorded.copied_finally))
    }

    fn emit_with_unwind_inner(
        &mut self,
        from_depth: usize,
        to_depth: usize,
        preserve_tos: bool,
        origin: WithAwait,
        capture_completion: bool,
        capture_events: bool,
    ) -> Result<RecordedWithUnwind, CompileError> {
        let mut completion = None;
        let mut unwind_starts = Vec::with_capacity(to_depth - from_depth);
        let mut events = capture_events.then(Vec::new);
        for index in (from_depth..to_depth).rev() {
            let original_start = self.assembler.item_count();
            let context = self.control.withs.active_with_exits[index];
            let original_withs = (capture_completion
                && self.context.python_version == crate::PythonVersion::Py314
                && from_depth == 0
                && to_depth == 1
                && preserve_tos
                && context.is_async
                && context.loop_depth == 1
                && context.exception_region_depth == 3
                && matches!(origin, WithAwait::Return(_)))
            .then(|| self.control.withs.clone());
            let start = self.assembler.label();
            self.assembler.mark(start);
            unwind_starts.push(self.control.withs.begin_unwind(index, start));
            self.assembler.set_location(context.location);
            if preserve_tos {
                if self.with_exit_stack_slots() == 2 {
                    self.emit(SWAP, 3, 0)?;
                }
                self.emit(SWAP, 2, 0)?;
            }
            let none = self.add_constant(Constant::None)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit(LOAD_CONST, none, 1)?;
            self.emit_with_exit_call()?;
            if context.is_async {
                let roles = Some(origin.roles(context.source_range));
                self.compile_awaitable_on_stack_with_native_roles(2, roles)?;
            }
            self.emit(POP_TOP, 0, -1)?;
            if let Some(withs) = original_withs
                && withs == self.control.withs
                && let WithAwait::Return(returned) = origin
                && let Some(event) = self.assembler.capture_completed_manager_await(
                    original_start,
                    start,
                    context.location,
                    none,
                    3,
                )
            {
                completion = Some(CompletedManagerUnwind {
                    returned,
                    withs,
                    unwind: *unwind_starts.last().unwrap(),
                    completion: event,
                });
            }
            if let Some(receipts) = &mut events {
                if let Some(event) = self.assembler.capture_copied_finally_manager_event(
                    original_start,
                    start,
                    none,
                    context.is_async,
                    context.location,
                    &self.output,
                ) {
                    receipts.push(event);
                } else {
                    events = None;
                }
            }
        }
        Ok(RecordedWithUnwind {
            unwinds: unwind_starts,
            completion,
            copied_finally: events,
        })
    }
}
