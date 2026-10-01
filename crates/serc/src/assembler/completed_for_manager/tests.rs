use super::*;
use crate::assembler::ExceptionPolicy;
use crate::assembler::block_graph::BlockGraph;
use crate::target::opcodes::{NOP, POP_EXCEPT, RERAISE};

struct Join {
    assembler: Assembler,
    guard: EmittedJump,
    normal: EmittedJump,
    exit: EmittedJump,
    entry: Label,
    continuation: Label,
    condition: SourceLocation,
}

fn location(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 0, 4)
}

fn items_mut(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items
}

fn instruction_mut(assembler: &mut Assembler, emitted: EmittedJump) -> &mut Instruction {
    let Item::Instruction(value) =
        &mut assembler.linear_items_mut()[emitted.instruction().item_index as usize]
    else {
        unreachable!()
    };
    value
}

fn fixture(version: PythonVersion, skip: &str) -> Join {
    let mut a = Assembler::with_python_version(version);
    let entry = a.label();
    let guard_join = a.label();
    let normal_join = a.label();
    let continuation = a.label();
    let start = a.label();
    let normal_end = a.label();
    let handler = a.label();
    let inner_join = a.label();
    let condition = location(3);
    if skip != "capture" {
        a.select_completed_for_manager_guard(condition);
    }
    let slots = if version >= PythonVersion::Py315 {
        2
    } else {
        1
    };
    a.set_location(location(1));
    a.emit_forward_with_depth(FOR_ITER, entry, 3 + slots, ExceptionPolicy::default());
    a.set_location(location(2));
    a.emit_with_depth(NOP, 0, 3 + slots, ExceptionPolicy::default());
    a.record_exception_owner_block_boundary(entry);
    a.mark(entry);
    a.emit_with_depth(END_FOR, 0, 2 + slots, ExceptionPolicy::default());
    a.emit_with_depth(POP_ITER, 0, 2, ExceptionPolicy::default());
    a.set_location(condition);
    a.emit_with_depth(LOAD_FAST, 0, 3, ExceptionPolicy::default());
    let guard =
        a.emit_forward_with_depth(POP_JUMP_IF_NONE, guard_join, 2, ExceptionPolicy::default());
    a.emitted_jump_mut(guard, "test guard")
        .local_policy
        .add_hidden_exception_owner_slots(3);
    if skip != "guard" {
        a.record_source_none_condition();
    }
    a.mark(start);
    a.exception_owner_provenance
        .ordinary_try_setups
        .insert(start, handler);
    a.set_location(location(4));
    a.emit_forward_with_depth(POP_JUMP_IF_FALSE, inner_join, 2, ExceptionPolicy::default());
    a.emit_with_depth(NOP, 0, 2, ExceptionPolicy::default());
    a.record_exception_owner_block_boundary(inner_join);
    a.mark(inner_join);
    a.mark(normal_end);
    a.set_location(SourceLocation::NONE);
    let normal =
        a.emit_forward_with_depth(JUMP_FORWARD, normal_join, 2, ExceptionPolicy::default());
    a.record_normal_try_exit(normal_end, normal_join, None);
    if skip == "preexisting shared" {
        a.shared_jump_copy_location_blocks.insert(normal_end);
    }
    if skip != "shared" {
        a.preserve_shared_jump_no_location(normal);
    }
    if skip == "preexisting empty" {
        a.mark_borrow_empty_block_if_unretained(normal);
    }
    if skip != "incoming empty" {
        a.mark_completed_for_incoming_empty(normal);
    }
    a.add_exception_region(start, normal_end, handler, 2, false);
    if skip != "region" {
        a.seal_completed_for_incoming_region(start, normal_end, handler, 2);
    }
    a.mark(handler);
    a.emit_with_depth(PUSH_EXC_INFO, 0, 4, ExceptionPolicy::default());
    a.set_location(location(5));
    a.emit_with_depth(POP_EXCEPT, 0, 2, ExceptionPolicy::default());
    a.emit_forward_with_depth(JUMP_FORWARD, normal_join, 2, ExceptionPolicy::default());
    a.emit_with_depth(RERAISE, 1, 2, ExceptionPolicy::default());
    a.record_exception_owner_block_boundary(normal_join);
    a.record_exception_owner_block_boundary(guard_join);
    a.mark(normal_join);
    a.mark(guard_join);
    a.set_location(condition);
    let exit = a.emit_forward_with_depth(JUMP_FORWARD, continuation, 2, ExceptionPolicy::default());
    if skip != "exit empty" {
        a.mark_borrow_empty_block(exit);
    }
    if skip != "retention" {
        a.preserve_redundant_jump_nop_after_threading(exit);
    }
    Join {
        assembler: a,
        guard,
        normal,
        exit,
        entry,
        continuation,
        condition,
    }
}

fn accepts(join: &Join) -> bool {
    join.assembler.original_completed_for_manager_join(
        join.exit,
        join.continuation,
        join.condition,
        0,
    )
}

#[test]
fn completed_for_borrow_route_tracks_only_the_real_policy_writer() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let mut join = fixture(version, "none");
        assert!(accepts(&join));
        join.assembler
            .prevent_finalizer_borrow_jump_threading_target(join.exit);
        assert!(accepts(&join));
        assert!(
            !instruction_mut(&mut join.assembler, join.exit)
                .jump_policy
                .allows_borrow_threading_target()
        );
        assert!(
            join.assembler
                .capture_completed_for_borrow_route(join.exit)
                .is_none()
        );

        let mut join = fixture(version, "none");
        join.assembler
            .prevent_borrow_jump_threading_target(join.exit);
        assert!(!accepts(&join));
    }
}

#[test]
fn completed_for_borrow_route_rejects_changed_or_detached_records() {
    for mode in 0..15 {
        let mut join = fixture(PythonVersion::Py314, "none");
        let original = join.assembler.capture_completed_for_borrow_route(join.exit);
        assert!(original.is_some());
        if mode != 0 {
            join.assembler
                .prevent_borrow_jump_threading_target(join.exit);
        }
        match mode {
            0 => {}
            1 => instruction_mut(&mut join.assembler, join.exit).location = location(30),
            2 => instruction_mut(&mut join.assembler, join.exit)
                .set_operand(Operand::Forward(join.entry)),
            3 => instruction_mut(&mut join.assembler, join.exit)
                .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
            4 => {
                join.assembler.completed_for_manager.exits.clear();
            }
            5 => {
                join.assembler.completed_for_manager.exits[0].retained = false;
            }
            6 => {
                join.assembler.completed_for_manager.exits[0].record.begin += 1;
            }
            7 => {
                let record = join.assembler.completed_for_manager.exits[0].record.clone();
                join.assembler.completed_for_manager.exits.push(Exit {
                    record,
                    retained: true,
                });
            }
            8 => {
                let last = *join.assembler.linear_items().last().unwrap();
                items_mut(&mut join.assembler).push(last);
            }
            9 => {
                items_mut(&mut join.assembler).insert(
                    0,
                    Item::Instruction(Instruction::new(
                        NOP,
                        Operand::Value(0),
                        location(20),
                        Some(0),
                    )),
                );
            }
            10 => {
                let label = join.assembler.completed_for_manager.exits[0].record.heads[0].label;
                join.assembler.borrow_block_aliases.insert(label);
            }
            11 => {
                let label = join.assembler.completed_for_manager.exits[0].record.heads[0].label;
                assert!(
                    join.assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&label)
                );
            }
            12 => {
                let token = join.assembler.capture_completed_for_borrow_route(join.exit);
                assert!(token.is_none());
                let current = *instruction_mut(&mut join.assembler, join.exit);
                join.assembler.completed_for_manager.exits[0]
                    .record
                    .set_instruction(current);
            }
            13 => {
                join.assembler.emission_stream = crate::assembler::ir::EmissionStreamId::next();
            }
            14 => {
                let label = join.assembler.completed_for_manager.exits[0].record.heads[0].label;
                join.assembler
                    .add_exception_region(join.entry, join.continuation, label, 0, false);
            }
            _ => unreachable!(),
        }
        let before = join
            .assembler
            .completed_for_manager
            .exits
            .iter()
            .map(|origin| (origin.record.clone(), origin.retained))
            .collect::<Vec<_>>();
        join.assembler.seal_completed_for_borrow_route(original);
        let after = join
            .assembler
            .completed_for_manager
            .exits
            .iter()
            .map(|origin| (origin.record.clone(), origin.retained))
            .collect::<Vec<_>>();
        assert_eq!(before, after, "mode {mode}");
        if mode != 0 && mode != 12 {
            assert!(!accepts(&join), "mode {mode}");
        }
    }
}

#[test]
fn only_real_ordered_writers_authorize_the_completed_for_join() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for skip in [
            "none",
            "capture",
            "guard",
            "shared",
            "incoming empty",
            "region",
            "exit empty",
            "retention",
            "preexisting shared",
            "preexisting empty",
        ] {
            let mut join = fixture(version, skip);
            assert_eq!(
                accepts(&join),
                version >= PythonVersion::Py314 && skip == "none",
                "{version:?} {skip}"
            );
            assert!(!join.assembler.original_normal_finally_manager_join(
                join.exit,
                join.continuation,
                join.condition,
                0
            ));
            assert!(!join.assembler.original_generator_manager_recovery_join(
                join.exit,
                join.continuation,
                join.condition,
                0
            ));
        }
    }
}

fn emit_decoy(assembler: &mut Assembler, condition: SourceLocation) {
    let entry = assembler.label();
    let exit = assembler.label();
    let slots = if assembler.python_version >= PythonVersion::Py315 {
        2
    } else {
        1
    };
    assembler.set_location(condition);
    assembler.emit_forward_with_depth(FOR_ITER, entry, 3 + slots, ExceptionPolicy::default());
    assembler.record_exception_owner_block_boundary(entry);
    assembler.mark(entry);
    assembler.emit_with_depth(END_FOR, 0, 2 + slots, ExceptionPolicy::default());
    assembler.emit_with_depth(POP_ITER, 0, 2, ExceptionPolicy::default());
    assembler.emit_with_depth(LOAD_FAST, 0, 3, ExceptionPolicy::default());
    let guard =
        assembler.emit_forward_with_depth(POP_JUMP_IF_NONE, exit, 2, ExceptionPolicy::default());
    assembler
        .emitted_jump_mut(guard, "decoy guard")
        .local_policy
        .add_hidden_exception_owner_slots(3);
    assembler.record_source_none_condition();
    assembler.mark(exit);
}

#[test]
fn capture_is_selected_bounded_and_restored() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let mut assembler = Assembler::with_python_version(version);
        for line in 1..65 {
            emit_decoy(&mut assembler, location(line));
        }
        assert!(assembler.completed_for_manager.guards.is_empty());
        assert!(assembler.completed_for_manager.incoming.is_empty());
        assert!(assembler.completed_for_manager.exits.is_empty());

        let selected = location(200);
        assembler.select_completed_for_manager_guard(selected);
        for line in 65..129 {
            emit_decoy(&mut assembler, location(line));
        }
        assert!(assembler.completed_for_manager.guards.is_empty());
        emit_decoy(&mut assembler, selected);
        assert_eq!(assembler.completed_for_manager.guards.len(), 1);
        let prefix_len = assembler.completed_for_manager.guards[0].prefix.len();
        emit_decoy(&mut assembler, selected);
        assert_eq!(assembler.completed_for_manager.guards.len(), 1);
        assert_eq!(
            assembler.completed_for_manager.guards[0].prefix.len(),
            prefix_len
        );

        let previous = assembler.take_completed_for_manager_capture();
        emit_decoy(&mut assembler, selected);
        assert!(assembler.completed_for_manager.guards.is_empty());
        assembler.select_completed_for_manager_guard(location(400));
        emit_decoy(&mut assembler, location(401));
        assert!(assembler.completed_for_manager.guards.is_empty());
        assembler.restore_completed_for_manager_capture(previous);
        assert_eq!(
            assembler.completed_for_manager.selected_condition,
            Some(selected)
        );
        assert_eq!(assembler.completed_for_manager.guards.len(), 1);
        assert_eq!(
            assembler.completed_for_manager.guards[0].prefix.len(),
            prefix_len
        );

        assembler.retire_completed_for_manager_capture();
        emit_decoy(&mut assembler, selected);
        assert!(assembler.completed_for_manager.selected_condition.is_none());
        assert!(assembler.completed_for_manager.guards.is_empty());
        assert!(assembler.completed_for_manager.incoming.is_empty());
        assert!(assembler.completed_for_manager.exits.is_empty());

        let mut join = fixture(version, "none");
        assert!(accepts(&join));
        let previous = join.assembler.take_completed_for_manager_capture();
        assert!(!accepts(&join));
        join.assembler
            .restore_completed_for_manager_capture(previous);
        assert!(accepts(&join));
        join.assembler.retire_completed_for_manager_capture();
        assert!(!accepts(&join));
    }
}

#[test]
fn every_original_record_and_physical_source_remains_bound() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for which in ["guard", "normal", "exit"] {
            for mutation in [
                "location",
                "owner",
                "hidden",
                "folded",
                "operand",
                "default",
                "shared",
                "borrow",
                "move",
                "equal replacement",
                "depth",
            ] {
                let mut join = fixture(version, "none");
                assert!(accepts(&join));
                let emitted = match which {
                    "guard" => join.guard,
                    "normal" => join.normal,
                    _ => join.exit,
                };
                let original = *instruction_mut(&mut join.assembler, emitted);
                let instruction = instruction_mut(&mut join.assembler, emitted);
                match mutation {
                    "location" => instruction.location = location(20),
                    "owner" => instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    "hidden" => instruction.local_policy.add_hidden_exception_owner_slots(1),
                    "folded" => instruction.add_pre_fold_extra_instructions(1),
                    "operand" => instruction.set_operand(Operand::Value(0)),
                    "default" => {
                        *instruction = Instruction::new(
                            NOP,
                            Operand::Value(0),
                            original.location,
                            original.depth_after(),
                        );
                    }
                    "shared" => instruction.no_location_policy.preserve_location(),
                    "borrow" => instruction.jump_policy.prevent_borrow_threading_target(),
                    "move" => {
                        *instruction =
                            Instruction::new(NOP, Operand::Value(0), location(30), Some(2));
                        items_mut(&mut join.assembler).push(Item::Instruction(original));
                    }
                    "equal replacement" => {
                        items_mut(&mut join.assembler).push(Item::Instruction(original));
                    }
                    "depth" => {
                        let mut value = Instruction::new(
                            original.opcode,
                            original.operand(),
                            original.location,
                            Some(99),
                        );
                        value.local_policy = original.local_policy;
                        value.no_location_policy = original.no_location_policy;
                        value.jump_policy = original.jump_policy;
                        value.fusion_policy = original.fusion_policy;
                        *instruction = value;
                    }
                    _ => unreachable!(),
                }
                if mutation == "shared" && which == "normal" {
                    continue;
                }
                assert!(!accepts(&join), "{version:?} {which} {mutation}");
            }
        }
    }
}

#[test]
fn complete_heads_incoming_edges_and_original_registries_are_required() {
    for mutation in [
        "guard boundary",
        "borrow",
        "omitted",
        "duplicate label",
        "foreign entry",
        "future source",
        "future alias",
        "missing origin",
        "legacy origin",
        "duplicate pair",
        "missing pair",
        "prefix",
        "missing strict",
        "duplicate strict",
    ] {
        let mut join = fixture(PythonVersion::Py314, "none");
        assert!(accepts(&join));
        let a = &mut join.assembler;
        match mutation {
            "guard boundary" => {
                a.exception_owner_provenance
                    .block_boundaries
                    .remove(&join.entry);
            }
            "borrow" => {
                a.borrow_unreachable_blocks.insert(join.entry);
            }
            "omitted" => {
                a.exception_owner_provenance
                    .omitted_nop_counts
                    .insert(join.entry, 1);
            }
            "duplicate label" => items_mut(a).push(Item::Label(join.entry)),
            "foreign entry" => items_mut(a).push(Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(join.entry),
                location(40),
                Some(2),
            ))),
            "future source" => a.record_exception_owner_block_boundary(join.continuation),
            "future alias" => a.mark(join.continuation),
            "missing origin" => a.exception_owner_provenance.source_none_conditions.clear(),
            "legacy origin" => {
                a.exception_owner_provenance.source_none_conditions[0]
                    .instruction
                    .location = location(50);
            }
            "duplicate pair" => {
                let pair = a.exception_owner_provenance.normal_try_exits[0];
                a.exception_owner_provenance.normal_try_exits.push(pair);
            }
            "missing pair" => a.exception_owner_provenance.normal_try_exits.clear(),
            "prefix" => {
                a.linear_items_mut()[1] = Item::Instruction(Instruction::new(
                    NOP,
                    Operand::Value(0),
                    location(60),
                    Some(2),
                ));
            }
            "missing strict" => a.completed_for_manager.guards.clear(),
            "duplicate strict" => {
                let original = &a.completed_for_manager.guards[0];
                let duplicate = Guard {
                    record: original.record.clone(),
                    prefix: original.prefix.clone(),
                };
                a.completed_for_manager.guards.push(duplicate);
            }
            _ => unreachable!(),
        }
        assert!(!accepts(&join), "{mutation}");
    }
}

#[test]
fn complete_region_and_exceptional_entry_identity_are_required() {
    for mutation in [
        "depth",
        "lasti",
        "fallback",
        "priority",
        "start",
        "end",
        "target",
        "duplicate",
        "handler instruction",
        "handler alias",
        "setup target",
    ] {
        let mut join = fixture(PythonVersion::Py314, "none");
        assert!(accepts(&join));
        let a = &mut join.assembler;
        let region = a.exception_regions[0];
        match mutation {
            "depth" => a.exception_regions[0].depth += 1,
            "lasti" => a.exception_regions[0].preserve_lasti = true,
            "fallback" => a.exception_regions[0].is_scope_fallback = true,
            "priority" => {
                a.exception_regions[0].priority =
                    ExceptionRegionPriority::Resolved { start: 0, end: 1 }
            }
            "start" => a.exception_regions[0].start = join.entry,
            "end" => a.exception_regions[0].end = join.entry,
            "target" => a.exception_regions[0].target = join.entry,
            "duplicate" => a.exception_regions.push(region),
            "handler instruction" => {
                let index = a.completed_for_manager.incoming[0].handler_item.unwrap();
                a.linear_items_mut()[index + 1] = Item::Instruction(Instruction::new(
                    NOP,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    Some(4),
                ));
            }
            "handler alias" => {
                let position = a.completed_for_manager.incoming[0].handler_item.unwrap();
                let label = a.label();
                items_mut(a).insert(position, Item::Label(label));
            }
            "setup target" => {
                a.exception_owner_provenance
                    .ordinary_try_setups
                    .insert(region.start, join.entry);
            }
            _ => unreachable!(),
        }
        assert!(!accepts(&join), "{mutation}");
    }
}

#[test]
fn prepared_stale_and_foreign_continuations_fail_closed() {
    for mutation in [
        "prepared",
        "foreign stream",
        "stale exit",
        "outside interval",
        "future borrow",
        "future exceptional",
        "future shared",
        "future folded",
        "future omitted",
        "future preserved",
    ] {
        let mut join = fixture(PythonVersion::Py314, "none");
        assert!(accepts(&join));
        match mutation {
            "prepared" => {
                join.assembler.mark(join.continuation);
                let graph = BlockGraph::from_items(
                    join.assembler.linear_items().to_vec(),
                    &super::super::FxHashSet::default(),
                );
                let reachable = vec![true; graph.len()];
                join.assembler.code = UnoptimizedCode::Prepared { graph, reachable };
            }
            "foreign stream" => join.exit = fixture(PythonVersion::Py314, "none").exit,
            "stale exit" => join.exit = join.normal,
            "outside interval" => {
                assert!(!join.assembler.original_completed_for_manager_join(
                    join.exit,
                    join.continuation,
                    join.condition,
                    usize::MAX
                ));
                continue;
            }
            "future borrow" => {
                join.assembler
                    .borrow_jump_entry_labels
                    .insert(join.continuation);
            }
            "future exceptional" => {
                let mut region = join.assembler.exception_regions[0];
                region.target = join.continuation;
                join.assembler.exception_regions.push(region);
            }
            "future shared" => {
                join.assembler
                    .shared_jump_copy_location_blocks
                    .insert(join.continuation);
            }
            "future folded" => {
                join.assembler
                    .folded_away_condition_targets
                    .insert(join.continuation);
            }
            "future omitted" => {
                join.assembler
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(join.continuation, 1);
            }
            "future preserved" => {
                join.assembler
                    .preserved_block_boundaries
                    .insert(join.continuation);
            }
            _ => unreachable!(),
        }
        assert!(!accepts(&join), "{mutation}");
    }
}
