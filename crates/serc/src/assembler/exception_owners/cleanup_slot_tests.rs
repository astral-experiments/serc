use crate::assembler::{
    Assembler, ExceptionPolicy, ExceptionRegionPriority, Instruction, Item, Label, LayoutAssembler,
    Operand, SourceLocation,
};
use crate::target::opcodes::{
    CALL, DELETE_FAST, LOAD_CONST, LOAD_FAST, LOAD_FAST_BORROW, LOAD_GLOBAL, NOP, NOT_TAKEN,
    POP_EXCEPT, POP_JUMP_IF_NONE, POP_TOP, RERAISE, STORE_FAST, SWAP,
};

struct Fixture {
    assembler: Assembler,
    entry: Label,
    frontier: Label,
    end: Label,
    owner: Label,
    foreign: Label,
    return_item: usize,
    pop_item: usize,
    calls: usize,
    named: bool,
}

fn manager_sequence_fixture(managers: usize) -> Fixture {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let entry = assembler.label();
    let frontier = assembler.label();
    let end = assembler.label();
    let owner = assembler.label();
    let foreign = assembler.label();
    assembler.mark(entry);
    assembler.record_exception_owner_block_boundary(entry);
    for index in 0..managers {
        if index == 1 {
            assembler.mark(foreign);
        }
        let depth = 2 * u32::try_from(managers - index).unwrap() + 1;
        let line = i32::try_from(index + 3).unwrap();
        assembler.set_location(SourceLocation::new(line, line, 4, 12));
        for (opcode, argument, depth) in [
            (SWAP, 3, depth),
            (SWAP, 2, depth),
            (LOAD_CONST, 0, depth + 1),
            (LOAD_CONST, 0, depth + 2),
            (LOAD_CONST, 0, depth + 3),
            (CALL, 3, depth - 1),
            (POP_TOP, 0, depth - 2),
        ] {
            assembler.emit_with_depth(opcode, argument, depth, ExceptionPolicy::default());
        }
    }
    assembler.mark(frontier);
    let pop_item = assembler.item_count();
    assembler.emit_with_depth(NOP, 0, 1, ExceptionPolicy::EXCLUDE_ALWAYS);
    assembler.mark_last_as_converted_pop_block();
    assembler.set_location(SourceLocation::new(12, 12, 7, 11));
    assembler.emit_with_depth(LOAD_FAST, 0, 2, ExceptionPolicy::default());
    let condition =
        assembler.emit_forward_with_depth(POP_JUMP_IF_NONE, end, 1, ExceptionPolicy::default());
    assembler
        .emitted_jump_mut(condition, "preparing manager return")
        .local_policy
        .add_hidden_exception_owner_slots(3);
    assembler.emit_with_depth(NOT_TAKEN, 0, 1, ExceptionPolicy::NonePresent);
    assembler.mark(end);
    assembler.mark(owner);
    assembler.emit_with_depth(RERAISE, 0, 0, ExceptionPolicy::default());
    assembler.add_exception_region_with_origin(entry, frontier, owner, 0, false, (entry, frontier));
    assembler.record_copied_manager_return_compaction(pop_item, 0, managers, None, false);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .copied_manager_return_compactions
            .len(),
        1
    );
    Fixture {
        assembler,
        entry,
        frontier,
        end,
        owner,
        foreign,
        return_item: 0,
        pop_item,
        calls: 0,
        named: false,
    }
}

#[test]
fn manager_return_cleanup_sequence_requires_complete_identity() {
    for managers in 3..=6 {
        for mutation in [
            "intact",
            "borrowed",
            "missing origin",
            "duplicate origin",
            "earlier operand",
            "earlier location",
            "earlier hidden",
            "missing group",
            "reversed groups",
            "source boundary",
            "block boundary",
            "jump target",
            "condition",
            "count",
            "fallback",
            "lasti",
            "short owner",
        ] {
            let (mut layout, _, _, end, foreign, _) = compact(manager_sequence_fixture(managers));
            let provenance = &mut layout.exception_owner_provenance;
            match mutation {
                "missing origin" => provenance.copied_manager_return_compactions.clear(),
                "duplicate origin" => provenance.copied_manager_return_compactions.push(
                    manager_sequence_fixture(managers)
                        .assembler
                        .exception_owner_provenance
                        .copied_manager_return_compactions
                        .remove(0),
                ),
                "missing group" => {
                    provenance.copied_manager_return_compactions[0]
                        .earlier_cleanup
                        .pop();
                }
                "reversed groups" => provenance.copied_manager_return_compactions[0]
                    .earlier_cleanup
                    .reverse(),
                "source boundary" => {
                    provenance.source_boundaries.insert(foreign);
                }
                "block boundary" => {
                    provenance.block_boundaries.insert(foreign);
                }
                "fallback" => layout.exception_regions[0].is_scope_fallback = true,
                "lasti" => layout.exception_regions[0].preserve_lasti = true,
                "short owner" => layout.exception_regions[0].end = foreign,
                _ => {}
            }
            let mut index = 0;
            for item in &mut layout.items {
                let Item::Instruction(record) = item else {
                    continue;
                };
                if index == 0 {
                    match mutation {
                        "earlier operand" => record.set_operand(Operand::Value(2)),
                        "earlier location" => record.location.column += 1,
                        "earlier hidden" => record.local_policy.add_hidden_exception_owner_slots(1),
                        _ => {}
                    }
                }
                if mutation == "borrowed" && record.opcode == LOAD_FAST {
                    record.opcode = LOAD_FAST_BORROW;
                }
                if mutation == "condition" && record.opcode == POP_JUMP_IF_NONE {
                    record.set_operand(Operand::Forward(foreign));
                }
                index += 1;
            }
            let mut position = 0;
            let positions = layout
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some((*label, position)),
                    Item::Instruction(_) => {
                        position += 1;
                        None
                    }
                })
                .collect::<rustc_hash::FxHashMap<_, _>>();
            let positions = super::OwnerLabelPositions::from_hashed(layout.items.len(), &positions);
            let indexed = layout
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(record) => Some(record),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let marker = indexed
                .iter()
                .position(|record| record.opcode == NOT_TAKEN)
                .unwrap();
            let mut targets = [end].into_iter().collect::<rustc_hash::FxHashSet<_>>();
            if mutation == "jump target" {
                targets.insert(foreign);
            }
            let decision = layout.copied_manager_return_compaction_owner(
                marker,
                0,
                super::ExceptionOwnerTail {
                    early_removals: if mutation == "count" { 5 } else { 4 },
                    ..super::ExceptionOwnerTail::default()
                },
                &indexed,
                &positions,
                &targets,
                false,
            );
            assert_eq!(
                decision.is_some(),
                matches!(mutation, "intact" | "borrowed"),
                "{managers} {mutation}"
            );
            if let Some(decision) = decision {
                assert_eq!(decision.source_region(), Some(0));
            }
        }
    }
}

fn fixture(named: bool, calls: usize) -> Fixture {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let entry = assembler.label();
    let frontier = assembler.label();
    let end = assembler.label();
    let owner = assembler.label();
    let foreign = assembler.label();
    assembler.mark(entry);
    assembler.record_exception_owner_block_boundary(entry);
    assembler.set_location(SourceLocation::new(5, 5, 12, 20));
    for _ in 0..calls {
        assembler.emit_with_depth(LOAD_GLOBAL, 1, 4, ExceptionPolicy::default());
        assembler.emit_with_depth(CALL, 0, 3, ExceptionPolicy::default());
        if named {
            assembler.emit_with_depth(POP_TOP, 0, 2, ExceptionPolicy::default());
        }
    }
    assembler.set_location(SourceLocation::new(6, 6, 12, 18));
    let return_item = assembler.item_count();
    let cleanup = if named {
        vec![
            (NOP, 0, 2),
            (NOP, 0, 2),
            (NOP, 0, 2),
            (POP_EXCEPT, 0, 1),
            (LOAD_CONST, 0, 2),
            (STORE_FAST, 1, 1),
            (DELETE_FAST, 1, 1),
            (POP_TOP, 0, 0),
        ]
    } else {
        vec![
            (SWAP, 2, 4),
            (NOP, 0, 4),
            (POP_EXCEPT, 0, 3),
            (SWAP, 3, 3),
            (SWAP, 2, 3),
            (LOAD_CONST, 0, 4),
            (LOAD_CONST, 0, 5),
            (LOAD_CONST, 0, 6),
            (CALL, 3, 2),
            (POP_TOP, 0, 1),
        ]
    };
    for (index, (opcode, operand, depth)) in cleanup.into_iter().enumerate() {
        let converted = opcode == NOP && index != 0;
        assembler.emit_with_depth(
            opcode,
            operand,
            depth,
            if converted {
                ExceptionPolicy::EXCLUDE_ALWAYS
            } else {
                ExceptionPolicy::default()
            },
        );
        if converted {
            assembler.mark_last_as_converted_pop_block();
        }
    }
    assembler.mark(frontier);
    let pop_item = assembler.item_count();
    assembler.emit_with_depth(NOP, 0, u32::from(!named), ExceptionPolicy::EXCLUDE_ALWAYS);
    assembler.mark_last_as_converted_pop_block();
    assembler.set_location(SourceLocation::new(9, 9, 7, 10));
    assembler.emit_with_depth(
        LOAD_FAST,
        0,
        1 + u32::from(!named),
        ExceptionPolicy::default(),
    );
    assembler.set_location(SourceLocation::new(9, 9, 7, 22));
    let condition = assembler.emit_forward_with_depth(
        POP_JUMP_IF_NONE,
        end,
        u32::from(!named),
        ExceptionPolicy::default(),
    );
    assembler
        .emitted_jump_mut(condition, "preparing copied cleanup test")
        .local_policy
        .add_hidden_exception_owner_slots(3);
    assembler.emit_with_depth(
        NOT_TAKEN,
        0,
        u32::from(!named),
        ExceptionPolicy::NonePresent,
    );
    assembler.mark(end);
    assembler.mark(owner);
    assembler.emit_with_depth(RERAISE, 0, 0, ExceptionPolicy::default());
    assembler.add_exception_region_with_origin(entry, frontier, owner, 0, false, (entry, frontier));
    Fixture {
        assembler,
        entry,
        frontier,
        end,
        owner,
        foreign,
        return_item,
        pop_item,
        calls,
        named,
    }
}

fn capture(fixture: &mut Fixture) -> usize {
    if fixture.named {
        fixture.assembler.record_copied_named_iterator_none_prefix(
            fixture.return_item,
            fixture.pop_item,
            fixture.owner,
        );
        fixture
            .assembler
            .exception_owner_provenance
            .copied_cleanup_return_none_prefixes
            .len()
    } else {
        fixture.assembler.record_saved_handler_manager_return_none(
            fixture.pop_item,
            0,
            fixture.owner,
        );
        fixture
            .assembler
            .exception_owner_provenance
            .copied_manager_return_compactions
            .len()
    }
}

fn compact(fixture: Fixture) -> (LayoutAssembler, Label, Label, Label, Label, usize) {
    let mut items = Vec::new();
    let mut removed = 0;
    for item in fixture.assembler.linear_items() {
        let mut item = *item;
        if let Item::Instruction(record) = &mut item {
            if record.opcode == NOP {
                removed += 1;
                continue;
            }
            record
                .local_policy
                .add_hidden_exception_owner_slots(removed);
            removed = 0;
        }
        items.push(item);
    }
    let mut provenance = fixture.assembler.exception_owner_provenance;
    provenance
        .removed_pop_block_boundaries
        .insert(fixture.frontier);
    provenance
        .removed_pop_block_widths
        .insert(fixture.frontier, 1);
    let mut regions = fixture.assembler.exception_regions;
    regions[0].priority = ExceptionRegionPriority::Resolved {
        start: 0,
        end: fixture.pop_item,
    };
    (
        LayoutAssembler {
            python_version: crate::PythonVersion::Py314,
            legacy_uniform_jump_seed: false,
            items,
            exception_regions: regions,
            exception_owner_provenance: provenance,
            preserved_block_boundaries: rustc_hash::FxHashSet::default(),
            removed_max_depth: None,
            next_label: fixture.foreign.as_u32() + 1,
            has_next_location: false,
        },
        fixture.entry,
        fixture.frontier,
        fixture.end,
        fixture.foreign,
        fixture.calls,
    )
}

#[test]
fn cleanup_slot_capture_requires_complete_unmodified_records() {
    for named in [false, true] {
        let length = if named { 8 } else { 10 };
        for index in 0..length {
            for mutation in [
                "intact", "depth", "operand", "policy", "hidden", "location", "fold", "barrier",
            ] {
                let mut fixture = fixture(named, 1);
                let record =
                    match &mut fixture.assembler.linear_items_mut()[fixture.return_item + index] {
                        Item::Instruction(record) => record,
                        Item::Label(_) => unreachable!(),
                    };
                match mutation {
                    "depth" => {
                        let Operand::Value(argument) = record.operand() else {
                            unreachable!()
                        };
                        let policy = record.exception_policy();
                        let fusion = record.fusion_policy;
                        *record = Instruction::value_with_depth(
                            record.opcode,
                            argument,
                            record.location,
                            99,
                        );
                        record.set_exception_policy(policy);
                        record.fusion_policy = fusion;
                    }
                    "operand" => record.set_operand(Operand::Value(99)),
                    "policy" => record.set_exception_policy(ExceptionPolicy::NonePresent),
                    "hidden" => record.local_policy.add_hidden_exception_owner_slots(1),
                    "location" => record.location.column += 1,
                    "fold" => record.add_pre_fold_extra_instructions(1),
                    "barrier" => record
                        .no_location_policy
                        .set_borrow_fallthrough(crate::assembler::BorrowFallthrough::Barrier),
                    _ => {}
                }
                let count = capture(&mut fixture);
                // Manager cells are captured verbatim by the existing recorder; source
                // selection fixes their stack context, and replay compares every field.
                let delegated = !named && index >= 3 && mutation == "location";
                let irrelevant_operand = named && index == 4 && mutation == "operand";
                assert_eq!(
                    count,
                    usize::from(mutation == "intact" || delegated || irrelevant_operand),
                    "named={named}/{index}/{mutation}"
                );
            }
        }
    }
}

#[test]
fn cleanup_slot_replay_rejects_foreign_state_and_ambiguous_origins() {
    for named in [false, true] {
        for mutation in [
            "intact",
            "duplicate origin",
            "missing origin",
            "changed live",
            "changed marker",
            "foreign owner",
            "foreign marker owner",
            "missing region end",
            "missing priority end",
            "depth",
            "lasti",
            "fallback",
            "source boundary",
            "jump target",
            "hidden count",
            "late fusion",
            "insertions",
            "copied entry",
            "retired entry",
            "internal entry",
            "missing entry",
            "duplicate entry",
        ] {
            let mut duplicate = fixture(named, 1);
            assert_eq!(capture(&mut duplicate), 1);
            let mut fixture = fixture(named, 1);
            assert_eq!(capture(&mut fixture), 1);
            let (mut layout, entry, frontier, end, foreign, calls) = compact(fixture);
            let provenance = &mut layout.exception_owner_provenance;
            match mutation {
                "duplicate origin" if named => provenance.copied_cleanup_return_none_prefixes.push(
                    duplicate
                        .assembler
                        .exception_owner_provenance
                        .copied_cleanup_return_none_prefixes
                        .remove(0),
                ),
                "duplicate origin" => provenance.copied_manager_return_compactions.push(
                    duplicate
                        .assembler
                        .exception_owner_provenance
                        .copied_manager_return_compactions
                        .remove(0),
                ),
                "missing origin" if named => provenance.copied_cleanup_return_none_prefixes.clear(),
                "missing origin" => provenance.copied_manager_return_compactions.clear(),
                "foreign owner" => layout.exception_regions[0].target = foreign,
                "foreign marker owner" => {
                    let mut region = layout.exception_regions[0];
                    region.start = frontier;
                    region.end = end;
                    layout.exception_regions.push(region);
                }
                "missing region end" => layout.exception_regions[0].end = foreign,
                "missing priority end" => {
                    layout.exception_regions[0].priority =
                        ExceptionRegionPriority::OriginalRange(entry, foreign);
                }
                "depth" => layout.exception_regions[0].depth = 1,
                "lasti" => layout.exception_regions[0].preserve_lasti = true,
                "fallback" => layout.exception_regions[0].is_scope_fallback = true,
                "source boundary" => {
                    provenance.source_boundaries.insert(frontier);
                }
                "copied entry" => {
                    provenance.normal_try_copy_blocks.insert(entry);
                }
                "retired entry" => {
                    provenance.retired_normal_try_copy_labels.insert(entry);
                }
                "internal entry" => {
                    provenance.normal_try_copy_internal_labels.push(entry);
                }
                "missing entry" => {
                    provenance.source_boundaries.remove(&entry);
                }
                "duplicate entry" => layout.items.insert(0, Item::Label(entry)),
                "changed marker" => {
                    let record = layout
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            Item::Instruction(record) if record.opcode == NOT_TAKEN => Some(record),
                            _ => None,
                        })
                        .unwrap();
                    record.location.column += 1;
                }
                "changed live" => {
                    let record = layout
                        .items
                        .iter_mut()
                        .filter_map(|item| match item {
                            Item::Instruction(record) => Some(record),
                            Item::Label(_) => None,
                        })
                        .find(|record| record.opcode == POP_EXCEPT)
                        .unwrap();
                    record.location.column += 1;
                }
                _ => {}
            }
            let mut position = 0;
            let positions = layout
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some((*label, position)),
                    Item::Instruction(_) => {
                        position += 1;
                        None
                    }
                })
                .collect::<rustc_hash::FxHashMap<_, _>>();
            let indexed = layout
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(record) => Some(record),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let marker = indexed
                .iter()
                .position(|record| record.opcode == NOT_TAKEN)
                .unwrap();
            let positions = super::OwnerLabelPositions::from_hashed(layout.items.len(), &positions);
            let mut targets = [end].into_iter().collect::<rustc_hash::FxHashSet<_>>();
            if mutation == "jump target" {
                targets.insert(frontier);
            }
            let mut tail = super::ExceptionOwnerTail {
                early_removals: calls + if named { 7 } else { 5 },
                ..super::ExceptionOwnerTail::default()
            };
            match mutation {
                "hidden count" => tail.early_removals += 1,
                "late fusion" => tail.post_compaction_fusions = 1,
                "insertions" => tail.insertions = 1,
                _ => {}
            }
            let decision = if named {
                layout.copied_cleanup_return_none_owner(
                    marker, 0, tail, &indexed, &positions, &targets,
                )
            } else {
                layout.copied_manager_return_compaction_owner(
                    marker, 0, tail, &indexed, &positions, &targets, false,
                )
            };
            // The manager resolver's caller already authenticates original arrays.
            let caller_owned = !named && mutation == "insertions";
            assert_eq!(
                decision.is_some(),
                mutation == "intact" || caller_owned,
                "named={named}/{mutation}"
            );
            if let Some(decision) = decision {
                assert_eq!(decision.source_region(), Some(0));
            }
        }
    }
}

#[test]
fn cleanup_slot_condition_transport_requires_one_actual_retarget() {
    for mutation in [
        "intact",
        "missing receipt",
        "duplicate receipt",
        "missing live",
        "duplicate live",
        "old live",
        "wrong source",
        "location",
        "hidden",
        "owner",
        "opcode",
        "duplicate origin",
        "old form",
    ] {
        let mut duplicate = fixture(true, 1);
        assert_eq!(capture(&mut duplicate), 1);
        let mut fixture = fixture(true, 1);
        assert_eq!(capture(&mut fixture), 1);
        let provenance = &mut fixture.assembler.exception_owner_provenance;
        let before = provenance.copied_cleanup_return_none_prefixes[0].condition;
        let mut after = before;
        after.retarget(Operand::Forward(fixture.foreign));
        match mutation {
            "location" => after.location.column += 1,
            "hidden" => after.local_policy.add_hidden_exception_owner_slots(1),
            "owner" => after.set_exception_policy(ExceptionPolicy::NonePresent),
            "opcode" => after.opcode = crate::target::opcodes::POP_JUMP_IF_FALSE,
            "duplicate origin" => provenance.copied_cleanup_return_none_prefixes.push(
                duplicate
                    .assembler
                    .exception_owner_provenance
                    .copied_cleanup_return_none_prefixes
                    .remove(0),
            ),
            "old form" => provenance.copied_cleanup_return_none_prefixes[0].named_iterator = None,
            _ => {}
        }
        let mut receipts = vec![(before, after)];
        let mut items = vec![Item::Instruction(after)];
        match mutation {
            "missing receipt" => receipts.clear(),
            "duplicate receipt" => receipts.push((before, after)),
            "missing live" => items.clear(),
            "duplicate live" => items.push(Item::Instruction(after)),
            "old live" => items.push(Item::Instruction(before)),
            "wrong source" => receipts[0].0.retarget(Operand::Forward(fixture.owner)),
            _ => {}
        }
        provenance.retarget_copied_cleanup_ordinary_try_conditions(&receipts, &items);
        assert_eq!(
            provenance.copied_cleanup_return_none_prefixes[0].condition,
            if mutation == "intact" { after } else { before },
            "{mutation}"
        );
    }
}
