use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU64, Ordering};

use ruff_index::newtype_index;
use rustc_hash::{FxHashMap, FxHashSet};

use super::block_graph::BlockGraph;
use crate::PythonVersion;
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

#[newtype_index]
pub(crate) struct Label;

#[newtype_index]
pub(crate) struct OperandPatchId;

#[newtype_index]
pub(crate) struct FoldAdmissionId;

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
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

    /// Removes one known vacancy without changing the local-load policy bits.
    pub(super) fn remove_hidden_exception_owner_slot(&mut self) {
        assert!(self.hidden_exception_owner_slots() > 0);
        self.0 -= 1 << Self::HIDDEN_EXCEPTION_OWNER_SLOTS_SHIFT;
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
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

    pub(super) fn prevent_block_inlining(&mut self) {
        self.block_policy
            .remove(NoLocationBlockPolicy::ALLOW_INLINING);
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
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
        let kind = self.0 & Self::OPERAND_KIND_MASK;
        if kind == Self::VALUE_OPERAND {
            return Operand::Value(payload);
        }
        match kind {
            Self::DEFERRED_OPERAND => Operand::Deferred(OperandPatchId::from_u32(payload)),
            Self::FORWARD_OPERAND => Operand::Forward(Label::from_u32(payload)),
            Self::BACKWARD_OPERAND => Operand::Backward(Label::from_u32(payload)),
            _ => unreachable!(),
        }
    }

    fn set_operand(&mut self, operand: Operand) -> u32 {
        let (kind, payload) = match operand {
            Operand::Value(payload) => (Self::VALUE_OPERAND, payload),
            Operand::Deferred(id) => (Self::DEFERRED_OPERAND, id.as_u32()),
            Operand::Forward(id) => (Self::FORWARD_OPERAND, id.as_u32()),
            Operand::Backward(id) => (Self::BACKWARD_OPERAND, id.as_u32()),
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

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
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
    const ORIGINAL_EMPTY_JOIN: u16 = 1 << 10;
    const ORIGINAL_SHARED_EMPTY_JOIN: u16 = 1 << 11;
    const ORIGINAL_UNIFORM_FOLD: u16 = 1 << 12;
    const RETAIN_NOP_AFTER_RETARGETING: u16 = 1 << 13;
    const ORIGINAL_MATCH_SUCCESS: u16 = 1 << 14;
    const AUTHORED_BRANCH_MARKER: u16 = 1 << 15;

    pub(super) const fn is_authored_branch_marker(self) -> bool {
        self.0 & Self::AUTHORED_BRANCH_MARKER != 0
    }

    pub(super) fn mark_authored_branch_marker(&mut self) {
        self.0 |= Self::AUTHORED_BRANCH_MARKER;
    }

    const SYNTHETIC: Self = Self(Self::PREVENT_SMALL_EXIT_INLINING);
    const DEFAULT: Self = Self(0);

    /// Combines optimization constraints and block provenance for a fused instruction.
    ///
    /// Tags tied to an original jump, including interrupt checks and retained jump NOPs,
    /// do not survive fusion.
    pub(super) const fn fused(first: Self, second: Self) -> Self {
        const PRESERVED: u16 = JumpPolicy::PREVENT_SMALL_EXIT_INLINING
            | JumpPolicy::PREVENT_THREADING_TARGET
            | JumpPolicy::PREVENT_BORROW_THREADING_TARGET
            | JumpPolicy::DEFER_REDUNDANT_REMOVAL
            | JumpPolicy::PRESERVE_NOP_AFTER_THREADING
            | JumpPolicy::BORROW_EMPTY_BLOCK
            | JumpPolicy::PRESERVE_EXPLICIT_NOP
            | JumpPolicy::NORMALIZED_CONDITIONAL_BACKEDGE
            | JumpPolicy::ORIGINAL_EMPTY_JOIN
            | JumpPolicy::ORIGINAL_SHARED_EMPTY_JOIN
            | JumpPolicy::AUTHORED_BRANCH_MARKER;
        Self((first.0 | second.0) & PRESERVED)
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

    pub(super) fn allow_explicit_nop_compaction(&mut self) {
        self.0 &= !Self::PRESERVE_EXPLICIT_NOP;
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

    /// Replaces a threaded jump's retained-NOP request with the inherited eval check.
    pub(super) fn mark_interruptible_after_threading(&mut self) {
        self.0 &= !Self::RETAINED_NOP_MASK;
        self.mark_interruptible();
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

    /// Keeps an authored line event when jump threading changes its exit target.
    pub(super) fn retain_nop_after_retargeting(&mut self) {
        self.retain_nop_for_direct_target();
        self.0 |= Self::RETAIN_NOP_AFTER_RETARGETING;
    }

    /// Identifies an unchanged literal fold after its copy budget has saturated.
    pub(super) const fn has_original_uniform_fold(self) -> bool {
        self.0 & Self::ORIGINAL_UNIFORM_FOLD != 0
    }

    pub(super) fn mark_original_uniform_fold(&mut self) {
        self.0 |= Self::ORIGINAL_UNIFORM_FOLD;
    }

    pub(super) fn retire_original_uniform_fold(&mut self) {
        self.0 &= !Self::ORIGINAL_UNIFORM_FOLD;
    }

    /// Identifies an emitted case-success edge before early cleanup-copy planning.
    pub(super) const fn is_original_match_success(self) -> bool {
        self.0 & Self::ORIGINAL_MATCH_SUCCESS != 0
    }

    pub(super) fn mark_original_match_success(&mut self) {
        self.0 |= Self::ORIGINAL_MATCH_SUCCESS;
    }

    fn retire_original_match_success(&mut self) {
        self.0 &= !Self::ORIGINAL_MATCH_SUCCESS;
    }

    pub(super) const fn is_original_empty_join(self) -> bool {
        self.0 & Self::ORIGINAL_EMPTY_JOIN != 0
    }

    pub(super) fn mark_original_empty_join(&mut self) {
        self.0 |= Self::ORIGINAL_EMPTY_JOIN;
    }

    pub(super) const fn is_original_shared_empty_join(self) -> bool {
        self.0 & Self::ORIGINAL_SHARED_EMPTY_JOIN != 0
    }

    pub(super) fn mark_original_shared_empty_join(&mut self) {
        self.0 |= Self::ORIGINAL_SHARED_EMPTY_JOIN;
    }

    fn clear_direct_target_retention(&mut self) {
        if matches!(self.retained_nop(), RetainedJumpNop::DirectTarget)
            && self.0 & Self::RETAIN_NOP_AFTER_RETARGETING == 0
        {
            self.0 &= !Self::RETAINED_NOP_MASK;
        }
    }

    pub(super) fn defer_removal(&mut self) {
        self.0 |= Self::DEFER_REDUNDANT_REMOVAL;
    }

    pub(super) fn retain_redundant_nop_after_threading(&mut self) {
        self.0 |= Self::DEFER_REDUNDANT_REMOVAL | Self::PRESERVE_NOP_AFTER_THREADING;
    }

    /// Gives an accepted original-join copy plan ownership of its predicted NOP slots.
    pub(super) fn consume_original_join_copy_prediction(&mut self) {
        assert!(self.is_original_empty_join());
        assert!(self.defers_redundant_removal());
        assert!(self.preserves_nop_after_threading());
        self.0 &= !(Self::DEFER_REDUNDANT_REMOVAL | Self::PRESERVE_NOP_AFTER_THREADING);
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
    /// Post-try marker stays excluded unless an original owned slot is certified.
    AlwaysAbsentPostTryCondition,
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
            | Self::AlwaysAbsentPostTryCondition
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
            Self::NoneAbsent
            | Self::IfExtendedAbsent
            | Self::AlwaysAbsent
            | Self::AlwaysAbsentPostTryCondition => NormalizedExceptionOwner::Absent,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
struct InstructionMetadata(NonZeroU16);

impl InstructionMetadata {
    const POLICY_MASK: u16 = 0b1_1111;
    const EXTRA_INSTRUCTIONS_SHIFT: u16 = 5;
    const EXTRA_INSTRUCTIONS_MASK: u16 = 0b111 << Self::EXTRA_INSTRUCTIONS_SHIFT;
    const UNCONDITIONAL_INLINING_ONLY: u16 = 1 << 8;
    const INITIAL_BORROW_ENTRY: u16 = 1 << 12;
    // Only the module annotation setup and its original folded jump carry this tag.
    const ORIGINAL_ANNOTATION_JUMP: u16 = 1 << 13;
    const RETIRED_CONTAINS: u16 = 1 << 15;
    const LATE_FOLD_LOCATION: u16 = 1 << 14;
    const LEGACY_JUMP_SEED_PRESENT: u16 = 1 << 9;
    const LEGACY_JUMP_SEED_SHIFT: u16 = 10;
    const LEGACY_JUMP_SEED_WIDTH: u16 = 3 << Self::LEGACY_JUMP_SEED_SHIFT;
    const LEGACY_JUMP_SEED: u16 = Self::LEGACY_JUMP_SEED_PRESENT | Self::LEGACY_JUMP_SEED_WIDTH;
    const FLAGS: u16 = Self::RETIRED_CONTAINS
        | Self::LATE_FOLD_LOCATION
        | Self::LEGACY_JUMP_SEED
        | Self::ORIGINAL_ANNOTATION_JUMP;

    const fn flags(self) -> u16 {
        self.0.get() & Self::FLAGS
    }

    const fn state(self) -> u16 {
        self.0.get() & !Self::FLAGS
    }

    fn with_flags(self, flags: u16) -> Self {
        Self(NonZeroU16::new(self.state() | flags).expect("instruction metadata reserves zero"))
    }

    // Only a direct original jump without earlier copied slots retains this proof.
    // Adding slots discards it without changing the other metadata fields.
    const fn has_initial_borrow_entry(self) -> bool {
        self.0.get() & Self::INITIAL_BORROW_ENTRY != 0
    }

    const fn normal_bits(self) -> u16 {
        self.0.get()
            & (Self::POLICY_MASK
                | Self::EXTRA_INSTRUCTIONS_MASK
                | Self::UNCONDITIONAL_INLINING_ONLY)
    }

    fn with_initial_borrow_entry(self, retain: bool) -> Self {
        let bits = self.0.get() & !Self::INITIAL_BORROW_ENTRY;
        let bits = if retain && self.pre_fold_extra_instructions() == 0 {
            bits | Self::INITIAL_BORROW_ENTRY
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
        // Store policy + 1 to reserve zero, leaving each field independently readable.
        let bits = (exception_policy as u16 + 1)
            | (u16::from(pre_fold_extra_instructions.min(4)) << Self::EXTRA_INSTRUCTIONS_SHIFT)
            | (u16::from(unconditional_inlining_only) * Self::UNCONDITIONAL_INLINING_ONLY);
        Self(NonZeroU16::new(bits).expect("exception policy reserves the zero discriminant"))
    }

    fn with_exception_policy(self, policy: ExceptionPolicy) -> Self {
        let bits = (self.0.get() & !Self::POLICY_MASK) | (policy as u16 + 1);
        Self(NonZeroU16::new(bits).expect("exception policy reserves the zero discriminant"))
    }

    fn with_added_pre_fold_extra_instructions(self, count: u8) -> Self {
        if count == 0 {
            return self;
        }
        let slots = self
            .pre_fold_extra_instructions()
            .saturating_add(count)
            .min(4);
        let bits = (self.0.get() & !(Self::EXTRA_INSTRUCTIONS_MASK | Self::INITIAL_BORROW_ENTRY))
            | (u16::from(slots) << Self::EXTRA_INSTRUCTIONS_SHIFT);
        Self(NonZeroU16::new(bits).expect("instruction metadata reserves zero"))
    }

    fn with_unconditional_inlining(self) -> Self {
        Self(self.0 | Self::UNCONDITIONAL_INLINING_ONLY)
    }

    const fn exception_policy(self) -> ExceptionPolicy {
        match (self.0.get() & Self::POLICY_MASK) - 1 {
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
            22 => ExceptionPolicy::AlwaysAbsentPostTryCondition,
            _ => unreachable!(),
        }
    }

    const fn pre_fold_extra_instructions(self) -> u8 {
        // Valid metadata reserves five slot counts; reject invalid encodings.
        match (self.0.get() & Self::EXTRA_INSTRUCTIONS_MASK) >> Self::EXTRA_INSTRUCTIONS_SHIFT {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            _ => unreachable!(),
        }
    }

    const fn only_allows_unconditional_inlining(self) -> bool {
        self.0.get() & Self::UNCONDITIONAL_INLINING_ONLY != 0
    }
}

impl Default for InstructionMetadata {
    fn default() -> Self {
        Self::new(ExceptionPolicy::default(), 0, false)
    }
}

/// First occurrences of the two Boolean objects in one original instruction owner.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub(super) enum EarlyBooleanAdmissions {
    #[default]
    None,
    True,
    False,
    TrueFalse,
    FalseTrue,
}

impl EarlyBooleanAdmissions {
    pub(super) const fn values(self) -> &'static [bool] {
        match self {
            Self::None => &[],
            Self::True => &[true],
            Self::False => &[false],
            Self::TrueFalse => &[true, false],
            Self::FalseTrue => &[false, true],
        }
    }

    pub(super) fn append(&mut self, value: bool) {
        *self = match (*self, value) {
            (Self::None, true) => Self::True,
            (Self::None, false) => Self::False,
            (Self::True, false) => Self::TrueFalse,
            (Self::False, true) => Self::FalseTrue,
            _ => *self,
        };
    }

    fn followed_by(mut self, other: Self) -> Self {
        for value in other.values() {
            self.append(*value);
        }
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
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
    pub(super) early_boolean_admissions: EarlyBooleanAdmissions,
    pub(super) folded_constant_admissions: Option<FoldAdmissionId>,
}

impl Instruction {
    const NO_DEPTH_AFTER: u32 = u32::MAX;

    /// Marks an instruction in the original module annotation and folded-jump pair.
    pub(super) fn mark_original_annotation_jump(&mut self) {
        self.metadata = self
            .metadata
            .with_flags(self.metadata.flags() | InstructionMetadata::ORIGINAL_ANNOTATION_JUMP);
    }

    pub(super) const fn is_original_annotation_jump(&self) -> bool {
        self.metadata.flags() & InstructionMetadata::ORIGINAL_ANNOTATION_JUMP != 0
    }

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
            early_boolean_admissions: EarlyBooleanAdmissions::default(),
            folded_constant_admissions: None,
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
            early_boolean_admissions: EarlyBooleanAdmissions::default(),
            folded_constant_admissions: None,
        }
    }

    /// Compares fields retained by a fusion-owner certificate, apart from its opcode policy.
    /// Initial jump borrowing is independent provenance and does not alter the owner proof.
    pub(super) fn same_fusion_owner_fields(&self, other: &Self) -> bool {
        self.operand_payload == other.operand_payload
            && self.location == other.location
            && self.depth_after == other.depth_after
            && self.local_policy == other.local_policy
            && self.no_location_policy == other.no_location_policy
            // The fusion policy includes the lossless operand kind as well as its own flags.
            && self.fusion_policy == other.fusion_policy
            && self.jump_policy == other.jump_policy
            && self.metadata.normal_bits() == other.metadata.normal_bits()
            && self.metadata.flags() == other.metadata.flags()
    }

    /// Retains the original membership operation while Boolean lowering moves it.
    pub(super) fn mark_retired_contains(&mut self) {
        self.metadata = self
            .metadata
            .with_flags(self.metadata.flags() | InstructionMetadata::RETIRED_CONTAINS);
    }

    pub(super) const fn is_retired_contains(&self) -> bool {
        self.metadata.flags() & InstructionMetadata::RETIRED_CONTAINS != 0
    }

    /// Defers the location of a late-folded constant until fused predecessors exist.
    pub(super) fn mark_late_fold_location(&mut self) {
        self.metadata = self
            .metadata
            .with_flags(self.metadata.flags() | InstructionMetadata::LATE_FOLD_LOCATION);
    }

    pub(super) const fn has_late_fold_location(&self) -> bool {
        self.metadata.flags() & InstructionMetadata::LATE_FOLD_LOCATION != 0
    }

    /// Freezes the width of an exact native 3.12 jump operand before offset resolution.
    ///
    /// Native jump threading changes the target pointer while retaining this operand.
    /// The offset solver reads only its encoded width before replacing it with a delta,
    /// so three independent metadata bits preserve the complete sizing input.
    pub(super) fn set_legacy_initial_jump_argument(&mut self, argument: u32) {
        debug_assert!(matches!(
            self.operand(),
            Operand::Forward(_) | Operand::Backward(_)
        ));
        let width = u16::from(super::extended_arg_count(argument));
        let flags = (self.metadata.flags() & !InstructionMetadata::LEGACY_JUMP_SEED)
            | InstructionMetadata::LEGACY_JUMP_SEED_PRESENT
            | (width << InstructionMetadata::LEGACY_JUMP_SEED_SHIFT);
        self.metadata = self.metadata.with_flags(flags);
    }

    /// Cancels a code object's incomplete native operand provenance as one transaction.
    pub(super) fn clear_legacy_initial_jump_argument(&mut self) {
        self.metadata = self
            .metadata
            .with_flags(self.metadata.flags() & !InstructionMetadata::LEGACY_JUMP_SEED);
    }

    /// Returns an exact native jump seed width, or none while provenance is incomplete.
    pub(super) const fn legacy_initial_jump_width(&self) -> Option<u8> {
        if self.metadata.flags() & InstructionMetadata::LEGACY_JUMP_SEED_PRESENT == 0 {
            None
        } else {
            Some(
                ((self.metadata.flags() & InstructionMetadata::LEGACY_JUMP_SEED_WIDTH)
                    >> InstructionMetadata::LEGACY_JUMP_SEED_SHIFT) as u8,
            )
        }
    }

    pub(super) const fn exception_policy(&self) -> ExceptionPolicy {
        self.metadata.exception_policy()
    }

    pub(super) fn set_exception_policy(&mut self, policy: ExceptionPolicy) {
        let metadata = self.metadata.with_exception_policy(policy);
        if metadata != self.metadata {
            self.jump_policy.retire_original_uniform_fold();
        }
        self.metadata = metadata;
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
        if count > 0 {
            // A positive addition is observable even when the four-slot budget
            // cannot change. It invalidates the original literal-only transaction.
            self.jump_policy.retire_original_uniform_fold();
        }
        self.metadata = self.metadata.with_added_pre_fold_extra_instructions(count);
    }

    pub(super) fn allow_unconditional_block_inlining(&mut self) {
        self.no_location_policy.allow_block_inlining();
        self.metadata = self.metadata.with_unconditional_inlining();
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

    /// Read a numeric argument without decoding unrelated symbolic operand variants.
    pub(super) const fn value_argument(&self) -> Option<u32> {
        if self.fusion_policy.0 & FusionPolicy::OPERAND_KIND_MASK == FusionPolicy::VALUE_OPERAND {
            Some(self.operand_payload)
        } else {
            None
        }
    }

    /// Both jump directions share the high operand-kind bit.
    pub(super) const fn jump_target(&self) -> Option<Label> {
        if self.fusion_policy.0 & FusionPolicy::FORWARD_OPERAND != 0 {
            Some(Label::from_u32(self.operand_payload))
        } else {
            None
        }
    }

    pub(super) const fn forward_target(&self) -> Option<Label> {
        if self.fusion_policy.0 & FusionPolicy::OPERAND_KIND_MASK == FusionPolicy::FORWARD_OPERAND {
            Some(Label::from_u32(self.operand_payload))
        } else {
            None
        }
    }

    pub(super) fn set_operand(&mut self, operand: Operand) {
        if self.operand() != operand {
            self.jump_policy.retire_original_uniform_fold();
            self.jump_policy.retire_original_match_success();
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

    /// Retires source-only literal-fold provenance on a physical copy into another block.
    pub(super) fn for_exit_copy(mut self) -> Self {
        self.jump_policy.retire_original_uniform_fold();
        self.metadata = self
            .metadata
            .with_flags(self.metadata.flags() & !InstructionMetadata::ORIGINAL_ANNOTATION_JUMP);
        self
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
            )
            .with_flags(
                (first.metadata.flags() | second.metadata.flags())
                    & !InstructionMetadata::ORIGINAL_ANNOTATION_JUMP,
            ),
            early_boolean_admissions: first
                .early_boolean_admissions
                .followed_by(second.early_boolean_admissions),
            // Constant admissions are consumed before superinstruction fusion.
            folded_constant_admissions: None,
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

/// An exact instruction that keeps an original normal-cleanup entry nonempty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BorrowCleanupCall {
    pub(super) opcode: Opcode,
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
    pub(super) python_version: PythonVersion,
    /// A source proof bounds every native 3.12 jump's initial operand to one byte.
    /// Independent of exact per-label provenance and its cancellation paths.
    pub(super) legacy_uniform_jump_seed: bool,
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
    /// Exact folded arm jumps awaiting the complete original return destination.
    pub(super) folded_arm_exit_origins: Vec<(Label, Instruction)>,
    /// Empty original anchors before folded constant-while exits without else suites.
    pub(super) folded_loop_exit_anchors: FxHashSet<Label>,
    pub(super) operand_patches: Vec<OperandPatch>,
    pub(super) deferred_none_constant_index: Option<u32>,
    pub(super) discarded_fold_admissions: FxHashMap<Label, Vec<u32>>,
    pub(super) folded_constant_admissions: Vec<Vec<u32>>,
    pub(super) early_boolean_admissions_certified: bool,
    pub(super) has_early_boolean_admissions: bool,
    pub(super) next_label: u32,
    pub(super) legacy_native_labels: Option<Box<super::legacy_native_labels::LegacyNativeLabels>>,
    pub(super) location: SourceLocation,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    /// Original tail location retained when lowering emits an already-reordered restore.
    pub(super) original_fallthrough_location: Option<(EmittedInstruction, SourceLocation)>,
    /// Source-authored match end awaiting its first emitted instruction.
    pub(super) original_match_join: Option<Label>,
    /// Synchronous for-else end labels and their producer stack depth.
    pub(super) original_sync_for_else_ends: FxHashMap<Label, u32>,
    /// Keeps this large state in place as the assembler changes optimization phases.
    pub(super) exception_owner_provenance: super::exception_owners::LazyExceptionOwnerProvenance,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) borrow_unreachable_blocks: FxHashSet<Label>,
    /// Cleanup targets whose original for-loop backedge array was omitted during lowering.
    pub(super) borrow_omitted_for_backedges: FxHashSet<Label>,
    /// Entries whose empty original fallthrough can be bypassed by a surviving jump.
    pub(super) borrow_jump_entry_labels: FxHashSet<Label>,
    pub(super) deferred_borrow_entries: Vec<DeferredBorrowEntry>,
    /// Normal edges whose following local return must prove its original copy width.
    pub(super) deferred_name_return_borrow_exits: Vec<super::NameReturnBorrowExit>,
    /// Source edges awaiting a located target larger than the native exit-copy budget.
    pub(super) deferred_noncopyable_borrow_exits: Vec<(EmittedJump, Instruction)>,
    /// Exact omitted ordinary-try POP slots in pending local-return targets.
    pub(super) name_return_pop_boundaries: FxHashSet<Label>,
    /// Ordinary-return `POP_BLOCK` records needed by the legacy CFG copy phases.
    pub(super) legacy_return_pop_blocks: FxHashMap<Label, usize>,
    /// Iterator-region aliases immediately following an ordinary-return POP boundary.
    pub(super) name_return_iterator_region_aliases: FxHashMap<Label, Label>,
    /// Disconnected loop exits and the normal entries that remain reachable.
    pub(super) borrow_loop_exit_blocks: FxHashMap<Label, BorrowLoopExitPolicy>,
    /// Normal finally entries whose original nonempty cleanup can retain an explicit jump.
    pub(super) borrow_finally_entries: FxHashMap<Label, Option<BorrowCleanupCall>>,
    pub(super) borrow_block_aliases: FxHashSet<Label>,
    /// Original ordinary jump blocks that can thread conditional predecessors after exit copying.
    pub(super) conditional_threading_targets: FxHashMap<Label, SourceLocation>,
    /// Exact pre-swap comprehension assignment arrays and their shared match exits.
    pub(super) comprehension_assignment_exits: FxHashMap<Label, (Vec<Label>, Vec<Instruction>)>,
    /// Original match fallthrough arrays ending in an ordinary two-store exchange.
    pub(super) match_join_store_origins: FxHashMap<Label, Vec<Item>>,
    /// Labels preceding an authored NOP omitted during statement lowering.
    pub(super) original_nop_targets: FxHashSet<Label>,
    /// Source-certified scope exits, separate from eager branch-return footers.
    pub(super) original_scope_exit_labels: FxHashSet<Label>,
    /// Normal handler cleanup originally emitted without source locations.
    pub(super) original_unlocated_cleanup_targets: FxHashSet<Label>,
    /// Exact source-certified named cleanup spans, used only during statement emission.
    pub(super) guarded_finally_handler_cleanup_origins: Vec<(usize, Vec<Item>)>,
    /// Unlocated Boolean discards with a fallback whose source instruction folded away.
    pub(super) original_unlocated_discard_targets: FxHashSet<Label>,
    /// Complete emitted conditional-finalizer arrays with a shared iterator discard.
    pub(super) copied_finally_iterator_break_origins: Vec<Vec<Item>>,
    pub(super) copied_finally_conjunction_break_origins: Vec<Vec<Item>>,
    /// Complete copied cleanup-try arrays certified at their original shared iterator join.
    pub(super) shared_finally_iterator_break_origins: Vec<Vec<Item>>,
    /// Original shared jump blocks whose source locations are resolved after exit copying.
    pub(super) deferred_shared_jump_blocks: FxHashSet<Label>,
    /// Original label-only prefixes of retained normal-finally `POP_BLOCK` instructions.
    pub(super) retained_pop_block_origins: FxHashMap<Label, Vec<Label>>,
    /// Shared jumps whose copies can acquire their new predecessor's source location.
    pub(super) shared_jump_copy_location_blocks: FxHashSet<Label>,
    /// Original nested joins awaiting a source-authored or threaded eval-check exit.
    pub(super) nested_if_unlocated_join_blocks: FxHashSet<Label>,
    /// Unique joins reached by the synthetic exits of early-folded conditions.
    pub(super) folded_condition_exit_targets: FxHashSet<Label>,
    pub(super) load_fast_borrowing_enabled: bool,
    pub(super) strict_owned_loads: bool,
    pub(super) forced_borrowed_loads: bool,
    pub(super) has_constant_pop_candidate: bool,
    pub(super) has_assertion_side_exit_candidate: bool,
    pub(super) has_proven_linear_assertion_side_exits: bool,
    pub(super) has_proven_linear_terminal_guards: bool,
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
    pub(super) python_version: PythonVersion,
    /// A source proof bounds every native 3.12 jump's initial operand to one byte.
    /// Independent of exact per-label provenance and its cancellation paths.
    pub(super) legacy_uniform_jump_seed: bool,
    pub(super) graph: BlockGraph,
    /// Original incoming edges removed before CPython resolves cleanup locations.
    pub(super) folded_original_predecessor_targets: FxHashSet<Label>,
    /// Source markers retained to prove that a folded edge was originally reachable.
    pub(super) folded_predecessor_sources: Vec<(Label, Label, SourceLocation)>,
    /// Empty original anchors before folded constant-while exits without else suites.
    pub(super) folded_loop_exit_anchors: FxHashSet<Label>,
    /// First-fold admissions retained before their next surviving source label.
    pub(super) discarded_fold_admissions: FxHashMap<Label, Vec<u32>>,
    pub(super) folded_constant_admissions: Vec<Vec<u32>>,
    pub(super) early_boolean_admissions_certified: bool,
    pub(super) has_early_boolean_admissions: bool,
    pub(super) initial_reachable: Option<Vec<bool>>,
    /// Erased handler entries must not mark a later block reached through label aliasing.
    pub(super) retired_exception_handler_targets: FxHashSet<Label>,
    pub(super) next_label: u32,
    pub(super) legacy_native_labels: Option<Box<super::legacy_native_labels::LegacyNativeLabels>>,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) exception_owner_provenance: Box<super::exception_owners::ExceptionOwnerProvenance>,
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
    /// Exact pre-swap comprehension assignment arrays and their shared match exits.
    pub(super) comprehension_assignment_exits: FxHashMap<Label, (Vec<Label>, Vec<Instruction>)>,
    /// Original match fallthrough arrays ending in an ordinary two-store exchange.
    pub(super) match_join_store_origins: FxHashMap<Label, Vec<Item>>,
    /// Labels preceding an authored NOP omitted during statement lowering.
    pub(super) original_nop_targets: FxHashSet<Label>,
    /// Source-certified scope exits, separate from eager branch-return footers.
    pub(super) original_scope_exit_labels: FxHashSet<Label>,
    /// Normal handler cleanup originally emitted without source locations.
    pub(super) original_unlocated_cleanup_targets: FxHashSet<Label>,
    /// Unlocated Boolean discards with a fallback whose source instruction folded away.
    pub(super) original_unlocated_discard_targets: FxHashSet<Label>,
    /// Complete emitted conditional-finalizer arrays with a shared iterator discard.
    pub(super) copied_finally_iterator_break_origins: Vec<Vec<Item>>,
    pub(super) copied_finally_conjunction_break_origins: Vec<Vec<Item>>,
    /// Complete copied cleanup-try arrays certified at their original shared iterator join.
    pub(super) shared_finally_iterator_break_origins: Vec<Vec<Item>>,
    /// Conditional entry labels mapped to their shared original unlocated jump array.
    pub(super) original_shared_unlocated_jump_entries: FxHashMap<Label, Label>,
    /// Original ordinary-jump heads, separate from synthetic source-location joins.
    pub(super) original_jump_head_targets: FxHashSet<Label>,
    pub(super) deferred_shared_jump_blocks: FxHashSet<Label>,
    /// Original label-only prefixes of retained normal-finally `POP_BLOCK` instructions.
    pub(super) retained_pop_block_origins: FxHashMap<Label, Vec<Label>>,
    pub(super) shared_jump_copy_location_blocks: FxHashSet<Label>,
    /// Emitted nested joins consumed when their exact jump is threaded to an eval-check exit.
    pub(super) nested_if_unlocated_join_blocks: FxHashSet<Label>,
    /// Unique joins reached by the synthetic exits of early-folded conditions.
    pub(super) folded_condition_exit_targets: FxHashSet<Label>,
    pub(super) has_assertion_side_exit_candidate: bool,
    pub(super) has_synthesized_percent_target_locations: bool,
    pub(super) has_next_location: bool,
}

/// The linear assembler state used after CFG optimization is complete.
#[derive(Debug)]
pub(super) struct LayoutAssembler {
    pub(super) python_version: PythonVersion,
    /// A source proof bounds every native 3.12 jump's initial operand to one byte.
    /// Independent of exact per-label provenance and its cancellation paths.
    pub(super) legacy_uniform_jump_seed: bool,
    pub(super) items: Vec<Item>,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) exception_owner_provenance: super::exception_owners::LazyExceptionOwnerProvenance,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
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
            python_version: PythonVersion::default(),
            legacy_uniform_jump_seed: false,
            code: UnoptimizedCode::Linear(Vec::new()),
            instruction_count: 0,
            emission_stream: EmissionStreamId::next(),
            folded_away_condition_targets: FxHashSet::default(),
            folded_away_loop_else_targets: FxHashSet::default(),
            folded_predecessor_sources: Vec::new(),
            folded_arm_exit_origins: Vec::new(),
            folded_loop_exit_anchors: FxHashSet::default(),
            operand_patches: Vec::new(),
            deferred_none_constant_index: None,
            discarded_fold_admissions: FxHashMap::default(),
            folded_constant_admissions: Vec::new(),
            early_boolean_admissions_certified: false,
            has_early_boolean_admissions: false,
            next_label: 0,
            legacy_native_labels: None,
            location: SourceLocation::NONE,
            exception_regions: Vec::new(),
            original_fallthrough_location: None,
            original_match_join: None,
            original_sync_for_else_ends: FxHashMap::default(),
            exception_owner_provenance:
                crate::assembler::exception_owners::LazyExceptionOwnerProvenance::default(),
            preserved_block_boundaries: FxHashSet::default(),
            borrow_unreachable_blocks: FxHashSet::default(),
            borrow_omitted_for_backedges: FxHashSet::default(),
            borrow_jump_entry_labels: FxHashSet::default(),
            deferred_borrow_entries: Vec::new(),
            deferred_name_return_borrow_exits: Vec::new(),
            deferred_noncopyable_borrow_exits: Vec::new(),
            name_return_pop_boundaries: FxHashSet::default(),
            legacy_return_pop_blocks: FxHashMap::default(),
            name_return_iterator_region_aliases: FxHashMap::default(),
            borrow_loop_exit_blocks: FxHashMap::default(),
            borrow_finally_entries: FxHashMap::default(),
            borrow_block_aliases: FxHashSet::default(),
            conditional_threading_targets: FxHashMap::default(),
            comprehension_assignment_exits: FxHashMap::default(),
            match_join_store_origins: FxHashMap::default(),
            original_nop_targets: FxHashSet::default(),
            original_scope_exit_labels: FxHashSet::default(),
            original_unlocated_cleanup_targets: FxHashSet::default(),
            guarded_finally_handler_cleanup_origins: Vec::new(),
            original_unlocated_discard_targets: FxHashSet::default(),
            copied_finally_iterator_break_origins: Vec::new(),
            copied_finally_conjunction_break_origins: Vec::new(),
            shared_finally_iterator_break_origins: Vec::new(),
            load_fast_borrowing_enabled: true,
            strict_owned_loads: false,
            forced_borrowed_loads: false,
            has_constant_pop_candidate: false,
            has_assertion_side_exit_candidate: false,
            has_proven_linear_assertion_side_exits: false,
            has_proven_linear_terminal_guards: false,
            has_synthesized_percent_target_locations: false,
            has_next_location: false,
            deferred_shared_jump_blocks: FxHashSet::default(),
            retained_pop_block_origins: FxHashMap::default(),
            shared_jump_copy_location_blocks: FxHashSet::default(),
            nested_if_unlocated_join_blocks: FxHashSet::default(),
            folded_condition_exit_targets: FxHashSet::default(),
        }
    }
}

#[cfg(test)]
mod initial_borrow_metadata_tests {
    use super::{
        ExceptionPolicy, Instruction, InstructionMetadata, Label, NonZeroU16, Operand,
        OperandPatchId, SourceLocation,
    };
    use crate::target::opcodes::{JUMP_FORWARD, LOAD_FAST};

    const POLICIES: [ExceptionPolicy; 23] = [
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
        ExceptionPolicy::AlwaysAbsentPostTryCondition,
    ];

    fn metadata_encodings() -> Vec<u16> {
        let mut encodings = Vec::with_capacity(276);
        for policy in 1..=23 {
            for slots in 0..=4 {
                for copy in [false, true] {
                    let raw = policy | (slots << 5) | (u16::from(copy) << 8);
                    encodings.push(raw);
                    if slots == 0 {
                        encodings.push(raw | (1 << 12));
                    }
                }
            }
        }
        encodings
    }

    #[test]
    fn specialized_operand_accessors_preserve_all_kinds_and_policy_bits() {
        for flags in 0..=u8::MAX {
            for payload in [0, 1, 255, 65_535, u32::MAX - 1] {
                let mut instruction =
                    Instruction::value_with_depth(LOAD_FAST, payload, SourceLocation::NONE, 0);
                instruction.fusion_policy = super::FusionPolicy(flags);
                assert_eq!(
                    instruction.value_argument(),
                    match instruction.operand() {
                        Operand::Value(value) => Some(value),
                        _ => None,
                    }
                );
                assert_eq!(
                    instruction.forward_target(),
                    match instruction.operand() {
                        Operand::Forward(label) => Some(label),
                        _ => None,
                    }
                );
                assert_eq!(
                    instruction.jump_target(),
                    match instruction.operand() {
                        Operand::Forward(label) | Operand::Backward(label) => Some(label),
                        _ => None,
                    }
                );
            }
        }
        // A numeric argument can use all 32 bits, including the reserved label value.
        let maximum = Instruction::value_with_depth(LOAD_FAST, u32::MAX, SourceLocation::NONE, 0);
        assert_eq!(maximum.value_argument(), Some(u32::MAX));
        assert_eq!(maximum.jump_target(), None);
    }

    #[test]
    fn fusion_owner_fields_preserve_every_normal_metadata_combination() {
        let original = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(Label::from_u32(1)),
            SourceLocation::NONE,
            None,
        );
        let mut same_count = 0;
        let encodings = metadata_encodings();
        for &first in &encodings {
            let mut a = original;
            a.metadata = InstructionMetadata(NonZeroU16::new(first).unwrap());
            for &second in &encodings {
                let mut b = original;
                b.metadata = InstructionMetadata(NonZeroU16::new(second).unwrap());
                let expected = a.exception_policy() == b.exception_policy()
                    && a.pre_fold_extra_instructions() == b.pre_fold_extra_instructions()
                    && a.only_allows_unconditional_inlining()
                        == b.only_allows_unconditional_inlining();
                assert_eq!(
                    a.same_fusion_owner_fields(&b),
                    expected,
                    "{first}, {second}"
                );
                same_count += usize::from(expected);
            }
        }
        // 46 owner states have both a tagged and an untagged encoding; 184 have one.
        assert_eq!(same_count, 46 * 4 + 184);
    }

    #[test]
    fn fusion_owner_fields_compare_packed_operands_depth_and_other_policies() {
        let source = SourceLocation::new(1, 1, 0, 1);
        let original = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(Label::from_u32(1)),
            source,
            None,
        );
        let mut changes = [original; 7];
        changes[0].set_operand(Operand::Forward(Label::from_u32(2)));
        changes[1].location = SourceLocation::new(2, 2, 0, 1);
        changes[2].depth_after = 0;
        changes[3].local_policy.mark_force_borrowed_load();
        changes[4].no_location_policy.percent_target_location = 3;
        changes[5].fusion_policy.prevent_with_previous();
        changes[6].jump_policy.mark_interruptible();
        for changed in changes {
            assert!(!original.same_fusion_owner_fields(&changed));
            assert!(!changed.same_fusion_owner_fields(&original));
        }
        let mut another_opcode = original;
        another_opcode.opcode = LOAD_FAST;
        assert!(original.same_fusion_owner_fields(&another_opcode));

        let operands = [
            Operand::Value(1),
            Operand::Deferred(OperandPatchId::from_u32(1)),
            Operand::Forward(Label::from_u32(1)),
            Operand::Backward(Label::from_u32(1)),
        ];
        for first in operands {
            for second in operands {
                let a = Instruction::new(JUMP_FORWARD, first, source, None);
                let b = Instruction::new(JUMP_FORWARD, second, source, None);
                assert_eq!(a.same_fusion_owner_fields(&b), first == second);
            }
        }
        let depths = [None, Some(0), Some(1), Some(u32::MAX - 1)];
        for first in depths {
            for second in depths {
                let a = Instruction::new(JUMP_FORWARD, operands[0], source, first);
                let b = Instruction::new(JUMP_FORWARD, operands[0], source, second);
                assert_eq!(a.same_fusion_owner_fields(&b), first == second);
            }
        }
    }

    #[test]
    fn constant_fold_provenance_survives_metadata_updates() {
        for raw in metadata_encodings() {
            let original = InstructionMetadata(NonZeroU16::new(raw).unwrap());
            let mut instruction =
                Instruction::new(LOAD_FAST, Operand::Value(0), SourceLocation::NONE, None);
            instruction.metadata = original;
            instruction.mark_retired_contains();
            instruction.mark_late_fold_location();
            assert_eq!(instruction.exception_policy(), original.exception_policy());
            assert_eq!(
                instruction.pre_fold_extra_instructions(),
                original.pre_fold_extra_instructions()
            );
            assert_eq!(
                instruction.metadata.has_initial_borrow_entry(),
                original.has_initial_borrow_entry()
            );
            instruction.set_exception_policy(ExceptionPolicy::NonePresent);
            instruction.add_pre_fold_extra_instructions(1);
            instruction.allow_unconditional_block_inlining();
            instruction.metadata = instruction.metadata.with_initial_borrow_entry(false);
            assert!(instruction.is_retired_contains());
            assert!(instruction.has_late_fold_location());
            assert_eq!(instruction.exception_policy(), ExceptionPolicy::NonePresent);
            assert_eq!(
                instruction.pre_fold_extra_instructions(),
                (original.pre_fold_extra_instructions() + 1).min(4)
            );
        }
    }

    #[test]
    fn native_jump_seed_survives_pointer_retargeting_and_policy_changes() {
        for argument in [0, 255, 256, 65_535, 65_536, 16_777_215, 16_777_216] {
            for raw in metadata_encodings() {
                let mut original = Instruction::synthetic(
                    JUMP_FORWARD,
                    Operand::Forward(Label::from_u32(1)),
                    SourceLocation::NONE,
                    None,
                );
                original.metadata = InstructionMetadata(NonZeroU16::new(raw).unwrap());
                original.mark_retired_contains();
                original.mark_late_fold_location();
                original.set_legacy_initial_jump_argument(argument);
                let width = Some(crate::assembler::extended_arg_count(argument));
                assert_eq!(original.legacy_initial_jump_width(), width);
                for policy in POLICIES {
                    let mut retargeted = original;
                    retargeted.retarget(Operand::Forward(Label::from_u32(999)));
                    retargeted.set_exception_policy(policy);
                    retargeted.add_pre_fold_extra_instructions(1);
                    assert_eq!(retargeted.legacy_initial_jump_width(), width);
                    assert!(retargeted.is_retired_contains());
                    assert!(retargeted.has_late_fold_location());
                    assert_eq!(retargeted.exception_policy(), policy);
                }
                // Empty-block elimination is an explicit native operand rewrite.
                original.set_legacy_initial_jump_argument(0);
                assert_eq!(original.legacy_initial_jump_width(), Some(0));
            }
        }
    }

    #[test]
    fn bit_field_metadata_preserves_all_276_states() {
        assert_eq!(size_of::<InstructionMetadata>(), 2);
        assert_eq!(size_of::<Option<InstructionMetadata>>(), 2);
        let mut encodings = metadata_encodings();
        encodings.sort_unstable();
        encodings.dedup();
        assert_eq!(encodings.len(), 276);
        assert!(!encodings.contains(&0));
        for raw in encodings {
            let value = InstructionMetadata(NonZeroU16::new(raw).unwrap());
            let origin = raw & (1 << 12) != 0;
            assert_eq!(value.has_initial_borrow_entry(), origin);
            assert_eq!(
                value.exception_policy(),
                POLICIES[usize::from((raw & 31) - 1)]
            );
            assert_eq!(
                u16::from(value.pre_fold_extra_instructions()),
                (raw >> 5) & 7
            );
            assert_eq!(
                value.only_allows_unconditional_inlining(),
                raw & (1 << 8) != 0
            );
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
            let marked = value.with_initial_borrow_entry(true);
            assert_eq!(
                marked.has_initial_borrow_entry(),
                value.pre_fold_extra_instructions() == 0
            );
            assert_eq!(marked.normal_bits(), value.normal_bits());
            let cleared = value.with_initial_borrow_entry(false);
            assert_eq!(cleared.0.get(), raw & !(1 << 12));
            assert_eq!(cleared.exception_policy(), value.exception_policy());
            assert_eq!(
                cleared.pre_fold_extra_instructions(),
                value.pre_fold_extra_instructions()
            );
            assert_eq!(
                cleared.only_allows_unconditional_inlining(),
                value.only_allows_unconditional_inlining()
            );
        }
    }

    #[test]
    fn wide_metadata_preserves_policy_updates_and_invalidates_changed_origins() {
        for raw in metadata_encodings() {
            let metadata = InstructionMetadata(NonZeroU16::new(raw).unwrap());
            let mut original = Instruction::synthetic(
                JUMP_FORWARD,
                Operand::Forward(Label::from_u32(1)),
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
            retargeted.set_operand(Operand::Forward(Label::from_u32(1)));
            assert_eq!(retargeted.metadata.has_initial_borrow_entry(), origin);
            retargeted.set_operand(Operand::Forward(Label::from_u32(2)));
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
