use rustc_hash::FxHashSet;

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
pub(super) struct LoadPolicy {
    pub(super) force_owned: bool,
    pub(super) strict_owned: bool,
}

impl LoadPolicy {
    const fn fused(first: Self, second: Self) -> Self {
        Self {
            force_owned: first.force_owned || second.force_owned,
            strict_owned: first.strict_owned || second.strict_owned,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum NoLocationBlockInlining {
    #[default]
    Disabled,
    Allowed,
    FirstCopyInterruptible,
}

impl NoLocationBlockInlining {
    const fn fused(first: Self, second: Self) -> Self {
        match (first, second) {
            (Self::FirstCopyInterruptible, _) | (_, Self::FirstCopyInterruptible) => {
                Self::FirstCopyInterruptible
            }
            (Self::Allowed, _) | (_, Self::Allowed) => Self::Allowed,
            (Self::Disabled, Self::Disabled) => Self::Disabled,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NoLocationPolicy {
    /// Whether an unlocated block ending at this instruction may be inlined.
    block_inlining: NoLocationBlockInlining,
    pub(super) preserve: bool,
    /// An unresolved percent target or its deferred-location side-table index.
    pub(super) percent_target_location: u8,
}

impl NoLocationPolicy {
    pub(super) const fn allows_block_inlining(self) -> bool {
        !matches!(self.block_inlining, NoLocationBlockInlining::Disabled)
    }

    /// Whether a copied forward exit becomes an interruptible backward jump after cold-block
    /// ordering. The original cleanup edge remains non-interruptible.
    pub(super) const fn first_copy_is_interruptible_backward(self) -> bool {
        matches!(
            self.block_inlining,
            NoLocationBlockInlining::FirstCopyInterruptible
        )
    }

    pub(super) fn allow_block_inlining(&mut self) {
        if matches!(self.block_inlining, NoLocationBlockInlining::Disabled) {
            self.block_inlining = NoLocationBlockInlining::Allowed;
        }
    }

    pub(super) fn make_first_copy_interruptible(&mut self) {
        debug_assert!(self.allows_block_inlining());
        self.block_inlining = NoLocationBlockInlining::FirstCopyInterruptible;
    }

    const fn fused(first: Self, second: Self) -> Self {
        Self {
            block_inlining: NoLocationBlockInlining::fused(
                first.block_inlining,
                second.block_inlining,
            ),
            preserve: first.preserve || second.preserve,
            percent_target_location: if matches!(first.percent_target_location, 0 | u8::MAX) {
                second.percent_target_location
            } else {
                first.percent_target_location
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FusionPolicy {
    pub(super) prevent_with_previous: bool,
    pub(super) prevent_with_next: bool,
}

impl FusionPolicy {
    const fn fused(first: Self, second: Self) -> Self {
        Self {
            prevent_with_previous: first.prevent_with_previous,
            prevent_with_next: second.prevent_with_next,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum BorrowFallthrough {
    #[default]
    Reachable,
    Barrier,
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
pub(super) struct JumpPolicy(u8);

impl JumpPolicy {
    const RETAINED_NOP_MASK: u8 = 0b11;
    const RETAINED_NOP_ALWAYS: u8 = 0b01;
    const RETAINED_NOP_DIRECT_TARGET: u8 = 0b10;
    const PREVENT_SMALL_EXIT_INLINING: u8 = 1 << 2;
    const PREVENT_THREADING_TARGET: u8 = 1 << 3;
    const DEFER_REDUNDANT_REMOVAL: u8 = 1 << 4;
    const PRESERVE_NOP_AFTER_THREADING: u8 = 1 << 5;
    const BORROW_EMPTY_BLOCK: u8 = 1 << 6;
    const PRESERVE_EXPLICIT_NOP: u8 = 1 << 7;

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
        policy
    }

    pub(super) const fn inline_small_exit(self) -> bool {
        self.0 & Self::PREVENT_SMALL_EXIT_INLINING == 0
    }

    pub(super) const fn allows_threading_target(self) -> bool {
        self.0 & Self::PREVENT_THREADING_TARGET == 0
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
            0 => RetainedJumpNop::None,
            Self::RETAINED_NOP_ALWAYS => RetainedJumpNop::Always,
            Self::RETAINED_NOP_DIRECT_TARGET => RetainedJumpNop::DirectTarget,
            _ => unreachable!(),
        }
    }

    pub(super) const fn always_retains_nop(self) -> bool {
        matches!(self.retained_nop(), RetainedJumpNop::Always)
    }

    pub(super) const fn retains_direct_target_nop(self) -> bool {
        matches!(self.retained_nop(), RetainedJumpNop::DirectTarget)
    }

    pub(super) fn retains_nop_for(self, operand: Operand) -> bool {
        match (self.retained_nop(), operand) {
            (RetainedJumpNop::Always, _) => true,
            (RetainedJumpNop::DirectTarget, Operand::Forward(_)) => true,
            (RetainedJumpNop::None | RetainedJumpNop::DirectTarget, _) => false,
        }
    }

    pub(super) fn retain_nop(&mut self) {
        self.0 = (self.0 & !Self::RETAINED_NOP_MASK) | Self::RETAINED_NOP_ALWAYS;
    }

    pub(super) fn retain_nop_for_direct_target(&mut self) {
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

    pub(super) fn resolve_deferred_removal(&mut self) {
        debug_assert!(self.defers_redundant_removal());
        self.0 &= !Self::DEFER_REDUNDANT_REMOVAL;
        if self.preserves_nop_after_threading() {
            self.0 &= !Self::PRESERVE_NOP_AFTER_THREADING;
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
            .field("allow_threading_target", &self.allows_threading_target())
            .field("defer_redundant_removal", &self.defers_redundant_removal())
            .field(
                "preserve_nop_after_threading",
                &self.preserves_nop_after_threading(),
            )
            .field("borrow_empty_block", &self.borrow_empty_block())
            .field("preserve_explicit_nop", &self.preserves_explicit_nop())
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
    Absent,
}

impl NormalizedExceptionOwner {
    const fn merged(self, other: Self) -> Self {
        if matches!(self, Self::Unchanged) {
            other
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExceptionPolicy {
    pub(super) exclusion: ExceptionExclusion,
    pub(super) normalized_owner: NormalizedExceptionOwner,
}

impl ExceptionPolicy {
    pub(crate) const EXCLUDE_IF_EXTENDED: Self = Self {
        exclusion: ExceptionExclusion::IfExtended,
        normalized_owner: NormalizedExceptionOwner::Unchanged,
    };

    pub(crate) const EXCLUDE_ALWAYS: Self = Self {
        exclusion: ExceptionExclusion::Always,
        normalized_owner: NormalizedExceptionOwner::Unchanged,
    };

    pub(crate) const fn normalized_owner(has_owner: bool) -> Self {
        Self {
            exclusion: ExceptionExclusion::None,
            normalized_owner: if has_owner {
                NormalizedExceptionOwner::Present
            } else {
                NormalizedExceptionOwner::Absent
            },
        }
    }

    const fn merged(self, other: Self) -> Self {
        Self {
            exclusion: self.exclusion.merged(other.exclusion),
            normalized_owner: self.normalized_owner.merged(other.normalized_owner),
        }
    }

    pub(super) fn exclude(&mut self, exclusion: ExceptionExclusion) {
        self.exclusion = self.exclusion.merged(exclusion);
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Instruction {
    pub(super) opcode: Opcode,
    pub(super) operand: Operand,
    pub(super) location: SourceLocation,
    // CPython resolves `NEXT_LOCATION` only after CFG optimization and instruction reordering.
    // Keep that provenance separate from the provisional source location used by earlier passes.
    pub(super) location_from_next: bool,
    pub(super) depth_after: Option<u32>,
    pub(super) load_policy: LoadPolicy,
    pub(super) no_location_policy: NoLocationPolicy,
    pub(super) fusion_policy: FusionPolicy,
    pub(super) converted_pop_block: bool,
    // Instruction slots removed by compiler-side constant folding. The count is capped at the
    // three slots that a normalized boolean value marker can reuse.
    pub(super) hidden_exception_owner_slots: u8,
    pub(super) jump_policy: JumpPolicy,
    // `NOT_TAKEN` is added after CPython labels exception handlers. The new instruction keeps
    // whatever exception target remains in its reused CFG slot, if there is one.
    pub(super) exception_policy: ExceptionPolicy,
    // A boundary used only by the final borrowed-load analysis. Earlier CFG passes must continue
    // to see ordinary control-flow reachability across this edge.
    pub(super) borrow_fallthrough: BorrowFallthrough,
}

impl Instruction {
    pub(super) fn new(
        opcode: Opcode,
        operand: Operand,
        location: SourceLocation,
        depth_after: Option<u32>,
    ) -> Self {
        Self {
            opcode,
            operand,
            location,
            location_from_next: false,
            depth_after,
            load_policy: LoadPolicy::default(),
            no_location_policy: NoLocationPolicy::default(),
            fusion_policy: FusionPolicy::default(),
            converted_pop_block: false,
            hidden_exception_owner_slots: 0,
            jump_policy: JumpPolicy::default(),
            exception_policy: ExceptionPolicy::default(),
            borrow_fallthrough: BorrowFallthrough::default(),
        }
    }

    /// Retargets a jump while keeping direct-target line-event provenance honest.
    pub(super) fn retarget(&mut self, operand: Operand) {
        debug_assert!(matches!(
            operand,
            Operand::Forward(_) | Operand::Backward(_)
        ));
        if self.operand != operand {
            self.jump_policy.clear_direct_target_retention();
            self.operand = operand;
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
        let (location, location_from_next) = if first.location_from_next {
            (second.location, second.location_from_next)
        } else {
            (first.location, false)
        };
        Self {
            opcode,
            operand,
            location,
            location_from_next,
            depth_after: second.depth_after,
            load_policy: LoadPolicy::fused(first.load_policy, second.load_policy),
            no_location_policy: NoLocationPolicy::fused(
                first.no_location_policy,
                second.no_location_policy,
            ),
            fusion_policy: FusionPolicy::fused(first.fusion_policy, second.fusion_policy),
            converted_pop_block: first.converted_pop_block || second.converted_pop_block,
            hidden_exception_owner_slots: first
                .hidden_exception_owner_slots
                .saturating_add(second.hidden_exception_owner_slots)
                .min(3),
            jump_policy: JumpPolicy::fused(first.jump_policy, second.jump_policy),
            exception_policy: first.exception_policy.merged(second.exception_policy),
            borrow_fallthrough: second.borrow_fallthrough,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Item {
    Instruction(Instruction),
    Label(Label),
}

#[derive(Debug)]
pub(super) enum UnoptimizedCode {
    /// Instructions are still being emitted and patched by the compiler.
    Linear(Vec<Item>),
    /// Constant reachability has prepared the graph that the CFG pipeline will consume.
    Prepared(BlockGraph),
}

#[derive(Debug)]
pub(crate) struct Assembler {
    pub(super) code: UnoptimizedCode,
    pub(super) operand_patches: Vec<OperandPatch>,
    pub(super) next_label: u32,
    pub(super) location: SourceLocation,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) borrow_unreachable_blocks: FxHashSet<Label>,
    pub(super) load_fast_borrowing_enabled: bool,
    pub(super) strict_owned_loads: bool,
    pub(super) has_synthesized_percent_target_locations: bool,
}

/// The block-aware assembler state used by CFG optimization passes.
///
/// Keeping this separate from [`Assembler`] makes it impossible to run a CFG
/// pass before instruction emission is complete.
#[derive(Debug)]
pub(super) struct OptimizationPipeline {
    pub(super) graph: BlockGraph,
    pub(super) next_label: u32,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) borrow_unreachable_blocks: FxHashSet<Label>,
    pub(super) has_synthesized_percent_target_locations: bool,
}

/// The linear assembler state used after CFG optimization is complete.
#[derive(Debug)]
pub(super) struct LayoutAssembler {
    pub(super) items: Vec<Item>,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) removed_max_depth: Option<u32>,
    pub(super) next_label: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ExceptionRegion {
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
            operand_patches: Vec::new(),
            next_label: 0,
            location: SourceLocation::NONE,
            exception_regions: Vec::new(),
            preserved_block_boundaries: FxHashSet::default(),
            borrow_unreachable_blocks: FxHashSet::default(),
            load_fast_borrowing_enabled: true,
            strict_owned_loads: false,
            has_synthesized_percent_target_locations: false,
        }
    }
}
