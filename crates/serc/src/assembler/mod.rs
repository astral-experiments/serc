mod block_graph;
mod exception_owners;
mod fusion_owner_slots;
mod ir;
mod layout;
mod passes;

#[cfg(test)]
mod tests;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::CompileError;
use crate::code_object::Constant;
use crate::target::opcodes::{
    CHECK_EXC_MATCH, CLEANUP_THROW, COMPARE_OP, CONTAINS_OP, COPY, COPY_FREE_VARS, DELETE_DEREF,
    DELETE_FAST, DICT_MERGE, DICT_UPDATE, END_SEND, EXTENDED_ARG, FOR_ITER, FORMAT_SIMPLE,
    GET_ANEXT, GET_LEN, GET_YIELD_FROM_ITER, IMPORT_FROM, IS_OP, JUMP_BACKWARD,
    JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, LIST_APPEND, LIST_EXTEND, LOAD_ATTR,
    LOAD_COMMON_CONSTANT, LOAD_CONST, LOAD_FAST, LOAD_FAST_AND_CLEAR, LOAD_FAST_BORROW,
    LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_CHECK, LOAD_FAST_LOAD_FAST, LOAD_GLOBAL,
    LOAD_SMALL_INT, LOAD_SPECIAL, LOAD_SUPER_ATTR, MAKE_CELL, MAP_ADD, MATCH_KEYS, MATCH_MAPPING,
    MATCH_SEQUENCE, NOP, NOT_TAKEN, POP_EXCEPT, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE,
    POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, POP_TOP, PUSH_EXC_INFO, RAISE_VARARGS, RERAISE, RESUME,
    RETURN_VALUE, SEND, SET_ADD, SET_FUNCTION_ATTRIBUTE, SET_UPDATE, STORE_DEREF, STORE_FAST,
    STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST, STORE_GLOBAL, SWAP, TO_BOOL, UNARY_NOT, UNPACK_EX,
    UNPACK_SEQUENCE, WITH_EXCEPT_START,
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

/// Binds a delegated try's normal jump while its finally target is being emitted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NameReturnBorrowExit {
    source: EmittedJump,
    original: Instruction,
}

/// Certifies one complete local-capture cleanup rotation.
///
/// A conditional marker identifies an original block entry with identical pop locations.
/// After an unpack or pop, all five operations must instead share a line: both replacement
/// NOPs disappear, and the moved store cannot fuse with either the prefix or trailing pops.
fn single_capture_cleanup_vacancies(items: &[Item]) -> Option<usize> {
    let [
        Item::Instruction(marker),
        Item::Instruction(outer),
        Item::Instruction(inner),
        Item::Instruction(first_pop),
        Item::Instruction(last_pop),
        Item::Instruction(store),
    ] = items
    else {
        return None;
    };
    let same_line_cleanup = matches!(marker.opcode, UNPACK_SEQUENCE | POP_TOP)
        && [inner, first_pop, last_pop, store]
            .iter()
            .all(|instruction| instruction.location.line == outer.location.line);
    if (marker.opcode != NOT_TAKEN && !same_line_cleanup)
        || outer.opcode != SWAP
        || outer.operand() != Operand::Value(3)
        || inner.opcode != SWAP
        || inner.operand() != Operand::Value(2)
        || first_pop.opcode != POP_TOP
        || last_pop.opcode != POP_TOP
        || first_pop.operand() != Operand::Value(0)
        || last_pop.operand() != Operand::Value(0)
        || store.opcode != STORE_FAST
        || !matches!(store.operand(), Operand::Value(_))
        || store.local_policy.may_store_null()
        || outer.location.line < 0
        || outer.location != inner.location
        || first_pop.location.line < 0
        || (!same_line_cleanup
            && (first_pop.location != last_pop.location || first_pop.location != store.location))
    {
        return None;
    }
    // Unknown location, fusion, or NOP-retention provenance needs the existing fallback.
    for instruction in [outer, inner, first_pop, last_pop, store] {
        if instruction.no_location_policy != ir::NoLocationPolicy::default()
            || instruction.fusion_policy != ir::FusionPolicy::default()
            || instruction.jump_policy != ir::JumpPolicy::default()
        {
            return None;
        }
    }
    Some(1 + usize::from(outer.location.line == store.location.line))
}

/// A complete capture rotation with a known permutation and NOP-removal count.
///
/// The original prefix separates this run from earlier captures. Keeping its consumers
/// lets owner prediction use the same permutation and subsequent fusion as static swaps.
struct CaptureCleanupRotation<'a> {
    swap_count: usize,
    removed_slots: usize,
    consumers: &'a [Item],
}

fn capture_cleanup_rotation(items: &[Item]) -> Option<CaptureCleanupRotation<'_>> {
    let [Item::Instruction(prefix), Item::Instruction(first_swap), ..] = items else {
        return None;
    };
    let unpacked =
        prefix.opcode == UNPACK_SEQUENCE && matches!(prefix.operand(), Operand::Value(0 | 1));
    let starred = prefix.opcode == UNPACK_EX && prefix.operand() == Operand::Value(0);
    let conditional = prefix.opcode == NOT_TAKEN && prefix.operand() == Operand::Value(0);
    let popped = prefix.opcode == POP_TOP && prefix.operand() == Operand::Value(0);
    if !(unpacked || starred || conditional || popped)
        || prefix.location.line < 0
        || first_swap.opcode != SWAP
        || first_swap.location.line < 0
        || (!conditional && first_swap.location.line != prefix.location.line)
    {
        return None;
    }
    let Operand::Value(depth) = first_swap.operand() else {
        return None;
    };
    let depth = usize::try_from(depth).ok()?;
    // The existing single-capture proof owns depth-three marker/pop rotations.
    if depth < 3 || ((conditional || popped) && depth < 4) {
        return None;
    }
    let swap_count = depth - 1;
    let swaps = items.get(1..depth)?;
    for (index, item) in swaps.iter().enumerate() {
        let Item::Instruction(instruction) = item else {
            return None;
        };
        if instruction.opcode != SWAP
            || instruction.operand() != Operand::Value(u32::try_from(depth - index).ok()?)
            || instruction.location.line != first_swap.location.line
        {
            return None;
        }
    }
    // An OR arm carries its capture to the forward join. Only the smaller swaps
    // have enough pop operands; the largest swap remains ahead of the rotated pops.
    let pops_end = depth.checked_add(swap_count)?;
    let pop_only_join = starred
        && items.get(depth..pops_end).is_some_and(|consumers| {
            consumers.iter().all(|item| {
                matches!(item, Item::Instruction(instruction)
                if instruction.opcode == POP_TOP && instruction.operand() == Operand::Value(0))
            })
        })
        && matches!(items.get(pops_end), Some(Item::Instruction(jump))
            if jump.opcode == JUMP_FORWARD && matches!(jump.operand(), Operand::Forward(_)));
    let consumer_count = if pop_only_join { swap_count } else { depth };
    let consumed = depth.checked_add(consumer_count)?;
    let consumers = items.get(depth..consumed)?;
    let Some(Item::Instruction(first_consumer)) = consumers.first() else {
        return None;
    };
    if first_consumer.location.line < 0
        || (pop_only_join && first_consumer.location.line != first_swap.location.line)
    {
        return None;
    }
    let mut pop_count = 0;
    let mut stores = Vec::new();
    for item in consumers {
        let Item::Instruction(instruction) = item else {
            return None;
        };
        if instruction.location.line != first_consumer.location.line {
            return None;
        }
        match (instruction.opcode, instruction.operand()) {
            (POP_TOP, Operand::Value(0)) if stores.is_empty() => pop_count += 1,
            (STORE_FAST, Operand::Value(local))
                if !instruction.local_policy.may_store_null() && !stores.contains(&local) =>
            {
                stores.push(local);
            }
            _ => return None,
        }
    }
    if pop_count < 2 || (!pop_only_join && stores.is_empty()) {
        return None;
    }
    // Close the emitted snapshot before predicting it. A later line cannot fuse with
    // the rotated final store. A sole capture's guard load is separated by pops;
    // with two captures, a large penultimate local cannot fuse with its guard load.
    let following = match items.get(consumed) {
        Some(Item::Instruction(instruction)) => instruction,
        Some(Item::Label(_)) => {
            // A following try can place labels before its located NOP. The complete
            // rotation stays before those labels, and the next-line NOP still closes
            // its fusion boundary. Do not look through any intervening instruction.
            let Some(Item::Instruction(nop)) = items[consumed..]
                .iter()
                .find(|item| !matches!(item, Item::Label(_)))
            else {
                return None;
            };
            if nop.opcode != NOP || nop.operand() != Operand::Value(0) {
                return None;
            }
            nop
        }
        None => return None,
    };
    let same_line_capture_guard = matches!(
        stores.as_slice(),
        [local]
            if following.opcode == LOAD_FAST
                && following.operand() == Operand::Value(*local)
    ) || matches!(
        stores.as_slice(),
        [penultimate, _]
            if *penultimate >= 16
                && following.opcode == LOAD_FAST
                && following.operand() == Operand::Value(*penultimate)
    );
    if following.location.line < 0
        || (!pop_only_join
            && following.location.line == first_consumer.location.line
            && !same_line_capture_guard)
    {
        return None;
    }
    // Preserve unknown provenance instead of predicting a different compaction schedule.
    for item in &items[..consumed] {
        let Item::Instruction(instruction) = item else {
            return None;
        };
        if instruction.no_location_policy != ir::NoLocationPolicy::default()
            || instruction.fusion_policy != ir::FusionPolicy::default()
            || instruction.jump_policy != ir::JumpPolicy::default()
        {
            return None;
        }
    }
    // The conditional starts a fresh native array. Its capture line needs one NOP
    // when it differs from the cleanup line; the other same-line NOPs disappear.
    let retained_line_nop = conditional && first_swap.location.line != first_consumer.location.line;
    Some(CaptureCleanupRotation {
        swap_count,
        removed_slots: consumer_count - 1 - usize::from(retained_line_nop),
        consumers,
    })
}

/// Constant-table operations needed by optimizations that follow exit-block duplication.
pub(crate) trait ConstantTable {
    fn get(&self, index: u32) -> &Constant;
    fn intern(&mut self, constant: Constant) -> Result<u32, CompileError>;
    fn len(&self) -> usize;
    fn retain_used(&mut self, used: &[bool], order: &[u32]) -> Vec<Option<u32>>;
}

/// Identifies the start of an emitted region without depending on inserted labels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EmittedRegionStart {
    instruction_count: usize,
    stream: ir::EmissionStreamId,
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
            && self.borrow_omitted_for_backedges.is_empty()
            && self.folded_loop_exit_anchors.is_empty()
            && !self.has_synthesized_percent_target_locations
            && assertion_constant_indices(items, LOAD_CONST, &mut []).unwrap_or(false)
    }

    fn push_linear_item(&mut self, item: Item) {
        let UnoptimizedCode::Linear(items) = &mut self.code else {
            panic!("linear instructions are unavailable after constant reachability analysis");
        };
        if matches!(item, Item::Instruction(_)) {
            self.instruction_count += 1;
            self.original_fallthrough_location = None;
            self.original_match_join = None;
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

    #[cfg(test)]
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
        Some(
            self.original_fallthrough_location
                .filter(|(emitted, _)| emitted.item_index as usize == index)
                .map_or(instruction.location, |(_, location)| location),
        )
    }

    /// Returns an ordinary fast-local store that can precede deferred restores.
    pub(crate) fn last_fast_store_location(&self) -> Option<(u32, SourceLocation)> {
        let instruction = self
            .linear_items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })?;
        if instruction.opcode != STORE_FAST || instruction.local_policy.may_store_null() {
            return None;
        }
        match instruction.operand() {
            Operand::Value(index) => Some((index, instruction.location)),
            _ => None,
        }
    }

    /// Retains the original final store's location after deferred static reordering.
    pub(crate) fn record_original_fallthrough_location(
        &mut self,
        emitted: EmittedInstruction,
        location: SourceLocation,
    ) {
        let instruction = self.emitted_instruction_mut(emitted, "recording original fallthrough");
        debug_assert_eq!(instruction.opcode, STORE_FAST);
        self.original_fallthrough_location = Some((emitted, location));
    }

    /// Returns whether the current block ends in a run that swap optimization rewrites.
    pub(crate) fn current_block_ends_with_multiple_swaps(&self) -> bool {
        self.linear_items()
            .iter()
            .rev()
            .take_while(
                |item| matches!(item, Item::Instruction(instruction) if instruction.opcode == SWAP),
            )
            .take(2)
            .count()
            == 2
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

    /// Whether an adjacent record already supplies a trailing NOP's line in its array.
    pub(crate) fn last_nop_repeats_previous_line(&self) -> bool {
        matches!(self.linear_items(),
            [.., Item::Instruction(previous), Item::Instruction(nop)]
                if nop.opcode == NOP
                    && nop.location.line >= 0
                    && previous.location.line == nop.location.line
                    && previous.opcode != NOT_TAKEN
                    && matches!(previous.operand(), Operand::Value(_))
                    && !ends_scope(previous.opcode))
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

    /// Checks the live call suffix of the current original source array.
    pub(crate) fn current_original_block_ends_with_discarded_call(&self) -> bool {
        let mut items = self.linear_items().iter().rev();
        for expected in [POP_TOP, crate::target::opcodes::CALL] {
            loop {
                match items.next() {
                    Some(Item::Label(label))
                        if !self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                            && !self.preserved_block_boundaries.contains(label) => {}
                    Some(Item::Instruction(instruction))
                        if instruction.opcode == expected
                            && !instruction.jump_policy.borrow_empty_block()
                            && instruction.no_location_policy.borrow_fallthrough()
                                == BorrowFallthrough::Reachable =>
                    {
                        break;
                    }
                    _ => return false,
                }
            }
        }
        true
    }

    /// Recognizes a nonempty, straight-line original source for a normal finally edge.
    pub(crate) fn current_original_block_has_nonempty_borrow_source(&self) -> bool {
        let mut first = None;
        for item in self.linear_items().iter().rev() {
            match item {
                Item::Label(label) => {
                    if self.preserved_block_boundaries.contains(label) {
                        return false;
                    }
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label)
                    {
                        return self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                            && first.is_some();
                    }
                }
                Item::Instruction(instruction) => {
                    if instruction.opcode == NOT_TAKEN
                        || !matches!(instruction.operand(), Operand::Value(_))
                        || ends_scope(instruction.opcode)
                        || instruction.jump_policy.borrow_empty_block()
                        || instruction.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::Reachable
                    {
                        return false;
                    }
                    first = Some(instruction);
                }
            }
        }
        false
    }

    /// Finds the last located record in the current original instruction array.
    /// Exception-range aliases do not interrupt CPython's within-array line donation.
    pub(crate) fn current_original_block_location(&self) -> Option<SourceLocation> {
        for item in self.linear_items().iter().rev() {
            match item {
                Item::Label(label) => {
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label)
                        || self.preserved_block_boundaries.contains(label)
                    {
                        return None;
                    }
                }
                Item::Instruction(instruction) => {
                    if instruction.opcode == NOT_TAKEN
                        || !matches!(instruction.operand(), Operand::Value(_))
                        || ends_scope(instruction.opcode)
                    {
                        return None;
                    }
                    if instruction.location.line >= 0 {
                        return Some(instruction.location);
                    }
                }
            }
        }
        None
    }

    /// Whether a source NOP reaches this join by fallthrough or a compacted cleanup jump.
    pub(crate) fn current_join_has_located_compacted_predecessor(&self) -> bool {
        let labels = self
            .linear_items()
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if let Some(Item::Instruction(instruction)) = self
            .linear_items()
            .get(self.linear_items().len().saturating_sub(labels.len() + 1))
            && instruction.opcode == NOP
            && instruction.location.line >= 0
            && !instruction.fusion_policy.is_converted_pop_block()
        {
            // Incoming jumps retain their own locations; the fallthrough keeps this NOP.
            return true;
        }
        let mut follows_pop_block = false;
        for item in self.linear_items() {
            match item {
                Item::Label(label) => {
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label)
                        || self.preserved_block_boundaries.contains(label)
                    {
                        follows_pop_block = false;
                    }
                }
                Item::Instruction(instruction) => {
                    if follows_pop_block
                        && instruction.opcode == JUMP_FORWARD
                        && instruction.location.line >= 0
                        && instruction.pre_fold_extra_instructions() > 0
                        && matches!(instruction.operand(), Operand::Forward(target)
                            if labels.contains(&target))
                    {
                        return true;
                    }
                    follows_pop_block = instruction.opcode == NOP
                        && instruction.fusion_policy.is_converted_pop_block();
                }
            }
        }
        false
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

    /// Whether an empty original join has an unconditional edge and no authored location.
    ///
    /// A source-authored match end can also have a wildcard fallthrough predecessor.
    ///
    /// Range and fusion labels do not establish a CPython instruction-array boundary.
    /// Hidden authored NOPs and handler entries also prevent treating the join as unlocated.
    pub(crate) fn at_original_unconditional_join(&self) -> bool {
        let items = self.linear_items();
        let labels = items
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let Some(Item::Instruction(previous)) =
            items.get(items.len().saturating_sub(labels.len() + 1))
        else {
            return false;
        };
        let match_join = self
            .original_match_join
            .filter(|label| labels.contains(label));
        if !(ends_scope(previous.opcode)
            || is_unconditional_jump(previous.opcode)
            || match_join.is_some())
            || self.join_labels_have_authored_location(&labels)
        {
            return false;
        }
        items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if is_unconditional_jump(instruction.opcode)
                    && matches!(instruction.operand(), Operand::Forward(label)
                        if labels.contains(&label)
                            && (match_join == Some(label)
                                || (self.exception_owner_provenance.source_boundaries.contains(&label)
                                    && self.exception_owner_provenance.block_boundaries.contains(&label)))))
        })
    }

    /// Returns the sole conditional donor at a source-authored match failure join.
    pub(crate) fn original_match_failure_location(&self) -> Option<SourceLocation> {
        let end = self.original_match_join?;
        let items = self.linear_items();
        let labels = items
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if !labels.contains(&end) || self.join_labels_have_authored_location(&labels) {
            return None;
        }
        let Item::Instruction(previous) = items.get(items.len().checked_sub(labels.len() + 1)?)?
        else {
            return None;
        };
        if !ends_scope(previous.opcode) && !is_unconditional_jump(previous.opcode) {
            return None;
        }
        let mut incoming = items.iter().filter_map(|item| match item {
            Item::Instruction(instruction)
                if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if labels.contains(&label)) => Some(instruction),
            _ => None,
        });
        let edge = incoming.next()?;
        (incoming.next().is_none()
            && is_conditional_jump(edge.opcode)
            && matches!(edge.operand(), Operand::Forward(_))
            && edge.location.line >= 0)
            .then_some(edge.location)
    }

    /// Excludes labels with authored locations or non-source boundary provenance.
    fn join_labels_have_authored_location(&self, labels: &[Label]) -> bool {
        labels.iter().any(|label| {
            self.original_nop_targets.contains(label)
                || self.borrow_block_aliases.contains(label)
                || self
                    .exception_owner_provenance
                    .fusion_boundaries
                    .contains(label)
                || self
                    .exception_owner_provenance
                    .continuation_boundaries
                    .contains(label)
                || self
                    .exception_owner_provenance
                    .setups
                    .iter()
                    .any(|(target, _)| target == label)
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == *label)
        })
    }

    /// Records an original conditional target whose edge folded into fallthrough.
    pub(crate) fn record_folded_condition_predecessor(&mut self, target: Label) {
        self.folded_away_condition_targets.insert(target);
    }

    /// Records the empty original anchor before a folded while loop's shared exit.
    pub(crate) fn record_folded_loop_exit_anchor(&mut self, target: Label) {
        self.record_folded_condition_predecessor(target);
        self.folded_loop_exit_anchors.insert(target);
    }

    /// Records a discarded linear loop else suite's original normal exit.
    pub(crate) fn record_folded_loop_else_predecessor(&mut self, target: Label) {
        self.folded_away_loop_else_targets.insert(target);
    }

    /// Records the existing NOP at a folded edge's source, without adding a CFG boundary.
    pub(crate) fn record_folded_predecessor_source(&mut self, source: Label, target: Label) {
        if !self.folded_away_condition_targets.contains(&target)
            && !self.folded_away_loop_else_targets.contains(&target)
        {
            return;
        }
        let marker = self.required_last_instruction_mut("recording a folded source marker");
        debug_assert_eq!(marker.opcode, NOP);
        let location = marker.location;
        self.folded_predecessor_sources
            .push((source, target, location));
    }

    /// Whether folded and emitted conditional edges share this original block head.
    pub(crate) fn has_folded_shared_conditional_entry(&self) -> bool {
        let items = self.linear_items();
        let labels = items
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if !labels
            .iter()
            .any(|label| self.folded_away_condition_targets.contains(label))
        {
            return false;
        }
        // A located physical fallthrough can supply a donor in the original block.
        // This certificate covers only joins reached through conditional edges.
        if !items
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(
                    ends_scope(instruction.opcode) || is_unconditional_jump(instruction.opcode),
                ),
                Item::Label(_) => None,
            })
            .unwrap_or(false)
        {
            return false;
        }
        let mut has_emitted_condition = false;
        for item in items {
            if let Item::Instruction(instruction) = item
                && matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if labels.contains(&label))
            {
                // Unconditional predecessors may receive a cleanup copy before
                // native line resolution; their original head needs separate handling.
                if !is_conditional_jump(instruction.opcode) {
                    return false;
                }
                has_emitted_condition = true;
            }
        }
        has_emitted_condition
    }

    /// Keeps an original non-copyable normal exit and its incoming break owners.
    pub(crate) fn preserve_folded_bridge_pop_block(
        &mut self,
        instruction_offset: usize,
        try_exclusions: &mut Vec<(Label, Label)>,
    ) {
        let items = self.linear_items();
        let mut instructions = Vec::new();
        let mut entries = FxHashMap::default();
        let mut pending = Vec::new();
        for (item_index, item) in items.iter().enumerate() {
            match item {
                Item::Label(label) => pending.push(*label),
                Item::Instruction(instruction) => {
                    for label in pending.drain(..) {
                        entries.insert(label, instructions.len());
                    }
                    instructions.push((item_index, instruction));
                }
            }
        }
        let Some(&(marker_item, marker)) = instructions.get(instruction_offset) else {
            return;
        };
        if marker.opcode != NOP
            || !marker.fusion_policy.is_converted_pop_block()
            || marker.location.line >= 0
            || instruction_offset == 0
            || self
                .exception_regions
                .iter()
                .any(|region| entries.get(&region.target) == Some(&instruction_offset))
            || self
                .exception_owner_provenance
                .setups
                .iter()
                .any(|(label, _)| entries.get(label) == Some(&instruction_offset))
            || !instructions
                .get(instruction_offset - 1)
                .is_some_and(|(_, previous)| {
                    ends_scope(previous.opcode) || is_unconditional_jump(previous.opcode)
                })
        {
            return;
        }
        // Native pre-line inlining can remove an unconditional predecessor when
        // its target is a small exit or has no locations. Admit only a completed,
        // located normal block; stop at its first terminator or label.
        let direct_folded_entry = entries.iter().any(|(label, &entry)| {
            entry == instruction_offset
                && (self.folded_away_condition_targets.contains(label)
                    || self.folded_away_loop_else_targets.contains(label))
        });
        let mut located = false;
        let mut minimum_original_size = 1;
        let mut normal_jump = false;
        let mut fallthrough_boundary = false;
        let mut conditional_boundary = false;
        for item in &items[marker_item + 1..] {
            let Item::Instruction(instruction) = item else {
                fallthrough_boundary = true;
                break;
            };
            if instruction.opcode == NOT_TAKEN || ends_scope(instruction.opcode) {
                break;
            }
            if is_conditional_jump(instruction.opcode) {
                conditional_boundary = true;
                break;
            }
            if is_unconditional_jump(instruction.opcode) {
                minimum_original_size += 1;
                normal_jump = true;
                break;
            }
            // Ignore other NOPs and late cell/closure setup insertions rather
            // than count them as original instructions or source locations.
            if !matches!(instruction.opcode, NOP | COPY_FREE_VARS | MAKE_CELL) {
                minimum_original_size += 1;
                located |= instruction.location.line >= 0;
            }
        }
        // Inlining replaces a jump with NOP and appends instructions. A block
        // already larger than native MAX_COPY_SIZE cannot become a small exit
        // when its own successor is copied during the pre-line fixed point.
        // A conditional terminator cannot enter that inlining path either.
        // Its direct folded predecessor still prevents a sole break location donor,
        // even when the conditional block is smaller than MAX_COPY_SIZE.
        if !located
            || !(normal_jump
                || (direct_folded_entry && (fallthrough_boundary || conditional_boundary)))
            || (!conditional_boundary && minimum_original_size <= 4)
        {
            return;
        }
        let mut conditional = false;
        let mut folded_bridge = false;
        let mut direct_unconditional = false;
        for (index, (_, instruction)) in instructions.iter().enumerate() {
            let (Operand::Forward(target) | Operand::Backward(target)) = instruction.operand()
            else {
                continue;
            };
            if entries.get(&target) != Some(&instruction_offset) {
                continue;
            }
            if is_conditional_jump(instruction.opcode) {
                conditional = true;
                continue;
            }
            // A direct folded edge remains an original predecessor alongside
            // a live break jump. The completed target cannot receive an early
            // cleanup copy: it is located and either exceeds MAX_COPY_SIZE
            // or ends in a conditional jump instead of a scope exit.
            if direct_folded_entry && is_unconditional_jump(instruction.opcode) {
                direct_unconditional = true;
                continue;
            }
            // This single-jump block has no emitted predecessor or fallthrough.
            // Its only demonstrated original entry is the folded conditional.
            if instruction.opcode != JUMP_FORWARD
                || index == 0
                || self
                    .exception_regions
                    .iter()
                    .any(|region| entries.get(&region.target) == Some(&index))
                || self
                    .exception_owner_provenance
                    .setups
                    .iter()
                    .any(|(label, _)| entries.get(label) == Some(&index))
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.local_policy.hidden_exception_owner_slots() != 0
                || !entries.iter().any(|(label, &entry)| {
                    entry == index && self.folded_away_condition_targets.contains(label)
                })
                || entries.iter().any(|(label, &entry)| {
                    entry == index && self.original_nop_targets.contains(label)
                })
                || !instructions.get(index - 1).is_some_and(|(_, previous)| {
                    ends_scope(previous.opcode) || is_unconditional_jump(previous.opcode)
                }) || instructions.iter().any(|(_, predecessor)| {
                matches!(predecessor.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if entries.get(&label) == Some(&index))
            }) {
                return;
            }
            folded_bridge = true;
        }
        if (conditional && folded_bridge) || (direct_folded_entry && direct_unconditional) {
            // A break within this try keeps its handler when its normal-exit
            // target cannot be copied. Cancel only a jump-only exclusion whose
            // exact authored break enters this completed original block.
            try_exclusions.retain(|&(start, end)| {
                let Some(&start) = entries.get(&start) else {
                    return true;
                };
                if entries.get(&end) != Some(&(start + 1)) {
                    return true;
                }
                let (_, instruction) = instructions[start];
                !(instruction.opcode == JUMP_FORWARD
                    && instruction.fusion_policy.is_authored_break()
                    && matches!(instruction.operand(), Operand::Forward(target)
                        if entries.get(&target) == Some(&instruction_offset)))
            });
            let Item::Instruction(marker) = &mut self.linear_items_mut()[marker_item] else {
                unreachable!("recorded POP_BLOCK marker must remain an instruction");
            };
            marker.no_location_policy.preserve_location();
        }
    }

    /// Keeps a manager's original `POP_BLOCK` unlocated after a folded loop exit.
    ///
    /// Before constant folding, an ordinary try's unlocated two-slot exit is copied
    /// onto its break predecessor. The folded conditional still enters the original
    /// exit, so the following manager cleanup has no sole location donor. Certify that
    /// original graph while its labels and emitted instruction slots are still intact.
    pub(crate) fn preserve_with_pop_after_folded_try_bridge(&mut self, instruction_offset: usize) {
        let items = self.linear_items();
        let mut instructions = Vec::new();
        let mut entries = FxHashMap::default();
        let mut labels = Vec::new();
        for (item_index, item) in items.iter().enumerate() {
            match item {
                Item::Label(label) => {
                    entries.insert(*label, instructions.len());
                    labels.push((*label, instructions.len()));
                }
                Item::Instruction(instruction) => {
                    instructions.push((item_index, *instruction));
                }
            }
        }
        let Some(&(marker_item, marker)) = instructions.get(instruction_offset) else {
            return;
        };
        if marker.opcode != NOP
            || marker.operand() != Operand::Value(0)
            || !marker.fusion_policy.is_converted_pop_block()
            || marker.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || marker.location != SourceLocation::NONE
            || marker.jump_policy != ir::JumpPolicy::default()
            || marker.no_location_policy != ir::NoLocationPolicy::default()
            || marker.pre_fold_extra_instructions() != 0
            || marker.local_policy.hidden_exception_owner_slots() != 0
        {
            return;
        }
        let is_handler_or_setup = |index| {
            self.exception_regions
                .iter()
                .any(|region| entries.get(&region.target) == Some(&index))
                || self
                    .exception_owner_provenance
                    .setups
                    .iter()
                    .any(|(label, _)| entries.get(label) == Some(&index))
        };
        if is_handler_or_setup(instruction_offset) {
            return;
        }
        // The manager cleanup must remain a located, non-copyable original block.
        // Ignore only bookkeeping aliases; a real successor starts another array.
        let mut located = false;
        let mut minimum_original_size = 1;
        let mut complete_exit = false;
        for item in &items[marker_item + 1..] {
            let instruction = match item {
                Item::Label(label) => {
                    if self.preserved_block_boundaries.contains(label)
                        || self.exception_owner_provenance.block_boundaries.contains(label)
                        || self.exception_regions.iter().any(|region| region.target == *label)
                        || instructions.iter().any(|(_, instruction)| {
                            matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                                if target == *label)
                        })
                    {
                        break;
                    }
                    continue;
                }
                Item::Instruction(instruction) => instruction,
            };
            if instruction.pre_fold_extra_instructions() != 0
                || instruction.local_policy.hidden_exception_owner_slots() != 0
                || is_conditional_jump(instruction.opcode)
            {
                return;
            }
            if !matches!(instruction.opcode, NOP | COPY_FREE_VARS | MAKE_CELL) {
                minimum_original_size += 1;
                located |= instruction.location.line >= 0;
            }
            if is_unconditional_jump(instruction.opcode) || ends_scope(instruction.opcode) {
                complete_exit = true;
                break;
            }
        }
        if !complete_exit || !located || minimum_original_size <= 4 {
            return;
        }
        let ordinary_jump_policy = |instruction: &Instruction| {
            let mut expected = ir::JumpPolicy::default();
            if instruction.jump_policy.is_interruptible() {
                expected.mark_interruptible();
            }
            instruction.jump_policy == expected
        };
        let mut conditional_break_policy = ir::JumpPolicy::default();
        conditional_break_policy.prevent_threading_target();
        conditional_break_policy.retain_nop_for_direct_target();
        let mut certified = false;
        for &(entry, target) in &self.exception_owner_provenance.normal_try_exits {
            let Some(&bridge_index) = entries.get(&entry) else {
                continue;
            };
            if entries.get(&target) != Some(&instruction_offset)
                || bridge_index == 0
                || bridge_index >= instruction_offset
                || is_handler_or_setup(bridge_index)
                || !labels.iter().any(|&(label, index)| {
                    index == bridge_index && self.folded_away_condition_targets.contains(&label)
                })
                || labels.iter().any(|&(label, index)| {
                    index == bridge_index
                        && (self.original_nop_targets.contains(&label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_counts
                                .contains_key(&label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_locations
                                .contains_key(&label))
                })
            {
                continue;
            }
            let (_, bridge) = instructions[bridge_index];
            let (_, previous) = instructions[bridge_index - 1];
            if bridge.opcode != JUMP_FORWARD
                || bridge.operand() != Operand::Forward(target)
                || bridge.pre_fold_extra_instructions() != 0
                || bridge.local_policy.hidden_exception_owner_slots() != 0
                || !ordinary_jump_policy(&bridge)
                || bridge.no_location_policy.preserves_location()
                || !(is_unconditional_jump(previous.opcode) || ends_scope(previous.opcode))
            {
                continue;
            }
            let mut has_break = false;
            let mut all_predecessors_copyable = true;
            for (index, (_, instruction)) in instructions.iter().enumerate() {
                let (Operand::Forward(label) | Operand::Backward(label)) = instruction.operand()
                else {
                    continue;
                };
                if entries.get(&label) != Some(&bridge_index) {
                    continue;
                }
                // A next-instruction edge can disappear before the original copy round.
                // Admit only the ordinary forward break edges whose complete bridge is copied.
                if !is_unconditional_jump(instruction.opcode)
                    || !matches!(instruction.operand(), Operand::Forward(_))
                    || !instruction.fusion_policy.is_authored_break()
                    // Ordinary conditional breaks preserve their source jump's NOP and
                    // reject incoming threading. Neither flag prevents copying their target.
                    || !(ordinary_jump_policy(instruction)
                        || instruction.jump_policy == conditional_break_policy)
                    || instruction.location.line < 0
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.local_policy.hidden_exception_owner_slots() != 0
                    || index + 1 >= bridge_index
                {
                    all_predecessors_copyable = false;
                    break;
                }
                has_break = true;
            }
            if has_break && all_predecessors_copyable {
                certified = true;
                break;
            }
        }
        if certified {
            let Item::Instruction(marker) = &mut self.linear_items_mut()[marker_item] else {
                unreachable!("certified manager POP_BLOCK must remain an instruction");
            };
            // Normal NOP compaction still owns removal and its vacated owner slot.
            marker.no_location_policy.preserve_location();
        }
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

    /// Whether an emitted forward break reaches this loop's exit label.
    pub(crate) fn has_authored_forward_break_to(&self, label: Label) -> bool {
        self.linear_items().iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if instruction.opcode == JUMP_FORWARD
                    && instruction.fusion_policy.is_authored_break()
                    && instruction.operand() == Operand::Forward(label))
        })
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

    /// Records an empty folded-condition entry without confusing pending label aliases.
    pub(crate) fn record_folded_condition_borrow_entry(&mut self, target: Label) {
        let labels = self
            .linear_items()
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<rustc_hash::FxHashSet<_>>();
        if labels.contains(&target) {
            return;
        }
        let mut sources = 0;
        let mut other_pending_targets = Vec::new();
        for item in self.linear_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
                && !labels.contains(&label)
            {
                if label == target {
                    sources += 1;
                } else {
                    other_pending_targets.push(label);
                }
            }
        }
        if sources != 1 {
            return;
        }
        if other_pending_targets.is_empty() {
            self.prevent_borrow_fallthrough_reachability(target);
        } else {
            self.deferred_borrow_entries.push(ir::DeferredBorrowEntry {
                target,
                other_pending_targets,
            });
        }
    }

    /// Resolves pending entries using label groups before any assembler optimization.
    fn resolve_folded_condition_borrow_entries(&mut self) {
        self.resolve_name_return_borrow_exits();
        if self.deferred_borrow_entries.is_empty() {
            return;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            unreachable!("pending folded condition entries must precede graph preparation");
        };
        let mut offsets = FxHashMap::default();
        let mut offset = 0usize;
        for item in items {
            match item {
                Item::Label(label) => {
                    offsets.insert(*label, offset);
                }
                Item::Instruction(_) => offset += 1,
            }
        }
        for entry in std::mem::take(&mut self.deferred_borrow_entries) {
            let Some(target_offset) = offsets.get(&entry.target) else {
                continue;
            };
            // A preceding live edge can later acquire a different label at this same
            // entry, notably when an inner Boolean condition ends at its outer join.
            // Do not count branches in the discarded remaining operands as independent
            // predecessors: the pending destinations were captured before those operands.
            if entry.other_pending_targets.iter().all(|label| {
                offsets
                    .get(label)
                    .is_some_and(|offset| offset != target_offset)
            }) {
                self.prevent_borrow_fallthrough_reachability(entry.target);
            }
        }
    }

    /// Records an instruction boundary while compiler emission is still in progress.
    pub(crate) fn emitted_region_start(&self) -> EmittedRegionStart {
        EmittedRegionStart {
            instruction_count: self.instruction_count(),
            stream: self.emission_stream,
        }
    }

    /// Whether a normal path through this region reaches its final fallthrough edge.
    ///
    /// Jumps to trailing labels do not establish fallthrough from the last instruction.
    /// Only ordinary edges inside the region are followed; exception entries and external
    /// targets cannot certify this edge. Removing an instruction invalidates the boundary.
    /// An instruction-empty region falls through when its emission stream is unchanged.
    pub(crate) fn emitted_region_has_final_fallthrough(&self, start: EmittedRegionStart) -> bool {
        if start.stream != self.emission_stream {
            return false;
        }
        let items = self.linear_items();
        let Some(first) = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| matches!(item, Item::Instruction(_)).then_some(index))
            .nth(start.instruction_count)
        else {
            // An unchanged empty region has no instruction that can stop fallthrough.
            return start.instruction_count == self.instruction_count();
        };
        let items = &items[first..];
        let last = items
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .expect("the emitted region starts with an instruction");
        let labels = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let mut pending = vec![0];
        let mut seen = vec![false; items.len()];
        while let Some(index) = pending.pop() {
            if std::mem::replace(&mut seen[index], true) {
                continue;
            }
            let fallthrough = match &items[index] {
                Item::Label(_) => true,
                Item::Instruction(instruction) => {
                    if (is_unconditional_jump(instruction.opcode)
                        || is_conditional_jump(instruction.opcode)
                        || matches!(instruction.opcode, FOR_ITER | SEND))
                        && let Operand::Forward(target) | Operand::Backward(target) =
                            instruction.operand()
                        && let Some(&target) = labels.get(&target)
                    {
                        pending.push(target);
                    }
                    !is_unconditional_jump(instruction.opcode) && !ends_scope(instruction.opcode)
                }
            };
            if fallthrough {
                if index == last {
                    return true;
                }
                if index + 1 < items.len() {
                    pending.push(index + 1);
                }
            }
        }
        false
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

    /// Counts the current original array before constant folding and global-call fusion.
    pub(crate) fn current_original_block_pre_fusion_instruction_count(&self) -> usize {
        let mut count = 0;
        for item in self.linear_items().iter().rev() {
            match item {
                Item::Label(label) => {
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label)
                        || self.preserved_block_boundaries.contains(label)
                    {
                        break;
                    }
                }
                Item::Instruction(instruction) => {
                    if instruction.opcode == NOT_TAKEN
                        || matches!(
                            instruction.operand(),
                            Operand::Forward(_) | Operand::Backward(_)
                        )
                        || ends_scope(instruction.opcode)
                    {
                        break;
                    }
                    count += 1
                        + usize::from(instruction.pre_fold_extra_instructions())
                        + usize::from(
                            instruction.opcode == LOAD_GLOBAL
                                && matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0),
                        );
                }
            }
        }
        count
    }

    /// Certifies a single comprehension restore followed by its result assignment.
    pub(crate) fn current_original_block_is_single_comprehension_assignment(&self) -> bool {
        let [
            ..,
            Item::Label(entry),
            Item::Instruction(swap),
            Item::Instruction(restore),
            Item::Instruction(store),
        ] = self.linear_items()
        else {
            return false;
        };
        swap.opcode == SWAP
            && swap.operand() == Operand::Value(2)
            && restore.opcode == STORE_FAST
            && restore.local_policy.may_store_null()
            && store.opcode == STORE_FAST
            && !store.local_policy.may_store_null()
            && restore.operand() != store.operand()
            && self
                .exception_owner_provenance
                .original_comprehension_normal_restores
                .get(entry)
                .is_some_and(|original| original.as_slice() == [*swap, *restore])
            && self
                .exception_owner_provenance
                .original_comprehension_normal_exits
                .iter()
                .any(|(target, jump)| {
                    target == entry
                        && jump.opcode == JUMP_FORWARD
                        && jump.operand() == Operand::Forward(*entry)
                })
            && [swap, restore, store].into_iter().all(|instruction| {
                instruction.pre_fold_extra_instructions() == 0
                    && !instruction.jump_policy.borrow_empty_block()
                    && instruction.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Reachable
            })
    }

    /// Captures the original assignment donor before its comprehension restore is exchanged.
    pub(crate) fn record_comprehension_assignment_exit(&mut self, exit: Label) {
        if !self.current_original_block_is_single_comprehension_assignment() {
            return;
        }
        let items = self.linear_items();
        let [
            ..,
            Item::Label(_),
            Item::Instruction(swap),
            Item::Instruction(restore),
            Item::Instruction(store),
        ] = items
        else {
            unreachable!("certified comprehension assignment array");
        };
        let instructions = [*swap, *restore, *store];
        // The cold cleanup's end can share the normal entry's original offset.
        // Retain that entire emitted label prefix, including its exact order.
        let mut entries = items[..items.len() - 3]
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        entries.reverse();
        self.comprehension_assignment_exits
            .insert(exit, (entries, instructions));
    }

    /// Certifies that the current empty join follows its captured comprehension assignment.
    pub(crate) fn at_comprehension_assignment_exit(&self) -> bool {
        let items = self.linear_items();
        let entry_len = items
            .iter()
            .rev()
            .take_while(|item| matches!(item, Item::Label(_)))
            .count();
        let source_end = items.len() - entry_len;
        items[source_end..].iter().any(|item| {
            let Item::Label(exit) = item else {
                return false;
            };
            let Some((entries, instructions)) = self.comprehension_assignment_exits.get(exit)
            else {
                return false;
            };
            let Some(source_start) = source_end.checked_sub(entries.len() + instructions.len())
            else {
                return false;
            };
            items[source_start..source_end].iter().copied().eq(entries
                .iter()
                .copied()
                .map(Item::Label)
                .chain(instructions.iter().copied().map(Item::Instruction)))
        })
    }

    /// Records the real entry and cleanup owner of a later typed-handler dispatch.
    pub(crate) fn record_handler_dispatch_array(
        &mut self,
        start: usize,
        target: Label,
        depth: u32,
    ) {
        let Some(entry_index) = start.checked_sub(1) else {
            return;
        };
        let Some(Item::Label(entry)) = self.linear_items().get(entry_index).copied() else {
            return;
        };
        let items = self.linear_items()[entry_index..].to_vec();
        self.exception_owner_provenance
            .handler_dispatch_arrays
            .push(exception_owners::HandlerDispatchArray {
                entry,
                items,
                target,
                depth,
            });
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
            // A fallthrough scan starts just after its marker. Include that original
            // prefix in the certificate, without counting its already-consumed slot.
            let capture_start = index
                .checked_sub(1)
                .filter(|previous| {
                    instruction.opcode == SWAP
                        && matches!(items[*previous], Item::Instruction(marker)
                            if marker.opcode == NOT_TAKEN)
                })
                .unwrap_or(index);
            if let Some(rotation) = capture_cleanup_rotation(&items[capture_start..]) {
                let consumed = 1 + rotation.swap_count + rotation.consumers.len();
                slots += rotation.removed_slots;
                for item in &items[index..capture_start + consumed] {
                    let Item::Instruction(instruction) = item else {
                        unreachable!("certified rotation contains only instructions");
                    };
                    slots += usize::from(instruction.local_policy.hidden_exception_owner_slots());
                }
                // Applying descending swaps rotates the last capture ahead of all other
                // consumers. Preserve that order when predicting greedy local-pair fusion.
                let reordered = rotation
                    .consumers
                    .last()
                    .into_iter()
                    .chain(rotation.consumers[..rotation.consumers.len() - 1].iter())
                    .map(|item| {
                        let Item::Instruction(instruction) = item else {
                            unreachable!("certified rotation contains only instructions");
                        };
                        *instruction
                    })
                    .collect::<Vec<_>>();
                let mut consumer = 0;
                while consumer + 1 < reordered.len() {
                    if will_fuse(reordered[consumer], reordered[consumer + 1]) {
                        slots += 1;
                        consumer += 2;
                    } else {
                        consumer += 1;
                    }
                }
                index = capture_start + consumed;
                continue;
            }
            if let Some(rotation) = index
                .checked_sub(1)
                .and_then(|previous| items.get(previous..index + 5))
                && let Some(vacancies) = single_capture_cleanup_vacancies(rotation)
            {
                // Consume the complete rotation: after the store moves, neither trailing pop
                // can fuse with the following instruction. Hidden original slots still count.
                slots += vacancies;
                for item in &rotation[1..] {
                    let Item::Instruction(instruction) = item else {
                        unreachable!("certified rotation contains only instructions");
                    };
                    slots += usize::from(instruction.local_policy.hidden_exception_owner_slots());
                }
                index += 5;
                continue;
            }
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

    /// Resolves the first condition slot in an exact loop-exit normal-finally prefix.
    pub(crate) fn current_normal_finally_condition_slot_owner(&self) -> Option<bool> {
        use crate::target::opcodes::{
            CALL, LOAD_ATTR, LOAD_DEREF, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, TO_BOOL,
        };

        let items = self.linear_items();
        let truth = matches!(items.last(), Some(Item::Instruction(instruction))
            if matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                && instruction.local_policy.hidden_exception_owner_slots() == 0);
        if !truth
            && !matches!(items.last(), Some(Item::Instruction(instruction))
            if matches!(instruction.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                && instruction.local_policy.hidden_exception_owner_slots() == 3)
        {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let begin = items.iter().rposition(|item| {
            matches!(item, Item::Label(label)
            if provenance.source_boundaries.contains(label)
                && provenance.block_boundaries.contains(label))
        })?;
        let prefix = &items[begin..];
        let mut leading_labels = FxHashSet::default();
        let mut labels = FxHashSet::default();
        let mut instructions = Vec::new();
        for item in prefix {
            match item {
                Item::Label(label) => {
                    if instructions.len() > if truth { 2 } else { 1 }
                        || self.preserved_block_boundaries.contains(label)
                        || provenance.setups.iter().any(|(setup, _)| setup == label)
                        || provenance.fusion_boundaries.contains(label)
                        || provenance.omitted_nop_counts.contains_key(label)
                        || provenance.omitted_nop_locations.contains_key(label)
                        || self
                            .exception_regions
                            .iter()
                            .any(|region| region.target == *label)
                        || (!instructions.is_empty() && provenance.block_boundaries.contains(label))
                    {
                        return None;
                    }
                    labels.insert(*label);
                    if instructions.is_empty() {
                        leading_labels.insert(*label);
                    }
                }
                Item::Instruction(instruction) => {
                    if instructions.len() == 8 {
                        return None;
                    }
                    instructions.push(*instruction);
                }
            }
        }
        // The first attribute test can follow a single converted POP_BLOCK. Its
        // range labels do not start an array; deleting that cell leaves the
        // appended marker in a protected condition slot. Defer the final owner
        // to the normal replay, including its retained-NULL-array proof.
        let single_pop_truth = match instructions.as_slice() {
            [pop, load, attribute, convert, condition]
                if truth
                    && attribute.opcode == LOAD_ATTR
                    && matches!(attribute.operand(), Operand::Value(argument) if argument & 1 == 0) =>
            {
                Some((*pop, *load, *convert, *condition, None))
            }
            [pop, load, method, call, convert, condition]
                if truth
                    && method.opcode == LOAD_ATTR
                    && matches!(method.operand(), Operand::Value(argument) if argument & 1 != 0)
                    && call.opcode == CALL
                    && call.operand() == Operand::Value(0)
                    && call.location == condition.location =>
            {
                Some((*pop, *load, *convert, *condition, Some(*method)))
            }
            _ => None,
        };
        if let Some((pop, load, convert, condition, method)) = single_pop_truth
            && pop.opcode == NOP
            && pop.operand() == Operand::Value(0)
            && pop.location == SourceLocation::NONE
            && pop.fusion_policy.is_converted_pop_block()
            && pop.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
            && load.opcode == LOAD_FAST
            && matches!(load.operand(), Operand::Value(_))
            && convert.opcode == TO_BOOL
            && convert.operand() == Operand::Value(0)
            && convert.location == condition.location
            && condition.location.line >= 0
            && matches!(condition.operand(), Operand::Forward(_))
            && instructions[1..].iter().all(|record| {
                record.location.line == condition.location.line
                    && record.location.end_line == condition.location.end_line
                    && record.location.column == condition.location.column
                    && record.location.end_column <= condition.location.end_column
                    && record.exception_policy() == ExceptionPolicy::default()
                    && !record.fusion_policy.is_converted_pop_block()
            })
            && instructions.iter().all(|record| {
                record.pre_fold_extra_instructions() == 0
                    && record.local_policy.hidden_exception_owner_slots() == 0
                    && record.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Reachable
            })
            && instructions[..instructions.len() - 1]
                .iter()
                .all(|record| matches!(record.operand(), Operand::Value(_)))
            && pop.depth_after().is_some()
            && condition.depth_after() == pop.depth_after()
            && load.depth_after() == pop.depth_after().map(|depth| depth + 1)
            && convert.depth_after() == load.depth_after()
            && method.is_none_or(|method| {
                method.depth_after() == pop.depth_after().map(|depth| depth + 2)
            })
            && provenance
                .original_primitive_condition_boundaries
                .iter()
                .filter(|original| **original == condition)
                .count()
                == 1
            && prefix[prefix
                .iter()
                .rposition(|item| matches!(item, Item::Label(_)))
                .map_or(0, |index| index + 1)..]
                .iter()
                .copied()
                .eq(instructions[1..].iter().copied().map(Item::Instruction))
            && matches!(items[..begin].iter().rev().find(|item| matches!(item, Item::Instruction(_))),
                Some(Item::Instruction(previous))
                    if ends_scope(previous.opcode)
                        || (previous.opcode == crate::target::opcodes::END_ASYNC_FOR
                            && matches!(previous.operand(), Operand::Backward(_))))
        {
            let incoming = items.iter().filter_map(|item| match item {
                Item::Instruction(jump)
                    if matches!(jump.operand(), Operand::Forward(target) | Operand::Backward(target)
                        if labels.contains(&target)) => Some(*jump),
                _ => None,
            }).collect::<Vec<_>>();
            if !incoming.is_empty()
                && incoming.iter().all(|jump| {
                    is_unconditional_jump(jump.opcode)
                        && matches!(jump.operand(), Operand::Forward(target) if leading_labels.contains(&target))
                })
            {
                return Some(true);
            }
        }
        let (plain, pop, owns_slot) = match instructions.as_slice() {
            [plain, pop, receiver, method, call, discard, load, condition]
                if !truth
                    && matches!(receiver.opcode, LOAD_FAST | LOAD_DEREF)
                    && method.opcode == LOAD_ATTR
                    && matches!(method.operand(), Operand::Value(argument) if argument & 1 != 0)
                    && call.opcode == CALL
                    && call.operand() == Operand::Value(0)
                    && discard.opcode == POP_TOP
                    && matches!(load.opcode, LOAD_FAST | LOAD_DEREF)
                    && matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                    && matches!(condition.operand(), Operand::Forward(_)) =>
            {
                (*plain, *pop, true)
            }
            [plain, pop, load, convert, condition]
                if truth
                    && matches!(load.opcode, LOAD_FAST | LOAD_DEREF)
                    && convert.opcode == TO_BOOL
                    && convert.operand() == Operand::Value(0)
                    && matches!(condition.operand(), Operand::Forward(_))
                    && condition.location.line >= 0
                    && load.location == condition.location
                    && convert.location == condition.location
                    && plain.location.line != condition.location.line
                    && provenance
                        .original_primitive_condition_boundaries
                        .iter()
                        .filter(|original| **original == *condition)
                        .count()
                        == 1 =>
            {
                (*plain, *pop, false)
            }
            _ => return None,
        };
        if plain.opcode != NOP
            || plain.operand() != Operand::Value(0)
            || plain.location.line < 0
            || plain.fusion_policy != ir::FusionPolicy::default()
            || pop.opcode != NOP
            || pop.operand() != Operand::Value(0)
            || pop.location != SourceLocation::NONE
            || !pop.fusion_policy.is_converted_pop_block()
            || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || instructions.iter().enumerate().any(|(index, instruction)| {
                instruction.pre_fold_extra_instructions() != 0
                    || instruction.local_policy.hidden_exception_owner_slots()
                        != if owns_slot && index == 7 { 3 } else { 0 }
                    || instruction.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::Reachable
                    || (index != 1 && instruction.exception_policy() != ExceptionPolicy::default())
                    || (index + 1 != instructions.len()
                        && !matches!(instruction.operand(), Operand::Value(_)))
            })
        {
            return None;
        }
        let incoming = items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(jump)
                    if matches!(jump.operand(), Operand::Forward(target) | Operand::Backward(target)
                    if labels.contains(&target)) =>
                {
                    Some(*jump)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [entry] = incoming.as_slice() else {
            return None;
        };
        if !is_conditional_jump(entry.opcode)
            || !matches!(entry.operand(), Operand::Forward(target) if leading_labels.contains(&target))
            || entry.location != plain.location
            || !matches!(items[..begin].iter().rev().find(|item| matches!(item, Item::Instruction(_))),
                Some(Item::Instruction(backedge))
                    if is_unconditional_jump(backedge.opcode)
                        && matches!(backedge.operand(), Operand::Backward(_)))
        {
            return None;
        }
        // The call/None prefix keeps its proven owned comparison slot. A direct
        // truth prefix has no call or comparison compaction: its first marker
        // retains the normal-finally NULL slot after the two leading NOPs merge.
        Some(owns_slot)
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

    /// Whether the next instruction starts at the recorded source-authored match end.
    pub(crate) fn at_original_match_join(&self) -> bool {
        self.original_match_join.is_some_and(|join| {
            self.linear_items()
                .iter()
                .rev()
                .take_while(|item| matches!(item, Item::Label(_)))
                .any(|item| matches!(item, Item::Label(label) if *label == join))
        })
    }

    /// Records the source-authored match end until its first instruction is emitted.
    pub(crate) fn mark_original_match_join(&mut self, label: Label) {
        let items = self.linear_items();
        let origin = match items {
            [
                ..,
                Item::Instruction(swap),
                Item::Instruction(first),
                Item::Instruction(last),
            ] if swap.opcode == SWAP
                && swap.operand() == Operand::Value(2)
                && first.opcode == STORE_FAST
                && last.opcode == STORE_FAST
                && !first.local_policy.may_store_null()
                && !last.local_policy.may_store_null()
                && matches!((first.operand(), last.operand()),
                        (Operand::Value(first), Operand::Value(last)) if first != last)
                && first.location.line >= 0
                && first.location.line == last.location.line =>
            {
                // Keep the entire original array, including every adjacent entry alias.
                // Static SWAP removal can exchange the last two stores before the
                // unlocated match cleanup receives its remaining fallthrough donor.
                items
                    .iter()
                    .rposition(|item| matches!(item, Item::Label(_)))
                    .map(|mut start| {
                        while start > 0 && matches!(items[start - 1], Item::Label(_)) {
                            start -= 1;
                        }
                        items[start..].to_vec()
                    })
            }
            _ => None,
        };
        // CPython's case-success jumps also establish this array when every
        // successful body terminates and lowering omits those unreachable edges.
        // Live success edges already represent the join and keep their own policies.
        let has_success_edge = items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if is_unconditional_jump(instruction.opcode)
                    && instruction.operand() == Operand::Forward(label))
        });
        if !has_success_edge {
            self.record_exception_owner_block_boundary(label);
        }
        self.mark(label);
        self.exception_owner_provenance
            .original_match_join_boundaries
            .insert(label);
        self.original_match_join = Some(label);
        if let Some(origin) = origin {
            self.match_join_store_origins.insert(label, origin);
        }
    }

    /// Retains an adjacent constant/Pass pair without inventing an original block entry.
    ///
    /// The caller has just compiled a constant expression and is about to emit Pass.
    /// Both existing labels must be fresh neighbors of that constant's single NOP.
    pub(crate) fn record_constant_pass_cluster(&mut self, pass_location: SourceLocation) -> bool {
        let items = self.linear_items();
        let Some(tail) = items.len().checked_sub(4).map(|start| &items[start..]) else {
            return false;
        };
        let [
            Item::Instruction(previous),
            Item::Label(start),
            Item::Instruction(constant),
            Item::Label(end),
        ] = tail
        else {
            return false;
        };
        if previous.opcode == NOP
            || previous.opcode == NOT_TAKEN
            || !matches!(previous.operand(), Operand::Value(_))
            || ends_scope(previous.opcode)
            || constant.opcode != NOP
            || constant.operand() != Operand::Value(0)
            || constant.location.line < 0
            || constant.location.line != pass_location.line
            || constant.fusion_policy.is_converted_pop_block()
            || constant.jump_policy.preserves_explicit_nop()
            || constant.jump_policy.borrow_empty_block()
            || constant.pre_fold_extra_instructions() != 0
            || constant.local_policy.hidden_exception_owner_slots() != 0
        {
            return false;
        }
        let cluster = exception_owners::ConstantPassCluster {
            start: *start,
            end: *end,
            constant_location: constant.location,
            pass_location,
        };
        self.exception_owner_provenance
            .constant_pass_clusters
            .push(cluster);
        true
    }

    /// Retains the original NOP head when statement lowering omits the instruction.
    pub(crate) fn record_omitted_nop_targets(&mut self, location: SourceLocation) {
        let labels = self
            .linear_items()
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if labels.is_empty()
            && let Some(Item::Instruction(previous)) = self.linear_items().last()
            && location.line >= 0
            && previous.location.line != location.line
            && !matches!(previous.opcode, NOP | NOT_TAKEN)
            && matches!(previous.operand(), Operand::Value(_))
            && !ends_scope(previous.opcode)
            && !previous.fusion_policy.is_converted_pop_block()
            && previous.pre_fold_extra_instructions() == 0
            && previous.local_policy.hidden_exception_owner_slots() == 0
        {
            let position = self.item_count();
            let count = self
                .exception_owner_provenance
                .omitted_nop_tail
                .filter(|(old_position, _, _)| *old_position == position)
                .map_or(1, |(_, _, count)| count + 1);
            self.exception_owner_provenance.omitted_nop_tail = Some((position, location, count));
        }
        self.exception_owner_provenance
            .omitted_nop_locations
            .extend(labels.iter().copied().map(|label| (label, location)));
        for label in &labels {
            *self
                .exception_owner_provenance
                .omitted_nop_counts
                .entry(*label)
                .or_default() += 1;
        }
        self.original_nop_targets.extend(labels);
    }

    /// Checks an omitted fallthrough Pass immediately before its normal try-end label.
    pub(crate) fn has_single_omitted_fallthrough_nop_before_label(
        &self,
        boundary: Label,
        expected: SourceLocation,
    ) -> bool {
        matches!(
            self.exception_owner_provenance.omitted_nop_tail,
            Some((position, location, 1)) if location == expected
                && matches!(self.linear_items().get(position..), Some([Item::Label(label)]) if *label == boundary)
        )
    }

    /// Checks that one omitted authored NOP precedes a later label at the current head.
    pub(crate) fn has_single_omitted_nop_before_label(
        &self,
        boundary: Label,
        expected: SourceLocation,
    ) -> bool {
        if expected.line < 0
            || self
                .exception_owner_provenance
                .omitted_nop_counts
                .contains_key(&boundary)
        {
            return false;
        }
        let mut seen_boundary = false;
        let mut found = false;
        for label in self
            .linear_items()
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(label),
                Item::Instruction(_) => None,
            })
        {
            if *label == boundary {
                seen_boundary = true;
                continue;
            }
            let Some(count) = self
                .exception_owner_provenance
                .omitted_nop_counts
                .get(label)
            else {
                continue;
            };
            if !seen_boundary
                || *count != 1
                || !self.original_nop_targets.contains(label)
                || self
                    .exception_owner_provenance
                    .omitted_nop_locations
                    .get(label)
                    != Some(&expected)
            {
                return false;
            }
            found = true;
        }
        seen_boundary && found
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

    /// Records an original for-loop backedge array omitted by loop-tail lowering.
    pub(crate) fn record_omitted_for_backedge(&mut self, cleanup: Label) {
        self.borrow_omitted_for_backedges.insert(cleanup);
    }

    /// Keeps CPython's block-local borrowed-load optimizer from visiting this block.
    pub(crate) fn prevent_borrow_reachability(&mut self, label: Label) {
        self.borrow_unreachable_blocks.insert(label);
    }

    /// Stops empty handler exits while allowing surviving jumps into normal cleanup.
    pub(crate) fn prevent_borrow_finally_fallthrough(&mut self, label: Label) {
        self.prevent_borrow_reachability(label);
        self.borrow_finally_entries.insert(label, None);
    }

    /// Keeps an empty original fallthrough disconnected without blocking independent jumps.
    pub(crate) fn prevent_borrow_fallthrough_reachability(&mut self, label: Label) {
        self.prevent_borrow_reachability(label);
        self.borrow_jump_entry_labels.insert(label);
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

    /// Tracks whether removing a taken constant jump empties its original block.
    pub(crate) fn mark_folded_jump_borrow_fallthrough(
        &mut self,
        emitted: EmittedJump,
        empty_original_block: bool,
    ) {
        let instruction = self.emitted_jump_mut(emitted, "marking folded jump borrow fallthrough");
        if instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Reachable {
            instruction
                .no_location_policy
                .set_borrow_fallthrough(if empty_original_block {
                    BorrowFallthrough::BarrierIfRemoved
                } else {
                    BorrowFallthrough::RetainedNopReachable
                });
        }
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

    /// Captures the complete normal jump before emission proceeds into its finalizer.
    pub(crate) fn record_name_return_borrow_exit(
        &mut self,
        source: EmittedJump,
    ) -> NameReturnBorrowExit {
        NameReturnBorrowExit {
            source,
            original: *self.emitted_jump_mut(source, "recording a normal-finally borrow exit"),
        }
    }

    /// Defers a normal-edge exemption until its complete local-return target is emitted.
    pub(crate) fn defer_name_return_borrow_exit(&mut self, source: EmittedJump) {
        let origin = self.record_name_return_borrow_exit(source);
        self.deferred_name_return_borrow_exits.push(origin);
    }

    /// Marks exception coverage resuming after a return, without an original instruction.
    pub(crate) fn mark_return_region_reentry(&mut self, boundary: Label) {
        self.mark(boundary);
        self.exception_owner_provenance
            .return_region_reentries
            .insert(boundary);
    }

    /// Captures a copied finalizer's donor across only artificial iterator disposal.
    pub(crate) fn record_copied_finally_iterator_return(
        &mut self,
        cleanup_end: usize,
        iterators: usize,
    ) {
        let Some(start) = cleanup_end.checked_sub(2) else {
            return;
        };
        let Some(items) = self.linear_items().get(start..) else {
            return;
        };
        let [Item::Instruction(call), Item::Instruction(pop), tail @ ..] = items else {
            return;
        };
        if call.opcode != crate::target::opcodes::CALL
            || !matches!(call.operand(), Operand::Value(_))
            || call.location.line < 0
            || pop.opcode != POP_TOP
            || pop.operand() != Operand::Value(0)
            || pop.location != call.location
            || iterators == 0
            || !tail.iter().any(|item| matches!(item, Item::Label(_)))
        {
            return;
        }
        let mut offset = 0;
        for item in tail {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            let (opcode, operand) = if offset == 2 * iterators {
                (RETURN_VALUE, Operand::Value(0))
            } else if offset < 2 * iterators && offset.is_multiple_of(2) {
                (SWAP, Operand::Value(2))
            } else {
                (POP_TOP, Operand::Value(0))
            };
            if instruction.opcode != opcode
                || instruction.operand() != operand
                || instruction.location != SourceLocation::NONE
                || instruction.no_location_policy.preserves_location()
                || instruction.pre_fold_extra_instructions() != 0
            {
                return;
            }
            offset += 1;
        }
        if offset != 2 * iterators + 1
            || !matches!(items.last(), Some(Item::Instruction(instruction))
                if instruction.opcode == RETURN_VALUE)
        {
            return;
        }
        let origin = items.to_vec();
        self.exception_owner_provenance
            .copied_finally_iterator_returns
            .push(origin);
    }

    /// Records the first erased ordinary-try POP before a return's remaining unwind.
    pub(crate) fn record_name_return_pop_boundary(&mut self, boundary: Label) {
        if !self.deferred_name_return_borrow_exits.is_empty() {
            self.name_return_pop_boundaries.insert(boundary);
        }
    }

    /// Certifies a metadata-only iterator-region label beside the return's unwind label.
    pub(crate) fn record_name_return_iterator_region_alias(&mut self, boundary: Label) {
        if self.deferred_name_return_borrow_exits.is_empty() {
            return;
        }
        let original = match self.linear_items() {
            [.., Item::Label(original), Item::Label(alias)] if *alias == boundary => *original,
            _ => return,
        };
        self.name_return_iterator_region_aliases
            .insert(boundary, original);
    }

    /// Rejects copy-based borrowing only for a proven oversized original return array.
    fn resolve_name_return_borrow_exits(&mut self) {
        if self.deferred_name_return_borrow_exits.is_empty() {
            return;
        }
        let origins = std::mem::take(&mut self.deferred_name_return_borrow_exits);
        let pop_boundaries = std::mem::take(&mut self.name_return_pop_boundaries);
        let iterator_region_aliases = std::mem::take(&mut self.name_return_iterator_region_aliases);
        let items = self.linear_items();
        let labels = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let jump_targets = items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => match instruction.operand() {
                    Operand::Forward(label) | Operand::Backward(label) => Some(label),
                    _ => None,
                },
                Item::Label(_) => None,
            })
            .collect::<rustc_hash::FxHashSet<_>>();
        let mut barriers = Vec::new();
        'origin: for origin in origins {
            if origin.source.instruction().stream != self.emission_stream {
                continue;
            }
            let index = usize::try_from(origin.source.instruction().item_index)
                .expect("emitted instruction index fits in usize");
            let Some(Item::Instruction(jump)) = items.get(index) else {
                continue;
            };
            let Operand::Forward(target) = jump.operand() else {
                continue;
            };
            let Some(&target_index) = labels.get(&target) else {
                continue;
            };
            if *jump != origin.original || jump.opcode != JUMP_FORWARD || target_index <= index {
                continue;
            }
            let mut size = 0;
            let mut pending_pop = None;
            let mut needs_iterator_pop = false;
            for item in &items[target_index + 1..] {
                match item {
                    Item::Label(label) if size == 0 => {
                        if self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                            || self.preserved_block_boundaries.contains(label)
                        {
                            continue 'origin;
                        }
                    }
                    Item::Label(label) => {
                        if needs_iterator_pop
                            || jump_targets.contains(label)
                            || self
                                .exception_owner_provenance
                                .block_boundaries
                                .contains(label)
                            || self.preserved_block_boundaries.contains(label)
                        {
                            continue 'origin;
                        }
                        if let Some(original) = iterator_region_aliases.get(label) {
                            // This fresh exception-range label adds no original slot.
                            // Its producer must place it directly after the exact POP
                            // boundary already counted by this return traversal.
                            if pending_pop != Some(*original)
                                || !pop_boundaries.contains(original)
                                || labels.get(original).zip(labels.get(label)).is_none_or(
                                    |(original_index, alias_index)| {
                                        original_index + 1 != *alias_index
                                    },
                                )
                            {
                                continue 'origin;
                            }
                        } else {
                            // Only the producer's omitted ordinary POP can add a slot.
                            if pending_pop.is_some() || !pop_boundaries.contains(label) {
                                continue 'origin;
                            }
                            pending_pop = Some(*label);
                            size += 1;
                        }
                    }
                    Item::Instruction(instruction) => {
                        match (instruction.opcode, instruction.operand()) {
                            (LOAD_FAST, Operand::Value(_)) if size == 0 => {}
                            (SWAP, Operand::Value(2)) if size > 0 && !needs_iterator_pop => {
                                needs_iterator_pop = true;
                            }
                            (POP_TOP, Operand::Value(0)) if needs_iterator_pop => {
                                needs_iterator_pop = false;
                            }
                            (RETURN_VALUE, Operand::Value(0))
                                if size > 0 && !needs_iterator_pop => {}
                            _ => continue 'origin,
                        }
                        pending_pop = None;
                        size += 1 + usize::from(instruction.pre_fold_extra_instructions());
                        if instruction.opcode == RETURN_VALUE {
                            if instruction.location.line >= 0 && size > 4 {
                                barriers.push(index);
                            }
                            break;
                        }
                    }
                }
            }
        }
        for index in barriers {
            let Item::Instruction(jump) = &mut self.linear_items_mut()[index] else {
                unreachable!("the captured normal jump was checked above");
            };
            jump.no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        }
    }

    /// Stops an erased normal edge only when its emitted finally target cannot copy.
    pub(crate) fn mark_noncopyable_finally_borrow_barrier(
        &mut self,
        origin: NameReturnBorrowExit,
        ordinal: usize,
    ) {
        if origin.source.instruction().stream != self.emission_stream {
            return;
        }
        let items = self.linear_items();
        let Some(index) = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, Item::Instruction(_)))
            .nth(ordinal)
            .map(|(index, _)| index)
        else {
            return;
        };
        let Item::Instruction(marker) = items[index] else {
            return;
        };
        if marker.opcode != NOP
            || !marker.fusion_policy.is_converted_pop_block()
            || marker.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
        {
            return;
        }
        let source_index = usize::try_from(origin.source.instruction().item_index)
            .expect("emitted instruction index fits in usize");
        let Some(Item::Instruction(jump)) = items.get(source_index) else {
            return;
        };
        let Operand::Forward(target) = jump.operand() else {
            return;
        };
        if *jump != origin.original
            || jump.opcode != JUMP_FORWARD
            || source_index >= index
            || !items[..index]
                .iter()
                .rev()
                .take_while(|item| matches!(item, Item::Label(_)))
                .any(|item| matches!(item, Item::Label(label) if *label == target))
        {
            return;
        }

        // Count a lower bound in this original array. Foreign labels and jumps
        // require another provenance proof; omitted slots never reduce this bound.
        let mut minimum_size = 1;
        let mut located = false;
        let mut terminal = None;
        for item in &items[index + 1..] {
            let Item::Instruction(instruction) = item else {
                return;
            };
            if instruction.opcode == NOT_TAKEN || is_unconditional_jump(instruction.opcode) {
                return;
            }
            if !matches!(instruction.opcode, NOP | COPY_FREE_VARS | MAKE_CELL) {
                minimum_size += 1;
                located |= instruction.location.line >= 0;
            }
            if is_conditional_jump(instruction.opcode) || ends_scope(instruction.opcode) {
                terminal = Some(instruction.opcode);
                break;
            }
        }
        let noncopyable = match terminal {
            Some(opcode) if is_conditional_jump(opcode) => true,
            Some(_) => minimum_size > 4,
            // The linear finalizer still has its ordinary jump to the following
            // name return. Copying that return retains one replaced-jump slot,
            // followed by LOAD/RETURN, before CPython applies MAX_COPY_SIZE=4.
            None => minimum_size + 3 > 4,
        };
        if located && noncopyable {
            let Item::Instruction(jump) = &mut self.linear_items_mut()[source_index] else {
                unreachable!("the original normal jump was checked above");
            };
            jump.no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        }
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

    /// Marks a prior empty jump after its target's copy cost is known, preserving NOP policies.
    pub(crate) fn mark_borrow_empty_block_if_unretained_at(&mut self, instruction_offset: usize) {
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
        if !instruction
            .jump_policy
            .retains_nop_for(instruction.operand())
            && !instruction.jump_policy.preserves_nop_after_threading()
            && !instruction.jump_policy.preserves_explicit_nop()
        {
            instruction.jump_policy.mark_borrow_empty_block();
        }
    }

    /// Checks that two labels delimit only the last emitted instruction.
    pub(crate) fn range_is_last_instruction(
        &self,
        start: Label,
        end: Label,
        opcode: Opcode,
    ) -> bool {
        let items = self.linear_items();
        let Some(index) = items
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
        else {
            return false;
        };
        matches!(items[index], Item::Instruction(instruction) if instruction.opcode == opcode)
            && items[..index]
                .iter()
                .rev()
                .take_while(|item| matches!(item, Item::Label(_)))
                .any(|item| *item == Item::Label(start))
            && items[index + 1..].contains(&Item::Label(end))
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

    /// Moves one compiler-removed NULL `POP_BLOCK` to its original linear successor.
    ///
    /// The caller already counted that record on the protected predecessor. Keeping it
    /// virtual leaves authored omitted NOPs and synthesized leading locations intact.
    /// Only a contiguous original array can transfer the vacancy and certify its boundary.
    pub(crate) fn transfer_linear_try_pop_block_slot(&mut self, boundary: Label) {
        if self
            .exception_owner_provenance
            .removed_pop_block_boundaries
            .contains(&boundary)
        {
            return;
        }
        let items = self.linear_items();
        let Some(end) = items.iter().position(|item| *item == Item::Label(boundary)) else {
            return;
        };
        let Some((source, Item::Instruction(previous))) = items[..end]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, item)| matches!(item, Item::Instruction(_)))
        else {
            return;
        };
        let Some((offset, Item::Instruction(next))) = items[end + 1..]
            .iter()
            .enumerate()
            .find(|(_, item)| matches!(item, Item::Instruction(_)))
        else {
            return;
        };
        let target = end + 1 + offset;
        if matches!(previous.opcode, NOP | NOT_TAKEN)
            || !matches!(previous.operand(), Operand::Value(_))
            || ends_scope(previous.opcode)
            || previous.local_policy.hidden_exception_owner_slots() == 0
            || previous.local_policy.hidden_exception_owner_slots()
                == LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS
            || matches!(next.opcode, NOP | NOT_TAKEN)
            || !matches!(next.operand(), Operand::Value(_))
            || ends_scope(next.opcode)
            || next.local_policy.hidden_exception_owner_slots()
                == LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS
            || previous.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
            || next.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
        {
            return;
        }
        let labels = items[source + 1..target]
            .iter()
            .map(|item| match item {
                Item::Label(label) => *label,
                Item::Instruction(_) => unreachable!(),
            })
            .collect::<Vec<_>>();
        if labels.iter().any(|label| {
            self.preserved_block_boundaries.contains(label)
                || self.original_nop_targets.contains(label)
                || self.exception_owner_provenance.omitted_nop_locations.contains_key(label)
                || self.exception_owner_provenance.omitted_nop_counts.contains_key(label)
                || self.exception_owner_provenance.fusion_boundaries.contains(label)
                || self.exception_owner_provenance.block_boundaries.contains(label)
                || self.exception_owner_provenance.source_boundaries.contains(label)
                || self.exception_owner_provenance.setup_nop_boundaries.contains(label)
                || self.exception_owner_provenance.setups.iter().any(|(target, _)| target == label)
                || self.exception_regions.iter().any(|region| region.target == *label)
        }) || self.exception_owner_provenance.omitted_nop_tail.is_some_and(|(position, _, _)| {
            source < position && position <= target
        }) || items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if labels.contains(&label)))
        }) {
            return;
        }
        let items = self.linear_items_mut();
        let Item::Instruction(previous) = &mut items[source] else {
            unreachable!();
        };
        previous.jump_policy.retire_original_uniform_fold();
        previous.local_policy.remove_hidden_exception_owner_slot();
        let Item::Instruction(next) = &mut items[target] else {
            unreachable!();
        };
        next.jump_policy.retire_original_uniform_fold();
        next.local_policy.add_hidden_exception_owner_slots(1);
        self.exception_owner_provenance
            .removed_pop_block_boundaries
            .insert(boundary);
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

    /// Certifies the same-line NOP authored immediately after a specialized super load.
    pub(crate) fn record_last_same_line_super_owner_slot(&mut self) {
        let instruction = self.required_last_instruction_mut("recording a super-load owner slot");
        if instruction.opcode == crate::target::opcodes::LOAD_SUPER_ATTR
            && matches!(instruction.operand(), Operand::Value(_))
            && instruction.location.line >= 0
            && instruction.location.line == instruction.location.end_line
            && instruction.exception_policy() == ExceptionPolicy::NoneUnchanged
            && instruction.local_policy.hidden_exception_owner_slots() == 1
            && instruction.pre_fold_extra_instructions() == 0
        {
            instruction.jump_policy.mark_original_uniform_fold();
            let original = *instruction;
            self.exception_owner_provenance
                .original_uniform_folds
                .push(original);
        }
    }

    /// Certifies contiguous literal-fold records that share their survivor's owner.
    pub(crate) fn record_last_uniform_fold_owner_slots(&mut self, count: usize) {
        if count >= 4 && count < usize::from(LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS) {
            let instruction = self.required_last_instruction_mut("recording original literal fold");
            let original_literal = match (instruction.opcode, instruction.operand()) {
                (
                    crate::target::opcodes::LIST_EXTEND | crate::target::opcodes::SET_UPDATE,
                    Operand::Value(1),
                ) => true,
                // Larger tuples use LIST_APPEND and have a different original slot count.
                (LOAD_CONST, Operand::Deferred(_)) => count <= 30,
                _ => false,
            };
            if original_literal
                && instruction.exception_policy() == ExceptionPolicy::NoneUnchanged
                && instruction.pre_fold_extra_instructions() == 4
                && usize::from(instruction.local_policy.hidden_exception_owner_slots()) == count
            {
                instruction.jump_policy.mark_original_uniform_fold();
                let original = *instruction;
                self.exception_owner_provenance
                    .original_uniform_folds
                    .push(original);
            }
        }
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
    pub(crate) fn defer_last_boolean_exception_owner(&mut self, entry: Label) {
        self.required_last_instruction_mut("deferring Boolean exception ownership")
            .set_exception_policy(ExceptionPolicy::NoneDynamic);
        if let Some(start) = self
            .linear_items()
            .iter()
            .rposition(|item| *item == Item::Label(entry))
        {
            let prefix = self.linear_items()[start..].to_vec();
            self.exception_owner_provenance
                .original_value_jump_prefixes
                .push((entry, prefix));
        }
    }

    /// Marks the fallthrough created by backward conditional normalization as a fresh block.
    pub(crate) fn mark_last_condition_fallthrough_fresh(&mut self) {
        let instruction =
            self.required_last_instruction_mut("marking a fresh condition fallthrough");
        if instruction.opcode == NOT_TAKEN {
            instruction.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
        }
    }

    /// Retains the post-try origin while a marker awaits an exact owned-slot proof.
    pub(crate) fn defer_last_post_try_condition_owner(&mut self) {
        let instruction =
            self.required_last_instruction_mut("deferring a post-try condition owner");
        if instruction.opcode == NOT_TAKEN {
            instruction.set_exception_policy(ExceptionPolicy::AlwaysAbsentPostTryCondition);
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
        // Native lowers STORE_FAST_MAYBE_NULL after local-pair fusion. Keep both
        // boundaries attached to the restore when static swaps move it.
        instruction.fusion_policy.prevent_with_previous();
        instruction.fusion_policy.prevent_with_next();
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
        self.record_original_empty_join(emitted);
    }

    /// Retains the empty final-pattern block before copying an unprotected literal return.
    /// The caller must exclude loops and cleanup suites and require directly returning cases.
    pub(crate) fn preserve_final_match_failure_return_join(&mut self, emitted: EmittedJump) {
        let jump = *self.emitted_jump_mut(emitted, "retaining a final match failure join");
        if jump.opcode != JUMP_FORWARD
            || !matches!(jump.operand(), Operand::Forward(_))
            || jump.location.line < 0
            || jump.pre_fold_extra_instructions() != 0
            || jump.exception_policy() != ExceptionPolicy::default()
        {
            return;
        }
        let index = usize::try_from(emitted.instruction().item_index)
            .expect("emitted instruction index should fit in usize");
        let items = &self.linear_items()[..index];
        let labels = items
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if labels.is_empty()
            || labels.iter().any(|label| {
                self.borrow_block_aliases.contains(label)
                    || self.original_nop_targets.contains(label)
                    || self.folded_away_condition_targets.contains(label)
            })
            || !matches!(items[..items.len() - labels.len()].last(),
                Some(Item::Instruction(previous)) if previous.opcode == RETURN_VALUE)
        {
            return;
        }
        let mut incoming = items.iter().filter_map(|item| match item {
            Item::Instruction(instruction)
                if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if labels.contains(&label)) => Some(instruction),
            _ => None,
        });
        let (Some(condition), None) = (incoming.next(), incoming.next()) else {
            return;
        };
        if condition.opcode != POP_JUMP_IF_FALSE
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition.location != jump.location
            || condition.pre_fold_extra_instructions() != 0
        {
            return;
        }
        // CPython emits fail_pop[0], then the match end label, immediately before
        // the outer branch jump. This actual conditional target starts its own
        // block; the preceding return excludes fallthrough into the empty join.
        let jump = self.emitted_jump_mut(emitted, "retaining a final match failure join");
        jump.jump_policy.mark_original_empty_join();
        jump.jump_policy.retain_redundant_nop_after_threading();
        jump.jump_policy.prevent_threading_target();
    }

    pub(crate) fn prevent_jump_inlining(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "preventing jump inlining")
            .jump_policy
            .prevent_small_exit_inlining();
    }

    /// Captures the exact final class-pattern failure cleanup before a normal try exit.
    pub(crate) fn record_normal_try_class_pattern_fallthrough(
        &mut self,
        entry: Label,
        target: Label,
        location: SourceLocation,
    ) {
        let items = self.linear_items();
        let mut instructions =
            items
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(index, item)| match item {
                    Item::Instruction(instruction) => Some((index, instruction)),
                    Item::Label(_) => None,
                });
        let (Some((_, jump)), Some((last_index, last_pop)), Some((first_index, first_pop))) = (
            instructions.next(),
            instructions.next(),
            instructions.next(),
        ) else {
            return;
        };
        let Some(start) = first_index.checked_sub(1) else {
            return;
        };
        let [
            Item::Label(first_label),
            Item::Instruction(_),
            Item::Label(last_label),
            Item::Instruction(_),
        ] = &items[start..=last_index]
        else {
            return;
        };
        // The second cleanup array has only the first pop as an original
        // predecessor. A guard or a nested cleanup must not acquire this route.
        if items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if label == *last_label))
        }) {
            return;
        }
        if jump.opcode == JUMP_FORWARD
            && jump.operand() == Operand::Forward(target)
            && last_pop.opcode == POP_TOP
            && first_pop.opcode == POP_TOP
            && last_pop.location == location
            && first_pop.location == location
            && last_pop.operand() == Operand::Value(0)
            && first_pop.operand() == Operand::Value(0)
            && first_pop.depth_after().is_some_and(|depth| {
                depth.checked_sub(1).is_some_and(|next_depth| {
                    last_pop.depth_after() == Some(next_depth)
                        && jump.depth_after() == last_pop.depth_after()
                })
            })
        {
            let origin = (*first_pop, *last_pop);
            let labels = (*first_label, *last_label);
            self.exception_owner_provenance
                .normal_try_class_pattern_donor_labels
                .insert(entry, labels);
            self.exception_owner_provenance
                .normal_try_fallthrough_donors
                .insert(entry, origin);
        }
    }

    /// Records the original two-slot normal exit of an ordinary try without an else suite.
    pub(crate) fn record_normal_try_exit(
        &mut self,
        entry: Label,
        target: Label,
        wildcard_call_location: Option<SourceLocation>,
    ) {
        self.exception_owner_provenance
            .normal_try_exits
            .push((entry, target));
        let Some(location) = wildcard_call_location else {
            return;
        };
        let mut instructions = self
            .linear_items()
            .iter()
            .rev()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            });
        let (Some(jump), Some(pop), Some(call)) = (
            instructions.next(),
            instructions.next(),
            instructions.next(),
        ) else {
            return;
        };
        if jump.opcode == JUMP_FORWARD
            && jump.operand() == Operand::Forward(target)
            && pop.opcode == POP_TOP
            && matches!(
                call.opcode,
                crate::target::opcodes::CALL | crate::target::opcodes::CALL_KW
            )
            && pop.location == location
            && call.location == location
        {
            let origin = (*call, *pop);
            self.exception_owner_provenance
                .normal_try_fallthrough_donors
                .insert(entry, origin);
        }
    }

    /// Records an inner try/except `POP_BLOCK` exit for owner transfer and redundant-jump removal.
    pub(crate) fn record_compound_try_owner_exit(
        &mut self,
        entry: Label,
        target: Label,
        wildcard_call_location: Option<SourceLocation>,
    ) {
        self.exception_owner_provenance
            .compound_try_owner_exits
            .push((entry, target));
        let Some(location) = wildcard_call_location else {
            return;
        };
        let mut instructions = self
            .linear_items()
            .iter()
            .rev()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            });
        let (Some(jump), Some(pop), Some(call)) = (
            instructions.next(),
            instructions.next(),
            instructions.next(),
        ) else {
            return;
        };
        if jump.opcode == JUMP_FORWARD
            && jump.operand() == Operand::Forward(target)
            && jump.location == location
            && pop.opcode == POP_TOP
            && matches!(
                call.opcode,
                crate::target::opcodes::CALL | crate::target::opcodes::CALL_KW
            )
            && pop.location == location
            && call.location == location
            && jump.depth_after() == pop.depth_after()
        {
            self.exception_owner_provenance
                .compound_try_fallthrough_donors
                .insert(entry, (call, pop));
        }
    }

    /// Records the live original else array without crossing a preserved empty boundary.
    pub(crate) fn record_populated_try_else_borrow_exit(&mut self, emitted: EmittedJump) {
        let jump = *self.emitted_jump_mut(emitted, "recording a populated try-else exit");
        let Operand::Forward(target) = jump.operand() else {
            return;
        };
        if jump.opcode != JUMP_FORWARD
            || jump.jump_policy.borrow_empty_block()
            || !jump.jump_policy.allows_borrow_threading_target()
            || jump.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Reachable
        {
            return;
        }
        let index = usize::try_from(emitted.instruction().item_index)
            .expect("emitted instruction index should fit in usize");
        let mut populated = false;
        let mut source = None;
        for item in self.linear_items()[..index].iter().rev() {
            match item {
                Item::Label(label) => {
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label)
                    {
                        if populated
                            && self
                                .exception_owner_provenance
                                .source_boundaries
                                .contains(label)
                        {
                            source = Some(*label);
                        }
                        break;
                    }
                    if self.preserved_block_boundaries.contains(label) {
                        return;
                    }
                }
                Item::Instruction(instruction) => {
                    if matches!(instruction.opcode, NOP | NOT_TAKEN)
                        || !matches!(instruction.operand(), Operand::Value(_))
                        || ends_scope(instruction.opcode)
                        || instruction.jump_policy.borrow_empty_block()
                        || instruction.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::Reachable
                    {
                        return;
                    }
                    populated = true;
                }
            }
        }
        if let Some(source) = source {
            self.exception_owner_provenance
                .populated_try_else_borrow_exits
                .push((source, target));
        }
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
        let original = *jump;
        jump.no_location_policy.allow_split_return_exit_copy();
        let updated = *jump;
        // Truth conditions keep the same unique source witness when this authored
        // copy policy changes, just like the source None conditions below.
        let mut primitive_origins = self
            .exception_owner_provenance
            .original_primitive_condition_boundaries
            .iter()
            .enumerate()
            .filter(|(_, origin)| **origin == original);
        if let Some((origin_index, _)) = primitive_origins.next()
            && primitive_origins.next().is_none()
            && self
                .linear_items()
                .iter()
                .filter(|item| {
                    **item == Item::Instruction(original) || **item == Item::Instruction(updated)
                })
                .take(2)
                .count()
                == 1
        {
            self.exception_owner_provenance
                .original_primitive_condition_boundaries[origin_index] = updated;
        }
        // Keep the exact source witness in sync with this authored policy change.
        // An ambiguous origin or instruction copy cannot acquire that witness.
        let mut origins = self
            .exception_owner_provenance
            .source_none_conditions
            .iter()
            .enumerate()
            .filter(|(_, origin)| origin.instruction == original);
        let Some((origin_index, _)) = origins.next() else {
            return;
        };
        if origins.next().is_some()
            || self
                .linear_items()
                .iter()
                .filter(|item| {
                    **item == Item::Instruction(original) || **item == Item::Instruction(updated)
                })
                .take(2)
                .count()
                != 1
        {
            return;
        }
        self.exception_owner_provenance.source_none_conditions[origin_index].instruction = updated;
    }

    pub(crate) fn preserve_last_inlined_jump_nop(&mut self) {
        self.last_jump_instruction_mut("retaining an inlined jump NOP")
            .jump_policy
            .retain_nop();
    }

    /// Certifies an original case-success edge without introducing a block boundary.
    pub(crate) fn record_original_match_success(
        &mut self,
        emitted: EmittedJump,
        original_empty_join: bool,
    ) {
        let instruction = self.emitted_jump_mut(emitted, "recording an original match success");
        debug_assert_eq!(instruction.opcode, JUMP_FORWARD);
        instruction.jump_policy.mark_original_match_success();
        if original_empty_join {
            // Validate the compiler's join certificate against the original boundary
            // and its emitted predecessors before allowing retained return-copy slots.
            self.record_original_empty_join(emitted);
        }
    }

    pub(crate) fn preserve_inlined_jump_nop(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "retaining an inlined jump NOP")
            .jump_policy
            .retain_nop();
    }

    /// Retains an authored Pass line without preventing threading through its join.
    pub(crate) fn preserve_authored_pass_jump_nop(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "retaining an authored Pass jump NOP")
            .jump_policy
            .retain_nop_after_retargeting();
    }

    pub(crate) fn preserve_direct_inlined_jump_nop(&mut self, emitted: EmittedJump) {
        let instruction = self.emitted_jump_mut(emitted, "retaining a directly inlined jump NOP");
        let Operand::Forward(_) = instruction.operand() else {
            panic!("retaining a directly inlined jump NOP requires a forward jump");
        };
        instruction.jump_policy.retain_nop_for_direct_target();
    }

    /// Records an original empty join and whether a later copied return retains its jump slot.
    pub(crate) fn preserve_empty_join_inlined_jump_nop(&mut self, emitted: EmittedJump) {
        self.preserve_direct_inlined_jump_nop(emitted);
        self.record_original_empty_join(emitted);
    }

    /// Retains a folded branch's exit when it starts in a nested if's original empty join.
    pub(crate) fn preserve_folded_nested_if_exit(&mut self, emitted: EmittedJump) {
        let original_policy = self
            .emitted_jump_mut(emitted, "inspecting a folded nested-if exit")
            .jump_policy;
        self.record_original_empty_join(emitted);
        let instruction = self.emitted_jump_mut(emitted, "retaining a folded nested-if exit");
        let original_empty_join = instruction.jump_policy.is_original_empty_join();
        // This certificate must not enroll the folded marker in an ordinary join-copy plan.
        instruction.jump_policy = original_policy;
        if original_empty_join {
            // Let incoming branches thread past this empty array before retaining its NOP.
            // Borrow-marker cleanup still requires one current predecessor for its line.
            instruction
                .jump_policy
                .retain_redundant_nop_after_threading();
        }
    }

    /// Records original empty-join identity independently of its jump-retention policy.
    fn record_original_empty_join(&mut self, emitted: EmittedJump) {
        let index = usize::try_from(emitted.instruction().item_index)
            .expect("emitted instruction index should fit in usize");
        let items = &self.linear_items()[..index];
        let labels = items
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let conditional_predecessors = items
            .iter()
            .filter(|item| {
                matches!(item, Item::Instruction(instruction)
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                            if labels.contains(&label)))
            })
            .take(2)
            .count();
        let has_fallthrough = items
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .is_some_and(|instruction| {
                !ends_scope(instruction.opcode) && !is_unconditional_jump(instruction.opcode)
            });
        // Unconditional incoming edges can copy the exit before line propagation.
        // A conditional edge remains: it can donate a line only when it is the
        // sole predecessor, as after a guard whose true arm terminates.
        let original_match_success = matches!(
            &self.linear_items()[index],
            Item::Instruction(instruction) if instruction.jump_policy.is_original_match_success()
        );
        let original_block = labels.iter().any(|label| {
            // Live case-success edges already establish a match end without adding
            // a source-boundary entry. A lone terminating guard's false edge has
            // its original array recorded by conditional-threading provenance.
            (self
                .exception_owner_provenance
                .source_boundaries
                .contains(label)
                || (original_match_success
                    && self
                        .exception_owner_provenance
                        .original_match_join_boundaries
                        .contains(label))
                || (conditional_predecessors == 1
                    && !has_fallthrough
                    && self.conditional_threading_targets.contains_key(label)))
                && !self.borrow_block_aliases.contains(label)
        });
        let original_empty_join = original_block
            && (conditional_predecessors == 0
                || (conditional_predecessors == 1 && !has_fallthrough));
        if original_empty_join {
            self.emitted_jump_mut(emitted, "retaining an original empty join")
                .jump_policy
                .mark_original_empty_join();
        } else if original_block && conditional_predecessors > 1 {
            // Multiple conditional edges keep this original join unlocated. Its
            // inherited emission line must not become an authored jump slot in
            // the normal-try copy plan, and does not authorize KI NOP retention.
            self.emitted_jump_mut(emitted, "recording an original shared empty join")
                .jump_policy
                .mark_original_shared_empty_join();
        }
    }

    /// Records a CPython ordinary jump block before small exits can replace its jump.
    pub(crate) fn mark_conditional_threading_target(
        &mut self,
        label: Label,
        location: SourceLocation,
    ) {
        self.conditional_threading_targets.insert(label, location);
    }

    /// Records the common backedge emitted after a loop body, before optimization.
    /// Per-arm backedges emitted by loop-tail lowering do not receive this certificate.
    pub(crate) fn record_last_original_loop_backedge(&mut self) {
        let (last, head) = self
            .linear_items()
            .split_last()
            .expect("original loop backedge");
        let Item::Instruction(instruction) = last else {
            unreachable!("original loop backedge must be the last instruction");
        };
        assert_eq!(instruction.opcode, JUMP_BACKWARD);
        let location = instruction.location;
        let labels = head
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        self.conditional_threading_targets
            .extend(labels.into_iter().map(|label| (label, location)));
    }

    /// Certifies a conditional join that starts at the jump after copied finalizer cleanup.
    pub(crate) fn record_copied_finally_break_head(
        &mut self,
        emitted: EmittedJump,
        target: Label,
        unwind_start: usize,
    ) {
        let jump = *self.emitted_jump_mut(emitted, "recording a copied-finally break head");
        if jump.opcode != JUMP_FORWARD
            || jump.operand() != Operand::Forward(target)
            || jump.location != SourceLocation::NONE
            || jump.pre_fold_extra_instructions() != 0
        {
            return;
        }
        let Some((Item::Instruction(_), head)) = self.linear_items().split_last() else {
            return;
        };
        let Some(unwind) = head.get(unwind_start..) else {
            return;
        };
        let labels = unwind
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let mut matching = unwind.iter().filter_map(|item| match item {
            Item::Instruction(instruction)
                if matches!(
                    instruction.opcode,
                    POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
                ) =>
            {
                match instruction.operand() {
                    Operand::Forward(label) if labels.contains(&label) => Some(label),
                    _ => None,
                }
            }
            _ => None,
        });
        let Some(label) = matching.next() else {
            return;
        };
        if matching.next().is_some()
            || !self
                .exception_owner_provenance
                .source_boundaries
                .contains(&label)
            || !self
                .exception_owner_provenance
                .block_boundaries
                .contains(&label)
            || self.borrow_block_aliases.contains(&label)
            || self.original_nop_targets.contains(&label)
        {
            return;
        }
        // The authored break NOP precedes this unwind. The selected label enters
        // the final jump itself, so its original array can participate in threading.
        self.conditional_threading_targets
            .insert(label, SourceLocation::NONE);
    }

    /// Certifies the actual false-edge head emitted after a copied finalizer unwinds.
    pub(crate) fn record_copied_finally_continue_head(
        &mut self,
        emitted: EmittedJump,
        target: Label,
        condition: SourceLocation,
        unwind_start: usize,
    ) {
        let jump = *self.emitted_jump_mut(emitted, "recording a copied-finally continue head");
        if jump.opcode != JUMP_BACKWARD
            || jump.operand() != Operand::Backward(target)
            || jump.location != SourceLocation::NONE
            || jump.pre_fold_extra_instructions() != 0
        {
            return;
        }
        let Some((Item::Instruction(last), head)) = self.linear_items().split_last() else {
            return;
        };
        if *last != jump {
            return;
        }
        let labels = head
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let Some(unwind) = head.get(unwind_start..) else {
            return;
        };
        let mut matching = unwind.iter().filter_map(|item| match item {
            Item::Instruction(instruction)
                if matches!(
                    instruction.opcode,
                    POP_JUMP_IF_FALSE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
                ) && instruction.location == condition =>
            {
                match instruction.operand() {
                    Operand::Forward(label) if labels.contains(&label) => Some(label),
                    _ => None,
                }
            }
            _ => None,
        });
        let Some(label) = matching.next() else {
            return;
        };
        if matching.next().is_some()
            || head
                .iter()
                .filter(|item| **item == Item::Label(label))
                .count()
                != 1
        {
            return;
        }
        self.conditional_threading_targets
            .insert(label, SourceLocation::NONE);
    }

    pub(crate) fn prevent_jump_threading_target(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "preventing jump threading through a target")
            .jump_policy
            .prevent_threading_target();
    }

    /// Keeps an original cleanup from certifying a borrowed-load route through its jump.
    /// Physical jump threading still transfers the break's owner and interrupt check.
    pub(crate) fn prevent_borrow_jump_threading_target(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(
            emitted,
            "preventing borrowed-load threading through a target",
        )
        .jump_policy
        .prevent_borrow_threading_target();
    }

    pub(crate) fn mark_jump_as_authored_break(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "marking an authored break jump")
            .fusion_policy
            .mark_authored_break();
    }

    /// Marks a backedge synthesized by late conditional normalization.
    pub(crate) fn mark_normalized_conditional_backedge(&mut self, emitted: EmittedJump) {
        self.emitted_jump_mut(emitted, "marking a normalized conditional backedge")
            .jump_policy
            .mark_normalized_conditional_backedge();
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

    /// Records a nested join until its original match-case successor is emitted.
    pub(crate) fn record_nested_if_unlocated_join(&mut self, emitted: EmittedJump) {
        let index = usize::try_from(emitted.instruction().item_index)
            .expect("emitted instruction index should fit in usize");
        if let Some(Item::Label(label)) = self.linear_items().get(index.wrapping_sub(1)) {
            self.nested_if_unlocated_join_blocks.insert(*label);
        }
    }

    /// Lets private copies inherit locations when the original join reaches a case exit.
    /// The caller captures the unlocated head before emitting this source-authored exit.
    pub(crate) fn inherit_nested_if_match_exit_locations(
        &mut self,
        emitted: EmittedJump,
        original_unlocated_head: bool,
    ) {
        self.inherit_nested_if_original_exit_locations(
            emitted,
            original_unlocated_head,
            JUMP_FORWARD,
        );
    }

    /// Uses only the original common backedge emitted after a while body.
    /// The caller captures the head before emission can remove an authored NOP.
    pub(crate) fn inherit_nested_if_while_backedge_locations(
        &mut self,
        emitted: EmittedJump,
        original_unlocated_head: bool,
    ) {
        self.inherit_nested_if_original_exit_locations(
            emitted,
            original_unlocated_head,
            JUMP_BACKWARD,
        );
    }

    fn inherit_nested_if_original_exit_locations(
        &mut self,
        emitted: EmittedJump,
        original_unlocated_head: bool,
        original_exit_opcode: Opcode,
    ) {
        if self.nested_if_unlocated_join_blocks.is_empty() {
            return;
        }
        let index = usize::try_from(emitted.instruction().item_index)
            .expect("emitted instruction index should fit in usize");
        let items = self.linear_items();
        let Some(Item::Instruction(exit)) = items.get(index) else {
            return;
        };
        if exit.opcode != original_exit_opcode
            || (exit.opcode == JUMP_FORWARD && !exit.jump_policy.is_interruptible())
            || !original_unlocated_head
            || exit.pre_fold_extra_instructions() != 0
            || exit.local_policy.hidden_exception_owner_slots() != 0
            || exit.jump_policy.always_retains_nop()
        {
            return;
        }
        let exit_labels = items[..index]
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let clean_labels = |labels: &[Label]| {
            !labels.is_empty()
                && labels.iter().all(|label| {
                    !self.original_nop_targets.contains(label)
                        && !self
                            .exception_owner_provenance
                            .omitted_nop_counts
                            .contains_key(label)
                        && !self
                            .exception_owner_provenance
                            .omitted_nop_locations
                            .contains_key(label)
                })
        };
        if !clean_labels(&exit_labels) {
            return;
        }
        let mut labels = Vec::new();
        let mut promoted = Vec::new();
        for item in &items[..index] {
            match item {
                Item::Label(label) => labels.push(*label),
                Item::Instruction(jump) => {
                    if jump.opcode == JUMP_FORWARD
                        && jump.location == SourceLocation::NONE
                        && jump.no_location_policy.preserves_location()
                        && !jump.jump_policy.is_interruptible()
                        && !jump.no_location_policy.is_artificial_eval_break()
                        && !jump.jump_policy.always_retains_nop()
                        && jump.pre_fold_extra_instructions() == 0
                        && jump.local_policy.hidden_exception_owner_slots() == 0
                        && clean_labels(&labels)
                        && let Operand::Forward(target) = jump.operand()
                        && exit_labels.contains(&target)
                    {
                        promoted.extend(
                            labels.iter().copied().filter(|label| {
                                self.nested_if_unlocated_join_blocks.contains(label)
                            }),
                        );
                    }
                    labels.clear();
                }
            }
        }
        // CPython first inlines this original NO_LOCATION case-success or common
        // while-backedge JUMP. The nested join then owns that eval-check exit before
        // locations are resolved.
        // Its copies and sole surviving original follow the existing copy policy.
        self.shared_jump_copy_location_blocks.extend(promoted);
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

    /// Reuses the emitted normal-exhaustion record for its original outer `POP_BLOCK`.
    pub(crate) fn adopt_async_for_end_pop_block(&mut self, emitted: EmittedInstruction) {
        let instruction = self.emitted_instruction_mut(emitted, "adopting an async-for POP_BLOCK");
        assert_eq!(instruction.opcode, NOP);
        assert_eq!(instruction.operand(), Operand::Value(0));
        assert_eq!(instruction.local_policy.hidden_exception_owner_slots(), 0);
        assert_eq!(instruction.pre_fold_extra_instructions(), 0);
        instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        instruction.fusion_policy.mark_converted_pop_block();
        // END_ASYNC_FOR ends the preceding original block. Its identical line does
        // not make this first record redundant within the new normal-exit block.
        instruction.jump_policy.preserve_explicit_nop();
    }

    /// Records the direct local None test following an authored normal-finally `POP_BLOCK`.
    pub(crate) fn record_normal_finally_none_prefix(&mut self, ordinal: usize) {
        let mut instructions = self
            .linear_items()
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .skip(ordinal);
        let (Some(pop), Some(load), Some(condition)) = (
            instructions.next(),
            instructions.next(),
            instructions.next(),
        ) else {
            return;
        };
        if pop.opcode == NOP
            && pop.fusion_policy.is_converted_pop_block()
            && pop.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
            && matches!(load.opcode, LOAD_FAST | crate::target::opcodes::LOAD_DEREF)
            && load.local_policy.hidden_exception_owner_slots() == 0
            && load.pre_fold_extra_instructions() == 0
            && matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            && matches!(condition.operand(), Operand::Forward(_))
            && condition.local_policy.hidden_exception_owner_slots() == 3
            && condition.pre_fold_extra_instructions() == 0
        {
            self.exception_owner_provenance
                .normal_finally_none_prefixes
                .push((pop, load, condition));
        }
    }

    /// Captures an unnamed-handler return through a finally's first local None test.
    pub(crate) fn record_copied_handler_return_none_prefix(&mut self, final_pop_item: usize) {
        let items = self.linear_items();
        let preceding = items[..final_pop_item]
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, item)| match item {
                Item::Instruction(instruction) => Some((index, *instruction)),
                Item::Label(_) => None,
            })
            .take(3)
            .collect::<Vec<_>>();
        let [(.., pop_except), (.., handler_pop), (swap_item, swap)] = preceding.as_slice() else {
            return;
        };
        let following = items[final_pop_item..]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .take(3)
            .collect::<Vec<_>>();
        let [final_pop, load, condition] = following.as_slice() else {
            return;
        };
        let records = [
            *swap,
            *handler_pop,
            *pop_except,
            *final_pop,
            *load,
            *condition,
        ];
        if swap.opcode != SWAP
            || swap.operand() != Operand::Value(2)
            || pop_except.opcode != POP_EXCEPT
            || pop_except.operand() != Operand::Value(0)
            || [handler_pop, final_pop].iter().any(|pop| {
                pop.opcode != NOP
                    || pop.operand() != Operand::Value(0)
                    || !pop.fusion_policy.is_converted_pop_block()
                    || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            })
            || !matches!(load.opcode, LOAD_FAST | crate::target::opcodes::LOAD_DEREF)
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition.local_policy.hidden_exception_owner_slots() != 3
            || records[..5]
                .iter()
                .any(|record| record.local_policy.hidden_exception_owner_slots() != 0)
            || records
                .iter()
                .any(|record| record.pre_fold_extra_instructions() != 0)
        {
            return;
        }
        let Some(entry) = items[..*swap_item]
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Label(label)
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label) =>
                {
                    Some(*label)
                }
                _ => None,
            })
        else {
            return;
        };
        self.exception_owner_provenance
            .copied_handler_return_none_prefixes
            .push((entry, records));
    }

    /// Records adjacent emitted return-unwind POPs and their direct local None test.
    pub(crate) fn record_copied_return_none_prefix(
        &mut self,
        return_item: usize,
        first_pop_item: usize,
        allow_additional_pop: bool,
    ) {
        let (pops, additional_pop, load, condition) =
            match self.linear_items().get(first_pop_item..) {
                Some(
                    [
                        Item::Instruction(first_pop),
                        Item::Instruction(middle_pop),
                        Item::Instruction(final_pop),
                        Item::Instruction(load),
                        Item::Instruction(condition),
                        ..,
                    ],
                ) if allow_additional_pop && final_pop.opcode == NOP => (
                    [*first_pop, *final_pop],
                    Some(*middle_pop),
                    *load,
                    *condition,
                ),
                Some(
                    [
                        Item::Instruction(first_pop),
                        Item::Instruction(final_pop),
                        Item::Instruction(load),
                        Item::Instruction(condition),
                        ..,
                    ],
                ) => ([*first_pop, *final_pop], None, *load, *condition),
                _ => return,
            };
        if pops.iter().chain(additional_pop.iter()).any(|pop| {
            pop.opcode != NOP
                || pop.operand() != Operand::Value(0)
                || !pop.fusion_policy.is_converted_pop_block()
                || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
                || pop.local_policy.hidden_exception_owner_slots() != 0
                || pop.pre_fold_extra_instructions() != 0
        }) || !matches!(load.opcode, LOAD_FAST | crate::target::opcodes::LOAD_DEREF)
            || load.local_policy.hidden_exception_owner_slots() != 0
            || load.pre_fold_extra_instructions() != 0
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition.local_policy.hidden_exception_owner_slots() != 3
            || condition.pre_fold_extra_instructions() != 0
        {
            return;
        }
        let return_nop = match self.linear_items().get(return_item..first_pop_item) {
            Some([Item::Instruction(authored), labels @ ..])
                if authored.opcode == NOP
                    && authored.exception_policy() == ExceptionPolicy::NoneUnchanged
                    && !authored.fusion_policy.is_converted_pop_block()
                    && authored.local_policy.hidden_exception_owner_slots() == 0
                    && authored.pre_fold_extra_instructions() == 0
                    && authored.location == pops[0].location
                    && authored.depth_after() == pops[0].depth_after()
                    && labels.iter().all(|item| matches!(item, Item::Label(_))) =>
            {
                Some(*authored)
            }
            _ => None,
        };
        if additional_pop.is_some() {
            let Some(authored) = return_nop else {
                return;
            };
            // Three removed cells require the complete authored-NOP/POP/POP/POP
            // chain. They are not an arbitrary transferred hidden-slot count.
            if authored.location.line < 0
                || pops.iter().chain(additional_pop.iter()).any(|pop| {
                    pop.location != authored.location
                        || pop.depth_after() != authored.depth_after()
                        || pop.jump_policy.borrow_empty_block()
                        || pop.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::Reachable
                })
            {
                return;
            }
        }
        self.exception_owner_provenance
            .copied_return_none_prefixes
            .push(exception_owners::CopiedReturnNonePrefix {
                return_nop,
                pops,
                additional_pop,
                load,
                condition,
            });
    }

    /// Captures an outer-manager return's two adjacent NULL POPs and first cleanup constant.
    pub(crate) fn record_copied_return_pop_prefix(&mut self, first_pop_item: usize) {
        let items = self.linear_items();
        let Some(
            [
                Item::Instruction(first),
                Item::Instruction(second),
                Item::Instruction(load),
                ..,
            ],
        ) = items.get(first_pop_item..)
        else {
            return;
        };
        let pops = [*first, *second];
        if pops.iter().any(|pop| {
            pop.opcode != NOP
                || pop.operand() != Operand::Value(0)
                || !pop.fusion_policy.is_converted_pop_block()
                || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
                || pop.local_policy.hidden_exception_owner_slots() != 0
                || pop.pre_fold_extra_instructions() != 0
        }) || load.opcode != LOAD_CONST
            || !matches!(load.operand(), Operand::Value(_))
            || load.exception_policy() != ExceptionPolicy::NoneUnchanged
            || load.local_policy.hidden_exception_owner_slots() != 0
            || load.pre_fold_extra_instructions() != 0
            || load.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Reachable
            || load.jump_policy.borrow_empty_block()
        {
            return;
        }
        let Some(entry) = items[..first_pop_item]
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Label(label)
                    if self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label) =>
                {
                    Some(*label)
                }
                _ => None,
            })
        else {
            return;
        };
        let records = [pops[0], pops[1], *load];
        self.exception_owner_provenance
            .copied_return_pop_prefixes
            .push(exception_owners::CopiedReturnPopPrefix { entry, records });
    }

    /// Captures the original return POP pair and its direct local truth test.
    pub(crate) fn record_copied_return_truth_prefix(
        &mut self,
        return_item: usize,
        first_pop_item: usize,
    ) {
        let Some(
            [
                Item::Instruction(first_pop),
                Item::Instruction(final_pop),
                Item::Instruction(load),
                Item::Instruction(to_bool),
                Item::Instruction(condition),
                ..,
            ],
        ) = self.linear_items().get(first_pop_item..)
        else {
            return;
        };
        let pops = [*first_pop, *final_pop];
        let load = *load;
        let to_bool = *to_bool;
        let condition = *condition;
        if pops.iter().any(|pop| {
            pop.opcode != NOP
                || pop.operand() != Operand::Value(0)
                || !pop.fusion_policy.is_converted_pop_block()
                || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
        }) || load.opcode != LOAD_FAST
            || !matches!(load.operand(), Operand::Value(_))
            || to_bool.opcode != TO_BOOL
            || to_bool.operand() != Operand::Value(0)
            || !matches!(condition.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || [pops[0], pops[1], load, to_bool, condition]
                .iter()
                .any(|instruction| {
                    instruction.local_policy.hidden_exception_owner_slots() != 0
                        || instruction.pre_fold_extra_instructions() != 0
                })
        {
            return;
        }
        let return_nop = match self.linear_items().get(return_item..first_pop_item) {
            Some([Item::Instruction(authored), labels @ ..])
                if authored.opcode == NOP
                    && authored.operand() == Operand::Value(0)
                    && authored.exception_policy() == ExceptionPolicy::NoneUnchanged
                    && !authored.fusion_policy.is_converted_pop_block()
                    && authored.local_policy.hidden_exception_owner_slots() == 0
                    && authored.pre_fold_extra_instructions() == 0
                    && authored.location == pops[0].location
                    && authored.depth_after() == pops[0].depth_after()
                    && labels.iter().all(|item| matches!(item, Item::Label(_))) =>
            {
                *authored
            }
            _ => return,
        };
        self.exception_owner_provenance
            .copied_return_truth_prefixes
            .push(exception_owners::CopiedReturnTruthPrefix {
                return_nop,
                pops,
                load,
                to_bool,
                condition,
            });
    }

    /// Captures an awaited manager return whose two omitted POPs precede its finally POP.
    pub(crate) fn record_copied_manager_return_none_prefix(
        &mut self,
        return_item: usize,
        pop_item: usize,
    ) {
        use crate::target::opcodes::{CALL, END_SEND, LOAD_CONST, SWAP};

        let items = self.linear_items();
        if return_item >= pop_item || pop_item >= items.len() {
            return;
        }
        let preceding = items[..return_item]
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, item)| match item {
                Item::Instruction(instruction) => Some((index, *instruction)),
                Item::Label(_) => None,
            })
            .take(2)
            .collect::<Vec<_>>();
        let [(_, discarded), (start, ended)] = preceding.as_slice() else {
            return;
        };
        if discarded.opcode != POP_TOP || ended.opcode != END_SEND {
            return;
        }
        let Some(Item::Label(entry)) = start.checked_sub(1).and_then(|index| items.get(index))
        else {
            return;
        };
        let entry = *entry;
        let prefix = items[*start..pop_item]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let Ok(prefix): Result<[Instruction; 10], _> = prefix.try_into() else {
            return;
        };
        if prefix.iter().map(|instruction| instruction.opcode).ne([
            END_SEND, POP_TOP, LOAD_FAST, SWAP, SWAP, LOAD_CONST, LOAD_CONST, LOAD_CONST, CALL,
            POP_TOP,
        ]) || prefix[3].operand() != Operand::Value(3)
            || prefix[4].operand() != Operand::Value(2)
            || prefix[5].operand() != prefix[6].operand()
            || prefix[6].operand() != prefix[7].operand()
            || prefix[8].operand() != Operand::Value(3)
            || items.get(return_item) != Some(&Item::Instruction(prefix[2]))
            || prefix.iter().any(|instruction| {
                instruction.local_policy.hidden_exception_owner_slots() != 0
                    || instruction.pre_fold_extra_instructions() != 0
            })
        {
            return;
        }
        let mut suffix = items[pop_item..].iter().filter_map(|item| match item {
            Item::Instruction(instruction) => Some(*instruction),
            Item::Label(_) => None,
        });
        let (Some(pop), Some(load), Some(condition)) =
            (suffix.next(), suffix.next(), suffix.next())
        else {
            return;
        };
        if pop.opcode == NOP
            && pop.fusion_policy.is_converted_pop_block()
            && pop.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
            && pop.local_policy.hidden_exception_owner_slots() == 0
            && pop.pre_fold_extra_instructions() == 0
            && load.opcode == LOAD_FAST
            && load.local_policy.hidden_exception_owner_slots() == 0
            && load.pre_fold_extra_instructions() == 0
            && matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            && matches!(condition.operand(), Operand::Forward(_))
            && condition.local_policy.hidden_exception_owner_slots() == 3
            && condition.pre_fold_extra_instructions() == 0
        {
            self.exception_owner_provenance
                .copied_manager_return_none_prefixes
                .push(exception_owners::CopiedManagerReturnNonePrefix {
                    entry,
                    prefix,
                    pop,
                    load,
                    condition,
                });
        }
    }

    /// Captures a constant return through an inner manager while its outer manager stays live.
    pub(crate) fn record_retained_manager_return_none_prefix(
        &mut self,
        return_item: usize,
        pop_item: usize,
        outer_handler: Label,
    ) {
        use crate::target::opcodes::{CALL, LOAD_CONST};

        let items = self.linear_items();
        let Some(before_pop) = items.get(return_item..pop_item) else {
            return;
        };
        let records = before_pop
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let [authored, first, second, third, call, discard] = records.as_slice() else {
            return;
        };
        if items.get(return_item) != Some(&Item::Instruction(*authored))
            || authored.opcode != NOP
            || authored.operand() != Operand::Value(0)
            || authored.location.line < 0
            || authored.fusion_policy.is_converted_pop_block()
            || [
                first.opcode,
                second.opcode,
                third.opcode,
                call.opcode,
                discard.opcode,
            ] != [LOAD_CONST, LOAD_CONST, LOAD_CONST, CALL, POP_TOP]
            || first.operand() != second.operand()
            || second.operand() != third.operand()
            || call.operand() != Operand::Value(3)
            || discard.operand() != Operand::Value(0)
            || [second, third, call, discard]
                .iter()
                .any(|record| record.location != first.location)
            || records.iter().any(|record| {
                record.exception_policy() != ExceptionPolicy::NoneUnchanged
                    || record.local_policy.hidden_exception_owner_slots() != 0
                    || record.pre_fold_extra_instructions() != 0
                    || !matches!(record.operand(), Operand::Value(_))
            })
        {
            return;
        }
        let mut suffix = items[pop_item..].iter().filter_map(|item| match item {
            Item::Instruction(instruction) => Some(*instruction),
            Item::Label(_) => None,
        });
        let (Some(pop), Some(load), Some(condition), Some(marker)) =
            (suffix.next(), suffix.next(), suffix.next(), suffix.next())
        else {
            return;
        };
        if pop.opcode != NOP
            || pop.operand() != Operand::Value(0)
            || !pop.fusion_policy.is_converted_pop_block()
            || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || pop.location.line >= 0
            || pop.local_policy.hidden_exception_owner_slots() != 0
            || pop.pre_fold_extra_instructions() != 0
            || pop.depth_after().is_none()
            || pop.depth_after() != discard.depth_after()
            || authored.depth_after() != pop.depth_after().and_then(|depth| depth.checked_add(2))
            || load.opcode != LOAD_FAST
            || !matches!(load.operand(), Operand::Value(_))
            || load.exception_policy() != ExceptionPolicy::NoneUnchanged
            || load.local_policy.hidden_exception_owner_slots() != 0
            || load.pre_fold_extra_instructions() != 0
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition.local_policy.hidden_exception_owner_slots() != 3
            || condition.pre_fold_extra_instructions() != 0
            || condition.exception_policy() != ExceptionPolicy::NoneUnchanged
            || marker.opcode != NOT_TAKEN
            || marker.operand() != Operand::Value(0)
            || marker.location != condition.location
            || marker.exception_policy() != ExceptionPolicy::NonePresent
            || marker.local_policy.hidden_exception_owner_slots() != 0
            || marker.pre_fold_extra_instructions() != 0
        {
            return;
        }
        // The preceding marker starts this exact original fallthrough array. Store
        // its complete prefix so an unrelated retained NOP cannot borrow this proof.
        let Some((entry_item, entry)) =
            items[..return_item]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, item)| match item {
                    Item::Instruction(instruction) if instruction.opcode == NOT_TAKEN => {
                        Some((index, *instruction))
                    }
                    _ => None,
                })
        else {
            return;
        };
        let prefix = items[entry_item + 1..pop_item]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let Some(return_offset) = prefix.len().checked_sub(records.len()) else {
            return;
        };
        if prefix.get(return_offset..) != Some(records.as_slice()) {
            return;
        }
        self.exception_owner_provenance
            .retained_manager_return_none_prefixes
            .push(exception_owners::RetainedManagerReturnNonePrefix {
                entry,
                prefix,
                return_offset,
                outer_handler,
                pop,
                load,
                condition,
                marker,
            });
    }

    /// Captures the actual manager unwind and first None test of its copied finally.
    pub(crate) fn record_copied_manager_return_compaction(
        &mut self,
        pop_item: usize,
        ordinary_try_pops: usize,
        manager_count: usize,
    ) {
        use crate::target::opcodes::{CALL, LOAD_CONST, SWAP};

        let items = self.linear_items();
        let Some(before) = items.get(..pop_item) else {
            return;
        };
        if !(1..=2).contains(&manager_count) {
            return;
        }
        let mut records = before
            .iter()
            .rev()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .take(7 * manager_count)
            .collect::<Vec<_>>();
        records.reverse();
        if records.len() != 7 * manager_count
            || records.as_chunks::<7>().0.iter().any(|cleanup| {
                cleanup.iter().map(|instruction| instruction.opcode).ne([
                    SWAP, SWAP, LOAD_CONST, LOAD_CONST, LOAD_CONST, CALL, POP_TOP,
                ]) || cleanup[0].operand() != Operand::Value(3)
                    || cleanup[1].operand() != Operand::Value(2)
                    || cleanup[2].operand() != cleanup[3].operand()
                    || cleanup[3].operand() != cleanup[4].operand()
                    || cleanup[5].operand() != Operand::Value(3)
                    || cleanup.iter().any(|instruction| {
                        instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.pre_fold_extra_instructions() != 0
                            || instruction.exception_policy() != ExceptionPolicy::NoneUnchanged
                    })
            })
        {
            return;
        }
        let cleanup = records[records.len() - 7..].to_vec();
        let earlier_cleanup = (manager_count == 2).then(|| {
            records[..7]
                .try_into()
                .expect("seven earlier cleanup records")
        });
        let mut suffix = items[pop_item..].iter().filter_map(|item| match item {
            Item::Instruction(instruction) => Some(*instruction),
            Item::Label(_) => None,
        });
        let (Some(pop), Some(load), Some(condition)) =
            (suffix.next(), suffix.next(), suffix.next())
        else {
            return;
        };
        if pop.opcode == NOP
            && pop.fusion_policy.is_converted_pop_block()
            && pop.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
            && pop.local_policy.hidden_exception_owner_slots() == 0
            && pop.pre_fold_extra_instructions() == 0
            && load.opcode == LOAD_FAST
            && load.local_policy.hidden_exception_owner_slots() == 0
            && load.pre_fold_extra_instructions() == 0
            && matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            && matches!(condition.operand(), Operand::Forward(_))
            && condition.local_policy.hidden_exception_owner_slots() == 3
            && condition.pre_fold_extra_instructions() == 0
        {
            self.exception_owner_provenance
                .copied_manager_return_compactions
                .push(exception_owners::CopiedManagerReturnCompaction {
                    ordinary_try_pops,
                    cleanup,
                    earlier_cleanup,
                    constant_return_prefix: None,
                    to_bool: None,
                    pop,
                    load,
                    condition,
                });
        }
    }

    /// Captures a constant return's exact cleanup before a synchronous manager exit.
    pub(crate) fn record_copied_constant_manager_return_compaction(
        &mut self,
        return_item: usize,
        pop_item: usize,
        named_handler: bool,
        missing_null_pops: usize,
        unnamed_handler_managers: usize,
    ) {
        use crate::target::opcodes::{CALL, DELETE_FAST, LOAD_CONST, STORE_FAST};

        if (named_handler && missing_null_pops != 2)
            || (!named_handler && missing_null_pops > 1)
            || (unnamed_handler_managers != 0
                && (named_handler
                    || !(1..=2).contains(&unnamed_handler_managers)
                    || missing_null_pops + 1 != unnamed_handler_managers))
        {
            return;
        }
        let items = self.linear_items();
        let Some(before) = items.get(return_item..pop_item) else {
            return;
        };
        let records = before
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let unnamed_handler = unnamed_handler_managers != 0;
        let prelude_start = 1 + usize::from(unnamed_handler);
        let prelude_len = if unnamed_handler {
            1 + 5 * (unnamed_handler_managers - 1)
        } else if named_handler {
            4
        } else {
            1
        };
        if records.len() != prelude_start + prelude_len + 5
            || items.get(return_item) != Some(&Item::Instruction(records[0]))
        {
            return;
        }
        let authored = records[0];
        let handler_pop = unnamed_handler.then_some(records[1]);
        let prelude = &records[prelude_start..prelude_start + prelude_len];
        let cleanup = &records[prelude_start + prelude_len..];
        if authored.opcode != NOP
            || authored.operand() != Operand::Value(0)
            || authored.fusion_policy.is_converted_pop_block()
            || authored.location.line < 0
            || prelude[0].location != authored.location
            || (!unnamed_handler
                && prelude
                    .iter()
                    .any(|instruction| instruction.location != authored.location))
            || handler_pop.is_some_and(|pop| {
                pop.opcode != NOP
                    || pop.operand() != Operand::Value(0)
                    || !pop.fusion_policy.is_converted_pop_block()
                    || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
                    || pop.location != authored.location
                    || pop.depth_after().is_none()
                    || pop.depth_after() != authored.depth_after()
            })
            || records.iter().enumerate().any(|(index, instruction)| {
                instruction.local_policy.hidden_exception_owner_slots() != 0
                    || instruction.pre_fold_extra_instructions() != 0
                    || (!(unnamed_handler && index == 1)
                        && instruction.exception_policy() != ExceptionPolicy::NoneUnchanged)
                    || !matches!(instruction.operand(), Operand::Value(_))
            })
            || (named_handler
                && (prelude.iter().map(|instruction| instruction.opcode).ne([
                    POP_EXCEPT,
                    LOAD_CONST,
                    STORE_FAST,
                    DELETE_FAST,
                ]) || prelude[2].operand() != prelude[3].operand()
                    || prelude[1].operand() != cleanup[0].operand()))
            || (!named_handler && !unnamed_handler && prelude[0].opcode != POP_TOP)
            || (unnamed_handler
                && (prelude[0].opcode != POP_EXCEPT
                    || prelude[0].operand() != Operand::Value(0)
                    || records[3..].as_chunks::<5>().0.iter().any(|manager| {
                        manager
                            .iter()
                            .map(|instruction| instruction.opcode)
                            .ne([LOAD_CONST, LOAD_CONST, LOAD_CONST, CALL, POP_TOP])
                            || manager[0].operand() != manager[1].operand()
                            || manager[1].operand() != manager[2].operand()
                            || manager[3].operand() != Operand::Value(3)
                            || manager[4].operand() != Operand::Value(0)
                            || manager
                                .iter()
                                .any(|record| record.location != manager[0].location)
                    })))
            || cleanup
                .iter()
                .map(|instruction| instruction.opcode)
                .ne([LOAD_CONST, LOAD_CONST, LOAD_CONST, CALL, POP_TOP])
            || cleanup[0].operand() != cleanup[1].operand()
            || cleanup[1].operand() != cleanup[2].operand()
            || cleanup[3].operand() != Operand::Value(3)
        {
            return;
        }
        let mut suffix = items[pop_item..].iter().filter_map(|item| match item {
            Item::Instruction(instruction) => Some(*instruction),
            Item::Label(_) => None,
        });
        let (Some(pop), Some(load), Some(condition)) =
            (suffix.next(), suffix.next(), suffix.next())
        else {
            return;
        };
        if pop.opcode == NOP
            && pop.fusion_policy.is_converted_pop_block()
            && pop.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
            && pop.local_policy.hidden_exception_owner_slots() == 0
            && pop.pre_fold_extra_instructions() == 0
            && matches!(load.opcode, LOAD_FAST | crate::target::opcodes::LOAD_DEREF)
            && load.local_policy.hidden_exception_owner_slots() == 0
            && load.pre_fold_extra_instructions() == 0
            && matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            && matches!(condition.operand(), Operand::Forward(_))
            && condition.local_policy.hidden_exception_owner_slots() == 3
            && condition.pre_fold_extra_instructions() == 0
        {
            self.exception_owner_provenance
                .copied_manager_return_compactions
                .push(exception_owners::CopiedManagerReturnCompaction {
                    ordinary_try_pops: usize::from(!named_handler && !unnamed_handler),
                    cleanup: cleanup.to_vec(),
                    earlier_cleanup: None,
                    constant_return_prefix: Some(exception_owners::ConstantManagerReturnPrefix {
                        authored_nop: authored,
                        retained_authored: false,
                        handler_pop,
                        instructions: prelude.to_vec(),
                        // Captured frontend nesting certifies the POPs before cleanup.
                        // The manager POP is counted separately in replay.
                        missing_null_pops,
                    }),
                    to_bool: None,
                    pop,
                    load,
                    condition,
                });
        }
    }

    /// Captures a constant manager return through an outer try into a primitive test.
    pub(crate) fn record_copied_try_manager_return_compaction(
        &mut self,
        return_item: usize,
        pop_item: usize,
        unnamed_handler: bool,
    ) {
        use crate::target::opcodes::CALL;

        let items = self.linear_items();
        let Some(before) = items.get(return_item..pop_item) else {
            return;
        };
        let records = before
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let cleanup_start = if unnamed_handler { 3 } else { 1 };
        if records.len() != cleanup_start + 5
            || items.get(return_item) != Some(&Item::Instruction(records[0]))
        {
            return;
        }
        let authored = records[0];
        let cleanup = &records[cleanup_start..];
        let handler_pop = unnamed_handler.then_some(records[1]);
        if authored.opcode != NOP
            || authored.operand() != Operand::Value(0)
            || authored.fusion_policy.is_converted_pop_block()
            || authored.location.line < 0
            || authored.depth_after().is_none()
            || cleanup
                .iter()
                .map(|record| record.opcode)
                .ne([LOAD_CONST, LOAD_CONST, LOAD_CONST, CALL, POP_TOP])
            || cleanup[0].operand() != cleanup[1].operand()
            || cleanup[1].operand() != cleanup[2].operand()
            || cleanup[3].operand() != Operand::Value(3)
            || cleanup[4].operand() != Operand::Value(0)
            || cleanup
                .iter()
                .any(|record| record.location != cleanup[0].location)
            || cleanup[4].depth_after() != Some(0)
            || authored.depth_after() != Some(if unnamed_handler { 3 } else { 2 })
            || (unnamed_handler
                && (records[1].opcode != NOP
                    || records[1].operand() != Operand::Value(0)
                    || !records[1].fusion_policy.is_converted_pop_block()
                    || records[1].location != authored.location
                    || records[1].depth_after() != authored.depth_after()
                    || records[2].opcode != POP_EXCEPT
                    || records[2].operand() != Operand::Value(0)
                    || records[2].location != authored.location
                    || records[2].depth_after() != Some(2)))
            || records.iter().enumerate().any(|(index, record)| {
                record.local_policy.hidden_exception_owner_slots() != 0
                    || record.pre_fold_extra_instructions() != 0
                    || !matches!(record.operand(), Operand::Value(_))
                    || record.exception_policy()
                        != if unnamed_handler && index == 1 {
                            ExceptionPolicy::EXCLUDE_ALWAYS
                        } else {
                            ExceptionPolicy::NoneUnchanged
                        }
            })
        {
            return;
        }
        let suffix = items[pop_item..]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .take(5)
            .collect::<Vec<_>>();
        let (pop, load, to_bool, condition, marker) = match suffix.as_slice() {
            [pop, load, to_bool, condition, marker] if to_bool.opcode == TO_BOOL => {
                (pop, load, Some(to_bool), condition, marker)
            }
            // The same unnamed-handler cleanup precedes a local None comparison.
            // Its three fused condition records are already modeled by None replay;
            // retain the existing generic POP-epoch requirement for this shape.
            [pop, load, condition, marker, ..]
                if unnamed_handler
                    && matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE) =>
            {
                (pop, load, None, condition, marker)
            }
            _ => return,
        };
        if pop.opcode != NOP
            || pop.operand() != Operand::Value(0)
            || !pop.fusion_policy.is_converted_pop_block()
            || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || pop.location.line >= 0
            || pop.depth_after() != Some(0)
            || load.opcode != LOAD_FAST
            || !matches!(load.operand(), Operand::Value(_))
            || to_bool.is_some_and(|record| record.operand() != Operand::Value(0))
            || (to_bool.is_some()
                && !matches!(condition.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE))
            || !matches!(condition.operand(), Operand::Forward(_))
            || marker.opcode != NOT_TAKEN
            || marker.operand() != Operand::Value(0)
            || marker.exception_policy()
                != if to_bool.is_some() {
                    ExceptionPolicy::NoneDynamicCondition
                } else {
                    ExceptionPolicy::NonePresent
                }
            || condition.location.line < 0
            || marker.location != condition.location
            || (if let Some(to_bool) = to_bool {
                load.location != condition.location || to_bool.location != condition.location
            } else {
                load.location.line != condition.location.line
                    || load.location.end_line != condition.location.end_line
                    || load.location.column != condition.location.column
                    || load.location.end_column > condition.location.end_column
            })
            || [load, condition]
                .into_iter()
                .chain(to_bool)
                .any(|record| record.exception_policy() != ExceptionPolicy::NoneUnchanged)
            || [pop, load, marker]
                .into_iter()
                .chain(to_bool)
                .any(|record| {
                    record.local_policy.hidden_exception_owner_slots() != 0
                        || record.pre_fold_extra_instructions() != 0
                })
            || condition.local_policy.hidden_exception_owner_slots()
                != if to_bool.is_some() { 0 } else { 3 }
            || condition.pre_fold_extra_instructions() != 0
        {
            return;
        }
        self.exception_owner_provenance
            .copied_manager_return_compactions
            .push(exception_owners::CopiedManagerReturnCompaction {
                ordinary_try_pops: 1,
                cleanup: cleanup.to_vec(),
                earlier_cleanup: None,
                constant_return_prefix: Some(exception_owners::ConstantManagerReturnPrefix {
                    authored_nop: authored,
                    retained_authored: !unnamed_handler,
                    handler_pop,
                    instructions: vec![if unnamed_handler {
                        records[2]
                    } else {
                        authored
                    }],
                    missing_null_pops: 0,
                }),
                to_bool: to_bool.copied(),
                pop: *pop,
                load: *load,
                condition: *condition,
            });
    }

    /// Captures the unchanged first array of an ordinary normal-finally entry.
    pub(crate) fn record_original_normal_finally_array(&mut self, ordinal: usize) {
        let items = self.linear_items();
        let Some(index) = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, Item::Instruction(_)))
            .nth(ordinal)
            .map(|(index, _)| index)
        else {
            return;
        };
        let Item::Instruction(marker) = items[index] else {
            return;
        };
        if marker.opcode != NOP
            || marker.operand() != Operand::Value(0)
            || !marker.fusion_policy.is_converted_pop_block()
            || marker.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
        {
            return;
        }
        let mut begin = index;
        while begin > 0 && matches!(items[begin - 1], Item::Label(_)) {
            begin -= 1;
        }
        let Some(Item::Label(entry)) = items.get(begin) else {
            return;
        };
        // One authored break establishes this original array independently of
        // the range labels around its POP_BLOCK. Do not alter borrow boundaries.
        let leading_labels = items[begin..index]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<rustc_hash::FxHashSet<_>>();
        let incoming_to_entry = items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(jump)
                    if matches!(jump.operand(), Operand::Forward(target) | Operand::Backward(target)
                        if leading_labels.contains(&target)) =>
                {
                    Some(*jump)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let authored_break_entry = matches!(incoming_to_entry.as_slice(), [jump]
            if is_unconditional_jump(jump.opcode)
                && jump.operand() == Operand::Forward(*entry)
                && jump.fusion_policy.is_authored_break()
                && jump.location.line >= 0);
        if ((!self
            .exception_owner_provenance
            .source_boundaries
            .contains(entry)
            || !self
                .exception_owner_provenance
                .block_boundaries
                .contains(entry))
            && !authored_break_entry)
            || !matches!(items.get(begin.wrapping_sub(1)), Some(Item::Instruction(previous))
                if ends_scope(previous.opcode))
        {
            return;
        }
        let mut retained_setup: Option<(Label, Label, Label)> = None;
        let mut end = index + 1;
        while matches!(items.get(end), Some(Item::Label(_))) {
            end += 1;
        }
        while let Some(item) = items.get(end) {
            let (instruction, crossed_setup_labels) = match item {
                Item::Instruction(instruction) => (instruction, false),
                Item::Label(_) => {
                    if retained_setup.is_some() {
                        return;
                    }
                    while matches!(items.get(end), Some(Item::Label(_))) {
                        end += 1;
                    }
                    let Some(Item::Instruction(instruction)) = items.get(end) else {
                        return;
                    };
                    if instruction.opcode != NOP {
                        return;
                    }
                    // This lookahead is provisional: the exact retained SETUP
                    // proof below must accept before any later record is scanned.
                    (instruction, true)
                }
            };
            end += 1;
            if is_conditional_jump(instruction.opcode) {
                if matches!(instruction.operand(), Operand::Forward(_)) {
                    while matches!(items.get(end), Some(Item::Label(_))) {
                        end += 1;
                    }
                    if !matches!(items.get(end), Some(Item::Instruction(marker)) if marker.opcode == NOT_TAKEN)
                    {
                        return;
                    }
                    end += 1;
                    let labels = items[begin..end]
                        .iter()
                        .filter_map(|item| match item {
                            Item::Label(label) => Some(*label),
                            Item::Instruction(_) => None,
                        })
                        .collect::<rustc_hash::FxHashSet<_>>();
                    if labels.iter().any(|label| {
                        (self.original_nop_targets.contains(label)
                            && !retained_setup
                                .is_some_and(|(start, end, _)| *label == start || *label == end))
                            || self
                                .exception_owner_provenance
                                .omitted_nop_counts
                                .contains_key(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_locations
                                .contains_key(label)
                            || self
                                .exception_owner_provenance
                                .fusion_boundaries
                                .contains(label)
                            || (self
                                .exception_owner_provenance
                                .setup_nop_boundaries
                                .contains(label)
                                && !retained_setup.is_some_and(|(start, end, _)| {
                                    *label == start || *label == end
                                }))
                    }) {
                        return;
                    }
                    let incoming = items.iter().filter_map(|item| match item {
                        Item::Instruction(jump) if matches!(jump.operand(), Operand::Forward(target) | Operand::Backward(target) if labels.contains(&target)) => Some(*jump),
                        _ => None,
                    }).collect::<Vec<_>>();
                    if incoming.is_empty()
                        || incoming.iter().any(|jump| {
                            !is_unconditional_jump(jump.opcode)
                                || jump.operand() != Operand::Forward(*entry)
                        })
                    {
                        return;
                    }
                    // This recorder runs after the finalizer suite and before its
                    // normal-exit jump. A label-only terminal join is not populated
                    // by that pending jump or by a later enclosing continuation.
                    let borrow_join = if retained_setup.is_none()
                        && let Operand::Forward(join) = instruction.operand()
                    {
                        let mut positions = items.iter().enumerate().filter_map(|(index, item)| {
                            (*item == Item::Label(join)).then_some(index)
                        });
                        positions.next().and_then(|position| {
                            if positions.next().is_some() || position < end {
                                return None;
                            }
                            let first =
                                items[position + 1..].iter().find_map(|item| match item {
                                    Item::Instruction(first) => Some(*first),
                                    Item::Label(_) => None,
                                })?;
                            (!matches!(first.opcode, NOP | NOT_TAKEN)
                                && matches!(first.operand(), Operand::Value(_))
                                && !is_conditional_jump(first.opcode)
                                && !is_unconditional_jump(first.opcode)
                                && !ends_scope(first.opcode)
                                && first.location.line >= 0
                                && first.pre_fold_extra_instructions() == 0
                                && !first.jump_policy.borrow_empty_block()
                                && first.no_location_policy.borrow_fallthrough()
                                    == BorrowFallthrough::Reachable)
                                .then_some((join, first))
                        })
                    } else {
                        None
                    };
                    let origin = exception_owners::OriginalNormalFinallyArray {
                        entry: *entry,
                        prefix: items[begin..end].to_vec(),
                        incoming,
                        retained_setup,
                        borrow_join,
                    };
                    self.exception_owner_provenance
                        .original_normal_finally_arrays
                        .push(origin);
                }
                return;
            }
            if !matches!(instruction.operand(), Operand::Value(_)) || ends_scope(instruction.opcode)
            {
                return;
            }
            // One authored retained SETUP can introduce range labels without
            // introducing another original instruction array.
            if instruction.opcode == NOP && retained_setup.is_none() {
                let mut before = end - 1;
                while before > index && matches!(items[before - 1], Item::Label(_)) {
                    before -= 1;
                }
                let mut after = end;
                while matches!(items.get(after), Some(Item::Label(_))) {
                    after += 1;
                }
                let before_labels = &items[before..end - 1];
                let after_labels = &items[end..after];
                let mut candidates = self
                    .exception_owner_provenance
                    .setup_nop_ranges
                    .iter()
                    .filter_map(|(start, finish)| {
                        if !before_labels.contains(&Item::Label(*start))
                            || !after_labels.contains(&Item::Label(*finish))
                        {
                            return None;
                        }
                        let mut setups = self.exception_owner_provenance.setups.iter().filter(
                            |(label, location)| {
                                *location == Some(instruction.location)
                                    && after_labels.contains(&Item::Label(*label))
                            },
                        );
                        let (setup, _) = setups.next()?;
                        if setups.next().is_some() {
                            return None;
                        }
                        Some((*start, *finish, *setup))
                    });
                if let Some(origin) = candidates.next() {
                    if candidates.next().is_some() {
                        return;
                    }
                    retained_setup = Some(origin);
                    end = after;
                } else if crossed_setup_labels {
                    return;
                }
            }
        }
    }

    /// Records the exact labels around one retained original normal `POP_BLOCK`.
    pub(crate) fn record_retained_normal_pop_block(
        &mut self,
        emitted: EmittedInstruction,
        entry: Label,
        start: Label,
        end: Label,
    ) {
        let instruction = *self.emitted_instruction_mut(emitted, "recording a normal POP_BLOCK");
        assert_eq!(instruction.opcode, NOP);
        assert_eq!(
            instruction.exception_policy(),
            ExceptionPolicy::EXCLUDE_ALWAYS
        );
        assert!(instruction.fusion_policy.is_converted_pop_block());
        assert!(instruction.jump_policy.preserves_explicit_nop());
        let index = usize::try_from(emitted.item_index).expect("emission index fits in usize");
        let items = self.linear_items();
        let mut before = items[..index]
            .iter()
            .rev()
            .take_while(|item| matches!(item, Item::Label(_)))
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        before.reverse();
        let after = items[index + 1..]
            .iter()
            .map(|item| match item {
                Item::Label(label) => *label,
                Item::Instruction(_) => unreachable!("the captured POP_BLOCK is still last"),
            })
            .collect::<Vec<_>>();
        // The emitter supplied these labels with no intervening original instruction.
        // A later graph pass must preserve this complete shape to use the certificate.
        assert_eq!(before, [entry, start]);
        assert_eq!(after.first(), Some(&end));
        self.exception_owner_provenance
            .retained_normal_pop_blocks
            .insert(
                entry,
                exception_owners::RetainedNormalPopBlock { before, after },
            );
    }

    /// Records the original empty prefix before a retained normal-finally `POP_BLOCK`.
    pub(crate) fn record_retained_pop_block_origin(&mut self, entry: Label) {
        let items = self.linear_items();
        let Some(Item::Instruction(marker)) = items.last() else {
            return;
        };
        if marker.opcode != NOP
            || !marker.fusion_policy.is_converted_pop_block()
            || !marker.jump_policy.preserves_explicit_nop()
            || marker.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || !self.preserved_block_boundaries.contains(&entry)
        {
            return;
        }
        let prefix = items[..items.len() - 1]
            .iter()
            .rev()
            .take_while(|item| matches!(item, Item::Label(_)))
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if prefix.contains(&entry) {
            self.retained_pop_block_origins.insert(entry, prefix);
        }
    }

    /// Records an ordinary handler exit copied from an original empty join.
    pub(crate) fn record_original_handler_cleanup_join_exit(&mut self, end: Label) {
        let items = self.linear_items();
        let Some([Item::Instruction(pop), Item::Instruction(jump)]) =
            items.get(items.len().saturating_sub(2)..)
        else {
            return;
        };
        if pop.opcode == POP_EXCEPT
            && pop.location == SourceLocation::NONE
            && jump.opcode == JUMP_FORWARD
            && jump.operand() == Operand::Forward(end)
            && jump.location == SourceLocation::NONE
        {
            self.exception_owner_provenance
                .original_handler_cleanup_join_exits
                .insert(end);
        }
    }

    /// Finds the sole original match donor for a manager's normal `POP_BLOCK`.
    pub(crate) fn original_manager_match_join(
        &self,
        protected_end: Label,
    ) -> Option<(Label, SourceLocation)> {
        let entry = self.original_match_join?;
        let items = self.linear_items();
        let aliases = items
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if !aliases.contains(&entry)
            || !aliases.contains(&protected_end)
            || self.join_labels_have_authored_location(&aliases)
        {
            return None;
        }
        let start = items.len() - aliases.len();
        let Item::Instruction(previous) = items.get(start.checked_sub(1)?)? else {
            return None;
        };
        let intact = |instruction: &Instruction| {
            instruction.pre_fold_extra_instructions() == 0
                && instruction.local_policy.hidden_exception_owner_slots() == 0
                && instruction.exception_policy() == ExceptionPolicy::default()
                && !instruction.fusion_policy.is_authored_break()
                && !instruction.fusion_policy.is_converted_pop_block()
                && !instruction.jump_policy.borrow_empty_block()
                && !instruction.jump_policy.always_retains_nop()
                && !instruction.no_location_policy.preserves_location()
                && instruction.depth_after().is_some()
        };
        let incoming = |targets: &[Label]| {
            items.iter().filter_map(move |item| match item {
                Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if targets.contains(&label)) => Some(instruction),
                _ => None,
            }).collect::<Vec<_>>()
        };
        let mut entries = incoming(&aliases).into_iter();
        if let Some(jump) = entries.next() {
            // A terminating fallback leaves only this original successful case.
            // Its located body is the sole donor for the separate manager array.
            return (entries.next().is_none()
                && ends_scope(previous.opcode)
                && jump.opcode == JUMP_FORWARD
                && jump.operand() == Operand::Forward(entry)
                && jump.jump_policy.is_original_match_success()
                && jump.location.line >= 0
                && !jump.no_location_policy.is_artificial_eval_break()
                && intact(jump))
            .then_some((entry, jump.location));
        }
        // With all successful cases terminated, CPython's dead case-success
        // edges still establish the end after the final class failure pops.
        let [
            head @ ..,
            Item::Label(first_label),
            Item::Instruction(first),
            Item::Label(last_label),
            Item::Instruction(last),
        ] = &items[..start]
        else {
            return None;
        };
        let provenance = &self.exception_owner_provenance;
        if !provenance.block_boundaries.contains(&entry)
            || !provenance.source_boundaries.contains(&entry)
            || first.opcode != POP_TOP
            || last.opcode != POP_TOP
            || first.operand() != Operand::Value(0)
            || last.operand() != Operand::Value(0)
            || first.location.line < 0
            || first.location != last.location
            || !intact(first)
            || !intact(last)
            || first.depth_after().and_then(|depth| depth.checked_sub(1)) != last.depth_after()
            || !incoming(&[*last_label]).is_empty()
        {
            return None;
        }
        let first_aliases = head
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .chain(std::iter::once(*first_label))
            .collect::<Vec<_>>();
        if self.join_labels_have_authored_location(&first_aliases)
            || self.join_labels_have_authored_location(&[*last_label])
            || !head.iter().rev().find_map(|item| match item {
                Item::Instruction(instruction) => Some(ends_scope(instruction.opcode)),
                Item::Label(_) => None,
            })?
        {
            return None;
        }
        let mut failures = incoming(&first_aliases).into_iter();
        let condition = failures.next()?;
        (failures.next().is_none()
            && condition.opcode == POP_JUMP_IF_NONE
            && matches!(condition.operand(), Operand::Forward(_))
            && condition.location.line == first.location.line
            && condition.location.column == first.location.column
            && intact(condition)
            && condition
                .depth_after()
                .and_then(|depth| depth.checked_sub(1))
                == first.depth_after())
        .then_some((entry, last.location))
    }

    /// Finds the original manager exit following a completed cold handler cleanup.
    pub(crate) fn original_manager_cleanup_join(&self, protected_end: Label) -> Option<Label> {
        let items = self.linear_items();
        let aliases = items
            .iter()
            .rev()
            .map_while(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        if !aliases.contains(&protected_end) {
            return None;
        }
        let start = items.len() - aliases.len();
        let [
            Item::Instruction(copy),
            Item::Instruction(pop),
            Item::Instruction(reraise),
        ] = items.get(start.checked_sub(3)?..start)?
        else {
            return None;
        };
        if copy.opcode != COPY
            || copy.operand() != Operand::Value(3)
            || pop.opcode != POP_EXCEPT
            || reraise.opcode != RERAISE
            || reraise.operand() != Operand::Value(1)
            || [copy, pop, reraise].iter().any(|instruction| {
                instruction.location != SourceLocation::NONE
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.local_policy.hidden_exception_owner_slots() != 0
            })
        {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut entries = aliases
            .iter()
            .copied()
            .filter(|label| provenance.block_boundaries.contains(label));
        let entry = entries.next()?;
        if entries.next().is_some()
            || !provenance.source_boundaries.contains(&entry)
            || !provenance
                .original_handler_cleanup_join_exits
                .contains(&entry)
        {
            return None;
        }
        Some(entry)
    }

    /// Records a manager's original `POP_BLOCK` at the head of its cleanup array.
    pub(crate) fn record_original_manager_pop_block(&mut self, label: Label) {
        self.exception_owner_provenance
            .original_manager_pop_blocks
            .push(label);
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
        let items = self.linear_items();
        let Item::Instruction(instruction) = items[index] else {
            unreachable!();
        };
        let retained_constant_pass = self.exception_owner_provenance.constant_pass_clusters.iter().any(|cluster| {
            index.checked_sub(3).is_some_and(|start| {
                matches!(&items[start..=index], [
                    Item::Label(first), Item::Instruction(constant),
                    Item::Label(second), Item::Instruction(pass),
                ] if *first == cluster.start && *second == cluster.end
                    && constant.opcode == NOP && constant.location == cluster.constant_location
                    && pass.opcode == NOP && pass.location == cluster.pass_location
                    && constant.operand() == Operand::Value(0) && pass.operand() == Operand::Value(0)
                    && !constant.fusion_policy.is_converted_pop_block()
                    && !pass.fusion_policy.is_converted_pop_block()
                    && !constant.jump_policy.preserves_explicit_nop()
                    && !pass.jump_policy.preserves_explicit_nop()
                    && !constant.jump_policy.borrow_empty_block()
                    && !pass.jump_policy.borrow_empty_block()
                    && constant.pre_fold_extra_instructions() == 0
                    && pass.pre_fold_extra_instructions() == 0
                    && constant.local_policy.hidden_exception_owner_slots() == 0
                    && pass.local_policy.hidden_exception_owner_slots() == 0)
            })
        });
        if instruction.opcode != NOP
            || instruction.jump_policy.preserves_explicit_nop()
            || retained_constant_pass
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
        // An incoming jump and the NOP's fallthrough share this block head. CPython
        // cannot donate the preceding block's location to its unlocated first instruction.
        if instruction.location.line >= 0 && current_location.line < 0 && self.at_jump_target() {
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
        // Capture the original fallthrough before CFG threading can invert its
        // predecessor. The SETUP NOP is the first record of this distinct CP array.
        let original_fallthrough = {
            let mut instructions = self.linear_items().iter().rev().filter_map(|item| {
                if let Item::Instruction(instruction) = item {
                    Some(*instruction)
                } else {
                    None
                }
            });
            matches!(
                (instructions.next(), instructions.next(), instructions.next()),
                (Some(nop), Some(marker), Some(condition))
                    if nop.opcode == NOP
                        && marker.opcode == NOT_TAKEN
                        && self
                            .exception_owner_provenance
                            .original_primitive_condition_boundaries
                            .contains(&condition)
            )
        };
        if original_fallthrough {
            self.exception_owner_provenance
                .original_fallthrough_setups
                .insert(start);
        }
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

    /// Identifies a synthetic range start immediately before a value-preserving jump.
    pub(crate) fn record_value_jump_owner_continuation(&mut self, label: Label) {
        self.exception_owner_provenance
            .value_jump_continuation_boundaries
            .insert(label);
    }

    /// Captures the two original NULL POP records around a saved handler return value.
    pub(crate) fn record_named_handler_pop_pair(&mut self, start: usize) {
        let items = self.linear_items();
        let Some(entry) = items[..start].iter().rev().find_map(|item| match item {
            Item::Label(label)
                if self
                    .exception_owner_provenance
                    .block_boundaries
                    .contains(label) =>
            {
                Some(*label)
            }
            _ => None,
        }) else {
            return;
        };
        let original = items[start..].to_vec();
        let records = original
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let [first, swap, second, pop_except] = records.as_slice() else {
            return;
        };
        if swap.opcode != SWAP
            || swap.operand() != Operand::Value(2)
            || pop_except.opcode != POP_EXCEPT
            || pop_except.operand() != Operand::Value(0)
            || [first, second].iter().any(|pop| {
                pop.opcode != NOP
                    || pop.operand() != Operand::Value(0)
                    || !pop.fusion_policy.is_converted_pop_block()
                    || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            })
            || records.iter().any(|instruction| {
                instruction.local_policy.hidden_exception_owner_slots() != 0
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.location != swap.location
                    || instruction.jump_policy.borrow_empty_block()
                    || instruction.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::Reachable
            })
        {
            return;
        }
        self.exception_owner_provenance
            .named_handler_pop_pairs
            .push(exception_owners::OriginalPopBlockSpan {
                entry,
                items: original,
            });
    }

    /// Captures an iterator return's final POP run and its first local load.
    pub(crate) fn record_copied_handler_finally_pop_prefix(&mut self, pop_item: usize) {
        let items = self.linear_items();
        let Some(Item::Instruction(pop)) = items.get(pop_item) else {
            return;
        };
        let Some((offset, Item::Instruction(load))) = items[pop_item + 1..]
            .iter()
            .enumerate()
            .find(|(_, item)| matches!(item, Item::Instruction(_)))
        else {
            return;
        };
        // An ordinary try outside the iterator contributes one additional POP
        // immediately before the finalizer POP. Retain both original NULL cells.
        let preceding_pop = pop_item
            .checked_sub(1)
            .and_then(|index| items.get(index))
            .and_then(|item| match item {
                Item::Instruction(instruction)
                    if instruction.fusion_policy.is_converted_pop_block() =>
                {
                    Some(instruction)
                }
                _ => None,
            });
        if [Some(pop), preceding_pop]
            .into_iter()
            .flatten()
            .any(|record| {
                record.opcode != NOP
                    || record.operand() != Operand::Value(0)
                    || !record.fusion_policy.is_converted_pop_block()
                    || record.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
                    || record.location != pop.location
                    || record.depth_after() != pop.depth_after()
                    || record.local_policy.hidden_exception_owner_slots() != 0
                    || record.pre_fold_extra_instructions() != 0
                    || record.jump_policy.borrow_empty_block()
                    || record.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::Reachable
            })
            || load.opcode != LOAD_FAST
            || !matches!(load.operand(), Operand::Value(_))
            || [pop, load].iter().any(|instruction| {
                instruction.local_policy.hidden_exception_owner_slots() != 0
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.jump_policy.borrow_empty_block()
                    || instruction.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::Reachable
            })
        {
            return;
        }
        let first_pop_item = if preceding_pop.is_some() {
            pop_item - 1
        } else {
            pop_item
        };
        let mut start = first_pop_item;
        while start > 0 && matches!(items[start - 1], Item::Label(_)) {
            start -= 1;
        }
        if start == first_pop_item {
            return;
        }
        let Some(entry) = items[..start].iter().rev().find_map(|item| match item {
            Item::Label(label)
                if self
                    .exception_owner_provenance
                    .block_boundaries
                    .contains(label) =>
            {
                Some(*label)
            }
            _ => None,
        }) else {
            return;
        };
        // The already captured handler pair authenticates this original return array.
        // Its replay must also succeed before this later POP can donate a NULL cell.
        if !self
            .exception_owner_provenance
            .named_handler_pop_pairs
            .iter()
            .any(|pair| pair.entry == entry)
        {
            return;
        }
        let original = items[start..=pop_item + 1 + offset].to_vec();
        self.exception_owner_provenance
            .copied_handler_finally_pops
            .push(exception_owners::OriginalPopBlockSpan {
                entry,
                items: original,
            });
    }

    /// Captures the exact emitted records of a constant-return handler cleanup.
    pub(crate) fn record_copied_handler_cleanup(
        &mut self,
        start: Label,
        end: Label,
        return_item: usize,
        named: bool,
    ) {
        self.record_copied_handler_cleanup_inner(start, end, return_item, named, None, false);
    }

    /// Captures an unnamed handler unwind and its immediately copied local assignment.
    pub(crate) fn record_copied_handler_assignment_cleanup(
        &mut self,
        start: Label,
        handler_end: Label,
        end: Label,
        return_item: usize,
    ) {
        self.record_copied_handler_cleanup_inner(
            start,
            end,
            return_item,
            false,
            Some(handler_end),
            false,
        );
    }

    /// Captures an inner iterator disposal before an enclosing named-handler cleanup.
    pub(crate) fn record_copied_iterator_handler_cleanup(
        &mut self,
        start: Label,
        end: Label,
        return_item: usize,
    ) {
        self.record_copied_handler_cleanup_inner(start, end, return_item, true, None, true);
    }

    fn record_copied_handler_cleanup_inner(
        &mut self,
        start: Label,
        end: Label,
        return_item: usize,
        named: bool,
        assignment_pop_boundary: Option<Label>,
        inner_iterator: bool,
    ) {
        let copied_assignment = assignment_pop_boundary.is_some();
        let items = self.linear_items();
        let Some(Item::Instruction(return_nop)) = items.get(return_item) else {
            return;
        };
        if return_nop.opcode != NOP {
            return;
        }
        let return_nop = *return_nop;
        let Some(start_at) = items.iter().position(|item| *item == Item::Label(start)) else {
            return;
        };
        let Some(end_at) = items.iter().position(|item| *item == Item::Label(end)) else {
            return;
        };
        if return_item >= start_at || start_at >= end_at {
            return;
        }
        let prefix = items[return_item..start_at]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let return_iterator = if inner_iterator {
            let [authored, iterator] = prefix.as_slice() else {
                return;
            };
            if !named
                || copied_assignment
                || authored.fusion_policy.is_converted_pop_block()
                || authored.location.line < 0
                || iterator.opcode != POP_TOP
                || iterator.operand() != Operand::Value(0)
                || iterator.location != authored.location
                || authored
                    .depth_after()
                    .and_then(|depth| depth.checked_sub(1))
                    != iterator.depth_after()
                || [authored, iterator].into_iter().any(|record| {
                    record.exception_policy() != ExceptionPolicy::NoneUnchanged
                        || record.local_policy.hidden_exception_owner_slots() != 0
                        || record.pre_fold_extra_instructions() != 0
                })
            {
                return;
            }
            Some(**iterator)
        } else {
            None
        };
        let (missing_null_pops, removed_prefix_slots) = if return_iterator.is_some() {
            // The inner ordinary try retires before iterator disposal; the two
            // named-handler POPs follow it, before the four protected clear records.
            (3, 0)
        } else {
            match (named, prefix.as_slice()) {
                (true, [_]) => (2, 1),
                (true, [_, first, second])
                    if return_nop.exception_policy() == ExceptionPolicy::NoneUnchanged
                        && return_nop.local_policy.hidden_exception_owner_slots() == 0
                        && return_nop.pre_fold_extra_instructions() == 0
                        && [first, second].into_iter().all(|pop| {
                            pop.opcode == NOP
                                && pop.operand() == Operand::Value(0)
                                && pop.fusion_policy.is_converted_pop_block()
                                && pop.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
                                && pop.local_policy.hidden_exception_owner_slots() == 0
                                && pop.pre_fold_extra_instructions() == 0
                                && pop.location == return_nop.location
                        }) =>
                {
                    (0, 3)
                }
                (false, [_, pop])
                    if pop.opcode == NOP && pop.fusion_policy.is_converted_pop_block() =>
                {
                    (0, 2)
                }
                _ => return,
            }
        };
        let mut instructions = items[start_at..end_at]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let assignment_pop = if let Some(boundary) = assignment_pop_boundary {
            let Some(pop_at) = items.iter().position(|item| *item == Item::Label(boundary)) else {
                return;
            };
            if start_at >= pop_at
                || pop_at >= end_at
                || items[start_at..pop_at]
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count()
                    != 1
            {
                return;
            }
            let [_, pop, constant, store] = instructions.as_slice() else {
                return;
            };
            if named
                || return_nop.exception_policy() != ExceptionPolicy::NoneUnchanged
                || return_nop.local_policy.hidden_exception_owner_slots() != 0
                || return_nop.pre_fold_extra_instructions() != 0
                || pop.opcode != NOP
                || pop.operand() != Operand::Value(0)
                || !pop.fusion_policy.is_converted_pop_block()
                || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
                || pop.local_policy.hidden_exception_owner_slots() != 0
                || pop.pre_fold_extra_instructions() != 0
                || constant.opcode != crate::target::opcodes::LOAD_CONST
                || store.opcode != crate::target::opcodes::STORE_FAST
                || [constant, store].into_iter().any(|instruction| {
                    instruction.exception_policy() != ExceptionPolicy::NoneUnchanged
                        || instruction.local_policy.hidden_exception_owner_slots() != 0
                        || instruction.pre_fold_extra_instructions() != 0
                })
            {
                return;
            }
            Some((boundary, instructions.remove(1)))
        } else {
            None
        };
        let expected_len = if copied_assignment {
            3
        } else if named {
            4
        } else {
            1
        };
        if instructions.len() != expected_len
            || instructions[0].opcode != crate::target::opcodes::POP_EXCEPT
            || (named
                && (instructions[1].opcode != crate::target::opcodes::LOAD_CONST
                    || instructions[2].opcode != crate::target::opcodes::STORE_FAST
                    || instructions[3].opcode != crate::target::opcodes::DELETE_FAST
                    || instructions[2].operand() != instructions[3].operand()))
        {
            return;
        }
        self.exception_owner_provenance
            .copied_handler_cleanups
            .push(exception_owners::CopiedHandlerCleanup {
                start,
                end,
                return_nop,
                return_iterator,
                instructions,
                missing_null_pops,
                removed_prefix_slots,
                assignment_pop,
                continued_call: None,
                continued_attribute_none: None,
            });
    }

    /// Captures a direct named-handler break followed by its copied local None test.
    pub(crate) fn record_copied_break_handler_cleanup(&mut self, break_item: usize) {
        let items = self.linear_items();
        let records = items
            .iter()
            .enumerate()
            .skip(break_item)
            .filter_map(|(index, item)| match item {
                Item::Instruction(instruction) => Some((index, *instruction)),
                Item::Label(_) => None,
            })
            .take(8)
            .collect::<Vec<_>>();
        if records.len() != 8 || records[0].0 != break_item {
            return;
        }
        let authored = records[0].1;
        let cleanup = &records[1..5];
        let load = records[5].1;
        let condition = records[6].1;
        let marker = records[7].1;
        if authored.opcode != NOP
            || authored.location.line < 0
            || cleanup[0].1.opcode != POP_EXCEPT
            || cleanup[1].1.opcode != LOAD_CONST
            || cleanup[2].1.opcode != STORE_FAST
            || cleanup[3].1.opcode != DELETE_FAST
            || cleanup[2].1.operand() != cleanup[3].1.operand()
            || cleanup.iter().any(|(_, instruction)| {
                instruction.location != authored.location
                    || instruction.exception_policy() != ExceptionPolicy::default()
            })
            || !matches!(load.opcode, LOAD_FAST | crate::target::opcodes::LOAD_DEREF)
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition.local_policy.hidden_exception_owner_slots() != 3
            || condition.pre_fold_extra_instructions() != 0
            || condition.exception_policy() != ExceptionPolicy::default()
            || marker.opcode != NOT_TAKEN
            || marker.location != condition.location
            || marker.operand() != Operand::Value(0)
            || records.iter().enumerate().any(|(index, (_, instruction))| {
                (index != 6 && instruction.local_policy.hidden_exception_owner_slots() != 0)
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::Reachable
            })
        {
            return;
        }
        let labels = items[break_item..records[7].0]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let provenance = &self.exception_owner_provenance;
        if labels.iter().any(|label| {
            provenance.source_boundaries.contains(label)
                || provenance.block_boundaries.contains(label)
                || provenance.fusion_boundaries.contains(label)
                || provenance.setups.iter().any(|(setup, _)| setup == label)
                || provenance.omitted_nop_counts.contains_key(label)
                || provenance.omitted_nop_locations.contains_key(label)
                || self.preserved_block_boundaries.contains(label)
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == *label)
        }) || items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
            if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                if labels.contains(&target)))
        }) {
            return;
        }
        let Some(start) = items[break_item + 1..records[1].0]
            .iter()
            .find_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
        else {
            return;
        };
        let Some(end) = items[records[4].0 + 1..records[5].0]
            .iter()
            .find_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
        else {
            return;
        };
        self.record_copied_handler_cleanup(start, end, break_item, true);
    }

    /// Captures a continued local None test with an emitted fallthrough suffix.
    pub(crate) fn record_copied_continue_none_handler_cleanup(&mut self, continue_item: usize) {
        let items = self.linear_items();
        let records = items
            .iter()
            .enumerate()
            .skip(continue_item)
            .filter_map(|(index, item)| match item {
                Item::Instruction(instruction) => Some((index, *instruction)),
                Item::Label(_) => None,
            })
            .take(8)
            .collect::<Vec<_>>();
        if records.len() != 8 || records[0].0 != continue_item {
            return;
        }
        let condition = records[6].1;
        if !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE) {
            return;
        }
        let Operand::Forward(target) = condition.operand() else {
            return;
        };
        let mut targets = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| (*item == Item::Label(target)).then_some(index));
        let Some(target_index) = targets.next() else {
            return;
        };
        if targets.next().is_some() || target_index <= records[7].0 {
            return;
        }
        let Some(suffix) = items[target_index + 1..]
            .iter()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
        else {
            return;
        };
        // This recorder runs before the pending continue jump is emitted. A terminal
        // join has no stable suffix yet and may acquire a fresh backward marker.
        if matches!(suffix.opcode, NOP | NOT_TAKEN)
            || is_conditional_jump(suffix.opcode)
            || is_unconditional_jump(suffix.opcode)
            || ends_scope(suffix.opcode)
        {
            return;
        }
        self.record_copied_break_handler_cleanup(continue_item);
    }

    /// Captures a direct named-handler continue and its first copied call or attribute None test.
    pub(crate) fn record_copied_continue_handler_cleanup(&mut self, continue_item: usize) {
        use crate::target::opcodes::CALL;

        let items = self.linear_items();
        let records = items
            .iter()
            .enumerate()
            .skip(continue_item)
            .filter_map(|(index, item)| match item {
                Item::Instruction(instruction) => Some((index, *instruction)),
                Item::Label(_) => None,
            })
            .take(12)
            .collect::<Vec<_>>();
        if records.len() < 9 || records[0].0 != continue_item {
            return;
        }
        let continuation = records[0].1;
        let cleanup = records[1..5]
            .iter()
            .map(|(_, instruction)| *instruction)
            .collect::<Vec<_>>();
        let attribute_none = records[5].1.opcode == LOAD_FAST
            && records[6].1.opcode == LOAD_ATTR
            && matches!(records[6].1.operand(), Operand::Value(argument) if argument & 1 == 0)
            && matches!(records[7].1.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            && records[7].1.local_policy.hidden_exception_owner_slots() == 3;
        let marker_at = if attribute_none { 8 } else { 11 };
        if records.len() <= marker_at {
            return;
        }
        let condition = &records[5..marker_at];
        let last = condition.last().expect("nonempty copied condition").1;
        let marker = records[marker_at].1;
        let call = if attribute_none {
            None
        } else {
            let call: [Instruction; 6] = std::array::from_fn(|index| records[index + 5].1);
            if call[0].opcode != LOAD_GLOBAL
                || !matches!(call[0].operand(), Operand::Value(argument) if argument & 1 != 0)
                || call[1].opcode != LOAD_FAST
                || call[2].opcode != LOAD_GLOBAL
                || !matches!(call[2].operand(), Operand::Value(argument) if argument & 1 == 0)
                || call[3].opcode != CALL
                || call[3].operand() != Operand::Value(2)
                || call[4].opcode != TO_BOOL
                || !matches!(call[5].opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
            {
                return;
            }
            Some(call)
        };
        if continuation.opcode != NOP
            || continuation.location.line < 0
            || cleanup[0].opcode != POP_EXCEPT
            || cleanup[1].opcode != LOAD_CONST
            || cleanup[2].opcode != STORE_FAST
            || cleanup[3].opcode != DELETE_FAST
            || cleanup[2].operand() != cleanup[3].operand()
            || cleanup
                .iter()
                .any(|instruction| instruction.location != continuation.location)
            || !matches!(last.operand(), Operand::Forward(_))
            || marker.opcode != NOT_TAKEN
            || marker.operand() != Operand::Value(0)
            || marker.location != last.location
            || records[..=marker_at]
                .iter()
                .enumerate()
                .any(|(index, (_, instruction))| {
                    instruction.local_policy.hidden_exception_owner_slots()
                        != if attribute_none && index == 7 { 3 } else { 0 }
                        || instruction.pre_fold_extra_instructions() != 0
                        || instruction.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::Reachable
                })
            || cleanup
                .iter()
                .chain(condition.iter().map(|(_, instruction)| instruction))
                .any(|instruction| instruction.exception_policy() != ExceptionPolicy::default())
        {
            return;
        }
        let labels = items[continue_item..records[marker_at].0]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let provenance = &self.exception_owner_provenance;
        if labels.iter().any(|label| {
            provenance.source_boundaries.contains(label)
                || provenance.block_boundaries.contains(label)
                || provenance.fusion_boundaries.contains(label)
                || provenance.setups.iter().any(|(setup, _)| setup == label)
                || provenance.omitted_nop_counts.contains_key(label)
                || provenance.omitted_nop_locations.contains_key(label)
                || self.preserved_block_boundaries.contains(label)
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == *label)
        }) || items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
            if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                if labels.contains(&target)))
        }) {
            return;
        }
        let Some(start) = items[continue_item + 1..records[1].0]
            .iter()
            .find_map(|item| {
                if let Item::Label(label) = item {
                    Some(*label)
                } else {
                    None
                }
            })
        else {
            return;
        };
        let Some(end) = items[records[4].0 + 1..records[5].0]
            .iter()
            .find_map(|item| {
                if let Item::Label(label) = item {
                    Some(*label)
                } else {
                    None
                }
            })
        else {
            return;
        };
        let previous_count = self
            .exception_owner_provenance
            .copied_handler_cleanups
            .len();
        self.record_copied_handler_cleanup(start, end, continue_item, true);
        if self
            .exception_owner_provenance
            .copied_handler_cleanups
            .len()
            == previous_count + 1
        {
            let origin =
                &mut self.exception_owner_provenance.copied_handler_cleanups[previous_count];
            if let Some(call) = call {
                origin.continued_call = Some((call, marker));
            } else {
                origin.continued_attribute_none =
                    Some((std::array::from_fn(|index| records[index + 5].1), marker));
            }
        }
    }

    /// Records an authored return NOP and its direct finally unwind records.
    pub(crate) fn record_early_return_nop_pair(&mut self, return_item: usize, target: Label) {
        let items = self.linear_items();
        let Some(Item::Instruction(return_nop)) = items.get(return_item) else {
            return;
        };
        let instructions = items[return_item..]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        if let [authored, iterator, pop] = instructions.as_slice()
            && authored == return_nop
            && authored.opcode == NOP
            && !authored.fusion_policy.is_converted_pop_block()
            && iterator.opcode == POP_TOP
            && pop.opcode == NOP
            && pop.fusion_policy.is_converted_pop_block()
            && authored.exception_policy() == ExceptionPolicy::NoneUnchanged
            && iterator.exception_policy() == ExceptionPolicy::NoneUnchanged
            && pop.exception_policy() == ExceptionPolicy::EXCLUDE_ALWAYS
            && iterator
                .depth_after()
                .and_then(|depth| depth.checked_add(1))
                == authored.depth_after()
            && pop.depth_after() == iterator.depth_after()
            && instructions.iter().all(|record| {
                record.operand() == Operand::Value(0)
                    && record.location == authored.location
                    && record.local_policy.hidden_exception_owner_slots() == 0
                    && record.pre_fold_extra_instructions() == 0
                    && record.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Reachable
                    && !record.jump_policy.borrow_empty_block()
            })
        {
            self.exception_owner_provenance
                .early_return_iterator_pops
                .push((*authored, *iterator, *pop, target));
            return;
        }
        let [authored, pop] = instructions.as_slice() else {
            return;
        };
        if authored != return_nop
            || authored.opcode != NOP
            || authored.exception_policy() != ExceptionPolicy::NoneUnchanged
            || authored.local_policy.hidden_exception_owner_slots() != 0
            || authored.pre_fold_extra_instructions() != 0
            || pop.opcode != NOP
            || !pop.fusion_policy.is_converted_pop_block()
            || pop.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
            || pop.local_policy.hidden_exception_owner_slots() != 0
            || pop.pre_fold_extra_instructions() != 0
        {
            return;
        }
        self.exception_owner_provenance
            .early_return_nop_pairs
            .push((*authored, *pop, target));
    }

    /// Records a simple For target without inserting a physical fusion boundary.
    pub(crate) fn record_for_target_owner_prefix(&mut self, entry: Label) {
        let items = self.linear_items();
        let Some(
            [
                Item::Label(label),
                Item::Instruction(nop),
                Item::Instruction(store),
            ],
        ) = items.get(items.len().saturating_sub(3)..)
        else {
            return;
        };
        if *label != entry
            || store.opcode != STORE_FAST
            || !matches!(store.operand(), Operand::Value(_))
            || store.local_policy.hidden_exception_owner_slots() != 0
            || store.pre_fold_extra_instructions() != 0
            || *nop != Instruction::new(NOP, Operand::Value(0), store.location, nop.depth_after())
        {
            return;
        }
        let store = *store;
        self.exception_owner_provenance
            .for_target_owner_prefixes
            .push((entry, store));
    }

    /// Starts a protected prefix within the same original instruction array.
    pub(crate) fn record_exception_owner_prefix(&mut self, label: Label) {
        self.record_exception_owner_continuation(label);
        self.exception_owner_provenance
            .protected_prefix_boundaries
            .insert(label);
    }

    /// Records an original array boundary reached by a primitive source condition.
    pub(crate) fn record_original_primitive_condition_boundary(&mut self, target: Label) {
        let condition = *self.last_jump_instruction_mut("recording an original condition boundary");
        assert!(matches!(
            condition.opcode,
            POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE
        ));
        assert_eq!(condition.operand(), Operand::Forward(target));
        self.exception_owner_provenance
            .original_primitive_condition_boundaries
            .push(condition);
    }

    /// Records the original jump into a comprehension's normal restore array.
    pub(crate) fn record_original_comprehension_normal_exit(
        &mut self,
        emitted: EmittedJump,
        entry: Label,
    ) {
        let jump = *self.emitted_jump_mut(emitted, "recording a comprehension normal exit");
        assert_eq!(jump.opcode, JUMP_FORWARD);
        assert_eq!(jump.operand(), Operand::Forward(entry));
        self.exception_owner_provenance
            .original_comprehension_normal_exits
            .push((entry, jump));
    }

    /// Anchors the protected continuation after an original synchronous normal restore.
    pub(crate) fn record_comprehension_restore_owner_prefix(&mut self, entry: Label) {
        let restore =
            *self.required_last_instruction_mut("recording a normal comprehension restore");
        assert_eq!(restore.opcode, STORE_FAST);
        self.exception_owner_provenance
            .comprehension_restore_owner_prefixes
            .push((entry, restore));
    }

    /// Seals the immediate normal restore records after their fusion policies are final.
    pub(crate) fn record_original_comprehension_normal_restores(&mut self, entry: Label) {
        let items = self.linear_items();
        let Some(start) = items.iter().rposition(|item| *item == Item::Label(entry)) else {
            return;
        };
        let Some(restores) = items[start + 1..]
            .iter()
            .map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        self.exception_owner_provenance
            .original_comprehension_normal_restores
            .insert(entry, restores);
    }

    /// Retains an original instruction-array boundary for borrowed-reference analysis.
    pub(crate) fn record_borrow_analysis_block_boundary(&mut self, label: Label) {
        self.exception_owner_provenance
            .borrow_only_block_boundaries
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

    /// Captures a source None comparison without changing its instruction metadata.
    pub(crate) fn record_source_none_condition(&mut self) {
        let items = self.linear_items();
        let Some(Item::Instruction(instruction)) = items.last().copied() else {
            return;
        };
        let Some(begin) = items.iter().rposition(|item| {
            matches!(item, Item::Label(label)
                if self.exception_owner_provenance.source_boundaries.contains(label)
                    && self.exception_owner_provenance.block_boundaries.contains(label))
        }) else {
            return;
        };
        let Item::Label(entry) = items[begin] else {
            unreachable!()
        };
        let labels = items[begin..]
            .iter()
            .filter_map(|item| {
                if let Item::Label(label) = item {
                    Some(*label)
                } else {
                    None
                }
            })
            .collect();
        // Leading aliases can span one early NOP removal. Keep only the original
        // labels and at most two NOPs needed to prove that normalization later.
        let mut entry_prefix = Some(Vec::new());
        let mut entry_nops = 0;
        for item in &items[begin..] {
            match item {
                Item::Label(_) => {}
                Item::Instruction(value) if value.opcode == NOP => {
                    entry_nops += 1;
                    if entry_nops > 2 {
                        entry_prefix = None;
                        break;
                    }
                }
                Item::Instruction(_) => break,
            }
            entry_prefix.as_mut().unwrap().push(*item);
        }
        self.exception_owner_provenance.source_none_conditions.push(
            exception_owners::SourceNoneCondition {
                entry,
                labels,
                instruction,
                entry_prefix,
            },
        );
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

    /// Retains the authored handler identity of an ordinary try's original SETUP.
    pub(crate) fn record_ordinary_try_exception_setup(
        &mut self,
        label: Label,
        retained_nop: bool,
        handler: Label,
    ) {
        self.record_exception_setup(label, retained_nop);
        self.exception_owner_provenance
            .ordinary_try_setups
            .insert(label, handler);
    }

    /// Records a completed ordinary await separately from async-manager protocol awaits.
    pub(crate) fn record_ordinary_await_cleanup(&mut self, yielded: Label, cleanup: Label) {
        let mut regions = self
            .exception_regions
            .iter()
            .filter(|region| region.start == yielded && region.target == cleanup);
        let Some(&region) = regions.next() else {
            return;
        };
        if regions.next().is_some()
            || region.is_scope_fallback
            || region.preserve_lasti
            || !matches!(region.priority, ExceptionRegionPriority::OriginalRange(start, end)
                if start == region.start && end == region.end)
        {
            return;
        }
        self.exception_owner_provenance
            .ordinary_await_cleanups
            .push(region);

        let [.., Item::Label(entry), Item::Instruction(end_send)] = self.linear_items() else {
            return;
        };
        if end_send.opcode != END_SEND {
            return;
        }
        let (entry, end_send) = (*entry, *end_send);
        let mut incoming = self.linear_items().iter().filter_map(|item| match item {
            Item::Instruction(instruction)
                if instruction.opcode == SEND
                    && instruction.operand() == Operand::Forward(entry) =>
            {
                Some(*instruction)
            }
            _ => None,
        });
        let Some(send) = incoming.next() else {
            return;
        };
        if incoming.next().is_some() {
            return;
        }
        let items = self.linear_items();
        let Some(cleanup_start) = items.iter().rposition(|item| *item == Item::Label(cleanup))
        else {
            return;
        };
        let original_cleanup = items[cleanup_start..items.len() - 2].to_vec();
        self.exception_owner_provenance.ordinary_await_ends.push(
            exception_owners::OrdinaryAwaitEnd {
                send,
                entry,
                end_send,
                cleanup: original_cleanup,
                bridge: None,
            },
        );
    }

    /// Authenticates the original `SEND`/`END_SEND` array before an async manager's setup.
    pub(crate) fn record_async_with_exception_setup(&mut self, label: Label) {
        self.record_exception_setup(label, false);
        let items = self.linear_items();
        let Some(Item::Instruction(end_send)) = items.last() else {
            return;
        };
        if end_send.opcode != END_SEND {
            return;
        }
        let begin = items[..items.len() - 1]
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .map_or(0, |index| index + 1);
        let prefix = &items[begin..];
        let Some(cleanup_end) = begin.checked_sub(1) else {
            return;
        };
        let Item::Instruction(cleanup_throw) = items[cleanup_end] else {
            return;
        };
        if cleanup_throw.opcode != CLEANUP_THROW {
            return;
        }
        let cleanup_begin = items[..cleanup_end]
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .map_or(0, |index| index + 1);
        let cleanup = &items[cleanup_begin..begin];
        if !self.exception_regions.iter().any(|region| {
            cleanup.contains(&Item::Label(region.target))
                && !prefix.contains(&Item::Label(region.target))
        }) {
            return;
        }
        let mut incoming = items.iter().filter_map(|item| match item {
            Item::Instruction(instruction)
                if instruction.opcode == SEND
                    && matches!(instruction.operand(), Operand::Forward(entry)
                        if prefix.contains(&Item::Label(entry))
                            && self.exception_owner_provenance.block_boundaries.contains(&entry)) =>
            {
                Some(*instruction)
            }
            _ => None,
        });
        let Some(send) = incoming.next() else {
            return;
        };
        if incoming.next().is_some() {
            return;
        }
        let origin = exception_owners::AsyncWithSetup {
            setup: label,
            send,
            prefix: prefix.to_vec(),
            cleanup: cleanup.to_vec(),
            bridge: None,
        };
        self.exception_owner_provenance
            .async_with_setups
            .push(origin);
    }

    /// Records a synchronous manager setup whose protected suffix has one lexical owner.
    pub(crate) fn record_synchronous_with_exception_setup(&mut self, label: Label) {
        self.record_exception_setup(label, false);
        self.exception_owner_provenance
            .synchronous_with_setups
            .insert(label);
    }

    /// Records a branch slot whose exclusion predicts a later normal-cleanup copy.
    pub(crate) fn record_normal_try_jump_exclusion(&mut self, start: Label, end: Label) {
        self.exception_owner_provenance
            .normal_try_jump_owners
            .entry((start, end))
            .or_default();
    }

    /// Binds a predicted jump exclusion to its exact original protected owner.
    pub(crate) fn record_normal_try_jump_owner(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
        depth: u32,
        preserve_lasti: bool,
        origin: (Label, Label),
    ) {
        if let Some(owners) = self
            .exception_owner_provenance
            .normal_try_jump_owners
            .get_mut(&(start, end))
        {
            owners.push(ExceptionRegion {
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(origin.0, origin.1),
                start,
                end,
                target,
                depth,
                preserve_lasti,
            });
        }
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
            // This patch resolves one uniquely attached source operand. Carry an
            // unchanged tuple-fold witness across that resolution, but never revive
            // a witness retired by an intervening mutation or physical copy.
            let original_uniform_fold = if instruction.opcode == LOAD_CONST
                && instruction.jump_policy.has_original_uniform_fold()
            {
                let mut originals = self
                    .exception_owner_provenance
                    .original_uniform_folds
                    .iter()
                    .enumerate()
                    .filter(|(_, original)| **original == *instruction);
                let original = originals.next().map(|(index, _)| index);
                original.filter(|_| originals.next().is_none())
            } else {
                None
            };
            instruction.set_operand(Operand::Value(argument));
            if let Some(index) = original_uniform_fold {
                instruction.jump_policy.mark_original_uniform_fold();
                self.exception_owner_provenance.original_uniform_folds[index] = *instruction;
            }
        }

        self.operand_patches.clear();
        Ok(())
    }

    pub(crate) fn used_constant_indices(
        &mut self,
        load_const: Opcode,
        constant_count: usize,
    ) -> Vec<bool> {
        self.resolve_folded_condition_borrow_entries();
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
            && self.borrow_omitted_for_backedges.is_empty()
            && self.folded_loop_exit_anchors.is_empty()
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
        // Retain the pre-remap witnesses throughout the pass so duplicate live
        // survivors stay duplicates when their constant indices change together.
        let original_uniform_folds = &self.exception_owner_provenance.original_uniform_folds;
        let remap = |instruction: &mut Instruction| {
            if instruction.opcode != load_const {
                return;
            }
            let Operand::Value(mut index) = instruction.operand() else {
                unreachable!("LOAD_CONST has a jump operand");
            };
            let mut original_uniform_fold = false;
            if index == u32::MAX {
                let old_index = usize::try_from(
                    deferred_none.expect("deferred None constant should be resolved"),
                )
                .expect("constant index should fit in usize");
                index = index_map[old_index].expect("deferred None constant should be retained");
            } else if let Some(new_index) = index_map[usize::try_from(index).unwrap()] {
                original_uniform_fold = instruction.opcode == LOAD_CONST
                    && instruction.jump_policy.has_original_uniform_fold()
                    && original_uniform_folds
                        .iter()
                        .filter(|original| **original == *instruction)
                        .count()
                        == 1;
                index = new_index;
            } else {
                // This unreachable load's constant was removed from the pool. Retain
                // its original depth and fold size until the block is discarded, but
                // do not leave an obsolete index for folded-admission bookkeeping.
                instruction.opcode = NOP;
                index = 0;
            }
            instruction.set_operand(Operand::Value(index));
            if original_uniform_fold {
                instruction.jump_policy.mark_original_uniform_fold();
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
            UnoptimizedCode::Prepared { graph, .. } => graph.for_each_instruction_mut(remap),
        }
        // retain_used moves the same constants without changing their identity.
        // Apply its mapping to every original witness only after all live records.
        for original in &mut self.exception_owner_provenance.original_uniform_folds {
            if original.opcode == LOAD_CONST
                && load_const == LOAD_CONST
                && original.jump_policy.has_original_uniform_fold()
                && let Operand::Value(index) = original.operand()
                && index != u32::MAX
                && let Some(Some(new_index)) = index_map.get(index as usize)
            {
                original.set_operand(Operand::Value(*new_index));
                original.jump_policy.mark_original_uniform_fold();
            }
        }
        // These source arrays were captured before unused constants were pruned.
        // Transport only surviving constant identities; replay still compares every
        // complete instruction and rejects any other change to the original array.
        if load_const == LOAD_CONST {
            self.exception_owner_provenance
                .copied_return_pop_prefixes
                .retain_mut(|origin| {
                    let load = &mut origin.records[2];
                    if load.opcode != LOAD_CONST {
                        return false;
                    }
                    let Operand::Value(index) = load.operand() else {
                        return false;
                    };
                    let Some(Some(remapped)) = index_map.get(index as usize) else {
                        return false;
                    };
                    load.set_operand(Operand::Value(*remapped));
                    true
                });
            self.exception_owner_provenance
                .original_normal_finally_arrays
                .retain_mut(|origin| {
                    origin.borrow_join = origin.borrow_join.and_then(|(join, mut first)| {
                        if first.opcode == LOAD_CONST {
                            let Operand::Value(index) = first.operand() else {
                                return None;
                            };
                            let remapped = (*index_map.get(index as usize)?)?;
                            first.set_operand(Operand::Value(remapped));
                        }
                        Some((join, first))
                    });
                    for item in &mut origin.prefix {
                        let Item::Instruction(instruction) = item else {
                            continue;
                        };
                        if instruction.opcode != LOAD_CONST {
                            continue;
                        }
                        let Operand::Value(index) = instruction.operand() else {
                            return false;
                        };
                        let Some(Some(remapped)) = index_map.get(index as usize) else {
                            return false;
                        };
                        instruction.set_operand(Operand::Value(*remapped));
                    }
                    true
                });
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
        self.resolve_folded_condition_borrow_entries();
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
