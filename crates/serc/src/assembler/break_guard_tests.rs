use super::ir::{
    BorrowFallthrough, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, JumpPolicy,
    UnoptimizedCode,
};
use super::{Assembler, Instruction, Item, Operand, SourceLocation};
use crate::PythonVersion;
use crate::target::opcodes::*;

fn insert_item(assembler: &mut Assembler, index: usize, item: Item) {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items.insert(index, item);
}

fn guard_mut(assembler: &mut Assembler) -> &mut Instruction {
    let Item::Instruction(guard) = &mut assembler.linear_items_mut()[0] else {
        unreachable!()
    };
    guard
}

#[test]
fn shared_while_break_joins_require_exact_edges_and_original_alias_roles() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for depth in [0, 2] {
            for mutation in [
                "intact",
                "single condition",
                "foreign donor",
                "different anchor",
                "backward condition",
                "condition depth",
                "condition owner",
                "condition hidden slots",
                "condition compacted slots",
                "condition unlocated",
                "break depth",
                "break owner",
                "break hidden slots",
                "break compacted slots",
                "break unlocated",
                "plain break",
                "threadable break",
                "unretained break",
                "duplicate break",
                "located fallthrough",
                "duplicate alias",
                "missing source",
                "missing block",
                "end source",
                "end block",
                "authored alias",
                "folded alias",
                "folded else",
                "folded exit",
                "omitted alias",
                "borrow alias",
                "fusion alias",
                "continuation alias",
                "preserved alias",
                "shared alias",
                "deferred alias",
                "setup alias",
                "handler alias",
            ] {
                let mut assembler = Assembler::with_python_version(version);
                let anchor = assembler.label();
                let end = assembler.label();
                let alias = assembler.label();
                let condition = SourceLocation::new(3, 3, 10, 15);
                let location = SourceLocation::new(8, 8, 8, 13);
                assembler.set_location(condition);
                assembler.emit_forward_with_depth(
                    POP_JUMP_IF_FALSE,
                    anchor,
                    depth,
                    ExceptionPolicy::default(),
                );
                if mutation != "single condition" {
                    assembler.emit_forward_with_depth(
                        POP_JUMP_IF_TRUE,
                        anchor,
                        depth,
                        ExceptionPolicy::default(),
                    );
                }
                if mutation == "foreign donor" {
                    assembler.emit_forward_with_depth(
                        JUMP_FORWARD,
                        anchor,
                        depth,
                        ExceptionPolicy::default(),
                    );
                }
                assembler.set_location(location);
                let emitted = assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    end,
                    depth,
                    ExceptionPolicy::default(),
                );
                assembler.mark_jump_as_authored_break(emitted);
                assembler.prevent_jump_threading_target(emitted);
                assembler.preserve_direct_inlined_jump_nop(emitted);
                let original = *assembler.emitted_jump_mut(emitted, "test");
                match mutation {
                    "backward condition" => {
                        guard_mut(&mut assembler).set_operand(Operand::Backward(anchor));
                    }
                    "condition depth" => {
                        *guard_mut(&mut assembler) = Instruction::new(
                            POP_JUMP_IF_FALSE,
                            Operand::Forward(anchor),
                            condition,
                            Some(depth + 1),
                        );
                    }
                    "condition owner" => guard_mut(&mut assembler)
                        .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    "condition hidden slots" => guard_mut(&mut assembler)
                        .local_policy
                        .add_hidden_exception_owner_slots(1),
                    "condition compacted slots" => {
                        guard_mut(&mut assembler).add_pre_fold_extra_instructions(1);
                    }
                    "condition unlocated" => {
                        guard_mut(&mut assembler).location = SourceLocation::NONE;
                    }
                    "different anchor" => {
                        guard_mut(&mut assembler).set_operand(Operand::Forward(alias));
                    }
                    "break depth" => {
                        let mut changed = Instruction::new(
                            JUMP_FORWARD,
                            Operand::Forward(end),
                            location,
                            Some(depth + 1),
                        );
                        changed.fusion_policy.mark_authored_break();
                        changed.jump_policy = original.jump_policy;
                        *assembler.emitted_jump_mut(emitted, "test") = changed;
                    }
                    "break owner" => assembler
                        .emitted_jump_mut(emitted, "test")
                        .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    "break hidden slots" => assembler
                        .emitted_jump_mut(emitted, "test")
                        .local_policy
                        .add_hidden_exception_owner_slots(1),
                    "break compacted slots" => assembler
                        .emitted_jump_mut(emitted, "test")
                        .add_pre_fold_extra_instructions(1),
                    "break unlocated" => {
                        assembler.emitted_jump_mut(emitted, "test").location = SourceLocation::NONE;
                    }
                    "plain break" => {
                        *assembler.emitted_jump_mut(emitted, "test") = Instruction::new(
                            JUMP_FORWARD,
                            Operand::Forward(end),
                            location,
                            Some(depth),
                        );
                    }
                    "threadable break" => {
                        assembler.emitted_jump_mut(emitted, "test").jump_policy =
                            JumpPolicy::default();
                    }
                    "unretained break" => {
                        let mut policy = JumpPolicy::default();
                        policy.prevent_threading_target();
                        assembler.emitted_jump_mut(emitted, "test").jump_policy = policy;
                    }
                    "duplicate break" => {
                        insert_item(&mut assembler, 0, Item::Instruction(original));
                    }
                    "located fallthrough" => {
                        assembler.emit_with_depth(NOP, 0, depth, ExceptionPolicy::default());
                    }
                    _ => {}
                }
                assembler.mark(anchor);
                assembler.record_exception_owner_block_boundary(anchor);
                assembler.mark(end);
                assembler.mark(alias);
                match mutation {
                    "duplicate alias" => assembler.mark(alias),
                    "missing source" => {
                        assembler
                            .exception_owner_provenance
                            .source_boundaries
                            .remove(&anchor);
                    }
                    "missing block" => {
                        assembler
                            .exception_owner_provenance
                            .block_boundaries
                            .remove(&anchor);
                    }
                    "end source" => {
                        assembler
                            .exception_owner_provenance
                            .source_boundaries
                            .insert(end);
                    }
                    "end block" => {
                        assembler
                            .exception_owner_provenance
                            .block_boundaries
                            .insert(end);
                    }
                    "different anchor" => assembler.record_exception_owner_block_boundary(alias),
                    "authored alias" => {
                        assembler.original_nop_targets.insert(alias);
                    }
                    "folded alias" => {
                        assembler.folded_away_condition_targets.insert(alias);
                    }
                    "folded else" => {
                        assembler.folded_away_loop_else_targets.insert(alias);
                    }
                    "folded exit" => {
                        assembler.folded_condition_exit_targets.insert(alias);
                    }
                    "omitted alias" => {
                        assembler
                            .exception_owner_provenance
                            .omitted_nop_counts
                            .insert(alias, 1);
                    }
                    "borrow alias" => {
                        assembler.borrow_block_aliases.insert(alias);
                    }
                    "fusion alias" => {
                        assembler
                            .exception_owner_provenance
                            .fusion_boundaries
                            .insert(alias);
                    }
                    "continuation alias" => {
                        assembler
                            .exception_owner_provenance
                            .continuation_boundaries
                            .insert(alias);
                    }
                    "preserved alias" => {
                        assembler.preserved_block_boundaries.insert(alias);
                    }
                    "shared alias" => {
                        assembler.shared_jump_copy_location_blocks.insert(alias);
                    }
                    "deferred alias" => {
                        assembler.deferred_shared_jump_blocks.insert(alias);
                    }
                    "setup alias" => assembler.record_exception_setup(alias, false),
                    "handler alias" => {
                        assembler.add_exception_region(anchor, end, alias, depth, false);
                    }
                    _ => {}
                }
                assert_eq!(
                    assembler.at_original_shared_while_break_join(),
                    version >= PythonVersion::Py314 && mutation == "intact",
                    "{version:?} depth={depth}: {mutation}"
                );
            }
        }
    }
}

#[test]
fn copied_finalizer_break_location_requires_the_exact_original_call_tail() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for depth in [0, 3] {
            for mutation in [
                "intact",
                "incoming boundary",
                "incoming entry",
                "backward incoming",
                "source boundary",
                "block boundary",
                "borrow alias",
                "preserved alias",
                "authored alias",
                "fusion alias",
                "continuation alias",
                "setup alias",
                "handler alias",
                "omitted slots",
                "omitted location",
                "shared alias",
                "deferred alias",
                "threaded alias",
                "folded alias",
                "folded exit",
                "unreachable alias",
                "omitted backedge",
                "borrow entry",
                "repeated alias",
                "third alias",
                "bound target",
                "interior label",
                "trailing label",
                "wrong unwind start",
                "wrong target",
                "unlocated donor",
                "foreign donor",
                "receiver opcode",
                "receiver line",
                "receiver column",
                "receiver width",
                "method opcode",
                "method argument",
                "method line",
                "method width",
                "call opcode",
                "call argument",
                "call location",
                "call depth",
                "call owner",
                "call slots",
                "call hidden slots",
                "call next location",
                "call barrier",
                "call early admission",
                "discard opcode",
                "discard location",
                "discard depth",
                "iterator opcode",
                "iterator argument",
                "iterator location",
                "iterator depth",
                "jump location",
                "jump depth",
                "jump owner",
                "jump slots",
                "jump retained",
                "jump threadable",
                "jump unauthored",
                "jump backward",
                "wrong iterator slots",
                "extra iterator opcode",
                "extra iterator depth",
                "extra iterator location",
                "extra iterator policy",
            ] {
                if version != PythonVersion::Py315 && mutation.starts_with("extra iterator ") {
                    continue;
                }
                let slots =
                    if (version == PythonVersion::Py315) != (mutation == "wrong iterator slots") {
                        2
                    } else {
                        1
                    };
                let mut assembler = Assembler::with_python_version(version);
                let boundary = assembler.label();
                let entry = assembler.label();
                let target = assembler.label();
                let other = assembler.label();
                let location = SourceLocation::new(10, 10, 8, 24);
                let receiver_location = SourceLocation::new(10, 10, 8, 16);
                let method_location = SourceLocation::new(10, 10, 8, 22);
                if mutation == "repeated alias" {
                    assembler.mark(entry);
                } else if mutation == "bound target" {
                    assembler.mark(target);
                }
                assembler.set_location(SourceLocation::new(4, 4, 8, 13));
                if matches!(
                    mutation,
                    "incoming boundary" | "incoming entry" | "backward incoming"
                ) {
                    let incoming = if mutation == "incoming boundary" {
                        boundary
                    } else {
                        entry
                    };
                    if mutation == "backward incoming" {
                        assembler.emit_backward(POP_JUMP_IF_FALSE, incoming);
                    } else {
                        assembler.emit_forward_with_depth(
                            POP_JUMP_IF_FALSE,
                            incoming,
                            depth + 1,
                            ExceptionPolicy::default(),
                        );
                    }
                }
                assembler.emit_with_depth(NOP, 0, depth + 1, ExceptionPolicy::default());
                if mutation == "third alias" {
                    assembler.mark(other);
                }
                assembler.mark(boundary);
                let start = assembler.item_count();
                assembler.mark(entry);
                let first = assembler.item_count();
                assembler.set_location(receiver_location);
                assembler.emit_with_depth(
                    LOAD_FAST,
                    2,
                    depth + slots + 1,
                    ExceptionPolicy::default(),
                );
                assembler.set_location(method_location);
                assembler.emit_with_depth(
                    LOAD_ATTR,
                    3,
                    depth + slots + 2,
                    ExceptionPolicy::default(),
                );
                assembler.set_location(location);
                assembler.emit_with_depth(CALL, 0, depth + slots + 1, ExceptionPolicy::default());
                assembler.emit_with_depth(POP_TOP, 0, depth + slots, ExceptionPolicy::default());
                assembler.set_location(SourceLocation::NONE);
                if slots == 2 {
                    assembler.emit_with_depth(POP_TOP, 0, depth + 1, ExceptionPolicy::default());
                }
                assembler.emit_with_depth(POP_TOP, 0, depth, ExceptionPolicy::default());
                let jump = assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    target,
                    depth,
                    ExceptionPolicy::default(),
                );
                assembler.prevent_jump_threading_target(jump);
                assembler.mark_jump_as_authored_break(jump);
                match mutation {
                    "source boundary" => {
                        assembler
                            .exception_owner_provenance
                            .source_boundaries
                            .insert(entry);
                    }
                    "block boundary" => {
                        assembler
                            .exception_owner_provenance
                            .block_boundaries
                            .insert(boundary);
                    }
                    "borrow alias" => {
                        assembler.borrow_block_aliases.insert(entry);
                    }
                    "preserved alias" => {
                        assembler.preserved_block_boundaries.insert(boundary);
                    }
                    "authored alias" => {
                        assembler.original_nop_targets.insert(entry);
                    }
                    "fusion alias" => {
                        assembler
                            .exception_owner_provenance
                            .fusion_boundaries
                            .insert(entry);
                    }
                    "continuation alias" => {
                        assembler
                            .exception_owner_provenance
                            .continuation_boundaries
                            .insert(boundary);
                    }
                    "setup alias" => {
                        assembler
                            .exception_owner_provenance
                            .setups
                            .push((entry, None));
                    }
                    "handler alias" => {
                        assembler.exception_regions.push(ExceptionRegion {
                            start: boundary,
                            end: target,
                            target: entry,
                            depth,
                            preserve_lasti: false,
                            is_scope_fallback: false,
                            priority: ExceptionRegionPriority::OriginalRange(boundary, target),
                        });
                    }
                    "omitted slots" => {
                        assembler
                            .exception_owner_provenance
                            .omitted_nop_counts
                            .insert(entry, 1);
                    }
                    "omitted location" => {
                        assembler
                            .exception_owner_provenance
                            .omitted_nop_locations
                            .insert(entry, location);
                    }
                    "shared alias" => {
                        assembler.shared_jump_copy_location_blocks.insert(entry);
                    }
                    "deferred alias" => {
                        assembler.deferred_shared_jump_blocks.insert(boundary);
                    }
                    "threaded alias" => {
                        assembler
                            .conditional_threading_targets
                            .insert(entry, SourceLocation::NONE);
                    }
                    "folded alias" => {
                        assembler.folded_away_condition_targets.insert(entry);
                    }
                    "folded exit" => {
                        assembler.folded_condition_exit_targets.insert(boundary);
                    }
                    "unreachable alias" => {
                        assembler.borrow_unreachable_blocks.insert(entry);
                    }
                    "omitted backedge" => {
                        assembler.borrow_omitted_for_backedges.insert(boundary);
                    }
                    "borrow entry" => {
                        assembler.borrow_jump_entry_labels.insert(entry);
                    }
                    "interior label" => {
                        assembler.linear_items_mut()[first + 1] = Item::Label(other);
                    }
                    "trailing label" => assembler.mark(other),
                    _ => {}
                }
                let record = if mutation.starts_with("receiver ") {
                    Some(0)
                } else if mutation.starts_with("method ") {
                    Some(1)
                } else if mutation.starts_with("call ") {
                    Some(2)
                } else if mutation.starts_with("discard ") {
                    Some(3)
                } else if mutation.starts_with("iterator ") {
                    Some(3 + slots as usize)
                } else if mutation.starts_with("extra iterator ") {
                    Some(4)
                } else if mutation.starts_with("jump ") {
                    Some(4 + slots as usize)
                } else {
                    None
                };
                if let Some(record) = record {
                    let Item::Instruction(instruction) =
                        &mut assembler.linear_items_mut()[first + record]
                    else {
                        unreachable!()
                    };
                    match mutation {
                        "receiver opcode"
                        | "method opcode"
                        | "call opcode"
                        | "discard opcode"
                        | "iterator opcode"
                        | "extra iterator opcode" => instruction.opcode = NOP,
                        "receiver line" | "method line" => instruction.location.line += 1,
                        "receiver column" => instruction.location.column += 1,
                        "receiver width" => instruction.location.end_column = 23,
                        "method width" => instruction.location.end_column -= 1,
                        "method argument" => instruction.set_operand(Operand::Value(2)),
                        "call argument" | "iterator argument" => {
                            instruction.set_operand(Operand::Value(1));
                        }
                        "call location" | "discard location" => {
                            instruction.location = SourceLocation::NONE;
                        }
                        "iterator location" | "jump location" | "extra iterator location" => {
                            instruction.location = location;
                        }
                        "call depth"
                        | "discard depth"
                        | "iterator depth"
                        | "jump depth"
                        | "extra iterator depth" => {
                            let mut changed = Instruction::new(
                                instruction.opcode,
                                instruction.operand(),
                                instruction.location,
                                instruction.depth_after().map(|depth| depth + 1),
                            );
                            changed.fusion_policy = instruction.fusion_policy;
                            changed.jump_policy = instruction.jump_policy;
                            *instruction = changed;
                        }
                        "call owner" | "jump owner" => {
                            instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                        }
                        "call slots" | "jump slots" => {
                            instruction.add_pre_fold_extra_instructions(1);
                        }
                        "call hidden slots" => {
                            instruction.local_policy.add_hidden_exception_owner_slots(1);
                        }
                        "call next location" => instruction.fusion_policy.mark_location_from_next(),
                        "call barrier" => instruction
                            .no_location_policy
                            .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                        "call early admission" => instruction.mark_original_annotation_jump(),
                        "jump retained" => instruction.jump_policy.retain_nop(),
                        "extra iterator policy" => instruction.jump_policy.retain_nop(),
                        "jump threadable" => instruction.jump_policy = JumpPolicy::default(),
                        "jump unauthored" => {
                            let mut changed = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(target),
                                SourceLocation::NONE,
                                Some(depth),
                            );
                            changed.jump_policy.prevent_threading_target();
                            *instruction = changed;
                        }
                        "jump backward" => instruction.set_operand(Operand::Backward(target)),
                        _ => unreachable!(),
                    }
                }
                let before = assembler.linear_items().to_vec();
                let accepted = assembler.locate_original_copied_finalizer_break(
                    jump,
                    if mutation == "wrong target" {
                        other
                    } else {
                        target
                    },
                    if mutation == "wrong unwind start" {
                        start + 1
                    } else {
                        start
                    },
                    match mutation {
                        "unlocated donor" => SourceLocation::NONE,
                        "foreign donor" => SourceLocation::new(9, 9, 8, 24),
                        _ => location,
                    },
                );
                assert_eq!(
                    accepted,
                    version >= PythonVersion::Py314 && mutation == "intact",
                    "{version:?} depth={depth}: {mutation}"
                );
                let mut expected = before;
                if accepted {
                    let Item::Instruction(last) = expected.last_mut().unwrap() else {
                        unreachable!()
                    };
                    last.location = location;
                }
                assert_eq!(assembler.linear_items(), expected, "{mutation}");
                assert_eq!(assembler.location(), SourceLocation::NONE);
            }
        }
    }
}

#[test]
fn break_guard_donors_require_the_exact_original_loop_frontier() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for slots in [
            0,
            if version == PythonVersion::Py315 {
                2
            } else {
                1
            },
        ] {
            for outer_depth in [0, 2] {
                for mutation in [
                    "intact",
                    "wrong loop target",
                    "backward break",
                    "unauthored break",
                    "threadable break",
                    "retained break",
                    "break depth",
                    "break location",
                    "break owner",
                    "break hidden slots",
                    "duplicate break",
                    "nop location",
                    "nop policy",
                    "nop opcode",
                    "missing nop",
                    "extra pop",
                    "pop location",
                    "pop operand",
                    "pop opcode",
                    "pop depth",
                    "pop policy",
                    "wrong slot count",
                    "interior label",
                    "guard location",
                    "guard depth",
                    "backward guard",
                    "guard policy",
                    "guard hidden slots",
                    "guard owner",
                    "second guard",
                    "same span outer guard",
                    "duplicate join",
                    "foreign alias",
                    "repeated join",
                    "missing source boundary",
                    "missing block boundary",
                    "authored join",
                    "folded join",
                    "omitted join",
                    "bound loop target",
                    "protected guard",
                    "protected break",
                    "completed region",
                    "duplicate region start",
                    "duplicate region end",
                ] {
                    if slots == 0 && mutation.starts_with("pop ") {
                        continue;
                    }
                    let mut assembler = Assembler::with_python_version(version);
                    let join = assembler.label();
                    let break_target = assembler.label();
                    let outer_break = assembler.label();
                    let body = assembler.label();
                    let guard_location = SourceLocation::new(4, 4, 7, 12);
                    let break_location = SourceLocation::new(8, 8, 8, 13);
                    let depth = outer_depth + slots;
                    assembler.set_location(guard_location);
                    assembler.emit_forward_with_depth(
                        POP_JUMP_IF_FALSE,
                        join,
                        depth,
                        ExceptionPolicy::default(),
                    );
                    assembler.mark(body);
                    assembler.record_exception_owner_block_boundary(body);
                    assembler.emit_with_depth(LOAD_CONST, 0, depth + 1, ExceptionPolicy::default());
                    assembler.emit_with_depth(POP_TOP, 0, depth, ExceptionPolicy::default());
                    let nop_index = assembler.linear_items().len();
                    assembler.set_location(break_location);
                    assembler.emit_with_depth(NOP, 0, depth, ExceptionPolicy::default());
                    for slot in 0..slots {
                        assembler.emit_with_depth(
                            POP_TOP,
                            0,
                            depth - slot - 1,
                            ExceptionPolicy::default(),
                        );
                    }
                    let jump = assembler.emit_forward_with_depth(
                        JUMP_FORWARD,
                        break_target,
                        outer_depth,
                        ExceptionPolicy::default(),
                    );
                    assembler.prevent_jump_threading_target(jump);
                    assembler.mark_jump_as_authored_break(jump);
                    assembler.mark(join);
                    assembler.record_exception_owner_block_boundary(join);
                    match mutation {
                        "wrong loop target" => assembler
                            .emitted_jump_mut(jump, "test")
                            .set_operand(Operand::Forward(outer_break)),
                        "backward break" => assembler
                            .emitted_jump_mut(jump, "test")
                            .set_operand(Operand::Backward(break_target)),
                        "unauthored break" => {
                            let mut expected = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(break_target),
                                break_location,
                                Some(outer_depth),
                            );
                            expected.jump_policy.prevent_threading_target();
                            *assembler.emitted_jump_mut(jump, "test") = expected;
                        }
                        "threadable break" => {
                            assembler.emitted_jump_mut(jump, "test").jump_policy =
                                JumpPolicy::default();
                        }
                        "retained break" => assembler
                            .emitted_jump_mut(jump, "test")
                            .jump_policy
                            .retain_nop(),
                        "break depth" => {
                            let mut expected = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(break_target),
                                break_location,
                                Some(outer_depth + 1),
                            );
                            expected.fusion_policy.mark_authored_break();
                            expected.jump_policy.prevent_threading_target();
                            *assembler.emitted_jump_mut(jump, "test") = expected;
                        }
                        "break location" => {
                            assembler.emitted_jump_mut(jump, "test").location = guard_location;
                        }
                        "break owner" => assembler
                            .emitted_jump_mut(jump, "test")
                            .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                        "break hidden slots" => assembler
                            .emitted_jump_mut(jump, "test")
                            .add_pre_fold_extra_instructions(1),
                        "duplicate break" => {
                            let original = *assembler.emitted_jump_mut(jump, "test");
                            insert_item(&mut assembler, 1, Item::Instruction(original));
                        }
                        "guard location" => guard_mut(&mut assembler).location = break_location,
                        "guard depth" => {
                            *guard_mut(&mut assembler) = Instruction::new(
                                POP_JUMP_IF_FALSE,
                                Operand::Forward(join),
                                guard_location,
                                Some(depth + 1),
                            );
                        }
                        "backward guard" => {
                            guard_mut(&mut assembler).set_operand(Operand::Backward(join));
                        }
                        "guard policy" => guard_mut(&mut assembler).jump_policy.retain_nop(),
                        "guard hidden slots" => {
                            guard_mut(&mut assembler).add_pre_fold_extra_instructions(1);
                        }
                        "guard owner" => guard_mut(&mut assembler)
                            .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                        "nop location" | "nop policy" | "nop opcode" => {
                            let Item::Instruction(nop) =
                                &mut assembler.linear_items_mut()[nop_index]
                            else {
                                unreachable!()
                            };
                            match mutation {
                                "nop location" => nop.location = guard_location,
                                "nop policy" => nop
                                    .no_location_policy
                                    .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                                _ => nop.opcode = NOT_TAKEN,
                            }
                        }
                        "missing nop" => {
                            assembler.linear_items_mut()[nop_index] = Item::Instruction(
                                Instruction::value_with_depth(POP_TOP, 0, break_location, depth),
                            );
                        }
                        "pop location" | "pop operand" | "pop opcode" | "pop depth"
                        | "pop policy" => {
                            let Item::Instruction(pop) =
                                &mut assembler.linear_items_mut()[nop_index + 1]
                            else {
                                unreachable!()
                            };
                            match mutation {
                                "pop location" => pop.location = guard_location,
                                "pop operand" => pop.set_operand(Operand::Value(1)),
                                "pop opcode" => pop.opcode = POP_EXCEPT,
                                "pop depth" => {
                                    *pop = Instruction::value_with_depth(
                                        POP_TOP,
                                        0,
                                        break_location,
                                        depth,
                                    );
                                }
                                _ => pop.jump_policy.retain_nop(),
                            }
                        }
                        "extra pop" => insert_item(
                            &mut assembler,
                            nop_index + 1,
                            Item::Instruction(Instruction::value_with_depth(
                                POP_TOP,
                                0,
                                break_location,
                                depth,
                            )),
                        ),
                        "interior label" => assembler.insert_label(nop_index + 1, outer_break),
                        "second guard" | "same span outer guard" => insert_item(
                            &mut assembler,
                            0,
                            Item::Instruction(Instruction::new(
                                POP_JUMP_IF_FALSE,
                                Operand::Forward(join),
                                guard_location,
                                Some(depth),
                            )),
                        ),
                        "duplicate join" => assembler.mark(join),
                        "foreign alias" => assembler.mark(outer_break),
                        "repeated join" => assembler.insert_label(0, join),
                        "missing source boundary" => {
                            assembler
                                .exception_owner_provenance
                                .source_boundaries
                                .remove(&join);
                        }
                        "missing block boundary" => {
                            assembler
                                .exception_owner_provenance
                                .block_boundaries
                                .remove(&join);
                        }
                        "authored join" => {
                            assembler.original_nop_targets.insert(join);
                        }
                        "folded join" => {
                            assembler.folded_away_condition_targets.insert(join);
                        }
                        "omitted join" => {
                            assembler
                                .exception_owner_provenance
                                .omitted_nop_counts
                                .insert(join, 1);
                        }
                        "bound loop target" => assembler.insert_label(0, break_target),
                        "protected guard"
                        | "protected break"
                        | "completed region"
                        | "duplicate region start"
                        | "duplicate region end" => {
                            let start = assembler.label();
                            let end = assembler.label();
                            let (start_index, end_index) = match mutation {
                                "protected guard"
                                | "duplicate region start"
                                | "duplicate region end" => (0, 1),
                                "protected break" => (nop_index, nop_index + slots as usize + 2),
                                _ => (2, nop_index),
                            };
                            assembler.insert_label(end_index, end);
                            assembler.insert_label(start_index, start);
                            if mutation == "duplicate region start" {
                                assembler.insert_label(0, start);
                            } else if mutation == "duplicate region end" {
                                assembler.insert_label(1, end);
                            }
                            assembler.exception_regions.push(ExceptionRegion {
                                start,
                                end,
                                target: outer_break,
                                depth: outer_depth,
                                preserve_lasti: false,
                                is_scope_fallback: false,
                                priority: ExceptionRegionPriority::OriginalRange(start, end),
                            });
                        }
                        _ => {}
                    }
                    assert_eq!(
                        assembler.original_break_guard_has_false_donor(
                            guard_location,
                            break_location,
                            break_target,
                            if mutation == "wrong slot count" {
                                slots + 1
                            } else {
                                slots
                            }
                        ),
                        version >= PythonVersion::Py314
                            && matches!(mutation, "intact" | "completed region"),
                        "{version:?} slots={slots} outer={outer_depth}: {mutation}"
                    );
                }
            }
        }
    }
}
