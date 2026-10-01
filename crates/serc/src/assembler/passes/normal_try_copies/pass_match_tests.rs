use super::*;
use crate::PythonVersion;
use crate::assembler::ir::JumpPolicy;
use crate::assembler::{Assembler, EmittedJump, ExceptionPolicy, UnoptimizedCode};
use crate::target::opcodes::{
    CALL, LOAD_CONST, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_TOP, RETURN_VALUE, UNPACK_SEQUENCE,
};

const START: Label = Label::from_u32(0);
const DONOR: Label = Label::from_u32(1);
const ENTRY: Label = Label::from_u32(2);
const HANDLER: Label = Label::from_u32(3);
const TARGET: Label = Label::from_u32(4);
const CLASS_LAST: Label = Label::from_u32(5);
const SPARE: Label = Label::from_u32(6);
const PASS: SourceLocation = SourceLocation::new(8, 8, 8, 12);
const CALL_LOCATION: SourceLocation = SourceLocation::new(10, 10, 8, 17);

fn instruction(opcode: crate::target::Opcode, operand: Operand, line: i32, depth: u32) -> Item {
    Item::Instruction(Instruction::new(
        opcode,
        operand,
        SourceLocation::new(line, line, 8, 17),
        Some(depth),
    ))
}

fn items(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!();
    };
    items
}

fn record(items: &mut [Item], index: usize) -> &mut Instruction {
    let Item::Instruction(instruction) = &mut items[index] else {
        unreachable!();
    };
    instruction
}

fn source(version: PythonVersion, shape: usize) -> (Assembler, EmittedJump) {
    let mut assembler = Assembler {
        python_version: version,
        next_label: 7,
        code: UnoptimizedCode::Linear(vec![
            Item::Label(START),
            instruction(POP_JUMP_IF_FALSE, Operand::Forward(DONOR), 7, 1),
            instruction(NOT_TAKEN, Operand::Value(0), 7, 1),
        ]),
        ..Assembler::default()
    };
    if shape >= 1 {
        items(&mut assembler).push(instruction(UNPACK_SEQUENCE, Operand::Value(0), 7, 1));
    }
    if shape == 2 {
        items(&mut assembler).push(instruction(POP_TOP, Operand::Value(0), 7, 0));
    }
    assembler.set_location(PASS);
    let emitted =
        assembler.emit_forward_with_depth(JUMP_FORWARD, ENTRY, 0, ExceptionPolicy::default());
    assembler.record_original_match_success(emitted, false);
    let jump = assembler.emitted_jump_mut(emitted, "test Pass success");
    if shape == 2 {
        jump.jump_policy.retain_redundant_nop_after_threading();
    } else {
        jump.jump_policy.retain_nop();
    }
    (assembler, emitted)
}

fn finish(mut assembler: Assembler, class_donor: bool) -> Assembler {
    items(&mut assembler).push(Item::Label(DONOR));
    if class_donor {
        items(&mut assembler).extend([
            instruction(POP_TOP, Operand::Value(0), 10, 1),
            Item::Label(CLASS_LAST),
            instruction(POP_TOP, Operand::Value(0), 10, 0),
        ]);
    } else {
        items(&mut assembler).extend([
            instruction(CALL, Operand::Value(0), 10, 1),
            instruction(POP_TOP, Operand::Value(0), 10, 0),
        ]);
    }
    items(&mut assembler).extend([
        Item::Label(ENTRY),
        instruction(JUMP_FORWARD, Operand::Forward(TARGET), 10, 0),
    ]);
    assembler.record_normal_try_exit(ENTRY, TARGET, (!class_donor).then_some(CALL_LOCATION));
    if class_donor {
        assembler.record_normal_try_class_pattern_fallthrough(ENTRY, TARGET, CALL_LOCATION);
        assert!(
            assembler
                .exception_owner_provenance
                .normal_try_class_pattern_donor_labels
                .contains_key(&ENTRY)
        );
    }
    assert!(
        assembler
            .exception_owner_provenance
            .normal_try_fallthrough_donors
            .contains_key(&ENTRY)
    );
    assembler
        .exception_owner_provenance
        .original_match_join_boundaries
        .insert(ENTRY);
    assembler.add_exception_region(START, ENTRY, HANDLER, 0, false);
    items(&mut assembler).extend([
        Item::Label(HANDLER),
        instruction(RETURN_VALUE, Operand::Value(0), 11, 0),
        Item::Label(TARGET),
    ]);
    for _ in 0..5 {
        items(&mut assembler).push(instruction(LOAD_CONST, Operand::Value(0), 12, 1));
    }
    items(&mut assembler).extend([
        instruction(RETURN_VALUE, Operand::Value(0), 12, 0),
        Item::Label(SPARE),
        instruction(RETURN_VALUE, Operand::Value(0), 13, 0),
    ]);
    assembler
}

fn fixture(version: PythonVersion, shape: usize, class_donor: bool) -> Assembler {
    let (mut assembler, emitted) = source(version, shape);
    assembler.record_original_pass_match_success(emitted, PASS);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .original_pass_match_successes
            .len(),
        1
    );
    finish(assembler, class_donor)
}

fn assert_selected(assembler: Assembler, selected: bool, context: &str) {
    let mut pipeline = assembler.into_optimization_pipeline();
    let before = pipeline.graph.copy_items();
    pipeline.prepare_conditional_normal_try_joins();
    let block = pipeline.graph.label_block(ENTRY).unwrap();
    let converted = pipeline.graph[block].items().iter().filter(|item| {
        matches!(item, Item::Instruction(instruction) if instruction.fusion_policy.is_converted_pop_block())
    }).count();
    assert_eq!(converted, usize::from(selected), "{context}");
    assert_eq!(
        pipeline
            .exception_owner_provenance
            .normal_try_copy_pop_ranges
            .len(),
        usize::from(selected),
        "{context}"
    );
    if !selected {
        assert_eq!(pipeline.graph.copy_items(), before, "{context}");
    }
}

#[test]
fn pass_match_writer_requires_an_original_uncompacted_source() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for shape in 0..3 {
            let (mut assembler, emitted) = source(version, shape);
            let original = assembler.linear_items().to_vec();
            assembler.record_original_pass_match_success(emitted, PASS);
            let origins = &assembler
                .exception_owner_provenance
                .original_pass_match_successes;
            if version < PythonVersion::Py314 {
                assert!(origins.is_empty());
            } else {
                let Item::Instruction(condition) = original[1] else {
                    unreachable!()
                };
                assert_eq!(origins.len(), 1);
                assert_eq!(origins[0].0, condition);
                assert!(
                    original[2..].iter().copied().eq(origins[0]
                        .1
                        .iter()
                        .copied()
                        .map(Item::Instruction))
                );
            }
        }
    }
    for shape in 0..3 {
        for mutation in 0..12 {
            let (mut assembler, emitted) = source(PythonVersion::Py314, shape);
            let jump = usize::try_from(emitted.instruction().item_index).unwrap();
            let mut location = PASS;
            match mutation {
                0 => location = SourceLocation::NONE,
                1 => record(items(&mut assembler), jump).location = SourceLocation::NONE,
                2 => {
                    let instruction = record(items(&mut assembler), jump);
                    instruction.set_operand(Operand::Forward(SPARE));
                    instruction.set_operand(Operand::Forward(ENTRY));
                }
                3 => record(items(&mut assembler), jump).add_pre_fold_extra_instructions(1),
                4 => record(items(&mut assembler), jump)
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                5 => record(items(&mut assembler), jump)
                    .jump_policy
                    .mark_borrow_empty_block(),
                6 => record(items(&mut assembler), jump)
                    .no_location_policy
                    .mark_artificial_eval_break(),
                7 => record(items(&mut assembler), jump).jump_policy = JumpPolicy::default(),
                8 => record(items(&mut assembler), 2).opcode = NOP,
                9 => record(items(&mut assembler), 1).opcode = LOAD_CONST,
                10 => items(&mut assembler).insert(2, Item::Label(SPARE)),
                11 => items(&mut assembler).insert(2, instruction(NOP, Operand::Value(0), 8, 0)),
                _ => unreachable!(),
            }
            // Insertions move the original emitted index; restore its valid handle.
            let emitted = if mutation >= 10 {
                let mut instruction = emitted.instruction();
                instruction.item_index += 1;
                EmittedJump(instruction)
            } else {
                emitted
            };
            assembler.record_original_pass_match_success(emitted, location);
            assert!(
                assembler
                    .exception_owner_provenance
                    .original_pass_match_successes
                    .is_empty(),
                "shape={shape} mutation={mutation}"
            );
        }
    }
}

#[test]
fn pass_match_reader_authenticates_every_original_record() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for shape in 0..3 {
            assert_selected(
                fixture(version, shape, false),
                true,
                "original wildcard donor",
            );
            for index in 1..=3 + shape {
                for mutation in 0..10 {
                    let mut assembler = fixture(version, shape, false);
                    let instruction = record(items(&mut assembler), index);
                    match mutation {
                        0 => instruction.location.column += 1,
                        1 => {
                            let mut changed = Instruction::new(
                                instruction.opcode,
                                instruction.operand(),
                                instruction.location,
                                Some(instruction.depth_after().unwrap() + 1),
                            );
                            changed.jump_policy = instruction.jump_policy;
                            *instruction = changed;
                        }
                        2 => instruction.set_exception_policy(ExceptionPolicy::NonePresent),
                        3 => instruction.set_exception_policy(ExceptionPolicy::AlwaysUnchanged),
                        4 => instruction.add_pre_fold_extra_instructions(1),
                        5 => instruction.local_policy.add_hidden_exception_owner_slots(1),
                        6 => instruction.no_location_policy.preserve_location(),
                        7 => instruction.fusion_policy.mark_converted_pop_block(),
                        8 => instruction.set_operand(match instruction.operand() {
                            Operand::Value(value) => Operand::Value(value + 1),
                            _ => Operand::Forward(SPARE),
                        }),
                        9 => instruction.jump_policy.prevent_threading_target(),
                        _ => unreachable!(),
                    }
                    assert_selected(
                        assembler,
                        false,
                        &format!("{version:?} shape={shape} index={index} mutation={mutation}"),
                    );
                }
            }
        }
    }
}

#[test]
fn pass_match_reader_requires_unique_physical_wildcard_predecessors() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for shape in 0..3 {
            for mutation in 0..13 {
                let mut assembler = fixture(version, shape, false);
                let jump = 3 + shape;
                match mutation {
                    0 => assembler
                        .exception_owner_provenance
                        .original_pass_match_successes
                        .clear(),
                    1 => {
                        let origin = assembler
                            .exception_owner_provenance
                            .original_pass_match_successes[0]
                            .clone();
                        assembler
                            .exception_owner_provenance
                            .original_pass_match_successes
                            .push(origin);
                    }
                    2 => {
                        assembler
                            .exception_owner_provenance
                            .original_match_join_boundaries
                            .remove(&ENTRY);
                    }
                    3 => assembler
                        .exception_owner_provenance
                        .normal_try_fallthrough_donors
                        .clear(),
                    4 => {
                        let condition = items(&mut assembler)[1];
                        items(&mut assembler).push(condition);
                    }
                    5 => {
                        let success = items(&mut assembler)[jump];
                        items(&mut assembler).push(success);
                    }
                    6 => {
                        let copy = items(&mut assembler)[1..=jump].to_vec();
                        items(&mut assembler).extend(copy);
                    }
                    7 => {
                        assembler.next_label = 8;
                        items(&mut assembler).insert(2, Item::Label(Label::from_u32(7)));
                    }
                    8 => items(&mut assembler).insert(2, instruction(NOP, Operand::Value(0), 8, 0)),
                    9 => {
                        items(&mut assembler).remove(2);
                    }
                    10 => items(&mut assembler).swap(1, 2),
                    11 => assembler.add_exception_region(START, ENTRY, DONOR, 0, false),
                    12 => {
                        assembler.original_nop_targets.insert(ENTRY);
                    }
                    _ => unreachable!(),
                }
                assert_selected(
                    assembler,
                    false,
                    &format!("{version:?} shape={shape} topology={mutation}"),
                );
            }
            assert_selected(
                fixture(version, shape, true),
                false,
                "class failure is not a wildcard donor",
            );
            let (mut ordinary, emitted) = source(version, shape);
            ordinary
                .emitted_jump_mut(emitted, "ordinary class success")
                .jump_policy = JumpPolicy::default();
            assert_selected(
                finish(ordinary, true),
                true,
                "existing class donor route remains available",
            );
        }
    }
}
