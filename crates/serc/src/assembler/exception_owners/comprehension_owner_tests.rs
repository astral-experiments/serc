use super::{OwnerInstructionPositions, UniqueLabelPositions};
use crate::assembler::ir::{EmissionStreamId, ExceptionPolicy, UnoptimizedCode};
use crate::assembler::{Assembler, EmittedJump, Item, Label, Operand, SourceLocation};
use crate::target::opcodes::*;

fn fixture(shape: &str, remap: bool) -> (Assembler, Label, EmittedJump) {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let source = assembler.label();
    let source_alias = assembler.label();
    let cleanup_end = assembler.label();
    let entry = assembler.label();
    let end = assembler.label();
    assembler.mark(source);
    assembler.mark(source_alias);
    let jump =
        assembler.emit_forward_with_depth(JUMP_FORWARD, entry, 0, ExceptionPolicy::default());
    assembler.record_original_comprehension_normal_exit(jump, entry);
    if remap {
        assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
        assert!(assembler.take_trailing_nop_location().is_some());
    }
    assembler.emit(NOP, 0);
    assembler.mark(cleanup_end);
    assembler.mark(entry);
    assembler.set_location(SourceLocation::new(2, 2, 13, 41));
    assembler.emit(SWAP, 2);
    assembler.emit(STORE_FAST, 0);
    assembler
        .required_last_instruction_mut("test restore")
        .local_policy
        .mark_maybe_null_comprehension_restore();
    assembler.record_original_comprehension_normal_restores(entry);
    assembler.record_comprehension_restore_owner_prefix(entry);
    assembler.emit(LOAD_CONST, 0);
    if shape == "conditional" {
        assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, end, 0, ExceptionPolicy::default());
    } else {
        if shape == "five slots" {
            assembler.emit(NOP, 0);
            assembler.emit(LOAD_CONST, 0);
        }
        assembler.emit(RETURN_VALUE, 0);
    }
    assembler.mark(end);
    assembler.emit(LOAD_CONST, 0);
    assembler.emit(RETURN_VALUE, 0);
    (assembler, entry, jump)
}

fn late_entries(assembler: &Assembler) -> Vec<Label> {
    let items = assembler.linear_items();
    let indexed = items
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    let labels = UniqueLabelPositions::new(items);
    let positions = OwnerInstructionPositions::default();
    let provenance = &assembler.exception_owner_provenance;
    let indexed_result =
        provenance.comprehension_owner_entries::<true>(items, &indexed, &labels, &positions);
    assert_eq!(
        indexed_result,
        provenance.comprehension_owner_entries::<false>(items, &indexed, &labels, &positions)
    );
    indexed_result.into_iter().collect()
}

#[test]
fn comprehension_owner_entries_bind_completed_original_sources() {
    for shape in ["four slots", "five slots", "conditional"] {
        for mutation in [
            "intact",
            "source location",
            "source target",
            "source policy",
            "source moved",
            "source moved and replaced",
            "source alias",
            "source alias order",
            "duplicate source alias",
            "stale stream",
            "remapped stream",
            "entry alias",
            "entry alias order",
            "duplicate entry",
            "restore location",
            "restore local",
            "restore policy",
            "restore interior alias",
            "unsealed",
            "duplicate origin",
        ] {
            let (mut assembler, entry, jump) = fixture(shape, mutation == "remapped stream");
            let other = assembler.label();
            let entry_index = assembler
                .linear_items()
                .iter()
                .position(|item| *item == Item::Label(entry))
                .unwrap();
            match mutation {
                "source location" | "source target" | "source policy" => {
                    let Item::Instruction(source) =
                        &mut assembler.linear_items_mut()[jump.instruction().item_index as usize]
                    else {
                        panic!()
                    };
                    match mutation {
                        "source location" => source.location = SourceLocation::new(9, 9, 0, 1),
                        "source target" => source.set_operand(Operand::Forward(other)),
                        _ => source.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    }
                }
                "source moved" => assembler.linear_items_mut().swap(2, 3),
                "source moved and replaced" => {
                    // The original survives elsewhere while an identical record replaces it.
                    assembler.linear_items_mut()[3] = assembler.linear_items()[2];
                }
                "source alias" => assembler.linear_items_mut()[0] = Item::Label(other),
                "source alias order" => assembler.linear_items_mut().swap(0, 1),
                "duplicate source alias" => {
                    assembler.linear_items_mut()[3] = assembler.linear_items()[0];
                }
                "stale stream" => assembler.emission_stream = EmissionStreamId::next(),
                "entry alias" => assembler.linear_items_mut()[entry_index - 1] = Item::Label(other),
                "entry alias order" => assembler
                    .linear_items_mut()
                    .swap(entry_index - 1, entry_index),
                "duplicate entry" => assembler.linear_items_mut()[3] = Item::Label(entry),
                "restore location" | "restore local" | "restore policy" => {
                    let Item::Instruction(restore) =
                        &mut assembler.linear_items_mut()[entry_index + 2]
                    else {
                        panic!()
                    };
                    match mutation {
                        "restore location" => restore.location = SourceLocation::NONE,
                        "restore local" => restore.set_operand(Operand::Value(2)),
                        _ => restore.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    }
                }
                "restore interior alias" => {
                    assembler.linear_items_mut()[entry_index + 1] = Item::Label(other);
                }
                "unsealed" => assembler
                    .exception_owner_provenance
                    .comprehension_normal_exit_sources
                    .get_mut(&entry)
                    .unwrap()
                    .entry_aliases
                    .clear(),
                "duplicate origin" => {
                    let original = assembler
                        .exception_owner_provenance
                        .original_comprehension_normal_exits[0];
                    assembler
                        .exception_owner_provenance
                        .original_comprehension_normal_exits
                        .push(original);
                }
                _ => {}
            }
            let boundaries = assembler
                .exception_owner_provenance
                .block_boundaries
                .clone();
            let source_boundaries = assembler
                .exception_owner_provenance
                .source_boundaries
                .clone();
            assembler.resolve_comprehension_owner_entries();
            let origin = &assembler
                .exception_owner_provenance
                .comprehension_normal_exit_sources[&entry];
            let accepted =
                shape != "four slots" && matches!(mutation, "intact" | "remapped stream");
            assert_eq!(
                origin.owner_entry_certified, accepted,
                "{shape}: {mutation}"
            );
            assert!(origin.source.is_none());
            assert_eq!(
                late_entries(&assembler),
                if accepted { vec![entry] } else { vec![] },
                "{shape}: {mutation}"
            );
            assert_eq!(
                assembler.exception_owner_provenance.block_boundaries,
                boundaries
            );
            assert_eq!(
                assembler.exception_owner_provenance.source_boundaries,
                source_boundaries
            );
            assert!(assembler.borrow_unreachable_blocks.is_empty());
            assert!(
                assembler
                    .exception_owner_provenance
                    .borrow_only_block_boundaries
                    .is_empty()
            );
        }
    }
}

#[test]
fn comprehension_owner_entries_revalidate_surviving_aliases_and_restores() {
    for mutation in [
        "intact",
        "merged source aliases",
        "foreign merged alias",
        "restore moved",
        "restore duplicate",
        "duplicate source alias",
        "missing source alias",
        "duplicate entry",
        "retired entry",
        "restore location",
        "restore policy",
        "entry alias order",
    ] {
        let (mut assembler, entry, _) = fixture("five slots", false);
        assembler.resolve_comprehension_owner_entries();
        assert_eq!(late_entries(&assembler), [entry]);
        let other = assembler.label();
        let entry_index = assembler
            .linear_items()
            .iter()
            .position(|item| *item == Item::Label(entry))
            .unwrap();
        let restore_index = entry_index + 2;
        match mutation {
            "merged source aliases" | "foreign merged alias" => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    panic!()
                };
                items.drain(2..4);
                if mutation == "foreign merged alias" {
                    items.insert(2, Item::Label(other));
                }
            }
            "restore moved" => assembler.linear_items_mut().swap(restore_index, 3),
            "restore duplicate" => {
                assembler.linear_items_mut()[3] = assembler.linear_items()[restore_index];
            }
            "duplicate source alias" => {
                assembler.linear_items_mut()[3] = assembler.linear_items()[0];
            }
            "missing source alias" => assembler.linear_items_mut()[0] = Item::Label(other),
            "duplicate entry" => assembler.linear_items_mut()[3] = Item::Label(entry),
            "retired entry" => {
                assembler
                    .exception_owner_provenance
                    .retired_comprehension_normal_entries
                    .insert(entry);
            }
            "restore location" | "restore policy" => {
                let Item::Instruction(restore) = &mut assembler.linear_items_mut()[restore_index]
                else {
                    panic!()
                };
                if mutation == "restore location" {
                    restore.location = SourceLocation::NONE;
                } else {
                    restore.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                }
            }
            "entry alias order" => assembler
                .linear_items_mut()
                .swap(entry_index - 1, entry_index),
            _ => {}
        }
        let accepted = matches!(mutation, "intact" | "merged source aliases");
        assert_eq!(
            late_entries(&assembler),
            if accepted { vec![entry] } else { vec![] },
            "{mutation}"
        );
    }
}

#[test]
fn comprehension_owner_entries_resolve_before_prepared_reentry() {
    let (mut assembler, entry, _) = fixture("five slots", false);
    assert_eq!(assembler.used_constant_indices(1), [true]);
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    assert!(
        assembler
            .exception_owner_provenance
            .comprehension_normal_exit_sources[&entry]
            .owner_entry_certified
    );
    assert!(
        assembler
            .exception_owner_provenance
            .comprehension_normal_exit_sources[&entry]
            .source
            .is_none()
    );
    assert_eq!(assembler.used_constant_indices(1), [true]);
    assembler.remap_constant_indices(&[Some(1)]);
    assert_eq!(assembler.used_constant_indices(2), [false, true]);
}

#[test]
fn comprehension_owner_entries_index_duplicate_foreign_source_records() {
    for duplicate in [false, true] {
        let (mut assembler, entry, _) = fixture("five slots", false);
        let original = assembler
            .exception_owner_provenance
            .original_comprehension_normal_exits[0]
            .1;
        // Four source signatures and a larger stream exercise the shared lookup index.
        for _ in 0..3 {
            let target = assembler.label();
            let mut source = original;
            source.set_operand(Operand::Forward(target));
            assembler
                .exception_owner_provenance
                .original_comprehension_normal_exits
                .push((target, source));
        }
        for _ in 0..32 {
            assembler.emit(NOP, 0);
        }
        if duplicate {
            *assembler.required_last_instruction_mut("foreign source") = original;
        }
        assembler.resolve_comprehension_owner_entries();
        assert_eq!(
            assembler
                .exception_owner_provenance
                .comprehension_normal_exit_sources[&entry]
                .owner_entry_certified,
            !duplicate
        );
    }
}
