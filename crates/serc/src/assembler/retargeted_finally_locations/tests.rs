use super::*;
use crate::assembler::ir::{BorrowFallthrough, EmittedInstruction, UnoptimizedCode};
use crate::target::opcodes::{RERAISE, RETURN_VALUE};

thread_local! {
    static EVENTS: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(super) fn trace(event: &'static str) {
    EVENTS.with_borrow_mut(|events| events.push(event));
}

#[test]
fn retargeted_finally_public_producer_emits_a_receipt() {
    EVENTS.with_borrow_mut(Vec::clear);
    crate::compile_with_options(
        "async def sample(manager, protocol, registration):\n    try:\n        async with manager:\n            raise Error\n    except Error as error:\n        raise\n    finally:\n        try:\n            if protocol is not None:\n                await protocol.close()\n        finally:\n            await registration.revoke()\n",
        "retargeted_finally.py", &crate::CompileOptions {
            python_version: crate::PythonVersion::Py314, ..crate::CompileOptions::default()
        },
    ).unwrap();
    EVENTS.with_borrow(|events| assert!(events.contains(&"replay"), "{events:?}"));
}

fn label(index: u32) -> Label {
    Label::from_u32(index)
}

fn record(opcode: crate::target::Opcode, operand: Operand, line: i32, depth: u32) -> Instruction {
    Instruction::new(
        opcode,
        operand,
        if line < 0 {
            SourceLocation::NONE
        } else {
            SourceLocation::new(line, line, 0, 8)
        },
        Some(depth),
    )
}

fn fixture() -> Assembler {
    let op = |opcode, value, line, depth| {
        Item::Instruction(record(opcode, Operand::Value(value), line, depth))
    };
    let jump = |target, line| {
        Item::Instruction(record(
            JUMP_FORWARD,
            Operand::Forward(label(target)),
            line,
            0,
        ))
    };
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Label(label(0)),
        jump(1, 1),
        Item::Label(label(1)),
        op(POP_TOP, 0, 7, 4),
        Item::Label(label(2)),
        op(POP_EXCEPT, 0, 7, 3),
        op(POP_TOP, 0, 7, 2),
        op(POP_TOP, 0, 7, 1),
        op(POP_TOP, 0, 7, 0),
        Item::Label(label(3)),
        jump(5, 7),
    ]);
    assembler.python_version = crate::PythonVersion::Py314;
    let emitted = EmittedJump(EmittedInstruction {
        stream: assembler.emission_stream,
        item_index: 10,
    });
    assembler.record_manager_suppression_exit(emitted, label(1), label(2), label(5));
    assert_eq!(
        assembler
            .exception_owner_provenance
            .manager_suppression_exits
            .len(),
        1
    );
    let mut bridge = record(JUMP_FORWARD, Operand::Forward(label(10)), -1, 0);
    bridge.jump_policy.prevent_small_exit_inlining();
    bridge
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut pop = record(NOP, Operand::Value(0), -1, 0);
    pop.fusion_policy.mark_converted_pop_block();
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items.extend([
        Item::Label(label(4)),
        op(RERAISE, 1, -1, 0),
        Item::Label(label(5)),
        Item::Label(label(6)),
        Item::Instruction(bridge),
        Item::Label(label(7)),
        op(RERAISE, 1, -1, 0),
        Item::Label(label(8)),
        jump(10, 9),
        Item::Label(label(9)),
        op(RERAISE, 1, -1, 0),
        Item::Label(label(10)),
        Item::Label(label(11)),
        Item::Instruction(pop),
        Item::Label(label(12)),
        op(NOP, 0, 11, 0),
        Item::Label(label(13)),
        Item::Label(label(14)),
        op(LOAD_FAST, 0, 12, 1),
        Item::Instruction(record(POP_JUMP_IF_NONE, Operand::Forward(label(15)), 12, 0)),
        op(NOT_TAKEN, 0, 12, 0),
        Item::Label(label(16)),
        op(LOAD_FAST, 1, 13, 1),
        op(POP_TOP, 0, 13, 0),
        Item::Label(label(15)),
        Item::Label(label(17)),
        Item::Instruction(pop),
        op(LOAD_FAST, 1, 15, 1),
        op(RETURN_VALUE, 0, 15, 0),
        Item::Label(label(18)),
        op(PUSH_EXC_INFO, 0, -1, 1),
        op(RERAISE, 0, -1, 0),
    ]);
    assembler.instruction_count = records(assembler.linear_items()).count();
    assembler.next_label = 19;
    assembler.exception_regions.push(ExceptionRegion {
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(label(14), label(17)),
        start: label(14),
        end: label(17),
        target: label(18),
        depth: 0,
        preserve_lasti: false,
    });
    assembler.exception_regions.push(ExceptionRegion {
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(label(1), label(4)),
        start: label(1),
        end: label(4),
        target: label(9),
        depth: 3,
        preserve_lasti: true,
    });
    let provenance = &mut assembler.exception_owner_provenance;
    provenance.source_boundaries.insert(label(10));
    provenance.block_boundaries.insert(label(10));
    provenance.setup_nop_ranges.push((label(12), label(13)));
    provenance
        .setups
        .push((label(14), Some(SourceLocation::new(11, 11, 0, 8))));
    provenance
        .compound_try_owner_exits
        .push((label(6), label(10)));
    assembler
}

fn capture(assembler: &mut Assembler) {
    assembler.record_original_normal_finally_array(12);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .original_normal_finally_arrays
            .len(),
        1
    );
    assembler.record_retargeted_normal_finally_prefix(12);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes
            .len(),
        1
    );
}

#[test]
fn retargeted_finally_requires_the_actual_one_hop_receipt() {
    for prepared in [false, true] {
        let mut assembler = fixture();
        capture(&mut assembler);
        if prepared {
            assembler.used_constant_indices(0);
            assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
        }
        let mut pipeline = assembler.into_optimization_pipeline();
        let original = pipeline.graph.copy_items();
        pipeline.propagate_retargeted_normal_finally_locations();
        assert_eq!(pipeline.graph.copy_items(), original);
        assert_eq!(pipeline.retargeted_finally_thread_candidates().len(), 1);
        pipeline.thread_forward_jumps();
        assert!(
            pipeline
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[0]
                .threaded
                .is_some()
        );
        let source = records(
            pipeline
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[0]
                .threaded
                .as_ref()
                .unwrap(),
        )
        .last()
        .unwrap();
        assert_eq!(source.operand(), Operand::Forward(label(10)));
        assert_eq!(source.pre_fold_extra_instructions(), 1);
        pipeline.initial_reachable = None;
        pipeline.remove_unreachable_instructions();
        pipeline.propagate_retargeted_normal_finally_locations();
        assert_eq!(marker_location(&pipeline).line, 7);
        let once = pipeline.graph.copy_items();
        pipeline.propagate_retargeted_normal_finally_locations();
        assert_eq!(pipeline.graph.copy_items(), once);
    }
}

fn pipeline(prepared: bool) -> OptimizationPipeline {
    let mut assembler = fixture();
    capture(&mut assembler);
    if prepared {
        assembler.used_constant_indices(0);
        assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    }
    assembler.into_optimization_pipeline()
}

fn position(pipeline: &OptimizationPipeline, item: Item) -> ItemPosition {
    pipeline
        .graph
        .iter()
        .find_map(|(block, contents)| {
            contents
                .items()
                .iter()
                .position(|candidate| *candidate == item)
                .map(|offset| ItemPosition::new(block, offset))
        })
        .unwrap()
}

fn marker_location(pipeline: &OptimizationPipeline) -> SourceLocation {
    let mut found = false;
    pipeline
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Label(entry) if *entry == label(10) => {
                found = true;
                None
            }
            Item::Instruction(instruction) if found => Some(instruction.location),
            _ => None,
        })
        .unwrap()
}

#[test]
fn retargeted_finally_transports_only_the_actual_borrow_mark() {
    for prepared in [false, true] {
        for mutation in 0..8 {
            let mut pipeline = pipeline(prepared);
            let original = *records(
                &pipeline
                    .exception_owner_provenance
                    .retargeted_normal_finally_prefixes[0]
                    .bridge,
            )
            .last()
            .unwrap();
            let at = position(&pipeline, Item::Instruction(original));
            let receipt = pipeline.retargeted_finally_borrow_candidates(&[at]);
            assert_eq!(receipt.len(), 1);
            if mutation != 1 {
                pipeline.graph.mark_borrow_empty_block(at);
            }
            match mutation {
                2 => pipeline.graph.instruction_mut(at).unwrap().location.column += 1,
                3 => pipeline.graph.insert_item(at, Item::Label(label(99))),
                4 => pipeline.graph.insert_item(at, Item::Instruction(original)),
                5 => pipeline.exception_regions[0].depth += 1,
                6 => {
                    let source = *pipeline
                        .exception_owner_provenance
                        .retargeted_normal_finally_prefixes[0]
                        .manager
                        .tail
                        .last()
                        .unwrap();
                    let source_at = position(&pipeline, source);
                    pipeline
                        .graph
                        .instruction_mut(source_at)
                        .unwrap()
                        .location
                        .column += 1;
                }
                7 => {
                    let moved = pipeline.graph.remove_item(at);
                    pipeline.graph.insert_item(at, moved);
                    pipeline.graph.insert_item(at, Item::Instruction(original));
                }
                _ => {}
            }
            pipeline.seal_retargeted_finally_borrow_marks(receipt);
            let transported = *records(
                &pipeline
                    .exception_owner_provenance
                    .retargeted_normal_finally_prefixes[0]
                    .bridge,
            )
            .last()
            .unwrap();
            assert_eq!(
                transported.jump_policy.borrow_empty_block(),
                mutation == 0,
                "{prepared} {mutation}"
            );
            if mutation == 0 {
                assert_eq!(pipeline.retargeted_finally_thread_candidates().len(), 1);
            }
        }
    }
}

#[test]
fn retargeted_finally_transports_only_actual_initial_borrow_writes() {
    for prepared in [false, true] {
        for mutation in 0..14 {
            let mut pipeline = pipeline(prepared);
            let receipt = pipeline.retargeted_finally_initial_borrow_candidates();
            assert_eq!(receipt.len(), 1);
            let at = receipt[0].position;
            let original = receipt[0].original;
            let expected = receipt[0].expected;
            if mutation != 1 {
                pipeline
                    .graph
                    .instruction_mut(at)
                    .unwrap()
                    .mark_initial_borrow_jump_target(label(5));
            }
            match mutation {
                2 => pipeline.graph.instruction_mut(at).unwrap().location.column += 1,
                3 => {
                    let source = pipeline.graph.instruction_mut(at).unwrap();
                    source.set_operand(Operand::Forward(label(99)));
                    source.mark_initial_borrow_jump_target(label(99));
                }
                4 => pipeline.graph.insert_item(at, Item::Label(label(99))),
                5 => pipeline.graph.insert_item(at, Item::Instruction(expected)),
                6 => pipeline.exception_regions[0].depth += 1,
                7 => {
                    let prefix = *pipeline
                        .exception_owner_provenance
                        .retargeted_normal_finally_prefixes[0]
                        .prefix
                        .last()
                        .unwrap();
                    let prefix_at = position(&pipeline, prefix);
                    pipeline
                        .graph
                        .instruction_mut(prefix_at)
                        .unwrap()
                        .location
                        .column += 1;
                }
                8 => {
                    let first = *records(
                        &pipeline
                            .exception_owner_provenance
                            .retargeted_normal_finally_prefixes[0]
                            .manager
                            .tail,
                    )
                    .next()
                    .unwrap();
                    let first_at = position(&pipeline, Item::Instruction(first));
                    pipeline
                        .graph
                        .instruction_mut(first_at)
                        .unwrap()
                        .location
                        .column += 1;
                }
                9 => {
                    *pipeline
                        .exception_owner_provenance
                        .retargeted_normal_finally_prefixes[0]
                        .manager
                        .tail
                        .last_mut()
                        .unwrap() = Item::Instruction(expected);
                }
                10 => {
                    let origin = &mut pipeline
                        .exception_owner_provenance
                        .retargeted_normal_finally_prefixes[0];
                    origin.threaded = Some(origin.manager.tail.clone());
                }
                11 => pipeline
                    .graph
                    .instruction_mut(at)
                    .unwrap()
                    .set_operand(Operand::Forward(label(99))),
                12 => pipeline
                    .graph
                    .instruction_mut(at)
                    .unwrap()
                    .jump_policy
                    .prevent_small_exit_inlining(),
                13 => {
                    let moved = pipeline.graph.remove_item(at);
                    pipeline.graph.insert_item(at, Item::Instruction(original));
                    pipeline.graph.insert_item(at, moved);
                }
                _ => {}
            }
            let before = {
                let origin = &pipeline
                    .exception_owner_provenance
                    .retargeted_normal_finally_prefixes[0];
                (origin.manager.tail.clone(), origin.threaded.clone())
            };
            pipeline.seal_retargeted_finally_initial_borrow_marks(receipt);
            let origin = &pipeline
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[0];
            if mutation == 0 {
                assert_eq!(
                    origin.manager.tail.last(),
                    Some(&Item::Instruction(expected))
                );
                assert!(
                    pipeline
                        .retargeted_finally_initial_borrow_candidates()
                        .is_empty()
                );
                assert_eq!(pipeline.retargeted_finally_thread_candidates().len(), 1);
                pipeline.thread_forward_jumps();
                let origin = &pipeline
                    .exception_owner_provenance
                    .retargeted_normal_finally_prefixes[0];
                let source = records(origin.threaded.as_ref().unwrap()).last().unwrap();
                assert_eq!(source.operand(), Operand::Forward(label(10)));
                assert_eq!(source.initial_borrow_jump_target(), None);
                pipeline.initial_reachable = None;
                pipeline.remove_unreachable_instructions();
                pipeline.propagate_retargeted_normal_finally_locations();
                assert_eq!(marker_location(&pipeline).line, 7);
            } else {
                assert_eq!(
                    (origin.manager.tail.clone(), origin.threaded.clone()),
                    before,
                    "{prepared} {mutation}"
                );
            }
        }
    }
}

#[test]
fn retargeted_finally_binds_every_original_record_and_policy() {
    let baseline = pipeline(false);
    let origin = &baseline
        .exception_owner_provenance
        .retargeted_normal_finally_prefixes[0];
    let originals = origin
        .manager
        .tail
        .iter()
        .chain(&origin.bridge)
        .chain(&origin.prefix)
        .chain(&origin.end)
        .chain(&origin.handler)
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(*instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    for prepared in [false, true] {
        for original in &originals {
            for mutation in 0..7 {
                let mut pipeline = pipeline(prepared);
                let at = position(&pipeline, Item::Instruction(*original));
                let record = pipeline.graph.instruction_mut(at).unwrap();
                match mutation {
                    0 => record.location.column += 1,
                    1 => record.local_policy.add_hidden_exception_owner_slots(1),
                    2 => record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    3 => record.no_location_policy.preserve_location(),
                    4 => record.jump_policy.prevent_small_exit_inlining(),
                    5 => record.fusion_policy.mark_converted_pop_block(),
                    _ => record.set_operand(Operand::Value(77)),
                }
                if record == original {
                    continue;
                }
                assert!(
                    pipeline.retargeted_finally_thread_candidates().is_empty(),
                    "{prepared} {original:?} {mutation}"
                );
            }
        }
    }
}

#[test]
fn retargeted_finally_distinguishes_terminal_and_interior_aliases() {
    for prepared in [false, true] {
        for mutation in 0..8 {
            let mut pipeline = pipeline(prepared);
            let origin = &pipeline
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[0];
            let conditional = *origin.prefix.last().unwrap();
            let not_taken = pipeline.graph.iter_items().copied().find(|item| matches!(item, Item::Instruction(instruction) if instruction.opcode == NOT_TAKEN)).unwrap();
            let at = position(
                &pipeline,
                if mutation == 0 {
                    not_taken
                } else {
                    conditional
                },
            );
            match mutation {
                0 | 1 => pipeline.graph.insert_item(at, Item::Label(label(99))),
                2 => pipeline
                    .graph
                    .instruction_mut(at)
                    .unwrap()
                    .set_operand(Operand::Forward(label(16))),
                3 => pipeline
                    .graph
                    .instruction_mut(at)
                    .unwrap()
                    .jump_policy
                    .prevent_small_exit_inlining(),
                4 => {
                    let at = position(&pipeline, Item::Label(label(17)));
                    pipeline.graph.insert_item(at, Item::Label(label(99)));
                }
                5 => {
                    let at = position(&pipeline, Item::Label(label(17)));
                    pipeline
                        .graph
                        .insert_item(at, Item::Instruction(record(NOP, Operand::Value(0), 22, 0)));
                }
                6 => pipeline
                    .exception_regions
                    .push(pipeline.exception_regions[0]),
                7 => {
                    pipeline.exception_regions[0].start = label(13);
                    pipeline.exception_regions[0].end = label(15);
                }
                _ => unreachable!(),
            }
            assert_eq!(
                pipeline.retargeted_finally_thread_candidates().len(),
                usize::from(matches!(mutation, 0 | 7)),
                "{prepared} {mutation}"
            );
        }
    }
}

fn ready_to_replay(prepared: bool) -> OptimizationPipeline {
    let mut pipeline = pipeline(prepared);
    pipeline.thread_forward_jumps();
    assert!(
        pipeline
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes[0]
            .threaded
            .is_some()
    );
    // The standalone fixture enters after the pipeline's initial reachability pass.
    pipeline.initial_reachable = None;
    pipeline.remove_unreachable_instructions();
    pipeline
}

#[test]
fn retargeted_finally_revalidates_full_late_records() {
    for prepared in [false, true] {
        let baseline = ready_to_replay(prepared);
        let items = baseline.graph.copy_items();
        let origin = &baseline
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes[0];
        let mut indices = Vec::new();
        for array in [
            &origin.prefix,
            &origin.end,
            &origin.handler,
            origin.threaded.as_ref().unwrap(),
        ] {
            let start = anchored_start(&items, array).unwrap();
            for (offset, item) in array.iter().enumerate() {
                if matches!(item, Item::Instruction(_)) && !indices.contains(&(start + offset)) {
                    indices.push(start + offset);
                }
            }
        }
        for index in indices {
            for mutation in 0..7 {
                let mut pipeline = ready_to_replay(prepared);
                let at = pipeline
                    .graph
                    .iter()
                    .flat_map(|(block, contents)| {
                        (0..contents.items().len())
                            .map(move |offset| ItemPosition::new(block, offset))
                    })
                    .nth(index)
                    .unwrap();
                let record = pipeline.graph.instruction_mut(at).unwrap();
                match mutation {
                    0 => record.location.column += 1,
                    1 => record.local_policy.add_hidden_exception_owner_slots(1),
                    2 => record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    3 => record.no_location_policy.preserve_location(),
                    4 => record.jump_policy.prevent_small_exit_inlining(),
                    5 => record.fusion_policy.mark_converted_pop_block(),
                    _ => record.set_operand(Operand::Value(77)),
                }
                if Item::Instruction(*record) == items[index] {
                    continue;
                }
                let before = pipeline.graph.copy_items();
                pipeline.propagate_retargeted_normal_finally_locations();
                assert_eq!(
                    pipeline.graph.copy_items(),
                    before,
                    "{prepared} {index} {mutation}"
                );
            }
        }
    }
}

#[test]
fn retargeted_finally_revalidates_aliases_regions_metadata_and_liveness() {
    for prepared in [false, true] {
        for mutation in 0..17 {
            let mut pipeline = ready_to_replay(prepared);
            match mutation {
                0 => pipeline.exception_regions[0].depth += 1,
                1 => pipeline.exception_regions[0].preserve_lasti = true,
                2 => pipeline.exception_regions[0].is_scope_fallback = true,
                3 => pipeline.exception_regions[0].target = label(9),
                4 => pipeline.exception_regions[0].start = label(12),
                5 => pipeline.exception_regions[0].end = label(16),
                6 => pipeline
                    .exception_regions
                    .push(pipeline.exception_regions[0]),
                7 => {
                    pipeline.exception_regions.remove(0);
                }
                8 => {
                    let first = pipeline
                        .graph
                        .iter_items()
                        .copied()
                        .find(|item| matches!(item, Item::Instruction(_)))
                        .unwrap();
                    let at = position(&pipeline, first);
                    pipeline
                        .graph
                        .instruction_mut(at)
                        .unwrap()
                        .set_operand(Operand::Forward(label(18)));
                }
                9 => {
                    let origin = &pipeline
                        .exception_owner_provenance
                        .retargeted_normal_finally_prefixes[0];
                    let source = *origin.threaded.as_ref().unwrap().last().unwrap();
                    let at = position(&pipeline, Item::Label(label(9)));
                    pipeline.graph.insert_item(at, source);
                }
                10 => {
                    let at = position(&pipeline, Item::Label(label(11)));
                    pipeline.graph.insert_item(at, Item::Label(label(99)));
                }
                11 => {
                    let at = position(&pipeline, Item::Label(label(17)));
                    pipeline.graph.insert_item(at, Item::Label(label(99)));
                }
                12 => {
                    let at = position(&pipeline, Item::Label(label(14)));
                    pipeline
                        .graph
                        .insert_item(at, Item::Instruction(record(NOP, Operand::Value(0), 20, 0)));
                }
                13 => pipeline.exception_owner_provenance.setup_nop_ranges.clear(),
                14 => pipeline.exception_owner_provenance.setups[0].1 = Some(SourceLocation::NONE),
                15 => {
                    pipeline
                        .exception_owner_provenance
                        .ordinary_try_setups
                        .insert(label(14), label(18));
                }
                16 => {
                    let at = position(&pipeline, Item::Label(label(9)));
                    pipeline.graph.insert_item(
                        at,
                        Item::Instruction(record(JUMP_FORWARD, Operand::Forward(label(13)), 30, 0)),
                    );
                }
                _ => unreachable!(),
            }
            let before = pipeline.graph.copy_items();
            pipeline.propagate_retargeted_normal_finally_locations();
            assert_eq!(pipeline.graph.copy_items(), before, "{prepared} {mutation}");
        }
    }
}

#[test]
fn retargeted_finally_rejects_wrong_capture_slots_and_duplicate_origins() {
    for mutation in 0..7 {
        let mut assembler = fixture();
        if mutation == 6 {
            assembler.record_original_normal_finally_array(12);
            *assembler
                .linear_items_mut()
                .iter_mut()
                .find(|item| **item == Item::Label(label(11)))
                .unwrap() = Item::Label(label(10));
            assembler.record_retargeted_normal_finally_prefix(12);
            assert!(
                assembler
                    .exception_owner_provenance
                    .retargeted_normal_finally_prefixes
                    .is_empty()
            );
            continue;
        }
        if mutation == 5 {
            capture(&mut assembler);
            assembler.record_retargeted_normal_finally_prefix(12);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .retargeted_normal_finally_prefixes
                    .len(),
                2
            );
            assert!(
                assembler
                    .into_optimization_pipeline()
                    .retargeted_finally_thread_candidates()
                    .is_empty()
            );
            continue;
        }
        let not_taken = assembler
            .linear_items_mut()
            .iter_mut()
            .find_map(|item| match item {
                Item::Instruction(instruction) if instruction.opcode == NOT_TAKEN => {
                    Some(instruction)
                }
                _ => None,
            })
            .unwrap();
        match mutation {
            0 => not_taken.opcode = NOP,
            1 => not_taken.set_operand(Operand::Value(1)),
            2 => not_taken.location.column += 1,
            3 => {
                *not_taken = Instruction::new(
                    not_taken.opcode,
                    not_taken.operand(),
                    not_taken.location,
                    Some(1),
                );
            }
            4 => {
                assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .clear();
            }
            _ => unreachable!(),
        }
        assembler.record_original_normal_finally_array(12);
        assembler.record_retargeted_normal_finally_prefix(12);
        assert!(
            assembler
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes
                .is_empty(),
            "{mutation}"
        );
    }
}

#[test]
fn retargeted_finally_constant_compaction_does_not_reinterpret_local_slots() {
    for prepared in [false, true] {
        let mut assembler = fixture();
        capture(&mut assembler);
        if prepared {
            assembler.used_constant_indices(3);
        }
        let original = assembler
            .exception_owner_provenance
            .retargeted_normal_finally_prefixes[0]
            .prefix
            .clone();
        assembler.remap_constant_indices(&[None, Some(0), None]);
        assert_eq!(
            assembler
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[0]
                .prefix,
            original
        );
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.thread_forward_jumps();
        assert!(
            pipeline
                .exception_owner_provenance
                .retargeted_normal_finally_prefixes[0]
                .threaded
                .is_some()
        );
    }
}
