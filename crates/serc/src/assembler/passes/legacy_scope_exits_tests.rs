use super::*;
use crate::code_object::NonReflexiveRemap;

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

fn instruction(opcode: crate::target::Opcode, argument: u32) -> Instruction {
    Instruction::new(
        opcode,
        Operand::Value(argument),
        SourceLocation::NONE,
        Some(0),
    )
}

fn footer(instructions: Vec<Instruction>) -> Assembler {
    let mut items = vec![
        Item::Label(Label::from_u32(0)),
        Item::Label(Label::from_u32(1)),
    ];
    items.extend(instructions.into_iter().map(Item::Instruction));
    let mut assembler = Assembler::from_linear_items(items);
    assembler.python_version = PythonVersion::Py312;
    assembler.next_label = 2;
    assembler.bind_legacy_native_label(Label::from_u32(0), 11);
    assembler.bind_legacy_native_label(Label::from_u32(1), 11);
    assembler.record_pending_original_scope_exit(Label::from_u32(0));
    assembler.certify_legacy_native_labels();
    assembler
}

fn promote(assembler: Assembler, constants: Option<&dyn ConstantTable>) -> OptimizationPipeline {
    let mut pipeline = assembler.into_optimization_pipeline();
    let arrays = pipeline.legacy_instruction_arrays();
    pipeline.initialize_legacy_native_jump_seeds(&arrays);
    pipeline.reject_unmodeled_empty_native_jump_targets();
    let before = pipeline.graph.copy_items();
    pipeline.promote_pending_original_scope_exits(&arrays, constants);
    assert_eq!(pipeline.graph.copy_items(), before);
    pipeline
}

#[test]
fn pending_scope_exit_promotes_only_the_explicit_none_footer_label() {
    for instructions in [
        vec![instruction(LOAD_CONST, 1), instruction(RETURN_VALUE, 0)],
        vec![instruction(RETURN_CONST, 1)],
    ] {
        let constants = Constants(vec![Constant::Bool(false), Constant::None]);
        let mut pipeline = promote(footer(instructions), Some(&constants));
        let expected = [Label::from_u32(0)].into_iter().collect();
        assert_eq!(pipeline.original_scope_exit_labels, expected);
        // Later jump-seed invalidation does not erase established original provenance.
        pipeline.legacy_native_labels.as_mut().unwrap().invalidate();
        assert_eq!(pipeline.original_scope_exit_labels, expected);
    }
}

#[test]
fn pending_scope_exit_rejects_locations_operands_and_nondefault_policies() {
    let constants = Constants(vec![Constant::None]);
    for case in 0..15 {
        let mut instructions = vec![instruction(LOAD_CONST, 0), instruction(RETURN_VALUE, 0)];
        match case {
            0 => instructions[0].location = SourceLocation::new(1, 1, 0, 1),
            1 => instructions[1].location = SourceLocation::new(1, 1, 0, 1),
            2 => instructions[0].set_operand(Operand::Value(1)),
            3 => instructions[1].set_operand(Operand::Value(1)),
            4 => instructions.insert(1, instruction(NOP, 0)),
            5 => instructions[0].opcode = LOAD_FAST,
            6 => instructions[0].no_location_policy.allow_block_inlining(),
            7 => instructions[0].set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
            8 => instructions[1].mark_original_annotation_jump(),
            9 => instructions.clear(),
            10 => instructions = vec![instruction(RETURN_VALUE, 0)],
            11 => instructions[0].location = SourceLocation::new(-1, 1, -1, -1),
            12 => instructions[0].local_policy.mark_force_borrowed_load(),
            13 => instructions[0].fusion_policy.prevent_with_next(),
            14 => instructions[1].jump_policy.retain_nop(),
            _ => unreachable!(),
        }
        let pipeline = promote(footer(instructions), Some(&constants));
        assert!(
            pipeline.original_scope_exit_labels.is_empty(),
            "case {case}"
        );
    }
    for constants in [Constants(vec![]), Constants(vec![Constant::Bool(false)])] {
        let pipeline = promote(footer(vec![instruction(RETURN_CONST, 0)]), Some(&constants));
        assert!(pipeline.original_scope_exit_labels.is_empty());
    }
    let pipeline = promote(footer(vec![instruction(RETURN_CONST, 0)]), None);
    assert!(pipeline.original_scope_exit_labels.is_empty());
}

#[test]
fn pending_scope_exit_rejects_absent_or_invalid_maps_without_removing_old_certificates() {
    let constants = Constants(vec![Constant::None]);
    for case in 0..4 {
        let mut assembler = footer(vec![instruction(RETURN_CONST, 0)]);
        assembler.record_original_scope_exit(Label::from_u32(1));
        match case {
            0 => assembler.legacy_native_labels = None,
            1 => assembler.invalidate_legacy_native_labels(),
            2 => assembler.bind_legacy_native_label(Label::from_u32(1), 12),
            3 => assembler.python_version = PythonVersion::Py313,
            _ => unreachable!(),
        }
        let pipeline = promote(assembler, Some(&constants));
        assert_eq!(
            pipeline.original_scope_exit_labels,
            [Label::from_u32(1)].into_iter().collect(),
            "case {case}"
        );
    }
}

#[test]
fn pending_scope_exit_waits_for_unrelated_jump_target_preflight() {
    let constants = Constants(vec![Constant::None]);
    for empty_target in [false, true] {
        let start = Label::from_u32(0);
        let end = Label::from_u32(1);
        let other = Label::from_u32(2);
        let mut items = vec![
            Item::Label(start),
            Item::Instruction(Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(other),
                SourceLocation::NONE,
                Some(0),
            )),
            Item::Label(end),
            Item::Instruction(instruction(RETURN_CONST, 0)),
            Item::Label(other),
        ];
        if !empty_target {
            items.push(Item::Instruction(instruction(RETURN_CONST, 0)));
        }
        let mut assembler = Assembler::from_linear_items(items);
        assembler.python_version = PythonVersion::Py312;
        assembler.next_label = 3;
        assembler.bind_legacy_native_label(start, 10);
        assembler.bind_legacy_native_label(end, 11);
        if empty_target {
            assembler.bind_legacy_native_label(other, 12);
        }
        assembler.record_pending_original_scope_exit(end);
        assembler.certify_legacy_native_labels();
        let pipeline = promote(assembler, Some(&constants));
        assert!(
            !pipeline
                .legacy_native_labels
                .as_ref()
                .unwrap()
                .is_certified()
        );
        assert!(pipeline.original_scope_exit_labels.is_empty());
    }
}
