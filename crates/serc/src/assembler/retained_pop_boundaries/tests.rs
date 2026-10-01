use super::*;
use crate::assembler::{BorrowFallthrough, ExceptionPolicy, ExceptionRegionPriority, Instruction};
use crate::target::opcodes::{LOAD_CONST, POP_JUMP_IF_TRUE, RETURN_VALUE};

struct Fixture {
    items: Vec<Item>,
    original: Vec<ExceptionRegion>,
    rebuilt: Vec<ExceptionRegion>,
    provenance: ExceptionOwnerProvenance,
    preserved: FxHashSet<Label>,
    empty: bool,
}

fn label(value: u32) -> Label {
    Label::from_u32(value)
}

fn fixture(empty: bool, hidden: u16) -> Fixture {
    let plain = |opcode, operand| {
        Instruction::new(
            opcode,
            operand,
            SourceLocation {
                line: 4,
                end_line: 4,
                column: 0,
                end_column: 4,
            },
            Some(0),
        )
    };
    let mut pop = plain(NOP, Operand::Value(0));
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    pop.fusion_policy.mark_converted_pop_block();
    pop.local_policy.add_hidden_exception_owner_slots(hidden);
    let items = vec![
        Item::Label(label(0)),
        Item::Instruction(plain(LOAD_CONST, Operand::Value(0))),
        Item::Label(label(1)),
        Item::Instruction(plain(LOAD_FAST, Operand::Value(0))),
        Item::Instruction(plain(POP_JUMP_IF_TRUE, Operand::Forward(label(2)))),
        Item::Label(label(2)),
        Item::Label(label(3)),
        Item::Instruction(pop),
        Item::Label(label(4)),
        Item::Instruction(plain(LOAD_CONST, Operand::Value(0))),
        Item::Instruction(plain(RETURN_VALUE, Operand::Value(0))),
        Item::Label(label(5)),
        Item::Instruction(plain(RETURN_VALUE, Operand::Value(0))),
        Item::Label(label(6)),
        Item::Instruction(plain(RETURN_VALUE, Operand::Value(0))),
    ];
    let ordinary = ExceptionRegion {
        start: label(if empty { 3 } else { 1 }),
        end: label(3),
        target: label(5),
        depth: 0,
        preserve_lasti: false,
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::Resolved { start: 2, end: 6 },
    };
    let outer = ExceptionRegion {
        start: label(0),
        end: label(5),
        target: label(6),
        depth: 1,
        preserve_lasti: true,
        ..ordinary
    };
    let mut provenance = ExceptionOwnerProvenance::default();
    if empty {
        provenance.setups.push((label(2), Some(pop.location)));
    } else {
        provenance.source_boundaries.insert(label(2));
        provenance.block_boundaries.insert(label(2));
    }
    Fixture {
        items,
        original: vec![ordinary, outer],
        rebuilt: if empty {
            vec![outer]
        } else {
            vec![
                ExceptionRegion {
                    end: label(2),
                    ..ordinary
                },
                outer,
            ]
        },
        provenance,
        preserved: FxHashSet::default(),
        empty,
    }
}

impl Fixture {
    fn capture(&self) -> ColdRewrite {
        ColdRewrite::capture(
            &self.items,
            &self.original,
            &self.provenance,
            &self.preserved,
        )
        .unwrap()
    }

    fn sealed(&self) -> RetainedPopBoundaries {
        let mut rewrite = self.capture();
        for (index, original) in self.original.iter().enumerate() {
            let output = if self.empty && index == 0 {
                &[][..]
            } else {
                let output = index - usize::from(self.empty);
                &self.rebuilt[output..=output]
            };
            rewrite.group(index, *original, output);
        }
        rewrite.finish(
            &self.items,
            &self.rebuilt,
            &self.provenance,
            &self.preserved,
        )
    }

    fn labels(&self, witness: &RetainedPopBoundaries) -> FxHashSet<Label> {
        witness.valid_labels(
            &self.items,
            &self.rebuilt,
            &self.provenance,
            &self.preserved,
        )
    }
}

#[test]
fn cold_pop_end_witness_requires_full_records_roles_and_order() {
    for empty in [false, true] {
        for hidden in [0, 1] {
            for mode in 0..51 {
                let mut f = fixture(empty, hidden);
                let witness = f.sealed();
                let Item::Instruction(pop) = &mut f.items[7] else {
                    unreachable!();
                };
                match mode {
                    0 => {}
                    1 => pop.location.column += 1,
                    2 => pop.set_operand(Operand::Value(1)),
                    3 => pop.opcode = LOAD_CONST,
                    4 => pop.set_exception_policy(ExceptionPolicy::NoneUnchanged),
                    5 => pop.local_policy.add_hidden_exception_owner_slots(1),
                    6 => pop.add_pre_fold_extra_instructions(1),
                    7 => pop
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                    8 => pop.jump_policy.prevent_small_exit_inlining(),
                    9 => f.items.swap(5, 6),
                    10 => {
                        f.items.remove(6);
                    }
                    11 => f.items.insert(6, Item::Label(label(3))),
                    12 => f.items.insert(6, Item::Label(label(20))),
                    13 => f.items.insert(
                        6,
                        Item::Instruction(Instruction::new(
                            NOP,
                            Operand::Value(0),
                            SourceLocation::NONE,
                            None,
                        )),
                    ),
                    14 => {
                        f.provenance.source_boundaries.insert(label(3));
                    }
                    15 => {
                        f.provenance.block_boundaries.insert(label(3));
                    }
                    16 => {
                        f.preserved.insert(label(3));
                    }
                    17 => {
                        f.provenance.fusion_boundaries.insert(label(3));
                    }
                    18 => {
                        f.provenance.retired_normal_try_copy_labels.insert(label(3));
                    }
                    19 => {
                        f.provenance.normal_try_copy_blocks.insert(label(3));
                    }
                    20 => {
                        f.provenance.normal_try_copy_inline_labels.insert(label(3));
                    }
                    21 => {
                        f.provenance.normal_try_copy_internal_labels.push(label(3));
                    }
                    22 => {
                        f.provenance
                            .normal_try_copy_jump_nop_successors
                            .insert(label(3));
                    }
                    23 => {
                        f.provenance.removed_pop_block_boundaries.insert(label(3));
                    }
                    24 => {
                        f.provenance.omitted_nop_counts.insert(label(3), 1);
                    }
                    25 => {
                        f.provenance
                            .omitted_nop_locations
                            .insert(label(3), SourceLocation::NONE);
                    }
                    26 => f.provenance.setups.push((label(3), None)),
                    27 => f.rebuilt[0].depth += 1,
                    28 => f.rebuilt[0].target = label(4),
                    29 => f.rebuilt[0].preserve_lasti = !f.rebuilt[0].preserve_lasti,
                    30 => f.rebuilt[0].is_scope_fallback = true,
                    31 => {
                        f.rebuilt[0].priority =
                            ExceptionRegionPriority::Resolved { start: 3, end: 6 }
                    }
                    32 => f.rebuilt.push(f.rebuilt[0]),
                    33 => f.rebuilt[0].start = label(4),
                    34 => f.rebuilt[0].end = label(4),
                    35 => {
                        f.items[4] = Item::Instruction(Instruction::new(
                            POP_JUMP_IF_TRUE,
                            Operand::Forward(label(3)),
                            SourceLocation::NONE,
                            Some(0),
                        ));
                    }
                    36 => {
                        f.items[3] = Item::Instruction(Instruction::new(
                            LOAD_FAST,
                            Operand::Value(1),
                            SourceLocation::NONE,
                            Some(0),
                        ));
                    }
                    37 => {
                        let moved = f.items.drain(5..8).collect::<Vec<_>>();
                        f.items.splice(12..12, moved);
                    }
                    38 => f.items[8] = Item::Label(label(20)),
                    39 => {
                        f.items[9] = Item::Instruction(Instruction::new(
                            LOAD_CONST,
                            Operand::Value(1),
                            SourceLocation::NONE,
                            Some(0),
                        ));
                    }
                    40 => f.items.insert(0, Item::Label(label(3))),
                    41 => {
                        f.provenance
                            .normal_try_copy_pop_ranges
                            .push((label(3), label(4)));
                    }
                    42 => {
                        f.items.swap(2, 8);
                    }
                    43..=50 => {
                        let Item::Instruction(record) = &mut f.items[if mode < 47 { 3 } else { 9 }]
                        else {
                            unreachable!()
                        };
                        match (mode - 43) % 4 {
                            0 => record.early_boolean_admissions.append(true),
                            1 => {
                                record.folded_constant_admissions =
                                    Some(crate::assembler::ir::FoldAdmissionId::from_u32(1));
                            }
                            2 => record
                                .no_location_policy
                                .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                            3 => record.jump_policy.prevent_small_exit_inlining(),
                            _ => unreachable!(),
                        }
                    }
                    _ => unreachable!(),
                }
                let expected = if mode == 0 {
                    if empty {
                        [label(3)].into_iter().collect()
                    } else {
                        [label(2), label(3)].into_iter().collect()
                    }
                } else {
                    FxHashSet::default()
                };
                assert_eq!(
                    f.labels(&witness),
                    expected,
                    "empty={empty} hidden={hidden} mode={mode}"
                );
            }
        }
    }
}

fn exclusion_fixture() -> Fixture {
    let mut f = fixture(false, 0);
    f.items.insert(7, Item::Label(label(7)));
    let owner = f.original[1];
    let before = ExceptionRegion {
        end: label(7),
        ..owner
    };
    let after = ExceptionRegion {
        start: label(4),
        ..owner
    };
    let second_before = ExceptionRegion {
        target: label(8),
        depth: 2,
        ..before
    };
    let second_after = ExceptionRegion {
        target: label(8),
        depth: 2,
        ..after
    };
    f.original = vec![f.original[0], before, after, second_before, second_after];
    f.rebuilt = vec![f.rebuilt[0], before, after, second_before, second_after];
    let returned = *f.items.last().unwrap();
    f.items.push(Item::Label(label(8)));
    f.items.push(returned);
    f.provenance.retained_pop_exclusions.push(PopExclusion {
        region_end: label(3),
        start: label(7),
        end: label(4),
        handlers: vec![label(6), label(8)],
    });
    f
}

#[test]
fn retained_pop_exclusions_require_authored_pair_and_complete_ordered_owners() {
    for mode in 0..25 {
        let mut f = exclusion_fixture();
        match mode {
            0 => {}
            1 => f.provenance.retained_pop_exclusions.clear(),
            2 => f
                .provenance
                .retained_pop_exclusions
                .push(f.provenance.retained_pop_exclusions[0].clone()),
            3 => f.provenance.retained_pop_exclusions[0].region_end = label(2),
            4 => f.provenance.retained_pop_exclusions[0].start = label(3),
            5 => f.provenance.retained_pop_exclusions[0].end = label(5),
            6 => f.provenance.retained_pop_exclusions[0].handlers.reverse(),
            7 => f.provenance.retained_pop_exclusions[0].handlers.clear(),
            8 => {
                f.provenance.retained_pop_exclusions[0].handlers.pop();
            }
            9 => f.provenance.retained_pop_exclusions[0]
                .handlers
                .push(label(8)),
            10 => f.provenance.retained_pop_exclusions[0].handlers[0] = label(5),
            11 => f.original[2].depth += 1,
            12 => f.original[2].preserve_lasti = false,
            13 => f.original[2].is_scope_fallback = true,
            14 => f.original[2].priority = ExceptionRegionPriority::Resolved { start: 4, end: 6 },
            15 => f.original[2].target = label(5),
            16 => f.original[1].end = label(3),
            17 => f.original[2].start = label(3),
            18 => f.original.push(f.original[1]),
            19 => f.original.push(f.original[2]),
            20 => {
                f.original.remove(2);
            }
            21 => f.original[1].preserve_lasti = false,
            22 => f.original[1].is_scope_fallback = true,
            23 => {
                f.original[3].depth = 1;
                f.original[4].depth = 1;
            }
            24 => f.items.insert(10, Item::Label(label(9))),
            _ => unreachable!(),
        }
        let rewrite = f.capture();
        assert_eq!(
            rewrite
                .origins
                .iter()
                .any(|origin| origin.exclusion.is_some()),
            mode == 0,
            "mode={mode}"
        );
    }
}

#[test]
fn retained_pop_exclusions_expire_on_later_endpoint_or_source_changes() {
    for mode in 0..29 {
        let mut f = exclusion_fixture();
        let witness = f.sealed();
        match mode {
            0 => {}
            1 => f.provenance.retained_pop_exclusions.clear(),
            2 => f
                .provenance
                .retained_pop_exclusions
                .push(f.provenance.retained_pop_exclusions[0].clone()),
            3 => f.provenance.retained_pop_exclusions[0].handlers.reverse(),
            4 => {
                f.provenance.source_boundaries.insert(label(4));
            }
            5 => {
                f.provenance.block_boundaries.insert(label(4));
            }
            6 => {
                f.preserved.insert(label(4));
            }
            7 => {
                f.provenance.fusion_boundaries.insert(label(4));
            }
            8 => {
                f.provenance.retired_normal_try_copy_labels.insert(label(4));
            }
            9 => {
                f.provenance.normal_try_copy_blocks.insert(label(4));
            }
            10 => {
                f.provenance.normal_try_copy_inline_labels.insert(label(4));
            }
            11 => {
                f.provenance.normal_try_copy_internal_labels.push(label(4));
            }
            12 => {
                f.provenance
                    .normal_try_copy_jump_nop_successors
                    .insert(label(4));
            }
            13 => {
                f.provenance.removed_pop_block_boundaries.insert(label(4));
            }
            14 => {
                f.provenance.omitted_nop_counts.insert(label(4), 1);
            }
            15 => {
                f.provenance
                    .omitted_nop_locations
                    .insert(label(4), SourceLocation::NONE);
            }
            16 => f.provenance.setups.push((label(4), None)),
            17 => f.items[9] = Item::Label(label(9)),
            18 => f.items.insert(9, Item::Label(label(9))),
            19 => f.items.insert(9, f.items[10]),
            20 => f.items.swap(7, 9),
            21 => {
                f.items[4] = Item::Instruction(Instruction::new(
                    POP_JUMP_IF_TRUE,
                    Operand::Forward(label(4)),
                    SourceLocation::NONE,
                    Some(0),
                ));
            }
            22 => f.rebuilt[1].depth += 1,
            23 => f.rebuilt[2].target = label(4),
            24 => {
                f.provenance
                    .normal_try_copy_pop_ranges
                    .push((label(4), label(5)));
            }
            25..=28 => {
                let Item::Instruction(record) = &mut f.items[10] else {
                    unreachable!()
                };
                match mode {
                    25 => record.early_boolean_admissions.append(true),
                    26 => {
                        record.folded_constant_admissions =
                            Some(crate::assembler::ir::FoldAdmissionId::from_u32(1));
                    }
                    27 => record
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                    28 => record.jump_policy.prevent_small_exit_inlining(),
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(
            f.labels(&witness).contains(&label(4)),
            mode == 0,
            "mode={mode}"
        );
    }
}

#[test]
fn cold_pop_end_witness_requires_complete_original_and_rebuilt_groups() {
    for empty in [false, true] {
        for mode in 0..8 {
            let f = fixture(empty, 1);
            let mut rewrite = f.capture();
            let outputs = if empty { &[][..] } else { &f.rebuilt[..1] };
            let mut original = f.original[0];
            if mode == 1 {
                original.depth += 1;
            }
            rewrite.group(usize::from(mode == 2), original, outputs);
            if mode != 3 {
                rewrite.group(1, f.original[1], &f.rebuilt[f.rebuilt.len() - 1..]);
            }
            if mode == 4 {
                rewrite.group(1, f.original[1], &[]);
            }
            let mut rebuilt = f.rebuilt.clone();
            match mode {
                5 => rebuilt.reverse(),
                6 => {
                    rebuilt.pop();
                }
                7 => rebuilt.push(f.original[0]),
                _ => {}
            }
            let witness = rewrite.finish(&f.items, &rebuilt, &f.provenance, &f.preserved);
            let valid = !witness
                .valid_labels(&f.items, &rebuilt, &f.provenance, &f.preserved)
                .is_empty();
            assert_eq!(
                valid,
                mode == 0 || (mode == 5 && empty),
                "empty={empty} mode={mode}"
            );
        }
    }
}

#[test]
fn cold_split_groups_keep_all_fragments_and_only_the_unique_pop_end() {
    for fragments in [2, 3] {
        for mode in 0..16 {
            let mut f = fixture(false, 0);
            let mut outputs = vec![f.rebuilt[0]; fragments];
            for (index, output) in outputs.iter_mut().enumerate().skip(1) {
                let index = u32::try_from(index).unwrap();
                output.start = label(4 + index);
                output.end = label(8 + index);
            }
            let mut regions = outputs.clone();
            regions.push(f.original[1]);
            let mut rewrite = f.capture();
            let mut original = f.original[0];
            match mode {
                0 => {}
                1 => outputs[1].target = label(6),
                2 => outputs[1].depth += 1,
                3 => outputs[1].preserve_lasti = true,
                4 => outputs[1].is_scope_fallback = true,
                5 => outputs[1].priority = ExceptionRegionPriority::Resolved { start: 4, end: 6 },
                6 => outputs[1].end = label(2),
                7 => outputs[0].end = label(4),
                8 => regions.swap(0, 1),
                9 => {
                    regions.remove(1);
                }
                10 => regions.insert(1, outputs[1]),
                11 => outputs.swap(0, 1),
                12 => outputs.clear(),
                13 => original.depth += 1,
                14 => rewrite.group(1, f.original[1], &f.original[1..]),
                15 => f.items.swap(5, 6),
                _ => unreachable!(),
            }
            rewrite.group(0, original, &outputs);
            rewrite.group(1, f.original[1], &f.original[1..]);
            let witness = rewrite.finish(&f.items, &regions, &f.provenance, &f.preserved);
            let labels = witness.valid_labels(&f.items, &regions, &f.provenance, &f.preserved);
            let expected = if mode == 0 {
                [label(2), label(3)].into_iter().collect()
            } else {
                FxHashSet::default()
            };
            assert_eq!(labels, expected, "fragments={fragments} mode={mode}");
        }
    }
}

#[test]
fn cold_pop_arrays_allow_only_the_late_load_fast_borrow_opcode_change() {
    let mut f = fixture(false, 0);
    let Item::Instruction(following) = &mut f.items[9] else {
        unreachable!()
    };
    following.opcode = LOAD_FAST;
    let witness = f.sealed();
    for index in [3, 9] {
        let Item::Instruction(record) = &mut f.items[index] else {
            unreachable!()
        };
        record.opcode = LOAD_FAST_BORROW;
    }
    assert_eq!(
        f.labels(&witness),
        [label(2), label(3)].into_iter().collect()
    );
}
