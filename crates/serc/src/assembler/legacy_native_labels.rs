//! Exact native labels retained independently of Serc's physical block labels.

use rustc_hash::FxHashMap;

use super::block_graph::{BasicBlockId, BlockGraph};
use super::ir::{Item, Label};

#[derive(Debug, Default)]
pub(super) struct LegacyNativeLabels {
    bindings: FxHashMap<Label, u32>,
    blocks: FxHashMap<BasicBlockId, u32>,
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
    pub(super) fn initialize_blocks(&mut self, graph: &BlockGraph) {
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
