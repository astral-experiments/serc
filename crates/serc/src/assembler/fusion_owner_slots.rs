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

struct View<'a> {
    items: &'a [Item],
    regions: &'a [ExceptionRegion],
    provenance: &'a ExceptionOwnerProvenance,
    labels: FxHashMap<Label, usize>,
    positions: FxHashMap<Label, usize>,
    boundaries: FxHashSet<Label>,
    references: FxHashSet<Label>,
    markers: FxHashMap<Key, Vec<(usize, usize)>>,
}
impl<'a> View<'a> {
    fn new(
        items: &'a [Item],
        regions: &'a [ExceptionRegion],
        provenance: &'a ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> Option<Self> {
        let mut labels = FxHashMap::default();
        let mut positions = FxHashMap::default();
        let mut boundaries = provenance.block_boundaries.clone();
        boundaries.extend(preserved);
        let mut references = boundaries.clone();
        let mut markers: FxHashMap<Key, Vec<(usize, usize)>> = FxHashMap::default();
        let mut previous = None;
        let mut n = 0;
        for (i, item) in items.iter().copied().enumerate() {
            match item {
                Item::Label(l) => {
                    if labels.insert(l, i).is_some() {
                        return None;
                    }
                    positions.insert(l, n);
                }
                Item::Instruction(ins) => {
                    if let Operand::Forward(l) | Operand::Backward(l) = ins.operand() {
                        references.insert(l);
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
        for r in regions {
            references.extend([r.start, r.end, r.target]);
            boundaries.insert(r.target);
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
        let mut retained_origins = provenance
            .retained_normal_pop_blocks
            .iter()
            .collect::<Vec<_>>();
        retained_origins.sort_unstable_by_key(|(entry, _)| {
            (labels.get(*entry).copied().unwrap_or(usize::MAX), entry.0)
        });
        for (_, origin) in retained_origins {
            references.extend(origin.before.iter().copied());
            references.extend(origin.after.iter().copied());
        }
        Some(Self {
            items,
            regions,
            provenance,
            labels,
            positions,
            boundaries,
            references,
            markers,
        })
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
        let mut owners = Vec::new();
        for (index, r) in self.regions.iter().enumerate() {
            let s = *self.positions.get(&r.start)?;
            let e = *self.positions.get(&r.end)?;
            if s <= pos && pos < e {
                // A cleanup stack can retain an owner on the original array record
                // even when the lowered condition is excluded. The local interval
                // certificate does not establish that record's cleanup provenance.
                if r.depth != 0 {
                    return None;
                }
                let (ps, pe) = match r.priority {
                    ExceptionRegionPriority::OriginalRange(s, e) => {
                        (*self.positions.get(&s)?, *self.positions.get(&e)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                owners.push((
                    (r.is_scope_fallback, Reverse(ps), pe, index),
                    (r.target, r.depth, r.preserve_lasti),
                ));
            }
        }
        match c.exception_policy().exclusion() {
            ExceptionExclusion::Always => return Some(None),
            ExceptionExclusion::None => {}
            ExceptionExclusion::IfExtended => return None,
        }
        Some(owners.into_iter().min_by_key(|x| x.0).map(|x| x.1))
    }
    fn no_late_insertions_or_setups(&self, start: usize, end: usize) -> bool {
        if self.items[start..=end].iter().any(|x|matches!(x,Item::Instruction(i) if matches!(i.opcode,COPY_FREE_VARS|MAKE_CELL|RETURN_GENERATOR)||i.pre_fold_extra_instructions()!=0||i.local_policy.hidden_exception_owner_slots()==super::LocalPolicy::MAX_HIDDEN_EXCEPTION_OWNER_SLOTS)){return false;}
        for (l, location) in &self.provenance.setups {
            let Some(&p) = self.labels.get(l) else {
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
    owner: Owner,
    boundaries: FxHashSet<Label>,
    setups: Vec<(Label, Option<super::SourceLocation>)>,
}
impl Certificate {
    fn range(&self, v: &View<'_>, borrow: bool) -> Option<(usize, usize, usize)> {
        let (c, m) = v.endpoint(self.condition, self.marker)?;
        let start = match self.anchor {
            Anchor::Entry => 0,
            Anchor::Label(l) => *v.labels.get(&l)?,
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
        let (start, condition, marker) = self.range(view, borrow)?;
        if self.setups != view.provenance.setups
            || self.boundaries
                != view
                    .boundaries
                    .iter()
                    .copied()
                    .filter(|l| self.items.iter().any(|x| matches!(x,Item::Label(k)if k==l)))
                    .collect()
            || view.owner(condition)? != self.owner
        {
            return None;
        }
        self.matches_interval(view, start, marker, borrow)?;
        Some(marker)
    }

    fn matches_interval(
        &self,
        view: &View<'_>,
        start: usize,
        marker: usize,
        borrow: bool,
    ) -> Option<()> {
        let original_labels = self
            .items
            .iter()
            .filter_map(|x| match x {
                Item::Label(l) => Some(*l),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashSet<_>>();
        let mut old = self.items.iter().copied();
        for item in view.items[start..=marker].iter().copied() {
            if let Item::Label(l) = item {
                if !original_labels.contains(&l) && !view.references.contains(&l) {
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
        let (owner, removed) = view.uniform_fallback_interval(start, condition, false)?;
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
                items,
                owner,
                boundaries,
                setups: view.provenance.setups.clone(),
            },
        };
        result.valid(view, false)?;
        Some(result)
    }

    fn valid(&self, view: &View<'_>, borrow: bool) -> Option<usize> {
        let marker = self.interval.valid(view, borrow)?;
        let (start, condition, _) = self.interval.range(view, borrow)?;
        if let Anchor::Label(_) = self.interval.anchor
            && view.original_array_start(start)? != start
        {
            return None;
        }
        let (owner, removed) = view.uniform_fallback_interval(start, condition, false)?;
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
        if marker.exception_policy().exclusion() != ExceptionExclusion::None
            || !matches!(
                marker.exception_policy().owner(),
                NormalizedExceptionOwner::Present | NormalizedExceptionOwner::Absent
            )
        {
            return None;
        }
        let (owner, removed_slots) =
            before.uniform_fallback_interval(start, condition_index, false)?;
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
        let original_labels = self
            .interval
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashSet<_>>();
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
                        original_labels.contains(label) || after.references.contains(label)
                    }
                    Item::Instruction(_) => true,
                })
                .collect::<Vec<_>>();
            if one_fusion(&self.interval.items, &items) && found.replace(items).is_some() {
                return None;
            }
        }
        self.interval.items = found?;
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
                view.uniform_fallback_slots(start, condition)?
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
        // No hidden SETUP/POP_BLOCK records or second true boundary may enter
        // the certificate, including aliases that only become adjacent later.
        for (label, _) in &self.provenance.setups {
            let p = *self.labels.get(label)?;
            if start <= p && p <= condition {
                return None;
            }
        }
        if !self
            .provenance
            .removed_pop_block_boundaries
            .iter()
            .all(|label| {
                self.labels
                    .get(label)
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
            if instruction.exception_policy() != ExceptionPolicy::NoneUnchanged
                || instruction.pre_fold_extra_instructions() != 0
                || instruction.only_allows_unconditional_inlining()
                || instruction.local_policy.hidden_exception_owner_slots() != 0
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
                let s = *self.positions.get(&region.start)?;
                let e = *self.positions.get(&region.end)?;
                if s <= pos && pos < e {
                    if covered.is_some()
                        || !region.is_scope_fallback
                        || region.depth != 0
                        || !region.preserve_lasti
                    {
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

#[derive(Default)]
pub(super) struct FusionOwnerCertificates {
    certificates: Vec<Certificate>,
    fallback_certificates: Vec<FallbackCertificate>,
    fresh_singleton_certificates: Vec<FreshSingletonCertificate>,
}

impl FusionOwnerCertificates {
    /// Captures authored arrays before any CFG pass can reuse their owner slots.
    pub(super) fn capture_original_owners(p: &OptimizationPipeline) -> Self {
        if p.exception_regions.is_empty() || !p.graph.iter_items().any(|item| {
            matches!(item, Item::Instruction(instruction) if instruction.opcode == NOT_TAKEN && matches!(instruction.exception_policy(), ExceptionPolicy::NoneUnchangedBooleanSingleton | ExceptionPolicy::NonePresent | ExceptionPolicy::NoneAbsent))
        }) {
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
        result
    }

    /// Moves original-array certificates into the existing later-pass retention path.
    pub(super) fn extend_original_owners(&mut self, mut original: Self) {
        self.fallback_certificates
            .append(&mut original.fallback_certificates);
        self.fresh_singleton_certificates
            .append(&mut original.fresh_singleton_certificates);
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
    /// Carries already-validated original fallback arrays across this exit-copy transaction.
    pub(super) fn retain_after_exit_copy(
        &mut self,
        p: &OptimizationPipeline,
        retargets: &[(Instruction, Instruction)],
    ) {
        self.fallback_certificates
            .retain_mut(|certificate| certificate.rebind_copied_exit(retargets).is_some());
        // The complete array, owner, slot count, setups, and boundaries must still match.
        self.retain_pipeline(p);
    }

    /// Transfers original fallback proofs across the immediately preceding fusion pass.
    pub(super) fn retain_after_fusion(&mut self, before: &[Item], p: &OptimizationPipeline) {
        if !self.fallback_certificates.is_empty() {
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
            }
        }
        self.retain_pipeline(p);
    }

    pub(super) fn retain_pipeline(&mut self, p: &OptimizationPipeline) {
        if self.certificates.is_empty()
            && self.fallback_certificates.is_empty()
            && self.fresh_singleton_certificates.is_empty()
        {
            return;
        }
        let items = p.graph.iter_items().copied().collect::<Vec<_>>();
        let Some(view) = View::new(
            &items,
            &p.exception_regions,
            &p.exception_owner_provenance,
            &p.preserved_block_boundaries,
        ) else {
            self.certificates.clear();
            self.fallback_certificates.clear();
            self.fresh_singleton_certificates.clear();
            return;
        };
        self.certificates
            .retain(|c| c.valid(&view, false).is_some());
        self.fallback_certificates
            .retain(|c| c.valid(&view, false).is_some());
        self.fresh_singleton_certificates
            .retain(|certificate| certificate.valid(&view, false).is_some());
    }
    pub(super) fn apply(
        self,
        items: &mut [Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) {
        if self.certificates.is_empty()
            && self.fallback_certificates.is_empty()
            && self.fresh_singleton_certificates.is_empty()
        {
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
            .collect::<Vec<_>>();
        for (index, owner) in changes {
            let Item::Instruction(marker) = &mut items[index] else {
                continue;
            };
            if owner.is_none() {
                marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
            } else {
                marker.set_exception_owner(NormalizedExceptionOwner::OverridePresent);
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
        certificates.retain_after_exit_copy(&pipeline, &retargets);
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
