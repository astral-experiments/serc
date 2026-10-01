use super::*;
use crate::assembler::block_graph::ItemPosition;
use crate::target::opcodes::{NOT_TAKEN, RERAISE, RETURN_VALUE, TO_BOOL};
use crate::target::operands::SpecialMethod;

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
    fixture_with_none(0)
}

fn fixture_with_none(none: u32) -> Assembler {
    let op = |opcode, argument, line, depth| {
        Item::Instruction(record(opcode, Operand::Value(argument), line, depth))
    };
    let jump = |target, line| {
        let mut record = record(JUMP_FORWARD, Operand::Forward(label(target)), line, 0);
        record.jump_policy.prevent_small_exit_inlining();
        Item::Instruction(record)
    };
    let boolean = record(POP_JUMP_IF_FALSE, Operand::Forward(label(12)), 3, 1);
    let value_prefix = vec![
        Item::Label(label(13)),
        op(COPY, 1, 3, 2),
        op(TO_BOOL, 0, 3, 2),
        Item::Instruction(boolean),
        Item::Label(label(14)),
        op(NOT_TAKEN, 0, 3, 1),
    ];
    let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(0));
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    pop.fusion_policy.mark_converted_pop_block();
    let mut items = vec![Item::Label(label(0)), op(LOAD_FAST, 1, 3, 1)];
    items.extend(&value_prefix);
    items.extend([
        Item::Label(label(16)),
        op(POP_TOP, 0, 3, 0),
        op(LOAD_FAST, 2, 3, 1),
        Item::Label(label(12)),
        op(STORE_FAST, 3, 3, 0),
        Item::Label(label(8)),
        jump(11, 3),
        Item::Label(label(9)),
        op(PUSH_EXC_INFO, 0, 4, 2),
        Item::Label(label(20)),
        op(LOAD_FAST, 0, 7, 2),
        op(POP_TOP, 0, 7, 1),
        Item::Label(label(21)),
        op(POP_EXCEPT, 0, 7, 0),
        op(LOAD_CONST, none, 7, 1),
        op(STORE_FAST, 4, 7, 0),
        op(DELETE_FAST, 4, 7, 0),
        jump(11, 7),
        Item::Label(label(22)),
        op(RERAISE, 1, 7, 0),
        Item::Label(label(11)),
        Item::Label(label(1)),
        Item::Instruction(pop),
        op(LOAD_FAST, 0, 9, 1),
        op(LOAD_ATTR, 6, 9, 1),
        op(COPY, 1, 9, 2),
        op(LOAD_SPECIAL, SpecialMethod::__EXIT__.argument(), 9, 3),
        op(SWAP, 2, 9, 3),
        op(SWAP, 3, 9, 3),
        op(LOAD_SPECIAL, SpecialMethod::__ENTER__.argument(), 9, 4),
        op(CALL, 0, 9, 3),
        Item::Label(label(26)),
        op(POP_TOP, 0, 9, 2),
        op(LOAD_FAST, 0, 10, 3),
        op(LOAD_ATTR, 9, 10, 4),
        op(CALL, 0, 10, 3),
        op(POP_TOP, 0, 10, 2),
        Item::Label(label(27)),
        jump(33, 10),
        Item::Label(label(28)),
        op(PUSH_EXC_INFO, 0, 9, 5),
        op(RERAISE, 2, 9, 4),
        Item::Label(label(30)),
        jump(33, 9),
        Item::Label(label(31)),
        op(RERAISE, 1, 9, 0),
        Item::Label(label(33)),
        op(LOAD_CONST, none, 11, 1),
        op(RETURN_VALUE, 0, 11, 0),
    ]);
    let mut assembler = Assembler::from_linear_items(items);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 34;
    assembler.exception_regions = vec![
        region(0, 8, 9, 0, false),
        region(20, 21, 22, 1, true),
        region(26, 27, 28, 2, true),
        region(28, 30, 31, 4, true),
    ];
    assembler.borrow_unreachable_blocks.insert(label(1));
    assembler.borrow_finally_entries.insert(label(1), None);
    assembler.borrow_jump_entry_labels.insert(label(1));
    assembler.preserved_block_boundaries.insert(label(12));
    let provenance = &mut assembler.exception_owner_provenance;
    provenance.source_boundaries.extend([label(11), label(12)]);
    provenance.block_boundaries.extend([label(11), label(12)]);
    provenance
        .compound_try_owner_exits
        .push((label(8), label(11)));
    provenance
        .original_value_jump_prefixes
        .push((label(13), value_prefix));
    provenance.ordinary_try_setups.insert(label(0), label(9));
    provenance.synchronous_with_setups.insert(label(26));
    provenance.setups.push((label(26), None));
    provenance.named_handler_cleanup_targets.insert(label(22));
    assembler
}

fn capture(assembler: &mut Assembler) {
    let ordinal = assembler
        .linear_items()
        .iter()
        .filter_map(instruction)
        .position(|record| record.opcode == NOP && record.location == SourceLocation::NONE)
        .unwrap();
    assembler.record_retained_store_finally_entry(label(1), ordinal, location(3), location(9));
}

fn ready() -> OptimizationPipeline {
    let mut assembler = fixture();
    capture(&mut assembler);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .retained_store_finally_entries
            .len(),
        1
    );
    let mut pipeline = assembler.into_optimization_pipeline();
    pipeline.borrow_finally_jump_sources.insert(label(8), true);
    pipeline
}

fn assert_guarded(pipeline: &mut OptimizationPipeline) {
    let items = pipeline.graph.copy_items();
    pipeline.admit_visited_retained_store_finally_entries();
    assert_eq!(pipeline.graph.copy_items(), items);
    assert!(pipeline.borrow_unreachable_blocks.contains(&label(1)));
    assert_eq!(pipeline.borrow_finally_entries.get(&label(1)), Some(&None));
    assert!(pipeline.borrow_jump_entry_labels.contains(&label(1)));
    assert!(!pipeline.borrow_block_aliases.contains(&label(1)));
    assert!(!pipeline.preserved_block_boundaries.contains(&label(1)));
}

#[test]
fn retained_store_waits_for_the_existing_original_source_visit() {
    for visit in [None, Some(false), Some(true)] {
        let mut pipeline = ready();
        pipeline.borrow_finally_jump_sources.remove(&label(8));
        if let Some(visit) = visit {
            pipeline.borrow_finally_jump_sources.insert(label(8), visit);
        }
        if visit != Some(true) {
            assert_guarded(&mut pipeline);
        } else {
            let items = pipeline.graph.copy_items();
            pipeline.admit_visited_retained_store_finally_entries();
            assert_eq!(pipeline.graph.copy_items(), items);
            assert!(!pipeline.borrow_unreachable_blocks.contains(&label(1)));
            assert!(!pipeline.borrow_finally_entries.contains_key(&label(1)));
            assert!(!pipeline.borrow_jump_entry_labels.contains(&label(1)));
            assert!(pipeline.borrow_block_aliases.contains(&label(1)));
            assert!(pipeline.preserved_block_boundaries.contains(&label(1)));
        }
    }
}

#[test]
fn retained_store_replays_complete_arrays_and_instruction_metadata() {
    let changes: [fn(&mut Instruction); 8] = [
        |record| record.opcode = NOP,
        |record| record.set_operand(Operand::Value(77)),
        |record| record.location.column += 1,
        |record| {
            *record = Instruction::new(
                record.opcode,
                record.operand(),
                record.location,
                record.depth_after().map(|depth| depth + 1),
            );
        },
        |record| record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
        |record| record.jump_policy.prevent_small_exit_inlining(),
        |record| record.no_location_policy.preserve_location(),
        |record| record.fusion_policy.mark_converted_pop_block(),
    ];
    let arrays = ready()
        .exception_owner_provenance
        .retained_store_finally_entries[0]
        .arrays
        .clone();
    for original in arrays {
        for (array_offset, record) in original
            .iter()
            .enumerate()
            .filter_map(|(index, item)| instruction(item).map(|record| (index, record)))
        {
            for change in changes {
                let mut pipeline = ready();
                let mut changed = record;
                change(&mut changed);
                if changed == record {
                    continue;
                }
                let label_at = original[..array_offset]
                    .iter()
                    .rposition(|item| matches!(item, Item::Label(_)))
                    .unwrap();
                let Item::Label(label) = original[label_at] else {
                    unreachable!();
                };
                let block = pipeline.graph.label_block(label).unwrap();
                let label_offset = pipeline.graph[block]
                    .items()
                    .iter()
                    .position(|item| *item == Item::Label(label))
                    .unwrap();
                let offset = label_offset + array_offset - label_at;
                assert_eq!(
                    pipeline.graph[block].items()[offset],
                    Item::Instruction(record)
                );
                *pipeline
                    .graph
                    .instruction_mut(ItemPosition::new(block, offset))
                    .unwrap() = changed;
                assert_guarded(&mut pipeline);
            }
        }
    }
}

#[test]
fn retained_store_rejects_role_owner_and_incoming_substitution() {
    let mutations: [fn(&mut OptimizationPipeline); 20] = [
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .source_boundaries
                .remove(&label(11));
        },
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .block_boundaries
                .insert(label(1));
        },
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .synchronous_with_setups
                .remove(&label(26));
        },
        |pipeline| {
            pipeline.exception_owner_provenance.setups[0].1 = Some(location(9));
        },
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .ordinary_try_setups
                .insert(label(26), label(28));
        },
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .named_handler_cleanup_targets
                .remove(&label(22));
        },
        |pipeline| {
            pipeline.exception_owner_provenance.compound_try_owner_exits[0].0 = label(12);
        },
        |pipeline| {
            pipeline.exception_regions[0].depth += 1;
        },
        |pipeline| {
            pipeline.exception_regions[1].end = label(22);
        },
        |pipeline| {
            pipeline.exception_regions[2].preserve_lasti = false;
        },
        |pipeline| {
            pipeline
                .exception_regions
                .push(pipeline.exception_regions[2]);
        },
        |pipeline| {
            pipeline.exception_regions[3].priority =
                ExceptionRegionPriority::OriginalRange(label(28), label(31));
        },
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .block_boundaries
                .remove(&label(11));
        },
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .source_boundaries
                .insert(label(1));
        },
        |pipeline| {
            pipeline.preserved_block_boundaries.remove(&label(12));
        },
        |pipeline| {
            pipeline.original_nop_targets.insert(label(12));
        },
        |pipeline| {
            pipeline.exception_regions[0].start = label(13);
        },
        |pipeline| {
            pipeline
                .exception_owner_provenance
                .original_value_jump_prefixes[0]
                .1[1] = Item::Instruction(record(NOP, Operand::Value(0), 3, 0));
        },
        |pipeline| {
            pipeline.exception_regions.swap(0, 1);
        },
        |pipeline| {
            pipeline.exception_regions.swap(2, 3);
        },
    ];
    for mutation in mutations {
        let mut pipeline = ready();
        mutation(&mut pipeline);
        assert_guarded(&mut pipeline);
    }
    let mut pipeline = ready();
    let block = pipeline.graph.label_block(label(33)).unwrap();
    let offset = pipeline.graph[block].items().len();
    pipeline.graph.insert_item(
        ItemPosition::new(block, offset),
        Item::Instruction(record(JUMP_FORWARD, Operand::Forward(label(1)), 11, 0)),
    );
    assert_guarded(&mut pipeline);
}

#[test]
fn retained_store_rejects_new_exception_region_owners() {
    for overlaps in [false, true] {
        let mut pipeline = ready();
        let mut added = pipeline.exception_regions[0];
        added.target = label(33);
        added.priority = ExceptionRegionPriority::Resolved {
            start: 10000,
            end: 10001,
        };
        if !overlaps {
            added.start = label(30);
            added.end = label(31);
        }
        pipeline.exception_regions.push(added);
        assert_guarded(&mut pipeline);
    }
}

#[test]
fn retained_store_does_not_cross_an_existing_walk_barrier() {
    for empty in [false, true] {
        let mut assembler = fixture();
        capture(&mut assembler);
        let mut pipeline = assembler.into_optimization_pipeline();
        let mut head = vec![Item::Label(label(34))];
        if !empty {
            let mut barrier = record(NOP, Operand::Value(0), 1, 0);
            barrier
                .no_location_policy
                .set_borrow_fallthrough(crate::assembler::BorrowFallthrough::Barrier);
            head.push(Item::Instruction(barrier));
        }
        pipeline.graph.insert_block(0, head);
        pipeline.record_borrow_cleanup_calls();
        assert_eq!(
            pipeline.borrow_finally_jump_sources.get(&label(8)),
            Some(&false)
        );
        assert!(pipeline.borrow_unreachable_blocks.contains(&label(1)));
        assert!(!pipeline.borrow_block_aliases.contains(&label(1)));
        assert!(!pipeline.preserved_block_boundaries.contains(&label(1)));
    }
}

#[test]
fn retained_store_authenticates_roles_before_normalization() {
    let mut assembler = fixture();
    capture(&mut assembler);
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(label(1));
    let mut pipeline = assembler.into_optimization_pipeline();
    pipeline.borrow_finally_jump_sources.insert(label(8), true);
    assert!(
        pipeline
            .exception_owner_provenance
            .retained_store_finally_entries
            .is_empty()
    );
    assert_guarded(&mut pipeline);
}

#[test]
fn retained_store_keeps_authored_cleanup_immutable_during_remapping() {
    let mut assembler = fixture();
    capture(&mut assembler);
    let origin = &mut assembler
        .exception_owner_provenance
        .retained_store_finally_entries[0];
    let authored = origin.handler_author;
    assert!(origin.remap_constants(&[Some(3)], None));
    assert_eq!(origin.handler_author, authored);
    assert_eq!(origin.handler_current_none, 3);
    assert!(origin.handler_projection_matches());
    origin.handler_author[1].location.line += 1;
    assert!(!origin.handler_projection_matches());
    assert!(!origin.remap_constants(&[None, None, None, Some(0)], None));
}

#[test]
fn retained_store_rejects_current_or_authored_cleanup_replay_drift() {
    for authored in [false, true] {
        let mut pipeline = ready();
        let origin = &mut pipeline
            .exception_owner_provenance
            .retained_store_finally_entries[0];
        if authored {
            origin.handler_author[4]
                .no_location_policy
                .preserve_location();
        } else {
            let Item::Instruction(record) = &mut origin.arrays[origin.handler_array][2] else {
                unreachable!();
            };
            record.set_operand(Operand::Value(8));
        }
        assert_guarded(&mut pipeline);
    }
}

#[test]
fn retained_store_replays_after_the_real_constant_remap() {
    let mut assembler = fixture_with_none(3);
    capture(&mut assembler);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .retained_store_finally_entries[0]
            .handler_author[1]
            .operand(),
        Operand::Value(3)
    );
    assembler.used_constant_indices(4);
    assembler.remap_constant_indices(&[None, None, None, Some(0)]);
    let mut pipeline = assembler.into_optimization_pipeline();
    let origin = &pipeline
        .exception_owner_provenance
        .retained_store_finally_entries[0];
    assert_eq!(origin.handler_author[1].operand(), Operand::Value(3));
    assert_eq!(origin.handler_current_none, 0);
    pipeline.borrow_finally_jump_sources.insert(label(8), true);
    pipeline.admit_visited_retained_store_finally_entries();
    assert!(!pipeline.borrow_unreachable_blocks.contains(&label(1)));
    assert!(pipeline.borrow_block_aliases.contains(&label(1)));
}
