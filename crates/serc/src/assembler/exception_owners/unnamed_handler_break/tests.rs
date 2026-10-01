use super::*;
use crate::PythonVersion;
use crate::assembler::UnoptimizedCode;
use crate::target::opcodes::{JUMP_FORWARD, RETURN_VALUE};

fn label(index: u32) -> Label {
    Label::from_u32(index)
}

fn fixture() -> (Assembler, usize, SourceLocation, SourceLocation) {
    let mut assembler = Assembler::with_python_version(PythonVersion::Py314);
    let authored = SourceLocation::new(5, 5, 8, 13);
    let load_location = SourceLocation::new(7, 7, 7, 12);
    let condition_location = SourceLocation::new(7, 7, 7, 24);
    let mut nop = Instruction::new(NOP, Operand::Value(0), authored, Some(1));
    nop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    let pop = Instruction::new(POP_EXCEPT, Operand::Value(0), authored, Some(0));
    let load = Instruction::new(LOAD_FAST, Operand::Value(0), load_location, Some(1));
    let mut condition = Instruction::new(
        POP_JUMP_IF_NONE,
        Operand::Forward(label(4)),
        condition_location,
        Some(0),
    );
    condition.local_policy.add_hidden_exception_owner_slots(3);
    let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), condition_location, Some(0));
    marker.set_exception_policy(ExceptionPolicy::NonePresent);
    let mut items = vec![
        Item::Label(label(0)),
        Item::Instruction(nop),
        Item::Label(label(1)),
        Item::Instruction(pop),
        Item::Label(label(2)),
        Item::Instruction(load),
        Item::Instruction(condition),
        Item::Instruction(marker),
        Item::Label(label(3)),
    ];
    for index in 4..=8 {
        items.push(Item::Label(label(index)));
        items.push(Item::Instruction(Instruction::new(
            RETURN_VALUE,
            Operand::Value(0),
            authored,
            Some(0),
        )));
    }
    assembler.code = UnoptimizedCode::Linear(items);
    assembler
        .exception_owner_provenance
        .source_boundaries
        .insert(label(0));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(label(0));
    assembler.exception_regions.push(ExceptionRegion {
        start: label(0),
        end: label(2),
        target: label(7),
        depth: 0,
        preserve_lasti: false,
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(label(0), label(2)),
    });
    (assembler, 1, load_location, condition_location)
}

fn identities() -> [Label; 5] {
    [label(5), label(6), label(8), label(8), label(7)]
}

fn capture(
    assembler: &mut Assembler,
    start: usize,
    load: SourceLocation,
    condition: SourceLocation,
) {
    assembler.record_unnamed_handler_break_none(start, identities(), load, condition);
}

fn bound() -> Assembler {
    let (mut assembler, start, load, condition) = fixture();
    capture(&mut assembler, start, load, condition);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .unnamed_handler_breaks
            .len(),
        1
    );
    assembler.add_stopiteration_exception_region_with_depth(label(2), label(3), label(8), 0);
    assembler.bind_unnamed_handler_break_owners();
    assert_eq!(
        assembler
            .exception_owner_provenance
            .unnamed_handler_breaks
            .len(),
        1
    );
    assembler
}

fn positions(items: &[Item]) -> (FxHashMap<Label, usize>, OwnerLabelPositions) {
    let mut map = FxHashMap::default();
    let mut positions = OwnerLabelPositions::new(32);
    let mut index = 0;
    for item in items {
        match item {
            Item::Label(label) => {
                map.insert(*label, index);
                positions.insert(*label, index);
            }
            Item::Instruction(_) => index += 1,
        }
    }
    (map, positions)
}

fn layout() -> LayoutAssembler {
    layout_from(bound())
}

fn layout_from(mut assembler: Assembler) -> LayoutAssembler {
    let (map, _) = positions(assembler.linear_items());
    let origin = &mut assembler.exception_owner_provenance.unnamed_handler_breaks[0];
    assert!(origin.resolve_priorities(&map));
    assembler.exception_regions = origin.owners.clone();
    let regions = assembler.exception_regions.clone();
    let pairs = regions
        .iter()
        .map(|region| (*region, *region))
        .collect::<Vec<_>>();
    assert!(origin.rebind_cold(&regions, &pairs, &regions));
    let mut items = assembler.linear_items().to_vec();
    let first_instruction = items
        .iter()
        .position(|item| matches!(item, Item::Instruction(_)))
        .unwrap();
    items.remove(first_instruction);
    let pop = items
        .iter_mut()
        .find_map(|item| match item {
            Item::Instruction(record) => Some(record),
            Item::Label(_) => None,
        })
        .unwrap();
    pop.local_policy.add_hidden_exception_owner_slots(1);
    LayoutAssembler {
        python_version: PythonVersion::Py314,
        legacy_uniform_jump_seed: false,
        items,
        exception_regions: assembler.exception_regions,
        exception_owner_provenance: assembler.exception_owner_provenance,
        preserved_block_boundaries: FxHashSet::default(),
        removed_max_depth: None,
        next_label: 32,
        has_next_location: false,
    }
}

#[test]
fn unnamed_handler_break_preserves_the_complete_ordered_alias_head() {
    for mutation in 0..8 {
        let (mut assembler, start, load, condition) = fixture();
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            unreachable!();
        };
        items.insert(0, Item::Label(label(9)));
        capture(&mut assembler, start + 1, load, condition);
        assert_eq!(
            assembler.exception_owner_provenance.unnamed_handler_breaks[0].aliases[0],
            (label(9), 0)
        );
        assembler.add_stopiteration_exception_region_with_depth(label(2), label(3), label(8), 0);
        assembler.bind_unnamed_handler_break_owners();
        let mut assembler = layout_from(assembler);
        match mutation {
            0 => {}
            1 => assembler.items.swap(0, 1),
            2 => {
                assembler.items.remove(0);
            }
            3 => assembler.items.insert(0, Item::Label(label(10))),
            4 => assembler.items.insert(1, Item::Label(label(10))),
            5 => assembler.items.insert(5, Item::Label(label(10))),
            6 => assembler.items.push(Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(label(9)),
                SourceLocation::NONE,
                Some(0),
            ))),
            7 => assembler.items.push(Item::Label(label(9))),
            _ => unreachable!(),
        }
        let (_, positions) = positions(&assembler.items);
        let records = assembler
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record) => Some(record),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let tail = ExceptionOwnerTail {
            early_removals: 4,
            ..ExceptionOwnerTail::default()
        };
        assert_eq!(
            assembler.unnamed_handler_break_null_owner(
                3,
                0,
                tail,
                &records,
                &positions,
                &FxHashSet::default()
            ),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn unnamed_handler_break_capture_checks_every_original_record_and_role() {
    for mutation in 0..25 {
        let (mut assembler, start, load, condition) = fixture();
        match mutation {
            0 => {}
            1 => assembler.python_version = PythonVersion::Py315,
            2 => assembler
                .exception_owner_provenance
                .source_boundaries
                .clear(),
            3 => assembler
                .exception_owner_provenance
                .block_boundaries
                .clear(),
            4 => {
                assembler
                    .exception_owner_provenance
                    .normal_try_copy_blocks
                    .insert(label(0));
            }
            5 => {
                assembler
                    .exception_owner_provenance
                    .retired_normal_try_copy_labels
                    .insert(label(0));
            }
            6 => {
                assembler
                    .exception_owner_provenance
                    .normal_try_copy_internal_labels
                    .push(label(0));
            }
            7 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(label(2));
            }
            8 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(label(2));
            }
            _ => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                let at = if mutation < 14 {
                    1
                } else if mutation < 17 {
                    3
                } else if mutation < 21 {
                    5
                } else if mutation < 24 {
                    6
                } else {
                    7
                };
                let Item::Instruction(record) = &mut items[at] else {
                    unreachable!();
                };
                match mutation {
                    9 => record.opcode = POP_EXCEPT,
                    10 => record.location = SourceLocation::NONE,
                    11 => record.set_operand(Operand::Value(1)),
                    12 => record.set_exception_policy(ExceptionPolicy::default()),
                    13 => record.fusion_policy.mark_converted_pop_block(),
                    14 => record.opcode = NOP,
                    15 => record.local_policy.add_hidden_exception_owner_slots(1),
                    16 => record.jump_policy.preserve_explicit_nop(),
                    17 => record.set_operand(Operand::Forward(label(4))),
                    18 => record.location = SourceLocation::NONE,
                    19 => record.local_policy.mark_force_borrowed_load(),
                    20 => record.fusion_policy.prevent_with_previous(),
                    21 => record.local_policy.add_hidden_exception_owner_slots(1),
                    22 => record.retarget(Operand::Backward(label(4))),
                    23 => record.set_exception_policy(ExceptionPolicy::NonePresent),
                    24 => record.set_exception_policy(ExceptionPolicy::NoneAbsent),
                    _ => unreachable!(),
                }
            }
        }
        capture(&mut assembler, start, load, condition);
        assert_eq!(
            assembler
                .exception_owner_provenance
                .unnamed_handler_breaks
                .len(),
            usize::from(mutation == 0),
            "mutation {mutation}"
        );
    }
}

#[test]
fn unnamed_handler_break_binding_requires_actual_fallback_and_incoming_identity() {
    for mutation in 0..7 {
        let (mut assembler, start, load, condition) = fixture();
        capture(&mut assembler, start, load, condition);
        assembler.add_stopiteration_exception_region_with_depth(label(2), label(3), label(8), 0);
        match mutation {
            0 => {}
            1 => assembler.exception_owner_provenance.unnamed_handler_breaks[0]
                .fallback_producers
                .clear(),
            2 => assembler.exception_regions[0].target = label(8),
            3 => assembler.exception_regions[0].depth = 1,
            4 => assembler
                .exception_regions
                .push(assembler.exception_regions[0]),
            5 => {
                let region = assembler.exception_owner_provenance.unnamed_handler_breaks[0]
                    .fallback_producers[0];
                assembler.exception_owner_provenance.unnamed_handler_breaks[0]
                    .fallback_producers
                    .push(region);
            }
            6 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                items.push(Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(label(0)),
                    SourceLocation::NONE,
                    Some(0),
                )));
            }
            _ => unreachable!(),
        }
        assembler.bind_unnamed_handler_break_owners();
        assert_eq!(
            assembler
                .exception_owner_provenance
                .unnamed_handler_breaks
                .len(),
            usize::from(mutation == 0),
            "mutation {mutation}"
        );
    }
}

#[test]
fn unnamed_handler_break_replay_preserves_full_records_and_complete_table() {
    for mutation in 0..33 {
        let mut assembler = layout();
        let mut tail = ExceptionOwnerTail {
            early_removals: 4,
            ..ExceptionOwnerTail::default()
        };
        let mut targets = FxHashSet::default();
        match mutation {
            0 => {}
            1 => assembler.python_version = PythonVersion::Py315,
            2 => tail.early_removals = 3,
            3 => tail.insertions = 1,
            4 => tail.late_removals = 1,
            5 => tail.post_compaction_fusions = 1,
            6 => assembler
                .exception_owner_provenance
                .source_boundaries
                .clear(),
            7 => assembler
                .exception_owner_provenance
                .block_boundaries
                .clear(),
            8 => {
                assembler
                    .exception_owner_provenance
                    .normal_try_copy_blocks
                    .insert(label(0));
            }
            9 => {
                assembler
                    .exception_owner_provenance
                    .retired_normal_try_copy_labels
                    .insert(label(1));
            }
            10 => {
                assembler
                    .exception_owner_provenance
                    .normal_try_copy_internal_labels
                    .push(label(2));
            }
            11 => {
                targets.insert(label(2));
            }
            12 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(label(2));
            }
            13 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(label(2));
            }
            14 => assembler.items.push(Item::Label(label(0))),
            15 => assembler
                .items
                .retain(|item| *item != Item::Label(label(5))),
            16 => assembler.items.push(Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(label(0)),
                SourceLocation::NONE,
                Some(0),
            ))),
            17 => assembler.exception_regions[0].target = label(8),
            18 => assembler.exception_regions[0].depth = 1,
            19 => assembler.exception_regions[0].preserve_lasti = true,
            20 => assembler.exception_regions[1].is_scope_fallback = false,
            21 => {
                assembler.exception_regions[1].priority =
                    ExceptionRegionPriority::Resolved { start: 10, end: 20 }
            }
            22 => assembler.exception_regions.swap(0, 1),
            23 => assembler
                .exception_regions
                .push(assembler.exception_regions[0]),
            24 => {
                let mut region = assembler.exception_regions[0];
                region.start = label(5);
                region.end = label(6);
                assembler.exception_regions.push(region);
            }
            25 => assembler.exception_owner_provenance.unnamed_handler_breaks[0].cold_table = None,
            _ => {
                let mut records = assembler.items.iter_mut().filter_map(|item| match item {
                    Item::Instruction(record) => Some(record),
                    Item::Label(_) => None,
                });
                let record = records
                    .nth(match mutation {
                        26 | 27 => 0,
                        28 | 29 => 1,
                        30 | 31 => 2,
                        _ => 3,
                    })
                    .unwrap();
                match mutation {
                    26 => record.local_policy.add_hidden_exception_owner_slots(1),
                    27 => record.location = SourceLocation::NONE,
                    28 => record.set_operand(Operand::Value(1)),
                    29 => record.local_policy.mark_force_borrowed_load(),
                    30 => record.retarget(Operand::Forward(label(6))),
                    31 => record.jump_policy.preserve_explicit_nop(),
                    32 => record.set_exception_policy(ExceptionPolicy::NoneAbsent),
                    _ => unreachable!(),
                }
            }
        }
        let (_, positions) = positions(&assembler.items);
        let records = assembler
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record) => Some(record),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            assembler.unnamed_handler_break_null_owner(3, 0, tail, &records, &positions, &targets),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn unnamed_handler_break_transport_requires_real_ordered_nonempty_groups() {
    for mutation in 0..14 {
        let mut assembler = bound();
        let (map, _) = positions(assembler.linear_items());
        let origin = &mut assembler.exception_owner_provenance.unnamed_handler_breaks[0];
        assert!(origin.resolve_priorities(&map));
        let mut before = origin.owners.clone();
        let mut pairs = before
            .iter()
            .map(|region| (*region, *region))
            .collect::<Vec<_>>();
        let mut current = before.clone();
        match mutation {
            0 => {}
            1 => {
                pairs.remove(0);
                current.remove(0);
            }
            2 => {
                pairs.remove(1);
                current.remove(1);
            }
            3 => pairs.swap(0, 1),
            4 => current.swap(0, 1),
            5 => {
                pairs.swap(0, 1);
                current.swap(0, 1);
            }
            6 => pairs[0].0.depth += 1,
            7 => {
                pairs[0].1.target = label(8);
                current[0] = pairs[0].1;
            }
            8 => {
                pairs[1].1.priority = ExceptionRegionPriority::Resolved {
                    start: 100,
                    end: 101,
                };
                current[1] = pairs[1].1;
            }
            9 => before.push(before[0]),
            10 => {
                before.remove(0);
            }
            11 => {
                pairs[0].1.preserve_lasti = true;
                current[0] = pairs[0].1;
            }
            12 => {
                pairs[1].1.is_scope_fallback = false;
                current[1] = pairs[1].1;
            }
            13 => {
                assert!(origin.rebind_cold(&before, &pairs, &current));
            }
            _ => unreachable!(),
        }
        assert_eq!(
            origin.rebind_cold(&before, &pairs, &current),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn unnamed_handler_break_condition_transport_is_exact_and_one_shot() {
    for mutation in 0..9 {
        let mut assembler = bound();
        let origin = &mut assembler.exception_owner_provenance.unnamed_handler_breaks[0];
        let before = origin.records[3];
        let mut after = before;
        after.retarget(Operand::Forward(label(9)));
        match mutation {
            0..=3 => {}
            4 => after.location = SourceLocation::NONE,
            5 => after.local_policy.add_hidden_exception_owner_slots(1),
            6 => after.retarget(Operand::Backward(label(9))),
            7 => after.set_exception_policy(ExceptionPolicy::NonePresent),
            8 => after.jump_policy.preserve_explicit_nop(),
            _ => unreachable!(),
        }
        let mut receipts = vec![(before, after)];
        let mut items = vec![Item::Instruction(after)];
        if mutation == 1 {
            receipts.push((before, after));
        }
        if mutation == 2 {
            items.push(Item::Instruction(after));
        }
        if mutation == 3 {
            origin.condition_transported = true;
        }
        assert_eq!(
            origin.retarget_condition(&receipts, &items),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn unnamed_handler_break_marker_head_receipt_requires_the_original_array() {
    for mutation in 0..14 {
        let mut assembler = bound();
        let mut origin = assembler
            .exception_owner_provenance
            .unnamed_handler_breaks
            .pop()
            .unwrap();
        let mut items = assembler.linear_items().to_vec();
        let mut fresh = label(10);
        match mutation {
            0 => {}
            1 => items.insert(0, Item::Label(label(9))),
            2 => items.swap(0, 2),
            3 => {
                let Item::Instruction(pop) = &mut items[3] else {
                    unreachable!();
                };
                pop.local_policy.add_hidden_exception_owner_slots(1);
            }
            4 => items.push(Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(label(0)),
                SourceLocation::NONE,
                Some(0),
            ))),
            5 => {
                items.remove(0);
            }
            6 => {
                let Item::Instruction(load) = &mut items[5] else {
                    unreachable!();
                };
                load.set_operand(Operand::Value(1));
            }
            7 => fresh = label(8),
            8 => items.push(Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(fresh),
                SourceLocation::NONE,
                Some(0),
            ))),
            9 => origin.records[4].set_exception_policy(ExceptionPolicy::NoneAbsent),
            10 => items.insert(7, Item::Label(label(9))),
            11 => origin.marker_head = Some(label(11)),
            12 => items.push(Item::Instruction(origin.records[4])),
            13 => items.swap(1, 2),
            _ => unreachable!(),
        }
        let graph = BlockGraph::from_items(items, &FxHashSet::default());
        let blocks = graph.iter().filter_map(|(block, value)| value.items().iter().any(|item| matches!(item, Item::Instruction(record) if record.opcode == NOT_TAKEN)).then_some(block)).collect::<Vec<_>>();
        let block = if mutation == 12 {
            *blocks.last().unwrap()
        } else {
            blocks[0]
        };
        let before = origin.aliases.clone();
        origin.prepend_marker_head(&graph, block, fresh);
        assert_eq!(
            origin.marker_head == Some(fresh),
            mutation == 0,
            "mutation {mutation}"
        );
        if mutation == 0 {
            assert_eq!(origin.aliases[..before.len()], before);
            assert_eq!(origin.aliases.last(), Some(&(fresh, 3)));
            let after = origin.aliases.clone();
            origin.prepend_marker_head(&graph, block, label(11));
            assert_eq!(origin.aliases, after);
        } else {
            assert_eq!(origin.aliases, before);
        }
    }
}
