use std::ops::Index;

use ruff_index::{IndexVec, newtype_index};
use rustc_hash::FxHashSet;
use smallvec::SmallVec;

use crate::target::opcodes::{
    COPY, IS_OP, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD, KW_NAMES, LOAD_CONST,
    LOAD_FAST, LOAD_FAST_BORROW, LOAD_FAST_CHECK, LOAD_FAST_LOAD_FAST, NOP, NOT_TAKEN,
    POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, RETURN_CONST,
    STORE_FAST, SWAP, TO_BOOL, UNARY_NOT,
};

#[cfg(test)]
use super::block_jump_target;
use super::{
    BorrowFallthrough, ExceptionPolicy, ExceptionRegion, Instruction, Item, Label, Operand,
    block_has_fallthrough, ends_scope, is_unconditional_jump,
};

/// A stable handle to a basic block.
///
/// Blocks live in an append-only arena. Reordering or removing a block from the
/// emitted order therefore never changes the identity of any other block.
#[newtype_index]
#[derive(Ord, PartialOrd)]
pub(super) struct BasicBlockId;

/// A temporary cursor into a block's item list.
///
/// Cursors are intentionally pass-local: inserting or removing an item can
/// shift later offsets in the same block, while the block identity stays
/// stable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ItemPosition {
    block: BasicBlockId,
    offset: usize,
}

impl ItemPosition {
    pub(super) const fn new(block: BasicBlockId, offset: usize) -> Self {
        Self { block, offset }
    }

    pub(super) const fn block(self) -> BasicBlockId {
        self.block
    }

    pub(super) const fn offset(self) -> usize {
        self.offset
    }
}

/// A mutable view of a block item that keeps labels opaque and immutable.
pub(super) enum ItemMut<'a> {
    Label,
    Instruction(&'a mut Instruction),
}

#[derive(Clone, Debug)]
pub(super) struct BasicBlock {
    items: Vec<Item>,
}

impl BasicBlock {
    fn new(items: Vec<Item>) -> Self {
        debug_assert!(!items.is_empty());
        Self { items }
    }

    pub(super) fn items(&self) -> &[Item] {
        &self.items
    }

    pub(super) fn first_label(&self) -> Option<Label> {
        self.items.iter().find_map(|item| match item {
            Item::Label(label) => Some(*label),
            Item::Instruction(_) => None,
        })
    }

    pub(super) fn last_instruction(&self) -> Option<&Instruction> {
        self.items.iter().rev().find_map(Item::as_instruction)
    }

    /// Finds the fallthrough and explicit target from the same final instruction.
    pub(super) fn successors(&self) -> (bool, Option<Label>) {
        let Some(instruction) = self.last_instruction() else {
            return (true, None);
        };
        let falls_through =
            !ends_scope(instruction.opcode) && !is_unconditional_jump(instruction.opcode);
        let target = instruction.jump_target();
        (falls_through, target)
    }
}

/// Conservative opcode and IR-policy summary for gating assembler passes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PipelineFeatures(u16);

impl PipelineFeatures {
    const FORWARD_JUMP: u16 = 1 << 0;
    const FORWARD_CONDITIONAL: u16 = 1 << 1;
    const SWAP_TRIGGER: u16 = 1 << 2;
    const FAST_LOAD: u16 = 1 << 3;
    const STORE_FAST_TRIGGER: u16 = 1 << 4;
    const BOOLEAN_CONVERSION: u16 = 1 << 5;
    const BORROW_EMPTY_BLOCK: u16 = 1 << 6;
    const JUMP: u16 = 1 << 7;
    const POP_CONDITIONAL: u16 = 1 << 8;
    const COPY_TOP: u16 = 1 << 9;
    const CHECKED_FAST_LOAD: u16 = 1 << 10;
    const IDENTITY_COMPARISON: u16 = 1 << 11;
    const NOT_TAKEN: u16 = 1 << 12;
    const ORIGINAL_FUSION_OWNER_CANDIDATE: u16 = 1 << 13;
    const CHECK_COPY: u16 = 1 << 14;
    const CHECK_FUSION_OWNER: u16 = 1 << 15;

    const FORWARD_FEATURES: u16 = Self::FORWARD_JUMP | Self::FORWARD_CONDITIONAL;
    const CHECK_FEATURES: u16 = Self::CHECK_COPY | Self::CHECK_FUSION_OWNER;

    /// Static features and the policy checks relevant to each semantic IR opcode.
    const BY_OPCODE: [u16; 256] = {
        let mut flags = [0; 256];
        flags[JUMP_BACKWARD.code() as usize] = Self::FORWARD_JUMP;
        flags[JUMP_BACKWARD_NO_INTERRUPT.code() as usize] = Self::FORWARD_JUMP;
        flags[JUMP_FORWARD.code() as usize] = Self::FORWARD_JUMP;
        flags[POP_JUMP_IF_FALSE.code() as usize] =
            Self::FORWARD_CONDITIONAL | Self::POP_CONDITIONAL;
        flags[POP_JUMP_IF_TRUE.code() as usize] = Self::FORWARD_CONDITIONAL | Self::POP_CONDITIONAL;
        flags[POP_JUMP_IF_NONE.code() as usize] = Self::POP_CONDITIONAL;
        flags[POP_JUMP_IF_NOT_NONE.code() as usize] = Self::POP_CONDITIONAL;
        flags[SWAP.code() as usize] = Self::SWAP_TRIGGER;
        flags[LOAD_FAST_CHECK.code() as usize] = Self::FAST_LOAD | Self::CHECKED_FAST_LOAD;
        flags[LOAD_FAST.code() as usize] = Self::FAST_LOAD;
        flags[LOAD_FAST_BORROW.code() as usize] = Self::FAST_LOAD;
        flags[LOAD_FAST_LOAD_FAST.code() as usize] = Self::FAST_LOAD;
        flags[STORE_FAST.code() as usize] = Self::STORE_FAST_TRIGGER;
        flags[TO_BOOL.code() as usize] = Self::BOOLEAN_CONVERSION;
        flags[UNARY_NOT.code() as usize] = Self::BOOLEAN_CONVERSION;
        flags[IS_OP.code() as usize] = Self::IDENTITY_COMPARISON;
        flags[COPY.code() as usize] = Self::CHECK_COPY;
        flags[NOT_TAKEN.code() as usize] = Self::NOT_TAKEN | Self::CHECK_FUSION_OWNER;
        flags[NOP.code() as usize] = Self::CHECK_FUSION_OWNER;
        flags[LOAD_CONST.code() as usize] = Self::CHECK_FUSION_OWNER;
        flags[RETURN_CONST.code() as usize] = Self::CHECK_FUSION_OWNER;
        flags[KW_NAMES.code() as usize] = Self::CHECK_FUSION_OWNER;
        flags
    };

    fn record(&mut self, instruction: &Instruction) {
        let opcode_features = Self::BY_OPCODE[usize::from(instruction.opcode.code())];
        let mut features = opcode_features & !(Self::FORWARD_FEATURES | Self::CHECK_FEATURES);
        if instruction.jump_target().is_some() {
            features |= Self::JUMP;
            if instruction.forward_target().is_some() {
                features |= opcode_features & Self::FORWARD_FEATURES;
            }
        }
        if opcode_features & Self::CHECK_COPY != 0 && instruction.value_argument() == Some(1) {
            features |= Self::COPY_TOP;
        }
        if opcode_features & Self::CHECK_FUSION_OWNER != 0 {
            let has_owner = if opcode_features & Self::NOT_TAKEN != 0 {
                matches!(
                    instruction.exception_policy(),
                    ExceptionPolicy::NoneUnchangedBooleanSingleton
                        | ExceptionPolicy::NonePresent
                        | ExceptionPolicy::NoneAbsent
                )
            } else {
                // Constant reachability can turn an unused indexed constant into a
                // NOP without changing the folded slots or its fallthrough policy.
                instruction.pre_fold_extra_instructions() == 2
                    && instruction.no_location_policy.borrow_fallthrough()
                        == BorrowFallthrough::Barrier
            };
            if has_owner {
                features |= Self::ORIGINAL_FUSION_OWNER_CANDIDATE;
            }
        }
        if instruction.jump_policy.borrow_empty_block() {
            features |= Self::BORROW_EMPTY_BLOCK;
        }
        self.0 |= features;
    }

    fn merge(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub(super) const fn has_forward_jump(self) -> bool {
        self.0 & Self::FORWARD_JUMP != 0
    }

    pub(super) const fn has_forward_conditional(self) -> bool {
        self.0 & Self::FORWARD_CONDITIONAL != 0
    }

    pub(super) const fn has_swap(self) -> bool {
        self.0 & Self::SWAP_TRIGGER != 0
    }

    pub(super) const fn has_fast_load(self) -> bool {
        self.0 & Self::FAST_LOAD != 0
    }

    pub(super) const fn has_checked_fast_load(self) -> bool {
        self.0 & Self::CHECKED_FAST_LOAD != 0
    }

    pub(super) const fn has_store_fast(self) -> bool {
        self.0 & Self::STORE_FAST_TRIGGER != 0
    }

    pub(super) const fn has_boolean_conversion(self) -> bool {
        self.0 & Self::BOOLEAN_CONVERSION != 0
    }

    pub(super) const fn has_identity_comparison(self) -> bool {
        self.0 & Self::IDENTITY_COMPARISON != 0
    }

    pub(super) const fn has_borrow_empty_block(self) -> bool {
        self.0 & Self::BORROW_EMPTY_BLOCK != 0
    }

    pub(super) const fn has_jump(self) -> bool {
        self.0 & Self::JUMP != 0
    }

    pub(super) const fn has_pop_conditional(self) -> bool {
        self.0 & Self::POP_CONDITIONAL != 0
    }

    pub(super) const fn has_copy_top(self) -> bool {
        self.0 & Self::COPY_TOP != 0
    }

    pub(super) const fn has_not_taken(self) -> bool {
        self.0 & Self::NOT_TAKEN != 0
    }

    /// Whether an authored marker or folded assertion could carry an original fusion owner.
    pub(super) const fn has_original_fusion_owner_candidate(self) -> bool {
        self.0 & Self::ORIGINAL_FUSION_OWNER_CANDIDATE != 0
    }
}

/// The assembler's block-aware intermediate representation.
///
/// `blocks` is an append-only arena and `order` is the current emission order.
/// Keeping those concepts separate lets CFG passes reorder or duplicate blocks
/// without invalidating identities captured by data-flow state.
#[derive(Clone, Debug)]
pub(super) struct BlockGraph {
    blocks: IndexVec<BasicBlockId, BasicBlock>,
    order: Vec<BasicBlockId>,
    label_blocks: Vec<Option<BasicBlockId>>,
    features: PipelineFeatures,
}

/// Traverses emitted items without maintaining a reverse block cursor.
struct GraphItems<'a> {
    graph: &'a BlockGraph,
    blocks: std::slice::Iter<'a, BasicBlockId>,
    items: std::slice::Iter<'a, Item>,
}

impl<'a> Iterator for GraphItems<'a> {
    type Item = &'a Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.items.next() {
                return Some(item);
            }
            self.items = self.graph[*self.blocks.next()?].items().iter();
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.items.len();
        (
            remaining,
            self.blocks.as_slice().is_empty().then_some(remaining),
        )
    }

    #[inline]
    fn nth(&mut self, mut n: usize) -> Option<Self::Item> {
        loop {
            let remaining = self.items.len();
            if let Some(item) = self.items.nth(n) {
                return Some(item);
            }
            n -= remaining;
            self.items = self.graph[*self.blocks.next()?].items().iter();
        }
    }

    #[inline]
    fn fold<B, F>(self, initial: B, mut fold: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        let Self {
            graph,
            blocks,
            items,
        } = self;
        let mut result = items.fold(initial, &mut fold);
        for block in blocks {
            result = graph[*block].items().iter().fold(result, &mut fold);
        }
        result
    }

    #[inline]
    fn any<F>(&mut self, mut predicate: F) -> bool
    where
        F: FnMut(Self::Item) -> bool,
    {
        loop {
            if self.items.any(&mut predicate) {
                return true;
            }
            let Some(block) = self.blocks.next() else {
                return false;
            };
            self.items = self.graph[*block].items().iter();
        }
    }
}

impl std::iter::FusedIterator for GraphItems<'_> {}

#[cfg(test)]
fn normalized_segment_lengths<'a>(
    items: impl IntoIterator<Item = &'a Item>,
    preserved_block_boundaries: &FxHashSet<Label>,
) -> (SmallVec<[usize; 16]>, PipelineFeatures) {
    normalized_segment_lengths_with_labels(items, preserved_block_boundaries, |_, _| {})
}

/// Visits each label after its normalized block's ordinal has been determined.
fn normalized_segment_lengths_with_labels<'a>(
    items: impl IntoIterator<Item = &'a Item>,
    preserved_block_boundaries: &FxHashSet<Label>,
    mut visit_label: impl FnMut(Label, usize),
) -> (SmallVec<[usize; 16]>, PipelineFeatures) {
    let mut lengths = SmallVec::new();
    let mut length = 0;
    let mut has_instruction = false;
    let mut features = PipelineFeatures::default();
    for item in items {
        match item {
            Item::Label(label) => {
                if has_instruction || (length > 0 && preserved_block_boundaries.contains(label)) {
                    lengths.push(length);
                    length = 0;
                    has_instruction = false;
                }
                visit_label(*label, lengths.len());
            }
            Item::Instruction(instruction) => features.record(instruction),
        }
        let ends_block = matches!(item, Item::Instruction(instruction) if
            instruction.value_argument().is_none()
                || ends_scope(instruction.opcode));
        length += 1;
        has_instruction |= matches!(item, Item::Instruction(_));
        if ends_block {
            lengths.push(length);
            length = 0;
            has_instruction = false;
        }
    }
    if length > 0 {
        lengths.push(length);
    }
    (lengths, features)
}

fn collect_segments<T>(
    items: impl IntoIterator<Item = T>,
    segment_lengths: &[usize],
) -> Vec<Vec<T>> {
    let mut items = items.into_iter();
    let mut segments = Vec::with_capacity(segment_lengths.len());
    for &length in segment_lengths {
        debug_assert!(length > 0);
        let mut segment = Vec::with_capacity(length);
        for _ in 0..length {
            segment.push(
                items
                    .next()
                    .expect("block segment plan exceeds available items"),
            );
        }
        segments.push(segment);
    }
    assert!(
        items.next().is_none(),
        "block segment plan did not consume every item"
    );
    segments
}

/// Fuses non-overlapping adjacent pairs while preserving label barriers and source locations.
pub(super) fn fuse_adjacent_instructions(
    items: &mut Vec<Item>,
    mut fuse: impl FnMut(&Instruction, &Instruction) -> Option<Instruction>,
) -> bool {
    let original_len = items.len();
    let mut read = 0;
    let mut write = 0;
    while read < original_len {
        let fused = if read + 1 < original_len {
            match (&items[read], &items[read + 1]) {
                (Item::Instruction(first), Item::Instruction(second)) => fuse(first, second),
                (Item::Label(_), _) | (_, Item::Label(_)) => None,
            }
        } else {
            None
        };
        if let Some(instruction) = fused {
            let second_location = match &items[read + 1] {
                Item::Instruction(second) => second.location,
                Item::Label(_) => unreachable!("a fused pair must contain two instructions"),
            };
            // CPython resolves missing locations before replacing the second half of a
            // superinstruction with a removable NOP. For a forward source-order pair,
            // preserve that location on an immediately following instruction before
            // compacting the pair. Reversed assignment stores retain the first location.
            if let Some(Item::Instruction(following)) = items.get_mut(read + 2)
                && following.location.line < 0
                && !following.no_location_policy.preserves_location()
                && second_location.line == instruction.location.line
                && second_location.column > instruction.location.column
            {
                following.location = second_location;
            }
            items[write] = Item::Instruction(instruction);
            read += 2;
        } else {
            if write != read {
                items[write] = items[read];
            }
            read += 1;
        }
        write += 1;
    }
    items.truncate(write);
    write != original_len
}

impl BlockGraph {
    #[cfg(test)]
    fn partition_items(
        items: Vec<Item>,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> (Vec<Vec<Item>>, PipelineFeatures) {
        Self::partition_items_with_labels(items, preserved_block_boundaries, |_, _| {})
    }

    fn partition_items_with_labels(
        items: Vec<Item>,
        preserved_block_boundaries: &FxHashSet<Label>,
        visit_label: impl FnMut(Label, usize),
    ) -> (Vec<Vec<Item>>, PipelineFeatures) {
        let (lengths, features) =
            normalized_segment_lengths_with_labels(&items, preserved_block_boundaries, visit_label);
        if lengths.len() == 1 {
            return (vec![items], features);
        }
        let mut remaining = items.as_slice();
        let mut segments = Vec::with_capacity(lengths.len());
        for length in lengths {
            debug_assert!(length > 0);
            let (segment, rest) = remaining.split_at(length);
            segments.push(segment.to_vec());
            remaining = rest;
        }
        debug_assert!(remaining.is_empty());
        (segments, features)
    }

    pub(super) fn from_items(
        items: Vec<Item>,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> Self {
        let mut label_blocks = Vec::new();
        let (blocks, features) = Self::partition_items_with_labels(
            items,
            preserved_block_boundaries,
            |label, ordinal| {
                Self::bind_label_in_index(
                    &mut label_blocks,
                    label,
                    BasicBlockId::from_usize(ordinal),
                );
            },
        );
        let blocks = blocks
            .into_iter()
            .map(BasicBlock::new)
            .collect::<IndexVec<BasicBlockId, _>>();
        let order = blocks.indices().collect();
        Self {
            blocks,
            order,
            label_blocks,
            features,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.order.len()
    }

    pub(super) fn item_count(&self) -> usize {
        self.iter().map(|(_, block)| block.items().len()).sum()
    }

    pub(super) const fn features(&self) -> PipelineFeatures {
        self.features
    }

    /// Records a local-initialization pass creating a checked load in existing graph items.
    pub(super) fn record_checked_fast_load(&mut self) {
        self.features.0 |= PipelineFeatures::CHECKED_FAST_LOAD;
    }

    /// Returns whether normalization would leave one jump-free block unchanged.
    pub(super) fn is_normalized_jump_free_single_block(
        &self,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> bool {
        let [block] = self.order.as_slice() else {
            return false;
        };
        let items = self[*block].items();
        if items.is_empty() {
            return false;
        }

        let mut has_instruction = false;
        for (index, item) in items.iter().enumerate() {
            match item {
                Item::Label(label) => {
                    if has_instruction || (index > 0 && preserved_block_boundaries.contains(label))
                    {
                        return false;
                    }
                }
                Item::Instruction(instruction) => {
                    if instruction.value_argument().is_none() {
                        return false;
                    }
                    has_instruction = true;
                    if ends_scope(instruction.opcode) && index + 1 < items.len() {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub(super) fn order(&self) -> &[BasicBlockId] {
        &self.order
    }

    /// Replaces the emission order without changing block identities.
    ///
    /// Blocks allocated in the arena may be omitted, but every emitted block
    /// must be a unique, valid arena entry.
    pub(super) fn replace_order(&mut self, order: Vec<BasicBlockId>) -> Result<(), String> {
        self.validate_order(&order)?;
        self.order = order;
        self.rebuild_label_index();
        Ok(())
    }

    /// Replaces the emission order and restores normalized block boundaries.
    ///
    /// Label lookup is rebuilt exactly once, either directly when the new
    /// order is already normalized or as part of repartitioning.
    pub(super) fn replace_order_and_normalize(
        &mut self,
        order: Vec<BasicBlockId>,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> Result<(), String> {
        self.validate_order(&order)?;
        self.order = order;
        let mut label_blocks = std::mem::take(&mut self.label_blocks);
        label_blocks.fill(None);
        let (segment_lengths, features) =
            self.plan_normalization(preserved_block_boundaries, &mut label_blocks);
        let prefix = segment_lengths
            .iter()
            .copied()
            .zip(self.order.iter().map(|block| self[*block].items.len()))
            .take_while(|(normalized, physical)| normalized == physical)
            .count();
        if prefix != segment_lengths.len() || prefix != self.order.len() {
            self.repartition_from_segments(&segment_lengths, features, prefix);
        }
        self.label_blocks = label_blocks;
        Ok(())
    }

    /// Retains emitted blocks that satisfy `keep` and refreshes label lookup.
    pub(super) fn retain_order(&mut self, mut keep: impl FnMut(BasicBlockId) -> bool) {
        self.order.retain(|block| keep(*block));
        self.rebuild_label_index();
    }

    pub(super) fn block_at(&self, position: usize) -> BasicBlockId {
        self.order[position]
    }

    pub(super) fn label_block(&self, label: Label) -> Option<BasicBlockId> {
        self.label_blocks.get(label.index()).copied().flatten()
    }

    fn bind_label(&mut self, label: Label, block: BasicBlockId) {
        Self::bind_label_in_index(&mut self.label_blocks, label, block);
    }

    fn bind_label_in_index(
        label_blocks: &mut Vec<Option<BasicBlockId>>,
        label: Label,
        block: BasicBlockId,
    ) {
        let index = label.index();
        if index >= label_blocks.len() {
            label_blocks.resize(index + 1, None);
        }
        let previous = label_blocks[index].replace(block);
        debug_assert_eq!(previous, None, "label is bound more than once");
    }

    /// Binds labels in a newly added block suffix in item order.
    /// Labels in the suffix must be unbound; earlier bindings remain unchanged.
    fn bind_block_labels(&mut self, block: BasicBlockId, start: usize) {
        for index in start..self[block].items.len() {
            if let Item::Label(label) = self[block].items[index] {
                self.bind_label(label, block);
            }
        }
    }

    fn unbind_label(&mut self, label: Label) {
        let index = label.index();
        if let Some(block) = self.label_blocks.get_mut(index) {
            *block = None;
        }
    }

    pub(super) fn push_block(&mut self, items: Vec<Item>) -> BasicBlockId {
        self.insert_block(self.order.len(), items)
    }

    pub(super) fn insert_block(&mut self, position: usize, items: Vec<Item>) -> BasicBlockId {
        let block = self.allocate_block(items);
        self.order.insert(position, block);
        block
    }

    pub(super) fn allocate_block(&mut self, items: Vec<Item>) -> BasicBlockId {
        let block = self.allocate_unbound_block(items);
        self.bind_block_labels(block, 0);
        block
    }

    /// Adds a block whose labels are installed separately by its caller.
    fn allocate_unbound_block(&mut self, items: Vec<Item>) -> BasicBlockId {
        let block = self.blocks.next_index();
        self.blocks.push(BasicBlock::new(items));
        block
    }

    pub(super) fn allocate_blocks_from_items(
        &mut self,
        items: Vec<Item>,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> Vec<BasicBlockId> {
        let first_block = self.blocks.len();
        let (segments, features) = Self::partition_items_with_labels(
            items,
            preserved_block_boundaries,
            |label, ordinal| {
                self.bind_label(label, BasicBlockId::from_usize(first_block + ordinal));
            },
        );
        self.features.merge(features);
        segments
            .into_iter()
            .map(|items| self.allocate_unbound_block(items))
            .collect()
    }

    /// Repartitions existing blocks without changing the identity of each
    /// block's first resulting segment.
    #[cfg(test)]
    pub(super) fn normalize_blocks(&mut self, boundaries: &FxHashSet<Label>) {
        self.normalize_blocks_from(0, boundaries);
    }

    /// Repartitions only after the first physical boundary normalization would change.
    pub(super) fn normalize_blocks_if_needed(&mut self, boundaries: &FxHashSet<Label>) {
        if let Err(prefix) = self.normalized_prefix(boundaries) {
            self.normalize_blocks_from(prefix, boundaries);
        }
    }

    /// Refreshes all normalization metadata but leaves verified prefix vectors in place.
    fn normalize_blocks_from(&mut self, prefix: usize, boundaries: &FxHashSet<Label>) {
        let mut label_blocks = std::mem::take(&mut self.label_blocks);
        label_blocks.fill(None);
        let (segment_lengths, features) = self.plan_normalization(boundaries, &mut label_blocks);
        self.repartition_from_segments(&segment_lengths, features, prefix);
        self.label_blocks = label_blocks;
    }

    /// Records normalized segment lengths and binds labels to their eventual arena identities.
    fn plan_normalization(
        &self,
        preserved_block_boundaries: &FxHashSet<Label>,
        label_blocks: &mut Vec<Option<BasicBlockId>>,
    ) -> (SmallVec<[usize; 16]>, PipelineFeatures) {
        // Normalization reuses ordered IDs before allocating any additional suffix blocks.
        let first_new_block = self.blocks.len();
        normalized_segment_lengths_with_labels(
            self.iter_items(),
            preserved_block_boundaries,
            |label, ordinal| {
                let block = self.order.get(ordinal).copied().unwrap_or_else(|| {
                    BasicBlockId::from_usize(first_new_block + ordinal - self.order.len())
                });
                Self::bind_label_in_index(label_blocks, label, block);
            },
        )
    }

    /// Applies an already inspected partition while preserving its reused block identities.
    fn repartition_from_segments(
        &mut self,
        segment_lengths: &[usize],
        features: PipelineFeatures,
        prefix: usize,
    ) {
        debug_assert!(prefix <= self.order.len().min(segment_lengths.len()));
        self.features.merge(features);
        let original_order = std::mem::take(&mut self.order);
        let items = original_order[prefix..]
            .iter()
            .copied()
            .flat_map(|block| std::mem::take(&mut self.blocks[block].items));
        let segments = collect_segments(items, &segment_lengths[prefix..]);

        let mut order = Vec::with_capacity(prefix + segments.len());
        order.extend_from_slice(&original_order[..prefix]);
        let mut reusable = original_order.into_iter().skip(prefix);
        for items in segments {
            if let Some(block) = reusable.next() {
                self.blocks[block].items = items;
                order.push(block);
            } else {
                order.push(self.allocate_unbound_block(items));
            }
        }
        self.order = order;
    }

    /// Rejoins certified internal continuations without changing any instruction or label.
    ///
    /// The caller supplies original-array owner boundaries, never general CFG entries.
    /// Validate every boundary before changing the graph; the surviving head keeps its ID.
    pub(super) fn join_continuation_blocks(
        &mut self,
        labels: &[Label],
        forbidden_entries: &FxHashSet<Label>,
    ) -> Result<(), String> {
        if labels.is_empty() {
            return Ok(());
        }
        let internal = labels.iter().copied().collect::<FxHashSet<_>>();
        if internal.len() != labels.len()
            || labels.iter().any(|label| {
                self.label_block(*label).is_none() || forbidden_entries.contains(label)
            })
            || self.iter_items().any(|item| {
                matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if internal.contains(&label)))
            })
        {
            return Err("original array continuation acquired a control-flow entry".into());
        }
        let mut join = Vec::with_capacity(self.order.len());
        let mut found = FxHashSet::default();
        for (position, block) in self.order.iter().copied().enumerate() {
            let head = self[block]
                .items()
                .iter()
                .map_while(|item| match item {
                    Item::Label(label) => Some(*label),
                    Item::Instruction(_) => None,
                })
                .collect::<Vec<_>>();
            let continuation = head.iter().any(|label| internal.contains(label));
            if continuation {
                if position == 0
                    || head.iter().any(|label| !internal.contains(label))
                    || self[self.order[position - 1]].last_instruction().is_none()
                    || !block_has_fallthrough(self[self.order[position - 1]].items())
                {
                    return Err("original array continuation lost its adjacent prefix".into());
                }
                found.extend(head);
            }
            join.push(continuation);
        }
        if found != internal {
            return Err("original array continuation is not a normalized block head".into());
        }
        let old_order = std::mem::take(&mut self.order);
        let mut order = Vec::with_capacity(old_order.len());
        for (block, continuation) in old_order.into_iter().zip(join) {
            if continuation {
                let head: BasicBlockId = *order.last().expect("validated continuation prefix");
                let items = std::mem::take(&mut self.blocks[block].items);
                self.blocks[head].items.extend(items);
            } else {
                order.push(block);
            }
        }
        self.order = order;
        self.rebuild_label_index();
        Ok(())
    }

    /// Returns whether normalization would preserve every physical block boundary.
    #[cfg(test)]
    pub(super) fn is_normalized(&self, boundaries: &FxHashSet<Label>) -> bool {
        self.normalized_prefix(boundaries).is_ok()
    }

    /// Returns the number of physical blocks that are already exact when a boundary differs.
    fn normalized_prefix(&self, boundaries: &FxHashSet<Label>) -> Result<(), usize> {
        let mut physical_lengths = self.order.iter().map(|block| self[*block].items.len());
        let mut prefix = 0;
        let mut length = 0;
        let mut has_instruction = false;
        for item in self.iter_items() {
            if let Item::Label(label) = item
                && (has_instruction || (length > 0 && boundaries.contains(label)))
            {
                if physical_lengths.next() != Some(length) {
                    return Err(prefix);
                }
                prefix += 1;
                length = 0;
                has_instruction = false;
            }
            let ends_block = matches!(item, Item::Instruction(instruction) if
                instruction.value_argument().is_none()
                    || ends_scope(instruction.opcode));
            length += 1;
            has_instruction |= matches!(item, Item::Instruction(_));
            if ends_block {
                if physical_lengths.next() != Some(length) {
                    return Err(prefix);
                }
                prefix += 1;
                length = 0;
                has_instruction = false;
            }
        }
        if length > 0 {
            if physical_lengths.next() != Some(length) {
                return Err(prefix);
            }
            prefix += 1;
        }
        if physical_lengths.next().is_some() {
            Err(prefix)
        } else {
            Ok(())
        }
    }

    pub(super) fn append_to_block(
        &mut self,
        block: BasicBlockId,
        items: impl IntoIterator<Item = Item>,
    ) {
        debug_assert!(self.order.contains(&block));
        let start = self[block].items.len();
        self.blocks[block].items.extend(items);
        self.bind_block_labels(block, start);
    }

    pub(super) fn insert_label(&mut self, block: BasicBlockId, position: usize, label: Label) {
        debug_assert!(self.order.contains(&block));
        self.blocks[block]
            .items
            .insert(position, Item::Label(label));
        self.bind_label(label, block);
    }

    /// Removes one item while keeping label lookup synchronized.
    pub(super) fn remove_item_at(&mut self, block: BasicBlockId, position: usize) -> Item {
        let item = self.blocks[block].items.remove(position);
        if let Item::Label(label) = item
            && self.label_block(label) == Some(block)
        {
            self.unbind_label(label);
        }
        item
    }

    /// Removes original instruction positions in graph order with one compaction per block.
    /// Adjacent duplicate positions are ignored; labels and conservative features are unchanged.
    pub(super) fn remove_ordered_instructions(&mut self, positions: &[ItemPosition]) {
        for positions in positions.chunk_by(|left, right| left.block == right.block) {
            if let [position] = positions {
                debug_assert!(matches!(self.item(*position), Item::Instruction(_)));
                self.remove_item(*position);
                continue;
            }

            debug_assert!(
                positions
                    .windows(2)
                    .all(|pair| pair[0].offset <= pair[1].offset)
            );
            let mut remaining = positions.iter().peekable();
            let mut offset = 0;
            self.retain_items(positions[0].block, |item| {
                let remove = remaining
                    .peek()
                    .is_some_and(|position| position.offset == offset);
                if remove {
                    debug_assert!(matches!(item, Item::Instruction(_)));
                    while remaining
                        .peek()
                        .is_some_and(|position| position.offset == offset)
                    {
                        remaining.next();
                    }
                }
                offset += 1;
                !remove
            });
            debug_assert_eq!(remaining.len(), 0);
        }
    }

    /// Retains items that satisfy `keep` while keeping label lookup synchronized.
    pub(super) fn retain_items(
        &mut self,
        block: BasicBlockId,
        mut keep: impl FnMut(&Item) -> bool,
    ) {
        let items = &mut self.blocks[block].items;
        let label_blocks = &mut self.label_blocks;
        items.retain(|item| {
            let retain = keep(item);
            if !retain && let Item::Label(label) = item {
                let index = label.index();
                if label_blocks.get(index).copied().flatten() == Some(block) {
                    label_blocks[index] = None;
                }
            }
            retain
        });
    }

    fn rebuild_label_index(&mut self) {
        let mut label_blocks = std::mem::take(&mut self.label_blocks);
        label_blocks.fill(None);
        for block in self.order.iter().copied() {
            for item in self[block].items() {
                if let Item::Label(label) = item {
                    Self::bind_label_in_index(&mut label_blocks, *label, block);
                }
            }
        }
        self.label_blocks = label_blocks;
    }

    /// Returns mutable instructions while exposing labels only as markers.
    pub(super) fn iter_items_mut(
        &mut self,
        block: BasicBlockId,
    ) -> impl Iterator<Item = ItemMut<'_>> {
        self.blocks[block].items.iter_mut().map(|item| match item {
            Item::Label(_) => ItemMut::Label,
            Item::Instruction(instruction) => ItemMut::Instruction(instruction),
        })
    }

    /// Fuses non-overlapping adjacent instruction pairs without exposing labels.
    pub(super) fn fuse_adjacent_instructions(
        &mut self,
        block: BasicBlockId,
        fuse: impl FnMut(&Instruction, &Instruction) -> Option<Instruction>,
    ) -> bool {
        fuse_adjacent_instructions(&mut self.blocks[block].items, fuse)
    }

    fn validate_order(&self, order: &[BasicBlockId]) -> Result<(), String> {
        let mut seen_blocks = vec![false; self.blocks.len()];
        for block in order.iter().copied() {
            let Some(seen) = seen_blocks.get_mut(block.index()) else {
                return Err(format!("block {} is outside the arena", block.index()));
            };
            if std::mem::replace(seen, true) {
                return Err(format!("block {} appears more than once", block.index()));
            }
        }
        Ok(())
    }

    /// Removes the extra label-only blocks introduced by the preserved
    /// boundaries used for the pre-assembly reachability scan.
    ///
    /// This is only valid before any pass has reordered or allocated blocks.
    pub(super) fn remove_initial_preserved_boundaries(
        &mut self,
        mut reachable: Option<&mut Vec<bool>>,
    ) {
        debug_assert_eq!(self.order.len(), self.blocks.len());
        debug_assert!(
            self.order
                .iter()
                .enumerate()
                .all(|(index, block)| block.index() == index)
        );
        // Only a label-only block with a physical successor can be coalesced. In
        // its absence the initial IDs, reachability bitmap and label index are final.
        if !self.order[..self.order.len().saturating_sub(1)]
            .iter()
            .any(|block| {
                self[*block]
                    .items()
                    .iter()
                    .all(|item| matches!(item, Item::Label(_)))
            })
        {
            return;
        }

        let original = std::mem::take(&mut self.blocks);
        let reachable_len = reachable.as_ref().map(|reachable| reachable.len());
        debug_assert!(reachable_len.is_none_or(|len| len == original.len()));
        let mut blocks: IndexVec<BasicBlockId, BasicBlock> =
            IndexVec::with_capacity(original.len());
        for (index, mut block) in original.into_iter().enumerate() {
            if blocks.last().is_some_and(|previous| {
                previous
                    .items()
                    .iter()
                    .all(|item| matches!(item, Item::Label(_)))
            }) {
                if let Some(reachable) = &mut reachable {
                    let previous = blocks.len() - 1;
                    reachable[previous] |= reachable[index];
                }
                blocks.last_mut().unwrap().items.append(&mut block.items);
            } else {
                if let Some(reachable) = &mut reachable {
                    reachable[blocks.len()] = reachable[index];
                }
                blocks.push(block);
            }
        }
        if let Some(reachable) = reachable {
            reachable.truncate(blocks.len());
        }
        self.blocks = blocks;
        self.order = (0..self.blocks.len())
            .map(BasicBlockId::from_usize)
            .collect();
        self.rebuild_label_index();
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (BasicBlockId, &BasicBlock)> {
        self.order
            .iter()
            .copied()
            .map(|block| (block, &self[block]))
    }

    pub(super) fn iter_items(&self) -> impl Iterator<Item = &Item> {
        GraphItems {
            graph: self,
            blocks: self.order.iter(),
            items: [].iter(),
        }
    }

    /// Copies the ordered item stream into a single allocation.
    pub(super) fn copy_items(&self) -> Vec<Item> {
        let mut items = Vec::with_capacity(self.item_count());
        for (_, block) in self.iter() {
            items.extend_from_slice(block.items());
        }
        items
    }

    pub(super) fn for_each_instruction_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        let blocks = &mut self.blocks;
        for block in self.order.iter().copied() {
            for item in &mut blocks[block].items {
                if let Item::Instruction(instruction) = item {
                    visit(instruction);
                }
            }
        }
    }

    pub(super) fn instructions_mut(
        &mut self,
        block: BasicBlockId,
    ) -> impl DoubleEndedIterator<Item = &mut Instruction> {
        self.blocks[block]
            .items
            .iter_mut()
            .filter_map(Item::as_instruction_mut)
    }

    pub(super) fn for_each_adjacent_instruction_pair_mut(
        &mut self,
        mut visit: impl FnMut(&mut Instruction, &mut Instruction),
    ) {
        let blocks = &mut self.blocks;
        for block in self.order.iter().copied() {
            let items = &mut blocks[block].items;
            for index in 0..items.len().saturating_sub(1) {
                let (before, after) = items.split_at_mut(index + 1);
                if let (Item::Instruction(first), Item::Instruction(second)) =
                    (&mut before[index], &mut after[0])
                {
                    visit(first, second);
                }
            }
        }
    }

    pub(super) fn item_positions(&self) -> Vec<ItemPosition> {
        let mut positions = Vec::with_capacity(
            self.iter()
                .map(|(_, contents)| contents.items().len())
                .sum(),
        );
        for (block, contents) in self.iter() {
            positions
                .extend((0..contents.items().len()).map(|offset| ItemPosition { block, offset }));
        }
        positions
    }

    /// Builds an ordered view of matching partitions without moving the graph's owned items.
    pub(super) fn matching_partition_positions(
        &self,
        boundary_labels: &FxHashSet<Label>,
        ends_block: impl Fn(&Instruction) -> bool,
        matches: impl Fn(&Instruction) -> bool,
    ) -> Vec<Vec<ItemPosition>> {
        let mut partitions = Vec::new();
        let mut segment = Vec::new();
        let mut contains_match = false;
        let finish = |segment: &mut Vec<_>, contains_match: &mut bool, partitions: &mut Vec<_>| {
            if std::mem::take(contains_match) {
                partitions.push(std::mem::take(segment));
            } else {
                segment.clear();
            }
        };
        for (block, contents) in self.iter() {
            for (offset, item) in contents.items().iter().enumerate() {
                if matches!(item, Item::Label(label) if boundary_labels.contains(label))
                    && !segment.is_empty()
                {
                    finish(&mut segment, &mut contains_match, &mut partitions);
                }
                segment.push(ItemPosition { block, offset });
                if let Item::Instruction(instruction) = item {
                    contains_match |= matches(instruction);
                    if ends_block(instruction) {
                        finish(&mut segment, &mut contains_match, &mut partitions);
                    }
                }
            }
        }
        if contains_match {
            partitions.push(segment);
        }
        partitions
    }

    pub(super) fn item(&self, position: ItemPosition) -> &Item {
        &self[position.block].items[position.offset]
    }

    pub(super) fn instruction_mut(&mut self, position: ItemPosition) -> Option<&mut Instruction> {
        match &mut self.blocks[position.block].items[position.offset] {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        }
    }

    /// Retains an erased CFG boundary until borrowed-load analysis finishes.
    pub(super) fn mark_borrow_empty_block(&mut self, position: ItemPosition) {
        let instruction = self
            .instruction_mut(position)
            .expect("borrow boundary instruction");
        instruction.jump_policy.mark_borrow_empty_block();
        let instruction = *instruction;
        self.features.record(&instruction);
    }

    pub(super) fn insert_item(&mut self, position: ItemPosition, item: Item) {
        debug_assert!(self.order.contains(&position.block));
        if let Item::Label(label) = item {
            self.bind_label(label, position.block);
        }
        self.blocks[position.block]
            .items
            .insert(position.offset, item);
    }

    pub(super) fn remove_item(&mut self, position: ItemPosition) -> Item {
        self.remove_item_at(position.block, position.offset)
    }

    pub(super) fn reachable_blocks(&self, exception_regions: &[ExceptionRegion]) -> Vec<bool> {
        let reachable = self.find_reachable_blocks(exception_regions);
        #[cfg(test)]
        assert_eq!(
            reachable,
            self.reference_reachable_blocks(exception_regions)
        );
        reachable
    }

    fn find_reachable_blocks(&self, exception_regions: &[ExceptionRegion]) -> Vec<bool> {
        let mut reachable = vec![false; self.blocks.len()];
        let Some(mut entry) = self.order.first().copied() else {
            return reachable;
        };
        if exception_regions.is_empty() && !self.features.has_jump() {
            let mut blocks = self.order.iter().copied();
            loop {
                let Some(block) = blocks.next() else {
                    return reachable;
                };
                let (falls_through, target) = self[block].successors();
                if target.is_some() {
                    // A low-level in-place edit may have introduced a new jump
                    // without refreshing features. The visited prefix has no
                    // outgoing jumps, so resume ordinary traversal at this block.
                    entry = block;
                    break;
                }
                reachable[block.index()] = true;
                if !falls_through {
                    return reachable;
                }
            }
        }
        let mut order_positions = vec![usize::MAX; self.blocks.len()];
        for (position, block) in self.order.iter().copied().enumerate() {
            order_positions[block.index()] = position;
        }
        if exception_regions.is_empty() {
            let mut pending = vec![entry];
            self.follow_reachable_blocks(&mut pending, &order_positions, &mut reachable);
            return reachable;
        }
        let mut region_blocks = exception_regions
            .iter()
            .filter_map(|region| {
                let start = self.label_block(region.start)?;
                let end = self.label_block(region.end)?;
                let start_position = order_positions[start.index()];
                let end_position = order_positions[end.index()];
                // An empty protected body still has an original control-flow entry.
                // Its handler is reachable only if that entry is reachable, including
                // when a copied finally body has become dead before optimization.
                Some((
                    start_position,
                    if start == end {
                        start_position + 1
                    } else {
                        end_position
                    },
                    self.label_block(region.target)?,
                ))
            })
            .collect::<Vec<_>>();
        let mut pending = Vec::with_capacity(region_blocks.len().saturating_add(1));
        pending.push(entry);
        if region_blocks.len() <= 4 {
            while let Some(mut block) = pending.pop() {
                loop {
                    if std::mem::replace(&mut reachable[block.index()], true) {
                        break;
                    }
                    let position = order_positions[block.index()];
                    let (falls_through, target) = self[block].successors();
                    if let Some(target) = target.and_then(|target| self.label_block(target)) {
                        pending.push(target);
                    }
                    region_blocks.retain(|(start, end, handler)| {
                        let is_protected = *start <= position && position < *end;
                        if is_protected {
                            pending.push(*handler);
                        }
                        !is_protected
                    });
                    if falls_through && let Some(next) = self.order.get(position + 1) {
                        block = *next;
                    } else {
                        break;
                    }
                }
            }
            return reachable;
        }
        let mut reachable_prefix = Vec::new();
        loop {
            self.follow_reachable_blocks(&mut pending, &order_positions, &mut reachable);

            if region_blocks.is_empty() {
                break;
            }
            if reachable_prefix.is_empty() {
                reachable_prefix.resize(self.order.len() + 1, 0);
            }
            for (position, block) in self.order.iter().copied().enumerate() {
                reachable_prefix[position + 1] =
                    reachable_prefix[position] + usize::from(reachable[block.index()]);
            }
            region_blocks.retain(|(start, end, handler)| {
                if *start < *end && reachable_prefix[*start] < reachable_prefix[*end] {
                    pending.push(*handler);
                    false
                } else {
                    true
                }
            });
            if pending.is_empty() {
                break;
            }
        }
        reachable
    }

    /// Exhausts ordinary CFG edges by walking fallthrough and queuing explicit targets.
    fn follow_reachable_blocks(
        &self,
        pending: &mut Vec<BasicBlockId>,
        order_positions: &[usize],
        reachable: &mut [bool],
    ) {
        while let Some(mut block) = pending.pop() {
            loop {
                if std::mem::replace(&mut reachable[block.index()], true) {
                    break;
                }
                let position = order_positions[block.index()];
                let (falls_through, target) = self[block].successors();
                if let Some(target) = target.and_then(|target| self.label_block(target)) {
                    pending.push(target);
                }
                if falls_through && let Some(next) = self.order.get(position + 1) {
                    block = *next;
                } else {
                    break;
                }
            }
        }
    }

    /// The unoptimized traversal verifies each unit-test call against the same full arena bitmap.
    #[cfg(test)]
    fn reference_reachable_blocks(&self, exception_regions: &[ExceptionRegion]) -> Vec<bool> {
        let mut reachable = vec![false; self.blocks.len()];
        let Some(entry) = self.order.first().copied() else {
            return reachable;
        };
        let mut order_positions = vec![usize::MAX; self.blocks.len()];
        for (position, block) in self.order.iter().copied().enumerate() {
            order_positions[block.index()] = position;
        }
        let mut region_blocks = exception_regions
            .iter()
            .filter_map(|region| {
                let start = self.label_block(region.start)?;
                let end = self.label_block(region.end)?;
                let start_position = order_positions[start.index()];
                let end_position = order_positions[end.index()];
                // An empty protected body still has an original control-flow entry.
                // Its handler is reachable only if that entry is reachable, including
                // when a copied finally body has become dead before optimization.
                Some((
                    start_position,
                    if start == end {
                        start_position + 1
                    } else {
                        end_position
                    },
                    self.label_block(region.target)?,
                ))
            })
            .collect::<Vec<_>>();
        let mut pending = Vec::with_capacity(region_blocks.len().saturating_add(1));
        pending.push(entry);
        if region_blocks.len() <= 4 {
            while let Some(block) = pending.pop() {
                if std::mem::replace(&mut reachable[block.index()], true) {
                    continue;
                }
                let position = order_positions[block.index()];
                if block_has_fallthrough(self[block].items())
                    && let Some(next) = self.order.get(position + 1)
                {
                    pending.push(*next);
                }
                if let Some(target) = block_jump_target(self[block].items())
                    && let Some(target) = self.label_block(target)
                {
                    pending.push(target);
                }
                region_blocks.retain(|(start, end, handler)| {
                    let is_protected = *start <= position && position < *end;
                    if is_protected {
                        pending.push(*handler);
                    }
                    !is_protected
                });
            }
            return reachable;
        }
        let mut reachable_prefix = Vec::new();
        loop {
            while let Some(block) = pending.pop() {
                if std::mem::replace(&mut reachable[block.index()], true) {
                    continue;
                }
                let position = order_positions[block.index()];
                if block_has_fallthrough(self[block].items())
                    && let Some(next) = self.order.get(position + 1)
                {
                    pending.push(*next);
                }
                if let Some(target) = block_jump_target(self[block].items())
                    && let Some(target) = self.label_block(target)
                {
                    pending.push(target);
                }
            }

            if region_blocks.is_empty() {
                break;
            }
            if reachable_prefix.is_empty() {
                reachable_prefix.resize(self.order.len() + 1, 0);
            }
            for (position, block) in self.order.iter().copied().enumerate() {
                reachable_prefix[position + 1] =
                    reachable_prefix[position] + usize::from(reachable[block.index()]);
            }
            region_blocks.retain(|(start, end, handler)| {
                if *start < *end && reachable_prefix[*start] < reachable_prefix[*end] {
                    pending.push(*handler);
                    false
                } else {
                    true
                }
            });
            if pending.is_empty() {
                break;
            }
        }
        reachable
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn item_reachability(&self, exception_regions: &[ExceptionRegion]) -> Vec<bool> {
        let reachable = self.reachable_blocks(exception_regions);
        self.iter()
            .flat_map(|(block, contents)| {
                let reachable = reachable[block.index()];
                contents
                    .items()
                    .iter()
                    .map(move |item| matches!(item, Item::Label(_)) || reachable)
            })
            .collect()
    }

    pub(super) fn into_items(mut self) -> Vec<Item> {
        match self.order.as_slice() {
            [] => return Vec::new(),
            [block] => return std::mem::take(&mut self.blocks[*block].items),
            _ => {}
        }
        let mut items = Vec::with_capacity(self.item_count());
        let mut seen = SmallVec::<[u64; 2]>::new();
        seen.resize(self.blocks.len().div_ceil(u64::BITS as usize), 0);
        for block in self.order {
            let index = block.index();
            let word = &mut seen[index / u64::BITS as usize];
            let bit = 1 << (index % u64::BITS as usize);
            assert_eq!(
                *word & bit,
                0,
                "block appears more than once in emission order"
            );
            *word |= bit;
            items.append(&mut self.blocks[block].items);
        }
        items
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn validate(&self) -> Result<(), String> {
        self.validate_order(&self.order)?;
        let mut labels = Vec::new();
        for block in self.order.iter().copied() {
            let contents = &self.blocks[block];
            if contents.items.is_empty() {
                return Err(format!("block {} is empty", block.index()));
            }
            for item in contents.items() {
                if let Item::Label(label) = item {
                    let index = label.index();
                    if index >= labels.len() {
                        labels.resize(index + 1, None);
                    }
                    if labels[index].replace(block).is_some() {
                        return Err(format!("label {} is bound more than once", label.as_u32()));
                    }
                }
            }
        }
        while labels.last() == Some(&None) {
            labels.pop();
        }
        let mut indexed = self.label_blocks.as_slice();
        while indexed.last() == Some(&None) {
            indexed = &indexed[..indexed.len() - 1];
        }
        if labels != indexed {
            return Err("label-to-block index is stale".to_string());
        }
        Ok(())
    }
}

impl Index<BasicBlockId> for BlockGraph {
    type Output = BasicBlock;

    fn index(&self, index: BasicBlockId) -> &Self::Output {
        &self.blocks[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::SourceLocation;
    use crate::target::opcodes::{
        JUMP_BACKWARD, JUMP_FORWARD, LOAD_FAST_CHECK, NOP, POP_JUMP_IF_NONE, POP_JUMP_IF_TRUE,
        RETURN_VALUE, STORE_FAST, SWAP, TO_BOOL,
    };

    fn instruction(opcode: crate::target::Opcode, operand: Operand) -> Item {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            SourceLocation::NONE,
            None,
        ))
    }

    #[test]
    fn item_iteration_matches_block_flattening() {
        let items = [
            Item::Label(Label::from_u32(0)),
            instruction(NOP, Operand::Value(0)),
            Item::Label(Label::from_u32(1)),
            instruction(LOAD_FAST, Operand::Value(1)),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];
        for first in 0..=items.len() {
            for second in first..=items.len() {
                let blocks =
                    [&items[..first], &items[first..second], &items[second..]].map(|items| {
                        BasicBlock {
                            items: items.to_vec(),
                        }
                    });
                let mut graph = BlockGraph {
                    blocks: blocks.into_iter().collect(),
                    order: Vec::new(),
                    label_blocks: Vec::new(),
                    features: PipelineFeatures::default(),
                };
                for order in [
                    &[][..],
                    &[0][..],
                    &[2, 0, 1][..],
                    &[1, 2][..],
                    &[0, 1, 2, 0][..],
                ] {
                    graph.order = order
                        .iter()
                        .copied()
                        .map(BasicBlockId::from_usize)
                        .collect();
                    let reference = || graph.iter().flat_map(|(_, block)| block.items());
                    let mut actual = graph.iter_items();
                    let mut expected = reference();
                    for _ in 0..=items.len() * 2 {
                        assert_eq!(actual.size_hint(), expected.size_hint());
                        assert_eq!(actual.next(), expected.next());
                    }
                    for skip in [0, 1, 2, items.len(), usize::MAX] {
                        let mut actual = graph.iter_items();
                        let mut expected = reference();
                        assert_eq!(actual.nth(skip), expected.nth(skip));
                        assert_eq!(actual.size_hint(), expected.size_hint());
                        assert_eq!(actual.collect::<Vec<_>>(), expected.collect::<Vec<_>>());
                    }
                    for stop in 0..=items.len() * 2 {
                        let mut actual = graph.iter_items();
                        let mut expected = reference();
                        let mut visited = Vec::new();
                        let mut expected_visited = Vec::new();
                        assert_eq!(
                            actual.any(|item| {
                                visited.push(item);
                                visited.len() == stop
                            }),
                            expected.any(|item| {
                                expected_visited.push(item);
                                expected_visited.len() == stop
                            }),
                        );
                        assert_eq!(visited, expected_visited);
                        assert_eq!(actual.size_hint(), expected.size_hint());
                        assert_eq!(actual.collect::<Vec<_>>(), expected.collect::<Vec<_>>());
                    }
                }
            }
        }
    }

    #[test]
    fn opcode_feature_table_matches_instruction_classification() {
        use crate::assembler::OperandPatchId;
        use crate::target::opcodes::{
            BEFORE_WITH, CALL, COMPARE_OP, CONTAINS_OP, COPY_FREE_VARS, FOR_ITER, LIST_APPEND,
            LOAD_FAST_AND_CLEAR, LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_GLOBAL, LOAD_SMALL_INT,
            MATCH_CLASS, POP_TOP, RAISE_VARARGS, RESUME, SEND, STORE_FAST_LOAD_FAST,
            STORE_FAST_STORE_FAST, UNPACK_EX, WITH_EXCEPT_START,
        };

        fn original(instruction: &Instruction) -> PipelineFeatures {
            use PipelineFeatures as Features;

            let mut flags = match instruction.opcode {
                opcode
                    if is_unconditional_jump(opcode)
                        && matches!(instruction.operand(), Operand::Forward(_)) =>
                {
                    Features::FORWARD_JUMP
                }
                POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE
                    if matches!(instruction.operand(), Operand::Forward(_)) =>
                {
                    Features::FORWARD_CONDITIONAL
                }
                SWAP => Features::SWAP_TRIGGER,
                LOAD_FAST_CHECK => Features::FAST_LOAD | Features::CHECKED_FAST_LOAD,
                LOAD_FAST | LOAD_FAST_BORROW | LOAD_FAST_LOAD_FAST => Features::FAST_LOAD,
                STORE_FAST => Features::STORE_FAST_TRIGGER,
                TO_BOOL | UNARY_NOT => Features::BOOLEAN_CONVERSION,
                IS_OP => Features::IDENTITY_COMPARISON,
                COPY if instruction.operand() == Operand::Value(1) => Features::COPY_TOP,
                NOT_TAKEN => {
                    Features::NOT_TAKEN
                        | if matches!(
                            instruction.exception_policy(),
                            ExceptionPolicy::NoneUnchangedBooleanSingleton
                                | ExceptionPolicy::NonePresent
                                | ExceptionPolicy::NoneAbsent
                        ) {
                            Features::ORIGINAL_FUSION_OWNER_CANDIDATE
                        } else {
                            0
                        }
                }
                NOP | LOAD_CONST | RETURN_CONST | KW_NAMES
                    if instruction.pre_fold_extra_instructions() == 2
                        && instruction.no_location_policy.borrow_fallthrough()
                            == BorrowFallthrough::Barrier =>
                {
                    Features::ORIGINAL_FUSION_OWNER_CANDIDATE
                }
                _ => 0,
            };
            if instruction.jump_policy.borrow_empty_block() {
                flags |= Features::BORROW_EMPTY_BLOCK;
            }
            if matches!(
                instruction.operand(),
                Operand::Forward(_) | Operand::Backward(_)
            ) {
                flags |= Features::JUMP;
            }
            if matches!(
                instruction.opcode,
                POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
            ) {
                flags |= Features::POP_CONDITIONAL;
            }
            PipelineFeatures(flags)
        }

        let classified = [
            JUMP_BACKWARD,
            JUMP_BACKWARD_NO_INTERRUPT,
            JUMP_FORWARD,
            POP_JUMP_IF_FALSE,
            POP_JUMP_IF_TRUE,
            POP_JUMP_IF_NONE,
            POP_JUMP_IF_NOT_NONE,
            SWAP,
            LOAD_FAST_CHECK,
            LOAD_FAST,
            LOAD_FAST_BORROW,
            LOAD_FAST_LOAD_FAST,
            STORE_FAST,
            TO_BOOL,
            UNARY_NOT,
            IS_OP,
            COPY,
            NOT_TAKEN,
            NOP,
            LOAD_CONST,
            RETURN_CONST,
            KW_NAMES,
        ];
        for (index, flags) in PipelineFeatures::BY_OPCODE.into_iter().enumerate() {
            let expected = classified
                .iter()
                .any(|opcode| usize::from(opcode.code()) == index);
            assert_eq!(flags != 0, expected, "opcode index {index}");
        }

        let unclassified = [
            BEFORE_WITH,
            CALL,
            COMPARE_OP,
            CONTAINS_OP,
            COPY_FREE_VARS,
            FOR_ITER,
            LIST_APPEND,
            LOAD_FAST_AND_CLEAR,
            LOAD_FAST_BORROW_LOAD_FAST_BORROW,
            LOAD_GLOBAL,
            LOAD_SMALL_INT,
            MATCH_CLASS,
            POP_TOP,
            RAISE_VARARGS,
            RESUME,
            RETURN_VALUE,
            SEND,
            STORE_FAST_LOAD_FAST,
            STORE_FAST_STORE_FAST,
            UNPACK_EX,
            WITH_EXCEPT_START,
        ];
        let target = Label::from_u32(1);
        for opcode in classified.into_iter().chain(unclassified) {
            for operand in [
                Operand::Value(0),
                Operand::Value(1),
                Operand::Value(2),
                Operand::Forward(target),
                Operand::Backward(target),
                Operand::Deferred(OperandPatchId::from_u32(0)),
            ] {
                for (folds, fallthrough, exception, borrow) in [
                    (
                        0,
                        BorrowFallthrough::Reachable,
                        ExceptionPolicy::NoneUnchanged,
                        false,
                    ),
                    (
                        2,
                        BorrowFallthrough::Barrier,
                        ExceptionPolicy::NoneUnchanged,
                        true,
                    ),
                    (
                        1,
                        BorrowFallthrough::Barrier,
                        ExceptionPolicy::NonePresent,
                        false,
                    ),
                    (
                        2,
                        BorrowFallthrough::BarrierIfRemoved,
                        ExceptionPolicy::NoneAbsent,
                        true,
                    ),
                    (
                        2,
                        BorrowFallthrough::Reachable,
                        ExceptionPolicy::NoneUnchangedBooleanSingleton,
                        false,
                    ),
                    (
                        3,
                        BorrowFallthrough::Barrier,
                        ExceptionPolicy::NoneDynamicCondition,
                        true,
                    ),
                ] {
                    let mut instruction =
                        Instruction::new(opcode, operand, SourceLocation::NONE, None);
                    instruction.add_pre_fold_extra_instructions(folds);
                    instruction
                        .no_location_policy
                        .set_borrow_fallthrough(fallthrough);
                    instruction.set_exception_policy(exception);
                    if borrow {
                        instruction.jump_policy.mark_borrow_empty_block();
                    }
                    let expected = original(&instruction);
                    let mut actual = PipelineFeatures::default();
                    actual.record(&instruction);
                    assert_eq!(actual, expected, "{instruction:?}");

                    let mut accumulated =
                        PipelineFeatures(PipelineFeatures::FAST_LOAD | PipelineFeatures::JUMP);
                    accumulated.record(&instruction);
                    assert_eq!(
                        accumulated.0,
                        expected.0 | PipelineFeatures::FAST_LOAD | PipelineFeatures::JUMP
                    );
                }
            }
        }
    }

    #[test]
    fn graph_construction_summarizes_all_pass_triggers() {
        let target = Label::from_u32(0);
        let mut borrow_marker =
            Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
        borrow_marker.jump_policy.mark_borrow_empty_block();
        let mut branch_marker =
            Instruction::new(NOT_TAKEN, Operand::Value(0), SourceLocation::NONE, None);
        branch_marker.set_exception_policy(ExceptionPolicy::NonePresent);
        let graph = BlockGraph::from_items(
            vec![
                instruction(JUMP_FORWARD, Operand::Forward(target)),
                instruction(POP_JUMP_IF_TRUE, Operand::Forward(target)),
                instruction(SWAP, Operand::Value(2)),
                instruction(LOAD_FAST_CHECK, Operand::Value(0)),
                instruction(STORE_FAST, Operand::Value(0)),
                instruction(TO_BOOL, Operand::Value(0)),
                instruction(IS_OP, Operand::Value(0)),
                instruction(COPY, Operand::Value(1)),
                Item::Instruction(borrow_marker),
                Item::Instruction(branch_marker),
                Item::Label(target),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let features = graph.features();

        assert!(features.has_forward_jump());
        assert!(features.has_forward_conditional());
        assert!(features.has_swap());
        assert!(features.has_fast_load());
        assert!(features.has_checked_fast_load());
        assert!(features.has_store_fast());
        assert!(features.has_boolean_conversion());
        assert!(features.has_identity_comparison());
        assert!(features.has_borrow_empty_block());
        assert!(features.has_jump());
        assert!(features.has_pop_conditional());
        assert!(features.has_copy_top());
        assert!(features.has_not_taken());
        assert!(features.has_original_fusion_owner_candidate());
    }

    #[test]
    fn graph_construction_does_not_mark_non_threadable_jumps_as_forward_candidates() {
        let target = Label::from_u32(0);
        let graph = BlockGraph::from_items(
            vec![
                instruction(JUMP_BACKWARD, Operand::Backward(target)),
                instruction(POP_JUMP_IF_NONE, Operand::Forward(target)),
                instruction(COPY, Operand::Value(2)),
                Item::Label(target),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let features = graph.features();

        assert!(!features.has_forward_jump());
        assert!(!features.has_forward_conditional());
        assert!(!features.has_swap());
        assert!(!features.has_fast_load());
        assert!(!features.has_checked_fast_load());
        assert!(!features.has_store_fast());
        assert!(!features.has_boolean_conversion());
        assert!(!features.has_identity_comparison());
        assert!(!features.has_borrow_empty_block());
        assert!(features.has_jump());
        assert!(features.has_pop_conditional());
        assert!(!features.has_copy_top());
        assert!(!features.has_not_taken());
        assert!(!features.has_original_fusion_owner_candidate());
    }

    #[test]
    fn original_fusion_owner_features_distinguish_authored_marker_and_folded_nop_policies() {
        let marker = |policy| {
            let mut instruction =
                Instruction::new(NOT_TAKEN, Operand::Value(0), SourceLocation::NONE, None);
            instruction.set_exception_policy(policy);
            Item::Instruction(instruction)
        };
        let folded = |opcode, folds, fallthrough| {
            let mut instruction =
                Instruction::new(opcode, Operand::Value(0), SourceLocation::NONE, None);
            instruction.add_pre_fold_extra_instructions(folds);
            instruction
                .no_location_policy
                .set_borrow_fallthrough(fallthrough);
            Item::Instruction(instruction)
        };
        for (item, has_marker, has_owner) in [
            (marker(ExceptionPolicy::NonePresent), true, true),
            (marker(ExceptionPolicy::NoneAbsent), true, true),
            (
                marker(ExceptionPolicy::NoneUnchangedBooleanSingleton),
                true,
                true,
            ),
            (marker(ExceptionPolicy::NoneDynamicCondition), true, false),
            (folded(NOP, 2, BorrowFallthrough::Barrier), false, true),
            (folded(NOP, 1, BorrowFallthrough::Barrier), false, false),
            (
                folded(NOP, 2, BorrowFallthrough::BarrierIfRemoved),
                false,
                false,
            ),
            (
                folded(LOAD_CONST, 2, BorrowFallthrough::Barrier),
                false,
                true,
            ),
            (
                folded(RETURN_CONST, 2, BorrowFallthrough::Barrier),
                false,
                true,
            ),
            (folded(KW_NAMES, 2, BorrowFallthrough::Barrier), false, true),
        ] {
            let graph = BlockGraph::from_items(vec![item], &FxHashSet::default());
            let features = graph.features();
            assert_eq!(features.has_not_taken(), has_marker);
            assert_eq!(features.has_original_fusion_owner_candidate(), has_owner);
        }
    }

    #[test]
    fn allocating_partitioned_blocks_keeps_the_feature_summary_conservative() {
        let target = Label::from_u32(0);
        let mut graph = BlockGraph::from_items(
            vec![instruction(RETURN_VALUE, Operand::Value(0))],
            &FxHashSet::default(),
        );
        assert_eq!(graph.features(), PipelineFeatures::default());

        graph.allocate_blocks_from_items(
            vec![
                instruction(JUMP_FORWARD, Operand::Forward(target)),
                instruction(SWAP, Operand::Value(2)),
                Item::Label(target),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );

        assert!(graph.features().has_forward_jump());
        assert!(graph.features().has_swap());
        assert!(graph.features().has_jump());
    }

    #[test]
    fn reachability_handles_straight_prefixes_cycles_and_chained_exception_entries() {
        let label = Label::from_u32;
        let mut graph = BlockGraph::from_items(Vec::new(), &FxHashSet::default());
        let blocks = [
            vec![
                Item::Label(label(0)),
                instruction(NOP, Operand::Value(0)),
                Item::Label(label(20)),
            ],
            vec![
                Item::Label(label(1)),
                instruction(POP_JUMP_IF_TRUE, Operand::Forward(label(4))),
            ],
            vec![
                Item::Label(label(2)),
                instruction(JUMP_FORWARD, Operand::Forward(label(6))),
            ],
            vec![Item::Label(label(3)), instruction(NOP, Operand::Value(0))],
            vec![
                Item::Label(label(4)),
                instruction(POP_JUMP_IF_TRUE, Operand::Backward(label(4))),
            ],
            vec![
                Item::Label(label(5)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            vec![Item::Label(label(6)), instruction(NOP, Operand::Value(0))],
            vec![
                Item::Label(label(7)),
                instruction(JUMP_BACKWARD, Operand::Backward(label(6))),
            ],
            vec![Item::Label(label(8))],
            vec![
                Item::Label(label(9)),
                instruction(POP_JUMP_IF_TRUE, Operand::Forward(label(99))),
            ],
            vec![
                Item::Label(label(10)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
        ]
        .into_iter()
        .map(|items| graph.push_block(items))
        .collect::<Vec<_>>();

        // Direct arena mutation need not update the conservative feature: a
        // detected jump must resume the general walk at the end of the prefix.
        assert!(!graph.features().has_jump());
        let no_handlers = [
            true, true, true, false, true, true, true, true, false, false, false,
        ];
        assert_eq!(graph.reachable_blocks(&[]), no_handlers);
        graph.features.0 |= PipelineFeatures::JUMP;
        assert_eq!(graph.reachable_blocks(&[]), no_handlers);

        let region = |start, end, target| ExceptionRegion {
            start: label(start),
            end: label(end),
            target: label(target),
            depth: 0,
            preserve_lasti: false,
            is_scope_fallback: false,
            priority: super::super::ExceptionRegionPriority::Resolved { start: 0, end: 0 },
        };
        let regions = [
            region(1, 2, 8),
            region(8, 8, 3),
            region(3, 4, 9),
            region(9, 9, 9),
            region(6, 7, 10),
            region(99, 10, 3),
        ];
        assert_eq!(
            graph.reachable_blocks(&regions[..1]),
            [
                true, true, true, false, true, true, true, true, true, true, true
            ],
        );
        for regions in [&regions[..4], &regions[..5], &regions[..]] {
            assert_eq!(graph.reachable_blocks(regions), [true; 11]);
        }

        graph
            .replace_order(
                [0, 1, 4, 6, 2, 8, 7, 9, 10, 5, 3]
                    .map(|index| blocks[index])
                    .into(),
            )
            .unwrap();
        let reachable = graph.reachable_blocks(&[]);
        assert!(reachable[blocks[4].index()]);
        assert!(!reachable[blocks[3].index()]);
        for regions in [&regions[..4], &regions[..5]] {
            assert!(graph.reachable_blocks(regions)[blocks[0].index()]);
        }
        graph.retain_order(|block| block != blocks[8]);
        assert!(!graph.reachable_blocks(&regions)[blocks[8].index()]);

        let mut straight = BlockGraph::from_items(Vec::new(), &FxHashSet::default());
        straight.push_block(vec![Item::Label(label(0))]);
        straight.push_block(vec![instruction(RETURN_VALUE, Operand::Value(0))]);
        straight.push_block(vec![instruction(NOP, Operand::Value(0))]);
        assert_eq!(straight.reachable_blocks(&[]), [true, true, false]);
        straight.retain_order(|block| block.index() != 0);
        assert_eq!(straight.reachable_blocks(&[]), [false, true, false]);
    }

    #[test]
    fn partitioning_bulk_copies_each_normalized_segment() {
        let entry = Label::from_u32(0);
        let preserved = Label::from_u32(1);
        let jump = Label::from_u32(2);
        let target = Label::from_u32(3);
        let items = vec![
            Item::Label(entry),
            Item::Label(preserved),
            instruction(NOP, Operand::Value(0)),
            Item::Label(jump),
            instruction(JUMP_FORWARD, Operand::Forward(target)),
            Item::Label(target),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];

        let (segments, _) =
            BlockGraph::partition_items(items.clone(), &FxHashSet::from_iter([preserved]));

        assert_eq!(
            segments.iter().map(Vec::len).collect::<Vec<_>>(),
            [1, 2, 2, 2]
        );
        assert!(segments.iter().flatten().zip(items).all(|(left, right)| {
            match (*left, right) {
                (Item::Label(left), Item::Label(right)) => left == right,
                (Item::Instruction(left), Item::Instruction(right)) => {
                    left.opcode == right.opcode && left.operand() == right.operand()
                }
                (Item::Label(_), Item::Instruction(_)) | (Item::Instruction(_), Item::Label(_)) => {
                    false
                }
            }
        }));
    }

    #[test]
    fn prefix_normalization_matches_full_repartition_across_mutations() {
        let labels = std::array::from_fn::<_, 5, _>(Label::from_usize);
        let extra = Label::from_u32(8);
        let graph = BlockGraph::from_items(
            labels
                .into_iter()
                .flat_map(|label| {
                    [
                        Item::Label(label),
                        instruction(NOP, Operand::Value(0)),
                        instruction(RETURN_VALUE, Operand::Value(0)),
                    ]
                })
                .collect(),
            &FxHashSet::default(),
        );
        let assert_graph_eq = |actual: &BlockGraph, expected: &BlockGraph, mask, order_case| {
            assert_eq!(
                actual.order, expected.order,
                "mask {mask} order {order_case}"
            );
            assert_eq!(
                actual.label_blocks, expected.label_blocks,
                "mask {mask} order {order_case}"
            );
            assert_eq!(
                actual.features, expected.features,
                "mask {mask} order {order_case}"
            );
            assert_eq!(
                actual.blocks.len(),
                expected.blocks.len(),
                "mask {mask} order {order_case}"
            );
            for block in actual.blocks.indices() {
                assert_eq!(
                    actual[block].items(),
                    expected[block].items(),
                    "mask {mask} order {order_case} block {block:?}"
                );
            }
        };
        for mask in 0..1024_u32 {
            let mut mutated = graph.clone();
            let original_order = mutated.order.clone();
            let first_instruction = ItemPosition::new(original_order[0], 1);
            mutated.instruction_mut(first_instruction).unwrap().opcode = SWAP;
            for index in 0..4 {
                if mask & (1 << index) != 0 {
                    let block = original_order[index];
                    let last = mutated[block].items().len() - 1;
                    mutated
                        .instruction_mut(ItemPosition::new(block, last))
                        .unwrap()
                        .opcode = NOP;
                }
                if mask & (1 << (index + 4)) != 0 {
                    mutated.remove_item_at(original_order[index + 1], 0);
                }
            }
            if mask & (1 << 8) != 0 {
                let block = original_order[2];
                mutated.insert_label(block, 1, extra);
                mutated.append_to_block(block, [instruction(NOP, Operand::Value(0))]);
            }
            if mask & (1 << 9) != 0 {
                mutated.retain_items(original_order[3], |_| false);
            }
            let boundaries = if mask & 1 == 0 {
                FxHashSet::from_iter([extra])
            } else {
                FxHashSet::default()
            };
            let mut actual = mutated.clone();
            let mut expected = mutated.clone();
            let verified = actual.normalized_prefix(&boundaries);
            let prefix = verified.err().unwrap_or(actual.order.len());
            let pointers = actual.order[..prefix]
                .iter()
                .map(|block| actual[*block].items().as_ptr())
                .collect::<Vec<_>>();
            actual.normalize_blocks_if_needed(&boundaries);
            if verified.is_err() {
                expected.normalize_blocks(&boundaries);
            }
            assert_graph_eq(&actual, &expected, mask, 0);
            for (&block, pointer) in actual.order[..prefix].iter().zip(pointers) {
                assert_eq!(
                    actual[block].items().as_ptr(),
                    pointer,
                    "mask {mask} block {block:?}"
                );
            }

            for order_case in 1..4 {
                let mut final_order = original_order.clone();
                match order_case {
                    1 => final_order.rotate_left(2),
                    2 => final_order.reverse(),
                    3 => {
                        final_order.remove(1);
                    }
                    _ => unreachable!(),
                }
                let mut actual = mutated.clone();
                let mut expected = mutated.clone();
                actual
                    .replace_order_and_normalize(final_order.clone(), &boundaries)
                    .unwrap();
                expected.replace_order(final_order).unwrap();
                if !expected.is_normalized(&boundaries) {
                    expected.normalize_blocks(&boundaries);
                }
                assert_graph_eq(&actual, &expected, mask, order_case);
            }
        }
    }

    #[test]
    fn normalized_segment_lengths_stay_inline_until_the_spill_boundary() {
        for count in [0, 1, 2, 15, 16, 17, 65] {
            let items = (0..count)
                .flat_map(|index| {
                    [
                        Item::Label(Label::from_usize(index)),
                        instruction(LOAD_FAST, Operand::Value(0)),
                        instruction(RETURN_VALUE, Operand::Value(0)),
                    ]
                })
                .collect::<Vec<_>>();
            let (lengths, features) = normalized_segment_lengths(&items, &FxHashSet::default());
            assert_eq!(lengths.as_slice(), vec![3; count]);
            assert_eq!(lengths.spilled(), count > 16);
            assert_eq!(features.has_fast_load(), count > 0);
            assert!(!features.has_jump());

            let graph = BlockGraph::from_items(items.clone(), &FxHashSet::default());
            assert_eq!(graph.len(), count);
            assert_eq!(graph.copy_items(), items);
        }
    }

    #[test]
    fn partitioning_reuses_a_single_segment_allocation() {
        let mut items = Vec::with_capacity(16);
        items.push(Item::Label(Label::from_u32(0)));
        items.push(instruction(NOP, Operand::Value(0)));
        items.push(instruction(RETURN_VALUE, Operand::Value(0)));
        let pointer = items.as_ptr();

        let (segments, _) = BlockGraph::partition_items(items, &FxHashSet::default());

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].as_ptr(), pointer);
    }

    #[test]
    fn initial_label_index_matches_rebuilding_normalized_blocks() {
        use crate::assembler::OperandPatchId;

        let labels = [97, 0, 9, 3, 48, 1, 73].map(Label::from_u32);
        let items = [
            instruction(NOP, Operand::Value(0)),
            Item::Label(labels[0]),
            Item::Label(labels[1]),
            instruction(LOAD_FAST, Operand::Value(0)),
            Item::Label(labels[2]),
            instruction(POP_JUMP_IF_TRUE, Operand::Forward(labels[4])),
            Item::Label(labels[3]),
            Item::Label(labels[4]),
            instruction(JUMP_BACKWARD, Operand::Backward(labels[2])),
            instruction(NOP, Operand::Deferred(OperandPatchId::from_u32(0))),
            Item::Label(labels[5]),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(labels[6]),
        ];

        for boundary_mask in 0..(1 << labels.len()) {
            let boundaries = labels
                .into_iter()
                .enumerate()
                .filter_map(|(index, label)| (boundary_mask & (1 << index) != 0).then_some(label))
                .collect::<FxHashSet<_>>();
            for length in 0..=items.len() {
                let mut graph = BlockGraph::from_items(items[..length].to_vec(), &boundaries);
                let initial = graph.label_blocks.clone();
                graph.rebuild_label_index();
                assert_eq!(
                    initial, graph.label_blocks,
                    "boundary mask {boundary_mask:#09b}, item count {length}"
                );
                assert_eq!(graph.copy_items(), items[..length]);
                assert_eq!(graph.label_block(Label::from_u32(125)), None);
            }
        }
    }

    #[test]
    fn partition_label_binding_matches_rebuilding_reordered_and_added_blocks() {
        let labels = [31, 5, 88, 1, 47].map(Label::from_u32);
        let added_labels = [12, 62, 28].map(Label::from_u32);
        let absent = Label::from_u32(120);
        let reserved = Label::from_u32(121);
        let items = [
            Item::Label(labels[0]),
            instruction(NOP, Operand::Value(0)),
            Item::Label(labels[1]),
            Item::Label(labels[2]),
            instruction(JUMP_FORWARD, Operand::Forward(labels[4])),
            Item::Label(labels[3]),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(labels[4]),
        ];
        let added_items = [
            Item::Label(added_labels[0]),
            Item::Label(added_labels[1]),
            instruction(JUMP_FORWARD, Operand::Forward(added_labels[2])),
            Item::Label(added_labels[2]),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];

        for boundary_mask in 0..(1 << labels.len()) {
            let boundaries = labels
                .into_iter()
                .chain(added_labels)
                .enumerate()
                .filter_map(|(index, label)| {
                    (index >= labels.len() || boundary_mask & (1 << index) != 0).then_some(label)
                })
                .collect::<FxHashSet<_>>();
            for split in 1..items.len() {
                let mut graph = BlockGraph::from_items(Vec::new(), &boundaries);
                graph.push_block(vec![Item::Label(absent)]);
                let first = graph.push_block(items[..split].to_vec());
                graph.allocate_block(vec![Item::Label(reserved)]);
                let second = graph.push_block(items[split..].to_vec());
                graph.replace_order(vec![second, first]).unwrap();
                let expected_items = graph.copy_items();

                graph.normalize_blocks(&boundaries);

                assert!(graph.is_normalized(&boundaries));
                assert_eq!(graph.copy_items(), expected_items);
                assert_eq!(graph.order()[0], second);
                if graph.len() > 1 {
                    assert_eq!(graph.order()[1], first);
                }
                assert_eq!(graph.label_block(absent), None);
                assert_eq!(graph.label_block(reserved), None);
                let normalized_index = graph.label_blocks.clone();
                graph.rebuild_label_index();
                assert_eq!(normalized_index, graph.label_blocks);

                let added = graph.allocate_blocks_from_items(added_items.to_vec(), &boundaries);
                assert_eq!(added.len(), 3);
                for (label, block) in added_labels.into_iter().zip(&added) {
                    assert_eq!(graph.label_block(label), Some(*block));
                }
                graph.order.extend(added);
                let allocated_index = graph.label_blocks.clone();
                graph.rebuild_label_index();
                assert_eq!(allocated_index, graph.label_blocks);
            }
        }
    }

    #[test]
    fn partition_label_binding_matches_previous_allocation_and_normalization() {
        fn normalize_reference(graph: &mut BlockGraph, boundaries: &FxHashSet<Label>) {
            graph.label_blocks.fill(None);
            let (lengths, features) = normalized_segment_lengths(graph.iter_items(), boundaries);
            graph.features.merge(features);
            let original_order = std::mem::take(&mut graph.order);
            let items = original_order
                .iter()
                .copied()
                .flat_map(|block| std::mem::take(&mut graph.blocks[block].items));
            let segments = collect_segments(items, &lengths);
            let mut reusable = original_order.into_iter();
            let mut order = Vec::with_capacity(segments.len());
            for items in segments {
                if let Some(block) = reusable.next() {
                    graph.blocks[block].items = items;
                    order.push(block);
                } else {
                    order.push(graph.allocate_block(items));
                }
            }
            graph.order = order;
            graph.rebuild_label_index();
        }

        fn allocate_reference(
            graph: &mut BlockGraph,
            items: Vec<Item>,
            boundaries: &FxHashSet<Label>,
        ) -> Vec<BasicBlockId> {
            let (segments, features) = BlockGraph::partition_items(items, boundaries);
            graph.features.merge(features);
            segments
                .into_iter()
                .map(|items| graph.allocate_block(items))
                .collect()
        }

        fn assert_graphs_match(actual: &BlockGraph, expected: &BlockGraph) {
            assert_eq!(actual.order, expected.order);
            assert_eq!(actual.label_blocks, expected.label_blocks);
            assert_eq!(actual.features, expected.features);
            assert_eq!(actual.blocks.len(), expected.blocks.len());
            for block in actual.blocks.indices() {
                assert_eq!(actual[block].items(), expected[block].items());
            }
        }

        let labels = [83, 3, 32, 1].map(Label::from_u32);
        let added_labels = [6, 61, 39].map(Label::from_u32);
        let omitted = Label::from_u32(110);
        let detached = Label::from_u32(125);
        let items = [
            Item::Label(labels[0]),
            instruction(NOP, Operand::Value(0)),
            Item::Label(labels[1]),
            Item::Label(labels[2]),
            instruction(LOAD_FAST, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(labels[3]),
        ];
        let added = [
            Item::Label(added_labels[0]),
            Item::Label(added_labels[1]),
            instruction(JUMP_FORWARD, Operand::Forward(added_labels[2])),
            Item::Label(added_labels[2]),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ];

        for boundary_mask in 0..8 {
            let boundaries = [labels[1], labels[2], added_labels[1]]
                .into_iter()
                .enumerate()
                .filter_map(|(index, label)| (boundary_mask & (1 << index) != 0).then_some(label))
                .collect::<FxHashSet<_>>();
            for split_mask in 0..1 << (items.len() - 1) {
                for reverse in [false, true] {
                    let mut base = BlockGraph::from_items(Vec::new(), &boundaries);
                    base.push_block(vec![Item::Label(omitted)]);
                    let mut order = Vec::new();
                    let mut start = 0;
                    for end in 1..=items.len() {
                        if end == items.len() || split_mask & (1 << (end - 1)) != 0 {
                            order.push(base.push_block(items[start..end].to_vec()));
                            start = end;
                        }
                    }
                    if reverse {
                        order.reverse();
                    }
                    base.replace_order(order).unwrap();
                    let detached_block = base.allocate_block(vec![Item::Label(detached)]);

                    let mut actual = base.clone();
                    let mut expected = base.clone();
                    actual.normalize_blocks(&boundaries);
                    normalize_reference(&mut expected, &boundaries);
                    assert_graphs_match(&actual, &expected);
                    assert_eq!(actual.label_block(omitted), None);
                    assert_eq!(actual.label_block(detached), None);

                    for length in [0, added.len()] {
                        let mut actual = base.clone();
                        let mut expected = base.clone();
                        let actual_ids = actual
                            .allocate_blocks_from_items(added[..length].to_vec(), &boundaries);
                        let expected_ids = allocate_reference(
                            &mut expected,
                            added[..length].to_vec(),
                            &boundaries,
                        );
                        assert_eq!(actual_ids, expected_ids);
                        assert_graphs_match(&actual, &expected);
                        assert_eq!(actual.label_block(detached), Some(detached_block));
                    }
                }
            }
        }

        let mut actual = BlockGraph::from_items(Vec::new(), &FxHashSet::default());
        actual.allocate_block(vec![Item::Label(detached)]);
        let mut expected = actual.clone();
        actual.normalize_blocks(&FxHashSet::default());
        normalize_reference(&mut expected, &FxHashSet::default());
        assert_graphs_match(&actual, &expected);
        assert_eq!(actual.label_block(detached), None);
    }

    #[test]
    fn matching_partitions_keep_their_prefix_and_stop_at_the_requested_boundaries() {
        let first = Label::from_u32(0);
        let consecutive = Label::from_u32(1);
        let last = Label::from_u32(2);
        let items = vec![
            instruction(NOP, Operand::Value(0)),
            Item::Label(first),
            instruction(NOP, Operand::Value(1)),
            instruction(SWAP, Operand::Value(2)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            instruction(NOP, Operand::Value(3)),
            Item::Label(consecutive),
            Item::Label(last),
            instruction(NOP, Operand::Value(4)),
            instruction(SWAP, Operand::Value(3)),
            instruction(NOP, Operand::Value(5)),
        ];
        let boundaries = FxHashSet::from_iter([first, consecutive, last]);
        let graph = BlockGraph::from_items(items.clone(), &boundaries);

        let partitions = graph.matching_partition_positions(
            &boundaries,
            |instruction| instruction.opcode == RETURN_VALUE,
            |instruction| instruction.opcode == SWAP,
        );
        let actual = partitions
            .iter()
            .map(|partition| {
                partition
                    .iter()
                    .map(|position| graph.item(*position))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), 2);
        for (actual, expected) in actual.into_iter().zip([&items[1..5], &items[7..]]) {
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.into_iter().zip(expected) {
                match (actual, expected) {
                    (Item::Label(actual), Item::Label(expected)) => assert_eq!(actual, expected),
                    (Item::Instruction(actual), Item::Instruction(expected)) => {
                        assert_eq!(actual.opcode, expected.opcode);
                        assert_eq!(actual.operand(), expected.operand());
                    }
                    _ => panic!("partition item differs from input"),
                }
            }
        }
    }

    #[test]
    fn block_ids_survive_reordering_and_insertion() {
        let first_label = Label::from_u32(0);
        let second_label = Label::from_u32(1);
        let inserted_label = Label::from_u32(2);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(first_label),
                instruction(JUMP_FORWARD, Operand::Forward(second_label)),
                Item::Label(second_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let first = graph.block_at(0);
        let second = graph.block_at(1);

        graph.replace_order(vec![second, first]).unwrap();
        let inserted = graph.insert_block(
            1,
            vec![
                Item::Label(inserted_label),
                instruction(NOP, Operand::Value(0)),
            ],
        );

        assert_eq!(graph.order(), &[second, inserted, first]);
        assert_eq!(graph.label_block(first_label), Some(first));
        assert_eq!(graph.label_block(second_label), Some(second));
        assert_eq!(graph.label_block(inserted_label), Some(inserted));
        graph.validate().unwrap();
        assert_eq!(
            graph.into_items(),
            [
                Item::Label(second_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
                Item::Label(inserted_label),
                instruction(NOP, Operand::Value(0)),
                Item::Label(first_label),
                instruction(JUMP_FORWARD, Operand::Forward(second_label)),
            ]
        );
    }

    #[test]
    fn replacing_order_rejects_duplicate_blocks() {
        let mut graph = BlockGraph::from_items(
            vec![instruction(RETURN_VALUE, Operand::Value(0))],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);

        let error = graph.replace_order(vec![block, block]).unwrap_err();

        assert_eq!(error, "block 0 appears more than once");
        assert_eq!(graph.order(), &[block]);
        graph.validate().unwrap();
    }

    #[test]
    #[should_panic(expected = "block appears more than once in emission order")]
    fn emitting_items_rejects_duplicate_blocks() {
        let mut graph = BlockGraph::from_items(Vec::new(), &FxHashSet::default());
        for _ in 0..130 {
            graph.push_block(vec![instruction(RETURN_VALUE, Operand::Value(0))]);
        }
        graph.order.push(graph.block_at(129));
        graph.into_items();
    }

    #[test]
    fn replacing_order_rejects_blocks_outside_the_arena() {
        let mut graph = BlockGraph::from_items(
            vec![instruction(RETURN_VALUE, Operand::Value(0))],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);

        let error = graph
            .replace_order(vec![BasicBlockId::from_usize(1)])
            .unwrap_err();

        assert_eq!(error, "block 1 is outside the arena");
        assert_eq!(graph.order(), &[block]);
        graph.validate().unwrap();
    }

    #[test]
    fn retaining_order_drops_labels_without_reusing_block_ids() {
        let dropped_label = Label::from_u32(0);
        let retained_label = Label::from_u32(1);
        let inserted_label = Label::from_u32(2);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(dropped_label),
                instruction(JUMP_FORWARD, Operand::Forward(retained_label)),
                Item::Label(retained_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let dropped = graph.block_at(0);
        let retained = graph.block_at(1);

        graph.retain_order(|block| block != dropped);
        let inserted = graph.push_block(vec![
            Item::Label(inserted_label),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);

        assert_eq!(graph.order(), &[retained, inserted]);
        assert_eq!(graph.label_block(dropped_label), None);
        assert_eq!(graph.label_block(retained_label), Some(retained));
        assert_eq!(graph.label_block(inserted_label), Some(inserted));
        assert_eq!(inserted.index(), 2);
        assert!(!graph[dropped].items().is_empty());

        graph.validate().unwrap();
        assert_eq!(
            graph.into_items(),
            [
                Item::Label(retained_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
                Item::Label(inserted_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]
        );
    }

    #[test]
    fn removing_an_item_removes_its_label_from_lookup() {
        let removed_label = Label::from_u32(0);
        let retained_label = Label::from_u32(1);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(removed_label),
                Item::Label(retained_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);

        let removed = graph.remove_item_at(block, 0);

        assert!(matches!(removed, Item::Label(label) if label == removed_label));
        assert_eq!(graph.label_block(removed_label), None);
        assert_eq!(graph.label_block(retained_label), Some(block));
        graph.validate().unwrap();
    }

    #[test]
    fn ordered_instruction_compaction_matches_reverse_removal() {
        let labels = [Label::from_u32(0), Label::from_u32(1), Label::from_u32(2)];
        let mut marked = Instruction::new(NOP, Operand::Value(3), SourceLocation::NONE, None);
        marked.jump_policy.mark_borrow_empty_block();
        let graph = BlockGraph::from_items(
            vec![
                Item::Label(labels[0]),
                instruction(NOP, Operand::Value(0)),
                instruction(NOP, Operand::Value(1)),
                instruction(TO_BOOL, Operand::Value(0)),
                instruction(NOP, Operand::Value(2)),
                Item::Instruction(marked),
                Item::Label(labels[1]),
                instruction(NOP, Operand::Value(4)),
                instruction(NOP, Operand::Value(5)),
                Item::Label(labels[2]),
                instruction(NOP, Operand::Value(6)),
                instruction(NOP, Operand::Value(7)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::from_iter([labels[1], labels[2]]),
        );
        for mask in 0..(1_u32 << 8) {
            let positions = graph
                .iter()
                .flat_map(|(block, basic_block)| {
                    basic_block
                        .items()
                        .iter()
                        .enumerate()
                        .filter_map(move |(offset, item)| {
                            let Item::Instruction(instruction) = item else {
                                return None;
                            };
                            let Operand::Value(bit) = instruction.operand() else {
                                return None;
                            };
                            (instruction.opcode == NOP && mask & (1 << bit) != 0)
                                .then_some(ItemPosition::new(block, offset))
                        })
                })
                .collect::<Vec<_>>();
            let mut reference = graph.clone();
            for position in positions.iter().rev() {
                reference.remove_item(*position);
            }
            let mut compacted = graph.clone();
            compacted.remove_ordered_instructions(&positions);
            let mut duplicated = graph.clone();
            let duplicates = positions
                .iter()
                .flat_map(|&position| [position; 2])
                .collect::<Vec<_>>();
            duplicated.remove_ordered_instructions(&duplicates);
            for actual in [compacted, duplicated] {
                assert_eq!(actual.copy_items(), reference.copy_items(), "mask {mask}");
                assert_eq!(actual.order(), graph.order());
                assert_eq!(actual.features(), graph.features());
                for label in labels {
                    assert_eq!(actual.label_block(label), graph.label_block(label));
                }
                actual.validate().unwrap();
            }
        }
    }

    #[test]
    fn retaining_items_removes_discarded_labels_from_lookup() {
        let removed_label = Label::from_u32(0);
        let retained_label = Label::from_u32(1);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(removed_label),
                Item::Label(retained_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);

        graph.retain_items(
            block,
            |item| !matches!(item, Item::Label(label) if *label == removed_label),
        );

        assert_eq!(graph.label_block(removed_label), None);
        assert_eq!(graph.label_block(retained_label), Some(block));
        graph.validate().unwrap();
    }

    #[test]
    fn inserting_a_label_adds_it_to_lookup() {
        let old_label = Label::from_u32(0);
        let new_label = Label::from_u32(1);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(old_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);

        graph.insert_label(block, 1, new_label);

        assert_eq!(graph.label_block(old_label), Some(block));
        assert_eq!(graph.label_block(new_label), Some(block));
        graph.validate().unwrap();
    }

    #[test]
    fn preserved_consecutive_label_starts_a_distinct_block() {
        let first = Label::from_u32(0);
        let boundary = Label::from_u32(1);
        let graph = BlockGraph::from_items(
            vec![
                Item::Label(first),
                Item::Label(boundary),
                instruction(NOP, Operand::Value(0)),
            ],
            &FxHashSet::from_iter([boundary]),
        );

        assert_eq!(graph.len(), 2);
        assert_ne!(graph.label_block(first), graph.label_block(boundary));
        graph.validate().unwrap();
    }

    #[test]
    fn normalizing_allocates_stable_ids_for_split_suffixes() {
        let first_label = Label::from_u32(0);
        let second_label = Label::from_u32(1);
        let mut graph = BlockGraph {
            blocks: IndexVec::new(),
            order: Vec::new(),
            label_blocks: Vec::new(),
            features: PipelineFeatures::default(),
        };
        let first = graph.push_block(vec![
            Item::Label(first_label),
            instruction(NOP, Operand::Value(0)),
            Item::Label(second_label),
            instruction(RETURN_VALUE, Operand::Value(0)),
        ]);

        assert!(!graph.is_normalized(&FxHashSet::default()));
        graph.normalize_blocks(&FxHashSet::default());
        assert!(graph.is_normalized(&FxHashSet::default()));

        let suffix = graph.block_at(1);
        assert_eq!(graph.block_at(0), first);
        assert_ne!(suffix, first);
        assert_eq!(graph.label_block(first_label), Some(first));
        assert_eq!(graph.label_block(second_label), Some(suffix));
        graph.validate().unwrap();
    }

    #[test]
    fn streaming_normalization_check_matches_segment_planner() {
        let entry = Label::from_u32(0);
        let preserved = Label::from_u32(1);
        let internal = Label::from_u32(2);
        let jump = Label::from_u32(3);
        let target = Label::from_u32(4);
        let items = vec![
            Item::Label(entry),
            Item::Label(preserved),
            instruction(NOP, Operand::Value(0)),
            Item::Label(internal),
            instruction(JUMP_FORWARD, Operand::Forward(target)),
            Item::Label(jump),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(target),
            instruction(NOP, Operand::Value(0)),
        ];

        for boundary_mask in 0..8 {
            let boundaries = [preserved, internal, jump]
                .into_iter()
                .enumerate()
                .filter_map(|(index, label)| (boundary_mask & (1 << index) != 0).then_some(label))
                .collect::<FxHashSet<_>>();
            let (normalized_lengths, _) = normalized_segment_lengths(&items, &boundaries);
            for split_mask in 0..(1 << (items.len() - 1)) {
                let mut graph = BlockGraph {
                    blocks: IndexVec::new(),
                    order: Vec::new(),
                    label_blocks: Vec::new(),
                    features: PipelineFeatures::default(),
                };
                let mut start = 0;
                let mut physical_lengths = Vec::new();
                for end in 1..=items.len() {
                    if end == items.len() || split_mask & (1 << (end - 1)) != 0 {
                        physical_lengths.push(end - start);
                        graph.push_block(items[start..end].to_vec());
                        start = end;
                    }
                }

                assert_eq!(
                    graph.is_normalized(&boundaries),
                    physical_lengths.as_slice() == normalized_lengths.as_slice(),
                    "boundary mask {boundary_mask:#05b}, split mask {split_mask:#010b}"
                );
            }
        }
    }

    #[test]
    fn replacing_and_normalizing_order_matches_separate_label_rebuild() {
        fn reference(
            graph: &mut BlockGraph,
            order: Vec<BasicBlockId>,
            boundaries: &FxHashSet<Label>,
        ) {
            graph.validate_order(&order).unwrap();
            graph.order = order;
            if graph.is_normalized(boundaries) {
                graph.rebuild_label_index();
            } else {
                graph.normalize_blocks(boundaries);
            }
        }

        let labels = [
            Label::from_u32(0),
            Label::from_u32(4),
            Label::from_u32(11),
            Label::from_u32(23),
        ];
        let items = [
            Item::Label(labels[0]),
            Item::Label(labels[1]),
            instruction(NOP, Operand::Value(0)),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(labels[2]),
            instruction(NOP, Operand::Value(0)),
            Item::Label(labels[3]),
        ];
        let mut normalized_cases = 0;
        let mut unnormalized_cases = 0;
        for boundary_mask in 0..4 {
            let boundaries = [labels[1], labels[3]]
                .into_iter()
                .enumerate()
                .filter_map(|(index, label)| (boundary_mask & (1 << index) != 0).then_some(label))
                .collect::<FxHashSet<_>>();
            for split_mask in 0..(1 << (items.len() - 1)) {
                let mut graph = BlockGraph::from_items(Vec::new(), &FxHashSet::default());
                let mut start = 0;
                for end in 1..=items.len() {
                    if end == items.len() || split_mask & (1 << (end - 1)) != 0 {
                        graph.push_block(items[start..end].to_vec());
                        start = end;
                    }
                }
                // Both an omitted nonempty block and an emitted empty block exercise
                // entries that the previous label index cannot be allowed to keep.
                let omitted = graph.allocate_block(vec![Item::Label(Label::from_u32(29))]);
                let empty = graph.allocate_block(vec![instruction(NOP, Operand::Value(0))]);
                graph.remove_item_at(empty, 0);
                let original_order = graph.order().to_vec();
                let orders = [
                    original_order.clone(),
                    original_order.iter().copied().rev().collect(),
                    original_order.iter().copied().skip(1).collect(),
                    original_order.iter().copied().chain([empty]).collect(),
                    [empty]
                        .into_iter()
                        .chain(original_order.iter().copied())
                        .collect(),
                    vec![omitted],
                    vec![],
                ];
                for order in orders {
                    let mut actual = graph.clone();
                    let mut expected = graph.clone();
                    expected.order.clone_from(&order);
                    if expected.is_normalized(&boundaries) {
                        normalized_cases += 1;
                    } else {
                        unnormalized_cases += 1;
                    }
                    reference(&mut expected, order.clone(), &boundaries);
                    actual
                        .replace_order_and_normalize(order, &boundaries)
                        .unwrap();
                    assert_eq!(actual.order(), expected.order());
                    assert_eq!(actual.label_blocks, expected.label_blocks);
                    assert_eq!(actual.copy_items(), expected.copy_items());
                    assert_eq!(actual.features(), expected.features());
                    assert_eq!(actual.blocks.len(), expected.blocks.len());
                    for block in actual.blocks.indices() {
                        assert_eq!(actual[block].items(), expected[block].items());
                    }
                    actual.validate().unwrap();
                }
            }
        }
        assert!(normalized_cases > 100);
        assert!(unnormalized_cases > 100);

        let mut graph = BlockGraph::from_items(vec![Item::Label(labels[0])], &FxHashSet::default());
        let order = graph.order().to_vec();
        let label_blocks = graph.label_blocks.clone();
        for invalid in [vec![order[0]; 2], vec![BasicBlockId::from_usize(100)]] {
            assert!(
                graph
                    .replace_order_and_normalize(invalid, &FxHashSet::default())
                    .is_err()
            );
            assert_eq!(graph.order(), order);
            assert_eq!(graph.label_blocks, label_blocks);
        }
    }

    #[test]
    fn recognizes_a_normalized_jump_free_single_block() {
        let label = Label::from_u32(0);
        let graph = BlockGraph::from_items(
            vec![
                Item::Label(label),
                instruction(NOP, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );

        assert!(graph.is_normalized_jump_free_single_block(&FxHashSet::default()));
    }

    #[test]
    fn single_block_fast_path_rejects_normalization_boundaries_and_jumps() {
        let first = Label::from_u32(0);
        let internal = Label::from_u32(1);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(first),
                instruction(NOP, Operand::Value(0)),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);
        graph.insert_label(block, 2, internal);
        assert!(!graph.is_normalized_jump_free_single_block(&FxHashSet::default()));

        let target = Label::from_u32(2);
        let graph = BlockGraph::from_items(
            vec![
                Item::Label(first),
                instruction(JUMP_FORWARD, Operand::Forward(target)),
            ],
            &FxHashSet::default(),
        );
        assert!(!graph.is_normalized_jump_free_single_block(&FxHashSet::default()));

        let preserved = Label::from_u32(3);
        let graph = BlockGraph::from_items(
            vec![
                Item::Label(first),
                Item::Label(preserved),
                instruction(NOP, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        assert!(!graph.is_normalized_jump_free_single_block(&FxHashSet::from_iter([preserved,])));
    }

    #[test]
    fn initial_boundary_removal_matches_unpreserved_partition_and_reachability() {
        let labels = [0, 1, 2, 3, 4, 5].map(Label::from_u32);
        let items = vec![
            Item::Label(labels[0]),
            Item::Label(labels[1]),
            instruction(JUMP_FORWARD, Operand::Forward(labels[3])),
            Item::Label(labels[2]),
            instruction(NOP, Operand::Value(0)),
            Item::Label(labels[3]),
            instruction(RETURN_VALUE, Operand::Value(0)),
            Item::Label(labels[4]),
            Item::Label(labels[5]),
        ];
        let expected = BlockGraph::from_items(items.clone(), &FxHashSet::default());
        let expected_reachable = expected.reachable_blocks(&[]);
        for mask in 0..1 << labels.len() {
            let boundaries = labels
                .into_iter()
                .enumerate()
                .filter_map(|(index, label)| (mask & (1 << index) != 0).then_some(label))
                .collect();
            let mut graph = BlockGraph::from_items(items.clone(), &boundaries);
            let mut reachable = graph.reachable_blocks(&[]);
            graph.remove_initial_preserved_boundaries(Some(&mut reachable));
            assert_eq!(reachable, expected_reachable, "boundary mask {mask:#08b}");
            assert_eq!(graph.order(), expected.order(), "boundary mask {mask:#08b}");
            assert_eq!(graph.label_blocks, expected.label_blocks);
            assert_eq!(graph.features(), expected.features());
            assert!(
                graph.iter().zip(expected.iter()).all(|(left, right)| {
                    left.0 == right.0 && left.1.items() == right.1.items()
                })
            );
            graph.validate().unwrap();
        }

        for items in [Vec::new(), vec![Item::Label(labels[0])]] {
            let mut graph = BlockGraph::from_items(items, &FxHashSet::from_iter([labels[0]]));
            let mut reachable = graph.reachable_blocks(&[]);
            let order_pointer = graph.order.as_ptr();
            graph.remove_initial_preserved_boundaries(Some(&mut reachable));
            assert_eq!(graph.order.as_ptr(), order_pointer);
            assert_eq!(reachable, graph.reachable_blocks(&[]));
        }
    }

    #[test]
    fn removes_only_initial_preserved_boundaries() {
        let entry = Label::from_u32(0);
        let preserved = Label::from_u32(1);
        let target = Label::from_u32(2);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(entry),
                Item::Label(preserved),
                instruction(NOP, Operand::Value(0)),
                Item::Label(target),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::from_iter([preserved]),
        );
        assert_eq!(graph.len(), 3);

        graph.remove_initial_preserved_boundaries(None);

        assert_eq!(graph.len(), 2);
        assert_eq!(graph.block_at(0).index(), 0);
        assert_eq!(graph.block_at(1).index(), 1);
        assert_eq!(graph.label_block(entry), Some(graph.block_at(0)));
        assert_eq!(graph.label_block(preserved), Some(graph.block_at(0)));
        assert_eq!(graph.label_block(target), Some(graph.block_at(1)));
        graph.validate().unwrap();
    }
}
