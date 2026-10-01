use super::*;
use crate::assembler::ir::{EmissionStreamId, EmittedInstruction, UnoptimizedCode};
use crate::target::opcodes::{
    CALL, LOAD_CONST, LOAD_FAST, LOAD_FAST_BORROW, LOAD_GLOBAL, NOT_TAKEN, POP_JUMP_IF_TRUE,
    RETURN_VALUE,
};

fn label(value: u32) -> Label {
    Label::from_u32(value)
}
fn location(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 0, 8)
}
fn record(opcode: crate::target::Opcode, value: u32, line: i32, depth: u32) -> Item {
    Item::Instruction(Instruction::new(
        opcode,
        Operand::Value(value),
        location(line),
        Some(depth),
    ))
}
fn items_mut(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items
}
fn source_fixture() -> (Assembler, EmittedJump) {
    let mut condition = Instruction::new(
        POP_JUMP_IF_NONE,
        Operand::Forward(label(2)),
        location(3),
        Some(0),
    );
    condition.local_policy.add_hidden_exception_owner_slots(3);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(label(3)),
        location(3),
        Some(0),
    );
    jump.jump_policy.mark_original_empty_join();
    jump.jump_policy.retain_redundant_nop_after_threading();
    jump.jump_policy.mark_borrow_empty_block();
    jump.no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Label(label(0)),
        record(LOAD_FAST, 0, 1, 1),
        Item::Instruction(condition),
        record(NOT_TAKEN, 0, 3, 0),
        Item::Label(label(1)),
        record(LOAD_FAST, 1, 4, 1),
        record(RAISE_VARARGS, 1, 4, 0),
        Item::Label(label(2)),
        Item::Instruction(jump),
    ]);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 5;
    assembler
        .exception_owner_provenance
        .source_boundaries
        .insert(label(2));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(label(2));
    let emitted = EmittedJump(EmittedInstruction {
        stream: assembler.emission_stream,
        item_index: 8,
    });
    (assembler, emitted)
}
fn fixture(prepared: bool) -> OptimizationPipeline {
    fixture_with_argument(prepared, false, false)
}

#[test]
fn guarded_loop_borrow_route_tracks_only_the_real_policy_writer() {
    let (mut assembler, emitted) = source_fixture();
    assembler.record_guarded_loop_borrow_nop(emitted);
    assembler.prevent_finalizer_borrow_jump_threading_target(emitted);
    let source = &assembler.exception_owner_provenance.guarded_loop_nops[0];
    assert!(source.matches_emitted(&assembler, emitted));
    assert!(!source.jump.jump_policy.allows_borrow_threading_target());
    assert!(
        assembler
            .capture_guarded_loop_borrow_route(emitted)
            .is_none()
    );

    let (mut assembler, emitted) = source_fixture();
    assembler.prevent_borrow_jump_threading_target(emitted);
    assembler.record_guarded_loop_borrow_nop(emitted);
    assert!(
        assembler
            .exception_owner_provenance
            .guarded_loop_nops
            .is_empty()
    );
}

#[test]
fn guarded_loop_borrow_route_rejects_changed_or_detached_records() {
    for mode in 0..18 {
        let (mut assembler, emitted) = source_fixture();
        assembler.record_guarded_loop_borrow_nop(emitted);
        let original = assembler.capture_guarded_loop_borrow_route(emitted);
        assert!(original.is_some());
        if mode != 0 {
            assembler.prevent_borrow_jump_threading_target(emitted);
        }
        match mode {
            0 => {}
            1 => assembler.emitted_jump_mut(emitted, "test").location = location(30),
            2 => assembler
                .emitted_jump_mut(emitted, "test")
                .set_operand(Operand::Forward(label(4))),
            3 => assembler
                .emitted_jump_mut(emitted, "test")
                .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
            4 => {
                assembler
                    .exception_owner_provenance
                    .guarded_loop_nops
                    .clear();
            }
            5 => {
                assembler.exception_owner_provenance.guarded_loop_nops[0].target = label(4);
            }
            6 => {
                let source = assembler.exception_owner_provenance.guarded_loop_nops[0].clone();
                assembler
                    .exception_owner_provenance
                    .guarded_loop_nops
                    .push(source);
            }
            7 => {
                items_mut(&mut assembler).swap(1, 2);
            }
            8 => {
                items_mut(&mut assembler).push(record(NOP, 0, 9, 0));
            }
            9 => {
                assembler.borrow_block_aliases.insert(label(2));
            }
            10 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&label(2));
            }
            11 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .remove(&label(2));
            }
            12 => {
                assembler.preserved_block_boundaries.insert(label(2));
            }
            13 => {
                items_mut(&mut assembler)[6] = record(RAISE_VARARGS, 2, 4, 0);
            }
            14 => {
                items_mut(&mut assembler)[7] = Item::Label(label(1));
            }
            15 => {
                assembler.emission_stream = EmissionStreamId::next();
            }
            16 => {
                assembler.add_exception_region(label(0), label(1), label(2), 0, false);
            }
            17 => {
                let original = assembler.exception_owner_provenance.guarded_loop_nops[0].condition;
                items_mut(&mut assembler)[1] = Item::Instruction(original);
            }
            _ => unreachable!(),
        }
        let before = assembler
            .exception_owner_provenance
            .guarded_loop_nops
            .clone();
        assembler.seal_guarded_loop_borrow_route(original);
        assert_eq!(
            before, assembler.exception_owner_provenance.guarded_loop_nops,
            "mode {mode}"
        );
    }
}
fn fixture_with_argument(prepared: bool, constant: bool, remap: bool) -> OptimizationPipeline {
    fixture_with_borrow_route(prepared, constant, remap, BorrowRoutePolicy::Original)
}

#[derive(Clone, Copy)]
enum BorrowRoutePolicy {
    Original,
    Blocked,
}

fn fixture_with_borrow_route(
    prepared: bool,
    constant: bool,
    remap: bool,
    policy: BorrowRoutePolicy,
) -> OptimizationPipeline {
    let (mut assembler, emitted) = source_fixture();
    assembler.record_guarded_loop_borrow_nop(emitted);
    if matches!(policy, BorrowRoutePolicy::Blocked) {
        assembler.prevent_finalizer_borrow_jump_threading_target(emitted);
    }
    assert_eq!(
        assembler.exception_owner_provenance.guarded_loop_nops.len(),
        1
    );
    let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
    pop.fusion_policy.mark_converted_pop_block();
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    items_mut(&mut assembler).extend([
        Item::Label(label(3)),
        Item::Label(label(4)),
        Item::Instruction(pop),
        record(LOAD_GLOBAL, 1, 5, 2),
        record(
            if constant { LOAD_CONST } else { LOAD_FAST },
            if constant { 2 } else { 0 },
            5,
            3,
        ),
        record(CALL, 1, 5, 1),
        record(POP_TOP, 0, 5, 0),
    ]);
    assembler
        .exception_owner_provenance
        .source_boundaries
        .insert(label(3));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(label(3));
    assembler
        .exception_owner_provenance
        .borrow_only_block_boundaries
        .insert(label(3));
    assembler.instruction_count = assembler
        .linear_items()
        .iter()
        .filter(|item| matches!(item, Item::Instruction(_)))
        .count();
    assembler.record_guarded_loop_target_pop(6);
    assert!(
        assembler.exception_owner_provenance.guarded_loop_nops[0]
            .destination
            .is_some()
    );
    items_mut(&mut assembler).extend([record(LOAD_CONST, 0, 5, 1), record(RETURN_VALUE, 0, 5, 0)]);
    assembler.instruction_count += 2;
    if prepared {
        assembler.used_constant_indices(if constant { 3 } else { 1 });
        assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    }
    if remap {
        assembler.remap_constant_indices(&[Some(0), Some(2), Some(1)]);
    }
    assembler.into_optimization_pipeline()
}
fn donate(pipeline: &mut OptimizationPipeline, receipts: &mut GuardedLoopNops) {
    pipeline.remove_guarded_loop_test_jumps(false, receipts);
    assert!(receipts.origins[0].live && receipts.origins[0].deferred);
    receipts.observe_donation(pipeline, false);
    let reversed = pipeline.capture_reversed_store_exits();
    let threaded = pipeline.capture_threaded_normal_try_joins();
    pipeline
        .duplicate_exit_blocks_with_store_origins(
            &FxHashSet::default(),
            (&mut Vec::new(), &mut Vec::new(), &mut Vec::new()),
            &reversed,
            &threaded,
            &mut [],
            None,
        )
        .unwrap();
    receipts.observe_donation(pipeline, true);
    assert!(receipts.origins[0].live && receipts.origins[0].donated);
}
fn complete(pipeline: &mut OptimizationPipeline, receipts: &mut GuardedLoopNops) {
    donate(pipeline, receipts);
    pipeline.remove_guarded_loop_test_jumps(true, receipts);
    assert!(receipts.origins[0].live && receipts.origins[0].rewritten);
    receipts.revalidate(pipeline);
    assert!(receipts.origins[0].live);
}

#[test]
fn guarded_loop_borrow_route_keeps_the_real_location_donation() {
    for prepared in [false, true] {
        let mut pipeline =
            fixture_with_borrow_route(prepared, false, false, BorrowRoutePolicy::Blocked);
        let mut receipts = GuardedLoopNops::capture(&pipeline);
        assert_eq!(receipts.origins.len(), 1);
        complete(&mut pipeline, &mut receipts);
    }
}

#[test]
fn guarded_loop_nop_uses_real_writers_and_keeps_analysis_barrier() {
    for prepared in [false, true] {
        let mut pipeline = fixture(prepared);
        let mut receipts = GuardedLoopNops::capture(&pipeline);
        assert_eq!(receipts.origins.len(), 1);
        complete(&mut pipeline, &mut receipts);
        let mut items = pipeline.graph.into_items();
        let original = items.clone();
        receipts.mark_before_borrow(
            &mut items,
            &pipeline.borrow_block_aliases,
            &pipeline.borrow_finally_entries,
        );
        let changed = original
            .iter()
            .zip(&items)
            .filter(|(a, b)| a != b)
            .collect::<Vec<_>>();
        let [(Item::Instruction(before), Item::Instruction(after))] = changed.as_slice() else {
            panic!("{changed:?}")
        };
        let mut expected = *before;
        expected
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved);
        assert_eq!(*after, expected);
        assert!(after.jump_policy.borrow_empty_block());
        let once = items.clone();
        receipts.mark_before_borrow(
            &mut items,
            &pipeline.borrow_block_aliases,
            &pipeline.borrow_finally_entries,
        );
        assert_eq!(items, once);
        OptimizationPipeline::optimize_guarded_loop_test_borrows(
            &mut items,
            &FxHashSet::from_iter([label(2), label(3)]),
        );
        assert!(
            items.contains(&Item::Instruction(expected)),
            "marker must survive real borrow analysis"
        );
        assert_eq!(items.iter().filter(|item| matches!(item, Item::Instruction(record) if record.location == location(5) && record.opcode == LOAD_FAST)).count(), 1);
        OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
        assert_eq!(items.len() + 1, once.len());
        assert!(!items.contains(&Item::Instruction(expected)));
    }
}

fn edit_pipeline(pipeline: &mut OptimizationPipeline, edit: impl FnOnce(&mut Vec<Item>)) {
    let mut items = pipeline.graph.copy_items();
    edit(&mut items);
    pipeline.graph = BlockGraph::from_items(items, &pipeline.preserved_block_boundaries);
}

#[test]
fn guarded_loop_target_entry_keeps_its_independent_borrowing_path() {
    let mut pipeline = fixture(false);
    let mut receipts = GuardedLoopNops::capture(&pipeline);
    complete(&mut pipeline, &mut receipts);
    let mut items = pipeline.graph.into_items();
    receipts.mark_before_borrow(
        &mut items,
        &pipeline.borrow_block_aliases,
        &pipeline.borrow_finally_entries,
    );
    // A separate real jump reaches the target independently of the marked source.
    items.splice(
        1..1,
        [
            record(LOAD_FAST, 0, 2, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_TRUE,
                Operand::Forward(label(3)),
                location(2),
                Some(0),
            )),
        ],
    );
    OptimizationPipeline::optimize_guarded_loop_test_borrows(
        &mut items,
        &FxHashSet::from_iter([label(2), label(3)]),
    );
    assert_eq!(items.iter().filter(|item| matches!(item, Item::Instruction(record) if record.location == location(5) && record.opcode == LOAD_FAST_BORROW)).count(), 1);
    let length = items.len();
    OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
    assert_eq!(items.len(), length - 1);

    // Before line donation that extra entry blocks the original target donor, so
    // the source certificate must not invent a replacement location for it.
    let mut pipeline = fixture(false);
    edit_pipeline(&mut pipeline, |items| {
        items.splice(
            1..1,
            [
                record(LOAD_FAST, 0, 2, 1),
                Item::Instruction(Instruction::new(
                    POP_JUMP_IF_TRUE,
                    Operand::Forward(label(3)),
                    location(2),
                    Some(0),
                )),
            ],
        );
    });
    let mut receipts = GuardedLoopNops::capture(&pipeline);
    assert_eq!(receipts.origins.len(), 1);
    pipeline.remove_guarded_loop_test_jumps(false, &mut receipts);
    receipts.observe_donation(&pipeline, false);
    let reversed = pipeline.capture_reversed_store_exits();
    let threaded = pipeline.capture_threaded_normal_try_joins();
    pipeline
        .duplicate_exit_blocks_with_store_origins(
            &FxHashSet::default(),
            (&mut Vec::new(), &mut Vec::new(), &mut Vec::new()),
            &reversed,
            &threaded,
            &mut [],
            None,
        )
        .unwrap();
    receipts.observe_donation(&pipeline, true);
    assert!(!receipts.origins[0].live);
}

#[test]
fn guarded_loop_target_census_rejects_moved_pop_and_identical_replacement() {
    for donated in [false, true] {
        let mut pipeline = fixture(false);
        let mut receipts = GuardedLoopNops::capture(&pipeline);
        if donated {
            complete(&mut pipeline, &mut receipts);
        }
        let pop = receipts.origins[0]
            .source
            .destination
            .as_ref()
            .unwrap()
            .prefix[0];
        edit_pipeline(&mut pipeline, |items| {
            // The old head still holds an identical replacement; the original survives
            // under a foreign head whose following records do not duplicate the array.
            items.extend([
                Item::Label(label(5)),
                pop,
                record(LOAD_CONST, 0, 7, 1),
                record(RETURN_VALUE, 0, 7, 0),
            ]);
        });
        if donated {
            receipts.revalidate(&pipeline);
            assert!(!receipts.origins[0].live);
        } else {
            assert!(GuardedLoopNops::capture(&pipeline).origins.is_empty());
            pipeline.remove_guarded_loop_test_jumps(false, &mut receipts);
            receipts.observe_donation(&pipeline, false);
            assert!(!receipts.origins[0].live);
        }
    }
}

#[test]
fn guarded_loop_bound_heads_records_and_required_borrow_bit_are_exact() {
    for mutation in 0..14 {
        let mut pipeline = fixture(false);
        match mutation {
            0 => {
                pipeline
                    .exception_owner_provenance
                    .borrow_only_block_boundaries
                    .remove(&label(3));
            }
            1 => {
                pipeline
                    .exception_owner_provenance
                    .borrow_only_block_boundaries
                    .insert(label(2));
            }
            2 => {
                pipeline
                    .exception_owner_provenance
                    .borrow_only_block_boundaries
                    .insert(label(4));
            }
            3 => {
                pipeline.borrow_block_aliases.insert(label(3));
            }
            4 => {
                pipeline.original_nop_targets.insert(label(3));
            }
            5 => {
                pipeline
                    .exception_owner_provenance
                    .setup_nop_boundaries
                    .insert(label(3));
            }
            6 => {
                let original = pipeline.exception_owner_provenance.guarded_loop_nops[0].clone();
                pipeline
                    .exception_owner_provenance
                    .guarded_loop_nops
                    .push(original);
            }
            8 => {
                pipeline.exception_owner_provenance.guarded_loop_nops[0]
                    .destination
                    .as_mut()
                    .unwrap()
                    .head
                    .push(Item::Label(label(3)));
            }
            _ => edit_pipeline(&mut pipeline, |items| {
                let at = items
                    .iter()
                    .position(|item| *item == Item::Label(label(3)))
                    .unwrap();
                match mutation {
                    7 => {
                        items.insert(at, Item::Label(label(5)));
                    }
                    9 => {
                        items.insert(at + 2, Item::Label(label(5)));
                    }
                    10 => {
                        items.remove(at);
                    }
                    11..=13 => {
                        let Item::Instruction(pop) = &mut items[at + 2] else {
                            unreachable!()
                        };
                        match mutation {
                            11 => pop.set_exception_policy(ExceptionPolicy::AlwaysAbsent),
                            12 => pop.local_policy.add_hidden_exception_owner_slots(1),
                            13 => pop.no_location_policy.preserve_location(),
                            _ => unreachable!(),
                        }
                    }
                    _ => unreachable!(),
                }
            }),
        }
        assert!(
            GuardedLoopNops::capture(&pipeline).origins.is_empty(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn guarded_loop_revalidates_restored_aliases_and_refuses_prepared_recapture() {
    for alias in [false, true] {
        let mut pipeline = fixture(true);
        let mut receipts = GuardedLoopNops::capture(&pipeline);
        complete(&mut pipeline, &mut receipts);
        if alias {
            pipeline.borrow_block_aliases.insert(label(3));
        } else {
            pipeline.borrow_finally_entries.insert(label(3), None);
        }
        let mut items = pipeline.graph.into_items();
        let original = items.clone();
        receipts.mark_before_borrow(
            &mut items,
            &pipeline.borrow_block_aliases,
            &pipeline.borrow_finally_entries,
        );
        assert_eq!(items, original);
    }
    let (mut assembler, emitted) = source_fixture();
    let graph = BlockGraph::from_items(assembler.linear_items().to_vec(), &FxHashSet::default());
    let reachable = vec![true; graph.len()];
    assembler.code = UnoptimizedCode::Prepared { graph, reachable };
    assembler.record_guarded_loop_borrow_nop(emitted);
    assembler.record_guarded_loop_target_pop(6);
    assert!(
        assembler
            .exception_owner_provenance
            .guarded_loop_nops
            .is_empty()
    );
}

#[test]
fn guarded_loop_return_fence_excludes_the_following_cold_array() {
    let mut pipeline = fixture(false);
    let mut receipts = GuardedLoopNops::capture(&pipeline);
    complete(&mut pipeline, &mut receipts);
    edit_pipeline(&mut pipeline, |items| {
        items.extend([
            Item::Label(label(5)),
            record(LOAD_CONST, 0, 7, 1),
            record(RETURN_VALUE, 0, 7, 0),
        ]);
    });
    receipts.revalidate(&pipeline);
    assert!(receipts.origins[0].live);
    edit_pipeline(&mut pipeline, |items| {
        let next = items
            .iter_mut()
            .find(|item| **item == Item::Label(label(5)))
            .unwrap();
        *next = Item::Label(label(6));
    });
    receipts.revalidate(&pipeline);
    assert!(
        receipts.origins[0].live,
        "the next disjoint array is beyond RETURN_VALUE"
    );
    edit_pipeline(&mut pipeline, |items| {
        let next = items
            .iter()
            .position(|item| *item == Item::Label(label(6)))
            .unwrap();
        items.insert(next, record(NOP, 0, 7, 0));
    });
    receipts.revalidate(&pipeline);
    assert!(
        !receipts.origins[0].live,
        "an extra record inside the original array is not a fence"
    );
}

#[test]
fn guarded_loop_constants_follow_actual_lowering_and_pool_remaps() {
    use crate::CompileError;
    use crate::code_object::{Constant, NonReflexiveRemap};
    use crate::target::opcodes::LOAD_COMMON_CONSTANT;
    struct Constants(Vec<Constant>);
    impl crate::assembler::ConstantTable for Constants {
        fn get(&self, index: u32) -> &Constant {
            &self.0[index as usize]
        }
        fn len(&self) -> usize {
            self.0.len()
        }
        fn intern(&mut self, _: Constant) -> Result<u32, CompileError> {
            unreachable!()
        }
        fn fold_sequence(&mut self, _: Vec<Constant>, _: bool) -> Result<u32, CompileError> {
            unreachable!()
        }
        fn fold_binary_sequence(
            &mut self,
            _: Constant,
            _: u32,
            _: Constant,
        ) -> Result<Option<u32>, CompileError> {
            unreachable!()
        }
        fn reallocate_non_reflexive_constant(
            &mut self,
            _: u32,
            _: &mut NonReflexiveRemap,
        ) -> Result<u32, CompileError> {
            unreachable!()
        }
        fn retain_used(&mut self, used: &[bool], _: &[u32], _: &[bool]) -> Vec<Option<u32>> {
            self.retain_used_in_order(used)
        }
        fn retain_used_in_order(&mut self, used: &[bool]) -> Vec<Option<u32>> {
            let mut retained = Vec::new();
            let mut mapping = Vec::new();
            for (index, value) in self.0.iter().enumerate() {
                if index == 0 || used[index] {
                    mapping.push(Some(u32::try_from(retained.len()).unwrap()));
                    retained.push(value.clone());
                } else {
                    mapping.push(None);
                }
            }
            self.0 = retained;
            mapping
        }
    }
    for prepared in [false, true] {
        for remap in [false, true] {
            for stale in [false, true] {
                let mut pipeline = fixture_with_argument(prepared, true, remap);
                pipeline.python_version = crate::PythonVersion::Py315;
                let mut receipts = GuardedLoopNops::capture(&pipeline);
                assert_eq!(
                    receipts.origins.len(),
                    1,
                    "prepared={prepared} remap={remap} stale={stale}"
                );
                donate(&mut pipeline, &mut receipts);
                if stale {
                    let target = receipts.origins[0].source.destination.as_mut().unwrap();
                    let Item::Instruction(argument) = &mut target.prefix[2] else {
                        unreachable!()
                    };
                    argument.set_operand(Operand::Value(99));
                }
                let mut values = vec![Constant::None, Constant::Bool(true), Constant::SignedInt(9)];
                if remap {
                    values.swap(1, 2);
                }
                let mut constants = Constants(values);
                pipeline.lower_guarded_loop_test_constants(&mut constants, &mut receipts);
                assert_eq!(receipts.origins[0].live, !stale);
                if stale {
                    continue;
                }
                let target = &receipts.origins[0]
                    .source
                    .destination
                    .as_ref()
                    .unwrap()
                    .prefix;
                assert!(
                    matches!(target[2], Item::Instruction(record) if record.opcode == LOAD_CONST && record.operand() == Operand::Value(1))
                );
                assert!(
                    matches!(target[5], Item::Instruction(record) if record.opcode == LOAD_COMMON_CONSTANT && record.operand() == Operand::Value(7))
                );
                assert_eq!(constants.0.len(), 2);
                pipeline.remove_guarded_loop_test_jumps(true, &mut receipts);
                receipts.revalidate(&pipeline);
                assert!(receipts.origins[0].live);
            }
        }
    }
}

#[test]
fn guarded_loop_source_rejects_wrong_stream_records_aliases_and_entries() {
    for mutation in 0..15 {
        let (mut assembler, emitted) = source_fixture();
        match mutation {
            0 => assembler.emission_stream = EmissionStreamId::next(),
            1 => {
                items_mut(&mut assembler).push(Item::Label(label(5)));
            }
            2 => {
                items_mut(&mut assembler).insert(0, Item::Label(label(2)));
            }
            3 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .clear();
            }
            4 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .clear();
            }
            5 => {
                assembler.borrow_block_aliases.insert(label(2));
            }
            6 => {
                assembler.original_nop_targets.insert(label(2));
            }
            7 => {
                assembler.preserved_block_boundaries.insert(label(2));
            }
            _ => {
                let index = match mutation {
                    8..=10 => 8,
                    11..=12 => 2,
                    _ => 6,
                };
                let Item::Instruction(value) = &mut items_mut(&mut assembler)[index] else {
                    unreachable!()
                };
                match mutation {
                    8 | 11 | 13 => value.local_policy.add_hidden_exception_owner_slots(1),
                    9 | 12 | 14 => value.set_exception_policy(ExceptionPolicy::NonePresent),
                    10 => value.add_pre_fold_extra_instructions(1),
                    _ => unreachable!(),
                }
            }
        }
        assembler.record_guarded_loop_borrow_nop(emitted);
        assert!(
            assembler
                .exception_owner_provenance
                .guarded_loop_nops
                .is_empty(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn empty_guarded_loop_receipt_is_inert() {
    let mut receipts = GuardedLoopNops::default();
    let mut items = vec![Item::Instruction(Instruction::new(
        NOP,
        Operand::Value(0),
        SourceLocation::NONE,
        Some(0),
    ))];
    let original = items.clone();
    receipts.mark_before_borrow(&mut items, &FxHashSet::default(), &FxHashMap::default());
    assert_eq!(items, original);
}
