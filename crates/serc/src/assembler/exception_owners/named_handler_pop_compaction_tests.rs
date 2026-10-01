use super::OwnerLabelPositions;
use crate::assembler::{Assembler, ExceptionPolicy, Item, LayoutAssembler, SourceLocation};
use crate::target::opcodes::{LOAD_FAST, NOP, POP_EXCEPT, RERAISE, RETURN_VALUE, SWAP};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExtraPop {
    None,
    SameArray,
    PreviousArray,
}

fn check_compacted_pair(extra: ExtraPop) {
    let mut emitted = Assembler::with_python_version(crate::PythonVersion::Py314);
    let entry = emitted.label();
    let pair_entry = emitted.label();
    let swap_start = emitted.label();
    let swap_end = emitted.label();
    let pop_except_start = emitted.label();
    let end = emitted.label();
    let handler = emitted.label();
    emitted.mark(entry);
    emitted.record_exception_owner_block_boundary(entry);
    emitted.set_location(SourceLocation::new(8, 8, 20, 35));
    emitted.emit_with_depth(LOAD_FAST, 0, 2, ExceptionPolicy::default());
    let whole_start = emitted.item_count();
    if extra != ExtraPop::None {
        emitted.emit_with_depth(NOP, 0, 2, ExceptionPolicy::EXCLUDE_ALWAYS);
        emitted.mark_last_as_converted_pop_block();
    }
    if extra == ExtraPop::PreviousArray {
        emitted.mark(pair_entry);
        emitted.record_exception_owner_block_boundary(pair_entry);
    }
    let pair_start = emitted.item_count();
    emitted.emit_with_depth(NOP, 0, 2, ExceptionPolicy::EXCLUDE_ALWAYS);
    emitted.mark_last_as_converted_pop_block();
    emitted.mark(swap_start);
    emitted.emit_with_depth(SWAP, 2, 2, ExceptionPolicy::default());
    emitted.mark(swap_end);
    emitted.emit_with_depth(NOP, 0, 2, ExceptionPolicy::EXCLUDE_ALWAYS);
    emitted.mark_last_as_converted_pop_block();
    emitted.mark(pop_except_start);
    emitted.emit_with_depth(POP_EXCEPT, 0, 1, ExceptionPolicy::default());

    if extra != ExtraPop::None {
        emitted.record_named_handler_pop_pair(whole_start);
        assert!(
            emitted
                .exception_owner_provenance
                .named_handler_pop_pairs
                .is_empty(),
            "the complete three-POP span is not a pair: {extra:?}"
        );
    }
    emitted.record_named_handler_pop_pair(pair_start);
    assert_eq!(
        emitted
            .exception_owner_provenance
            .named_handler_pop_pairs
            .len(),
        1,
        "the unchanged four-record suffix captures: {extra:?}"
    );
    emitted.mark(end);
    emitted.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
    emitted.mark(handler);
    emitted.emit_with_depth(RERAISE, 0, 0, ExceptionPolicy::default());
    emitted.add_exception_region_with_origin(entry, end, handler, 0, false, (entry, end));

    let mut pipeline = emitted.into_optimization_pipeline();
    pipeline.remove_redundant_nops();
    let items = pipeline.graph.copy_items();
    let records = items
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
        [LOAD_FAST, SWAP, POP_EXCEPT, RETURN_VALUE, RERAISE],
        "real NOP cleanup: {extra:?}"
    );
    assert_eq!(
        records[1].local_policy.hidden_exception_owner_slots(),
        if extra == ExtraPop::SameArray { 2 } else { 1 },
        "actual SWAP compaction count: {extra:?}"
    );
    assert_eq!(records[2].local_policy.hidden_exception_owner_slots(), 1);

    // Preserve the pass's output verbatim; no synthesized hidden-slot metadata.
    let layout = LayoutAssembler {
        python_version: pipeline.python_version,
        legacy_uniform_jump_seed: pipeline.legacy_uniform_jump_seed,
        items,
        exception_regions: pipeline.exception_regions,
        exception_owner_provenance: pipeline.exception_owner_provenance,
        preserved_block_boundaries: pipeline.preserved_block_boundaries,
        removed_max_depth: None,
        next_label: pipeline.next_label,
        has_next_location: pipeline.has_next_location,
    };
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
    let (slots, aliases, method_donors) =
        layout.captured_pop_slots(&rustc_hash::FxHashSet::default(), &positions);
    assert!(method_donors.is_empty());
    if extra == ExtraPop::SameArray {
        assert!(slots.is_empty());
        assert!(aliases.is_empty());
    } else {
        let owner = positions[&if extra == ExtraPop::PreviousArray {
            pair_entry
        } else {
            entry
        }];
        assert_eq!(slots.len(), 2);
        assert_eq!(slots.get(&1), Some(&owner));
        assert_eq!(slots.get(&2), Some(&owner));
        assert_eq!(
            aliases,
            [swap_start, swap_end, pop_except_start]
                .into_iter()
                .collect()
        );
    }
}

#[test]
fn named_handler_pair_survives_real_two_pop_compaction() {
    check_compacted_pair(ExtraPop::None);
}

#[test]
fn named_handler_pair_rejects_real_same_array_three_pop_compaction() {
    check_compacted_pair(ExtraPop::SameArray);
}

#[test]
fn named_handler_pair_excludes_a_previous_array_pop() {
    check_compacted_pair(ExtraPop::PreviousArray);
}
