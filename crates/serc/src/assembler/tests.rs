use super::{
    Assembler, AssemblerStage, Instruction, InstructionFlags, Item, Operand, SourceLocation,
};
use crate::target::opcodes::*;

#[test]
fn instruction_constructors_set_audited_defaults() {
    let regular = Instruction::new(LOAD_CONST, Operand::Value(0), SourceLocation::NONE, None);
    assert_eq!(regular.flags, InstructionFlags::DEFAULT);
    assert!(regular.has_flag(InstructionFlags::INLINE_SMALL_EXIT));
    assert!(regular.has_flag(InstructionFlags::ALLOW_JUMP_THREADING_TARGET));

    let synthetic = Instruction::synthetic(NOP, Operand::Value(0), SourceLocation::NONE, None);
    assert!(!synthetic.has_flag(InstructionFlags::INLINE_SMALL_EXIT));
    assert!(synthetic.has_flag(InstructionFlags::ALLOW_JUMP_THREADING_TARGET));
    assert_eq!(
        synthetic.preserve_direct_inlined_jump_nop, None,
        "synthetic instructions do not inherit jump provenance"
    );
    assert_eq!(synthetic.normalized_exception_owner, None);
}

#[test]
fn fusion_uses_each_flags_propagation_policy() {
    let mut first = Instruction::new(
        LOAD_FAST,
        Operand::Value(1),
        SourceLocation::new(2, 2, 0, 1),
        Some(1),
    );
    for flag in [
        InstructionFlags::FORCE_OWNED_LOAD,
        InstructionFlags::ALLOW_NO_LOCATION_BLOCK_INLINING,
        InstructionFlags::DEFER_REDUNDANT_JUMP_REMOVAL,
        InstructionFlags::CONVERTED_POP_BLOCK,
        InstructionFlags::EXCLUDE_EXCEPTION_IF_EXTENDED,
        InstructionFlags::PREVENT_FUSION_WITH_NEXT,
        InstructionFlags::PRESERVE_INLINED_JUMP_NOP,
    ] {
        first.insert_flag(flag);
    }
    first.remove_flag(InstructionFlags::ALLOW_JUMP_THREADING_TARGET);
    first.preserve_direct_inlined_jump_nop = Some(super::Label(1));
    first.normalized_exception_owner = Some(false);

    let mut second = Instruction::new(
        LOAD_FAST,
        Operand::Value(2),
        SourceLocation::new(2, 2, 2, 3),
        Some(2),
    );
    for flag in [
        InstructionFlags::STRICT_OWNED_LOAD,
        InstructionFlags::PRESERVE_NO_LOCATION,
        InstructionFlags::PRESERVE_NOP_AFTER_JUMP_THREADING,
        InstructionFlags::EXCLUDE_EXCEPTION,
        InstructionFlags::PREVENT_FUSION_WITH_PREVIOUS,
        InstructionFlags::BORROW_UNREACHABLE_ENTRY,
        InstructionFlags::PRESERVE_INLINED_JUMP_NOP,
    ] {
        second.insert_flag(flag);
    }
    second.remove_flag(InstructionFlags::INLINE_SMALL_EXIT);
    second.preserve_direct_inlined_jump_nop = Some(super::Label(2));
    second.normalized_exception_owner = Some(true);

    let fused = Instruction::fused(LOAD_FAST_LOAD_FAST, Operand::Value(0x12), first, second);

    for flag in [
        InstructionFlags::FORCE_OWNED_LOAD,
        InstructionFlags::STRICT_OWNED_LOAD,
        InstructionFlags::ALLOW_NO_LOCATION_BLOCK_INLINING,
        InstructionFlags::PRESERVE_NO_LOCATION,
        InstructionFlags::DEFER_REDUNDANT_JUMP_REMOVAL,
        InstructionFlags::PRESERVE_NOP_AFTER_JUMP_THREADING,
        InstructionFlags::CONVERTED_POP_BLOCK,
        InstructionFlags::EXCLUDE_EXCEPTION,
        InstructionFlags::EXCLUDE_EXCEPTION_IF_EXTENDED,
    ] {
        assert!(fused.has_flag(flag), "missing merged flag {flag:?}");
    }
    for flag in [
        InstructionFlags::INLINE_SMALL_EXIT,
        InstructionFlags::PRESERVE_INLINED_JUMP_NOP,
        InstructionFlags::ALLOW_JUMP_THREADING_TARGET,
        InstructionFlags::PREVENT_FUSION_WITH_NEXT,
        InstructionFlags::PREVENT_FUSION_WITH_PREVIOUS,
        InstructionFlags::BORROW_UNREACHABLE_ENTRY,
    ] {
        assert!(!fused.has_flag(flag), "unexpected merged flag {flag:?}");
    }
    assert_eq!(fused.location, first.location);
    assert_eq!(fused.depth_after, second.depth_after);
    assert_eq!(fused.preserve_direct_inlined_jump_nop, None);
    assert_eq!(fused.normalized_exception_owner, Some(false));

    let mut first_edge = Instruction::new(LOAD_FAST, Operand::Value(1), SourceLocation::NONE, None);
    first_edge.insert_flag(InstructionFlags::PREVENT_FUSION_WITH_PREVIOUS);
    first_edge.insert_flag(InstructionFlags::BORROW_UNREACHABLE_ENTRY);
    let mut second_edge =
        Instruction::new(LOAD_FAST, Operand::Value(2), SourceLocation::NONE, None);
    second_edge.insert_flag(InstructionFlags::PREVENT_FUSION_WITH_NEXT);
    let fused_edges = Instruction::fused(
        LOAD_FAST_LOAD_FAST,
        Operand::Value(0x12),
        first_edge,
        second_edge,
    );
    for flag in [
        InstructionFlags::INLINE_SMALL_EXIT,
        InstructionFlags::ALLOW_JUMP_THREADING_TARGET,
        InstructionFlags::PREVENT_FUSION_WITH_NEXT,
        InstructionFlags::PREVENT_FUSION_WITH_PREVIOUS,
        InstructionFlags::BORROW_UNREACHABLE_ENTRY,
    ] {
        assert!(
            fused_edges.has_flag(flag),
            "missing edge-selected flag {flag:?}"
        );
    }
}

#[test]
fn owned_fast_loads_use_real_opcodes_with_provenance() {
    let mut assembler = Assembler::default();
    assembler.emit_owned_fast_with_depth(3, 1);
    let Some(Item::Instruction(instruction)) = assembler.items.last() else {
        panic!("owned load was not emitted");
    };
    assert_eq!(instruction.opcode, LOAD_FAST);
    assert!(instruction.has_flag(InstructionFlags::FORCE_OWNED_LOAD));
    assert!(!instruction.has_flag(InstructionFlags::STRICT_OWNED_LOAD));
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
        items: vec![
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
        ],
        next_label: 3,
        preserved_block_boundaries: rustc_hash::FxHashSet::from_iter([preserved]),
        ..Assembler::default()
    }
}

#[test]
fn reuses_constant_reachability_graph_with_finish_partitioning() {
    let expected = assembler_with_preserved_constant_boundary()
        .finish()
        .unwrap();
    let mut assembler = assembler_with_preserved_constant_boundary();

    assert_eq!(
        assembler.used_constant_indices(LOAD_CONST),
        rustc_hash::FxHashSet::from_iter([0, 2])
    );
    assert!(assembler.items.is_empty());
    assert!(assembler.graph.is_some());
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

fn assembler_with_graph(items: Vec<Item>) -> Assembler {
    Assembler {
        graph: Some(super::block_graph::BlockGraph::from_items(
            items,
            &rustc_hash::FxHashSet::default(),
        )),
        ..Assembler::default()
    }
}

fn graph_opcodes(assembler: &Assembler) -> Vec<crate::target::Opcode> {
    assembler
        .graph
        .as_ref()
        .unwrap()
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
    let mut assembler = Assembler::default();
    let label = assembler.label();
    assembler.graph = assembler_with_graph(vec![
        Item::Label(label),
        pass_instruction(LOAD_FAST, 0, 1),
        pass_instruction(DELETE_FAST, 0, 1),
        Item::Instruction(Instruction::new(
            JUMP_BACKWARD,
            Operand::Backward(label),
            SourceLocation::new(1, 1, 0, 0),
            None,
        )),
    ])
    .graph;

    assembler.add_checks_for_uninitialized_loads(1, 1);

    assert_eq!(
        graph_opcodes(&assembler),
        [LOAD_FAST_CHECK, DELETE_FAST, JUMP_BACKWARD]
    );
}

#[test]
fn redundant_swap_removal_preserves_forward_scan_semantics() {
    let mut assembler = assembler_with_graph(vec![
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
    converted.insert_flag(InstructionFlags::CONVERTED_POP_BLOCK);
    let mut assembler = assembler_with_graph(vec![
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
