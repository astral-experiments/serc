use super::*;
use crate::assembler::ir::{EmittedInstruction, ExceptionPolicy};
use crate::target::opcodes::{LOAD_CONST, NOP, POP_JUMP_IF_FALSE, RETURN_VALUE};

fn label(value: u32) -> Label {
    Label::from_u32(value)
}
fn location(line: i32) -> super::super::SourceLocation {
    super::super::SourceLocation::new(line, line, 0, 8)
}
fn fixture() -> (Assembler, EmittedJump) {
    let guard = Instruction::new(
        POP_JUMP_IF_FALSE,
        Operand::Forward(label(1)),
        location(2),
        Some(0),
    );
    let raised = Instruction::new(RAISE_VARARGS, Operand::Value(1), location(3), Some(0));
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(label(3)),
        location(2),
        Some(0),
    );
    jump.jump_policy.prevent_small_exit_inlining();
    jump.jump_policy.retain_nop_for_direct_target();
    jump.no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Instruction(guard),
        Item::Label(label(0)),
        Item::Instruction(raised),
        Item::Label(label(1)),
        Item::Label(label(2)),
        Item::Instruction(jump),
    ]);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 8;
    assembler
        .exception_owner_provenance
        .compound_try_owner_exits
        .push((label(2), label(3)));
    assembler
        .exception_owner_provenance
        .source_boundaries
        .insert(label(1));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(label(1));
    let emitted = EmittedJump(EmittedInstruction {
        stream: assembler.emission_stream,
        item_index: 5,
    });
    (assembler, emitted)
}
fn context(assembler: &Assembler) -> Context<'_> {
    Context {
        provenance: &assembler.exception_owner_provenance,
        regions: &assembler.exception_regions,
        aliases: &assembler.borrow_block_aliases,
        preserved: &assembler.preserved_block_boundaries,
        original_nops: &assembler.original_nop_targets,
        unreachable: &assembler.borrow_unreachable_blocks,
        finally_entries: &assembler.borrow_finally_entries,
        finally_sources: None,
    }
}
fn items_mut(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items
}
fn complete(assembler: &mut Assembler) {
    items_mut(assembler).extend([
        Item::Label(label(3)),
        Item::Instruction(Instruction::new(
            NOP,
            Operand::Value(0),
            location(5),
            Some(0),
        )),
        Item::Instruction(Instruction::new(
            LOAD_CONST,
            Operand::Value(0),
            location(6),
            Some(1),
        )),
        Item::Instruction(Instruction::new(
            RETURN_VALUE,
            Operand::Value(0),
            location(6),
            Some(0),
        )),
    ]);
}
fn matches(assembler: &Assembler, origin: &Source) -> bool {
    let sources = labels(&origin.head).map(|label| (label, true)).collect();
    matches_with_sources(assembler, origin, Some(&sources))
}
fn matches_with_sources(
    assembler: &Assembler,
    origin: &Source,
    sources: Option<&FxHashMap<Label, bool>>,
) -> bool {
    if !origin.matches_items(assembler.linear_items(), 5, &context(assembler)) {
        return false;
    }
    let graph = BlockGraph::from_items(assembler.linear_items().to_vec(), &FxHashSet::default());
    let Some(block) = graph.label_block(label(2)) else {
        return false;
    };
    let Some(Item::Instruction(jump)) = graph[block].items().get(2) else {
        return false;
    };
    let context = Context {
        finally_sources: sources,
        ..context(assembler)
    };
    origin.matches(&graph, ItemPosition::new(block, 2), *jump, &context)
}

#[test]
fn strict_compound_try_nop_preserves_legacy_and_requires_original_source() {
    for version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        let (mut assembler, emitted) = fixture();
        assembler.python_version = version;
        assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
        assert!(
            assembler
                .exception_owner_provenance
                .ownerless_compound_try_nops
                .is_empty()
        );
        if version < crate::PythonVersion::Py314 {
            assert!(
                assembler
                    .exception_owner_provenance
                    .synchronous_compound_try_nops
                    .is_empty()
            );
            continue;
        }
        let origin = assembler
            .exception_owner_provenance
            .synchronous_compound_try_nops[0]
            .clone();
        complete(&mut assembler);
        assert!(matches(&assembler, &origin));
        assembler.remap_constant_indices(&[Some(1), Some(0)]);
        assert!(matches(&assembler, &origin));
    }
}

#[test]
fn strict_compound_try_nop_capture_refuses_changed_context_and_records() {
    for mutation in 0..14 {
        let (mut assembler, emitted) = fixture();
        match mutation {
            0 => {
                assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .clear();
            }
            1 => {
                assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .push((label(2), label(3)));
            }
            2 => {
                assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .push((label(1), label(4)));
            }
            3 => {
                assembler.borrow_block_aliases.insert(label(1));
            }
            4 => {
                assembler
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(label(2), 1);
            }
            5 => {
                items_mut(&mut assembler)[3] = Item::Label(label(3));
            }
            6 => {
                items_mut(&mut assembler)[2] = Item::Instruction(Instruction::new(
                    RAISE_VARARGS,
                    Operand::Value(1),
                    location(3),
                    Some(1),
                ));
            }
            7 => {
                let Item::Instruction(guard) = &mut items_mut(&mut assembler)[0] else {
                    unreachable!()
                };
                guard.location = location(9);
            }
            8 => {
                let Item::Instruction(jump) = &mut items_mut(&mut assembler)[5] else {
                    unreachable!()
                };
                jump.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            }
            9 => {
                items_mut(&mut assembler)[1] = Item::Label(label(1));
            }
            10 => {
                let Item::Instruction(guard) = &mut items_mut(&mut assembler)[0] else {
                    unreachable!()
                };
                guard.local_policy.add_hidden_exception_owner_slots(1);
            }
            11 => {
                let original = items_mut(&mut assembler)[5];
                items_mut(&mut assembler).insert(0, original);
            }
            12 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&label(1));
            }
            13 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .remove(&label(1));
            }
            _ => unreachable!(),
        }
        assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
        assert!(
            assembler
                .exception_owner_provenance
                .synchronous_compound_try_nops
                .is_empty(),
            "mutation {mutation}"
        );
        assert!(
            assembler
                .exception_owner_provenance
                .ownerless_compound_try_nops
                .is_empty(),
            "strict failure must not fall back"
        );
    }
}

#[test]
fn strict_compound_try_nop_late_replay_refuses_foreign_or_mutated_sources() {
    for mutation in 0..16 {
        let (mut assembler, emitted) = fixture();
        assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
        let origin = assembler
            .exception_owner_provenance
            .synchronous_compound_try_nops[0]
            .clone();
        complete(&mut assembler);
        match mutation {
            0 => {
                assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .clear();
            }
            1 => {
                assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .push((label(2), label(3)));
            }
            2 => {
                assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .push((label(1), label(4)));
            }
            3 => {
                assembler.borrow_unreachable_blocks.insert(label(1));
            }
            4 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&label(1));
            }
            5 => {
                items_mut(&mut assembler)[3] = Item::Label(label(4));
            }
            6 => {
                let Item::Instruction(raised) = &mut items_mut(&mut assembler)[2] else {
                    unreachable!()
                };
                raised.set_operand(Operand::Value(2));
            }
            7 => {
                let Item::Instruction(guard) = &mut items_mut(&mut assembler)[0] else {
                    unreachable!()
                };
                guard.location = location(9);
            }
            8 => {
                let Item::Instruction(jump) = &mut items_mut(&mut assembler)[5] else {
                    unreachable!()
                };
                jump.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            }
            9 => {
                items_mut(&mut assembler)[6] = Item::Label(label(1));
            }
            10 => {
                let original = items_mut(&mut assembler)[5];
                items_mut(&mut assembler).extend([Item::Label(label(4)), original]);
            }
            11 => {
                let original = items_mut(&mut assembler)[0];
                items_mut(&mut assembler).extend([Item::Label(label(4)), original]);
            }
            12 => {
                let original = items_mut(&mut assembler)[2];
                items_mut(&mut assembler).extend([Item::Label(label(4)), original]);
            }
            13 => {
                items_mut(&mut assembler).extend([
                    Item::Label(label(4)),
                    Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(label(2)),
                        location(7),
                        Some(0),
                    )),
                ]);
            }
            14 => {
                assembler.original_nop_targets.insert(label(2));
            }
            15 => {
                let guard = items_mut(&mut assembler)[0];
                items_mut(&mut assembler)[0] = Item::Instruction(Instruction::new(
                    NOP,
                    Operand::Value(0),
                    location(8),
                    Some(0),
                ));
                items_mut(&mut assembler).extend([Item::Label(label(4)), guard]);
            }
            _ => unreachable!(),
        }
        assert!(!matches(&assembler, &origin), "mutation {mutation}");
    }
}

#[test]
fn strict_compound_try_nop_refuses_stale_stream_and_prepared_recapture() {
    let (_, foreign) = fixture();
    let (mut assembler, emitted) = fixture();
    assembler.record_synchronous_compound_try_nop(label(2), label(3), foreign, true);
    assert!(
        assembler
            .exception_owner_provenance
            .synchronous_compound_try_nops
            .is_empty()
    );
    let graph = BlockGraph::from_items(assembler.linear_items().to_vec(), &FxHashSet::default());
    let reachable = graph.reachable_blocks(&[]);
    assembler.code = UnoptimizedCode::Prepared { graph, reachable };
    assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
    assert!(
        assembler
            .exception_owner_provenance
            .synchronous_compound_try_nops
            .is_empty()
    );
    assert!(
        assembler
            .exception_owner_provenance
            .ownerless_compound_try_nops
            .is_empty()
    );
}

#[test]
fn strict_compound_try_nop_changes_only_the_real_late_nop_owner() {
    for (mode, late) in [
        ("strict", false),
        ("strict", true),
        ("failed strict", true),
        ("late finally", true),
        ("missing summary", true),
        ("empty summary", true),
        ("legacy", false),
        ("legacy", true),
    ] {
        let (mut assembler, emitted) = fixture();
        assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
        let origin = assembler
            .exception_owner_provenance
            .synchronous_compound_try_nops[0]
            .clone();
        if mode == "failed strict" {
            assembler.borrow_block_aliases.insert(label(1));
        } else if mode == "legacy" {
            assembler
                .exception_owner_provenance
                .synchronous_compound_try_nops
                .clear();
            assembler
                .exception_owner_provenance
                .ownerless_compound_try_nops
                .push((label(2), label(3), origin.jump));
        }
        complete(&mut assembler);
        assembler.borrow_finally_entries.insert(label(3), None);
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.record_borrow_cleanup_calls();
        for entry in labels(&origin.head) {
            assert_eq!(
                pipeline.borrow_finally_jump_sources.get(&entry),
                Some(&true)
            );
        }
        if mode == "late finally" {
            pipeline.borrow_finally_jump_sources.insert(label(1), false);
        } else if mode == "missing summary" {
            pipeline.borrow_finally_jump_sources.remove(&label(1));
        } else if mode == "empty summary" {
            pipeline.borrow_finally_jump_sources.clear();
        }
        pipeline.remove_redundant_forward_jumps(late);
        let block = pipeline.graph.label_block(label(2)).unwrap();
        let Item::Instruction(nop) = pipeline.graph[block].items().last().unwrap() else {
            unreachable!()
        };
        let mut expected = origin.jump;
        expected.opcode = NOP;
        expected.set_operand(Operand::Value(0));
        if late && matches!(mode, "strict" | "legacy") {
            expected.set_exception_owner(super::super::NormalizedExceptionOwner::Absent);
        }
        assert_eq!(*nop, expected, "{mode}");
    }
}

fn add_foreign_role(assembler: &mut Assembler, mutation: usize) {
    let entry = label(1);
    let provenance = &mut assembler.exception_owner_provenance;
    match mutation {
        0 => {
            provenance.setups.push((entry, None));
        }
        1 => {
            provenance.ordinary_try_setups.insert(entry, label(7));
        }
        2 => {
            provenance.named_handler_cleanup_targets.insert(entry);
        }
        3 => {
            provenance.setup_nop_ranges.push((entry, label(7)));
        }
        4 => {
            provenance.original_fallthrough_setups.insert(entry);
        }
        5 => {
            provenance.return_region_reentries.insert(entry);
        }
        6 => {
            provenance.loop_finally_region_boundaries.insert(entry);
        }
        7 => {
            provenance.normal_try_copy_blocks.insert(entry);
        }
        8 => {
            provenance.borrow_only_block_boundaries.insert(entry);
        }
        9 => {
            provenance.omitted_nop_counts.insert(entry, 1);
        }
        10 => {
            provenance.continuation_boundaries.insert(entry);
        }
        11 => {
            provenance.fusion_boundaries.insert(entry);
        }
        12 => {
            assembler.borrow_finally_entries.insert(entry, None);
        }
        13 | 14 => {
            assembler.exception_regions.push(ExceptionRegion {
                is_scope_fallback: false,
                priority: super::super::ExceptionRegionPriority::OriginalRange(label(0), label(2)),
                start: label(0),
                end: label(2),
                target: if mutation == 13 { entry } else { label(3) },
                depth: 0,
                preserve_lasti: false,
            });
        }
        15 => {
            let Item::Instruction(jump) = &mut items_mut(assembler)[5] else {
                unreachable!()
            };
            jump.no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
        }
        16 => {
            let Item::Instruction(jump) = &mut items_mut(assembler)[5] else {
                unreachable!()
            };
            jump.add_pre_fold_extra_instructions(1);
        }
        17 => {
            let Item::Instruction(raised) = &mut items_mut(assembler)[2] else {
                unreachable!()
            };
            raised.local_policy.add_hidden_exception_owner_slots(1);
        }
        _ => unreachable!(),
    }
}

#[test]
fn strict_compound_try_nop_refuses_foreign_roles_before_and_after_capture() {
    for mutation in 0..18 {
        let (mut assembler, emitted) = fixture();
        add_foreign_role(&mut assembler, mutation);
        assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
        assert!(
            assembler
                .exception_owner_provenance
                .synchronous_compound_try_nops
                .is_empty(),
            "capture mutation {mutation}"
        );
        assert!(
            assembler
                .exception_owner_provenance
                .ownerless_compound_try_nops
                .is_empty()
        );

        let (mut assembler, emitted) = fixture();
        assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
        let origin = assembler
            .exception_owner_provenance
            .synchronous_compound_try_nops[0]
            .clone();
        complete(&mut assembler);
        add_foreign_role(&mut assembler, mutation);
        assert!(!matches(&assembler, &origin), "late mutation {mutation}");
    }
}

#[test]
fn strict_compound_try_nop_checks_every_current_target_alias_for_exception_entry() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for alias_index in [6, 7] {
            let (mut assembler, emitted) = fixture();
            assembler.python_version = version;
            assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
            let origin = assembler
                .exception_owner_provenance
                .synchronous_compound_try_nops[0]
                .clone();
            complete(&mut assembler);
            items_mut(&mut assembler).insert(alias_index, Item::Label(label(7)));
            assert!(matches(&assembler, &origin));
            assembler.exception_regions.push(ExceptionRegion {
                is_scope_fallback: false,
                priority: super::super::ExceptionRegionPriority::OriginalRange(label(0), label(2)),
                start: label(0),
                end: label(2),
                target: label(7),
                depth: 0,
                preserve_lasti: false,
            });
            assert!(!matches(&assembler, &origin), "{version:?} {alias_index}");
        }
    }
}

#[test]
fn strict_compound_try_nop_requires_the_real_reachable_summary_at_late_replay() {
    let (mut assembler, emitted) = fixture();
    assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, false);
    assert!(
        assembler
            .exception_owner_provenance
            .synchronous_compound_try_nops
            .is_empty()
    );
    assembler.record_synchronous_compound_try_nop(label(2), label(3), emitted, true);
    let mut origin = assembler
        .exception_owner_provenance
        .synchronous_compound_try_nops[0]
        .clone();
    complete(&mut assembler);
    assert!(!matches_with_sources(&assembler, &origin, None));
    for entries in [
        vec![],
        vec![(label(1), true)],
        vec![(label(1), false), (label(2), false)],
        vec![(label(1), true), (label(2), false)],
    ] {
        let sources = entries.into_iter().collect();
        assert!(!matches_with_sources(&assembler, &origin, Some(&sources)));
    }
    assert!(matches(&assembler, &origin));
    origin.normally_reachable = false;
    assert!(!matches(&assembler, &origin));
}
