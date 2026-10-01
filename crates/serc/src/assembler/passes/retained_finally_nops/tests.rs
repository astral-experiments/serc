use super::*;
use crate::assembler::{Assembler, UnoptimizedCode};
use crate::target::opcodes::{LOAD_FAST, LOAD_GLOBAL};

fn label(value: u32) -> Label {
    Label::from_u32(value)
}

fn location(line: i32) -> SourceLocation {
    SourceLocation {
        line,
        end_line: line,
        column: 0,
        end_column: 8,
    }
}

fn record(opcode: crate::target::Opcode, operand: Operand, line: i32, depth: u32) -> Item {
    Item::Instruction(Instruction::new(
        opcode,
        operand,
        location(line),
        Some(depth),
    ))
}

fn fixture(fallback: bool, absent: bool, prepared: bool) -> OptimizationPipeline {
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(label(4)),
        location(3),
        Some(0),
    );
    jump.jump_policy.mark_original_empty_join();
    jump.jump_policy.prevent_small_exit_inlining();
    jump.jump_policy.retain_redundant_nop_after_threading();
    jump.no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
    let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
    pop.fusion_policy.mark_converted_pop_block();
    pop.set_exception_policy(if absent {
        ExceptionPolicy::AlwaysAbsent
    } else {
        ExceptionPolicy::EXCLUDE_ALWAYS
    });
    let mut ret = Instruction::new(RETURN_VALUE, Operand::Value(0), location(5), Some(0));
    ret.add_pre_fold_extra_instructions(1);
    let items = vec![
        Item::Label(label(0)),
        Item::Label(label(1)),
        record(LOAD_CONST, Operand::Value(0), 2, 1),
        record(POP_TOP, Operand::Value(0), 2, 0),
        record(JUMP_FORWARD, Operand::Forward(label(2)), 2, 0),
        Item::Label(label(2)),
        Item::Label(label(3)),
        Item::Instruction(jump),
        Item::Label(label(4)),
        Item::Label(label(5)),
        Item::Instruction(pop),
        record(LOAD_GLOBAL, Operand::Value(1), 5, 2),
        record(LOAD_FAST, Operand::Value(0), 5, 3),
        record(CALL, Operand::Value(1), 5, 1),
        record(POP_TOP, Operand::Value(0), 5, 0),
        record(LOAD_CONST, Operand::Value(0), 5, 1),
        Item::Instruction(ret),
        Item::Label(label(6)),
        record(LOAD_CONST, Operand::Value(0), 6, 1),
        record(RETURN_VALUE, Operand::Value(0), 6, 0),
        Item::Label(label(7)),
        record(LOAD_CONST, Operand::Value(0), 7, 1),
        record(RETURN_VALUE, Operand::Value(0), 7, 0),
        Item::Label(label(8)),
        record(LOAD_CONST, Operand::Value(0), 8, 1),
        record(RETURN_VALUE, Operand::Value(0), 8, 0),
    ];
    let mut assembler = Assembler::from_linear_items(items);
    assembler.python_version = PythonVersion::Py314;
    assembler.next_label = 9;
    assembler
        .exception_owner_provenance
        .compound_try_owner_exits
        .push((label(3), label(4)));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .extend([label(3), label(4)]);
    assembler
        .exception_owner_provenance
        .source_boundaries
        .insert(label(3));
    assembler.borrow_finally_entries.insert(label(5), None);
    assembler.borrow_unreachable_blocks.insert(label(5));
    assembler.borrow_jump_entry_labels.insert(label(5));
    assembler.exception_regions = vec![
        ExceptionRegion {
            start: label(1),
            end: label(3),
            target: label(6),
            depth: 0,
            preserve_lasti: false,
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::Resolved { start: 1, end: 6 },
        },
        ExceptionRegion {
            start: label(1),
            end: label(5),
            target: label(7),
            depth: 0,
            preserve_lasti: false,
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::Resolved { start: 1, end: 9 },
        },
    ];
    if fallback {
        assembler.exception_regions.push(ExceptionRegion {
            start: label(0),
            end: label(8),
            target: label(8),
            depth: 0,
            preserve_lasti: true,
            is_scope_fallback: true,
            priority: ExceptionRegionPriority::Resolved { start: 0, end: 23 },
        });
    }
    if prepared {
        assembler.used_constant_indices(1);
        assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    }
    let mut pipeline = assembler.into_optimization_pipeline();
    pipeline.exception_owner_provenance.block_boundaries =
        FxHashSet::from_iter([label(3), label(4)]);
    pipeline.borrow_finally_jump_sources.insert(label(3), true);
    pipeline
}

fn edit_items(pipeline: &mut OptimizationPipeline, edit: impl FnOnce(&mut Vec<Item>)) {
    let mut items = pipeline.graph.copy_items();
    edit(&mut items);
    pipeline.graph = BlockGraph::from_items(items, &pipeline.preserved_block_boundaries);
}

fn finish(pipeline: &mut OptimizationPipeline, proof: &mut RetainedFinallyNops) {
    for stage in [
        RegionStage::Threading,
        RegionStage::ExitCopy,
        RegionStage::Cold,
    ] {
        assert!(proof.before_region_rewrite(pipeline, stage));
        proof.after_region_rewrite(pipeline, &[], stage);
        assert!(proof.origins[0].live);
        if stage == RegionStage::Threading {
            pipeline.remove_redundant_forward_jumps(false);
        }
    }
    proof.observe_pop_cleanup(pipeline, false);
    pipeline.remove_redundant_nops();
    proof.observe_pop_cleanup(pipeline, true);
    assert!(proof.origins[0].removed_pop);
    pipeline.remove_redundant_forward_jumps_with_finally_nops(
        true,
        proof,
        &mut crate::assembler::guarded_loop_nops::GuardedLoopNops::default(),
    );
    assert!(proof.origins[0].receipt.is_some());
    proof.revalidate(pipeline);
    assert!(proof.origins[0].live);
}

fn entries(
    pipeline: &OptimizationPipeline,
    proof: &RetainedFinallyNops,
) -> FxHashMap<Label, Label> {
    proof.entries(
        &pipeline.graph.copy_items(),
        &pipeline.exception_regions,
        &HeadView::pipeline(pipeline),
    )
}

#[test]
fn retained_finally_nop_requires_original_constructor_and_real_rewrites() {
    for fallback in [false, true] {
        for absent in [false, true] {
            for prepared in [false, true] {
                let mut pipeline = fixture(fallback, absent, prepared);
                let mut proof = RetainedFinallyNops::capture(&pipeline);
                assert_eq!(proof.origins.len(), 1);
                assert!(entries(&pipeline, &proof).is_empty());
                finish(&mut pipeline, &mut proof);
                let expected = FxHashMap::from_iter([(label(3), label(4))]);
                assert_eq!(entries(&pipeline, &proof), expected);
                assert_eq!(entries(&pipeline, &proof), expected);
            }
        }
    }
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py315,
    ] {
        let mut pipeline = fixture(false, false, false);
        pipeline.python_version = version;
        assert!(RetainedFinallyNops::capture(&pipeline).origins.is_empty());
    }
}

#[test]
fn retained_finally_nop_rejects_changed_original_records_and_metadata() {
    let original = fixture(false, false, false).graph.copy_items();
    for index in 5..17 {
        for mutation in 0..4 {
            let mut pipeline = fixture(false, false, false);
            let mut proof = RetainedFinallyNops::capture(&pipeline);
            edit_items(&mut pipeline, |items| match &mut items[index] {
                Item::Label(value) => *value = label(90),
                Item::Instruction(value) => match mutation {
                    0 => value.location.column += 1,
                    1 => value.set_operand(Operand::Value(99)),
                    2 => value.set_exception_policy(ExceptionPolicy::AlwaysAbsent),
                    _ => value.local_policy.add_hidden_exception_owner_slots(1),
                },
            });
            assert_ne!(pipeline.graph.copy_items(), original);
            assert!(
                !proof.before_region_rewrite(&pipeline, RegionStage::Threading),
                "index={index} mutation={mutation}"
            );
        }
    }
    for mutation in 0..9 {
        let mut pipeline = fixture(false, false, false);
        let mut proof = RetainedFinallyNops::capture(&pipeline);
        match mutation {
            0 => {
                pipeline.borrow_unreachable_blocks.insert(label(3));
            }
            1 => {
                pipeline.borrow_block_aliases.insert(label(3));
            }
            2 => {
                pipeline.borrow_finally_entries.remove(&label(5));
            }
            3 => {
                pipeline
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .push((label(3), label(4)));
            }
            4 => {
                pipeline.exception_owner_provenance.compound_try_owner_exits[0].1 = label(5);
            }
            5 => {
                pipeline
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(label(4), 1);
            }
            6 => {
                pipeline
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(label(2));
            }
            7 => {
                pipeline
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(label(2));
            }
            _ => edit_items(&mut pipeline, |items| {
                items.extend([Item::Label(label(9)), original[7]]);
            }),
        }
        assert!(
            !proof.before_region_rewrite(&pipeline, RegionStage::Threading),
            "mutation={mutation}"
        );
    }
}

#[test]
fn retained_finally_nop_rejects_noncanonical_pop_and_fallback_regions() {
    for policy in [
        ExceptionPolicy::NoneUnchanged,
        ExceptionPolicy::NoneAbsent,
        ExceptionPolicy::IfExtendedUnchanged,
        ExceptionPolicy::AlwaysPresent,
        ExceptionPolicy::AlwaysOverridePresent,
        ExceptionPolicy::AlwaysDynamic,
    ] {
        let mut pipeline = fixture(false, false, false);
        edit_items(&mut pipeline, |items| {
            let Item::Instruction(pop) = &mut items[10] else {
                panic!()
            };
            pop.set_exception_policy(policy);
        });
        assert!(RetainedFinallyNops::capture(&pipeline).origins.is_empty());
    }
    for mutation in 0..9 {
        let mut pipeline = fixture(true, true, false);
        match mutation {
            0 => pipeline.exception_regions[2].depth = 1,
            1 => pipeline.exception_regions[2].preserve_lasti = false,
            2 => pipeline.exception_regions[2].is_scope_fallback = false,
            3 => pipeline.exception_regions[2].end = label(7),
            4 => pipeline.exception_regions[2].start = label(4),
            5 => pipeline.exception_regions[1].end = label(6),
            6 => {
                pipeline
                    .exception_regions
                    .push(pipeline.exception_regions[2]);
            }
            7 => pipeline.exception_regions[0].start = label(80),
            _ => pipeline.exception_regions[0].end = label(80),
        }
        assert!(
            RetainedFinallyNops::capture(&pipeline).origins.is_empty(),
            "mutation={mutation}"
        );
    }
}

#[test]
fn retained_finally_nop_transports_only_exact_ordered_region_receipts() {
    for mutation in 0..8 {
        let mut pipeline = fixture(false, false, false);
        let mut proof = RetainedFinallyNops::capture(&pipeline);
        assert!(proof.before_region_rewrite(&pipeline, RegionStage::Threading));
        let original = pipeline.exception_regions[0];
        edit_items(&mut pipeline, |items| {
            items.insert(2, Item::Label(label(9)));
        });
        pipeline.exception_regions[0].start = label(9);
        let mut receipts = vec![(original, pipeline.exception_regions[0])];
        match mutation {
            0 => {}
            1 => receipts.clear(),
            2 => pipeline.exception_regions[0].depth = 1,
            3 => receipts[0].1.depth = 1,
            4 => {
                pipeline
                    .exception_regions
                    .push(pipeline.exception_regions[0]);
            }
            5 => pipeline.exception_regions.swap(0, 1),
            6 => {
                receipts.push(receipts[0]);
            }
            _ => pipeline.exception_regions[0].end = label(5),
        }
        proof.after_region_rewrite(&pipeline, &receipts, RegionStage::Threading);
        assert_eq!(proof.origins[0].live, mutation == 0, "mutation={mutation}");
    }
}

#[test]
fn retained_finally_nop_rejects_late_source_target_and_membership_drift() {
    for mutation in 0..9 {
        let mut pipeline = fixture(true, true, false);
        let mut proof = RetainedFinallyNops::capture(&pipeline);
        finish(&mut pipeline, &mut proof);
        assert_eq!(entries(&pipeline, &proof).len(), 1);
        match mutation {
            0 => edit_items(&mut pipeline, |items| {
                let index = items
                    .iter()
                    .position(|item| *item == Item::Label(label(3)))
                    .unwrap()
                    + 1;
                let Item::Instruction(record) = &mut items[index] else {
                    panic!()
                };
                record.location.line += 1;
            }),
            1 => edit_items(&mut pipeline, |items| {
                let index = items
                    .iter()
                    .position(|item| *item == Item::Label(label(5)))
                    .unwrap()
                    + 1;
                let Item::Instruction(record) = &mut items[index] else {
                    panic!()
                };
                record.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
            }),
            2 => edit_items(&mut pipeline, |items| {
                items.insert(8, Item::Label(label(9)));
            }),
            3 => pipeline.exception_regions[2].start = label(3),
            4 => pipeline.exception_regions[2].preserve_lasti = false,
            5 => pipeline.exception_regions[0].end = label(4),
            6 => {
                pipeline
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(label(3), 1);
            }
            7 => {
                pipeline.borrow_jump_entry_labels.remove(&label(5));
            }
            _ => {
                pipeline.exception_regions.swap(0, 1);
            }
        }
        proof.revalidate(&pipeline);
        assert!(entries(&pipeline, &proof).is_empty(), "mutation={mutation}");
    }
}

#[test]
fn retained_finally_nop_rejects_duplicate_labels_before_graph_construction() {
    let pipeline = fixture(true, true, false);
    let proof = RetainedFinallyNops::capture(&pipeline);
    let original = pipeline.graph.copy_items();
    for duplicated in [0, 1, 2, 3, 4, 5, 6, 7, 8] {
        let mut items = original.clone();
        items.insert(5, Item::Label(label(duplicated)));
        assert!(positions(&items).is_none());
        assert!(
            proof
                .entries(
                    &items,
                    &pipeline.exception_regions,
                    &HeadView::pipeline(&pipeline)
                )
                .is_empty()
        );
    }
}

#[test]
fn retained_finally_nop_entries_admit_only_the_exact_fallthrough() {
    use super::super::locals::BorrowEntryLabels;
    use crate::target::opcodes::{LOAD_FAST_BORROW, POP_JUMP_IF_FALSE};

    for kind in 0..6 {
        let mut pipeline = fixture(false, false, false);
        let mut proof = RetainedFinallyNops::capture(&pipeline);
        finish(&mut pipeline, &mut proof);
        let mut entries = entries(&pipeline, &proof);
        let mut items = pipeline.graph.copy_items();
        let index = items
            .iter()
            .position(|item| *item == Item::Label(label(3)))
            .unwrap()
            + 1;
        match kind {
            0 => {}
            1 => entries.clear(),
            2 => {
                entries.insert(label(3), label(6));
            }
            3 => {
                let Item::Instruction(source) = &mut items[index] else {
                    panic!()
                };
                source
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Reachable);
            }
            4 => items[index] = record(JUMP_FORWARD, Operand::Forward(label(4)), 3, 0),
            _ => items[index] = record(POP_JUMP_IF_FALSE, Operand::Forward(label(4)), 3, 0),
        }
        OptimizationPipeline::optimize_load_fast(
            &mut items,
            &pipeline.exception_regions,
            BorrowEntryLabels {
                unreachable: &pipeline.borrow_unreachable_blocks,
                loop_exits: &FxHashMap::default(),
                finally_entries: &pipeline.borrow_finally_entries,
                finally_jump_sources: &pipeline.borrow_finally_jump_sources,
                jump_entries: &pipeline.borrow_jump_entry_labels,
                original_boundaries: &pipeline.exception_owner_provenance.block_boundaries,
                pre_copy_jump_entries: &FxHashMap::default(),
                authored_finally_break_entries: &FxHashMap::default(),
                retained_nop_finally_entries: &entries,
            },
            &FxHashSet::default(),
            &FxHashSet::default(),
            false,
        );
        let loaded = items
            .iter()
            .find_map(|item| match item {
                Item::Instruction(record)
                    if matches!(record.opcode, LOAD_FAST | LOAD_FAST_BORROW) =>
                {
                    Some(record.opcode)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(loaded == LOAD_FAST_BORROW, kind == 0, "kind={kind}");
    }
}

#[test]
fn retained_finally_nop_fallback_split_keeps_original_handler_and_membership() {
    for mutation in 0..7 {
        let mut pipeline = fixture(true, true, false);
        let mut proof = RetainedFinallyNops::capture(&pipeline);
        assert!(proof.before_region_rewrite(&pipeline, RegionStage::Threading));
        let original = pipeline.exception_regions[2];
        edit_items(&mut pipeline, |items| {
            let end = items
                .iter()
                .position(|item| *item == Item::Label(label(6)))
                .unwrap();
            items.insert(end, Item::Label(label(9)));
        });
        pipeline.exception_regions[2].end = label(9);
        let mut receipts = vec![(original, pipeline.exception_regions[2])];
        match mutation {
            0 => {}
            1 => pipeline.exception_regions[2].target = label(9),
            2 => {
                pipeline.exception_regions[2].priority =
                    ExceptionRegionPriority::Resolved { start: 0, end: 24 }
            }
            3 => {
                receipts[0].1.end = label(7);
            }
            4 => {
                pipeline.exception_regions[2].end = label(4);
                receipts[0].1.end = label(4);
            }
            5 => {
                pipeline
                    .exception_regions
                    .push(pipeline.exception_regions[2]);
            }
            _ => {
                pipeline.exception_regions[2].start = label(4);
                receipts[0].1.start = label(4);
            }
        }
        proof.after_region_rewrite(&pipeline, &receipts, RegionStage::Threading);
        assert_eq!(proof.origins[0].live, mutation == 0, "mutation={mutation}");
    }
}

#[test]
fn retained_finally_nop_requires_fresh_reachability_and_an_unforged_receipt() {
    for mutation in 0..4 {
        let mut pipeline = fixture(false, false, false);
        let mut proof = RetainedFinallyNops::capture(&pipeline);
        finish(&mut pipeline, &mut proof);
        match mutation {
            0 => proof.origins[0].receipt = None,
            1 => proof.origins[0]
                .receipt
                .as_mut()
                .unwrap()
                .set_exception_policy(ExceptionPolicy::AlwaysAbsent),
            2 => proof.origins[0].removed_pop = false,
            _ => edit_items(&mut pipeline, |items| {
                items.insert(2, record(RETURN_VALUE, Operand::Value(0), 2, 0));
            }),
        }
        proof.revalidate(&pipeline);
        assert!(entries(&pipeline, &proof).is_empty(), "mutation={mutation}");
    }
}
