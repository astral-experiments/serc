mod block_graph;
mod exception_owners;
mod ir;
mod layout;
mod passes;

#[cfg(test)]
mod tests;

use rustc_hash::FxHashMap;

use crate::CompileError;
use crate::code_object::Constant;
use crate::target::opcodes::{
    CHECK_EXC_MATCH, COMPARE_OP, CONTAINS_OP, COPY, COPY_FREE_VARS, DELETE_DEREF, DELETE_FAST,
    DICT_MERGE, DICT_UPDATE, END_SEND, EXTENDED_ARG, FOR_ITER, FORMAT_SIMPLE, GET_ANEXT, GET_LEN,
    GET_YIELD_FROM_ITER, IMPORT_FROM, IS_OP, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT,
    JUMP_FORWARD, LIST_APPEND, LIST_EXTEND, LOAD_ATTR, LOAD_COMMON_CONSTANT, LOAD_CONST, LOAD_FAST,
    LOAD_FAST_AND_CLEAR, LOAD_FAST_BORROW, LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_CHECK,
    LOAD_FAST_LOAD_FAST, LOAD_GLOBAL, LOAD_SMALL_INT, LOAD_SPECIAL, LOAD_SUPER_ATTR, MAKE_CELL,
    MAP_ADD, MATCH_KEYS, MATCH_MAPPING, MATCH_SEQUENCE, NOP, NOT_TAKEN, POP_EXCEPT,
    POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, POP_TOP,
    PUSH_EXC_INFO, RAISE_VARARGS, RERAISE, RESUME, RETURN_VALUE, SEND, SET_ADD,
    SET_FUNCTION_ATTRIBUTE, SET_UPDATE, STORE_DEREF, STORE_FAST, STORE_FAST_LOAD_FAST,
    STORE_FAST_STORE_FAST, STORE_GLOBAL, SWAP, TO_BOOL, UNARY_NOT, UNPACK_SEQUENCE,
    WITH_EXCEPT_START,
};
use crate::target::operands::ResumeLocation;
use crate::target::{
    Opcode, is_conditional_jump, is_scope_exit as ends_scope, is_unconditional_jump,
    stack_counts as opcode_stack_counts,
};

#[cfg(any(test, debug_assertions))]
use ir::AssemblerStage;
pub(crate) use ir::{
    AssembledCode, Assembler, EmittedInstruction, EmittedJump, ExceptionPolicy, Label,
    OperandPatchId, SourceLocation,
};
use ir::{
    BorrowFallthrough, BorrowLoopExitPolicy, ExceptionExclusion, ExceptionRegion,
    ExceptionRegionPriority, Instruction, Item, LayoutAssembler, LocalPolicy,
    NormalizedExceptionOwner, Operand, OperandPatch, OptimizationPipeline, UnoptimizedCode,
};

/// Constant-table operations needed by optimizations that follow exit-block duplication.
pub(crate) trait ConstantTable {
    fn get(&self, index: u32) -> &Constant;
    fn intern(&mut self, constant: Constant) -> Result<u32, CompileError>;
    fn len(&self) -> usize;
    fn retain_used(&mut self, used: &[bool], order: &[u32]) -> Vec<Option<u32>>;
}

/// Source bookkeeping retained when lowering removes a trailing NOP.
pub(crate) struct RemovedNop {
    pub(crate) location: SourceLocation,
    pub(crate) exclusion: Option<(Label, Label)>,
    pub(crate) original_slots: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BooleanValueMarkerScan {
    cursor_item: usize,
    previous_opcode: Option<Opcode>,
    seen_marker: bool,
    blocked: bool,
}

impl BooleanValueMarkerScan {
    pub(crate) const fn new(cursor_item: usize) -> Self {
        Self {
            cursor_item,
            previous_opcode: None,
            seen_marker: false,
            blocked: false,
        }
    }
}

fn reachable_constant_indices(
    graph: &block_graph::BlockGraph,
    reachable: &[bool],
    load_const: Opcode,
    constant_count: usize,
) -> Vec<bool> {
    let mut used = vec![false; constant_count];
    let mut remaining = constant_count.saturating_sub(1);
    for (block, contents) in graph.iter() {
        if !reachable[block.index()] {
            continue;
        }
        for item in contents.items() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            if instruction.opcode == load_const
                && let Operand::Value(index) = instruction.operand()
                && let Some(slot) = used.get_mut(index as usize)
            {
                let newly_used = !std::mem::replace(slot, true);
                if index != 0 && newly_used {
                    remaining -= 1;
                    if remaining == 0 {
                        return used;
                    }
                }
            }
        }
    }
    used
}

/// Marks reachable constants in an assertion-only stream and reports linear-optimizer eligibility.
///
/// `None` rejects the grammar; `Some(false)` preserves valid streams that still require the CFG.
fn assertion_constant_indices(
    items: &[Item],
    load_const: Opcode,
    used: &mut [bool],
) -> Option<bool> {
    let mut items = items.iter();
    let mut assertion_count = 0;
    let mut ended_scope = false;
    let mut linear_eligible = true;

    while let Some(item) = items.next() {
        if ended_scope {
            return None;
        }
        let Item::Instruction(instruction) = item else {
            return None;
        };
        linear_eligible &= instruction.opcode != SWAP
            && !instruction.jump_policy.borrow_empty_block()
            && instruction.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Barrier;
        if matches!(
            instruction.opcode,
            POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
        ) {
            let Operand::Forward(target) = instruction.operand() else {
                return None;
            };
            let Some(Item::Instruction(not_taken)) = items.next() else {
                return None;
            };
            let Some(Item::Instruction(assertion_error)) = items.next() else {
                return None;
            };
            let Some(Item::Instruction(raise)) = items.next() else {
                return None;
            };
            let Some(Item::Label(label)) = items.next() else {
                return None;
            };
            if not_taken.opcode != NOT_TAKEN
                || !matches!(not_taken.operand(), Operand::Value(0))
                || assertion_error.opcode != LOAD_COMMON_CONSTANT
                || !matches!(assertion_error.operand(), Operand::Value(0))
                || raise.opcode != RAISE_VARARGS
                || !matches!(raise.operand(), Operand::Value(1))
                || *label != target
            {
                return None;
            }
            linear_eligible &= !not_taken.jump_policy.borrow_empty_block()
                && !assertion_error.jump_policy.borrow_empty_block()
                && !raise.jump_policy.borrow_empty_block();
            assertion_count += 1;
            continue;
        }
        let Operand::Value(argument) = instruction.operand() else {
            return None;
        };
        if instruction.opcode == NOT_TAKEN {
            return None;
        }
        if instruction.opcode == load_const
            && let Some(slot) = used.get_mut(argument as usize)
        {
            *slot = true;
        }
        ended_scope = ends_scope(instruction.opcode);
    }

    (assertion_count > 0 && ended_scope).then_some(linear_eligible)
}

impl Assembler {
    fn linear_items(&self) -> &[Item] {
        let UnoptimizedCode::Linear(items) = &self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        items
    }

    fn linear_items_mut(&mut self) -> &mut [Item] {
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        items
    }

    pub(crate) fn reserve_items(&mut self, additional: usize) {
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        items.reserve(additional);
    }

    pub(in crate::assembler) fn has_only_linear_assertion_side_exits(&self) -> bool {
        if self.has_proven_linear_assertion_side_exits {
            return true;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return false;
        };
        self.has_assertion_side_exit_candidate
            && self.exception_regions.is_empty()
            && self.preserved_block_boundaries.is_empty()
            && self.borrow_unreachable_blocks.is_empty()
            && !self.has_synthesized_percent_target_locations
            && assertion_constant_indices(items, LOAD_CONST, &mut []).unwrap_or(false)
    }

    fn push_linear_item(&mut self, item: Item) {
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        if matches!(item, Item::Instruction(_)) {
            self.instruction_count += 1;
        }
        items.push(item);
    }

    #[cfg(test)]
    fn from_linear_items(items: Vec<Item>) -> Self {
        let mut assembler = Self::default();
        for item in items {
            assembler.push_linear_item(item);
        }
        assembler
    }

    #[cfg(test)]
    fn pop_linear_item(&mut self) -> Option<Item> {
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        let item = items.pop()?;
        if matches!(item, Item::Instruction(_)) {
            self.instruction_count -= 1;
        }
        Some(item)
    }

    #[cfg(test)]
    fn clear_linear_items(&mut self) {
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        items.clear();
        self.instruction_count = 0;
    }

    pub(crate) fn disable_load_fast_borrowing(&mut self) {
        self.load_fast_borrowing_enabled = false;
    }

    /// Prevents local loads emitted while enabled from becoming borrowed loads.
    pub(crate) fn set_strict_owned_loads(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.strict_owned_loads, enabled)
    }

    pub(crate) fn set_forced_borrowed_loads(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.forced_borrowed_loads, enabled)
    }

    pub(crate) fn location(&self) -> SourceLocation {
        self.location
    }

    pub(crate) fn set_location(&mut self, location: SourceLocation) {
        self.location = location;
    }

    /// Restores an unlocated Boolean exit while retaining any early-folded location fallback.
    pub(crate) fn prepare_loop_tail_discard(&mut self, preserve_folded_location: bool) {
        let instruction = self.required_last_instruction_mut("preparing a loop-tail discard");
        debug_assert_eq!(instruction.opcode, POP_TOP);
        if !preserve_folded_location {
            instruction.location = SourceLocation::NONE;
            return;
        }

        // Only the existing join labels identify this originally unlocated discard.
        // A fully folded Boolean has no join and does not need exit-copy provenance.
        let items = self.linear_items();
        let index = items
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .expect("the loop-tail discard exists");
        let labels = items[..index]
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        self.original_unlocated_discard_targets.extend(labels);
    }

    pub(crate) fn last_instruction_location(&self) -> Option<SourceLocation> {
        let items = self.linear_items();
        let index = items
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))?;
        let Item::Instruction(instruction) = items[index] else {
            unreachable!("instruction index points to a label");
        };
        Some(instruction.location)
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
        let index = self
            .linear_items()
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))?;
        let Item::Instruction(instruction) = &mut self.linear_items_mut()[index] else {
            unreachable!("instruction index points to a label");
        };
        Some(instruction)
    }

    fn required_last_instruction_mut(&mut self, operation: &str) -> &mut Instruction {
        self.last_instruction_mut()
            .unwrap_or_else(|| panic!("{operation} requires a preceding instruction"))
    }

    fn last_jump_instruction_mut(&mut self, operation: &str) -> &mut Instruction {
        let instruction = self.required_last_instruction_mut(operation);
        assert!(
            matches!(
                instruction.operand(),
                Operand::Forward(_) | Operand::Backward(_)
            ),
            "{operation} requires a jump instruction"
        );
        instruction
    }

    fn emitted_instruction_mut(
        &mut self,
        emitted: EmittedInstruction,
        operation: &str,
    ) -> &mut Instruction {
        assert_eq!(
            emitted.stream, self.emission_stream,
            "{operation} received a handle from a different assembler"
        );
        let index = usize::try_from(emitted.item_index)
            .expect("emitted instruction index should fit in usize");
        assert_eq!(
            self.linear_items()
                .iter()
                .rposition(|item| matches!(item, Item::Instruction(_))),
            Some(index),
            "{operation} received a stale emitted-instruction handle"
        );
        let items = self.linear_items_mut();
        let Some(Item::Instruction(instruction)) = items.get_mut(index) else {
            panic!("{operation} received a stale emitted-instruction handle");
        };
        instruction
    }

    fn emitted_jump_mut(&mut self, emitted: EmittedJump, operation: &str) -> &mut Instruction {
        let instruction = self.emitted_instruction_mut(emitted.instruction(), operation);
        assert!(
            matches!(
                instruction.operand(),
                Operand::Forward(_) | Operand::Backward(_)
            ),
            "{operation} requires an emitted jump"
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
                                instruction.operand(),
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

    /// Whether the current original control-flow block has no emitted instructions.
    pub(crate) fn at_empty_borrow_block(&self) -> bool {
        for item in self.linear_items().iter().rev() {
            match item {
                Item::Label(label) => {
                    if self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(label)
                        && !self.borrow_block_aliases.contains(label)
                    {
                        return self.instruction_count() > 0;
                    }
                }
                Item::Instruction(instruction) => {
                    return instruction.opcode == NOT_TAKEN
                        || matches!(
                            instruction.operand(),
                            Operand::Forward(_) | Operand::Backward(_)
                        )
                        || ends_scope(instruction.opcode);
                }
            }
        }
        // The entry prologue keeps its original block nonempty.
        false
    }

    /// Returns the preceding instruction's location within the current fallthrough block.
    pub(crate) fn current_block_last_instruction_location(&self) -> Option<SourceLocation> {
        self.linear_items()[self.current_block_start()..]
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location),
                Item::Label(_) => None,
            })
    }

    /// Whether the next instruction starts a block with an emitted incoming jump.
    pub(crate) fn at_jump_target(&self) -> bool {
        let items = self.linear_items();
        let labels = items
            .iter()
            .rev()
            .take_while(|item| matches!(item, Item::Label(_)))
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        !labels.is_empty()
            && items.iter().any(|item| {
                matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if labels.contains(&label)))
            })
    }

    /// Whether the latest scope exit has no explicit incoming edge at the current position.
    pub(crate) fn has_terminated_fallthrough(&self) -> bool {
        !self.at_jump_target()
            && self
                .linear_items()
                .iter()
                .rev()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(ends_scope(instruction.opcode)),
                    Item::Label(_) => None,
                })
                .unwrap_or(false)
    }

    /// Whether multiple emitted branches enter a label before jump threading.
    pub(crate) fn has_multiple_jump_sources(&self, label: Label) -> bool {
        self.linear_items()
            .iter()
            .filter(|item| {
                matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                        if target == label))
            })
            .take(2)
            .count()
            > 1
    }

    pub(crate) fn instruction_count(&self) -> usize {
        let _ = self.linear_items();
        self.instruction_count
    }

    pub(crate) fn item_count(&self) -> usize {
        self.linear_items().len()
    }

    pub(crate) fn instruction_opcode_at(&self, instruction_offset: usize) -> Option<Opcode> {
        self.linear_items()
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.opcode),
                Item::Label(_) => None,
            })
            .nth(instruction_offset)
    }

    pub(crate) fn instruction_location_at(
        &self,
        instruction_offset: usize,
    ) -> Option<SourceLocation> {
        self.linear_items()
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.location),
                Item::Label(_) => None,
            })
            .nth(instruction_offset)
    }

    /// Counts emitted instructions before constant folding and global-call fusion.
    pub(crate) fn pre_fusion_instruction_count_since(&self, instruction_offset: usize) -> usize {
        self.linear_items()
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .skip(instruction_offset)
            .map(|instruction| {
                1 + usize::from(instruction.pre_fold_extra_instructions())
                    + usize::from(instruction.opcode == LOAD_GLOBAL
                        && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0))
            })
            .sum()
    }

    /// Marks an emitted instruction as ownerless when normalization does not reuse a stale slot.
    pub(crate) fn exclude_normalized_exception_owner_at(&mut self, instruction_offset: usize) {
        let instruction = self
            .linear_items_mut()
            .iter_mut()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .nth(instruction_offset)
            .expect("instruction offset should reference an emitted instruction");
        instruction.set_exception_owner(NormalizedExceptionOwner::Absent);
    }

    pub(crate) fn contains_opcode(&self, opcode: Opcode) -> bool {
        self.linear_items().iter().any(
            |item| matches!(item, Item::Instruction(instruction) if instruction.opcode == opcode),
        )
    }

    /// Returns whether a normalized Boolean value marker was already emitted in this code unit.
    pub(crate) fn contains_boolean_value_marker(&self) -> bool {
        let mut previous = None;
        for instruction in self.linear_items().iter().filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        }) {
            if previous == Some(NOT_TAKEN) && instruction.opcode == POP_TOP {
                return true;
            }
            previous = Some(instruction.opcode);
        }
        false
    }

    /// Advances a Boolean-value marker scan over newly emitted items.
    pub(crate) fn advance_boolean_value_marker_scan(
        &self,
        scan: &mut BooleanValueMarkerScan,
    ) -> bool {
        if scan.blocked {
            return false;
        }
        let items = self.linear_items();
        for instruction in items[scan.cursor_item..]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
        {
            if matches!(
                instruction.opcode,
                LOAD_FAST_AND_CLEAR | FOR_ITER | PUSH_EXC_INFO
            ) {
                scan.blocked = true;
                break;
            }
            if scan.previous_opcode == Some(NOT_TAKEN) && instruction.opcode == POP_TOP {
                scan.seen_marker = true;
            }
            scan.previous_opcode = Some(instruction.opcode);
        }
        scan.cursor_item = items.len();
        scan.seen_marker && !scan.blocked
    }

    pub(crate) fn contains_opcode_since(&self, opcode: Opcode, instruction_offset: usize) -> bool {
        self.linear_items()
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .skip(instruction_offset)
            .any(|instruction| instruction.opcode == opcode)
    }

    /// Returns whether `opcode` was emitted since the captured linear-item offset.
    pub(crate) fn contains_opcode_since_item(&self, opcode: Opcode, item_offset: usize) -> bool {
        self.linear_items().iter().skip(item_offset).any(
            |item| matches!(item, Item::Instruction(instruction) if instruction.opcode == opcode),
        )
    }

    /// Returns whether a terminal synchronous-with owner barrier was emitted since `item_offset`.
    pub(crate) fn contains_terminal_with_barrier_since_item(&self, item_offset: usize) -> bool {
        self.linear_items().iter().skip(item_offset).any(|item| {
            matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.opcode,
                        LOAD_FAST_AND_CLEAR | NOT_TAKEN | FOR_ITER | PUSH_EXC_INFO))
        })
    }

    /// Returns whether an iterator-tail condition-owner barrier was emitted since `item_offset`.
    pub(crate) fn contains_iterator_tail_barrier_since_item(&self, item_offset: usize) -> bool {
        self.linear_items().iter().skip(item_offset).any(|item| {
            matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.opcode, NOT_TAKEN | SEND | FOR_ITER | PUSH_EXC_INFO))
        })
    }

    /// Returns how many instruction slots optimization can vacate before a normalized marker.
    ///
    /// CPython compacts instructions without clearing their exception-handler fields. Later
    /// insertions can reuse those stale slots, so normalized instructions may acquire an owner. A
    /// fallthrough scan crosses labels that only delimit exception regions, matching CPython's
    /// pseudo-instruction representation of those boundaries.
    fn dynamic_exception_owner_slots(
        &self,
        cross_labels: bool,
        include_conservative_region_vacancy: bool,
    ) -> usize {
        fn will_remove_static_swap(
            instruction: Instruction,
            following: &[Item],
            fusion_boundaries: &rustc_hash::FxHashSet<Label>,
        ) -> bool {
            if instruction.opcode != SWAP {
                return false;
            }
            let Operand::Value(depth) = instruction.operand() else {
                return false;
            };
            let mut consumers = following
                .iter()
                .filter(
                    |item| !matches!(item, Item::Label(label) if fusion_boundaries.contains(label)),
                )
                .map(|item| match item {
                    Item::Instruction(instruction) => Some(*instruction),
                    Item::Label(_) => None,
                });
            let mut endpoints = None;
            let mut consumed = 0;
            for consumer in &mut consumers {
                let Some(consumer) = consumer else {
                    return false;
                };
                if endpoints.is_some_and(|(first, _): (Instruction, Instruction)| {
                    first.location.line >= 0 && consumer.location.line != first.location.line
                }) {
                    return false;
                }
                if consumer.opcode == NOP {
                    continue;
                }
                if !matches!(consumer.opcode, STORE_FAST | POP_TOP) {
                    return false;
                }
                endpoints = Some((endpoints.map_or(consumer, |(first, _)| first), consumer));
                consumed += 1;
                if consumed == depth {
                    break;
                }
            }
            let Some((first, last)) = endpoints.filter(|_| consumed == depth) else {
                return false;
            };
            let stored = |instruction: Instruction| {
                (instruction.opcode == STORE_FAST).then_some(instruction.operand())
            };
            let first_store = stored(first);
            let last_store = stored(last);
            if first_store == last_store {
                return false;
            }
            // CPython rejects a permutation that changes which value a repeated store keeps.
            !following
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) if instruction.opcode != NOP => {
                        Some(*instruction)
                    }
                    _ => None,
                })
                .take(depth as usize)
                .skip(1)
                .take(depth.saturating_sub(2) as usize)
                .any(|instruction| {
                    stored(instruction).is_some_and(|local| {
                        Some(local) == first_store || Some(local) == last_store
                    })
                })
        }

        fn will_fuse(first: Instruction, second: Instruction) -> bool {
            if first.fusion_policy.prevents_with_next()
                || first.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
                || second.fusion_policy.prevents_with_previous()
            {
                return false;
            }
            let (Operand::Value(first_argument), Operand::Value(second_argument)) =
                (first.operand(), second.operand())
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
            block_labels.extend(&self.exception_owner_provenance.block_boundaries);
            for region in &self.exception_regions {
                block_labels.insert(region.target);
            }
            for item in items {
                if let Item::Instruction(instruction) = item
                    && let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                {
                    block_labels.insert(label);
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
                                    instruction.operand(),
                                    Operand::Forward(_) | Operand::Backward(_)
                                ) || ends_scope(instruction.opcode)))
                    }
                })
                .map_or(0, |index| index + 1)
        } else {
            items
                .iter()
                .rposition(|item| match item {
                    Item::Label(label) => !self
                        .exception_owner_provenance
                        .fusion_boundaries
                        .contains(label),
                    Item::Instruction(instruction) => {
                        instruction.opcode == NOT_TAKEN
                            || (!is_conditional_jump(instruction.opcode)
                                && (matches!(
                                    instruction.operand(),
                                    Operand::Forward(_) | Operand::Backward(_)
                                ) || ends_scope(instruction.opcode)))
                    }
                })
                .map_or(0, |index| index + 1)
        };
        let mut slots = 0;
        let mut late_inserted_slots = 0;
        let mut nop_count = 0;
        let mut index = start;
        while index < items.len() {
            let instruction = match items[index] {
                Item::Instruction(instruction) => instruction,
                Item::Label(label)
                    if self
                        .exception_owner_provenance
                        .fusion_boundaries
                        .contains(&label)
                        || block_labels
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
            slots += usize::from(instruction.local_policy.hidden_exception_owner_slots());
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
                    && following.fusion_policy.prevents_with_previous()
                    && matches!(instruction.opcode, POP_TOP | STORE_FAST)
                    && !(instruction.opcode == STORE_FAST
                        && instruction.fusion_policy.prevents_with_previous())
            });
            if matches!(
                instruction.opcode,
                LOAD_FAST_BORROW_LOAD_FAST_BORROW
                    | LOAD_FAST_LOAD_FAST
                    | STORE_FAST_LOAD_FAST
                    | STORE_FAST_STORE_FAST
            ) || matches!(
                (instruction.opcode, instruction.operand()),
                (LOAD_GLOBAL, Operand::Value(argument)) if argument & 1 != 0
            ) || matches!(
                (instruction.opcode, instruction.operand()),
                (COMPARE_OP, Operand::Value(argument)) if argument & 16 != 0
            ) || instruction.fusion_policy.is_converted_pop_block()
                || removable_nop
                || applied_comprehension_swap
                || will_remove_static_swap(
                    instruction,
                    &items[index + 1..],
                    &self.exception_owner_provenance.fusion_boundaries,
                )
            {
                slots += 1;
                index += 1;
                continue;
            }
            if following.is_some_and(|following| will_fuse(instruction, following)) {
                slots += 1;
                slots += following.map_or(0, |following| {
                    usize::from(following.local_policy.hidden_exception_owner_slots())
                });
                index += 2;
                continue;
            }
            index += 1;
        }
        // Late prologue insertions consume stale slots instead of creating them.
        // COPY_FREE_VARS and MAKE_CELL are the two such insertions relevant here.
        let slots = slots.saturating_sub(late_inserted_slots);
        if cross_labels && include_conservative_region_vacancy && nop_count >= 2 {
            // Consecutive protected regions can leave a stale slot even when their removed pseudo
            // instructions are absent from our IR.
            slots.max(1)
        } else {
            slots
        }
    }

    /// Requires an original entry before the current span, without a later region transition.
    pub(crate) fn current_block_starts_at_source_boundary(&self) -> bool {
        let mut boundary = false;
        let mut opaque_boundary = false;
        let mut setup_nop = None;
        for item in self.linear_items().iter().rev() {
            match item {
                Item::Label(label)
                    if self
                        .exception_owner_provenance
                        .fusion_boundaries
                        .contains(label) => {}
                Item::Label(label) => {
                    boundary = true;
                    if self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(label)
                    {
                        return true;
                    }
                    if let Some((_, Some(location))) = self
                        .exception_owner_provenance
                        .setups
                        .iter()
                        .find(|(setup, _)| setup == label)
                    {
                        if setup_nop.replace(*location).is_some() {
                            return false;
                        }
                    } else if !self
                        .exception_owner_provenance
                        .setup_nop_boundaries
                        .contains(label)
                    {
                        opaque_boundary = true;
                    }
                }
                Item::Instruction(instruction) => {
                    // A retained SETUP occupies an original slot but starts no new block.
                    // Only its authored exclusion labels may separate it from the entry.
                    if boundary
                        && !opaque_boundary
                        && instruction.opcode == NOP
                        && setup_nop.take() == Some(instruction.location)
                    {
                        boundary = false;
                        continue;
                    }
                    if boundary
                        || instruction.opcode == NOT_TAKEN
                        || (!is_conditional_jump(instruction.opcode)
                            && (matches!(
                                instruction.operand(),
                                Operand::Forward(_) | Operand::Backward(_)
                            ) || ends_scope(instruction.opcode)))
                    {
                        return false;
                    }
                }
            }
        }
        false
    }

    pub(crate) fn current_block_dynamic_exception_owner_slots(&self) -> usize {
        self.dynamic_exception_owner_slots(false, false)
    }

    /// Returns whether optimization can leave a dynamic exception owner in the current block.
    pub(crate) fn current_block_has_dynamic_exception_owner(&self) -> bool {
        self.current_block_dynamic_exception_owner_slots() > 0
    }

    /// Returns how many stale exception-owner slots can cross labels that only delimit regions.
    pub(crate) fn fallthrough_dynamic_exception_owner_slots(&self) -> usize {
        self.dynamic_exception_owner_slots(true, true)
    }

    /// Returns whether a concrete stale owner slot crosses only exception-region labels.
    ///
    /// Unlike the ordinary fallthrough query, this ignores vacancies inferred solely from
    /// consecutive protected regions whose removed pseudo instructions are absent from our IR.
    pub(crate) fn fallthrough_has_concrete_dynamic_exception_owner(&self) -> bool {
        self.dynamic_exception_owner_slots(true, false) > 0
    }

    /// Certifies a straight entry prefix whose original SETUP slots all survive as NOPs.
    ///
    /// This permits a concrete vacancy query to bypass the conservative multiple-region
    /// fallback. Unrecorded NOPs, region transitions, and deleted original records remain
    /// unknown; the caller must still check for concrete vacancies such as load fusion.
    pub(crate) fn entry_has_retained_exception_setups(&self) -> bool {
        if !self.exception_regions.is_empty() {
            return false;
        }
        let items = self.linear_items();
        let Some(Item::Instruction(last)) = items.last() else {
            return false;
        };
        if !is_conditional_jump(last.opcode) {
            return false;
        }
        let mut resumed = false;
        let mut pending_nop = None;
        let mut previous = None;
        let mut retained_setups = 0;
        for (index, item) in items.iter().enumerate() {
            match item {
                Item::Label(label) => {
                    if self.preserved_block_boundaries.contains(label)
                        || self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                    {
                        return false;
                    }
                    if let Some((_, location)) = self
                        .exception_owner_provenance
                        .setups
                        .iter()
                        .find(|(setup, _)| setup == label)
                    {
                        if location.is_none() || pending_nop.take() != *location {
                            return false;
                        }
                        retained_setups += 1;
                    } else if !self
                        .exception_owner_provenance
                        .setup_nop_boundaries
                        .contains(label)
                        && !self
                            .exception_owner_provenance
                            .fusion_boundaries
                            .contains(label)
                    {
                        return false;
                    }
                }
                Item::Instruction(instruction) => {
                    let previous = previous.replace(*instruction);
                    if pending_nop.is_some()
                        || instruction.fusion_policy.is_converted_pop_block()
                        || instruction.local_policy.hidden_exception_owner_slots() != 0
                        || instruction.pre_fold_extra_instructions() != 0
                        || previous.is_some_and(|previous: Instruction| {
                            previous.opcode == NOP
                                && (instruction.location.line < 0
                                    || previous.location.line == instruction.location.line)
                        })
                    {
                        return false;
                    }
                    if !resumed {
                        if matches!(instruction.opcode, COPY_FREE_VARS | MAKE_CELL) {
                            continue;
                        }
                        if instruction.opcode != RESUME
                            || !matches!(instruction.operand(), Operand::Value(0))
                        {
                            return false;
                        }
                        resumed = true;
                        continue;
                    }
                    if matches!(instruction.opcode, RESUME | COPY_FREE_VARS | MAKE_CELL)
                        || (index + 1 != items.len()
                            && (is_conditional_jump(instruction.opcode)
                                || !matches!(instruction.operand(), Operand::Value(_))
                                || ends_scope(instruction.opcode)))
                        || matches!(
                            instruction.opcode,
                            NOT_TAKEN | FOR_ITER | SEND | POP_EXCEPT | PUSH_EXC_INFO
                        )
                    {
                        return false;
                    }
                    if instruction.opcode == NOP {
                        if instruction.location.line < 0
                            || previous.is_some_and(|previous| {
                                previous.location.line == instruction.location.line
                            })
                        {
                            return false;
                        }
                        pending_nop = Some(instruction.location);
                    }
                }
            }
        }
        resumed && pending_nop.is_none() && retained_setups >= 2
    }

    /// Returns whether optimization can leave a dynamic exception owner before a normalized
    /// condition marker, including across labels that only delimit exception regions.
    ///
    /// CPython represents an exception-region start with a pseudo instruction rather than a CFG
    /// label, so compaction before a try body can shift stale owners into a marker appended
    /// inside the body.
    pub(crate) fn fallthrough_has_dynamic_exception_owner(&self) -> bool {
        self.fallthrough_dynamic_exception_owner_slots() > 0
    }

    /// Returns whether the current block resumes directly after a plain `yield`.
    pub(crate) fn current_block_resumes_after_yield(&self) -> bool {
        for item in self.linear_items().iter().rev() {
            let Item::Instruction(instruction) = item else {
                break;
            };
            if instruction.opcode == RESUME {
                return matches!(
                    instruction.operand(),
                    Operand::Value(argument)
                        if argument & 3 == ResumeLocation::AFTER_YIELD.argument()
                );
            }
            if instruction.opcode == NOT_TAKEN
                || (!is_conditional_jump(instruction.opcode)
                    && (matches!(
                        instruction.operand(),
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
        self.push_linear_item(Item::Label(label));
    }

    /// Retains the original NOP head when statement lowering omits the instruction.
    pub(crate) fn record_omitted_nop_targets(&mut self) {
        let labels = self
            .linear_items()
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        self.original_nop_targets.extend(labels);
    }

    /// Retains the artificial location of normal handler cleanup before propagation.
    pub(crate) fn record_unlocated_cleanup_targets(&mut self) {
        let labels = self
            .linear_items()
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        self.original_unlocated_cleanup_targets.extend(labels);
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

    /// Preserve an optimization boundary that aliases an existing original CFG block.
    pub(crate) fn preserve_block_boundary_alias(&mut self, label: Label) {
        self.preserve_block_boundary(label);
        self.borrow_block_aliases.insert(label);
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

    /// Preserves empty original loop headers only when their leading NOPs are erased.
    pub(crate) fn mark_initial_nops_borrow_barrier_if_removed(&mut self, label: Label) {
        let start = self
            .linear_items()
            .iter()
            .rposition(|item| matches!(item, Item::Label(candidate) if *candidate == label))
            .expect("initial NOP marker requires an emitted label");
        for item in &mut self.linear_items_mut()[start + 1..] {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            if instruction.opcode != NOP {
                break;
            }
            if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Reachable {
                instruction
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved);
            }
        }
    }

    /// Stops disconnected break fallthrough while retaining independent entries to the loop exit.
    pub(crate) fn prevent_borrow_loop_exit_reachability(&mut self, label: Label) {
        self.prevent_borrow_reachability(label);
        self.borrow_loop_exit_blocks
            .insert(label, BorrowLoopExitPolicy::default());
    }

    /// Keeps surviving break jumps into a loop's nonempty continuation reachable.
    pub(crate) fn allow_borrow_surviving_break_entry(&mut self, label: Label) {
        self.borrow_loop_exit_blocks
            .get_mut(&label)
            .expect("surviving break entry requires a loop exit marker")
            .allow_surviving_break = true;
    }

    /// Retains borrowing in a small exit that CPython may copy onto a protected break.
    pub(crate) fn preserve_borrow_small_exit_entry(&mut self, label: Label) {
        self.borrow_loop_exit_blocks
            .get_mut(&label)
            .expect("small exit entry requires a loop exit marker")
            .preserve_small_exit = true;
    }

    /// Stops the final borrowed-load analysis at the last emitted instruction.
    pub(crate) fn mark_last_borrow_fallthrough_barrier(&mut self) {
        self.last_instruction_mut()
            .expect("borrow fallthrough barrier requires a preceding instruction")
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    }

    /// Keeps a folded condition's empty block only when its located NOP is removed.
    pub(crate) fn mark_last_nop_borrow_barrier_if_removed(&mut self) {
        let instruction = self.required_last_instruction_mut("marking a removable NOP barrier");
        assert_eq!(instruction.opcode, NOP);
        if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Reachable {
            instruction
                .no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::BarrierIfRemoved);
        }
    }

    /// Stops borrowed-load analysis at a specifically identified emitted instruction.
    pub(crate) fn mark_borrow_fallthrough_barrier(&mut self, emitted: EmittedInstruction) {
        self.emitted_instruction_mut(emitted, "marking a borrow fallthrough barrier")
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
    }

    /// Keeps borrowed-load traversal connected through a retained redundant-jump NOP.
    pub(crate) fn mark_retained_nop_borrow_fallthrough(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "marking retained NOP borrow fallthrough")
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
    }

    /// Connects a retained NOP without overriding an explicit borrowed-load barrier.
    pub(crate) fn mark_retained_nop_borrow_fallthrough_if_unblocked(
        &mut self,
        emitted: EmittedJump,
    ) {
        let instruction =
            self.emitted_jump_mut(emitted, "marking unblocked retained NOP borrow fallthrough");
        if instruction.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Barrier {
            instruction
                .no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::RetainedNopReachable);
        }
    }

    /// Retains a redundant jump as an analysis-only marker for CPython's empty CFG block.
    pub(crate) fn mark_borrow_empty_block(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "marking an empty borrowed-load block")
            .jump_policy
            .mark_borrow_empty_block();
    }

    /// Records a folded condition's synthetic exit to its newly allocated, unique join.
    pub(crate) fn mark_folded_condition_exit(&mut self, emitted: EmittedJump) {
        let instruction = self.emitted_jump_mut(emitted, "marking a folded condition exit");
        let Operand::Forward(target) = instruction.operand() else {
            panic!("a folded condition exit must have a forward target");
        };
        instruction.jump_policy.mark_borrow_empty_block();
        self.folded_condition_exit_targets.insert(target);
    }

    /// Marks an empty borrowed-load block without overriding retained-NOP policies.
    pub(crate) fn mark_borrow_empty_block_if_unretained(&mut self, emitted: EmittedJump) {
        let instruction = self.emitted_jump_mut(emitted, "marking an unretained empty block");
        if !instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
            && !instruction.jump_policy.preserves_nop_after_threading()
            && !instruction.jump_policy.preserves_explicit_nop()
        {
            instruction.jump_policy.mark_borrow_empty_block();
        }
    }

    /// Marks a previously emitted jump after label insertion has moved its item position.
    pub(crate) fn mark_borrow_empty_block_at(&mut self, instruction_offset: usize) {
        let instruction = self
            .linear_items_mut()
            .iter_mut()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .nth(instruction_offset)
            .expect("instruction offset should reference an emitted jump");
        debug_assert!(matches!(
            instruction.operand(),
            Operand::Forward(_) | Operand::Backward(_)
        ));
        instruction.jump_policy.mark_borrow_empty_block();
    }

    pub(crate) fn mark_before_trailing_instructions(&mut self, label: Label, count: usize) {
        let instruction_count = self.instruction_count();
        let count = count.max(1);
        let index = if count > instruction_count {
            self.linear_items().len()
        } else {
            self.linear_items()
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(index, item)| matches!(item, Item::Instruction(_)).then_some(index))
                .nth(count - 1)
                .expect("instruction count should match the linear stream")
        };
        self.insert_label(index, label);
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
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        items.insert(index, Item::Label(label));
    }

    pub(crate) fn fusion_barrier(&mut self) -> Label {
        let label = self.label();
        self.exception_owner_provenance
            .fusion_boundaries
            .insert(label);
        self.mark(label);
        label
    }

    /// Records compiler-side fold vacancies on the instruction that replaced them.
    pub(crate) fn add_last_hidden_exception_owner_slots(&mut self, slots: usize) {
        let slots =
            u16::try_from(slots.min(usize::from(LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS)))
                .expect("capped hidden slot count fits in u16");
        let instruction = self.required_last_instruction_mut("recording hidden owner slots");
        instruction
            .local_policy
            .add_hidden_exception_owner_slots(slots);
    }

    /// Places removed records on their first surviving successor after a source join.
    pub(crate) fn add_hidden_exception_owner_slots_at(&mut self, item_offset: usize, count: usize) {
        if let Some(instruction) =
            self.linear_items_mut()[item_offset..]
                .iter_mut()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
        {
            instruction.local_policy.add_hidden_exception_owner_slots(
                u16::try_from(
                    count.min(usize::from(LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS)),
                )
                .expect("capped hidden slot count fits in u16"),
            );
        }
    }

    /// Preserves the block size before constant folding for CPython's earlier exit-copy pass.
    pub(crate) fn add_last_constant_fold_slots(&mut self, count: usize) {
        self.add_last_hidden_exception_owner_slots(count);
        let count = u8::try_from(count.min(4)).expect("capped instruction count fits in u8");
        let instruction = self.required_last_instruction_mut("recording pre-fold block size");
        instruction.add_pre_fold_extra_instructions(count);
    }

    /// Certifies contiguous literal-fold records that share their survivor's owner.
    pub(crate) fn record_last_uniform_fold_owner_slots(&mut self, count: usize) {
        // The copy budget saturates at four, so larger counts cannot reject foreign additions.
        if count == 0 || count >= 4 {
            return;
        }
        let instruction = *self.required_last_instruction_mut("recording uniform fold owner slots");
        if usize::from(instruction.local_policy.hidden_exception_owner_slots()) == count {
            self.exception_owner_provenance
                .uniform_fold_owner_slots
                .push((
                    instruction.location,
                    instruction.opcode,
                    u16::try_from(count).expect("exact fold count fits in owner slots"),
                ));
        }
    }

    /// Records deleted operations on their first surviving successor, after any join labels.
    pub(crate) fn add_constant_fold_slots_at(&mut self, item_offset: usize, count: usize) {
        let instruction = self.linear_items_mut()[item_offset..]
            .iter_mut()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .expect("folded operations require a surviving successor");
        instruction.local_policy.add_hidden_exception_owner_slots(
            u16::try_from(count.min(usize::from(LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS)))
                .expect("capped hidden slot count fits in u16"),
        );
        instruction.add_pre_fold_extra_instructions(
            u8::try_from(count.min(4)).expect("capped instruction count fits in u8"),
        );
    }

    /// Counts the converted jump NOP when the compiler emits an inlined exit directly.
    pub(crate) fn add_last_inlined_jump_slot(&mut self) {
        self.required_last_instruction_mut("recording an inlined jump slot")
            .add_pre_fold_extra_instructions(1);
    }

    /// Retains original instruction slots that share one lowered boundary label.
    pub(crate) fn add_last_original_block_slots(&mut self, slots: usize) {
        self.required_last_instruction_mut("recording original block slots")
            .add_pre_fold_extra_instructions(u8::try_from(slots).unwrap_or(u8::MAX));
    }

    /// Resolves condition markers whose instructions all precede CFG optimization.
    pub(crate) fn defer_last_original_condition_exception_owner(&mut self) {
        let instruction =
            self.required_last_instruction_mut("deferring an original condition owner");
        let mut policy = instruction.exception_policy();
        policy.set_owner(NormalizedExceptionOwner::DynamicConditionStrict);
        instruction.set_exception_policy(policy);
    }

    /// Defers Boolean-marker ownership until all CFG removals and late insertions are known.
    pub(crate) fn defer_last_boolean_exception_owner(&mut self) {
        self.required_last_instruction_mut("deferring Boolean exception ownership")
            .set_exception_policy(ExceptionPolicy::NoneDynamic);
    }

    /// Marks the fallthrough created by backward conditional normalization as a fresh block.
    pub(crate) fn mark_last_condition_fallthrough_fresh(&mut self) {
        let instruction =
            self.required_last_instruction_mut("marking a fresh condition fallthrough");
        if instruction.opcode == NOT_TAKEN {
            instruction.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
        }
    }

    /// Keeps the normalized exception owner on a trailing condition fallthrough marker.
    pub(crate) fn preserve_last_not_taken_exception_owner(&mut self) {
        let instruction =
            self.required_last_instruction_mut("preserving a NOT_TAKEN exception owner");
        if instruction.opcode == NOT_TAKEN {
            instruction.set_exception_owner(NormalizedExceptionOwner::Present);
        }
    }

    /// Keeps a trailing condition marker in its existing structural exception region.
    pub(crate) fn retain_last_not_taken_structural_exception_owner(&mut self) {
        let instruction =
            self.required_last_instruction_mut("retaining a NOT_TAKEN structural exception owner");
        if instruction.opcode == NOT_TAKEN {
            instruction.set_exception_owner(NormalizedExceptionOwner::Unchanged);
        }
    }

    /// Restores the innermost normalized exception owner on a trailing condition marker.
    pub(crate) fn override_last_not_taken_exception_owner(&mut self) {
        let instruction =
            self.required_last_instruction_mut("overriding a NOT_TAKEN exception owner");
        if instruction.opcode == NOT_TAKEN {
            instruction.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
        }
    }

    /// Prevents fusion with the next instruction without introducing a CFG boundary.
    pub(crate) fn prevent_last_instruction_fusion(&mut self) {
        self.required_last_instruction_mut("preventing fusion with the next instruction")
            .fusion_policy
            .prevent_with_next();
    }

    /// Prevents a specifically identified instruction from fusing with its successor.
    pub(crate) fn prevent_instruction_fusion(&mut self, emitted: EmittedInstruction) {
        self.emitted_instruction_mut(emitted, "preventing fusion with the next instruction")
            .fusion_policy
            .prevent_with_next();
    }

    /// Prevents a specifically identified instruction from fusing with its predecessor.
    pub(crate) fn prevent_instruction_fusion_with_previous(&mut self, emitted: EmittedInstruction) {
        self.emitted_instruction_mut(emitted, "preventing fusion with the previous instruction")
            .fusion_policy
            .prevent_with_previous();
    }

    /// Records that a `STORE_FAST` restores a value saved by an inlined comprehension.
    /// The saved stack slot may contain NULL, unlike an ordinary local store.
    pub(crate) fn mark_store_fast_as_maybe_null_comprehension_restore(
        &mut self,
        emitted: EmittedInstruction,
    ) {
        let instruction = self.emitted_instruction_mut(emitted, "marking a maybe-null restore");
        assert_eq!(
            instruction.opcode, STORE_FAST,
            "marking a maybe-null restore requires STORE_FAST"
        );
        instruction
            .local_policy
            .mark_maybe_null_comprehension_restore();
    }

    pub(crate) fn mark_load_fast_as_post_analysis_closure(&mut self, emitted: EmittedInstruction) {
        let instruction =
            self.emitted_instruction_mut(emitted, "marking a post-analysis closure load");
        assert_eq!(
            instruction.opcode, LOAD_FAST,
            "marking a post-analysis closure load requires LOAD_FAST"
        );
        instruction.local_policy.mark_post_analysis_closure_load();
    }

    pub(crate) fn mark_load_fast_as_known_initialized(&mut self, emitted: EmittedInstruction) {
        let instruction = self.emitted_instruction_mut(emitted, "marking an initialized load");
        assert_eq!(
            instruction.opcode, LOAD_FAST,
            "marking an initialized load requires LOAD_FAST"
        );
        instruction.local_policy.mark_known_initialized_load();
    }

    /// Defers a redundant jump until the pass after exit-block duplication.
    pub(crate) fn defer_jump_removal(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "deferring redundant jump removal")
            .jump_policy
            .defer_removal();
    }
    pub(crate) fn allow_transitive_small_exit_inlining(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "allowing transitive small-exit inlining")
            .fusion_policy
            .allow_transitive_small_exit_inlining();
    }

    pub(crate) fn preserve_redundant_jump_nop_after_threading(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "retaining a redundant jump NOP after threading")
            .jump_policy
            .retain_redundant_nop_after_threading();
    }

    pub(crate) fn prevent_jump_inlining(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "preventing jump inlining")
            .jump_policy
            .prevent_small_exit_inlining();
    }

    /// Allows a direct jump to copy a return split by an exception-region boundary.
    pub(crate) fn allow_split_return_copy(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "allowing a split return copy")
            .no_location_policy
            .allow_split_return_exit_copy();
    }

    /// Allows a condition jump followed by a marker to copy a split return.
    pub(crate) fn allow_last_condition_split_return_copy(&mut self) {
        let mut instructions =
            self.linear_items_mut()
                .iter_mut()
                .rev()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                });
        let marker = instructions
            .next()
            .expect("condition marker should be present");
        assert_eq!(marker.opcode, NOT_TAKEN, "condition marker should be last");
        let jump = instructions
            .next()
            .expect("condition jump should precede marker");
        assert!(
            matches!(jump.operand(), Operand::Forward(_) | Operand::Backward(_)),
            "condition marker should follow a jump"
        );
        jump.no_location_policy.allow_split_return_exit_copy();
    }

    pub(crate) fn preserve_last_inlined_jump_nop(&mut self) {
        self.last_jump_instruction_mut("retaining an inlined jump NOP")
            .jump_policy
            .retain_nop();
    }

    pub(crate) fn preserve_inlined_jump_nop(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "retaining an inlined jump NOP")
            .jump_policy
            .retain_nop();
    }

    pub(crate) fn preserve_direct_inlined_jump_nop(&mut self, emitted: EmittedJump) {
        let instruction = self.emitted_jump_mut(emitted, "retaining a directly inlined jump NOP");
        let Operand::Forward(_) = instruction.operand() else {
            panic!("retaining a directly inlined jump NOP requires a forward jump");
        };
        instruction.jump_policy.retain_nop_for_direct_target();
    }

    /// Records a CPython ordinary jump block before small exits can replace its jump.
    pub(crate) fn mark_conditional_threading_target(
        &mut self,
        label: Label,
        location: SourceLocation,
    ) {
        self.conditional_threading_targets.insert(label, location);
    }

    pub(crate) fn prevent_jump_threading_target(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "preventing jump threading through a target")
            .jump_policy
            .prevent_threading_target();
    }

    pub(crate) fn mark_jump_as_authored_break(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "marking an authored break jump")
            .fusion_policy
            .mark_authored_break();
    }

    pub(crate) fn prepare_no_location_block_for_inlining(&mut self, emitted: EmittedJump) {
        let instruction =
            self.emitted_jump_mut(emitted, "preparing a no-location block for inlining");
        instruction.location = SourceLocation::NONE;
        instruction.jump_policy.prevent_threading_target();
        instruction.no_location_policy.allow_block_inlining();
    }

    /// Allows an unlocated terminal cleanup to be copied onto its predecessor edges.
    pub(crate) fn allow_last_no_location_block_inlining(&mut self) {
        self.required_last_instruction_mut("allowing an unlocated terminal block to be inlined")
            .no_location_policy
            .allow_block_inlining();
    }

    /// Allows early copying onto unconditional predecessors without enabling conditional copies.
    pub(crate) fn allow_last_unconditional_block_inlining(&mut self) {
        self.required_last_instruction_mut("allowing an unconditional block to be inlined")
            .allow_unconditional_block_inlining();
    }

    pub(crate) fn make_no_location_copy_interruptible(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "making a no-location copy interruptible")
            .no_location_policy
            .make_first_copy_interruptible();
    }

    /// Preserves an original JUMP's eval-breaker check if block ordering makes it backward.
    pub(crate) fn mark_jump_as_interruptible(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "marking an interruptible jump")
            .jump_policy
            .mark_interruptible();
    }

    /// Gives a copied no-location block its predecessor's location even when that predecessor is
    /// itself a control-flow join.
    pub(crate) fn make_last_no_location_copy_inherit_shared_predecessor(&mut self) {
        self.required_last_instruction_mut(
            "making a no-location copy inherit its shared predecessor",
        )
        .no_location_policy
        .make_copy_inherit_shared_predecessor();
    }

    /// Records that an eval-break location came from an artificial `NO_LOCATION` instruction.
    pub(crate) fn mark_as_artificial_eval_break(&mut self, emitted: EmittedJump) {
        let instruction =
            self.emitted_jump_mut(emitted, "marking an artificial eval-break instruction");
        debug_assert!(matches!(instruction.opcode, JUMP_FORWARD | JUMP_BACKWARD));
        instruction.no_location_policy.mark_artificial_eval_break();
    }

    pub(crate) fn preserve_last_no_location(&mut self) {
        self.required_last_instruction_mut("preserving a no-location instruction")
            .no_location_policy
            .preserve_location();
    }

    pub(crate) fn preserve_no_location(&mut self, emitted: EmittedInstruction) {
        self.emitted_instruction_mut(emitted, "preserving a no-location instruction")
            .no_location_policy
            .preserve_location();
    }

    /// Keeps an original shared jump unlocated while allowing its copies to inherit locations.
    pub(crate) fn preserve_shared_jump_no_location(&mut self, emitted: EmittedJump) {
        self.preserve_no_location(emitted.instruction());
        let index = usize::try_from(emitted.instruction().item_index)
            .expect("emitted instruction index should fit in usize");
        if let Some(Item::Label(label)) = self.linear_items().get(index.wrapping_sub(1)) {
            self.shared_jump_copy_location_blocks.insert(*label);
        }
    }

    /// Keeps a shared join unlocated without changing a NOP produced by exit copying.
    pub(crate) fn defer_shared_jump_no_location(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "deferring a shared jump's source location");
        let index = usize::try_from(emitted.instruction().item_index)
            .expect("emitted instruction index should fit in usize");
        if let Some(Item::Label(label)) = self.linear_items().get(index.wrapping_sub(1)) {
            self.deferred_shared_jump_blocks.insert(*label);
        }
    }

    pub(crate) fn inherit_percent_target_location_for_last_instruction(&mut self) {
        self.required_last_instruction_mut(
            "inheriting a predecessor location for a synthesized percent instruction",
        )
        .no_location_policy
        .percent_target_location = u8::MAX;
        self.has_synthesized_percent_target_locations = true;
    }

    pub(crate) fn mark_assertion_side_exit_candidate(&mut self) {
        self.has_assertion_side_exit_candidate = true;
    }

    /// Gives the last instruction its final successor's location after optimization.
    /// This mirrors CPython's `NEXT_LOCATION` pseudo-location.
    pub(crate) fn use_next_location_for_last_instruction(&mut self) {
        self.has_next_location = true;
        self.required_last_instruction_mut("using the next instruction location")
            .fusion_policy
            .mark_location_from_next();
    }

    /// Records the source range carried by a manager's normal and suppressed exits.
    pub(crate) fn record_with_context_location(&mut self, location: SourceLocation) {
        self.exception_owner_provenance
            .with_context_locations
            .push(location);
    }

    /// Records authored no-op finalizers whose copied NOPs precede original `POP_BLOCK` records.
    pub(crate) fn record_noop_finally_location(&mut self, location: SourceLocation) {
        if !self
            .exception_owner_provenance
            .noop_finally_locations
            .contains(&location)
        {
            self.exception_owner_provenance
                .noop_finally_locations
                .push(location);
        }
    }

    pub(crate) fn mark_last_as_converted_pop_block(&mut self) {
        self.required_last_instruction_mut("marking a converted POP_BLOCK")
            .fusion_policy
            .mark_converted_pop_block();
    }

    pub(crate) fn preserve_last_nop(&mut self) {
        let instruction = self.required_last_instruction_mut("preserving an explicit NOP");
        assert_eq!(
            instruction.opcode, NOP,
            "preserving an explicit NOP requires a NOP instruction"
        );
        instruction.jump_policy.preserve_explicit_nop();
    }

    /// Removes a trailing NOP, preserving its location, exclusion labels, and original block size.
    pub(crate) fn take_trailing_nop_location(&mut self) -> Option<RemovedNop> {
        let current_location = self.location;
        let index = self
            .linear_items()
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))?;
        let items = self.linear_items_mut();
        let Item::Instruction(instruction) = items[index] else {
            unreachable!();
        };
        if instruction.opcode != NOP
            || instruction.jump_policy.preserves_explicit_nop()
            || instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
        {
            return None;
        }
        if instruction.location.line >= 0
            && current_location.line >= 0
            && instruction.location.line != current_location.line
        {
            return None;
        }
        // Only a directly adjacent NOP shares the replacement jump's original block.
        // A surviving label separates the removed NOP from the replacement jump.
        let original_slots = if index + 1 == items.len() {
            1 + usize::from(instruction.pre_fold_extra_instructions())
        } else {
            0
        };
        let exclusion = if index > 0 {
            match (items[index - 1], items.get(index + 1).copied()) {
                (Item::Label(start), Some(Item::Label(end))) => Some((start, end)),
                _ => None,
            }
        } else {
            None
        };
        let original_nop_targets = items[..index]
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        self.original_nop_targets.extend(original_nop_targets);
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            unreachable!();
        };
        items.remove(index);
        self.instruction_count -= 1;
        // A later emission can reuse the removed item's index. Start a fresh stream so a copied
        // handle to the NOP cannot alias the replacement instruction.
        self.emission_stream = ir::EmissionStreamId::next();
        Some(RemovedNop {
            location: instruction.location,
            exclusion,
            original_slots,
        })
    }

    /// Identifies range splits that only exclude an authored SETUP NOP.
    pub(crate) fn record_setup_nop_boundaries(&mut self, start: Label, end: Label) {
        self.exception_owner_provenance
            .setup_nop_boundaries
            .extend([start, end]);
        self.exception_owner_provenance
            .setup_nop_ranges
            .push((start, end));
    }

    /// Identifies a range boundary with no original instruction or control-flow join.
    pub(crate) fn record_exception_owner_continuation(&mut self, label: Label) {
        self.exception_owner_provenance
            .continuation_boundaries
            .insert(label);
    }

    /// Starts a protected prefix within the same original instruction array.
    pub(crate) fn record_exception_owner_prefix(&mut self, label: Label) {
        self.record_exception_owner_continuation(label);
        self.exception_owner_provenance
            .protected_prefix_boundaries
            .insert(label);
    }

    /// Retains a source CFG boundary after its conditional jump is folded away.
    pub(crate) fn record_exception_owner_block_boundary(&mut self, label: Label) {
        self.exception_owner_provenance
            .block_boundaries
            .insert(label);
        self.exception_owner_provenance
            .source_boundaries
            .insert(label);
    }

    /// Preserves the original fallthrough block between short-circuit operands.
    pub(crate) fn record_short_circuit_owner_continuation(&mut self) {
        let follows_condition = self
            .linear_items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction.opcode == NOT_TAKEN),
                Item::Label(_) => None,
            });
        if follows_condition == Some(true) {
            let continuation = self.label();
            self.record_exception_owner_block_boundary(continuation);
            self.mark(continuation);
        }
    }

    /// Records the original SETUP instruction independently of split exception ranges.
    pub(crate) fn record_exception_setup(&mut self, label: Label, retained_nop: bool) {
        let nop_location = retained_nop.then(|| {
            self.last_instruction_location()
                .expect("setup NOP should precede its region")
        });
        self.exception_owner_provenance
            .setups
            .push((label, nop_location));
    }

    /// Records a synchronous manager setup whose protected suffix has one lexical owner.
    pub(crate) fn record_synchronous_with_exception_setup(&mut self, label: Label) {
        self.record_exception_setup(label, false);
        self.exception_owner_provenance
            .synchronous_with_setups
            .insert(label);
    }

    pub(crate) fn has_exception_regions(&self) -> bool {
        !self.exception_regions.is_empty()
    }

    pub(crate) fn add_exception_region(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
        depth: u32,
        preserve_lasti: bool,
    ) {
        self.add_exception_region_with_origin(
            start,
            end,
            target,
            depth,
            preserve_lasti,
            (start, end),
        );
    }

    /// Keeps split protected ranges ordered by their original setup position.
    pub(crate) fn add_exception_region_with_origin(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
        depth: u32,
        preserve_lasti: bool,
        origin: (Label, Label),
    ) {
        self.exception_regions.push(ExceptionRegion {
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(origin.0, origin.1),
            start,
            end,
            target,
            depth,
            preserve_lasti,
        });
    }

    /// Adds the outermost handler that converts escaping `StopIteration` exceptions.
    pub(crate) fn add_stopiteration_exception_region(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
    ) {
        self.exception_regions.push(ExceptionRegion {
            is_scope_fallback: true,
            priority: ExceptionRegionPriority::OriginalRange(start, end),
            start,
            end,
            target,
            depth: 0,
            preserve_lasti: true,
        });
    }

    pub(crate) fn emit(&mut self, opcode: Opcode, argument: u32) -> EmittedInstruction {
        self.emit_operand(opcode, Operand::Value(argument))
    }

    pub(crate) fn emit_with_depth(
        &mut self,
        opcode: Opcode,
        argument: u32,
        depth_after: u32,
        exception_policy: ExceptionPolicy,
    ) -> EmittedInstruction {
        self.emit_operand_with_depth_and_exception_policy(
            opcode,
            Operand::Value(argument),
            Some(depth_after),
            exception_policy,
        )
    }

    pub(crate) fn emit_value_with_depth_discard(
        &mut self,
        opcode: Opcode,
        argument: u32,
        depth_after: u32,
        exception_policy: ExceptionPolicy,
    ) {
        if opcode == LOAD_FAST || exception_policy != ExceptionPolicy::default() {
            self.emit_with_depth(opcode, argument, depth_after, exception_policy);
            return;
        }

        if !self.has_constant_pop_candidate
            && opcode == POP_TOP
            && self.linear_items().last().is_some_and(|item| {
                matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_CONST | LOAD_COMMON_CONSTANT | LOAD_SMALL_INT))
            })
        {
            self.has_constant_pop_candidate = true;
        }

        self.push_linear_item(Item::Instruction(Instruction::value_with_depth(
            opcode,
            argument,
            self.location,
            depth_after,
        )));
    }

    pub(crate) fn emit_owned_fast_with_depth(
        &mut self,
        argument: u32,
        depth_after: u32,
    ) -> EmittedInstruction {
        self.emit_operand_with_depth_and_ownership(
            LOAD_FAST,
            Operand::Value(argument),
            Some(depth_after),
            true,
            ExceptionPolicy::default(),
        )
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
            if !matches!(instruction.operand(), Operand::Deferred(id) if id == expected) {
                return Err(CompileError::Internal(format!(
                    "{state} operand patch {id} has no instruction"
                )));
            }
        }

        for (item_index, item) in items.iter_mut().enumerate() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            let Operand::Deferred(id) = instruction.operand() else {
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
            instruction.set_operand(Operand::Value(argument));
        }

        self.operand_patches.clear();
        Ok(())
    }

    pub(crate) fn used_constant_indices(
        &mut self,
        load_const: Opcode,
        constant_count: usize,
    ) -> Vec<bool> {
        if let UnoptimizedCode::Prepared { graph, reachable } = &self.code {
            return reachable_constant_indices(graph, reachable, load_const, constant_count);
        }

        let mut used = vec![false; constant_count];
        let mut remaining = constant_count.saturating_sub(1);
        let mut ended_scope = false;
        let mut straight_line = true;
        for item in self.linear_items() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            if ended_scope || !matches!(instruction.operand(), Operand::Value(_)) {
                straight_line = false;
                break;
            }
            if instruction.opcode == load_const
                && let Operand::Value(index) = instruction.operand()
                && let Some(slot) = used.get_mut(index as usize)
            {
                let newly_used = !std::mem::replace(slot, true);
                if index != 0 && newly_used {
                    remaining -= 1;
                    if remaining == 0 {
                        return used;
                    }
                }
            }
            ended_scope = ends_scope(instruction.opcode);
        }
        if straight_line {
            return used;
        }
        if self.has_assertion_side_exit_candidate
            && self.exception_regions.is_empty()
            && self.preserved_block_boundaries.is_empty()
            && self.borrow_unreachable_blocks.is_empty()
            && !self.has_synthesized_percent_target_locations
            && let Some(linear_eligible) =
                assertion_constant_indices(self.linear_items(), load_const, &mut used)
        {
            self.has_proven_linear_assertion_side_exits = linear_eligible;
            return used;
        }

        let UnoptimizedCode::Linear(items) =
            std::mem::replace(&mut self.code, UnoptimizedCode::Linear(Vec::new()))
        else {
            unreachable!();
        };
        let graph = block_graph::BlockGraph::from_items(items, &self.preserved_block_boundaries);
        let reachable = graph.reachable_blocks(&self.exception_regions);
        let used = reachable_constant_indices(&graph, &reachable, load_const, constant_count);
        self.code = UnoptimizedCode::Prepared { graph, reachable };
        used
    }

    pub(crate) fn remap_constant_indices(&mut self, load_const: Opcode, index_map: &[Option<u32>]) {
        let mut discarded_admissions = self
            .discarded_fold_admissions
            .iter_mut()
            .collect::<Vec<_>>();
        discarded_admissions.sort_unstable_by_key(|(label, _)| label.0);
        for (_, admissions) in discarded_admissions {
            admissions.retain_mut(|index| {
                if let Some(remapped) = index_map[*index as usize] {
                    *index = remapped;
                    true
                } else {
                    false
                }
            });
        }
        let deferred_none = self.deferred_none_constant_index;
        let remap = |instruction: &mut Instruction| {
            if instruction.opcode != load_const {
                return;
            }
            let Operand::Value(mut index) = instruction.operand() else {
                unreachable!("LOAD_CONST has a jump operand");
            };
            if index == u32::MAX {
                let old_index = usize::try_from(
                    deferred_none.expect("deferred None constant should be resolved"),
                )
                .expect("constant index should fit in usize");
                index = index_map[old_index].expect("deferred None constant should be retained");
            } else if let Some(new_index) = index_map[usize::try_from(index).unwrap()] {
                index = new_index;
            } else {
                // This unreachable load's constant was removed from the pool. Retain
                // its original depth and fold size until the block is discarded, but
                // do not leave an obsolete index for folded-admission bookkeeping.
                instruction.opcode = NOP;
                index = 0;
            }
            instruction.set_operand(Operand::Value(index));
        };
        match &mut self.code {
            UnoptimizedCode::Linear(items) => {
                for item in items {
                    if let Item::Instruction(instruction) = item {
                        remap(instruction);
                    }
                }
            }
            UnoptimizedCode::Prepared { graph, .. } => graph.for_each_instruction_mut(remap),
        }
    }

    pub(crate) const fn deferred_none_constant_index(&self) -> Option<u32> {
        self.deferred_none_constant_index
    }

    /// Retains a folded value's construction at an omitted expression's source position.
    pub(crate) fn record_discarded_fold_admission(&mut self, label: Label, index: u32) {
        self.discarded_fold_admissions
            .entry(label)
            .or_default()
            .push(index);
    }

    pub(crate) fn set_deferred_none_constant_index(&mut self, index: u32) {
        self.deferred_none_constant_index = Some(index);
    }

    /// Removes a side-effect-free constant whose value is immediately discarded.
    pub(crate) fn optimize_constant_pops(&mut self) {
        if !self.has_constant_pop_candidate {
            return;
        }
        let is_constant_load =
            |opcode| matches!(opcode, LOAD_CONST | LOAD_COMMON_CONSTANT | LOAD_SMALL_INT);
        let items = self.linear_items_mut();
        let mut has_forward_constant_jump = false;

        for index in 0..items.len().saturating_sub(1) {
            let [Item::Instruction(load), Item::Instruction(next)] = &mut items[index..index + 2]
            else {
                continue;
            };
            if !is_constant_load(load.opcode) {
                continue;
            }
            if next.opcode == POP_TOP {
                load.opcode = NOP;
                load.set_operand(Operand::Value(0));
                next.opcode = NOP;
                next.set_operand(Operand::Value(0));
            } else if next.opcode == JUMP_FORWARD && matches!(next.operand(), Operand::Forward(_)) {
                has_forward_constant_jump = true;
            }
        }

        if !has_forward_constant_jump {
            return;
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
            let Operand::Forward(target) = jump.operand() else {
                continue;
            };
            if !is_constant_load(load.opcode) || jump.opcode != JUMP_FORWARD {
                continue;
            }
            let Some(target_position) = label_positions.get(&target).copied() else {
                continue;
            };
            let mut target = items[target_position + 1..].iter().filter_map(|item| {
                if let Item::Instruction(instruction) = item {
                    Some(*instruction)
                } else {
                    None
                }
            });
            let (Some(pop), Some(return_load), Some(return_value)) =
                (target.next(), target.next(), target.next())
            else {
                continue;
            };
            if pop.opcode == POP_TOP
                && is_constant_load(return_load.opcode)
                && return_value.opcode == RETURN_VALUE
            {
                replacements.push((index, return_load, return_value));
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
    ) -> EmittedJump {
        EmittedJump(self.emit_operand_with_depth_and_exception_policy(
            opcode,
            Operand::Forward(label),
            Some(depth_after),
            exception_policy,
        ))
    }

    pub(crate) fn emit_backward(&mut self, opcode: Opcode, label: Label) -> EmittedJump {
        EmittedJump(self.emit_operand(opcode, Operand::Backward(label)))
    }

    pub(crate) fn emit_backward_with_depth_and_exception_policy(
        &mut self,
        opcode: Opcode,
        label: Label,
        depth_after: u32,
        exception_policy: ExceptionPolicy,
    ) -> EmittedJump {
        EmittedJump(self.emit_operand_with_depth_and_exception_policy(
            opcode,
            Operand::Backward(label),
            Some(depth_after),
            exception_policy,
        ))
    }

    fn emit_operand(&mut self, opcode: Opcode, operand: Operand) -> EmittedInstruction {
        self.emit_operand_with_depth(opcode, operand, None)
    }

    fn emit_operand_with_depth(
        &mut self,
        opcode: Opcode,
        operand: Operand,
        depth_after: Option<u32>,
    ) -> EmittedInstruction {
        self.emit_operand_with_depth_and_exception_policy(
            opcode,
            operand,
            depth_after,
            ExceptionPolicy::default(),
        )
    }

    fn emit_operand_with_depth_and_exception_policy(
        &mut self,
        opcode: Opcode,
        operand: Operand,
        depth_after: Option<u32>,
        exception_policy: ExceptionPolicy,
    ) -> EmittedInstruction {
        self.emit_operand_with_depth_and_ownership(
            opcode,
            operand,
            depth_after,
            false,
            exception_policy,
        )
    }

    fn emit_operand_with_depth_and_ownership(
        &mut self,
        opcode: Opcode,
        operand: Operand,
        depth_after: Option<u32>,
        explicitly_owned: bool,
        exception_policy: ExceptionPolicy,
    ) -> EmittedInstruction {
        let strict_owned_load = self.strict_owned_loads && opcode == LOAD_FAST;
        let force_owned_load = explicitly_owned
            || strict_owned_load
            || (opcode == LOAD_FAST && !self.load_fast_borrowing_enabled);
        let mut instruction = Instruction::new(opcode, operand, self.location, depth_after);
        instruction.set_exception_policy(exception_policy);
        instruction.local_policy = LocalPolicy::for_load(force_owned_load, strict_owned_load);
        if self.forced_borrowed_loads && opcode == LOAD_FAST {
            instruction.local_policy.mark_force_borrowed_load();
        }
        if !self.has_constant_pop_candidate
            && (opcode == POP_TOP
                || (opcode == JUMP_FORWARD && matches!(operand, Operand::Forward(_))))
            && self.linear_items().last().is_some_and(|item| {
                matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_CONST | LOAD_COMMON_CONSTANT | LOAD_SMALL_INT))
            })
        {
            self.has_constant_pop_candidate = true;
        }
        let emitted = EmittedInstruction {
            stream: self.emission_stream,
            item_index: u32::try_from(self.linear_items().len())
                .expect("assembler item index should fit in u32"),
        };
        self.push_linear_item(Item::Instruction(instruction));
        emitted
    }
}

fn opcode_stack_effect(opcode: Opcode, argument: u32) -> i64 {
    let (popped, pushed) = opcode_stack_counts(opcode, argument);
    i64::try_from(pushed).unwrap() - i64::try_from(popped).unwrap()
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
    match instruction.operand() {
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
