use super::{
    Anchor, COPY_FREE_VARS, Certificate, ExceptionPolicy, ExceptionRegion, FxHashSet, Instruction,
    Item, Label, MAKE_CELL, NOP, NOT_TAKEN, Operand, Owner, POP_JUMP_IF_FALSE, RETURN_GENERATOR,
    View, boundaries_in, cold_region_replacements, incoming, is_scope_exit, key, labels_in,
    one_fusion, same_item, same_manager_region,
};
use crate::assembler::SourceLocation;
use crate::target::opcodes::{COPY, IS_OP, LOAD_COMMON_CONSTANT, LOAD_GLOBAL, RESUME};
use crate::target::operands::CommonConstant;

#[cfg(test)]
mod tests;

/// Binds a protected builtin guard cell reused after observed local fusions.
pub(super) struct BuiltinGuardCertificate {
    interval: Certificate,
    region: ExceptionRegion,
    authored_region: ExceptionRegion,
    authored_regions: Vec<ExceptionRegion>,
    regions: Vec<ExceptionRegion>,
    cold_transported: bool,
    incoming: Vec<Instruction>,
    source_boundaries: FxHashSet<Label>,
}

impl BuiltinGuardCertificate {
    pub(super) fn capture_all(before: &View<'_>, after: &View<'_>) -> Vec<Self> {
        let mut result = Vec::new();
        let mut start = 0;
        let mut anchor = Some(Anchor::Entry);
        let mut has_instruction = false;
        let mut previous = None;
        for (index, item) in before.items.iter().copied().enumerate() {
            match item {
                Item::Label(label) if before.boundaries.contains(&label) => {
                    // A terminator clears the previous array, but the next authored
                    // entry still starts an independent original instruction array.
                    if has_instruction
                        || (anchor.is_none()
                            && before.provenance.block_boundaries.contains(&label)
                            && before.provenance.source_boundaries.contains(&label)
                            && !before.provenance.normal_try_copy_blocks.contains(&label)
                            && !before
                                .provenance
                                .retired_normal_try_copy_labels
                                .contains(&label)
                            && !before
                                .provenance
                                .normal_try_copy_internal_labels
                                .contains(&label))
                    {
                        start = index;
                        has_instruction = false;
                        anchor = Some(Anchor::Label(label));
                    }
                }
                Item::Label(_) => {}
                Item::Instruction(instruction) => {
                    if instruction.opcode == NOT_TAKEN {
                        if let Some(anchor) = anchor.clone()
                            && let Some(certificate) =
                                Self::capture(before, after, anchor, start, index)
                        {
                            result.push(certificate);
                        }
                        anchor = previous
                            .filter(|prior| key(*prior, instruction).is_some())
                            .map(|prior| Anchor::Marker(prior, instruction));
                        start = index + 1;
                        has_instruction = false;
                        previous = None;
                    } else {
                        previous = Some(instruction);
                        has_instruction = true;
                        if is_scope_exit(instruction.opcode)
                            || (instruction.opcode.has_jump()
                                && !crate::target::is_conditional_jump(instruction.opcode))
                        {
                            start = index + 1;
                            anchor = None;
                            previous = None;
                            has_instruction = false;
                        }
                    }
                }
            }
        }
        result
    }

    fn capture(
        before: &View<'_>,
        after: &View<'_>,
        anchor: Anchor,
        start: usize,
        marker: usize,
    ) -> Option<Self> {
        let tail = guard(before, start, marker)?;
        let (condition, marker_instruction) = (tail[4].1, tail[5].1);
        let owner = before.owner(tail[4].0)??;
        let insertions = stable_late_slots(before, start, marker, &anchor)?;
        if owner.1 != 0 || owner.2 {
            return None;
        }
        for (index, _) in &tail[..5] {
            if before.owner(*index)? != Some(owner) {
                return None;
            }
        }
        let position = before.instruction_position(tail[4].0);
        let mut candidates = before.regions.iter().filter(|region| {
            !region.is_scope_fallback
                && (region.target, region.depth, region.preserve_lasti) == owner
                && before
                    .label_position(region.start)
                    .is_some_and(|start| *start <= position)
                && before
                    .label_position(region.end)
                    .is_some_and(|end| position < *end)
        });
        let region = *candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        before.label_position(region.target)?;
        let mut interval = Certificate {
            anchor,
            condition,
            marker: marker_instruction,
            items: Vec::new(),
            labels: FxHashSet::default(),
            owner: Some(owner),
            boundaries: FxHashSet::default(),
            setups: before.provenance.setups.clone(),
        };
        let (after_start, _, after_marker) = interval.range(after, false)?;
        guard(after, after_start, after_marker)?;
        if !bounded_fusions(
            &before.items[start..=marker],
            &after.items[after_start..=after_marker],
            insertions,
        ) {
            return None;
        }
        interval.items = after.items[after_start..=after_marker].to_vec();
        interval.labels = labels_in(&interval.items);
        interval.boundaries = boundaries_in(&interval.items, &after.boundaries);
        let incoming_edges = incoming(before, &interval.labels);
        if incoming_edges != incoming(after, &interval.labels) {
            return None;
        }
        let source_boundaries =
            boundaries_in(&interval.items, &before.provenance.source_boundaries);
        let certificate = Self {
            interval,
            region,
            authored_region: region,
            authored_regions: before.regions.to_vec(),
            regions: before.regions.to_vec(),
            cold_transported: false,
            incoming: incoming_edges,
            source_boundaries,
        };
        certificate.valid(after, false)?;
        Some(certificate)
    }

    pub(super) fn valid(&self, view: &View<'_>, borrow: bool) -> Option<usize> {
        let (start, _, marker) = self.interval.valid_array(view, borrow)?;
        let mut authored_shape = self.region;
        authored_shape.start = self.authored_region.start;
        authored_shape.end = self.authored_region.end;
        if self.regions.len() != view.regions.len()
            || !same_manager_region(&authored_shape, &self.authored_region)
            || !self
                .regions
                .iter()
                .zip(view.regions)
                .all(|(a, b)| same_manager_region(a, b))
            || self.incoming != incoming(view, &self.interval.labels)
            || self.source_boundaries
                != boundaries_in(
                    &view.items[start..=marker],
                    &view.provenance.source_boundaries,
                )
            || stable_late_slots(view, start, marker, &self.interval.anchor).is_none()
        {
            return None;
        }
        let tail = guard(view, start, marker)?;
        let begin = *view.label_position(self.region.start)?;
        let end = *view.label_position(self.region.end)?;
        view.label_position(self.region.target)?;
        for (index, _) in &tail[..5] {
            let position = view.instruction_position(*index);
            if !(begin <= position && position < end) || view.owner(*index)? != self.interval.owner
            {
                return None;
            }
        }
        Some(marker)
    }

    pub(super) fn owner(&self) -> Owner {
        self.interval.owner
    }

    pub(super) fn rebind_cold_regions(
        &mut self,
        view: &View<'_>,
        retargets: &[(ExceptionRegion, ExceptionRegion)],
        groups: &[(ExceptionRegion, usize)],
    ) -> Option<()> {
        if self.valid(view, false).is_some() {
            return Some(());
        }
        if self.cold_transported {
            return None;
        }
        let (start, _, marker) = self.interval.valid_array(view, false)?;
        let tail = guard(view, start, marker)?;
        cold_region_replacements(
            &self.authored_regions,
            view.regions,
            retargets,
            Some(groups),
        )?;
        let mut owners = view.regions.iter().filter(|region| {
            let mut shape = **region;
            shape.start = self.region.start;
            shape.end = self.region.end;
            same_manager_region(&shape, &self.region)
                && tail[..5].iter().all(|(index, _)| {
                    let position = view.instruction_position(*index);
                    view.label_position(region.start)
                        .is_some_and(|start| *start <= position)
                        && view
                            .label_position(region.end)
                            .is_some_and(|end| position < *end)
                })
        });
        let selected = *owners.next()?;
        if owners.next().is_some() {
            return None;
        }
        self.region = selected;
        self.regions = view.regions.to_vec();
        self.cold_transported = true;
        self.valid(view, false)?;
        Some(())
    }
}

fn guard(view: &View<'_>, start: usize, marker: usize) -> Option<[(usize, Instruction); 6]> {
    let tail = view.items[start..=marker]
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(offset, item)| match item {
            Item::Instruction(instruction) => Some((start + offset, *instruction)),
            Item::Label(_) => None,
        })
        .take(6)
        .collect::<Vec<_>>();
    let [marker, condition, identity, common, copy, global] = tail.as_slice() else {
        return None;
    };
    let Operand::Value(argument) = global.1.operand() else {
        return None;
    };
    let Operand::Forward(target) = condition.1.operand() else {
        return None;
    };
    let Operand::Value(builtin) = common.1.operand() else {
        return None;
    };
    if ![
        CommonConstant::BUILTIN_TUPLE.argument(),
        CommonConstant::BUILTIN_ALL.argument(),
        CommonConstant::BUILTIN_ANY.argument(),
    ]
    .contains(&builtin)
    {
        return None;
    }
    let depth = global.1.depth_after()?;
    if argument & 1 != 0 || depth == 0 {
        return None;
    }
    let location = global.1.location;
    let expected = [
        (LOAD_GLOBAL, Operand::Value(argument), depth),
        (COPY, Operand::Value(1), depth.checked_add(1)?),
        (
            LOAD_COMMON_CONSTANT,
            Operand::Value(builtin),
            depth.checked_add(2)?,
        ),
        (IS_OP, Operand::Value(0), depth.checked_add(1)?),
        (POP_JUMP_IF_FALSE, Operand::Forward(target), depth),
        (NOT_TAKEN, Operand::Value(0), depth),
    ];
    for ((_, actual), (opcode, operand, depth)) in
        [global, copy, common, identity, condition, marker]
            .into_iter()
            .zip(expected)
    {
        let mut expected = Instruction::new(opcode, operand, location, Some(depth));
        if opcode == NOT_TAKEN {
            expected.set_exception_policy(ExceptionPolicy::NoneDynamicConditionStrict);
        }
        if *actual != expected {
            return None;
        }
    }
    key(condition.1, marker.1)?;
    Some([*global, *copy, *common, *identity, *condition, *marker])
}

fn stable_late_slots(view: &View<'_>, start: usize, end: usize, anchor: &Anchor) -> Option<usize> {
    let mut insertions = 0;
    let mut suffix = start;
    if start == 0 && matches!(anchor, Anchor::Entry) {
        let mut previous = None;
        let mut prefix = view.items[..=end]
            .iter()
            .enumerate()
            .filter(|(_, item)| !matches!(item, Item::Label(label) if !view.is_reference(*label)))
            .peekable();
        while let Some((index, Item::Instruction(instruction))) = prefix.peek().copied() {
            if instruction.opcode != MAKE_CELL {
                break;
            }
            let Operand::Value(cell) = instruction.operand() else {
                return None;
            };
            if previous.is_some_and(|previous| cell <= previous)
                || *instruction
                    != Instruction::new(
                        MAKE_CELL,
                        Operand::Value(cell),
                        SourceLocation::NONE,
                        Some(0),
                    )
            {
                return None;
            }
            previous = Some(cell);
            insertions += 1;
            suffix = index + 1;
            prefix.next();
        }
        if insertions > 0 {
            let (index, Item::Instruction(resume)) = prefix.next()? else {
                return None;
            };
            if *resume != Instruction::new(RESUME, Operand::Value(0), resume.location, Some(0))
                || view.regions.iter().any(|region| {
                    view.label_position(region.start)
                        .is_none_or(|start| *start <= view.instruction_position(index))
                })
            {
                return None;
            }
        }
    }
    if view.items[suffix..=end].iter().any(|item| matches!(item,
        Item::Instruction(instruction) if matches!(instruction.opcode, COPY_FREE_VARS | MAKE_CELL | RETURN_GENERATOR))) {
        return None;
    }
    for (label, location) in &view.provenance.setups {
        let &position = view.label_index(*label)?;
        if start <= position && position <= end {
            let previous = view.items[start..position]
                .iter()
                .rev()
                .find_map(|item| match item {
                    Item::Instruction(instruction) => Some(instruction),
                    Item::Label(_) => None,
                });
            if !location.is_some_and(|location| {
                previous.is_some_and(|instruction| {
                    instruction.opcode == NOP && instruction.location == location
                })
            }) {
                return None;
            }
        }
    }
    if view.items[start..=end].iter().any(|item| {
        matches!(item, Item::Label(label)
        if view.provenance.removed_pop_block_boundaries.contains(label)
            || view.provenance.removed_pop_block_widths.contains_key(label))
    }) {
        return None;
    }
    Some(insertions)
}

fn bounded_fusions(before: &[Item], after: &[Item], insertions: usize) -> bool {
    let Some(removed) = before.len().checked_sub(after.len()) else {
        return false;
    };
    // Entry cells are inserted after compaction, consuming retired slots before
    // the marker can reuse one of the five unchanged, equally protected guard cells.
    if !removed
        .checked_sub(insertions)
        .is_some_and(|slots| (1..=5).contains(&slots))
    {
        return false;
    }
    let (mut left, mut right, mut fusions) = (0, 0, 0);
    while left < before.len() && right < after.len() {
        if same_item(before[left], after[right], false) {
            left += 1;
            right += 1;
        } else if left + 1 < before.len()
            && one_fusion(&before[left..left + 2], &after[right..=right])
        {
            left += 2;
            right += 1;
            fusions += 1;
        } else {
            return false;
        }
    }
    left == before.len() && right == after.len() && fusions == removed
}
