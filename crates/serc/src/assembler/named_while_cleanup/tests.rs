use super::*;
use crate::assembler::Label;
use crate::assembler::UnoptimizedCode;
use crate::assembler::exception_owners::SourceNoneCondition;

fn label(value: u32) -> Label {
    Label::from_u32(value)
}

fn location(line: i32) -> SourceLocation {
    SourceLocation::new(line, line, 4, 12)
}

fn fixture() -> Assembler {
    let mut condition = Instruction::new(
        POP_JUMP_IF_NONE,
        Operand::Forward(label(3)),
        location(2),
        Some(1),
    );
    condition.local_policy.add_hidden_exception_owner_slots(3);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(label(4)),
        location(4),
        Some(1),
    );
    jump.fusion_policy.mark_authored_break();
    jump.jump_policy.prevent_threading_target();
    jump.jump_policy.retain_nop_for_direct_target();
    jump.add_pre_fold_extra_instructions(1);
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Label(label(0)),
        Item::Instruction(Instruction::new(
            LOAD_FAST,
            Operand::Value(0),
            location(2),
            Some(2),
        )),
        Item::Instruction(condition),
        Item::Label(label(1)),
        Item::Instruction(jump),
        Item::Label(label(2)),
        Item::Instruction(Instruction::new(
            JUMP_BACKWARD,
            Operand::Backward(label(0)),
            location(5),
            Some(1),
        )),
        Item::Label(label(3)),
        Item::Label(label(4)),
    ]);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 5;
    let provenance = &mut assembler.exception_owner_provenance;
    for entry in [label(0), label(3)] {
        provenance.source_boundaries.insert(entry);
        provenance.block_boundaries.insert(entry);
    }
    provenance.source_none_conditions.push(SourceNoneCondition {
        entry: label(0),
        labels: vec![label(0)],
        instruction: condition,
        entry_prefix: Some(vec![Item::Label(label(0))]),
    });
    assembler
}

fn apply(assembler: &mut Assembler) -> bool {
    assembler.defer_original_named_while_none_cleanup(0, 0, location(2), location(2), location(4))
}

fn instruction(assembler: &mut Assembler, index: usize) -> &mut Instruction {
    let Item::Instruction(instruction) = &mut assembler.linear_items_mut()[index] else {
        panic!("instruction fixture index");
    };
    instruction
}

fn items(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items
}

#[test]
fn named_while_none_cleanup_changes_only_the_authored_break_policy() {
    for version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
        let mut assembler = fixture();
        assembler.python_version = version;
        let mut expected = assembler.linear_items().to_vec();
        let Item::Instruction(jump) = &mut expected[4] else {
            unreachable!()
        };
        jump.jump_policy.defer_removal();
        assert!(apply(&mut assembler));
        assert_eq!(assembler.linear_items(), expected);
        assert!(!apply(&mut assembler));
    }
}

#[test]
fn named_while_none_cleanup_rejects_changed_records_and_aliases() {
    let mutations: &[fn(&mut Assembler)] = &[
        |a| a.python_version = crate::PythonVersion::Py313,
        |a| {
            a.exception_owner_provenance.source_none_conditions.clear();
        },
        |a| {
            let origin = &a.exception_owner_provenance.source_none_conditions[0];
            let duplicate = SourceNoneCondition {
                entry: origin.entry,
                labels: origin.labels.clone(),
                instruction: origin.instruction,
                entry_prefix: origin.entry_prefix.clone(),
            };
            a.exception_owner_provenance
                .source_none_conditions
                .push(duplicate);
        },
        |a| a.exception_owner_provenance.source_none_conditions[0].entry = label(1),
        |a| {
            a.exception_owner_provenance.source_none_conditions[0]
                .labels
                .push(label(1));
        },
        |a| a.exception_owner_provenance.source_none_conditions[0].entry_prefix = None,
        |a| {
            a.exception_owner_provenance.source_none_conditions[0]
                .instruction
                .location = location(3);
        },
        |a| {
            a.exception_owner_provenance
                .source_boundaries
                .remove(&label(0));
        },
        |a| {
            a.exception_owner_provenance
                .block_boundaries
                .remove(&label(3));
        },
        |a| {
            a.exception_owner_provenance
                .source_boundaries
                .insert(label(4));
        },
        |a| {
            a.folded_away_condition_targets.insert(label(3));
        },
        |a| {
            a.folded_away_loop_else_targets.insert(label(4));
        },
        |a| {
            a.exception_owner_provenance
                .omitted_nop_counts
                .insert(label(3), 1);
        },
        |a| a.exception_owner_provenance.setups.push((label(4), None)),
        |a| {
            items(a).insert(0, Item::Label(label(0)));
        },
        |a| {
            items(a).insert(0, Item::Label(label(5)));
        },
        |a| {
            items(a).push(Item::Label(label(5)));
        },
        |a| {
            a.linear_items_mut().swap(7, 8);
        },
        |a| instruction(a, 1).local_policy.mark_known_initialized_load(),
        |a| instruction(a, 1).no_location_policy.preserve_location(),
        |a| {
            instruction(a, 2)
                .local_policy
                .add_hidden_exception_owner_slots(1);
        },
        |a| instruction(a, 2).add_pre_fold_extra_instructions(1),
        |a| instruction(a, 2).no_location_policy.preserve_location(),
        |a| {
            instruction(a, 4)
                .local_policy
                .add_hidden_exception_owner_slots(1);
        },
        |a| instruction(a, 4).no_location_policy.preserve_location(),
        |a| instruction(a, 4).add_pre_fold_extra_instructions(1),
        |a| instruction(a, 4).jump_policy.defer_removal(),
        |a| instruction(a, 6).jump_policy.defer_removal(),
        |a| instruction(a, 2).set_operand(Operand::Backward(label(3))),
        |a| instruction(a, 4).set_operand(Operand::Forward(label(3))),
        |a| instruction(a, 6).set_operand(Operand::Backward(label(1))),
        |a| {
            let previous = *instruction(a, 6);
            *instruction(a, 6) = Instruction::new(
                previous.opcode,
                previous.operand(),
                previous.location,
                Some(2),
            );
        },
        |a| {
            instruction(a, 4)
                .set_exception_policy(crate::assembler::ExceptionPolicy::EXCLUDE_ALWAYS);
        },
        |a| {
            a.exception_regions.push(crate::assembler::ExceptionRegion {
                start: label(0),
                end: label(2),
                target: label(3),
                depth: 1,
                preserve_lasti: true,
                is_scope_fallback: false,
                priority: crate::assembler::ExceptionRegionPriority::OriginalRange(
                    label(0),
                    label(2),
                ),
            });
        },
        |a| {
            let duplicate = a.linear_items()[4];
            items(a).insert(0, duplicate);
        },
        |a| {
            let duplicate = a.linear_items()[2];
            items(a).insert(4, duplicate);
        },
        |a| {
            let duplicate = a.linear_items()[4];
            items(a).insert(4, duplicate);
        },
        |a| {
            a.linear_items_mut().swap(2, 4);
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut assembler = fixture();
        mutate(&mut assembler);
        let original = assembler.linear_items().to_vec();
        assert!(!apply(&mut assembler), "mutation {index}");
        assert_eq!(assembler.linear_items(), original, "mutation {index}");
    }
}

#[test]
fn named_while_none_cleanup_requires_the_same_source_interval_and_locations() {
    for (start, local, name, condition, authored_break) in [
        (1, 0, location(2), location(2), location(4)),
        (0, 1, location(2), location(2), location(4)),
        (0, 0, location(3), location(2), location(4)),
        (0, 0, location(2), location(3), location(4)),
        (0, 0, location(2), location(2), location(3)),
    ] {
        let mut assembler = fixture();
        let original = assembler.linear_items().to_vec();
        assert!(!assembler.defer_original_named_while_none_cleanup(
            start,
            local,
            name,
            condition,
            authored_break
        ));
        assert_eq!(assembler.linear_items(), original);
    }
}
