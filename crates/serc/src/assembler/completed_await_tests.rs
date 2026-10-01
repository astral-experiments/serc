use super::{
    Assembler, CompletedAwaitTrySetups, ExceptionPolicy, ExceptionRegionPriority, Instruction,
    Item, Label, Operand, SourceLocation, UnoptimizedCode, block_graph,
};
use crate::target::Opcode;
use crate::target::opcodes::{
    CLEANUP_THROW, END_SEND, JUMP_BACKWARD_NO_INTERRUPT, LOAD_CONST, NOP, POP_TOP, RESUME, SEND,
    YIELD_VALUE,
};
use crate::target::operands::ResumeLocation;
use rustc_hash::FxHashSet;

fn fixture(
    retained: bool,
) -> (
    Assembler,
    CompletedAwaitTrySetups,
    [Label; 4],
    [(Label, Label); 2],
) {
    let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
    let mut pairs = [(Label::from_u32(0), Label::from_u32(0)); 2];
    for (index, pair) in pairs.iter_mut().enumerate() {
        let start = assembler.label();
        let after = assembler.label();
        let setup = assembler.label();
        let handler = assembler.label();
        assembler.mark(start);
        let line = 10 + i32::try_from(index).unwrap();
        assembler.set_location(SourceLocation::new(line, line, 0, 8));
        if retained {
            assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
        }
        assembler.mark(after);
        if retained {
            assembler.record_setup_nop_boundaries(start, after);
        }
        assembler.record_ordinary_try_exception_setup(setup, retained, handler);
        assembler.mark(setup);
        *pair = (setup, handler);
    }
    let captured = assembler.capture_completed_await_try_setups(pairs).unwrap();
    let sending = assembler.label();
    let yielded = assembler.label();
    let resumed = assembler.label();
    let cleanup = assembler.label();
    let completed = assembler.label();
    assembler.set_location(SourceLocation::new(20, 20, 4, 21));
    assembler.emit(LOAD_CONST, 2);
    assembler.mark(sending);
    assembler.emit_forward_with_depth(SEND, completed, 1, ExceptionPolicy::default());
    assembler.mark(yielded);
    assembler.emit(YIELD_VALUE, 1);
    assembler.mark(resumed);
    assembler.emit(RESUME, ResumeLocation::AFTER_AWAIT.argument());
    assembler.emit_backward(JUMP_BACKWARD_NO_INTERRUPT, sending);
    assembler.mark(cleanup);
    assembler.emit(CLEANUP_THROW, 0);
    assembler.mark(completed);
    assembler.emit(END_SEND, 0);
    assembler.add_exception_region(yielded, resumed, cleanup, 1, false);
    assembler.record_ordinary_await_cleanup(yielded, cleanup);
    let aliases = [
        assembler.label(),
        assembler.label(),
        assembler.label(),
        assembler.label(),
    ];
    for label in aliases {
        assembler.mark(label);
    }
    (assembler, captured, aliases, pairs)
}

fn record_mut(assembler: &mut Assembler, opcode: Opcode) -> &mut Instruction {
    assembler
        .linear_items_mut()
        .iter_mut()
        .find_map(|item| match item {
            Item::Instruction(record) if record.opcode == opcode => Some(record),
            _ => None,
        })
        .unwrap()
}

#[test]
fn completed_await_pop_frontier_requires_full_setup_and_await_records() {
    for mutation in [
        "intact",
        "setup location",
        "setup opcode",
        "setup policy",
        "setup argument",
        "setup provenance location",
        "setup provenance duplicate",
        "setup target",
        "setup alias",
        "setup duplicate record",
        "send location",
        "send operand",
        "end location",
        "end argument",
        "cleanup location",
        "region start",
        "region end",
        "region target",
        "region depth",
        "region lasti",
        "region fallback",
        "region priority",
        "region duplicate",
        "region competing",
        "region removed",
        "await provenance removed",
        "await provenance duplicate",
        "foreign alias",
        "alias reorder",
        "alias missing",
        "alias duplicate",
        "interposed operation",
        "unrelated region",
        "setups after send",
        "ambiguous competing endpoint",
    ] {
        let (mut assembler, captured, aliases, pairs) = fixture(true);
        let foreign = assembler.label();
        match mutation {
            "setup location" => record_mut(&mut assembler, NOP).location.column += 1,
            "setup opcode" => record_mut(&mut assembler, NOP).opcode = POP_TOP,
            "setup policy" => {
                record_mut(&mut assembler, NOP).set_exception_policy(ExceptionPolicy::NonePresent);
            }
            "setup argument" => record_mut(&mut assembler, NOP).set_operand(Operand::Value(3)),
            "setup provenance location" => assembler.exception_owner_provenance.setups[0].1 = None,
            "setup provenance duplicate" => {
                let duplicate = assembler.exception_owner_provenance.setups[0];
                assembler.exception_owner_provenance.setups.push(duplicate);
            }
            "setup target" => {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .insert(pairs[0].0, foreign);
            }
            "setup alias" => {
                let at = assembler
                    .linear_items()
                    .iter()
                    .position(|item| *item == Item::Label(pairs[0].0))
                    .unwrap();
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    panic!()
                };
                items.insert(at + 1, Item::Label(foreign));
            }
            "setup duplicate record" => {
                let copied = *record_mut(&mut assembler, NOP);
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    panic!()
                };
                items.insert(0, Item::Instruction(copied));
            }
            "send location" => record_mut(&mut assembler, SEND).location.column += 1,
            "send operand" => {
                record_mut(&mut assembler, SEND).set_operand(Operand::Forward(foreign));
            }
            "end location" => record_mut(&mut assembler, END_SEND).location.column += 1,
            "end argument" => record_mut(&mut assembler, END_SEND).set_operand(Operand::Value(1)),
            "cleanup location" => record_mut(&mut assembler, CLEANUP_THROW).location.column += 1,
            "region start" => assembler.exception_regions[0].start = foreign,
            "region end" => assembler.exception_regions[0].end = foreign,
            "region target" => assembler.exception_regions[0].target = foreign,
            "region depth" => assembler.exception_regions[0].depth += 1,
            "region lasti" => assembler.exception_regions[0].preserve_lasti = true,
            "region fallback" => assembler.exception_regions[0].is_scope_fallback = true,
            "region priority" => {
                assembler.exception_regions[0].priority = ExceptionRegionPriority::Resolved {
                    start: 99,
                    end: 100,
                }
            }
            "region duplicate" => assembler
                .exception_regions
                .push(assembler.exception_regions[0]),
            "region competing" => {
                let original = assembler.exception_regions[0];
                assembler.add_exception_region(original.start, original.end, foreign, 2, false);
            }
            "region removed" => assembler.exception_regions.clear(),
            "await provenance removed" => assembler
                .exception_owner_provenance
                .ordinary_await_cleanups
                .clear(),
            "await provenance duplicate" => {
                let original = assembler.exception_owner_provenance.ordinary_await_cleanups[0];
                assembler
                    .exception_owner_provenance
                    .ordinary_await_cleanups
                    .push(original);
            }
            "foreign alias" => assembler.mark(foreign),
            "alias reorder" => {
                let len = assembler.linear_items().len();
                assembler.linear_items_mut().swap(len - 1, len - 2);
            }
            "alias missing" => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    panic!()
                };
                items.pop();
            }
            "alias duplicate" => assembler.mark(aliases[0]),
            "interposed operation" => {
                assembler.emit(NOP, 0);
            }
            "unrelated region" => {
                assembler.add_exception_region(pairs[0].0, pairs[1].0, foreign, 0, false);
            }
            "setups after send" => {
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    panic!()
                };
                let prefix = items.drain(..9).collect::<Vec<_>>();
                let send = items
                    .iter()
                    .position(
                        |item| matches!(item, Item::Instruction(record) if record.opcode == SEND),
                    )
                    .unwrap();
                let after_send = send + 1;
                items.splice(after_send..after_send, prefix);
            }
            "ambiguous competing endpoint" => {
                let original = assembler.exception_regions[0];
                let UnoptimizedCode::Linear(items) = &mut assembler.code else {
                    panic!()
                };
                let send = items
                    .iter()
                    .position(
                        |item| matches!(item, Item::Instruction(record) if record.opcode == SEND),
                    )
                    .unwrap();
                items.insert(send, Item::Label(foreign));
                let end = items
                    .iter()
                    .position(|item| *item == Item::Label(original.end))
                    .unwrap();
                items.insert(end, Item::Label(foreign));
                let handler = assembler.label();
                assembler.add_exception_region(pairs[0].0, foreign, handler, 0, false);
            }
            "intact" => {}
            _ => unreachable!(),
        }
        assert_eq!(
            assembler.completed_ordinary_await_has_unwind_frontier(aliases, &captured),
            matches!(mutation, "intact" | "unrelated region"),
            "{mutation}"
        );
    }
}

#[test]
fn completed_await_pop_frontier_preserves_zero_width_setups_and_remapping() {
    for retained in [false, true] {
        let (mut assembler, captured, aliases, pairs) = fixture(retained);
        assert!(assembler.completed_ordinary_await_has_unwind_frontier(aliases, &captured));
        assembler.remap_constant_indices(&[Some(0), None, Some(1)]);
        assert!(assembler.completed_ordinary_await_has_unwind_frontier(aliases, &captured));
        assert!(assembler.completed_ordinary_await_has_unwind_frontier(aliases, &captured));
        let graph = block_graph::BlockGraph::from_items(
            assembler.linear_items().to_vec(),
            &FxHashSet::default(),
        );
        let reachable = graph.reachable_blocks(&assembler.exception_regions);
        assembler.code = UnoptimizedCode::Prepared { graph, reachable };
        assert!(
            assembler
                .capture_completed_await_try_setups(pairs)
                .is_none()
        );
        assert!(!assembler.completed_ordinary_await_has_unwind_frontier(aliases, &captured));
    }
}
