use crate::assembler::ir::{
    BorrowFallthrough, EmissionStreamId, ExceptionPolicy, ExceptionRegionPriority,
};
use crate::assembler::{Assembler, Instruction, Item, Label, Operand, SourceLocation};
use crate::target::opcodes::*;
use crate::target::operands::BinaryOperation;

fn fixture(slots: usize, retained: bool) -> (Assembler, Label) {
    fixture_with_depth(slots, retained, 0)
}

fn fixture_with_depth(slots: usize, retained: bool, depth: u32) -> (Assembler, Label) {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let source = assembler.label();
    let source_alias = assembler.label();
    let cleanup_end = assembler.label();
    let entry = assembler.label();
    let nop_start = assembler.label();
    let nop_end = assembler.label();
    let setup = assembler.label();
    let end = assembler.label();
    let original_end = assembler.label();
    let handler = assembler.label();
    let done = assembler.label();
    assembler.mark(source);
    assembler.mark(source_alias);
    let jump =
        assembler.emit_forward_with_depth(JUMP_FORWARD, entry, 0, ExceptionPolicy::default());
    assembler.record_original_comprehension_normal_exit(jump, entry);
    assembler.emit(NOP, 0);
    assembler.mark(cleanup_end);
    assembler.mark(entry);
    assembler.set_location(SourceLocation::new(2, 2, 13, 41));
    assembler.emit(SWAP, 2);
    assembler.emit(STORE_FAST, 0);
    assembler
        .required_last_instruction_mut("normal restore")
        .local_policy
        .mark_maybe_null_comprehension_restore();
    assembler.record_original_comprehension_normal_restores(entry);
    assembler.record_comprehension_restore_owner_prefix(entry);
    for _ in 3..slots {
        assembler.emit(BINARY_OP, BinaryOperation::OR.argument());
    }
    assembler.emit_with_depth(STORE_FAST, 1, depth, ExceptionPolicy::default());
    if retained {
        assembler.mark(nop_start);
        assembler.set_location(SourceLocation::new(3, 6, 4, 21));
        assembler.emit_with_depth(NOP, 0, depth, ExceptionPolicy::EXCLUDE_ALWAYS);
        assembler.mark(nop_end);
        assembler.record_setup_nop_boundaries(nop_start, nop_end);
    }
    assembler.record_ordinary_try_exception_setup(setup, retained, handler);
    assembler.mark(setup);
    assembler.set_location(SourceLocation::new(4, 4, 7, 11));
    assembler.emit(LOAD_FAST, 2);
    assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, end, 0, ExceptionPolicy::default());
    assembler.emit(LOAD_CONST, 0);
    assembler.emit(POP_TOP, 0);
    assembler.mark(end);
    assembler.emit_forward_with_depth(JUMP_FORWARD, done, 0, ExceptionPolicy::default());
    assembler.mark(original_end);
    assembler.mark(handler);
    assembler.set_location(SourceLocation::new(6, 6, 4, 21));
    assembler.emit(NOP, 0);
    assembler.mark(done);
    assembler.emit(LOAD_CONST, 0);
    assembler.emit(RETURN_VALUE, 0);
    assembler.add_exception_region_with_origin(
        setup,
        end,
        handler,
        depth,
        false,
        (setup, original_end),
    );
    (assembler, entry)
}

fn certified(assembler: &Assembler, entry: Label) -> bool {
    assembler
        .exception_owner_provenance
        .comprehension_normal_exit_sources[&entry]
        .owner_entry_certified
}

#[test]
fn comprehension_setup_entries_bind_source_region_and_alias_identities() {
    for mutation in [
        "intact",
        "later nop remap",
        "stale stream",
        "stale nop stream",
        "wrong nop item",
        "nop moved",
        "nop duplicate",
        "nop opcode",
        "nop operand",
        "nop location",
        "nop depth",
        "nop exception policy",
        "nop jump policy",
        "nop local policy",
        "nop no-location policy",
        "prefix opcode",
        "prefix operand",
        "prefix location",
        "prefix policy",
        "prefix no-location policy",
        "foreign alias",
        "duplicate start",
        "duplicate nop end",
        "duplicate setup",
        "duplicate end",
        "duplicate original end",
        "duplicate handler",
        "missing start",
        "missing nop end",
        "missing setup",
        "missing end",
        "missing original end",
        "missing handler",
        "missing range",
        "duplicate range",
        "foreign range",
        "missing setup record",
        "duplicate setup record",
        "setup location",
        "wrong handler provenance",
        "missing origin",
        "duplicate origin",
        "foreign same-handler origin",
        "wrong original end",
        "wrong priority start",
        "resolved priority",
        "wrong region start",
        "wrong region end",
        "wrong region target",
        "wrong depth",
        "lasti",
        "fallback",
        "duplicate region",
        "unsealed region",
        "incoming start",
        "incoming end",
        "incoming setup",
        "exceptional incoming",
        "captured prefix",
        "captured handler",
    ] {
        let (mut assembler, entry) = fixture(4, true);
        let other = assembler.label();
        let original_region = assembler.exception_regions[0];
        let ExceptionRegionPriority::OriginalRange(_, original_end) = original_region.priority
        else {
            panic!()
        };
        let origin = assembler
            .exception_owner_provenance
            .comprehension_normal_exit_sources[&entry]
            .following_setup
            .as_ref()
            .unwrap();
        let (source_index, start, nop_end, setup, handler) = (
            origin.source.item_index as usize,
            origin.start,
            origin.end,
            origin.setup,
            origin.handler,
        );
        let original_nop = origin.instruction;
        let entry_index = assembler
            .linear_items()
            .iter()
            .position(|item| *item == Item::Label(entry))
            .unwrap();
        match mutation {
            "later nop remap" => {
                assembler.emit(NOP, 0);
                assert!(assembler.take_trailing_nop_location().is_some());
            }
            "stale stream" => assembler.emission_stream = EmissionStreamId::next(),
            "stale nop stream" | "wrong nop item" | "captured prefix" | "captured handler" => {
                let witness = assembler
                    .exception_owner_provenance
                    .comprehension_normal_exit_sources
                    .get_mut(&entry)
                    .unwrap()
                    .following_setup
                    .as_mut()
                    .unwrap();
                match mutation {
                    "stale nop stream" => witness.source.stream = EmissionStreamId::next(),
                    "wrong nop item" => witness.source.item_index -= 1,
                    "captured prefix" => witness.prefix[3].set_operand(Operand::Value(9)),
                    _ => witness.handler = other,
                }
            }
            "nop moved" => assembler.linear_items_mut().swap(source_index, 3),
            "nop duplicate" => assembler.linear_items_mut()[3] = Item::Instruction(original_nop),
            "nop opcode"
            | "nop operand"
            | "nop location"
            | "nop depth"
            | "nop exception policy"
            | "nop jump policy"
            | "nop local policy"
            | "nop no-location policy" => {
                let Item::Instruction(nop) = &mut assembler.linear_items_mut()[source_index] else {
                    panic!()
                };
                match mutation {
                    "nop opcode" => nop.opcode = LOAD_CONST,
                    "nop operand" => nop.set_operand(Operand::Value(1)),
                    "nop location" => nop.location = SourceLocation::NONE,
                    "nop depth" => {
                        let mut changed =
                            Instruction::new(nop.opcode, nop.operand(), nop.location, Some(1));
                        changed.set_exception_policy(nop.exception_policy());
                        *nop = changed;
                    }
                    "nop exception policy" => nop.set_exception_policy(ExceptionPolicy::default()),
                    "nop jump policy" => nop.jump_policy.preserve_explicit_nop(),
                    "nop no-location policy" => nop
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                    _ => nop.local_policy.mark_maybe_null_comprehension_restore(),
                }
            }
            "prefix opcode"
            | "prefix operand"
            | "prefix location"
            | "prefix policy"
            | "prefix no-location policy" => {
                let Item::Instruction(instruction) =
                    &mut assembler.linear_items_mut()[entry_index + 3]
                else {
                    panic!()
                };
                match mutation {
                    "prefix opcode" => instruction.opcode = BUILD_MAP,
                    "prefix operand" => instruction.set_operand(Operand::Value(1)),
                    "prefix location" => instruction.location = SourceLocation::NONE,
                    "prefix no-location policy" => instruction
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                    _ => instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                }
            }
            "foreign alias" => assembler.linear_items_mut()[source_index - 1] = Item::Label(other),
            "duplicate start"
            | "duplicate nop end"
            | "duplicate setup"
            | "duplicate end"
            | "duplicate handler"
            | "missing start"
            | "missing nop end"
            | "missing setup"
            | "missing end"
            | "missing handler"
            | "duplicate original end"
            | "missing original end" => {
                let label = match mutation {
                    "duplicate start" | "missing start" => start,
                    "duplicate nop end" | "missing nop end" => nop_end,
                    "duplicate setup" | "missing setup" => setup,
                    "duplicate end" | "missing end" => original_region.end,
                    "duplicate original end" | "missing original end" => original_end,
                    _ => handler,
                };
                if mutation.starts_with("duplicate") {
                    assembler.linear_items_mut()[3] = Item::Label(label);
                } else {
                    let index = assembler
                        .linear_items()
                        .iter()
                        .position(|item| *item == Item::Label(label))
                        .unwrap();
                    assembler.linear_items_mut()[index] = Item::Label(other);
                }
            }
            "missing range" => assembler
                .exception_owner_provenance
                .setup_nop_ranges
                .clear(),
            "duplicate range" => assembler
                .exception_owner_provenance
                .setup_nop_ranges
                .push((start, nop_end)),
            "foreign range" => {
                assembler.exception_owner_provenance.setup_nop_ranges[0] = (start, other);
            }
            "missing setup record" => assembler.exception_owner_provenance.setups.clear(),
            "duplicate setup record" => {
                let record = assembler.exception_owner_provenance.setups[0];
                assembler.exception_owner_provenance.setups.push(record);
            }
            "setup location" => assembler.exception_owner_provenance.setups[0].1 = None,
            "wrong handler provenance" => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(setup, other);
            }
            "missing origin" => assembler
                .exception_owner_provenance
                .ordinary_try_region_origins
                .clear(),
            "duplicate origin" => {
                let record = assembler
                    .exception_owner_provenance
                    .ordinary_try_region_origins[0];
                assembler
                    .exception_owner_provenance
                    .ordinary_try_region_origins
                    .push(record);
            }
            "wrong original end" => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::OriginalRange(setup, handler);
            }
            "foreign same-handler origin" => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_region_origins[0]
                    .0 = other;
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(other, handler);
            }
            "wrong priority start" => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::OriginalRange(start, original_region.end);
            }
            "resolved priority" => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::Resolved { start: 0, end: 1 }
            }
            "wrong region start" => assembler.exception_regions[0].start = start,
            "wrong region end" => assembler.exception_regions[0].end = handler,
            "wrong region target" => assembler.exception_regions[0].target = other,
            "wrong depth" => assembler.exception_regions[0].depth = 1,
            "lasti" => assembler.exception_regions[0].preserve_lasti = true,
            "fallback" => assembler.exception_regions[0].is_scope_fallback = true,
            "duplicate region" => assembler.exception_regions.push(original_region),
            "unsealed region" => {
                assembler
                    .exception_owner_provenance
                    .comprehension_normal_exit_sources
                    .get_mut(&entry)
                    .unwrap()
                    .following_setup
                    .as_mut()
                    .unwrap()
                    .region = None;
            }
            "incoming start" | "incoming end" | "incoming setup" => {
                let target = match mutation {
                    "incoming start" => start,
                    "incoming end" => nop_end,
                    _ => setup,
                };
                assembler.emit_forward_with_depth(
                    JUMP_FORWARD,
                    target,
                    0,
                    ExceptionPolicy::default(),
                );
            }
            "exceptional incoming" => {
                let mut region = original_region;
                region.start = start;
                region.target = nop_end;
                assembler.exception_regions.push(region);
            }
            _ => {}
        }
        assembler.resolve_comprehension_owner_entries();
        assert_eq!(
            certified(&assembler, entry),
            matches!(mutation, "intact" | "later nop remap"),
            "{mutation}"
        );
        assert!(
            assembler
                .exception_owner_provenance
                .comprehension_normal_exit_sources[&entry]
                .source
                .is_none()
        );
    }
}

#[test]
fn comprehension_setup_entries_preserve_slot_limits_and_prepared_lifecycle() {
    for slots in [3, 4, 5] {
        for retained in [false, true] {
            let (mut assembler, entry) = fixture(slots, retained);
            let captured = assembler
                .exception_owner_provenance
                .comprehension_normal_exit_sources[&entry]
                .following_setup
                .is_some();
            assert_eq!(captured, slots == 4 && retained);
            assembler.resolve_comprehension_owner_entries();
            assert_eq!(
                certified(&assembler, entry),
                slots == 5 || slots == 4 && retained
            );
        }
    }
    let (mut assembler, entry) = fixture(4, true);
    assert_eq!(assembler.used_constant_indices(1), [true]);
    assert!(certified(&assembler, entry));
    assembler.remap_constant_indices(&[Some(1)]);
    assert_eq!(assembler.used_constant_indices(2), [false, true]);
    assert!(certified(&assembler, entry));

    let (mut assembler, entry) = fixture(4, true);
    let region = assembler.exception_regions[0];
    assembler.exception_regions[0].depth = 1;
    assembler.resolve_comprehension_owner_entries();
    assert!(!certified(&assembler, entry));
    assembler.exception_regions[0] = region;
    assembler.resolve_comprehension_owner_entries();
    assert!(!certified(&assembler, entry));
}

#[test]
fn comprehension_setup_entries_index_duplicate_foreign_nop_records() {
    for duplicate in [false, true] {
        let (mut assembler, entry) = fixture(4, true);
        let original = assembler
            .exception_owner_provenance
            .original_comprehension_normal_exits[0]
            .1;
        let nop = assembler
            .exception_owner_provenance
            .comprehension_normal_exit_sources[&entry]
            .following_setup
            .as_ref()
            .unwrap()
            .instruction;
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
            *assembler.required_last_instruction_mut("foreign original NOP") = nop;
        }
        assembler.resolve_comprehension_owner_entries();
        assert_eq!(certified(&assembler, entry), !duplicate);
    }
}

#[test]
fn comprehension_setup_entries_capture_only_four_real_original_records() {
    for mutation in [
        "intact",
        "folded prefix",
        "hidden prefix",
        "barrier prefix",
        "nop prefix",
        "copy prefix",
        "global null prefix",
        "branch prefix",
        "scope exit prefix",
        "nop policy",
        "nop location",
        "nop no-location",
        "foreign prefix alias",
    ] {
        let (mut assembler, entry) = fixture(4, true);
        let witness = assembler
            .exception_owner_provenance
            .comprehension_normal_exit_sources
            .get_mut(&entry)
            .unwrap()
            .following_setup
            .take()
            .unwrap();
        let nop_index = witness.source.item_index as usize;
        let prefix_index = nop_index - 3;
        if mutation == "foreign prefix alias" {
            assembler.linear_items_mut()[prefix_index] = Item::Label(witness.handler);
        } else if mutation.starts_with("nop ") && mutation != "nop prefix" {
            let Item::Instruction(nop) = &mut assembler.linear_items_mut()[nop_index] else {
                panic!()
            };
            match mutation {
                "nop policy" => nop.set_exception_policy(ExceptionPolicy::default()),
                "nop location" => nop.location = SourceLocation::NONE,
                _ => nop
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier),
            }
        } else {
            let Item::Instruction(instruction) = &mut assembler.linear_items_mut()[prefix_index]
            else {
                panic!()
            };
            match mutation {
                "folded prefix" => instruction.add_pre_fold_extra_instructions(1),
                "hidden prefix" => instruction.local_policy.add_hidden_exception_owner_slots(1),
                "barrier prefix" => instruction
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier),
                "nop prefix" => instruction.opcode = NOP,
                "copy prefix" => instruction.opcode = COPY,
                "global null prefix" => {
                    instruction.opcode = LOAD_GLOBAL;
                    instruction.set_operand(Operand::Value(1));
                }
                "branch prefix" => {
                    instruction.opcode = JUMP_FORWARD;
                    instruction.set_operand(Operand::Forward(witness.handler));
                }
                "scope exit prefix" => instruction.opcode = RETURN_VALUE,
                _ => {}
            }
        }
        let captured = assembler
            .exception_owner_provenance
            .capture_comprehension_following_setup(
                &assembler.linear_items()[..nop_index + 2],
                assembler.emission_stream,
                witness.setup,
                witness.handler,
            );
        assert_eq!(captured.is_some(), mutation == "intact", "{mutation}");
    }
}

#[test]
fn comprehension_setup_entries_bind_positive_depth_to_the_original_frontier() {
    for depth in [1, 2, 3, 4] {
        for mutation in [
            "intact",
            "later nop remap",
            "current region depth",
            "captured region depth",
            "both region depths",
            "both nop depths",
            "both store depths",
            "both store opcodes",
            "lasti",
            "fallback",
            "missing ordinary setup",
            "foreign handler",
            "foreign origin tuple",
            "foreign same-handler tuple",
            "own depth-zero tuple",
            "stale source",
            "stale nop source",
            "duplicate jump",
            "duplicate nop",
            "foreign entry alias",
            "foreign setup alias",
            "duplicate endpoint",
            "changed captured endpoint",
            "changed region priority",
        ] {
            let (mut assembler, entry) = fixture_with_depth(4, true, depth);
            assert!(
                assembler
                    .exception_owner_provenance
                    .ordinary_try_region_origins
                    .is_empty()
            );
            assert!(
                !assembler
                    .exception_owner_provenance
                    .is_original_ordinary_try_region(assembler.exception_regions[0],)
            );
            let other = assembler.label();
            let original_region = assembler.exception_regions[0];
            let witness = assembler
                .exception_owner_provenance
                .comprehension_normal_exit_sources
                .get_mut(&entry)
                .unwrap()
                .following_setup
                .as_mut()
                .unwrap();
            let nop_index = witness.source.item_index as usize;
            let store_index = nop_index - 2;
            let setup = witness.setup;
            let nop = witness.instruction;
            match mutation {
                "captured region depth" | "both region depths" => {
                    witness.region.as_mut().unwrap().depth += 1;
                }
                "both nop depths" => {
                    let mut changed =
                        Instruction::new(NOP, nop.operand(), nop.location, Some(depth + 1));
                    changed.set_exception_policy(nop.exception_policy());
                    witness.instruction = changed;
                    assembler.linear_items_mut()[nop_index] = Item::Instruction(changed);
                }
                "both store depths" | "both store opcodes" => {
                    let original = witness.prefix[3];
                    let changed = Instruction::new(
                        if mutation == "both store opcodes" {
                            STORE_DEREF
                        } else {
                            STORE_FAST
                        },
                        original.operand(),
                        original.location,
                        Some(depth + u32::from(mutation == "both store depths")),
                    );
                    witness.prefix[3] = changed;
                    assembler.linear_items_mut()[store_index] = Item::Instruction(changed);
                }
                "stale nop source" => witness.source.stream = EmissionStreamId::next(),
                "changed captured endpoint" => witness.region.as_mut().unwrap().end = other,
                _ => {}
            }
            match mutation {
                "current region depth" | "both region depths" => {
                    assembler.exception_regions[0].depth += 1;
                }
                "lasti" => assembler.exception_regions[0].preserve_lasti = true,
                "fallback" => assembler.exception_regions[0].is_scope_fallback = true,
                "missing ordinary setup" => {
                    assembler
                        .exception_owner_provenance
                        .ordinary_try_setups
                        .remove(&setup);
                }
                "foreign handler" => {
                    assembler
                        .exception_owner_provenance
                        .ordinary_try_setups
                        .insert(setup, other);
                }
                "foreign origin tuple" | "foreign same-handler tuple" | "own depth-zero tuple" => {
                    let origin = if mutation == "own depth-zero tuple" {
                        setup
                    } else {
                        other
                    };
                    if mutation == "foreign same-handler tuple" {
                        assembler
                            .exception_owner_provenance
                            .ordinary_try_setups
                            .insert(other, original_region.target);
                    }
                    assembler
                        .exception_owner_provenance
                        .ordinary_try_region_origins
                        .push((
                            origin,
                            original_region.start,
                            original_region.end,
                            original_region.target,
                        ));
                }
                "stale source" => assembler.emission_stream = EmissionStreamId::next(),
                "duplicate jump" => {
                    let original = assembler
                        .exception_owner_provenance
                        .original_comprehension_normal_exits[0]
                        .1;
                    assembler.linear_items_mut()[3] = Item::Instruction(original);
                }
                "duplicate nop" => assembler.linear_items_mut()[3] = Item::Instruction(nop),
                "foreign entry alias" => {
                    assembler.linear_items_mut()[store_index - 4] = Item::Label(other);
                }
                "foreign setup alias" => {
                    assembler.linear_items_mut()[nop_index + 1] = Item::Label(other);
                }
                "duplicate endpoint" => {
                    assembler.linear_items_mut()[3] = Item::Label(original_region.end);
                }
                "changed region priority" => {
                    assembler.exception_regions[0].priority =
                        ExceptionRegionPriority::OriginalRange(setup, other);
                }
                "later nop remap" => {
                    assembler.emit(NOP, 0);
                    assert!(assembler.take_trailing_nop_location().is_some());
                }
                _ => {}
            }
            assembler.resolve_comprehension_owner_entries();
            assert_eq!(
                certified(&assembler, entry),
                matches!(mutation, "intact" | "later nop remap"),
                "depth {depth}: {mutation}"
            );
        }
        let (mut assembler, entry) = fixture_with_depth(4, true, depth);
        assert_eq!(assembler.used_constant_indices(1), [true]);
        assert!(certified(&assembler, entry));
        assembler.remap_constant_indices(&[Some(1)]);
        assert_eq!(assembler.used_constant_indices(2), [false, true]);
        assert!(certified(&assembler, entry));
    }
}
