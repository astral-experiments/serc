use super::*;
use crate::assembler::Label;
use crate::assembler::block_graph::BlockGraph;
use crate::target::opcodes::{LOAD_FAST, RETURN_VALUE, STORE_FAST};

fn label(value: u32) -> Label {
    Label::from_u32(value)
}
fn location(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 4, 16)
}

fn fixture(prefix: bool) -> Assembler {
    let instruction = |opcode, operand, line, depth| {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            location(line),
            Some(depth),
        ))
    };
    let mut items = vec![Item::Label(label(0))];
    let primitive = Instruction::new(
        POP_JUMP_IF_FALSE,
        Operand::Forward(label(4)),
        location(3),
        Some(1),
    );
    if prefix {
        let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location(3), Some(1));
        marker.set_exception_policy(ExceptionPolicy::NonePresent);
        items.extend([
            Item::Instruction(primitive),
            Item::Instruction(marker),
            Item::Label(label(1)),
        ]);
    }
    let mut bridge = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(label(3)),
        location(4),
        Some(1),
    );
    bridge.jump_policy.prevent_small_exit_inlining();
    items.extend([
        instruction(LOAD_FAST, Operand::Value(0), 4, 3),
        instruction(POP_JUMP_IF_FALSE, Operand::Forward(label(2)), 4, 2),
        instruction(NOT_TAKEN, Operand::Value(0), 4, 2),
        instruction(LOAD_FAST, Operand::Value(1), 4, 2),
        instruction(POP_JUMP_IF_FALSE, Operand::Forward(label(4)), 4, 1),
        instruction(NOT_TAKEN, Operand::Value(0), 4, 1),
        Item::Instruction(bridge),
        Item::Label(label(2)),
        instruction(POP_TOP, Operand::Value(0), 4, 1),
        instruction(JUMP_FORWARD, Operand::Forward(label(4)), 4, 1),
        Item::Label(label(3)),
    ]);
    let mut assembler = Assembler::from_linear_items(items);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 5;
    assembler.record_original_comparison_fallthrough(label(2), label(3));
    assert_eq!(
        assembler
            .exception_owner_provenance
            .original_comparison_fallthroughs
            .len(),
        1
    );
    items_mut(&mut assembler).extend([
        instruction(LOAD_FAST, Operand::Value(0), 5, 2),
        instruction(STORE_FAST, Operand::Value(1), 5, 1),
        Item::Label(label(4)),
    ]);
    for label in [label(1), label(3), label(4)] {
        assembler
            .exception_owner_provenance
            .source_boundaries
            .insert(label);
        assembler
            .exception_owner_provenance
            .block_boundaries
            .insert(label);
    }
    if prefix {
        assembler
            .exception_owner_provenance
            .original_primitive_condition_boundaries
            .push(primitive);
    }
    assembler
}

fn items_mut(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items
}

fn apply(assembler: &Assembler, prefix: bool) -> bool {
    assembler.at_original_named_chain_cleanup(
        0,
        location(4),
        prefix.then_some((location(3), ExceptionPolicy::NonePresent)),
    )
}

#[test]
fn named_chain_cleanup_uses_live_receipts_without_mutating_records() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        for prefix in [false, true] {
            let mut assembler = fixture(prefix);
            assembler.python_version = version;
            let before = assembler.linear_items().to_vec();
            assert!(apply(&assembler, prefix));
            assert!(apply(&assembler, prefix));
            assert_eq!(assembler.linear_items(), before);
            assert!(!apply(&assembler, !prefix));
            assert!(!assembler.at_original_named_chain_cleanup(usize::MAX, location(4), None));
            assert!(!assembler.at_original_named_chain_cleanup(0, location(6), None));
        }
    }
}

#[test]
fn named_chain_cleanup_rejects_stale_duplicate_or_changed_original_records() {
    for record_index in 0..6 {
        for mutation in 0..8 {
            let mut assembler = fixture(true);
            let original = assembler
                .exception_owner_provenance
                .original_comparison_fallthroughs[0]
                .2[record_index];
            let position = assembler
                .linear_items()
                .iter()
                .position(|item| *item == Item::Instruction(original))
                .unwrap();
            match mutation {
                0 => {
                    items_mut(&mut assembler).remove(position);
                }
                1 => {
                    items_mut(&mut assembler).insert(0, Item::Instruction(original));
                }
                2 => {
                    items_mut(&mut assembler).remove(position);
                    items_mut(&mut assembler).insert(0, Item::Instruction(original));
                }
                _ => {
                    let mut changed = original;
                    match mutation {
                        3 => changed.local_policy.add_hidden_exception_owner_slots(1),
                        4 => changed.add_pre_fold_extra_instructions(1),
                        5 => changed.no_location_policy.preserve_location(),
                        6 => changed.set_exception_policy(ExceptionPolicy::NonePresent),
                        7 => changed.jump_policy.defer_removal(),
                        _ => unreachable!(),
                    }
                    items_mut(&mut assembler)[position] = Item::Instruction(changed);
                    // Even a forged equal receipt cannot admit a different original constructor.
                    assembler
                        .exception_owner_provenance
                        .original_comparison_fallthroughs[0]
                        .2[record_index] = changed;
                }
            }
            assert!(
                !apply(&assembler, true),
                "record {record_index}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn named_chain_cleanup_rejects_foreign_aliases_entries_and_receipts() {
    let mutations: &[fn(&mut Assembler)] = &[
        |a| a.python_version = crate::PythonVersion::Py313,
        |a| {
            a.exception_owner_provenance
                .original_comparison_fallthroughs
                .clear();
        },
        |a| {
            let record = a
                .exception_owner_provenance
                .original_comparison_fallthroughs[0];
            a.exception_owner_provenance
                .original_comparison_fallthroughs
                .push(record);
        },
        |a| {
            a.exception_owner_provenance
                .original_comparison_fallthroughs[0]
                .0 = label(0);
        },
        |a| {
            a.exception_owner_provenance
                .original_comparison_fallthroughs[0]
                .1 = label(4);
        },
        |a| {
            a.exception_owner_provenance
                .original_primitive_condition_boundaries
                .clear();
        },
        |a| {
            let record = a
                .exception_owner_provenance
                .original_primitive_condition_boundaries[0];
            a.exception_owner_provenance
                .original_primitive_condition_boundaries
                .push(record);
        },
        |a| {
            a.exception_owner_provenance
                .original_primitive_condition_boundaries[0]
                .location = location(9);
        },
        |a| {
            a.exception_owner_provenance
                .source_boundaries
                .remove(&label(3));
        },
        |a| {
            a.exception_owner_provenance
                .block_boundaries
                .remove(&label(4));
        },
        |a| {
            a.exception_owner_provenance
                .source_boundaries
                .insert(label(2));
        },
        |a| {
            a.exception_owner_provenance
                .block_boundaries
                .insert(label(2));
        },
        |a| {
            a.folded_away_condition_targets.insert(label(4));
        },
        |a| {
            a.folded_away_loop_else_targets.insert(label(3));
        },
        |a| {
            a.exception_owner_provenance
                .omitted_nop_counts
                .insert(label(2), 1);
        },
        |a| a.exception_owner_provenance.setups.push((label(4), None)),
        |a| {
            a.original_nop_targets.insert(label(3));
        },
        |a| {
            items_mut(a).push(Item::Label(label(5)));
        },
        |a| {
            items_mut(a).insert(0, Item::Label(label(4)));
        },
        |a| {
            let index = a
                .linear_items()
                .iter()
                .position(|item| *item == Item::Label(label(3)))
                .unwrap();
            items_mut(a).insert(index, Item::Label(label(5)));
        },
        |a| {
            let index = a
                .linear_items()
                .iter()
                .position(|item| *item == Item::Label(label(2)))
                .unwrap();
            items_mut(a).insert(index, Item::Label(label(5)));
        },
        |a| {
            let record = a
                .exception_owner_provenance
                .original_primitive_condition_boundaries[0];
            items_mut(a).insert(0, Item::Instruction(record));
        },
        |a| {
            items_mut(a).insert(
                1,
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    location(2),
                    Some(0),
                )),
            );
        },
        |a| {
            let index = a
                .linear_items()
                .iter()
                .position(|item| *item == Item::Label(label(3)))
                .unwrap();
            items_mut(a).insert(
                index + 1,
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    location(5),
                    Some(0),
                )),
            );
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut assembler = fixture(true);
        mutate(&mut assembler);
        assert!(!apply(&assembler, true), "mutation {index}");
    }
    for target in 1..=4 {
        for backward in [false, true] {
            let mut assembler = fixture(true);
            let operand = if backward {
                Operand::Backward(label(target))
            } else {
                Operand::Forward(label(target))
            };
            items_mut(&mut assembler).insert(
                0,
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    operand,
                    location(9),
                    Some(1),
                )),
            );
            assert!(
                !apply(&assembler, true),
                "foreign target {target}, backward {backward}"
            );
        }
    }
}

#[test]
fn named_chain_cleanup_does_not_recapture_prepared_sources() {
    let mut assembler = fixture(true);
    let graph = BlockGraph::from_items(
        assembler.linear_items().to_vec(),
        &rustc_hash::FxHashSet::default(),
    );
    let reachable = vec![true; graph.len()];
    assembler.code = UnoptimizedCode::Prepared { graph, reachable };
    assert!(!apply(&assembler, true));
}
