use super::ir::{
    BorrowFallthrough, ExceptionPolicy, FusionPolicy, JumpPolicy, LocalPolicy, NoLocationPolicy,
    RetainedJumpNop, UnoptimizedCode,
};
use super::{
    Assembler, AssemblerStage, BooleanValueMarkerScan, EmittedJump, ExceptionExclusion,
    Instruction, Item, NormalizedExceptionOwner, Operand, OperandPatchId, SourceLocation,
};
use crate::target::opcodes::*;
use crate::target::operands::{CommonConstant, Conversion, FunctionAttribute, ResumeLocation};

#[test]
fn folded_try_manager_pop_requires_original_forward_breaks() {
    for adjacent in [false, true] {
        for mutation in [
            "intact",
            "missing exit",
            "missing folded edge",
            "unlocated break",
            "conditional break",
            "backward break",
            "hidden break",
            "extra break slot",
            "retained break",
            "authored bridge nop",
            "located pop",
            "small cleanup",
            "not authored",
            "handler alias",
            "setup alias",
            "blocked bridge",
            "blocked break",
        ] {
            let mut assembler = Assembler::default();
            let entry = assembler.label();
            let target = assembler.label();
            let location = SourceLocation::new(6, 6, 8, 13);
            assembler.push_linear_item(Item::Instruction(Instruction::new(
                POP_TOP,
                Operand::Value(0),
                location,
                Some(2),
            )));
            let mut jump =
                Instruction::new(JUMP_FORWARD, Operand::Forward(entry), location, Some(2));
            if mutation != "not authored" {
                jump.fusion_policy.mark_authored_break();
            }
            jump.jump_policy.prevent_threading_target();
            jump.jump_policy.retain_nop_for_direct_target();
            match mutation {
                "unlocated break" => jump.location = SourceLocation::NONE,
                "conditional break" => jump.opcode = POP_JUMP_IF_FALSE,
                "backward break" => jump.set_operand(Operand::Backward(entry)),
                "hidden break" => jump.local_policy.add_hidden_exception_owner_slots(1),
                "extra break slot" => jump.add_pre_fold_extra_instructions(1),
                "retained break" => jump.jump_policy.retain_nop(),
                "blocked break" => jump.jump_policy.prevent_small_exit_inlining(),
                _ => {}
            }
            assembler.push_linear_item(Item::Instruction(jump));
            if !adjacent {
                assembler.push_linear_item(Item::Instruction(Instruction::new(
                    RAISE_VARARGS,
                    Operand::Value(0),
                    location,
                    Some(2),
                )));
            }
            assembler.mark(entry);
            if mutation == "handler alias" {
                assembler.add_exception_region(entry, target, entry, 2, false);
            }
            if mutation == "setup alias" {
                assembler.record_exception_setup(entry, false);
            }
            if mutation != "missing folded edge" {
                assembler.record_folded_loop_exit_anchor(entry);
            }
            if mutation == "authored bridge nop" {
                assembler.original_nop_targets.insert(entry);
            }
            if mutation != "missing exit" {
                assembler
                    .exception_owner_provenance
                    .normal_try_exits
                    .push((entry, target));
            }
            let mut bridge = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::NONE,
                Some(2),
            );
            if mutation == "blocked bridge" {
                bridge.jump_policy.prevent_small_exit_inlining();
            }
            assembler.push_linear_item(Item::Instruction(bridge));
            assembler.mark(target);
            let marker_index = assembler.instruction_count();
            let mut marker =
                Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(2));
            marker.fusion_policy.mark_converted_pop_block();
            marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            if mutation == "located pop" {
                marker.location = location;
            }
            let marker_item = assembler.linear_items().len();
            assembler.push_linear_item(Item::Instruction(marker));
            for _ in 0..if mutation == "small cleanup" { 1 } else { 4 } {
                assembler.push_linear_item(Item::Instruction(Instruction::new(
                    LOAD_CONST,
                    Operand::Value(0),
                    location,
                    Some(2),
                )));
            }
            assembler.push_linear_item(Item::Instruction(Instruction::new(
                RETURN_VALUE,
                Operand::Value(0),
                location,
                Some(2),
            )));
            let before = assembler.linear_items().to_vec();
            assembler.preserve_with_pop_after_folded_try_bridge(marker_index);
            if matches!(mutation, "intact" | "blocked bridge") {
                marker.no_location_policy.preserve_location();
                assert_eq!(
                    assembler.linear_items()[marker_item],
                    Item::Instruction(marker)
                );
            } else {
                assert_eq!(assembler.linear_items(), before, "{adjacent}: {mutation}");
            }
        }
    }
}

#[test]
fn shared_legacy_join_requires_original_unmodified_forward_edges() {
    for version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        for mutation in [
            "intact",
            "conditional",
            "backward opcode",
            "backward",
            "hidden",
            "compacted",
            "missing source",
            "missing block",
            "folded condition",
            "folded else",
            "authored nop",
            "borrow alias",
            "fusion",
            "continuation",
            "setup",
            "handler",
            "no fallthrough",
            "populated join",
        ] {
            let mut assembler = Assembler::with_python_version(version);
            let join = assembler.label();
            let start = assembler.label();
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(join),
                SourceLocation::NONE,
                Some(0),
            );
            match mutation {
                "conditional" => jump.opcode = POP_JUMP_IF_FALSE,
                "backward opcode" => jump.opcode = JUMP_BACKWARD,
                "backward" => jump.set_operand(Operand::Backward(join)),
                "hidden" => jump.local_policy.add_hidden_exception_owner_slots(1),
                "compacted" => jump.add_pre_fold_extra_instructions(1),
                _ => {}
            }
            let fallthrough = Instruction::new(
                if mutation == "no fallthrough" {
                    RETURN_VALUE
                } else {
                    STORE_FAST
                },
                Operand::Value(0),
                SourceLocation::new(3, 3, 4, 9),
                Some(0),
            );
            assembler.code = UnoptimizedCode::Linear(vec![
                Item::Label(start),
                Item::Instruction(jump),
                Item::Instruction(fallthrough),
                Item::Label(join),
            ]);
            assembler.record_exception_owner_block_boundary(join);
            match mutation {
                "missing source" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&join);
                }
                "missing block" => {
                    assembler
                        .exception_owner_provenance
                        .block_boundaries
                        .remove(&join);
                }
                "folded condition" => {
                    assembler.folded_away_condition_targets.insert(join);
                }
                "folded else" => {
                    assembler.folded_away_loop_else_targets.insert(join);
                }
                "authored nop" => {
                    assembler.original_nop_targets.insert(join);
                }
                "borrow alias" => {
                    assembler.borrow_block_aliases.insert(join);
                }
                "fusion" => {
                    assembler
                        .exception_owner_provenance
                        .fusion_boundaries
                        .insert(join);
                }
                "continuation" => {
                    assembler
                        .exception_owner_provenance
                        .continuation_boundaries
                        .insert(join);
                }
                "setup" => assembler.record_exception_setup(join, false),
                "handler" => assembler.add_exception_region(start, join, join, 0, false),
                "populated join" => {
                    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
                }
                _ => {}
            }
            assert!(
                !assembler.at_original_shared_handler_join(),
                "{version:?}: {mutation}"
            );
            assert_eq!(
                assembler.at_original_shared_legacy_join(),
                version == crate::PythonVersion::Py313 && mutation == "intact",
                "{version:?}: {mutation}"
            );
        }
    }
}

#[test]
fn original_while_exit_donor_requires_one_unmodified_condition_edge() {
    let location = SourceLocation::new(9, 9, 10, 17);
    for version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        for opcode in [POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE] {
            for backedge_opcode in [JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT] {
                for mutation in [
                    "intact",
                    "alias",
                    "missing labels",
                    "populated join",
                    "no incoming",
                    "shared join",
                    "condition opcode",
                    "condition backward",
                    "condition location",
                    "condition depth",
                    "condition folded",
                    "condition hidden",
                    "condition preserved",
                    "backedge opcode",
                    "backedge forward",
                    "backedge to exit",
                    "backedge depth",
                    "backedge folded",
                    "backedge hidden",
                    "backedge preserved",
                    "missing source",
                    "missing block",
                    "authored nop",
                    "borrow alias",
                    "fusion boundary",
                    "continuation boundary",
                    "setup boundary",
                    "handler boundary",
                    "folded condition",
                    "folded else",
                ] {
                    let mut assembler = Assembler::with_python_version(version);
                    let head = assembler.label();
                    let exit = assembler.label();
                    let alias = assembler.label();
                    let mut condition =
                        Instruction::new(opcode, Operand::Forward(exit), location, Some(0));
                    let mut backedge = Instruction::new(
                        backedge_opcode,
                        Operand::Backward(head),
                        SourceLocation::new(10, 10, 8, 22),
                        Some(0),
                    );
                    match mutation {
                        "no incoming" => condition.set_operand(Operand::Forward(head)),
                        "condition opcode" => condition.opcode = NOP,
                        "condition backward" => condition.set_operand(Operand::Backward(exit)),
                        "condition location" => condition.location = SourceLocation::NONE,
                        "condition depth" => {
                            condition =
                                Instruction::new(opcode, Operand::Forward(exit), location, Some(1));
                        }
                        "condition folded" => condition.add_pre_fold_extra_instructions(1),
                        "condition hidden" => {
                            condition.local_policy.add_hidden_exception_owner_slots(1);
                        }
                        "condition preserved" => condition.no_location_policy.preserve_location(),
                        "backedge opcode" => backedge.opcode = JUMP_FORWARD,
                        "backedge forward" => backedge.set_operand(Operand::Forward(head)),
                        "backedge to exit" => backedge.set_operand(Operand::Backward(exit)),
                        "backedge depth" => {
                            backedge = Instruction::new(
                                backedge_opcode,
                                Operand::Backward(head),
                                backedge.location,
                                Some(1),
                            );
                        }
                        "backedge folded" => backedge.add_pre_fold_extra_instructions(1),
                        "backedge hidden" => {
                            backedge.local_policy.add_hidden_exception_owner_slots(1);
                        }
                        "backedge preserved" => backedge.no_location_policy.preserve_location(),
                        _ => {}
                    }
                    assembler.push_linear_item(Item::Label(head));
                    assembler.push_linear_item(Item::Instruction(condition));
                    if mutation == "shared join" {
                        assembler.push_linear_item(Item::Instruction(condition));
                    }
                    assembler.push_linear_item(Item::Instruction(backedge));
                    if mutation != "missing labels" {
                        assembler.push_linear_item(Item::Label(exit));
                    }
                    if mutation == "alias" {
                        assembler.push_linear_item(Item::Label(alias));
                    }
                    if mutation == "populated join" {
                        assembler.push_linear_item(Item::Instruction(Instruction::new(
                            NOP,
                            Operand::Value(0),
                            location,
                            Some(0),
                        )));
                    }
                    let provenance = &mut assembler.exception_owner_provenance;
                    if mutation != "missing source" {
                        provenance.source_boundaries.insert(exit);
                    }
                    if mutation != "missing block" {
                        provenance.block_boundaries.insert(exit);
                    }
                    match mutation {
                        "fusion boundary" => {
                            provenance.fusion_boundaries.insert(exit);
                        }
                        "continuation boundary" => {
                            provenance.continuation_boundaries.insert(exit);
                        }
                        "setup boundary" => provenance.setups.push((exit, None)),
                        "handler boundary" => {
                            assembler.add_exception_region(head, alias, exit, 0, false);
                        }
                        "authored nop" => {
                            assembler.original_nop_targets.insert(exit);
                        }
                        "borrow alias" => {
                            assembler.borrow_block_aliases.insert(exit);
                        }
                        "folded condition" => {
                            assembler.folded_away_condition_targets.insert(exit);
                        }
                        "folded else" => {
                            assembler.folded_away_loop_else_targets.insert(exit);
                        }
                        _ => {}
                    }
                    let original = assembler.linear_items().to_vec();
                    assert_eq!(
                        assembler.original_while_exit_has_false_donor(location),
                        matches!(mutation, "intact" | "alias"),
                        "{version:?} {opcode:?} {backedge_opcode:?} {mutation}",
                    );
                    assert_eq!(assembler.linear_items(), original);
                }
            }
        }
    }
}

#[test]
fn nested_finally_exit_donor_requires_full_records_and_one_original_edge() {
    let location = SourceLocation::new(9, 9, 16, 34);
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for mutation in 0..36 {
            let mut assembler = Assembler::with_python_version(version);
            let end = assembler.label();
            let alias = assembler.label();
            let mut records = [
                Instruction::new(POP_TOP, Operand::Value(0), location, Some(1)),
                Instruction::new(JUMP_FORWARD, Operand::Forward(end), location, Some(1)),
                Instruction::new(COPY, Operand::Value(3), SourceLocation::NONE, Some(5)),
                Instruction::new(POP_EXCEPT, Operand::Value(0), SourceLocation::NONE, Some(4)),
                Instruction::new(RERAISE, Operand::Value(1), SourceLocation::NONE, Some(1)),
            ];
            if (1..=30).contains(&mutation) {
                let instruction = &mut records[(mutation - 1) / 6];
                match (mutation - 1) % 6 {
                    0 => instruction.opcode = NOP,
                    1 => instruction.set_operand(Operand::Value(7)),
                    2 => instruction.location = SourceLocation::new(99, 99, 0, 1),
                    3 => {
                        *instruction = Instruction::new(
                            instruction.opcode,
                            instruction.operand(),
                            instruction.location,
                            Some(2),
                        );
                    }
                    4 => instruction.local_policy.add_hidden_exception_owner_slots(1),
                    5 => instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    _ => unreachable!(),
                }
            }
            if mutation == 31 {
                assembler.push_linear_item(Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(alias),
                    location,
                    Some(1),
                )));
            }
            for record in records {
                assembler.push_linear_item(Item::Instruction(record));
            }
            assembler.mark(end);
            assembler.record_exception_owner_block_boundary(end);
            assembler.mark(alias);
            match mutation {
                32 => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&end);
                }
                33 => {
                    assembler
                        .exception_owner_provenance
                        .block_boundaries
                        .remove(&end);
                }
                34 => {
                    assembler.folded_away_condition_targets.insert(alias);
                }
                35 => {
                    assembler.folded_away_loop_else_targets.insert(alias);
                }
                _ => {}
            }
            let before = assembler.linear_items().to_vec();
            assert_eq!(
                assembler.original_nested_finally_exit_has_donor(location),
                mutation == 0,
                "{version:?}: mutation {mutation}",
            );
            assert_eq!(assembler.linear_items(), before);
        }
    }
}

#[test]
fn recovery_normal_exit_requires_a_populated_original_source() {
    for version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        for mutation in [
            "intact",
            "exception alias",
            "nop only",
            "empty",
            "missing source",
            "preserved head",
            "preserved alias",
            "separate empty array",
            "jump",
            "return",
            "marker",
            "empty policy",
            "borrow barrier",
        ] {
            let mut assembler = Assembler::with_python_version(version);
            let head = assembler.label();
            let alias = assembler.label();
            assembler.mark(head);
            assembler.record_exception_owner_block_boundary(head);
            let location = SourceLocation::new(4, 4, 8, 18);
            let mut work = Instruction::new(CALL, Operand::Value(0), location, Some(1));
            match mutation {
                "missing source" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&head);
                }
                "preserved head" => {
                    assembler.preserved_block_boundaries.insert(head);
                }
                "jump" => work.set_operand(Operand::Forward(alias)),
                "return" => work.opcode = RETURN_VALUE,
                "marker" => work.opcode = NOT_TAKEN,
                "nop only" => work.opcode = NOP,
                "empty policy" => work.jump_policy.mark_borrow_empty_block(),
                "borrow barrier" => work
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                _ => {}
            }
            if mutation != "empty" {
                assembler.push_linear_item(Item::Instruction(work));
                assembler.push_linear_item(Item::Instruction(Instruction::new(
                    if mutation == "nop only" { NOP } else { POP_TOP },
                    Operand::Value(0),
                    location,
                    Some(0),
                )));
            }
            assembler.mark(alias);
            match mutation {
                "preserved alias" => {
                    assembler.preserved_block_boundaries.insert(alias);
                }
                "separate empty array" => {
                    assembler.record_exception_owner_block_boundary(alias);
                }
                _ => {}
            }
            assert_eq!(
                assembler.current_original_block_has_nonempty_borrow_source(),
                matches!(mutation, "intact" | "exception alias" | "nop only"),
                "{version:?} {mutation}"
            );
            assert_eq!(
                assembler.current_original_block_has_runtime_borrow_source(),
                matches!(mutation, "intact" | "exception alias"),
                "{version:?} {mutation}"
            );
        }
    }
}

fn outer_try_iterator_return(
    version: crate::PythonVersion,
    opcode: crate::target::Opcode,
) -> (Assembler, super::CopiedFinallyReturnPrefix, super::Label) {
    let mut assembler = Assembler::with_python_version(version);
    let entry = assembler.label();
    let outer_pop = assembler.label();
    let slots = if version == crate::PythonVersion::Py315 {
        2
    } else {
        1
    };
    assembler.mark(entry);
    assembler.set_location(SourceLocation::new(8, 9, 12, 20));
    assembler.emit_with_depth(opcode, 0, slots + 2, ExceptionPolicy::default());
    assembler.emit_with_depth(POP_TOP, 0, slots + 1, ExceptionPolicy::default());
    let prefix = assembler.copied_finally_return_prefix().unwrap();
    assembler.set_location(SourceLocation::NONE);
    assembler.emit_with_depth(SWAP, slots + 1, slots + 1, ExceptionPolicy::default());
    for depth in (1..=slots).rev() {
        assembler.emit_with_depth(POP_TOP, 0, depth, ExceptionPolicy::default());
    }
    assembler.mark(outer_pop);
    assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
    (assembler, prefix, outer_pop)
}

#[test]
fn outer_try_iterator_return_binds_original_prefix_and_canonical_footer() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for opcode in [CALL, END_SEND] {
            for mutation in [
                "intact",
                "prefix span",
                "prefix opcode",
                "prefix operand",
                "prefix offset",
                "foreign stream",
                "donor moved",
                "footer opcode",
                "footer operand",
                "footer depth",
                "footer location",
                "footer local",
                "footer no location",
                "footer jump",
                "footer fusion",
                "footer owner",
                "footer width",
                "footer admission",
                "footer folded",
                "footer annotation",
                "footer extra",
                "missing boundary",
                "duplicate boundary",
                "extra alias",
                "foreign incoming",
                "prepared",
                "legacy",
            ] {
                let (mut assembler, mut prefix, outer_pop) =
                    outer_try_iterator_return(version, opcode);
                let foreign = assembler.label();
                let last = assembler.item_count() - 1;
                match mutation {
                    "prefix span" => {
                        for index in 1..3 {
                            let Item::Instruction(record) =
                                &mut assembler.linear_items_mut()[index]
                            else {
                                panic!()
                            };
                            record.location = SourceLocation::new(10, 10, 0, 4);
                        }
                    }
                    "prefix opcode" => {
                        let Item::Instruction(record) = &mut assembler.linear_items_mut()[1] else {
                            panic!()
                        };
                        record.opcode = if opcode == CALL { END_SEND } else { CALL };
                    }
                    "prefix operand" => {
                        let Item::Instruction(record) = &mut assembler.linear_items_mut()[1] else {
                            panic!()
                        };
                        record.set_operand(Operand::Value(1));
                    }
                    "prefix offset" => prefix.cleanup_end += 1,
                    "foreign stream" => prefix.stream = super::ir::EmissionStreamId::next(),
                    "donor moved" => assembler.linear_items_mut().swap(1, 2),
                    "footer extra" => {
                        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                            unreachable!()
                        };
                        items.insert(last, pass_instruction(NOP, 0, 0));
                    }
                    "missing boundary" => {
                        assembler.linear_items_mut()[last - 1] = Item::Label(foreign);
                    }
                    "duplicate boundary" => {
                        assembler.linear_items_mut()[0] = Item::Label(outer_pop);
                    }
                    "extra alias" => {
                        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                            unreachable!()
                        };
                        items.insert(last, Item::Label(foreign));
                    }
                    "foreign incoming" => {
                        assembler.linear_items_mut()[0] = Item::Instruction(Instruction::new(
                            JUMP_FORWARD,
                            Operand::Forward(outer_pop),
                            SourceLocation::NONE,
                            Some(0),
                        ));
                    }
                    "prepared" => {
                        let graph = super::block_graph::BlockGraph::from_items(
                            assembler.linear_items().to_vec(),
                            &rustc_hash::FxHashSet::default(),
                        );
                        let reachable = graph.reachable_blocks(&[]);
                        assembler.code = UnoptimizedCode::Prepared { graph, reachable };
                        assert!(assembler.copied_finally_return_prefix().is_none());
                    }
                    "legacy" => assembler.python_version = crate::PythonVersion::Py313,
                    "intact" => {}
                    _ => {
                        let Item::Instruction(record) = &mut assembler.linear_items_mut()[3] else {
                            panic!()
                        };
                        match mutation {
                            "footer opcode" => record.opcode = NOP,
                            "footer operand" => record.set_operand(Operand::Value(0)),
                            "footer depth" => {
                                *record = Instruction::new(
                                    record.opcode,
                                    record.operand(),
                                    record.location,
                                    None,
                                );
                            }
                            "footer location" => record.location = SourceLocation::new(1, 1, 0, 4),
                            "footer local" => record.local_policy.mark_force_borrowed_load(),
                            "footer no location" => record
                                .no_location_policy
                                .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                            "footer jump" => record.jump_policy.retain_nop(),
                            "footer fusion" => record.fusion_policy.prevent_with_previous(),
                            "footer owner" => {
                                record.set_exception_owner(NormalizedExceptionOwner::Absent);
                            }
                            "footer width" => record.add_pre_fold_extra_instructions(1),
                            "footer admission" => record.early_boolean_admissions.append(true),
                            "footer folded" => {
                                record.folded_constant_admissions =
                                    Some(super::ir::FoldAdmissionId::from_u32(0));
                            }
                            "footer annotation" => record.mark_original_annotation_jump(),
                            _ => unreachable!(),
                        }
                    }
                }
                assembler.record_copied_finally_outer_try_iterator_return(&prefix, outer_pop);
                assert_eq!(
                    assembler
                        .exception_owner_provenance
                        .copied_finally_iterator_returns
                        .len(),
                    usize::from(mutation == "intact"),
                    "{version:?} {opcode:?} {mutation}",
                );
            }
        }
    }
}

#[test]
fn outer_try_iterator_return_rejects_noncoverage_boundary_metadata() {
    for mutation in [
        "authored",
        "borrow alias",
        "source",
        "block",
        "borrow block",
        "preserved",
        "fusion",
        "continuation",
        "setup",
        "handler",
        "folded condition",
        "folded loop",
        "omitted count",
        "omitted location",
    ] {
        let (mut assembler, prefix, outer_pop) =
            outer_try_iterator_return(crate::PythonVersion::Py314, CALL);
        let provenance = &mut assembler.exception_owner_provenance;
        match mutation {
            "authored" => {
                assembler.original_nop_targets.insert(outer_pop);
            }
            "borrow alias" => {
                assembler.borrow_block_aliases.insert(outer_pop);
            }
            "source" => {
                provenance.source_boundaries.insert(outer_pop);
            }
            "block" => {
                provenance.block_boundaries.insert(outer_pop);
            }
            "borrow block" => {
                provenance.borrow_only_block_boundaries.insert(outer_pop);
            }
            "preserved" => {
                assembler.preserved_block_boundaries.insert(outer_pop);
            }
            "fusion" => {
                provenance.fusion_boundaries.insert(outer_pop);
            }
            "continuation" => {
                provenance.continuation_boundaries.insert(outer_pop);
            }
            "setup" => provenance.setups.push((outer_pop, None)),
            "handler" => assembler
                .exception_regions
                .push(super::ir::ExceptionRegion {
                    is_scope_fallback: false,
                    priority: super::ir::ExceptionRegionPriority::OriginalRange(
                        outer_pop, outer_pop,
                    ),
                    start: outer_pop,
                    end: outer_pop,
                    target: outer_pop,
                    depth: 0,
                    preserve_lasti: false,
                }),
            "folded condition" => {
                assembler.folded_away_condition_targets.insert(outer_pop);
            }
            "folded loop" => {
                assembler.folded_away_loop_else_targets.insert(outer_pop);
            }
            "omitted count" => {
                provenance.omitted_nop_counts.insert(outer_pop, 1);
            }
            "omitted location" => {
                provenance
                    .omitted_nop_locations
                    .insert(outer_pop, SourceLocation::new(1, 1, 0, 4));
            }
            _ => unreachable!(),
        }
        let before = assembler.linear_items().to_vec();
        assembler.record_copied_finally_outer_try_iterator_return(&prefix, outer_pop);
        assert!(
            assembler
                .exception_owner_provenance
                .copied_finally_iterator_returns
                .is_empty(),
            "{mutation}"
        );
        assert_eq!(assembler.linear_items(), before, "{mutation}");
    }
}

#[test]
fn outer_try_iterator_return_replays_only_the_unchanged_original_array_once() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for mutation in [
            "intact",
            "donor",
            "return",
            "missing boundary",
            "incoming",
            "preserved",
        ] {
            let (mut assembler, prefix, outer_pop) = outer_try_iterator_return(version, CALL);
            let donor = prefix.original[0].location;
            assembler.record_copied_finally_outer_try_iterator_return(&prefix, outer_pop);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .copied_finally_iterator_returns
                    .len(),
                1
            );
            let last = assembler.item_count() - 1;
            let foreign = assembler.label();
            match mutation {
                "donor" => {
                    let Item::Instruction(record) = &mut assembler.linear_items_mut()[1] else {
                        panic!()
                    };
                    record.set_operand(Operand::Value(1));
                }
                "return" => {
                    let Item::Instruction(record) = &mut assembler.linear_items_mut()[last] else {
                        panic!()
                    };
                    record.jump_policy.retain_nop();
                }
                "missing boundary" => assembler.linear_items_mut()[last - 1] = Item::Label(foreign),
                "incoming" => {
                    assembler.linear_items_mut()[0] = Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(outer_pop),
                        SourceLocation::NONE,
                        Some(0),
                    ));
                }
                "preserved" => {
                    assembler.preserved_block_boundaries.insert(outer_pop);
                }
                "intact" => {}
                _ => unreachable!(),
            }
            assembler.remap_constant_indices(&[]);
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.propagate_locations_within_blocks();
            assert!(
                pipeline
                    .exception_owner_provenance
                    .copied_finally_iterator_returns
                    .is_empty()
            );
            let items = pipeline.graph.copy_items();
            let Some(Item::Instruction(last)) = items.last() else {
                panic!()
            };
            assert_eq!(
                last.location,
                if mutation == "intact" {
                    donor
                } else {
                    SourceLocation::NONE
                },
                "{version:?} {mutation}"
            );
            pipeline.propagate_locations_within_blocks();
            assert_eq!(pipeline.graph.copy_items(), items);
        }
    }
}

#[test]
fn conditional_normal_try_borrow_head_requires_proven_fallthrough() {
    let source = super::Label::from_u32(0);
    let fallthrough = super::Label::from_u32(1);
    let head = super::Label::from_u32(2);
    let exit = super::Label::from_u32(3);
    let extra = super::Label::from_u32(4);
    let located = SourceLocation::new(1, 1, 0, 1);

    for (variation, expected) in [
        ("converging", true),
        ("missing-provenance", false),
        ("wrong-entry", false),
        ("wrong-target", false),
        ("intervening-conditional", false),
        ("unreachable-fallthrough", false),
        ("extra-incoming", false),
        ("python-315", false),
        ("live-jump", true),
    ] {
        let instruction =
            |opcode, operand| Item::Instruction(Instruction::new(opcode, operand, located, None));
        let mut items = Vec::new();
        if variation == "extra-incoming" {
            items.extend([
                instruction(JUMP_FORWARD, Operand::Forward(source)),
                Item::Label(extra),
                instruction(JUMP_FORWARD, Operand::Forward(head)),
            ]);
        }
        items.extend([
            Item::Label(source),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(POP_JUMP_IF_FALSE, Operand::Forward(head)),
            instruction(NOT_TAKEN, Operand::Value(0)),
        ]);
        if variation == "unreachable-fallthrough" {
            items.push(instruction(JUMP_FORWARD, Operand::Forward(exit)));
        }
        items.push(Item::Label(fallthrough));
        if variation == "intervening-conditional" {
            items.extend([
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(POP_JUMP_IF_TRUE, Operand::Forward(exit)),
                instruction(NOT_TAKEN, Operand::Value(0)),
            ]);
        }
        items.extend([
            instruction(NOP, Operand::Value(0)),
            Item::Label(head),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(exit),
                SourceLocation::NONE,
                None,
            )),
        ]);
        if variation == "live-jump" {
            items.extend([Item::Label(extra), instruction(NOP, Operand::Value(0))]);
        }
        items.extend([
            Item::Label(exit),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 5;
        assembler.python_version = if variation == "python-315" {
            crate::PythonVersion::Py315
        } else {
            crate::PythonVersion::Py314
        };
        if variation != "missing-provenance" {
            assembler.exception_owner_provenance.normal_try_exits.push((
                if variation == "wrong-entry" {
                    fallthrough
                } else {
                    head
                },
                if variation == "wrong-target" {
                    fallthrough
                } else {
                    exit
                },
            ));
        }
        let mut pipeline = assembler.into_optimization_pipeline();
        assert_eq!(
            pipeline.record_borrow_empty_jump_heads(),
            expected,
            "{variation}"
        );
        let jump = pipeline
            .graph
            .iter_items()
            .find_map(|item| match item {
                Item::Instruction(jump)
                    if jump.opcode == JUMP_FORWARD && jump.location == SourceLocation::NONE =>
                {
                    Some(jump)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            jump.jump_policy.borrow_empty_block(),
            expected,
            "{variation}"
        );
        if variation == "live-jump" {
            pipeline.remove_redundant_forward_jumps(true);
            assert!(pipeline.graph.iter_items().any(|item| {
                matches!(item, Item::Instruction(jump)
                    if jump.opcode == JUMP_FORWARD && jump.operand() == Operand::Forward(exit))
            }));
        }
    }
}

#[test]
fn pending_try_continue_copy_binds_original_unwind_and_fallback_aliases() {
    for mutation in [
        "intact",
        "copy start",
        "copy offset",
        "missing copy",
        "duplicate copy",
        "fallback",
        "missing fallback",
        "duplicate fallback",
        "populated fallback",
        "jump target",
        "jump location",
        "jump opcode",
        "condition location",
        "condition target",
        "condition duplicated",
        "condition outside unwind",
    ] {
        let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
        let target = assembler.label();
        let copy_start = assembler.label();
        let join = assembler.label();
        let fallback = assembler.label();
        let foreign = assembler.label();
        assembler.mark(target);
        assembler.emit(NOP, 0);
        let unwind_start = assembler.item_count();
        assembler.mark(copy_start);
        let location = SourceLocation::new(4, 4, 7, 16);
        assembler.set_location(location);
        assembler.emit(LOAD_FAST, 0);
        let condition = assembler.emit_forward_with_depth(
            POP_JUMP_IF_FALSE,
            join,
            0,
            ExceptionPolicy::default(),
        );
        let condition_index = usize::try_from(condition.instruction().item_index).unwrap();
        assembler.emit(LOAD_FAST, 1);
        assembler.emit(POP_TOP, 0);
        assembler.mark(join);
        assembler.mark(fallback);
        assembler.set_location(SourceLocation::NONE);
        let jump = assembler.emit_backward(JUMP_BACKWARD, target);
        let last = assembler.item_count() - 1;
        let mut captured_copy = copy_start;
        let mut captured_fallback = fallback;
        let mut captured_offset = unwind_start;
        match mutation {
            "copy start" => captured_copy = foreign,
            "copy offset" => captured_offset += 1,
            "missing copy" => assembler.linear_items_mut()[unwind_start] = Item::Label(foreign),
            "duplicate copy" => assembler.linear_items_mut()[0] = Item::Label(copy_start),
            "fallback" => captured_fallback = foreign,
            "missing fallback" => assembler.linear_items_mut()[last - 1] = Item::Label(foreign),
            "duplicate fallback" => assembler.linear_items_mut()[0] = Item::Label(fallback),
            "populated fallback" => {
                assembler.linear_items_mut()[last - 1] = Item::Instruction(Instruction::new(
                    NOP,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    None,
                ));
            }
            "jump target" => assembler
                .emitted_jump_mut(jump, "test")
                .set_operand(Operand::Backward(foreign)),
            "jump location" => assembler.emitted_jump_mut(jump, "test").location = location,
            "jump opcode" => assembler.emitted_jump_mut(jump, "test").opcode = JUMP_FORWARD,
            "condition location" => {
                let Item::Instruction(record) = &mut assembler.linear_items_mut()[condition_index]
                else {
                    panic!()
                };
                record.location = SourceLocation::NONE;
            }
            "condition target" => {
                let Item::Instruction(record) = &mut assembler.linear_items_mut()[condition_index]
                else {
                    panic!()
                };
                record.set_operand(Operand::Forward(foreign));
            }
            "condition duplicated" => {
                assembler.linear_items_mut()[unwind_start + 3] =
                    assembler.linear_items()[condition_index];
            }
            "condition outside unwind" => captured_offset = last - 1,
            _ => {}
        }
        let before = *assembler.emitted_jump_mut(jump, "before certificate");
        let accepted = assembler.allow_pending_try_copied_finally_continue_exit(
            jump,
            target,
            location,
            captured_offset,
            captured_copy,
            captured_fallback,
        );
        assert_eq!(accepted, mutation == "intact", "{mutation}");
        let after = *assembler.emitted_jump_mut(jump, "after certificate");
        if accepted {
            assert!(after.no_location_policy.allows_block_inlining());
            assert!(!after.only_allows_unconditional_inlining());
            assert!(assembler.conditional_threading_targets.is_empty());
        } else {
            assert_eq!(after, before, "{mutation}");
        }
    }
}

#[test]
fn nested_guard_location_requires_one_original_condition_and_threadable_bypasses() {
    for condition in [POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE] {
        for terminal_opcode in [RETURN_VALUE, RAISE_VARARGS] {
            for mutation in [
                "intact",
                "no bypass",
                "no guard",
                "extra condition",
                "late bypass",
                "guard location",
                "terminal location",
                "guard depth",
                "terminal depth",
                "guard folded",
                "guard hidden",
                "guard owner",
                "guard backward",
                "guard target",
                "bypass opcode",
                "bypass location",
                "bypass folded",
                "bypass retained",
                "terminal folded",
                "terminal hidden",
                "terminal opcode",
                "populated join",
                "missing source boundary",
                "missing block boundary",
                "folded condition",
                "folded loop else",
            ] {
                let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
                let bypass_target = assembler.label();
                let guard_target = assembler.label();
                let foreign = assembler.label();
                let condition_location = SourceLocation::new(8, 8, 7, 12);
                let terminal_location = SourceLocation::new(9, 9, 8, 20);
                if mutation != "no bypass" {
                    assembler.set_location(if mutation == "bypass location" {
                        SourceLocation::NONE
                    } else {
                        SourceLocation::new(4, 4, 8, 20)
                    });
                    let bypass = assembler.emit_forward_with_depth(
                        if mutation == "bypass opcode" {
                            POP_JUMP_IF_FALSE
                        } else {
                            JUMP_FORWARD
                        },
                        bypass_target,
                        0,
                        ExceptionPolicy::default(),
                    );
                    if mutation == "bypass folded" {
                        assembler
                            .emitted_jump_mut(bypass, "mutating bypass")
                            .add_pre_fold_extra_instructions(1);
                    }
                    if mutation == "bypass retained" {
                        assembler
                            .emitted_jump_mut(bypass, "mutating bypass")
                            .jump_policy
                            .retain_nop();
                    }
                }
                assembler.set_location(condition_location);
                let guard = assembler.emit_forward_with_depth(
                    if mutation == "no guard" {
                        NOP
                    } else {
                        condition
                    },
                    guard_target,
                    u32::from(mutation == "guard depth"),
                    ExceptionPolicy::default(),
                );
                let instruction = assembler.emitted_jump_mut(guard, "mutating guard");
                match mutation {
                    "guard location" => instruction.location.column += 1,
                    "guard folded" => instruction.add_pre_fold_extra_instructions(1),
                    "guard hidden" => instruction.local_policy.add_hidden_exception_owner_slots(1),
                    "guard owner" => {
                        instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                    }
                    "guard backward" => instruction.set_operand(Operand::Backward(guard_target)),
                    "guard target" => instruction.set_operand(Operand::Forward(foreign)),
                    _ => {}
                }
                if mutation == "extra condition" || mutation == "late bypass" {
                    assembler.emit_forward_with_depth(
                        if mutation == "extra condition" {
                            condition
                        } else {
                            JUMP_FORWARD
                        },
                        guard_target,
                        0,
                        ExceptionPolicy::default(),
                    );
                }
                assembler.set_location(terminal_location);
                let terminal = assembler.emit_with_depth(
                    if mutation == "terminal opcode" {
                        POP_TOP
                    } else {
                        terminal_opcode
                    },
                    u32::from(terminal_opcode == RAISE_VARARGS),
                    u32::from(mutation == "terminal depth"),
                    ExceptionPolicy::default(),
                );
                let instruction = assembler.emitted_instruction_mut(terminal, "mutating terminal");
                match mutation {
                    "terminal location" => instruction.location.column += 1,
                    "terminal folded" => instruction.add_pre_fold_extra_instructions(1),
                    "terminal hidden" => {
                        instruction.local_policy.add_hidden_exception_owner_slots(1);
                    }
                    _ => {}
                }
                for target in [guard_target, bypass_target] {
                    assembler.record_exception_owner_block_boundary(target);
                    assembler.mark(target);
                }
                match mutation {
                    "populated join" => {
                        assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
                    }
                    "missing source boundary" => {
                        assembler
                            .exception_owner_provenance
                            .source_boundaries
                            .remove(&guard_target);
                    }
                    "missing block boundary" => {
                        assembler
                            .exception_owner_provenance
                            .block_boundaries
                            .remove(&guard_target);
                    }
                    "folded condition" => {
                        assembler.folded_away_condition_targets.insert(guard_target);
                    }
                    "folded loop else" => {
                        assembler.folded_away_loop_else_targets.insert(guard_target);
                    }
                    _ => {}
                }
                assert_eq!(
                    assembler.original_nested_guard_has_false_donor(
                        condition_location,
                        terminal_location
                    ),
                    mutation == "intact",
                    "{condition:?}, {terminal_opcode:?}, {mutation}",
                );
            }
        }
    }
}

#[test]
fn noncopyable_cleanup_borrow_exits_preserve_their_exact_source_and_target() {
    for empty_source in [false, true] {
        for target_shape in ["four slots", "five slots", "conditional"] {
            for mutation in [
                "intact",
                "source location",
                "source owner",
                "source target",
                "source opcode",
                "stale stream",
                "remapped stream",
                "changed remapped source",
                "unlocated target",
                "interior target label",
                "source label",
                "source label order",
                "populated source",
            ] {
                let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
                let target = assembler.label();
                let other = assembler.label();
                let end = assembler.label();
                let entry = assembler.label();
                let alias = assembler.label();
                assembler.mark(entry);
                assembler.mark(alias);
                let source = assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    target,
                    0,
                    ExceptionPolicy::default(),
                );
                if empty_source {
                    assembler.record_noncopyable_empty_borrow_exit(source);
                } else {
                    assembler.record_noncopyable_borrow_exit(source);
                }
                let location = SourceLocation::new(3, 3, 4, 19);
                match mutation {
                    "source location" | "changed remapped source" => {
                        assembler
                            .emitted_jump_mut(source, "changing the source")
                            .location = location;
                    }
                    "source owner" => assembler
                        .emitted_jump_mut(source, "changing the owner")
                        .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    "source target" => assembler
                        .emitted_jump_mut(source, "changing the target")
                        .set_operand(Operand::Forward(other)),
                    "source opcode" => {
                        assembler
                            .emitted_jump_mut(source, "changing the opcode")
                            .opcode = NOP;
                    }
                    "stale stream" => {
                        assembler.emission_stream = super::ir::EmissionStreamId::next();
                    }
                    "source label" => assembler.linear_items_mut()[0] = Item::Label(other),
                    "source label order" => assembler.linear_items_mut().swap(0, 1),
                    "populated source" => {
                        assembler.linear_items_mut()[0] = Item::Instruction(Instruction::new(
                            NOP,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(0),
                        ));
                    }
                    _ => {}
                }
                if matches!(mutation, "remapped stream" | "changed remapped source") {
                    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
                    assert!(assembler.take_trailing_nop_location().is_some());
                    assert_eq!(
                        assembler.deferred_noncopyable_borrow_exits.len(),
                        usize::from(mutation == "remapped stream")
                    );
                }
                assembler.mark(target);
                if mutation != "unlocated target" {
                    assembler.set_location(location);
                }
                if target_shape == "conditional" {
                    assembler.emit_with_depth(LOAD_FAST, 0, 1, ExceptionPolicy::default());
                    if mutation == "interior target label" {
                        assembler.mark(other);
                    }
                    assembler.emit_forward_with_depth(
                        POP_JUMP_IF_FALSE,
                        end,
                        0,
                        ExceptionPolicy::default(),
                    );
                    assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
                    assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
                } else {
                    assembler.emit_with_depth(LOAD_GLOBAL, 1, 2, ExceptionPolicy::default());
                    if mutation == "interior target label" {
                        assembler.mark(other);
                    }
                    if target_shape == "five slots" {
                        assembler.emit_with_depth(LOAD_FAST, 0, 3, ExceptionPolicy::default());
                    }
                    assembler.emit_with_depth(
                        CALL,
                        u32::from(target_shape == "five slots"),
                        1,
                        ExceptionPolicy::default(),
                    );
                    assembler.emit_with_depth(RAISE_VARARGS, 1, 0, ExceptionPolicy::default());
                }
                assembler.mark(end);
                assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
                assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());

                let mut expected = assembler.linear_items().to_vec();
                let admitted = target_shape != "four slots"
                    && (matches!(mutation, "intact" | "remapped stream")
                        || (matches!(
                            mutation,
                            "source label" | "source label order" | "populated source"
                        ) && !empty_source));
                if admitted {
                    let Item::Instruction(instruction) = &mut expected[2] else {
                        unreachable!();
                    };
                    instruction
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                }
                assembler.resolve_noncopyable_borrow_exits();
                assert!(assembler.deferred_noncopyable_borrow_exits.is_empty());
                assert_eq!(
                    assembler.linear_items(),
                    expected,
                    "{empty_source}: {target_shape}: {mutation}"
                );
                let Item::Instruction(instruction) = assembler.linear_items()[2] else {
                    unreachable!();
                };
                assert!(instruction.jump_policy.allows_threading_target());
                assert!(instruction.jump_policy.allows_borrow_threading_target());
                let expected_labels = if admitted && empty_source {
                    [entry, alias].into_iter().collect()
                } else {
                    rustc_hash::FxHashSet::default()
                };
                assert_eq!(assembler.borrow_unreachable_blocks, expected_labels);
                let expected_boundaries = if admitted && empty_source {
                    [target].into_iter().collect()
                } else {
                    rustc_hash::FxHashSet::default()
                };
                assert_eq!(
                    assembler
                        .exception_owner_provenance
                        .borrow_only_block_boundaries,
                    expected_boundaries
                );
            }
        }
    }
}

#[test]
fn noncopyable_retained_finally_nops_keep_source_local_borrow_barriers() {
    for prepared in [false, true] {
        for retained in [false, true] {
            for width in [4, 5] {
                let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
                let target = assembler.label();
                assembler.set_location(SourceLocation::new(2, 2, 4, 16));
                let source = assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    target,
                    0,
                    ExceptionPolicy::default(),
                );
                if retained {
                    assembler.preserve_redundant_jump_nop_after_threading(source);
                    assembler.mark_retained_nop_borrow_fallthrough(source);
                }
                assembler.record_noncopyable_borrow_exit(source);
                assembler.mark(target);
                assembler.set_location(SourceLocation::new(4, 4, 4, 20));
                assembler.emit_with_depth(LOAD_GLOBAL, 1, 2, ExceptionPolicy::default());
                if width == 5 {
                    assembler.emit_with_depth(LOAD_CONST, 0, 3, ExceptionPolicy::default());
                }
                assembler.emit_with_depth(
                    CALL,
                    u32::from(width == 5),
                    1,
                    ExceptionPolicy::default(),
                );
                assembler.emit_with_depth(RAISE_VARARGS, 1, 0, ExceptionPolicy::default());
                let mut original = assembler.linear_items().to_vec();
                if width == 5 {
                    let Item::Instruction(jump) = &mut original[0] else {
                        unreachable!()
                    };
                    jump.no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                }
                if prepared {
                    assembler.used_constant_indices(1);
                    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
                } else {
                    assembler.resolve_noncopyable_borrow_exits();
                }
                assert!(assembler.deferred_noncopyable_borrow_exits.is_empty());
                assert!(assembler.borrow_unreachable_blocks.is_empty());
                assert!(
                    assembler
                        .exception_owner_provenance
                        .borrow_only_block_boundaries
                        .is_empty()
                );
                let pipeline = assembler.into_optimization_pipeline();
                assert_eq!(pipeline.graph.copy_items(), original);
                let jump = pipeline
                    .graph
                    .iter_items()
                    .find_map(|item| match item {
                        Item::Instruction(jump) if jump.opcode == JUMP_FORWARD => Some(jump),
                        _ => None,
                    })
                    .unwrap();
                assert!(jump.jump_policy.allows_threading_target());
                assert!(jump.jump_policy.allows_borrow_threading_target());
            }
        }
    }
}

#[test]
fn empty_noncopyable_borrow_exits_require_an_original_alias_array() {
    for empty_source in [false, true] {
        for populated in [false, true] {
            let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
            let target = assembler.label();
            if populated {
                let entry = assembler.label();
                assembler.mark(entry);
                assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
            }
            let source = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                target,
                0,
                ExceptionPolicy::default(),
            );
            if empty_source {
                assembler.record_noncopyable_empty_borrow_exit(source);
            } else {
                assembler.record_noncopyable_borrow_exit(source);
            }
            assembler.mark(target);
            assembler.set_location(SourceLocation::new(3, 3, 4, 19));
            assembler.emit_with_depth(LOAD_GLOBAL, 1, 2, ExceptionPolicy::default());
            assembler.emit_with_depth(LOAD_CONST, 0, 3, ExceptionPolicy::default());
            assembler.emit_with_depth(CALL, 1, 1, ExceptionPolicy::default());
            assembler.emit_with_depth(RAISE_VARARGS, 1, 0, ExceptionPolicy::default());
            assembler.resolve_noncopyable_borrow_exits();
            let Item::Instruction(original) =
                assembler.linear_items()[source.instruction().item_index as usize]
            else {
                unreachable!();
            };
            assert_eq!(
                original.no_location_policy.borrow_fallthrough(),
                if empty_source {
                    BorrowFallthrough::Reachable
                } else {
                    BorrowFallthrough::Barrier
                }
            );
            assert!(assembler.borrow_unreachable_blocks.is_empty());
            assert!(
                assembler
                    .exception_owner_provenance
                    .borrow_only_block_boundaries
                    .is_empty()
            );
        }
    }
}

#[test]
fn noncopyable_cleanup_borrow_exits_resolve_before_prepared_graph_reuse() {
    for empty_source in [false, true] {
        let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
        let target = assembler.label();
        let entry = assembler.label();
        assembler.mark(entry);
        let source =
            assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
        if empty_source {
            assembler.record_noncopyable_empty_borrow_exit(source);
        } else {
            assembler.record_noncopyable_borrow_exit(source);
        }
        assembler.mark(target);
        assembler.set_location(SourceLocation::new(3, 3, 4, 19));
        assembler.emit_with_depth(LOAD_GLOBAL, 1, 2, ExceptionPolicy::default());
        assembler.emit_with_depth(LOAD_CONST, 0, 3, ExceptionPolicy::default());
        assembler.emit_with_depth(CALL, 1, 1, ExceptionPolicy::default());
        assembler.emit_with_depth(RAISE_VARARGS, 1, 0, ExceptionPolicy::default());
        assert_eq!(assembler.used_constant_indices(1), [true]);
        assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
        assert!(assembler.deferred_noncopyable_borrow_exits.is_empty());
        assert_eq!(
            assembler.borrow_unreachable_blocks.contains(&entry),
            empty_source
        );
        assert_eq!(
            assembler
                .exception_owner_provenance
                .borrow_only_block_boundaries
                .contains(&target),
            empty_source
        );
        assert_eq!(assembler.used_constant_indices(1), [true]);
    }
}

#[test]
fn generator_call_join_origin_requires_an_original_intact_fallback() {
    for version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        for mutation in [
            "intact",
            "no original entry",
            "located jump",
            "no direct retention",
            "no empty join",
            "extra original slot",
            "preserved location",
            "changed first opcode",
            "changed call opcode",
            "changed call arity",
            "changed call depth",
            "unlocated call",
            "missing guard",
            "second guard",
            "fallback interior label",
            "source alias",
            "borrow alias",
            "borrow empty",
            "borrow threading denied",
            "borrow fallthrough barrier",
            "hidden owner slot",
            "explicit exception policy",
            "split return policy",
        ] {
            let mut assembler = Assembler::with_python_version(version);
            let fallback = assembler.label();
            let entry = assembler.label();
            let target = assembler.label();
            let alias = assembler.label();
            let location = SourceLocation::new(3, 3, 8, 13);
            let full_call = SourceLocation::new(3, 3, 8, 43);
            assembler.set_location(location);
            if mutation != "missing guard" {
                assembler.emit_forward_with_depth(
                    POP_JUMP_IF_FALSE,
                    fallback,
                    3,
                    ExceptionPolicy::default(),
                );
            }
            if mutation == "second guard" {
                assembler.emit_forward_with_depth(
                    POP_JUMP_IF_FALSE,
                    fallback,
                    3,
                    ExceptionPolicy::default(),
                );
            }
            assembler.emit_forward_with_depth(JUMP_FORWARD, entry, 3, ExceptionPolicy::default());
            assembler.mark(fallback);
            assembler.emit_with_depth(
                if mutation == "changed first opcode" {
                    NOP
                } else {
                    PUSH_NULL
                },
                0,
                4,
                ExceptionPolicy::default(),
            );
            assembler.emit_with_depth(LOAD_CONST, 0, 5, ExceptionPolicy::default());
            assembler.emit_with_depth(MAKE_FUNCTION, 0, 5, ExceptionPolicy::default());
            assembler.emit_with_depth(LOAD_FAST, 1, 6, ExceptionPolicy::default());
            assembler.emit_with_depth(GET_ITER, 0, 6, ExceptionPolicy::default());
            assembler.emit_with_depth(CALL, 0, 5, ExceptionPolicy::default());
            if mutation == "fallback interior label" {
                assembler.mark(alias);
            }
            assembler.set_location(if mutation == "unlocated call" {
                SourceLocation::NONE
            } else {
                full_call
            });
            assembler.emit_with_depth(
                if mutation == "changed call opcode" {
                    LOAD_FAST
                } else {
                    CALL
                },
                if mutation == "changed call arity" {
                    2
                } else {
                    1
                },
                if mutation == "changed call depth" {
                    4
                } else {
                    3
                },
                ExceptionPolicy::default(),
            );
            if mutation != "no original entry" {
                assembler.record_exception_owner_block_boundary(entry);
            }
            assembler.mark(entry);
            if mutation == "borrow alias" {
                assembler.borrow_block_aliases.insert(entry);
            }
            if mutation == "source alias" {
                assembler.mark(alias);
            }
            assembler.set_location(SourceLocation::NONE);
            let emitted = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                target,
                3,
                ExceptionPolicy::default(),
            );
            assembler.preserve_empty_join_inlined_jump_nop(emitted);
            let jump = assembler.emitted_jump_mut(emitted, "mutating generator join proof");
            if mutation == "located jump" {
                jump.location = full_call;
            }
            if matches!(mutation, "no direct retention" | "no empty join") {
                jump.jump_policy = JumpPolicy::default();
                if mutation == "no direct retention" {
                    jump.jump_policy.mark_original_empty_join();
                } else {
                    jump.jump_policy.retain_nop_for_direct_target();
                }
            }
            if mutation == "extra original slot" {
                jump.add_pre_fold_extra_instructions(1);
            }
            if mutation == "borrow empty" {
                jump.jump_policy.mark_borrow_empty_block();
            }
            if mutation == "borrow threading denied" {
                jump.jump_policy.prevent_borrow_threading_target();
            }
            if mutation == "borrow fallthrough barrier" {
                jump.no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier);
            }
            if mutation == "hidden owner slot" {
                jump.local_policy.add_hidden_exception_owner_slots(1);
            }
            if mutation == "explicit exception policy" {
                jump.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            }
            if mutation == "split return policy" {
                assembler.allow_split_return_copy(emitted);
            }
            if mutation == "preserved location" {
                assembler.preserve_no_location(emitted.instruction());
            }
            assembler.record_original_generator_call_join(emitted);
            let expected = version >= crate::PythonVersion::Py314 && mutation == "intact";
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .original_generator_call_joins
                    .len(),
                usize::from(expected),
                "{version:?}: {mutation}",
            );
            if expected {
                let (recorded_entry, original) = &assembler
                    .exception_owner_provenance
                    .original_generator_call_joins[0];
                assert_eq!(*recorded_entry, entry);
                assert_eq!(original.first(), Some(&Item::Label(fallback)));
                assert_eq!(
                    &original[original.len() - 2..],
                    &[
                        Item::Label(entry),
                        assembler.linear_items().last().copied().unwrap(),
                    ]
                );
            }
        }
    }
}

#[test]
fn generator_call_join_constant_transport_preserves_only_original_survivors() {
    for mutation in [
        "intact",
        "removed constant",
        "changed location",
        "changed policy",
    ] {
        let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
        let fallback = assembler.label();
        let entry = assembler.label();
        let target = assembler.label();
        assembler.mark(fallback);
        assembler.set_location(SourceLocation::new(3, 3, 8, 43));
        assembler.emit_with_depth(LOAD_CONST, 2, 4, ExceptionPolicy::default());
        assembler.emit_with_depth(LOAD_CONST, 3, 5, ExceptionPolicy::default());
        assembler.emit_with_depth(CALL, 1, 3, ExceptionPolicy::default());
        assembler.mark(entry);
        assembler.set_location(SourceLocation::NONE);
        let jump =
            assembler.emit_forward_with_depth(JUMP_FORWARD, target, 3, ExceptionPolicy::default());
        assembler.preserve_empty_join_inlined_jump_nop(jump);
        // Transport is tested independently from producer admission.
        let original = assembler.linear_items().to_vec();
        assembler
            .exception_owner_provenance
            .original_generator_call_joins
            .push((entry, original));
        if mutation == "changed location" {
            let Item::Instruction(first) = &mut assembler.linear_items_mut()[1] else {
                unreachable!()
            };
            first.location.end_column += 1;
        }
        if mutation == "changed policy" {
            let Item::Instruction(first) = &mut assembler.linear_items_mut()[1] else {
                unreachable!()
            };
            first.no_location_policy.preserve_location();
        }
        assembler.remap_constant_indices(&[
            Some(0),
            None,
            Some(1),
            (mutation != "removed constant").then_some(2),
        ]);
        let origins = &assembler
            .exception_owner_provenance
            .original_generator_call_joins;
        if mutation == "removed constant" {
            assert!(origins.is_empty());
        } else {
            assert_eq!(origins.len(), 1);
            let original = &origins[0].1;
            let Item::Instruction(first) = original[1] else {
                unreachable!()
            };
            let Item::Instruction(second) = original[2] else {
                unreachable!()
            };
            assert_eq!(first.operand(), Operand::Value(1));
            assert_eq!(second.operand(), Operand::Value(2));
            assert_eq!(
                original.as_slice() == assembler.linear_items(),
                mutation == "intact"
            );
        }
    }
}

#[test]
fn located_borrow_join_retention_requires_original_unique_entry() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for mutation in [
            "intact",
            "missing source head",
            "foreign source head",
            "missing labels",
            "borrow alias",
            "shared condition",
            "shared original tag",
            "live fallthrough",
            "unlocated",
            "not borrow empty",
            "no direct retention",
            "already retained",
        ] {
            let mut assembler = Assembler::with_python_version(version);
            let join = assembler.label();
            let target = assembler.label();
            let unrelated = assembler.label();
            let location = SourceLocation::new(4, 4, 7, 24);
            assembler.set_location(location);
            assembler.emit_forward_with_depth(
                POP_JUMP_IF_FALSE,
                join,
                0,
                ExceptionPolicy::default(),
            );
            if matches!(mutation, "shared condition" | "shared original tag") {
                assembler.emit_forward_with_depth(
                    POP_JUMP_IF_FALSE,
                    join,
                    0,
                    ExceptionPolicy::default(),
                );
            }
            assembler.emit_with_depth(
                if mutation == "live fallthrough" {
                    POP_TOP
                } else {
                    RETURN_VALUE
                },
                0,
                0,
                ExceptionPolicy::default(),
            );
            if mutation != "missing labels" {
                assembler.mark(join);
            }
            if mutation == "foreign source head" {
                assembler.record_exception_owner_block_boundary(unrelated);
            } else if mutation != "missing source head" {
                assembler.record_exception_owner_block_boundary(join);
            }
            if mutation == "borrow alias" {
                assembler.borrow_block_aliases.insert(join);
            }
            if mutation == "unlocated" {
                assembler.set_location(SourceLocation::NONE);
            }
            let exit = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                target,
                0,
                ExceptionPolicy::default(),
            );
            let jump = assembler.emitted_jump_mut(exit, "preparing the normal join");
            if mutation != "not borrow empty" {
                jump.jump_policy.mark_borrow_empty_block();
            }
            if mutation != "no direct retention" {
                jump.jump_policy.retain_nop_for_direct_target();
            }
            if mutation == "already retained" {
                jump.jump_policy.retain_nop();
            }
            if mutation == "shared original tag" {
                jump.jump_policy.mark_original_shared_empty_join();
            }
            let original = *jump;
            assembler.preserve_located_borrow_join_nop(exit);
            let actual = *assembler.emitted_jump_mut(exit, "checking the normal join");
            let mut expected = original;
            if mutation == "intact" {
                expected.jump_policy.retain_redundant_nop_after_threading();
            }
            assert_eq!(actual, expected, "{version:?}/{mutation}");
            assert!(!actual.jump_policy.is_original_empty_join(), "{mutation}");
        }
    }
}

#[test]
fn retained_borrow_join_revalidates_live_entries_after_original_proof() {
    let join = super::Label::from_u32(0);
    let other = super::Label::from_u32(1);
    let location = SourceLocation::new(4, 4, 7, 24);
    for mutation in [
        "intact",
        "lost entry",
        "retargeted entry",
        "new conditional entry",
        "new fallthrough",
        "unlocated",
        "barrier if removed",
        "stale original tag",
    ] {
        let mut marker = Instruction::new(NOP, Operand::Value(0), location, Some(0));
        marker.jump_policy.mark_borrow_empty_block();
        marker.jump_policy.retain_nop();
        if mutation == "unlocated" {
            marker.location = SourceLocation::NONE;
        }
        if mutation == "barrier if removed" {
            marker
                .no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved);
        }
        if mutation == "stale original tag" {
            marker.jump_policy.mark_original_empty_join();
        }
        let condition = Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(if mutation == "retargeted entry" {
                other
            } else {
                join
            }),
            location,
            Some(0),
        ));
        let mut items = Vec::new();
        if mutation != "lost entry" {
            items.push(condition);
        }
        if matches!(mutation, "new conditional entry" | "stale original tag") {
            items.push(condition);
        }
        items.push(pass_instruction(
            if mutation == "new fallthrough" {
                POP_TOP
            } else {
                RETURN_VALUE
            },
            0,
            5,
        ));
        items.push(Item::Label(join));
        items.push(Item::Instruction(marker));
        items.push(Item::Label(other));
        items.push(pass_instruction(RETURN_VALUE, 0, 6));
        super::OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
        marker.jump_policy.clear_borrow_empty_block();
        assert_eq!(
            items.contains(&Item::Instruction(marker)),
            mutation == "intact",
            "{mutation}",
        );
    }
}

#[test]
fn awaited_finally_borrow_barrier_requires_its_first_noncopyable_original_array() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for prefix_size in 0..=4 {
            for mutation in [
                "intact",
                "missing witness",
                "duplicate witness",
                "changed send location",
                "changed send depth",
                "changed send target",
                "changed await location",
                "changed constant location",
                "protocol await",
                "cold bridge",
                "no backedge",
                "other incoming edge",
                "wrong backedge",
                "source boundary",
                "duplicate original label",
                "preserved boundary",
                "borrow alias",
                "authored nop target",
                "exception target",
                "earlier label",
                "changed source",
                "not converted pop",
                "owned pop",
                "ignored slots",
                "hidden folded slots",
            ] {
                let mut assembler = Assembler::with_python_version(version);
                let target = assembler.label();
                let send_entry = assembler.label();
                let end = assembler.label();
                let other = assembler.label();
                let location = SourceLocation::new(7, 7, 8, 24);
                assembler.set_location(location);
                let source = assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    target,
                    0,
                    ExceptionPolicy::default(),
                );
                let origin = assembler.record_name_return_borrow_exit(source);
                assembler.mark(target);
                let ordinal = assembler.instruction_count();
                assembler.emit_with_depth(
                    NOP,
                    0,
                    0,
                    if mutation == "owned pop" {
                        ExceptionPolicy::default()
                    } else {
                        ExceptionPolicy::EXCLUDE_ALWAYS
                    },
                );
                if mutation != "not converted pop" {
                    assembler.mark_last_as_converted_pop_block();
                }
                for _ in 0..prefix_size {
                    assembler.emit_with_depth(LOAD_FAST, 0, 0, ExceptionPolicy::default());
                }
                if mutation == "ignored slots" {
                    for opcode in [NOP, COPY_FREE_VARS, MAKE_CELL] {
                        assembler.emit_with_depth(opcode, 0, 0, ExceptionPolicy::default());
                    }
                }
                if mutation == "earlier label" {
                    assembler.record_exception_owner_block_boundary(other);
                    assembler.mark(other);
                }
                if mutation == "changed await location" {
                    assembler.set_location(SourceLocation::NONE);
                }
                let awaitable = assembler.emit_with_depth(
                    GET_AWAITABLE,
                    u32::from(mutation == "protocol await"),
                    0,
                    ExceptionPolicy::default(),
                );
                if mutation == "hidden folded slots" {
                    let instruction = assembler
                        .emitted_instruction_mut(awaitable, "adding omitted original slots");
                    instruction.add_pre_fold_extra_instructions(4);
                    instruction.local_policy.add_hidden_exception_owner_slots(4);
                }
                assembler.set_location(if mutation == "changed constant location" {
                    SourceLocation::NONE
                } else {
                    location
                });
                assembler.emit_with_depth(LOAD_CONST, 0, 0, ExceptionPolicy::default());
                assembler.set_location(location);
                assembler.mark(send_entry);
                let send =
                    assembler.emit_forward_with_depth(SEND, end, 0, ExceptionPolicy::default());
                let original_send = *assembler.emitted_jump_mut(send, "saving the await");
                if mutation != "missing witness" {
                    for _ in 0..=usize::from(mutation == "duplicate witness") {
                        assembler
                            .exception_owner_provenance
                            .ordinary_await_ends
                            .push(super::exception_owners::OrdinaryAwaitEnd {
                                send: original_send,
                                entry: end,
                                end_send: Instruction::new(
                                    END_SEND,
                                    Operand::Value(0),
                                    location,
                                    Some(0),
                                ),
                                cleanup: Vec::new(),
                                bridge: (mutation == "cold bridge")
                                    .then_some((other, original_send)),
                            });
                    }
                }
                match mutation {
                    "changed send location" => {
                        assembler.emitted_jump_mut(send, "mutating SEND").location =
                            SourceLocation::NONE;
                    }
                    "changed send depth" => {
                        *assembler.emitted_jump_mut(send, "mutating SEND") =
                            Instruction::new(SEND, Operand::Forward(end), location, Some(1));
                    }
                    "changed send target" => assembler
                        .emitted_jump_mut(send, "mutating SEND")
                        .set_operand(Operand::Forward(other)),
                    "source boundary" => {
                        assembler
                            .exception_owner_provenance
                            .source_boundaries
                            .insert(send_entry);
                    }
                    "preserved boundary" => {
                        assembler.preserved_block_boundaries.insert(send_entry);
                    }
                    "borrow alias" => {
                        assembler.borrow_block_aliases.insert(send_entry);
                    }
                    "authored nop target" => {
                        assembler.original_nop_targets.insert(send_entry);
                    }
                    "exception target" => {
                        assembler.add_exception_region(other, end, send_entry, 0, false);
                    }
                    "changed source" => {
                        let Item::Instruction(instruction) = &mut assembler.linear_items_mut()[0]
                        else {
                            unreachable!();
                        };
                        instruction.location = SourceLocation::NONE;
                    }
                    _ => {}
                }
                assembler.emit_with_depth(YIELD_VALUE, 1, 0, ExceptionPolicy::default());
                assembler.emit_with_depth(
                    RESUME,
                    ResumeLocation::AFTER_AWAIT.argument(),
                    0,
                    ExceptionPolicy::default(),
                );
                if mutation != "no backedge" {
                    assembler.emit_backward(
                        if mutation == "wrong backedge" {
                            JUMP_BACKWARD
                        } else {
                            JUMP_BACKWARD_NO_INTERRUPT
                        },
                        send_entry,
                    );
                }
                if mutation == "other incoming edge" {
                    assembler.emit_forward_with_depth(
                        JUMP_FORWARD,
                        send_entry,
                        0,
                        ExceptionPolicy::default(),
                    );
                }
                assembler.mark(end);
                assembler.emit_with_depth(END_SEND, 0, 0, ExceptionPolicy::default());
                if mutation == "duplicate original label" {
                    assembler.mark(send_entry);
                }
                for _ in 0..8 {
                    assembler.emit_with_depth(LOAD_FAST, 0, 0, ExceptionPolicy::default());
                }
                assembler.mark_noncopyable_finally_borrow_barrier(origin, ordinal);
                let Item::Instruction(source) = assembler.linear_items()[0] else {
                    unreachable!();
                };
                assert_eq!(
                    source.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier,
                    version == crate::PythonVersion::Py314
                        && prefix_size >= 2
                        && matches!(mutation, "intact" | "ignored slots" | "hidden folded slots"),
                    "{version:?}, prefix={prefix_size}, {mutation}",
                );
            }
        }
    }
}

#[test]
fn guarded_cleanup_exit_requires_its_original_raise_edge() {
    for opcode in [POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE] {
        for mutation in [
            "intact",
            "other incoming edge",
            "backward condition",
            "condition location",
            "condition depth",
            "fallthrough arm",
            "raise depth",
            "jump location",
            "jump owner",
            "jump hidden slot",
            "jump folded slot",
            "not original",
            "borrow alias",
            "authored nop",
            "preserved boundary",
            "missing source",
            "missing block",
            "borrow marker",
        ] {
            let mut assembler = Assembler::default();
            let entry = assembler.label();
            let target = assembler.label();
            let location = SourceLocation::new(7, 7, 8, 21);
            assembler.set_location(location);
            let condition =
                assembler.emit_forward_with_depth(opcode, entry, 0, ExceptionPolicy::default());
            match mutation {
                "backward condition" => assembler
                    .emitted_jump_mut(condition, "mutating the guard")
                    .set_operand(Operand::Backward(entry)),
                "condition location" => {
                    assembler
                        .emitted_jump_mut(condition, "mutating the guard")
                        .location = SourceLocation::new(6, 6, 8, 21);
                }
                "condition depth" => {
                    *assembler.emitted_jump_mut(condition, "mutating the guard") =
                        Instruction::new(opcode, Operand::Forward(entry), location, Some(1));
                }
                _ => {}
            }
            if mutation == "other incoming edge" {
                assembler.emit_forward_with_depth(opcode, entry, 0, ExceptionPolicy::default());
            }
            assembler.emit_with_depth(
                if mutation == "fallthrough arm" {
                    POP_TOP
                } else {
                    RAISE_VARARGS
                },
                1,
                u32::from(mutation == "raise depth"),
                ExceptionPolicy::default(),
            );
            assembler.record_exception_owner_block_boundary(entry);
            assembler.mark(entry);
            let exit = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                target,
                0,
                ExceptionPolicy::default(),
            );
            assembler.preserve_redundant_jump_nop_after_threading(exit);
            match mutation {
                "jump location" => {
                    assembler
                        .emitted_jump_mut(exit, "mutating the exit")
                        .location = SourceLocation::NONE;
                }
                "jump owner" => assembler
                    .emitted_jump_mut(exit, "mutating the exit")
                    .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "jump hidden slot" => assembler
                    .emitted_jump_mut(exit, "mutating the exit")
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                "jump folded slot" => assembler
                    .emitted_jump_mut(exit, "mutating the exit")
                    .add_pre_fold_extra_instructions(1),
                "not original" => {
                    assembler
                        .emitted_jump_mut(exit, "mutating the exit")
                        .jump_policy = JumpPolicy::default();
                }
                "borrow alias" => {
                    assembler.borrow_block_aliases.insert(entry);
                }
                "authored nop" => {
                    assembler.original_nop_targets.insert(entry);
                }
                "preserved boundary" => {
                    assembler.preserved_block_boundaries.insert(entry);
                }
                "missing source" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&entry);
                }
                "missing block" => {
                    assembler
                        .exception_owner_provenance
                        .block_boundaries
                        .remove(&entry);
                }
                "borrow marker" => assembler.mark_borrow_empty_block(exit),
                _ => {}
            }
            let original = assembler.linear_items().to_vec();
            assembler.record_guarded_try_else_exit(exit);
            assert_eq!(assembler.linear_items(), original, "{opcode:?} {mutation}");
            assert_eq!(
                assembler.exception_owner_provenance.guarded_try_else_exits,
                if mutation == "intact" {
                    vec![(entry, target)]
                } else {
                    Vec::new()
                },
                "{opcode:?} {mutation}"
            );
        }
    }
}

#[test]
fn guarded_cleanup_exit_is_absorbed_only_by_its_exact_outer_pop() {
    for policy in [
        ExceptionPolicy::AlwaysUnchanged,
        ExceptionPolicy::AlwaysAbsent,
    ] {
        for mutation in [
            "intact",
            "no certificate",
            "different target",
            "different span",
            "not pop",
            "owned pop",
        ] {
            let mut assembler = Assembler::default();
            let entry = assembler.label();
            let target = assembler.label();
            let other = assembler.label();
            let location = SourceLocation::new(7, 7, 8, 21);
            assembler.set_location(location);
            assembler.emit_forward_with_depth(
                POP_JUMP_IF_FALSE,
                entry,
                0,
                ExceptionPolicy::default(),
            );
            assembler.emit_with_depth(RAISE_VARARGS, 1, 0, ExceptionPolicy::default());
            assembler.record_exception_owner_block_boundary(entry);
            assembler.mark(entry);
            let exit = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                target,
                0,
                ExceptionPolicy::default(),
            );
            assembler.preserve_redundant_jump_nop_after_threading(exit);
            if mutation != "no certificate" {
                assembler.record_guarded_try_else_exit(exit);
            }
            if mutation == "different target" {
                assembler.exception_owner_provenance.guarded_try_else_exits[0].1 = other;
            }
            assembler
                .emitted_jump_mut(exit, "finishing deferred jump threading")
                .jump_policy
                .resolve_deferred_removal();
            assembler.mark(target);
            if mutation == "different span" {
                assembler.set_location(SourceLocation::new(7, 7, 9, 21));
            }
            assembler.emit_with_depth(
                NOP,
                0,
                0,
                if mutation == "owned pop" {
                    ExceptionPolicy::NonePresent
                } else {
                    policy
                },
            );
            if mutation != "not pop" {
                assembler.mark_last_as_converted_pop_block();
            }
            assembler.set_location(SourceLocation::new(9, 9, 4, 12));
            assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
            assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
            assembler.mark(other);
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.remove_redundant_forward_jumps(true);
            let markers = pipeline.graph.iter_items().filter(|item| {
                matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP)
            }).count();
            assert_eq!(
                markers,
                if mutation == "intact" { 1 } else { 2 },
                "{policy:?} {mutation}"
            );
        }
    }
}

#[test]
fn copied_loop_handler_none_cleanup_requires_exact_records_and_boundaries() {
    for continued in [false, true] {
        for opcode in [POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE] {
            for mutation in [
                "intact",
                "wrong origin",
                "missing start",
                "missing end",
                "authored opcode",
                "authored location",
                "cleanup opcode",
                "cleanup location",
                "cleanup policy",
                "binding",
                "nonlocal load",
                "condition opcode",
                "condition target",
                "condition slots",
                "condition policy",
                "marker opcode",
                "marker operand",
                "marker location",
                "extra slots",
                "extra fold",
                "unreachable",
                "source boundary",
                "block boundary",
                "fusion boundary",
                "setup",
                "omitted nop",
                "preserved boundary",
                "incoming edge",
                "missing suffix",
                "terminal suffix",
                "duplicate target",
            ] {
                let start = super::Label::from_u32(0);
                let end = super::Label::from_u32(1);
                let target = super::Label::from_u32(2);
                let location = SourceLocation::new(5, 5, 8, 16);
                let guard = SourceLocation::new(7, 7, 7, 19);
                let mut records = [
                    Instruction::new(NOP, Operand::Value(0), location, None),
                    Instruction::new(POP_EXCEPT, Operand::Value(0), location, None),
                    Instruction::new(LOAD_CONST, Operand::Value(0), location, None),
                    Instruction::new(STORE_FAST, Operand::Value(3), location, None),
                    Instruction::new(DELETE_FAST, Operand::Value(3), location, None),
                    Instruction::new(LOAD_FAST, Operand::Value(2), guard, None),
                    Instruction::new(opcode, Operand::Forward(target), guard, None),
                    Instruction::new(NOT_TAKEN, Operand::Value(0), guard, None),
                ];
                records[6].local_policy.add_hidden_exception_owner_slots(3);
                match mutation {
                    "authored opcode" => records[0].opcode = POP_TOP,
                    "authored location" => records[0].location = SourceLocation::NONE,
                    "cleanup opcode" => records[1].opcode = POP_TOP,
                    "cleanup location" => records[3].location = guard,
                    "cleanup policy" => {
                        records[2].set_exception_policy(ExceptionPolicy::NonePresent);
                    }
                    "binding" => records[4].set_operand(Operand::Value(4)),
                    "nonlocal load" => records[5].opcode = LOAD_GLOBAL,
                    "condition opcode" => records[6].opcode = POP_JUMP_IF_FALSE,
                    "condition target" => records[6].set_operand(Operand::Backward(target)),
                    "condition slots" => records[6].local_policy = LocalPolicy::default(),
                    "condition policy" => {
                        records[6].set_exception_policy(ExceptionPolicy::NonePresent);
                    }
                    "marker opcode" => records[7].opcode = NOP,
                    "marker operand" => records[7].set_operand(Operand::Value(1)),
                    "marker location" => records[7].location = location,
                    "extra slots" => records[5].local_policy.add_hidden_exception_owner_slots(1),
                    "extra fold" => records[4].add_pre_fold_extra_instructions(1),
                    "unreachable" => records[0]
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                    _ => {}
                }
                let mut items = vec![Item::Instruction(records[0])];
                if mutation != "missing start" {
                    items.push(Item::Label(start));
                }
                items.extend(records[1..5].iter().copied().map(Item::Instruction));
                if mutation != "missing end" {
                    items.push(Item::Label(end));
                }
                items.extend(records[5..].iter().copied().map(Item::Instruction));
                items.push(Item::Label(target));
                if mutation == "duplicate target" {
                    items.push(Item::Label(target));
                }
                if mutation != "missing suffix" {
                    items.push(Item::Instruction(Instruction::new(
                        if mutation == "terminal suffix" {
                            RETURN_VALUE
                        } else {
                            LOAD_CONST
                        },
                        Operand::Value(0),
                        guard,
                        None,
                    )));
                }
                if mutation == "incoming edge" {
                    items.push(Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(start),
                        guard,
                        None,
                    )));
                }
                let mut assembler = Assembler::from_linear_items(items.clone());
                let provenance = &mut assembler.exception_owner_provenance;
                match mutation {
                    "source boundary" => {
                        provenance.source_boundaries.insert(start);
                    }
                    "block boundary" => {
                        provenance.block_boundaries.insert(start);
                    }
                    "fusion boundary" => {
                        provenance.fusion_boundaries.insert(start);
                    }
                    "setup" => {
                        provenance.setups.push((start, None));
                    }
                    "omitted nop" => {
                        provenance.omitted_nop_counts.insert(start, 1);
                    }
                    "preserved boundary" => {
                        assembler.preserved_block_boundaries.insert(start);
                    }
                    _ => {}
                }
                let origin = usize::from(mutation == "wrong origin");
                if continued {
                    assembler.record_copied_continue_none_handler_cleanup(origin);
                } else {
                    assembler.record_copied_break_handler_cleanup(origin);
                }
                let expected = mutation == "intact"
                    || (!continued
                        && matches!(
                            mutation,
                            "missing suffix" | "terminal suffix" | "duplicate target"
                        ));
                let captures = &assembler.exception_owner_provenance.copied_handler_cleanups;
                assert_eq!(
                    captures.len(),
                    usize::from(expected),
                    "{continued} {opcode:?} {mutation}"
                );
                if let [capture] = captures.as_slice() {
                    assert_eq!(capture.start, start);
                    assert_eq!(capture.end, end);
                    assert_eq!(capture.return_nop, records[0]);
                    assert_eq!(capture.instructions, records[1..5]);
                    assert_eq!(capture.missing_null_pops, 2);
                    assert_eq!(capture.removed_prefix_slots, 1);
                }
                assert_eq!(assembler.linear_items(), items.as_slice());
            }
        }
    }
}

#[test]
fn ordinary_try_fragments_retain_only_their_exact_original_setup() {
    for split in [false, true] {
        for mutation in [
            "intact",
            "missing setup",
            "different setup handler",
            "manager setup",
            "wrong origin",
            "depth at capture",
            "lasti at capture",
            "removed setup",
            "retargeted setup",
            "duplicate witness",
            "different witness setup",
            "different start",
            "different end",
            "different target",
            "fallback",
            "depth",
            "lasti",
        ] {
            let mut assembler = Assembler::default();
            let setup = assembler.label();
            let fragment = if split { assembler.label() } else { setup };
            let end = assembler.label();
            let handler = assembler.label();
            let other = assembler.label();
            if !matches!(mutation, "missing setup" | "manager setup") {
                assembler.record_ordinary_try_exception_setup(
                    setup,
                    false,
                    if mutation == "different setup handler" {
                        other
                    } else {
                        handler
                    },
                );
            } else if mutation == "manager setup" {
                assembler.record_exception_setup(setup, false);
                assembler
                    .exception_owner_provenance
                    .synchronous_with_setups
                    .insert(setup);
            }
            assembler.add_exception_region_with_origin(
                fragment,
                end,
                handler,
                u32::from(mutation == "depth at capture"),
                mutation == "lasti at capture",
                (
                    if mutation == "wrong origin" {
                        other
                    } else {
                        setup
                    },
                    end,
                ),
            );
            let mut region = assembler.exception_regions[0];
            // Optimization resolves priorities to positions; origin identity must
            // survive independently and must not be inferred from these numbers.
            region.priority = super::ExceptionRegionPriority::Resolved { start: 7, end: 19 };
            let provenance = &mut assembler.exception_owner_provenance;
            match mutation {
                "removed setup" => {
                    provenance.ordinary_try_setups.remove(&setup);
                }
                "retargeted setup" => {
                    provenance.ordinary_try_setups.insert(setup, other);
                }
                "duplicate witness" => provenance
                    .ordinary_try_region_origins
                    .push(provenance.ordinary_try_region_origins[0]),
                "different witness setup" => {
                    provenance.ordinary_try_region_origins[0].0 = other;
                }
                "different start" => region.start = other,
                "different end" => region.end = other,
                "different target" => region.target = other,
                "fallback" => region.is_scope_fallback = true,
                "depth" => region.depth = 1,
                "lasti" => region.preserve_lasti = true,
                _ => {}
            }
            assert_eq!(
                provenance.is_original_ordinary_try_region(region),
                mutation == "intact",
                "split={split}: {mutation}"
            );
        }
    }
}

#[test]
fn constant_pair_layout_preserves_only_the_swap_owner_witness() {
    for recorded in [false, true] {
        for first_local in [3, 24] {
            for mutation in [
                "intact",
                "nonconstant",
                "wrong swap",
                "nonlocal store",
                "same local",
                "unlocated",
                "other line",
                "missing fold",
                "extra fold",
                "missing hidden",
                "extra hidden",
                "owned swap",
                "borrow barrier",
                "borrow alias",
                "fusion barrier",
                "location donor",
                "no copy",
                "constant witness",
                "store witness",
                "constant hidden",
                "store fold",
                "prefix",
                "interior label",
                "suffix",
            ] {
                let location = SourceLocation::new(7, 7, 4, 28);
                let instruction = |opcode, argument, depth| {
                    Instruction::new(opcode, Operand::Value(argument), location, Some(depth))
                };
                let mut records = [
                    instruction(LOAD_CONST, 0, 1),
                    instruction(LOAD_CONST, 0, 2),
                    instruction(SWAP, 2, 2),
                    instruction(STORE_FAST, first_local, 1),
                    instruction(STORE_FAST, first_local + 1, 0),
                ];
                if mutation != "missing fold" {
                    records[2].add_pre_fold_extra_instructions(1);
                }
                if mutation != "missing hidden" {
                    records[2].local_policy.add_hidden_exception_owner_slots(1);
                }
                if recorded {
                    records[2].jump_policy.mark_original_uniform_fold();
                }
                match mutation {
                    "nonconstant" => records[0].opcode = LOAD_FAST,
                    "wrong swap" => records[2].set_operand(Operand::Value(3)),
                    "nonlocal store" => records[3].opcode = STORE_NAME,
                    "same local" => records[4].set_operand(Operand::Value(first_local)),
                    "unlocated" => records[2].location = SourceLocation::NONE,
                    "other line" => records[4].location = SourceLocation::new(8, 8, 4, 28),
                    "extra fold" => records[2].add_pre_fold_extra_instructions(1),
                    "extra hidden" => records[2].local_policy.add_hidden_exception_owner_slots(1),
                    "owned swap" => records[2].set_exception_policy(ExceptionPolicy::NonePresent),
                    "borrow barrier" => records[2]
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                    "borrow alias" => records[2].jump_policy.mark_borrow_empty_block(),
                    "fusion barrier" => records[2].fusion_policy.prevent_with_previous(),
                    "location donor" => records[2].fusion_policy.mark_location_from_next(),
                    "no copy" => records[2].jump_policy.prevent_small_exit_inlining(),
                    "constant witness" => records[0].jump_policy.mark_original_uniform_fold(),
                    "store witness" => records[3].jump_policy.mark_original_uniform_fold(),
                    "constant hidden" => {
                        records[0].local_policy.add_hidden_exception_owner_slots(1);
                    }
                    "store fold" => records[3].add_pre_fold_extra_instructions(1),
                    _ => {}
                }
                let mut items = records.map(Item::Instruction).to_vec();
                let mut assembler = Assembler::default();
                let label = assembler.label();
                match mutation {
                    "prefix" => items.insert(0, Item::Instruction(instruction(NOP, 0, 0))),
                    "interior label" => items.insert(3, Item::Label(label)),
                    "suffix" => items.push(Item::Instruction(instruction(NOP, 0, 0))),
                    _ => {}
                }
                let expected =
                    (mutation == "intact").then_some([records[2], records[3], records[4]]);
                assert_eq!(
                    super::constant_pair_stores(&items),
                    expected,
                    "{recorded} {first_local} {mutation}"
                );
                items.insert(0, Item::Label(label));
                assert_eq!(
                    super::constant_pair_store_tail(&items),
                    expected,
                    "tail {recorded} {first_local} {mutation}"
                );
            }
        }
    }
}

#[test]
fn folded_handler_break_deferral_requires_exact_original_edges() {
    for count in [1, 2] {
        for mutation in [
            "intact",
            "no anchor",
            "two anchors",
            "no source boundary",
            "no block boundary",
            "other folded target",
            "loop else",
            "authored nop",
            "hidden nop",
            "borrow alias",
            "fallthrough",
            "no edges",
            "foreign target",
            "conditional",
            "backward",
            "unlocated",
            "unknown depth",
            "different depth",
            "not authored",
            "threadable",
            "always nop",
            "no nop",
            "extra slot",
            "no slot",
            "owner",
            "preserved location",
            "hidden owner",
        ] {
            let mut assembler = Assembler::default();
            let anchor = assembler.label();
            let alias = assembler.label();
            let head = assembler.label();
            if mutation != "no anchor" {
                assembler.record_folded_loop_exit_anchor(anchor);
            }
            assembler.record_exception_owner_block_boundary(anchor);
            match mutation {
                "two anchors" => assembler.record_folded_loop_exit_anchor(alias),
                "no source boundary" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&anchor);
                }
                "no block boundary" => {
                    assembler
                        .exception_owner_provenance
                        .block_boundaries
                        .remove(&anchor);
                }
                "other folded target" => {
                    assembler.folded_away_condition_targets.insert(alias);
                }
                "loop else" => {
                    assembler.folded_away_loop_else_targets.insert(anchor);
                }
                "authored nop" => {
                    assembler.original_nop_targets.insert(anchor);
                }
                "hidden nop" => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .insert(anchor, 1);
                }
                "borrow alias" => {
                    assembler.borrow_block_aliases.insert(alias);
                }
                _ => {}
            }
            for index in 0..count {
                if mutation == "no edges" {
                    break;
                }
                let changed = index + 1 == count;
                let mut jump = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(anchor),
                    SourceLocation::new(4 + index, 4 + index, 8, 13),
                    if changed && mutation == "unknown depth" {
                        None
                    } else {
                        Some(1)
                    },
                );
                jump.fusion_policy.mark_authored_break();
                jump.jump_policy.prevent_threading_target();
                jump.jump_policy.retain_nop_for_direct_target();
                jump.add_pre_fold_extra_instructions(1);
                if changed {
                    match mutation {
                        "foreign target" => jump.set_operand(Operand::Forward(alias)),
                        "conditional" => jump.opcode = POP_JUMP_IF_FALSE,
                        "backward" => jump.set_operand(Operand::Backward(anchor)),
                        "unlocated" => jump.location = SourceLocation::NONE,
                        "different depth" => {
                            jump = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(anchor),
                                jump.location,
                                Some(2),
                            );
                            jump.fusion_policy.mark_authored_break();
                            jump.jump_policy.prevent_threading_target();
                            jump.jump_policy.retain_nop_for_direct_target();
                            jump.add_pre_fold_extra_instructions(1);
                        }
                        "not authored" => {
                            jump = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(anchor),
                                jump.location,
                                Some(1),
                            );
                            jump.jump_policy.prevent_threading_target();
                            jump.jump_policy.retain_nop_for_direct_target();
                            jump.add_pre_fold_extra_instructions(1);
                        }
                        "threadable" => jump.jump_policy = JumpPolicy::default(),
                        "always nop" => jump.jump_policy.retain_nop(),
                        "no nop" => {
                            jump.jump_policy = JumpPolicy::default();
                            jump.jump_policy.prevent_threading_target();
                        }
                        "extra slot" => jump.add_pre_fold_extra_instructions(1),
                        "no slot" => {
                            jump = Instruction::new(
                                JUMP_FORWARD,
                                Operand::Forward(anchor),
                                jump.location,
                                Some(1),
                            );
                            jump.fusion_policy.mark_authored_break();
                            jump.jump_policy.prevent_threading_target();
                            jump.jump_policy.retain_nop_for_direct_target();
                        }
                        "owner" => jump.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                        "preserved location" => jump.no_location_policy.preserve_location(),
                        "hidden owner" => jump.local_policy.add_hidden_exception_owner_slots(1),
                        _ => {}
                    }
                }
                assembler.push_linear_item(Item::Instruction(jump));
            }
            assembler.push_linear_item(Item::Instruction(Instruction::new(
                if mutation == "fallthrough" {
                    NOP
                } else {
                    JUMP_BACKWARD
                },
                Operand::Backward(head),
                SourceLocation::NONE,
                Some(1),
            )));
            assembler.mark(anchor);
            assembler.mark(alias);
            let before = assembler.linear_items().to_vec();
            assert_eq!(
                assembler.defer_original_folded_while_break_cleanup_edges(),
                mutation == "intact" || (mutation == "different depth" && count == 1),
                "{count}: {mutation}"
            );
            if mutation != "intact" && !(mutation == "different depth" && count == 1) {
                assert_eq!(assembler.linear_items(), before, "{count}: {mutation}");
            }
        }
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn instruction_items_remain_cache_compact() {
    assert_eq!(std::mem::size_of::<Option<super::Label>>(), 4);
    assert_eq!(std::mem::size_of::<Option<OperandPatchId>>(), 4);
    assert_eq!(
        std::mem::size_of::<Option<super::block_graph::BasicBlockId>>(),
        4
    );

    // NonZeroU16 policy metadata preserves a niche for Item and Option<Instruction>.
    // The full-width folded-admission handle raises the previous 36-byte budget to
    // 40 bytes (11.1% per item), keeping occurrence identity intact through exit copies.
    assert_eq!(std::mem::size_of::<Instruction>(), 40);
    assert_eq!(std::mem::size_of::<Item>(), 40);
    assert_eq!(
        std::mem::size_of::<Option<Instruction>>(),
        std::mem::size_of::<Instruction>(),
    );
}

#[test]
fn expression_join_call_footers_require_intact_records_and_depths() {
    let location = SourceLocation::new(2, 2, 4, 20);
    for arguments in [0, 1, 3] {
        for mutation in [
            "intact",
            "located pop",
            "incoming depth",
            "incoming opcode",
            "call depth",
            "call location",
            "call owner",
            "pop depth",
            "pop operand",
            "pop owner",
            "split suffix",
            "extra prefix",
            "missing boundary",
            "authored nop",
            "borrow alias",
            "hidden nop",
            "trailing alias",
        ] {
            let mut assembler = Assembler::default();
            let join = assembler.label();
            assembler.set_location(location);
            assembler.emit_forward_with_depth(
                if mutation == "incoming opcode" {
                    POP_JUMP_IF_TRUE
                } else {
                    JUMP_FORWARD
                },
                join,
                arguments + 2 + u32::from(mutation == "incoming depth"),
                ExceptionPolicy::default(),
            );
            if mutation != "missing boundary" {
                assembler.record_exception_owner_block_boundary(join);
            }
            assembler.mark(join);
            match mutation {
                "authored nop" => {
                    assembler.original_nop_targets.insert(join);
                }
                "borrow alias" => {
                    assembler.borrow_block_aliases.insert(join);
                }
                "hidden nop" => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .insert(join, 1);
                }
                _ => {}
            }
            if mutation == "extra prefix" {
                assembler.emit_with_depth(NOP, 0, arguments + 2, ExceptionPolicy::default());
            }
            assembler.set_location(if mutation == "call location" {
                SourceLocation::NONE
            } else {
                location
            });
            assembler.emit_with_depth(
                CALL,
                arguments,
                1 + u32::from(mutation == "call depth"),
                if mutation == "call owner" {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::default()
                },
            );
            if mutation == "split suffix" {
                let alias = assembler.label();
                assembler.mark(alias);
            }
            assembler.set_location(if mutation == "located pop" {
                location
            } else {
                SourceLocation::NONE
            });
            assembler.emit_with_depth(
                POP_TOP,
                u32::from(mutation == "pop operand"),
                u32::from(mutation == "pop depth"),
                if mutation == "pop owner" {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::default()
                },
            );
            if mutation == "trailing alias" {
                let alias = assembler.label();
                assembler.mark(alias);
            }
            assert_eq!(
                assembler.follows_original_join_footer(),
                matches!(mutation, "intact" | "located pop"),
                "{arguments}: {mutation}",
            );
        }
    }
}

#[test]
fn compact_instruction_metadata_keeps_block_size_and_exception_policy_independent() {
    let policies = [
        ExceptionPolicy::NoneUnchanged,
        ExceptionPolicy::NonePresent,
        ExceptionPolicy::NoneOverridePresent,
        ExceptionPolicy::NoneAbsent,
        ExceptionPolicy::IfExtendedUnchanged,
        ExceptionPolicy::IfExtendedPresent,
        ExceptionPolicy::IfExtendedOverridePresent,
        ExceptionPolicy::IfExtendedAbsent,
        ExceptionPolicy::AlwaysUnchanged,
        ExceptionPolicy::AlwaysPresent,
        ExceptionPolicy::AlwaysOverridePresent,
        ExceptionPolicy::AlwaysAbsent,
        ExceptionPolicy::NoneDynamic,
        ExceptionPolicy::IfExtendedDynamic,
        ExceptionPolicy::AlwaysDynamic,
        ExceptionPolicy::NoneDynamicCondition,
        ExceptionPolicy::IfExtendedDynamicCondition,
        ExceptionPolicy::AlwaysDynamicCondition,
        ExceptionPolicy::NoneDynamicConditionStrict,
        ExceptionPolicy::IfExtendedDynamicConditionStrict,
        ExceptionPolicy::AlwaysDynamicConditionStrict,
    ];
    for unconditional_copy in [false, true] {
        for policy in policies {
            for count in 0..=5 {
                let mut instruction =
                    Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None);
                if unconditional_copy {
                    instruction.allow_unconditional_block_inlining();
                }
                instruction.set_exception_policy(policy);
                instruction.add_pre_fold_extra_instructions(count);
                assert_eq!(instruction.exception_policy(), policy);
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(instruction.local_policy.hidden_exception_owner_slots(), 0);

                instruction.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    policy.exclusion()
                );
                assert_eq!(
                    instruction.exception_policy().owner(),
                    NormalizedExceptionOwner::OverridePresent
                );
                instruction.exclude_from_exception_table(ExceptionExclusion::Always);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    ExceptionExclusion::Always
                );
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(
                    instruction.only_allows_unconditional_inlining(),
                    unconditional_copy
                );
            }
        }
    }
}

#[test]
fn compact_initial_borrow_metadata_keeps_all_logical_fields_independent() {
    let policies = [
        ExceptionPolicy::NoneUnchanged,
        ExceptionPolicy::NonePresent,
        ExceptionPolicy::NoneOverridePresent,
        ExceptionPolicy::NoneAbsent,
        ExceptionPolicy::IfExtendedUnchanged,
        ExceptionPolicy::IfExtendedPresent,
        ExceptionPolicy::IfExtendedOverridePresent,
        ExceptionPolicy::IfExtendedAbsent,
        ExceptionPolicy::AlwaysUnchanged,
        ExceptionPolicy::AlwaysPresent,
        ExceptionPolicy::AlwaysOverridePresent,
        ExceptionPolicy::AlwaysAbsent,
        ExceptionPolicy::NoneDynamic,
        ExceptionPolicy::IfExtendedDynamic,
        ExceptionPolicy::AlwaysDynamic,
        ExceptionPolicy::NoneDynamicCondition,
        ExceptionPolicy::IfExtendedDynamicCondition,
        ExceptionPolicy::AlwaysDynamicCondition,
        ExceptionPolicy::NoneDynamicConditionStrict,
        ExceptionPolicy::IfExtendedDynamicConditionStrict,
        ExceptionPolicy::AlwaysDynamicConditionStrict,
    ];
    for unconditional_copy in [false, true] {
        for policy in policies {
            for count in 0..=5 {
                let mut instruction = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(super::Label::from_u32(u32::MAX - 1)),
                    SourceLocation::NONE,
                    None,
                );
                if unconditional_copy {
                    instruction.allow_unconditional_block_inlining();
                }
                instruction.set_exception_policy(policy);
                instruction.add_pre_fold_extra_instructions(count);
                instruction.mark_initial_borrow_jump_target(super::Label::from_u32(u32::MAX - 1));
                assert_eq!(
                    instruction.initial_borrow_jump_target(),
                    (count == 0).then_some(super::Label::from_u32(u32::MAX - 1))
                );
                assert_eq!(instruction.exception_policy(), policy);
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(instruction.local_policy.hidden_exception_owner_slots(), 0);

                instruction.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    policy.exclusion()
                );
                assert_eq!(
                    instruction.exception_policy().owner(),
                    NormalizedExceptionOwner::OverridePresent
                );
                instruction.exclude_from_exception_table(ExceptionExclusion::Always);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    ExceptionExclusion::Always
                );
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(
                    instruction.only_allows_unconditional_inlining(),
                    unconditional_copy
                );
                assert_eq!(
                    instruction.initial_borrow_jump_target(),
                    (count == 0).then_some(super::Label::from_u32(u32::MAX - 1))
                );
                instruction.add_pre_fold_extra_instructions(1);
                assert_eq!(instruction.initial_borrow_jump_target(), None);
                instruction.add_pre_fold_extra_instructions(0);
                assert_eq!(instruction.initial_borrow_jump_target(), None);
            }
        }
    }
}

#[test]
fn compact_operands_preserve_full_width_payloads() {
    let operands = [
        Operand::Value(u32::MAX),
        Operand::Deferred(OperandPatchId::from_u32(u32::MAX - 1)),
        Operand::Forward(super::Label::from_u32(u32::MAX - 1)),
        Operand::Backward(super::Label::from_u32(u32::MAX - 1)),
    ];
    let mut instruction = Instruction::new(NOP, operands[0], SourceLocation::NONE, None);
    for operand in operands {
        instruction.set_operand(operand);
        assert_eq!(instruction.operand(), operand);
    }

    let mut first = Instruction::new(NOP, operands[2], SourceLocation::NONE, None);
    first.fusion_policy.prevent_with_previous();
    let mut second = Instruction::new(NOP, operands[3], SourceLocation::NONE, None);
    second.fusion_policy.prevent_with_next();
    let fused = Instruction::fused(NOP, operands[1], first, second);
    assert_eq!(fused.operand(), operands[1]);
    assert!(fused.fusion_policy.prevents_with_previous());
    assert!(fused.fusion_policy.prevents_with_next());
}

#[test]
fn instruction_constructors_set_audited_defaults() {
    let regular = Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None);
    assert_eq!(regular.local_policy, LocalPolicy::default());
    assert!(!regular.local_policy.may_store_null());
    assert_eq!(regular.no_location_policy, NoLocationPolicy::default());
    assert_eq!(regular.fusion_policy, FusionPolicy::default());
    assert!(!regular.fusion_policy.location_from_next());
    assert!(!regular.fusion_policy.is_converted_pop_block());
    assert_eq!(regular.local_policy.hidden_exception_owner_slots(), 0);
    assert_eq!(regular.jump_policy, JumpPolicy::default());
    assert!(!regular.fusion_policy.is_authored_break());
    assert!(!regular.fusion_policy.transitive_small_exit_inlining());

    let synthetic = Instruction::synthetic(NOP, Operand::Value(0), SourceLocation::NONE, None);
    assert!(!synthetic.jump_policy.inline_small_exit());
    let mut expected_synthetic_policy = JumpPolicy::default();
    expected_synthetic_policy.prevent_small_exit_inlining();
    assert_eq!(synthetic.jump_policy, expected_synthetic_policy);
    assert!(!synthetic.fusion_policy.is_authored_break());
    assert_eq!(synthetic.exception_policy(), ExceptionPolicy::default());
    assert_eq!(
        regular.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
    assert_eq!(
        synthetic.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
}

#[test]
fn direct_value_instruction_constructor_matches_regular_instruction() {
    let location = SourceLocation::new(3, 4, 2, 8);
    let regular = Instruction::new(LOAD_CONST, Operand::Value(u32::MAX), location, Some(7));
    let direct = Instruction::value_with_depth(LOAD_CONST, u32::MAX, location, 7);

    assert_eq!(direct.opcode, regular.opcode);
    assert_eq!(direct.operand(), regular.operand());
    assert_eq!(direct.location, regular.location);
    assert_eq!(direct.depth_after(), regular.depth_after());
    assert_eq!(direct.local_policy, regular.local_policy);
    assert_eq!(direct.no_location_policy, regular.no_location_policy);
    assert_eq!(direct.fusion_policy, regular.fusion_policy);
    assert_eq!(direct.jump_policy, regular.jump_policy);
    assert_eq!(direct.exception_policy(), regular.exception_policy());
}

#[test]
fn packed_exception_policy_preserves_all_states() {
    let exclusions = [
        ExceptionExclusion::None,
        ExceptionExclusion::IfExtended,
        ExceptionExclusion::Always,
    ];
    let owners = [
        NormalizedExceptionOwner::Unchanged,
        NormalizedExceptionOwner::Present,
        NormalizedExceptionOwner::OverridePresent,
        NormalizedExceptionOwner::Absent,
    ];
    for exclusion in exclusions {
        for owner in owners {
            let mut policy = ExceptionPolicy::default();
            policy.exclude(exclusion);
            policy.set_owner(owner);
            assert_eq!(policy.exclusion(), exclusion);
            assert_eq!(policy.owner(), owner);
        }
    }
    assert_eq!(
        ExceptionPolicy::EXCLUDE_IF_EXTENDED.exclusion(),
        ExceptionExclusion::IfExtended
    );
    assert_eq!(
        ExceptionPolicy::EXCLUDE_ALWAYS.exclusion(),
        ExceptionExclusion::Always
    );
    assert_eq!(
        ExceptionPolicy::normalized_owner(true).owner(),
        NormalizedExceptionOwner::Present
    );
    assert_eq!(
        ExceptionPolicy::normalized_owner(false).owner(),
        NormalizedExceptionOwner::Absent
    );
}

#[test]
fn empty_borrow_block_mark_reports_retained_nop_controls() {
    for retention in ["none", "direct", "always", "threaded", "explicit"] {
        let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
        let target = assembler.label();
        let jump =
            assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
        let policy = &mut assembler
            .emitted_jump_mut(jump, "preparing retention")
            .jump_policy;
        match retention {
            "none" => {}
            "direct" => policy.retain_nop_for_direct_target(),
            "always" => policy.retain_nop(),
            "threaded" => policy.preserve_nop_after_threading_for_test(),
            "explicit" => policy.preserve_explicit_nop(),
            _ => unreachable!(),
        }
        let mut expected = *assembler.emitted_jump_mut(jump, "capturing retention");
        if retention == "none" {
            expected.jump_policy.mark_borrow_empty_block();
        }
        assert_eq!(
            assembler.mark_borrow_empty_block_if_unretained(jump),
            retention == "none",
            "{retention}",
        );
        assert_eq!(
            *assembler.emitted_jump_mut(jump, "checking retention"),
            expected
        );
    }
}

#[test]
fn jump_policy_defaults_are_explicit() {
    let policy = JumpPolicy::default();
    assert!(policy.inline_small_exit());
    assert_eq!(policy.retained_nop(), RetainedJumpNop::None);
    assert!(policy.allows_threading_target());
    assert!(policy.allows_borrow_threading_target());
    assert!(!policy.defers_redundant_removal());
    assert!(!policy.preserves_nop_after_threading());
}

#[test]
fn jump_policy_fusion_is_exhaustive() {
    fn policy(bits: u8, retained_nop: RetainedJumpNop) -> JumpPolicy {
        let mut policy = JumpPolicy::default();
        if bits & 1 == 0 {
            policy.prevent_small_exit_inlining();
        }
        match retained_nop {
            RetainedJumpNop::None => {}
            RetainedJumpNop::DirectTarget => policy.retain_nop_for_direct_target(),
            RetainedJumpNop::Always => policy.retain_nop(),
        }
        if bits & 2 == 0 {
            policy.prevent_threading_target();
        }
        if bits & 4 != 0 {
            policy.defer_removal();
        }
        if bits & 8 != 0 {
            policy.preserve_nop_after_threading_for_test();
        }
        if bits & 16 == 0 {
            policy.prevent_borrow_threading_target();
        }
        policy
    }

    let retained_nops = [
        RetainedJumpNop::None,
        RetainedJumpNop::DirectTarget,
        RetainedJumpNop::Always,
    ];
    for first_bits in 0..32 {
        for second_bits in 0..32 {
            for first_retained in retained_nops {
                for second_retained in retained_nops {
                    let first = policy(first_bits, first_retained);
                    let second = policy(second_bits, second_retained);
                    let expected_bits =
                        u8::from(first.inline_small_exit() && second.inline_small_exit())
                            | (u8::from(
                                first.allows_threading_target() && second.allows_threading_target(),
                            ) << 1)
                            | (u8::from(
                                first.defers_redundant_removal()
                                    || second.defers_redundant_removal(),
                            ) << 2)
                            | (u8::from(
                                first.preserves_nop_after_threading()
                                    || second.preserves_nop_after_threading(),
                            ) << 3)
                            | (u8::from(
                                first.allows_borrow_threading_target()
                                    && second.allows_borrow_threading_target(),
                            ) << 4);
                    assert_eq!(
                        JumpPolicy::fused(first, second),
                        policy(expected_bits, RetainedJumpNop::None)
                    );
                }
            }
        }
    }
}

#[test]
fn retained_jump_nop_distinguishes_direct_target_provenance() {
    let expected = super::Label::from_u32(1);
    let mut instruction = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(expected),
        SourceLocation::NONE,
        None,
    );
    instruction.jump_policy.retain_nop_for_direct_target();
    assert!(
        instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
    );

    instruction.retarget(Operand::Forward(expected));
    assert_eq!(
        instruction.jump_policy.retained_nop(),
        RetainedJumpNop::DirectTarget
    );
    instruction.retarget(Operand::Forward(super::Label::from_u32(2)));
    assert_eq!(
        instruction.jump_policy.retained_nop(),
        RetainedJumpNop::None
    );
    instruction.retarget(Operand::Forward(expected));
    assert!(
        !instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
    );

    instruction.jump_policy.retain_nop();
    instruction.retarget(Operand::Backward(expected));
    assert!(instruction.jump_policy.always_retains_nop());
    assert!(
        instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
    );
}

#[test]
fn deferred_jump_removal_promotes_post_threading_nop_retention() {
    let mut policy = JumpPolicy::default();
    policy.retain_redundant_nop_after_threading();
    assert!(policy.defers_redundant_removal());
    assert!(policy.preserves_nop_after_threading());

    policy.resolve_deferred_removal();
    assert!(!policy.defers_redundant_removal());
    assert!(!policy.preserves_nop_after_threading());
    assert_eq!(policy.retained_nop(), RetainedJumpNop::Always);
}

#[test]
fn chained_comparison_arm_nop_keeps_threading_and_original_copy_cost() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for retain in [false, true] {
            for through_join in [false, true] {
                let mut assembler = Assembler::with_python_version(version);
                let source = assembler.label();
                let join = assembler.label();
                let target = assembler.label();
                assembler.mark(source);
                assembler.set_location(SourceLocation::new(2, 2, 12, 22));
                let jump = assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    if through_join { join } else { target },
                    0,
                    ExceptionPolicy::default(),
                );
                if retain {
                    assembler.preserve_chained_comparison_arm_jump_nop(jump);
                }
                if through_join {
                    assembler.mark(join);
                    assembler.set_location(SourceLocation::NONE);
                    let jump = assembler.emit_forward_with_depth(
                        JUMP_FORWARD,
                        target,
                        0,
                        ExceptionPolicy::default(),
                    );
                    assembler.preserve_direct_inlined_jump_nop(jump);
                }
                assembler.mark(target);
                assembler.set_location(SourceLocation::new(3, 3, 15, 20));
                assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
                assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
                assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());

                let mut pipeline = assembler.into_optimization_pipeline();
                let source_block = pipeline.graph.label_block(source).unwrap();
                let target_block = pipeline.graph.label_block(target).unwrap();
                let original = *pipeline.graph[source_block].last_instruction().unwrap();
                let original_target = pipeline.graph[target_block].items().to_vec();
                assert_eq!(pipeline.thread_forward_jumps(), through_join);
                let mut expected = original;
                expected.retarget(Operand::Forward(target));
                expected.add_pre_fold_extra_instructions(u8::from(through_join));
                let actual = pipeline.graph[source_block].last_instruction().unwrap();
                assert_eq!(*actual, expected);
                assert!(!actual.jump_policy.always_retains_nop());
                assert_eq!(actual.jump_policy.retains_nop_for(actual.operand()), retain);
                assert_eq!(pipeline.graph[target_block].items(), original_target);
            }
        }
    }
}

#[test]
#[should_panic(expected = "stale emitted-instruction handle")]
fn emitted_instruction_handles_reject_stale_instructions() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    let jump =
        assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.emit(NOP, 0);
    assembler.defer_jump_removal(jump);
}

#[test]
fn emitted_instruction_handles_reject_reused_item_indices() {
    let mut assembler = Assembler::default();
    let removed = assembler.emit(NOP, 0);
    assert!(assembler.take_trailing_nop_location().is_some());
    assembler.emit(LOAD_CONST, 0);

    let mutation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assembler.prevent_instruction_fusion(removed);
    }));

    assert!(mutation.is_err());
    let Some(Item::Instruction(replacement)) = assembler.linear_items().last() else {
        panic!("replacement instruction was not emitted");
    };
    assert!(!replacement.fusion_policy.prevents_with_next());
}

#[test]
#[should_panic(
    expected = "preventing fusion with the next instruction requires a preceding instruction"
)]
fn instruction_policy_mutation_requires_a_preceding_instruction() {
    Assembler::default().prevent_last_instruction_fusion();
}

#[test]
#[should_panic(expected = "handle from a different assembler")]
fn emitted_instruction_handles_reject_a_different_assembler() {
    let mut first = Assembler::default();
    let emitted = first.emit(NOP, 0);

    let mut second = Assembler::default();
    second.emit(NOP, 0);
    second.prevent_instruction_fusion(emitted);
}

#[test]
#[should_panic(expected = "preventing jump inlining requires an emitted jump")]
fn typed_jump_policy_mutation_rejects_a_non_jump_handle() {
    let mut assembler = Assembler::default();
    let instruction = assembler.emit(NOP, 0);
    assembler.prevent_jump_inlining(EmittedJump(instruction));
}

#[test]
#[should_panic(expected = "retaining a directly inlined jump NOP requires a forward jump")]
fn direct_jump_nop_provenance_requires_a_forward_jump() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    let jump = assembler.emit_backward(JUMP_BACKWARD, target);
    assembler.preserve_direct_inlined_jump_nop(jump);
}

#[test]
fn nested_match_copy_budget_rejects_compacted_arms_atomically() {
    let [first, second, join, success, exit] = [0, 1, 2, 3, 4].map(super::Label::from_u32);
    for compacted_arm in [None, Some(0), Some(1)] {
        let mut items = Vec::new();
        for (index, entry) in [first, second].into_iter().enumerate() {
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(join),
                SourceLocation::NONE,
                Some(0),
            );
            if compacted_arm == Some(index) {
                jump.add_pre_fold_extra_instructions(1);
            }
            items.extend([Item::Label(entry), Item::Instruction(jump)]);
        }
        let mut join_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(success),
            SourceLocation::NONE,
            Some(0),
        );
        join_jump.jump_policy.mark_original_empty_join();
        items.extend([
            Item::Label(join),
            Item::Instruction(join_jump),
            Item::Label(success),
        ]);
        let mut assembler = Assembler::from_linear_items(items);
        let emitted =
            assembler.emit_forward_with_depth(JUMP_FORWARD, exit, 0, ExceptionPolicy::default());
        assembler.record_original_match_success(emitted, false);
        assembler.preserve_nested_match_exit_copy_budget(emitted);
        let entries = assembler
            .exception_owner_provenance
            .nested_match_exit_copy_origins
            .iter()
            .map(|(entry, ..)| *entry)
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            if compacted_arm.is_none() {
                vec![first, second]
            } else {
                Vec::new()
            },
            "compacted arm: {compacted_arm:?}"
        );
    }
}

#[test]
fn borrow_fallthrough_barrier_stays_on_predecessor_across_labels_and_emission() {
    let mut assembler = Assembler::default();
    let predecessor = assembler.emit(LOAD_CONST, 0);
    let boundary = assembler.label();
    assembler.mark(boundary);
    assembler.mark_borrow_fallthrough_barrier(predecessor);
    assembler.emit(LOAD_CONST, 1);

    let [
        Item::Instruction(predecessor),
        Item::Label(actual_boundary),
        Item::Instruction(next),
    ] = assembler.linear_items()
    else {
        panic!(
            "unexpected instruction stream: {:?}",
            assembler.linear_items()
        );
    };
    assert_eq!(*actual_boundary, boundary);
    assert_eq!(
        predecessor.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(
        next.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
}

#[test]
fn jump_policy_handles_remain_valid_across_trailing_labels() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    let jump =
        assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.mark(target);

    assembler.prevent_jump_inlining(jump);

    let Some(Item::Instruction(instruction)) = assembler.linear_items().first() else {
        panic!("jump was not emitted");
    };
    assert!(!instruction.jump_policy.inline_small_exit());
}

#[test]
fn value_emission_attaches_exception_policy_across_intervening_labels() {
    let mut assembler = Assembler::default();
    let normalized_owner = ExceptionPolicy::normalized_owner(false);
    assembler.emit_with_depth(NOT_TAKEN, 0, 0, normalized_owner);
    let first_boundary = assembler.label();
    assembler.mark(first_boundary);
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
    let second_boundary = assembler.label();
    assembler.mark(second_boundary);
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());

    let [
        Item::Instruction(normalized),
        Item::Label(actual_first_boundary),
        Item::Instruction(excluded),
        Item::Label(actual_second_boundary),
        Item::Instruction(default),
    ] = assembler.linear_items()
    else {
        panic!("expected three instructions separated by labels");
    };
    assert_eq!(*actual_first_boundary, first_boundary);
    assert_eq!(*actual_second_boundary, second_boundary);
    assert_eq!(normalized.exception_policy(), normalized_owner);
    assert_eq!(excluded.exception_policy(), ExceptionPolicy::EXCLUDE_ALWAYS);
    assert_eq!(default.exception_policy(), ExceptionPolicy::default());
}

#[test]
fn jump_emission_attaches_exception_policy_after_an_intervening_label() {
    let mut assembler = Assembler::default();
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
    let boundary = assembler.label();
    assembler.mark(boundary);
    let target = assembler.label();
    assembler.emit_forward_with_depth(
        JUMP_FORWARD,
        target,
        0,
        ExceptionPolicy::EXCLUDE_IF_EXTENDED,
    );

    let [
        Item::Instruction(default),
        Item::Label(actual_boundary),
        Item::Instruction(excluded),
    ] = assembler.linear_items()
    else {
        panic!("expected two instructions separated by one label");
    };
    assert_eq!(*actual_boundary, boundary);
    assert_eq!(default.exception_policy(), ExceptionPolicy::default());
    assert_eq!(
        excluded.exception_policy(),
        ExceptionPolicy::EXCLUDE_IF_EXTENDED
    );
}

#[test]
fn exception_exclusion_names_layout_policy() {
    assert!(!ExceptionExclusion::None.applies(1));
    assert!(!ExceptionExclusion::IfExtended.applies(0));
    assert!(ExceptionExclusion::IfExtended.applies(1));
    assert!(ExceptionExclusion::Always.applies(0));
}

#[test]
fn terminal_with_barrier_scan_ignores_prefix_and_labels() {
    let mut assembler = Assembler::default();
    assembler.emit(NOT_TAKEN, 0);
    let prefix_boundary = assembler.label();
    assembler.mark(prefix_boundary);
    let body_start = assembler.item_count();
    let body_boundary = assembler.label();
    assembler.mark(body_boundary);
    assert!(!assembler.contains_opcode_since_item(LOAD_FAST_AND_CLEAR, body_start));
    assert!(!assembler.contains_terminal_with_barrier_since_item(body_start));
    assembler.emit(NOP, 0);
    assert!(!assembler.contains_terminal_with_barrier_since_item(body_start));
    assert!(matches!(
        assembler.pop_linear_item(),
        Some(Item::Instruction(instruction)) if instruction.opcode == NOP
    ));
    assert!(!assembler.contains_terminal_with_barrier_since_item(body_start));

    for opcode in [LOAD_FAST_AND_CLEAR, NOT_TAKEN, FOR_ITER, PUSH_EXC_INFO] {
        let mut assembler = Assembler::default();
        assembler.emit(NOT_TAKEN, 0);
        let prefix_boundary = assembler.label();
        assembler.mark(prefix_boundary);
        let body_start = assembler.item_count();
        let body_boundary = assembler.label();
        assembler.mark(body_boundary);
        assembler.push_linear_item(Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )));
        assert!(assembler.contains_terminal_with_barrier_since_item(body_start));
        assert!(assembler.contains_opcode_since_item(opcode, body_start));
        assert!(!assembler.contains_terminal_with_barrier_since_item(assembler.item_count()));
    }
}

#[test]
fn boolean_value_marker_scan_ignores_prefix_and_stops_at_barriers() {
    let mut assembler = Assembler::default();
    assembler.emit(NOT_TAKEN, 0);
    assembler.emit(POP_TOP, 0);
    let mut scan = BooleanValueMarkerScan::new(assembler.item_count());
    assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));

    let boundary = assembler.label();
    assembler.emit(NOT_TAKEN, 0);
    assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));
    assembler.mark(boundary);
    assembler.emit(POP_TOP, 0);
    assert!(assembler.advance_boolean_value_marker_scan(&mut scan));

    for opcode in [LOAD_FAST_AND_CLEAR, FOR_ITER, PUSH_EXC_INFO] {
        let mut assembler = Assembler::default();
        let mut scan = BooleanValueMarkerScan::new(assembler.item_count());
        assembler.emit(NOT_TAKEN, 0);
        assembler.emit(POP_TOP, 0);
        assert!(assembler.advance_boolean_value_marker_scan(&mut scan));
        assembler.emit(opcode, 0);
        assembler.emit(NOT_TAKEN, 0);
        assembler.emit(POP_TOP, 0);
        assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));
        assembler.emit(NOT_TAKEN, 0);
        assembler.emit(POP_TOP, 0);
        assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));
    }
}

#[test]
fn iterator_tail_barrier_scan_ignores_prefix_and_labels() {
    let mut assembler = Assembler::default();
    assembler.emit(NOT_TAKEN, 0);
    let prefix_boundary = assembler.label();
    assembler.mark(prefix_boundary);
    let body_start = assembler.item_count();
    let body_boundary = assembler.label();
    assembler.mark(body_boundary);
    assembler.emit(NOP, 0);
    assert!(!assembler.contains_iterator_tail_barrier_since_item(body_start));

    for opcode in [NOT_TAKEN, SEND, FOR_ITER, PUSH_EXC_INFO] {
        let mut assembler = Assembler::default();
        assembler.emit(NOT_TAKEN, 0);
        let prefix_boundary = assembler.label();
        assembler.mark(prefix_boundary);
        let body_start = assembler.item_count();
        let body_boundary = assembler.label();
        assembler.mark(body_boundary);
        assembler.push_linear_item(Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )));
        assert!(assembler.contains_iterator_tail_barrier_since_item(body_start));
        assert!(!assembler.contains_iterator_tail_barrier_since_item(assembler.item_count()));
    }
}

#[test]
fn instruction_offsets_follow_label_insertions_and_trailing_removal() {
    let mut assembler = Assembler::default();
    let first = SourceLocation::new(1, 1, 0, 1);
    let second = SourceLocation::new(2, 2, 0, 1);
    let third = SourceLocation::new(3, 3, 0, 1);
    assembler.set_location(first);
    assembler.emit(LOAD_CONST, 0);
    assembler.set_location(second);
    assembler.emit(NOP, 0);

    let trailing = assembler.label();
    assembler.mark(trailing);
    let before_nop = assembler.label();
    assembler.mark_before_trailing_instructions(before_nop, 1);
    let before_labels = assembler.label();
    assembler.mark_before_trailing_labels(before_labels);

    assert_eq!(assembler.instruction_count(), 2);
    assert_eq!(assembler.instruction_opcode_at(0), Some(LOAD_CONST));
    assert_eq!(assembler.instruction_opcode_at(1), Some(NOP));
    assert_eq!(assembler.instruction_opcode_at(2), None);
    assert_eq!(assembler.instruction_opcode_at(usize::MAX), None);
    assert_eq!(assembler.instruction_location_at(0), Some(first));
    assert_eq!(assembler.instruction_location_at(1), Some(second));
    assert_eq!(assembler.instruction_location_at(2), None);
    assert_eq!(assembler.instruction_item_index(0), Some(0));
    assert_eq!(assembler.instruction_item_index(1), Some(2));
    assert_eq!(assembler.instruction_item_index(2), None);
    assembler.exclude_normalized_exception_owner_at(1);
    assert_eq!(
        assembler
            .instruction_at(1)
            .unwrap()
            .exception_policy()
            .owner(),
        NormalizedExceptionOwner::Absent
    );
    assert_eq!(
        assembler
            .instruction_at(0)
            .unwrap()
            .exception_policy()
            .owner(),
        NormalizedExceptionOwner::Unchanged
    );
    assert!(assembler.contains_opcode_since(NOP, 1));
    assert!(!assembler.contains_opcode_since(LOAD_CONST, 1));
    assert!(!assembler.contains_opcode_since(NOP, 2));
    assert_eq!(assembler.pre_fusion_instruction_count_since(1), 1);
    assert_eq!(assembler.pre_fusion_instruction_count_since(usize::MAX), 0);

    assert!(assembler.take_trailing_nop_location().is_some());
    assert_eq!(assembler.instruction_count(), 1);
    assert_eq!(assembler.instruction_opcode_at(0), Some(LOAD_CONST));
    assert_eq!(assembler.instruction_opcode_at(1), None);

    assembler.set_location(third);
    assembler.emit(RETURN_VALUE, 0);
    assert_eq!(assembler.instruction_count(), 2);
    assert_eq!(assembler.instruction_opcode_at(1), Some(RETURN_VALUE));
    assert_eq!(assembler.instruction_location_at(1), Some(third));
    assert!(assembler.contains_opcode_since(RETURN_VALUE, 1));
    assert!(!assembler.contains_opcode_since(NOP, 0));
    assert_eq!(assembler.pre_fusion_instruction_count_since(0), 2);
    assert!(assembler.is_small_linear_scope_exit_since(1));
    assert!(!assembler.is_small_linear_scope_exit_since(0));
    assert!(!assembler.is_small_linear_scope_exit_since(2));
}

#[test]
fn fallthrough_termination_accounts_for_jumps_to_any_trailing_label() {
    let mut assembler = Assembler::default();
    let success = assembler.label();
    let unused = assembler.label();
    assembler.emit_forward_with_depth(JUMP_FORWARD, success, 0, ExceptionPolicy::default());
    assembler.emit(RETURN_VALUE, 0);
    assert!(assembler.has_terminated_fallthrough());

    assembler.mark(unused);
    assert!(assembler.has_terminated_fallthrough());
    assembler.mark(success);
    assert!(assembler.at_jump_target());
    assert!(!assembler.has_terminated_fallthrough());

    assembler.emit(NOP, 0);
    let nonterminal = assembler.label();
    assembler.mark(nonterminal);
    assert!(!assembler.has_terminated_fallthrough());
}

#[test]
fn dynamic_exception_owner_tracks_fusible_loads_and_null_globals() {
    let location = SourceLocation::new(1, 1, 0, 1);

    let mut fusible = Assembler::default();
    fusible.set_location(location);
    fusible.emit(LOAD_FAST, 1);
    fusible.emit(LOAD_FAST, 2);
    assert_eq!(fusible.current_block_dynamic_exception_owner_slots(), 1);
    assert!(fusible.current_block_has_dynamic_exception_owner());

    let mut mixed = Assembler::default();
    mixed.set_location(location);
    mixed.emit(LOAD_FAST, 1);
    mixed.emit(LOAD_CONST, 0);
    assert_eq!(mixed.current_block_dynamic_exception_owner_slots(), 0);
    assert!(!mixed.current_block_has_dynamic_exception_owner());

    let mut blocked = Assembler::default();
    blocked.set_location(location);
    blocked.emit(LOAD_FAST, 1);
    blocked.prevent_last_instruction_fusion();
    blocked.emit(LOAD_FAST, 2);
    assert_eq!(blocked.current_block_dynamic_exception_owner_slots(), 0);
    assert!(!blocked.current_block_has_dynamic_exception_owner());

    let mut null_global = Assembler::default();
    null_global.emit(LOAD_GLOBAL, 1);
    assert_eq!(null_global.current_block_dynamic_exception_owner_slots(), 1);
    assert!(null_global.current_block_has_dynamic_exception_owner());

    let mut regular_global = Assembler::default();
    regular_global.emit(LOAD_GLOBAL, 0);
    assert_eq!(
        regular_global.current_block_dynamic_exception_owner_slots(),
        0
    );
    assert!(!regular_global.current_block_has_dynamic_exception_owner());

    let mut three_slots = Assembler::default();
    three_slots.set_location(location);
    three_slots.emit(STORE_FAST, 1);
    three_slots.emit(STORE_FAST, 2);
    three_slots.emit(LOAD_GLOBAL, 1);
    three_slots.emit(LOAD_GLOBAL, 3);
    assert_eq!(three_slots.current_block_dynamic_exception_owner_slots(), 3);

    let mut redundant_store = Assembler::default();
    redundant_store.set_location(location);
    redundant_store.emit(STORE_FAST, 1);
    redundant_store.emit(STORE_FAST, 1);
    assert_eq!(
        redundant_store.current_block_dynamic_exception_owner_slots(),
        0
    );

    let mut converted_pop_block = Assembler::default();
    converted_pop_block.emit(NOP, 0);
    converted_pop_block.mark_last_as_converted_pop_block();
    assert_eq!(
        converted_pop_block.current_block_dynamic_exception_owner_slots(),
        1
    );

    let mut hidden_slots = Assembler::default();
    hidden_slots.emit(LOAD_CONST, 0);
    hidden_slots.add_last_hidden_exception_owner_slots(2);
    assert_eq!(
        hidden_slots.current_block_dynamic_exception_owner_slots(),
        2
    );
    let boundary = hidden_slots.label();
    hidden_slots.mark(boundary);
    assert_eq!(
        hidden_slots.current_block_dynamic_exception_owner_slots(),
        0
    );

    let mut removable_nop = Assembler::default();
    removable_nop.set_location(SourceLocation::new(1, 1, 0, 1));
    removable_nop.emit(NOP, 0);
    removable_nop.emit(LOAD_CONST, 0);
    assert_eq!(
        removable_nop.current_block_dynamic_exception_owner_slots(),
        1
    );

    let mut retained_nop = Assembler::default();
    retained_nop.set_location(SourceLocation::new(1, 1, 0, 1));
    retained_nop.emit(NOP, 0);
    retained_nop.set_location(SourceLocation::new(2, 2, 0, 1));
    retained_nop.emit(LOAD_CONST, 0);
    assert_eq!(
        retained_nop.current_block_dynamic_exception_owner_slots(),
        0
    );
}

#[test]
fn dynamic_exception_owner_counts_only_removable_comprehension_restore_swaps() {
    for (consumer, expected_slots) in [(STORE_FAST, 1), (STORE_DEREF, 0), (CALL, 0)] {
        for restore_count in 1..=3 {
            let mut assembler = Assembler::default();
            assembler.set_location(SourceLocation::new(1, 1, 0, 1));
            assembler.emit(SWAP, restore_count + 1);
            for index in 1..=restore_count {
                if index > 1 {
                    assembler.fusion_barrier();
                }
                let restore = assembler.emit(STORE_FAST, index);
                assembler.prevent_instruction_fusion_with_previous(restore);
                assembler.prevent_last_instruction_fusion();
            }
            assembler.emit(consumer, restore_count + 1);
            assert_eq!(
                assembler.current_block_dynamic_exception_owner_slots(),
                expected_slots,
            );
        }
    }
}

#[test]
fn dynamic_exception_owner_stops_at_emitted_boundaries() {
    let mut assembler = Assembler::default();
    assembler.set_location(SourceLocation::new(1, 1, 0, 1));
    assembler.emit(LOAD_FAST, 1);
    assembler.emit(LOAD_FAST, 2);
    assert!(assembler.current_block_has_dynamic_exception_owner());

    let boundary = assembler.label();
    assembler.mark(boundary);
    assert!(!assembler.current_block_has_dynamic_exception_owner());

    assembler.emit(LOAD_GLOBAL, 1);
    assert!(assembler.current_block_has_dynamic_exception_owner());
    assembler.emit(NOT_TAKEN, 0);
    assert!(!assembler.current_block_has_dynamic_exception_owner());
}

#[test]
fn fallthrough_dynamic_exception_owner_crosses_exception_region_labels() {
    let mut hidden_slot = Assembler::default();
    hidden_slot.emit(LOAD_CONST, 0);
    hidden_slot.add_last_hidden_exception_owner_slots(1);
    let boundary = hidden_slot.label();
    hidden_slot.mark(boundary);
    assert!(!hidden_slot.current_block_has_dynamic_exception_owner());
    assert!(hidden_slot.fallthrough_has_dynamic_exception_owner());
    assert!(hidden_slot.fallthrough_has_concrete_dynamic_exception_owner());

    let mut consumed_by_prologue = Assembler::default();
    consumed_by_prologue.emit(LOAD_CONST, 0);
    consumed_by_prologue.add_last_hidden_exception_owner_slots(1);
    let boundary = consumed_by_prologue.label();
    consumed_by_prologue.mark(boundary);
    consumed_by_prologue.emit(COPY_FREE_VARS, 1);
    assert!(!consumed_by_prologue.fallthrough_has_dynamic_exception_owner());

    let mut hard_boundary = Assembler::default();
    hard_boundary.emit(LOAD_CONST, 0);
    hard_boundary.add_last_hidden_exception_owner_slots(1);
    let boundary = hard_boundary.label();
    hard_boundary.emit_forward_with_depth(
        POP_JUMP_IF_FALSE,
        boundary,
        0,
        ExceptionPolicy::default(),
    );
    hard_boundary.mark(boundary);
    assert!(!hard_boundary.fallthrough_has_dynamic_exception_owner());
}

#[test]
fn fallthrough_dynamic_exception_owner_preserves_conservative_vacancies() {
    let location = SourceLocation::new(1, 1, 0, 1);

    let mut protected_nops = Assembler::default();
    protected_nops.set_location(location);
    protected_nops.emit(NOP, 0);
    let boundary = protected_nops.label();
    protected_nops.mark(boundary);
    protected_nops.emit(NOP, 0);
    assert!(protected_nops.fallthrough_has_dynamic_exception_owner());
    assert!(protected_nops.fallthrough_has_concrete_dynamic_exception_owner());

    let mut conservative_only = Assembler::default();
    conservative_only.set_location(SourceLocation::new(1, 1, 0, 1));
    conservative_only.emit(NOP, 0);
    let boundary = conservative_only.label();
    conservative_only.mark(boundary);
    conservative_only.set_location(SourceLocation::new(2, 2, 0, 1));
    conservative_only.emit(NOP, 0);
    assert!(conservative_only.fallthrough_has_dynamic_exception_owner());
    assert!(!conservative_only.fallthrough_has_concrete_dynamic_exception_owner());
}

#[test]
fn current_block_resume_after_yield_stops_at_emitted_boundaries() {
    let mut assembler = Assembler::default();
    assembler.emit(RESUME, ResumeLocation::AFTER_YIELD.argument());
    assert!(assembler.current_block_resumes_after_yield());

    let boundary = assembler.label();
    assembler.mark(boundary);
    assert!(!assembler.current_block_resumes_after_yield());

    assembler.emit(RESUME, ResumeLocation::AFTER_YIELD_FROM.argument());
    assert!(!assembler.current_block_resumes_after_yield());

    assembler.emit(RESUME, ResumeLocation::AFTER_AWAIT.argument());
    assert!(!assembler.current_block_resumes_after_yield());

    assembler.emit(RESUME, ResumeLocation::AFTER_YIELD.argument());
    assert!(assembler.current_block_resumes_after_yield());
    assembler.emit(NOT_TAKEN, 0);
    assert!(!assembler.current_block_resumes_after_yield());
}

#[test]
fn fusion_preserves_each_typed_instruction_policy() {
    let mut first = Instruction::new(
        LOAD_FAST,
        Operand::Value(1),
        SourceLocation::new(2, 2, 0, 1),
        Some(1),
    );
    first.local_policy = LocalPolicy::for_load(true, false);
    first.local_policy.mark_maybe_null_comprehension_restore();
    first.no_location_policy.allow_block_inlining();
    first.no_location_policy.make_first_copy_interruptible();
    first.no_location_policy.mark_artificial_eval_break();
    first
        .no_location_policy
        .make_copy_inherit_shared_predecessor();
    first.no_location_policy.percent_target_location = 7;
    first.fusion_policy.mark_converted_pop_block();
    first.local_policy.add_hidden_exception_owner_slots(2);
    first.add_pre_fold_extra_instructions(2);
    first.fusion_policy.prevent_with_next();
    first.jump_policy.prevent_threading_target();
    first.jump_policy.defer_removal();
    first.jump_policy.retain_nop();
    first.jump_policy.mark_borrow_empty_block();
    first.jump_policy.preserve_explicit_nop();
    first.fusion_policy.mark_authored_break();
    first.fusion_policy.allow_transitive_small_exit_inlining();
    first.exclude_from_exception_table(ExceptionExclusion::IfExtended);
    first.set_exception_owner(NormalizedExceptionOwner::Absent);

    let mut second = Instruction::new(
        LOAD_FAST,
        Operand::Value(2),
        SourceLocation::new(2, 2, 2, 3),
        Some(2),
    );
    second.local_policy = LocalPolicy::for_load(false, true);
    second.no_location_policy.preserve_location();
    second.fusion_policy.prevent_with_previous();
    second.jump_policy.prevent_small_exit_inlining();
    second.jump_policy.retain_redundant_nop_after_threading();
    second.jump_policy.retain_nop_for_direct_target();
    second.exclude_from_exception_table(ExceptionExclusion::Always);
    second.set_exception_owner(NormalizedExceptionOwner::Present);
    second.local_policy.add_hidden_exception_owner_slots(2);
    second.add_pre_fold_extra_instructions(3);
    second
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);

    let fused = Instruction::fused(LOAD_FAST_LOAD_FAST, Operand::Value(0x12), first, second);

    let mut expected_local_policy = LocalPolicy::for_load(true, true);
    expected_local_policy.mark_maybe_null_comprehension_restore();
    expected_local_policy.add_hidden_exception_owner_slots(4);
    assert_eq!(fused.local_policy, expected_local_policy);
    assert!(fused.no_location_policy.allows_block_inlining());
    assert!(
        fused
            .no_location_policy
            .first_copy_is_interruptible_backward()
    );
    assert!(fused.no_location_policy.is_artificial_eval_break());
    assert!(fused.no_location_policy.copy_inherits_shared_predecessor());
    assert!(fused.no_location_policy.preserves_location());
    assert_eq!(fused.no_location_policy.percent_target_location, 7);
    assert!(!fused.fusion_policy.prevents_with_previous());
    assert!(!fused.fusion_policy.prevents_with_next());
    assert!(fused.fusion_policy.is_converted_pop_block());
    assert!(fused.jump_policy.preserves_explicit_nop());
    assert!(fused.fusion_policy.is_authored_break());
    assert!(fused.fusion_policy.transitive_small_exit_inlining());
    assert_eq!(fused.local_policy.hidden_exception_owner_slots(), 4);
    assert_eq!(fused.pre_fold_extra_instructions(), 4);
    assert_eq!(fused.location, first.location);
    assert_eq!(fused.depth_after(), second.depth_after());
    let mut expected_jump_policy = JumpPolicy::default();
    expected_jump_policy.prevent_small_exit_inlining();
    expected_jump_policy.prevent_threading_target();
    expected_jump_policy.retain_redundant_nop_after_threading();
    expected_jump_policy.mark_borrow_empty_block();
    expected_jump_policy.preserve_explicit_nop();
    assert_eq!(fused.jump_policy, expected_jump_policy);
    assert_eq!(
        fused.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(
        fused.exception_policy().exclusion(),
        ExceptionExclusion::Always
    );
    assert_eq!(
        fused.exception_policy().owner(),
        NormalizedExceptionOwner::Absent
    );

    let mut next_first = Instruction::new(
        LOAD_FAST,
        Operand::Value(1),
        SourceLocation::new(3, 3, 0, 1),
        None,
    );
    next_first.fusion_policy.mark_location_from_next();
    let next_second = Instruction::new(
        LOAD_FAST,
        Operand::Value(2),
        SourceLocation::new(4, 4, 0, 1),
        None,
    );
    let fused_next = Instruction::fused(
        LOAD_FAST_LOAD_FAST,
        Operand::Value(0x12),
        next_first,
        next_second,
    );
    assert_eq!(fused_next.location, next_second.location);
    assert!(!fused_next.fusion_policy.location_from_next());

    let mut first_edge = Instruction::new(LOAD_FAST, Operand::Value(1), SourceLocation::NONE, None);
    first_edge.fusion_policy.prevent_with_previous();
    first_edge
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut second_edge =
        Instruction::new(LOAD_FAST, Operand::Value(2), SourceLocation::NONE, None);
    second_edge.fusion_policy.prevent_with_next();
    let fused_edges = Instruction::fused(
        LOAD_FAST_LOAD_FAST,
        Operand::Value(0x12),
        first_edge,
        second_edge,
    );
    assert!(fused_edges.fusion_policy.prevents_with_previous());
    assert!(fused_edges.fusion_policy.prevents_with_next());
    assert_eq!(fused_edges.jump_policy, JumpPolicy::default());
    assert_eq!(
        fused_edges.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
}

#[test]
fn owned_fast_loads_use_real_opcodes_with_provenance() {
    let mut assembler = Assembler::default();
    assembler.emit_owned_fast_with_depth(3, 1);
    let Some(Item::Instruction(instruction)) = assembler.linear_items().last() else {
        panic!("owned load was not emitted");
    };
    assert_eq!(instruction.opcode, LOAD_FAST);
    assert!(instruction.local_policy.force_owned_load());
    assert!(!instruction.local_policy.strict_owned_load());
}

#[test]
fn operand_patches_survive_item_insertion() {
    let mut assembler = Assembler::default();
    let patch = assembler.emit_placeholder_with_depth(LOAD_CONST, 1);
    let preceding = assembler.label();
    assembler.mark_before_trailing_instructions(preceding, 1);

    assembler.patch_argument(patch, 37).unwrap();
    assembler.resolve_operand_patches().unwrap();

    assert!(matches!(assembler.linear_items()[0], Item::Label(label) if label == preceding));
    assert!(matches!(
        assembler.linear_items()[1],
        Item::Instruction(instruction) if instruction.operand() == Operand::Value(37)
    ));
}

#[test]
fn operand_patches_reject_duplicate_resolution() {
    let mut assembler = Assembler::default();
    let patch = assembler.emit_placeholder_with_depth(LOAD_CONST, 1);

    assembler.patch_argument(patch, 1).unwrap();
    let error = assembler.patch_argument(patch, 2).unwrap_err();

    assert!(error.to_string().contains("resolved more than once"));
}

#[test]
fn operand_patches_must_be_resolved_and_attached() {
    let mut unresolved = Assembler::default();
    unresolved.emit_placeholder_with_depth(LOAD_CONST, 1);
    let error = unresolved.resolve_operand_patches().unwrap_err();
    assert!(error.to_string().contains("was never resolved"));

    let mut detached = Assembler::default();
    let patch = detached.emit_placeholder_with_depth(LOAD_CONST, 1);
    detached.patch_argument(patch, 1).unwrap();
    detached.clear_linear_items();
    let error = detached.resolve_operand_patches().unwrap_err();
    assert!(error.to_string().contains("has no instruction"));
}

#[test]
fn operand_patches_reject_duplicate_and_unknown_attachments() {
    let mut duplicate = Assembler::default();
    let patch = duplicate.emit_placeholder_with_depth(LOAD_CONST, 1);
    duplicate.patch_argument(patch, 1).unwrap();
    let item = duplicate.linear_items()[0];
    duplicate.push_linear_item(item);
    let error = duplicate.resolve_operand_patches().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("belongs to more than one instruction")
    );

    let mut unknown = Assembler::default();
    let patch = unknown.emit_placeholder_with_depth(LOAD_CONST, 1);
    unknown.patch_argument(patch, 1).unwrap();
    let Item::Instruction(mut instruction) = unknown.linear_items()[0] else {
        unreachable!();
    };
    instruction.set_operand(Operand::Deferred(OperandPatchId::from_u32(1)));
    unknown.push_linear_item(Item::Instruction(instruction));
    let error = unknown.resolve_operand_patches().unwrap_err();
    assert!(error.to_string().contains("unknown operand patch"));
}

#[test]
fn structural_validation_reports_the_pass_and_broken_label() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(target));

    let error = assembler
        .validate_structure(AssemblerStage::ThreadForwardJumps)
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("thread_forward_jumps"));
    assert!(message.contains("unbound label"));
}

#[test]
fn structural_validation_rejects_duplicate_labels() {
    let mut assembler = Assembler::default();
    let label = assembler.label();
    assembler.mark(label);
    assembler.mark(label);

    let error = assembler
        .validate_structure(AssemblerStage::FinalLayout)
        .unwrap_err();
    assert!(error.to_string().contains("bound more than once"));
}

#[test]
fn structural_validation_rejects_a_misdirected_operand() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.mark(target);
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(target));

    let error = assembler
        .validate_structure(AssemblerStage::OptimizeBooleanConversions)
        .unwrap_err();
    assert!(error.to_string().contains("forward operand"));
}

#[test]
fn structural_validation_rejects_a_label_operand_on_a_non_jump() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.emit_operand(NOP, Operand::Forward(target));
    assembler.mark(target);

    let error = assembler
        .validate_structure(AssemblerStage::RemoveUnreachableInitial)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("label operand with non-jump opcode")
    );
}

#[test]
fn exception_regions_only_require_final_order_after_cfg_passes() {
    let mut assembler = Assembler::default();
    let end = assembler.label();
    let start = assembler.label();
    let target = assembler.label();
    assembler.mark(end);
    assembler.mark(start);
    assembler.mark(target);
    assembler.add_exception_region(start, end, target, 0, false);

    assembler
        .validate_structure(AssemblerStage::RemoveUnreachableInitial)
        .unwrap();
    let error = assembler
        .validate_structure(AssemblerStage::FinalLayout)
        .unwrap_err();
    assert!(error.to_string().contains("starts at"));
}

#[test]
fn removes_unreachable_jumps() {
    let jump = JUMP_FORWARD;
    let resume = RESUME;
    let mut assembler = Assembler::default();
    let start = assembler.label();
    let end = assembler.label();

    assembler.mark(start);
    assembler.emit(resume, 0);
    assembler.emit_operand(jump, Operand::Forward(end));
    assembler.emit_backward(jump, start);
    assembler.mark(end);

    assert_eq!(assembler.finish().unwrap(), [RESUME.code(), 0]);
}

#[test]
fn emits_extended_arguments() {
    let mut assembler = Assembler::default();
    assembler.emit(LOAD_CONST, 0x1234);
    assert_eq!(
        assembler.finish().unwrap(),
        [EXTENDED_ARG.code(), 0x12, LOAD_CONST.code(), 0x34]
    );
}

#[test]
fn target_caches_and_extended_arguments_share_the_same_layout() {
    use crate::PythonVersion::{Py314, Py315};

    for version in [Py314, Py315] {
        let mut assembler = Assembler::with_python_version(version);
        assembler.emit(RESUME, ResumeLocation::AT_FUNC_START.argument());
        assembler.emit(IMPORT_NAME, if version == Py314 { 64 } else { 256 });
        assembler.emit(RETURN_VALUE, 0);
        let expected = if version == Py314 {
            vec![
                RESUME.code(),
                0,
                IMPORT_NAME.code(),
                64,
                RETURN_VALUE.code(),
                0,
            ]
        } else {
            vec![
                RESUME.code_for(Py315),
                0,
                0,
                0,
                EXTENDED_ARG.code_for(Py315),
                1,
                IMPORT_NAME.code_for(Py315),
                0,
                RETURN_VALUE.code_for(Py315),
                0,
            ]
        };
        assert_eq!(assembler.finish().unwrap(), expected);
    }
}

#[test]
fn manager_while_break_requires_original_role_and_full_call_records() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for method in [false, true] {
            for mutation in [
                "intact",
                "no role",
                "foreign role",
                "wrong body",
                "wrong depth",
                "receiver span",
                "callee span",
                "call span",
                "call arity",
                "pop span",
                "hidden load",
                "borrow load",
                "folded call",
                "owned pop",
                "jump span",
                "not break",
                "threadable",
                "borrow jump",
                "two breaks",
                "fallthrough",
                "extra alias",
                "source boundary",
                "folded end",
                "no source",
                "late receiver",
                "late callee",
                "late call",
                "late pop",
                "late jump",
                "late no role",
                "late foreign source",
            ] {
                let mut assembler = Assembler::with_python_version(version);
                let source = assembler.label();
                let entry = assembler.label();
                let protected_end = assembler.label();
                let foreign = assembler.label();
                let receiver = SourceLocation::new(10, 10, 12, 15);
                let callee = if method {
                    SourceLocation::new(10, 10, 12, 21)
                } else {
                    receiver
                };
                let call = SourceLocation::new(10, 10, 12, if method { 23 } else { 17 });
                let mut records = [
                    Instruction::new(LOAD_FAST, Operand::Value(0), receiver, Some(3)),
                    Instruction::new(
                        if method { LOAD_ATTR } else { PUSH_NULL },
                        Operand::Value(u32::from(method)),
                        callee,
                        Some(4),
                    ),
                    Instruction::new(CALL, Operand::Value(0), call, Some(3)),
                    Instruction::new(POP_TOP, Operand::Value(0), call, Some(2)),
                    Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(entry),
                        SourceLocation::NONE,
                        Some(2),
                    ),
                ];
                records[4].fusion_policy.mark_authored_break();
                records[4].jump_policy.prevent_threading_target();
                match mutation {
                    "receiver span" => records[0].location.column += 1,
                    "callee span" => records[1].location.column += 1,
                    "call span" => records[2].location.column += 1,
                    "call arity" => records[2].set_operand(Operand::Value(1)),
                    "pop span" => records[3].location.column += 1,
                    "hidden load" => records[0].local_policy.add_hidden_exception_owner_slots(1),
                    "borrow load" => records[0].local_policy.mark_force_borrowed_load(),
                    "folded call" => records[2].add_pre_fold_extra_instructions(1),
                    "owned pop" => records[3].set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    "jump span" => records[4].location = call,
                    "not break" => records[4].fusion_policy = FusionPolicy::default(),
                    "threadable" => records[4].jump_policy = JumpPolicy::default(),
                    "borrow jump" => records[4].jump_policy.prevent_borrow_threading_target(),
                    _ => {}
                }
                if mutation != "no source" {
                    assembler.mark(source);
                }
                for record in records {
                    assembler.push_linear_item(Item::Instruction(record));
                }
                if mutation == "two breaks" {
                    assembler.push_linear_item(Item::Instruction(records[4]));
                }
                assembler.push_linear_item(Item::Instruction(Instruction::new(
                    if mutation == "fallthrough" {
                        POP_TOP
                    } else {
                        RAISE_VARARGS
                    },
                    Operand::Value(1),
                    SourceLocation::new(12, 12, 8, 26),
                    Some(2),
                )));
                if mutation == "extra alias" {
                    assembler.mark(foreign);
                }
                assembler.mark(entry);
                assembler.mark(protected_end);
                if mutation != "no role" {
                    assembler.record_original_manager_while_end(
                        if mutation == "foreign role" {
                            foreign
                        } else {
                            entry
                        },
                        usize::from(mutation == "wrong body"),
                        if mutation == "wrong depth" { 3 } else { 2 },
                        call,
                        callee,
                        receiver,
                    );
                }
                if mutation == "source boundary" {
                    assembler.record_exception_owner_block_boundary(entry);
                }
                if mutation == "folded end" {
                    assembler.folded_away_loop_else_targets.insert(entry);
                }
                let accepted = version == crate::PythonVersion::Py315
                    && (mutation == "intact" || mutation.starts_with("late "));
                assert_eq!(
                    assembler.original_manager_while_call_break_join(protected_end, 0, 2),
                    accepted.then_some(entry),
                    "{version:?} method={method} {mutation}"
                );
                if !accepted {
                    continue;
                }
                let mut pop =
                    Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(2));
                pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                pop.fusion_policy.mark_converted_pop_block();
                pop.jump_policy.preserve_explicit_nop();
                assembler.push_linear_item(Item::Instruction(pop));
                assembler.push_linear_item(Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    call,
                    Some(0),
                )));
                assembler
                    .exception_owner_provenance
                    .original_manager_pop_blocks
                    .push(entry);
                assembler.preserve_block_boundary(entry);
                if mutation == "late no role" {
                    assembler
                        .exception_owner_provenance
                        .original_manager_while_ends
                        .clear();
                }
                if mutation == "late foreign source" {
                    assembler
                        .exception_owner_provenance
                        .original_manager_pop_donors
                        .get_mut(&entry)
                        .unwrap()
                        .source = foreign;
                }
                let mut pipeline = assembler.into_optimization_pipeline();
                let source_block = pipeline.graph.label_block(source).unwrap();
                let changed = match mutation {
                    "late receiver" => Some(0),
                    "late callee" => Some(1),
                    "late call" => Some(2),
                    "late pop" => Some(3),
                    "late jump" => Some(4),
                    _ => None,
                };
                if let Some(index) = changed {
                    pipeline
                        .graph
                        .instructions_mut(source_block)
                        .nth(index)
                        .unwrap()
                        .jump_policy
                        .prevent_borrow_threading_target();
                }
                pipeline.replay_original_manager_pop_locations();
                let target = pipeline.graph.label_block(entry).unwrap();
                assert_eq!(
                    pipeline
                        .graph
                        .instructions_mut(target)
                        .next()
                        .unwrap()
                        .location,
                    if mutation == "intact" {
                        call
                    } else {
                        SourceLocation::NONE
                    },
                    "method={method} {mutation}"
                );
            }
        }
    }
}

#[test]
fn manager_populated_branch_requires_one_intact_original_donor() {
    use crate::PythonVersion::{Py314, Py315};

    for version in [Py314, Py315] {
        for mutation in [
            "intact",
            "two donors",
            "fallthrough",
            "adjacent donor",
            "jump only",
            "wrong depth",
            "wrong span",
            "unlocated",
            "same manager line",
            "hidden slots",
            "folded slots",
            "authored break",
            "converted pop",
            "marker donor",
            "missing source entry",
            "missing donor entry",
            "multiple entries",
            "folded join",
            "authored join",
            "wrong target alias",
            "outside body",
        ] {
            let mut assembler = Assembler::with_python_version(version);
            let entry = assembler.label();
            let protected_end = assembler.label();
            let source = assembler.label();
            let location = SourceLocation::new(10, 10, 12, 20);
            let mut donor = Instruction::new(STORE_FAST, Operand::Value(0), location, Some(2));
            let mut jump =
                Instruction::new(JUMP_FORWARD, Operand::Forward(entry), location, Some(2));
            match mutation {
                "wrong depth" => {
                    donor = Instruction::new(STORE_FAST, Operand::Value(0), location, Some(3));
                }
                "wrong span" => jump.location.column += 1,
                "unlocated" => donor.location = SourceLocation::NONE,
                "hidden slots" => donor.local_policy.add_hidden_exception_owner_slots(1),
                "folded slots" => jump.add_pre_fold_extra_instructions(1),
                "authored break" => jump.fusion_policy.mark_authored_break(),
                "converted pop" => donor.fusion_policy.mark_converted_pop_block(),
                "marker donor" => donor.opcode = NOT_TAKEN,
                "wrong target alias" => jump.set_operand(Operand::Forward(protected_end)),
                _ => {}
            }
            assembler.mark(source);
            if mutation != "missing donor entry" {
                assembler.record_exception_owner_block_boundary(source);
            }
            assembler.push_linear_item(Item::Instruction(donor));
            if mutation == "jump only" {
                let boundary = assembler.label();
                assembler.mark(boundary);
            }
            assembler.push_linear_item(Item::Instruction(jump));
            if mutation == "two donors" {
                assembler.push_linear_item(Item::Instruction(jump));
            }
            if mutation != "adjacent donor" {
                assembler.push_linear_item(Item::Instruction(Instruction::new(
                    if mutation == "fallthrough" {
                        POP_TOP
                    } else {
                        RETURN_VALUE
                    },
                    Operand::Value(0),
                    SourceLocation::new(12, 12, 12, 18),
                    Some(0),
                )));
            }
            assembler.mark(entry);
            assembler.mark(protected_end);
            assembler.record_exception_owner_block_boundary(entry);
            match mutation {
                "missing source entry" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&entry);
                }
                "multiple entries" => {
                    assembler.record_exception_owner_block_boundary(protected_end);
                }
                "folded join" => {
                    assembler.folded_away_condition_targets.insert(entry);
                }
                "authored join" => {
                    assembler.original_nop_targets.insert(entry);
                }
                _ => {}
            }
            let manager_location = if mutation == "same manager line" {
                location
            } else {
                SourceLocation::new(8, 8, 9, 18)
            };
            assert_eq!(
                assembler.original_manager_populated_branch_join(
                    protected_end,
                    if mutation == "outside body" { 2 } else { 0 },
                    manager_location,
                    2,
                ),
                (mutation == "intact").then_some(entry),
                "{version:?}, {mutation}"
            );
        }
    }
}

#[test]
fn manager_branch_alias_requires_two_proven_dead_original_exits() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for mutation in 0..25 {
            let mut assembler = Assembler::with_python_version(version);
            let source = assembler.label();
            let entry = assembler.label();
            let protected_end = assembler.label();
            let dead = assembler.label();
            let returned_aliases = [assembler.label(), assembler.label(), assembler.label()];
            let handler = assembler.label();
            let cleanup = assembler.label();
            let exceptional = assembler.label();
            let donor_location = SourceLocation::new(10, 10, 12, 20);
            let manager_location = SourceLocation::new(8, 8, 9, 18);
            let raise_location = SourceLocation::new(14, 14, 12, 30);
            let instruction = |opcode, operand, location, depth| {
                Item::Instruction(Instruction::new(opcode, operand, location, Some(depth)))
            };
            let mut normal = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(dead),
                manager_location,
                Some(2),
            );
            normal
                .no_location_policy
                .set_borrow_fallthrough(super::ir::BorrowFallthrough::Barrier);
            let mut items = vec![
                Item::Label(source),
                instruction(STORE_FAST, Operand::Value(0), donor_location, 2),
                instruction(JUMP_FORWARD, Operand::Forward(entry), donor_location, 2),
                instruction(RETURN_VALUE, Operand::Value(0), manager_location, 0),
                Item::Label(returned_aliases[0]),
                Item::Label(returned_aliases[1]),
                Item::Label(returned_aliases[2]),
                Item::Instruction(normal),
                Item::Label(handler),
                instruction(LOAD_CONST, Operand::Value(0), raise_location, 4),
                instruction(RAISE_VARARGS, Operand::Value(1), raise_location, 3),
                Item::Label(cleanup),
                instruction(POP_EXCEPT, Operand::Value(0), raise_location, 2),
                instruction(LOAD_CONST, Operand::Value(0), raise_location, 3),
                instruction(STORE_FAST, Operand::Value(1), raise_location, 2),
                instruction(
                    crate::target::opcodes::DELETE_FAST,
                    Operand::Value(1),
                    raise_location,
                    2,
                ),
                instruction(JUMP_FORWARD, Operand::Forward(dead), raise_location, 2),
                Item::Label(exceptional),
                instruction(RERAISE, Operand::Value(1), SourceLocation::NONE, 2),
                Item::Label(dead),
                Item::Label(entry),
                Item::Label(protected_end),
            ];
            for label in [source, entry, dead] {
                assembler.record_exception_owner_block_boundary(label);
            }
            assembler
                .exception_owner_provenance
                .return_region_reentries
                .insert(returned_aliases[0]);
            assembler
                .exception_owner_provenance
                .named_handler_cleanup_targets
                .insert(exceptional);
            assembler.original_unlocated_cleanup_targets.insert(cleanup);
            let mut region = super::ExceptionRegion {
                is_scope_fallback: false,
                priority: super::ExceptionRegionPriority::OriginalRange(handler, cleanup),
                start: handler,
                end: cleanup,
                target: exceptional,
                depth: 3,
                preserve_lasti: true,
            };
            match mutation {
                1 => items[3] = instruction(POP_TOP, Operand::Value(0), manager_location, 0),
                2 => items[3] = instruction(RETURN_VALUE, Operand::Value(0), manager_location, 1),
                3 => {
                    items[7] =
                        instruction(JUMP_FORWARD, Operand::Forward(dead), manager_location, 2);
                }
                4 => items[10] = instruction(POP_TOP, Operand::Value(0), raise_location, 3),
                5 => items[14] = instruction(STORE_FAST, Operand::Value(2), raise_location, 2),
                6 => {
                    items[16] =
                        instruction(JUMP_FORWARD, Operand::Forward(entry), raise_location, 2);
                }
                7 => {
                    assembler.original_unlocated_cleanup_targets.clear();
                }
                8 => {
                    assembler
                        .exception_owner_provenance
                        .return_region_reentries
                        .clear();
                }
                9 => {
                    assembler
                        .exception_owner_provenance
                        .named_handler_cleanup_targets
                        .clear();
                }
                10 => region.target = returned_aliases[0],
                11 => region.depth = 2,
                12 => region.preserve_lasti = false,
                13 => region.is_scope_fallback = true,
                14 => {
                    region.priority =
                        super::ExceptionRegionPriority::OriginalRange(source, cleanup);
                }
                15 => {
                    assembler.preserved_block_boundaries.insert(cleanup);
                }
                16 => {
                    assembler.record_exception_owner_block_boundary(returned_aliases[1]);
                }
                17 => {
                    assembler.folded_away_condition_targets.insert(cleanup);
                }
                18 => {
                    assembler.original_nop_targets.insert(returned_aliases[0]);
                }
                19 => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .insert(cleanup, 1);
                }
                20 => {
                    items[9] = instruction(
                        JUMP_FORWARD,
                        Operand::Forward(returned_aliases[0]),
                        raise_location,
                        4,
                    );
                }
                21 => {
                    items[9] =
                        instruction(JUMP_FORWARD, Operand::Forward(cleanup), raise_location, 4);
                }
                22 => {
                    let Item::Instruction(store) = &mut items[14] else {
                        unreachable!()
                    };
                    store.local_policy.add_hidden_exception_owner_slots(1);
                }
                23 => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&dead);
                }
                24 => items[1] = instruction(STORE_FAST, Operand::Value(0), donor_location, 3),
                _ => {}
            }
            assembler.exception_regions.push(region);
            for item in items {
                assembler.push_linear_item(item);
            }
            assert_eq!(
                assembler.original_manager_populated_branch_join(
                    protected_end,
                    0,
                    manager_location,
                    2
                ),
                (mutation == 0).then_some(entry),
                "{version:?}, mutation {mutation}",
            );
        }
    }
}

#[test]
fn completed_cold_manager_join_requires_original_footer_and_single_boundary() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for mutation in 0..9 {
            let mut assembler = Assembler::with_python_version(version);
            let entry = assembler.label();
            let protected_end = assembler.label();
            let mut copy = Instruction::new(COPY, Operand::Value(3), SourceLocation::NONE, Some(6));
            let mut pop =
                Instruction::new(POP_EXCEPT, Operand::Value(0), SourceLocation::NONE, Some(5));
            let mut reraise =
                Instruction::new(RERAISE, Operand::Value(1), SourceLocation::NONE, Some(2));
            match mutation {
                1 => copy.set_operand(Operand::Value(2)),
                2 => pop.opcode = POP_TOP,
                3 => reraise.set_operand(Operand::Value(0)),
                4 => copy.location = SourceLocation::new(4, 4, 8, 16),
                5 => pop.add_pre_fold_extra_instructions(1),
                _ => {}
            }
            for record in [copy, pop, reraise] {
                assembler.push_linear_item(Item::Instruction(record));
            }
            assembler.mark(entry);
            assembler.record_exception_owner_block_boundary(entry);
            if mutation == 6 {
                let alias = assembler.label();
                assembler.mark(alias);
                assembler.record_exception_owner_block_boundary(alias);
            }
            if mutation != 7 {
                assembler.mark(protected_end);
            }
            assert_eq!(
                assembler.original_manager_cleanup_join(protected_end, mutation != 8),
                (mutation == 0).then_some(entry),
                "{version:?}: {mutation}",
            );
        }
    }
}

#[test]
fn manager_finally_break_donor_requires_every_target_iterator_pop() {
    use crate::PythonVersion::{Py314, Py315};

    for (version, slots) in [(Py314, 1), (Py315, 2)] {
        for mutation in ["intact", "missing pop", "wrong depth", "located pop"] {
            let mut assembler = Assembler::with_python_version(version);
            let entry = assembler.label();
            let protected_end = assembler.label();
            let location = SourceLocation::new(10, 10, 16, 32);
            let mut items = vec![Item::Instruction(Instruction::new(
                STORE_FAST,
                Operand::Value(3),
                location,
                Some(2 + slots),
            ))];
            for index in 0..slots {
                items.push(Item::Instruction(Instruction::new(
                    POP_TOP,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    Some(1 + slots - index),
                )));
            }
            match mutation {
                "missing pop" => {
                    items.pop();
                }
                "wrong depth" | "located pop" => {
                    let Item::Instruction(pop) = &mut items[1] else {
                        unreachable!();
                    };
                    if mutation == "wrong depth" {
                        *pop = Instruction::new(
                            POP_TOP,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(2 + slots),
                        );
                    } else {
                        pop.location = location;
                    }
                }
                _ => {}
            }
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(entry),
                SourceLocation::NONE,
                Some(2),
            );
            jump.fusion_policy.mark_authored_break();
            items.extend([
                Item::Instruction(jump),
                Item::Instruction(Instruction::new(
                    RAISE_VARARGS,
                    Operand::Value(1),
                    SourceLocation::new(12, 12, 12, 28),
                    Some(2),
                )),
                Item::Label(entry),
                Item::Label(protected_end),
            ]);
            for item in items {
                assembler.push_linear_item(item);
            }
            assembler.record_exception_owner_block_boundary(entry);
            assert_eq!(
                assembler.original_manager_copied_finally_break_join(protected_end),
                (mutation == "intact").then_some((entry, location)),
                "{version:?}, {mutation}"
            );
        }
    }
}

#[test]
fn iterator_conditional_return_counts_the_original_try_unwind_slot() {
    use crate::code_object::Constant;
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    let source = "def choose(items, value, flag):\n    for item in items:\n        try:\n            return value if flag else {}\n        except ValueError:\n            pass\n";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let module = compile_with_options(
            source,
            "iterator_return.py",
            &CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        let code = module
            .code
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) => Some(code),
                _ => None,
            })
            .unwrap();
        let returns = code
            .bytecode
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|instruction| instruction[0] == RETURN_VALUE.code_for(version))
            .count();
        // CPython's four-slot copy limit includes the erased POP_BLOCK. The
        // second iterator slot makes 3.15's conditional cleanup too large to copy.
        assert_eq!(
            returns,
            if version == PythonVersion::Py315 {
                2
            } else {
                3
            }
        );
        let borrowed_value = [LOAD_FAST_BORROW.code_for(version), 1];
        assert_eq!(
            code.bytecode.as_chunks::<2>().0.contains(&borrowed_value),
            version == PythonVersion::Py314
        );
    }
}

#[test]
fn builtin_iterable_return_retains_the_original_try_unwind_slot() {
    use crate::code_object::Constant;
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    for callable in ["tuple", "list"] {
        let source = format!(
            "def choose(items, values):\n    for item in items:\n        try:\n            return {callable}(value for value in values)\n        except ValueError:\n            pass\n"
        );
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            let module = compile_with_options(
                &source,
                "builtin_iterator_return.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let code = module
                .code
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            let returns = code
                .bytecode
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|instruction| instruction[0] == RETURN_VALUE.code_for(version))
                .count();
            // The located optimized-path jump and its normal-call fallback join
            // before the try POP_BLOCK and the iterator's shared return cleanup.
            assert_eq!(
                returns,
                if version == PythonVersion::Py314 && callable == "tuple" {
                    3
                } else {
                    2
                },
                "{version:?}: {callable}"
            );
        }
    }
}

#[test]
fn python315_resume_entry_and_depth_have_distinct_encodings() {
    use crate::PythonVersion::Py315;
    use crate::target::GENEXPR_RESUME_ARGUMENT;

    for (argument, expected) in [
        (ResumeLocation::AFTER_YIELD.at_depth(true), 9),
        (ResumeLocation::AFTER_AWAIT.at_depth(true), 11),
        (GENEXPR_RESUME_ARGUMENT, 4),
    ] {
        let mut assembler = Assembler::with_python_version(Py315);
        assembler.emit(RESUME, argument);
        assert_eq!(
            assembler.finish().unwrap(),
            [RESUME.code_for(Py315), expected, 0, 0]
        );
    }
}

#[test]
fn python315_yield_from_uses_quickened_get_iter_and_cache() {
    use crate::PythonVersion::Py315;

    let mut assembler = Assembler::with_python_version(Py315);
    assembler.emit(GET_YIELD_FROM_ITER, 0);
    assert_eq!(
        assembler.finish().unwrap(),
        [GET_ITER.code_for(Py315), 3, 0, 0]
    );
}

#[test]
fn python315_borrow_traversal_crosses_removed_constant_branches() {
    use crate::PythonVersion::{Py314, Py315};
    use crate::code_object::Constant;
    use crate::{CompileOptions, compile_with_options};

    for source in [
        "def f(x):\n    if False:\n        pass\n    return x\n",
        "def f(x):\n    while False:\n        pass\n    return x\n",
        "def f(x):\n    if flag:\n        return\n    assert False, x.value\n",
        "def f(x):\n    if flag:\n        pass\n    else:\n        assert False, f'bad: {x}'\n",
    ] {
        for version in [Py314, Py315] {
            let module = compile_with_options(
                source,
                "borrow.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let code = module
                .code
                .constants
                .iter()
                .find_map(|constant| {
                    if let Constant::Code(code) = constant {
                        Some(code)
                    } else {
                        None
                    }
                })
                .unwrap();
            let expected = if version == Py314 {
                LOAD_FAST
            } else {
                LOAD_FAST_BORROW
            };
            assert!(
                code.bytecode
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|word| *word == [expected.code_for(version), 0]),
                "{version:?}: {source}"
            );
        }
    }
}

#[test]
fn terminal_assignment_fallthrough_keeps_value_owned() {
    use crate::code_object::Constant;
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let module = compile_with_options(
            "def f(self, value):\n    self.value = {} if value is None else value\n",
            "tail.py",
            &CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        let code = module
            .code
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) => Some(code),
                _ => None,
            })
            .unwrap();
        assert!(code.bytecode.windows(6).any(|window| {
            window
                == [
                    LOAD_FAST.code_for(version),
                    1,
                    LOAD_FAST_BORROW.code_for(version),
                    0,
                    STORE_ATTR.code_for(version),
                    0,
                ]
        }));
    }
}

#[test]
fn emits_full_width_extended_arguments_and_zeroed_inline_caches() {
    let mut assembler = Assembler::default();
    assembler.emit(LOAD_GLOBAL, 0x1234_5678);
    assert_eq!(
        assembler.finish().unwrap(),
        [
            EXTENDED_ARG.code(),
            0x12,
            EXTENDED_ARG.code(),
            0x34,
            EXTENDED_ARG.code(),
            0x56,
            LOAD_GLOBAL.code(),
            0x78,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]
    );
}

#[test]
fn fused_final_artifacts_preserve_cache_extended_argument_location_and_depth_semantics() {
    let first_location = SourceLocation::new(7, 7, 0, 1);
    let second_location = SourceLocation::new(8, 8, 2, 4);
    let assembler = super::LayoutAssembler {
        python_version: crate::PythonVersion::Py314,
        legacy_uniform_jump_seed: false,
        items: vec![
            Item::Instruction(Instruction::new(
                LOAD_GLOBAL,
                Operand::Value(0x1234_5678),
                first_location,
                Some(3),
            )),
            Item::Label(super::Label::from_u32(0)),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(0x1234),
                first_location,
                None,
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(2),
                second_location,
                Some(12),
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(3),
                SourceLocation::NONE,
                Some(5),
            )),
        ],
        exception_regions: Vec::new(),
        exception_owner_provenance: Box::default(),
        preserved_block_boundaries: rustc_hash::FxHashSet::default(),
        removed_max_depth: None,
        next_label: 1,
        has_next_location: false,
    };

    let (bytecode, line_table, max_depth) =
        assembler.encode_final_artifacts(&[3, 1, 0, 0], &[0x1234_5678, 0x1234, 2, 3], 12, 7);

    assert_eq!(
        bytecode,
        [
            EXTENDED_ARG.code(),
            0x12,
            EXTENDED_ARG.code(),
            0x34,
            EXTENDED_ARG.code(),
            0x56,
            LOAD_GLOBAL.code(),
            0x78,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            EXTENDED_ARG.code(),
            0x12,
            LOAD_CONST.code(),
            0x34,
            LOAD_CONST.code(),
            2,
            LOAD_CONST.code(),
            3,
        ]
    );
    assert_eq!(line_table, [0x87, 0x01, 0x81, 0x01, 0xd8, 2, 4, 0xf8]);
    assert_eq!(max_depth, 12);
}

#[test]
fn unextended_final_artifacts_preserve_jump_cache_location_and_depth_semantics() {
    let first_location = SourceLocation::new(7, 7, 0, 1);
    let second_location = SourceLocation::new(8, 8, 2, 4);
    let target = super::Label::from_u32(0);
    let assembler = super::LayoutAssembler {
        python_version: crate::PythonVersion::Py314,
        legacy_uniform_jump_seed: false,
        items: vec![
            Item::Instruction(Instruction::new(
                LOAD_GLOBAL,
                Operand::Value(7),
                first_location,
                Some(3),
            )),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(target),
                first_location,
                Some(2),
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(9),
                second_location,
                Some(12),
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(10),
                SourceLocation::NONE,
                Some(5),
            )),
            Item::Label(target),
            Item::Instruction(Instruction::new(
                JUMP_BACKWARD,
                Operand::Backward(target),
                second_location,
                Some(1),
            )),
        ],
        exception_regions: Vec::new(),
        exception_owner_provenance: Box::default(),
        preserved_block_boundaries: rustc_hash::FxHashSet::default(),
        removed_max_depth: None,
        next_label: 1,
        has_next_location: false,
    };
    let mut labels = vec![None];
    let code_units = assembler.label_positions(&[0; 5], &mut labels);
    let expected = assembler.encode_final_artifacts(&[0; 5], &[7, 2, 9, 10, 2], code_units, 7);

    let actual = assembler
        .encode_unextended_final_artifacts(&labels, code_units, 7)
        .unwrap()
        .expect("all arguments fit in one code unit");

    assert_eq!(actual, expected);
}

#[test]
fn extended_forward_jump_uses_the_iterative_layout_fallback() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.emit(LOAD_CONST, 0);
    assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, target, 0, ExceptionPolicy::default());
    for _ in 0..=u8::MAX {
        assembler.emit(RESUME, ResumeLocation::AT_FUNC_START.argument());
    }
    assembler.mark(target);
    assembler.emit(RETURN_VALUE, 0);

    let bytecode = assembler.finish().unwrap();

    assert_eq!(
        &bytecode[..6],
        [
            LOAD_CONST.code(),
            0,
            EXTENDED_ARG.code(),
            1,
            POP_JUMP_IF_FALSE.code(),
            1,
        ]
    );
}

#[test]
fn removes_an_overwritten_local_store() {
    let mut assembler = Assembler::default();
    assembler.emit(STORE_FAST, 1);
    assembler.emit(STORE_FAST, 1);
    assert_eq!(
        assembler.finish().unwrap(),
        [POP_TOP.code(), 0, STORE_FAST.code(), 1]
    );
}

#[test]
fn constant_pop_candidates_are_recorded_during_emission() {
    let mut separated = Assembler::default();
    separated.emit(LOAD_CONST, 0);
    let boundary = separated.label();
    separated.mark(boundary);
    separated.emit(POP_TOP, 0);
    assert!(!separated.has_constant_pop_candidate);
    separated.optimize_constant_pops();
    assert!(matches!(
        separated.linear_items(),
        [Item::Instruction(load), Item::Label(_), Item::Instruction(pop)]
            if load.opcode == LOAD_CONST && pop.opcode == POP_TOP
    ));

    let mut direct = Assembler::default();
    direct.emit(
        LOAD_COMMON_CONSTANT,
        CommonConstant::ASSERTIONERROR.argument(),
    );
    direct.emit(POP_TOP, 0);
    assert!(direct.has_constant_pop_candidate);
    direct.optimize_constant_pops();
    assert!(matches!(
        direct.linear_items(),
        [Item::Instruction(first), Item::Instruction(second)]
            if first.opcode == NOP && second.opcode == NOP
    ));

    let mut forwarded = Assembler::default();
    let target = forwarded.label();
    forwarded.emit(LOAD_SMALL_INT, 0);
    forwarded.emit_forward_with_depth(JUMP_FORWARD, target, 1, ExceptionPolicy::default());
    forwarded.mark(target);
    forwarded.emit(POP_TOP, 0);
    forwarded.emit(LOAD_CONST, 1);
    forwarded.emit(RETURN_VALUE, 0);
    assert!(forwarded.has_constant_pop_candidate);
    forwarded.optimize_constant_pops();
    assert!(matches!(
        forwarded.linear_items()[..2],
        [Item::Instruction(load), Item::Instruction(return_value)]
            if load.opcode == LOAD_CONST
                && matches!(load.operand(), Operand::Value(1))
                && return_value.opcode == RETURN_VALUE
    ));
}

#[test]
fn discarded_constant_precedes_late_sequence_admission() {
    use crate::code_object::Constant;
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    let values = (0..31)
        .map(|index| format!("'item_{index}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("()\nfor item in [{values}]:\n    use(item)\nother = (1, 2)\nf(**kw)\n");
    let module = compile_with_options(
        &source,
        "discarded_before_sequence.py",
        &CompileOptions {
            python_version: PythonVersion::Py315,
            ..CompileOptions::default()
        },
    )
    .unwrap();
    assert!(matches!(
        module.code.constants.as_slice(),
        [Constant::String(first), Constant::Tuple(empty), Constant::Tuple(sequence), Constant::Tuple(pair)]
            if first == "item_0"
                && empty.is_empty()
                && sequence.len() == 31
                && matches!(pair.as_slice(), [Constant::Int(1), Constant::Int(2)])
    ));
}

#[test]
fn discarded_value_emission_matches_handle_emission() {
    fn assert_same_stream(expected: &Assembler, actual: &Assembler) {
        assert_eq!(expected.instruction_count(), actual.instruction_count());
        assert_eq!(
            expected.has_constant_pop_candidate,
            actual.has_constant_pop_candidate
        );
        assert_eq!(expected.linear_items().len(), actual.linear_items().len());
        for (expected, actual) in expected.linear_items().iter().zip(actual.linear_items()) {
            let (Item::Instruction(expected), Item::Instruction(actual)) = (expected, actual)
            else {
                panic!("emission streams should contain only instructions");
            };
            assert_eq!(actual.opcode, expected.opcode);
            assert_eq!(actual.operand(), expected.operand());
            assert_eq!(actual.location, expected.location);
            assert_eq!(actual.depth_after(), expected.depth_after());
            assert_eq!(actual.local_policy, expected.local_policy);
            assert_eq!(actual.no_location_policy, expected.no_location_policy);
            assert_eq!(actual.fusion_policy, expected.fusion_policy);
            assert_eq!(actual.jump_policy, expected.jump_policy);
            assert_eq!(actual.exception_policy(), expected.exception_policy());
        }
    }

    let location = SourceLocation::new(2, 2, 4, 12);
    for constant in [LOAD_CONST, LOAD_COMMON_CONSTANT, LOAD_SMALL_INT] {
        let mut expected = Assembler::default();
        let mut actual = Assembler::default();
        expected.set_location(location);
        actual.set_location(location);
        expected.emit_with_depth(constant, 3, 1, ExceptionPolicy::default());
        actual.emit_value_with_depth_discard(constant, 3, 1, ExceptionPolicy::default());
        expected.emit_with_depth(POP_TOP, 0, 0, ExceptionPolicy::default());
        actual.emit_value_with_depth_discard(POP_TOP, 0, 0, ExceptionPolicy::default());

        assert!(actual.has_constant_pop_candidate);
        assert_same_stream(&expected, &actual);
    }

    for (disable_borrowing, strict_owned, forced_borrowed) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut expected = Assembler::default();
        let mut actual = Assembler::default();
        expected.set_location(location);
        actual.set_location(location);
        if disable_borrowing {
            expected.disable_load_fast_borrowing();
            actual.disable_load_fast_borrowing();
        }
        expected.set_strict_owned_loads(strict_owned);
        actual.set_strict_owned_loads(strict_owned);
        expected.set_forced_borrowed_loads(forced_borrowed);
        actual.set_forced_borrowed_loads(forced_borrowed);
        expected.emit_with_depth(LOAD_FAST, 2, 1, ExceptionPolicy::default());
        actual.emit_value_with_depth_discard(LOAD_FAST, 2, 1, ExceptionPolicy::default());

        assert_same_stream(&expected, &actual);
    }

    let mut expected = Assembler::default();
    let mut actual = Assembler::default();
    expected.set_location(location);
    actual.set_location(location);
    expected.emit_with_depth(LOAD_CONST, 2, 1, ExceptionPolicy::EXCLUDE_ALWAYS);
    actual.emit_value_with_depth_discard(LOAD_CONST, 2, 1, ExceptionPolicy::EXCLUDE_ALWAYS);
    assert_same_stream(&expected, &actual);
}

fn assembler_with_preserved_constant_boundary() -> Assembler {
    let entry = super::Label::from_u32(0);
    let preserved = super::Label::from_u32(1);
    let target = super::Label::from_u32(2);
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Label(entry),
        Item::Label(preserved),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(LOAD_CONST, 1, 1),
        Item::Label(target),
        pass_instruction(LOAD_CONST, 2, 2),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);
    assembler.next_label = 3;
    assembler.preserved_block_boundaries = rustc_hash::FxHashSet::from_iter([preserved]);
    assembler
}

#[test]
fn constant_reachability_prepares_the_optimization_graph() {
    let expected = assembler_with_preserved_constant_boundary()
        .finish()
        .unwrap();
    let mut assembler = assembler_with_preserved_constant_boundary();

    assert_eq!(assembler.used_constant_indices(3), [true, false, true]);
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    assembler.remap_constant_indices(&[Some(0), Some(1), Some(2)]);

    assert_eq!(assembler.finish().unwrap(), expected);
}

#[test]
fn prepared_reachability_survives_preserved_boundary_removal() {
    let mut assembler = assembler_with_preserved_constant_boundary();
    assert_eq!(assembler.used_constant_indices(3), [true, false, true]);
    let UnoptimizedCode::Prepared { reachable, .. } = &assembler.code else {
        panic!("constant reachability should prepare the optimization graph");
    };
    assert_eq!(reachable, &[true, true, false, true]);

    let mut pipeline = assembler.into_optimization_pipeline();
    assert_eq!(
        pipeline.initial_reachable.as_deref(),
        Some([true, false, true].as_slice())
    );
    assert_eq!(pipeline.remove_unreachable_instructions(), None);
    assert!(pipeline.initial_reachable.is_none());
    assert_eq!(pipeline.graph.len(), 2);
}

#[test]
fn straight_line_constant_reachability_uses_dense_indices() {
    let mut assembler = Assembler::from_linear_items(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        pass_instruction(LOAD_CONST, 2, 1),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert_eq!(assembler.used_constant_indices(3), [true, false, true]);
}

#[test]
fn assertion_constant_reachability_keeps_the_optimization_stream_linear() {
    let target = super::Label::from_u32(0);
    let assembler = || {
        let mut assembler = Assembler::from_linear_items(vec![
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_TRUE,
                Operand::Forward(target),
                SourceLocation::new(1, 1, 0, 0),
                None,
            )),
            pass_instruction(NOT_TAKEN, 0, 1),
            pass_instruction(
                LOAD_COMMON_CONSTANT,
                CommonConstant::ASSERTIONERROR.argument(),
                1,
            ),
            pass_instruction(RAISE_VARARGS, 1, 1),
            Item::Label(target),
            pass_instruction(LOAD_CONST, 2, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]);
        assembler.next_label = 1;
        assembler.has_assertion_side_exit_candidate = true;
        assembler
    };
    let expected = assembler().finish().unwrap();
    let mut assembler = assembler();

    assert_eq!(assembler.used_constant_indices(3), [true, false, true]);
    assert!(matches!(assembler.code, UnoptimizedCode::Linear(_)));
    assert!(assembler.has_proven_linear_assertion_side_exits);
    assembler.remap_constant_indices(&[Some(0), Some(1), Some(2)]);

    assert_eq!(assembler.finish().unwrap(), expected);
}

#[test]
fn assertion_constant_reachability_rejects_mixed_control_flow() {
    let assertion_target = super::Label::from_u32(0);
    let other_target = super::Label::from_u32(1);
    let mut assembler = Assembler::from_linear_items(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(assertion_target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(assertion_target),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(other_target),
            SourceLocation::new(2, 2, 0, 0),
            None,
        )),
        pass_instruction(LOAD_CONST, 1, 2),
        Item::Label(other_target),
        pass_instruction(LOAD_CONST, 2, 3),
        pass_instruction(RETURN_VALUE, 0, 3),
    ]);
    assembler.next_label = 2;
    assembler.has_assertion_side_exit_candidate = true;

    assert_eq!(assembler.used_constant_indices(3), [true, false, true]);
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
}

#[test]
fn assertion_constant_reachability_keeps_swap_and_borrow_markers_off_the_linear_fast_path() {
    for borrow_marker in [false, true] {
        let target = super::Label::from_u32(0);
        let mut jump = Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        );
        if borrow_marker {
            jump.jump_policy.mark_borrow_empty_block();
        }
        let mut items = vec![pass_instruction(LOAD_CONST, 0, 1)];
        if !borrow_marker {
            items.extend([
                pass_instruction(LOAD_CONST, 2, 1),
                pass_instruction(SWAP, 2, 1),
                pass_instruction(POP_TOP, 0, 1),
            ]);
        }
        items.extend([
            Item::Instruction(jump),
            pass_instruction(NOT_TAKEN, 0, 1),
            pass_instruction(
                LOAD_COMMON_CONSTANT,
                CommonConstant::ASSERTIONERROR.argument(),
                1,
            ),
            pass_instruction(RAISE_VARARGS, 1, 1),
            Item::Label(target),
            pass_instruction(LOAD_CONST, 2, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]);
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 1;
        assembler.has_assertion_side_exit_candidate = true;

        assert_eq!(assembler.used_constant_indices(3), [true, false, true]);
        assert!(matches!(assembler.code, UnoptimizedCode::Linear(_)));
        assert!(!assembler.has_proven_linear_assertion_side_exits);
        assert!(!assembler.has_only_linear_assertion_side_exits());
    }
}

fn pass_instruction(opcode: crate::target::Opcode, argument: u32, line: i32) -> Item {
    Item::Instruction(Instruction::new(
        opcode,
        Operand::Value(argument),
        SourceLocation::new(line, line, 0, 0),
        None,
    ))
}

fn optimization_pipeline(items: Vec<Item>) -> super::OptimizationPipeline {
    let next_label = items
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) => Some(label.as_u32() + 1),
            Item::Instruction(instruction) => match instruction.operand() {
                Operand::Forward(label) | Operand::Backward(label) => Some(label.as_u32() + 1),
                Operand::Value(_) | Operand::Deferred(_) => None,
            },
        })
        .max()
        .unwrap_or(0);
    let mut assembler = Assembler::from_linear_items(items);
    assembler.next_label = next_label;
    assembler.into_optimization_pipeline()
}

fn unpacked_or_capture_items(depth: u32) -> Vec<Item> {
    let mut items = vec![pass_instruction(UNPACK_SEQUENCE, 1, 3)];
    for distance in (2..=depth).rev() {
        items.push(pass_instruction(SWAP, distance, 3));
    }
    for column in 0..depth - 1 {
        let source_column = i32::try_from(column).unwrap();
        let mut pop = Instruction::new(
            POP_TOP,
            Operand::Value(0),
            SourceLocation::new(3, 3, source_column, source_column + 1),
            Some(column),
        );
        if column.is_multiple_of(2) {
            pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        }
        items.push(Item::Instruction(pop));
    }
    items.extend([
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(super::Label::from_u32(0)),
            SourceLocation::new(3, 3, 0, 20),
            Some(1),
        )),
        Item::Label(super::Label::from_u32(0)),
        pass_instruction(RETURN_VALUE, 0, 4),
    ]);
    items
}

#[test]
fn unpacked_or_capture_rotations_preserve_the_largest_swap_and_pop_records() {
    for depth in [3, 5, 7, 9] {
        let items = unpacked_or_capture_items(depth);
        let depth = usize::try_from(depth).unwrap();
        let rotation = super::capture_cleanup_rotation(&items).unwrap();
        assert_eq!(rotation.swap_count, depth - 1);
        assert_eq!(rotation.removed_slots, depth - 2);
        assert_eq!(rotation.consumer_rotation, 1);
        assert_eq!(rotation.consumers.len(), depth - 1);
        let mut pipeline = optimization_pipeline(items.clone());
        let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
        assert!(pipeline.apply_static_swaps((blocks, cleanup)).is_empty());
        let actual = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
        assert_eq!(actual.len(), items.len());
        assert_eq!(&actual[..2], &items[..2]);
        for index in 2..depth {
            let (Item::Instruction(before), Item::Instruction(after)) =
                (items[index], actual[index])
            else {
                panic!()
            };
            assert_eq!(after.opcode, NOP);
            assert_eq!(after.operand(), Operand::Value(0));
            assert_eq!(after.exception_policy(), before.exception_policy());
            assert_eq!(after.depth_after(), before.depth_after());
        }
        let mut pops = items[depth..2 * depth - 1].to_vec();
        pops.rotate_right(1);
        assert_eq!(&actual[depth..2 * depth - 1], pops);
        assert_eq!(&actual[2 * depth - 1..], &items[2 * depth - 1..]);
    }
}

#[test]
fn unpacked_or_capture_rotations_require_the_complete_original_array() {
    for mutation in 0..17 {
        let mut items = unpacked_or_capture_items(5);
        match mutation {
            0 => items[0] = pass_instruction(UNPACK_SEQUENCE, 0, 3),
            1 => items[0] = pass_instruction(UNPACK_SEQUENCE, 2, 3),
            2 => items[0] = pass_instruction(POP_TOP, 0, 3),
            3 => items[2] = pass_instruction(SWAP, 3, 3),
            4 => items[2] = pass_instruction(SWAP, 4, 4),
            5 => items[6] = pass_instruction(POP_TOP, 0, 4),
            6 => items[6] = pass_instruction(POP_TOP, 1, 3),
            7 => items[6] = pass_instruction(STORE_FAST, 0, 3),
            8 => items.insert(4, Item::Label(super::Label::from_u32(1))),
            9 => items.insert(5, pass_instruction(NOP, 0, 3)),
            10 => {
                items.remove(8);
            }
            11 => items.insert(9, pass_instruction(POP_TOP, 0, 3)),
            12 => {
                items.truncate(9);
            }
            13 => {
                let Item::Instruction(jump) = &mut items[9] else {
                    panic!()
                };
                jump.set_operand(Operand::Backward(super::Label::from_u32(0)));
            }
            14 => {
                let Item::Instruction(jump) = &mut items[9] else {
                    panic!()
                };
                jump.opcode = POP_JUMP_IF_FALSE;
            }
            15 => {
                let Item::Instruction(swap) = &mut items[2] else {
                    panic!()
                };
                swap.jump_policy.retain_nop();
            }
            16 => {
                let Item::Instruction(pop) = &mut items[6] else {
                    panic!()
                };
                pop.no_location_policy.preserve_location();
            }
            _ => unreachable!(),
        }
        assert!(
            super::capture_cleanup_rotation(&items).is_none(),
            "mutation {mutation}"
        );
    }
}

fn split_line_capture_items(depth: usize, same_line_pops: usize) -> Vec<Item> {
    let mut items = vec![pass_instruction(UNPACK_SEQUENCE, 1, 5)];
    for distance in (2..=depth).rev() {
        items.push(pass_instruction(SWAP, u32::try_from(distance).unwrap(), 5));
    }
    for index in 0..depth - 1 {
        let column = i32::try_from(index).unwrap();
        let line = if index < same_line_pops { 5 } else { 3 };
        let mut pop = Instruction::new(
            POP_TOP,
            Operand::Value(0),
            SourceLocation::new(line, line, column, column + 1),
            Some(u32::try_from(depth - index).unwrap()),
        );
        if index.is_multiple_of(2) {
            pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        }
        items.push(Item::Instruction(pop));
    }
    items.push(pass_instruction(STORE_FAST, 0, 3));
    items.push(pass_instruction(LOAD_FAST, 1, 3));
    items
}

#[test]
fn split_line_capture_rotations_preserve_high_swaps_and_the_outer_tail() {
    for (depth, count) in [(5, 3), (6, 3), (7, 4), (8, 4), (9, 6)] {
        let items = split_line_capture_items(depth, count);
        let rotation = super::capture_cleanup_rotation(&items).unwrap();
        assert_eq!(rotation.swap_count, depth - 1);
        assert_eq!(rotation.removed_slots, count - 1);
        assert_eq!(rotation.consumer_rotation, 1);
        assert_eq!(rotation.consumers.len(), count);
        let mut pipeline = optimization_pipeline(items.clone());
        let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
        assert!(pipeline.apply_static_swaps((blocks, cleanup)).is_empty());
        let actual = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
        assert_eq!(actual.len(), items.len());
        let first_removed = depth - count + 1;
        assert_eq!(&actual[..first_removed], &items[..first_removed]);
        for index in first_removed..depth {
            let (Item::Instruction(before), Item::Instruction(after)) =
                (items[index], actual[index])
            else {
                panic!()
            };
            assert_eq!(after.opcode, NOP);
            assert_eq!(after.operand(), Operand::Value(0));
            assert_eq!(after.exception_policy(), before.exception_policy());
            assert_eq!(after.depth_after(), before.depth_after());
        }
        let mut pops = items[depth..depth + count].to_vec();
        pops.rotate_right(1);
        assert_eq!(&actual[depth..depth + count], pops);
        assert_eq!(&actual[depth + count..], &items[depth + count..]);
    }
}

#[test]
fn split_line_capture_slot_prediction_keeps_outer_store_fusion() {
    for (depth, count) in [(5, 3), (6, 3), (7, 4), (8, 4), (9, 6)] {
        for fuses in [false, true] {
            let mut items = split_line_capture_items(depth, count);
            *items.last_mut().unwrap() = pass_instruction(LOAD_FAST, if fuses { 1 } else { 16 }, 3);
            let assembler = Assembler::from_linear_items(items);
            assert_eq!(
                assembler.current_block_dynamic_exception_owner_slots(),
                count - 1 + usize::from(fuses)
            );
        }
    }
}

#[test]
fn split_line_capture_rotation_requires_the_complete_two_line_tail() {
    for mutation in 0..21 {
        let mut items = split_line_capture_items(7, 4);
        match mutation {
            0 => items = split_line_capture_items(7, 0),
            1 => items = split_line_capture_items(7, 1),
            2 => items = split_line_capture_items(7, 2),
            3 => items = split_line_capture_items(7, 6),
            4 => items[13] = pass_instruction(STORE_FAST, 0, 5),
            5 => items[13] = pass_instruction(STORE_DEREF, 0, 3),
            6 => {
                let Item::Instruction(store) = &mut items[13] else {
                    panic!()
                };
                store.local_policy.mark_maybe_null_comprehension_restore();
            }
            7 => items[13] = pass_instruction(STORE_FAST, 0, -1),
            8 => {
                let Item::Instruction(store) = &mut items[13] else {
                    panic!()
                };
                store.set_operand(Operand::Forward(super::Label::from_u32(0)));
            }
            9 => items[8] = pass_instruction(POP_TOP, 1, 5),
            10 => items[8] = pass_instruction(STORE_FAST, 1, 5),
            11 => items[8] = pass_instruction(POP_TOP, 0, 4),
            12 => items[11] = pass_instruction(POP_TOP, 0, 4),
            13 => items[12] = pass_instruction(POP_TOP, 1, 3),
            14 => items[12] = pass_instruction(STORE_FAST, 1, 3),
            15 => items[12] = Item::Label(super::Label::from_u32(0)),
            16 => items[12] = pass_instruction(NOP, 0, 3),
            17 => {
                items.remove(12);
            }
            18 => items.insert(12, pass_instruction(STORE_FAST, 1, 3)),
            19 => items.truncate(13),
            20 => items.insert(8, Item::Label(super::Label::from_u32(0))),
            _ => unreachable!(),
        }
        assert!(
            super::partial_capture_cleanup_rotation(&items, 7).is_none(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn split_line_capture_rotation_requires_unchanged_prefix_and_swap_records() {
    for mutation in 0..9 {
        let mut items = split_line_capture_items(7, 4);
        match mutation {
            0 => items[0] = pass_instruction(UNPACK_SEQUENCE, 0, 5),
            1 => items[0] = pass_instruction(UNPACK_SEQUENCE, 2, 5),
            2 => items[0] = pass_instruction(UNPACK_EX, 0, 5),
            3 => items[0] = pass_instruction(UNPACK_SEQUENCE, 1, -1),
            4 => items[2] = pass_instruction(SWAP, 5, 5),
            5 => items[2] = pass_instruction(SWAP, 6, 4),
            6 => items[2] = pass_instruction(SWAP, 6, -1),
            7 => items.insert(3, Item::Label(super::Label::from_u32(0))),
            8 => items.insert(3, pass_instruction(NOP, 0, 5)),
            _ => unreachable!(),
        }
        assert!(
            super::capture_cleanup_rotation(&items).is_none(),
            "mutation {mutation}"
        );
    }
    for index in 0..14 {
        for policy in 0..4 {
            let mut items = split_line_capture_items(7, 4);
            let Item::Instruction(instruction) = &mut items[index] else {
                panic!()
            };
            match policy {
                0 => instruction.jump_policy.retain_nop(),
                1 => instruction.no_location_policy.preserve_location(),
                2 => instruction.fusion_policy.prevent_with_previous(),
                3 => instruction.fusion_policy.prevent_with_next(),
                _ => unreachable!(),
            }
            assert!(
                super::capture_cleanup_rotation(&items).is_none(),
                "index {index}, policy {policy}"
            );
        }
    }
}

fn split_nested_capture_items(depth: usize, pops: usize, offset: u32, guard: bool) -> Vec<Item> {
    let mut items = vec![pass_instruction(UNPACK_SEQUENCE, 1, 5)];
    for distance in (2..=depth).rev() {
        items.push(pass_instruction(SWAP, u32::try_from(distance).unwrap(), 5));
    }
    for index in 0..pops {
        let column = i32::try_from(index).unwrap();
        let mut pop = Instruction::new(
            POP_TOP,
            Operand::Value(0),
            SourceLocation::new(5, 5, column, column + 1),
            Some(u32::try_from(depth - index - 1).unwrap()),
        );
        if index.is_multiple_of(2) {
            pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        }
        items.push(Item::Instruction(pop));
    }
    for distance in (2..=depth - pops).rev() {
        items.push(pass_instruction(SWAP, u32::try_from(distance).unwrap(), 5));
    }
    items.extend(vec![pass_instruction(POP_TOP, 0, 3); depth - pops - 2]);
    items.extend([
        pass_instruction(STORE_FAST, offset, 3),
        pass_instruction(STORE_FAST, offset + 1, 3),
    ]);
    if guard {
        items.push(pass_instruction(LOAD_FAST, offset + 2, 3));
    }
    items.push(pass_instruction(LOAD_CONST, 0, 7));
    items
}

#[test]
fn split_nested_capture_rotations_predict_both_runs_and_exact_pop_donor() {
    for (depth, pops) in [(5, 3), (6, 3), (7, 3), (8, 4), (10, 5)] {
        for offset in [0, 16] {
            for guard in [false, true] {
                let items = split_nested_capture_items(depth, pops, offset, guard);
                let rotation = super::nested_capture_cleanup_rotation(&items).unwrap();
                assert_eq!(rotation.removed_slots, depth - 2);
                let donor = rotation.projected_pop_donor.unwrap();
                assert_eq!(donor.0, depth + pops - 1);
                assert_eq!(Item::Instruction(donor.1), items[depth + pops - 2]);
                let assembler = Assembler::from_linear_items(items.clone());
                assert_eq!(
                    assembler.current_block_dynamic_exception_owner_slots(),
                    depth - 2 + usize::from(offset == 0 && (guard || depth - pops == 2))
                );
                let mut expected = items[..=(depth - pops)].to_vec();
                expected.extend(rotation.consumers.iter().copied());
                let consumed = 1 + rotation.swap_count + rotation.consumers.len();
                expected.extend_from_slice(&items[consumed..]);
                let mut pipeline = optimization_pipeline(items);
                let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
                pipeline.apply_static_swaps((blocks, cleanup));
                let actual = pipeline.graph.iter_items().filter(|item| {
                    !matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP)
                }).copied().collect::<Vec<_>>();
                assert_eq!(actual, expected);
                let Item::Instruction(nop) = pipeline.graph.iter_items().nth(depth + pops).unwrap()
                else {
                    panic!()
                };
                assert_eq!(nop.opcode, NOP);
                assert_eq!(nop.location, SourceLocation::NONE);
                assert!(nop.no_location_policy.preserves_location());
            }
        }
    }
}

#[test]
fn split_nested_capture_donor_requires_the_exact_projected_record() {
    let items = split_nested_capture_items(8, 4, 0, false);
    let mut pipeline = optimization_pipeline(items);
    let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
    let positions = pipeline.graph.item_positions();
    pipeline
        .graph
        .instruction_mut(positions[10])
        .unwrap()
        .location
        .column += 1;
    pipeline.apply_static_swaps((blocks, cleanup));
    let nop = pipeline.graph.instruction_mut(positions[12]).unwrap();
    assert_eq!(nop.opcode, NOP);
    assert_eq!(nop.location.line, 5);
    assert!(!nop.no_location_policy.preserves_location());
}

#[test]
fn split_nested_capture_rotations_require_both_closed_arrays() {
    for mutation in 0..12 {
        let mut items = split_nested_capture_items(8, 4, 0, false);
        match mutation {
            0 => items[1] = pass_instruction(SWAP, 7, 5),
            1 => items[12] = pass_instruction(SWAP, 3, 5),
            2 => items[13] = pass_instruction(SWAP, 3, 3),
            3 => items[10] = pass_instruction(POP_TOP, 0, 3),
            4 => items[15] = pass_instruction(POP_TOP, 0, 4),
            5 => items[18] = pass_instruction(STORE_FAST, 0, 3),
            6 => items[18] = pass_instruction(STORE_DEREF, 1, 3),
            7 => items[19] = pass_instruction(SWAP, 2, 7),
            8 => items.insert(12, Item::Label(super::Label::from_u32(0))),
            9 => items.insert(13, pass_instruction(NOP, 0, 5)),
            10 => items.truncate(19),
            11 => {
                let Item::Instruction(store) = &mut items[18] else {
                    panic!()
                };
                store.local_policy.mark_maybe_null_comprehension_restore();
            }
            _ => unreachable!(),
        }
        assert!(
            super::nested_capture_cleanup_rotation(&items).is_none(),
            "mutation {mutation}"
        );
    }
    for index in 0..19 {
        for policy in 0..3 {
            let mut items = split_nested_capture_items(8, 4, 0, false);
            let Item::Instruction(instruction) = &mut items[index] else {
                panic!()
            };
            match policy {
                0 => instruction.jump_policy.retain_nop(),
                1 => instruction.no_location_policy.preserve_location(),
                2 => instruction.fusion_policy.prevent_with_previous(),
                _ => unreachable!(),
            }
            assert!(
                super::nested_capture_cleanup_rotation(&items).is_none(),
                "index {index}, policy {policy}"
            );
        }
    }
}

fn nested_capture_items(depth: u32, captures: u32, split_line: bool, guard: bool) -> Vec<Item> {
    let capture_line = if split_line { 4 } else { 3 };
    let mut items = vec![pass_instruction(UNPACK_SEQUENCE, 1, capture_line)];
    for distance in (2..=depth).rev() {
        items.push(pass_instruction(SWAP, distance, capture_line));
    }
    items.extend([pass_instruction(POP_TOP, 0, 3); 2]);
    for distance in (2..=depth - 2).rev() {
        items.push(pass_instruction(SWAP, distance, 3));
    }
    for _ in 0..depth - 2 - captures {
        items.push(pass_instruction(POP_TOP, 0, 3));
    }
    for local in 0..captures {
        items.push(pass_instruction(STORE_FAST, local, 3));
    }
    if guard {
        items.push(pass_instruction(LOAD_FAST, captures, 3));
    }
    items.push(pass_instruction(LOAD_CONST, 0, 5));
    items
}

#[test]
fn nested_capture_rotations_preserve_arrays_and_predict_compaction() {
    for (depth, captures) in [(4, 2), (5, 3), (6, 2), (7, 3), (8, 4)] {
        for split_line in [false, true] {
            for guard in [false, true] {
                let items = nested_capture_items(depth, captures, split_line, guard);
                let rotation = super::nested_capture_cleanup_rotation(&items).unwrap();
                let swaps = usize::try_from(2 * depth - 4).unwrap();
                assert_eq!(rotation.swap_count, swaps);
                assert_eq!(rotation.removed_slots, swaps);
                assert_eq!(rotation.consumer_rotation, 0);
                let consumed = 1 + swaps + rotation.consumers.len();
                let expected = std::iter::once(items[0])
                    .chain(rotation.consumers.iter().copied())
                    .chain(items[consumed..].iter().copied())
                    .collect::<Vec<_>>();
                let assembler = Assembler::from_linear_items(items.clone());
                let stores = captures - 2 + u32::from(depth - 2 == captures);
                let fused = stores / 2 + u32::from(guard && !stores.is_multiple_of(2));
                assert_eq!(
                    assembler.current_block_dynamic_exception_owner_slots(),
                    swaps + usize::try_from(fused).unwrap(),
                );
                let mut pipeline = optimization_pipeline(items.clone());
                let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
                pipeline.apply_static_swaps((blocks, cleanup));
                assert_eq!(pipeline.graph.iter_items().count(), items.len());
                let actual = pipeline.graph.iter_items().filter(|item| {
                    !matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP)
                }).copied().collect::<Vec<_>>();
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn nested_capture_rotations_bound_guard_fusion_and_starred_donors() {
    for opcode in [LOAD_FAST, LOAD_FAST_BORROW, LOAD_FAST_CHECK] {
        for offset in [0, 16] {
            let mut items = nested_capture_items(7, 3, true, true);
            items[0] = pass_instruction(UNPACK_EX, 0, 4);
            let guard = items.len() - 2;
            for item in &mut items {
                if let Item::Instruction(instruction) = item
                    && instruction.opcode == STORE_FAST
                    && let Operand::Value(local) = instruction.operand()
                {
                    instruction.set_operand(Operand::Value(local + offset));
                }
            }
            items[guard] = pass_instruction(opcode, 3 + offset, 3);
            let rotation = super::nested_capture_cleanup_rotation(&items).unwrap();
            assert_eq!(rotation.consumers.len(), 7 + usize::from(offset == 0));
            let assembler = Assembler::from_linear_items(items.clone());
            assert_eq!(
                assembler.current_block_dynamic_exception_owner_slots(),
                10 + usize::from(offset == 0),
            );
            let mut pipeline = optimization_pipeline(items.clone());
            let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
            pipeline.apply_static_swaps((blocks, cleanup));
            let Item::Instruction(nop) = pipeline.graph.iter_items().nth(1).unwrap() else {
                panic!()
            };
            assert_eq!(nop.opcode, NOP);
            assert_eq!(nop.location, SourceLocation::NONE);
            assert!(nop.no_location_policy.preserves_location());
            if offset == 0 {
                for index in [0, guard] {
                    for policy in 0..3 {
                        let mut changed = items.clone();
                        let Item::Instruction(instruction) = &mut changed[index] else {
                            panic!()
                        };
                        match policy {
                            0 => instruction.jump_policy.retain_nop(),
                            1 => instruction.no_location_policy.preserve_location(),
                            2 => instruction.fusion_policy.prevent_with_previous(),
                            _ => unreachable!(),
                        }
                        assert!(super::nested_capture_cleanup_rotation(&changed).is_none());
                    }
                }
            }
        }
    }
    let items = vec![
        pass_instruction(UNPACK_EX, 0, 3),
        pass_instruction(SWAP, 2, 3),
        pass_instruction(STORE_FAST, 0, 3),
        pass_instruction(STORE_FAST, 1, 3),
        pass_instruction(LOAD_CONST, 0, 4),
    ];
    assert!(super::capture_cleanup_rotation(&items).is_none());
    let mut pipeline = optimization_pipeline(items);
    let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
    pipeline.apply_static_swaps((blocks, cleanup));
    let Item::Instruction(nop) = pipeline.graph.iter_items().nth(1).unwrap() else {
        panic!()
    };
    assert_eq!(nop.opcode, NOP);
    assert_eq!(nop.location.line, 3);
    assert!(!nop.no_location_policy.preserves_location());
}

#[test]
fn nested_capture_rotations_require_complete_original_arrays() {
    for mutation in 0..21 {
        let mut items = nested_capture_items(6, 2, false, false);
        match mutation {
            0 => items[0] = pass_instruction(UNPACK_SEQUENCE, 2, 3),
            1 => items[0] = pass_instruction(LOAD_CONST, 1, 3),
            2 => items[1] = pass_instruction(SWAP, 5, 3),
            3 => items[2] = pass_instruction(SWAP, 3, 3),
            4 => items[2] = pass_instruction(SWAP, 5, 4),
            5 => items[6] = pass_instruction(POP_TOP, 1, 3),
            6 => items[6] = pass_instruction(POP_TOP, 0, 4),
            7 => items[8] = pass_instruction(SWAP, 3, 3),
            8 => items[9] = pass_instruction(SWAP, 3, 4),
            9 => items[11] = pass_instruction(POP_TOP, 0, 4),
            10 => items[14] = pass_instruction(STORE_FAST, 0, 3),
            11 => items[14] = pass_instruction(STORE_DEREF, 1, 3),
            12 => items[14] = pass_instruction(STORE_FAST, 1, -1),
            13 => items[15] = pass_instruction(NOP, 0, 5),
            14 => items[15] = pass_instruction(LOAD_CONST, 0, -1),
            15 => items.insert(7, Item::Label(super::Label::from_u32(0))),
            16 => items.insert(10, pass_instruction(NOP, 0, 3)),
            17 => {
                items.remove(7);
            }
            18 => items.truncate(15),
            19 => {
                let Item::Instruction(store) = &mut items[14] else {
                    panic!()
                };
                store.local_policy.mark_maybe_null_comprehension_restore();
            }
            20 => items[8] = pass_instruction(SWAP, u32::MAX, 3),
            _ => unreachable!(),
        }
        assert!(
            super::nested_capture_cleanup_rotation(&items).is_none(),
            "mutation {mutation}"
        );
    }
    for index in 0..15 {
        for policy in 0..3 {
            let mut items = nested_capture_items(6, 2, false, false);
            let Item::Instruction(instruction) = &mut items[index] else {
                panic!()
            };
            match policy {
                0 => instruction.jump_policy.retain_nop(),
                1 => instruction.no_location_policy.preserve_location(),
                2 => instruction.fusion_policy.prevent_with_previous(),
                _ => unreachable!(),
            }
            assert!(
                super::nested_capture_cleanup_rotation(&items).is_none(),
                "index {index}, policy {policy}"
            );
        }
    }
}

fn mapping_capture_items(captures: u32, wildcards: u32) -> Vec<Item> {
    let unpacked = captures + wildcards;
    let depth = unpacked + 2;
    let mut items = vec![pass_instruction(UNPACK_SEQUENCE, unpacked, 3)];
    for _ in 0..captures {
        for depth in (2..=depth).rev() {
            items.push(pass_instruction(SWAP, depth, 3));
        }
    }
    for _ in 0..wildcards + 2 {
        items.push(pass_instruction(POP_TOP, 0, 3));
    }
    for local in 0..captures {
        items.push(pass_instruction(STORE_FAST, local, 3));
    }
    items.push(pass_instruction(LOAD_CONST, 0, 4));
    items
}

#[test]
fn mapping_capture_rotations_preserve_arrays_and_predict_native_compaction() {
    for (captures, wildcards) in [(1, 1), (1, 2), (2, 1), (2, 2), (3, 1), (4, 1)] {
        let items = mapping_capture_items(captures, wildcards);
        let swaps = captures * (captures + wildcards + 1);
        let assembler = Assembler::from_linear_items(items.clone());
        assert_eq!(
            assembler.current_block_dynamic_exception_owner_slots(),
            usize::try_from(swaps + captures / 2).unwrap(),
        );
        let mut pipeline = optimization_pipeline(items.clone());
        let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
        let gaps = pipeline.apply_static_swaps((blocks, cleanup));
        assert!(gaps.is_empty());
        assert_eq!(pipeline.graph.iter_items().count(), items.len());
        let actual = pipeline
            .graph
            .iter_items()
            .filter(
                |item| !matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP),
            )
            .copied()
            .collect::<Vec<_>>();
        let consumers = 1 + usize::try_from(swaps).unwrap();
        let stores = items.len() - 1 - usize::try_from(captures).unwrap();
        let expected = std::iter::once(items[0])
            .chain(items[stores..items.len() - 1].iter().copied())
            .chain(items[consumers..stores].iter().copied())
            .chain(std::iter::once(*items.last().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}

#[test]
fn mapping_capture_rotations_require_complete_unchanged_same_line_shapes() {
    for mutation in 0..9 {
        let mut items = mapping_capture_items(2, 1);
        match mutation {
            0 => items[0] = pass_instruction(UNPACK_SEQUENCE, 4, 3),
            1 => items[4] = pass_instruction(SWAP, 3, 3),
            2 => items[4] = pass_instruction(SWAP, 2, 5),
            3 => items[10] = pass_instruction(POP_TOP, 0, 5),
            4 => items[13] = pass_instruction(STORE_FAST, 0, 3),
            5 => items[13] = pass_instruction(STORE_DEREF, 1, 3),
            6 => {
                let Item::Instruction(instruction) = &mut items[4] else {
                    unreachable!()
                };
                instruction.no_location_policy.preserve_location();
            }
            7 => items[14] = pass_instruction(LOAD_CONST, 0, 3),
            8 => {
                items.insert(9, Item::Label(super::Label::from_u32(0)));
            }
            _ => unreachable!(),
        }
        assert!(
            super::mapping_capture_cleanup_rotation(&items).is_none(),
            "mutation {mutation}"
        );
    }
    let mut incomplete = mapping_capture_items(2, 1);
    incomplete.pop();
    assert!(super::mapping_capture_cleanup_rotation(&incomplete).is_none());
}

#[test]
fn while_else_conjunction_footer_requires_exact_ordered_failure_edges() {
    let failure = super::Label::from_u32(0);
    let end = super::Label::from_u32(1);
    let first_location = SourceLocation::new(10, 10, 15, 20);
    let second_location = SourceLocation::new(11, 11, 19, 24);
    let conditions = [
        (first_location, POP_JUMP_IF_FALSE),
        (second_location, POP_JUMP_IF_TRUE),
    ];
    let mut break_jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(end),
        SourceLocation::new(6, 6, 12, 17),
        Some(0),
    );
    break_jump.fusion_policy.mark_authored_break();
    break_jump.jump_policy.prevent_threading_target();
    break_jump.jump_policy.retain_nop_for_direct_target();
    break_jump.add_pre_fold_extra_instructions(1);
    let first = Instruction::new(
        POP_JUMP_IF_FALSE,
        Operand::Forward(failure),
        first_location,
        Some(0),
    );
    let second = Instruction::new(
        POP_JUMP_IF_TRUE,
        Operand::Forward(failure),
        second_location,
        Some(0),
    );
    for mutation in [
        "original",
        "swapped conditions",
        "wrong polarity",
        "missing condition",
        "extra condition",
        "foreign jump",
        "shared break target",
        "second break",
        "condition depth",
        "hidden condition slot",
        "condition location policy",
        "condition owner",
        "condition jump policy",
        "condition fusion policy",
        "condition early admission",
        "footer owner",
        "break policy",
        "folded target",
        "missing original boundary",
        "extra footer alias",
        "singleton",
        "singleton foreign policies",
    ] {
        let mut changed = second;
        match mutation {
            "wrong polarity" => changed.opcode = POP_JUMP_IF_FALSE,
            "shared break target" => changed.set_operand(Operand::Forward(end)),
            "condition depth" => {
                changed =
                    Instruction::new(second.opcode, second.operand(), second.location, Some(1));
            }
            "hidden condition slot" => changed.local_policy.add_hidden_exception_owner_slots(1),
            "condition location policy" => changed.no_location_policy.preserve_location(),
            "condition owner" => changed.set_exception_owner(NormalizedExceptionOwner::Absent),
            "condition jump policy" => changed.jump_policy.prevent_threading_target(),
            "condition fusion policy" => {
                changed.fusion_policy.allow_transitive_small_exit_inlining();
            }
            "condition early admission" => changed.early_boolean_admissions.append(true),
            _ => {}
        }
        let mut items = vec![
            Item::Instruction(break_jump),
            Item::Instruction(first),
            Item::Instruction(changed),
            Item::Instruction(Instruction::new(
                POP_TOP,
                Operand::Value(0),
                SourceLocation::new(12, 12, 12, 19),
                Some(0),
            )),
            Item::Label(failure),
            Item::Label(end),
        ];
        if matches!(mutation, "break policy" | "singleton foreign policies") {
            let Item::Instruction(jump) = &mut items[0] else {
                unreachable!()
            };
            jump.jump_policy.defer_removal();
        }
        if matches!(mutation, "footer owner" | "singleton foreign policies") {
            let Item::Instruction(pop) = &mut items[3] else {
                unreachable!()
            };
            pop.set_exception_owner(NormalizedExceptionOwner::Absent);
        }
        if mutation == "singleton foreign policies" {
            let Item::Instruction(condition) = &mut items[1] else {
                unreachable!()
            };
            condition.set_exception_owner(NormalizedExceptionOwner::Absent);
            condition.early_boolean_admissions.append(true);
        }
        match mutation {
            "swapped conditions" => items.swap(1, 2),
            "missing condition" | "singleton" | "singleton foreign policies" => {
                items.remove(2);
            }
            "extra condition" => items.insert(3, Item::Instruction(second)),
            "foreign jump" => items.insert(
                3,
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(failure),
                    second_location,
                    Some(0),
                )),
            ),
            "second break" => items.insert(1, Item::Instruction(break_jump)),
            "extra footer alias" => items.push(Item::Label(super::Label::from_u32(2))),
            _ => {}
        }
        let mut assembler = Assembler::from_linear_items(items);
        assembler.record_exception_owner_block_boundary(failure);
        if mutation == "folded target" {
            assembler.folded_away_condition_targets.insert(failure);
        } else if mutation == "missing original boundary" {
            assembler
                .exception_owner_provenance
                .source_boundaries
                .remove(&failure);
        }
        let original_items = assembler.linear_items().to_vec();
        assert_eq!(
            assembler.at_original_while_else_break_conjunction_join(&conditions),
            mutation == "original",
            "{mutation}",
        );
        if matches!(mutation, "singleton" | "singleton foreign policies") {
            assert!(assembler.at_original_while_else_break_join(first_location));
        }
        assert_eq!(assembler.linear_items(), original_items, "{mutation}");
    }
}

#[test]
fn sync_for_else_finally_exit_requires_exact_producer_and_records() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for mutation in [
            "intact",
            "legacy 312",
            "legacy 313",
            "missing producer",
            "wrong producer depth",
            "missing source boundary",
            "extra alias",
            "repeated end",
            "folded end",
            "omitted nop",
            "borrow alias",
            "foreign edge",
            "extra break",
            "break policy",
            "break owner",
            "break hidden slot",
            "break folded slot",
            "cleanup owner",
            "call owner",
            "call policy",
            "exit owner",
            "exit policy",
            "exit location",
            "outer owner",
        ] {
            let mut assembler = Assembler::with_python_version(version);
            let end = assembler.label();
            let target = assembler.label();
            let else_entry = assembler.label();
            let break_location = SourceLocation::new(5, 5, 12, 17);
            let call_location = SourceLocation::new(7, 7, 8, 17);
            let slots = 1 + u32::from(version == crate::PythonVersion::Py315);
            let depth = slots;
            assembler.set_location(break_location);
            for remaining in (0..slots).rev() {
                assembler.emit_with_depth(
                    POP_TOP,
                    0,
                    depth + remaining,
                    if mutation == "cleanup owner" {
                        ExceptionPolicy::EXCLUDE_ALWAYS
                    } else {
                        ExceptionPolicy::default()
                    },
                );
            }
            let authored = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                end,
                depth,
                ExceptionPolicy::default(),
            );
            assembler.mark_jump_as_authored_break(authored);
            assembler.prevent_jump_threading_target(authored);
            let jump = assembler.emitted_jump_mut(authored, "mutating the break control");
            match mutation {
                "break policy" => jump.jump_policy.defer_removal(),
                "break owner" => jump.set_exception_owner(NormalizedExceptionOwner::Absent),
                "break hidden slot" => jump.local_policy.add_hidden_exception_owner_slots(1),
                "break folded slot" => jump.add_pre_fold_extra_instructions(1),
                _ => {}
            }
            if matches!(mutation, "foreign edge" | "extra break") {
                let extra = assembler.emit_forward_with_depth(
                    if mutation == "foreign edge" {
                        POP_JUMP_IF_FALSE
                    } else {
                        JUMP_FORWARD
                    },
                    end,
                    depth,
                    ExceptionPolicy::default(),
                );
                if mutation == "extra break" {
                    assembler.mark_jump_as_authored_break(extra);
                    assembler.prevent_jump_threading_target(extra);
                }
            }
            if mutation == "repeated end" {
                assembler.mark(end);
            }
            assembler.mark(else_entry);
            assembler.set_location(call_location);
            assembler.emit_with_depth(CALL, 0, depth + 1, ExceptionPolicy::default());
            let call_pop = assembler.emit_with_depth(
                POP_TOP,
                0,
                depth,
                if mutation == "call owner" {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::default()
                },
            );
            if mutation == "call policy" {
                assembler
                    .emitted_instruction_mut(call_pop, "mutating the donor control")
                    .fusion_policy
                    .prevent_with_next();
            }
            if mutation == "extra alias" {
                let alias = assembler.label();
                assembler.mark(alias);
            }
            assembler.record_exception_owner_block_boundary(end);
            if mutation != "missing producer" {
                assembler.record_sync_for_else_end(
                    end,
                    depth + u32::from(mutation == "wrong producer depth"),
                );
            }
            assembler.mark(end);
            match mutation {
                "missing source boundary" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&end);
                }
                "folded end" => {
                    assembler.folded_away_condition_targets.insert(end);
                }
                "omitted nop" => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .insert(end, 1);
                }
                "borrow alias" => {
                    assembler.borrow_block_aliases.insert(end);
                }
                "outer owner" => {
                    assembler.add_exception_region(else_entry, target, target, depth, false);
                }
                _ => {}
            }
            let exit = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                target,
                depth,
                if mutation == "exit owner" {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::default()
                },
            );
            match mutation {
                "exit policy" => {
                    assembler
                        .emitted_jump_mut(exit, "mutating the exit control")
                        .no_location_policy
                        .allow_block_inlining();
                }
                "exit location" => {
                    assembler
                        .emitted_jump_mut(exit, "mutating the exit control")
                        .location = break_location;
                }
                "legacy 312" => assembler.python_version = crate::PythonVersion::Py312,
                "legacy 313" => assembler.python_version = crate::PythonVersion::Py313,
                _ => {}
            }
            let original = assembler.linear_items().to_vec();
            assembler.preserve_sync_for_else_finally_exit(exit, break_location, call_location);
            if mutation != "intact" {
                assert_eq!(assembler.linear_items(), original, "{version:?}/{mutation}");
                continue;
            }
            let retained = assembler.emitted_jump_mut(exit, "checking the certified exit");
            assert!(retained.jump_policy.is_original_empty_join());
            assert!(retained.jump_policy.preserves_nop_after_threading());
            assert!(retained.jump_policy.allows_threading_target());
            // The ordinary finalizer borrowing rule still runs after this request.
            retained.jump_policy.mark_borrow_empty_block();
            assembler.mark(target);
            assembler.set_location(SourceLocation::new(8, 8, 4, 10));
            assembler.emit_with_depth(LOAD_CONST, 0, depth + 1, ExceptionPolicy::default());
            assembler.emit_with_depth(RETURN_VALUE, 0, depth, ExceptionPolicy::default());
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.thread_forward_jumps();
            assert!(
                pipeline.graph.iter_items().any(|item| {
                    matches!(item, Item::Instruction(jump)
                    if jump.fusion_policy.is_authored_break()
                        && jump.operand() == Operand::Forward(target))
                }),
                "the break must bypass the original donor block on {version:?}"
            );
            // Match the production early pass that resolves deferred retention.
            pipeline.remove_redundant_forward_jumps(false);
            pipeline.remove_redundant_forward_jumps(true);
            pipeline.remove_redundant_nops();
            let donor_opcodes = pipeline
                .graph
                .iter_items()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) if instruction.location == call_location => {
                        Some(instruction.opcode)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(donor_opcodes, [CALL, POP_TOP, NOP], "{version:?}");
        }
    }
}

#[test]
fn class_failure_return_join_requires_original_pop_and_class_edges() {
    for mutation in [
        "intact",
        "legacy",
        "extra join edge",
        "wrong condition",
        "missing class fold slots",
        "foreign pop owner",
        "wrong pop depth",
        "extra alias",
        "missing match boundary",
        "omitted join nop",
        "extra class edge",
        "reversed patterns",
        "foreign exit owner",
    ] {
        let mut assembler = Assembler::with_python_version(if mutation == "legacy" {
            crate::PythonVersion::Py313
        } else {
            crate::PythonVersion::Py314
        });
        let failure = assembler.label();
        let zero_pop = assembler.label();
        let end = assembler.label();
        let target = assembler.label();
        let mut patterns = vec![
            (SourceLocation::new(3, 3, 13, 36), 0),
            (SourceLocation::new(3, 3, 26, 35), 0),
        ];
        if mutation == "extra join edge" {
            assembler.set_location(patterns[0].0);
            assembler.emit_forward_with_depth(
                POP_JUMP_IF_FALSE,
                zero_pop,
                0,
                ExceptionPolicy::default(),
            );
        }
        for &(location, positional) in &patterns {
            assembler.set_location(location);
            assembler.emit_with_depth(MATCH_CLASS, positional, 1, ExceptionPolicy::default());
            assembler.emit_with_depth(COPY, 1, 2, ExceptionPolicy::default());
            if mutation != "missing class fold slots" {
                assembler.add_last_hidden_exception_owner_slots(2);
            }
            assembler.emit_forward_with_depth(
                if mutation == "wrong condition" {
                    POP_JUMP_IF_FALSE
                } else {
                    POP_JUMP_IF_NONE
                },
                failure,
                1,
                ExceptionPolicy::default(),
            );
            assembler.emit_with_depth(NOT_TAKEN, 0, 1, ExceptionPolicy::default());
        }
        if mutation == "extra class edge" {
            assembler.emit_forward_with_depth(
                POP_JUMP_IF_NONE,
                failure,
                1,
                ExceptionPolicy::default(),
            );
        }
        assembler.set_location(SourceLocation::new(4, 4, 19, 23));
        assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
        assembler.mark(failure);
        assembler.set_location(patterns[0].0);
        assembler.emit_with_depth(
            POP_TOP,
            0,
            u32::from(mutation == "wrong pop depth"),
            if mutation == "foreign pop owner" {
                ExceptionPolicy::EXCLUDE_ALWAYS
            } else {
                ExceptionPolicy::default()
            },
        );
        assembler.mark(zero_pop);
        if mutation == "extra alias" {
            let extra = assembler.label();
            assembler.mark(extra);
        }
        if mutation == "missing match boundary" {
            assembler.mark(end);
        } else {
            assembler.mark_original_match_join(end);
        }
        if mutation == "omitted join nop" {
            assembler
                .exception_owner_provenance
                .omitted_nop_counts
                .insert(end, 1);
        }
        let exit = assembler.emit_forward_with_depth(
            JUMP_FORWARD,
            target,
            0,
            if mutation == "foreign exit owner" {
                ExceptionPolicy::EXCLUDE_ALWAYS
            } else {
                ExceptionPolicy::default()
            },
        );
        if mutation == "reversed patterns" {
            patterns.reverse();
        }
        let original = assembler.linear_items().to_vec();
        assembler.preserve_class_match_failure_return_join(exit, &patterns);
        if mutation == "intact" {
            let jump = assembler.emitted_jump_mut(exit, "checking certified failure exit");
            assert!(jump.jump_policy.is_original_empty_join());
            assert!(jump.jump_policy.preserves_nop_after_threading());
            assert_eq!(assembler.linear_items().len(), original.len());
        } else {
            assert_eq!(assembler.linear_items(), original, "{mutation}");
        }
    }
}

#[test]
fn original_match_end_accepts_empty_failure_aliases_only() {
    for mutation in [
        "alias",
        "exact end",
        "wildcard fallthrough",
        "source boundary",
        "missing match end",
        "foreign match end",
        "foreign target",
        "conditional edge",
        "backward edge",
        "authored nop",
        "borrow alias",
        "fusion boundary",
        "continuation boundary",
        "setup boundary",
        "handler entry",
        "located alias",
    ] {
        let mut assembler = Assembler::default();
        let alias = assembler.label();
        let end = assembler.label();
        let foreign = assembler.label();
        let target = match mutation {
            "exact end" => end,
            "foreign target" => foreign,
            _ => alias,
        };
        assembler.set_location(SourceLocation::new(3, 3, 8, 20));
        if mutation == "backward edge" {
            assembler.emit_backward(JUMP_BACKWARD, target);
        } else {
            let opcode = if mutation == "conditional edge" {
                POP_JUMP_IF_FALSE
            } else {
                JUMP_FORWARD
            };
            assembler.emit_forward_with_depth(opcode, target, 0, ExceptionPolicy::default());
        }
        assembler.set_location(SourceLocation::new(4, 4, 12, 18));
        assembler.emit(
            if mutation == "wildcard fallthrough" {
                POP_TOP
            } else {
                RETURN_VALUE
            },
            0,
        );
        assembler.mark(alias);
        if mutation == "located alias" {
            assembler.emit(NOP, 0);
        }
        match mutation {
            "missing match end" | "source boundary" => assembler.mark(end),
            "foreign match end" => {
                assembler.mark(end);
                assembler.original_match_join = Some(foreign);
            }
            _ => assembler.mark_original_match_join(end),
        }
        let provenance = &mut assembler.exception_owner_provenance;
        match mutation {
            "source boundary" => {
                provenance.source_boundaries.insert(alias);
                provenance.block_boundaries.insert(alias);
            }
            "authored nop" => {
                assembler.original_nop_targets.insert(alias);
            }
            "borrow alias" => {
                assembler.borrow_block_aliases.insert(alias);
            }
            "fusion boundary" => {
                provenance.fusion_boundaries.insert(alias);
            }
            "continuation boundary" => {
                provenance.continuation_boundaries.insert(alias);
            }
            "setup boundary" => provenance.setups.push((alias, None)),
            "handler entry" => assembler.exception_regions.push(super::ExceptionRegion {
                priority: super::ExceptionRegionPriority::OriginalRange(alias, end),
                is_scope_fallback: false,
                start: alias,
                end,
                target: alias,
                depth: 0,
                preserve_lasti: false,
            }),
            _ => {}
        }
        assert_eq!(
            assembler.at_original_unconditional_join(),
            matches!(
                mutation,
                "alias" | "exact end" | "wildcard fallthrough" | "source boundary"
            ),
            "{mutation}",
        );
    }
}

#[test]
fn shared_match_failure_join_requires_original_aliases_and_equal_depths() {
    let failure = super::Label::from_u32(0);
    let end = super::Label::from_u32(1);
    let cleanup = super::Label::from_u32(2);
    let condition = Instruction::new(
        POP_JUMP_IF_FALSE,
        Operand::Forward(failure),
        SourceLocation::new(3, 3, 20, 25),
        Some(1),
    );
    let pop = Instruction::new(
        POP_TOP,
        Operand::Value(0),
        SourceLocation::new(3, 3, 10, 26),
        Some(1),
    );
    for mutation in 0..10 {
        let mut items = vec![
            Item::Instruction(condition),
            pass_instruction(RETURN_VALUE, 0, 4),
            Item::Label(cleanup),
            Item::Instruction(pop),
            Item::Label(failure),
        ];
        match mutation {
            1 => items[3] = pass_instruction(NOP, 0, 3),
            2 => {
                items[0] = Item::Instruction(Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(failure),
                    condition.location,
                    Some(0),
                ));
            }
            3 => items.insert(0, Item::Instruction(condition)),
            4 => {
                items[0] = Item::Instruction(Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(cleanup),
                    condition.location,
                    Some(1),
                ));
            }
            5 => {
                let mut changed = pop;
                changed.no_location_policy.preserve_location();
                items[3] = Item::Instruction(changed);
            }
            _ => {}
        }
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 3;
        if mutation == 6 {
            assembler.mark(end);
        } else {
            assembler.mark_original_match_join(end);
        }
        match mutation {
            7 => {
                assembler.original_nop_targets.insert(failure);
            }
            8 => {
                assembler
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(failure, 1);
            }
            9 => {
                assembler.emit(NOP, 0);
            }
            _ => {}
        }
        assert_eq!(
            assembler.at_shared_match_failure_join(),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn match_failure_after_success_copy_requires_closed_aliases_and_full_records() {
    let pattern_location = SourceLocation::new(3, 3, 9, 18);
    for mutation in [
        "original",
        "extra alias",
        "hidden success slot",
        "foreign failure owner",
        "duplicate failure",
        "range-end edge",
        "missing match boundary",
    ] {
        let mut assembler = Assembler::default();
        let failure = assembler.label();
        let end = assembler.label();
        let try_end = assembler.label();
        assembler.set_location(pattern_location);
        let condition = assembler.emit_forward_with_depth(
            POP_JUMP_IF_FALSE,
            failure,
            0,
            ExceptionPolicy::default(),
        );
        if mutation == "foreign failure owner" {
            assembler
                .emitted_jump_mut(condition, "changing test owner")
                .set_exception_owner(NormalizedExceptionOwner::Absent);
        }
        assembler.emit_with_depth(NOT_TAKEN, 0, 0, ExceptionPolicy::default());
        if mutation == "duplicate failure" {
            assembler.emit_forward_with_depth(
                POP_JUMP_IF_FALSE,
                failure,
                0,
                ExceptionPolicy::default(),
            );
            assembler.emit_with_depth(NOT_TAKEN, 0, 0, ExceptionPolicy::default());
        } else if mutation == "range-end edge" {
            assembler.emit_forward_with_depth(JUMP_FORWARD, try_end, 0, ExceptionPolicy::default());
        }
        assembler.set_location(SourceLocation::new(4, 4, 12, 19));
        assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
        assembler.emit_with_depth(POP_TOP, 0, 0, ExceptionPolicy::default());
        let success =
            assembler.emit_forward_with_depth(JUMP_FORWARD, end, 0, ExceptionPolicy::default());
        assembler.record_original_match_success(success, false);
        assembler.mark_jump_as_interruptible(success);
        assembler.defer_jump_removal(success);
        if mutation == "hidden success slot" {
            assembler
                .emitted_jump_mut(success, "changing test slot")
                .local_policy
                .add_hidden_exception_owner_slots(1);
        }
        assembler.mark(failure);
        if mutation == "extra alias" {
            let alias = assembler.label();
            assembler.mark(alias);
        }
        if mutation == "missing match boundary" {
            assembler.mark(end);
        } else {
            assembler.mark_original_match_join(end);
        }
        assembler.mark(try_end);
        let original_items = assembler.linear_items().to_vec();
        assert_eq!(
            assembler.original_match_failure_after_success_copy(try_end, pattern_location, 0),
            mutation == "original",
            "{mutation}",
        );
        assert_eq!(assembler.linear_items(), original_items, "{mutation}");
    }
}

#[test]
fn retained_match_finally_entry_requires_complete_original_endpoints() {
    let pattern = SourceLocation::new(3, 3, 9, 18);
    for mutation in [
        "original",
        "source pop owner",
        "source jump location",
        "source extra alias",
        "source copy policy",
        "late success edge",
        "duplicate origin",
        "target extra alias",
        "target foreign entry",
        "target pop owner",
        "short cleanup",
        "same donor line",
        "owned first load",
        "hidden first slot",
        "target setup",
        "python315",
        "dead original",
        "dead reachable",
        "dead incoming branch",
        "dead exception target",
        "dead extra alias",
        "dead wrong origin",
        "dead missing named target",
        "dead missing cleanup marker",
        "dead wrong binding",
        "dead changed owner",
        "dead changed copy policy",
        "dead third edge",
    ] {
        let mut assembler = Assembler::default();
        let failure = assembler.label();
        let join = assembler.label();
        let source = assembler.label();
        let handler = assembler.label();
        let target = assembler.label();
        let entry = assembler.label();
        assembler.set_location(pattern);
        assembler.emit_forward_with_depth(
            POP_JUMP_IF_FALSE,
            failure,
            0,
            ExceptionPolicy::default(),
        );
        assembler.emit_with_depth(NOT_TAKEN, 0, 0, ExceptionPolicy::default());
        assembler.set_location(SourceLocation::new(4, 4, 12, 19));
        assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
        assembler.emit_with_depth(POP_TOP, 0, 0, ExceptionPolicy::default());
        let success =
            assembler.emit_forward_with_depth(JUMP_FORWARD, join, 0, ExceptionPolicy::default());
        assembler.record_original_match_success(success, false);
        assembler.mark_jump_as_interruptible(success);
        assembler.defer_jump_removal(success);
        assembler.mark(failure);
        assembler.mark_original_match_join(join);
        if mutation == "source extra alias" {
            let extra = assembler.label();
            assembler.mark(extra);
        }
        assembler.mark(source);
        assembler.set_location(SourceLocation::NONE);
        let owner = if mutation == "source pop owner" {
            ExceptionPolicy::default()
        } else {
            ExceptionPolicy::EXCLUDE_ALWAYS
        };
        assembler.emit_with_depth(NOP, 0, 0, owner);
        assembler.mark_last_as_converted_pop_block();
        let exit =
            assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
        assembler.prepare_no_location_block_for_inlining(exit);
        if mutation != "source copy policy" {
            assembler.allow_last_unconditional_block_inlining();
        }
        assembler.prevent_jump_inlining(exit);
        if mutation == "source jump location" {
            assembler
                .emitted_jump_mut(exit, "changing source location")
                .location = pattern;
        }
        assembler.record_compound_try_owner_exit(source, target, None);
        if mutation == "duplicate origin" {
            assembler.record_compound_try_owner_exit(source, target, None);
        }
        assembler.mark(handler);
        if mutation.starts_with("dead ") {
            for _ in 0..if mutation == "dead third edge" { 2 } else { 1 } {
                let body_start = assembler.label();
                let body_end = assembler.label();
                let name_cleanup = assembler.label();
                assembler.mark(body_start);
                assembler.set_location(SourceLocation::new(6, 8, 8, 22));
                if mutation == "dead incoming branch" {
                    assembler.emit_forward_with_depth(
                        POP_JUMP_IF_FALSE,
                        body_end,
                        1,
                        ExceptionPolicy::default(),
                    );
                }
                assembler.emit_with_depth(
                    if mutation == "dead reachable" {
                        POP_TOP
                    } else {
                        RAISE_VARARGS
                    },
                    if mutation == "dead reachable" { 0 } else { 2 },
                    1,
                    ExceptionPolicy::default(),
                );
                assembler.mark(body_end);
                if mutation == "dead extra alias" {
                    let alias = assembler.label();
                    assembler.mark(alias);
                }
                if mutation != "dead missing cleanup marker" {
                    assembler.record_unlocated_cleanup_targets();
                }
                assembler.emit_with_depth(
                    POP_EXCEPT,
                    0,
                    0,
                    if mutation == "dead changed owner" {
                        ExceptionPolicy::EXCLUDE_ALWAYS
                    } else {
                        ExceptionPolicy::default()
                    },
                );
                assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
                assembler.emit_with_depth(STORE_FAST, 2, 0, ExceptionPolicy::default());
                assembler.emit_with_depth(
                    DELETE_FAST,
                    if mutation == "dead wrong binding" {
                        3
                    } else {
                        2
                    },
                    0,
                    ExceptionPolicy::default(),
                );
                let dead_index = assembler.item_count();
                let dead_exit = assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    target,
                    0,
                    ExceptionPolicy::default(),
                );
                assembler.prevent_jump_inlining(dead_exit);
                if mutation == "dead changed copy policy" {
                    assembler.prepare_no_location_block_for_inlining(dead_exit);
                }
                if mutation != "dead missing named target" {
                    assembler.record_named_handler_cleanup_target(name_cleanup);
                }
                assembler.add_exception_region(body_start, body_end, name_cleanup, 1, true);
                if mutation == "dead wrong origin" {
                    assembler.exception_regions.last_mut().unwrap().priority =
                        super::ExceptionRegionPriority::OriginalRange(handler, body_end);
                }
                if mutation == "dead exception target" {
                    assembler.add_exception_region(body_start, body_end, body_end, 1, true);
                }
                assembler.mark(name_cleanup);
                assembler.set_location(SourceLocation::NONE);
                assembler.emit_with_depth(RERAISE, 1, 0, ExceptionPolicy::default());
                if mutation == "dead third edge" {
                    assert!(
                        assembler.is_original_unreachable_named_cleanup_exit(dead_index, target)
                    );
                }
            }
        } else {
            assembler.emit_with_depth(RERAISE, 0, 0, ExceptionPolicy::default());
        }
        assembler.mark(target);
        assembler
            .exception_owner_provenance
            .block_boundaries
            .insert(target);
        if mutation == "target extra alias" {
            let extra = assembler.label();
            assembler.mark(extra);
        }
        assembler.mark(entry);
        assembler.prevent_borrow_finally_fallthrough(entry);
        assembler.prevent_borrow_fallthrough_reachability(entry);
        if mutation == "target foreign entry" {
            assembler.prevent_borrow_reachability(target);
        }
        if mutation == "target setup" {
            assembler.record_exception_setup(entry, true);
        }
        let pop_ordinal = assembler.instruction_count();
        let owner = if mutation == "target pop owner" {
            ExceptionPolicy::default()
        } else {
            ExceptionPolicy::EXCLUDE_ALWAYS
        };
        assembler.emit_with_depth(NOP, 0, 0, owner);
        assembler.mark_last_as_converted_pop_block();
        assembler.set_location(if mutation == "same donor line" {
            pattern
        } else {
            SourceLocation::new(9, 9, 8, 12)
        });
        assembler.emit_with_depth(LOAD_FAST, 0, 1, ExceptionPolicy::default());
        if let Some(Item::Instruction(first)) = assembler.linear_items_mut().last_mut() {
            if mutation == "owned first load" {
                first.local_policy = LocalPolicy::for_load(true, false);
            }
            if mutation == "hidden first slot" {
                first.local_policy.add_hidden_exception_owner_slots(1);
            }
        }
        assembler.emit_with_depth(LOAD_ATTR, 0, 1, ExceptionPolicy::default());
        assembler.emit_with_depth(LOAD_ATTR, 1, 2, ExceptionPolicy::default());
        assembler.emit_with_depth(CALL, 0, 1, ExceptionPolicy::default());
        if mutation != "short cleanup" {
            assembler.emit_with_depth(GET_AWAITABLE, 0, 1, ExceptionPolicy::default());
        }
        if mutation == "late success edge" {
            let late = assembler.emit_forward_with_depth(
                JUMP_FORWARD,
                join,
                0,
                ExceptionPolicy::default(),
            );
            assembler.record_original_match_success(late, false);
            assembler.mark_jump_as_interruptible(late);
        }
        if mutation == "python315" {
            assembler.python_version = crate::PythonVersion::Py315;
        }
        let items = assembler.linear_items().to_vec();
        let origins = assembler
            .exception_owner_provenance
            .compound_try_owner_exits
            .clone();
        assembler.admit_retained_match_finally_entry(entry, pop_ordinal, pattern);
        let admitted = matches!(mutation, "original" | "dead original");
        assert_eq!(
            assembler.borrow_unreachable_blocks.contains(&entry),
            !admitted,
            "{mutation}"
        );
        assert_eq!(
            assembler.borrow_finally_entries.contains_key(&entry),
            !admitted,
            "{mutation}"
        );
        assert_eq!(
            assembler.borrow_jump_entry_labels.contains(&entry),
            !admitted,
            "{mutation}"
        );
        assert_eq!(
            assembler.borrow_block_aliases.contains(&entry),
            admitted,
            "{mutation}"
        );
        assert_eq!(assembler.linear_items(), items, "{mutation}");
        assert_eq!(
            assembler
                .exception_owner_provenance
                .compound_try_owner_exits,
            origins,
            "{mutation}"
        );
    }
}

fn pattern_capture_swap_pipeline() -> super::OptimizationPipeline {
    optimization_pipeline(
        [
            (UNPACK_SEQUENCE, 4),
            (SWAP, 4),
            (SWAP, 3),
            (SWAP, 2),
            (SWAP, 4),
            (SWAP, 3),
            (SWAP, 2),
            (POP_TOP, 0),
            (SWAP, 3),
            (SWAP, 2),
            (STORE_FAST, 1),
            (STORE_FAST, 2),
            (STORE_FAST, 3),
        ]
        .into_iter()
        .map(|(opcode, argument)| pass_instruction(opcode, argument, 3))
        .collect(),
    )
}

#[test]
fn static_swap_store_gaps_preserve_original_slots_until_fusion() {
    let mut pipeline = pattern_capture_swap_pipeline();
    let original_len = pipeline.graph.iter_items().count();
    let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
    let gaps = pipeline.apply_static_swaps((blocks, cleanup));
    assert_eq!(gaps.len(), 1);
    assert_eq!(pipeline.graph.iter_items().count(), original_len);
    assert!(pipeline.compact_static_swap_store_gaps(gaps));
    assert_eq!(pipeline.graph.iter_items().count(), original_len - 2);
    pipeline.fuse_superinstructions();
    assert_eq!(
        graph_opcodes(&pipeline),
        [
            UNPACK_SEQUENCE,
            NOP,
            NOP,
            NOP,
            NOP,
            NOP,
            NOP,
            STORE_FAST_STORE_FAST,
            POP_TOP,
            STORE_FAST,
        ],
    );
}

#[test]
fn static_swap_store_gaps_accept_only_transported_copy_entry_labels() {
    let mut pipeline = pattern_capture_swap_pipeline();
    let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
    let mut gaps = pipeline.apply_static_swaps((blocks, cleanup));
    let block = pipeline.graph.order()[0];
    let entry = super::Label::from_u32(0);
    for gap in &mut gaps {
        gap.prepend_copy_entry(&pipeline.graph, block, entry);
    }
    pipeline.graph.insert_label(block, 0, entry);
    assert!(pipeline.compact_static_swap_store_gaps(gaps));
    assert_eq!(pipeline.graph[block].first_label(), Some(entry));
}

#[test]
fn static_swap_store_gaps_require_unchanged_original_prefixes() {
    for mutation in 0..4 {
        let mut pipeline = pattern_capture_swap_pipeline();
        let (blocks, cleanup, _) = pipeline.optimize_swap_runs();
        let gaps = pipeline.apply_static_swaps((blocks, cleanup));
        assert_eq!(gaps.len(), 1);
        let block = pipeline.graph.order()[0];
        let positions = pipeline.graph.item_positions();
        match mutation {
            0 => {
                pipeline
                    .graph
                    .instruction_mut(positions[0])
                    .unwrap()
                    .location
                    .line += 1;
            }
            1 => {
                pipeline
                    .graph
                    .instruction_mut(positions[8])
                    .unwrap()
                    .location
                    .column += 1;
            }
            2 => pipeline
                .graph
                .insert_label(block, 0, super::Label::from_u32(0)),
            3 => {
                let copied = pipeline.graph[block].items().to_vec();
                pipeline.graph.retain_order(|candidate| candidate != block);
                pipeline.graph.push_block(copied);
            }
            _ => unreachable!(),
        }
        let before = pipeline.graph.copy_items();
        assert!(!pipeline.compact_static_swap_store_gaps(gaps));
        assert_eq!(pipeline.graph.copy_items(), before);
    }
}

#[test]
fn backward_conditional_normalization_preserves_successor_labels() {
    for labelled_successor in [false, true] {
        let restart = super::Label::from_u32(0);
        let mut items = vec![
            Item::Label(restart),
            pass_instruction(LOAD_FAST, 0, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Backward(restart),
                SourceLocation::new(1, 1, 0, 1),
                None,
            )),
        ];
        if labelled_successor {
            items.push(Item::Label(super::Label::from_u32(1)));
        }
        items.extend([
            pass_instruction(NOT_TAKEN, 0, 1),
            pass_instruction(LOAD_CONST, 0, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]);
        let mut pipeline = optimization_pipeline(items);

        assert!(pipeline.normalize_backward_conditionals().unwrap());
        assert!(!pipeline.normalize_backward_conditionals().unwrap());

        pipeline.graph.validate().unwrap();
        for item in pipeline.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(target) | Operand::Backward(target) = instruction.operand()
            {
                assert!(pipeline.graph.label_block(target).is_some());
            }
        }
        assert_eq!(
            graph_opcodes(&pipeline),
            [
                LOAD_FAST,
                POP_JUMP_IF_TRUE,
                NOT_TAKEN,
                JUMP_BACKWARD,
                LOAD_CONST,
                RETURN_VALUE
            ],
        );
    }
}

#[test]
fn assertion_side_exit_fast_path_accepts_all_conditional_directions() {
    let targets = [
        super::Label::from_u32(0),
        super::Label::from_u32(1),
        super::Label::from_u32(2),
        super::Label::from_u32(3),
    ];
    let mut items = Vec::new();
    for (line, (opcode, target)) in [
        POP_JUMP_IF_TRUE,
        POP_JUMP_IF_FALSE,
        POP_JUMP_IF_NONE,
        POP_JUMP_IF_NOT_NONE,
    ]
    .into_iter()
    .zip(targets)
    .enumerate()
    {
        let line = i32::try_from(line + 1).unwrap();
        items.push(pass_instruction(LOAD_FAST, 0, line));
        items.push(Item::Instruction(Instruction::new(
            opcode,
            Operand::Forward(target),
            SourceLocation::new(line, line, 0, 0),
            None,
        )));
        items.push(pass_instruction(NOT_TAKEN, 0, line));
        items.push(pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            line,
        ));
        items.push(pass_instruction(RAISE_VARARGS, 1, line));
        items.push(Item::Label(target));
    }
    items.push(pass_instruction(RETURN_VALUE, 0, 4));

    let mut assembler = optimization_pipeline(items);
    assembler.has_assertion_side_exit_candidate = true;
    assert!(assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_mixed_control_flow() {
    let assertion_target = super::Label::from_u32(0);
    let other_target = super::Label::from_u32(1);
    let assertion = [
        pass_instruction(LOAD_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(assertion_target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(assertion_target),
    ];

    let mut unconditional = assertion.to_vec();
    unconditional.push(Item::Instruction(Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(other_target),
        SourceLocation::NONE,
        None,
    )));
    unconditional.push(Item::Label(other_target));
    unconditional.push(pass_instruction(RETURN_VALUE, 0, 2));
    let mut unconditional = optimization_pipeline(unconditional);
    unconditional.has_assertion_side_exit_candidate = true;
    assert!(!unconditional.has_only_assertion_side_exits());

    let mut conditional = assertion.to_vec();
    conditional.push(pass_instruction(LOAD_FAST, 1, 2));
    conditional.push(Item::Instruction(Instruction::new(
        POP_JUMP_IF_FALSE,
        Operand::Forward(other_target),
        SourceLocation::new(2, 2, 0, 0),
        None,
    )));
    conditional.push(pass_instruction(NOT_TAKEN, 0, 2));
    conditional.push(pass_instruction(LOAD_CONST, 0, 2));
    conditional.push(Item::Label(other_target));
    conditional.push(pass_instruction(RETURN_VALUE, 0, 2));
    let mut conditional = optimization_pipeline(conditional);
    conditional.has_assertion_side_exit_candidate = true;
    assert!(!conditional.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_single_block_without_scanning() {
    let assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert_eq!(assembler.graph.len(), 1);
    assert!(!assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_annotation_guard_without_scanning() {
    let target = super::Label::from_u32(0);
    let assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(LOAD_GLOBAL, 0, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.graph.len() >= 3);
    assert!(!assembler.has_assertion_side_exit_candidate);
    assert!(!assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_exception_regions() {
    let target = super::Label::from_u32(0);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);
    assembler.exception_regions.push(super::ExceptionRegion {
        priority: super::ExceptionRegionPriority::OriginalRange(target, target),
        is_scope_fallback: false,
        start: target,
        end: target,
        target,
        depth: 0,
        preserve_lasti: false,
    });
    assembler.has_assertion_side_exit_candidate = true;

    assert!(!assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_propagates_final_condition_location() {
    let target = super::Label::from_u32(0);
    let location = SourceLocation::new(7, 7, 4, 19);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 7),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            location,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 7),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            7,
        ),
        pass_instruction(RAISE_VARARGS, 1, 7),
        Item::Label(target),
        Item::Instruction(Instruction::new(
            LOAD_CONST,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )),
        Item::Instruction(Instruction::new(
            RETURN_VALUE,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )),
    ]);
    assembler.has_assertion_side_exit_candidate = true;
    assert!(assembler.has_only_assertion_side_exits());

    assembler.propagate_assertion_exit_locations();

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions[instructions.len() - 2].location, location);
}

fn graph_opcodes(assembler: &super::OptimizationPipeline) -> Vec<crate::target::Opcode> {
    assembler
        .graph
        .iter_items()
        .filter_map(|item| {
            let Item::Instruction(instruction) = item else {
                return None;
            };
            Some(instruction.opcode)
        })
        .collect()
}

#[test]
fn threads_unconditional_forward_jumps_without_conditional_jumps() {
    let first_target = super::Label::from_u32(0);
    let final_target = super::Label::from_u32(1);
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(first_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(first_target),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.thread_forward_jumps());

    let graph = &assembler.graph;
    let first = graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .unwrap();
    assert!(matches!(first.operand(), Operand::Forward(label) if label == final_target));
}

#[test]
fn folded_condition_loop_tails_require_original_provenance_before_threading() {
    for version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        for source_opcode in [JUMP_FORWARD, JUMP_BACKWARD_NO_INTERRUPT] {
            for case in [
                "excluded condition",
                "default policy",
                "missing marker",
                "source in another array",
                "different marker span",
                "while source boundary",
                "while exit anchor",
                "ambiguous sources",
                "changed exception owner",
                "changed jump policy",
                "changed location policy",
                "hidden original slot",
                "hidden owner slot",
            ] {
                let mut assembler = Assembler {
                    python_version: version,
                    ..Assembler::default()
                };
                let head = assembler.label();
                let incoming = assembler.label();
                let continuation = assembler.label();
                let marker = assembler.label();
                let fallthrough = assembler.label();
                let discarded = assembler.label();
                let end = assembler.label();
                let location = SourceLocation::new(20, 21, 8, 19);
                assembler.record_folded_condition_predecessor(discarded);
                if case == "while exit anchor" {
                    assembler.record_folded_loop_exit_anchor(discarded);
                }
                if case == "source in another array" {
                    let foreign = assembler.label();
                    let foreign_end = assembler.label();
                    assembler.mark(foreign);
                    assembler.set_location(location);
                    assembler.emit_with_depth(NOP, 0, 1, ExceptionPolicy::default());
                    assembler.record_folded_predecessor_source(foreign, discarded);
                    assembler.mark(foreign_end);
                    let removed = assembler.take_trailing_nop_location().unwrap();
                    assert_eq!(removed.original_slots, 0);
                    assembler.emit_with_depth(LOAD_CONST, 0, 2, ExceptionPolicy::default());
                    assembler.emit_with_depth(POP_TOP, 0, 1, ExceptionPolicy::default());
                }
                assembler.mark(head);
                assembler.record_exception_owner_block_boundary(head);
                assembler.set_location(SourceLocation::new(2, 2, 4, 12));
                assembler.emit_forward_with_depth(FOR_ITER, end, 2, ExceptionPolicy::default());
                assembler.mark(incoming);
                let source = Instruction::new(
                    source_opcode,
                    Operand::Forward(continuation),
                    SourceLocation::new(10, 10, 8, 15),
                    Some(1),
                );
                assembler.push_linear_item(Item::Instruction(source));
                assembler.mark(continuation);
                assembler.record_exception_owner_block_boundary(continuation);
                assembler.mark(marker);
                if case == "while source boundary" {
                    assembler.record_exception_owner_block_boundary(marker);
                }
                let second_marker = (case == "ambiguous sources").then(|| assembler.label());
                if let Some(second_marker) = second_marker {
                    assembler.mark(second_marker);
                }
                assembler.set_location(location);
                assembler.emit_with_depth(NOP, 0, 1, ExceptionPolicy::default());
                if !matches!(
                    case,
                    "default policy" | "missing marker" | "source in another array"
                ) {
                    assembler.record_folded_predecessor_source(marker, discarded);
                }
                if let Some(second_marker) = second_marker {
                    assembler.record_folded_predecessor_source(second_marker, discarded);
                }
                assembler.record_exception_owner_block_boundary(fallthrough);
                assembler.mark(fallthrough);
                // Exercise the same early NOP removal that exposes the loop backedge.
                // Reuse this IR on legacy targets to check that the new policy stays modern.
                let removed = assembler.take_trailing_nop_location().unwrap();
                assert_eq!(removed.location, location);
                assert_eq!(removed.original_slots, 0);
                let tail_location = if case == "different marker span" {
                    SourceLocation::new(20, 21, 9, 19)
                } else {
                    removed.location
                };
                let mut tail = Instruction::new(
                    JUMP_BACKWARD,
                    Operand::Backward(head),
                    tail_location,
                    Some(1),
                );
                tail.set_exception_policy(if case == "default policy" {
                    ExceptionPolicy::default()
                } else {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                });
                match case {
                    "changed exception owner" => {
                        tail.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
                    }
                    "changed jump policy" => tail.jump_policy.prevent_small_exit_inlining(),
                    "changed location policy" => tail.no_location_policy.preserve_location(),
                    "hidden original slot" => tail.add_pre_fold_extra_instructions(1),
                    "hidden owner slot" => tail.local_policy.add_hidden_exception_owner_slots(1),
                    _ => {}
                }
                assembler.push_linear_item(Item::Instruction(tail));
                assembler.mark(discarded);
                assembler.mark(end);
                assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());

                let mut pipeline = assembler.into_optimization_pipeline();
                let source_block = pipeline.graph.label_block(incoming).unwrap();
                let tail_block = pipeline.graph.label_block(continuation).unwrap();
                let original_tail = pipeline.graph[tail_block].items().to_vec();
                let retained = case == "default policy"
                    || (case == "excluded condition" && version >= crate::PythonVersion::Py314);
                assert_eq!(
                    pipeline.thread_forward_jumps(),
                    !retained,
                    "{version:?}, {source_opcode:?}, {case}",
                );
                let mut expected = source;
                if !retained {
                    expected.retarget(Operand::Backward(head));
                    expected.opcode = JUMP_BACKWARD;
                    expected.add_pre_fold_extra_instructions(1);
                    if source_opcode != JUMP_FORWARD {
                        expected.jump_policy.mark_interruptible();
                    }
                }
                assert_eq!(
                    pipeline.graph[source_block].last_instruction(),
                    Some(&expected),
                    "incoming jump: {version:?}, {source_opcode:?}, {case}",
                );
                assert_eq!(
                    pipeline.graph[tail_block].items(),
                    original_tail,
                    "tail array: {version:?}, {source_opcode:?}, {case}",
                );
            }
        }
    }
}

#[test]
fn threads_forward_operands_with_backward_jump_kinds() {
    for source_opcode in [JUMP_FORWARD, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT] {
        for intermediate in [None, Some(JUMP_BACKWARD)] {
            let entry = super::Label::from_u32(0);
            let source_label = super::Label::from_u32(1);
            let first_target = super::Label::from_u32(2);
            let last_target = super::Label::from_u32(3);
            let mut source = Instruction::new(
                source_opcode,
                Operand::Forward(first_target),
                SourceLocation::new(10, 10, 2, 9),
                Some(0),
            );
            source.mark_initial_borrow_jump_target(first_target);
            source.jump_policy.retain_nop_for_direct_target();
            let mut items = vec![
                Item::Label(entry),
                pass_instruction(NOP, 0, 1),
                Item::Label(source_label),
                Item::Instruction(source),
                Item::Label(first_target),
            ];
            if let Some(opcode) = intermediate {
                items.extend([
                    Item::Instruction(Instruction::new(
                        opcode,
                        Operand::Forward(last_target),
                        SourceLocation::NONE,
                        None,
                    )),
                    Item::Label(last_target),
                ]);
            }
            items.push(Item::Instruction(Instruction::new(
                JUMP_BACKWARD_NO_INTERRUPT,
                Operand::Backward(entry),
                SourceLocation::NONE,
                None,
            )));
            let mut assembler = optimization_pipeline(items);
            assert!(assembler.exception_regions.is_empty());
            assert!(assembler.thread_forward_jumps());
            let block = assembler.graph.label_block(source_label).unwrap();
            let actual = *assembler.graph[block].last_instruction().unwrap();
            let keeps_interrupt = source_opcode != JUMP_FORWARD
                && (source_opcode == JUMP_BACKWARD || intermediate.is_some());
            let mut expected = source;
            expected.add_pre_fold_extra_instructions(1 + u8::from(intermediate.is_some()));
            expected.retarget(Operand::Backward(entry));
            expected.opcode = if keeps_interrupt {
                JUMP_BACKWARD
            } else {
                JUMP_BACKWARD_NO_INTERRUPT
            };
            if keeps_interrupt {
                expected.jump_policy.mark_interruptible();
            }
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        }
    }
}

#[test]
fn forward_operand_threading_preserves_barriers_and_unthreaded_metadata() {
    for control in 0..4 {
        let entry = super::Label::from_u32(0);
        let source_label = super::Label::from_u32(1);
        let target = super::Label::from_u32(2);
        let mut source = Instruction::new(
            JUMP_BACKWARD,
            if control == 2 {
                Operand::Backward(target)
            } else {
                Operand::Forward(target)
            },
            SourceLocation::new(10, 10, 2, 9),
            Some(0),
        );
        let mut destination = Instruction::new(
            if control == 3 {
                RETURN_VALUE
            } else {
                JUMP_BACKWARD_NO_INTERRUPT
            },
            if control == 3 {
                Operand::Value(0)
            } else {
                Operand::Backward(entry)
            },
            SourceLocation::NONE,
            None,
        );
        source.jump_policy.retain_nop_for_direct_target();
        if control == 0 {
            source.jump_policy.retain_nop();
        }
        if control == 1 {
            destination.jump_policy.retain_nop();
        }
        let mut assembler = optimization_pipeline(vec![
            Item::Label(entry),
            pass_instruction(NOP, 0, 1),
            Item::Label(source_label),
            Item::Instruction(source),
            Item::Label(target),
            Item::Instruction(destination),
        ]);
        let original = format!("{:?}", assembler.graph.iter_items().collect::<Vec<_>>());
        assert!(!assembler.thread_forward_jumps(), "control {control}");
        assert_eq!(
            format!("{:?}", assembler.graph.iter_items().collect::<Vec<_>>()),
            original,
            "control {control}",
        );
    }
}

#[test]
fn a_threaded_target_cycle_keeps_direct_target_retention() {
    let first = super::Label::from_u32(0);
    let second = super::Label::from_u32(1);
    let mut source = Instruction::new(
        JUMP_BACKWARD,
        Operand::Forward(first),
        SourceLocation::new(10, 10, 2, 9),
        Some(0),
    );
    source.jump_policy.retain_nop_for_direct_target();
    let mut interruptible_target = Instruction::new(
        JUMP_BACKWARD_NO_INTERRUPT,
        Operand::Forward(first),
        SourceLocation::NONE,
        None,
    );
    interruptible_target.jump_policy.mark_interruptible();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(source),
        Item::Label(first),
        Item::Instruction(Instruction::new(
            JUMP_BACKWARD_NO_INTERRUPT,
            Operand::Forward(second),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(second),
        Item::Instruction(interruptible_target),
    ]);
    assembler.thread_forward_jumps();
    let actual = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) => Some(*instruction),
            Item::Label(_) => None,
        })
        .unwrap();
    let mut expected = source;
    expected.add_pre_fold_extra_instructions(2);
    // The bounded walk returns to the same forward operand, so retargeting has
    // not cleared the original direct-target NOP policy. Inherited interrupt
    // checks must not overwrite it or trigger mark_interruptible's assertion.
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
}

#[test]
fn threads_unconditional_forward_jumps_without_conditional_targets() {
    let conditional_target = super::Label::from_u32(0);
    let first_target = super::Label::from_u32(1);
    let final_target = super::Label::from_u32(2);
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(conditional_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(first_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(first_target),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 1),
        Item::Label(conditional_target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.thread_forward_jumps());

    let graph = &assembler.graph;
    let first_jump = graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.opcode == JUMP_FORWARD => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert!(matches!(first_jump.operand(), Operand::Forward(label) if label == final_target));
}

#[test]
fn threads_opposite_conditional_jumps_through_a_new_fallthrough_label() {
    let inner_target = super::Label::from_u32(0);
    let final_target = super::Label::from_u32(1);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(COPY, 1, 1),
        pass_instruction(TO_BOOL, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(inner_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(POP_TOP, 0, 1),
        Item::Label(inner_target),
        pass_instruction(COPY, 1, 2),
        pass_instruction(TO_BOOL, 0, 2),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 2),
        pass_instruction(POP_TOP, 0, 2),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 3),
    ]);

    assert!(assembler.thread_forward_jumps());

    let fallthrough = super::Label::from_u32(2);
    let items = assembler.graph.iter_items().copied().collect::<Vec<_>>();
    let jumps = items
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(instruction)
                if matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE) =>
            {
                Some(instruction.operand())
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        jumps,
        [
            Operand::Forward(fallthrough),
            Operand::Forward(final_target),
        ]
    );
    assert!(
        items
            .windows(2)
            .any(|items| matches!(items, [Item::Label(label), Item::Instruction(instruction)] if *label == fallthrough && instruction.opcode == POP_TOP))
    );
}

#[test]
fn threads_chained_conditional_jumps_to_the_final_target() {
    let middle_target = super::Label::from_u32(0);
    let inner_target = super::Label::from_u32(1);
    let final_target = super::Label::from_u32(2);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(COPY, 1, 1),
        pass_instruction(TO_BOOL, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(middle_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(POP_TOP, 0, 1),
        Item::Label(middle_target),
        pass_instruction(COPY, 1, 2),
        pass_instruction(TO_BOOL, 0, 2),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(inner_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 2),
        pass_instruction(POP_TOP, 0, 2),
        Item::Label(inner_target),
        pass_instruction(COPY, 1, 3),
        pass_instruction(TO_BOOL, 0, 3),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 3),
        pass_instruction(POP_TOP, 0, 3),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 4),
    ]);

    assert!(assembler.thread_forward_jumps());

    let jumps = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) if instruction.opcode == POP_JUMP_IF_FALSE => {
                Some(instruction.operand())
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(jumps, [Operand::Forward(final_target); 3]);
    let policies = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction.exception_policy()),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    for start in [0, 5] {
        assert_eq!(
            policies[start..start + 3],
            [ExceptionPolicy::EXCLUDE_ALWAYS; 3]
        );
        assert_eq!(policies[start + 3], ExceptionPolicy::default());
    }
    assert_eq!(policies[10..], [ExceptionPolicy::default(); 6]);
}

#[test]
fn threading_reports_no_edge_change_for_terminal_targets() {
    let target = super::Label::from_u32(0);
    let mut direct_jump = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);
    assert!(!direct_jump.thread_forward_jumps());

    let mut direct_condition = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(RETURN_VALUE, 0, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);
    assert!(!direct_condition.thread_forward_jumps());
}

#[test]
fn threaded_edge_change_retains_second_reachability_cleanup_and_removed_depth() {
    let bypassed = super::Label::from_u32(0);
    let target = super::Label::from_u32(1);
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(bypassed),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(bypassed),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            Some(7),
        )),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.thread_forward_jumps());
    assert_eq!(assembler.remove_unreachable_instructions(), Some(7));
    assert_eq!(graph_opcodes(&assembler), [JUMP_FORWARD, RETURN_VALUE]);
}

#[test]
fn redundant_jump_removal_preserves_borrow_traversal_barrier() {
    let target = super::Label::from_u32(0);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    jump.no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(jump),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0].opcode, LOAD_CONST);
    assert_eq!(
        instructions[0].no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(instructions[1].opcode, RETURN_VALUE);
}

#[test]
fn redundant_jump_removal_streams_consecutive_cross_block_jumps() {
    let intervening_label = super::Label::from_u32(0);
    let first_target = super::Label::from_u32(1);
    let second_target = super::Label::from_u32(2);
    let location = SourceLocation::new(7, 7, 0, 0);
    let mut first = Instruction::new(JUMP_FORWARD, Operand::Forward(first_target), location, None);
    first.exclude_from_exception_table(ExceptionExclusion::IfExtended);
    first
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let second = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(second_target),
        SourceLocation::NONE,
        None,
    );
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 7),
        Item::Instruction(first),
        Item::Label(intervening_label),
        Item::Label(first_target),
        Item::Instruction(second),
        Item::Label(second_target),
        Item::Instruction(Instruction::new(
            RETURN_VALUE,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0].opcode, LOAD_CONST);
    assert_eq!(
        instructions[0].exception_policy().exclusion(),
        ExceptionExclusion::IfExtended
    );
    assert_eq!(
        instructions[0].no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(instructions[1].opcode, RETURN_VALUE);
    assert_eq!(instructions[1].location, location);
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>(),
        [intervening_label, first_target, second_target]
    );
}

#[test]
fn deferred_nonadjacent_forward_jump_is_still_resolved() {
    let target = super::Label::from_u32(0);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    jump.jump_policy.defer_removal();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(jump),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assembler.remove_redundant_forward_jumps(false);

    let jump = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.opcode == JUMP_FORWARD => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert_eq!(jump.operand(), Operand::Forward(target));
    assert!(!jump.jump_policy.defers_redundant_removal());
}

#[test]
fn borrow_empty_block_marker_survives_until_local_analysis() {
    let target = super::Label::from_u32(0);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    jump.jump_policy.mark_borrow_empty_block();
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(jump),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assembler.remove_redundant_forward_jumps(false);
    let retained = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.jump_policy.borrow_empty_block() => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert_eq!(retained.opcode, JUMP_FORWARD);

    assembler.remove_redundant_forward_jumps(true);
    let marker = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.jump_policy.borrow_empty_block() => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert_eq!(marker.opcode, NOP);

    let mut items = assembler.graph.into_items();
    super::OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
    assert_eq!(
        items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.opcode),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [LOAD_CONST, RETURN_VALUE]
    );
}

#[test]
fn uninitialized_load_analysis_follows_single_block_backedges() {
    let label = super::Label::from_u32(0);
    let mut assembler = optimization_pipeline(vec![
        Item::Label(label),
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(DELETE_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_BACKWARD,
            Operand::Backward(label),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
    ]);

    assembler.add_checks_for_uninitialized_loads(1, 1, &rustc_hash::FxHashSet::default());

    assert_eq!(
        graph_opcodes(&assembler),
        [LOAD_FAST_CHECK, DELETE_FAST, JUMP_BACKWARD]
    );
}

#[test]
fn uninitialized_load_analysis_resets_locals_above_the_fast_mask_at_block_entry() {
    let target = super::Label::from_u32(0);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(STORE_FAST, 63, 1),
        pass_instruction(STORE_FAST, 64, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(target),
        pass_instruction(LOAD_FAST, 63, 2),
        pass_instruction(LOAD_FAST, 64, 2),
        pass_instruction(LOAD_FAST, 64, 2),
    ]);

    assembler.add_checks_for_uninitialized_loads(65, 0, &rustc_hash::FxHashSet::default());

    assert_eq!(
        graph_opcodes(&assembler),
        [
            STORE_FAST,
            STORE_FAST,
            JUMP_FORWARD,
            LOAD_FAST,
            LOAD_FAST_CHECK,
            LOAD_FAST
        ]
    );
}

#[test]
fn assertion_uninitialized_load_analysis_preserves_low_and_high_local_semantics() {
    let target = super::Label::from_u32(0);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(STORE_FAST, 63, 1),
        pass_instruction(STORE_FAST, 64, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(target),
        pass_instruction(LOAD_FAST_CHECK, 63, 2),
        pass_instruction(LOAD_FAST, 64, 2),
        pass_instruction(LOAD_FAST_CHECK, 64, 2),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);
    assembler.has_assertion_side_exit_candidate = true;
    assert!(assembler.has_only_assertion_side_exits());

    assembler.add_checks_for_assertion_side_exits(65, 0);

    assert_eq!(
        graph_opcodes(&assembler),
        [
            STORE_FAST,
            STORE_FAST,
            POP_JUMP_IF_TRUE,
            NOT_TAKEN,
            LOAD_COMMON_CONSTANT,
            RAISE_VARARGS,
            LOAD_FAST,
            LOAD_FAST_CHECK,
            LOAD_FAST,
            RETURN_VALUE,
        ]
    );
}

#[test]
fn linear_assertion_optimizer_matches_the_block_graph_pipeline() {
    let target = super::Label::from_u32(0);
    let items = vec![
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(TO_BOOL, 0, 1),
        pass_instruction(TO_BOOL, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            Some(0),
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(target),
        pass_instruction(LOAD_CONST, 0, 2),
        pass_instruction(LOAD_CONST, 1, 2),
        pass_instruction(STORE_FAST, 1, 2),
        pass_instruction(STORE_FAST, 1, 2),
        pass_instruction(LOAD_FAST, 1, 3),
        pass_instruction(LOAD_FAST, 0, 3),
        pass_instruction(IS_OP, 0, 3),
        pass_instruction(RETURN_VALUE, 0, 3),
    ];
    let mut linear = Assembler::from_linear_items(items.clone());
    linear.next_label = 1;
    linear.has_assertion_side_exit_candidate = true;
    assert!(linear.has_only_linear_assertion_side_exits());
    let graph =
        super::block_graph::BlockGraph::from_items(items, &rustc_hash::FxHashSet::default());
    let reachable = graph.reachable_blocks(&[]);
    let generic = Assembler {
        code: UnoptimizedCode::Prepared { graph, reachable },
        next_label: 1,
        has_assertion_side_exit_candidate: true,
        ..Assembler::default()
    };
    assert!(!generic.has_only_linear_assertion_side_exits());

    let actual = linear.finish_code(1, 2, 1).unwrap();
    let expected = generic.finish_code(1, 2, 1).unwrap();

    assert_eq!(actual.bytecode, expected.bytecode);
    assert_eq!(actual.line_table, expected.line_table);
    assert_eq!(actual.exception_table, expected.exception_table);
    assert_eq!(actual.max_depth, expected.max_depth);
    assert_eq!(actual.removed_max_depth, expected.removed_max_depth);
}

#[test]
fn terminal_guard_optimizer_matches_the_graph_and_keeps_side_local_state_isolated() {
    for final_line in [-1, 4] {
        for terminal in [RETURN_VALUE, RAISE_VARARGS] {
            let first = super::Label::from_u32(0);
            let second = super::Label::from_u32(1);
            let value = |opcode, argument, line, depth| {
                Item::Instruction(Instruction::new(
                    opcode,
                    Operand::Value(argument),
                    SourceLocation::new(line, line, 0, 0),
                    Some(depth),
                ))
            };
            let jump = |opcode, target, line| {
                Item::Instruction(Instruction::new(
                    opcode,
                    Operand::Forward(target),
                    SourceLocation::new(line, line, 0, 0),
                    Some(0),
                ))
            };
            let build = || {
                let mut assembler = Assembler::from_linear_items(vec![
                    value(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
                    value(LOAD_FAST, 0, 1, 1),
                    jump(POP_JUMP_IF_FALSE, first, 1),
                    value(NOT_TAKEN, 0, 1, 0),
                    value(LOAD_CONST, 1, 2, 1),
                    value(STORE_FAST, 1, 2, 0),
                    value(LOAD_FAST, 1, 2, 1),
                    value(terminal, u32::from(terminal == RAISE_VARARGS), 2, 0),
                    Item::Label(first),
                    value(LOAD_FAST, 0, 3, 1),
                    jump(POP_JUMP_IF_NONE, second, 3),
                    value(NOT_TAKEN, 0, 3, 0),
                    value(DELETE_FAST, 0, 3, 0),
                    value(LOAD_CONST, 3, 3, 1),
                    value(RETURN_VALUE, 0, 3, 0),
                    Item::Label(second),
                    value(LOAD_CONST, 4, final_line, 1),
                    value(POP_TOP, 0, final_line, 0),
                    value(LOAD_FAST, 1, final_line, 1),
                    value(POP_TOP, 0, final_line, 0),
                    value(LOAD_FAST, 0, final_line, 1),
                    value(RETURN_VALUE, 0, final_line, 0),
                ]);
                assembler.next_label = 2;
                assembler
            };
            let mut linear = build();
            assert!(!linear.has_proven_linear_terminal_guards);
            assert!(linear.has_only_linear_terminal_guards());
            assert_eq!(
                linear.used_constant_indices(6),
                [false, true, false, true, true, false]
            );
            assert!(linear.has_proven_linear_terminal_guards);
            assert!(matches!(linear.code, UnoptimizedCode::Linear(_)));
            let remap = [Some(0), Some(1), None, Some(2), Some(3), None];
            linear.remap_constant_indices(&remap);
            assert!(
                super::terminal_guard_constant_indices(linear.linear_items(), &mut [])
                    .unwrap_or(false)
            );
            assert!(linear.has_only_linear_terminal_guards());

            let mut generic = build();
            generic.remap_constant_indices(&remap);
            let graph = super::block_graph::BlockGraph::from_items(
                generic.linear_items().to_vec(),
                &rustc_hash::FxHashSet::default(),
            );
            let reachable = graph.reachable_blocks(&[]);
            generic.code = UnoptimizedCode::Prepared { graph, reachable };
            let actual = linear.finish_code(1, 2, 1).unwrap();
            let expected = generic.finish_code(1, 2, 1).unwrap();
            assert_eq!(actual.bytecode, expected.bytecode);
            assert_eq!(actual.line_table, expected.line_table);
            assert_eq!(actual.exception_table, expected.exception_table);
            assert_eq!(actual.max_depth, expected.max_depth);
            assert_eq!(actual.removed_max_depth, expected.removed_max_depth);
        }
    }
}

#[test]
fn single_terminal_guards_keep_fallthrough_marker_aliases_and_match_the_graph() {
    let [target, alias, trailing, unexpected] = [0, 1, 2, 3].map(super::Label::from_u32);
    let value = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::new(line, line, 0, 5),
            Some(depth),
        ))
    };
    let branch = |opcode, target| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Forward(target),
            SourceLocation::new(2, 2, 0, 5),
            Some(0),
        ))
    };
    let make_items = |condition, terminal_side, trailing_label| {
        let mut items = vec![
            value(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
            value(LOAD_FAST, 0, 2, 1),
            branch(condition, target),
            value(NOT_TAKEN, 0, 2, 0),
            Item::Label(alias),
            value(LOAD_CONST, 2, 3, 1),
            value(STORE_FAST, 1, 3, 0),
            value(DELETE_FAST, 0, 3, 0),
        ];
        if terminal_side {
            items.extend([value(LOAD_FAST, 1, 4, 1), value(RETURN_VALUE, 0, 4, 0)]);
        }
        items.extend([
            Item::Label(target),
            value(LOAD_FAST, 1, 5, 1),
            value(POP_TOP, 0, 5, 0),
            value(LOAD_FAST, 0, 5, 1),
            value(POP_TOP, 0, 5, 0),
            value(LOAD_CONST, 3, 6, 1),
            value(RETURN_VALUE, 0, 6, 0),
        ]);
        if trailing_label {
            items.push(Item::Label(trailing));
        }
        items
    };
    let make_assembler = |items| {
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 3;
        assembler
    };
    for condition in [
        POP_JUMP_IF_FALSE,
        POP_JUMP_IF_TRUE,
        POP_JUMP_IF_NONE,
        POP_JUMP_IF_NOT_NONE,
    ] {
        for terminal_side in [false, true] {
            for trailing_label in [false, true] {
                let items = make_items(condition, terminal_side, trailing_label);
                assert!(super::terminal_guard_constant_indices(&items, &mut []).is_none());
                assert_eq!(
                    super::single_marker_alias_guard_constant_indices(&items, &mut []),
                    Some(true)
                );
                let mut linear = make_assembler(items.clone());
                assert_eq!(
                    linear.used_constant_indices(5),
                    [false, false, true, true, false]
                );
                assert!(matches!(linear.code, UnoptimizedCode::Linear(_)));
                assert!(linear.has_proven_linear_terminal_guards);
                assert_terminal_guards_match_graph(|| make_assembler(items.clone()), 2, 1);
            }
        }
    }
    let original = make_items(POP_JUMP_IF_FALSE, true, true);
    let mut too_many_labels = make_assembler(original.clone());
    too_many_labels.next_label = 5;
    assert!(!too_many_labels.has_only_linear_terminal_guards());
    let mut before_marker = original.clone();
    before_marker.swap(3, 4);
    let mut inside_side = original.clone();
    inside_side.insert(8, Item::Label(unexpected));
    let mut duplicate_target = original.clone();
    duplicate_target.push(Item::Label(target));
    let mut duplicate_alias = original.clone();
    duplicate_alias.push(Item::Label(alias));
    let mut second_jump = original;
    second_jump.insert(8, branch(POP_JUMP_IF_FALSE, unexpected));
    for items in [
        before_marker,
        inside_side,
        duplicate_target,
        duplicate_alias,
        second_jump,
    ] {
        assert_eq!(
            super::single_marker_alias_guard_constant_indices(&items, &mut []),
            None
        );
    }
}

#[test]
fn single_terminal_guards_keep_empty_stack_return_aliases_and_borrow_across_them() {
    let [target, marker_alias, side_alias, join_alias] = [0, 1, 2, 3].map(super::Label::from_u32);
    let value = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            if line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(line, line, 0, 8)
            },
            Some(depth),
        ))
    };
    let forced = |local, line| {
        let Item::Instruction(mut load) = value(LOAD_FAST, local, line, 1) else {
            unreachable!()
        };
        load.local_policy = LocalPolicy::for_load(true, false);
        Item::Instruction(load)
    };
    let make_items = |condition, side, join, previous_line, load_line, return_line| {
        let mut items = vec![
            value(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
            value(LOAD_FAST, 0, 2, 1),
            Item::Instruction(Instruction::new(
                condition,
                Operand::Forward(target),
                SourceLocation::new(2, 2, 0, 8),
                Some(0),
            )),
            value(NOT_TAKEN, 0, 2, 0),
            Item::Label(marker_alias),
            forced(1, previous_line),
            value(POP_TOP, 0, previous_line, 0),
        ];
        if side {
            items.push(Item::Label(side_alias));
        }
        items.extend([
            value(LOAD_CONST, 1, load_line, 1),
            value(RETURN_VALUE, 0, return_line, 0),
            Item::Label(target),
            forced(2, previous_line),
            value(POP_TOP, 0, previous_line, 0),
        ]);
        if join {
            items.push(Item::Label(join_alias));
        }
        items.extend([
            value(LOAD_CONST, 2, load_line, 1),
            value(RETURN_VALUE, 0, return_line, 0),
        ]);
        items
    };
    let assembler = |items, join| {
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = if join { 4 } else { 3 };
        assembler
    };
    for condition in [POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE] {
        for (side, join) in [(true, false), (false, true), (true, true)] {
            for previous_line in [-1, 4] {
                for load_line in [-1, 4] {
                    for return_line in [-1, 4] {
                        let items = make_items(
                            condition,
                            side,
                            join,
                            previous_line,
                            load_line,
                            return_line,
                        );
                        let same_located_source = previous_line >= 0
                            && previous_line == load_line
                            && load_line == return_line;
                        assert_eq!(
                            super::single_marker_alias_guard_constant_indices(&items, &mut []),
                            same_located_source.then_some(true)
                        );
                        if !same_located_source {
                            continue;
                        }
                        assert_terminal_guards_match_graph(|| assembler(items.clone(), join), 3, 3);
                        let actual = assembler(items, join).finish_code(1, 3, 3).unwrap();
                        for local in [1, 2] {
                            assert!(
                                actual
                                    .bytecode
                                    .as_chunks::<2>()
                                    .0
                                    .iter()
                                    .any(|pair| *pair == [LOAD_FAST_BORROW.code(), local])
                            );
                        }
                    }
                }
            }
        }
    }
    let original = make_items(POP_JUMP_IF_FALSE, true, false, 4, 4, 4);
    let tail = original
        .iter()
        .position(|item| *item == Item::Label(side_alias))
        .unwrap();
    for (relative, replacement) in [
        (-1_isize, value(POP_TOP, 0, 3, 1)),
        (1, value(LOAD_SMALL_INT, 1, 4, 1)),
        (1, value(LOAD_CONST, 1, 4, 2)),
        (2, value(RETURN_VALUE, 0, 4, 1)),
        (2, value(RAISE_VARARGS, 1, 4, 0)),
    ] {
        let mut changed = original.clone();
        let index = tail.checked_add_signed(relative).unwrap();
        changed[index] = replacement;
        assert_eq!(
            super::single_marker_alias_guard_constant_indices(&changed, &mut []),
            None
        );
    }
    for location in [
        SourceLocation::new(4, 4, 12, 18),
        SourceLocation::new(4, 5, 3, 11),
    ] {
        let mut matching = original.clone();
        for index in [tail - 1, tail + 1, tail + 2] {
            let Item::Instruction(instruction) = &mut matching[index] else {
                unreachable!()
            };
            instruction.location = location;
        }
        assert_eq!(
            super::single_marker_alias_guard_constant_indices(&matching, &mut []),
            Some(true)
        );
        assert_terminal_guards_match_graph(|| assembler(matching.clone(), false), 3, 3);
        for index in [tail - 1, tail + 1, tail + 2] {
            let mut mismatch = original.clone();
            let Item::Instruction(instruction) = &mut mismatch[index] else {
                unreachable!()
            };
            instruction.location = location;
            assert_eq!(
                super::single_marker_alias_guard_constant_indices(&mismatch, &mut []),
                None
            );
        }
    }
    let mut special_load = original;
    let Item::Instruction(load) = &mut special_load[tail + 1] else {
        unreachable!()
    };
    load.no_location_policy.preserve_location();
    assert_eq!(
        super::single_marker_alias_guard_constant_indices(&special_load, &mut []),
        None
    );
}

#[test]
fn terminal_guard_recognition_keeps_nested_and_nonterminal_branches_on_the_graph() {
    let first = super::Label::from_u32(0);
    let second = super::Label::from_u32(1);
    let jump = |target| {
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            Some(0),
        ))
    };
    for side in [
        vec![pass_instruction(POP_TOP, 0, 2)],
        vec![
            jump(second),
            pass_instruction(NOT_TAKEN, 0, 2),
            pass_instruction(RAISE_VARARGS, 0, 2),
            Item::Label(second),
            pass_instruction(RETURN_VALUE, 0, 2),
        ],
    ] {
        let mut items = vec![
            pass_instruction(LOAD_FAST, 0, 1),
            jump(first),
            pass_instruction(NOT_TAKEN, 0, 1),
        ];
        items.extend(side);
        items.extend([Item::Label(first), pass_instruction(RETURN_VALUE, 0, 3)]);
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 2;
        assert!(!assembler.has_only_linear_terminal_guards());
        assembler.used_constant_indices(2);
        assert!(!assembler.has_proven_linear_terminal_guards);
        assert!(!assembler.has_only_linear_terminal_guards());
    }
}

#[test]
fn linear_constant_return_matches_the_block_graph_pipeline() {
    let resume = SourceLocation::new(0, 1, 0, 0);
    let pass = SourceLocation::new(2, 2, 4, 8);
    let same_line = SourceLocation::new(2, 2, 12, 16);
    let following_line = SourceLocation::new(3, 3, 4, 12);
    for (nop, load, ret, opcode, argument, retains_nop) in [
        (None, following_line, following_line, LOAD_CONST, 0, false),
        (
            Some(pass),
            SourceLocation::NONE,
            SourceLocation::NONE,
            LOAD_CONST,
            0,
            false,
        ),
        (
            Some(pass),
            same_line,
            SourceLocation::NONE,
            LOAD_SMALL_INT,
            42,
            false,
        ),
        (
            Some(pass),
            following_line,
            SourceLocation::NONE,
            LOAD_CONST,
            513,
            true,
        ),
    ] {
        let mut items = vec![Item::Instruction(Instruction::new(
            RESUME,
            Operand::Value(ResumeLocation::AT_FUNC_START.argument()),
            resume,
            Some(0),
        ))];
        if let Some(nop) = nop {
            items.push(Item::Instruction(Instruction::new(
                NOP,
                Operand::Value(0),
                nop,
                Some(0),
            )));
        }
        items.extend([
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                load,
                Some(1),
            )),
            Item::Instruction(Instruction::new(
                RETURN_VALUE,
                Operand::Value(0),
                ret,
                Some(0),
            )),
        ]);
        let linear = Assembler::from_linear_items(items.clone());
        assert!(linear.has_only_linear_constant_return());
        let graph =
            super::block_graph::BlockGraph::from_items(items, &rustc_hash::FxHashSet::default());
        let reachable = graph.reachable_blocks(&[]);
        let generic = Assembler {
            code: UnoptimizedCode::Prepared { graph, reachable },
            ..Assembler::default()
        };
        assert!(!generic.has_only_linear_constant_return());

        let actual = linear.finish_code(1, 70, 65).unwrap();
        let expected = generic.finish_code(1, 70, 65).unwrap();
        assert_eq!(actual.bytecode, expected.bytecode);
        assert_eq!(actual.line_table, expected.line_table);
        assert_eq!(actual.exception_table, expected.exception_table);
        assert_eq!(actual.max_depth, expected.max_depth);
        assert_eq!(actual.removed_max_depth, expected.removed_max_depth);
        assert_eq!(
            actual
                .bytecode
                .as_chunks::<2>()
                .0
                .iter()
                .any(|instruction| instruction[0] == NOP.code()),
            retains_nop
        );
    }
}

#[test]
fn linear_constant_return_rejects_nontrivial_instructions_and_metadata() {
    let original = || {
        vec![
            pass_instruction(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1),
            pass_instruction(NOP, 0, 2),
            pass_instruction(LOAD_CONST, 0, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]
    };
    for replacement in [
        pass_instruction(LOAD_FAST, 0, 2),
        pass_instruction(UNPACK_SEQUENCE, 1, 2),
        pass_instruction(NOP, 0, -1),
        Item::Label(super::Label::from_u32(0)),
    ] {
        let mut items = original();
        items[1] = replacement;
        assert!(!Assembler::from_linear_items(items).has_only_linear_constant_return());
    }
    let mut items = original();
    let Item::Instruction(nop) = &mut items[1] else {
        unreachable!();
    };
    nop.jump_policy.preserve_explicit_nop();
    assert!(!Assembler::from_linear_items(items).has_only_linear_constant_return());

    let mut assembler = Assembler::from_linear_items(original());
    assembler.label();
    assert!(!assembler.has_only_linear_constant_return());
    let mut assembler = Assembler::from_linear_items(original());
    assembler.exception_owner_provenance.omitted_nop_tail =
        Some((1, SourceLocation::new(2, 2, 0, 0), 1));
    assert!(!assembler.has_only_linear_constant_return());
    let mut assembler = Assembler::from_linear_items(original());
    assembler.has_next_location = true;
    assert!(!assembler.has_only_linear_constant_return());
}

fn assert_linear_matches_graph(
    build: impl Fn() -> Assembler,
    local_count: usize,
    parameter_count: usize,
) {
    let linear = build();
    assert!(linear.has_only_linear_instructions(local_count));
    let mut generic = build();
    let graph = super::block_graph::BlockGraph::from_items(
        generic.linear_items().to_vec(),
        &rustc_hash::FxHashSet::default(),
    );
    let reachable = graph.reachable_blocks(&[]);
    generic.code = UnoptimizedCode::Prepared { graph, reachable };
    assert!(!generic.has_only_linear_instructions(local_count));

    let actual = linear.finish_code(1, local_count, parameter_count).unwrap();
    let expected = generic
        .finish_code(1, local_count, parameter_count)
        .unwrap();
    assert_eq!(actual.bytecode, expected.bytecode);
    assert_eq!(actual.line_table, expected.line_table);
    assert_eq!(actual.exception_table, expected.exception_table);
    assert_eq!(actual.max_depth, expected.max_depth);
    assert_eq!(actual.removed_max_depth, expected.removed_max_depth);
}

#[test]
fn ordinary_linear_blocked_swaps_match_the_block_graph_pipeline() {
    let item = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            if line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(line, line, 0, 8)
            },
            Some(depth),
        ))
    };
    let prefix = [
        item(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
        item(LOAD_FAST, 0, 2, 1),
        item(LOAD_FAST, 1, 2, 2),
    ];
    for line in [-1, 2, 3] {
        for (locals, parameters, body) in [
            (
                2,
                2,
                vec![
                    item(SWAP, 2, line, 2),
                    item(POP_TOP, 0, 3, 1),
                    item(IMPORT_FROM, 0, 3, 2),
                    item(POP_TOP, 0, 3, 1),
                ],
            ),
            (
                3,
                2,
                vec![
                    item(SWAP, 2, line, 2),
                    item(POP_TOP, 0, 3, 1),
                    item(IMPORT_FROM, 0, 4, 2),
                    item(STORE_FAST, 2, 4, 1),
                    item(STORE_FAST, 0, 5, 0),
                    item(LOAD_FAST, 2, 5, 1),
                ],
            ),
            (
                2,
                1,
                vec![item(SWAP, 2, line, 2), item(STORE_GLOBAL, 0, 3, 1)],
            ),
            (
                2,
                2,
                vec![
                    item(SWAP, 2, line, 2),
                    item(STORE_ATTR, 0, 3, 0),
                    item(LOAD_FAST, 0, 3, 1),
                ],
            ),
            (
                3,
                3,
                vec![
                    item(SWAP, 2, line, 2),
                    item(LOAD_FAST, 2, 3, 3),
                    item(BUILD_TUPLE, 3, 3, 1),
                ],
            ),
            (
                3,
                3,
                vec![
                    item(LOAD_FAST, 2, 2, 3),
                    item(SWAP, 3, line, 3),
                    item(POP_TOP, 0, 3, 2),
                    item(IMPORT_FROM, 0, 3, 3),
                    item(BUILD_TUPLE, 3, 3, 1),
                ],
            ),
        ] {
            let mut items = prefix.to_vec();
            items.extend(body);
            items.push(item(RETURN_VALUE, 0, 6, 0));
            assert_linear_matches_graph(
                || Assembler::from_linear_items(items.clone()),
                locals,
                parameters,
            );
        }
    }
}

#[test]
fn ordinary_linear_blocked_swaps_reject_rewritable_or_nonordinary_swaps() {
    let value = |opcode, argument, depth| {
        Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::new(2, 2, 0, 8),
            Some(depth),
        )
    };
    let item = |opcode, argument, depth| Item::Instruction(value(opcode, argument, depth));
    let prefix = [
        item(RESUME, ResumeLocation::AT_FUNC_START.argument(), 0),
        item(LOAD_FAST, 0, 1),
        item(LOAD_FAST, 1, 2),
    ];
    let mut marked = value(SWAP, 2, 2);
    marked.add_pre_fold_extra_instructions(1);
    for body in [
        vec![
            item(SWAP, 2, 2),
            item(STORE_FAST, 0, 1),
            item(STORE_FAST, 1, 0),
            item(LOAD_FAST, 0, 1),
        ],
        vec![
            item(SWAP, 2, 2),
            item(POP_TOP, 0, 1),
            item(POP_TOP, 0, 0),
            item(LOAD_FAST, 0, 1),
        ],
        vec![
            item(SWAP, 2, 2),
            item(POP_TOP, 0, 1),
            item(NOP, 0, 1),
            item(IMPORT_FROM, 0, 2),
            item(POP_TOP, 0, 1),
        ],
        vec![item(SWAP, 2, 2), item(TO_BOOL, 0, 2), item(POP_TOP, 0, 1)],
        vec![
            item(SWAP, 2, 2),
            item(LOAD_FAST, 0, 3),
            item(SWAP, 2, 3),
            item(STORE_GLOBAL, 0, 2),
            item(POP_TOP, 0, 1),
        ],
        vec![item(SWAP, 1, 2), item(STORE_GLOBAL, 0, 1)],
        vec![Item::Instruction(marked), item(STORE_GLOBAL, 0, 1)],
    ] {
        let mut items = prefix.to_vec();
        items.extend(body);
        items.push(item(RETURN_VALUE, 0, 0));
        assert!(!Assembler::from_linear_items(items).has_only_linear_instructions(2));
    }
}

#[test]
fn ordinary_linear_instructions_match_the_block_graph_pipeline() {
    let instruction = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::new(line, line, 0, 0),
            Some(depth),
        ))
    };
    let unlocated = |opcode, argument, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::NONE,
            Some(depth),
        ))
    };
    let resume = instruction(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0);
    let ret = instruction(RETURN_VALUE, 0, 8, 0);
    let scenarios = [
        (
            3,
            1,
            vec![
                instruction(LOAD_FAST, 0, 2, 1),
                instruction(UNPACK_SEQUENCE, 2, 2, 2),
                instruction(STORE_FAST, 1, 2, 1),
                instruction(STORE_FAST, 2, 2, 0),
                instruction(LOAD_FAST, 1, 3, 1),
                instruction(FORMAT_SIMPLE, 0, 3, 1),
                instruction(LOAD_FAST, 2, 3, 2),
                instruction(CONVERT_VALUE, Conversion::REPR.argument(), 3, 2),
                instruction(LOAD_CONST, 0, 3, 3),
                instruction(FORMAT_WITH_SPEC, 0, 3, 2),
                instruction(BUILD_STRING, 2, 3, 1),
            ],
        ),
        (
            3,
            1,
            vec![
                instruction(LOAD_FAST, 0, 2, 1),
                instruction(UNPACK_EX, 1, 2, 2),
                instruction(STORE_FAST, 1, 2, 1),
                instruction(STORE_FAST, 2, 2, 0),
                instruction(LOAD_FAST, 2, 3, 1),
                instruction(GET_ITER, 0, 3, 1),
            ],
        ),
        (
            0,
            0,
            vec![
                instruction(LOAD_SMALL_INT, 0, 2, 1),
                instruction(LOAD_CONST, 0, 2, 2),
                instruction(IMPORT_NAME, 0, 2, 1),
                instruction(IMPORT_FROM, 1, 2, 2),
                instruction(STORE_NAME, 1, 2, 1),
                instruction(POP_TOP, 0, 2, 0),
                instruction(LOAD_CONST, 1, 4, 1),
                unlocated(MAKE_FUNCTION, 0, 1),
                instruction(STORE_NAME, 2, 4, 0),
                instruction(LOAD_CONST, 0, 8, 1),
            ],
        ),
        (
            4,
            2,
            vec![
                instruction(NOP, 0, 2, 0),
                instruction(LOAD_FAST, 0, 3, 1),
                instruction(PUSH_NULL, 0, 3, 2),
                instruction(LOAD_FAST, 1, 3, 3),
                instruction(CALL, 1, 3, 1),
                unlocated(STORE_FAST, 2, 0),
                instruction(LOAD_FAST, 2, 4, 1),
                instruction(COPY, 1, 4, 2),
                instruction(POP_TOP, 0, 4, 1),
                instruction(STORE_FAST, 3, 4, 0),
                instruction(DELETE_FAST, 2, 5, 0),
                instruction(LOAD_FAST, 2, 5, 1),
                instruction(POP_TOP, 0, 5, 0),
                instruction(LOAD_FAST, 2, 6, 1),
                instruction(LOAD_FAST, 3, 6, 2),
                instruction(BUILD_TUPLE, 2, 6, 1),
            ],
        ),
        (
            2,
            2,
            vec![
                instruction(LOAD_FAST, 0, 2, 1),
                instruction(LOAD_FAST, 1, 2, 2),
                instruction(IS_OP, 0, 2, 1),
                instruction(TO_BOOL, 0, 2, 1),
                instruction(UNARY_NOT, 0, 2, 1),
            ],
        ),
        (
            2,
            2,
            vec![
                instruction(LOAD_FAST, 1, 2, 1),
                instruction(LOAD_FAST, 0, 2, 2),
                instruction(STORE_ATTR, 0, 2, 0),
                instruction(LOAD_FAST, 0, 3, 1),
                instruction(LOAD_SMALL_INT, 2, 3, 2),
                instruction(BUILD_LIST, 2, 3, 1),
                instruction(LOAD_FAST, 1, 4, 2),
                instruction(LIST_EXTEND, 1, 4, 1),
            ],
        ),
    ];
    for (local_count, parameter_count, body) in scenarios {
        for (owned, borrowed) in [(false, false), (true, false), (false, true)] {
            let mut items = vec![resume];
            items.extend(body.iter().copied());
            items.push(ret);
            if owned || borrowed {
                for item in &mut items {
                    if let Item::Instruction(instruction) = item
                        && instruction.opcode == LOAD_FAST
                    {
                        instruction.local_policy = LocalPolicy::for_load(owned, owned);
                        if borrowed {
                            instruction.local_policy.mark_force_borrowed_load();
                        }
                    }
                }
            }
            assert!(!Assembler::from_linear_items(items.clone()).has_only_linear_constant_return());
            assert_linear_matches_graph(
                || Assembler::from_linear_items(items.clone()),
                local_count,
                parameter_count,
            );
        }
    }
}

#[test]
fn ordinary_linear_cells_and_closures_match_the_block_graph_pipeline() {
    let instruction = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            if line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(line, line, 0, 0)
            },
            Some(depth),
        ))
    };
    let resume = instruction(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0);
    let ret = instruction(RETURN_VALUE, 0, 5, 0);
    let mut closure_load = Instruction::new(
        LOAD_FAST,
        Operand::Value(1),
        SourceLocation::new(2, 2, 0, 0),
        Some(1),
    );
    closure_load.local_policy.mark_post_analysis_closure_load();
    closure_load.fusion_policy.prevent_with_previous();
    for (local_count, parameter_count, items) in [
        (
            0,
            0,
            vec![
                instruction(MAKE_CELL, 0, -1, 0),
                resume,
                instruction(LOAD_LOCALS, 0, 1, 1),
                instruction(STORE_DEREF, 0, 1, 0),
                instruction(SETUP_ANNOTATIONS, 0, 1, 0),
                instruction(LOAD_CONST, 0, 2, 1),
                instruction(MAKE_FUNCTION, 0, -1, 1),
                instruction(STORE_NAME, 0, 2, 0),
                instruction(LOAD_FAST, 0, -1, 1),
                instruction(STORE_NAME, 1, -1, 0),
                instruction(LOAD_CONST, 1, -1, 1),
                ret,
            ],
        ),
        (
            1,
            1,
            vec![
                instruction(COPY_FREE_VARS, 1, -1, 0),
                resume,
                instruction(LOAD_DEREF, 1, 2, 1),
                instruction(LOAD_FAST, 0, 2, 2),
                instruction(BUILD_TUPLE, 2, 2, 1),
                ret,
            ],
        ),
        (
            1,
            0,
            vec![
                instruction(MAKE_CELL, 1, -1, 0),
                resume,
                Item::Instruction(closure_load),
                instruction(BUILD_TUPLE, 1, 2, 1),
                ret,
            ],
        ),
    ] {
        assert_linear_matches_graph(
            || Assembler::from_linear_items(items.clone()),
            local_count,
            parameter_count,
        );
    }
}

#[test]
fn ordinary_linear_disjoint_cell_initializers_match_the_block_graph_pipeline() {
    let item = |opcode, argument, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::new(2, 2, 0, 20),
            Some(depth),
        ))
    };
    let prefix = |opcode, argument| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::NONE,
            Some(0),
        ))
    };
    let closure = |local, depth| {
        let Item::Instruction(mut instruction) = item(LOAD_FAST, local, depth) else {
            unreachable!();
        };
        instruction.local_policy.mark_post_analysis_closure_load();
        instruction.fusion_policy.prevent_with_previous();
        Item::Instruction(instruction)
    };
    let resume = item(RESUME, ResumeLocation::AT_FUNC_START.argument(), 0);
    let ret = item(RETURN_VALUE, 0, 0);
    for (local_count, cell, other) in [(3, 1, 2), (64, 63, 62)] {
        for setup in [MAKE_CELL, COPY_FREE_VARS] {
            let items = vec![
                prefix(setup, cell),
                resume,
                item(LOAD_FAST, 0, 1),
                item(STORE_DEREF, cell, 0),
                closure(cell, 1),
                item(BUILD_TUPLE, 1, 1),
                item(LOAD_CONST, 0, 2),
                item(MAKE_FUNCTION, 0, 2),
                item(
                    SET_FUNCTION_ATTRIBUTE,
                    FunctionAttribute::CLOSURE.argument(),
                    1,
                ),
                item(STORE_FAST, other, 0),
                item(LOAD_FAST, other, 1),
                ret,
            ];
            assert_linear_matches_graph(
                || Assembler::from_linear_items(items.clone()),
                local_count,
                1,
            );
        }
    }
    for body in [
        vec![
            item(LOAD_FAST, 1, 1),
            item(STORE_DEREF, 0, 0),
            item(LOAD_FAST, 1, 1),
        ],
        vec![closure(0, 1), item(POP_TOP, 0, 0), item(LOAD_FAST, 1, 1)],
        vec![item(LOAD_FAST, 1, 1), item(POP_TOP, 0, 0), closure(0, 1)],
    ] {
        let mut items = vec![resume];
        items.extend(body);
        items.push(ret);
        assert_linear_matches_graph(|| Assembler::from_linear_items(items.clone()), 2, 0);
    }
    for (local_count, cell) in [(2, 0), (64, 63), (65, 64)] {
        for body in [
            vec![
                item(LOAD_CONST, 0, 1),
                item(STORE_DEREF, cell, 0),
                item(LOAD_FAST, cell, 1),
            ],
            vec![
                closure(cell, 1),
                item(POP_TOP, 0, 0),
                item(LOAD_FAST, cell, 1),
            ],
            vec![
                item(LOAD_FAST, cell, 1),
                item(POP_TOP, 0, 0),
                closure(cell, 1),
            ],
        ] {
            let mut items = vec![resume];
            items.extend(body);
            items.push(ret);
            assert!(!Assembler::from_linear_items(items).has_only_linear_instructions(local_count));
        }
    }
}

#[test]
fn ordinary_linear_unprotected_folds_and_class_footer_match_the_block_graph_pipeline() {
    let instruction = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            if line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(line, line, 0, 0)
            },
            Some(depth),
        ))
    };
    for opcode in [LOAD_CONST, LOAD_SMALL_INT] {
        for count in [1, 2, 3, 4, 9, 1023] {
            for class_footer in [false, true] {
                let build = |recorded| {
                    let mut assembler = Assembler::from_linear_items(vec![
                        instruction(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
                        instruction(opcode, 1, 2, 1),
                    ]);
                    assembler.add_last_constant_fold_slots(count);
                    if recorded {
                        assembler.record_last_uniform_fold_owner_slots(count);
                    }
                    for item in [
                        instruction(STORE_NAME, 0, 2, 0),
                        instruction(LOAD_CONST, 0, -1, 1),
                        instruction(RETURN_VALUE, 0, -1, 0),
                    ] {
                        assembler.push_linear_item(item);
                    }
                    if class_footer {
                        assembler.allow_last_no_location_block_inlining();
                    }
                    assembler
                };
                assert!(build(false).has_only_linear_instructions(0));
                assert_linear_matches_graph(|| build(true), 0, 0);
            }
        }
    }
}

#[test]
fn linear_literal_fold_history_matches_the_block_graph_pipeline() {
    let item = |opcode, argument, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::new(2, 2, 0, 0),
            Some(depth),
        ))
    };
    for opcode in [LOAD_CONST, LOAD_SMALL_INT, LIST_EXTEND, SET_UPDATE] {
        for count in [1, 4, 9, 1023] {
            for extra in [0, 4] {
                let mut items = vec![item(RESUME, ResumeLocation::AT_FUNC_START.argument(), 0)];
                if matches!(opcode, LIST_EXTEND | SET_UPDATE) {
                    items.push(item(
                        if opcode == LIST_EXTEND {
                            BUILD_LIST
                        } else {
                            BUILD_SET
                        },
                        0,
                        1,
                    ));
                    items.push(item(LOAD_CONST, 0, 2));
                }
                let Item::Instruction(mut folded) = item(opcode, 1, 1) else {
                    unreachable!()
                };
                folded.local_policy.add_hidden_exception_owner_slots(count);
                folded.add_pre_fold_extra_instructions(extra);
                folded.jump_policy.mark_original_uniform_fold();
                items.push(Item::Instruction(folded));
                items.push(item(RETURN_VALUE, 0, 0));
                let build = || {
                    let mut assembler = Assembler::from_linear_items(items.clone());
                    assembler
                        .exception_owner_provenance
                        .original_uniform_folds
                        .push(folded);
                    assembler
                };
                assert_linear_matches_graph(build, 0, 0);
            }
        }
    }
}

#[test]
fn ordinary_linear_instructions_reject_control_flow_and_unhandled_policies() {
    let instruction = |opcode, argument, depth| {
        Instruction::new(
            opcode,
            Operand::Value(argument),
            SourceLocation::new(1, 1, 0, 0),
            Some(depth),
        )
    };
    let original = || {
        vec![
            Item::Instruction(instruction(
                RESUME,
                ResumeLocation::AT_FUNC_START.argument(),
                0,
            )),
            Item::Instruction(instruction(LOAD_FAST, 0, 1)),
            Item::Instruction(instruction(RETURN_VALUE, 0, 0)),
        ]
    };
    assert!(Assembler::from_linear_items(original()).has_only_linear_instructions(0));

    for possible_nop in [NOP, TO_BOOL, UNARY_NOT] {
        let mut items = original();
        items.insert(2, Item::Instruction(instruction(UNPACK_SEQUENCE, 1, 1)));
        items.insert(3, Item::Instruction(instruction(possible_nop, 0, 1)));
        assert!(!Assembler::from_linear_items(items).has_only_linear_instructions(0));
    }

    let cell_store = vec![
        original()[0],
        Item::Instruction(instruction(LOAD_FAST, 0, 1)),
        Item::Instruction(instruction(STORE_DEREF, 0, 0)),
        Item::Instruction(instruction(LOAD_FAST, 0, 1)),
        original()[2],
    ];
    assert!(!Assembler::from_linear_items(cell_store).has_only_linear_instructions(1));

    let mut policy_prefix = instruction(MAKE_CELL, 0, 0);
    policy_prefix.location = SourceLocation::NONE;
    policy_prefix.no_location_policy.preserve_location();
    let mut prefixed = original();
    prefixed.insert(0, Item::Instruction(policy_prefix));
    assert!(!Assembler::from_linear_items(prefixed).has_only_linear_instructions(0));

    let mut hidden_slots = instruction(LOAD_FAST, 0, 1);
    hidden_slots
        .local_policy
        .add_hidden_exception_owner_slots(1);
    let mut closure_load = instruction(LOAD_FAST, 0, 1);
    closure_load.local_policy.mark_post_analysis_closure_load();
    let mut no_location = instruction(NOP, 0, 0);
    no_location.location = SourceLocation::NONE;
    let mut retained_nop = instruction(NOP, 0, 0);
    retained_nop.jump_policy.preserve_explicit_nop();
    let mut exceptional = instruction(LOAD_FAST, 0, 1);
    exceptional.set_exception_policy(ExceptionPolicy::NonePresent);
    let target = super::Label::from_u32(0);
    for replacement in [
        Item::Instruction(instruction(SWAP, 2, 1)),
        Item::Instruction(instruction(RETURN_VALUE, 0, 0)),
        Item::Instruction(instruction(RETURN_GENERATOR, 0, 0)),
        Item::Instruction(hidden_slots),
        Item::Instruction(closure_load),
        Item::Instruction(no_location),
        Item::Instruction(retained_nop),
        Item::Instruction(exceptional),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            Some(0),
        )),
        Item::Label(target),
    ] {
        let mut items = original();
        items[1] = replacement;
        assert!(!Assembler::from_linear_items(items).has_only_linear_instructions(0));
    }

    let mut allocated = Assembler::from_linear_items(original());
    allocated.label();
    assert!(!allocated.has_only_linear_instructions(0));
    let mut provenance = Assembler::from_linear_items(original());
    provenance.exception_owner_provenance.omitted_nop_tail =
        Some((1, SourceLocation::new(2, 2, 0, 0), 1));
    assert!(!provenance.has_only_linear_instructions(0));
    let mut next_location = Assembler::from_linear_items(original());
    next_location.has_next_location = true;
    assert!(!next_location.has_only_linear_instructions(0));
}

#[test]
fn maybe_null_comprehension_restore_checks_only_the_first_later_load() {
    let mut restore = Instruction::new(STORE_FAST, Operand::Value(0), SourceLocation::NONE, None);
    restore.local_policy.mark_maybe_null_comprehension_restore();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(restore),
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(LOAD_FAST, 0, 1),
    ]);

    assembler.add_checks_for_uninitialized_loads(1, 1, &rustc_hash::FxHashSet::default());

    assert_eq!(
        graph_opcodes(&assembler),
        [STORE_FAST, LOAD_FAST_CHECK, LOAD_FAST]
    );
}

#[test]
fn local_rewrites_report_only_instruction_changes() {
    let mut stores = optimization_pipeline(vec![
        pass_instruction(STORE_FAST, 0, 1),
        pass_instruction(STORE_FAST, 1, 1),
        pass_instruction(STORE_FAST, 1, 2),
        pass_instruction(STORE_FAST, 1, 2),
    ]);
    assert!(stores.optimize_redundant_store_fast());
    let stored = stores.graph.copy_items();
    assert_eq!(
        graph_opcodes(&stores),
        [STORE_FAST, STORE_FAST, POP_TOP, STORE_FAST]
    );
    assert!(!stores.optimize_redundant_store_fast());
    assert_eq!(stores.graph.copy_items(), stored);

    let mut loads = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST_CHECK, 0, 1),
        pass_instruction(LOAD_FAST_CHECK, 1, 1),
        pass_instruction(LOAD_FAST_CHECK, 0, 1),
        pass_instruction(DELETE_FAST, 0, 1),
        pass_instruction(LOAD_FAST_CHECK, 0, 1),
    ]);
    assert!(loads.remove_redundant_checked_loads());
    let checked = loads.graph.copy_items();
    assert_eq!(
        graph_opcodes(&loads),
        [
            LOAD_FAST_CHECK,
            LOAD_FAST_CHECK,
            LOAD_FAST,
            DELETE_FAST,
            LOAD_FAST_CHECK
        ]
    );
    assert!(!loads.remove_redundant_checked_loads());
    assert_eq!(loads.graph.copy_items(), checked);
}

#[test]
fn initialization_passes_record_new_checked_loads_for_later_cleanup() {
    for (multiple_blocks, assertion_pass) in [(false, false), (true, false), (true, true)] {
        let mut items = vec![pass_instruction(LOAD_FAST, 0, 1)];
        if multiple_blocks {
            let target = super::Label::from_u32(0);
            items.extend([
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(target),
                    SourceLocation::NONE,
                    None,
                )),
                Item::Label(target),
            ]);
        }
        items.push(pass_instruction(LOAD_FAST, 0, 2));
        let mut assembler = optimization_pipeline(items);
        assert!(assembler.graph.features().has_fast_load());
        assert!(!assembler.graph.features().has_checked_fast_load());
        assert!(!assembler.remove_redundant_checked_loads());

        if assertion_pass {
            assembler.add_checks_for_assertion_side_exits(1, 0);
        } else {
            let donors = assembler.capture_local_nop_donors();
            assembler.add_checks_for_uninitialized_loads(1, 0, &donors);
        }

        assert!(assembler.graph.features().has_checked_fast_load());
        assert_eq!(graph_opcodes(&assembler)[0], LOAD_FAST_CHECK);
    }
}

#[test]
fn swap_runs_report_only_changed_slots() {
    for (depths, changed, opcodes) in [([2, 3], false, [SWAP, SWAP]), ([2, 2], true, [NOP, NOP])] {
        let mut assembler = optimization_pipeline(
            depths
                .into_iter()
                .map(|depth| pass_instruction(SWAP, depth, 1))
                .collect(),
        );
        let before = assembler.graph.copy_items();
        assert_eq!(assembler.optimize_swap_runs().2, changed);
        assert_eq!(graph_opcodes(&assembler), opcodes);
        if !changed {
            assert_eq!(assembler.graph.copy_items(), before);
        }
        let after = assembler.graph.copy_items();
        assert!(!assembler.optimize_swap_runs().2);
        assert_eq!(assembler.graph.copy_items(), after);
    }

    let swaps = |depths: [u32; 3]| {
        depths
            .into_iter()
            .map(|depth| pass_instruction(SWAP, depth, 1))
            .collect::<Vec<_>>()
    };
    let mut assembler = optimization_pipeline(swaps([3, 2, 3]));
    assert!(assembler.optimize_swap_runs().2);
    let expected = swaps([2, 3, 2]);
    assert_eq!(assembler.graph.copy_items(), expected);
    assert!(!assembler.optimize_swap_runs().2);
    assert_eq!(assembler.graph.copy_items(), expected);
}

#[test]
fn redundant_swap_removal_preserves_forward_scan_semantics() {
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(SWAP, 2, 1),
        Item::Label(super::Label::from_u32(0)),
        pass_instruction(SWAP, 2, 1),
        pass_instruction(SWAP, 2, 1),
        pass_instruction(POP_TOP, 0, 1),
        Item::Label(super::Label::from_u32(1)),
        pass_instruction(POP_TOP, 0, 1),
    ]);

    assert!(assembler.remove_redundant_swaps_before_pops());

    assert_eq!(graph_opcodes(&assembler), [SWAP, SWAP, POP_TOP, POP_TOP]);
}

#[test]
fn redundant_swap_removal_preserves_distinct_pop_lines() {
    for (first_pop_line, second_pop_line, retain_rotation) in
        [(5, 5, false), (5, 6, true), (-1, 6, false)]
    {
        let mut items = vec![pass_instruction(SWAP, 3, 4), pass_instruction(SWAP, 2, 4)];
        let pops_and_store = [
            pass_instruction(POP_TOP, 0, first_pop_line),
            pass_instruction(POP_TOP, 0, second_pop_line),
            pass_instruction(STORE_FAST, 1, 3),
        ];
        items.extend(pops_and_store.iter().copied());
        let mut assembler = optimization_pipeline(items);

        assert_eq!(
            assembler.remove_redundant_swaps_before_pops(),
            !retain_rotation
        );
        assert!(!assembler.remove_redundant_swaps_before_pops());

        let mut expected = vec![pass_instruction(SWAP, 3, 4)];
        if retain_rotation {
            expected.push(pass_instruction(SWAP, 2, 4));
        }
        if retain_rotation {
            expected.extend(pops_and_store);
        } else {
            expected.extend([
                pass_instruction(POP_TOP, 0, second_pop_line),
                pass_instruction(POP_TOP, 0, first_pop_line),
                pops_and_store[2],
            ]);
        }
        assert_eq!(
            assembler.graph.iter_items().copied().collect::<Vec<_>>(),
            expected,
            "pop lines {first_pop_line} and {second_pop_line}"
        );
    }
}

#[test]
fn redundant_swap_removal_permutates_pop_spans_without_moving_slot_metadata() {
    let first_location = SourceLocation::new(5, 5, 18, 68);
    let last_location = SourceLocation::new(5, 5, 17, 73);
    let mut first = Instruction::new(POP_TOP, Operand::Value(0), first_location, Some(2));
    let mut last = Instruction::new(POP_TOP, Operand::Value(0), last_location, Some(1));
    first.set_exception_policy(ExceptionPolicy::NonePresent);
    last.set_exception_policy(ExceptionPolicy::NoneAbsent);
    let outer_swap = pass_instruction(SWAP, 3, 5);
    let mut assembler = optimization_pipeline(vec![
        outer_swap,
        pass_instruction(SWAP, 2, 5),
        Item::Instruction(first),
        Item::Instruction(last),
    ]);

    assert!(assembler.remove_redundant_swaps_before_pops());
    first.location = last_location;
    last.location = first_location;
    assert_eq!(
        assembler.graph.copy_items(),
        [
            outer_swap,
            Item::Instruction(first),
            Item::Instruction(last)
        ]
    );
    assert!(!assembler.remove_redundant_swaps_before_pops());
}

#[test]
fn redundant_nop_removal_preserves_forward_scan_semantics() {
    let mut converted = match pass_instruction(NOP, 0, 1) {
        Item::Instruction(instruction) => instruction,
        Item::Label(_) => unreachable!(),
    };
    converted.fusion_policy.mark_converted_pop_block();
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(NOP, 0, 1),
        Item::Label(super::Label::from_u32(0)),
        pass_instruction(NOP, 0, -1),
        pass_instruction(LOAD_CONST, 0, 1),
        pass_instruction(NOP, 0, -1),
        Item::Instruction(converted),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);

    assembler.remove_redundant_nops();

    assert_eq!(graph_opcodes(&assembler), [NOP, LOAD_CONST, RETURN_VALUE]);
}

#[test]
fn cold_block_terminal_restore_accounts_for_the_drained_range() {
    let source = r"async def f(iterator):
    async for item in iterator:
        if condition:
            await call()
            break
    assert result
";

    crate::compile(source, "cold_terminal_restore.py").unwrap();
}

#[test]
fn consecutive_pop_slot_width_respects_original_array_boundaries() {
    use super::{ExceptionRegion, ExceptionRegionPriority, Label};

    let converted = || {
        let mut instruction = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
        instruction.fusion_policy.mark_converted_pop_block();
        Item::Instruction(instruction)
    };
    let setup = Label::from_u32(0);
    let end = Label::from_u32(1);
    let interior = Label::from_u32(2);
    let handler = Label::from_u32(3);
    let location = SourceLocation::new(1, 1, 0, 0);
    for (case, expected_width) in [
        ("contiguous", Some(2)),
        ("bookkeeping_alias", Some(2)),
        ("end_inside_run", Some(2)),
        ("jump_into_first", None),
        ("jump_into_interior", None),
        ("block_boundary_first", None),
        ("block_boundary_interior", None),
        ("removed_source_nop", None),
        ("hidden_pop_record", None),
        ("setup_nop_inside_run", None),
        ("count_does_not_fit", None),
    ] {
        let mut items = vec![
            pass_instruction(NOP, 0, 1),
            Item::Label(setup),
            pass_instruction(LOAD_CONST, 0, 2),
        ];
        if case.starts_with("jump_into_") {
            items.push(Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(if case == "jump_into_first" {
                    end
                } else {
                    interior
                }),
                location,
                None,
            )));
        }
        if case == "removed_source_nop" {
            items.push(pass_instruction(NOP, 0, -1));
        }
        if case != "end_inside_run" {
            items.push(Item::Label(end));
        }
        let mut first = converted();
        if case == "hidden_pop_record" {
            let Item::Instruction(instruction) = &mut first else {
                unreachable!()
            };
            instruction.local_policy.add_hidden_exception_owner_slots(1);
        }
        items.push(first);
        if case == "end_inside_run" {
            items.push(Item::Label(end));
        }
        items.push(Item::Label(interior));
        if case == "bookkeeping_alias" {
            items.push(Item::Label(Label::from_u32(4)));
            items.push(Item::Label(Label::from_u32(5)));
        }
        let count = if case == "count_does_not_fit" {
            usize::from(LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS)
        } else {
            1
        };
        items.extend((0..count).map(|_| converted()));
        items.extend([
            pass_instruction(LOAD_FAST, 0, 3),
            Item::Label(handler),
            pass_instruction(RETURN_VALUE, 0, 4),
        ]);
        let mut assembler = optimization_pipeline(items);
        assembler.exception_regions.push(ExceptionRegion {
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(setup, end),
            start: setup,
            end,
            target: handler,
            depth: 0,
            preserve_lasti: false,
        });
        assembler
            .exception_owner_provenance
            .setups
            .push((setup, Some(location)));
        if case == "setup_nop_inside_run" {
            assembler
                .exception_owner_provenance
                .setups
                .push((interior, Some(SourceLocation::NONE)));
        }
        if case.starts_with("block_boundary_") {
            assembler
                .exception_owner_provenance
                .block_boundaries
                .insert(if case == "block_boundary_first" {
                    end
                } else {
                    interior
                });
        }
        assembler.remove_redundant_nops();
        assert_eq!(
            assembler
                .exception_owner_provenance
                .removed_pop_block_widths
                .get(&end)
                .copied(),
            expected_width,
            "{case}",
        );
    }
}

#[test]
fn duplicated_exit_blocks_retain_transitively_targeted_blocks() {
    let source = r"def f():
    try:
        result = first()
    except Error as outer:
        try:
            result = second()
        except Error as inner:
            pass
    return result
";

    crate::compile(source, "nested_exit_copy.py").unwrap();
}

#[test]
fn duplicated_finally_exits_keep_internal_jump_labels_local() {
    for prefix in ["", "async "] {
        for condition in ["flag", "not flag"] {
            let source = format!(
                "{prefix}def function(flag):\n    try:\n        pass\n    except Exception as error:\n        return None\n    finally:\n        try:\n            if {condition}:\n                cleanup()\n        except Exception:\n            pass\n",
            );
            // Shared exits can already contain an inlined return when conditional
            // predecessors duplicate them. Their internal jumps must use copied labels.
            crate::compile(&source, "copied_finally_internal_jump.py").unwrap();
        }
    }
}

#[test]
fn empty_exception_regions_follow_their_original_entry() {
    for reachable_entry in [false, true] {
        let start = super::Label::from_u32(0);
        let end = super::Label::from_u32(1);
        let handler = super::Label::from_u32(2);
        let mut items = Vec::new();
        if !reachable_entry {
            items.extend([
                pass_instruction(LOAD_CONST, 0, 1),
                pass_instruction(RETURN_VALUE, 0, 1),
            ]);
        }
        items.extend([
            Item::Label(start),
            Item::Label(end),
            pass_instruction(LOAD_CONST, 0, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
            Item::Label(handler),
            pass_instruction(RERAISE, 0, -1),
        ]);
        let mut assembler = optimization_pipeline(items);
        assembler.exception_regions.push(super::ExceptionRegion {
            priority: super::ExceptionRegionPriority::OriginalRange(start, end),
            is_scope_fallback: false,
            start,
            end,
            target: handler,
            depth: 0,
            preserve_lasti: false,
        });
        let handler = assembler.graph.label_block(handler).unwrap();
        let reachable = assembler
            .graph
            .reachable_blocks(&assembler.exception_regions);
        assert_eq!(reachable[handler.index()], reachable_entry);
    }
}

#[test]
fn constant_return_pop_requires_authored_default_records() {
    for version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        for mutation in [
            "intact",
            "custom owner",
            "hidden slot",
            "synthetic marker",
            "converted pop",
            "different span",
            "double marker",
            "wrong depth",
            "non-source entry",
        ] {
            let mut assembler = Assembler {
                python_version: version,
                ..Assembler::default()
            };
            let entry = assembler.label();
            let donor = SourceLocation::new(2, 2, 8, 13);
            let location = SourceLocation::new(5, 5, 15, 19);
            let mut pop = Instruction::value_with_depth(NOP, 0, SourceLocation::NONE, 0);
            pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            pop.fusion_policy.mark_converted_pop_block();
            let mut tail = vec![
                Item::Instruction(Instruction::value_with_depth(NOP, 0, location, 0)),
                Item::Instruction(Instruction::value_with_depth(LOAD_CONST, 0, location, 1)),
                Item::Instruction(Instruction::value_with_depth(RETURN_VALUE, 0, location, 0)),
            ];
            let Item::Instruction(nop) = &mut tail[0] else {
                unreachable!();
            };
            match mutation {
                "custom owner" => nop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "hidden slot" => nop.local_policy.add_hidden_exception_owner_slots(1),
                "synthetic marker" => {
                    *nop = Instruction::synthetic(NOP, Operand::Value(0), location, Some(0));
                }
                "converted pop" => nop.fusion_policy.mark_converted_pop_block(),
                "different span" => nop.location = SourceLocation::new(5, 5, 8, 19),
                "wrong depth" => *nop = Instruction::value_with_depth(NOP, 0, location, 1),
                _ => {}
            }
            if mutation == "double marker" {
                tail.insert(1, tail[0]);
            }
            assembler.push_linear_item(Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(entry),
                donor,
                Some(0),
            )));
            assembler.push_linear_item(Item::Instruction(Instruction::value_with_depth(
                STORE_FAST, 0, donor, 0,
            )));
            assembler.mark(entry);
            if mutation != "non-source entry" {
                assembler.record_exception_owner_block_boundary(entry);
            }
            assembler.push_linear_item(Item::Instruction(pop));
            for item in tail {
                assembler.push_linear_item(item);
            }
            assembler.record_copied_return_fallthrough_pop(2);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .original_fallthrough_pop_blocks
                    .contains_key(&entry),
                mutation == "intact" && version >= crate::PythonVersion::Py314,
                "{version:?}, {mutation}",
            );
        }
    }
}

#[test]
fn copied_return_pop_origins_retire_only_with_erased_unreachable_arrays() {
    for reachable in [false, true] {
        let mut assembler = Assembler::default();
        let origin = assembler.label();
        let exit = assembler.label();
        assembler.emit_forward_with_depth(
            JUMP_FORWARD,
            if reachable { origin } else { exit },
            0,
            ExceptionPolicy::default(),
        );
        assembler.record_exception_owner_block_boundary(origin);
        assembler.mark(origin);
        let first_pop_item = assembler.item_count();
        for _ in 0..2 {
            assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
            assembler.mark_last_as_converted_pop_block();
        }
        assembler.set_location(SourceLocation::new(2, 2, 0, 4));
        assembler.emit(LOAD_CONST, 1);
        assembler.record_copied_return_pop_prefix(first_pop_item);
        assembler.emit(POP_TOP, 0);
        assembler.emit_forward_with_depth(JUMP_FORWARD, exit, 0, ExceptionPolicy::default());
        assembler.mark(exit);
        assembler.emit(LOAD_CONST, 0);
        assembler.emit(RETURN_VALUE, 0);
        assert_eq!(
            assembler
                .exception_owner_provenance
                .copied_return_pop_prefixes
                .len(),
            1
        );
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.remove_unreachable_instructions();
        assert_eq!(
            pipeline
                .exception_owner_provenance
                .copied_return_pop_prefixes
                .len(),
            usize::from(reachable),
            "reachable={reachable}",
        );
        if reachable {
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .copied_return_pop_prefixes[0]
                    .entry,
                origin,
            );
        }
    }
}

#[test]
fn no_location_block_inlining_reaches_a_fixed_point() {
    let restart = super::Label::from_u32(0);
    let source = super::Label::from_u32(1);
    let first_cleanup = super::Label::from_u32(2);
    let second_cleanup = super::Label::from_u32(3);
    let mut first_cleanup_jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(second_cleanup),
        SourceLocation::NONE,
        None,
    );
    first_cleanup_jump.jump_policy.prevent_threading_target();
    first_cleanup_jump.no_location_policy.allow_block_inlining();
    let mut second_cleanup_jump = Instruction::new(
        JUMP_BACKWARD_NO_INTERRUPT,
        Operand::Backward(restart),
        SourceLocation::NONE,
        None,
    );
    second_cleanup_jump.jump_policy.prevent_threading_target();
    second_cleanup_jump
        .no_location_policy
        .allow_block_inlining();
    let mut assembler = optimization_pipeline(vec![
        Item::Label(restart),
        pass_instruction(NOP, 0, 0),
        Item::Label(source),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(first_cleanup),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(first_cleanup),
        pass_instruction(POP_EXCEPT, 0, -1),
        Item::Instruction(first_cleanup_jump),
        Item::Label(second_cleanup),
        pass_instruction(POP_TOP, 0, -1),
        Item::Instruction(second_cleanup_jump),
    ]);

    // Track only origin lifecycle here; existing assertions below verify the copied CFG.
    // Source is replaced by inlined records, both cleanup originals disappear, and restart survives.
    let records = [
        Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None),
        Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None),
        Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None),
    ];
    assembler
        .exception_owner_provenance
        .copied_return_pop_prefixes = [restart, source, first_cleanup, second_cleanup]
        .into_iter()
        .map(|entry| super::exception_owners::CopiedReturnPopPrefix { entry, records })
        .collect();

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());
    assembler.propagate_locations_within_blocks();

    assert_eq!(
        assembler
            .exception_owner_provenance
            .copied_return_pop_prefixes
            .iter()
            .map(|origin| origin.entry)
            .collect::<Vec<_>>(),
        [restart],
    );

    assert_eq!(
        graph_opcodes(&assembler),
        [
            NOP,
            LOAD_CONST,
            POP_EXCEPT,
            POP_TOP,
            JUMP_BACKWARD_NO_INTERRUPT
        ]
    );
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location.line),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [0, 1, 1, 1, 1]
    );
}

#[test]
fn shared_inlined_cleanup_locations_distinguish_exit_and_continuation() {
    for terminal in [JUMP_BACKWARD_NO_INTERRUPT, RERAISE, JUMP_BACKWARD] {
        let restart = super::Label::from_u32(0);
        let condition = super::Label::from_u32(1);
        let body = super::Label::from_u32(2);
        let first_cleanup = super::Label::from_u32(3);
        let second_cleanup = super::Label::from_u32(4);
        let mut first_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(second_cleanup),
            SourceLocation::NONE,
            None,
        );
        first_jump.jump_policy.prevent_threading_target();
        first_jump.no_location_policy.allow_block_inlining();
        let mut last = Instruction::new(
            terminal,
            if terminal == RERAISE {
                Operand::Value(0)
            } else {
                Operand::Backward(restart)
            },
            SourceLocation::NONE,
            None,
        );
        last.jump_policy.prevent_threading_target();
        last.no_location_policy.allow_block_inlining();
        let mut assembler = optimization_pipeline(vec![
            Item::Label(restart),
            pass_instruction(NOP, 0, 0),
            Item::Label(condition),
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(first_cleanup),
                SourceLocation::new(1, 1, 0, 0),
                None,
            )),
            Item::Label(body),
            pass_instruction(LOAD_CONST, 0, 2),
            Item::Label(first_cleanup),
            pass_instruction(POP_EXCEPT, 0, -1),
            Item::Instruction(first_jump),
            Item::Label(second_cleanup),
            pass_instruction(POP_EXCEPT, 0, -1),
            Item::Instruction(last),
        ]);

        let original_location_absence = assembler.capture_original_location_absence();
        assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());
        assembler.propagate_locations_within_blocks();

        let cleanups = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) if instruction.opcode == POP_EXCEPT => {
                    Some(instruction.location.line)
                }
                Item::Instruction(_) | Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert!(!cleanups.is_empty());
        assert_eq!(cleanups.len() % 2, 0);
        let mut outer_locations = cleanups
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| pair[1])
            .collect::<Vec<_>>();
        outer_locations.sort_unstable();
        // Native source resolution duplicates exits and eval checks. A normal
        // continuation keeps its original shared, unlocated cleanup boundary.
        if terminal == JUMP_BACKWARD_NO_INTERRUPT {
            assert!(outer_locations.iter().all(|line| *line < 0));
        } else {
            assert_eq!(outer_locations, [1, 2]);
        }
    }
}

#[test]
fn no_location_block_inlining_crosses_exception_splits() {
    let restart = super::Label::from_u32(0);
    let source = super::Label::from_u32(1);
    let cleanup = super::Label::from_u32(2);
    let outer_unwind = super::Label::from_u32(3);
    let handler = super::Label::from_u32(4);
    let mut cleanup_jump = Instruction::new(
        JUMP_BACKWARD_NO_INTERRUPT,
        Operand::Backward(restart),
        SourceLocation::NONE,
        None,
    );
    cleanup_jump.jump_policy.prevent_threading_target();
    cleanup_jump.no_location_policy.allow_block_inlining();
    let mut assembler = optimization_pipeline(vec![
        Item::Label(restart),
        pass_instruction(NOP, 0, 0),
        Item::Label(source),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(cleanup),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(cleanup),
        pass_instruction(POP_EXCEPT, 0, -1),
        Item::Label(outer_unwind),
        pass_instruction(POP_EXCEPT, 0, -1),
        Item::Instruction(cleanup_jump),
        Item::Label(handler),
        pass_instruction(RERAISE, 0, -1),
    ]);
    assembler.exception_regions.push(super::ExceptionRegion {
        priority: super::ExceptionRegionPriority::OriginalRange(source, outer_unwind),
        is_scope_fallback: false,
        start: source,
        end: outer_unwind,
        target: handler,
        depth: 0,
        preserve_lasti: true,
    });

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());
    assembler.propagate_locations_within_blocks();

    assert_eq!(
        graph_opcodes(&assembler),
        [
            NOP,
            LOAD_CONST,
            POP_EXCEPT,
            POP_EXCEPT,
            JUMP_BACKWARD_NO_INTERRUPT,
            POP_EXCEPT,
            JUMP_BACKWARD_NO_INTERRUPT,
            RERAISE,
        ]
    );
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location.line),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [0, 1, 1, 1, 1, -1, -1, -1]
    );
    assert_eq!(assembler.exception_regions.len(), 2);
}

#[test]
fn unconditional_cleanup_inlining_keeps_the_conditional_target_shared() {
    let restart = super::Label::from_u32(0);
    let conditional = super::Label::from_u32(1);
    let normal = super::Label::from_u32(2);
    let handler = super::Label::from_u32(3);
    let cleanup = super::Label::from_u32(4);
    let mut backedge = Instruction::new(
        JUMP_BACKWARD,
        Operand::Backward(restart),
        SourceLocation::NONE,
        None,
    );
    backedge.jump_policy.prevent_threading_target();
    backedge.allow_unconditional_block_inlining();
    let mut assembler = optimization_pipeline(vec![
        Item::Label(restart),
        pass_instruction(NOP, 0, 1),
        Item::Label(conditional),
        pass_instruction(LOAD_CONST, 0, 2),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(cleanup),
            SourceLocation::new(2, 2, 0, 1),
            None,
        )),
        Item::Label(normal),
        pass_instruction(LOAD_CONST, 0, 3),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(cleanup),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(handler),
        pass_instruction(POP_EXCEPT, 0, 4),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(cleanup),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(cleanup),
        Item::Instruction(backedge),
    ]);

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());

    let branch = assembler.graph.label_block(conditional).unwrap();
    assert_eq!(
        assembler.graph[branch]
            .last_instruction()
            .unwrap()
            .operand(),
        Operand::Forward(cleanup)
    );
    for (start, end) in [(normal, handler), (handler, cleanup)] {
        let mut within_source = false;
        let mut found_backedge = false;
        for item in assembler.graph.iter_items() {
            match item {
                Item::Label(label) if *label == start => within_source = true,
                Item::Label(label) if *label == end => break,
                Item::Instruction(instruction) if within_source => {
                    found_backedge |= instruction.opcode == JUMP_BACKWARD
                        && instruction.operand() == Operand::Backward(restart);
                }
                _ => {}
            }
        }
        assert!(found_backedge);
    }
}

#[test]
fn direct_target_return_copy_crosses_an_exception_boundary() {
    let return_start = super::Label::from_u32(0);
    let return_split = super::Label::from_u32(1);
    let handler = super::Label::from_u32(2);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(return_start),
        SourceLocation::new(1, 1, 0, 0),
        None,
    );
    jump.jump_policy.retain_nop_for_direct_target();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(jump),
        Item::Label(return_start),
        pass_instruction(LOAD_CONST, 0, 2),
        Item::Label(return_split),
        pass_instruction(RETURN_VALUE, 0, 2),
        Item::Label(handler),
        pass_instruction(RERAISE, 0, -1),
    ]);
    assembler.exception_regions.push(super::ExceptionRegion {
        priority: super::ExceptionRegionPriority::OriginalRange(return_start, return_split),
        is_scope_fallback: false,
        start: return_start,
        end: return_split,
        target: handler,
        depth: 0,
        preserve_lasti: false,
    });

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());

    assert_eq!(
        graph_opcodes(&assembler),
        [NOP, LOAD_CONST, RETURN_VALUE, RETURN_VALUE, RERAISE]
    );
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location.line),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [1, 2, 2, 2, -1]
    );
}

#[test]
fn retained_setup_nop_identity_survives_consecutive_labels_without_aliasing_removed_setups() {
    let location = SourceLocation::new(2, 9, 4, 16);
    let start = super::Label::from_u32(0);
    let end = super::Label::from_u32(1);
    let intermediate = super::Label::from_u32(2);
    let setup = super::Label::from_u32(3);
    let nop = Item::Instruction(Instruction::new(NOP, Operand::Value(0), location, None));
    for retained in [false, true] {
        // Both layouts have the same NOP location followed by consecutive labels.
        // Only the first contains that instruction inside the authored SETUP range.
        let items = if retained {
            vec![
                Item::Label(start),
                nop,
                Item::Label(end),
                Item::Label(intermediate),
                Item::Label(setup),
            ]
        } else {
            vec![
                nop,
                Item::Label(start),
                Item::Label(end),
                Item::Label(intermediate),
                Item::Label(setup),
            ]
        };
        let mut label_positions = rustc_hash::FxHashMap::default();
        let mut position = 0;
        for item in &items {
            match item {
                Item::Instruction(_) => position += 1,
                Item::Label(label) => {
                    label_positions.insert(*label, position);
                }
            }
        }
        let assembler = super::LayoutAssembler {
            python_version: crate::PythonVersion::Py314,
            legacy_uniform_jump_seed: false,
            items,
            exception_regions: Vec::new(),
            exception_owner_provenance: Box::new(
                super::exception_owners::ExceptionOwnerProvenance {
                    setups: vec![(setup, Some(location))],
                    setup_nop_ranges: vec![(start, end)],
                    setup_nop_boundaries: [start, end].into_iter().collect(),
                    ..Default::default()
                },
            ),
            preserved_block_boundaries: rustc_hash::FxHashSet::default(),
            removed_max_depth: None,
            next_label: 4,
            has_next_location: false,
        };
        let positions = assembler.retained_setup_nops(&label_positions);
        assert_eq!(positions.contains(&0), retained);
        assert_eq!(positions.len(), usize::from(retained));
    }
}

#[test]
fn emitted_region_requires_a_reachable_final_fallthrough_edge() {
    // Reaching the trailing join through a jump does not make the return fall through.
    let mut assembler = Assembler::default();
    let start = assembler.emitted_region_start();
    let join = assembler.label();
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(join));
    assembler.emit(RETURN_VALUE, 0);
    assembler.mark(join);
    assert!(!assembler.emitted_region_has_final_fallthrough(start));

    // An instruction emitted after a return is not a normal predecessor either.
    let mut assembler = Assembler::default();
    let start = assembler.emitted_region_start();
    assembler.emit(RETURN_VALUE, 0);
    assembler.emit(STORE_FAST, 0);
    assert!(!assembler.emitted_region_has_final_fallthrough(start));

    // Ordinary flow continues through label-only blocks, unlike borrow traversal.
    let mut assembler = Assembler::default();
    let start = assembler.emitted_region_start();
    let join = assembler.label();
    let alias = assembler.label();
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(join));
    assembler.emit(RETURN_VALUE, 0);
    assembler.mark(join);
    assembler.mark(alias);
    assembler.emit(STORE_FAST, 0);
    assert!(assembler.emitted_region_has_final_fallthrough(start));
}

#[test]
fn emitted_region_follows_runtime_branches_but_not_monitoring_targets() {
    for opcode in [POP_JUMP_IF_FALSE, FOR_ITER, SEND, END_ASYNC_FOR] {
        let mut assembler = Assembler::default();
        let start = assembler.emitted_region_start();
        let join = assembler.label();
        assembler.emit_operand(opcode, Operand::Forward(join));
        assembler.emit(RETURN_VALUE, 0);
        assembler.mark(join);
        assembler.emit(STORE_FAST, 0);
        assert_eq!(
            assembler.emitted_region_has_final_fallthrough(start),
            opcode != END_ASYNC_FOR
        );
    }
}

#[test]
fn emitted_region_boundaries_survive_label_insertion_but_not_instruction_removal() {
    let mut assembler = Assembler::default();
    assembler.emit(RETURN_VALUE, 0);
    let start = assembler.emitted_region_start();
    assembler.emit(STORE_FAST, 0);
    let label = assembler.label();
    assembler.mark_before_trailing_instructions(label, 2);
    assert!(assembler.emitted_region_has_final_fallthrough(start));

    assembler.emit(NOP, 0);
    assert!(assembler.take_trailing_nop_location().is_some());
    assert!(!assembler.emitted_region_has_final_fallthrough(start));
    assert!(!Assembler::default().emitted_region_has_final_fallthrough(start));
}

#[test]
fn empty_emitted_regions_fall_through_without_reusing_a_previous_terminator() {
    let mut assembler = Assembler::default();
    assembler.emit(RETURN_VALUE, 0);
    let start = assembler.emitted_region_start();
    assert!(assembler.emitted_region_has_final_fallthrough(start));

    // Labels and omitted no-op provenance do not introduce a terminal edge.
    let label = assembler.label();
    assembler.mark(label);
    assembler.record_omitted_nop_targets(assembler.location);
    assert!(assembler.emitted_region_has_final_fallthrough(start));

    assembler.emit(RETURN_VALUE, 0);
    assert!(!assembler.emitted_region_has_final_fallthrough(start));
}

#[test]
fn unpack_rotation_nop_removal_does_not_expose_capture_store_cascades() {
    let cases = [
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 3, 1),
                pass_instruction(NOP, 0, 1),
                pass_instruction(SWAP, 3, 2),
            ],
            vec![UNPACK_SEQUENCE, SWAP],
        ),
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 3, 1),
                pass_instruction(NOP, 0, 2),
                pass_instruction(SWAP, 3, 3),
            ],
            vec![UNPACK_SEQUENCE, NOP, SWAP],
        ),
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 1, 1),
                pass_instruction(NOP, 0, 1),
                pass_instruction(NOP, 0, 2),
                pass_instruction(STORE_FAST_STORE_FAST, 1, 3),
            ],
            vec![UNPACK_SEQUENCE, NOP, STORE_FAST_STORE_FAST],
        ),
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 1, 1),
                pass_instruction(NOP, 0, 2),
                pass_instruction(STORE_FAST, 0, 3),
            ],
            vec![UNPACK_SEQUENCE, NOP, STORE_FAST],
        ),
    ];
    for (items, expected) in cases {
        let mut assembler = optimization_pipeline(items);
        assembler.remove_redundant_nops();
        assert_eq!(graph_opcodes(&assembler), expected);
    }
}

#[test]
fn canceled_unpack_rotations_preserve_distinct_capture_nop_lines() {
    let cases = [
        (
            vec![
                (UNPACK_SEQUENCE, 5),
                (NOP, 5),
                (NOP, 5),
                (NOP, 6),
                (NOP, 6),
                (STORE_FAST, 3),
            ],
            vec![(UNPACK_SEQUENCE, 5), (NOP, 6), (STORE_FAST, 3)],
        ),
        (
            vec![(UNPACK_SEQUENCE, 5), (NOP, 5), (NOP, 6), (STORE_FAST, 6)],
            vec![(UNPACK_SEQUENCE, 5), (STORE_FAST, 6)],
        ),
        (
            vec![(UNPACK_SEQUENCE, 5), (NOP, 6), (NOP, 7), (STORE_FAST, 3)],
            vec![(UNPACK_SEQUENCE, 5), (NOP, 6), (NOP, 7), (STORE_FAST, 3)],
        ),
        (
            vec![
                (UNPACK_SEQUENCE, 5),
                (NOP, 5),
                (NOP, 6),
                (UNPACK_SEQUENCE, 7),
                (NOP, 8),
                (STORE_FAST, 9),
            ],
            vec![
                (UNPACK_SEQUENCE, 5),
                (NOP, 6),
                (UNPACK_SEQUENCE, 7),
                (NOP, 8),
                (STORE_FAST, 9),
            ],
        ),
    ];
    for (input, expected) in cases {
        let items = input
            .into_iter()
            .map(|(opcode, line)| pass_instruction(opcode, 0, line))
            .collect();
        let mut assembler = optimization_pipeline(items);
        assembler.remove_redundant_nops();
        let actual = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => {
                    Some((instruction.opcode, instruction.location.line))
                }
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}

#[test]
fn original_location_absence_needs_an_unlocated_label_with_no_donor() {
    let [entry, left, right, join, alias] =
        std::array::from_fn(|index| super::Label::from_u32(u32::try_from(index).unwrap()));
    let jump = |opcode, target, line| {
        Item::Instruction(Instruction::new(
            opcode,
            target,
            SourceLocation::new(line, line, 0, 0),
            None,
        ))
    };

    let pipeline = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        jump(POP_JUMP_IF_TRUE, Operand::Forward(join), 1),
        pass_instruction(NOP, 0, -1),
        Item::Label(join),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);
    assert!(pipeline.capture_original_location_absence().is_empty());

    let pipeline = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        jump(JUMP_FORWARD, Operand::Forward(join), 1),
        Item::Label(join),
        pass_instruction(NOP, 0, -1),
        pass_instruction(RETURN_VALUE, 0, -1),
    ]);
    assert!(pipeline.capture_original_location_absence().is_empty());

    let pipeline = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        jump(POP_JUMP_IF_TRUE, Operand::Forward(join), 1),
        Item::Label(join),
        pass_instruction(NOP, 0, -1),
        pass_instruction(RETURN_VALUE, 0, -1),
    ]);
    assert_eq!(
        pipeline.capture_original_location_absence(),
        [join].into_iter().collect()
    );

    let pipeline = optimization_pipeline(vec![
        Item::Label(entry),
        pass_instruction(NOP, 0, -1),
        pass_instruction(LOAD_CONST, 0, 1),
        jump(JUMP_BACKWARD, Operand::Backward(entry), 1),
    ]);
    assert!(pipeline.capture_original_location_absence().is_empty());

    let mut pipeline = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        jump(POP_JUMP_IF_TRUE, Operand::Forward(right), 1),
        Item::Label(left),
        pass_instruction(LOAD_CONST, 0, 2),
        jump(JUMP_FORWARD, Operand::Forward(join), -1),
        Item::Label(right),
        pass_instruction(LOAD_CONST, 0, 3),
        jump(JUMP_FORWARD, Operand::Forward(join), -1),
        Item::Label(join),
        Item::Label(alias),
        pass_instruction(NOP, 0, -1),
        pass_instruction(RETURN_VALUE, 0, -1),
    ]);
    assert_eq!(
        pipeline.capture_original_location_absence(),
        [join, alias].into_iter().collect()
    );
    pipeline.original_nop_targets.insert(alias);
    assert!(pipeline.capture_original_location_absence().is_empty());
}

#[test]
fn original_location_absence_matches_the_unoptimized_graph() {
    const SEGMENTS: usize = 8;
    let labels: [[super::Label; 2]; SEGMENTS] = std::array::from_fn(|index| {
        std::array::from_fn(|alias| {
            super::Label::from_u32(u32::try_from(index * 7 + alias).unwrap())
        })
    });
    for case in 0_u32..256 {
        let mut random = case + 1;
        let mut next = || {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            random >> 16
        };
        let mut items = Vec::new();
        let mut nop_targets = Vec::new();
        if next() & 1 != 0 {
            items.push(pass_instruction(NOP, 0, -1));
        }
        for (index, aliases) in labels.iter().enumerate() {
            for label in aliases {
                items.push(Item::Label(*label));
                if next().is_multiple_of(11) {
                    nop_targets.push(*label);
                }
            }
            let mut head = Instruction::new(
                NOP,
                Operand::Value(0),
                if next().is_multiple_of(3) {
                    SourceLocation::new(1, 1, 0, 0)
                } else {
                    SourceLocation::NONE
                },
                None,
            );
            if next().is_multiple_of(9) {
                head.add_pre_fold_extra_instructions(1);
            }
            items.push(Item::Instruction(head));
            let target_index = usize::try_from(next()).unwrap() % SEGMENTS;
            let target = labels[target_index][usize::try_from(next() % 2).unwrap()];
            let backwards = target_index <= index;
            let target = if backwards {
                Operand::Backward(target)
            } else {
                Operand::Forward(target)
            };
            let (opcode, operand) = match next() % 5 {
                0 => (NOP, Operand::Value(0)),
                1 => (RETURN_VALUE, Operand::Value(0)),
                2 if backwards => (JUMP_BACKWARD, target),
                2 => (JUMP_FORWARD, target),
                _ => (POP_JUMP_IF_TRUE, target),
            };
            items.push(Item::Instruction(Instruction::new(
                opcode,
                operand,
                SourceLocation::NONE,
                None,
            )));
        }
        items.push(Item::Label(super::Label::from_u32(100)));
        let mut pipeline = optimization_pipeline(items);
        if case.is_multiple_of(2) {
            pipeline.next_label = 0;
        }
        pipeline.original_nop_targets.extend(nop_targets);
        assert_eq!(
            pipeline.capture_original_location_absence(),
            pipeline.capture_original_location_absence_unoptimized_for_test(),
            "graph case {case}",
        );
    }
}

#[test]
fn original_location_absence_tracks_sparse_aliases_and_transitive_predecessors() {
    let left = super::Label::from_u32(2);
    let right = super::Label::from_u32(16);
    let join = super::Label::from_u32(33);
    let alias = super::Label::from_u32(57);
    let chained = super::Label::from_u32(80);
    let trailing = super::Label::from_u32(96);
    let jump = |opcode, target, line| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Forward(target),
            SourceLocation::new(line, line, 0, 0),
            None,
        ))
    };
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        jump(POP_JUMP_IF_TRUE, right, 1),
        Item::Label(left),
        pass_instruction(LOAD_CONST, 0, 2),
        jump(JUMP_FORWARD, join, -1),
        Item::Label(right),
        pass_instruction(LOAD_CONST, 0, 3),
        jump(JUMP_FORWARD, join, -1),
        Item::Label(join),
        Item::Label(alias),
        pass_instruction(NOP, 0, -1),
        jump(JUMP_FORWARD, chained, -1),
        Item::Label(chained),
        pass_instruction(RETURN_VALUE, 0, -1),
        Item::Label(trailing),
    ]);
    assembler.next_label = 0;

    assert_eq!(
        assembler.capture_original_location_absence(),
        [join, alias, chained].into_iter().collect()
    );
}

#[test]
fn cleanup_prefix_locations_retain_original_donors_through_late_writers() {
    for full_copy_path in [false, true] {
        for original_donor in [false, true] {
            for deferred_location in [0, 1, 2] {
                let [first, second, intermediate, cleanup, suffix, exit, handler] =
                    std::array::from_fn(|index| {
                        super::Label::from_u32(u32::try_from(index).unwrap())
                    });
                let jump = |opcode, target, line| {
                    Item::Instruction(Instruction::new(
                        opcode,
                        Operand::Forward(target),
                        SourceLocation::new(line, line, 0, 0),
                        None,
                    ))
                };
                let mut items = vec![
                    pass_instruction(LOAD_CONST, 0, 12),
                    pass_instruction(LOAD_CONST, 1, 12),
                    pass_instruction(LOAD_FAST, 0, 12),
                    jump(POP_JUMP_IF_TRUE, second, 12),
                    Item::Label(first),
                    pass_instruction(NOP, 0, 12),
                    jump(JUMP_FORWARD, intermediate, 12),
                    Item::Label(second),
                    pass_instruction(NOP, 0, 12),
                    jump(JUMP_FORWARD, intermediate, 12),
                    Item::Label(intermediate),
                    jump(JUMP_FORWARD, cleanup, if original_donor { 12 } else { -1 }),
                    Item::Label(cleanup),
                    pass_instruction(SWAP, 2, -1),
                    pass_instruction(POP_TOP, 0, -1),
                    Item::Label(suffix),
                    pass_instruction(NOP, 0, 2),
                    pass_instruction(NOP, 0, 2),
                    pass_instruction(NOP, 0, 2),
                ];
                if full_copy_path {
                    items.extend([
                        jump(JUMP_FORWARD, exit, 2),
                        Item::Label(exit),
                        pass_instruction(RETURN_VALUE, 0, 2),
                    ]);
                } else {
                    items.push(pass_instruction(RETURN_VALUE, 0, 2));
                }
                items.extend([Item::Label(handler), pass_instruction(RERAISE, 0, -1)]);
                let mut assembler = optimization_pipeline(items);
                if deferred_location != 0 {
                    let block = assembler.graph.label_block(cleanup).unwrap();
                    let pop = assembler
                        .graph
                        .instructions_mut(block)
                        .find(|instruction| instruction.opcode == POP_TOP)
                        .unwrap();
                    if deferred_location == 1 {
                        pop.fusion_policy.mark_location_from_next();
                    } else {
                        pop.no_location_policy.percent_target_location = 1;
                    }
                }
                assembler.exception_regions.push(super::ExceptionRegion {
                    priority: super::ExceptionRegionPriority::OriginalRange(cleanup, suffix),
                    is_scope_fallback: false,
                    start: cleanup,
                    end: suffix,
                    target: handler,
                    depth: 0,
                    preserve_lasti: false,
                });
                let original = assembler.capture_original_location_absence();
                assert_eq!(original.contains(&cleanup), !original_donor);
                assert!(assembler.thread_forward_jumps());
                assembler.remove_unreachable_instructions();
                let target = assembler.graph.label_block(cleanup).unwrap();
                let suffix_block = assembler.graph.label_block(suffix).unwrap();
                assert_ne!(target, suffix_block);
                let incoming = assembler
                    .graph
                    .iter_items()
                    .filter(|item| {
                        matches!(item, Item::Instruction(instruction)
                    if instruction.operand() == Operand::Forward(cleanup))
                    })
                    .count();
                assert_eq!(incoming, 2);
                // Reconstructing this certificate now would lose the original located predecessor.
                assert!(
                    assembler
                        .capture_original_location_absence()
                        .contains(&cleanup)
                );
                let copies = assembler.duplicate_exit_blocks(&original, &mut Vec::new());
                assert_eq!(copies.is_empty(), !full_copy_path);
                assembler.remove_redundant_forward_jumps(true);
                assembler.propagate_locations_within_blocks();
                let target = assembler.graph.label_block(cleanup).unwrap();
                let locations = assembler.graph[target]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.location.line),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    locations,
                    vec![
                        if original_donor || deferred_location != 0 {
                            12
                        } else {
                            -1
                        };
                        2
                    ]
                );
                let suffix_block = assembler.graph.label_block(suffix).unwrap();
                assert_eq!(
                    assembler.graph[suffix_block]
                        .items()
                        .iter()
                        .find_map(|item| match item {
                            Item::Instruction(instruction) => Some(instruction.location.line),
                            Item::Label(_) => None,
                        })
                        .unwrap(),
                    2
                );
            }
        }
    }
}

#[test]
fn uninitialized_load_analysis_preserves_original_exception_region_priority() {
    for fallback in [false, true] {
        for resolved in [false, true] {
            for cleared in [false, true] {
                let outer = super::Label::from_u32(0);
                let inner = super::Label::from_u32(1);
                let fragment = super::Label::from_u32(2);
                let end = super::Label::from_u32(3);
                let handler = super::Label::from_u32(4);
                let other_handler = super::Label::from_u32(5);
                let mut instructions = vec![
                    Item::Label(outer),
                    pass_instruction(LOAD_CONST, 0, 1),
                    pass_instruction(STORE_FAST, 0, 1),
                    Item::Label(inner),
                    pass_instruction(NOP, 0, 2),
                    Item::Label(fragment),
                ];
                if cleared {
                    instructions.push(pass_instruction(DELETE_FAST, 0, 3));
                }
                instructions.extend([
                    pass_instruction(LOAD_CONST, 0, 4),
                    pass_instruction(POP_TOP, 0, 4),
                    Item::Label(end),
                    pass_instruction(LOAD_CONST, 0, 5),
                    pass_instruction(RETURN_VALUE, 0, 5),
                    Item::Label(handler),
                    pass_instruction(LOAD_FAST, 0, 6),
                    pass_instruction(POP_TOP, 0, 6),
                    pass_instruction(RERAISE, 0, -1),
                    Item::Label(other_handler),
                    pass_instruction(RERAISE, 0, -1),
                ]);
                let mut assembler = optimization_pipeline(instructions);
                let priority = |start: super::Label| {
                    if resolved {
                        super::ExceptionRegionPriority::Resolved {
                            start: start.index(),
                            end: 20,
                        }
                    } else {
                        super::ExceptionRegionPriority::OriginalRange(start, end)
                    }
                };
                assembler.exception_regions.extend([
                    super::ExceptionRegion {
                        start: inner,
                        end,
                        target: handler,
                        depth: 0,
                        preserve_lasti: false,
                        is_scope_fallback: false,
                        priority: priority(inner),
                    },
                    super::ExceptionRegion {
                        start: fragment,
                        end,
                        target: other_handler,
                        depth: 0,
                        preserve_lasti: false,
                        is_scope_fallback: fallback,
                        priority: priority(if fallback { fragment } else { outer }),
                    },
                ]);
                assembler.add_checks_for_uninitialized_loads(
                    1,
                    0,
                    &rustc_hash::FxHashSet::default(),
                );
                let block = assembler.graph.label_block(handler).unwrap();
                let load = assembler.graph[block]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.opcode),
                        Item::Label(_) => None,
                    })
                    .unwrap();
                assert_eq!(load, if cleared { LOAD_FAST_CHECK } else { LOAD_FAST });
            }
        }
    }
}

#[test]
fn late_redundant_jump_removal_revisits_newly_adjacent_targets() {
    let intermediate = super::Label::from_u32(0);
    let target = super::Label::from_u32(1);
    let location = SourceLocation::new(7, 7, 3, 9);
    let mut jump = Instruction::new(JUMP_FORWARD, Operand::Forward(target), location, None);
    jump.exclude_from_exception_table(ExceptionExclusion::IfExtended);
    jump.no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(POP_TOP, 0, 7),
        Item::Instruction(jump),
        Item::Label(intermediate),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 8),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0].opcode, POP_TOP);
    assert_eq!(
        instructions[0].exception_policy().exclusion(),
        ExceptionExclusion::IfExtended
    );
    assert_eq!(
        instructions[0].no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(instructions[1].opcode, RETURN_VALUE);
    assert_eq!(instructions[1].location.line, 8);
}

#[test]
fn late_jump_retries_preserve_deferred_and_borrowed_markers() {
    for policy in ["deferred", "borrowed", "retained"] {
        let intermediate = super::Label::from_u32(0);
        let target = super::Label::from_u32(1);
        let mut jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        );
        match policy {
            "deferred" => jump.jump_policy.defer_removal(),
            "borrowed" => jump.jump_policy.mark_borrow_empty_block(),
            "retained" => jump.jump_policy.retain_nop(),
            _ => unreachable!(),
        }
        let mut assembler = optimization_pipeline(vec![
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(jump),
            Item::Label(intermediate),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::NONE,
                None,
            )),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, 1),
        ]);

        assembler.remove_redundant_forward_jumps(true);

        let instructions = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(instructions.len(), 3, "{policy}");
        let marker = instructions[1];
        match policy {
            "deferred" => {
                assert_eq!(marker.opcode, JUMP_FORWARD);
                assert!(!marker.jump_policy.defers_redundant_removal());
            }
            "borrowed" => {
                assert_eq!(marker.opcode, NOP);
                assert!(marker.jump_policy.borrow_empty_block());
            }
            "retained" => {
                assert_eq!(marker.opcode, NOP);
                assert!(marker.jump_policy.always_retains_nop());
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn late_jump_retries_require_proven_empty_line_blocks() {
    for (middle_line, next_line, preserved, retains_jump) in [
        (-1, 8, false, false),
        (7, 8, false, true),
        (7, 7, false, false),
        (7, -1, false, true),
        (-1, 8, true, true),
    ] {
        let intermediate = super::Label::from_u32(0);
        let target = super::Label::from_u32(1);
        let mut middle = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            if middle_line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(middle_line, middle_line, 3, 9)
            },
            None,
        );
        if preserved {
            middle.no_location_policy.preserve_location();
        }
        let mut assembler = optimization_pipeline(vec![
            pass_instruction(POP_TOP, 0, 7),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::new(7, 7, 3, 9),
                None,
            )),
            Item::Label(intermediate),
            Item::Instruction(middle),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, next_line),
        ]);
        assembler.remove_redundant_forward_jumps(true);
        let instructions = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(instructions.len(), if retains_jump { 3 } else { 2 });
        if retains_jump {
            assert_eq!(instructions[1].opcode, JUMP_FORWARD);
            assert_eq!(instructions[1].location, SourceLocation::new(7, 7, 3, 9));
            assert_eq!(instructions[1].operand(), Operand::Forward(target));
        }
    }
}

#[test]
fn shared_unlocated_jump_retries_require_original_conditional_predecessors() {
    for case in [
        "shared",
        "one",
        "aliases",
        "merged_arrays",
        "unreachable",
        "located_nop",
        "omitted_nop",
        "hidden_slot",
        "lost_predecessor",
    ] {
        let entry = super::Label::from_u32(0);
        let target = super::Label::from_u32(1);
        let alias = super::Label::from_u32(2);
        let second = super::Label::from_u32(3);
        let instruction = |opcode, target, line| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Forward(target),
                SourceLocation::new(line, line, 0, 1),
                None,
            ))
        };
        let mut items = Vec::new();
        if case == "unreachable" {
            items.push(instruction(JUMP_FORWARD, second, 1));
        }
        items.push(instruction(POP_JUMP_IF_FALSE, entry, 2));
        items.push(Item::Label(second));
        if case != "one" {
            items.push(instruction(
                POP_JUMP_IF_TRUE,
                if matches!(case, "aliases" | "merged_arrays") {
                    alias
                } else {
                    entry
                },
                3,
            ));
        }
        items.extend([
            pass_instruction(POP_TOP, 0, 7),
            instruction(JUMP_FORWARD, target, 7),
            Item::Label(entry),
        ]);
        if case == "merged_arrays" {
            let mut first_array = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::NONE,
                None,
            );
            first_array.no_location_policy.preserve_location();
            items.push(Item::Instruction(first_array));
        }
        items.push(Item::Label(alias));
        if case == "located_nop" {
            items.push(pass_instruction(NOP, 0, 9));
        }
        let mut middle = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        );
        middle.no_location_policy.preserve_location();
        if case == "hidden_slot" {
            middle.local_policy.add_hidden_exception_owner_slots(1);
        }
        items.extend([
            Item::Instruction(middle),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, 8),
        ]);
        let mut assembler = optimization_pipeline(items);
        if case == "omitted_nop" {
            assembler
                .exception_owner_provenance
                .omitted_nop_locations
                .insert(entry, SourceLocation::new(9, 9, 0, 1));
        }
        assembler.record_shared_unlocated_jump_entries();
        assert_eq!(
            assembler
                .original_shared_unlocated_jump_entries
                .contains_key(&entry),
            matches!(case, "shared" | "aliases" | "lost_predecessor"),
            "{case}"
        );
        // Later compaction can hide an authored NOP or merge labels. Neither creates
        // an original shared unlocated array; losing an edge also invalidates admission.
        let head = assembler.graph.label_block(entry).unwrap();
        assembler.graph.retain_items(
            head,
            |item| !matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP),
        );
        assembler.graph.for_each_instruction_mut(|instruction| {
            if instruction.opcode == POP_JUMP_IF_TRUE {
                if matches!(case, "aliases" | "merged_arrays") {
                    instruction.retarget(Operand::Forward(entry));
                } else if case == "lost_predecessor" {
                    instruction.retarget(Operand::Forward(target));
                }
            }
        });
        assembler.remove_redundant_forward_jumps(true);
        let kept_body_jump = assembler.graph.iter_items().any(|item| matches!(item,
            Item::Instruction(instruction) if instruction.opcode == JUMP_FORWARD && instruction.location.line == 7));
        assert_eq!(
            kept_body_jump,
            !matches!(case, "shared" | "aliases"),
            "{case}"
        );
    }
}

#[test]
fn folded_borrow_entries_resolve_before_graph_preparation() {
    let mut assembler = Assembler::default();
    let other = assembler.label();
    let target = assembler.label();
    assembler.emit(LOAD_CONST, 0);
    assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, other, 0, ExceptionPolicy::default());
    assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.record_folded_condition_borrow_entry(target);
    // A second branch emitted in the dead remainder must not replace the captured
    // predecessor set. The earlier independent destination is in a different block.
    assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.mark(target);
    assembler.emit(LOAD_CONST, 1);
    assembler.emit(RETURN_VALUE, 0);
    assembler.mark(other);
    assembler.emit(LOAD_CONST, 2);
    assembler.emit(RETURN_VALUE, 0);

    assembler.used_constant_indices(3);
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    assert!(assembler.deferred_borrow_entries.is_empty());
    assert!(assembler.borrow_unreachable_blocks.contains(&target));
    assert!(assembler.borrow_jump_entry_labels.contains(&target));
    // A second consumer must not reinterpret the already prepared graph's offsets.
    assembler.used_constant_indices(3);
    let pipeline = assembler.into_optimization_pipeline();
    assert!(pipeline.borrow_jump_entry_labels.contains(&target));
}

#[test]
fn folded_borrow_entries_preserve_a_future_alias() {
    let mut assembler = Assembler::default();
    let outer = assembler.label();
    let inner = assembler.label();
    assembler.emit(LOAD_CONST, 0);
    assembler.emit_forward_with_depth(POP_JUMP_IF_TRUE, outer, 0, ExceptionPolicy::default());
    assembler.emit_forward_with_depth(JUMP_FORWARD, inner, 0, ExceptionPolicy::default());
    assembler.record_folded_condition_borrow_entry(inner);
    // The label names differ during emission, but their completed original offsets
    // establish one block entered by the earlier live outer condition.
    assembler.mark(inner);
    assembler.mark(outer);
    assembler.emit(LOAD_CONST, 1);
    assembler.emit(RETURN_VALUE, 0);

    assembler.optimize_constant_pops();
    assert!(assembler.deferred_borrow_entries.is_empty());
    assert!(!assembler.borrow_unreachable_blocks.contains(&inner));
    assert!(!assembler.borrow_jump_entry_labels.contains(&inner));
    let pipeline = assembler.into_optimization_pipeline();
    assert!(!pipeline.borrow_jump_entry_labels.contains(&inner));
}

#[test]
fn copied_regions_retarget_only_handlers_copied_with_instructions() {
    for fallback in [false, true] {
        for shape in ["external", "internal", "other region end"] {
            let start = super::Label::from_u32(0);
            let end = super::Label::from_u32(1);
            let handler = super::Label::from_u32(2);
            let outer_handler = super::Label::from_u32(3);
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(start),
                SourceLocation::new(1, 1, 0, 0),
                None,
            );
            jump.jump_policy.retain_nop_for_direct_target();
            let mut items = vec![
                Item::Instruction(jump),
                Item::Label(start),
                pass_instruction(LOAD_CONST, 0, 2),
            ];
            if shape == "internal" {
                items.push(Item::Label(handler));
            }
            items.push(pass_instruction(RETURN_VALUE, 0, 2));
            if shape == "other region end" {
                items.extend([Item::Label(end), pass_instruction(RERAISE, 0, -1)]);
            }
            if shape != "internal" {
                items.extend([Item::Label(handler), pass_instruction(RERAISE, 0, -1)]);
            }
            items.extend([Item::Label(outer_handler), pass_instruction(RERAISE, 0, -1)]);
            let region_end = if shape == "other region end" {
                end
            } else {
                handler
            };
            let priority = super::ExceptionRegionPriority::OriginalRange(start, region_end);
            let mut assembler = optimization_pipeline(items);
            assembler.exception_regions.push(super::ExceptionRegion {
                priority,
                is_scope_fallback: fallback,
                start,
                end: region_end,
                target: handler,
                depth: 0,
                preserve_lasti: true,
            });
            if shape == "other region end" {
                // Another region's trailing end is also a handler label. Its boundary
                // alias must not redirect the first region's actual handler.
                assembler.exception_regions.push(super::ExceptionRegion {
                    priority: super::ExceptionRegionPriority::OriginalRange(start, handler),
                    is_scope_fallback: fallback,
                    start,
                    end: handler,
                    target: outer_handler,
                    depth: 1,
                    preserve_lasti: false,
                });
            }

            let original = assembler.capture_original_location_absence();
            assembler.duplicate_exit_blocks(&original, &mut Vec::new());

            let copied = assembler
                .exception_regions
                .iter()
                .find(|region| {
                    region.start != start
                        && matches!(
                            region.priority,
                            super::ExceptionRegionPriority::OriginalRange(first, last)
                                if first == start && last == region_end
                        )
                })
                .expect("the exit copy must retain its protected region");
            assert_ne!(copied.end, region_end, "{shape}/{fallback}");
            assert_eq!(copied.depth, 0, "{shape}/{fallback}");
            assert!(copied.preserve_lasti, "{shape}/{fallback}");
            assert_eq!(copied.is_scope_fallback, fallback, "{shape}/{fallback}");
            if shape == "internal" {
                assert_ne!(copied.target, handler, "{shape}/{fallback}");
                assert_eq!(copied.target, copied.end, "{shape}/{fallback}");
                let target = assembler.graph.label_block(copied.target).unwrap();
                assert_eq!(
                    assembler.graph[target].last_instruction().unwrap().opcode,
                    RETURN_VALUE,
                    "{shape}/{fallback}",
                );
            } else {
                assert_eq!(copied.target, handler, "{shape}/{fallback}");
                let target = assembler.graph.label_block(copied.target).unwrap();
                assert_eq!(
                    assembler.graph[target].last_instruction().unwrap().opcode,
                    RERAISE,
                    "{shape}/{fallback}",
                );
            }
            if shape == "other region end" {
                let other = assembler
                    .exception_regions
                    .iter()
                    .find(|region| region.start != start && region.depth == 1)
                    .expect("the second copied region must retain its handler");
                assert_eq!(other.target, outer_handler);
            }
        }
    }
}

#[test]
fn unlocated_borrow_marker_does_not_block_redundant_jump_removal() {
    for location in [SourceLocation::NONE, SourceLocation::new(2, 2, 0, 0)] {
        let target = super::Label::from_u32(1);
        let mut marker = Instruction::new(NOP, Operand::Value(0), location, None);
        marker.jump_policy.mark_borrow_empty_block();
        let mut assembler = optimization_pipeline(vec![
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::new(1, 1, 0, 0),
                None,
            )),
            Item::Label(super::Label::from_u32(0)),
            Item::Instruction(marker),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, 3),
        ]);

        assembler.remove_redundant_forward_jumps(true);

        let instructions = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.opcode == JUMP_FORWARD)
                .count(),
            usize::from(location.line >= 0),
        );
        assert!(instructions.contains(&&marker));
    }
}

#[test]
fn retained_borrow_marker_does_not_create_a_skipped_line_event() {
    let first_target = super::Label::from_u32(1);
    let second_target = super::Label::from_u32(2);
    let mut marker = Instruction::new(
        NOP,
        Operand::Value(0),
        SourceLocation::new(2, 2, 0, 1),
        None,
    );
    marker.jump_policy.mark_borrow_empty_block();
    marker.jump_policy.retain_nop();
    marker
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved);
    let mut second_jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(second_target),
        SourceLocation::new(3, 3, 0, 1),
        None,
    );
    second_jump
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut pipeline = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(first_target),
            SourceLocation::new(1, 1, 0, 1),
            None,
        )),
        Item::Instruction(marker),
        Item::Label(first_target),
        Item::Instruction(second_jump),
        Item::Label(second_target),
        pass_instruction(RETURN_VALUE, 0, 3),
    ]);

    pipeline.remove_redundant_forward_jumps(true);
    let mut items = pipeline.graph.into_items();
    super::OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);

    // If the marker's line event survives, the earlier edge must still skip it.
    let first_jump_survives = items.iter().any(|item| {
        matches!(item, Item::Instruction(instruction)
            if instruction.opcode == JUMP_FORWARD
                && instruction.operand() == Operand::Forward(first_target))
    });
    let marker_survives = items.iter().any(|item| {
        matches!(item, Item::Instruction(instruction)
            if instruction.opcode == NOP && instruction.location.line == 2)
    });
    assert!(first_jump_survives || !marker_survives);
}

#[test]
fn borrow_marker_conversion_retries_without_another_jump_deletion() {
    let target = super::Label::from_u32(1);
    let mut marker = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    marker.jump_policy.mark_borrow_empty_block();
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(super::Label::from_u32(0)),
        Item::Instruction(marker),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 3),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        instructions
            .iter()
            .map(|instruction| instruction.opcode)
            .collect::<Vec<_>>(),
        [LOAD_CONST, NOP, RETURN_VALUE]
    );
    assert!(instructions[1].jump_policy.borrow_empty_block());
    assert_eq!(instructions[1].location, SourceLocation::NONE);
    assert_eq!(instructions[1].operand(), Operand::Value(0));
}

#[test]
fn split_return_condition_transports_only_a_unique_exact_source_witness() {
    use super::Label;
    use super::exception_owners::SourceNoneCondition;

    let location = SourceLocation::new(13, 13, 15, 35);
    let mut condition = Instruction::new(
        POP_JUMP_IF_NOT_NONE,
        Operand::Forward(Label::from_u32(3)),
        location,
        Some(2),
    );
    condition.local_policy.add_hidden_exception_owner_slots(3);
    let mut updated = condition;
    updated.no_location_policy.allow_split_return_exit_copy();
    let marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, Some(2));

    for change in [
        "unique",
        "no origin",
        "duplicate origin",
        "original clone",
        "updated clone",
        "different operand",
        "different location",
        "different hidden slots",
    ] {
        let mut items = vec![Item::Label(Label::from_u32(0))];
        match change {
            "original clone" => items.push(Item::Instruction(condition)),
            "updated clone" => items.push(Item::Instruction(updated)),
            _ => {}
        }
        items.push(Item::Instruction(condition));
        items.push(Item::Instruction(marker));
        let mut assembler = Assembler::from_linear_items(items.clone());
        let mut source = condition;
        match change {
            "different operand" => source.set_operand(Operand::Forward(Label::from_u32(4))),
            "different location" => source.location = SourceLocation::new(14, 14, 15, 35),
            "different hidden slots" => source.local_policy.add_hidden_exception_owner_slots(1),
            _ => {}
        }
        let source_count = match change {
            "no origin" => 0,
            "duplicate origin" => 2,
            _ => 1,
        };
        for _ in 0..source_count {
            assembler
                .exception_owner_provenance
                .source_none_conditions
                .push(SourceNoneCondition {
                    entry: Label::from_u32(0),
                    labels: vec![Label::from_u32(0)],
                    instruction: source,
                    entry_prefix: Some(vec![Item::Label(Label::from_u32(0))]),
                });
        }
        let condition_item = items.len() - 2;
        items[condition_item] = Item::Instruction(updated);
        for _ in 0..2 {
            assembler.allow_last_condition_split_return_copy();
            assert_eq!(assembler.linear_items(), items.as_slice(), "{change}");
            let origins = &assembler.exception_owner_provenance.source_none_conditions;
            assert_eq!(origins.len(), source_count, "{change}");
            for origin in origins {
                assert_eq!(
                    origin.instruction,
                    if change == "unique" { updated } else { source },
                    "{change}"
                );
                assert_eq!(origin.entry, Label::from_u32(0), "{change}");
                assert_eq!(origin.labels, vec![Label::from_u32(0)], "{change}");
                assert_eq!(
                    origin.entry_prefix,
                    Some(vec![Item::Label(Label::from_u32(0))]),
                    "{change}"
                );
            }
        }
    }
}

#[test]
fn iterator_break_finalizers_copy_every_target_iterator_cell() {
    use crate::code_object::Constant;
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    let shared_return = "def choose(items, resource):\n    for item in items:\n        try:\n            if item:\n                break\n        finally:\n            try:\n                resource.close()\n            except OSError:\n                pass\n    return items\n";
    let guarded_handler = "def choose(items, resource, visit):\n    for item in items:\n        try:\n            visit(item)\n            break\n        except Exception as error:\n            if item:\n                break\n        finally:\n            if resource is not None:\n                try:\n                    resource.close()\n                except OSError:\n                    pass\n    visit(resource)\n";
    for (source, guarded) in [(shared_return, false), (guarded_handler, true)] {
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            let module = compile_with_options(
                source,
                "iterator_break_finalizer.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let code = module
                .code
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            let instructions = code.bytecode.as_chunks::<2>().0;
            let iterator_width = if version == PythonVersion::Py315 {
                2
            } else {
                1
            };
            let copied_disposals = if guarded {
                instructions
                    .windows(iterator_width + 1)
                    .filter(|window| {
                        window[..iterator_width]
                            .iter()
                            .all(|instruction| *instruction == [POP_TOP.code_for(version), 0])
                            && window[iterator_width][0] == JUMP_FORWARD.code_for(version)
                    })
                    .count()
            } else {
                let returns = instructions
                    .iter()
                    .filter(|instruction| instruction[0] == RETURN_VALUE.code_for(version))
                    .count();
                assert_eq!(returns, 3, "{version:?}");
                instructions
                    .windows(iterator_width + 2)
                    .filter(|window| {
                        window[..iterator_width]
                            .iter()
                            .all(|instruction| *instruction == [POP_TOP.code_for(version), 0])
                            && matches!(window[iterator_width][0], opcode
                                if opcode == LOAD_FAST.code_for(version)
                                    || opcode == LOAD_FAST_BORROW.code_for(version))
                            && window[iterator_width + 1][0] == RETURN_VALUE.code_for(version)
                    })
                    .count()
            };
            // Both predecessors receive the complete target-specific iterator cleanup,
            // whether they then copy a return or jump to an ordinary continuation.
            assert_eq!(copied_disposals, 2, "{version:?}, guarded={guarded}");
        }
    }
}

#[test]
fn ordinary_linear_terminal_raises_match_the_block_graph_pipeline() {
    let item = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            if line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(line, line, 2, 15)
            },
            Some(depth),
        ))
    };
    let resume = item(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0);
    let scenarios = [
        (0, 0, 0, vec![]),
        (
            0,
            0,
            0,
            vec![
                item(LOAD_GLOBAL, 1, 2, 2),
                item(LOAD_CONST, 300, 2, 3),
                item(CALL, 1, 2, 1),
                item(POP_TOP, 0, 2, 0),
                item(NOP, 0, 3, 0),
            ],
        ),
        (1, 1, 1, vec![item(LOAD_FAST, 0, 5, 1)]),
        (1, 0, 1, vec![item(LOAD_FAST, 0, 5, 1)]),
        (
            1,
            1,
            0,
            vec![
                item(LOAD_GLOBAL, 1, 4, 2),
                item(LOAD_FAST, 0, 4, 3),
                item(CALL, 1, 4, 1),
                item(COPY, 1, 4, 2),
                item(POP_TOP, 0, 4, 1),
            ],
        ),
        (
            2,
            2,
            2,
            vec![item(LOAD_FAST, 0, 5, 1), item(LOAD_FAST, 1, 5, 2)],
        ),
        (
            2,
            2,
            1,
            vec![
                item(LOAD_FAST, 0, 3, 1),
                item(STORE_FAST, 1, 3, 0),
                item(DELETE_FAST, 1, 4, 0),
                item(LOAD_FAST, 1, 4, 1),
                item(LOAD_GLOBAL, 1, 5, 3),
                item(LOAD_CONST, 300, 5, 4),
                item(CALL, 1, -1, 2),
            ],
        ),
    ];
    for (argument, local_count, parameter_count, body) in scenarios {
        for raise_line in [-1, 5] {
            let mut items = vec![resume];
            items.extend_from_slice(&body);
            items.push(item(RAISE_VARARGS, argument, raise_line, 0));
            assert_linear_matches_graph(
                || Assembler::from_linear_items(items.clone()),
                local_count,
                parameter_count,
            );
        }
    }

    for argument in 0..=2 {
        let mut items = vec![item(COPY_FREE_VARS, 1, -1, 0), resume];
        if argument > 0 {
            items.push(item(LOAD_DEREF, 0, 4, 1));
        }
        if argument == 2 {
            items.push(item(LOAD_CONST, 300, 5, 2));
        }
        items.push(item(RAISE_VARARGS, argument, 5, 0));
        assert_linear_matches_graph(|| Assembler::from_linear_items(items.clone()), 0, 0);
    }
}

#[test]
fn ordinary_linear_terminal_raises_reject_other_exits_and_metadata() {
    let location = SourceLocation::new(5, 5, 2, 15);
    let value = |opcode, argument, depth| {
        Instruction::new(opcode, Operand::Value(argument), location, Some(depth))
    };
    let resume = Item::Instruction(value(RESUME, ResumeLocation::AT_FUNC_START.argument(), 0));
    let load = Item::Instruction(value(LOAD_CONST, 1, 1));
    for terminal in [
        value(RETURN_VALUE, 0, 0),
        value(RAISE_VARARGS, 1, 0),
        value(RAISE_VARARGS, 2, 0),
    ] {
        let assembler = Assembler::from_linear_items(vec![resume, Item::Instruction(terminal)]);
        assert!(!assembler.has_only_linear_instructions(0));
    }

    let mut inline = value(RAISE_VARARGS, 1, 0);
    inline.location = SourceLocation::NONE;
    inline.no_location_policy.allow_block_inlining();
    let mut preserved = value(RAISE_VARARGS, 1, 0);
    preserved.no_location_policy.preserve_location();
    let mut exceptional = value(RAISE_VARARGS, 1, 0);
    exceptional.set_exception_policy(ExceptionPolicy::NonePresent);
    let mut folding = value(RAISE_VARARGS, 1, 0);
    folding.add_pre_fold_extra_instructions(1);
    for terminal in [
        value(RAISE_VARARGS, 3, 0),
        value(RAISE_VARARGS, 1, 1),
        value(RERAISE, 0, 0),
        inline,
        preserved,
        exceptional,
        folding,
    ] {
        let assembler =
            Assembler::from_linear_items(vec![resume, load, Item::Instruction(terminal)]);
        assert!(!assembler.has_only_linear_instructions(0));
    }

    let raise = Item::Instruction(value(RAISE_VARARGS, 1, 0));
    for items in [
        vec![resume, load, raise, load, raise],
        vec![resume, load, raise, Item::Label(super::Label::from_u32(0))],
        vec![resume, Item::Label(super::Label::from_u32(0)), load, raise],
    ] {
        assert!(!Assembler::from_linear_items(items).has_only_linear_instructions(0));
    }
}

fn assert_terminal_guards_match_graph(
    build: impl Fn() -> Assembler,
    local_count: usize,
    parameter_count: usize,
) {
    let linear = build();
    assert!(linear.has_only_linear_terminal_guards());
    let mut generic = build();
    let graph = super::block_graph::BlockGraph::from_items(
        generic.linear_items().to_vec(),
        &rustc_hash::FxHashSet::default(),
    );
    let reachable = graph.reachable_blocks(&[]);
    generic.code = UnoptimizedCode::Prepared { graph, reachable };
    let actual = linear.finish_code(1, local_count, parameter_count).unwrap();
    let expected = generic
        .finish_code(1, local_count, parameter_count)
        .unwrap();
    assert_eq!(actual.bytecode, expected.bytecode);
    assert_eq!(actual.line_table, expected.line_table);
    assert_eq!(actual.exception_table, expected.exception_table);
    assert_eq!(actual.max_depth, expected.max_depth);
    assert_eq!(actual.removed_max_depth, expected.removed_max_depth);
}

#[test]
fn terminal_guards_with_unreferenced_trailing_labels_match_the_block_graph_pipeline() {
    let location = |line| {
        if line < 0 {
            SourceLocation::NONE
        } else {
            SourceLocation::new(line, line, 2, 15)
        }
    };
    let item = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            location(line),
            Some(depth),
        ))
    };
    let jump = |target, line| {
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(target),
            location(line),
            Some(0),
        ))
    };
    let first = super::Label::from_u32(0);
    let second = super::Label::from_u32(1);
    let trailing = super::Label::from_u32(2);
    let extra = super::Label::from_u32(3);
    for (terminal, argument) in [(RETURN_VALUE, 0), (RAISE_VARARGS, 0), (RAISE_VARARGS, 2)] {
        for line in [-1, 6] {
            for suffix in [
                vec![Item::Label(trailing)],
                vec![Item::Label(trailing), Item::Label(extra)],
            ] {
                let mut items = vec![
                    item(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
                    item(LOAD_FAST, 0, 2, 1),
                    item(TO_BOOL, 0, 2, 1),
                    jump(first, 2),
                    item(NOT_TAKEN, 0, 2, 0),
                    item(LOAD_CONST, 1, 3, 1),
                    item(RETURN_VALUE, 0, 3, 0),
                    Item::Label(first),
                    item(LOAD_FAST, 1, 4, 1),
                    item(TO_BOOL, 0, 4, 1),
                    jump(second, 4),
                    item(NOT_TAKEN, 0, 4, 0),
                    item(LOAD_FAST, 0, 5, 1),
                    item(RAISE_VARARGS, 1, 5, 0),
                    Item::Label(second),
                ];
                if terminal == RETURN_VALUE || argument > 0 {
                    items.push(item(LOAD_FAST, 1, line, 1));
                }
                if argument == 2 {
                    items.push(item(LOAD_CONST, 2, line, 2));
                }
                items.push(item(terminal, argument, line, 0));
                items.extend(suffix);
                assert_terminal_guards_match_graph(
                    || {
                        let mut assembler = Assembler::from_linear_items(items.clone());
                        assembler.next_label = 4;
                        assembler
                    },
                    2,
                    2,
                );
            }
        }
    }

    let original = vec![
        item(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
        item(LOAD_FAST, 0, 2, 1),
        item(TO_BOOL, 0, 2, 1),
        jump(first, 2),
        item(NOT_TAKEN, 0, 2, 0),
        item(RAISE_VARARGS, 0, 3, 0),
        Item::Label(first),
        item(RAISE_VARARGS, 0, 4, 0),
    ];
    for suffix in [
        vec![Item::Label(first)],
        vec![Item::Label(trailing), Item::Label(first)],
        vec![Item::Label(trailing), Item::Label(trailing)],
        vec![Item::Label(trailing), item(NOP, 0, 5, 0)],
        vec![Item::Label(trailing), item(RAISE_VARARGS, 0, 5, 0)],
    ] {
        let mut items = original.clone();
        items.extend(suffix);
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 3;
        assert!(!assembler.has_only_linear_terminal_guards());
    }
}

#[test]
fn short_circuit_terminal_guards_merge_local_state_like_the_block_graph_pipeline() {
    let location = |line| SourceLocation::new(line, line, 2, 15);
    let value = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            location(line),
            Some(depth),
        ))
    };
    let target = super::Label::from_u32(0);
    let jump = |opcode, line| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Forward(target),
            location(line),
            Some(0),
        ))
    };
    for mode in ["before", "walrus between", "delete between", "unset"] {
        for (terminal, argument) in [(RETURN_VALUE, 0), (RAISE_VARARGS, 0), (RAISE_VARARGS, 1)] {
            for third in [false, true] {
                let mut items = vec![value(
                    RESUME,
                    ResumeLocation::AT_FUNC_START.argument(),
                    1,
                    0,
                )];
                if matches!(mode, "before" | "delete between") {
                    items.extend([value(LOAD_CONST, 1, 2, 1), value(STORE_FAST, 1, 2, 0)]);
                }
                items.extend([
                    value(LOAD_FAST, 0, 3, 1),
                    value(TO_BOOL, 0, 3, 1),
                    jump(POP_JUMP_IF_FALSE, 3),
                    value(NOT_TAKEN, 0, 3, 0),
                ]);
                if mode == "delete between" {
                    items.push(value(DELETE_FAST, 1, 4, 0));
                }
                items.push(value(LOAD_FAST, 0, 4, 1));
                if mode == "walrus between" {
                    items.extend([value(COPY, 1, 4, 2), value(STORE_FAST, 1, 4, 1)]);
                }
                items.extend([
                    value(TO_BOOL, 0, 4, 1),
                    jump(POP_JUMP_IF_FALSE, 4),
                    value(NOT_TAKEN, 0, 4, 0),
                ]);
                if third {
                    items.extend([
                        value(LOAD_FAST, 0, 5, 1),
                        jump(POP_JUMP_IF_NONE, 5),
                        value(NOT_TAKEN, 0, 5, 0),
                    ]);
                }
                if terminal == RETURN_VALUE || argument != 0 {
                    items.push(value(LOAD_FAST, 0, 6, 1));
                }
                items.extend([
                    value(terminal, argument, 6, 0),
                    Item::Label(target),
                    value(LOAD_FAST, 1, 8, 1),
                    value(RETURN_VALUE, 0, 8, 0),
                ]);
                let build = || {
                    let mut assembler = Assembler::from_linear_items(items.clone());
                    assembler.next_label = 1;
                    assembler
                };
                assert_terminal_guards_match_graph(build, 2, 1);
                let code = build().finish_code(1, 2, 1).unwrap();
                let last_load = code
                    .bytecode
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .rev()
                    .nth(1)
                    .unwrap();
                assert_eq!(last_load[0] == LOAD_FAST_CHECK.code(), mode != "before");
            }
        }
    }
}

#[test]
fn short_circuit_terminal_guards_reject_graph_only_exit_and_value_rewrites() {
    let location = SourceLocation::new(2, 2, 2, 15);
    let value = |opcode, argument, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            location,
            Some(depth),
        ))
    };
    let target = super::Label::from_u32(0);
    let other = super::Label::from_u32(1);
    let jump = |target| {
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(target),
            location,
            Some(0),
        ))
    };
    let baseline = vec![
        value(RESUME, ResumeLocation::AT_FUNC_START.argument(), 0),
        value(LOAD_FAST, 0, 1),
        value(TO_BOOL, 0, 1),
        jump(target),
        value(NOT_TAKEN, 0, 0),
        value(LOAD_FAST, 0, 1),
        value(TO_BOOL, 0, 1),
        jump(target),
        value(NOT_TAKEN, 0, 0),
        value(RAISE_VARARGS, 0, 0),
        Item::Label(target),
        value(LOAD_FAST, 0, 1),
        value(RETURN_VALUE, 0, 0),
    ];
    let recognize = |items| {
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 2;
        assembler.has_only_linear_terminal_guards()
    };
    assert!(recognize(baseline.clone()));
    for change in [
        "unlocated target",
        "only unstable target location",
        "first split-copy policy",
        "second split-copy policy",
        "artificial target exit",
        "value-preserving conditional",
        "different target",
        "missing marker",
    ] {
        let mut items = baseline.clone();
        match change {
            "unlocated target" | "only unstable target location" => {
                for item in &mut items[11..] {
                    let Item::Instruction(instruction) = item else {
                        unreachable!();
                    };
                    instruction.location = SourceLocation::NONE;
                }
                if change == "only unstable target location" {
                    items.insert(11, value(NOP, 0, 0));
                }
            }
            "first split-copy policy" | "second split-copy policy" => {
                let index = if change == "first split-copy policy" {
                    3
                } else {
                    7
                };
                let Item::Instruction(instruction) = &mut items[index] else {
                    unreachable!();
                };
                instruction
                    .no_location_policy
                    .allow_split_return_exit_copy();
            }
            "artificial target exit" => {
                let Item::Instruction(instruction) = &mut items[12] else {
                    unreachable!();
                };
                instruction.no_location_policy.mark_artificial_eval_break();
            }
            "value-preserving conditional" => items.insert(2, value(COPY, 1, 2)),
            "different target" => items[7] = jump(other),
            "missing marker" => items[8] = value(POP_TOP, 0, 0),
            _ => unreachable!(),
        }
        assert!(!recognize(items), "{change}");
    }
}

#[test]
fn falling_forward_guards_match_the_block_graph_pipeline() {
    let location = |line| SourceLocation::new(line, line, 2, 15);
    let value = |opcode, argument, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            location(line),
            Some(depth),
        ))
    };
    let first = super::Label::from_u32(0);
    let second = super::Label::from_u32(1);
    let jump = |opcode, target, line| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Forward(target),
            location(line),
            Some(0),
        ))
    };
    for mode in ["call", "store", "delete", "side walrus", "condition walrus"] {
        for second_guard in [false, true] {
            let mut items = vec![value(
                RESUME,
                ResumeLocation::AT_FUNC_START.argument(),
                1,
                0,
            )];
            if matches!(mode, "call" | "delete") {
                items.extend([value(LOAD_CONST, 1, 2, 1), value(STORE_FAST, 2, 2, 0)]);
            }
            items.push(value(LOAD_FAST, 0, 3, 1));
            if mode == "condition walrus" {
                items.extend([value(COPY, 1, 3, 2), value(STORE_FAST, 2, 3, 1)]);
            }
            items.extend([
                value(TO_BOOL, 0, 3, 1),
                jump(POP_JUMP_IF_FALSE, first, 3),
                value(NOT_TAKEN, 0, 3, 0),
            ]);
            match mode {
                "store" => {
                    items.extend([value(LOAD_FAST, 1, 4, 1), value(STORE_FAST, 2, 4, 0)]);
                }
                "delete" => items.push(value(DELETE_FAST, 2, 4, 0)),
                "side walrus" => items.extend([
                    value(LOAD_FAST, 1, 4, 1),
                    value(COPY, 1, 4, 2),
                    value(STORE_FAST, 2, 4, 1),
                    value(POP_TOP, 0, 4, 0),
                ]),
                "call" | "condition walrus" => items.extend([
                    value(LOAD_FAST, 1, 4, 1),
                    value(PUSH_NULL, 0, 4, 2),
                    value(CALL, 0, 4, 1),
                    value(POP_TOP, 0, 4, 0),
                ]),
                _ => unreachable!(),
            }
            items.push(Item::Label(first));
            if second_guard {
                items.extend([
                    value(LOAD_FAST, 0, 5, 1),
                    jump(POP_JUMP_IF_NONE, second, 5),
                    value(NOT_TAKEN, 0, 5, 0),
                    value(LOAD_CONST, 3, 6, 1),
                    value(STORE_GLOBAL, 0, 6, 0),
                    Item::Label(second),
                ]);
            }
            items.extend([
                value(LOAD_FAST, 2, 8, 1),
                value(LOAD_CONST, 4, 8, 2),
                value(BUILD_TUPLE, 2, 8, 1),
                value(RETURN_VALUE, 0, 8, 0),
            ]);
            let build = || {
                let mut assembler = Assembler::from_linear_items(items.clone());
                assembler.next_label = 2;
                assembler
            };
            assert_terminal_guards_match_graph(build, 3, 2);
            let mut assembler = build();
            assert_eq!(
                assembler.used_constant_indices(5),
                [
                    false,
                    matches!(mode, "call" | "delete"),
                    false,
                    second_guard,
                    true,
                ]
            );
            assert!(matches!(assembler.code, UnoptimizedCode::Linear(_)));
            let code = assembler.finish_code(1, 3, 2).unwrap();
            let checked = code
                .bytecode
                .as_chunks::<2>()
                .0
                .contains(&[LOAD_FAST_CHECK.code(), 2]);
            assert_eq!(checked, matches!(mode, "store" | "delete" | "side walrus"));
        }
    }

    let items = vec![
        value(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1, 0),
        value(LOAD_FAST, 0, 3, 1),
        value(TO_BOOL, 0, 3, 1),
        jump(POP_JUMP_IF_FALSE, first, 3),
        value(NOT_TAKEN, 0, 3, 0),
        value(LOAD_FAST, 1, 4, 1),
        value(TO_BOOL, 0, 4, 1),
        jump(POP_JUMP_IF_FALSE, first, 4),
        value(NOT_TAKEN, 0, 4, 0),
        value(LOAD_FAST, 0, 5, 1),
        value(STORE_FAST, 2, 5, 0),
        Item::Label(first),
        value(LOAD_FAST, 2, 8, 1),
        value(RETURN_VALUE, 0, 8, 0),
    ];
    assert_terminal_guards_match_graph(
        || {
            let mut assembler = Assembler::from_linear_items(items.clone());
            assembler.next_label = 1;
            assembler
        },
        3,
        2,
    );
}

#[test]
fn falling_forward_guards_reject_unstable_joins_and_nonstatement_arms() {
    let location = SourceLocation::new(2, 2, 2, 15);
    let value = |opcode, argument, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            location,
            Some(depth),
        ))
    };
    let target = super::Label::from_u32(0);
    let jump = Item::Instruction(Instruction::new(
        POP_JUMP_IF_FALSE,
        Operand::Forward(target),
        location,
        Some(0),
    ));
    let baseline = vec![
        value(RESUME, ResumeLocation::AT_FUNC_START.argument(), 0),
        value(LOAD_FAST, 0, 1),
        value(TO_BOOL, 0, 1),
        jump,
        value(NOT_TAKEN, 0, 0),
        value(LOAD_FAST, 0, 1),
        value(STORE_FAST, 1, 0),
        Item::Label(target),
        value(LOAD_FAST, 0, 1),
        value(RETURN_VALUE, 0, 0),
    ];
    let recognizes = |items| {
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 1;
        assembler.has_only_linear_terminal_guards()
    };
    assert!(recognizes(baseline.clone()));
    for change in [
        "no genuine action",
        "side keeps a stack value",
        "guard has a stack value",
        "marker has a stack value",
        "unlocated first join instruction",
        "unstable first join instruction",
        "copy policy",
        "artificial join exit",
        "threadable copy",
        "generator",
    ] {
        let mut items = baseline.clone();
        match change {
            "no genuine action" => items[6] = value(POP_TOP, 0, 0),
            "side keeps a stack value" => items.insert(7, value(LOAD_FAST, 0, 1)),
            "guard has a stack value" | "marker has a stack value" => {
                let index = if change == "guard has a stack value" {
                    3
                } else {
                    4
                };
                let Item::Instruction(instruction) = &mut items[index] else {
                    unreachable!();
                };
                *instruction = Instruction::new(
                    instruction.opcode,
                    instruction.operand(),
                    instruction.location,
                    Some(1),
                );
            }
            "unlocated first join instruction" => {
                let Item::Instruction(instruction) = &mut items[8] else {
                    unreachable!();
                };
                instruction.location = SourceLocation::NONE;
            }
            "unstable first join instruction" => items.insert(8, value(NOP, 0, 0)),
            "copy policy" => {
                let Item::Instruction(instruction) = &mut items[3] else {
                    unreachable!();
                };
                instruction
                    .no_location_policy
                    .allow_split_return_exit_copy();
            }
            "artificial join exit" => {
                let Item::Instruction(instruction) = &mut items[9] else {
                    unreachable!();
                };
                instruction.no_location_policy.mark_artificial_eval_break();
            }
            "threadable copy" => items.insert(2, value(COPY, 1, 2)),
            "generator" => items.insert(1, value(RETURN_GENERATOR, 0, 0)),
            _ => unreachable!(),
        }
        assert!(!recognizes(items), "{change}");
    }
}

#[test]
fn terminal_guard_fallthrough_alias_locations_match_the_graph() {
    let [target, alias, trailing] = [0, 1, 2].map(super::Label::from_u32);
    let located = SourceLocation::new(2, 2, 4, 11);
    let later = SourceLocation::new(3, 3, 2, 8);
    let item = |opcode, argument, location, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(argument),
            location,
            Some(depth),
        ))
    };
    for (marker, first_location, return_location, preserve, nop, trailing_label) in [
        (
            located,
            SourceLocation::NONE,
            SourceLocation::NONE,
            false,
            false,
            false,
        ),
        (
            later,
            SourceLocation::NONE,
            SourceLocation::NONE,
            false,
            false,
            false,
        ),
        (
            located,
            SourceLocation::NONE,
            SourceLocation::NONE,
            false,
            false,
            true,
        ),
        (
            located,
            SourceLocation::NONE,
            SourceLocation::NONE,
            true,
            false,
            false,
        ),
        (
            located,
            SourceLocation::NONE,
            SourceLocation::NONE,
            false,
            true,
            false,
        ),
        (located, located, SourceLocation::NONE, false, false, false),
        (located, SourceLocation::NONE, later, false, false, false),
    ] {
        let mut first = Instruction::new(LOAD_CONST, Operand::Value(0), first_location, Some(1));
        if preserve {
            first.no_location_policy.preserve_location();
        }
        let mut items = vec![
            item(
                RESUME,
                ResumeLocation::AT_FUNC_START.argument(),
                SourceLocation::new(1, 1, 0, 0),
                0,
            ),
            item(LOAD_FAST, 0, located, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(target),
                located,
                Some(0),
            )),
            item(NOT_TAKEN, 0, marker, 0),
            Item::Label(alias),
        ];
        if nop {
            items.push(item(NOP, 0, SourceLocation::NONE, 0));
        }
        items.extend([
            Item::Instruction(first),
            item(RETURN_VALUE, 0, return_location, 0),
            Item::Label(target),
            item(LOAD_CONST, 0, SourceLocation::NONE, 1),
            item(RETURN_VALUE, 0, SourceLocation::NONE, 0),
        ]);
        if trailing_label {
            items.push(Item::Label(trailing));
        }
        let make_assembler = || {
            let mut assembler = Assembler::from_linear_items(items.clone());
            assembler.next_label = 3;
            assembler
        };
        assert_terminal_guards_match_graph(make_assembler, 1, 1);
    }
}

#[test]
fn terminal_guard_alias_rejects_additional_incoming_edges() {
    let [target, alias] = [0, 1].map(super::Label::from_u32);
    let location = SourceLocation::new(2, 2, 4, 11);
    let instruction = |opcode, operand, depth| {
        Item::Instruction(Instruction::new(opcode, operand, location, Some(depth)))
    };
    let original = vec![
        instruction(RESUME, Operand::Value(0), 0),
        instruction(LOAD_FAST, Operand::Value(0), 1),
        instruction(POP_JUMP_IF_FALSE, Operand::Forward(target), 0),
        instruction(NOT_TAKEN, Operand::Value(0), 0),
        Item::Label(alias),
        instruction(LOAD_CONST, Operand::Value(0), 1),
        instruction(RETURN_VALUE, Operand::Value(0), 0),
        Item::Label(target),
        instruction(LOAD_CONST, Operand::Value(0), 1),
        instruction(RETURN_VALUE, Operand::Value(0), 0),
    ];
    let mut valid = Assembler::from_linear_items(original.clone());
    valid.next_label = 2;
    assert!(valid.has_only_linear_terminal_guards());

    for (opcode, operand, prepend) in [
        (JUMP_BACKWARD, Operand::Backward(alias), false),
        (POP_JUMP_IF_TRUE, Operand::Backward(alias), false),
        (POP_JUMP_IF_TRUE, Operand::Forward(alias), true),
    ] {
        let mut items = original.clone();
        if prepend {
            items.splice(
                1..1,
                [
                    instruction(LOAD_FAST, Operand::Value(0), 1),
                    instruction(opcode, operand, 0),
                    instruction(NOT_TAKEN, Operand::Value(0), 0),
                ],
            );
        } else {
            items.splice(
                5..5,
                [
                    instruction(LOAD_FAST, Operand::Value(0), 1),
                    instruction(opcode, operand, 0),
                ],
            );
        }
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 2;
        assert!(!assembler.has_only_linear_terminal_guards());
        assert!(!assembler.has_only_linear_assertion_side_exits());
        assert!(!assembler.has_only_linear_constant_return());
        assert!(!assembler.has_only_linear_instructions(1));
        assembler.used_constant_indices(1);
        assert!(!assembler.has_proven_linear_terminal_guards);
        assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    }
}
