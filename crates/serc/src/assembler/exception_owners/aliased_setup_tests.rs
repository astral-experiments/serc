use super::{ExceptionOwnerProvenance, OwnerLabelPositions};
use crate::assembler::{
    ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
    LayoutAssembler, Operand, SourceLocation,
};
use crate::target::opcodes::{LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_NONE, RESUME};

fn label(value: u32) -> Label {
    Label::from_u32(value)
}

fn fixture(join: bool) -> LayoutAssembler {
    let setup_location = SourceLocation::new(3, 7, 4, 24);
    let condition_location = SourceLocation::new(6, 6, 8, 25);
    let mut pop = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(2));
    pop.fusion_policy.mark_converted_pop_block();
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    let load = Instruction::new(LOAD_FAST, Operand::Value(0), condition_location, Some(3));
    let mut condition = Instruction::new(
        POP_JUMP_IF_NONE,
        Operand::Forward(label(8)),
        condition_location,
        Some(2),
    );
    condition.local_policy.add_hidden_exception_owner_slots(3);
    let mut nop = pop;
    nop.location = setup_location;
    nop.local_policy.add_hidden_exception_owner_slots(1);
    let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), condition_location, Some(2));
    marker.set_exception_policy(ExceptionPolicy::NonePresent);
    let mut setups = vec![(label(4), Some(setup_location))];
    if !join {
        setups.push((label(1), None));
    }
    LayoutAssembler {
        python_version: crate::PythonVersion::Py314,
        legacy_uniform_jump_seed: false,
        items: vec![
            Item::Instruction(Instruction::new(
                RESUME,
                Operand::Value(0),
                SourceLocation::NONE,
                Some(0),
            )),
            Item::Label(label(1)),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(0),
                SourceLocation::NONE,
                Some(1),
            )),
            Item::Label(label(2)),
            Item::Label(label(3)),
            Item::Label(label(4)),
            Item::Instruction(nop),
            Item::Instruction(load),
            Item::Instruction(condition),
            Item::Instruction(marker),
            Item::Label(label(8)),
            Item::Label(label(9)),
            Item::Label(label(10)),
        ],
        exception_regions: vec![ExceptionRegion {
            start: label(1),
            end: label(9),
            target: label(10),
            depth: 2,
            preserve_lasti: true,
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(label(1), label(9)),
        }],
        exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
            setups,
            synchronous_with_setups: [label(1)].into_iter().collect(),
            setup_nop_ranges: vec![(label(2), label(3))],
            normal_finally_none_prefixes: vec![(pop, load, condition)],
            block_boundaries: if join {
                [label(2)].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            },
            source_boundaries: if join {
                [label(2)].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            },
            ..ExceptionOwnerProvenance::default()
        }),
        preserved_block_boundaries: rustc_hash::FxHashSet::default(),
        removed_max_depth: None,
        next_label: 11,
        has_next_location: false,
    }
}

fn recognizes_at(assembler: &LayoutAssembler, marker: usize, start: usize, join: bool) -> bool {
    let mut positions = OwnerLabelPositions::new(assembler.items.len());
    let mut records = Vec::new();
    for item in &assembler.items {
        match item {
            Item::Label(label) => {
                positions.insert(*label, records.len());
            }
            Item::Instruction(instruction) => records.push(instruction),
        }
    }
    let targets = if join {
        [label(2), label(8)].into_iter().collect()
    } else {
        [label(8)].into_iter().collect()
    };
    assembler.aliased_setup_none_owner(marker, start, &records, &positions, &targets) == Some(0)
}

fn recognizes(assembler: &LayoutAssembler, join: bool) -> bool {
    recognizes_at(assembler, 5, if join { 2 } else { 0 }, join)
}

#[test]
fn retained_setup_alias_selects_only_the_original_owned_test() {
    for join in [false, true] {
        assert!(recognizes(&fixture(join), join));
    }
}

#[test]
fn retained_setup_alias_requires_an_entry_prologue() {
    let mut assembler = fixture(true);
    assembler.items.remove(2);
    assembler.items.remove(0);
    assert!(!recognizes_at(&assembler, 3, 0, false));
    assert!(!recognizes_at(&assembler, 3, 0, true));
}

#[test]
fn retained_setup_alias_rejects_changed_sources_regions_and_boundaries() {
    for join in [false, true] {
        for mutation in [
            "different version",
            "missing source",
            "duplicate source",
            "source load",
            "source pop",
            "duplicate label",
            "range endpoint",
            "setup location",
            "unknown setup",
            "copied array",
            "interior boundary",
            "marker depth",
            "marker operand",
            "marker hidden",
            "marker barrier",
            "wrong owner",
            "join source",
            "join block",
            "manager setup",
        ] {
            let mut assembler = fixture(join);
            if (!join && matches!(mutation, "join source" | "join block"))
                || (join && mutation == "manager setup")
            {
                continue;
            }
            match mutation {
                "different version" => assembler.python_version = crate::PythonVersion::Py315,
                "missing source" => assembler
                    .exception_owner_provenance
                    .normal_finally_none_prefixes
                    .clear(),
                "duplicate source" => {
                    let source = assembler
                        .exception_owner_provenance
                        .normal_finally_none_prefixes[0];
                    assembler
                        .exception_owner_provenance
                        .normal_finally_none_prefixes
                        .push(source);
                }
                "source load" => assembler
                    .exception_owner_provenance
                    .normal_finally_none_prefixes[0]
                    .1
                    .set_operand(Operand::Value(2)),
                "source pop" => {
                    assembler
                        .exception_owner_provenance
                        .normal_finally_none_prefixes[0]
                        .0
                        .location = SourceLocation::new(2, 2, 0, 4);
                }
                "duplicate label" => assembler.items.insert(4, Item::Label(label(2))),
                "range endpoint" => {
                    assembler.exception_owner_provenance.setup_nop_ranges[0].1 = label(8);
                }
                "setup location" => {
                    assembler.exception_owner_provenance.setups[0].1 =
                        Some(SourceLocation::new(2, 2, 0, 4));
                }
                "unknown setup" => assembler
                    .exception_owner_provenance
                    .setups
                    .push((label(3), None)),
                "copied array" => {
                    assembler
                        .exception_owner_provenance
                        .normal_try_copy_blocks
                        .insert(label(2));
                }
                "interior boundary" => {
                    assembler
                        .exception_owner_provenance
                        .block_boundaries
                        .insert(label(11));
                    assembler.items.insert(7, Item::Label(label(11)));
                }
                "wrong owner" => assembler.exception_regions[0].is_scope_fallback = true,
                "join source" => assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .clear(),
                "join block" => assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .clear(),
                "manager setup" => assembler
                    .exception_owner_provenance
                    .synchronous_with_setups
                    .clear(),
                _ => {
                    let marker = assembler
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            Item::Instruction(record) if record.opcode == NOT_TAKEN => Some(record),
                            _ => None,
                        })
                        .unwrap();
                    match mutation {
                        "marker depth" => {
                            *marker = Instruction::new(
                                NOT_TAKEN,
                                Operand::Value(0),
                                marker.location,
                                Some(3),
                            );
                        }
                        "marker operand" => marker.set_operand(Operand::Value(1)),
                        "marker hidden" => marker.local_policy.add_hidden_exception_owner_slots(1),
                        "marker barrier" => marker
                            .no_location_policy
                            .set_borrow_fallthrough(crate::assembler::BorrowFallthrough::Barrier),
                        _ => unreachable!(),
                    }
                }
            }
            assert!(!recognizes(&assembler, join), "join={join}: {mutation}");
        }
    }
}
