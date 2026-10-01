use super::*;
use crate::assembler::exception_owners::OrdinaryAwaitEnd;
use crate::assembler::ir::UnoptimizedCode;
use crate::target::opcodes::{CALL, LOAD_GLOBAL, RERAISE, RETURN_VALUE};

fn label(index: u32) -> Label {
    Label::from_u32(index)
}

fn instruction(opcode: crate::target::Opcode, operand: Operand, line: i32) -> Instruction {
    Instruction::new(
        opcode,
        operand,
        SourceLocation::new(line, line, 0, 8),
        Some(0),
    )
}

fn fixture() -> Assembler {
    let op =
        |opcode, value, line| Item::Instruction(instruction(opcode, Operand::Value(value), line));
    let jump = |target, line| {
        Item::Instruction(instruction(
            JUMP_FORWARD,
            Operand::Forward(label(target)),
            line,
        ))
    };
    let mut marker = instruction(NOP, Operand::Value(0), -1);
    marker.location = SourceLocation::NONE;
    marker.fusion_policy.mark_converted_pop_block();
    marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    let send = instruction(SEND, Operand::Forward(label(9)), 12);
    let end_send = instruction(END_SEND, Operand::Value(0), 12);
    let items = vec![
        Item::Label(label(0)),
        jump(1, 1),
        Item::Label(label(1)),
        jump(4, 7),
        Item::Label(label(2)),
        jump(4, 9),
        Item::Label(label(3)),
        op(RERAISE, 1, -1),
        Item::Label(label(4)),
        Item::Label(label(5)),
        Item::Instruction(marker),
        Item::Label(label(6)),
        op(NOP, 0, 11),
        Item::Label(label(7)),
        Item::Label(label(8)),
        op(LOAD_GLOBAL, 1, 12),
        op(CALL, 0, 12),
        op(GET_AWAITABLE, 0, 12),
        op(LOAD_CONST, 0, 12),
        Item::Label(label(12)),
        Item::Instruction(send),
        Item::Label(label(13)),
        op(RERAISE, 1, -1),
        Item::Label(label(9)),
        Item::Instruction(end_send),
        op(POP_TOP, 0, 12),
        Item::Label(label(10)),
        jump(14, 12),
        Item::Label(label(11)),
        op(PUSH_EXC_INFO, 0, -1),
        op(RERAISE, 0, -1),
        Item::Label(label(14)),
        op(LOAD_CONST, 0, 15),
        op(RETURN_VALUE, 0, 15),
    ];
    let mut assembler = Assembler::from_linear_items(items);
    assembler.python_version = crate::PythonVersion::Py314;
    assembler.next_label = 15;
    assembler.exception_regions.push(ExceptionRegion {
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(label(8), label(10)),
        start: label(8),
        end: label(10),
        target: label(11),
        depth: 0,
        preserve_lasti: false,
    });
    let provenance = &mut assembler.exception_owner_provenance;
    provenance.source_boundaries.insert(label(4));
    provenance.block_boundaries.insert(label(4));
    provenance.setup_nop_ranges.push((label(6), label(7)));
    provenance
        .setups
        .push((label(8), Some(SourceLocation::new(11, 11, 0, 8))));
    provenance.ordinary_try_setups.insert(label(8), label(11));
    provenance.ordinary_await_ends.push(OrdinaryAwaitEnd {
        send,
        entry: label(9),
        end_send,
        cleanup: Vec::new(),
        bridge: None,
    });
    assembler
}

fn capture(assembler: &mut Assembler) {
    assembler.record_awaited_normal_finally_prefix(4);
    assert_eq!(
        assembler
            .exception_owner_provenance
            .awaited_normal_finally_prefixes
            .len(),
        1
    );
}

fn marker_location(pipeline: &OptimizationPipeline) -> SourceLocation {
    pipeline
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(record) if record.fusion_policy.is_converted_pop_block() => {
                Some(record.location)
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn awaited_normal_finally_replays_only_the_original_live_donor() {
    for prepared in [false, true] {
        let mut assembler = fixture();
        capture(&mut assembler);
        if prepared {
            assembler.used_constant_indices(1);
            assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
        }
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.propagate_awaited_normal_finally_entry_locations();
        assert_eq!(marker_location(&pipeline).line, 7);
        let once = pipeline.graph.copy_items();
        pipeline.propagate_awaited_normal_finally_entry_locations();
        assert_eq!(pipeline.graph.copy_items(), once);
    }
}

#[test]
fn awaited_normal_finally_rejects_each_changed_original_record() {
    let original = fixture();
    let protected = original.awaited_normal_finally_prefix(4).unwrap();
    let records = protected
        .prefix
        .iter()
        .chain(&protected.completion)
        .chain(&protected.handler)
        .chain(&protected.sources[0])
        .copied()
        .filter(|item| matches!(item, Item::Instruction(_)))
        .collect::<Vec<_>>();
    for original in records {
        for mutate in 0..6 {
            let mut assembler = fixture();
            capture(&mut assembler);
            let record = assembler
                .linear_items_mut()
                .iter_mut()
                .find(|item| **item == original)
                .unwrap();
            let Item::Instruction(record) = record else {
                unreachable!()
            };
            match mutate {
                0 => record.location.column += 1,
                1 => record.local_policy.add_hidden_exception_owner_slots(1),
                2 => record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                3 => record.no_location_policy.preserve_location(),
                4 => record.jump_policy.prevent_small_exit_inlining(),
                _ => record.fusion_policy.mark_converted_pop_block(),
            }
            if Item::Instruction(*record) == original {
                continue;
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            let before = marker_location(&pipeline);
            pipeline.propagate_awaited_normal_finally_entry_locations();
            assert_eq!(
                marker_location(&pipeline),
                before,
                "{original:?} mutation {mutate}"
            );
        }
    }
}

#[test]
fn awaited_normal_finally_authenticates_aliases_regions_and_source_occurrences() {
    for mutation in 0..20 {
        let mut assembler = fixture();
        capture(&mut assembler);
        match mutation {
            0 => assembler.exception_regions[0].depth += 1,
            1 => assembler.exception_regions[0].preserve_lasti = true,
            2 => assembler.exception_regions[0].is_scope_fallback = true,
            3 => assembler.exception_regions[0].end = label(14),
            4 => assembler.exception_regions[0].target = label(14),
            5 => {
                assembler.exception_regions[0].priority =
                    ExceptionRegionPriority::OriginalRange(label(7), label(10));
            }
            6 => assembler
                .exception_regions
                .push(assembler.exception_regions[0]),
            7 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                let index = items
                    .iter()
                    .position(|item| *item == Item::Label(label(8)))
                    .unwrap();
                items.insert(
                    index,
                    Item::Instruction(instruction(NOP, Operand::Value(0), 11)),
                );
            }
            8 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                items.push(Item::Label(label(8)));
                assert!(unique_label_positions(items).is_none());
                assert!(assembler.awaited_normal_finally_prefix(4).is_none());
                // BlockGraph also rejects this malformed input before replay.
                continue;
            }
            9 => {
                // The original donor survives locally, but a nonsnapshotted upstream
                // edge now bypasses it. Fresh reachability must revoke the donation.
                let Item::Instruction(jump) = &mut assembler.linear_items_mut()[1] else {
                    unreachable!()
                };
                jump.retarget(Operand::Forward(label(14)));
            }
            10 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                let donor = items[3];
                items.push(Item::Label(label(15)));
                items.push(donor);
            }
            11 => {
                // A moved record and identical replacement do not preserve the
                // original source occurrence, even when the entry operand is equal.
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                items.swap(2, 4);
            }
            12 | 13 => {
                let mut other = assembler.exception_regions[0];
                other.target = label(14);
                assembler.exception_regions.push(other);
                if mutation == 13 {
                    assembler.exception_regions.reverse();
                }
            }
            14 => assembler.exception_owner_provenance.setups[0].1 = None,
            15 => assembler.exception_owner_provenance.setup_nop_ranges[0].1 = label(8),
            16 => {
                let mut other = assembler.exception_regions[0];
                other.target = label(4);
                other.priority = ExceptionRegionPriority::OriginalRange(label(0), label(14));
                assembler.exception_regions.push(other);
            }
            17 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                items.insert(2, Item::Label(label(15)));
            }
            18 | 19 => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    unreachable!();
                };
                let endpoint = if mutation == 18 { 9 } else { 11 };
                let index = items
                    .iter()
                    .position(|item| *item == Item::Label(label(endpoint)))
                    .unwrap();
                items.insert(index, Item::Label(label(15)));
            }
            _ => unreachable!(),
        }
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.propagate_awaited_normal_finally_entry_locations();
        assert_eq!(
            marker_location(&pipeline),
            SourceLocation::NONE,
            "mutation {mutation}"
        );
    }
}

#[test]
fn awaited_normal_finally_constant_transport_is_identity_preserving() {
    for prepared in [false, true] {
        let mut assembler = fixture();
        capture(&mut assembler);
        if prepared {
            assembler.used_constant_indices(1);
        }
        assembler.remap_constant_indices(&[Some(3)]);
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.propagate_awaited_normal_finally_entry_locations();
        assert_eq!(marker_location(&pipeline).line, 7);

        let mut assembler = fixture();
        capture(&mut assembler);
        if prepared {
            assembler.used_constant_indices(1);
        }
        assembler.remap_constant_indices(&[None]);
        assert!(
            assembler
                .exception_owner_provenance
                .awaited_normal_finally_prefixes
                .is_empty()
        );
    }
}

pub(in crate::assembler) fn assert_common_constant_transport(
    lower: impl Fn(&mut OptimizationPipeline, &mut dyn crate::assembler::ConstantTable),
) {
    use crate::CompileError;
    use crate::code_object::{Constant, NonReflexiveRemap};
    use crate::target::opcodes::LOAD_COMMON_CONSTANT;

    struct Constants(Vec<Constant>);
    impl crate::assembler::ConstantTable for Constants {
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
            let mut retained = Vec::new();
            let mut mapping = Vec::new();
            for (index, value) in self.0.iter().enumerate() {
                if index == 0 || used[index] {
                    mapping.push(Some(u32::try_from(retained.len()).unwrap()));
                    retained.push(value.clone());
                } else {
                    mapping.push(None);
                }
            }
            self.0 = retained;
            mapping
        }
    }

    for (prepared, stale) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut assembler = fixture();
        assembler.python_version = crate::PythonVersion::Py315;
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            unreachable!()
        };
        let call = items
            .iter()
            .position(|item| matches!(item, Item::Instruction(record) if record.opcode == CALL))
            .unwrap();
        items.insert(
            call,
            Item::Instruction(instruction(LOAD_CONST, Operand::Value(2), 12)),
        );
        let none = items.iter().position(|item| matches!(item, Item::Instruction(record) if record.opcode == LOAD_CONST && record.operand() == Operand::Value(0))).unwrap();
        items.insert(
            none,
            Item::Instruction(instruction(PUSH_NULL, Operand::Value(0), 12)),
        );
        capture(&mut assembler);
        if prepared {
            assembler.used_constant_indices(3);
        }
        let mut pipeline = assembler.into_optimization_pipeline();
        if stale {
            pipeline
                .exception_owner_provenance
                .awaited_normal_finally_prefixes[0]
                .visit_records_mut(|record| {
                    if record.opcode == LOAD_CONST && record.operand() == Operand::Value(2) {
                        record.set_operand(Operand::Value(99));
                    }
                });
        }
        let mut constants = Constants(vec![
            Constant::None,
            Constant::Bool(true),
            Constant::SignedInt(1000),
        ]);
        lower(&mut pipeline, &mut constants);
        assert!(matches!(
            constants.0.as_slice(),
            [Constant::None, Constant::SignedInt(1000)]
        ));
        if stale {
            assert!(
                pipeline
                    .exception_owner_provenance
                    .awaited_normal_finally_prefixes
                    .is_empty()
            );
            pipeline.propagate_awaited_normal_finally_entry_locations();
            assert_eq!(marker_location(&pipeline), SourceLocation::NONE);
            continue;
        }
        let origin = &pipeline
            .exception_owner_provenance
            .awaited_normal_finally_prefixes[0];
        assert!(origin.prefix.iter().any(|item| matches!(item, Item::Instruction(record) if record.opcode == LOAD_COMMON_CONSTANT)));
        assert!(origin.prefix.iter().any(|item| matches!(item, Item::Instruction(record) if record.opcode == LOAD_CONST && record.operand() == Operand::Value(1))));
        pipeline.propagate_awaited_normal_finally_entry_locations();
        assert_eq!(marker_location(&pipeline).line, 7);
    }
}
