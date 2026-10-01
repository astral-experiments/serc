use super::*;
use crate::target::opcodes::{LOAD_FAST_BORROW, RETURN_VALUE};

fn l(value: u32) -> Label {
    Label::from_u32(value)
}

fn loc(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 4, 12)
}

fn value(opcode: crate::target::Opcode, argument: u32, line: i32, depth: u32) -> Item {
    Item::Instruction(Instruction::value_with_depth(
        opcode,
        argument,
        loc(line),
        depth,
    ))
}

fn append(assembler: &mut Assembler, items: impl IntoIterator<Item = Item>) {
    for item in items {
        assembler.push_linear_item(item);
    }
}

fn fixture(collapse: bool) -> Assembler {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let exit = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(l(1)),
        SourceLocation::NONE,
        Some(1),
    );
    append(
        &mut assembler,
        [
            Item::Label(l(0)),
            value(LOAD_CONST, 0, 2, 1),
            Item::Instruction(exit),
            Item::Label(l(1)),
        ],
    );
    for argument in [1, 3, 5] {
        append(
            &mut assembler,
            [
                value(LOAD_GLOBAL, argument, 3, 3),
                value(CALL, 0, 3, 2),
                value(POP_TOP, 0, 3, 1),
            ],
        );
    }
    let returned = assembler.linear_items().len();
    append(&mut assembler, [value(LOAD_FAST, 1, 4, 2)]);
    assembler
        .exception_owner_provenance
        .original_if_expression_joins
        .push((l(1), exit));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(l(1));
    let mut source = assembler
        .capture_retained_setup_truth_start(returned, 1, loc(4))
        .unwrap();
    source.bind_unwind(l(2), (l(3), l(4)), l(5));
    append(
        &mut assembler,
        [
            Item::Label(l(2)),
            Item::Label(l(3)),
            value(SWAP, 2, 4, 2),
            value(POP_TOP, 0, 4, 1),
            Item::Label(l(4)),
            Item::Label(l(5)),
        ],
    );
    let pop = assembler.linear_items().len();
    let mut removed = Instruction::value_with_depth(NOP, 0, loc(4), 1);
    removed.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    removed.fusion_policy.mark_converted_pop_block();
    let mut setup = Instruction::value_with_depth(NOP, 0, loc(6), 1);
    setup.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    let mut marker = Instruction::value_with_depth(NOT_TAKEN, 0, loc(7), 1);
    marker.set_exception_policy(ExceptionPolicy::NoneDynamicCondition);
    append(
        &mut assembler,
        [
            Item::Instruction(removed),
            Item::Label(l(6)),
            Item::Instruction(setup),
            Item::Label(l(7)),
            Item::Label(l(8)),
            value(LOAD_FAST, 0, 7, 2),
            value(TO_BOOL, 0, 7, 2),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(l(9)),
                loc(7),
                Some(1),
            )),
            Item::Instruction(marker),
            value(LOAD_CONST, 0, 8, 2),
            value(RETURN_VALUE, 0, 8, 0),
        ],
    );
    if collapse {
        append(
            &mut assembler,
            [
                Item::Label(l(12)),
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(l(9)),
                    loc(9),
                    Some(1),
                )),
            ],
        );
        assembler
            .exception_owner_provenance
            .normal_try_exits
            .push((l(12), l(9)));
    }
    append(
        &mut assembler,
        [
            Item::Label(l(9)),
            value(LOAD_CONST, 0, 10, 2),
            value(RETURN_VALUE, 0, 10, 0),
            Item::Label(l(10)),
            value(LOAD_CONST, 0, 11, 2),
            value(RETURN_VALUE, 0, 11, 0),
        ],
    );
    assembler
        .exception_owner_provenance
        .setup_nop_ranges
        .push((l(6), l(7)));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(l(9));
    assembler
        .exception_owner_provenance
        .setups
        .push((l(8), Some(loc(6))));
    assembler.exception_regions.push(ExceptionRegion {
        start: l(8),
        end: l(9),
        target: l(10),
        depth: 1,
        preserve_lasti: false,
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(l(8), l(9)),
    });
    assembler.next_label = 100;
    assembler.record_retained_setup_truth(source, pop, 0, loc(7), loc(6), loc(4));
    assert_eq!(
        assembler
            .exception_owner_provenance
            .retained_setup_truths
            .len(),
        1
    );
    assembler
}

fn pipeline(prepared: bool, collapse: bool) -> OptimizationPipeline {
    let mut assembler = fixture(collapse);
    if prepared {
        let _ = assembler.used_constant_indices(1);
    }
    let pipeline = assembler.into_optimization_pipeline();
    assert_eq!(
        pipeline
            .exception_owner_provenance
            .retained_setup_truths
            .len(),
        1
    );
    pipeline
}

fn marker_head(pipeline: &mut OptimizationPipeline) {
    let marker = *pipeline.exception_owner_provenance.retained_setup_truths[0]
        .current
        .last()
        .unwrap();
    let block = pipeline
        .graph
        .iter()
        .find(|(_, block)| block.items().first() == Some(&marker))
        .unwrap()
        .0;
    for origin in &mut pipeline.exception_owner_provenance.retained_setup_truths {
        origin.prepend_marker_head(&pipeline.graph, block, l(100));
        assert_eq!(origin.marker_head, Some(l(100)));
    }
    pipeline.graph.insert_label(block, 0, l(100));
}

fn find_position(graph: &BlockGraph, item: Item) -> ItemPosition {
    graph
        .iter()
        .find_map(|(block, data)| {
            data.items()
                .iter()
                .position(|candidate| *candidate == item)
                .map(|offset| ItemPosition::new(block, offset))
        })
        .unwrap()
}

fn remove_pop(pipeline: &mut OptimizationPipeline) {
    let origin = &pipeline.exception_owner_provenance.retained_setup_truths[0];
    let pop = find_position(&pipeline.graph, origin.current[origin.pop]);
    let setup = find_position(&pipeline.graph, origin.current[origin.pop + 2]);
    pipeline.transport_retained_setup_truth_pop(&[pop], &[(setup, 1)]);
    assert_eq!(
        pipeline
            .exception_owner_provenance
            .retained_setup_truths
            .len(),
        1
    );
    pipeline
        .graph
        .instruction_mut(setup)
        .unwrap()
        .local_policy
        .add_hidden_exception_owner_slots(1);
    pipeline.graph.remove_item(pop);
    assert!(
        pipeline.exception_owner_provenance.retained_setup_truths[0]
            .records(&pipeline.graph.copy_items())
    );
}

#[test]
fn retained_setup_capture_and_seal_bind_every_record_and_alias() {
    let count = fixture(false).linear_items().len();
    for index in 0..count {
        let mut assembler = fixture(false);
        let selected = &assembler.exception_owner_provenance.retained_setup_truths[0];
        let relevant = index < selected.source.start + selected.original.len()
            || assembler.linear_items()[index] == Item::Label(l(9));
        if !relevant {
            continue;
        }
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            unreachable!()
        };
        match &mut items[index] {
            Item::Instruction(value) => value.local_policy.add_hidden_exception_owner_slots(1),
            Item::Label(label) => *label = l(99),
        }
        assembler.seal_retained_setup_truths();
        assert!(
            assembler
                .exception_owner_provenance
                .retained_setup_truths
                .is_empty(),
            "index {index}"
        );
    }
    for mutation in 0..7 {
        let mut assembler = fixture(false);
        match mutation {
            0 => {}
            1 => {
                assembler.exception_owner_provenance.retained_setup_truths[0]
                    .source
                    .start += 1;
            }
            2 => {
                assembler.exception_regions[0].depth += 1;
            }
            3 => {
                assembler.exception_regions[0].preserve_lasti = true;
            }
            4 => {
                assembler.exception_owner_provenance.retained_setup_truths[0]
                    .source
                    .stream =
                    Assembler::with_python_version(crate::PythonVersion::Py314).emission_stream;
            }
            5 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!()
                };
                items.insert(0, Item::Label(l(99)));
            }
            6 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!()
                };
                let at = items
                    .iter()
                    .position(|item| *item == Item::Label(l(9)))
                    .unwrap();
                items.insert(at, Item::Label(l(99)));
            }
            _ => unreachable!(),
        }
        assembler.seal_retained_setup_truths();
        assert_eq!(
            assembler
                .exception_owner_provenance
                .retained_setup_truths
                .len(),
            usize::from(mutation == 0),
            "mutation {mutation}"
        );
    }
}

#[test]
fn retained_setup_marker_head_requires_exact_fresh_writer_position() {
    for prepared in [false, true] {
        for mutation in 0..7 {
            let mut pipeline = pipeline(prepared, false);
            let origin = &mut pipeline.exception_owner_provenance.retained_setup_truths[0];
            let marker = *origin.current.last().unwrap();
            let marker_block = pipeline
                .graph
                .iter()
                .find(|(_, block)| block.items().first() == Some(&marker))
                .unwrap()
                .0;
            let mut selected = marker_block;
            let mut fresh = l(100);
            match mutation {
                0 => {}
                1 => selected = pipeline.graph.label_block(l(1)).unwrap(),
                2 => fresh = l(1),
                3 => pipeline.graph.insert_label(marker_block, 0, l(101)),
                4 => {
                    pipeline
                        .graph
                        .instruction_mut(ItemPosition::new(marker_block, 0))
                        .unwrap()
                        .local_policy
                        .add_hidden_exception_owner_slots(1);
                }
                5 => {
                    origin.current_exit.set_operand(Operand::Forward(l(9)));
                }
                6 => {
                    origin.current.insert(1, value(NOP, 0, 1, 1));
                }
                _ => unreachable!(),
            }
            origin.prepend_marker_head(&pipeline.graph, selected, fresh);
            assert_eq!(
                origin.marker_head,
                (mutation == 0).then_some(l(100)),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn retained_setup_location_transport_binds_index_and_full_writer_records() {
    for prepared in [false, true] {
        let mut positive = pipeline(prepared, false);
        let original = positive.exception_owner_provenance.retained_setup_truths[0]
            .source
            .exit;
        positive.propagate_locations_within_blocks();
        let origin = &positive.exception_owner_provenance.retained_setup_truths[0];
        assert_eq!(origin.source.exit, original);
        assert_eq!(origin.current_exit.location, loc(2));
        assert!(origin.records(&positive.graph.copy_items()));
        for mutation in 0..7 {
            let mut pipeline = pipeline(prepared, false);
            let mut captured = pipeline.before_retained_setup_truth_locations();
            let position = captured[0].1;
            let before = pipeline.exception_owner_provenance.retained_setup_truths[0].current_exit;
            let mut after = before;
            after.location = loc(2);
            *pipeline.graph.instruction_mut(position).unwrap() = after;
            let mut changes = vec![(position, before, after)];
            match mutation {
                0 => {}
                1 => captured.clear(),
                2 => captured.push(captured[0]),
                3 => changes.push(changes[0]),
                4 => {
                    changes[0].1.location = loc(3);
                }
                5 => {
                    changes[0]
                        .2
                        .local_policy
                        .add_hidden_exception_owner_slots(1);
                }
                6 => {
                    pipeline
                        .graph
                        .instruction_mut(position)
                        .unwrap()
                        .set_operand(Operand::Forward(l(9)));
                }
                _ => unreachable!(),
            }
            pipeline.after_retained_setup_truth_locations(&captured, &changes);
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .retained_setup_truths
                    .len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn retained_setup_pop_transport_requires_one_exact_indexed_donation() {
    for prepared in [false, true] {
        for mutation in 0..7 {
            let mut pipeline = pipeline(prepared, false);
            marker_head(&mut pipeline);
            let origin = &pipeline.exception_owner_provenance.retained_setup_truths[0];
            let pop = find_position(&pipeline.graph, origin.current[origin.pop]);
            let setup = find_position(&pipeline.graph, origin.current[origin.pop + 2]);
            let guard = find_position(&pipeline.graph, origin.current[origin.current.len() - 5]);
            let original = origin.original.clone();
            let mut removed = vec![pop];
            let mut donations = vec![(setup, 1)];
            match mutation {
                0 => {}
                1 => donations.clear(),
                2 => donations[0].1 = 2,
                3 => donations[0].0 = guard,
                4 => donations.push(donations[0]),
                5 => removed.push(guard),
                6 => {
                    pipeline
                        .graph
                        .instruction_mut(pop)
                        .unwrap()
                        .set_exception_policy(ExceptionPolicy::default());
                }
                _ => unreachable!(),
            }
            pipeline.transport_retained_setup_truth_pop(&removed, &donations);
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .retained_setup_truths
                    .len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
            if mutation == 0 {
                assert_eq!(
                    pipeline.exception_owner_provenance.retained_setup_truths[0].original,
                    original
                );
                pipeline
                    .graph
                    .instruction_mut(setup)
                    .unwrap()
                    .local_policy
                    .add_hidden_exception_owner_slots(1);
                pipeline.graph.remove_item(pop);
                assert!(
                    pipeline.exception_owner_provenance.retained_setup_truths[0]
                        .records(&pipeline.graph.copy_items())
                );
            }
        }
    }
}

#[test]
fn retained_setup_borrow_transport_changes_only_the_same_guard_load() {
    for prepared in [false, true] {
        for mutation in 0..8 {
            let mut pipeline = pipeline(prepared, false);
            marker_head(&mut pipeline);
            pipeline.propagate_locations_within_blocks();
            remove_pop(&mut pipeline);
            let origin = &pipeline.exception_owner_provenance.retained_setup_truths[0];
            assert!(origin.records(&pipeline.graph.copy_items()));
            for role in origin.roles.as_ref().unwrap() {
                assert_eq!(
                    *role,
                    LabelRoles::capture(&pipeline.exception_owner_provenance, role.label)
                );
            }
            assert!(origin.roles_match(
                &pipeline.exception_owner_provenance,
                &pipeline.exception_regions
            ));
            assert!(same_regions(
                origin.regions.as_ref().unwrap(),
                &pipeline.exception_regions
            ));
            let live = pipeline.graph.reachable_blocks(&pipeline.exception_regions);
            for label in origin
                .current
                .iter()
                .chain(&origin.current_predecessor)
                .chain(&origin.target_head)
                .filter_map(label)
            {
                let block = pipeline.graph.label_block(label).unwrap();
                assert_eq!(
                    live.get(block.index()),
                    Some(&true),
                    "label {label:?}, block {block:?}"
                );
            }
            pipeline.validate_retained_setup_truth_graph();
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .retained_setup_truths
                    .len(),
                1
            );
            let mut origins =
                std::mem::take(&mut pipeline.exception_owner_provenance.retained_setup_truths);
            let mut items = pipeline.graph.copy_items();
            let mut selected = RetainedSetupTruth::before_borrow(
                &mut origins,
                &items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
            );
            assert_eq!(selected.len(), 1);
            let at = selected[0].1;
            let mut borrowed = selected[0].2;
            borrowed.opcode = LOAD_FAST_BORROW;
            items[at] = Item::Instruction(borrowed);
            match mutation {
                0 => {}
                1 => {
                    items[at] = Item::Instruction(selected[0].2);
                }
                2 => {
                    selected[0].1 += 1;
                }
                3 => {
                    borrowed.set_operand(Operand::Value(99));
                    items[at] = Item::Instruction(borrowed);
                }
                4 => {
                    borrowed.local_policy.add_hidden_exception_owner_slots(1);
                    items[at] = Item::Instruction(borrowed);
                }
                5 => {
                    items[at + 1] = value(NOP, 0, 7, 2);
                }
                6 => {
                    pipeline.exception_regions[0].depth += 1;
                }
                7 => {
                    pipeline
                        .exception_owner_provenance
                        .borrow_only_block_boundaries
                        .insert(l(8));
                }
                _ => unreachable!(),
            }
            RetainedSetupTruth::after_borrow(
                &mut origins,
                &items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &selected,
            );
            assert_eq!(
                origins.len(),
                usize::from(mutation <= 1),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn retained_setup_cold_transport_requires_complete_indexed_region_groups() {
    for prepared in [false, true] {
        for mutation in 0..12 {
            let mut pipeline = pipeline(prepared, false);
            marker_head(&mut pipeline);
            let mut origins =
                std::mem::take(&mut pipeline.exception_owner_provenance.retained_setup_truths);
            let mut receipt = RetainedSetupColdRewrite::capture(
                &pipeline.graph,
                &pipeline.exception_regions,
                &mut origins,
            )
            .unwrap();
            receipt.output(0);
            receipt.group(0, 1);
            for region in &mut pipeline.exception_regions {
                region.start = pipeline.graph[pipeline.graph.label_block(region.start).unwrap()]
                    .first_label()
                    .unwrap();
                region.end = pipeline.graph[pipeline.graph.label_block(region.end).unwrap()]
                    .first_label()
                    .unwrap();
            }
            match mutation {
                0 => {}
                1 => receipt.output(0),
                2 => receipt.group(0, 0),
                3 => {
                    pipeline.exception_regions[0].depth += 1;
                }
                4 => {
                    pipeline.exception_regions[0].preserve_lasti = true;
                }
                5 => {
                    pipeline.exception_regions[0].is_scope_fallback = true;
                }
                6 => {
                    pipeline.exception_regions[0].priority =
                        ExceptionRegionPriority::Resolved { start: 0, end: 1 };
                }
                7 => {
                    pipeline.exception_regions[0].target = l(9);
                }
                8 => {
                    pipeline
                        .exception_regions
                        .push(pipeline.exception_regions[0]);
                }
                9 => {
                    let block = pipeline.graph.label_block(l(9)).unwrap();
                    pipeline.graph.insert_label(block, 0, l(99));
                }
                10 => {
                    pipeline
                        .graph
                        .push_block(vec![Item::Label(l(99)), value(RETURN_VALUE, 0, 20, 0)]);
                }
                11 => {
                    let block = pipeline.graph.label_block(l(8)).unwrap();
                    pipeline
                        .graph
                        .instructions_mut(block)
                        .next()
                        .unwrap()
                        .set_operand(Operand::Value(99));
                }
                _ => unreachable!(),
            }
            receipt.finish(
                &pipeline.graph,
                &pipeline.exception_regions,
                None,
                &mut origins,
            );
            assert_eq!(
                origins.len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn retained_setup_target_collapse_requires_the_actual_unique_deleted_jump() {
    for prepared in [false, true] {
        for mutation in 0..11 {
            let mut pipeline = pipeline(prepared, true);
            let mut origins =
                std::mem::take(&mut pipeline.exception_owner_provenance.retained_setup_truths);
            let original = origins[0].target_collapse.as_ref().unwrap().original;
            let position = find_position(&pipeline.graph, original[1]);
            let removed = FxHashSet::from_iter([position]);
            let mut receipts = RetainedSetupCollapse::capture(
                &pipeline.graph,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &mut origins,
                &removed,
            );
            assert_eq!(receipts.len(), 1);
            pipeline.graph.remove_item(position);
            match mutation {
                0 => {}
                1 => receipts.clear(),
                2 => {
                    pipeline.graph.insert_item(position, original[1]);
                }
                3 => {
                    pipeline
                        .graph
                        .push_block(vec![Item::Label(l(98)), original[1]]);
                }
                4 => {
                    let block = pipeline.graph.label_block(l(9)).unwrap();
                    pipeline.graph.insert_label(block, 0, l(99));
                }
                5 => {
                    pipeline.exception_regions[0].target = l(12);
                }
                6 => {
                    pipeline
                        .exception_owner_provenance
                        .borrow_only_block_boundaries
                        .insert(l(12));
                }
                7 => {
                    pipeline.exception_regions[0].depth += 1;
                }
                8 => {
                    let block = pipeline.graph.label_block(l(8)).unwrap();
                    pipeline
                        .graph
                        .instructions_mut(block)
                        .next_back()
                        .unwrap()
                        .set_operand(Operand::Forward(l(12)));
                }
                9 => {
                    pipeline.graph.push_block(vec![
                        Item::Label(l(98)),
                        Item::Instruction(Instruction::new(
                            JUMP_FORWARD,
                            Operand::Forward(l(9)),
                            loc(21),
                            Some(1),
                        )),
                    ]);
                }
                10 => {
                    pipeline
                        .graph
                        .remove_item(ItemPosition::new(position.block(), 0));
                }
                _ => unreachable!(),
            }
            RetainedSetupCollapse::finish(
                &receipts,
                &pipeline.graph,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &mut origins,
            );
            assert_eq!(
                origins.len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
            if mutation == 0 {
                assert_eq!(
                    origins[0].target_head,
                    [Item::Label(l(12)), Item::Label(l(9))]
                );
                assert_eq!(
                    origins[0].target_collapse.as_ref().unwrap().original,
                    original
                );
                RetainedSetupCollapse::finish(
                    &receipts,
                    &pipeline.graph,
                    &pipeline.exception_regions,
                    &pipeline.exception_owner_provenance,
                    &mut origins,
                );
                assert!(origins.is_empty());
            }
        }
    }
}

#[test]
fn retained_setup_constants_and_prepared_lifecycle_preserve_exact_identity() {
    for prepared in [false, true] {
        let mut assembler = fixture(false);
        if prepared {
            let _ = assembler.used_constant_indices(1);
        }
        assembler.remap_constant_indices(&[Some(4)]);
        let pipeline = assembler.into_optimization_pipeline();
        let origin = &pipeline.exception_owner_provenance.retained_setup_truths[0];
        assert!(origin.records(&pipeline.graph.copy_items()));
        assert!(
            origin
                .current_predecessor
                .iter()
                .filter_map(instruction)
                .filter(|value| value.opcode == LOAD_CONST)
                .all(|value| value.operand() == Operand::Value(4))
        );
    }
    let mut assembler = fixture(false);
    let start = assembler.exception_owner_provenance.retained_setup_truths[0]
        .source
        .start;
    let returned = start
        + assembler.exception_owner_provenance.retained_setup_truths[0]
            .source
            .prefix
            .len()
        - 1;
    let stale = assembler
        .capture_retained_setup_truth_start(returned, 1, loc(4))
        .unwrap();
    let _ = assembler.used_constant_indices(1);
    assert!(
        assembler
            .capture_retained_setup_truth_start(returned, 1, loc(4))
            .is_none()
    );
    let count = assembler
        .exception_owner_provenance
        .retained_setup_truths
        .len();
    assembler.record_retained_setup_truth(stale, returned + 7, 0, loc(7), loc(6), loc(4));
    assert_eq!(
        assembler
            .exception_owner_provenance
            .retained_setup_truths
            .len(),
        count
    );
    let mut origin = fixture(false)
        .exception_owner_provenance
        .retained_setup_truths
        .pop()
        .unwrap();
    assert!(!origin.remap_constants(&[None], None));
}

#[test]
fn retained_setup_final_consumer_requires_reachability_and_full_late_roles() {
    for prepared in [false, true] {
        for mutation in 0..8 {
            let mut pipeline = pipeline(prepared, false);
            marker_head(&mut pipeline);
            pipeline.propagate_locations_within_blocks();
            remove_pop(&mut pipeline);
            if mutation == 1 {
                pipeline.graph.insert_block(
                    0,
                    vec![
                        Item::Label(l(99)),
                        Item::Instruction(Instruction::new(
                            JUMP_FORWARD,
                            Operand::Forward(l(9)),
                            loc(1),
                            Some(1),
                        )),
                    ],
                );
            }
            pipeline.validate_retained_setup_truth_graph();
            if mutation == 1 {
                assert!(
                    pipeline
                        .exception_owner_provenance
                        .retained_setup_truths
                        .is_empty()
                );
                continue;
            }
            let mut origins =
                std::mem::take(&mut pipeline.exception_owner_provenance.retained_setup_truths);
            let items = pipeline.graph.copy_items();
            let selected = RetainedSetupTruth::before_borrow(
                &mut origins,
                &items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
            );
            RetainedSetupTruth::after_borrow(
                &mut origins,
                &items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &selected,
            );
            assert_eq!(origins.len(), 1);
            pipeline.exception_owner_provenance.retained_setup_truths = origins;
            let mut layout = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_regions: pipeline.exception_regions,
                exception_owner_provenance: pipeline.exception_owner_provenance,
                preserved_block_boundaries: FxHashSet::default(),
                removed_max_depth: None,
                next_label: pipeline.next_label,
                has_next_location: false,
            };
            assert_eq!(layout.retained_setup_truth_null_markers().len(), 1);
            match mutation {
                0 => {}
                2 => {
                    layout.exception_owner_provenance.setups[0].1 = None;
                }
                3 => {
                    layout
                        .exception_owner_provenance
                        .return_region_reentries
                        .insert(l(8));
                }
                4 => {
                    layout.exception_regions[0].target = l(1);
                }
                5 => {
                    let at = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(l(9)))
                        .unwrap();
                    layout.items.insert(at, Item::Label(l(99)));
                }
                6 => {
                    let origin = &layout.exception_owner_provenance.retained_setup_truths[0];
                    layout.items.push(*origin.current.last().unwrap());
                }
                7 => {
                    layout.exception_owner_provenance.retained_setup_truths[0].borrow_validated =
                        false;
                }
                _ => unreachable!(),
            }
            let selected = layout.retained_setup_truth_null_markers();
            assert_eq!(
                selected.len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
            assert_eq!(selected, layout.retained_setup_truth_null_markers());
        }
    }
}

#[test]
fn retained_setup_real_cold_and_nop_writers_preserve_the_original_source() {
    for prepared in [false, true] {
        let mut pipeline = pipeline(prepared, false);
        marker_head(&mut pipeline);
        pipeline.propagate_locations_within_blocks();
        let original = pipeline.exception_owner_provenance.retained_setup_truths[0]
            .original
            .clone();
        pipeline.push_cold_blocks_to_end(None, None);
        assert_eq!(
            pipeline
                .exception_owner_provenance
                .retained_setup_truths
                .len(),
            1
        );
        pipeline.remove_redundant_nops();
        assert_eq!(
            pipeline
                .exception_owner_provenance
                .retained_setup_truths
                .len(),
            1
        );
        let origin = &pipeline.exception_owner_provenance.retained_setup_truths[0];
        assert!(origin.removed);
        assert_eq!(origin.original, original);
        assert!(origin.records(&pipeline.graph.copy_items()));
        pipeline.validate_retained_setup_truth_graph();
        assert_eq!(
            pipeline
                .exception_owner_provenance
                .retained_setup_truths
                .len(),
            1
        );
    }
}

#[test]
fn retained_setup_start_refuses_other_fusions_and_source_edge_identities() {
    for mutation in 0..9 {
        let mut assembler = fixture(false);
        let origin = assembler
            .exception_owner_provenance
            .retained_setup_truths
            .pop()
            .unwrap();
        let returned = origin.source.start + origin.source.prefix.len() - 1;
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            unreachable!()
        };
        match mutation {
            0 => {}
            1 => {
                if let Item::Instruction(value) = &mut items[origin.source.start + 1] {
                    value.set_operand(Operand::Value(0));
                }
            }
            2 => {
                if let Item::Instruction(value) = &mut items[origin.source.start + 1] {
                    value.local_policy.add_hidden_exception_owner_slots(1);
                }
            }
            3 => {
                if let Item::Instruction(value) = &mut items[returned] {
                    value.opcode = LOAD_FAST_BORROW;
                }
            }
            4 => {
                items.insert(origin.source.start, Item::Label(l(99)));
            }
            5 => {
                items.push(Item::Instruction(origin.source.exit));
            }
            6 => {
                assembler
                    .exception_owner_provenance
                    .original_if_expression_joins
                    .push((l(1), origin.source.exit));
            }
            7 => {
                assembler
                    .exception_owner_provenance
                    .original_if_expression_joins
                    .clear();
            }
            8 => {
                items[origin.source.start + 2] = value(PUSH_NULL, 0, 3, 3);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            assembler
                .capture_retained_setup_truth_start(returned, 1, loc(4))
                .is_some(),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn retained_setup_cold_split_preserves_complete_original_membership() {
    for prepared in [false, true] {
        for mutation in 0..8 {
            let mut assembler = fixture(false);
            let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                unreachable!()
            };
            let target = items
                .iter()
                .position(|item| *item == Item::Label(l(9)))
                .unwrap();
            items.splice(
                target..target,
                [
                    Item::Label(l(20)),
                    value(RETURN_VALUE, 0, 20, 0),
                    Item::Label(l(21)),
                    value(RETURN_VALUE, 0, 21, 0),
                ],
            );
            items.extend([Item::Label(l(22)), value(RETURN_VALUE, 0, 22, 0)]);
            if prepared {
                let _ = assembler.used_constant_indices(1);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            marker_head(&mut pipeline);
            let mut origins =
                std::mem::take(&mut pipeline.exception_owner_provenance.retained_setup_truths);
            let mut receipt = RetainedSetupColdRewrite::capture(
                &pipeline.graph,
                &pipeline.exception_regions,
                &mut origins,
            )
            .unwrap();
            let moved = pipeline.graph.label_block(l(21)).unwrap();
            let fence = pipeline.graph.label_block(l(22)).unwrap();
            let mut order = pipeline.graph.order().to_vec();
            order.retain(|block| *block != moved);
            let at = order.iter().position(|block| *block == fence).unwrap();
            order.insert(at, moved);
            pipeline.graph.replace_order(order).unwrap();
            let original = pipeline.exception_regions[0];
            let start = pipeline.graph[pipeline.graph.label_block(original.start).unwrap()]
                .first_label()
                .unwrap();
            pipeline.exception_regions = vec![
                ExceptionRegion { start, ..original },
                ExceptionRegion {
                    start: l(21),
                    end: l(22),
                    ..original
                },
            ];
            receipt.output(0);
            if mutation != 1 {
                receipt.output(0);
            }
            receipt.group(0, if mutation == 2 { 1 } else { 2 });
            match mutation {
                0..=2 => {}
                3 => pipeline.exception_regions.swap(0, 1),
                4 => {
                    pipeline.exception_regions[1].depth += 1;
                }
                5 => {
                    pipeline.exception_regions[1].priority =
                        ExceptionRegionPriority::Resolved { start: 0, end: 1 };
                }
                6 => {
                    pipeline.exception_regions[1].start = l(22);
                }
                7 => {
                    pipeline.exception_regions[0].end = l(20);
                }
                _ => unreachable!(),
            }
            receipt.finish(
                &pipeline.graph,
                &pipeline.exception_regions,
                None,
                &mut origins,
            );
            assert_eq!(
                origins.len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
            if mutation == 0 {
                assert!(same_regions(
                    origins[0].regions.as_ref().unwrap(),
                    &pipeline.exception_regions
                ));
            }
        }
    }
}

#[test]
fn retained_setup_cold_zero_output_requires_an_unrelated_empty_original() {
    for prepared in [false, true] {
        for empty in [false, true] {
            let mut assembler = fixture(false);
            append(
                &mut assembler,
                [
                    Item::Label(l(22)),
                    value(RETURN_VALUE, 0, 22, 0),
                    Item::Label(l(23)),
                    value(RETURN_VALUE, 0, 23, 0),
                ],
            );
            assembler.exception_regions.push(ExceptionRegion {
                start: l(22),
                end: if empty { l(22) } else { l(23) },
                target: l(23),
                depth: 0,
                preserve_lasti: false,
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(l(22), l(23)),
            });
            if prepared {
                let _ = assembler.used_constant_indices(1);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            marker_head(&mut pipeline);
            let mut origins =
                std::mem::take(&mut pipeline.exception_owner_provenance.retained_setup_truths);
            let mut receipt = RetainedSetupColdRewrite::capture(
                &pipeline.graph,
                &pipeline.exception_regions,
                &mut origins,
            )
            .unwrap();
            receipt.output(0);
            receipt.group(0, 1);
            receipt.group(1, 0);
            pipeline.exception_regions.truncate(1);
            let region = &mut pipeline.exception_regions[0];
            region.start = pipeline.graph[pipeline.graph.label_block(region.start).unwrap()]
                .first_label()
                .unwrap();
            receipt.finish(
                &pipeline.graph,
                &pipeline.exception_regions,
                None,
                &mut origins,
            );
            assert_eq!(
                origins.len(),
                usize::from(empty),
                "prepared {prepared}, empty {empty}"
            );
        }
    }
}

fn region_fixture() -> (FxHashMap<Label, usize>, Vec<ExceptionRegion>) {
    let positions = (0..25)
        .map(|index| (Label::from_u32(index), index as usize))
        .collect();
    let region = ExceptionRegion {
        start: Label::from_u32(1),
        end: Label::from_u32(20),
        target: Label::from_u32(21),
        depth: 1,
        preserve_lasti: false,
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(Label::from_u32(1), Label::from_u32(20)),
    };
    (positions, vec![region])
}

#[test]
fn setup_regions_bind_complete_original_group() {
    let (positions, mut regions) = region_fixture();
    assert!(same_regions(
        &original_setup_regions(&regions, &positions, Label::from_u32(1), 3).unwrap(),
        &regions
    ));
    let original = regions[0];
    regions[0].end = Label::from_u32(10);
    regions.push(ExceptionRegion {
        start: Label::from_u32(12),
        ..original
    });
    assert!(same_regions(
        &original_setup_regions(&regions, &positions, Label::from_u32(1), 3).unwrap(),
        &regions
    ));
    let reject = |changed: &[ExceptionRegion]| {
        assert!(original_setup_regions(changed, &positions, Label::from_u32(1), 3).is_none());
    };
    reject(&regions[..1]);
    reject(&regions[1..]);
    reject(&[regions[0], regions[0], regions[1]]);
    reject(&[regions[1], regions[0]]);
    reject(&[
        regions[0],
        ExceptionRegion {
            start: Label::from_u32(9),
            ..regions[1]
        },
    ]);
    reject(&[
        regions[0],
        ExceptionRegion {
            start: Label::from_u32(10),
            ..regions[1]
        },
    ]);
    reject(&[
        ExceptionRegion {
            end: Label::from_u32(3),
            ..regions[0]
        },
        regions[1],
    ]);
    reject(&[
        regions[0],
        ExceptionRegion {
            end: Label::from_u32(19),
            ..regions[1]
        },
    ]);
    reject(&[
        regions[0],
        ExceptionRegion {
            depth: 2,
            ..regions[1]
        },
    ]);
    reject(&[
        regions[0],
        ExceptionRegion {
            preserve_lasti: true,
            ..regions[1]
        },
    ]);
    reject(&[
        regions[0],
        ExceptionRegion {
            is_scope_fallback: true,
            ..regions[1]
        },
    ]);
    reject(&[
        regions[0],
        ExceptionRegion {
            target: Label::from_u32(22),
            ..regions[1]
        },
    ]);
    reject(&[
        regions[0],
        ExceptionRegion {
            priority: ExceptionRegionPriority::OriginalRange(
                Label::from_u32(2),
                Label::from_u32(20),
            ),
            ..regions[1]
        },
    ]);
    reject(&[
        regions[0],
        regions[1],
        ExceptionRegion {
            start: Label::from_u32(22),
            end: Label::from_u32(23),
            ..original
        },
    ]);
    reject(&[
        regions[0],
        regions[1],
        ExceptionRegion {
            start: Label::from_u32(22),
            end: Label::from_u32(23),
            priority: ExceptionRegionPriority::OriginalRange(
                Label::from_u32(22),
                Label::from_u32(23),
            ),
            ..original
        },
    ]);
    for missing in [1, 10, 12, 20, 21] {
        let mut changed = positions.clone();
        changed.remove(&Label::from_u32(missing));
        assert!(original_setup_regions(&regions, &changed, Label::from_u32(1), 3).is_none());
    }
    let unrelated = ExceptionRegion {
        start: Label::from_u32(22),
        end: Label::from_u32(23),
        target: Label::from_u32(24),
        priority: ExceptionRegionPriority::OriginalRange(Label::from_u32(22), Label::from_u32(23)),
        ..original
    };
    let mut with_unrelated = regions.clone();
    with_unrelated.insert(1, unrelated);
    assert!(same_regions(
        &original_setup_regions(&with_unrelated, &positions, Label::from_u32(1), 3).unwrap(),
        &regions
    ));
}
