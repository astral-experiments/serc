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
use std::cmp::Reverse;

type Key = (i32, i32, i32, i32, u8, u32);
type Owner = Option<(Label, u32, bool)>;

fn same_instruction(a: Instruction, b: Instruction, borrow: bool) -> bool {
    (a.opcode == b.opcode
        || borrow
            && matches!(
                (a.opcode, b.opcode),
                (LOAD_FAST, LOAD_FAST_BORROW)
                    | (LOAD_FAST_LOAD_FAST, LOAD_FAST_BORROW_LOAD_FAST_BORROW)
            ))
        && a.operand() == b.operand()
        && a.location == b.location
        && a.depth_after() == b.depth_after()
        && a.local_policy == b.local_policy
        && a.no_location_policy == b.no_location_policy
        && a.fusion_policy == b.fusion_policy
        && a.jump_policy == b.jump_policy
        && a.exception_policy() == b.exception_policy()
        && a.pre_fold_extra_instructions() == b.pre_fold_extra_instructions()
        && a.only_allows_unconditional_inlining() == b.only_allows_unconditional_inlining()
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
    let Operand::Forward(target) = condition.operand() else {
        return None;
    };
    let l = marker.location;
    Some((
        l.line,
        l.end_line,
        l.column,
        l.end_column,
        condition.opcode.code(),
        target.0,
    ))
}

/// Reusable allocations for indexes whose records are rebuilt at every checkpoint.
#[derive(Default)]
struct ViewTables {
    labels: FxHashMap<Label, (usize, usize)>,
    markers: FxHashMap<Key, SmallVec<[(usize, usize); 1]>>,
}

impl ViewTables {
    fn clear(&mut self) {
        self.labels.clear();
        self.markers.clear();
    }
}

struct View<'a> {
    items: &'a [Item],
    regions: &'a [ExceptionRegion],
    provenance: &'a ExceptionOwnerProvenance,
    labels: FxHashMap<Label, (usize, usize)>,
    boundaries: FxHashSet<Label>,
    references: FxHashSet<Label>,
    markers: FxHashMap<Key, SmallVec<[(usize, usize); 1]>>,
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
            mut markers,
        } = tables;
        labels.clear();
        markers.clear();
        let mut boundaries = provenance.block_boundaries.clone();
        boundaries.extend(preserved);
        let mut previous = None;
        let mut n = 0;
        for (i, item) in items.iter().copied().enumerate() {
            match item {
                Item::Label(l) => {
                    if labels.insert(l, (i, n)).is_some() {
                        return None;
                    }
                }
                Item::Instruction(ins) => {
                    if let Operand::Forward(l) | Operand::Backward(l) = ins.operand() {
                        boundaries.insert(l);
                    }
                    if let Some((p, prior)) = previous {
                        if let Some(k) = key(prior, ins) {
                            markers.entry(k).or_default().push((p, i));
                        }
                    }
                    previous = Some((i, ins));
                    n += 1;
                }
            }
        }
        for region in regions {
            boundaries.insert(region.target);
        }
        let mut references = boundaries.clone();
        for r in regions {
            references.extend([r.start, r.end]);
            if let ExceptionRegionPriority::OriginalRange(s, e) = r.priority {
                references.extend([s, e]);
            }
        }
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
        for origin in provenance.retained_normal_pop_blocks.values() {
            references.extend(origin.before.iter().copied());
            references.extend(origin.after.iter().copied());
        }
        Some(Self {
            items,
            regions,
            provenance,
            labels,
            boundaries,
            references,
            markers,
        })
    }
    fn into_tables(self) -> ViewTables {
        ViewTables {
            labels: self.labels,
            markers: self.markers,
        }
    }

    fn label_index(&self, label: &Label) -> Option<&usize> {
        self.labels.get(label).map(|(index, _)| index)
    }

    fn label_position(&self, label: &Label) -> Option<&usize> {
        self.labels.get(label).map(|(_, position)| position)
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
        let pos = self.items[..condition_index]
            .iter()
            .filter(|x| matches!(x, Item::Instruction(_)))
            .count();
        let mut owner = None;
        for (index, r) in self.regions.iter().enumerate() {
            let s = *self.label_position(&r.start)?;
            let e = *self.label_position(&r.end)?;
            if s <= pos && pos < e {
                // A cleanup stack can retain an owner on the original array record
                // even when the lowered condition is excluded. The local interval
                // certificate does not establish that record's cleanup provenance.
                if r.depth != 0 {
                    return None;
                }
                let (ps, pe) = match r.priority {
                    ExceptionRegionPriority::OriginalRange(s, e) => {
                        (*self.label_position(&s)?, *self.label_position(&e)?)
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
            let Some(&p) = self.label_index(l) else {
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
            Anchor::Label(l) => *v.label_index(&l)?,
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
                if !self.labels.contains(&l) && !view.references.contains(&l) {
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
            || tail[1].operand() != Operand::Value(0)
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
        let boundaries = view
            .boundaries
            .iter()
            .copied()
            .filter(|label| {
                items
                    .iter()
                    .any(|item| matches!(item, Item::Label(current) if current == label))
            })
            .collect();
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
            let position = view.items[..condition]
                .iter()
                .filter(|item| matches!(item, Item::Instruction(_)))
                .count();
            let mut expected = Vec::new();
            for region in &self.manager_regions {
                let begin = *view.label_position(&region.start)?;
                let end = *view.label_position(&region.end)?;
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
        let after_start = *after.label_index(&anchor)?;
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
        let boundaries = after
            .boundaries
            .iter()
            .copied()
            .filter(|label| {
                items
                    .iter()
                    .any(|item| matches!(item, Item::Label(l) if l == label))
            })
            .collect();
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
        let boundaries = before
            .boundaries
            .iter()
            .copied()
            .filter(|l| items.iter().any(|x| matches!(x, Item::Label(k) if k == l)))
            .collect();
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
                || !matches!(before.operand(), Operand::Forward(_))
                || !matches!(after.operand(), Operand::Forward(_))
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
                        self.interval.labels.contains(label) || after.references.contains(label)
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
                self.label_index(label)
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
            let position = *self.label_index(label)?;
            if start <= position && position <= condition {
                return None;
            }
        }
        if self
            .provenance
            .removed_pop_block_boundaries
            .iter()
            .any(|label| {
                self.label_index(label)
                    .is_none_or(|&position| start <= position && position <= condition)
            })
        {
            return None;
        }
        let mut position = self.items[..start]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
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
                let begin = *self.label_position(&region.start)?;
                let end = *self.label_position(&region.end)?;
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
            let p = *self.label_index(label)?;
            if start <= p && p <= condition {
                return None;
            }
        }
        if !self
            .provenance
            .removed_pop_block_boundaries
            .iter()
            .all(|label| {
                self.label_index(label)
                    .is_some_and(|&p| p < start || p > condition)
            })
        {
            return None;
        }
        let mut owner = None;
        let mut removed = 0;
        let mut previous_instruction = None;
        let mut pos = self.items[..start]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
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
                let s = *self.label_position(&region.start)?;
                let e = *self.label_position(&region.end)?;
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
                    } else if after.references.contains(label) {
                        return None;
                    }
                }
            }
        }
        // SETUP records survive early compaction and fusion, then disappear before
        // NOT_TAKEN is appended. Its live ordinal selects a concrete old-array cell.
        let ordinal = after.items[..after_marker]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        let source_index = *cells.get(ordinal)?.as_ref()?;
        let Item::Instruction(source) = after.items[source_index] else {
            return None;
        };
        if source.exception_policy() != ExceptionPolicy::NoneUnchanged {
            return None;
        }
        let position = after.items[..source_index]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        let mut regions = Vec::new();
        for (index, region) in after.regions.iter().enumerate() {
            let begin = *after.label_position(&region.start)?;
            let end = *after.label_position(&region.end)?;
            if begin <= position && position < end {
                let (priority_start, priority_end) = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*after.label_position(&start)?, *after.label_position(&end)?)
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
        let original_position = before.items[..original_source]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        if !(*before.label_position(&region.start)? <= original_position
            && original_position < *before.label_position(&region.end)?)
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
                boundaries: after
                    .boundaries
                    .iter()
                    .copied()
                    .filter(|label| {
                        after.items[..=after_marker].iter().any(
                            |item| matches!(item, Item::Label(candidate) if candidate == label),
                        )
                    })
                    .collect(),
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
        let position = view.items[..source]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        if !(*view.label_position(&region.start)? <= position
            && position < *view.label_position(&region.end)?)
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

        let instructions = origin
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Instruction(instruction) => Some(*instruction),
                Item::Label(_) => None,
            })
            .collect::<Vec<_>>();
        let [.., check, condition, marker] = instructions.as_slice() else {
            return None;
        };
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
        let mut interval = Certificate {
            anchor: Anchor::Label(origin.entry),
            condition: *condition,
            marker: *marker,
            items: origin.items.clone(),
            labels: labels_in(&origin.items),
            owner: Some((region.target, region.depth, region.preserve_lasti)),
            boundaries: view.boundaries.clone(),
            setups: view.provenance.setups.clone(),
        };
        interval
            .boundaries
            .retain(|label| origin.items.contains(&Item::Label(*label)));
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
        let position = view.items[..index]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        Self::owner_at(view, position)
    }

    fn owner_at(view: &View<'_>, position: usize) -> Option<ExceptionRegion> {
        let mut owner = None;
        for (order, region) in view.regions.iter().enumerate() {
            let start = *view.label_position(&region.start)?;
            let end = *view.label_position(&region.end)?;
            if start <= position && position < end {
                let (start, end) = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*view.label_position(&start)?, *view.label_position(&end)?)
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

    /// Accounts only for a global-call NULL fold or an observed pair of local loads.
    fn slots(&self, view: &View<'_>, start: usize, condition: usize) -> Option<usize> {
        use crate::target::opcodes::{
            BUILD_TUPLE, CALL, CHECK_EXC_MATCH, LOAD_ATTR, LOAD_CONST, LOAD_FAST_CHECK, LOAD_GLOBAL,
        };

        if !view.no_late_insertions_or_setups(start, condition)
            || view.provenance.setups.iter().any(|(label, _)| {
                view.label_index(label)
                    .is_none_or(|position| start <= *position && *position <= condition)
            })
            || view
                .provenance
                .removed_pop_block_boundaries
                .iter()
                .any(|label| {
                    view.label_index(label)
                        .is_none_or(|position| start <= *position && *position <= condition)
                })
        {
            return None;
        }
        let mut removed = 0;
        let mut global_call = false;
        let mut call = false;
        let mut position = view.items[..start]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        for (offset, item) in view.items[start..=condition].iter().enumerate() {
            let index = start + offset;
            let instruction = match item {
                Item::Label(label) => {
                    if offset != 0 && view.references.contains(label) {
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
                || !same_manager_region(&Self::owner_at(view, position)?, &self.region)
            {
                return None;
            }
            match instruction.opcode {
                LOAD_GLOBAL => {
                    let Operand::Value(argument) = instruction.operand() else {
                        return None;
                    };
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
                CALL => call = true,
                POP_JUMP_IF_FALSE if index == condition => {}
                _ => return None,
            }
            position += 1;
        }
        (removed <= 1 && (!global_call || call)).then_some(removed)
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

/// A folded assertion's original empty success array under only the scope fallback.
struct FoldedAssertSuccessCertificate {
    entry: Label,
    next: Label,
    jump: Instruction,
    fallback: ExceptionRegion,
}

impl FoldedAssertSuccessCertificate {
    fn sole_fallback(view: &View<'_>, index: usize) -> Option<ExceptionRegion> {
        let position = view.items[..index]
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        let mut owner = None;
        for region in view.regions {
            let start = *view.label_position(&region.start)?;
            let end = *view.label_position(&region.end)?;
            if start <= position && position < end {
                if !region.is_scope_fallback
                    || region.depth != 0
                    || !region.preserve_lasti
                    || owner.replace(*region).is_some()
                {
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
            || !matches!(jump.operand(), Operand::Forward(_))
            || jump.location != nop.location
            || jump.depth_after() != nop.depth_after()
            || jump.exception_policy() != ExceptionPolicy::NoneUnchanged
            || jump.pre_fold_extra_instructions() != 0
            || jump.local_policy != super::LocalPolicy::default()
            || jump.no_location_policy != super::ir::NoLocationPolicy::default()
            || jump.jump_policy != super::ir::JumpPolicy::default()
        {
            return None;
        }
        let fallback = Self::sole_fallback(view, index + 2)?;
        if !same_manager_region(&Self::sole_fallback(view, index)?, &fallback) {
            return None;
        }
        let result = Self {
            entry: *entry,
            next: *next,
            jump: *jump,
            fallback,
        };
        result.valid(view)?;
        Some(result)
    }

    fn valid(&self, view: &View<'_>) -> Option<usize> {
        let start = *view.label_index(&self.entry)?;
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
        let fallback = Self::sole_fallback(view, start + 1)?;
        // Cold ordering can split the enclosing fallback range. Its target,
        // stack state, and original priority still identify this same owner.
        let mut shape = fallback;
        shape.start = self.fallback.start;
        shape.end = self.fallback.end;
        same_manager_region(&shape, &self.fallback).then_some(start + 1)
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
    scratch: Option<Box<ViewTables>>,
}

impl FusionOwnerCertificates {
    /// Captures authored arrays before any CFG pass can reuse their owner slots.
    pub(super) fn capture_original_owners(p: &OptimizationPipeline) -> Self {
        if p.exception_regions.is_empty() || (p.exception_owner_provenance.handler_dispatch_arrays.is_empty() && !p.graph.iter_items().any(|item| {
            matches!(item, Item::Instruction(instruction)
                if (instruction.opcode == NOT_TAKEN && matches!(instruction.exception_policy(), ExceptionPolicy::NoneUnchangedBooleanSingleton | ExceptionPolicy::NonePresent | ExceptionPolicy::NoneAbsent))
                    || (instruction.opcode == NOP && instruction.pre_fold_extra_instructions() == 2
                        && instruction.no_location_policy.borrow_fallthrough() == super::BorrowFallthrough::Barrier))
        })) {
            return Self::default();
        }
        let items = p.graph.iter_items().copied().collect::<Vec<_>>();
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
    pub(super) fn extend_original_owners(&mut self, mut original: Self) {
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
            || !p
                .graph
                .iter_items()
                .any(|x| matches!(x,Item::Instruction(i)if i.opcode==NOT_TAKEN))
        {
            Vec::new()
        } else {
            p.graph.iter_items().copied().collect()
        }
    }
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
                                    let prior = before[..start]
                                        .iter()
                                        .filter_map(|x| match x {
                                            Item::Instruction(i) => Some(*i),
                                            Item::Label(_) => None,
                                        })
                                        .collect::<Vec<_>>();
                                    if prior.len() >= 2
                                        && key(prior[prior.len() - 2], prior[prior.len() - 1])
                                            .is_some()
                                    {
                                        Some(Anchor::Marker(
                                            prior[prior.len() - 2],
                                            prior[prior.len() - 1],
                                        ))
                                    } else {
                                        None
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
                                        boundaries: a.boundaries.clone(),
                                        setups: p.exception_owner_provenance.setups.clone(),
                                    };
                                    if let Some((s, c, m)) = cert.range(&a, false) {
                                        if a.owner(c) == Some(owner)
                                            && a.no_late_insertions_or_setups(s, m)
                                            && one_fusion(&before[start..=index], &after[s..=m])
                                        {
                                            cert.items = after[s..=m].to_vec();
                                            cert.labels = labels_in(&cert.items);
                                            cert.boundaries.retain(|l| {
                                                cert.items
                                                    .iter()
                                                    .any(|x| matches!(x,Item::Label(k)if k==l))
                                            });
                                            result.certificates.push(cert);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(certificate) =
                            ManagerSetupCertificate::capture(&b, &a, start, index, previous)
                        {
                            result.manager_setup_certificates.push(certificate);
                        }
                        if let Some(cert) =
                            FallbackCertificate::capture(&b, &a, start, index, previous)
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
                            FallbackCertificate::capture(&b, &a, start, index, previous)
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
                let replacements = region_retargets
                    .iter()
                    .filter_map(|(original, rebuilt)| {
                        let mut original_shape = *rebuilt;
                        original_shape.start = region.start;
                        original_shape.end = region.end;
                        (same_manager_region(original, region)
                            && same_manager_region(&original_shape, region))
                        .then_some(*rebuilt)
                    })
                    .collect::<Vec<_>>();
                if replacements.is_empty() {
                    transferred.push(*region);
                } else {
                    transferred.extend(replacements);
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
    pub(super) fn retain_after_fusion(&mut self, before: &[Item], p: &OptimizationPipeline) {
        if !self.fallback_certificates.is_empty() || !self.handler_dispatch_certificates.is_empty()
        {
            let items = p.graph.iter_items().copied().collect::<Vec<_>>();
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
                self.fallback_certificates.retain_mut(|certificate| {
                    certificate.valid(&after, false).is_some()
                        || certificate.rebind_one_fusion(&before, &after).is_some()
                });
                self.handler_dispatch_certificates
                    .retain_mut(|certificate| {
                        certificate.valid(&after, false).is_some()
                            || certificate.rebind_one_fusion(&before, &after).is_some()
                    });
                self.retain_view(&after);
                return;
            }
        }
        self.retain_pipeline(p);
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
        let items = p.graph.iter_items().copied().collect::<Vec<_>>();
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
                let replacements = retargets
                    .iter()
                    .filter_map(|(original, rebuilt)| {
                        let mut original_shape = *rebuilt;
                        original_shape.start = region.start;
                        original_shape.end = region.end;
                        (same_manager_region(original, region)
                            && same_manager_region(&original_shape, region))
                        .then_some(*rebuilt)
                    })
                    .collect::<Vec<_>>();
                if replacements.is_empty() {
                    transferred.push(*region);
                } else {
                    transferred.extend(replacements);
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
                let position = view.items[..source]
                    .iter()
                    .filter(|item| matches!(item, Item::Instruction(_)))
                    .count();
                let mut candidates = retargets.iter().filter_map(|(original, rebuilt)| {
                    if !same_manager_region(original, &certificate.region) {
                        return None;
                    }
                    let start = *view.label_position(&rebuilt.start)?;
                    let end = *view.label_position(&rebuilt.end)?;
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
        let items = p.graph.iter_items().copied().collect::<Vec<_>>();
        let mut tables = self.scratch.take().unwrap_or_default();
        let Some(view) = View::with_tables(
            &items,
            &p.exception_regions,
            &p.exception_owner_provenance,
            &p.preserved_block_boundaries,
            std::mem::take(&mut *tables),
        ) else {
            self.clear();
            return;
        };
        self.retain_view(&view);
        if self.is_empty() {
            return;
        }
        *tables = view.into_tables();
        // Keep only allocation capacity; every record is rebuilt by the next pass.
        tables.clear();
        self.scratch = Some(tables);
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
    pub(super) fn manager_owners(
        &self,
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> Vec<(Instruction, Instruction, ExceptionRegion)> {
        if self.manager_setup_certificates.is_empty() {
            return Vec::new();
        }
        let Some(view) = View::new(items, regions, provenance, preserved) else {
            return Vec::new();
        };
        self.manager_setup_certificates
            .iter()
            .filter_map(|certificate| {
                let (condition, marker) = certificate.valid(&view, true)?;
                let (Item::Instruction(condition), Item::Instruction(marker)) =
                    (items[condition], items[marker])
                else {
                    return None;
                };
                Some((condition, marker, certificate.region))
            })
            .collect()
    }

    pub(super) fn apply(
        self,
        items: &mut [Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) {
        if self.is_empty() {
            return;
        }
        let Some(view) = View::new(items, regions, provenance, preserved) else {
            return;
        };
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
        let mut condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(Label(1)));
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
    fn captures_one_actual_fusion() {
        let (a, b) = pair();
        assert!(one_fusion(&a, &b));
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
        items.insert(2, Item::Label(Label(2)));
        assert!(valid(&c, &items, false));
    }
    #[test]
    fn rejects_referenced_added_label() {
        let (c, mut items) = certificate();
        items.insert(2, Item::Label(Label(2)));
        items.push(Item::Instruction(instruction(
            JUMP_FORWARD,
            Operand::Forward(Label(2)),
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
        i.retarget(Operand::Forward(Label(3)));
        assert!(!valid(&c, &items, false));
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
        let mut condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(Label(2)));
        if excluded {
            condition.exclude_from_exception_table(ExceptionExclusion::Always);
        }
        let mut marker = instruction(NOT_TAKEN, Operand::Value(0));
        marker.set_exception_owner(owner);
        let items = vec![
            Item::Label(Label(0)),
            Item::Instruction(instruction(LOAD_FAST, Operand::Value(2))),
            Item::Instruction(instruction(LOAD_FAST, Operand::Value(3))),
            Item::Instruction(condition),
            Item::Instruction(marker),
            Item::Label(Label(1)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label(2)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label(3)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
        ];
        Assembler {
            code: UnoptimizedCode::Linear(items),
            exception_regions: vec![ExceptionRegion {
                start: Label(0),
                end: Label(1),
                target: Label(3),
                depth: 0,
                preserve_lasti: true,
                is_scope_fallback: false,
                priority: ExceptionRegionPriority::OriginalRange(Label(0), Label(1)),
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
            .insert(Label(0));
        pipeline
            .exception_owner_provenance
            .source_boundaries
            .insert(Label(0));
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
                    .push((Label(0), None)),
                8 => {
                    edit_singleton(&mut pipeline, |items| {
                        let marker = items
                            .iter()
                            .position(|item| {
                                matches!(item,
                            Item::Instruction(i) if i.opcode == NOT_TAKEN)
                            })
                            .unwrap();
                        items.insert(marker, Item::Label(Label(4)));
                    });
                    pipeline
                        .exception_owner_provenance
                        .block_boundaries
                        .insert(Label(4));
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
            start: Label(2),
            end: Label(3),
            target: Label(3),
            depth: 2,
            preserve_lasti: true,
            is_scope_fallback: true,
            priority: ExceptionRegionPriority::OriginalRange(Label(2), Label(3)),
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
        let mut items = vec![Item::Label(Label(0))];
        if fallthrough {
            items.push(Item::Instruction(instruction(LOAD_FAST, Operand::Value(0))));
            items.push(Item::Instruction(instruction(
                POP_JUMP_IF_FALSE,
                Operand::Forward(Label(2)),
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
            (POP_JUMP_IF_FALSE, Operand::Forward(Label(2))),
            (NOT_TAKEN, Operand::Value(0)),
        ] {
            let mut item = instruction(op, arg);
            item.location = SourceLocation::new(2, 2, 1, 5);
            if op == NOT_TAKEN {
                item.set_exception_policy(policy);
            }
            items.push(Item::Instruction(item));
        }
        for label in [Label(1), Label(2), Label(3)] {
            items.push(Item::Label(label));
            items.push(Item::Instruction(instruction(
                RETURN_VALUE,
                Operand::Value(0),
            )));
        }
        let mut p = Assembler {
            code: UnoptimizedCode::Linear(items),
            exception_regions: vec![ExceptionRegion {
                start: Label(0),
                end: Label(1),
                target: Label(3),
                depth: 0,
                preserve_lasti: true,
                is_scope_fallback: true,
                priority: ExceptionRegionPriority::OriginalRange(Label(0), Label(1)),
            }],
            next_label: 4,
            ..Assembler::default()
        }
        .into_optimization_pipeline();
        p.exception_owner_provenance
            .block_boundaries
            .insert(Label(0));
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
            .insert(Label(0));
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
                .insert(Label(0));
            edit_singleton(&mut p, |items| {
                let index = items
                    .iter()
                    .position(|item| {
                        matches!(item, Item::Instruction(instruction)
                            if instruction.opcode == crate::target::opcodes::UNPACK_SEQUENCE)
                    })
                    .unwrap();
                items.insert(index, Item::Label(Label(4)));
            });
            p.next_label = 5;
            let mut split = p.exception_regions[0];
            p.exception_regions[0].end = Label(4);
            split.start = Label(4);
            match mode {
                1 => p.exception_owner_provenance.synchronous_with_setups.clear(),
                2 => split.priority = ExceptionRegionPriority::Resolved { start: 0, end: 1 },
                3 => split.target = Label(2),
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
                .insert(Label(0));
            edit_singleton(&mut p, |items| {
                let index = items
                    .iter()
                    .position(|item| {
                        matches!(item, Item::Instruction(instruction)
                        if instruction.opcode == crate::target::opcodes::UNPACK_SEQUENCE)
                    })
                    .unwrap();
                items.insert(index, Item::Label(Label(4)));
            });
            p.next_label = 5;
            let mut certificates = FusionOwnerCertificates::capture_original_owners(&p);
            assert_eq!(certificates.fresh_singleton_certificates.len(), 1);
            assert!(certificates.has_manager_setup_certificates());
            let original = p.exception_regions[0];
            let mut rebuilt = original;
            rebuilt.start = Label(4);
            if mode == 2 {
                rebuilt.depth += 1;
            } else if mode == 3 {
                rebuilt.priority = ExceptionRegionPriority::Resolved { start: 0, end: 1 };
            }
            p.exception_regions[0] = rebuilt;
            let mut reported_original = original;
            if mode == 4 {
                reported_original.end = Label(2);
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
                Operand::Forward(Label(0)),
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
            i.retarget(Operand::Forward(Label(3)));
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
                edit_singleton(&mut p, |items| items.insert(10, Item::Label(Label(4))));
                p.exception_owner_provenance
                    .block_boundaries
                    .insert(Label(4));
            } else {
                p.exception_regions[0].target = Label(2);
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
        let condition = instruction(POP_JUMP_IF_TRUE, Operand::Forward(Label(2)));
        let mut marker = instruction(NOT_TAKEN, Operand::Value(0));
        marker.set_exception_policy(policy);
        let mut items = vec![Item::Label(Label(0))];
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
            Item::Label(Label(1)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label(2)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
            Item::Label(Label(3)),
            Item::Instruction(instruction(RETURN_VALUE, Operand::Value(0))),
        ]);
        let mut pipeline = Assembler {
            code: UnoptimizedCode::Linear(items),
            exception_regions: vec![ExceptionRegion {
                start: Label(0),
                end: Label(1),
                target: Label(3),
                depth: 0,
                preserve_lasti: true,
                is_scope_fallback: true,
                priority: ExceptionRegionPriority::OriginalRange(Label(0), Label(1)),
            }],
            next_label: 4,
            ..Assembler::default()
        }
        .into_optimization_pipeline();
        pipeline
            .exception_owner_provenance
            .block_boundaries
            .insert(Label(0));
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
                        Operand::Backward(Label(0)),
                    ))),
                    1 => pipeline.exception_regions[0].target = Label(0),
                    _ => {
                        pipeline.preserved_block_boundaries.insert(Label(0));
                    }
                }
                let view = View::new(
                    &items,
                    &pipeline.exception_regions,
                    &pipeline.exception_owner_provenance,
                    &pipeline.preserved_block_boundaries,
                )
                .unwrap();
                assert!(view.boundaries.contains(&Label(0)));
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
            items.splice(0..0, [Item::Label(Label(4)), Item::Label(Label(5))]);
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
                Anchor::Label(Label(0))
            ));
            assert_eq!(
                certificate.valid(&view, false),
                Some((
                    marker,
                    if globals == 0 {
                        None
                    } else {
                        Some((Label(3), 0, true))
                    }
                ))
            );
            // A second original boundary is ambiguous both at capture and later.
            provenance.block_boundaries.insert(Label(4));
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
                condition.retarget(Operand::Forward(Label(3)));
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
                    condition.retarget(Operand::Forward(Label(3)));
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
                .position(|item| matches!(item, Item::Label(Label(2))))
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
