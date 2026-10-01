use super::*;
use crate::assembler::exception_owners::OrdinaryAwaitEnd;
use crate::target::opcodes::{
    CLEANUP_THROW, GET_AWAITABLE, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL, PUSH_EXC_INFO, RERAISE,
    RETURN_VALUE,
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
    let send = record(SEND, Operand::Forward(label(5)), 2, 2);
    let end_send = record(END_SEND, Operand::Value(0), 2, 1);
    let cleanup = vec![Item::Label(label(4)), op(CLEANUP_THROW, 0, 2, 2)];
    let mut normal = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(label(20)),
        SourceLocation::NONE,
        Some(0),
    );
    normal.jump_policy.prevent_small_exit_inlining();
    normal.no_location_policy.preserve_location();
    normal
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut dead = record(JUMP_FORWARD, Operand::Forward(label(20)), 6, 0);
    dead.jump_policy.prevent_small_exit_inlining();
    let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    pop.fusion_policy.mark_converted_pop_block();
    let cleanup_send = record(SEND, Operand::Forward(label(26)), 11, 2);
    let cleanup_end = record(END_SEND, Operand::Value(0), 11, 1);
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
        Item::Instruction(send),
        Item::Label(label(3)),
        op(NOP, 0, 2, 2),
        Item::Label(label(31)),
        jump(JUMP_FORWARD, 2, 2, 2),
        cleanup[0],
        cleanup[1],
        Item::Label(label(5)),
        Item::Instruction(end_send),
        op(POP_TOP, 0, 2, 0),
        jump(JUMP_FORWARD, 10, 2, 0),
        Item::Label(label(6)),
        op(LOAD_FAST, 0, 3, 1),
        jump(POP_JUMP_IF_FALSE, 10, 3, 0),
        Item::Label(label(7)),
        op(LOAD_GLOBAL, 1, 4, 2),
        op(CALL, 0, 4, 1),
        op(POP_TOP, 0, 4, 0),
        Item::Label(label(10)),
        Item::Label(label(11)),
        Item::Instruction(normal),
        Item::Label(label(12)),
        op(PUSH_EXC_INFO, 0, 5, 2),
        op(POP_TOP, 0, 5, 1),
        op(RAISE_VARARGS, 0, 6, 1),
        Item::Label(label(13)),
        op(POP_EXCEPT, 0, 6, 0),
        Item::Instruction(dead),
        Item::Label(label(14)),
        op(RERAISE, 1, 6, 0),
        Item::Label(label(20)),
        Item::Label(label(21)),
        Item::Instruction(pop),
        Item::Label(label(22)),
        op(NOP, 0, 10, 0),
        Item::Label(label(23)),
        Item::Label(label(24)),
        op(LOAD_GLOBAL, 1, 11, 2),
        op(LOAD_FAST, 0, 11, 3),
        op(CALL, 1, 11, 1),
        op(GET_AWAITABLE, 0, 11, 1),
        op(LOAD_CONST, 0, 11, 2),
        Item::Label(label(25)),
        Item::Instruction(cleanup_send),
        Item::Label(label(27)),
        op(CLEANUP_THROW, 0, 11, 2),
        Item::Label(label(26)),
        Item::Instruction(cleanup_end),
        op(POP_TOP, 0, 11, 0),
        Item::Label(label(28)),
        jump(JUMP_FORWARD, 30, 11, 0),
        Item::Label(label(29)),
        op(PUSH_EXC_INFO, 0, 12, 2),
        op(RERAISE, 0, 12, 0),
        Item::Label(label(30)),
        op(LOAD_CONST, 0, 13, 1),
        op(RETURN_VALUE, 0, 13, 0),
    ]);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 32;
    let awaited_region = region(3, 31, 4, 2, false);
    assembler.exception_regions = vec![
        awaited_region,
        region(12, 13, 14, 1, true),
        region(24, 28, 29, 0, false),
    ];
    assembler.borrow_unreachable_blocks.insert(label(21));
    assembler.borrow_finally_entries.insert(label(21), None);
    assembler.borrow_jump_entry_labels.insert(label(21));
    assembler.preserved_block_boundaries.insert(label(21));
    let provenance = &mut assembler.exception_owner_provenance;
    provenance.source_boundaries.insert(label(20));
    provenance.block_boundaries.insert(label(20));
    provenance
        .compound_try_owner_exits
        .push((label(11), label(20)));
    provenance.setup_nop_ranges.push((label(22), label(23)));
    provenance.setups.push((label(24), Some(location(10))));
    provenance.ordinary_try_setups.insert(label(24), label(29));
    provenance.ordinary_await_cleanups.push(awaited_region);
    provenance.ordinary_await_ends = vec![
        OrdinaryAwaitEnd {
            send,
            entry: label(5),
            end_send,
            cleanup,
            bridge: None,
        },
        OrdinaryAwaitEnd {
            send: cleanup_send,
            entry: label(26),
            end_send: cleanup_end,
            cleanup: Vec::new(),
            bridge: None,
        },
    ];
    assembler
}

fn capture(assembler: &mut Assembler) {
    let pop = assembler
        .linear_items()
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(record) => Some(record),
            Item::Label(_) => None,
        })
        .position(|record| record.fusion_policy.is_converted_pop_block())
        .unwrap();
    assembler.record_independent_finally_entry(
        label(21),
        pop,
        location(2),
        location(3),
        location(4),
    );
    assert_eq!(
        assembler
            .exception_owner_provenance
            .independent_finally_entries
            .len(),
        1
    );
    assert_eq!(
        assembler.borrow_finally_entries.get(&label(21)),
        Some(&None)
    );
}

#[test]
fn independent_entry_waits_for_the_original_source_visit() {
    for prepared in [false, true] {
        for visited in [None, Some(false), Some(true)] {
            let mut assembler = fixture();
            capture(&mut assembler);
            if prepared {
                assembler.used_constant_indices(1);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            if let Some(visited) = visited {
                pipeline
                    .borrow_finally_jump_sources
                    .insert(label(5), visited);
            }
            let before = pipeline.graph.copy_items();
            pipeline.admit_visited_independent_finally_entries();
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
            assert!(pipeline.preserved_block_boundaries.contains(&label(21)));
            assert_eq!(
                pipeline.borrow_block_aliases.contains(&label(21)),
                visited == Some(true)
            );
        }
    }
}

#[test]
fn independent_entry_rejects_changed_pending_records() {
    let mut original = fixture();
    capture(&mut original);
    let witness = &mut original
        .exception_owner_provenance
        .independent_finally_entries[0];
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
    for record in records {
        for mutation in 0..5 {
            let mut assembler = fixture();
            capture(&mut assembler);
            let item = assembler
                .linear_items_mut()
                .iter_mut()
                .find(|item| **item == Item::Instruction(record))
                .unwrap();
            let Item::Instruction(changed) = item else {
                unreachable!()
            };
            match mutation {
                0 => changed.location.column += 1,
                1 => changed.local_policy.add_hidden_exception_owner_slots(1),
                2 => changed.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                3 => changed.no_location_policy.preserve_location(),
                _ => changed.jump_policy.prevent_small_exit_inlining(),
            }
            if *changed == record {
                continue;
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.borrow_finally_jump_sources.insert(label(5), true);
            pipeline.admit_visited_independent_finally_entries();
            assert!(
                pipeline.borrow_finally_entries.contains_key(&label(21)),
                "{record:?}, {mutation}"
            );
        }
    }
}

#[test]
fn independent_entry_uses_the_existing_original_barrier_walk() {
    for barrier in [false, true] {
        let mut assembler = fixture();
        if barrier {
            let prior = assembler.linear_items_mut().iter_mut().find(|item| {
                matches!(item, Item::Instruction(record) if record.opcode == LOAD_CONST && record.location == location(2))
            }).unwrap();
            let Item::Instruction(prior) = prior else {
                unreachable!()
            };
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
fn independent_entry_rejects_changed_pending_roles_and_regions() {
    for mutation in 0..14 {
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
            6 => {
                assembler.exception_regions[0].depth += 1;
            }
            7 => {
                assembler.exception_regions[1].preserve_lasti = false;
            }
            8 => {
                assembler.exception_regions[1].start = label(6);
            }
            9 => {
                assembler
                    .exception_regions
                    .push(assembler.exception_regions[1]);
            }
            10 => {
                assembler.exception_regions.remove(1);
            }
            11 => {
                assembler.push_linear_item(Item::Instruction(record(
                    JUMP_FORWARD,
                    Operand::Forward(label(10)),
                    15,
                    0,
                )));
            }
            12 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&label(20));
            }
            _ => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(label(21));
            }
        }
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.borrow_finally_jump_sources.insert(label(5), true);
        pipeline.admit_visited_independent_finally_entries();
        assert!(
            pipeline.borrow_finally_entries.contains_key(&label(21)),
            "mutation {mutation}"
        );
    }
}
