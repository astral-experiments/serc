use std::cell::OnceCell;
use std::cmp::Reverse;

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::block_graph::BasicBlockId;
use super::{
    Instruction, Item, Label, LayoutAssembler, NormalizedExceptionOwner, Operand, SourceLocation,
};
use crate::target::opcodes::{
    COMPARE_OP, COPY_FREE_VARS, FOR_ITER, LOAD_CONST, LOAD_FAST_BORROW_LOAD_FAST_BORROW,
    LOAD_FAST_LOAD_FAST, LOAD_GLOBAL, MAKE_CELL, NOP, NOT_TAKEN, POP_JUMP_IF_NONE,
    POP_JUMP_IF_NOT_NONE, POP_TOP, RETURN_GENERATOR, SEND, STORE_FAST_LOAD_FAST,
    STORE_FAST_STORE_FAST,
};
use crate::target::{is_conditional_jump, is_scope_exit};

#[cfg(test)]
thread_local! {
    static FORCE_COMPLETE_OWNER_REPLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FORCE_ALL_OWNER_DECISIONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static VERIFY_COMPLETE_OWNER_REPLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Original labels for a Python 3.12 async-manager protocol await.
#[derive(Debug)]
pub(super) struct LegacyProtocolAwaitSetup {
    pub(super) yielded: Label,
    pub(super) yielded_end: Label,
    pub(super) cleanup: Label,
    pub(super) send: Label,
    pub(super) exit: Label,
}

/// Original CFG facts that cannot be recovered from split exception ranges or optimized jumps.
#[derive(Debug, Default)]
pub(super) struct ExceptionOwnerProvenance {
    // The first folded condition can share the module annotation setup's source line.
    pub(super) module_annotation_nop_pairs: Vec<(Instruction, Instruction)>,
    // The same setup can precede a folded negated condition's original jump.
    pub(super) module_annotation_jump_pairs: Vec<(Instruction, Instruction)>,
    // Original block, setup and jump-derived NOP after an actual redundant-jump conversion.
    pub(super) module_annotation_jump_nops: Vec<(BasicBlockId, Instruction, Instruction)>,
    pub(super) setups: Vec<(Label, Option<SourceLocation>)>,
    // Original 3.12 SETUP entries and targets, captured before region splitting.
    pub(super) legacy_native_setup_targets: Vec<(Label, Label)>,
    // Protocol awaits do not participate in ordinary-await owner propagation.
    pub(super) legacy_protocol_await_setups: Vec<LegacyProtocolAwaitSetup>,
    // Authored ordinary-try SETUP and its original handler, independently of split ranges.
    pub(super) ordinary_try_setups: FxHashMap<Label, Label>,
    // Authored name-clearing exception handlers, independent of overlapping outer regions.
    pub(super) named_handler_cleanup_targets: FxHashSet<Label>,
    // Original ordinary-await suspension regions, resolved alongside their live regions.
    pub(super) ordinary_await_cleanups: Vec<super::ExceptionRegion>,
    // Authored SEND, completion entry, and END_SEND before any cleanup or copying.
    pub(super) ordinary_await_ends: Vec<OrdinaryAwaitEnd>,
    // Empty runtime-while anchor, shared exit, and exact terminal authored break.
    pub(super) empty_while_break_anchors: Vec<(Label, Label, Instruction)>,
    // Authored return NOP, immediately following finally POP, and original try target.
    pub(super) early_return_nop_pairs: Vec<(Instruction, Instruction, Label)>,
    // Authored return NOP, iterator disposal, finally POP, and original try target.
    pub(super) early_return_iterator_pops: Vec<(Instruction, Instruction, Instruction, Label)>,
    pub(super) copied_handler_cleanups: Vec<CopiedHandlerCleanup>,
    pub(super) named_handler_pop_pairs: Vec<OriginalPopBlockSpan>,
    pub(super) named_handler_try_prefixes: Vec<OriginalPopBlockSpan>,
    pub(super) continued_handler_try_prefixes: Vec<OriginalPopBlockSpan>,
    pub(super) copied_handler_finally_pops: Vec<OriginalPopBlockSpan>,
    pub(super) copied_method_cleanup_pop_spans: Vec<OriginalPopBlockSpan>,
    pub(super) iterator_return_pop_spans: Vec<(Instruction, OriginalPopBlockSpan)>,
    pub(super) fused_manager_owners: Vec<(Instruction, Instruction, super::ExceptionRegion)>,
    pub(super) handler_dispatch_arrays: Vec<HandlerDispatchArray>,
    // Matched named-handler entry through its first primitive truth condition.
    pub(super) named_handler_truth_prefixes: Vec<NamedHandlerTruthPrefix>,
    pub(super) block_boundaries: FxHashSet<Label>,
    // Original entries that split borrowed-load analysis without changing the physical CFG.
    pub(super) borrow_only_block_boundaries: FxHashSet<Label>,
    pub(super) source_boundaries: FxHashSet<Label>,
    pub(super) continuation_boundaries: FxHashSet<Label>,
    // Synthetic range starts before value-preserving boolean jumps.
    pub(super) value_jump_continuation_boundaries: FxHashSet<Label>,
    // Complete authored COPY/TO_BOOL/condition/marker records and intervening labels.
    pub(super) original_value_jump_prefixes: Vec<(Label, Vec<Item>)>,
    // Complete source arrays containing a retained SETUP before a value Boolean.
    pub(super) retained_setup_value_arrays: Vec<RetainedSetupValueArray>,
    // Region re-entry after a return emits no original instruction or unwind slot.
    pub(super) return_region_reentries: FxHashSet<Label>,
    pub(super) loop_finally_region_boundaries: FxHashSet<Label>,
    // Proven CALL/POP or unique conditional-join donor and its exact disposal array.
    pub(super) copied_finally_iterator_returns: Vec<(SourceLocation, Vec<Item>)>,
    // Fresh authored break array through the first truth marker, with two omitted POPs.
    pub(super) copied_conjunction_break_prefixes: Vec<Vec<Item>>,
    // Named break: two handler POPs and the following finalizer POP are omitted.
    pub(super) copied_named_break_truth_prefixes: Vec<CopiedNamedBreakTruthPrefix>,
    // Ordinary break, ended try/finally ranges and copied first truth array.
    pub(super) copied_shared_break_truth_prefixes: Vec<Vec<Item>>,
    pub(super) fusion_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_boundaries: FxHashSet<Label>,
    pub(super) setup_nop_ranges: Vec<(Label, Label)>,
    // Retained SETUPs emitted directly into an authored primitive conditional fallthrough.
    pub(super) original_fallthrough_setups: FxHashSet<Label>,
    // One authored normal POP_BLOCK, not an extra synthetic end record or a copied exit.
    pub(super) retained_normal_pop_blocks: FxHashMap<Label, RetainedNormalPopBlock>,
    pub(super) original_normal_finally_arrays: Vec<OriginalNormalFinallyArray>,
    pub(super) normal_finally_none_prefixes: Vec<(Instruction, Instruction, Instruction)>,
    pub(super) normal_finally_method_none_prefixes: Vec<[Instruction; 5]>,
    pub(super) normal_finally_truth_prefixes: Vec<[Instruction; 4]>,
    pub(super) copied_return_none_prefixes: Vec<CopiedReturnNonePrefix>,
    pub(super) copied_break_none_nops: Vec<(Vec<Label>, [Instruction; 4])>,
    pub(super) copied_bare_handler_none_prefixes: Vec<(Label, Label, [Instruction; 10])>,
    pub(super) copied_iterator_attribute_none_prefixes: Vec<[Instruction; 4]>,
    pub(super) copied_cleanup_return_none_prefixes: Vec<CopiedCleanupReturnNonePrefix>,
    pub(super) copied_return_pop_prefixes: Vec<CopiedReturnPopPrefix>,
    pub(super) copied_return_truth_prefixes: Vec<CopiedReturnTruthPrefix>,
    pub(super) copied_return_membership_prefixes: Vec<CopiedReturnMembershipPrefix>,
    pub(super) copied_return_call_prefixes: Vec<CopiedReturnCallPrefix>,
    pub(super) copied_return_attribute_truth_prefixes: Vec<CopiedReturnAttributeTruthPrefix>,
    pub(super) copied_handler_return_none_prefixes: Vec<(Label, [Instruction; 6])>,
    pub(super) saved_named_handler_none_prefixes: Vec<SavedNamedHandlerNonePrefix>,
    pub(super) copied_manager_return_none_prefixes: Vec<CopiedManagerReturnNonePrefix>,
    pub(super) retained_manager_return_none_prefixes: Vec<RetainedManagerReturnNonePrefix>,
    pub(super) copied_manager_return_compactions: Vec<CopiedManagerReturnCompaction>,
    pub(super) uncompacted_conditional_arrays: Vec<UncompactedConditionalArray>,
    pub(super) original_primitive_condition_boundaries: Vec<Instruction>,
    // Authored if-tests that begin by directly evaluating this name.
    pub(super) original_initial_name_if_locations: FxHashSet<SourceLocation>,
    // Authored while-tests that are a direct name.
    pub(super) original_name_while_locations: FxHashSet<SourceLocation>,
    // Exact comparison failure/fallthrough/cleanup records; no synthetic head label.
    pub(super) original_comparison_fallthroughs: Vec<(Label, Label, [Instruction; 6])>,
    // Source-authored match ends establish original arrays independently of case exits.
    pub(super) original_match_join_boundaries: FxHashSet<Label>,
    // Original leaf-block entry and exit, case-success join, and following return array.
    pub(super) nested_match_exit_copy_origins: Vec<(Label, Instruction, Label, Label)>,
    // Exact post-threading head and original fallthrough donor of a shared case return.
    pub(super) shared_match_return_donors: FxHashMap<Label, (Instruction, Instruction)>,
    // Authored normal exits and their immediate, fully emitted restore records.
    pub(super) original_comprehension_normal_exits: Vec<(Label, Instruction)>,
    pub(super) original_comprehension_normal_restores: FxHashMap<Label, Vec<Instruction>>,
    // Exact NOPs emitted for authored Pass statements, before graph rewrites.
    pub(super) original_pass_nops: Vec<Instruction>,
    // Original simple For target stores, before their tracing NOP is compacted.
    pub(super) for_target_owner_prefixes: Vec<(Label, Instruction)>,
    // Exact final normal restore, after its fusion policies are complete.
    pub(super) comprehension_restore_owner_prefixes: Vec<(Label, Instruction)>,
    pub(super) retired_comprehension_normal_entries: FxHashSet<Label>,
    pub(super) synchronous_with_setups: FxHashSet<Label>,
    pub(super) async_with_setups: Vec<AsyncWithSetup>,
    pub(super) removed_pop_block_boundaries: FxHashSet<Label>,
    pub(super) removed_pop_block_widths: FxHashMap<Label, u16>,
    // Exact empty normal-finalizer jumps whose range ends are synthetic manager holes.
    pub(super) synthetic_manager_join_exits: Vec<(Label, Instruction)>,
    // Original ordinary-try exits contain POP_BLOCK followed by JUMP_NO_INTERRUPT.
    pub(super) normal_try_exits: Vec<(Label, Label)>,
    // Sole authored break, retired ordinary-try bridge, and intact normal-finally array.
    pub(super) normal_try_break_pop_owners: Vec<NormalTryBreakPopOwner>,
    // Exact final wildcard call/discard or class-pattern failure pops at a normal try exit.
    pub(super) normal_try_fallthrough_donors: FxHashMap<Label, (Instruction, Instruction)>,
    // Exact final wildcard CALL/POP before a compound try's original normal exit.
    pub(super) compound_try_fallthrough_donors: FxHashMap<Label, (Instruction, Instruction)>,
    // Original adjacent arrays containing the two final class-pattern failure pops.
    pub(super) normal_try_class_pattern_donor_labels: FxHashMap<Label, (Label, Label)>,
    // First records of the original manager cleanup arrays, before line resolution.
    pub(super) original_manager_pop_blocks: Vec<Label>,
    // Exact located POP records at certified original sole-fallthrough joins.
    pub(super) original_fallthrough_pop_blocks: FxHashMap<Label, Instruction>,
    // Ordinary handler exits copied from an original unlocated POP_EXCEPT/jump array.
    pub(super) original_handler_cleanup_join_exits: FxHashSet<Label>,
    // Source-emitted named cleanup arrays, before original POP_BLOCK restoration.
    pub(super) original_named_handler_cleanup_joins: Vec<(Label, [Instruction; 5])>,
    // Original try/except POP_BLOCK exits inside their own finally, for owner transfer
    // and late redundant-exit removal, but not ordinary-try exit copying.
    pub(super) compound_try_owner_exits: Vec<(Label, Label)>,
    // Exact original direct exits whose late retained NOP represents an ownerless POP_BLOCK.
    pub(super) ownerless_compound_try_nops: Vec<(Label, Label, Instruction)>,
    // Sole original guard-false try-else edges; absorption by a later POP_BLOCK only.
    pub(super) guarded_try_else_exits: Vec<(Label, Label)>,
    // Populated authored else arrays with direct normal edges into their own finally.
    pub(super) populated_try_else_borrow_exits: Vec<(Label, Label)>,
    pub(super) omitted_nop_locations: FxHashMap<Label, SourceLocation>,
    pub(super) omitted_nop_counts: FxHashMap<Label, usize>,
    // Native early-copy slots from eager footer jumps and copied omitted records.
    pub(super) legacy_early_copy_slots: FxHashMap<Label, usize>,
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
    // Exact survivors of uniformly owned removed records, including saturated literal folds.
    pub(super) original_uniform_folds: Vec<Instruction>,
    // Exact source comparison and original labels, before any jump copying or cleanup.
    pub(super) source_none_conditions: Vec<SourceNoneCondition>,
    // Exact terminal pairs produced by original conditional jump threading.
    pub(super) threaded_condition_origins: Vec<(Instruction, Instruction)>,
}

impl ExceptionOwnerProvenance {
    /// Validates fields whose producer contract requires an authored jump.
    pub(super) fn legacy_seed_capture_shapes_valid(&self) -> bool {
        let forward =
            |instruction: &Instruction| matches!(instruction.operand(), Operand::Forward(_));
        if !self
            .ordinary_await_ends
            .iter()
            .map(|origin| &origin.send)
            .chain(self.async_with_setups.iter().map(|origin| &origin.send))
            .all(forward)
            || !self
                .original_primitive_condition_boundaries
                .iter()
                .chain(
                    self.source_none_conditions
                        .iter()
                        .map(|origin| &origin.instruction),
                )
                .chain(
                    self.original_comprehension_normal_exits
                        .iter()
                        .map(|(_, instruction)| instruction),
                )
                .all(|instruction| {
                    matches!(
                        instruction.operand(),
                        Operand::Forward(_) | Operand::Backward(_)
                    )
                })
        {
            return false;
        }
        if !self.named_handler_truth_prefixes.iter().all(|origin| {
            origin.items.iter().all(|item| match item {
                Item::Instruction(instruction) => matches!(
                    instruction.operand(),
                    Operand::Value(_) | Operand::Forward(_)
                ),
                Item::Label(_) => true,
            })
        }) {
            return false;
        }
        if !self.original_normal_finally_arrays.iter().all(|origin| {
            origin.incoming.iter().all(|instruction| {
                matches!(instruction.operand(), Operand::Forward(target) if target == origin.entry)
            })
        }) {
            return false;
        }
        self.normal_finally_none_prefixes
            .iter()
            .map(|origin| &origin.2)
            .chain(
                self.normal_finally_method_none_prefixes
                    .iter()
                    .map(|prefix| &prefix[4]),
            )
            .chain(
                self.normal_finally_truth_prefixes
                    .iter()
                    .map(|prefix| &prefix[3]),
            )
            .chain(
                self.copied_return_none_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_cleanup_return_none_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_return_truth_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_return_membership_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_return_call_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_return_attribute_truth_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_manager_return_none_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.retained_manager_return_none_prefixes
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_manager_return_compactions
                    .iter()
                    .map(|origin| &origin.condition),
            )
            .chain(
                self.copied_named_break_truth_prefixes
                    .iter()
                    .map(|origin| &origin.original_condition),
            )
            .chain(
                self.copied_handler_return_none_prefixes
                    .iter()
                    .map(|origin| &origin.1[5]),
            )
            .chain(
                self.copied_bare_handler_none_prefixes
                    .iter()
                    .map(|origin| &origin.2[8]),
            )
            .chain(
                self.copied_iterator_attribute_none_prefixes
                    .iter()
                    .map(|prefix| &prefix[3]),
            )
            .all(forward)
    }

    /// Visits full instruction witnesses for exact native seeds and cancellation.
    ///
    /// Visit every record when clearing: a seeded jump may have become a NOP.
    /// Seeding filters jump operands separately and validates all targets first.
    /// Each record is resolved or cleared independently; callbacks do not mutate
    /// map keys, membership, graph targets, or the native-label lookup.
    #[expect(
        clippy::iter_over_hash_type,
        reason = "capture callbacks resolve or clear each record independently of map order"
    )]
    pub(super) fn visit_legacy_seed_captures_mut(
        &mut self,
        mut visit: impl FnMut(&mut Instruction),
    ) {
        for origin in &mut self.ordinary_await_ends {
            visit(&mut origin.send);
            visit(&mut origin.end_send);
            for item in &mut origin.cleanup {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
            if let Some(record) = origin.bridge.as_mut() {
                visit(&mut record.1);
            }
        }
        for record in &mut self.empty_while_break_anchors {
            visit(&mut record.2);
        }
        for record in &mut self.early_return_nop_pairs {
            visit(&mut record.0);
            visit(&mut record.1);
        }
        for record in &mut self.early_return_iterator_pops {
            visit(&mut record.0);
            visit(&mut record.1);
            visit(&mut record.2);
        }
        for origin in &mut self.copied_handler_cleanups {
            visit(&mut origin.return_nop);
            if let Some(instruction) = origin.return_iterator.as_mut() {
                visit(instruction);
            }
            for instruction in &mut origin.instructions {
                visit(instruction);
            }
            if let Some(record) = origin.assignment_pop.as_mut() {
                visit(&mut record.1);
            }
            if let Some(record) = origin.continued_call.as_mut() {
                for instruction in &mut record.0 {
                    visit(instruction);
                }
                visit(&mut record.1);
            }
            if let Some(record) = origin.continued_attribute_none.as_mut() {
                for instruction in &mut record.0 {
                    visit(instruction);
                }
                visit(&mut record.1);
            }
        }
        for origin in &mut self.named_handler_pop_pairs {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in &mut self.named_handler_try_prefixes {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in &mut self.continued_handler_try_prefixes {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in self
            .copied_handler_finally_pops
            .iter_mut()
            .chain(&mut self.copied_method_cleanup_pop_spans)
        {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for record in &mut self.iterator_return_pop_spans {
            visit(&mut record.0);
            for item in &mut record.1.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for record in &mut self.fused_manager_owners {
            visit(&mut record.0);
            visit(&mut record.1);
        }
        for origin in &mut self.named_handler_truth_prefixes {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in &mut self.handler_dispatch_arrays {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for record in &mut self.original_value_jump_prefixes {
            for item in &mut record.1 {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in &mut self.retained_setup_value_arrays {
            for item in &mut origin.prefix {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for record in &mut self.copied_finally_iterator_returns {
            for item in &mut record.1 {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for prefix in &mut self.copied_conjunction_break_prefixes {
            for item in prefix.iter_mut() {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in &mut self.copied_named_break_truth_prefixes {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
            visit(&mut origin.original_condition);
        }
        for prefix in &mut self.copied_shared_break_truth_prefixes {
            for item in prefix.iter_mut() {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in &mut self.original_normal_finally_arrays {
            for item in &mut origin.prefix {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
            for instruction in &mut origin.incoming {
                visit(instruction);
            }
            if let Some(record) = origin.borrow_join.as_mut() {
                visit(&mut record.1);
            }
        }
        for record in &mut self.normal_finally_none_prefixes {
            visit(&mut record.0);
            visit(&mut record.1);
            visit(&mut record.2);
        }
        for prefix in &mut self.normal_finally_method_none_prefixes {
            for instruction in prefix.iter_mut() {
                visit(instruction);
            }
        }
        for prefix in &mut self.normal_finally_truth_prefixes {
            for instruction in prefix.iter_mut() {
                visit(instruction);
            }
        }
        for origin in &mut self.copied_return_none_prefixes {
            if let Some(instruction) = origin.return_nop.as_mut() {
                visit(instruction);
            }
            for instruction in &mut origin.pops {
                visit(instruction);
            }
            if let Some(instruction) = origin.additional_pop.as_mut() {
                visit(instruction);
            }
            visit(&mut origin.load);
            visit(&mut origin.condition);
        }
        for record in &mut self.copied_break_none_nops {
            for instruction in &mut record.1 {
                visit(instruction);
            }
        }
        for record in &mut self.copied_bare_handler_none_prefixes {
            for instruction in &mut record.2 {
                visit(instruction);
            }
        }
        for prefix in &mut self.copied_iterator_attribute_none_prefixes {
            for instruction in prefix.iter_mut() {
                visit(instruction);
            }
        }
        for origin in &mut self.copied_cleanup_return_none_prefixes {
            for instruction in &mut origin.cleanup {
                visit(instruction);
            }
            visit(&mut origin.pop);
            visit(&mut origin.load);
            visit(&mut origin.condition);
            visit(&mut origin.marker);
        }
        for origin in &mut self.copied_return_pop_prefixes {
            for instruction in &mut origin.records {
                visit(instruction);
            }
        }
        for origin in &mut self.copied_return_truth_prefixes {
            visit(&mut origin.return_nop);
            for instruction in &mut origin.pops {
                visit(instruction);
            }
            visit(&mut origin.load);
            visit(&mut origin.to_bool);
            visit(&mut origin.condition);
        }
        for origin in &mut self.copied_return_membership_prefixes {
            visit(&mut origin.return_value);
            for instruction in &mut origin.pops {
                visit(instruction);
            }
            visit(&mut origin.load);
            visit(&mut origin.constant);
            visit(&mut origin.contains);
            visit(&mut origin.condition);
            visit(&mut origin.marker);
        }
        for origin in &mut self.copied_return_call_prefixes {
            visit(&mut origin.return_nop);
            for instruction in &mut origin.pops {
                visit(instruction);
            }
            for instruction in &mut origin.prefix {
                visit(instruction);
            }
            visit(&mut origin.condition);
        }
        for origin in &mut self.copied_return_attribute_truth_prefixes {
            visit(&mut origin.return_nop);
            for instruction in &mut origin.pops {
                visit(instruction);
            }
            for instruction in &mut origin.prefix {
                visit(instruction);
            }
            visit(&mut origin.condition);
        }
        for record in &mut self.copied_handler_return_none_prefixes {
            for instruction in &mut record.1 {
                visit(instruction);
            }
        }
        for origin in &mut self.saved_named_handler_none_prefixes {
            for item in &mut origin.items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for origin in &mut self.copied_manager_return_none_prefixes {
            for instruction in &mut origin.prefix {
                visit(instruction);
            }
            visit(&mut origin.pop);
            visit(&mut origin.load);
            visit(&mut origin.condition);
        }
        for origin in &mut self.retained_manager_return_none_prefixes {
            visit(&mut origin.entry);
            for instruction in &mut origin.prefix {
                visit(instruction);
            }
            visit(&mut origin.pop);
            visit(&mut origin.load);
            visit(&mut origin.condition);
            visit(&mut origin.marker);
        }
        for origin in &mut self.copied_manager_return_compactions {
            for instruction in &mut origin.cleanup {
                visit(instruction);
            }
            if let Some(prefix) = origin.earlier_cleanup.as_mut() {
                for instruction in prefix.iter_mut() {
                    visit(instruction);
                }
            }
            if let Some(prefix) = origin.named_handler_cleanup.as_mut() {
                for instruction in prefix.iter_mut() {
                    visit(instruction);
                }
            }
            if let Some(origin) = origin.constant_return_prefix.as_mut() {
                visit(&mut origin.authored_nop);
                if let Some(instruction) = origin.retained_inner_try_pop.as_mut() {
                    visit(instruction);
                }
                if let Some(instruction) = origin.handler_pop.as_mut() {
                    visit(instruction);
                }
                for instruction in &mut origin.instructions {
                    visit(instruction);
                }
            }
            if let Some(instruction) = origin.to_bool.as_mut() {
                visit(instruction);
            }
            visit(&mut origin.pop);
            visit(&mut origin.load);
            visit(&mut origin.condition);
        }
        for origin in &mut self.uncompacted_conditional_arrays {
            for item in &mut origin.prefix {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for instruction in &mut self.original_pass_nops {
            visit(instruction);
        }
        for instruction in &mut self.original_primitive_condition_boundaries {
            visit(instruction);
        }
        for (_, _, records) in &mut self.original_comparison_fallthroughs {
            for instruction in records {
                visit(instruction);
            }
        }
        for record in self.shared_match_return_donors.values_mut() {
            visit(&mut record.0);
            visit(&mut record.1);
        }
        for record in &mut self.original_comprehension_normal_exits {
            visit(&mut record.1);
        }
        for prefix in self.original_comprehension_normal_restores.values_mut() {
            for instruction in prefix.iter_mut() {
                visit(instruction);
            }
        }
        for record in &mut self.for_target_owner_prefixes {
            visit(&mut record.1);
        }
        for record in &mut self.comprehension_restore_owner_prefixes {
            visit(&mut record.1);
        }
        for origin in &mut self.async_with_setups {
            visit(&mut origin.send);
            for item in &mut origin.prefix {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
            for item in &mut origin.cleanup {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
            if let Some(record) = origin.bridge.as_mut() {
                visit(&mut record.1);
            }
        }
        for record in &mut self.synthetic_manager_join_exits {
            visit(&mut record.1);
        }
        for origin in &mut self.normal_try_break_pop_owners {
            for item in &mut origin.prefix {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
        for record in self.normal_try_fallthrough_donors.values_mut() {
            visit(&mut record.0);
            visit(&mut record.1);
        }
        for record in self.compound_try_fallthrough_donors.values_mut() {
            visit(&mut record.0);
            visit(&mut record.1);
        }
        for instruction in self.original_fallthrough_pop_blocks.values_mut() {
            visit(instruction);
        }
        for record in &mut self.ownerless_compound_try_nops {
            visit(&mut record.2);
        }
        for origin in self.normal_try_copy_join_origins.values_mut() {
            visit(&mut origin.store);
            visit(&mut origin.jump);
        }
        for record in self.normal_try_copy_join_donors.values_mut() {
            visit(&mut record.0);
        }
        for instruction in &mut self.original_uniform_folds {
            visit(instruction);
        }
        for origin in &mut self.source_none_conditions {
            visit(&mut origin.instruction);
            if let Some(prefix) = origin.entry_prefix.as_mut() {
                for item in prefix.iter_mut() {
                    if let Item::Instruction(instruction) = item {
                        visit(instruction);
                    }
                }
            }
        }
        for record in &mut self.threaded_condition_origins {
            visit(&mut record.0);
            visit(&mut record.1);
        }
    }
}

/// Original try-exit slot absorbed by a normal-finally marker after jump threading.
#[derive(Debug)]
pub(super) struct NormalTryBreakPopOwner {
    pub(super) source_labels: Vec<Label>,
    pub(super) handler: Label,
    pub(super) entry: Label,
    pub(super) prefix: Vec<Item>,
    pub(super) exit: Label,
    pub(super) location: SourceLocation,
    pub(super) region: super::ExceptionRegion,
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

/// Original return-unwind POP records followed by a copied-finally local None test.
#[derive(Debug)]
pub(super) struct CopiedReturnNonePrefix {
    pub(super) return_nop: Option<Instruction>,
    pub(super) pops: [Instruction; 2],
    // A second ordinary try unwinds between the first POP and the finally POP.
    pub(super) additional_pop: Option<Instruction>,
    pub(super) load: Instruction,
    pub(super) condition: Instruction,
}

/// Exact emitted iterator/manager cleanup before a copied local None finalizer.
#[derive(Debug)]
pub(super) struct CopiedCleanupReturnNonePrefix {
    pub(super) entry: Label,
    pub(super) cleanup: Vec<Instruction>,
    pub(super) manager: bool,
    pub(super) handler: bool,
    pub(super) pop: Instruction,
    pub(super) load: Instruction,
    pub(super) condition: Instruction,
    pub(super) marker: Instruction,
}

/// Two adjacent NULL return POPs and their unchanged first cleanup record.
#[derive(Debug)]
pub(super) struct CopiedReturnPopPrefix {
    pub(super) entry: Label,
    pub(super) records: [Instruction; 3],
}

/// A constant return's original source entry and global-attribute truth call.
#[derive(Debug)]
pub(super) struct CopiedReturnCallPrefix {
    pub(super) entry: Label,
    pub(super) return_nop: Instruction,
    pub(super) pops: [Instruction; 2],
    pub(super) prefix: [Instruction; 5],
    pub(super) condition: Instruction,
}

/// A constant return's original source entry and local attribute truth test.
#[derive(Debug)]
pub(super) struct CopiedReturnAttributeTruthPrefix {
    pub(super) entry: Label,
    pub(super) return_nop: Instruction,
    pub(super) pops: [Instruction; 2],
    pub(super) prefix: [Instruction; 3],
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

/// A direct-name return's two original NULL POPs and source-literal membership test.
#[derive(Debug)]
pub(super) struct CopiedReturnMembershipPrefix {
    pub(super) entry: Label,
    pub(super) return_value: Instruction,
    pub(super) cleanup_labels: Vec<Label>,
    pub(super) pops: [Instruction; 2],
    pub(super) load: Instruction,
    pub(super) constant: Instruction,
    pub(super) contains: Instruction,
    pub(super) condition: Instruction,
    pub(super) marker: Instruction,
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

/// An unchanged conditional return array whose outer manager survives the copied finally.
#[derive(Debug)]
pub(super) struct RetainedManagerReturnNonePrefix {
    pub(super) entry: Instruction,
    pub(super) prefix: Vec<Instruction>,
    pub(super) return_offset: usize,
    pub(super) outer_handler: Label,
    pub(super) pop: Instruction,
    pub(super) load: Instruction,
    pub(super) condition: Instruction,
    pub(super) marker: Instruction,
}

/// Exact manager cleanup and first copied-finally test, before either compaction pass.
#[derive(Debug)]
pub(super) struct CopiedManagerReturnCompaction {
    pub(super) ordinary_try_pops: usize,
    /// A still-live manager can be replaced only by a captured original cleanup cell.
    pub(super) retained_outer_handler: Option<Label>,
    pub(super) cleanup: Vec<Instruction>,
    pub(super) earlier_cleanup: Option<[Instruction; 7]>,
    /// `SWAP`, `POP_EXCEPT`, and name clearing after two omitted named-handler POPs.
    pub(super) named_handler_cleanup: Option<[Instruction; 5]>,
    pub(super) constant_return_prefix: Option<ConstantManagerReturnPrefix>,
    pub(super) to_bool: Option<Instruction>,
    pub(super) pop: Instruction,
    pub(super) load: Instruction,
    pub(super) condition: Instruction,
}

/// Exact non-preserving return cleanup before its manager's omitted POP.
#[derive(Debug)]
pub(super) struct ConstantManagerReturnPrefix {
    pub(super) authored_nop: Instruction,
    pub(super) retained_authored: bool,
    // Adjacent original ordinary-try POP that replaces the authored NOP before a manager.
    pub(super) retained_inner_try_pop: Option<Instruction>,
    pub(super) handler_pop: Option<Instruction>,
    pub(super) instructions: Vec<Instruction>,
    pub(super) missing_null_pops: usize,
}

/// An original saved-value return array with two omitted named-handler POP cells.
#[derive(Debug)]
pub(super) struct SavedNamedHandlerNonePrefix {
    pub(super) entry: Label,
    pub(super) items: Vec<Item>,
    pub(super) unwind_offset: usize,
}

/// A captured named-handler break, retaining its original condition across exact retargets.
#[derive(Debug)]
pub(super) struct CopiedNamedBreakTruthPrefix {
    pub(super) entry: Label,
    pub(super) items: Vec<Item>,
    pub(super) original_condition: Instruction,
}

/// Original constant-return cleanup, including NULL records omitted under a manager.
#[derive(Debug)]
pub(super) struct CopiedHandlerCleanup {
    pub(super) start: Label,
    pub(super) end: Label,
    pub(super) return_nop: Instruction,
    pub(super) return_iterator: Option<Instruction>,
    pub(super) instructions: Vec<Instruction>,
    pub(super) missing_null_pops: usize,
    pub(super) removed_prefix_slots: u16,
    // Exact inner-finally POP removed immediately before the captured local assignment.
    pub(super) assignment_pop: Option<(Label, Instruction)>,
    pub(super) continued_call: Option<([Instruction; 6], Instruction)>,
    // Direct named-handler continue: the original finalizer POP follows name cleanup.
    pub(super) continued_attribute_none: Option<([Instruction; 3], Instruction)>,
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

/// The original fallthrough array containing one named SETUP and its first truth test.
#[derive(Debug)]
pub(super) struct NamedHandlerTruthPrefix {
    pub(super) entry: Label,
    pub(super) setup: Label,
    pub(super) target: Label,
    pub(super) items: Vec<Item>,
}

/// The original normal-finally prefix and the jumps that entered its fresh array.
#[derive(Debug)]
pub(super) struct OriginalNormalFinallyArray {
    pub(super) entry: Label,
    pub(super) prefix: Vec<Item>,
    pub(super) incoming: Vec<Instruction>,
    pub(super) retained_setup: Option<(Label, Label, Label)>,
    // Captured before the normal-finally exit is emitted: this join already has code.
    pub(super) borrow_join: Option<(Label, Instruction)>,
}

/// Whether a slot is protected, with its exact source region when known.
#[derive(Clone, Copy, Debug)]
pub(super) enum ExceptionOwner {
    Absent,
    /// An owner exists, but layout must infer its source region.
    Present,
    // An earlier original-array record can have a different owner from the condition.
    SourceRegion(usize),
}

/// The strength of an ownership result reconstructed from original-array provenance.
#[derive(Clone, Copy, Debug)]
pub(super) enum ExceptionOwnerDecision {
    Tentative(ExceptionOwner),
    Definitive(ExceptionOwner),
    Override(ExceptionOwner),
}

impl ExceptionOwnerDecision {
    /// Replaces the inferred owner with an exact source region or an unowned slot.
    fn override_with_region(source_region: Option<usize>) -> Self {
        Self::Override(source_region.map_or(ExceptionOwner::Absent, ExceptionOwner::SourceRegion))
    }

    fn owner(self) -> ExceptionOwner {
        match self {
            Self::Tentative(owner) | Self::Definitive(owner) | Self::Override(owner) => owner,
        }
    }

    pub(super) fn has_owner(self) -> bool {
        !matches!(self.owner(), ExceptionOwner::Absent)
    }

    pub(super) fn source_region(self) -> Option<usize> {
        match self.owner() {
            ExceptionOwner::SourceRegion(region) => Some(region),
            ExceptionOwner::Absent | ExceptionOwner::Present => None,
        }
    }

    pub(super) fn is_definitive(self) -> bool {
        !matches!(self, Self::Tentative(_))
    }

    pub(super) fn overrides_existing(self) -> bool {
        matches!(self, Self::Override(_))
    }
}

/// Decisions addressed by the dense final instruction index used throughout owner replay.
#[derive(Default)]
pub(super) struct DynamicExceptionOwners {
    // Zero is vacant; the low two bits encode strength and the rest encode the owner.
    slots: Vec<usize>,
    instruction_count: usize,
}

impl DynamicExceptionOwners {
    fn new(instruction_count: usize) -> Self {
        Self {
            slots: Vec::new(),
            instruction_count,
        }
    }

    #[inline]
    pub(super) fn get(&self, index: usize) -> Option<ExceptionOwnerDecision> {
        let value = *self.slots.get(index)?;
        if value == 0 {
            return None;
        }
        let owner = match value >> 2 {
            0 => ExceptionOwner::Absent,
            1 => ExceptionOwner::Present,
            region => ExceptionOwner::SourceRegion(region - 2),
        };
        Some(match value & 3 {
            1 => ExceptionOwnerDecision::Tentative(owner),
            2 => ExceptionOwnerDecision::Definitive(owner),
            3 => ExceptionOwnerDecision::Override(owner),
            _ => unreachable!("populated owner slots contain a decision strength"),
        })
    }

    #[inline]
    fn contains_key(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|value| *value != 0)
    }

    fn insert(&mut self, index: usize, decision: ExceptionOwnerDecision) {
        if index >= self.slots.len() {
            self.slots.resize(self.instruction_count.max(index + 1), 0);
        }
        let (strength, owner) = match decision {
            ExceptionOwnerDecision::Tentative(owner) => (1, owner),
            ExceptionOwnerDecision::Definitive(owner) => (2, owner),
            ExceptionOwnerDecision::Override(owner) => (3, owner),
        };
        let owner = match owner {
            ExceptionOwner::Absent => 0,
            ExceptionOwner::Present => 4,
            ExceptionOwner::SourceRegion(region) => region
                .checked_add(2)
                .and_then(|owner| owner.checked_mul(4))
                .expect("allocated exception region index fits in an owner slot"),
        };
        self.slots[index] = owner | strength;
    }
}

impl Extend<(usize, ExceptionOwnerDecision)> for DynamicExceptionOwners {
    fn extend<T: IntoIterator<Item = (usize, ExceptionOwnerDecision)>>(&mut self, values: T) {
        for (index, decision) in values {
            self.insert(index, decision);
        }
    }
}

/// Keeps the final owner writer and its successor; labels never write an owner themselves.
fn dynamic_owner_replay_item_limit(
    items: &[Item],
    instruction_count: usize,
    last_indexed_writer: Option<usize>,
) -> usize {
    // A source proof outside the final instruction stream cannot certify a shorter replay.
    if last_indexed_writer.is_some_and(|index| index >= instruction_count) {
        return items.len();
    }

    let mut index = instruction_count;
    let mut after_next_instruction = items.len();
    for (item_index, item) in items.iter().enumerate().rev() {
        let Item::Instruction(instruction) = item else {
            continue;
        };
        index -= 1;
        if last_indexed_writer.is_some_and(|last| index <= last)
            || instruction.opcode == NOT_TAKEN
            || is_conditional_jump(instruction.opcode)
            || matches!(
                instruction.exception_policy().owner(),
                NormalizedExceptionOwner::Dynamic
                    | NormalizedExceptionOwner::DynamicCondition
                    | NormalizedExceptionOwner::DynamicConditionStrict
            )
        {
            return after_next_instruction;
        }
        after_next_instruction = item_index + 1;
    }
    0
}

/// Source certificates that can make an otherwise ordinary instruction decide an owner.
#[derive(Default)]
struct OwnerDecisionIndices {
    positions: Vec<bool>,
}

impl OwnerDecisionIndices {
    fn new<'a>(instruction_count: usize, sources: impl IntoIterator<Item = &'a usize>) -> Self {
        let mut positions = Vec::new();
        for &index in sources {
            if index < instruction_count {
                if positions.is_empty() {
                    positions.resize(instruction_count, false);
                }
                positions[index] = true;
            }
        }
        Self { positions }
    }

    /// The common history update has already run. Such an instruction cannot decide an
    /// owner, prepare a pending decision, or reset the array in the final block update.
    fn can_finish_ordinary(
        &self,
        instruction: &Instruction,
        index: usize,
        pending_threaded_append: Option<usize>,
    ) -> bool {
        matches!(instruction.operand(), Operand::Value(_))
            && !matches!(instruction.opcode, NOT_TAKEN | FOR_ITER | SEND)
            && !is_conditional_jump(instruction.opcode)
            && !is_scope_exit(instruction.opcode)
            && !matches!(
                instruction.exception_policy().owner(),
                NormalizedExceptionOwner::Dynamic
                    | NormalizedExceptionOwner::DynamicCondition
                    | NormalizedExceptionOwner::DynamicConditionStrict
            )
            && pending_threaded_append != Some(index)
            && !self.positions.get(index).copied().unwrap_or(false)
    }

    /// The complete common history for these ordinary records is independent of
    /// source certificates other than a completed comprehension restore.
    fn can_replay_ordinary_history(
        &self,
        instruction: &Instruction,
        index: usize,
        pending_threaded_append: Option<usize>,
    ) -> bool {
        !matches!(instruction.opcode, NOP | POP_TOP)
            && matches!(
                instruction.exception_policy(),
                super::ExceptionPolicy::NoneUnchanged
                    | super::ExceptionPolicy::NoneUnchangedBooleanSingleton
            )
            && instruction.no_location_policy.borrow_fallthrough()
                != super::BorrowFallthrough::Barrier
            && instruction.local_policy.hidden_exception_owner_slots() == 0
            && instruction.pre_fold_extra_instructions() == 0
            && self.can_finish_ordinary(instruction, index, pending_threaded_append)
    }
}

/// A complete original array through one value Boolean, captured before CFG rewrites.
#[derive(Debug)]
pub(super) struct RetainedSetupValueArray {
    entry: Label,
    value_entry: Label,
    prefix: Vec<Item>,
    setups: Vec<(Label, Option<SourceLocation>)>,
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

    fn instruction(&mut self, instruction: &Instruction, previous: Option<&Instruction>) {
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

/// An emitted conditional array whose unchanged records prove a NULL append cell.
#[derive(Debug)]
pub(super) struct UncompactedConditionalArray {
    pub(super) prefix: Vec<Item>,
    comprehension_entry: Option<Label>,
    source_entry: Option<Label>,
    fallthrough: Option<[Instruction; 2]>,
}

type OwnerSignaturePositions = FxHashMap<OwnerInstructionSignature, SmallVec<[usize; 1]>>;

/// Whole-input lookup tables shared by conditional-array certificates.
struct UncompactedPrefixIndexes {
    positions: FxHashMap<Label, usize>,
    repeated_labels: FxHashSet<Label>,
    forward_jumps: FxHashMap<Label, smallvec::SmallVec<[usize; 1]>>,
    targets: FxHashSet<Label>,
    authored_break_targets: FxHashSet<Label>,
    pass_positions: OnceCell<(OwnerSignaturePositions, OwnerSignaturePositions)>,
    fallthrough_setup_pairs: OnceCell<FxHashMap<(Label, Instruction), FxHashSet<Label>>>,
}

impl UncompactedPrefixIndexes {
    fn new(all: &[Item]) -> Self {
        let mut positions = FxHashMap::default();
        let mut repeated_labels = FxHashSet::default();
        let mut forward_jumps = FxHashMap::<_, smallvec::SmallVec<[usize; 1]>>::default();
        let mut targets = FxHashSet::default();
        let mut authored_break_targets = FxHashSet::default();
        for (index, item) in all.iter().enumerate() {
            match item {
                Item::Label(label) => {
                    if positions.insert(*label, index).is_some() {
                        repeated_labels.insert(*label);
                    }
                }
                Item::Instruction(instruction) => {
                    if let Operand::Forward(label) = instruction.operand() {
                        forward_jumps.entry(label).or_default().push(index);
                    }
                    if let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                    {
                        targets.insert(label);
                        if instruction.fusion_policy.is_authored_break() {
                            authored_break_targets.insert(label);
                        }
                    }
                }
            }
        }
        Self {
            positions,
            repeated_labels,
            forward_jumps,
            targets,
            authored_break_targets,
            pass_positions: OnceCell::new(),
            fallthrough_setup_pairs: OnceCell::new(),
        }
    }

    /// Finds the same recorded SETUP/NOP pair without rescanning all prior SETUPs.
    fn has_fallthrough_setup_pair(
        &self,
        provenance: &ExceptionOwnerProvenance,
        all: &[Item],
        labels: &FxHashSet<Label>,
        instruction: Instruction,
    ) -> bool {
        let pairs = self.fallthrough_setup_pairs.get_or_init(|| {
            let instructions: Vec<_> = all
                .iter()
                .enumerate()
                .filter_map(|(index, item)| matches!(item, Item::Instruction(_)).then_some(index))
                .collect();
            let mut setups = FxHashMap::<SourceLocation, Vec<usize>>::default();
            let mut setup_locations = Vec::new();
            for (label, location) in &provenance.setups {
                if let (Some(location), Some(&position)) = (location, self.positions.get(label)) {
                    setups
                        .entry(*location)
                        .or_insert_with(|| {
                            setup_locations.push(*location);
                            Vec::new()
                        })
                        .push(position);
                }
            }
            for location in setup_locations {
                setups
                    .get_mut(&location)
                    .expect("recorded setup location")
                    .sort_unstable();
            }

            let mut pairs = FxHashMap::<(Label, Instruction), FxHashSet<Label>>::default();
            for &(start, end) in &provenance.setup_nop_ranges {
                let (Some(&start_at), Some(&end_at)) =
                    (self.positions.get(&start), self.positions.get(&end))
                else {
                    continue;
                };
                if start_at >= end_at
                    || !self.has_unique_label(start)
                    || !self.has_unique_label(end)
                {
                    continue;
                }
                let first = instructions.partition_point(|&index| index < start_at);
                let after = instructions.partition_point(|&index| index < end_at);
                if after != first + 1 {
                    continue;
                }
                let Item::Instruction(value) = all[instructions[first]] else {
                    unreachable!();
                };
                let Some(positions) = setups.get(&value.location) else {
                    continue;
                };
                let setup = positions.partition_point(|&index| index < end_at);
                let next_instruction = instructions.get(after).copied().unwrap_or(all.len());
                if positions
                    .get(setup)
                    .is_some_and(|&index| index < next_instruction)
                {
                    pairs.entry((start, value)).or_default().insert(end);
                }
            }
            pairs
        });
        labels.iter().any(|&start| {
            pairs
                .get(&(start, instruction))
                .is_some_and(|ends| ends.iter().any(|end| labels.contains(end)))
        })
    }

    fn has_unique_label(&self, label: Label) -> bool {
        self.positions.contains_key(&label) && !self.repeated_labels.contains(&label)
    }

    fn has_unique_forward_jump(&self, all: &[Item], instruction: Instruction) -> bool {
        let Operand::Forward(label) = instruction.operand() else {
            return false;
        };
        self.forward_jumps.get(&label).is_some_and(|positions| {
            let mut matches = positions
                .iter()
                .filter(|&&position| all[position] == Item::Instruction(instruction));
            matches.next().is_some() && matches.next().is_none()
        })
    }

    fn has_unique_authored_pass(
        &self,
        all: &[Item],
        original: &[Instruction],
        instruction: &Instruction,
    ) -> bool {
        let (authored, emitted) = self.pass_positions.get_or_init(|| {
            let mut authored: FxHashMap<_, SmallVec<[usize; 1]>> = FxHashMap::default();
            for (index, instruction) in original.iter().enumerate() {
                authored
                    .entry(OwnerInstructionPositions::signature(instruction))
                    .or_default()
                    .push(index);
            }
            let mut emitted: FxHashMap<_, SmallVec<[usize; 1]>> = FxHashMap::default();
            for (index, item) in all.iter().enumerate() {
                if let Item::Instruction(instruction) = item {
                    let signature = OwnerInstructionPositions::signature(instruction);
                    if authored.contains_key(&signature) {
                        emitted.entry(signature).or_default().push(index);
                    }
                }
            }
            (authored, emitted)
        });
        let signature = OwnerInstructionPositions::signature(instruction);
        authored.get(&signature).is_some_and(|positions| {
            positions
                .iter()
                .filter(|&&index| original[index] == *instruction)
                .take(2)
                .count()
                == 1
        }) && emitted.get(&signature).is_some_and(|positions| {
            positions
                .iter()
                .filter(|&&index| all[index] == Item::Instruction(*instruction))
                .take(2)
                .count()
                == 1
        })
    }
}

/// Final instruction positions, with compiler-generated labels indexed without hashing.
struct OwnerLabelPositions {
    dense: Vec<usize>,
    sparse: FxHashMap<Label, usize>,
    dense_limit: usize,
}

impl OwnerLabelPositions {
    fn new(item_count: usize) -> Self {
        Self {
            dense: Vec::new(),
            sparse: FxHashMap::default(),
            dense_limit: item_count.saturating_mul(2).max(64),
        }
    }

    #[inline]
    fn insert(&mut self, label: Label, position: usize) {
        let index = label.index();
        if index >= self.dense_limit {
            self.sparse.insert(label, position);
            return;
        }
        if index >= self.dense.len() {
            self.dense.resize(index + 1, usize::MAX);
        }
        self.dense[index] = position;
    }

    #[inline]
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "The shared lookup interface takes the same borrowed labels as its sparse map."
    )]
    fn get(&self, label: &Label) -> Option<&usize> {
        let index = label.index();
        if index >= self.dense_limit {
            return self.sparse.get(label);
        }
        self.dense
            .get(index)
            .filter(|position| **position != usize::MAX)
    }

    #[cfg(test)]
    fn from_hashed(item_count: usize, positions: &FxHashMap<Label, usize>) -> Self {
        let mut result = Self::new(item_count);
        #[expect(
            clippy::iter_over_hash_type,
            reason = "Each distinct label writes its own indexed position."
        )]
        for (&label, &position) in positions {
            result.insert(label, position);
        }
        result
    }
}

impl std::ops::Index<&Label> for OwnerLabelPositions {
    type Output = usize;

    fn index(&self, label: &Label) -> &Self::Output {
        self.get(label)
            .expect("indexed exception owner label is bound")
    }
}

/// Immutable label memberships read together by final exception-owner replay.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
struct OwnerReplayLabelFlags(u16);

impl OwnerReplayLabelFlags {
    const BOUNDARY: u16 = 1 << 0;
    const JUMP_TARGET: u16 = 1 << 1;
    const REGION_END: u16 = 1 << 2;
    const SOURCE_BOUNDARY: u16 = 1 << 3;
    const REMOVED_POP_BLOCK: u16 = 1 << 4;
    const CONTINUATION: u16 = 1 << 5;
    const SETUP_NOP: u16 = 1 << 6;
    const FUSION: u16 = 1 << 7;
    const OMITTED_NOP: u16 = 1 << 8;
    const ORIGINAL_BLOCK: u16 = 1 << 9;
    const VALUE_JUMP_CONTINUATION: u16 = 1 << 10;
    const PROTECTED_PREFIX: u16 = 1 << 11;
    const RETIRED_TRY_COPY: u16 = 1 << 12;
    const SYNCHRONOUS_WITH: u16 = 1 << 13;
    const SETUP: u16 = 1 << 14;

    #[inline]
    const fn contains(self, flags: u16) -> bool {
        self.0 & flags != 0
    }
}

/// Keeps compiler-generated labels dense without allocating for arbitrary external labels.
struct OwnerReplayLabels {
    dense: Vec<OwnerReplayLabelFlags>,
    sparse: FxHashMap<Label, OwnerReplayLabelFlags>,
    dense_limit: usize,
}

impl OwnerReplayLabels {
    fn new(item_count: usize, label_count: usize) -> Self {
        let dense_limit = item_count.saturating_mul(2).max(64);
        Self {
            dense: vec![OwnerReplayLabelFlags::default(); label_count.min(dense_limit)],
            sparse: FxHashMap::default(),
            dense_limit,
        }
    }

    fn record<'a>(&mut self, labels: impl IntoIterator<Item = &'a Label>, flags: u16) {
        for &label in labels {
            let index = label.index();
            if index >= self.dense_limit {
                self.sparse.entry(label).or_default().0 |= flags;
            } else {
                if index >= self.dense.len() {
                    self.dense
                        .resize(index + 1, OwnerReplayLabelFlags::default());
                }
                self.dense[index].0 |= flags;
            }
        }
    }

    #[inline]
    fn get(&self, label: Label) -> OwnerReplayLabelFlags {
        let index = label.index();
        if index >= self.dense_limit {
            self.sparse.get(&label).copied().unwrap_or_default()
        } else {
            self.dense.get(index).copied().unwrap_or_default()
        }
    }
}

/// Item and instruction positions for labels that occur exactly once.
struct UniqueLabelPositions {
    dense: Vec<Option<(usize, usize)>>,
    sparse: FxHashMap<Label, Option<(usize, usize)>>,
    dense_limit: usize,
}

impl UniqueLabelPositions {
    fn new(items: &[Item]) -> Self {
        let mut positions = Self {
            dense: Vec::new(),
            sparse: FxHashMap::default(),
            dense_limit: items.len().saturating_mul(2).max(64),
        };
        let mut instruction_index = 0;
        for (item_index, item) in items.iter().enumerate() {
            match item {
                Item::Label(label) => {
                    let index = label.index();
                    if index >= positions.dense_limit {
                        positions
                            .sparse
                            .entry(*label)
                            .and_modify(|position| *position = None)
                            .or_insert(Some((item_index, instruction_index)));
                    } else {
                        if index >= positions.dense.len() {
                            positions.dense.resize(index + 1, None);
                        }
                        let slot = &mut positions.dense[index];
                        *slot = Some(if slot.is_some() {
                            (usize::MAX, usize::MAX)
                        } else {
                            (item_index, instruction_index)
                        });
                    }
                }
                Item::Instruction(_) => instruction_index += 1,
            }
        }
        positions
    }

    fn get(&self, label: Label) -> Option<(usize, usize)> {
        let index = label.index();
        if index >= self.dense_limit {
            return self.sparse.get(&label).copied().flatten();
        }
        self.dense
            .get(index)
            .copied()
            .flatten()
            .filter(|(item_index, _)| *item_index != usize::MAX)
    }
}

/// Candidate source positions narrowed by opcode and location, then checked in full.
#[derive(Default)]
struct OwnerInstructionPositions {
    candidates: FxHashSet<OwnerInstructionSignature>,
    positions: OnceCell<FxHashMap<OwnerInstructionSignature, SmallVec<[usize; 1]>>>,
}

type OwnerInstructionSignature = (crate::target::Opcode, i32, i32, i32, i32);

impl OwnerInstructionPositions {
    fn for_sources<'a>(
        instruction_count: usize,
        source_count: usize,
        sources: impl IntoIterator<Item = &'a Instruction>,
    ) -> Self {
        // One-off source certificates are cheaper to look up directly. For larger input,
        // retain only the signatures actually queried, rather than indexing all records.
        if instruction_count < 32 || source_count < 4 {
            return Self::default();
        }
        Self {
            candidates: sources.into_iter().map(Self::signature).collect(),
            positions: OnceCell::new(),
        }
    }

    fn signature(instruction: &Instruction) -> OwnerInstructionSignature {
        let location = instruction.location;
        (
            instruction.opcode,
            location.line,
            location.end_line,
            location.column,
            location.end_column,
        )
    }

    fn positions(&self, instructions: &[&Instruction], original: &Instruction) -> Option<&[usize]> {
        if self.candidates.is_empty() {
            return None;
        }
        let key = Self::signature(original);
        if !self.candidates.contains(&key) {
            return None;
        }
        let positions = self.positions.get_or_init(|| {
            let mut positions: FxHashMap<_, SmallVec<[usize; 1]>> = self
                .candidates
                .iter()
                .map(|key| (*key, SmallVec::new()))
                .collect();
            for (position, instruction) in instructions.iter().enumerate() {
                if let Some(matches) = positions.get_mut(&Self::signature(instruction)) {
                    matches.push(position);
                }
            }
            positions
        });
        positions.get(&key).map(SmallVec::as_slice)
    }

    fn unique(&self, instructions: &[&Instruction], original: &Instruction) -> Option<usize> {
        if let Some(candidates) = self.positions(instructions, original) {
            let mut matches = candidates
                .iter()
                .copied()
                .filter(|&position| *instructions[position] == *original);
            let position = matches.next()?;
            return matches.next().is_none().then_some(position);
        }
        let mut matches = instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| ***instruction == *original);
        let (position, _) = matches.next()?;
        matches.next().is_none().then_some(position)
    }

    fn unique_pair(
        &self,
        instructions: &[&Instruction],
        first: &Instruction,
        second: &Instruction,
    ) -> Option<usize> {
        if let Some(candidates) = self.positions(instructions, first) {
            let mut matches = candidates.iter().copied().filter(|&position| {
                *instructions[position] == *first
                    && instructions
                        .get(position + 1)
                        .is_some_and(|instruction| **instruction == *second)
            });
            let position = matches.next()?;
            return matches.next().is_none().then_some(position);
        }
        let mut matches = instructions
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| *pair[0] == *first && *pair[1] == *second);
        let (position, _) = matches.next()?;
        matches.next().is_none().then_some(position)
    }
}

impl ExceptionOwnerProvenance {
    /// Checks labels belonging to a captured normal-try copy, including its endpoints.
    fn is_normal_try_copy_label(&self, label: Label) -> bool {
        self.normal_try_copy_blocks.contains(&label)
            || self.normal_try_copy_jump_nop_successors.contains(&label)
            || self.normal_try_copy_inline_labels.contains(&label)
            || self.normal_try_copy_internal_labels.contains(&label)
            || self
                .normal_try_copy_steps
                .iter()
                .any(|(start, end, _)| *start == label || *end == label)
            || self
                .normal_try_copy_pop_ranges
                .iter()
                .any(|(start, end)| *start == label || *end == label)
    }

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
    fn original_for_target_prefix_positions(
        &self,
        items: &[Item],
        unique_labels: &OnceCell<UniqueLabelPositions>,
    ) -> FxHashMap<usize, usize> {
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
                let (index, position) = unique_labels
                    .get_or_init(|| UniqueLabelPositions::new(items))
                    .get(*entry)?;
                let mut expected = *store;
                expected.local_policy.add_hidden_exception_owner_slots(1);
                if !matches!(items.get(index + 1), Some(Item::Instruction(actual)) if *actual == expected) {
                    return None;
                }
                Some((position, position))
            })
            .collect()
    }

    /// Locates complete unchanged Boolean expansions at their unique original labels.
    fn original_value_jump_positions(
        &self,
        items: &[Item],
        unique_labels: &OnceCell<UniqueLabelPositions>,
    ) -> FxHashMap<Label, usize> {
        self.original_value_jump_prefixes
            .iter()
            .filter_map(|(entry, prefix)| {
                if !self.value_jump_continuation_boundaries.contains(entry)
                    || self.retired_normal_try_copy_labels.contains(entry)
                    || self.is_normal_try_copy_label(*entry)
                    || self
                        .original_value_jump_prefixes
                        .iter()
                        .filter(|(label, _)| label == entry)
                        .count()
                        != 1
                {
                    return None;
                }
                let (start, position) = unique_labels
                    .get_or_init(|| UniqueLabelPositions::new(items))
                    .get(*entry)?;
                if items.get(start..start.checked_add(prefix.len())?)? != prefix.as_slice() {
                    return None;
                }
                let records = prefix
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(instruction),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>();
                let [copy, to_bool, condition, marker] = records.as_slice() else {
                    return None;
                };
                (copy.opcode == crate::target::opcodes::COPY
                    && copy.operand() == Operand::Value(1)
                    && to_bool.opcode == crate::target::opcodes::TO_BOOL
                    && is_conditional_jump(condition.opcode)
                    && matches!(condition.operand(), Operand::Forward(_))
                    && marker.opcode == NOT_TAKEN
                    && marker.exception_policy() == super::ExceptionPolicy::NoneDynamic)
                    .then(|| (*entry, position + 3))
            })
            .collect()
    }

    /// Captures resolved source records before later passes can copy or change the array.
    fn capture_retained_setup_value_arrays(&mut self, items: &[Item]) {
        if self.original_value_jump_prefixes.is_empty()
            || self.synchronous_with_setups.is_empty()
            || !self.setups.iter().any(|(label, location)| {
                location.is_none() && self.synchronous_with_setups.contains(label)
            })
            || !self.setups.iter().any(|(_, location)| location.is_some())
        {
            return;
        }
        for (value_entry, value_prefix) in &self.original_value_jump_prefixes {
            let mut starts = items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| (*item == Item::Label(*value_entry)).then_some(index));
            let Some(start) = starts.next() else {
                continue;
            };
            if starts.next().is_some()
                || items.get(start..start + value_prefix.len()) != Some(value_prefix.as_slice())
            {
                continue;
            }
            let Some(begin) = items[..start].iter().rposition(|item| {
                matches!(item, Item::Label(label)
                    if self.source_boundaries.contains(label)
                        && self.block_boundaries.contains(label))
            }) else {
                continue;
            };
            let Item::Label(entry) = items[begin] else {
                unreachable!();
            };
            let prefix = &items[begin..start + value_prefix.len()];
            let setups = self
                .setups
                .iter()
                .filter(|(label, _)| prefix.contains(&Item::Label(*label)))
                .copied()
                .collect::<Vec<_>>();
            if setups
                .iter()
                .filter(|(_, location)| location.is_some())
                .count()
                != 1
                || !setups.iter().any(|(label, location)| {
                    location.is_none() && self.synchronous_with_setups.contains(label)
                })
            {
                continue;
            }
            self.retained_setup_value_arrays
                .push(RetainedSetupValueArray {
                    entry,
                    value_entry: *value_entry,
                    prefix: prefix.to_vec(),
                    setups,
                });
        }
    }

    /// Finds a unique unchanged survivor whose literal-fold witness has not retired.
    fn original_uniform_fold_positions(&self, instructions: &[&Instruction]) -> FxHashSet<usize> {
        self.original_uniform_fold_positions_with_index::<false>(
            instructions,
            &OwnerInstructionPositions::default(),
        )
    }

    fn original_uniform_fold_positions_with_index<const INDEX_ORIGINS: bool>(
        &self,
        instructions: &[&Instruction],
        instruction_positions: &OwnerInstructionPositions,
    ) -> FxHashSet<usize> {
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
                if INDEX_ORIGINS {
                    instruction_positions.unique(instructions, original)
                } else {
                    let mut matches = instructions
                        .iter()
                        .enumerate()
                        .filter(|(_, instruction)| ***instruction == *original);
                    let (index, _) = matches.next()?;
                    matches.next().is_none().then_some(index)
                }
            })
            .collect()
    }

    /// Retains whole original arrays, before optimization can remove their live records.
    pub(super) fn capture_uncompacted_conditional_arrays(
        &mut self,
        items: &[Item],
        allow_fallthrough: bool,
    ) {
        use crate::target::opcodes::{POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE};

        self.capture_retained_setup_value_arrays(items);
        let indexes = OnceCell::new();
        let original_boundary_counts = OnceCell::new();
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
                    let certified = if self.uncompacted_prefix(prefix, items, &indexes, 0, false) {
                        Some((prefix, None, None, None))
                    } else if let Some((entry, prefix, restore_count)) =
                        self.comprehension_normal_prefix(prefix, items)
                        && self.uncompacted_prefix(prefix, items, &indexes, restore_count, false)
                    {
                        Some((prefix, Some(entry), None, None))
                    } else if let Some((entry, prefix)) = self.original_source_prefix(prefix)
                        && self.uncompacted_prefix(prefix, items, &indexes, 0, true)
                    {
                        Some((prefix, None, Some(entry), None))
                    } else if allow_fallthrough
                        && let Some(anchor) = self.uncompacted_nested_try_fallthrough(
                            prefix,
                            &items[..begin],
                            items,
                            &indexes,
                            &original_boundary_counts,
                        )
                    {
                        Some((prefix, None, None, Some(anchor)))
                    } else {
                        None
                    };
                    if let Some((prefix, comprehension_entry, source_entry, fallthrough)) =
                        certified
                    {
                        self.uncompacted_conditional_arrays
                            .push(UncompactedConditionalArray {
                                prefix: prefix.to_vec(),
                                comprehension_entry,
                                source_entry,
                                fallthrough,
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

    /// Authenticates a primitive condition falling directly into two nested tries.
    fn uncompacted_nested_try_fallthrough(
        &self,
        prefix: &[Item],
        preceding: &[Item],
        all: &[Item],
        indexes: &OnceCell<UncompactedPrefixIndexes>,
        original_boundary_counts: &OnceCell<FxHashMap<Instruction, usize>>,
    ) -> Option<[Instruction; 2]> {
        use crate::target::opcodes::{LOAD_FAST, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, TO_BOOL};

        let [.., Item::Instruction(condition), Item::Instruction(marker)] = preceding else {
            return None;
        };
        let records = prefix
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let [outer, inner, load, to_bool, jump, next_marker] = records.as_slice() else {
            return None;
        };
        let unique_original = |instruction: &Instruction| {
            original_boundary_counts
                .get_or_init(|| {
                    let mut counts = FxHashMap::default();
                    for original in &self.original_primitive_condition_boundaries {
                        *counts.entry(*original).or_insert(0) += 1;
                    }
                    counts
                })
                .get(instruction)
                == Some(&1)
        };
        // Consecutive authored header lines leave no place for an omitted
        // statement between the original fallthrough, two SETUPs and test.
        if !matches!(condition.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
            || !self
                .original_name_while_locations
                .contains(&condition.location)
            || marker.opcode != NOT_TAKEN
            || marker.location != condition.location
            || marker.depth_after() != condition.depth_after()
            || outer.opcode != NOP
            || inner.opcode != NOP
            || load.opcode != LOAD_FAST
            || to_bool.opcode != TO_BOOL
            || !matches!(jump.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
            || !self
                .original_initial_name_if_locations
                .contains(&jump.location)
            || !unique_original(jump)
            || !indexes
                .get_or_init(|| UncompactedPrefixIndexes::new(all))
                .has_unique_forward_jump(all, *jump)
            || next_marker.opcode != NOT_TAKEN
            || condition.location.line < 0
            || condition.location.line.checked_add(1) != Some(outer.location.line)
            || outer.location.line.checked_add(1) != Some(inner.location.line)
            || inner.location.line.checked_add(1) != Some(load.location.line)
            || load.location != to_bool.location
            || load.location != jump.location
            || load.location != next_marker.location
            || !unique_original(condition)
            || !indexes
                .get_or_init(|| UncompactedPrefixIndexes::new(all))
                .has_unique_forward_jump(all, *condition)
            || !self.uncompacted_prefix_inner(prefix, all, indexes, 0, false, true)
        {
            return None;
        }
        Some([*condition, *marker])
    }

    /// Selects the exact normal restore entry, excluding preceding cleanup aliases.
    fn comprehension_normal_prefix<'a>(
        &self,
        prefix: &'a [Item],
        all: &[Item],
    ) -> Option<(Label, &'a [Item], usize)> {
        use crate::target::opcodes::{
            BUILD_LIST, BUILD_MAP, BUILD_TUPLE, CALL, DICT_UPDATE, LIST_EXTEND, LOAD_ATTR,
            LOAD_CONST, LOAD_FAST, LOAD_SMALL_INT, STORE_DEREF, STORE_FAST, STORE_NAME, SWAP,
        };

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
            // The original seal contains adjacent instructions. A live label
            // inside the restore would split that array, even if filtering it
            // produced the same sequence of opcodes.
            let mut records = prefix[start..]
                .iter()
                .skip_while(|item| matches!(item, Item::Label(_)));
            if !original
                .iter()
                .all(|instruction| records.next() == Some(&Item::Instruction(*instruction)))
            {
                continue;
            }
            let next = records.find_map(|item| {
                if let Item::Instruction(instruction) = item {
                    Some(*instruction)
                } else {
                    None
                }
            });
            // All authored restores are STORE_FAST_MAYBE_NULL in CPython,
            // including cells. They remain distinct through superinstruction
            // insertion. The following operation is not swappable, so only n
            // swappable restores precede it and SWAP(n + 1) cannot be eliminated.
            if next.is_some_and(|next| {
                matches!(
                    next.opcode,
                    BUILD_LIST
                        | BUILD_MAP
                        | BUILD_TUPLE
                        | CALL
                        | DICT_UPDATE
                        | LIST_EXTEND
                        | LOAD_ATTR
                        | LOAD_CONST
                        | LOAD_FAST
                        | LOAD_SMALL_INT
                        | STORE_DEREF
                        | STORE_NAME
                )
            }) {
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

    /// Proves the unchanged success array after an empty class pattern.
    fn empty_class_success_prefix(prefix: &[Item], all: &[Item]) -> bool {
        use crate::target::opcodes::{COPY, MATCH_CLASS, POP_JUMP_IF_NONE, UNPACK_SEQUENCE};

        let [
            Item::Instruction(unpack),
            Item::Instruction(discard),
            Item::Instruction(next),
            ..,
        ] = prefix
        else {
            return false;
        };
        let (Some(unpack_depth), Some(discard_depth)) =
            (unpack.depth_after(), discard.depth_after())
        else {
            return false;
        };
        if unpack.opcode != UNPACK_SEQUENCE
            || unpack.operand() != Operand::Value(0)
            || discard.opcode != POP_TOP
            || discard.operand() != Operand::Value(0)
            || !discard.fusion_policy.location_from_next()
            || discard.location != next.location
            || unpack_depth.checked_sub(1) != Some(discard_depth)
        {
            return false;
        }
        let mut starts = all
            .iter()
            .enumerate()
            .filter_map(|(index, item)| (*item == Item::Instruction(*unpack)).then_some(index));
        let Some(start) = starts.next() else {
            return false;
        };
        if starts.next().is_some() {
            return false;
        }
        let Some(start) = start.checked_sub(4) else {
            return false;
        };
        let [
            Item::Instruction(class),
            Item::Instruction(copy),
            Item::Instruction(failure),
            Item::Instruction(marker),
        ] = &all[start..start + 4]
        else {
            return false;
        };
        let (Some(class_depth), Some(copy_depth), Some(failure_depth), Some(marker_depth)) = (
            class.depth_after(),
            copy.depth_after(),
            failure.depth_after(),
            marker.depth_after(),
        ) else {
            return false;
        };
        // The two hidden COPY cells are the None comparison in the preceding
        // array. The authored failure conditional starts a fresh success array;
        // UNPACK(0) and the subject discard both survive in that new array.
        class.opcode == MATCH_CLASS
            && class.operand() == Operand::Value(0)
            && copy.opcode == COPY
            && copy.operand() == Operand::Value(1)
            && copy.local_policy.hidden_exception_owner_slots() == 2
            && copy.pre_fold_extra_instructions() == 0
            && failure.opcode == POP_JUMP_IF_NONE
            && matches!(failure.operand(), Operand::Forward(_))
            && marker.opcode == NOT_TAKEN
            && [class, copy, failure, marker]
                .iter()
                .all(|record| record.location == unpack.location)
            && class_depth.checked_add(1) == Some(copy_depth)
            && failure_depth == class_depth
            && marker_depth == failure_depth
            && failure_depth.checked_sub(1) == Some(unpack_depth)
    }

    fn uncompacted_prefix(
        &self,
        prefix: &[Item],
        all: &[Item],
        indexes: &OnceCell<UncompactedPrefixIndexes>,
        comprehension_restore_count: usize,
        source_head: bool,
    ) -> bool {
        self.uncompacted_prefix_inner(
            prefix,
            all,
            indexes,
            comprehension_restore_count,
            source_head,
            false,
        )
    }

    fn uncompacted_prefix_inner(
        &self,
        prefix: &[Item],
        all: &[Item],
        indexes: &OnceCell<UncompactedPrefixIndexes>,
        comprehension_restore_count: usize,
        source_head: bool,
        fallthrough_head: bool,
    ) -> bool {
        use crate::target::opcodes::{
            BINARY_OP, BUILD_LIST, BUILD_MAP, BUILD_STRING, BUILD_TUPLE, CALL, CALL_INTRINSIC_1,
            CALL_KW, CONTAINS_OP, DICT_UPDATE, FORMAT_SIMPLE, GET_ITER, IMPORT_FROM, IMPORT_NAME,
            LIST_APPEND, LIST_EXTEND, LOAD_ATTR, LOAD_CONST, LOAD_DEREF, LOAD_FAST, LOAD_GLOBAL,
            LOAD_NAME, LOAD_SMALL_INT, MAKE_FUNCTION, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE,
            PUSH_NULL, RESUME, SET_FUNCTION_ATTRIBUTE, STORE_DEREF, STORE_FAST, STORE_NAME,
            TO_BOOL, UNPACK_SEQUENCE, YIELD_VALUE,
        };
        use crate::target::operands::UnaryIntrinsic;

        let empty_class_success = Self::empty_class_success_prefix(prefix, all);
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
        let indexes = || indexes.get_or_init(|| UncompactedPrefixIndexes::new(all));
        // A raw emitted return or copied backedge is not necessarily a CP
        // record. The compiler's original branch target establishes the left
        // edge independently; the preceding terminator only bounds this scan.
        let has_original_target = || {
            self.original_primitive_condition_boundaries
                .iter()
                .any(|condition| {
                    matches!(condition.operand(), Operand::Forward(label) if leading_labels.contains(&label))
                        && indexes().has_unique_forward_jump(all, *condition)
                })
                || leading_labels
                    .iter()
                    .any(|label| indexes().authored_break_targets.contains(label))
                || self.original_match_join_boundaries.iter().any(|entry| {
                    leading_labels.contains(entry) && indexes().has_unique_label(*entry)
                })
        };
        if source_head {
            // Graph jump targets alone include copied exits. Require the compiler's
            // original source boundary and exactly one original array at this head.
            let mut boundaries = leading_labels
                .iter()
                .filter(|label| self.block_boundaries.contains(*label));
            let Some(entry) = boundaries.next() else {
                return false;
            };
            if !self.source_boundaries.contains(entry) {
                return false;
            }
            if boundaries.next().is_some() {
                // Nested primitive branches can end at adjacent source labels.
                // They share one instruction-array head only when each alias is
                // the target of its own still-literal authored conditional.
                if leading_labels
                    .iter()
                    .filter(|label| self.block_boundaries.contains(*label))
                    .any(|label| {
                        !self.source_boundaries.contains(label)
                            || !indexes().has_unique_label(*label)
                            || !self.original_primitive_condition_boundaries.iter().any(
                                |condition| {
                                    condition.operand() == Operand::Forward(*label)
                                        && indexes().has_unique_forward_jump(all, *condition)
                                },
                            )
                    })
                {
                    return false;
                }
            }
        } else if fallthrough_head {
            // No other control-flow edge may enter this predecessor fallthrough.
            if leading_labels.is_empty()
                || leading_labels.iter().any(|label| {
                    indexes().targets.contains(label)
                        || self.block_boundaries.contains(label)
                        || self.source_boundaries.contains(label)
                })
            {
                return false;
            }
        } else if comprehension_restore_count == 0
            && !empty_class_success
            && (leading_labels.is_empty() || !has_original_target())
        {
            return false;
        }
        let UncompactedPrefixIndexes {
            positions, targets, ..
        } = indexes();
        let mut instructions = Vec::new();
        let mut instruction_item_positions = Vec::new();
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
                        || (fallthrough_head
                            && !instructions.is_empty()
                            && (self.block_boundaries.contains(label)
                                || self.source_boundaries.contains(label)))
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
                    if comprehension_restore_count > 0 {
                        instruction_item_positions.push(item_index);
                    }
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
        // A one-cell closure tuple is built from LOAD_CLOSURE, not constants.
        // Its complete function-construction sequence cannot fold or unpack;
        // retain all six original records and their exact source identity.
        let closure_members = instructions
            .windows(6)
            .enumerate()
            .filter_map(|(start, parts)| {
                let [cell, tuple, code, function, closure, store] = parts else {
                    unreachable!()
                };
                (cell.opcode == LOAD_FAST
                    && cell.local_policy.is_post_analysis_closure_load()
                    && tuple.opcode == BUILD_TUPLE
                    && tuple.operand() == Operand::Value(1)
                    && code.opcode == LOAD_CONST
                    && function.opcode == MAKE_FUNCTION
                    && function.operand() == Operand::Value(0)
                    && closure.opcode == SET_FUNCTION_ATTRIBUTE
                    && closure.operand() == Operand::Value(8)
                    && store.opcode == STORE_FAST
                    && parts.iter().all(|part| part.location == cell.location))
                .then_some(start + 1..start + 5)
            })
            .flatten()
            .collect::<FxHashSet<_>>();
        // A local function with deferred annotations constructs the annotation and body
        // functions separately. All six original records survive; the attribute attachment
        // neither folds constants nor consumes an instruction-array cell.
        let annotated_function_members = instructions
            .windows(6)
            .enumerate()
            .filter_map(|(start, parts)| {
                let [
                    annotations,
                    annotation_function,
                    code,
                    function,
                    attribute,
                    store,
                ] = parts
                else {
                    unreachable!()
                };
                (annotations.opcode == LOAD_CONST
                    && annotation_function.opcode == MAKE_FUNCTION
                    && annotation_function.operand() == Operand::Value(0)
                    && code.opcode == LOAD_CONST
                    && function.opcode == MAKE_FUNCTION
                    && function.operand() == Operand::Value(0)
                    && attribute.opcode == SET_FUNCTION_ATTRIBUTE
                    && attribute.operand() == Operand::Value(16)
                    && store.opcode == STORE_FAST
                    && parts
                        .iter()
                        .all(|part| part.location == annotations.location))
                .then_some(start..start + 6)
            })
            .flatten()
            .collect::<FxHashSet<_>>();
        let original_folds = if fallthrough_head {
            // This certificate already requires exactly SETUP, SETUP, LOAD_FAST,
            // TO_BOOL, jump, marker. It cannot contain a constant-fold witness.
            FxHashSet::default()
        } else {
            self.original_uniform_fold_positions(&instructions.iter().collect::<Vec<_>>())
        };
        // A discarded async yield retains all four records in the original array.
        // Resume changes only its exception-depth bit, without creating a vacancy.
        let yield_members = instructions
            .windows(4)
            .enumerate()
            .filter_map(|(start, parts)| {
                let [wrap, yielded, resume, discard] = parts else {
                    unreachable!();
                };
                (comprehension_restore_count > 0
                    && start > comprehension_restore_count
                    && wrap.opcode == CALL_INTRINSIC_1
                    && wrap.operand() == Operand::Value(UnaryIntrinsic::ASYNC_GEN_WRAP.argument())
                    && yielded.opcode == YIELD_VALUE
                    && yielded.operand() == Operand::Value(0)
                    && resume.opcode == RESUME
                    && matches!(resume.operand(), Operand::Value(1 | 5))
                    && discard.opcode == POP_TOP
                    && discard.operand() == Operand::Value(0)
                    && instruction_item_positions[start + 3]
                        == instruction_item_positions[start] + 3
                    && parts.iter().all(|part| part.location == wrap.location)
                    && wrap.depth_after().is_some()
                    && yielded.depth_after() == wrap.depth_after()
                    && resume.depth_after() == wrap.depth_after()
                    && wrap
                        .depth_after()
                        .and_then(|depth| depth.checked_sub(1))
                        .is_some_and(|depth| discard.depth_after() == Some(depth)))
                .then_some(start..start + 4)
            })
            .flatten()
            .collect::<FxHashSet<_>>();
        let mut setup_count = 0;
        let mut folded_slots = 0usize;
        let mut membership_folds = 0;
        let mut null_setup_cells = FxHashSet::default();
        // These entry opcodes extend the original comprehension certificate.
        // Their original array must remain physically continuous through the
        // marker, including after the collection operation.
        let mut expanded_collection = comprehension_restore_count > 0
            && instructions
                .get(comprehension_restore_count + 1)
                .is_some_and(|next| {
                    matches!(
                        next.opcode,
                        BUILD_TUPLE | LIST_EXTEND | LOAD_CONST | LOAD_SMALL_INT
                    )
                });
        for (index, instruction) in instructions.iter().enumerate() {
            let membership_fold = instruction.opcode == LOAD_CONST
                && original_folds.contains(&index)
                && instructions
                    .get(index + 1)
                    .is_some_and(|next| next.opcode == CONTAINS_OP);
            if ((instruction.local_policy.hidden_exception_owner_slots() != 0
                || instruction.pre_fold_extra_instructions() != 0)
                && !membership_fold)
                || instruction.no_location_policy.borrow_fallthrough()
                    != super::BorrowFallthrough::Reachable
                || instruction.jump_policy.borrow_empty_block()
                || instruction.location.line < 0
            {
                return false;
            }
            if membership_fold {
                membership_folds += 1;
                if membership_folds > 1 {
                    return false;
                }
                folded_slots +=
                    usize::from(instruction.local_policy.hidden_exception_owner_slots());
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
                    let paired = if fallthrough_head {
                        indexes().has_fallthrough_setup_pair(self, all, &labels, *instruction)
                    } else {
                        self.setup_nop_ranges.iter().any(|(start, end)| {
                            let (Some(&start_at), Some(&end_at)) =
                                (positions.get(start), positions.get(end))
                            else {
                                return false;
                            };
                            if start_at >= end_at
                                || !indexes().has_unique_label(*start)
                                || !indexes().has_unique_label(*end)
                            {
                                return false;
                            }
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
                        })
                    };
                    let authored_pass = !paired
                        && comprehension_restore_count > 0
                        && index > comprehension_restore_count
                        && indexes().has_unique_authored_pass(
                            all,
                            &self.original_pass_nops,
                            instruction,
                        );
                    if (!paired && !authored_pass)
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
                    if paired {
                        null_setup_cells.insert(index + folded_slots);
                        setup_count += 1;
                    }
                }
                LOAD_FAST => {
                    // CP's superinstruction operands each occupy four bits.
                    // Larger local indices retain separate original records.
                    if matches!(instruction.operand(), Operand::Value(0..=15))
                        && instructions.get(index + 1).is_some_and(|next| {
                            next.opcode == LOAD_FAST
                                && next.location.line == instruction.location.line
                                && matches!(next.operand(), Operand::Value(0..=15))
                        })
                    {
                        return false;
                    }
                }
                LOAD_DEREF | STORE_DEREF => {
                    // Cell accesses keep their own records throughout this array;
                    // local-load/store superinstructions cannot compact them.
                }
                BUILD_TUPLE | MAKE_FUNCTION | SET_FUNCTION_ATTRIBUTE
                    if closure_members.contains(&index) => {}
                MAKE_FUNCTION | SET_FUNCTION_ATTRIBUTE
                    if annotated_function_members.contains(&index) => {}
                BUILD_TUPLE
                    if comprehension_restore_count > 0
                        && index > comprehension_restore_count
                        && let Operand::Value(count @ 1..) = instruction.operand()
                        && let Some(previous) = instructions.get(index - 1)
                        // Consume the restored runtime result, or follow an
                        // independently certified operation that cannot fold.
                        && let suffix = &instructions[comprehension_restore_count + 1..index]
                        && ((suffix.iter().all(|part| matches!(part.opcode, LOAD_FAST | LOAD_CONST | LOAD_SMALL_INT))
                                && usize::try_from(count).is_ok_and(|count| count > suffix.len()))
                            || matches!(previous.opcode, LIST_EXTEND | CALL_INTRINSIC_1))
                        && instruction_item_positions[index]
                            - instruction_item_positions[comprehension_restore_count]
                            == index - comprehension_restore_count
                        && let (Some(before), Some(after)) =
                            (previous.depth_after(), instruction.depth_after())
                        && before.checked_sub(count - 1) == Some(after)
                        && instructions
                            .get(index + 1)
                            .is_some_and(|next| next.opcode != UNPACK_SEQUENCE) =>
                {
                    expanded_collection = true;
                }
                BUILD_LIST => {
                    // Empty local lists retain their existing certificate. Nonempty
                    // lists below CPython's three-item constant-fold threshold also
                    // keep every record unless used for iteration or containment.
                    let unchanged =
                        instructions.get(index + 1).is_some_and(|next| {
                            match instruction.operand() {
                                Operand::Value(0) => {
                                    next.opcode == STORE_FAST
                                        // These retained operations block the
                                        // constant-list-to-tuple fold from reaching
                                        // this empty list.
                                        || (comprehension_restore_count > 0
                                            && index > comprehension_restore_count
                                            && instruction_item_positions[index]
                                                - instruction_item_positions[comprehension_restore_count]
                                                == index - comprehension_restore_count
                                            && instruction_item_positions[index + 1]
                                                == instruction_item_positions[index] + 1
                                            && (matches!(next.opcode, LOAD_FAST | LIST_EXTEND)
                                                || (next.opcode == BUILD_LIST
                                                    && next.operand() == Operand::Value(0)
                                                    && instructions.get(index + 2).is_some_and(|next| next.opcode == LIST_EXTEND)
                                                    && instruction_item_positions[index + 2]
                                                        == instruction_item_positions[index] + 2)
                                                || instructions.get(index + 1..index + 4)
                                                    .is_some_and(|parts| {
                                                        matches!(parts[0].opcode, LOAD_CONST | LOAD_SMALL_INT)
                                                            && matches!(parts[1].opcode, LOAD_CONST | LOAD_SMALL_INT)
                                                            && parts[2].opcode == BUILD_LIST
                                                            && parts[2].operand() == Operand::Value(2)
                                                            && instruction_item_positions[index + 3]
                                                                == instruction_item_positions[index] + 3
                                                    })))
                                }
                                Operand::Value(1..=2) => {
                                    !matches!(next.opcode, GET_ITER | CONTAINS_OP)
                                }
                                _ => false,
                            }
                        });
                    if !unchanged {
                        return false;
                    }
                    if instruction.operand() == Operand::Value(0)
                        && instructions[index + 1].opcode != STORE_FAST
                    {
                        expanded_collection = true;
                    }
                }
                BUILD_MAP => {
                    // CPython does not fold BUILD_MAP; after the certified restore
                    // the comprehension supplies a runtime value to this array.
                    if comprehension_restore_count == 0
                        || index <= comprehension_restore_count
                        || !matches!(instruction.operand(), Operand::Value(1..))
                    {
                        return false;
                    }
                }
                DICT_UPDATE => {
                    // The update is a standalone, non-swappable record. Restrict
                    // it to the authenticated comprehension continuation.
                    if comprehension_restore_count == 0
                        || index <= comprehension_restore_count
                        || !matches!(instruction.operand(), Operand::Value(1..))
                    {
                        return false;
                    }
                }
                LIST_EXTEND | LIST_APPEND => {
                    // The restored runtime value or a subsequent simple load feeds
                    // this list operation. A continuous prefix excludes copied arrays.
                    if comprehension_restore_count == 0
                        || index <= comprehension_restore_count
                        || instruction_item_positions[index]
                            - instruction_item_positions[comprehension_restore_count]
                            != index - comprehension_restore_count
                        || !matches!(instruction.operand(), Operand::Value(1..))
                        || (index != comprehension_restore_count + 1
                            && !matches!(
                                instructions[index - 1].opcode,
                                LOAD_FAST | LOAD_CONST | LOAD_SMALL_INT | BUILD_LIST | BUILD_TUPLE
                            )
                            && !(instruction.opcode == LIST_APPEND
                                && instructions[index - 1].opcode == LIST_EXTEND
                                && instructions[index - 1].operand() == Operand::Value(1)))
                        || (instruction.opcode == LIST_APPEND
                            && index == comprehension_restore_count + 1)
                    {
                        return false;
                    }
                    let Operand::Value(depth) = instruction.operand() else {
                        unreachable!()
                    };
                    let (Some(before), Some(after)) = (
                        instructions[index - 1].depth_after(),
                        instruction.depth_after(),
                    ) else {
                        return false;
                    };
                    if before.checked_sub(1) != Some(after) || after < depth {
                        return false;
                    }
                    expanded_collection = true;
                }
                CALL_INTRINSIC_1 | YIELD_VALUE | RESUME | POP_TOP
                    if yield_members.contains(&index) =>
                {
                    expanded_collection = true;
                }
                CALL_INTRINSIC_1 => {
                    // LIST_EXTEND prevents constant-list-to-tuple folding; an
                    // immediately iterated result would instead be optimized away.
                    if comprehension_restore_count == 0
                        || index <= comprehension_restore_count
                        || instruction.operand()
                            != Operand::Value(UnaryIntrinsic::LIST_TO_TUPLE.argument())
                        || instruction_item_positions[index]
                            - instruction_item_positions[comprehension_restore_count]
                            != index - comprehension_restore_count
                        || instructions[comprehension_restore_count + 1..index]
                            .iter()
                            .rev()
                            .find(|part| matches!(part.opcode, LIST_EXTEND | BUILD_LIST))
                            .is_none_or(|part| part.opcode != LIST_EXTEND)
                        || !matches!(instructions[index - 1].opcode, LIST_APPEND | LIST_EXTEND)
                        || instructions
                            .get(index + 1)
                            .is_none_or(|next| next.opcode == GET_ITER)
                        || instructions[index - 1].depth_after() != instruction.depth_after()
                        || instruction.depth_after().is_none()
                    {
                        return false;
                    }
                    expanded_collection = true;
                }
                STORE_FAST => {
                    // Certified comprehension restores are still MAYBE_NULL
                    // pseudo instructions during CP superinstruction insertion.
                    // Ordinary adjacent stores/loads can fuse on the same line.
                    if !(1..=comprehension_restore_count).contains(&index)
                        && matches!(instruction.operand(), Operand::Value(0..=15))
                        && instructions.get(index + 1).is_some_and(|next| {
                            matches!(next.opcode, LOAD_FAST | STORE_FAST)
                                && next.location.line == instruction.location.line
                                && matches!(next.operand(), Operand::Value(0..=15))
                        })
                    {
                        return false;
                    }
                }
                LOAD_GLOBAL => {
                    // CP removes the separately emitted PUSH_NULL before
                    // marker insertion. Keep its original cell in the SETUP
                    // coordinates; a folded prefix is accepted only when the
                    // append lands on an authenticated NULL SETUP cell below.
                    if matches!(instruction.operand(), Operand::Value(value) if value & 1 != 0) {
                        folded_slots += 1;
                    }
                }
                PUSH_NULL => {
                    // Attribute, local, and module-name calls retain this separate record;
                    // a direct global call instead folds it into LOAD_GLOBAL.
                    if instructions
                        .get(index.wrapping_sub(1))
                        .is_none_or(|previous| {
                            !matches!(previous.opcode, LOAD_ATTR | LOAD_FAST | LOAD_NAME)
                        })
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
                    // A trailing literal after one runtime interpolation also
                    // retains all records. Require the complete three-piece
                    // producer, not an arbitrary constant suffix.
                    let trailing_literal = instruction.operand() == Operand::Value(3)
                        && index.checked_sub(4).is_some_and(|start| {
                            let [prefix, value, format, suffix] = &instructions[start..index]
                            else {
                                return false;
                            };
                            prefix.opcode == LOAD_CONST
                                && matches!(
                                    value.opcode,
                                    LOAD_FAST | LOAD_NAME | LOAD_ATTR | CALL | CALL_KW
                                )
                                && format.opcode == FORMAT_SIMPLE
                                && format.operand() == Operand::Value(0)
                                && suffix.opcode == LOAD_CONST
                                && [prefix, value, format, suffix]
                                    .iter()
                                    .all(|part| part.location.line == instruction.location.line)
                        });
                    // Large joined-string lowering remains outside this proof.
                    if !matches!(instruction.operand(), Operand::Value(2..=30))
                        || (!trailing_literal
                            && instructions
                                .get(index.wrapping_sub(1))
                                .is_none_or(|previous| previous.opcode != FORMAT_SIMPLE))
                    {
                        return false;
                    }
                }
                crate::target::opcodes::UNPACK_SEQUENCE if empty_class_success && index == 0 => {}
                POP_TOP if empty_class_success && index == 1 => {}
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
                CONTAINS_OP => {
                    if !matches!(instruction.operand(), Operand::Value(0 | 1))
                        || !index
                            .checked_sub(1)
                            .is_some_and(|previous| original_folds.contains(&previous))
                    {
                        return false;
                    }
                }
                LOAD_NAME | LOAD_ATTR | LOAD_CONST | STORE_NAME | CALL | IMPORT_NAME => {}
                crate::target::opcodes::SWAP if comprehension_restore_count > 0 && index == 0 => {}
                _ => return false,
            }
        }
        if expanded_collection {
            // A retained SETUP is wrapped in synthetic exception-range labels;
            // they do not split its original array. Admit only the complete
            // adjacent quartet and reject any independently authored boundary.
            let mut item_index = instruction_item_positions[comprehension_restore_count] + 1;
            let marker_index = instruction_item_positions[instructions.len() - 1];
            while item_index < marker_index {
                if matches!(prefix[item_index], Item::Instruction(_)) {
                    item_index += 1;
                    continue;
                }
                let Some(
                    [
                        Item::Label(start),
                        Item::Instruction(nop),
                        Item::Label(end),
                        Item::Label(setup),
                    ],
                ) = prefix.get(item_index..item_index + 4)
                else {
                    return false;
                };
                let mut setup_matches = self
                    .setups
                    .iter()
                    .filter(|(label, _)| [start, end, setup].contains(&label));
                if item_index + 4 > marker_index
                    || nop.opcode != NOP
                    || nop.operand() != Operand::Value(0)
                    || nop.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
                    || !self.setup_nop_ranges.contains(&(*start, *end))
                    || self
                        .setup_nop_ranges
                        .iter()
                        .filter(|(candidate_start, candidate_end)| {
                            [start, end, setup].contains(&candidate_start)
                                || [start, end, setup].contains(&candidate_end)
                        })
                        .count()
                        != 1
                    || self.original_fallthrough_setups.contains(start)
                    || setup_matches.next() != Some(&(*setup, Some(nop.location)))
                    || setup_matches.next().is_some()
                    || [start, end, setup].iter().any(|label| {
                        !indexes().has_unique_label(**label)
                            || targets.contains(*label)
                            || self.block_boundaries.contains(*label)
                            || self.source_boundaries.contains(*label)
                    })
                {
                    return false;
                }
                item_index += 4;
            }
        }
        // Retained SETUPs need an authored NOP proof. A certified comprehension
        // restore followed by a non-swappable operation establishes its array head,
        // including a condition reached without crossing a new SETUP.
        (setup_count > 0 || comprehension_restore_count > 0)
            // With no folds, the append is fresh. Certified membership/global
            // folds may instead leave the retained SETUP in that exact array cell.
            && (folded_slots == 0 || null_setup_cells.contains(&(instructions.len() - 1)))
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
    fn uncompacted_conditional_null_markers<const INDEX_ORIGINS: bool>(
        &self,
        jump_targets: &FxHashSet<Label>,
        unique_labels: &OnceCell<UniqueLabelPositions>,
    ) -> FxHashSet<usize> {
        use crate::target::opcodes::{LOAD_FAST, LOAD_FAST_BORROW, LOAD_FAST_CHECK};

        let mut result = FxHashSet::default();
        let fallthrough_setup_labels = OnceCell::<FxHashSet<Label>>::new();
        let fallthrough_handler_labels = OnceCell::<FxHashSet<Label>>::new();
        let fallthrough_copy_internal_labels = OnceCell::<FxHashSet<Label>>::new();
        let origins = &self
            .exception_owner_provenance
            .uncompacted_conditional_arrays;
        for origin in origins {
            let Some(first) = origin.prefix.first() else {
                continue;
            };
            let entry = match first {
                Item::Label(entry) => Some(*entry),
                Item::Instruction(unpack)
                    if unpack.opcode == crate::target::opcodes::UNPACK_SEQUENCE
                        && unpack.operand() == Operand::Value(0)
                        && origin.source_entry.is_none()
                        && origin.comprehension_entry.is_none() =>
                {
                    None
                }
                Item::Instruction(_) => continue,
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
                && (entry != Some(original_entry)
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
            let (start, mut instruction_position) = if INDEX_ORIGINS {
                if let Some(entry) = entry
                    && (origins.len() > 1 || unique_labels.get().is_some())
                {
                    let Some(position) = unique_labels
                        .get_or_init(|| UniqueLabelPositions::new(&self.items))
                        .get(entry)
                    else {
                        continue;
                    };
                    position
                } else {
                    // A single origin needs no index allocation. The sole non-label origin
                    // is an empty class pattern; it still requires exact item uniqueness.
                    let mut instruction_position = 0;
                    let mut matches = self.items.iter().enumerate().filter_map(|(index, item)| {
                        let position = instruction_position;
                        instruction_position += usize::from(matches!(item, Item::Instruction(_)));
                        (item == first).then_some((index, position))
                    });
                    let Some(position) = matches.next() else {
                        continue;
                    };
                    if matches.next().is_some() {
                        continue;
                    }
                    position
                }
            } else {
                let starts = self
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| (item == first).then_some(index))
                    .collect::<Vec<_>>();
                let [start] = starts.as_slice() else { continue };
                (*start, 0)
            };
            let fallthrough_unsafe_alias = |label: &Label| {
                let provenance = &self.exception_owner_provenance;
                provenance.removed_pop_block_boundaries.contains(label)
                    || provenance.omitted_nop_counts.contains_key(label)
                    || provenance.omitted_nop_locations.contains_key(label)
                    || provenance.fusion_boundaries.contains(label)
                    || provenance.normal_try_copy_blocks.contains(label)
                    || provenance.normal_try_copy_inline_labels.contains(label)
                    || fallthrough_copy_internal_labels
                        .get_or_init(|| {
                            provenance
                                .normal_try_copy_internal_labels
                                .iter()
                                .copied()
                                .collect()
                        })
                        .contains(label)
                    || provenance
                        .normal_try_copy_jump_nop_successors
                        .contains(label)
            };
            let is_setup_boundary = |label: &Label| {
                if origin.fallthrough.is_some() {
                    fallthrough_setup_labels
                        .get_or_init(|| {
                            self.exception_owner_provenance
                                .setups
                                .iter()
                                .map(|(setup, _)| *setup)
                                .collect()
                        })
                        .contains(label)
                } else {
                    self.exception_owner_provenance
                        .setups
                        .iter()
                        .any(|(setup, _)| setup == label)
                }
            };
            let is_handler_boundary = |label: &Label| {
                if origin.fallthrough.is_some() {
                    fallthrough_handler_labels
                        .get_or_init(|| {
                            self.exception_regions
                                .iter()
                                .map(|region| region.target)
                                .collect()
                        })
                        .contains(label)
                } else {
                    self.exception_regions
                        .iter()
                        .any(|region| region.target == *label)
                }
            };
            if let Some([condition, marker]) = origin.fallthrough {
                if start == 0
                    || self.items[start - 1] != Item::Instruction(marker)
                    || leading_labels.iter().any(
                        |item| matches!(item, Item::Label(label) if jump_targets.contains(label)),
                    )
                {
                    continue;
                }
                let mut before_marker = start - 1;
                while let Some(Item::Label(label)) = before_marker
                    .checked_sub(1)
                    .and_then(|index| self.items.get(index))
                {
                    if jump_targets.contains(label)
                        || (origin.fallthrough.is_some() && fallthrough_unsafe_alias(label))
                        || self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                        || self.preserved_block_boundaries.contains(label)
                        || is_setup_boundary(label)
                        || is_handler_boundary(label)
                    {
                        break;
                    }
                    before_marker -= 1;
                }
                if before_marker == 0
                    || self.items[before_marker - 1] != Item::Instruction(condition)
                {
                    continue;
                }
            }
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
            let mut cursor = start;
            let mut valid = true;
            for (original_index, expected) in origin.prefix.iter().enumerate() {
                while let Some(Item::Label(label)) = self.items.get(cursor)
                    && !original_labels.contains(label)
                {
                    // Range-only aliases carry no original instruction cells.
                    if jump_targets.contains(label)
                        || (origin.fallthrough.is_some() && fallthrough_unsafe_alias(label))
                        || self
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label)
                        || self
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label)
                        || self.preserved_block_boundaries.contains(label)
                        || is_setup_boundary(label)
                        || is_handler_boundary(label)
                    {
                        valid = false;
                        break;
                    }
                    cursor += 1;
                }
                let same = match (expected, self.items.get(cursor)) {
                    (Item::Label(expected), Some(Item::Label(current))) => expected == current,
                    (Item::Instruction(expected), Some(Item::Instruction(current))) => {
                        // Borrowing and local initialization checks rewrite only the
                        // opcode after CP's slot selection. Every other captured policy,
                        // location, operand, and depth must retain its original identity.
                        let mut expected = *expected;
                        if expected.opcode == LOAD_FAST
                            && matches!(current.opcode, LOAD_FAST_BORROW | LOAD_FAST_CHECK)
                        {
                            expected.opcode = current.opcode;
                        }
                        if entry.is_none()
                            && original_index == 1
                            && expected.opcode == POP_TOP
                            && expected.fusion_policy.location_from_next()
                        {
                            // The captured next record has this exact location.
                            // resolve_next_locations only clears this proven bit.
                            expected.fusion_policy.clear_location_from_next();
                        }
                        expected == *current
                    }
                    _ => false,
                };
                if !valid || !same {
                    valid = false;
                    break;
                }
                if INDEX_ORIGINS {
                    // Any additional accepted range aliases above are labels only.
                    instruction_position += usize::from(matches!(expected, Item::Instruction(_)));
                }
                cursor += 1;
            }
            if valid {
                let marker = if INDEX_ORIGINS {
                    instruction_position - 1
                } else {
                    self.items[..cursor]
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count()
                        - 1
                };
                result.insert(marker);
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
        label_positions: &OwnerLabelPositions,
    ) -> (
        FxHashMap<usize, usize>,
        FxHashSet<Label>,
        FxHashMap<usize, usize>,
    ) {
        let provenance = &self.exception_owner_provenance;
        let mut slots = FxHashMap::default();
        let mut aliases = FxHashSet::default();
        let mut method_donors = FxHashMap::default();
        for (origin, pop_count, requires_handler_pair, returned_value, method_cleanup) in provenance
            .named_handler_pop_pairs
            .iter()
            .map(|origin| (origin, 2, false, None, false))
            .chain(provenance.copied_handler_finally_pops.iter().map(|origin| {
                let pops = origin
                    .items
                    .iter()
                    .filter(|item| {
                        matches!(item, Item::Instruction(instruction)
                        if instruction.fusion_policy.is_converted_pop_block())
                    })
                    .count();
                (origin, pops, true, None, false)
            }))
            .chain(provenance.copied_method_cleanup_pop_spans.iter().filter_map(|origin| {
                let pops = origin.items.iter().filter(|item| matches!(item,
                    Item::Instruction(record) if record.fusion_policy.is_converted_pop_block())).count();
                (matches!(origin.items.last(), Some(Item::Instruction(record)) if matches!(record.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE))
                    && (3..=4).contains(&pops)).then_some((origin, pops, false, None, true))
            }))
            .chain(
                provenance
                    .iterator_return_pop_spans
                    .iter()
                    .map(|(value, origin)| (origin, 2, false, Some(*value), false)),
            )
        {
            let Some(&array) = label_positions.get(&origin.entry) else {
                continue;
            };
            if requires_handler_pair
                && (!(1..=2).contains(&pop_count) || !slots.values().any(|entry| *entry == array))
            {
                continue;
            }
            if provenance.is_normal_try_copy_label(origin.entry)
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
                    provenance.is_normal_try_copy_label(*label)
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
            let mut pending_pops = 0_u16;
            let mut removed_pops = 0;
            for item in &origin.items {
                match *item {
                    Item::Instruction(instruction)
                        if instruction.fusion_policy.is_converted_pop_block() =>
                    {
                        pending_pops += 1;
                        removed_pops += 1;
                    }
                    Item::Instruction(mut instruction) => {
                        if pending_pops > 0 {
                            successors.push(expected.len());
                            instruction
                                .local_policy
                                .add_hidden_exception_owner_slots(pending_pops);
                            pending_pops = 0;
                        }
                        expected.push(Item::Instruction(instruction));
                    }
                    label @ Item::Label(_) => expected.push(label),
                }
            }
            if pending_pops != 0
                || removed_pops != pop_count
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
            if let Some(mut value) = returned_value {
                let Some(Item::Instruction(current_value)) =
                    begin.checked_sub(1).and_then(|index| self.items.get(index))
                else {
                continue;
                };
                if current_value.opcode == super::LOAD_FAST_BORROW {
                    value.opcode = super::LOAD_FAST_BORROW;
                }
                if *current_value != value
                    || !provenance.source_boundaries.contains(&origin.entry)
                    || provenance
                        .retired_normal_try_copy_labels
                        .contains(&origin.entry)
                    || labels.iter().any(|label| {
                        provenance.retired_normal_try_copy_labels.contains(label)
                            || provenance.fusion_boundaries.contains(label)
                            || provenance.omitted_nop_counts.contains_key(label)
                    })
                {
                continue;
                }
            }
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
            for (offset, successor) in successors.into_iter().enumerate() {
                let index = first
                    + expected[..successor]
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count();
                slots.insert(index, array);
                if method_cleanup && offset == 0 { method_donors.insert(index + 3, array); }
            }
            aliases.extend(labels);
        }
        (slots, aliases, method_donors)
    }

    /// Resolves append slots in the first captured normal-POP instruction array.
    fn retained_normal_pop_terminators(
        &self,
        jump_targets: &FxHashSet<Label>,
        label_positions: &OwnerLabelPositions,
    ) -> FxHashMap<usize, Option<usize>> {
        let mut result = FxHashMap::default();
        if self
            .exception_owner_provenance
            .retained_normal_pop_blocks
            .is_empty()
        {
            return result;
        }
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
            let mut previous = Some(marker);
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
                tail.instruction(instruction, previous);
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
                previous = Some(instruction);
            }
        }
        result
    }

    /// Matches a captured normal-finally method test after only POP removal and borrowing.
    fn normal_finally_method_none_prefix(
        &self,
        condition_index: usize,
        array_start: usize,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<(usize, u16)> {
        let load_index = condition_index.checked_sub(3)?;
        let load = *instructions[load_index];
        let removed_pop = load.local_policy.hidden_exception_owner_slots();
        if removed_pop > 1 {
            return None;
        }
        let mut matches = self
            .exception_owner_provenance
            .normal_finally_method_none_prefixes
            .iter()
            .filter(|origin| {
                let mut expected_load = origin[1];
                expected_load
                    .local_policy
                    .add_hidden_exception_owner_slots(removed_pop);
                if load.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                    expected_load.opcode = load.opcode;
                }
                load == expected_load
                    && instructions[load_index + 1] == &origin[2]
                    && instructions[load_index + 2] == &origin[3]
                    && instructions[condition_index] == &origin[4]
            });
        let origin = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        let prefix_start = if removed_pop == 0 {
            let pop_index = load_index.checked_sub(1)?;
            let pop = *instructions[pop_index];
            let mut expected_pop = origin[0];
            expected_pop.location = pop.location;
            if pop.exception_policy() == super::ExceptionPolicy::AlwaysAbsent {
                expected_pop.set_exception_policy(super::ExceptionPolicy::AlwaysAbsent);
            }
            if pop != expected_pop {
                return None;
            }
            pop_index
        } else {
            load_index
        };
        if array_start > prefix_start
            || jump_targets.iter().any(|label| {
                positions.get(label).is_some_and(|position| {
                    prefix_start < *position && *position <= condition_index + 1
                })
            })
        {
            return None;
        }
        Some((load_index, removed_pop))
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
            let Some(original_pop_at) = origin
                .prefix
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            let mut expected = origin.prefix.clone();
            // When an incoming jump is retained as the immediately preceding NOP,
            // CPython propagates its location to the leading POP_BLOCK and keeps
            // both that record and the differently located SETUP NOP. Serc folds
            // the POP into the setup instead. Reconcile only the captured pair,
            // with an exact retained-jump witness and no intervening rewrites.
            let mut compound_try_exit = false;
            let removed_pop = retained_setup_nop.is_some_and(|setup| {
                let Item::Instruction(pop) = origin.prefix[original_pop_at] else {
                    return false;
                };
                let Some(Item::Instruction(previous)) =
                    start.checked_sub(1).and_then(|index| self.items.get(index))
                else {
                    return false;
                };
                let mut located = origin
                    .incoming
                    .iter()
                    .filter(|jump| jump.location.line >= 0);
                let Some(jump) = located.next() else {
                    return false;
                };
                // Cold-handler placement also turns an ordinary try's normal
                // exit into a NOP. Authenticate that original exit label and the
                // complete converted record before using it as the line donor.
                let mut retired_jump = *jump;
                retired_jump.opcode = NOP;
                retired_jump.set_operand(Operand::Value(0));
                if self
                    .exception_owner_provenance
                    .ownerless_compound_try_nops
                    .iter()
                    .any(|(source, target, original)| {
                        *target == origin.entry
                            && original == jump
                            && start
                                .checked_sub(2)
                                .is_some_and(|index| self.items[index] == Item::Label(*source))
                    })
                {
                    // Redundant-jump removal already restores this captured POP_BLOCK owner.
                    retired_jump.set_exception_owner(NormalizedExceptionOwner::Absent);
                }
                compound_try_exit = *previous == retired_jump
                    && start.checked_sub(2).is_some_and(|index| {
                        let Item::Label(exit) = self.items[index] else {
                            return false;
                        };
                        let mut exits = self
                            .exception_owner_provenance
                            .compound_try_owner_exits
                            .iter()
                            .filter(|(_, target)| *target == origin.entry);
                        exits.next() == Some(&(exit, origin.entry))
                            && exits.next().is_none()
                            && self
                                .items
                                .iter()
                                .filter(|item| **item == Item::Label(exit))
                                .count()
                                == 1
                    });
                located.next().is_none()
                    && (jump.jump_policy.retains_nop_for(jump.operand()) || compound_try_exit)
                    && previous.opcode == NOP
                    && previous.location == jump.location
                    && previous.depth_after() == jump.depth_after()
                    && previous.local_policy == jump.local_policy
                    && previous.no_location_policy == jump.no_location_policy
                    && previous.jump_policy == jump.jump_policy
                    && (previous.exception_policy() == jump.exception_policy()
                        || (compound_try_exit
                            && previous.exception_policy() == retired_jump.exception_policy()))
                    && previous.local_policy.hidden_exception_owner_slots() == 0
                    && previous.pre_fold_extra_instructions() == 0
                    && setup.location.line >= 0
                    && setup.location.line != previous.location.line
                    && setup.local_policy.hidden_exception_owner_slots() == 0
                    && setup.pre_fold_extra_instructions() == 0
                    && pop.opcode == NOP
                    && pop.fusion_policy.is_converted_pop_block()
                    && pop.location.line < 0
                    && pop.exception_policy() == super::ExceptionPolicy::EXCLUDE_ALWAYS
                    && pop.local_policy.hidden_exception_owner_slots() == 0
                    && pop.pre_fold_extra_instructions() == 0
                    && origin.prefix[original_pop_at + 1..]
                        .iter()
                        .find_map(|item| match item {
                            Item::Instruction(instruction) => Some(*instruction),
                            Item::Label(_) => None,
                        })
                        == Some(setup)
            });
            if removed_pop {
                expected.remove(original_pop_at);
                let Some(Item::Instruction(setup)) = expected[original_pop_at..]
                    .iter_mut()
                    .find(|item| matches!(item, Item::Instruction(_)))
                else {
                    continue;
                };
                setup.local_policy.add_hidden_exception_owner_slots(1);
            }
            let (current, end) =
                if authored_break_location.is_some() || retained_setup_nop.is_some() {
                    let original_labels = labels.iter().copied().collect::<FxHashSet<_>>();
                    let mut cursor = *start;
                    let mut current = Vec::with_capacity(expected.len());
                    let mut valid = true;
                    for _ in &expected {
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
                    let Some(current) = self.items.get(*start..*start + expected.len()) else {
                        continue;
                    };
                    (current.to_vec(), *start + expected.len())
                };
            let Some(marker_at) = expected
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            let (Item::Instruction(marker), Item::Instruction(original_marker)) =
                (current[marker_at], &mut expected[marker_at])
            else {
                continue;
            };
            // Location propagation changes the retained POP_BLOCK's line, not its record identity.
            if !removed_pop {
                original_marker.location = marker.location;
            }
            // Normal-finally lowering can refine this excluded POP_BLOCK's owner
            // after capture without changing its original instruction cell.
            let excluded_pop_owner_refined = !removed_pop
                && retained_setup_nop.is_some()
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
            }
            // Borrowing follows marker normalization and changes only the opcode,
            // including in an ordinary finalizer array. Keep the operand, location,
            // policies, depth, labels, and every other original record exact.
            for (expected, current) in expected.iter_mut().zip(&current) {
                if let (Item::Instruction(expected), Item::Instruction(current)) =
                    (expected, current)
                    && expected.opcode == LOAD_FAST
                    && current.opcode == LOAD_FAST_BORROW
                {
                    expected.opcode = LOAD_FAST_BORROW;
                }
            }
            if current != expected
                || marker.opcode != NOP
                || (!removed_pop
                    && (!marker.fusion_policy.is_converted_pop_block()
                        || (marker.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
                            && !excluded_pop_owner_refined)
                        || marker.local_policy.hidden_exception_owner_slots() != 0))
                || marker.pre_fold_extra_instructions() != 0
                || marker.jump_policy.borrow_empty_block()
                || marker.no_location_policy.borrow_fallthrough()
                    != super::BorrowFallthrough::Reachable
            {
                continue;
            }
            let mut tail = ExceptionOwnerTail::default();
            let mut previous = Some(&marker);
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
                tail.instruction(instruction, previous);
                previous = Some(instruction);
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
                if removed_pop && compound_try_exit {
                    // The same original exit retains its inner POP_BLOCK's line.
                    // Serc represents that ownerless slot with the retired jump NOP.
                    let logical_start = self.items[..*start]
                        .iter()
                        .filter(|item| matches!(item, Item::Instruction(_)))
                        .count();
                    result.insert(logical_start - 1);
                }
            }
        }
        result
    }

    #[cfg(test)]
    pub(super) fn retained_setup_nops(
        &self,
        label_positions: &FxHashMap<Label, usize>,
    ) -> FxHashSet<usize> {
        let instructions: Vec<_> = self
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect();
        let label_positions = OwnerLabelPositions::from_hashed(self.items.len(), label_positions);
        self.retained_setup_nops_with_instructions(&label_positions, &instructions)
    }

    /// Identifies SETUP records that still occupy their authored one-instruction ranges.
    fn retained_setup_nops_with_instructions(
        &self,
        label_positions: &OwnerLabelPositions,
        instructions: &[&Instruction],
    ) -> FxHashSet<usize> {
        if self.exception_owner_provenance.setup_nop_ranges.is_empty() {
            return FxHashSet::default();
        }
        let setup_locations: Vec<_> = self
            .exception_owner_provenance
            .setups
            .iter()
            .filter_map(|(label, location)| Some((*label_positions.get(label)?, (*location)?)))
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
        label_positions: &OwnerLabelPositions,
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
        label_positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
        retained_setup_nops: &FxHashSet<usize>,
    ) -> FxHashSet<usize> {
        let provenance = &self.exception_owner_provenance;
        let mut result = FxHashSet::default();
        if retained_setup_nops.is_empty() {
            return result;
        }
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
                || provenance.is_normal_try_copy_label(*label)
        };
        for origin in &provenance.source_none_conditions {
            if !provenance.source_boundaries.contains(&origin.entry)
                || !provenance.block_boundaries.contains(&origin.entry)
                || provenance.is_normal_try_copy_label(origin.entry)
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
                .source_none_entry_prefix(
                    origin,
                    *begin,
                    *end,
                    label_positions,
                    &boundary,
                    &|label| provenance.is_normal_try_copy_label(*label),
                )
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
                                    || !previous.is_some_and(|instruction: &Instruction| {
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
                        tail.instruction(instruction, previous);
                        if tail.insertions != 0 || tail.post_compaction_fusions != 0 {
                            valid = false;
                            break;
                        }
                        previous = Some(instruction);
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

    /// Proves the retained SETUP's NULL cell after exact late manager-SETUP compaction.
    fn retained_setup_value_null_markers(
        &self,
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
        retained_setup_nops: &FxHashSet<usize>,
        value_jump_positions: &FxHashMap<Label, usize>,
    ) -> FxHashSet<usize> {
        let provenance = &self.exception_owner_provenance;
        let mut result = FxHashSet::default();
        for origin in &provenance.retained_setup_value_arrays {
            let Some(&marker_index) = value_jump_positions.get(&origin.value_entry) else {
                continue;
            };
            if !provenance.source_boundaries.contains(&origin.entry)
                || !provenance.block_boundaries.contains(&origin.entry)
                || provenance
                    .retained_setup_value_arrays
                    .iter()
                    .filter(|other| other.value_entry == origin.value_entry)
                    .count()
                    != 1
            {
                continue;
            }
            let mut starts =
                self.items.iter().enumerate().filter_map(|(index, item)| {
                    (*item == Item::Label(origin.entry)).then_some(index)
                });
            let Some(begin) = starts.next() else {
                continue;
            };
            let Some(current) = self.items.get(begin..begin + origin.prefix.len()) else {
                continue;
            };
            if starts.next().is_some()
                || current
                    .iter()
                    .zip(&origin.prefix)
                    .any(|(actual, expected)| {
                        let mut expected = *expected;
                        if let (Item::Instruction(actual), Item::Instruction(expected)) =
                            (actual, &mut expected)
                            && expected.opcode == crate::target::opcodes::LOAD_FAST
                            && actual.opcode == crate::target::opcodes::LOAD_FAST_BORROW
                        {
                            expected.opcode = actual.opcode;
                        }
                        *actual != expected
                    })
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
                || labels.iter().any(|label| {
                    self.items
                        .iter()
                        .filter(|item| **item == Item::Label(*label))
                        .count()
                        != 1
                        || provenance.retired_normal_try_copy_labels.contains(label)
                        || provenance.is_normal_try_copy_label(*label)
                        || provenance.removed_pop_block_boundaries.contains(label)
                        || provenance.omitted_nop_counts.contains_key(label)
                        || provenance.omitted_nop_locations.contains_key(label)
                        || (*label != origin.entry
                            && (provenance.block_boundaries.contains(label)
                                || provenance.source_boundaries.contains(label)
                                || self.preserved_block_boundaries.contains(label)
                                || jump_targets.contains(label)
                                || self
                                    .exception_regions
                                    .iter()
                                    .any(|region| region.target == *label)))
                })
                || provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| labels.contains(label))
                    .copied()
                    .collect::<Vec<_>>()
                    != origin.setups
            {
                continue;
            }
            let Some(&array_start) = positions.get(&origin.entry) else {
                continue;
            };
            let mut logical = array_start;
            let mut removed = 0;
            let mut null_cell = None;
            let mut previous = None;
            let mut tail = ExceptionOwnerTail::default();
            let mut valid = true;
            for item in &current[..current.len() - 1] {
                match item {
                    Item::Label(label) => {
                        if *label == origin.entry {
                            continue;
                        }
                        let setups = origin
                            .setups
                            .iter()
                            .filter(|(setup, _)| setup == label)
                            .collect::<Vec<_>>();
                        match setups.as_slice() {
                            [(_, None)] if provenance.synchronous_with_setups.contains(label) => {
                                removed += 1;
                            }
                            [(_, Some(location))] => {
                                if null_cell.is_some()
                                    || !logical
                                        .checked_sub(1)
                                        .is_some_and(|index| retained_setup_nops.contains(&index))
                                    || !previous.is_some_and(|instruction: &Instruction| {
                                        instruction.opcode == NOP
                                            && instruction.location == *location
                                    })
                                {
                                    valid = false;
                                    break;
                                }
                                null_cell = Some(logical - 1 + removed);
                            }
                            [] => {
                                let retained_endpoint =
                                    provenance.setup_nop_ranges.iter().any(|(start, end)| {
                                        (label == start || label == end)
                                            && labels.contains(start)
                                            && labels.contains(end)
                                            && positions.get(start).is_some_and(|index| {
                                                retained_setup_nops.contains(index)
                                                    && positions.get(end) == Some(&(index + 1))
                                            })
                                    });
                                let value_label = provenance
                                    .original_value_jump_prefixes
                                    .iter()
                                    .any(|(entry, prefix)| {
                                        *entry == origin.value_entry
                                            && prefix.contains(&Item::Label(*label))
                                    });
                                if !retained_endpoint
                                    && !value_label
                                    && !provenance.continuation_boundaries.contains(label)
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
                        let final_condition = logical + 1 == marker_index
                            && matches!(
                                instruction.opcode,
                                crate::target::opcodes::POP_JUMP_IF_FALSE
                                    | crate::target::opcodes::POP_JUMP_IF_TRUE
                            )
                            && matches!(instruction.operand(), Operand::Forward(_));
                        // A captured original SETUP remains a NULL-owner record when
                        // the compiler explicitly preserves that absence through copies.
                        if instruction.local_policy.hidden_exception_owner_slots() != 0
                            || instruction.pre_fold_extra_instructions() != 0
                            || (instruction.exception_policy()
                                != super::ExceptionPolicy::NoneUnchanged
                                && !(instruction.opcode == NOP
                                    && retained_setup_nops.contains(&logical)
                                    && instruction.exception_policy()
                                        == super::ExceptionPolicy::EXCLUDE_ALWAYS))
                            || instruction.no_location_policy.borrow_fallthrough()
                                != super::BorrowFallthrough::Reachable
                            || instruction.jump_policy.borrow_empty_block()
                            || instruction.fusion_policy.is_converted_pop_block()
                            || (instruction.opcode == NOP
                                && !retained_setup_nops.contains(&logical))
                            || (!matches!(instruction.operand(), Operand::Value(_))
                                && !final_condition)
                            || matches!(instruction.opcode, NOT_TAKEN | FOR_ITER | SEND)
                            || is_scope_exit(instruction.opcode)
                        {
                            valid = false;
                            break;
                        }
                        tail.instruction(instruction, previous);
                        if tail.insertions != 0 || tail.post_compaction_fusions != 0 {
                            valid = false;
                            break;
                        }
                        previous = Some(instruction);
                        logical += 1;
                    }
                }
            }
            // The exact Boolean expansion is already revalidated. Reinsert the
            // preceding erased manager SETUPs to recover the retained NOP's cell;
            // earlier folds occur before this final compaction and cancel out.
            if valid && removed > 0 && logical == marker_index && null_cell == Some(marker_index) {
                result.insert(marker_index);
            }
        }
        result
    }

    /// Recovers an original named-handler cleanup cell before a complete value Boolean.
    fn copied_handler_value_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<usize> {
        if tail.insertions != 0 || tail.late_removals != 0 || tail.post_compaction_fusions > 2 {
            return None;
        }
        let end = marker.checked_sub(4)?;
        let start = end.checked_sub(4)?;
        let provenance = &self.exception_owner_provenance;
        let mut origins = provenance.copied_handler_cleanups.iter().filter(|origin| {
            positions.get(&origin.start) == Some(&start) && positions.get(&origin.end) == Some(&end)
        });
        let origin = origins.next()?;
        if origins.next().is_some()
            || array > start
            || origin.instructions.len() != 4
            || origin.missing_null_pops != 0
            || origin.removed_prefix_slots != 3
            || origin.return_iterator.is_some()
            || origin.assignment_pop.is_some()
            || origin.continued_call.is_some()
            || origin.continued_attribute_none.is_some()
            || origin.return_nop.opcode != NOP
            || origin.return_nop.location != origin.instructions[0].location
            || [origin.start, origin.end].into_iter().any(|label| {
                provenance.retired_normal_try_copy_labels.contains(&label)
                    || provenance.normal_try_copy_blocks.contains(&label)
                    || provenance.normal_try_copy_internal_labels.contains(&label)
                    || self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(label))
                        .count()
                        != 1
            })
            || !provenance
                .removed_pop_block_boundaries
                .contains(&origin.end)
            || provenance
                .removed_pop_block_widths
                .get(&origin.end)
                .copied()
                .unwrap_or(1)
                != 1
            || jump_targets
                .iter()
                .chain(&provenance.source_boundaries)
                .chain(&provenance.block_boundaries)
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| array < *position && *position <= marker)
                })
        {
            return None;
        }
        for (offset, expected) in origin.instructions.iter().enumerate() {
            let mut expected = *expected;
            if offset == 0 {
                expected.local_policy.add_hidden_exception_owner_slots(3);
            }
            if *instructions[start + offset] != expected {
                return None;
            }
        }
        let load = instructions[end];
        if !matches!(
            load.opcode,
            crate::target::opcodes::LOAD_FAST | crate::target::opcodes::LOAD_FAST_BORROW
        ) || load.local_policy.hidden_exception_owner_slots() != 1
            || load.pre_fold_extra_instructions() != 0
            || load.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            || load.no_location_policy.borrow_fallthrough() != super::BorrowFallthrough::Reachable
            || load.jump_policy.borrow_empty_block()
            || load.location.line != instructions[marker].location.line
            || instructions[end + 1..marker].iter().any(|record| {
                record.local_policy.hidden_exception_owner_slots() != 0
                    || record.pre_fold_extra_instructions() != 0
                    || record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            })
        {
            return None;
        }
        let mut prefix_tail = ExceptionOwnerTail::default();
        for record in &instructions[array..start] {
            if record.local_policy.hidden_exception_owner_slots() != 0
                || record.pre_fold_extra_instructions() != 0
                || record.opcode == NOP
                || record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                || record.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
                || record.jump_policy.borrow_empty_block()
            {
                return None;
            }
            prefix_tail.instruction(record, None);
        }
        // The authored return NOP and two handler POPs precede the four exact
        // cleanup records; the finalizer POP follows them and transfers to LOAD_FAST.
        // COPY/TO_BOOL consume the entire newer fusion tail, leaving this early array.
        if prefix_tail.insertions != 0
            || prefix_tail.post_compaction_fusions != tail.post_compaction_fusions
            || prefix_tail.early_removals.checked_add(4) != Some(tail.early_removals)
        {
            return None;
        }
        let original_start = start
            .checked_add(prefix_tail.early_removals)?
            .checked_add(3)?;
        let offset = marker.checked_sub(original_start)?;
        if offset >= origin.instructions.len() {
            return None;
        }
        let region = self.uniform_exception_owner(start, end, positions)?;
        let owner = self.exception_regions[region];
        (!owner.is_scope_fallback && !owner.preserve_lasti).then_some(region)
    }

    /// Replays the exact finalizer NULL cell after a saved named-handler return.
    fn saved_named_handler_none_is_null(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> bool {
        if tail.early_removals != 4
            || tail.insertions != 0
            || tail.late_removals != 0
            || tail.post_compaction_fusions != 0
        {
            return false;
        }
        let provenance = &self.exception_owner_provenance;
        let mut matching = provenance
            .saved_named_handler_none_prefixes
            .iter()
            .filter(|origin| {
                origin.items.last() == Some(&Item::Instruction(*instructions[marker]))
                    && positions.get(&origin.entry) == Some(&array)
            });
        let Some(origin) = matching.next() else {
            return false;
        };
        if matching.next().is_some()
            || !provenance.source_boundaries.contains(&origin.entry)
            || !provenance.block_boundaries.contains(&origin.entry)
            || provenance
                .retired_normal_try_copy_labels
                .contains(&origin.entry)
            || provenance.normal_try_copy_blocks.contains(&origin.entry)
            || provenance
                .normal_try_copy_internal_labels
                .contains(&origin.entry)
            || array.checked_add(origin.unwind_offset + 7) != Some(marker)
            || jump_targets
                .iter()
                .chain(&provenance.source_boundaries)
                .chain(&provenance.block_boundaries)
                .chain(&self.preserved_block_boundaries)
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| array < *position && *position <= marker)
                })
            || provenance.setups.iter().any(|(label, _)| {
                positions
                    .get(label)
                    .is_some_and(|position| array <= *position && *position <= marker)
            })
            || self.exception_regions.iter().any(|region| {
                positions
                    .get(&region.target)
                    .is_some_and(|position| array < *position && *position <= marker)
            })
        {
            return false;
        }
        let mut original = 0;
        let mut live = array;
        let mut labels = FxHashSet::default();
        for item in &origin.items {
            match *item {
                Item::Label(label) => {
                    if !labels.insert(label)
                        || positions.get(&label) != Some(&live)
                        || provenance.retired_normal_try_copy_labels.contains(&label)
                        || provenance.normal_try_copy_blocks.contains(&label)
                        || provenance.normal_try_copy_internal_labels.contains(&label)
                        || self
                            .items
                            .iter()
                            .filter(|item| **item == Item::Label(label))
                            .count()
                            != 1
                    {
                        return false;
                    }
                }
                Item::Instruction(mut expected) => {
                    let offset = original;
                    original += 1;
                    if offset == origin.unwind_offset + 5 {
                        // Only the captured finalizer POP is erased from this live prefix.
                        continue;
                    }
                    let Some(actual) = instructions.get(live) else {
                        return false;
                    };
                    if offset == origin.unwind_offset + 6 {
                        expected.local_policy.add_hidden_exception_owner_slots(1);
                    }
                    if expected.opcode == crate::target::opcodes::LOAD_FAST
                        && actual.opcode == crate::target::opcodes::LOAD_FAST_BORROW
                    {
                        expected.opcode = actual.opcode;
                    }
                    if **actual != expected {
                        return false;
                    }
                    live += 1;
                }
            }
        }
        // The omitted handler POPs precede SWAP and POP_EXCEPT. They put the
        // finalizer POP at original SWAP+7, exactly the appended marker's cell.
        original == origin.unwind_offset + 9 && live == marker + 1
    }

    /// Replays only fully captured constant-return cleanup and its original NULL cells.
    fn copied_cleanup_return_none_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<ExceptionOwnerDecision> {
        if tail.insertions != 0 || tail.late_removals != 0 || tail.post_compaction_fusions != 0 {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut matching = provenance
            .copied_cleanup_return_none_prefixes
            .iter()
            .filter(|origin| {
                origin.marker == *instructions[marker]
                    && marker > 0
                    && origin.condition == *instructions[marker - 1]
            });
        let origin = matching.next()?;
        if matching.next().is_some()
            || positions.get(&origin.entry) != Some(&array)
            || !provenance.source_boundaries.contains(&origin.entry)
            || !provenance.block_boundaries.contains(&origin.entry)
            || provenance
                .retired_normal_try_copy_labels
                .contains(&origin.entry)
            || provenance.normal_try_copy_blocks.contains(&origin.entry)
            || provenance
                .normal_try_copy_internal_labels
                .contains(&origin.entry)
            || self
                .items
                .iter()
                .filter(|item| **item == Item::Label(origin.entry))
                .count()
                != 1
            || jump_targets
                .iter()
                .chain(&provenance.source_boundaries)
                .chain(&provenance.block_boundaries)
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| array < *position && *position <= marker)
                })
        {
            return None;
        }
        let removed = if origin.manager {
            0
        } else {
            1 + usize::from(origin.handler)
        };
        let mut live = origin.cleanup[removed..].to_vec();
        if removed > 0 {
            live[0]
                .local_policy
                .add_hidden_exception_owner_slots(u16::try_from(removed).ok()?);
        }
        let start = marker.checked_sub(live.len() + 2)?;
        if start < array
            || instructions[start..start + live.len()]
                .iter()
                .zip(&live)
                .any(|(actual, expected)| **actual != *expected)
        {
            return None;
        }
        let mut load = origin.load;
        load.local_policy.add_hidden_exception_owner_slots(1);
        if instructions[marker - 2].opcode == crate::target::opcodes::LOAD_FAST_BORROW {
            load.opcode = crate::target::opcodes::LOAD_FAST_BORROW;
        }
        if *instructions[marker - 2] != load
            || origin.pop.opcode != NOP
            || !origin.pop.fusion_policy.is_converted_pop_block()
            || !provenance.removed_pop_block_boundaries.iter().any(|label| {
                positions.get(label) == Some(&(marker - 2))
                    && provenance
                        .removed_pop_block_widths
                        .get(label)
                        .copied()
                        .unwrap_or(1)
                        == 1
            })
        {
            return None;
        }
        let mut prefix = ExceptionOwnerTail::default();
        for record in &instructions[array..start] {
            if record.opcode == NOP
                || record.local_policy.hidden_exception_owner_slots() != 0
                || record.pre_fold_extra_instructions() != 0
                || record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                || record.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
                || record.jump_policy.borrow_empty_block()
            {
                return None;
            }
            prefix.instruction(record, None);
        }
        if prefix.insertions != 0
            || prefix.post_compaction_fusions != 0
            || prefix.early_removals.checked_add(removed + 4) != Some(tail.early_removals)
        {
            return None;
        }
        let original_start = start.checked_add(prefix.early_removals)?;
        let slot = marker.checked_sub(original_start)?;
        // Pinned codegen unwinds one manager POP before its exit call. A named
        // handler contributes two POPs before POP_EXCEPT/name clearing; an ordinary
        // try contributes one POP before the finalizer. None of those cells is owned.
        let source = match (origin.manager, origin.handler, slot) {
            (false, false, 1) => Some(start),
            (false, false, 2) | (false, true, 1 | 4) => None,
            (false, true, 2..=3) => Some(start + slot - 2),
            (true, false, 1 | 7 | 8) | (true, true, 1 | 7 | 8 | 13) => None,
            (true, _, 2..=6) => Some(start + slot - 1),
            (true, true, 9..=12) => Some(start + slot - 3),
            _ => return None,
        };
        let protected_start = if origin.manager {
            start + if origin.handler { 6 } else { 1 }
        } else {
            start
        };
        let region = self.uniform_exception_owner(protected_start, marker - 2, positions)?;
        let owner = self.exception_regions[region];
        if owner.is_scope_fallback
            || owner.preserve_lasti
            || source.is_some_and(|source| {
                self.uniform_exception_owner(source, source + 1, positions) != Some(region)
            })
        {
            return None;
        }
        Some(ExceptionOwnerDecision::override_with_region(
            source.map(|_| region),
        ))
    }

    /// Recovers an owned handler cell across its fully captured finalizer SETUP.
    fn named_handler_try_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<usize> {
        use crate::target::opcodes::{
            DELETE_FAST, LOAD_FAST, LOAD_FAST_BORROW, POP_EXCEPT, STORE_FAST,
        };
        if tail.insertions != 0 || tail.late_removals != 0 || tail.post_compaction_fusions != 0 {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut origins = provenance
            .named_handler_try_prefixes
            .iter()
            .filter(|origin| {
                origin.items.last() == Some(&Item::Instruction(*instructions[marker]))
            });
        let origin = origins.next()?;
        if origins.next().is_some()
            || positions.get(&origin.entry) != Some(&array)
            || !provenance.source_boundaries.contains(&origin.entry)
            || !provenance.block_boundaries.contains(&origin.entry)
            || provenance
                .retired_normal_try_copy_labels
                .contains(&origin.entry)
            || provenance.normal_try_copy_blocks.contains(&origin.entry)
            || provenance
                .normal_try_copy_internal_labels
                .contains(&origin.entry)
            || self
                .items
                .iter()
                .filter(|item| **item == Item::Label(origin.entry))
                .count()
                != 1
        {
            return None;
        }
        let original = origin
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record) => Some(*record),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        if original.len() != 13 {
            return None;
        }
        let mut expected = Vec::new();
        let mut pending = 0_u16;
        let mut labels = Vec::new();
        for item in &origin.items {
            match *item {
                Item::Instruction(record) if record.fusion_policy.is_converted_pop_block() => {
                    pending += 1;
                }
                Item::Instruction(mut record) => {
                    record
                        .local_policy
                        .add_hidden_exception_owner_slots(pending);
                    pending = 0;
                    expected.push(record);
                }
                Item::Label(label) => labels.push((label, expected.len())),
            }
        }
        let start = marker.checked_sub(expected.len().checked_sub(1)?)?;
        if start < array
            || pending != 0
            || expected.len() != 10
            || expected.iter().enumerate().any(|(offset, expected)| {
                let mut expected = *expected;
                let actual = instructions[start + offset];
                if expected.opcode == LOAD_FAST && actual.opcode == LOAD_FAST_BORROW {
                    expected.opcode = LOAD_FAST_BORROW;
                }
                expected != *actual
            })
            || labels.iter().any(|(label, offset)| {
                positions.get(label) != Some(&(start + offset))
                    || self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(*label))
                        .count()
                        != 1
                    || jump_targets.contains(label)
                    || provenance.block_boundaries.contains(label)
                    || provenance.source_boundaries.contains(label)
                    || provenance.retired_normal_try_copy_labels.contains(label)
                    || provenance.normal_try_copy_blocks.contains(label)
                    || provenance.normal_try_copy_internal_labels.contains(label)
            })
            || jump_targets
                .iter()
                .chain(&provenance.block_boundaries)
                .chain(&provenance.source_boundaries)
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| array < *position && *position <= marker)
                })
        {
            return None;
        }
        let setup_at = start + 5;
        if !provenance.setup_nop_ranges.iter().any(|(begin, end)| {
            labels.contains(&(*begin, 5))
                && labels.contains(&(*end, 6))
                && positions.get(begin) == Some(&setup_at)
                && positions.get(end) == Some(&(setup_at + 1))
        }) || !provenance.setups.iter().any(|(label, location)| {
            labels.contains(&(*label, 6))
                && *location == Some(original[8].location)
                && positions.get(label) == Some(&(setup_at + 1))
        }) {
            return None;
        }
        let mut prefix = ExceptionOwnerTail::default();
        for record in &instructions[array..start] {
            if record.opcode == NOP
                || record.local_policy.hidden_exception_owner_slots() != 0
                || record.pre_fold_extra_instructions() != 0
                || record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                || record.no_location_policy.borrow_fallthrough()
                    != super::BorrowFallthrough::Reachable
                || record.jump_policy.borrow_empty_block()
                || !matches!(record.operand(), Operand::Value(_))
                || is_scope_exit(record.opcode)
            {
                return None;
            }
            prefix.instruction(record, None);
        }
        if prefix.insertions != 0
            || prefix.post_compaction_fusions != 0
            || prefix.early_removals.checked_add(3) != Some(tail.early_removals)
        {
            return None;
        }
        let slot = marker.checked_sub(start.checked_add(prefix.early_removals)?)?;
        // POP_EXCEPT and all three name-clearing records share the ordinary
        // owner exposed by the named handler's two original POP_BLOCKs.
        if !(3..=6).contains(&slot)
            || [
                instructions[start + 1].opcode,
                instructions[start + 2].opcode,
                instructions[start + 3].opcode,
                instructions[start + 4].opcode,
            ] != [POP_EXCEPT, LOAD_CONST, STORE_FAST, DELETE_FAST]
        {
            return None;
        }
        let region = self.uniform_exception_owner(start + 1, start + 5, positions)?;
        let owner = self.exception_regions[region];
        (!owner.is_scope_fallback && !owner.preserve_lasti).then_some(region)
    }

    /// Replays a copied continue whose inserted marker reuses its original SETUP.
    fn continued_handler_try_is_null(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<()> {
        use crate::target::opcodes::{LOAD_FAST, LOAD_FAST_BORROW};
        if marker.checked_sub(array) != Some(5)
            || tail.early_removals != 6
            || tail.insertions != 0
            || tail.late_removals != 0
            || tail.post_compaction_fusions != 0
        {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut origins = provenance
            .continued_handler_try_prefixes
            .iter()
            .filter(|origin| {
                origin.items.last() == Some(&Item::Instruction(*instructions[marker]))
            });
        let origin = origins.next()?;
        if origins.next().is_some()
            || positions.get(&origin.entry) != Some(&array)
            || !provenance.source_boundaries.contains(&origin.entry)
            || !provenance.block_boundaries.contains(&origin.entry)
            || provenance
                .retired_normal_try_copy_labels
                .contains(&origin.entry)
            || provenance.normal_try_copy_blocks.contains(&origin.entry)
            || provenance
                .normal_try_copy_internal_labels
                .contains(&origin.entry)
            || self
                .items
                .iter()
                .filter(|item| **item == Item::Label(origin.entry))
                .count()
                != 1
        {
            return None;
        }
        let original = origin
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record) => Some(*record),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        if original.len() != 9 {
            return None;
        }
        let mut expected = Vec::new();
        let mut labels = Vec::new();
        let mut pending = 0_u16;
        let mut index = 0;
        for item in &origin.items {
            match *item {
                Item::Instruction(mut record) => {
                    if matches!(index, 1 | 2 | 4) {
                        pending += 1;
                    } else {
                        record
                            .local_policy
                            .add_hidden_exception_owner_slots(pending);
                        pending = 0;
                        expected.push(record);
                    }
                    index += 1;
                }
                Item::Label(label) => labels.push((label, expected.len())),
            }
        }
        if pending != 0
            || expected.len() != 6
            || expected.iter().enumerate().any(|(offset, expected)| {
                let mut expected = *expected;
                let actual = instructions[array + offset];
                if expected.opcode == LOAD_FAST && actual.opcode == LOAD_FAST_BORROW {
                    expected.opcode = LOAD_FAST_BORROW;
                }
                expected != *actual
            })
            || labels.iter().any(|(label, offset)| {
                positions.get(label) != Some(&(array + offset))
                    || self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(*label))
                        .count()
                        != 1
                    || jump_targets.contains(label)
                    || provenance.block_boundaries.contains(label)
                    || provenance.source_boundaries.contains(label)
                    || provenance.retired_normal_try_copy_labels.contains(label)
                    || provenance.normal_try_copy_blocks.contains(label)
                    || provenance.normal_try_copy_internal_labels.contains(label)
            })
            || jump_targets
                .iter()
                .chain(&provenance.block_boundaries)
                .chain(&provenance.source_boundaries)
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| array < *position && *position <= marker)
                })
        {
            return None;
        }
        // POP_TOP, authored continue NOP, handler POP, POP_EXCEPT, finally POP,
        // then SETUP: compacted marker cell five still contains that NULL SETUP.
        let setup_at = array + 2;
        if !provenance.setup_nop_ranges.iter().any(|(begin, end)| {
            labels.contains(&(*begin, 2))
                && labels.contains(&(*end, 3))
                && positions.get(begin) == Some(&setup_at)
                && positions.get(end) == Some(&(setup_at + 1))
        }) || !provenance.setups.iter().any(|(label, location)| {
            labels.contains(&(*label, 3))
                && *location == Some(original[5].location)
                && positions.get(label) == Some(&(setup_at + 1))
        }) {
            return None;
        }
        Some(())
    }

    /// Replays a retired original try-exit JUMP on its unchanged normal-finally NOP.
    fn normal_try_break_pop_owners(
        &self,
        positions: &OwnerLabelPositions,
        targets: &FxHashSet<Label>,
    ) -> FxHashMap<usize, ExceptionOwnerDecision> {
        use crate::target::opcodes::{LOAD_FAST, LOAD_FAST_BORROW};

        let mut result = FxHashMap::default();
        for origin in &self.exception_owner_provenance.normal_try_break_pop_owners {
            // Threading retires the bridge's sole instruction; its original labels
            // remain empty immediately before the same original inner handler.
            let Some(first) = origin.source_labels.first() else {
                continue;
            };
            let Some(source) = self
                .items
                .iter()
                .position(|item| *item == Item::Label(*first))
            else {
                continue;
            };
            if origin
                .source_labels
                .iter()
                .enumerate()
                .any(|(offset, label)| {
                    self.items.get(source + offset) != Some(&Item::Label(*label))
                        || targets.contains(label)
                        || positions.get(label) != positions.get(&origin.handler)
                })
                || self.items.get(source + origin.source_labels.len())
                    != Some(&Item::Label(origin.handler))
            {
                continue;
            }
            let Some(start) = self
                .items
                .iter()
                .position(|item| *item == Item::Label(origin.entry))
            else {
                continue;
            };
            let Some(current) = self.items.get(start..start + origin.prefix.len()) else {
                continue;
            };
            if self.items.get(start + origin.prefix.len()) != Some(&Item::Label(origin.exit)) {
                continue;
            }
            let mut expected = origin.prefix.clone();
            let Some(marker_at) = expected
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            let Item::Instruction(marker) = &mut expected[marker_at] else {
                unreachable!();
            };
            marker.location = origin.location;
            for (expected, current) in expected.iter_mut().zip(current) {
                if let (Item::Instruction(expected), Item::Instruction(current)) =
                    (expected, current)
                    && expected.opcode == LOAD_FAST
                    && current.opcode == LOAD_FAST_BORROW
                {
                    expected.opcode = LOAD_FAST_BORROW;
                }
            }
            if current != expected.as_slice()
                || current.iter().any(|item| {
                    matches!(item, Item::Label(label)
                    if targets.contains(label)
                        || self.exception_regions.iter().any(|region| region.target == *label))
                })
            {
                continue;
            }
            let Some(&marker) = positions.get(&origin.entry) else {
                continue;
            };
            if positions.get(&origin.region.end) != Some(&marker) {
                continue;
            }
            let Some(&region_start) = positions.get(&origin.region.start) else {
                continue;
            };
            if region_start >= marker {
                continue;
            }
            let mut regions = self
                .exception_regions
                .iter()
                .enumerate()
                .filter(|(_, region)| {
                    let mut original_labels = **region;
                    original_labels.start = origin.region.start;
                    original_labels.end = origin.region.end;
                    positions.get(&region.start) == Some(&region_start)
                        && positions.get(&region.end) == Some(&marker)
                        && super::fusion_owner_slots::same_manager_region(
                            &original_labels,
                            &origin.region,
                        )
                });
            let Some((region, _)) = regions.next() else {
                continue;
            };
            if regions.next().is_some() {
                continue;
            }
            result.insert(
                marker,
                ExceptionOwnerDecision::override_with_region(Some(region)),
            );
        }
        result
    }

    /// Resolves inserted markers after fusion and complete SETUP provenance is available.
    pub(super) fn dynamic_exception_owners(&self) -> DynamicExceptionOwners {
        #[cfg(test)]
        {
            let bound_replay = !FORCE_COMPLETE_OWNER_REPLAY.get();
            let gate_decisions = !FORCE_ALL_OWNER_DECISIONS.get();
            let owners = if gate_decisions {
                self.dynamic_exception_owners_with_replay::<true>(bound_replay)
            } else {
                self.dynamic_exception_owners_with_replay::<false>(bound_replay)
            };
            if VERIFY_COMPLETE_OWNER_REPLAY.get() {
                let complete = self.dynamic_exception_owners_with_replay::<false>(false);
                assert_eq!(owners.instruction_count, complete.instruction_count);
                assert_eq!(owners.slots, complete.slots);
                if bound_replay && gate_decisions {
                    let ungated = self.dynamic_exception_owners_with_replay::<false>(true);
                    assert_eq!(owners.instruction_count, ungated.instruction_count);
                    assert_eq!(owners.slots, ungated.slots);
                }
            }
            owners
        }
        #[cfg(not(test))]
        {
            self.dynamic_exception_owners_with_replay::<true>(true)
        }
    }

    /// Returns the entry-prologue fact only when some original owner can be replayed.
    fn dynamic_owner_replay_entry(&self) -> Option<bool> {
        let provenance = &self.exception_owner_provenance;
        let entry_prologue = || {
            self.items
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                })
                .is_some_and(|opcode| {
                    matches!(opcode, COPY_FREE_VARS | MAKE_CELL | RETURN_GENERATOR)
                })
        };
        let dynamic = |instruction: &Instruction| {
            matches!(
                instruction.exception_policy().owner(),
                NormalizedExceptionOwner::Dynamic
                    | NormalizedExceptionOwner::DynamicCondition
                    | NormalizedExceptionOwner::DynamicConditionStrict
            )
        };
        if provenance.synchronous_with_setups.is_empty()
            && provenance.source_boundaries.is_empty()
            && provenance.normal_try_break_pop_owners.is_empty()
            && provenance.named_handler_truth_prefixes.is_empty()
        {
            let has_entry_prologue = entry_prologue();
            let has_retained_setup = provenance
                .setups
                .iter()
                .any(|(_, location)| location.is_some());
            // This is stricter than the general marker test: a passing instruction is
            // either dynamic or itself NOT_TAKEN, so the general test cannot reject it.
            self.items
                .iter()
                .any(|item| {
                    let Item::Instruction(instruction) = item else {
                        return false;
                    };
                    dynamic(instruction)
                        || (has_retained_setup
                            && instruction.opcode == NOT_TAKEN
                            && (matches!(
                                instruction.exception_policy().owner(),
                                NormalizedExceptionOwner::Present
                                    | NormalizedExceptionOwner::Absent
                            ) || (has_entry_prologue
                                && instruction.exception_policy()
                                    == super::ExceptionPolicy::NoneUnchanged)))
                })
                .then_some(has_entry_prologue)
        } else {
            // With independent strict provenance, only the general marker/captured-array
            // gate can reject this unit. Avoid scanning when captured arrays satisfy it.
            if provenance.normal_try_break_pop_owners.is_empty()
                && provenance.named_handler_truth_prefixes.is_empty()
                && provenance.copied_conjunction_break_prefixes.is_empty()
                && provenance.uncompacted_conditional_arrays.is_empty()
                && !self.items.iter().any(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if instruction.opcode == NOT_TAKEN || dynamic(instruction))
                })
            {
                return None;
            }
            Some(entry_prologue())
        }
    }

    fn dynamic_exception_owners_with_replay<const GATE_DECISIONS: bool>(
        &self,
        bound_replay: bool,
    ) -> DynamicExceptionOwners {
        if self.exception_regions.is_empty() {
            return DynamicExceptionOwners::default();
        }
        // Only markers, dynamic policies, or captured arrays can change owners.
        let provenance = &self.exception_owner_provenance;
        let Some(has_entry_prologue) = self.dynamic_owner_replay_entry() else {
            return DynamicExceptionOwners::default();
        };
        let unique_labels = OnceCell::new();
        let original_value_jump_positions = self
            .exception_owner_provenance
            .original_value_jump_positions(&self.items, &unique_labels);
        let original_async_with_setups = self.original_async_with_setups();
        let setups: FxHashMap<_, _> = self
            .exception_owner_provenance
            .setups
            .iter()
            .copied()
            .collect();
        let mut label_positions = OwnerLabelPositions::new(self.items.len());
        let mut indexed_instructions = Vec::with_capacity(self.items.len());
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
        let instruction_positions = if GATE_DECISIONS {
            OwnerInstructionPositions::for_sources(
                indexed_instructions.len(),
                provenance.original_uniform_folds.len()
                    + provenance.comprehension_restore_owner_prefixes.len()
                    + provenance.threaded_condition_origins.len(),
                provenance
                    .original_uniform_folds
                    .iter()
                    .chain(
                        provenance
                            .comprehension_restore_owner_prefixes
                            .iter()
                            .map(|(_, instruction)| instruction),
                    )
                    .chain(
                        provenance
                            .threaded_condition_origins
                            .iter()
                            .map(|(first, _)| first),
                    ),
            )
        } else {
            OwnerInstructionPositions::default()
        };
        let threaded_conditions = self.threaded_condition_positions_with_index::<GATE_DECISIONS>(
            &indexed_instructions,
            &instruction_positions,
        );
        let original_uniform_folds = self
            .exception_owner_provenance
            .original_uniform_fold_positions_with_index::<GATE_DECISIONS>(
                &indexed_instructions,
                &instruction_positions,
            );
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
                    || unique_labels
                        .get_or_init(|| UniqueLabelPositions::new(&self.items))
                        .get(*entry)
                        .is_none()
                {
                    return None;
                }
                let start = *label_positions.get(entry)?;
                let index = if GATE_DECISIONS {
                    instruction_positions.unique(&indexed_instructions, restore)?
                } else {
                    let mut matches = indexed_instructions
                        .iter()
                        .enumerate()
                        .filter(|(_, instruction)| ***instruction == *restore);
                    let (index, _) = matches.next()?;
                    if index < start || matches.next().is_some() {
                        return None;
                    }
                    index
                };
                (index >= start).then_some((index, start))
            })
            .collect::<FxHashMap<_, _>>();
        let for_target_prefixes = self
            .exception_owner_provenance
            .original_for_target_prefix_positions(&self.items, &unique_labels);
        let retained_setup_nops =
            self.retained_setup_nops_with_instructions(&label_positions, &indexed_instructions);
        // An original entry prefix with no SETUP, region, or policy owner can
        // supply only NULL records, including its certified early-folded slots.
        let null_original_entry_prefix =
            |start: usize, setup_label: Label, owners: &DynamicExceptionOwners| {
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
                            ) && !owners.contains_key(index)
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
        let retained_setup_value_null_markers = self.retained_setup_value_null_markers(
            &label_positions,
            &jump_targets,
            &retained_setup_nops,
            &original_value_jump_positions,
        );
        let retained_normal_pop_terminators =
            self.retained_normal_pop_terminators(&jump_targets, &label_positions);
        let (captured_pop_slots, captured_pop_aliases, captured_method_donors) =
            self.captured_pop_slots(&jump_targets, &label_positions);
        let original_normal_finally_null_markers =
            self.original_normal_finally_null_markers(&jump_targets);
        let decision_indices = if GATE_DECISIONS {
            OwnerDecisionIndices::new(
                indexed_instructions.len(),
                retained_normal_pop_terminators
                    .keys()
                    .chain(threaded_conditions.keys())
                    .chain(&original_normal_finally_null_markers)
                    .chain(&retained_setup_null_markers)
                    .chain(&retained_setup_value_null_markers),
            )
        } else {
            OwnerDecisionIndices::default()
        };
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
        let boundaries = || {
            provenance
                .block_boundaries
                .iter()
                .chain(&self.preserved_block_boundaries)
                .chain(self.exception_regions.iter().map(|region| &region.target))
        };
        let mut replay_labels =
            OwnerReplayLabels::new(self.items.len(), usize::try_from(self.next_label).unwrap());
        replay_labels.record(boundaries(), OwnerReplayLabelFlags::BOUNDARY);
        replay_labels.record(&jump_targets, OwnerReplayLabelFlags::JUMP_TARGET);
        replay_labels.record(
            self.exception_regions.iter().map(|region| &region.end),
            OwnerReplayLabelFlags::REGION_END,
        );
        replay_labels.record(
            &provenance.source_boundaries,
            OwnerReplayLabelFlags::SOURCE_BOUNDARY,
        );
        replay_labels.record(
            &provenance.removed_pop_block_boundaries,
            OwnerReplayLabelFlags::REMOVED_POP_BLOCK,
        );
        replay_labels.record(
            &provenance.continuation_boundaries,
            OwnerReplayLabelFlags::CONTINUATION,
        );
        replay_labels.record(
            &provenance.setup_nop_boundaries,
            OwnerReplayLabelFlags::SETUP_NOP,
        );
        replay_labels.record(&provenance.fusion_boundaries, OwnerReplayLabelFlags::FUSION);
        replay_labels.record(
            provenance
                .omitted_nop_counts
                .keys()
                .chain(provenance.omitted_nop_locations.keys()),
            OwnerReplayLabelFlags::OMITTED_NOP,
        );
        replay_labels.record(
            &provenance.block_boundaries,
            OwnerReplayLabelFlags::ORIGINAL_BLOCK,
        );
        replay_labels.record(
            &provenance.value_jump_continuation_boundaries,
            OwnerReplayLabelFlags::VALUE_JUMP_CONTINUATION,
        );
        replay_labels.record(
            &provenance.protected_prefix_boundaries,
            OwnerReplayLabelFlags::PROTECTED_PREFIX,
        );
        replay_labels.record(
            &provenance.retired_normal_try_copy_labels,
            OwnerReplayLabelFlags::RETIRED_TRY_COPY,
        );
        replay_labels.record(
            &provenance.synchronous_with_setups,
            OwnerReplayLabelFlags::SYNCHRONOUS_WITH,
        );
        replay_labels.record(setups.keys(), OwnerReplayLabelFlags::SETUP);
        let replay_items = if bound_replay {
            // Apart from dynamic policies and markers, the loop writes only through these
            // precomputed indices or the successor of a threaded conditional. Its label arm
            // only prepares the state for later instructions. The post-loop source proofs
            // still run against the complete instruction stream below.
            let last_indexed_writer = original_normal_finally_null_markers
                .iter()
                .chain(&retained_setup_null_markers)
                .chain(&retained_setup_value_null_markers)
                .chain(threaded_conditions.keys())
                .copied()
                .max();
            let limit = dynamic_owner_replay_item_limit(
                &self.items,
                indexed_instructions.len(),
                last_indexed_writer,
            );
            &self.items[..limit]
        } else {
            &self.items[..]
        };
        let mut owners = DynamicExceptionOwners::new(indexed_instructions.len());
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
        let mut transferred_setup_null_slot = None;
        let mut retained_pop_prefix = None;
        let mut early_return_nop_slot = None;
        let mut early_return_iterator_slot = None;
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
        let mut captured_method_owner_slots = Vec::new();
        let mut certified_method_markers = FxHashMap::default();
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
        let copied_handler_cleanups_by_end = OnceCell::new();
        for item in replay_items {
            match item {
                Item::Label(label) => {
                    let flags = replay_labels.get(*label);
                    let is_boundary = flags.contains(OwnerReplayLabelFlags::BOUNDARY);
                    let is_join = flags.contains(
                        OwnerReplayLabelFlags::BOUNDARY | OwnerReplayLabelFlags::JUMP_TARGET,
                    );
                    let is_region_end = flags.contains(OwnerReplayLabelFlags::REGION_END);
                    let setup = flags
                        .contains(OwnerReplayLabelFlags::SETUP)
                        .then(|| setups.get(label))
                        .flatten();
                    let is_source_boundary = flags.contains(OwnerReplayLabelFlags::SOURCE_BOUNDARY);
                    if is_join || setup.is_some() || is_source_boundary {
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                        captured_method_owner_slots.clear();
                    }
                    let copied_handler_cleanups = if owner_history_exact
                        && !self
                            .exception_owner_provenance
                            .copied_handler_cleanups
                            .is_empty()
                    {
                        let by_end = copied_handler_cleanups_by_end.get_or_init(|| {
                            let mut by_end: FxHashMap<_, SmallVec<[&CopiedHandlerCleanup; 1]>> =
                                FxHashMap::default();
                            for original in &self.exception_owner_provenance.copied_handler_cleanups
                            {
                                by_end.entry(original.end).or_default().push(original);
                            }
                            by_end
                        });
                        by_end.get(label)
                    } else {
                        None
                    };
                    for original in copied_handler_cleanups.into_iter().flatten() {
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
                        if let Some(mut iterator) = original.return_iterator {
                            // Only the authored NOP disappears into this exact iterator
                            // POP_TOP. The captured name-clear records remain unchanged.
                            iterator.local_policy.add_hidden_exception_owner_slots(1);
                            if start == 0
                                || original_array_start > start - 1
                                || *indexed_instructions[start - 1] != iterator
                                || original.removed_prefix_slots != 0
                                || original.missing_null_pops != 3
                                || jump_targets
                                    .iter()
                                    .chain(&self.exception_owner_provenance.source_boundaries)
                                    .chain(&self.exception_owner_provenance.block_boundaries)
                                    .any(|label| {
                                        label_positions.get(label).is_some_and(|position| {
                                            start - 1 < *position && *position < instruction_index
                                        })
                                    })
                            {
                                continue;
                            }
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
                                original.continued_attribute_none,
                            ));
                        }
                    }
                    if is_join
                        || setup.is_some()
                        || is_source_boundary
                        || flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                    {
                        retained_pop_prefix = None;
                    }
                    let continues_original_block = flags
                        .contains(OwnerReplayLabelFlags::CONTINUATION)
                        && !is_join
                        && setup.is_none()
                        && !flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK);
                    if !continues_original_block {
                        has_uninterrupted_fusion = false;
                    }
                    if is_join
                        || (setup.is_none() && !flags.contains(OwnerReplayLabelFlags::SETUP_NOP))
                    {
                        pending_retained_prefix = None;
                    }
                    if transferred_pop_setup.is_some_and(|(setup, _)| {
                        is_join
                            || is_region_end
                            || is_source_boundary
                            || flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                            || flags.contains(OwnerReplayLabelFlags::OMITTED_NOP)
                            || (*label != setup
                                && !flags.contains(OwnerReplayLabelFlags::SETUP_NOP))
                    }) {
                        transferred_pop_setup = None;
                    }
                    if is_join
                        || setup.is_some()
                        || flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                    {
                        pending_threaded_null_append = None;
                    }
                    // Only range endpoints and recorded bookkeeping continuations may
                    // intervene between this authenticated await condition and its marker.
                    // An authored entry or an unknown label ends the original-array proof.
                    if is_join
                        || setup.is_some()
                        || is_source_boundary
                        || flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                        || !(is_region_end || flags.contains(OwnerReplayLabelFlags::CONTINUATION))
                    {
                        pending_await_null_append = None;
                    }
                    if is_join
                        || setup.is_some()
                        || is_region_end
                        || flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                    {
                        // A marker-only range end adds no original cell between
                        // an authored primitive condition and its appended marker.
                        // Carry only an already-proven erased POP slot across it.
                        let original_pop_append = pending_null_append == Some(instruction_index)
                            && owner_history_exact
                            && pop_slot_epoch_known
                            && is_region_end
                            && !is_join
                            && setup.is_none()
                            && !is_source_boundary
                            && !flags.contains(OwnerReplayLabelFlags::FUSION)
                            && !flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                            && !flags.contains(OwnerReplayLabelFlags::OMITTED_NOP)
                            && previous.is_some_and(|condition: &Instruction| {
                                is_conditional_jump(condition.opcode)
                                    && matches!(condition.operand(), Operand::Forward(_))
                                    && self
                                        .exception_owner_provenance
                                        .original_primitive_condition_boundaries
                                        .iter()
                                        .filter(|origin| *origin == condition)
                                        .count()
                                        == 1
                                    && indexed_instructions
                                        .iter()
                                        .filter(|record| **record == condition)
                                        .count()
                                        == 1
                                    && indexed_instructions.get(instruction_index).is_some_and(
                                        |marker| {
                                            marker.opcode == NOT_TAKEN
                                                && marker.location == condition.location
                                                && marker.depth_after() == condition.depth_after()
                                        },
                                    )
                            });
                        if !original_pop_append {
                            pending_null_append = None;
                        }
                        pending_fresh_null_append = None;
                        pending_normal_pop_null_append = None;
                    }
                    if is_join
                        || setup.is_some()
                        || flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                        || (!is_region_end && !continues_original_block)
                    {
                        pending_compacted_append = None;
                    }
                    if has_instruction && is_boundary {
                        retained_pop_prefix = None;
                        original_array_start = instruction_index;
                        transferred_setup_null_slot = None;
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                        captured_method_owner_slots.clear();
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
                    if is_region_end
                        && !continues_original_block
                        && !flags.contains(OwnerReplayLabelFlags::SETUP_NOP)
                    {
                        ended_owned_prefix = if flags
                            .contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                        {
                            retained_owned_suffix.or(source_owned_prefix).map(
                                |(start, setup_tail)| (start, instruction_index, setup_tail, tail),
                            )
                        } else {
                            None
                        };
                        if flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
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
                        } else if !previous.is_some_and(|instruction: &Instruction| {
                            is_conditional_jump(instruction.opcode)
                        }) {
                            ended_owned_suffix = None;
                        }
                        source_owned_prefix = None;
                        // An exception-range end can hide an original POP_BLOCK removed during
                        // compaction. One original try-exit join proves a narrower exception:
                        // its unchanged primitive truth array has no erased records, and this
                        // endpoint only splits the condition from its newly appended marker.
                        source_block &= owner_history_exact
                            && pop_slot_epoch_known
                            && tail.early_removals == 0
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && instruction_index == original_array_start + 3
                            && matches!(indexed_instructions.get(instruction_index), Some(marker)
                            if marker.opcode == NOT_TAKEN
                                && marker.exception_policy() == super::ExceptionPolicy::NoneDynamicCondition
                                && previous.is_some_and(|condition: &Instruction| {
                                    condition.location == marker.location
                                        && matches!(condition.opcode,
                                            crate::target::opcodes::POP_JUMP_IF_FALSE | crate::target::opcodes::POP_JUMP_IF_TRUE)
                                        && matches!(condition.operand(), Operand::Forward(_))
                                        && self.exception_owner_provenance.original_primitive_condition_boundaries
                                            .iter().filter(|original| *original == condition).count() == 1
                                }))
                            && matches!(&indexed_instructions[original_array_start..instruction_index],
                            [load, to_bool, condition]
                                if matches!(load.opcode, crate::target::opcodes::LOAD_FAST | crate::target::opcodes::LOAD_FAST_BORROW)
                                    && matches!(load.operand(), Operand::Value(_))
                                    && to_bool.opcode == crate::target::opcodes::TO_BOOL
                                    && to_bool.operand() == Operand::Value(0)
                                    && [load, to_bool, condition].iter().all(|record| {
                                        record.location == condition.location
                                            && record.exception_policy() == super::ExceptionPolicy::NoneUnchanged
                                            && record.local_policy.hidden_exception_owner_slots() == 0
                                            && record.pre_fold_extra_instructions() == 0
                                    }))
                            && self
                                .exception_owner_provenance
                                .normal_try_exits
                                .iter()
                                .filter(|(entry, target)| {
                                    label_positions.get(entry) == Some(&original_array_start)
                                        && label_positions.get(target)
                                            == Some(&original_array_start)
                                        && self
                                            .exception_owner_provenance
                                            .block_boundaries
                                            .contains(target)
                                        && self
                                            .exception_owner_provenance
                                            .source_boundaries
                                            .contains(target)
                                })
                                .count()
                                == 1
                            && !jump_targets.iter().any(|target| {
                                label_positions.get(target).is_some_and(|position| {
                                    original_array_start <= *position
                                        && *position <= instruction_index
                                })
                            })
                            && !self
                                .exception_owner_provenance
                                .normal_try_copy_pop_ranges
                                .iter()
                                .any(|(start, end)| {
                                    [start, end].iter().any(|label| {
                                        label_positions.get(label).is_some_and(|position| {
                                            original_array_start <= *position
                                                && *position <= instruction_index
                                        })
                                    })
                                });
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
                            && flags.contains(OwnerReplayLabelFlags::ORIGINAL_BLOCK)
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
                        if !flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                            && !retained_setup_head_alias
                        {
                            retained_epoch_known = false;
                        }
                        if flags.contains(OwnerReplayLabelFlags::JUMP_TARGET)
                            || !previous.is_some_and(|instruction: &Instruction| {
                                is_conditional_jump(instruction.opcode)
                            })
                        {
                            retained_owned_suffix = None;
                        }
                    }
                    if has_instruction
                        && !is_boundary
                        && setup.is_none()
                        && !is_region_end
                        && !continues_original_block
                        && !flags.contains(OwnerReplayLabelFlags::SETUP_NOP)
                    {
                        let provenance = &self.exception_owner_provenance;
                        let same_tail = |other: ExceptionOwnerTail| {
                            other.early_removals == tail.early_removals
                                && other.insertions == tail.insertions
                                && other.late_removals == tail.late_removals
                                && other.post_compaction_fusions == tail.post_compaction_fusions
                        };
                        // An unchanged alias of this authored normal POP adds no record.
                        // Preserve an existing epoch only through its exact local or method None test;
                        // the following marker ends the original array and resets the epoch.
                        retained_epoch_known &= owner_history_exact
                            && pop_slot_epoch_known
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && ended_owned_suffix == Some(instruction_index)
                            && ended_owned_prefix.is_some_and(|(start, end, _, end_tail)| {
                                original_array_start <= start
                                    && start < end
                                    && end == instruction_index
                                    && same_tail(end_tail)
                            })
                            && ended_pop_slot.is_some_and(|(start, pop_tail, width)| {
                                start == instruction_index && width == 1 && same_tail(pop_tail)
                            })
                            && !flags.contains(OwnerReplayLabelFlags::JUMP_TARGET)
                            && !flags.contains(OwnerReplayLabelFlags::OMITTED_NOP)
                            && !flags.contains(
                                OwnerReplayLabelFlags::SOURCE_BOUNDARY
                                    | OwnerReplayLabelFlags::FUSION
                                    | OwnerReplayLabelFlags::REMOVED_POP_BLOCK
                                    | OwnerReplayLabelFlags::RETIRED_TRY_COPY,
                            )
                            && !provenance.is_normal_try_copy_label(*label)
                            && unique_labels
                                .get_or_init(|| UniqueLabelPositions::new(&self.items))
                                .get(*label)
                                .is_some()
                            && ((indexed_instructions
                                .get(instruction_index + 2)
                                .is_some_and(|marker| marker.opcode == NOT_TAKEN)
                                && provenance
                                    .normal_finally_none_prefixes
                                    .iter()
                                    .filter(|(_, original_load, original_condition)| {
                                        let Some(load) =
                                            indexed_instructions.get(instruction_index)
                                        else {
                                            return false;
                                        };
                                        let mut expected_load = *original_load;
                                        expected_load
                                            .local_policy
                                            .add_hidden_exception_owner_slots(1);
                                        if load.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                                            expected_load.opcode = load.opcode;
                                        }
                                        **load == expected_load
                                            && indexed_instructions
                                                .get(instruction_index + 1)
                                                .is_some_and(|condition| {
                                                    **condition == *original_condition
                                                })
                                    })
                                    .count()
                                    == 1)
                                || (indexed_instructions
                                    .get(instruction_index + 4)
                                    .is_some_and(|marker| marker.opcode == NOT_TAKEN)
                                    && self.normal_finally_method_none_prefix(
                                        instruction_index + 3,
                                        original_array_start,
                                        &indexed_instructions,
                                        &label_positions,
                                        &jump_targets,
                                    ) == Some((instruction_index, 1))));
                        // Labels that alias the certified POP_BLOCK end add no records.
                        // A later unknown label or a real jump join ends the proof.
                        if ended_owned_suffix != Some(instruction_index)
                            || flags.contains(OwnerReplayLabelFlags::JUMP_TARGET)
                        {
                            ended_owned_suffix = None;
                        }
                        retained_owned_suffix = None;
                        source_owned_prefix = None;
                        if (ended_owned_prefix
                            .is_none_or(|(_, end, _, _)| end != instruction_index)
                            || flags.contains(OwnerReplayLabelFlags::JUMP_TARGET))
                            && !previous.is_some_and(|instruction: &Instruction| {
                                is_conditional_jump(instruction.opcode)
                            })
                            && (flags.contains(OwnerReplayLabelFlags::JUMP_TARGET)
                                || !original_value_jump_positions.contains_key(label))
                        {
                            ended_owned_prefix = None;
                        }
                    }
                    if has_instruction
                        && !is_boundary
                        && (flags.contains(OwnerReplayLabelFlags::JUMP_TARGET)
                            || (setup.is_none()
                                && !continues_original_block
                                && !flags.contains(OwnerReplayLabelFlags::SETUP_NOP)
                                && !previous.is_some_and(|instruction: &Instruction| {
                                    is_conditional_jump(instruction.opcode)
                                })))
                    {
                        late_setup_history_known = false;
                        // A value-jump range start adds no original record or join.
                        // Keep the older position-sensitive history conservative.
                        value_jump_setup_history_known &= !flags
                            .contains(OwnerReplayLabelFlags::JUMP_TARGET)
                            && !flags.contains(OwnerReplayLabelFlags::REMOVED_POP_BLOCK)
                            && flags.contains(OwnerReplayLabelFlags::VALUE_JUMP_CONTINUATION);
                    }
                    if has_instruction
                        && !captured_pop_aliases.contains(label)
                        && !continues_original_block
                        && !flags.contains(OwnerReplayLabelFlags::PROTECTED_PREFIX)
                        && ended_owned_suffix != Some(instruction_index)
                        && !previous.is_some_and(|instruction: &Instruction| {
                            is_conditional_jump(instruction.opcode)
                        })
                    {
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                        captured_method_owner_slots.clear();
                    }
                    source_block |= is_source_boundary;
                    if is_boundary && is_source_boundary {
                        source_owned_prefix = Some((instruction_index, tail));
                    }
                    if owner_history_exact
                        && flags.contains(OwnerReplayLabelFlags::PROTECTED_PREFIX)
                    {
                        source_owned_prefix = Some((instruction_index, tail));
                    }
                    if let Some(nop_location) = setup {
                        ended_owned_suffix = None;
                        source_owned_prefix = None;
                        let retained = nop_location.is_some_and(|location| {
                            previous.is_some_and(|previous: &Instruction| {
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
                            let known_setup = flags
                                .contains(OwnerReplayLabelFlags::SYNCHRONOUS_WITH)
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
                            && (flags.contains(OwnerReplayLabelFlags::SYNCHRONOUS_WITH)
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
                    if GATE_DECISIONS
                        && decision_indices.can_replay_ordinary_history(
                            instruction,
                            instruction_index,
                            pending_threaded_null_append,
                        )
                    {
                        // Ordinary records consume one-instruction certificates and retain
                        // exact owner history; NOPs, POPs, barriers, and erased cells still
                        // take the complete source replay below.
                        transferred_pop_setup = None;
                        pending_compacted_append = None;
                        pending_threaded_null_append = None;
                        pending_retained_prefix = None;
                        pending_null_append = None;
                        pending_fresh_null_append = None;
                        pending_await_null_append = None;
                        pending_normal_pop_null_append = None;
                        if ended_uniform_with_suffix {
                            if ended_owned_suffix != Some(instruction_index) {
                                ended_owned_suffix = None;
                            }
                            uniform_with_suffix = None;
                            retained_owned_suffix = None;
                        }
                        ended_uniform_with_suffix = false;
                        if ended_owned_suffix == Some(instruction_index) {
                            ended_pop_slot = ended_pop_slot.filter(|(start, _, width)| {
                                *start == instruction_index && *width == 0
                            });
                        }
                        let previous_fusions = tail.post_compaction_fusions;
                        tail.instruction(instruction, previous);
                        if tail.post_compaction_fusions > previous_fusions {
                            has_uninterrupted_fusion = true;
                        }
                        if owner_history_exact
                            && source_owned_prefix.is_none()
                            && comprehension_restore_prefixes.get(&instruction_index)
                                == Some(&original_array_start)
                        {
                            source_owned_prefix = Some((instruction_index + 1, tail));
                        }
                        instruction_index += 1;
                        has_instruction = true;
                        previous = Some(instruction);
                        continue;
                    }
                    // Capture the single original POP before the NOP consumes its
                    // transferred slot. Only the immediately adjacent authored SETUP
                    // may use this proof; another instruction or boundary retires it.
                    // Its explicit exclusion changes neither original record nor slot.
                    transferred_pop_setup = None;
                    if instruction.opcode == NOP
                        && !instruction.fusion_policy.is_converted_pop_block()
                        && matches!(
                            instruction.exception_policy(),
                            super::ExceptionPolicy::NoneUnchanged
                                | super::ExceptionPolicy::AlwaysUnchanged
                        )
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
                            // Early compaction leaves the retained SETUP in its original
                            // NULL cell. Record that physical index before consuming the
                            // adjacent removed POP; later append must reuse this exact cell.
                            if let Some(slot) = instruction_index
                                .checked_add(tail.early_removals)
                                .and_then(|index| index.checked_add(1))
                            {
                                transferred_setup_null_slot = Some((
                                    original_array_start,
                                    instruction_index,
                                    slot,
                                    tail.early_removals + 1,
                                ));
                            }
                        }
                    }
                    // The authored return NOP compacts into iterator disposal.
                    // Replay that exact survivor before counting its hidden NOP;
                    // its original owned cell follows the authored cell by one.
                    if instruction.opcode == POP_TOP
                        && instruction.local_policy.hidden_exception_owner_slots() == 1
                        && instruction.pre_fold_extra_instructions() == 0
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && original_array_start < instruction_index
                        && let Some(region) = self.uniform_exception_owner(
                            instruction_index,
                            instruction_index + 1,
                            &label_positions,
                        )
                        && !self.exception_regions[region].is_scope_fallback
                        && !self.exception_regions[region].preserve_lasti
                        && self
                            .exception_owner_provenance
                            .early_return_iterator_pops
                            .iter()
                            .filter(|(_, iterator, _, target)| {
                                let mut expected = *iterator;
                                expected.local_policy.add_hidden_exception_owner_slots(1);
                                expected == *instruction
                                    && self.exception_regions[region].target == *target
                            })
                            .count()
                            == 1
                        && indexed_instructions
                            .iter()
                            .filter(|record| ***record == *instruction)
                            .count()
                            == 1
                        && let Some(slot) = instruction_index
                            .checked_add(tail.early_removals)
                            .and_then(|slot| slot.checked_add(1))
                    {
                        early_return_iterator_slot =
                            Some((original_array_start, instruction_index, slot, region));
                    }
                    if instruction.opcode == NOP {
                        early_return_iterator_slot = None;
                        early_return_nop_slot = None;
                        early_return_truth_slot = None;
                        if owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && matches!(
                                instruction.local_policy.hidden_exception_owner_slots(),
                                1..=3
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
                                // The authored return NOP and ordinary-try POPs compact
                                // into the finally POP. The authored cell stays owned;
                                // every captured POP cell remains NULL.
                                self.exception_owner_provenance
                                    .copied_return_none_prefixes
                                    .iter()
                                    .filter(|origin| {
                                        let removed = if origin.additional_pop.is_some() {
                                            3
                                        } else {
                                            2
                                        };
                                        let mut expected_pop = origin.pops[1];
                                        expected_pop
                                            .local_policy
                                            .add_hidden_exception_owner_slots(removed);
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
                                        && !owners.contains_key(*index)
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
                    // A nested retained SETUP keeps the preceding SETUP's protected
                    // suffix in the same original array. Preserve that exact owned
                    // prefix before the new live NOP; the existing slot arithmetic
                    // must still place a later append strictly inside this prefix.
                    let retained_setup_prefix = || {
                        retained_owned_suffix.filter(|(start, _)| {
                            retained_epoch_known
                                && pop_slot_epoch_known
                                && retained_setup_nops.contains(&instruction_index)
                                && !instruction.fusion_policy.is_converted_pop_block()
                                && tail.insertions == 0
                                && tail.late_removals == 0
                                && tail.post_compaction_fusions == 0
                                && original_array_start <= *start
                                && *start < instruction_index
                                && self
                                    .uniform_exception_owner(
                                        *start,
                                        instruction_index,
                                        &label_positions,
                                    )
                                    .is_some_and(|region| {
                                        !self.exception_regions[region].is_scope_fallback
                                    })
                        })
                    };
                    let clean_nop = instruction.opcode == NOP
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                        && instruction.pre_fold_extra_instructions() == 0
                        && owner_history_exact;
                    let setup_prefix = if GATE_DECISIONS {
                        clean_nop
                            .then(|| source_owned_prefix.or_else(retained_setup_prefix))
                            .flatten()
                    } else {
                        // The ungated test oracle also keeps the original eager proof.
                        let retained_setup_prefix = retained_setup_prefix();
                        clean_nop
                            .then_some(source_owned_prefix.or(retained_setup_prefix))
                            .flatten()
                    };
                    pending_retained_prefix = setup_prefix
                        .map(|(start, start_tail)| (start, instruction_index, start_tail, tail))
                        // This exact POP/SETUP pair also retains the earlier cleanup's
                        // donor span. The SETUP cell itself remains unowned and lies
                        // outside that span; its exclusion does not erase history.
                        .or(transferred_pop_setup.and(ended_owned_prefix));
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
                        captured_method_owner_slots.clear();
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
                    if (!GATE_DECISIONS
                        || instruction.local_policy.hidden_exception_owner_slots() > 0)
                        && captured_pop_slots.get(&instruction_index) == Some(&original_array_start)
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                    {
                        // Literal replay authenticated every adjacent POP in this run.
                        for offset in 0..instruction.local_policy.hidden_exception_owner_slots() {
                            captured_null_slots.push((
                                instruction_index,
                                tail.early_removals + usize::from(offset),
                            ));
                        }
                    }
                    // Locate the two original NULL cells before their transferred count
                    // is consumed. A surrounding manager has not unwound at this point.
                    // Literal replay of its unique original entry and successor keeps
                    // folded records elsewhere in the prefix from becoming POP witnesses.
                    if owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction.local_policy.hidden_exception_owner_slots() == 2
                    {
                        for origin in &self.exception_owner_provenance.copied_return_pop_prefixes {
                            let mut expected = origin.records[2];
                            expected.local_policy.add_hidden_exception_owner_slots(2);
                            let provenance = &self.exception_owner_provenance;
                            if *instruction == expected
                                && label_positions.get(&origin.entry) == Some(&original_array_start)
                                && provenance.block_boundaries.contains(&origin.entry)
                                && unique_labels
                                    .get_or_init(|| UniqueLabelPositions::new(&self.items))
                                    .get(origin.entry)
                                    .is_some()
                                && indexed_instructions
                                    .iter()
                                    .filter(|candidate| ***candidate == expected)
                                    .count()
                                    == 1
                                && !provenance.is_normal_try_copy_label(origin.entry)
                                && !provenance
                                    .retired_normal_try_copy_labels
                                    .contains(&origin.entry)
                            {
                                captured_null_slots.push((instruction_index, tail.early_removals));
                                captured_null_slots
                                    .push((instruction_index, tail.early_removals + 1));
                            }
                        }
                    }
                    let previous_fusions = tail.post_compaction_fusions;
                    tail.instruction(instruction, previous);
                    // The exact named-handler POP span also certifies its surviving
                    // SWAP and POP_EXCEPT cells. After counting the preceding erased
                    // POP, this is the survivor's old slot, with its own structural owner.
                    if (!GATE_DECISIONS
                        || (instruction.local_policy.hidden_exception_owner_slots() == 1
                            && matches!(
                                instruction.opcode,
                                crate::target::opcodes::SWAP | crate::target::opcodes::POP_EXCEPT
                            )))
                        && captured_pop_slots.get(&instruction_index) == Some(&original_array_start)
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
                        && !owners.contains_key(instruction_index)
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
                    // A source-captured method cleanup retains its final POP_TOP cell.
                    // Keep its old slot relative to the actual native array, including
                    // every earlier setup and compaction; authored try labels do not split it.
                    if instruction.opcode == crate::target::opcodes::POP_TOP
                        && instruction.operand() == Operand::Value(0)
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && let Some(receiver) = instruction_index.checked_sub(3)
                        && captured_method_donors.get(&instruction_index)
                            == Some(&original_array_start)
                        && indexed_instructions[receiver]
                            .local_policy
                            .hidden_exception_owner_slots()
                            == 2
                        && matches!(
                            indexed_instructions[receiver].opcode,
                            crate::target::opcodes::LOAD_FAST
                                | crate::target::opcodes::LOAD_FAST_BORROW
                        )
                        && indexed_instructions[receiver + 1].opcode
                            == crate::target::opcodes::LOAD_ATTR
                        && matches!(indexed_instructions[receiver + 1].operand(), Operand::Value(argument) if argument & 1 != 0)
                        && indexed_instructions[receiver + 2].opcode == crate::target::opcodes::CALL
                        && indexed_instructions[receiver + 2].operand() == Operand::Value(0)
                        && (receiver..=instruction_index).all(|index| !owners.contains_key(index))
                        && let Some(region) = self.uniform_exception_owner(
                            receiver,
                            instruction_index + 1,
                            &label_positions,
                        )
                        && !self.exception_regions[region].is_scope_fallback
                        && !self.exception_regions[region].preserve_lasti
                        && self.exception_regions[region].depth == 0
                        && let Some(slot) = instruction_index.checked_add(tail.early_removals)
                    {
                        captured_method_owner_slots.push((
                            original_array_start,
                            slot,
                            region,
                            instruction_index + 2,
                        ));
                    }
                    // Uniformity can change the following histories only for an
                    // instruction with erased cells of its own.
                    let uniform_fold = (!GATE_DECISIONS
                        || instruction.local_policy.hidden_exception_owner_slots() > 0
                        || instruction.pre_fold_extra_instructions() > 0)
                        && (original_uniform_folds.contains(&instruction_index)
                            || (instruction.local_policy.hidden_exception_owner_slots() > 0
                                && self
                                    .exception_owner_provenance
                                    .uniform_fold_owner_slots
                                    .iter()
                                    .any(|(location, opcode, slots)| {
                                        instruction.location == *location
                                            && instruction.opcode == *opcode
                                            && instruction
                                                .local_policy
                                                .hidden_exception_owner_slots()
                                                == *slots
                                            && instruction.pre_fold_extra_instructions()
                                                == (*slots).min(4) as u8
                                    })));
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
                        && (!GATE_DECISIONS
                            || instruction.local_policy.hidden_exception_owner_slots() > 0)
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
                    // All histories above are needed by later markers. The remaining
                    // decision rules are entered only for markers, conditions, dynamic
                    // policies, indexed source certificates, or a pending threaded append.
                    // Restrict the shortcut to instructions whose final block update also
                    // leaves those histories intact.
                    if GATE_DECISIONS
                        && decision_indices.can_finish_ordinary(
                            instruction,
                            instruction_index,
                            threaded_null_append,
                        )
                    {
                        instruction_index += 1;
                        has_instruction = true;
                        previous = Some(instruction);
                        continue;
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
                            // Local fusion leaves a newer tail than peephole compaction.
                            // Boolean COPY/TO_BOOL expansion consumes that tail first; use
                            // its geometry while a fusion slot remains for this marker.
                            let (removed, setup_removed) = if instruction.exception_policy().owner()
                                == NormalizedExceptionOwner::Dynamic
                                && owner_history_exact
                                && tail.post_compaction_fusions > tail.insertions + insertions
                            {
                                (
                                    tail.post_compaction_fusions,
                                    setup_tail.post_compaction_fusions,
                                )
                            } else {
                                (tail.early_removals, setup_tail.early_removals)
                            };
                            let protected_original_instructions =
                                (instruction_index - start + removed - setup_removed)
                                    .saturating_sub(
                                        insertions + tail.insertions - setup_tail.insertions,
                                    );
                            let stale_slots = removed.saturating_sub(tail.insertions + insertions);
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
                            // Fusion leaves its newer stale tail after early compaction.
                            // If no records changed after this retained SETUP, exactly one
                            // slot before its protected suffix is the original NULL NOP.
                            let fused_retained_setup_cell = instruction.opcode == NOT_TAKEN
                                && instruction.exception_policy().owner()
                                    == NormalizedExceptionOwner::DynamicCondition
                                && owner_history_exact
                                && pop_slot_epoch_known
                                && tail.insertions == 0
                                && tail.post_compaction_fusions > 0
                                && tail.post_compaction_fusions
                                    == setup_tail.post_compaction_fusions
                                && tail.early_removals == setup_tail.early_removals
                                && tail
                                    .post_compaction_fusions
                                    .checked_sub(instruction_index - start)
                                    == Some(1)
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
                            // More prefix fusions can place the append before SETUP,
                            // inside the same original entry's entirely NULL prefix.
                            // No suffix rewrite may change the fusion-stage coordinates.
                            let fused_unprotected_prefix = instruction.opcode == NOT_TAKEN
                                && instruction.exception_policy().owner()
                                    == NormalizedExceptionOwner::DynamicCondition
                                && original_array_start == 0
                                && owner_history_exact
                                && pop_slot_epoch_known
                                && tail.insertions == 0
                                && tail.post_compaction_fusions
                                    == setup_tail.post_compaction_fusions
                                && tail.early_removals == setup_tail.early_removals
                                && tail
                                    .post_compaction_fusions
                                    .checked_sub(instruction_index - start)
                                    .is_some_and(|before_suffix| before_suffix > 1)
                                && null_original_entry_prefix(start, setup_label, &owners);
                            has_owner &= !(fused_retained_setup_cell || fused_unprotected_prefix);
                            proven_unprotected_slot |=
                                fused_retained_setup_cell || fused_unprotected_prefix;
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
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive());
                        // Presence is already proved for a value-preserving Boolean
                        // marker. Retained array provenance supplies its exact source owner.
                        let source_region = (has_owner
                            && instruction.exception_policy().owner()
                                == NormalizedExceptionOwner::Dynamic
                            && instruction_index > 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| owner.source_region().is_none()))
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
                        let owner = if let Some(region) = source_region {
                            ExceptionOwner::SourceRegion(region)
                        } else if has_owner {
                            ExceptionOwner::Present
                        } else {
                            ExceptionOwner::Absent
                        };
                        let decision = if proven_unprotected_slot
                            || prologue_consumed_all_slots
                            || instruction.exception_policy().owner()
                                != NormalizedExceptionOwner::DynamicCondition
                        {
                            ExceptionOwnerDecision::Definitive(owner)
                        } else {
                            ExceptionOwnerDecision::Tentative(owner)
                        };
                        owners.insert(instruction_index, decision);
                    }
                    // These replay rules only resolve inserted markers.
                    if instruction.opcode == NOT_TAKEN {
                        // Primitive match tests keep their unchanged marker policy. They
                        // still append a fresh NULL record once entry-prologue insertion
                        // consumes every exact early-compaction slot in the original array.
                        if instruction.exception_policy() == super::ExceptionPolicy::NoneUnchanged
                            && has_entry_prologue
                            && instruction.local_policy.hidden_exception_owner_slots() == 0
                            && instruction.pre_fold_extra_instructions() == 0
                            && original_array_start == 0
                            && owner_history_exact
                            && tail.insertions > 0
                            && tail.early_removals <= tail.insertions
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::Definitive(ExceptionOwner::Absent),
                            );
                        }
                        // Local fusion leaves a newer stale suffix inside the original array.
                        // Entry prologue insertion consumes that suffix first; once all fusion
                        // slots are used, the marker reuses the older peephole-compaction tail.
                        if instruction.exception_policy().owner()
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
                                                        && !owners.contains_key(index)
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
                                                && nop.local_policy.hidden_exception_owner_slots()
                                                    == 0
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
                                    .is_some_and(|remaining| {
                                        remaining > protected_before_compaction
                                    })
                                && null_original_entry_prefix(start, setup_label, &owners);
                            if prologue_consumed_all_slots
                                || prologue_reuses_null_setup
                                || prologue_reuses_null_prefix
                                || (tail.insertions == 0 && removed > protected_before_compaction)
                            {
                                owners.insert(
                                    instruction_index,
                                    ExceptionOwnerDecision::Definitive(ExceptionOwner::Absent),
                                );
                            }
                        }
                        // A retained SETUP may follow an owned source record and one erased
                        // outer POP. An exact append slot can still belong to that earlier
                        // handler, even though it falls before the new protected suffix.
                        if instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && let Some((start, setup_tail, setup)) = protected_suffix
                            && setup_tail.insertions == 0
                            && setup_tail.late_removals == 0
                            && setup_tail.post_compaction_fusions == 0
                            && tail.early_removals.checked_sub(setup_tail.early_removals) == Some(3)
                            && let Some(region) = self.retained_setup_predecessor_owner(
                                instruction_index,
                                original_array_start,
                                start,
                                setup,
                                setup_tail.early_removals,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                                &owners,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        // A retained SETUP can follow removals owned by an outer handler. Only
                        // certify a reused inner owner when the last compaction lands entirely
                        // within a known, uniformly owned suffix; other outcomes remain unknown.
                        if matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::DynamicCondition
                        ) && tail.insertions == 0
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
                                    ExceptionOwnerDecision::Override(ExceptionOwner::Present),
                                );
                            }
                        }
                        // A retained SETUP fixes this original array's left edge. Local
                        // fusion leaves its old suffix in place even when an intervening
                        // POP_BLOCK changes the owner. The last F surviving records span at
                        // least F old records, so a uniform clean suffix of that width proves
                        // the owner of a marker appended after F fusion removals.
                        if instruction.exception_policy().owner()
                            == NormalizedExceptionOwner::Absent
                            && instruction.exception_policy().exclusion()
                                == super::ExceptionExclusion::None
                            && owner_history_exact
                            && !owners.contains_key(instruction_index)
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
                                            && previous.local_policy.hidden_exception_owner_slots()
                                                == 0
                                            && previous.pre_fold_extra_instructions() == 0
                                            && !matches!(
                                                previous.opcode,
                                                LOAD_FAST_BORROW_LOAD_FAST_BORROW
                                                    | LOAD_FAST_LOAD_FAST
                                                    | STORE_FAST_LOAD_FAST
                                                    | STORE_FAST_STORE_FAST
                                            )
                                            && !owners.contains_key(suffix_start + offset)
                                    })
                            {
                                owners.insert(
                                    instruction_index,
                                    ExceptionOwnerDecision::override_with_region(Some(region)),
                                );
                            }
                        }
                        // A late SETUP removal compacts the live array after prologue insertion.
                        // With L removed SETUPs and W surviving records in the final uniform
                        // manager-owned suffix, L <= W places the appended slot inside that
                        // suffix. Its owner is the preceding condition's manager even when an
                        // outer generator handler already structurally covers the marker.
                        if matches!(
                            instruction.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        ) && uniform_with_suffix.is_some_and(|start| {
                            tail.late_removals > 0
                                && tail.late_removals <= instruction_index - start
                        }) {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::Override(ExceptionOwner::Present),
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
                            && previous.is_some_and(|previous: &Instruction| {
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
                        if instruction.exception_policy().exclusion()
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
                            && let Some(region) = self.uniform_exception_owner(
                                start,
                                instruction_index,
                                &label_positions,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        if source_block && tail.early_removals == 0 && tail.late_removals == 0 {
                            // This conditional's original array begins at a source join and has
                            // no removed records. Appending its marker allocates a NULL owner.
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::Definitive(ExceptionOwner::Absent),
                            );
                        }
                        // Fusion leaves stale records in a uniformly owned original source
                        // array. A marker appended into that suffix inherits the same owner.
                        if source_block
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions > 0
                            && instruction.exception_policy().exclusion()
                                == super::ExceptionExclusion::None
                            && let Some((start, _)) = source_owned_prefix
                            && let Some(region) = self.uniform_exception_owner(
                                start,
                                instruction_index,
                                &label_positions,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        // Late SETUP compaction can leave an ordinary-try record behind
                        // a manager return. Its owner survives in the appended marker slot.
                        if instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && original_array_start == 0
                            && owner_history_exact
                            && matches!(tail.insertions, 0 | 2)
                            && tail.late_removals == 3
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && let Some(region) = self.entry_manager_return_setup_owner(
                                instruction_index,
                                tail.insertions,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                                &late_setup_positions,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        // This source certificate identifies the retained authored return
                        // NOP and every live prefix record. It proves only this NULL POP cell;
                        // the generic POP epoch remains unknown after unrelated NOPs.
                        if owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && self.retained_manager_return_none_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(None),
                            );
                        }
                        // The original manager POP precedes its cleanup. A later local-load
                        // fusion compacts the live array again, superseding those early slots.
                        if matches!(
                            instruction.exception_policy(),
                            super::ExceptionPolicy::NonePresent
                                | super::ExceptionPolicy::NoneAbsent
                        ) && owner_history_exact
                            && pop_slot_epoch_known
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && let Some(decision) = self.copied_manager_return_compaction_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                                false,
                            )
                        {
                            owners.insert(instruction_index, decision);
                        }
                        // A captured truth-manager cleanup also authenticates a retained
                        // authored return NOP. Its exact records provide their own proof;
                        // the generic POP epoch stays unknown for every other retained NOP.
                        if instruction.exception_policy()
                            == super::ExceptionPolicy::NoneDynamicCondition
                            && previous.is_some_and(|condition: &Instruction| {
                                matches!(
                                    condition.opcode,
                                    crate::target::opcodes::POP_JUMP_IF_FALSE
                                        | crate::target::opcodes::POP_JUMP_IF_TRUE
                                )
                            })
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && let Some(decision) = self.copied_manager_return_compaction_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                                false,
                            )
                        {
                            owners.insert(instruction_index, decision);
                        }
                        // This exact adjacent authored-NOP/inner-try-POP pair certifies the
                        // retained NULL cell independently of the generic POP epoch.
                        if instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && let Some(decision) = self.copied_manager_return_compaction_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                                true,
                            )
                        {
                            owners.insert(instruction_index, decision);
                        }
                        // A retained return POP can hide two earlier NULL POP records.
                        // Its captured call prefix proves that sufficiently early compaction
                        // reuses the preceding try's array, without restoring the POP epoch.
                        if instruction.exception_policy()
                            == super::ExceptionPolicy::NoneDynamicCondition
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && let Some(region) = self.copied_return_call_parent_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        // A retained return POP can hide two earlier NULL POP records.
                        // Its captured attribute prefix locates the original return and POPs;
                        // sufficiently early compaction
                        // reuses the preceding try's array, without restoring the POP epoch.
                        if instruction.exception_policy()
                            == super::ExceptionPolicy::NoneDynamicCondition
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && let Some(decision) = self.copied_return_attribute_truth_owner(
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
                    }
                    let method_none_prefix =
                        if matches!(instruction.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                            && owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                        {
                            self.normal_finally_method_none_prefix(
                                instruction_index,
                                original_array_start,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        } else {
                            None
                        };
                    // The marker after a method call is two records later than a local
                    // None marker. Keep the preceding try's exact POP boundary until its
                    // old slot is resolved; the marker's range labels may end that epoch.
                    let original_method_none_marker = method_none_prefix
                        .filter(|(load_index, removed_pop)| {
                            *removed_pop == 1
                                && retained_epoch_known
                                && pop_slot_epoch_known
                                && ended_owned_prefix.is_some_and(|(start, end, _, _)| {
                                    start >= original_array_start && end == *load_index
                                })
                                && ended_pop_slot.is_some_and(|(start, _, width)| {
                                    start == *load_index && width == 1
                                })
                        })
                        .and_then(|_| indexed_instructions.get(instruction_index + 1))
                        .filter(|marker| marker.opcode == NOT_TAKEN)
                        .map(|marker| (instruction_index + 1, **marker));
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
                    // A truth condition has the same authored normal POP, but its
                    // separate TO_BOOL leaves the marker one record later. Resolve
                    // its exact captured prefix before the marker's exclusion label.
                    let original_truth_marker = if matches!(
                        instruction.opcode,
                        crate::target::opcodes::POP_JUMP_IF_FALSE
                            | crate::target::opcodes::POP_JUMP_IF_TRUE
                    ) && owner_history_exact
                        && retained_epoch_known
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && instruction_index >= 2
                        && ended_owned_prefix.is_some_and(|(start, end, _, _)| {
                            start >= original_array_start && end == instruction_index - 2
                        })
                        && ended_pop_slot.is_some_and(|(start, _, width)| {
                            start == instruction_index - 2 && width == 1
                        })
                        && self
                            .exception_owner_provenance
                            .normal_finally_truth_prefixes
                            .iter()
                            .filter(|[_, original_load, original_to_bool, original_condition]| {
                                let load = *indexed_instructions[instruction_index - 2];
                                let mut expected_load = *original_load;
                                expected_load
                                    .local_policy
                                    .add_hidden_exception_owner_slots(1);
                                if load.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                                    expected_load.opcode = load.opcode;
                                }
                                load == expected_load
                                    && *indexed_instructions[instruction_index - 1]
                                        == *original_to_bool
                                    && instruction == original_condition
                            })
                            .count()
                            == 1
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
                        .or(original_method_none_marker)
                        .or(original_truth_marker)
                        && marker.exception_policy().exclusion() == super::ExceptionExclusion::None
                        && matches!(
                            marker.exception_policy().owner(),
                            NormalizedExceptionOwner::Absent
                                | NormalizedExceptionOwner::Present
                                | NormalizedExceptionOwner::Dynamic
                                | NormalizedExceptionOwner::DynamicCondition
                                | NormalizedExceptionOwner::DynamicConditionStrict
                        )
                        && (marker.exception_policy().owner() != NormalizedExceptionOwner::Dynamic
                            || original_value_jump_positions
                                .values()
                                .any(|position| *position == marker_index))
                        && owner_history_exact
                        && tail.late_removals == 0
                        && let Some((start, end, start_tail, end_tail)) = ended_owned_prefix
                        && (tail.insertions == 0
                            || (start_tail.insertions == tail.insertions
                                && (tail.post_compaction_fusions <= tail.insertions
                                    || original_array_start == 0)))
                    {
                        // Prologue insertion consumes the newer fusion suffix first. Once
                        // exhausted, the marker reuses the earlier compaction tail, as it
                        // does for a protected suffix that has not reached its cleanup.
                        // The complete Boolean expansion consumes two newer fusion slots.
                        // Its records already occur in marker_index and the suffix width.
                        let conditional_insertions = if marker.exception_policy().owner()
                            == NormalizedExceptionOwner::Dynamic
                        {
                            2
                        } else {
                            0
                        };
                        let (removed, suffix_width, prefix_width) = if tail.post_compaction_fusions
                            > tail.insertions + conditional_insertions
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
                                    ExceptionOwnerDecision::override_with_region(Some(region)),
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
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                    }
                    // An unchanged authored conditional array appends into a fresh NULL
                    // cell. Certify its original terminator before a marker-only range
                    // label obscures the array epoch; no removed cell can be reused.
                    if is_conditional_jump(instruction.opcode)
                        && matches!(instruction.operand(), Operand::Forward(_))
                        && instruction.exception_policy() == super::ExceptionPolicy::NoneUnchanged
                        && owner_history_exact
                        && retained_epoch_known
                        && pop_slot_epoch_known
                        && tail.early_removals == 0
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && original_array_start > 0
                        && self
                            .exception_owner_provenance
                            .block_boundaries
                            .iter()
                            .any(|label| label_positions.get(label) == Some(&original_array_start))
                        && self
                            .exception_owner_provenance
                            .original_primitive_condition_boundaries
                            .iter()
                            .filter(|origin| **origin == *instruction)
                            .count()
                            == 1
                        && indexed_instructions
                            .iter()
                            .filter(|record| ***record == *instruction)
                            .count()
                            == 1
                        && !boundaries()
                            .chain(&jump_targets)
                            .chain(&self.exception_owner_provenance.source_boundaries)
                            .chain(&self.exception_owner_provenance.removed_pop_block_boundaries)
                            .chain(setups.keys())
                            .any(|label| {
                                label_positions.get(label) == Some(&(instruction_index + 1))
                            })
                        && let Some(marker) = indexed_instructions.get(instruction_index + 1)
                        && marker.opcode == NOT_TAKEN
                        && marker.exception_policy() == super::ExceptionPolicy::NonePresent
                        && marker.location == instruction.location
                        && marker.depth_after() == instruction.depth_after()
                        && marker.pre_fold_extra_instructions() == 0
                        && marker.local_policy.hidden_exception_owner_slots() == 0
                        && marker.no_location_policy.borrow_fallthrough()
                            != super::BorrowFallthrough::Barrier
                        && owners
                            .get(instruction_index + 1)
                            .is_none_or(|owner| !owner.is_definitive())
                    {
                        owners.insert(
                            instruction_index + 1,
                            ExceptionOwnerDecision::override_with_region(None),
                        );
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
                        && !owners.contains_key(instruction_index)
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
                            ExceptionOwnerDecision::override_with_region(Some(*region)),
                        );
                    }
                    // Exact copied-cleanup donors outrank generic absent-owner guesses.
                    // Retain the conclusion until those broader rules have finished.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && previous.is_some_and(|condition| {
                            matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
                                && matches!(condition.operand(), Operand::Forward(_))
                        })
                        && let Some((_, _, region, _)) = captured_method_owner_slots.iter().find(
                            |(array, slot, _, condition)| {
                                *array == original_array_start
                                    && *slot == instruction_index
                                    && *condition + 1 == instruction_index
                            },
                        )
                    {
                        certified_method_markers.insert(
                            instruction_index,
                            ExceptionOwnerDecision::override_with_region(Some(*region)),
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
                            ExceptionOwnerDecision::override_with_region(None),
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
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive()))
                            || (reuses_null_setup
                                && owners
                                    .get(instruction_index)
                                    .is_none_or(|owner| !owner.is_definitive()))
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(source_region),
                            );
                        }
                    }
                    // A bare return through two unnamed handlers retains both POP_EXCEPTs.
                    // Its late marker reuses the second POP_EXCEPT's original array cell.
                    if instruction.opcode == NOT_TAKEN
                        && owner_history_exact
                        && pop_slot_epoch_known
                        && tail.early_removals == 7
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && owners
                            .get(instruction_index)
                            .is_none_or(|owner| !owner.is_definitive())
                        && let Some(region) = self.copied_bare_handler_none_owner(
                            instruction_index,
                            original_array_start,
                            &indexed_instructions,
                            &label_positions,
                            &jump_targets,
                        )
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision::override_with_region(Some(region)),
                        );
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
                                || owners.contains_key(pop_index)
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
                        && !owners.contains_key(instruction_index - 1)
                        && !owners.contains_key(instruction_index)
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
                            .filter(|origin| origin.additional_pop.is_none())
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
                                            && !owners.contains_key(index)
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
                    if let Some((load_index, removed_pop)) = method_none_prefix
                        && tail.early_removals == 3 + usize::from(removed_pop)
                        && (load_index..=instruction_index).all(|index| !owners.contains_key(index))
                        && let Some(region) = self.uniform_exception_owner(
                            load_index,
                            instruction_index + 1,
                            &label_positions,
                        )
                        && !self.exception_regions[region].is_scope_fallback
                    {
                        // With only this POP and comparison removed, normalization appends
                        // into a method-test cell. Earlier compaction uses the protected
                        // prefix path above instead of borrowing this suffix's owner.
                        pending_normal_none_owner = Some((instruction_index + 1, Some(region)));
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
                            && !boundaries()
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
                            .get(instruction_index)
                            .is_none_or(|owner| !owner.is_definitive() || owner.has_owner())
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision::override_with_region(Some(region)),
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
                            ExceptionOwnerDecision::override_with_region(None),
                        );
                    }
                    // This independent source certificate proves every erased cell in
                    // a direct-name return's membership array. It never revives the
                    // generic history flag invalidated by an uncertified literal fold.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                        && owners
                            .get(instruction_index)
                            .is_none_or(|owner| !owner.is_definitive())
                        && let Some(owner) = self.copied_return_membership_owner(
                            instruction_index,
                            original_array_start,
                            tail,
                            &indexed_instructions,
                            &label_positions,
                            &jump_targets,
                        )
                    {
                        owners.insert(instruction_index, owner);
                    }
                    // The captured marker aliases may delimit exception ranges without
                    // introducing another original array or invalidating its fresh cell.
                    if original_normal_finally_null_markers.contains(&instruction_index) {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision::override_with_region(None),
                        );
                    }
                    // The retained SETUP's original cell is NULL, even if its compacted
                    // live record is surrounded by protected instructions. An appended
                    // marker may reuse it only when no subsequent edit shifts that cell.
                    if instruction.opcode == NOT_TAKEN
                        && instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                        && owner_history_exact
                        && tail.insertions == 0
                        && tail.late_removals == 0
                        && tail.post_compaction_fusions == 0
                        && let Some((array, setup, slot, removals)) = transferred_setup_null_slot
                        && array == original_array_start
                        && slot == instruction_index
                        && removals == tail.early_removals
                        && instruction_index > setup
                        && indexed_instructions
                            .get(instruction_index - 1)
                            .is_some_and(|jump| {
                                is_conditional_jump(jump.opcode)
                                    && matches!(jump.operand(), Operand::Forward(_))
                                    && jump.location == instruction.location
                            })
                        && !jump_targets.iter().any(|label| {
                            label_positions.get(label).is_some_and(|position| {
                                setup < *position && *position <= instruction_index
                            })
                        })
                        && owners
                            .get(instruction_index)
                            .is_none_or(|owner| !owner.is_definitive())
                    {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision::override_with_region(None),
                        );
                    }
                    if retained_setup_null_markers.contains(&instruction_index) {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision::override_with_region(None),
                        );
                    }
                    if retained_setup_value_null_markers.contains(&instruction_index) {
                        owners.insert(
                            instruction_index,
                            ExceptionOwnerDecision::override_with_region(None),
                        );
                    }
                    if instruction.opcode == NOT_TAKEN {
                        if instruction.exception_policy().exclusion()
                            == super::ExceptionExclusion::None
                            && let Some((index, region)) = pending_normal_none_owner.take()
                            && index == instruction_index
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(region),
                            );
                        }
                        if instruction_index > 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
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
                                        ExceptionOwnerDecision::override_with_region(Some(
                                            source_region,
                                        )),
                                    );
                                }
                            }
                        }
                        // A complete value-Boolean expansion consumes up to two newer
                        // fusion slots. Its marker can then reuse an older named-handler
                        // cleanup cell, whose owner differs from the copied finalizer.
                        if instruction.exception_policy() == super::ExceptionPolicy::NoneDynamic
                            && owner_history_exact
                            && pop_slot_epoch_known
                            && original_value_jump_positions
                                .values()
                                .any(|position| *position == instruction_index)
                            && owners.get(instruction_index).is_none_or(|decision| {
                                !decision.is_definitive()
                                    || (decision.has_owner()
                                        && decision.source_region().is_none()
                                        && !decision.overrides_existing())
                            })
                            && let Some(region) = self.copied_handler_value_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        // A captured named-handler prefix crosses only its own retained
                        // SETUP; it does not restore the generic POP epoch that NOP ends.
                        if instruction.exception_policy()
                            == super::ExceptionPolicy::NoneDynamicCondition
                            && owner_history_exact
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && let Some(region) = self.named_handler_try_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        if instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && owner_history_exact
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && self
                                .continued_handler_try_is_null(
                                    instruction_index,
                                    original_array_start,
                                    tail,
                                    &indexed_instructions,
                                    &label_positions,
                                    &jump_targets,
                                )
                                .is_some()
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(None),
                            );
                        }
                        // This saved-value cleanup certifies two omitted handler POPs and
                        // the exact later finalizer NULL cell without changing generic history.
                        if instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && owner_history_exact
                            && pop_slot_epoch_known
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && self.saved_named_handler_none_is_null(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(None),
                            );
                        }
                        // A complete captured unwind supplies its own original array proof;
                        // it does not revive the generic POP epoch after manager range aliases.
                        if instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && owner_history_exact
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && let Some(decision) = self.copied_cleanup_return_none_owner(
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
                        // Resolve only a concrete live cleanup record in the original array.
                        // Missing named-handler POPs precede this span; all existing hidden
                        // records remain counted exactly once by the ordinary tail scanner.
                        if owner_history_exact
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
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && let Some((
                                array,
                                start,
                                end,
                                end_removed,
                                missing,
                                continued_call,
                                continued_attribute_none,
                            )) = copied_handler_prefix
                            && array == original_array_start
                            && end <= instruction_index
                            && tail.early_removals >= end_removed
                            && (if let Some((prefix, marker)) = continued_attribute_none {
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
                            } else if let Some((prefix, marker)) = continued_call {
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
                                previous.is_some_and(|previous: &Instruction| {
                                    let none_test =
                                        matches!(
                                            previous.opcode,
                                            POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
                                        ) && previous.local_policy.hidden_exception_owner_slots() == 3;
                                    // Both handler forms retain an authenticated cleanup prefix.
                                    // A named handler also contributes two omitted NULL POPs;
                                    // its name cleanup remains four distinct live records.
                                    // An inner iterator consumes the authored return NOP,
                                    // while its ordinary try and the named handler contribute
                                    // three omitted NULL POPs before the unchanged name cleanup.
                                    // The original capture already revalidated that iterator and
                                    // every cleanup record at this exact start/end pair.
                                    let iterator_handler_truth_prefix = missing == 3
                                        && end - start == 4
                                        && end_removed == 1
                                        && self.exception_owner_provenance
                                            .copied_handler_cleanups.iter()
                                            .filter(|origin| {
                                                origin.missing_null_pops == 3
                                                    && origin.removed_prefix_slots == 0
                                                    && origin.return_iterator.is_some()
                                                    && origin.instructions.len() == 4
                                                    && origin.assignment_pop.is_none()
                                                    && origin.continued_call.is_none()
                                                    && origin.continued_attribute_none.is_none()
                                                    && label_positions.get(&origin.start) == Some(&start)
                                                    && label_positions.get(&origin.end) == Some(&end)
                                                    && self.exception_owner_provenance
                                                        .removed_pop_block_boundaries.contains(&origin.end)
                                                    && self.exception_owner_provenance
                                                        .removed_pop_block_widths.get(&origin.end)
                                                        .copied().unwrap_or(1) == 1
                                            }).count() == 1;
                                    let handler_truth_prefix = (missing == 0
                                        && end - start == 1
                                        && indexed_instructions[start].opcode
                                            == crate::target::opcodes::POP_EXCEPT
                                        && indexed_instructions[start]
                                            .local_policy
                                            .hidden_exception_owner_slots()
                                            == 2)
                                        || (missing == 2
                                            && end - start == 4
                                            && end_removed == 1
                                            && indexed_instructions[start].opcode
                                                == crate::target::opcodes::POP_EXCEPT
                                            && indexed_instructions[start]
                                                .local_policy
                                                .hidden_exception_owner_slots()
                                                == 1
                                            && indexed_instructions[start + 1].opcode == crate::target::opcodes::LOAD_CONST
                                            && indexed_instructions[start + 2].opcode
                                                == crate::target::opcodes::STORE_FAST
                                            && indexed_instructions[start + 3].opcode
                                                == crate::target::opcodes::DELETE_FAST
                                            && indexed_instructions[start + 2].operand()
                                                == indexed_instructions[start + 3].operand())
                                        || iterator_handler_truth_prefix;
                                    // Authenticate the single finalizer POP and every live
                                    // suffix record after the captured handler cleanup.
                                    let truth_test = handler_truth_prefix
                                        && instruction_index - end == 3
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
                                        && indexed_instructions[end + 1].opcode
                                            == crate::target::opcodes::TO_BOOL
                                        && indexed_instructions[end + 1]
                                            .local_policy
                                            .hidden_exception_owner_slots()
                                            == 0
                                        && matches!(
                                            previous.opcode,
                                            crate::target::opcodes::POP_JUMP_IF_FALSE
                                                | crate::target::opcodes::POP_JUMP_IF_TRUE
                                        )
                                        && previous.local_policy.hidden_exception_owner_slots() == 0
                                        && previous.location.line >= 0
                                        && instruction.location == previous.location
                                        && indexed_instructions[end..instruction_index].iter().all(
                                            |record| {
                                                record.location == previous.location
                                                    && record.pre_fold_extra_instructions() == 0
                                                    && record.exception_policy()
                                                        == super::ExceptionPolicy::NoneUnchanged
                                            },
                                        )
                                        && (ended_pop_slot
                                            .is_some_and(|(pop, _, width)| pop == end && width == 1)
                                            // The named cleanup's exact end label also records
                                            // this POP when the generic owned-prefix epoch ends
                                            // at POP_EXCEPT. Do not revive that broader epoch.
                                            || (missing == 2
                                                && end - start == 4
                                                && end_removed == 1
                                                && self.exception_owner_provenance
                                                    .copied_handler_cleanups.iter()
                                                    .filter(|origin| {
                                                        origin.missing_null_pops == 2
                                                            && origin.removed_prefix_slots == 1
                                                            && origin.instructions.len() == 4
                                                            && origin.assignment_pop.is_none()
                                                            && origin.continued_call.is_none()
                                                            && origin.continued_attribute_none.is_none()
                                                            && label_positions.get(&origin.start) == Some(&start)
                                                            && label_positions.get(&origin.end) == Some(&end)
                                                            && self.exception_owner_provenance
                                                                .removed_pop_block_boundaries.contains(&origin.end)
                                                            && self.exception_owner_provenance
                                                                .removed_pop_block_widths.get(&origin.end)
                                                                .copied().unwrap_or(1) == 1
                                                    }).count() == 1)
                                            || iterator_handler_truth_prefix)
                                        && self
                                            .exception_owner_provenance
                                            .original_primitive_condition_boundaries
                                            .iter()
                                            .filter(|condition| *condition == previous)
                                            .count()
                                            == 1
                                        && !jump_targets
                                            .iter()
                                            .chain(&self.exception_owner_provenance.source_boundaries)
                                            .chain(&self.exception_owner_provenance.block_boundaries)
                                            .any(|label| {
                                                label_positions.get(label).is_some_and(|position| {
                                                    end < *position && *position <= instruction_index
                                                })
                                            });
                                    none_test || truth_test
                                })
                            })
                            && let Some(removed) = tail.early_removals.checked_add(missing)
                        {
                            let suffix =
                                instruction_index - end + tail.early_removals - end_removed;
                            // The authenticated direct continue originally emitted its NOP,
                            // two handler POPs, the four name-cleanup records, then the
                            // finalizer POP. The exact attribute None prefix appends into
                            // that last NULL cell; it adds no earlier live or hidden record.
                            if continued_attribute_none.is_some()
                                && array == start
                                && end - start == 4
                                && end_removed == 1
                                && missing == 2
                                && tail.early_removals == end_removed + 3
                                && instruction_index - array == 1 + missing + end - start
                            {
                                owners.insert(
                                    instruction_index,
                                    ExceptionOwnerDecision::override_with_region(None),
                                );
                            }
                            // Equality selects the original NULL finally POP immediately after
                            // cleanup, not its preceding POP_EXCEPT. Its one transferred record
                            // must be the only hidden slot on the immediately following load.
                            if removed == suffix
                                && (instruction_index - end == 2
                                    || (missing == 2
                                        && end - start == 4
                                        && instruction_index - end == 3
                                        && continued_call.is_none()
                                        && continued_attribute_none.is_none()
                                        && previous.is_some_and(|condition| {
                                            matches!(
                                                condition.opcode,
                                                crate::target::opcodes::POP_JUMP_IF_FALSE
                                                    | crate::target::opcodes::POP_JUMP_IF_TRUE
                                            )
                                        })))
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
                                && (if instruction_index - end == 2 {
                                    self.exception_owner_provenance
                                        .source_none_conditions
                                        .iter()
                                        .filter(|origin| Some(&origin.instruction) == previous)
                                        .count()
                                        == 1
                                } else {
                                    self.exception_owner_provenance
                                        .original_primitive_condition_boundaries
                                        .iter()
                                        .filter(|condition| Some(*condition) == previous)
                                        .count()
                                        == 1
                                })
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
                                    ExceptionOwnerDecision::override_with_region(None),
                                );
                            }
                            if removed > suffix && removed - suffix <= end - start {
                                let source = end - (removed - suffix);
                                if let Some(region) = self.uniform_exception_owner(
                                    source,
                                    source + 1,
                                    &label_positions,
                                ) && !self.exception_regions[region].is_scope_fallback
                                {
                                    owners.insert(
                                        instruction_index,
                                        ExceptionOwnerDecision::override_with_region(Some(region)),
                                    );
                                }
                            }
                        }
                        // Before the captured authored-return slot, an appended marker can
                        // reuse the uniformly protected prefix. The retained POP remains NULL.
                        if owner_history_exact
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
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
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
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        // A returned async iterator leaves its owned POP_TOP cell in
                        // the old array. The adjacent finalizer POP transfers one hidden
                        // slot to the local load; neither changes the disposal cell's owner.
                        if owner_history_exact
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
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && let Some((array, iterator, slot, region)) =
                                early_return_iterator_slot
                            && original_array_start == array
                            && instruction_index == slot
                            && matches!(indexed_instructions.get(iterator + 1..instruction_index),
                            Some([load, to_bool, condition])
                                if matches!(load.opcode, crate::target::opcodes::LOAD_FAST | crate::target::opcodes::LOAD_FAST_BORROW)
                                    && matches!(load.operand(), Operand::Value(_))
                                    && load.local_policy.hidden_exception_owner_slots() == 1
                                    && to_bool.opcode == crate::target::opcodes::TO_BOOL
                                    && to_bool.operand() == Operand::Value(0)
                                    && to_bool.local_policy.hidden_exception_owner_slots() == 0
                                    && matches!(condition.opcode, crate::target::opcodes::POP_JUMP_IF_FALSE | crate::target::opcodes::POP_JUMP_IF_TRUE)
                                    && matches!(condition.operand(), Operand::Forward(_))
                                    && condition.local_policy.hidden_exception_owner_slots() == 0
                                    && [load, to_bool, condition].iter().all(|record| {
                                        record.pre_fold_extra_instructions() == 0
                                            && record.exception_policy() == super::ExceptionPolicy::NoneUnchanged
                                            && record.location == instruction.location
                                            && record.no_location_policy.borrow_fallthrough() == super::BorrowFallthrough::Reachable
                                            && !record.jump_policy.borrow_empty_block()
                                    }))
                            && self
                                .exception_owner_provenance
                                .removed_pop_block_boundaries
                                .iter()
                                .any(|label| {
                                    label_positions.get(label) == Some(&(iterator + 1))
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
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
                        // An attribute None test follows the same captured iterator disposal.
                        // Its append cell is either that owned POP_TOP or the following NULL POP.
                        if owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && owners
                                .get(instruction_index)
                                .is_none_or(|owner| !owner.is_definitive())
                            && let Some((array, iterator, slot, region)) =
                                early_return_iterator_slot
                            && original_array_start == array
                            && instruction_index.checked_sub(iterator) == Some(4)
                            && (instruction_index == slot
                                || slot.checked_add(1) == Some(instruction_index))
                            && self
                                .exception_owner_provenance
                                .copied_iterator_attribute_none_prefixes
                                .iter()
                                .filter(|[original_pop, original_load, attribute, condition]| {
                                    let mut load = *original_load;
                                    load.local_policy.add_hidden_exception_owner_slots(1);
                                    let actual = indexed_instructions[iterator + 1];
                                    if actual.opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                                        load.opcode = actual.opcode;
                                    }
                                    self.exception_owner_provenance
                                        .early_return_iterator_pops
                                        .iter()
                                        .any(|(_, original_iterator, pop, target)| {
                                            let mut expected_iterator = *original_iterator;
                                            expected_iterator
                                                .local_policy
                                                .add_hidden_exception_owner_slots(1);
                                            pop == original_pop
                                                && expected_iterator
                                                    == *indexed_instructions[iterator]
                                                && self.exception_regions[region].target == *target
                                        })
                                        && *actual == load
                                        && *indexed_instructions[iterator + 2] == *attribute
                                        && *indexed_instructions[iterator + 3] == *condition
                                        && instruction.location == condition.location
                                })
                                .count()
                                == 1
                            && self
                                .exception_owner_provenance
                                .removed_pop_block_boundaries
                                .iter()
                                .any(|label| {
                                    label_positions.get(label) == Some(&(iterator + 1))
                                        && self
                                            .exception_owner_provenance
                                            .removed_pop_block_widths
                                            .get(label)
                                            .copied()
                                            .unwrap_or(1)
                                            == 1
                                })
                            && !jump_targets.iter().any(|label| {
                                label_positions
                                    .get(label)
                                    .is_some_and(|&p| iterator < p && p <= instruction_index)
                            })
                        {
                            let source_region = (instruction_index == slot).then_some(region);
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(source_region),
                            );
                        }
                        // The explicit TO_BOOL places a truthiness marker one cell later
                        // than a None test. Either original POP cell is still NULL; the
                        // preceding authored return NOP and unchanged earlier array cells
                        // retain the ordinary try's owner rather than the scope fallback.
                        if owner_history_exact
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
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && let Some((array, pop, slot)) = early_return_truth_slot
                            && original_array_start == array
                            && instruction_index - pop == 4
                            && (instruction_index <= slot
                                || slot.checked_add(1) == Some(instruction_index)
                                || slot.checked_add(2) == Some(instruction_index))
                            && instruction.location == indexed_instructions[pop + 3].location
                        {
                            let protected = instruction_index <= slot;
                            let source_region = protected
                                .then(|| {
                                    (slot - instruction_index <= pop - array
                                        && indexed_instructions[array..pop].iter().all(|record| {
                                            record.exception_policy()
                                                == super::ExceptionPolicy::NoneUnchanged
                                                && record.opcode != NOP
                                                && record
                                                    .local_policy
                                                    .hidden_exception_owner_slots()
                                                    == 0
                                                && record.pre_fold_extra_instructions() == 0
                                                && record.no_location_policy.borrow_fallthrough()
                                                    != super::BorrowFallthrough::Barrier
                                        }))
                                    .then(|| {
                                        self.uniform_exception_owner(array, pop, &label_positions)
                                    })
                                    .flatten()
                                    .filter(|region| {
                                        !self.exception_regions[*region].is_scope_fallback
                                            && !self.exception_regions[*region].preserve_lasti
                                    })
                                })
                                .flatten();
                            if !protected || source_region.is_some() {
                                owners.insert(
                                    instruction_index,
                                    ExceptionOwnerDecision::override_with_region(source_region),
                                );
                            }
                        }
                        // A captured POP pair remains the same original array when
                        // the actual threading pass retargets its terminal condition.
                        // Only that exact rewritten condition/marker pair can supersede
                        // the generic Strict presence estimate below.
                        let threaded_paired_pop_marker = instruction.exception_policy().owner()
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
                                    .contains(&(*condition, *instruction))
                                    // Another copied condition can leave an identical
                                    // historical origin after later normalization.
                                    // The captured POP prefix must identify the sole
                                    // surviving condition/marker pair.
                                    && indexed_instructions
                                        .windows(2)
                                        .filter(|pair| {
                                            pair[0] == condition && *pair[1] == *instruction
                                        })
                                        .count()
                                        == 1
                            });
                        // One captured finally POP has the same NULL cell for a truth
                        // test. Its explicit TO_BOOL moves the appended marker one cell
                        // past the None-test shape without adding an original POP.
                        let single_pop_truth_marker = instruction.local_policy.hidden_exception_owner_slots() == 0
                            && instruction.pre_fold_extra_instructions() == 0
                            && early_return_nop_slot.is_some_and(|(_, pop, _, _)| {
                                indexed_instructions[pop]
                                    .local_policy
                                    .hidden_exception_owner_slots() == 1
                                    && matches!(
                                        indexed_instructions.get(pop + 1..instruction_index),
                                        Some([load, to_bool, condition])
                                            if matches!(load.opcode,
                                                crate::target::opcodes::LOAD_FAST
                                                    | crate::target::opcodes::LOAD_FAST_BORROW)
                                                && matches!(load.operand(), Operand::Value(_))
                                                && to_bool.opcode == crate::target::opcodes::TO_BOOL
                                                && to_bool.operand() == Operand::Value(0)
                                                && matches!(condition.opcode,
                                                    crate::target::opcodes::POP_JUMP_IF_FALSE | crate::target::opcodes::POP_JUMP_IF_TRUE)
                                                && matches!(condition.operand(), Operand::Forward(_))
                                                && [load, to_bool, condition].iter().all(|record| {
                                                    record.local_policy.hidden_exception_owner_slots() == 0
                                                        && record.pre_fold_extra_instructions() == 0
                                                        && record.exception_policy()
                                                            == super::ExceptionPolicy::NoneUnchanged
                                                        && record.location == instruction.location
                                                        && record.no_location_policy.borrow_fallthrough()
                                                            == super::BorrowFallthrough::Reachable
                                                        && !record.jump_policy.borrow_empty_block()
                                                })
                                    )
                            });
                        // A value-preserving Boolean introduces COPY and TO_BOOL after
                        // the original POP_BLOCK has been compacted. When the unchanged
                        // expansion reaches the adjacent captured POP cell, it inherits
                        // that cell's NULL owner instead of the enclosing try owner.
                        let single_pop_boolean_marker = instruction.exception_policy()
                            == super::ExceptionPolicy::NoneDynamic
                            && original_value_jump_positions
                                .values()
                                .any(|position| *position == instruction_index)
                            && early_return_nop_slot.is_some_and(|(_, pop, slot, _)| {
                                indexed_instructions[pop]
                                    .local_policy
                                    .hidden_exception_owner_slots()
                                    == 1
                                    && instruction_index == slot + 1
                                    && matches!(
                                        indexed_instructions.get(pop + 1..instruction_index),
                                        Some([load, copy, to_bool, condition])
                                            if matches!(load.opcode,
                                                crate::target::opcodes::LOAD_FAST
                                                    | crate::target::opcodes::LOAD_FAST_BORROW)
                                                && matches!(load.operand(), Operand::Value(_))
                                                && copy.opcode == crate::target::opcodes::COPY
                                                && copy.operand() == Operand::Value(1)
                                                && to_bool.opcode == crate::target::opcodes::TO_BOOL
                                                && to_bool.operand() == Operand::Value(0)
                                                && matches!(condition.opcode,
                                                    crate::target::opcodes::POP_JUMP_IF_FALSE | crate::target::opcodes::POP_JUMP_IF_TRUE)
                                                && matches!(condition.operand(), Operand::Forward(_))
                                                && [load, copy, to_bool, condition].iter().all(|record| {
                                                    record.local_policy.hidden_exception_owner_slots() == 0
                                                        && record.pre_fold_extra_instructions() == 0
                                                        && record.exception_policy()
                                                            == super::ExceptionPolicy::NoneUnchanged
                                                        && record.no_location_policy.borrow_fallthrough()
                                                            == super::BorrowFallthrough::Reachable
                                                        && !record.jump_policy.borrow_empty_block()
                                                })
                                    )
                            });
                        // A global one-argument call likewise compacts its PUSH_NULL,
                        // so the condition's appended marker can reuse the captured POP.
                        // Authenticate the entire unchanged call and condition sequence.
                        let single_pop_call_marker = instruction.exception_policy()
                            == super::ExceptionPolicy::NonePresent
                            && early_return_nop_slot.is_some_and(|(_, pop, slot, _)| {
                                indexed_instructions[pop]
                                    .local_policy
                                    .hidden_exception_owner_slots()
                                    == 1
                                    && instruction_index == slot + 1
                                    && matches!(
                                        indexed_instructions.get(pop + 1..instruction_index),
                                        Some([global, arg, call, to_bool, condition])
                                            if global.opcode == crate::target::opcodes::LOAD_GLOBAL
                                                && matches!(global.operand(), Operand::Value(value) if value & 1 == 1)
                                                && matches!(arg.opcode,
                                                    crate::target::opcodes::LOAD_FAST
                                                        | crate::target::opcodes::LOAD_FAST_BORROW)
                                                && matches!(arg.operand(), Operand::Value(_))
                                                && call.opcode == crate::target::opcodes::CALL
                                                && call.operand() == Operand::Value(1)
                                                && to_bool.opcode == crate::target::opcodes::TO_BOOL
                                                && to_bool.operand() == Operand::Value(0)
                                                && matches!(condition.opcode,
                                                    crate::target::opcodes::POP_JUMP_IF_FALSE | crate::target::opcodes::POP_JUMP_IF_TRUE)
                                                && matches!(condition.operand(), Operand::Forward(_))
                                                && [global, arg, call, to_bool, condition].iter().all(|record| {
                                                    record.local_policy.hidden_exception_owner_slots() == 0
                                                        && record.pre_fold_extra_instructions() == 0
                                                        && record.exception_policy()
                                                            == super::ExceptionPolicy::NoneUnchanged
                                                        && record.no_location_policy.borrow_fallthrough()
                                                            == super::BorrowFallthrough::Reachable
                                                        && !record.jump_policy.borrow_empty_block()
                                                })
                                                && call.location == instruction.location
                                                && to_bool.location == instruction.location
                                                && condition.location == instruction.location
                                    )
                            });
                        // Early compaction leaves the authored exit NOP and its adjacent
                        // POP_BLOCK records in their old array cells. The first retains its
                        // try owner; either captured POP is NULL despite a scope fallback.
                        if owner_history_exact
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && instruction.exception_policy().exclusion()
                                == super::ExceptionExclusion::None
                            && (matches!(
                                instruction.exception_policy().owner(),
                                NormalizedExceptionOwner::Present
                                    | NormalizedExceptionOwner::DynamicCondition
                            ) || threaded_paired_pop_marker
                                || single_pop_boolean_marker
                                || single_pop_call_marker)
                            && owners.get(instruction_index).is_none_or(|decision| {
                                !decision.is_definitive()
                                    || ((threaded_paired_pop_marker
                                        || single_pop_boolean_marker
                                        || single_pop_call_marker)
                                        && decision.has_owner()
                                        && decision.source_region().is_none()
                                        && !decision.overrides_existing())
                            })
                            && let Some((array, pop, slot, region)) = early_return_nop_slot
                            && original_array_start == array
                            && instruction_index.checked_sub(slot).is_some_and(|distance| {
                                distance
                                    <= usize::from(
                                        indexed_instructions[pop]
                                            .local_policy
                                            .hidden_exception_owner_slots(),
                                    )
                            })
                            && ((instruction_index - pop == 3
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
                                && previous.is_some_and(|previous: &Instruction| {
                                    matches!(
                                        previous.opcode,
                                        POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
                                    ) && previous.local_policy.hidden_exception_owner_slots() == 3
                                }))
                                || single_pop_truth_marker
                                || single_pop_boolean_marker
                                || single_pop_call_marker)
                        {
                            let source_region = (instruction_index == slot).then_some(region);
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(source_region),
                            );
                        }
                        // The last SETUP compaction copies live records left and leaves
                        // their old tail cells intact. Revalidate this original entry array
                        // separately: retained SETUP NOPs can carry earlier POP annotations
                        // that intentionally invalidate the generic early-history proofs.
                        if original_array_start == 0
                            && owner_history_exact
                            && tail.late_removals == late_setup_positions.len()
                            && tail.post_compaction_fusions == 0
                            && instruction.exception_policy().exclusion()
                                == super::ExceptionExclusion::None
                            && instruction.exception_policy().owner()
                                == NormalizedExceptionOwner::Present
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
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
                                ExceptionOwnerDecision::override_with_region(None),
                            );
                        }
                        // This original excluded NOP supplies its own exact donor cell;
                        // the capture is limited to the certified copied manager-break array.
                        if instruction.exception_policy() == super::ExceptionPolicy::NonePresent
                            && owner_history_exact
                            && self.copied_break_none_nop_is_null(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(None),
                            );
                        }
                        if instruction.exception_policy()
                            == super::ExceptionPolicy::NoneDynamicCondition
                            && owner_history_exact
                            && tail.early_removals == 1
                            && tail.insertions == 0
                            && tail.late_removals == 0
                            && tail.post_compaction_fusions == 0
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && self.copied_named_break_truth_is_null(
                                instruction_index,
                                original_array_start,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(None),
                            );
                        }
                        if owner_history_exact
                            && instruction.exception_policy()
                                == super::ExceptionPolicy::NoneDynamicCondition
                            && owners
                                .get(instruction_index)
                                .is_none_or(|decision| !decision.is_definitive())
                            && let Some(region) = self.copied_shared_break_truth_owner(
                                instruction_index,
                                original_array_start,
                                tail,
                                &indexed_instructions,
                                &label_positions,
                                &jump_targets,
                            )
                        {
                            owners.insert(
                                instruction_index,
                                ExceptionOwnerDecision::override_with_region(Some(region)),
                            );
                        }
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
                        transferred_setup_null_slot = None;
                        captured_null_slots.clear();
                        captured_handler_owner_slots.clear();
                        captured_method_owner_slots.clear();
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
                        previous = Some(instruction);
                    }
                }
            }
        }
        let certified = self
            .uncompacted_conditional_null_markers::<GATE_DECISIONS>(&jump_targets, &unique_labels);
        #[expect(
            clippy::iter_over_hash_type,
            reason = "Each distinct marker overwrites only its own owner."
        )]
        for marker in certified {
            if marker < indexed_instructions.len() {
                owners.insert(marker, ExceptionOwnerDecision::override_with_region(None));
            }
        }
        owners.extend(self.normal_try_break_pop_owners(&label_positions, &jump_targets));
        // The generic fresh-array rule cannot see this pair of omitted POPs.
        // Replay the exact source certificate after all generic NULL conclusions.
        owners.extend(self.copied_conjunction_break_owners(&label_positions, &jump_targets));
        owners.extend(self.named_handler_truth_owners(&label_positions, &jump_targets));
        owners.extend(certified_method_markers);
        owners
    }

    /// Replays the single late SETUP vacancy in an unchanged matched-handler array.
    fn named_handler_truth_owners(
        &self,
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> FxHashMap<usize, ExceptionOwnerDecision> {
        let provenance = &self.exception_owner_provenance;
        let mut owners = FxHashMap::default();
        if provenance.named_handler_truth_prefixes.is_empty() {
            return owners;
        }
        let unique = UniqueLabelPositions::new(&self.items);
        for origin in &provenance.named_handler_truth_prefixes {
            let Some((item_start, start)) = unique.get(origin.entry) else {
                continue;
            };
            let marker = start + 4;
            let within = |label: &Label| {
                positions
                    .get(label)
                    .is_some_and(|position| (start..=marker).contains(position))
            };
            // A copied or independently entered alias is a different physical array,
            // even when it shares the captured entry's current instruction position.
            if jump_targets.iter().any(within)
                || self
                    .exception_regions
                    .iter()
                    .any(|region| within(&region.target))
                || [
                    &provenance.normal_try_copy_blocks,
                    &provenance.normal_try_copy_jump_nop_successors,
                    &provenance.normal_try_copy_inline_labels,
                    &provenance.retired_normal_try_copy_labels,
                ]
                .iter()
                .any(|labels| labels.iter().any(within))
                || provenance
                    .normal_try_copy_internal_labels
                    .iter()
                    .any(within)
                || provenance
                    .normal_try_copy_steps
                    .iter()
                    .any(|(a, b, _)| within(a) || within(b))
                || provenance
                    .normal_try_copy_pop_ranges
                    .iter()
                    .any(|(a, b)| within(a) || within(b))
                || provenance.omitted_nop_counts.keys().any(within)
                || provenance.omitted_nop_locations.keys().any(within)
                || !provenance
                    .named_handler_cleanup_targets
                    .contains(&origin.target)
                || positions.get(&origin.setup) != Some(&(start + 1))
                || provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| within(label))
                    .copied()
                    .collect::<Vec<_>>()
                    != [(origin.setup, None)]
            {
                continue;
            }
            let captured_labels = origin
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<FxHashSet<_>>();
            let mut cursor = item_start;
            let exact = origin.items.iter().all(|expected| {
                while let Some(Item::Label(label)) = self.items.get(cursor)
                    && !captured_labels.contains(label)
                {
                    // Exception-range aliases add no native array records or edges.
                    if provenance.block_boundaries.contains(label)
                        || provenance.source_boundaries.contains(label)
                        || provenance.removed_pop_block_boundaries.contains(label)
                        || self.preserved_block_boundaries.contains(label)
                        || provenance.setups.iter().any(|(setup, _)| setup == label)
                        || unique.get(*label).is_none()
                    {
                        return false;
                    }
                    cursor += 1;
                }
                let same = match (self.items.get(cursor), expected) {
                    (Some(Item::Label(current)), Item::Label(expected)) => {
                        current == expected
                            && unique.get(*current).is_some()
                            && (*current == origin.entry
                                || *current == origin.setup
                                || (!provenance.block_boundaries.contains(current)
                                    && !provenance.source_boundaries.contains(current)
                                    && !self.preserved_block_boundaries.contains(current)))
                    }
                    (Some(Item::Instruction(current)), Item::Instruction(expected)) => {
                        let mut expected = *expected;
                        if expected.opcode == crate::target::opcodes::LOAD_FAST
                            && matches!(
                                current.opcode,
                                crate::target::opcodes::LOAD_FAST_BORROW
                                    | crate::target::opcodes::LOAD_FAST_CHECK
                            )
                        {
                            expected.opcode = current.opcode;
                        }
                        *current == expected
                    }
                    _ => false,
                };
                cursor += 1;
                same
            });
            if !exact {
                continue;
            }
            let current = &self.items[item_start..cursor];
            let Some(region) = self.uniform_exception_owner(start + 1, marker, positions) else {
                continue;
            };
            let owner = self.exception_regions[region];
            let store = current
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                })
                .unwrap();
            if owner.target != origin.target
                || owner.is_scope_fallback
                || !owner.preserve_lasti
                || store.depth_after() != Some(owner.depth)
            {
                continue;
            }
            // Original cells are STORE, SETUP(NULL), LOAD, TO_BOOL, condition.
            // The SETUP prevents early STORE/LOAD fusion. Its late removal leaves
            // the old condition cell at index four for the appended NOT_TAKEN.
            owners.insert(
                marker,
                ExceptionOwnerDecision::override_with_region(Some(region)),
            );
        }
        owners
    }

    /// Replays the retained break NOP reused after exactly three local-load fusions.
    fn copied_break_none_nop_is_null(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> bool {
        if tail.insertions != 0
            || tail.late_removals != 0
            || tail.post_compaction_fusions != 3
            || tail.early_removals != 7
            || marker < array + 3
        {
            return false;
        }
        let authored = marker - 3;
        let provenance = &self.exception_owner_provenance;
        let mut origins = provenance
            .copied_break_none_nops
            .iter()
            .filter(|(_, records)| {
                let mut expected = *records;
                if instructions[authored + 1].opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                    expected[1].opcode = crate::target::opcodes::LOAD_FAST_BORROW;
                }
                instructions[authored..=marker]
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| **actual == expected)
            });
        let Some((aliases, _)) = origins.next() else {
            return false;
        };
        if origins.next().is_some()
            || aliases.iter().any(|label| {
                positions.get(label) != Some(&(authored + 1))
                    || self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(*label))
                        .count()
                        != 1
            })
            || jump_targets
                .iter()
                .chain(&provenance.source_boundaries)
                .chain(&provenance.block_boundaries)
                .chain(&provenance.fusion_boundaries)
                .chain(&self.preserved_block_boundaries)
                .chain(provenance.omitted_nop_counts.keys())
                .chain(provenance.omitted_nop_locations.keys())
                .chain(provenance.setups.iter().map(|(label, _)| label))
                .chain(self.exception_regions.iter().map(|region| &region.target))
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| array < *position && *position <= marker)
                })
        {
            return false;
        }
        let prefix = &instructions[array..authored];
        if prefix.iter().any(|record| {
            !matches!(record.operand(), Operand::Value(_))
                || is_scope_exit(record.opcode)
                || matches!(
                    record.opcode,
                    NOP | NOT_TAKEN | STORE_FAST_LOAD_FAST | STORE_FAST_STORE_FAST
                )
                || record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                || record.local_policy.hidden_exception_owner_slots() != 0
                || record.pre_fold_extra_instructions() != 0
                || record.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
        }) {
            return false;
        }
        // The one early global NULL fold and the None comparison precede the later
        // local pass. Its three fusions all occur before this intact four-record
        // suffix. The appended marker therefore reuses the authored NOP's old cell,
        // whose unconditional exclusion already proves a NULL owner.
        prefix
            .iter()
            .filter(|record| {
                matches!(
                    record.opcode,
                    LOAD_FAST_LOAD_FAST | LOAD_FAST_BORROW_LOAD_FAST_BORROW
                )
            })
            .count()
            == 3
            && prefix
                .iter()
                .filter(|record| {
                    matches!((record.opcode, record.operand()),
                (LOAD_GLOBAL, Operand::Value(argument)) if argument & 1 != 0)
                })
                .count()
                == 1
    }

    /// Resolves only the captured named break's original NULL finalizer POP cell.
    fn copied_named_break_truth_is_null(
        &self,
        marker: usize,
        array: usize,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        targets: &FxHashSet<Label>,
    ) -> bool {
        if marker.checked_sub(array) != Some(7) {
            return false;
        }
        let provenance = &self.exception_owner_provenance;
        let mut origins = provenance
            .copied_named_break_truth_prefixes
            .iter()
            .filter(|origin| {
                positions.get(&origin.entry) == Some(&array)
                    && provenance.source_boundaries.contains(&origin.entry)
                    && provenance.block_boundaries.contains(&origin.entry)
            });
        let Some(origin) = origins.next() else {
            return false;
        };
        if origins.next().is_some() {
            return false;
        }
        let records = origin
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(record) => Some(*record),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let [
            authored,
            pop,
            none,
            store,
            delete,
            load,
            truth,
            condition,
            expected_marker,
        ] = records.as_slice()
        else {
            return false;
        };
        let Some(actual) = instructions.get(array..=marker) else {
            return false;
        };
        let mut expected_pop = *pop;
        expected_pop
            .local_policy
            .add_hidden_exception_owner_slots(1);
        if actual
            .iter()
            .copied()
            .zip([
                expected_pop,
                *none,
                *store,
                *delete,
                *load,
                *truth,
                *condition,
                *expected_marker,
            ])
            .any(|(actual, expected)| *actual != expected)
            || provenance
                .original_primitive_condition_boundaries
                .iter()
                .filter(|record| **record == origin.original_condition)
                .count()
                != 1
        {
            return false;
        }
        let mut ordinal = 0usize;
        let mut handler_ends = Vec::new();
        let mut finally_ends = Vec::new();
        for item in &origin.items {
            match item {
                Item::Instruction(_) => ordinal += 1,
                Item::Label(label) => {
                    let expected = array + ordinal.saturating_sub(1);
                    if positions.get(label) != Some(&expected)
                        || provenance.omitted_nop_counts.contains_key(label)
                        || provenance.omitted_nop_locations.contains_key(label)
                        || provenance.fusion_boundaries.contains(label)
                        || provenance.continuation_boundaries.contains(label)
                        || provenance.setups.iter().any(|(setup, _)| setup == label)
                        // A captured leading short-circuit alias can preserve this
                        // same array entry. An interior preserved boundary still rejects.
                        || (ordinal > 0 && self.preserved_block_boundaries.contains(label))
                    {
                        return false;
                    }
                    if ordinal > 0
                        && (targets.contains(label)
                            || provenance.source_boundaries.contains(label)
                            || provenance.block_boundaries.contains(label)
                            || self
                                .exception_regions
                                .iter()
                                .any(|region| region.start == *label || region.target == *label))
                    {
                        return false;
                    }
                    if ordinal == 1 {
                        handler_ends.push(*label);
                    }
                    if ordinal == 5 {
                        finally_ends.push(*label);
                    }
                }
            }
        }
        if targets
            .iter()
            .chain(&provenance.source_boundaries)
            .chain(&provenance.block_boundaries)
            .any(|label| {
                positions
                    .get(label)
                    .is_some_and(|position| array < *position && *position <= marker)
            })
        {
            return false;
        }
        let handlers = self
            .exception_regions
            .iter()
            .filter(|region| !region.is_scope_fallback && handler_ends.contains(&region.end))
            .collect::<Vec<_>>();
        let finalizers = self
            .exception_regions
            .iter()
            .enumerate()
            .filter(|(_, region)| !region.is_scope_fallback && finally_ends.contains(&region.end))
            .collect::<Vec<_>>();
        if handlers.len() != 2
            || handlers[0].target == handlers[1].target
            || handlers.iter().any(|region| {
                !region.preserve_lasti
                    || Some(region.depth) != authored.depth_after()
                    || positions
                        .get(&region.start)
                        .is_none_or(|position| *position >= array)
            })
            || finalizers.len() != 1
        {
            return false;
        }
        let (finalizer_index, finalizer) = finalizers[0];
        if finalizer.preserve_lasti
            || Some(finalizer.depth) != pop.depth_after()
            || positions
                .get(&finalizer.start)
                .is_none_or(|position| *position >= array)
            || self.uniform_exception_owner(array, array + 4, positions) != Some(finalizer_index)
        {
            return false;
        }
        // Original cells: NOP, POP, POP, four name-clear records, finally POP,
        // LOAD, TO_BOOL, branch. The compacted seven-record prefix appends its
        // marker into original cell seven, the separately authenticated NULL POP.
        true
    }

    /// Resolves a copied break marker whose old cell precedes its whole cleanup.
    fn copied_shared_break_truth_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        targets: &FxHashSet<Label>,
    ) -> Option<usize> {
        let nop = marker.checked_sub(4)?;
        if array >= nop
            || tail.insertions != 0
            || tail.late_removals != 0
            || tail.post_compaction_fusions != 0
            || tail.early_removals <= 4
            || tail.early_removals > marker - array
        {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut matching = provenance
            .copied_shared_break_truth_prefixes
            .iter()
            .filter(|origin| {
                let Some(
                    [
                        Item::Instruction(original_nop),
                        Item::Label(first),
                        Item::Label(second),
                        Item::Instruction(load),
                        Item::Instruction(truth),
                        Item::Instruction(condition),
                        marker_tail @ ..,
                    ],
                ) = origin.get(..)
                else {
                    return false;
                };
                let ([Item::Instruction(original_marker)]
                | [Item::Label(_), Item::Instruction(original_marker)]) = marker_tail
                else {
                    return false;
                };
                let ranges = [*first, *second];
                *instructions[nop] == *original_nop
                    && *instructions[nop + 1] == *load
                    && *instructions[nop + 2] == *truth
                    && *instructions[nop + 3] == *condition
                    && *instructions[marker] == *original_marker
                    && self
                        .items
                        .windows(origin.len())
                        .filter(|window| *window == origin.as_slice())
                        .count()
                        == 1
                    && ranges.iter().all(|label| {
                        positions.get(label) == Some(&(nop + 1))
                            && !targets.contains(label)
                            && !provenance.source_boundaries.contains(label)
                            && !provenance.block_boundaries.contains(label)
                            && !provenance.fusion_boundaries.contains(label)
                            && !provenance.continuation_boundaries.contains(label)
                            && !provenance.omitted_nop_counts.contains_key(label)
                            && !provenance.omitted_nop_locations.contains_key(label)
                            && !self.preserved_block_boundaries.contains(label)
                    })
                    && !self.exception_regions.iter().any(|region| {
                        ranges.contains(&region.start) || ranges.contains(&region.target)
                    })
            });
        let origin = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        // Every early erased cell in this uninterrupted prefix is the NULL paired
        // with an odd LOAD_GLOBAL. The omitted POPs add two cells to both the old
        // cleanup width and its removal count, so they cancel: four live records
        // (authored NOP, load, truth, condition) separate the donor from this marker.
        let prefix = &instructions[array..nop];
        if prefix.iter().any(|record| {
            record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                || record.opcode == NOP
                || is_conditional_jump(record.opcode)
                || is_scope_exit(record.opcode)
                || matches!(record.operand(), Operand::Forward(_) | Operand::Backward(_))
                || record.local_policy.hidden_exception_owner_slots() != 0
                || record.pre_fold_extra_instructions() != 0
                || record.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
        }) || prefix
            .iter()
            .filter(|record| {
                record.opcode == LOAD_GLOBAL
                    && matches!(record.operand(), Operand::Value(argument) if argument & 1 != 0)
            })
            .count()
            != tail.early_removals
            || targets
                .iter()
                .chain(&provenance.block_boundaries)
                .chain(&provenance.source_boundaries)
                .chain(&provenance.fusion_boundaries)
                .chain(&provenance.continuation_boundaries)
                .chain(&self.preserved_block_boundaries)
                .chain(provenance.setups.iter().map(|(label, _)| label))
                .any(|label| {
                    positions
                        .get(label)
                        .is_some_and(|position| array < *position && *position <= marker)
                })
        {
            return None;
        }
        let region = self.uniform_exception_owner(array, nop, positions)?;
        let owner = &self.exception_regions[region];
        let ranges = origin[1..3]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let ended = self
            .exception_regions
            .iter()
            .filter(|region| !region.is_scope_fallback && ranges.contains(&region.end))
            .collect::<Vec<_>>();
        (!owner.is_scope_fallback
            && !owner.preserve_lasti
            && Some(owner.depth) == instructions[nop].depth_after()
            && ranges.contains(&owner.end)
            && ended.len() == 2
            && ended[0].target != ended[1].target
            && ended
                .iter()
                .all(|region| !region.preserve_lasti && region.depth == owner.depth))
        .then_some(region)
    }

    /// Replays the stale truth slot left by a break's two omitted `POP_BLOCK`s.
    fn copied_conjunction_break_owners(
        &self,
        positions: &OwnerLabelPositions,
        targets: &FxHashSet<Label>,
    ) -> FxHashMap<usize, ExceptionOwnerDecision> {
        let mut result = FxHashMap::default();
        let provenance = &self.exception_owner_provenance;
        for origin in &provenance.copied_conjunction_break_prefixes {
            let Some(nop_offset) = origin
                .iter()
                .position(|item| matches!(item, Item::Instruction(_)))
            else {
                continue;
            };
            let Some(Item::Instruction(nop)) = origin.get(nop_offset) else {
                continue;
            };
            if origin.len() < nop_offset + 5 {
                continue;
            }
            let Some(load_offset) = origin
                .iter()
                .enumerate()
                .skip(nop_offset + 1)
                .find_map(|(index, item)| matches!(item, Item::Instruction(_)).then_some(index))
            else {
                continue;
            };
            let mut matches = self
                .items
                .windows(origin.len())
                .enumerate()
                .filter(|(_, items)| *items == origin.as_slice());
            let Some((item_start, _)) = matches.next() else {
                continue;
            };
            if matches.next().is_some()
                || provenance
                    .copied_conjunction_break_prefixes
                    .iter()
                    .filter(|prefix| *prefix == origin)
                    .count()
                    != 1
            {
                continue;
            }
            let start = self.items[..item_start + nop_offset]
                .iter()
                .filter(|item| matches!(item, Item::Instruction(_)))
                .count();
            let marker = start + 4;
            let head_labels = origin[..nop_offset]
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            let range_labels = origin[nop_offset + 1..load_offset]
                .iter()
                .filter_map(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            if !head_labels.iter().any(|label| {
                provenance.source_boundaries.contains(label)
                    && provenance.block_boundaries.contains(label)
                    && positions.get(label) == Some(&start)
            }) || range_labels.iter().any(|label| {
                positions.get(label) != Some(&(start + 1))
                    || targets.contains(label)
                    || provenance.source_boundaries.contains(label)
                    || provenance.block_boundaries.contains(label)
                    || self.preserved_block_boundaries.contains(label)
                    || provenance.setups.iter().any(|(entry, _)| entry == label)
                    || provenance.fusion_boundaries.contains(label)
                    || provenance.continuation_boundaries.contains(label)
                    || provenance.omitted_nop_counts.contains_key(label)
                    || provenance.omitted_nop_locations.contains_key(label)
            }) || self.exception_regions.iter().any(|region| {
                range_labels.contains(&region.start) || range_labels.contains(&region.target)
            }) {
                continue;
            }
            let ended = self
                .exception_regions
                .iter()
                .filter(|region| !region.is_scope_fallback && range_labels.contains(&region.end))
                .collect::<Vec<_>>();
            if ended.len() != 2
                || ended[0].target == ended[1].target
                || ended.iter().any(|region| {
                    region.preserve_lasti
                        || Some(region.depth) != nop.depth_after()
                        || positions
                            .get(&region.start)
                            .is_none_or(|position| *position > start)
                })
            {
                continue;
            }
            let Some(region) = self.uniform_exception_owner(start + 1, marker, positions) else {
                continue;
            };
            let owner = &self.exception_regions[region];
            if !owner.is_scope_fallback || owner.depth != 0 || !owner.preserve_lasti {
                continue;
            }
            // Original cells are NOP, POP, POP, LOAD_FAST, TO_BOOL, condition.
            // Removing the first two NOPs leaves four live records; appending at
            // cell four reuses the old TO_BOOL owner. The later truth array has
            // no captured prefix and still appends into fresh NULL storage.
            result.insert(
                marker,
                ExceptionOwnerDecision::override_with_region(Some(region)),
            );
        }
        result
    }

    /// Replays the complete original bare-return array through two unnamed handlers.
    fn copied_bare_handler_none_owner(
        &self,
        marker: usize,
        array: usize,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        targets: &FxHashSet<Label>,
    ) -> Option<usize> {
        if marker.checked_sub(array) != Some(5)
            || targets.iter().any(|label| {
                positions
                    .get(label)
                    .is_some_and(|&p| array < p && p <= marker)
            })
        {
            return None;
        }
        let mut matching = self
            .exception_owner_provenance
            .copied_bare_handler_none_prefixes
            .iter()
            .filter(|(entry, _, records)| {
                if positions.get(entry) != Some(&array) || *instructions[marker] != records[9] {
                    return false;
                }
                let mut inner = records[3];
                inner.local_policy.add_hidden_exception_owner_slots(2);
                let mut outer = records[5];
                outer.local_policy.add_hidden_exception_owner_slots(1);
                let mut load = records[7];
                let actual_load = instructions[array + 3];
                if !matches!(
                    actual_load.opcode,
                    crate::target::opcodes::LOAD_FAST | crate::target::opcodes::LOAD_FAST_BORROW
                ) {
                    return false;
                }
                // Borrow analysis can require an owned reference without changing this cell.
                load.opcode = actual_load.opcode;
                load.local_policy = super::LocalPolicy::for_load(
                    actual_load.local_policy.force_owned_load(),
                    false,
                );
                load.local_policy.add_hidden_exception_owner_slots(1);
                *instructions[array] == records[0]
                    && *instructions[array + 1] == inner
                    && *instructions[array + 2] == outer
                    && *actual_load == load
                    && *instructions[array + 4] == records[8]
            });
        let (_, finally_handler, _) = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        let region = self.uniform_exception_owner(array + 2, array + 3, positions)?;
        let owner = &self.exception_regions[region];
        (!owner.is_scope_fallback && !owner.preserve_lasti && owner.target == *finally_handler)
            .then_some(region)
    }

    /// Recognizes the unchanged result discard at an authored ordinary-await entry.
    fn unchanged_discarded_await_end(
        &self,
        start: usize,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
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

    /// Recovers the owned record immediately before an authenticated POP/SETUP pair.
    fn retained_setup_predecessor_owner(
        &self,
        marker: usize,
        array_start: usize,
        setup_start: usize,
        setup: Label,
        setup_removed: usize,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
        owners: &DynamicExceptionOwners,
    ) -> Option<usize> {
        use crate::target::opcodes::{LOAD_DEREF, LOAD_FAST, LOAD_FAST_BORROW};

        let nop_index = setup_start.checked_sub(1)?;
        let donor_index = nop_index.checked_sub(1)?;
        // The sole transferred POP precedes the retained SETUP. Neither it nor
        // the three folded None-comparison records can donate this old slot.
        if array_start > donor_index
            || setup_start.checked_add(2) != Some(marker)
            || donor_index.checked_add(setup_removed.checked_sub(1)?) != Some(marker)
        {
            return None;
        }
        let donor = *instructions.get(donor_index)?;
        let nop = *instructions.get(nop_index)?;
        let load = *instructions.get(setup_start)?;
        let condition = *instructions.get(marker.checked_sub(1)?)?;
        if !matches!(donor.opcode, LOAD_FAST | LOAD_FAST_BORROW | POP_TOP)
            || donor.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            || owners.contains_key(donor_index)
            || donor.local_policy.hidden_exception_owner_slots() != 0
            || nop.opcode != NOP
            || nop.operand() != Operand::Value(0)
            || nop.fusion_policy.is_converted_pop_block()
            || nop.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
            || nop.local_policy.hidden_exception_owner_slots() != 1
            || !matches!(load.opcode, LOAD_FAST | LOAD_FAST_BORROW | LOAD_DEREF)
            || load.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            || load.local_policy.hidden_exception_owner_slots() != 0
            || !matches!(condition.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE)
            || !matches!(condition.operand(), Operand::Forward(_))
            || condition.exception_policy() != super::ExceptionPolicy::NoneUnchanged
            || condition.local_policy.hidden_exception_owner_slots() != 3
            || [donor, nop, load, condition].iter().any(|instruction| {
                instruction.pre_fold_extra_instructions() != 0
                    || instruction.only_allows_unconditional_inlining()
                    || instruction.no_location_policy.borrow_fallthrough()
                        == super::BorrowFallthrough::Barrier
                    || instruction.location.line < 0
            })
        {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let in_head = |label: &Label| {
            positions
                .get(label)
                .is_some_and(|position| donor_index < *position && *position <= marker)
        };
        if provenance
            .source_boundaries
            .iter()
            .chain(&provenance.block_boundaries)
            .chain(&provenance.fusion_boundaries)
            .chain(&self.preserved_block_boundaries)
            .chain(jump_targets)
            .chain(&provenance.normal_try_copy_blocks)
            .chain(&provenance.retired_normal_try_copy_labels)
            .chain(&provenance.normal_try_copy_inline_labels)
            .chain(&provenance.normal_try_copy_jump_nop_successors)
            .chain(&provenance.normal_try_copy_internal_labels)
            .chain(provenance.omitted_nop_counts.keys())
            .chain(provenance.omitted_nop_locations.keys())
            .any(in_head)
            || provenance
                .normal_try_copy_steps
                .iter()
                .any(|(start, end, _)| in_head(start) || in_head(end))
            || provenance
                .normal_try_copy_pop_ranges
                .iter()
                .any(|(start, end)| in_head(start) || in_head(end))
            || self
                .exception_regions
                .iter()
                .any(|region| in_head(&region.target))
        {
            return None;
        }
        let mut ranges = provenance.setup_nop_ranges.iter().filter(|(start, end)| {
            positions.get(start) == Some(&nop_index) && positions.get(end) == Some(&setup_start)
        });
        let &(range_start, range_end) = ranges.next()?;
        if ranges.next().is_some()
            || provenance
                .setup_nop_boundaries
                .iter()
                .any(|label| in_head(label) && *label != range_start && *label != range_end)
        {
            return None;
        }
        let mut pops = provenance
            .removed_pop_block_boundaries
            .iter()
            .filter(|label| in_head(label));
        let pop = pops.next()?;
        if pops.next().is_some()
            || positions.get(pop) != Some(&nop_index)
            || provenance
                .removed_pop_block_widths
                .get(pop)
                .copied()
                .unwrap_or(1)
                != 1
        {
            return None;
        }
        let mut setups = provenance.setups.iter().filter(|(label, _)| in_head(label));
        let &(label, location) = setups.next()?;
        if setups.next().is_some()
            || label != setup
            || positions.get(&label) != Some(&setup_start)
            || location != Some(nop.location)
        {
            return None;
        }
        if [range_start, range_end, *pop, setup].iter().any(|label| {
            self.items
                .iter()
                .filter(|item| **item == Item::Label(*label))
                .count()
                != 1
        }) {
            return None;
        }
        let handler = provenance.ordinary_try_setups.get(&setup)?;
        let inner = self.uniform_exception_owner(setup_start, marker + 1, positions)?;
        if self.exception_regions[inner].is_scope_fallback
            || self.exception_regions[inner].target != *handler
            || self.exception_regions[inner].preserve_lasti
        {
            return None;
        }
        let outer = self.uniform_exception_owner(donor_index, donor_index + 1, positions)?;
        let owner = self.exception_regions[outer];
        (!owner.is_scope_fallback && !owner.preserve_lasti && owner.target != *handler)
            .then_some(outer)
    }

    /// Replays only the original NULL POPs of an intact retained-manager return array.
    fn retained_manager_return_none_owner(
        &self,
        marker: usize,
        array_start: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> bool {
        use crate::target::opcodes::{LOAD_FAST, LOAD_FAST_BORROW};

        let Some(load_index) = marker.checked_sub(2) else {
            return false;
        };
        let Some(entry_index) = array_start.checked_sub(1) else {
            return false;
        };
        if array_start > load_index {
            return false;
        }
        let mut matches = self
            .exception_owner_provenance
            .retained_manager_return_none_prefixes
            .iter()
            .filter(|origin| {
                if *instructions[entry_index] != origin.entry
                    || array_start.checked_add(origin.prefix.len()) != Some(load_index)
                    || *instructions[marker - 1] != origin.condition
                    || *instructions[marker] != origin.marker
                    || origin.return_offset.checked_add(6) != Some(origin.prefix.len())
                {
                    return false;
                }
                if jump_targets
                    .iter()
                    .chain(&self.exception_owner_provenance.source_boundaries)
                    .chain(&self.exception_owner_provenance.block_boundaries)
                    .chain(&self.preserved_block_boundaries)
                    .chain(self.exception_regions.iter().map(|region| &region.target))
                    .any(|label| {
                        positions
                            .get(label)
                            .is_some_and(|position| array_start < *position && *position <= marker)
                    })
                {
                    return false;
                }
                let mut expected_load = origin.load;
                expected_load
                    .local_policy
                    .add_hidden_exception_owner_slots(1);
                if instructions[load_index].opcode == LOAD_FAST_BORROW {
                    expected_load.opcode = LOAD_FAST_BORROW;
                }
                if *instructions[load_index] != expected_load {
                    return false;
                }
                let mut prefix_tail = ExceptionOwnerTail::default();
                for (offset, (actual, expected)) in instructions[array_start..load_index]
                    .iter()
                    .zip(&origin.prefix)
                    .enumerate()
                {
                    let mut expected = *expected;
                    if expected.opcode == LOAD_FAST && actual.opcode == LOAD_FAST_BORROW {
                        expected.opcode = LOAD_FAST_BORROW;
                    }
                    if **actual != expected
                        || actual.local_policy.hidden_exception_owner_slots() != 0
                        || actual.pre_fold_extra_instructions() != 0
                        || actual.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                        || (actual.opcode == NOP && offset != origin.return_offset)
                        || actual.no_location_policy.borrow_fallthrough()
                            == super::BorrowFallthrough::Barrier
                        || actual.only_allows_unconditional_inlining()
                        || !matches!(actual.operand(), Operand::Value(_))
                        || is_scope_exit(actual.opcode)
                        || matches!((actual.opcode, actual.operand()),
                            (COMPARE_OP, Operand::Value(argument)) if argument & 16 != 0)
                    {
                        return false;
                    }
                    prefix_tail.instruction(actual, None);
                }
                if prefix_tail.insertions != 0
                    || prefix_tail.post_compaction_fusions != 0
                    || prefix_tail.early_removals.checked_add(4) != Some(tail.early_removals)
                {
                    return false;
                }
                // The omitted inner-WITH POP precedes the five cleanup records.
                // Its surrounding ordinary-try POP follows those records, immediately
                // before the captured final POP. Both original cells are NULL.
                let Some(ordinary_pop) = load_index
                    .checked_add(prefix_tail.early_removals)
                    .and_then(|index| index.checked_add(1))
                else {
                    return false;
                };
                if marker != ordinary_pop && ordinary_pop.checked_add(1) != Some(marker) {
                    return false;
                }
                // Bind the existing real owner to the exact manager retained by the
                // frontend; never clear an arbitrary protected or fallback region.
                let Some(region) = self.uniform_exception_owner(load_index, marker + 1, positions)
                else {
                    return false;
                };
                let owner = self.exception_regions[region];
                !owner.is_scope_fallback
                    && owner.preserve_lasti
                    && owner.target == origin.outer_handler
                    && origin.pop.depth_after() == Some(owner.depth)
            });
        matches.next().is_some() && matches.next().is_none()
    }

    /// Projects the appended marker through the last compaction of a captured manager return.
    fn copied_manager_return_compaction_owner(
        &self,
        marker: usize,
        array_start: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
        retained_inner_try_only: bool,
    ) -> Option<ExceptionOwnerDecision> {
        use crate::target::opcodes::LOAD_FAST_BORROW;

        let condition_index = marker.checked_sub(1)?;
        let condition = *instructions.get(condition_index)?;
        let mut origins = self
            .exception_owner_provenance
            .copied_manager_return_compactions
            .iter()
            .filter(|origin| {
                !retained_inner_try_only
                    || origin
                        .constant_return_prefix
                        .as_ref()
                        .is_some_and(|prefix| prefix.retained_inner_try_pop.is_some())
            })
            .filter_map(|origin| {
                let load_index = marker.checked_sub(2 + usize::from(origin.to_bool.is_some()))?;
                let load = *instructions.get(load_index)?;
                if origin
                    .to_bool
                    .is_some_and(|to_bool| *instructions[load_index + 1] != to_bool)
                    || origin.to_bool.is_some()
                        != matches!(
                            condition.opcode,
                            crate::target::opcodes::POP_JUMP_IF_FALSE
                                | crate::target::opcodes::POP_JUMP_IF_TRUE
                        )
                {
                    return None;
                }
                let start = load_index.checked_sub(origin.cleanup.len())?;
                let prefix_start = start.checked_sub(
                    origin
                        .constant_return_prefix
                        .as_ref()
                        .map_or(0, |prefix| prefix.instructions.len())
                        + origin
                            .named_handler_cleanup
                            .map_or(0, |prefix| prefix.len()),
                )?;
                if array_start > prefix_start
                    || jump_targets
                        .iter()
                        .chain(&self.exception_owner_provenance.source_boundaries)
                        .chain(&self.exception_owner_provenance.block_boundaries)
                        .any(|label| {
                            positions.get(label).is_some_and(|position| {
                                prefix_start < *position && *position <= marker
                            })
                        })
                {
                    return None;
                }
                let mut expected_load = origin.load;
                expected_load
                    .local_policy
                    .add_hidden_exception_owner_slots(1);
                if load.opcode == LOAD_FAST_BORROW {
                    expected_load.opcode = LOAD_FAST_BORROW;
                }
                if *condition != origin.condition
                    || *load != expected_load
                    || origin.pop.opcode != NOP
                    || !origin.pop.fusion_policy.is_converted_pop_block()
                    || origin.pop.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
                    || !instructions[start..load_index]
                        .iter()
                        .zip(&origin.cleanup)
                        .all(|(actual, expected)| **actual == *expected)
                    || !origin.earlier_cleanup.is_none_or(|cleanup| {
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
                {
                    return None;
                }
                if let Some(prefix) = origin.named_handler_cleanup {
                    if tail.post_compaction_fusions != 0
                        || origin.constant_return_prefix.is_some()
                        || origin.earlier_cleanup.is_some()
                        || !instructions[prefix_start..start]
                            .iter()
                            .zip(prefix)
                            .all(|(actual, expected)| **actual == expected)
                    {
                        return None;
                    }
                }
                if let Some(prefix) = &origin.constant_return_prefix {
                    if prefix.retained_inner_try_pop.is_some_and(|pop| {
                        !prefix.retained_authored
                            || prefix.handler_pop.is_some()
                            || prefix.instructions.as_slice() != [pop]
                            || pop.opcode != NOP
                            || !pop.fusion_policy.is_converted_pop_block()
                            || pop.exception_policy() != super::ExceptionPolicy::EXCLUDE_ALWAYS
                            || pop.location != prefix.authored_nop.location
                            || pop.depth_after() != prefix.authored_nop.depth_after()
                            || origin.ordinary_try_pops != 0
                            || prefix.missing_null_pops != 0
                    }) {
                        return None;
                    }
                    if tail.post_compaction_fusions != 0
                        || prefix.authored_nop.opcode != NOP
                        || prefix
                            .instructions
                            .first()
                            .is_none_or(|first| first.location != prefix.authored_nop.location)
                        || !instructions[prefix_start..start]
                            .iter()
                            .zip(&prefix.instructions)
                            .enumerate()
                            .all(|(offset, (actual, expected))| {
                                let mut expected = *expected;
                                if offset == 0 && prefix.retained_inner_try_pop.is_some() {
                                    expected.local_policy.add_hidden_exception_owner_slots(1);
                                } else if offset == 0 && !prefix.retained_authored {
                                    expected.local_policy.add_hidden_exception_owner_slots(
                                        1 + u16::from(prefix.handler_pop.is_some()),
                                    );
                                }
                                **actual == expected
                            })
                    {
                        return None;
                    }
                }
                Some((origin, start, prefix_start, load_index))
            });
        let (origin, start, prefix_start, load_index) = origins.next()?;
        if origins.next().is_some() {
            return None;
        }
        // The prefix may contain certified literal folds, LOAD_GLOBAL folds, and
        // local-pair folds. No unrelated erased POP may shift the manager's old cells.
        let mut prefix_tail = ExceptionOwnerTail::default();
        for (offset, instruction) in instructions[array_start..start].iter().enumerate() {
            let authored_return_slots =
                origin.constant_return_prefix.as_ref().map_or(0, |prefix| {
                    if array_start + offset == prefix_start
                        && prefix.retained_inner_try_pop.is_some()
                    {
                        1
                    } else if array_start + offset == prefix_start && !prefix.retained_authored {
                        1 + usize::from(prefix.handler_pop.is_some())
                    } else {
                        0
                    }
                });
            // A small literal tuple removes only records with its survivor's
            // owner. Its exact unsaturated frontend certificate accounts for those
            // cells before the captured manager cleanup, independently of POPs.
            let literal_slots = if instruction.opcode == LOAD_CONST
                && authored_return_slots == 0
                && self
                    .exception_owner_provenance
                    .uniform_fold_owner_slots
                    .iter()
                    .any(|(location, opcode, slots)| {
                        instruction.location == *location
                            && instruction.opcode == *opcode
                            && instruction.local_policy.hidden_exception_owner_slots() == *slots
                            && u16::from(instruction.pre_fold_extra_instructions()) == *slots
                    }) {
                usize::from(instruction.local_policy.hidden_exception_owner_slots())
            } else {
                0
            };
            if usize::from(instruction.local_policy.hidden_exception_owner_slots())
                != authored_return_slots + literal_slots
                || usize::from(instruction.pre_fold_extra_instructions()) != literal_slots
                || (instruction.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                    && !origin
                        .constant_return_prefix
                        .as_ref()
                        .is_some_and(|prefix| {
                            prefix.retained_inner_try_pop.is_some()
                                && array_start + offset == prefix_start
                                && instruction.exception_policy()
                                    == super::ExceptionPolicy::EXCLUDE_ALWAYS
                        }))
                || (instruction.opcode == NOP
                    && !origin
                        .constant_return_prefix
                        .as_ref()
                        .is_some_and(|prefix| {
                            prefix.retained_authored
                                && array_start + offset == prefix_start
                                && if let Some(pop) = prefix.retained_inner_try_pop {
                                    let mut expected = pop;
                                    expected.local_policy.add_hidden_exception_owner_slots(1);
                                    **instruction == expected
                                } else {
                                    **instruction == prefix.authored_nop
                                }
                        }))
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
            prefix_tail.instruction(instruction, None);
        }
        if prefix_tail.insertions != 0
            || prefix_tail
                .early_removals
                .checked_add(if origin.to_bool.is_some() { 1 } else { 4 })
                != Some(tail.early_removals)
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
            // The manager's omitted POP immediately precedes its cleanup records.
            // Constant returns can omit earlier handler/try POPs before the prelude;
            // the optional outer ordinary-try POP and captured finally POP follow it.
            // These original POP records are NULL before compaction removes them.
            let original_start = start
                .checked_add(prefix_tail.early_removals)?
                // A preceding captured manager contributes its own omitted POP.
                .checked_add(usize::from(origin.earlier_cleanup.is_some()))?
                // Named-handler cleanup omits one POP before SWAP and another
                // before POP_EXCEPT. Both precede the captured manager records.
                .checked_add(2 * usize::from(origin.named_handler_cleanup.is_some()))?
                .checked_add(
                    origin
                        .constant_return_prefix
                        .as_ref()
                        .map_or(0, |prefix| prefix.missing_null_pops),
                )?
                .checked_add(1)?;
            let source = marker.checked_sub(original_start)?;
            if source < origin.cleanup.len() {
                true
            } else if source == origin.cleanup.len()
                || (source == origin.cleanup.len() + 1 && origin.ordinary_try_pops == 1)
            {
                false
            } else {
                return None;
            }
        };
        let region = self.uniform_exception_owner(start, load_index, positions)?;
        let owner = self.exception_regions[region];
        let current = self.uniform_exception_owner(marker, marker + 1, positions);
        let current_owner_matches = if let Some(handler) = origin.retained_outer_handler {
            // The frontend retained this exact outer manager after the inner exit.
            // A captured named-handler prefix can leave the append slot inside the
            // manager cleanup's ordinary-try owner, before its following NULL POPs.
            (!owns_cleanup_record || origin.named_handler_cleanup.is_some())
                && current.is_some_and(|region| {
                    let retained = self.exception_regions[region];
                    !retained.is_scope_fallback
                        && retained.preserve_lasti
                        && retained.target == handler
                        && retained.depth == owner.depth
                        && origin.pop.depth_after() == retained.depth.checked_add(1)
                        && self.uniform_exception_owner(load_index, marker + 1, positions)
                            == Some(region)
                })
        } else {
            current.is_none_or(|current| self.exception_regions[current].is_scope_fallback)
        };
        (!owner.is_scope_fallback && !owner.preserve_lasti && current_owner_matches).then_some(
            ExceptionOwnerDecision::override_with_region(owns_cleanup_record.then_some(region)),
        )
    }

    /// Locates a retained NULL SETUP record in an entry array's last compacted tail.
    fn entry_retained_setup_null_marker(
        &self,
        marker: usize,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
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
                    || !matches!(
                        instruction.exception_policy(),
                        super::ExceptionPolicy::NoneUnchanged
                            | super::ExceptionPolicy::AlwaysUnchanged
                    )
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
        entry_insertions: usize,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
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
        let current_owner = self.exception_regions[current];
        let outer_setup_matches = if entry_insertions == 2 {
            current_owner.is_scope_fallback
                && self
                    .exception_regions
                    .iter()
                    .any(|region| region.is_scope_fallback && region.start == *scope)
        } else {
            // An ordinary scope can retain an outer synchronous manager instead.
            // Bind that setup to the live guard owner before restoring the exact
            // ordinary-try POP_TOP donor left by the same three SETUP removals.
            entry_insertions == 0
                && !current_owner.is_scope_fallback
                && current_owner.preserve_lasti
                && current_owner.depth == owner.depth
                && self
                    .exception_owner_provenance
                    .synchronous_with_setups
                    .contains(scope)
                && self.exception_regions.iter().any(|region| {
                    region.start == *scope
                        && !region.is_scope_fallback
                        && region.preserve_lasti
                        && region.target == current_owner.target
                        && region.depth == current_owner.depth
                })
                && self.uniform_exception_owner(marker.checked_sub(2)?, marker + 1, positions)
                    == Some(current)
        };
        (!owner.is_scope_fallback
            && !owner.preserve_lasti
            && owner.target == handler
            && outer_setup_matches)
            .then_some(region)
    }

    /// Resolves only a captured return call whose appended cell precedes its whole cleanup.
    fn copied_return_call_parent_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<usize> {
        let pop = marker.checked_sub(7)?;
        if pop <= array {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut matches = provenance
            .copied_return_call_prefixes
            .iter()
            .filter(|origin| {
                let mut expected_pop = origin.pops[1];
                expected_pop
                    .local_policy
                    .add_hidden_exception_owner_slots(2);
                positions.get(&origin.entry) == Some(&array)
                    && provenance.source_boundaries.contains(&origin.entry)
                    && provenance.block_boundaries.contains(&origin.entry)
                    && !provenance
                        .retired_normal_try_copy_labels
                        .contains(&origin.entry)
                    && !provenance.normal_try_copy_blocks.contains(&origin.entry)
                    && !provenance
                        .normal_try_copy_internal_labels
                        .contains(&origin.entry)
                    && self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(origin.entry))
                        .count()
                        == 1
                    && *instructions[pop] == expected_pop
                    && instructions
                        .iter()
                        .filter(|record| ***record == expected_pop)
                        .count()
                        == 1
                    && origin.return_nop.location == expected_pop.location
                    && origin.return_nop.depth_after() == expected_pop.depth_after()
                    && instructions[pop + 1..marker - 1]
                        .iter()
                        .copied()
                        .copied()
                        .eq(origin.prefix)
                    && *instructions[marker - 1] == origin.condition
                    && matches!(origin.condition.operand(), Operand::Forward(_))
                    && jump_targets.iter().all(|label| {
                        positions
                            .get(label)
                            .is_none_or(|index| *index <= array || *index > marker)
                    })
                    && provenance.block_boundaries.iter().all(|label| {
                        positions
                            .get(label)
                            .is_none_or(|index| *index <= array || *index > marker)
                    })
            });
        matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        // Seven live records from the surviving POP through the condition plus
        // the two captured erased records precede this newly appended marker.
        // A smaller removal count reaches one of those cleanup cells instead.
        let suffix_width = marker - pop + 2;
        let prefix_width = pop - array;
        if tail.early_removals <= suffix_width
            || tail.early_removals > suffix_width + prefix_width
            || instructions[array..pop].iter().any(|record| {
                record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                    || record.opcode == NOP
                    || record.no_location_policy.borrow_fallthrough()
                        == super::BorrowFallthrough::Barrier
            })
        {
            return None;
        }
        let region = self.uniform_exception_owner(array, pop, positions)?;
        (!self.exception_regions[region].is_scope_fallback
            && !self.exception_regions[region].preserve_lasti)
            .then_some(region)
    }

    /// Locates an appended attribute-truth marker in its captured return/POP array.
    fn copied_return_attribute_truth_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<ExceptionOwnerDecision> {
        let pop = marker.checked_sub(5)?;
        if pop <= array {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let mut matches = provenance
            .copied_return_attribute_truth_prefixes
            .iter()
            .filter(|origin| {
                let mut expected_pop = origin.pops[1];
                expected_pop
                    .local_policy
                    .add_hidden_exception_owner_slots(2);
                let mut expected_load = origin.prefix[0];
                if instructions[pop + 1].opcode == crate::target::opcodes::LOAD_FAST_BORROW {
                    expected_load.opcode = crate::target::opcodes::LOAD_FAST_BORROW;
                }
                positions.get(&origin.entry) == Some(&array)
                    && provenance.source_boundaries.contains(&origin.entry)
                    && provenance.block_boundaries.contains(&origin.entry)
                    && !provenance
                        .retired_normal_try_copy_labels
                        .contains(&origin.entry)
                    && !provenance.normal_try_copy_blocks.contains(&origin.entry)
                    && !provenance
                        .normal_try_copy_internal_labels
                        .contains(&origin.entry)
                    && self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(origin.entry))
                        .count()
                        == 1
                    && *instructions[pop] == expected_pop
                    && instructions
                        .iter()
                        .filter(|record| ***record == expected_pop)
                        .count()
                        == 1
                    && origin.return_nop.location == expected_pop.location
                    && origin.return_nop.depth_after() == expected_pop.depth_after()
                    && *instructions[pop + 1] == expected_load
                    && instructions[pop + 2..marker - 1]
                        .iter()
                        .copied()
                        .copied()
                        .eq(origin.prefix[1..].iter().copied())
                    && *instructions[marker - 1] == origin.condition
                    && matches!(origin.condition.operand(), Operand::Forward(_))
                    && jump_targets.iter().all(|label| {
                        positions
                            .get(label)
                            .is_none_or(|index| *index <= array || *index > marker)
                    })
                    && provenance.block_boundaries.iter().all(|label| {
                        positions
                            .get(label)
                            .is_none_or(|index| *index <= array || *index > marker)
                    })
            });
        matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        // The authored return NOP and first POP are the two captured erased
        // records. The surviving second POP carries exactly those two hidden cells.
        let return_slot = pop.checked_add(tail.early_removals.checked_sub(2)?)?;
        if marker > return_slot {
            return (return_slot.checked_add(1) == Some(marker)
                || return_slot.checked_add(2) == Some(marker))
            .then_some(ExceptionOwnerDecision::override_with_region(None));
        }
        if return_slot - marker > pop - array
            || instructions[array..pop].iter().any(|record| {
                record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                    || record.opcode == NOP
                    || usize::from(record.pre_fold_extra_instructions())
                        != usize::from(record.local_policy.hidden_exception_owner_slots())
                    || (record.local_policy.hidden_exception_owner_slots() != 0
                        && record.opcode != crate::target::opcodes::LOAD_CONST)
                    || record.no_location_policy.borrow_fallthrough()
                        == super::BorrowFallthrough::Barrier
            })
        {
            return None;
        }
        let region = self.uniform_exception_owner(array, pop, positions)?;
        (!self.exception_regions[region].is_scope_fallback
            && !self.exception_regions[region].preserve_lasti)
            .then_some(ExceptionOwnerDecision::override_with_region(Some(region)))
    }

    /// Replays a source-certified membership return without assuming generic fold history.
    fn copied_return_membership_owner(
        &self,
        marker: usize,
        array: usize,
        tail: ExceptionOwnerTail,
        instructions: &[&Instruction],
        positions: &OwnerLabelPositions,
        jump_targets: &FxHashSet<Label>,
    ) -> Option<ExceptionOwnerDecision> {
        use crate::target::opcodes::LOAD_FAST_BORROW;

        let load = marker.checked_sub(4)?;
        if load <= array
            || tail.insertions != 0
            || tail.late_removals != 0
            || tail.post_compaction_fusions != 0
        {
            return None;
        }
        let provenance = &self.exception_owner_provenance;
        let copied = |label: &Label| {
            provenance.retired_normal_try_copy_labels.contains(label)
                || provenance.is_normal_try_copy_label(*label)
        };
        let mut matches = provenance
            .copied_return_membership_prefixes
            .iter()
            .filter(|origin| {
                let mut expected_load = origin.load;
                expected_load
                    .local_policy
                    .add_hidden_exception_owner_slots(2);
                if instructions[load].opcode == LOAD_FAST_BORROW {
                    expected_load.opcode = LOAD_FAST_BORROW;
                }
                positions.get(&origin.entry) == Some(&array)
                    && provenance.block_boundaries.contains(&origin.entry)
                    && provenance.source_boundaries.contains(&origin.entry)
                    && !copied(&origin.entry)
                    && self
                        .items
                        .iter()
                        .filter(|item| **item == Item::Label(origin.entry))
                        .count()
                        == 1
                    && origin.cleanup_labels.iter().all(|label| {
                        positions.get(label) == Some(&load)
                            && self
                                .items
                                .iter()
                                .filter(|item| **item == Item::Label(*label))
                                .count()
                                == 1
                            && !copied(label)
                    })
                    && *instructions[load - 1] == origin.return_value
                    && instructions
                        .iter()
                        .filter(|record| ***record == origin.return_value)
                        .count()
                        == 1
                    && origin
                        .pops
                        .iter()
                        .all(|pop| pop.depth_after() == origin.return_value.depth_after())
                    && *instructions[load] == expected_load
                    && *instructions[load + 1] == origin.constant
                    && *instructions[load + 2] == origin.contains
                    && *instructions[load + 3] == origin.condition
                    && *instructions[marker] == origin.marker
                    && provenance
                        .block_boundaries
                        .iter()
                        .chain(&provenance.source_boundaries)
                        .chain(&self.preserved_block_boundaries)
                        .chain(jump_targets)
                        .all(|label| {
                            positions
                                .get(label)
                                .is_none_or(|position| *position <= array || *position > marker)
                        })
            });
        matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        let prefix = &instructions[array..load];
        if prefix.iter().any(|record| {
            record.exception_policy() != super::ExceptionPolicy::NoneUnchanged
                || record.opcode == NOP
                || !matches!(record.operand(), Operand::Value(_))
                || record.local_policy.hidden_exception_owner_slots() != 0
                || record.pre_fold_extra_instructions() != 0
                || record.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
        }) {
            return None;
        }
        // Only the original PUSH_NULL paired with each flagged global disappears
        // before the two captured POPs. The membership suffix removes its three
        // literal loads and one Boolean-conversion cell afterward. Any other
        // compaction, including a local fusion, fails this complete accounting.
        let globals = prefix
            .iter()
            .filter(|record| {
                matches!((record.opcode, record.operand()), (LOAD_GLOBAL, Operand::Value(argument))
                if argument & 1 != 0)
            })
            .count();
        if tail.early_removals != globals.checked_add(6)? {
            return None;
        }
        let first_pop = load.checked_add(globals)?;
        if marker == first_pop || first_pop.checked_add(1) == Some(marker) {
            return Some(ExceptionOwnerDecision::override_with_region(None));
        }
        if marker >= first_pop || first_pop - marker > prefix.len() {
            return None;
        }
        let region = self.uniform_exception_owner(array, load, positions)?;
        (!self.exception_regions[region].is_scope_fallback
            && !self.exception_regions[region].preserve_lasti)
            .then_some(ExceptionOwnerDecision::override_with_region(Some(region)))
    }

    /// Finds unchanged, unambiguous terminal pairs from the actual threading pass.
    #[cfg(test)]
    fn threaded_condition_positions(
        &self,
        instructions: &[&Instruction],
    ) -> FxHashMap<usize, NormalizedExceptionOwner> {
        self.threaded_condition_positions_with_index::<false>(
            instructions,
            &OwnerInstructionPositions::default(),
        )
    }

    fn threaded_condition_positions_with_index<const INDEX_ORIGINS: bool>(
        &self,
        instructions: &[&Instruction],
        instruction_positions: &OwnerInstructionPositions,
    ) -> FxHashMap<usize, NormalizedExceptionOwner> {
        let origins = &self.exception_owner_provenance.threaded_condition_origins;
        if origins.is_empty() {
            return FxHashMap::default();
        }
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
                let position = if INDEX_ORIGINS {
                    instruction_positions.unique_pair(instructions, &origin.0, &origin.1)?
                } else {
                    let mut matches = instructions
                        .windows(2)
                        .enumerate()
                        .filter(|(_, pair)| *pair[0] == origin.0 && *pair[1] == origin.1);
                    let (position, _) = matches.next()?;
                    if matches.next().is_some() {
                        return None;
                    }
                    position
                };
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
        positions: &OwnerLabelPositions,
    ) -> bool {
        self.uniform_exception_owner(start, end, positions)
            .is_some()
    }

    fn uniform_exception_owner(
        &self,
        start: usize,
        end: usize,
        positions: &OwnerLabelPositions,
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
    fn single_dynamic_owner_admission_scan_matches_both_original_gates() {
        use super::{
            ExceptionOwnerProvenance, NamedHandlerTruthPrefix, UncompactedConditionalArray,
        };
        use crate::assembler::{
            ExceptionPolicy, Instruction, Item, Label, LayoutAssembler, NormalizedExceptionOwner,
            Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            COPY_FREE_VARS, LOAD_CONST, MAKE_CELL, NOT_TAKEN, RETURN_GENERATOR,
        };

        fn original(assembler: &LayoutAssembler) -> Option<bool> {
            let provenance = &assembler.exception_owner_provenance;
            let dynamic = |instruction: &Instruction| {
                matches!(
                    instruction.exception_policy().owner(),
                    NormalizedExceptionOwner::Dynamic
                        | NormalizedExceptionOwner::DynamicCondition
                        | NormalizedExceptionOwner::DynamicConditionStrict
                )
            };
            if provenance.normal_try_break_pop_owners.is_empty()
                && provenance.named_handler_truth_prefixes.is_empty()
                && provenance.copied_conjunction_break_prefixes.is_empty()
                && provenance.uncompacted_conditional_arrays.is_empty()
                && !assembler.items.iter().any(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if instruction.opcode == NOT_TAKEN || dynamic(instruction))
                })
            {
                return None;
            }
            let retained = provenance
                .setups
                .iter()
                .any(|(_, location)| location.is_some());
            let prologue = assembler
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction.opcode),
                    Item::Label(_) => None,
                })
                .is_some_and(|opcode| {
                    matches!(opcode, COPY_FREE_VARS | MAKE_CELL | RETURN_GENERATOR)
                });
            if provenance.synchronous_with_setups.is_empty()
                && provenance.source_boundaries.is_empty()
                && provenance.normal_try_break_pop_owners.is_empty()
                && provenance.named_handler_truth_prefixes.is_empty()
                && !assembler.items.iter().any(|item| {
                    let Item::Instruction(instruction) = item else {
                        return false;
                    };
                    dynamic(instruction)
                        || (retained
                            && instruction.opcode == NOT_TAKEN
                            && (matches!(
                                instruction.exception_policy().owner(),
                                NormalizedExceptionOwner::Present
                                    | NormalizedExceptionOwner::Absent
                            ) || (prologue
                                && instruction.exception_policy()
                                    == ExceptionPolicy::NoneUnchanged)))
                })
            {
                return None;
            }
            Some(prologue)
        }

        let label = Label::from_u32(0);
        let location = SourceLocation::new(1, 1, 0, 2);
        for mask in 0..32 {
            for retained in [false, true] {
                for prologue in [LOAD_CONST, COPY_FREE_VARS, MAKE_CELL, RETURN_GENERATOR] {
                    for opcode in [LOAD_CONST, NOT_TAKEN] {
                        for policy in [
                            ExceptionPolicy::NoneUnchanged,
                            ExceptionPolicy::NoneUnchangedBooleanSingleton,
                            ExceptionPolicy::NonePresent,
                            ExceptionPolicy::NoneAbsent,
                            ExceptionPolicy::AlwaysUnchanged,
                        ] {
                            for dynamic in [
                                None,
                                Some(NormalizedExceptionOwner::Dynamic),
                                Some(NormalizedExceptionOwner::DynamicCondition),
                                Some(NormalizedExceptionOwner::DynamicConditionStrict),
                            ] {
                                let mut provenance = ExceptionOwnerProvenance::default();
                                if mask & 1 != 0 {
                                    provenance.source_boundaries.insert(label);
                                }
                                if mask & 2 != 0 {
                                    provenance.synchronous_with_setups.insert(label);
                                }
                                if mask & 4 != 0 {
                                    provenance
                                        .copied_conjunction_break_prefixes
                                        .push(Vec::new());
                                }
                                if mask & 8 != 0 {
                                    provenance.uncompacted_conditional_arrays.push(
                                        UncompactedConditionalArray {
                                            prefix: Vec::new(),
                                            comprehension_entry: None,
                                            source_entry: None,
                                            fallthrough: None,
                                        },
                                    );
                                }
                                if mask & 16 != 0 {
                                    provenance.named_handler_truth_prefixes.push(
                                        NamedHandlerTruthPrefix {
                                            entry: label,
                                            setup: label,
                                            target: label,
                                            items: Vec::new(),
                                        },
                                    );
                                }
                                provenance
                                    .setups
                                    .push((label, retained.then_some(location)));
                                let mut candidate =
                                    Instruction::new(opcode, Operand::Value(0), location, None);
                                candidate.set_exception_policy(policy);
                                if let Some(dynamic) = dynamic {
                                    candidate.set_exception_owner(dynamic);
                                }
                                let assembler = LayoutAssembler {
                                    python_version: crate::PythonVersion::Py314,
                                    legacy_uniform_jump_seed: false,
                                    items: vec![
                                        Item::Label(label),
                                        Item::Instruction(Instruction::new(
                                            prologue,
                                            Operand::Value(0),
                                            location,
                                            None,
                                        )),
                                        Item::Instruction(candidate),
                                    ],
                                    exception_regions: Vec::new(),
                                    exception_owner_provenance: Box::new(provenance),
                                    preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                                    removed_max_depth: None,
                                    next_label: 1,
                                    has_next_location: false,
                                };
                                assert_eq!(
                                    assembler.dynamic_owner_replay_entry(),
                                    original(&assembler),
                                    "mask={mask}, retained={retained}, prologue={prologue:?}, opcode={opcode:?}, policy={policy:?}, dynamic={dynamic:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn with_owner_replay_mode<T>(
        force: bool,
        force_all_decisions: bool,
        verify: bool,
        operation: impl FnOnce() -> T,
    ) -> T {
        struct Reset(bool, bool, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                super::FORCE_COMPLETE_OWNER_REPLAY.set(self.0);
                super::FORCE_ALL_OWNER_DECISIONS.set(self.1);
                super::VERIFY_COMPLETE_OWNER_REPLAY.set(self.2);
            }
        }
        let _reset = Reset(
            super::FORCE_COMPLETE_OWNER_REPLAY.replace(force),
            super::FORCE_ALL_OWNER_DECISIONS.replace(force_all_decisions),
            super::VERIFY_COMPLETE_OWNER_REPLAY.replace(verify),
        );
        operation()
    }

    #[test]
    fn owner_instruction_signature_index_checks_full_identity_and_adjacent_pairs() {
        use super::OwnerInstructionPositions;
        use crate::assembler::{ExceptionPolicy, Instruction, Operand, SourceLocation};
        use crate::target::opcodes::{LOAD_CONST, LOAD_FAST, NOT_TAKEN};

        let instruction = |opcode, argument, line| {
            Instruction::new(
                opcode,
                Operand::Value(argument),
                SourceLocation::new(line, line, 1, 5),
                None,
            )
        };
        let original = instruction(LOAD_FAST, 0, 2);
        let changed_operand = instruction(LOAD_FAST, 1, 2);
        let mut changed_policy = original;
        changed_policy.set_exception_policy(ExceptionPolicy::NonePresent);
        let next = instruction(NOT_TAKEN, 0, 2);
        let missing = instruction(LOAD_FAST, 3, 2);
        let filler = instruction(LOAD_CONST, 9, 10);
        let unregistered = instruction(LOAD_FAST, 4, 12);
        let mut records = vec![filler; 48];
        records[4] = original;
        records[5] = next;
        records[10] = changed_operand;
        records[15] = original;
        records[20] = changed_policy;
        records[21] = next;
        records[28] = unregistered;
        records[29] = next;
        records[47] = original;
        for (case, records) in [
            ("unique pair", records.clone()),
            ("repeated pair", {
                records[16] = next;
                records
            }),
        ] {
            let instructions = records.iter().collect::<Vec<_>>();
            let sources = [
                &original,
                &changed_operand,
                &changed_policy,
                &next,
                &missing,
                &filler,
            ];
            let indexed =
                OwnerInstructionPositions::for_sources(instructions.len(), sources.len(), sources);
            let direct = OwnerInstructionPositions::default();
            assert!(indexed.positions.get().is_none());
            for source in sources {
                assert_eq!(
                    indexed.unique(&instructions, source),
                    direct.unique(&instructions, source),
                    "{case}: {source:?}"
                );
            }
            assert_eq!(indexed.unique(&instructions, &original), None);
            assert_eq!(indexed.unique(&instructions, &changed_operand), Some(10));
            assert_eq!(indexed.unique(&instructions, &changed_policy), Some(20));
            assert_eq!(indexed.unique(&instructions, &missing), None);
            assert_eq!(indexed.unique(&instructions, &unregistered), Some(28));
            for (first, second) in [
                (original, next),
                (changed_policy, next),
                (missing, next),
                (original, missing),
                (unregistered, next),
            ] {
                assert_eq!(
                    indexed.unique_pair(&instructions, &first, &second),
                    direct.unique_pair(&instructions, &first, &second),
                    "{case}: {first:?}, {second:?}"
                );
            }
            assert_eq!(
                indexed.unique_pair(&instructions, &original, &next),
                (case == "unique pair").then_some(4)
            );
            assert_eq!(
                indexed.unique_pair(&instructions, &changed_policy, &next),
                Some(20)
            );
            assert!(indexed.positions.get().is_some());
            assert!(direct.positions.get().is_none());
            let short = OwnerInstructionPositions::for_sources(31, sources.len(), sources);
            assert!(short.candidates.is_empty());
            let scarce = OwnerInstructionPositions::for_sources(48, 3, sources);
            assert!(scarce.candidates.is_empty());
        }
    }

    #[test]
    fn owner_instruction_signature_index_matches_full_replay_with_many_certificates() {
        use std::fmt::Write;

        let mut source = String::from("def run(value, rows):\n    try:\n");
        for index in 0..8 {
            writeln!(
                source,
                "        result{index} = [item{index} for item{index} in rows if item{index} and value]"
            )
            .unwrap();
            writeln!(
                source,
                "        if value in (0, 1, 2, 3, 4, 5, 6, 7, 8, 9) and result{index}:\n            value += len(result{index})"
            )
            .unwrap();
        }
        source.push_str(
            "        return value or rows\n    except ValueError as error:\n        return error if value else rows\n",
        );
        let compile = || {
            crate::compile(&source, "owner_instruction_signature_index.py")
                .expect("owner source-index example compiles")
                .marshal()
        };
        let gated = with_owner_replay_mode(false, false, true, compile);
        let direct = with_owner_replay_mode(false, true, false, compile);
        let complete = with_owner_replay_mode(true, true, false, compile);
        assert_eq!(gated, direct);
        assert_eq!(gated, complete);
    }

    #[test]
    fn owner_replay_bound_preserves_writers_successors_labels_and_unknown_indices() {
        use super::dynamic_owner_replay_item_limit;
        use crate::assembler::{Instruction, Item, Label, NormalizedExceptionOwner, Operand};
        use crate::target::opcodes::{LOAD_CONST, NOT_TAKEN, POP_JUMP_IF_FALSE, RETURN_VALUE};

        let first = Label::from_u32(0);
        let second = Label::from_u32(1);
        let value = |opcode| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(0),
                super::SourceLocation::NONE,
                None,
            ))
        };
        let marker = value(NOT_TAKEN);
        let conditional = Item::Instruction(Instruction::new(
            POP_JUMP_IF_FALSE,
            Operand::Forward(second),
            super::SourceLocation::NONE,
            None,
        ));
        for owner in [
            NormalizedExceptionOwner::Dynamic,
            NormalizedExceptionOwner::DynamicCondition,
            NormalizedExceptionOwner::DynamicConditionStrict,
        ] {
            let Item::Instruction(mut dynamic) = value(LOAD_CONST) else {
                unreachable!();
            };
            dynamic.set_exception_owner(owner);
            for writer in [marker, conditional, Item::Instruction(dynamic)] {
                let items = [
                    Item::Label(first),
                    value(LOAD_CONST),
                    writer,
                    Item::Label(second),
                    value(LOAD_CONST),
                    Item::Label(first),
                    value(RETURN_VALUE),
                    Item::Label(second),
                ];
                assert_eq!(dynamic_owner_replay_item_limit(&items, 4, None), 5);
                assert_eq!(dynamic_owner_replay_item_limit(&items, 4, Some(0)), 5);
                assert_eq!(dynamic_owner_replay_item_limit(&items, 4, Some(2)), 7);
                assert_eq!(dynamic_owner_replay_item_limit(&items, 4, Some(3)), 8);
                assert_eq!(dynamic_owner_replay_item_limit(&items, 4, Some(4)), 8);
                assert_eq!(
                    dynamic_owner_replay_item_limit(&items, 4, Some(usize::MAX)),
                    8
                );
            }
        }
        let items = [
            Item::Label(first),
            value(LOAD_CONST),
            Item::Label(second),
            value(RETURN_VALUE),
            Item::Label(first),
        ];
        assert_eq!(dynamic_owner_replay_item_limit(&items, 2, None), 0);
        assert_eq!(dynamic_owner_replay_item_limit(&items, 2, Some(0)), 4);
        assert_eq!(dynamic_owner_replay_item_limit(&items, 2, Some(1)), 5);
        assert_eq!(dynamic_owner_replay_item_limit(&[], 0, None), 0);
        assert_eq!(dynamic_owner_replay_item_limit(&items[..1], 0, None), 0);
    }

    #[test]
    fn ordinary_decision_gate_keeps_all_indexed_writers_and_pending_appends() {
        use super::OwnerDecisionIndices;
        use crate::assembler::{Instruction, Label, NormalizedExceptionOwner, Operand};
        use crate::target::opcodes::{
            FOR_ITER, LOAD_CONST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE,
            POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, RAISE_VARARGS, RERAISE, RETURN_VALUE, SEND,
        };

        let value =
            |opcode| Instruction::new(opcode, Operand::Value(0), super::SourceLocation::NONE, None);
        let ordinary = value(LOAD_CONST);
        let none = OwnerDecisionIndices::new(12, [usize::MAX, 12].iter());
        assert!(none.positions.is_empty());
        assert!(none.can_finish_ordinary(&ordinary, 0, None));
        assert!(none.can_finish_ordinary(&value(NOP), 5, Some(4)));
        assert!(!none.can_finish_ordinary(&ordinary, 5, Some(5)));

        let normal_pop_terminators = [1];
        let threaded_conditions = [3, 5];
        let normal_finally_markers = [5, 7];
        let retained_setup_markers = [8];
        let retained_value_setup_markers = [10];
        let indexed = OwnerDecisionIndices::new(
            12,
            normal_pop_terminators
                .iter()
                .chain(&threaded_conditions)
                .chain(&normal_finally_markers)
                .chain(&retained_setup_markers)
                .chain(&retained_value_setup_markers),
        );
        for index in 0..12 {
            assert_eq!(
                indexed.can_finish_ordinary(&ordinary, index, None),
                !matches!(index, 1 | 3 | 5 | 7 | 8 | 10),
            );
        }
        for opcode in [
            NOT_TAKEN,
            FOR_ITER,
            SEND,
            POP_JUMP_IF_FALSE,
            POP_JUMP_IF_NONE,
            POP_JUMP_IF_NOT_NONE,
            POP_JUMP_IF_TRUE,
            RETURN_VALUE,
            RAISE_VARARGS,
            RERAISE,
        ] {
            assert!(!none.can_finish_ordinary(&value(opcode), 0, None));
        }
        for owner in [
            NormalizedExceptionOwner::Dynamic,
            NormalizedExceptionOwner::DynamicCondition,
            NormalizedExceptionOwner::DynamicConditionStrict,
        ] {
            let mut dynamic = ordinary;
            dynamic.set_exception_owner(owner);
            assert!(!none.can_finish_ordinary(&dynamic, 0, None));
        }
        for operand in [
            Operand::Forward(Label::from_u32(0)),
            Operand::Backward(Label::from_u32(0)),
        ] {
            let mut nonvalue = ordinary;
            nonvalue.set_operand(operand);
            assert!(!none.can_finish_ordinary(&nonvalue, 0, None));
        }
    }

    #[test]
    fn ordinary_history_gate_keeps_source_sensitive_instructions_on_full_replay() {
        use super::OwnerDecisionIndices;
        use crate::assembler::{BorrowFallthrough, ExceptionPolicy, Instruction, Operand};
        use crate::target::opcodes::{
            COMPARE_OP, COPY_FREE_VARS, LOAD_CONST, LOAD_FAST_LOAD_FAST, LOAD_GLOBAL, MAKE_CELL,
            NOP, NOT_TAKEN, POP_TOP, RETURN_GENERATOR, RETURN_VALUE, SWAP,
        };

        let value =
            |opcode| Instruction::new(opcode, Operand::Value(0), super::SourceLocation::NONE, None);
        let indices = OwnerDecisionIndices::new(3, [1].iter());
        for opcode in [
            LOAD_CONST,
            LOAD_FAST_LOAD_FAST,
            LOAD_GLOBAL,
            COMPARE_OP,
            COPY_FREE_VARS,
            MAKE_CELL,
            RETURN_GENERATOR,
            SWAP,
        ] {
            let instruction = value(opcode);
            assert!(indices.can_replay_ordinary_history(&instruction, 0, Some(2)));
            assert!(!indices.can_replay_ordinary_history(&instruction, 1, None));
            assert!(!indices.can_replay_ordinary_history(&instruction, 2, Some(2)));
        }
        for opcode in [NOP, POP_TOP, NOT_TAKEN, RETURN_VALUE] {
            assert!(!indices.can_replay_ordinary_history(&value(opcode), 0, None));
        }
        for policy in [
            ExceptionPolicy::NoneUnchanged,
            ExceptionPolicy::NoneUnchangedBooleanSingleton,
            ExceptionPolicy::NonePresent,
            ExceptionPolicy::NoneAbsent,
            ExceptionPolicy::NoneDynamic,
            ExceptionPolicy::IfExtendedUnchanged,
            ExceptionPolicy::AlwaysUnchanged,
        ] {
            let mut instruction = value(LOAD_CONST);
            instruction.set_exception_policy(policy);
            assert_eq!(
                indices.can_replay_ordinary_history(&instruction, 0, None),
                matches!(
                    policy,
                    ExceptionPolicy::NoneUnchanged | ExceptionPolicy::NoneUnchangedBooleanSingleton
                ),
            );
        }
        let mut hidden = value(LOAD_CONST);
        hidden.local_policy.add_hidden_exception_owner_slots(1);
        assert!(!indices.can_replay_ordinary_history(&hidden, 0, None));
        let mut folded = value(LOAD_CONST);
        folded.add_pre_fold_extra_instructions(1);
        assert!(!indices.can_replay_ordinary_history(&folded, 0, None));
        let mut barrier = value(LOAD_CONST);
        barrier
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        assert!(!indices.can_replay_ordinary_history(&barrier, 0, None));
    }

    #[test]
    fn bounded_owner_replay_matches_full_decisions_and_marshaled_nested_code() {
        let tail = "    value += 1\n".repeat(160);
        for template in [
            "def run(value):\n    try:\n        if value is None:\n            value = 1\n    except (ValueError, TypeError):\n        value = 2\n#TAIL    return value\n",
            "def run(value, manager):\n    try:\n        with manager:\n            if value and manager:\n                value += 2\n    except ValueError as error:\n        value = 3 if error else 4\n    finally:\n        value += 1\n#TAIL    return value\n",
            "def run(value, values):\n    try:\n        result = [item for item in values if item and value]\n        if result:\n            value += len(result)\n    except (ValueError, TypeError):\n        value = 0\n#TAIL    return value\n",
            "async def run(value, manager):\n    try:\n        async with manager:\n            if value is not None and await manager:\n                value += 1\n    finally:\n        value += 2\n#TAIL    return value\n",
            "def run(value, manager):\n    try:\n        if manager:\n            value = manager(value)\n    except ValueError:\n        value = 1\n#TAIL    if value is None:\n        return 0\n    return value\n",
            "def run(value, manager):\n    try:\n        with manager:\n            if manager:\n                value += 1\n    finally:\n        value += 2\n#TAIL    return value and manager\n",
        ] {
            let source = template.replace("#TAIL", &tail);
            let compile = || {
                crate::compile(&source, "owner_replay.py")
                    .expect("owner replay example compiles")
                    .marshal()
            };
            let bounded = with_owner_replay_mode(false, false, true, compile);
            let ungated_bounded = with_owner_replay_mode(false, true, false, compile);
            let gated_complete = with_owner_replay_mode(true, false, true, compile);
            let complete = with_owner_replay_mode(true, true, false, compile);
            assert_eq!(bounded, ungated_bounded, "{template}");
            assert_eq!(bounded, gated_complete, "{template}");
            assert_eq!(bounded, complete, "{template}");
        }
    }

    #[test]
    fn ordinary_decision_gate_replays_histories_before_and_between_markers() {
        use std::fmt::Write;

        let mut ordinary = String::new();
        for index in 0..90 {
            writeln!(ordinary, "        value = (value + {index}) * 3").unwrap();
        }
        for template in [
            "def run(value, manager):\n    try:\n#ORDINARY        if value is None:\n            value += 1\n#ORDINARY        if value and manager:\n            return value\n#ORDINARY    except ValueError as error:\n        return value if error else manager\n    finally:\n        manager(value)\n",
            "def run(value, manager):\n    try:\n        with manager:\n            if value is None:\n                return value\n#ORDINARY        if value is not None and manager:\n            return value\n#ORDINARY    except ValueError as error:\n        if error is None:\n            return value\n        return value or manager\n",
            "def run(value, manager):\n    try:\n#ORDINARY        try:\n            if value and manager:\n                return value\n        except ValueError:\n            value += 1\n#ORDINARY        try:\n            if value is None:\n                return manager(value)\n        finally:\n            value += 3\n    finally:\n        manager(value)\n",
            "def run(value, manager):\n    try:\n#ORDINARY        payload = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]\n        options = {1, 3, 5, 7, 9, 11, 13}\n        if value is None or value in payload:\n            return value\n#ORDINARY    except ValueError as error:\n        try:\n            if error is None:\n                return manager(options)\n        finally:\n            manager(value)\n",
            "def run(value, manager):\n    for item in manager:\n        try:\n            if item and value:\n                return value\n#DEEP            if value is None:\n                break\n        finally:\n            manager(value)\n    return value\n",
            "async def run(value, manager):\n    try:\n        async with manager:\n            if value is not None:\n                await manager(value)\n#ORDINARY        if value or manager:\n            return value\n    finally:\n        await manager(value)\n",
        ] {
            let source = template.replace("#ORDINARY", &ordinary).replace(
                "#DEEP",
                &ordinary.replace("        value", "            value"),
            );
            let compile = || {
                crate::compile(&source, "owner_decision_gate.py")
                    .expect("owner decision example compiles")
                    .marshal()
            };
            let gated = with_owner_replay_mode(false, false, true, compile);
            let ungated = with_owner_replay_mode(false, true, false, compile);
            let complete = with_owner_replay_mode(true, true, false, compile);
            assert_eq!(gated, ungated, "{template}");
            assert_eq!(gated, complete, "{template}");
        }
    }

    #[test]
    fn ordinary_history_gate_matches_full_replay_for_fusion_and_comprehension_restores() {
        for source in [
            "def outer(value, manager, rows):\n    def run(flag):\n        nonlocal value\n        try:\n            with manager:\n                left, right = value, flag\n                value = left + right\n                if len(rows) > value:\n                    value = str(value)\n                return value if flag else len(rows)\n        except ValueError as error:\n            return value if error else flag\n        finally:\n            manager(value)\n    return run\n",
            "def run(flag, manager, rows):\n    try:\n        with manager:\n            values = [left + right for left, right in rows if left and flag]\n            pairs = {item: len(str(item)) for item in values if item is not None}\n            if len(pairs) > len(values):\n                return pairs if flag else values\n            return values or pairs\n    except ValueError as error:\n        return error if flag else rows\n    finally:\n        manager(rows)\n",
            "def run(value, manager, rows):\n    try:\n        with manager:\n            left, right = value, len(rows)\n            if left < right:\n                yield value\n            yield from (item for item in rows if item and value)\n            if len(rows) > right:\n                yield left if value else right\n    except ValueError as error:\n        yield error if value else manager\n    finally:\n        manager(value)\n",
            "async def run(value, manager, rows):\n    try:\n        async with manager:\n            left, right = value, len(rows)\n            result = [item for item in rows if item and value]\n            if left < right:\n                await manager(result)\n            if len(result) > right:\n                return result if value else rows\n    except ValueError as error:\n        return error if value else manager\n    finally:\n        await manager(value)\n",
        ] {
            let compile = || {
                crate::compile(source, "owner_history_gate.py")
                    .expect("owner history example compiles")
                    .marshal()
            };
            let gated = with_owner_replay_mode(false, false, true, compile);
            let ungated = with_owner_replay_mode(false, true, false, compile);
            let complete = with_owner_replay_mode(true, true, false, compile);
            assert_eq!(gated, ungated, "{source}");
            assert_eq!(gated, complete, "{source}");
        }
    }

    #[test]
    fn ordinary_history_gate_preserves_nested_finally_marker_owners() {
        use crate::{CompileOptions, PythonVersion};

        for source in [
            "def run(flag):\n    for _ in range(2):\n        try:\n            try:\n                consume('body')\n            finally:\n                try:\n                    consume('finally')\n                    break\n                finally:\n                    flag and consume(1)\n        except Exception:\n            consume('except')\n",
            "def run(flag):\n    for _ in range(2):\n        try:\n            try:\n                consume('body')\n            finally:\n                try:\n                    consume('finally')\n                    consume(flag)\n                    break\n                finally:\n                    if consume(flag):\n                        consume(1)\n        except Exception:\n            consume('except')\n",
        ] {
            for python_version in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                let compile = || {
                    crate::compile_with_options(
                        source,
                        "owner_history_cleanup.py",
                        &CompileOptions {
                            python_version,
                            ..CompileOptions::default()
                        },
                    )
                    .expect("nested finally example compiles")
                    .marshal()
                };
                let gated = with_owner_replay_mode(false, false, true, compile);
                let ungated = with_owner_replay_mode(false, true, false, compile);
                let complete = with_owner_replay_mode(true, true, false, compile);
                assert_eq!(gated, ungated, "{python_version:?}: {source}");
                assert_eq!(gated, complete, "{python_version:?}: {source}");
            }
        }
    }

    #[test]
    fn ordinary_decision_gate_retains_threaded_nonconditional_and_ordinary_successor() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{
            ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label,
            LayoutAssembler, NormalizedExceptionOwner, Operand, SourceLocation,
        };
        use crate::target::opcodes::{LOAD_CONST, LOAD_FAST_LOAD_FAST};

        let start = Label::from_u32(0);
        let end = Label::from_u32(1);
        let location = SourceLocation::new(1, 1, 0, 1);
        let mut origin = Instruction::new(LOAD_CONST, Operand::Forward(end), location, None);
        origin.set_exception_policy(ExceptionPolicy::AlwaysUnchanged);
        let mut successor = Instruction::new(LOAD_CONST, Operand::Value(1), location, None);
        successor.set_exception_owner(NormalizedExceptionOwner::DynamicConditionStrict);
        let mut assembler = LayoutAssembler {
            python_version: crate::PythonVersion::Py314,
            legacy_uniform_jump_seed: false,
            items: vec![
                Item::Label(start),
                Item::Instruction(Instruction::new(
                    LOAD_FAST_LOAD_FAST,
                    Operand::Value(0),
                    location,
                    None,
                )),
                Item::Instruction(origin),
                Item::Instruction(successor),
                Item::Instruction(Instruction::new(
                    LOAD_CONST,
                    Operand::Value(2),
                    location,
                    None,
                )),
                Item::Label(end),
            ],
            exception_regions: vec![ExceptionRegion {
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(start, end),
                start,
                end,
                target: end,
                depth: 0,
                preserve_lasti: false,
            }],
            exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                threaded_condition_origins: vec![(origin, successor)],
                ..ExceptionOwnerProvenance::default()
            }),
            preserved_block_boundaries: rustc_hash::FxHashSet::default(),
            removed_max_depth: None,
            next_label: 2,
            has_next_location: false,
        };
        let instructions = assembler
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            assembler.threaded_condition_positions(&instructions),
            [(1, NormalizedExceptionOwner::DynamicConditionStrict)]
                .into_iter()
                .collect(),
        );
        let gated = assembler.dynamic_exception_owners_with_replay::<true>(true);
        let ungated = assembler.dynamic_exception_owners_with_replay::<false>(true);
        assert_eq!(gated.slots, ungated.slots);
        let threaded = gated.get(2).expect("threaded successor has an owner");
        assert!(threaded.overrides_existing());
        assert!(!threaded.has_owner());

        assembler
            .exception_owner_provenance
            .threaded_condition_origins
            .clear();
        let gated = assembler.dynamic_exception_owners_with_replay::<true>(true);
        let ungated = assembler.dynamic_exception_owners_with_replay::<false>(true);
        assert_eq!(gated.slots, ungated.slots);
        assert!(!gated.get(2).unwrap().overrides_existing());
    }

    #[test]
    fn dense_owner_decisions_keep_strength_sources_and_overwrite_order() {
        use super::{DynamicExceptionOwners, ExceptionOwner, ExceptionOwnerDecision};

        let mut indexed = DynamicExceptionOwners::new(16);
        assert!(indexed.slots.is_empty());
        assert!(!indexed.contains_key(0));
        assert!(indexed.get(16).is_none());
        assert!(indexed.slots.is_empty());

        let mut index = 0;
        for owner in [
            ExceptionOwner::Absent,
            ExceptionOwner::Present,
            ExceptionOwner::SourceRegion(0),
            ExceptionOwner::SourceRegion(27),
            ExceptionOwner::SourceRegion(usize::MAX / 4 - 2),
        ] {
            for expected in [
                ExceptionOwnerDecision::Tentative(owner),
                ExceptionOwnerDecision::Definitive(owner),
                ExceptionOwnerDecision::Override(owner),
            ] {
                indexed.insert(index, expected);
                let actual = indexed.get(index).unwrap();
                assert!(indexed.contains_key(index));
                assert_eq!(actual.has_owner(), expected.has_owner());
                assert_eq!(actual.source_region(), expected.source_region());
                assert_eq!(actual.is_definitive(), expected.is_definitive());
                assert_eq!(actual.overrides_existing(), expected.overrides_existing());
                index += 1;
            }
        }
        assert_eq!(indexed.slots.len(), 16);
        assert!(indexed.get(15).is_none());

        indexed.extend([
            (2, ExceptionOwnerDecision::Tentative(ExceptionOwner::Absent)),
            (
                18,
                ExceptionOwnerDecision::Definitive(ExceptionOwner::Present),
            ),
            (
                2,
                ExceptionOwnerDecision::Override(ExceptionOwner::SourceRegion(4)),
            ),
        ]);
        assert!(indexed.get(17).is_none());
        assert!(indexed.get(18).unwrap().has_owner());
        assert!(indexed.get(2).unwrap().overrides_existing());
        assert_eq!(indexed.get(2).unwrap().source_region(), Some(4));
    }

    #[test]
    fn uncompacted_indexes_count_exact_records_with_shared_targets() {
        use super::UncompactedPrefixIndexes;
        use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
        use crate::target::opcodes::{POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE};

        let target = Label::from_u32(1);
        let repeated = Label::from_u32(2);
        let absent = Label::from_u32(3);
        let condition = |opcode, target| {
            Instruction::new(opcode, Operand::Forward(target), SourceLocation::NONE, None)
        };
        let unique = condition(POP_JUMP_IF_FALSE, target);
        let duplicate = condition(POP_JUMP_IF_TRUE, target);
        let items = [
            Item::Instruction(unique),
            Item::Label(repeated),
            Item::Instruction(duplicate),
            Item::Label(target),
            Item::Instruction(duplicate),
            Item::Label(repeated),
        ];
        let index = UncompactedPrefixIndexes::new(&items);
        assert!(index.has_unique_label(target));
        assert!(!index.has_unique_label(repeated));
        assert!(!index.has_unique_label(absent));
        assert!(index.has_unique_forward_jump(&items, unique));
        assert!(!index.has_unique_forward_jump(&items, duplicate));
        assert!(!index.has_unique_forward_jump(&items, condition(POP_JUMP_IF_FALSE, absent)));
    }

    #[test]
    fn owner_label_indexes_preserve_missing_repeated_and_sparse_labels() {
        use super::{OwnerLabelPositions, UniqueLabelPositions};
        use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
        use crate::target::opcodes::LOAD_CONST;

        let dense = Label::from_u32(3);
        let dense_unique = Label::from_u32(5);
        let missing = Label::from_u32(6);
        let sparse = Label::from_u32(500);
        let sparse_unique = Label::from_u32(600);
        let items = [
            Item::Label(dense),
            Item::Instruction(Instruction::new(
                LOAD_CONST,
                Operand::Value(0),
                SourceLocation::NONE,
                None,
            )),
            Item::Label(sparse),
            Item::Label(dense_unique),
            Item::Label(sparse_unique),
            Item::Label(dense),
            Item::Label(sparse),
            Item::Label(dense),
            Item::Label(sparse),
        ];
        let unique = UniqueLabelPositions::new(&items);
        assert_eq!(unique.get(dense), None);
        assert_eq!(unique.get(sparse), None);
        assert_eq!(unique.get(missing), None);
        assert_eq!(unique.get(dense_unique), Some((3, 1)));
        assert_eq!(unique.get(sparse_unique), Some((4, 1)));

        let mut owners = OwnerLabelPositions::new(items.len());
        for (index, item) in items.iter().enumerate() {
            if let Item::Label(label) = item {
                owners.insert(*label, index);
            }
        }
        assert_eq!(owners.get(&dense), Some(&7));
        assert_eq!(owners.get(&sparse), Some(&8));
        assert_eq!(owners.get(&missing), None);
        assert_eq!(owners[&dense_unique], 3);
        assert_eq!(owners[&sparse_unique], 4);
    }

    #[test]
    fn owner_replay_flags_match_independent_sets_for_dense_and_outlier_labels() {
        use super::{OwnerReplayLabelFlags as Flags, OwnerReplayLabels};
        use crate::assembler::Label;
        use rustc_hash::FxHashSet;

        let group = |ids: &[u32]| {
            ids.iter()
                .copied()
                .map(Label::from_u32)
                .collect::<FxHashSet<_>>()
        };
        let groups = [
            (Flags::BOUNDARY, group(&[0, 7, 63, 64, 1_000_000])),
            (Flags::JUMP_TARGET, group(&[7, 63, 65, 1_000_001])),
            (Flags::SOURCE_BOUNDARY, group(&[0, 7, 64, 1_000_001])),
            (Flags::OMITTED_NOP, group(&[7, 63, 1_000_000])),
            (Flags::OMITTED_NOP, group(&[7, 64, 1_000_001])),
            (Flags::SETUP, group(&[2, 63, 64, 1_000_001])),
        ];
        for initial_labels in [0, 8, 100_000] {
            let mut indexed = OwnerReplayLabels::new(1, initial_labels);
            for (flag, labels) in &groups {
                indexed.record(labels, *flag);
            }
            assert!(indexed.dense.len() <= 64);
            for id in (0..=70).chain(999_999..=1_000_002) {
                let label = Label::from_u32(id);
                for mask in [
                    Flags::BOUNDARY,
                    Flags::JUMP_TARGET,
                    Flags::SOURCE_BOUNDARY,
                    Flags::OMITTED_NOP,
                    Flags::SETUP,
                    Flags::BOUNDARY | Flags::JUMP_TARGET | Flags::SETUP,
                    Flags::REGION_END | Flags::SOURCE_BOUNDARY,
                    Flags::CONTINUATION | Flags::RETIRED_TRY_COPY,
                ] {
                    assert_eq!(
                        indexed.get(label).contains(mask),
                        groups
                            .iter()
                            .any(|(flag, labels)| mask & flag != 0 && labels.contains(&label)),
                        "label {id}, mask {mask}"
                    );
                }
            }
        }
    }

    #[test]
    fn dynamic_replay_preserves_non_marker_dynamic_policies() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{
            ExceptionRegion, ExceptionRegionPriority, Instruction, Item, Label, LayoutAssembler,
            NormalizedExceptionOwner, Operand, SourceLocation,
        };
        use crate::target::opcodes::LOAD_CONST;

        let start = Label::from_u32(0);
        let end = Label::from_u32(1);
        for owner in [
            NormalizedExceptionOwner::Unchanged,
            NormalizedExceptionOwner::Dynamic,
            NormalizedExceptionOwner::DynamicCondition,
            NormalizedExceptionOwner::DynamicConditionStrict,
        ] {
            let mut instruction = Instruction::new(
                LOAD_CONST,
                Operand::Value(0),
                SourceLocation::new(1, 1, 0, 1),
                None,
            );
            instruction.set_exception_owner(owner);
            let mut assembler = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items: vec![
                    Item::Label(start),
                    Item::Instruction(instruction),
                    Item::Label(end),
                ],
                exception_regions: vec![ExceptionRegion {
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(start, end),
                    start,
                    end,
                    target: end,
                    depth: 0,
                    preserve_lasti: false,
                }],
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    source_boundaries: [start].into_iter().collect(),
                    block_boundaries: [start].into_iter().collect(),
                    ..ExceptionOwnerProvenance::default()
                }),
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 2,
                has_next_location: false,
            };
            let owners = assembler.dynamic_exception_owners();
            assert_eq!(
                owners.contains_key(0),
                owner != NormalizedExceptionOwner::Unchanged,
                "{owner:?}",
            );
            for tail_length in [0, 1, 160] {
                let tail = Instruction::new(
                    LOAD_CONST,
                    Operand::Value(1),
                    SourceLocation::new(2, 2, 0, 1),
                    None,
                );
                assembler.items.truncate(2);
                assembler
                    .items
                    .extend(std::iter::repeat_n(Item::Instruction(tail), tail_length));
                assembler.items.push(Item::Label(end));
                let bounded = assembler.dynamic_exception_owners_with_replay::<true>(true);
                let complete = assembler.dynamic_exception_owners_with_replay::<false>(false);
                assert_eq!(bounded.instruction_count, complete.instruction_count);
                assert_eq!(bounded.slots, complete.slots, "{owner:?}, {tail_length}");
            }
        }
    }

    #[test]
    fn for_target_owner_prefix_requires_the_exact_surviving_entry() {
        use std::cell::OnceCell;

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
            let body = Label::from_u32(2);
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
                Item::Label(Label::from_u32(0)),
                Item::Instruction(Instruction::new(
                    LOAD_CONST,
                    Operand::Value(0),
                    location,
                    None,
                )),
                Item::Label(Label::from_u32(1)),
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
                "intervening label" => items.insert(5, Item::Label(Label::from_u32(3))),
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

            let positions =
                provenance.original_for_target_prefix_positions(&items, &OnceCell::new());
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

        let setup = Label::from_u32(0);
        let end = Label::from_u32(1);
        let exit = Label::from_u32(2);
        let bookkeeping = Label::from_u32(3);
        let handler = Label::from_u32(4);
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
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
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
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    setups: vec![(setup, Some(location))],
                    ..ExceptionOwnerProvenance::default()
                }),
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 5,
                has_next_location: false,
            };
            if outer_region {
                // An earlier outer handler invalidates the first unprotected array
                // origin even though all compaction and insertion counts are zero.
                let outer_start = Label::from_u32(5);
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
            let decision = decisions.get(3).expect("dynamic marker decision");
            assert!(!decision.has_owner());
            assert_eq!(decision.is_definitive(), !barrier && !outer_region);
            assert_eq!(decision.overrides_existing(), !barrier && !outer_region);

            if !barrier && !outer_region {
                let Item::Instruction(instruction) = &mut assembler.items[5] else {
                    unreachable!();
                };
                instruction.opcode = LOAD_FAST;
                let decisions = assembler.dynamic_exception_owners();
                let decision = decisions.get(3).expect("ordinary dynamic owner decision");
                assert!(!decision.overrides_existing());
            }
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
                Item::Label(Label::from_u32(0)),
                plain(NOP, Operand::Value(0)),
                plain(NOP, Operand::Value(0)),
                plain(STORE_FAST, Operand::Value(4)),
                plain(STORE_FAST, Operand::Value(5)),
                plain(LOAD_FAST, Operand::Value(0)),
                plain(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(2))),
                plain(NOT_TAKEN, Operand::Value(0)),
                Item::Label(Label::from_u32(1)),
                plain(LOAD_CONST, Operand::Value(0)),
                plain(RETURN_VALUE, Operand::Value(0)),
                Item::Label(Label::from_u32(2)),
                plain(JUMP_FORWARD, Operand::Forward(Label::from_u32(3))),
                Item::Label(Label::from_u32(3)),
                plain(LOAD_CONST, Operand::Value(0)),
                plain(RETURN_VALUE, Operand::Value(0)),
                Item::Label(Label::from_u32(4)),
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
                    start: Label::from_u32(0),
                    end: Label::from_u32(1),
                    target: Label::from_u32(4),
                    depth: 2,
                    preserve_lasti: true,
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(
                        Label::from_u32(0),
                        Label::from_u32(1),
                    ),
                }],
                next_label: 6,
                ..Assembler::default()
            }
            .into_optimization_pipeline();
            pipeline
                .conditional_threading_targets
                .insert(Label::from_u32(2), location);
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
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
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
                    assembler.items.insert(1, Item::Label(Label::from_u32(5)));
                    assembler
                        .exception_owner_provenance
                        .setups
                        .push((Label::from_u32(5), None));
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
                    condition.retarget(Operand::Forward(Label::from_u32(1)));
                }
                "duplicate origin" => assembler
                    .exception_owner_provenance
                    .threaded_condition_origins
                    .push(origin),
                "duplicate pair" => assembler
                    .items
                    .extend([Item::Instruction(origin.0), Item::Instruction(origin.1)]),
                "new join" => {
                    assembler
                        .items
                        .insert(marker_item, Item::Label(Label::from_u32(5)));
                    assembler
                        .preserved_block_boundaries
                        .insert(Label::from_u32(5));
                }
                "bookkeeping" => {
                    assembler
                        .items
                        .insert(marker_item, Item::Label(Label::from_u32(5)));
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
                        .insert(marker_item - 1, Item::Label(Label::from_u32(5)));
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
            let decision = owners.get(marker_index).expect("dynamic marker decision");
            assert_eq!(
                !decision.has_owner() && decision.is_definitive() && decision.overrides_existing(),
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
                Operand::Forward(Label::from_u32(9)),
                location,
                None,
            );
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, None);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);
            let mut assembler = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items: vec![
                    Item::Label(Label::from_u32(0)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label::from_u32(1)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label::from_u32(2)),
                    Item::Label(Label::from_u32(3)),
                    plain(NOP, Operand::Value(0)),
                    Item::Label(Label::from_u32(4)),
                    Item::Label(Label::from_u32(5)),
                    Item::Label(Label::from_u32(6)),
                    plain(LOAD_FAST, Operand::Value(0)),
                    Item::Instruction(condition),
                    Item::Instruction(marker),
                    Item::Label(Label::from_u32(8)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                    Item::Label(Label::from_u32(9)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                ],
                exception_regions: Vec::new(),
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    setups: vec![
                        (Label::from_u32(1), None),
                        (Label::from_u32(2), None),
                        (Label::from_u32(5), Some(location)),
                        (Label::from_u32(6), None),
                    ],
                    block_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    source_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    setup_nop_boundaries: [Label::from_u32(3), Label::from_u32(4)]
                        .into_iter()
                        .collect(),
                    setup_nop_ranges: vec![(Label::from_u32(3), Label::from_u32(4))],
                    synchronous_with_setups: [Label::from_u32(1), Label::from_u32(2)]
                        .into_iter()
                        .collect(),
                    source_none_conditions: vec![SourceNoneCondition {
                        entry: Label::from_u32(0),
                        labels: (0..=6).map(Label::from_u32).collect(),
                        instruction: condition,
                        entry_prefix: Some(vec![Item::Label(Label::from_u32(0))]),
                    }],
                    ..ExceptionOwnerProvenance::default()
                }),
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 11,
                has_next_location: false,
            };
            match change {
                "bookkeeping" => assembler.items.insert(12, Item::Label(Label::from_u32(7))),
                "one setup owned load" => {
                    assembler.items.remove(4);
                    let provenance = &mut assembler.exception_owner_provenance;
                    provenance
                        .setups
                        .retain(|(label, _)| *label != Label::from_u32(2));
                    provenance.source_none_conditions[0]
                        .labels
                        .retain(|label| *label != Label::from_u32(2));
                    provenance
                        .synchronous_with_setups
                        .remove(&Label::from_u32(2));
                }
                "no origin" => assembler
                    .exception_owner_provenance
                    .source_none_conditions
                    .clear(),
                "retarget" => {
                    let mut changed = condition;
                    changed.set_operand(Operand::Forward(Label::from_u32(8)));
                    assembler.items[11] = Item::Instruction(changed);
                }
                "duplicate condition" => assembler.items.push(Item::Instruction(condition)),
                "new join" => assembler.items.insert(10, Item::Label(Label::from_u32(10))),
                "old target" => {
                    assembler
                        .preserved_block_boundaries
                        .insert(Label::from_u32(3));
                }
                "incoming jump" => assembler
                    .items
                    .push(plain(JUMP_FORWARD, Operand::Forward(Label::from_u32(3)))),
                "copied entry" => {
                    assembler
                        .exception_owner_provenance
                        .normal_try_copy_blocks
                        .insert(Label::from_u32(0));
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
                        Operand::Forward(Label::from_u32(9)),
                        location,
                        None,
                    );
                    changed.local_policy.add_hidden_exception_owner_slots(2);
                    assembler.items[11] = Item::Instruction(changed);
                    assembler.exception_owner_provenance.source_none_conditions[0].instruction =
                        changed;
                }
                "duplicate label" => assembler.items.push(Item::Label(Label::from_u32(0))),
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
            let positions = crate::assembler::exception_owners::OwnerLabelPositions::from_hashed(
                assembler.items.len(),
                &positions,
            );
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
            let mut source_prefix = vec![Item::Label(Label::from_u32(0))];
            if returned {
                source_prefix.push(Item::Instruction(plain(
                    NOP,
                    Operand::Value(0),
                    prefix_location,
                )));
            }
            source_prefix.push(Item::Label(Label::from_u32(10)));
            if !handler {
                source_prefix.push(Item::Instruction(source_pop));
            }
            let prefix_instruction = if handler {
                plain(PUSH_EXC_INFO, Operand::Value(0), SourceLocation::NONE)
            } else {
                current_pop
            };
            let mut condition = plain(
                POP_JUMP_IF_NOT_NONE,
                Operand::Forward(Label::from_u32(9)),
                location,
            );
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = plain(NOT_TAKEN, Operand::Value(0), location);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);
            let mut assembler = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items: vec![
                    Item::Instruction(plain(
                        JUMP_FORWARD,
                        Operand::Forward(Label::from_u32(0)),
                        prefix_location,
                    )),
                    Item::Label(Label::from_u32(0)),
                    Item::Label(Label::from_u32(10)),
                    Item::Instruction(prefix_instruction),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Label(Label::from_u32(1)),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Label(Label::from_u32(2)),
                    Item::Label(Label::from_u32(3)),
                    Item::Instruction(plain(NOP, Operand::Value(0), setup_location)),
                    Item::Label(Label::from_u32(4)),
                    Item::Label(Label::from_u32(5)),
                    Item::Label(Label::from_u32(6)),
                    Item::Instruction(plain(LOAD_FAST, Operand::Value(0), location)),
                    Item::Instruction(condition),
                    Item::Instruction(marker),
                    Item::Label(Label::from_u32(8)),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Instruction(plain(RETURN_VALUE, Operand::Value(0), location)),
                    Item::Label(Label::from_u32(9)),
                    Item::Instruction(plain(LOAD_CONST, Operand::Value(0), location)),
                    Item::Instruction(plain(RETURN_VALUE, Operand::Value(0), location)),
                ],
                exception_regions: Vec::new(),
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    setups: vec![
                        (Label::from_u32(1), None),
                        (Label::from_u32(2), None),
                        (Label::from_u32(5), Some(setup_location)),
                        (Label::from_u32(6), None),
                    ],
                    block_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    source_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    setup_nop_boundaries: [Label::from_u32(3), Label::from_u32(4)]
                        .into_iter()
                        .collect(),
                    setup_nop_ranges: vec![(Label::from_u32(3), Label::from_u32(4))],
                    source_none_conditions: vec![SourceNoneCondition {
                        entry: Label::from_u32(0),
                        labels: vec![
                            Label::from_u32(0),
                            Label::from_u32(10),
                            Label::from_u32(1),
                            Label::from_u32(2),
                            Label::from_u32(3),
                            Label::from_u32(4),
                            Label::from_u32(5),
                            Label::from_u32(6),
                        ],
                        instruction: condition,
                        entry_prefix: Some(source_prefix),
                    }],
                    ..ExceptionOwnerProvenance::default()
                }),
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 12,
                has_next_location: false,
            };
            if handler {
                assembler
                    .exception_owner_provenance
                    .setups
                    .insert(0, (Label::from_u32(10), None));
                assembler
                    .preserved_block_boundaries
                    .insert(Label::from_u32(10));
            }
            match change {
                "setup on source entry" => assembler
                    .exception_owner_provenance
                    .setups
                    .insert(0, (Label::from_u32(0), None)),
                "missing prefix" => {
                    assembler.exception_owner_provenance.source_none_conditions[0].entry_prefix =
                        None;
                }
                "uncaptured alias" => {
                    assembler.exception_owner_provenance.source_none_conditions[0]
                        .entry_prefix
                        .as_mut()
                        .unwrap()
                        .retain(|item| *item != Item::Label(Label::from_u32(10)));
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
                    assembler
                        .preserved_block_boundaries
                        .insert(Label::from_u32(10));
                }
                "two predecessors" => assembler.items.push(Item::Instruction(plain(
                    JUMP_FORWARD,
                    Operand::Forward(Label::from_u32(0)),
                    prefix_location,
                ))),
                "unknown interior label" => {
                    assembler.items.insert(5, Item::Label(Label::from_u32(11)));
                    assembler.exception_owner_provenance.source_none_conditions[0]
                        .labels
                        .insert(2, Label::from_u32(11));
                }
                "copied alias" => {
                    assembler
                        .exception_owner_provenance
                        .normal_try_copy_blocks
                        .insert(Label::from_u32(10));
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
                    .retain(|(label, _)| *label != Label::from_u32(6)),
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
            let positions = crate::assembler::exception_owners::OwnerLabelPositions::from_hashed(
                assembler.items.len(),
                &positions,
            );
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
                Operand::Forward(Label::from_u32(9)),
                location,
                None,
            );
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), location, None);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);
            LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items: vec![
                    Item::Label(Label::from_u32(0)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label::from_u32(1)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    Item::Label(Label::from_u32(2)),
                    Item::Label(Label::from_u32(3)),
                    plain(NOP, Operand::Value(0)),
                    Item::Label(Label::from_u32(4)),
                    Item::Label(Label::from_u32(5)),
                    Item::Label(Label::from_u32(6)),
                    plain(LOAD_FAST, Operand::Value(0)),
                    Item::Instruction(condition),
                    Item::Label(Label::from_u32(7)),
                    Item::Instruction(marker),
                    Item::Label(Label::from_u32(8)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                    Item::Label(Label::from_u32(9)),
                    plain(LOAD_CONST, Operand::Value(0)),
                    plain(RETURN_VALUE, Operand::Value(0)),
                ],
                exception_regions: vec![ExceptionRegion {
                    is_scope_fallback: false,
                    priority: ExceptionRegionPriority::OriginalRange(
                        Label::from_u32(0),
                        Label::from_u32(8),
                    ),
                    start: Label::from_u32(0),
                    end: Label::from_u32(7),
                    target: Label::from_u32(9),
                    depth: 0,
                    preserve_lasti: false,
                }],
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    setups: vec![
                        (Label::from_u32(1), None),
                        (Label::from_u32(2), None),
                        (Label::from_u32(5), Some(location)),
                        (Label::from_u32(6), None),
                    ],
                    block_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    source_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    setup_nop_boundaries: [Label::from_u32(3), Label::from_u32(4)]
                        .into_iter()
                        .collect(),
                    setup_nop_ranges: vec![(Label::from_u32(3), Label::from_u32(4))],
                    synchronous_with_setups: [Label::from_u32(1), Label::from_u32(2)]
                        .into_iter()
                        .collect(),
                    source_none_conditions: vec![SourceNoneCondition {
                        entry: Label::from_u32(0),
                        labels: (0..=6).map(Label::from_u32).collect(),
                        instruction: condition,
                        entry_prefix: Some(vec![Item::Label(Label::from_u32(0))]),
                    }],
                    ..ExceptionOwnerProvenance::default()
                }),
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
                        .push(plain(JUMP_FORWARD, Operand::Forward(Label::from_u32(7))));
                }
                "source boundary" => {
                    provenance.source_boundaries.insert(Label::from_u32(7));
                }
                "block boundary" => {
                    provenance.block_boundaries.insert(Label::from_u32(7));
                }
                "setup boundary" => {
                    provenance.setups.push((Label::from_u32(7), None));
                }
                "pop boundary" => {
                    provenance
                        .removed_pop_block_boundaries
                        .insert(Label::from_u32(7));
                }
                "copied boundary" => {
                    provenance.normal_try_copy_blocks.insert(Label::from_u32(7));
                }
                "preserved boundary" => {
                    assembler
                        .preserved_block_boundaries
                        .insert(Label::from_u32(7));
                }
                "handler target" => {
                    assembler.exception_regions[0].target = Label::from_u32(7);
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
            let positions = crate::assembler::exception_owners::OwnerLabelPositions::from_hashed(
                assembler.items.len(),
                &positions,
            );
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
    fn copied_return_pop_constants_remap_without_reviving_removed_or_changed_records() {
        use crate::assembler::{Assembler, ExceptionPolicy, Item, Operand, SourceLocation};
        use crate::target::opcodes::{LOAD_CONST, NOP};

        for change in ["surviving constant", "removed constant", "changed location"] {
            let mut assembler = Assembler::default();
            let entry = assembler.label();
            assembler.record_exception_owner_block_boundary(entry);
            assembler.mark(entry);
            let first_pop_item = assembler.item_count();
            for _ in 0..2 {
                assembler.emit_with_depth(NOP, 0, 0, ExceptionPolicy::EXCLUDE_ALWAYS);
                assembler.mark_last_as_converted_pop_block();
            }
            assembler.set_location(SourceLocation::new(9, 9, 4, 12));
            let load_item = assembler.item_count();
            assembler.emit(LOAD_CONST, 2);
            assembler.record_copied_return_pop_prefix(first_pop_item);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .copied_return_pop_prefixes
                    .len(),
                1,
            );
            let original_records = assembler
                .exception_owner_provenance
                .copied_return_pop_prefixes[0]
                .records;
            if change == "changed location" {
                let Item::Instruction(load) = &mut assembler.linear_items_mut()[load_item] else {
                    unreachable!();
                };
                load.location = SourceLocation::new(8, 8, 4, 12);
            }
            let retained = change != "removed constant";
            assembler.remap_constant_indices(&[Some(0), None, retained.then_some(1)]);
            let Item::Instruction(live) = assembler.linear_items()[load_item] else {
                unreachable!();
            };
            assert_eq!(
                live.opcode,
                if retained { LOAD_CONST } else { NOP },
                "{change}"
            );
            assert_eq!(
                live.operand(),
                Operand::Value(u32::from(retained)),
                "{change}"
            );
            let origins = &assembler
                .exception_owner_provenance
                .copied_return_pop_prefixes;
            assert_eq!(origins.len(), usize::from(retained), "{change}");
            if retained {
                assert_eq!(origins[0].entry, entry, "{change}");
                assert_eq!(origins[0].records[..2], original_records[..2], "{change}");
                assert_eq!(
                    origins[0].records[2].operand(),
                    Operand::Value(1),
                    "{change}"
                );
                assert_eq!(
                    origins[0].records[2] == live,
                    change == "surviving constant",
                    "{change}",
                );
            }
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
            assembler.remap_constant_indices(&[Some(0), None, retained.then_some(1)]);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .original_normal_finally_arrays
                    .len(),
                usize::from(retained),
                "{change}"
            );
            let layout = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
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
    fn nested_finally_retired_try_exit_requires_original_edge() {
        use super::{ExceptionOwnerProvenance, OriginalNormalFinallyArray};
        use crate::assembler::{
            ExceptionPolicy, Instruction, Item, Label, LayoutAssembler, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            JUMP_FORWARD, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, TO_BOOL,
        };

        for change in [
            "unchanged",
            "refined owner",
            "refined missing edge",
            "unproven refined owner",
            "different refined record",
            "missing edge",
            "wrong source",
            "wrong target",
            "duplicate edge",
            "duplicate source label",
            "moved source label",
            "changed donor location",
            "changed donor depth",
            "changed setup slots",
        ] {
            let source = Label::from_u32(0);
            let entry = Label::from_u32(1);
            let setup_start = Label::from_u32(2);
            let setup_end = Label::from_u32(3);
            let setup_label = Label::from_u32(4);
            let exit = Label::from_u32(5);
            let location = SourceLocation::new(9, 9, 15, 19);
            let plain = |opcode, operand| Instruction::new(opcode, operand, location, None);
            let mut jump = plain(JUMP_FORWARD, Operand::Forward(entry));
            jump.location = SourceLocation::new(3, 3, 8, 12);
            if change.contains("refined") {
                jump.jump_policy.retain_nop_for_direct_target();
            }
            let mut donor = jump;
            donor.opcode = NOP;
            donor.set_operand(Operand::Value(0));
            let mut pop = plain(NOP, Operand::Value(0));
            pop.location = SourceLocation::NONE;
            pop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            pop.fusion_policy.mark_converted_pop_block();
            let mut setup = plain(NOP, Operand::Value(0));
            setup.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            let mut marker = plain(NOT_TAKEN, Operand::Value(0));
            marker.set_exception_policy(ExceptionPolicy::normalized_owner(true));
            let prefix = vec![
                Item::Label(entry),
                Item::Instruction(pop),
                Item::Label(setup_start),
                Item::Instruction(setup),
                Item::Label(setup_end),
                Item::Label(setup_label),
                Item::Instruction(plain(LOAD_FAST, Operand::Value(0))),
                Item::Instruction(plain(TO_BOOL, Operand::Value(0))),
                Item::Instruction(plain(POP_JUMP_IF_FALSE, Operand::Forward(exit))),
                Item::Instruction(marker),
            ];
            let mut items = vec![Item::Label(source), Item::Instruction(donor)];
            items.extend_from_slice(&prefix);
            items.remove(3);
            let Item::Instruction(setup) = &mut items[4] else {
                unreachable!()
            };
            setup.local_policy.add_hidden_exception_owner_slots(1);
            let mut assembler = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_regions: Vec::new(),
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    source_boundaries: [entry].into_iter().collect(),
                    block_boundaries: [entry].into_iter().collect(),
                    compound_try_owner_exits: vec![(source, entry)],
                    setups: vec![(setup_label, Some(location))],
                    setup_nop_ranges: vec![(setup_start, setup_end)],
                    original_normal_finally_arrays: vec![OriginalNormalFinallyArray {
                        entry,
                        prefix,
                        incoming: vec![jump],
                        retained_setup: Some((setup_start, setup_end, setup_label)),
                        borrow_join: None,
                    }],
                    ..ExceptionOwnerProvenance::default()
                }),
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 6,
                has_next_location: false,
            };
            match change {
                "unchanged" => {}
                "refined owner"
                | "refined missing edge"
                | "unproven refined owner"
                | "different refined record" => {
                    let Item::Instruction(donor) = &mut assembler.items[1] else {
                        unreachable!()
                    };
                    donor.set_exception_policy(ExceptionPolicy::normalized_owner(false));
                    if change == "refined missing edge" {
                        assembler
                            .exception_owner_provenance
                            .compound_try_owner_exits
                            .clear();
                    }
                    if change != "unproven refined owner" {
                        let mut original = jump;
                        if change == "different refined record" {
                            original.location.line += 1;
                        }
                        assembler
                            .exception_owner_provenance
                            .ownerless_compound_try_nops
                            .push((source, entry, original));
                    }
                }
                "missing edge" => assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .clear(),
                "wrong source" => {
                    assembler
                        .exception_owner_provenance
                        .compound_try_owner_exits[0]
                        .0 = exit;
                }
                "wrong target" => {
                    assembler
                        .exception_owner_provenance
                        .compound_try_owner_exits[0]
                        .1 = exit;
                }
                "duplicate edge" => assembler
                    .exception_owner_provenance
                    .compound_try_owner_exits
                    .push((source, entry)),
                "duplicate source label" => assembler.items.push(Item::Label(source)),
                "moved source label" => assembler.items.swap(0, 1),
                "changed donor location" | "changed donor depth" => {
                    let Item::Instruction(donor) = &mut assembler.items[1] else {
                        unreachable!()
                    };
                    if change == "changed donor location" {
                        donor.location.line += 1;
                    } else {
                        *donor = Instruction::new(
                            donor.opcode,
                            donor.operand(),
                            donor.location,
                            Some(1),
                        );
                    }
                }
                "changed setup slots" => {
                    let Item::Instruction(setup) = &mut assembler.items[4] else {
                        unreachable!()
                    };
                    setup.local_policy.add_hidden_exception_owner_slots(1);
                }
                _ => unreachable!(),
            }
            let expected = if matches!(change, "unchanged" | "refined owner") {
                [0, 5].into_iter().collect()
            } else {
                rustc_hash::FxHashSet::default()
            };
            assert_eq!(
                assembler.original_normal_finally_null_markers(&rustc_hash::FxHashSet::default()),
                expected,
                "{change}"
            );
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
            let entry = Label::from_u32(0);
            let range_end = Label::from_u32(1);
            let marker_alias = Label::from_u32(2);
            let exit = Label::from_u32(3);
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
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items: prefix.clone(),
                exception_regions: Vec::new(),
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    source_boundaries: [entry].into_iter().collect(),
                    block_boundaries: [entry].into_iter().collect(),
                    original_normal_finally_arrays: vec![OriginalNormalFinallyArray {
                        retained_setup: None,
                        borrow_join: None,
                        entry,
                        prefix,
                        incoming: vec![plain(JUMP_FORWARD, Operand::Forward(entry))],
                    }],
                    ..ExceptionOwnerProvenance::default()
                }),
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
                "foreign inserted label" => {
                    assembler.items.insert(4, Item::Label(Label::from_u32(4)));
                }
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
                    assembler.items[5] = Item::Instruction(plain(
                        POP_JUMP_IF_FALSE,
                        Operand::Forward(Label::from_u32(4)),
                    ));
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
            CALL, JUMP_FORWARD, LOAD_ATTR, LOAD_FAST, NOT_TAKEN, POP_JUMP_IF_FALSE, STORE_FAST,
            SWAP, TO_BOOL,
        };

        for (opcode, case) in [CALL, LOAD_ATTR, LOAD_FAST].into_iter().flat_map(|opcode| {
            ["original", "unsealed", "cloned", "retired", "split restore"]
                .map(|case| (opcode, case))
        }) {
            let entry = Label::from_u32(0);
            let exit = Label::from_u32(1);
            let clone = Label::from_u32(2);
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
                Item::Instruction(plain(opcode, Operand::Value(0))),
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
            } else if case == "split restore" {
                // Even a non-target label splits the originally sealed array.
                items.insert(3, Item::Label(Label::from_u32(3)));
            }
            provenance.capture_uncompacted_conditional_arrays(&items, false);
            assert_eq!(
                provenance.uncompacted_conditional_arrays.len(),
                usize::from(!matches!(case, "unsealed" | "split restore")),
                "{opcode:?}: {case}"
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
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_owner_provenance: Box::new(provenance),
                exception_regions: vec![],
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 4,
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
            let unique = super::OnceCell::new();
            let markers = assembler.uncompacted_conditional_null_markers::<true>(&targets, &unique);
            assert_eq!(markers, expected, "{opcode:?}: {case}");
            assert_eq!(
                markers,
                assembler.uncompacted_conditional_null_markers::<false>(&targets, &unique),
                "{opcode:?}: {case}"
            );
        }
    }

    #[test]
    fn expanded_comprehension_continuations_require_a_contiguous_marker() {
        use super::{ExceptionOwnerProvenance, OnceCell};
        use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
        use crate::target::opcodes::{
            BUILD_TUPLE, LOAD_CONST, LOAD_FAST, NOT_TAKEN, POP_JUMP_IF_FALSE, STORE_FAST, SWAP,
            TO_BOOL,
        };

        let entry = Label::from_u32(0);
        let exit = Label::from_u32(1);
        let alias = Label::from_u32(2);
        let make = |opcode, operand, depth| {
            Item::Instruction(Instruction::new(
                opcode,
                operand,
                SourceLocation::new(3, 3, 0, 4),
                Some(depth),
            ))
        };
        let prefix = vec![
            Item::Label(entry),
            make(SWAP, Operand::Value(2), 2),
            make(STORE_FAST, Operand::Value(0), 1),
            make(LOAD_CONST, Operand::Value(0), 2),
            make(BUILD_TUPLE, Operand::Value(2), 1),
            make(STORE_FAST, Operand::Value(20), 0),
            make(LOAD_FAST, Operand::Value(20), 1),
            make(TO_BOOL, Operand::Value(0), 1),
            make(POP_JUMP_IF_FALSE, Operand::Forward(exit), 0),
            make(NOT_TAKEN, Operand::Value(0), 0),
        ];
        let provenance = ExceptionOwnerProvenance::default();
        for gap in [None, Some(5), Some(7), Some(9)] {
            let mut candidate = prefix.clone();
            if let Some(gap) = gap {
                candidate.insert(gap, Item::Label(alias));
            }
            let mut all = candidate.clone();
            all.push(Item::Label(exit));
            assert_eq!(
                provenance.uncompacted_prefix(&candidate, &all, &OnceCell::new(), 1, false),
                gap.is_none(),
                "gap: {gap:?}"
            );
        }
    }

    #[test]
    fn comprehension_async_yield_requires_an_intact_discard_sequence() {
        use super::{ExceptionOwnerProvenance, OnceCell};
        use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
        use crate::target::opcodes::{
            CALL_INTRINSIC_1, LOAD_FAST, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_TOP, RESUME, STORE_FAST,
            SWAP, TO_BOOL, YIELD_VALUE,
        };
        use crate::target::operands::UnaryIntrinsic;

        let entry = Label::from_u32(0);
        let exit = Label::from_u32(1);
        let make = |opcode, operand, depth| {
            Item::Instruction(Instruction::new(
                opcode,
                operand,
                SourceLocation::new(3, 3, 0, 4),
                depth,
            ))
        };
        for case in [
            "intact",
            "resume in handler",
            "wrong intrinsic",
            "yield from",
            "resume after await",
            "stored result",
            "different location",
            "unknown depth",
            "changed depth",
            "depth underflow",
            "interior label",
            "following label",
        ] {
            let mut prefix = vec![
                Item::Label(entry),
                make(SWAP, Operand::Value(2), Some(2)),
                make(STORE_FAST, Operand::Value(0), Some(1)),
                make(LOAD_FAST, Operand::Value(20), Some(2)),
                make(
                    CALL_INTRINSIC_1,
                    Operand::Value(UnaryIntrinsic::ASYNC_GEN_WRAP.argument()),
                    Some(2),
                ),
                make(YIELD_VALUE, Operand::Value(0), Some(2)),
                make(RESUME, Operand::Value(1), Some(2)),
                make(POP_TOP, Operand::Value(0), Some(1)),
                make(LOAD_FAST, Operand::Value(20), Some(2)),
                make(TO_BOOL, Operand::Value(0), Some(2)),
                make(POP_JUMP_IF_FALSE, Operand::Forward(exit), Some(1)),
                make(NOT_TAKEN, Operand::Value(0), Some(1)),
            ];
            match case {
                "intact" => {}
                "resume in handler" => prefix[6] = make(RESUME, Operand::Value(5), Some(2)),
                "wrong intrinsic" => {
                    prefix[4] = make(
                        CALL_INTRINSIC_1,
                        Operand::Value(UnaryIntrinsic::LIST_TO_TUPLE.argument()),
                        Some(2),
                    );
                }
                "yield from" => prefix[5] = make(YIELD_VALUE, Operand::Value(1), Some(2)),
                "resume after await" => prefix[6] = make(RESUME, Operand::Value(3), Some(2)),
                "stored result" => prefix[7] = make(STORE_FAST, Operand::Value(20), Some(1)),
                "different location" => {
                    let Item::Instruction(resume) = &mut prefix[6] else {
                        unreachable!()
                    };
                    resume.location = SourceLocation::new(4, 4, 0, 4);
                }
                "unknown depth" => prefix[7] = make(POP_TOP, Operand::Value(0), None),
                "changed depth" => prefix[6] = make(RESUME, Operand::Value(1), Some(3)),
                "depth underflow" => {
                    prefix[4] = make(
                        CALL_INTRINSIC_1,
                        Operand::Value(UnaryIntrinsic::ASYNC_GEN_WRAP.argument()),
                        Some(0),
                    );
                    prefix[5] = make(YIELD_VALUE, Operand::Value(0), Some(0));
                    prefix[6] = make(RESUME, Operand::Value(1), Some(0));
                    prefix[7] = make(POP_TOP, Operand::Value(0), None);
                }
                "interior label" => prefix.insert(6, Item::Label(Label::from_u32(2))),
                "following label" => prefix.insert(8, Item::Label(Label::from_u32(2))),
                _ => unreachable!(),
            }
            let mut all = prefix.clone();
            all.push(Item::Label(exit));
            assert_eq!(
                ExceptionOwnerProvenance::default().uncompacted_prefix(
                    &prefix,
                    &all,
                    &OnceCell::new(),
                    1,
                    false,
                ),
                matches!(case, "intact" | "resume in handler"),
                "{case}",
            );
        }
    }

    #[test]
    fn expanded_comprehension_continuations_accept_only_a_sealed_setup() {
        use super::{ExceptionOwnerProvenance, OnceCell};
        use crate::assembler::{
            ExceptionPolicy, Instruction, Item, Label, Operand, SourceLocation,
        };
        use crate::target::opcodes::{
            BUILD_TUPLE, JUMP_FORWARD, LOAD_CONST, LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE,
            STORE_FAST, SWAP, TO_BOOL,
        };

        let entry = Label::from_u32(0);
        let exit = Label::from_u32(1);
        let start = Label::from_u32(2);
        let end = Label::from_u32(3);
        let setup = Label::from_u32(4);
        let alias = Label::from_u32(5);
        let make = |opcode, operand, line, depth| {
            Item::Instruction(Instruction::new(
                opcode,
                operand,
                SourceLocation::new(line, line, 0, 4),
                Some(depth),
            ))
        };
        let mut nop = Instruction::new(
            NOP,
            Operand::Value(0),
            SourceLocation::new(3, 3, 0, 4),
            Some(0),
        );
        nop.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
        for case in [
            "sealed",
            "fallthrough",
            "target",
            "block",
            "source",
            "duplicate label",
            "duplicate pair",
            "duplicate setup",
            "cross-role start",
            "cross-role end",
            "setup as range end",
            "start as setup",
            "moved nop",
            "extra alias",
            "terminator",
            "mismatched location",
        ] {
            let mut prefix = vec![
                Item::Label(entry),
                make(SWAP, Operand::Value(2), 2, 2),
                make(STORE_FAST, Operand::Value(0), 2, 1),
                make(LOAD_CONST, Operand::Value(0), 2, 2),
                make(BUILD_TUPLE, Operand::Value(2), 2, 1),
                make(STORE_FAST, Operand::Value(20), 2, 0),
                Item::Label(start),
                Item::Instruction(nop),
                Item::Label(end),
                Item::Label(setup),
                make(LOAD_FAST, Operand::Value(20), 4, 1),
                make(TO_BOOL, Operand::Value(0), 4, 1),
                make(POP_JUMP_IF_FALSE, Operand::Forward(exit), 4, 0),
                make(NOT_TAKEN, Operand::Value(0), 4, 0),
            ];
            let mut provenance = ExceptionOwnerProvenance {
                setups: vec![(setup, Some(nop.location))],
                setup_nop_ranges: vec![(start, end)],
                ..ExceptionOwnerProvenance::default()
            };
            let mut extra = None;
            match case {
                "sealed" => {}
                "fallthrough" => {
                    provenance.original_fallthrough_setups.insert(start);
                }
                "target" => extra = Some(make(JUMP_FORWARD, Operand::Forward(start), 5, 0)),
                "block" => {
                    provenance.block_boundaries.insert(setup);
                }
                "source" => {
                    provenance.source_boundaries.insert(end);
                }
                "duplicate label" => extra = Some(Item::Label(start)),
                "duplicate pair" => provenance.setup_nop_ranges.push((start, end)),
                "duplicate setup" => provenance.setups.push((setup, Some(nop.location))),
                "cross-role start" => provenance.setup_nop_ranges.push((alias, start)),
                "cross-role end" => provenance.setup_nop_ranges.push((end, alias)),
                "setup as range end" => provenance.setup_nop_ranges.push((alias, setup)),
                "start as setup" => provenance.setups.push((start, Some(nop.location))),
                "moved nop" => prefix.swap(6, 7),
                "extra alias" => prefix.insert(9, Item::Label(alias)),
                "terminator" => {
                    prefix.insert(6, make(JUMP_FORWARD, Operand::Forward(alias), 2, 0));
                }
                "mismatched location" => {
                    provenance.setups[0].1 = Some(SourceLocation::new(9, 9, 0, 4));
                }
                _ => unreachable!(),
            }
            let mut all = prefix.clone();
            all.push(Item::Label(exit));
            all.extend(extra);
            assert_eq!(
                provenance.uncompacted_prefix(&prefix, &all, &OnceCell::new(), 1, false),
                case == "sealed",
                "{case}"
            );
        }
    }

    #[test]
    fn uncompacted_marker_index_matches_full_scans_across_aliases_and_duplicate_origins() {
        use super::{ExceptionOwnerProvenance, UncompactedConditionalArray};
        use crate::assembler::{
            Instruction, Item, Label, LayoutAssembler, Operand, SourceLocation,
        };
        use crate::target::opcodes::{LOAD_CONST, NOT_TAKEN, UNPACK_SEQUENCE};

        let first = Label::from_u32(0);
        let second = Label::from_u32(1);
        let aliases = [Label::from_u32(2), Label::from_u32(3), Label::from_u32(4)];
        let instruction = |opcode, argument, line| {
            Item::Instruction(Instruction::new(
                opcode,
                Operand::Value(argument),
                SourceLocation::new(line, line, 0, 4),
                None,
            ))
        };
        for case in [
            "all",
            "single",
            "duplicate label",
            "duplicate instruction",
            "entered alias",
        ] {
            let mut items = vec![
                instruction(LOAD_CONST, 0, 1),
                instruction(LOAD_CONST, 1, 1),
                Item::Label(first),
                Item::Label(aliases[0]),
                instruction(LOAD_CONST, 2, 2),
                instruction(NOT_TAKEN, 0, 2),
                Item::Label(second),
                instruction(LOAD_CONST, 3, 3),
                Item::Label(aliases[1]),
                instruction(NOT_TAKEN, 0, 3),
                instruction(UNPACK_SEQUENCE, 0, 4),
                Item::Label(aliases[2]),
                instruction(NOT_TAKEN, 0, 4),
                instruction(LOAD_CONST, 4, 5),
            ];
            let mut origins = vec![
                vec![items[2], items[4], items[5]],
                vec![items[6], items[7], items[9]],
                vec![items[10], items[12]],
            ];
            let mut targets = rustc_hash::FxHashSet::default();
            let expected: &[_] = match case {
                "single" => {
                    origins.truncate(1);
                    &[3]
                }
                "duplicate label" => {
                    items.push(Item::Label(first));
                    &[5, 7]
                }
                "duplicate instruction" => {
                    items.push(items[10]);
                    &[3, 5]
                }
                "entered alias" => {
                    targets.insert(aliases[0]);
                    &[5, 7]
                }
                _ => &[3, 5, 7],
            };
            let provenance = ExceptionOwnerProvenance {
                uncompacted_conditional_arrays: origins
                    .into_iter()
                    .map(|prefix| UncompactedConditionalArray {
                        prefix,
                        comprehension_entry: None,
                        source_entry: None,
                        fallthrough: None,
                    })
                    .collect(),
                ..ExceptionOwnerProvenance::default()
            };
            let assembler = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_owner_provenance: Box::new(provenance),
                exception_regions: vec![],
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 5,
                has_next_location: false,
            };
            let unique = super::OnceCell::new();
            let markers = assembler.uncompacted_conditional_null_markers::<true>(&targets, &unique);
            assert_eq!(markers, expected.iter().copied().collect(), "{case}");
            assert_eq!(
                markers,
                assembler.uncompacted_conditional_null_markers::<false>(&targets, &unique),
                "{case}"
            );
            assert_eq!(unique.get().is_some(), case != "single", "{case}");
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
            "missing leading labels",
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
            let incoming = plain(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(1)), 1);
            let mut items = vec![
                Item::Instruction(incoming),
                Item::Instruction(plain(NOT_TAKEN, Operand::Value(0), 1)),
                Item::Instruction(plain(
                    JUMP_BACKWARD,
                    Operand::Backward(Label::from_u32(9)),
                    1,
                )),
                Item::Label(Label::from_u32(1)),
                Item::Label(Label::from_u32(2)),
                // Generator SETUP markers may have the default exception policy.
                Item::Instruction(plain(NOP, Operand::Value(0), 2)),
                Item::Label(Label::from_u32(3)),
                Item::Label(Label::from_u32(4)),
                Item::Instruction(plain(LOAD_FAST, Operand::Value(0), 3)),
                Item::Instruction(plain(LOAD_ATTR, Operand::Value(0), 3)),
                Item::Instruction(plain(TO_BOOL, Operand::Value(0), 3)),
                Item::Instruction(plain(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(Label::from_u32(5)),
                    3,
                )),
                Item::Instruction(plain(NOT_TAKEN, Operand::Value(0), 3)),
                Item::Label(Label::from_u32(5)),
                Item::Instruction(plain(RETURN_VALUE, Operand::Value(0), 4)),
            ];
            let mut provenance = ExceptionOwnerProvenance {
                original_primitive_condition_boundaries: vec![incoming],
                setups: vec![(Label::from_u32(4), Some(location(2)))],
                setup_nop_ranges: vec![(Label::from_u32(2), Label::from_u32(3))],
                ..ExceptionOwnerProvenance::default()
            };
            match case {
                "authored break" => {
                    let mut jump = plain(JUMP_FORWARD, Operand::Forward(Label::from_u32(1)), 1);
                    jump.fusion_policy.mark_authored_break();
                    items[0] = Item::Instruction(jump);
                    provenance.original_primitive_condition_boundaries.clear();
                }
                "missing leading labels" => {
                    items.retain(|item| !matches!(item, Item::Label(label) if matches!(label.as_u32(), 1 | 2)));
                    // A still-literal incoming condition cannot authenticate an unlabeled array.
                    assert!(!provenance.uncompacted_prefix(
                        &items[3..=10],
                        &items,
                        &std::cell::OnceCell::new(),
                        0,
                        false,
                    ));
                }
                "unknown origin" => provenance.original_primitive_condition_boundaries.clear(),
                "changed origin" => {
                    items[0] = Item::Instruction(plain(
                        POP_JUMP_IF_FALSE,
                        Operand::Forward(Label::from_u32(6)),
                        1,
                    ));
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
                    provenance.omitted_nop_counts.insert(Label::from_u32(4), 1);
                }
                "interior entry" => items.push(Item::Instruction(plain(
                    JUMP_FORWARD,
                    Operand::Forward(Label::from_u32(4)),
                    4,
                ))),
                _ => {}
            }
            provenance.capture_uncompacted_conditional_arrays(&items, false);
            let mut assembler = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_owner_provenance: Box::new(provenance),
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
                "range alias" => assembler.items.insert(12, Item::Label(Label::from_u32(10))),
                "changed policy" => {
                    let Item::Instruction(instruction) = &mut assembler.items[8] else {
                        unreachable!()
                    };
                    instruction.jump_policy.prevent_threading_target();
                }
                "duplicate final entry" => assembler.items.push(Item::Label(Label::from_u32(1))),
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
            let unique = super::OnceCell::new();
            let markers = assembler.uncompacted_conditional_null_markers::<true>(&targets, &unique);
            assert_eq!(markers, expected, "{case}");
            assert_eq!(
                markers,
                assembler.uncompacted_conditional_null_markers::<false>(&targets, &unique),
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
            let entry = Label::from_u32(0);
            let mut items = vec![
                // The folded while test belongs to an earlier original array.
                plain(NOP, Operand::Value(0), 1),
                Item::Label(entry),
                Item::Label(Label::from_u32(1)),
                plain(NOP, Operand::Value(0), 2),
                Item::Label(Label::from_u32(2)),
                Item::Label(Label::from_u32(3)),
                plain(LOAD_FAST, Operand::Value(0), 3),
                plain(LOAD_ATTR, Operand::Value(1), 3),
                plain(CALL, Operand::Value(0), 3),
                plain(POP_TOP, Operand::Value(0), 3),
                plain(LOAD_FAST, Operand::Value(0), 4),
                plain(LOAD_ATTR, Operand::Value(0), 4),
                plain(TO_BOOL, Operand::Value(0), 4),
                plain(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(4)), 4),
                plain(NOT_TAKEN, Operand::Value(0), 4),
            ];
            let mut provenance = ExceptionOwnerProvenance {
                block_boundaries: [entry].into_iter().collect(),
                source_boundaries: [entry].into_iter().collect(),
                setups: vec![(Label::from_u32(3), Some(location(2)))],
                setup_nop_ranges: vec![(Label::from_u32(1), Label::from_u32(2))],
                ..ExceptionOwnerProvenance::default()
            };
            match case {
                "block only" => provenance.source_boundaries.clear(),
                "source only" => provenance.block_boundaries.clear(),
                "adjacent boundary" => {
                    provenance.block_boundaries.insert(Label::from_u32(5));
                    provenance.source_boundaries.insert(Label::from_u32(5));
                    items.insert(1, Item::Label(Label::from_u32(5)));
                }
                "interior boundary" => {
                    provenance.block_boundaries.insert(Label::from_u32(3));
                    provenance.source_boundaries.insert(Label::from_u32(3));
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
            provenance.capture_uncompacted_conditional_arrays(&items, false);
            match case {
                "copied entry" => items[1] = Item::Label(Label::from_u32(6)),
                "retired source" => provenance.source_boundaries.clear(),
                "retired block" => provenance.block_boundaries.clear(),
                _ => {}
            }
            let assembler = LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_owner_provenance: Box::new(provenance),
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
            let targets = [Label::from_u32(4)].into_iter().collect();
            let unique = super::OnceCell::new();
            let markers = assembler.uncompacted_conditional_null_markers::<true>(&targets, &unique);
            assert_eq!(markers, expected, "{case}");
            assert_eq!(
                markers,
                assembler.uncompacted_conditional_null_markers::<false>(&targets, &unique),
                "{case}"
            );
        }
    }

    #[test]
    fn retained_pop_none_tail_requires_exact_protected_slot_history() {
        use super::{ExceptionOwnerProvenance, OwnerLabelPositions, RetainedNormalPopBlock};
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
            let entry = Label::from_u32(0);
            let body = Label::from_u32(1);
            let exit = Label::from_u32(2);
            let marker_start = Label::from_u32(3);
            let marker_end = Label::from_u32(4);
            let handler = Label::from_u32(5);
            let fallback = Label::from_u32(6);
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
                items.insert(4, Item::Label(Label::from_u32(7)));
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
                Label::from_u32(7)
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
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_regions: regions,
                exception_owner_provenance: Box::new(provenance),
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
                .collect::<rustc_hash::FxHashMap<_, _>>();
            let positions = OwnerLabelPositions::from_hashed(assembler.items.len(), &positions);
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
                let decision = decisions.get(4).expect("marker after region-end label");
                assert_eq!(decision.source_region(), Some(0), "{case}");
                assert!(
                    decision.has_owner()
                        && decision.is_definitive()
                        && decision.overrides_existing(),
                    "{case}"
                );
            } else if case == "excluded marker" {
                assert!(
                    !decisions
                        .get(4)
                        .is_some_and(super::ExceptionOwnerDecision::has_owner),
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
            (LOAD_CONST, Operand::Deferred(OperandPatchId::from_u32(0))),
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
            assembler.remap_constant_indices(&[Some(0), None, retained.then_some(1)]);
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
            BorrowFallthrough, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority,
            Instruction, Item, Label, LayoutAssembler, NormalizedExceptionOwner, Operand,
            SourceLocation,
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
                Operand::Forward(Label::from_u32(12)),
                guard_location,
                None,
            );
            condition.local_policy.add_hidden_exception_owner_slots(3);
            let mut marker = Instruction::new(NOT_TAKEN, Operand::Value(0), guard_location, None);
            marker.set_exception_owner(NormalizedExceptionOwner::Present);

            let mut items = vec![
                Item::Label(Label::from_u32(0)),
                Item::Instruction(plain(RETURN_GENERATOR)), // 0: inserted generator prologue.
                Item::Instruction(plain(POP_TOP)),
                Item::Label(Label::from_u32(1)), // Removed scope SETUP: compact position 2, old position 2.
                Item::Instruction(plain(RESUME)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Label(Label::from_u32(2)), // First manager SETUP: compact position 4, old position 5.
                Item::Instruction(plain(LOAD_CONST)),
                Item::Label(Label::from_u32(3)), // Second manager SETUP: compact position 5, old position 7.
                Item::Label(Label::from_u32(4)),
                Item::Instruction(outer), // 5: retained outer SETUP.
                Item::Label(Label::from_u32(5)),
                Item::Label(Label::from_u32(6)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Instruction(plain(YIELD_VALUE)),
                Item::Instruction(plain(RESUME)),
                Item::Instruction(plain(POP_TOP)),
                Item::Label(Label::from_u32(7)), // Erased outer normal POP_BLOCK, compact position 10.
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
                Item::Label(Label::from_u32(8)),
                Item::Instruction(inner), // Target source 10; control source 13.
                Item::Label(Label::from_u32(9)),
                Item::Label(Label::from_u32(10)),
                Item::Instruction(plain(LOAD_FAST)),
                Item::Instruction(condition),
                Item::Label(Label::from_u32(11)), // A bookkeeping marker alias is not a new original array.
                Item::Instruction(marker),        // Target m=13; control m=16; both select m-3.
                Item::Label(Label::from_u32(12)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Instruction(plain(RETURN_VALUE)),
                Item::Label(Label::from_u32(13)),
                Item::Label(Label::from_u32(14)),
                Item::Instruction(plain(LOAD_CONST)),
                Item::Instruction(plain(RETURN_VALUE)),
            ]);
            LayoutAssembler {
                python_version: crate::PythonVersion::Py314,
                legacy_uniform_jump_seed: false,
                items,
                exception_regions: vec![ExceptionRegion {
                    is_scope_fallback: true,
                    priority: ExceptionRegionPriority::OriginalRange(
                        Label::from_u32(1),
                        Label::from_u32(13),
                    ),
                    start: Label::from_u32(1),
                    end: Label::from_u32(13),
                    target: Label::from_u32(14),
                    depth: 0,
                    preserve_lasti: false,
                }],
                exception_owner_provenance: Box::new(ExceptionOwnerProvenance {
                    setups: vec![
                        (Label::from_u32(1), None),
                        (Label::from_u32(2), None),
                        (Label::from_u32(3), None),
                        (Label::from_u32(6), Some(outer_location)),
                        (Label::from_u32(10), Some(inner_location)),
                    ],
                    block_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    source_boundaries: [Label::from_u32(0)].into_iter().collect(),
                    synchronous_with_setups: [Label::from_u32(2), Label::from_u32(3)]
                        .into_iter()
                        .collect(),
                    setup_nop_boundaries: [
                        Label::from_u32(4),
                        Label::from_u32(5),
                        Label::from_u32(8),
                        Label::from_u32(9),
                    ]
                    .into_iter()
                    .collect(),
                    setup_nop_ranges: vec![
                        (Label::from_u32(4), Label::from_u32(5)),
                        (Label::from_u32(8), Label::from_u32(9)),
                    ],
                    removed_pop_block_boundaries: [Label::from_u32(7)].into_iter().collect(),
                    removed_pop_block_widths: [(Label::from_u32(7), 1)].into_iter().collect(),
                    ..ExceptionOwnerProvenance::default()
                }),
                preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                removed_max_depth: None,
                next_label: 15,
                has_next_location: false,
            }
        }

        fn recognizes(assembler: &LayoutAssembler) -> bool {
            let mut instructions = Vec::new();
            let mut positions =
                crate::assembler::exception_owners::OwnerLabelPositions::new(assembler.items.len());
            for item in &assembler.items {
                match item {
                    Item::Label(label) => {
                        positions.insert(*label, instructions.len());
                    }
                    Item::Instruction(instruction) => instructions.push(instruction),
                }
            }
            assembler.entry_retained_setup_null_marker(
                positions[&Label::from_u32(11)],
                &instructions,
                &positions,
                &[Label::from_u32(12)].into_iter().collect(),
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
        fn last_setup_compaction_accepts_only_unchanged_original_setup_owners() {
            for intervening_call in [false, true] {
                for policy in [
                    ExceptionPolicy::NoneUnchanged,
                    ExceptionPolicy::EXCLUDE_ALWAYS,
                    ExceptionPolicy::NonePresent,
                    ExceptionPolicy::NoneAbsent,
                    ExceptionPolicy::IfExtendedUnchanged,
                    ExceptionPolicy::AlwaysPresent,
                    ExceptionPolicy::AlwaysAbsent,
                    ExceptionPolicy::AlwaysOverridePresent,
                    ExceptionPolicy::AlwaysDynamic,
                ] {
                    let mut assembler = fixture(intervening_call);
                    for item in &mut assembler.items {
                        if let Item::Instruction(instruction) = item
                            && instruction.opcode == NOP
                        {
                            instruction.set_exception_policy(policy);
                        }
                    }
                    assert_eq!(
                        recognizes(&assembler),
                        matches!(
                            policy,
                            ExceptionPolicy::NoneUnchanged | ExceptionPolicy::AlwaysUnchanged
                        ),
                        "{intervening_call}: {policy:?}"
                    );
                }
            }
        }

        #[test]
        fn last_setup_compaction_rejects_unproven_records_and_array_boundaries() {
            let rejects = |name: &str, change: fn(&mut LayoutAssembler)| {
                let mut assembler = fixture(false);
                change(&mut assembler);
                assert!(!recognizes(&assembler), "{name}");
            };
            rejects("changed retained range", |assembler| {
                assembler.exception_owner_provenance.setup_nop_ranges[1].1 = Label::from_u32(11);
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
                    .insert(Label::from_u32(7));
            });
            rejects("unknown removed SETUP", |assembler| {
                assembler
                    .exception_owner_provenance
                    .synchronous_with_setups
                    .remove(&Label::from_u32(3));
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
                    .insert(Label::from_u32(7));
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

    #[test]
    fn fallthrough_marker_requires_direct_name_statement_tests() {
        use super::ExceptionOwnerProvenance;
        use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
        use crate::target::opcodes::{LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, TO_BOOL};

        let location = |line| SourceLocation::new(line, line, 0, 4);
        let plain = |opcode, operand, line| Instruction::new(opcode, operand, location(line), None);
        let predecessor = plain(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(9)), 1);
        let target = plain(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(10)), 4);
        let items = vec![
            Item::Instruction(predecessor),
            Item::Instruction(plain(NOT_TAKEN, Operand::Value(0), 1)),
            Item::Label(Label::from_u32(1)),
            Item::Label(Label::from_u32(2)),
            Item::Instruction(plain(NOP, Operand::Value(0), 2)),
            Item::Label(Label::from_u32(3)),
            Item::Label(Label::from_u32(4)),
            Item::Instruction(plain(NOP, Operand::Value(0), 3)),
            Item::Label(Label::from_u32(5)),
            Item::Label(Label::from_u32(6)),
            Item::Instruction(plain(LOAD_FAST, Operand::Value(0), 4)),
            Item::Instruction(plain(TO_BOOL, Operand::Value(0), 4)),
            Item::Instruction(target),
            Item::Instruction(plain(NOT_TAKEN, Operand::Value(0), 4)),
            Item::Label(Label::from_u32(9)),
            Item::Label(Label::from_u32(10)),
        ];
        for (while_witness, if_witness, duplicate_boundary) in [
            (true, true, false),
            (false, true, false),
            (true, false, false),
            (false, false, false),
            (true, true, true),
        ] {
            let mut provenance = ExceptionOwnerProvenance {
                original_primitive_condition_boundaries: vec![predecessor, target],
                setups: vec![
                    (Label::from_u32(3), Some(location(2))),
                    (Label::from_u32(5), Some(location(3))),
                ],
                setup_nop_ranges: vec![
                    (Label::from_u32(2), Label::from_u32(3)),
                    (Label::from_u32(4), Label::from_u32(5)),
                ],
                original_name_while_locations: while_witness
                    .then_some(location(1))
                    .into_iter()
                    .collect(),
                original_initial_name_if_locations: if_witness
                    .then_some(location(4))
                    .into_iter()
                    .collect(),
                ..ExceptionOwnerProvenance::default()
            };
            if duplicate_boundary {
                provenance
                    .original_primitive_condition_boundaries
                    .push(target);
            }
            provenance.capture_uncompacted_conditional_arrays(&items, true);
            assert_eq!(
                provenance.uncompacted_conditional_arrays.len(),
                usize::from(while_witness && if_witness && !duplicate_boundary),
                "while={while_witness}, if={if_witness}, duplicate={duplicate_boundary}"
            );
        }
    }

    #[test]
    fn fallthrough_setup_index_matches_original_pair_predicate() {
        use rustc_hash::FxHashSet;

        use super::{ExceptionOwnerProvenance, UncompactedPrefixIndexes};
        use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
        use crate::target::opcodes::{NOP, POP_TOP};

        let label = Label::from_u32;
        let location = SourceLocation::new(1, 1, 0, 4);
        let nop = Instruction::new(NOP, Operand::Value(0), location, None);
        let other = Instruction::new(POP_TOP, Operand::Value(0), location, None);
        for flags in 0..256 {
            let mut all = vec![Item::Label(label(1)), Item::Instruction(nop)];
            if flags & 1 != 0 {
                all.push(Item::Instruction(other));
            }
            if flags & 2 != 0 {
                all.push(Item::Label(label(3)));
            }
            all.push(Item::Label(label(2)));
            if flags & 4 != 0 {
                all.push(Item::Instruction(other));
            }
            if flags & 2 == 0 {
                all.push(Item::Label(label(3)));
            }
            if flags & 8 != 0 {
                all.push(Item::Label(label(1)));
            }
            if flags & 16 != 0 {
                all.push(Item::Label(label(2)));
            }
            all.push(Item::Label(label(4)));
            let mut labels = [label(1), label(2), label(3), label(4)]
                .into_iter()
                .collect::<FxHashSet<_>>();
            if flags & 32 != 0 {
                labels.remove(&label(1));
            }
            if flags & 64 != 0 {
                labels.remove(&label(2));
            }
            let setup_location = (flags & 128 == 0).then_some(location);
            let provenance = ExceptionOwnerProvenance {
                setup_nop_ranges: vec![(label(1), label(2)), (label(1), label(4))],
                setups: vec![(label(3), setup_location)],
                ..ExceptionOwnerProvenance::default()
            };
            let indexes = UncompactedPrefixIndexes::new(&all);
            for instruction in [nop, other] {
                let original = provenance.setup_nop_ranges.iter().any(|(start, end)| {
                    let (Some(&start_at), Some(&end_at)) =
                        (indexes.positions.get(start), indexes.positions.get(end))
                    else {
                        return false;
                    };
                    start_at < end_at
                        && indexes.has_unique_label(*start)
                        && indexes.has_unique_label(*end)
                        && labels.contains(start)
                        && labels.contains(end)
                        && all[start_at..end_at]
                            .iter()
                            .filter_map(|item| match item {
                                Item::Instruction(value) => Some(value),
                                Item::Label(_) => None,
                            })
                            .eq([&instruction])
                        && provenance.setups.iter().any(|(label, location)| {
                            *location == Some(instruction.location)
                                && indexes.positions.get(label).is_some_and(|&position| {
                                    position >= end_at
                                        && all[end_at..position]
                                            .iter()
                                            .all(|item| matches!(item, Item::Label(_)))
                                })
                        })
                });
                assert_eq!(
                    indexes.has_fallthrough_setup_pair(&provenance, &all, &labels, instruction),
                    original,
                    "flags={flags}, instruction={instruction:?}"
                );
            }
        }
    }

    #[test]
    fn fallthrough_marker_rejects_inserted_provenance_boundaries() {
        use super::{ExceptionOwnerProvenance, UncompactedConditionalArray};
        use crate::assembler::ir::{ExceptionRegion, ExceptionRegionPriority};
        use crate::assembler::{
            Instruction, Item, Label, LayoutAssembler, Operand, SourceLocation,
        };
        use crate::target::opcodes::{LOAD_FAST, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, TO_BOOL};

        let location = |line| SourceLocation::new(line, line, 0, 4);
        let plain = |opcode, operand, line| Instruction::new(opcode, operand, location(line), None);
        let condition = plain(POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(9)), 1);
        let marker = plain(NOT_TAKEN, Operand::Value(0), 1);
        let prefix = vec![
            Item::Label(Label::from_u32(1)),
            Item::Instruction(plain(NOP, Operand::Value(0), 2)),
            Item::Label(Label::from_u32(2)),
            Item::Instruction(plain(NOP, Operand::Value(0), 3)),
            Item::Instruction(plain(LOAD_FAST, Operand::Value(0), 4)),
            Item::Instruction(plain(TO_BOOL, Operand::Value(0), 4)),
            Item::Instruction(plain(
                POP_JUMP_IF_FALSE,
                Operand::Forward(Label::from_u32(10)),
                4,
            )),
            Item::Instruction(plain(NOT_TAKEN, Operand::Value(0), 4)),
        ];
        let alias = Label::from_u32(5);
        for position in ["predecessor", "prefix"] {
            for boundary in [
                "range",
                "unrelated setup and handler",
                "setup",
                "handler",
                "omitted count",
                "omitted location",
                "removed pop",
                "fusion",
                "copied block",
                "copied inline",
                "copied internal",
                "copied successor",
            ] {
                let mut items = vec![Item::Instruction(condition), Item::Instruction(marker)];
                items.extend(prefix.iter().copied());
                let insert_at = if position == "predecessor" { 1 } else { 4 };
                items.insert(insert_at, Item::Label(alias));
                let mut provenance = ExceptionOwnerProvenance {
                    uncompacted_conditional_arrays: vec![UncompactedConditionalArray {
                        prefix: prefix.clone(),
                        comprehension_entry: None,
                        source_entry: None,
                        fallthrough: Some([condition, marker]),
                    }],
                    ..ExceptionOwnerProvenance::default()
                };
                match boundary {
                    "setup" => {
                        provenance
                            .setups
                            .extend([(alias, None), (alias, Some(location(1)))]);
                    }
                    "unrelated setup and handler" => {
                        provenance.setups.push((Label::from_u32(8), None));
                    }
                    "omitted count" => {
                        provenance.omitted_nop_counts.insert(alias, 1);
                    }
                    "omitted location" => {
                        provenance.omitted_nop_locations.insert(alias, location(1));
                    }
                    "removed pop" => {
                        provenance.removed_pop_block_boundaries.insert(alias);
                    }
                    "fusion" => {
                        provenance.fusion_boundaries.insert(alias);
                    }
                    "copied block" => {
                        provenance.normal_try_copy_blocks.insert(alias);
                    }
                    "copied inline" => {
                        provenance.normal_try_copy_inline_labels.insert(alias);
                    }
                    "copied internal" => {
                        provenance.normal_try_copy_internal_labels.push(alias);
                    }
                    "copied successor" => {
                        provenance.normal_try_copy_jump_nop_successors.insert(alias);
                    }
                    _ => {}
                }
                let exception_regions =
                    if boundary == "handler" || boundary == "unrelated setup and handler" {
                        let target = if boundary == "handler" {
                            alias
                        } else {
                            Label::from_u32(8)
                        };
                        vec![ExceptionRegion {
                            is_scope_fallback: false,
                            priority: ExceptionRegionPriority::OriginalRange(
                                Label::from_u32(1),
                                Label::from_u32(2),
                            ),
                            start: Label::from_u32(1),
                            end: Label::from_u32(2),
                            target,
                            depth: 0,
                            preserve_lasti: false,
                        }]
                    } else {
                        vec![]
                    };
                let assembler = LayoutAssembler {
                    python_version: crate::PythonVersion::Py314,
                    legacy_uniform_jump_seed: false,
                    items,
                    exception_owner_provenance: Box::new(provenance),
                    exception_regions,
                    preserved_block_boundaries: rustc_hash::FxHashSet::default(),
                    removed_max_depth: None,
                    next_label: 11,
                    has_next_location: false,
                };
                let targets = rustc_hash::FxHashSet::default();
                let unique = super::OnceCell::new();
                let markers =
                    assembler.uncompacted_conditional_null_markers::<true>(&targets, &unique);
                let expected = if boundary == "range" || boundary == "unrelated setup and handler" {
                    [7].into_iter().collect()
                } else {
                    rustc_hash::FxHashSet::default()
                };
                assert_eq!(markers, expected, "{position}: {boundary}");
                assert_eq!(
                    markers,
                    assembler.uncompacted_conditional_null_markers::<false>(&targets, &unique)
                );
            }
        }
    }
}
