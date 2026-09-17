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
    // Authored ordinary-try SETUP and its original handler, independently of split ranges.
    pub(super) ordinary_try_setups: FxHashMap<Label, Label>,
    // Original ordinary-await suspension regions, resolved alongside their live regions.
    pub(super) ordinary_await_cleanups: Vec<super::ExceptionRegion>,
    // Authored SEND, completion entry, and END_SEND before any cleanup or copying.
    pub(super) ordinary_await_ends: Vec<OrdinaryAwaitEnd>,
    // Authored return NOP, immediately following finally POP, and original try target.
    pub(super) early_return_nop_pairs: Vec<(Instruction, Instruction, Label)>,
    pub(super) copied_handler_cleanups: Vec<CopiedHandlerCleanup>,
    pub(super) named_handler_pop_pairs: Vec<OriginalPopBlockSpan>,
    pub(super) copied_handler_finally_pops: Vec<OriginalPopBlockSpan>,
    pub(super) fused_manager_owners: Vec<(Instruction, Instruction, super::ExceptionRegion)>,
    pub(super) handler_dispatch_arrays: Vec<HandlerDispatchArray>,
    pub(super) block_boundaries: FxHashSet<Label>,
    // Original entries that split borrowed-load analysis without changing the physical CFG.
    pub(super) borrow_only_block_boundaries: FxHashSet<Label>,
    pub(super) source_boundaries: FxHashSet<Label>,
    pub(super) continuation_boundaries: FxHashSet<Label>,
    // Synthetic range starts before value-preserving boolean jumps.
    pub(super) value_jump_continuation_boundaries: FxHashSet<Label>,
    // Region re-entry after a return emits no original instruction or unwind slot.
    pub(super) return_region_reentries: FxHashSet<Label>,
    // Exact copied-finally CALL/POP and its artificial iterator-disposal/return array.
    pub(super) copied_finally_iterator_returns: Vec<Vec<Item>>,
    pub(super) fusion_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_ranges: Vec<(Label, Label)>,
    // Retained SETUPs emitted directly into an authored primitive conditional fallthrough.
    pub(super) original_fallthrough_setups: FxHashSet<Label>,
    // One authored normal POP_BLOCK, not an extra synthetic end record or a copied exit.
    pub(super) retained_normal_pop_blocks: FxHashMap<Label, RetainedNormalPopBlock>,
    pub(super) original_normal_finally_arrays: Vec<OriginalNormalFinallyArray>,
    pub(super) normal_finally_none_prefixes: Vec<(Instruction, Instruction, Instruction)>,
    pub(super) copied_return_none_prefixes: Vec<CopiedReturnNonePrefix>,
    pub(super) copied_return_truth_prefixes: Vec<CopiedReturnTruthPrefix>,
    pub(super) copied_handler_return_none_prefixes: Vec<(Label, [Instruction; 6])>,
    pub(super) copied_manager_return_none_prefixes: Vec<CopiedManagerReturnNonePrefix>,
    pub(super) copied_manager_return_compactions: Vec<CopiedManagerReturnCompaction>,
    pub(super) uncompacted_conditional_arrays: Vec<UncompactedConditionalArray>,
    pub(super) original_primitive_condition_boundaries: Vec<Instruction>,
    // Authored normal exits and their immediate, fully emitted restore records.
    pub(super) original_comprehension_normal_exits: Vec<(Label, Instruction)>,
    pub(super) original_comprehension_normal_restores: FxHashMap<Label, Vec<Instruction>>,
    // Original simple For target stores, before their tracing NOP is compacted.
    pub(super) for_target_owner_prefixes: Vec<(Label, Instruction)>,
    // Exact final normal restore, after its fusion policies are complete.
    pub(super) comprehension_restore_owner_prefixes: Vec<(Label, Instruction)>,
    pub(super) retired_comprehension_normal_entries: FxHashSet<Label>,
    pub(super) synchronous_with_setups: FxHashSet<Label>,
    pub(super) async_with_setups: Vec<AsyncWithSetup>,
    pub(super) removed_pop_block_boundaries: FxHashSet<Label>,
    pub(super) removed_pop_block_widths: FxHashMap<Label, u16>,
    // Original ordinary-try exits contain POP_BLOCK followed by JUMP_NO_INTERRUPT.
    pub(super) normal_try_exits: Vec<(Label, Label)>,
    // Exact final wildcard call/discard or class-pattern failure pops at a normal try exit.
    pub(super) normal_try_fallthrough_donors: FxHashMap<Label, (Instruction, Instruction)>,
    // Original adjacent arrays containing the two final class-pattern failure pops.
    pub(super) normal_try_class_pattern_donor_labels: FxHashMap<Label, (Label, Label)>,
    // First records of the original manager cleanup arrays, before line resolution.
    pub(super) original_manager_pop_blocks: Vec<Label>,
    // Ordinary handler exits copied from an original unlocated POP_EXCEPT/jump array.
    pub(super) original_handler_cleanup_join_exits: FxHashSet<Label>,
    // Original try/except POP_BLOCK exits inside their own finally, for owner transfer
    // and late redundant-exit removal, but not ordinary-try exit copying.
    pub(super) compound_try_owner_exits: Vec<(Label, Label)>,
    // Populated authored else arrays with direct normal edges into their own finally.
    pub(super) populated_try_else_borrow_exits: Vec<(Label, Label)>,
    pub(super) omitted_nop_locations: FxHashMap<Label, SourceLocation>,
    pub(super) omitted_nop_counts: FxHashMap<Label, usize>,
    // One omitted fallthrough marker with no original label: item offset, location, count.
    pub(super) omitted_nop_tail: Option<(usize, SourceLocation, usize)>,
    // Original constant-expression/Pass pairs sharing only their fresh marker labels.
    pub(super) constant_pass_clusters: Vec<ConstantPassCluster>,
    // Original owners of branch jumps excluded only by an eager cleanup-copy prediction.
    pub(super) normal_try_jump_owners: FxHashMap<(Label, Label), Vec<super::ExceptionRegion>>,
    pub(super) normal_try_copy_blocks: FxHashSet<Label>,
    pub(super) normal_try_copy_steps: Vec<(Label, Label, usize)>,
    // Captured source/continuation labels whose instructions were erased by reachability pruning.
    pub(super) retired_normal_try_copy_labels: FxHashSet<Label>,
    // Candidate original fallthrough stores; only a successful static swap certifies them.
    pub(super) normal_try_copy_join_origins: FxHashMap<Label, TupleJoinLocationOrigin>,
    // Exact original tuple-join jump and the assignment donor certified before its exchange.
    // Copies inherit their own predecessors; this donor is consumed only once.
    pub(super) normal_try_copy_join_donors: FxHashMap<Label, (Instruction, SourceLocation)>,
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
    // Exact survivors of literal collection folds whose copy budget saturated.
    pub(super) original_uniform_folds: Vec<Instruction>,
    // Exact source comparison and original labels, before any jump copying or cleanup.
    pub(super) source_none_conditions: Vec<SourceNoneCondition>,
    // Exact terminal pairs produced by original conditional jump threading.
    pub(super) threaded_condition_origins: Vec<(Instruction, Instruction)>,
}

/// Exact original `POP_BLOCK` records and the successors receiving their erased slots.
#[derive(Debug)]
pub(super) struct OriginalPopBlockSpan {
    pub(super) entry: Label,
    pub(super) items: Vec<Item>,
}

/// A later typed handler's authored dispatch, before its late marker is excluded.
#[derive(Debug)]
pub(super) struct HandlerDispatchArray {
    pub(super) entry: Label,
    pub(super) items: Vec<Item>,
    pub(super) target: Label,
    pub(super) depth: u32,
}

/// One completed async manager entry and its original late `SETUP_WITH` record.
#[derive(Debug)]
pub(super) struct AsyncWithSetup {
    pub(super) setup: Label,
    pub(super) send: Instruction,
    pub(super) prefix: Vec<Item>,
    pub(super) cleanup: Vec<Item>,
    // Filled only where the cold pass converts this original fallthrough to a jump.
    pub(super) bridge: Option<(Label, Instruction)>,
}

/// An ordinary await's completion and the original cleanup fallthrough entering it.
#[derive(Debug)]
pub(super) struct OrdinaryAwaitEnd {
    pub(super) send: Instruction,
    pub(super) entry: Label,
    pub(super) end_send: Instruction,
    pub(super) cleanup: Vec<Item>,
    // Set only where the cold pass converts this exact original fallthrough.
    pub(super) bridge: Option<(Label, Instruction)>,
}

/// Original assignment fallthrough into a selected tuple-copy join, before static swaps.
#[derive(Clone, Copy, Debug)]
pub(super) struct TupleJoinLocationOrigin {
    pub(super) predecessor: Label,
    pub(super) store: Instruction,
    pub(super) jump: Instruction,
}

/// Two adjacent original NULL POP records followed by a copied-finally local None test.
#[derive(Debug)]
pub(super) struct CopiedReturnNonePrefix {
    pub(super) return_nop: Option<Instruction>,
    pub(super) pops: [Instruction; 2],
    pub(super) load: Instruction,
    pub(super) condition: Instruction,
}

/// A constant return's original POP pair and complete local truthiness test.
#[derive(Debug)]
pub(super) struct CopiedReturnTruthPrefix {
    pub(super) return_nop: Instruction,
    pub(super) pops: [Instruction; 2],
    pub(super) load: Instruction,
    pub(super) to_bool: Instruction,
    pub(super) condition: Instruction,
}

/// Exact awaited return and manager unwind, with two omitted POPs before its finally POP.
#[derive(Debug)]
pub(super) struct CopiedManagerReturnNonePrefix {
    pub(super) entry: Label,
    pub(super) prefix: [Instruction; 10],
    pub(super) pop: Instruction,
    pub(super) load: Instruction,
    pub(super) condition: Instruction,
}

/// Exact manager cleanup and first copied-finally test, before either compaction pass.
#[derive(Debug)]
pub(super) struct CopiedManagerReturnCompaction {
    pub(super) ordinary_try_pops: usize,
    pub(super) cleanup: [Instruction; 7],
    pub(super) earlier_cleanup: Option<[Instruction; 7]>,
    pub(super) pop: Instruction,
    pub(super) load: Instruction,
    pub(super) condition: Instruction,
}

/// Original constant-return cleanup, including NULL records omitted under a manager.
#[derive(Debug)]
pub(super) struct CopiedHandlerCleanup {
    pub(super) start: Label,
    pub(super) end: Label,
    pub(super) return_nop: Instruction,
    pub(super) instructions: Vec<Instruction>,
    pub(super) missing_null_pops: usize,
    pub(super) removed_prefix_slots: u16,
    // Exact inner-finally POP removed immediately before the captured local assignment.
    pub(super) assignment_pop: Option<(Label, Instruction)>,
    pub(super) continued_call: Option<([Instruction; 6], Instruction)>,
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
    pub(super) retained_setup: Option<(Label, Label, Label)>,
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
    comprehension_entry: Option<Label>,
    source_entry: Option<Label>,
}

impl ExceptionOwnerProvenance {
    /// Retires an async entry before erased labels can migrate to another live array.
    pub(super) fn retire_async_with_setup(&mut self, label: Label) {
        self.ordinary_await_ends.retain(|origin| {
            origin.entry != label
                && !origin.cleanup.contains(&Item::Label(label))
                && origin.bridge.is_none_or(|(bridge, _)| bridge != label)
        });
        self.async_with_setups.retain(|origin| {
            origin.setup != label
                && !origin.prefix.contains(&Item::Label(label))
                && !origin.cleanup.contains(&Item::Label(label))
                && origin.bridge.is_none_or(|(bridge, _)| bridge != label)
        });
    }

    /// Finds unchanged For target stores with exactly their original tracing NOP removed.
    fn original_for_target_prefix_positions(&self, items: &[Item]) -> FxHashMap<usize, usize> {
        self.for_target_owner_prefixes
            .iter()
            .filter_map(|(entry, store)| {
                if !self.source_boundaries.contains(entry)
                    || !self.block_boundaries.contains(entry)
                    || self.retired_normal_try_copy_labels.contains(entry)
                    || self.normal_try_copy_blocks.contains(entry)
                    || self.for_target_owner_prefixes.iter().filter(|(label, _)| label == entry).count() != 1
                {
                    return None;
                }
                let mut entries = items.iter().enumerate().filter_map(|(index, item)| {
                    (*item == Item::Label(*entry)).then_some(index)
                });
                let index = entries.next()?;
                if entries.next().is_some() {
                    return None;
                }
                let mut expected = *store;
                expected.local_policy.add_hidden_exception_owner_slots(1);
                if !matches!(items.get(index + 1), Some(Item::Instruction(actual)) if *actual == expected) {
                    return None;
                }
                let position = items[..index].iter().filter(|item| matches!(item, Item::Instruction(_))).count();
                Some((position, position))
            })
            .collect()
    }

    /// Finds a unique unchanged survivor whose literal-fold witness has not retired.
    fn original_uniform_fold_positions(&self, instructions: &[&Instruction]) -> FxHashSet<usize> {
        self.original_uniform_folds
            .iter()
            .filter_map(|original| {
                if !original.jump_policy.has_original_uniform_fold()
                    || self
                        .original_uniform_folds
                        .iter()
                        .filter(|other| *other == original)
                        .count()
                        != 1
                {
                    return None;
                }
                let mut matches = instructions
                    .iter()
                    .enumerate()
                    .filter(|(_, instruction)| ***instruction == *original);
                let (index, _) = matches.next()?;
                matches.next().is_none().then_some(index)
            })
            .collect()
    }

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
                {
                    let prefix = &items[begin..=end];
                    let certified = if self.uncompacted_prefix(prefix, items, 0, false) {
                        Some((prefix, None, None))
                    } else if let Some((entry, prefix, restore_count)) =
                        self.comprehension_normal_prefix(prefix, items)
                        && self.uncompacted_prefix(prefix, items, restore_count, false)
                    {
                        Some((prefix, Some(entry), None))
                    } else if let Some((entry, prefix)) = self.original_source_prefix(prefix)
                        && self.uncompacted_prefix(prefix, items, 0, true)
                    {
                        Some((prefix, None, Some(entry)))
                    } else {
                        None
                    };
                    if let Some((prefix, comprehension_entry, source_entry)) = certified {
                        self.uncompacted_conditional_arrays
                            .push(UncompactedConditionalArray {
                                prefix: prefix.to_vec(),
                                comprehension_entry,
                                source_entry,
                            });
                    }
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

    /// Selects the exact normal restore entry, excluding preceding cleanup aliases.
    fn comprehension_normal_prefix<'a>(
        &self,
        prefix: &'a [Item],
        all: &[Item],
    ) -> Option<(Label, &'a [Item], usize)> {
        use crate::target::opcodes::{CALL, STORE_FAST, SWAP};

        let leading = prefix
            .iter()
            .take_while(|item| matches!(item, Item::Label(_)))
            .copied()
            .collect::<Vec<_>>();
        for (entry, jump) in &self.original_comprehension_normal_exits {
            let Some(original) = self.original_comprehension_normal_restores.get(entry) else {
                continue;
            };
            let [swap, restores @ ..] = original.as_slice() else {
                continue;
            };
            if restores.is_empty()
                || swap.opcode != SWAP
                || !matches!(swap.operand(), Operand::Value(depth)
                    if usize::try_from(depth).is_ok_and(|depth| depth == original.len()))
                || restores.iter().any(|restore| {
                    restore.opcode != STORE_FAST || restore.location != swap.location
                })
            {
                continue;
            }
            let Some(start) = leading.iter().position(|item| *item == Item::Label(*entry)) else {
                continue;
            };
            if all
                .iter()
                .filter(|item| **item == Item::Label(*entry))
                .count()
                != 1
                || all
                    .iter()
                    .filter(|item| **item == Item::Instruction(*jump))
                    .count()
                    != 1
            {
                continue;
            }
            let incoming = all
                .iter()
                .filter_map(|item| {
                    if let Item::Instruction(instruction) = item
                        && let Operand::Forward(label) | Operand::Backward(label) =
                            instruction.operand()
                        && leading.contains(&Item::Label(label))
                    {
                        Some(*instruction)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if incoming.as_slice() != [*jump] {
                continue;
            }
            let instructions = prefix[start..]
                .iter()
                .filter_map(|item| {
                    if let Item::Instruction(instruction) = item {
                        Some(*instruction)
                    } else {
                        None
                    }
                })
                .take(original.len() + 1)
                .collect::<Vec<_>>();
            // All authored restores are STORE_FAST_MAYBE_NULL in CPython,
            // including cells. They remain distinct through superinstruction
            // insertion. CALL prevents static elimination of SWAP(n + 1),
            // because only n swappable restores precede it.
            if instructions.starts_with(original)
                && instructions
                    .get(original.len())
                    .is_some_and(|call| call.opcode == CALL)
            {
                return Some((*entry, &prefix[start..], restores.len()));
            }
        }
        None
    }

    /// Selects the original array after an already-folded source condition.
    fn original_source_prefix<'a>(&self, prefix: &'a [Item]) -> Option<(Label, &'a [Item])> {
        let boundary = prefix.iter().rposition(|item| {
            matches!(item, Item::Label(label)
                if self.block_boundaries.contains(label) && self.source_boundaries.contains(label))
        })?;
        let Item::Label(entry) = prefix[boundary] else {
            unreachable!()
        };
        // Keep every adjacent label so a second original array cannot disappear
        // behind a selected alias. Range-only labels may share the same head.
        let begin = prefix[..boundary]
            .iter()
            .rposition(|item| matches!(item, Item::Instruction(_)))
            .map_or(0, |index| index + 1);
        Some((entry, &prefix[begin..]))
    }

    fn uncompacted_prefix(
        &self,
        prefix: &[Item],
        all: &[Item],
        comprehension_restore_count: usize,
        source_head: bool,
    ) -> bool {
        use crate::target::opcodes::{
            BINARY_OP, BUILD_MAP, BUILD_STRING, CALL, CALL_KW, FORMAT_SIMPLE, IMPORT_FROM,
            IMPORT_NAME, LOAD_ATTR, LOAD_CONST, LOAD_DEREF, LOAD_FAST, LOAD_GLOBAL, LOAD_NAME,
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
        if source_head {
            // Graph jump targets alone include copied exits. Require the compiler's
            // original source boundary and exactly one original array at this head.
            let mut boundaries = leading_labels
                .iter()
                .filter(|label| self.block_boundaries.contains(*label));
            let Some(entry) = boundaries.next() else {
                return false;
            };
            if boundaries.next().is_some() || !self.source_boundaries.contains(entry) {
                return false;
            }
        } else if !has_original_target && comprehension_restore_count == 0 {
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
        let mut from_import_members = FxHashSet::default();
        for (item_index, item) in prefix.iter().enumerate() {
            match item {
                Item::Label(label) => {
                    if !labels.insert(*label)
                        || (!instructions.is_empty() && targets.contains(label))
                        || (source_head
                            && !instructions.is_empty()
                            && self.block_boundaries.contains(label))
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
                Item::Instruction(instruction) => {
                    // This complete single-name import retains all six original records.
                    // Certify its module-stack discard without admitting arbitrary POPs
                    // or import fragments separated by labels.
                    if instruction.opcode == IMPORT_FROM
                        && let Some(start) = item_index.checked_sub(3)
                        && let Some(
                            [
                                Item::Instruction(level),
                                Item::Instruction(names),
                                Item::Instruction(module),
                                Item::Instruction(imported),
                                Item::Instruction(store),
                                Item::Instruction(discard),
                            ],
                        ) = prefix.get(start..start + 6)
                        && level.opcode == LOAD_SMALL_INT
                        && names.opcode == LOAD_CONST
                        && module.opcode == IMPORT_NAME
                        && imported.opcode == IMPORT_FROM
                        && store.opcode == STORE_FAST
                        && discard.opcode == POP_TOP
                        && discard.operand() == Operand::Value(0)
                        && [level, names, module, imported, store, discard]
                            .iter()
                            .all(|part| {
                                part.location == level.location
                                    && matches!(part.operand(), Operand::Value(_))
                            })
                    {
                        from_import_members.insert(instructions.len());
                        from_import_members.insert(instructions.len() + 2);
                    }
                    instructions.push(*instruction);
                }
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
                LOAD_DEREF => {
                    // A cell load feeding the terminal truth conversion keeps its
                    // original record; local superinstructions do not fuse it.
                    if index + 2 != body.len() {
                        return false;
                    }
                }
                BUILD_MAP => {
                    // The certified restore is followed by CALL, whose runtime
                    // result is the last value of this nonempty dictionary. Both
                    // records survive without constant folding or compaction.
                    if comprehension_restore_count == 0
                        || index != comprehension_restore_count + 2
                        || !matches!(instruction.operand(), Operand::Value(1..))
                        || instructions
                            .get(index.wrapping_sub(1))
                            .is_none_or(|previous| previous.opcode != CALL)
                    {
                        return false;
                    }
                }
                STORE_FAST => {
                    // Certified comprehension restores are still MAYBE_NULL
                    // pseudo instructions during CP superinstruction insertion.
                    // Ordinary adjacent stores/loads can fuse on the same line.
                    if !(1..=comprehension_restore_count).contains(&index)
                        && instructions.get(index + 1).is_some_and(|next| {
                            matches!(next.opcode, LOAD_FAST | STORE_FAST)
                                && next.location.line == instruction.location.line
                        })
                    {
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
                    // Attribute and module-name calls retain this separate record;
                    // a direct global call instead folds it into LOAD_GLOBAL.
                    if instructions
                        .get(index.wrapping_sub(1))
                        .is_none_or(|previous| !matches!(previous.opcode, LOAD_ATTR | LOAD_NAME))
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
                    // Binary operations retain one original record unless constant
                    // folding consumes both preceding constant loads with it.
                    if !matches!(instruction.operand(), Operand::Value(0..=26))
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
                                !matches!(
                                    previous.opcode,
                                    LOAD_FAST | LOAD_DEREF | LOAD_NAME | LOAD_ATTR | CALL
                                )
                            })
                    {
                        return false;
                    }
                }
                CALL_KW => {
                    // The keyword-name tuple is loaded directly, not built and folded.
                    // The call retains its own record just like positional CALL.
                    if instructions
                        .get(index.wrapping_sub(1))
                        .is_none_or(|previous| previous.opcode != LOAD_CONST)
                    {
                        return false;
                    }
                }
                FORMAT_SIMPLE => {
                    // Runtime interpolation keeps both the value and format records.
                    // Constant and conversion paths need their own compaction proof.
                    if instruction.operand() != Operand::Value(0)
                        || instructions
                            .get(index.wrapping_sub(1))
                            .is_none_or(|previous| {
                                !matches!(
                                    previous.opcode,
                                    LOAD_FAST | LOAD_NAME | LOAD_ATTR | CALL | CALL_KW
                                )
                            })
                    {
                        return false;
                    }
                }
                BUILD_STRING => {
                    // This is the direct small-string producer, ending in a runtime
                    // interpolation. Large joined-string lowering is not covered.
                    if !matches!(instruction.operand(), Operand::Value(2..=30))
                        || instructions
                            .get(index.wrapping_sub(1))
                            .is_none_or(|previous| previous.opcode != FORMAT_SIMPLE)
                    {
                        return false;
                    }
                }
                IMPORT_FROM | POP_TOP if from_import_members.contains(&index) => {}
                POP_TOP => {
                    // Discarding a call result retains both original records at an
                    // authenticated source or branch head. Constant and COPY/POP
                    // pairs can compact and cannot certify a fresh slot.
                    if instructions
                        .get(index.wrapping_sub(1))
                        .is_none_or(|previous| !matches!(previous.opcode, CALL | CALL_KW))
                    {
                        return false;
                    }
                }
                LOAD_NAME | LOAD_ATTR | LOAD_CONST | STORE_NAME | CALL | IMPORT_NAME => {}
                crate::target::opcodes::SWAP if comprehension_restore_count > 0 && index == 0 => {}
                _ => return false,
            }
        }
        // Retained SETUPs need an authored NOP proof. A certified comprehension
        // restore followed by CALL already establishes the original array head,
        // including a condition reached without crossing a new SETUP.
        (setup_count > 0 || comprehension_restore_count > 0)
            && self
                .setups
                .iter()
                .all(|(label, location)| !labels.contains(label) || location.is_some())
    }
}

impl LayoutAssembler {
    /// Keeps only the unchanged `SEND` entry immediately before its authored async setup.
    fn original_async_with_setups(&self) -> FxHashSet<Label> {
        let mut result = FxHashSet::default();
        for origin in &self.exception_owner_provenance.async_with_setups {
            let Some(first) = origin.prefix.first() else {
                continue;
            };
            let mut entries = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| *item == first);
            let Some((start, _)) = entries.next() else {
                continue;
            };
            let end = start + origin.prefix.len();
            if entries.next().is_some()
                || self.items.get(start..end) != Some(origin.prefix.as_slice())
                || self.items.get(end) != Some(&Item::Label(origin.setup))
                || self
                    .items
                    .iter()
                    .filter(|item| **item == Item::Label(origin.setup))
                    .count()
                    != 1
                || self
                    .exception_owner_provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| *label == origin.setup)
                    .count()
                    != 1
            {
                continue;
            }
            let Some(first_cleanup) = origin.cleanup.first() else {
                continue;
            };
            let mut cleanups = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| *item == first_cleanup);
            let Some((cleanup_start, _)) = cleanups.next() else {
                continue;
            };
            let cleanup_end = cleanup_start + origin.cleanup.len();
            if cleanups.next().is_some()
                || self.items.get(cleanup_start..cleanup_end) != Some(origin.cleanup.as_slice())
            {
                continue;
            }
            if let Some((label, instruction)) = origin.bridge {
                // The cold pass's exact fresh block must still immediately follow
                // the original cleanup. Equal opcodes or locations are insufficient.
                if self.items.get(cleanup_end..cleanup_end + 2)
                    != Some([Item::Label(label), Item::Instruction(instruction)].as_slice())
                    || self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(label))
                        .count()
                        != 1
                {
                    continue;
                }
            } else if cleanup_end != start {
                continue;
            }
            // Only the original SEND and the authenticated conversion of the
            // original cleanup fallthrough may enter this array.
            let mut incoming = self.items.iter().filter_map(|item| match item {
                Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if origin.prefix.contains(&Item::Label(label)) || label == origin.setup) =>
                {
                    Some(instruction)
                }
                _ => None,
            });
            let mut seen_send = false;
            let mut seen_bridge = false;
            let exact = incoming.all(|instruction| {
                if *instruction == origin.send && !seen_send {
                    seen_send = true;
                    true
                } else if origin
                    .bridge
                    .is_some_and(|(_, bridge)| bridge == *instruction)
                    && !seen_bridge
                {
                    seen_bridge = true;
                    true
                } else {
                    false
                }
            });
            if exact && seen_send && seen_bridge == origin.bridge.is_some() {
                result.insert(origin.setup);
            }
        }
        result
    }

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
            if let Some(source_entry) = origin.source_entry
                && (!self
                    .exception_owner_provenance
                    .block_boundaries
                    .contains(&source_entry)
                    || !self
                        .exception_owner_provenance
                        .source_boundaries
                        .contains(&source_entry))
            {
                continue;
            }
            if let Some(original_entry) = origin.comprehension_entry
                && (original_entry != *entry
                    || self
                        .exception_owner_provenance
                        .retired_comprehension_normal_entries
                        .contains(&original_entry))
            {
                continue;
            }
            let leading_labels = origin
                .prefix
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
                .copied()
                .collect::<Vec<_>>();
            if origin.comprehension_entry.is_some()
                && origin.prefix.iter().any(|item| {
                    matches!(item, Item::Label(label)
                        if !leading_labels.contains(item) && jump_targets.contains(label))
                })
            {
                continue;
            }
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
    /// Replays captured `POP_BLOCK` spans, retaining exact labels and successor records.
    fn captured_pop_slots(
        &self,
        jump_targets: &FxHashSet<Label>,
        label_positions: &FxHashMap<Label, usize>,
    ) -> (FxHashMap<usize, usize>, FxHashSet<Label>) {
        let provenance = &self.exception_owner_provenance;
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
        let mut slots = FxHashMap::default();
        let mut aliases = FxHashSet::default();
        for (origin, pop_count) in provenance
            .named_handler_pop_pairs
            .iter()
            .map(|origin| (origin, 2))
            .chain(
                provenance
                    .copied_handler_finally_pops
                    .iter()
                    .map(|origin| (origin, 1)),
            )
        {
            let Some(&array) = label_positions.get(&origin.entry) else {
                continue;
            };
            if pop_count == 1 && !slots.values().any(|entry| *entry == array) {
                continue;
            }
            if copied(&origin.entry)
                || !provenance.block_boundaries.contains(&origin.entry)
                || self
                    .items
                    .iter()
                    .filter(|item| **item == Item::Label(origin.entry))
                    .count()
                    != 1
            {
                continue;
            }
            let labels = origin
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            if labels.is_empty()
                || labels.iter().any(|label| {
                    copied(label)
                        || jump_targets.contains(label)
                        || provenance.block_boundaries.contains(label)
                        || provenance.source_boundaries.contains(label)
                        || self.preserved_block_boundaries.contains(label)
                        || provenance.setups.iter().any(|(setup, _)| setup == label)
                        || self
                            .exception_regions
                            .iter()
                            .any(|region| region.target == *label)
                        || self
                            .items
                            .iter()
                            .filter(|item| **item == Item::Label(*label))
                            .count()
                            != 1
                })
            {
                continue;
            }
            let mut expected = Vec::new();
            let mut successors = Vec::new();
            let mut pending_pop = false;
            for item in &origin.items {
                match *item {
                    Item::Instruction(instruction)
                        if instruction.fusion_policy.is_converted_pop_block() =>
                    {
                        pending_pop = true;
                    }
                    Item::Instruction(mut instruction) => {
                        if pending_pop {
                            successors.push(expected.len());
                            instruction.local_policy.add_hidden_exception_owner_slots(1);
                            pending_pop = false;
                        }
                        expected.push(Item::Instruction(instruction));
                    }
                    label @ Item::Label(_) => expected.push(label),
                }
            }
            if pending_pop
                || successors.len() != pop_count
                || !matches!(expected.first(), Some(Item::Label(_)))
            {
                continue;
            }
            let Some(begin) = self
                .items
                .iter()
                .position(|item| Some(item) == expected.first())
            else {
                continue;
            };
            let Some(current) = self.items.get(begin..begin + expected.len()) else {
                continue;
            };
            if !expected.iter().zip(current).all(|(expected, current)| {
                let mut expected = *expected;
                if let (Item::Instruction(expected), Item::Instruction(current)) =
                    (&mut expected, current)
                    && expected.opcode == super::LOAD_FAST
                    && current.opcode == super::LOAD_FAST_BORROW
                {
                    expected.opcode = super::LOAD_FAST_BORROW;
                }
                expected == *current
            }) {
                continue;
            }
            let first = self.items[..begin]
                .iter()
                .filter(|item| matches!(item, Item::Instruction(_)))
                .count();
            if first < array {
                continue;
            }
            for successor in successors {
                let index = first
                    + expected[..successor]
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count();
                slots.insert(index, array);
            }
            aliases.extend(labels);
        }
        (slots, aliases)
    }

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
            let mut unchanged_prefix = true;
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
                unchanged_prefix &=
                    instruction.exception_policy() == super::ExceptionPolicy::default();
                tail.instruction(*instruction, previous);
                // Global-call and boolean-comparison folding also remove protected
                // records. Their complete prefix must retain its original owner policy.
                if (tail.early_removals != usize::from(none_tail) * 3 && !unchanged_prefix)
                    || tail.insertions != 0
                    || tail.late_removals != 0
                    || tail.post_compaction_fusions != 0
                {
                    break;
                }
                if is_conditional_jump(instruction.opcode) {
                    if matches!(instruction.operand(), Operand::Forward(_)) {
                        let owner = if tail.early_removals > 0 {
                            // All removed records are in this uniformly protected suffix.
                            // Compaction leaves its old tail available for the appended
                            // marker, beyond the retained NULL-owned POP_BLOCK.
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
            let retained_setup_nop = origin.retained_setup.and_then(|(start, end, setup)| {
                if !self
                    .exception_owner_provenance
                    .setup_nop_ranges
                    .contains(&(start, end))
                {
                    return None;
                }
                let start_at = origin
                    .prefix
                    .iter()
                    .position(|item| *item == Item::Label(start))?;
                let end_at = origin
                    .prefix
                    .iter()
                    .position(|item| *item == Item::Label(end))?;
                let setup_at = origin
                    .prefix
                    .iter()
                    .position(|item| *item == Item::Label(setup))?;
                if !(start_at < end_at && end_at <= setup_at) {
                    return None;
                }
                let mut instructions =
                    origin.prefix[start_at..end_at]
                        .iter()
                        .filter_map(|item| match item {
                            Item::Instruction(instruction) => Some(*instruction),
                            Item::Label(_) => None,
                        });
                let nop = instructions.next()?;
                if instructions.next().is_some()
                    || nop.opcode != NOP
                    || !self
                        .exception_owner_provenance
                        .setups
                        .contains(&(setup, Some(nop.location)))
                {
                    return None;
                }
                Some(nop)
            });
            if origin.retained_setup.is_some() && retained_setup_nop.is_none() {
                continue;
            }
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
            // Captured head aliases share the original array's first cell.
            let retained_setup_head = |label: Label| {
                retained_setup_nop.is_some()
                    && origin
                        .prefix
                        .iter()
                        .take_while(|item| matches!(item, Item::Label(_)))
                        .any(|item| *item == Item::Label(label))
            };
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
                        && !((authored_break_location.is_some() || retained_setup_nop.is_some())
                            && *label == origin.entry))
                        || (self.preserved_block_boundaries.contains(label)
                            && !retained_setup_head(*label))
                        || self
                            .exception_regions
                            .iter()
                            .any(|region| region.target == *label)
                        || self
                            .exception_owner_provenance
                            .setups
                            .iter()
                            .any(|(setup, _)| {
                                setup == label
                                    && !origin
                                        .retained_setup
                                        .is_some_and(|(_, _, retained)| *setup == retained)
                            })
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
                        || (self
                            .exception_owner_provenance
                            .setup_nop_boundaries
                            .contains(label)
                            && !origin
                                .retained_setup
                                .is_some_and(|(start, end, _)| *label == start || *label == end))
                        || (*label != origin.entry
                            && (self
                                .exception_owner_provenance
                                .source_boundaries
                                .contains(label)
                                || (self
                                    .exception_owner_provenance
                                    .block_boundaries
                                    .contains(label)
                                    && !retained_setup_head(*label))))
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
            let (current, end) =
                if authored_break_location.is_some() || retained_setup_nop.is_some() {
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
            // Normal-finally lowering can refine this excluded POP_BLOCK's owner
            // after capture without changing its original instruction cell.
            let excluded_pop_owner_refined = retained_setup_nop.is_some()
                && original_marker.exception_policy() == super::ExceptionPolicy::EXCLUDE_ALWAYS
                && marker.exception_policy() == super::ExceptionPolicy::AlwaysAbsent;
            if excluded_pop_owner_refined {
                original_marker.set_exception_owner(NormalizedExceptionOwner::Absent);
            }
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
            if retained_setup_nop.is_some() {
                for (expected, current) in expected.iter_mut().zip(&current) {
                    if let (Item::Instruction(expected), Item::Instruction(current)) =
                        (expected, current)
                        && expected.opcode == LOAD_FAST
                        && current.opcode == LOAD_FAST_BORROW
                    {
                        expected.opcode = LOAD_FAST_BORROW;
                    }
                }
            }
            if current != expected
                || marker.opcode != NOP
                || !marker.fusion_policy.is_converted_pop_block()
                || (marker.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
                    && !excluded_pop_owner_refined)
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
                    || (instruction.opcode == NOP && retained_setup_nop != Some(*instruction))
                    || matches!(instruction.opcode, NOT_TAKEN | FOR_ITER | SEND)
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
                ) || ((authored_break_location.is_some() || retained_setup_nop.is_some())
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
        let original_async_with_setups = self.original_async_with_setups();
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
        let original_uniform_folds = self
            .exception_owner_provenance
            .original_uniform_fold_positions(&indexed_instructions);
        let comprehension_restore_prefixes = self
            .exception_owner_provenance
            .comprehension_restore_owner_prefixes
            .iter()
            .filter_map(|(entry, restore)| {
                if self
                    .exception_owner_provenance
                    .retired_comprehension_normal_entries
                    .contains(entry)
                    || restore.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                    || restore.local_policy.hidden_exception_owner_slots() != 0
                    || restore.pre_fold_extra_instructions() != 0
                    || self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(*entry))
                        .count()
                        != 1
                {
                    return None;
                }
                let start = *label_positions.get(entry)?;
                let mut matches = indexed_instructions
                    .iter()
                    .enumerate()
                    .filter(|(_, instruction)| ***instruction == *restore);
                let (index, _) = matches.next()?;
                (index >= start && matches.next().is_none()).then_some((index, start))
            })
            .collect::<FxHashMap<_, _>>();
        let for_target_prefixes = self
            .exception_owner_provenance
            .original_for_target_prefix_positions(&self.items);
        let retained_setup_nops = self.retained_setup_nops(&label_positions);
        // An original entry prefix with no SETUP, region, or policy owner can
        // supply only NULL records, including its certified early-folded slots.
        let null_original_entry_prefix =
            |start: usize,
             setup_label: Label,
             owners: &FxHashMap<usize, ExceptionOwnerDecision>| {
                start
                    .checked_sub(1)
                    .is_some_and(|index| retained_setup_nops.contains(&index))
                    && self
                        .exception_owner_provenance
                        .setups
                        .iter()
                        .all(|(label, _)| {
                            *label == setup_label
                                || label_positions
                                    .get(label)
                                    .is_some_and(|position| *position > start)
                        })
                    && self.exception_regions.iter().all(|region| {
                        match (
                            label_positions.get(&region.start),
                            label_positions.get(&region.end),
                        ) {
                            (Some(begin), Some(end)) => *begin >= start || *end == 0,
                            _ => false,
                        }
                    })
                    && indexed_instructions[..start]
                        .iter()
                        .enumerate()
                        .all(|(index, previous)| {
                            matches!(
                                previous.exception_policy(),
                                super::ExceptionPolicy::NoneUnchanged
                                    | super::ExceptionPolicy::AlwaysUnchanged
                            ) && !owners.contains_key(&index)
                                && previous.no_location_policy.borrow_fallthrough()
                                    != super::BorrowFallthrough::Barrier
                                && (previous.opcode != NOP
                                    || (retained_setup_nops.contains(&index)
                                        && previous.local_policy.hidden_exception_owner_slots()
                                            == 0
                                        && previous.pre_fold_extra_instructions() == 0))
                        })
            };
        let retained_setup_null_markers =
            self.retained_setup_null_markers(&label_positions, &jump_targets, &retained_setup_nops);
        let retained_normal_pop_terminators =
            self.retained_normal_pop_terminators(&jump_targets, &label_positions);
        let (captured_pop_slots, captured_pop_aliases) =
            self.captured_pop_slots(&jump_targets, &label_positions);
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
        let mut retained_owned_suffix: Option<(usize, ExceptionOwnerTail)> = None;
        let mut ended_owned_prefix = None;
        let mut source_owned_prefix = None;
        let mut pending_retained_prefix = None;
        let mut transferred_pop_setup: Option<(Label, usize)> = None;
        let mut retained_pop_prefix = None;
        let mut early_return_nop_slot = None;
        let mut early_return_truth_slot = None;
        let mut owner_history_exact = true;
        let mut pop_slot_epoch_known = true;
        let mut retained_epoch_known = true;
        let mut uniform_with_suffix = None;
        let mut late_setup_positions = Vec::new();
        let mut late_setup_history_known = true;
        let mut value_jump_setup_history_known = true;
        let mut ended_uniform_with_suffix = false;
        let mut ended_owned_suffix = None;
        let mut ended_pop_slot = None;
        let mut captured_null_slots = Vec::new();
        let mut captured_handler_owner_slots = Vec::new();
        let mut pending_null_append = None;
        let mut pending_fresh_null_append = None;
        let mut pending_await_null_append = None;
        let mut pending_normal_pop_null_append = None;
        let mut original_array_start = 0;
        let mut pending_compacted_append = None;
        let mut pending_normal_none_owner = None;
        let mut copied_handler_prefix = None;
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
                    {
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                    }
                    for original in &self.exception_owner_provenance.copied_handler_cleanups {
                        if original.end != *label || !owner_history_exact {
                            continue;
                        }
                        let Some(&start) = label_positions.get(&original.start) else {
                            continue;
                        };
                        if original_array_start > start
                            || start >= instruction_index
                            || instruction_index - start != original.instructions.len()
                            || tail.insertions != 0
                            || tail.late_removals != 0
                            || tail.post_compaction_fusions != 0
                            || original.return_nop.opcode != NOP
                            || original.return_nop.location != original.instructions[0].location
                        {
                            continue;
                        }
                        if let Some((boundary, _)) = original.assignment_pop
                            && (original_array_start + 1 != start
                                || indexed_instructions[original_array_start].opcode != POP_TOP
                                || indexed_instructions[original_array_start]
                                    .local_policy
                                    .hidden_exception_owner_slots()
                                    != 0
                                || tail.early_removals != 3
                                || label_positions.get(&boundary) != Some(&(start + 1))
                                || !self
                                    .exception_owner_provenance
                                    .removed_pop_block_boundaries
                                    .contains(&boundary)
                                || self
                                    .exception_owner_provenance
                                    .removed_pop_block_widths
                                    .get(&boundary)
                                    .copied()
                                    .unwrap_or(1)
                                    != 1
                                || jump_targets.iter().any(|label| {
                                    label_positions.get(label).is_some_and(|position| {
                                        start < *position && *position < instruction_index
                                    })
                                })
                                || self
                                    .uniform_exception_owner(
                                        start + 1,
                                        instruction_index,
                                        &label_positions,
                                    )
                                    .is_none_or(|region| {
                                        self.exception_regions[region].is_scope_fallback
                                    }))
                        {
                            continue;
                        }
                        // The return NOP and any emitted NULL POPs compact into
                        // POP_EXCEPT. Omitted POPs are counted separately during replay.
                        let hidden = original.removed_prefix_slots;
                        let exact =
                            original
                                .instructions
                                .iter()
                                .enumerate()
                                .all(|(offset, expected)| {
                                    let mut expected = *expected;
                                    if offset == 0 {
                                        expected
                                            .local_policy
                                            .add_hidden_exception_owner_slots(hidden);
                                    } else if offset == 1 && original.assignment_pop.is_some() {
                                        // The captured inner POP compacts into LOAD_CONST.
                                        // Its NULL slot remains distinct from both owned
                                        // assignment records when replay selects the old slot.
                                        expected.local_policy.add_hidden_exception_owner_slots(1);
                                    }
                                    *indexed_instructions[start + offset] == expected
                                });
                        if exact {
                            copied_handler_prefix = Some((
                                original_array_start,
                                start,
                                instruction_index,
                                tail.early_removals,
                                original.missing_null_pops,
                                original.continued_call,
                            ));
                        }
                    }
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
                    if transferred_pop_setup.is_some_and(|(setup, _)| {
                        boundaries.contains(label)
                            || jump_targets.contains(label)
                            || region_ends.contains(label)
                            || self
                                .exception_owner_provenance
                                .source_boundaries
                                .contains(label)
                            || self
                                .exception_owner_provenance
                                .removed_pop_block_boundaries
                                .contains(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_counts
                                .contains_key(label)
                            || self
                                .exception_owner_provenance
                                .omitted_nop_locations
                                .contains_key(label)
                            || (*label != setup
                                && !self
                                    .exception_owner_provenance
                                    .setup_nop_boundaries
                                    .contains(label))
                    }) {
                        transferred_pop_setup = None;
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
                    // Only range endpoints and recorded bookkeeping continuations may
                    // intervene between this authenticated await condition and its marker.
                    // An authored entry or an unknown label ends the original-array proof.
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
                        || !(region_ends.contains(label)
                            || self
                                .exception_owner_provenance
                                .continuation_boundaries
                                .contains(label))
                    {
                        pending_await_null_append = None;
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
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                        tail = ExceptionOwnerTail::default();
                        late_setup_positions.clear();
                        late_setup_history_known = true;
                        value_jump_setup_history_known = true;
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
                        if self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                            && (retained_epoch_known
                                // Named-handler cleanup labels can end the retained-SETUP
                                // epoch without changing this original array. An exact,
                                // uniformly owned prefix still locates its final NULL POP.
                                || (owner_history_exact
                                    && pop_slot_epoch_known
                                    && tail.insertions == 0
                                    && tail.late_removals == 0
                                    && tail.post_compaction_fusions == 0
                                    && ended_owned_prefix.is_some_and(
                                        |(start, end, start_tail, end_tail)| {
                                            original_array_start <= start
                                                && start < end
                                                && end == instruction_index
                                                && start_tail.early_removals
                                                    == end_tail.early_removals
                                                && self.has_uniform_exception_owner(
                                                    start,
                                                    end,
                                                    &label_positions,
                                                )
                                        },
                                    )))
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
                        // Cold-block reordering can name an exclusion endpoint with the
                        // original head label instead of its adjacent SETUP-range label.
                        // At an unchanged fresh head, that alias contributes no POP record.
                        let retained_setup_head_alias = !has_instruction
                            && instruction_index == original_array_start
                            && owner_history_exact
                            && tail.early_removals == 0
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && self
                                .exception_owner_provenance
                                .block_boundaries
                                .contains(label)
                            && retained_setup_nops.contains(&instruction_index)
                            && self
                                .exception_owner_provenance
                                .setups
                                .iter()
                                .filter(|(setup, _)| {
                                    label_positions.get(setup) == Some(&(instruction_index + 1))
                                })
                                .count()
                                == 1
                            && !self
                                .exception_owner_provenance
                                .removed_pop_block_boundaries
                                .iter()
                                .chain(self.exception_owner_provenance.omitted_nop_counts.keys())
                                .chain(self.exception_owner_provenance.omitted_nop_locations.keys())
                                .any(|origin| {
                                    label_positions.get(origin) == Some(&instruction_index)
                                });
                        if !self
                            .exception_owner_provenance
                            .removed_pop_block_boundaries
                            .contains(label)
                            && !retained_setup_head_alias
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
                        // A value-jump range start adds no original record or join.
                        // Keep the older position-sensitive history conservative.
                        value_jump_setup_history_known &= !jump_targets.contains(label)
                            && !self
                                .exception_owner_provenance
                                .removed_pop_block_boundaries
                                .contains(label)
                            && self
                                .exception_owner_provenance
                                .value_jump_continuation_boundaries
                                .contains(label);
                    }
                    if has_instruction
                        && !captured_pop_aliases.contains(label)
                        && !continues_original_block
                        && !self
                            .exception_owner_provenance
                            .protected_prefix_boundaries
                            .contains(label)
                        && ended_owned_suffix != Some(instruction_index)
                        && !previous.is_some_and(|instruction: Instruction| {
                            is_conditional_jump(instruction.opcode)
                        })
                    {
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
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
                            let known_setup = self
                                .exception_owner_provenance
                                .synchronous_with_setups
                                .contains(label)
                                || original_async_with_setups.contains(label)
                                || self.exception_regions.iter().any(|region| {
                                    region.is_scope_fallback && region.start == *label
                                });
                            late_setup_history_known &= known_setup;
                            value_jump_setup_history_known &= known_setup;
                        }
                        tail.late_removals += usize::from(!retained);
                        // A certified async SEND continuation has the same late-SETUP
                        // suffix. Its captured entry, cleanup, and incoming edges must all
                        // remain unchanged before this array can donate the marker's owner.
                        uniform_with_suffix = (!retained
                            && (self
                                .exception_owner_provenance
                                .synchronous_with_setups
                                .contains(label)
                                || original_async_with_setups.contains(label)))
                        .then_some(instruction_index);
                        ended_uniform_with_suffix = false;
                        protected_suffix = (retained && unprotected_setups.contains(label))
                            .then_some((instruction_index, tail, *label));
                        // A copied return can transfer its outer POP into this authored
                        // SETUP NOP. That exact pair starts a new protected suffix without
                        // restoring the broader retained-SETUP or POP-slot epoch.
                        let transferred_pop =
                            transferred_pop_setup.take() == Some((*label, original_array_start));
                        retained_owned_suffix = (retained
                            && (retained_epoch_known || transferred_pop))
                            .then_some((instruction_index, tail));
                    }
                }
                Item::Instruction(instruction) => {
                    // Capture the single original POP before the NOP consumes its
                    // transferred slot. Only the immediately adjacent authored SETUP
                    // may use this proof; another instruction or boundary retires it.
                    transferred_pop_setup = None;
                    if instruction.opcode == NOP
                        && !instruction.fusion_policy.is_converted_pop_block()
                        && instruction.exception_policy() == super::ExceptionPolicy::NoneUnchanged
                        && instruction.local_policy.hidden_exception_owner_slots() == 1
                        && instruction.pre_fold_extra_instructions() == 0
                        && instruction.no_location_policy.borrow_fallthrough()
                            != super::BorrowFallthrough::Barrier
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && ended_pop_slot.is_some_and(|(index, pop_tail, width)| {
                            index == instruction_index
                                && width == 1
                                && pop_tail.early_removals == tail.early_removals
                        })
                        && ended_owned_prefix.is_some_and(|(start, end, _, _)| {
                            original_array_start <= start
                                && start < end
                                && end == instruction_index
                                && self.has_uniform_exception_owner(start, end, &label_positions)
                        })
                        && !self
                            .exception_owner_provenance
                            .omitted_nop_counts
                            .keys()
                            .chain(self.exception_owner_provenance.omitted_nop_locations.keys())
                            .any(|origin| {
                                label_positions.get(origin).is_some_and(|position| {
                                    *position == instruction_index
                                        || *position == instruction_index + 1
                                })
                            })
                        && !self
                            .exception_owner_provenance
                            .source_boundaries
                            .iter()
                            .chain(jump_targets.iter())
                            .any(|origin| label_positions.get(origin) == Some(&instruction_index))
                        && self
                            .exception_owner_provenance
                            .setup_nop_ranges
                            .iter()
                            .filter(|(start, end)| {
                                label_positions.get(start) == Some(&instruction_index)
                                    && label_positions.get(end) == Some(&(instruction_index + 1))
                            })
                            .count()
                            == 1
                    {
                        let mut matching_setups = self
                            .exception_owner_provenance
                            .setups
                            .iter()
                            .filter(|(label, location)| {
                                *location == Some(instruction.location)
                                    && label_positions.get(label) == Some(&(instruction_index + 1))
                            });
                        if let Some((setup, _)) = matching_setups.next()
                            && matching_setups.next().is_none()
                        {
                            transferred_pop_setup = Some((*setup, original_array_start));
                        }
                    }
                    if instruction.opcode == NOP {
                        early_return_nop_slot = None;
                        early_return_truth_slot = None;
                        if owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && matches!(
                                instruction.local_policy.hidden_exception_owner_slots(),
                                1 | 2
                            )
                            && instruction.pre_fold_extra_instructions() == 0
                            && instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Barrier
                            && original_array_start < instruction_index
                            && let Some(region) = self.uniform_exception_owner(
                                instruction_index - 1,
                                instruction_index,
                                &label_positions,
                            )
                            && !self.exception_regions[region].is_scope_fallback
                            && !self.exception_regions[region].preserve_lasti
                        {
                            let matching_pairs = if instruction
                                .local_policy
                                .hidden_exception_owner_slots()
                                == 1
                            {
                                self.exception_owner_provenance
                                    .early_return_nop_pairs
                                    .iter()
                                    .filter(|(_, pop, target)| {
                                        let mut expected = *pop;
                                        expected.local_policy.add_hidden_exception_owner_slots(1);
                                        expected == *instruction
                                            && self.exception_regions[region].target == *target
                                    })
                                    .count()
                            } else {
                                // The authored return NOP and ordinary-try POP compact
                                // into the finally POP. Their old cells remain owned and
                                // NULL respectively, before the retained POP's NULL cell.
                                self.exception_owner_provenance
                                    .copied_return_none_prefixes
                                    .iter()
                                    .filter(|origin| {
                                        let mut expected_pop = origin.pops[1];
                                        expected_pop
                                            .local_policy
                                            .add_hidden_exception_owner_slots(2);
                                        let Some(load) =
                                            indexed_instructions.get(instruction_index + 1)
                                        else {
                                            return false;
                                        };
                                        let mut expected_load = origin.load;
                                        if load.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                                            expected_load.opcode = load.opcode;
                                        }
                                        origin.return_nop.is_some()
                                            && expected_pop == *instruction
                                            && expected_load == **load
                                            && indexed_instructions
                                                .get(instruction_index + 2)
                                                .is_some_and(|condition| {
                                                    **condition == origin.condition
                                                })
                                            && jump_targets.iter().all(|target| {
                                                label_positions.get(target).is_none_or(|position| {
                                                    *position <= instruction_index
                                                        || *position > instruction_index + 3
                                                })
                                            })
                                    })
                                    .count()
                            };
                            if matching_pairs == 1
                                && let Some(slot) =
                                    instruction_index.checked_add(tail.early_removals)
                            {
                                early_return_nop_slot =
                                    Some((original_array_start, instruction_index, slot, region));
                            }
                            // Truthiness retains TO_BOOL as a separate original cell.
                            // Replay the complete captured prefix before locating its two
                            // NULL POP cells in the uncompacted instruction array.
                            if matching_pairs == 0
                                && instruction.local_policy.hidden_exception_owner_slots() == 2
                                && self
                                    .exception_owner_provenance
                                    .copied_return_truth_prefixes
                                    .iter()
                                    .filter(|origin| {
                                        let mut pop = origin.pops[1];
                                        pop.local_policy.add_hidden_exception_owner_slots(2);
                                        let Some(load) =
                                            indexed_instructions.get(instruction_index + 1)
                                        else {
                                            return false;
                                        };
                                        let mut expected_load = origin.load;
                                        if load.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                                            expected_load.opcode = load.opcode;
                                        }
                                        pop == *instruction
                                            && origin.return_nop.location == instruction.location
                                            && origin.return_nop.depth_after()
                                                == instruction.depth_after()
                                            && expected_load == **load
                                            && indexed_instructions
                                                .get(instruction_index + 2)
                                                .is_some_and(|to_bool| **to_bool == origin.to_bool)
                                            && indexed_instructions
                                                .get(instruction_index + 3)
                                                .is_some_and(|condition| {
                                                    **condition == origin.condition
                                                })
                                            && jump_targets.iter().all(|target| {
                                                label_positions.get(target).is_none_or(|position| {
                                                    *position <= instruction_index
                                                        || *position > instruction_index + 4
                                                })
                                            })
                                    })
                                    .count()
                                    == 1
                                && let Some(slot) =
                                    instruction_index.checked_add(tail.early_removals)
                            {
                                early_return_truth_slot =
                                    Some((original_array_start, instruction_index, slot));
                            }
                        }
                    }
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
                        // A retained SETUP NOP remains a live record during the final
                        // SETUP compaction. Unlike an unknown folded NOP, it preserves
                        // this array's exact late-removal positions.
                        late_setup_history_known &= retained_setup_nops
                            .contains(&instruction_index)
                            && instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Barrier;
                        value_jump_setup_history_known = false;
                        has_uninterrupted_fusion = false;
                        // An exact retained SETUP NOP is a known live array record.
                        // It preserves an existing POP_BLOCK epoch without reviving
                        // one lost at an unknown source join or borrow barrier.
                        pop_slot_epoch_known &= retained_setup_nops.contains(&instruction_index)
                            && instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Barrier;
                        ended_pop_slot = None;
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                        pending_null_append = None;
                        pending_fresh_null_append = None;
                        pending_await_null_append = None;
                        pending_normal_pop_null_append = None;
                    }
                    let appends_to_null_slot =
                        pending_null_append.take() == Some(instruction_index);
                    let appends_to_await_null_slot =
                        pending_await_null_append.take() == Some(instruction_index);
                    let appends_to_fresh_null_slot = pending_fresh_null_append.take()
                        == Some(instruction_index)
                        || appends_to_await_null_slot;
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
                    if captured_pop_slots.get(&instruction_index) == Some(&original_array_start)
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                    {
                        captured_null_slots.push((instruction_index, tail.early_removals));
                    }
                    let previous_fusions = tail.post_compaction_fusions;
                    tail.instruction(*instruction, previous);
                    // The exact named-handler POP span also certifies its surviving
                    // SWAP and POP_EXCEPT cells. After counting the preceding erased
                    // POP, this is the survivor's old slot, with its own structural owner.
                    if captured_pop_slots.get(&instruction_index) == Some(&original_array_start)
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && matches!(
                            instruction.opcode,
                            crate::target::opcodes::SWAP | crate::target::opcodes::POP_EXCEPT
                        )
                        && instruction.local_policy.hidden_exception_owner_slots() == 1
                        && instruction.pre_fold_extra_instructions() == 0
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Unchanged
                        && !owners.contains_key(&instruction_index)
                        && let Some(region) = self.uniform_exception_owner(
                            instruction_index,
                            instruction_index + 1,
                            &label_positions,
                        )
                        && !self.exception_regions[region].is_scope_fallback
                        && let Some(slot) = instruction_index.checked_add(tail.early_removals)
                    {
                        captured_handler_owner_slots.push((original_array_start, slot, region));
                    }
                    if tail.post_compaction_fusions > previous_fusions
                        && instruction.no_location_policy.borrow_fallthrough()
                            != super::BorrowFallthrough::Barrier
                    {
                        has_uninterrupted_fusion = true;
                    }
                    let uniform_fold = original_uniform_folds.contains(&instruction_index)
                        || (instruction.local_policy.hidden_exception_owner_slots() > 0
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
                                }));
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
                    // The exact For target store carries its removed tracing NOP. Start
                    // after that record; later clean cells can donate an owner without
                    // assuming the hidden slot itself has the same exception handler.
                    if owner_history_exact
                        && source_owned_prefix.is_none()
                        && for_target_prefixes.get(&instruction_index)
                            == Some(&original_array_start)
                    {
                        source_owned_prefix = Some((instruction_index + 1, tail));
                    }
                    // A completed normal restore supplies a protected suffix within its
                    // original array. Static SWAP elimination may move this exact record;
                    // start after it, beyond any hidden slot transferred to an earlier store.
                    if owner_history_exact
                        && source_owned_prefix.is_none()
                        && comprehension_restore_prefixes.get(&instruction_index)
                            == Some(&original_array_start)
                    {
                        source_owned_prefix = Some((instruction_index + 1, tail));
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
                            // At exactly one record before the protected suffix, reuse
                            // lands on this authored SETUP's retained, unconditionally NULL NOP.
                            let retained_setup_cell = tail.insertions == 0
                                && protected_original_instructions.checked_add(1)
                                    == Some(stale_slots)
                                && start.checked_sub(1).is_some_and(|index| {
                                    original_array_start <= index
                                        && retained_setup_nops.contains(&index)
                                        && indexed_instructions.get(index).is_some_and(|nop| {
                                            nop.opcode == NOP
                                                && nop.exception_policy().exclusion()
                                                    == super::ExceptionExclusion::Always
                                                && nop.local_policy.hidden_exception_owner_slots()
                                                    == 0
                                                && nop.pre_fold_extra_instructions() == 0
                                        })
                                });
                            // Prefix insertion can consume the later fusion tail before
                            // this marker is appended. Keep the policy's COPY/TO_BOOL
                            // insertion adjustment in both original-width calculations.
                            let prologue_consumed_fusion_prefix = instruction.opcode == NOT_TAKEN
                                && original_array_start == 0
                                && tail.insertions > 0
                                && tail.insertions <= start
                                && tail.insertions == setup_tail.insertions
                                && tail.post_compaction_fusions <= tail.insertions
                                && insertions <= instruction_index - start
                                && null_original_entry_prefix(start, setup_label, &owners);
                            proven_unprotected_slot = owner_history_exact
                                && (tail.post_compaction_fusions == 0
                                    || prologue_consumed_fusion_prefix)
                                && stale_slots > protected_original_instructions
                                && (retained_setup_cell || {
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
                                });
                        }
                        // Entry prologue insertion consumes the exact early-compaction
                        // tail before jump normalization appends this marker. With no
                        // later vacancies, the next record has a fresh NULL owner.
                        let prologue_consumed_all_slots = instruction.opcode == NOT_TAKEN
                            && instruction.exception_policy().owner()
                                == NormalizedExceptionOwner::DynamicCondition
                            && original_array_start == 0
                            && owner_history_exact
                            && tail.insertions > 0
                            && tail.early_removals <= tail.insertions
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && !has_owner
                            && owners
                                .get(&instruction_index)
                                .is_none_or(|owner| !owner.definitive);
                        // Presence is already proved for a value-preserving Boolean
                        // marker. Retained array provenance supplies its exact source owner.
                        let source_region = (has_owner
                            && instruction.exception_policy().owner()
                                == NormalizedExceptionOwner::Dynamic
                            && instruction_index > 0
                            && owners.get(&instruction_index).is_none_or(
                                |owner: &ExceptionOwnerDecision| owner.source_region.is_none(),
                            ))
                        .then(|| {
                            self.exception_owner_provenance
                                .fused_manager_owners
                                .iter()
                                .find_map(|(condition, marker, region)| {
                                    (!region.is_scope_fallback
                                        && instruction == marker
                                        && indexed_instructions[instruction_index - 1] == condition)
                                        .then(|| {
                                            self.exception_regions.iter().position(|candidate| {
                                                super::fusion_owner_slots::same_manager_region(
                                                    candidate, region,
                                                )
                                            })
                                        })
                                        .flatten()
                                })
                        })
                        .flatten();
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner,
                                source_region,
                                override_existing: false,
                                definitive: proven_unprotected_slot
                                    || prologue_consumed_all_slots
                                    || instruction.exception_policy().owner()
                                        != NormalizedExceptionOwner::DynamicCondition,
                            },
                        );
                    }
                    // Local fusion leaves a newer stale suffix inside the original array.
                    // Entry prologue insertion consumes that suffix first; once all fusion
                    // slots are used, the marker reuses the older peephole-compaction tail.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Present
                        && tail.late_removals == 0
                        && let Some((start, setup_tail, setup_label)) = protected_suffix
                    {
                        let (removed, protected_before_compaction) =
                            if tail.post_compaction_fusions > tail.insertions {
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
                        // Entry prologue insertion can move the append onto the retained
                        // NULL SETUP immediately before this protected suffix. Certify
                        // that specific original cell before considering the wider prefix.
                        let prologue_reuses_original_tail = owner_history_exact
                            && original_array_start == 0
                            && tail.insertions > 0
                            && tail.insertions == setup_tail.insertions
                            && tail.post_compaction_fusions <= tail.insertions;
                        let prologue_reuses_null_setup = prologue_reuses_original_tail
                            && tail.early_removals.checked_sub(tail.insertions)
                                == protected_before_compaction.checked_add(1)
                            && start.checked_sub(1).is_some_and(|index| {
                                retained_setup_nops.contains(&index)
                                    && indexed_instructions.get(index).is_some_and(|nop| {
                                        nop.opcode == NOP
                                            && (nop.exception_policy().exclusion()
                                                == super::ExceptionExclusion::Always
                                                || (nop.exception_policy()
                                                    == super::ExceptionPolicy::NoneUnchanged
                                                    && !owners.contains_key(&index)
                                                    && self.exception_regions.iter().all(
                                                        |region| match (
                                                            label_positions.get(&region.start),
                                                            label_positions.get(&region.end),
                                                        ) {
                                                            (Some(begin), Some(end)) => {
                                                                index < *begin || *end <= index
                                                            }
                                                            _ => false,
                                                        },
                                                    )))
                                            && nop.local_policy.hidden_exception_owner_slots() == 0
                                            && nop.pre_fold_extra_instructions() == 0
                                            && nop.no_location_policy.borrow_fallthrough()
                                                != super::BorrowFallthrough::Barrier
                                    })
                            });
                        // An earlier donor is also NULL when the entire original entry
                        // prefix was unprotected. Reject erased SETUP origins as well as
                        // surviving owners before relying on the compacted prefix.
                        let prologue_reuses_null_prefix = prologue_reuses_original_tail
                            && tail.insertions <= start
                            && tail
                                .early_removals
                                .checked_sub(tail.insertions)
                                .is_some_and(|remaining| remaining > protected_before_compaction)
                            && null_original_entry_prefix(start, setup_label, &owners);
                        if prologue_consumed_all_slots
                            || prologue_reuses_null_setup
                            || prologue_reuses_null_prefix
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
                    // A retained SETUP fixes this original array's left edge. Local
                    // fusion leaves its old suffix in place even when an intervening
                    // POP_BLOCK changes the owner. The last F surviving records span at
                    // least F old records, so a uniform clean suffix of that width proves
                    // the owner of a marker appended after F fusion removals.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Absent
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && owner_history_exact
                        && !owners.contains_key(&instruction_index)
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions > 0
                        && let Some((start, _, _)) = protected_suffix
                        && original_array_start < start
                        && start <= instruction_index
                        && tail.post_compaction_fusions <= instruction_index - start
                    {
                        let suffix_start = instruction_index - tail.post_compaction_fusions;
                        if let Some(region) = self.uniform_exception_owner(
                            suffix_start,
                            instruction_index,
                            &label_positions,
                        ) && !self.exception_regions[region].is_scope_fallback
                            && indexed_instructions[suffix_start..instruction_index]
                                .iter()
                                .enumerate()
                                .all(|(offset, previous)| {
                                    previous.exception_policy().exclusion()
                                        == super::ExceptionExclusion::None
                                        && previous.exception_policy().owner()
                                            == NormalizedExceptionOwner::Unchanged
                                        && previous.local_policy.hidden_exception_owner_slots() == 0
                                        && previous.pre_fold_extra_instructions() == 0
                                        && !matches!(
                                            previous.opcode,
                                            LOAD_FAST_BORROW_LOAD_FAST_BORROW
                                                | LOAD_FAST_LOAD_FAST
                                                | STORE_FAST_LOAD_FAST
                                                | STORE_FAST_STORE_FAST
                                        )
                                        && !owners.contains_key(&(suffix_start + offset))
                                })
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
                    // Late SETUP compaction can leave an ordinary-try record behind
                    // a manager return. Its owner survives in the appended marker slot.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                        && original_array_start == 0
                        && owner_history_exact
                        && tail.insertions == 2
                        && tail.late_removals == 3
                        && tail.post_compaction_fusions == 0
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|owner| !owner.definitive)
                        && let Some(region) = self.entry_manager_return_setup_owner(
                            instruction_index,
                            &indexed_instructions,
                            &label_positions,
                            &jump_targets,
                            &late_setup_positions,
                        )
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
                    // The original manager POP precedes its cleanup. A later local-load
                    // fusion compacts the live array again, superseding those early slots.
                    if instruction.opcode == NOT_TAKEN
                        && matches!(
                            instruction.exception_policy(),
                            super::ExceptionPolicy::NonePresent
                                | super::ExceptionPolicy::NoneAbsent
                        )
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|owner| !owner.definitive)
                        && let Some(decision) = self.copied_manager_return_compaction_owner(
                            instruction_index,
                            original_array_start,
                            tail,
                            &indexed_instructions,
                            &label_positions,
                            &jump_targets,
                        )
                    {
                        owners.insert(instruction_index, decision);
                    }
                    // Resolve the exact normal-finally None prefix before a marker's
                    // range-end label can erase the preceding try's array certificate.
                    let original_none_marker =
                        if matches!(instruction.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                            && owner_history_exact
                            && retained_epoch_known
                            && pop_slot_epoch_known
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && instruction_index > 0
                            && ended_owned_prefix.is_some_and(|(start, end, _, _)| {
                                start >= original_array_start && end == instruction_index - 1
                            })
                            && ended_pop_slot.is_some_and(|(start, _, width)| {
                                start == instruction_index - 1 && width == 1
                            })
                            && self
                                .exception_owner_provenance
                                .normal_finally_none_prefixes
                                .iter()
                                .any(|(_, original_load, original_condition)| {
                                    let load = *indexed_instructions[instruction_index - 1];
                                    let mut expected_load = *original_load;
                                    expected_load
                                        .local_policy
                                        .add_hidden_exception_owner_slots(1);
                                    if load.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                                        expected_load.opcode = load.opcode;
                                    }
                                    instruction == original_condition && load == expected_load
                                })
                        {
                            indexed_instructions
                                .get(instruction_index + 1)
                                .filter(|marker| marker.opcode == NOT_TAKEN)
                                .map(|marker| (instruction_index + 1, **marker))
                        } else {
                            None
                        };
                    // A normal cleanup can share its original array with the preceding try.
                    // If compaction skips the entire cleanup suffix, the marker reuses a slot
                    // in that earlier protected prefix, rather than its current condition.
                    if let Some((marker_index, marker)) = (instruction.opcode == NOT_TAKEN)
                        .then_some((instruction_index, *instruction))
                        .or(original_none_marker)
                        && marker.exception_policy().exclusion() == super::ExceptionExclusion::None
                        && matches!(
                            marker.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && owner_history_exact
                        && tail.late_removals == 0
                        && let Some((start, end, start_tail, end_tail)) = ended_owned_prefix
                        && (tail.insertions == 0
                            || (tail.post_compaction_fusions <= tail.insertions
                                && start_tail.insertions == tail.insertions))
                    {
                        // Prologue insertion consumes the newer fusion suffix first. Once
                        // exhausted, the marker reuses the earlier compaction tail, as it
                        // does for a protected suffix that has not reached its cleanup.
                        let (removed, suffix_width, prefix_width) = if tail.post_compaction_fusions
                            > tail.insertions
                        {
                            (
                                tail.post_compaction_fusions,
                                marker_index - end + tail.post_compaction_fusions
                                    - end_tail.post_compaction_fusions,
                                end - start + end_tail.post_compaction_fusions
                                    - start_tail.post_compaction_fusions,
                            )
                        } else {
                            (
                                tail.early_removals,
                                marker_index - end + tail.early_removals - end_tail.early_removals,
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
                            if marker_index == instruction_index {
                                owners.insert(
                                    instruction_index,
                                    ExceptionOwnerDecision {
                                        has_owner: true,
                                        source_region: Some(region),
                                        definitive: true,
                                        override_existing: true,
                                    },
                                );
                            } else {
                                pending_compacted_append = Some((marker_index, region));
                            }
                        }
                    }
                    // After fusion, the stale suffix is the pre-fusion live array.
                    // Include the retained NULL POP_BLOCK in the cleanup width. A slot
                    // strictly before it belongs to the certified protected prefix.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::Present
                        )
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
                            // retained NOP is the only record before the protected body,
                            // or follows an unchanged ordinary-await completion and discard.
                            // Earlier arrays cannot contribute stale slots across this
                            // authored boundary, even if an enclosing region began earlier.
                            || ((start == original_array_start + 1
                                || (start == original_array_start + 3
                                    && self.unchanged_discarded_await_end(
                                        original_array_start,
                                        &indexed_instructions,
                                        &label_positions,
                                    )))
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
                        if start == original_array_start + 3
                            && self.unchanged_discarded_await_end(
                                original_array_start,
                                &indexed_instructions,
                                &label_positions,
                            )
                        {
                            pending_await_null_append = Some(instruction_index + 1);
                        }
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
                    // Captured handler and finalizer POPs retain their own NULL cells.
                    // A later POP must not replace an earlier physical record.
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && captured_null_slots.iter().any(|(start, removed)| {
                            *start >= original_array_start
                                && start.checked_add(*removed) == Some(instruction_index + 1)
                        })
                    {
                        pending_null_append = Some(instruction_index + 1);
                    }
                    // Range endpoints between the condition and its marker do not
                    // change an authenticated original slot. Replay at the marker,
                    // retaining the same array, epoch, and exact-history requirements.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && instruction.exception_policy().owner()
                            != NormalizedExceptionOwner::Dynamic
                        && !owners.contains_key(&instruction_index)
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && let Some((_, _, region)) =
                            captured_handler_owner_slots
                                .iter()
                                .find(|(array, slot, _)| {
                                    *array == original_array_start && *slot == instruction_index
                                })
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: true,
                                source_region: Some(*region),
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    // This separate origin proof starts with one retained normal POP_BLOCK.
                    // Known protected folds leave an owned stale tail. Carry that
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
                    // Earlier folds and fusions precede this compaction, so their removed
                    // slot counts do not affect its surviving records or SETUP positions.
                    // The appended marker reuses its pre-compaction physical slot, which
                    // can be an erased NULL SETUP or an earlier manager-entry instruction.
                    if instruction.opcode == NOT_TAKEN
                        && late_setup_history_known
                        && late_setup_positions.len() == tail.late_removals
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::OverridePresent
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && uniform_with_suffix
                            .is_some_and(|start| tail.late_removals > instruction_index - start)
                    {
                        let removed_before = late_setup_positions
                            .partition_point(|position| *position < instruction_index);
                        let reuses_null_setup =
                            late_setup_positions.get(removed_before) == Some(&instruction_index);
                        let source_region = if reuses_null_setup {
                            None
                        } else {
                            let source = instruction_index - removed_before;
                            self.uniform_exception_owner(source, source + 1, &label_positions)
                        };
                        // A structural override is weaker than a certified old-array
                        // region or NULL SETUP, but must not replace an earlier
                        // definitive decision. Only an exact SETUP position proves NULL;
                        // failure to recover a source owner does not.
                        if instruction.exception_policy().owner()
                            != NormalizedExceptionOwner::OverridePresent
                            || (source_region.is_some_and(|region| {
                                !self.exception_regions[region].is_scope_fallback
                            }) && owners
                                .get(&instruction_index)
                                .is_none_or(|owner| !owner.definitive))
                            || (reuses_null_setup
                                && owners
                                    .get(&instruction_index)
                                    .is_none_or(|owner| !owner.definitive))
                        {
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
                    }
                    // The unnamed-handler return has two separate NULL POP cells. If
                    // both disappear, the later comparison append can reuse POP_EXCEPT's
                    // original protected cell. Replay only the captured unwind records;
                    // metadata labels must not turn this into a new original array.
                    if matches!(instruction.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction_index >= 3
                    {
                        let pop_index = instruction_index - 2;
                        let swap_index = pop_index - 1;
                        for (entry, [swap, _, pop_except, _, load, condition]) in &self
                            .exception_owner_provenance
                            .copied_handler_return_none_prefixes
                        {
                            if instruction != condition
                                || label_positions.get(entry) != Some(&original_array_start)
                                || original_array_start > swap_index
                                || jump_targets.iter().any(|label| {
                                    label_positions.get(label).is_some_and(|position| {
                                        swap_index < *position && *position <= instruction_index + 1
                                    })
                                })
                            {
                                continue;
                            }
                            let mut expected_pop = *pop_except;
                            expected_pop
                                .local_policy
                                .add_hidden_exception_owner_slots(1);
                            let mut expected_load = *load;
                            expected_load
                                .local_policy
                                .add_hidden_exception_owner_slots(1);
                            if indexed_instructions[instruction_index - 1].opcode
                                == crate::target::opcodes::LOAD_FAST_BORROW
                            {
                                expected_load.opcode = crate::target::opcodes::LOAD_FAST_BORROW;
                            }
                            if *indexed_instructions[swap_index] != *swap
                                || *indexed_instructions[pop_index] != expected_pop
                                || *indexed_instructions[instruction_index - 1] != expected_load
                                || owners.contains_key(&pop_index)
                            {
                                continue;
                            }
                            // The final POP and three comparison cells follow POP_EXCEPT;
                            // all earlier removals shift its original cell to the right.
                            if tail
                                .early_removals
                                .checked_sub(4)
                                .and_then(|removed| pop_index.checked_add(removed))
                                == Some(instruction_index + 1)
                                && let Some(region) = self.uniform_exception_owner(
                                    pop_index,
                                    pop_index + 1,
                                    &label_positions,
                                )
                                && !self.exception_regions[region].is_scope_fallback
                                && !self.exception_regions[region].preserve_lasti
                            {
                                pending_normal_none_owner =
                                    Some((instruction_index + 1, Some(region)));
                            }
                        }
                    }
                    // A normal-finally local None test removes exactly three comparison
                    // records. With a retained POP its marker reuses IS_OP; with one
                    // deleted POP it reuses LOAD_CONST(None). Both belong to the unchanged
                    // local-test region, not the surrounding resumable-scope fallback.
                    if matches!(instruction.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction_index > 0
                        && !owners.contains_key(&(instruction_index - 1))
                        && !owners.contains_key(&instruction_index)
                    {
                        let load_index = instruction_index - 1;
                        let load = *indexed_instructions[load_index];
                        let normal_prefixes = self
                            .exception_owner_provenance
                            .normal_finally_none_prefixes
                            .iter()
                            .map(|(pop, load, condition)| (pop, load, condition, None, false));
                        let copied_prefixes = self
                            .exception_owner_provenance
                            .copied_manager_return_none_prefixes
                            .iter()
                            .map(|origin| {
                                (
                                    &origin.pop,
                                    &origin.load,
                                    &origin.condition,
                                    Some(origin),
                                    false,
                                )
                            });
                        let paired_prefixes = self
                            .exception_owner_provenance
                            .copied_return_none_prefixes
                            .iter()
                            .map(|origin| {
                                (&origin.pops[1], &origin.load, &origin.condition, None, true)
                            });
                        for (original_pop, original_load, original_condition, copied, paired) in
                            normal_prefixes
                                .chain(copied_prefixes)
                                .chain(paired_prefixes)
                        {
                            if instruction != original_condition {
                                continue;
                            }
                            let removed_pop = load.local_policy.hidden_exception_owner_slots();
                            if (paired && removed_pop != 2) || (!paired && removed_pop > 1) {
                                continue;
                            }
                            let mut expected_load = *original_load;
                            expected_load
                                .local_policy
                                .add_hidden_exception_owner_slots(removed_pop);
                            if load.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                                expected_load.opcode = load.opcode;
                            }
                            if load != expected_load {
                                continue;
                            }
                            let prefix_start = if removed_pop == 0 {
                                let Some(pop_index) = load_index.checked_sub(1) else {
                                    continue;
                                };
                                let pop = *indexed_instructions[pop_index];
                                let mut expected_pop = *original_pop;
                                expected_pop.location = pop.location;
                                // Strengthening an already unconditional POP exclusion to
                                // an explicit absent owner preserves this original NULL cell.
                                if expected_pop.exception_policy()
                                    == super::ExceptionPolicy::EXCLUDE_ALWAYS
                                    && pop.exception_policy()
                                        == super::ExceptionPolicy::AlwaysAbsent
                                {
                                    expected_pop
                                        .set_exception_policy(super::ExceptionPolicy::AlwaysAbsent);
                                }
                                if pop != expected_pop {
                                    continue;
                                }
                                pop_index
                            } else {
                                load_index
                            };
                            if original_array_start > prefix_start
                                || jump_targets.iter().any(|label| {
                                    label_positions.get(label).is_some_and(|position| {
                                        prefix_start < *position
                                            && *position <= instruction_index + 1
                                    })
                                })
                            {
                                continue;
                            }
                            let omitted_pops = if let Some(origin) = copied {
                                if removed_pop != 1
                                    || tail.early_removals != 4
                                    || label_positions.get(&origin.entry)
                                        != Some(&original_array_start)
                                    || original_array_start.checked_add(origin.prefix.len())
                                        != Some(load_index)
                                    || jump_targets.iter().any(|label| {
                                        label_positions.get(label).is_some_and(|position| {
                                            original_array_start < *position
                                                && *position <= instruction_index + 1
                                        })
                                    })
                                    || indexed_instructions[original_array_start..load_index]
                                        .iter()
                                        .zip(origin.prefix)
                                        .any(|(actual, mut expected)| {
                                            if expected.opcode == crate::target::opcodes::LOAD_FAST
                                                && actual.opcode
                                                    == crate::target::opcodes::LOAD_FAST_BORROW
                                            {
                                                expected.opcode = actual.opcode;
                                            }
                                            **actual != expected
                                        })
                                {
                                    continue;
                                }
                                // Only this intact source prefix omits the manager POP and
                                // surrounding ordinary-try POP. Do not change generic history.
                                2
                            } else {
                                0
                            };
                            // Earlier compaction can instead put the appended marker in
                            // the captured POP_BLOCK's original NULL cell. Remove only this
                            // prefix's hidden records to recover that exact old slot.
                            let comparison_removals = usize::from(
                                original_condition
                                    .local_policy
                                    .hidden_exception_owner_slots(),
                            );
                            let original_pop_slot = tail
                                .early_removals
                                .checked_add(omitted_pops)
                                .and_then(|removed| {
                                    removed
                                        // A captured pair's first POP precedes the final NULL
                                        // POP. Keep its removal in the earlier-slot count.
                                        .checked_sub(
                                            comparison_removals + usize::from(removed_pop > 0),
                                        )
                                })
                                .and_then(|removed_before| load_index.checked_add(removed_before));
                            if removed_pop > 0 && original_pop_slot == Some(instruction_index + 1) {
                                pending_normal_none_owner = Some((instruction_index + 1, None));
                                continue;
                            }
                            // With both original POPs erased, earlier folds can put the
                            // appended marker before either NULL cell. The captured pair
                            // fixes their positions; only a uniformly protected original
                            // prefix can donate the old instruction's owner.
                            if paired
                                && pop_slot_epoch_known
                                && original_pop_slot
                                    .and_then(|last_pop| last_pop.checked_sub(1))
                                    .is_some_and(|first_pop| instruction_index + 1 < first_pop)
                                && let Some(region) = self.uniform_exception_owner(
                                    original_array_start,
                                    load_index,
                                    &label_positions,
                                )
                                && !self.exception_regions[region].is_scope_fallback
                                && !self.exception_regions[region].preserve_lasti
                                && indexed_instructions[original_array_start..load_index]
                                    .iter()
                                    .enumerate()
                                    .all(|(offset, previous)| {
                                        let index = original_array_start + offset;
                                        previous.exception_policy().exclusion()
                                            == super::ExceptionExclusion::None
                                            && previous.exception_policy().owner()
                                                == NormalizedExceptionOwner::Unchanged
                                            && !owners.contains_key(&index)
                                            && (previous
                                                .local_policy
                                                .hidden_exception_owner_slots()
                                                == 0
                                                || original_uniform_folds.contains(&index)
                                                || self
                                                    .exception_owner_provenance
                                                    .uniform_fold_owner_slots
                                                    .iter()
                                                    .any(|(location, opcode, slots)| {
                                                        previous.location == *location
                                                            && previous.opcode == *opcode
                                                            && previous
                                                                .local_policy
                                                                .hidden_exception_owner_slots()
                                                                == *slots
                                                            && previous
                                                                .pre_fold_extra_instructions()
                                                                == (*slots).min(4) as u8
                                                    }))
                                    })
                            {
                                pending_normal_none_owner =
                                    Some((instruction_index + 1, Some(region)));
                                continue;
                            }
                            if paired
                                || copied.is_some()
                                || tail.early_removals != 3 + usize::from(removed_pop)
                            {
                                continue;
                            }
                            if let Some(region) = self.uniform_exception_owner(
                                load_index,
                                instruction_index + 1,
                                &label_positions,
                            ) && !self.exception_regions[region].is_scope_fallback
                            {
                                pending_normal_none_owner =
                                    Some((instruction_index + 1, Some(region)));
                            }
                        }
                    }
                    // A retained normal POP_BLOCK separates the old protected prefix from
                    // the cleanup suffix without removing its NULL cell. Local fusion
                    // compacts that live array again. If its stale slot stays inside the
                    // unchanged cleanup suffix, use that suffix's exact region, even when
                    // source-level cleanup exclusions hide it around the late marker.
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && threaded_conditions.get(&instruction_index)
                            != Some(&NormalizedExceptionOwner::DynamicConditionStrict)
                        && owner_history_exact
                        && has_uninterrupted_fusion
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions > 0
                        && let Some(pop) = indexed_instructions
                            [original_array_start..instruction_index]
                            .iter()
                            .rposition(|instruction| instruction.opcode == NOP)
                            .map(|offset| original_array_start + offset)
                        && indexed_instructions[pop]
                            .fusion_policy
                            .is_converted_pop_block()
                        && indexed_instructions[pop].exception_policy().exclusion()
                            == super::ExceptionExclusion::Always
                    {
                        let suffix = &indexed_instructions[pop + 1..=instruction_index];
                        let fused = suffix
                            .iter()
                            .filter(|instruction| {
                                matches!(
                                    instruction.opcode,
                                    LOAD_FAST_LOAD_FAST
                                        | LOAD_FAST_BORROW_LOAD_FAST_BORROW
                                        | STORE_FAST_LOAD_FAST
                                        | STORE_FAST_STORE_FAST
                                )
                            })
                            .count();
                        if fused > 0
                            && tail.post_compaction_fusions <= suffix.len() + fused
                            && suffix.iter().all(|instruction| {
                                instruction.exception_policy()
                                    == super::ExceptionPolicy::NoneUnchanged
                                    && instruction.local_policy.hidden_exception_owner_slots() == 0
                                    && instruction.pre_fold_extra_instructions() == 0
                                    && instruction.no_location_policy.borrow_fallthrough()
                                        != super::BorrowFallthrough::Barrier
                                    && !instruction.only_allows_unconditional_inlining()
                            })
                            && let Some(region) = self.uniform_exception_owner(
                                pop + 1,
                                instruction_index + 1,
                                &label_positions,
                            )
                            && !self.exception_regions[region].is_scope_fallback
                            && self.exception_regions[region].depth > 0
                            && self.exception_regions[region].preserve_lasti
                        {
                            pending_compacted_append = Some((instruction_index + 1, region));
                        }
                    }
                    // A Boolean identity or membership fold removes IS_OP or CONTAINS_OP
                    // before TO_BOOL takes its place. After a retained original-head
                    // POP_BLOCK, that first compaction leaves the condition's manager
                    // owner in the next slot.
                    // Any subsequent local fusion still reuses the same uniformly owned
                    // suffix. Account for every removed record before trusting that slot.
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && threaded_conditions.get(&instruction_index)
                            != Some(&NormalizedExceptionOwner::DynamicConditionStrict)
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && instruction_index >= original_array_start + 2
                    {
                        let pop = indexed_instructions[original_array_start];
                        let comparison = indexed_instructions[instruction_index - 1];
                        let suffix =
                            &indexed_instructions[original_array_start + 1..=instruction_index];
                        let fused = suffix
                            .iter()
                            .filter(|instruction| {
                                matches!(
                                    instruction.opcode,
                                    LOAD_FAST_LOAD_FAST
                                        | LOAD_FAST_BORROW_LOAD_FAST_BORROW
                                        | STORE_FAST_LOAD_FAST
                                        | STORE_FAST_STORE_FAST
                                )
                            })
                            .count();
                        if pop.opcode == NOP
                            && pop.fusion_policy.is_converted_pop_block()
                            && pop.exception_policy().exclusion()
                                == super::ExceptionExclusion::Always
                            && pop.local_policy.hidden_exception_owner_slots() == 0
                            && pop.pre_fold_extra_instructions() == 0
                            && pop.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Barrier
                            && !pop.only_allows_unconditional_inlining()
                            && matches!(
                                comparison.opcode,
                                crate::target::opcodes::CONTAINS_OP | crate::target::opcodes::IS_OP
                            )
                            && comparison.location == instruction.location
                            && comparison.local_policy.hidden_exception_owner_slots() == 1
                            && tail.post_compaction_fusions == fused
                            && tail.early_removals == 1 + fused
                            && self
                                .exception_owner_provenance
                                .source_boundaries
                                .iter()
                                .any(|label| {
                                    self.exception_owner_provenance
                                        .block_boundaries
                                        .contains(label)
                                        && label_positions.get(label) == Some(&original_array_start)
                                })
                            && !boundaries
                                .iter()
                                .chain(&self.exception_owner_provenance.source_boundaries)
                                .chain(&jump_targets)
                                .any(|label| {
                                    label_positions.get(label).is_some_and(|position| {
                                        original_array_start < *position
                                            && *position <= instruction_index
                                    })
                                })
                            && suffix.iter().enumerate().all(|(offset, instruction)| {
                                instruction.exception_policy()
                                    == super::ExceptionPolicy::NoneUnchanged
                                    && instruction.local_policy.hidden_exception_owner_slots()
                                        == u16::from(offset + 2 == suffix.len())
                                    && instruction.pre_fold_extra_instructions() == 0
                                    && instruction.no_location_policy.borrow_fallthrough()
                                        != super::BorrowFallthrough::Barrier
                                    && !instruction.only_allows_unconditional_inlining()
                            })
                            && let Some(region) = self.uniform_exception_owner(
                                original_array_start + 1,
                                instruction_index + 1,
                                &label_positions,
                            )
                            && !self.exception_regions[region].is_scope_fallback
                            && self.exception_regions[region].depth > 0
                            && self.exception_regions[region].preserve_lasti
                        {
                            pending_compacted_append = Some((instruction_index + 1, region));
                        }
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
                    if threaded_conditions.get(&instruction_index)
                        == Some(&NormalizedExceptionOwner::DynamicConditionStrict)
                        && owner_history_exact
                        && has_uninterrupted_fusion
                        && tail.post_compaction_fusions == 1
                        && tail.insertions == 0
                        && tail.late_removals == 0
                    {
                        pending_threaded_null_append = Some(instruction_index + 1);
                    }
                    // Value-jump expansion writes three NULL-owned tail cells. Known
                    // late SETUP compaction by one to three records makes the appended
                    // marker reuse one of those cells, regardless of earlier folds or
                    // prologue insertions. Zero removals can expose an older stale cell.
                    if threaded_conditions.get(&instruction_index)
                        == Some(&NormalizedExceptionOwner::Dynamic)
                        && owner_history_exact
                        && value_jump_setup_history_known
                        && (1..=3).contains(&tail.late_removals)
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
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && let Some((index, region)) = pending_normal_none_owner.take()
                        && index == instruction_index
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|owner| !owner.definitive)
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision {
                                has_owner: region.is_some(),
                                source_region: region,
                                definitive: true,
                                override_existing: true,
                            },
                        );
                    }
                    if instruction.opcode == NOT_TAKEN
                        && instruction_index > 0
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|owner| !owner.definitive)
                    {
                        for (condition, marker, region) in
                            &self.exception_owner_provenance.fused_manager_owners
                        {
                            if instruction == marker
                                && indexed_instructions[instruction_index - 1] == condition
                                && let Some(source_region) =
                                    self.exception_regions.iter().position(|candidate| {
                                        super::fusion_owner_slots::same_manager_region(
                                            candidate, region,
                                        )
                                    })
                            {
                                owners.insert(
                                    instruction_index,
                                    ExceptionOwnerDecision {
                                        has_owner: true,
                                        source_region: Some(source_region),
                                        definitive: true,
                                        override_existing: true,
                                    },
                                );
                            }
                        }
                    }
                    // Resolve only a concrete live cleanup record in the original array.
                    // Missing named-handler POPs precede this span; all existing hidden
                    // records remain counted exactly once by the ordinary tail scanner.
                    if instruction.opcode == NOT_TAKEN
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|decision| !decision.definitive)
                        && let Some((array, start, end, end_removed, missing, continued_call)) =
                            copied_handler_prefix
                        && array == original_array_start
                        && end <= instruction_index
                        && tail.early_removals >= end_removed
                        && (if let Some((prefix, marker)) = continued_call {
                            instruction_index - end == prefix.len()
                                && *instruction == marker
                                && indexed_instructions[end..instruction_index]
                                    .iter()
                                    .zip(prefix)
                                    .all(|(actual, mut expected)| {
                                        if expected.opcode == crate::target::opcodes::LOAD_FAST
                                            && actual.opcode
                                                == crate::target::opcodes::LOAD_FAST_BORROW
                                        {
                                            expected.opcode = actual.opcode;
                                        }
                                        **actual == expected
                                    })
                        } else {
                            previous.is_some_and(|previous: Instruction| {
                                matches!(previous.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                                    && previous.local_policy.hidden_exception_owner_slots() == 3
                            })
                        })
                        && let Some(removed) = tail.early_removals.checked_add(missing)
                    {
                        let suffix = instruction_index - end + tail.early_removals - end_removed;
                        // Equality selects the original NULL finally POP immediately after
                        // cleanup, not its preceding POP_EXCEPT. Its one transferred record
                        // must be the only hidden slot on the immediately following load.
                        if removed == suffix
                            && instruction_index - end == 2
                            && matches!(
                                indexed_instructions[end].opcode,
                                crate::target::opcodes::LOAD_FAST
                                    | crate::target::opcodes::LOAD_FAST_BORROW
                                    | crate::target::opcodes::LOAD_DEREF
                            )
                            && indexed_instructions[end]
                                .local_policy
                                .hidden_exception_owner_slots()
                                == 1
                            && self
                                .exception_owner_provenance
                                .source_none_conditions
                                .iter()
                                .filter(|origin| Some(origin.instruction) == previous)
                                .count()
                                == 1
                            && self
                                .exception_owner_provenance
                                .removed_pop_block_boundaries
                                .iter()
                                .any(|label| {
                                    label_positions.get(label) == Some(&end)
                                        && self
                                            .exception_owner_provenance
                                            .removed_pop_block_widths
                                            .get(label)
                                            .copied()
                                            .unwrap_or(1)
                                            == 1
                                })
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
                        if removed > suffix && removed - suffix <= end - start {
                            let source = end - (removed - suffix);
                            if let Some(region) =
                                self.uniform_exception_owner(source, source + 1, &label_positions)
                                && !self.exception_regions[region].is_scope_fallback
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
                    }
                    // Before the captured authored-return slot, an appended marker can
                    // reuse the uniformly protected prefix. The retained POP remains NULL.
                    if instruction.opcode == NOT_TAKEN
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                        )
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|decision| !decision.definitive)
                        && let Some((array, pop, slot, region)) = early_return_nop_slot
                        && original_array_start == array
                        && pop < instruction_index
                        && instruction_index < slot
                        && self.uniform_exception_owner(array, pop, &label_positions)
                            == Some(region)
                        && indexed_instructions[array..pop].iter().all(|instruction| {
                            instruction.exception_policy().exclusion()
                                == super::ExceptionExclusion::None
                                && instruction.exception_policy().owner()
                                    == NormalizedExceptionOwner::Unchanged
                                && instruction.local_policy.hidden_exception_owner_slots() == 0
                                && instruction.pre_fold_extra_instructions() == 0
                                && instruction.no_location_policy.borrow_fallthrough()
                                    != super::BorrowFallthrough::Barrier
                        })
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
                    // The explicit TO_BOOL places a truthiness marker one cell later
                    // than a None test. Either original POP cell is still NULL; the
                    // preceding authored return NOP retains its protected owner.
                    if instruction.opcode == NOT_TAKEN
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                        )
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                        && instruction.pre_fold_extra_instructions() == 0
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|decision| !decision.definitive)
                        && let Some((array, pop, slot)) = early_return_truth_slot
                        && original_array_start == array
                        && instruction_index - pop == 4
                        && (slot.checked_add(1) == Some(instruction_index)
                            || slot.checked_add(2) == Some(instruction_index))
                        && instruction.location == indexed_instructions[pop + 3].location
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
                    // A captured POP pair remains the same original array when
                    // the actual threading pass retargets its terminal condition.
                    // Only that exact rewritten condition/marker pair can supersede
                    // the generic Strict presence estimate below.
                    let threaded_paired_pop_marker = instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::DynamicConditionStrict
                        && early_return_nop_slot.is_some_and(|(_, pop, _, _)| {
                            indexed_instructions[pop]
                                .local_policy
                                .hidden_exception_owner_slots()
                                == 2
                        })
                        && previous.is_some_and(|condition| {
                            self.exception_owner_provenance
                                .threaded_condition_origins
                                .contains(&(condition, *instruction))
                                // Another copied condition can leave an identical
                                // historical origin after later normalization.
                                // The captured POP prefix must identify the sole
                                // surviving condition/marker pair.
                                && indexed_instructions
                                    .windows(2)
                                    .filter(|pair| {
                                        *pair[0] == condition && *pair[1] == *instruction
                                    })
                                    .count()
                                    == 1
                        });
                    // Early compaction leaves the authored exit NOP and its adjacent
                    // POP_BLOCK records in their old array cells. The first retains its
                    // try owner; either captured POP is NULL despite a scope fallback.
                    if instruction.opcode == NOT_TAKEN
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && (matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                        ) || threaded_paired_pop_marker)
                        && owners.get(&instruction_index).is_none_or(|decision| {
                            !decision.definitive
                                || (threaded_paired_pop_marker
                                    && decision.has_owner
                                    && decision.source_region.is_none()
                                    && !decision.override_existing)
                        })
                        && let Some((array, pop, slot, region)) = early_return_nop_slot
                        && original_array_start == array
                        && (instruction_index == slot
                            || slot.checked_add(1) == Some(instruction_index)
                            || (indexed_instructions[pop]
                                .local_policy
                                .hidden_exception_owner_slots()
                                == 2
                                && slot.checked_add(2) == Some(instruction_index)))
                        && instruction_index - pop == 3
                        && matches!(
                            indexed_instructions[pop + 1].opcode,
                            crate::target::opcodes::LOAD_FAST
                                | crate::target::opcodes::LOAD_FAST_BORROW
                                | crate::target::opcodes::LOAD_DEREF
                        )
                        && indexed_instructions[pop + 1]
                            .local_policy
                            .hidden_exception_owner_slots()
                            == 0
                        && previous.is_some_and(|previous: Instruction| {
                            matches!(previous.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                                && previous.local_policy.hidden_exception_owner_slots() == 3
                        })
                    {
                        let source_region = (instruction_index == slot).then_some(region);
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
                    // The last SETUP compaction copies live records left and leaves
                    // their old tail cells intact. Revalidate this original entry array
                    // separately: retained SETUP NOPs can carry earlier POP annotations
                    // that intentionally invalidate the generic early-history proofs.
                    if instruction.opcode == NOT_TAKEN
                        && original_array_start == 0
                        && owner_history_exact
                        && tail.late_removals == late_setup_positions.len()
                        && tail.post_compaction_fusions == 0
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Present
                        && owners
                            .get(&instruction_index)
                            .is_none_or(|decision| !decision.definitive)
                        && self.entry_retained_setup_null_marker(
                            instruction_index,
                            &indexed_instructions,
                            &label_positions,
                            &jump_targets,
                            &late_setup_positions,
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
                    instruction_index += 1;
                    has_instruction = true;
                    if matches!(instruction.opcode, FOR_ITER | SEND) {
                        has_uninterrupted_fusion = false;
                        late_setup_history_known = false;
                        value_jump_setup_history_known = false;
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
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                        tail = ExceptionOwnerTail::default();
                        has_uninterrupted_fusion = false;
                        late_setup_positions.clear();
                        late_setup_history_known = true;
                        value_jump_setup_history_known = true;
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

    /// Recognizes the unchanged result discard at an authored ordinary-await entry.
    fn unchanged_discarded_await_end(
        &self,
        start: usize,
        instructions: &[&Instruction],
        positions: &FxHashMap<Label, usize>,
    ) -> bool {
        let Some([end_send, discard]) = instructions.get(start..start + 2) else {
            return false;
        };
        if discard.opcode != POP_TOP
            || discard.operand() != Operand::Value(0)
            || discard.location != end_send.location
            || !matches!(
                (discard.depth_after(), end_send.depth_after()),
                (Some(discard_depth), Some(end_depth))
                    if discard_depth.checked_add(1) == Some(end_depth)
            )
            || discard.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            || discard.local_policy.hidden_exception_owner_slots() != 0
            || discard.pre_fold_extra_instructions() != 0
            || discard.no_location_policy.borrow_fallthrough()
                != super::BorrowFallthrough::Reachable
        {
            return false;
        }
        let mut origins = self
            .exception_owner_provenance
            .ordinary_await_ends
            .iter()
            .filter(|origin| {
                origin.end_send == **end_send && positions.get(&origin.entry) == Some(&start)
            });
        let Some(origin) = origins.next() else {
            return false;
        };
        let entry = &origin.entry;
        if origins.next().is_some()
            || !self
                .exception_owner_provenance
                .block_boundaries
                .contains(entry)
            || self
                .exception_owner_provenance
                .normal_try_copy_blocks
                .contains(entry)
            || self
                .exception_owner_provenance
                .normal_try_copy_inline_labels
                .contains(entry)
            || self
                .exception_owner_provenance
                .normal_try_copy_internal_labels
                .contains(entry)
            || self
                .exception_owner_provenance
                .normal_try_copy_jump_nop_successors
                .contains(entry)
        {
            return false;
        }
        let Some(first_cleanup) = origin.cleanup.first() else {
            return false;
        };
        let mut cleanups = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| *item == first_cleanup);
        let Some((cleanup_start, _)) = cleanups.next() else {
            return false;
        };
        let cleanup_end = cleanup_start + origin.cleanup.len();
        if cleanups.next().is_some()
            || self.items.get(cleanup_start..cleanup_end) != Some(origin.cleanup.as_slice())
        {
            return false;
        }
        if let Some((label, bridge)) = origin.bridge {
            if self.items.get(cleanup_end..cleanup_end + 2)
                != Some([Item::Label(label), Item::Instruction(bridge)].as_slice())
                || self
                    .items
                    .iter()
                    .filter(|item| **item == Item::Label(label))
                    .count()
                    != 1
            {
                return false;
            }
        } else if self.items.get(cleanup_end) != Some(&Item::Label(*entry)) {
            return false;
        }
        let mut incoming = instructions.iter().filter(|instruction| {
            matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                if positions.get(&label) == Some(&start))
        });
        let mut seen_send = false;
        let mut seen_bridge = false;
        let exact = incoming.all(|instruction| {
            if **instruction == origin.send && !seen_send {
                seen_send = true;
                true
            } else if origin
                .bridge
                .is_some_and(|(_, bridge)| bridge == **instruction)
                && !seen_bridge
            {
                seen_bridge = true;
                true
            } else {
                false
            }
        });
        exact && seen_send && seen_bridge == origin.bridge.is_some()
    }

    /// Projects the appended marker through the last compaction of a captured manager return.
    fn copied_manager_return_compaction_owner(
        &self,
        marker: usize,
        array_start: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &FxHashMap<Label, usize>,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<ExceptionOwnerDecision> {
        use crate::target::opcodes::LOAD_FAST_BORROW;

        let condition_index = marker.checked_sub(1)?;
        let load_index = marker.checked_sub(2)?;
        let start = load_index.checked_sub(7)?;
        if array_start > start
            || jump_targets
                .iter()
                .chain(&self.exception_owner_provenance.source_boundaries)
                .chain(&self.exception_owner_provenance.block_boundaries)
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| start < *position && *position <= marker)
                })
        {
            return None;
        }
        let condition = *instructions.get(condition_index)?;
        let load = *instructions.get(load_index)?;
        let mut origins = self
            .exception_owner_provenance
            .copied_manager_return_compactions
            .iter()
            .filter(|origin| {
                let mut expected_load = origin.load;
                expected_load
                    .local_policy
                    .add_hidden_exception_owner_slots(1);
                if load.opcode == LOAD_FAST_BORROW {
                    expected_load.opcode = LOAD_FAST_BORROW;
                }
                *condition == origin.condition
                    && *load == expected_load
                    && origin.pop.opcode == NOP
                    && origin.pop.fusion_policy.is_converted_pop_block()
                    && origin.pop.exception_policy() == super::ExceptionPolicy::EXCLUDE_ALWAYS
                    && instructions[start..load_index]
                        .iter()
                        .zip(origin.cleanup)
                        .all(|(actual, expected)| **actual == expected)
                    && origin.earlier_cleanup.is_none_or(|cleanup| {
                        start.checked_sub(cleanup.len()).is_some_and(|earlier| {
                            array_start <= earlier
                                && instructions[earlier..start]
                                    .iter()
                                    .zip(cleanup)
                                    .all(|(actual, expected)| **actual == expected)
                                && !jump_targets
                                    .iter()
                                    .chain(&self.exception_owner_provenance.source_boundaries)
                                    .chain(&self.exception_owner_provenance.block_boundaries)
                                    .any(|label| {
                                        positions.get(label).is_some_and(|position| {
                                            earlier < *position && *position <= start
                                        })
                                    })
                        })
                    })
            });
        let origin = origins.next()?;
        if origins.next().is_some() {
            return None;
        }
        // The prefix must contain only the known LOAD_GLOBAL and local-pair folds.
        // In particular, no unrelated erased POP can shift the manager's old cells.
        let mut prefix_tail = ExceptionOwnerTail::default();
        for instruction in &instructions[array_start..start] {
            if instruction.local_policy.hidden_exception_owner_slots() != 0
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                || instruction.opcode == NOP
                || instruction.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
                || instruction.only_allows_unconditional_inlining()
                || matches!((instruction.opcode, instruction.operand()),
                    (COMPARE_OP, Operand::Value(argument)) if argument & 16 != 0)
                || !matches!(instruction.operand(), Operand::Value(_))
                || is_scope_exit(instruction.opcode)
            {
                return None;
            }
            prefix_tail.instruction(**instruction, None);
        }
        if prefix_tail.insertions != 0
            || prefix_tail.early_removals.checked_add(4) != Some(tail.early_removals)
            || prefix_tail.post_compaction_fusions != tail.post_compaction_fusions
        {
            return None;
        }
        let owns_cleanup_record = if tail.post_compaction_fusions > 0 {
            // Local-load fusion copies full live records left. The stale append cell
            // therefore refers to that live array, after the earlier POP removals.
            let source = marker.checked_sub(tail.post_compaction_fusions)?;
            if !(start..load_index).contains(&source) {
                return None;
            }
            true
        } else {
            // The manager's omitted POP is immediately before these seven records;
            // the optional ordinary-try POP and captured finally POP follow them.
            // These original POP records are NULL before compaction removes them.
            let original_start = start
                .checked_add(prefix_tail.early_removals)?
                // A preceding captured manager contributes its own omitted POP.
                .checked_add(usize::from(origin.earlier_cleanup.is_some()))?
                .checked_add(1)?;
            match marker.checked_sub(original_start)? {
                0..=6 => true,
                7 => false,
                8 if origin.ordinary_try_pops == 1 => false,
                _ => return None,
            }
        };
        let region = self.uniform_exception_owner(start, load_index, positions)?;
        let owner = self.exception_regions[region];
        let current = self.uniform_exception_owner(marker, marker + 1, positions);
        (!owner.is_scope_fallback
            && !owner.preserve_lasti
            && current.is_none_or(|current| self.exception_regions[current].is_scope_fallback))
        .then_some(ExceptionOwnerDecision {
            has_owner: owns_cleanup_record,
            source_region: owns_cleanup_record.then_some(region),
            definitive: true,
            override_existing: true,
        })
    }

    /// Locates a retained NULL SETUP record in an entry array's last compacted tail.
    fn entry_retained_setup_null_marker(
        &self,
        marker: usize,
        instructions: &[&Instruction],
        positions: &FxHashMap<Label, usize>,
        jump_targets: &FxHashSet<Label>,
        removed_positions: &[usize],
    ) -> bool {
        let Some(condition) = marker
            .checked_sub(1)
            .and_then(|index| instructions.get(index))
        else {
            return false;
        };
        let Some(append) = instructions.get(marker) else {
            return false;
        };
        let Some(source) = marker.checked_sub(removed_positions.len()) else {
            return false;
        };
        let provenance = &self.exception_owner_provenance;
        if removed_positions.is_empty()
            || removed_positions
                .last()
                .is_none_or(|position| *position >= marker)
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition.local_policy.hidden_exception_owner_slots() != 3
            || condition.pre_fold_extra_instructions() != 0
            || condition.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            || append.opcode != NOT_TAKEN
            || append.operand() != Operand::Value(0)
            || append.location != condition.location
            || append.local_policy.hidden_exception_owner_slots() != 0
            || append.pre_fold_extra_instructions() != 0
            || append.no_location_policy.borrow_fallthrough() != super::BorrowFallthrough::Reachable
            || append.jump_policy.borrow_empty_block()
            || jump_targets
                .iter()
                .chain(&provenance.block_boundaries)
                .chain(&provenance.source_boundaries)
                .chain(&self.preserved_block_boundaries)
                .chain(self.exception_regions.iter().map(|region| &region.target))
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| 0 < *position && *position <= marker)
                })
            || provenance
                .normal_try_copy_blocks
                .iter()
                .chain(&provenance.normal_try_copy_jump_nop_successors)
                .chain(&provenance.normal_try_copy_inline_labels)
                .chain(&provenance.normal_try_copy_internal_labels)
                .chain(
                    provenance
                        .normal_try_copy_steps
                        .iter()
                        .flat_map(|(a, b, _)| [a, b]),
                )
                .chain(
                    provenance
                        .normal_try_copy_pop_ranges
                        .iter()
                        .flat_map(|(a, b)| [a, b]),
                )
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| *position <= marker)
                })
            || instructions[..marker]
                .iter()
                .enumerate()
                .any(|(index, instruction)| {
                    (index + 1 < marker && !matches!(instruction.operand(), Operand::Value(_)))
                        || matches!(instruction.opcode, NOT_TAKEN | FOR_ITER | SEND)
                        || (is_scope_exit(instruction.opcode)
                            && instruction.opcode != RETURN_GENERATOR)
                        || instruction.no_location_policy.borrow_fallthrough()
                            != super::BorrowFallthrough::Reachable
                        || instruction.jump_policy.borrow_empty_block()
                })
        {
            return false;
        }
        let mut removed = Vec::new();
        let mut retained = FxHashSet::default();
        for (label, location) in &provenance.setups {
            let Some(&position) = positions.get(label) else {
                return false;
            };
            if position > marker {
                continue;
            }
            if let Some(location) = location {
                let Some(index) = position.checked_sub(1) else {
                    return false;
                };
                let instruction = instructions[index];
                if instruction.opcode != NOP
                    || instruction.operand() != Operand::Value(0)
                    || instruction.location != *location
                    || instruction.fusion_policy.is_converted_pop_block()
                    || instruction.pre_fold_extra_instructions() != 0
                    || instruction.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                    || provenance
                        .setup_nop_ranges
                        .iter()
                        .filter(|(start, end)| {
                            positions.get(start) == Some(&index)
                                && positions.get(end) == Some(&position)
                        })
                        .count()
                        != 1
                    || !retained.insert(index)
                {
                    return false;
                }
            } else if provenance.synchronous_with_setups.contains(label)
                || self
                    .exception_regions
                    .iter()
                    .any(|region| region.is_scope_fallback && region.start == *label)
            {
                removed.push(position);
            } else {
                return false;
            }
        }
        removed.sort_unstable();
        if removed
            .iter()
            .enumerate()
            .map(|(offset, position)| position + offset)
            .ne(removed_positions.iter().copied())
            || !retained.contains(&source)
            || instructions[..marker]
                .iter()
                .enumerate()
                .any(|(index, instruction)| instruction.opcode == NOP && !retained.contains(&index))
        {
            return false;
        }
        // Earlier POP compaction can annotate this surviving SETUP with one hidden
        // record. It does not change the SETUP's NULL owner or its final live slot.
        let hidden = instructions[source]
            .local_policy
            .hidden_exception_owner_slots();
        hidden == 0
            || (hidden == 1
                && provenance.removed_pop_block_boundaries.iter().any(|label| {
                    positions.get(label) == Some(&source)
                        && provenance
                            .removed_pop_block_widths
                            .get(label)
                            .copied()
                            .unwrap_or(1)
                            == 1
                })
                && !provenance
                    .omitted_nop_counts
                    .keys()
                    .chain(provenance.omitted_nop_locations.keys())
                    .any(|label| {
                        positions
                            .get(label)
                            .is_some_and(|position| *position == source || *position == source + 1)
                    }))
    }

    /// Reconstructs one entry-array marker slot after three authenticated SETUP removals.
    fn entry_manager_return_setup_owner(
        &self,
        marker: usize,
        instructions: &[&Instruction],
        positions: &FxHashMap<Label, usize>,
        jump_targets: &FxHashSet<Label>,
        removed_positions: &[usize],
    ) -> Option<usize> {
        let condition = instructions.get(marker.checked_sub(1)?)?;
        if !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || condition.local_policy.hidden_exception_owner_slots() != 3
            || jump_targets
                .iter()
                .chain(&self.exception_owner_provenance.block_boundaries)
                .chain(&self.exception_owner_provenance.source_boundaries)
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| 0 < *position && *position <= marker)
                })
            || instructions[..marker - 1].iter().any(|instruction| {
                matches!(
                    instruction.operand(),
                    Operand::Forward(_) | Operand::Backward(_)
                )
            })
        {
            return None;
        }
        let mut removed = Vec::new();
        let mut retained = Vec::new();
        for (label, location) in &self.exception_owner_provenance.setups {
            let &position = positions.get(label)?;
            if position > marker {
                continue;
            }
            if location.is_some_and(|location| {
                position.checked_sub(1).is_some_and(|index| {
                    instructions[index].opcode == NOP && instructions[index].location == location
                })
            }) {
                retained.push(position);
            } else {
                removed.push((position, *label));
            }
        }
        removed.sort_unstable_by_key(|(position, _)| *position);
        let [
            (scope_start, scope),
            (try_start, ordinary),
            (manager_start, manager),
        ] = removed.as_slice()
        else {
            return None;
        };
        let handler = *self
            .exception_owner_provenance
            .ordinary_try_setups
            .get(ordinary)?;
        if *scope_start >= *try_start
            || *try_start >= *manager_start
            || retained.as_slice() != [*try_start]
            || !self
                .exception_regions
                .iter()
                .any(|region| region.is_scope_fallback && region.start == *scope)
            || !self
                .exception_owner_provenance
                .synchronous_with_setups
                .contains(manager)
            || removed
                .iter()
                .enumerate()
                .map(|(offset, (position, _))| position + offset)
                .ne(removed_positions.iter().copied())
            || removed_positions
                .last()
                .is_none_or(|position| *position >= marker)
        {
            return None;
        }
        // Compaction copies complete records left without clearing the old suffix.
        // The appended slot retains the corresponding pre-compaction record's owner.
        let source = marker.checked_sub(removed_positions.len())?;
        let original = *instructions.get(source)?;
        if source < *manager_start
            || original.opcode != POP_TOP
            || original.operand() != Operand::Value(0)
            || original.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            || original.local_policy.hidden_exception_owner_slots() != 0
            || original.pre_fold_extra_instructions() != 0
        {
            return None;
        }
        let region = self.uniform_exception_owner(source, source + 1, positions)?;
        let owner = self.exception_regions[region];
        let current = self.uniform_exception_owner(marker, marker + 1, positions)?;
        (!owner.is_scope_fallback
            && !owner.preserve_lasti
            && owner.target == handler
            && self.exception_regions[current].is_scope_fallback)
            .then_some(region)
    }

    /// Finds unchanged, unambiguous terminal pairs from the actual threading pass.
    fn threaded_condition_positions(&self) -> FxHashMap<usize, NormalizedExceptionOwner> {
        let origins = &self.exception_owner_provenance.threaded_condition_origins;
        if origins.is_empty() {
            return FxHashMap::default();
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
                    || !matches!(
                        origin.1.exception_policy().owner(),
                        NormalizedExceptionOwner::DynamicConditionStrict
                            | NormalizedExceptionOwner::Dynamic
                    )
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
                if matches.next().is_some() {
                    return None;
                }
                let owner = origin.1.exception_policy().owner();
                if owner == NormalizedExceptionOwner::Dynamic {
                    let start = position.checked_sub(2)?;
                    let [copy, to_bool] = &instructions[start..position] else {
                        return None;
                    };
                    if copy.opcode != crate::target::opcodes::COPY
                        || copy.operand() != Operand::Value(1)
                        || to_bool.opcode != crate::target::opcodes::TO_BOOL
                        || [copy, to_bool].iter().any(|instruction| {
                            instruction.exception_policy()
                                != super::ExceptionPolicy::AlwaysUnchanged
                                || instruction.pre_fold_extra_instructions() != 0
                                || instruction.local_policy.hidden_exception_owner_slots() != 0
                        })
                    {
                        return None;
                    }
                }
                Some((position, owner))
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
    fn for_target_owner_prefix_requires_the_exact_surviving_entry() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{
            ExceptionPolicy, Instruction, Item, Label, Operand, SourceLocation,
        };
        use crate::target::opcodes::{LOAD_CONST, POP_TOP, STORE_FAST, STORE_FAST_LOAD_FAST};

        for case in [
            "valid",
            "intervening label",
            "changed operand",
            "changed policy",
            "fused store",
            "hidden zero",
            "hidden two",
            "duplicate body",
            "duplicate origin",
            "missing source boundary",
            "missing block boundary",
            "retired entry",
            "copied entry",
        ] {
            let body = Label(2);
            let location = SourceLocation::new(4, 4, 8, 12);
            let original = Instruction::new(STORE_FAST, Operand::Value(4), location, None);
            let mut surviving = original;
            surviving
                .local_policy
                .add_hidden_exception_owner_slots(match case {
                    "hidden zero" => 0,
                    "hidden two" => 2,
                    _ => 1,
                });
            match case {
                "changed operand" => surviving.set_operand(Operand::Value(5)),
                "changed policy" => surviving.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                "fused store" => surviving.opcode = STORE_FAST_LOAD_FAST,
                _ => {}
            }

            // The body starts at instruction 2, although its store is item 5.
            let mut items = vec![
                Item::Label(Label(0)),
                Item::Instruction(Instruction::new(
                    LOAD_CONST,
                    Operand::Value(0),
                    location,
                    None,
                )),
                Item::Label(Label(1)),
                Item::Instruction(Instruction::new(POP_TOP, Operand::Value(0), location, None)),
                Item::Label(body),
                Item::Instruction(surviving),
            ];
            let mut provenance = ExceptionOwnerProvenance {
                for_target_owner_prefixes: vec![(body, original)],
                source_boundaries: [body].into_iter().collect(),
                block_boundaries: [body].into_iter().collect(),
                ..ExceptionOwnerProvenance::default()
            };
            match case {
                "intervening label" => items.insert(5, Item::Label(Label(3))),
                "duplicate body" => items.push(Item::Label(body)),
                "duplicate origin" => provenance.for_target_owner_prefixes.push((body, original)),
                "missing source boundary" => provenance.source_boundaries.clear(),
                "missing block boundary" => provenance.block_boundaries.clear(),
                "retired entry" => {
                    provenance.retired_normal_try_copy_labels.insert(body);
                }
                "copied entry" => {
                    provenance.normal_try_copy_blocks.insert(body);
                }
                _ => {}
            }

            let positions = provenance.original_for_target_prefix_positions(&items);
            let expected = if case == "valid" {
                [(2, 2)].into_iter().collect()
            } else {
                rustc_hash::FxHashMap::default()
            };
            assert_eq!(positions, expected, "{case}");
        }
    }

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
    fn normal_finally_constant_remapping_preserves_only_exact_surviving_arrays() {
        use crate::assembler::{Assembler, ExceptionPolicy, Item, LayoutAssembler, SourceLocation};
        use crate::target::opcodes::{
            CALL, JUMP_FORWARD, LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_TOP,
            PUSH_NULL, RERAISE, TO_BOOL,
        };

        for change in ["surviving constant", "removed constant", "changed location"] {
            let mut assembler = Assembler::default();
            let entry = assembler.label();
            let setup_start = assembler.label();
            let setup_end = assembler.label();
            let setup = assembler.label();
            let exit = assembler.label();
            assembler.emit(LOAD_CONST, 1);
            assembler.emit(POP_TOP, 0);
            assembler.emit_forward_with_depth(JUMP_FORWARD, entry, 0, ExceptionPolicy::default());
            assembler.emit(RERAISE, 0);
            assembler.record_exception_owner_block_boundary(entry);
            assembler.mark(entry);
            let pop_ordinal = assembler.instruction_count();
            assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
            assembler.mark_last_as_converted_pop_block();
            assembler.set_location(SourceLocation::new(9, 9, 4, 12));
            assembler.emit(LOAD_FAST, 1);
            assembler.emit(PUSH_NULL, 0);
            let load_item = assembler.item_count();
            assembler.emit(LOAD_CONST, 2);
            assembler.emit(CALL, 1);
            assembler.emit(POP_TOP, 0);
            assembler.mark(setup_start);
            assembler.set_location(SourceLocation::new(10, 12, 4, 8));
            assembler.emit(NOP, 0);
            assembler.mark(setup_end);
            assembler.record_setup_nop_boundaries(setup_start, setup_end);
            assembler.mark(setup);
            assembler.record_exception_setup(setup, true);
            assembler.set_location(SourceLocation::new(11, 11, 7, 11));
            assembler.emit(LOAD_FAST, 0);
            assembler.emit(TO_BOOL, 0);
            assembler.emit_forward_with_depth(
                POP_JUMP_IF_FALSE,
                exit,
                0,
                ExceptionPolicy::default(),
            );
            let marker_ordinal = assembler.instruction_count();
            assembler.emit_with_depth(NOT_TAKEN, 0, 0, ExceptionPolicy::NoneAbsent);
            assembler.record_original_normal_finally_array(pop_ordinal);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .original_normal_finally_arrays
                    .len(),
                1
            );

            if change == "changed location" {
                let Item::Instruction(load) = &mut assembler.linear_items_mut()[load_item] else {
                    unreachable!();
                };
                load.location = SourceLocation::new(8, 8, 4, 12);
            }
            assembler.optimize_constant_pops();
            assert!(
                matches!(assembler.linear_items()[0], Item::Instruction(instruction) if instruction.opcode == NOP)
            );
            assert!(
                matches!(assembler.linear_items()[load_item], Item::Instruction(instruction) if instruction.opcode == LOAD_CONST)
            );
            let retained = change != "removed constant";
            assembler.remap_constant_indices(LOAD_CONST, &[Some(0), None, retained.then_some(1)]);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .original_normal_finally_arrays
                    .len(),
                usize::from(retained),
                "{change}"
            );
            let layout = LayoutAssembler {
                items: assembler.linear_items().to_vec(),
                exception_regions: assembler.exception_regions,
                exception_owner_provenance: assembler.exception_owner_provenance,
                preserved_block_boundaries: assembler.preserved_block_boundaries,
                removed_max_depth: None,
                next_label: assembler.next_label,
                has_next_location: false,
            };
            let markers =
                layout.original_normal_finally_null_markers(&[entry, exit].into_iter().collect());
            let expected = if change == "surviving constant" {
                [marker_ordinal].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            };
            assert_eq!(markers, expected, "{change}");
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
                        retained_setup: None,
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
    fn comprehension_restore_spans_require_a_live_original_entry() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{
            Instruction, Item, Label, LayoutAssembler, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            CALL, JUMP_FORWARD, NOT_TAKEN, POP_JUMP_IF_FALSE, STORE_FAST, SWAP, TO_BOOL,
        };

        for case in ["original", "unsealed", "cloned", "retired"] {
            let entry = Label(0);
            let exit = Label(1);
            let clone = Label(2);
            let plain = |opcode, operand| {
                Instruction::new(opcode, operand, SourceLocation::new(3, 3, 0, 4), None)
            };
            let incoming = plain(JUMP_FORWARD, Operand::Forward(entry));
            // Two same-line restores require the complete authored span proof.
            let restores = vec![
                plain(SWAP, Operand::Value(3)),
                plain(STORE_FAST, Operand::Value(0)),
                plain(STORE_FAST, Operand::Value(1)),
            ];
            let mut items = vec![Item::Instruction(incoming), Item::Label(entry)];
            items.extend(restores.iter().copied().map(Item::Instruction));
            items.extend([
                Item::Instruction(plain(CALL, Operand::Value(0))),
                Item::Instruction(plain(TO_BOOL, Operand::Value(0))),
                Item::Instruction(plain(POP_JUMP_IF_FALSE, Operand::Forward(exit))),
                Item::Instruction(plain(NOT_TAKEN, Operand::Value(0))),
                Item::Label(exit),
            ]);
            let mut provenance = ExceptionOwnerProvenance {
                original_comprehension_normal_exits: vec![(entry, incoming)],
                original_comprehension_normal_restores: [(entry, restores)].into_iter().collect(),
                ..ExceptionOwnerProvenance::default()
            };
            if case == "unsealed" {
                provenance.original_comprehension_normal_restores.clear();
            }
            provenance.capture_uncompacted_conditional_arrays(&items);
            assert_eq!(
                provenance.uncompacted_conditional_arrays.len(),
                usize::from(case != "unsealed"),
                "{case}"
            );
            if case == "cloned" {
                // The original is gone; its sole live replacement has a fresh entry.
                items[0] = Item::Instruction(plain(JUMP_FORWARD, Operand::Forward(clone)));
                items[1] = Item::Label(clone);
            } else if case == "retired" {
                // Reusing identical records cannot revive an erased original entry.
                provenance
                    .retired_comprehension_normal_entries
                    .insert(entry);
            }
            let assembler = LayoutAssembler {
                items,
                exception_owner_provenance: provenance,
                exception_regions: vec![],
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 3,
                has_next_location: false,
            };
            let targets = [if case == "cloned" { clone } else { entry }, exit]
                .into_iter()
                .collect();
            let expected = if case == "original" {
                [7].into_iter().collect()
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
    fn uncompacted_source_arrays_require_one_head_and_retained_call_discard() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{
            Instruction, Item, Label, LayoutAssembler, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            CALL, LOAD_ATTR, LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_TOP,
            TO_BOOL,
        };

        for case in [
            "original",
            "block only",
            "source only",
            "adjacent boundary",
            "interior boundary",
            "constant discard",
            "hidden record",
            "copied entry",
            "retired source",
            "retired block",
        ] {
            let location = |line| SourceLocation::new(line, line, 0, 4);
            let plain = |opcode, operand, line| {
                Item::Instruction(Instruction::new(opcode, operand, location(line), None))
            };
            let entry = Label(0);
            let mut items = vec![
                // The folded while test belongs to an earlier original array.
                plain(NOP, Operand::Value(0), 1),
                Item::Label(entry),
                Item::Label(Label(1)),
                plain(NOP, Operand::Value(0), 2),
                Item::Label(Label(2)),
                Item::Label(Label(3)),
                plain(LOAD_FAST, Operand::Value(0), 3),
                plain(LOAD_ATTR, Operand::Value(1), 3),
                plain(CALL, Operand::Value(0), 3),
                plain(POP_TOP, Operand::Value(0), 3),
                plain(LOAD_FAST, Operand::Value(0), 4),
                plain(LOAD_ATTR, Operand::Value(0), 4),
                plain(TO_BOOL, Operand::Value(0), 4),
                plain(POP_JUMP_IF_FALSE, Operand::Forward(Label(4)), 4),
                plain(NOT_TAKEN, Operand::Value(0), 4),
            ];
            let mut provenance = ExceptionOwnerProvenance {
                block_boundaries: [entry].into_iter().collect(),
                source_boundaries: [entry].into_iter().collect(),
                setups: vec![(Label(3), Some(location(2)))],
                setup_nop_ranges: vec![(Label(1), Label(2))],
                ..ExceptionOwnerProvenance::default()
            };
            match case {
                "block only" => provenance.source_boundaries.clear(),
                "source only" => provenance.block_boundaries.clear(),
                "adjacent boundary" => {
                    provenance.block_boundaries.insert(Label(5));
                    provenance.source_boundaries.insert(Label(5));
                    items.insert(1, Item::Label(Label(5)));
                }
                "interior boundary" => {
                    provenance.block_boundaries.insert(Label(3));
                    provenance.source_boundaries.insert(Label(3));
                }
                "constant discard" => items[8] = plain(LOAD_CONST, Operand::Value(0), 3),
                "hidden record" => {
                    let Item::Instruction(instruction) = &mut items[6] else {
                        unreachable!()
                    };
                    instruction.local_policy.add_hidden_exception_owner_slots(1);
                }
                _ => {}
            }
            provenance.capture_uncompacted_conditional_arrays(&items);
            match case {
                "copied entry" => items[1] = Item::Label(Label(6)),
                "retired source" => provenance.source_boundaries.clear(),
                "retired block" => provenance.block_boundaries.clear(),
                _ => {}
            }
            let assembler = LayoutAssembler {
                items,
                exception_owner_provenance: provenance,
                exception_regions: vec![],
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 7,
                has_next_location: false,
            };
            let expected = if case == "original" {
                [10].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            };
            assert_eq!(
                assembler.uncompacted_conditional_null_markers(&[Label(4)].into_iter().collect()),
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
            COMPARE_OP, LOAD_ATTR, LOAD_FAST, LOAD_GLOBAL, NOP, NOT_TAKEN, POP_JUMP_IF_NONE,
            POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, RERAISE, RETURN_VALUE,
        };

        for case in [
            "none",
            "not none",
            "present",
            "global fold",
            "boolean compare",
            "excluded folded prefix",
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
            let opcode = if case == "boolean compare" {
                POP_JUMP_IF_TRUE
            } else if case == "not none" {
                POP_JUMP_IF_NOT_NONE
            } else {
                POP_JUMP_IF_NONE
            };
            let mut conditional = plain(opcode, Operand::Forward(exit));
            if case != "boolean compare" {
                conditional
                    .local_policy
                    .add_hidden_exception_owner_slots(if case == "hidden two" { 2 } else { 3 });
            }
            let mut first = if matches!(case, "global fold" | "excluded folded prefix") {
                plain(LOAD_GLOBAL, Operand::Value(1))
            } else {
                plain(LOAD_FAST, Operand::Value(0))
            };
            if case == "excluded folded prefix" {
                first.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            }
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
                Item::Instruction(if case == "boolean compare" {
                    plain(COMPARE_OP, Operand::Value(88))
                } else {
                    plain(LOAD_ATTR, Operand::Value(0))
                }),
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
            let expected = matches!(
                case,
                "none"
                    | "not none"
                    | "present"
                    | "global fold"
                    | "boolean compare"
                    | "excluded marker"
            );
            assert_eq!(
                certified.get(&3).copied(),
                expected.then_some(Some(0)),
                "{case}"
            );
            let decisions = assembler.dynamic_exception_owners();
            if matches!(
                case,
                "none" | "not none" | "present" | "global fold" | "boolean compare"
            ) {
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

    #[test]
    fn saturated_literal_fold_rejects_changed_and_copied_origins() {
        use crate::assembler::OperandPatchId;
        use crate::assembler::ir::UnoptimizedCode;
        use crate::assembler::{
            Assembler, ExceptionPolicy, Instruction, Item, Operand, SourceLocation,
        };
        use crate::target::opcodes::{LIST_EXTEND, LOAD_CONST, NOP, SET_UPDATE};

        for (opcode, operand) in [
            (LIST_EXTEND, Operand::Value(1)),
            (SET_UPDATE, Operand::Value(1)),
            (LOAD_CONST, Operand::Deferred(OperandPatchId(0))),
        ] {
            for count in [4, 9, 28, 30] {
                let location = SourceLocation::new(1, 1, 0, 12);
                let mut assembler = Assembler {
                    code: UnoptimizedCode::Linear(vec![Item::Instruction(Instruction::new(
                        opcode, operand, location, None,
                    ))]),
                    ..Assembler::default()
                };
                assembler.add_last_constant_fold_slots(count);
                assembler.record_last_uniform_fold_owner_slots(count);
                let Item::Instruction(original) = assembler.linear_items()[0] else {
                    unreachable!();
                };
                let provenance = &assembler.exception_owner_provenance;
                assert!(provenance.uniform_fold_owner_slots.is_empty());
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&original])
                        .contains(&0)
                );

                let mut unchanged = original;
                unchanged.add_pre_fold_extra_instructions(0);
                unchanged.set_operand(operand);
                unchanged.set_exception_policy(ExceptionPolicy::NoneUnchanged);
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&unchanged])
                        .contains(&0)
                );

                // The public count cannot change at saturation, but a foreign
                // earlier instruction must still invalidate the literal transaction.
                let mut foreign = original;
                foreign.add_pre_fold_extra_instructions(1);
                assert_eq!(
                    foreign.pre_fold_extra_instructions(),
                    original.pre_fold_extra_instructions()
                );
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&foreign])
                        .is_empty()
                );

                let mut changed_operand = original;
                changed_operand.set_operand(Operand::Value(2));
                changed_operand.set_operand(operand);
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&changed_operand])
                        .is_empty()
                );
                let mut changed_policy = original;
                changed_policy.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                changed_policy.set_exception_policy(ExceptionPolicy::NoneUnchanged);
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&changed_policy])
                        .is_empty()
                );

                let mut extra_hidden = original;
                extra_hidden
                    .local_policy
                    .add_hidden_exception_owner_slots(1);
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&extra_hidden])
                        .is_empty()
                );
                let fused = Instruction::fused(
                    opcode,
                    operand,
                    original,
                    Instruction::new(NOP, Operand::Value(0), location, None),
                );
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&fused])
                        .is_empty()
                );

                // Even when the original is deleted and only one physical copy
                // survives, uniqueness cannot revive its retired source identity.
                let sole_copy = original.for_exit_copy();
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&sole_copy])
                        .is_empty()
                );
                assert!(
                    provenance
                        .original_uniform_fold_positions(&[&original, &original])
                        .is_empty()
                );
                assembler
                    .exception_owner_provenance
                    .original_uniform_folds
                    .push(original);
                assert!(
                    assembler
                        .exception_owner_provenance
                        .original_uniform_fold_positions(&[&original])
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn deferred_tuple_fold_resolution_preserves_only_unchanged_origins() {
        use crate::assembler::{Assembler, ExceptionPolicy, Item, Operand};
        use crate::target::opcodes::LOAD_CONST;

        for (count, mutation) in [
            (4, "none"),
            (9, "none"),
            (30, "none"),
            (31, "none"),
            (1023, "none"),
            (9, "foreign fold"),
            (9, "operand round trip"),
            (9, "policy round trip"),
            (9, "extra hidden slot"),
            (9, "physical copy"),
            (9, "duplicate witness"),
        ] {
            let mut assembler = Assembler::default();
            let patch = assembler.emit_placeholder_with_depth(LOAD_CONST, 1);
            assembler.add_last_constant_fold_slots(count);
            assembler.record_last_uniform_fold_owner_slots(count);
            let Item::Instruction(mut instruction) = assembler.linear_items()[0] else {
                unreachable!();
            };
            match mutation {
                "foreign fold" => instruction.add_pre_fold_extra_instructions(1),
                "operand round trip" => {
                    instruction.set_operand(Operand::Value(1));
                    instruction.set_operand(Operand::Deferred(patch));
                }
                "policy round trip" => {
                    instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                    instruction.set_exception_policy(ExceptionPolicy::NoneUnchanged);
                }
                "extra hidden slot" => instruction.local_policy.add_hidden_exception_owner_slots(1),
                "physical copy" => instruction = instruction.for_exit_copy(),
                "duplicate witness" => assembler
                    .exception_owner_provenance
                    .original_uniform_folds
                    .push(instruction),
                "none" => {}
                _ => unreachable!(),
            }
            assembler.linear_items_mut()[0] = Item::Instruction(instruction);
            assembler.patch_argument(patch, 17).unwrap();
            assembler.resolve_operand_patches().unwrap();
            let Item::Instruction(resolved) = assembler.linear_items()[0] else {
                unreachable!();
            };
            assert_eq!(resolved.operand(), Operand::Value(17));
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .original_uniform_fold_positions(&[&resolved])
                    .contains(&0),
                count <= 30 && mutation == "none",
                "{count}: {mutation}",
            );
        }
    }

    #[test]
    fn tuple_fold_constant_remapping_preserves_identity_and_duplicates() {
        use crate::assembler::{Assembler, ExceptionPolicy, Item, Operand};
        use crate::target::opcodes::{LOAD_CONST, NOP};

        for mutation in [
            "none",
            "foreign fold",
            "operand round trip",
            "policy round trip",
            "extra hidden slot",
            "physical copy",
            "duplicate survivor",
            "duplicate witness",
            "removed constant",
        ] {
            let mut assembler = Assembler::default();
            let patch = assembler.emit_placeholder_with_depth(LOAD_CONST, 1);
            assembler.add_last_constant_fold_slots(9);
            assembler.record_last_uniform_fold_owner_slots(9);
            assembler.patch_argument(patch, 2).unwrap();
            assembler.resolve_operand_patches().unwrap();
            let Item::Instruction(mut instruction) = assembler.linear_items()[0] else {
                unreachable!();
            };
            match mutation {
                "foreign fold" => instruction.add_pre_fold_extra_instructions(1),
                "operand round trip" => {
                    instruction.set_operand(Operand::Value(1));
                    instruction.set_operand(Operand::Value(2));
                }
                "policy round trip" => {
                    instruction.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                    instruction.set_exception_policy(ExceptionPolicy::NoneUnchanged);
                }
                "extra hidden slot" => instruction.local_policy.add_hidden_exception_owner_slots(1),
                "physical copy" => instruction = instruction.for_exit_copy(),
                "duplicate survivor" => assembler.push_linear_item(Item::Instruction(instruction)),
                "duplicate witness" => assembler
                    .exception_owner_provenance
                    .original_uniform_folds
                    .push(instruction),
                "none" | "removed constant" => {}
                _ => unreachable!(),
            }
            assembler.linear_items_mut()[0] = Item::Instruction(instruction);
            let retained = mutation != "removed constant";
            assembler.remap_constant_indices(LOAD_CONST, &[Some(0), None, retained.then_some(1)]);
            let instructions = assembler
                .linear_items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                instructions[0].opcode,
                if retained { LOAD_CONST } else { NOP }
            );
            assert_eq!(
                instructions[0].operand(),
                Operand::Value(u32::from(retained))
            );
            let certified = assembler
                .exception_owner_provenance
                .original_uniform_fold_positions(&instructions);
            assert_eq!(certified.contains(&0), mutation == "none", "{mutation}");
            assert!(certified.len() <= 1, "{mutation}");
        }
    }

    mod entry_retained_setup_null_marker_tests {
        use super::super::ExceptionOwnerProvenance;
        use crate::assembler::{
            BorrowFallthrough, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
            LayoutAssembler, NormalizedExceptionOwner, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            CALL, LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_NONE, POP_TOP, RESUME,
            RETURN_GENERATOR, RETURN_VALUE, YIELD_VALUE,
        };

        fn fixture(intervening_call: bool) -> LayoutAssembler {
            let location = SourceLocation::new(1, 1, 0, 1);
            let outer_location = SourceLocation::new(4, 4, 4, 8);
            let inner_location = SourceLocation::new(8, 8, 8, 12);
            let guard_location = SourceLocation::new(9, 9, 12, 32);
            let plain = |opcode| Instruction::new(opcode, Operand::Value(0), location, None);
            let outer = Instruction::new(NOP, Operand::Value(0), outer_location, None);
            let mut inner = Instruction::new(NOP, Operand::Value(0), inner_location, None);
            let mut condition = Instruction::new(
                POP_JUMP_IF_NONE,
                Operand::Forward(Label(12)),
                guard_location,
                None,
            );
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), guard_location, None);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);

            let mut items = vec![
                Item::Label(Label(0)),
                Item::Instruction(plain(RETURN_GENERATOR)), // 0: inserted generator prologue.
                Item::Instruction(plain(POP_TOP)),
                Item::Label(Label(1)), // Removed scope SETUP: compact position 2, old position 2.
                Item::Instruction(plain(RESUME)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Label(Label(2)), // First manager SETUP: compact position 4, old position 5.
                Item::Instruction(plain(LOAD_CONST)),
                Item::Label(Label(3)), // Second manager SETUP: compact position 5, old position 7.
                Item::Label(Label(4)),
                Item::Instruction(outer), // 5: retained outer SETUP.
                Item::Label(Label(5)),
                Item::Label(Label(6)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Instruction(plain(YIELD_VALUE)),
                Item::Instruction(plain(RESUME)),
                Item::Instruction(plain(POP_TOP)),
                Item::Label(Label(7)), // Erased outer normal POP_BLOCK, compact position 10.
            ];
            if intervening_call {
                // The earlier POP vacancy moves to the call, leaving the inner SETUP unchanged.
                let mut receiver = plain(LOAD_FAST);
                receiver.local_policy.add_hidden_exception_owner_slots(1);
                items.extend([
                    Item::Instruction(receiver),
                    Item::Instruction(plain(CALL)),
                    Item::Instruction(plain(POP_TOP)),
                ]);
            } else {
                inner.local_policy.add_hidden_exception_owner_slots(1);
            }
            items.extend([
                Item::Label(Label(8)),
                Item::Instruction(inner), // Target source 10; control source 13.
                Item::Label(Label(9)),
                Item::Label(Label(10)),
                Item::Instruction(plain(LOAD_FAST)),
                Item::Instruction(condition),
                Item::Label(Label(11)), // A bookkeeping marker alias is not a new original array.
                Item::Instruction(marker), // Target m=13; control m=16; both select m-3.
                Item::Label(Label(12)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Instruction(plain(RETURN_VALUE)),
                Item::Label(Label(13)),
                Item::Label(Label(14)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Instruction(plain(RETURN_VALUE)),
            ]);
            LayoutAssembler {
                items,
                exception_regions: vec![ExceptionRegion {
                    is_scope_fallback: true,
                    priority: ExceptionRegionPriority::OriginalRange(Label(1), Label(13)),
                    start: Label(1),
                    end: Label(13),
                    target: Label(14),
                    depth: 0,
                    preserve_lasti: false,
                }],
                exception_owner_provenance: ExceptionOwnerProvenance {
                    setups: vec![
                        (Label(1), None),
                        (Label(2), None),
                        (Label(3), None),
                        (Label(6), Some(outer_location)),
                        (Label(10), Some(inner_location)),
                    ],
                    block_boundaries: [Label(0)].into_iter().collect(),
                    source_boundaries: [Label(0)].into_iter().collect(),
                    synchronous_with_setups: [Label(2), Label(3)].into_iter().collect(),
                    setup_nop_boundaries: [Label(4), Label(5), Label(8), Label(9)]
                        .into_iter()
                        .collect(),
                    setup_nop_ranges: vec![(Label(4), Label(5)), (Label(8), Label(9))],
                    removed_pop_block_boundaries: [Label(7)].into_iter().collect(),
                    removed_pop_block_widths: [(Label(7), 1)].into_iter().collect(),
                    ..ExceptionOwnerProvenance::default()
                },
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 15,
                has_next_location: false,
            }
        }

        fn recognizes(assembler: &LayoutAssembler) -> bool {
            let mut instructions = Vec::new();
            let mut positions = rustc_hash::FxHashMap::default();
            for item in &assembler.items {
                match item {
                    Item::Label(label) => {
                        positions.insert(*label, instructions.len());
                    }
                    Item::Instruction(instruction) => instructions.push(instruction),
                }
            }
            assembler.entry_retained_setup_null_marker(
                positions[&Label(11)],
                &instructions,
                &positions,
                &[Label(12)].into_iter().collect(),
                &[2, 5, 7],
            )
        }

        #[test]
        fn last_setup_compaction_selects_the_retained_null_record() {
            assert!(
                recognizes(&fixture(false)),
                "POP transferred to the retained SETUP"
            );
            assert!(
                recognizes(&fixture(true)),
                "POP transferred to an intervening call"
            );
        }

        #[test]
        fn last_setup_compaction_rejects_unproven_records_and_array_boundaries() {
            let rejects = |name: &str, change: fn(&mut LayoutAssembler)| {
                let mut assembler = fixture(false);
                change(&mut assembler);
                assert!(!recognizes(&assembler), "{name}");
            };
            rejects("changed retained range", |assembler| {
                assembler.exception_owner_provenance.setup_nop_ranges[1].1 = Label(11);
            });
            rejects("uncertified hidden POP", |assembler| {
                assembler
                    .exception_owner_provenance
                    .removed_pop_block_boundaries
                    .clear();
            });
            rejects("real original join", |assembler| {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(Label(7));
            });
            rejects("unknown removed SETUP", |assembler| {
                assembler
                    .exception_owner_provenance
                    .synchronous_with_setups
                    .remove(&Label(3));
            });
            rejects("earlier condition marker", |assembler| {
                let instruction = assembler
                    .items
                    .iter_mut()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })
                    .nth(6)
                    .unwrap();
                instruction.opcode = NOT_TAKEN;
            });
            rejects("copied original array", |assembler| {
                assembler
                    .exception_owner_provenance
                    .normal_try_copy_blocks
                    .insert(Label(7));
            });
            rejects("borrow barrier", |assembler| {
                let instruction = assembler
                    .items
                    .iter_mut()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })
                    .nth(3)
                    .unwrap();
                instruction
                    .no_location_policy
                    .set_borrow_fallthrough(BorrowFallthrough::Barrier);
            });
        }
    }
}
