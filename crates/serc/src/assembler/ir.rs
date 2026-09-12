use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU64, Ordering};

use rustc_hash::{FxHashMap, FxHashSet};

use super::block_graph::BlockGraph;
use crate::target::Opcode;

pub(crate) struct AssembledCode {
    pub(crate) bytecode: Vec<u8>,
    pub(crate) line_table: Vec<u8>,
    pub(crate) exception_table: Vec<u8>,
    pub(crate) max_depth: u32,
    pub(crate) removed_max_depth: Option<u32>,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug)]
pub(super) enum AssemblerStage {
    RemoveUnreachableInitial,
    OptimizeBooleanConversions,
    ThreadForwardJumps,
    RemoveUnreachableAfterThreading,
    RemoveRedundantForwardJumpsEarly,
    OptimizeRedundantStoreFast,
    OptimizeSwapRuns,
    ApplyStaticSwaps,
    DuplicateExitBlocks,
    AddChecksForUninitializedLoads,
    FuseSuperinstructions,
    PushColdBlocksToEnd,
    RemoveRedundantCheckedLoads,
    PropagateLocationsWithinBlocks,
    RemoveRedundantSwapsBeforePops,
    RemoveRedundantNops,
    RemoveRedundantForwardJumpsLate,
    OptimizeLoadFast,
    FinalLayout,
}

#[cfg(any(test, debug_assertions))]
impl AssemblerStage {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::RemoveUnreachableInitial => "remove_unreachable_instructions (initial)",
            Self::OptimizeBooleanConversions => "optimize_boolean_conversions",
            Self::ThreadForwardJumps => "thread_forward_jumps",
            Self::RemoveUnreachableAfterThreading => {
                "remove_unreachable_instructions (after threading)"
            }
            Self::RemoveRedundantForwardJumpsEarly => "remove_redundant_forward_jumps (early)",
            Self::OptimizeRedundantStoreFast => "optimize_redundant_store_fast",
            Self::OptimizeSwapRuns => "optimize_swap_runs",
            Self::ApplyStaticSwaps => "apply_static_swaps",
            Self::DuplicateExitBlocks => "duplicate_exit_blocks",
            Self::AddChecksForUninitializedLoads => "add_checks_for_uninitialized_loads",
            Self::FuseSuperinstructions => "fuse_superinstructions",
            Self::PushColdBlocksToEnd => "push_cold_blocks_to_end",
            Self::RemoveRedundantCheckedLoads => "remove_redundant_checked_loads",
            Self::PropagateLocationsWithinBlocks => "propagate_locations_within_blocks",
            Self::RemoveRedundantSwapsBeforePops => "remove_redundant_swaps_before_pops",
            Self::RemoveRedundantNops => "remove_redundant_nops",
            Self::RemoveRedundantForwardJumpsLate => "remove_redundant_forward_jumps (late)",
            Self::OptimizeLoadFast => "optimize_load_fast",
            Self::FinalLayout => "final jump layout",
        }
    }

    pub(super) const fn requires_full_reachability(self) -> bool {
        matches!(
            self,
            Self::RemoveUnreachableInitial | Self::RemoveUnreachableAfterThreading
        )
    }

    pub(super) const fn requires_ordered_exception_regions(self) -> bool {
        matches!(self, Self::FinalLayout)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Label(pub(super) u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OperandPatchId(pub(super) u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EmissionStreamId(u64);

impl EmissionStreamId {
    pub(super) fn next() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(id, 0, "assembler emission stream IDs exhausted");
        Self(id)
    }
}

/// Identifies an instruction while compiler emission is still in progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EmittedInstruction {
    pub(super) stream: EmissionStreamId,
    pub(super) item_index: u32,
}

/// Identifies an emitted jump without allowing ordinary instructions at call sites.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EmittedJump(pub(super) EmittedInstruction);

impl EmittedJump {
    pub(crate) const fn instruction(self) -> EmittedInstruction {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Operand {
    Value(u32),
    Deferred(OperandPatchId),
    Forward(Label),
    Backward(Label),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct OperandPatch {
    pub(super) item_index: u32,
    pub(super) argument: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LocalPolicy(u16);

impl LocalPolicy {
    const FORCE_OWNED_LOAD: u16 = 1 << 0;
    const STRICT_OWNED_LOAD: u16 = 1 << 1;
    const MAYBE_NULL_COMPREHENSION_RESTORE: u16 = 1 << 2;
    const FORCE_BORROWED_LOAD: u16 = 1 << 3;
    const POST_ANALYSIS_CLOSURE_LOAD: u16 = 1 << 4;
    const KNOWN_INITIALIZED_LOAD: u16 = 1 << 5;
    // Long unprotected prefixes can move a reused owner slot before the protected suffix.
    pub(super) const MAX_HIDDEN_EXCEPTION_OWNER_SLOTS: u16 = u16::MAX >> 6;
    const HIDDEN_EXCEPTION_OWNER_SLOTS_SHIFT: u8 = 6;
    const HIDDEN_EXCEPTION_OWNER_SLOTS_MASK: u16 =
        Self::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS << Self::HIDDEN_EXCEPTION_OWNER_SLOTS_SHIFT;

    pub(super) const fn for_load(force_owned: bool, strict_owned: bool) -> Self {
        let mut policy = 0;
        if force_owned {
            policy |= Self::FORCE_OWNED_LOAD;
        }
        if strict_owned {
            policy |= Self::STRICT_OWNED_LOAD;
        }
        Self(policy)
    }

    pub(super) const fn force_owned_load(self) -> bool {
        self.0 & Self::FORCE_OWNED_LOAD != 0
    }

    pub(super) const fn strict_owned_load(self) -> bool {
        self.0 & Self::STRICT_OWNED_LOAD != 0
    }

    pub(super) const fn force_borrowed_load(self) -> bool {
        self.0 & Self::FORCE_BORROWED_LOAD != 0
    }

    pub(super) fn mark_force_borrowed_load(&mut self) {
        self.0 |= Self::FORCE_BORROWED_LOAD;
    }

    pub(super) const fn may_store_null(self) -> bool {
        self.0 & Self::MAYBE_NULL_COMPREHENSION_RESTORE != 0
    }

    pub(super) fn mark_maybe_null_comprehension_restore(&mut self) {
        self.0 |= Self::MAYBE_NULL_COMPREHENSION_RESTORE;
    }

    pub(super) const fn is_post_analysis_closure_load(self) -> bool {
        self.0 & Self::POST_ANALYSIS_CLOSURE_LOAD != 0
    }

    pub(super) fn mark_post_analysis_closure_load(&mut self) {
        self.0 |= Self::POST_ANALYSIS_CLOSURE_LOAD;
    }

    pub(super) const fn is_known_initialized_load(self) -> bool {
        self.0 & Self::KNOWN_INITIALIZED_LOAD != 0
    }

    pub(super) fn mark_known_initialized_load(&mut self) {
        self.0 |= Self::KNOWN_INITIALIZED_LOAD;
    }

    pub(super) const fn hidden_exception_owner_slots(self) -> u16 {
        (self.0 & Self::HIDDEN_EXCEPTION_OWNER_SLOTS_MASK)
            >> Self::HIDDEN_EXCEPTION_OWNER_SLOTS_SHIFT
    }

    pub(super) fn add_hidden_exception_owner_slots(&mut self, slots: u16) {
        let slots = self
            .hidden_exception_owner_slots()
            .saturating_add(slots)
            .min(Self::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS);
        self.0 = (self.0 & !Self::HIDDEN_EXCEPTION_OWNER_SLOTS_MASK)
            | (slots << Self::HIDDEN_EXCEPTION_OWNER_SLOTS_SHIFT);
    }

    const fn fused(first: Self, second: Self) -> Self {
        let slots = first
            .hidden_exception_owner_slots()
            .saturating_add(second.hidden_exception_owner_slots());
        let slots = if slots > Self::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS {
            Self::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS
        } else {
            slots
        };
        Self(
            (first.0 | second.0) & !Self::HIDDEN_EXCEPTION_OWNER_SLOTS_MASK
                | (slots << Self::HIDDEN_EXCEPTION_OWNER_SLOTS_SHIFT),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct NoLocationBlockPolicy(u8);

impl NoLocationBlockPolicy {
    const ALLOW_INLINING: u8 = 1 << 0;
    const FIRST_COPY_INTERRUPTIBLE: u8 = 1 << 1;
    const ARTIFICIAL_EVAL_BREAK: u8 = 1 << 2;
    const COPY_INHERITS_SHARED_PREDECESSOR: u8 = 1 << 3;
    const COPY_SPLIT_RETURN_EXIT: u8 = 1 << 4;
    const PRESERVE: u8 = 1 << 5;
    // Borrowed-load traversal is inherited from the second fused item.
    const BORROW_FALLTHROUGH_MASK: u8 = 0b11 << 6;
    const BORROW_BARRIER: u8 = 0b01 << 6;
    const BORROW_RETAINED_NOP_REACHABLE: u8 = 0b10 << 6;
    const BORROW_BARRIER_IF_REMOVED: u8 = 0b11 << 6;

    const fn fused(first: Self, second: Self) -> Self {
        Self(
            (first.0 | second.0) & !Self::BORROW_FALLTHROUGH_MASK
                | second.0 & Self::BORROW_FALLTHROUGH_MASK,
        )
    }

    const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    fn insert(&mut self, flag: u8) {
        self.0 |= flag;
    }

    fn remove(&mut self, flag: u8) {
        self.0 &= !flag;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NoLocationPolicy {
    /// Whether an unlocated block ending at this instruction may be inlined.
    block_policy: NoLocationBlockPolicy,
    /// An unresolved percent target or its deferred-location side-table index.
    pub(super) percent_target_location: u8,
}

impl NoLocationPolicy {
    pub(super) const fn allows_block_inlining(self) -> bool {
        self.block_policy
            .contains(NoLocationBlockPolicy::ALLOW_INLINING)
    }

    /// Whether a copied forward exit becomes an interruptible backward jump after cold-block
    /// ordering. The original cleanup edge remains non-interruptible.
    pub(super) const fn first_copy_is_interruptible_backward(self) -> bool {
        self.block_policy
            .contains(NoLocationBlockPolicy::FIRST_COPY_INTERRUPTIBLE)
    }

    /// Whether this eval-break instruction was emitted with CPython's `NO_LOCATION`.
    pub(super) const fn is_artificial_eval_break(self) -> bool {
        self.block_policy
            .contains(NoLocationBlockPolicy::ARTIFICIAL_EVAL_BREAK)
    }

    /// Whether a copied no-location block inherits a source location even when its predecessor
    /// is itself the join of multiple incoming edges.
    pub(super) const fn copy_inherits_shared_predecessor(self) -> bool {
        self.block_policy
            .contains(NoLocationBlockPolicy::COPY_INHERITS_SHARED_PREDECESSOR)
    }

    /// Whether a direct normal-exit jump may copy a return split by an exception-region boundary
    /// without retaining the source jump as a NOP.
    pub(super) const fn copies_split_return_exit(self) -> bool {
        self.block_policy
            .contains(NoLocationBlockPolicy::COPY_SPLIT_RETURN_EXIT)
    }
    pub(super) fn allow_block_inlining(&mut self) {
        self.block_policy
            .insert(NoLocationBlockPolicy::ALLOW_INLINING);
    }

    pub(super) fn make_first_copy_interruptible(&mut self) {
        debug_assert!(self.allows_block_inlining());
        self.block_policy
            .insert(NoLocationBlockPolicy::FIRST_COPY_INTERRUPTIBLE);
    }

    pub(super) fn mark_artificial_eval_break(&mut self) {
        self.block_policy
            .insert(NoLocationBlockPolicy::ARTIFICIAL_EVAL_BREAK);
    }

    pub(super) fn clear_artificial_eval_break(&mut self) {
        self.block_policy
            .remove(NoLocationBlockPolicy::ARTIFICIAL_EVAL_BREAK);
    }

    pub(super) fn make_copy_inherit_shared_predecessor(&mut self) {
        self.block_policy
            .insert(NoLocationBlockPolicy::COPY_INHERITS_SHARED_PREDECESSOR);
    }

    pub(super) fn allow_split_return_exit_copy(&mut self) {
        self.block_policy
            .insert(NoLocationBlockPolicy::COPY_SPLIT_RETURN_EXIT);
    }

    pub(super) fn clear_split_return_exit_copy(&mut self) {
        self.block_policy
            .remove(NoLocationBlockPolicy::COPY_SPLIT_RETURN_EXIT);
    }

    pub(super) const fn preserves_location(self) -> bool {
        self.block_policy.contains(NoLocationBlockPolicy::PRESERVE)
    }

    pub(super) fn preserve_location(&mut self) {
        self.block_policy.insert(NoLocationBlockPolicy::PRESERVE);
    }

    /// Allows a copied shared jump to inherit its new predecessor's location.
    pub(super) fn clear_preserved_location(&mut self) {
        self.block_policy.remove(NoLocationBlockPolicy::PRESERVE);
    }

    pub(super) const fn borrow_fallthrough(self) -> BorrowFallthrough {
        match self.block_policy.0 & NoLocationBlockPolicy::BORROW_FALLTHROUGH_MASK {
            0 => BorrowFallthrough::Reachable,
            NoLocationBlockPolicy::BORROW_BARRIER => BorrowFallthrough::Barrier,
            NoLocationBlockPolicy::BORROW_RETAINED_NOP_REACHABLE => {
                BorrowFallthrough::RetainedNopReachable
            }
            NoLocationBlockPolicy::BORROW_BARRIER_IF_REMOVED => BorrowFallthrough::BarrierIfRemoved,
            _ => unreachable!(),
        }
    }

    pub(super) fn set_borrow_fallthrough(&mut self, fallthrough: BorrowFallthrough) {
        let fallthrough = match fallthrough {
            BorrowFallthrough::Reachable => 0,
            BorrowFallthrough::Barrier => NoLocationBlockPolicy::BORROW_BARRIER,
            BorrowFallthrough::RetainedNopReachable => {
                NoLocationBlockPolicy::BORROW_RETAINED_NOP_REACHABLE
            }
            BorrowFallthrough::BarrierIfRemoved => NoLocationBlockPolicy::BORROW_BARRIER_IF_REMOVED,
        };
        self.block_policy.0 =
            self.block_policy.0 & !NoLocationBlockPolicy::BORROW_FALLTHROUGH_MASK | fallthrough;
    }

    const fn fused(first: Self, second: Self) -> Self {
        Self {
            block_policy: NoLocationBlockPolicy::fused(first.block_policy, second.block_policy),
            percent_target_location: if matches!(first.percent_target_location, 0 | u8::MAX) {
                second.percent_target_location
            } else {
                first.percent_target_location
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FusionPolicy(u8);

impl FusionPolicy {
    const PREVENT_WITH_PREVIOUS: u8 = 1 << 0;
    const PREVENT_WITH_NEXT: u8 = 1 << 1;
    // The four lossless operand variants share the two unused fusion-policy bits.
    const OPERAND_KIND_MASK: u8 = 0b11 << 2;
    const VALUE_OPERAND: u8 = 0b00 << 2;
    const DEFERRED_OPERAND: u8 = 0b01 << 2;
    const FORWARD_OPERAND: u8 = 0b10 << 2;
    const BACKWARD_OPERAND: u8 = 0b11 << 2;
    // Jump provenance occupies the remaining four bits; fusion keeps its asymmetric location rule.
    const AUTHORED_BREAK: u8 = 1 << 4;
    const TRANSITIVE_SMALL_EXIT_INLINING: u8 = 1 << 5;
    const LOCATION_FROM_NEXT: u8 = 1 << 6;
    const CONVERTED_POP_BLOCK: u8 = 1 << 7;

    pub(super) const fn prevents_with_previous(self) -> bool {
        self.0 & Self::PREVENT_WITH_PREVIOUS != 0
    }

    pub(super) const fn prevents_with_next(self) -> bool {
        self.0 & Self::PREVENT_WITH_NEXT != 0
    }

    pub(super) fn prevent_with_previous(&mut self) {
        self.0 |= Self::PREVENT_WITH_PREVIOUS;
    }

    pub(super) fn prevent_with_next(&mut self) {
        self.0 |= Self::PREVENT_WITH_NEXT;
    }

    const fn operand(self, payload: u32) -> Operand {
        match self.0 & Self::OPERAND_KIND_MASK {
            Self::VALUE_OPERAND => Operand::Value(payload),
            Self::DEFERRED_OPERAND => Operand::Deferred(OperandPatchId(payload)),
            Self::FORWARD_OPERAND => Operand::Forward(Label(payload)),
            Self::BACKWARD_OPERAND => Operand::Backward(Label(payload)),
            _ => unreachable!(),
        }
    }

    fn set_operand(&mut self, operand: Operand) -> u32 {
        let (kind, payload) = match operand {
            Operand::Value(payload) => (Self::VALUE_OPERAND, payload),
            Operand::Deferred(OperandPatchId(payload)) => (Self::DEFERRED_OPERAND, payload),
            Operand::Forward(Label(payload)) => (Self::FORWARD_OPERAND, payload),
            Operand::Backward(Label(payload)) => (Self::BACKWARD_OPERAND, payload),
        };
        self.0 = (self.0 & !Self::OPERAND_KIND_MASK) | kind;
        payload
    }

    pub(super) const fn is_authored_break(self) -> bool {
        self.0 & Self::AUTHORED_BREAK != 0
    }

    pub(super) fn mark_authored_break(&mut self) {
        self.0 |= Self::AUTHORED_BREAK;
    }

    pub(super) const fn transitive_small_exit_inlining(self) -> bool {
        self.0 & Self::TRANSITIVE_SMALL_EXIT_INLINING != 0
    }

    pub(super) fn allow_transitive_small_exit_inlining(&mut self) {
        self.0 |= Self::TRANSITIVE_SMALL_EXIT_INLINING;
    }

    pub(super) const fn location_from_next(self) -> bool {
        self.0 & Self::LOCATION_FROM_NEXT != 0
    }

    pub(super) fn mark_location_from_next(&mut self) {
        self.0 |= Self::LOCATION_FROM_NEXT;
    }

    pub(super) fn clear_location_from_next(&mut self) {
        self.0 &= !Self::LOCATION_FROM_NEXT;
    }

    pub(super) const fn is_converted_pop_block(self) -> bool {
        self.0 & Self::CONVERTED_POP_BLOCK != 0
    }

    pub(super) fn mark_converted_pop_block(&mut self) {
        self.0 |= Self::CONVERTED_POP_BLOCK;
    }

    const fn fused(first: Self, second: Self) -> Self {
        let location_from_next = if first.location_from_next() {
            second.0 & Self::LOCATION_FROM_NEXT
        } else {
            0
        };
        Self(
            first.0 & Self::PREVENT_WITH_PREVIOUS
                | second.0 & Self::PREVENT_WITH_NEXT
                | (first.0 | second.0)
                    & (Self::AUTHORED_BREAK
                        | Self::TRANSITIVE_SMALL_EXIT_INLINING
                        | Self::CONVERTED_POP_BLOCK)
                | location_from_next,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum BorrowFallthrough {
    #[default]
    Reachable,
    Barrier,
    RetainedNopReachable,
    BarrierIfRemoved,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum RetainedJumpNop {
    #[default]
    None,
    // Retain the jump's line event only while it still points at its original direct target.
    DirectTarget,
    Always,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct JumpPolicy(u16);

impl JumpPolicy {
    const RETAINED_NOP_MASK: u16 = 0b11;
    const RETAINED_NOP_ALWAYS: u16 = 0b01;
    const RETAINED_NOP_DIRECT_TARGET: u16 = 0b10;
    // Surviving original JUMPs need an interrupt check and do not request a retained
    // jump NOP. Share the otherwise unused fourth tag to keep the IR compact.
    const INTERRUPTIBLE: u16 = 0b11;
    const PREVENT_SMALL_EXIT_INLINING: u16 = 1 << 2;
    const PREVENT_THREADING_TARGET: u16 = 1 << 3;
    const DEFER_REDUNDANT_REMOVAL: u16 = 1 << 4;
    const PRESERVE_NOP_AFTER_THREADING: u16 = 1 << 5;
    const BORROW_EMPTY_BLOCK: u16 = 1 << 6;
    const PRESERVE_EXPLICIT_NOP: u16 = 1 << 7;
    const NORMALIZED_CONDITIONAL_BACKEDGE: u16 = 1 << 8;
    const PREVENT_BORROW_THREADING_TARGET: u16 = 1 << 9;

    const SYNTHETIC: Self = Self(Self::PREVENT_SMALL_EXIT_INLINING);
    const DEFAULT: Self = Self(0);

    pub(super) const fn fused(first: Self, second: Self) -> Self {
        let mut policy = Self::DEFAULT;
        if !first.inline_small_exit() || !second.inline_small_exit() {
            policy.0 |= Self::PREVENT_SMALL_EXIT_INLINING;
        }
        if !first.allows_threading_target() || !second.allows_threading_target() {
            policy.0 |= Self::PREVENT_THREADING_TARGET;
        }
        if !first.allows_borrow_threading_target() || !second.allows_borrow_threading_target() {
            policy.0 |= Self::PREVENT_BORROW_THREADING_TARGET;
        }
        if first.defers_redundant_removal() || second.defers_redundant_removal() {
            policy.0 |= Self::DEFER_REDUNDANT_REMOVAL;
        }
        if first.preserves_nop_after_threading() || second.preserves_nop_after_threading() {
            policy.0 |= Self::PRESERVE_NOP_AFTER_THREADING;
        }
        if first.borrow_empty_block() || second.borrow_empty_block() {
            policy.0 |= Self::BORROW_EMPTY_BLOCK;
        }
        if first.preserves_explicit_nop() || second.preserves_explicit_nop() {
            policy.0 |= Self::PRESERVE_EXPLICIT_NOP;
        }
        if first.is_normalized_conditional_backedge() || second.is_normalized_conditional_backedge()
        {
            policy.0 |= Self::NORMALIZED_CONDITIONAL_BACKEDGE;
        }
        policy
    }

    pub(super) const fn inline_small_exit(self) -> bool {
        self.0 & Self::PREVENT_SMALL_EXIT_INLINING == 0
    }

    pub(super) const fn allows_threading_target(self) -> bool {
        self.0 & Self::PREVENT_THREADING_TARGET == 0
    }

    pub(super) const fn allows_borrow_threading_target(self) -> bool {
        self.0 & Self::PREVENT_BORROW_THREADING_TARGET == 0
    }

    pub(super) const fn defers_redundant_removal(self) -> bool {
        self.0 & Self::DEFER_REDUNDANT_REMOVAL != 0
    }

    pub(super) const fn preserves_nop_after_threading(self) -> bool {
        self.0 & Self::PRESERVE_NOP_AFTER_THREADING != 0
    }

    pub(super) const fn borrow_empty_block(self) -> bool {
        self.0 & Self::BORROW_EMPTY_BLOCK != 0
    }

    pub(super) const fn preserves_explicit_nop(self) -> bool {
        self.0 & Self::PRESERVE_EXPLICIT_NOP != 0
    }

    pub(super) const fn is_normalized_conditional_backedge(self) -> bool {
        self.0 & Self::NORMALIZED_CONDITIONAL_BACKEDGE != 0
    }

    pub(super) fn mark_normalized_conditional_backedge(&mut self) {
        self.0 |= Self::NORMALIZED_CONDITIONAL_BACKEDGE;
    }

    pub(super) fn mark_borrow_empty_block(&mut self) {
        self.0 |= Self::BORROW_EMPTY_BLOCK;
    }

    pub(super) fn clear_borrow_empty_block(&mut self) {
        self.0 &= !Self::BORROW_EMPTY_BLOCK;
    }

    pub(super) fn preserve_explicit_nop(&mut self) {
        self.0 |= Self::PRESERVE_EXPLICIT_NOP;
    }

    pub(super) const fn retained_nop(self) -> RetainedJumpNop {
        match self.0 & Self::RETAINED_NOP_MASK {
            0 | Self::INTERRUPTIBLE => RetainedJumpNop::None,
            Self::RETAINED_NOP_ALWAYS => RetainedJumpNop::Always,
            Self::RETAINED_NOP_DIRECT_TARGET => RetainedJumpNop::DirectTarget,
            _ => unreachable!(),
        }
    }

    pub(super) const fn is_interruptible(self) -> bool {
        self.0 & Self::RETAINED_NOP_MASK == Self::INTERRUPTIBLE
    }

    pub(super) fn mark_interruptible(&mut self) {
        debug_assert!(matches!(self.retained_nop(), RetainedJumpNop::None));
        self.0 = (self.0 & !Self::RETAINED_NOP_MASK) | Self::INTERRUPTIBLE;
    }

    pub(super) const fn always_retains_nop(self) -> bool {
        matches!(self.retained_nop(), RetainedJumpNop::Always)
    }

    pub(super) fn retains_nop_for(self, operand: Operand) -> bool {
        match (self.retained_nop(), operand) {
            (RetainedJumpNop::Always, _) => true,
            (RetainedJumpNop::DirectTarget, Operand::Forward(_)) => true,
            (RetainedJumpNop::None | RetainedJumpNop::DirectTarget, _) => false,
        }
    }

    pub(super) fn retain_nop(&mut self) {
        debug_assert!(!self.is_interruptible());
        self.0 = (self.0 & !Self::RETAINED_NOP_MASK) | Self::RETAINED_NOP_ALWAYS;
    }

    pub(super) fn retain_nop_for_direct_target(&mut self) {
        debug_assert!(!self.is_interruptible());
        if !self.always_retains_nop() {
            self.0 = (self.0 & !Self::RETAINED_NOP_MASK) | Self::RETAINED_NOP_DIRECT_TARGET;
        }
    }

    fn clear_direct_target_retention(&mut self) {
        if matches!(self.retained_nop(), RetainedJumpNop::DirectTarget) {
            self.0 &= !Self::RETAINED_NOP_MASK;
        }
    }

    pub(super) fn defer_removal(&mut self) {
        self.0 |= Self::DEFER_REDUNDANT_REMOVAL;
    }

    pub(super) fn retain_redundant_nop_after_threading(&mut self) {
        self.0 |= Self::DEFER_REDUNDANT_REMOVAL | Self::PRESERVE_NOP_AFTER_THREADING;
    }

    #[cfg(test)]
    pub(super) fn preserve_nop_after_threading_for_test(&mut self) {
        self.0 |= Self::PRESERVE_NOP_AFTER_THREADING;
    }

    pub(super) fn prevent_small_exit_inlining(&mut self) {
        self.0 |= Self::PREVENT_SMALL_EXIT_INLINING;
    }

    pub(super) fn prevent_threading_target(&mut self) {
        self.0 |= Self::PREVENT_THREADING_TARGET;
    }

    pub(super) fn prevent_borrow_threading_target(&mut self) {
        self.0 |= Self::PREVENT_BORROW_THREADING_TARGET;
    }

    pub(super) fn resolve_deferred_removal(&mut self) {
        debug_assert!(self.defers_redundant_removal());
        self.0 &= !Self::DEFER_REDUNDANT_REMOVAL;
        if self.preserves_nop_after_threading() {
            // The redundant jump becomes a NOP, so it no longer carries an interrupt
            // check. Replace that pending-jump tag with the retained-NOP tag.
            self.0 &= !(Self::PRESERVE_NOP_AFTER_THREADING | Self::RETAINED_NOP_MASK);
            self.retain_nop();
        }
    }
}

impl std::fmt::Debug for JumpPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JumpPolicy")
            .field("inline_small_exit", &self.inline_small_exit())
            .field("retained_nop", &self.retained_nop())
            .field("interruptible", &self.is_interruptible())
            .field("allow_threading_target", &self.allows_threading_target())
            .field(
                "allow_borrow_threading_target",
                &self.allows_borrow_threading_target(),
            )
            .field("defer_redundant_removal", &self.defers_redundant_removal())
            .field(
                "preserve_nop_after_threading",
                &self.preserves_nop_after_threading(),
            )
            .field("borrow_empty_block", &self.borrow_empty_block())
            .field("preserve_explicit_nop", &self.preserves_explicit_nop())
            .field(
                "normalized_conditional_backedge",
                &self.is_normalized_conditional_backedge(),
            )
            .finish()
    }
}

impl Default for JumpPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ExceptionExclusion {
    #[default]
    None,
    IfExtended,
    Always,
}

impl ExceptionExclusion {
    const fn merged(self, other: Self) -> Self {
        match (self, other) {
            (Self::Always, _) | (_, Self::Always) => Self::Always,
            (Self::IfExtended, _) | (_, Self::IfExtended) => Self::IfExtended,
            (Self::None, Self::None) => Self::None,
        }
    }

    pub(super) const fn applies(self, extended_args: u8) -> bool {
        match self {
            Self::None => false,
            Self::IfExtended => extended_args > 0,
            Self::Always => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum NormalizedExceptionOwner {
    #[default]
    Unchanged,
    Present,
    OverridePresent,
    Absent,
    Dynamic,
    DynamicCondition,
    DynamicConditionStrict,
}

impl NormalizedExceptionOwner {
    const fn merged(self, other: Self) -> Self {
        match (self, other) {
            (Self::OverridePresent, _) | (_, Self::OverridePresent) => Self::OverridePresent,
            (Self::Unchanged, other) => other,
            (owner, _) => owner,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum ExceptionPolicy {
    #[default]
    NoneUnchanged,
    NonePresent,
    NoneOverridePresent,
    NoneAbsent,
    IfExtendedUnchanged,
    IfExtendedPresent,
    IfExtendedOverridePresent,
    IfExtendedAbsent,
    AlwaysUnchanged,
    AlwaysPresent,
    AlwaysOverridePresent,
    AlwaysAbsent,
    NoneDynamic,
    IfExtendedDynamic,
    AlwaysDynamic,
    NoneDynamicCondition,
    IfExtendedDynamicCondition,
    AlwaysDynamicCondition,
    NoneDynamicConditionStrict,
    IfExtendedDynamicConditionStrict,
    AlwaysDynamicConditionStrict,
    /// Authored Boolean-singleton failure marker; ownership stays unchanged unless certified.
    NoneUnchangedBooleanSingleton,
}

impl ExceptionPolicy {
    #[cfg(test)]
    pub(crate) const EXCLUDE_IF_EXTENDED: Self = Self::IfExtendedUnchanged;
    pub(crate) const EXCLUDE_ALWAYS: Self = Self::AlwaysUnchanged;

    const fn new(exclusion: ExceptionExclusion, owner: NormalizedExceptionOwner) -> Self {
        match (exclusion, owner) {
            (ExceptionExclusion::None, NormalizedExceptionOwner::DynamicConditionStrict) => {
                Self::NoneDynamicConditionStrict
            }
            (ExceptionExclusion::IfExtended, NormalizedExceptionOwner::DynamicConditionStrict) => {
                Self::IfExtendedDynamicConditionStrict
            }
            (ExceptionExclusion::Always, NormalizedExceptionOwner::DynamicConditionStrict) => {
                Self::AlwaysDynamicConditionStrict
            }
            (ExceptionExclusion::None, NormalizedExceptionOwner::DynamicCondition) => {
                Self::NoneDynamicCondition
            }
            (ExceptionExclusion::IfExtended, NormalizedExceptionOwner::DynamicCondition) => {
                Self::IfExtendedDynamicCondition
            }
            (ExceptionExclusion::Always, NormalizedExceptionOwner::DynamicCondition) => {
                Self::AlwaysDynamicCondition
            }
            (ExceptionExclusion::None, NormalizedExceptionOwner::Dynamic) => Self::NoneDynamic,
            (ExceptionExclusion::IfExtended, NormalizedExceptionOwner::Dynamic) => {
                Self::IfExtendedDynamic
            }
            (ExceptionExclusion::Always, NormalizedExceptionOwner::Dynamic) => Self::AlwaysDynamic,
            (ExceptionExclusion::None, NormalizedExceptionOwner::Unchanged) => Self::NoneUnchanged,
            (ExceptionExclusion::None, NormalizedExceptionOwner::Present) => Self::NonePresent,
            (ExceptionExclusion::None, NormalizedExceptionOwner::OverridePresent) => {
                Self::NoneOverridePresent
            }
            (ExceptionExclusion::None, NormalizedExceptionOwner::Absent) => Self::NoneAbsent,
            (ExceptionExclusion::IfExtended, NormalizedExceptionOwner::Unchanged) => {
                Self::IfExtendedUnchanged
            }
            (ExceptionExclusion::IfExtended, NormalizedExceptionOwner::Present) => {
                Self::IfExtendedPresent
            }
            (ExceptionExclusion::IfExtended, NormalizedExceptionOwner::OverridePresent) => {
                Self::IfExtendedOverridePresent
            }
            (ExceptionExclusion::IfExtended, NormalizedExceptionOwner::Absent) => {
                Self::IfExtendedAbsent
            }
            (ExceptionExclusion::Always, NormalizedExceptionOwner::Unchanged) => {
                Self::AlwaysUnchanged
            }
            (ExceptionExclusion::Always, NormalizedExceptionOwner::Present) => Self::AlwaysPresent,
            (ExceptionExclusion::Always, NormalizedExceptionOwner::OverridePresent) => {
                Self::AlwaysOverridePresent
            }
            (ExceptionExclusion::Always, NormalizedExceptionOwner::Absent) => Self::AlwaysAbsent,
        }
    }

    /// Checks original block vacancies when the compiler has not established an owner.
    pub(crate) const fn defer_condition_owner(self) -> Self {
        if matches!(self, Self::NoneAbsent) {
            Self::NoneDynamicCondition
        } else {
            self
        }
    }

    pub(crate) const fn normalized_owner(has_owner: bool) -> Self {
        if has_owner {
            Self::NonePresent
        } else {
            Self::NoneAbsent
        }
    }

    pub(super) const fn exclusion(self) -> ExceptionExclusion {
        match self {
            Self::NoneUnchanged
            | Self::NonePresent
            | Self::NoneOverridePresent
            | Self::NoneAbsent
            | Self::NoneDynamic
            | Self::NoneDynamicCondition
            | Self::NoneDynamicConditionStrict
            | Self::NoneUnchangedBooleanSingleton => ExceptionExclusion::None,
            Self::IfExtendedUnchanged
            | Self::IfExtendedPresent
            | Self::IfExtendedOverridePresent
            | Self::IfExtendedAbsent
            | Self::IfExtendedDynamic
            | Self::IfExtendedDynamicCondition
            | Self::IfExtendedDynamicConditionStrict => ExceptionExclusion::IfExtended,
            Self::AlwaysUnchanged
            | Self::AlwaysPresent
            | Self::AlwaysOverridePresent
            | Self::AlwaysAbsent
            | Self::AlwaysDynamic
            | Self::AlwaysDynamicCondition
            | Self::AlwaysDynamicConditionStrict => ExceptionExclusion::Always,
        }
    }

    pub(super) const fn owner(self) -> NormalizedExceptionOwner {
        match self {
            Self::NoneDynamicConditionStrict
            | Self::IfExtendedDynamicConditionStrict
            | Self::AlwaysDynamicConditionStrict => {
                NormalizedExceptionOwner::DynamicConditionStrict
            }
            Self::NoneDynamicCondition
            | Self::IfExtendedDynamicCondition
            | Self::AlwaysDynamicCondition => NormalizedExceptionOwner::DynamicCondition,
            Self::NoneDynamic | Self::IfExtendedDynamic | Self::AlwaysDynamic => {
                NormalizedExceptionOwner::Dynamic
            }
            Self::NoneUnchanged
            | Self::IfExtendedUnchanged
            | Self::AlwaysUnchanged
            | Self::NoneUnchangedBooleanSingleton => NormalizedExceptionOwner::Unchanged,
            Self::NonePresent | Self::IfExtendedPresent | Self::AlwaysPresent => {
                NormalizedExceptionOwner::Present
            }
            Self::NoneOverridePresent
            | Self::IfExtendedOverridePresent
            | Self::AlwaysOverridePresent => NormalizedExceptionOwner::OverridePresent,
            Self::NoneAbsent | Self::IfExtendedAbsent | Self::AlwaysAbsent => {
                NormalizedExceptionOwner::Absent
            }
        }
    }

    pub(super) fn set_owner(&mut self, owner: NormalizedExceptionOwner) {
        *self = Self::new(self.exclusion(), owner);
    }

    const fn merged(self, other: Self) -> Self {
        Self::new(
            self.exclusion().merged(other.exclusion()),
            self.owner().merged(other.owner()),
        )
    }

    pub(super) fn exclude(&mut self, exclusion: ExceptionExclusion) {
        *self = Self::new(self.exclusion().merged(exclusion), self.owner());
    }
}

/// Packs independent instruction metadata with zero reserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InstructionMetadata(NonZeroU16);

impl InstructionMetadata {
    const POLICY_COUNT: u16 = 22;
    const NORMAL_STATES: u16 = Self::POLICY_COUNT * 5 * 2;

    // Values 221..=264 retain all 22 exception policies and both copy modes for
    // a direct original jump with no earlier copied slots. Adding slots discards
    // this conservative proof; existing slot counts keep their full range.
    const fn has_initial_borrow_entry(self) -> bool {
        self.0.get() > Self::NORMAL_STATES
    }

    const fn normal_bits(self) -> u16 {
        if self.has_initial_borrow_entry() {
            self.0.get() - Self::NORMAL_STATES - 1
        } else {
            self.0.get() - 1
        }
    }

    fn with_initial_borrow_entry(self, retain: bool) -> Self {
        let bits = self.normal_bits() + 1;
        let bits = if retain && self.pre_fold_extra_instructions() == 0 {
            bits + Self::NORMAL_STATES
        } else {
            bits
        };
        Self(NonZeroU16::new(bits).expect("instruction metadata reserves zero"))
    }

    fn new(
        exception_policy: ExceptionPolicy,
        pre_fold_extra_instructions: u8,
        unconditional_inlining_only: bool,
    ) -> Self {
        // The early exit pass copies at most four instructions, so four extra instructions
        // on any one surviving instruction are sufficient to rule it out.
        // 22 policies, five slot counts, and two copy modes occupy 220 nonzero values.
        let bits = ((u16::from(pre_fold_extra_instructions.min(4)) * Self::POLICY_COUNT)
            + exception_policy as u16)
            * 2
            + u16::from(unconditional_inlining_only)
            + 1;
        Self(NonZeroU16::new(bits).expect("exception policy reserves the zero discriminant"))
    }

    const fn exception_policy(self) -> ExceptionPolicy {
        match (self.normal_bits() / 2) % Self::POLICY_COUNT {
            0 => ExceptionPolicy::NoneUnchanged,
            1 => ExceptionPolicy::NonePresent,
            2 => ExceptionPolicy::NoneOverridePresent,
            3 => ExceptionPolicy::NoneAbsent,
            4 => ExceptionPolicy::IfExtendedUnchanged,
            5 => ExceptionPolicy::IfExtendedPresent,
            6 => ExceptionPolicy::IfExtendedOverridePresent,
            7 => ExceptionPolicy::IfExtendedAbsent,
            8 => ExceptionPolicy::AlwaysUnchanged,
            9 => ExceptionPolicy::AlwaysPresent,
            10 => ExceptionPolicy::AlwaysOverridePresent,
            11 => ExceptionPolicy::AlwaysAbsent,
            12 => ExceptionPolicy::NoneDynamic,
            13 => ExceptionPolicy::IfExtendedDynamic,
            14 => ExceptionPolicy::AlwaysDynamic,
            15 => ExceptionPolicy::NoneDynamicCondition,
            16 => ExceptionPolicy::IfExtendedDynamicCondition,
            17 => ExceptionPolicy::AlwaysDynamicCondition,
            18 => ExceptionPolicy::NoneDynamicConditionStrict,
            19 => ExceptionPolicy::IfExtendedDynamicConditionStrict,
            20 => ExceptionPolicy::AlwaysDynamicConditionStrict,
            21 => ExceptionPolicy::NoneUnchangedBooleanSingleton,
            _ => unreachable!(),
        }
    }

    const fn pre_fold_extra_instructions(self) -> u8 {
        // Valid metadata reserves five slot counts; reject invalid encodings.
        match self.normal_bits() / (Self::POLICY_COUNT * 2) {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            _ => unreachable!(),
        }
    }

    const fn only_allows_unconditional_inlining(self) -> bool {
        self.normal_bits() & 1 != 0
    }
}

impl Default for InstructionMetadata {
    fn default() -> Self {
        Self::new(ExceptionPolicy::default(), 0, false)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Instruction {
    pub(super) opcode: Opcode,
    operand_payload: u32,
    pub(super) location: SourceLocation,
    depth_after: u32,
    pub(super) local_policy: LocalPolicy,
    pub(super) no_location_policy: NoLocationPolicy,
    pub(super) fusion_policy: FusionPolicy,
    pub(super) jump_policy: JumpPolicy,
    // `NOT_TAKEN` is added after CPython labels exception handlers. The new instruction keeps
    // whatever exception target remains in its reused CFG slot, if there is one.
    metadata: InstructionMetadata,
}

impl Instruction {
    const NO_DEPTH_AFTER: u32 = u32::MAX;

    pub(super) fn new(
        opcode: Opcode,
        operand: Operand,
        location: SourceLocation,
        depth_after: Option<u32>,
    ) -> Self {
        let mut fusion_policy = FusionPolicy::default();
        let operand_payload = fusion_policy.set_operand(operand);
        Self {
            opcode,
            operand_payload,
            location,
            depth_after: depth_after.unwrap_or(Self::NO_DEPTH_AFTER),
            local_policy: LocalPolicy::default(),
            no_location_policy: NoLocationPolicy::default(),
            fusion_policy,
            jump_policy: JumpPolicy::default(),
            metadata: InstructionMetadata::default(),
        }
    }

    pub(super) fn value_with_depth(
        opcode: Opcode,
        argument: u32,
        location: SourceLocation,
        depth_after: u32,
    ) -> Self {
        Self {
            opcode,
            operand_payload: argument,
            location,
            depth_after,
            local_policy: LocalPolicy::default(),
            no_location_policy: NoLocationPolicy::default(),
            fusion_policy: FusionPolicy::default(),
            jump_policy: JumpPolicy::default(),
            metadata: InstructionMetadata::default(),
        }
    }

    pub(super) const fn exception_policy(&self) -> ExceptionPolicy {
        self.metadata.exception_policy()
    }

    pub(super) fn set_exception_policy(&mut self, policy: ExceptionPolicy) {
        self.metadata = InstructionMetadata::new(
            policy,
            self.pre_fold_extra_instructions(),
            self.only_allows_unconditional_inlining(),
        )
        .with_initial_borrow_entry(self.metadata.has_initial_borrow_entry());
    }

    pub(super) fn set_exception_owner(&mut self, owner: NormalizedExceptionOwner) {
        let mut policy = self.exception_policy();
        policy.set_owner(owner);
        self.set_exception_policy(policy);
    }

    pub(super) fn exclude_from_exception_table(&mut self, exclusion: ExceptionExclusion) {
        let mut policy = self.exception_policy();
        policy.exclude(exclusion);
        self.set_exception_policy(policy);
    }

    pub(super) const fn pre_fold_extra_instructions(&self) -> u8 {
        self.metadata.pre_fold_extra_instructions()
    }

    pub(super) fn add_pre_fold_extra_instructions(&mut self, count: u8) {
        self.metadata = InstructionMetadata::new(
            self.exception_policy(),
            self.pre_fold_extra_instructions().saturating_add(count),
            self.only_allows_unconditional_inlining(),
        )
        .with_initial_borrow_entry(self.metadata.has_initial_borrow_entry());
    }

    pub(super) fn allow_unconditional_block_inlining(&mut self) {
        self.no_location_policy.allow_block_inlining();
        self.metadata = InstructionMetadata::new(
            self.exception_policy(),
            self.pre_fold_extra_instructions(),
            true,
        )
        .with_initial_borrow_entry(self.metadata.has_initial_borrow_entry());
    }

    pub(super) const fn only_allows_unconditional_inlining(&self) -> bool {
        self.metadata.only_allows_unconditional_inlining()
    }

    pub(super) const fn depth_after(&self) -> Option<u32> {
        if self.depth_after == Self::NO_DEPTH_AFTER {
            None
        } else {
            Some(self.depth_after)
        }
    }

    /// Includes the exception-entry peak consumed by a surviving cleanup handler.
    pub(super) fn maximum_stack_depth(&self) -> u32 {
        self.depth_after().map_or(0, |depth| {
            // CLEANUP_THROW starts an exception handler and pops before any instruction
            // can record its incoming depth. Preserve that live peak when dead-code
            // removal causes the compiler to recompute the maximum from surviving IR.
            depth + u32::from(self.opcode == crate::target::opcodes::CLEANUP_THROW)
        })
    }

    pub(super) const fn operand(&self) -> Operand {
        self.fusion_policy.operand(self.operand_payload)
    }

    pub(super) fn set_operand(&mut self, operand: Operand) {
        if self.operand() != operand {
            self.metadata = self.metadata.with_initial_borrow_entry(false);
        }
        self.operand_payload = self.fusion_policy.set_operand(operand);
    }

    /// Records an initial direct edge until its operand changes or the proof is consumed.
    pub(super) fn mark_initial_borrow_jump_target(&mut self, target: Label) {
        debug_assert!(
            matches!(self.operand(), Operand::Forward(label) | Operand::Backward(label) if label == target)
        );
        self.metadata = self.metadata.with_initial_borrow_entry(true);
    }

    pub(super) const fn initial_borrow_jump_target(&self) -> Option<Label> {
        if self.metadata.has_initial_borrow_entry() {
            match self.operand() {
                Operand::Forward(target) | Operand::Backward(target) => Some(target),
                Operand::Value(_) | Operand::Deferred(_) => None,
            }
        } else {
            None
        }
    }

    pub(super) fn clear_initial_borrow_jump_target(&mut self) {
        self.metadata = self.metadata.with_initial_borrow_entry(false);
    }

    /// Retargets a jump while keeping direct-target line-event provenance honest.
    pub(super) fn retarget(&mut self, operand: Operand) {
        debug_assert!(matches!(
            operand,
            Operand::Forward(_) | Operand::Backward(_)
        ));
        if self.operand() != operand {
            self.jump_policy.clear_direct_target_retention();
            self.no_location_policy.clear_split_return_exit_copy();
            self.set_operand(operand);
        }
    }

    /// Creates an instruction introduced by an assembler pass rather than the compiler.
    pub(super) fn synthetic(
        opcode: Opcode,
        operand: Operand,
        location: SourceLocation,
        depth_after: Option<u32>,
    ) -> Self {
        let mut instruction = Self::new(opcode, operand, location, depth_after);
        instruction.jump_policy = JumpPolicy::SYNTHETIC;
        instruction
    }

    /// Combines the provenance of two instructions replaced by one superinstruction.
    pub(super) fn fused(opcode: Opcode, operand: Operand, first: Self, second: Self) -> Self {
        let location = if first.fusion_policy.location_from_next() {
            second.location
        } else {
            first.location
        };
        let mut fusion_policy = FusionPolicy::fused(first.fusion_policy, second.fusion_policy);
        let operand_payload = fusion_policy.set_operand(operand);
        Self {
            opcode,
            operand_payload,
            location,
            depth_after: second.depth_after,
            local_policy: LocalPolicy::fused(first.local_policy, second.local_policy),
            no_location_policy: NoLocationPolicy::fused(
                first.no_location_policy,
                second.no_location_policy,
            ),
            fusion_policy,
            jump_policy: JumpPolicy::fused(first.jump_policy, second.jump_policy),
            metadata: InstructionMetadata::new(
                first.exception_policy().merged(second.exception_policy()),
                first.pre_fold_extra_instructions() + second.pre_fold_extra_instructions(),
                first.only_allows_unconditional_inlining()
                    || second.only_allows_unconditional_inlining(),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Item {
    Instruction(Instruction),
    Label(Label),
}

#[derive(Debug)]
pub(super) enum UnoptimizedCode {
    /// Instructions are still being emitted and patched by the compiler.
    Linear(Vec<Item>),
    /// Constant reachability has prepared the graph that the CFG pipeline will consume.
    Prepared {
        graph: BlockGraph,
        reachable: Vec<bool>,
    },
}

/// A call that keeps an original normal-cleanup entry nonempty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BorrowCleanupCall {
    pub(super) argument: u32,
    pub(super) location: SourceLocation,
}

/// Normal entries retained across an omitted loop backedge.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct BorrowLoopExitPolicy {
    pub(super) allow_surviving_break: bool,
    pub(super) preserve_small_exit: bool,
    pub(super) cleanup_call: Option<BorrowCleanupCall>,
}

/// A folded condition whose entry must be compared with other pending destinations.
#[derive(Debug)]
pub(super) struct DeferredBorrowEntry {
    pub(super) target: Label,
    pub(super) other_pending_targets: Vec<Label>,
}

#[derive(Debug)]
pub(crate) struct Assembler {
    pub(super) code: UnoptimizedCode,
    /// Number of instructions in the linear item stream; labels do not consume offsets.
    pub(super) instruction_count: usize,
    pub(super) emission_stream: EmissionStreamId,
    /// Original conditional target edges removed by constant folding during lowering.
    pub(super) folded_away_condition_targets: FxHashSet<Label>,
    /// Original normal predecessors from discarded linear constant-loop else suites.
    pub(super) folded_away_loop_else_targets: FxHashSet<Label>,
    /// Existing source marker, target, and marker location of an original folded edge.
    pub(super) folded_predecessor_sources: Vec<(Label, Label, SourceLocation)>,
    pub(super) operand_patches: Vec<OperandPatch>,
    pub(super) deferred_none_constant_index: Option<u32>,
    pub(super) discarded_fold_admissions: FxHashMap<Label, Vec<u32>>,
    pub(super) next_label: u32,
    pub(super) location: SourceLocation,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    /// Original tail location retained when lowering emits an already-reordered restore.
    pub(super) original_fallthrough_location: Option<(EmittedInstruction, SourceLocation)>,
    pub(super) exception_owner_provenance: super::exception_owners::ExceptionOwnerProvenance,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) borrow_unreachable_blocks: FxHashSet<Label>,
    /// Cleanup targets whose original for-loop backedge array was omitted during lowering.
    pub(super) borrow_omitted_for_backedges: FxHashSet<Label>,
    /// Entries whose empty original fallthrough can be bypassed by a surviving jump.
    pub(super) borrow_jump_entry_labels: FxHashSet<Label>,
    pub(super) deferred_borrow_entries: Vec<DeferredBorrowEntry>,
    /// Disconnected loop exits and the normal entries that remain reachable.
    pub(super) borrow_loop_exit_blocks: FxHashMap<Label, BorrowLoopExitPolicy>,
    /// Normal finally entries whose original nonempty cleanup can retain an explicit jump.
    pub(super) borrow_finally_entries: FxHashMap<Label, Option<BorrowCleanupCall>>,
    pub(super) borrow_block_aliases: FxHashSet<Label>,
    /// Original ordinary jump blocks that can thread conditional predecessors after exit copying.
    pub(super) conditional_threading_targets: FxHashMap<Label, SourceLocation>,
    /// Labels preceding an authored NOP omitted during statement lowering.
    pub(super) original_nop_targets: FxHashSet<Label>,
    /// Normal handler cleanup originally emitted without source locations.
    pub(super) original_unlocated_cleanup_targets: FxHashSet<Label>,
    /// Unlocated Boolean discards with a fallback whose source instruction folded away.
    pub(super) original_unlocated_discard_targets: FxHashSet<Label>,
    /// Original shared jump blocks whose source locations are resolved after exit copying.
    pub(super) deferred_shared_jump_blocks: FxHashSet<Label>,
    /// Original label-only prefixes of retained normal-finally `POP_BLOCK` instructions.
    pub(super) retained_pop_block_origins: FxHashMap<Label, Vec<Label>>,
    /// Shared jumps whose copies can acquire their new predecessor's source location.
    pub(super) shared_jump_copy_location_blocks: FxHashSet<Label>,
    /// Unique joins reached by the synthetic exits of early-folded conditions.
    pub(super) folded_condition_exit_targets: FxHashSet<Label>,
    pub(super) load_fast_borrowing_enabled: bool,
    pub(super) strict_owned_loads: bool,
    pub(super) forced_borrowed_loads: bool,
    pub(super) has_constant_pop_candidate: bool,
    pub(super) has_assertion_side_exit_candidate: bool,
    pub(super) has_proven_linear_assertion_side_exits: bool,
    pub(super) has_synthesized_percent_target_locations: bool,
    pub(super) has_next_location: bool,
}

/// A bounded proof retained between initial jump threading and exit copying.
///
/// The exact interleaved instructions and labels must remain contiguous. Physical
/// normalization may split this sequence at its captured owner labels, but no label,
/// instruction field, or relative position may change.
#[derive(Debug)]
pub(super) struct ThreadedBorrowSnapshot {
    pub(super) items: Vec<Item>,
    pub(super) eligible_jumps: FxHashSet<usize>,
}

pub(super) type ThreadedBorrowEntries = FxHashMap<Label, ThreadedBorrowSnapshot>;

/// Original reachable authored-break spans and their permitted finally routes.
#[derive(Debug, Default)]
pub(super) struct AuthoredFinallyBreakOrigins {
    pub(super) snapshots: ThreadedBorrowEntries,
    pub(super) routes: FxHashMap<Label, Label>,
}

/// A threaded authored break consumed before copies can inherit its labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AuthoredFinallyBreakEntry {
    pub(super) original_source: Label,
    pub(super) target: Label,
}

pub(super) type AuthoredFinallyBreakEntries = FxHashMap<Label, AuthoredFinallyBreakEntry>;

/// The block-aware assembler state used by CFG optimization passes.
///
/// Keeping this separate from [`Assembler`] makes it impossible to run a CFG
/// pass before instruction emission is complete.
#[derive(Debug)]
pub(super) struct OptimizationPipeline {
    pub(super) graph: BlockGraph,
    /// Original incoming edges removed before CPython resolves cleanup locations.
    pub(super) folded_original_predecessor_targets: FxHashSet<Label>,
    /// Source markers retained to prove that a folded edge was originally reachable.
    pub(super) folded_predecessor_sources: Vec<(Label, Label, SourceLocation)>,
    /// First-fold admissions retained before their next surviving source label.
    pub(super) discarded_fold_admissions: FxHashMap<Label, Vec<u32>>,
    pub(super) initial_reachable: Option<Vec<bool>>,
    pub(super) next_label: u32,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) exception_owner_provenance: super::exception_owners::ExceptionOwnerProvenance,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) borrow_unreachable_blocks: FxHashSet<Label>,
    /// Cleanup targets whose original for-loop backedge array was omitted during lowering.
    pub(super) borrow_omitted_for_backedges: FxHashSet<Label>,
    pub(super) borrow_jump_entry_labels: FxHashSet<Label>,
    /// Positive jump entries captured before exit copying; copies receive fresh labels.
    pub(super) borrow_pre_copy_jump_entries: FxHashMap<Label, Label>,
    /// Original forward jump-only routes eligible for intentional entry propagation.
    pub(super) borrow_threaded_jump_targets: FxHashMap<Label, Label>,
    /// Full block snapshots keep threaded origins independent of hidden-slot packing.
    pub(super) borrow_threaded_jump_entries: ThreadedBorrowEntries,
    /// Separately certified authored breaks into original normal-finally cleanup.
    pub(super) borrow_authored_finally_break_origins: AuthoredFinallyBreakOrigins,
    pub(super) borrow_authored_finally_break_entries: AuthoredFinallyBreakEntries,
    pub(super) borrow_loop_exit_blocks: FxHashMap<Label, BorrowLoopExitPolicy>,
    pub(super) borrow_finally_entries: FxHashMap<Label, Option<BorrowCleanupCall>>,
    pub(super) borrow_finally_jump_sources: FxHashMap<Label, bool>,
    pub(super) borrow_block_aliases: FxHashSet<Label>,
    /// Original ordinary jump blocks that can thread conditional predecessors after exit copying.
    pub(super) conditional_threading_targets: FxHashMap<Label, SourceLocation>,
    /// Labels preceding an authored NOP omitted during statement lowering.
    pub(super) original_nop_targets: FxHashSet<Label>,
    /// Normal handler cleanup originally emitted without source locations.
    pub(super) original_unlocated_cleanup_targets: FxHashSet<Label>,
    /// Unlocated Boolean discards with a fallback whose source instruction folded away.
    pub(super) original_unlocated_discard_targets: FxHashSet<Label>,
    /// Conditional entry labels mapped to their shared original unlocated jump array.
    pub(super) original_shared_unlocated_jump_entries: FxHashMap<Label, Label>,
    /// Original ordinary-jump heads, separate from synthetic source-location joins.
    pub(super) original_jump_head_targets: FxHashSet<Label>,
    pub(super) deferred_shared_jump_blocks: FxHashSet<Label>,
    /// Original label-only prefixes of retained normal-finally `POP_BLOCK` instructions.
    pub(super) retained_pop_block_origins: FxHashMap<Label, Vec<Label>>,
    pub(super) shared_jump_copy_location_blocks: FxHashSet<Label>,
    /// Unique joins reached by the synthetic exits of early-folded conditions.
    pub(super) folded_condition_exit_targets: FxHashSet<Label>,
    pub(super) has_assertion_side_exit_candidate: bool,
    pub(super) has_synthesized_percent_target_locations: bool,
    pub(super) has_next_location: bool,
}

/// The linear assembler state used after CFG optimization is complete.
#[derive(Debug)]
pub(super) struct LayoutAssembler {
    pub(super) items: Vec<Item>,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) exception_owner_provenance: super::exception_owners::ExceptionOwnerProvenance,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) removed_max_depth: Option<u32>,
    pub(super) next_label: u32,
    pub(super) has_next_location: bool,
}

#[derive(Clone, Copy, Debug)]
/// Preserves the order of nested protected regions before exits are copied and blocks reordered.
pub(super) enum ExceptionRegionPriority {
    OriginalRange(Label, Label),
    Resolved { start: usize, end: usize },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ExceptionRegion {
    /// Scope-wide handlers remain outside nested handlers even after exclusions split them.
    pub(super) is_scope_fallback: bool,
    /// Exclusions and copied exits retain the enclosing region's original priority.
    pub(super) priority: ExceptionRegionPriority,
    pub(super) start: Label,
    pub(super) end: Label,
    pub(super) target: Label,
    pub(super) depth: u32,
    pub(super) preserve_lasti: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceLocation {
    pub(crate) line: i32,
    pub(crate) end_line: i32,
    pub(crate) column: i32,
    pub(crate) end_column: i32,
}

impl SourceLocation {
    pub(crate) const NONE: Self = Self {
        line: -1,
        end_line: -1,
        column: -1,
        end_column: -1,
    };

    pub(crate) const fn new(line: i32, end_line: i32, column: i32, end_column: i32) -> Self {
        Self {
            line,
            end_line,
            column,
            end_column,
        }
    }
}

impl Default for Assembler {
    fn default() -> Self {
        Self {
            code: UnoptimizedCode::Linear(Vec::new()),
            instruction_count: 0,
            emission_stream: EmissionStreamId::next(),
            folded_away_condition_targets: FxHashSet::default(),
            folded_away_loop_else_targets: FxHashSet::default(),
            folded_predecessor_sources: Vec::new(),
            operand_patches: Vec::new(),
            deferred_none_constant_index: None,
            discarded_fold_admissions: FxHashMap::default(),
            next_label: 0,
            location: SourceLocation::NONE,
            exception_regions: Vec::new(),
            original_fallthrough_location: None,
            exception_owner_provenance: super::exception_owners::ExceptionOwnerProvenance::default(
            ),
            preserved_block_boundaries: FxHashSet::default(),
            borrow_unreachable_blocks: FxHashSet::default(),
            borrow_omitted_for_backedges: FxHashSet::default(),
            borrow_jump_entry_labels: FxHashSet::default(),
            deferred_borrow_entries: Vec::new(),
            borrow_loop_exit_blocks: FxHashMap::default(),
            borrow_finally_entries: FxHashMap::default(),
            borrow_block_aliases: FxHashSet::default(),
            conditional_threading_targets: FxHashMap::default(),
            original_nop_targets: FxHashSet::default(),
            original_unlocated_cleanup_targets: FxHashSet::default(),
            original_unlocated_discard_targets: FxHashSet::default(),
            load_fast_borrowing_enabled: true,
            strict_owned_loads: false,
            forced_borrowed_loads: false,
            has_constant_pop_candidate: false,
            has_assertion_side_exit_candidate: false,
            has_proven_linear_assertion_side_exits: false,
            has_synthesized_percent_target_locations: false,
            has_next_location: false,
            deferred_shared_jump_blocks: FxHashSet::default(),
            retained_pop_block_origins: FxHashMap::default(),
            shared_jump_copy_location_blocks: FxHashSet::default(),
            folded_condition_exit_targets: FxHashSet::default(),
        }
    }
}

#[cfg(test)]
mod initial_borrow_metadata_tests {
    use super::{
        ExceptionPolicy, Instruction, InstructionMetadata, Label, NonZeroU16, Operand,
        SourceLocation,
    };
    use crate::target::opcodes::JUMP_FORWARD;

    const POLICIES: [ExceptionPolicy; 22] = [
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
        ExceptionPolicy::NoneUnchangedBooleanSingleton,
    ];

    #[test]
    fn wide_initial_borrow_metadata_preserves_all_264_encodings() {
        let mut above_byte_capacity = 0;
        for raw in 1..=264 {
            let value = InstructionMetadata(NonZeroU16::new(raw).unwrap());
            let origin = raw > 220;
            let payload = if origin { raw - 221 } else { raw - 1 };
            assert_eq!(value.has_initial_borrow_entry(), origin);
            assert_eq!(
                value.exception_policy(),
                POLICIES[usize::from(payload / 2 % 22)]
            );
            assert_eq!(u16::from(value.pre_fold_extra_instructions()), payload / 44);
            assert_eq!(value.only_allows_unconditional_inlining(), payload % 2 != 0);
            if origin {
                assert_eq!(value.pre_fold_extra_instructions(), 0);
            }
            let rebuilt = InstructionMetadata::new(
                value.exception_policy(),
                value.pre_fold_extra_instructions(),
                value.only_allows_unconditional_inlining(),
            )
            .with_initial_borrow_entry(origin);
            assert_eq!(rebuilt.0.get(), raw);
            let cleared = value.with_initial_borrow_entry(false);
            assert!((1..=220).contains(&cleared.0.get()));
            assert_eq!(cleared.exception_policy(), value.exception_policy());
            assert_eq!(
                cleared.pre_fold_extra_instructions(),
                value.pre_fold_extra_instructions()
            );
            assert_eq!(
                cleared.only_allows_unconditional_inlining(),
                value.only_allows_unconditional_inlining()
            );
            if raw > u16::from(u8::MAX) {
                above_byte_capacity += 1;
                assert!(u8::try_from(rebuilt.0.get()).is_err());
            }
        }
        assert_eq!(above_byte_capacity, 9);
        for copy in [false, true] {
            let tagged =
                InstructionMetadata::new(ExceptionPolicy::NoneUnchangedBooleanSingleton, 0, copy)
                    .with_initial_borrow_entry(true);
            assert_eq!(tagged.0.get(), 263 + u16::from(copy));
        }
    }

    #[test]
    fn wide_metadata_preserves_policy_updates_and_invalidates_changed_origins() {
        for raw in 1..=264 {
            let metadata = InstructionMetadata(NonZeroU16::new(raw).unwrap());
            let mut original = Instruction::synthetic(
                JUMP_FORWARD,
                Operand::Forward(Label(1)),
                SourceLocation::NONE,
                None,
            );
            original.metadata = metadata;
            let origin = metadata.has_initial_borrow_entry();
            for policy in POLICIES {
                let mut changed = original;
                changed.set_exception_policy(policy);
                assert_eq!(changed.exception_policy(), policy);
                assert_eq!(
                    changed.pre_fold_extra_instructions(),
                    original.pre_fold_extra_instructions()
                );
                assert_eq!(
                    changed.only_allows_unconditional_inlining(),
                    original.only_allows_unconditional_inlining()
                );
                assert_eq!(changed.metadata.has_initial_borrow_entry(), origin);
            }
            let mut copied = original;
            copied.allow_unconditional_block_inlining();
            assert!(copied.only_allows_unconditional_inlining());
            assert_eq!(copied.exception_policy(), original.exception_policy());
            assert_eq!(
                copied.pre_fold_extra_instructions(),
                original.pre_fold_extra_instructions()
            );
            assert_eq!(copied.metadata.has_initial_borrow_entry(), origin);
            for extra in [0, 1, 4, u8::MAX] {
                let mut changed = original;
                changed.add_pre_fold_extra_instructions(extra);
                assert_eq!(
                    changed.pre_fold_extra_instructions(),
                    original
                        .pre_fold_extra_instructions()
                        .saturating_add(extra)
                        .min(4)
                );
                assert_eq!(changed.exception_policy(), original.exception_policy());
                assert_eq!(
                    changed.only_allows_unconditional_inlining(),
                    original.only_allows_unconditional_inlining()
                );
                assert_eq!(
                    changed.metadata.has_initial_borrow_entry(),
                    origin && extra == 0
                );
            }
            let mut retargeted = original;
            retargeted.set_operand(Operand::Forward(Label(1)));
            assert_eq!(retargeted.metadata.has_initial_borrow_entry(), origin);
            retargeted.set_operand(Operand::Forward(Label(2)));
            assert!(!retargeted.metadata.has_initial_borrow_entry());
            assert_eq!(retargeted.exception_policy(), original.exception_policy());
            assert_eq!(
                retargeted.pre_fold_extra_instructions(),
                original.pre_fold_extra_instructions()
            );
            assert_eq!(
                retargeted.only_allows_unconditional_inlining(),
                original.only_allows_unconditional_inlining()
            );
        }
    }
}
