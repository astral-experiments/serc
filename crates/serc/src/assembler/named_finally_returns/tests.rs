use super::*;
use crate::assembler::{ConstantTable, ExceptionPolicy};
use crate::code_object::{Constant, NonReflexiveRemap};
use crate::target::opcodes::{JUMP_FORWARD, RERAISE};
use crate::{CompileError, PythonVersion};

fn label(value: u32) -> Label {
    Label::from_u32(value)
}

fn location() -> SourceLocation {
    SourceLocation::new(9, 9, 8, 18)
}

fn record(opcode: Opcode, value: u32, depth: u32) -> Item {
    Item::Instruction(Instruction::new(
        opcode,
        Operand::Value(value),
        SourceLocation::NONE,
        Some(depth),
    ))
}

fn items(assembler: &mut Assembler) -> &mut Vec<Item> {
    let UnoptimizedCode::Linear(items) = &mut assembler.code else {
        unreachable!()
    };
    items
}

fn prefix(opcode: Opcode, version: PythonVersion) -> (Assembler, NamedFinallyReturnPrefix) {
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Label(label(0)),
        Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(0),
            location(),
            Some(2),
        )),
    ]);
    assembler.python_version = version;
    assembler.next_label = 6;
    let prefix = assembler.named_finally_return_prefix(opcode).unwrap();
    (assembler, prefix)
}

fn footer(assembler: &mut Assembler, opcode: Opcode) {
    let mut returned = Instruction::new(
        RETURN_VALUE,
        Operand::Value(0),
        SourceLocation::NONE,
        Some(0),
    );
    if opcode == NOP {
        returned.no_location_policy.allow_block_inlining();
    }
    items(assembler).extend([
        Item::Label(label(1)),
        record(SWAP, 2, 2),
        Item::Label(label(2)),
        Item::Label(label(3)),
        record(POP_EXCEPT, 0, 1),
        record(LOAD_CONST, 2, 2),
        record(STORE_FAST, 1, 1),
        record(DELETE_FAST, 1, 1),
        Item::Label(label(4)),
        Item::Instruction(returned),
    ]);
    assembler.exception_regions.push(ExceptionRegion {
        start: label(1),
        end: label(2),
        target: label(5),
        depth: 1,
        preserve_lasti: true,
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(label(1), label(2)),
    });
}

fn capture(assembler: &mut Assembler, prefix: &NamedFinallyReturnPrefix) {
    assembler.record_named_finally_return(prefix, label(4), label(3), label(5), 1, 2);
}

fn fixture(opcode: Opcode, version: PythonVersion) -> Assembler {
    let (mut assembler, prefix) = prefix(opcode, version);
    footer(&mut assembler, opcode);
    capture(&mut assembler, &prefix);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .named_finally_returns
            .len(),
        1
    );
    items(&mut assembler).extend([Item::Label(label(5)), record(RERAISE, 1, 0)]);
    assembler
}

fn returned(pipeline: &OptimizationPipeline) -> SourceLocation {
    pipeline
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(record) if record.opcode == RETURN_VALUE => Some(record.location),
            _ => None,
        })
        .unwrap()
}

fn exit_locations(pipeline: &mut OptimizationPipeline) {
    pipeline.retain_named_finally_returns();
    let stores = pipeline.capture_reversed_store_exits();
    let joins = pipeline.capture_threaded_normal_try_joins();
    let mut donations = Vec::new();
    pipeline
        .duplicate_exit_blocks_with_store_origins(
            &FxHashSet::default(),
            (&mut Vec::new(), &mut Vec::new(), &mut donations),
            &stores,
            &joins,
            &mut [],
            None,
        )
        .unwrap();
    pipeline.transport_named_finally_return_donations(&donations);
}

#[test]
fn named_finally_returns_use_real_donations_and_replay_once() {
    for opcode in [NOP, POP_TOP, DELETE_ATTR] {
        for prepared in [false, true] {
            let mut assembler = fixture(opcode, PythonVersion::Py314);
            if prepared {
                assembler.used_constant_indices(3);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            exit_locations(&mut pipeline);
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .named_finally_returns
                    .len(),
                1
            );
            pipeline.propagate_locations_within_blocks();
            assert_eq!(returned(&pipeline), location());
            let once = pipeline.graph.copy_items();
            pipeline.propagate_named_finally_return_locations();
            assert_eq!(pipeline.graph.copy_items(), once);
        }
    }
}

#[test]
fn named_finally_return_capture_rejects_each_noncanonical_record() {
    for opcode in [NOP, POP_TOP, DELETE_ATTR] {
        for index in [1, 3, 6, 7, 8, 9, 11] {
            for mutation in 0..9 {
                let (mut assembler, prefix) = prefix(opcode, PythonVersion::Py314);
                footer(&mut assembler, opcode);
                let Item::Instruction(record) = &mut assembler.linear_items_mut()[index] else {
                    unreachable!()
                };
                match mutation {
                    0 => record.location.column += 1,
                    1 => record.set_operand(Operand::Value(90)),
                    2 => {
                        *record = Instruction::new(
                            record.opcode,
                            record.operand(),
                            record.location,
                            Some(20),
                        );
                    }
                    3 => record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                    4 => record.local_policy.add_hidden_exception_owner_slots(1),
                    5 => record.no_location_policy.preserve_location(),
                    6 => record.jump_policy.prevent_small_exit_inlining(),
                    7 => record.fusion_policy.mark_converted_pop_block(),
                    _ => record.add_pre_fold_extra_instructions(1),
                }
                capture(&mut assembler, &prefix);
                assert!(
                    assembler
                        .exception_owner_provenance
                        .named_finally_returns
                        .is_empty(),
                    "{opcode:?} index {index} mutation {mutation}"
                );
            }
        }
    }
}

#[test]
fn named_finally_return_capture_binds_all_labels_and_the_real_swap_region() {
    for mutation in 0..17 {
        let (mut assembler, prefix) = prefix(POP_TOP, PythonVersion::Py314);
        footer(&mut assembler, POP_TOP);
        match mutation {
            0 => assembler.exception_regions[0].depth += 1,
            1 => assembler.exception_regions[0].preserve_lasti = false,
            2 => assembler.exception_regions[0].is_scope_fallback = true,
            3 => assembler.exception_regions[0].start = label(0),
            4 => assembler.exception_regions[0].end = label(4),
            5 => assembler.exception_regions[0].target = label(0),
            6 => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::OriginalRange(label(0), label(2));
            }
            7 => assembler
                .exception_regions
                .push(assembler.exception_regions[0]),
            8 => {
                items(&mut assembler).insert(0, Item::Label(label(2)));
            }
            9 => {
                items(&mut assembler).insert(5, Item::Label(label(30)));
            }
            10 => items(&mut assembler).push(record(NOP, 0, 0)),
            11 => {
                assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .insert(label(1));
            }
            12 => {
                assembler
                    .exception_owner_provenance
                    .borrow_only_block_boundaries
                    .insert(label(2));
            }
            13 => {
                assembler.borrow_unreachable_blocks.insert(label(3));
            }
            14 => {
                assembler.borrow_finally_entries.insert(label(4), None);
            }
            15 => {
                items(&mut assembler).insert(
                    0,
                    Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(label(3)),
                        location(),
                        Some(0),
                    )),
                );
            }
            _ => {
                assembler
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(label(4), 1);
            }
        }
        capture(&mut assembler, &prefix);
        assert!(
            assembler
                .exception_owner_provenance
                .named_finally_returns
                .is_empty(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn named_finally_return_rejects_prepared_and_foreign_stream_capture() {
    let (mut assembler, captured) = prefix(NOP, PythonVersion::Py314);
    footer(&mut assembler, NOP);
    items(&mut assembler).extend([Item::Label(label(5)), record(RERAISE, 1, 0)]);
    assembler.used_constant_indices(3);
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    assert!(assembler.named_finally_return_prefix(NOP).is_none());
    capture(&mut assembler, &captured);
    assert!(
        assembler
            .exception_owner_provenance
            .named_finally_returns
            .is_empty()
    );
    let (_, foreign) = prefix(POP_TOP, PythonVersion::Py314);
    let (mut assembler, _) = prefix(POP_TOP, PythonVersion::Py314);
    footer(&mut assembler, POP_TOP);
    capture(&mut assembler, &foreign);
    assert!(
        assembler
            .exception_owner_provenance
            .named_finally_returns
            .is_empty()
    );
}

#[test]
fn named_finally_return_late_roles_and_original_records_remain_exact() {
    for mutation in 0..18 {
        let mut pipeline = fixture(POP_TOP, PythonVersion::Py314).into_optimization_pipeline();
        exit_locations(&mut pipeline);
        match mutation {
            0 => {
                pipeline.preserved_block_boundaries.insert(label(1));
            }
            1 => {
                pipeline.borrow_block_aliases.insert(label(2));
            }
            2 => {
                pipeline.borrow_unreachable_blocks.insert(label(3));
            }
            3 => {
                pipeline.borrow_finally_entries.insert(label(4), None);
            }
            4 => {
                pipeline.borrow_finally_jump_sources.insert(label(1), true);
            }
            5 => {
                pipeline
                    .exception_owner_provenance
                    .return_region_reentries
                    .insert(label(2));
            }
            6 => {
                pipeline
                    .exception_owner_provenance
                    .loop_finally_region_boundaries
                    .insert(label(3));
            }
            7 => {
                pipeline
                    .exception_owner_provenance
                    .normal_try_copy_internal_labels
                    .push(label(4));
            }
            8 => pipeline.exception_regions[0].target = label(1),
            12 => {
                pipeline
                    .exception_owner_provenance
                    .normal_try_copy_blocks
                    .insert(label(1));
            }
            13 => {
                pipeline
                    .exception_owner_provenance
                    .original_fallthrough_setups
                    .insert(label(2));
            }
            14 => {
                pipeline
                    .exception_owner_provenance
                    .named_handler_cleanup_targets
                    .insert(label(3));
            }
            15 => {
                pipeline
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(label(4), label(5));
            }
            16 => {
                pipeline
                    .exception_owner_provenance
                    .setup_nop_ranges
                    .push((label(1), label(2)));
            }
            17 => {
                pipeline.borrow_omitted_for_backedges.insert(label(3));
            }
            _ => pipeline.graph.for_each_instruction_mut(|record| {
                if record.opcode == DELETE_FAST {
                    match mutation {
                        9 => record.location = location(),
                        10 => record.local_policy.add_hidden_exception_owner_slots(1),
                        _ => record.set_operand(Operand::Value(2)),
                    }
                }
            }),
        }
        pipeline.retain_named_finally_returns();
        assert!(
            pipeline
                .exception_owner_provenance
                .named_finally_returns
                .is_empty(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn named_finally_return_late_replay_rejects_every_changed_record_and_head() {
    for index in 0..11 {
        let mut pipeline = fixture(POP_TOP, PythonVersion::Py314).into_optimization_pipeline();
        let old = pipeline.exception_owner_provenance.named_finally_returns[0].items[index];
        let mut items = pipeline.graph.copy_items();
        let at = items.iter_mut().find(|item| **item == old).unwrap();
        match at {
            Item::Label(label) => *label = Label::from_u32(40),
            Item::Instruction(record) => record.local_policy.add_hidden_exception_owner_slots(1),
        }
        pipeline.graph = crate::assembler::block_graph::BlockGraph::from_items(
            items,
            &pipeline.preserved_block_boundaries,
        );
        pipeline.retain_named_finally_returns();
        assert!(
            pipeline
                .exception_owner_provenance
                .named_finally_returns
                .is_empty(),
            "index {index}"
        );
    }
}

#[test]
fn named_finally_return_donations_require_unique_real_head_and_full_records() {
    for mutation in 0..6 {
        let mut pipeline = fixture(POP_TOP, PythonVersion::Py314).into_optimization_pipeline();
        let mut donations = Vec::new();
        for (index, head) in [(2, vec![label(1)]), (5, vec![label(2), label(3)])] {
            let Item::Instruction(before) =
                pipeline.exception_owner_provenance.named_finally_returns[0].items[index]
            else {
                unreachable!()
            };
            let mut after = before;
            after.location = location();
            pipeline.graph.for_each_instruction_mut(|record| {
                if *record == before {
                    *record = after;
                }
            });
            donations.push(NamedFinallyReturnDonation {
                head,
                before,
                after,
            });
        }
        match mutation {
            0 => {
                donations.remove(0);
            }
            1 => donations[0].head.push(label(20)),
            2 => donations[0]
                .before
                .local_policy
                .add_hidden_exception_owner_slots(1),
            3 => donations[0].after.location.column += 1,
            4 => donations.push(NamedFinallyReturnDonation {
                head: donations[0].head.clone(),
                before: donations[0].before,
                after: donations[0].after,
            }),
            _ => donations[0].head = vec![label(3)],
        }
        pipeline.transport_named_finally_return_donations(&donations);
        assert!(
            pipeline
                .exception_owner_provenance
                .named_finally_returns
                .is_empty(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn named_finally_return_capture_only_region_may_split_without_new_entries() {
    let mut pipeline = fixture(POP_TOP, PythonVersion::Py314).into_optimization_pipeline();
    exit_locations(&mut pipeline);
    pipeline.exception_regions[0].start = label(0);
    pipeline.exception_regions[0].end = label(3);
    pipeline.propagate_locations_within_blocks();
    assert_eq!(returned(&pipeline), location());
}

struct Constants(Vec<Constant>);
impl ConstantTable for Constants {
    fn get(&self, index: u32) -> &Constant {
        &self.0[index as usize]
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn intern(&mut self, _: Constant) -> Result<u32, CompileError> {
        unreachable!()
    }
    fn fold_sequence(&mut self, _: Vec<Constant>, _: bool) -> Result<u32, CompileError> {
        unreachable!()
    }
    fn fold_binary_sequence(
        &mut self,
        _: Constant,
        _: u32,
        _: Constant,
    ) -> Result<Option<u32>, CompileError> {
        unreachable!()
    }
    fn reallocate_non_reflexive_constant(
        &mut self,
        _: u32,
        _: &mut NonReflexiveRemap,
    ) -> Result<u32, CompileError> {
        unreachable!()
    }
    fn retain_used(&mut self, used: &[bool], _: &[u32], _: &[bool]) -> Vec<Option<u32>> {
        self.retain_used_in_order(used)
    }
    fn retain_used_in_order(&mut self, used: &[bool]) -> Vec<Option<u32>> {
        let mut values = Vec::new();
        let mapping = self
            .0
            .iter()
            .enumerate()
            .map(|(index, value)| {
                if index == 0 || used[index] {
                    let new = u32::try_from(values.len()).unwrap();
                    values.push(value.clone());
                    Some(new)
                } else {
                    None
                }
            })
            .collect();
        self.0 = values;
        mapping
    }
}

pub(in crate::assembler) fn assert_common_constant_transport(
    lower: impl Fn(&mut OptimizationPipeline, &mut dyn ConstantTable),
) {
    for prepared in [false, true] {
        for mutation in 0..3 {
            let stale = mutation != 0;
            let mut assembler = fixture(POP_TOP, PythonVersion::Py315);
            if prepared {
                assembler.used_constant_indices(3);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            exit_locations(&mut pipeline);
            if mutation == 1 {
                let Item::Instruction(record) =
                    &mut pipeline.exception_owner_provenance.named_finally_returns[0].items[6]
                else {
                    unreachable!()
                };
                record.set_operand(Operand::Value(99));
            }
            let mut constants = Constants(vec![
                Constant::None,
                Constant::Bool(true),
                if mutation == 2 {
                    Constant::Bool(false)
                } else {
                    Constant::None
                },
            ]);
            lower(&mut pipeline, &mut constants);
            assert_eq!(constants.0.len(), 1);
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .named_finally_returns
                    .len(),
                usize::from(!stale)
            );
            pipeline.propagate_locations_within_blocks();
            assert_eq!(
                returned(&pipeline),
                if stale {
                    SourceLocation::NONE
                } else {
                    location()
                }
            );
        }
    }
}

#[test]
fn named_finally_return_emitted_constant_remap_never_normalizes_stale_records() {
    for prepared in [false, true] {
        for stale in [false, true] {
            let mut assembler = fixture(POP_TOP, PythonVersion::Py314);
            if prepared {
                assembler.used_constant_indices(3);
            }
            if stale {
                let Item::Instruction(record) =
                    &mut assembler.exception_owner_provenance.named_finally_returns[0].items[6]
                else {
                    unreachable!()
                };
                record.set_operand(Operand::Value(99));
            }
            assembler.remap_constant_indices(&[Some(0), None, Some(1)]);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .named_finally_returns
                    .len(),
                usize::from(!stale)
            );
            let mut pipeline = assembler.into_optimization_pipeline();
            exit_locations(&mut pipeline);
            pipeline.propagate_locations_within_blocks();
            assert_eq!(
                returned(&pipeline),
                if stale {
                    SourceLocation::NONE
                } else {
                    location()
                }
            );
        }
    }
}
