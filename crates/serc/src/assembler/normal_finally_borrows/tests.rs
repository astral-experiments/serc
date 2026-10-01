use super::*;
use crate::assembler::{BorrowFallthrough, BorrowLoopExitPolicy, ExceptionPolicy, Label};
use crate::target::opcodes::NOP;

fn location(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 4, 12)
}

fn items_mut(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!();
    };
    items
}

fn fixture(version: PythonVersion, opcode: Opcode) -> (Assembler, Label) {
    let mut assembler = Assembler::with_python_version(version);
    let head = assembler.label();
    assembler.mark(head);
    assembler.record_exception_owner_block_boundary(head);
    assembler.set_location(location(1));
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
    for instruction in [
        Instruction::new(opcode, Operand::Value(1), location(2), Some(1)),
        Instruction::new(POP_TOP, Operand::Value(0), location(2), Some(0)),
        Instruction::new(LOAD_SMALL_INT, Operand::Value(0), location(3), Some(1)),
        Instruction::new(STORE_FAST, Operand::Value(2), location(4), Some(0)),
    ] {
        items_mut(&mut assembler).push(Item::Instruction(instruction));
    }
    (assembler, head)
}

fn accepts(assembler: &Assembler, opcode: Opcode) -> bool {
    assembler.current_original_call_store_source(
        (opcode, 1, location(2)),
        (0, location(3)),
        (2, location(4)),
    )
}

#[test]
fn original_call_store_requires_every_complete_suffix_record() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for opcode in [CALL, CALL_KW] {
            let (assembler, _) = fixture(version, opcode);
            assert_eq!(accepts(&assembler, opcode), version == PythonVersion::Py314);
        }
    }
    for position in 2..6 {
        for mutation in [
            "operand",
            "location",
            "depth",
            "owner",
            "hidden",
            "folded",
            "barrier",
            "label",
            "duplicate",
            "move",
        ] {
            let (mut assembler, _) = fixture(PythonVersion::Py314, CALL);
            let Item::Instruction(mut instruction) = assembler.linear_items()[position] else {
                unreachable!();
            };
            match mutation {
                "operand" => instruction.set_operand(Operand::Value(99)),
                "location" => instruction.location.column += 1,
                "depth" => {
                    instruction = Instruction::new(
                        instruction.opcode,
                        instruction.operand(),
                        instruction.location,
                        Some(9),
                    );
                }
                "owner" => instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "hidden" => instruction.local_policy.add_hidden_exception_owner_slots(1),
                "folded" => instruction.add_pre_fold_extra_instructions(1),
                "barrier" => instruction
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                "label" | "duplicate" | "move" => {}
                _ => unreachable!(),
            }
            assembler.linear_items_mut()[position] = Item::Instruction(instruction);
            match mutation {
                "label" => {
                    let label = assembler.label();
                    items_mut(&mut assembler).insert(position.max(3), Item::Label(label));
                }
                "duplicate" => {
                    let suffix = assembler.linear_items()[2..6].to_vec();
                    items_mut(&mut assembler).extend(suffix);
                }
                "move" => assembler.linear_items_mut().swap(position, position - 1),
                _ => {}
            }
            assert!(!accepts(&assembler, CALL), "{position} {mutation}");
        }
    }
}

#[test]
fn original_call_store_refuses_independent_array_roles() {
    for placement in ["head", "head alias", "tail alias"] {
        for role in [
            "preserved",
            "unreachable",
            "borrow alias",
            "jump entry",
            "finally entry",
            "loop exit",
            "threaded",
            "folded condition",
            "folded else",
            "folded exit",
            "nop",
            "cleanup",
            "deferred",
            "shared copy",
            "omitted count",
            "omitted location",
            "borrow boundary",
            "retained pop",
            "ordinary setup",
            "setup",
            "exception target",
        ] {
            let (mut assembler, head) = fixture(PythonVersion::Py314, CALL);
            let label = if placement == "head" {
                head
            } else {
                let alias = assembler.label();
                if placement == "head alias" {
                    items_mut(&mut assembler).insert(0, Item::Label(alias));
                } else {
                    assembler.mark(alias);
                }
                alias
            };
            assert!(accepts(&assembler, CALL), "inert {placement}");
            match role {
                "preserved" => {
                    assembler.preserved_block_boundaries.insert(label);
                }
                "unreachable" => {
                    assembler.borrow_unreachable_blocks.insert(label);
                }
                "borrow alias" => {
                    assembler.borrow_block_aliases.insert(label);
                }
                "jump entry" => {
                    assembler.borrow_jump_entry_labels.insert(label);
                }
                "finally entry" => {
                    assembler.borrow_finally_entries.insert(label, None);
                }
                "loop exit" => {
                    assembler
                        .borrow_loop_exit_blocks
                        .insert(label, BorrowLoopExitPolicy::default());
                }
                "threaded" => {
                    assembler
                        .conditional_threading_targets
                        .insert(label, location(1));
                }
                "folded condition" => {
                    assembler.folded_away_condition_targets.insert(label);
                }
                "folded else" => {
                    assembler.folded_away_loop_else_targets.insert(label);
                }
                "folded exit" => {
                    assembler.folded_condition_exit_targets.insert(label);
                }
                "nop" => {
                    assembler.original_nop_targets.insert(label);
                }
                "cleanup" => {
                    assembler.original_unlocated_cleanup_targets.insert(label);
                }
                "deferred" => {
                    assembler.deferred_shared_jump_blocks.insert(label);
                }
                "shared copy" => {
                    assembler.shared_jump_copy_location_blocks.insert(label);
                }
                "omitted count" => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .insert(label, 1);
                }
                "omitted location" => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_locations
                        .insert(label, location(1));
                }
                "borrow boundary" => {
                    assembler
                        .exception_owner_provenance
                        .borrow_only_block_boundaries
                        .insert(label);
                }
                "retained pop" => {
                    assembler
                        .retained_pop_block_origins
                        .insert(label, Vec::new());
                }
                "ordinary setup" => {
                    assembler
                        .exception_owner_provenance
                        .ordinary_try_setups
                        .insert(label, head);
                }
                "setup" => {
                    assembler
                        .exception_owner_provenance
                        .setups
                        .push((label, Some(location(1))));
                }
                "exception target" => {
                    assembler.add_exception_region(head, label, label, 1, true);
                }
                _ => unreachable!(),
            }
            assert!(!accepts(&assembler, CALL), "{placement} {role}");
        }
    }
}

#[test]
fn original_call_store_refuses_prepared_graphs() {
    let (mut assembler, _) = fixture(PythonVersion::Py314, CALL);
    let graph = crate::assembler::block_graph::BlockGraph::from_items(
        assembler.linear_items().to_vec(),
        &rustc_hash::FxHashSet::default(),
    );
    let reachable = vec![true; graph.len()];
    assembler.code = UnoptimizedCode::Prepared { graph, reachable };
    assert!(!accepts(&assembler, CALL));
}

#[test]
fn original_call_store_binds_one_registered_head_and_contiguous_suffix() {
    for mutation in [
        "source missing",
        "block missing",
        "split boundary",
        "duplicate label",
        "second head",
        "interior entry",
        "empty",
    ] {
        let (mut assembler, head) = fixture(PythonVersion::Py314, CALL);
        let alias = assembler.label();
        match mutation {
            "source missing" => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&head);
            }
            "block missing" => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .remove(&head);
            }
            "split boundary" => {
                items_mut(&mut assembler).insert(0, Item::Label(alias));
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&head);
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(alias);
            }
            "duplicate label" => {
                items_mut(&mut assembler).insert(0, Item::Label(head));
            }
            "second head" => {
                items_mut(&mut assembler).insert(0, Item::Label(alias));
                assembler.record_exception_owner_block_boundary(alias);
            }
            "interior entry" => {
                assembler.mark(alias);
                items_mut(&mut assembler).insert(
                    0,
                    Item::Instruction(Instruction::new(
                        crate::target::opcodes::JUMP_FORWARD,
                        Operand::Forward(alias),
                        location(1),
                        Some(0),
                    )),
                );
            }
            "empty" => items_mut(&mut assembler).clear(),
            _ => unreachable!(),
        }
        assert!(!accepts(&assembler, CALL), "{mutation}");
    }
}
