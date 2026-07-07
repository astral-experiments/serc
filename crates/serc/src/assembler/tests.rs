use super::ir::{
    BorrowFallthrough, ExceptionPolicy, FusionPolicy, JumpPolicy, LoadPolicy, NoLocationPolicy,
    RetainedJumpNop, UnoptimizedCode,
};
use super::{
    Assembler, AssemblerStage, ExceptionExclusion, Instruction, Item, NormalizedExceptionOwner,
    Operand, SourceLocation,
};
use crate::target::opcodes::*;

#[test]
#[cfg(target_pointer_width = "64")]
fn instruction_items_remain_cache_compact() {
    assert_eq!(std::mem::size_of::<Instruction>(), 48);
    assert_eq!(std::mem::size_of::<Item>(), 48);
}

#[test]
fn instruction_constructors_set_audited_defaults() {
    let regular = Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None);
    assert_eq!(regular.load_policy, LoadPolicy::default());
    assert_eq!(regular.no_location_policy, NoLocationPolicy::default());
    assert_eq!(regular.fusion_policy, FusionPolicy::default());
    assert!(!regular.converted_pop_block);
    assert_eq!(regular.jump_policy, JumpPolicy::default());

    let synthetic = Instruction::synthetic(NOP, Operand::Value(0), SourceLocation::NONE, None);
    assert!(!synthetic.jump_policy.inline_small_exit());
    let mut expected_synthetic_policy = JumpPolicy::default();
    expected_synthetic_policy.prevent_small_exit_inlining();
    assert_eq!(synthetic.jump_policy, expected_synthetic_policy);
    assert_eq!(synthetic.exception_policy, ExceptionPolicy::default());
    assert_eq!(regular.borrow_fallthrough, BorrowFallthrough::Reachable);
    assert_eq!(synthetic.borrow_fallthrough, BorrowFallthrough::Reachable);
}

#[test]
fn jump_policy_defaults_are_explicit() {
    let policy = JumpPolicy::default();
    assert!(policy.inline_small_exit());
    assert_eq!(policy.retained_nop(), RetainedJumpNop::None);
    assert!(policy.allows_threading_target());
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
        policy
    }

    let retained_nops = [
        RetainedJumpNop::None,
        RetainedJumpNop::DirectTarget,
        RetainedJumpNop::Always,
    ];
    for first_bits in 0..16 {
        for second_bits in 0..16 {
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
                            ) << 3);
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
    assert!(instruction.jump_policy.retains_nop_for(instruction.operand));

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
    assert!(!instruction.jump_policy.retains_nop_for(instruction.operand));

    instruction.jump_policy.retain_nop();
    instruction.retarget(Operand::Backward(expected));
    assert!(instruction.jump_policy.always_retains_nop());
    assert!(instruction.jump_policy.retains_nop_for(instruction.operand));
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
#[should_panic(expected = "deferring redundant jump removal requires a preceding instruction")]
fn jump_policy_mutation_requires_a_preceding_instruction() {
    Assembler::default().defer_last_jump_removal();
}

#[test]
#[should_panic(
    expected = "preventing fusion with the next instruction requires a preceding instruction"
)]
fn instruction_policy_mutation_requires_a_preceding_instruction() {
    Assembler::default().prevent_last_instruction_fusion();
}

#[test]
#[should_panic(expected = "preventing jump inlining requires a jump instruction")]
fn jump_policy_mutation_requires_a_jump() {
    let mut assembler = Assembler::default();
    assembler.emit(NOP, 0);
    assembler.prevent_last_jump_inlining();
}

#[test]
#[should_panic(expected = "retaining a directly inlined jump NOP requires a forward jump")]
fn direct_jump_nop_provenance_requires_a_forward_jump() {
    let mut assembler = Assembler::default();
    let target = assembler.label();
    assembler.emit_backward(JUMP_BACKWARD, target);
    assembler.preserve_last_direct_inlined_jump_nop();
}

#[test]
fn borrow_fallthrough_barrier_stays_on_predecessor_across_labels_and_emission() {
    let mut assembler = Assembler::default();
    assembler.emit(LOAD_CONST, 0);
    let boundary = assembler.label();
    assembler.mark(boundary);
    assembler.mark_last_borrow_fallthrough_barrier();
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
    assert_eq!(predecessor.borrow_fallthrough, BorrowFallthrough::Barrier);
    assert_eq!(next.borrow_fallthrough, BorrowFallthrough::Reachable);
}

#[test]
#[should_panic(expected = "borrow fallthrough barrier requires a preceding instruction")]
fn borrow_fallthrough_barrier_requires_a_predecessor() {
    let mut assembler = Assembler::default();
    let boundary = assembler.label();
    assembler.mark(boundary);
    assembler.mark_last_borrow_fallthrough_barrier();
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
    assert_eq!(normalized.exception_policy, normalized_owner);
    assert_eq!(excluded.exception_policy, ExceptionPolicy::EXCLUDE_ALWAYS);
    assert_eq!(default.exception_policy, ExceptionPolicy::default());
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
    assert_eq!(default.exception_policy, ExceptionPolicy::default());
    assert_eq!(
        excluded.exception_policy,
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
fn fusion_preserves_each_typed_instruction_policy() {
    let mut first = Instruction::new(
        LOAD_FAST,
        Operand::Value(1),
        SourceLocation::new(2, 2, 0, 1),
        Some(1),
    );
    first.load_policy.force_owned = true;
    first.no_location_policy.allow_block_inlining = true;
    first.converted_pop_block = true;
    first.fusion_policy.prevent_with_next = true;
    first.jump_policy.prevent_threading_target();
    first.jump_policy.defer_removal();
    first.jump_policy.retain_nop();
    first
        .exception_policy
        .exclude(ExceptionExclusion::IfExtended);
    first.exception_policy.normalized_owner = NormalizedExceptionOwner::Absent;

    let mut second = Instruction::new(
        LOAD_FAST,
        Operand::Value(2),
        SourceLocation::new(2, 2, 2, 3),
        Some(2),
    );
    second.load_policy.strict_owned = true;
    second.no_location_policy.preserve = true;
    second.fusion_policy.prevent_with_previous = true;
    second.jump_policy.prevent_small_exit_inlining();
    second.jump_policy.retain_redundant_nop_after_threading();
    second.jump_policy.retain_nop_for_direct_target();
    second.exception_policy.exclude(ExceptionExclusion::Always);
    second.exception_policy.normalized_owner = NormalizedExceptionOwner::Present;
    second.borrow_fallthrough = BorrowFallthrough::Barrier;

    let fused = Instruction::fused(LOAD_FAST_LOAD_FAST, Operand::Value(0x12), first, second);

    assert_eq!(
        fused.load_policy,
        LoadPolicy {
            force_owned: true,
            strict_owned: true,
        }
    );
    assert_eq!(
        fused.no_location_policy,
        NoLocationPolicy {
            allow_block_inlining: true,
            preserve: true,
        }
    );
    assert_eq!(fused.fusion_policy, FusionPolicy::default());
    assert!(fused.converted_pop_block);
    assert_eq!(fused.location, first.location);
    assert_eq!(fused.depth_after, second.depth_after);
    let mut expected_jump_policy = JumpPolicy::default();
    expected_jump_policy.prevent_small_exit_inlining();
    expected_jump_policy.prevent_threading_target();
    expected_jump_policy.retain_redundant_nop_after_threading();
    assert_eq!(fused.jump_policy, expected_jump_policy);
    assert_eq!(fused.borrow_fallthrough, BorrowFallthrough::Barrier);
    assert_eq!(
        fused.exception_policy,
        ExceptionPolicy {
            exclusion: ExceptionExclusion::Always,
            normalized_owner: NormalizedExceptionOwner::Absent,
        }
    );

    let mut first_edge = Instruction::new(LOAD_FAST, Operand::Value(1), SourceLocation::NONE, None);
    first_edge.fusion_policy.prevent_with_previous = true;
    first_edge.borrow_fallthrough = BorrowFallthrough::Barrier;
    let mut second_edge =
        Instruction::new(LOAD_FAST, Operand::Value(2), SourceLocation::NONE, None);
    second_edge.fusion_policy.prevent_with_next = true;
    let fused_edges = Instruction::fused(
        LOAD_FAST_LOAD_FAST,
        Operand::Value(0x12),
        first_edge,
        second_edge,
    );
    assert_eq!(
        fused_edges.fusion_policy,
        FusionPolicy {
            prevent_with_previous: true,
            prevent_with_next: true,
        }
    );
    assert_eq!(fused_edges.jump_policy, JumpPolicy::default());
    assert_eq!(fused_edges.borrow_fallthrough, BorrowFallthrough::Reachable);
}

#[test]
fn owned_fast_loads_use_real_opcodes_with_provenance() {
    let mut assembler = Assembler::default();
    assembler.emit_owned_fast_with_depth(3, 1);
    let Some(Item::Instruction(instruction)) = assembler.linear_items().last() else {
        panic!("owned load was not emitted");
    };
    assert_eq!(instruction.opcode, LOAD_FAST);
    assert!(instruction.load_policy.force_owned);
    assert!(!instruction.load_policy.strict_owned);
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
fn removes_an_overwritten_local_store() {
    let mut assembler = Assembler::default();
    assembler.emit(STORE_FAST, 1);
    assembler.emit(STORE_FAST, 1);
    assert_eq!(
        assembler.finish().unwrap(),
        [POP_TOP.code(), 0, STORE_FAST.code(), 1]
    );
}

fn assembler_with_preserved_constant_boundary() -> Assembler {
    let entry = super::Label(0);
    let preserved = super::Label(1);
    let target = super::Label(2);
    Assembler {
        code: UnoptimizedCode::Linear(vec![
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
        ]),
        next_label: 3,
        preserved_block_boundaries: rustc_hash::FxHashSet::from_iter([preserved]),
        ..Assembler::default()
    }
}

#[test]
fn constant_reachability_prepares_the_optimization_graph() {
    let expected = assembler_with_preserved_constant_boundary()
        .finish()
        .unwrap();
    let mut assembler = assembler_with_preserved_constant_boundary();

    assert_eq!(
        assembler.used_constant_indices(LOAD_CONST),
        rustc_hash::FxHashSet::from_iter([0, 2])
    );
    assert!(matches!(assembler.code, UnoptimizedCode::Prepared(_)));
    assembler.remap_constant_indices(LOAD_CONST, &[Some(0), Some(1), Some(2)]);

    assert_eq!(assembler.finish().unwrap(), expected);
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
            Item::Label(label)
            | Item::Instruction(Instruction {
                operand: Operand::Forward(label) | Operand::Backward(label),
                ..
            }) => Some(label.0 + 1),
            Item::Instruction(_) => None,
        })
        .max()
        .unwrap_or(0);
    Assembler {
        code: UnoptimizedCode::Linear(items),
        next_label,
        ..Assembler::default()
    }
    .into_optimization_pipeline()
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

    assembler.remove_redundant_swaps_before_pops();

    assert_eq!(graph_opcodes(&assembler), [SWAP, SWAP, POP_TOP, POP_TOP]);
}

#[test]
fn redundant_nop_removal_preserves_forward_scan_semantics() {
    let mut converted = match pass_instruction(NOP, 0, 1) {
        Item::Instruction(instruction) => instruction,
        Item::Label(_) => unreachable!(),
    };
    converted.converted_pop_block = true;
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
