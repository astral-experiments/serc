use super::ir::{
    BorrowFallthrough, ExceptionPolicy, FusionPolicy, JumpPolicy, LocalPolicy, NoLocationPolicy,
    RetainedJumpNop, UnoptimizedCode,
};
use super::{
    Assembler, AssemblerStage, BooleanValueMarkerScan, EmittedJump, ExceptionExclusion,
    Instruction, Item, NormalizedExceptionOwner, Operand, OperandPatchId, SourceLocation,
};
use crate::target::opcodes::*;
use crate::target::operands::{CommonConstant, ResumeLocation};

#[test]
#[cfg(target_pointer_width = "64")]
fn instruction_items_remain_cache_compact() {
    // Five copied-slot counts plus the original-entry state give six metadata states.
    // 23 policies * 6 states * 2 copy modes require 276 nonzero values, so metadata
    // uses NonZeroU16. The intentional 32-to-36-byte increase costs 12.5% per item.
    // Keep the Item and Option layouts compact despite the wider metadata.
    assert_eq!(std::mem::size_of::<Instruction>(), 36);
    assert_eq!(std::mem::size_of::<Item>(), 36);
    assert_eq!(
        std::mem::size_of::<Option<Instruction>>(),
        std::mem::size_of::<Instruction>(),
    );
}

#[test]
fn compact_instruction_metadata_keeps_block_size_and_exception_policy_independent() {
    let policies = [
        ExceptionPolicy::NoneUnchanged,
        ExceptionPolicy::NonePresent,
        ExceptionPolicy::NoneOverridePresent,
        ExceptionPolicy::NoneAbsent,
        ExceptionPolicy::IfExtendedUnchanged,
        ExceptionPolicy::IfExtendedPresent,
        ExceptionPolicy::IfExtendedOverridePresent,
        ExceptionPolicy::IfExtendedAbsent,
        ExceptionPolicy::AlwaysUnchanged,
        ExceptionPolicy::AlwaysPresent,
        ExceptionPolicy::AlwaysOverridePresent,
        ExceptionPolicy::AlwaysAbsent,
        ExceptionPolicy::NoneDynamic,
        ExceptionPolicy::IfExtendedDynamic,
        ExceptionPolicy::AlwaysDynamic,
        ExceptionPolicy::NoneDynamicCondition,
        ExceptionPolicy::IfExtendedDynamicCondition,
        ExceptionPolicy::AlwaysDynamicCondition,
        ExceptionPolicy::NoneDynamicConditionStrict,
        ExceptionPolicy::IfExtendedDynamicConditionStrict,
        ExceptionPolicy::AlwaysDynamicConditionStrict,
    ];
    for unconditional_copy in [false, true] {
        for policy in policies {
            for count in 0..=5 {
                let mut instruction =
                    Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None);
                if unconditional_copy {
                    instruction.allow_unconditional_block_inlining();
                }
                instruction.set_exception_policy(policy);
                instruction.add_pre_fold_extra_instructions(count);
                assert_eq!(instruction.exception_policy(), policy);
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(instruction.local_policy.hidden_exception_owner_slots(), 0);

                instruction.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    policy.exclusion()
                );
                assert_eq!(
                    instruction.exception_policy().owner(),
                    NormalizedExceptionOwner::OverridePresent
                );
                instruction.exclude_from_exception_table(ExceptionExclusion::Always);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    ExceptionExclusion::Always
                );
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(
                    instruction.only_allows_unconditional_inlining(),
                    unconditional_copy
                );
            }
        }
    }
}

#[test]
fn compact_initial_borrow_metadata_keeps_all_logical_fields_independent() {
    let policies = [
        ExceptionPolicy::NoneUnchanged,
        ExceptionPolicy::NonePresent,
        ExceptionPolicy::NoneOverridePresent,
        ExceptionPolicy::NoneAbsent,
        ExceptionPolicy::IfExtendedUnchanged,
        ExceptionPolicy::IfExtendedPresent,
        ExceptionPolicy::IfExtendedOverridePresent,
        ExceptionPolicy::IfExtendedAbsent,
        ExceptionPolicy::AlwaysUnchanged,
        ExceptionPolicy::AlwaysPresent,
        ExceptionPolicy::AlwaysOverridePresent,
        ExceptionPolicy::AlwaysAbsent,
        ExceptionPolicy::NoneDynamic,
        ExceptionPolicy::IfExtendedDynamic,
        ExceptionPolicy::AlwaysDynamic,
        ExceptionPolicy::NoneDynamicCondition,
        ExceptionPolicy::IfExtendedDynamicCondition,
        ExceptionPolicy::AlwaysDynamicCondition,
        ExceptionPolicy::NoneDynamicConditionStrict,
        ExceptionPolicy::IfExtendedDynamicConditionStrict,
        ExceptionPolicy::AlwaysDynamicConditionStrict,
    ];
    for unconditional_copy in [false, true] {
        for policy in policies {
            for count in 0..=5 {
                let mut instruction = Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(super::Label(u32::MAX)),
                    SourceLocation::NONE,
                    None,
                );
                if unconditional_copy {
                    instruction.allow_unconditional_block_inlining();
                }
                instruction.set_exception_policy(policy);
                instruction.add_pre_fold_extra_instructions(count);
                instruction.mark_initial_borrow_jump_target(super::Label(u32::MAX));
                assert_eq!(
                    instruction.initial_borrow_jump_target(),
                    (count == 0).then_some(super::Label(u32::MAX))
                );
                assert_eq!(instruction.exception_policy(), policy);
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(instruction.local_policy.hidden_exception_owner_slots(), 0);

                instruction.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    policy.exclusion()
                );
                assert_eq!(
                    instruction.exception_policy().owner(),
                    NormalizedExceptionOwner::OverridePresent
                );
                instruction.exclude_from_exception_table(ExceptionExclusion::Always);
                assert_eq!(
                    instruction.exception_policy().exclusion(),
                    ExceptionExclusion::Always
                );
                assert_eq!(instruction.pre_fold_extra_instructions(), count.min(4));
                assert_eq!(
                    instruction.only_allows_unconditional_inlining(),
                    unconditional_copy
                );
                assert_eq!(
                    instruction.initial_borrow_jump_target(),
                    (count == 0).then_some(super::Label(u32::MAX))
                );
                instruction.add_pre_fold_extra_instructions(1);
                assert_eq!(instruction.initial_borrow_jump_target(), None);
                instruction.add_pre_fold_extra_instructions(0);
                assert_eq!(instruction.initial_borrow_jump_target(), None);
            }
        }
    }
}

#[test]
fn compact_operands_preserve_full_width_payloads() {
    let operands = [
        Operand::Value(u32::MAX),
        Operand::Deferred(OperandPatchId(u32::MAX)),
        Operand::Forward(super::Label(u32::MAX)),
        Operand::Backward(super::Label(u32::MAX)),
    ];
    let mut instruction = Instruction::new(NOP, operands[0], SourceLocation::NONE, None);
    for operand in operands {
        instruction.set_operand(operand);
        assert_eq!(instruction.operand(), operand);
    }

    let mut first = Instruction::new(NOP, operands[2], SourceLocation::NONE, None);
    first.fusion_policy.prevent_with_previous();
    let mut second = Instruction::new(NOP, operands[3], SourceLocation::NONE, None);
    second.fusion_policy.prevent_with_next();
    let fused = Instruction::fused(NOP, operands[1], first, second);
    assert_eq!(fused.operand(), operands[1]);
    assert!(fused.fusion_policy.prevents_with_previous());
    assert!(fused.fusion_policy.prevents_with_next());
}

#[test]
fn instruction_constructors_set_audited_defaults() {
    let regular = Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None);
    assert_eq!(regular.local_policy, LocalPolicy::default());
    assert!(!regular.local_policy.may_store_null());
    assert_eq!(regular.no_location_policy, NoLocationPolicy::default());
    assert_eq!(regular.fusion_policy, FusionPolicy::default());
    assert!(!regular.fusion_policy.location_from_next());
    assert!(!regular.fusion_policy.is_converted_pop_block());
    assert_eq!(regular.local_policy.hidden_exception_owner_slots(), 0);
    assert_eq!(regular.jump_policy, JumpPolicy::default());
    assert!(!regular.fusion_policy.is_authored_break());
    assert!(!regular.fusion_policy.transitive_small_exit_inlining());

    let synthetic = Instruction::synthetic(NOP, Operand::Value(0), SourceLocation::NONE, None);
    assert!(!synthetic.jump_policy.inline_small_exit());
    let mut expected_synthetic_policy = JumpPolicy::default();
    expected_synthetic_policy.prevent_small_exit_inlining();
    assert_eq!(synthetic.jump_policy, expected_synthetic_policy);
    assert!(!synthetic.fusion_policy.is_authored_break());
    assert_eq!(synthetic.exception_policy(), ExceptionPolicy::default());
    assert_eq!(
        regular.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
    assert_eq!(
        synthetic.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
}

#[test]
fn direct_value_instruction_constructor_matches_regular_instruction() {
    let location = SourceLocation::new(3, 4, 2, 8);
    let regular = Instruction::new(LOAD_CONST, Operand::Value(u32::MAX), location, Some(7));
    let direct = Instruction::value_with_depth(LOAD_CONST, u32::MAX, location, 7);

    assert_eq!(direct.opcode, regular.opcode);
    assert_eq!(direct.operand(), regular.operand());
    assert_eq!(direct.location, regular.location);
    assert_eq!(direct.depth_after(), regular.depth_after());
    assert_eq!(direct.local_policy, regular.local_policy);
    assert_eq!(direct.no_location_policy, regular.no_location_policy);
    assert_eq!(direct.fusion_policy, regular.fusion_policy);
    assert_eq!(direct.jump_policy, regular.jump_policy);
    assert_eq!(direct.exception_policy(), regular.exception_policy());
}

#[test]
fn packed_exception_policy_preserves_all_states() {
    let exclusions = [
        ExceptionExclusion::None,
        ExceptionExclusion::IfExtended,
        ExceptionExclusion::Always,
    ];
    let owners = [
        NormalizedExceptionOwner::Unchanged,
        NormalizedExceptionOwner::Present,
        NormalizedExceptionOwner::OverridePresent,
        NormalizedExceptionOwner::Absent,
    ];
    for exclusion in exclusions {
        for owner in owners {
            let mut policy = ExceptionPolicy::default();
            policy.exclude(exclusion);
            policy.set_owner(owner);
            assert_eq!(policy.exclusion(), exclusion);
            assert_eq!(policy.owner(), owner);
        }
    }
    assert_eq!(
        ExceptionPolicy::EXCLUDE_IF_EXTENDED.exclusion(),
        ExceptionExclusion::IfExtended
    );
    assert_eq!(
        ExceptionPolicy::EXCLUDE_ALWAYS.exclusion(),
        ExceptionExclusion::Always
    );
    assert_eq!(
        ExceptionPolicy::normalized_owner(true).owner(),
        NormalizedExceptionOwner::Present
    );
    assert_eq!(
        ExceptionPolicy::normalized_owner(false).owner(),
        NormalizedExceptionOwner::Absent
    );
}

#[test]
fn jump_policy_defaults_are_explicit() {
    let policy = JumpPolicy::default();
    assert!(policy.inline_small_exit());
    assert_eq!(policy.retained_nop(), RetainedJumpNop::None);
    assert!(policy.allows_threading_target());
    assert!(policy.allows_borrow_threading_target());
    assert!(!policy.defers_redundant_removal());
    assert!(!policy.preserves_nop_after_threading());
}

#[test]
fn jump_policy_fusion_is_exhaustive() {
    fn policy(bits: u8, retained_nop: RetainedJumpNop) -> JumpPolicy {
        let mut policy = JumpPolicy::default();
        if bits & 1 == 0 {
            policy.prevent_small_exit_inlining();
        }
        match retained_nop {
            RetainedJumpNop::None => {}
            RetainedJumpNop::DirectTarget => policy.retain_nop_for_direct_target(),
            RetainedJumpNop::Always => policy.retain_nop(),
        }
        if bits & 2 == 0 {
            policy.prevent_threading_target();
        }
        if bits & 4 != 0 {
            policy.defer_removal();
        }
        if bits & 8 != 0 {
            policy.preserve_nop_after_threading_for_test();
        }
        if bits & 16 == 0 {
            policy.prevent_borrow_threading_target();
        }
        policy
    }

    let retained_nops = [
        RetainedJumpNop::None,
        RetainedJumpNop::DirectTarget,
        RetainedJumpNop::Always,
    ];
    for first_bits in 0..32 {
        for second_bits in 0..32 {
            for first_retained in retained_nops {
                for second_retained in retained_nops {
                    let first = policy(first_bits, first_retained);
                    let second = policy(second_bits, second_retained);
                    let expected_bits =
                        u8::from(first.inline_small_exit() && second.inline_small_exit())
                            | (u8::from(
                                first.allows_threading_target() && second.allows_threading_target(),
                            ) << 1)
                            | (u8::from(
                                first.defers_redundant_removal()
                                    || second.defers_redundant_removal(),
                            ) << 2)
                            | (u8::from(
                                first.preserves_nop_after_threading()
                                    || second.preserves_nop_after_threading(),
                            ) << 3)
                            | (u8::from(
                                first.allows_borrow_threading_target()
                                    && second.allows_borrow_threading_target(),
                            ) << 4);
                    assert_eq!(
                        JumpPolicy::fused(first, second),
                        policy(expected_bits, RetainedJumpNop::None)
                    );
                }
            }
        }
    }
}

#[test]
fn retained_jump_nop_distinguishes_direct_target_provenance() {
    let expected = super::Label(1);
    let mut instruction = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(expected),
        SourceLocation::NONE,
        None,
    );
    instruction.jump_policy.retain_nop_for_direct_target();
    assert!(
        instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
    );

    instruction.retarget(Operand::Forward(expected));
    assert_eq!(
        instruction.jump_policy.retained_nop(),
        RetainedJumpNop::DirectTarget
    );
    instruction.retarget(Operand::Forward(super::Label(2)));
    assert_eq!(
        instruction.jump_policy.retained_nop(),
        RetainedJumpNop::None
    );
    instruction.retarget(Operand::Forward(expected));
    assert!(
        !instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
    );

    instruction.jump_policy.retain_nop();
    instruction.retarget(Operand::Backward(expected));
    assert!(instruction.jump_policy.always_retains_nop());
    assert!(
        instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
    );
}

#[test]
fn deferred_jump_removal_promotes_post_threading_nop_retention() {
    let mut policy = JumpPolicy::default();
    policy.retain_redundant_nop_after_threading();
    assert!(policy.defers_redundant_removal());
    assert!(policy.preserves_nop_after_threading());

    policy.resolve_deferred_removal();
    assert!(!policy.defers_redundant_removal());
    assert!(!policy.preserves_nop_after_threading());
    assert_eq!(policy.retained_nop(), RetainedJumpNop::Always);
}

#[test]
#[should_panic(expected = "stale emitted-instruction handle")]
fn emitted_instruction_handles_reject_stale_instructions() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    let jump =
        assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.emit(NOP, 0);
    assembler.defer_jump_removal(jump);
}

#[test]
fn emitted_instruction_handles_reject_reused_item_indices() {
    let mut assembler = Assembler::default();
    let removed = assembler.emit(NOP, 0);
    assert!(assembler.take_trailing_nop_location().is_some());
    assembler.emit(LOAD_CONST, 0);

    let mutation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assembler.prevent_instruction_fusion(removed);
    }));

    assert!(mutation.is_err());
    let Some(Item::Instruction(replacement)) = assembler.linear_items().last() else {
        panic!("replacement instruction was not emitted");
    };
    assert!(!replacement.fusion_policy.prevents_with_next());
}

#[test]
#[should_panic(
    expected = "preventing fusion with the next instruction requires a preceding instruction"
)]
fn instruction_policy_mutation_requires_a_preceding_instruction() {
    Assembler::default().prevent_last_instruction_fusion();
}

#[test]
#[should_panic(expected = "handle from a different assembler")]
fn emitted_instruction_handles_reject_a_different_assembler() {
    let mut first = Assembler::default();
    let emitted = first.emit(NOP, 0);

    let mut second = Assembler::default();
    second.emit(NOP, 0);
    second.prevent_instruction_fusion(emitted);
}

#[test]
#[should_panic(expected = "preventing jump inlining requires an emitted jump")]
fn typed_jump_policy_mutation_rejects_a_non_jump_handle() {
    let mut assembler = Assembler::default();
    let instruction = assembler.emit(NOP, 0);
    assembler.prevent_jump_inlining(EmittedJump(instruction));
}

#[test]
#[should_panic(expected = "retaining a directly inlined jump NOP requires a forward jump")]
fn direct_jump_nop_provenance_requires_a_forward_jump() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    let jump = assembler.emit_backward(JUMP_BACKWARD, target);
    assembler.preserve_direct_inlined_jump_nop(jump);
}

#[test]
fn borrow_fallthrough_barrier_stays_on_predecessor_across_labels_and_emission() {
    let mut assembler = Assembler::default();
    let predecessor = assembler.emit(LOAD_CONST, 0);
    let boundary = assembler.label();
    assembler.mark(boundary);
    assembler.mark_borrow_fallthrough_barrier(predecessor);
    assembler.emit(LOAD_CONST, 1);

    let [
        Item::Instruction(predecessor),
        Item::Label(actual_boundary),
        Item::Instruction(next),
    ] = assembler.linear_items()
    else {
        panic!(
            "unexpected instruction stream: {:?}",
            assembler.linear_items()
        );
    };
    assert_eq!(*actual_boundary, boundary);
    assert_eq!(
        predecessor.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(
        next.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
}

#[test]
fn jump_policy_handles_remain_valid_across_trailing_labels() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    let jump =
        assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.mark(target);

    assembler.prevent_jump_inlining(jump);

    let Some(Item::Instruction(instruction)) = assembler.linear_items().first() else {
        panic!("jump was not emitted");
    };
    assert!(!instruction.jump_policy.inline_small_exit());
}

#[test]
fn value_emission_attaches_exception_policy_across_intervening_labels() {
    let mut assembler = Assembler::default();
    let normalized_owner = ExceptionPolicy::normalized_owner(false);
    assembler.emit_with_depth(NOT_TAKEN, 0, 0, normalized_owner);
    let first_boundary = assembler.label();
    assembler.mark(first_boundary);
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
    let second_boundary = assembler.label();
    assembler.mark(second_boundary);
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());

    let [
        Item::Instruction(normalized),
        Item::Label(actual_first_boundary),
        Item::Instruction(excluded),
        Item::Label(actual_second_boundary),
        Item::Instruction(default),
    ] = assembler.linear_items()
    else {
        panic!("expected three instructions separated by labels");
    };
    assert_eq!(*actual_first_boundary, first_boundary);
    assert_eq!(*actual_second_boundary, second_boundary);
    assert_eq!(normalized.exception_policy(), normalized_owner);
    assert_eq!(excluded.exception_policy(), ExceptionPolicy::EXCLUDE_ALWAYS);
    assert_eq!(default.exception_policy(), ExceptionPolicy::default());
}

#[test]
fn jump_emission_attaches_exception_policy_after_an_intervening_label() {
    let mut assembler = Assembler::default();
    assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::default());
    let boundary = assembler.label();
    assembler.mark(boundary);
    let target = assembler.label();
    assembler.emit_forward_with_depth(
        JUMP_FORWARD,
        target,
        0,
        ExceptionPolicy::EXCLUDE_IF_EXTENDED,
    );

    let [
        Item::Instruction(default),
        Item::Label(actual_boundary),
        Item::Instruction(excluded),
    ] = assembler.linear_items()
    else {
        panic!("expected two instructions separated by one label");
    };
    assert_eq!(*actual_boundary, boundary);
    assert_eq!(default.exception_policy(), ExceptionPolicy::default());
    assert_eq!(
        excluded.exception_policy(),
        ExceptionPolicy::EXCLUDE_IF_EXTENDED
    );
}

#[test]
fn exception_exclusion_names_layout_policy() {
    assert!(!ExceptionExclusion::None.applies(1));
    assert!(!ExceptionExclusion::IfExtended.applies(0));
    assert!(ExceptionExclusion::IfExtended.applies(1));
    assert!(ExceptionExclusion::Always.applies(0));
}

#[test]
fn terminal_with_barrier_scan_ignores_prefix_and_labels() {
    let mut assembler = Assembler::default();
    assembler.emit(NOT_TAKEN, 0);
    let prefix_boundary = assembler.label();
    assembler.mark(prefix_boundary);
    let body_start = assembler.item_count();
    let body_boundary = assembler.label();
    assembler.mark(body_boundary);
    assert!(!assembler.contains_opcode_since_item(LOAD_FAST_AND_CLEAR, body_start));
    assert!(!assembler.contains_terminal_with_barrier_since_item(body_start));
    assembler.emit(NOP, 0);
    assert!(!assembler.contains_terminal_with_barrier_since_item(body_start));
    assert!(matches!(
        assembler.pop_linear_item(),
        Some(Item::Instruction(instruction)) if instruction.opcode == NOP
    ));
    assert!(!assembler.contains_terminal_with_barrier_since_item(body_start));

    for opcode in [LOAD_FAST_AND_CLEAR, NOT_TAKEN, FOR_ITER, PUSH_EXC_INFO] {
        let mut assembler = Assembler::default();
        assembler.emit(NOT_TAKEN, 0);
        let prefix_boundary = assembler.label();
        assembler.mark(prefix_boundary);
        let body_start = assembler.item_count();
        let body_boundary = assembler.label();
        assembler.mark(body_boundary);
        assembler.push_linear_item(Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )));
        assert!(assembler.contains_terminal_with_barrier_since_item(body_start));
        assert!(assembler.contains_opcode_since_item(opcode, body_start));
        assert!(!assembler.contains_terminal_with_barrier_since_item(assembler.item_count()));
    }
}

#[test]
fn boolean_value_marker_scan_ignores_prefix_and_stops_at_barriers() {
    let mut assembler = Assembler::default();
    assembler.emit(NOT_TAKEN, 0);
    assembler.emit(POP_TOP, 0);
    let mut scan = BooleanValueMarkerScan::new(assembler.item_count());
    assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));

    let boundary = assembler.label();
    assembler.emit(NOT_TAKEN, 0);
    assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));
    assembler.mark(boundary);
    assembler.emit(POP_TOP, 0);
    assert!(assembler.advance_boolean_value_marker_scan(&mut scan));

    for opcode in [LOAD_FAST_AND_CLEAR, FOR_ITER, PUSH_EXC_INFO] {
        let mut assembler = Assembler::default();
        let mut scan = BooleanValueMarkerScan::new(assembler.item_count());
        assembler.emit(NOT_TAKEN, 0);
        assembler.emit(POP_TOP, 0);
        assert!(assembler.advance_boolean_value_marker_scan(&mut scan));
        assembler.emit(opcode, 0);
        assembler.emit(NOT_TAKEN, 0);
        assembler.emit(POP_TOP, 0);
        assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));
        assembler.emit(NOT_TAKEN, 0);
        assembler.emit(POP_TOP, 0);
        assert!(!assembler.advance_boolean_value_marker_scan(&mut scan));
    }
}

#[test]
fn iterator_tail_barrier_scan_ignores_prefix_and_labels() {
    let mut assembler = Assembler::default();
    assembler.emit(NOT_TAKEN, 0);
    let prefix_boundary = assembler.label();
    assembler.mark(prefix_boundary);
    let body_start = assembler.item_count();
    let body_boundary = assembler.label();
    assembler.mark(body_boundary);
    assembler.emit(NOP, 0);
    assert!(!assembler.contains_iterator_tail_barrier_since_item(body_start));

    for opcode in [NOT_TAKEN, SEND, FOR_ITER, PUSH_EXC_INFO] {
        let mut assembler = Assembler::default();
        assembler.emit(NOT_TAKEN, 0);
        let prefix_boundary = assembler.label();
        assembler.mark(prefix_boundary);
        let body_start = assembler.item_count();
        let body_boundary = assembler.label();
        assembler.mark(body_boundary);
        assembler.push_linear_item(Item::Instruction(Instruction::new(
            opcode,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )));
        assert!(assembler.contains_iterator_tail_barrier_since_item(body_start));
        assert!(!assembler.contains_iterator_tail_barrier_since_item(assembler.item_count()));
    }
}

#[test]
fn instruction_offsets_follow_label_insertions_and_trailing_removal() {
    let mut assembler = Assembler::default();
    assembler.emit(LOAD_CONST, 0);
    assembler.emit(NOP, 0);

    let trailing = assembler.label();
    assembler.mark(trailing);
    let before_nop = assembler.label();
    assembler.mark_before_trailing_instructions(before_nop, 1);
    let before_labels = assembler.label();
    assembler.mark_before_trailing_labels(before_labels);

    assert_eq!(assembler.instruction_count(), 2);
    assert_eq!(assembler.instruction_opcode_at(0), Some(LOAD_CONST));
    assert_eq!(assembler.instruction_opcode_at(1), Some(NOP));
    assert_eq!(assembler.instruction_opcode_at(2), None);

    assert!(assembler.take_trailing_nop_location().is_some());
    assert_eq!(assembler.instruction_count(), 1);
    assert_eq!(assembler.instruction_opcode_at(0), Some(LOAD_CONST));
    assert_eq!(assembler.instruction_opcode_at(1), None);

    assembler.emit(RETURN_VALUE, 0);
    assert_eq!(assembler.instruction_count(), 2);
    assert_eq!(assembler.instruction_opcode_at(1), Some(RETURN_VALUE));
}

#[test]
fn dynamic_exception_owner_tracks_fusible_loads_and_null_globals() {
    let location = SourceLocation::new(1, 1, 0, 1);

    let mut fusible = Assembler::default();
    fusible.set_location(location);
    fusible.emit(LOAD_FAST, 1);
    fusible.emit(LOAD_FAST, 2);
    assert_eq!(fusible.current_block_dynamic_exception_owner_slots(), 1);
    assert!(fusible.current_block_has_dynamic_exception_owner());

    let mut mixed = Assembler::default();
    mixed.set_location(location);
    mixed.emit(LOAD_FAST, 1);
    mixed.emit(LOAD_CONST, 0);
    assert_eq!(mixed.current_block_dynamic_exception_owner_slots(), 0);
    assert!(!mixed.current_block_has_dynamic_exception_owner());

    let mut blocked = Assembler::default();
    blocked.set_location(location);
    blocked.emit(LOAD_FAST, 1);
    blocked.prevent_last_instruction_fusion();
    blocked.emit(LOAD_FAST, 2);
    assert_eq!(blocked.current_block_dynamic_exception_owner_slots(), 0);
    assert!(!blocked.current_block_has_dynamic_exception_owner());

    let mut null_global = Assembler::default();
    null_global.emit(LOAD_GLOBAL, 1);
    assert_eq!(null_global.current_block_dynamic_exception_owner_slots(), 1);
    assert!(null_global.current_block_has_dynamic_exception_owner());

    let mut regular_global = Assembler::default();
    regular_global.emit(LOAD_GLOBAL, 0);
    assert_eq!(
        regular_global.current_block_dynamic_exception_owner_slots(),
        0
    );
    assert!(!regular_global.current_block_has_dynamic_exception_owner());

    let mut three_slots = Assembler::default();
    three_slots.set_location(location);
    three_slots.emit(STORE_FAST, 1);
    three_slots.emit(STORE_FAST, 2);
    three_slots.emit(LOAD_GLOBAL, 1);
    three_slots.emit(LOAD_GLOBAL, 3);
    assert_eq!(three_slots.current_block_dynamic_exception_owner_slots(), 3);

    let mut redundant_store = Assembler::default();
    redundant_store.set_location(location);
    redundant_store.emit(STORE_FAST, 1);
    redundant_store.emit(STORE_FAST, 1);
    assert_eq!(
        redundant_store.current_block_dynamic_exception_owner_slots(),
        0
    );

    let mut converted_pop_block = Assembler::default();
    converted_pop_block.emit(NOP, 0);
    converted_pop_block.mark_last_as_converted_pop_block();
    assert_eq!(
        converted_pop_block.current_block_dynamic_exception_owner_slots(),
        1
    );

    let mut hidden_slots = Assembler::default();
    hidden_slots.emit(LOAD_CONST, 0);
    hidden_slots.add_last_hidden_exception_owner_slots(2);
    assert_eq!(
        hidden_slots.current_block_dynamic_exception_owner_slots(),
        2
    );
    let boundary = hidden_slots.label();
    hidden_slots.mark(boundary);
    assert_eq!(
        hidden_slots.current_block_dynamic_exception_owner_slots(),
        0
    );

    let mut removable_nop = Assembler::default();
    removable_nop.set_location(SourceLocation::new(1, 1, 0, 1));
    removable_nop.emit(NOP, 0);
    removable_nop.emit(LOAD_CONST, 0);
    assert_eq!(
        removable_nop.current_block_dynamic_exception_owner_slots(),
        1
    );

    let mut retained_nop = Assembler::default();
    retained_nop.set_location(SourceLocation::new(1, 1, 0, 1));
    retained_nop.emit(NOP, 0);
    retained_nop.set_location(SourceLocation::new(2, 2, 0, 1));
    retained_nop.emit(LOAD_CONST, 0);
    assert_eq!(
        retained_nop.current_block_dynamic_exception_owner_slots(),
        0
    );
}

#[test]
fn dynamic_exception_owner_counts_only_removable_comprehension_restore_swaps() {
    for (consumer, expected_slots) in [(STORE_FAST, 1), (STORE_DEREF, 0), (CALL, 0)] {
        for restore_count in 1..=3 {
            let mut assembler = Assembler::default();
            assembler.set_location(SourceLocation::new(1, 1, 0, 1));
            assembler.emit(SWAP, restore_count + 1);
            for index in 1..=restore_count {
                if index > 1 {
                    assembler.fusion_barrier();
                }
                let restore = assembler.emit(STORE_FAST, index);
                assembler.prevent_instruction_fusion_with_previous(restore);
                assembler.prevent_last_instruction_fusion();
            }
            assembler.emit(consumer, restore_count + 1);
            assert_eq!(
                assembler.current_block_dynamic_exception_owner_slots(),
                expected_slots,
            );
        }
    }
}

#[test]
fn dynamic_exception_owner_stops_at_emitted_boundaries() {
    let mut assembler = Assembler::default();
    assembler.set_location(SourceLocation::new(1, 1, 0, 1));
    assembler.emit(LOAD_FAST, 1);
    assembler.emit(LOAD_FAST, 2);
    assert!(assembler.current_block_has_dynamic_exception_owner());

    let boundary = assembler.label();
    assembler.mark(boundary);
    assert!(!assembler.current_block_has_dynamic_exception_owner());

    assembler.emit(LOAD_GLOBAL, 1);
    assert!(assembler.current_block_has_dynamic_exception_owner());
    assembler.emit(NOT_TAKEN, 0);
    assert!(!assembler.current_block_has_dynamic_exception_owner());
}

#[test]
fn fallthrough_dynamic_exception_owner_crosses_exception_region_labels() {
    let mut hidden_slot = Assembler::default();
    hidden_slot.emit(LOAD_CONST, 0);
    hidden_slot.add_last_hidden_exception_owner_slots(1);
    let boundary = hidden_slot.label();
    hidden_slot.mark(boundary);
    assert!(!hidden_slot.current_block_has_dynamic_exception_owner());
    assert!(hidden_slot.fallthrough_has_dynamic_exception_owner());
    assert!(hidden_slot.fallthrough_has_concrete_dynamic_exception_owner());

    let mut consumed_by_prologue = Assembler::default();
    consumed_by_prologue.emit(LOAD_CONST, 0);
    consumed_by_prologue.add_last_hidden_exception_owner_slots(1);
    let boundary = consumed_by_prologue.label();
    consumed_by_prologue.mark(boundary);
    consumed_by_prologue.emit(COPY_FREE_VARS, 1);
    assert!(!consumed_by_prologue.fallthrough_has_dynamic_exception_owner());

    let mut hard_boundary = Assembler::default();
    hard_boundary.emit(LOAD_CONST, 0);
    hard_boundary.add_last_hidden_exception_owner_slots(1);
    let boundary = hard_boundary.label();
    hard_boundary.emit_forward_with_depth(
        POP_JUMP_IF_FALSE,
        boundary,
        0,
        ExceptionPolicy::default(),
    );
    hard_boundary.mark(boundary);
    assert!(!hard_boundary.fallthrough_has_dynamic_exception_owner());
}

#[test]
fn fallthrough_dynamic_exception_owner_preserves_conservative_vacancies() {
    let location = SourceLocation::new(1, 1, 0, 1);

    let mut protected_nops = Assembler::default();
    protected_nops.set_location(location);
    protected_nops.emit(NOP, 0);
    let boundary = protected_nops.label();
    protected_nops.mark(boundary);
    protected_nops.emit(NOP, 0);
    assert!(protected_nops.fallthrough_has_dynamic_exception_owner());
    assert!(protected_nops.fallthrough_has_concrete_dynamic_exception_owner());

    let mut conservative_only = Assembler::default();
    conservative_only.set_location(SourceLocation::new(1, 1, 0, 1));
    conservative_only.emit(NOP, 0);
    let boundary = conservative_only.label();
    conservative_only.mark(boundary);
    conservative_only.set_location(SourceLocation::new(2, 2, 0, 1));
    conservative_only.emit(NOP, 0);
    assert!(conservative_only.fallthrough_has_dynamic_exception_owner());
    assert!(!conservative_only.fallthrough_has_concrete_dynamic_exception_owner());
}

#[test]
fn current_block_resume_after_yield_stops_at_emitted_boundaries() {
    let mut assembler = Assembler::default();
    assembler.emit(RESUME, ResumeLocation::AFTER_YIELD.argument());
    assert!(assembler.current_block_resumes_after_yield());

    let boundary = assembler.label();
    assembler.mark(boundary);
    assert!(!assembler.current_block_resumes_after_yield());

    assembler.emit(RESUME, ResumeLocation::AFTER_YIELD_FROM.argument());
    assert!(!assembler.current_block_resumes_after_yield());

    assembler.emit(RESUME, ResumeLocation::AFTER_AWAIT.argument());
    assert!(!assembler.current_block_resumes_after_yield());

    assembler.emit(RESUME, ResumeLocation::AFTER_YIELD.argument());
    assert!(assembler.current_block_resumes_after_yield());
    assembler.emit(NOT_TAKEN, 0);
    assert!(!assembler.current_block_resumes_after_yield());
}

#[test]
fn fusion_preserves_each_typed_instruction_policy() {
    let mut first = Instruction::new(
        LOAD_FAST,
        Operand::Value(1),
        SourceLocation::new(2, 2, 0, 1),
        Some(1),
    );
    first.local_policy = LocalPolicy::for_load(true, false);
    first.local_policy.mark_maybe_null_comprehension_restore();
    first.no_location_policy.allow_block_inlining();
    first.no_location_policy.make_first_copy_interruptible();
    first.no_location_policy.mark_artificial_eval_break();
    first
        .no_location_policy
        .make_copy_inherit_shared_predecessor();
    first.no_location_policy.percent_target_location = 7;
    first.fusion_policy.mark_converted_pop_block();
    first.local_policy.add_hidden_exception_owner_slots(2);
    first.add_pre_fold_extra_instructions(2);
    first.fusion_policy.prevent_with_next();
    first.jump_policy.prevent_threading_target();
    first.jump_policy.defer_removal();
    first.jump_policy.retain_nop();
    first.jump_policy.mark_borrow_empty_block();
    first.jump_policy.preserve_explicit_nop();
    first.fusion_policy.mark_authored_break();
    first.fusion_policy.allow_transitive_small_exit_inlining();
    first.exclude_from_exception_table(ExceptionExclusion::IfExtended);
    first.set_exception_owner(NormalizedExceptionOwner::Absent);

    let mut second = Instruction::new(
        LOAD_FAST,
        Operand::Value(2),
        SourceLocation::new(2, 2, 2, 3),
        Some(2),
    );
    second.local_policy = LocalPolicy::for_load(false, true);
    second.no_location_policy.preserve_location();
    second.fusion_policy.prevent_with_previous();
    second.jump_policy.prevent_small_exit_inlining();
    second.jump_policy.retain_redundant_nop_after_threading();
    second.jump_policy.retain_nop_for_direct_target();
    second.exclude_from_exception_table(ExceptionExclusion::Always);
    second.set_exception_owner(NormalizedExceptionOwner::Present);
    second.local_policy.add_hidden_exception_owner_slots(2);
    second.add_pre_fold_extra_instructions(3);
    second
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);

    let fused = Instruction::fused(LOAD_FAST_LOAD_FAST, Operand::Value(0x12), first, second);

    let mut expected_local_policy = LocalPolicy::for_load(true, true);
    expected_local_policy.mark_maybe_null_comprehension_restore();
    expected_local_policy.add_hidden_exception_owner_slots(4);
    assert_eq!(fused.local_policy, expected_local_policy);
    assert!(fused.no_location_policy.allows_block_inlining());
    assert!(
        fused
            .no_location_policy
            .first_copy_is_interruptible_backward()
    );
    assert!(fused.no_location_policy.is_artificial_eval_break());
    assert!(fused.no_location_policy.copy_inherits_shared_predecessor());
    assert!(fused.no_location_policy.preserves_location());
    assert_eq!(fused.no_location_policy.percent_target_location, 7);
    assert!(!fused.fusion_policy.prevents_with_previous());
    assert!(!fused.fusion_policy.prevents_with_next());
    assert!(fused.fusion_policy.is_converted_pop_block());
    assert!(fused.jump_policy.preserves_explicit_nop());
    assert!(fused.fusion_policy.is_authored_break());
    assert!(fused.fusion_policy.transitive_small_exit_inlining());
    assert_eq!(fused.local_policy.hidden_exception_owner_slots(), 4);
    assert_eq!(fused.pre_fold_extra_instructions(), 4);
    assert_eq!(fused.location, first.location);
    assert_eq!(fused.depth_after(), second.depth_after());
    let mut expected_jump_policy = JumpPolicy::default();
    expected_jump_policy.prevent_small_exit_inlining();
    expected_jump_policy.prevent_threading_target();
    expected_jump_policy.retain_redundant_nop_after_threading();
    expected_jump_policy.mark_borrow_empty_block();
    expected_jump_policy.preserve_explicit_nop();
    assert_eq!(fused.jump_policy, expected_jump_policy);
    assert_eq!(
        fused.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(
        fused.exception_policy().exclusion(),
        ExceptionExclusion::Always
    );
    assert_eq!(
        fused.exception_policy().owner(),
        NormalizedExceptionOwner::Absent
    );

    let mut next_first = Instruction::new(
        LOAD_FAST,
        Operand::Value(1),
        SourceLocation::new(3, 3, 0, 1),
        None,
    );
    next_first.fusion_policy.mark_location_from_next();
    let next_second = Instruction::new(
        LOAD_FAST,
        Operand::Value(2),
        SourceLocation::new(4, 4, 0, 1),
        None,
    );
    let fused_next = Instruction::fused(
        LOAD_FAST_LOAD_FAST,
        Operand::Value(0x12),
        next_first,
        next_second,
    );
    assert_eq!(fused_next.location, next_second.location);
    assert!(!fused_next.fusion_policy.location_from_next());

    let mut first_edge = Instruction::new(LOAD_FAST, Operand::Value(1), SourceLocation::NONE, None);
    first_edge.fusion_policy.prevent_with_previous();
    first_edge
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut second_edge =
        Instruction::new(LOAD_FAST, Operand::Value(2), SourceLocation::NONE, None);
    second_edge.fusion_policy.prevent_with_next();
    let fused_edges = Instruction::fused(
        LOAD_FAST_LOAD_FAST,
        Operand::Value(0x12),
        first_edge,
        second_edge,
    );
    assert!(fused_edges.fusion_policy.prevents_with_previous());
    assert!(fused_edges.fusion_policy.prevents_with_next());
    assert_eq!(fused_edges.jump_policy, JumpPolicy::default());
    assert_eq!(
        fused_edges.no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Reachable
    );
}

#[test]
fn owned_fast_loads_use_real_opcodes_with_provenance() {
    let mut assembler = Assembler::default();
    assembler.emit_owned_fast_with_depth(3, 1);
    let Some(Item::Instruction(instruction)) = assembler.linear_items().last() else {
        panic!("owned load was not emitted");
    };
    assert_eq!(instruction.opcode, LOAD_FAST);
    assert!(instruction.local_policy.force_owned_load());
    assert!(!instruction.local_policy.strict_owned_load());
}

#[test]
fn operand_patches_survive_item_insertion() {
    let mut assembler = Assembler::default();
    let patch = assembler.emit_placeholder_with_depth(LOAD_CONST, 1);
    let preceding = assembler.label();
    assembler.mark_before_trailing_instructions(preceding, 1);

    assembler.patch_argument(patch, 37).unwrap();
    assembler.resolve_operand_patches().unwrap();

    assert!(matches!(assembler.linear_items()[0], Item::Label(label) if label == preceding));
    assert!(matches!(
        assembler.linear_items()[1],
        Item::Instruction(instruction) if instruction.operand() == Operand::Value(37)
    ));
}

#[test]
fn operand_patches_reject_duplicate_resolution() {
    let mut assembler = Assembler::default();
    let patch = assembler.emit_placeholder_with_depth(LOAD_CONST, 1);

    assembler.patch_argument(patch, 1).unwrap();
    let error = assembler.patch_argument(patch, 2).unwrap_err();

    assert!(error.to_string().contains("resolved more than once"));
}

#[test]
fn operand_patches_must_be_resolved_and_attached() {
    let mut unresolved = Assembler::default();
    unresolved.emit_placeholder_with_depth(LOAD_CONST, 1);
    let error = unresolved.resolve_operand_patches().unwrap_err();
    assert!(error.to_string().contains("was never resolved"));

    let mut detached = Assembler::default();
    let patch = detached.emit_placeholder_with_depth(LOAD_CONST, 1);
    detached.patch_argument(patch, 1).unwrap();
    detached.clear_linear_items();
    let error = detached.resolve_operand_patches().unwrap_err();
    assert!(error.to_string().contains("has no instruction"));
}

#[test]
fn operand_patches_reject_duplicate_and_unknown_attachments() {
    let mut duplicate = Assembler::default();
    let patch = duplicate.emit_placeholder_with_depth(LOAD_CONST, 1);
    duplicate.patch_argument(patch, 1).unwrap();
    let item = duplicate.linear_items()[0];
    duplicate.push_linear_item(item);
    let error = duplicate.resolve_operand_patches().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("belongs to more than one instruction")
    );

    let mut unknown = Assembler::default();
    let patch = unknown.emit_placeholder_with_depth(LOAD_CONST, 1);
    unknown.patch_argument(patch, 1).unwrap();
    let Item::Instruction(mut instruction) = unknown.linear_items()[0] else {
        unreachable!();
    };
    instruction.set_operand(Operand::Deferred(OperandPatchId(1)));
    unknown.push_linear_item(Item::Instruction(instruction));
    let error = unknown.resolve_operand_patches().unwrap_err();
    assert!(error.to_string().contains("unknown operand patch"));
}

#[test]
fn structural_validation_reports_the_pass_and_broken_label() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(target));

    let error = assembler
        .validate_structure(AssemblerStage::ThreadForwardJumps)
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("thread_forward_jumps"));
    assert!(message.contains("unbound label"));
}

#[test]
fn structural_validation_rejects_duplicate_labels() {
    let mut assembler = Assembler::default();
    let label = assembler.label();
    assembler.mark(label);
    assembler.mark(label);

    let error = assembler
        .validate_structure(AssemblerStage::FinalLayout)
        .unwrap_err();
    assert!(error.to_string().contains("bound more than once"));
}

#[test]
fn structural_validation_rejects_a_misdirected_operand() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.mark(target);
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(target));

    let error = assembler
        .validate_structure(AssemblerStage::OptimizeBooleanConversions)
        .unwrap_err();
    assert!(error.to_string().contains("forward operand"));
}

#[test]
fn structural_validation_rejects_a_label_operand_on_a_non_jump() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.emit_operand(NOP, Operand::Forward(target));
    assembler.mark(target);

    let error = assembler
        .validate_structure(AssemblerStage::RemoveUnreachableInitial)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("label operand with non-jump opcode")
    );
}

#[test]
fn exception_regions_only_require_final_order_after_cfg_passes() {
    let mut assembler = Assembler::default();
    let end = assembler.label();
    let start = assembler.label();
    let target = assembler.label();
    assembler.mark(end);
    assembler.mark(start);
    assembler.mark(target);
    assembler.add_exception_region(start, end, target, 0, false);

    assembler
        .validate_structure(AssemblerStage::RemoveUnreachableInitial)
        .unwrap();
    let error = assembler
        .validate_structure(AssemblerStage::FinalLayout)
        .unwrap_err();
    assert!(error.to_string().contains("starts at"));
}

#[test]
fn removes_unreachable_jumps() {
    let jump = JUMP_FORWARD;
    let resume = RESUME;
    let mut assembler = Assembler::default();
    let start = assembler.label();
    let end = assembler.label();

    assembler.mark(start);
    assembler.emit(resume, 0);
    assembler.emit_operand(jump, Operand::Forward(end));
    assembler.emit_backward(jump, start);
    assembler.mark(end);

    assert_eq!(assembler.finish().unwrap(), [RESUME.code(), 0]);
}

#[test]
fn emits_extended_arguments() {
    let mut assembler = Assembler::default();
    assembler.emit(LOAD_CONST, 0x1234);
    assert_eq!(
        assembler.finish().unwrap(),
        [EXTENDED_ARG.code(), 0x12, LOAD_CONST.code(), 0x34]
    );
}

#[test]
fn target_caches_and_extended_arguments_share_the_same_layout() {
    use crate::PythonVersion::{Py314, Py315};

    for version in [Py314, Py315] {
        let mut assembler = Assembler::with_python_version(version);
        assembler.emit(RESUME, ResumeLocation::AT_FUNC_START.argument());
        assembler.emit(IMPORT_NAME, if version == Py314 { 64 } else { 256 });
        assembler.emit(RETURN_VALUE, 0);
        let expected = if version == Py314 {
            vec![
                RESUME.code(),
                0,
                IMPORT_NAME.code(),
                64,
                RETURN_VALUE.code(),
                0,
            ]
        } else {
            vec![
                RESUME.code_for(Py315),
                0,
                0,
                0,
                EXTENDED_ARG.code_for(Py315),
                1,
                IMPORT_NAME.code_for(Py315),
                0,
                RETURN_VALUE.code_for(Py315),
                0,
            ]
        };
        assert_eq!(assembler.finish().unwrap(), expected);
    }
}

#[test]
fn python315_resume_entry_and_depth_have_distinct_encodings() {
    use crate::PythonVersion::Py315;
    use crate::target::GENEXPR_RESUME_ARGUMENT;

    for (argument, expected) in [
        (ResumeLocation::AFTER_YIELD.at_depth(true), 9),
        (ResumeLocation::AFTER_AWAIT.at_depth(true), 11),
        (GENEXPR_RESUME_ARGUMENT, 4),
    ] {
        let mut assembler = Assembler::with_python_version(Py315);
        assembler.emit(RESUME, argument);
        assert_eq!(
            assembler.finish().unwrap(),
            [RESUME.code_for(Py315), expected, 0, 0]
        );
    }
}

#[test]
fn python315_yield_from_uses_quickened_get_iter_and_cache() {
    use crate::PythonVersion::Py315;

    let mut assembler = Assembler::with_python_version(Py315);
    assembler.emit(GET_YIELD_FROM_ITER, 0);
    assert_eq!(
        assembler.finish().unwrap(),
        [GET_ITER.code_for(Py315), 3, 0, 0]
    );
}

#[test]
fn python315_borrow_traversal_crosses_removed_constant_branches() {
    use crate::PythonVersion::{Py314, Py315};
    use crate::code_object::Constant;
    use crate::{CompileOptions, compile_with_options};

    for source in [
        "def f(x):\n    if False:\n        pass\n    return x\n",
        "def f(x):\n    while False:\n        pass\n    return x\n",
        "def f(x):\n    if flag:\n        return\n    assert False, x.value\n",
        "def f(x):\n    if flag:\n        pass\n    else:\n        assert False, f'bad: {x}'\n",
    ] {
        for version in [Py314, Py315] {
            let module = compile_with_options(
                source,
                "borrow.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let code = module
                .code
                .constants
                .iter()
                .find_map(|constant| {
                    if let Constant::Code(code) = constant {
                        Some(code)
                    } else {
                        None
                    }
                })
                .unwrap();
            let expected = if version == Py314 {
                LOAD_FAST
            } else {
                LOAD_FAST_BORROW
            };
            assert!(
                code.bytecode
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|word| *word == [expected.code_for(version), 0]),
                "{version:?}: {source}"
            );
        }
    }
}

#[test]
fn terminal_assignment_fallthrough_keeps_value_owned() {
    use crate::code_object::Constant;
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let module = compile_with_options(
            "def f(self, value):\n    self.value = {} if value is None else value\n",
            "tail.py",
            &CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        let code = module
            .code
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) => Some(code),
                _ => None,
            })
            .unwrap();
        assert!(code.bytecode.windows(6).any(|window| {
            window
                == [
                    LOAD_FAST.code_for(version),
                    1,
                    LOAD_FAST_BORROW.code_for(version),
                    0,
                    STORE_ATTR.code_for(version),
                    0,
                ]
        }));
    }
}

#[test]
fn emits_full_width_extended_arguments_and_zeroed_inline_caches() {
    let mut assembler = Assembler::default();
    assembler.emit(LOAD_GLOBAL, 0x1234_5678);
    assert_eq!(
        assembler.finish().unwrap(),
        [
            EXTENDED_ARG.code(),
            0x12,
            EXTENDED_ARG.code(),
            0x34,
            EXTENDED_ARG.code(),
            0x56,
            LOAD_GLOBAL.code(),
            0x78,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]
    );
}

#[test]
fn fused_final_artifacts_preserve_cache_extended_argument_location_and_depth_semantics() {
    let first_location = SourceLocation::new(7, 7, 0, 1);
    let second_location = SourceLocation::new(8, 8, 2, 4);
    let assembler = super::LayoutAssembler {
        python_version: crate::PythonVersion::Py314,
        items: vec![
            Item::Instruction(Instruction::new(
                LOAD_GLOBAL,
                Operand::Value(0x1234_5678),
                first_location,
                Some(3),
            )),
            Item::Label(super::Label(0)),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(0x1234),
                first_location,
                None,
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(2),
                second_location,
                Some(12),
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(3),
                SourceLocation::NONE,
                Some(5),
            )),
        ],
        exception_regions: Vec::new(),
        exception_owner_provenance: super::exception_owners::ExceptionOwnerProvenance::default(),
        preserved_block_boundaries: rustc_hash::FxHashSet::default(),
        removed_max_depth: None,
        next_label: 1,
        has_next_location: false,
    };

    let (bytecode, line_table, max_depth) =
        assembler.encode_final_artifacts(&[3, 1, 0, 0], &[0x1234_5678, 0x1234, 2, 3], 12, 7);

    assert_eq!(
        bytecode,
        [
            EXTENDED_ARG.code(),
            0x12,
            EXTENDED_ARG.code(),
            0x34,
            EXTENDED_ARG.code(),
            0x56,
            LOAD_GLOBAL.code(),
            0x78,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            EXTENDED_ARG.code(),
            0x12,
            LOAD_CONST.code(),
            0x34,
            LOAD_CONST.code(),
            2,
            LOAD_CONST.code(),
            3,
        ]
    );
    assert_eq!(line_table, [0x87, 0x01, 0x81, 0x01, 0xd8, 2, 4, 0xf8]);
    assert_eq!(max_depth, 12);
}

#[test]
fn unextended_final_artifacts_preserve_jump_cache_location_and_depth_semantics() {
    let first_location = SourceLocation::new(7, 7, 0, 1);
    let second_location = SourceLocation::new(8, 8, 2, 4);
    let target = super::Label(0);
    let assembler = super::LayoutAssembler {
        python_version: crate::PythonVersion::Py314,
        items: vec![
            Item::Instruction(Instruction::new(
                LOAD_GLOBAL,
                Operand::Value(7),
                first_location,
                Some(3),
            )),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(target),
                first_location,
                Some(2),
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(9),
                second_location,
                Some(12),
            )),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(10),
                SourceLocation::NONE,
                Some(5),
            )),
            Item::Label(target),
            Item::Instruction(Instruction::new(
                JUMP_BACKWARD,
                Operand::Backward(target),
                second_location,
                Some(1),
            )),
        ],
        exception_regions: Vec::new(),
        exception_owner_provenance: super::exception_owners::ExceptionOwnerProvenance::default(),
        preserved_block_boundaries: rustc_hash::FxHashSet::default(),
        removed_max_depth: None,
        next_label: 1,
        has_next_location: false,
    };
    let mut labels = vec![None];
    let code_units = assembler.label_positions(&[0; 5], &mut labels);
    let expected = assembler.encode_final_artifacts(&[0; 5], &[7, 2, 9, 10, 2], code_units, 7);

    let actual = assembler
        .encode_unextended_final_artifacts(&labels, code_units, 7)
        .unwrap()
        .expect("all arguments fit in one code unit");

    assert_eq!(actual, expected);
}

#[test]
fn extended_forward_jump_uses_the_iterative_layout_fallback() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.emit(LOAD_CONST, 0);
    assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, target, 0, ExceptionPolicy::default());
    for _ in 0..=u8::MAX {
        assembler.emit(RESUME, ResumeLocation::AT_FUNC_START.argument());
    }
    assembler.mark(target);
    assembler.emit(RETURN_VALUE, 0);

    let bytecode = assembler.finish().unwrap();

    assert_eq!(
        &bytecode[..6],
        [
            LOAD_CONST.code(),
            0,
            EXTENDED_ARG.code(),
            1,
            POP_JUMP_IF_FALSE.code(),
            1,
        ]
    );
}

#[test]
fn removes_an_overwritten_local_store() {
    let mut assembler = Assembler::default();
    assembler.emit(STORE_FAST, 1);
    assembler.emit(STORE_FAST, 1);
    assert_eq!(
        assembler.finish().unwrap(),
        [POP_TOP.code(), 0, STORE_FAST.code(), 1]
    );
}

#[test]
fn constant_pop_candidates_are_recorded_during_emission() {
    let mut separated = Assembler::default();
    separated.emit(LOAD_CONST, 0);
    let boundary = separated.label();
    separated.mark(boundary);
    separated.emit(POP_TOP, 0);
    assert!(!separated.has_constant_pop_candidate);
    separated.optimize_constant_pops();
    assert!(matches!(
        separated.linear_items(),
        [Item::Instruction(load), Item::Label(_), Item::Instruction(pop)]
            if load.opcode == LOAD_CONST && pop.opcode == POP_TOP
    ));

    let mut direct = Assembler::default();
    direct.emit(
        LOAD_COMMON_CONSTANT,
        CommonConstant::ASSERTIONERROR.argument(),
    );
    direct.emit(POP_TOP, 0);
    assert!(direct.has_constant_pop_candidate);
    direct.optimize_constant_pops();
    assert!(matches!(
        direct.linear_items(),
        [Item::Instruction(first), Item::Instruction(second)]
            if first.opcode == NOP && second.opcode == NOP
    ));

    let mut forwarded = Assembler::default();
    let target = forwarded.label();
    forwarded.emit(LOAD_SMALL_INT, 0);
    forwarded.emit_forward_with_depth(JUMP_FORWARD, target, 1, ExceptionPolicy::default());
    forwarded.mark(target);
    forwarded.emit(POP_TOP, 0);
    forwarded.emit(LOAD_CONST, 1);
    forwarded.emit(RETURN_VALUE, 0);
    assert!(forwarded.has_constant_pop_candidate);
    forwarded.optimize_constant_pops();
    assert!(matches!(
        forwarded.linear_items()[..2],
        [Item::Instruction(load), Item::Instruction(return_value)]
            if load.opcode == LOAD_CONST
                && matches!(load.operand(), Operand::Value(1))
                && return_value.opcode == RETURN_VALUE
    ));
}

#[test]
fn discarded_value_emission_matches_handle_emission() {
    fn assert_same_stream(expected: &Assembler, actual: &Assembler) {
        assert_eq!(expected.instruction_count(), actual.instruction_count());
        assert_eq!(
            expected.has_constant_pop_candidate,
            actual.has_constant_pop_candidate
        );
        assert_eq!(expected.linear_items().len(), actual.linear_items().len());
        for (expected, actual) in expected.linear_items().iter().zip(actual.linear_items()) {
            let (Item::Instruction(expected), Item::Instruction(actual)) = (expected, actual)
            else {
                panic!("emission streams should contain only instructions");
            };
            assert_eq!(actual.opcode, expected.opcode);
            assert_eq!(actual.operand(), expected.operand());
            assert_eq!(actual.location, expected.location);
            assert_eq!(actual.depth_after(), expected.depth_after());
            assert_eq!(actual.local_policy, expected.local_policy);
            assert_eq!(actual.no_location_policy, expected.no_location_policy);
            assert_eq!(actual.fusion_policy, expected.fusion_policy);
            assert_eq!(actual.jump_policy, expected.jump_policy);
            assert_eq!(actual.exception_policy(), expected.exception_policy());
        }
    }

    let location = SourceLocation::new(2, 2, 4, 12);
    for constant in [LOAD_CONST, LOAD_COMMON_CONSTANT, LOAD_SMALL_INT] {
        let mut expected = Assembler::default();
        let mut actual = Assembler::default();
        expected.set_location(location);
        actual.set_location(location);
        expected.emit_with_depth(constant, 3, 1, ExceptionPolicy::default());
        actual.emit_value_with_depth_discard(constant, 3, 1, ExceptionPolicy::default());
        expected.emit_with_depth(POP_TOP, 0, 0, ExceptionPolicy::default());
        actual.emit_value_with_depth_discard(POP_TOP, 0, 0, ExceptionPolicy::default());

        assert!(actual.has_constant_pop_candidate);
        assert_same_stream(&expected, &actual);
    }

    for (disable_borrowing, strict_owned, forced_borrowed) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut expected = Assembler::default();
        let mut actual = Assembler::default();
        expected.set_location(location);
        actual.set_location(location);
        if disable_borrowing {
            expected.disable_load_fast_borrowing();
            actual.disable_load_fast_borrowing();
        }
        expected.set_strict_owned_loads(strict_owned);
        actual.set_strict_owned_loads(strict_owned);
        expected.set_forced_borrowed_loads(forced_borrowed);
        actual.set_forced_borrowed_loads(forced_borrowed);
        expected.emit_with_depth(LOAD_FAST, 2, 1, ExceptionPolicy::default());
        actual.emit_value_with_depth_discard(LOAD_FAST, 2, 1, ExceptionPolicy::default());

        assert_same_stream(&expected, &actual);
    }

    let mut expected = Assembler::default();
    let mut actual = Assembler::default();
    expected.set_location(location);
    actual.set_location(location);
    expected.emit_with_depth(LOAD_CONST, 2, 1, ExceptionPolicy::EXCLUDE_ALWAYS);
    actual.emit_value_with_depth_discard(LOAD_CONST, 2, 1, ExceptionPolicy::EXCLUDE_ALWAYS);
    assert_same_stream(&expected, &actual);
}

fn assembler_with_preserved_constant_boundary() -> Assembler {
    let entry = super::Label(0);
    let preserved = super::Label(1);
    let target = super::Label(2);
    let mut assembler = Assembler::from_linear_items(vec![
        Item::Label(entry),
        Item::Label(preserved),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(LOAD_CONST, 1, 1),
        Item::Label(target),
        pass_instruction(LOAD_CONST, 2, 2),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);
    assembler.next_label = 3;
    assembler.preserved_block_boundaries = rustc_hash::FxHashSet::from_iter([preserved]);
    assembler
}

#[test]
fn constant_reachability_prepares_the_optimization_graph() {
    let expected = assembler_with_preserved_constant_boundary()
        .finish()
        .unwrap();
    let mut assembler = assembler_with_preserved_constant_boundary();

    assert_eq!(
        assembler.used_constant_indices(LOAD_CONST, 3),
        [true, false, true]
    );
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    assembler.remap_constant_indices(LOAD_CONST, &[Some(0), Some(1), Some(2)]);

    assert_eq!(assembler.finish().unwrap(), expected);
}

#[test]
fn prepared_reachability_survives_preserved_boundary_removal() {
    let mut assembler = assembler_with_preserved_constant_boundary();
    assert_eq!(
        assembler.used_constant_indices(LOAD_CONST, 3),
        [true, false, true]
    );
    let UnoptimizedCode::Prepared { reachable, .. } = &assembler.code else {
        panic!("constant reachability should prepare the optimization graph");
    };
    assert_eq!(reachable, &[true, true, false, true]);

    let mut pipeline = assembler.into_optimization_pipeline();
    assert_eq!(
        pipeline.initial_reachable.as_deref(),
        Some([true, false, true].as_slice())
    );
    assert_eq!(pipeline.remove_unreachable_instructions(), None);
    assert!(pipeline.initial_reachable.is_none());
    assert_eq!(pipeline.graph.len(), 2);
}

#[test]
fn straight_line_constant_reachability_uses_dense_indices() {
    let mut assembler = Assembler::from_linear_items(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        pass_instruction(LOAD_CONST, 2, 1),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert_eq!(
        assembler.used_constant_indices(LOAD_CONST, 3),
        [true, false, true]
    );
}

#[test]
fn assertion_constant_reachability_keeps_the_optimization_stream_linear() {
    let target = super::Label(0);
    let assembler = || {
        let mut assembler = Assembler::from_linear_items(vec![
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_TRUE,
                Operand::Forward(target),
                SourceLocation::new(1, 1, 0, 0),
                None,
            )),
            pass_instruction(NOT_TAKEN, 0, 1),
            pass_instruction(
                LOAD_COMMON_CONSTANT,
                CommonConstant::ASSERTIONERROR.argument(),
                1,
            ),
            pass_instruction(RAISE_VARARGS, 1, 1),
            Item::Label(target),
            pass_instruction(LOAD_CONST, 2, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]);
        assembler.next_label = 1;
        assembler.has_assertion_side_exit_candidate = true;
        assembler
    };
    let expected = assembler().finish().unwrap();
    let mut assembler = assembler();

    assert_eq!(
        assembler.used_constant_indices(LOAD_CONST, 3),
        [true, false, true]
    );
    assert!(matches!(assembler.code, UnoptimizedCode::Linear(_)));
    assert!(assembler.has_proven_linear_assertion_side_exits);
    assembler.remap_constant_indices(LOAD_CONST, &[Some(0), Some(1), Some(2)]);

    assert_eq!(assembler.finish().unwrap(), expected);
}

#[test]
fn assertion_constant_reachability_rejects_mixed_control_flow() {
    let assertion_target = super::Label(0);
    let other_target = super::Label(1);
    let mut assembler = Assembler::from_linear_items(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(assertion_target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(assertion_target),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(other_target),
            SourceLocation::new(2, 2, 0, 0),
            None,
        )),
        pass_instruction(LOAD_CONST, 1, 2),
        Item::Label(other_target),
        pass_instruction(LOAD_CONST, 2, 3),
        pass_instruction(RETURN_VALUE, 0, 3),
    ]);
    assembler.next_label = 2;
    assembler.has_assertion_side_exit_candidate = true;

    assert_eq!(
        assembler.used_constant_indices(LOAD_CONST, 3),
        [true, false, true]
    );
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
}

#[test]
fn assertion_constant_reachability_keeps_swap_and_borrow_markers_off_the_linear_fast_path() {
    for borrow_marker in [false, true] {
        let target = super::Label(0);
        let mut jump = Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        );
        if borrow_marker {
            jump.jump_policy.mark_borrow_empty_block();
        }
        let mut items = vec![pass_instruction(LOAD_CONST, 0, 1)];
        if !borrow_marker {
            items.extend([
                pass_instruction(LOAD_CONST, 2, 1),
                pass_instruction(SWAP, 2, 1),
                pass_instruction(POP_TOP, 0, 1),
            ]);
        }
        items.extend([
            Item::Instruction(jump),
            pass_instruction(NOT_TAKEN, 0, 1),
            pass_instruction(
                LOAD_COMMON_CONSTANT,
                CommonConstant::ASSERTIONERROR.argument(),
                1,
            ),
            pass_instruction(RAISE_VARARGS, 1, 1),
            Item::Label(target),
            pass_instruction(LOAD_CONST, 2, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]);
        let mut assembler = Assembler::from_linear_items(items);
        assembler.next_label = 1;
        assembler.has_assertion_side_exit_candidate = true;

        assert_eq!(
            assembler.used_constant_indices(LOAD_CONST, 3),
            [true, false, true]
        );
        assert!(matches!(assembler.code, UnoptimizedCode::Linear(_)));
        assert!(!assembler.has_proven_linear_assertion_side_exits);
        assert!(!assembler.has_only_linear_assertion_side_exits());
    }
}

fn pass_instruction(opcode: crate::target::Opcode, argument: u32, line: i32) -> Item {
    Item::Instruction(Instruction::new(
        opcode,
        Operand::Value(argument),
        SourceLocation::new(line, line, 0, 0),
        None,
    ))
}

fn optimization_pipeline(items: Vec<Item>) -> super::OptimizationPipeline {
    let next_label = items
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) => Some(label.0 + 1),
            Item::Instruction(instruction) => match instruction.operand() {
                Operand::Forward(label) | Operand::Backward(label) => Some(label.0 + 1),
                Operand::Value(_) | Operand::Deferred(_) => None,
            },
        })
        .max()
        .unwrap_or(0);
    let mut assembler = Assembler::from_linear_items(items);
    assembler.next_label = next_label;
    assembler.into_optimization_pipeline()
}

#[test]
fn backward_conditional_normalization_preserves_successor_labels() {
    for labelled_successor in [false, true] {
        let restart = super::Label(0);
        let mut items = vec![
            Item::Label(restart),
            pass_instruction(LOAD_FAST, 0, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Backward(restart),
                SourceLocation::new(1, 1, 0, 1),
                None,
            )),
        ];
        if labelled_successor {
            items.push(Item::Label(super::Label(1)));
        }
        items.extend([
            pass_instruction(NOT_TAKEN, 0, 1),
            pass_instruction(LOAD_CONST, 0, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]);
        let mut pipeline = optimization_pipeline(items);

        assert!(pipeline.normalize_backward_conditionals().unwrap());
        assert!(!pipeline.normalize_backward_conditionals().unwrap());

        pipeline.graph.validate().unwrap();
        for item in pipeline.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(target) | Operand::Backward(target) = instruction.operand()
            {
                assert!(pipeline.graph.label_block(target).is_some());
            }
        }
        assert_eq!(
            graph_opcodes(&pipeline),
            [
                LOAD_FAST,
                POP_JUMP_IF_TRUE,
                NOT_TAKEN,
                JUMP_BACKWARD,
                LOAD_CONST,
                RETURN_VALUE
            ],
        );
    }
}

#[test]
fn assertion_side_exit_fast_path_accepts_all_conditional_directions() {
    let targets = [
        super::Label(0),
        super::Label(1),
        super::Label(2),
        super::Label(3),
    ];
    let mut items = Vec::new();
    for (line, (opcode, target)) in [
        POP_JUMP_IF_TRUE,
        POP_JUMP_IF_FALSE,
        POP_JUMP_IF_NONE,
        POP_JUMP_IF_NOT_NONE,
    ]
    .into_iter()
    .zip(targets)
    .enumerate()
    {
        let line = i32::try_from(line + 1).unwrap();
        items.push(pass_instruction(LOAD_FAST, 0, line));
        items.push(Item::Instruction(Instruction::new(
            opcode,
            Operand::Forward(target),
            SourceLocation::new(line, line, 0, 0),
            None,
        )));
        items.push(pass_instruction(NOT_TAKEN, 0, line));
        items.push(pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            line,
        ));
        items.push(pass_instruction(RAISE_VARARGS, 1, line));
        items.push(Item::Label(target));
    }
    items.push(pass_instruction(RETURN_VALUE, 0, 4));

    let mut assembler = optimization_pipeline(items);
    assembler.has_assertion_side_exit_candidate = true;
    assert!(assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_mixed_control_flow() {
    let assertion_target = super::Label(0);
    let other_target = super::Label(1);
    let assertion = [
        pass_instruction(LOAD_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(assertion_target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(assertion_target),
    ];

    let mut unconditional = assertion.to_vec();
    unconditional.push(Item::Instruction(Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(other_target),
        SourceLocation::NONE,
        None,
    )));
    unconditional.push(Item::Label(other_target));
    unconditional.push(pass_instruction(RETURN_VALUE, 0, 2));
    let mut unconditional = optimization_pipeline(unconditional);
    unconditional.has_assertion_side_exit_candidate = true;
    assert!(!unconditional.has_only_assertion_side_exits());

    let mut conditional = assertion.to_vec();
    conditional.push(pass_instruction(LOAD_FAST, 1, 2));
    conditional.push(Item::Instruction(Instruction::new(
        POP_JUMP_IF_FALSE,
        Operand::Forward(other_target),
        SourceLocation::new(2, 2, 0, 0),
        None,
    )));
    conditional.push(pass_instruction(NOT_TAKEN, 0, 2));
    conditional.push(pass_instruction(LOAD_CONST, 0, 2));
    conditional.push(Item::Label(other_target));
    conditional.push(pass_instruction(RETURN_VALUE, 0, 2));
    let mut conditional = optimization_pipeline(conditional);
    conditional.has_assertion_side_exit_candidate = true;
    assert!(!conditional.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_single_block_without_scanning() {
    let assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert_eq!(assembler.graph.len(), 1);
    assert!(!assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_annotation_guard_without_scanning() {
    let target = super::Label(0);
    let assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(LOAD_GLOBAL, 0, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.graph.len() >= 3);
    assert!(!assembler.has_assertion_side_exit_candidate);
    assert!(!assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_rejects_exception_regions() {
    let target = super::Label(0);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);
    assembler.exception_regions.push(super::ExceptionRegion {
        priority: super::ExceptionRegionPriority::OriginalRange(target, target),
        is_scope_fallback: false,
        start: target,
        end: target,
        target,
        depth: 0,
        preserve_lasti: false,
    });
    assembler.has_assertion_side_exit_candidate = true;

    assert!(!assembler.has_only_assertion_side_exits());
}

#[test]
fn assertion_side_exit_fast_path_propagates_final_condition_location() {
    let target = super::Label(0);
    let location = SourceLocation::new(7, 7, 4, 19);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST, 0, 7),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            location,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 7),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            7,
        ),
        pass_instruction(RAISE_VARARGS, 1, 7),
        Item::Label(target),
        Item::Instruction(Instruction::new(
            LOAD_CONST,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )),
        Item::Instruction(Instruction::new(
            RETURN_VALUE,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )),
    ]);
    assembler.has_assertion_side_exit_candidate = true;
    assert!(assembler.has_only_assertion_side_exits());

    assembler.propagate_assertion_exit_locations();

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions[instructions.len() - 2].location, location);
}

fn graph_opcodes(assembler: &super::OptimizationPipeline) -> Vec<crate::target::Opcode> {
    assembler
        .graph
        .iter_items()
        .filter_map(|item| {
            let Item::Instruction(instruction) = item else {
                return None;
            };
            Some(instruction.opcode)
        })
        .collect()
}

#[test]
fn threads_unconditional_forward_jumps_without_conditional_jumps() {
    let first_target = super::Label(0);
    let final_target = super::Label(1);
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(first_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(first_target),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.thread_forward_jumps());

    let graph = &assembler.graph;
    let first = graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .unwrap();
    assert!(matches!(first.operand(), Operand::Forward(label) if label == final_target));
}

#[test]
fn threads_forward_operands_with_backward_jump_kinds() {
    for source_opcode in [JUMP_FORWARD, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT] {
        for intermediate in [None, Some(JUMP_BACKWARD)] {
            let entry = super::Label(0);
            let source_label = super::Label(1);
            let first_target = super::Label(2);
            let last_target = super::Label(3);
            let mut source = Instruction::new(
                source_opcode,
                Operand::Forward(first_target),
                SourceLocation::new(10, 10, 2, 9),
                Some(0),
            );
            source.mark_initial_borrow_jump_target(first_target);
            source.jump_policy.retain_nop_for_direct_target();
            let mut items = vec![
                Item::Label(entry),
                pass_instruction(NOP, 0, 1),
                Item::Label(source_label),
                Item::Instruction(source),
                Item::Label(first_target),
            ];
            if let Some(opcode) = intermediate {
                items.extend([
                    Item::Instruction(Instruction::new(
                        opcode,
                        Operand::Forward(last_target),
                        SourceLocation::NONE,
                        None,
                    )),
                    Item::Label(last_target),
                ]);
            }
            items.push(Item::Instruction(Instruction::new(
                JUMP_BACKWARD_NO_INTERRUPT,
                Operand::Backward(entry),
                SourceLocation::NONE,
                None,
            )));
            let mut assembler = optimization_pipeline(items);
            assert!(assembler.exception_regions.is_empty());
            assert!(assembler.thread_forward_jumps());
            let block = assembler.graph.label_block(source_label).unwrap();
            let actual = *assembler.graph[block].last_instruction().unwrap();
            let keeps_interrupt = source_opcode != JUMP_FORWARD
                && (source_opcode == JUMP_BACKWARD || intermediate.is_some());
            let mut expected = source;
            expected.add_pre_fold_extra_instructions(1 + u8::from(intermediate.is_some()));
            expected.retarget(Operand::Backward(entry));
            expected.opcode = if keeps_interrupt {
                JUMP_BACKWARD
            } else {
                JUMP_BACKWARD_NO_INTERRUPT
            };
            if keeps_interrupt {
                expected.jump_policy.mark_interruptible();
            }
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        }
    }
}

#[test]
fn forward_operand_threading_preserves_barriers_and_unthreaded_metadata() {
    for control in 0..4 {
        let entry = super::Label(0);
        let source_label = super::Label(1);
        let target = super::Label(2);
        let mut source = Instruction::new(
            JUMP_BACKWARD,
            if control == 2 {
                Operand::Backward(target)
            } else {
                Operand::Forward(target)
            },
            SourceLocation::new(10, 10, 2, 9),
            Some(0),
        );
        let mut destination = Instruction::new(
            if control == 3 {
                RETURN_VALUE
            } else {
                JUMP_BACKWARD_NO_INTERRUPT
            },
            if control == 3 {
                Operand::Value(0)
            } else {
                Operand::Backward(entry)
            },
            SourceLocation::NONE,
            None,
        );
        source.jump_policy.retain_nop_for_direct_target();
        if control == 0 {
            source.jump_policy.retain_nop();
        }
        if control == 1 {
            destination.jump_policy.retain_nop();
        }
        let mut assembler = optimization_pipeline(vec![
            Item::Label(entry),
            pass_instruction(NOP, 0, 1),
            Item::Label(source_label),
            Item::Instruction(source),
            Item::Label(target),
            Item::Instruction(destination),
        ]);
        let original = format!("{:?}", assembler.graph.iter_items().collect::<Vec<_>>());
        assert!(!assembler.thread_forward_jumps(), "control {control}");
        assert_eq!(
            format!("{:?}", assembler.graph.iter_items().collect::<Vec<_>>()),
            original,
            "control {control}",
        );
    }
}

#[test]
fn a_threaded_target_cycle_keeps_direct_target_retention() {
    let first = super::Label(0);
    let second = super::Label(1);
    let mut source = Instruction::new(
        JUMP_BACKWARD,
        Operand::Forward(first),
        SourceLocation::new(10, 10, 2, 9),
        Some(0),
    );
    source.jump_policy.retain_nop_for_direct_target();
    let mut interruptible_target = Instruction::new(
        JUMP_BACKWARD_NO_INTERRUPT,
        Operand::Forward(first),
        SourceLocation::NONE,
        None,
    );
    interruptible_target.jump_policy.mark_interruptible();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(source),
        Item::Label(first),
        Item::Instruction(Instruction::new(
            JUMP_BACKWARD_NO_INTERRUPT,
            Operand::Forward(second),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(second),
        Item::Instruction(interruptible_target),
    ]);
    assembler.thread_forward_jumps();
    let actual = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) => Some(*instruction),
            Item::Label(_) => None,
        })
        .unwrap();
    let mut expected = source;
    expected.add_pre_fold_extra_instructions(2);
    // The bounded walk returns to the same forward operand, so retargeting has
    // not cleared the original direct-target NOP policy. Inherited interrupt
    // checks must not overwrite it or trigger mark_interruptible's assertion.
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
}

#[test]
fn threads_unconditional_forward_jumps_without_conditional_targets() {
    let conditional_target = super::Label(0);
    let first_target = super::Label(1);
    let final_target = super::Label(2);
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(conditional_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(first_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(first_target),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 1),
        Item::Label(conditional_target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.thread_forward_jumps());

    let graph = &assembler.graph;
    let first_jump = graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.opcode == JUMP_FORWARD => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert!(matches!(first_jump.operand(), Operand::Forward(label) if label == final_target));
}

#[test]
fn threads_opposite_conditional_jumps_through_a_new_fallthrough_label() {
    let inner_target = super::Label(0);
    let final_target = super::Label(1);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(COPY, 1, 1),
        pass_instruction(TO_BOOL, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(inner_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(POP_TOP, 0, 1),
        Item::Label(inner_target),
        pass_instruction(COPY, 1, 2),
        pass_instruction(TO_BOOL, 0, 2),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 2),
        pass_instruction(POP_TOP, 0, 2),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 3),
    ]);

    assert!(assembler.thread_forward_jumps());

    let fallthrough = super::Label(2);
    let items = assembler.graph.iter_items().copied().collect::<Vec<_>>();
    let jumps = items
        .iter()
        .filter_map(|item| match item {
            Item::Instruction(instruction)
                if matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE) =>
            {
                Some(instruction.operand())
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        jumps,
        [
            Operand::Forward(fallthrough),
            Operand::Forward(final_target),
        ]
    );
    assert!(
        items
            .windows(2)
            .any(|items| matches!(items, [Item::Label(label), Item::Instruction(instruction)] if *label == fallthrough && instruction.opcode == POP_TOP))
    );
}

#[test]
fn threads_chained_conditional_jumps_to_the_final_target() {
    let middle_target = super::Label(0);
    let inner_target = super::Label(1);
    let final_target = super::Label(2);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(COPY, 1, 1),
        pass_instruction(TO_BOOL, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(middle_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(POP_TOP, 0, 1),
        Item::Label(middle_target),
        pass_instruction(COPY, 1, 2),
        pass_instruction(TO_BOOL, 0, 2),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(inner_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 2),
        pass_instruction(POP_TOP, 0, 2),
        Item::Label(inner_target),
        pass_instruction(COPY, 1, 3),
        pass_instruction(TO_BOOL, 0, 3),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(final_target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 3),
        pass_instruction(POP_TOP, 0, 3),
        Item::Label(final_target),
        pass_instruction(RETURN_VALUE, 0, 4),
    ]);

    assert!(assembler.thread_forward_jumps());

    let jumps = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) if instruction.opcode == POP_JUMP_IF_FALSE => {
                Some(instruction.operand())
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(jumps, [Operand::Forward(final_target); 3]);
    let policies = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction.exception_policy()),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    for start in [0, 5] {
        assert_eq!(
            policies[start..start + 3],
            [ExceptionPolicy::EXCLUDE_ALWAYS; 3]
        );
        assert_eq!(policies[start + 3], ExceptionPolicy::default());
    }
    assert_eq!(policies[10..], [ExceptionPolicy::default(); 6]);
}

#[test]
fn threading_reports_no_edge_change_for_terminal_targets() {
    let target = super::Label(0);
    let mut direct_jump = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);
    assert!(!direct_jump.thread_forward_jumps());

    let mut direct_condition = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        )),
        pass_instruction(RETURN_VALUE, 0, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);
    assert!(!direct_condition.thread_forward_jumps());
}

#[test]
fn threaded_edge_change_retains_second_reachability_cleanup_and_removed_depth() {
    let bypassed = super::Label(0);
    let target = super::Label(1);
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(bypassed),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(bypassed),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            Some(7),
        )),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assert!(assembler.thread_forward_jumps());
    assert_eq!(assembler.remove_unreachable_instructions(), Some(7));
    assert_eq!(graph_opcodes(&assembler), [JUMP_FORWARD, RETURN_VALUE]);
}

#[test]
fn redundant_jump_removal_preserves_borrow_traversal_barrier() {
    let target = super::Label(0);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    jump.no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(jump),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0].opcode, LOAD_CONST);
    assert_eq!(
        instructions[0].no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(instructions[1].opcode, RETURN_VALUE);
}

#[test]
fn redundant_jump_removal_streams_consecutive_cross_block_jumps() {
    let intervening_label = super::Label(0);
    let first_target = super::Label(1);
    let second_target = super::Label(2);
    let location = SourceLocation::new(7, 7, 0, 0);
    let mut first = Instruction::new(JUMP_FORWARD, Operand::Forward(first_target), location, None);
    first.exclude_from_exception_table(ExceptionExclusion::IfExtended);
    first
        .no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let second = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(second_target),
        SourceLocation::NONE,
        None,
    );
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 7),
        Item::Instruction(first),
        Item::Label(intervening_label),
        Item::Label(first_target),
        Item::Instruction(second),
        Item::Label(second_target),
        Item::Instruction(Instruction::new(
            RETURN_VALUE,
            Operand::Value(0),
            SourceLocation::NONE,
            None,
        )),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0].opcode, LOAD_CONST);
    assert_eq!(
        instructions[0].exception_policy().exclusion(),
        ExceptionExclusion::IfExtended
    );
    assert_eq!(
        instructions[0].no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(instructions[1].opcode, RETURN_VALUE);
    assert_eq!(instructions[1].location, location);
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>(),
        [intervening_label, first_target, second_target]
    );
}

#[test]
fn deferred_nonadjacent_forward_jump_is_still_resolved() {
    let target = super::Label(0);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    jump.jump_policy.defer_removal();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(jump),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assembler.remove_redundant_forward_jumps(false);

    let jump = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.opcode == JUMP_FORWARD => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert_eq!(jump.operand(), Operand::Forward(target));
    assert!(!jump.jump_policy.defers_redundant_removal());
}

#[test]
fn borrow_empty_block_marker_survives_until_local_analysis() {
    let target = super::Label(0);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    jump.jump_policy.mark_borrow_empty_block();
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(jump),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 1),
    ]);

    assembler.remove_redundant_forward_jumps(false);
    let retained = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.jump_policy.borrow_empty_block() => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert_eq!(retained.opcode, JUMP_FORWARD);

    assembler.remove_redundant_forward_jumps(true);
    let marker = assembler
        .graph
        .iter_items()
        .find_map(|item| match item {
            Item::Instruction(instruction) if instruction.jump_policy.borrow_empty_block() => {
                Some(instruction)
            }
            Item::Instruction(_) | Item::Label(_) => None,
        })
        .unwrap();
    assert_eq!(marker.opcode, NOP);

    let mut items = assembler.graph.into_items();
    super::OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
    assert_eq!(
        items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.opcode),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [LOAD_CONST, RETURN_VALUE]
    );
}

#[test]
fn uninitialized_load_analysis_follows_single_block_backedges() {
    let label = super::Label(0);
    let mut assembler = optimization_pipeline(vec![
        Item::Label(label),
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(DELETE_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_BACKWARD,
            Operand::Backward(label),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
    ]);

    assembler.add_checks_for_uninitialized_loads(1, 1);

    assert_eq!(
        graph_opcodes(&assembler),
        [LOAD_FAST_CHECK, DELETE_FAST, JUMP_BACKWARD]
    );
}

#[test]
fn uninitialized_load_analysis_resets_locals_above_the_fast_mask_at_block_entry() {
    let target = super::Label(0);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(STORE_FAST, 63, 1),
        pass_instruction(STORE_FAST, 64, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(target),
        pass_instruction(LOAD_FAST, 63, 2),
        pass_instruction(LOAD_FAST, 64, 2),
        pass_instruction(LOAD_FAST, 64, 2),
    ]);

    assembler.add_checks_for_uninitialized_loads(65, 0);

    assert_eq!(
        graph_opcodes(&assembler),
        [
            STORE_FAST,
            STORE_FAST,
            JUMP_FORWARD,
            LOAD_FAST,
            LOAD_FAST_CHECK,
            LOAD_FAST
        ]
    );
}

#[test]
fn assertion_uninitialized_load_analysis_preserves_low_and_high_local_semantics() {
    let target = super::Label(0);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(STORE_FAST, 63, 1),
        pass_instruction(STORE_FAST, 64, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(target),
        pass_instruction(LOAD_FAST_CHECK, 63, 2),
        pass_instruction(LOAD_FAST, 64, 2),
        pass_instruction(LOAD_FAST_CHECK, 64, 2),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);
    assembler.has_assertion_side_exit_candidate = true;
    assert!(assembler.has_only_assertion_side_exits());

    assembler.add_checks_for_assertion_side_exits(65, 0);

    assert_eq!(
        graph_opcodes(&assembler),
        [
            STORE_FAST,
            STORE_FAST,
            POP_JUMP_IF_TRUE,
            NOT_TAKEN,
            LOAD_COMMON_CONSTANT,
            RAISE_VARARGS,
            LOAD_FAST,
            LOAD_FAST_CHECK,
            LOAD_FAST,
            RETURN_VALUE,
        ]
    );
}

#[test]
fn linear_assertion_optimizer_matches_the_block_graph_pipeline() {
    let target = super::Label(0);
    let items = vec![
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(TO_BOOL, 0, 1),
        pass_instruction(TO_BOOL, 0, 1),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_TRUE,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            Some(0),
        )),
        pass_instruction(NOT_TAKEN, 0, 1),
        pass_instruction(
            LOAD_COMMON_CONSTANT,
            CommonConstant::ASSERTIONERROR.argument(),
            1,
        ),
        pass_instruction(RAISE_VARARGS, 1, 1),
        Item::Label(target),
        pass_instruction(LOAD_CONST, 0, 2),
        pass_instruction(LOAD_CONST, 1, 2),
        pass_instruction(STORE_FAST, 1, 2),
        pass_instruction(STORE_FAST, 1, 2),
        pass_instruction(LOAD_FAST, 1, 3),
        pass_instruction(LOAD_FAST, 0, 3),
        pass_instruction(IS_OP, 0, 3),
        pass_instruction(RETURN_VALUE, 0, 3),
    ];
    let mut linear = Assembler::from_linear_items(items.clone());
    linear.next_label = 1;
    linear.has_assertion_side_exit_candidate = true;
    assert!(linear.has_only_linear_assertion_side_exits());
    let graph =
        super::block_graph::BlockGraph::from_items(items, &rustc_hash::FxHashSet::default());
    let reachable = graph.reachable_blocks(&[]);
    let generic = Assembler {
        code: UnoptimizedCode::Prepared { graph, reachable },
        next_label: 1,
        has_assertion_side_exit_candidate: true,
        ..Assembler::default()
    };
    assert!(!generic.has_only_linear_assertion_side_exits());

    let actual = linear.finish_code(1, 2, 1).unwrap();
    let expected = generic.finish_code(1, 2, 1).unwrap();

    assert_eq!(actual.bytecode, expected.bytecode);
    assert_eq!(actual.line_table, expected.line_table);
    assert_eq!(actual.exception_table, expected.exception_table);
    assert_eq!(actual.max_depth, expected.max_depth);
    assert_eq!(actual.removed_max_depth, expected.removed_max_depth);
}

#[test]
fn linear_constant_return_matches_the_block_graph_pipeline() {
    let resume = SourceLocation::new(0, 1, 0, 0);
    let pass = SourceLocation::new(2, 2, 4, 8);
    let same_line = SourceLocation::new(2, 2, 12, 16);
    let following_line = SourceLocation::new(3, 3, 4, 12);
    for (nop, load, ret, opcode, argument, retains_nop) in [
        (None, following_line, following_line, LOAD_CONST, 0, false),
        (
            Some(pass),
            SourceLocation::NONE,
            SourceLocation::NONE,
            LOAD_CONST,
            0,
            false,
        ),
        (
            Some(pass),
            same_line,
            SourceLocation::NONE,
            LOAD_SMALL_INT,
            42,
            false,
        ),
        (
            Some(pass),
            following_line,
            SourceLocation::NONE,
            LOAD_CONST,
            513,
            true,
        ),
    ] {
        let mut items = vec![Item::Instruction(Instruction::new(
            RESUME,
            Operand::Value(ResumeLocation::AT_FUNC_START.argument()),
            resume,
            Some(0),
        ))];
        if let Some(nop) = nop {
            items.push(Item::Instruction(Instruction::new(
                NOP,
                Operand::Value(0),
                nop,
                Some(0),
            )));
        }
        items.extend([
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                load,
                Some(1),
            )),
            Item::Instruction(Instruction::new(
                RETURN_VALUE,
                Operand::Value(0),
                ret,
                Some(0),
            )),
        ]);
        let linear = Assembler::from_linear_items(items.clone());
        assert!(linear.has_only_linear_constant_return());
        let graph =
            super::block_graph::BlockGraph::from_items(items, &rustc_hash::FxHashSet::default());
        let reachable = graph.reachable_blocks(&[]);
        let generic = Assembler {
            code: UnoptimizedCode::Prepared { graph, reachable },
            ..Assembler::default()
        };
        assert!(!generic.has_only_linear_constant_return());

        let actual = linear.finish_code(1, 70, 65).unwrap();
        let expected = generic.finish_code(1, 70, 65).unwrap();
        assert_eq!(actual.bytecode, expected.bytecode);
        assert_eq!(actual.line_table, expected.line_table);
        assert_eq!(actual.exception_table, expected.exception_table);
        assert_eq!(actual.max_depth, expected.max_depth);
        assert_eq!(actual.removed_max_depth, expected.removed_max_depth);
        assert_eq!(
            actual
                .bytecode
                .as_chunks::<2>()
                .0
                .iter()
                .any(|instruction| instruction[0] == NOP.code()),
            retains_nop
        );
    }
}

#[test]
fn linear_constant_return_rejects_nontrivial_instructions_and_metadata() {
    let original = || {
        vec![
            pass_instruction(RESUME, ResumeLocation::AT_FUNC_START.argument(), 1),
            pass_instruction(NOP, 0, 2),
            pass_instruction(LOAD_CONST, 0, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
        ]
    };
    for replacement in [
        pass_instruction(LOAD_FAST, 0, 2),
        pass_instruction(UNPACK_SEQUENCE, 1, 2),
        pass_instruction(NOP, 0, -1),
        Item::Label(super::Label(0)),
    ] {
        let mut items = original();
        items[1] = replacement;
        assert!(!Assembler::from_linear_items(items).has_only_linear_constant_return());
    }
    let mut items = original();
    let Item::Instruction(nop) = &mut items[1] else {
        unreachable!();
    };
    nop.jump_policy.preserve_explicit_nop();
    assert!(!Assembler::from_linear_items(items).has_only_linear_constant_return());

    let mut assembler = Assembler::from_linear_items(original());
    assembler.label();
    assert!(!assembler.has_only_linear_constant_return());
    let mut assembler = Assembler::from_linear_items(original());
    assembler.exception_owner_provenance.omitted_nop_tail =
        Some((1, SourceLocation::new(2, 2, 0, 0), 1));
    assert!(!assembler.has_only_linear_constant_return());
    let mut assembler = Assembler::from_linear_items(original());
    assembler.has_next_location = true;
    assert!(!assembler.has_only_linear_constant_return());
}

#[test]
fn maybe_null_comprehension_restore_checks_only_the_first_later_load() {
    let mut restore = Instruction::new(STORE_FAST, Operand::Value(0), SourceLocation::NONE, None);
    restore.local_policy.mark_maybe_null_comprehension_restore();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(restore),
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(LOAD_FAST, 0, 1),
    ]);

    assembler.add_checks_for_uninitialized_loads(1, 1);

    assert_eq!(
        graph_opcodes(&assembler),
        [STORE_FAST, LOAD_FAST_CHECK, LOAD_FAST]
    );
}

#[test]
fn local_rewrites_report_only_instruction_changes() {
    let mut stores = optimization_pipeline(vec![
        pass_instruction(STORE_FAST, 0, 1),
        pass_instruction(STORE_FAST, 1, 1),
        pass_instruction(STORE_FAST, 1, 2),
        pass_instruction(STORE_FAST, 1, 2),
    ]);
    assert!(stores.optimize_redundant_store_fast());
    let stored = stores.graph.copy_items();
    assert_eq!(
        graph_opcodes(&stores),
        [STORE_FAST, STORE_FAST, POP_TOP, STORE_FAST]
    );
    assert!(!stores.optimize_redundant_store_fast());
    assert_eq!(stores.graph.copy_items(), stored);

    let mut loads = optimization_pipeline(vec![
        pass_instruction(LOAD_FAST_CHECK, 0, 1),
        pass_instruction(LOAD_FAST_CHECK, 1, 1),
        pass_instruction(LOAD_FAST_CHECK, 0, 1),
        pass_instruction(DELETE_FAST, 0, 1),
        pass_instruction(LOAD_FAST_CHECK, 0, 1),
    ]);
    assert!(loads.remove_redundant_checked_loads());
    let checked = loads.graph.copy_items();
    assert_eq!(
        graph_opcodes(&loads),
        [
            LOAD_FAST_CHECK,
            LOAD_FAST_CHECK,
            LOAD_FAST,
            DELETE_FAST,
            LOAD_FAST_CHECK
        ]
    );
    assert!(!loads.remove_redundant_checked_loads());
    assert_eq!(loads.graph.copy_items(), checked);
}

#[test]
fn swap_runs_report_only_changed_slots() {
    for (depths, changed, opcodes) in [([2, 3], false, [SWAP, SWAP]), ([2, 2], true, [NOP, NOP])] {
        let mut assembler = optimization_pipeline(
            depths
                .into_iter()
                .map(|depth| pass_instruction(SWAP, depth, 1))
                .collect(),
        );
        let before = assembler.graph.copy_items();
        assert_eq!(assembler.optimize_swap_runs().2, changed);
        assert_eq!(graph_opcodes(&assembler), opcodes);
        if !changed {
            assert_eq!(assembler.graph.copy_items(), before);
        }
        let after = assembler.graph.copy_items();
        assert!(!assembler.optimize_swap_runs().2);
        assert_eq!(assembler.graph.copy_items(), after);
    }

    let swaps = |depths: [u32; 3]| {
        depths
            .into_iter()
            .map(|depth| pass_instruction(SWAP, depth, 1))
            .collect::<Vec<_>>()
    };
    let mut assembler = optimization_pipeline(swaps([3, 2, 3]));
    assert!(assembler.optimize_swap_runs().2);
    let expected = swaps([2, 3, 2]);
    assert_eq!(assembler.graph.copy_items(), expected);
    assert!(!assembler.optimize_swap_runs().2);
    assert_eq!(assembler.graph.copy_items(), expected);
}

#[test]
fn redundant_swap_removal_preserves_forward_scan_semantics() {
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(SWAP, 2, 1),
        Item::Label(super::Label(0)),
        pass_instruction(SWAP, 2, 1),
        pass_instruction(SWAP, 2, 1),
        pass_instruction(POP_TOP, 0, 1),
        Item::Label(super::Label(1)),
        pass_instruction(POP_TOP, 0, 1),
    ]);

    assert!(assembler.remove_redundant_swaps_before_pops());

    assert_eq!(graph_opcodes(&assembler), [SWAP, SWAP, POP_TOP, POP_TOP]);
}

#[test]
fn redundant_swap_removal_preserves_distinct_pop_lines() {
    for (first_pop_line, second_pop_line, retain_rotation) in
        [(5, 5, false), (5, 6, true), (-1, 6, false)]
    {
        let mut items = vec![pass_instruction(SWAP, 3, 4), pass_instruction(SWAP, 2, 4)];
        let pops_and_store = [
            pass_instruction(POP_TOP, 0, first_pop_line),
            pass_instruction(POP_TOP, 0, second_pop_line),
            pass_instruction(STORE_FAST, 1, 3),
        ];
        items.extend(pops_and_store.iter().copied());
        let mut assembler = optimization_pipeline(items);

        assert_eq!(
            assembler.remove_redundant_swaps_before_pops(),
            !retain_rotation
        );
        assert!(!assembler.remove_redundant_swaps_before_pops());

        let mut expected = vec![pass_instruction(SWAP, 3, 4)];
        if retain_rotation {
            expected.push(pass_instruction(SWAP, 2, 4));
        }
        expected.extend(pops_and_store);
        assert_eq!(
            assembler.graph.iter_items().copied().collect::<Vec<_>>(),
            expected,
            "pop lines {first_pop_line} and {second_pop_line}"
        );
    }
}

#[test]
fn redundant_nop_removal_preserves_forward_scan_semantics() {
    let mut converted = match pass_instruction(NOP, 0, 1) {
        Item::Instruction(instruction) => instruction,
        Item::Label(_) => unreachable!(),
    };
    converted.fusion_policy.mark_converted_pop_block();
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(NOP, 0, 1),
        Item::Label(super::Label(0)),
        pass_instruction(NOP, 0, -1),
        pass_instruction(LOAD_CONST, 0, 1),
        pass_instruction(NOP, 0, -1),
        Item::Instruction(converted),
        pass_instruction(RETURN_VALUE, 0, 2),
    ]);

    assembler.remove_redundant_nops();

    assert_eq!(graph_opcodes(&assembler), [NOP, LOAD_CONST, RETURN_VALUE]);
}

#[test]
fn cold_block_terminal_restore_accounts_for_the_drained_range() {
    let source = r"async def f(iterator):
    async for item in iterator:
        if condition:
            await call()
            break
    assert result
";

    crate::compile(source, "cold_terminal_restore.py").unwrap();
}

#[test]
fn consecutive_pop_slot_width_respects_original_array_boundaries() {
    use super::{ExceptionRegion, ExceptionRegionPriority, Label};

    let converted = || {
        let mut instruction = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
        instruction.fusion_policy.mark_converted_pop_block();
        Item::Instruction(instruction)
    };
    let setup = Label(0);
    let end = Label(1);
    let interior = Label(2);
    let handler = Label(3);
    let location = SourceLocation::new(1, 1, 0, 0);
    for (case, expected_width) in [
        ("contiguous", Some(2)),
        ("bookkeeping_alias", Some(2)),
        ("end_inside_run", Some(2)),
        ("jump_into_first", None),
        ("jump_into_interior", None),
        ("block_boundary_first", None),
        ("block_boundary_interior", None),
        ("removed_source_nop", None),
        ("hidden_pop_record", None),
        ("setup_nop_inside_run", None),
        ("count_does_not_fit", None),
    ] {
        let mut items = vec![
            pass_instruction(NOP, 0, 1),
            Item::Label(setup),
            pass_instruction(LOAD_CONST, 0, 2),
        ];
        if case.starts_with("jump_into_") {
            items.push(Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(if case == "jump_into_first" {
                    end
                } else {
                    interior
                }),
                location,
                None,
            )));
        }
        if case == "removed_source_nop" {
            items.push(pass_instruction(NOP, 0, -1));
        }
        if case != "end_inside_run" {
            items.push(Item::Label(end));
        }
        let mut first = converted();
        if case == "hidden_pop_record" {
            let Item::Instruction(instruction) = &mut first else {
                unreachable!()
            };
            instruction.local_policy.add_hidden_exception_owner_slots(1);
        }
        items.push(first);
        if case == "end_inside_run" {
            items.push(Item::Label(end));
        }
        items.push(Item::Label(interior));
        if case == "bookkeeping_alias" {
            items.push(Item::Label(Label(4)));
            items.push(Item::Label(Label(5)));
        }
        let count = if case == "count_does_not_fit" {
            usize::from(LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS)
        } else {
            1
        };
        items.extend((0..count).map(|_| converted()));
        items.extend([
            pass_instruction(LOAD_FAST, 0, 3),
            Item::Label(handler),
            pass_instruction(RETURN_VALUE, 0, 4),
        ]);
        let mut assembler = optimization_pipeline(items);
        assembler.exception_regions.push(ExceptionRegion {
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(setup, end),
            start: setup,
            end,
            target: handler,
            depth: 0,
            preserve_lasti: false,
        });
        assembler
            .exception_owner_provenance
            .setups
            .push((setup, Some(location)));
        if case == "setup_nop_inside_run" {
            assembler
                .exception_owner_provenance
                .setups
                .push((interior, Some(SourceLocation::NONE)));
        }
        if case.starts_with("block_boundary_") {
            assembler
                .exception_owner_provenance
                .block_boundaries
                .insert(if case == "block_boundary_first" {
                    end
                } else {
                    interior
                });
        }
        assembler.remove_redundant_nops();
        assert_eq!(
            assembler
                .exception_owner_provenance
                .removed_pop_block_widths
                .get(&end)
                .copied(),
            expected_width,
            "{case}",
        );
    }
}

#[test]
fn duplicated_exit_blocks_retain_transitively_targeted_blocks() {
    let source = r"def f():
    try:
        result = first()
    except Error as outer:
        try:
            result = second()
        except Error as inner:
            pass
    return result
";

    crate::compile(source, "nested_exit_copy.py").unwrap();
}

#[test]
fn duplicated_finally_exits_keep_internal_jump_labels_local() {
    for prefix in ["", "async "] {
        for condition in ["flag", "not flag"] {
            let source = format!(
                "{prefix}def function(flag):\n    try:\n        pass\n    except Exception as error:\n        return None\n    finally:\n        try:\n            if {condition}:\n                cleanup()\n        except Exception:\n            pass\n",
            );
            // Shared exits can already contain an inlined return when conditional
            // predecessors duplicate them. Their internal jumps must use copied labels.
            crate::compile(&source, "copied_finally_internal_jump.py").unwrap();
        }
    }
}

#[test]
fn empty_exception_regions_follow_their_original_entry() {
    for reachable_entry in [false, true] {
        let start = super::Label(0);
        let end = super::Label(1);
        let handler = super::Label(2);
        let mut items = Vec::new();
        if !reachable_entry {
            items.extend([
                pass_instruction(LOAD_CONST, 0, 1),
                pass_instruction(RETURN_VALUE, 0, 1),
            ]);
        }
        items.extend([
            Item::Label(start),
            Item::Label(end),
            pass_instruction(LOAD_CONST, 0, 2),
            pass_instruction(RETURN_VALUE, 0, 2),
            Item::Label(handler),
            pass_instruction(RERAISE, 0, -1),
        ]);
        let mut assembler = optimization_pipeline(items);
        assembler.exception_regions.push(super::ExceptionRegion {
            priority: super::ExceptionRegionPriority::OriginalRange(start, end),
            is_scope_fallback: false,
            start,
            end,
            target: handler,
            depth: 0,
            preserve_lasti: false,
        });
        let handler = assembler.graph.label_block(handler).unwrap();
        let reachable = assembler
            .graph
            .reachable_blocks(&assembler.exception_regions);
        assert_eq!(reachable[handler.index()], reachable_entry);
    }
}

#[test]
fn copied_return_pop_origins_retire_only_with_erased_unreachable_arrays() {
    for reachable in [false, true] {
        let mut assembler = Assembler::default();
        let origin = assembler.label();
        let exit = assembler.label();
        assembler.emit_forward_with_depth(
            JUMP_FORWARD,
            if reachable { origin } else { exit },
            0,
            ExceptionPolicy::default(),
        );
        assembler.record_exception_owner_block_boundary(origin);
        assembler.mark(origin);
        let first_pop_item = assembler.item_count();
        for _ in 0..2 {
            assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
            assembler.mark_last_as_converted_pop_block();
        }
        assembler.set_location(SourceLocation::new(2, 2, 0, 4));
        assembler.emit(LOAD_CONST, 1);
        assembler.record_copied_return_pop_prefix(first_pop_item);
        assembler.emit(POP_TOP, 0);
        assembler.emit_forward_with_depth(JUMP_FORWARD, exit, 0, ExceptionPolicy::default());
        assembler.mark(exit);
        assembler.emit(LOAD_CONST, 0);
        assembler.emit(RETURN_VALUE, 0);
        assert_eq!(
            assembler
                .exception_owner_provenance
                .copied_return_pop_prefixes
                .len(),
            1
        );
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.remove_unreachable_instructions();
        assert_eq!(
            pipeline
                .exception_owner_provenance
                .copied_return_pop_prefixes
                .len(),
            usize::from(reachable),
            "reachable={reachable}",
        );
        if reachable {
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .copied_return_pop_prefixes[0]
                    .entry,
                origin,
            );
        }
    }
}

#[test]
fn no_location_block_inlining_reaches_a_fixed_point() {
    let restart = super::Label(0);
    let source = super::Label(1);
    let first_cleanup = super::Label(2);
    let second_cleanup = super::Label(3);
    let mut first_cleanup_jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(second_cleanup),
        SourceLocation::NONE,
        None,
    );
    first_cleanup_jump.jump_policy.prevent_threading_target();
    first_cleanup_jump.no_location_policy.allow_block_inlining();
    let mut second_cleanup_jump = Instruction::new(
        JUMP_BACKWARD_NO_INTERRUPT,
        Operand::Backward(restart),
        SourceLocation::NONE,
        None,
    );
    second_cleanup_jump.jump_policy.prevent_threading_target();
    second_cleanup_jump
        .no_location_policy
        .allow_block_inlining();
    let mut assembler = optimization_pipeline(vec![
        Item::Label(restart),
        pass_instruction(NOP, 0, 0),
        Item::Label(source),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(first_cleanup),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(first_cleanup),
        pass_instruction(POP_EXCEPT, 0, -1),
        Item::Instruction(first_cleanup_jump),
        Item::Label(second_cleanup),
        pass_instruction(POP_TOP, 0, -1),
        Item::Instruction(second_cleanup_jump),
    ]);

    // Track only origin lifecycle here; existing assertions below verify the copied CFG.
    // Source is replaced by inlined records, both cleanup originals disappear, and restart survives.
    let records = [
        Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None),
        Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None),
        Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None),
    ];
    assembler
        .exception_owner_provenance
        .copied_return_pop_prefixes = [restart, source, first_cleanup, second_cleanup]
        .into_iter()
        .map(|entry| super::exception_owners::CopiedReturnPopPrefix { entry, records })
        .collect();

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());
    assembler.propagate_locations_within_blocks();

    assert_eq!(
        assembler
            .exception_owner_provenance
            .copied_return_pop_prefixes
            .iter()
            .map(|origin| origin.entry)
            .collect::<Vec<_>>(),
        [restart],
    );

    assert_eq!(
        graph_opcodes(&assembler),
        [
            NOP,
            LOAD_CONST,
            POP_EXCEPT,
            POP_TOP,
            JUMP_BACKWARD_NO_INTERRUPT
        ]
    );
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location.line),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [0, 1, 1, 1, 1]
    );
}

#[test]
fn shared_inlined_cleanup_locations_distinguish_exit_and_continuation() {
    for terminal in [JUMP_BACKWARD_NO_INTERRUPT, RERAISE, JUMP_BACKWARD] {
        let restart = super::Label(0);
        let condition = super::Label(1);
        let body = super::Label(2);
        let first_cleanup = super::Label(3);
        let second_cleanup = super::Label(4);
        let mut first_jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(second_cleanup),
            SourceLocation::NONE,
            None,
        );
        first_jump.jump_policy.prevent_threading_target();
        first_jump.no_location_policy.allow_block_inlining();
        let mut last = Instruction::new(
            terminal,
            if terminal == RERAISE {
                Operand::Value(0)
            } else {
                Operand::Backward(restart)
            },
            SourceLocation::NONE,
            None,
        );
        last.jump_policy.prevent_threading_target();
        last.no_location_policy.allow_block_inlining();
        let mut assembler = optimization_pipeline(vec![
            Item::Label(restart),
            pass_instruction(NOP, 0, 0),
            Item::Label(condition),
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(Instruction::new(
                POP_JUMP_IF_FALSE,
                Operand::Forward(first_cleanup),
                SourceLocation::new(1, 1, 0, 0),
                None,
            )),
            Item::Label(body),
            pass_instruction(LOAD_CONST, 0, 2),
            Item::Label(first_cleanup),
            pass_instruction(POP_EXCEPT, 0, -1),
            Item::Instruction(first_jump),
            Item::Label(second_cleanup),
            pass_instruction(POP_EXCEPT, 0, -1),
            Item::Instruction(last),
        ]);

        let original_location_absence = assembler.capture_original_location_absence();
        assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());
        assembler.propagate_locations_within_blocks();

        let cleanups = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) if instruction.opcode == POP_EXCEPT => {
                    Some(instruction.location.line)
                }
                Item::Instruction(_) | Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert!(!cleanups.is_empty());
        assert_eq!(cleanups.len() % 2, 0);
        let mut outer_locations = cleanups
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| pair[1])
            .collect::<Vec<_>>();
        outer_locations.sort_unstable();
        // Native source resolution duplicates exits and eval checks. A normal
        // continuation keeps its original shared, unlocated cleanup boundary.
        if terminal == JUMP_BACKWARD_NO_INTERRUPT {
            assert!(outer_locations.iter().all(|line| *line < 0));
        } else {
            assert_eq!(outer_locations, [1, 2]);
        }
    }
}

#[test]
fn no_location_block_inlining_crosses_exception_splits() {
    let restart = super::Label(0);
    let source = super::Label(1);
    let cleanup = super::Label(2);
    let outer_unwind = super::Label(3);
    let handler = super::Label(4);
    let mut cleanup_jump = Instruction::new(
        JUMP_BACKWARD_NO_INTERRUPT,
        Operand::Backward(restart),
        SourceLocation::NONE,
        None,
    );
    cleanup_jump.jump_policy.prevent_threading_target();
    cleanup_jump.no_location_policy.allow_block_inlining();
    let mut assembler = optimization_pipeline(vec![
        Item::Label(restart),
        pass_instruction(NOP, 0, 0),
        Item::Label(source),
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(cleanup),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(cleanup),
        pass_instruction(POP_EXCEPT, 0, -1),
        Item::Label(outer_unwind),
        pass_instruction(POP_EXCEPT, 0, -1),
        Item::Instruction(cleanup_jump),
        Item::Label(handler),
        pass_instruction(RERAISE, 0, -1),
    ]);
    assembler.exception_regions.push(super::ExceptionRegion {
        priority: super::ExceptionRegionPriority::OriginalRange(source, outer_unwind),
        is_scope_fallback: false,
        start: source,
        end: outer_unwind,
        target: handler,
        depth: 0,
        preserve_lasti: true,
    });

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());
    assembler.propagate_locations_within_blocks();

    assert_eq!(
        graph_opcodes(&assembler),
        [
            NOP,
            LOAD_CONST,
            POP_EXCEPT,
            POP_EXCEPT,
            JUMP_BACKWARD_NO_INTERRUPT,
            POP_EXCEPT,
            JUMP_BACKWARD_NO_INTERRUPT,
            RERAISE,
        ]
    );
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location.line),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [0, 1, 1, 1, 1, -1, -1, -1]
    );
    assert_eq!(assembler.exception_regions.len(), 2);
}

#[test]
fn unconditional_cleanup_inlining_keeps_the_conditional_target_shared() {
    let restart = super::Label(0);
    let conditional = super::Label(1);
    let normal = super::Label(2);
    let handler = super::Label(3);
    let cleanup = super::Label(4);
    let mut backedge = Instruction::new(
        JUMP_BACKWARD,
        Operand::Backward(restart),
        SourceLocation::NONE,
        None,
    );
    backedge.jump_policy.prevent_threading_target();
    backedge.allow_unconditional_block_inlining();
    let mut assembler = optimization_pipeline(vec![
        Item::Label(restart),
        pass_instruction(NOP, 0, 1),
        Item::Label(conditional),
        pass_instruction(LOAD_CONST, 0, 2),
        Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(cleanup),
            SourceLocation::new(2, 2, 0, 1),
            None,
        )),
        Item::Label(normal),
        pass_instruction(LOAD_CONST, 0, 3),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(cleanup),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(handler),
        pass_instruction(POP_EXCEPT, 0, 4),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(cleanup),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(cleanup),
        Item::Instruction(backedge),
    ]);

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());

    let branch = assembler.graph.label_block(conditional).unwrap();
    assert_eq!(
        assembler.graph[branch]
            .last_instruction()
            .unwrap()
            .operand(),
        Operand::Forward(cleanup)
    );
    for (start, end) in [(normal, handler), (handler, cleanup)] {
        let mut within_source = false;
        let mut found_backedge = false;
        for item in assembler.graph.iter_items() {
            match item {
                Item::Label(label) if *label == start => within_source = true,
                Item::Label(label) if *label == end => break,
                Item::Instruction(instruction) if within_source => {
                    found_backedge |= instruction.opcode == JUMP_BACKWARD
                        && instruction.operand() == Operand::Backward(restart);
                }
                _ => {}
            }
        }
        assert!(found_backedge);
    }
}

#[test]
fn direct_target_return_copy_crosses_an_exception_boundary() {
    let return_start = super::Label(0);
    let return_split = super::Label(1);
    let handler = super::Label(2);
    let mut jump = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(return_start),
        SourceLocation::new(1, 1, 0, 0),
        None,
    );
    jump.jump_policy.retain_nop_for_direct_target();
    let mut assembler = optimization_pipeline(vec![
        Item::Instruction(jump),
        Item::Label(return_start),
        pass_instruction(LOAD_CONST, 0, 2),
        Item::Label(return_split),
        pass_instruction(RETURN_VALUE, 0, 2),
        Item::Label(handler),
        pass_instruction(RERAISE, 0, -1),
    ]);
    assembler.exception_regions.push(super::ExceptionRegion {
        priority: super::ExceptionRegionPriority::OriginalRange(return_start, return_split),
        is_scope_fallback: false,
        start: return_start,
        end: return_split,
        target: handler,
        depth: 0,
        preserve_lasti: false,
    });

    let original_location_absence = assembler.capture_original_location_absence();
    assembler.duplicate_exit_blocks(&original_location_absence, &mut Vec::new());

    assert_eq!(
        graph_opcodes(&assembler),
        [NOP, LOAD_CONST, RETURN_VALUE, RETURN_VALUE, RERAISE]
    );
    assert_eq!(
        assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location.line),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>(),
        [1, 2, 2, 2, -1]
    );
}

#[test]
fn retained_setup_nop_identity_survives_consecutive_labels_without_aliasing_removed_setups() {
    let location = SourceLocation::new(2, 9, 4, 16);
    let start = super::Label(0);
    let end = super::Label(1);
    let intermediate = super::Label(2);
    let setup = super::Label(3);
    let nop = Item::Instruction(Instruction::new(NOP, Operand::Value(0), location, None));
    for retained in [false, true] {
        // Both layouts have the same NOP location followed by consecutive labels.
        // Only the first contains that instruction inside the authored SETUP range.
        let items = if retained {
            vec![
                Item::Label(start),
                nop,
                Item::Label(end),
                Item::Label(intermediate),
                Item::Label(setup),
            ]
        } else {
            vec![
                nop,
                Item::Label(start),
                Item::Label(end),
                Item::Label(intermediate),
                Item::Label(setup),
            ]
        };
        let mut label_positions = rustc_hash::FxHashMap::default();
        let mut position = 0;
        for item in &items {
            match item {
                Item::Instruction(_) => position += 1,
                Item::Label(label) => {
                    label_positions.insert(*label, position);
                }
            }
        }
        let assembler = super::LayoutAssembler {
            python_version: crate::PythonVersion::Py314,
            items,
            exception_regions: Vec::new(),
            exception_owner_provenance: super::exception_owners::ExceptionOwnerProvenance {
                setups: vec![(setup, Some(location))],
                setup_nop_ranges: vec![(start, end)],
                setup_nop_boundaries: [start, end].into_iter().collect(),
                ..Default::default()
            },
            preserved_block_boundaries: rustc_hash::FxHashSet::default(),
            removed_max_depth: None,
            next_label: 4,
            has_next_location: false,
        };
        let positions = assembler.retained_setup_nops(&label_positions);
        assert_eq!(positions.contains(&0), retained);
        assert_eq!(positions.len(), usize::from(retained));
    }
}

#[test]
fn emitted_region_requires_a_reachable_final_fallthrough_edge() {
    // Reaching the trailing join through a jump does not make the return fall through.
    let mut assembler = Assembler::default();
    let start = assembler.emitted_region_start();
    let join = assembler.label();
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(join));
    assembler.emit(RETURN_VALUE, 0);
    assembler.mark(join);
    assert!(!assembler.emitted_region_has_final_fallthrough(start));

    // An instruction emitted after a return is not a normal predecessor either.
    let mut assembler = Assembler::default();
    let start = assembler.emitted_region_start();
    assembler.emit(RETURN_VALUE, 0);
    assembler.emit(STORE_FAST, 0);
    assert!(!assembler.emitted_region_has_final_fallthrough(start));

    // Ordinary flow continues through label-only blocks, unlike borrow traversal.
    let mut assembler = Assembler::default();
    let start = assembler.emitted_region_start();
    let join = assembler.label();
    let alias = assembler.label();
    assembler.emit_operand(JUMP_FORWARD, Operand::Forward(join));
    assembler.emit(RETURN_VALUE, 0);
    assembler.mark(join);
    assembler.mark(alias);
    assembler.emit(STORE_FAST, 0);
    assert!(assembler.emitted_region_has_final_fallthrough(start));
}

#[test]
fn emitted_region_follows_runtime_branches_but_not_monitoring_targets() {
    for opcode in [POP_JUMP_IF_FALSE, FOR_ITER, SEND, END_ASYNC_FOR] {
        let mut assembler = Assembler::default();
        let start = assembler.emitted_region_start();
        let join = assembler.label();
        assembler.emit_operand(opcode, Operand::Forward(join));
        assembler.emit(RETURN_VALUE, 0);
        assembler.mark(join);
        assembler.emit(STORE_FAST, 0);
        assert_eq!(
            assembler.emitted_region_has_final_fallthrough(start),
            opcode != END_ASYNC_FOR
        );
    }
}

#[test]
fn emitted_region_boundaries_survive_label_insertion_but_not_instruction_removal() {
    let mut assembler = Assembler::default();
    assembler.emit(RETURN_VALUE, 0);
    let start = assembler.emitted_region_start();
    assembler.emit(STORE_FAST, 0);
    let label = assembler.label();
    assembler.mark_before_trailing_instructions(label, 2);
    assert!(assembler.emitted_region_has_final_fallthrough(start));

    assembler.emit(NOP, 0);
    assert!(assembler.take_trailing_nop_location().is_some());
    assert!(!assembler.emitted_region_has_final_fallthrough(start));
    assert!(!Assembler::default().emitted_region_has_final_fallthrough(start));
}

#[test]
fn empty_emitted_regions_fall_through_without_reusing_a_previous_terminator() {
    let mut assembler = Assembler::default();
    assembler.emit(RETURN_VALUE, 0);
    let start = assembler.emitted_region_start();
    assert!(assembler.emitted_region_has_final_fallthrough(start));

    // Labels and omitted no-op provenance do not introduce a terminal edge.
    let label = assembler.label();
    assembler.mark(label);
    assembler.record_omitted_nop_targets(assembler.location);
    assert!(assembler.emitted_region_has_final_fallthrough(start));

    assembler.emit(RETURN_VALUE, 0);
    assert!(!assembler.emitted_region_has_final_fallthrough(start));
}

#[test]
fn unpack_rotation_nop_removal_does_not_expose_capture_store_cascades() {
    let cases = [
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 3, 1),
                pass_instruction(NOP, 0, 1),
                pass_instruction(SWAP, 3, 2),
            ],
            vec![UNPACK_SEQUENCE, SWAP],
        ),
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 3, 1),
                pass_instruction(NOP, 0, 2),
                pass_instruction(SWAP, 3, 3),
            ],
            vec![UNPACK_SEQUENCE, NOP, SWAP],
        ),
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 1, 1),
                pass_instruction(NOP, 0, 1),
                pass_instruction(NOP, 0, 2),
                pass_instruction(STORE_FAST_STORE_FAST, 1, 3),
            ],
            vec![UNPACK_SEQUENCE, NOP, STORE_FAST_STORE_FAST],
        ),
        (
            vec![
                pass_instruction(UNPACK_SEQUENCE, 1, 1),
                pass_instruction(NOP, 0, 2),
                pass_instruction(STORE_FAST, 0, 3),
            ],
            vec![UNPACK_SEQUENCE, NOP, STORE_FAST],
        ),
    ];
    for (items, expected) in cases {
        let mut assembler = optimization_pipeline(items);
        assembler.remove_redundant_nops();
        assert_eq!(graph_opcodes(&assembler), expected);
    }
}

#[test]
fn canceled_unpack_rotations_preserve_distinct_capture_nop_lines() {
    let cases = [
        (
            vec![
                (UNPACK_SEQUENCE, 5),
                (NOP, 5),
                (NOP, 5),
                (NOP, 6),
                (NOP, 6),
                (STORE_FAST, 3),
            ],
            vec![(UNPACK_SEQUENCE, 5), (NOP, 6), (STORE_FAST, 3)],
        ),
        (
            vec![(UNPACK_SEQUENCE, 5), (NOP, 5), (NOP, 6), (STORE_FAST, 6)],
            vec![(UNPACK_SEQUENCE, 5), (STORE_FAST, 6)],
        ),
        (
            vec![(UNPACK_SEQUENCE, 5), (NOP, 6), (NOP, 7), (STORE_FAST, 3)],
            vec![(UNPACK_SEQUENCE, 5), (NOP, 6), (NOP, 7), (STORE_FAST, 3)],
        ),
        (
            vec![
                (UNPACK_SEQUENCE, 5),
                (NOP, 5),
                (NOP, 6),
                (UNPACK_SEQUENCE, 7),
                (NOP, 8),
                (STORE_FAST, 9),
            ],
            vec![
                (UNPACK_SEQUENCE, 5),
                (NOP, 6),
                (UNPACK_SEQUENCE, 7),
                (NOP, 8),
                (STORE_FAST, 9),
            ],
        ),
    ];
    for (input, expected) in cases {
        let items = input
            .into_iter()
            .map(|(opcode, line)| pass_instruction(opcode, 0, line))
            .collect();
        let mut assembler = optimization_pipeline(items);
        assembler.remove_redundant_nops();
        let actual = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => {
                    Some((instruction.opcode, instruction.location.line))
                }
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}

#[test]
fn cleanup_prefix_locations_retain_original_donors_through_late_writers() {
    for full_copy_path in [false, true] {
        for original_donor in [false, true] {
            for deferred_location in [0, 1, 2] {
                let [first, second, intermediate, cleanup, suffix, exit, handler] =
                    std::array::from_fn(|index| super::Label(u32::try_from(index).unwrap()));
                let jump = |opcode, target, line| {
                    Item::Instruction(Instruction::new(
                        opcode,
                        Operand::Forward(target),
                        SourceLocation::new(line, line, 0, 0),
                        None,
                    ))
                };
                let mut items = vec![
                    pass_instruction(LOAD_CONST, 0, 12),
                    pass_instruction(LOAD_CONST, 1, 12),
                    pass_instruction(LOAD_FAST, 0, 12),
                    jump(POP_JUMP_IF_TRUE, second, 12),
                    Item::Label(first),
                    pass_instruction(NOP, 0, 12),
                    jump(JUMP_FORWARD, intermediate, 12),
                    Item::Label(second),
                    pass_instruction(NOP, 0, 12),
                    jump(JUMP_FORWARD, intermediate, 12),
                    Item::Label(intermediate),
                    jump(JUMP_FORWARD, cleanup, if original_donor { 12 } else { -1 }),
                    Item::Label(cleanup),
                    pass_instruction(SWAP, 2, -1),
                    pass_instruction(POP_TOP, 0, -1),
                    Item::Label(suffix),
                    pass_instruction(NOP, 0, 2),
                    pass_instruction(NOP, 0, 2),
                    pass_instruction(NOP, 0, 2),
                ];
                if full_copy_path {
                    items.extend([
                        jump(JUMP_FORWARD, exit, 2),
                        Item::Label(exit),
                        pass_instruction(RETURN_VALUE, 0, 2),
                    ]);
                } else {
                    items.push(pass_instruction(RETURN_VALUE, 0, 2));
                }
                items.extend([Item::Label(handler), pass_instruction(RERAISE, 0, -1)]);
                let mut assembler = optimization_pipeline(items);
                if deferred_location != 0 {
                    let block = assembler.graph.label_block(cleanup).unwrap();
                    let pop = assembler
                        .graph
                        .instructions_mut(block)
                        .find(|instruction| instruction.opcode == POP_TOP)
                        .unwrap();
                    if deferred_location == 1 {
                        pop.fusion_policy.mark_location_from_next();
                    } else {
                        pop.no_location_policy.percent_target_location = 1;
                    }
                }
                assembler.exception_regions.push(super::ExceptionRegion {
                    priority: super::ExceptionRegionPriority::OriginalRange(cleanup, suffix),
                    is_scope_fallback: false,
                    start: cleanup,
                    end: suffix,
                    target: handler,
                    depth: 0,
                    preserve_lasti: false,
                });
                let original = assembler.capture_original_location_absence();
                assert_eq!(original.contains(&cleanup), !original_donor);
                assert!(assembler.thread_forward_jumps());
                assembler.remove_unreachable_instructions();
                let target = assembler.graph.label_block(cleanup).unwrap();
                let suffix_block = assembler.graph.label_block(suffix).unwrap();
                assert_ne!(target, suffix_block);
                let incoming = assembler
                    .graph
                    .iter_items()
                    .filter(|item| {
                        matches!(item, Item::Instruction(instruction)
                    if instruction.operand() == Operand::Forward(cleanup))
                    })
                    .count();
                assert_eq!(incoming, 2);
                // Reconstructing this certificate now would lose the original located predecessor.
                assert!(
                    assembler
                        .capture_original_location_absence()
                        .contains(&cleanup)
                );
                let copies = assembler.duplicate_exit_blocks(&original, &mut Vec::new());
                assert_eq!(copies.is_empty(), !full_copy_path);
                assembler.remove_redundant_forward_jumps(true);
                assembler.propagate_locations_within_blocks();
                let target = assembler.graph.label_block(cleanup).unwrap();
                let locations = assembler.graph[target]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.location.line),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    locations,
                    vec![
                        if original_donor || deferred_location != 0 {
                            12
                        } else {
                            -1
                        };
                        2
                    ]
                );
                let suffix_block = assembler.graph.label_block(suffix).unwrap();
                assert_eq!(
                    assembler.graph[suffix_block]
                        .items()
                        .iter()
                        .find_map(|item| match item {
                            Item::Instruction(instruction) => Some(instruction.location.line),
                            Item::Label(_) => None,
                        })
                        .unwrap(),
                    2
                );
            }
        }
    }
}

#[test]
fn uninitialized_load_analysis_preserves_original_exception_region_priority() {
    for fallback in [false, true] {
        for resolved in [false, true] {
            for cleared in [false, true] {
                let outer = super::Label(0);
                let inner = super::Label(1);
                let fragment = super::Label(2);
                let end = super::Label(3);
                let handler = super::Label(4);
                let other_handler = super::Label(5);
                let mut instructions = vec![
                    Item::Label(outer),
                    pass_instruction(LOAD_CONST, 0, 1),
                    pass_instruction(STORE_FAST, 0, 1),
                    Item::Label(inner),
                    pass_instruction(NOP, 0, 2),
                    Item::Label(fragment),
                ];
                if cleared {
                    instructions.push(pass_instruction(DELETE_FAST, 0, 3));
                }
                instructions.extend([
                    pass_instruction(LOAD_CONST, 0, 4),
                    pass_instruction(POP_TOP, 0, 4),
                    Item::Label(end),
                    pass_instruction(LOAD_CONST, 0, 5),
                    pass_instruction(RETURN_VALUE, 0, 5),
                    Item::Label(handler),
                    pass_instruction(LOAD_FAST, 0, 6),
                    pass_instruction(POP_TOP, 0, 6),
                    pass_instruction(RERAISE, 0, -1),
                    Item::Label(other_handler),
                    pass_instruction(RERAISE, 0, -1),
                ]);
                let mut assembler = optimization_pipeline(instructions);
                let priority = |start: super::Label| {
                    if resolved {
                        super::ExceptionRegionPriority::Resolved {
                            start: usize::try_from(start.0).unwrap(),
                            end: 20,
                        }
                    } else {
                        super::ExceptionRegionPriority::OriginalRange(start, end)
                    }
                };
                assembler.exception_regions.extend([
                    super::ExceptionRegion {
                        start: inner,
                        end,
                        target: handler,
                        depth: 0,
                        preserve_lasti: false,
                        is_scope_fallback: false,
                        priority: priority(inner),
                    },
                    super::ExceptionRegion {
                        start: fragment,
                        end,
                        target: other_handler,
                        depth: 0,
                        preserve_lasti: false,
                        is_scope_fallback: fallback,
                        priority: priority(if fallback { fragment } else { outer }),
                    },
                ]);
                assembler.add_checks_for_uninitialized_loads(1, 0);
                let block = assembler.graph.label_block(handler).unwrap();
                let load = assembler.graph[block]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction.opcode),
                        Item::Label(_) => None,
                    })
                    .unwrap();
                assert_eq!(load, if cleared { LOAD_FAST_CHECK } else { LOAD_FAST });
            }
        }
    }
}

#[test]
fn late_redundant_jump_removal_revisits_newly_adjacent_targets() {
    let intermediate = super::Label(0);
    let target = super::Label(1);
    let location = SourceLocation::new(7, 7, 3, 9);
    let mut jump = Instruction::new(JUMP_FORWARD, Operand::Forward(target), location, None);
    jump.exclude_from_exception_table(ExceptionExclusion::IfExtended);
    jump.no_location_policy
        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(POP_TOP, 0, 7),
        Item::Instruction(jump),
        Item::Label(intermediate),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        )),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 8),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0].opcode, POP_TOP);
    assert_eq!(
        instructions[0].exception_policy().exclusion(),
        ExceptionExclusion::IfExtended
    );
    assert_eq!(
        instructions[0].no_location_policy.borrow_fallthrough(),
        BorrowFallthrough::Barrier
    );
    assert_eq!(instructions[1].opcode, RETURN_VALUE);
    assert_eq!(instructions[1].location.line, 8);
}

#[test]
fn late_jump_retries_preserve_deferred_and_borrowed_markers() {
    for policy in ["deferred", "borrowed", "retained"] {
        let intermediate = super::Label(0);
        let target = super::Label(1);
        let mut jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        );
        match policy {
            "deferred" => jump.jump_policy.defer_removal(),
            "borrowed" => jump.jump_policy.mark_borrow_empty_block(),
            "retained" => jump.jump_policy.retain_nop(),
            _ => unreachable!(),
        }
        let mut assembler = optimization_pipeline(vec![
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(jump),
            Item::Label(intermediate),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::NONE,
                None,
            )),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, 1),
        ]);

        assembler.remove_redundant_forward_jumps(true);

        let instructions = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(instructions.len(), 3, "{policy}");
        let marker = instructions[1];
        match policy {
            "deferred" => {
                assert_eq!(marker.opcode, JUMP_FORWARD);
                assert!(!marker.jump_policy.defers_redundant_removal());
            }
            "borrowed" => {
                assert_eq!(marker.opcode, NOP);
                assert!(marker.jump_policy.borrow_empty_block());
            }
            "retained" => {
                assert_eq!(marker.opcode, NOP);
                assert!(marker.jump_policy.always_retains_nop());
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn late_jump_retries_require_proven_empty_line_blocks() {
    for (middle_line, next_line, preserved, retains_jump) in [
        (-1, 8, false, false),
        (7, 8, false, true),
        (7, 7, false, false),
        (7, -1, false, true),
        (-1, 8, true, true),
    ] {
        let intermediate = super::Label(0);
        let target = super::Label(1);
        let mut middle = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            if middle_line < 0 {
                SourceLocation::NONE
            } else {
                SourceLocation::new(middle_line, middle_line, 3, 9)
            },
            None,
        );
        if preserved {
            middle.no_location_policy.preserve_location();
        }
        let mut assembler = optimization_pipeline(vec![
            pass_instruction(POP_TOP, 0, 7),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::new(7, 7, 3, 9),
                None,
            )),
            Item::Label(intermediate),
            Item::Instruction(middle),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, next_line),
        ]);
        assembler.remove_redundant_forward_jumps(true);
        let instructions = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(instructions.len(), if retains_jump { 3 } else { 2 });
        if retains_jump {
            assert_eq!(instructions[1].opcode, JUMP_FORWARD);
            assert_eq!(instructions[1].location, SourceLocation::new(7, 7, 3, 9));
            assert_eq!(instructions[1].operand(), Operand::Forward(target));
        }
    }
}

#[test]
fn shared_unlocated_jump_retries_require_original_conditional_predecessors() {
    for case in [
        "shared",
        "one",
        "aliases",
        "merged_arrays",
        "unreachable",
        "located_nop",
        "omitted_nop",
        "hidden_slot",
        "lost_predecessor",
    ] {
        let entry = super::Label(0);
        let target = super::Label(1);
        let alias = super::Label(2);
        let second = super::Label(3);
        let instruction = |opcode, target, line| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Forward(target),
                SourceLocation::new(line, line, 0, 1),
                None,
            ))
        };
        let mut items = Vec::new();
        if case == "unreachable" {
            items.push(instruction(JUMP_FORWARD, second, 1));
        }
        items.push(instruction(POP_JUMP_IF_FALSE, entry, 2));
        items.push(Item::Label(second));
        if case != "one" {
            items.push(instruction(
                POP_JUMP_IF_TRUE,
                if matches!(case, "aliases" | "merged_arrays") {
                    alias
                } else {
                    entry
                },
                3,
            ));
        }
        items.extend([
            pass_instruction(POP_TOP, 0, 7),
            instruction(JUMP_FORWARD, target, 7),
            Item::Label(entry),
        ]);
        if case == "merged_arrays" {
            let mut first_array = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::NONE,
                None,
            );
            first_array.no_location_policy.preserve_location();
            items.push(Item::Instruction(first_array));
        }
        items.push(Item::Label(alias));
        if case == "located_nop" {
            items.push(pass_instruction(NOP, 0, 9));
        }
        let mut middle = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::NONE,
            None,
        );
        middle.no_location_policy.preserve_location();
        if case == "hidden_slot" {
            middle.local_policy.add_hidden_exception_owner_slots(1);
        }
        items.extend([
            Item::Instruction(middle),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, 8),
        ]);
        let mut assembler = optimization_pipeline(items);
        if case == "omitted_nop" {
            assembler
                .exception_owner_provenance
                .omitted_nop_locations
                .insert(entry, SourceLocation::new(9, 9, 0, 1));
        }
        assembler.record_shared_unlocated_jump_entries();
        assert_eq!(
            assembler
                .original_shared_unlocated_jump_entries
                .contains_key(&entry),
            matches!(case, "shared" | "aliases" | "lost_predecessor"),
            "{case}"
        );
        // Later compaction can hide an authored NOP or merge labels. Neither creates
        // an original shared unlocated array; losing an edge also invalidates admission.
        let head = assembler.graph.label_block(entry).unwrap();
        assembler.graph.retain_items(
            head,
            |item| !matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP),
        );
        assembler.graph.for_each_instruction_mut(|instruction| {
            if instruction.opcode == POP_JUMP_IF_TRUE {
                if matches!(case, "aliases" | "merged_arrays") {
                    instruction.retarget(Operand::Forward(entry));
                } else if case == "lost_predecessor" {
                    instruction.retarget(Operand::Forward(target));
                }
            }
        });
        assembler.remove_redundant_forward_jumps(true);
        let kept_body_jump = assembler.graph.iter_items().any(|item| matches!(item,
            Item::Instruction(instruction) if instruction.opcode == JUMP_FORWARD && instruction.location.line == 7));
        assert_eq!(
            kept_body_jump,
            !matches!(case, "shared" | "aliases"),
            "{case}"
        );
    }
}

#[test]
fn folded_borrow_entries_resolve_before_graph_preparation() {
    let mut assembler = Assembler::default();
    let other = assembler.label();
    let target = assembler.label();
    assembler.emit(LOAD_CONST, 0);
    assembler.emit_forward_with_depth(POP_JUMP_IF_FALSE, other, 0, ExceptionPolicy::default());
    assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.record_folded_condition_borrow_entry(target);
    // A second branch emitted in the dead remainder must not replace the captured
    // predecessor set. The earlier independent destination is in a different block.
    assembler.emit_forward_with_depth(JUMP_FORWARD, target, 0, ExceptionPolicy::default());
    assembler.mark(target);
    assembler.emit(LOAD_CONST, 1);
    assembler.emit(RETURN_VALUE, 0);
    assembler.mark(other);
    assembler.emit(LOAD_CONST, 2);
    assembler.emit(RETURN_VALUE, 0);

    assembler.used_constant_indices(LOAD_CONST, 3);
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
    assert!(assembler.deferred_borrow_entries.is_empty());
    assert!(assembler.borrow_unreachable_blocks.contains(&target));
    assert!(assembler.borrow_jump_entry_labels.contains(&target));
    // A second consumer must not reinterpret the already prepared graph's offsets.
    assembler.used_constant_indices(LOAD_CONST, 3);
    let pipeline = assembler.into_optimization_pipeline();
    assert!(pipeline.borrow_jump_entry_labels.contains(&target));
}

#[test]
fn folded_borrow_entries_preserve_a_future_alias() {
    let mut assembler = Assembler::default();
    let outer = assembler.label();
    let inner = assembler.label();
    assembler.emit(LOAD_CONST, 0);
    assembler.emit_forward_with_depth(POP_JUMP_IF_TRUE, outer, 0, ExceptionPolicy::default());
    assembler.emit_forward_with_depth(JUMP_FORWARD, inner, 0, ExceptionPolicy::default());
    assembler.record_folded_condition_borrow_entry(inner);
    // The label names differ during emission, but their completed original offsets
    // establish one block entered by the earlier live outer condition.
    assembler.mark(inner);
    assembler.mark(outer);
    assembler.emit(LOAD_CONST, 1);
    assembler.emit(RETURN_VALUE, 0);

    assembler.optimize_constant_pops();
    assert!(assembler.deferred_borrow_entries.is_empty());
    assert!(!assembler.borrow_unreachable_blocks.contains(&inner));
    assert!(!assembler.borrow_jump_entry_labels.contains(&inner));
    let pipeline = assembler.into_optimization_pipeline();
    assert!(!pipeline.borrow_jump_entry_labels.contains(&inner));
}

#[test]
fn copied_regions_retarget_only_handlers_copied_with_instructions() {
    for fallback in [false, true] {
        for shape in ["external", "internal", "other region end"] {
            let start = super::Label(0);
            let end = super::Label(1);
            let handler = super::Label(2);
            let outer_handler = super::Label(3);
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(start),
                SourceLocation::new(1, 1, 0, 0),
                None,
            );
            jump.jump_policy.retain_nop_for_direct_target();
            let mut items = vec![
                Item::Instruction(jump),
                Item::Label(start),
                pass_instruction(LOAD_CONST, 0, 2),
            ];
            if shape == "internal" {
                items.push(Item::Label(handler));
            }
            items.push(pass_instruction(RETURN_VALUE, 0, 2));
            if shape == "other region end" {
                items.extend([Item::Label(end), pass_instruction(RERAISE, 0, -1)]);
            }
            if shape != "internal" {
                items.extend([Item::Label(handler), pass_instruction(RERAISE, 0, -1)]);
            }
            items.extend([Item::Label(outer_handler), pass_instruction(RERAISE, 0, -1)]);
            let region_end = if shape == "other region end" {
                end
            } else {
                handler
            };
            let priority = super::ExceptionRegionPriority::OriginalRange(start, region_end);
            let mut assembler = optimization_pipeline(items);
            assembler.exception_regions.push(super::ExceptionRegion {
                priority,
                is_scope_fallback: fallback,
                start,
                end: region_end,
                target: handler,
                depth: 0,
                preserve_lasti: true,
            });
            if shape == "other region end" {
                // Another region's trailing end is also a handler label. Its boundary
                // alias must not redirect the first region's actual handler.
                assembler.exception_regions.push(super::ExceptionRegion {
                    priority: super::ExceptionRegionPriority::OriginalRange(start, handler),
                    is_scope_fallback: fallback,
                    start,
                    end: handler,
                    target: outer_handler,
                    depth: 1,
                    preserve_lasti: false,
                });
            }

            let original = assembler.capture_original_location_absence();
            assembler.duplicate_exit_blocks(&original, &mut Vec::new());

            let copied = assembler
                .exception_regions
                .iter()
                .find(|region| {
                    region.start != start
                        && matches!(
                            region.priority,
                            super::ExceptionRegionPriority::OriginalRange(first, last)
                                if first == start && last == region_end
                        )
                })
                .expect("the exit copy must retain its protected region");
            assert_ne!(copied.end, region_end, "{shape}/{fallback}");
            assert_eq!(copied.depth, 0, "{shape}/{fallback}");
            assert!(copied.preserve_lasti, "{shape}/{fallback}");
            assert_eq!(copied.is_scope_fallback, fallback, "{shape}/{fallback}");
            if shape == "internal" {
                assert_ne!(copied.target, handler, "{shape}/{fallback}");
                assert_eq!(copied.target, copied.end, "{shape}/{fallback}");
                let target = assembler.graph.label_block(copied.target).unwrap();
                assert_eq!(
                    assembler.graph[target].last_instruction().unwrap().opcode,
                    RETURN_VALUE,
                    "{shape}/{fallback}",
                );
            } else {
                assert_eq!(copied.target, handler, "{shape}/{fallback}");
                let target = assembler.graph.label_block(copied.target).unwrap();
                assert_eq!(
                    assembler.graph[target].last_instruction().unwrap().opcode,
                    RERAISE,
                    "{shape}/{fallback}",
                );
            }
            if shape == "other region end" {
                let other = assembler
                    .exception_regions
                    .iter()
                    .find(|region| region.start != start && region.depth == 1)
                    .expect("the second copied region must retain its handler");
                assert_eq!(other.target, outer_handler);
            }
        }
    }
}

#[test]
fn unlocated_borrow_marker_does_not_block_redundant_jump_removal() {
    for location in [SourceLocation::NONE, SourceLocation::new(2, 2, 0, 0)] {
        let target = super::Label(1);
        let mut marker = Instruction::new(NOP, Operand::Value(0), location, None);
        marker.jump_policy.mark_borrow_empty_block();
        let mut assembler = optimization_pipeline(vec![
            pass_instruction(LOAD_CONST, 0, 1),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::new(1, 1, 0, 0),
                None,
            )),
            Item::Label(super::Label(0)),
            Item::Instruction(marker),
            Item::Label(target),
            pass_instruction(RETURN_VALUE, 0, 3),
        ]);

        assembler.remove_redundant_forward_jumps(true);

        let instructions = assembler
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.opcode == JUMP_FORWARD)
                .count(),
            usize::from(location.line >= 0),
        );
        assert!(instructions.contains(&&marker));
    }
}

#[test]
fn borrow_marker_conversion_retries_without_another_jump_deletion() {
    let target = super::Label(1);
    let mut marker = Instruction::new(
        JUMP_FORWARD,
        Operand::Forward(target),
        SourceLocation::NONE,
        None,
    );
    marker.jump_policy.mark_borrow_empty_block();
    let mut assembler = optimization_pipeline(vec![
        pass_instruction(LOAD_CONST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(target),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
        Item::Label(super::Label(0)),
        Item::Instruction(marker),
        Item::Label(target),
        pass_instruction(RETURN_VALUE, 0, 3),
    ]);

    assembler.remove_redundant_forward_jumps(true);

    let instructions = assembler
        .graph
        .iter_items()
        .filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        instructions
            .iter()
            .map(|instruction| instruction.opcode)
            .collect::<Vec<_>>(),
        [LOAD_CONST, NOP, RETURN_VALUE]
    );
    assert!(instructions[1].jump_policy.borrow_empty_block());
    assert_eq!(instructions[1].location, SourceLocation::NONE);
    assert_eq!(instructions[1].operand(), Operand::Value(0));
}

#[test]
fn split_return_condition_transports_only_a_unique_exact_source_witness() {
    use super::Label;
    use super::exception_owners::SourceNoneCondition;

    let location = SourceLocation::new(13, 13, 15, 35);
    let mut condition = Instruction::new(
        POP_JUMP_IF_NOT_NONE,
        Operand::Forward(Label(3)),
        location,
        Some(2),
    );
    condition.local_policy.add_hidden_exception_owner_slots(3);
    let mut updated = condition;
    updated.no_location_policy.allow_split_return_exit_copy();
    let marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, Some(2));

    for change in [
        "unique",
        "no origin",
        "duplicate origin",
        "original clone",
        "updated clone",
        "different operand",
        "different location",
        "different hidden slots",
    ] {
        let mut items = vec![Item::Label(Label(0))];
        match change {
            "original clone" => items.push(Item::Instruction(condition)),
            "updated clone" => items.push(Item::Instruction(updated)),
            _ => {}
        }
        items.push(Item::Instruction(condition));
        items.push(Item::Instruction(marker));
        let mut assembler = Assembler::from_linear_items(items.clone());
        let mut source = condition;
        match change {
            "different operand" => source.set_operand(Operand::Forward(Label(4))),
            "different location" => source.location = SourceLocation::new(14, 14, 15, 35),
            "different hidden slots" => source.local_policy.add_hidden_exception_owner_slots(1),
            _ => {}
        }
        let source_count = match change {
            "no origin" => 0,
            "duplicate origin" => 2,
            _ => 1,
        };
        for _ in 0..source_count {
            assembler
                .exception_owner_provenance
                .source_none_conditions
                .push(SourceNoneCondition {
                    entry: Label(0),
                    labels: vec![Label(0)],
                    instruction: source,
                    entry_prefix: Some(vec![Item::Label(Label(0))]),
                });
        }
        let condition_item = items.len() - 2;
        items[condition_item] = Item::Instruction(updated);
        for _ in 0..2 {
            assembler.allow_last_condition_split_return_copy();
            assert_eq!(assembler.linear_items(), items.as_slice(), "{change}");
            let origins = &assembler.exception_owner_provenance.source_none_conditions;
            assert_eq!(origins.len(), source_count, "{change}");
            for origin in origins {
                assert_eq!(
                    origin.instruction,
                    if change == "unique" { updated } else { source },
                    "{change}"
                );
                assert_eq!(origin.entry, Label(0), "{change}");
                assert_eq!(origin.labels, vec![Label(0)], "{change}");
                assert_eq!(
                    origin.entry_prefix,
                    Some(vec![Item::Label(Label(0))]),
                    "{change}"
                );
            }
        }
    }
}
