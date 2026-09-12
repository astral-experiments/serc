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
    // Original entries that split borrowed-load analysis without changing the physical CFG.
    pub(super) borrow_only_block_boundaries: FxHashSet<Label>,
    pub(super) source_boundaries: FxHashSet<Label>,
    pub(super) continuation_boundaries: FxHashSet<Label>,
    pub(super) fusion_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_ranges: Vec<(Label, Label)>,
    // Retained SETUPs emitted directly into an authored primitive conditional fallthrough.
    pub(super) original_fallthrough_setups: FxHashSet<Label>,
    // One authored normal POP_BLOCK, not an extra synthetic end record or a copied exit.
    pub(super) retained_normal_pop_blocks: FxHashMap<Label, RetainedNormalPopBlock>,
    pub(super) original_normal_finally_arrays: Vec<OriginalNormalFinallyArray>,
    pub(super) uncompacted_conditional_arrays: Vec<UncompactedConditionalArray>,
    pub(super) original_primitive_condition_boundaries: Vec<Instruction>,
    pub(super) synchronous_with_setups: FxHashSet<Label>,
    pub(super) removed_pop_block_boundaries: FxHashSet<Label>,
    pub(super) removed_pop_block_widths: FxHashMap<Label, u16>,
    // Original ordinary-try exits contain POP_BLOCK followed by JUMP_NO_INTERRUPT.
    pub(super) normal_try_exits: Vec<(Label, Label)>,
    // First records of the original manager cleanup arrays, before line resolution.
    pub(super) original_manager_pop_blocks: Vec<Label>,
    // Original try/except POP_BLOCK exits inside their own finally, for owner transfer only.
    pub(super) compound_try_owner_exits: Vec<(Label, Label)>,
    pub(super) omitted_nop_locations: FxHashMap<Label, SourceLocation>,
    pub(super) omitted_nop_counts: FxHashMap<Label, usize>,
    // One omitted fallthrough marker with no original label: item offset, location, count.
    pub(super) omitted_nop_tail: Option<(usize, SourceLocation, usize)>,
    // Original constant-expression/Pass pairs sharing only their fresh marker labels.
    pub(super) constant_pass_clusters: Vec<ConstantPassCluster>,
    pub(super) normal_try_copy_blocks: FxHashSet<Label>,
    pub(super) normal_try_copy_steps: Vec<(Label, Label, usize)>,
    // Existing copied-entry labels immediately following replaced original jump NOPs.
    pub(super) normal_try_copy_jump_nop_successors: FxHashSet<Label>,
    // Physical labels appended by a certified original-array copy, before region aliases.
    pub(super) normal_try_copy_inline_labels: FxHashSet<Label>,
    // Owner boundaries inside a restored original array are not independent CFG entries.
    pub(super) normal_try_copy_internal_labels: Vec<Label>,
    pub(super) normal_try_copy_pop_ranges: Vec<(Label, Label)>,
    pub(super) protected_prefix_boundaries: FxHashSet<Label>,
    pub(super) noop_finally_locations: Vec<SourceLocation>,
    pub(super) with_context_locations: Vec<SourceLocation>,
    pub(super) uniform_fold_owner_slots: Vec<(SourceLocation, crate::target::Opcode, u16)>,
    // Exact source comparison and original labels, before any jump copying or cleanup.
    pub(super) source_none_conditions: Vec<SourceNoneCondition>,
    // Exact terminal pairs produced by original conditional jump threading.
    pub(super) threaded_condition_origins: Vec<(Instruction, Instruction)>,
}

/// A single-line source None comparison whose three removed records precede late SETUP cleanup.
#[derive(Debug)]
pub(super) struct SourceNoneCondition {
    pub(super) entry: Label,
    pub(super) labels: Vec<Label>,
    pub(super) instruction: Instruction,
    // Original leading labels and at most two NOPs; None retains the strict old path.
    pub(super) entry_prefix: Option<Vec<Item>>,
}

/// Two authored NOPs within one original array, delimited by existing constant labels.
#[derive(Debug)]
pub(super) struct ConstantPassCluster {
    pub(super) start: Label,
    pub(super) end: Label,
    pub(super) constant_location: SourceLocation,
    pub(super) pass_location: SourceLocation,
}

/// Exact labels around a retained original `POP_BLOCK`, captured before cleanup is emitted.
#[derive(Debug)]
pub(super) struct RetainedNormalPopBlock {
    pub(super) before: Vec<Label>,
    pub(super) after: Vec<Label>,
}

/// The original normal-finally prefix and the jumps that entered its fresh array.
#[derive(Debug)]
pub(super) struct OriginalNormalFinallyArray {
    pub(super) entry: Label,
    pub(super) prefix: Vec<Item>,
    pub(super) incoming: Vec<Instruction>,
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

/// An original conditional array whose supported records cannot be compacted.
#[derive(Debug)]
pub(super) struct UncompactedConditionalArray {
    prefix: Vec<Item>,
}

impl ExceptionOwnerProvenance {
    /// Retains whole original arrays, before optimization can remove their live records.
    pub(super) fn capture_uncompacted_conditional_arrays(&mut self, items: &[Item]) {
        use crate::target::opcodes::{POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE};

        let mut begin = 0;
        for (index, item) in items.iter().enumerate() {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            if matches!(instruction.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
                && matches!(instruction.operand(), Operand::Forward(_))
            {
                let next = items[index + 1..]
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))
                    .map(|offset| index + 1 + offset);
                if let Some(end) = next
                    && matches!(items[end], Item::Instruction(marker) if marker.opcode == NOT_TAKEN)
                    && self.uncompacted_prefix(&items[begin..=end], items)
                {
                    self.uncompacted_conditional_arrays
                        .push(UncompactedConditionalArray {
                            prefix: items[begin..=end].to_vec(),
                        });
                }
            }
            // Labels alone are not original array boundaries. A terminator ends
            // the original array; the already-emitted NOT_TAKEN is not a CP record yet.
            if instruction.opcode == NOT_TAKEN
                || is_scope_exit(instruction.opcode)
                || matches!(
                    instruction.operand(),
                    Operand::Forward(_) | Operand::Backward(_)
                )
            {
                begin = index + 1;
            }
        }
    }

    fn uncompacted_prefix(&self, prefix: &[Item], all: &[Item]) -> bool {
        use crate::target::opcodes::{
            BINARY_OP, CALL, IMPORT_NAME, LOAD_ATTR, LOAD_CONST, LOAD_FAST, LOAD_GLOBAL, LOAD_NAME,
            LOAD_SMALL_INT, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, PUSH_NULL, STORE_FAST, STORE_NAME,
            TO_BOOL,
        };

        let leading_labels = prefix
            .iter()
            .map_while(|item| {
                if let Item::Label(label) = item {
                    Some(*label)
                } else {
                    None
                }
            })
            .collect::<FxHashSet<_>>();
        // A raw emitted return or copied backedge is not necessarily a CP
        // record. The compiler's original branch target establishes the left
        // edge independently; the preceding terminator only bounds this scan.
        let has_original_target = self.original_primitive_condition_boundaries.iter().any(|condition| {
            matches!(condition.operand(), Operand::Forward(label) if leading_labels.contains(&label))
                && all.iter().filter(|item| **item == Item::Instruction(*condition)).count() == 1
        }) || all.iter().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if instruction.fusion_policy.is_authored_break()
                && matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                    if leading_labels.contains(&label)))
        });
        if !has_original_target {
            return false;
        }
        let positions = all
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if let Item::Label(label) = item {
                    Some((*label, index))
                } else {
                    None
                }
            })
            .collect::<FxHashMap<_, _>>();
        let targets = all
            .iter()
            .filter_map(|item| {
                if let Item::Instruction(instruction) = item
                    && let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                {
                    Some(label)
                } else {
                    None
                }
            })
            .collect::<FxHashSet<_>>();
        let mut instructions = Vec::new();
        let mut labels = FxHashSet::default();
        for item in prefix {
            match item {
                Item::Label(label) => {
                    if !labels.insert(*label)
                        || (!instructions.is_empty() && targets.contains(label))
                        || self.removed_pop_block_boundaries.contains(label)
                        || self.omitted_nop_counts.contains_key(label)
                        || self.omitted_nop_locations.contains_key(label)
                        || self.fusion_boundaries.contains(label)
                        || self.normal_try_copy_blocks.contains(label)
                        || self.normal_try_copy_inline_labels.contains(label)
                        || self.normal_try_copy_internal_labels.contains(label)
                        || self.normal_try_copy_jump_nop_successors.contains(label)
                    {
                        return false;
                    }
                }
                Item::Instruction(instruction) => instructions.push(*instruction),
            }
        }
        let [body @ .., condition, marker] = instructions.as_slice() else {
            return false;
        };
        if marker.opcode != NOT_TAKEN
            || !matches!(condition.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || body.last().is_none_or(|last| last.opcode != TO_BOOL)
        {
            return false;
        }
        let mut setup_count = 0;
        for (index, instruction) in instructions.iter().enumerate() {
            if instruction.local_policy.hidden_exception_owner_slots() != 0
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.no_location_policy.borrow_fallthrough()
                    != super::BorrowFallthrough::Reachable
                || instruction.jump_policy.borrow_empty_block()
                || instruction.location.line < 0
            {
                return false;
            }
            if index >= body.len() {
                continue;
            }
            if !matches!(instruction.operand(), Operand::Value(_)) {
                return false;
            }
            match instruction.opcode {
                NOP => {
                    // SETUP becomes NOP only after peephole optimization. Count
                    // its existing record once, using the exact authored range.
                    let paired = self.setup_nop_ranges.iter().any(|(start, end)| {
                        let (Some(&start_at), Some(&end_at)) =
                            (positions.get(start), positions.get(end))
                        else {
                            return false;
                        };
                        let range = &all[start_at..end_at];
                        labels.contains(start)
                            && labels.contains(end)
                            && range
                                .iter()
                                .filter_map(|item| {
                                    if let Item::Instruction(value) = item {
                                        Some(value)
                                    } else {
                                        None
                                    }
                                })
                                .eq([instruction])
                            && self.setups.iter().any(|(label, location)| {
                                *location == Some(instruction.location)
                                    && positions.get(label).is_some_and(|&position| {
                                        position >= end_at
                                            && all[end_at..position]
                                                .iter()
                                                .all(|item| matches!(item, Item::Label(_)))
                                    })
                            })
                    });
                    if !paired
                        || instruction.fusion_policy.is_converted_pop_block()
                        || instructions
                            .get(index.wrapping_sub(1))
                            .is_some_and(|previous| {
                                previous.location.line == instruction.location.line
                            })
                        || instructions
                            .get(index + 1)
                            .is_none_or(|next| next.location.line == instruction.location.line)
                    {
                        return false;
                    }
                    setup_count += 1;
                }
                LOAD_FAST => {
                    // Superinstruction creation would compact two live records.
                    if instructions
                        .get(index + 1)
                        .is_some_and(|next| next.opcode == LOAD_FAST)
                    {
                        return false;
                    }
                }
                STORE_FAST => {
                    // CP fuses adjacent local stores/loads only on the same
                    // line. Distinct located lines keep both original records.
                    if instructions.get(index + 1).is_some_and(|next| {
                        matches!(next.opcode, LOAD_FAST | STORE_FAST)
                            && next.location.line == instruction.location.line
                    }) {
                        return false;
                    }
                }
                LOAD_GLOBAL => {
                    // The low bit represents a separately emitted PUSH_NULL
                    // that CP folds into this record before marker insertion.
                    if !matches!(instruction.operand(), Operand::Value(value) if value & 1 == 0) {
                        return false;
                    }
                }
                PUSH_NULL => {
                    // Imported attribute calls retain this separate record;
                    // a direct global call instead folds it into LOAD_GLOBAL.
                    if instructions
                        .get(index.wrapping_sub(1))
                        .is_none_or(|previous| previous.opcode != LOAD_ATTR)
                    {
                        return false;
                    }
                }
                LOAD_SMALL_INT => {
                    // Small integer specialization replaces LOAD_CONST in place.
                    if !matches!(instruction.operand(), Operand::Value(0..=255)) {
                        return false;
                    }
                }
                BINARY_OP => {
                    // Subscript is one original record unless constant folding
                    // consumes both preceding constant loads with it.
                    if instruction.operand() != Operand::Value(26)
                        || instructions[..index]
                            .iter()
                            .rev()
                            .filter(|previous| previous.opcode != NOP)
                            .take(2)
                            .filter(|previous| {
                                matches!(previous.opcode, LOAD_CONST | LOAD_SMALL_INT)
                            })
                            .count()
                            == 2
                    {
                        return false;
                    }
                }
                TO_BOOL => {
                    // The one conversion is emitted for a nonconstant terminal
                    // condition. Pseudo short-circuit COPY lowering is excluded.
                    if index + 1 != body.len()
                        || instructions
                            .get(index.wrapping_sub(1))
                            .is_none_or(|previous| {
                                !matches!(previous.opcode, LOAD_FAST | LOAD_NAME | LOAD_ATTR | CALL)
                            })
                    {
                        return false;
                    }
                }
                LOAD_NAME | LOAD_ATTR | LOAD_CONST | STORE_NAME | CALL | IMPORT_NAME => {}
                _ => return false,
            }
        }
        // Unlocated POP_BLOCK and arbitrary emitted NOPs need different
        // retention proofs. This certificate covers authored retained SETUPs.
        setup_count > 0
            && self
                .setups
                .iter()
                .all(|(label, location)| !labels.contains(label) || location.is_some())
    }
}

impl LayoutAssembler {
    /// Maps a proven virgin append cell to its unchanged conditional and marker.
    fn uncompacted_conditional_null_markers(
        &self,
        jump_targets: &FxHashSet<Label>,
    ) -> FxHashSet<usize> {
        use crate::target::opcodes::{LOAD_FAST, LOAD_FAST_BORROW};

        let mut result = FxHashSet::default();
        for origin in &self
            .exception_owner_provenance
            .uncompacted_conditional_arrays
        {
            let Some(Item::Label(entry)) = origin.prefix.first() else {
                continue;
            };
            let starts = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| (*item == Item::Label(*entry)).then_some(index))
                .collect::<Vec<_>>();
            let [start] = starts.as_slice() else { continue };
            let original_labels = origin
                .prefix
                .iter()
                .filter_map(|item| {
                    if let Item::Label(label) = item {
                        Some(*label)
                    } else {
                        None
                    }
                })
                .collect::<FxHashSet<_>>();
            let mut cursor = *start;
            let mut valid = true;
            for expected in &origin.prefix {
                while let Some(Item::Label(label)) = self.items.get(cursor)
                    && !original_labels.contains(label)
                {
                    // Range-only aliases carry no original instruction cells.
                    if jump_targets.contains(label)
                        || self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                        || self.preserved_block_boundaries.contains(label)
                        || self
                            .exception_owner_provenance
                            .setups
                            .iter()
                            .any(|(setup, _)| setup == label)
                        || self
                            .exception_regions
                            .iter()
                            .any(|region| region.target == *label)
                    {
                        valid = false;
                        break;
                    }
                    cursor += 1;
                }
                let same = match (expected, self.items.get(cursor)) {
                    (Item::Label(expected), Some(Item::Label(current))) => expected == current,
                    (Item::Instruction(expected), Some(Item::Instruction(current))) => {
                        // Borrowing rewrites only the opcode after CP's slot
                        // selection. Every other captured policy/depth byte must
                        // still belong to the same original record.
                        let mut expected = *expected;
                        if expected.opcode == LOAD_FAST && current.opcode == LOAD_FAST_BORROW {
                            expected.opcode = LOAD_FAST_BORROW;
                        }
                        expected == *current
                    }
                    _ => false,
                };
                if !valid || !same {
                    valid = false;
                    break;
                }
                cursor += 1;
            }
            if valid {
                result.insert(
                    self.items[..cursor]
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count()
                        - 1,
                );
            }
        }
        result
    }
}

impl LayoutAssembler {
    /// Resolves append slots in the first captured normal-POP instruction array.
    fn retained_normal_pop_terminators(
        &self,
        jump_targets: &FxHashSet<Label>,
        label_positions: &FxHashMap<Label, usize>,
    ) -> FxHashMap<usize, Option<usize>> {
        let mut result = FxHashMap::default();
        for (entry, origin) in self.items.iter().filter_map(|item| {
            let Item::Label(entry) = item else {
                return None;
            };
            self.exception_owner_provenance
                .retained_normal_pop_blocks
                .get(entry)
                .map(|origin| (entry, origin))
        }) {
            if !self
                .exception_owner_provenance
                .source_boundaries
                .contains(entry)
                || !self
                    .exception_owner_provenance
                    .block_boundaries
                    .contains(entry)
                || origin.before.first() != Some(entry)
                || origin.after.is_empty()
            {
                continue;
            }
            let aliases = origin
                .before
                .iter()
                .chain(&origin.after)
                .copied()
                .collect::<FxHashSet<_>>();
            if aliases.len() != origin.before.len() + origin.after.len()
                || aliases.iter().any(|label| {
                    jump_targets.contains(label)
                        || self.preserved_block_boundaries.contains(label)
                        || self
                            .exception_regions
                            .iter()
                            .any(|region| region.target == *label)
                        || self
                            .exception_owner_provenance
                            .setups
                            .iter()
                            .any(|(setup, _)| setup == label)
                        || self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                        || (*label != *entry
                            && (self
                                .exception_owner_provenance
                                .source_boundaries
                                .contains(label)
                                || self
                                    .exception_owner_provenance
                                    .block_boundaries
                                    .contains(label)))
                })
            {
                continue;
            }
            let entries = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    matches!(item, Item::Label(label) if label == entry).then_some(index)
                })
                .collect::<Vec<_>>();
            let [begin] = entries.as_slice() else {
                continue;
            };
            let mut cursor = *begin;
            if !origin.before.iter().all(|label| {
                let same = self.items.get(cursor) == Some(&Item::Label(*label));
                cursor += 1;
                same
            }) {
                continue;
            }
            let Some(Item::Instruction(marker)) = self.items.get(cursor) else {
                continue;
            };
            if marker.opcode != NOP
                || marker.operand() != Operand::Value(0)
                || marker.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
                || !marker.fusion_policy.is_converted_pop_block()
                || !marker.jump_policy.preserves_explicit_nop()
                || marker.jump_policy.borrow_empty_block()
                || marker.local_policy.hidden_exception_owner_slots() != 0
                || marker.pre_fold_extra_instructions() != 0
                || marker.no_location_policy.borrow_fallthrough()
                    != super::BorrowFallthrough::Reachable
            {
                continue;
            }
            cursor += 1;
            if !origin.after.iter().all(|label| {
                let same = self.items.get(cursor) == Some(&Item::Label(*label));
                cursor += 1;
                same
            }) {
                continue;
            }
            let logical_start = self.items[..cursor]
                .iter()
                .filter(|item| matches!(item, Item::Instruction(_)))
                .count();
            let mut tail = ExceptionOwnerTail::default();
            let mut previous = Some(*marker);
            for (offset, item) in self.items[cursor..].iter().enumerate() {
                // Even an apparently empty foreign label can stand for a different
                // original array. Captured aliases above are the only permitted joins.
                let Item::Instruction(instruction) = item else {
                    break;
                };
                let none_tail =
                    matches!(instruction.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && instruction.local_policy.hidden_exception_owner_slots() == 3
                        && instruction.exception_policy() == super::ExceptionPolicy::default();
                if (instruction.local_policy.hidden_exception_owner_slots() != 0 && !none_tail)
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.no_location_policy.borrow_fallthrough()
                        != super::BorrowFallthrough::Reachable
                    || matches!(instruction.opcode, NOP | NOT_TAKEN | FOR_ITER | SEND)
                    || is_scope_exit(instruction.opcode)
                {
                    break;
                }
                tail.instruction(*instruction, previous);
                if tail.early_removals != usize::from(none_tail) * 3
                    || tail.insertions != 0
                    || tail.late_removals != 0
                    || tail.post_compaction_fusions != 0
                {
                    break;
                }
                if is_conditional_jump(instruction.opcode) {
                    if matches!(instruction.operand(), Operand::Forward(_)) {
                        let owner = if none_tail {
                            // The three specialized None records are the only removals.
                            // The new append index is the old IS_OP cell, whose owner
                            // belongs to this protected suffix after the NULL POP_BLOCK.
                            let Some(owner) = self.uniform_exception_owner(
                                logical_start,
                                logical_start + offset + 1,
                                label_positions,
                            ) else {
                                break;
                            };
                            if self.exception_regions[owner].is_scope_fallback {
                                break;
                            }
                            Some(owner)
                        } else {
                            None
                        };
                        result.insert(logical_start + offset, owner);
                    }
                    break;
                }
                if !matches!(instruction.operand(), Operand::Value(_)) {
                    break;
                }
                previous = Some(*instruction);
            }
        }
        result
    }

    /// Proves the exact appended marker of an unchanged original normal-finally array.
    fn original_normal_finally_null_markers(
        &self,
        jump_targets: &FxHashSet<Label>,
    ) -> FxHashSet<usize> {
        use crate::target::opcodes::{LOAD_FAST, LOAD_FAST_BORROW};

        let mut result = FxHashSet::default();
        for origin in &self
            .exception_owner_provenance
            .original_normal_finally_arrays
        {
            let authored_break_location = match origin.incoming.as_slice() {
                [jump] if jump.fusion_policy.is_authored_break() && jump.location.line >= 0 => {
                    Some(jump.location)
                }
                _ => None,
            };
            if origin.incoming.is_empty()
                || origin.incoming.iter().any(|jump| {
                    !crate::target::is_unconditional_jump(jump.opcode)
                        || jump.operand() != Operand::Forward(origin.entry)
                })
                || ((!self
                    .exception_owner_provenance
                    .source_boundaries
                    .contains(&origin.entry)
                    || !self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(&origin.entry))
                    && authored_break_location.is_none())
            {
                continue;
            }
            let labels = origin
                .prefix
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            if labels.iter().copied().collect::<FxHashSet<_>>().len() != labels.len()
                || labels.first() != Some(&origin.entry)
                || labels.iter().any(|label| {
                    (jump_targets.contains(label)
                        && !(authored_break_location.is_some() && *label == origin.entry))
                        || self.preserved_block_boundaries.contains(label)
                        || self
                            .exception_regions
                            .iter()
                            .any(|region| region.target == *label)
                        || self
                            .exception_owner_provenance
                            .setups
                            .iter()
                            .any(|(setup, _)| setup == label)
                        || self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .normal_try_copy_blocks
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .normal_try_copy_inline_labels
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .normal_try_copy_internal_labels
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .normal_try_copy_jump_nop_successors
                            .contains(label)
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
                        || self
                            .exception_owner_provenance
                            .setup_nop_boundaries
                            .contains(label)
                        || (*label != origin.entry
                            && (self
                                .exception_owner_provenance
                                .source_boundaries
                                .contains(label)
                                || self
                                    .exception_owner_provenance
                                    .block_boundaries
                                    .contains(label)))
                })
            {
                continue;
            }
            let starts = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| (*item == Item::Label(origin.entry)).then_some(index))
                .collect::<Vec<_>>();
            let [start] = starts.as_slice() else {
                continue;
            };
            let (current, end) = if authored_break_location.is_some() {
                let original_labels = labels.iter().copied().collect::<FxHashSet<_>>();
                let mut cursor = *start;
                let mut current = Vec::with_capacity(origin.prefix.len());
                let mut valid = true;
                for _ in &origin.prefix {
                    while let Some(Item::Label(label)) = self.items.get(cursor)
                        && !original_labels.contains(label)
                    {
                        // The graph can label the already-emitted marker's
                        // fallthrough. Such aliases do not add original cells.
                        if jump_targets.contains(label)
                            || self
                                .exception_owner_provenance
                                .block_boundaries
                                .contains(label)
                            || self
                                .exception_owner_provenance
                                .source_boundaries
                                .contains(label)
                            || self.preserved_block_boundaries.contains(label)
                            || self
                                .exception_owner_provenance
                                .setups
                                .iter()
                                .any(|(setup, _)| setup == label)
                            || self
                                .exception_regions
                                .iter()
                                .any(|region| region.target == *label)
                        {
                            valid = false;
                            break;
                        }
                        cursor += 1;
                    }
                    if !valid {
                        break;
                    }
                    let Some(item) = self.items.get(cursor) else {
                        valid = false;
                        break;
                    };
                    current.push(*item);
                    cursor += 1;
                }
                if !valid {
                    continue;
                }
                (current, cursor)
            } else {
                let Some(current) = self.items.get(*start..*start + origin.prefix.len()) else {
                    continue;
                };
                (current.to_vec(), *start + origin.prefix.len())
            };
            let Some(marker_at) = origin
                .prefix
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            let mut expected = origin.prefix.clone();
            let (Item::Instruction(marker), Item::Instruction(original_marker)) =
                (current[marker_at], &mut expected[marker_at])
            else {
                continue;
            };
            // Location propagation changes the retained POP_BLOCK's line, not its record identity.
            original_marker.location = marker.location;
            if let Some(location) = authored_break_location {
                // CP propagates its sole jump predecessor's location before
                // NOP removal. A distinct next line retains this original slot.
                let next = current[marker_at + 1..].iter().find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                });
                if marker.location != location
                    || next.is_none_or(|next| {
                        next.location.line < 0 || next.location.line == location.line
                    })
                {
                    continue;
                }
                for (expected, current) in expected.iter_mut().zip(&current) {
                    if let (Item::Instruction(expected), Item::Instruction(current)) =
                        (expected, current)
                        && expected.opcode == LOAD_FAST
                        && current.opcode == LOAD_FAST_BORROW
                    {
                        // Borrowing follows marker normalization and changes
                        // only this opcode, not the original instruction cell.
                        expected.opcode = LOAD_FAST_BORROW;
                    }
                }
            }
            if current != expected
                || marker.opcode != NOP
                || !marker.fusion_policy.is_converted_pop_block()
                || marker.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
                || marker.local_policy.hidden_exception_owner_slots() != 0
                || marker.pre_fold_extra_instructions() != 0
                || marker.jump_policy.borrow_empty_block()
                || marker.no_location_policy.borrow_fallthrough()
                    != super::BorrowFallthrough::Reachable
            {
                continue;
            }
            let mut tail = ExceptionOwnerTail::default();
            let mut previous = Some(marker);
            let mut valid = true;
            for item in &current[marker_at + 1..current.len() - 1] {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                if instruction.local_policy.hidden_exception_owner_slots() != 0
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.no_location_policy.borrow_fallthrough()
                        != super::BorrowFallthrough::Reachable
                    || matches!(instruction.opcode, NOP | NOT_TAKEN | FOR_ITER | SEND)
                    || is_scope_exit(instruction.opcode)
                {
                    valid = false;
                    break;
                }
                tail.instruction(*instruction, previous);
                previous = Some(*instruction);
            }
            if !valid
                || tail.early_removals != 0
                || tail.insertions != 0
                || tail.late_removals != 0
                || tail.post_compaction_fusions != 0
            {
                continue;
            }
            if let Some(Item::Instruction(last)) = current.last()
                && last.opcode == NOT_TAKEN
                && last.exception_policy().exclusion() == super::ExceptionExclusion::None
                && (matches!(
                    last.exception_policy().owner(),
                    NormalizedExceptionOwner::Absent | NormalizedExceptionOwner::DynamicCondition
                ) || (authored_break_location.is_some()
                    && last.exception_policy().owner() == NormalizedExceptionOwner::Present))
            {
                let logical_end = self.items[..end]
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count();
                result.insert(logical_end - 1);
            }
        }
        result
    }

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

    /// Proves only original entry aliases and a leading normal `POP_BLOCK` marker.
    ///
    /// CPython removes the first of two same-line NOPs before SETUP cleanup. The
    /// retained `POP_BLOCK` can also inherit an entry location before that pass.
    /// Interior targets and any other prefix rewrite remain outside this proof.
    fn source_none_entry_prefix(
        &self,
        origin: &SourceNoneCondition,
        begin: usize,
        end: usize,
        label_positions: &FxHashMap<Label, usize>,
        boundary: &impl Fn(&Label) -> bool,
        copied: &impl Fn(&Label) -> bool,
    ) -> Option<(Vec<Label>, Option<Instruction>)> {
        let source = origin.entry_prefix.as_ref()?;
        if source.first() != Some(&Item::Label(origin.entry)) {
            return None;
        }
        let mut labels = Vec::new();
        let mut nops = Vec::new();
        for item in source {
            match item {
                Item::Label(label) => {
                    if copied(label) || (!nops.is_empty() && boundary(label)) {
                        return None;
                    }
                    labels.push(*label);
                }
                Item::Instruction(instruction) if instruction.opcode == NOP => {
                    nops.push(*instruction);
                }
                Item::Instruction(_) => return None,
            }
        }
        let current: Vec<_> = self.items[begin..end]
            .iter()
            .copied()
            .take_while(|item| match item {
                Item::Label(_) => true,
                Item::Instruction(instruction) => instruction.opcode == NOP,
            })
            .collect();
        let mut current_labels = Vec::new();
        let mut current_nops = Vec::new();
        for item in &current {
            match item {
                Item::Label(label) if current_nops.is_empty() => current_labels.push(*label),
                Item::Instruction(instruction) => current_nops.push(*instruction),
                Item::Label(_) => return None,
            }
        }
        if current_labels != labels {
            return None;
        }
        let (mut expected, early_removed) = match nops.as_slice() {
            [] if current_nops.is_empty() => return Some((labels, None)),
            [pop] => (*pop, 0),
            [plain, pop]
                if plain.opcode == NOP
                    && plain.operand() == Operand::Value(0)
                    && plain.location.line >= 0
                    && plain.location == pop.location
                    && plain.local_policy == super::LocalPolicy::default()
                    && plain.no_location_policy == super::ir::NoLocationPolicy::default()
                    && plain.fusion_policy == super::ir::FusionPolicy::default()
                    && plain.jump_policy == super::ir::JumpPolicy::default()
                    && plain.exception_policy() == super::ExceptionPolicy::default()
                    && plain.pre_fold_extra_instructions() == 0 =>
            {
                (*pop, 1)
            }
            _ => return None,
        };
        if expected.opcode != NOP
            || expected.operand() != Operand::Value(0)
            || !expected.fusion_policy.is_converted_pop_block()
            || expected.local_policy.hidden_exception_owner_slots() != 0
            || expected.pre_fold_extra_instructions() != 0
            || expected.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
            || expected.no_location_policy.borrow_fallthrough()
                != super::BorrowFallthrough::Reachable
            || expected.jump_policy.borrow_empty_block()
        {
            return None;
        }
        let [current] = current_nops.as_slice() else {
            return None;
        };
        if expected.location == SourceLocation::NONE {
            let entry_position = label_positions.get(&origin.entry)?;
            if self
                .exception_regions
                .iter()
                .any(|region| label_positions.get(&region.target) == Some(entry_position))
            {
                return None;
            }
            let mut incoming = Vec::new();
            for item in &self.items {
                if let Item::Instruction(instruction) = item
                    && let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                    && label_positions.get(&label) == Some(entry_position)
                {
                    incoming.push(instruction.location);
                }
            }
            if let Some(previous) = self.items[..begin]
                .iter()
                .rev()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                && !is_scope_exit(previous.opcode)
                && !crate::target::is_unconditional_jump(previous.opcode)
            {
                incoming.push(previous.location);
            }
            let [location] = incoming.as_slice() else {
                return None;
            };
            if location.line < 0 || *location != current.location {
                return None;
            }
            expected.location = *location;
        }
        expected
            .local_policy
            .add_hidden_exception_owner_slots(early_removed);
        (expected == *current).then_some((labels, Some(*current)))
    }

    /// Certifies a NULL SETUP record reused after a retained SETUP in one original array.
    fn retained_setup_null_markers(
        &self,
        label_positions: &FxHashMap<Label, usize>,
        jump_targets: &FxHashSet<Label>,
        retained_setup_nops: &FxHashSet<usize>,
    ) -> FxHashSet<usize> {
        let provenance = &self.exception_owner_provenance;
        let mut result = FxHashSet::default();
        if retained_setup_nops.is_empty() {
            return result;
        }
        let copied = |label: &Label| {
            provenance.normal_try_copy_blocks.contains(label)
                || provenance
                    .normal_try_copy_jump_nop_successors
                    .contains(label)
                || provenance.normal_try_copy_inline_labels.contains(label)
                || provenance.normal_try_copy_internal_labels.contains(label)
                || provenance
                    .normal_try_copy_steps
                    .iter()
                    .any(|(a, b, _)| a == label || b == label)
                || provenance
                    .normal_try_copy_pop_ranges
                    .iter()
                    .any(|(a, b)| a == label || b == label)
        };
        let boundary = |label: &Label| {
            provenance.block_boundaries.contains(label)
                || provenance.source_boundaries.contains(label)
                || self.preserved_block_boundaries.contains(label)
                || jump_targets.contains(label)
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.target == *label)
                || provenance.removed_pop_block_boundaries.contains(label)
                || copied(label)
        };
        for origin in &provenance.source_none_conditions {
            if !provenance.source_boundaries.contains(&origin.entry)
                || !provenance.block_boundaries.contains(&origin.entry)
                || copied(&origin.entry)
                || provenance
                    .setups
                    .iter()
                    .any(|(label, _)| *label == origin.entry)
                || provenance
                    .source_none_conditions
                    .iter()
                    .filter(|other| {
                        other.entry == origin.entry && other.instruction == origin.instruction
                    })
                    .count()
                    != 1
            {
                continue;
            }
            let entries: Vec<_> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| (*item == Item::Label(origin.entry)).then_some(index))
                .collect();
            let conditions: Vec<_> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    (*item == Item::Instruction(origin.instruction)).then_some(index)
                })
                .collect();
            let ([begin], [end]) = (entries.as_slice(), conditions.as_slice()) else {
                continue;
            };
            if begin >= end {
                continue;
            }
            let labels: Vec<_> = self.items[*begin..*end]
                .iter()
                .filter_map(|item| {
                    if let Item::Label(label) = item {
                        Some(*label)
                    } else {
                        None
                    }
                })
                .collect();
            if labels != origin.labels
                || labels.iter().any(|label| {
                    self.items
                        .iter()
                        .filter(|item| **item == Item::Label(*label))
                        .count()
                        != 1
                })
            {
                continue;
            }
            let Some(mut logical) = label_positions.get(&origin.entry).copied() else {
                continue;
            };
            let (entry_aliases, entry_pop) = self
                .source_none_entry_prefix(origin, *begin, *end, label_positions, &boundary, &copied)
                .unwrap_or_default();
            let mut removed = Vec::new();
            let mut null_setup_cells = FxHashSet::default();
            let mut retained = false;
            let mut valid = true;
            let mut previous = None;
            let mut tail = ExceptionOwnerTail::default();
            for item in &self.items[*begin + 1..*end] {
                match item {
                    Item::Label(label) => {
                        let entry_alias = previous.is_none() && entry_aliases.contains(label);
                        if boundary(label) && !entry_alias {
                            valid = false;
                            break;
                        }
                        let setups: Vec<_> = provenance
                            .setups
                            .iter()
                            .filter(|(setup, _)| setup == label)
                            .collect();
                        match setups.as_slice() {
                            [(_, Some(location))] => {
                                if !logical
                                    .checked_sub(1)
                                    .is_some_and(|index| retained_setup_nops.contains(&index))
                                    || !previous.is_some_and(|instruction: Instruction| {
                                        instruction.opcode == NOP
                                            && instruction.location == *location
                                    })
                                {
                                    valid = false;
                                    break;
                                }
                                // Adjacent same-line SETUPs may retain the second NOP,
                                // even though our location marker precedes both labels.
                                // Both original cells are NULL; do not infer which one
                                // supplied the surviving physical NOP from that marker.
                                null_setup_cells.insert(logical - 1 + removed.len());
                                retained = true;
                            }
                            [(_, None)] => {
                                let cell = logical + removed.len();
                                null_setup_cells.insert(cell);
                                removed.push(cell);
                            }
                            [] => {
                                let retained_range_endpoint =
                                    provenance.setup_nop_ranges.iter().any(|(start, end)| {
                                        (label == start || label == end)
                                            && labels.contains(start)
                                            && labels.contains(end)
                                            && label_positions.get(start).is_some_and(|index| {
                                                retained_setup_nops.contains(index)
                                                    && label_positions.get(end)
                                                        == Some(&(index + 1))
                                            })
                                    });
                                if !retained_range_endpoint
                                    && !provenance.continuation_boundaries.contains(label)
                                    && !entry_alias
                                {
                                    valid = false;
                                    break;
                                }
                            }
                            _ => {
                                valid = false;
                                break;
                            }
                        }
                    }
                    Item::Instruction(instruction) => {
                        let entry_pop = previous.is_none() && entry_pop == Some(*instruction);
                        if (instruction.local_policy.hidden_exception_owner_slots() != 0
                            && !entry_pop)
                            || instruction.pre_fold_extra_instructions() != 0
                            || instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Reachable
                            || (instruction.fusion_policy.is_converted_pop_block() && !entry_pop)
                            || instruction.jump_policy.borrow_empty_block()
                            || !matches!(instruction.operand(), Operand::Value(_))
                            || matches!(instruction.opcode, NOT_TAKEN | FOR_ITER | SEND)
                            || is_scope_exit(instruction.opcode)
                            || (instruction.opcode == NOP
                                && !retained_setup_nops.contains(&logical)
                                && !entry_pop)
                            || (instruction.opcode != NOP
                                && instruction.exception_policy()
                                    != super::ExceptionPolicy::default())
                        {
                            valid = false;
                            break;
                        }
                        tail.instruction(*instruction, previous);
                        if tail.insertions != 0 || tail.post_compaction_fusions != 0 {
                            valid = false;
                            break;
                        }
                        previous = Some(*instruction);
                        logical += 1;
                    }
                }
            }
            let condition = origin.instruction;
            if !valid
                || !retained
                || !null_setup_cells.contains(&(logical + 1))
                || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                || !matches!(condition.operand(), Operand::Forward(_))
                || condition.local_policy.hidden_exception_owner_slots() != 3
                || condition.pre_fold_extra_instructions() != 0
                || condition.no_location_policy.borrow_fallthrough()
                    != super::BorrowFallthrough::Reachable
                || condition.exception_policy() != super::ExceptionPolicy::default()
                || condition.jump_policy.borrow_empty_block()
            {
                continue;
            }
            // Region endpoints do not split the original instruction array. Only an
            // actual block entry or SETUP between the condition and marker invalidates it.
            for item in &self.items[*end + 1..] {
                match item {
                    Item::Label(label) => {
                        if boundary(label)
                            || provenance.setups.iter().any(|(setup, _)| setup == label)
                        {
                            break;
                        }
                    }
                    Item::Instruction(marker) => {
                        if marker.opcode == NOT_TAKEN
                            && marker.operand() == Operand::Value(0)
                            && marker.location == condition.location
                            && marker.exception_policy().owner()
                                == NormalizedExceptionOwner::Present
                            && marker.exception_policy().exclusion()
                                == super::ExceptionExclusion::None
                            && marker.local_policy.hidden_exception_owner_slots() == 0
                            && marker.pre_fold_extra_instructions() == 0
                            && !marker.jump_policy.borrow_empty_block()
                            && marker.no_location_policy.borrow_fallthrough()
                                == super::BorrowFallthrough::Reachable
                        {
                            result.insert(logical + 1);
                        }
                        break;
                    }
                }
            }
        }
        result
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
        let threaded_conditions = self.threaded_condition_positions();
        let setups: FxHashMap<_, _> = self
            .exception_owner_provenance
            .setups
            .iter()
            .copied()
            .collect();
        let mut label_positions = FxHashMap::default();
        let mut indexed_instructions = Vec::new();
        let mut jump_targets = FxHashSet::default();
        let mut position = 0;
        for item in &self.items {
            match item {
                Item::Label(label) => {
                    label_positions.insert(*label, position);
                }
                Item::Instruction(instruction) => {
                    indexed_instructions.push(instruction);
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
        let retained_setup_null_markers =
            self.retained_setup_null_markers(&label_positions, &jump_targets, &retained_setup_nops);
        let retained_normal_pop_terminators =
            self.retained_normal_pop_terminators(&jump_targets, &label_positions);
        let original_normal_finally_null_markers =
            self.original_normal_finally_null_markers(&jump_targets);
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
        let mut retained_pop_prefix = None;
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
        let mut pending_normal_pop_null_append = None;
        let mut original_array_start = 0;
        let mut pending_compacted_append = None;
        let mut pending_threaded_null_append = None;
        let mut has_uninterrupted_fusion = false;
        let mut instruction_index = 0;
        for item in &self.items {
            match item {
                Item::Label(label) => {
                    if boundaries.contains(label)
                        || jump_targets.contains(label)
                        || setups.contains_key(label)
                        || self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                    {
                        retained_pop_prefix = None;
                    }
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
                    if !continues_original_block {
                        has_uninterrupted_fusion = false;
                    }
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
                        || self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                    {
                        pending_threaded_null_append = None;
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
                        pending_normal_pop_null_append = None;
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
                        retained_pop_prefix = None;
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
                    // A surviving converted POP_BLOCK is an original NULL record, not
                    // an erased slot. Keep its preceding protected prefix separately from
                    // SETUP and removed-POP certificates. Only local fusion can select it.
                    if instruction.opcode == NOP {
                        retained_pop_prefix = None;
                        if instruction.fusion_policy.is_converted_pop_block()
                            // CPython removes the preceding authored return NOP during
                            // optimize_cfg, before insert_superinstructions compacts the
                            // live array. Its one hidden slot does not survive that fusion.
                            && instruction.local_policy.hidden_exception_owner_slots() <= 1
                            && instruction.pre_fold_extra_instructions() == 0
                            && instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Barrier
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions > 0
                            && original_array_start < instruction_index
                            && let Some(region) = self.uniform_exception_owner(
                                instruction_index - 1,
                                instruction_index,
                                &label_positions,
                            )
                            && !self.exception_regions[region].is_scope_fallback
                        {
                            let start = indexed_instructions
                                [original_array_start..instruction_index]
                                .iter()
                                .enumerate()
                                .rev()
                                .map(|(offset, instruction)| {
                                    (original_array_start + offset, instruction)
                                })
                                .take_while(|(index, instruction)| {
                                    instruction.exception_policy().exclusion()
                                        == super::ExceptionExclusion::None
                                        && instruction.exception_policy().owner()
                                            == NormalizedExceptionOwner::Unchanged
                                        && !owners.contains_key(index)
                                        && self.uniform_exception_owner(
                                            *index,
                                            *index + 1,
                                            &label_positions,
                                        ) == Some(region)
                                })
                                .map(|(index, _)| index)
                                .last();
                            if let Some(start) = start {
                                retained_pop_prefix = Some((
                                    original_array_start,
                                    start,
                                    instruction_index,
                                    tail,
                                    region,
                                ));
                            }
                        }
                    } else if instruction.exception_policy().exclusion()
                        != super::ExceptionExclusion::None
                        || instruction.no_location_policy.borrow_fallthrough()
                            == super::BorrowFallthrough::Barrier
                    {
                        retained_pop_prefix = None;
                    }
                    let compacted_append = pending_compacted_append.take();
                    let threaded_null_append = pending_threaded_null_append.take();
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
                        has_uninterrupted_fusion = false;
                        // An exact retained SETUP NOP is a known live array record.
                        // It preserves an existing POP_BLOCK epoch without reviving
                        // one lost at an unknown source join or borrow barrier.
                        pop_slot_epoch_known &= retained_setup_nops.contains(&instruction_index)
                            && instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Barrier;
                        ended_pop_slot = None;
                        pending_null_append = None;
                        pending_fresh_null_append = None;
                        pending_normal_pop_null_append = None;
                    }
                    let appends_to_null_slot =
                        pending_null_append.take() == Some(instruction_index);
                    let appends_to_fresh_null_slot =
                        pending_fresh_null_append.take() == Some(instruction_index);
                    let appends_to_normal_pop_null_slot =
                        pending_normal_pop_null_append.take() == Some(instruction_index);
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
                    let previous_fusions = tail.post_compaction_fusions;
                    tail.instruction(*instruction, previous);
                    if tail.post_compaction_fusions > previous_fusions
                        && instruction.no_location_policy.borrow_fallthrough()
                            != super::BorrowFallthrough::Barrier
                    {
                        has_uninterrupted_fusion = true;
                    }
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
                            proven_unprotected_slot = owner_history_exact
                                && tail.post_compaction_fusions == 0
                                && stale_slots > protected_original_instructions
                                && {
                                    let first_unprotected_setup = self
                                        .exception_owner_provenance
                                        .setups
                                        .first()
                                        .is_some_and(|(first, _)| *first == setup_label)
                                        && !self.exception_regions.iter().any(|region| {
                                            label_positions
                                                .get(&region.start)
                                                .is_some_and(|position| *position < start)
                                        });
                                    // Earlier arrays cannot supply this array's stale records.
                                    // A later SETUP has the same NULL prefix when no protected
                                    // region or removed SETUP precedes it in this original array.
                                    let unprotected_array_prefix =
                                        original_array_start < start
                                            && !self.exception_owner_provenance.setups.iter().any(
                                                |(label, _)| {
                                                    *label != setup_label
                                                        && label_positions.get(label).is_none_or(
                                                            |position| {
                                                                original_array_start <= *position
                                                                    && *position < start
                                                            },
                                                        )
                                                },
                                            )
                                            && !self.exception_regions.iter().any(|region| match (
                                                label_positions.get(&region.start),
                                                label_positions.get(&region.end),
                                            ) {
                                                (Some(begin), Some(end)) => {
                                                    *begin < start && *end > original_array_start
                                                }
                                                _ => true,
                                            });
                                    first_unprotected_setup || unprotected_array_prefix
                                };
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
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::DynamicCondition
                        )
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
                    // After fusion, the stale suffix is the pre-fusion live array.
                    // Include the retained NULL POP_BLOCK in the cleanup width. A slot
                    // strictly before it belongs to the certified protected prefix.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::DynamicCondition
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && let Some((array_start, start, end, end_tail, region)) =
                            retained_pop_prefix
                        && original_array_start == array_start
                        && tail.post_compaction_fusions >= end_tail.post_compaction_fusions
                    {
                        let removed = tail.post_compaction_fusions;
                        let suffix_width =
                            instruction_index - end + removed - end_tail.post_compaction_fusions;
                        // The checked live prefix is a conservative lower bound for its
                        // pre-fusion width; never infer owners before that checked span.
                        let prefix_width = end - start;
                        if removed > suffix_width && removed <= suffix_width + prefix_width {
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
                        && ((self
                            .exception_owner_provenance
                            .setups
                            .first()
                            .is_some_and(|(first, _)| *first == setup_label)
                            && !self.exception_regions.iter().any(|region| {
                                label_positions
                                    .get(&region.start)
                                    .is_some_and(|position| *position < start)
                            }))
                            // A later SETUP also owns a fresh original array when its
                            // retained NOP is the only record before the protected body.
                            // Earlier arrays cannot contribute stale slots across this
                            // authored boundary, even if an enclosing region began earlier.
                            || (start == original_array_start + 1
                                && self
                                    .exception_owner_provenance
                                    .block_boundaries
                                    .iter()
                                    .chain(
                                        &self
                                            .exception_owner_provenance
                                            .original_fallthrough_setups,
                                    )
                                    .any(|label| {
                                        label_positions.get(label) == Some(&original_array_start)
                                    })))
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
                    // This separate origin proof starts with one retained normal POP_BLOCK.
                    // A specialized None tail reuses its old IS_OP owner. Carry that
                    // certificate across the marker's exception-range endpoints, using
                    // the same path as other exact compacted-source owners.
                    if owner_history_exact
                        && let Some(owner) = retained_normal_pop_terminators.get(&instruction_index)
                    {
                        if let Some(owner) = owner {
                            pending_compacted_append = Some((instruction_index + 1, *owner));
                        } else {
                            pending_normal_pop_null_append = Some(instruction_index + 1);
                        }
                    }
                    if instruction.opcode == NOT_TAKEN
                        && (appends_to_null_slot
                            || appends_to_normal_pop_null_slot
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
                    // The threading pass appends a fresh NULL-owned conditional after
                    // turning its old record into a NOP. CPython removes early NOPs before
                    // local fusion. Exactly one fusion leaves the last live record (this
                    // conditional) in the first stale slot, irrespective of earlier NOPs.
                    // Late SETUP removal or prologue insertion would change that slot.
                    // A fusion before an unknown source join does not belong to this array.
                    if threaded_conditions.contains(&instruction_index)
                        && owner_history_exact
                        && has_uninterrupted_fusion
                        && tail.post_compaction_fusions == 1
                        && tail.insertions == 0
                        && tail.late_removals == 0
                    {
                        pending_threaded_null_append = Some(instruction_index + 1);
                    }
                    if threaded_null_append == Some(instruction_index) {
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
                    // The captured marker aliases may delimit exception ranges without
                    // introducing another original array or invalidating its fresh cell.
                    if original_normal_finally_null_markers.contains(&instruction_index) {
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
                    if retained_setup_null_markers.contains(&instruction_index) {
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
                    instruction_index += 1;
                    has_instruction = true;
                    if matches!(instruction.opcode, FOR_ITER | SEND) {
                        has_uninterrupted_fusion = false;
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
                        retained_pop_prefix = None;
                        original_array_start = instruction_index;
                        tail = ExceptionOwnerTail::default();
                        has_uninterrupted_fusion = false;
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
        let certified = self.uncompacted_conditional_null_markers(&jump_targets);
        for marker in 0..instruction_index {
            if !certified.contains(&marker) {
                continue;
            }
            owners.insert(
                marker,
                ExceptionOwnerDecision {
                    has_owner: false,
                    source_region: None,
                    definitive: true,
                    override_existing: true,
                },
            );
        }
        owners
    }

    /// Finds unchanged, unambiguous terminal pairs from the actual threading pass.
    fn threaded_condition_positions(&self) -> FxHashSet<usize> {
        let origins = &self.exception_owner_provenance.threaded_condition_origins;
        if origins.is_empty() {
            return FxHashSet::default();
        }
        let instructions = self
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        origins
            .iter()
            .filter_map(|origin| {
                if !matches!(origin.0.operand(), Operand::Forward(_))
                    || origin.0.exception_policy().exclusion() != super::ExceptionExclusion::Always
                    || origin.0.exception_policy().owner() != NormalizedExceptionOwner::Unchanged
                    || origin.1.exception_policy().exclusion() != super::ExceptionExclusion::None
                    || origin.1.exception_policy().owner()
                        != NormalizedExceptionOwner::DynamicConditionStrict
                {
                    return None;
                }
                // Copies with indistinguishable source positions cannot identify an origin.
                if origins.iter().filter(|other| *other == origin).count() != 1 {
                    return None;
                }
                let mut matches = instructions
                    .windows(2)
                    .enumerate()
                    .filter(|(_, pair)| pair[0] == origin.0 && pair[1] == origin.1);
                let (position, _) = matches.next()?;
                matches.next().is_none().then_some(position)
            })
            .collect()
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

    #[test]
    fn threaded_null_tail_requires_one_fusion_and_the_original_pair() {
        use crate::assembler::{
            Assembler, BorrowFallthrough, ExceptionRegion, ExceptionRegionPriority, Instruction,
            Item, Label, LayoutAssembler, Operand, SourceLocation, UnoptimizedCode,
        };
        use crate::target::opcodes::{
            COPY_FREE_VARS, JUMP_FORWARD, LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE,
            RETURN_VALUE, STORE_FAST, STORE_FAST_STORE_FAST,
        };

        for change in [
            "unchanged",
            "bookkeeping",
            "no origin",
            "two fusions",
            "prologue",
            "late setup",
            "opaque count",
            "retarget",
            "duplicate origin",
            "duplicate pair",
            "new join",
            "intervening barrier",
            "unknown source join",
        ] {
            let location = SourceLocation::new(4, 4, 0, 8);
            let plain = |opcode, operand| {
                Item::Instruction(Instruction::new(opcode, operand, location, None))
            };
            let mut items = vec![
                Item::Label(Label(0)),
                plain(NOP, Operand::Value(0)),
                plain(NOP, Operand::Value(0)),
                plain(STORE_FAST, Operand::Value(4)),
                plain(STORE_FAST, Operand::Value(5)),
                plain(LOAD_FAST, Operand::Value(0)),
                plain(POP_JUMP_IF_FALSE, Operand::Forward(Label(2))),
                plain(NOT_TAKEN, Operand::Value(0)),
                Item::Label(Label(1)),
                plain(LOAD_CONST, Operand::Value(0)),
                plain(RETURN_VALUE, Operand::Value(0)),
                Item::Label(Label(2)),
                plain(JUMP_FORWARD, Operand::Forward(Label(3))),
                Item::Label(Label(3)),
                plain(LOAD_CONST, Operand::Value(0)),
                plain(RETURN_VALUE, Operand::Value(0)),
                Item::Label(Label(4)),
                plain(LOAD_CONST, Operand::Value(0)),
                plain(RETURN_VALUE, Operand::Value(0)),
            ];
            if change == "two fusions" {
                items.insert(5, plain(LOAD_FAST, Operand::Value(1)));
            } else if change == "prologue" {
                items.insert(1, plain(COPY_FREE_VARS, Operand::Value(1)));
            }
            let mut pipeline = Assembler {
                code: UnoptimizedCode::Linear(items),
                exception_regions: vec![ExceptionRegion {
                    start: Label(0),
                    end: Label(1),
                    target: Label(4),
                    depth: 2,
                    preserve_lasti: true,
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(Label(0), Label(1)),
                }],
                next_label: 6,
                ..Assembler::default()
            }
            .into_optimization_pipeline();
            pipeline
                .conditional_threading_targets
                .insert(Label(2), location);
            assert!(pipeline.thread_original_conditional_jump_targets());
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .threaded_condition_origins
                    .len(),
                1
            );
            pipeline.fuse_superinstructions();
            pipeline.remove_redundant_nops();
            let fused_store = pipeline
                .graph
                .iter_items()
                .find_map(|item| match item {
                    Item::Instruction(i) if i.opcode == STORE_FAST_STORE_FAST => Some(i),
                    _ => None,
                })
                .unwrap();
            assert_eq!(fused_store.local_policy.hidden_exception_owner_slots(), 2);
            let mut assembler = LayoutAssembler {
                items: pipeline.graph.into_items(),
                exception_regions: pipeline.exception_regions,
                exception_owner_provenance: pipeline.exception_owner_provenance,
                preserved_block_boundaries: pipeline.preserved_block_boundaries,
                removed_max_depth: None,
                next_label: pipeline.next_label,
                has_next_location: false,
            };
            let marker_item = assembler
                .items
                .iter()
                .position(|item| matches!(item, Item::Instruction(i) if i.opcode == NOT_TAKEN))
                .unwrap();
            let origin = assembler
                .exception_owner_provenance
                .threaded_condition_origins[0];
            match change {
                "no origin" => assembler
                    .exception_owner_provenance
                    .threaded_condition_origins
                    .clear(),
                "late setup" => {
                    assembler.items.insert(1, Item::Label(Label(5)));
                    assembler
                        .exception_owner_provenance
                        .setups
                        .push((Label(5), None));
                }
                "opaque count" => {
                    let store = assembler
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            Item::Instruction(i) if i.opcode == STORE_FAST_STORE_FAST => Some(i),
                            _ => None,
                        })
                        .unwrap();
                    store.local_policy.add_hidden_exception_owner_slots(
                        crate::assembler::LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS,
                    );
                }
                "retarget" => {
                    let Item::Instruction(condition) = &mut assembler.items[marker_item - 1] else {
                        unreachable!();
                    };
                    condition.retarget(Operand::Forward(Label(1)));
                }
                "duplicate origin" => assembler
                    .exception_owner_provenance
                    .threaded_condition_origins
                    .push(origin),
                "duplicate pair" => assembler
                    .items
                    .extend([Item::Instruction(origin.0), Item::Instruction(origin.1)]),
                "new join" => {
                    assembler.items.insert(marker_item, Item::Label(Label(5)));
                    assembler.preserved_block_boundaries.insert(Label(5));
                }
                "bookkeeping" => {
                    assembler.items.insert(marker_item, Item::Label(Label(5)));
                }
                "intervening barrier" => {
                    let mut barrier = Instruction::new(NOP, Operand::Value(0), location, None);
                    barrier
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                    assembler
                        .items
                        .insert(marker_item - 1, Item::Instruction(barrier));
                }
                "unknown source join" => {
                    assembler
                        .items
                        .insert(marker_item - 1, Item::Label(Label(5)));
                }
                _ => {}
            }
            let marker_index = assembler
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .position(|instruction| instruction.opcode == NOT_TAKEN)
                .unwrap();
            let owners = assembler.dynamic_exception_owners();
            let decision = owners.get(&marker_index).expect("dynamic marker decision");
            assert_eq!(
                !decision.has_owner && decision.definitive && decision.override_existing,
                matches!(change, "unchanged" | "bookkeeping"),
                "{change}"
            );
        }
    }

    #[test]
    fn retained_setup_null_slot_requires_source_identity_and_an_exact_vacancy() {
        use super::{ExceptionOwnerProvenance, SourceNoneCondition};
        use crate::assembler::{
            BorrowFallthrough, Instruction, Item, Label, LayoutAssembler, NormalizedExceptionOwner,
            Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            COPY_FREE_VARS, JUMP_FORWARD, LOAD_CONST, LOAD_FAST, LOAD_FAST_LOAD_FAST, NOP,
            NOT_TAKEN, POP_JUMP_IF_NOT_NONE, RETURN_VALUE,
        };
        for change in [
            "null setup",
            "bookkeeping",
            "one setup owned load",
            "no origin",
            "retarget",
            "duplicate condition",
            "new join",
            "old target",
            "incoming jump",
            "copied entry",
            "retained nop changed",
            "prefix hidden",
            "extra count",
            "prologue",
            "fusion",
            "barrier",
            "multiline count",
            "duplicate label",
        ] {
            let location = SourceLocation::new(4, 4, 4, 12);
            let plain = |opcode, operand| {
                Item::Instruction(Instruction::new(opcode, operand, location, None))
            };
            let mut condition = Instruction::new(
                POP_JUMP_IF_NOT_NONE,
                Operand::Forward(Label(9)),
                location,
                None,
            );
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, None);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);
            let mut assembler = LayoutAssembler {
                items: vec![
                    Item::Label(Label(0)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label(1)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label(2)),
                    Item::Label(Label(3)),
                    plain(NOP, Operand::Value(0)),
                    Item::Label(Label(4)),
                    Item::Label(Label(5)),
                    Item::Label(Label(6)),
                    plain(LOAD_FAST, Operand::Value(0)),
                    Item::Instruction(condition),
                    Item::Instruction(marker),
                    Item::Label(Label(8)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                    Item::Label(Label(9)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                ],
                exception_regions: Vec::new(),
                exception_owner_provenance: ExceptionOwnerProvenance {
                    setups: vec![
                        (Label(1), None),
                        (Label(2), None),
                        (Label(5), Some(location)),
                        (Label(6), None),
                    ],
                    block_boundaries: [Label(0)].into_iter().collect(),
                    source_boundaries: [Label(0)].into_iter().collect(),
                    setup_nop_boundaries: [Label(3), Label(4)].into_iter().collect(),
                    setup_nop_ranges: vec![(Label(3), Label(4))],
                    synchronous_with_setups: [Label(1), Label(2)].into_iter().collect(),
                    source_none_conditions: vec![SourceNoneCondition {
                        entry: Label(0),
                        labels: (0..=6).map(Label).collect(),
                        instruction: condition,
                        entry_prefix: Some(vec![Item::Label(Label(0))]),
                    }],
                    ..ExceptionOwnerProvenance::default()
                },
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 11,
                has_next_location: false,
            };
            match change {
                "bookkeeping" => assembler.items.insert(12, Item::Label(Label(7))),
                "one setup owned load" => {
                    assembler.items.remove(4);
                    let provenance = &mut assembler.exception_owner_provenance;
                    provenance.setups.retain(|(label, _)| *label != Label(2));
                    provenance.source_none_conditions[0]
                        .labels
                        .retain(|label| *label != Label(2));
                    provenance.synchronous_with_setups.remove(&Label(2));
                }
                "no origin" => assembler
                    .exception_owner_provenance
                    .source_none_conditions
                    .clear(),
                "retarget" => {
                    let mut changed = condition;
                    changed.set_operand(Operand::Forward(Label(8)));
                    assembler.items[11] = Item::Instruction(changed);
                }
                "duplicate condition" => assembler.items.push(Item::Instruction(condition)),
                "new join" => assembler.items.insert(10, Item::Label(Label(10))),
                "old target" => {
                    assembler.preserved_block_boundaries.insert(Label(3));
                }
                "incoming jump" => assembler
                    .items
                    .push(plain(JUMP_FORWARD, Operand::Forward(Label(3)))),
                "copied entry" => {
                    assembler
                        .exception_owner_provenance
                        .normal_try_copy_blocks
                        .insert(Label(0));
                }
                "retained nop changed" => assembler.items[6] = plain(LOAD_CONST, Operand::Value(0)),
                "prefix hidden" => {
                    let Item::Instruction(mut instruction) = assembler.items[1] else {
                        unreachable!()
                    };
                    instruction.local_policy.add_hidden_exception_owner_slots(1);
                    assembler.items[1] = Item::Instruction(instruction);
                }
                "extra count" => {
                    let mut changed = condition;
                    changed.add_pre_fold_extra_instructions(1);
                    assembler.items[11] = Item::Instruction(changed);
                    assembler.exception_owner_provenance.source_none_conditions[0].instruction =
                        changed;
                }
                "prologue" => assembler.items[1] = plain(COPY_FREE_VARS, Operand::Value(1)),
                "fusion" => assembler.items[1] = plain(LOAD_FAST_LOAD_FAST, Operand::Value(1)),
                "barrier" => {
                    let Item::Instruction(mut instruction) = assembler.items[1] else {
                        unreachable!()
                    };
                    instruction
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                    assembler.items[1] = Item::Instruction(instruction);
                }
                "multiline count" => {
                    let mut changed = Instruction::new(
                        POP_JUMP_IF_NOT_NONE,
                        Operand::Forward(Label(9)),
                        location,
                        None,
                    );
                    changed.local_policy.add_hidden_exception_owner_slots(2);
                    assembler.items[11] = Item::Instruction(changed);
                    assembler.exception_owner_provenance.source_none_conditions[0].instruction =
                        changed;
                }
                "duplicate label" => assembler.items.push(Item::Label(Label(0))),
                _ => {}
            }
            let mut positions = rustc_hash::FxHashMap::default();
            let mut targets = rustc_hash::FxHashSet::default();
            let mut logical = 0;
            for item in &assembler.items {
                match item {
                    Item::Label(label) => {
                        positions.insert(*label, logical);
                    }
                    Item::Instruction(instruction) => {
                        if let Operand::Forward(label) | Operand::Backward(label) =
                            instruction.operand()
                        {
                            targets.insert(label);
                        }
                        logical += 1;
                    }
                }
            }
            let retained = assembler.retained_setup_nops(&positions);
            let markers = assembler.retained_setup_null_markers(&positions, &targets, &retained);
            assert_eq!(
                markers.contains(&5),
                matches!(change, "null setup" | "bookkeeping"),
                "{change}"
            );
            assert_eq!(
                markers.len(),
                usize::from(matches!(change, "null setup" | "bookkeeping")),
                "{change}"
            );
        }
    }

    #[test]
    fn source_none_setup_cells_require_the_exact_entry_prefix() {
        use super::{ExceptionOwnerProvenance, SourceNoneCondition};
        use crate::assembler::{
            BorrowFallthrough, ExceptionPolicy, Instruction, Item, Label, LayoutAssembler,
            NormalizedExceptionOwner, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            JUMP_FORWARD, LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_NOT_NONE,
            PUSH_EXC_INFO, RETURN_VALUE,
        };

        for change in [
            "normal pop",
            "return pop",
            "handler setup alias",
            "setup on source entry",
            "missing prefix",
            "uncaptured alias",
            "different nop lines",
            "interior return target",
            "two predecessors",
            "unknown interior label",
            "copied alias",
            "wrong early count",
            "changed pop opcode",
            "missing setup cell",
            "prefix barrier",
        ] {
            let returned = matches!(
                change,
                "return pop"
                    | "different nop lines"
                    | "interior return target"
                    | "wrong early count"
            );
            let handler = change == "handler setup alias";
            let location = SourceLocation::new(9, 9, 23, 38);
            let prefix_location = SourceLocation::new(4, 4, 12, 18);
            let setup_location = SourceLocation::new(8, 14, 16, 33);
            let plain =
                |opcode, operand, location| Instruction::new(opcode, operand, location, None);
            let mut source_pop = plain(
                NOP,
                Operand::Value(0),
                if returned {
                    prefix_location
                } else {
                    SourceLocation::NONE
                },
            );
            source_pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            source_pop.fusion_policy.mark_converted_pop_block();
            let mut current_pop = source_pop;
            current_pop.location = prefix_location;
            if returned {
                current_pop.local_policy.add_hidden_exception_owner_slots(1);
            }
            let mut source_prefix = vec![Item::Label(Label(0))];
            if returned {
                source_prefix.push(Item::Instruction(plain(
                    NOP,
                    Operand::Value(0),
                    prefix_location,
                )));
            }
            source_prefix.push(Item::Label(Label(10)));
            if !handler {
                source_prefix.push(Item::Instruction(source_pop));
            }
            let prefix_instruction = if handler {
                plain(PUSH_EXC_INFO, Operand::Value(0), SourceLocation::NONE)
            } else {
                current_pop
            };
            let mut condition = plain(POP_JUMP_IF_NOT_NONE, Operand::Forward(Label(9)), location);
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = plain(NOT_TAKEN, Operand::Value(0), location);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);
            let mut assembler = LayoutAssembler {
                items: vec![
                    Item::Instruction(plain(
                        JUMP_FORWARD,
                        Operand::Forward(Label(0)),
                        prefix_location,
                    )),
                    Item::Label(Label(0)),
                    Item::Label(Label(10)),
                    Item::Instruction(prefix_instruction),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Label(Label(1)),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Label(Label(2)),
                    Item::Label(Label(3)),
                    Item::Instruction(plain(NOP, Operand::Value(0), setup_location)),
                    Item::Label(Label(4)),
                    Item::Label(Label(5)),
                    Item::Label(Label(6)),
                    Item::Instruction(plain(LOAD_FAST, Operand::Value(0), location)),
                    Item::Instruction(condition),
                    Item::Instruction(marker),
                    Item::Label(Label(8)),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Instruction(plain(RETURN_VALUE, Operand::Value(0), location)),
                    Item::Label(Label(9)),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Instruction(plain(RETURN_VALUE, Operand::Value(0), location)),
                ],
                exception_regions: Vec::new(),
                exception_owner_provenance: ExceptionOwnerProvenance {
                    setups: vec![
                        (Label(1), None),
                        (Label(2), None),
                        (Label(5), Some(setup_location)),
                        (Label(6), None),
                    ],
                    block_boundaries: [Label(0)].into_iter().collect(),
                    source_boundaries: [Label(0)].into_iter().collect(),
                    setup_nop_boundaries: [Label(3), Label(4)].into_iter().collect(),
                    setup_nop_ranges: vec![(Label(3), Label(4))],
                    source_none_conditions: vec![SourceNoneCondition {
                        entry: Label(0),
                        labels: vec![
                            Label(0),
                            Label(10),
                            Label(1),
                            Label(2),
                            Label(3),
                            Label(4),
                            Label(5),
                            Label(6),
                        ],
                        instruction: condition,
                        entry_prefix: Some(source_prefix),
                    }],
                    ..ExceptionOwnerProvenance::default()
                },
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 12,
                has_next_location: false,
            };
            if handler {
                assembler
                    .exception_owner_provenance
                    .setups
                    .insert(0, (Label(10), None));
                assembler.preserved_block_boundaries.insert(Label(10));
            }
            match change {
                "setup on source entry" => assembler
                    .exception_owner_provenance
                    .setups
                    .insert(0, (Label(0), None)),
                "missing prefix" => {
                    assembler.exception_owner_provenance.source_none_conditions[0].entry_prefix =
                        None;
                }
                "uncaptured alias" => {
                    assembler.exception_owner_provenance.source_none_conditions[0]
                        .entry_prefix
                        .as_mut()
                        .unwrap()
                        .retain(|item| *item != Item::Label(Label(10)));
                }
                "different nop lines" => {
                    let prefix = assembler.exception_owner_provenance.source_none_conditions[0]
                        .entry_prefix
                        .as_mut()
                        .unwrap();
                    let Item::Instruction(instruction) = &mut prefix[1] else {
                        unreachable!()
                    };
                    instruction.location = location;
                }
                "interior return target" => {
                    assembler.preserved_block_boundaries.insert(Label(10));
                }
                "two predecessors" => assembler.items.push(Item::Instruction(plain(
                    JUMP_FORWARD,
                    Operand::Forward(Label(0)),
                    prefix_location,
                ))),
                "unknown interior label" => {
                    assembler.items.insert(5, Item::Label(Label(11)));
                    assembler.exception_owner_provenance.source_none_conditions[0]
                        .labels
                        .insert(2, Label(11));
                }
                "copied alias" => {
                    assembler
                        .exception_owner_provenance
                        .normal_try_copy_blocks
                        .insert(Label(10));
                }
                "wrong early count" => {
                    let Item::Instruction(instruction) = &mut assembler.items[3] else {
                        unreachable!()
                    };
                    instruction.local_policy.add_hidden_exception_owner_slots(1);
                }
                "changed pop opcode" => {
                    assembler.items[3] =
                        Item::Instruction(plain(LOAD_CONST, Operand::Value(0), prefix_location));
                }
                "missing setup cell" => assembler
                    .exception_owner_provenance
                    .setups
                    .retain(|(label, _)| *label != Label(6)),
                "prefix barrier" => {
                    let Item::Instruction(instruction) = &mut assembler.items[3] else {
                        unreachable!()
                    };
                    instruction
                        .no_location_policy
                        .set_borrow_fallthrough(BorrowFallthrough::Barrier);
                }
                _ => {}
            }
            let mut positions = rustc_hash::FxHashMap::default();
            let mut targets = rustc_hash::FxHashSet::default();
            let mut logical = 0;
            let mut marker_index = None;
            for item in &assembler.items {
                match item {
                    Item::Label(label) => {
                        positions.insert(*label, logical);
                    }
                    Item::Instruction(instruction) => {
                        if let Operand::Forward(label) | Operand::Backward(label) =
                            instruction.operand()
                        {
                            targets.insert(label);
                        }
                        if instruction.opcode == NOT_TAKEN {
                            marker_index = Some(logical);
                        }
                        logical += 1;
                    }
                }
            }
            let retained = assembler.retained_setup_nops(&positions);
            let markers = assembler.retained_setup_null_markers(&positions, &targets, &retained);
            let positive = matches!(change, "normal pop" | "return pop" | "handler setup alias");
            assert_eq!(
                markers.contains(&marker_index.unwrap()),
                positive,
                "{change}"
            );
            assert_eq!(markers.len(), usize::from(positive), "{change}");
        }
    }

    #[test]
    fn retained_setup_null_marker_allows_only_bookkeeping_region_ends() {
        use super::{ExceptionOwnerProvenance, SourceNoneCondition};
        use crate::assembler::{
            ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label, LayoutAssembler,
            NormalizedExceptionOwner, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            JUMP_FORWARD, LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_NOT_NONE, RETURN_VALUE,
        };

        let location = SourceLocation::new(4, 4, 4, 12);
        let plain =
            |opcode, operand| Item::Instruction(Instruction::new(opcode, operand, location, None));
        let fixture = || {
            let mut condition = Instruction::new(
                POP_JUMP_IF_NOT_NONE,
                Operand::Forward(Label(9)),
                location,
                None,
            );
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, None);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);
            LayoutAssembler {
                items: vec![
                    Item::Label(Label(0)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label(1)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label(2)),
                    Item::Label(Label(3)),
                    plain(NOP, Operand::Value(0)),
                    Item::Label(Label(4)),
                    Item::Label(Label(5)),
                    Item::Label(Label(6)),
                    plain(LOAD_FAST, Operand::Value(0)),
                    Item::Instruction(condition),
                    Item::Label(Label(7)),
                    Item::Instruction(marker),
                    Item::Label(Label(8)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                    Item::Label(Label(9)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                ],
                exception_regions: vec![ExceptionRegion {
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(Label(0), Label(8)),
                    start: Label(0),
                    end: Label(7),
                    target: Label(9),
                    depth: 0,
                    preserve_lasti: false,
                }],
                exception_owner_provenance: ExceptionOwnerProvenance {
                    setups: vec![
                        (Label(1), None),
                        (Label(2), None),
                        (Label(5), Some(location)),
                        (Label(6), None),
                    ],
                    block_boundaries: [Label(0)].into_iter().collect(),
                    source_boundaries: [Label(0)].into_iter().collect(),
                    setup_nop_boundaries: [Label(3), Label(4)].into_iter().collect(),
                    setup_nop_ranges: vec![(Label(3), Label(4))],
                    synchronous_with_setups: [Label(1), Label(2)].into_iter().collect(),
                    source_none_conditions: vec![SourceNoneCondition {
                        entry: Label(0),
                        labels: (0..=6).map(Label).collect(),
                        instruction: condition,
                        entry_prefix: Some(vec![Item::Label(Label(0))]),
                    }],
                    ..ExceptionOwnerProvenance::default()
                },
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 10,
                has_next_location: false,
            }
        };

        for boundary in [
            "region end only",
            "jump target",
            "source boundary",
            "block boundary",
            "setup boundary",
            "pop boundary",
            "copied boundary",
            "preserved boundary",
            "handler target",
        ] {
            let mut assembler = fixture();
            let provenance = &mut assembler.exception_owner_provenance;
            match boundary {
                "region end only" => {}
                "jump target" => {
                    assembler
                        .items
                        .push(plain(JUMP_FORWARD, Operand::Forward(Label(7))));
                }
                "source boundary" => {
                    provenance.source_boundaries.insert(Label(7));
                }
                "block boundary" => {
                    provenance.block_boundaries.insert(Label(7));
                }
                "setup boundary" => {
                    provenance.setups.push((Label(7), None));
                }
                "pop boundary" => {
                    provenance.removed_pop_block_boundaries.insert(Label(7));
                }
                "copied boundary" => {
                    provenance.normal_try_copy_blocks.insert(Label(7));
                }
                "preserved boundary" => {
                    assembler.preserved_block_boundaries.insert(Label(7));
                }
                "handler target" => {
                    assembler.exception_regions[0].target = Label(7);
                }
                _ => unreachable!(),
            }
            let mut positions = rustc_hash::FxHashMap::default();
            let mut targets = rustc_hash::FxHashSet::default();
            let mut logical = 0;
            for item in &assembler.items {
                match item {
                    Item::Label(label) => {
                        positions.insert(*label, logical);
                    }
                    Item::Instruction(instruction) => {
                        if let Operand::Forward(label) | Operand::Backward(label) =
                            instruction.operand()
                        {
                            targets.insert(label);
                        }
                        logical += 1;
                    }
                }
            }
            let retained = assembler.retained_setup_nops(&positions);
            let markers = assembler.retained_setup_null_markers(&positions, &targets, &retained);
            let expected: rustc_hash::FxHashSet<usize> = if boundary == "region end only" {
                [5].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            };
            assert_eq!(markers, expected, "{boundary}");
        }
    }

    #[test]
    fn ordinary_normal_finally_markers_require_the_unchanged_original_array() {
        use super::{ExceptionOwnerProvenance, OriginalNormalFinallyArray};
        use crate::assembler::{
            ExceptionPolicy, Instruction, Item, Label, LayoutAssembler, NormalizedExceptionOwner,
            Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            JUMP_BACKWARD, JUMP_FORWARD, LOAD_FAST, LOAD_GLOBAL, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE,
            TO_BOOL,
        };
        for change in [
            "unchanged",
            "propagated pop location",
            "missing incoming",
            "wrong incoming target",
            "conditional incoming",
            "backward incoming",
            "missing source boundary",
            "jump entry",
            "preserved entry",
            "copied entry",
            "setup alias",
            "interior source boundary",
            "foreign inserted label",
            "missing pop",
            "hidden global slot",
            "nonconverted pop",
            "changed branch",
            "duplicate entry",
            "already owned marker",
            "omitted nop count",
            "omitted nop location",
            "fusion alias",
            "setup nop alias",
            "continuation only",
            "copied jump successor",
        ] {
            let entry = Label(0);
            let range_end = Label(1);
            let marker_alias = Label(2);
            let exit = Label(3);
            let location = SourceLocation::new(9, 9, 15, 19);
            let plain = |opcode, operand| Instruction::new(opcode, operand, location, None);
            let mut pop = plain(NOP, Operand::Value(0));
            pop.location = SourceLocation::NONE;
            pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            pop.fusion_policy.mark_converted_pop_block();
            let mut marker = plain(NOT_TAKEN, Operand::Value(0));
            marker.set_exception_owner(NormalizedExceptionOwner::Absent);
            let prefix = vec![
                Item::Label(entry),
                Item::Instruction(pop),
                Item::Label(range_end),
                Item::Instruction(plain(LOAD_FAST, Operand::Value(0))),
                Item::Instruction(plain(TO_BOOL, Operand::Value(0))),
                Item::Instruction(plain(POP_JUMP_IF_FALSE, Operand::Forward(exit))),
                Item::Label(marker_alias),
                Item::Instruction(marker),
            ];
            let mut assembler = LayoutAssembler {
                items: prefix.clone(),
                exception_regions: Vec::new(),
                exception_owner_provenance: ExceptionOwnerProvenance {
                    source_boundaries: [entry].into_iter().collect(),
                    block_boundaries: [entry].into_iter().collect(),
                    original_normal_finally_arrays: vec![OriginalNormalFinallyArray {
                        entry,
                        prefix,
                        incoming: vec![plain(JUMP_FORWARD, Operand::Forward(entry))],
                    }],
                    ..ExceptionOwnerProvenance::default()
                },
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 5,
                has_next_location: false,
            };
            let mut targets = rustc_hash::FxHashSet::default();
            match change {
                "unchanged" => {}
                "propagated pop location" => {
                    let Item::Instruction(pop) = &mut assembler.items[1] else {
                        unreachable!()
                    };
                    pop.location = location;
                }
                "missing incoming" => assembler
                    .exception_owner_provenance
                    .original_normal_finally_arrays[0]
                    .incoming
                    .clear(),
                "wrong incoming target" => {
                    assembler
                        .exception_owner_provenance
                        .original_normal_finally_arrays[0]
                        .incoming[0] = plain(JUMP_FORWARD, Operand::Forward(exit));
                }
                "conditional incoming" => {
                    assembler
                        .exception_owner_provenance
                        .original_normal_finally_arrays[0]
                        .incoming[0] = plain(POP_JUMP_IF_FALSE, Operand::Forward(entry));
                }
                "backward incoming" => {
                    assembler
                        .exception_owner_provenance
                        .original_normal_finally_arrays[0]
                        .incoming[0] = plain(JUMP_BACKWARD, Operand::Backward(entry));
                }
                "omitted nop count" => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_counts
                        .insert(marker_alias, 1);
                }
                "omitted nop location" => {
                    assembler
                        .exception_owner_provenance
                        .omitted_nop_locations
                        .insert(range_end, location);
                }
                "fusion alias" => {
                    assembler
                        .exception_owner_provenance
                        .fusion_boundaries
                        .insert(range_end);
                }
                "setup nop alias" => {
                    assembler
                        .exception_owner_provenance
                        .setup_nop_boundaries
                        .insert(range_end);
                }
                "continuation only" => {
                    assembler
                        .exception_owner_provenance
                        .continuation_boundaries
                        .insert(marker_alias);
                }
                "copied jump successor" => {
                    assembler
                        .exception_owner_provenance
                        .normal_try_copy_jump_nop_successors
                        .insert(marker_alias);
                }
                "missing source boundary" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .clear();
                }
                "jump entry" => {
                    targets.insert(entry);
                }
                "preserved entry" => {
                    assembler.preserved_block_boundaries.insert(entry);
                }
                "copied entry" => {
                    assembler
                        .exception_owner_provenance
                        .normal_try_copy_blocks
                        .insert(entry);
                }
                "setup alias" => assembler
                    .exception_owner_provenance
                    .setups
                    .push((range_end, None)),
                "interior source boundary" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .insert(marker_alias);
                }
                "foreign inserted label" => assembler.items.insert(4, Item::Label(Label(4))),
                "missing pop" => {
                    assembler.items.remove(1);
                }
                "hidden global slot" => {
                    let load = Item::Instruction(plain(LOAD_GLOBAL, Operand::Value(1)));
                    assembler.items[3] = load;
                    assembler
                        .exception_owner_provenance
                        .original_normal_finally_arrays[0]
                        .prefix[3] = load;
                }
                "nonconverted pop" => {
                    let mut nonconverted = pop;
                    nonconverted.fusion_policy = crate::assembler::ir::FusionPolicy::default();
                    let nonconverted = Item::Instruction(nonconverted);
                    assembler.items[1] = nonconverted;
                    assembler
                        .exception_owner_provenance
                        .original_normal_finally_arrays[0]
                        .prefix[1] = nonconverted;
                }
                "changed branch" => {
                    assembler.items[5] =
                        Item::Instruction(plain(POP_JUMP_IF_FALSE, Operand::Forward(Label(4))));
                }
                "duplicate entry" => assembler.items.push(Item::Label(entry)),
                "already owned marker" => {
                    let mut owned = marker;
                    owned.set_exception_owner(NormalizedExceptionOwner::Present);
                    assembler.items[7] = Item::Instruction(owned);
                    assembler
                        .exception_owner_provenance
                        .original_normal_finally_arrays[0]
                        .prefix[7] = Item::Instruction(owned);
                }
                _ => unreachable!(),
            }
            let expected = if matches!(
                change,
                "unchanged" | "propagated pop location" | "continuation only"
            ) {
                [4].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            };
            assert_eq!(
                assembler.original_normal_finally_null_markers(&targets),
                expected,
                "{change}"
            );
        }
    }

    #[test]
    fn uncompacted_arrays_require_original_boundaries_and_unchanged_records() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{
            Instruction, Item, Label, LayoutAssembler, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            COPY, JUMP_BACKWARD, JUMP_FORWARD, LOAD_ATTR, LOAD_CONST, LOAD_FAST, LOAD_FAST_BORROW,
            NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, RETURN_VALUE, TO_BOOL,
        };

        for case in [
            "primitive",
            "authored break",
            "borrowed",
            "range alias",
            "unknown origin",
            "changed origin",
            "duplicate origin",
            "unlocated setup",
            "same-line setup",
            "hidden record",
            "folded record",
            "constant conversion",
            "local fusion",
            "copy record",
            "omitted record",
            "interior entry",
            "changed policy",
            "duplicate final entry",
        ] {
            let location = |line| SourceLocation::new(line, line, 0, 4);
            let plain =
                |opcode, operand, line| Instruction::new(opcode, operand, location(line), None);
            let incoming = plain(POP_JUMP_IF_FALSE, Operand::Forward(Label(1)), 1);
            let mut items = vec![
                Item::Instruction(incoming),
                Item::Instruction(plain(NOT_TAKEN, Operand::Value(0), 1)),
                Item::Instruction(plain(JUMP_BACKWARD, Operand::Backward(Label(9)), 1)),
                Item::Label(Label(1)),
                Item::Label(Label(2)),
                // Generator SETUP markers may have the default exception policy.
                Item::Instruction(plain(NOP, Operand::Value(0), 2)),
                Item::Label(Label(3)),
                Item::Label(Label(4)),
                Item::Instruction(plain(LOAD_FAST, Operand::Value(0), 3)),
                Item::Instruction(plain(LOAD_ATTR, Operand::Value(0), 3)),
                Item::Instruction(plain(TO_BOOL, Operand::Value(0), 3)),
                Item::Instruction(plain(POP_JUMP_IF_FALSE, Operand::Forward(Label(5)), 3)),
                Item::Instruction(plain(NOT_TAKEN, Operand::Value(0), 3)),
                Item::Label(Label(5)),
                Item::Instruction(plain(RETURN_VALUE, Operand::Value(0), 4)),
            ];
            let mut provenance = ExceptionOwnerProvenance {
                original_primitive_condition_boundaries: vec![incoming],
                setups: vec![(Label(4), Some(location(2)))],
                setup_nop_ranges: vec![(Label(2), Label(3))],
                ..ExceptionOwnerProvenance::default()
            };
            match case {
                "authored break" => {
                    let mut jump = plain(JUMP_FORWARD, Operand::Forward(Label(1)), 1);
                    jump.fusion_policy.mark_authored_break();
                    items[0] = Item::Instruction(jump);
                    provenance.original_primitive_condition_boundaries.clear();
                }
                "unknown origin" => provenance.original_primitive_condition_boundaries.clear(),
                "changed origin" => {
                    items[0] =
                        Item::Instruction(plain(POP_JUMP_IF_FALSE, Operand::Forward(Label(6)), 1));
                }
                "duplicate origin" => items.push(Item::Instruction(incoming)),
                "unlocated setup" => {
                    items[5] = Item::Instruction(Instruction::new(
                        NOP,
                        Operand::Value(0),
                        SourceLocation::NONE,
                        None,
                    ));
                    provenance.setups[0].1 = Some(SourceLocation::NONE);
                }
                "same-line setup" => {
                    items[5] = Item::Instruction(plain(NOP, Operand::Value(0), 3));
                    provenance.setups[0].1 = Some(location(3));
                }
                "hidden record" => {
                    let Item::Instruction(instruction) = &mut items[8] else {
                        unreachable!()
                    };
                    instruction.local_policy.add_hidden_exception_owner_slots(1);
                }
                "folded record" => {
                    let Item::Instruction(instruction) = &mut items[8] else {
                        unreachable!()
                    };
                    instruction.add_pre_fold_extra_instructions(1);
                }
                "constant conversion" => {
                    items[9] = Item::Instruction(plain(LOAD_CONST, Operand::Value(0), 3));
                }
                "local fusion" => {
                    items[9] = Item::Instruction(plain(LOAD_FAST, Operand::Value(1), 3));
                }
                "copy record" => items[9] = Item::Instruction(plain(COPY, Operand::Value(1), 3)),
                "omitted record" => {
                    provenance.omitted_nop_counts.insert(Label(4), 1);
                }
                "interior entry" => items.push(Item::Instruction(plain(
                    JUMP_FORWARD,
                    Operand::Forward(Label(4)),
                    4,
                ))),
                _ => {}
            }
            provenance.capture_uncompacted_conditional_arrays(&items);
            let mut assembler = LayoutAssembler {
                items,
                exception_owner_provenance: provenance,
                exception_regions: vec![],
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 11,
                has_next_location: false,
            };
            match case {
                "borrowed" => {
                    let Item::Instruction(instruction) = &mut assembler.items[8] else {
                        unreachable!()
                    };
                    instruction.opcode = LOAD_FAST_BORROW;
                }
                "range alias" => assembler.items.insert(12, Item::Label(Label(10))),
                "changed policy" => {
                    let Item::Instruction(instruction) = &mut assembler.items[8] else {
                        unreachable!()
                    };
                    instruction.jump_policy.prevent_threading_target();
                }
                "duplicate final entry" => assembler.items.push(Item::Label(Label(1))),
                _ => {}
            }
            let targets = assembler
                .items
                .iter()
                .filter_map(|item| {
                    if let Item::Instruction(instruction) = item
                        && let Operand::Forward(label) | Operand::Backward(label) =
                            instruction.operand()
                    {
                        Some(label)
                    } else {
                        None
                    }
                })
                .collect();
            let expected = if matches!(
                case,
                "primitive" | "authored break" | "borrowed" | "range alias"
            ) {
                [8].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            };
            assert_eq!(
                assembler.uncompacted_conditional_null_markers(&targets),
                expected,
                "{case}"
            );
        }
    }

    #[test]
    fn retained_pop_none_tail_requires_exact_protected_slot_history() {
        use super::{ExceptionOwnerProvenance, RetainedNormalPopBlock};
        use crate::assembler::{
            ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
            LayoutAssembler, NormalizedExceptionOwner, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            LOAD_ATTR, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, RERAISE,
            RETURN_VALUE,
        };

        for case in [
            "none",
            "not none",
            "present",
            "hidden two",
            "prior fold",
            "foreign label",
            "unknown entry",
            "missing pop",
            "unpreserved pop",
            "only fallback",
            "protected gap",
            "excluded marker",
        ] {
            let entry = Label(0);
            let body = Label(1);
            let exit = Label(2);
            let marker_start = Label(3);
            let marker_end = Label(4);
            let handler = Label(5);
            let fallback = Label(6);
            let location = SourceLocation::new(3, 3, 4, 20);
            let plain = |opcode, operand| Instruction::new(opcode, operand, location, None);
            let mut pop = plain(NOP, Operand::Value(0));
            pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            pop.fusion_policy.mark_converted_pop_block();
            pop.jump_policy.preserve_explicit_nop();
            let opcode = if case == "not none" {
                POP_JUMP_IF_NOT_NONE
            } else {
                POP_JUMP_IF_NONE
            };
            let mut conditional = plain(opcode, Operand::Forward(exit));
            conditional
                .local_policy
                .add_hidden_exception_owner_slots(if case == "hidden two" { 2 } else { 3 });
            let mut first = plain(LOAD_FAST, Operand::Value(0));
            if case == "prior fold" {
                first.add_pre_fold_extra_instructions(1);
            }
            if case == "missing pop" {
                pop = plain(NOP, Operand::Value(0));
            }
            if case == "unpreserved pop" {
                pop = plain(NOP, Operand::Value(0));
                pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                pop.fusion_policy.mark_converted_pop_block();
            }
            let mut marker = plain(NOT_TAKEN, Operand::Value(0));
            marker.set_exception_owner(if case == "present" {
                NormalizedExceptionOwner::Present
            } else {
                NormalizedExceptionOwner::DynamicCondition
            });
            if case == "excluded marker" {
                marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            }
            let mut items = vec![
                Item::Label(entry),
                Item::Instruction(pop),
                Item::Label(body),
                Item::Instruction(first),
                Item::Instruction(plain(LOAD_ATTR, Operand::Value(0))),
                Item::Instruction(conditional),
                Item::Label(marker_start),
                Item::Instruction(marker),
                Item::Label(marker_end),
                Item::Label(exit),
                Item::Instruction(plain(RETURN_VALUE, Operand::Value(0))),
                Item::Label(handler),
                Item::Instruction(plain(RERAISE, Operand::Value(0))),
                Item::Label(fallback),
                Item::Instruction(plain(RERAISE, Operand::Value(0))),
            ];
            if matches!(case, "foreign label" | "protected gap") {
                items.insert(4, Item::Label(Label(7)));
            }
            let mut provenance = ExceptionOwnerProvenance {
                source_boundaries: [entry].into_iter().collect(),
                block_boundaries: [entry].into_iter().collect(),
                retained_normal_pop_blocks: [(
                    entry,
                    RetainedNormalPopBlock {
                        before: vec![entry],
                        after: vec![body],
                    },
                )]
                .into_iter()
                .collect(),
                ..ExceptionOwnerProvenance::default()
            };
            if case == "unknown entry" {
                provenance.source_boundaries.clear();
            }
            let protected_start = if case == "protected gap" {
                Label(7)
            } else {
                body
            };
            let mut regions = vec![
                ExceptionRegion {
                    start: protected_start,
                    end: marker_start,
                    target: handler,
                    depth: 2,
                    preserve_lasti: true,
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(protected_start, marker_start),
                },
                ExceptionRegion {
                    start: entry,
                    end: exit,
                    target: fallback,
                    depth: 0,
                    preserve_lasti: true,
                    is_scope_fallback: true,
                    priority: ExceptionRegionPriority::OriginalRange(entry, exit),
                },
            ];
            if case == "only fallback" {
                regions.remove(0);
            }
            let assembler = LayoutAssembler {
                items,
                exception_regions: regions,
                exception_owner_provenance: provenance,
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 8,
                has_next_location: false,
            };
            let mut position = 0;
            let positions = assembler
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some((*label, position)),
                    Item::Instruction(_) => {
                        position += 1;
                        None
                    }
                })
                .collect();
            let certified = assembler
                .retained_normal_pop_terminators(&[exit].into_iter().collect(), &positions);
            let expected = matches!(case, "none" | "not none" | "present" | "excluded marker");
            assert_eq!(
                certified.get(&3).copied(),
                expected.then_some(Some(0)),
                "{case}"
            );
            let decisions = assembler.dynamic_exception_owners();
            if matches!(case, "none" | "not none" | "present") {
                let decision = decisions.get(&4).expect("marker after region-end label");
                assert_eq!(decision.source_region, Some(0), "{case}");
                assert!(
                    decision.has_owner && decision.definitive && decision.override_existing,
                    "{case}"
                );
            } else if case == "excluded marker" {
                assert!(
                    !decisions.get(&4).is_some_and(|decision| decision.has_owner),
                    "{case}"
                );
            }
        }
    }
}
