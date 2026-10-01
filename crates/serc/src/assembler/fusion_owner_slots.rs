//! Preserve original-array owners across certified unchanged intervals and known slot changes.
use super::exception_owners::ExceptionOwnerProvenance;
use super::{
    ExceptionExclusion, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority, Instruction,
    Item, Label, NormalizedExceptionOwner, Operand, OptimizationPipeline,
};
use crate::target::is_scope_exit;
use crate::target::opcodes::{
    COPY_FREE_VARS, LOAD_FAST, LOAD_FAST_BORROW, LOAD_FAST_BORROW_LOAD_FAST_BORROW,
    LOAD_FAST_LOAD_FAST, MAKE_CELL, NOP, NOT_TAKEN, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE,
    POP_JUMP_IF_NOT_NONE, POP_JUMP_IF_TRUE, RETURN_GENERATOR, STORE_FAST, STORE_FAST_LOAD_FAST,
    STORE_FAST_STORE_FAST,
};
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use std::cell::OnceCell;
use std::cmp::Reverse;

type Key = (i32, i32, i32, i32, u8, Label);
type Owner = Option<(Label, u32, bool)>;

fn same_instruction(a: Instruction, b: Instruction, borrow: bool) -> bool {
    (a.opcode == b.opcode
        || borrow
            && matches!(
                (a.opcode, b.opcode),
                (LOAD_FAST, LOAD_FAST_BORROW)
                    | (LOAD_FAST_LOAD_FAST, LOAD_FAST_BORROW_LOAD_FAST_BORROW)
            ))
        && a.same_fusion_owner_fields(&b)
}
fn same_item(a: Item, b: Item, borrow: bool) -> bool {
    match (a, b) {
        (Item::Label(a), Item::Label(b)) => a == b,
        (Item::Instruction(a), Item::Instruction(b)) => same_instruction(a, b, borrow),
        _ => false,
    }
}
fn key(condition: Instruction, marker: Instruction) -> Option<Key> {
    if marker.opcode != NOT_TAKEN
        || !matches!(
            condition.opcode,
            POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
        )
    {
        return None;
    }
    let target = condition.forward_target()?;
    let l = marker.location;
    Some((
        l.line,
        l.end_line,
        l.column,
        l.end_column,
        condition.opcode.code(),
        target,
    ))
}

/// Indexes normally consecutive compiler labels without hashing every owner lookup.
struct LabelPositions {
    dense: Vec<LabelPositionSlot>,
    sparse: FxHashMap<Label, (usize, usize)>,
    dense_limit: usize,
    generation: u32,
}

#[derive(Clone, Copy, Default)]
struct LabelPositionSlot {
    position: (usize, usize),
    generation: u32,
}

impl Default for LabelPositions {
    fn default() -> Self {
        Self {
            dense: Vec::new(),
            sparse: FxHashMap::default(),
            dense_limit: 0,
            generation: 1,
        }
    }
}

impl LabelPositions {
    fn prepare(&mut self, item_count: usize) {
        self.clear();
        // Large or unusually sparse label IDs must not dictate the allocation size.
        self.dense_limit = item_count.saturating_mul(2).max(64);
    }

    fn clear(&mut self) {
        self.sparse.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.dense.fill(LabelPositionSlot::default());
            self.generation = 1;
        }
    }

    #[inline]
    fn insert(&mut self, label: Label, position: (usize, usize)) -> Option<(usize, usize)> {
        let index = label.index();
        if index >= self.dense_limit {
            return self.sparse.insert(label, position);
        }
        if index >= self.dense.len() {
            self.dense.resize(index + 1, LabelPositionSlot::default());
        }
        let slot = &mut self.dense[index];
        let previous = (slot.generation == self.generation).then_some(slot.position);
        *slot = LabelPositionSlot {
            position,
            generation: self.generation,
        };
        previous
    }

    #[inline]
    fn get(&self, label: Label) -> Option<&(usize, usize)> {
        let index = label.index();
        if index >= self.dense_limit {
            return self.sparse.get(&label);
        }
        let slot = self.dense.get(index)?;
        (slot.generation == self.generation).then_some(&slot.position)
    }
}

/// Reusable allocations for indexes whose records are rebuilt at every checkpoint.
#[derive(Default)]
struct ViewTables {
    labels: LabelPositions,
    boundaries: FxHashSet<Label>,
    markers: FxHashMap<Key, SmallVec<[(usize, usize); 1]>>,
    label_item_indices: Vec<usize>,
}

impl ViewTables {
    fn clear(&mut self) {
        self.labels.clear();
        self.boundaries.clear();
        self.markers.clear();
        self.label_item_indices.clear();
    }
}

#[derive(Default)]
struct RetentionScratch {
    items: Vec<Item>,
    tables: ViewTables,
}

struct View<'a> {
    items: &'a [Item],
    regions: &'a [ExceptionRegion],
    provenance: &'a ExceptionOwnerProvenance,
    labels: LabelPositions,
    boundaries: FxHashSet<Label>,
    supplementary_references: OnceCell<FxHashSet<Label>>,
    markers: FxHashMap<Key, SmallVec<[(usize, usize); 1]>>,
    label_item_indices: Vec<usize>,
}
impl<'a> View<'a> {
    fn new(
        items: &'a [Item],
        regions: &'a [ExceptionRegion],
        provenance: &'a ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> Option<Self> {
        Self::with_tables(items, regions, provenance, preserved, ViewTables::default())
    }

    fn with_tables(
        items: &'a [Item],
        regions: &'a [ExceptionRegion],
        provenance: &'a ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
        tables: ViewTables,
    ) -> Option<Self> {
        let ViewTables {
            mut labels,
            mut boundaries,
            mut markers,
            mut label_item_indices,
        } = tables;
        labels.prepare(items.len());
        boundaries.clear();
        markers.clear();
        label_item_indices.clear();
        label_item_indices.reserve(items.len().div_ceil(4));
        boundaries.extend(&provenance.block_boundaries);
        boundaries.extend(preserved);
        let mut previous = None;
        let mut n = 0;
        for (i, item) in items.iter().enumerate() {
            match item {
                Item::Label(l) => {
                    label_item_indices.push(i);
                    if labels.insert(*l, (i, n)).is_some() {
                        return None;
                    }
                }
                Item::Instruction(ins) => {
                    if let Some(l) = ins.jump_target() {
                        boundaries.insert(l);
                    }
                    if ins.opcode == NOT_TAKEN
                        && let Some(p) = previous
                        && let Item::Instruction(prior) = &items[p]
                        && let Some(k) = key(*prior, *ins)
                    {
                        markers.entry(k).or_default().push((p, i));
                    }
                    previous = Some(i);
                    n += 1;
                }
            }
        }
        for region in regions {
            boundaries.insert(region.target);
        }
        Some(Self {
            items,
            regions,
            provenance,
            labels,
            boundaries,
            supplementary_references: OnceCell::new(),
            markers,
            label_item_indices,
        })
    }

    fn collect_supplementary_references(&self) -> FxHashSet<Label> {
        let mut references = FxHashSet::default();
        for r in self.regions {
            references.extend([r.start, r.end]);
            if let ExceptionRegionPriority::OriginalRange(s, e) = r.priority {
                references.extend([s, e]);
            }
        }
        let provenance = self.provenance;
        references.extend(provenance.source_boundaries.iter().copied());
        references.extend(provenance.continuation_boundaries.iter().copied());
        references.extend(provenance.fusion_boundaries.iter().copied());
        references.extend(provenance.setup_nop_boundaries.iter().copied());
        references.extend(provenance.synchronous_with_setups.iter().copied());
        references.extend(provenance.removed_pop_block_boundaries.iter().copied());
        references.extend(provenance.protected_prefix_boundaries.iter().copied());
        references.extend(provenance.removed_pop_block_widths.keys().copied());
        for (l, _) in &provenance.setups {
            references.insert(*l);
        }
        for (s, e) in &provenance.setup_nop_ranges {
            references.extend([*s, *e]);
        }
        #[expect(
            clippy::iter_over_hash_type,
            reason = "The references set is only queried for membership."
        )]
        for origin in provenance.retained_normal_pop_blocks.values() {
            references.extend(origin.before.iter().copied());
            references.extend(origin.after.iter().copied());
        }
        references
    }

    fn is_reference(&self, label: Label) -> bool {
        self.boundaries.contains(&label)
            || self
                .supplementary_references
                .get_or_init(|| self.collect_supplementary_references())
                .contains(&label)
    }

    fn into_tables(self) -> ViewTables {
        ViewTables {
            labels: self.labels,
            boundaries: self.boundaries,
            markers: self.markers,
            label_item_indices: self.label_item_indices,
        }
    }

    fn label_index(&self, label: Label) -> Option<&usize> {
        self.labels.get(label).map(|(index, _)| index)
    }

    fn label_position(&self, label: Label) -> Option<&usize> {
        self.labels.get(label).map(|(_, position)| position)
    }

    /// Returns the number of instructions before an item or the end of the view.
    fn instruction_position(&self, item_index: usize) -> usize {
        debug_assert!(item_index <= self.items.len());
        item_index
            - self
                .label_item_indices
                .partition_point(|index| *index < item_index)
    }

    fn endpoint(&self, condition: Instruction, marker: Instruction) -> Option<(usize, usize)> {
        let candidates = self.markers.get(&key(condition, marker)?)?;
        let mut found = None;
        for &(c, m) in candidates {
            let (Item::Instruction(a), Item::Instruction(b)) = (self.items[c], self.items[m])
            else {
                return None;
            };
            if same_instruction(condition, a, false) && same_instruction(marker, b, false) {
                if found.is_some() {
                    return None;
                }
                found = Some((c, m));
            }
        }
        found
    }
    fn owner(&self, condition_index: usize) -> Option<Owner> {
        let Item::Instruction(c) = self.items[condition_index] else {
            return None;
        };
        if c.exception_policy().owner() != NormalizedExceptionOwner::Unchanged {
            return None;
        }
        let pos = self.instruction_position(condition_index);
        let mut owner = None;
        for (index, r) in self.regions.iter().enumerate() {
            let s = *self.label_position(r.start)?;
            let e = *self.label_position(r.end)?;
            if s <= pos && pos < e {
                // A cleanup stack can retain an owner on the original array record
                // even when the lowered condition is excluded. The local interval
                // certificate does not establish that record's cleanup provenance.
                if r.depth != 0 {
                    return None;
                }
                let (ps, pe) = match r.priority {
                    ExceptionRegionPriority::OriginalRange(s, e) => {
                        (*self.label_position(s)?, *self.label_position(e)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                let candidate = (
                    (r.is_scope_fallback, Reverse(ps), pe, index),
                    (r.target, r.depth, r.preserve_lasti),
                );
                if owner
                    .as_ref()
                    .is_none_or(|(priority, _)| candidate.0 < *priority)
                {
                    owner = Some(candidate);
                }
            }
        }
        match c.exception_policy().exclusion() {
            ExceptionExclusion::Always => return Some(None),
            ExceptionExclusion::None => {}
            ExceptionExclusion::IfExtended => return None,
        }
        Some(owner.map(|(_, owner)| owner))
    }
    fn no_late_insertions_or_setups(&self, start: usize, end: usize) -> bool {
        if self.items[start..=end].iter().any(|x|matches!(x,Item::Instruction(i) if matches!(i.opcode,COPY_FREE_VARS|MAKE_CELL|RETURN_GENERATOR)||i.pre_fold_extra_instructions()!=0||i.local_policy.hidden_exception_owner_slots()==super::LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS)){return false;}
        for (l, location) in &self.provenance.setups {
            let Some(&p) = self.label_index(*l) else {
                return false;
            };
            if start <= p && p <= end {
                let previous = self.items[start..p].iter().rev().find_map(|x| match x {
                    Item::Instruction(i) => Some(i),
                    Item::Label(_) => None,
                });
                if !location.is_some_and(|loc| {
                    previous.is_some_and(|i| i.opcode == NOP && i.location == loc)
                }) {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Clone)]
enum Anchor {
    Entry,
    Label(Label),
    Marker(Instruction, Instruction),
    OriginalMarker(Instruction, Instruction),
}
struct Certificate {
    anchor: Anchor,
    condition: Instruction,
    marker: Instruction,
    items: Vec<Item>,
    labels: FxHashSet<Label>,
    owner: Owner,
    boundaries: FxHashSet<Label>,
    setups: Vec<(Label, Option<super::SourceLocation>)>,
}
impl Certificate {
    fn range(&self, v: &View<'_>, borrow: bool) -> Option<(usize, usize, usize)> {
        let (c, m) = v.endpoint(self.condition, self.marker)?;
        let start = match self.anchor {
            Anchor::Entry => 0,
            Anchor::Label(l) => *v.label_index(l)?,
            Anchor::Marker(c, m) => v.endpoint(c, m)?.1 + 1,
            Anchor::OriginalMarker(prior, marker) => {
                // Repeated sequence predicates may have equal predecessor pairs.
                // Only the entire captured array ending at the unique current
                // marker identifies its original fallthrough; pair order cannot.
                let candidates = v.markers.get(&key(prior, marker)?)?;
                let mut found = None;
                for &(pc, pm) in candidates {
                    let (
                        Item::Instruction(candidate_condition),
                        Item::Instruction(candidate_marker),
                    ) = (v.items[pc], v.items[pm])
                    else {
                        return None;
                    };
                    let start = pm + 1;
                    if start <= c
                        && same_instruction(prior, candidate_condition, false)
                        && same_instruction(marker, candidate_marker, false)
                        && self.matches_interval(v, start, m, borrow).is_some()
                    {
                        if found.replace(start).is_some() {
                            return None;
                        }
                    }
                }
                found?
            }
        };
        (start <= c).then_some((start, c, m))
    }
    fn valid(&self, view: &View<'_>, borrow: bool) -> Option<usize> {
        let (_, condition, marker) = self.valid_array(view, borrow)?;
        (view.owner(condition)? == self.owner).then_some(marker)
    }

    fn valid_array(&self, view: &View<'_>, borrow: bool) -> Option<(usize, usize, usize)> {
        let (start, condition, marker) = self.range(view, borrow)?;
        if self.setups != view.provenance.setups || !self.same_boundaries(view) {
            return None;
        }
        self.matches_interval(view, start, marker, borrow)?;
        Some((start, condition, marker))
    }

    /// Checks the original interval's boundary set without rebuilding its intersection.
    fn same_boundaries(&self, view: &View<'_>) -> bool {
        self.boundaries
            .iter()
            .all(|label| self.labels.contains(label) && view.boundaries.contains(label))
            && self
                .labels
                .iter()
                .filter(|label| view.boundaries.contains(*label))
                .count()
                == self.boundaries.len()
    }

    fn matches_interval(
        &self,
        view: &View<'_>,
        start: usize,
        marker: usize,
        borrow: bool,
    ) -> Option<()> {
        let mut old = self.items.iter().copied();
        for item in view.items[start..=marker].iter().copied() {
            if let Item::Label(l) = item {
                if !self.labels.contains(&l) && !view.is_reference(l) {
                    continue;
                }
            }
            if !same_item(old.next()?, item, borrow) {
                return None;
            }
        }
        old.next().is_none().then_some(())
    }
}

fn labels_in(items: &[Item]) -> FxHashSet<Label> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) => Some(*label),
            Item::Instruction(_) => None,
        })
        .collect()
}

/// Intersects an interval's labels with the view's boundaries without scanning the whole view.
fn boundaries_in(items: &[Item], boundaries: &FxHashSet<Label>) -> FxHashSet<Label> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) if boundaries.contains(label) => Some(*label),
            Item::Label(_) | Item::Instruction(_) => None,
        })
        .collect()
}

fn one_fusion(before: &[Item], after: &[Item]) -> bool {
    if before.len() != after.len() + 1 {
        return false;
    }
    let prefix = before
        .iter()
        .zip(after)
        .take_while(|(a, b)| same_item(**a, **b, false))
        .count();
    let (
        Some(Item::Instruction(first)),
        Some(Item::Instruction(second)),
        Some(Item::Instruction(fused)),
    ) = (
        before.get(prefix),
        before.get(prefix + 1),
        after.get(prefix),
    )
    else {
        return false;
    };
    let op = match (first.opcode, second.opcode) {
        (LOAD_FAST, LOAD_FAST) => LOAD_FAST_LOAD_FAST,
        (STORE_FAST, LOAD_FAST) => STORE_FAST_LOAD_FAST,
        (STORE_FAST, STORE_FAST) => STORE_FAST_STORE_FAST,
        _ => return false,
    };
    let (Operand::Value(a), Operand::Value(b)) = (first.operand(), second.operand()) else {
        return false;
    };
    a < 16
        && b < 16
        && first.location.line == second.location.line
        && same_instruction(
            Instruction::fused(op, Operand::Value((a << 4) | b), *first, *second),
            *fused,
            false,
        )
        && before[prefix + 2..]
            .iter()
            .zip(&after[prefix + 1..])
            .all(|(a, b)| same_item(*a, *b, false))
}

/// Retains a uniform fallback owner across a stable original array with only
/// accounted-for `LOAD_GLOBAL/PUSH_NULL` compaction. The zero-removal case uses
/// the fresh NULL slot immediately beyond the original live end.
struct FallbackCertificate {
    interval: Certificate,
    removed_slots: usize,
    observed_local_fusion: bool,
}

/// An authored singleton's entire original fallthrough array, with no vacant-slot reuse.
struct FreshSingletonCertificate {
    interval: Certificate,
    // Protected ranges transferred through cold splits, or recorded named-handler copy splits.
    manager_regions: Vec<ExceptionRegion>,
    // A separate fresh-array proof may retain overlapping named-handler/outer ranges.
    named_handler_target: Option<Label>,
}

impl FreshSingletonCertificate {
    fn capture(view: &View<'_>, anchor: Anchor, start: usize, marker: usize) -> Option<Self> {
        let instructions = view.items[start..=marker]
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let tail = match instructions.as_slice() {
            [constant, identity, condition, marker] => [*constant, *identity, *condition, *marker],
            [unpack, constant, identity, condition, marker]
                if unpack.opcode == crate::target::opcodes::UNPACK_SEQUENCE =>
            {
                [*constant, *identity, *condition, *marker]
            }
            _ => return None,
        };
        if tail[0].opcode != crate::target::opcodes::LOAD_CONST
            || !matches!(tail[0].operand(), Operand::Value(_))
            || tail[1].opcode != crate::target::opcodes::IS_OP
            || tail[1].value_argument() != Some(0)
            || tail[2].opcode != POP_JUMP_IF_FALSE
            || tail[3].exception_policy() != ExceptionPolicy::NoneUnchangedBooleanSingleton
        {
            return None;
        }
        key(tail[2], tail[3])?;
        let condition = view.endpoint(tail[2], tail[3])?.0;
        let mut manager_regions = view
            .regions
            .iter()
            .copied()
            .filter(|region| {
                !region.is_scope_fallback
                    && region.depth > 0
                    && view.regions.iter().any(|original| {
                        if !view
                            .provenance
                            .synchronous_with_setups
                            .contains(&original.start)
                        {
                            return false;
                        }
                        // Source exclusions retain the original setup range in priority.
                        // Match that origin and every owner field; only the split's
                        // physical endpoints may differ from the registered manager.
                        let mut same_origin = *region;
                        same_origin.start = original.start;
                        same_origin.end = original.end;
                        same_manager_region(&same_origin, original)
                    })
            })
            .collect::<Vec<_>>();
        let (owner, removed, named_handler_target) = if let Some((owner, removed)) =
            view.uniform_owned_interval(start, condition, false, Some(&manager_regions), false)
        {
            (owner, removed, None)
        } else {
            // Overlapping exception ranges do not populate a never-used array cell.
            // Keep this authored handler proof separate from uniform-owner recovery.
            let (target, regions) = view.fresh_named_handler_singleton_regions(start, condition)?;
            manager_regions = regions;
            (None, 0, Some(target))
        };
        if removed != 0 {
            return None;
        }
        let items = view.items[start..=marker].to_vec();
        let boundaries = boundaries_in(&items, &view.boundaries);
        let result = Self {
            interval: Certificate {
                anchor,
                condition: tail[2],
                marker: tail[3],
                labels: labels_in(&items),
                items,
                owner,
                boundaries,
                setups: view.provenance.setups.clone(),
            },
            manager_regions,
            named_handler_target,
        };
        result.valid(view, false)?;
        Some(result)
    }

    fn valid(&self, view: &View<'_>, borrow: bool) -> Option<usize> {
        let (start, condition, marker) = self.interval.valid_array(view, borrow)?;
        if let Anchor::Label(_) = self.interval.anchor
            && view.original_array_start(start)? != start
        {
            return None;
        }
        if let Some(target) = self.named_handler_target {
            let (current_target, current) =
                view.fresh_named_handler_singleton_regions(start, condition)?;
            if current_target != target {
                return None;
            }
            let position = view.instruction_position(condition);
            let mut expected = Vec::new();
            for region in &self.manager_regions {
                let begin = *view.label_position(region.start)?;
                let end = *view.label_position(region.end)?;
                if begin <= position && position < end {
                    expected.push(region);
                }
            }
            return (expected.len() == current.len()
                && expected
                    .iter()
                    .all(|old| current.iter().any(|new| same_manager_region(old, new))))
            .then_some(marker);
        }
        let (owner, removed) = view.uniform_owned_interval(
            start,
            condition,
            false,
            Some(&self.manager_regions),
            false,
        )?;
        (owner == self.interval.owner && removed == 0).then_some(marker)
    }
}
impl FallbackCertificate {
    fn capture(
        before: &View<'_>,
        after: &View<'_>,
        start: usize,
        marker_index: usize,
        previous: Option<(usize, Instruction)>,
    ) -> Option<Self> {
        if matches!(before.items.get(marker_index), Some(Item::Instruction(marker))
            if marker.exception_policy() == ExceptionPolicy::AlwaysAbsentPostTryCondition)
            && let Some(certificate) =
                Self::capture_post_try_fusion(before, after, start, marker_index, previous?)
        {
            return Some(certificate);
        }
        let start = before.original_array_start(start)?;
        let Item::Label(anchor) = before.items[start] else {
            return None;
        };
        let result = Self::capture_interval(
            before,
            Anchor::Label(anchor),
            start,
            marker_index,
            previous?,
        )?;
        result.valid(after, false)?;
        Some(result)
    }

    /// Reuses a protected slot only after one observed local-load fusion in this array.
    fn capture_post_try_fusion(
        before: &View<'_>,
        after: &View<'_>,
        start: usize,
        marker_index: usize,
        (condition_index, condition): (usize, Instruction),
    ) -> Option<Self> {
        let Item::Instruction(marker) = before.items[marker_index] else {
            return None;
        };
        key(condition, marker)?;
        let owner = before.post_try_fusion_array(start, condition_index, marker_index, false)?;
        let Item::Label(anchor) = before.items[start] else {
            return None;
        };
        let (after_condition, after_marker) = after.endpoint(condition, marker)?;
        let after_start = *after.label_index(anchor)?;
        if after_start > after_condition {
            return None;
        }
        if before.items[start..=marker_index].len()
            != after.items[after_start..=after_marker].len() + 1
            || !one_fusion(
                &before.items[start..=marker_index],
                &after.items[after_start..=after_marker],
            )
        {
            return None;
        }
        let prefix = before.items[start..=marker_index]
            .iter()
            .zip(&after.items[after_start..=after_marker])
            .take_while(|(a, b)| same_item(**a, **b, false))
            .count();
        if !matches!(&before.items[start + prefix..start + prefix + 2],
            [Item::Instruction(first), Item::Instruction(second)]
                if first.opcode == LOAD_FAST && second.opcode == LOAD_FAST)
            || after.post_try_fusion_array(after_start, after_condition, after_marker, true)?
                != owner
        {
            return None;
        }
        let items = after.items[after_start..=after_marker].to_vec();
        let boundaries = boundaries_in(&items, &after.boundaries);
        let result = Self {
            interval: Certificate {
                anchor: Anchor::Label(anchor),
                condition,
                marker,
                labels: labels_in(&items),
                items,
                owner,
                boundaries,
                setups: before.provenance.setups.clone(),
            },
            removed_slots: 1,
            observed_local_fusion: true,
        };
        result.valid(after, false)?;
        Some(result)
    }

    /// Captures a complete original array after its authored conditional predecessor.
    fn capture_original_fallthrough(
        view: &View<'_>,
        anchor: Anchor,
        start: usize,
        marker: usize,
        previous: Option<(usize, Instruction)>,
    ) -> Option<Self> {
        if !matches!(anchor, Anchor::OriginalMarker(_, _)) {
            return None;
        }
        let result = Self::capture_interval(view, anchor, start, marker, previous?)?;
        result.valid(view, false)?;
        Some(result)
    }

    fn capture_interval(
        before: &View<'_>,
        anchor: Anchor,
        start: usize,
        marker_index: usize,
        (condition_index, condition): (usize, Instruction),
    ) -> Option<Self> {
        let Item::Instruction(marker) = before.items[marker_index] else {
            return None;
        };
        key(condition, marker)?;
        let (owner, removed_slots) = if marker.exception_policy()
            == ExceptionPolicy::AlwaysAbsentPostTryCondition
            && matches!(anchor, Anchor::Label(_))
        {
            before.post_try_fold_array(start, condition_index, marker_index)?
        } else {
            if marker.exception_policy().exclusion() != ExceptionExclusion::None
                || !matches!(
                    marker.exception_policy().owner(),
                    NormalizedExceptionOwner::Present | NormalizedExceptionOwner::Absent
                )
            {
                return None;
            }
            before.uniform_fallback_interval(start, condition_index, false)?
        };
        let items = before.items[start..=marker_index].to_vec();
        let boundaries = boundaries_in(&items, &before.boundaries);
        let result = Self {
            interval: Certificate {
                anchor,
                condition,
                marker,
                labels: labels_in(&items),
                items,
                owner,
                boundaries,
                setups: before.provenance.setups.clone(),
            },
            removed_slots,
            observed_local_fusion: false,
        };
        Some(result)
    }
    /// Updates only the two conditional identities recorded by the exit-copy pass.
    fn rebind_copied_exit(&mut self, retargets: &[(Instruction, Instruction)]) -> Option<()> {
        let Anchor::OriginalMarker(origin_condition, origin_marker) = self.interval.anchor else {
            return Some(());
        };
        let rebind = |condition: Instruction| -> Option<Instruction> {
            let mut matches = retargets
                .iter()
                .filter(|(before, _)| same_instruction(condition, *before, false));
            let Some((before, after)) = matches.next() else {
                return Some(condition);
            };
            if matches.next().is_some()
                || !crate::target::is_conditional_jump(before.opcode)
                || before.forward_target().is_none()
                || after.forward_target().is_none()
            {
                return None;
            }
            let mut expected = *before;
            expected.retarget(after.operand());
            same_instruction(expected, *after, false).then_some(*after)
        };
        let origin_condition = rebind(origin_condition)?;
        let condition = rebind(self.interval.condition)?;
        let stored = self
            .interval
            .items
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                Item::Instruction(instruction) if instruction.opcode != NOT_TAKEN => {
                    Some(instruction)
                }
                _ => None,
            })?;
        if !same_instruction(*stored, self.interval.condition, false) {
            return None;
        }
        *stored = condition;
        self.interval.condition = condition;
        self.interval.anchor = Anchor::OriginalMarker(origin_condition, origin_marker);
        Some(())
    }
    /// Carries a uniformly owned authored array through one observed local fusion.
    fn rebind_one_fusion(&mut self, before: &View<'_>, after: &View<'_>) -> Option<()> {
        let Anchor::OriginalMarker(prior, marker) = self.interval.anchor else {
            return None;
        };
        if self.observed_local_fusion {
            return None;
        }
        self.valid(before, false)?;
        let (_, end) = after.endpoint(self.interval.condition, self.interval.marker)?;
        let mut found = None;
        for &(condition_index, marker_index) in after.markers.get(&key(prior, marker)?)? {
            let (Item::Instruction(condition), Item::Instruction(current_marker)) =
                (after.items[condition_index], after.items[marker_index])
            else {
                return None;
            };
            if marker_index >= end
                || !same_instruction(prior, condition, false)
                || !same_instruction(marker, current_marker, false)
            {
                continue;
            }
            let items = after.items[marker_index + 1..=end]
                .iter()
                .copied()
                .filter(|item| match item {
                    Item::Label(label) => {
                        self.interval.labels.contains(label) || after.is_reference(*label)
                    }
                    Item::Instruction(_) => true,
                })
                .collect::<Vec<_>>();
            if one_fusion(&self.interval.items, &items) && found.replace(items).is_some() {
                return None;
            }
        }
        self.interval.items = found?;
        self.interval.labels = labels_in(&self.interval.items);
        self.removed_slots += 1;
        self.observed_local_fusion = true;
        // The complete interval, exact owner, setup list, and boundaries must still match.
        self.valid(after, false)?;
        Some(())
    }

    fn valid(&self, view: &View<'_>, borrow: bool) -> Option<(usize, Owner)> {
        let marker = self.interval.valid(view, borrow)?;
        let (start, condition, _) = self.interval.range(view, borrow)?;
        let (owner, removed) = match self.interval.anchor {
            Anchor::Label(_) => {
                if view.original_array_start(start)? != start {
                    return None;
                }
                if self.interval.marker.exception_policy()
                    == ExceptionPolicy::AlwaysAbsentPostTryCondition
                {
                    if self.observed_local_fusion {
                        (
                            view.post_try_fusion_array(start, condition, marker, true)?,
                            1,
                        )
                    } else {
                        view.post_try_fold_array(start, condition, marker)?
                    }
                } else {
                    view.uniform_fallback_slots(start, condition)?
                }
            }
            Anchor::OriginalMarker(_, _) => {
                view.uniform_fallback_interval(start, condition, self.observed_local_fusion)?
            }
            Anchor::Entry | Anchor::Marker(_, _) => return None,
        };
        if owner != self.interval.owner || removed != self.removed_slots {
            return None;
        }
        Some((marker, if removed == 0 { None } else { owner }))
    }
}

impl View<'_> {
    /// Requires an uninterrupted source array with a single uniform fallback owner.
    fn post_try_fusion_array(
        &self,
        start: usize,
        condition: usize,
        marker: usize,
        fused: bool,
    ) -> Option<Owner> {
        if start > condition || condition >= marker || marker >= self.items.len() {
            return None;
        }
        let Item::Label(anchor) = self.items[start] else {
            return None;
        };
        if self.original_array_start(start)? != start
            || !self.provenance.source_boundaries.contains(&anchor)
            || self.items[start + 1..=marker].iter().any(|item| {
                matches!(item,
                Item::Label(label) if self.boundaries.contains(label)
                    || self.provenance.source_boundaries.contains(label))
            })
            || !self.no_late_insertions_or_setups(start, marker)
        {
            return None;
        }
        let (owner, removed) = self.uniform_fallback_interval(start, condition, fused)?;
        (removed == usize::from(fused)).then_some(owner)
    }

    /// Certifies literal-fold vacancies in one uninterrupted post-try source array.
    fn post_try_fold_array(
        &self,
        start: usize,
        condition: usize,
        marker: usize,
    ) -> Option<(Owner, usize)> {
        if start > condition || condition >= marker || marker >= self.items.len() {
            return None;
        }
        let (Item::Label(anchor), Item::Instruction(marker_instruction)) =
            (self.items[start], self.items[marker])
        else {
            return None;
        };
        if self.original_array_start(start)? != start
            || !self.provenance.source_boundaries.contains(&anchor)
            || self.items[start + 1..=marker].iter().any(|item| {
                matches!(item,
                Item::Label(label) if self.boundaries.contains(label)
                    || self.provenance.source_boundaries.contains(label))
            })
            || marker_instruction.pre_fold_extra_instructions() != 0
            || marker_instruction
                .local_policy
                .hidden_exception_owner_slots()
                != 0
            || self.provenance.setups.iter().any(|(label, _)| {
                self.label_index(*label)
                    .is_none_or(|&index| start <= index && index <= marker)
            })
            || !self.items[start..condition].iter().any(|item| {
                matches!(item, Item::Instruction(instruction)
                    if self.uniform_literal_fold_slots(*instruction).is_some())
            })
        {
            return None;
        }
        self.uniform_owned_interval(start, condition, false, None, true)
    }

    /// Matches the unsaturated literal-fold count to its unique recorded survivor.
    fn uniform_literal_fold_slots(&self, instruction: Instruction) -> Option<usize> {
        let hidden = instruction.local_policy.hidden_exception_owner_slots();
        if instruction.opcode != crate::target::opcodes::LOAD_CONST
            || !(1..4).contains(&hidden)
            || u16::from(instruction.pre_fold_extra_instructions()) != hidden
        {
            return None;
        }
        let key = (instruction.location, instruction.opcode, hidden);
        let same_survivor = |current: Instruction| {
            (
                current.location,
                current.opcode,
                current.local_policy.hidden_exception_owner_slots(),
            ) == key
        };
        (self
            .provenance
            .uniform_fold_owner_slots
            .iter()
            .filter(|original| **original == key)
            .count()
            == 1
            && self
                .items
                .iter()
                .filter(
                    |item| matches!(item, Item::Instruction(current) if same_survivor(*current)),
                )
                .count()
                == 1)
            .then_some(usize::from(hidden))
    }

    /// Finds the sole original boundary in an adjacent run of labels.
    fn original_array_start(&self, start: usize) -> Option<usize> {
        let mut first = start;
        while first > 0 && matches!(self.items[first - 1], Item::Label(_)) {
            first -= 1;
        }
        let mut anchor = None;
        for (offset, item) in self.items[first..].iter().enumerate() {
            let Item::Label(label) = item else {
                break;
            };
            if self.provenance.block_boundaries.contains(label)
                && anchor.replace(first + offset).is_some()
            {
                return None;
            }
        }
        anchor
    }
    fn uniform_fallback_slots(&self, start: usize, condition: usize) -> Option<(Owner, usize)> {
        let Item::Label(anchor) = self.items[start] else {
            return None;
        };
        // Only the original graph/source provenance establishes an array start.
        // A target introduced by a later pass is merely a conservative split.
        if !self.provenance.block_boundaries.contains(&anchor) {
            return None;
        }
        self.uniform_fallback_interval(start, condition, false)
    }

    fn uniform_fallback_interval(
        &self,
        start: usize,
        condition: usize,
        observed_local_fusion: bool,
    ) -> Option<(Owner, usize)> {
        self.uniform_owned_interval(start, condition, observed_local_fusion, None, false)
    }

    /// Proves a literal singleton array under one authored named-handler target.
    /// No owner is copied: the marker is appended beyond every original live cell.
    fn fresh_named_handler_singleton_regions(
        &self,
        start: usize,
        condition: usize,
    ) -> Option<(Label, Vec<ExceptionRegion>)> {
        if start > condition || condition >= self.items.len() {
            return None;
        }
        // Missing labels, hidden SETUP/POP cells, and unknown original entries fail closed.
        for (label, _) in &self.provenance.setups {
            let position = *self.label_index(*label)?;
            if start <= position && position <= condition {
                return None;
            }
        }
        if self
            .provenance
            .removed_pop_block_boundaries
            .iter()
            .any(|label| {
                self.label_index(*label)
                    .is_none_or(|&position| start <= position && position <= condition)
            })
        {
            return None;
        }
        let mut position = self.instruction_position(start);
        let mut original_regions: Option<Vec<ExceptionRegion>> = None;
        for (offset, item) in self.items[start..=condition].iter().enumerate() {
            let instruction = match item {
                Item::Label(label) => {
                    if offset != 0 && self.boundaries.contains(label) {
                        return None;
                    }
                    continue;
                }
                Item::Instruction(instruction) => instruction,
            };
            if instruction.exception_policy() != ExceptionPolicy::NoneUnchanged
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.local_policy.hidden_exception_owner_slots() != 0
                || instruction.only_allows_unconditional_inlining()
                || instruction.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
                || !matches!(
                    instruction.opcode,
                    crate::target::opcodes::UNPACK_SEQUENCE
                        | crate::target::opcodes::LOAD_CONST
                        | crate::target::opcodes::IS_OP
                        | POP_JUMP_IF_FALSE
                )
                || (instruction.opcode.has_jump() && start + offset != condition)
            {
                return None;
            }
            let mut current = Vec::new();
            for region in self.regions {
                let begin = *self.label_position(region.start)?;
                let end = *self.label_position(region.end)?;
                if begin <= position && position < end {
                    current.push(*region);
                }
            }
            if let Some(expected) = &original_regions {
                if expected.len() != current.len()
                    || !expected
                        .iter()
                        .all(|old| current.iter().any(|new| same_manager_region(old, new)))
                {
                    return None;
                }
            } else {
                original_regions = Some(current);
            }
            position += 1;
        }
        let regions = original_regions?;
        let mut named = regions.iter().filter(|region| {
            !region.is_scope_fallback
                && region.preserve_lasti
                && region.depth > 0
                && self
                    .provenance
                    .named_handler_cleanup_targets
                    .contains(&region.target)
        });
        let target = named.next()?.target;
        if named.next().is_some() {
            return None;
        }
        Some((target, regions))
    }

    fn uniform_owned_interval(
        &self,
        start: usize,
        condition: usize,
        observed_local_fusion: bool,
        manager_regions: Option<&[ExceptionRegion]>,
        allow_literal_folds: bool,
    ) -> Option<(Owner, usize)> {
        // No hidden SETUP/POP_BLOCK records or second true boundary may enter
        // the certificate, including aliases that only become adjacent later.
        for (label, _) in &self.provenance.setups {
            let p = *self.label_index(*label)?;
            if start <= p && p <= condition {
                return None;
            }
        }
        if !self
            .provenance
            .removed_pop_block_boundaries
            .iter()
            .all(|label| {
                self.label_index(*label)
                    .is_some_and(|&p| p < start || p > condition)
            })
        {
            return None;
        }
        let mut owner = None;
        let mut removed = 0;
        let mut previous_instruction = None;
        let mut pos = self.instruction_position(start);
        for (offset, item) in self.items[start..=condition].iter().copied().enumerate() {
            let instruction = match item {
                Item::Label(label) => {
                    if offset != 0 && self.boundaries.contains(&label) {
                        return None;
                    }
                    continue;
                }
                Item::Instruction(instruction) => instruction,
            };
            let literal_fold_slots = if allow_literal_folds {
                self.uniform_literal_fold_slots(instruction)
            } else {
                None
            };
            if instruction.exception_policy() != ExceptionPolicy::NoneUnchanged
                || instruction.pre_fold_extra_instructions() != 0 && literal_fold_slots.is_none()
                || instruction.only_allows_unconditional_inlining()
                || instruction.local_policy.hidden_exception_owner_slots() != 0
                    && literal_fold_slots.is_none()
                || instruction.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
                || matches!(
                    instruction.opcode,
                    NOP | NOT_TAKEN
                        | crate::target::opcodes::COPY
                        | COPY_FREE_VARS
                        | MAKE_CELL
                        | RETURN_GENERATOR
                        | crate::target::opcodes::FOR_ITER
                        | crate::target::opcodes::SEND
                )
                || !observed_local_fusion
                    && matches!(
                        instruction.opcode,
                        LOAD_FAST_LOAD_FAST
                            | LOAD_FAST_BORROW_LOAD_FAST_BORROW
                            | STORE_FAST_LOAD_FAST
                            | STORE_FAST_STORE_FAST
                    )
                || is_scope_exit(instruction.opcode)
                || instruction.opcode.has_jump() && start + offset != condition
                || instruction.opcode == crate::target::opcodes::COMPARE_OP
                    && matches!(instruction.operand(), Operand::Value(argument) if argument & 16 != 0)
            {
                return None;
            }
            if instruction.opcode == STORE_FAST
                && instruction.fusion_policy.prevents_with_previous()
                && previous_instruction.is_some_and(|previous: Instruction| {
                    matches!(
                        previous.opcode,
                        crate::target::opcodes::POP_TOP | STORE_FAST
                    )
                })
            {
                return None;
            }
            previous_instruction = Some(instruction);
            let mut covered = None;
            for region in self.regions {
                let s = *self.label_position(region.start)?;
                let e = *self.label_position(region.end)?;
                if s <= pos && pos < e {
                    let fallback = region.is_scope_fallback && region.depth == 0;
                    // This authored singleton array has no erased or inserted records.
                    // One unchanged synchronous manager may own the live records, while
                    // the late marker appends into the fresh NULL slot after them.
                    let manager = manager_regions.is_some_and(|regions| {
                        regions
                            .iter()
                            .any(|original| same_manager_region(original, region))
                    });
                    if covered.is_some() || !(fallback || manager) || !region.preserve_lasti {
                        return None;
                    }
                    covered = Some((region.target, region.depth, region.preserve_lasti));
                }
            }
            let current = covered?;
            if owner.is_some_and(|old| old != current) {
                return None;
            }
            owner = Some(current);
            removed += literal_fold_slots.unwrap_or(0);
            if matches!((instruction.opcode, instruction.operand()),
                (crate::target::opcodes::LOAD_GLOBAL, Operand::Value(argument)) if argument & 1 != 0)
                || matches!(
                    instruction.opcode,
                    LOAD_FAST_LOAD_FAST
                        | LOAD_FAST_BORROW_LOAD_FAST_BORROW
                        | STORE_FAST_LOAD_FAST
                        | STORE_FAST_STORE_FAST
                )
            {
                removed += 1;
            }
            pos += 1;
        }
        Some((Some(owner?), removed))
    }
}

/// Retains one exact manager-entry cell across the known local fusion transaction.
struct ManagerSetupCertificate {
    interval: Certificate,
    source: Instruction,
    region: ExceptionRegion,
    regions: Vec<ExceptionRegion>,
}

pub(super) fn same_manager_region(a: &ExceptionRegion, b: &ExceptionRegion) -> bool {
    a.start == b.start
        && a.end == b.end
        && a.target == b.target
        && a.depth == b.depth
        && a.preserve_lasti == b.preserve_lasti
        && a.is_scope_fallback == b.is_scope_fallback
        && match (a.priority, b.priority) {
            (
                ExceptionRegionPriority::OriginalRange(a, b),
                ExceptionRegionPriority::OriginalRange(c, d),
            ) => a == c && b == d,
            (
                ExceptionRegionPriority::Resolved { start: a, end: b },
                ExceptionRegionPriority::Resolved { start: c, end: d },
            ) => a == c && b == d,
            _ => false,
        }
}

impl ManagerSetupCertificate {
    fn capture(
        before: &View<'_>,
        after: &View<'_>,
        start: usize,
        marker: usize,
        previous: Option<(usize, Instruction)>,
    ) -> Option<Self> {
        // Entry arrays have an unambiguous physical origin, including the resumable prefix.
        if start != 0 {
            return None;
        }
        let (_, condition) = previous?;
        let Item::Instruction(marker_instruction) = before.items[marker] else {
            return None;
        };
        if marker_instruction.exception_policy().exclusion() != ExceptionExclusion::None {
            return None;
        }
        let (_, after_marker) = after.endpoint(condition, marker_instruction)?;
        // An unchanged transaction preserves the same concrete SETUP-array cells.
        if before.items[..=marker] != after.items[..=after_marker]
            && !one_fusion(&before.items[..=marker], &after.items[..=after_marker])
        {
            return None;
        }
        let mut cells = Vec::new();
        for (index, item) in after.items[..after_marker].iter().enumerate() {
            match item {
                Item::Instruction(instruction) => {
                    if instruction.opcode == NOP {
                        return None;
                    }
                    cells.push(Some(index));
                }
                Item::Label(label) => {
                    if after.boundaries.contains(label) {
                        return None;
                    }
                    if let Some((_, location)) = after
                        .provenance
                        .setups
                        .iter()
                        .find(|(setup, _)| setup == label)
                    {
                        if location.is_some()
                            || !(after.provenance.synchronous_with_setups.contains(label)
                                || after.regions.iter().any(|region| {
                                    region.is_scope_fallback && region.start == *label
                                }))
                        {
                            return None;
                        }
                        cells.push(None);
                    } else if after.is_reference(*label) {
                        return None;
                    }
                }
            }
        }
        // SETUP records survive early compaction and fusion, then disappear before
        // NOT_TAKEN is appended. Its live ordinal selects a concrete old-array cell.
        let ordinal = after.instruction_position(after_marker);
        let source_index = *cells.get(ordinal)?.as_ref()?;
        let Item::Instruction(source) = after.items[source_index] else {
            return None;
        };
        if source.exception_policy() != ExceptionPolicy::NoneUnchanged {
            return None;
        }
        let position = after.instruction_position(source_index);
        let mut regions = Vec::new();
        for (index, region) in after.regions.iter().enumerate() {
            let begin = *after.label_position(region.start)?;
            let end = *after.label_position(region.end)?;
            if begin <= position && position < end {
                let (priority_start, priority_end) = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*after.label_position(start)?, *after.label_position(end)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                regions.push((
                    (
                        region.is_scope_fallback,
                        Reverse(priority_start),
                        priority_end,
                        index,
                    ),
                    *region,
                ));
            }
        }
        let region = regions.into_iter().min_by_key(|(key, _)| *key)?.1;
        if region.is_scope_fallback
            || !after
                .provenance
                .synchronous_with_setups
                .contains(&region.start)
        {
            return None;
        }
        let mut original_sources =
            before.items[..marker]
                .iter()
                .enumerate()
                .filter_map(|(index, item)| match item {
                    Item::Instruction(instruction)
                        if same_instruction(source, *instruction, false) =>
                    {
                        Some(index)
                    }
                    _ => None,
                });
        let original_source = original_sources.next()?;
        if original_sources.next().is_some() {
            return None;
        }
        let original_position = before.instruction_position(original_source);
        if !(*before.label_position(region.start)? <= original_position
            && original_position < *before.label_position(region.end)?)
        {
            return None;
        }
        let certificate = Self {
            interval: Certificate {
                anchor: Anchor::Entry,
                condition,
                marker: marker_instruction,
                items: after.items[..=after_marker].to_vec(),
                labels: labels_in(&after.items[..=after_marker]),
                owner: None,
                boundaries: boundaries_in(&after.items[..=after_marker], &after.boundaries),
                setups: after.provenance.setups.clone(),
            },
            source,
            region,
            regions: after.regions.to_vec(),
        };
        certificate.valid(after, false)?;
        Some(certificate)
    }

    fn valid(&self, view: &View<'_>, borrow: bool) -> Option<(usize, usize)> {
        let (start, condition, marker) = self.interval.range(view, borrow)?;
        if self.interval.setups != view.provenance.setups
            || self.regions.len() != view.regions.len()
            || !self
                .regions
                .iter()
                .zip(view.regions)
                .all(|(a, b)| same_manager_region(a, b))
        {
            return None;
        }
        if !self.interval.same_boundaries(view) {
            return None;
        }
        self.interval
            .matches_interval(view, start, marker, borrow)?;
        let mut sources =
            view.items[start..marker]
                .iter()
                .enumerate()
                .filter_map(|(offset, item)| match item {
                    Item::Instruction(instruction)
                        if same_instruction(self.source, *instruction, borrow) =>
                    {
                        Some(start + offset)
                    }
                    _ => None,
                });
        let source = sources.next()?;
        if sources.next().is_some() {
            return None;
        }
        let mut regions = view
            .regions
            .iter()
            .filter(|region| same_manager_region(region, &self.region));
        let region = regions.next()?;
        if regions.next().is_some() {
            return None;
        }
        let position = view.instruction_position(source);
        if !(*view.label_position(region.start)? <= position
            && position < *view.label_position(region.end)?)
        {
            return None;
        }
        Some((condition, marker))
    }
}

/// One uniformly owned original dispatch with at most one accounted-for compaction.
struct HandlerDispatchCertificate {
    interval: Certificate,
    region: ExceptionRegion,
    removed: usize,
    observed_fusion: bool,
}

impl HandlerDispatchCertificate {
    fn capture(
        view: &View<'_>,
        origin: &super::exception_owners::HandlerDispatchArray,
    ) -> Option<Self> {
        use crate::target::opcodes::CHECK_EXC_MATCH;

        let mut instructions = origin.items.iter().rev().filter_map(|item| match item {
            Item::Instruction(instruction) => Some(instruction),
            Item::Label(_) => None,
        });
        let (marker, condition, check) = (
            instructions.next()?,
            instructions.next()?,
            instructions.next()?,
        );
        if check.opcode != CHECK_EXC_MATCH
            || condition.opcode != POP_JUMP_IF_FALSE
            || marker.exception_policy() != ExceptionPolicy::EXCLUDE_ALWAYS
        {
            return None;
        }
        let (condition_index, _) = view.endpoint(*condition, *marker)?;
        let region = Self::owner(view, condition_index)?;
        if region.target != origin.target
            || region.depth != origin.depth
            || !region.preserve_lasti
            || region.is_scope_fallback
        {
            return None;
        }
        let interval = Certificate {
            anchor: Anchor::Label(origin.entry),
            condition: *condition,
            marker: *marker,
            items: origin.items.clone(),
            labels: labels_in(&origin.items),
            owner: Some((region.target, region.depth, region.preserve_lasti)),
            boundaries: boundaries_in(&origin.items, &view.boundaries),
            setups: view.provenance.setups.clone(),
        };
        let mut result = Self {
            interval,
            region,
            removed: 0,
            observed_fusion: false,
        };
        let (start, condition, _) = result.interval.valid_array(view, false)?;
        result.removed = result.slots(view, start, condition)?;
        result.valid(view, false)?;
        Some(result)
    }

    /// Selects the concrete live record's owner with the layout's existing priority.
    fn owner(view: &View<'_>, index: usize) -> Option<ExceptionRegion> {
        let Item::Instruction(instruction) = view.items[index] else {
            return None;
        };
        if instruction.exception_policy() != ExceptionPolicy::NoneUnchanged {
            return None;
        }
        let position = view.instruction_position(index);
        Self::owner_at(view, position)
    }

    fn owner_at(view: &View<'_>, position: usize) -> Option<ExceptionRegion> {
        let mut owner = None;
        for (order, region) in view.regions.iter().enumerate() {
            let start = *view.label_position(region.start)?;
            let end = *view.label_position(region.end)?;
            if start <= position && position < end {
                let (start, end) = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*view.label_position(start)?, *view.label_position(end)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                let candidate = (
                    (region.is_scope_fallback, Reverse(start), end, order),
                    *region,
                );
                if owner
                    .as_ref()
                    .is_none_or(|(priority, _)| candidate.0 < *priority)
                {
                    owner = Some(candidate);
                }
            }
        }
        owner.map(|(_, region)| region)
    }

    /// Checks that one exact region owns every instruction in a half-open interval.
    fn owns_interval(
        view: &View<'_>,
        first: usize,
        end: usize,
        expected: &ExceptionRegion,
    ) -> Option<()> {
        if first >= end {
            return Some(());
        }
        let mut owner = None;
        for (order, region) in view.regions.iter().enumerate() {
            // Even an unrelated region with missing physical bounds invalidates the old lookup.
            let physical_start = *view.label_position(region.start)?;
            let physical_end = *view.label_position(region.end)?;
            if physical_start.max(first) >= physical_end.min(end) {
                continue;
            }
            let (priority_start, priority_end) = match region.priority {
                ExceptionRegionPriority::OriginalRange(start, end) => {
                    (*view.label_position(start)?, *view.label_position(end)?)
                }
                ExceptionRegionPriority::Resolved { start, end } => (start, end),
            };
            let priority = (
                region.is_scope_fallback,
                Reverse(priority_start),
                priority_end,
                order,
            );
            if owner
                .as_ref()
                .is_none_or(|(previous, _, _, _)| priority < *previous)
            {
                owner = Some((priority, region, physical_start, physical_end));
            }
        }
        let (_, region, physical_start, physical_end) = owner?;
        // Exact manager equality includes both physical endpoints. A different winner of the
        // global priority owns at least one cell; this winner owns all only if it spans the range.
        (same_manager_region(region, expected) && physical_start <= first && end <= physical_end)
            .then_some(())
    }

    /// Accounts only for a global-call NULL fold or an observed pair of local loads.
    fn slots(&self, view: &View<'_>, start: usize, condition: usize) -> Option<usize> {
        use crate::target::opcodes::{
            BUILD_TUPLE, CALL, CHECK_EXC_MATCH, LOAD_ATTR, LOAD_CONST, LOAD_FAST_CHECK, LOAD_GLOBAL,
        };

        if !view.no_late_insertions_or_setups(start, condition)
            || view.provenance.setups.iter().any(|(label, _)| {
                view.label_index(*label)
                    .is_none_or(|position| start <= *position && *position <= condition)
            })
            || view
                .provenance
                .removed_pop_block_boundaries
                .iter()
                .any(|label| {
                    view.label_index(*label)
                        .is_none_or(|position| start <= *position && *position <= condition)
                })
        {
            return None;
        }
        let mut removed = 0;
        let mut global_call = false;
        let mut call = false;
        for (offset, item) in view.items[start..=condition].iter().enumerate() {
            let index = start + offset;
            let instruction = match item {
                Item::Label(label) => {
                    if offset != 0 && view.is_reference(*label) {
                        return None;
                    }
                    continue;
                }
                Item::Instruction(instruction) => instruction,
            };
            if instruction.exception_policy() != ExceptionPolicy::NoneUnchanged
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.local_policy.hidden_exception_owner_slots() != 0
                || instruction.only_allows_unconditional_inlining()
                || instruction.no_location_policy.borrow_fallthrough()
                    == super::BorrowFallthrough::Barrier
            {
                return None;
            }
            match instruction.opcode {
                LOAD_GLOBAL => {
                    let argument = instruction.value_argument()?;
                    if argument & 1 != 0 {
                        removed += 1;
                        global_call = true;
                    }
                }
                LOAD_FAST_LOAD_FAST | LOAD_FAST_BORROW_LOAD_FAST_BORROW if self.observed_fusion => {
                    removed += 1;
                }
                LOAD_FAST | LOAD_FAST_BORROW | LOAD_FAST_CHECK | LOAD_CONST | BUILD_TUPLE
                | CHECK_EXC_MATCH => {}
                LOAD_ATTR if matches!(instruction.operand(), Operand::Value(argument) if argument & 1 == 0) =>
                    {}
                // Code generation emits a zero-argument method lookup as one LOAD_ATTR
                // record followed by CALL. Unlike a global call, it has no PUSH_NULL
                // record to compact, so the existing one-slot certificate is unchanged.
                LOAD_ATTR
                    if matches!(instruction.operand(), Operand::Value(argument) if argument & 1 != 0)
                        && matches!(view.items.get(index + 1), Some(Item::Instruction(next))
                            if next.opcode == CALL && next.value_argument() == Some(0)) => {}
                CALL => call = true,
                POP_JUMP_IF_FALSE if index == condition => {}
                _ => return None,
            }
        }
        if removed > 1 || (global_call && !call) {
            return None;
        }
        Self::owns_interval(
            view,
            view.instruction_position(start),
            view.instruction_position(condition + 1),
            &self.region,
        )?;
        Some(removed)
    }

    fn valid(&self, view: &View<'_>, borrow: bool) -> Option<usize> {
        let (start, condition, marker) = self.interval.valid_array(view, borrow)?;
        (self.slots(view, start, condition)? == self.removed).then_some(marker)
    }

    fn rebind_one_fusion(&mut self, before: &View<'_>, after: &View<'_>) -> Option<()> {
        if self.observed_fusion || self.removed != 0 {
            return None;
        }
        self.valid(before, false)?;
        let (start, _, marker) = self.interval.range(after, false)?;
        let items = after.items[start..=marker].to_vec();
        if !one_fusion(&self.interval.items, &items) {
            return None;
        }
        self.interval.items = items;
        self.interval.labels = labels_in(&self.interval.items);
        self.observed_fusion = true;
        self.removed = 1;
        self.valid(after, false)?;
        Some(())
    }

    fn rebind_cold_region(
        &mut self,
        view: &View<'_>,
        retargets: &[(ExceptionRegion, ExceptionRegion)],
    ) -> Option<()> {
        if self.valid(view, false).is_some() {
            return Some(());
        }
        let (_, condition, _) = self.interval.valid_array(view, false)?;
        let selected = Self::owner(view, condition)?;
        let mut replacements = retargets.iter().filter_map(|(original, rebuilt)| {
            let mut shape = *rebuilt;
            shape.start = self.region.start;
            shape.end = self.region.end;
            (same_manager_region(original, &self.region)
                && same_manager_region(&shape, &self.region)
                && same_manager_region(rebuilt, &selected))
            .then_some(*rebuilt)
        });
        let region = replacements.next()?;
        if replacements.next().is_some() {
            return None;
        }
        self.region = region;
        self.valid(view, false)?;
        Some(())
    }
}

/// A folded assertion's empty success array under one authenticated owner.
struct FoldedAssertSuccessCertificate {
    entry: Label,
    next: Label,
    jump: Instruction,
    owner: ExceptionRegion,
}

impl FoldedAssertSuccessCertificate {
    fn sole_owner(view: &View<'_>, index: usize) -> Option<ExceptionRegion> {
        let position = view.instruction_position(index);
        let mut owner = None;
        for region in view.regions {
            let start = *view.label_position(region.start)?;
            let end = *view.label_position(region.end)?;
            if start <= position && position < end {
                if !region.preserve_lasti || owner.replace(*region).is_some() {
                    return None;
                }
            }
        }
        owner
    }

    fn capture(view: &View<'_>, index: usize) -> Option<Self> {
        let [
            Item::Instruction(nop),
            Item::Label(entry),
            Item::Instruction(jump),
            Item::Label(next),
        ] = view.items.get(index..index + 4)?
        else {
            return None;
        };
        // This is the complete constant-true assertion producer: its three old
        // condition records fold to a NOP before a separate empty success array.
        let mut expected =
            Instruction::new(NOP, Operand::Value(0), nop.location, nop.depth_after());
        expected.add_pre_fold_extra_instructions(2);
        expected
            .no_location_policy
            .set_borrow_fallthrough(super::BorrowFallthrough::Barrier);
        if *nop != expected
            || nop.location.line < 0
            || jump.opcode != crate::target::opcodes::JUMP_FORWARD
            || jump.forward_target().is_none()
            || jump.location != nop.location
            || jump.depth_after() != nop.depth_after()
            || jump.exception_policy() != ExceptionPolicy::NoneUnchanged
            || jump.pre_fold_extra_instructions() != 0
            || jump.local_policy != super::LocalPolicy::default()
            || jump.no_location_policy != super::ir::NoLocationPolicy::default()
        {
            return None;
        }
        let owner = Self::sole_owner(view, index + 2)?;
        let scope_fallback = owner.is_scope_fallback && owner.depth == 0;
        // This original SETUP owns the single manager's two saved exit slots.
        // Capture it before cold ordering can split the protected range.
        let synchronous_manager = !owner.is_scope_fallback
            && owner.depth == 2
            && view
                .provenance
                .synchronous_with_setups
                .contains(&owner.start);
        // The branch emitter authenticates this separate assertion-success array
        // before asking inlining to retain its source jump on a direct copy.
        let mut direct_target = super::ir::JumpPolicy::default();
        direct_target.retain_nop_for_direct_target();
        direct_target.mark_original_empty_join();
        if !(scope_fallback || synchronous_manager)
            || (jump.jump_policy != super::ir::JumpPolicy::default()
                && !(synchronous_manager && jump.jump_policy == direct_target))
        {
            return None;
        }
        if !same_manager_region(&Self::sole_owner(view, index)?, &owner) {
            return None;
        }
        let result = Self {
            entry: *entry,
            next: *next,
            jump: *jump,
            owner,
        };
        result.valid(view)?;
        Some(result)
    }

    fn valid(&self, view: &View<'_>) -> Option<usize> {
        let start = *view.label_index(self.entry)?;
        if view.items.get(start..start + 3)?
            != [
                Item::Label(self.entry),
                Item::Instruction(self.jump),
                Item::Label(self.next),
            ]
            .as_slice()
            || !view.provenance.block_boundaries.contains(&self.entry)
            || !view.provenance.source_boundaries.contains(&self.entry)
            || view.provenance.normal_try_copy_blocks.contains(&self.entry)
            || view
                .provenance
                .normal_try_copy_inline_labels
                .contains(&self.entry)
            || view
                .provenance
                .normal_try_copy_internal_labels
                .contains(&self.entry)
            || view
                .provenance
                .normal_try_copy_jump_nop_successors
                .contains(&self.entry)
            || view.items.iter().any(|item| {
                matches!(item, Item::Instruction(instruction)
                    if matches!(instruction.operand(), Operand::Forward(label) | Operand::Backward(label)
                        if label == self.entry))
            })
            || view.regions.iter().any(|region| region.target == self.entry)
        {
            return None;
        }
        let owner = Self::sole_owner(view, start + 1)?;
        // Cold ordering can split the enclosing owner range. Its target,
        // stack state, and original priority still identify this same owner.
        let mut shape = owner;
        shape.start = self.owner.start;
        shape.end = self.owner.end;
        same_manager_region(&shape, &self.owner).then_some(start + 1)
    }
}

#[derive(Default)]
pub(super) struct FusionOwnerCertificates {
    certificates: Vec<Certificate>,
    handler_dispatch_certificates: Vec<HandlerDispatchCertificate>,
    manager_setup_certificates: Vec<ManagerSetupCertificate>,
    fallback_certificates: Vec<FallbackCertificate>,
    fresh_singleton_certificates: Vec<FreshSingletonCertificate>,
    folded_assert_success_certificates: Vec<FoldedAssertSuccessCertificate>,
    scratch: Option<Box<RetentionScratch>>,
}

impl FusionOwnerCertificates {
    /// Captures authored arrays before any CFG pass can reuse their owner slots.
    pub(super) fn capture_original_owners(p: &OptimizationPipeline) -> Self {
        if p.exception_regions.is_empty()
            || (p.exception_owner_provenance.handler_dispatch_arrays.is_empty()
                && (!p.graph.features().has_original_fusion_owner_candidate()
                    || !p.graph.iter_items().any(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if (instruction.opcode == NOT_TAKEN
                                && matches!(instruction.exception_policy(),
                                    ExceptionPolicy::NoneUnchangedBooleanSingleton
                                        | ExceptionPolicy::NonePresent | ExceptionPolicy::NoneAbsent))
                                || (instruction.opcode == NOP
                                    && instruction.pre_fold_extra_instructions() == 2
                                    && instruction.no_location_policy.borrow_fallthrough()
                                        == super::BorrowFallthrough::Barrier))
                    })))
        {
            return Self::default();
        }
        let items = p.graph.copy_items();
        let Some(view) = View::new(
            &items,
            &p.exception_regions,
            &p.exception_owner_provenance,
            &p.preserved_block_boundaries,
        ) else {
            return Self::default();
        };
        let mut result = Self::default();
        let mut start = 0;
        let mut anchor = None;
        let mut previous = None;
        let mut has_instruction = false;
        for (index, item) in items.iter().copied().enumerate() {
            match item {
                Item::Label(label) => {
                    if view.boundaries.contains(&label) {
                        if has_instruction {
                            start = index;
                            has_instruction = false;
                            anchor = None;
                        }
                        if let Some(original) = view.original_array_start(start) {
                            start = original;
                            let Item::Label(original) = items[start] else {
                                unreachable!()
                            };
                            anchor = Some(Anchor::Label(original));
                        }
                    }
                }
                Item::Instruction(instruction) => {
                    if instruction.opcode == NOT_TAKEN {
                        if let Some(origin) = anchor.clone()
                            && let Some(certificate) =
                                FreshSingletonCertificate::capture(&view, origin, start, index)
                        {
                            result.fresh_singleton_certificates.push(certificate);
                        }
                        if let Some(origin) = anchor.clone()
                            && let Some(certificate) =
                                FallbackCertificate::capture_original_fallthrough(
                                    &view, origin, start, index, previous,
                                )
                        {
                            result.fallback_certificates.push(certificate);
                        }
                        // In the original graph, a conditional terminates its array. The
                        // following marker is the compiler's placeholder for CPython's late
                        // append; the next instruction begins the original fallthrough array.
                        anchor = previous
                            .filter(|(_, prior)| key(*prior, instruction).is_some())
                            .map(|(_, prior)| Anchor::OriginalMarker(prior, instruction));
                        start = index + 1;
                        previous = None;
                        has_instruction = false;
                        continue;
                    }
                    has_instruction = true;
                    previous = Some((index, instruction));
                    if is_scope_exit(instruction.opcode)
                        || instruction.opcode.has_jump()
                            && !crate::target::is_conditional_jump(instruction.opcode)
                    {
                        start = index + 1;
                        anchor = None;
                        previous = None;
                        has_instruction = false;
                    }
                }
            }
        }
        result.folded_assert_success_certificates = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, Item::Instruction(instruction) if instruction.opcode == NOP))
            .filter_map(|(index, _)| FoldedAssertSuccessCertificate::capture(&view, index))
            .collect();
        result.handler_dispatch_certificates = p
            .exception_owner_provenance
            .handler_dispatch_arrays
            .iter()
            .filter_map(|origin| HandlerDispatchCertificate::capture(&view, origin))
            .collect();
        result
    }

    /// Moves original-array certificates into the existing later-pass retention path.
    fn extend_original_owners(&mut self, mut original: Self) {
        self.handler_dispatch_certificates
            .append(&mut original.handler_dispatch_certificates);
        self.fallback_certificates
            .append(&mut original.fallback_certificates);
        self.fresh_singleton_certificates
            .append(&mut original.fresh_singleton_certificates);
        self.folded_assert_success_certificates
            .append(&mut original.folded_assert_success_certificates);
        if self.scratch.is_none() {
            self.scratch = original.scratch.take();
        }
    }

    pub(super) fn snapshot(p: &OptimizationPipeline) -> Vec<Item> {
        if p.exception_regions.is_empty()
            || !p.graph.features().has_not_taken()
            || !p
                .graph
                .iter_items()
                .any(|x| matches!(x,Item::Instruction(i)if i.opcode==NOT_TAKEN))
        {
            Vec::new()
        } else {
            p.graph.copy_items()
        }
    }

    /// Retains authored certificates before capturing new certificates on the same fused graph.
    pub(super) fn after_fusion(
        self,
        before: Vec<Item>,
        p: &OptimizationPipeline,
        changed: bool,
    ) -> Self {
        self.after_fusion_with_snapshot_reuse::<true>(before, p, changed)
    }

    fn after_fusion_with_snapshot_reuse<const REUSE_UNCHANGED: bool>(
        mut self,
        mut before: Vec<Item>,
        p: &OptimizationPipeline,
        changed: bool,
    ) -> Self {
        if before.is_empty() {
            self.retain_after_fusion(&before, p);
            let mut result = Self::default();
            result.extend_original_owners(self);
            return result;
        }

        // The fusion pass reports every changed instruction or following location.
        // When unchanged, one snapshot and one index describe both sides of the
        // transaction, including certificates that admit the original owner as-is.
        if REUSE_UNCHANGED && !changed {
            let Some(view) = View::new(
                &before,
                &p.exception_regions,
                &p.exception_owner_provenance,
                &p.preserved_block_boundaries,
            ) else {
                self.retain_pipeline(p);
                let mut result = Self::default();
                result.extend_original_owners(self);
                return result;
            };
            let retain_tables = self.retain_captured_fusion_views(&view, &view);
            let mut result = Self::capture_views(&view, &view);
            if retain_tables {
                let mut tables = view.into_tables();
                tables.clear();
                before.clear();
                self.scratch = Some(Box::new(RetentionScratch {
                    items: before,
                    tables,
                }));
            }
            result.extend_original_owners(self);
            return result;
        }

        let mut after = p.graph.copy_items();
        let (Some(before_view), Some(after_view)) = (
            View::new(
                &before,
                &p.exception_regions,
                &p.exception_owner_provenance,
                &p.preserved_block_boundaries,
            ),
            View::new(
                &after,
                &p.exception_regions,
                &p.exception_owner_provenance,
                &p.preserved_block_boundaries,
            ),
        ) else {
            self.retain_pipeline(p);
            let mut result = Self::default();
            result.extend_original_owners(self);
            return result;
        };
        let retain_tables = self.retain_captured_fusion_views(&before_view, &after_view);
        let mut result = Self::capture_views(&before_view, &after_view);
        if retain_tables {
            let mut tables = after_view.into_tables();
            tables.clear();
            after.clear();
            self.scratch = Some(Box::new(RetentionScratch {
                items: after,
                tables,
            }));
        }
        result.extend_original_owners(self);
        result
    }

    fn retain_captured_fusion_views(&mut self, before: &View<'_>, after: &View<'_>) -> bool {
        if self.has_fusion_transfer_certificates() {
            self.retain_fusion_views(before, after);
            false
        } else if self.is_empty() {
            false
        } else {
            self.retain_view(after);
            if self.is_empty() {
                self.scratch = None;
                false
            } else {
                self.scratch.is_none()
            }
        }
    }

    #[cfg(test)]
    pub(super) fn capture(before: &[Item], p: &OptimizationPipeline) -> Self {
        if before.is_empty() {
            return Self::default();
        }
        let after = p.graph.iter_items().copied().collect::<Vec<_>>();
        let (Some(b), Some(a)) = (
            View::new(
                before,
                &p.exception_regions,
                &p.exception_owner_provenance,
                &p.preserved_block_boundaries,
            ),
            View::new(
                &after,
                &p.exception_regions,
                &p.exception_owner_provenance,
                &p.preserved_block_boundaries,
            ),
        ) else {
            return Self::default();
        };
        Self::capture_views(&b, &a)
    }

    fn capture_views(b: &View<'_>, a: &View<'_>) -> Self {
        let before = b.items;
        let after = a.items;
        let mut result = Self::default();
        let mut start = 0;
        let mut has_instruction = false;
        let mut previous = None;
        for (index, item) in before.iter().copied().enumerate() {
            match item {
                Item::Label(l) => {
                    if has_instruction && b.boundaries.contains(&l) {
                        start = index;
                        has_instruction = false;
                    }
                }
                Item::Instruction(marker) => {
                    if marker.opcode == NOT_TAKEN {
                        if let Some((condition_index, condition)) = previous {
                            if key(condition, marker).is_some()
                                && matches!(
                                    marker.exception_policy().owner(),
                                    NormalizedExceptionOwner::Unchanged
                                        | NormalizedExceptionOwner::DynamicCondition
                                        | NormalizedExceptionOwner::DynamicConditionStrict
                                )
                                && marker.exception_policy().exclusion() == ExceptionExclusion::None
                                && b.no_late_insertions_or_setups(start, index)
                            {
                                let anchor = if start == 0 {
                                    Some(Anchor::Entry)
                                } else if let Item::Label(l) = before[start] {
                                    Some(Anchor::Label(l))
                                } else {
                                    let mut prior =
                                        before[..start].iter().rev().filter_map(|x| match x {
                                            Item::Instruction(i) => Some(*i),
                                            Item::Label(_) => None,
                                        });
                                    match (prior.next(), prior.next()) {
                                        (Some(marker), Some(condition))
                                            if key(condition, marker).is_some() =>
                                        {
                                            Some(Anchor::Marker(condition, marker))
                                        }
                                        _ => None,
                                    }
                                };
                                if let (Some(anchor), Some(owner)) =
                                    (anchor, b.owner(condition_index))
                                {
                                    let mut cert = Certificate {
                                        anchor,
                                        condition,
                                        marker,
                                        items: Vec::new(),
                                        labels: FxHashSet::default(),
                                        owner,
                                        boundaries: FxHashSet::default(),
                                        setups: a.provenance.setups.clone(),
                                    };
                                    if let Some((s, c, m)) = cert.range(a, false) {
                                        if a.owner(c) == Some(owner)
                                            && a.no_late_insertions_or_setups(s, m)
                                            && one_fusion(&before[start..=index], &after[s..=m])
                                        {
                                            cert.items = after[s..=m].to_vec();
                                            cert.labels = labels_in(&cert.items);
                                            cert.boundaries =
                                                boundaries_in(&cert.items, &a.boundaries);
                                            result.certificates.push(cert);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(certificate) =
                            ManagerSetupCertificate::capture(b, a, start, index, previous)
                        {
                            result.manager_setup_certificates.push(certificate);
                        }
                        if let Some(cert) =
                            FallbackCertificate::capture(b, a, start, index, previous)
                        {
                            result.fallback_certificates.push(cert);
                        }
                        start = index + 1;
                        has_instruction = false;
                        previous = None;
                        continue;
                    }
                    has_instruction = true;
                    previous = Some((index, marker));
                    if is_scope_exit(marker.opcode)
                        || (marker.opcode.has_jump()
                            && !matches!(
                                marker.opcode,
                                POP_JUMP_IF_FALSE
                                    | POP_JUMP_IF_TRUE
                                    | POP_JUMP_IF_NONE
                                    | POP_JUMP_IF_NOT_NONE
                            ))
                    {
                        if let Some(cert) =
                            FallbackCertificate::capture(b, a, start, index, previous)
                        {
                            result.fallback_certificates.push(cert);
                        }
                        start = index + 1;
                        has_instruction = false;
                        previous = None;
                    }
                }
            }
        }
        result
    }
    /// Carries original fallback arrays and named-handler singleton proofs across exit copying.
    pub(super) fn retain_after_exit_copy(
        &mut self,
        p: &OptimizationPipeline,
        retargets: &[(Instruction, Instruction)],
        region_retargets: &[(ExceptionRegion, ExceptionRegion)],
    ) {
        // Copies carve exact holes in the original named-handler ranges. Transfer only
        // those certificates through the producer's records before checking the full array.
        for certificate in self
            .fresh_singleton_certificates
            .iter_mut()
            .filter(|certificate| certificate.named_handler_target.is_some())
        {
            let mut transferred = Vec::new();
            for region in &certificate.manager_regions {
                let previous_len = transferred.len();
                transferred.extend(region_retargets.iter().filter_map(|(original, rebuilt)| {
                    let mut original_shape = *rebuilt;
                    original_shape.start = region.start;
                    original_shape.end = region.end;
                    (same_manager_region(original, region)
                        && same_manager_region(&original_shape, region))
                    .then_some(*rebuilt)
                }));
                if transferred.len() == previous_len {
                    transferred.push(*region);
                }
            }
            certificate.manager_regions = transferred;
        }
        self.fallback_certificates
            .retain_mut(|certificate| certificate.rebind_copied_exit(retargets).is_some());
        // The complete array, owner, slot count, setups, and boundaries must still match.
        self.retain_pipeline(p);
    }

    /// Transfers original fallback proofs across the immediately preceding fusion pass.
    fn retain_after_fusion(&mut self, before: &[Item], p: &OptimizationPipeline) {
        if self.has_fusion_transfer_certificates() {
            let items = p.graph.copy_items();
            if let (Some(before), Some(after)) = (
                View::new(
                    before,
                    &p.exception_regions,
                    &p.exception_owner_provenance,
                    &p.preserved_block_boundaries,
                ),
                View::new(
                    &items,
                    &p.exception_regions,
                    &p.exception_owner_provenance,
                    &p.preserved_block_boundaries,
                ),
            ) {
                self.retain_fusion_views(&before, &after);
                return;
            }
        }
        self.retain_pipeline(p);
    }

    fn has_fusion_transfer_certificates(&self) -> bool {
        !self.fallback_certificates.is_empty() || !self.handler_dispatch_certificates.is_empty()
    }

    fn retain_fusion_views(&mut self, before: &View<'_>, after: &View<'_>) {
        self.fallback_certificates.retain_mut(|certificate| {
            certificate.valid(after, false).is_some()
                || certificate.rebind_one_fusion(before, after).is_some()
        });
        self.handler_dispatch_certificates
            .retain_mut(|certificate| {
                certificate.valid(after, false).is_some()
                    || certificate.rebind_one_fusion(before, after).is_some()
            });
        self.retain_view(after);
    }

    pub(super) fn has_manager_setup_certificates(&self) -> bool {
        !self.handler_dispatch_certificates.is_empty()
            || !self.manager_setup_certificates.is_empty()
            || self
                .fresh_singleton_certificates
                .iter()
                .any(|certificate| !certificate.manager_regions.is_empty())
    }

    /// Transfers exact source cells through the cold pass's recorded region splits.
    pub(super) fn retain_after_cold_regions(
        &mut self,
        p: &OptimizationPipeline,
        retargets: &[(ExceptionRegion, ExceptionRegion)],
    ) {
        if self.is_empty() {
            return;
        }
        let items = p.graph.copy_items();
        let Some(view) = View::new(
            &items,
            &p.exception_regions,
            &p.exception_owner_provenance,
            &p.preserved_block_boundaries,
        ) else {
            self.clear();
            return;
        };
        self.handler_dispatch_certificates
            .retain_mut(|certificate| certificate.rebind_cold_region(&view, retargets).is_some());
        for certificate in &mut self.fresh_singleton_certificates {
            let mut transferred = Vec::new();
            for region in &certificate.manager_regions {
                let previous_len = transferred.len();
                transferred.extend(retargets.iter().filter_map(|(original, rebuilt)| {
                    let mut original_shape = *rebuilt;
                    original_shape.start = region.start;
                    original_shape.end = region.end;
                    (same_manager_region(original, region)
                        && same_manager_region(&original_shape, region))
                    .then_some(*rebuilt)
                }));
                if transferred.len() == previous_len {
                    transferred.push(*region);
                }
            }
            certificate.manager_regions = transferred;
        }
        if !retargets.is_empty() && !self.manager_setup_certificates.is_empty() {
            self.manager_setup_certificates.retain_mut(|certificate| {
                let Some((start, _, marker)) = certificate.interval.range(&view, false) else {
                    return false;
                };
                if certificate
                    .interval
                    .matches_interval(&view, start, marker, false)
                    .is_none()
                {
                    return false;
                }
                let mut sources =
                    view.items[start..marker]
                        .iter()
                        .enumerate()
                        .filter_map(|(offset, item)| match item {
                            Item::Instruction(instruction)
                                if same_instruction(certificate.source, *instruction, false) =>
                            {
                                Some(start + offset)
                            }
                            _ => None,
                        });
                let Some(source) = sources.next() else {
                    return false;
                };
                if sources.next().is_some() {
                    return false;
                }
                let position = view.instruction_position(source);
                let mut candidates = retargets.iter().filter_map(|(original, rebuilt)| {
                    if !same_manager_region(original, &certificate.region) {
                        return None;
                    }
                    let start = *view.label_position(rebuilt.start)?;
                    let end = *view.label_position(rebuilt.end)?;
                    (start <= position && position < end).then_some(*rebuilt)
                });
                let Some(region) = candidates.next() else {
                    return false;
                };
                if candidates.next().is_some() {
                    return false;
                }
                // The pass explicitly preserves target, depth, lasti and priority;
                // only a recorded replacement containing this exact cell can transfer.
                certificate.region = region;
                certificate.regions = view.regions.to_vec();
                certificate.valid(&view, false).is_some()
            });
        }
        self.retain_view(&view);
    }

    fn is_empty(&self) -> bool {
        self.certificates.is_empty()
            && self.handler_dispatch_certificates.is_empty()
            && self.manager_setup_certificates.is_empty()
            && self.fallback_certificates.is_empty()
            && self.fresh_singleton_certificates.is_empty()
            && self.folded_assert_success_certificates.is_empty()
    }

    fn clear(&mut self) {
        self.certificates.clear();
        self.handler_dispatch_certificates.clear();
        self.manager_setup_certificates.clear();
        self.fallback_certificates.clear();
        self.fresh_singleton_certificates.clear();
        self.folded_assert_success_certificates.clear();
    }

    pub(super) fn retain_pipeline(&mut self, p: &OptimizationPipeline) {
        if self.is_empty() {
            return;
        }
        let mut scratch = self.scratch.take().unwrap_or_default();
        for (_, block) in p.graph.iter() {
            scratch.items.extend_from_slice(block.items());
        }
        let Some(view) = View::with_tables(
            &scratch.items,
            &p.exception_regions,
            &p.exception_owner_provenance,
            &p.preserved_block_boundaries,
            std::mem::take(&mut scratch.tables),
        ) else {
            self.clear();
            return;
        };
        self.retain_view(&view);
        if self.is_empty() {
            return;
        }
        scratch.tables = view.into_tables();
        // Keep only allocation capacity; every record is rebuilt by the next pass.
        scratch.tables.clear();
        scratch.items.clear();
        self.scratch = Some(scratch);
    }

    fn retain_view(&mut self, view: &View<'_>) {
        self.certificates.retain(|c| c.valid(view, false).is_some());
        self.handler_dispatch_certificates
            .retain(|c| c.valid(view, false).is_some());
        self.manager_setup_certificates
            .retain(|certificate| certificate.valid(view, false).is_some());
        self.fallback_certificates
            .retain(|c| c.valid(view, false).is_some());
        self.fresh_singleton_certificates
            .retain(|certificate| certificate.valid(view, false).is_some());
        self.folded_assert_success_certificates
            .retain(|certificate| certificate.valid(view).is_some());
    }
    /// Exports exact surviving manager cells before layout chooses owner regions.
    fn manager_owners(&self, view: &View<'_>) -> Vec<(Instruction, Instruction, ExceptionRegion)> {
        if self.manager_setup_certificates.is_empty() {
            return Vec::new();
        }
        self.manager_setup_certificates
            .iter()
            .filter_map(|certificate| {
                let (condition, marker) = certificate.valid(view, true)?;
                let (Item::Instruction(condition), Item::Instruction(marker)) =
                    (view.items[condition], view.items[marker])
                else {
                    return None;
                };
                Some((condition, marker, certificate.region))
            })
            .collect()
    }

    /// Validates manager exports and policy updates against the same unchanged items.
    pub(super) fn finalize(
        self,
        items: &mut [Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> Vec<(Instruction, Instruction, ExceptionRegion)> {
        if self.is_empty() {
            return Vec::new();
        }
        let Some(view) = View::new(items, regions, provenance, preserved) else {
            return Vec::new();
        };
        let manager_owners = self.manager_owners(&view);
        let changes = self
            .certificates
            .iter()
            .filter_map(|c| Some((c.valid(&view, true)?, c.owner)))
            .chain(
                self.fallback_certificates
                    .iter()
                    .filter_map(|c| c.valid(&view, true)),
            )
            .chain(
                self.fresh_singleton_certificates
                    .iter()
                    .filter_map(|certificate| Some((certificate.valid(&view, true)?, None))),
            )
            .chain(
                self.folded_assert_success_certificates
                    .iter()
                    .filter_map(|certificate| Some((certificate.valid(&view)?, None))),
            )
            .collect::<Vec<_>>();
        let dispatch_markers = self
            .handler_dispatch_certificates
            .iter()
            .filter(|certificate| certificate.removed == 1)
            .filter_map(|certificate| certificate.valid(&view, true))
            .collect::<Vec<_>>();
        for (index, owner) in changes {
            let Item::Instruction(marker) = &mut items[index] else {
                continue;
            };
            if owner.is_none() {
                marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            } else if marker.exception_policy() == ExceptionPolicy::AlwaysAbsentPostTryCondition {
                // Only the complete post-try array proof may retire this explicit exclusion.
                marker.set_exception_policy(ExceptionPolicy::NoneOverridePresent);
            } else {
                marker.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
            }
        }
        for index in dispatch_markers {
            if let Item::Instruction(marker) = &mut items[index] {
                // The exact original dispatch and every surviving record still select
                // this same cleanup owner. Retire only its predicted late-slot exclusion;
                // layout can therefore recover that owner from the preceding condition.
                marker.set_exception_policy(ExceptionPolicy::NoneOverridePresent);
            }
        }
        manager_owners
    }

    #[cfg(test)]
    fn apply(
        self,
        items: &mut [Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) {
        let _ = self.finalize(items, regions, provenance, preserved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::SourceLocation;
    use crate::target::opcodes::JUMP_FORWARD;

    fn instruction(op: crate::target::Opcode, arg: Operand) -> Instruction {
        Instruction::new(op, arg, SourceLocation::new(1, 1, 0, 1), Some(0))
    }
    fn pair() -> (Vec<Item>, Vec<Item>) {
        let a = instruction(LOAD_FAST, Operand::Value(2));
        let b = instruction(LOAD_FAST, Operand::Value(3));
        (
            vec![Item::Instruction(a), Item::Instruction(b)],
            vec![Item::Instruction(Instruction::fused(
                LOAD_FAST_LOAD_FAST,
                Operand::Value(35),
                a,
                b,
            ))],
        )
    }
    fn certificate() -> (Certificate, Vec<Item>) {
        let mut condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(Label::from_u32(1)));
        condition.exclude_from_exception_table(ExceptionExclusion::Always);
        let marker = instruction(NOT_TAKEN, Operand::Value(0));
        let items = vec![
            Item::Instruction(instruction(LOAD_FAST, Operand::Value(0))),
            Item::Instruction(condition),
            Item::Instruction(marker),
        ];
        let cert = Certificate {
            anchor: Anchor::Entry,
            condition,
            marker,
            items: items.clone(),
            labels: labels_in(&items),
            owner: None,
            boundaries: FxHashSet::default(),
            setups: Vec::new(),
        };
        (cert, items)
    }
    fn valid(cert: &Certificate, items: &[Item], borrow: bool) -> bool {
        let provenance = ExceptionOwnerProvenance::default();
        let preserved = FxHashSet::default();
        let view = View::new(items, &[], &provenance, &preserved).unwrap();
        cert.valid(&view, borrow).is_some()
    }

    #[test]
    fn interval_boundaries_match_whole_view_membership() {
        let boundaries = [
            Label::from_u32(1),
            Label::from_u32(3),
            Label::from_u32(u32::MAX - 1),
        ]
        .into_iter()
        .collect();
        let items = [
            Item::Label(Label::from_u32(4)),
            Item::Label(Label::from_u32(3)),
            Item::Instruction(instruction(
                JUMP_FORWARD,
                Operand::Forward(Label::from_u32(1)),
            )),
            Item::Label(Label::from_u32(u32::MAX - 1)),
            Item::Label(Label::from_u32(3)),
        ];
        assert_eq!(
            boundaries_in(&items, &boundaries),
            [Label::from_u32(3), Label::from_u32(u32::MAX - 1)]
                .into_iter()
                .collect()
        );
        assert!(boundaries_in(&items[..1], &boundaries).is_empty());
        assert!(boundaries_in(&items, &FxHashSet::default()).is_empty());
    }

    #[test]
    fn instruction_positions_count_only_preceding_instructions() {
        let provenance = ExceptionOwnerProvenance::default();
        let preserved = FxHashSet::default();
        let items = [
            Item::Label(Label::from_u32(0)),
            Item::Label(Label::from_u32(1)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(2)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(3)),
        ];
        let view = View::new(&items, &[], &provenance, &preserved).unwrap();
        let positions: Vec<_> = (0..=items.len())
            .map(|index| view.instruction_position(index))
            .collect();
        assert_eq!(positions, [0, 0, 0, 1, 1, 2, 3, 3]);
        assert_eq!(view.label_position(Label::from_u32(2)), Some(&1));

        let mut duplicate = items.to_vec();
        duplicate.push(Item::Label(Label::from_u32(2)));
        assert!(View::new(&duplicate, &[], &provenance, &preserved).is_none());
        assert_eq!(
            View::new(&[], &[], &provenance, &preserved)
                .unwrap()
                .instruction_position(0),
            0
        );
    }

    #[test]
    fn sparse_instruction_positions_match_prefix_counts_across_reused_tables() {
        let provenance = ExceptionOwnerProvenance::default();
        let preserved = FxHashSet::default();
        let mut tables = ViewTables::default();
        for mask in 0_u32..1024 {
            let items = (0..10)
                .map(|index| {
                    if mask & (1 << index) == 0 {
                        Item::Instruction(instruction(NOP, Operand::Value(0)))
                    } else {
                        Item::Label(Label::from_u32(index))
                    }
                })
                .collect::<Vec<_>>();
            let view = View::with_tables(&items, &[], &provenance, &preserved, tables).unwrap();
            let mut instruction_count = 0;
            for (index, item) in items.iter().enumerate() {
                assert_eq!(view.instruction_position(index), instruction_count);
                if matches!(item, Item::Instruction(_)) {
                    instruction_count += 1;
                }
            }
            assert_eq!(view.instruction_position(items.len()), instruction_count);
            tables = view.into_tables();
        }
        let empty = View::with_tables(&[], &[], &provenance, &preserved, tables).unwrap();
        assert_eq!(empty.instruction_position(0), 0);
    }

    #[test]
    fn marker_candidates_preserve_instruction_order_across_labels() {
        let provenance = ExceptionOwnerProvenance::default();
        let preserved = FxHashSet::default();
        let condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(Label::from_u32(99)));
        let marker = instruction(NOT_TAKEN, Operand::Value(0));
        let items = [
            Item::Instruction(marker),
            Item::Instruction(condition),
            Item::Label(Label::from_u32(1)),
            Item::Label(Label::from_u32(2)),
            Item::Instruction(marker),
            Item::Instruction(marker),
            Item::Instruction(condition),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Instruction(marker),
            Item::Instruction(condition),
            Item::Label(Label::from_u32(3)),
            Item::Instruction(marker),
            Item::Label(Label::from_u32(99)),
        ];
        let view = View::new(&items[..9], &[], &provenance, &preserved).unwrap();
        assert_eq!(view.endpoint(condition, marker), Some((1, 4)));

        let view = View::new(&items, &[], &provenance, &preserved).unwrap();
        assert_eq!(
            view.markers[&key(condition, marker).unwrap()].as_slice(),
            &[(1, 4), (9, 11)]
        );
        assert_eq!(view.endpoint(condition, marker), None);
    }

    #[test]
    fn label_positions_keep_dense_and_sparse_memberships_exact() {
        let mut labels = LabelPositions::default();
        labels.prepare(3);
        assert_eq!(labels.insert(Label::from_u32(0), (0, 0)), None);
        assert_eq!(labels.insert(Label::from_u32(5), (1, 0)), None);
        assert_eq!(labels.insert(Label::from_u32(90), (2, 1)), None);
        assert_eq!(labels.insert(Label::from_u32(u32::MAX - 1), (3, 1)), None);
        assert_eq!(labels.dense.len(), 6);
        assert_eq!(labels.get(Label::from_u32(0)), Some(&(0, 0)));
        assert_eq!(labels.get(Label::from_u32(5)), Some(&(1, 0)));
        assert_eq!(labels.get(Label::from_u32(90)), Some(&(2, 1)));
        assert_eq!(labels.get(Label::from_u32(u32::MAX - 1)), Some(&(3, 1)));
        assert_eq!(labels.get(Label::from_u32(4)), None);
        assert_eq!(labels.get(Label::from_u32(91)), None);
        assert_eq!(labels.insert(Label::from_u32(5), (4, 2)), Some((1, 0)));
        assert_eq!(labels.insert(Label::from_u32(90), (5, 2)), Some((2, 1)));
        assert_eq!(
            labels.insert(Label::from_u32(u32::MAX - 1), (6, 3)),
            Some((3, 1))
        );

        // The same label moves from sparse to dense, then back on a shorter graph.
        labels.clear();
        labels.prepare(100);
        assert_eq!(labels.get(Label::from_u32(5)), None);
        assert_eq!(labels.get(Label::from_u32(90)), None);
        assert_eq!(labels.get(Label::from_u32(u32::MAX - 1)), None);
        assert_eq!(labels.insert(Label::from_u32(90), (4, 2)), None);
        assert_eq!(labels.get(Label::from_u32(90)), Some(&(4, 2)));
        assert_eq!(labels.dense.len(), 91);
        labels.clear();
        labels.prepare(3);
        assert_eq!(labels.get(Label::from_u32(90)), None);
        assert_eq!(labels.insert(Label::from_u32(90), (1, 1)), None);
        assert_eq!(labels.get(Label::from_u32(90)), Some(&(1, 1)));
        assert_eq!(labels.dense.len(), 91);

        let provenance = ExceptionOwnerProvenance::default();
        let preserved = FxHashSet::default();
        let items = [
            Item::Label(Label::from_u32(u32::MAX - 1)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(1)),
        ];
        let view = View::new(&items, &[], &provenance, &preserved).unwrap();
        assert_eq!(view.label_index(Label::from_u32(u32::MAX - 1)), Some(&0));
        assert_eq!(view.label_position(Label::from_u32(1)), Some(&1));
        for duplicate in [Label::from_u32(1), Label::from_u32(u32::MAX - 1)] {
            let mut duplicate_items = items.to_vec();
            duplicate_items.push(Item::Label(duplicate));
            assert!(View::new(&duplicate_items, &[], &provenance, &preserved).is_none());
        }
    }
    #[test]
    fn label_position_generation_wrap_cannot_revive_stale_records() {
        let mut labels = LabelPositions::default();
        labels.prepare(3);
        assert_eq!(labels.insert(Label::from_u32(1), (1, 1)), None);
        assert_eq!(labels.insert(Label::from_u32(u32::MAX - 1), (9, 9)), None);
        // Simulate a record left from the generation to which a wrap returns.
        labels.dense[1].generation = 1;
        labels.generation = u32::MAX;
        assert_eq!(labels.insert(Label::from_u32(2), (2, 2)), None);
        labels.clear();
        assert_eq!(labels.generation, 1);
        assert_eq!(labels.get(Label::from_u32(1)), None);
        assert_eq!(labels.get(Label::from_u32(2)), None);
        assert_eq!(labels.get(Label::from_u32(u32::MAX - 1)), None);
        assert_eq!(labels.insert(Label::from_u32(1), (3, 3)), None);
        assert_eq!(labels.get(Label::from_u32(1)), Some(&(3, 3)));
        assert_eq!(labels.insert(Label::from_u32(1), (4, 4)), Some((3, 3)));
    }
    #[test]
    fn references_include_control_flow_and_supplementary_provenance() {
        let mut provenance = ExceptionOwnerProvenance::default();
        provenance.source_boundaries.insert(Label::from_u32(2));
        let preserved = FxHashSet::from_iter([Label::from_u32(1)]);
        let items = [Item::Instruction(instruction(
            JUMP_FORWARD,
            Operand::Forward(Label::from_u32(3)),
        ))];
        let view = View::new(&items, &[], &provenance, &preserved).unwrap();
        assert!(view.is_reference(Label::from_u32(1)));
        assert!(view.is_reference(Label::from_u32(3)));
        assert!(view.is_reference(Label::from_u32(2)));
        assert!(!view.is_reference(Label::from_u32(4)));
    }
    #[test]
    fn reused_view_tables_rebuild_all_boundary_memberships() {
        let sparse = Label::from_u32(u32::MAX - 1);
        let new_sparse = Label::from_u32(u32::MAX - 2);
        let make_regions = |target| {
            [ExceptionRegion {
                start: Label::from_u32(10),
                end: Label::from_u32(20),
                target,
                depth: 0,
                preserve_lasti: false,
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::Resolved { start: 0, end: 1 },
            }]
        };
        let make_items = |target, duplicate| {
            [
                Item::Label(Label::from_u32(1)),
                Item::Instruction(instruction(JUMP_FORWARD, Operand::Forward(target))),
                Item::Label(sparse),
                Item::Instruction(instruction(JUMP_FORWARD, Operand::Forward(duplicate))),
            ]
        };
        let mut first_provenance = ExceptionOwnerProvenance::default();
        first_provenance
            .block_boundaries
            .extend([Label::from_u32(1), sparse]);
        first_provenance
            .source_boundaries
            .insert(Label::from_u32(5));
        let first_preserved = FxHashSet::from_iter([Label::from_u32(2), sparse]);
        let first_items = make_items(Label::from_u32(3), sparse);
        let first_regions = make_regions(Label::from_u32(4));
        let first = View::new(
            &first_items,
            &first_regions,
            &first_provenance,
            &first_preserved,
        )
        .unwrap();
        assert_eq!(
            first.boundaries,
            FxHashSet::from_iter([
                Label::from_u32(1),
                Label::from_u32(2),
                Label::from_u32(3),
                Label::from_u32(4),
                sparse
            ])
        );
        assert!(first.is_reference(Label::from_u32(5)));

        let mut next_provenance = ExceptionOwnerProvenance::default();
        next_provenance
            .block_boundaries
            .extend([Label::from_u32(11), new_sparse]);
        next_provenance.source_boundaries.insert(Label::from_u32(6));
        let next_preserved = FxHashSet::from_iter([Label::from_u32(12), new_sparse]);
        let next_items = make_items(Label::from_u32(13), new_sparse);
        let next_regions = make_regions(new_sparse);
        let next = View::with_tables(
            &next_items,
            &next_regions,
            &next_provenance,
            &next_preserved,
            first.into_tables(),
        )
        .unwrap();
        assert_eq!(
            next.boundaries,
            FxHashSet::from_iter([
                Label::from_u32(11),
                Label::from_u32(12),
                Label::from_u32(13),
                new_sparse
            ])
        );
        assert_eq!(next.label_index(sparse), Some(&2));
        assert!(!next.is_reference(sparse));
        assert!(!next.is_reference(Label::from_u32(5)));
        assert!(next.is_reference(Label::from_u32(6)));

        let mut tables = next.into_tables();
        tables.clear();
        let empty_provenance = ExceptionOwnerProvenance::default();
        let empty_preserved = FxHashSet::default();
        let empty =
            View::with_tables(&[], &[], &empty_provenance, &empty_preserved, tables).unwrap();
        assert!(empty.boundaries.is_empty());
        assert!(!empty.is_reference(new_sparse));

        for duplicate in [Label::from_u32(1), sparse] {
            let clean = View::new(&first_items, &[], &empty_provenance, &empty_preserved).unwrap();
            let mut duplicate_items = first_items.to_vec();
            duplicate_items.push(Item::Label(duplicate));
            assert!(
                View::with_tables(
                    &duplicate_items,
                    &[],
                    &empty_provenance,
                    &empty_preserved,
                    clean.into_tables(),
                )
                .is_none()
            );
        }
    }
    #[test]
    fn handler_interval_owner_matches_individual_cells_and_preserves_failures() {
        use ExceptionRegionPriority::{OriginalRange, Resolved};

        let provenance = ExceptionOwnerProvenance::default();
        let preserved = FxHashSet::default();
        let items = [
            Item::Label(Label::from_u32(10)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(100)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(500)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(1_000)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(50_000)),
            Item::Instruction(instruction(NOP, Operand::Value(0))),
            Item::Label(Label::from_u32(1_000_000)),
        ];
        let make_region = |start, end, target, priority| ExceptionRegion {
            start: Label::from_u32(start),
            end: Label::from_u32(end),
            target: Label::from_u32(target),
            depth: 1,
            preserve_lasti: true,
            is_scope_fallback: false,
            priority,
        };
        let assert_interval =
            |regions: &[ExceptionRegion], expected: &ExceptionRegion, first, end, wanted| {
                let view = View::new(&items, regions, &provenance, &preserved).unwrap();
                let individual_cells = (first..end).all(|position| {
                    HandlerDispatchCertificate::owner_at(&view, position)
                        .is_some_and(|owner| same_manager_region(&owner, expected))
                });
                assert_eq!(individual_cells, wanted);
                assert_eq!(
                    HandlerDispatchCertificate::owns_interval(&view, first, end, expected)
                        .is_some(),
                    wanted
                );
            };
        let base = make_region(10, 1_000_000, 10, Resolved { start: 0, end: 5 });
        let better = make_region(100, 1_000, 100, Resolved { start: 4, end: 5 });
        let lower = make_region(100, 1_000, 100, Resolved { start: 0, end: 8 });
        let tied = make_region(100, 1_000, 100, Resolved { start: 0, end: 5 });
        let mut fallback = better;
        fallback.is_scope_fallback = true;
        assert_interval(&[base, better], &base, 0, 5, false);
        assert_interval(&[base, better], &base, 3, 5, true);
        assert_interval(&[base, better], &better, 1, 3, true);
        assert_interval(&[base, better], &better, 0, 4, false);
        assert_interval(&[base, lower], &base, 0, 5, true);
        assert_interval(&[fallback, base], &base, 0, 5, true);
        assert_interval(&[base, tied], &base, 0, 5, true);
        assert_interval(&[tied, base], &base, 0, 5, false);
        assert_interval(&[base, base], &base, 0, 5, true);
        assert_interval(&[], &base, 0, 5, false);

        let missing_priority = make_region(
            1_000,
            50_000,
            100,
            OriginalRange(Label::from_u32(u32::MAX - 1), Label::from_u32(u32::MAX - 2)),
        );
        let missing_bounds =
            make_region(u32::MAX - 1, 1_000_000, 100, Resolved { start: 4, end: 5 });
        assert_interval(&[base, missing_priority], &base, 0, 3, true);
        assert_interval(&[base, missing_priority], &base, 0, 4, false);
        assert_interval(&[base, missing_bounds], &base, 0, 1, false);
        assert_interval(&[missing_bounds, base], &base, 0, 1, false);
        assert_interval(&[missing_bounds], &base, 2, 2, true);
        assert_interval(&[], &base, 2, 2, true);
        assert_interval(&[], &base, 3, 2, true);

        let mut inverted = missing_priority;
        inverted.start = Label::from_u32(1_000);
        inverted.end = Label::from_u32(100);
        let mut empty = missing_priority;
        empty.start = Label::from_u32(500);
        empty.end = Label::from_u32(500);
        assert_interval(&[inverted, base, empty], &base, 0, 5, true);
    }
    #[test]
    fn captures_one_actual_fusion() {
        let (a, b) = pair();
        assert!(one_fusion(&a, &b));
    }

    #[test]
    fn captures_fusion_after_only_the_immediately_preceding_marker_pair() {
        let first_target = Label::from_u32(1);
        let second_target = Label::from_u32(2);
        let bookkeeping = Item::Label(Label::from_u32(3));
        let first_condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(first_target));
        let second_condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(second_target));
        let marker = instruction(NOT_TAKEN, Operand::Value(0));
        let nop = Item::Instruction(instruction(NOP, Operand::Value(0)));
        let provenance = ExceptionOwnerProvenance::default();
        let preserved = FxHashSet::default();

        for (separator, captured) in [(bookkeeping, true), (nop, false)] {
            let make_items = |fusing: Vec<Item>| {
                let mut items = vec![nop; 64];
                items.extend([
                    Item::Instruction(first_condition),
                    separator,
                    Item::Instruction(marker),
                ]);
                items.extend(fusing);
                items.extend([
                    Item::Instruction(second_condition),
                    Item::Instruction(marker),
                    Item::Label(first_target),
                    Item::Label(second_target),
                ]);
                items
            };
            let (original, fused) = pair();
            let before_items = make_items(original);
            let after_items = make_items(fused);
            let before = View::new(&before_items, &[], &provenance, &preserved).unwrap();
            let after = View::new(&after_items, &[], &provenance, &preserved).unwrap();
            let certificates = FusionOwnerCertificates::capture_views(&before, &after);
            if captured {
                let [certificate] = certificates.certificates.as_slice() else {
                    panic!("exactly one fusion should be captured");
                };
                assert!(
                    matches!(certificate.anchor, Anchor::Marker(condition, actual_marker)
                        if condition == first_condition && actual_marker == marker)
                );
            } else {
                assert!(certificates.certificates.is_empty());
            }
        }
    }

    #[test]
    fn rejects_two_actual_fusions() {
        let (mut a, mut b) = pair();
        let (c, d) = pair();
        a.extend(c);
        b.extend(d);
        assert!(!one_fusion(&a, &b));
    }
    #[test]
    fn rejects_changed_fused_argument() {
        let (a, mut b) = pair();
        let Item::Instruction(i) = &mut b[0] else {
            unreachable!()
        };
        i.set_operand(Operand::Value(34));
        assert!(!one_fusion(&a, &b));
    }
    #[test]
    fn permits_unused_bookkeeping_label() {
        let (c, mut items) = certificate();
        items.insert(2, Item::Label(Label::from_u32(2)));
        assert!(valid(&c, &items, false));
    }
    #[test]
    fn rejects_referenced_added_label() {
        let (c, mut items) = certificate();
        items.insert(2, Item::Label(Label::from_u32(2)));
        items.push(Item::Instruction(instruction(
            JUMP_FORWARD,
            Operand::Forward(Label::from_u32(2)),
        )));
        assert!(!valid(&c, &items, false));
    }
    #[test]
    fn rejects_later_instruction_insertion() {
        let (c, mut items) = certificate();
        items.insert(2, Item::Instruction(instruction(NOP, Operand::Value(0))));
        assert!(!valid(&c, &items, false));
    }
    #[test]
    fn rejects_later_terminal_retarget() {
        let (c, mut items) = certificate();
        let Item::Instruction(i) = &mut items[1] else {
            unreachable!()
        };
        i.retarget(Operand::Forward(Label::from_u32(3)));
        assert!(!valid(&c, &items, false));
    }

    #[test]
    fn instruction_borrowing_is_directional_and_ignores_only_initial_jump_borrowing() {
        for (owned, borrowed) in [
            (LOAD_FAST, LOAD_FAST_BORROW),
            (LOAD_FAST_LOAD_FAST, LOAD_FAST_BORROW_LOAD_FAST_BORROW),
        ] {
            let original = instruction(owned, Operand::Value(2));
            let changed = instruction(borrowed, Operand::Value(2));
            assert!(same_instruction(original, changed, true));
            assert!(!same_instruction(original, changed, false));
            assert!(!same_instruction(changed, original, true));
            assert!(!same_instruction(
                original,
                instruction(NOP, Operand::Value(2)),
                true
            ));
            assert!(!same_instruction(
                original,
                instruction(borrowed, Operand::Value(3)),
                true
            ));
        }
        assert!(!same_instruction(
            instruction(LOAD_FAST, Operand::Value(2)),
            instruction(LOAD_FAST_BORROW_LOAD_FAST_BORROW, Operand::Value(2)),
            true,
        ));

        let original = instruction(JUMP_FORWARD, Operand::Forward(Label::from_u32(4)));
        let mut tagged = original;
        tagged.mark_initial_borrow_jump_target(Label::from_u32(4));
        assert!(same_instruction(original, tagged, false));
        assert!(same_instruction(tagged, original, false));
        tagged.set_exception_policy(ExceptionPolicy::NonePresent);
        assert!(!same_instruction(original, tagged, false));
    }

    #[test]
    fn borrowing_requires_exact_remaining_fields() {
        let (c, mut items) = certificate();
        let Item::Instruction(i) = &mut items[0] else {
            unreachable!()
        };
        i.opcode = LOAD_FAST_BORROW;
        assert!(!valid(&c, &items, false));
        assert!(valid(&c, &items, true));
        let Item::Instruction(i) = &mut items[0] else {
            unreachable!()
        };
        i.set_operand(Operand::Value(1));
        assert!(!valid(&c, &items, true));
    }
    #[test]
    fn preserves_terminal_exclusion() {
        let (c, mut items) = certificate();
        let Item::Instruction(i) = &mut items[1] else {
            unreachable!()
        };
        i.set_exception_policy(ExceptionPolicy::default());
        assert!(!valid(&c, &items, false));
    }
    fn protected_pipeline(excluded: bool, owner: NormalizedExceptionOwner) -> OptimizationPipeline {
        use crate::assembler::{Assembler, UnoptimizedCode};
        use crate::target::opcodes::RETURN_VALUE;
        let mut condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(Label::from_u32(2)));
        if excluded {
            condition.exclude_from_exception_table(ExceptionExclusion::Always);
        }
        let mut marker = instruction(NOT_TAKEN, Operand::Value(0));
        marker.set_exception_owner(owner);
        let items = vec![
            Item::Label(Label::from_u32(0)),
            Item::Instruction(instruction(LOAD_FAST, Operand::Value(2))),
            Item::Instruction(instruction(LOAD_FAST, Operand::Value(3))),
            Item::Instruction(condition),
            Item::Instruction(marker),
            Item::Label(Label::from_u32(1)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label::from_u32(2)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label::from_u32(3)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
        ];
        Assembler {
            code: UnoptimizedCode::Linear(items),
            exception_regions: vec![ExceptionRegion {
                start: Label::from_u32(0),
                end: Label::from_u32(1),
                target: Label::from_u32(3),
                depth: 0,
                preserve_lasti: true,
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(
                    Label::from_u32(0),
                    Label::from_u32(1),
                ),
            }],
            next_label: 4,
            ..Assembler::default()
        }
        .into_optimization_pipeline()
    }
    fn post_try_fusion_pipeline() -> OptimizationPipeline {
        let mut pipeline = protected_pipeline(false, NormalizedExceptionOwner::Absent);
        pipeline.exception_regions[0].is_scope_fallback = true;
        pipeline
            .exception_owner_provenance
            .block_boundaries
            .insert(Label::from_u32(0));
        pipeline
            .exception_owner_provenance
            .source_boundaries
            .insert(Label::from_u32(0));
        edit_singleton(&mut pipeline, |items| {
            for item in items {
                if let Item::Instruction(marker) = item
                    && marker.opcode == NOT_TAKEN
                {
                    marker.set_exception_policy(ExceptionPolicy::AlwaysAbsentPostTryCondition);
                }
            }
        });
        pipeline
    }

    fn compare_fusion_snapshot_reuse(
        mut pipeline: OptimizationPipeline,
        changed: bool,
        has_fallback: bool,
        has_manager: bool,
    ) {
        fn counts(certificates: &FusionOwnerCertificates) -> [usize; 6] {
            [
                certificates.certificates.len(),
                certificates.handler_dispatch_certificates.len(),
                certificates.manager_setup_certificates.len(),
                certificates.fallback_certificates.len(),
                certificates.fresh_singleton_certificates.len(),
                certificates.folded_assert_success_certificates.len(),
            ]
        }

        let original_fast = FusionOwnerCertificates::capture_original_owners(&pipeline);
        let original_general = FusionOwnerCertificates::capture_original_owners(&pipeline);
        let before = FusionOwnerCertificates::snapshot(&pipeline);
        assert!(!before.is_empty());
        let before_len = before.len();
        let actual_changed = pipeline.fuse_superinstructions();
        assert_eq!(pipeline.graph.item_count() < before_len, changed);
        assert_eq!(actual_changed, changed);
        let fast = original_fast.after_fusion(before.clone(), &pipeline, changed);
        let general =
            original_general.after_fusion_with_snapshot_reuse::<false>(before, &pipeline, changed);
        assert_eq!(counts(&fast), counts(&general));
        assert_eq!(!fast.fallback_certificates.is_empty(), has_fallback);
        assert_eq!(!fast.manager_setup_certificates.is_empty(), has_manager);
        assert_eq!(fast.scratch.is_some(), general.scratch.is_some());
        if let Some(scratch) = fast.scratch.as_ref() {
            assert!(scratch.items.capacity() >= pipeline.graph.item_count());
        }

        let mut fast_items = pipeline.graph.copy_items();
        let mut general_items = fast_items.clone();
        let fast_owners = fast.finalize(
            &mut fast_items,
            &pipeline.exception_regions,
            &pipeline.exception_owner_provenance,
            &pipeline.preserved_block_boundaries,
        );
        let general_owners = general.finalize(
            &mut general_items,
            &pipeline.exception_regions,
            &pipeline.exception_owner_provenance,
            &pipeline.preserved_block_boundaries,
        );
        assert_eq!(fast_items, general_items);
        assert_eq!(fast_owners.len(), general_owners.len());
        for ((fast_condition, fast_marker, fast_owner), (condition, marker, owner)) in
            fast_owners.into_iter().zip(general_owners)
        {
            assert_eq!(fast_condition, condition);
            assert_eq!(fast_marker, marker);
            assert!(same_manager_region(&fast_owner, &owner));
        }
    }

    #[test]
    fn unchanged_fusion_snapshot_still_captures_fallback_and_manager_owners() {
        compare_fusion_snapshot_reuse(
            fallback_pipeline(1, ExceptionPolicy::NoneAbsent),
            false,
            true,
            false,
        );

        let mut manager = fallback_pipeline(1, ExceptionPolicy::NoneAbsent);
        let start = Label::from_u32(0);
        manager.exception_regions[0].is_scope_fallback = false;
        manager.exception_owner_provenance.block_boundaries.clear();
        manager
            .exception_owner_provenance
            .setups
            .push((start, None));
        manager
            .exception_owner_provenance
            .synchronous_with_setups
            .insert(start);
        compare_fusion_snapshot_reuse(manager, false, false, true);

        compare_fusion_snapshot_reuse(
            singleton_pipeline(true, false, ExceptionPolicy::NoneUnchangedBooleanSingleton),
            false,
            false,
            false,
        );
    }

    #[test]
    fn changed_fusion_snapshot_keeps_the_original_and_fused_owner_views() {
        compare_fusion_snapshot_reuse(post_try_fusion_pipeline(), true, true, false);
    }

    #[test]
    fn post_try_local_fusion_reuses_only_its_original_fallback_slot() {
        let mut pipeline = post_try_fusion_pipeline();
        let before = FusionOwnerCertificates::snapshot(&pipeline);
        pipeline.fuse_superinstructions();
        let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
        assert_eq!(certificates.fallback_certificates.len(), 1);
        let mut items = pipeline.graph.into_items();
        let mut expected = items.clone();
        for item in &mut expected {
            if let Item::Instruction(marker) = item
                && marker.opcode == NOT_TAKEN
            {
                marker.set_exception_policy(ExceptionPolicy::NoneOverridePresent);
            }
        }
        certificates.apply(
            &mut items,
            &pipeline.exception_regions,
            &pipeline.exception_owner_provenance,
            &pipeline.preserved_block_boundaries,
        );
        assert_eq!(items, expected);
    }

    fn post_try_literal_fold_pipeline() -> OptimizationPipeline {
        let mut pipeline = post_try_fusion_pipeline();
        let mut constant = instruction(crate::target::opcodes::LOAD_CONST, Operand::Value(0));
        constant.location = SourceLocation::new(2, 2, 10, 20);
        constant.local_policy.add_hidden_exception_owner_slots(2);
        constant.add_pre_fold_extra_instructions(2);
        edit_singleton(&mut pipeline, |items| {
            items[2] = Item::Instruction(constant);
        });
        pipeline
            .exception_owner_provenance
            .uniform_fold_owner_slots
            .push((constant.location, constant.opcode, 2));
        pipeline
    }

    #[test]
    fn post_try_literal_fold_reuses_only_its_certified_original_slot() {
        let pipeline = post_try_literal_fold_pipeline();
        let before = FusionOwnerCertificates::snapshot(&pipeline);
        let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
        assert_eq!(certificates.fallback_certificates.len(), 1);
        let mut items = before.clone();
        certificates.apply(
            &mut items,
            &pipeline.exception_regions,
            &pipeline.exception_owner_provenance,
            &pipeline.preserved_block_boundaries,
        );
        let mut expected = before;
        for item in &mut expected {
            if let Item::Instruction(marker) = item
                && marker.opcode == NOT_TAKEN
            {
                marker.set_exception_policy(ExceptionPolicy::NoneOverridePresent);
            }
        }
        assert_eq!(items, expected);
    }

    #[test]
    fn post_try_literal_fold_rejects_missing_or_changed_provenance() {
        for change in 0..4 {
            let mut pipeline = post_try_literal_fold_pipeline();
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            match change {
                0 => pipeline
                    .exception_owner_provenance
                    .uniform_fold_owner_slots
                    .clear(),
                1 => pipeline.exception_owner_provenance.uniform_fold_owner_slots[0].2 = 1,
                2 => edit_singleton(&mut pipeline, |items| {
                    let Item::Instruction(constant) = &mut items[2] else {
                        unreachable!()
                    };
                    constant.set_operand(Operand::Value(1));
                }),
                3 => pipeline.exception_regions[0].is_scope_fallback = false,
                _ => unreachable!(),
            }
            let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
            assert!(
                certificates.fallback_certificates.is_empty(),
                "case {change}"
            );
        }
    }

    #[test]
    fn post_try_fusion_preserves_fresh_and_uncertified_markers() {
        for change in 0..12 {
            let mut pipeline = post_try_fusion_pipeline();
            match change {
                0 | 1 => edit_singleton(&mut pipeline, |items| {
                    for item in items {
                        if let Item::Instruction(marker) = item
                            && marker.opcode == NOT_TAKEN
                        {
                            marker.set_exception_policy(if change == 0 {
                                ExceptionPolicy::AlwaysAbsent
                            } else {
                                ExceptionPolicy::EXCLUDE_ALWAYS
                            });
                        }
                    }
                }),
                2 => pipeline
                    .exception_owner_provenance
                    .source_boundaries
                    .clear(),
                3 => pipeline.exception_owner_provenance.block_boundaries.clear(),
                4 => pipeline.exception_regions[0].depth = 1,
                5 => pipeline.exception_regions[0].is_scope_fallback = false,
                6 => pipeline
                    .exception_regions
                    .push(pipeline.exception_regions[0]),
                7 => pipeline
                    .exception_owner_provenance
                    .setups
                    .push((Label::from_u32(0), None)),
                8 => {
                    edit_singleton(&mut pipeline, |items| {
                        let marker = items
                            .iter()
                            .position(|item| {
                                matches!(item,
                            Item::Instruction(i) if i.opcode == NOT_TAKEN)
                            })
                            .unwrap();
                        items.insert(marker, Item::Label(Label::from_u32(4)));
                    });
                    pipeline
                        .exception_owner_provenance
                        .block_boundaries
                        .insert(Label::from_u32(4));
                }
                9 => edit_singleton(&mut pipeline, |items| {
                    let Item::Instruction(first) = &mut items[1] else {
                        unreachable!()
                    };
                    first.opcode = STORE_FAST;
                }),
                10 => edit_singleton(&mut pipeline, |items| {
                    let (second_pair, _) = pair();
                    items.splice(1..1, second_pair);
                }),
                11 => {}
                _ => unreachable!(),
            }
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            if change != 11 {
                pipeline.fuse_superinstructions();
            }
            let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
            assert!(
                certificates.fallback_certificates.is_empty(),
                "case {change}"
            );
        }
    }

    #[test]
    fn post_try_fusion_invalidates_later_owner_boundary_and_instruction_changes() {
        for change in 0..4 {
            let mut pipeline = post_try_fusion_pipeline();
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            pipeline.fuse_superinstructions();
            let mut certificates = FusionOwnerCertificates::capture(&before, &pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            match change {
                0 => pipeline.exception_regions[0].depth = 1,
                1 => pipeline
                    .exception_owner_provenance
                    .source_boundaries
                    .clear(),
                2 => edit_singleton(&mut pipeline, |items| {
                    let marker = items
                        .iter()
                        .position(|item| {
                            matches!(item,
                        Item::Instruction(i) if i.opcode == NOT_TAKEN)
                        })
                        .unwrap();
                    items.insert(
                        marker,
                        Item::Instruction(instruction(NOP, Operand::Value(0))),
                    );
                }),
                3 => edit_singleton(&mut pipeline, |items| {
                    let Item::Instruction(fused) = &mut items[1] else {
                        unreachable!()
                    };
                    fused.set_operand(Operand::Value(34));
                }),
                _ => unreachable!(),
            }
            certificates.retain_pipeline(&pipeline);
            assert!(
                certificates.fallback_certificates.is_empty(),
                "case {change}"
            );
        }
    }

    #[test]
    fn observed_fusion_transports_null_and_protected_terminal_owner() {
        for (excluded, owner) in [
            (true, NormalizedExceptionOwner::DynamicConditionStrict),
            (true, NormalizedExceptionOwner::DynamicCondition),
            (false, NormalizedExceptionOwner::Unchanged),
        ] {
            let mut pipeline = protected_pipeline(excluded, owner);
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            pipeline.fuse_superinstructions();
            let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
            assert_eq!(certificates.certificates.len(), 1);
            let mut items = pipeline.graph.into_items();
            certificates.apply(
                &mut items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &pipeline.preserved_block_boundaries,
            );
            let marker = items
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(ins) if ins.opcode == NOT_TAKEN => Some(ins),
                    Item::Instruction(_) | Item::Label(_) => None,
                })
                .unwrap();
            assert_eq!(
                marker.exception_policy(),
                if excluded {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::NoneOverridePresent
                }
            );
        }
    }
    #[test]
    fn preserves_explicit_marker_owner_policies() {
        for owner in [
            NormalizedExceptionOwner::Present,
            NormalizedExceptionOwner::OverridePresent,
            NormalizedExceptionOwner::Absent,
            NormalizedExceptionOwner::Dynamic,
        ] {
            for excluded in [true, false] {
                let mut pipeline = protected_pipeline(excluded, owner);
                let before = FusionOwnerCertificates::snapshot(&pipeline);
                pipeline.fuse_superinstructions();
                let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
                assert!(certificates.certificates.is_empty());
                let mut items = pipeline.graph.into_items();
                let unchanged = items.clone();
                certificates.apply(
                    &mut items,
                    &pipeline.exception_regions,
                    &pipeline.exception_owner_provenance,
                    &pipeline.preserved_block_boundaries,
                );
                assert_eq!(items.len(), unchanged.len());
                assert!(
                    items
                        .iter()
                        .copied()
                        .zip(unchanged)
                        .all(|(a, b)| same_item(a, b, false))
                );
            }
        }
    }
    #[test]
    fn rejects_covered_cleanup_stack_before_terminal_exclusion() {
        for depth in [1, 2] {
            for fallback in [false, true] {
                for excluded in [false, true] {
                    for owner in [
                        NormalizedExceptionOwner::Unchanged,
                        NormalizedExceptionOwner::DynamicCondition,
                        NormalizedExceptionOwner::DynamicConditionStrict,
                    ] {
                        let mut pipeline = protected_pipeline(excluded, owner);
                        pipeline.exception_regions[0].depth = depth;
                        pipeline.exception_regions[0].is_scope_fallback = fallback;
                        let before = FusionOwnerCertificates::snapshot(&pipeline);
                        pipeline.fuse_superinstructions();
                        let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
                        assert!(certificates.certificates.is_empty());
                    }
                }
            }
        }
    }
    #[test]
    fn permits_noncovering_cleanup_region() {
        let mut pipeline =
            protected_pipeline(true, NormalizedExceptionOwner::DynamicConditionStrict);
        pipeline.exception_regions.push(ExceptionRegion {
            start: Label::from_u32(2),
            end: Label::from_u32(3),
            target: Label::from_u32(3),
            depth: 2,
            preserve_lasti: true,
            is_scope_fallback: true,
            priority: ExceptionRegionPriority::OriginalRange(
                Label::from_u32(2),
                Label::from_u32(3),
            ),
        });
        let before = FusionOwnerCertificates::snapshot(&pipeline);
        pipeline.fuse_superinstructions();
        let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
        assert_eq!(certificates.certificates.len(), 1);
    }
    #[test]
    fn changed_handler_depth_invalidates_observed_fusion() {
        let mut pipeline = protected_pipeline(false, NormalizedExceptionOwner::Unchanged);
        let before = FusionOwnerCertificates::snapshot(&pipeline);
        pipeline.fuse_superinstructions();
        let mut certificates = FusionOwnerCertificates::capture(&before, &pipeline);
        assert_eq!(certificates.certificates.len(), 1);
        pipeline.exception_regions[0].depth += 1;
        certificates.retain_pipeline(&pipeline);
        assert!(certificates.certificates.is_empty());
    }
    fn singleton_pipeline(
        fallthrough: bool,
        unpack: bool,
        policy: ExceptionPolicy,
    ) -> OptimizationPipeline {
        use crate::assembler::{Assembler, UnoptimizedCode};
        use crate::target::opcodes::{IS_OP, LOAD_CONST, RETURN_VALUE, UNPACK_SEQUENCE};
        let mut items = vec![Item::Label(Label::from_u32(0))];
        if fallthrough {
            items.push(Item::Instruction(instruction(LOAD_FAST, Operand::Value(0))));
            items.push(Item::Instruction(instruction(
                POP_JUMP_IF_FALSE,
                Operand::Forward(Label::from_u32(2)),
            )));
            let mut marker = instruction(NOT_TAKEN, Operand::Value(0));
            marker.set_exception_policy(ExceptionPolicy::NoneDynamicConditionStrict);
            items.push(Item::Instruction(marker));
        }
        if unpack {
            items.push(Item::Instruction(instruction(
                UNPACK_SEQUENCE,
                Operand::Value(2),
            )));
        }
        for (op, arg) in [
            (LOAD_CONST, Operand::Value(1)),
            (IS_OP, Operand::Value(0)),
            (POP_JUMP_IF_FALSE, Operand::Forward(Label::from_u32(2))),
            (NOT_TAKEN, Operand::Value(0)),
        ] {
            let mut item = instruction(op, arg);
            item.location = SourceLocation::new(2, 2, 1, 5);
            if op == NOT_TAKEN {
                item.set_exception_policy(policy);
            }
            items.push(Item::Instruction(item));
        }
        for label in [Label::from_u32(1), Label::from_u32(2), Label::from_u32(3)] {
            items.push(Item::Label(label));
            items.push(Item::Instruction(instruction(
                RETURN_VALUE,
                Operand::Value(0),
            )));
        }
        let mut p = Assembler {
            code: UnoptimizedCode::Linear(items),
            exception_regions: vec![ExceptionRegion {
                start: Label::from_u32(0),
                end: Label::from_u32(1),
                target: Label::from_u32(3),
                depth: 0,
                preserve_lasti: true,
                is_scope_fallback: true,
                priority: ExceptionRegionPriority::OriginalRange(
                    Label::from_u32(0),
                    Label::from_u32(1),
                ),
            }],
            next_label: 4,
            ..Assembler::default()
        }
        .into_optimization_pipeline();
        p.exception_owner_provenance
            .block_boundaries
            .insert(Label::from_u32(0));
        p
    }

    fn edit_singleton(p: &mut OptimizationPipeline, edit: impl FnOnce(&mut Vec<Item>)) {
        let mut items = p.graph.iter_items().copied().collect::<Vec<_>>();
        edit(&mut items);
        p.graph =
            crate::assembler::block_graph::BlockGraph::from_items(items, &FxHashSet::default());
    }

    #[test]
    fn singleton_original_array_without_compaction_uses_fresh_null_slot() {
        for fallthrough in [false, true] {
            for unpack in [false, true] {
                let mut p = singleton_pipeline(
                    fallthrough,
                    unpack,
                    ExceptionPolicy::NoneUnchangedBooleanSingleton,
                );
                let mut cert = FusionOwnerCertificates::capture_original_owners(&p);
                assert_eq!(cert.fresh_singleton_certificates.len(), 1);
                p.fuse_superinstructions();
                cert.retain_pipeline(&p);
                let mut items = p.graph.into_items();
                cert.apply(
                    &mut items,
                    &p.exception_regions,
                    &p.exception_owner_provenance,
                    &p.preserved_block_boundaries,
                );
                let marker = items
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(i) if i.opcode == NOT_TAKEN && i.location.line == 2 => {
                            Some(i)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(marker.exception_policy(), ExceptionPolicy::EXCLUDE_ALWAYS);
            }
        }
    }

    #[test]
    fn singleton_origin_does_not_override_any_existing_owner_policy() {
        for policy in [
            ExceptionPolicy::NoneUnchanged,
            ExceptionPolicy::NonePresent,
            ExceptionPolicy::NoneAbsent,
            ExceptionPolicy::NoneOverridePresent,
            ExceptionPolicy::NoneDynamic,
            ExceptionPolicy::NoneDynamicCondition,
            ExceptionPolicy::NoneDynamicConditionStrict,
            ExceptionPolicy::AlwaysUnchanged,
            ExceptionPolicy::AlwaysOverridePresent,
            ExceptionPolicy::IfExtendedUnchanged,
            ExceptionPolicy::IfExtendedOverridePresent,
            ExceptionPolicy::IfExtendedDynamicConditionStrict,
        ] {
            let p = singleton_pipeline(true, false, policy);
            assert!(
                FusionOwnerCertificates::capture_original_owners(&p)
                    .fresh_singleton_certificates
                    .is_empty()
            );
        }
        for exclusion in [ExceptionExclusion::Always, ExceptionExclusion::IfExtended] {
            let mut policy = ExceptionPolicy::NoneUnchangedBooleanSingleton;
            policy.exclude(exclusion);
            assert_eq!(policy.owner(), NormalizedExceptionOwner::Unchanged);
            let p = singleton_pipeline(true, false, policy);
            assert!(
                FusionOwnerCertificates::capture_original_owners(&p)
                    .fresh_singleton_certificates
                    .is_empty()
            );
        }
    }

    #[test]
    fn singleton_declines_mixed_normal_and_deeper_owners() {
        for mode in 0..4 {
            let mut p =
                singleton_pipeline(true, false, ExceptionPolicy::NoneUnchangedBooleanSingleton);
            match mode {
                0 => p.exception_regions[0].depth = 1,
                1 => p.exception_regions[0].is_scope_fallback = false,
                2 => p.exception_regions[0].preserve_lasti = false,
                _ => p.exception_regions.push(p.exception_regions[0]),
            }
            assert!(
                FusionOwnerCertificates::capture_original_owners(&p)
                    .fresh_singleton_certificates
                    .is_empty()
            );
        }
    }

    #[test]
    fn singleton_manager_requires_known_setup_and_stable_owner() {
        let mut p = singleton_pipeline(true, true, ExceptionPolicy::NoneUnchangedBooleanSingleton);
        p.exception_regions[0].is_scope_fallback = false;
        p.exception_regions[0].depth = 2;
        assert!(
            FusionOwnerCertificates::capture_original_owners(&p)
                .fresh_singleton_certificates
                .is_empty()
        );
        p.exception_owner_provenance
            .synchronous_with_setups
            .insert(Label::from_u32(0));
        let mut certificates = FusionOwnerCertificates::capture_original_owners(&p);
        assert_eq!(certificates.fresh_singleton_certificates.len(), 1);
        p.exception_regions[0].depth += 1;
        certificates.retain_pipeline(&p);
        assert!(certificates.fresh_singleton_certificates.is_empty());
    }

    #[test]
    fn singleton_manager_requires_exact_source_split_origin() {
        for mode in 0..7 {
            let mut p =
                singleton_pipeline(true, true, ExceptionPolicy::NoneUnchangedBooleanSingleton);
            p.exception_regions[0].is_scope_fallback = false;
            p.exception_regions[0].depth = 2;
            p.exception_owner_provenance
                .synchronous_with_setups
                .insert(Label::from_u32(0));
            edit_singleton(&mut p, |items| {
                let index = items
                    .iter()
                    .position(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if instruction.opcode == crate::target::opcodes::UNPACK_SEQUENCE)
                    })
                    .unwrap();
                items.insert(index, Item::Label(Label::from_u32(4)));
            });
            p.next_label = 5;
            let mut split = p.exception_regions[0];
            p.exception_regions[0].end = Label::from_u32(4);
            split.start = Label::from_u32(4);
            match mode {
                1 => p.exception_owner_provenance.synchronous_with_setups.clear(),
                2 => split.priority = ExceptionRegionPriority::Resolved { start: 0, end: 1 },
                3 => split.target = Label::from_u32(2),
                5 => split.depth += 1,
                6 => split.preserve_lasti = false,
                _ => {}
            }
            p.exception_regions.push(split);
            if mode == 4 {
                p.exception_regions.push(split);
            }
            let certificates = FusionOwnerCertificates::capture_original_owners(&p);
            assert_eq!(
                certificates.fresh_singleton_certificates.len(),
                usize::from(mode == 0)
            );
        }
    }

    #[test]
    fn singleton_manager_transfers_only_recorded_cold_region_splits() {
        for mode in 0..5 {
            let mut p =
                singleton_pipeline(true, true, ExceptionPolicy::NoneUnchangedBooleanSingleton);
            p.exception_regions[0].is_scope_fallback = false;
            p.exception_regions[0].depth = 2;
            p.exception_owner_provenance
                .synchronous_with_setups
                .insert(Label::from_u32(0));
            edit_singleton(&mut p, |items| {
                let index = items
                    .iter()
                    .position(|item| {
                        matches!(item, Item::Instruction(instruction)
                        if instruction.opcode == crate::target::opcodes::UNPACK_SEQUENCE)
                    })
                    .unwrap();
                items.insert(index, Item::Label(Label::from_u32(4)));
            });
            p.next_label = 5;
            let mut certificates = FusionOwnerCertificates::capture_original_owners(&p);
            assert_eq!(certificates.fresh_singleton_certificates.len(), 1);
            assert!(certificates.has_manager_setup_certificates());
            let original = p.exception_regions[0];
            let mut rebuilt = original;
            rebuilt.start = Label::from_u32(4);
            if mode == 2 {
                rebuilt.depth += 1;
            } else if mode == 3 {
                rebuilt.priority = ExceptionRegionPriority::Resolved { start: 0, end: 1 };
            }
            p.exception_regions[0] = rebuilt;
            let mut reported_original = original;
            if mode == 4 {
                reported_original.end = Label::from_u32(2);
            }
            if mode == 0 {
                certificates.retain_after_cold_regions(&p, &[]);
            } else {
                certificates.retain_after_cold_regions(&p, &[(reported_original, rebuilt)]);
            }
            assert_eq!(
                certificates.fresh_singleton_certificates.len(),
                usize::from(mode == 1)
            );
        }
    }

    #[test]
    fn singleton_declines_unaccounted_original_slots_and_other_identity_tests() {
        for mode in 0..8 {
            let mut p =
                singleton_pipeline(true, true, ExceptionPolicy::NoneUnchangedBooleanSingleton);
            edit_singleton(&mut p, |items| {
                let at = items.iter().position(|item| matches!(item, Item::Instruction(i) if i.opcode == crate::target::opcodes::IS_OP)).unwrap();
                if mode >= 4 {
                    let op = [
                        NOP,
                        crate::target::opcodes::COPY,
                        crate::target::opcodes::LOAD_GLOBAL,
                        crate::target::opcodes::PUSH_NULL,
                    ][mode - 4];
                    items.insert(at, Item::Instruction(instruction(op, Operand::Value(1))));
                } else {
                    let Item::Instruction(i) = &mut items[at] else {
                        unreachable!()
                    };
                    match mode {
                        0 => i.set_operand(Operand::Value(1)),
                        1 => i.local_policy.add_hidden_exception_owner_slots(1),
                        2 => i.add_pre_fold_extra_instructions(1),
                        _ => i.exclude_from_exception_table(ExceptionExclusion::Always),
                    }
                }
            });
            assert!(
                FusionOwnerCertificates::capture_original_owners(&p)
                    .fresh_singleton_certificates
                    .is_empty()
            );
        }
    }

    #[test]
    fn singleton_requires_original_boundary_or_retained_original_predecessor() {
        let mut p =
            singleton_pipeline(false, false, ExceptionPolicy::NoneUnchangedBooleanSingleton);
        p.exception_owner_provenance.block_boundaries.clear();
        edit_singleton(&mut p, |items| {
            items.push(Item::Instruction(instruction(
                JUMP_FORWARD,
                Operand::Forward(Label::from_u32(0)),
            )));
        });
        assert!(
            FusionOwnerCertificates::capture_original_owners(&p)
                .fresh_singleton_certificates
                .is_empty()
        );
        let mut p = singleton_pipeline(true, false, ExceptionPolicy::NoneUnchangedBooleanSingleton);
        let mut cert = FusionOwnerCertificates::capture_original_owners(&p);
        assert_eq!(cert.fresh_singleton_certificates.len(), 1);
        edit_singleton(&mut p, |items| {
            let Item::Instruction(i) = &mut items[2] else {
                unreachable!()
            };
            i.retarget(Operand::Forward(Label::from_u32(3)));
        });
        cert.retain_pipeline(&p);
        assert!(cert.fresh_singleton_certificates.is_empty());
    }

    #[test]
    fn singleton_later_edit_invalidates_permanently_even_when_bytes_return() {
        let mut p = singleton_pipeline(true, false, ExceptionPolicy::NoneUnchangedBooleanSingleton);
        let original = p.graph.iter_items().copied().collect::<Vec<_>>();
        let mut cert = FusionOwnerCertificates::capture_original_owners(&p);
        assert_eq!(cert.fresh_singleton_certificates.len(), 1);
        edit_singleton(&mut p, |items| {
            let at = items.iter().position(|item| matches!(item, Item::Instruction(i) if i.opcode == crate::target::opcodes::IS_OP)).unwrap();
            items.insert(at, Item::Instruction(instruction(NOP, Operand::Value(0))));
        });
        cert.retain_pipeline(&p);
        assert!(cert.fresh_singleton_certificates.is_empty());
        p.graph =
            crate::assembler::block_graph::BlockGraph::from_items(original, &FxHashSet::default());
        cert.retain_pipeline(&p);
        assert!(cert.fresh_singleton_certificates.is_empty());
    }

    #[test]
    fn singleton_packed_origin_retains_unchanged_semantics_and_extra_slot_bits() {
        let mut i = instruction(NOT_TAKEN, Operand::Value(0));
        i.set_exception_policy(ExceptionPolicy::NoneUnchangedBooleanSingleton);
        i.add_pre_fold_extra_instructions(4);
        assert_eq!(
            i.exception_policy(),
            ExceptionPolicy::NoneUnchangedBooleanSingleton
        );
        assert_eq!(
            i.exception_policy().owner(),
            NormalizedExceptionOwner::Unchanged
        );
        assert_eq!(i.exception_policy().exclusion(), ExceptionExclusion::None);
        assert_eq!(i.pre_fold_extra_instructions(), 4);
        i.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
        assert_eq!(i.exception_policy(), ExceptionPolicy::NoneOverridePresent);
        assert_eq!(i.pre_fold_extra_instructions(), 4);
    }

    fn repeated_sequence_predicates() -> OptimizationPipeline {
        use crate::target::opcodes::{COMPARE_OP, GET_LEN, LOAD_SMALL_INT, MATCH_SEQUENCE};
        let mut p = singleton_pipeline(true, true, ExceptionPolicy::NoneUnchangedBooleanSingleton);
        edit_singleton(&mut p, |items| {
            let pair = items[2..4].to_vec();
            items[1] = Item::Instruction(instruction(MATCH_SEQUENCE, Operand::Value(0)));
            let mut length_check = vec![
                Item::Instruction(instruction(GET_LEN, Operand::Value(0))),
                Item::Instruction(instruction(LOAD_SMALL_INT, Operand::Value(2))),
                Item::Instruction(instruction(COMPARE_OP, Operand::Value(72))),
            ];
            length_check.extend(pair);
            items.splice(4..4, length_check);
        });
        p
    }

    #[test]
    fn singleton_repeated_predecessors_require_the_unique_complete_interval() {
        let p = repeated_sequence_predicates();
        let items = p.graph.iter_items().copied().collect::<Vec<_>>();
        let view = View::new(
            &items,
            &p.exception_regions,
            &p.exception_owner_provenance,
            &p.preserved_block_boundaries,
        )
        .unwrap();
        let (Item::Instruction(condition), Item::Instruction(marker)) = (items[2], items[3]) else {
            unreachable!()
        };
        assert!(view.endpoint(condition, marker).is_none());
        let certificates = FusionOwnerCertificates::capture_original_owners(&p);
        assert_eq!(certificates.fresh_singleton_certificates.len(), 1);
        let interval = &certificates.fresh_singleton_certificates[0].interval;
        assert_eq!(interval.range(&view, false).unwrap().0, 9);
        // The old generic resolver still declines the identical predecessor pairs.
        let generic = Certificate {
            anchor: Anchor::Marker(condition, marker),
            condition: interval.condition,
            marker: interval.marker,
            items: interval.items.clone(),
            labels: interval.labels.clone(),
            owner: interval.owner,
            boundaries: interval.boundaries.clone(),
            setups: interval.setups.clone(),
        };
        assert!(generic.valid(&view, false).is_none());
    }

    #[test]
    fn singleton_complete_interval_cannot_be_duplicated_or_gain_an_owner_boundary() {
        for mode in 0..3 {
            let mut p = repeated_sequence_predicates();
            let mut certificates = FusionOwnerCertificates::capture_original_owners(&p);
            assert_eq!(certificates.fresh_singleton_certificates.len(), 1);
            if mode == 0 {
                edit_singleton(&mut p, |items| {
                    let duplicate = items[7..14].to_vec();
                    items.splice(14..14, duplicate);
                });
                assert!(
                    FusionOwnerCertificates::capture_original_owners(&p)
                        .fresh_singleton_certificates
                        .is_empty()
                );
            } else if mode == 1 {
                edit_singleton(&mut p, |items| {
                    items.insert(10, Item::Label(Label::from_u32(4)));
                });
                p.exception_owner_provenance
                    .block_boundaries
                    .insert(Label::from_u32(4));
            } else {
                p.exception_regions[0].target = Label::from_u32(2);
            }
            certificates.retain_pipeline(&p);
            assert!(certificates.fresh_singleton_certificates.is_empty());
        }
    }

    #[test]
    fn singleton_sequence_boolean_gap_survives_the_full_compiler_pipeline() {
        use crate::code_object::Constant;
        use crate::target::opcodes::{GET_LEN, IS_OP, MATCH_SEQUENCE, UNPACK_SEQUENCE};
        fn varint(bytes: &[u8], at: &mut usize) -> usize {
            let mut value = 0;
            loop {
                let byte = bytes[*at];
                *at += 1;
                value = (value << 6) | usize::from(byte & 0x3f);
                if byte & 0x40 == 0 {
                    return value;
                }
            }
        }
        for singleton in ["True", "False"] {
            let source = format!(
                "async def f(value):\n    match value:\n        case ({singleton}, payload):\n            yield payload\n"
            );
            let module = crate::compile(&source, "singleton_sequence_interval.py").unwrap();
            let function = module
                .code
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "f" => Some(code),
                    _ => None,
                })
                .unwrap();
            let words = function
                .bytecode
                .as_chunks::<2>()
                .0
                .iter()
                .collect::<Vec<_>>();
            for opcode in [MATCH_SEQUENCE, GET_LEN, UNPACK_SEQUENCE] {
                assert_eq!(
                    words.iter().filter(|word| word[0] == opcode.code()).count(),
                    1
                );
            }
            let identity = words
                .iter()
                .enumerate()
                .filter_map(|(i, word)| (word[0] == IS_OP.code()).then_some(i))
                .collect::<Vec<_>>();
            assert_eq!(identity.len(), 1);
            let condition = identity[0] + 1;
            assert_eq!(words[condition][0], POP_JUMP_IF_FALSE.code());
            let marker = words
                .iter()
                .enumerate()
                .skip(condition + 1)
                .find_map(|(i, word)| (word[0] == NOT_TAKEN.code()).then_some(i))
                .unwrap();
            let mut at = 0;
            let mut ranges = Vec::new();
            while at < function.exception_table.len() {
                let start = varint(&function.exception_table, &mut at);
                let length = varint(&function.exception_table, &mut at);
                let target = varint(&function.exception_table, &mut at);
                let depth_and_lasti = varint(&function.exception_table, &mut at);
                ranges.push((start..start + length, target, depth_and_lasti));
            }
            assert!(
                ranges
                    .iter()
                    .any(|(range, _, dl)| range.contains(&condition) && *dl == 1)
            );
            assert!(
                !ranges.iter().any(|(range, _, _)| range.contains(&marker)),
                "{singleton}"
            );
        }
    }

    fn fallback_pipeline(globals: usize, policy: ExceptionPolicy) -> OptimizationPipeline {
        use crate::assembler::{Assembler, UnoptimizedCode};
        use crate::target::opcodes::{LOAD_CONST, LOAD_GLOBAL, RETURN_VALUE};
        let condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(Label::from_u32(2)));
        let mut marker = instruction(NOT_TAKEN, Operand::Value(0));
        marker.set_exception_policy(policy);
        let mut items = vec![Item::Label(Label::from_u32(0))];
        for _ in 0..globals {
            items.push(Item::Instruction(instruction(
                LOAD_GLOBAL,
                Operand::Value(1),
            )));
        }
        items.extend([
            Item::Instruction(instruction(LOAD_CONST, Operand::Value(0))),
            Item::Instruction(condition),
            Item::Instruction(marker),
            Item::Label(Label::from_u32(1)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label::from_u32(2)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label::from_u32(3)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
        ]);
        let mut pipeline = Assembler {
            code: UnoptimizedCode::Linear(items),
            exception_regions: vec![ExceptionRegion {
                start: Label::from_u32(0),
                end: Label::from_u32(1),
                target: Label::from_u32(3),
                depth: 0,
                preserve_lasti: true,
                is_scope_fallback: true,
                priority: ExceptionRegionPriority::OriginalRange(
                    Label::from_u32(0),
                    Label::from_u32(1),
                ),
            }],
            next_label: 4,
            ..Assembler::default()
        }
        .into_optimization_pipeline();
        pipeline
            .exception_owner_provenance
            .block_boundaries
            .insert(Label::from_u32(0));
        pipeline
    }
    #[test]
    fn fallback_slots_distinguish_fresh_and_global_call_tail() {
        for globals in [0, 1, 3] {
            for policy in [ExceptionPolicy::NonePresent, ExceptionPolicy::NoneAbsent] {
                let mut pipeline = fallback_pipeline(globals, policy);
                let before = FusionOwnerCertificates::snapshot(&pipeline);
                pipeline.fuse_superinstructions();
                let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
                assert_eq!(certificates.fallback_certificates.len(), 1);
                let mut items = pipeline.graph.into_items();
                certificates.apply(
                    &mut items,
                    &pipeline.exception_regions,
                    &pipeline.exception_owner_provenance,
                    &pipeline.preserved_block_boundaries,
                );
                let marker = items
                    .iter()
                    .find_map(|i| match i {
                        Item::Instruction(i) if i.opcode == NOT_TAKEN => Some(i),
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(
                    marker.exception_policy(),
                    if globals == 0 {
                        ExceptionPolicy::EXCLUDE_ALWAYS
                    } else {
                        ExceptionPolicy::NoneOverridePresent
                    }
                );
            }
        }
    }
    #[test]
    fn fallback_slots_preserve_explicit_exclusions_and_override() {
        for policy in [
            ExceptionPolicy::NoneOverridePresent,
            ExceptionPolicy::IfExtendedPresent,
            ExceptionPolicy::AlwaysPresent,
            ExceptionPolicy::IfExtendedAbsent,
            ExceptionPolicy::AlwaysAbsent,
            ExceptionPolicy::NoneDynamic,
        ] {
            let mut pipeline = fallback_pipeline(1, policy);
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            pipeline.fuse_superinstructions();
            let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
            assert!(certificates.fallback_certificates.is_empty());
            let mut items = pipeline.graph.into_items();
            let original = items.clone();
            certificates.apply(
                &mut items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &pipeline.preserved_block_boundaries,
            );
            assert!(
                items
                    .into_iter()
                    .zip(original)
                    .all(|(a, b)| same_item(a, b, false))
            );
        }
    }
    #[test]
    fn fallback_slots_reject_missing_boundary_mixed_and_deeper_regions() {
        for variant in 0..4 {
            let mut pipeline = fallback_pipeline(1, ExceptionPolicy::NoneAbsent);
            match variant {
                0 => {
                    pipeline.preserved_block_boundaries.clear();
                    pipeline.exception_owner_provenance.block_boundaries.clear();
                }
                1 => {
                    pipeline.exception_regions[0].depth = 1;
                }
                2 => {
                    pipeline.exception_regions[0].is_scope_fallback = false;
                }
                _ => {
                    let mut region = pipeline.exception_regions[0];
                    region.is_scope_fallback = false;
                    pipeline.exception_regions.push(region);
                }
            }
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            pipeline.fuse_superinstructions();
            assert!(
                FusionOwnerCertificates::capture(&before, &pipeline)
                    .fallback_certificates
                    .is_empty()
            );
        }
    }
    #[test]
    fn fallback_slots_reject_late_targets_without_original_boundary() {
        for globals in [0, 1] {
            for variant in 0..3 {
                let mut pipeline = fallback_pipeline(globals, ExceptionPolicy::NoneAbsent);
                // Clear provenance after graph construction has recorded its targets.
                pipeline.exception_owner_provenance.block_boundaries.clear();
                pipeline.preserved_block_boundaries.clear();
                let mut items = pipeline.graph.into_items();
                match variant {
                    0 => items.push(Item::Instruction(instruction(
                        crate::target::opcodes::JUMP_BACKWARD,
                        Operand::Backward(Label::from_u32(0)),
                    ))),
                    1 => pipeline.exception_regions[0].target = Label::from_u32(0),
                    _ => {
                        pipeline
                            .preserved_block_boundaries
                            .insert(Label::from_u32(0));
                    }
                }
                let view = View::new(
                    &items,
                    &pipeline.exception_regions,
                    &pipeline.exception_owner_provenance,
                    &pipeline.preserved_block_boundaries,
                )
                .unwrap();
                assert!(view.boundaries.contains(&Label::from_u32(0)));
                let (condition, marker) = *view.markers.values().next().unwrap().first().unwrap();
                let Item::Instruction(condition_instruction) = items[condition] else {
                    unreachable!();
                };
                assert!(view.uniform_fallback_slots(0, condition).is_none());
                assert!(
                    FallbackCertificate::capture(
                        &view,
                        &view,
                        0,
                        marker,
                        Some((condition, condition_instruction))
                    )
                    .is_none()
                );
            }
        }
    }
    #[test]
    fn fallback_slots_resolve_only_unambiguous_leading_original_alias() {
        for globals in [0, 1] {
            let pipeline = fallback_pipeline(globals, ExceptionPolicy::NoneAbsent);
            let mut items = pipeline.graph.into_items();
            items.splice(
                0..0,
                [
                    Item::Label(Label::from_u32(4)),
                    Item::Label(Label::from_u32(5)),
                ],
            );
            let mut provenance = pipeline.exception_owner_provenance;
            let view = View::new(
                &items,
                &pipeline.exception_regions,
                &provenance,
                &pipeline.preserved_block_boundaries,
            )
            .unwrap();
            let (condition, marker) = *view.markers.values().next().unwrap().first().unwrap();
            let Item::Instruction(condition_instruction) = items[condition] else {
                unreachable!();
            };
            let certificate = FallbackCertificate::capture(
                &view,
                &view,
                0,
                marker,
                Some((condition, condition_instruction)),
            )
            .unwrap();
            assert!(matches!(
                certificate.interval.anchor,
                Anchor::Label(label) if label == Label::from_u32(0)
            ));
            assert_eq!(
                certificate.valid(&view, false),
                Some((
                    marker,
                    if globals == 0 {
                        None
                    } else {
                        Some((Label::from_u32(3), 0, true))
                    }
                ))
            );
            // A second original boundary is ambiguous both at capture and later.
            provenance.block_boundaries.insert(Label::from_u32(4));
            let changed = View::new(
                &items,
                &pipeline.exception_regions,
                &provenance,
                &pipeline.preserved_block_boundaries,
            )
            .unwrap();
            assert!(certificate.valid(&changed, false).is_none());
            assert!(
                FallbackCertificate::capture(
                    &changed,
                    &changed,
                    0,
                    marker,
                    Some((condition, condition_instruction))
                )
                .is_none()
            );
        }
    }
    #[test]
    fn fallback_slots_reject_later_insertions_and_retargets() {
        for retarget in [false, true] {
            let mut pipeline = fallback_pipeline(1, ExceptionPolicy::NoneAbsent);
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            pipeline.fuse_superinstructions();
            let certificates = FusionOwnerCertificates::capture(&before, &pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            let mut items = pipeline.graph.into_items();
            if retarget {
                let condition = items
                    .iter_mut()
                    .find_map(|i| match i {
                        Item::Instruction(i) if i.opcode == POP_JUMP_IF_TRUE => Some(i),
                        _ => None,
                    })
                    .unwrap();
                condition.retarget(Operand::Forward(Label::from_u32(3)));
            } else {
                items.insert(1, Item::Instruction(instruction(NOP, Operand::Value(0))));
            }
            certificates.apply(
                &mut items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &pipeline.preserved_block_boundaries,
            );
            let marker = items
                .iter()
                .find_map(|i| match i {
                    Item::Instruction(i) if i.opcode == NOT_TAKEN => Some(i),
                    _ => None,
                })
                .unwrap();
            assert_eq!(marker.exception_policy(), ExceptionPolicy::NoneAbsent);
        }
    }

    fn original_fallback_pipeline(globals: usize) -> OptimizationPipeline {
        let mut pipeline = singleton_pipeline(true, false, ExceptionPolicy::NoneAbsent);
        edit_singleton(&mut pipeline, |items| {
            let start = items
                .iter()
                .position(|item| matches!(item, Item::Instruction(i) if i.location.line == 2))
                .unwrap();
            items.drain(start..start + 2);
            let mut prefix = Vec::new();
            for _ in 0..globals {
                let mut load = instruction(crate::target::opcodes::LOAD_GLOBAL, Operand::Value(1));
                load.location = SourceLocation::new(2, 2, 1, 5);
                prefix.push(Item::Instruction(load));
            }
            let mut load = instruction(LOAD_FAST, Operand::Value(0));
            load.location = SourceLocation::new(2, 2, 1, 5);
            prefix.push(Item::Instruction(load));
            items.splice(start..start, prefix);
        });
        pipeline
    }

    #[test]
    fn original_fallback_slots_follow_conditional_fallthrough_and_borrowed_loads() {
        for globals in [0, 1, 3] {
            let mut pipeline = original_fallback_pipeline(globals);
            let mut certificates = FusionOwnerCertificates::capture_original_owners(&pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            assert!(certificates.fresh_singleton_certificates.is_empty());
            pipeline.fuse_superinstructions();
            certificates.retain_pipeline(&pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            let mut items = pipeline.graph.into_items();
            for item in &mut items {
                if let Item::Instruction(i) = item
                    && i.opcode == LOAD_FAST
                {
                    i.opcode = LOAD_FAST_BORROW;
                }
            }
            certificates.apply(
                &mut items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &pipeline.preserved_block_boundaries,
            );
            let marker = items
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(i) if i.opcode == NOT_TAKEN && i.location.line == 2 => {
                        Some(i)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                marker.exception_policy(),
                if globals == 0 {
                    ExceptionPolicy::EXCLUDE_ALWAYS
                } else {
                    ExceptionPolicy::NoneOverridePresent
                }
            );
        }
    }

    #[test]
    fn original_fallback_slots_reject_changed_origin_or_array() {
        for change in 0..3 {
            let mut pipeline = original_fallback_pipeline(1);
            let mut certificates = FusionOwnerCertificates::capture_original_owners(&pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            edit_singleton(&mut pipeline, |items| {
                if change == 0 {
                    let Item::Instruction(condition) = &mut items[2] else {
                        unreachable!()
                    };
                    condition.retarget(Operand::Forward(Label::from_u32(3)));
                } else {
                    let start = items
                        .iter()
                        .position(
                            |item| matches!(item, Item::Instruction(i) if i.location.line == 2),
                        )
                        .unwrap();
                    if change == 1 {
                        items.insert(
                            start,
                            Item::Instruction(instruction(
                                crate::target::opcodes::LOAD_GLOBAL,
                                Operand::Value(1),
                            )),
                        );
                    } else {
                        let Item::Instruction(instruction) = &mut items[start] else {
                            unreachable!()
                        };
                        instruction.local_policy.add_hidden_exception_owner_slots(1);
                    }
                }
            });
            certificates.retain_pipeline(&pipeline);
            assert!(certificates.fallback_certificates.is_empty());
        }
    }

    #[test]
    fn original_fallback_slots_reject_nonuniform_and_unrecorded_owners() {
        for change in 0..4 {
            let mut pipeline = original_fallback_pipeline(1);
            match change {
                0 => pipeline.exception_regions[0].depth = 1,
                1 => pipeline.exception_regions[0].is_scope_fallback = false,
                2 => pipeline
                    .exception_regions
                    .push(pipeline.exception_regions[0]),
                _ => edit_singleton(&mut pipeline, |items| {
                    let start = items
                        .iter()
                        .position(
                            |item| matches!(item, Item::Instruction(i) if i.location.line == 2),
                        )
                        .unwrap();
                    items.insert(
                        start,
                        Item::Instruction(instruction(NOP, Operand::Value(0))),
                    );
                }),
            }
            assert!(
                FusionOwnerCertificates::capture_original_owners(&pipeline)
                    .fallback_certificates
                    .is_empty()
            );
        }
    }

    #[test]
    fn original_fallback_slots_survive_only_recorded_conditional_exit_copies() {
        let mut pipeline = original_fallback_pipeline(1);
        edit_singleton(&mut pipeline, |items| {
            let target = items
                .iter()
                .position(|item| matches!(item, Item::Label(label) if matches!(label.as_u32(), 2)))
                .unwrap();
            let Item::Instruction(exit) = &mut items[target + 1] else {
                unreachable!()
            };
            exit.location = SourceLocation::NONE;
        });
        let mut without_transfer = FusionOwnerCertificates::capture_original_owners(&pipeline);
        let mut certificates = FusionOwnerCertificates::capture_original_owners(&pipeline);
        assert_eq!(certificates.fallback_certificates.len(), 1);
        let original_location_absence = pipeline.capture_original_location_absence();
        let mut retargets = Vec::new();
        pipeline.duplicate_exit_blocks(&original_location_absence, &mut retargets);
        assert!(!retargets.is_empty());
        without_transfer.retain_pipeline(&pipeline);
        assert!(without_transfer.fallback_certificates.is_empty());
        certificates.retain_after_exit_copy(&pipeline, &retargets, &[]);
        assert_eq!(certificates.fallback_certificates.len(), 1);
        let mut items = pipeline.graph.into_items();
        certificates.apply(
            &mut items,
            &pipeline.exception_regions,
            &pipeline.exception_owner_provenance,
            &pipeline.preserved_block_boundaries,
        );
        let marker = items
            .iter()
            .find_map(|item| match item {
                Item::Instruction(i) if i.opcode == NOT_TAKEN && i.location.line == 2 => Some(i),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            marker.exception_policy(),
            ExceptionPolicy::NoneOverridePresent
        );
    }

    #[test]
    fn original_fallback_slots_survive_one_observed_local_fusion() {
        for globals in [0, 1] {
            let mut pipeline = original_fallback_pipeline(globals);
            edit_singleton(&mut pipeline, |items| {
                let index = items.iter().position(|item| matches!(item,
                    Item::Instruction(instruction) if instruction.opcode == LOAD_FAST && instruction.location.line == 2)).unwrap();
                let mut load = instruction(LOAD_FAST, Operand::Value(1));
                load.location = SourceLocation::new(2, 2, 1, 5);
                items.insert(index, Item::Instruction(load));
            });
            let mut unchanged = FusionOwnerCertificates::capture_original_owners(&pipeline);
            let mut certificates = FusionOwnerCertificates::capture_original_owners(&pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            pipeline.fuse_superinstructions();
            unchanged.retain_pipeline(&pipeline);
            assert!(unchanged.fallback_certificates.is_empty());
            certificates.retain_after_fusion(&before, &pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            let mut items = pipeline.graph.into_items();
            certificates.apply(
                &mut items,
                &pipeline.exception_regions,
                &pipeline.exception_owner_provenance,
                &pipeline.preserved_block_boundaries,
            );
            let marker = items
                .iter()
                .find_map(|item| match item {
                    Item::Instruction(instruction)
                        if instruction.opcode == NOT_TAKEN && instruction.location.line == 2 =>
                    {
                        Some(instruction)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                marker.exception_policy(),
                ExceptionPolicy::NoneOverridePresent
            );
        }
    }

    #[test]
    fn original_fallback_fusion_transfer_rejects_unproved_changes() {
        for change in [0, 1, 2] {
            let mut pipeline = original_fallback_pipeline(1);
            edit_singleton(&mut pipeline, |items| {
                let index = items.iter().position(|item| matches!(item,
                    Item::Instruction(instruction) if instruction.opcode == LOAD_FAST && instruction.location.line == 2)).unwrap();
                let mut load = instruction(LOAD_FAST, Operand::Value(1));
                load.location = SourceLocation::new(2, 2, 1, 5);
                items.insert(index, Item::Instruction(load));
                if change == 2 {
                    items.insert(index, Item::Instruction(load));
                    items.insert(index, Item::Instruction(load));
                }
            });
            let mut certificates = FusionOwnerCertificates::capture_original_owners(&pipeline);
            assert_eq!(certificates.fallback_certificates.len(), 1);
            let before = FusionOwnerCertificates::snapshot(&pipeline);
            pipeline.fuse_superinstructions();
            if change < 2 {
                edit_singleton(&mut pipeline, |items| {
                    let fused = items
                        .iter_mut()
                        .find_map(|item| match item {
                            Item::Instruction(instruction)
                                if instruction.opcode == LOAD_FAST_LOAD_FAST =>
                            {
                                Some(instruction)
                            }
                            _ => None,
                        })
                        .unwrap();
                    if change == 0 {
                        fused.set_operand(Operand::Value(34));
                    } else {
                        fused.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                    }
                });
            }
            certificates.retain_after_fusion(&before, &pipeline);
            assert!(certificates.fallback_certificates.is_empty());
        }
    }
}
