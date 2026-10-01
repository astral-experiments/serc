use super::*;
use crate::assembler::exception_owners::OrdinaryAwaitEnd;
use crate::target::opcodes::{
    CALL, CLEANUP_THROW, DELETE_FAST, GET_AWAITABLE, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL,
    POP_EXCEPT, POP_JUMP_IF_FALSE, PUSH_EXC_INFO, RERAISE, RETURN_VALUE, STORE_FAST,
};

fn label(index: u32) -> Label {
    Label::from_u32(index)
}
fn location(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 0, 8)
}
fn record(opcode: crate::target::Opcode, operand: Operand, line: i32, depth: u32) -> Instruction {
    Instruction::new(opcode, operand, location(line), Some(depth))
}
fn region(start: u32, end: u32, target: u32, depth: u32, lasti: bool) -> ExceptionRegion {
    ExceptionRegion {
        start: label(start),
        end: label(end),
        target: label(target),
        priority: ExceptionRegionPriority::OriginalRange(label(start), label(end)),
        depth,
        preserve_lasti: lasti,
        is_scope_fallback: false,
    }
}

fn fixture() -> Assembler {
    let op = |opcode, value, line, depth| {
        Item::Instruction(record(opcode, Operand::Value(value), line, depth))
    };
    let jump = |opcode, target, line, depth| {
        Item::Instruction(record(opcode, Operand::Forward(label(target)), line, depth))
    };
    let awaited = |entry, cleanup, line| OrdinaryAwaitEnd {
        send: record(SEND, Operand::Forward(label(entry)), line, 2),
        entry: label(entry),
        end_send: record(END_SEND, Operand::Value(0), line, 1),
        cleanup: vec![Item::Label(label(cleanup)), op(CLEANUP_THROW, 0, line, 2)],
        bridge: None,
    };
    let first = awaited(5, 4, 2);
    let last = awaited(15, 9, 4);
    let finalizer = awaited(26, 27, 11);
    let mut normal = record(JUMP_FORWARD, Operand::Forward(label(20)), 4, 0);
    normal.jump_policy.mark_original_empty_join();
    normal.jump_policy.retain_redundant_nop_after_threading();
    normal.jump_policy.prevent_small_exit_inlining();
    normal
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
    let unlocated = |opcode, operand, depth| {
        Instruction::new(opcode, operand, SourceLocation::NONE, Some(depth))
    };
    let author = [
        unlocated(POP_EXCEPT, Operand::Value(0), 0),
        unlocated(LOAD_CONST, Operand::Value(1), 1),
        unlocated(STORE_FAST, Operand::Value(1), 0),
        unlocated(DELETE_FAST, Operand::Value(1), 0),
        unlocated(JUMP_FORWARD, Operand::Forward(label(20)), 0),
    ];
    let mut current = author;
    current[4].allow_unconditional_block_inlining();
    current[4].jump_policy.prevent_small_exit_inlining();
    let mut pop = unlocated(NOP, Operand::Value(0), 0);
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    pop.fusion_policy.mark_converted_pop_block();
    let mut setup = record(NOP, Operand::Value(0), 10, 0);
    setup.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Label(label(0)),
        op(LOAD_FAST, 0, 1, 1),
        jump(POP_JUMP_IF_FALSE, 6, 1, 0),
        Item::Label(label(1)),
        op(LOAD_GLOBAL, 1, 2, 2),
        op(CALL, 0, 2, 1),
        op(GET_AWAITABLE, 0, 2, 1),
        op(LOAD_CONST, 0, 2, 2),
        Item::Label(label(2)),
        Item::Instruction(first.send),
        Item::Label(label(3)),
        op(NOP, 0, 2, 2),
        Item::Label(label(31)),
        jump(JUMP_FORWARD, 2, 2, 2),
        first.cleanup[0],
        first.cleanup[1],
        Item::Label(first.entry),
        Item::Instruction(first.end_send),
        op(POP_TOP, 0, 2, 0),
        jump(JUMP_FORWARD, 10, 2, 0),
        Item::Label(label(6)),
        op(LOAD_GLOBAL, 1, 4, 2),
        op(CALL, 0, 4, 1),
        op(GET_AWAITABLE, 0, 4, 1),
        op(LOAD_CONST, 0, 4, 2),
        Item::Label(label(7)),
        Item::Instruction(last.send),
        Item::Label(label(8)),
        op(NOP, 0, 4, 2),
        Item::Label(label(32)),
        jump(JUMP_FORWARD, 7, 4, 2),
        last.cleanup[0],
        last.cleanup[1],
        Item::Label(last.entry),
        Item::Instruction(last.end_send),
        op(POP_TOP, 0, 4, 0),
        Item::Label(label(10)),
        Item::Label(label(11)),
        Item::Instruction(normal),
        Item::Label(label(12)),
        op(PUSH_EXC_INFO, 0, 5, 2),
        op(POP_TOP, 0, 5, 1),
        Item::Label(label(13)),
        Item::Instruction(current[0]),
        Item::Instruction(current[1]),
        Item::Instruction(current[2]),
        Item::Instruction(current[3]),
        Item::Instruction(current[4]),
        Item::Label(label(14)),
        op(RERAISE, 1, 6, 0),
        Item::Label(label(20)),
        Item::Label(label(21)),
        Item::Instruction(pop),
        Item::Label(label(22)),
        Item::Instruction(setup),
        Item::Label(label(23)),
        Item::Label(label(33)),
        Item::Label(label(24)),
        op(LOAD_GLOBAL, 1, 11, 2),
        op(LOAD_FAST, 0, 11, 3),
        op(CALL, 1, 11, 1),
        op(GET_AWAITABLE, 0, 11, 1),
        op(LOAD_CONST, 0, 11, 2),
        Item::Label(label(25)),
        Item::Instruction(finalizer.send),
        Item::Label(label(36)),
        op(NOP, 0, 11, 2),
        Item::Label(label(37)),
        jump(JUMP_FORWARD, 25, 11, 2),
        finalizer.cleanup[0],
        finalizer.cleanup[1],
        Item::Label(finalizer.entry),
        Item::Instruction(finalizer.end_send),
        op(POP_TOP, 0, 11, 0),
        Item::Label(label(28)),
        jump(JUMP_FORWARD, 30, 11, 0),
        Item::Label(label(29)),
        op(PUSH_EXC_INFO, 0, 12, 2),
        op(RERAISE, 0, 12, 0),
        Item::Label(label(34)),
        jump(JUMP_FORWARD, 30, 12, 0),
        Item::Label(label(35)),
        op(PUSH_EXC_INFO, 0, 12, 2),
        op(RERAISE, 0, 12, 0),
        Item::Label(label(30)),
        op(LOAD_CONST, 0, 13, 1),
        op(RETURN_VALUE, 0, 13, 0),
    ]);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 38;
    assembler.exception_regions = vec![
        region(3, 31, 4, 2, false),
        region(8, 32, 9, 2, false),
        region(12, 13, 14, 1, true),
        region(24, 28, 29, 0, false),
        region(33, 34, 35, 0, false),
        region(36, 37, 27, 2, false),
    ];
    assembler.borrow_unreachable_blocks.insert(label(21));
    assembler.borrow_finally_entries.insert(label(21), None);
    assembler.borrow_jump_entry_labels.insert(label(21));
    assembler.preserved_block_boundaries.insert(label(21));
    let provenance = &mut assembler.exception_owner_provenance;
    provenance.source_boundaries.extend([label(20), label(11)]);
    provenance.block_boundaries.extend([label(20), label(11)]);
    provenance
        .compound_try_owner_exits
        .push((label(11), label(20)));
    provenance.setup_nop_ranges.push((label(22), label(23)));
    provenance
        .setups
        .extend([(label(33), Some(location(10))), (label(24), None)]);
    provenance.ordinary_try_setups.insert(label(24), label(29));
    provenance.ordinary_await_cleanups.extend([
        region(3, 31, 4, 2, false),
        region(8, 32, 9, 2, false),
        region(36, 37, 27, 2, false),
    ]);
    provenance.ordinary_await_ends = vec![first, last, finalizer];
    provenance
        .original_named_handler_cleanup_joins
        .push((label(13), author));
    provenance.named_handler_cleanup_targets.insert(label(14));
    assembler
}

fn capture(assembler: &mut Assembler) {
    let ordinal = assembler
        .linear_items()
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(record) => Some(record),
            Item::Label(_) => None,
        })
        .position(|record| record.fusion_policy.is_converted_pop_block())
        .unwrap();
    assembler.record_retained_normal_finally_entry(label(21), ordinal, location(2), location(4));
    assert_eq!(
        assembler
            .exception_owner_provenance
            .retained_normal_finally_entries
            .len(),
        1
    );
    assert_eq!(
        assembler.borrow_finally_entries.get(&label(21)),
        Some(&None)
    );
}

#[test]
fn retained_normal_entry_waits_for_original_visit_through_preparation_and_remapping() {
    for prepared in [false, true] {
        for remapped in [false, true] {
            for visited in [None, Some(false), Some(true)] {
                let mut assembler = fixture();
                capture(&mut assembler);
                let author = assembler
                    .exception_owner_provenance
                    .original_named_handler_cleanup_joins[0];
                if prepared {
                    assembler.used_constant_indices(2);
                }
                if remapped {
                    assembler.remap_constant_indices(&[Some(3), Some(2)]);
                }
                assert_eq!(
                    assembler
                        .exception_owner_provenance
                        .original_named_handler_cleanup_joins[0],
                    author
                );
                let mut pipeline = assembler.into_optimization_pipeline();
                if let Some(visited) = visited {
                    pipeline
                        .borrow_finally_jump_sources
                        .insert(label(5), visited);
                }
                let before = pipeline.graph.copy_items();
                pipeline.admit_visited_retained_normal_finally_entries();
                assert_eq!(pipeline.graph.copy_items(), before);
                assert_eq!(
                    pipeline.borrow_finally_entries.contains_key(&label(21)),
                    visited != Some(true)
                );
                assert_eq!(
                    pipeline.borrow_unreachable_blocks.contains(&label(21)),
                    visited != Some(true)
                );
                assert_eq!(
                    pipeline.borrow_jump_entry_labels.contains(&label(21)),
                    visited != Some(true)
                );
                assert_eq!(
                    pipeline.borrow_block_aliases.contains(&label(21)),
                    visited == Some(true)
                );
                assert!(pipeline.preserved_block_boundaries.contains(&label(21)));
            }
        }
    }
}

#[test]
fn retained_normal_entry_rejects_every_changed_current_record() {
    let mut original = fixture();
    capture(&mut original);
    let witness = &mut original
        .exception_owner_provenance
        .retained_normal_finally_entries[0];
    let mut records = witness
        .arrays
        .iter()
        .flat_map(|(_, _, items)| items)
        .filter_map(|item| match item {
            Item::Instruction(record) => Some(*record),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    witness
        .prefix
        .visit_records_mut(|record| records.push(*record));
    for original in records {
        for mutation in 0..6 {
            let mut assembler = fixture();
            capture(&mut assembler);
            let Item::Instruction(changed) = assembler
                .linear_items_mut()
                .iter_mut()
                .find(|item| **item == Item::Instruction(original))
                .unwrap()
            else {
                unreachable!()
            };
            match mutation {
                0 => changed.location.column += 1,
                1 => changed.local_policy.add_hidden_exception_owner_slots(1),
                2 => changed.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                3 => changed.no_location_policy.preserve_location(),
                4 => changed.jump_policy.prevent_small_exit_inlining(),
                _ => changed.set_operand(Operand::Value(99)),
            }
            if *changed == original {
                continue;
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.borrow_finally_jump_sources.insert(label(5), true);
            pipeline.admit_visited_retained_normal_finally_entries();
            assert!(
                pipeline.borrow_finally_entries.contains_key(&label(21)),
                "{original:?}, {mutation}"
            );
        }
    }
}

#[test]
fn retained_normal_entry_preserves_the_author_snapshot() {
    for record in 0..5 {
        for prepared in [false, true] {
            let mut assembler = fixture();
            capture(&mut assembler);
            if prepared {
                assembler.used_constant_indices(2);
            }
            assembler.remap_constant_indices(&[Some(3), Some(2)]);
            assembler
                .exception_owner_provenance
                .original_named_handler_cleanup_joins[0]
                .1[record]
                .location
                .column += 1;
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.borrow_finally_jump_sources.insert(label(5), true);
            pipeline.admit_visited_retained_normal_finally_entries();
            assert!(pipeline.borrow_finally_entries.contains_key(&label(21)));
        }
    }
}

#[test]
fn retained_normal_entry_uses_the_existing_source_barrier_walk() {
    for barrier in [false, true] {
        let mut assembler = fixture();
        if barrier {
            let Item::Instruction(prior) = assembler.linear_items_mut().iter_mut().find(|item| matches!(item, Item::Instruction(record) if record.opcode == LOAD_CONST && record.location == location(2))).unwrap() else { unreachable!() };
            prior
                .no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        }
        capture(&mut assembler);
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.record_borrow_cleanup_calls();
        assert_eq!(
            pipeline.borrow_finally_jump_sources.get(&label(5)),
            Some(&!barrier)
        );
        assert_eq!(
            pipeline.borrow_finally_entries.contains_key(&label(21)),
            barrier
        );
    }
}

#[test]
fn retained_normal_entry_rejects_changed_roles_edges_and_region_groups() {
    for mutation in 0..30 {
        let mut assembler = fixture();
        capture(&mut assembler);
        match mutation {
            0 => {
                assembler.borrow_block_aliases.insert(label(5));
            }
            1 => {
                assembler.preserved_block_boundaries.insert(label(10));
            }
            2 => {
                assembler.borrow_unreachable_blocks.insert(label(11));
            }
            3 => {
                assembler.original_nop_targets.insert(label(13));
            }
            4 => {
                assembler
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(label(20), 1);
            }
            5 => {
                assembler
                    .exception_owner_provenance
                    .omitted_nop_locations
                    .insert(label(21), location(4));
            }
            6 => assembler.exception_regions[0].depth += 1,
            7 => assembler.exception_regions[2].preserve_lasti = false,
            8 => assembler.exception_regions[2].start = label(6),
            9 => assembler
                .exception_regions
                .push(assembler.exception_regions[2]),
            10 => {
                assembler.exception_regions.remove(2);
            }
            11 => assembler.push_linear_item(Item::Instruction(record(
                JUMP_FORWARD,
                Operand::Forward(label(10)),
                15,
                0,
            ))),
            12 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&label(20));
            }
            13 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(label(21));
            }
            14 => {
                assembler
                    .exception_owner_provenance
                    .named_handler_cleanup_targets
                    .remove(&label(14));
            }
            15 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(label(13));
            }
            16 => assembler.exception_regions[1].target = label(5),
            17 => assembler.exception_regions[1].end = label(15),
            18 => {
                assembler
                    .exception_owner_provenance
                    .original_named_handler_cleanup_joins
                    .clear();
            }
            19 => assembler
                .exception_owner_provenance
                .original_named_handler_cleanup_joins
                .push(
                    assembler
                        .exception_owner_provenance
                        .original_named_handler_cleanup_joins[0],
                ),
            20 => assembler.push_linear_item(Item::Instruction(record(
                JUMP_FORWARD,
                Operand::Forward(label(13)),
                15,
                0,
            ))),
            21 => {
                assembler.exception_owner_provenance.setups.remove(0);
            }
            22 => {
                assembler.exception_owner_provenance.setups.remove(1);
            }
            23 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .remove(&label(24));
            }
            24 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(label(33), label(35));
            }
            25 => {
                assembler.exception_regions[3].priority =
                    ExceptionRegionPriority::OriginalRange(label(23), label(28));
            }
            26 => assembler.exception_regions[4].depth = 1,
            27 => assembler.push_linear_item(Item::Instruction(record(
                JUMP_FORWARD,
                Operand::Forward(label(24)),
                15,
                0,
            ))),
            28 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(label(5));
            }
            _ => assembler.exception_owner_provenance.setup_nop_ranges[0].1 = label(24),
        }
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.borrow_finally_jump_sources.insert(label(5), true);
        pipeline.admit_visited_retained_normal_finally_entries();
        assert!(
            pipeline.borrow_finally_entries.contains_key(&label(21)),
            "mutation {mutation}"
        );
    }
}

#[test]
fn retained_normal_entry_rejects_unauthored_setup_and_handler_layouts() {
    for mutation in 0..12 {
        let mut assembler = fixture();
        match mutation {
            0 => {
                assembler.exception_owner_provenance.setups.remove(0);
            }
            1 => {
                assembler.exception_owner_provenance.setups.remove(1);
            }
            2 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .remove(&label(24));
            }
            3 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(label(33), label(35));
            }
            4 => {
                assembler.exception_regions[3].priority =
                    ExceptionRegionPriority::OriginalRange(label(23), label(28));
            }
            5 => assembler.exception_regions[4].end = label(30),
            6 => assembler.exception_regions[4].depth = 1,
            7 => {
                assembler.exception_regions.remove(4);
            }
            8 => assembler
                .exception_regions
                .push(assembler.exception_regions[3]),
            9 => {
                assembler
                    .exception_owner_provenance
                    .original_named_handler_cleanup_joins[0]
                    .1[0]
                    .location
                    .column += 1;
            }
            10 => assembler.push_linear_item(Item::Instruction(record(
                JUMP_FORWARD,
                Operand::Forward(label(24)),
                15,
                0,
            ))),
            _ => assembler.exception_owner_provenance.setup_nop_ranges[0].1 = label(24),
        }
        let ordinal = assembler
            .linear_items()
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record) => Some(record),
                Item::Label(_) => None,
            })
            .position(|record| record.fusion_policy.is_converted_pop_block())
            .unwrap();
        assert!(
            assembler
                .original_retained_normal_finally_entry(
                    label(21),
                    ordinal,
                    location(2),
                    location(4)
                )
                .is_none(),
            "mutation {mutation}"
        );
    }
}
