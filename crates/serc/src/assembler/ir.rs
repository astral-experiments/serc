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

#[derive(Clone, Copy, Debug)]
pub(crate) struct InstructionId(pub(super) usize);

#[derive(Clone, Copy, Debug)]
pub(super) enum Operand {
    Value(u32),
    Forward(Label),
    Backward(Label),
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) struct InstructionFlags: u16 {
        const FORCE_OWNED_LOAD = 1 << 0;
        const STRICT_OWNED_LOAD = 1 << 1;
        const INLINE_SMALL_EXIT = 1 << 2;
        const PRESERVE_INLINED_JUMP_NOP = 1 << 3;
        // Whether an incoming jump may be threaded through this instruction.
        const ALLOW_JUMP_THREADING_TARGET = 1 << 4;
        // Whether an unlocated block ending here may be inlined into predecessors.
        const ALLOW_NO_LOCATION_BLOCK_INLINING = 1 << 5;
        const PRESERVE_NO_LOCATION = 1 << 6;
        const PREVENT_FUSION_WITH_NEXT = 1 << 7;
        const PREVENT_FUSION_WITH_PREVIOUS = 1 << 8;
        const DEFER_REDUNDANT_JUMP_REMOVAL = 1 << 9;
        const PRESERVE_NOP_AFTER_JUMP_THREADING = 1 << 10;
        const CONVERTED_POP_BLOCK = 1 << 11;
        // Stop borrowed-load traversal at an optimized-away empty CFG block without exposing a
        // label to the other assembler passes.
        const BORROW_UNREACHABLE_ENTRY = 1 << 12;
    }
}

impl InstructionFlags {
    pub(super) const DEFAULT: Self =
        Self::INLINE_SMALL_EXIT.union(Self::ALLOW_JUMP_THREADING_TARGET);

    pub(super) const FUSE_OR: Self = Self::FORCE_OWNED_LOAD
        .union(Self::STRICT_OWNED_LOAD)
        .union(Self::ALLOW_NO_LOCATION_BLOCK_INLINING)
        .union(Self::PRESERVE_NO_LOCATION)
        .union(Self::DEFER_REDUNDANT_JUMP_REMOVAL)
        .union(Self::PRESERVE_NOP_AFTER_JUMP_THREADING)
        .union(Self::CONVERTED_POP_BLOCK);
    pub(super) const FUSE_AND: Self =
        Self::INLINE_SMALL_EXIT.union(Self::ALLOW_JUMP_THREADING_TARGET);
    pub(super) const FUSE_FROM_FIRST: Self =
        Self::PREVENT_FUSION_WITH_PREVIOUS.union(Self::BORROW_UNREACHABLE_ENTRY);
    pub(super) const FUSE_FROM_SECOND: Self = Self::PREVENT_FUSION_WITH_NEXT;
    pub(super) const FUSE_CLEARED: Self = Self::PRESERVE_INLINED_JUMP_NOP;
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
pub(super) struct ExceptionPolicy {
    pub(super) exclusion: ExceptionExclusion,
    pub(super) normalized_owner: NormalizedExceptionOwner,
}

impl ExceptionPolicy {
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
    pub(super) depth_after: Option<u32>,
    pub(super) flags: InstructionFlags,
    // Retain the jump's line event only if its original target is copied directly.
    pub(super) preserve_direct_inlined_jump_nop: Option<Label>,
    // `NOT_TAKEN` is added after CPython labels exception handlers. The new instruction keeps
    // whatever exception target remains in its reused CFG slot, if there is one.
    pub(super) exception_policy: ExceptionPolicy,
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
            depth_after,
            flags: InstructionFlags::DEFAULT,
            preserve_direct_inlined_jump_nop: None,
            exception_policy: ExceptionPolicy::default(),
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
        instruction.remove_flag(InstructionFlags::INLINE_SMALL_EXIT);
        instruction
    }

    pub(super) const fn has_flag(&self, flag: InstructionFlags) -> bool {
        self.flags.contains(flag)
    }

    pub(super) fn insert_flag(&mut self, flag: InstructionFlags) {
        self.flags.insert(flag);
    }

    pub(super) fn remove_flag(&mut self, flag: InstructionFlags) {
        self.flags.remove(flag);
    }

    pub(super) fn set_flag(&mut self, flag: InstructionFlags, enabled: bool) {
        self.flags.set(flag, enabled);
    }

    /// Combines the provenance of two instructions replaced by one superinstruction.
    pub(super) fn fused(opcode: Opcode, operand: Operand, first: Self, second: Self) -> Self {
        let mut flags = ((first.flags | second.flags) & InstructionFlags::FUSE_OR)
            | ((first.flags & second.flags) & InstructionFlags::FUSE_AND)
            | (first.flags & InstructionFlags::FUSE_FROM_FIRST)
            | (second.flags & InstructionFlags::FUSE_FROM_SECOND);
        flags.remove(InstructionFlags::FUSE_CLEARED);

        Self {
            opcode,
            operand,
            location: first.location,
            depth_after: second.depth_after,
            flags,
            preserve_direct_inlined_jump_nop: None,
            exception_policy: first.exception_policy.merged(second.exception_policy),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Item {
    Instruction(Instruction),
    Label(Label),
}

#[derive(Debug)]
pub(crate) struct Assembler {
    pub(super) items: Vec<Item>,
    /// The block IR exists only while the assembler pass pipeline is running.
    pub(super) graph: Option<BlockGraph>,
    pub(super) next_label: u32,
    pub(super) location: SourceLocation,
    pub(super) exception_regions: Vec<ExceptionRegion>,
    pub(super) preserved_block_boundaries: FxHashSet<Label>,
    pub(super) borrow_unreachable_blocks: FxHashSet<Label>,
    pub(super) load_fast_borrowing_enabled: bool,
    pub(super) strict_owned_loads: bool,
    pub(super) next_instruction_borrow_unreachable: bool,
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
            items: Vec::new(),
            graph: None,
            next_label: 0,
            location: SourceLocation::NONE,
            exception_regions: Vec::new(),
            preserved_block_boundaries: FxHashSet::default(),
            borrow_unreachable_blocks: FxHashSet::default(),
            load_fast_borrowing_enabled: true,
            strict_owned_loads: false,
            next_instruction_borrow_unreachable: false,
        }
    }
}
