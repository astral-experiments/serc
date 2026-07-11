use std::ops::Index;

use rustc_hash::FxHashSet;

use super::{
    ExceptionRegion, Instruction, Item, Label, Operand, block_has_fallthrough, block_jump_target,
    ends_scope,
};

/// A stable handle to a basic block.
///
/// Blocks live in an append-only arena. Reordering or removing a block from the
/// emitted order therefore never changes the identity of any other block.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct BasicBlockId(usize);

const UNBOUND_BLOCK: BasicBlockId = BasicBlockId(usize::MAX);

impl BasicBlockId {
    pub(super) const fn index(self) -> usize {
        self.0
    }
}

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
        self.items.iter().rev().find_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        })
    }
}

/// The assembler's block-aware intermediate representation.
///
/// `blocks` is an append-only arena and `order` is the current emission order.
/// Keeping those concepts separate lets CFG passes reorder or duplicate blocks
/// without invalidating identities captured by data-flow state.
#[derive(Debug)]
pub(super) struct BlockGraph {
    blocks: Vec<BasicBlock>,
    order: Vec<BasicBlockId>,
    label_blocks: Vec<BasicBlockId>,
}

fn normalized_segment_lengths<'a>(
    items: impl IntoIterator<Item = &'a Item>,
    item_count: usize,
    preserved_block_boundaries: &FxHashSet<Label>,
) -> Vec<usize> {
    let mut lengths = Vec::with_capacity(item_count);
    let mut length = 0;
    let mut has_instruction = false;
    for item in items {
        if let Item::Label(label) = item
            && (has_instruction || (length > 0 && preserved_block_boundaries.contains(label)))
        {
            lengths.push(length);
            length = 0;
            has_instruction = false;
        }
        let ends_block = matches!(item, Item::Instruction(instruction) if
            !matches!(instruction.operand, Operand::Value(_))
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
    lengths
}

fn collect_segments<T>(
    items: impl IntoIterator<Item = T>,
    segment_lengths: Vec<usize>,
) -> Vec<Vec<T>> {
    let mut items = items.into_iter();
    let mut segments = Vec::with_capacity(segment_lengths.len());
    for length in segment_lengths {
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

impl BlockGraph {
    fn partition_items(
        items: Vec<Item>,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> Vec<Vec<Item>> {
        let lengths = normalized_segment_lengths(&items, items.len(), preserved_block_boundaries);
        collect_segments(items, lengths)
    }

    pub(super) fn from_items(
        items: Vec<Item>,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> Self {
        let blocks = Self::partition_items(items, preserved_block_boundaries)
            .into_iter()
            .map(BasicBlock::new)
            .collect::<Vec<_>>();
        let order = (0..blocks.len()).map(BasicBlockId).collect();
        let mut graph = Self {
            blocks,
            order,
            label_blocks: Vec::new(),
        };
        graph.rebuild_label_index();
        graph
    }

    pub(super) fn len(&self) -> usize {
        self.order.len()
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
                    if !matches!(instruction.operand, Operand::Value(_)) {
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
        if self.is_normalized(preserved_block_boundaries) {
            self.rebuild_label_index();
        } else {
            self.normalize_blocks(preserved_block_boundaries);
        }
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
        self.label_blocks
            .get(usize::try_from(label.0).unwrap())
            .copied()
            .filter(|block| *block != UNBOUND_BLOCK)
    }

    fn clear_label_index(&mut self) {
        self.label_blocks.fill(UNBOUND_BLOCK);
    }

    fn bind_label(&mut self, label: Label, block: BasicBlockId) {
        let index = usize::try_from(label.0).unwrap();
        if index >= self.label_blocks.len() {
            self.label_blocks.resize(index + 1, UNBOUND_BLOCK);
        }
        let previous = std::mem::replace(&mut self.label_blocks[index], block);
        debug_assert_eq!(previous, UNBOUND_BLOCK, "label is bound more than once");
    }

    fn unbind_label(&mut self, label: Label) {
        let index = usize::try_from(label.0).unwrap();
        if let Some(block) = self.label_blocks.get_mut(index) {
            *block = UNBOUND_BLOCK;
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
        let block = BasicBlockId(self.blocks.len());
        self.blocks.push(BasicBlock::new(items));
        let labels = self[block]
            .items()
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        for label in labels {
            self.bind_label(label, block);
        }
        block
    }

    pub(super) fn allocate_blocks_from_items(
        &mut self,
        items: Vec<Item>,
        preserved_block_boundaries: &FxHashSet<Label>,
    ) -> Vec<BasicBlockId> {
        Self::partition_items(items, preserved_block_boundaries)
            .into_iter()
            .map(|items| self.allocate_block(items))
            .collect()
    }

    /// Repartitions existing blocks without changing the identity of each
    /// block's first resulting segment.
    pub(super) fn normalize_blocks(&mut self, preserved_block_boundaries: &FxHashSet<Label>) {
        // Labels may move from an existing block into a newly allocated suffix.
        // Rebuild the index after repartitioning instead of treating that move
        // as a duplicate binding.
        self.clear_label_index();
        let item_count = self
            .order
            .iter()
            .map(|block| self[*block].items.len())
            .sum();
        let segment_lengths =
            normalized_segment_lengths(self.iter_items(), item_count, preserved_block_boundaries);
        let original_order = std::mem::take(&mut self.order);
        let items = original_order
            .iter()
            .copied()
            .flat_map(|block| std::mem::take(&mut self.blocks[block.index()].items));
        let segments = collect_segments(items, segment_lengths);

        let mut reusable = original_order.into_iter();
        let mut order = Vec::with_capacity(segments.len());
        for items in segments {
            if let Some(block) = reusable.next() {
                self.blocks[block.index()].items = items;
                order.push(block);
            } else {
                order.push(self.allocate_block(items));
            }
        }
        self.order = order;
        self.rebuild_label_index();
    }

    /// Returns whether normalization would preserve every physical block boundary.
    pub(super) fn is_normalized(&self, boundaries: &FxHashSet<Label>) -> bool {
        let lengths = self
            .order
            .iter()
            .map(|block| self[*block].items.len())
            .collect::<Vec<_>>();
        let item_count = lengths.iter().sum();
        normalized_segment_lengths(self.iter_items(), item_count, boundaries) == lengths
    }

    /// Repartitions the ordered instruction stream for a pass whose logical
    /// block boundaries are narrower than the assembler's structural ones.
    /// Existing block IDs are reused in order; newly split suffixes receive
    /// fresh IDs from the append-only arena.
    pub(super) fn repartition(
        &mut self,
        boundary_labels: &FxHashSet<Label>,
        starts_block: impl Fn(&Instruction) -> bool,
        ends_block: impl Fn(&Instruction) -> bool,
    ) {
        let item_count = self
            .order
            .iter()
            .map(|block| self[*block].items.len())
            .sum();
        let mut segment_lengths = Vec::with_capacity(item_count);
        let mut segment_length = 0;
        let mut has_instruction = false;
        for (_, block) in self.iter() {
            for item in block.items() {
                let starts_here = match item {
                    Item::Label(label) => boundary_labels.contains(label),
                    Item::Instruction(instruction) => starts_block(instruction),
                };
                if starts_here && has_instruction {
                    segment_lengths.push(segment_length);
                    segment_length = 0;
                    has_instruction = false;
                }
                let ends_here = match item {
                    Item::Instruction(instruction) => ends_block(instruction),
                    Item::Label(_) => false,
                };
                has_instruction |= matches!(item, Item::Instruction(_));
                segment_length += 1;
                if ends_here {
                    segment_lengths.push(segment_length);
                    segment_length = 0;
                    has_instruction = false;
                }
            }
        }
        if segment_length > 0 {
            segment_lengths.push(segment_length);
        }

        if segment_lengths.len() == self.order.len()
            && self
                .order
                .iter()
                .zip(&segment_lengths)
                .all(|(block, length)| self[*block].items.len() == *length)
        {
            return;
        }

        self.clear_label_index();
        let original_order = std::mem::take(&mut self.order);
        let items = original_order
            .iter()
            .copied()
            .flat_map(|block| std::mem::take(&mut self.blocks[block.index()].items));
        let segments = collect_segments(items, segment_lengths);

        let mut reusable = original_order.into_iter();
        let mut order = Vec::with_capacity(segments.len());
        for items in segments {
            if let Some(block) = reusable.next() {
                self.blocks[block.index()].items = items;
                order.push(block);
            } else {
                order.push(self.allocate_block(items));
            }
        }
        self.order = order;
        self.rebuild_label_index();
    }

    pub(super) fn append_to_block(
        &mut self,
        block: BasicBlockId,
        items: impl IntoIterator<Item = Item>,
    ) {
        debug_assert!(self.order.contains(&block));
        let start = self[block].items.len();
        self.blocks[block.index()].items.extend(items);
        let labels = self[block].items[start..]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        for label in labels {
            self.bind_label(label, block);
        }
    }

    pub(super) fn insert_label(&mut self, block: BasicBlockId, position: usize, label: Label) {
        debug_assert!(self.order.contains(&block));
        self.blocks[block.index()]
            .items
            .insert(position, Item::Label(label));
        self.bind_label(label, block);
    }

    /// Removes one item while keeping label lookup synchronized.
    pub(super) fn remove_item_at(&mut self, block: BasicBlockId, position: usize) -> Item {
        let item = self.blocks[block.index()].items.remove(position);
        if let Item::Label(label) = item
            && self.label_block(label) == Some(block)
        {
            self.unbind_label(label);
        }
        item
    }

    /// Retains items that satisfy `keep` while keeping label lookup synchronized.
    pub(super) fn retain_items(
        &mut self,
        block: BasicBlockId,
        mut keep: impl FnMut(&Item) -> bool,
    ) {
        let items = &mut self.blocks[block.index()].items;
        let label_blocks = &mut self.label_blocks;
        items.retain(|item| {
            let retain = keep(item);
            if !retain && let Item::Label(label) = item {
                let index = usize::try_from(label.0).unwrap();
                if label_blocks.get(index).copied() == Some(block) {
                    label_blocks[index] = UNBOUND_BLOCK;
                }
            }
            retain
        });
    }

    fn rebuild_label_index(&mut self) {
        let mut label_blocks = std::mem::take(&mut self.label_blocks);
        label_blocks.fill(UNBOUND_BLOCK);
        for block in self.order.iter().copied() {
            for item in self[block].items() {
                if let Item::Label(label) = item {
                    let index = usize::try_from(label.0).unwrap();
                    if index >= label_blocks.len() {
                        label_blocks.resize(index + 1, UNBOUND_BLOCK);
                    }
                    let previous = std::mem::replace(&mut label_blocks[index], block);
                    debug_assert_eq!(previous, UNBOUND_BLOCK, "label is bound more than once");
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
        self.blocks[block.index()]
            .items
            .iter_mut()
            .map(|item| match item {
                Item::Label(_) => ItemMut::Label,
                Item::Instruction(instruction) => ItemMut::Instruction(instruction),
            })
    }

    /// Replaces block contents while preserving the exact label sequence.
    pub(super) fn replace_items_preserving_labels(
        &mut self,
        block: BasicBlockId,
        items: impl Iterator<Item = Item> + Clone,
    ) {
        let old_labels = self[block].items().iter().filter_map(|item| match item {
            Item::Label(label) => Some(*label),
            Item::Instruction(_) => None,
        });
        let new_labels = items.clone().filter_map(|item| match item {
            Item::Label(label) => Some(label),
            Item::Instruction(_) => None,
        });
        assert!(
            old_labels.eq(new_labels),
            "label-preserving replacement changed block labels"
        );
        let block_items = &mut self.blocks[block.index()].items;
        block_items.clear();
        block_items.extend(items);
    }

    /// Fuses non-overlapping adjacent instruction pairs without exposing labels.
    pub(super) fn fuse_adjacent_instructions(
        &mut self,
        block: BasicBlockId,
        mut fuse: impl FnMut(Instruction, Instruction) -> Option<Instruction>,
    ) {
        let items = &mut self.blocks[block.index()].items;
        let original_len = items.len();
        let mut read = 0;
        let mut write = 0;
        while read < original_len {
            let fused = read
                .checked_add(1)
                .filter(|second| *second < original_len)
                .and_then(|second| match (items[read], items[second]) {
                    (Item::Instruction(first), Item::Instruction(second)) => fuse(first, second),
                    (Item::Label(_), _) | (_, Item::Label(_)) => None,
                });
            if let Some(instruction) = fused {
                let Item::Instruction(second) = items[read + 1] else {
                    unreachable!("a fused pair must contain two instructions");
                };
                // CPython resolves missing locations before replacing the second half of a
                // superinstruction with a removable NOP. For a forward source-order pair,
                // preserve that location on an immediately following instruction before
                // compacting the pair. Reversed assignment stores retain the first location.
                if let Some(Item::Instruction(following)) = items.get_mut(read + 2)
                    && following.location.line < 0
                    && !following.no_location_policy.preserve
                    && second.location.line == instruction.location.line
                    && second.location.column > instruction.location.column
                {
                    following.location = second.location;
                }
                items[write] = Item::Instruction(instruction);
                read += 2;
            } else {
                items[write] = items[read];
                read += 1;
            }
            write += 1;
        }
        items.truncate(write);
    }

    fn validate_order(&self, order: &[BasicBlockId]) -> Result<(), String> {
        let mut seen_blocks = FxHashSet::default();
        for block in order.iter().copied() {
            if !seen_blocks.insert(block) {
                return Err(format!("block {} appears more than once", block.index()));
            }
            if self.blocks.get(block.index()).is_none() {
                return Err(format!("block {} is outside the arena", block.index()));
            }
        }
        Ok(())
    }

    /// Removes the extra label-only blocks introduced by the preserved
    /// boundaries used for the pre-assembly reachability scan.
    ///
    /// This is only valid before any pass has reordered or allocated blocks.
    pub(super) fn remove_initial_preserved_boundaries(&mut self) {
        debug_assert_eq!(self.order.len(), self.blocks.len());
        debug_assert!(
            self.order
                .iter()
                .enumerate()
                .all(|(index, block)| block.index() == index)
        );

        let original = std::mem::take(&mut self.blocks);
        let mut blocks: Vec<BasicBlock> = Vec::with_capacity(original.len());
        for mut block in original {
            if blocks.last().is_some_and(|previous| {
                previous
                    .items()
                    .iter()
                    .all(|item| matches!(item, Item::Label(_)))
            }) {
                blocks.last_mut().unwrap().items.append(&mut block.items);
            } else {
                blocks.push(block);
            }
        }
        self.blocks = blocks;
        self.order = (0..self.blocks.len()).map(BasicBlockId).collect();
        self.rebuild_label_index();
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (BasicBlockId, &BasicBlock)> {
        self.order
            .iter()
            .copied()
            .map(|block| (block, &self[block]))
    }

    pub(super) fn iter_items(&self) -> impl Iterator<Item = &Item> {
        self.iter().flat_map(|(_, block)| block.items())
    }

    pub(super) fn for_each_instruction_mut(&mut self, mut visit: impl FnMut(&mut Instruction)) {
        let blocks = &mut self.blocks;
        for block in self.order.iter().copied() {
            for item in &mut blocks[block.index()].items {
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
        self.blocks[block.index()]
            .items
            .iter_mut()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(instruction),
                Item::Label(_) => None,
            })
    }

    pub(super) fn for_each_adjacent_instruction_pair_mut(
        &mut self,
        block: BasicBlockId,
        mut visit: impl FnMut(&mut Instruction, &mut Instruction),
    ) {
        let items = &mut self.blocks[block.index()].items;
        for index in 0..items.len().saturating_sub(1) {
            let (before, after) = items.split_at_mut(index + 1);
            if let (Item::Instruction(first), Item::Instruction(second)) =
                (&mut before[index], &mut after[0])
            {
                visit(first, second);
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

    pub(super) fn block_item_positions(&self, block: BasicBlockId) -> Vec<ItemPosition> {
        (0..self[block].items().len())
            .map(|offset| ItemPosition { block, offset })
            .collect()
    }

    /// Builds a pass-local ordered view of block contents without moving the
    /// graph's owned items or changing stable block identities.
    pub(super) fn partition_positions(
        &self,
        boundary_labels: &FxHashSet<Label>,
        ends_block: impl Fn(&Instruction) -> bool,
    ) -> Vec<Vec<ItemPosition>> {
        let positions = self.item_positions();
        let mut segment_lengths = Vec::with_capacity(positions.len());
        let mut segment_length = 0;
        for position in &positions {
            let item = self.item(*position);
            if matches!(item, Item::Label(label) if boundary_labels.contains(label))
                && segment_length > 0
            {
                segment_lengths.push(segment_length);
                segment_length = 0;
            }
            let ends_here =
                matches!(item, Item::Instruction(instruction) if ends_block(instruction));
            segment_length += 1;
            if ends_here {
                segment_lengths.push(segment_length);
                segment_length = 0;
            }
        }
        if segment_length > 0 {
            segment_lengths.push(segment_length);
        }
        collect_segments(positions, segment_lengths)
    }

    pub(super) fn item(&self, position: ItemPosition) -> &Item {
        &self[position.block].items[position.offset]
    }

    pub(super) fn instruction_mut(&mut self, position: ItemPosition) -> Option<&mut Instruction> {
        match &mut self.blocks[position.block.index()].items[position.offset] {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        }
    }

    pub(super) fn instruction_at_mut(
        &mut self,
        block: BasicBlockId,
        offset: usize,
    ) -> Option<&mut Instruction> {
        match &mut self.blocks[block.index()].items[offset] {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        }
    }

    pub(super) fn insert_item(&mut self, position: ItemPosition, item: Item) {
        debug_assert!(self.order.contains(&position.block));
        if let Item::Label(label) = item {
            self.bind_label(label, position.block);
        }
        self.blocks[position.block.index()]
            .items
            .insert(position.offset, item);
    }

    pub(super) fn remove_item(&mut self, position: ItemPosition) -> Item {
        self.remove_item_at(position.block, position.offset)
    }

    pub(super) fn reachable_blocks(&self, exception_regions: &[ExceptionRegion]) -> Vec<bool> {
        let mut reachable = vec![false; self.blocks.len()];
        let Some(entry) = self.order.first().copied() else {
            return reachable;
        };
        let mut order_positions = vec![usize::MAX; self.blocks.len()];
        for (position, block) in self.order.iter().copied().enumerate() {
            order_positions[block.index()] = position;
        }
        let region_blocks = exception_regions
            .iter()
            .filter_map(|region| {
                Some((
                    self.label_block(region.start)?,
                    self.label_block(region.end)?,
                    self.label_block(region.target)?,
                ))
            })
            .collect::<Vec<_>>();
        let mut pending = vec![entry];
        pending.extend(
            region_blocks
                .iter()
                .filter_map(|(start, end, handler)| (start == end).then_some(*handler)),
        );
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
            for (start, end, handler) in &region_blocks {
                if order_positions[start.index()] <= position
                    && position < order_positions[end.index()]
                {
                    pending.push(*handler);
                }
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

    pub(super) fn into_items(self) -> Vec<Item> {
        let mut blocks = self.blocks.into_iter().map(Some).collect::<Vec<_>>();
        let item_count = self
            .order
            .iter()
            .map(|block| {
                blocks[block.index()]
                    .as_ref()
                    .expect("block appears more than once in emission order")
                    .items
                    .len()
            })
            .sum();
        let mut items = Vec::with_capacity(item_count);
        for block in self.order {
            items.append(
                &mut blocks[block.index()]
                    .take()
                    .expect("block appears more than once in emission order")
                    .items,
            );
        }
        items
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn validate(&self) -> Result<(), String> {
        self.validate_order(&self.order)?;
        let mut labels = Vec::new();
        for block in self.order.iter().copied() {
            let contents = &self.blocks[block.index()];
            if contents.items.is_empty() {
                return Err(format!("block {} is empty", block.index()));
            }
            for item in contents.items() {
                if let Item::Label(label) = item {
                    let index = usize::try_from(label.0).unwrap();
                    if index >= labels.len() {
                        labels.resize(index + 1, UNBOUND_BLOCK);
                    }
                    if std::mem::replace(&mut labels[index], block) != UNBOUND_BLOCK {
                        return Err(format!("label {} is bound more than once", label.0));
                    }
                }
            }
        }
        while labels.last() == Some(&UNBOUND_BLOCK) {
            labels.pop();
        }
        let mut indexed = self.label_blocks.as_slice();
        while indexed.last() == Some(&UNBOUND_BLOCK) {
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
        &self.blocks[index.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::SourceLocation;
    use crate::target::opcodes::{JUMP_FORWARD, NOP, RETURN_VALUE};

    fn instruction(opcode: crate::target::Opcode, operand: Operand) -> Item {
        Item::Instruction(Instruction::new(
            opcode,
            operand,
            SourceLocation::NONE,
            None,
        ))
    }

    #[test]
    fn block_ids_survive_reordering_and_insertion() {
        let first_label = Label(0);
        let second_label = Label(1);
        let inserted_label = Label(2);
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
    fn replacing_order_rejects_blocks_outside_the_arena() {
        let mut graph = BlockGraph::from_items(
            vec![instruction(RETURN_VALUE, Operand::Value(0))],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);

        let error = graph.replace_order(vec![BasicBlockId(1)]).unwrap_err();

        assert_eq!(error, "block 1 is outside the arena");
        assert_eq!(graph.order(), &[block]);
        graph.validate().unwrap();
    }

    #[test]
    fn retaining_order_drops_labels_without_reusing_block_ids() {
        let dropped_label = Label(0);
        let retained_label = Label(1);
        let inserted_label = Label(2);
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
    }

    #[test]
    fn removing_an_item_removes_its_label_from_lookup() {
        let removed_label = Label(0);
        let retained_label = Label(1);
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
    fn retaining_items_removes_discarded_labels_from_lookup() {
        let removed_label = Label(0);
        let retained_label = Label(1);
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
    #[should_panic(expected = "label-preserving replacement changed block labels")]
    fn replacing_items_preserving_labels_rejects_changed_labels() {
        let old_label = Label(0);
        let new_label = Label(1);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(old_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let block = graph.block_at(0);

        graph.replace_items_preserving_labels(
            block,
            [
                Item::Label(new_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ]
            .into_iter(),
        );
    }

    #[test]
    fn inserting_a_label_adds_it_to_lookup() {
        let old_label = Label(0);
        let new_label = Label(1);
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
        let first = Label(0);
        let boundary = Label(1);
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
    fn repartition_reuses_the_leading_block_id_and_moves_labels() {
        let first_label = Label(0);
        let second_label = Label(1);
        let mut graph = BlockGraph::from_items(
            vec![
                Item::Label(first_label),
                instruction(NOP, Operand::Value(0)),
                Item::Label(second_label),
                instruction(RETURN_VALUE, Operand::Value(0)),
            ],
            &FxHashSet::default(),
        );
        let first = graph.block_at(0);
        let second = graph.block_at(1);

        graph.repartition(&FxHashSet::default(), |_| false, |_| false);

        assert_eq!(graph.order(), &[first]);
        assert_ne!(first, second);
        assert_eq!(graph.label_block(first_label), Some(first));
        assert_eq!(graph.label_block(second_label), Some(first));
        graph.validate().unwrap();
    }

    #[test]
    fn normalizing_allocates_stable_ids_for_split_suffixes() {
        let first_label = Label(0);
        let second_label = Label(1);
        let mut graph = BlockGraph {
            blocks: Vec::new(),
            order: Vec::new(),
            label_blocks: Vec::new(),
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
    fn recognizes_a_normalized_jump_free_single_block() {
        let label = Label(0);
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
        let first = Label(0);
        let internal = Label(1);
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

        let target = Label(2);
        let graph = BlockGraph::from_items(
            vec![
                Item::Label(first),
                instruction(JUMP_FORWARD, Operand::Forward(target)),
            ],
            &FxHashSet::default(),
        );
        assert!(!graph.is_normalized_jump_free_single_block(&FxHashSet::default()));

        let preserved = Label(3);
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
    fn removes_only_initial_preserved_boundaries() {
        let entry = Label(0);
        let preserved = Label(1);
        let target = Label(2);
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

        graph.remove_initial_preserved_boundaries();

        assert_eq!(graph.len(), 2);
        assert_eq!(graph.block_at(0).index(), 0);
        assert_eq!(graph.block_at(1).index(), 1);
        assert_eq!(graph.label_block(entry), Some(graph.block_at(0)));
        assert_eq!(graph.label_block(preserved), Some(graph.block_at(0)));
        assert_eq!(graph.label_block(target), Some(graph.block_at(1)));
        graph.validate().unwrap();
    }
}
