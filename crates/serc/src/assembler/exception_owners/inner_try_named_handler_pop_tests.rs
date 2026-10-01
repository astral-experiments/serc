use super::OwnerLabelPositions;
use crate::assembler::ir::FusionPolicy;
use crate::assembler::{
    Assembler, BorrowFallthrough, ExceptionPolicy, Instruction, Item, Label, LayoutAssembler,
    Operand, SourceLocation,
};
use crate::target::opcodes::{LOAD_FAST, NOP, POP_EXCEPT, RERAISE, RETURN_VALUE, SWAP};

struct Fixture {
    emitted: Assembler,
    start: usize,
    records: [usize; 5],
    entry: Label,
    aliases: [Label; 3],
    end: Label,
    handler: Label,
}

fn fixture() -> Fixture {
    let mut emitted = Assembler::with_python_version(crate::PythonVersion::Py314);
    let entry = emitted.label();
    let aliases = [emitted.label(), emitted.label(), emitted.label()];
    let end = emitted.label();
    let handler = emitted.label();
    emitted.mark(entry);
    emitted.record_exception_owner_block_boundary(entry);
    emitted.set_location(SourceLocation::new(8, 8, 20, 35));
    emitted.emit_with_depth(LOAD_FAST, 0, 2, ExceptionPolicy::default());
    let start = emitted.item_count();
    let mut records = [0; 5];
    for (index, (opcode, operand, depth)) in [
        (NOP, 0, 2),
        (NOP, 0, 2),
        (SWAP, 2, 2),
        (NOP, 0, 2),
        (POP_EXCEPT, 0, 1),
    ]
    .into_iter()
    .enumerate()
    {
        if index >= 2 {
            emitted.mark(aliases[index - 2]);
        }
        records[index] = emitted.item_count();
        emitted.emit_with_depth(
            opcode,
            operand,
            depth,
            if opcode == NOP {
                ExceptionPolicy::EXCLUDE_ALWAYS
            } else {
                ExceptionPolicy::default()
            },
        );
        if opcode == NOP {
            emitted.mark_last_as_converted_pop_block();
        }
    }
    Fixture {
        emitted,
        start,
        records,
        entry,
        aliases,
        end,
        handler,
    }
}

fn capture(fixture: &mut Fixture) -> usize {
    fixture
        .emitted
        .record_inner_try_named_handler_pop_span(fixture.start);
    fixture
        .emitted
        .exception_owner_provenance
        .inner_try_named_handler_pop_spans
        .len()
}

fn compact(mut fixture: Fixture) -> LayoutAssembler {
    fixture.emitted.mark(fixture.end);
    fixture
        .emitted
        .emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
    fixture.emitted.mark(fixture.handler);
    fixture
        .emitted
        .emit_with_depth(RERAISE, 0, 0, ExceptionPolicy::default());
    fixture.emitted.add_exception_region_with_origin(
        fixture.entry,
        fixture.end,
        fixture.handler,
        0,
        false,
        (fixture.entry, fixture.end),
    );
    let mut pipeline = fixture.emitted.into_optimization_pipeline();
    pipeline.remove_redundant_nops();
    LayoutAssembler {
        python_version: pipeline.python_version,
        legacy_uniform_jump_seed: pipeline.legacy_uniform_jump_seed,
        items: pipeline.graph.copy_items(),
        exception_regions: pipeline.exception_regions,
        exception_owner_provenance: pipeline.exception_owner_provenance,
        preserved_block_boundaries: pipeline.preserved_block_boundaries,
        removed_max_depth: None,
        next_label: pipeline.next_label,
        has_next_location: pipeline.has_next_location,
    }
}

fn replay(layout: &LayoutAssembler, targets: &rustc_hash::FxHashSet<Label>) -> bool {
    let mut positions = OwnerLabelPositions::new(layout.items.len());
    let mut index = 0;
    for item in &layout.items {
        match item {
            Item::Label(label) => {
                positions.insert(*label, index);
            }
            Item::Instruction(_) => index += 1,
        }
    }
    let (slots, aliases, donors) = layout.captured_pop_slots(targets, &positions);
    assert!(donors.is_empty());
    if slots.is_empty() {
        assert!(aliases.is_empty());
        false
    } else {
        assert_eq!(slots.len(), 2);
        assert_eq!(slots.get(&1), Some(&0));
        assert_eq!(slots.get(&2), Some(&0));
        assert_eq!(aliases.len(), 3);
        true
    }
}

fn mutate(record: &mut Instruction, mutation: &str) {
    match mutation {
        "opcode" => record.opcode = LOAD_FAST,
        "operand" => record.set_operand(Operand::Value(99)),
        "location" => record.location.column += 1,
        "policy" => record.set_exception_policy(ExceptionPolicy::NonePresent),
        "hidden" => record.local_policy.add_hidden_exception_owner_slots(1),
        "fold" => record.add_pre_fold_extra_instructions(1),
        "borrow" => record.jump_policy.mark_borrow_empty_block(),
        "barrier" => record
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier),
        "depth" => {
            let mut changed =
                Instruction::new(record.opcode, record.operand(), record.location, Some(99));
            changed.set_exception_policy(record.exception_policy());
            changed.fusion_policy = record.fusion_policy;
            changed.local_policy = record.local_policy;
            changed.jump_policy = record.jump_policy;
            changed.no_location_policy = record.no_location_policy;
            *record = changed;
        }
        _ => unreachable!(),
    }
}

const MUTATIONS: [&str; 9] = [
    "opcode", "operand", "location", "policy", "hidden", "fold", "borrow", "barrier", "depth",
];

#[test]
fn inner_try_named_handler_span_replays_real_three_pop_compaction() {
    let mut fixture = fixture();
    assert_eq!(capture(&mut fixture), 1);
    let layout = compact(fixture);
    let records = layout
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(record) => Some(record),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        records
            .iter()
            .map(|record| record.opcode)
            .collect::<Vec<_>>(),
        [LOAD_FAST, SWAP, POP_EXCEPT, RETURN_VALUE, RERAISE]
    );
    assert_eq!(records[1].local_policy.hidden_exception_owner_slots(), 2);
    assert_eq!(records[2].local_policy.hidden_exception_owner_slots(), 1);
    assert!(replay(&layout, &rustc_hash::FxHashSet::default()));
}

#[test]
fn inner_try_named_handler_capture_rejects_changed_records() {
    for index in 0..5 {
        for mutation in MUTATIONS {
            let mut fixture = fixture();
            let Item::Instruction(record) =
                &mut fixture.emitted.linear_items_mut()[fixture.records[index]]
            else {
                unreachable!()
            };
            mutate(record, mutation);
            assert_eq!(capture(&mut fixture), 0, "record {index}/{mutation}");
        }
    }
}

#[test]
fn inner_try_named_handler_capture_requires_complete_target_span() {
    for mutation in [
        "missing",
        "extra",
        "ordinary nop",
        "unknown depth",
        "zero depth",
        "missing entry",
        "312",
        "313",
        "315",
    ] {
        let mut fixture = fixture();
        match mutation {
            "missing" => {
                fixture.start = fixture.records[1];
            }
            "extra" => {
                fixture
                    .emitted
                    .emit_with_depth(NOP, 0, 1, ExceptionPolicy::default());
            }
            "ordinary nop" => {
                let Item::Instruction(record) =
                    &mut fixture.emitted.linear_items_mut()[fixture.records[0]]
                else {
                    unreachable!()
                };
                record.fusion_policy = FusionPolicy::default();
            }
            "unknown depth" | "zero depth" => {
                let Item::Instruction(record) =
                    &mut fixture.emitted.linear_items_mut()[fixture.records[2]]
                else {
                    unreachable!()
                };
                *record = Instruction::new(
                    SWAP,
                    Operand::Value(2),
                    record.location,
                    (mutation == "zero depth").then_some(0),
                );
            }
            "missing entry" => fixture
                .emitted
                .exception_owner_provenance
                .block_boundaries
                .clear(),
            "312" => fixture.emitted.python_version = crate::PythonVersion::Py312,
            "313" => fixture.emitted.python_version = crate::PythonVersion::Py313,
            "315" => fixture.emitted.python_version = crate::PythonVersion::Py315,
            _ => unreachable!(),
        }
        assert_eq!(capture(&mut fixture), 0, "{mutation}");
    }
}

#[test]
fn inner_try_named_handler_replay_requires_unchanged_live_records() {
    for opcode in [SWAP, POP_EXCEPT] {
        for mutation in MUTATIONS {
            let mut fixture = fixture();
            assert_eq!(capture(&mut fixture), 1);
            let mut layout = compact(fixture);
            let record = layout
                .items
                .iter_mut()
                .find_map(|item| match item {
                    Item::Instruction(record) if record.opcode == opcode => Some(record),
                    _ => None,
                })
                .unwrap();
            mutate(record, mutation);
            assert!(
                !replay(&layout, &rustc_hash::FxHashSet::default()),
                "{opcode:?}/{mutation}"
            );
        }
    }
}

#[test]
fn inner_try_named_handler_replay_rejects_changed_labels_and_arrays() {
    for mutation in [
        "missing capture",
        "missing entry",
        "duplicate entry",
        "duplicate alias",
        "missing alias",
        "reordered aliases",
        "unknown alias",
        "jump target",
        "source boundary",
        "block boundary",
        "preserved boundary",
        "setup",
        "exception target",
    ] {
        let mut fixture = fixture();
        assert_eq!(capture(&mut fixture), 1);
        let entry = fixture.entry;
        let [alias, second, _] = fixture.aliases;
        let mut layout = compact(fixture);
        let mut targets = rustc_hash::FxHashSet::default();
        let at = layout
            .items
            .iter()
            .position(|item| *item == Item::Label(alias))
            .unwrap();
        match mutation {
            "missing capture" => layout
                .exception_owner_provenance
                .inner_try_named_handler_pop_spans
                .clear(),
            "missing entry" => {
                layout
                    .exception_owner_provenance
                    .block_boundaries
                    .remove(&entry);
            }
            "duplicate entry" => layout.items.push(Item::Label(entry)),
            "duplicate alias" => layout.items.push(Item::Label(alias)),
            "missing alias" => {
                layout.items.remove(at);
            }
            "reordered aliases" => {
                let other = layout
                    .items
                    .iter()
                    .position(|item| *item == Item::Label(second))
                    .unwrap();
                layout.items.swap(at, other);
            }
            "unknown alias" => {
                layout
                    .items
                    .insert(at + 1, Item::Label(Label::from_u32(layout.next_label)));
                layout.next_label += 1;
            }
            "jump target" => {
                targets.insert(alias);
            }
            "source boundary" => {
                layout
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(alias);
            }
            "block boundary" => {
                layout
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(alias);
            }
            "preserved boundary" => {
                layout.preserved_block_boundaries.insert(alias);
            }
            "setup" => layout.exception_owner_provenance.setups.push((alias, None)),
            "exception target" => layout.exception_regions[0].target = alias,
            _ => unreachable!(),
        }
        assert!(!replay(&layout, &targets), "{mutation}");
    }
}
