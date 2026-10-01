use super::*;
use crate::assembler::{ExceptionPolicy, NOP, block_graph};
use rustc_hash::FxHashSet;

struct Fixture {
    assembler: Assembler,
    source: HeldAwaitSource,
    start: usize,
    entry: Label,
    location: SourceLocation,
    aliases: [Label; 2],
}

fn items_mut(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items
}

fn fixture() -> Fixture {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let outer = assembler.label();
    let outer_handler = assembler.label();
    assembler.record_ordinary_try_exception_setup(outer, false, outer_handler);
    assembler.mark(outer);
    assembler.set_location(SourceLocation::new(1, 1, 0, 1));
    assembler.emit(NOP, 0);
    let loop_start = assembler.item_count();
    let loop_entry = assembler.label();
    assembler.mark(loop_entry);
    assembler.emit(NOP, 0);
    let inner = assembler.label();
    let inner_handler = assembler.label();
    assembler.record_ordinary_try_exception_setup(inner, false, inner_handler);
    assembler.mark(inner);
    assembler.emit(NOP, 0);
    let manager_start = assembler.item_count();
    let manager_entry = assembler.label();
    assembler.mark(manager_entry);
    assembler.emit(NOP, 0);
    let source = assembler
        .capture_held_await_source(
            &[(outer, outer_handler), (inner, inner_handler)],
            loop_start,
            Some(manager_start),
        )
        .unwrap();
    let entry = assembler.label();
    let sending = assembler.label();
    let completed = assembler.label();
    let yielded = assembler.label();
    let yield_end = assembler.label();
    let cleanup = assembler.label();
    let location = SourceLocation::new(10, 10, 4, 18);
    let value =
        |op, arg, depth| Item::Instruction(Instruction::value_with_depth(op, arg, location, depth));
    let start = assembler.item_count();
    items_mut(&mut assembler).extend([
        Item::Label(entry),
        value(SWAP, 3, 3),
        value(SWAP, 2, 3),
        value(LOAD_CONST, 2, 4),
        value(LOAD_CONST, 2, 5),
        value(LOAD_CONST, 2, 6),
        value(CALL, 3, 2),
        value(GET_AWAITABLE, 2, 2),
        value(LOAD_CONST, 2, 3),
        Item::Label(sending),
        Item::Instruction(Instruction::new(
            SEND,
            Operand::Forward(completed),
            location,
            Some(3),
        )),
        Item::Label(yielded),
        value(YIELD_VALUE, 1, 3),
        Item::Label(yield_end),
        value(RESUME, ResumeLocation::AFTER_AWAIT.argument(), 3),
        Item::Instruction(Instruction::new(
            JUMP_BACKWARD_NO_INTERRUPT,
            Operand::Backward(sending),
            location,
            Some(3),
        )),
        Item::Label(cleanup),
        value(CLEANUP_THROW, 0, 3),
        Item::Label(completed),
        value(END_SEND, 0, 2),
        value(POP_TOP, 0, 1),
    ]);
    assembler.record_exception_setup(yielded, false);
    assembler.record_exception_owner_block_boundary(completed);
    assembler.add_exception_region(yielded, yield_end, cleanup, 3, false);
    let aliases = [assembler.label(), assembler.label()];
    Fixture {
        assembler,
        source,
        start,
        entry,
        location,
        aliases,
    }
}

impl Fixture {
    fn capture(&self) -> Option<CompletedManagerAwait> {
        self.assembler
            .capture_completed_manager_await(self.start, self.entry, self.location, 2, 3)
    }

    fn finish(&mut self) {
        for alias in self.aliases {
            self.assembler.mark(alias);
        }
    }

    fn check(&self, origin: &CompletedManagerAwait) -> bool {
        self.assembler
            .completed_manager_await_has_frontier(origin, self.aliases, &self.source)
    }
}

#[test]
fn held_manager_rejects_invalid_saved_offsets_without_indexing() {
    let mut f = fixture();
    let mut origin = f.capture().unwrap();
    f.finish();
    origin.start = usize::MAX;
    assert!(!f.check(&origin));
    origin.start = f.start;
    f.source.entries[0].0 = usize::MAX;
    assert!(!f.check(&origin));
    assert!(source_entry(f.assembler.linear_items(), usize::MAX).is_none());
}

#[test]
fn held_manager_capture_binds_every_original_record_field() {
    for slot in [1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 15, 17, 19, 20] {
        for mutation in 0..8 {
            let mut f = fixture();
            assert!(f.capture().is_some());
            let Item::Instruction(record) = &mut f.assembler.linear_items_mut()[f.start + slot]
            else {
                unreachable!()
            };
            match mutation {
                0 => record.location.column += 1,
                1 => record.set_operand(Operand::Value(99)),
                2 => record.opcode = NOP,
                3 => {
                    *record = Instruction::new(
                        record.opcode,
                        record.operand(),
                        record.location,
                        Some(99),
                    );
                }
                4 => record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                5 => record.add_pre_fold_extra_instructions(1),
                6 => record.jump_policy.preserve_explicit_nop(),
                7 => record.mark_late_fold_location(),
                _ => unreachable!(),
            }
            assert!(f.capture().is_none(), "slot {slot}, mutation {mutation}");
        }
    }
}

#[test]
fn held_manager_replay_binds_records_occurrences_and_source_entries() {
    for mutation in [
        "intact",
        "record",
        "location",
        "moved equal record",
        "foreign alias",
        "duplicate alias",
        "alias reorder",
        "alias missing",
        "source entry",
        "source setup",
        "foreign source label",
        "foreign stream",
        "constant remap",
        "prepared",
        "yield role",
        "end role",
        "ordinary await role",
    ] {
        let mut f = fixture();
        let origin = f.capture().unwrap();
        f.finish();
        match mutation {
            "record" => {
                let Item::Instruction(record) = &mut f.assembler.linear_items_mut()[f.start + 19]
                else {
                    unreachable!()
                };
                record.add_pre_fold_extra_instructions(1);
            }
            "location" => {
                let Item::Instruction(record) = &mut f.assembler.linear_items_mut()[f.start + 20]
                else {
                    unreachable!()
                };
                record.location.line += 1;
            }
            "moved equal record" => {
                let copied = f.assembler.linear_items()[f.start + 19];
                f.assembler.linear_items_mut()[1] = copied;
            }
            "foreign alias" => {
                let label = f.assembler.label();
                items_mut(&mut f.assembler).insert(f.start + 19, Item::Label(label));
            }
            "duplicate alias" => f.assembler.mark(f.entry),
            "alias reorder" => {
                let len = f.assembler.item_count();
                f.assembler.linear_items_mut().swap(len - 1, len - 2);
            }
            "alias missing" => {
                items_mut(&mut f.assembler).pop();
            }
            "source entry" => {
                let at = f.source.entries[0].0;
                f.assembler.linear_items_mut()[at + 1] =
                    Item::Instruction(Instruction::new(NOP, Operand::Value(9), f.location, None));
            }
            "source setup" => {
                f.assembler.exception_owner_provenance.setups.remove(0);
            }
            "foreign source label" => {
                let label = f.assembler.label();
                items_mut(&mut f.assembler).insert(f.source.entries[0].0, Item::Label(label));
            }
            "foreign stream" => f.assembler.emission_stream = EmissionStreamId::next(),
            "constant remap" => f
                .assembler
                .remap_constant_indices(&[Some(0), None, Some(1)]),
            "prepared" => {
                let graph = block_graph::BlockGraph::from_items(
                    f.assembler.linear_items().to_vec(),
                    &FxHashSet::default(),
                );
                let reachable = graph.reachable_blocks(&f.assembler.exception_regions);
                f.assembler.code = UnoptimizedCode::Prepared { graph, reachable };
            }
            "yield role" => {
                f.assembler.exception_owner_provenance.setups.pop();
            }
            "end role" => {
                f.assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .clear();
            }
            "ordinary await role" => {
                let yielded = f.assembler.exception_regions[0].start;
                let cleanup = f.assembler.exception_regions[0].target;
                items_mut(&mut f.assembler).truncate(f.start + 20);
                f.assembler.record_ordinary_await_cleanup(yielded, cleanup);
                items_mut(&mut f.assembler).push(*origin.items.last().unwrap());
                f.finish();
            }
            "intact" => {}
            _ => unreachable!(),
        }
        assert_eq!(f.check(&origin), mutation == "intact", "{mutation}");
        if mutation == "intact" {
            assert!(f.check(&origin));
        }
    }
}

#[test]
fn held_manager_requires_unique_complete_cleanup_region_at_capture_and_replay() {
    for after in [false, true] {
        for mutation in [
            "start",
            "end",
            "handler",
            "depth",
            "lasti",
            "fallback",
            "priority",
            "duplicate region",
            "missing region",
            "competing region",
            "unbound start",
            "unbound end",
            "unbound handler",
            "duplicate handler",
            "unrelated bound region",
        ] {
            let mut f = fixture();
            let origin = f.capture().unwrap();
            if after {
                f.finish();
            }
            let foreign = f.assembler.label();
            let original = f.assembler.exception_regions[0];
            match mutation {
                "start" => f.assembler.exception_regions[0].start = original.end,
                "end" => f.assembler.exception_regions[0].end = original.start,
                "handler" => f.assembler.exception_regions[0].target = original.start,
                "depth" => f.assembler.exception_regions[0].depth += 1,
                "lasti" => f.assembler.exception_regions[0].preserve_lasti = true,
                "fallback" => f.assembler.exception_regions[0].is_scope_fallback = true,
                "priority" => {
                    f.assembler.exception_regions[0].priority =
                        ExceptionRegionPriority::OriginalRange(original.end, original.start);
                }
                "duplicate region" => f.assembler.exception_regions.push(original),
                "missing region" => f.assembler.exception_regions.clear(),
                "competing region" => f.assembler.exception_regions.push(ExceptionRegion {
                    depth: 9,
                    ..original
                }),
                "unbound start" => f.assembler.exception_regions.push(ExceptionRegion {
                    start: foreign,
                    ..original
                }),
                "unbound end" => f.assembler.exception_regions.push(ExceptionRegion {
                    end: foreign,
                    ..original
                }),
                "unbound handler" => f.assembler.exception_regions.push(ExceptionRegion {
                    target: foreign,
                    ..original
                }),
                "duplicate handler" => {
                    f.assembler.linear_items_mut()[0] = Item::Label(original.target);
                }
                "unrelated bound region" => {
                    let start = f.source.setups.origins[0].0;
                    let end = f.source.setups.origins[1].0;
                    let Item::Label(target) = f.source.entries[0].1[0] else {
                        unreachable!()
                    };
                    f.assembler
                        .add_exception_region(start, end, target, 0, false);
                }
                _ => unreachable!(),
            }
            let accepted = if after {
                f.check(&origin)
            } else {
                f.capture().is_some()
            };
            assert_eq!(
                accepted,
                mutation == "unrelated bound region",
                "{after}, {mutation}"
            );
        }
    }
}
