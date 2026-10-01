use super::ir::{EmittedInstruction, JumpPolicy};
use super::{
    Assembler, EmittedJump, ExceptionPolicy, Instruction, Item, Label, Operand, SourceLocation,
};
use crate::PythonVersion;
use crate::target::opcodes::*;

struct Join {
    assembler: Assembler,
    source: EmittedJump,
    continuation: Label,
    condition: SourceLocation,
    guard: usize,
    bypass: usize,
    aliases: [Label; 3],
}

fn fixture(version: PythonVersion) -> Join {
    let mut assembler = Assembler::with_python_version(version);
    let aliases = [assembler.label(), assembler.label(), assembler.label()];
    let continuation = assembler.label();
    let condition = SourceLocation::new(5, 5, 7, 11);
    assembler.set_location(SourceLocation::new(2, 2, 4, 8));
    assembler.emit_with_depth(NOP, 0, 2, ExceptionPolicy::default());
    let guard = assembler.item_count();
    assembler.set_location(condition);
    assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, aliases[0], 2, ExceptionPolicy::default());
    assembler.set_location(SourceLocation::new(7, 7, 8, 14));
    assembler.emit_with_depth(LOAD_CONST, 0, 3, ExceptionPolicy::default());
    assembler.emit_with_depth(POP_TOP, 0, 2, ExceptionPolicy::default());
    let bypass = assembler.item_count();
    assembler.emit_forward_with_depth(JUMP_FORWARD, aliases[1], 2, ExceptionPolicy::default());
    assembler.set_location(SourceLocation::NONE);
    assembler.emit_with_depth(RERAISE, 1, 2, ExceptionPolicy::default());
    for alias in aliases {
        assembler.mark(alias);
    }
    for alias in &aliases[..2] {
        assembler.record_exception_owner_block_boundary(*alias);
    }
    assembler.set_location(condition);
    let source = assembler.emit_forward_with_depth(
        JUMP_FORWARD,
        continuation,
        2,
        ExceptionPolicy::default(),
    );
    assembler.preserve_redundant_jump_nop_after_threading(source);
    Join {
        assembler,
        source,
        continuation,
        condition,
        guard,
        bypass,
        aliases,
    }
}

fn instruction_mut(assembler: &mut Assembler, index: usize) -> &mut Instruction {
    let Item::Instruction(instruction) = &mut assembler.linear_items_mut()[index] else {
        unreachable!()
    };
    instruction
}

fn accepts(join: &mut Join) -> bool {
    join.assembler.original_normal_finally_manager_join(
        join.source,
        join.continuation,
        join.condition,
        1,
    )
}

#[test]
fn normal_manager_join_requires_complete_source_and_incoming_records() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for mutation in [
            "intact",
            "guard opcode",
            "guard backward",
            "guard span",
            "guard depth",
            "guard owner",
            "guard hidden",
            "guard folded",
            "guard policy",
            "guard outside",
            "bypass backward",
            "bypass opcode",
            "bypass unlocated",
            "bypass depth",
            "bypass owner",
            "bypass hidden",
            "bypass folded",
            "bypass policy",
            "duplicate guard",
            "duplicate bypass",
            "duplicate source",
            "foreign alias edge",
            "source target",
            "source opcode",
            "source backward",
            "source span",
            "source depth",
            "source owner",
            "source hidden",
            "source folded",
            "source policy",
            "source missing depth",
            "populated head",
            "located head",
            "foreign continuation",
            "condition span",
        ] {
            let mut join = fixture(version);
            let source = join.source.instruction().item_index as usize;
            let guard = *instruction_mut(&mut join.assembler, join.guard);
            let bypass = *instruction_mut(&mut join.assembler, join.bypass);
            let jump = *instruction_mut(&mut join.assembler, source);
            match mutation {
                "intact" => {}
                "guard opcode" => {
                    instruction_mut(&mut join.assembler, join.guard).opcode = JUMP_FORWARD;
                }
                "guard backward" => instruction_mut(&mut join.assembler, join.guard)
                    .set_operand(Operand::Backward(join.aliases[0])),
                "guard span" => {
                    instruction_mut(&mut join.assembler, join.guard)
                        .location
                        .column += 1;
                }
                "guard depth" => {
                    *instruction_mut(&mut join.assembler, join.guard) =
                        Instruction::new(guard.opcode, guard.operand(), guard.location, Some(3));
                }
                "guard owner" => instruction_mut(&mut join.assembler, join.guard)
                    .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "guard hidden" => instruction_mut(&mut join.assembler, join.guard)
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                "guard folded" => instruction_mut(&mut join.assembler, join.guard)
                    .add_pre_fold_extra_instructions(1),
                "guard policy" => instruction_mut(&mut join.assembler, join.guard)
                    .jump_policy
                    .defer_removal(),
                "guard outside" => {
                    join.assembler.linear_items_mut().swap(0, join.guard);
                }
                "bypass backward" => instruction_mut(&mut join.assembler, join.bypass)
                    .set_operand(Operand::Backward(join.aliases[1])),
                "bypass opcode" => {
                    instruction_mut(&mut join.assembler, join.bypass).opcode = POP_JUMP_IF_TRUE;
                }
                "bypass unlocated" => {
                    instruction_mut(&mut join.assembler, join.bypass).location =
                        SourceLocation::NONE;
                }
                "bypass depth" => {
                    *instruction_mut(&mut join.assembler, join.bypass) =
                        Instruction::new(bypass.opcode, bypass.operand(), bypass.location, Some(3));
                }
                "bypass owner" => instruction_mut(&mut join.assembler, join.bypass)
                    .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "bypass hidden" => instruction_mut(&mut join.assembler, join.bypass)
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                "bypass folded" => instruction_mut(&mut join.assembler, join.bypass)
                    .add_pre_fold_extra_instructions(1),
                "bypass policy" => instruction_mut(&mut join.assembler, join.bypass)
                    .jump_policy
                    .defer_removal(),
                "duplicate guard" => *instruction_mut(&mut join.assembler, 2) = guard,
                "duplicate bypass" => *instruction_mut(&mut join.assembler, 2) = bypass,
                "duplicate source" => *instruction_mut(&mut join.assembler, 2) = jump,
                "foreign alias edge" => {
                    *instruction_mut(&mut join.assembler, 2) = Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(join.aliases[2]),
                        condition(),
                        Some(2),
                    );
                }
                "source target" => instruction_mut(&mut join.assembler, source)
                    .set_operand(Operand::Forward(join.aliases[0])),
                "source opcode" => {
                    instruction_mut(&mut join.assembler, source).opcode = JUMP_BACKWARD;
                }
                "source backward" => instruction_mut(&mut join.assembler, source)
                    .set_operand(Operand::Backward(join.continuation)),
                "source span" => instruction_mut(&mut join.assembler, source).location.column += 1,
                "source depth" => {
                    *instruction_mut(&mut join.assembler, source) =
                        Instruction::new(jump.opcode, jump.operand(), jump.location, Some(3));
                }
                "source owner" => instruction_mut(&mut join.assembler, source)
                    .set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "source hidden" => instruction_mut(&mut join.assembler, source)
                    .local_policy
                    .add_hidden_exception_owner_slots(1),
                "source folded" => {
                    instruction_mut(&mut join.assembler, source).add_pre_fold_extra_instructions(1);
                }
                "source policy" => {
                    instruction_mut(&mut join.assembler, source).jump_policy =
                        JumpPolicy::default();
                }
                "source missing depth" => {
                    *instruction_mut(&mut join.assembler, source) =
                        Instruction::new(jump.opcode, jump.operand(), jump.location, None);
                }
                "populated head" | "located head" => {
                    join.assembler.linear_items_mut()[source - 1] =
                        Item::Instruction(Instruction::new(
                            NOP,
                            Operand::Value(0),
                            if mutation == "located head" {
                                condition()
                            } else {
                                SourceLocation::NONE
                            },
                            Some(2),
                        ));
                }
                "foreign continuation" => join.continuation = join.assembler.label(),
                "condition span" => join.condition.column += 1,
                _ => unreachable!(),
            }
            let before = join.assembler.linear_items().to_vec();
            assert_eq!(
                accepts(&mut join),
                mutation == "intact",
                "{version:?}: {mutation}"
            );
            assert_eq!(before, join.assembler.linear_items());
        }
    }
}

fn condition() -> SourceLocation {
    SourceLocation::new(5, 5, 7, 11)
}

#[test]
fn normal_manager_join_rejects_changed_aliases_and_original_boundaries() {
    for mutation in [
        "intact",
        "repeated alias",
        "bound continuation",
        "missing source",
        "missing block",
        "folded condition",
        "folded else",
        "folded exit",
        "omitted count",
        "omitted line",
        "authored nop",
        "borrow alias",
        "fusion boundary",
        "continuation boundary",
        "handler entry",
    ] {
        let mut join = fixture(PythonVersion::Py314);
        let alias = join.aliases[0];
        match mutation {
            "intact" => {}
            "repeated alias" => join.assembler.linear_items_mut()[0] = Item::Label(alias),
            "bound continuation" => {
                join.assembler.linear_items_mut()[0] = Item::Label(join.continuation);
            }
            "missing source" => {
                join.assembler
                    .exception_owner_provenance
                    .source_boundaries
                    .remove(&alias);
            }
            "missing block" => {
                join.assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .remove(&alias);
            }
            "folded condition" => {
                join.assembler.folded_away_condition_targets.insert(alias);
            }
            "folded else" => {
                join.assembler.folded_away_loop_else_targets.insert(alias);
            }
            "folded exit" => {
                join.assembler.folded_condition_exit_targets.insert(alias);
            }
            "omitted count" => {
                join.assembler
                    .exception_owner_provenance
                    .omitted_nop_counts
                    .insert(alias, 1);
            }
            "omitted line" => {
                join.assembler
                    .exception_owner_provenance
                    .omitted_nop_locations
                    .insert(alias, condition());
            }
            "authored nop" => {
                join.assembler.original_nop_targets.insert(alias);
            }
            "borrow alias" => {
                join.assembler.borrow_block_aliases.insert(alias);
            }
            "fusion boundary" => {
                join.assembler
                    .exception_owner_provenance
                    .fusion_boundaries
                    .insert(alias);
            }
            "continuation boundary" => {
                join.assembler
                    .exception_owner_provenance
                    .continuation_boundaries
                    .insert(alias);
            }
            "handler entry" => join.assembler.record_exception_setup(alias, false),
            _ => unreachable!(),
        }
        assert_eq!(accepts(&mut join), mutation == "intact", "{mutation}");
    }
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        assert!(!accepts(&mut fixture(version)));
    }
}

#[test]
fn normal_manager_join_cannot_reuse_a_foreign_or_stale_emission_handle() {
    for mutation in ["foreign stream", "earlier instruction", "moved instruction"] {
        let mut join = fixture(PythonVersion::Py314);
        match mutation {
            "foreign stream" => join.source = fixture(PythonVersion::Py314).source,
            "earlier instruction" => {
                join.source = EmittedJump(EmittedInstruction {
                    stream: join.source.instruction().stream,
                    item_index: u32::try_from(join.guard).unwrap(),
                });
            }
            "moved instruction" => {
                join.assembler
                    .emit_with_depth(NOP, 0, 2, ExceptionPolicy::default());
            }
            _ => unreachable!(),
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| accepts(&mut join))).is_err(),
            "{mutation}"
        );
    }
}
