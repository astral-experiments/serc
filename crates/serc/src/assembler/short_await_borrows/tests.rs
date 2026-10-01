use super::*;
use crate::assembler::ir::{BorrowFallthrough, EmissionStreamId, UnoptimizedCode};
use crate::assembler::{ExceptionPolicy, SourceLocation};
use crate::target::opcodes::{NOP, POP_TOP, RETURN_VALUE};
use crate::target::operands::ResumeLocation;

struct Fixture {
    assembler: Assembler,
    source: EmittedJump,
    target: Label,
    send: Label,
    end: Label,
    yielded: Label,
    yielded_end: Label,
    cleanup: Label,
}

fn fixture() -> Fixture {
    fixture_with_padding(0)
}

fn fixture_with_padding(padding: usize) -> Fixture {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    for _ in 0..padding {
        assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
    }
    let source_head = assembler.label();
    let source_alias = assembler.label();
    let target = assembler.label();
    let target_alias = assembler.label();
    let send = assembler.label();
    let end = assembler.label();
    let yielded = assembler.label();
    let yielded_end = assembler.label();
    let cleanup = assembler.label();
    assembler.mark(source_head);
    assembler.mark(source_alias);
    assembler.set_location(SourceLocation::new(2, 2, 4, 16));
    let source =
        assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.record_noncopyable_short_await_borrow_exit(source);
    assembler.mark(target);
    assembler.mark(target_alias);
    assembler.set_location(SourceLocation::new(6, 6, 10, 15));
    assembler.emit_with_depth(LOAD_FAST, 0, 1, ExceptionPolicy::default());
    assembler.set_location(SourceLocation::new(6, 6, 4, 15));
    assembler.emit_with_depth(GET_AWAITABLE, 0, 1, ExceptionPolicy::default());
    assembler.emit_with_depth(LOAD_CONST, 0, 2, ExceptionPolicy::default());
    assembler.mark(send);
    assembler.emit_forward_with_depth(SEND, end, 2, ExceptionPolicy::default());
    assembler.record_exception_setup(yielded, false);
    assembler.mark(yielded);
    assembler.emit_with_depth(YIELD_VALUE, 1, 2, ExceptionPolicy::default());
    assembler.mark(yielded_end);
    assembler.emit_with_depth(
        RESUME,
        ResumeLocation::AFTER_AWAIT.argument(),
        2,
        ExceptionPolicy::default(),
    );
    assembler.emit_backward_with_depth_and_exception_policy(
        JUMP_BACKWARD_NO_INTERRUPT,
        send,
        2,
        ExceptionPolicy::default(),
    );
    assembler.mark(cleanup);
    assembler.add_exception_region(yielded, yielded_end, cleanup, 2, false);
    assembler.emit_with_depth(CLEANUP_THROW, 0, 2, ExceptionPolicy::default());
    assembler.record_exception_owner_block_boundary(end);
    assembler.mark(end);
    assembler.emit_with_depth(END_SEND, 0, 1, ExceptionPolicy::default());
    assembler.record_ordinary_await_cleanup(yielded, cleanup);
    assembler.record_short_await_borrow_target(0, send, end, yielded, yielded_end, cleanup);
    assert!(
        assembler.deferred_noncopyable_borrow_exits[0]
            .short_await
            .as_ref()
            .unwrap()
            .completed
            .is_some()
    );
    Fixture {
        assembler,
        source,
        target,
        send,
        end,
        yielded,
        yielded_end,
        cleanup,
    }
}

fn finish(assembler: &mut Assembler) {
    assembler.emit_with_depth(POP_TOP, 0, 0, ExceptionPolicy::default());
    assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
    assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
}

fn assert_result(mut fixture: Fixture, admitted: bool, prepared: bool) {
    finish(&mut fixture.assembler);
    let mut expected = fixture.assembler.linear_items().to_vec();
    if admitted {
        let Item::Instruction(source) =
            &mut expected[fixture.source.instruction().item_index as usize]
        else {
            panic!()
        };
        source
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    }
    if prepared {
        fixture.assembler.used_constant_indices(2);
        assert!(matches!(
            fixture.assembler.code,
            UnoptimizedCode::Prepared { .. }
        ));
    } else {
        fixture.assembler.resolve_noncopyable_borrow_exits();
    }
    fixture.assembler.resolve_noncopyable_borrow_exits();
    assert!(
        fixture
            .assembler
            .deferred_noncopyable_borrow_exits
            .is_empty()
    );
    assert!(fixture.assembler.borrow_unreachable_blocks.is_empty());
    assert!(
        fixture
            .assembler
            .exception_owner_provenance
            .borrow_only_block_boundaries
            .is_empty()
    );
    if !prepared {
        assert_eq!(fixture.assembler.linear_items(), expected);
    }
    if unique_labels(&expected).is_some() {
        assert_eq!(
            fixture
                .assembler
                .into_optimization_pipeline()
                .graph
                .copy_items(),
            expected
        );
    }
}

#[test]
fn short_await_proof_is_source_local_and_consumed_before_preparation() {
    for prepared in [false, true] {
        assert_result(fixture(), true, prepared);
        let mut independent = fixture();
        let location = SourceLocation::new(8, 8, 4, 15);
        let other = independent.assembler.label();
        independent.assembler.mark(other);
        independent.assembler.set_location(location);
        independent.assembler.emit_forward_with_depth(
            JUMP_FORWARD,
            independent.target,
            0,
            ExceptionPolicy::default(),
        );
        assert_result(independent, true, prepared);
    }
}

#[test]
fn moving_the_complete_await_before_its_source_revokes_the_proof() {
    let mut fixture = fixture_with_padding(32);
    let origin = fixture.assembler.deferred_noncopyable_borrow_exits[0]
        .short_await
        .as_ref()
        .unwrap();
    let target = origin.completed.as_ref().unwrap().items.clone();
    let start = anchored_start(fixture.assembler.linear_items(), &target).unwrap();
    let replacement_head = fixture.assembler.label();
    let items = fixture.assembler.linear_items_mut();
    let padding = items[1..=target.len()].to_vec();
    items[1..=target.len()].copy_from_slice(&target);
    items[start..start + target.len()].copy_from_slice(&padding);
    items[start] = Item::Label(replacement_head);
    assert_result(fixture, false, false);
}

#[test]
fn every_original_short_await_record_remains_exact() {
    let indexes = fixture()
        .assembler
        .linear_items()
        .iter()
        .enumerate()
        .filter_map(|(index, item)| matches!(item, Item::Instruction(_)).then_some(index))
        .collect::<Vec<_>>();
    for index in indexes {
        for mutation in 0..9 {
            let mut fixture = fixture();
            let Item::Instruction(record) = &mut fixture.assembler.linear_items_mut()[index] else {
                panic!()
            };
            match mutation {
                0 => record.opcode = NOP,
                1 => record.set_operand(Operand::Value(99)),
                2 => record.location = SourceLocation::NONE,
                3 => {
                    *record =
                        Instruction::new(record.opcode, record.operand(), record.location, Some(9));
                }
                4 => record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                5 => record.jump_policy.preserve_explicit_nop(),
                6 => record
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                7 => record.local_policy.mark_maybe_null_comprehension_restore(),
                _ => record.add_pre_fold_extra_instructions(1),
            }
            assert_result(fixture, false, false);
        }
    }
}

#[test]
fn foreign_aliases_sources_regions_and_loop_entries_revoke_short_await_proof() {
    for mutation in [
        "duplicate source",
        "source alias",
        "target alias",
        "send alias",
        "end alias",
        "yield alias",
        "yield-end alias",
        "cleanup alias",
        "foreign send",
        "foreign end",
        "region depth",
        "region endpoint",
        "duplicate region",
        "duplicate origin",
        "foreign exception",
        "stale stream",
        "early constants",
        "later nop",
    ] {
        let mut fixture = fixture();
        let assembler = &mut fixture.assembler;
        let other = assembler.label();
        match mutation {
            "duplicate source" => {
                let index = fixture.source.instruction().item_index as usize;
                let original = assembler.linear_items()[index];
                assembler.mark(other);
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    panic!()
                };
                items.push(original);
            }
            "source alias" | "target alias" | "send alias" | "end alias" | "yield alias"
            | "yield-end alias" | "cleanup alias" => {
                let target = match mutation {
                    "source alias" => match assembler.linear_items()[0] {
                        Item::Label(label) => label,
                        Item::Instruction(_) => panic!(),
                    },
                    "target alias" => fixture.target,
                    "send alias" => fixture.send,
                    "end alias" => fixture.end,
                    "yield alias" => fixture.yielded,
                    "yield-end alias" => fixture.yielded_end,
                    _ => fixture.cleanup,
                };
                assembler.mark(target);
            }
            "foreign send" | "foreign end" => {
                assembler.mark(other);
                assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    if mutation == "foreign send" {
                        fixture.send
                    } else {
                        fixture.end
                    },
                    0,
                    ExceptionPolicy::default(),
                );
            }
            "region depth" => assembler.exception_regions[0].depth += 1,
            "region endpoint" => assembler.exception_regions[0].end = other,
            "duplicate region" => assembler
                .exception_regions
                .push(assembler.exception_regions[0]),
            "duplicate origin" => {
                let original = assembler.exception_owner_provenance.ordinary_await_cleanups[0];
                assembler
                    .exception_owner_provenance
                    .ordinary_await_cleanups
                    .push(original);
            }
            "foreign exception" => {
                let mut region = assembler.exception_regions[0];
                region.target = fixture.end;
                assembler.exception_regions.push(region);
            }
            "stale stream" => assembler.emission_stream = EmissionStreamId::next(),
            "early constants" => assembler.remap_constant_indices(&[Some(1)]),
            _ => {
                assembler.emit_with_depth(NOP, 0, 1, ExceptionPolicy::default());
                assert!(assembler.take_trailing_nop_location().is_some());
            }
        }
        assert_result(fixture, false, false);
    }
}

#[test]
fn capture_requires_the_actual_none_index_and_completed_ordinary_await() {
    for mutation in [
        "wrong none",
        "wrong send",
        "wrong end",
        "wrong yielded",
        "wrong cleanup",
        "missing ordinary origin",
    ] {
        let mut fixture = fixture();
        fixture.assembler.deferred_noncopyable_borrow_exits[0]
            .short_await
            .as_mut()
            .unwrap()
            .completed = None;
        let other = fixture.assembler.label();
        if mutation == "missing ordinary origin" {
            fixture
                .assembler
                .exception_owner_provenance
                .ordinary_await_cleanups
                .clear();
        }
        fixture.assembler.record_short_await_borrow_target(
            u32::from(mutation == "wrong none"),
            if mutation == "wrong send" {
                other
            } else {
                fixture.send
            },
            if mutation == "wrong end" {
                other
            } else {
                fixture.end
            },
            if mutation == "wrong yielded" {
                other
            } else {
                fixture.yielded
            },
            fixture.yielded_end,
            if mutation == "wrong cleanup" {
                other
            } else {
                fixture.cleanup
            },
        );
        assert_result(fixture, false, false);
    }
}
