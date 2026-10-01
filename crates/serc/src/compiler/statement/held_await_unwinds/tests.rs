use super::super::{OrdinaryTryBodyContext, SourceLocation, WithBodyNormalExit, WithExitContext};
use super::*;
use ruff_python_parser::parse_module;
use ruff_text_size::TextSize;

fn returned() -> TextRange {
    TextRange::new(TextSize::new(30), TextSize::new(40))
}

fn fixture(body: &[Stmt], kind: UnwindKind) -> ControlFlowState<'_> {
    let mut control = super::super::completed_await_tests::fixture(body);
    control.finally.active_return_finally_contexts[0].loop_depth = 0;
    control.finally.active_return_finally_contexts[0].preserves_conditional_exit_owner = false;
    control.finally.active_loop_finally_contexts[0] =
        control.finally.active_return_finally_contexts[0];
    control.finally.protected_try_loop_depths[0] = (1, 0);
    control.loops.contexts[0].exception_region_depth = 2;
    match kind {
        UnwindKind::OrdinaryAwait => {
            control
                .exceptions
                .ordinary_try_bodies
                .push(OrdinaryTryBodyContext {
                    setup: Label::from_u32(19),
                    original_end: Label::from_u32(20),
                    target: Label::from_u32(21),
                    depth: 0,
                    exclusion_index: 3,
                });
            control
                .exceptions
                .active_try_marker_slots
                .push(control.exceptions.active_try_marker_slots[0]);
            control
                .exceptions
                .active_exception_region_exclusions
                .push(Vec::new());
        }
        UnwindKind::ManagerAwait => {
            control.withs.active_with_exits.push(WithExitContext {
                source_range: TextRange::new(TextSize::new(10), TextSize::new(20)),
                location: SourceLocation::new(2, 2, 3, 9),
                handler: Label::from_u32(22),
                is_async: true,
                suspension_epoch: 4,
                body_start_item: 40,
                terminal_statement_range: Some(TextRange::new(
                    TextSize::new(50),
                    TextSize::new(60),
                )),
                normal_exit: WithBodyNormalExit::Terminated,
                terminal_condition_preceded_by_finally: false,
                exception_region_depth: 3,
                finally_end_depth: 0,
                normal_finally_depth: 0,
                loop_depth: 1,
                preserves_none_condition_owner: false,
            });
            control
                .withs
                .active_with_region_exclusions
                .push(vec![(Label::from_u32(70), Label::from_u32(71))]);
        }
    }
    control
}

fn capture<'ast>(
    control: &ControlFlowState<'ast>,
    kind: UnwindKind,
) -> Option<SourceFrontier<'ast>> {
    SourceFrontier::capture(
        control,
        returned(),
        &control.finally.active_loop_finally_contexts[0],
        kind,
    )
}

fn source(kind: UnwindKind) -> HeldAwaitSource {
    use crate::assembler::Assembler;
    use crate::target::opcodes::NOP;
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let mut ordinary = Vec::new();
    let mut loop_start = 0;
    for index in 0..if kind == UnwindKind::OrdinaryAwait {
        3
    } else {
        2
    } {
        if index == 1 {
            loop_start = assembler.item_count();
            let entry = assembler.label();
            assembler.mark(entry);
            assembler.emit(NOP, 0);
        }
        let setup = assembler.label();
        let handler = assembler.label();
        assembler.record_ordinary_try_exception_setup(setup, false, handler);
        assembler.mark(setup);
        assembler.emit(NOP, 0);
        ordinary.push((setup, handler));
    }
    let manager_start = if kind == UnwindKind::ManagerAwait {
        let start = assembler.item_count();
        let entry = assembler.label();
        assembler.mark(entry);
        assembler.emit(NOP, 0);
        Some(start)
    } else {
        None
    };
    assembler
        .capture_held_await_source(&ordinary, loop_start, manager_start)
        .unwrap()
}

#[test]
fn held_await_aliases_require_actual_collectors_and_the_unwound_context() {
    let parsed = parse_module("if value is not None:\n    finish(value)\n").unwrap();
    for kind in [UnwindKind::OrdinaryAwait, UnwindKind::ManagerAwait] {
        let mut control = fixture(parsed.suite(), kind);
        let finalizer = control.finally.active_return_finally_contexts[0];
        let original = HeldAwaitOrdinaryPopFrontier {
            kind,
            returned: returned(),
            original: capture(&control, kind).unwrap(),
            source: source(kind),
        };
        let general = Label::from_u32(100);
        let boundaries = [general, Label::from_u32(101), Label::from_u32(102)];
        let label = Label::from_u32(103);
        let starts = vec![
            Some(label);
            if kind == UnwindKind::OrdinaryAwait {
                4
            } else {
                3
            }
        ];
        let check = |control: &ControlFlowState<'_>, starts: &[Option<Label>]| match kind {
            UnwindKind::OrdinaryAwait => original
                .ordinary_aliases(control, returned(), &finalizer, boundaries, starts)
                .is_some(),
            UnwindKind::ManagerAwait => original
                .manager_aliases(control, returned(), &finalizer, general, starts)
                .is_some(),
        };
        assert!(!check(&control, &starts));
        control.finally.active_return_finally_contexts.clear();
        assert!(check(&control, &starts));
        for index in 0..starts.len() {
            for replacement in [None, Some(Label::from_u32(104)), Some(general)] {
                let mut changed = starts.clone();
                changed[index] = replacement;
                assert!(!check(&control, &changed), "{kind:?} collector {index}");
            }
        }
        assert!(!check(&control, &starts[..starts.len() - 1]));
        let mut extra = starts.clone();
        extra.push(Some(label));
        assert!(!check(&control, &extra));
        control.exceptions.active_exception_region_exclusions[0].push((general, label));
        assert!(!check(&control, &starts));
        control.exceptions.active_exception_region_exclusions[0].clear();
        assert!(check(&control, &starts));
        control.loops.contexts.clear();
        assert!(!check(&control, &starts));
    }
}

#[test]
fn held_await_source_binds_complete_finalizer_records() {
    let parsed = parse_module("if value is not None:\n    finish(value)\n").unwrap();
    let mutations: &[fn(&mut FinallyExitContext<'_>)] = &[
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
        |c| c.preserves_conditional_exit_owner = true,
    ];
    for kind in [UnwindKind::OrdinaryAwait, UnwindKind::ManagerAwait] {
        let original = capture(&fixture(parsed.suite(), kind), kind).unwrap();
        for (index, mutate) in mutations.iter().enumerate() {
            let mut control = fixture(parsed.suite(), kind);
            mutate(&mut control.finally.active_loop_finally_contexts[0]);
            mutate(&mut control.finally.active_return_finally_contexts[0]);
            assert!(
                capture(&control, kind).as_ref() != Some(&original),
                "{kind:?} finalizer {index}"
            );
        }
    }
}

#[test]
fn held_await_source_binds_every_manager_field_and_prior_exclusion() {
    let parsed = parse_module("if value is not None:\n    finish(value)\n").unwrap();
    let body = parsed.suite();
    let original = capture(
        &fixture(body, UnwindKind::ManagerAwait),
        UnwindKind::ManagerAwait,
    )
    .unwrap();
    let mutations: &[fn(&mut WithExitContext)] = &[
        |c| c.source_range = returned(),
        |c| c.location.column += 1,
        |c| c.handler = Label::from_u32(99),
        |c| c.is_async = false,
        |c| c.suspension_epoch += 1,
        |c| c.body_start_item += 1,
        |c| c.terminal_statement_range = Some(returned()),
        |c| c.normal_exit = WithBodyNormalExit::Unknown,
        |c| c.terminal_condition_preceded_by_finally = true,
        |c| c.exception_region_depth += 1,
        |c| c.finally_end_depth += 1,
        |c| c.normal_finally_depth += 1,
        |c| c.loop_depth += 1,
        |c| c.preserves_none_condition_owner = true,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut control = fixture(body, UnwindKind::ManagerAwait);
        mutate(&mut control.withs.active_with_exits[0]);
        assert!(
            capture(&control, UnwindKind::ManagerAwait).as_ref() != Some(&original),
            "manager {index}"
        );
    }
    let mut control = fixture(body, UnwindKind::ManagerAwait);
    control.withs.active_with_region_exclusions[0].clear();
    assert!(capture(&control, UnwindKind::ManagerAwait).as_ref() != Some(&original));
    control.withs.active_with_region_exclusions[0].push((Label::from_u32(71), Label::from_u32(70)));
    assert!(capture(&control, UnwindKind::ManagerAwait).as_ref() != Some(&original));
}

#[test]
fn held_await_source_binds_scoped_policies_collectors_and_context_truncation() {
    let parsed = parse_module("if value is not None:\n    finish(value)\n").unwrap();
    let body = parsed.suite();
    let replacement = parse_module("if value is not None:\n    finish(value)\n").unwrap();
    for kind in [UnwindKind::OrdinaryAwait, UnwindKind::ManagerAwait] {
        let original = capture(&fixture(body, kind), kind).unwrap();
        let mutations: &[fn(&mut ControlFlowState<'_>)] = &[
            |c| c.loops.contexts[0].body_start_item += 1,
            |c| c.loops.contexts[0].exception_region_depth += 1,
            |c| c.loops.contexts[0].continue_label = Label::from_u32(99),
            |c| c.loops.contexts.clear(),
            |c| c.loops.split_loop_tail_prefix_end = Some(returned()),
            |c| c.conditions.include_scoped_none_condition_owner = true,
            |c| c.exceptions.ordinary_try_bodies[0].target = Label::from_u32(99),
            |c| c.exceptions.ordinary_try_bodies[0].original_end = Label::from_u32(99),
            |c| c.exceptions.ordinary_try_bodies.swap(0, 1),
            |c| {
                c.exceptions.active_exception_region_exclusions[0]
                    .push((Label::from_u32(90), Label::from_u32(91)));
            },
            |c| {
                c.exceptions
                    .active_exception_region_exclusions
                    .pop()
                    .map(|_| ())
                    .unwrap();
            },
            |c| c.exceptions.active_try_marker_slots[0] = None,
            |c| c.exceptions.active_try_body_start = Some((99, 99)),
            |c| c.exceptions.active_try_body_tail = Some(returned()),
            |c| c.withs.active_terminal_withs += 1,
            |c| c.withs.terminal_with_condition = Some(returned()),
            |c| c.finally.active_conditional_cleanup_finally_try_bodies += 1,
            |c| c.finally.preserve_finally_break_exit_loop_range = Some(returned()),
            |c| c.finally.active_normal_finally_bodies += 1,
            |c| c.finally.active_copied_finally_bodies += 1,
            |c| c.finally.active_finally_end_blocks += 1,
            |c| c.finally.protected_try_loop_depths[0].1 += 1,
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut control = fixture(body, kind);
            control.finally.active_return_finally_contexts.clear();
            assert!(capture(&control, kind).as_ref() == Some(&original));
            mutate(&mut control);
            assert!(
                capture(&control, kind).as_ref() != Some(&original),
                "{kind:?} mutation {index}"
            );
        }
        let mut control = fixture(body, kind);
        control.finally.active_return_finally_contexts[0].body = replacement.suite();
        control.finally.active_loop_finally_contexts[0].body = replacement.suite();
        assert!(capture(&control, kind).as_ref() != Some(&original));
    }
}

#[test]
fn held_await_manager_unwind_identity_includes_all_actual_collector_fields() {
    use super::super::with_unwind::WithUnwind;
    let a = WithUnwind::test_origin(0, Label::from_u32(1), 3, 2);
    for b in [
        WithUnwind::test_origin(1, Label::from_u32(1), 3, 2),
        WithUnwind::test_origin(0, Label::from_u32(2), 3, 2),
        WithUnwind::test_origin(0, Label::from_u32(1), 4, 2),
        WithUnwind::test_origin(0, Label::from_u32(1), 3, 1),
    ] {
        assert_ne!(a, b);
    }
}
