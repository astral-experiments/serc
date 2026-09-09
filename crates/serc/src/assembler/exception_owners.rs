use std::cmp::Reverse;

use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    Instruction, Item, Label, LayoutAssembler, NormalizedExceptionOwner, Operand, SourceLocation,
};
use crate::target::opcodes::{
    COMPARE_OP, COPY_FREE_VARS, FOR_ITER, LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_LOAD_FAST,
    LOAD_GLOBAL, MAKE_CELL, NOP, NOT_TAKEN, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, POP_TOP,
    RETURN_GENERATOR, SEND, STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST,
};
use crate::target::{is_conditional_jump, is_scope_exit};

/// Original CFG facts that cannot be recovered from split exception ranges or optimized jumps.
#[derive(Debug, Default)]
pub(super) struct ExceptionOwnerProvenance {
    pub(super) setups: Vec<(Label, Option<SourceLocation>)>,
    pub(super) block_boundaries: FxHashSet<Label>,
    pub(super) source_boundaries: FxHashSet<Label>,
    pub(super) continuation_boundaries: FxHashSet<Label>,
    pub(super) fusion_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_ranges: Vec<(Label, Label)>,
    pub(super) synchronous_with_setups: FxHashSet<Label>,
    pub(super) removed_pop_block_boundaries: FxHashSet<Label>,
    pub(super) removed_pop_block_widths: FxHashMap<Label, u16>,
    pub(super) protected_prefix_boundaries: FxHashSet<Label>,
    pub(super) noop_finally_locations: Vec<SourceLocation>,
    pub(super) with_context_locations: Vec<SourceLocation>,
    pub(super) uniform_fold_owner_slots: Vec<(SourceLocation, crate::target::Opcode, u16)>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ExceptionOwnerDecision {
    pub(super) has_owner: bool,
    // An earlier original-array record can have a different owner from the condition.
    pub(super) source_region: Option<usize>,
    pub(super) definitive: bool,
    pub(super) override_existing: bool,
}

/// The unused instruction-array suffix across CPython's three relevant assembler stages.
#[derive(Clone, Copy, Debug, Default)]
struct ExceptionOwnerTail {
    early_removals: usize,
    insertions: usize,
    late_removals: usize,
    // Local-load fusion compacts the live array again after the first peephole pass.
    post_compaction_fusions: usize,
}

impl ExceptionOwnerTail {
    fn has_owner_after_insertions(self, conditional_insertions: usize) -> bool {
        // COPY and TO_BOOL are inserted before SETUP instructions become removable NOPs.
        self.late_removals
            + self
                .early_removals
                .saturating_sub(self.insertions + conditional_insertions)
            > 0
    }

    fn instruction(&mut self, instruction: Instruction, previous: Option<Instruction>) {
        self.early_removals += usize::from(instruction.local_policy.hidden_exception_owner_slots());
        if matches!(
            instruction.opcode,
            LOAD_FAST_BORROW_LOAD_FAST_BORROW
                | LOAD_FAST_LOAD_FAST
                | STORE_FAST_LOAD_FAST
                | STORE_FAST_STORE_FAST
        ) {
            self.post_compaction_fusions += 1;
            self.early_removals += 1;
        } else if matches!((instruction.opcode, instruction.operand()), (LOAD_GLOBAL, Operand::Value(argument)) if argument & 1 != 0)
        {
            self.early_removals += 1;
        }
        // Boolean comparison specialization removes the original COMPARE_OP before
        // its TO_BOOL slot becomes the surviving instruction.
        if matches!((instruction.opcode, instruction.operand()),
            (COMPARE_OP, Operand::Value(argument)) if argument & 16 != 0)
        {
            self.early_removals += 1;
        }
        if matches!(
            instruction.opcode,
            COPY_FREE_VARS | MAKE_CELL | RETURN_GENERATOR
        ) || (instruction.opcode == POP_TOP
            && previous.is_some_and(|previous| previous.opcode == RETURN_GENERATOR))
        {
            self.insertions += 1;
        }
    }
}

impl LayoutAssembler {
    /// Identifies SETUP records that still occupy their authored one-instruction ranges.
    pub(super) fn retained_setup_nops(
        &self,
        label_positions: &FxHashMap<Label, usize>,
    ) -> FxHashSet<usize> {
        let setup_locations: Vec<_> = self
            .exception_owner_provenance
            .setups
            .iter()
            .filter_map(|(label, location)| Some((*label_positions.get(label)?, (*location)?)))
            .collect();
        let instructions: Vec<_> = self
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect();
        self.exception_owner_provenance
            .setup_nop_ranges
            .iter()
            .filter_map(|(start, end)| {
                let start = *label_positions.get(start)?;
                let end = *label_positions.get(end)?;
                let instruction = instructions.get(start)?;
                (end == start + 1
                    && instruction.opcode == NOP
                    && setup_locations.contains(&(end, instruction.location))
                    && instruction.local_policy.hidden_exception_owner_slots() == 0
                    && instruction.pre_fold_extra_instructions() == 0)
                    .then_some(start)
            })
            .collect()
    }

    /// Resolves inserted markers after fusion and complete SETUP provenance is available.
    pub(super) fn dynamic_exception_owners(&self) -> FxHashMap<usize, ExceptionOwnerDecision> {
        if self.exception_regions.is_empty() {
            return FxHashMap::default();
        }
        let has_retained_setup = self
            .exception_owner_provenance
            .setups
            .iter()
            .any(|(_, location)| location.is_some());
        if self
            .exception_owner_provenance
            .synchronous_with_setups
            .is_empty()
            && self.exception_owner_provenance.source_boundaries.is_empty()
            && !self.items.iter().any(|item| {
                let Item::Instruction(instruction) = item else {
                    return false;
                };
                matches!(
                    instruction.exception_policy().owner(),
                    NormalizedExceptionOwner::Dynamic
                        | NormalizedExceptionOwner::DynamicCondition
                        | NormalizedExceptionOwner::DynamicConditionStrict
                ) || (has_retained_setup
                    && instruction.opcode == NOT_TAKEN
                    && matches!(
                        instruction.exception_policy().owner(),
                        NormalizedExceptionOwner::Present | NormalizedExceptionOwner::Absent
                    ))
            })
        {
            return FxHashMap::default();
        }
        let setups: FxHashMap<_, _> = self
            .exception_owner_provenance
            .setups
            .iter()
            .copied()
            .collect();
        let mut label_positions = FxHashMap::default();
        let mut jump_targets = FxHashSet::default();
        let mut position = 0;
        for item in &self.items {
            match item {
                Item::Label(label) => {
                    label_positions.insert(*label, position);
                }
                Item::Instruction(instruction) => {
                    position += 1;
                    if let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                    {
                        jump_targets.insert(label);
                    }
                }
            }
        }
        let retained_setup_nops = self.retained_setup_nops(&label_positions);
        let unprotected_setups: FxHashSet<_> = setups
            .keys()
            .filter(|label| {
                let Some(position) = label_positions.get(label) else {
                    return false;
                };
                !self.exception_regions.iter().any(|region| {
                    label_positions
                        .get(&region.start)
                        .is_some_and(|start| start < position)
                        && label_positions
                            .get(&region.end)
                            .is_some_and(|end| position < end)
                })
            })
            .copied()
            .collect();
        let mut boundaries = self.exception_owner_provenance.block_boundaries.clone();
        boundaries.extend(&self.preserved_block_boundaries);
        boundaries.extend(self.exception_regions.iter().map(|region| region.target));
        let region_ends: FxHashSet<_> = self
            .exception_regions
            .iter()
            .map(|region| region.end)
            .collect();
        let mut owners = FxHashMap::default();
        let mut tail = ExceptionOwnerTail::default();
        let mut previous = None;
        let mut has_instruction = false;
        let mut source_block = false;
        let mut protected_suffix = None;
        let mut retained_owned_suffix = None;
        let mut ended_owned_prefix = None;
        let mut source_owned_prefix = None;
        let mut pending_retained_prefix = None;
        let mut owner_history_exact = true;
        let mut pop_slot_epoch_known = true;
        let mut retained_epoch_known = true;
        let mut uniform_with_suffix = None;
        let mut late_setup_positions = Vec::new();
        let mut late_setup_history_known = true;
        let mut ended_uniform_with_suffix = false;
        let mut ended_owned_suffix = None;
        let mut ended_pop_slot = None;
        let mut pending_null_append = None;
        let mut pending_fresh_null_append = None;
        let mut original_array_start = 0;
        let mut pending_compacted_append = None;
        let mut instruction_index = 0;
        for item in &self.items {
            match item {
                Item::Label(label) => {
                    let continues_original_block = self
                        .exception_owner_provenance
                        .continuation_boundaries
                        .contains(label)
                        && !boundaries.contains(label)
                        && !jump_targets.contains(label)
                        && !setups.contains_key(label)
                        && !self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label);
                    if boundaries.contains(label)
                        || jump_targets.contains(label)
                        || (!setups.contains_key(label)
                            && !self
                                .exception_owner_provenance
                                .setup_nop_boundaries
                                .contains(label))
                    {
                        pending_retained_prefix = None;
                    }
                    if boundaries.contains(label)
                        || jump_targets.contains(label)
                        || setups.contains_key(label)
                        || region_ends.contains(label)
                        || self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                    {
                        pending_null_append = None;
                        pending_fresh_null_append = None;
                    }
                    if boundaries.contains(label)
                        || jump_targets.contains(label)
                        || setups.contains_key(label)
                        || self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                        || (!region_ends.contains(label) && !continues_original_block)
                    {
                        pending_compacted_append = None;
                    }
                    if has_instruction && boundaries.contains(label) {
                        original_array_start = instruction_index;
                        tail = ExceptionOwnerTail::default();
                        late_setup_positions.clear();
                        late_setup_history_known = true;
                        previous = None;
                        has_instruction = false;
                        source_block = false;
                        protected_suffix = None;
                        retained_owned_suffix = None;
                        retained_epoch_known = true;
                        ended_owned_prefix = None;
                        source_owned_prefix = None;
                        owner_history_exact = true;
                        pop_slot_epoch_known = true;
                        uniform_with_suffix = None;
                        ended_owned_suffix = None;
                    }
                    if region_ends.contains(label)
                        && !continues_original_block
                        && !self
                            .exception_owner_provenance
                            .setup_nop_boundaries
                            .contains(label)
                    {
                        ended_owned_prefix = if self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                        {
                            retained_owned_suffix.or(source_owned_prefix).map(
                                |(start, setup_tail)| (start, instruction_index, setup_tail, tail),
                            )
                        } else {
                            None
                        };
                        if retained_epoch_known
                            && self
                                .exception_owner_provenance
                                .removed_pop_block_boundaries
                                .contains(label)
                        {
                            ended_owned_suffix = Some(instruction_index);
                            ended_pop_slot = pop_slot_epoch_known.then_some((
                                instruction_index,
                                tail,
                                self.exception_owner_provenance
                                    .removed_pop_block_widths
                                    .get(label)
                                    .copied()
                                    .unwrap_or(1),
                            ));
                        } else if !previous.is_some_and(|instruction: Instruction| {
                            is_conditional_jump(instruction.opcode)
                        }) {
                            ended_owned_suffix = None;
                        }
                        source_owned_prefix = None;
                        // An exception-range end can hide an original POP_BLOCK removed during
                        // compaction. Only an explicit source join restores the zero-slot proof.
                        // Exclusions around a retained SETUP NOP remove no original instruction.
                        source_block = false;
                        ended_uniform_with_suffix = true;
                        // A deleted authored POP_BLOCK preserves this original array's epoch
                        // when its slot was transferred without crossing a source or jump join.
                        if !self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                        {
                            retained_epoch_known = false;
                        }
                        if jump_targets.contains(label)
                            || !previous.is_some_and(|instruction: Instruction| {
                                is_conditional_jump(instruction.opcode)
                            })
                        {
                            retained_owned_suffix = None;
                        }
                    }
                    if has_instruction
                        && !boundaries.contains(label)
                        && !setups.contains_key(label)
                        && !region_ends.contains(label)
                        && !continues_original_block
                        && !self
                            .exception_owner_provenance
                            .setup_nop_boundaries
                            .contains(label)
                    {
                        retained_epoch_known = false;
                        // Labels that alias the certified POP_BLOCK end add no records.
                        // A later unknown label or a real jump join ends the proof.
                        if ended_owned_suffix != Some(instruction_index)
                            || jump_targets.contains(label)
                        {
                            ended_owned_suffix = None;
                        }
                        retained_owned_suffix = None;
                        source_owned_prefix = None;
                        if (ended_owned_prefix
                            .is_none_or(|(_, end, _, _)| end != instruction_index)
                            || jump_targets.contains(label))
                            && !previous.is_some_and(|instruction: Instruction| {
                                is_conditional_jump(instruction.opcode)
                            })
                        {
                            ended_owned_prefix = None;
                        }
                    }
                    if has_instruction
                        && !boundaries.contains(label)
                        && (jump_targets.contains(label)
                            || (!setups.contains_key(label)
                                && !continues_original_block
                                && !self
                                    .exception_owner_provenance
                                    .setup_nop_boundaries
                                    .contains(label)
                                && !previous.is_some_and(|instruction: Instruction| {
                                    is_conditional_jump(instruction.opcode)
                                })))
                    {
                        late_setup_history_known = false;
                    }
                    source_block |= self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(label);
                    if boundaries.contains(label)
                        && self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                    {
                        source_owned_prefix = Some((instruction_index, tail));
                    }
                    if owner_history_exact
                        && self
                            .exception_owner_provenance
                            .protected_prefix_boundaries
                            .contains(label)
                    {
                        source_owned_prefix = Some((instruction_index, tail));
                    }
                    if let Some(nop_location) = setups.get(label) {
                        ended_owned_suffix = None;
                        source_owned_prefix = None;
                        let retained = nop_location.is_some_and(|location| {
                            previous.is_some_and(|previous: Instruction| {
                                previous.opcode == NOP && previous.location == location
                            })
                        });
                        // A retained SETUP keeps its NULL record between the old prefix
                        // and the new protected suffix. Preserve only the exact prefix
                        // captured immediately before that authored NOP.
                        ended_owned_prefix = pending_retained_prefix
                            .take()
                            .filter(|(_, end, _, _)| retained && *end + 1 == instruction_index);
                        if !retained {
                            late_setup_positions
                                .push(instruction_index + late_setup_positions.len());
                            late_setup_history_known &= self
                                .exception_owner_provenance
                                .synchronous_with_setups
                                .contains(label)
                                || self.exception_regions.iter().any(|region| {
                                    region.is_scope_fallback && region.start == *label
                                });
                        }
                        tail.late_removals += usize::from(!retained);
                        uniform_with_suffix = (!retained
                            && self
                                .exception_owner_provenance
                                .synchronous_with_setups
                                .contains(label))
                        .then_some(instruction_index);
                        ended_uniform_with_suffix = false;
                        protected_suffix = (retained && unprotected_setups.contains(label))
                            .then_some((instruction_index, tail, *label));
                        retained_owned_suffix =
                            (retained && retained_epoch_known).then_some((instruction_index, tail));
                    }
                }
                Item::Instruction(instruction) => {
                    let compacted_append = pending_compacted_append.take();
                    pending_retained_prefix = (instruction.opcode == NOP
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                        && instruction.pre_fold_extra_instructions() == 0
                        && owner_history_exact)
                        .then_some(source_owned_prefix)
                        .flatten()
                        .map(|(start, start_tail)| (start, instruction_index, start_tail, tail));
                    // A NOP can stand for a folded source branch whose original join
                    // starts a different instruction array. Without its source identity,
                    // earlier slot counts cannot locate a later POP_BLOCK exactly.
                    if instruction.opcode == NOP
                        || instruction.no_location_policy.borrow_fallthrough()
                            == super::BorrowFallthrough::Barrier
                    {
                        late_setup_history_known = false;
                        // An exact retained SETUP NOP is a known live array record.
                        // It preserves an existing POP_BLOCK epoch without reviving
                        // one lost at an unknown source join or borrow barrier.
                        pop_slot_epoch_known &= retained_setup_nops.contains(&instruction_index)
                            && instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Barrier;
                        ended_pop_slot = None;
                        pending_null_append = None;
                        pending_fresh_null_append = None;
                    }
                    let appends_to_null_slot =
                        pending_null_append.take() == Some(instruction_index);
                    let appends_to_fresh_null_slot =
                        pending_fresh_null_append.take() == Some(instruction_index);
                    if ended_uniform_with_suffix && instruction.opcode != NOT_TAKEN {
                        if ended_owned_suffix != Some(instruction_index) {
                            ended_owned_suffix = None;
                        }
                        uniform_with_suffix = None;
                        retained_owned_suffix = None;
                    }
                    ended_uniform_with_suffix = false;
                    if instruction.opcode != NOT_TAKEN
                        && (instruction.exception_policy().exclusion()
                            != super::ExceptionExclusion::None
                            || instruction.exception_policy().owner()
                                != NormalizedExceptionOwner::Unchanged)
                    {
                        ended_owned_suffix = None;
                    }
                    if ended_owned_suffix == Some(instruction_index) {
                        // Every transferred record must be an authored POP_BLOCK. A
                        // folded source NOP or saturated count cannot prove a NULL span.
                        ended_pop_slot = ended_pop_slot.filter(|(start, _, width)| {
                            *start == instruction_index
                                && instruction.local_policy.hidden_exception_owner_slots() == *width
                        });
                    }
                    tail.instruction(*instruction, previous);
                    let uniform_fold = instruction.local_policy.hidden_exception_owner_slots() > 0
                        && self
                            .exception_owner_provenance
                            .uniform_fold_owner_slots
                            .iter()
                            .any(|(location, opcode, slots)| {
                                instruction.location == *location
                                    && instruction.opcode == *opcode
                                    && instruction.local_policy.hidden_exception_owner_slots()
                                        == *slots
                                    && instruction.pre_fold_extra_instructions()
                                        == (*slots).min(4) as u8
                            });
                    if instruction.local_policy.hidden_exception_owner_slots()
                        == super::LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS
                        || (instruction.pre_fold_extra_instructions() > 0 && !uniform_fold)
                    {
                        owner_history_exact = false;
                    }
                    if instruction.local_policy.hidden_exception_owner_slots()
                        == super::LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS
                        || (instruction.pre_fold_extra_instructions() > 0 && !uniform_fold)
                        || instruction.exception_policy().exclusion()
                            != super::ExceptionExclusion::None
                    {
                        ended_owned_prefix = None;
                    }
                    if (instruction.local_policy.hidden_exception_owner_slots() > 0
                        || instruction.pre_fold_extra_instructions() > 0)
                        && !uniform_fold
                        || instruction.exception_policy().exclusion()
                            != super::ExceptionExclusion::None
                    {
                        retained_owned_suffix = None;
                        source_owned_prefix = None;
                    }
                    if matches!(
                        instruction.exception_policy().owner(),
                        NormalizedExceptionOwner::Dynamic
                            | NormalizedExceptionOwner::DynamicCondition
                            | NormalizedExceptionOwner::DynamicConditionStrict
                    ) {
                        let insertions = if instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Dynamic
                        {
                            2
                        } else {
                            0
                        };
                        let mut has_owner = tail.has_owner_after_insertions(insertions);
                        let mut proven_unprotected_slot = false;
                        if let Some((start, setup_tail, setup_label)) = protected_suffix
                            && tail.late_removals == 0
                        {
                            // Compaction leaves the original tail records untouched. If the
                            // reused slot precedes this SETUP's protected suffix, its owner is
                            // still NULL even though the block has unused array capacity.
                            let protected_original_instructions = (instruction_index - start
                                + tail.early_removals
                                - setup_tail.early_removals)
                                .saturating_sub(
                                    insertions + tail.insertions - setup_tail.insertions,
                                );
                            let stale_slots = tail
                                .early_removals
                                .saturating_sub(tail.insertions + insertions);
                            has_owner &= stale_slots <= protected_original_instructions;
                            // Exact early compaction can reach the unprotected records before
                            // the first SETUP. Exclude any earlier protected origin, including an
                            // empty region removed after its original SETUP was recorded.
                            proven_unprotected_slot = self
                                .exception_owner_provenance
                                .setups
                                .first()
                                .is_some_and(|(first, _)| *first == setup_label)
                                && owner_history_exact
                                && tail.post_compaction_fusions == 0
                                && stale_slots > protected_original_instructions
                                && !self.exception_regions.iter().any(|region| {
                                    label_positions
                                        .get(&region.start)
                                        .is_some_and(|position| *position < start)
                                });
                        }
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner,
                                source_region: None,
                                override_existing: false,
                                definitive: proven_unprotected_slot
                                    || instruction.exception_policy().owner()
                                        != NormalizedExceptionOwner::DynamicCondition,
                            },
                        );
                    }
                    // The last compaction determines which stale array the marker reuses.
                    // Local fusion compacts the already optimized live array; otherwise,
                    // the first peephole pass leaves the original suffix untouched.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Present
                        && tail.late_removals == 0
                        && let Some((start, setup_tail, _)) = protected_suffix
                    {
                        let (removed, protected_before_compaction) =
                            if tail.post_compaction_fusions > 0 {
                                (
                                    tail.post_compaction_fusions,
                                    instruction_index - start + tail.post_compaction_fusions
                                        - setup_tail.post_compaction_fusions,
                                )
                            } else {
                                (
                                    tail.early_removals,
                                    instruction_index - start + tail.early_removals
                                        - setup_tail.early_removals,
                                )
                            };
                        // Prologue insertion consumes vacancies from every earlier compaction.
                        // Once none remain, the appended marker gets a fresh NULL owner.
                        let prologue_consumed_all_slots =
                            tail.insertions > 0 && tail.early_removals <= tail.insertions;
                        if prologue_consumed_all_slots
                            || (tail.insertions == 0 && removed > protected_before_compaction)
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision {
                                    has_owner: false,
                                    source_region: None,
                                    definitive: true,
                                    override_existing: false,
                                },
                            );
                        }
                    }
                    // A retained SETUP can follow removals owned by an outer handler. Only
                    // certify a reused inner owner when the last compaction lands entirely
                    // within a known, uniformly owned suffix; other outcomes remain unknown.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Absent
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && let Some((start, setup_tail)) = retained_owned_suffix
                    {
                        let (removed, suffix_before_compaction) =
                            if tail.post_compaction_fusions > 0 {
                                (
                                    tail.post_compaction_fusions,
                                    instruction_index - start + tail.post_compaction_fusions
                                        - setup_tail.post_compaction_fusions,
                                )
                            } else {
                                (
                                    tail.early_removals,
                                    instruction_index - start + tail.early_removals
                                        - setup_tail.early_removals,
                                )
                            };
                        if removed > 0
                            && removed <= suffix_before_compaction
                            && self.has_uniform_exception_owner(
                                start,
                                instruction_index,
                                &label_positions,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision {
                                    has_owner: true,
                                    source_region: None,
                                    definitive: true,
                                    override_existing: true,
                                },
                            );
                        }
                    }
                    // A late SETUP removal compacts the live array after prologue insertion.
                    // With L removed SETUPs and W surviving records in the final uniform
                    // manager-owned suffix, L <= W places the appended slot inside that
                    // suffix. Its owner is the preceding condition's manager even when an
                    // outer generator handler already structurally covers the marker.
                    if instruction.opcode == NOT_TAKEN
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && uniform_with_suffix.is_some_and(|start| {
                            tail.late_removals > 0
                                && tail.late_removals <= instruction_index - start
                        })
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: true,
                                source_region: None,
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    let reusable_suffix_slots = if tail.late_removals > 0 {
                        tail.late_removals
                    } else if tail.insertions == 0 && owner_history_exact {
                        if tail.post_compaction_fusions > 0 {
                            tail.post_compaction_fusions
                        } else {
                            tail.early_removals
                        }
                    } else {
                        0
                    };
                    let folded_none_suffix_slots = if tail.late_removals == 0
                        && tail.insertions == 0
                        && tail.post_compaction_fusions == 0
                        && previous.is_some_and(|previous: Instruction| {
                            matches!(previous.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                                && previous.local_policy.hidden_exception_owner_slots() >= 3
                        }) {
                        // LOAD_CONST(None), IS_OP, and TO_BOOL share the condition's owner.
                        // They still occupy its original suffix after early compaction.
                        3
                    } else {
                        0
                    };
                    // Compaction leaves the old array suffix untouched. A certified erased
                    // POP_BLOCK starts a uniform suffix in that same array. Its surviving
                    // width is a lower bound before compaction; a reused slot within that
                    // bound has this exact owner even if earlier records have other owners.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Unchanged
                                | NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && let Some(start) = ended_owned_suffix
                        && reusable_suffix_slots > 0
                        && reusable_suffix_slots
                            <= instruction_index - start + folded_none_suffix_slots
                        && let Some(region) =
                            self.uniform_exception_owner(start, instruction_index, &label_positions)
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: true,
                                source_region: Some(region),
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    if instruction.opcode == NOT_TAKEN
                        && source_block
                        && tail.early_removals == 0
                        && tail.late_removals == 0
                    {
                        // This conditional's original array begins at a source join and has
                        // no removed records. Appending its marker allocates a NULL owner.
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: false,
                                source_region: None,
                                definitive: true,
                                override_existing: false,
                            },
                        );
                    }
                    // Fusion leaves stale records in a uniformly owned original source
                    // array. A marker appended into that suffix inherits the same owner.
                    if instruction.opcode == NOT_TAKEN
                        && source_block
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions > 0
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && let Some((start, _)) = source_owned_prefix
                        && let Some(region) =
                            self.uniform_exception_owner(start, instruction_index, &label_positions)
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: true,
                                source_region: Some(region),
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    // A normal cleanup can share its original array with the preceding try.
                    // If compaction skips the entire cleanup suffix, the marker reuses a slot
                    // in that earlier protected prefix, rather than its current condition.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && owner_history_exact
                        && tail.late_removals == 0
                        && let Some((start, end, start_tail, end_tail)) = ended_owned_prefix
                        && (tail.insertions == 0
                            || (tail.post_compaction_fusions == 0
                                && start_tail.insertions == tail.insertions))
                    {
                        let (removed, suffix_width, prefix_width) = if tail.post_compaction_fusions
                            > 0
                        {
                            (
                                tail.post_compaction_fusions,
                                instruction_index - end + tail.post_compaction_fusions
                                    - end_tail.post_compaction_fusions,
                                end - start + end_tail.post_compaction_fusions
                                    - start_tail.post_compaction_fusions,
                            )
                        } else {
                            (
                                tail.early_removals,
                                instruction_index - end + tail.early_removals
                                    - end_tail.early_removals,
                                end - start + end_tail.early_removals - start_tail.early_removals,
                            )
                        };
                        // Prologue records inserted before this entire prefix shift only
                        // the live array. The appended slot is therefore later in the old
                        // tail by that insertion count. Once all vacancies are consumed,
                        // this compaction no longer supplies a certified owner slot.
                        let removed = removed.saturating_sub(tail.insertions);
                        if removed > suffix_width
                            && removed <= suffix_width + prefix_width
                            && let Some(region) =
                                self.uniform_exception_owner(start, end, &label_positions)
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision {
                                    has_owner: true,
                                    source_region: Some(region),
                                    definitive: true,
                                    override_existing: true,
                                },
                            );
                        }
                    }
                    // Certify the fresh append slot at the original terminator, before
                    // bookkeeping labels ahead of NOT_TAKEN obscure its array epoch.
                    // With no removals or insertions, the first retained SETUP's block
                    // appends into a zero-initialized slot beyond its original live end.
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && owner_history_exact
                        && retained_epoch_known
                        && instruction.no_location_policy.borrow_fallthrough()
                            != super::BorrowFallthrough::Barrier
                        && tail.early_removals == 0
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && let Some((start, _, setup_label)) = protected_suffix
                        && self
                            .exception_owner_provenance
                            .setups
                            .first()
                            .is_some_and(|(first, _)| *first == setup_label)
                        && !self.exception_regions.iter().any(|region| {
                            label_positions
                                .get(&region.start)
                                .is_some_and(|position| *position < start)
                        })
                    {
                        pending_fresh_null_append = Some(instruction_index + 1);
                    }
                    // If the appended slot falls within an exact erased POP_BLOCK run,
                    // it reuses a NULL record. Resolve the original terminator before
                    // cold-block labeling inserts a bookkeeping label ahead of NOT_TAKEN.
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && let Some((start, start_tail, width)) = ended_pop_slot
                        && ended_owned_suffix == Some(start)
                        && start_tail.early_removals <= instruction_index + 1 - start
                        && instruction_index + 1 - start
                            < start_tail.early_removals + usize::from(width)
                    {
                        pending_null_append = Some(instruction_index + 1);
                    }
                    if instruction.opcode == NOT_TAKEN
                        && (appends_to_null_slot
                            || (appends_to_fresh_null_slot
                                && instruction.exception_policy().owner()
                                    == NormalizedExceptionOwner::DynamicCondition))
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: false,
                                source_region: None,
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    // Reinsert certified late SETUP records into this original array.
                    // The appended marker reuses its pre-compaction physical slot, which
                    // can be an erased NULL SETUP or an earlier manager-entry instruction.
                    if instruction.opcode == NOT_TAKEN
                        && late_setup_history_known
                        && owner_history_exact
                        && late_setup_positions.len() == tail.late_removals
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && uniform_with_suffix
                            .is_some_and(|start| tail.late_removals > instruction_index - start)
                    {
                        let removed_before = late_setup_positions
                            .partition_point(|position| *position < instruction_index);
                        let source_region = if late_setup_positions.get(removed_before)
                            == Some(&instruction_index)
                        {
                            None
                        } else {
                            let source = instruction_index - removed_before;
                            self.uniform_exception_owner(source, source + 1, &label_positions)
                        };
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: source_region.is_some(),
                                source_region,
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    // Capture the original array's owner before range-exclusion labels at the
                    // conditional erase its provenance. A compacted, uniformly owned array
                    // leaves the same owner in every stale slot reused by a forward marker.
                    // An implicit generator fallback is not an authored original-slot owner.
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && owner_history_exact
                        && retained_epoch_known
                        && pop_slot_epoch_known
                        && tail.early_removals > 0
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && let Some(region) = self.uniform_exception_owner(
                            original_array_start,
                            instruction_index + 1,
                            &label_positions,
                        )
                        && !self.exception_regions[region].is_scope_fallback
                    {
                        pending_compacted_append = Some((instruction_index + 1, region));
                    }
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && instruction.exception_policy().owner()
                            != NormalizedExceptionOwner::Dynamic
                        && let Some((index, region)) = compacted_append
                        && index == instruction_index
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|owner| !owner.definitive || owner.has_owner)
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: true,
                                source_region: Some(region),
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    instruction_index += 1;
                    has_instruction = true;
                    if matches!(instruction.opcode, FOR_ITER | SEND) {
                        late_setup_history_known = false;
                        source_block = false;
                        protected_suffix = None;
                        uniform_with_suffix = None;
                        ended_owned_suffix = None;
                        retained_owned_suffix = None;
                        ended_owned_prefix = None;
                        source_owned_prefix = None;
                    }
                    if instruction.opcode == NOT_TAKEN
                        || (instruction.opcode != RETURN_GENERATOR
                            && !is_conditional_jump(instruction.opcode)
                            && (!matches!(instruction.operand(), Operand::Value(_))
                                || is_scope_exit(instruction.opcode)))
                    {
                        original_array_start = instruction_index;
                        tail = ExceptionOwnerTail::default();
                        late_setup_positions.clear();
                        late_setup_history_known = true;
                        previous = None;
                        has_instruction = false;
                        source_block = false;
                        protected_suffix = None;
                        uniform_with_suffix = None;
                        ended_owned_suffix = None;
                        retained_owned_suffix = None;
                        retained_epoch_known = true;
                        ended_owned_prefix = None;
                        source_owned_prefix = None;
                        owner_history_exact = true;
                        pop_slot_epoch_known = true;
                    } else {
                        previous = Some(*instruction);
                    }
                }
            }
        }
        owners
    }

    /// Proves that every surviving record in a suffix has the preceding condition's owner.
    fn has_uniform_exception_owner(
        &self,
        start: usize,
        end: usize,
        positions: &FxHashMap<Label, usize>,
    ) -> bool {
        self.uniform_exception_owner(start, end, positions)
            .is_some()
    }

    fn uniform_exception_owner(
        &self,
        start: usize,
        end: usize,
        positions: &FxHashMap<Label, usize>,
    ) -> Option<usize> {
        if start >= end {
            return None;
        }
        let priority = |index, region: &super::ExceptionRegion| {
            let (start, end) = match region.priority {
                super::ExceptionRegionPriority::OriginalRange(start, end) => {
                    (positions[&start], positions[&end])
                }
                super::ExceptionRegionPriority::Resolved { start, end } => (start, end),
            };
            (region.is_scope_fallback, Reverse(start), end, index)
        };
        let (owner_index, owner) = self
            .exception_regions
            .iter()
            .enumerate()
            .filter(|(_, region)| positions[&region.start] < end && positions[&region.end] >= end)
            .min_by_key(|(index, region)| priority(*index, region))?;
        if positions[&owner.start] > start {
            return None;
        }
        let owner_priority = priority(owner_index, owner);
        (!self
            .exception_regions
            .iter()
            .enumerate()
            .any(|(index, region)| {
                positions[&region.start] < end
                    && positions[&region.end] > start
                    && priority(index, region) < owner_priority
                    && (region.target != owner.target
                        || region.depth != owner.depth
                        || region.preserve_lasti != owner.preserve_lasti)
            }))
        .then_some(owner_index)
    }
}

#[cfg(test)]
mod tests {
    use super::ExceptionOwnerTail;

    #[test]
    fn setup_removal_follows_boolean_and_prologue_insertions() {
        assert!(
            !ExceptionOwnerTail {
                early_removals: 2,
                ..ExceptionOwnerTail::default()
            }
            .has_owner_after_insertions(2)
        );
        assert!(
            ExceptionOwnerTail {
                early_removals: 3,
                ..ExceptionOwnerTail::default()
            }
            .has_owner_after_insertions(2)
        );
        assert!(
            !ExceptionOwnerTail {
                early_removals: 3,
                insertions: 1,
                late_removals: 0,
                ..ExceptionOwnerTail::default()
            }
            .has_owner_after_insertions(2)
        );
        assert!(
            ExceptionOwnerTail {
                early_removals: 0,
                insertions: 3,
                late_removals: 1,
                ..ExceptionOwnerTail::default()
            }
            .has_owner_after_insertions(2)
        );
    }

    #[test]
    fn fresh_null_append_requires_first_unobstructed_protected_block() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{
            BorrowFallthrough, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
            LayoutAssembler, NormalizedExceptionOwner, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, RETURN_VALUE,
        };

        let setup = Label(0);
        let end = Label(1);
        let exit = Label(2);
        let bookkeeping = Label(3);
        let handler = Label(4);
        let location = SourceLocation::new(2, 2, 0, 4);
        let plain =
            |opcode, operand| Item::Instruction(Instruction::new(opcode, operand, location, None));
        for (barrier, outer_region) in [(false, false), (true, false), (false, true)] {
            let mut conditional =
                Instruction::new(POP_JUMP_IF_FALSE, Operand::Forward(exit), location, None);
            if barrier {
                conditional
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier);
            }
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, None);
            marker.set_exception_owner(NormalizedExceptionOwner::DynamicCondition);
            let mut assembler = LayoutAssembler {
                items: vec![
                    plain(NOP, Operand::Value(0)),
                    Item::Label(setup),
                    plain(LOAD_FAST, Operand::Value(0)),
                    Item::Instruction(conditional),
                    Item::Label(bookkeeping),
                    Item::Instruction(marker),
                    Item::Label(end),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                    Item::Label(exit),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                    Item::Label(handler),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                ],
                exception_regions: vec![ExceptionRegion {
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(setup, end),
                    start: setup,
                    end,
                    target: handler,
                    depth: 0,
                    preserve_lasti: false,
                }],
                exception_owner_provenance: ExceptionOwnerProvenance {
                    setups: vec![(setup, Some(location))],
                    ..ExceptionOwnerProvenance::default()
                },
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 5,
                has_next_location: false,
            };
            if outer_region {
                // An earlier outer handler invalidates the first unprotected array
                // origin even though all compaction and insertion counts are zero.
                let outer_start = Label(5);
                assembler.items.insert(0, Item::Label(outer_start));
                assembler.exception_regions.push(ExceptionRegion {
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(outer_start, end),
                    start: outer_start,
                    end,
                    target: handler,
                    depth: 0,
                    preserve_lasti: false,
                });
                assembler.next_label = 6;
            }
            let decisions = assembler.dynamic_exception_owners();
            let decision = decisions.get(&3).expect("dynamic marker decision");
            assert!(!decision.has_owner);
            assert_eq!(decision.definitive, !barrier && !outer_region);
            assert_eq!(decision.override_existing, !barrier && !outer_region);
        }
    }
}
