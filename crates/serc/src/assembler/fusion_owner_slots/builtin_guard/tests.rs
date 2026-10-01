use super::super::{
    ExceptionOwnerProvenance, ExceptionRegionPriority, LOAD_FAST, LOAD_FAST_LOAD_FAST,
};
use super::*;
use crate::assembler::SourceLocation;
use crate::target::opcodes::{JUMP_FORWARD, RETURN_VALUE};

const BUILTIN_CONSTANTS: [u32; 3] = [
    CommonConstant::BUILTIN_TUPLE.argument(),
    CommonConstant::BUILTIN_ALL.argument(),
    CommonConstant::BUILTIN_ANY.argument(),
];

struct Fixture {
    before: Vec<Item>,
    after: Vec<Item>,
    regions: Vec<ExceptionRegion>,
    provenance: ExceptionOwnerProvenance,
    preserved: FxHashSet<Label>,
}

fn label(index: u32) -> Label {
    Label::from_u32(index)
}

fn instruction(
    opcode: crate::target::Opcode,
    operand: Operand,
    line: i32,
    depth: u32,
) -> Instruction {
    Instruction::new(
        opcode,
        operand,
        SourceLocation::new(line, line, 0, 1),
        Some(depth),
    )
}

impl Fixture {
    fn new(fusions: usize) -> Self {
        Self::with_common_constant(fusions, CommonConstant::BUILTIN_TUPLE.argument())
    }

    fn with_entry_cells(fusions: usize, constant: u32, cells: usize) -> Self {
        let mut fixture = Self::with_common_constant(fusions, constant);
        let mut prefix = (0..cells)
            .map(|cell| {
                Item::Instruction(Instruction::new(
                    MAKE_CELL,
                    Operand::Value(u32::try_from(cell).unwrap()),
                    SourceLocation::NONE,
                    Some(0),
                ))
            })
            .collect::<Vec<_>>();
        prefix.push(Item::Instruction(instruction(
            RESUME,
            Operand::Value(0),
            1,
            0,
        )));
        for items in [&mut fixture.before, &mut fixture.after] {
            items.splice(..1, prefix.iter().copied());
        }
        fixture.provenance.source_boundaries.remove(&label(0));
        fixture.provenance.block_boundaries.remove(&label(0));
        fixture
    }

    fn with_common_constant(fusions: usize, constant: u32) -> Self {
        let mut before = vec![Item::Label(label(0))];
        let mut after = before.clone();
        for _ in 0..fusions {
            let first = instruction(LOAD_FAST, Operand::Value(0), 1, 1);
            let second = instruction(LOAD_FAST, Operand::Value(1), 1, 2);
            before.extend([Item::Instruction(first), Item::Instruction(second)]);
            after.push(Item::Instruction(Instruction::fused(
                LOAD_FAST_LOAD_FAST,
                Operand::Value(1),
                first,
                second,
            )));
        }
        let nop = instruction(NOP, Operand::Value(0), 2, 0);
        let mut marker = instruction(NOT_TAKEN, Operand::Value(0), 3, 1);
        marker.set_exception_policy(ExceptionPolicy::NoneDynamicConditionStrict);
        let tail = vec![
            Item::Instruction(nop),
            Item::Label(label(1)),
            Item::Instruction(instruction(LOAD_GLOBAL, Operand::Value(0), 3, 1)),
            Item::Instruction(instruction(COPY, Operand::Value(1), 3, 2)),
            Item::Instruction(instruction(
                LOAD_COMMON_CONSTANT,
                Operand::Value(constant),
                3,
                3,
            )),
            Item::Instruction(instruction(IS_OP, Operand::Value(0), 3, 2)),
            Item::Instruction(instruction(
                POP_JUMP_IF_FALSE,
                Operand::Forward(label(3)),
                3,
                1,
            )),
            Item::Instruction(marker),
            Item::Label(label(2)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0), 4, 0)),
            Item::Label(label(3)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0), 5, 0)),
            Item::Label(label(4)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0), 6, 0)),
        ];
        before.extend_from_slice(&tail);
        after.extend(tail);
        Self {
            before,
            after,
            regions: vec![ExceptionRegion {
                start: label(1),
                end: label(2),
                target: label(4),
                depth: 0,
                preserve_lasti: false,
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(label(1), label(2)),
            }],
            provenance: ExceptionOwnerProvenance {
                source_boundaries: [label(0)].into_iter().collect(),
                block_boundaries: [label(0)].into_iter().collect(),
                setups: vec![(label(1), Some(nop.location))],
                ..ExceptionOwnerProvenance::default()
            },
            preserved: FxHashSet::default(),
        }
    }

    fn view<'a>(&'a self, items: &'a [Item]) -> View<'a> {
        View::new(items, &self.regions, &self.provenance, &self.preserved).unwrap()
    }

    fn capture(&self) -> Vec<BuiltinGuardCertificate> {
        BuiltinGuardCertificate::capture_all(&self.view(&self.before), &self.view(&self.after))
    }
}

#[test]
fn authored_entries_after_terminators_require_original_boundaries() {
    for (fusions, constant) in [1, 4, 5]
        .into_iter()
        .flat_map(|fusions| (0..=5).map(move |constant| (fusions, constant)))
    {
        for boundary in 0..6 {
            let mut fixture = Fixture::with_common_constant(fusions, constant);
            match boundary {
                0 => {}
                1 => {
                    fixture.provenance.source_boundaries.remove(&label(0));
                }
                2 => {
                    fixture.provenance.block_boundaries.remove(&label(0));
                }
                3 => {
                    fixture.provenance.normal_try_copy_blocks.insert(label(0));
                }
                4 => {
                    fixture
                        .provenance
                        .retired_normal_try_copy_labels
                        .insert(label(0));
                }
                5 => fixture
                    .provenance
                    .normal_try_copy_internal_labels
                    .push(label(0)),
                _ => unreachable!(),
            }
            for items in [&mut fixture.before, &mut fixture.after] {
                items.insert(
                    0,
                    Item::Instruction(instruction(JUMP_FORWARD, Operand::Forward(label(0)), 1, 0)),
                );
            }
            let certificates = fixture.capture();
            assert_eq!(
                certificates.len(),
                usize::from(boundary == 0 && BUILTIN_CONSTANTS.contains(&constant)),
                "fusions={fusions}, constant={constant}, boundary={boundary}"
            );
        }
    }
}

#[test]
fn guard_capture_requires_the_complete_authored_instruction_metadata() {
    for (fusions, constant) in [1, 4, 5]
        .into_iter()
        .flat_map(|fusions| BUILTIN_CONSTANTS.map(|constant| (fusions, constant)))
    {
        for opcode in [
            LOAD_GLOBAL,
            COPY,
            LOAD_COMMON_CONSTANT,
            IS_OP,
            POP_JUMP_IF_FALSE,
            NOT_TAKEN,
        ] {
            for mutation in 0..13 {
                let mut fixture = Fixture::with_common_constant(fusions, constant);
                for items in [&mut fixture.before, &mut fixture.after] {
                    let instruction = items
                        .iter_mut()
                        .find_map(|item| match item {
                            Item::Instruction(instruction) if instruction.opcode == opcode => {
                                Some(instruction)
                            }
                            Item::Label(_) | Item::Instruction(_) => None,
                        })
                        .unwrap();
                    match mutation {
                        0 => instruction.local_policy.mark_force_borrowed_load(),
                        1 => instruction
                            .local_policy
                            .mark_maybe_null_comprehension_restore(),
                        2 => instruction.local_policy.mark_post_analysis_closure_load(),
                        3 => instruction.local_policy.mark_known_initialized_load(),
                        4 => instruction.local_policy.add_hidden_exception_owner_slots(1),
                        5 => instruction.no_location_policy.preserve_location(),
                        6 => instruction.fusion_policy.prevent_with_previous(),
                        7 => instruction.jump_policy.mark_interruptible(),
                        8 => instruction.jump_policy.prevent_small_exit_inlining(),
                        9 => instruction.add_pre_fold_extra_instructions(1),
                        10 => instruction.mark_retired_contains(),
                        11 => instruction.mark_late_fold_location(),
                        12 => {
                            *instruction = Instruction::new(
                                instruction.opcode,
                                instruction.operand(),
                                instruction.location,
                                Some(instruction.depth_after().unwrap() + 1),
                            );
                        }
                        _ => unreachable!(),
                    }
                }
                assert!(
                    fixture.capture().is_empty(),
                    "fusions={fusions}, constant={constant}, {opcode:?} mutation {mutation}"
                );
            }
        }
    }
}

#[test]
fn cold_transport_requires_the_complete_ordered_producer_receipts() {
    for mutation in 0..8 {
        let mut fixture = Fixture::new(1);
        let mut unrelated = fixture.regions[0];
        unrelated.start = label(3);
        unrelated.end = label(4);
        unrelated.priority = ExceptionRegionPriority::OriginalRange(label(3), label(4));
        fixture.regions.push(unrelated);
        let mut certificate = fixture.capture().pop().unwrap();
        let original = fixture.regions[0];
        let position = fixture
            .after
            .iter()
            .position(|item| *item == Item::Label(label(2)))
            .unwrap();
        fixture.after.insert(position, Item::Label(label(5)));
        fixture.regions[0].end = label(5);
        let mut receipts = vec![(original, fixture.regions[0]), (unrelated, unrelated)];
        let groups = vec![(original, 1), (unrelated, 1)];
        match mutation {
            0 => {}
            1 => fixture.regions.swap(0, 1),
            2 => receipts.swap(0, 1),
            3 => {
                fixture.regions.swap(0, 1);
                receipts.swap(0, 1);
            }
            4 => {
                receipts.pop();
            }
            5 => {
                receipts.push(receipts[1]);
            }
            6 => {
                receipts[1].0.target = label(3);
            }
            7 => {
                receipts[1].1.target = label(3);
                fixture.regions[1].target = label(3);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            certificate
                .rebind_cold_regions(&fixture.view(&fixture.after), &receipts, &groups)
                .is_some(),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn zero_output_regions_require_the_complete_census_and_a_surviving_guard_owner() {
    for zero_position in 0..3 {
        for mutation in 0..32 {
            let mut fixture = Fixture::new(1);
            let original = fixture.regions[0];
            let unrelated = ExceptionRegion {
                start: label(3),
                end: label(4),
                priority: ExceptionRegionPriority::OriginalRange(label(3), label(4)),
                ..original
            };
            let removed = ExceptionRegion {
                depth: 1,
                ..unrelated
            };
            fixture.regions.push(unrelated);
            fixture.regions.insert(zero_position, removed);
            let authored = fixture.regions.clone();
            let owner_position = usize::from(zero_position == 0);
            let unrelated_position = 2 - usize::from(zero_position == 2);
            let mut certificate = fixture.capture().pop().unwrap();
            let position = fixture
                .after
                .iter()
                .position(|item| *item == Item::Label(label(2)))
                .unwrap();
            fixture.after.insert(position, Item::Label(label(5)));
            let rebuilt = ExceptionRegion {
                end: label(5),
                ..original
            };
            fixture.regions = vec![rebuilt, unrelated];
            let mut receipts = vec![(original, rebuilt), (unrelated, unrelated)];
            let mut groups = authored
                .iter()
                .enumerate()
                .map(|(index, region)| (*region, usize::from(index != zero_position)))
                .collect::<Vec<_>>();
            match mutation {
                0 => {}
                1 => groups[zero_position].1 = 1,
                2 => groups[zero_position].1 = 2,
                3 => groups[owner_position].1 = 0,
                4 => groups[unrelated_position].1 = 0,
                5 => groups.clear(),
                6 => {
                    groups.remove(zero_position);
                }
                7 => groups.push(groups[zero_position]),
                8 => groups.swap(0, 1),
                9 => groups[zero_position].0.target = label(3),
                10 => groups[zero_position].0.depth += 1,
                11 => groups[zero_position].0.preserve_lasti = true,
                12 => groups[zero_position].0.is_scope_fallback = true,
                13 => {
                    groups[zero_position].0.priority =
                        ExceptionRegionPriority::OriginalRange(label(0), label(4));
                }
                14 => groups[zero_position].0.start = label(0),
                15 => groups[zero_position].0.end = label(2),
                16 => receipts.clear(),
                17 => receipts.reverse(),
                18 => {
                    receipts.pop();
                }
                19 => receipts.push(receipts[0]),
                20 => receipts[0].0.start = label(0),
                21 => {
                    receipts[1].1.depth = 2;
                    fixture.regions[1] = receipts[1].1;
                }
                22 => fixture.regions.reverse(),
                23 => {
                    fixture.regions.reverse();
                    receipts.reverse();
                }
                24 | 25 => {
                    fixture.regions.remove(0);
                    receipts.remove(0);
                    groups[owner_position].1 = 0;
                    if mutation == 25 {
                        fixture.regions[0].start = original.start;
                        fixture.regions[0].end = original.end;
                        receipts[0].1 = fixture.regions[0];
                    }
                }
                26 => {
                    fixture.regions.clear();
                    receipts.clear();
                    for (_, count) in &mut groups {
                        *count = 0;
                    }
                }
                27 => groups[owner_position].0.target = label(3),
                28 => {
                    fixture.regions.insert(0, rebuilt);
                    receipts.insert(0, (original, rebuilt));
                    groups[owner_position].1 = 2;
                }
                29 => {
                    groups[zero_position].0 = unrelated;
                }
                30 | 31 => {
                    fixture.regions.pop();
                    receipts.pop();
                    groups[unrelated_position].1 = usize::from(mutation == 31);
                }
                _ => unreachable!(),
            }
            if matches!(mutation, 24..=26 | 28) {
                assert!(
                    cold_region_replacements(
                        &authored,
                        &fixture.regions,
                        &receipts,
                        Some(&groups),
                    )
                    .is_some(),
                    "owner rejection must follow authentic transport: {mutation}"
                );
            }
            assert_eq!(
                certificate
                    .rebind_cold_regions(&fixture.view(&fixture.after), &receipts, &groups)
                    .is_some(),
                matches!(mutation, 0 | 30),
                "zero position {zero_position}, mutation {mutation}"
            );
            assert!(same_manager_region(&certificate.authored_region, &original));
            assert_eq!(certificate.authored_regions.len(), authored.len());
            assert!(
                certificate
                    .authored_regions
                    .iter()
                    .zip(&authored)
                    .all(|(actual, expected)| same_manager_region(actual, expected))
            );
            if matches!(mutation, 0 | 30) {
                assert!(certificate.cold_transported);
                assert!(
                    certificate
                        .valid(&fixture.view(&fixture.after), false)
                        .is_some()
                );
                let previous = fixture.regions.clone();
                fixture.regions[0] = original;
                let next_receipts = previous
                    .iter()
                    .copied()
                    .zip(fixture.regions.iter().copied())
                    .collect::<Vec<_>>();
                let next_groups = previous
                    .iter()
                    .map(|region| (*region, 1))
                    .collect::<Vec<_>>();
                assert!(
                    cold_region_replacements(
                        &previous,
                        &fixture.regions,
                        &next_receipts,
                        Some(&next_groups),
                    )
                    .is_some()
                );
                assert!(
                    certificate
                        .rebind_cold_regions(
                            &fixture.view(&fixture.after),
                            &next_receipts,
                            &next_groups,
                        )
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn only_named_builtins_with_one_to_five_observed_fusions_supply_guard_cells() {
    assert_eq!(BUILTIN_CONSTANTS, [2, 3, 4]);
    for constant in
        (0..=u32::from(u8::MAX)).chain([u32::from(u8::MAX) + 1, u32::from(u16::MAX), u32::MAX])
    {
        for fusions in 0..=7 {
            let fixture = Fixture::with_common_constant(fusions, constant);
            let certificates = fixture.capture();
            assert_eq!(
                certificates.len(),
                usize::from(BUILTIN_CONSTANTS.contains(&constant) && (1..=5).contains(&fusions)),
                "fusions={fusions}, constant={constant}"
            );
            for certificate in certificates {
                assert!(
                    certificate
                        .valid(&fixture.view(&fixture.after), false)
                        .is_some()
                );
            }
        }
    }
}

#[test]
fn original_entry_cells_consume_only_observed_retired_slots() {
    for constant in BUILTIN_CONSTANTS {
        for cells in 0..=6 {
            for fusions in 0_usize..=12 {
                let fixture = Fixture::with_entry_cells(fusions, constant, cells);
                let certificates = fixture.capture();
                let expected = fusions
                    .checked_sub(cells)
                    .is_some_and(|slots| (1..=5).contains(&slots));
                assert_eq!(
                    certificates.len(),
                    usize::from(expected),
                    "constant={constant}, cells={cells}, fusions={fusions}"
                );
                for certificate in certificates {
                    assert!(
                        certificate
                            .valid(&fixture.view(&fixture.after), false)
                            .is_some()
                    );
                }
            }
        }
    }
}

#[test]
fn entry_cell_accounting_requires_the_complete_ordered_prefix() {
    for constant in BUILTIN_CONSTANTS {
        for mutation in 0..13 {
            let mut fixture = Fixture::with_entry_cells(3, constant, 2);
            let certificate = fixture.capture().pop().unwrap();
            for items in [&mut fixture.before, &mut fixture.after] {
                let index = if mutation >= 9 { 2 } else { 0 };
                let Item::Instruction(instruction) = &mut items[index] else {
                    unreachable!();
                };
                match mutation {
                    0 => instruction.fusion_policy.prevent_with_previous(),
                    1 => instruction.location = SourceLocation::new(1, 1, 0, 1),
                    2 => {
                        *instruction = Instruction::new(
                            MAKE_CELL,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(1),
                        );
                    }
                    3 => instruction.local_policy.mark_known_initialized_load(),
                    4 => {
                        *instruction = Instruction::new(
                            MAKE_CELL,
                            Operand::Value(1),
                            SourceLocation::NONE,
                            Some(0),
                        );
                    }
                    5 => {
                        *instruction = Instruction::new(
                            MAKE_CELL,
                            Operand::Value(2),
                            SourceLocation::NONE,
                            Some(0),
                        );
                    }
                    6 => {
                        *instruction = Instruction::new(
                            MAKE_CELL,
                            Operand::Forward(label(3)),
                            SourceLocation::NONE,
                            Some(0),
                        );
                    }
                    7 => {
                        *instruction = Instruction::new(
                            COPY_FREE_VARS,
                            Operand::Value(1),
                            SourceLocation::NONE,
                            Some(0),
                        );
                    }
                    8 => {
                        *instruction = Instruction::new(
                            RETURN_GENERATOR,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            Some(0),
                        );
                    }
                    9 => {
                        *instruction = Instruction::new(
                            RESUME,
                            Operand::Value(1),
                            instruction.location,
                            Some(0),
                        );
                    }
                    10 => {
                        *instruction =
                            Instruction::new(NOP, Operand::Value(0), instruction.location, Some(0));
                    }
                    11 => {
                        *instruction = Instruction::new(
                            RESUME,
                            Operand::Value(0),
                            instruction.location,
                            Some(1),
                        );
                    }
                    12 => instruction.no_location_policy.preserve_location(),
                    _ => unreachable!(),
                }
            }
            assert!(
                fixture.capture().is_empty(),
                "constant={constant}, mutation={mutation}"
            );
            assert!(
                certificate
                    .valid(&fixture.view(&fixture.after), false)
                    .is_none()
            );
        }
    }
}

#[test]
fn entry_cell_prefix_ignores_only_unreferenced_labels() {
    for index in 0..=2 {
        for mutation in 0..4 {
            let mut fixture = Fixture::with_entry_cells(3, BUILTIN_CONSTANTS[0], 2);
            let certificate = fixture.capture().pop().unwrap();
            fixture.after.insert(index, Item::Label(label(20)));
            match mutation {
                0 => {}
                1 => {
                    fixture.provenance.source_boundaries.insert(label(20));
                }
                2 => {
                    fixture.provenance.block_boundaries.insert(label(20));
                }
                3 => {
                    fixture.preserved.insert(label(20));
                }
                _ => unreachable!(),
            }
            assert_eq!(
                certificate
                    .valid(&fixture.view(&fixture.after), false)
                    .is_some(),
                mutation == 0,
                "index={index}, mutation={mutation}"
            );
        }
    }
}

#[test]
fn changed_guard_prefix_owner_and_boundaries_expire_the_certificate() {
    for (fusions, constant) in [1, 4, 5]
        .into_iter()
        .flat_map(|fusions| BUILTIN_CONSTANTS.map(|constant| (fusions, constant)))
    {
        for mutation in 0..15 {
            let mut fixture = Fixture::with_common_constant(fusions, constant);
            let certificate = fixture.capture().pop().unwrap();
            match mutation {
                0 => fixture.regions[0].depth = 1,
                1 => fixture.regions[0].preserve_lasti = true,
                2 => fixture.regions[0].target = label(3),
                3 => fixture.regions[0].is_scope_fallback = true,
                4 => fixture.regions[0].end = label(3),
                5 => {
                    fixture.regions[0].priority =
                        ExceptionRegionPriority::OriginalRange(label(0), label(2));
                }
                6 => fixture.provenance.setups.clear(),
                7 => {
                    fixture.provenance.source_boundaries.insert(label(1));
                }
                8 => {
                    fixture.provenance.block_boundaries.insert(label(1));
                }
                9 => fixture.after.push(Item::Instruction(instruction(
                    JUMP_FORWARD,
                    Operand::Forward(label(1)),
                    9,
                    0,
                ))),
                10 => fixture.after.retain(|item| *item != Item::Label(label(4))),
                _ => {
                    let target = match mutation {
                        11 => LOAD_FAST_LOAD_FAST,
                        12 => LOAD_COMMON_CONSTANT,
                        13 => POP_JUMP_IF_FALSE,
                        _ => NOT_TAKEN,
                    };
                    let item = fixture
                        .after
                        .iter_mut()
                        .find_map(|item| match item {
                            Item::Instruction(instruction) if instruction.opcode == target => {
                                Some(instruction)
                            }
                            Item::Label(_) | Item::Instruction(_) => None,
                        })
                        .unwrap();
                    item.location.line += 1;
                }
            }
            assert!(
                certificate
                    .valid(&fixture.view(&fixture.after), false)
                    .is_none(),
                "fusions={fusions}, constant={constant}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn only_exact_cold_receipts_transfer_the_unchanged_guard_owner() {
    for mutation in 0..7 {
        let mut fixture = Fixture::new(1);
        let mut certificate = fixture.capture().pop().unwrap();
        let original = fixture.regions[0];
        let position = fixture
            .after
            .iter()
            .position(|item| *item == Item::Label(label(2)))
            .unwrap();
        fixture.after.insert(position, Item::Label(label(5)));
        fixture.regions[0].end = label(5);
        let mut receipts = vec![(original, fixture.regions[0])];
        let groups = vec![(original, 1)];
        match mutation {
            0 => {}
            1 => receipts.clear(),
            2 => receipts.push(receipts[0]),
            3 => receipts[0].0.target = label(3),
            4 => {
                receipts[0].1.depth = 1;
                fixture.regions[0].depth = 1;
            }
            5 => receipts[0].1.end = label(3),
            6 => fixture.regions.push(fixture.regions[0]),
            _ => unreachable!(),
        }
        assert_eq!(
            certificate
                .rebind_cold_regions(&fixture.view(&fixture.after), &receipts, &groups)
                .is_some(),
            mutation == 0
        );
        assert!(same_manager_region(&certificate.authored_region, &original));
        assert!(same_manager_region(
            &certificate.authored_regions[0],
            &original
        ));
    }
}
