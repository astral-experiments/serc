use crate::assembler::{
    Assembler, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Item, Label,
    LayoutAssembler, Operand, SourceLocation,
};
use crate::target::opcodes::{
    CALL, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE,
    POP_TOP, RERAISE, RETURN_VALUE,
};

struct OrdinaryPopLabels {
    outer_start: Label,
    setup_alias: Label,
    setup: Label,
    entry: Label,
    iterator_alias: Label,
    omitted_frontier: Label,
    finally_alias: Label,
    finally_frontier: Label,
    guard_exit: Label,
    original_end: Label,
    outer_end: Label,
    handler: Label,
    finally_handler: Label,
    fallback: Label,
    foreign: Label,
}

struct OrdinaryPopFixture {
    assembler: Assembler,
    labels: OrdinaryPopLabels,
    return_item: usize,
    pop_item: usize,
    calls: usize,
}

fn ordinary_pop_fixture(calls: usize) -> OrdinaryPopFixture {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let labels = OrdinaryPopLabels {
        outer_start: assembler.label(),
        setup_alias: assembler.label(),
        setup: assembler.label(),
        entry: assembler.label(),
        iterator_alias: assembler.label(),
        omitted_frontier: assembler.label(),
        finally_alias: assembler.label(),
        finally_frontier: assembler.label(),
        guard_exit: assembler.label(),
        original_end: assembler.label(),
        outer_end: assembler.label(),
        handler: assembler.label(),
        finally_handler: assembler.label(),
        fallback: assembler.label(),
        foreign: assembler.label(),
    };
    assembler.record_ordinary_try_exception_setup(labels.setup, false, labels.handler);
    assembler.mark(labels.outer_start);
    assembler.mark(labels.setup_alias);
    assembler.mark(labels.setup);
    assembler.set_location(SourceLocation::new(6, 6, 15, 19));
    assembler.emit_with_depth(LOAD_FAST, 1, 2, ExceptionPolicy::default());
    assembler.emit_forward_with_depth(
        POP_JUMP_IF_FALSE,
        labels.original_end,
        1,
        ExceptionPolicy::default(),
    );
    assembler.mark(labels.entry);
    assembler.record_exception_owner_block_boundary(labels.entry);
    for line in 0..calls {
        assembler.set_location(SourceLocation::new(
            7 + i32::try_from(line).unwrap(),
            7 + i32::try_from(line).unwrap(),
            12,
            19,
        ));
        // Fused LOAD_GLOBAL/PUSH_NULL contributes exactly one early removal.
        assembler.emit_with_depth(LOAD_GLOBAL, 1, 3, ExceptionPolicy::default());
        assembler.emit_with_depth(CALL, 0, 2, ExceptionPolicy::default());
        assembler.emit_with_depth(POP_TOP, 0, 1, ExceptionPolicy::default());
    }
    assembler.set_location(SourceLocation::new(12, 12, 16, 22));
    let return_item = assembler.item_count();
    assembler.emit_with_depth(NOP, 0, 1, ExceptionPolicy::default());
    assembler.mark(labels.iterator_alias);
    assembler.mark(labels.omitted_frontier);
    assembler.emit_with_depth(POP_TOP, 0, 0, ExceptionPolicy::default());
    assembler.mark(labels.finally_alias);
    assembler.mark(labels.finally_frontier);
    let pop_item = assembler.item_count();
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
    assembler.mark_last_as_converted_pop_block();
    assembler.set_location(SourceLocation::new(16, 16, 7, 12));
    assembler.emit_with_depth(LOAD_FAST, 0, 1, ExceptionPolicy::default());
    assembler.set_location(SourceLocation::new(16, 16, 7, 24));
    let conditional = assembler.emit_forward_with_depth(
        POP_JUMP_IF_NONE,
        labels.guard_exit,
        0,
        ExceptionPolicy::default(),
    );
    assembler
        .emitted_jump_mut(conditional, "preparing the original None condition")
        .local_policy
        .add_hidden_exception_owner_slots(3);
    assembler.emit_with_depth(NOT_TAKEN, 0, 0, ExceptionPolicy::NonePresent);
    assembler.mark(labels.guard_exit);
    assembler.emit_with_depth(LOAD_CONST, 0, 1, ExceptionPolicy::default());
    assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
    assembler.mark(labels.original_end);
    assembler.mark(labels.outer_end);
    assembler.mark(labels.handler);
    assembler.emit_with_depth(RERAISE, 0, 1, ExceptionPolicy::default());
    assembler.mark(labels.finally_handler);
    assembler.emit_with_depth(RERAISE, 0, 0, ExceptionPolicy::default());
    assembler.mark(labels.fallback);
    assembler.emit_with_depth(RERAISE, 1, 0, ExceptionPolicy::default());
    assembler.add_exception_region_with_origin(
        labels.setup,
        labels.omitted_frontier,
        labels.handler,
        1,
        false,
        (labels.setup, labels.original_end),
    );
    assembler.add_exception_region_with_origin(
        labels.outer_start,
        labels.finally_frontier,
        labels.finally_handler,
        0,
        false,
        (labels.outer_start, labels.outer_end),
    );
    assembler.exception_regions.push(ExceptionRegion {
        start: labels.outer_start,
        end: labels.fallback,
        target: labels.fallback,
        depth: 0,
        preserve_lasti: true,
        is_scope_fallback: true,
        priority: ExceptionRegionPriority::OriginalRange(labels.outer_start, labels.outer_end),
    });
    assembler.record_copied_cleanup_return_none_prefix(return_item, pop_item, false, false, None);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .copied_cleanup_return_none_prefixes
            .len(),
        1
    );
    assembler
        .exception_owner_provenance
        .copied_cleanup_return_none_prefixes[0]
        .omitted_ordinary_try = Some(super::CopiedCleanupOrdinaryTryPop {
        setup: labels.setup,
        original_end: labels.original_end,
        target: labels.handler,
        omitted_frontier: labels.omitted_frontier,
        finally_target: labels.finally_handler,
        finally_frontier: labels.finally_frontier,
        bound: None,
    });
    OrdinaryPopFixture {
        assembler,
        labels,
        return_item,
        pop_item,
        calls,
    }
}

#[test]
fn omitted_ordinary_pop_binds_one_exact_source_fragment() {
    for mutation in [
        "intact",
        "missing setup",
        "duplicate setup",
        "setup retarget",
        "manager setup",
        "missing fragment",
        "duplicate fragment",
        "other origin setup",
        "other origin end",
        "resolved before capture",
        "depth zero",
        "depth two",
        "lasti",
        "fallback",
        "other target",
        "foreign co-located end",
        "ordinary still owns iterator",
        "authored return outside fragment",
        "outer target",
        "outer end",
        "outer lasti",
    ] {
        let mut fixture = ordinary_pop_fixture(0);
        let OrdinaryPopFixture {
            assembler, labels, ..
        } = &mut fixture;
        match mutation {
            "missing setup" => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .clear();
            }
            "duplicate setup" => {
                assembler
                    .exception_owner_provenance
                    .setups
                    .push((labels.setup, None));
            }
            "setup retarget" => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(labels.setup, labels.foreign);
            }
            "manager setup" => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .clear();
                assembler
                    .exception_owner_provenance
                    .synchronous_with_setups
                    .insert(labels.setup);
            }
            "missing fragment" => {
                assembler.exception_regions.remove(0);
            }
            "duplicate fragment" => {
                assembler
                    .exception_regions
                    .push(assembler.exception_regions[0]);
            }
            "other origin setup" => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::OriginalRange(labels.outer_start, labels.original_end);
            }
            "other origin end" => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::OriginalRange(labels.setup, labels.outer_end);
            }
            "resolved before capture" => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::Resolved { start: 1, end: 30 };
            }
            "depth zero" => assembler.exception_regions[0].depth = 0,
            "depth two" => assembler.exception_regions[0].depth = 2,
            "lasti" => assembler.exception_regions[0].preserve_lasti = true,
            "fallback" => assembler.exception_regions[0].is_scope_fallback = true,
            "other target" => assembler.exception_regions[0].target = labels.finally_handler,
            "foreign co-located end" => assembler.exception_regions[0].end = labels.iterator_alias,
            "ordinary still owns iterator" => {
                assembler.exception_regions[0].end = labels.finally_frontier;
            }
            "authored return outside fragment" => {
                assembler.exception_regions[0].start = labels.omitted_frontier;
            }
            "outer target" => assembler.exception_regions[1].target = labels.handler,
            "outer end" => assembler.exception_regions[1].end = labels.omitted_frontier,
            "outer lasti" => assembler.exception_regions[1].preserve_lasti = true,
            _ => {}
        }
        assembler.bind_copied_cleanup_ordinary_try_pops();
        let origin = &assembler
            .exception_owner_provenance
            .copied_cleanup_return_none_prefixes[0];
        let bound = origin
            .omitted_ordinary_try
            .as_ref()
            .and_then(|witness| witness.bound.as_ref());
        assert_eq!(bound.is_some(), mutation == "intact", "{mutation}");
        // This family's depth-one capture must not change the shared depth-zero table.
        assert!(
            assembler
                .exception_owner_provenance
                .ordinary_try_region_origins
                .is_empty()
        );
        if let Some(bound) = bound {
            assert_eq!(bound.ordinary.start, labels.setup);
            assert_eq!(bound.ordinary.end, labels.omitted_frontier);
            assert_eq!(
                bound.ordinary_start_aliases,
                [labels.outer_start, labels.setup_alias, labels.setup]
            );
            assert_eq!(
                bound.ordinary_end_aliases,
                [labels.iterator_alias, labels.omitted_frontier]
            );
            assert_eq!(bound.finally_owner.target, labels.finally_handler);
        }
    }
}

// The binding table's "foreign co-located end" is deliberately rejected: source
// lowering named one concrete omitted POP frontier. In the late table, using an
// original captured head alias is deliberately accepted. Those are different
// stages and must not be reduced to a same-offset predicate.

fn compact_ordinary_pop_fixture(
    mut fixture: OrdinaryPopFixture,
) -> (LayoutAssembler, OrdinaryPopLabels, usize) {
    fixture.assembler.bind_copied_cleanup_ordinary_try_pops();
    let original_positions = fixture
        .assembler
        .linear_items()
        .iter()
        .enumerate()
        .filter_map(|(at, item)| {
            if let Item::Label(label) = item {
                Some((*label, at))
            } else {
                None
            }
        })
        .collect::<rustc_hash::FxHashMap<_, _>>();
    let resolve = |region: &mut ExceptionRegion| {
        if let ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
            region.priority = ExceptionRegionPriority::Resolved {
                start: original_positions[&start],
                end: original_positions[&end],
            };
        }
    };
    for region in &mut fixture.assembler.exception_regions {
        resolve(region);
    }
    let origin = &mut fixture
        .assembler
        .exception_owner_provenance
        .copied_cleanup_return_none_prefixes[0];
    let bound = origin
        .omitted_ordinary_try
        .as_mut()
        .unwrap()
        .bound
        .as_mut()
        .unwrap();
    resolve(&mut bound.ordinary);
    resolve(&mut bound.finally_owner);
    let mut items = Vec::new();
    let mut pending_removed = 0;
    for (index, item) in fixture.assembler.linear_items().iter().enumerate() {
        if index == fixture.return_item || index == fixture.pop_item {
            pending_removed += 1;
            continue;
        }
        let mut item = *item;
        if let Item::Instruction(record) = &mut item {
            record
                .local_policy
                .add_hidden_exception_owner_slots(pending_removed);
            pending_removed = 0;
        }
        items.push(item);
    }
    assert_eq!(pending_removed, 0);
    let provenance = &mut fixture.assembler.exception_owner_provenance;
    provenance
        .removed_pop_block_boundaries
        .insert(fixture.labels.finally_frontier);
    provenance
        .removed_pop_block_widths
        .insert(fixture.labels.finally_frontier, 1);
    let layout = LayoutAssembler {
        python_version: crate::PythonVersion::Py314,
        legacy_uniform_jump_seed: false,
        items,
        exception_regions: fixture.assembler.exception_regions,
        exception_owner_provenance: fixture.assembler.exception_owner_provenance,
        preserved_block_boundaries: rustc_hash::FxHashSet::default(),
        removed_max_depth: None,
        next_label: fixture.labels.foreign.as_u32() + 1,
        has_next_location: false,
    };
    (layout, fixture.labels, fixture.calls)
}

#[test]
fn omitted_ordinary_pop_replay_revalidates_original_aliases_and_cells() {
    for calls in 0..=3 {
        for mutation in [
            "intact",
            "original head aliases",
            "foreign late end alias",
            "foreign frontier prefix",
            "foreign frontier suffix",
            "reversed frontier group",
            "duplicated return head",
            "reversed return head",
            "missing bound witness",
            "manager witness",
            "handler witness",
            "missing frontier",
            "late setup retarget",
            "late original end",
            "late handler",
            "late depth",
            "late lasti",
            "late fallback",
            "late duplicate fragment",
            "outer handler",
            "outer depth",
            "outer lasti",
            "outer fallback",
            "retired entry",
            "copied entry",
            "internal entry",
            "missing source head",
            "interior source head",
            "interior jump",
            "changed iterator cell",
            "changed load",
            "changed condition",
            "changed marker",
            "pop width",
            "extra hidden count",
            "insertions",
            "late removals",
            "late fusion",
        ] {
            let (mut layout, labels, calls) =
                compact_ordinary_pop_fixture(ordinary_pop_fixture(calls));
            let provenance = &mut layout.exception_owner_provenance;
            match mutation {
                "original head aliases" => {
                    layout.exception_regions[0].start = labels.setup_alias;
                    layout.exception_regions[0].end = labels.iterator_alias;
                    layout.exception_regions[1].end = labels.finally_alias;
                }
                "foreign late end alias" => {
                    let at = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(labels.omitted_frontier))
                        .unwrap();
                    layout.items.insert(at, Item::Label(labels.foreign));
                    layout.exception_regions[0].end = labels.foreign;
                }
                "foreign frontier prefix" | "foreign frontier suffix" => {
                    let wanted = if mutation == "foreign frontier prefix" {
                        labels.iterator_alias
                    } else {
                        labels.omitted_frontier
                    };
                    let at = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(wanted))
                        .unwrap();
                    layout.items.insert(
                        at + usize::from(mutation == "foreign frontier suffix"),
                        Item::Label(labels.foreign),
                    );
                }
                "reversed frontier group" => {
                    let first = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(labels.iterator_alias))
                        .unwrap();
                    let last = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(labels.omitted_frontier))
                        .unwrap();
                    layout.items.swap(first, last);
                }
                "duplicated return head" => {
                    let at = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(labels.iterator_alias))
                        .unwrap();
                    layout.items.insert(at, Item::Label(labels.entry));
                }
                "reversed return head" => {
                    layout
                        .items
                        .retain(|item| *item != Item::Label(labels.entry));
                    let at = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(labels.omitted_frontier))
                        .unwrap();
                    layout.items.insert(at + 1, Item::Label(labels.entry));
                }
                "missing bound witness" => {
                    provenance.copied_cleanup_return_none_prefixes[0]
                        .omitted_ordinary_try
                        .as_mut()
                        .unwrap()
                        .bound = None;
                }
                "manager witness" => {
                    provenance.copied_cleanup_return_none_prefixes[0].manager = true;
                }
                "handler witness" => {
                    provenance.copied_cleanup_return_none_prefixes[0].handler = true;
                }
                "missing frontier" => layout
                    .items
                    .retain(|item| *item != Item::Label(labels.omitted_frontier)),
                "late setup retarget" => {
                    provenance
                        .ordinary_try_setups
                        .insert(labels.setup, labels.foreign);
                }
                "late original end" => {
                    layout.exception_regions[0].priority = ExceptionRegionPriority::Resolved {
                        start: 1,
                        end: usize::MAX,
                    };
                }
                "late handler" => layout.exception_regions[0].target = labels.finally_handler,
                "late depth" => layout.exception_regions[0].depth = 0,
                "late lasti" => layout.exception_regions[0].preserve_lasti = true,
                "late fallback" => layout.exception_regions[0].is_scope_fallback = true,
                "late duplicate fragment" => {
                    layout.exception_regions.push(layout.exception_regions[0]);
                }
                "outer handler" => layout.exception_regions[1].target = labels.handler,
                "outer depth" => layout.exception_regions[1].depth = 1,
                "outer lasti" => layout.exception_regions[1].preserve_lasti = true,
                "outer fallback" => layout.exception_regions[1].is_scope_fallback = true,
                "retired entry" => {
                    provenance
                        .retired_normal_try_copy_labels
                        .insert(labels.entry);
                }
                "copied entry" => {
                    provenance.normal_try_copy_blocks.insert(labels.entry);
                }
                "internal entry" => {
                    provenance
                        .normal_try_copy_internal_labels
                        .push(labels.entry);
                }
                "missing source head" => {
                    provenance.source_boundaries.remove(&labels.entry);
                }
                "interior source head" => {
                    provenance.source_boundaries.insert(labels.finally_frontier);
                }
                "pop width" => {
                    provenance
                        .removed_pop_block_widths
                        .insert(labels.finally_frontier, 2);
                }
                _ => {}
            }
            if matches!(
                mutation,
                "changed iterator cell" | "changed load" | "changed condition" | "changed marker"
            ) {
                let wanted = match mutation {
                    "changed iterator cell" => POP_TOP,
                    "changed load" => LOAD_FAST,
                    "changed condition" => POP_JUMP_IF_NONE,
                    _ => NOT_TAKEN,
                };
                // Last matching record is the captured cleanup cell, not the earlier guard/call.
                let record = layout
                    .items
                    .iter_mut()
                    .rev()
                    .find_map(|item| match item {
                        Item::Instruction(record) if record.opcode == wanted => Some(record),
                        _ => None,
                    })
                    .unwrap();
                record.location.column += 1;
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
            let array = positions[&labels.entry];
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
            let mut targets = [labels.guard_exit, labels.original_end]
                .into_iter()
                .collect::<rustc_hash::FxHashSet<_>>();
            if mutation == "interior jump" {
                targets.insert(labels.finally_frontier);
            }
            let mut tail = super::ExceptionOwnerTail {
                early_removals: calls + 5,
                ..super::ExceptionOwnerTail::default()
            };
            match mutation {
                "extra hidden count" => tail.early_removals += 1,
                "insertions" => tail.insertions = 1,
                "late removals" => tail.late_removals = 1,
                "late fusion" => tail.post_compaction_fusions = 1,
                _ => {}
            }
            let decision = layout.copied_cleanup_return_none_owner(
                marker, array, tail, &indexed, &positions, &targets,
            );
            let admits = calls < 3 && matches!(mutation, "intact" | "original head aliases");
            assert_eq!(decision.is_some(), admits, "calls={calls}/{mutation}");
            if let Some(decision) = decision {
                assert_eq!(
                    decision.source_region(),
                    (calls == 1).then_some(1),
                    "calls={calls}/{mutation}"
                );
                assert!(decision.is_definitive() && decision.overrides_existing());
            }
        }
    }
}

#[test]
fn omitted_ordinary_pop_binds_before_prepared_constant_reachability() {
    let mut fixture = ordinary_pop_fixture(0);
    assert_eq!(fixture.assembler.used_constant_indices(2), [true, false]);
    assert!(matches!(
        fixture.assembler.code,
        crate::assembler::UnoptimizedCode::Prepared { .. }
    ));
    assert!(
        fixture
            .assembler
            .exception_owner_provenance
            .copied_cleanup_return_none_prefixes[0]
            .omitted_ordinary_try
            .as_ref()
            .unwrap()
            .bound
            .is_some()
    );
    let pipeline = fixture.assembler.into_optimization_pipeline();
    let bound = pipeline
        .exception_owner_provenance
        .copied_cleanup_return_none_prefixes[0]
        .omitted_ordinary_try
        .as_ref()
        .unwrap()
        .bound
        .as_ref()
        .unwrap();
    assert!(matches!(
        bound.ordinary.priority,
        ExceptionRegionPriority::Resolved { .. }
    ));
}

#[test]
fn omitted_ordinary_pop_discards_missing_priority_identity() {
    let mut fixture = ordinary_pop_fixture(0);
    fixture.assembler.bind_copied_cleanup_ordinary_try_pops();
    let positions = fixture
        .assembler
        .linear_items()
        .iter()
        .enumerate()
        .filter_map(|(at, item)| match item {
            Item::Label(label) if *label != fixture.labels.original_end => Some((*label, at)),
            _ => None,
        })
        .collect();
    let witness = fixture
        .assembler
        .exception_owner_provenance
        .copied_cleanup_return_none_prefixes[0]
        .omitted_ordinary_try
        .as_mut()
        .unwrap();
    witness.resolve_priorities(&positions);
    assert!(witness.bound.is_none());
}

#[test]
fn omitted_ordinary_pop_condition_requires_one_exact_exit_copy_receipt() {
    for mutation in [
        "intact",
        "no receipt",
        "duplicate receipt",
        "wrong original",
        "changed opcode",
        "changed location",
        "changed hidden slots",
        "changed fold history",
        "changed owner policy",
        "missing live record",
        "duplicate live record",
        "old record survives",
        "unbound origin",
        "manager",
        "handler",
        "old origin",
        "duplicate origin",
    ] {
        let mut fixture = ordinary_pop_fixture(0);
        fixture.assembler.bind_copied_cleanup_ordinary_try_pops();
        let provenance = &mut fixture.assembler.exception_owner_provenance;
        let before = provenance.copied_cleanup_return_none_prefixes[0].condition;
        let mut after = before;
        after.set_operand(Operand::Forward(fixture.labels.foreign));
        match mutation {
            "changed opcode" => after.opcode = POP_JUMP_IF_FALSE,
            "changed location" => after.location.column += 1,
            "changed hidden slots" => after.local_policy.add_hidden_exception_owner_slots(1),
            "changed fold history" => after.add_pre_fold_extra_instructions(1),
            "changed owner policy" => after.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
            "unbound origin" => {
                provenance.copied_cleanup_return_none_prefixes[0]
                    .omitted_ordinary_try
                    .as_mut()
                    .unwrap()
                    .bound = None;
            }
            "manager" => provenance.copied_cleanup_return_none_prefixes[0].manager = true,
            "handler" => provenance.copied_cleanup_return_none_prefixes[0].handler = true,
            "old origin" => {
                provenance.copied_cleanup_return_none_prefixes[0].omitted_ordinary_try = None;
            }
            "duplicate origin" => {
                let mut duplicate = ordinary_pop_fixture(0);
                duplicate.assembler.bind_copied_cleanup_ordinary_try_pops();
                provenance.copied_cleanup_return_none_prefixes.push(
                    duplicate
                        .assembler
                        .exception_owner_provenance
                        .copied_cleanup_return_none_prefixes
                        .pop()
                        .unwrap(),
                );
            }
            _ => {}
        }
        let mut receipts = vec![(before, after)];
        let mut items = vec![Item::Instruction(after)];
        match mutation {
            "no receipt" => receipts.clear(),
            "duplicate receipt" => receipts.push((before, after)),
            "wrong original" => receipts[0]
                .0
                .set_operand(Operand::Forward(fixture.labels.handler)),
            "missing live record" => items.clear(),
            "duplicate live record" => items.push(Item::Instruction(after)),
            "old record survives" => items.push(Item::Instruction(before)),
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
