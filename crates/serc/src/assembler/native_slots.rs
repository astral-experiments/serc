//! Instruction-array ownership across native CFG compaction and jump normalization.

/// Evidence about an instruction's exception owner before NOP compaction.
///
/// The native array carries the whole instruction record when compacting. Some
/// callers can prove that a SETUP slot is ownerless, but cannot establish the
/// owner of every other instruction in the original block. The SETUP variants
/// retain the proof's origin because callers admit different setup histories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SlotOwner {
    RetainedSetup,
    RemovedSetup,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
struct Slot<Owner> {
    owner: Owner,
    removed: bool,
}

/// The physical cells of one native block, including the suffix left by compaction.
///
/// Callers construct this only after proving the original array's boundaries and
/// the instructions removed before normalization. An unknown owner is never used
/// as evidence for an exception-table decision.
#[derive(Debug)]
pub(super) struct NativeSlots<Owner = SlotOwner> {
    slots: Vec<Slot<Owner>>,
}

impl<Owner> Default for NativeSlots<Owner> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<Owner: Copy> NativeSlots<Owner> {
    pub(super) fn push(&mut self, owner: Owner) {
        self.slots.push(Slot {
            owner,
            removed: false,
        });
    }

    pub(super) fn push_removed(&mut self, owner: Owner) {
        self.slots.push(Slot {
            owner,
            removed: true,
        });
    }

    pub(super) fn set_last_owner(&mut self, owner: Owner) {
        self.slots
            .last_mut()
            .expect("a retained SETUP follows its emitted instruction")
            .owner = owner;
    }

    /// Return the old owner of the cell where jump normalization appends
    /// `NOT_TAKEN` after CPython's NOP compaction. Compaction only writes below
    /// the new used length, so that cell is unchanged and can be read directly.
    /// `None` means it lies beyond the captured cells; the caller must prove
    /// that the slot has no earlier history before treating it as ownerless.
    pub(super) fn appended_owner(self) -> Option<Owner> {
        let used = self.slots.iter().filter(|slot| !slot.removed).count();
        self.slots.get(used).map(|slot| slot.owner)
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeSlots, SlotOwner};

    #[test]
    fn marker_is_beyond_captured_cells_when_no_slots_were_removed() {
        let mut slots = NativeSlots::default();
        slots.push(SlotOwner::Unknown);
        slots.push(SlotOwner::RetainedSetup);
        slots.push(SlotOwner::Unknown);
        assert_eq!(slots.appended_owner(), None);
    }

    #[test]
    fn marker_reuses_the_original_cell_at_the_compacted_length() {
        let mut slots = NativeSlots::default();
        slots.push_removed(SlotOwner::Unknown);
        slots.push_removed(SlotOwner::Unknown);
        slots.push(SlotOwner::RetainedSetup);
        slots.push(SlotOwner::Unknown);
        assert_eq!(slots.appended_owner(), Some(SlotOwner::RetainedSetup));
    }

    #[test]
    fn marker_does_not_infer_the_owner_of_an_uncertified_cell() {
        let mut slots = NativeSlots::default();
        slots.push(SlotOwner::RetainedSetup);
        slots.push_removed(SlotOwner::Unknown);
        slots.push(SlotOwner::Unknown);
        assert_eq!(slots.appended_owner(), Some(SlotOwner::Unknown));
    }

    #[test]
    fn marker_reuses_a_previously_owned_cell() {
        let mut slots = NativeSlots::<Option<u8>>::default();
        slots.push(None);
        slots.push_removed(Some(7));
        slots.push(Some(9));
        assert_eq!(slots.appended_owner(), Some(Some(9)));
    }
}
