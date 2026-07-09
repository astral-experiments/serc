mod block_graph;
mod ir;
mod layout;
mod passes;

#[cfg(test)]
mod tests;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::CompileError;
use crate::target::opcodes::{
    CHECK_EXC_MATCH, COMPARE_OP, CONTAINS_OP, COPY, COPY_FREE_VARS, DELETE_DEREF, DELETE_FAST,
    DICT_MERGE, DICT_UPDATE, END_SEND, EXTENDED_ARG, FOR_ITER, FORMAT_SIMPLE, GET_ANEXT, GET_LEN,
    GET_YIELD_FROM_ITER, IMPORT_FROM, IS_OP, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT,
    JUMP_FORWARD, LIST_APPEND, LIST_EXTEND, LOAD_ATTR, LOAD_COMMON_CONSTANT, LOAD_CONST, LOAD_FAST,
    LOAD_FAST_AND_CLEAR, LOAD_FAST_BORROW, LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_CHECK,
    LOAD_FAST_LOAD_FAST, LOAD_GLOBAL, LOAD_SMALL_INT, LOAD_SPECIAL, LOAD_SUPER_ATTR, MAKE_CELL,
    MAP_ADD, MATCH_KEYS, MATCH_MAPPING, MATCH_SEQUENCE, NOP, NOT_TAKEN, POP_EXCEPT,
    POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, POP_TOP, PUSH_EXC_INFO, RERAISE, RESUME, RETURN_VALUE,
    SEND, SET_ADD, SET_FUNCTION_ATTRIBUTE, SET_UPDATE, STORE_DEREF, STORE_FAST,
    STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST, STORE_GLOBAL, SWAP, TO_BOOL, UNARY_NOT,
    WITH_EXCEPT_START,
};
use crate::target::operands::ResumeLocation;
use crate::target::{
    Opcode, is_conditional_jump, is_scope_exit as ends_scope, is_unconditional_jump,
    num_popped as opcode_num_popped, num_pushed as opcode_num_pushed,
};

#[cfg(any(test, debug_assertions))]
use ir::AssemblerStage;
pub(crate) use ir::{
    AssembledCode, Assembler, ExceptionPolicy, Label, OperandPatchId, SourceLocation,
};
use ir::{
    BorrowFallthrough, ExceptionExclusion, ExceptionRegion, Instruction, Item, LayoutAssembler,
    LoadPolicy, NormalizedExceptionOwner, Operand, OperandPatch, OptimizationPipeline,
    UnoptimizedCode,
};

fn reachable_constant_indices(
    graph: &block_graph::BlockGraph,
    exception_regions: &[ExceptionRegion],
    load_const: Opcode,
) -> FxHashSet<u32> {
    let reachable = graph.item_reachability(exception_regions);
    graph
        .iter_items()
        .enumerate()
        .filter_map(|(position, item)| {
            let Item::Instruction(instruction) = item else {
                return None;
            };
            if reachable[position]
                && instruction.opcode == load_const
                && let Operand::Value(index) = instruction.operand
            {
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

impl Assembler {
    fn linear_items(&self) -> &[Item] {
        let UnoptimizedCode::Linear(items) = &self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        items
    }

    fn linear_items_mut(&mut self) -> &mut Vec<Item> {
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        items
    }

    pub(crate) fn disable_load_fast_borrowing(&mut self) {
        self.load_fast_borrowing_enabled = false;
    }

    /// Prevents local loads emitted while enabled from becoming borrowed loads.
    pub(crate) fn set_strict_owned_loads(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.strict_owned_loads, enabled)
    }

    pub(crate) fn location(&self) -> SourceLocation {
        self.location
    }

    pub(crate) fn set_location(&mut self, location: SourceLocation) {
        self.location = location;
    }

    pub(crate) fn last_instruction_location(&self) -> Option<SourceLocation> {
        self.linear_items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location),
                Item::Label(_) => None,
            })
    }

    pub(crate) fn last_instruction_is_nop(&self) -> bool {
        self.linear_items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.opcode == NOP),
                Item::Label(_) => None,
            })
            .unwrap_or(false)
    }

    fn last_instruction_mut(&mut self) -> Option<&mut Instruction> {
        self.linear_items_mut()
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
    }

    fn required_last_instruction_mut(&mut self, operation: &str) -> &mut Instruction {
        self.last_instruction_mut()
            .unwrap_or_else(|| panic!("{operation} requires a preceding instruction"))
    }

    fn last_jump_instruction_mut(&mut self, operation: &str) -> &mut Instruction {
        let instruction = self.required_last_instruction_mut(operation);
        assert!(
            matches!(
                instruction.operand,
                Operand::Forward(_) | Operand::Backward(_)
            ),
            "{operation} requires a jump instruction"
        );
        instruction
    }

    fn current_block_start(&self) -> usize {
        self.linear_items()
            .iter()
            .rposition(|item| match item {
                Item::Label(_) => true,
                Item::Instruction(instruction) => {
                    instruction.opcode == NOT_TAKEN
                        || (!is_conditional_jump(instruction.opcode)
                            && (matches!(
                                instruction.operand,
                                Operand::Forward(_) | Operand::Backward(_)
                            ) || ends_scope(instruction.opcode)))
                }
            })
            .map_or(0, |index| index + 1)
    }

    /// Returns an opaque token for the current emitted control-flow block.
    pub(crate) fn current_block_token(&self) -> usize {
        self.current_block_start()
    }

    pub(crate) fn instruction_count(&self) -> usize {
        self.linear_items()
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count()
    }

    pub(crate) fn contains_opcode(&self, opcode: Opcode) -> bool {
        self.linear_items().iter().any(
            |item| matches!(item, Item::Instruction(instruction) if instruction.opcode == opcode),
        )
    }

    /// Returns how many instruction slots optimization can vacate before a normalized marker.
    ///
    /// CPython compacts instructions without clearing their exception-handler fields. Later
    /// insertions can reuse those stale slots, so normalized instructions may acquire an owner. A
    /// fallthrough scan crosses labels that only delimit exception regions, matching CPython's
    /// pseudo-instruction representation of those boundaries.
    fn dynamic_exception_owner_slots(&self, cross_labels: bool) -> usize {
        fn will_fuse(first: Instruction, second: Instruction) -> bool {
            if first.fusion_policy.prevent_with_next
                || first.borrow_fallthrough == BorrowFallthrough::Barrier
                || second.fusion_policy.prevent_with_previous
            {
                return false;
            }
            let (Operand::Value(first_argument), Operand::Value(second_argument)) =
                (first.operand, second.operand)
            else {
                return false;
            };
            first_argument < 16
                && second_argument < 16
                && first.location.line == second.location.line
                && !(first.opcode == STORE_FAST
                    && second.opcode == STORE_FAST
                    && first_argument == second_argument)
                && matches!(
                    (first.opcode, second.opcode),
                    (LOAD_FAST_BORROW, LOAD_FAST_BORROW)
                        | (LOAD_FAST, LOAD_FAST)
                        | (
                            STORE_FAST,
                            LOAD_FAST | LOAD_FAST_BORROW | LOAD_FAST_CHECK | STORE_FAST
                        )
                )
        }

        let items = self.linear_items();
        let block_labels = cross_labels.then(|| {
            let mut block_labels = self.preserved_block_boundaries.clone();
            for region in &self.exception_regions {
                block_labels.insert(region.target);
            }
            for item in items {
                if let Item::Instruction(Instruction {
                    operand: Operand::Forward(label) | Operand::Backward(label),
                    ..
                }) = item
                {
                    block_labels.insert(*label);
                }
            }
            block_labels
        });
        let start = if let Some(block_labels) = &block_labels {
            items
                .iter()
                .rposition(|item| match item {
                    Item::Label(label) => block_labels.contains(label),
                    Item::Instruction(instruction) => {
                        instruction.opcode == NOT_TAKEN
                            || (!is_conditional_jump(instruction.opcode)
                                && (matches!(
                                    instruction.operand,
                                    Operand::Forward(_) | Operand::Backward(_)
                                ) || ends_scope(instruction.opcode)))
                    }
                })
                .map_or(0, |index| index + 1)
        } else {
            self.current_block_start()
        };
        let mut slots = 0;
        let mut late_inserted_slots = 0;
        let mut nop_count = 0;
        let mut index = start;
        while index < items.len() {
            let instruction = match items[index] {
                Item::Instruction(instruction) => instruction,
                Item::Label(label)
                    if block_labels
                        .as_ref()
                        .is_some_and(|block_labels| !block_labels.contains(&label)) =>
                {
                    index += 1;
                    continue;
                }
                Item::Label(_) => break,
            };
            if block_labels.is_some() {
                late_inserted_slots +=
                    usize::from(matches!(instruction.opcode, COPY_FREE_VARS | MAKE_CELL));
                nop_count += usize::from(instruction.opcode == NOP);
            }
            slots += usize::from(instruction.hidden_exception_owner_slots);
            let following = items.get(index + 1).and_then(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            });
            let following_location = if let Some(block_labels) = &block_labels {
                let mut following = None;
                for item in &items[index + 1..] {
                    match item {
                        Item::Instruction(instruction) => {
                            following = Some(*instruction);
                            break;
                        }
                        Item::Label(label) if block_labels.contains(label) => break,
                        Item::Label(_) => {}
                    }
                }
                following
            } else {
                following
            };

            let removable_nop = instruction.opcode == NOP
                && (instruction.location.line < 0
                    || following_location.is_some_and(|following| {
                        following.location.line == instruction.location.line
                    }));
            let applied_comprehension_swap = following.is_some_and(|following| {
                following.opcode == STORE_FAST
                    && following.fusion_policy.prevent_with_previous
                    && matches!(instruction.opcode, POP_TOP | STORE_FAST)
                    && !(instruction.opcode == STORE_FAST
                        && instruction.fusion_policy.prevent_with_previous)
            });
            if matches!(
                instruction.opcode,
                LOAD_FAST_BORROW_LOAD_FAST_BORROW
                    | LOAD_FAST_LOAD_FAST
                    | STORE_FAST_LOAD_FAST
                    | STORE_FAST_STORE_FAST
            ) || matches!(
                (instruction.opcode, instruction.operand),
                (LOAD_GLOBAL, Operand::Value(argument)) if argument & 1 != 0
            ) || instruction.converted_pop_block
                || removable_nop
                || applied_comprehension_swap
            {
                slots += 1;
                index += 1;
                continue;
            }
            if following.is_some_and(|following| will_fuse(instruction, following)) {
                slots += 1;
                slots += following.map_or(0, |following| {
                    usize::from(following.hidden_exception_owner_slots)
                });
                index += 2;
                continue;
            }
            index += 1;
        }
        // Late prologue insertions consume stale slots instead of creating them.
        // COPY_FREE_VARS and MAKE_CELL are the two such insertions relevant here.
        let slots = slots.saturating_sub(late_inserted_slots);
        if cross_labels && nop_count >= 2 {
            // Consecutive protected regions can leave a stale slot even when their removed pseudo
            // instructions are absent from our IR.
            slots.max(1)
        } else {
            slots
        }
    }

    pub(crate) fn current_block_dynamic_exception_owner_slots(&self) -> usize {
        self.dynamic_exception_owner_slots(false)
    }

    /// Returns whether optimization can leave a dynamic exception owner in the current block.
    pub(crate) fn current_block_has_dynamic_exception_owner(&self) -> bool {
        self.current_block_dynamic_exception_owner_slots() > 0
    }

    /// Returns whether optimization can leave a dynamic exception owner before a normalized
    /// condition marker, including across labels that only delimit exception regions.
    ///
    /// CPython represents an exception-region start with a pseudo instruction rather than a CFG
    /// label, so compaction before a try body can shift stale owners into a marker appended
    /// inside the body.
    pub(crate) fn fallthrough_has_dynamic_exception_owner(&self) -> bool {
        self.dynamic_exception_owner_slots(true) > 0
    }

    /// Returns whether the current block resumes directly after a plain `yield`.
    pub(crate) fn current_block_resumes_after_yield(&self) -> bool {
        for item in self.linear_items().iter().rev() {
            let Item::Instruction(instruction) = item else {
                break;
            };
            if instruction.opcode == RESUME {
                return matches!(
                    instruction.operand,
                    Operand::Value(argument)
                        if argument & 3 == ResumeLocation::AFTER_YIELD.argument()
                );
            }
            if instruction.opcode == NOT_TAKEN
                || (!is_conditional_jump(instruction.opcode)
                    && (matches!(
                        instruction.operand,
                        Operand::Forward(_) | Operand::Backward(_)
                    ) || ends_scope(instruction.opcode)))
            {
                break;
            }
        }
        false
    }

    pub(crate) fn label(&mut self) -> Label {
        let label = Label(self.next_label);
        self.next_label += 1;
        label
    }

    pub(crate) fn mark(&mut self, label: Label) {
        self.linear_items_mut().push(Item::Label(label));
    }

    /// Inserts a boundary before every join label at the current instruction position.
    pub(crate) fn mark_before_trailing_labels(&mut self, label: Label) {
        let index = self
            .linear_items()
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .map_or(0, |index| index + 1);
        self.insert_label(index, label);
    }

    /// Keep a source CFG boundary after jump threading removes its last incoming edge.
    pub(crate) fn preserve_block_boundary(&mut self, label: Label) {
        self.preserved_block_boundaries.insert(label);
    }

    /// Keep a source CFG boundary unless the instruction stream already ends at one.
    ///
    /// Coalescing consecutive join labels avoids creating an artificial empty block between
    /// nested expressions while retaining the boundary after ordinary instruction sequences.
    pub(crate) fn preserve_block_boundary_if_new(&mut self, label: Label) {
        if !matches!(self.linear_items().last(), Some(Item::Label(_))) {
            self.preserve_block_boundary(label);
        }
    }

    /// Keeps CPython's block-local borrowed-load optimizer from visiting this block.
    pub(crate) fn prevent_borrow_reachability(&mut self, label: Label) {
        self.borrow_unreachable_blocks.insert(label);
    }

    /// Stops the final borrowed-load analysis at the last emitted instruction.
    pub(crate) fn mark_last_borrow_fallthrough_barrier(&mut self) {
        self.last_instruction_mut()
            .expect("borrow fallthrough barrier requires a preceding instruction")
            .borrow_fallthrough = BorrowFallthrough::Barrier;
    }

    /// Retains a redundant jump as an analysis-only marker for CPython's empty CFG block.
    pub(crate) fn mark_last_borrow_empty_block(&mut self) {
        self.last_jump_instruction_mut("marking an empty borrowed-load block")
            .jump_policy
            .mark_borrow_empty_block();
    }

    pub(crate) fn mark_before_trailing_instructions(&mut self, label: Label, count: usize) {
        let items = self.linear_items();
        let index = items
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, item)| matches!(item, Item::Instruction(_)))
            .nth(count.saturating_sub(1))
            .map_or(items.len(), |(index, _)| index);
        self.insert_label(index, label);
    }

    pub(crate) fn mark_before_trailing_converted_pop_block(&mut self, label: Label) -> bool {
        let items = self.linear_items();
        let mut instructions = items.iter().enumerate().rev().filter_map(|(index, item)| {
            let Item::Instruction(instruction) = item else {
                return None;
            };
            Some((index, instruction))
        });
        let Some((_, _)) = instructions.next() else {
            return false;
        };
        let Some((mut index, instruction)) = instructions.next() else {
            return false;
        };
        if !instruction.converted_pop_block {
            return false;
        }
        if let Some((previous_index, previous)) = instructions.next()
            && previous.opcode == NOP
        {
            index = previous_index;
        }
        self.insert_label(index, label);
        true
    }

    fn insert_label(&mut self, index: usize, label: Label) {
        let patch_index = u32::try_from(index).expect("assembler item index should fit in u32");
        for patch in &mut self.operand_patches {
            if patch.item_index >= patch_index {
                patch.item_index = patch
                    .item_index
                    .checked_add(1)
                    .expect("too many assembler items");
            }
        }
        self.linear_items_mut().insert(index, Item::Label(label));
    }

    pub(crate) fn fusion_barrier(&mut self) {
        let label = self.label();
        self.mark(label);
    }

    /// Records compiler-side fold vacancies on the instruction that replaced them.
    pub(crate) fn add_last_hidden_exception_owner_slots(&mut self, slots: usize) {
        let slots = u8::try_from(slots.min(3)).expect("capped hidden slot count fits in u8");
        let instruction = self.required_last_instruction_mut("recording hidden owner slots");
        instruction.hidden_exception_owner_slots = instruction
            .hidden_exception_owner_slots
            .saturating_add(slots)
            .min(3);
    }

    /// Prevents fusion with the next instruction without introducing a CFG boundary.
    pub(crate) fn prevent_last_instruction_fusion(&mut self) {
        self.required_last_instruction_mut("preventing fusion with the next instruction")
            .fusion_policy
            .prevent_with_next = true;
    }

    /// Prevents fusion with the previous instruction without introducing a CFG boundary.
    pub(crate) fn prevent_last_instruction_fusion_with_previous(&mut self) {
        self.required_last_instruction_mut("preventing fusion with the previous instruction")
            .fusion_policy
            .prevent_with_previous = true;
    }

    /// Defers a redundant jump until the pass after exit-block duplication.
    pub(crate) fn defer_last_jump_removal(&mut self) {
        self.last_jump_instruction_mut("deferring redundant jump removal")
            .jump_policy
            .defer_removal();
    }

    /// Allows incoming jumps to thread through the last jump before retaining it as a NOP.
    pub(crate) fn preserve_last_redundant_jump_nop_after_threading(&mut self) {
        let policy = &mut self
            .last_jump_instruction_mut("retaining a redundant jump NOP after threading")
            .jump_policy;
        policy.retain_redundant_nop_after_threading();
    }

    pub(crate) fn prevent_last_jump_inlining(&mut self) {
        self.last_jump_instruction_mut("preventing jump inlining")
            .jump_policy
            .prevent_small_exit_inlining();
    }

    pub(crate) fn preserve_last_inlined_jump_nop(&mut self) {
        self.last_jump_instruction_mut("retaining an inlined jump NOP")
            .jump_policy
            .retain_nop();
    }

    pub(crate) fn preserve_last_direct_inlined_jump_nop(&mut self) {
        let instruction = self.last_jump_instruction_mut("retaining a directly inlined jump NOP");
        let Operand::Forward(_) = instruction.operand else {
            panic!("retaining a directly inlined jump NOP requires a forward jump");
        };
        instruction.jump_policy.retain_nop_for_direct_target();
    }

    pub(crate) fn prevent_last_jump_threading_target(&mut self) {
        self.last_jump_instruction_mut("preventing jump threading through a target")
            .jump_policy
            .prevent_threading_target();
    }

    pub(crate) fn prepare_last_no_location_block_for_inlining(&mut self) {
        let instruction =
            self.last_jump_instruction_mut("preparing a no-location block for inlining");
        instruction.location = SourceLocation::NONE;
        instruction.jump_policy.prevent_threading_target();
        instruction.no_location_policy.allow_block_inlining = true;
    }

    pub(crate) fn preserve_last_no_location(&mut self) {
        self.required_last_instruction_mut("preserving a no-location instruction")
            .no_location_policy
            .preserve = true;
    }

    pub(crate) fn inherit_percent_target_location_for_last_instruction(&mut self) {
        self.required_last_instruction_mut(
            "inheriting a predecessor location for a synthesized percent instruction",
        )
        .no_location_policy
        .percent_target_location = u8::MAX;
        self.has_synthesized_percent_target_locations = true;
    }

    /// Gives the last instruction its final successor's location after optimization.
    /// This mirrors CPython's `NEXT_LOCATION` pseudo-location.
    pub(crate) fn use_next_location_for_last_instruction(&mut self) {
        self.required_last_instruction_mut("using the next instruction location")
            .location_from_next = true;
    }

    pub(crate) fn mark_last_as_converted_pop_block(&mut self) {
        self.required_last_instruction_mut("marking a converted POP_BLOCK")
            .converted_pop_block = true;
    }

    pub(crate) fn preserve_last_nop(&mut self) {
        let instruction = self.required_last_instruction_mut("preserving an explicit NOP");
        assert_eq!(
            instruction.opcode, NOP,
            "preserving an explicit NOP requires a NOP instruction"
        );
        instruction.jump_policy.preserve_explicit_nop();
    }

    /// Removes a trailing NOP and returns its location and immediate exclusion labels, if any.
    pub(crate) fn take_trailing_nop_location(
        &mut self,
    ) -> Option<(SourceLocation, Option<(Label, Label)>)> {
        let current_location = self.location;
        let items = self.linear_items_mut();
        let index = items
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))?;
        let Item::Instruction(instruction) = items[index] else {
            unreachable!();
        };
        if instruction.opcode != NOP || instruction.jump_policy.preserves_explicit_nop() {
            return None;
        }
        if instruction.location.line >= 0
            && current_location.line >= 0
            && instruction.location.line != current_location.line
        {
            return None;
        }
        let exclusion = if index > 0 {
            match (items[index - 1], items.get(index + 1).copied()) {
                (Item::Label(start), Some(Item::Label(end))) => Some((start, end)),
                _ => None,
            }
        } else {
            None
        };
        items.remove(index);
        Some((instruction.location, exclusion))
    }

    pub(crate) fn add_exception_region(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
        depth: u32,
        preserve_lasti: bool,
    ) {
        self.exception_regions.push(ExceptionRegion {
            start,
            end,
            target,
            depth,
            preserve_lasti,
        });
    }

    pub(crate) fn emit(&mut self, opcode: Opcode, argument: u32) {
        self.emit_operand(opcode, Operand::Value(argument));
    }

    pub(crate) fn emit_with_depth(
        &mut self,
        opcode: Opcode,
        argument: u32,
        depth_after: u32,
        exception_policy: ExceptionPolicy,
    ) {
        self.emit_operand_with_depth_and_exception_policy(
            opcode,
            Operand::Value(argument),
            Some(depth_after),
            exception_policy,
        );
    }

    pub(crate) fn emit_owned_fast_with_depth(&mut self, argument: u32, depth_after: u32) {
        self.emit_operand_with_depth_and_ownership(
            LOAD_FAST,
            Operand::Value(argument),
            Some(depth_after),
            true,
            ExceptionPolicy::default(),
        );
    }

    pub(crate) fn emit_placeholder_with_depth(
        &mut self,
        opcode: Opcode,
        depth_after: u32,
    ) -> OperandPatchId {
        let id = OperandPatchId(
            u32::try_from(self.operand_patches.len()).expect("too many operand patches"),
        );
        self.operand_patches.push(OperandPatch {
            item_index: u32::try_from(self.linear_items().len())
                .expect("assembler item index should fit in u32"),
            argument: None,
        });
        self.emit_operand_with_depth(opcode, Operand::Deferred(id), Some(depth_after));
        id
    }

    pub(crate) fn patch_argument(
        &mut self,
        id: OperandPatchId,
        argument: u32,
    ) -> Result<(), CompileError> {
        let index = usize::try_from(id.0).expect("operand patch ID should fit in usize");
        let patch = self
            .operand_patches
            .get_mut(index)
            .ok_or_else(|| CompileError::Internal(format!("unknown operand patch {}", id.0)))?;
        if patch.argument.is_some() {
            return Err(CompileError::Internal(format!(
                "operand patch {} was resolved more than once",
                id.0
            )));
        }
        patch.argument = Some(argument);
        Ok(())
    }

    pub(crate) fn resolve_operand_patches(&mut self) -> Result<(), CompileError> {
        if self.operand_patches.is_empty() {
            return Ok(());
        }
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            return Err(CompileError::Internal(
                "operand patches reached the prepared assembler graph".to_string(),
            ));
        };

        for (id, patch) in self.operand_patches.iter().enumerate() {
            let state = if patch.argument.is_some() {
                "resolved"
            } else {
                "unresolved"
            };
            let expected = OperandPatchId(u32::try_from(id).expect("too many operand patches"));
            let item_index = usize::try_from(patch.item_index)
                .expect("assembler item index should fit in usize");
            let Some(Item::Instruction(instruction)) = items.get(item_index) else {
                return Err(CompileError::Internal(format!(
                    "{state} operand patch {id} has no instruction"
                )));
            };
            if !matches!(instruction.operand, Operand::Deferred(id) if id == expected) {
                return Err(CompileError::Internal(format!(
                    "{state} operand patch {id} has no instruction"
                )));
            }
        }

        for (item_index, item) in items.iter_mut().enumerate() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            let Operand::Deferred(id) = instruction.operand else {
                continue;
            };
            let index = usize::try_from(id.0).expect("operand patch ID should fit in usize");
            let Some(patch) = self.operand_patches.get(index) else {
                return Err(CompileError::Internal(format!(
                    "instruction references unknown operand patch {}",
                    id.0
                )));
            };
            if usize::try_from(patch.item_index).expect("assembler item index should fit in usize")
                != item_index
            {
                return Err(CompileError::Internal(format!(
                    "operand patch {} belongs to more than one instruction",
                    id.0
                )));
            }
            let Some(argument) = patch.argument else {
                return Err(CompileError::Internal(format!(
                    "operand patch {} was never resolved",
                    id.0
                )));
            };
            instruction.operand = Operand::Value(argument);
        }

        self.operand_patches.clear();
        Ok(())
    }

    pub(crate) fn used_constant_indices(&mut self, load_const: Opcode) -> FxHashSet<u32> {
        if let UnoptimizedCode::Prepared(graph) = &self.code {
            return reachable_constant_indices(graph, &self.exception_regions, load_const);
        }

        let mut used = FxHashSet::default();
        let mut ended_scope = false;
        let mut straight_line = true;
        for item in self.linear_items() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            if ended_scope || !matches!(instruction.operand, Operand::Value(_)) {
                straight_line = false;
                break;
            }
            if instruction.opcode == load_const
                && let Operand::Value(index) = instruction.operand
            {
                used.insert(index);
            }
            ended_scope = ends_scope(instruction.opcode);
        }
        if straight_line {
            return used;
        }

        let UnoptimizedCode::Linear(items) =
            std::mem::replace(&mut self.code, UnoptimizedCode::Linear(Vec::new()))
        else {
            unreachable!();
        };
        self.code = UnoptimizedCode::Prepared(block_graph::BlockGraph::from_items(
            items,
            &self.preserved_block_boundaries,
        ));
        let UnoptimizedCode::Prepared(graph) = &self.code else {
            unreachable!();
        };
        reachable_constant_indices(graph, &self.exception_regions, load_const)
    }

    pub(crate) fn remap_constant_indices(&mut self, load_const: Opcode, index_map: &[Option<u32>]) {
        let remap = |instruction: &mut Instruction| {
            if instruction.opcode != load_const {
                return;
            }
            let Operand::Value(index) = &mut instruction.operand else {
                unreachable!("LOAD_CONST has a jump operand");
            };
            if let Some(new_index) = index_map[usize::try_from(*index).unwrap()] {
                *index = new_index;
            }
        };
        match &mut self.code {
            UnoptimizedCode::Linear(items) => {
                for item in items {
                    if let Item::Instruction(instruction) = item {
                        remap(instruction);
                    }
                }
            }
            UnoptimizedCode::Prepared(graph) => graph.for_each_instruction_mut(remap),
        }
    }

    /// Removes a side-effect-free constant whose value is immediately discarded.
    pub(crate) fn optimize_constant_pops(&mut self) {
        let is_constant_load =
            |opcode| matches!(opcode, LOAD_CONST | LOAD_COMMON_CONSTANT | LOAD_SMALL_INT);
        let items = self.linear_items_mut();

        for index in 0..items.len().saturating_sub(1) {
            let [Item::Instruction(load), Item::Instruction(pop)] = &mut items[index..index + 2]
            else {
                continue;
            };
            if !is_constant_load(load.opcode) || pop.opcode != POP_TOP {
                continue;
            }
            load.opcode = NOP;
            load.operand = Operand::Value(0);
            pop.opcode = NOP;
            pop.operand = Operand::Value(0);
        }

        let mut label_positions = FxHashMap::default();
        for (index, item) in items.iter().enumerate() {
            if let Item::Label(label) = item {
                label_positions.insert(*label, index);
            }
        }
        let mut replacements = Vec::new();
        for index in 0..items.len().saturating_sub(1) {
            let [Item::Instruction(load), Item::Instruction(jump)] = items[index..index + 2] else {
                continue;
            };
            let Operand::Forward(target) = jump.operand else {
                continue;
            };
            if !is_constant_load(load.opcode) || jump.opcode != JUMP_FORWARD {
                continue;
            }
            let Some(target_position) = label_positions.get(&target).copied() else {
                continue;
            };
            let target = items[target_position + 1..]
                .iter()
                .filter_map(|item| {
                    if let Item::Instruction(instruction) = item {
                        Some(*instruction)
                    } else {
                        None
                    }
                })
                .take(3)
                .collect::<Vec<_>>();
            let [pop, return_load, return_value] = target.as_slice() else {
                continue;
            };
            if pop.opcode == POP_TOP
                && is_constant_load(return_load.opcode)
                && return_value.opcode == RETURN_VALUE
            {
                replacements.push((index, *return_load, *return_value));
            }
        }
        for (index, return_load, return_value) in replacements {
            items[index] = Item::Instruction(return_load);
            items[index + 1] = Item::Instruction(return_value);
        }
    }

    pub(crate) fn emit_forward_with_depth(
        &mut self,
        opcode: Opcode,
        label: Label,
        depth_after: u32,
        exception_policy: ExceptionPolicy,
    ) {
        self.emit_operand_with_depth_and_exception_policy(
            opcode,
            Operand::Forward(label),
            Some(depth_after),
            exception_policy,
        );
    }

    pub(crate) fn emit_backward(&mut self, opcode: Opcode, label: Label) {
        self.emit_operand(opcode, Operand::Backward(label));
    }

    pub(crate) fn emit_backward_with_depth(
        &mut self,
        opcode: Opcode,
        label: Label,
        depth_after: u32,
    ) {
        self.emit_operand_with_depth(opcode, Operand::Backward(label), Some(depth_after));
    }

    fn emit_operand(&mut self, opcode: Opcode, operand: Operand) {
        self.emit_operand_with_depth(opcode, operand, None);
    }

    fn emit_operand_with_depth(
        &mut self,
        opcode: Opcode,
        operand: Operand,
        depth_after: Option<u32>,
    ) {
        self.emit_operand_with_depth_and_exception_policy(
            opcode,
            operand,
            depth_after,
            ExceptionPolicy::default(),
        );
    }

    fn emit_operand_with_depth_and_exception_policy(
        &mut self,
        opcode: Opcode,
        operand: Operand,
        depth_after: Option<u32>,
        exception_policy: ExceptionPolicy,
    ) {
        self.emit_operand_with_depth_and_ownership(
            opcode,
            operand,
            depth_after,
            false,
            exception_policy,
        );
    }

    fn emit_operand_with_depth_and_ownership(
        &mut self,
        opcode: Opcode,
        operand: Operand,
        depth_after: Option<u32>,
        explicitly_owned: bool,
        exception_policy: ExceptionPolicy,
    ) {
        let strict_owned_load = self.strict_owned_loads && opcode == LOAD_FAST;
        let force_owned_load = explicitly_owned
            || strict_owned_load
            || (opcode == LOAD_FAST && !self.load_fast_borrowing_enabled);
        let mut instruction = Instruction::new(opcode, operand, self.location, depth_after);
        instruction.exception_policy = exception_policy;
        instruction.load_policy = LoadPolicy {
            force_owned: force_owned_load,
            strict_owned: strict_owned_load,
        };
        self.linear_items_mut().push(Item::Instruction(instruction));
    }
}

fn opcode_stack_effect(opcode: Opcode, argument: u32) -> i64 {
    i64::try_from(opcode_num_pushed(opcode, argument)).unwrap()
        - i64::try_from(opcode_num_popped(opcode, argument)).unwrap()
}

fn block_has_fallthrough(block: &[Item]) -> bool {
    let Some(instruction) = block.iter().rev().find_map(|item| {
        if let Item::Instruction(instruction) = item {
            Some(instruction)
        } else {
            None
        }
    }) else {
        return true;
    };
    !ends_scope(instruction.opcode) && !is_unconditional_jump(instruction.opcode)
}

fn cleanup_block_creation_order(block: &[Item]) -> u32 {
    block
        .iter()
        .filter_map(|item| {
            let Item::Label(label) = item else {
                return None;
            };
            Some(label.0)
        })
        .min()
        .expect("cleanup block has a label")
}

fn block_jump_target(block: &[Item]) -> Option<Label> {
    let instruction = block.iter().rev().find_map(|item| {
        if let Item::Instruction(instruction) = item {
            Some(instruction)
        } else {
            None
        }
    })?;
    match instruction.operand {
        Operand::Forward(target) | Operand::Backward(target) => Some(target),
        Operand::Value(_) | Operand::Deferred(_) => None,
    }
}

fn extended_arg_count(argument: u32) -> u8 {
    match argument {
        0..=0xff => 0,
        0x100..=0xffff => 1,
        0x1_0000..=0xff_ffff => 2,
        _ => 3,
    }
}
