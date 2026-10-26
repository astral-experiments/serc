//! Exact native labels retained independently of Serc's physical block labels.

use std::ops::Index;

use rustc_hash::FxHashMap;

use super::block_graph::{BasicBlockId, BlockGraph};
use super::ir::{Item, Label};

/// Original arrays keyed by stable block IDs; later copied blocks remain absent.
#[derive(Debug)]
pub(super) struct LegacyInstructionArrays(Vec<Option<usize>>);

impl LegacyInstructionArrays {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    pub(super) fn insert(&mut self, block: BasicBlockId, array: usize) {
        let index = block.index();
        if index >= self.0.len() {
            self.0.resize(index + 1, None);
        }
        self.0[index] = Some(array);
    }

    pub(super) fn get(&self, block: BasicBlockId) -> Option<&usize> {
        self.0.get(block.index()).and_then(Option::as_ref)
    }
}

impl Index<&BasicBlockId> for LegacyInstructionArrays {
    type Output = usize;

    fn index(&self, block: &BasicBlockId) -> &Self::Output {
        self.get(*block).expect("no entry found for key")
    }
}

#[derive(Debug, Default)]
pub(super) struct LegacyNativeLabels {
    bindings: FxHashMap<Label, u32>,
    blocks: FxHashMap<BasicBlockId, u32>,
    original_arrays: FxHashMap<BasicBlockId, usize>,
    pub(super) exit_copy_budget: Option<u8>,
    certified: bool,
    invalidated: bool,
}

impl LegacyNativeLabels {
    pub(super) fn bind(&mut self, physical: Label, native: u32) {
        debug_assert!(native > 0);
        if self
            .bindings
            .insert(physical, native)
            .is_some_and(|old| old != native)
        {
            self.invalidate();
        }
    }

    pub(super) fn certify(&mut self) {
        self.certified = !self.invalidated;
    }

    pub(super) fn invalidate(&mut self) {
        self.invalidated = true;
        self.certified = false;
    }

    pub(super) fn is_certified(&self) -> bool {
        self.certified
    }

    /// Binds original native block labels after exact codegen alias resolution.
    pub(super) fn initialize_blocks(
        &mut self,
        graph: &BlockGraph,
        original_arrays: &LegacyInstructionArrays,
    ) {
        if !self.certified {
            return;
        }
        for block in graph.order().iter().copied() {
            let mut native = None;
            for item in graph[block].items() {
                if let Item::Label(label) = item
                    && let Some(&bound) = self.bindings.get(label)
                {
                    // The compiler already canonicalized true native aliases. Distinct
                    // original offsets collapsed by early lowering need a later producer
                    // certificate; do not guess which surviving physical alias wins.
                    if native.is_some_and(|previous| previous != bound) {
                        self.invalidate();
                        return;
                    }
                    native = Some(bound);
                }
            }
            if let Some(native) = native {
                self.blocks.insert(block, native);
            }
        }
        // Exception-range labels can split one original instruction array into
        // physical fragments. Certify only groups beginning with a known native
        // entry and no later native entry after live instructions. Unbound or
        // unreferenced codegen entries retain the physical-block fallback.
        let mut groups = FxHashMap::<usize, (Option<u32>, bool, bool)>::default();
        for block in graph.order().iter().copied() {
            let Some(&array) = original_arrays.get(block) else {
                continue;
            };
            let (label, seen_instruction, valid) =
                groups
                    .entry(array)
                    .or_insert((None, false, self.blocks.contains_key(&block)));
            if let Some(&bound) = self.blocks.get(&block) {
                if *seen_instruction || label.is_some_and(|previous| previous != bound) {
                    *valid = false;
                }
                *label = Some(bound);
            }
            *seen_instruction |= graph[block].last_instruction().is_some();
        }
        for block in graph.order().iter().copied() {
            let Some(&array) = original_arrays.get(block) else {
                continue;
            };
            let &(label, _, valid) = &groups[&array];
            if valid {
                self.original_arrays.insert(block, array);
                if let Some(label) = label {
                    self.blocks.insert(block, label);
                }
            }
        }
    }

    /// Checks native array occupancy without crossing an original source entry.
    pub(super) fn target_is_nonempty(&self, graph: &BlockGraph, target: BasicBlockId) -> bool {
        if graph[target].last_instruction().is_some() {
            return true;
        }
        let Some(array) = self.original_arrays.get(&target) else {
            return false;
        };
        let Some(index) = graph.order().iter().position(|block| *block == target) else {
            return false;
        };
        graph.order()[index + 1..]
            .iter()
            .take_while(|block| self.original_arrays.get(block) == Some(array))
            .any(|block| graph[*block].last_instruction().is_some())
    }

    pub(super) fn block_label(&self, block: BasicBlockId) -> Option<u32> {
        self.certified
            .then(|| self.blocks.get(&block).copied())
            .flatten()
    }

    pub(super) fn target_label(&self, graph: &BlockGraph, target: Label) -> Option<u32> {
        self.block_label(graph.label_block(target)?)
    }

    /// Starts the allocator used by one native CFG phase from its live block list.
    pub(super) fn next_cfg_label(&self, graph: &BlockGraph) -> u32 {
        graph
            .order()
            .iter()
            .filter(|block| graph[**block].last_instruction().is_some())
            .filter_map(|block| self.blocks.get(block).copied())
            .max()
            .unwrap_or(0)
            + 1
    }

    pub(super) fn allocate_for_block(&mut self, block: BasicBlockId, next: &mut u32) -> u32 {
        *self.blocks.entry(block).or_insert_with(|| {
            let label = *next;
            *next += 1;
            label
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::{Instruction, Operand, SourceLocation};
    use crate::target::opcodes::{NOP, RETURN_VALUE};
    use rustc_hash::FxHashSet;

    impl FromIterator<(BasicBlockId, usize)> for LegacyInstructionArrays {
        fn from_iter<T: IntoIterator<Item = (BasicBlockId, usize)>>(iter: T) -> Self {
            let iter = iter.into_iter();
            let mut arrays = Self::with_capacity(iter.size_hint().0);
            for (block, array) in iter {
                arrays.insert(block, array);
            }
            arrays
        }
    }

    #[test]
    fn indexed_arrays_preserve_missing_entries_and_stable_ids() {
        let first = BasicBlockId::from_usize(0);
        let later = BasicBlockId::from_usize(3);
        let entries = [(later, 0), (first, usize::MAX), (later, 7)];
        let expected = entries.into_iter().collect::<FxHashMap<_, _>>();
        let actual = entries.into_iter().collect::<LegacyInstructionArrays>();
        for index in 0..8 {
            let block = BasicBlockId::from_usize(index);
            assert_eq!(actual.get(block), expected.get(&block));
        }
        assert_eq!(actual[&later], 7);
    }

    fn fragmented_graph() -> BlockGraph {
        BlockGraph::from_items(
            vec![
                Item::Label(Label::from_u32(0)),
                Item::Instruction(Instruction::new(
                    NOP,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    None,
                )),
                Item::Label(Label::from_u32(1)),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    None,
                )),
            ],
            &FxHashSet::default(),
        )
    }

    #[test]
    fn native_array_label_survives_an_empty_metadata_head() {
        let mut graph = fragmented_graph();
        let head = graph.label_block(Label::from_u32(0)).unwrap();
        let tail = graph.label_block(Label::from_u32(1)).unwrap();
        let arrays = [(head, 7), (tail, 7)].into_iter().collect();
        let mut native = LegacyNativeLabels::default();
        native.bind(Label::from_u32(0), 255);
        native.certify();
        native.initialize_blocks(&graph, &arrays);
        graph.retain_items(head, |item| matches!(item, Item::Label(_)));

        assert!(native.is_certified());
        assert!(native.target_is_nonempty(&graph, head));
        assert_eq!(native.block_label(tail), Some(255));
        assert_eq!(native.next_cfg_label(&graph), 256);
        let mut next = 256;
        assert_eq!(native.allocate_for_block(tail, &mut next), 255);
        assert_eq!(next, 256);
        graph.retain_items(tail, |item| matches!(item, Item::Label(_)));
        assert!(!native.target_is_nonempty(&graph, head));
    }

    #[test]
    fn native_fragment_certificate_declines_unknown_and_distinct_entries() {
        // Unknown original entry, later native entry after live instructions,
        // distinct native entries in one coarse array, and a real next array.
        for case in 0..4 {
            let mut graph = fragmented_graph();
            let head = graph.label_block(Label::from_u32(0)).unwrap();
            let tail = graph.label_block(Label::from_u32(1)).unwrap();
            let arrays = [(head, 7), (tail, if case == 3 { 8 } else { 7 })]
                .into_iter()
                .collect();
            let mut native = LegacyNativeLabels::default();
            if case >= 2 {
                native.bind(Label::from_u32(0), 255);
            }
            if case == 1 || case == 2 {
                native.bind(Label::from_u32(1), 256);
            }
            native.certify();
            native.initialize_blocks(&graph, &arrays);
            graph.retain_items(head, |item| matches!(item, Item::Label(_)));
            assert!(native.is_certified());
            assert!(!native.target_is_nonempty(&graph, head), "case {case}");
            assert_eq!(
                native.block_label(tail),
                (case == 1 || case == 2).then_some(256)
            );
        }
    }

    #[test]
    fn native_fragment_certificate_never_skips_an_intervening_array() {
        let mut graph = fragmented_graph();
        let head = graph.label_block(Label::from_u32(0)).unwrap();
        let tail = graph.label_block(Label::from_u32(1)).unwrap();
        let arrays = [(head, 7), (tail, 7)].into_iter().collect();
        let mut native = LegacyNativeLabels::default();
        native.bind(Label::from_u32(0), 255);
        native.certify();
        native.initialize_blocks(&graph, &arrays);
        graph.retain_items(head, |item| matches!(item, Item::Label(_)));
        graph.insert_block(1, vec![Item::Label(Label::from_u32(2))]);
        assert!(!native.target_is_nonempty(&graph, head));
    }
}
