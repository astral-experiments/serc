use super::{
    CompletedAwaitOrdinaryPopFrontier, ControlFlowState, FinallyExitContext, IteratorCleanup,
    Label, LoopBreakPolicy, LoopContext, OrdinaryTryBodyContext, SourceLocation, Stmt, TextRange,
    TryMarkerDirectCondition, TryMarkerSlotContext,
};
use ruff_python_parser::parse_module;
use ruff_text_size::TextSize;

pub(super) fn fixture(body: &[Stmt]) -> ControlFlowState<'_> {
    let mut control = ControlFlowState::default();
    let finalizer = FinallyExitContext {
        source_range: TextRange::new(TextSize::new(0), TextSize::new(1000)),
        body,
        enclosing_finally_try_depth: 0,
        pass_finally_depth: 0,
        loop_depth: 1,
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
        preserves_conditional_exit_owner: true,
    };
    control
        .finally
        .active_return_finally_contexts
        .push(finalizer);
    control.finally.active_loop_finally_contexts.push(finalizer);
    control.finally.active_finally_try_bodies = 1;
    control.finally.protected_try_loop_depths.push((1, 1));
    control.finally.innermost_finally_exception_exclusion_depth = Some(1);
    control
        .finally
        .active_conditional_cleanup_finally_try_bodies = 1;
    control.loops.contexts.push(LoopContext {
        body_start: 1,
        body_start_item: 2,
        continue_label: Label::from_u32(3),
        break_label: Label::from_u32(4),
        iterator_cleanup: IteratorCleanup::None,
        has_orelse: false,
        copied_continue_header_is_linear: true,
        prevent_break_exit_inlining: false,
        with_depth: 0,
        finally_end_depth: 0,
        exception_region_depth: 0,
        break_target_keeps_exception_owner: false,
        break_target_is_short_return: false,
        protected_finally_break_owner: false,
        break_policy: LoopBreakPolicy::Jump,
    });
    for index in 1..=2 {
        control
            .exceptions
            .ordinary_try_bodies
            .push(OrdinaryTryBodyContext {
                setup: Label::from_u32(10 + 3 * index),
                original_end: Label::from_u32(11 + 3 * index),
                target: Label::from_u32(12 + 3 * index),
                depth: 0,
                exclusion_index: index as usize,
            });
        control
            .exceptions
            .active_try_marker_slots
            .push(Some(TryMarkerSlotContext {
                ownerless_prefix_slots: 0,
                protected_start_instruction: index as usize,
                setup_location: Some(SourceLocation::new(
                    i32::try_from(index).unwrap(),
                    i32::try_from(index).unwrap(),
                    0,
                    4,
                )),
                direct_condition: TryMarkerDirectCondition::None,
                handler_starts_with_optimized_boolean_generator: false,
            }));
    }
    control.exceptions.active_exception_region_exclusions = vec![Vec::new(); 3];
    control
}

fn returned() -> TextRange {
    TextRange::new(TextSize::new(30), TextSize::new(40))
}

#[test]
fn completed_await_source_binds_every_saved_finalizer_and_loop_field() {
    let parsed = parse_module("if value is not None:\n    finish(value)\n").unwrap();
    let body = parsed.suite();
    let original = CompletedAwaitOrdinaryPopFrontier::capture(&fixture(body), returned()).unwrap();
    let finalizer_mutations: &[fn(&mut FinallyExitContext<'_>)] = &[
        |c| c.source_range = TextRange::new(TextSize::new(0), TextSize::new(999)),
        |c| c.enclosing_finally_try_depth += 1,
        |c| c.pass_finally_depth += 1,
        |c| c.loop_depth += 1,
        |c| c.with_depth += 1,
        |c| c.handler_depth += 1,
        |c| c.finally_end_depth += 1,
        |c| c.try_body_depth += 1,
        |c| c.exception_region_depth += 1,
        |c| c.suspension_exception_depth += 1,
        |c| c.loop_context_depth += 1,
        |c| c.copy_on_return = false,
        |c| c.loop_exit_outer_return_depth = Some(0),
        |c| c.handler = Label::from_u32(99),
        |c| c.depth += 1,
        |c| c.preserves_conditional_exit_owner = false,
    ];
    for (index, mutate) in finalizer_mutations.iter().enumerate() {
        let mut changed = fixture(body);
        mutate(&mut changed.finally.active_return_finally_contexts[0]);
        mutate(&mut changed.finally.active_loop_finally_contexts[0]);
        assert!(
            CompletedAwaitOrdinaryPopFrontier::capture(&changed, returned()).as_ref()
                != Some(&original),
            "finalizer {index}"
        );
    }
    let loop_mutations: &[fn(&mut LoopContext)] = &[
        |c| c.body_start += 1,
        |c| c.body_start_item += 1,
        |c| c.continue_label = Label::from_u32(98),
        |c| c.break_label = Label::from_u32(99),
        |c| c.iterator_cleanup = IteratorCleanup::Sync,
        |c| c.has_orelse = true,
        |c| c.copied_continue_header_is_linear = false,
        |c| c.prevent_break_exit_inlining = true,
        |c| c.with_depth += 1,
        |c| c.finally_end_depth += 1,
        |c| c.exception_region_depth += 1,
        |c| c.break_target_keeps_exception_owner = true,
        |c| c.break_target_is_short_return = true,
        |c| c.protected_finally_break_owner = true,
        |c| c.break_policy = LoopBreakPolicy::Return,
    ];
    for (index, mutate) in loop_mutations.iter().enumerate() {
        let mut changed = fixture(body);
        mutate(&mut changed.loops.contexts[0]);
        assert!(
            CompletedAwaitOrdinaryPopFrontier::capture(&changed, returned()).as_ref()
                != Some(&original),
            "loop {index}"
        );
    }
    let replacement = parse_module("if value is not None:\n    finish(value)\n").unwrap();
    let mut changed = fixture(body);
    changed.finally.active_return_finally_contexts[0].body = replacement.suite();
    changed.finally.active_loop_finally_contexts[0].body = replacement.suite();
    assert!(
        CompletedAwaitOrdinaryPopFrontier::capture(&changed, returned()).as_ref()
            != Some(&original)
    );
}

#[test]
fn completed_await_source_binds_markers_ordinary_bodies_and_collectors() {
    let parsed = parse_module("finish(value)\n").unwrap();
    let body = parsed.suite();
    let original = CompletedAwaitOrdinaryPopFrontier::capture(&fixture(body), returned()).unwrap();
    let mutations: &[fn(&mut ControlFlowState<'_>)] = &[
        |c| c.exceptions.ordinary_try_bodies[0].setup = Label::from_u32(99),
        |c| c.exceptions.ordinary_try_bodies[0].original_end = Label::from_u32(99),
        |c| c.exceptions.ordinary_try_bodies[0].target = Label::from_u32(99),
        |c| c.exceptions.ordinary_try_bodies[0].depth += 1,
        |c| c.exceptions.ordinary_try_bodies[0].exclusion_index = 2,
        |c| c.exceptions.ordinary_try_bodies.swap(0, 1),
        |c| c.exceptions.ordinary_try_bodies.truncate(1),
        |c| c.exceptions.active_try_marker_slots[0] = None,
        |c| {
            c.exceptions.active_try_marker_slots[0]
                .as_mut()
                .unwrap()
                .ownerless_prefix_slots += 1;
        },
        |c| {
            c.exceptions.active_try_marker_slots[0]
                .as_mut()
                .unwrap()
                .protected_start_instruction += 1;
        },
        |c| {
            c.exceptions.active_try_marker_slots[0]
                .as_mut()
                .unwrap()
                .setup_location = None;
        },
        |c| {
            c.exceptions.active_try_marker_slots[0]
                .as_mut()
                .unwrap()
                .direct_condition = TryMarkerDirectCondition::Ownerless(TextRange::default());
        },
        |c| {
            c.exceptions.active_try_marker_slots[0]
                .as_mut()
                .unwrap()
                .handler_starts_with_optimized_boolean_generator = true;
        },
        |c| {
            c.exceptions.active_exception_region_exclusions[1]
                .push((Label::from_u32(90), Label::from_u32(91)));
        },
        |c| c.exceptions.active_exception_region_exclusions.truncate(2),
        |c| c.finally.active_conditional_cleanup_finally_try_bodies += 1,
        |c| c.finally.preserve_finally_break_exit_loop_range = Some(TextRange::default()),
        |c| c.finally.borrow_reachable_finally_loop_range = Some(TextRange::default()),
        |c| c.finally.active_finally_try_bodies += 1,
        |c| c.finally.active_copied_finally_bodies += 1,
        |c| c.finally.active_normal_finally_bodies += 1,
        |c| c.finally.active_finally_end_blocks += 1,
        |c| c.finally.active_overriding_finally_returns += 1,
        |c| c.finally.protected_try_loop_depths[0].1 += 1,
        |c| c.finally.innermost_finally_exception_exclusion_depth = None,
        |c| c.exceptions.active_exception_group_handlers += 1,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = fixture(body);
        mutate(&mut changed);
        assert!(
            CompletedAwaitOrdinaryPopFrontier::capture(&changed, returned()).as_ref()
                != Some(&original),
            "mutation {index}"
        );
    }
}

#[test]
fn completed_await_source_requires_the_actual_three_collector_frontiers_after_truncation() {
    let parsed = parse_module("finish(value)\n").unwrap();
    let mut control = fixture(parsed.suite());
    let original = CompletedAwaitOrdinaryPopFrontier::capture(&control, returned()).unwrap();
    let finalizer = control.finally.active_return_finally_contexts[0];
    let boundaries = [
        Label::from_u32(30),
        Label::from_u32(31),
        Label::from_u32(32),
    ];
    let frontier = Label::from_u32(33);
    let starts = [Some(frontier); 3];
    assert!(
        original
            .unwind_aliases(&control, returned(), &finalizer, boundaries, &starts)
            .is_none()
    );
    control.finally.active_return_finally_contexts.clear();
    assert_eq!(
        original.unwind_aliases(&control, returned(), &finalizer, boundaries, &starts),
        Some([boundaries[0], boundaries[1], boundaries[2], frontier])
    );
    for changed in [
        vec![],
        vec![Some(frontier); 2],
        vec![Some(frontier); 4],
        vec![None, Some(frontier), Some(frontier)],
        vec![Some(frontier), None, Some(frontier)],
        vec![Some(frontier), Some(frontier), None],
        vec![Some(boundaries[0]), Some(frontier), Some(frontier)],
        vec![Some(frontier), Some(boundaries[0]), Some(frontier)],
        vec![Some(frontier), Some(frontier), Some(boundaries[0])],
        vec![Some(boundaries[0]); 3],
    ] {
        assert!(
            original
                .unwind_aliases(&control, returned(), &finalizer, boundaries, &changed)
                .is_none()
        );
    }
    control.loops.contexts.clear();
    assert!(
        original
            .unwind_aliases(&control, returned(), &finalizer, boundaries, &starts)
            .is_none()
    );
}
