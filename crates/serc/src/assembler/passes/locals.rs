use std::cmp::Reverse;
use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::super::ir::{
    AuthoredFinallyBreakEntries, AuthoredFinallyBreakEntry, AuthoredFinallyBreakOrigins,
    BorrowCleanupCall, ExceptionRegionPriority,
};
use super::super::{
    BorrowFallthrough, BorrowLoopExitPolicy, CHECK_EXC_MATCH, COMPARE_OP, COPY, DELETE_DEREF,
    DELETE_FAST, DICT_MERGE, DICT_UPDATE, END_SEND, ExceptionRegion, FOR_ITER, FORMAT_SIMPLE,
    GET_ANEXT, GET_LEN, GET_YIELD_FROM_ITER, IMPORT_FROM, Instruction, Item,
    JUMP_BACKWARD_NO_INTERRUPT, LIST_APPEND, LIST_EXTEND, LOAD_ATTR, LOAD_COMMON_CONSTANT,
    LOAD_CONST, LOAD_FAST, LOAD_FAST_AND_CLEAR, LOAD_FAST_BORROW,
    LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_CHECK, LOAD_FAST_LOAD_FAST, LOAD_SMALL_INT,
    LOAD_SPECIAL, LOAD_SUPER_ATTR, Label, MAP_ADD, MATCH_KEYS, MATCH_MAPPING, MATCH_SEQUENCE, NOP,
    NOT_TAKEN, Operand, POP_EXCEPT, POP_TOP, PUSH_EXC_INFO, RERAISE, RETURN_VALUE, SEND, SET_ADD,
    SET_FUNCTION_ATTRIBUTE, SET_UPDATE, STORE_DEREF, STORE_FAST, STORE_FAST_LOAD_FAST,
    STORE_FAST_STORE_FAST, STORE_GLOBAL, SWAP, WITH_EXCEPT_START, block_has_fallthrough,
    block_jump_target, ends_scope, opcode_stack_counts, opcode_stack_effect,
};
use crate::assembler::OptimizationPipeline;
use crate::assembler::block_graph::ItemMut;
use crate::assembler::ir::{ThreadedBorrowEntries, ThreadedBorrowSnapshot};
use crate::target::{
    is_conditional_jump, is_unconditional_jump,
    opcodes::{
        CALL, CALL_KW, END_FOR, GET_AWAITABLE, GET_ITER, LOAD_GLOBAL, POP_JUMP_IF_FALSE,
        POP_JUMP_IF_NONE, STORE_ATTR, TO_BOOL,
    },
};

/// Original block entries retained for CPython's final borrowing traversal.
#[derive(Clone, Copy)]
pub(in crate::assembler) struct BorrowEntryLabels<'a> {
    pub(super) unreachable: &'a FxHashSet<Label>,
    pub(super) loop_exits: &'a FxHashMap<Label, BorrowLoopExitPolicy>,
    pub(super) finally_entries: &'a FxHashMap<Label, Option<BorrowCleanupCall>>,
    pub(super) finally_jump_sources: &'a FxHashMap<Label, bool>,
    pub(super) jump_entries: &'a FxHashSet<Label>,
    pub(super) original_boundaries: &'a FxHashSet<Label>,
    pub(super) pre_copy_jump_entries: &'a FxHashMap<Label, Label>,
    pub(super) authored_finally_break_entries: &'a AuthoredFinallyBreakEntries,
}

/// An original direct source and its nonempty normal-finally destination.
pub(super) struct RetainedFinallyOrigin {
    source_labels: Vec<Label>,
    // Empty for a retained jump NOP; otherwise this complete prefix must survive
    // unchanged after the ordinary jump disappears.
    source_prefix: Vec<Instruction>,
    // A recorded try-range end can split the physical graph without ending this array.
    source_exit_alias: Option<Label>,
    // A conditional may enter the retained jump after an earlier empty original array.
    conditional_entry: Option<(Label, Instruction)>,
    // The setup-only source certificate also requires this original populated join.
    setup_join: Option<(Label, Instruction)>,
    target_labels: Vec<Label>,
    target: Label,
    jump_location: crate::assembler::SourceLocation,
    first_live: Instruction,
}

/// A normal-finally `POP_BLOCK` followed by one original nested `SETUP_FINALLY`.
pub(super) struct SetupFinallyOrigin {
    entry: Label,
    items: Vec<Item>,
    final_prefix: Vec<Item>,
    setup_offset: usize,
    setup_can_disappear: bool,
    aliases: [Label; 2],
    previous_entries: Vec<(Label, Option<BorrowCleanupCall>)>,
    previous_alias: bool,
    // The emitter's normal try exit and its complete populated source array.
    normal_source: Option<(Label, Vec<Item>)>,
}

impl OptimizationPipeline {
    /// Joins only the emitter's recorded setup span, never an arbitrary empty block.
    fn capture_setup_finally_origins(&mut self) -> Vec<SetupFinallyOrigin> {
        if self.borrow_finally_entries.is_empty() {
            return Vec::new();
        }
        let mut targets = self
            .exception_regions
            .iter()
            .map(|region| region.target)
            .collect::<FxHashSet<_>>();
        for item in self.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
            {
                targets.insert(label);
            }
        }
        let blocks = self
            .graph
            .iter()
            .map(|(_, block)| block.items())
            .collect::<Vec<_>>();
        let mut origins = Vec::new();
        for (block_index, window) in blocks.windows(3).enumerate() {
            let [head, setup, body] = window else {
                unreachable!()
            };
            let [
                Item::Label(original),
                Item::Label(entry),
                Item::Instruction(pop),
            ] = *head
            else {
                continue;
            };
            let [Item::Label(start), Item::Instruction(nop)] = *setup else {
                continue;
            };
            let [
                Item::Label(end),
                Item::Label(protected),
                Item::Instruction(first),
                ..,
            ] = *body
            else {
                continue;
            };
            let provenance = &self.exception_owner_provenance;
            // An earlier call keeps a primitive conditional prefix nonempty even
            // when the enclosing call continues in successor arrays. Keep the
            // original branch and complete prefix for the same late identity check.
            let conditional_call_prefix = matches!(body.get(body.len().saturating_sub(3)..),
                Some([Item::Instruction(load), Item::Instruction(test), Item::Instruction(jump)])
                    if load.opcode == LOAD_FAST
                        && matches!(load.operand(), Operand::Value(_))
                        && test.opcode == TO_BOOL
                        && test.operand() == Operand::Value(0)
                        && jump.opcode == POP_JUMP_IF_FALSE
                        && matches!(jump.operand(), Operand::Forward(_))
                        && load.location.line >= 0
                        && load.location == test.location
                        && test.location == jump.location)
                && body.iter().any(|item| {
                    matches!(item, Item::Instruction(call)
                        if call.opcode == CALL && matches!(call.operand(), Operand::Value(_)))
                })
                && body[2..].iter().all(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if !instruction.jump_policy.borrow_empty_block()
                            && instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Reachable)
                });
            // A context-manager call and its enter call keep this nested setup's
            // original array nonempty. Authenticate the complete synchronous entry
            // sequence and its recorded protected boundary before extending the span.
            let manager_call_prefix = matches!(body.get(body.len().saturating_sub(7)..),
                Some([
                    Item::Instruction(context_call),
                    Item::Instruction(copy),
                    Item::Instruction(exit),
                    Item::Instruction(swap_two),
                    Item::Instruction(swap_three),
                    Item::Instruction(enter),
                    Item::Instruction(call),
                ])
                    if context_call.opcode == CALL
                        && matches!(context_call.operand(), Operand::Value(_))
                        && copy.opcode == COPY && copy.operand() == Operand::Value(1)
                        && exit.opcode == LOAD_SPECIAL && exit.operand() == Operand::Value(1)
                        && swap_two.opcode == SWAP && swap_two.operand() == Operand::Value(2)
                        && swap_three.opcode == SWAP && swap_three.operand() == Operand::Value(3)
                        && enter.opcode == LOAD_SPECIAL && enter.operand() == Operand::Value(0)
                        && call.opcode == CALL && call.operand() == Operand::Value(0)
                        && context_call.location.line >= 0
                        && [copy, exit, swap_two, swap_three, enter, call]
                            .iter().all(|instruction| instruction.location == context_call.location))
                && matches!(blocks.get(block_index + 3),
                    Some([Item::Label(manager), ..])
                        if provenance.synchronous_with_setups.contains(manager))
                && body[2..].iter().all(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if matches!(instruction.operand(), Operand::Value(_))
                            && !instruction.jump_policy.borrow_empty_block()
                            && instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Reachable)
                });
            if self.borrow_finally_entries.get(entry) != Some(&None)
                || self.borrow_loop_exit_blocks.contains_key(entry)
                || !self.borrow_jump_entry_labels.contains(entry)
                || !self.borrow_unreachable_blocks.contains(entry)
                || !self.preserved_block_boundaries.contains(entry)
                || !provenance.block_boundaries.contains(original)
                || self.preserved_block_boundaries.contains(original)
                || self.borrow_unreachable_blocks.contains(original)
                || !provenance.setup_nop_ranges.contains(&(*start, *end))
                || !provenance.setups.contains(&(*protected, Some(nop.location)))
                || [entry, start, end, protected].iter().any(|label| {
                    targets.contains(label) || provenance.block_boundaries.contains(label)
                })
                || [start, end, protected].iter().any(|label| {
                    self.preserved_block_boundaries.contains(label)
                        || self.borrow_unreachable_blocks.contains(label)
                })
                || pop.opcode != NOP
                || !pop.fusion_policy.is_converted_pop_block()
                || nop.opcode != NOP
                || nop.operand() != Operand::Value(0)
                || nop.fusion_policy.is_converted_pop_block()
                || nop.location.line < 0
                // A discarded or assigned call leaves this protected array nonempty.
                // An awaited call does too: its arguments are consumed before the
                // following SEND array, while borrowing still checks values kept live.
                // Returns and unwind markers need their separate exit-copy proof.
                || !(matches!(body.get(body.len().saturating_sub(2)..),
                    Some([Item::Instruction(call), Item::Instruction(consumer)])
                        if (call.opcode == CALL && consumer.opcode == POP_TOP)
                            || (matches!(call.opcode, CALL | CALL_KW)
                                && consumer.opcode == STORE_FAST))
                    || matches!(body.get(body.len().saturating_sub(3)..),
                        Some([Item::Instruction(call), Item::Instruction(awaitable), Item::Instruction(value)])
                            if matches!(call.opcode, CALL | CALL_KW)
                                && matches!(call.operand(), Operand::Value(_))
                                && awaitable.opcode == GET_AWAITABLE
                                && awaitable.operand() == Operand::Value(0)
                                && value.opcode == LOAD_CONST
                                && matches!(value.operand(), Operand::Value(_))
                                && value.location == awaitable.location
                                && matches!((blocks.get(block_index + 3), blocks.get(block_index + 4)),
                                    (Some([Item::Label(_), Item::Instruction(send)]),
                                     Some([Item::Label(yielded), Item::Instruction(yield_value)]))
                                        if send.opcode == SEND
                                            && matches!(send.operand(), Operand::Forward(_))
                                            && send.location == awaitable.location
                                            && yield_value.opcode == crate::target::opcodes::YIELD_VALUE
                                            && yield_value.operand() == Operand::Value(1)
                                            && provenance.ordinary_await_cleanups.iter().filter(|region| {
                                                region.start == *yielded
                                                    && !region.is_scope_fallback
                                                    && !region.preserve_lasti
                                            }).count() == 1))
                    || conditional_call_prefix
                    || manager_call_prefix)
                || [pop, nop, first].iter().any(|instruction| {
                    instruction.jump_policy.borrow_empty_block()
                        || instruction.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Reachable
                })
            {
                continue;
            }
            let aliases = [*original, *entry];
            let items = head
                .iter()
                .chain(setup.iter())
                .chain(body.iter())
                .copied()
                .collect();
            let mut final_prefix = head[..2].to_vec();
            final_prefix.extend_from_slice(setup);
            final_prefix.extend_from_slice(body);
            let normal_source = if manager_call_prefix {
                (|| {
                    // Cold handlers still separate the normal source from its target
                    // here. Follow the emitter's exact edge before requiring adjacency
                    // after relocation and redundant-jump removal.
                    let mut exits = provenance
                        .compound_try_owner_exits
                        .iter()
                        .filter(|&&(_, target)| target == *original);
                    let &(source_exit, _) = exits.next()?;
                    if exits.next().is_some() {
                        return None;
                    }
                    let source_exit_index = blocks.iter().position(|block| {
                        matches!(block.first(), Some(Item::Label(label)) if *label == source_exit)
                    })?;
                    let source_blocks =
                        blocks.get(source_exit_index.checked_sub(1)?..=source_exit_index)?;
                    let [source, exit] = source_blocks else {
                        return None;
                    };
                    let [Item::Label(exit), Item::Instruction(jump)] = *exit else {
                        return None;
                    };
                    if jump.opcode != crate::target::opcodes::JUMP_FORWARD
                        || jump.operand() != Operand::Forward(*original)
                        || jump.fusion_policy.is_authored_break()
                        || jump.jump_policy.borrow_empty_block()
                        || jump.jump_policy.retained_nop()
                            != crate::assembler::ir::RetainedJumpNop::None
                        || jump.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::Reachable
                        || self.borrow_finally_jump_sources.get(exit) != Some(&true)
                        || provenance
                            .compound_try_owner_exits
                            .iter()
                            .filter(|&&(start, target)| start == *exit && target == *original)
                            .count()
                            != 1
                        || targets.contains(exit)
                        || provenance.block_boundaries.contains(exit)
                        || self.preserved_block_boundaries.contains(exit)
                        || self.borrow_unreachable_blocks.contains(exit)
                    {
                        return None;
                    }
                    let labels = source
                        .iter()
                        .take_while(|item| matches!(item, Item::Label(_)))
                        .count();
                    if labels == 0 {
                        return None;
                    }
                    let mut prefix = source[..labels].to_vec();
                    for item in &source[labels..] {
                        let Item::Instruction(instruction) = item else {
                            return None;
                        };
                        // These emitter slots disappear before borrowing; a real NOP
                        // or any independent control-flow marker still rejects replay.
                        if instruction.opcode == NOP
                            && instruction.fusion_policy.is_converted_pop_block()
                            && instruction.exception_policy()
                                == crate::assembler::ExceptionPolicy::EXCLUDE_ALWAYS
                            && instruction.jump_policy.retained_nop()
                                == crate::assembler::ir::RetainedJumpNop::None
                            && !instruction.jump_policy.preserves_explicit_nop()
                            && !instruction.jump_policy.borrow_empty_block()
                            && !instruction.no_location_policy.preserves_location()
                            && instruction.no_location_policy.borrow_fallthrough()
                                == BorrowFallthrough::Reachable
                        {
                            continue;
                        }
                        if matches!(instruction.opcode, NOP | NOT_TAKEN)
                            || !matches!(instruction.operand(), Operand::Value(_))
                            || ends_scope(instruction.opcode)
                            || instruction.jump_policy.borrow_empty_block()
                            || instruction.no_location_policy.borrow_fallthrough()
                                != BorrowFallthrough::Reachable
                        {
                            return None;
                        }
                        prefix.push(*item);
                    }
                    if prefix.len() == labels {
                        return None;
                    }
                    prefix.push(Item::Label(*exit));
                    Some((*exit, prefix))
                })()
            } else {
                None
            };
            origins.push(SetupFinallyOrigin {
                entry: *entry,
                aliases,
                previous_entries: aliases
                    .iter()
                    .filter_map(|label| {
                        self.borrow_finally_entries
                            .get(label)
                            .map(|entry| (*label, *entry))
                    })
                    .collect(),
                items,
                final_prefix,
                setup_offset: 3,
                setup_can_disappear: nop.location.line == first.location.line,
                previous_alias: self.borrow_block_aliases.contains(entry),
                normal_source,
            });
        }
        for origin in &origins {
            self.borrow_block_aliases.insert(origin.entry);
        }
        origins
    }

    /// Revokes setup aliases if cleanup moved, was copied, or lost its exact entry span.
    pub(super) fn retain_setup_finally_origins(
        items: &[Item],
        origins: &[SetupFinallyOrigin],
        entries: &mut FxHashMap<Label, Option<BorrowCleanupCall>>,
        aliases: &mut FxHashSet<Label>,
    ) -> FxHashMap<Label, Label> {
        let mut normal_entries = FxHashMap::default();
        for origin in origins {
            let matches = |prefix: &[Item], checked_source_len: usize| {
                items.windows(prefix.len()).any(|actual| {
                    actual.iter().zip(prefix).enumerate().all(
                        |(offset, (actual, expected))| match (actual, expected) {
                            (Item::Label(actual), Item::Label(expected)) => actual == expected,
                            (Item::Instruction(actual), Item::Instruction(expected)) => {
                                // Definite-assignment analysis may add a check to this
                                // same source slot. Target replay stays opcode-literal.
                                (actual.opcode == expected.opcode
                                    || (offset < checked_source_len
                                        && expected.opcode == LOAD_FAST
                                        && actual.opcode == LOAD_FAST_CHECK))
                                    && actual.operand() == expected.operand()
                                    && actual.location == expected.location
                                    && !actual.jump_policy.borrow_empty_block()
                                    && actual.no_location_policy.borrow_fallthrough()
                                        == BorrowFallthrough::Reachable
                            }
                            _ => false,
                        },
                    )
                })
            };
            let mut erased = origin.final_prefix.clone();
            erased.remove(origin.setup_offset);
            // Pseudo conversion and NOP removal precede borrowing. A surviving
            // explicit jump can enter the same-line body after its setup disappears;
            // the existing unreachable marker still forbids ordinary fallthrough.
            let retained = entries.get(&origin.entry).is_some_and(Option::is_some)
                && (matches(&origin.final_prefix, 0)
                    || (origin.setup_can_disappear && matches(&erased, 0)));
            if retained && let Some((source, prefix)) = &origin.normal_source {
                let target = if matches(&origin.final_prefix, 0) {
                    &origin.final_prefix
                } else {
                    &erased
                };
                let original_edge = prefix.iter().chain(target).copied().collect::<Vec<_>>();
                // Only the original jump may disappear between these unchanged
                // populated arrays. Copies, moved sources and extra labels revoke it.
                if matches(&original_edge, prefix.len()) {
                    normal_entries.insert(*source, origin.aliases[0]);
                }
            }
            if !retained {
                for label in origin.aliases {
                    entries.remove(&label);
                }
                entries.extend(origin.previous_entries.iter().copied());
                if !origin.previous_alias {
                    aliases.remove(&origin.entry);
                }
            }
        }
        normal_entries
    }

    /// Certifies a populated ordinary-try body before its synthetic range-end split.
    fn ordinary_try_borrow_source(&self, items: &[Item], end: Label) -> Option<Label> {
        let labels = items
            .iter()
            .take_while(|item| matches!(item, Item::Label(_)))
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        let mut setups = labels.iter().filter_map(|label| {
            self.exception_owner_provenance
                .ordinary_try_setups
                .get(label)
                .map(|handler| (*label, *handler))
        });
        let (source, handler) = setups.next()?;
        let mut regions = self.exception_regions.iter().filter(|region| {
            region.start == source && region.end == end && region.target == handler
        });
        let region = regions.next()?;
        if setups.next().is_some()
            || regions.next().is_some()
            || region.is_scope_fallback
            || region.preserve_lasti
            || labels.last() != Some(&source)
            || self.exception_owner_provenance.setups.iter()
                .filter(|(label, _)| *label == source).count() != 1
            || labels.iter().any(|label| {
                self.exception_owner_provenance.block_boundaries.contains(label)
                    || self.exception_owner_provenance.source_boundaries.contains(label)
                    || self.preserved_block_boundaries.contains(label)
                    || self.borrow_unreachable_blocks.contains(label)
                    || self.borrow_jump_entry_labels.contains(label)
                    || self.borrow_finally_entries.contains_key(label)
                    || self.exception_regions.iter().any(|region| region.target == *label)
                    || self.graph.iter_items().filter(|item| **item == Item::Label(*label)).count() != 1
                    || self.graph.iter_items().any(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                                if target == *label))
                    })
            })
        {
            return None;
        }
        // The caller authenticates the normal exit and snapshots every live source
        // instruction. A setup label alone never admits an empty source array.
        Some(source)
    }

    /// Records exact original arrays before jump copying can replace either endpoint.
    pub(super) fn capture_retained_finally_origins(&self) -> Vec<RetainedFinallyOrigin> {
        if self.borrow_finally_entries.is_empty() {
            return Vec::new();
        }
        let blocks = self
            .graph
            .iter()
            .map(|(_, block)| block.items())
            .collect::<Vec<_>>();
        blocks
            .iter()
            .enumerate()
            .filter_map(|(block_index, contents)| {
                let (Item::Instruction(jump), leading) = contents.split_last()? else {
                    return None;
                };
                if jump.opcode != crate::target::opcodes::JUMP_FORWARD
                    || jump.jump_policy.borrow_empty_block()
                    || jump.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Reachable
                {
                    return None;
                }
                let setup_source = if let [Item::Label(alias)] = leading
                    && let Operand::Forward(target) = jump.operand()
                    && self.exception_owner_provenance.compound_try_owner_exits
                        .iter().filter(|origin| **origin == (*alias, target)).count() == 1
                    && let Some(source_items) =
                        block_index.checked_sub(1).and_then(|index| blocks.get(index)).copied()
                {
                    self.ordinary_try_borrow_source(source_items, *alias)
                } else {
                    None
                };
                // A located pass can transfer its line to the normal jump and
                // survive as that jump's DirectTarget NOP. Its setup and range-end
                // aliases then share one original source, with no live prefix.
                let direct_setup_source = if jump.jump_policy.retained_nop()
                    == crate::assembler::ir::RetainedJumpNop::DirectTarget
                    && jump.location.line >= 0
                    && leading.iter().all(|item| matches!(item, Item::Label(_)))
                    && let Some(Item::Label(alias)) = leading.last()
                    && let Operand::Forward(target) = jump.operand()
                    && self.exception_owner_provenance.compound_try_owner_exits
                        .iter().filter(|origin| **origin == (*alias, target)).count() == 1
                    && !self.exception_owner_provenance.block_boundaries.contains(alias)
                    && !self.exception_owner_provenance.source_boundaries.contains(alias)
                    && !self.preserved_block_boundaries.contains(alias)
                    && !self.borrow_unreachable_blocks.contains(alias)
                    && !self.borrow_jump_entry_labels.contains(alias)
                    && !self.borrow_finally_entries.contains_key(alias)
                    && !self.exception_regions.iter().any(|region| region.target == *alias)
                    && !self.graph.iter_items().any(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                                if target == *alias))
                    })
                {
                    self.ordinary_try_borrow_source(&leading[..leading.len() - 1], *alias)
                } else {
                    None
                };
                // A try-range end can split a populated original source array.
                // Its leading exception aliases do not create another CPython block.
                // Capture the complete span; the checks below require exactly one
                // original boundary and a nonempty, straight-line surviving prefix.
                let source_exit_alias = if let [Item::Label(alias)] = leading
                    && let Operand::Forward(target) = jump.operand()
                    && self.exception_owner_provenance.compound_try_owner_exits
                        .iter().filter(|origin| **origin == (*alias, target)).count() == 1
                    && !self.exception_owner_provenance.block_boundaries.contains(alias)
                    && !self.exception_owner_provenance.source_boundaries.contains(alias)
                    && !self.preserved_block_boundaries.contains(alias)
                    && !self.borrow_unreachable_blocks.contains(alias)
                    && !self.borrow_jump_entry_labels.contains(alias)
                    && !self.borrow_finally_entries.contains_key(alias)
                    && !self.exception_regions.iter().any(|region| region.target == *alias)
                    && !self.graph.iter_items().any(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                                if target == *alias))
                    })
                    && let Some(source_items) =
                        block_index.checked_sub(1).and_then(|index| blocks.get(index)).copied()
                    && (setup_source.is_some()
                        || source_items.iter().take_while(|item| matches!(item, Item::Label(_))).any(|item| {
                            matches!(item, Item::Label(source)
                                if self.exception_owner_provenance.source_boundaries.contains(source)
                                    && self.exception_owner_provenance.block_boundaries.contains(source))
                        }))
                    && source_items.iter().any(|item| matches!(item, Item::Instruction(_)))
                {
                    Some(*alias)
                } else {
                    None
                };
                let setup_source = setup_source
                    .filter(|_| source_exit_alias.is_some())
                    .or(direct_setup_source);
                let leading = if source_exit_alias.is_some() {
                    blocks[block_index - 1]
                } else {
                    leading
                };
                let source_label_count = leading
                    .iter()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .count();
                let source_labels = leading[..source_label_count]
                    .iter()
                    .map(|item| match item {
                        Item::Label(label) => *label,
                        Item::Instruction(_) => unreachable!(),
                    })
                    .collect::<Vec<_>>();
                let mut source_prefix = Vec::new();
                for item in &leading[source_label_count..] {
                    let Item::Instruction(instruction) = item else {
                        return None;
                    };
                    // CPython removes these recorded POP_BLOCK slots before borrowing.
                    // Their disappearance does not empty a source whose complete live
                    // prefix survives. Arbitrary or explicitly retained NOPs are excluded.
                    if instruction.opcode == NOP
                        && instruction.operand() == Operand::Value(0)
                        && instruction.fusion_policy.is_converted_pop_block()
                        && instruction.exception_policy()
                            == crate::assembler::ExceptionPolicy::EXCLUDE_ALWAYS
                        && instruction.jump_policy.retained_nop()
                            == crate::assembler::ir::RetainedJumpNop::None
                        && !instruction.jump_policy.preserves_explicit_nop()
                        && !instruction.jump_policy.borrow_empty_block()
                        && !instruction.no_location_policy.preserves_location()
                        && instruction.no_location_policy.borrow_fallthrough()
                            == BorrowFallthrough::Reachable
                    {
                        continue;
                    }
                    // A sole authored, located NOP can itself survive as the whole
                    // ordinary-try body. Exact replay must keep that instruction;
                    // removed POP_BLOCKs and label-only sources remain excluded.
                    let retained_source_nop = setup_source.is_some()
                        && source_exit_alias.is_some()
                        && leading.len() == source_label_count + 1
                        && instruction.opcode == NOP
                        && instruction.operand() == Operand::Value(0)
                        && instruction.location.line >= 0
                        && !instruction.fusion_policy.is_converted_pop_block()
                        && instruction.pre_fold_extra_instructions() == 0
                        && instruction.local_policy.hidden_exception_owner_slots() == 0
                        && instruction.jump_policy.retained_nop()
                            == crate::assembler::ir::RetainedJumpNop::None;
                    if (matches!(instruction.opcode, NOP | NOT_TAKEN) && !retained_source_nop)
                        || !matches!(instruction.operand(), Operand::Value(_))
                        || ends_scope(instruction.opcode)
                        || instruction.jump_policy.borrow_empty_block()
                        || instruction.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::Reachable
                    {
                        return None;
                    }
                    source_prefix.push(*instruction);
                }
                // A source containing only removed POP_BLOCKs is still an empty array.
                if source_label_count != leading.len() && source_prefix.is_empty() {
                    return None;
                }
                if source_prefix.is_empty() {
                    if jump.jump_policy.retained_nop()
                        != crate::assembler::ir::RetainedJumpNop::DirectTarget
                        || jump.location.line < 0
                    {
                        return None;
                    }
                } else if jump.fusion_policy.is_authored_break()
                    || !jump.jump_policy.allows_borrow_threading_target()
                    || jump.jump_policy.retained_nop()
                        != crate::assembler::ir::RetainedJumpNop::None
                {
                    return None;
                }
                let Operand::Forward(target) = jump.operand() else {
                    return None;
                };
                let target_block = self.graph.label_block(target)?;
                let target_items = self.graph[target_block].items();
                let setup_join = if setup_source.is_some() {
                    let mut originals = self.exception_owner_provenance
                        .original_normal_finally_arrays.iter().filter(|origin| origin.entry == target);
                    let original = originals.next()?;
                    if originals.next().is_some() || !original.incoming.contains(jump) {
                        return None;
                    }
                    let (join, first) = original.borrow_join?;
                    if !target_items.iter().any(|item| {
                        matches!(item, Item::Instruction(condition)
                            if is_conditional_jump(condition.opcode)
                                && condition.operand() == Operand::Forward(join))
                    }) || !self.original_borrow_join_survives(join, first) {
                        return None;
                    }
                    Some((join, first))
                } else {
                    None
                };
                let first_instruction = target_items
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(_)))?;
                let target_labels = target_items[..first_instruction]
                    .iter()
                    .filter_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    })
                    .collect::<Vec<_>>();
                // A producer-certified populated else join may itself be a preserved
                // source boundary. We start in that array; we never cross an empty
                // preserved boundary. Exact prefix replay below must keep it populated.
                let preserved_else_source = if source_exit_alias.is_none()
                    && !source_prefix.is_empty()
                {
                    let mut origins = self.exception_owner_provenance
                        .populated_try_else_borrow_exits.iter().filter(|(source, destination)| {
                            *destination == target && source_labels.contains(source)
                        });
                    origins.next().and_then(|&(source, _)| {
                        (origins.next().is_none()
                            && self.exception_owner_provenance.source_boundaries.contains(&source)
                            && self.preserved_block_boundaries.contains(&source))
                        .then_some(source)
                    })
                } else {
                    None
                };
                // A live condition can target the retained jump after an empty
                // manager-end array. Keep that incoming edge separate: ordinary
                // fallthrough through the earlier boundary is still disconnected.
                let conditional_entry = if source_prefix.is_empty()
                    && source_exit_alias.is_none()
                    && setup_source.is_none()
                    && !jump.fusion_policy.is_authored_break()
                    && source_labels.iter().filter(|label| {
                        self.exception_owner_provenance.block_boundaries.contains(*label)
                    }).count() > 1
                {
                    let entry = source_labels.iter().rev().copied().find(|label| {
                        self.exception_owner_provenance.block_boundaries.contains(label)
                    })?;
                    let mut incoming = self.graph.iter().filter_map(|(_, block)| {
                        let items = block.items();
                        let Item::Instruction(condition) = items.last()? else {
                            return None;
                        };
                        if !is_conditional_jump(condition.opcode)
                            || condition.operand() != Operand::Forward(entry)
                            || condition.location != jump.location
                            || condition.jump_policy.borrow_empty_block()
                            || condition.no_location_policy.borrow_fallthrough()
                                != BorrowFallthrough::Reachable
                        {
                            return None;
                        }
                        let source = items.iter().rev().find_map(|item| match item {
                            Item::Label(label) => Some(*label),
                            Item::Instruction(_) => None,
                        })?;
                        Some((source, *condition))
                    });
                    let entry = incoming.next()?;
                    if incoming.next().is_some() {
                        return None;
                    }
                    Some(entry)
                } else {
                    None
                };
                // Consecutive exception aliases may share one original array. Only
                // the captured conditional can bypass earlier empty source arrays.
                for (labels, preserved_source, setup_source, conditional_source) in [
                    (&source_labels, preserved_else_source, setup_source, conditional_entry.is_some()),
                    (&target_labels, None, None, false),
                ] {
                    let expected_boundaries = usize::from(setup_source.is_none());
                    if (!conditional_source && labels
                        .iter()
                        .filter(|label| {
                            self.exception_owner_provenance
                                .block_boundaries
                                .contains(*label)
                        })
                        .count()
                        != expected_boundaries)
                        || labels.iter().any(|label| {
                            self.preserved_block_boundaries.contains(label)
                                && Some(*label) != preserved_source
                        })
                    {
                        return None;
                    }
                }
                if source_labels
                    .iter()
                    .any(|label| self.borrow_unreachable_blocks.contains(label))
                    || !self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(&target)
                    || !target_labels.contains(&target)
                    || !target_labels
                        .iter()
                        .any(|label| self.borrow_unreachable_blocks.contains(label))
                    || target_labels.iter().any(|label| {
                        self.borrow_unreachable_blocks.contains(label)
                            && (!self.borrow_finally_entries.contains_key(label)
                                || !self.borrow_jump_entry_labels.contains(label))
                    })
                {
                    return None;
                }
                let mut first_live = None;
                for item in &target_items[first_instruction..] {
                    let Item::Instruction(instruction) = item else {
                        // Do not borrow a live instruction from a later original array.
                        return None;
                    };
                    if instruction.opcode == NOP
                        && instruction.fusion_policy.is_converted_pop_block()
                        && !instruction.jump_policy.borrow_empty_block()
                        && instruction.no_location_policy.borrow_fallthrough()
                            == BorrowFallthrough::Reachable
                    {
                        continue;
                    }
                    if matches!(instruction.opcode, NOP | NOT_TAKEN)
                        || !matches!(instruction.operand(), Operand::Value(_))
                        || instruction.jump_policy.borrow_empty_block()
                        || instruction.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::Reachable
                    {
                        return None;
                    }
                    first_live = Some(*instruction);
                    break;
                }
                Some(RetainedFinallyOrigin {
                    source_labels,
                    source_prefix,
                    source_exit_alias,
                    conditional_entry,
                    setup_join,
                    target_labels,
                    target,
                    jump_location: jump.location,
                    first_live: first_live?,
                })
            })
            .collect()
    }

    /// Requires the same populated conditional join captured before exit emission.
    fn original_borrow_join_survives(&self, join: Label, first: Instruction) -> bool {
        let Some(block) = self.graph.label_block(join) else {
            return false;
        };
        self.graph[block]
            .items()
            .iter()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction == first),
                Item::Label(_) => None,
            })
            == Some(true)
    }

    /// Keeps only original normal edges whose source and target arrays survive.
    pub(super) fn retained_finally_entries(
        &self,
        origins: &[RetainedFinallyOrigin],
    ) -> FxHashMap<Label, Label> {
        let mut entries = FxHashMap::default();
        if origins.is_empty() {
            return entries;
        }
        let items = self.graph.copy_items();
        let positions = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        for origin in origins {
            if let Some((source, expected)) = origin.conditional_entry {
                let Some(block) = self.graph.label_block(source) else {
                    continue;
                };
                let source_items = self.graph[block].items();
                let Some(Item::Instruction(actual)) = source_items.last() else {
                    continue;
                };
                if actual.opcode != expected.opcode
                    || actual.operand() != expected.operand()
                    || actual.location != expected.location
                    || actual.jump_policy.borrow_empty_block()
                    || actual.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::Reachable
                    || source_items.iter().rev().find_map(|item| match item {
                        Item::Label(label) => Some(*label),
                        Item::Instruction(_) => None,
                    }) != Some(source)
                {
                    continue;
                }
            }
            if origin
                .setup_join
                .is_some_and(|(join, first)| !self.original_borrow_join_survives(join, first))
            {
                continue;
            }
            let Some(start) = origin
                .source_labels
                .first()
                .and_then(|label| positions.get(label))
                .copied()
            else {
                continue;
            };
            if let Some(Item::Label(alias)) =
                start.checked_sub(1).and_then(|index| items.get(index))
            {
                // Removing a preceding ordinary try's normal jump can leave its
                // range-end alias beside this unchanged original source array.
                // Authenticate that original edge; an arbitrary merged label or
                // an actual original block boundary must still stop replay.
                if origin.source_exit_alias.is_none()
                    || origin.source_prefix.is_empty()
                    || self.exception_owner_provenance.normal_try_exits.iter().filter(|&&(entry, target)| {
                        entry == *alias
                            && origin.source_labels.contains(&target)
                            && self.exception_owner_provenance.block_boundaries.contains(&target)
                    }).count() != 1
                    || self.exception_owner_provenance.block_boundaries.contains(alias)
                    || self.exception_owner_provenance.source_boundaries.contains(alias)
                    || self.preserved_block_boundaries.contains(alias)
                    || self.borrow_unreachable_blocks.contains(alias)
                    || self.borrow_jump_entry_labels.contains(alias)
                    || self.borrow_finally_entries.contains_key(alias)
                    || self.exception_regions.iter().any(|region| region.target == *alias)
                    || items.iter().any(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                                if target == *alias))
                    })
                    || start.checked_sub(2).is_some_and(|index| matches!(items[index], Item::Label(_)))
                {
                    continue;
                }
            }
            if !origin.source_labels.iter().enumerate().all(|(offset, label)| {
                matches!(items.get(start + offset), Some(Item::Label(candidate)) if candidate == label)
            }) {
                continue;
            }
            let mut cursor = start + origin.source_labels.len();
            if origin.source_prefix.is_empty() {
                let Some(Item::Instruction(nop)) = items.get(cursor) else {
                    continue;
                };
                if nop.opcode != NOP
                    || nop.location != origin.jump_location
                    || nop.jump_policy.retained_nop()
                        != crate::assembler::ir::RetainedJumpNop::DirectTarget
                    || nop.jump_policy.borrow_empty_block()
                    || nop.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Reachable
                {
                    continue;
                }
                cursor += 1;
            } else {
                // The source was not a jump-only original array. Require every
                // original instruction before its erased jump, with no new labels,
                // copied instructions, retained jump NOP, or intervening empty array.
                let prefix_matches =
                    origin
                        .source_prefix
                        .iter()
                        .enumerate()
                        .all(|(offset, expected)| {
                            matches!(items.get(cursor + offset), Some(Item::Instruction(actual))
                            if origin.source_exit_alias.is_none_or(|_| actual == expected)
                                && actual.opcode == expected.opcode
                                && actual.operand() == expected.operand()
                                && actual.location == expected.location
                                && !actual.jump_policy.borrow_empty_block()
                                && actual.no_location_policy.borrow_fallthrough()
                                    == BorrowFallthrough::Reachable)
                        });
                if !prefix_matches {
                    continue;
                }
                cursor += origin.source_prefix.len();
            }
            if let Some(alias) = origin.source_exit_alias {
                if !matches!(items.get(cursor), Some(Item::Label(actual)) if *actual == alias)
                    || self.exception_owner_provenance.block_boundaries.contains(&alias)
                    || self.exception_owner_provenance.source_boundaries.contains(&alias)
                    || self.preserved_block_boundaries.contains(&alias)
                    || self.borrow_unreachable_blocks.contains(&alias)
                    || self.borrow_jump_entry_labels.contains(&alias)
                    || self.borrow_finally_entries.contains_key(&alias)
                    || self.exception_regions.iter().any(|region| region.target == alias)
                    || items.iter().any(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if matches!(instruction.operand(), Operand::Forward(target) | Operand::Backward(target)
                                if target == alias))
                    })
                {
                    continue;
                }
                cursor += 1;
            }
            // Exact adjacency excludes intervening empty arrays and newly merged labels.
            let target_start = cursor;
            let aliases_match = origin.target_labels.iter().enumerate().all(|(offset, label)| {
                matches!(items.get(target_start + offset), Some(Item::Label(candidate)) if candidate == label)
            });
            if !aliases_match {
                continue;
            }
            cursor += origin.target_labels.len();
            let Some(Item::Instruction(first)) = items.get(cursor) else {
                continue;
            };
            if first.opcode != origin.first_live.opcode
                || first.operand() != origin.first_live.operand()
                || first.location != origin.first_live.location
                || first.jump_policy.borrow_empty_block()
                || first.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Reachable
            {
                continue;
            }
            let source = origin.source_exit_alias.unwrap_or_else(|| {
                *origin
                    .source_labels
                    .last()
                    .expect("one original source label")
            });
            entries.insert(
                origin
                    .conditional_entry
                    .map_or(source, |(source, _)| source),
                origin.target,
            );
        }
        entries
    }
}

/// Tracks which analysis markers will remain as physical instructions after borrowing.
struct BorrowMarkerRetention {
    incoming_jumps: FxHashMap<Label, usize>,
    incoming: usize,
    fallthrough: bool,
}

impl BorrowMarkerRetention {
    fn new(items: &[Item]) -> Self {
        let mut incoming_jumps = FxHashMap::<Label, usize>::default();
        for item in items {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(target) | Operand::Backward(target) = instruction.operand()
            {
                *incoming_jumps.entry(target).or_default() += 1;
            }
        }
        Self {
            incoming_jumps,
            incoming: 0,
            fallthrough: true,
        }
    }

    fn keeps(&mut self, item: &Item) -> bool {
        match item {
            Item::Label(label) => {
                self.incoming += self.incoming_jumps.get(label).copied().unwrap_or_default();
                true
            }
            Item::Instruction(instruction) => {
                let predecessors = self.incoming + usize::from(self.fallthrough);
                // A retained line event needs one current normal entry. Shared joins can
                // carry an authored branch's line that does not belong on every path.
                let keep = !(instruction.opcode == NOP
                    && instruction.jump_policy.borrow_empty_block()
                    && !(instruction.jump_policy.always_retains_nop()
                        && instruction.no_location_policy.borrow_fallthrough()
                            != BorrowFallthrough::BarrierIfRemoved
                        && instruction.location.line >= 0
                        && predecessors == 1));
                if keep {
                    self.incoming = 0;
                    self.fallthrough = block_has_fallthrough(&[*item]);
                }
                keep
            }
        }
    }
}

fn instruction_ends_borrow_block(instruction: &Instruction) -> bool {
    !matches!(instruction.operand(), Operand::Value(_))
        || ends_scope(instruction.opcode)
        || instruction.opcode == NOT_TAKEN
        || (instruction.opcode == NOP && instruction.jump_policy.always_retains_nop())
        || instruction.jump_policy.borrow_empty_block()
        || instruction.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
}

/// Preserves a for-loop's explicit entry past an erased original backedge array.
fn for_iter_enters_original_cleanup(
    instruction: &Instruction,
    target: Label,
    items: &[Item],
    original_boundaries: &FxHashSet<Label>,
    unreachable: &FxHashSet<Label>,
) -> bool {
    if instruction.opcode != FOR_ITER || !original_boundaries.contains(&target) {
        return false;
    }
    let mut at_target = false;
    for item in items {
        match item {
            Item::Label(label) => {
                at_target |= *label == target;
                // Earlier empty arrays do not block this explicit edge. An empty
                // array at or after its actual target still stops traversal.
                if at_target && unreachable.contains(label) {
                    return false;
                }
            }
            Item::Instruction(first) => {
                return at_target
                    && first.opcode == END_FOR
                    && first.operand() == Operand::Value(0)
                    && first.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Reachable
                    && !first.jump_policy.borrow_empty_block();
            }
        }
    }
    false
}

/// Preserves an explicit false edge past an earlier erased original array.
fn conditional_enters_original_block(
    instruction: &Instruction,
    target: Label,
    items: &[Item],
    original_boundaries: &FxHashSet<Label>,
    unreachable: &FxHashSet<Label>,
) -> bool {
    if instruction.opcode != POP_JUMP_IF_FALSE
        || instruction.operand() != Operand::Forward(target)
        || !original_boundaries.contains(&target)
        || instruction.jump_policy.borrow_empty_block()
        || instruction.jump_policy.is_normalized_conditional_backedge()
        || !instruction.jump_policy.allows_borrow_threading_target()
        || instruction.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Reachable
    {
        return false;
    }
    let mut at_target = false;
    for item in items {
        match item {
            Item::Label(label) => {
                if *label == target {
                    at_target = true;
                } else if at_target && original_boundaries.contains(label) {
                    // An empty original target cannot inherit a later array's body.
                    return false;
                }
                // Only erased arrays strictly before this explicit target are bypassed.
                if at_target && unreachable.contains(label) {
                    return false;
                }
            }
            Item::Instruction(first) => {
                return at_target
                    && !matches!(first.opcode, NOP | NOT_TAKEN)
                    && first.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Reachable
                    && !first.jump_policy.borrow_empty_block();
            }
        }
    }
    false
}

fn borrow_block_ranges(
    items: &[Item],
    boundary_labels: &FxHashSet<Label>,
    borrow_unreachable_labels: &FxHashSet<Label>,
    preserved_block_boundaries: &FxHashSet<Label>,
) -> (Vec<Range<usize>>, Vec<usize>, Vec<bool>) {
    let label_count = items
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) => usize::try_from(label.0).ok(),
            Item::Instruction(_) => None,
        })
        .max()
        .map_or(0, |index| index + 1);
    let mut label_blocks = vec![usize::MAX; label_count];
    let mut ranges = Vec::new();
    let mut unreachable = Vec::new();
    let mut start = 0;
    let mut has_instruction = false;
    let mut has_block_label = false;
    let mut is_unreachable = false;

    for (index, item) in items.iter().enumerate() {
        // Empty original blocks stop CPython's borrow traversal. Exception-range
        // aliases alone do not establish an original block before this boundary.
        if let Item::Label(label) = item
            && boundary_labels.contains(label)
            && (has_instruction || (has_block_label && preserved_block_boundaries.contains(label)))
        {
            ranges.push(start..index);
            unreachable.push(is_unreachable);
            start = index;
            has_instruction = false;
            has_block_label = false;
            is_unreachable = false;
        }

        if let Item::Label(label) = item {
            label_blocks[usize::try_from(label.0).unwrap()] = ranges.len();
            is_unreachable |= borrow_unreachable_labels.contains(label);
            has_block_label |= boundary_labels.contains(label);
        }
        has_instruction |= matches!(item, Item::Instruction(_));

        let ends_block = matches!(item, Item::Instruction(instruction)
            if instruction_ends_borrow_block(instruction));
        if ends_block {
            ranges.push(start..index + 1);
            unreachable.push(is_unreachable);
            start = index + 1;
            has_instruction = false;
            has_block_label = false;
            is_unreachable = false;
        }
    }
    if start < items.len() {
        ranges.push(start..items.len());
        unreachable.push(is_unreachable);
    }

    (ranges, label_blocks, unreachable)
}

fn assertion_borrow_block_ranges(items: &[Item]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;

    for (index, item) in items.iter().enumerate() {
        let ends_block = matches!(item, Item::Instruction(instruction)
            if instruction_ends_borrow_block(instruction));
        if ends_block {
            ranges.push(start..index + 1);
            start = index + 1;
        }
    }
    if start < items.len() {
        ranges.push(start..items.len());
    }

    ranges
}

pub(in crate::assembler) fn optimize_linear_assertion_load_fast(items: &mut [Item]) {
    #[derive(Clone, Copy)]
    struct Reference {
        producer: u32,
        local: u32,
    }

    impl Reference {
        const NONE: u32 = u32::MAX;

        fn new(producer: Option<usize>, local: Option<u32>) -> Self {
            Self {
                producer: producer.map_or(Self::NONE, |producer| {
                    u32::try_from(producer).expect("borrow candidate index should fit in u32")
                }),
                local: local.unwrap_or(Self::NONE),
            }
        }

        const fn producer(self) -> Option<usize> {
            if self.producer == Self::NONE {
                None
            } else {
                Some(self.producer as usize)
            }
        }

        const fn local(self) -> Option<u32> {
            if self.local == Self::NONE {
                None
            } else {
                Some(self.local)
            }
        }

        const fn is_empty(self) -> bool {
            self.producer == Self::NONE && self.local == Self::NONE
        }
    }

    impl Default for Reference {
        fn default() -> Self {
            Self {
                producer: Self::NONE,
                local: Self::NONE,
            }
        }
    }

    #[derive(Default)]
    struct ReferenceStack {
        depth: usize,
        references: SmallVec<[(usize, Reference); 16]>,
    }

    impl ReferenceStack {
        fn reset(&mut self, depth: usize) {
            self.depth = depth;
            self.references.clear();
        }

        fn push(&mut self, reference: Reference) {
            if !reference.is_empty() {
                self.references.push((self.depth, reference));
            }
            self.depth += 1;
        }

        fn push_unknown(&mut self, count: usize) {
            self.depth += count;
        }

        fn pop(&mut self) -> Reference {
            if self.depth == 0 {
                return Reference::default();
            }
            self.depth -= 1;
            if self
                .references
                .last()
                .is_some_and(|(slot, _)| *slot == self.depth)
            {
                self.references.pop().unwrap().1
            } else {
                Reference::default()
            }
        }

        fn pop_many(&mut self, count: usize) {
            self.depth = self.depth.saturating_sub(count);
            while self
                .references
                .last()
                .is_some_and(|(slot, _)| *slot >= self.depth)
            {
                self.references.pop();
            }
        }

        fn copy_from_top(&self, distance: u32) -> Reference {
            let Some(slot) = self.depth.checked_sub(usize::try_from(distance).unwrap()) else {
                return Reference::default();
            };
            self.references
                .iter()
                .rev()
                .find_map(|(candidate, reference)| (*candidate == slot).then_some(*reference))
                .unwrap_or_default()
        }
    }

    fn kill_local(unsafe_loads: &mut [bool], stack: &ReferenceStack, local: u32) {
        for (_, reference) in &stack.references {
            if reference.local() == Some(local)
                && let Some(producer) = reference.producer()
            {
                unsafe_loads[producer] = true;
            }
        }
    }

    fn store_local(
        unsafe_loads: &mut [bool],
        stack: &ReferenceStack,
        local: u32,
        reference: Reference,
    ) {
        kill_local(unsafe_loads, stack, local);
        if let Some(producer) = reference.producer() {
            unsafe_loads[producer] = true;
        }
    }

    let mut start = 0;
    let mut stack = ReferenceStack::default();
    let mut candidates = SmallVec::<[usize; 16]>::new();
    let mut unsafe_loads = SmallVec::<[bool; 16]>::new();
    while start < items.len() {
        candidates.clear();
        let mut end = start;
        let mut has_local_mutation = false;
        let mut requires_stack_analysis = false;
        while end < items.len() {
            if let Item::Instruction(instruction) = &mut items[end] {
                if instruction.local_policy.force_borrowed_load() && instruction.opcode == LOAD_FAST
                {
                    instruction.opcode = LOAD_FAST_BORROW;
                } else if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_LOAD_FAST) {
                    candidates.push(end - start);
                    requires_stack_analysis |= instruction.local_policy.strict_owned_load()
                        || instruction.local_policy.force_owned_load();
                }
                has_local_mutation |= matches!(
                    instruction.opcode,
                    DELETE_FAST
                        | LOAD_FAST_AND_CLEAR
                        | STORE_FAST
                        | STORE_FAST_LOAD_FAST
                        | STORE_FAST_STORE_FAST
                );
                let ends_block = !matches!(instruction.operand(), Operand::Value(_))
                    || ends_scope(instruction.opcode)
                    || instruction.opcode == NOT_TAKEN
                    || (instruction.opcode == NOP && instruction.jump_policy.always_retains_nop())
                    || instruction.jump_policy.borrow_empty_block()
                    || instruction.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Barrier;
                end += 1;
                if ends_block {
                    break;
                }
            } else {
                end += 1;
            }
        }
        if candidates.is_empty() {
            start = end;
            continue;
        }

        let last = items[start..end]
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
            .expect("a borrow block with candidates should contain an instruction");
        let consumes_stack = (!matches!(last.operand(), Operand::Value(_))
            || ends_scope(last.opcode))
            && last.depth_after() == Some(0);
        if !has_local_mutation && !requires_stack_analysis && consumes_stack {
            for offset in &candidates {
                let Item::Instruction(instruction) = &mut items[start + offset] else {
                    unreachable!();
                };
                instruction.opcode = match instruction.opcode {
                    LOAD_FAST => LOAD_FAST_BORROW,
                    LOAD_FAST_LOAD_FAST => LOAD_FAST_BORROW_LOAD_FAST_BORROW,
                    _ => unreachable!("borrow candidates should remain local loads"),
                };
            }
            start = end;
            continue;
        }

        let range = start..end;
        let mut cumulative_effect = 0_i64;
        let mut start_depth = None;
        for item in &items[range.clone()] {
            let Item::Instruction(instruction) = *item else {
                continue;
            };
            let argument = match instruction.operand() {
                Operand::Value(argument) => argument,
                Operand::Forward(_) | Operand::Backward(_) => 0,
                Operand::Deferred(_) => {
                    unreachable!("operand patches are resolved before assembler passes")
                }
            };
            cumulative_effect += opcode_stack_effect(instruction.opcode, argument);
            if let Some(depth_after) = instruction.depth_after() {
                start_depth = Some(i64::from(depth_after) - cumulative_effect);
                break;
            }
        }
        stack.reset(usize::try_from(start_depth.unwrap_or(0).max(0)).unwrap());
        unsafe_loads.clear();
        unsafe_loads.resize(candidates.len(), false);
        let mut candidate = 0;

        for (offset, item) in items[range.clone()].iter().enumerate() {
            let Item::Instruction(instruction) = *item else {
                continue;
            };
            let argument = match instruction.operand() {
                Operand::Value(argument) => argument,
                Operand::Forward(_) | Operand::Backward(_) => 0,
                Operand::Deferred(_) => {
                    unreachable!("operand patches are resolved before assembler passes")
                }
            };
            match instruction.opcode {
                DELETE_FAST => kill_local(&mut unsafe_loads, &stack, argument),
                LOAD_FAST => {
                    debug_assert_eq!(candidates[candidate], offset);
                    stack.push(Reference::new(Some(candidate), Some(argument)));
                    candidate += 1;
                }
                LOAD_FAST_AND_CLEAR => {
                    kill_local(&mut unsafe_loads, &stack, argument);
                    stack.push(Reference::new(None, Some(argument)));
                }
                LOAD_FAST_LOAD_FAST => {
                    debug_assert_eq!(candidates[candidate], offset);
                    stack.push(Reference::new(Some(candidate), Some(argument >> 4)));
                    stack.push(Reference::new(Some(candidate), Some(argument & 15)));
                    candidate += 1;
                }
                STORE_FAST => {
                    let reference = stack.pop();
                    store_local(&mut unsafe_loads, &stack, argument, reference);
                }
                STORE_FAST_LOAD_FAST => {
                    let reference = stack.pop();
                    store_local(&mut unsafe_loads, &stack, argument >> 4, reference);
                    stack.push(Reference::new(None, Some(argument & 15)));
                }
                STORE_FAST_STORE_FAST => {
                    let reference = stack.pop();
                    store_local(&mut unsafe_loads, &stack, argument >> 4, reference);
                    let reference = stack.pop();
                    store_local(&mut unsafe_loads, &stack, argument & 15, reference);
                }
                COPY => stack.push(stack.copy_from_top(argument)),
                SWAP => unreachable!("linear assertion borrowing requires a SWAP-free stream"),
                FORMAT_SIMPLE | GET_ANEXT | GET_LEN | GET_YIELD_FROM_ITER | MATCH_KEYS
                | MATCH_MAPPING | MATCH_SEQUENCE | WITH_EXCEPT_START | IMPORT_FROM => {
                    let (popped, pushed) = opcode_stack_counts(instruction.opcode, argument);
                    for (producer, _) in items[range.clone()]
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| matches!(item, Item::Instruction(_)))
                        .take(pushed.saturating_sub(popped))
                    {
                        stack.push(Reference::new(
                            candidates.binary_search(&producer).ok(),
                            None,
                        ));
                    }
                }
                DICT_MERGE | DICT_UPDATE | LIST_APPEND | LIST_EXTEND | MAP_ADD | RERAISE
                | SET_ADD | SET_UPDATE => {
                    let (popped, pushed) = opcode_stack_counts(instruction.opcode, argument);
                    stack.pop_many(popped.saturating_sub(pushed));
                }
                END_SEND | SET_FUNCTION_ATTRIBUTE => {
                    let top = stack.pop();
                    stack.pop();
                    stack.push(top);
                }
                CHECK_EXC_MATCH => {
                    stack.pop();
                    stack.push_unknown(1);
                }
                FOR_ITER => stack.push_unknown(1),
                LOAD_ATTR | LOAD_SUPER_ATTR => {
                    let receiver = stack.pop();
                    if instruction.opcode == LOAD_SUPER_ATTR {
                        stack.pop_many(2);
                    }
                    stack.push_unknown(1);
                    if argument & 1 != 0 {
                        stack.push(receiver);
                    }
                }
                PUSH_EXC_INFO | LOAD_SPECIAL => {
                    let top = stack.pop();
                    stack.push_unknown(1);
                    stack.push(top);
                }
                SEND => {
                    stack.pop();
                    stack.push_unknown(1);
                }
                _ => {
                    let (popped, pushed) = opcode_stack_counts(instruction.opcode, argument);
                    stack.pop_many(popped);
                    stack.push_unknown(pushed);
                }
            }
        }
        debug_assert_eq!(candidate, candidates.len());
        for (_, reference) in &stack.references {
            if let Some(producer) = reference.producer() {
                unsafe_loads[producer] = true;
            }
        }
        for (candidate, offset) in candidates.iter().copied().enumerate() {
            if unsafe_loads[candidate] {
                continue;
            }
            let Item::Instruction(instruction) = items[start + offset] else {
                unreachable!();
            };
            if instruction.local_policy.strict_owned_load() {
                continue;
            }
            if instruction.local_policy.force_owned_load() {
                let mut following =
                    items[start + offset + 1..end]
                        .iter()
                        .filter_map(|item| match *item {
                            Item::Instruction(instruction) if instruction.opcode != NOP => {
                                Some(instruction.opcode)
                            }
                            _ => None,
                        });
                let first = following.next();
                let second = following.next();
                let directly_consumed = matches!(
                    first,
                    Some(RETURN_VALUE | STORE_DEREF | STORE_GLOBAL)
                ) || matches!((first, second), (Some(POP_TOP), Some(next)) if next != POP_EXCEPT)
                    || matches!(
                        (first, second),
                        (
                            Some(LOAD_COMMON_CONSTANT | LOAD_CONST | LOAD_SMALL_INT),
                            Some(COMPARE_OP)
                        )
                    );
                if !directly_consumed {
                    continue;
                }
            }
            let Item::Instruction(instruction) = &mut items[start + offset] else {
                unreachable!();
            };
            instruction.opcode = match instruction.opcode {
                LOAD_FAST => LOAD_FAST_BORROW,
                LOAD_FAST_LOAD_FAST => LOAD_FAST_BORROW_LOAD_FAST_BORROW,
                _ => unreachable!("borrow candidates should remain local loads"),
            };
        }
        start = end;
    }
}

/// Returns the same local-pair fusion used by the graph pass and copied-entry admission.
pub(super) fn fuse_pair(first: &Instruction, second: &Instruction) -> Option<Instruction> {
    let opcode = match (first.opcode, second.opcode) {
        (LOAD_FAST_BORROW, LOAD_FAST_BORROW) => LOAD_FAST_BORROW_LOAD_FAST_BORROW,
        (LOAD_FAST, LOAD_FAST) => LOAD_FAST_LOAD_FAST,
        (STORE_FAST, LOAD_FAST | LOAD_FAST_BORROW) => STORE_FAST_LOAD_FAST,
        (STORE_FAST, STORE_FAST) => STORE_FAST_STORE_FAST,
        _ => return None,
    };
    if first.fusion_policy.prevents_with_next()
        || first.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Barrier
        || second.fusion_policy.prevents_with_previous()
    {
        return None;
    }
    let (Operand::Value(first_argument), Operand::Value(second_argument)) =
        (first.operand(), second.operand())
    else {
        return None;
    };
    if first_argument >= 16 || second_argument >= 16 || first.location.line != second.location.line
    {
        return None;
    }
    Some(Instruction::fused(
        opcode,
        Operand::Value((first_argument << 4) | second_argument),
        *first,
        *second,
    ))
}

impl OptimizationPipeline {
    /// Records forward routes through original jump-only arrays to a nonempty array.
    ///
    /// An original NOP-only array cannot fall through into the following array for
    /// borrowed-load analysis. Only an explicit jump may extend a route across it.
    fn original_borrow_threading_targets(
        graph: &crate::assembler::block_graph::BlockGraph,
        original_block_boundaries: &FxHashSet<Label>,
        nonempty_targets: &FxHashSet<Label>,
    ) -> FxHashMap<Label, Label> {
        let mut links = FxHashMap::default();
        let mut pending = Vec::new();
        let mut saw_instruction = false;
        for item in graph.iter_items() {
            match item {
                Item::Label(label) => {
                    if original_block_boundaries.contains(label) {
                        if saw_instruction {
                            pending.clear();
                        }
                        saw_instruction = false;
                    }
                    pending.push(*label);
                }
                Item::Instruction(instruction) => {
                    if !matches!(instruction.opcode, NOP | NOT_TAKEN)
                        && matches!(instruction.operand(), Operand::Value(_))
                    {
                        pending.clear();
                    } else if instruction.opcode == crate::target::opcodes::JUMP_FORWARD
                        && instruction.jump_policy.allows_threading_target()
                        && instruction.jump_policy.allows_borrow_threading_target()
                        && !instruction.jump_policy.always_retains_nop()
                        && !instruction.jump_policy.borrow_empty_block()
                        && let Operand::Forward(target) = instruction.operand()
                    {
                        for label in &pending {
                            links.insert(*label, target);
                        }
                    }
                    saw_instruction = true;
                    if instruction.opcode.has_jump() || instruction_ends_borrow_block(instruction) {
                        pending.clear();
                        saw_instruction = false;
                    }
                }
            }
        }
        let mut routes = FxHashMap::default();
        for item in graph.iter_items() {
            let Item::Label(source) = item else {
                continue;
            };
            let mut target = *source;
            // A cyclic route cannot reach a nonempty array within this bound.
            for _ in 0..links.len() {
                let Some(next) = links.get(&target).copied() else {
                    break;
                };
                target = next;
                if nonempty_targets.contains(&target) {
                    routes.insert(*source, target);
                    break;
                }
            }
        }
        routes
    }

    /// Marks actual direct jumps before optimizer synthesis, threading, or copying.
    ///
    /// Source entries may be implicit. The instruction carries its initial target
    /// through block normalization and newly assigned labels. Only the separately
    /// recorded original jump-only route may retain a separate snapshot during
    /// intentional threading; the compact instruction tag still clears on operand
    /// changes. Empty fallthrough cannot acquire either proof.
    pub(super) fn mark_initial_borrow_jump_entries(
        graph: &mut crate::assembler::block_graph::BlockGraph,
        original_block_boundaries: &FxHashSet<Label>,
    ) -> FxHashMap<Label, Label> {
        let targets = graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction)
                    if crate::target::is_unconditional_jump(instruction.opcode) =>
                {
                    match instruction.operand() {
                        Operand::Forward(target) | Operand::Backward(target) => Some(target),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        let nonempty_targets =
            Self::nonempty_original_borrow_jump_entries(graph, original_block_boundaries, &targets);
        let routes = Self::original_borrow_threading_targets(
            graph,
            original_block_boundaries,
            &nonempty_targets,
        );
        let mut has_prefix = false;
        for position in graph.item_positions() {
            match *graph.item(position) {
                Item::Label(label) => {
                    if original_block_boundaries.contains(&label) {
                        has_prefix = false;
                    }
                }
                Item::Instruction(instruction) => {
                    if crate::target::is_unconditional_jump(instruction.opcode)
                        && has_prefix
                        && !instruction.jump_policy.borrow_empty_block()
                        && instruction.no_location_policy.borrow_fallthrough()
                            == BorrowFallthrough::Reachable
                        && let Operand::Forward(target) | Operand::Backward(target) =
                            instruction.operand()
                        && (nonempty_targets.contains(&target) || routes.contains_key(&target))
                    {
                        graph
                            .instruction_mut(position)
                            .unwrap()
                            .mark_initial_borrow_jump_target(target);
                    }
                    if instruction.opcode.has_jump() || instruction_ends_borrow_block(&instruction)
                    {
                        has_prefix = false;
                    } else if !matches!(instruction.opcode, NOP | NOT_TAKEN) {
                        has_prefix = true;
                    }
                }
            }
        }
        routes
    }

    /// Consumes instruction origins before copies can inherit them.
    pub(in crate::assembler) fn take_pre_copy_borrow_jump_entries(
        graph: &mut crate::assembler::block_graph::BlockGraph,
        original_block_boundaries: &FxHashSet<Label>,
    ) -> FxHashMap<Label, Label> {
        let entries = Self::capture_pre_copy_borrow_jump_entries(graph, original_block_boundaries);
        Self::clear_initial_borrow_jump_entries(graph);
        entries
    }

    pub(super) fn clear_initial_borrow_jump_entries(
        graph: &mut crate::assembler::block_graph::BlockGraph,
    ) {
        graph.for_each_instruction_mut(Instruction::clear_initial_borrow_jump_target);
    }

    /// Retains jump-entry admissions whose initial original array has a real prefix.
    ///
    /// A converted `POP_BLOCK` NOP can precede a separately emitted loop header. Once
    /// that NOP disappears, a jump to its original array still cannot enter the header.
    /// Capture this restriction before threading and NOP removal merge those targets.
    pub(super) fn nonempty_original_borrow_jump_entries(
        graph: &crate::assembler::block_graph::BlockGraph,
        original_block_boundaries: &FxHashSet<Label>,
        candidates: &FxHashSet<Label>,
    ) -> FxHashSet<Label> {
        let mut nonempty = FxHashSet::default();
        let mut pending = Vec::new();
        let mut saw_instruction = false;
        for item in graph.iter_items() {
            match item {
                Item::Label(label) => {
                    if original_block_boundaries.contains(label) {
                        if saw_instruction {
                            pending.clear();
                        }
                        saw_instruction = false;
                    }
                    if candidates.contains(label) {
                        pending.push(*label);
                    }
                }
                Item::Instruction(instruction) => {
                    if !matches!(instruction.opcode, NOP | NOT_TAKEN)
                        && matches!(instruction.operand(), Operand::Value(_))
                    {
                        nonempty.extend(pending.drain(..));
                    }
                    saw_instruction = true;
                    if instruction.opcode.has_jump() || instruction_ends_borrow_block(instruction) {
                        pending.clear();
                        saw_instruction = false;
                    }
                }
            }
        }
        nonempty
    }

    /// Captures positive entries from the graph at the caller's current stage.
    ///
    /// A later copied prefix cannot inherit this certificate: copies receive fresh
    /// labels, and both the nearest label and literal target must still match.
    /// Only instructions that retain their initial direct target can contribute.
    pub(in crate::assembler) fn capture_pre_copy_borrow_jump_entries(
        graph: &crate::assembler::block_graph::BlockGraph,
        original_block_boundaries: &FxHashSet<Label>,
    ) -> FxHashMap<Label, Label> {
        Self::capture_pre_copy_borrow_entries_with_threaded(
            graph,
            original_block_boundaries,
            &ThreadedBorrowEntries::default(),
        )
    }

    /// Saves only original jumps that followed their previously captured exact route.
    pub(in crate::assembler) fn snapshot_threaded_borrow_entries(
        graph: &crate::assembler::block_graph::BlockGraph,
        eligible_global_instructions: &FxHashSet<usize>,
    ) -> ThreadedBorrowEntries {
        let mut entries = ThreadedBorrowEntries::default();
        if eligible_global_instructions.is_empty() {
            return entries;
        }
        let mut global_instruction = 0;
        for (_, contents) in graph.iter() {
            let count = contents
                .items()
                .iter()
                .filter(|item| matches!(item, Item::Instruction(_)))
                .count();
            let eligible_jumps = (0..count)
                .filter(|index| {
                    eligible_global_instructions.contains(&(global_instruction + index))
                })
                .collect::<FxHashSet<_>>();
            global_instruction += count;
            if !eligible_jumps.is_empty()
                && let Some(Item::Label(anchor)) = contents.items().first()
            {
                entries.insert(
                    *anchor,
                    ThreadedBorrowSnapshot {
                        items: contents.items().to_vec(),
                        eligible_jumps,
                    },
                );
            }
        }
        entries
    }

    /// Finds unchanged captured spans using their stable first label.
    ///
    /// Threading can insert exception-owner labels inside a physical block. The
    /// normalization before exit copying splits that block at those labels and can
    /// reuse its arena id for a different segment. Match every captured item at its
    /// exact relative position, then return only cursors into the current graph.
    fn verified_threaded_borrow_positions(
        graph: &crate::assembler::block_graph::BlockGraph,
        threaded: &ThreadedBorrowEntries,
    ) -> FxHashSet<crate::assembler::block_graph::ItemPosition> {
        let mut eligible = FxHashSet::default();
        if threaded.is_empty() {
            return eligible;
        }
        let positions = graph.item_positions();
        for (index, position) in positions.iter().copied().enumerate() {
            let Item::Label(label) = graph.item(position) else {
                continue;
            };
            let Some(snapshot) = threaded.get(label) else {
                continue;
            };
            let Some(span) = positions[index..].get(..snapshot.items.len()) else {
                continue;
            };
            if !span
                .iter()
                .zip(&snapshot.items)
                .all(|(position, item)| graph.item(*position) == item)
            {
                continue;
            }
            let mut instruction_index = 0;
            for position in span {
                if matches!(graph.item(*position), Item::Instruction(_)) {
                    if snapshot.eligible_jumps.contains(&instruction_index) {
                        eligible.insert(*position);
                    }
                    instruction_index += 1;
                }
            }
        }
        eligible
    }

    /// Consumes both compact and snapshot origins before any exit blocks can be copied.
    pub(in crate::assembler) fn take_pre_copy_borrow_entries_with_threaded(
        graph: &mut crate::assembler::block_graph::BlockGraph,
        original_block_boundaries: &FxHashSet<Label>,
        threaded: &mut ThreadedBorrowEntries,
    ) -> FxHashMap<Label, Label> {
        if threaded.is_empty() {
            return Self::take_pre_copy_borrow_jump_entries(graph, original_block_boundaries);
        }
        let entries = Self::capture_pre_copy_borrow_entries_with_threaded(
            graph,
            original_block_boundaries,
            threaded,
        );
        Self::clear_initial_borrow_jump_entries(graph);
        threaded.clear();
        entries
    }

    fn capture_pre_copy_borrow_entries_with_threaded(
        graph: &crate::assembler::block_graph::BlockGraph,
        original_block_boundaries: &FxHashSet<Label>,
        threaded: &ThreadedBorrowEntries,
    ) -> FxHashMap<Label, Label> {
        let mut entries = FxHashMap::default();
        let threaded_positions = Self::verified_threaded_borrow_positions(graph, threaded);
        let mut nearest_label = None;
        let mut has_prefix = false;
        for (block, contents) in graph.iter() {
            for (offset, item) in contents.items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        nearest_label = Some(*label);
                        if original_block_boundaries.contains(label) {
                            has_prefix = false;
                        }
                    }
                    Item::Instruction(instruction) => {
                        if crate::target::is_unconditional_jump(instruction.opcode)
                            && has_prefix
                            && !instruction.jump_policy.borrow_empty_block()
                            && instruction.no_location_policy.borrow_fallthrough()
                                == BorrowFallthrough::Reachable
                            && let Some(source) = nearest_label
                            && let Operand::Forward(target) | Operand::Backward(target) =
                                instruction.operand()
                            && (instruction.initial_borrow_jump_target() == Some(target)
                                || threaded_positions.contains(
                                    &crate::assembler::block_graph::ItemPosition::new(
                                        block, offset,
                                    ),
                                ))
                        {
                            entries.insert(source, target);
                        }
                        if instruction.opcode.has_jump()
                            || instruction_ends_borrow_block(instruction)
                        {
                            nearest_label = None;
                            has_prefix = false;
                        } else if !matches!(instruction.opcode, NOP | NOT_TAKEN) {
                            has_prefix = true;
                        }
                    }
                }
            }
        }
        entries
    }
    /// Consumes original break spans before threading can retarget them.
    pub(super) fn take_authored_finally_break_threading_targets(
        graph: &crate::assembler::block_graph::BlockGraph,
        origins: &mut AuthoredFinallyBreakOrigins,
    ) -> FxHashMap<usize, Label> {
        let origins = std::mem::take(origins);
        let verified = Self::verified_threaded_borrow_positions(graph, &origins.snapshots);
        let mut targets = FxHashMap::default();
        let mut instruction_index = 0;
        for position in graph.item_positions() {
            let Item::Instruction(instruction) = graph.item(position) else {
                continue;
            };
            if verified.contains(&position)
                && instruction.opcode == crate::target::opcodes::JUMP_FORWARD
                && instruction.fusion_policy.is_authored_break()
                && !instruction.jump_policy.borrow_empty_block()
                && let Operand::Forward(target) = instruction.operand()
                && let Some(finally_target) = origins.routes.get(&target)
            {
                targets.insert(instruction_index, *finally_target);
            }
            instruction_index += 1;
        }
        targets
    }

    /// Freezes matched threaded breaks before either exit-copy path can rename them.
    pub(in crate::assembler) fn take_pre_copy_authored_finally_break_entries(
        graph: &crate::assembler::block_graph::BlockGraph,
        original_boundaries: &FxHashSet<Label>,
        origins: &mut AuthoredFinallyBreakOrigins,
    ) -> AuthoredFinallyBreakEntries {
        let origins = std::mem::take(origins);
        let verified = Self::verified_threaded_borrow_positions(graph, &origins.snapshots);
        let mut entries = AuthoredFinallyBreakEntries::default();
        let mut nearest_label = None;
        let mut original_source = None;
        for position in graph.item_positions() {
            match graph.item(position) {
                Item::Label(label) => {
                    nearest_label = Some(*label);
                    if original_boundaries.contains(label) {
                        original_source = Some(*label);
                    }
                }
                Item::Instruction(instruction) => {
                    if verified.contains(&position)
                        && instruction.opcode == crate::target::opcodes::JUMP_FORWARD
                        && instruction.fusion_policy.is_authored_break()
                        && !instruction.jump_policy.borrow_empty_block()
                        && let (Some(source), Some(original_source)) =
                            (nearest_label, original_source)
                        && origins.snapshots.contains_key(&original_source)
                        && let Operand::Forward(target) = instruction.operand()
                    {
                        entries.insert(
                            source,
                            AuthoredFinallyBreakEntry {
                                original_source,
                                target,
                            },
                        );
                    }
                    if instruction.opcode.has_jump() || instruction_ends_borrow_block(instruction) {
                        nearest_label = None;
                        original_source = None;
                    }
                }
            }
        }
        entries
    }

    /// Keeps high-local initialization across physical splits of one original CFG block.
    ///
    /// CPython checks locals above index 63 within each block. Exception boundaries and
    /// copied exits can split that block physically without losing earlier initialization.
    fn preserve_high_local_initialization(&mut self, local_count: usize) {
        if local_count <= 64 {
            return;
        }
        let mut block_labels = self.preserved_block_boundaries.clone();
        // Original source joins and jump targets stay distinct after their jumps disappear.
        block_labels.extend(&self.exception_owner_provenance.block_boundaries);
        for region in &self.exception_regions {
            block_labels.insert(region.target);
        }
        for item in self.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(target) | Operand::Backward(target) = instruction.operand()
            {
                block_labels.insert(target);
            }
        }
        let mut initialized = vec![false; local_count - 64];
        let mut previous_has_fallthrough = false;
        for block in self.graph.order().to_vec() {
            let continues_region = previous_has_fallthrough
                && self.graph[block]
                    .items()
                    .iter()
                    .take_while(|item| matches!(item, Item::Label(_)))
                    .all(
                        |item| matches!(item, Item::Label(label) if !block_labels.contains(label)),
                    );
            if !continues_region {
                initialized.fill(false);
            }
            previous_has_fallthrough =
                self.graph[block]
                    .last_instruction()
                    .is_none_or(|instruction| {
                        matches!(instruction.operand(), Operand::Value(_))
                            && !ends_scope(instruction.opcode)
                    });
            for instruction in self.graph.instructions_mut(block) {
                let Operand::Value(argument) = instruction.operand() else {
                    continue;
                };
                let Some(local) = usize::try_from(argument)
                    .ok()
                    .and_then(|index| index.checked_sub(64))
                else {
                    continue;
                };
                let Some(initialized) = initialized.get_mut(local) else {
                    continue;
                };
                match instruction.opcode {
                    DELETE_FAST | LOAD_FAST_AND_CLEAR => *initialized = false,
                    STORE_FAST => *initialized = !instruction.local_policy.may_store_null(),
                    LOAD_FAST | LOAD_FAST_CHECK => {
                        if *initialized {
                            instruction.opcode = LOAD_FAST;
                            instruction.local_policy.mark_known_initialized_load();
                        }
                        *initialized = true;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Converts potentially uninitialized local loads on an assertion-only success path.
    ///
    /// Every side exit is terminal and contains no local operations, so the low-local state can
    /// flow linearly through the graph. CPython resets its high-local state at each block entry.
    pub(in crate::assembler) fn add_checks_for_assertion_side_exits(
        &mut self,
        local_count: usize,
        parameter_count: usize,
    ) {
        if local_count == 0 {
            return;
        }

        let initialized = parameter_count.min(local_count).min(64);
        let mut unsafe_locals = u64::MAX
            .checked_shl(u32::try_from(initialized).unwrap())
            .unwrap_or(0);
        let mut unsafe_high_locals = vec![true; local_count.saturating_sub(64)];
        let order = self.graph.order().to_vec();
        for block in order {
            unsafe_high_locals.fill(true);
            for instruction in self.graph.instructions_mut(block) {
                if !matches!(
                    instruction.opcode,
                    DELETE_FAST | LOAD_FAST_AND_CLEAR | STORE_FAST | LOAD_FAST_CHECK | LOAD_FAST
                ) {
                    continue;
                }
                if instruction.local_policy.is_post_analysis_closure_load()
                    || instruction.local_policy.is_known_initialized_load()
                {
                    continue;
                }
                let Operand::Value(argument) = instruction.operand() else {
                    continue;
                };
                let Ok(local) = usize::try_from(argument) else {
                    continue;
                };
                if local >= local_count {
                    continue;
                }

                match instruction.opcode {
                    DELETE_FAST | LOAD_FAST_AND_CLEAR => {
                        if local < 64 {
                            unsafe_locals |= 1 << local;
                        } else {
                            unsafe_high_locals[local - 64] = true;
                        }
                    }
                    STORE_FAST => {
                        let may_store_null = instruction.local_policy.may_store_null();
                        if local < 64 {
                            if may_store_null {
                                unsafe_locals |= 1 << local;
                            } else {
                                unsafe_locals &= !(1 << local);
                            }
                        } else {
                            unsafe_high_locals[local - 64] = may_store_null;
                        }
                    }
                    LOAD_FAST_CHECK | LOAD_FAST => {
                        let is_unsafe = if local < 64 {
                            let bit = 1 << local;
                            let is_unsafe = unsafe_locals & bit != 0;
                            unsafe_locals &= !bit;
                            is_unsafe
                        } else {
                            std::mem::replace(&mut unsafe_high_locals[local - 64], false)
                        };
                        if is_unsafe {
                            instruction.opcode = LOAD_FAST_CHECK;
                        } else {
                            instruction.opcode = LOAD_FAST;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Converts potentially uninitialized local loads to checked loads.
    ///
    /// Mirrors CPython's `add_checks_for_loads_of_uninitialized_variables`
    /// flow-graph pass. A set bit means that a local may be uninitialized on
    /// at least one path into the block.
    pub(in crate::assembler) fn add_checks_for_uninitialized_loads(
        &mut self,
        local_count: usize,
        parameter_count: usize,
    ) {
        if local_count == 0 {
            return;
        }

        self.preserve_high_local_initialization(local_count);

        let graph = &self.graph;
        if !graph.features().has_fast_load() {
            return;
        }
        let order = graph.order().to_vec();
        if order.is_empty() {
            return;
        }
        if self.exception_regions.is_empty()
            && order.len() == 1
            && block_jump_target(graph[order[0]].items()).is_none()
        {
            let graph = &mut self.graph;
            let mut unsafe_locals = vec![true; local_count];
            unsafe_locals[..parameter_count.min(local_count).min(64)].fill(false);
            for instruction in graph.instructions_mut(order[0]) {
                if instruction.local_policy.is_post_analysis_closure_load()
                    || instruction.local_policy.is_known_initialized_load()
                {
                    continue;
                }
                let Operand::Value(argument) = instruction.operand() else {
                    continue;
                };
                let Ok(local) = usize::try_from(argument) else {
                    continue;
                };
                if local >= local_count {
                    continue;
                }
                match instruction.opcode {
                    DELETE_FAST | LOAD_FAST_AND_CLEAR => unsafe_locals[local] = true,
                    STORE_FAST => {
                        unsafe_locals[local] = instruction.local_policy.may_store_null();
                    }
                    LOAD_FAST_CHECK => {
                        if !unsafe_locals[local] {
                            instruction.opcode = LOAD_FAST;
                        }
                        unsafe_locals[local] = false;
                    }
                    LOAD_FAST => {
                        if unsafe_locals[local] {
                            instruction.opcode = LOAD_FAST_CHECK;
                        }
                        unsafe_locals[local] = false;
                    }
                    _ => {}
                }
            }
            return;
        }
        let block_slot_count = order
            .iter()
            .map(|block| block.index())
            .max()
            .map_or(0, |index| index + 1);
        let mut block_indices = vec![usize::MAX; block_slot_count];
        let mut block_starts = vec![0; block_slot_count];
        let mut item_count = 0;
        for (index, block) in order.iter().copied().enumerate() {
            block_indices[block.index()] = index;
            block_starts[block.index()] = item_count;
            item_count += graph[block].items().len();
        }
        let mut label_positions = vec![None; usize::try_from(self.next_label).unwrap()];
        for (index, item) in graph.iter_items().enumerate() {
            if let Item::Label(label) = item {
                label_positions[usize::try_from(label.0).unwrap()] = Some(index);
            }
        }

        let mut exception_regions = self
            .exception_regions
            .iter()
            .enumerate()
            .filter_map(|(index, region)| {
                let start = label_positions
                    .get(usize::try_from(region.start.0).unwrap())
                    .copied()
                    .flatten()?;
                let end = label_positions
                    .get(usize::try_from(region.end.0).unwrap())
                    .copied()
                    .flatten()?;
                if end <= start {
                    return None;
                }
                let target = graph.label_block(region.target).and_then(|block| {
                    block_indices
                        .get(block.index())
                        .copied()
                        .filter(|index| *index != usize::MAX)
                })?;
                let (priority_start, priority_end) = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => (
                        label_positions
                            .get(usize::try_from(start.0).unwrap())
                            .copied()
                            .flatten()?,
                        label_positions
                            .get(usize::try_from(end.0).unwrap())
                            .copied()
                            .flatten()?,
                    ),
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                let priority = (
                    region.is_scope_fallback,
                    Reverse(priority_start),
                    priority_end,
                    index,
                );
                Some((start, end - start, target, priority))
            })
            .collect::<Vec<_>>();
        // Fragment boundaries do not change original exception nesting. Apply lower-priority
        // ranges first so the same original owner used by exception tables wins each slot.
        exception_regions.sort_unstable_by_key(|(_, _, _, priority)| Reverse(*priority));
        let mut exception_successors = vec![None; item_count];
        for (start, span, target, _) in exception_regions {
            exception_successors[start..start + span].fill(Some(target));
        }

        let initialized = parameter_count.min(local_count).min(64);
        let mut unsafe_at_entry = vec![0_u64; order.len()];
        unsafe_at_entry[0] = u64::MAX
            .checked_shl(u32::try_from(initialized).unwrap())
            .unwrap_or(0);
        let mut unsafe_high_locals = vec![true; local_count.saturating_sub(64)];
        let mut needs_check = vec![false; item_count];
        let mut checked_loads_needing_check = vec![false; item_count];
        // Visit the entry and forward CFG edges first; the monotonic worklist still revisits
        // loop headers, while assertion-heavy acyclic graphs usually converge in one pass.
        let mut pending = (0..order.len()).rev().collect::<Vec<_>>();
        let mut queued = vec![true; order.len()];

        while let Some(block_index) = pending.pop() {
            queued[block_index] = false;
            let mut unsafe_locals = unsafe_at_entry[block_index];
            unsafe_high_locals.fill(true);

            let mut propagate = |target: usize, state: u64| {
                let current = &mut unsafe_at_entry[target];
                let changed = state & !*current != 0;
                *current |= state;
                if changed && !queued[target] {
                    queued[target] = true;
                    pending.push(target);
                }
            };

            let block = order[block_index];
            for (offset, item) in graph[block].items().iter().enumerate() {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                if instruction.local_policy.is_post_analysis_closure_load()
                    || instruction.local_policy.is_known_initialized_load()
                {
                    continue;
                }
                if let Some(target) = exception_successors[block_starts[block.index()] + offset] {
                    propagate(target, unsafe_locals);
                }
                let Operand::Value(argument) = instruction.operand() else {
                    continue;
                };
                let Ok(local) = usize::try_from(argument) else {
                    continue;
                };
                if local >= local_count {
                    continue;
                }
                match instruction.opcode {
                    DELETE_FAST | LOAD_FAST_AND_CLEAR => {
                        if local < 64 {
                            unsafe_locals |= 1 << local;
                        } else {
                            unsafe_high_locals[local - 64] = true;
                        }
                    }
                    STORE_FAST => {
                        if local < 64 {
                            if instruction.local_policy.may_store_null() {
                                unsafe_locals |= 1 << local;
                            } else {
                                unsafe_locals &= !(1 << local);
                            }
                        } else {
                            unsafe_high_locals[local - 64] =
                                instruction.local_policy.may_store_null();
                        }
                    }
                    LOAD_FAST_CHECK => {
                        let is_unsafe = if local < 64 {
                            unsafe_locals & (1 << local) != 0
                        } else {
                            unsafe_high_locals[local - 64]
                        };
                        if is_unsafe {
                            checked_loads_needing_check[block_starts[block.index()] + offset] =
                                true;
                        }
                        if local < 64 {
                            unsafe_locals &= !(1 << local);
                        } else {
                            unsafe_high_locals[local - 64] = false;
                        }
                    }
                    LOAD_FAST => {
                        let is_unsafe = if local < 64 {
                            unsafe_locals & (1 << local) != 0
                        } else {
                            unsafe_high_locals[local - 64]
                        };
                        if is_unsafe {
                            needs_check[block_starts[block.index()] + offset] = true;
                        }
                        if local < 64 {
                            unsafe_locals &= !(1 << local);
                        } else {
                            unsafe_high_locals[local - 64] = false;
                        }
                    }
                    _ => {}
                }
            }

            if block_has_fallthrough(graph[block].items()) && block_index + 1 < order.len() {
                propagate(block_index + 1, unsafe_locals);
            }
            if let Some(target) = block_jump_target(graph[block].items())
                && let Some(target) = graph.label_block(target)
            {
                if let Some(target) = block_indices
                    .get(target.index())
                    .copied()
                    .filter(|target| *target != usize::MAX)
                {
                    propagate(target, unsafe_locals);
                }
            }
        }

        let graph = &mut self.graph;
        for block in order {
            for (offset, item) in graph.iter_items_mut(block).enumerate() {
                let ItemMut::Instruction(instruction) = item else {
                    continue;
                };
                let index = block_starts[block.index()] + offset;
                match instruction.opcode {
                    LOAD_FAST_CHECK if !checked_loads_needing_check[index] => {
                        instruction.opcode = LOAD_FAST;
                    }
                    LOAD_FAST if needs_check[index] => {
                        instruction.opcode = LOAD_FAST_CHECK;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Removes repeated checked loads within a basic block.
    ///
    /// Once a checked load succeeds, CPython's definite-assignment scan treats
    /// that local as initialized for subsequent instructions on the same path.
    pub(in crate::assembler) fn remove_redundant_checked_loads(&mut self) {
        let graph = &mut self.graph;
        if !graph.features().has_fast_load() {
            return;
        }
        let checked_blocks = graph
            .order()
            .iter()
            .copied()
            .filter(|block| {
                graph[*block].items().iter().any(|item| {
                    matches!(item, Item::Instruction(instruction) if instruction.opcode == LOAD_FAST_CHECK)
                })
            })
            .collect::<Vec<_>>();
        if checked_blocks.is_empty() {
            return;
        }
        for block in checked_blocks {
            let mut initialized = FxHashSet::default();
            for instruction in graph.instructions_mut(block) {
                let Operand::Value(argument) = instruction.operand() else {
                    break;
                };
                match instruction.opcode {
                    DELETE_DEREF | DELETE_FAST | LOAD_FAST_AND_CLEAR => {
                        initialized.remove(&argument);
                    }
                    LOAD_FAST_CHECK => {
                        if initialized.contains(&argument) {
                            instruction.opcode = LOAD_FAST;
                        }
                        initialized.insert(argument);
                    }
                    LOAD_FAST | STORE_DEREF => {
                        initialized.insert(argument);
                    }
                    STORE_FAST => {
                        if instruction.local_policy.may_store_null() {
                            initialized.remove(&argument);
                        } else {
                            initialized.insert(argument);
                        }
                    }
                    LOAD_FAST_LOAD_FAST | STORE_FAST_LOAD_FAST | STORE_FAST_STORE_FAST => {
                        initialized.extend([argument >> 4, argument & 15]);
                    }
                    _ => {}
                }
                if ends_scope(instruction.opcode) {
                    break;
                }
            }
        }
    }

    /// Retains original source entries when later threading changes a jump destination.
    fn record_borrow_finally_jump_sources(&mut self) {
        self.borrow_authored_finally_break_origins =
            crate::assembler::ir::AuthoredFinallyBreakOrigins::default();
        if self.borrow_finally_entries.is_empty() {
            return;
        }
        // Empty original blocks and folded-assertion barriers stop the borrowing
        // traversal. Exception-only entries do not establish a normal source path.
        let order = self.graph.order();
        let capacity = order
            .iter()
            .map(|block| block.index() + 1)
            .max()
            .unwrap_or(0);
        let mut positions = vec![usize::MAX; capacity];
        for (position, block) in order.iter().copied().enumerate() {
            positions[block.index()] = position;
        }
        let mut borrowed = vec![false; capacity];
        let mut pending = order.first().copied().into_iter().collect::<Vec<_>>();
        while let Some(block) = pending.pop() {
            if std::mem::replace(&mut borrowed[block.index()], true) {
                continue;
            }
            let items = self.graph[block].items();
            let Some(last) = items.iter().rev().find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            }) else {
                continue;
            };
            if block_has_fallthrough(items)
                && last.no_location_policy.borrow_fallthrough() != BorrowFallthrough::Barrier
                && let Some(next) = order.get(positions[block.index()] + 1)
            {
                pending.push(*next);
            }
            if let Some(target) = block_jump_target(items)
                && let Some(target) = self.graph.label_block(target)
            {
                pending.push(target);
            }
        }
        let mut sources = FxHashMap::default();
        let mut authored_breaks = FxHashSet::default();
        let mut global_instruction = 0;
        for block in order.iter().copied() {
            let items = self.graph[block].items();
            let instruction_count = items
                .iter()
                .filter(|item| matches!(item, Item::Instruction(_)))
                .count();
            let first_instruction = global_instruction;
            global_instruction += instruction_count;
            let Some(last) = items.iter().rev().find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            }) else {
                continue;
            };
            // Preserve the complete original array of a normally reached authored break,
            // including any preceding statements. Its explicit finally route is checked
            // after cleanup-call capture and again after actual threading.
            if borrowed[block.index()]
                && matches!(items.first(), Some(Item::Label(label))
                    if self.exception_owner_provenance.block_boundaries.contains(label))
                && last.opcode == crate::target::opcodes::JUMP_FORWARD
                && last.fusion_policy.is_authored_break()
                && last.location.line >= 0
                && matches!(last.operand(), Operand::Forward(_))
                && !last.jump_policy.borrow_empty_block()
                && !last.jump_policy.always_retains_nop()
                && last.no_location_policy.borrow_fallthrough() == BorrowFallthrough::Reachable
            {
                authored_breaks.insert(first_instruction + instruction_count - 1);
            }
            if !is_unconditional_jump(last.opcode)
                || last.fusion_policy.is_authored_break()
                || last.jump_policy.borrow_empty_block()
            {
                continue;
            }
            for item in items
                .iter()
                .take_while(|item| matches!(item, Item::Label(_)))
            {
                if let Item::Label(label) = item {
                    sources.insert(*label, borrowed[block.index()]);
                }
            }
        }
        self.borrow_finally_jump_sources = sources;
        self.borrow_authored_finally_break_origins.snapshots =
            Self::snapshot_threaded_borrow_entries(&self.graph, &authored_breaks);
    }

    /// Records nonempty cleanup targets before optimizer passes can merge their labels.
    #[cfg(test)]
    pub(in crate::assembler) fn record_borrow_cleanup_calls(&mut self) {
        let _ = self.record_borrow_cleanup_calls_with_setups();
    }

    /// Captures cleanup calls together with their original nested setup spans.
    pub(super) fn record_borrow_cleanup_calls_with_setups(&mut self) -> Vec<SetupFinallyOrigin> {
        if self.borrow_loop_exit_blocks.is_empty() && self.borrow_finally_entries.is_empty() {
            return Vec::new();
        }
        self.record_borrow_finally_jump_sources();
        let origins = self.capture_setup_finally_origins();
        let mut boundaries = self.preserved_block_boundaries.clone();
        boundaries.extend(self.borrow_unreachable_blocks.iter().copied());
        boundaries.extend(self.exception_regions.iter().map(|region| region.target));
        for item in self.graph.iter_items() {
            if let Item::Instruction(instruction) = item
                && let Operand::Forward(label) | Operand::Backward(label) = instruction.operand()
            {
                boundaries.insert(label);
            }
        }
        let preserved = self
            .preserved_block_boundaries
            .difference(&self.borrow_block_aliases)
            .copied()
            .collect();
        let calls =
            self.borrow_loop_exit_blocks
                .keys()
                .chain(self.borrow_finally_entries.keys())
                .copied()
                .collect::<FxHashSet<_>>()
                .into_iter()
                .map(|label| {
                    let call = (|| {
                        // Only a recorded nested SETUP_FINALLY span can extend this
                        // original physical block. Other empty blocks remain barriers.
                        let original = self.graph.label_block(label)?;
                        let items = origins
                            .iter()
                            .find(|origin| origin.entry == label)
                            .map_or_else(
                                || self.graph[original].items(),
                                |origin| origin.items.as_slice(),
                            );
                        let (blocks, label_blocks, _) = borrow_block_ranges(
                            items,
                            &boundaries,
                            &self.borrow_unreachable_blocks,
                            &preserved,
                        );
                        let range =
                            blocks.get(*label_blocks.get(usize::try_from(label.0).unwrap())?)?;
                        let instructions = items[range.clone()]
                            .iter()
                            .filter_map(|item| match item {
                                Item::Instruction(instruction) => Some(instruction),
                                Item::Label(_) => None,
                            })
                            .collect::<Vec<_>>();
                        let first = instructions.first()?;
                        if first.opcode != NOP || !first.fusion_policy.is_converted_pop_block() {
                            return None;
                        }
                        let live = instructions
                            .iter()
                            .copied()
                            .filter(|instruction| instruction.opcode != NOP)
                            .collect::<Vec<_>>();
                        let scope_exit = live.last().is_some_and(|last| ends_scope(last.opcode));
                        // Two discarded calls and the scope exit contribute at least five
                        // surviving instructions, beyond CPython's four-instruction copy limit.
                        // CALL/POP_TOP pairs survive constant folding and local fusion.
                        let large_terminal_cleanup = scope_exit
                            && live
                                .windows(2)
                                .filter(|pair| pair[0].opcode == CALL && pair[1].opcode == POP_TOP)
                                .nth(1)
                                .is_some();
                        // A local or attribute condition ends this original cleanup array
                        // before any call. Its first load cannot fold or fuse with the
                        // attribute/conversion/jump, so it witnesses the original entry.
                        // Match the complete primitive head; interior NOPs and extra prefixes
                        // must not borrow a witness from a different original array.
                        let primitive_condition = match instructions.as_slice() {
                            [_, load, attributes @ .., to_bool, jump] => load.opcode == LOAD_FAST
                                && matches!(load.operand(), Operand::Value(_))
                                && attributes.iter().all(|attribute| {
                                    attribute.opcode == LOAD_ATTR
                                        && matches!(attribute.operand(), Operand::Value(argument)
                                                if argument & 1 == 0)
                                })
                                && to_bool.opcode == TO_BOOL
                                && to_bool.operand() == Operand::Value(0)
                                && jump.opcode == POP_JUMP_IF_FALSE
                                && matches!(jump.operand(), Operand::Forward(_)),
                            [_, load, jump] => {
                                load.opcode == LOAD_FAST
                                    && matches!(load.operand(), Operand::Value(_))
                                    && jump.opcode == POP_JUMP_IF_NONE
                                    && matches!(jump.operand(), Operand::Forward(_))
                            }
                            _ => false,
                        } && instructions.iter().all(|instruction| {
                            !instruction.jump_policy.borrow_empty_block()
                                && instruction.no_location_policy.borrow_fallthrough()
                                    == BorrowFallthrough::Reachable
                        });
                        // A for cleanup starts a separate loop block after GET_ITER.
                        // This complete local/attribute prefix survives folding and cannot
                        // fuse its local load across the iterator conversion.
                        let primitive_iteration = match instructions.as_slice() {
                            [_, load, attributes @ .., get_iter] => load.opcode == LOAD_FAST
                                && matches!(load.operand(), Operand::Value(_))
                                && attributes.len() <= 1
                                && attributes.iter().all(|attribute| {
                                    attribute.opcode == LOAD_ATTR
                                        && matches!(attribute.operand(), Operand::Value(argument)
                                            if argument & 1 == 0)
                                })
                                && get_iter.opcode == GET_ITER
                                && get_iter.operand() == Operand::Value(0),
                            _ => false,
                        } && items[range.clone()]
                            .iter()
                            .skip_while(|item| matches!(item, Item::Label(_)))
                            .all(|item| matches!(item, Item::Instruction(_)))
                            && instructions.iter().all(|instruction| {
                                !instruction.jump_policy.borrow_empty_block()
                                    && instruction.no_location_policy.borrow_fallthrough()
                                        == BorrowFallthrough::Reachable
                            });
                        // Restoring a global receiver's attribute has no CALL. The
                        // store survives folding and local fusion, even when its trailing
                        // ordinary jump is removed. Match the complete original array.
                        let attribute_assignment = match instructions.as_slice() {
                            [_, value, receiver, store, jump] => {
                                value.opcode == LOAD_FAST
                                    && matches!(value.operand(), Operand::Value(_))
                                    && receiver.opcode == LOAD_GLOBAL
                                    && matches!(receiver.operand(), Operand::Value(argument)
                                    if argument & 1 == 0)
                                    && store.opcode == STORE_ATTR
                                    && matches!(store.operand(), Operand::Value(_))
                                    && is_unconditional_jump(jump.opcode)
                                    && matches!(jump.operand(), Operand::Forward(_))
                            }
                            _ => false,
                        };
                        // An inlined comprehension splits the call-argument array after
                        // saving its local. GET_ITER consumes the original local before
                        // that boundary and survives both folding and local fusion.
                        let comprehension_iteration = match instructions.as_slice() {
                            [_, prefix @ .., load, get_iter, clear, swap] => {
                                !prefix.is_empty()
                                    && load.opcode == LOAD_FAST
                                    && matches!(load.operand(), Operand::Value(_))
                                    && get_iter.opcode == GET_ITER
                                    && get_iter.operand() == Operand::Value(0)
                                    && clear.opcode == LOAD_FAST_AND_CLEAR
                                    && matches!(clear.operand(), Operand::Value(_))
                                    && swap.opcode == SWAP
                                    && swap.operand() == Operand::Value(2)
                            }
                            _ => false,
                        };
                        let unsplit_reachable_array = items[range.clone()]
                            .iter()
                            .skip_while(|item| matches!(item, Item::Label(_)))
                            .all(|item| matches!(item, Item::Instruction(_)))
                            && instructions.iter().all(|instruction| {
                                !instruction.jump_policy.borrow_empty_block()
                                    && instruction.no_location_policy.borrow_fallthrough()
                                        == BorrowFallthrough::Reachable
                            });
                        // Preserve any existing CALL witness before considering the
                        // other entry witnesses in the same original block.
                        let candidates = instructions
                            .iter()
                            .enumerate()
                            .filter(|(_, instruction)| instruction.opcode == CALL)
                            .chain(
                                instructions
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, instruction)| instruction.opcode != CALL),
                            );
                        for (index, instruction) in candidates {
                            // Captured calls, iterator prefixes and primitive conditions
                            // remain nonempty for an ordinary original jump. Suspension
                            // witnesses retain the authored-break route during traversal.
                            let alternate_finally_witness =
                                self.borrow_finally_entries.contains_key(&label)
                                    && !scope_exit
                                    && (instruction.opcode == CALL_KW
                                        || (instruction.opcode == GET_AWAITABLE
                                            && instruction.operand() == Operand::Value(0))
                                        || (primitive_iteration && instruction.opcode == GET_ITER)
                                        || (primitive_condition && index == 1)
                                        || (unsplit_reachable_array
                                            && ((attribute_assignment
                                                && instruction.opcode == STORE_ATTR)
                                                || (comprehension_iteration
                                                    && instruction.opcode == GET_ITER))));
                            if (instruction.opcode != CALL && !alternate_finally_witness)
                                || instruction.location.line < 0
                            {
                                continue;
                            }
                            let Operand::Value(argument) = instruction.operand() else {
                                continue;
                            };
                            // These five operations survive constant/NOP folding before exit copying:
                            // LOAD_FAST, LOAD_ATTR, CALL, POP_TOP, and the scope exit. The receiver
                            // load cannot form a fast-fast fusion with LOAD_ATTR. Small return calls
                            // lack POP_TOP and retain the existing empty-exit behavior.
                            if scope_exit
                                && !large_terminal_cleanup
                                && !(live.first().is_some_and(|first| first.opcode == LOAD_FAST)
                                    && live.get(1).is_some_and(|second| second.opcode == LOAD_ATTR)
                                    && instructions
                                        .get(index + 1)
                                        .is_some_and(|next| next.opcode == POP_TOP))
                            {
                                continue;
                            }
                            if instructions[..index].iter().any(|candidate| {
                                candidate.jump_policy.borrow_empty_block()
                                    || candidate.no_location_policy.borrow_fallthrough()
                                        != BorrowFallthrough::Reachable
                            }) {
                                continue;
                            }
                            let aliases = items[range.clone()]
                                .iter()
                                .take_while(|item| matches!(item, Item::Label(_)))
                                .filter_map(|item| match item {
                                    Item::Label(label) => Some(*label),
                                    Item::Instruction(_) => None,
                                })
                                .collect::<Vec<_>>();
                            return Some((
                                BorrowCleanupCall {
                                    opcode: instruction.opcode,
                                    argument,
                                    location: instruction.location,
                                },
                                aliases,
                            ));
                        }
                        None
                    })();
                    (label, call)
                })
                .collect::<Vec<_>>();
        let mut finally_entries = FxHashMap::default();
        for (label, entry) in calls {
            if let Some(policy) = self.borrow_loop_exit_blocks.get_mut(&label) {
                policy.cleanup_call = entry
                    .as_ref()
                    .filter(|(call, _)| call.opcode == CALL)
                    .map(|(call, _)| *call);
            }
            if self.borrow_finally_entries.contains_key(&label) {
                let (call, aliases) =
                    entry.map_or((None, vec![label]), |(call, aliases)| (Some(call), aliases));
                for alias in aliases {
                    // An independent unreachable marker keeps its own barrier even when
                    // it shares this original physical block with normal cleanup.
                    if self.borrow_unreachable_blocks.contains(&alias)
                        && !self.borrow_finally_entries.contains_key(&alias)
                    {
                        continue;
                    }
                    finally_entries
                        .entry(alias)
                        .and_modify(|prior| {
                            if *prior != call {
                                *prior = None;
                            }
                        })
                        .or_insert(call);
                }
            }
        }
        self.borrow_finally_entries = finally_entries;
        if !self
            .borrow_authored_finally_break_origins
            .snapshots
            .is_empty()
        {
            let cleanup_targets = self
                .borrow_finally_entries
                .iter()
                .filter_map(|(label, call)| call.map(|_| *label))
                .collect();
            let nonempty = Self::nonempty_original_borrow_jump_entries(
                &self.graph,
                &self.exception_owner_provenance.block_boundaries,
                &cleanup_targets,
            );
            self.borrow_authored_finally_break_origins.routes =
                Self::original_borrow_threading_targets(
                    &self.graph,
                    &self.exception_owner_provenance.block_boundaries,
                    &nonempty,
                );
        }
        origins
    }

    /// Mirrors CPython's final `optimize_load_fast` CFG pass.
    ///
    /// An owned local load can become a borrowed load only when its value is
    /// consumed in the same basic block, is not stored into another local, and
    /// remains supported by the original local until it is consumed.
    #[cfg(test)]
    pub(in crate::assembler) fn optimize_load_fast(
        items: &mut [Item],
        exception_regions: &[ExceptionRegion],
        entries: BorrowEntryLabels<'_>,
        preserved_block_boundaries: &FxHashSet<Label>,
        borrow_block_aliases: &FxHashSet<Label>,
        assertion_side_exits: bool,
    ) {
        Self::optimize_load_fast_with_retained_entries(
            items,
            exception_regions,
            entries,
            preserved_block_boundaries,
            borrow_block_aliases,
            assertion_side_exits,
            &FxHashMap::default(),
        );
    }

    /// Admits retained direct NOP edges using separately verified original arrays.
    pub(in crate::assembler) fn optimize_load_fast_with_retained_entries(
        items: &mut [Item],
        exception_regions: &[ExceptionRegion],
        entries: BorrowEntryLabels<'_>,
        preserved_block_boundaries: &FxHashSet<Label>,
        borrow_block_aliases: &FxHashSet<Label>,
        assertion_side_exits: bool,
        retained_finally_entries: &FxHashMap<Label, Label>,
    ) {
        #[derive(Clone, Copy)]
        struct Reference {
            producer: u32,
            local: u32,
        }

        impl Reference {
            const NONE: u32 = u32::MAX;

            fn new(producer: usize, local: Option<u32>) -> Self {
                let producer = u32::try_from(producer)
                    .expect("assembler item offset should fit in a compact borrow reference");
                debug_assert_ne!(
                    producer,
                    Self::NONE,
                    "assembler item offset should leave the compact borrow sentinel available"
                );
                debug_assert_ne!(local, Some(Self::NONE));
                let local = local.unwrap_or(Self::NONE);
                Self { producer, local }
            }

            const fn producer(self) -> Option<usize> {
                if self.producer == Self::NONE {
                    None
                } else {
                    Some(self.producer as usize)
                }
            }

            const fn local(self) -> Option<u32> {
                if self.local == Self::NONE {
                    None
                } else {
                    Some(self.local)
                }
            }

            const fn is_empty(self) -> bool {
                self.producer == Self::NONE && self.local == Self::NONE
            }
        }

        impl Default for Reference {
            fn default() -> Self {
                Self {
                    producer: Self::NONE,
                    local: Self::NONE,
                }
            }
        }

        #[derive(Default)]
        struct ReferenceStack {
            depth: usize,
            references: SmallVec<[(usize, Reference); 16]>,
        }

        impl ReferenceStack {
            fn reset(&mut self, depth: usize) {
                self.depth = depth;
                self.references.clear();
            }

            fn push(&mut self, reference: Reference) {
                if !reference.is_empty() {
                    self.references.push((self.depth, reference));
                }
                self.depth += 1;
            }

            fn push_unknown(&mut self, count: usize) {
                self.depth += count;
            }

            fn pop(&mut self) -> Reference {
                if self.depth == 0 {
                    return Reference::default();
                }
                self.depth -= 1;
                if self
                    .references
                    .last()
                    .is_some_and(|(slot, _)| *slot == self.depth)
                {
                    self.references.pop().unwrap().1
                } else {
                    Reference::default()
                }
            }

            fn pop_many(&mut self, count: usize) {
                self.depth = self.depth.saturating_sub(count);
                while self
                    .references
                    .last()
                    .is_some_and(|(slot, _)| *slot >= self.depth)
                {
                    self.references.pop();
                }
            }

            fn get(&self, slot: usize) -> Reference {
                self.references
                    .binary_search_by_key(&slot, |(candidate, _)| *candidate)
                    .ok()
                    .map_or_else(Reference::default, |index| self.references[index].1)
            }

            fn set(&mut self, slot: usize, reference: Reference) {
                match self
                    .references
                    .binary_search_by_key(&slot, |(candidate, _)| *candidate)
                {
                    Ok(index) if reference.is_empty() => {
                        self.references.remove(index);
                    }
                    Ok(index) => self.references[index].1 = reference,
                    Err(index) if !reference.is_empty() => {
                        self.references.insert(index, (slot, reference));
                    }
                    Err(_) => {}
                }
            }

            fn copy_from_top(&self, distance: u32) -> Reference {
                self.depth
                    .checked_sub(usize::try_from(distance).unwrap())
                    .map_or_else(Reference::default, |slot| self.get(slot))
            }

            fn swap_from_top(&mut self, distance: u32) {
                let Some(slot) = self.depth.checked_sub(usize::try_from(distance).unwrap()) else {
                    return;
                };
                let top = self.depth.saturating_sub(1);
                if slot >= self.depth {
                    return;
                }
                let left = self.get(slot);
                let right = self.get(top);
                self.set(slot, right);
                self.set(top, left);
            }
        }

        fn kill_local(unsafe_loads: &mut [bool], stack: &ReferenceStack, local: u32) {
            for (_, reference) in &stack.references {
                if reference.local() == Some(local)
                    && let Some(producer) = reference.producer()
                {
                    unsafe_loads[producer] = true;
                }
            }
        }

        fn store_local(
            unsafe_loads: &mut [bool],
            stack: &ReferenceStack,
            local: u32,
            reference: Reference,
        ) {
            kill_local(unsafe_loads, stack, local);
            if let Some(producer) = reference.producer() {
                unsafe_loads[producer] = true;
            }
        }

        let BorrowEntryLabels {
            unreachable: borrow_unreachable_labels,
            loop_exits: borrow_loop_exit_labels,
            finally_entries: borrow_finally_entries,
            finally_jump_sources: borrow_finally_jump_sources,
            jump_entries: borrow_jump_entry_labels,
            original_boundaries: original_block_boundaries,
            pre_copy_jump_entries,
            authored_finally_break_entries,
        } = entries;

        if !items.iter().any(|item| {
            matches!(
                item,
                Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_LOAD_FAST)
            )
        }) {
            return;
        }

        let (blocks, reachable) = if assertion_side_exits {
            (assertion_borrow_block_ranges(items), None)
        } else {
            let mut block_labels = FxHashSet::default();
            for item in items.iter() {
                if let Item::Instruction(instruction) = item
                    && let Operand::Forward(label) | Operand::Backward(label) =
                        instruction.operand()
                {
                    block_labels.insert(label);
                }
            }
            // CPython's borrow analysis operates on CFG blocks. Protected-range
            // boundaries do not create blocks, but exception handlers do.
            for region in exception_regions {
                block_labels.insert(region.target);
            }
            block_labels.extend(borrow_unreachable_labels.iter().copied());
            block_labels.extend(preserved_block_boundaries.iter().copied());

            let borrow_boundaries = if borrow_block_aliases.is_empty() {
                std::borrow::Cow::Borrowed(preserved_block_boundaries)
            } else {
                std::borrow::Cow::Owned(
                    preserved_block_boundaries
                        .difference(borrow_block_aliases)
                        .copied()
                        .collect(),
                )
            };
            let (blocks, label_blocks, borrow_unreachable_blocks) = borrow_block_ranges(
                items,
                &block_labels,
                borrow_unreachable_labels,
                &borrow_boundaries,
            );
            let jump_entry_blocks = if borrow_jump_entry_labels.is_empty() {
                Vec::new()
            } else {
                blocks
                    .iter()
                    .map(|range| {
                        let mut has_entry = false;
                        for item in &items[range.clone()] {
                            if let Item::Label(label) = item
                                && borrow_unreachable_labels.contains(label)
                            {
                                if !borrow_jump_entry_labels.contains(label) {
                                    return false;
                                }
                                has_entry = true;
                            }
                        }
                        has_entry
                    })
                    .collect::<Vec<_>>()
            };
            // Cold-handler relocation keeps its shared warm break block nonempty. The
            // surviving authored break still reaches its exit, even when an adjacent
            // removed loop anchor records a disconnected fallthrough at that target.
            let cold_join_targets = items
                .iter()
                .filter_map(|item| {
                    let Item::Instruction(instruction) = item else {
                        return None;
                    };
                    if instruction.opcode == JUMP_BACKWARD_NO_INTERRUPT
                        && let Operand::Backward(label) = instruction.operand()
                    {
                        return Some(label);
                    }
                    None
                })
                .collect::<FxHashSet<_>>();
            // A removed break reaches an empty original block, but a branch around the
            // loop can still enter its nonempty continuation. Keep that entry available
            // only when every unreachable marker here belongs to the disconnected loop.
            let loop_exit_entry_policies = if borrow_loop_exit_labels.is_empty() {
                Vec::new()
            } else {
                blocks
                    .iter()
                    .map(|range| {
                        let labels = items[range.clone()].iter().filter_map(|item| match item {
                            Item::Label(label) if borrow_unreachable_labels.contains(label) => {
                                Some(label)
                            }
                            _ => None,
                        });
                        let mut has_loop_exit = false;
                        let mut policy = BorrowLoopExitPolicy {
                            allow_surviving_break: true,
                            preserve_small_exit: true,
                            cleanup_call: None,
                        };
                        for label in labels {
                            let entry = borrow_loop_exit_labels.get(label)?;
                            if !has_loop_exit {
                                policy.cleanup_call = entry.cleanup_call;
                            } else if policy.cleanup_call != entry.cleanup_call {
                                policy.cleanup_call = None;
                            }
                            has_loop_exit = true;
                            policy.allow_surviving_break &= entry.allow_surviving_break;
                            policy.preserve_small_exit &= entry.preserve_small_exit;
                        }
                        has_loop_exit.then_some(policy)
                    })
                    .collect::<Vec<_>>()
            };
            // A normal finally entry is eligible only when every collapsed unreachable
            // marker names the same original cleanup. An adjacent empty join remains a
            // barrier even when later optimization merges its labels into this range.
            let finally_entry_calls = if borrow_finally_entries.is_empty() {
                Vec::new()
            } else {
                blocks
                    .iter()
                    .map(|range| {
                        let mut call = None;
                        for item in &items[range.clone()] {
                            if let Item::Label(label) = item
                                && borrow_unreachable_labels.contains(label)
                            {
                                let entry = (*borrow_finally_entries.get(label)?)?;
                                if call.is_some_and(|prior| prior != entry) {
                                    return None;
                                }
                                call = Some(entry);
                            }
                        }
                        call
                    })
                    .collect::<Vec<_>>()
            };
            // Threaded destinations do not change whether the original source could
            // reach the jump. Every surviving source marker must agree; copied heads
            // are explicitly uncertified and cannot inherit a predecessor's marker.
            let finally_jump_sources = if borrow_finally_jump_sources.is_empty() {
                Vec::new()
            } else {
                blocks
                    .iter()
                    .map(|range| {
                        let mut source = None;
                        for item in &items[range.clone()] {
                            if let Item::Label(label) = item
                                && let Some(reachable) = borrow_finally_jump_sources.get(label)
                            {
                                source = Some(source.unwrap_or(true) && *reachable);
                            }
                        }
                        source.unwrap_or(false)
                    })
                    .collect::<Vec<_>>()
            };
            let surviving_instruction_offsets = (!borrow_finally_entries.is_empty()
                || !pre_copy_jump_entries.is_empty()
                || items.iter().any(|item| {
                    matches!(item, Item::Instruction(instruction)
                        if instruction.jump_policy.borrow_empty_block()
                            && !matches!(instruction.operand(), Operand::Value(_)))
                }))
            .then(|| {
                let mut retention = BorrowMarkerRetention::new(items);
                let mut offsets = Vec::with_capacity(items.len() + 1);
                let mut count = 0;
                offsets.push(count);
                for item in items.iter() {
                    if retention.keeps(item) && matches!(item, Item::Instruction(_)) {
                        count += 1;
                    }
                    offsets.push(count);
                }
                offsets
            });
            let mut reachable = vec![false; blocks.len()];
            let mut pending = (!blocks.is_empty())
                .then_some((0, false))
                .into_iter()
                .collect::<Vec<_>>();
            while let Some((block, surviving_entry)) = pending.pop() {
                // Native exit copying precedes NOP removal and local fusion. The final
                // block can be smaller, so conservatively retain the existing borrowing
                // for any remaining small exit. Large continuations still stop at the
                // empty original backedge block, even inside protected regions.
                let may_copy_small_exit = loop_exit_entry_policies
                    .get(block)
                    .copied()
                    .flatten()
                    .is_some_and(|policy| policy.preserve_small_exit)
                    && {
                        let instructions =
                            items[blocks[block].clone()]
                                .iter()
                                .filter_map(|item| match item {
                                    Item::Instruction(instruction) if instruction.opcode != NOP => {
                                        Some(instruction)
                                    }
                                    _ => None,
                                });
                        let mut count = 0;
                        let mut terminal = false;
                        for instruction in instructions {
                            count += 1;
                            terminal = ends_scope(instruction.opcode);
                        }
                        terminal && count <= 4
                    };
                if borrow_unreachable_blocks[block] && !surviving_entry && !may_copy_small_exit {
                    continue;
                }
                if std::mem::replace(&mut reachable[block], true) {
                    continue;
                }
                let Some(last) = items[blocks[block].clone()].iter().rev().find_map(|item| {
                    let Item::Instruction(instruction) = item else {
                        return None;
                    };
                    Some(instruction)
                }) else {
                    continue;
                };
                let borrow_traversal_stops = last.no_location_policy.borrow_fallthrough()
                    == BorrowFallthrough::Barrier
                    || last.jump_policy.borrow_empty_block();
                let folded_jump_has_no_fallthrough = last.opcode == NOP
                    && last.jump_policy.always_retains_nop()
                    && !matches!(
                        last.no_location_policy.borrow_fallthrough(),
                        BorrowFallthrough::RetainedNopReachable
                            | BorrowFallthrough::BarrierIfRemoved
                    );
                if !borrow_traversal_stops
                    && !folded_jump_has_no_fallthrough
                    && block_has_fallthrough(&[Item::Instruction(*last)])
                    && block + 1 < blocks.len()
                {
                    // A surviving original direct jump can become a physical NOP
                    // when cold handlers move away. Its exact target certificate
                    // admits fallthrough without reopening erased handler exits.
                    let original_finally_entry = items[blocks[block].clone()]
                        .iter()
                        .rev()
                        .find_map(|item| match item {
                            Item::Label(label) => Some(label),
                            Item::Instruction(_) => None,
                        })
                        .and_then(|source| retained_finally_entries.get(source))
                        .is_some_and(|target| {
                            label_blocks
                                .get(usize::try_from(target.0).unwrap())
                                .copied()
                                == Some(block + 1)
                                && items[blocks[block + 1].clone()].iter().all(|item| {
                                    !matches!(item, Item::Label(label)
                                            if borrow_unreachable_labels.contains(label)
                                                && !borrow_finally_entries.contains_key(label))
                                })
                                && items[blocks[block + 1].clone()]
                                    .iter()
                                    .find_map(|item| match item {
                                        Item::Instruction(instruction) => Some(instruction),
                                        Item::Label(_) => None,
                                    })
                                    .is_some_and(|instruction| {
                                        (!matches!(instruction.opcode, NOP | NOT_TAKEN)
                                            || (instruction.opcode == NOP
                                                && items[blocks[block + 1].clone()].iter().any(
                                                    |item| {
                                                        matches!(item, Item::Label(label)
                                                        if borrow_block_aliases.contains(label)
                                                            && borrow_finally_entries.get(label)
                                                                .is_some_and(Option::is_some))
                                                    },
                                                )))
                                            && !instruction.jump_policy.borrow_empty_block()
                                    })
                        });
                    let retained_finally_entry = last.opcode == NOP
                        && last.jump_policy.retained_nop()
                            == crate::assembler::ir::RetainedJumpNop::DirectTarget
                        && finally_jump_sources.get(block).copied().unwrap_or(false)
                        && jump_entry_blocks.get(block + 1).copied().unwrap_or(false)
                        // Earlier physical instructions can precede the exact
                        // original source labels after block merging. Replay has
                        // authenticated the final source label and its sole NOP.
                        && items[blocks[block].clone()]
                            .iter()
                            .rev()
                            .take_while(|item| !matches!(item, Item::Label(_)))
                            .filter(|item| matches!(item, Item::Instruction(_)))
                            .count()
                            == 1
                        && original_finally_entry;
                    // Removing a redundant ordinary jump preserves fallthrough when
                    // its original source still has its complete nonempty prefix.
                    // The same endpoint certificate rejects empty or merged targets.
                    // A sole surviving authored NOP is populated too, but only
                    // the exact source/target replay above may admit this route.
                    // DirectTarget jump surrogates retain their separate policy.
                    let retained_source_nop = last.opcode == NOP
                        && last.operand() == Operand::Value(0)
                        && last.location.line >= 0
                        && !last.fusion_policy.is_converted_pop_block()
                        && last.pre_fold_extra_instructions() == 0
                        && last.local_policy.hidden_exception_owner_slots() == 0
                        && last.jump_policy.retained_nop()
                            == crate::assembler::ir::RetainedJumpNop::None
                        && items[blocks[block].clone()]
                            .iter()
                            .filter(|item| matches!(item, Item::Instruction(_)))
                            .count()
                            == 1
                        && original_finally_entry;
                    let erased_finally_entry = (!matches!(last.opcode, NOP | NOT_TAKEN)
                        || retained_source_nop)
                        && finally_jump_sources.get(block).copied().unwrap_or(false)
                        && jump_entry_blocks.get(block + 1).copied().unwrap_or(false)
                        && original_finally_entry;
                    pending.push((block + 1, retained_finally_entry || erased_finally_entry));
                }
                // A removed normal edge can stop fallthrough at an empty CFG block,
                // but a surviving threaded jump still reaches its explicit target.
                // Ignore analysis-only NOPs when deciding whether a marked jump survives:
                // a zero-distance surrogate still represents the removed empty block.
                if let Operand::Forward(label) | Operand::Backward(label) = last.operand()
                    && let Some(target) = label_blocks
                        .get(usize::try_from(label.0).unwrap())
                        .copied()
                        .filter(|target| *target != usize::MAX)
                    && (!last.jump_policy.borrow_empty_block()
                        || surviving_instruction_offsets
                            .as_ref()
                            .is_some_and(|offsets| {
                                offsets[blocks[block].end] != offsets[blocks[target].start]
                            }))
                {
                    // Replay the captured conditional through its sole retained NOP.
                    // This admission belongs to the live conditional, never to another
                    // route entering an earlier empty label in the same physical range.
                    if is_conditional_jump(last.opcode)
                        && let Some(finally) = items[blocks[block].clone()]
                            .iter()
                            .rev()
                            .find_map(|item| match item {
                                Item::Label(label) => Some(label),
                                Item::Instruction(_) => None,
                            })
                            .and_then(|source| retained_finally_entries.get(source))
                        && let Some(finally_block) = label_blocks
                            .get(usize::try_from(finally.0).unwrap())
                            .copied()
                        && finally_block == target + 1
                        && finally_block < blocks.len()
                        && {
                            let mut instructions = items[blocks[target].clone()].iter().filter_map(
                                |item| match item {
                                    Item::Instruction(instruction) => Some(instruction),
                                    Item::Label(_) => None,
                                },
                            );
                            instructions.next().is_some_and(|nop| {
                                nop.opcode == NOP
                                    && nop.location == last.location
                                    && nop.jump_policy.retained_nop()
                                        == crate::assembler::ir::RetainedJumpNop::DirectTarget
                                    && !nop.jump_policy.borrow_empty_block()
                                    && nop.no_location_policy.borrow_fallthrough()
                                        == BorrowFallthrough::Reachable
                            }) && instructions.next().is_none()
                        }
                        && items[blocks[finally_block].clone()].iter().all(|item| {
                            !matches!(item, Item::Label(label)
                                if borrow_unreachable_labels.contains(label)
                                    && !borrow_finally_entries.contains_key(label))
                        })
                    {
                        pending.push((finally_block, true));
                    }
                    let for_cleanup_entry = for_iter_enters_original_cleanup(
                        last,
                        label,
                        &items[blocks[target].clone()],
                        original_block_boundaries,
                        borrow_unreachable_labels,
                    );
                    let conditional_entry = conditional_enters_original_block(
                        last,
                        label,
                        &items[blocks[target].clone()],
                        original_block_boundaries,
                        borrow_unreachable_labels,
                    );
                    let shared_break = last.fusion_policy.is_authored_break()
                        && items[blocks[block].clone()].iter().any(|item| {
                            matches!(item, Item::Label(label) if cold_join_targets.contains(label))
                        });
                    // Removed breaks have only fallthrough. When enabled for this exit,
                    // a surviving authored break can enter its nonempty continuation;
                    // other exits allow only independently authored jumps around the loop.
                    let independent_loop_entry = loop_exit_entry_policies
                        .get(target)
                        .copied()
                        .flatten()
                        .is_some_and(|policy| {
                            policy.allow_surviving_break || !last.fusion_policy.is_authored_break()
                        });
                    // A surviving authored jump can enter its original nonempty cleanup.
                    // The label supplies identity; the matching call checks that its body
                    // survived. Copied exits rename labels, and other unreachable markers
                    // keep the merged range ineligible through loop_exit_entry_policies.
                    let cleanup_entry = is_unconditional_jump(last.opcode)
                        && last.fusion_policy.is_authored_break()
                        && !last.jump_policy.borrow_empty_block()
                        && borrow_loop_exit_labels
                            .get(&label)
                            .and_then(|entry| entry.cleanup_call)
                            .is_some_and(|call| {
                                loop_exit_entry_policies
                                    .get(target)
                                    .copied()
                                    .flatten()
                                    .is_some_and(|entry| entry.cleanup_call == Some(call))
                                    && {
                                        let mut at_target = false;
                                        items[blocks[target].clone()].iter().any(
                                            |item| match item {
                                                Item::Label(candidate) => {
                                                    at_target |= *candidate == label;
                                                    false
                                                }
                                                Item::Instruction(instruction) => {
                                                    at_target
                                                        && instruction.opcode == CALL
                                                        && call.opcode == CALL
                                                        && instruction.operand()
                                                            == Operand::Value(call.argument)
                                                        && instruction.location == call.location
                                                }
                                            },
                                        )
                                    }
                            });
                    // Unlike the fallthrough and conditional paths, a surviving ordinary
                    // jump can enter the original nonempty normal-finally block directly.
                    // Only aliases captured before label merging identify that entry.
                    // Calls, conditions, attribute stores and iterator setup are captured
                    // from the same original array; require their exact surviving witness.
                    let finally_entry = is_unconditional_jump(last.opcode)
                        // A zero-distance surrogate can represent an empty native
                        // block, so it cannot certify an independent entry.
                        && surviving_instruction_offsets.as_ref().is_some_and(|offsets| {
                            offsets[blocks[block].end] != offsets[blocks[target].start]
                        })
                        && finally_jump_sources.get(block).copied().unwrap_or(false)
                        && !last.fusion_policy.is_authored_break()
                        && !last.jump_policy.borrow_empty_block()
                        && borrow_finally_entries
                            .get(&label)
                            .copied()
                            .flatten()
                            .is_some_and(|call| {
                                finally_entry_calls.get(target).copied().flatten() == Some(call)
                                    && {
                                        let mut at_target = false;
                                        items[blocks[target].clone()].iter().any(
                                            |item| match item {
                                                Item::Label(candidate) => {
                                                    at_target |= *candidate == label;
                                                    false
                                                }
                                                Item::Instruction(instruction) => {
                                                    if !at_target {
                                                        return false;
                                                    }
                                                    if call.opcode == LOAD_FAST {
                                                        // The captured primitive condition
                                                        // witnesses its first live record,
                                                        // never a later equal local load.
                                                        if instruction.opcode == NOP
                                                            && instruction
                                                                .fusion_policy
                                                                .is_converted_pop_block()
                                                            && !instruction
                                                                .jump_policy
                                                                .borrow_empty_block()
                                                            && instruction
                                                                .no_location_policy
                                                                .borrow_fallthrough()
                                                                == BorrowFallthrough::Reachable
                                                        {
                                                            return false;
                                                        }
                                                        at_target = false;
                                                    }
                                                    matches!(
                                                        call.opcode,
                                                        CALL | CALL_KW | GET_ITER | LOAD_FAST | STORE_ATTR
                                                    ) && instruction.opcode == call.opcode
                                                        && instruction.operand()
                                                            == Operand::Value(call.argument)
                                                        && instruction.location == call.location
                                                }
                                            },
                                        )
                                    }
                            });
                    // A normal cleanup can follow an empty original handler exit while
                    // a live branch targets the cleanup directly. Preserve that jump's
                    // entry without turning a removed jump's NOP into a new CFG edge.
                    let mut has_original_prefix = false;
                    let mut nearest_label = None;
                    for item in &items[blocks[block].clone()] {
                        match item {
                            Item::Label(label) => {
                                nearest_label = Some(*label);
                                if original_block_boundaries.contains(label) {
                                    has_original_prefix = false;
                                }
                            }
                            Item::Instruction(instruction) if std::ptr::eq(instruction, last) => {
                                break;
                            }
                            Item::Instruction(instruction)
                                if !matches!(instruction.opcode, NOP | NOT_TAKEN) =>
                            {
                                has_original_prefix = true;
                            }
                            Item::Instruction(_) => {}
                        }
                    }
                    // The surviving instructions must belong to this original array.
                    // Threading can leave a final jump after an originally empty join;
                    // instructions before that authored boundary do not prove an entry.
                    // A standalone jump remains unknown here, rather than proving empty.
                    let independent_jump_entry = crate::target::is_unconditional_jump(last.opcode)
                        && last.no_location_policy.borrow_fallthrough()
                            == BorrowFallthrough::Reachable
                        && !last.jump_policy.borrow_empty_block()
                        && has_original_prefix
                        // Cold-block movement can leave an analysis-only zero-distance
                        // jump. CPython removes that edge; it cannot reopen an empty array.
                        && surviving_instruction_offsets.as_ref().is_some_and(|offsets| {
                            offsets[blocks[block].end] != offsets[blocks[target].start]
                        })
                        && nearest_label.and_then(|source| pre_copy_jump_entries.get(&source))
                            == Some(&label)
                        && jump_entry_blocks.get(target).copied().unwrap_or(false);
                    // An original reachable break may thread past an empty loop exit
                    // into its captured nonempty finally array. Its separate origin
                    // cannot reopen generic empty blocks or transfer to copied labels.
                    let authored_finally_entry = is_unconditional_jump(last.opcode)
                        && last.fusion_policy.is_authored_break()
                        && !last.jump_policy.borrow_empty_block()
                        && last.no_location_policy.borrow_fallthrough()
                            == BorrowFallthrough::Reachable
                        && surviving_instruction_offsets
                            .as_ref()
                            .is_some_and(|offsets| {
                                offsets[blocks[block].end] != offsets[blocks[target].start]
                            })
                        && nearest_label
                            .and_then(|source| authored_finally_break_entries.get(&source))
                            .is_some_and(|entry| {
                                entry.target == label
                                    && items[blocks[block].clone()].iter().rev().find_map(|item| {
                                        match item {
                                            Item::Label(label)
                                                if original_block_boundaries.contains(label) =>
                                            {
                                                Some(*label)
                                            }
                                            _ => None,
                                        }
                                    }) == Some(entry.original_source)
                                    && jump_entry_blocks.get(target).copied().unwrap_or(false)
                                    && borrow_finally_entries
                                        .get(&label)
                                        .copied()
                                        .flatten()
                                        .is_some_and(|call| {
                                            finally_entry_calls.get(target).copied().flatten()
                                                == Some(call)
                                                && {
                                                    let mut at_target = false;
                                                    items[blocks[target].clone()].iter().any(
                                                        |item| match item {
                                                            Item::Label(candidate) => {
                                                                at_target |= *candidate == label;
                                                                false
                                                            }
                                                            Item::Instruction(instruction) => {
                                                                if !at_target {
                                                                    return false;
                                                                }
                                                                if call.opcode == LOAD_FAST {
                                                                    // A primitive condition
                                                                    // witnesses this entry's
                                                                    // first live instruction,
                                                                    // never a later equal load.
                                                                    if instruction.opcode == NOP
                                                                        && instruction
                                                                            .fusion_policy
                                                                            .is_converted_pop_block()
                                                                        && !instruction
                                                                            .jump_policy
                                                                            .borrow_empty_block()
                                                                        && instruction
                                                                            .no_location_policy
                                                                            .borrow_fallthrough()
                                                                            == BorrowFallthrough::Reachable
                                                                    {
                                                                        return false;
                                                                    }
                                                                    at_target = false;
                                                                }
                                                                instruction.opcode == call.opcode
                                                                    && instruction.operand()
                                                                        == Operand::Value(
                                                                            call.argument,
                                                                        )
                                                                    && instruction.location
                                                                        == call.location
                                                            }
                                                        },
                                                    )
                                                }
                                        })
                            });
                    pending.push((
                        target,
                        for_cleanup_entry
                            || conditional_entry
                            || shared_break
                            || independent_loop_entry
                            || cleanup_entry
                            || finally_entry
                            || independent_jump_entry
                            || authored_finally_entry,
                    ));
                }
            }

            (blocks, Some(reachable))
        };

        let mut stack = ReferenceStack::default();
        let mut unsafe_loads = Vec::new();
        for (block, range) in blocks.into_iter().enumerate() {
            let mut has_borrow_candidate = false;
            for item in &mut items[range.clone()] {
                let Item::Instruction(instruction) = item else {
                    continue;
                };
                if instruction.local_policy.force_borrowed_load() && instruction.opcode == LOAD_FAST
                {
                    instruction.opcode = LOAD_FAST_BORROW;
                } else if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_LOAD_FAST) {
                    has_borrow_candidate = true;
                }
            }
            if reachable
                .as_ref()
                .is_some_and(|reachable| !reachable[block])
                || !has_borrow_candidate
            {
                continue;
            }
            let mut cumulative_effect = 0_i64;
            let mut start_depth = None;
            for item in &items[range.clone()] {
                let Item::Instruction(instruction) = *item else {
                    continue;
                };
                let Operand::Value(argument) = instruction.operand() else {
                    cumulative_effect += opcode_stack_effect(instruction.opcode, 0);
                    if let Some(depth_after) = instruction.depth_after() {
                        start_depth = Some(i64::from(depth_after) - cumulative_effect);
                        break;
                    }
                    continue;
                };
                cumulative_effect += opcode_stack_effect(instruction.opcode, argument);
                if let Some(depth_after) = instruction.depth_after() {
                    start_depth = Some(i64::from(depth_after) - cumulative_effect);
                    break;
                }
            }
            stack.reset(usize::try_from(start_depth.unwrap_or(0).max(0)).unwrap());
            unsafe_loads.clear();
            unsafe_loads.resize(range.len(), false);

            for (offset, item) in items[range.clone()].iter().enumerate() {
                let Item::Instruction(instruction) = *item else {
                    continue;
                };
                let argument = match instruction.operand() {
                    Operand::Value(argument) => argument,
                    Operand::Forward(_) | Operand::Backward(_) => 0,
                    Operand::Deferred(_) => {
                        unreachable!("operand patches are resolved before assembler passes")
                    }
                };
                match instruction.opcode {
                    DELETE_FAST => kill_local(&mut unsafe_loads, &stack, argument),
                    LOAD_FAST => stack.push(Reference::new(offset, Some(argument))),
                    LOAD_FAST_AND_CLEAR => {
                        kill_local(&mut unsafe_loads, &stack, argument);
                        stack.push(Reference::new(offset, Some(argument)));
                    }
                    LOAD_FAST_LOAD_FAST => {
                        stack.push(Reference::new(offset, Some(argument >> 4)));
                        stack.push(Reference::new(offset, Some(argument & 15)));
                    }
                    STORE_FAST => {
                        let reference = stack.pop();
                        store_local(&mut unsafe_loads, &stack, argument, reference);
                    }
                    STORE_FAST_LOAD_FAST => {
                        let reference = stack.pop();
                        store_local(&mut unsafe_loads, &stack, argument >> 4, reference);
                        stack.push(Reference::new(offset, Some(argument & 15)));
                    }
                    STORE_FAST_STORE_FAST => {
                        let reference = stack.pop();
                        store_local(&mut unsafe_loads, &stack, argument >> 4, reference);
                        let reference = stack.pop();
                        store_local(&mut unsafe_loads, &stack, argument & 15, reference);
                    }
                    COPY => stack.push(stack.copy_from_top(argument)),
                    SWAP => stack.swap_from_top(argument),
                    // These instructions retain all their existing inputs.
                    FORMAT_SIMPLE | GET_ANEXT | GET_LEN | GET_YIELD_FROM_ITER | MATCH_KEYS
                    | MATCH_MAPPING | MATCH_SEQUENCE | WITH_EXCEPT_START | IMPORT_FROM => {
                        let (popped, pushed) = opcode_stack_counts(instruction.opcode, argument);
                        // CPython's inner loop shadows the instruction index,
                        // attributing these references to the first
                        // instructions in the block.
                        for (producer, _) in items[range.clone()]
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| matches!(item, Item::Instruction(_)))
                            .take(pushed.saturating_sub(popped))
                        {
                            stack.push(Reference::new(producer, None));
                        }
                    }
                    // These consume only their top inputs and retain the
                    // container deeper on the stack.
                    DICT_MERGE | DICT_UPDATE | LIST_APPEND | LIST_EXTEND | MAP_ADD | RERAISE
                    | SET_ADD | SET_UPDATE => {
                        let (popped, pushed) = opcode_stack_counts(instruction.opcode, argument);
                        stack.pop_many(popped.saturating_sub(pushed));
                    }
                    END_SEND | SET_FUNCTION_ATTRIBUTE => {
                        let top = stack.pop();
                        stack.pop();
                        stack.push(top);
                    }
                    CHECK_EXC_MATCH => {
                        stack.pop();
                        stack.push_unknown(1);
                    }
                    FOR_ITER => stack.push_unknown(1),
                    LOAD_ATTR | LOAD_SUPER_ATTR => {
                        let receiver = stack.pop();
                        if instruction.opcode == LOAD_SUPER_ATTR {
                            stack.pop_many(2);
                        }
                        stack.push_unknown(1);
                        if argument & 1 != 0 {
                            stack.push(receiver);
                        }
                    }
                    PUSH_EXC_INFO | LOAD_SPECIAL => {
                        let top = stack.pop();
                        stack.push_unknown(1);
                        stack.push(top);
                    }
                    SEND => {
                        stack.pop();
                        stack.push_unknown(1);
                    }
                    _ => {
                        let (popped, pushed) = opcode_stack_counts(instruction.opcode, argument);
                        stack.pop_many(popped);
                        stack.push_unknown(pushed);
                    }
                }
            }

            for (_, reference) in &stack.references {
                if let Some(producer) = reference.producer() {
                    unsafe_loads[producer] = true;
                }
            }
            for (offset, unsafe_load) in unsafe_loads.iter().copied().enumerate() {
                if unsafe_load {
                    continue;
                }
                let Item::Instruction(instruction) = items[range.start + offset] else {
                    continue;
                };
                let local_policy = instruction.local_policy;
                if local_policy.strict_owned_load() {
                    continue;
                }
                if local_policy.force_owned_load() {
                    let mut following = items[range.start + offset + 1..range.end]
                        .iter()
                        .filter_map(|item| match *item {
                            Item::Instruction(instruction) if instruction.opcode != NOP => {
                                Some(instruction.opcode)
                            }
                            _ => None,
                        });
                    let first = following.next();
                    let second = following.next();
                    let directly_consumed = matches!(
                        first,
                        Some(RETURN_VALUE | STORE_DEREF | STORE_GLOBAL)
                    ) || matches!((first, second), (Some(POP_TOP), Some(next)) if next != POP_EXCEPT)
                        || matches!(
                            (first, second),
                            (
                                Some(LOAD_COMMON_CONSTANT | LOAD_CONST | LOAD_SMALL_INT),
                                Some(COMPARE_OP)
                            )
                        );
                    if !directly_consumed {
                        continue;
                    }
                }
                let Item::Instruction(instruction) = &mut items[range.start + offset] else {
                    unreachable!();
                };
                match instruction.opcode {
                    LOAD_FAST => {
                        instruction.opcode = LOAD_FAST_BORROW;
                    }
                    LOAD_FAST_LOAD_FAST => {
                        instruction.opcode = LOAD_FAST_BORROW_LOAD_FAST_BORROW;
                    }
                    _ => {}
                }
            }
        }
    }

    pub(in crate::assembler) fn remove_borrow_empty_block_markers(items: &mut Vec<Item>) {
        if !items.iter().any(|item| {
            matches!(item, Item::Instruction(instruction) if instruction.jump_policy.borrow_empty_block())
        }) {
            return;
        }

        let mut retention = BorrowMarkerRetention::new(items);
        items.retain(|item| retention.keeps(item));
        for item in items {
            if let Item::Instruction(instruction) = item {
                instruction.jump_policy.clear_borrow_empty_block();
            }
        }
    }

    pub(in crate::assembler) fn fuse_superinstructions(&mut self) {
        let features = self.graph.features();
        if !features.has_fast_load() && !features.has_store_fast() {
            return;
        }

        let graph = &mut self.graph;
        let order = graph.order().to_vec();
        for block in order {
            graph.fuse_adjacent_instructions(block, fuse_pair);
        }
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::{FxHashMap, FxHashSet};

    use super::{assertion_borrow_block_ranges, borrow_block_ranges};
    use crate::assembler::{
        BorrowFallthrough, BorrowLoopExitPolicy, Instruction, Item, JUMP_FORWARD,
        LOAD_COMMON_CONSTANT, LOAD_CONST, LOAD_FAST, LOAD_FAST_BORROW, Label, NOP, NOT_TAKEN,
        Operand, POP_JUMP_IF_TRUE, POP_TOP, RETURN_VALUE, SourceLocation,
    };
    use crate::target::opcodes::RAISE_VARARGS;

    fn instruction(opcode: crate::target::Opcode, operand: Operand) -> Item {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            SourceLocation::NONE,
            None,
        ))
    }

    #[test]
    fn removed_borrow_markers_preserve_entry_counts_for_following_markers() {
        let target = Label(0);
        let alias = Label(1);
        for (mut items, retained) in [
            (
                vec![
                    instruction(POP_JUMP_IF_TRUE, Operand::Forward(target)),
                    Item::Label(target),
                ],
                false,
            ),
            (
                vec![
                    instruction(JUMP_FORWARD, Operand::Forward(target)),
                    Item::Label(target),
                ],
                true,
            ),
            (
                vec![
                    instruction(POP_JUMP_IF_TRUE, Operand::Forward(target)),
                    instruction(JUMP_FORWARD, Operand::Forward(alias)),
                    Item::Label(target),
                    Item::Label(alias),
                ],
                false,
            ),
            (vec![instruction(RETURN_VALUE, Operand::Value(0))], false),
        ] {
            for location in [SourceLocation::NONE, SourceLocation::new(2, 2, 4, 8)] {
                let mut marker = Instruction::new(NOP, Operand::Value(0), location, None);
                marker.jump_policy.retain_nop();
                marker.jump_policy.mark_borrow_empty_block();
                items.push(Item::Instruction(marker));
            }
            items.push(instruction(RETURN_VALUE, Operand::Value(0)));
            super::OptimizationPipeline::remove_borrow_empty_block_markers(&mut items);
            let nops = items.iter().filter(|item| {
                matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP)
            }).count();
            assert_eq!(nops, usize::from(retained));
            assert!(items.iter().all(|item| {
                !matches!(item, Item::Instruction(instruction)
                    if instruction.jump_policy.borrow_empty_block())
            }));
        }
    }

    #[test]
    fn pre_copy_jump_certificates_stop_at_analysis_block_boundaries() {
        let source = Label(0);
        let cleanup = Label(1);
        for marker in [
            instruction(super::NOT_TAKEN, Operand::Value(0)),
            instruction(NOP, Operand::Forward(cleanup)),
        ] {
            let items = vec![
                Item::Label(source),
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(POP_TOP, Operand::Value(0)),
                marker,
                instruction(JUMP_FORWARD, Operand::Forward(cleanup)),
                Item::Label(cleanup),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ];
            let mut graph =
                crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
            super::OptimizationPipeline::mark_initial_borrow_jump_entries(
                &mut graph,
                &FxHashSet::default(),
            );
            assert!(
                super::OptimizationPipeline::capture_pre_copy_borrow_jump_entries(
                    &graph,
                    &FxHashSet::default(),
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn pre_copy_jump_certificates_reject_copied_prefixes_and_empty_joins() {
        let cleanup = Label(0);
        let unrelated_barrier = Label(1);
        let original_join = Label(2);
        let original_entry = Label(3);
        let copied_entry = Label(4);
        let old_target = Label(5);
        for edge in [JUMP_FORWARD, NOP] {
            for has_live_prefix in [false, true] {
                for follows_original_join in [false, true] {
                    for shares_unrelated_barrier in [false, true] {
                        for copied_prefix in [false, true] {
                            for retargeted in [false, true] {
                                let mut items = vec![Item::Label(original_entry)];
                                if has_live_prefix {
                                    items.extend([
                                        instruction(LOAD_FAST, Operand::Value(1)),
                                        instruction(POP_TOP, Operand::Value(0)),
                                    ]);
                                } else {
                                    items.push(instruction(NOP, Operand::Value(0)));
                                }
                                if follows_original_join {
                                    items.push(Item::Label(original_join));
                                }
                                let jump = items.len();
                                items.extend([
                                    instruction(
                                        edge,
                                        Operand::Forward(if retargeted {
                                            old_target
                                        } else {
                                            cleanup
                                        }),
                                    ),
                                    instruction(LOAD_CONST, Operand::Value(0)),
                                    instruction(RETURN_VALUE, Operand::Value(0)),
                                    Item::Label(old_target),
                                    Item::Label(cleanup),
                                ]);
                                let mut blocked = FxHashSet::from_iter([cleanup]);
                                if shares_unrelated_barrier {
                                    items.push(Item::Label(unrelated_barrier));
                                    blocked.insert(unrelated_barrier);
                                }
                                let load = items.len();
                                items.push(instruction(LOAD_FAST, Operand::Value(0)));
                                items.push(instruction(RETURN_VALUE, Operand::Value(0)));
                                let boundaries = FxHashSet::from_iter([original_join]);
                                let mut graph =
                                    crate::assembler::block_graph::BlockGraph::from_items(
                                        items.clone(),
                                        &FxHashSet::default(),
                                    );
                                super::OptimizationPipeline::mark_initial_borrow_jump_entries(
                                    &mut graph,
                                    &boundaries,
                                );
                                let entries = super::OptimizationPipeline::capture_pre_copy_borrow_jump_entries(&graph, &boundaries);
                                if retargeted {
                                    let Item::Instruction(jump) = &mut items[jump] else {
                                        unreachable!()
                                    };
                                    jump.retarget(Operand::Forward(cleanup));
                                }
                                if copied_prefix {
                                    items.insert(1, Item::Label(copied_entry));
                                }
                                super::OptimizationPipeline::optimize_load_fast(
                                    &mut items,
                                    &[],
                                    super::BorrowEntryLabels {
                                        unreachable: &blocked,
                                        loop_exits: &FxHashMap::default(),
                                        finally_entries: &FxHashMap::default(),
                                        finally_jump_sources: &FxHashMap::default(),
                                        jump_entries: &FxHashSet::from_iter([cleanup]),
                                        original_boundaries: &boundaries,
                                        pre_copy_jump_entries: &entries,
                                        authored_finally_break_entries: &FxHashMap::default(),
                                    },
                                    &FxHashSet::default(),
                                    &FxHashSet::default(),
                                    false,
                                );
                                let Item::Instruction(actual) =
                                    items[load + usize::from(copied_prefix)]
                                else {
                                    unreachable!()
                                };
                                assert_eq!(
                                    actual.opcode,
                                    if edge == JUMP_FORWARD
                                        && has_live_prefix
                                        && !follows_original_join
                                        && !shares_unrelated_barrier
                                        && !copied_prefix
                                        && !retargeted
                                    {
                                        LOAD_FAST_BORROW
                                    } else {
                                        LOAD_FAST
                                    }
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn initial_borrow_jump_origins_survive_implicit_entries_and_normalization() {
        use crate::assembler::block_graph::{BlockGraph, ItemPosition};
        let leading = Label(0);
        let source = Label(1);
        let cleanup = Label(2);
        for implicit in [false, true] {
            let mut items = Vec::new();
            if !implicit {
                items.push(Item::Label(leading));
            }
            items.push(instruction(NOP, Operand::Value(0)));
            if !implicit {
                items.push(Item::Label(source));
            }
            items.extend([
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(POP_TOP, Operand::Value(0)),
                instruction(JUMP_FORWARD, Operand::Forward(cleanup)),
                Item::Label(cleanup),
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]);
            let boundaries = FxHashSet::from_iter([leading, source, cleanup]);
            let mut graph = BlockGraph::from_items(items, &FxHashSet::default());
            super::OptimizationPipeline::mark_initial_borrow_jump_entries(&mut graph, &boundaries);
            let old_source = graph
                .order()
                .iter()
                .copied()
                .find(|block| {
                    graph[*block]
                        .last_instruction()
                        .is_some_and(|i| i.opcode == JUMP_FORWARD)
                })
                .unwrap();
            let first = graph.order()[0];
            graph.remove_item(ItemPosition::new(first, usize::from(!implicit)));
            graph.normalize_blocks(&FxHashSet::default());
            let new_source = graph
                .order()
                .iter()
                .copied()
                .find(|block| {
                    graph[*block]
                        .last_instruction()
                        .is_some_and(|i| i.opcode == JUMP_FORWARD)
                })
                .unwrap();
            if !implicit {
                assert_ne!(old_source, new_source);
            }
            let assigned = Label(3);
            graph.insert_label(new_source, graph[new_source].items().len() - 3, assigned);
            let entries = super::OptimizationPipeline::take_pre_copy_borrow_jump_entries(
                &mut graph,
                &boundaries,
            );
            assert_eq!(entries.get(&assigned), Some(&cleanup));
            assert!(graph.iter_items().all(|item| match item {
                Item::Instruction(i) => i.initial_borrow_jump_target().is_none(),
                Item::Label(_) => true,
            }));
            // A same-target instruction copied after the proof was consumed cannot recapture it.
            let copied = BlockGraph::from_items(graph.iter_items().copied().collect(), &boundaries);
            assert!(
                super::OptimizationPipeline::capture_pre_copy_borrow_jump_entries(
                    &copied,
                    &boundaries
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn initial_borrow_jump_origins_reject_empty_targets_retargeting_and_synthesis() {
        use crate::assembler::block_graph::BlockGraph;
        let source = Label(0);
        let empty = Label(1);
        let cleanup = Label(2);
        for direct in [false, true] {
            for mutation in 0..3 {
                let items = vec![
                    Item::Label(source),
                    instruction(LOAD_CONST, Operand::Value(0)),
                    instruction(POP_TOP, Operand::Value(0)),
                    instruction(
                        JUMP_FORWARD,
                        Operand::Forward(if direct { cleanup } else { empty }),
                    ),
                    Item::Label(empty),
                    instruction(NOP, Operand::Value(0)),
                    instruction(JUMP_FORWARD, Operand::Forward(cleanup)),
                    Item::Label(cleanup),
                    instruction(LOAD_FAST, Operand::Value(0)),
                    instruction(RETURN_VALUE, Operand::Value(0)),
                ];
                let boundaries = FxHashSet::from_iter([source, empty, cleanup]);
                let mut graph = BlockGraph::from_items(items, &boundaries);
                super::OptimizationPipeline::mark_initial_borrow_jump_entries(
                    &mut graph,
                    &boundaries,
                );
                let source_block = graph.label_block(source).unwrap();
                let jump = graph.instructions_mut(source_block).next_back().unwrap();
                match mutation {
                    0 => jump.retarget(Operand::Forward(cleanup)),
                    1 => {
                        jump.set_operand(Operand::Forward(empty));
                        jump.retarget(Operand::Forward(cleanup));
                    }
                    2 => {
                        *jump = Instruction::synthetic(
                            JUMP_FORWARD,
                            Operand::Forward(cleanup),
                            SourceLocation::NONE,
                            None,
                        );
                    }
                    _ => unreachable!(),
                }
                let entries = super::OptimizationPipeline::take_pre_copy_borrow_jump_entries(
                    &mut graph,
                    &boundaries,
                );
                assert_eq!(
                    entries.get(&source),
                    (direct && mutation == 0).then_some(&cleanup)
                );
            }
        }
        let mut jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(cleanup),
            SourceLocation::NONE,
            None,
        );
        jump.mark_initial_borrow_jump_target(cleanup);
        let fused = Instruction::fused(LOAD_FAST, Operand::Value(0), jump, jump);
        assert_eq!(fused.initial_borrow_jump_target(), None);
    }

    #[test]
    fn original_borrow_jump_entry_targets_keep_empty_arrays_separate() {
        let target = Label(0);
        let alias = Label(1);
        let header = Label(2);
        let candidates = FxHashSet::from_iter([target, alias]);
        let boundaries = FxHashSet::from_iter([target, alias, header]);
        for prefix in 0..5 {
            let mut items = vec![Item::Label(target), Item::Label(alias)];
            match prefix {
                0 => {}
                1 => items.push(instruction(NOP, Operand::Value(0))),
                2 => items.push(instruction(NOT_TAKEN, Operand::Value(0))),
                3 => items.extend([
                    instruction(LOAD_CONST, Operand::Value(0)),
                    instruction(POP_TOP, Operand::Value(0)),
                ]),
                4 => items.push(instruction(JUMP_FORWARD, Operand::Forward(header))),
                _ => unreachable!(),
            }
            items.extend([
                Item::Label(header),
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]);
            let mut graph =
                crate::assembler::block_graph::BlockGraph::from_items(items, &boundaries);
            let expected = if matches!(prefix, 0 | 3) {
                candidates.clone()
            } else {
                FxHashSet::default()
            };
            assert_eq!(
                super::OptimizationPipeline::nonempty_original_borrow_jump_entries(
                    &graph,
                    &boundaries,
                    &candidates,
                ),
                expected,
            );
            let mut reachable = vec![true; graph.order().len()];
            graph.remove_initial_preserved_boundaries(Some(&mut reachable));
            assert_eq!(
                super::OptimizationPipeline::nonempty_original_borrow_jump_entries(
                    &graph,
                    &boundaries,
                    &candidates,
                ),
                expected,
            );
        }
    }

    #[test]
    fn protected_async_loop_exit_retains_borrowing_only_for_small_continuations() {
        let exit = Label(0);
        for preserve_small_exit in [false, true] {
            for padding in 0..3 {
                let mut items = vec![
                    instruction(LOAD_CONST, Operand::Value(0)),
                    instruction(POP_TOP, Operand::Value(0)),
                    Item::Label(exit),
                ];
                for _ in 0..padding {
                    items.push(instruction(LOAD_CONST, Operand::Value(0)));
                    items.push(instruction(POP_TOP, Operand::Value(0)));
                }
                let load = items.len();
                items.push(instruction(LOAD_FAST, Operand::Value(0)));
                items.push(instruction(RETURN_VALUE, Operand::Value(0)));
                let exits = FxHashMap::from_iter([(
                    exit,
                    BorrowLoopExitPolicy {
                        allow_surviving_break: false,
                        preserve_small_exit,
                        cleanup_call: None,
                    },
                )]);
                super::OptimizationPipeline::optimize_load_fast(
                    &mut items,
                    &[],
                    super::BorrowEntryLabels {
                        unreachable: &FxHashSet::from_iter([exit]),
                        loop_exits: &exits,
                        finally_entries: &FxHashMap::default(),
                        finally_jump_sources: &FxHashMap::default(),
                        jump_entries: &FxHashSet::default(),
                        original_boundaries: &FxHashSet::default(),
                        pre_copy_jump_entries: &FxHashMap::default(),
                        authored_finally_break_entries: &FxHashMap::default(),
                    },
                    &FxHashSet::default(),
                    &FxHashSet::default(),
                    false,
                );
                let Item::Instruction(instruction) = items[load] else {
                    unreachable!()
                };
                assert_eq!(
                    instruction.opcode,
                    if preserve_small_exit && padding < 2 {
                        LOAD_FAST_BORROW
                    } else {
                        LOAD_FAST
                    }
                );
            }
        }
    }

    #[test]
    fn borrowed_ranges_keep_protected_boundaries_and_split_handlers() {
        let protected_start = Label(0);
        let protected_end = Label(1);
        let handler = Label(2);
        let target = Label(3);
        let preserved = Label(4);
        let unreachable = Label(5);
        let items = vec![
            Item::Label(protected_start),
            instruction(LOAD_FAST, Operand::Value(0)),
            Item::Label(protected_end),
            instruction(LOAD_FAST, Operand::Value(0)),
            Item::Label(handler),
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(JUMP_FORWARD, Operand::Forward(target)),
            Item::Label(target),
            instruction(LOAD_FAST, Operand::Value(0)),
            Item::Label(preserved),
            instruction(LOAD_FAST, Operand::Value(0)),
            Item::Label(unreachable),
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];
        let boundary_labels = FxHashSet::from_iter([handler, target, preserved, unreachable]);
        let unreachable_labels = FxHashSet::from_iter([unreachable]);

        let (ranges, label_blocks, unreachable_blocks) = borrow_block_ranges(
            &items,
            &boundary_labels,
            &unreachable_labels,
            &FxHashSet::default(),
        );

        assert_eq!(ranges, [0..4, 4..7, 7..9, 9..11, 11..14]);
        assert_eq!(label_blocks, [0, 0, 1, 2, 3, 4]);
        assert_eq!(unreachable_blocks, [false, false, false, false, true]);
    }

    #[test]
    fn borrowed_ranges_split_retained_nops_and_fallthrough_barriers() {
        let mut retained = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
        retained.jump_policy.retain_nop();
        let mut barrier =
            Instruction::new(LOAD_FAST, Operand::Value(0), SourceLocation::NONE, None);
        barrier
            .no_location_policy
            .set_borrow_fallthrough(BorrowFallthrough::Barrier);
        let items = vec![
            instruction(LOAD_FAST, Operand::Value(0)),
            Item::Instruction(retained),
            instruction(LOAD_FAST, Operand::Value(0)),
            Item::Instruction(barrier),
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];

        let (ranges, _, unreachable) = borrow_block_ranges(
            &items,
            &FxHashSet::default(),
            &FxHashSet::default(),
            &FxHashSet::default(),
        );

        assert_eq!(ranges, [0..2, 2..4, 4..6]);
        assert_eq!(unreachable, [false, false, false]);
    }

    #[test]
    fn borrowed_ranges_preserve_empty_original_blocks_without_splitting_range_aliases() {
        let empty = Label(0);
        let alias = Label(1);
        let accepted = Label(2);
        let items = vec![
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(POP_JUMP_IF_TRUE, Operand::Forward(empty)),
            Item::Label(empty),
            Item::Label(alias),
            Item::Label(accepted),
            instruction(LOAD_FAST, Operand::Value(1)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];
        let preserved = FxHashSet::from_iter([accepted]);
        let (ranges, labels, _) = borrow_block_ranges(
            &items,
            &FxHashSet::from_iter([empty, accepted]),
            &FxHashSet::default(),
            &preserved,
        );
        assert_eq!(ranges, [0..2, 2..4, 4..7]);
        assert_eq!(labels, [1, 1, 2]);

        let (ranges, _, _) = borrow_block_ranges(
            &items,
            &FxHashSet::from_iter([accepted]),
            &FxHashSet::default(),
            &preserved,
        );
        assert_eq!(ranges, [0..2, 2..7]);
    }

    #[test]
    fn assertion_borrowed_ranges_preserve_all_reference_stack_boundaries() {
        let first = Label(0);
        let second = Label(1);
        let items = vec![
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(POP_JUMP_IF_TRUE, Operand::Forward(first)),
            instruction(NOT_TAKEN, Operand::Value(0)),
            instruction(LOAD_COMMON_CONSTANT, Operand::Value(0)),
            instruction(RAISE_VARARGS, Operand::Value(1)),
            Item::Label(first),
            instruction(LOAD_FAST, Operand::Value(1)),
            instruction(POP_JUMP_IF_TRUE, Operand::Forward(second)),
            instruction(NOT_TAKEN, Operand::Value(0)),
            instruction(LOAD_COMMON_CONSTANT, Operand::Value(0)),
            instruction(RAISE_VARARGS, Operand::Value(1)),
            Item::Label(second),
            instruction(LOAD_FAST, Operand::Value(2)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];

        let (expected, _, _) = borrow_block_ranges(
            &items,
            &FxHashSet::from_iter([first, second]),
            &FxHashSet::default(),
            &FxHashSet::default(),
        );

        assert_eq!(assertion_borrow_block_ranges(&items), expected);
        assert_eq!(expected, [0..2, 2..3, 3..5, 5..8, 8..9, 9..11, 11..14]);
    }
    fn threaded_entry_fixture(extra_hop: bool) -> crate::assembler::OptimizationPipeline {
        let source = Label(0);
        let join = Label(1);
        let cleanup = Label(2);
        let second = Label(3);
        let mut items = vec![
            Item::Label(source),
            instruction(LOAD_FAST, Operand::Value(1)),
            instruction(POP_TOP, Operand::Value(0)),
            instruction(JUMP_FORWARD, Operand::Forward(join)),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(join),
            instruction(
                JUMP_FORWARD,
                Operand::Forward(if extra_hop { second } else { cleanup }),
            ),
        ];
        if extra_hop {
            items.extend([
                Item::Label(second),
                instruction(JUMP_FORWARD, Operand::Forward(cleanup)),
            ]);
        }
        items.extend([
            Item::Label(cleanup),
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);
        let mut assembler = crate::assembler::Assembler::from_linear_items(items);
        assembler.next_label = 4;
        assembler.exception_owner_provenance.block_boundaries =
            FxHashSet::from_iter([source, join, cleanup, second]);
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.borrow_threaded_jump_targets =
            super::OptimizationPipeline::mark_initial_borrow_jump_entries(
                &mut pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
            );
        pipeline
    }

    fn borrowed_cleanup_after_entries(
        mut items: Vec<Item>,
        entries: &FxHashMap<Label, Label>,
        boundaries: &FxHashSet<Label>,
    ) -> crate::target::Opcode {
        super::OptimizationPipeline::optimize_load_fast(
            &mut items,
            &[],
            super::BorrowEntryLabels {
                unreachable: &FxHashSet::from_iter([Label(2)]),
                loop_exits: &FxHashMap::default(),
                finally_entries: &FxHashMap::default(),
                finally_jump_sources: &FxHashMap::default(),
                jump_entries: &FxHashSet::from_iter([Label(2)]),
                original_boundaries: boundaries,
                pre_copy_jump_entries: entries,
                authored_finally_break_entries: &FxHashMap::default(),
            },
            &FxHashSet::default(),
            &FxHashSet::default(),
            false,
        );
        items
            .iter()
            .find_map(|item| match item {
                Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_BORROW)
                        && instruction.operand() == Operand::Value(0) =>
                {
                    Some(instruction.opcode)
                }
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn original_threaded_entry_reaches_nonempty_cleanup() {
        let mut pipeline = threaded_entry_fixture(false);
        assert_eq!(
            pipeline.borrow_threaded_jump_targets.get(&Label(1)),
            Some(&Label(2))
        );
        assert!(pipeline.thread_forward_jumps());
        let boundaries = pipeline.exception_owner_provenance.block_boundaries.clone();
        let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
            &mut pipeline.graph,
            &boundaries,
            &mut pipeline.borrow_threaded_jump_entries,
        );
        assert_eq!(entries.get(&Label(0)), Some(&Label(2)));
        assert_eq!(
            borrowed_cleanup_after_entries(pipeline.graph.into_items(), &entries, &boundaries),
            LOAD_FAST_BORROW
        );
    }

    #[test]
    fn original_threaded_entry_can_follow_multiple_original_jumps() {
        let mut pipeline = threaded_entry_fixture(true);
        assert_eq!(
            pipeline.borrow_threaded_jump_targets.get(&Label(1)),
            Some(&Label(2))
        );
        assert_eq!(
            pipeline.borrow_threaded_jump_targets.get(&Label(3)),
            Some(&Label(2))
        );
        assert!(pipeline.thread_forward_jumps());
        let entries = super::OptimizationPipeline::capture_pre_copy_borrow_entries_with_threaded(
            &pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
            &pipeline.borrow_threaded_jump_entries,
        );
        assert_eq!(entries.get(&Label(0)), Some(&Label(2)));
    }

    #[test]
    fn original_nop_array_cannot_extend_a_route_by_fallthrough() {
        let mut pipeline = threaded_entry_fixture(false);
        let mut items = pipeline.graph.into_items();
        let join = items
            .iter()
            .position(|item| matches!(item, Item::Label(Label(1))))
            .unwrap();
        items[join + 1] = instruction(NOP, Operand::Value(0));
        pipeline.graph =
            crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
        super::OptimizationPipeline::clear_initial_borrow_jump_entries(&mut pipeline.graph);
        let routes = super::OptimizationPipeline::mark_initial_borrow_jump_entries(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
        );
        assert!(!routes.contains_key(&Label(1)));
        assert!(
            super::OptimizationPipeline::capture_pre_copy_borrow_jump_entries(
                &pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
            )
            .is_empty()
        );
    }

    #[test]
    fn original_jump_cycle_cannot_prove_a_nonempty_target() {
        let mut pipeline = threaded_entry_fixture(true);
        let mut items = pipeline.graph.into_items();
        let second = items
            .iter()
            .position(|item| matches!(item, Item::Label(Label(3))))
            .unwrap();
        items[second + 1] = instruction(JUMP_FORWARD, Operand::Forward(Label(1)));
        pipeline.graph =
            crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
        super::OptimizationPipeline::clear_initial_borrow_jump_entries(&mut pipeline.graph);
        let routes = super::OptimizationPipeline::mark_initial_borrow_jump_entries(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
        );
        assert!(!routes.contains_key(&Label(1)) && !routes.contains_key(&Label(3)));
    }

    #[test]
    fn retained_original_jump_head_does_not_receive_a_route() {
        let mut pipeline = threaded_entry_fixture(false);
        let mut items = pipeline.graph.into_items();
        let join = items
            .iter()
            .position(|item| matches!(item, Item::Label(Label(1))))
            .unwrap();
        let Item::Instruction(jump) = &mut items[join + 1] else {
            unreachable!()
        };
        jump.jump_policy.retain_nop();
        pipeline.graph =
            crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
        super::OptimizationPipeline::clear_initial_borrow_jump_entries(&mut pipeline.graph);
        let routes = super::OptimizationPipeline::mark_initial_borrow_jump_entries(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
        );
        assert!(!routes.contains_key(&Label(1)));
    }

    #[test]
    fn unrelated_retarget_cannot_renew_an_original_route() {
        let mut pipeline = threaded_entry_fixture(false);
        pipeline.graph.for_each_instruction_mut(|instruction| {
            if instruction.initial_borrow_jump_target() == Some(Label(1)) {
                instruction.retarget(Operand::Forward(Label(2)));
            }
        });
        pipeline.thread_forward_jumps();
        assert!(
            super::OptimizationPipeline::capture_pre_copy_borrow_jump_entries(
                &pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
            )
            .is_empty()
        );
    }

    #[test]
    fn zero_distance_entry_cannot_reopen_cleanup_after_cold_movement() {
        for analysis_marker in [false, true] {
            let mut pipeline = threaded_entry_fixture(false);
            pipeline.thread_forward_jumps();
            let boundaries = pipeline.exception_owner_provenance.block_boundaries.clone();
            let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
                &mut pipeline.graph,
                &boundaries,
                &mut pipeline.borrow_threaded_jump_entries,
            );
            assert_eq!(entries.get(&Label(0)), Some(&Label(2)));
            let mut items = pipeline.graph.into_items();
            let cleanup = items
                .iter()
                .position(|item| matches!(item, Item::Label(Label(2))))
                .unwrap();
            // Model the reordered warm arrays after intervening cold arrays move away.
            items.drain(4..cleanup);
            if analysis_marker {
                let mut marker =
                    Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
                marker.jump_policy.mark_borrow_empty_block();
                items.insert(4, Item::Instruction(marker));
            }
            assert_eq!(
                borrowed_cleanup_after_entries(items, &entries, &boundaries),
                LOAD_FAST
            );
        }
    }

    #[test]
    fn copied_prefix_cannot_inherit_a_consumed_threaded_entry() {
        let mut pipeline = threaded_entry_fixture(false);
        pipeline.thread_forward_jumps();
        let boundaries = pipeline.exception_owner_provenance.block_boundaries.clone();
        let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
            &mut pipeline.graph,
            &boundaries,
            &mut pipeline.borrow_threaded_jump_entries,
        );
        assert!(pipeline.graph.iter_items().all(|item| !matches!(item,
            Item::Instruction(instruction) if instruction.initial_borrow_jump_target().is_some())));
        let mut items = pipeline.graph.into_items();
        items.insert(1, Item::Label(Label(4)));
        assert_eq!(
            borrowed_cleanup_after_entries(items, &entries, &boundaries),
            LOAD_FAST
        );
    }

    #[test]
    fn threaded_origin_snapshot_rejects_any_changed_instruction_or_policy() {
        for change in 0..3 {
            let mut pipeline = threaded_entry_fixture(false);
            pipeline.thread_forward_jumps();
            assert!(!pipeline.borrow_threaded_jump_entries.is_empty());
            pipeline.graph.for_each_instruction_mut(|instruction| {
                if instruction.opcode == JUMP_FORWARD
                    && instruction.operand() == Operand::Forward(Label(2))
                {
                    match change {
                        0 => instruction.retarget(Operand::Forward(Label(3))),
                        1 => instruction.add_pre_fold_extra_instructions(1),
                        _ => {
                            instruction.location = SourceLocation {
                                line: 1,
                                end_line: 1,
                                column: 0,
                                end_column: 1,
                            }
                        }
                    }
                }
            });
            let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
                &mut pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
                &mut pipeline.borrow_threaded_jump_entries,
            );
            assert!(entries.is_empty());
            assert!(pipeline.borrow_threaded_jump_entries.is_empty());
        }
    }

    #[test]
    fn threaded_origin_snapshot_checks_new_original_boundaries_and_consumes_proof() {
        let mut pipeline = threaded_entry_fixture(false);
        pipeline.thread_forward_jumps();
        let source_block = pipeline.graph.label_block(Label(0)).unwrap();
        pipeline.graph.insert_label(source_block, 3, Label(4));
        pipeline
            .exception_owner_provenance
            .block_boundaries
            .insert(Label(4));
        let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
            &mut pipeline.borrow_threaded_jump_entries,
        );
        assert!(entries.is_empty());
        assert!(pipeline.borrow_threaded_jump_entries.is_empty());
    }

    #[test]
    fn threaded_origin_snapshot_requires_captured_label_identity() {
        let mut pipeline = threaded_entry_fixture(false);
        pipeline.thread_forward_jumps();
        let mut items = pipeline.graph.into_items();
        for item in &mut items {
            if matches!(item, Item::Label(Label(0))) {
                *item = Item::Label(Label(4));
            }
        }
        pipeline.graph =
            crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
        pipeline
            .exception_owner_provenance
            .block_boundaries
            .remove(&Label(0));
        pipeline
            .exception_owner_provenance
            .block_boundaries
            .insert(Label(4));
        let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
            &mut pipeline.borrow_threaded_jump_entries,
        );
        assert!(entries.is_empty());
        assert!(pipeline.borrow_threaded_jump_entries.is_empty());
    }

    fn threaded_owner_split_fixture() -> crate::assembler::OptimizationPipeline {
        let mut pipeline = threaded_entry_fixture(false);
        pipeline
            .exception_regions
            .push(crate::assembler::ExceptionRegion {
                is_scope_fallback: false,
                priority: crate::assembler::ir::ExceptionRegionPriority::OriginalRange(
                    Label(0),
                    Label(1),
                ),
                start: Label(0),
                end: Label(1),
                target: Label(2),
                depth: 0,
                preserve_lasti: false,
            });
        assert!(pipeline.thread_forward_jumps());
        assert_eq!(pipeline.next_label, 6);
        pipeline
    }

    #[test]
    fn threaded_origin_survives_physical_split_at_captured_owner_labels() {
        let mut pipeline = threaded_owner_split_fixture();
        let before = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
        assert_eq!(
            pipeline.graph.label_block(Label(0)),
            pipeline.graph.label_block(Label(4))
        );
        pipeline
            .graph
            .normalize_blocks(&pipeline.preserved_block_boundaries);
        assert_ne!(
            pipeline.graph.label_block(Label(0)),
            pipeline.graph.label_block(Label(4))
        );
        assert_eq!(
            pipeline.graph.iter_items().copied().collect::<Vec<_>>(),
            before
        );
        let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
            &mut pipeline.borrow_threaded_jump_entries,
        );
        assert_eq!(entries.get(&Label(4)), Some(&Label(2)));
        assert!(pipeline.borrow_threaded_jump_entries.is_empty());
    }

    #[test]
    fn threaded_origin_rejects_owner_label_moved_across_an_instruction() {
        let mut pipeline = threaded_owner_split_fixture();
        let mut items = pipeline.graph.into_items();
        let owner = items
            .iter()
            .position(|item| *item == Item::Label(Label(4)))
            .unwrap();
        let label = items.remove(owner);
        items.insert(owner - 1, label);
        pipeline.graph =
            crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
        let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
            &mut pipeline.borrow_threaded_jump_entries,
        );
        assert!(entries.is_empty());
    }

    #[test]
    fn threaded_origin_rejects_an_uncaptured_internal_label() {
        let mut pipeline = threaded_owner_split_fixture();
        let source = pipeline.graph.label_block(Label(0)).unwrap();
        pipeline.graph.insert_label(source, 2, Label(6));
        pipeline
            .graph
            .normalize_blocks(&pipeline.preserved_block_boundaries);
        let entries = super::OptimizationPipeline::take_pre_copy_borrow_entries_with_threaded(
            &mut pipeline.graph,
            &pipeline.exception_owner_provenance.block_boundaries,
            &mut pipeline.borrow_threaded_jump_entries,
        );
        assert!(entries.is_empty());
    }

    fn authored_finally_break_fixture(reachable: bool) -> crate::assembler::OptimizationPipeline {
        use crate::target::opcodes::{CALL, PUSH_NULL};

        let mut jump = Instruction::new(
            JUMP_FORWARD,
            Operand::Forward(Label(2)),
            SourceLocation {
                line: 1,
                end_line: 1,
                column: 0,
                end_column: 5,
            },
            None,
        );
        jump.fusion_policy.mark_authored_break();
        jump.jump_policy.prevent_threading_target();
        let mut pop_block = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
        pop_block.fusion_policy.mark_converted_pop_block();
        let call = Instruction::new(CALL, Operand::Value(0), jump.location, None);
        let items = vec![
            Item::Label(Label(0)),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(
                POP_JUMP_IF_TRUE,
                Operand::Forward(if reachable { Label(1) } else { Label(5) }),
            ),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(Label(1)),
            Item::Instruction(jump),
            Item::Label(Label(4)),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(Label(2)),
            instruction(JUMP_FORWARD, Operand::Forward(Label(3))),
            Item::Label(Label(3)),
            Item::Instruction(pop_block),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(PUSH_NULL, Operand::Value(0)),
            Item::Instruction(call),
            instruction(POP_TOP, Operand::Value(0)),
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(POP_TOP, Operand::Value(0)),
            instruction(JUMP_FORWARD, Operand::Forward(Label(5))),
            Item::Label(Label(5)),
            instruction(LOAD_CONST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];
        let mut assembler = crate::assembler::Assembler::from_linear_items(items);
        assembler.next_label = 6;
        assembler.exception_owner_provenance.block_boundaries = (0..6).map(Label).collect();
        let mut pipeline = assembler.into_optimization_pipeline();
        pipeline.borrow_finally_entries.insert(Label(3), None);
        pipeline.borrow_unreachable_blocks.insert(Label(3));
        pipeline.borrow_jump_entry_labels.insert(Label(3));
        pipeline.record_borrow_cleanup_calls();
        pipeline
    }

    fn authored_finally_load_opcode(
        mut items: Vec<Item>,
        entries: &crate::assembler::ir::AuthoredFinallyBreakEntries,
        calls: &FxHashMap<Label, Option<crate::assembler::ir::BorrowCleanupCall>>,
        boundaries: &FxHashSet<Label>,
        unreachable: &FxHashSet<Label>,
    ) -> crate::target::Opcode {
        super::OptimizationPipeline::optimize_load_fast(
            &mut items,
            &[],
            super::BorrowEntryLabels {
                unreachable,
                loop_exits: &FxHashMap::default(),
                finally_entries: calls,
                finally_jump_sources: &FxHashMap::default(),
                jump_entries: &FxHashSet::from_iter([Label(3)]),
                original_boundaries: boundaries,
                pre_copy_jump_entries: &FxHashMap::default(),
                authored_finally_break_entries: entries,
            },
            &FxHashSet::default(),
            &FxHashSet::default(),
            false,
        );
        items
            .iter()
            .find_map(|item| match item {
                Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_BORROW) =>
                {
                    Some(instruction.opcode)
                }
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn copied_setup_finally_body_revokes_an_already_threaded_borrow_entry() {
        use crate::target::opcodes::{CALL, PUSH_NULL};

        for change in 0..3 {
            let mut pipeline = authored_finally_break_fixture(true);
            let original = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
            let head = original
                .iter()
                .position(|item| *item == Item::Label(Label(3)))
                .unwrap();
            let pop = original[head + 1];
            let call = original
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction) if instruction.opcode == CALL => {
                        Some(*instruction)
                    }
                    _ => None,
                })
                .unwrap();
            let nop = Instruction::new(
                NOP,
                Operand::Value(0),
                SourceLocation {
                    line: 2,
                    end_line: 2,
                    ..call.location
                },
                None,
            );
            let mut items = original[..head].to_vec();
            items.extend([
                Item::Label(Label(3)),
                Item::Label(Label(6)),
                pop,
                Item::Label(Label(7)),
                Item::Instruction(nop),
                Item::Label(Label(8)),
                Item::Label(Label(9)),
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(PUSH_NULL, Operand::Value(0)),
                Item::Instruction(call),
                instruction(POP_TOP, Operand::Value(0)),
                Item::Label(Label(10)),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
                Item::Label(Label(5)),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]);
            pipeline.graph =
                crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
            pipeline.next_label = 12;
            pipeline.borrow_finally_entries = [(Label(6), None)].into_iter().collect();
            pipeline.borrow_unreachable_blocks = [Label(6)].into_iter().collect();
            pipeline.borrow_jump_entry_labels = [Label(6)].into_iter().collect();
            pipeline.preserved_block_boundaries.insert(Label(6));
            pipeline
                .exception_owner_provenance
                .setup_nop_ranges
                .push((Label(7), Label(8)));
            pipeline
                .exception_owner_provenance
                .setups
                .push((Label(9), Some(nop.location)));
            if change == 2 {
                pipeline.borrow_loop_exit_blocks.insert(
                    Label(6),
                    crate::assembler::BorrowLoopExitPolicy {
                        allow_surviving_break: false,
                        preserve_small_exit: false,
                        cleanup_call: None,
                    },
                );
            }
            let origins = pipeline.record_borrow_cleanup_calls_with_setups();
            if change == 2 {
                assert!(origins.is_empty());
                assert!(pipeline.borrow_block_aliases.is_empty());
                assert_eq!(
                    pipeline.borrow_loop_exit_blocks[&Label(6)].cleanup_call,
                    None
                );
                continue;
            }
            assert_eq!(origins.len(), 1);
            pipeline.thread_forward_jumps();
            let entries = super::OptimizationPipeline::take_pre_copy_authored_finally_break_entries(
                &pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
                &mut pipeline.borrow_authored_finally_break_origins,
            );
            assert_eq!(entries[&Label(1)].target, Label(3));
            let mut items = pipeline.graph.into_items();
            items.retain(|item| {
                !matches!(item, Item::Instruction(instruction)
                if instruction.fusion_policy.is_converted_pop_block())
            });
            // A late copy keeps the call's spelling and location but replaces the
            // protected body's label, invalidating the earlier setup certificate.
            if change == 1 {
                let label = items
                    .iter_mut()
                    .find(|item| **item == Item::Label(Label(9)))
                    .unwrap();
                *label = Item::Label(Label(11));
            }
            super::OptimizationPipeline::retain_setup_finally_origins(
                &items,
                &origins,
                &mut pipeline.borrow_finally_entries,
                &mut pipeline.borrow_block_aliases,
            );
            assert_eq!(
                pipeline.borrow_block_aliases.contains(&Label(6)),
                change == 0
            );
            if change == 1 {
                assert_eq!(
                    pipeline.borrow_finally_entries,
                    [(Label(6), None)].into_iter().collect()
                );
            }
            super::OptimizationPipeline::optimize_load_fast(
                &mut items,
                &[],
                super::BorrowEntryLabels {
                    unreachable: &pipeline.borrow_unreachable_blocks,
                    loop_exits: &FxHashMap::default(),
                    finally_entries: &pipeline.borrow_finally_entries,
                    finally_jump_sources: &FxHashMap::default(),
                    jump_entries: &pipeline.borrow_jump_entry_labels,
                    original_boundaries: &pipeline.exception_owner_provenance.block_boundaries,
                    pre_copy_jump_entries: &FxHashMap::default(),
                    authored_finally_break_entries: &entries,
                },
                &pipeline.preserved_block_boundaries,
                &pipeline.borrow_block_aliases,
                false,
            );
            let opcode = items
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction)
                        if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_BORROW) =>
                    {
                        Some(instruction.opcode)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                opcode,
                if change == 0 {
                    LOAD_FAST_BORROW
                } else {
                    LOAD_FAST
                }
            );
        }
    }

    #[test]
    fn authored_finally_break_requires_an_original_normal_path() {
        for reachable in [false, true] {
            let mut pipeline = authored_finally_break_fixture(reachable);
            assert_eq!(
                pipeline
                    .borrow_authored_finally_break_origins
                    .snapshots
                    .contains_key(&Label(1)),
                reachable,
            );
            // The real pipeline records the intermediate jump-only array as
            // empty after the original authored route has been captured.
            pipeline.record_borrow_empty_jump_heads();
            let intermediate = pipeline.graph.label_block(Label(2)).unwrap();
            assert!(
                pipeline.graph[intermediate]
                    .last_instruction()
                    .unwrap()
                    .jump_policy
                    .borrow_empty_block()
            );
            pipeline.thread_forward_jumps();
            let entries = super::OptimizationPipeline::take_pre_copy_authored_finally_break_entries(
                &pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
                &mut pipeline.borrow_authored_finally_break_origins,
            );
            assert_eq!(entries.contains_key(&Label(1)), reachable);
            assert!(
                pipeline
                    .borrow_authored_finally_break_origins
                    .snapshots
                    .is_empty()
            );
            assert!(
                pipeline
                    .borrow_authored_finally_break_origins
                    .routes
                    .is_empty()
            );
            if reachable {
                assert_eq!(entries[&Label(1)].target, Label(3),);
                assert_eq!(
                    authored_finally_load_opcode(
                        pipeline.graph.into_items(),
                        &entries,
                        &pipeline.borrow_finally_entries,
                        &pipeline.exception_owner_provenance.block_boundaries,
                        &pipeline.borrow_unreachable_blocks,
                    ),
                    LOAD_FAST_BORROW,
                );
            }
        }
    }

    #[test]
    fn authored_finally_break_retains_only_captured_owner_labels() {
        for new_original_boundary in [false, true] {
            let mut pipeline = authored_finally_break_fixture(true);
            pipeline
                .exception_regions
                .push(crate::assembler::ExceptionRegion {
                    is_scope_fallback: false,
                    priority: crate::assembler::ir::ExceptionRegionPriority::OriginalRange(
                        Label(1),
                        Label(2),
                    ),
                    start: Label(1),
                    end: Label(2),
                    target: Label(5),
                    depth: 0,
                    preserve_lasti: false,
                });
            pipeline.thread_forward_jumps();
            assert_eq!(pipeline.next_label, 8);
            let before = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
            pipeline
                .graph
                .normalize_blocks(&pipeline.preserved_block_boundaries);
            assert_eq!(
                pipeline.graph.iter_items().copied().collect::<Vec<_>>(),
                before
            );
            if new_original_boundary {
                pipeline
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(Label(6));
            }
            let entries = super::OptimizationPipeline::take_pre_copy_authored_finally_break_entries(
                &pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
                &mut pipeline.borrow_authored_finally_break_origins,
            );
            if new_original_boundary {
                assert!(entries.is_empty());
            } else {
                assert_eq!(entries[&Label(6)].original_source, Label(1));
                assert_eq!(
                    authored_finally_load_opcode(
                        pipeline.graph.into_items(),
                        &entries,
                        &pipeline.borrow_finally_entries,
                        &pipeline.exception_owner_provenance.block_boundaries,
                        &pipeline.borrow_unreachable_blocks,
                    ),
                    LOAD_FAST_BORROW,
                );
            }
        }
    }

    #[test]
    fn authored_finally_break_rejects_changed_original_or_threaded_spans() {
        for after_threading in [false, true] {
            for change in 0..3 {
                let mut pipeline = authored_finally_break_fixture(true);
                if after_threading {
                    pipeline.thread_forward_jumps();
                }
                pipeline.graph.for_each_instruction_mut(|instruction| {
                    if instruction.fusion_policy.is_authored_break() {
                        match change {
                            0 => instruction.retarget(Operand::Forward(Label(5))),
                            1 => instruction.location.column += 1,
                            _ => instruction.add_pre_fold_extra_instructions(1),
                        }
                    }
                });
                if !after_threading {
                    pipeline.thread_forward_jumps();
                }
                let entries =
                    super::OptimizationPipeline::take_pre_copy_authored_finally_break_entries(
                        &pipeline.graph,
                        &pipeline.exception_owner_provenance.block_boundaries,
                        &mut pipeline.borrow_authored_finally_break_origins,
                    );
                assert!(entries.is_empty());
                assert!(
                    pipeline
                        .borrow_authored_finally_break_origins
                        .snapshots
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn authored_finally_break_requires_an_explicit_route_to_a_captured_call() {
        for missing_call in [false, true] {
            let mut pipeline = authored_finally_break_fixture(true);
            pipeline.graph.for_each_instruction_mut(|instruction| {
                if (missing_call && instruction.opcode == crate::target::opcodes::CALL)
                    || (!missing_call
                        && instruction.opcode == JUMP_FORWARD
                        && instruction.operand() == Operand::Forward(Label(3)))
                {
                    instruction.opcode = NOP;
                    instruction.set_operand(Operand::Value(0));
                }
            });
            pipeline.record_borrow_cleanup_calls();
            assert!(
                pipeline
                    .borrow_authored_finally_break_origins
                    .routes
                    .is_empty()
            );
            pipeline.thread_forward_jumps();
            let entries = super::OptimizationPipeline::take_pre_copy_authored_finally_break_entries(
                &pipeline.graph,
                &pipeline.exception_owner_provenance.block_boundaries,
                &mut pipeline.borrow_authored_finally_break_origins,
            );
            assert!(entries.is_empty());
        }
    }

    #[test]
    fn authored_finally_break_keeps_final_entry_barriers() {
        use crate::target::opcodes::CALL;

        for rejection in 0..7 {
            let mut pipeline = authored_finally_break_fixture(true);
            pipeline.thread_forward_jumps();
            let mut entries =
                super::OptimizationPipeline::take_pre_copy_authored_finally_break_entries(
                    &pipeline.graph,
                    &pipeline.exception_owner_provenance.block_boundaries,
                    &mut pipeline.borrow_authored_finally_break_origins,
                );
            assert!(entries.contains_key(&Label(1)));
            let mut items = pipeline.graph.into_items();
            let jump = items.iter().position(|item| matches!(item,
                Item::Instruction(instruction) if instruction.fusion_policy.is_authored_break()
            )).unwrap();
            let cleanup = items
                .iter()
                .position(|item| *item == Item::Label(Label(3)))
                .unwrap();
            match rejection {
                0 => entries.clear(),
                1 => items.insert(jump, Item::Label(Label(6))),
                2 => {
                    items.drain(jump + 1..cleanup);
                }
                3 => {
                    for item in &mut items {
                        if let Item::Instruction(instruction) = item
                            && instruction.opcode == CALL
                        {
                            instruction.location.column += 1;
                        }
                    }
                }
                4 => {
                    items.insert(cleanup, Item::Label(Label(6)));
                    pipeline.borrow_unreachable_blocks.insert(Label(6));
                }
                5 => {
                    for item in &mut items {
                        if let Item::Instruction(instruction) = item
                            && instruction.opcode == LOAD_FAST
                        {
                            instruction.local_policy =
                                crate::assembler::ir::LocalPolicy::for_load(true, true);
                        }
                    }
                }
                _ => {
                    items.insert(cleanup, Item::Label(Label(6)));
                    pipeline.borrow_unreachable_blocks.insert(Label(6));
                    pipeline.borrow_finally_entries.insert(Label(6), None);
                }
            }
            assert_eq!(
                authored_finally_load_opcode(
                    items,
                    &entries,
                    &pipeline.borrow_finally_entries,
                    &pipeline.exception_owner_provenance.block_boundaries,
                    &pipeline.borrow_unreachable_blocks,
                ),
                LOAD_FAST,
            );
        }
    }
    #[test]
    fn for_iter_original_cleanup_requires_the_explicit_original_entry() {
        use crate::assembler::{Instruction, Item, Label, Operand, SourceLocation};
        use crate::target::opcodes::{END_FOR, FOR_ITER, JUMP_FORWARD, LOAD_CONST, POP_ITER};

        let empty = Label(0);
        let target = Label(1);
        let instruction =
            |opcode, operand| Instruction::new(opcode, operand, SourceLocation::NONE, None);
        for change in [
            "earlier empty array",
            "ordinary entry",
            "empty target",
            "later empty alias",
            "non-for edge",
            "new target",
            "instruction before target",
            "non-cleanup target",
        ] {
            let mut edge = instruction(FOR_ITER, Operand::Forward(target));
            let mut items = vec![
                Item::Label(empty),
                Item::Label(target),
                Item::Instruction(instruction(END_FOR, Operand::Value(0))),
                Item::Instruction(instruction(POP_ITER, Operand::Value(0))),
            ];
            let mut boundaries = rustc_hash::FxHashSet::from_iter([target]);
            let mut unreachable = rustc_hash::FxHashSet::from_iter([empty]);
            match change {
                "earlier empty array" => {}
                "ordinary entry" => {
                    unreachable.clear();
                }
                "empty target" => {
                    unreachable.insert(target);
                }
                "later empty alias" => {
                    items.insert(2, Item::Label(Label(2)));
                    unreachable.insert(Label(2));
                }
                "non-for edge" => {
                    edge = instruction(JUMP_FORWARD, Operand::Forward(target));
                }
                "new target" => {
                    boundaries.clear();
                }
                "instruction before target" => {
                    items.insert(
                        1,
                        Item::Instruction(instruction(LOAD_CONST, Operand::Value(0))),
                    );
                }
                "non-cleanup target" => {
                    items[2] = Item::Instruction(instruction(LOAD_CONST, Operand::Value(0)));
                }
                _ => unreachable!(),
            }
            assert_eq!(
                super::for_iter_enters_original_cleanup(
                    &edge,
                    target,
                    &items,
                    &boundaries,
                    &unreachable
                ),
                matches!(change, "earlier empty array" | "ordinary entry"),
                "{change}"
            );
        }
    }
}

#[cfg(test)]
mod retained_finally_origin_tests {
    use rustc_hash::{FxHashMap, FxHashSet};

    use super::{BorrowEntryLabels, OptimizationPipeline};
    use crate::assembler::block_graph::BlockGraph;
    use crate::assembler::{
        Assembler, Instruction, Item, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, LOAD_CONST,
        LOAD_FAST, LOAD_FAST_BORROW, Label, NOP, Operand, POP_TOP, RETURN_VALUE, SourceLocation,
    };

    fn instruction(opcode: crate::target::Opcode, operand: Operand) -> Item {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            SourceLocation::new(20, 20, 0, 8),
            None,
        ))
    }

    #[derive(Clone, Copy, Debug)]
    enum Change {
        None,
        EmptyOriginalTarget,
        MergedSourceAlias,
        LateUnrelatedTargetMarker,
        ErasedJumpNop,
    }

    #[test]
    fn retained_finally_source_aliases_require_one_surviving_original_array() {
        use crate::target::opcodes::{CALL, POP_JUMP_IF_FALSE, PUSH_NULL, STORE_FAST};

        for change in [
            "ordinary call",
            "original exit boundary",
            "second source boundary",
            "preserved exit boundary",
            "shared exit",
            "conditional source",
            "late boundary",
            "changed prefix",
        ] {
            let source = Label(0);
            let alias = Label(1);
            let cold = Label(2);
            let target = Label(3);
            let finally = Label(4);
            let mut prefix = vec![
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(PUSH_NULL, Operand::Value(0)),
                instruction(CALL, Operand::Value(0)),
                instruction(POP_TOP, Operand::Value(0)),
            ];
            if change == "conditional source" {
                prefix.splice(
                    0..0,
                    [
                        instruction(LOAD_FAST, Operand::Value(1)),
                        instruction(POP_JUMP_IF_FALSE, Operand::Forward(cold)),
                    ],
                );
            }
            let cleanup = [
                Item::Label(target),
                Item::Label(finally),
                instruction(LOAD_FAST, Operand::Value(1)),
                instruction(POP_TOP, Operand::Value(0)),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ];
            let source_alias = Label(5);
            let mut original = vec![Item::Label(source_alias), Item::Label(source)];
            original.extend(prefix.iter().copied());
            original.extend([
                Item::Label(alias),
                instruction(JUMP_FORWARD, Operand::Forward(target)),
                Item::Label(cold),
            ]);
            if change == "shared exit" {
                original.push(instruction(
                    JUMP_BACKWARD_NO_INTERRUPT,
                    Operand::Backward(alias),
                ));
            } else {
                original.extend([
                    instruction(LOAD_CONST, Operand::Value(0)),
                    instruction(RETURN_VALUE, Operand::Value(0)),
                ]);
            }
            original.extend(cleanup);
            let mut assembler = Assembler::from_linear_items(original);
            assembler.next_label = 6;
            assembler.exception_owner_provenance.block_boundaries =
                [source, cold, target].into_iter().collect();
            assembler
                .exception_owner_provenance
                .source_boundaries
                .insert(source);
            assembler
                .exception_owner_provenance
                .compound_try_owner_exits
                .push((alias, target));
            if change == "second source boundary" {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(source_alias);
            }
            if change == "original exit boundary" {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(alias);
            }
            if change == "preserved exit boundary" {
                assembler.preserved_block_boundaries.insert(alias);
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.borrow_finally_entries.insert(finally, None);
            pipeline.borrow_unreachable_blocks.insert(finally);
            pipeline.borrow_jump_entry_labels.insert(finally);
            let origins = pipeline.capture_retained_finally_origins();
            let captured = matches!(change, "ordinary call" | "late boundary" | "changed prefix");
            assert_eq!(origins.len(), usize::from(captured), "{change}");
            if !captured {
                continue;
            }
            assert_eq!(origins[0].source_exit_alias, Some(alias));
            // Remove only the original normal jump and move its cold neighbor.
            // The range-end alias and complete source array must still survive.
            let mut live = vec![Item::Label(source_alias), Item::Label(source)];
            if change == "changed prefix" {
                *prefix.last_mut().unwrap() = instruction(STORE_FAST, Operand::Value(0));
            }
            live.extend(prefix);
            live.push(Item::Label(alias));
            live.extend(cleanup);
            live.extend([
                Item::Label(cold),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]);
            if change == "late boundary" {
                pipeline.preserved_block_boundaries.insert(alias);
            }
            pipeline.graph = BlockGraph::from_items(live, &pipeline.preserved_block_boundaries);
            let retained = pipeline.retained_finally_entries(&origins);
            assert_eq!(
                retained.get(&alias).copied(),
                (change == "ordinary call").then_some(target),
                "{change}",
            );
        }
    }

    #[test]
    fn retained_finally_entries_preserve_original_array_and_late_barrier_identity() {
        for change in [
            Change::None,
            Change::EmptyOriginalTarget,
            Change::MergedSourceAlias,
            Change::LateUnrelatedTargetMarker,
            Change::ErasedJumpNop,
        ] {
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(Label(2)),
                SourceLocation::new(10, 10, 0, 8),
                None,
            );
            jump.jump_policy.retain_nop_for_direct_target();
            let mut pop_block =
                Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
            pop_block.fusion_policy.mark_converted_pop_block();
            let mut original = vec![
                Item::Label(Label(0)),
                Item::Instruction(jump),
                Item::Label(Label(1)),
                Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(Label(2)),
                    SourceLocation::NONE,
                    None,
                )),
                Item::Label(Label(2)),
                Item::Label(Label(3)),
                Item::Instruction(pop_block),
            ];
            // This separates the disappearing POP_BLOCK from the live cleanup.
            // Physical label merging must not turn that empty array into an entry.
            if matches!(change, Change::EmptyOriginalTarget) {
                original.push(Item::Label(Label(4)));
            }
            original.extend([
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(POP_TOP, Operand::Value(0)),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]);
            let mut assembler = Assembler::from_linear_items(original);
            assembler.next_label = 5;
            assembler.exception_owner_provenance.block_boundaries =
                [Label(0), Label(1), Label(2)].into_iter().collect();
            if matches!(change, Change::EmptyOriginalTarget) {
                assembler
                    .exception_owner_provenance
                    .block_boundaries
                    .insert(Label(4));
            }
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.borrow_finally_entries.insert(Label(3), None);
            pipeline.borrow_unreachable_blocks.insert(Label(3));
            pipeline.borrow_jump_entry_labels.insert(Label(3));
            pipeline.record_borrow_finally_jump_sources();
            let origins = pipeline.capture_retained_finally_origins();

            // Model the final physical layout: the cold route moved after the
            // return, POP_BLOCK disappeared, and the original jump kept its line.
            jump.opcode = NOP;
            jump.set_operand(Operand::Value(0));
            let mut items = vec![Item::Label(Label(0))];
            if matches!(change, Change::MergedSourceAlias) {
                items.insert(0, Item::Label(Label(4)));
            }
            if !matches!(change, Change::ErasedJumpNop) {
                items.push(Item::Instruction(jump));
            }
            items.extend([Item::Label(Label(2)), Item::Label(Label(3))]);
            if matches!(change, Change::EmptyOriginalTarget) {
                items.push(Item::Label(Label(4)));
            }
            items.extend([
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(POP_TOP, Operand::Value(0)),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
                Item::Label(Label(1)),
                instruction(JUMP_BACKWARD_NO_INTERRUPT, Operand::Backward(Label(2))),
            ]);
            pipeline.graph = BlockGraph::from_items(items, &FxHashSet::default());
            let retained = pipeline.retained_finally_entries(&origins);
            let mut items = pipeline.graph.iter_items().copied().collect::<Vec<_>>();
            if matches!(change, Change::LateUnrelatedTargetMarker) {
                let load = items
                    .iter()
                    .position(|item| matches!(item, Item::Instruction(value) if value.opcode == LOAD_FAST))
                    .unwrap();
                items.insert(load, Item::Label(Label(4)));
                pipeline.borrow_unreachable_blocks.insert(Label(4));
                // Certify a generic jump entry, but not this finally array. The
                // newly collapsed marker must still veto the retained NOP edge.
                pipeline.borrow_jump_entry_labels.insert(Label(4));
            }
            OptimizationPipeline::optimize_load_fast_with_retained_entries(
                &mut items,
                &[],
                BorrowEntryLabels {
                    unreachable: &pipeline.borrow_unreachable_blocks,
                    loop_exits: &FxHashMap::default(),
                    finally_entries: &pipeline.borrow_finally_entries,
                    finally_jump_sources: &pipeline.borrow_finally_jump_sources,
                    jump_entries: &pipeline.borrow_jump_entry_labels,
                    original_boundaries: &pipeline.exception_owner_provenance.block_boundaries,
                    pre_copy_jump_entries: &FxHashMap::default(),
                    authored_finally_break_entries: &FxHashMap::default(),
                },
                &FxHashSet::default(),
                &FxHashSet::default(),
                false,
                &retained,
            );
            let actual = items
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(value)
                        if matches!(value.opcode, LOAD_FAST | LOAD_FAST_BORROW) =>
                    {
                        Some(value.opcode)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                actual,
                if matches!(change, Change::None) {
                    LOAD_FAST_BORROW
                } else {
                    LOAD_FAST
                },
                "{change:?}",
            );
        }
    }

    #[test]
    fn removed_borrow_barrier_keeps_its_successor_entry_separate() {
        use crate::assembler::BorrowFallthrough;
        use crate::target::opcodes::{POP_JUMP_IF_TRUE, RAISE_VARARGS};

        for direct_successor in [false, true] {
            let entry = Label(0);
            let empty = Label(1);
            let successor = Label(2);
            let mut jump = Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(successor),
                SourceLocation::NONE,
                None,
            );
            jump.no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::Barrier);
            let mut assembler = Assembler::from_linear_items(vec![
                Item::Label(entry),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(
                    POP_JUMP_IF_TRUE,
                    Operand::Forward(if direct_successor { successor } else { empty }),
                ),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RAISE_VARARGS, Operand::Value(1)),
                Item::Label(empty),
                Item::Instruction(jump),
                Item::Label(successor),
                instruction(LOAD_FAST, Operand::Value(0)),
                instruction(POP_TOP, Operand::Value(0)),
                instruction(LOAD_CONST, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]);
            assembler.next_label = 3;
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.remove_redundant_forward_jumps(true);
            let boundaries = pipeline
                .exception_owner_provenance
                .borrow_only_block_boundaries
                .clone();
            let mut items = pipeline.graph.into_items();
            super::OptimizationPipeline::optimize_load_fast(
                &mut items,
                &[],
                super::BorrowEntryLabels {
                    unreachable: &pipeline.borrow_unreachable_blocks,
                    loop_exits: &FxHashMap::default(),
                    finally_entries: &FxHashMap::default(),
                    finally_jump_sources: &FxHashMap::default(),
                    jump_entries: &FxHashSet::default(),
                    original_boundaries: &FxHashSet::default(),
                    pre_copy_jump_entries: &FxHashMap::default(),
                    authored_finally_break_entries: &FxHashMap::default(),
                },
                &boundaries,
                &FxHashSet::default(),
                false,
            );
            let actual = items.iter().find_map(|item| match item {
                Item::Instruction(instruction)
                    if matches!(instruction.opcode, LOAD_FAST | LOAD_FAST_BORROW) =>
                {
                    Some(instruction.opcode)
                }
                _ => None,
            });
            assert_eq!(
                actual,
                Some(if direct_successor {
                    LOAD_FAST_BORROW
                } else {
                    LOAD_FAST
                }),
                "direct successor: {direct_successor}",
            );
        }
    }
}
