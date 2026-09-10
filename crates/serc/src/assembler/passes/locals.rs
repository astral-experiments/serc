use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::super::ir::BorrowCleanupCall;
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
use crate::target::{is_unconditional_jump, opcodes::CALL};

/// Original block entries retained for CPython's final borrowing traversal.
#[derive(Clone, Copy)]
pub(in crate::assembler) struct BorrowEntryLabels<'a> {
    pub(super) unreachable: &'a FxHashSet<Label>,
    pub(super) loop_exits: &'a FxHashMap<Label, BorrowLoopExitPolicy>,
    pub(super) finally_entries: &'a FxHashMap<Label, Option<BorrowCleanupCall>>,
    pub(super) finally_jump_sources: &'a FxHashMap<Label, bool>,
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

        let ends_block = matches!(item, Item::Instruction(instruction) if
            !matches!(instruction.operand(), Operand::Value(_))
                || ends_scope(instruction.opcode)
                || instruction.opcode == NOT_TAKEN
                || (instruction.opcode == NOP && instruction.jump_policy.always_retains_nop())
                || instruction.jump_policy.borrow_empty_block()
                || instruction.no_location_policy.borrow_fallthrough()
                    == BorrowFallthrough::Barrier);
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
        let ends_block = matches!(item, Item::Instruction(instruction) if
            !matches!(instruction.operand(), Operand::Value(_))
                || ends_scope(instruction.opcode)
                || instruction.opcode == NOT_TAKEN
                || (instruction.opcode == NOP && instruction.jump_policy.always_retains_nop())
                || instruction.jump_policy.borrow_empty_block()
                || instruction.no_location_policy.borrow_fallthrough()
                    == BorrowFallthrough::Barrier);
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

impl OptimizationPipeline {
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
                Some((start, end - start, target, index))
            })
            .collect::<Vec<_>>();
        exception_regions.sort_unstable_by(
            |(left_start, left_span, _, left_index), (right_start, right_span, _, right_index)| {
                left_start
                    .cmp(right_start)
                    .then_with(|| right_span.cmp(left_span))
                    .then_with(|| right_index.cmp(left_index))
            },
        );
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
        for block in order.iter().copied() {
            let items = self.graph[block].items();
            let Some(last) = items.iter().rev().find_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            }) else {
                continue;
            };
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
    }

    /// Records nonempty cleanup targets before optimizer passes can merge their labels.
    pub(in crate::assembler) fn record_borrow_cleanup_calls(&mut self) {
        if self.borrow_loop_exit_blocks.is_empty() && self.borrow_finally_entries.is_empty() {
            return;
        }
        self.record_borrow_finally_jump_sources();
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
        let calls = self
            .borrow_loop_exit_blocks
            .keys()
            .chain(self.borrow_finally_entries.keys())
            .copied()
            .collect::<FxHashSet<_>>()
            .into_iter()
            .map(|label| {
                let call = (|| {
                    // Capture inside this original physical block. Flattening the whole
                    // graph could borrow a call from beyond a separate empty block.
                    let original = self.graph.label_block(label)?;
                    let items = self.graph[original].items();
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
                    for (index, instruction) in instructions.iter().enumerate() {
                        if instruction.opcode != CALL || instruction.location.line < 0 {
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
                policy.cleanup_call = entry.as_ref().map(|(call, _)| *call);
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
    }

    /// Mirrors CPython's final `optimize_load_fast` CFG pass.
    ///
    /// An owned local load can become a borrowed load only when its value is
    /// consumed in the same basic block, is not stored into another local, and
    /// remains supported by the original local until it is consumed.
    pub(in crate::assembler) fn optimize_load_fast(
        items: &mut [Item],
        exception_regions: &[ExceptionRegion],
        entries: BorrowEntryLabels<'_>,
        preserved_block_boundaries: &FxHashSet<Label>,
        borrow_block_aliases: &FxHashSet<Label>,
        assertion_side_exits: bool,
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
                    && last.no_location_policy.borrow_fallthrough()
                        != BorrowFallthrough::RetainedNopReachable;
                if !borrow_traversal_stops
                    && !folded_jump_has_no_fallthrough
                    && block_has_fallthrough(&[Item::Instruction(*last)])
                    && block + 1 < blocks.len()
                {
                    pending.push((block + 1, false));
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
                                                    at_target
                                                        && instruction.opcode == CALL
                                                        && instruction.operand()
                                                            == Operand::Value(call.argument)
                                                        && instruction.location == call.location
                                                }
                                            },
                                        )
                                    }
                            });
                    pending.push((
                        target,
                        shared_break || independent_loop_entry || cleanup_entry || finally_entry,
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
        fn fuse_pair(first: &Instruction, second: &Instruction) -> Option<Instruction> {
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
            if first_argument >= 16
                || second_argument >= 16
                || first.location.line != second.location.line
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
}
