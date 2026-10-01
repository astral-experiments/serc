use super::*;
use crate::assembler::{ExceptionRegionPriority, UnoptimizedCode};
use crate::target::opcodes::RETURN_VALUE;

fn label(value: u32) -> Label {
    Label::from_u32(value)
}

fn fixture() -> (Assembler, usize, usize) {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    for _ in 0..16 {
        assembler.label();
    }
    let setup_location = SourceLocation::new(3, 6, 8, 12);
    let expression_location = SourceLocation::new(4, 4, 15, 35);
    let cleanup_location = SourceLocation::new(2, 2, 9, 18);
    let guard_location = SourceLocation::new(9, 9, 7, 25);
    let plain = |opcode, argument, location, depth| {
        Instruction::new(opcode, Operand::Value(argument), location, Some(depth))
    };
    let mut authored = plain(NOP, 0, setup_location, 2);
    authored.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    let mut inner_pop = plain(NOP, 0, expression_location, 3);
    inner_pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    inner_pop.fusion_policy.mark_converted_pop_block();
    let mut pop = plain(NOP, 0, SourceLocation::NONE, 1);
    pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    pop.fusion_policy.mark_converted_pop_block();
    let mut condition = Instruction::new(
        POP_JUMP_IF_NONE,
        Operand::Forward(label(10)),
        guard_location,
        Some(1),
    );
    condition.local_policy.add_hidden_exception_owner_slots(3);
    let mut marker = plain(NOT_TAKEN, 0, guard_location, 1);
    marker.set_exception_policy(ExceptionPolicy::NonePresent);
    let mut items = vec![
        Item::Label(label(0)),
        Item::Instruction(plain(NOP, 0, cleanup_location, 2)),
        Item::Label(label(8)),
        Item::Instruction(authored),
        Item::Label(label(9)),
        Item::Label(label(4)),
    ];
    let return_item = items.len();
    items.extend(
        [
            plain(LOAD_GLOBAL, 0, expression_location, 3),
            plain(LOAD_ATTR, 1, expression_location, 4),
            plain(LOAD_CONST, 0, expression_location, 5),
            plain(CALL, 1, expression_location, 3),
            inner_pop,
        ]
        .map(Item::Instruction),
    );
    items.push(Item::Label(label(12)));
    items.extend(
        [
            plain(SWAP, 3, cleanup_location, 3),
            plain(SWAP, 2, cleanup_location, 3),
            plain(LOAD_CONST, 0, cleanup_location, 4),
            plain(LOAD_CONST, 0, cleanup_location, 5),
            plain(LOAD_CONST, 0, cleanup_location, 6),
            plain(CALL, 3, cleanup_location, 2),
            plain(POP_TOP, 0, cleanup_location, 1),
        ]
        .map(Item::Instruction),
    );
    items.push(Item::Label(label(13)));
    let pop_item = items.len();
    items.extend(
        [
            pop,
            plain(LOAD_DEREF, 0, guard_location, 2),
            condition,
            marker,
        ]
        .map(Item::Instruction),
    );
    items.extend([
        Item::Label(label(10)),
        Item::Instruction(plain(RETURN_VALUE, 0, guard_location, 0)),
    ]);
    items.extend([5, 6, 1, 2, 3, 7, 11].map(|value| Item::Label(label(value))));
    items.push(Item::Instruction(plain(NOP, 0, SourceLocation::NONE, 0)));
    items.push(Item::Label(label(14)));
    assembler.code = UnoptimizedCode::Linear(items);
    assembler.exception_owner_provenance.setups =
        vec![(label(0), None), (label(4), Some(setup_location))];
    assembler
        .exception_owner_provenance
        .ordinary_try_setups
        .extend([(label(0), label(2)), (label(4), label(6))]);
    assembler
        .exception_owner_provenance
        .setup_nop_ranges
        .push((label(8), label(9)));
    assembler
        .exception_owner_provenance
        .source_boundaries
        .insert(label(8));
    assembler
        .exception_owner_provenance
        .block_boundaries
        .insert(label(8));
    assembler.exception_regions = vec![
        ExceptionRegion {
            start: label(4),
            end: label(12),
            target: label(6),
            depth: 2,
            preserve_lasti: false,
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(label(4), label(5)),
        },
        ExceptionRegion {
            start: label(0),
            end: label(13),
            target: label(2),
            depth: 0,
            preserve_lasti: false,
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(label(0), label(1)),
        },
    ];
    (assembler, return_item, pop_item)
}

fn capture(assembler: &mut Assembler, return_item: usize, pop_item: usize) {
    assembler.record_copied_nested_try_manager_return(
        return_item,
        pop_item,
        [
            (label(0), label(1), label(2)),
            (label(4), label(5), label(6)),
        ],
        label(3),
        label(7),
    );
}

fn bind(assembler: &mut Assembler) {
    assembler.add_stopiteration_exception_region_with_depth(label(0), label(14), label(11), 0);
    assembler.bind_copied_nested_try_manager_returns();
}

fn layout(mut assembler: Assembler) -> LayoutAssembler {
    let positions = unique_label_positions(assembler.linear_items()).unwrap();
    for region in &mut assembler.exception_regions {
        if let ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
            region.priority = ExceptionRegionPriority::Resolved {
                start: positions[&start],
                end: positions[&end],
            };
        }
    }
    for origin in &mut assembler
        .exception_owner_provenance
        .copied_nested_try_manager_returns
    {
        origin.resolve_priorities(&positions);
    }
    let origin = &assembler
        .exception_owner_provenance
        .copied_nested_try_manager_returns[0];
    let inner_pop = origin.inner_pop;
    let pop = origin.pop;
    let cleanup = origin.cleanup[0];
    let load = origin.load;
    let mut items = assembler.linear_items().to_vec();
    items.retain(|item| *item != Item::Instruction(inner_pop) && *item != Item::Instruction(pop));
    for item in &mut items {
        if let Item::Instruction(record) = item
            && (*record == cleanup || *record == load)
        {
            record.local_policy.add_hidden_exception_owner_slots(1);
        }
    }
    LayoutAssembler {
        python_version: crate::PythonVersion::Py314,
        legacy_uniform_jump_seed: false,
        items,
        exception_regions: assembler.exception_regions,
        exception_owner_provenance: assembler.exception_owner_provenance,
        preserved_block_boundaries: FxHashSet::default(),
        removed_max_depth: None,
        next_label: 16,
        has_next_location: false,
    }
}

fn replay(assembler: &LayoutAssembler, early: usize) -> bool {
    let mut positions = OwnerLabelPositions::new(16);
    let mut instructions = Vec::new();
    for item in &assembler.items {
        match item {
            Item::Label(label) => {
                positions.insert(*label, instructions.len());
            }
            Item::Instruction(record) => instructions.push(record),
        }
    }
    let marker = instructions
        .iter()
        .position(|record| record.opcode == NOT_TAKEN)
        .unwrap();
    let jump_targets = instructions
        .iter()
        .filter_map(|record| match record.operand() {
            Operand::Forward(label) | Operand::Backward(label) => Some(label),
            _ => None,
        })
        .collect();
    assembler.copied_nested_try_manager_null_owner(
        marker,
        positions[&label(8)],
        ExceptionOwnerTail {
            early_removals: early,
            ..ExceptionOwnerTail::default()
        },
        &instructions,
        &positions,
        &jump_targets,
    )
}

#[test]
fn nested_try_manager_source_capture_rejects_changed_setup_and_records() {
    for mode in 0..12 {
        let (mut assembler, return_item, pop_item) = fixture();
        match mode {
            0 => {}
            1 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .remove(&label(4));
            }
            2 => {
                assembler
                    .exception_owner_provenance
                    .setups
                    .push((label(4), None));
            }
            3 => {
                assembler
                    .exception_owner_provenance
                    .setup_nop_ranges
                    .clear();
            }
            4 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .clear();
            }
            5 => {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(label(12));
            }
            6 => {
                assembler.python_version = crate::PythonVersion::Py315;
            }
            7..=11 => {
                let index = match mode {
                    7 => return_item,
                    8 => return_item + 4,
                    9 => pop_item - 2,
                    10 => pop_item + 1,
                    _ => pop_item + 2,
                };
                let Item::Instruction(record) = &mut assembler.linear_items_mut()[index] else {
                    panic!()
                };
                match mode {
                    7 => record.local_policy.add_hidden_exception_owner_slots(1),
                    8 => record.set_exception_policy(ExceptionPolicy::NoneUnchanged),
                    9 => record.set_operand(Operand::Value(4)),
                    10 => record.opcode = LOAD_ATTR,
                    _ => record.jump_policy.prevent_threading_target(),
                }
            }
            _ => unreachable!(),
        }
        capture(&mut assembler, return_item, pop_item);
        assert_eq!(
            assembler
                .exception_owner_provenance
                .copied_nested_try_manager_returns
                .len(),
            usize::from(mode == 0),
            "mode {mode}"
        );
    }
}

#[test]
fn nested_try_manager_owner_requires_original_regions_and_exact_replay() {
    for mode in 0..24 {
        let (mut assembler, return_item, pop_item) = fixture();
        capture(&mut assembler, return_item, pop_item);
        if mode == 23 {
            capture(&mut assembler, return_item, pop_item);
        }
        bind(&mut assembler);
        assert!(
            !assembler
                .exception_owner_provenance
                .copied_nested_try_manager_returns
                .is_empty()
        );
        assembler.remap_constant_indices(&[Some(4)]);
        let mut assembler = layout(assembler);
        match mode {
            0 => {}
            1 => {
                assembler.exception_regions[0].start = label(9);
            }
            2 => {
                assembler.exception_regions[2].target = label(7);
            }
            3 => {
                assembler.exception_regions[2].preserve_lasti = false;
            }
            4 => {
                assembler.exception_regions[2].is_scope_fallback = false;
            }
            5 => {
                assembler.exception_regions[0].depth = 1;
            }
            6 => {
                assembler.exception_regions[1].priority =
                    ExceptionRegionPriority::Resolved { start: 0, end: 100 };
            }
            7 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(label(4), label(2));
            }
            8 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .clear();
            }
            9 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(label(12));
            }
            10 => {
                assembler
                    .exception_owner_provenance
                    .normal_try_copy_blocks
                    .insert(label(8));
            }
            11 => {
                assembler
                    .exception_owner_provenance
                    .copied_nested_try_manager_returns[0]
                    .inner_pop
                    .set_exception_policy(ExceptionPolicy::NoneUnchanged);
            }
            12 => {
                assembler
                    .exception_owner_provenance
                    .copied_nested_try_manager_returns[0]
                    .pop
                    .set_exception_policy(ExceptionPolicy::NoneUnchanged);
            }
            13 => {
                assembler.items.insert(
                    0,
                    Item::Instruction(Instruction::new(
                        crate::target::opcodes::JUMP_FORWARD,
                        Operand::Forward(label(8)),
                        SourceLocation::NONE,
                        Some(2),
                    )),
                );
            }
            14 => {
                let at = assembler
                    .items
                    .iter()
                    .position(|item| *item == Item::Label(label(4)))
                    .unwrap();
                assembler.items.insert(at, Item::Label(label(15)));
            }
            15..=20 => {
                let origin = &assembler
                    .exception_owner_provenance
                    .copied_nested_try_manager_returns[0];
                let wanted = match mode {
                    15 => LOAD_CONST,
                    16 | 17 => SWAP,
                    18 => LOAD_DEREF,
                    19 => origin.condition.opcode,
                    _ => NOT_TAKEN,
                };
                let record = assembler
                    .items
                    .iter_mut()
                    .find_map(|item| match item {
                        Item::Instruction(record) if record.opcode == wanted => Some(record),
                        _ => None,
                    })
                    .unwrap();
                match mode {
                    15 => record.set_operand(Operand::Value(0)),
                    16 => record.local_policy.add_hidden_exception_owner_slots(1),
                    17 => record.set_operand(Operand::Value(4)),
                    18 => record.location.column += 1,
                    19 => record.jump_policy.prevent_borrow_threading_target(),
                    _ => record.set_exception_policy(ExceptionPolicy::NoneUnchanged),
                }
            }
            21 => {
                assembler
                    .exception_owner_provenance
                    .copied_nested_try_manager_returns[0]
                    .regions = None;
            }
            22 | 23 => {}
            _ => unreachable!(),
        }
        assert_eq!(
            replay(&assembler, if mode == 22 { 4 } else { 5 }),
            matches!(mode, 0 | 1),
            "mode {mode}"
        );
    }
}

#[test]
fn nested_try_manager_mapping_and_binding_are_one_shot_and_complete() {
    for mode in 0..6 {
        let (mut assembler, return_item, pop_item) = fixture();
        capture(&mut assembler, return_item, pop_item);
        assembler.add_stopiteration_exception_region_with_depth(label(0), label(14), label(11), 0);
        if mode == 1 {
            assembler.exception_regions[2].target = label(15);
        }
        if mode == 2 {
            assembler.exception_regions[2].depth = 1;
        }
        assembler.bind_copied_nested_try_manager_returns();
        if matches!(mode, 1 | 2) {
            assert!(
                assembler
                    .exception_owner_provenance
                    .copied_nested_try_manager_returns
                    .is_empty()
            );
            continue;
        }
        if mode == 3 {
            assembler.bind_copied_nested_try_manager_returns();
            assert!(
                assembler
                    .exception_owner_provenance
                    .copied_nested_try_manager_returns
                    .is_empty()
            );
            continue;
        }
        let origin = &mut assembler
            .exception_owner_provenance
            .copied_nested_try_manager_returns[0];
        let valid = origin.remap_constants(
            if mode == 4 {
                &[]
            } else if mode == 5 {
                &[None]
            } else {
                &[Some(3)]
            },
            None,
        );
        assert_eq!(valid, mode < 4, "mode {mode}");
    }
}

#[test]
fn nested_try_manager_entry_receipt_covers_earlier_aliases_and_incoming_edges() {
    for mode in 0..5 {
        let (mut assembler, return_item, pop_item) = fixture();
        let at = assembler
            .linear_items()
            .iter()
            .position(|item| *item == Item::Label(label(8)))
            .unwrap();
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            panic!()
        };
        items.insert(at, Item::Label(label(15)));
        capture(&mut assembler, return_item + 1, pop_item + 1);
        bind(&mut assembler);
        let mut assembler = layout(assembler);
        let at = assembler
            .items
            .iter()
            .position(|item| *item == Item::Label(label(15)))
            .unwrap();
        match mode {
            0 => {}
            1 => assembler.items.insert(
                0,
                Item::Instruction(Instruction::new(
                    crate::target::opcodes::JUMP_FORWARD,
                    Operand::Forward(label(15)),
                    SourceLocation::NONE,
                    Some(2),
                )),
            ),
            2 => assembler.items.insert(at, Item::Label(label(16))),
            3 => assembler.items.swap(at, at + 1),
            4 => {
                assembler.items.remove(at);
            }
            _ => unreachable!(),
        }
        assert_eq!(replay(&assembler, 5), mode == 0, "mode {mode}");
    }
}
