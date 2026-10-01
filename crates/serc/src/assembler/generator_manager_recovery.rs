use super::{
    Assembler, BorrowFallthrough, EmittedJump, Instruction, Item, Label, Operand, SourceLocation,
    UnoptimizedCode, trailing_labels,
};
use crate::PythonVersion;
use crate::target::opcodes::JUMP_FORWARD;

#[derive(Debug)]
pub(super) struct Source {
    emitted: EmittedJump,
    entry: Label,
    target: Label,
    head: Vec<Label>,
    boundaries: Vec<(bool, bool)>,
    original: Instruction,
}

impl Source {
    fn capture(
        assembler: &Assembler,
        emitted: EmittedJump,
        entry: Label,
        target: Label,
    ) -> Option<Self> {
        let UnoptimizedCode::Linear(items) = &assembler.code else {
            return None;
        };
        if assembler.python_version < PythonVersion::Py314
            || emitted.instruction().stream != assembler.emission_stream
        {
            return None;
        }
        let index = usize::try_from(emitted.instruction().item_index).ok()?;
        let Item::Instruction(original) = items.get(index)? else {
            return None;
        };
        let depth = original.depth_after()?;
        if *original
            != Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                original.location,
                Some(depth),
            )
            || original.location.line < 0
            || index + 1 != items.len()
        {
            return None;
        }
        let head = trailing_labels(&items[..index]).collect::<Vec<_>>();
        let source = Self {
            emitted,
            entry,
            target,
            boundaries: head
                .iter()
                .map(|label| {
                    (
                        assembler
                            .exception_owner_provenance
                            .source_boundaries
                            .contains(label),
                        assembler
                            .exception_owner_provenance
                            .block_boundaries
                            .contains(label),
                    )
                })
                .collect(),
            head,
            original: *original,
        };
        source
            .matches(assembler, index, original, 0)
            .then_some(source)
    }

    pub(super) fn matches(
        &self,
        assembler: &Assembler,
        index: usize,
        instruction: &Instruction,
        lower: usize,
    ) -> bool {
        let UnoptimizedCode::Linear(items) = &assembler.code else {
            return false;
        };
        let source_index = usize::try_from(self.emitted.instruction().item_index).ok();
        if self.emitted.instruction().stream != assembler.emission_stream
            || source_index != Some(index)
            || index >= items.len()
            || self.head.is_empty()
            || !self.head.contains(&self.entry)
            || self.head.contains(&self.target)
            || index
                .checked_sub(self.head.len())
                .is_none_or(|start| start < lower)
            || *instruction != self.original
            || items[index] != Item::Instruction(self.original)
            || !trailing_labels(&items[..index]).eq(self.head.iter().copied())
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(self.original))
                .count()
                != 1
        {
            return false;
        }
        let provenance = &assembler.exception_owner_provenance;
        let mut pairs = provenance
            .normal_try_exits
            .iter()
            .filter(|(entry, target)| self.head.contains(entry) || self.head.contains(target));
        if pairs.next() != Some(&(self.entry, self.target)) || pairs.next().is_some() {
            return false;
        }
        self.head.len() == self.boundaries.len() && self.head.iter().zip(&self.boundaries).all(|(label, boundaries)| {
            items.iter().filter(|item| **item == Item::Label(*label)).count() == 1
                && *boundaries == (provenance.source_boundaries.contains(label), provenance.block_boundaries.contains(label))
                && !items.iter().any(|item| matches!(item, Item::Instruction(value)
                    if matches!(value.operand(), Operand::Forward(target) | Operand::Backward(target)
                        if target == *label)))
                && !assembler.exception_regions.iter().any(|region| region.target == *label)
                && !assembler.folded_away_condition_targets.contains(label)
                && !assembler.folded_away_loop_else_targets.contains(label)
                && !assembler.folded_condition_exit_targets.contains(label)
                && !assembler.preserved_block_boundaries.contains(label)
                && !assembler.borrow_unreachable_blocks.contains(label)
                && !assembler.borrow_jump_entry_labels.contains(label)
                && !assembler.borrow_block_aliases.contains(label)
                && !assembler.borrow_loop_exit_blocks.contains_key(label)
                && !assembler.borrow_finally_entries.contains_key(label)
                && !assembler.conditional_threading_targets.contains_key(label)
                && !assembler.original_nop_targets.contains(label)
                && !assembler.original_unlocated_cleanup_targets.contains(label)
                && !assembler.retained_pop_block_origins.contains_key(label)
                && !assembler.deferred_shared_jump_blocks.contains(label)
                && !assembler.shared_jump_copy_location_blocks.contains(label)
                && !provenance.ordinary_try_setups.contains_key(label)
                && !provenance.omitted_nop_counts.contains_key(label)
                && !provenance.omitted_nop_locations.contains_key(label)
        })
    }
}

impl Assembler {
    /// Records only the real default-to-Barrier write on an original recovery exit.
    pub(crate) fn mark_generator_manager_recovery_barrier(
        &mut self,
        emitted: EmittedJump,
        entry: Label,
        target: Label,
    ) {
        let origin = Source::capture(self, emitted, entry, target);
        self.mark_borrow_fallthrough_barrier(emitted.instruction());
        if let Some(mut origin) = origin {
            origin
                .original
                .no_location_policy
                .set_borrow_fallthrough(BorrowFallthrough::Barrier);
            let index = usize::try_from(emitted.instruction().item_index)
                .expect("emitted item index fits usize");
            if origin.matches(self, index, &origin.original, 0) {
                self.generator_manager_recovery_sources.push(origin);
            }
        }
    }

    /// Certifies the generator's original join with one unchanged recovery Barrier.
    pub(crate) fn original_generator_manager_recovery_join(
        &mut self,
        source: EmittedJump,
        continuation: Label,
        condition: SourceLocation,
        finalbody_start: usize,
    ) -> bool {
        let UnoptimizedCode::Linear(items) = &self.code else {
            return false;
        };
        let index = usize::try_from(source.instruction().item_index).ok();
        if source.instruction().stream != self.emission_stream
            || self.generator_manager_recovery_sources.is_empty()
            || items.iter().rposition(|item| matches!(item, Item::Instruction(_))) != index
            || index.and_then(|index| items.get(index)).is_none_or(|item| !matches!(item,
                Item::Instruction(instruction) if matches!(instruction.operand(), Operand::Forward(_) | Operand::Backward(_))))
        {
            return false;
        }
        let index = index.expect("checked original jump index");
        let aliases = trailing_labels(&items[..index]).collect::<Vec<_>>();
        if self
            .exception_regions
            .iter()
            .any(|region| aliases.contains(&region.target))
        {
            return false;
        }
        self.original_normal_finally_manager_join_inner(
            source,
            continuation,
            condition,
            finalbody_start,
            true,
        )
    }
}

#[cfg(test)]
mod tests;
