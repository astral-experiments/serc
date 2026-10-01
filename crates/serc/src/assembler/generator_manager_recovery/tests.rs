use super::*;
use crate::assembler::ExceptionPolicy;
use crate::assembler::block_graph::BlockGraph;
use crate::target::opcodes::{LOAD_CONST, NOP, POP_JUMP_IF_FALSE, POP_TOP, RERAISE, RETURN_VALUE};

struct Join {
    assembler: Assembler,
    exit: EmittedJump,
    source: EmittedJump,
    entry: Label,
    target: Label,
    continuation: Label,
    condition: SourceLocation,
}

fn location(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 0, 4)
}

fn fixture(version: PythonVersion, before: impl FnOnce(&mut Assembler, EmittedJump)) -> Join {
    fixture_with_sources(version, 1, before)
}

fn fixture_with_sources(
    version: PythonVersion,
    count: usize,
    before: impl FnOnce(&mut Assembler, EmittedJump),
) -> Join {
    let mut assembler = Assembler::with_python_version(version);
    let entry = assembler.label();
    let target = assembler.label();
    let continuation = assembler.label();
    let condition = location(2);
    assembler.set_location(location(1));
    assembler.emit_with_depth(NOP, 0, 2, ExceptionPolicy::default());
    assembler.set_location(condition);
    assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, target, 2, ExceptionPolicy::default());
    assembler.set_location(location(3));
    assembler.emit_with_depth(LOAD_CONST, 0, 3, ExceptionPolicy::default());
    assembler.emit_with_depth(POP_TOP, 0, 2, ExceptionPolicy::default());
    assembler.emit_forward_with_depth(JUMP_FORWARD, target, 2, ExceptionPolicy::default());
    assembler.set_location(location(4));
    assembler.emit_with_depth(NOP, 0, 2, ExceptionPolicy::default());
    assembler.mark(entry);
    let source =
        assembler.emit_forward_with_depth(JUMP_FORWARD, target, 2, ExceptionPolicy::default());
    assembler.record_normal_try_exit(entry, target, None);
    before(&mut assembler, source);
    assembler.mark_generator_manager_recovery_barrier(source, entry, target);
    for _ in 1..count {
        let additional = assembler.label();
        assembler.set_location(location(5));
        assembler.emit_with_depth(NOP, 0, 2, ExceptionPolicy::default());
        assembler.mark(additional);
        let emitted =
            assembler.emit_forward_with_depth(JUMP_FORWARD, target, 2, ExceptionPolicy::default());
        assembler.record_normal_try_exit(additional, target, None);
        assembler.mark_generator_manager_recovery_barrier(emitted, additional, target);
    }
    assembler.set_location(SourceLocation::NONE);
    assembler.emit_with_depth(RERAISE, 1, 2, ExceptionPolicy::default());
    assembler.mark(target);
    assembler.record_exception_owner_block_boundary(target);
    assembler.set_location(condition);
    let exit = assembler.emit_forward_with_depth(
        JUMP_FORWARD,
        continuation,
        2,
        ExceptionPolicy::default(),
    );
    assembler.preserve_redundant_jump_nop_after_threading(exit);
    Join {
        assembler,
        exit,
        source,
        entry,
        target,
        continuation,
        condition,
    }
}

fn accepts(join: &mut Join) -> bool {
    join.assembler.original_generator_manager_recovery_join(
        join.exit,
        join.continuation,
        join.condition,
        1,
    )
}

fn source_mut(join: &mut Join) -> &mut Instruction {
    let index = usize::try_from(join.source.instruction().item_index).unwrap();
    let Item::Instruction(instruction) = &mut join.assembler.linear_items_mut()[index] else {
        unreachable!()
    };
    instruction
}

#[test]
fn recovery_receipt_requires_the_real_default_to_barrier_write() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for mutation in [
            "intact",
            "already barrier",
            "reachable nop",
            "hidden",
            "folded",
            "owner",
            "missing pair",
            "duplicate pair",
        ] {
            let mut join = fixture(version, |assembler, source| match mutation {
                "intact" => {}
                "already barrier" => {
                    assembler.mark_borrow_fallthrough_barrier(source.instruction());
                }
                "reachable nop" => assembler.mark_retained_nop_borrow_fallthrough(source),
                "hidden" => assembler
                    .emitted_jump_mut(source, "test")
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                "folded" => assembler
                    .emitted_jump_mut(source, "test")
                    .add_pre_fold_extra_instructions(1),
                "owner" => assembler
                    .emitted_jump_mut(source, "test")
                    .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "missing pair" => assembler
                    .exception_owner_provenance
                    .normal_try_exits
                    .clear(),
                "duplicate pair" => {
                    let pair = assembler.exception_owner_provenance.normal_try_exits[0];
                    assembler
                        .exception_owner_provenance
                        .normal_try_exits
                        .push(pair);
                }
                _ => unreachable!(),
            });
            assert_eq!(
                source_mut(&mut join)
                    .no_location_policy
                    .borrow_fallthrough(),
                BorrowFallthrough::Barrier
            );
            let expected = version >= PythonVersion::Py314 && mutation == "intact";
            assert_eq!(
                join.assembler.generator_manager_recovery_sources.len(),
                usize::from(expected),
                "{version:?} {mutation}"
            );
            assert_eq!(accepts(&mut join), expected, "{version:?} {mutation}");
            assert!(!join.assembler.original_normal_finally_manager_join(
                join.exit,
                join.continuation,
                join.condition,
                1
            ));
        }
    }
}

#[test]
fn recovery_receipt_binds_the_original_source_head_and_registry() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for mutation in [
            "intact",
            "record",
            "location",
            "depth",
            "backward",
            "target",
            "owner",
            "hidden",
            "folded",
            "value operand",
            "source boundary",
            "block boundary",
            "entry alias",
            "missing label",
            "duplicate label",
            "missing receipt",
            "duplicate receipt",
            "missing pair",
            "duplicate pair",
            "conflicting pair",
            "incoming pair",
            "foreign incoming",
            "exception target",
            "borrow entry",
            "folded entry",
            "omitted slot",
            "prepared",
            "foreign stream",
            "stale exit",
            "outside interval",
            "moved source",
            "equal replacement",
        ] {
            let mut join = fixture(version, |_, _| {});
            let index = usize::try_from(join.source.instruction().item_index).unwrap();
            let original = *source_mut(&mut join);
            match mutation {
                "intact" => {}
                "record" => source_mut(&mut join)
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved),
                "location" => source_mut(&mut join).location.column += 1,
                "depth" => {
                    let mut changed = Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(join.target),
                        original.location,
                        Some(3),
                    );
                    changed
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                    *source_mut(&mut join) = changed;
                }
                "backward" => {
                    let target = join.target;
                    source_mut(&mut join).set_operand(Operand::Backward(target));
                }
                "target" => {
                    let target = join.continuation;
                    source_mut(&mut join).set_operand(Operand::Forward(target));
                }
                "owner" => {
                    source_mut(&mut join).set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                }
                "hidden" => source_mut(&mut join)
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                "folded" => source_mut(&mut join).add_pre_fold_extra_instructions(1),
                "value operand" => source_mut(&mut join).set_operand(Operand::Value(0)),
                "source boundary" => {
                    join.assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .insert(join.entry);
                }
                "block boundary" => {
                    join.assembler
                        .exception_owner_provenance
                        .block_boundaries
                        .insert(join.entry);
                }
                "entry alias" => {
                    let alias = join.assembler.label();
                    join.assembler.linear_items_mut()[index - 2] = Item::Label(alias);
                }
                "missing label" => {
                    join.assembler.linear_items_mut()[index - 1] = Item::Instruction(
                        Instruction::new(NOP, Operand::Value(0), location(4), Some(2)),
                    );
                }
                "duplicate label" => {
                    join.assembler.linear_items_mut()[0] = Item::Label(join.entry);
                }
                "missing receipt" => join.assembler.generator_manager_recovery_sources.clear(),
                "duplicate receipt" => {
                    let origin = &join.assembler.generator_manager_recovery_sources[0];
                    let duplicate = Source {
                        emitted: origin.emitted,
                        entry: origin.entry,
                        target: origin.target,
                        head: origin.head.clone(),
                        boundaries: origin.boundaries.clone(),
                        original: origin.original,
                    };
                    join.assembler
                        .generator_manager_recovery_sources
                        .push(duplicate);
                }
                "missing pair" => join
                    .assembler
                    .exception_owner_provenance
                    .normal_try_exits
                    .clear(),
                "duplicate pair" => join
                    .assembler
                    .exception_owner_provenance
                    .normal_try_exits
                    .push((join.entry, join.target)),
                "conflicting pair" => join
                    .assembler
                    .exception_owner_provenance
                    .normal_try_exits
                    .push((join.entry, join.continuation)),
                "incoming pair" => join
                    .assembler
                    .exception_owner_provenance
                    .normal_try_exits
                    .push((join.continuation, join.entry)),
                "foreign incoming" => {
                    join.assembler.linear_items_mut()[0] = Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(join.entry),
                        location(1),
                        Some(2),
                    ));
                }
                "exception target" => join.assembler.add_exception_region(
                    join.target,
                    join.continuation,
                    join.entry,
                    2,
                    true,
                ),
                "borrow entry" => {
                    join.assembler.borrow_jump_entry_labels.insert(join.entry);
                }
                "folded entry" => {
                    join.assembler
                        .folded_away_condition_targets
                        .insert(join.entry);
                }
                "omitted slot" => {
                    join.assembler
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .insert(join.entry, 1);
                }
                "prepared" => {
                    join.assembler.mark(join.continuation);
                    join.assembler
                        .emit_with_depth(RETURN_VALUE, 0, 1, ExceptionPolicy::default());
                    let graph = BlockGraph::from_items(
                        join.assembler.linear_items().to_vec(),
                        &rustc_hash::FxHashSet::default(),
                    );
                    let reachable = vec![true; graph.len()];
                    join.assembler.code = UnoptimizedCode::Prepared { graph, reachable };
                }
                "foreign stream" => join.exit = fixture(PythonVersion::Py314, |_, _| {}).exit,
                "stale exit" => join.exit = join.source,
                "outside interval" => {
                    assert!(!join.assembler.original_generator_manager_recovery_join(
                        join.exit,
                        join.continuation,
                        join.condition,
                        index
                    ));
                    continue;
                }
                "moved source" => join.assembler.linear_items_mut().swap(0, index),
                "equal replacement" => {
                    join.assembler.linear_items_mut()[0] = Item::Instruction(original);
                }
                _ => unreachable!(),
            }
            assert_eq!(accepts(&mut join), mutation == "intact", "{mutation}");
        }
    }
}

#[test]
fn generator_join_requires_exactly_one_recorded_recovery_source() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let mut join = fixture_with_sources(version, 2, |_, _| {});
        assert_eq!(join.assembler.generator_manager_recovery_sources.len(), 2);
        assert!(!accepts(&mut join));
        let mut join = fixture(version, |_, _| {});
        let mut original = *source_mut(&mut join);
        original
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Reachable);
        *source_mut(&mut join) = original;
        assert!(!accepts(&mut join));
        assert!(join.assembler.original_normal_finally_manager_join(
            join.exit,
            join.continuation,
            join.condition,
            1
        ));
    }
}

#[test]
fn generator_join_rejects_exceptional_entry_through_every_alias() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for position in ["selected", "before", "after", "inert"] {
            let mut join = fixture(version, |_, _| {});
            let alias = if position == "selected" {
                join.target
            } else {
                let alias = join.assembler.label();
                let index = usize::try_from(join.exit.instruction().item_index).unwrap();
                let position = if position == "before" {
                    index - 1
                } else {
                    index
                };
                let UnoptimizedCode::Linear(items) = &mut join.assembler.code else {
                    unreachable!()
                };
                items.insert(position, Item::Label(alias));
                join.exit.0.item_index += 1;
                alias
            };
            if position != "inert" {
                join.assembler
                    .add_exception_region(join.entry, join.target, alias, 2, true);
            }
            assert_eq!(accepts(&mut join), position == "inert", "{position}");
        }
    }
}
