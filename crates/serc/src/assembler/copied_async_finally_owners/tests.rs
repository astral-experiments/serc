use super::*;
use crate::CompileError;
use crate::assembler::ConstantTable;
use crate::code_object::{Constant, NonReflexiveRemap};
use crate::target::opcodes::{JUMP_FORWARD, RETURN_VALUE};

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
    fn retain_used(&mut self, _: &[bool], _: &[u32], _: &[bool]) -> Vec<Option<u32>> {
        unreachable!()
    }
    fn retain_used_in_order(&mut self, _: &[bool]) -> Vec<Option<u32>> {
        unreachable!()
    }
}

fn loc() -> SourceLocation {
    SourceLocation::new(10, 10, 4, 21)
}
fn l(index: u32) -> Label {
    Label::from_u32(index)
}
fn value(opcode: crate::target::Opcode, argument: u32, depth: u32) -> Item {
    Item::Instruction(Instruction::value_with_depth(
        opcode,
        argument,
        loc(),
        depth,
    ))
}

fn append(assembler: &mut Assembler, items: impl IntoIterator<Item = Item>) {
    for item in items {
        assembler.push_linear_item(item);
    }
}

fn event(
    assembler: &mut Assembler,
    base: u32,
    depth: u32,
    asynchronous: bool,
) -> CopiedFinallyManagerEvent {
    let start = assembler.linear_items().len();
    append(
        assembler,
        [
            Item::Label(l(base)),
            value(SWAP, 3, depth),
            value(SWAP, 2, depth),
            value(LOAD_CONST, 0, depth + 1),
            value(LOAD_CONST, 0, depth + 2),
            value(LOAD_CONST, 0, depth + 3),
            value(CALL, 3, depth - 1),
        ],
    );
    if asynchronous {
        append(
            assembler,
            [
                value(GET_AWAITABLE, 2, depth - 1),
                value(LOAD_CONST, 0, depth),
                Item::Label(l(base + 1)),
                Item::Instruction(Instruction::new(
                    SEND,
                    Operand::Forward(l(base + 5)),
                    loc(),
                    Some(depth),
                )),
                Item::Label(l(base + 2)),
                value(YIELD_VALUE, 1, depth),
                Item::Label(l(base + 3)),
                value(RESUME, ResumeLocation::AFTER_AWAIT.argument(), depth),
                Item::Instruction(Instruction::new(
                    JUMP_BACKWARD_NO_INTERRUPT,
                    Operand::Backward(l(base + 1)),
                    loc(),
                    Some(depth),
                )),
                Item::Label(l(base + 4)),
                value(CLEANUP_THROW, 0, depth),
                Item::Label(l(base + 5)),
                value(END_SEND, 0, depth - 1),
                value(POP_TOP, 0, depth - 2),
            ],
        );
        assembler.exception_regions.push(ExceptionRegion {
            start: l(base + 2),
            end: l(base + 3),
            target: l(base + 4),
            depth,
            preserve_lasti: false,
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(l(base + 2), l(base + 3)),
        });
    } else {
        append(assembler, [value(POP_TOP, 0, depth - 2)]);
    }
    assembler
        .capture_copied_finally_manager_event(
            start,
            l(base),
            0,
            asynchronous,
            loc(),
            &Constants(vec![Constant::None]),
        )
        .unwrap()
}

fn fixture() -> Assembler {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    append(&mut assembler, [Item::Label(l(0)), value(LOAD_FAST, 0, 5)]);
    append(
        &mut assembler,
        [
            Item::Label(l(2)),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(l(9)),
                loc(),
                Some(5),
            )),
            Item::Label(l(3)),
            value(NOP, 0, 5),
        ],
    );
    assembler.record_ordinary_try_exception_setup(l(0), false, l(91));
    append(&mut assembler, [Item::Label(l(9))]);
    let asynchronous = event(&mut assembler, 10, 5, true);
    let synchronous = event(&mut assembler, 30, 3, false);
    append(&mut assembler, [Item::Label(l(40)), Item::Label(l(41))]);
    let pop = assembler
        .linear_items()
        .iter()
        .filter(|item| record(item).is_some())
        .count();
    let mut removed = Instruction::value_with_depth(NOP, 0, SourceLocation::NONE, 1);
    removed.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
    removed.fusion_policy.mark_converted_pop_block();
    let mut branch = Instruction::new(POP_JUMP_IF_NONE, Operand::Forward(l(42)), loc(), Some(1));
    branch.local_policy.add_hidden_exception_owner_slots(3);
    let mut marker = Instruction::value_with_depth(NOT_TAKEN, 0, loc(), 1);
    marker.set_exception_policy(ExceptionPolicy::NonePresent);
    append(
        &mut assembler,
        [
            Item::Instruction(removed),
            value(LOAD_FAST, 1, 2),
            Item::Instruction(branch),
            Item::Instruction(marker),
        ],
    );
    append(
        &mut assembler,
        [
            value(LOAD_CONST, 0, 2),
            value(RETURN_VALUE, 0, 1),
            Item::Label(l(42)),
            value(LOAD_CONST, 0, 2),
            value(RETURN_VALUE, 0, 1),
            Item::Label(l(90)),
            Item::Label(l(91)),
            value(LOAD_CONST, 0, 1),
            value(RETURN_VALUE, 0, 0),
            Item::Label(l(92)),
            Item::Instruction(Instruction::new(
                JUMP_BACKWARD_NO_INTERRUPT,
                Operand::Backward(l(9)),
                loc(),
                Some(5),
            )),
        ],
    );
    assembler.exception_regions.push(ExceptionRegion {
        start: l(0),
        end: l(40),
        target: l(91),
        depth: 0,
        preserve_lasti: false,
        is_scope_fallback: false,
        priority: ExceptionRegionPriority::OriginalRange(l(0), l(90)),
    });
    assembler
        .exception_owner_provenance
        .ordinary_try_region_origins
        .push((l(0), l(0), l(40), l(91)));
    assembler.next_label = 100;
    assembler.record_copied_async_finally_owner(
        vec![asynchronous, synchronous],
        (l(0), l(90), l(91)),
        [l(40), l(41)],
        pop,
        1,
        loc(),
        loc(),
    );
    assert_eq!(
        assembler
            .exception_owner_provenance
            .copied_async_finally_owners
            .len(),
        1
    );
    assembler
}

#[test]
fn copied_async_finally_seal_binds_every_emitted_record_and_alias() {
    for mutation in 0..16 {
        let mut assembler = fixture();
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            unreachable!()
        };
        let start = items
            .iter()
            .position(|item| *item == Item::Label(l(10)))
            .unwrap();
        match mutation {
            0 => {}
            1 => {
                if let Item::Instruction(instruction) = &mut items[start + 1] {
                    instruction.local_policy.add_hidden_exception_owner_slots(1);
                }
            }
            2 => {
                if let Item::Instruction(instruction) = &mut items[start + 2] {
                    instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                }
            }
            3 => {
                if let Item::Instruction(instruction) = &mut items[start + 3] {
                    instruction.set_operand(Operand::Value(1));
                }
            }
            4 => {
                items.insert(start, Item::Label(l(99)));
            }
            5 => {
                items.insert(start + 1, Item::Label(l(10)));
            }
            6 => {
                items.swap(start + 1, start + 3);
            }
            7 => {
                items.remove(start + 7);
            }
            8 => {
                assembler.exception_regions[0].depth += 1;
            }
            9 => {
                assembler.exception_regions[0].preserve_lasti = true;
            }
            10 => {
                assembler
                    .exception_regions
                    .push(assembler.exception_regions[0]);
            }
            11 => {
                assembler.exception_owner_provenance.setups[0].1 = Some(loc());
            }
            12 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(l(0), l(42));
            }
            13 => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_region_origins
                    .clear();
            }
            14 => {
                items.insert(0, Item::Label(l(0)));
            }
            15 => {
                assembler.exception_regions[0].target = l(99);
            }
            _ => unreachable!(),
        }
        assembler.seal_copied_async_finally_owners();
        assert_eq!(
            assembler
                .exception_owner_provenance
                .copied_async_finally_owners
                .len(),
            usize::from(mutation == 0),
            "mutation {mutation}"
        );
    }
}

#[test]
fn copied_async_finally_stale_prepared_capture_is_refused() {
    let mut assembler = fixture();
    let origin = assembler
        .exception_owner_provenance
        .copied_async_finally_owners
        .pop()
        .unwrap();
    let pop = assembler.linear_items()[..origin.original_start + origin.pop_offset]
        .iter()
        .filter(|item| record(item).is_some())
        .count();
    let _ = assembler.used_constant_indices(1);
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    assert!(
        assembler
            .capture_copied_finally_manager_event(
                origin.events[0].start,
                l(10),
                0,
                true,
                loc(),
                &Constants(vec![Constant::None])
            )
            .is_none()
    );
    assert!(
        assembler
            .copied_async_finally_owner(
                origin.events,
                (l(0), l(90), l(91)),
                [l(40), l(41)],
                pop,
                1,
                loc(),
                loc()
            )
            .is_none()
    );
}

#[test]
fn copied_async_finally_complete_heads_reject_foreign_trailing_aliases() {
    for target in [l(0), l(42)] {
        let mut assembler = fixture();
        let UnoptimizedCode::Linear(items) = &mut assembler.code else {
            unreachable!()
        };
        let start = items
            .iter()
            .position(|item| *item == Item::Label(target))
            .unwrap();
        let length = head(items, start).unwrap().len();
        items.insert(start + length, Item::Label(l(100)));
        assembler.seal_copied_async_finally_owners();
        assert!(
            assembler
                .exception_owner_provenance
                .copied_async_finally_owners
                .is_empty()
        );

        for prepared in [false, true] {
            let mut pipeline = pipeline(prepared);
            pipeline.push_cold_blocks_to_end(None, None);
            let origin = &pipeline
                .exception_owner_provenance
                .copied_async_finally_owners[0];
            let items = graph_items(&pipeline.graph);
            let start = anchored_start(&items, &origin.original).unwrap();
            let positions = pipeline.graph.item_positions();
            let pop = positions[start + origin.pop_offset];
            let load = positions[start + origin.pop_offset + 1];
            pipeline.transport_copied_async_finally_pop(&[pop], &[(load, 1)]);
            pipeline
                .graph
                .instruction_mut(load)
                .unwrap()
                .local_policy
                .add_hidden_exception_owner_slots(1);
            pipeline.graph.remove_item(pop);
            let origin = &pipeline
                .exception_owner_provenance
                .copied_async_finally_owners[0];
            assert!(origin.graph_unchanged(&pipeline.graph, &pipeline.exception_regions, true));
            let mut layout = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items: pipeline.graph.into_items(),
                exception_regions: pipeline.exception_regions,
                exception_owner_provenance: pipeline.exception_owner_provenance,
                preserved_block_boundaries: FxHashSet::default(),
                removed_max_depth: None,
                next_label: pipeline.next_label,
                has_next_location: false,
            };
            assert_eq!(layout.copied_async_finally_null_markers().len(), 1);
            let start = layout
                .items
                .iter()
                .position(|item| *item == Item::Label(target))
                .unwrap();
            let length = head(&layout.items, start).unwrap().len();
            layout.items.insert(start + length, Item::Label(l(200)));
            let origin = &layout
                .exception_owner_provenance
                .copied_async_finally_owners[0];
            assert!(!origin.records_unchanged(&layout.items, true));
            assert!(layout.copied_async_finally_null_markers().is_empty());
        }
    }
}

#[test]
fn copied_async_finally_constant_remaps_keep_none_identity_or_revoke() {
    for prepared in [false, true] {
        for removed in [false, true] {
            let mut assembler = fixture();
            if prepared {
                let _ = assembler.used_constant_indices(1);
            }
            assembler.seal_copied_async_finally_owners();
            let mut pipeline = assembler.into_optimization_pipeline();
            let origin = &mut pipeline
                .exception_owner_provenance
                .copied_async_finally_owners[0];
            assert_eq!(
                origin.remap_constants(&[if removed { None } else { Some(4) }], None),
                !removed
            );
            if !removed {
                assert!(origin.events.iter().all(|event| event.none == 4));
                assert!(
                    origin
                        .events
                        .iter()
                        .flat_map(|event| &event.items)
                        .filter_map(record)
                        .filter(|record| record.opcode == LOAD_CONST)
                        .all(|record| record.operand() == Operand::Value(4))
                );
            }
        }
    }
}

fn pipeline(prepared: bool) -> OptimizationPipeline {
    let mut assembler = fixture();
    if prepared {
        let _ = assembler.used_constant_indices(1);
    }
    assembler.into_optimization_pipeline()
}

#[test]
fn copied_async_finally_normalization_preserves_complete_ordered_items() {
    for prepared in [false, true] {
        for mutation in 0..6 {
            let mut pipeline = pipeline(prepared);
            let mut origins = std::mem::take(
                &mut pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners,
            );
            let mut receipt = CopiedFinallyNormalization::capture(
                &pipeline.graph,
                &pipeline.exception_regions,
                &mut origins,
            )
            .unwrap();
            let mut items = graph_items(&pipeline.graph);
            match mutation {
                0 => {}
                1 => {
                    receipt.0.insert(0, Item::Label(l(10)));
                }
                2 => {
                    let last = items.pop().unwrap();
                    items.insert(0, last);
                }
                3 => {
                    items.swap(1, 2);
                }
                4 => {
                    items[1] = value(LOAD_FAST, 99, 5);
                }
                5 => {
                    items.insert(0, Item::Label(l(99)));
                }
                _ => unreachable!(),
            }
            pipeline.graph = BlockGraph::from_items(items, &FxHashSet::default());
            receipt.finish(&pipeline.graph, &mut origins);
            assert_eq!(
                origins.len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn copied_async_finally_marker_head_requires_the_actual_adjacent_fresh_block() {
    for prepared in [false, true] {
        for mutation in 0..7 {
            let mut pipeline = pipeline(prepared);
            let mut origins = std::mem::take(
                &mut pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners,
            );
            let origin = &mut origins[0];
            assert!(origin.bind_graph_heads(&pipeline.graph));
            let marker = *origin.original.last().unwrap();
            let marker_block = pipeline
                .graph
                .iter()
                .find(|(_, block)| block.items().first() == Some(&marker))
                .unwrap()
                .0;
            let mut selected = marker_block;
            let mut fresh = l(100);
            match mutation {
                0 => {}
                1 => {
                    selected = pipeline.graph.label_block(l(10)).unwrap();
                }
                2 => {
                    fresh = l(10);
                }
                3 => {
                    pipeline.graph.insert_label(marker_block, 0, l(101));
                }
                4 => {
                    pipeline
                        .graph
                        .instruction_mut(ItemPosition::new(marker_block, 0))
                        .unwrap()
                        .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                }
                5 => {
                    let source = pipeline.graph.label_block(l(40)).unwrap();
                    pipeline
                        .graph
                        .instructions_mut(source)
                        .next_back()
                        .unwrap()
                        .set_operand(Operand::Forward(l(91)));
                }
                6 => {
                    origin.original.insert(0, origin.original[0]);
                }
                _ => unreachable!(),
            }
            origin.prepend_marker_head(&pipeline.graph, selected, fresh);
            assert_eq!(
                origin.marker_label,
                (mutation == 0).then_some(l(100)),
                "mutation {mutation}"
            );
            if mutation == 0 {
                pipeline.graph.insert_label(selected, 0, fresh);
                assert!(origin.graph_unchanged(
                    &pipeline.graph,
                    &pipeline.exception_regions,
                    false
                ));
            }
        }
    }
}

#[test]
fn copied_async_finally_manager_capture_rejects_noncanonical_policy_and_constants() {
    for mutation in 0..24 {
        let mut assembler = fixture();
        let original = &assembler
            .exception_owner_provenance
            .copied_async_finally_owners[0]
            .events[0];
        let start = original.start;
        let end = start + original.items.len();
        while assembler.linear_items().len() > end {
            assembler.pop_linear_item();
        }
        let mut constants = Constants(vec![Constant::None]);
        if (1..=16).contains(&mutation) {
            let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                unreachable!()
            };
            let instruction = items[start..]
                .iter_mut()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .nth(mutation - 1);
            if let Some(instruction) = instruction {
                instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            }
        } else {
            match mutation {
                0 => {}
                17 => {
                    constants.0[0] = Constant::Bool(false);
                }
                18 => {
                    assembler.exception_regions[0].depth += 1;
                }
                19 => {
                    assembler.exception_regions[0].preserve_lasti = true;
                }
                20 => {
                    assembler.exception_regions[0].target = l(42);
                }
                21 => {
                    assembler
                        .exception_regions
                        .push(assembler.exception_regions[0]);
                }
                22 => {
                    assembler.exception_regions[0].priority =
                        ExceptionRegionPriority::OriginalRange(l(11), l(13));
                }
                23 => {
                    constants.0.clear();
                }
                _ => unreachable!(),
            }
        }
        // The canonical event has 15 instructions; no mutation is needed for its absent sixteenth.
        let expected = mutation == 0 || mutation == 16;
        assert_eq!(
            assembler
                .capture_copied_finally_manager_event(start, l(10), 0, true, loc(), &constants)
                .is_some(),
            expected,
            "mutation {mutation}"
        );
    }
}

#[test]
fn copied_async_finally_requires_the_real_removed_pop_and_single_donation() {
    for prepared in [false, true] {
        for mutation in 0..9 {
            let mut pipeline = pipeline(prepared);
            let original = &pipeline
                .exception_owner_provenance
                .copied_async_finally_owners[0];
            let items = graph_items(&pipeline.graph);
            let start = anchored_start(&items, &original.original).unwrap();
            let positions = pipeline.graph.item_positions();
            let pop = positions[start + original.pop_offset];
            let load = positions[start + original.pop_offset + 1];
            let mut removed = vec![pop];
            let mut donations = vec![(load, 1)];
            match mutation {
                0 => {}
                1 => {
                    removed.clear();
                }
                2 => {
                    donations.clear();
                }
                3 => {
                    donations[0].1 = 2;
                }
                4 => {
                    removed.push(load);
                }
                5 => {
                    pipeline.graph.instruction_mut(pop).unwrap().location = loc();
                }
                6 => {
                    pipeline
                        .graph
                        .instruction_mut(load)
                        .unwrap()
                        .set_operand(Operand::Value(7));
                }
                7 => {
                    removed.push(pop);
                }
                8 => {
                    donations.push((load, 1));
                }
                _ => unreachable!(),
            }
            pipeline.transport_copied_async_finally_pop(&removed, &donations);
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners[0]
                    .pop_removed,
                mutation == 0,
                "mutation {mutation}"
            );
        }
    }
}

#[test]
fn copied_async_finally_real_cold_writer_binds_each_original_manager_array() {
    for prepared in [false, true] {
        let mut pipeline = pipeline(prepared);
        pipeline.push_cold_blocks_to_end(None, None);
        let origins = &pipeline
            .exception_owner_provenance
            .copied_async_finally_owners;
        assert_eq!(origins.len(), 1);
        let origin = &origins[0];
        assert!(origin.events[0].bridge.is_some());
        assert!(origin.events[1].bridge.is_none());
        assert!(origin.marker_label.is_some());
        assert!(origin.graph_heads.len() > 1);
        assert!(origin.graph_unchanged(&pipeline.graph, &pipeline.exception_regions, true));
        assert!(same_regions(
            origin.regions.as_ref().unwrap(),
            &pipeline.exception_regions
        ));
    }
}

#[test]
fn copied_async_finally_threaded_regions_require_indexed_complete_writer_events() {
    for prepared in [false, true] {
        for mutation in 0..12 {
            let mut pipeline = pipeline(prepared);
            let mut origins = std::mem::take(
                &mut pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners,
            );
            let items = graph_items(&pipeline.graph);
            let labels = unique_label_positions(&items).unwrap();
            let positions = pipeline
                .graph
                .item_positions()
                .into_iter()
                .filter(|position| record(pipeline.graph.item(*position)).is_some())
                .collect::<Vec<_>>();
            let source = 1;
            let target = positions.len() - 1;
            let source_item = items
                .iter()
                .enumerate()
                .filter(|(_, item)| record(item).is_some())
                .nth(source)
                .unwrap()
                .0;
            let target_item = items
                .iter()
                .enumerate()
                .filter(|(_, item)| record(item).is_some())
                .nth(target)
                .unwrap()
                .0;
            let mut receipt = CopiedFinallyThreadRewrite::capture(
                &pipeline.graph,
                &pipeline.exception_regions,
                &mut origins,
                &[(source, target)],
            )
            .unwrap();
            let mut output = Vec::new();
            let mut lineage = Vec::new();
            for (index, region) in pipeline.exception_regions.iter().enumerate() {
                let range = labels[&region.start]..labels[&region.end];
                assert!(!range.contains(&target_item));
                if range.contains(&source_item) {
                    output.push(ExceptionRegion {
                        end: l(100),
                        ..*region
                    });
                    output.push(ExceptionRegion {
                        start: l(101),
                        ..*region
                    });
                    lineage.extend([index, index]);
                } else {
                    output.push(*region);
                    lineage.push(index);
                }
            }
            let position = positions[source];
            pipeline
                .graph
                .insert_label(position.block(), position.offset() + 1, l(101));
            pipeline
                .graph
                .insert_label(position.block(), position.offset(), l(100));
            receipt.emitted(source, target, l(100), l(101));
            match mutation {
                0 => {}
                1 => {
                    receipt.events.clear();
                }
                2 => {
                    receipt.events.push(receipt.events[0]);
                }
                3 => {
                    receipt.events[0].0 += 1;
                }
                4 => {
                    receipt.events[0].1 -= 1;
                }
                5 => {
                    receipt.events[0].2 = l(99);
                }
                6 => {
                    lineage.pop();
                }
                7 => {
                    lineage[0] = 1;
                }
                8 => {
                    output[0].depth += 1;
                }
                9 => {
                    output.swap(0, 1);
                }
                10 => {
                    receipt.regions[0].preserve_lasti = true;
                }
                11 => {
                    output.pop();
                    lineage.pop();
                }
                _ => unreachable!(),
            }
            receipt.finish(&pipeline.graph, &output, &lineage, &mut origins);
            assert_eq!(
                origins.len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn copied_async_finally_cold_regions_reject_incomplete_or_changed_lineage() {
    for prepared in [false, true] {
        for mutation in 0..19 {
            let mut assembler = fixture();
            assembler.exception_regions.push(ExceptionRegion {
                start: l(2),
                end: l(2),
                target: l(2),
                depth: 0,
                preserve_lasti: false,
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(l(2), l(2)),
            });
            if prepared {
                let _ = assembler.used_constant_indices(1);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            for block in pipeline.graph.order().to_vec() {
                if pipeline.graph[block].first_label().is_none() {
                    let fresh = l(pipeline.next_label);
                    pipeline.next_label += 1;
                    for origin in &mut pipeline
                        .exception_owner_provenance
                        .copied_async_finally_owners
                    {
                        origin.prepend_marker_head(&pipeline.graph, block, fresh);
                    }
                    pipeline.graph.insert_label(block, 0, fresh);
                }
            }
            let mut receipt = CopiedFinallyColdRewrite::capture(
                &pipeline.graph,
                &pipeline.exception_regions,
                &mut pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners,
            )
            .unwrap();
            let mut emitted = Vec::new();
            pipeline.push_cold_blocks_to_end(Some(&mut emitted), None);
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners
                    .len(),
                1
            );
            // This fixture's original regions are unique; the test uses the real writer's
            // explicit old/new pairs to build an independently mutable event receipt.
            for (original, _) in emitted {
                let matches = receipt
                    .regions
                    .iter()
                    .enumerate()
                    .filter(|(_, region)| same_manager_region(region, &original))
                    .collect::<Vec<_>>();
                assert_eq!(matches.len(), 1);
                receipt.output(matches[0].0);
            }
            for index in 0..receipt.regions.len() {
                receipt.group(
                    index,
                    receipt
                        .outputs
                        .iter()
                        .filter(|original| **original == index)
                        .count(),
                );
            }
            assert_eq!(receipt.groups.last(), Some(&(2, 0)));
            for (block, contents) in pipeline.graph.iter() {
                if !receipt.heads.contains_key(&block) {
                    let [Item::Label(label), Item::Instruction(instruction)] = contents.items()
                    else {
                        panic!("unexpected cold output")
                    };
                    receipt.bridge(block, *label, *instruction);
                }
            }
            let mut origins = std::mem::take(
                &mut pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners,
            );
            origins[0].regions = Some(receipt.regions.clone());
            let mut regions = pipeline.exception_regions.clone();
            match mutation {
                0 => {}
                1 => {
                    receipt.outputs.pop();
                }
                2 => {
                    receipt.outputs[0] += 1;
                }
                3 => {
                    receipt.groups.pop();
                }
                4 => {
                    receipt.groups[0].1 += 1;
                }
                5 => {
                    receipt.groups.swap(0, 1);
                }
                6 => {
                    receipt.regions.swap(0, 1);
                }
                7 => {
                    receipt.regions[0].depth += 1;
                }
                8 => {
                    receipt.ranges[0].end += 1;
                }
                9 => {
                    receipt.bridges.clear();
                }
                10 => {
                    receipt.bridges.push(receipt.bridges[0]);
                }
                11 => {
                    regions[0].end = l(91);
                }
                12 => {
                    regions[0].preserve_lasti = true;
                }
                13 => {
                    origins[0].events[0].bridge = None;
                }
                14 => {
                    receipt.ranges[2].end += 1;
                }
                15 => {
                    receipt.regions[2].start = l(10);
                }
                16 => {
                    receipt.groups[2].0 = usize::MAX;
                }
                17 => {
                    receipt.groups.swap(1, 2);
                }
                18 => {
                    receipt.regions[2].end = l(9);
                }
                _ => unreachable!(),
            }
            receipt.finish(&pipeline.graph, &regions, None, &mut origins);
            assert_eq!(
                origins.len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn copied_async_finally_empty_regions_must_be_disjoint_from_every_witness() {
    for prepared in [false, true] {
        for mutation in 0..4 {
            let mut assembler = fixture();
            let mut empty = ExceptionRegion {
                start: l(2),
                end: l(2),
                target: l(2),
                depth: 0,
                preserve_lasti: false,
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(l(2), l(2)),
            };
            match mutation {
                0 => {}
                1 => {
                    empty.start = l(10);
                    empty.end = l(10);
                }
                2 => {
                    empty.target = l(91);
                }
                3 => {
                    empty.priority = assembler.exception_regions[0].priority;
                }
                _ => unreachable!(),
            }
            assembler.exception_regions.push(empty);
            if prepared {
                let _ = assembler.used_constant_indices(1);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            if pipeline
                .exception_owner_provenance
                .copied_async_finally_owners
                .is_empty()
            {
                assert_ne!(mutation, 0);
                continue;
            }
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners
                    .len(),
                1
            );
            pipeline.push_cold_blocks_to_end(None, None);
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners
                    .len(),
                usize::from(mutation == 0),
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn copied_async_finally_rechecks_upstream_liveness_and_original_occurrences() {
    for prepared in [false, true] {
        for mutation in 0..4 {
            let mut pipeline = pipeline(prepared);
            let mut origins = std::mem::take(
                &mut pipeline
                    .exception_owner_provenance
                    .copied_async_finally_owners,
            );
            assert!(origins[0].bind_graph_heads(&pipeline.graph));
            match mutation {
                0 => {}
                1 => {
                    let upstream = pipeline.graph.label_block(l(2)).unwrap();
                    pipeline
                        .graph
                        .instructions_mut(upstream)
                        .next()
                        .unwrap()
                        .set_operand(Operand::Forward(l(91)));
                }
                2 => {
                    let block = pipeline.graph.label_block(l(10)).unwrap();
                    pipeline
                        .graph
                        .instructions_mut(block)
                        .next()
                        .unwrap()
                        .local_policy
                        .add_hidden_exception_owner_slots(1);
                }
                3 => {
                    let source = pipeline.graph.label_block(l(10)).unwrap();
                    let foreign = pipeline.graph.label_block(l(92)).unwrap();
                    let expected = origins[0].events[0].items[1];
                    pipeline
                        .graph
                        .instructions_mut(source)
                        .next()
                        .unwrap()
                        .set_operand(Operand::Value(2));
                    pipeline
                        .graph
                        .insert_item(ItemPosition::new(foreign, 1), expected);
                }
                _ => unreachable!(),
            }
            assert_eq!(
                origins[0].graph_unchanged(&pipeline.graph, &pipeline.exception_regions, false),
                mutation == 0,
                "prepared {prepared}, mutation {mutation}"
            );
        }
    }
}
