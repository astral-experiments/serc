//! Exact native labels retained independently of Serc's physical block labels.

use rustc_hash::FxHashMap;

use super::block_graph::{BasicBlockId, BlockGraph};
use super::ir::{Item, Label};

#[derive(Debug, Default)]
pub(super) struct LegacyNativeLabels {
    bindings: FxHashMap<Label, u32>,
    blocks: FxHashMap<BasicBlockId, u32>,
    original_arrays: FxHashMap<BasicBlockId, usize>,
    pending_scope_exits: FxHashMap<Label, u32>,
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

    /// Retains the producer's physical/native pair without certifying its footer yet.
    pub(super) fn record_pending_scope_exit(&mut self, label: Label) {
        if !self.invalidated
            && let Some(&native) = self.bindings.get(&label)
            && native != 0
        {
            self.pending_scope_exits.insert(label, native);
        }
    }

    /// Admits only complete, contiguous arrays whose every label has the same binding.
    pub(super) fn pending_scope_exit_arrays(
        &self,
        graph: &BlockGraph,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
    ) -> Vec<(Label, usize)> {
        if !self.certified {
            return Vec::new();
        }
        self.pending_scope_exits
            .iter()
            .filter_map(|(&label, &native)| {
                if native == 0 || self.bindings.get(&label) != Some(&native) {
                    return None;
                }
                let target = graph.label_block(label)?;
                let array = *original_arrays.get(&target)?;
                let mut in_group = false;
                let mut group_ended = false;
                let mut found_target = false;
                let mut seen_instruction = false;
                for &block in graph.order() {
                    if original_arrays.get(&block) != Some(&array) {
                        group_ended |= in_group;
                        continue;
                    }
                    if group_ended
                        || self.original_arrays.get(&block) != Some(&array)
                        || self.blocks.get(&block) != Some(&native)
                    {
                        return None;
                    }
                    in_group = true;
                    found_target |= block == target;
                    for item in graph[block].items() {
                        match item {
                            Item::Label(alias) => {
                                if seen_instruction || self.bindings.get(alias) != Some(&native) {
                                    return None;
                                }
                            }
                            Item::Instruction(_) => seen_instruction = true,
                        }
                    }
                }
                found_target.then_some((label, array))
            })
            .collect()
    }

    /// Binds original native block labels after exact codegen alias resolution.
    pub(super) fn initialize_blocks(
        &mut self,
        graph: &BlockGraph,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
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
            let Some(&array) = original_arrays.get(&block) else {
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
            let Some(&array) = original_arrays.get(&block) else {
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
    use crate::target::opcodes::{LOAD_CONST, NOP, RETURN_VALUE};
    use rustc_hash::FxHashSet;

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

    fn pending_footer() -> (
        BlockGraph,
        LegacyNativeLabels,
        FxHashMap<BasicBlockId, usize>,
    ) {
        let graph = BlockGraph::from_items(
            vec![
                Item::Label(Label::from_u32(0)),
                Item::Label(Label::from_u32(1)),
                Item::Instruction(Instruction::new(
                    LOAD_CONST,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    Some(1),
                )),
                Item::Instruction(Instruction::new(
                    RETURN_VALUE,
                    Operand::Value(0),
                    SourceLocation::NONE,
                    Some(0),
                )),
            ],
            &FxHashSet::default(),
        );
        let arrays = graph.order().iter().map(|&block| (block, 7)).collect();
        let mut native = LegacyNativeLabels::default();
        native.bind(Label::from_u32(0), 11);
        native.bind(Label::from_u32(1), 11);
        native.record_pending_scope_exit(Label::from_u32(0));
        (graph, native, arrays)
    }

    #[test]
    fn pending_scope_exit_requires_whole_map_and_initial_array_validation() {
        let (graph, mut native, arrays) = pending_footer();
        assert!(native.pending_scope_exit_arrays(&graph, &arrays).is_empty());
        native.certify();
        assert!(native.pending_scope_exit_arrays(&graph, &arrays).is_empty());
        native.initialize_blocks(&graph, &arrays);
        assert_eq!(
            native.pending_scope_exit_arrays(&graph, &arrays),
            [(Label::from_u32(0), 7)]
        );
        native.invalidate();
        native.certify();
        assert!(native.pending_scope_exit_arrays(&graph, &arrays).is_empty());
    }

    #[test]
    fn pending_scope_exit_rejects_missing_stale_or_conflicting_provenance() {
        for case in 0..8 {
            let (graph, mut native, mut arrays) = pending_footer();
            native.certify();
            native.initialize_blocks(&graph, &arrays);
            let block = graph.label_block(Label::from_u32(0)).unwrap();
            match case {
                0 => native.pending_scope_exits.clear(),
                1 => {
                    native.bindings.remove(&Label::from_u32(1));
                }
                2 => native.bind(Label::from_u32(1), 12),
                3 => {
                    native.pending_scope_exits.insert(Label::from_u32(0), 12);
                }
                4 => {
                    native.original_arrays.remove(&block);
                }
                5 => {
                    native.blocks.insert(block, 12);
                }
                6 => {
                    arrays.insert(block, 8);
                }
                7 => {
                    native.pending_scope_exits.clear();
                    native.bind(Label::from_u32(9), 11);
                    native.record_pending_scope_exit(Label::from_u32(9));
                }
                _ => unreachable!(),
            }
            assert!(
                native.pending_scope_exit_arrays(&graph, &arrays).is_empty(),
                "case {case}"
            );
        }
        let mut absent = LegacyNativeLabels::default();
        absent.record_pending_scope_exit(Label::from_u32(0));
        assert!(absent.pending_scope_exits.is_empty());
    }

    #[test]
    fn pending_scope_exit_rejects_later_entries_and_noncontiguous_arrays() {
        let graph = fragmented_graph();
        let arrays = graph.order().iter().map(|&block| (block, 7)).collect();
        let mut native = LegacyNativeLabels::default();
        for label in [Label::from_u32(0), Label::from_u32(1)] {
            native.bind(label, 11);
        }
        native.record_pending_scope_exit(Label::from_u32(0));
        native.certify();
        native.initialize_blocks(&graph, &arrays);
        assert!(native.is_certified());
        assert!(native.pending_scope_exit_arrays(&graph, &arrays).is_empty());

        let (mut graph, mut native, mut arrays) = pending_footer();
        let first = graph.insert_block(0, vec![Item::Label(Label::from_u32(2))]);
        let middle = graph.insert_block(1, vec![Item::Label(Label::from_u32(3))]);
        arrays.insert(first, 7);
        arrays.insert(middle, 8);
        native.bind(Label::from_u32(2), 11);
        native.bind(Label::from_u32(3), 12);
        native.certify();
        native.initialize_blocks(&graph, &arrays);
        assert!(native.is_certified());
        assert!(native.pending_scope_exit_arrays(&graph, &arrays).is_empty());
    }

    #[test]
    fn pending_scope_exit_requires_every_fragment_certificate_and_known_alias() {
        for case in 0..4 {
            let (mut graph, mut native, mut arrays) = pending_footer();
            let head = graph.insert_block(0, vec![Item::Label(Label::from_u32(2))]);
            arrays.insert(head, 7);
            native.bind(Label::from_u32(2), 11);
            native.certify();
            native.initialize_blocks(&graph, &arrays);
            match case {
                0 => {}
                1 => {
                    native.original_arrays.remove(&head);
                }
                2 => {
                    native.bindings.remove(&Label::from_u32(2));
                }
                3 => {
                    native.blocks.insert(head, 12);
                }
                _ => unreachable!(),
            }
            assert_eq!(
                native.pending_scope_exit_arrays(&graph, &arrays),
                if case == 0 {
                    vec![(Label::from_u32(0), 7)]
                } else {
                    vec![]
                },
                "case {case}"
            );
        }
    }
}
