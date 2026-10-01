use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    ExceptionOwnerDecision, ExceptionOwnerProvenance, OwnerLabelPositions, UniqueLabelPositions,
};
use crate::assembler::fusion_owner_slots::same_manager_region;
use crate::assembler::ir::EmissionStreamId;
use crate::assembler::{
    Assembler, EmittedInstruction, ExceptionPolicy, ExceptionRegion, ExceptionRegionPriority,
    Instruction, Item, Label, LayoutAssembler, Operand, OptimizationPipeline, SourceLocation,
    UnoptimizedCode,
};
use crate::target::opcodes::{
    FOR_ITER, GET_ITER, LOAD_CONST, NOP, POP_JUMP_IF_FALSE, POP_JUMP_IF_TRUE, RAISE_VARARGS,
};

/// The original NULL POP and omitted literal records in one normal-finally array.
#[derive(Debug)]
pub(in crate::assembler) struct FoldedNormalFinallyNop {
    source: Option<EmittedInstruction>,
    guard_source: EmittedInstruction,
    entry: Label,
    prefix: Vec<Item>,
    // The following FOR_ITER block is separate and gains a boundary during preparation.
    boundary_roles: Vec<(Label, bool, bool)>,
    pop_at: usize,
    load_at: usize,
    iterator_at: usize,
    guard: Instruction,
    header: Instruction,
    owner: Option<OriginalOwner>,
}

#[derive(Debug)]
struct OriginalOwner {
    region: ExceptionRegion,
    setup: Label,
    endpoints: Vec<(Label, Vec<Label>)>,
    overlaps: Vec<ExceptionRegion>,
    roles: Vec<[bool; 3]>,
}

fn aliases(items: &[Item], at: usize) -> (usize, Vec<Label>) {
    let mut begin = at;
    while begin > 0 && matches!(items[begin - 1], Item::Label(_)) {
        begin -= 1;
    }
    let mut end = at;
    while matches!(items.get(end), Some(Item::Label(_))) {
        end += 1;
    }
    let labels = items[begin..end]
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) => Some(*label),
            Item::Instruction(_) => None,
        })
        .collect();
    (begin, labels)
}

fn record(items: &[Item], at: usize) -> Option<Instruction> {
    match items.get(at)? {
        Item::Instruction(instruction) => Some(*instruction),
        Item::Label(_) => None,
    }
}

fn unique_record(items: &[Item], expected: Instruction) -> bool {
    items
        .iter()
        .filter(|item| **item == Item::Instruction(expected))
        .count()
        == 1
}

fn regions_valid(regions: &[ExceptionRegion], labels: &UniqueLabelPositions) -> bool {
    regions.iter().all(|region| {
        [region.start, region.end, region.target]
            .into_iter()
            .all(|label| labels.get(label).is_some())
            && match region.priority {
                ExceptionRegionPriority::OriginalRange(start, end) => {
                    labels.get(start).is_some() && labels.get(end).is_some()
                }
                ExceptionRegionPriority::Resolved { .. } => true,
            }
    })
}

fn overlapping_regions(
    regions: &[ExceptionRegion],
    start: usize,
    end: usize,
    positions: &OwnerLabelPositions,
) -> Vec<ExceptionRegion> {
    regions
        .iter()
        .filter(|region| positions[&region.start] < end && positions[&region.end] > start)
        .copied()
        .collect()
}

fn positions(items: &[Item], labels: &UniqueLabelPositions) -> Option<OwnerLabelPositions> {
    let mut positions = OwnerLabelPositions::new(items.len());
    for item in items {
        if let Item::Label(label) = item {
            positions.insert(*label, labels.get(*label)?.1);
        }
    }
    Some(positions)
}

fn region_roles(
    region: &ExceptionRegion,
    pop: usize,
    positions: &OwnerLabelPositions,
) -> [bool; 3] {
    std::array::from_fn(|offset| {
        positions[&region.start] <= pop + offset && pop + offset < positions[&region.end]
    })
}

fn endpoints(
    items: &[Item],
    unique: &UniqueLabelPositions,
    regions: &[ExceptionRegion],
    setup: Label,
) -> Option<Vec<(Label, Vec<Label>)>> {
    let mut endpoints = vec![(setup, aliases(items, unique.get(setup)?.0).1)];
    for region in regions {
        let mut labels = vec![region.start, region.end, region.target];
        if let ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
            labels.extend([start, end]);
        }
        for label in labels {
            if !endpoints.iter().any(|(original, _)| *original == label) {
                endpoints.push((label, aliases(items, unique.get(label)?.0).1));
            }
        }
    }
    Some(endpoints)
}

fn authorized_region_rebuild(
    before: &[ExceptionRegion],
    after: &[ExceptionRegion],
    retargets: &[(ExceptionRegion, ExceptionRegion)],
    copied: bool,
) -> bool {
    if before.iter().any(|region| {
        before
            .iter()
            .filter(|other| same_manager_region(region, other))
            .count()
            != 1
    }) || retargets.iter().any(|(original, rebuilt)| {
        let mut shape = *rebuilt;
        shape.start = original.start;
        shape.end = original.end;
        !before
            .iter()
            .any(|region| same_manager_region(region, original))
            || !same_manager_region(&shape, original)
    }) {
        return false;
    }
    let mut expected = Vec::new();
    for original in before {
        let previous = expected.len();
        expected.extend(retargets.iter().filter_map(|(region, rebuilt)| {
            same_manager_region(region, original).then_some(*rebuilt)
        }));
        if expected.len() == previous {
            expected.push(*original);
        }
    }
    expected.len() <= after.len()
        && (copied || expected.len() == after.len())
        && expected
            .iter()
            .zip(after)
            .all(|(a, b)| same_manager_region(a, b))
        && expected.iter().all(|region| {
            after
                .iter()
                .filter(|other| same_manager_region(region, other))
                .count()
                == 1
        })
}

impl FoldedNormalFinallyNop {
    fn pop(&self) -> Instruction {
        record(&self.prefix, self.pop_at).unwrap()
    }

    fn load(&self) -> Instruction {
        record(&self.prefix, self.load_at).unwrap()
    }

    fn metadata_matches(&self, provenance: &ExceptionOwnerProvenance) -> bool {
        let load = self.load();
        provenance.source_boundaries.contains(&self.entry)
            && provenance.block_boundaries.contains(&self.entry)
            && provenance.uniform_fold_owner_slots.contains(&(
                load.location,
                load.opcode,
                load.local_policy.hidden_exception_owner_slots(),
            ))
            && self.boundary_roles.iter().all(|(label, source, block)| {
                provenance.source_boundaries.contains(label) == *source
                    && provenance.block_boundaries.contains(label) == *block
            })
            && self.prefix.iter().enumerate().all(|(index, item)| {
                let Item::Label(label) = item else {
                    return true;
                };
                !provenance.fusion_boundaries.contains(label)
                    && !provenance.setup_nop_boundaries.contains(label)
                    && !provenance.omitted_nop_counts.contains_key(label)
                    && !provenance.omitted_nop_locations.contains_key(label)
                    && (index < self.pop_at
                        || index > self.iterator_at
                        || (!provenance.source_boundaries.contains(label)
                            && !provenance.block_boundaries.contains(label)))
            })
    }

    fn has_handler_entry(&self, regions: &[ExceptionRegion]) -> bool {
        regions
            .iter()
            .any(|region| self.prefix.contains(&Item::Label(region.target)))
    }

    fn matches_prefix(
        &self,
        items: &[Item],
        labels: &UniqueLabelPositions,
        located: bool,
    ) -> Option<(usize, usize)> {
        let Item::Label(entry) = *self.prefix.first()? else {
            return None;
        };
        let (start, instruction_start) = labels.get(entry)?;
        let current = items.get(start..start.checked_add(self.prefix.len())?)?;
        if !matches!(items.get(start.wrapping_sub(1)), Some(Item::Instruction(previous)) if previous.opcode == RAISE_VARARGS)
            || record(items, start + self.prefix.len()) != Some(self.header)
        {
            return None;
        }
        for (offset, (actual, original)) in current.iter().zip(&self.prefix).enumerate() {
            let mut expected = *original;
            if offset == self.pop_at && located {
                let Item::Instruction(pop) = &mut expected else {
                    return None;
                };
                pop.location = self.guard.location;
            }
            if *actual != expected
                || matches!(actual, Item::Label(label) if labels.get(*label).map(|p| p.0) != Some(start + offset))
            {
                return None;
            }
        }
        let head: FxHashSet<_> = self.prefix[..self.pop_at]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect();
        let inner: FxHashSet<_> = self.prefix[self.pop_at + 1..=self.iterator_at]
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .collect();
        let mut incoming = items.iter().filter_map(|item| match item {
            Item::Instruction(jump) if matches!(jump.operand(), Operand::Forward(target) | Operand::Backward(target) if head.contains(&target) || inner.contains(&target)) => Some(*jump),
            _ => None,
        });
        let mut expected_pop = self.pop();
        if located {
            expected_pop.location = self.guard.location;
        }
        if incoming.next() != Some(self.guard)
            || incoming.next().is_some()
            || !unique_record(items, self.guard)
            || !unique_record(items, self.load())
            || !unique_record(items, expected_pop)
            || !unique_record(items, record(&self.prefix, self.iterator_at)?)
            || !unique_record(items, self.header)
        {
            return None;
        }
        Some((start, instruction_start))
    }

    fn live_owner(
        &self,
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        labels: &UniqueLabelPositions,
        positions: &OwnerLabelPositions,
        located: bool,
    ) -> Option<(usize, usize)> {
        let owner = self.owner.as_ref()?;
        let (_, pop) = self.matches_prefix(items, labels, located)?;
        if !regions_valid(regions, labels)
            || !self.metadata_matches(provenance)
            || self.has_handler_entry(regions)
            || !provenance.synchronous_with_setups.contains(&owner.setup)
            || owner.endpoints.iter().any(|(label, expected)| {
                labels
                    .get(*label)
                    .is_none_or(|(at, _)| aliases(items, at).1 != *expected)
            })
        {
            return None;
        }
        let index =
            LayoutAssembler::uniform_exception_owner_in(regions, pop + 1, pop + 3, positions)?;
        let overlaps = overlapping_regions(regions, pop, pop + 3, positions);
        if !same_manager_region(&regions[index], &owner.region)
            || overlaps.len() != owner.overlaps.len()
            || overlaps
                .iter()
                .zip(&owner.overlaps)
                .any(|(current, original)| !same_manager_region(current, original))
            || overlaps
                .iter()
                .zip(&owner.roles)
                .any(|(region, roles)| region_roles(region, pop, positions) != *roles)
            || owner.roles.len() != overlaps.len()
            || overlaps.iter().any(|region| {
                regions
                    .iter()
                    .filter(|current| same_manager_region(current, region))
                    .count()
                    != 1
            })
        {
            return None;
        }
        Some((pop, index))
    }

    fn transport_regions(
        &mut self,
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        retargets: &[(ExceptionRegion, ExceptionRegion)],
        labels: &UniqueLabelPositions,
        positions: &OwnerLabelPositions,
    ) -> Option<()> {
        let owner = self.owner.as_ref()?;
        let (_, pop) = self.matches_prefix(items, labels, true)?;
        if !regions_valid(regions, labels)
            || !self.metadata_matches(provenance)
            || self.has_handler_entry(regions)
            || !provenance.synchronous_with_setups.contains(&owner.setup)
        {
            return None;
        }
        // Only range endpoints are rewritten by these passes. The setup and every
        // handler retain their complete original alias groups.
        for label in
            std::iter::once(owner.setup).chain(owner.overlaps.iter().map(|region| region.target))
        {
            let expected = &owner
                .endpoints
                .iter()
                .find(|(original, _)| *original == label)?
                .1;
            if aliases(items, labels.get(label)?.0).1 != *expected {
                return None;
            }
        }
        let mut transferred = Vec::new();
        let mut selected = Vec::new();
        for (original, expected_roles) in owner.overlaps.iter().zip(&owner.roles) {
            let mut group = retargets
                .iter()
                .filter_map(|(before, after)| {
                    same_manager_region(before, original).then_some(*after)
                })
                .collect::<Vec<_>>();
            if group.is_empty() {
                group.push(*original);
            }
            let mut actual_roles = [false; 3];
            for rebuilt in &group {
                let mut shape = *rebuilt;
                shape.start = original.start;
                shape.end = original.end;
                if !same_manager_region(&shape, original)
                    || regions
                        .iter()
                        .filter(|current| same_manager_region(current, rebuilt))
                        .count()
                        != 1
                    || group
                        .iter()
                        .filter(|current| same_manager_region(current, rebuilt))
                        .count()
                        != 1
                {
                    return None;
                }
                for (role, covered) in actual_roles
                    .iter_mut()
                    .zip(region_roles(rebuilt, pop, positions))
                {
                    if *role && covered {
                        return None;
                    }
                    *role |= covered;
                }
            }
            if actual_roles != *expected_roles {
                return None;
            }
            if same_manager_region(original, &owner.region) {
                selected.clone_from(&group);
            }
            transferred.extend(group.into_iter().filter(|region| {
                region_roles(region, pop, positions)
                    .into_iter()
                    .any(|covered| covered)
            }));
        }
        let overlaps = overlapping_regions(regions, pop, pop + 3, positions);
        if overlaps.len() != transferred.len()
            || overlaps
                .iter()
                .zip(&transferred)
                .any(|(current, expected)| !same_manager_region(current, expected))
        {
            return None;
        }
        let index =
            LayoutAssembler::uniform_exception_owner_in(regions, pop + 1, pop + 3, positions)?;
        if !selected
            .iter()
            .any(|region| same_manager_region(region, &regions[index]))
        {
            return None;
        }
        let endpoints = endpoints(items, labels, &overlaps, owner.setup)?;
        self.owner = Some(OriginalOwner {
            region: regions[index],
            setup: owner.setup,
            endpoints,
            roles: overlaps
                .iter()
                .map(|region| region_roles(region, pop, positions))
                .collect(),
            overlaps,
        });
        self.live_owner(items, regions, provenance, labels, positions, true)?;
        Some(())
    }

    pub(in crate::assembler) fn resolve_priority(
        &mut self,
        positions: &FxHashMap<Label, usize>,
    ) -> bool {
        let Some(owner) = &mut self.owner else {
            return true;
        };
        for region in std::iter::once(&mut owner.region).chain(&mut owner.overlaps) {
            if let ExceptionRegionPriority::OriginalRange(start, end) = region.priority {
                let (Some(&start), Some(&end)) = (positions.get(&start), positions.get(&end))
                else {
                    return false;
                };
                region.priority = ExceptionRegionPriority::Resolved { start, end };
            }
        }
        true
    }

    pub(in crate::assembler) fn resolve_operand(
        &mut self,
        stream: EmissionStreamId,
        index: usize,
        old: Instruction,
        new: Instruction,
    ) {
        let Some(source) = self.source else { return };
        let Some(begin) = (source.item_index as usize).checked_sub(self.pop_at) else {
            return;
        };
        if source.stream == stream && index == begin + self.load_at && self.load() == old {
            self.prefix[self.load_at] = Item::Instruction(new);
        }
    }

    fn remap_constant(&mut self, indices: &[Option<u32>]) -> bool {
        let mut load = self.load();
        let Operand::Value(index) = load.operand() else {
            return false;
        };
        let Some(Some(mapped)) = indices.get(index as usize) else {
            return false;
        };
        load.set_operand(Operand::Value(*mapped));
        self.prefix[self.load_at] = Item::Instruction(load);
        true
    }
}

impl Assembler {
    /// Captures the exact normal POP prefix before any exceptional finalizer copy.
    pub(crate) fn record_folded_normal_finally_nop(
        &mut self,
        ordinal: usize,
        protected_end: Label,
        location: SourceLocation,
        count: u16,
    ) {
        if self.python_version < crate::PythonVersion::Py314 || !(1..=3).contains(&count) {
            return;
        }
        let items = self.linear_items();
        let Some(pop_index) = self.instruction_item_index(ordinal) else {
            return;
        };
        let Some(pop) = record(items, pop_index) else {
            return;
        };
        let Some(depth) = pop.depth_after() else {
            return;
        };
        let mut expected_pop =
            Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, Some(depth));
        expected_pop.fusion_policy.mark_converted_pop_block();
        expected_pop.set_exception_policy(pop.exception_policy());
        if pop != expected_pop
            || !matches!(
                pop.exception_policy(),
                ExceptionPolicy::EXCLUDE_ALWAYS | ExceptionPolicy::AlwaysAbsent
            )
        {
            return;
        }
        let (begin, head) = aliases(items, pop_index);
        let entries = head
            .iter()
            .filter(|label| {
                self.exception_owner_provenance
                    .source_boundaries
                    .contains(label)
                    && self
                        .exception_owner_provenance
                        .block_boundaries
                        .contains(label)
            })
            .copied()
            .collect::<Vec<_>>();
        let [entry] = entries.as_slice() else { return };
        if !head.contains(&protected_end) {
            return;
        }
        let following = items[pop_index + 1..]
            .iter()
            .enumerate()
            .filter_map(|(offset, item)| {
                matches!(item, Item::Instruction(_)).then_some(pop_index + 1 + offset)
            })
            .take(3)
            .collect::<Vec<_>>();
        let [load_index, iterator_index, header_index] = following.as_slice() else {
            return;
        };
        let (Some(load), Some(iterator), Some(header)) = (
            record(items, *load_index),
            record(items, *iterator_index),
            record(items, *header_index),
        ) else {
            return;
        };
        let mut expected_load =
            Instruction::new(LOAD_CONST, load.operand(), location, Some(depth + 1));
        let iterator_depth =
            depth + 1 + u32::from(self.python_version == crate::PythonVersion::Py315);
        expected_load
            .local_policy
            .add_hidden_exception_owner_slots(count);
        expected_load.add_pre_fold_extra_instructions(u8::try_from(count).unwrap());
        if load != expected_load
            || !matches!(load.operand(), Operand::Value(_) | Operand::Deferred(_))
            || !self
                .exception_owner_provenance
                .uniform_fold_owner_slots
                .contains(&(location, LOAD_CONST, count))
            || iterator
                != Instruction::new(GET_ITER, Operand::Value(0), location, Some(iterator_depth))
            || header.opcode != FOR_ITER
            || !matches!(header.operand(), Operand::Forward(_))
            || header.location != location
            || header
                != Instruction::new(
                    FOR_ITER,
                    header.operand(),
                    location,
                    Some(iterator_depth + 1),
                )
            || *header_index == iterator_index + 1
        {
            return;
        }
        let labels = UniqueLabelPositions::new(items);
        let mut incoming = items.iter().enumerate().filter_map(|(index, item)| match item {
            Item::Instruction(jump) if matches!(jump.operand(), Operand::Forward(target) | Operand::Backward(target) if head.contains(&target)) => Some((index, *jump)),
            _ => None,
        });
        let Some((guard_index, guard)) = incoming.next() else {
            return;
        };
        if incoming.next().is_some()
            || guard_index >= begin
            || !matches!(guard.opcode, POP_JUMP_IF_FALSE | POP_JUMP_IF_TRUE)
            || !matches!(guard.operand(), Operand::Forward(_))
            || guard.location.line < 0
            || guard != Instruction::new(guard.opcode, guard.operand(), guard.location, Some(depth))
        {
            return;
        }
        let origin = FoldedNormalFinallyNop {
            source: Some(EmittedInstruction {
                stream: self.emission_stream,
                item_index: u32::try_from(pop_index).unwrap(),
            }),
            guard_source: EmittedInstruction {
                stream: self.emission_stream,
                item_index: u32::try_from(guard_index).unwrap(),
            },
            entry: *entry,
            prefix: items[begin..*header_index].to_vec(),
            boundary_roles: items[begin..=*iterator_index]
                .iter()
                .filter_map(|item| {
                    let Item::Label(label) = item else {
                        return None;
                    };
                    Some((
                        *label,
                        self.exception_owner_provenance
                            .source_boundaries
                            .contains(label),
                        self.exception_owner_provenance
                            .block_boundaries
                            .contains(label),
                    ))
                })
                .collect(),
            pop_at: pop_index - begin,
            load_at: load_index - begin,
            iterator_at: iterator_index - begin,
            guard,
            header,
            owner: None,
        };
        if origin.metadata_matches(&self.exception_owner_provenance)
            && origin.matches_prefix(items, &labels, false).is_some()
        {
            self.exception_owner_provenance
                .folded_normal_finally_nops
                .push(origin);
        }
    }

    /// Seals the original region before graph preparation can move or copy records.
    pub(in crate::assembler) fn bind_folded_normal_finally_nops(&mut self) {
        if self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .iter()
            .all(|origin| origin.source.is_none())
        {
            return;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            for origin in &mut self.exception_owner_provenance.folded_normal_finally_nops {
                origin.source = None;
                origin.owner = None;
            }
            return;
        };
        let unique = UniqueLabelPositions::new(items);
        let mut positions = OwnerLabelPositions::new(items.len());
        for item in items {
            if let Item::Label(label) = item {
                let Some((_, index)) = unique.get(*label) else {
                    for origin in &mut self.exception_owner_provenance.folded_normal_finally_nops {
                        origin.source = None;
                        origin.owner = None;
                    }
                    return;
                };
                positions.insert(*label, index);
            }
        }
        let bound = self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .iter()
            .map(|origin| {
                let source = origin.source?;
                if !regions_valid(&self.exception_regions, &unique)
                    || !origin.metadata_matches(&self.exception_owner_provenance)
                    || origin.has_handler_entry(&self.exception_regions)
                {
                    return None;
                }
                let (begin, pop) = origin.matches_prefix(items, &unique, false)?;
                if source.stream != self.emission_stream
                    || source.item_index as usize != begin + origin.pop_at
                    || origin.guard_source.stream != self.emission_stream
                    || record(items, origin.guard_source.item_index as usize) != Some(origin.guard)
                {
                    return None;
                }
                let owner_index = LayoutAssembler::uniform_exception_owner_in(
                    &self.exception_regions,
                    pop + 1,
                    pop + 3,
                    &positions,
                )?;
                let region = self.exception_regions[owner_index];
                let ExceptionRegionPriority::OriginalRange(setup, _) = region.priority else {
                    return None;
                };
                if region.is_scope_fallback
                    || !region.preserve_lasti
                    || region.depth != origin.pop().depth_after().unwrap()
                    || !self
                        .exception_owner_provenance
                        .synchronous_with_setups
                        .contains(&setup)
                {
                    return None;
                }
                if self
                    .exception_regions
                    .iter()
                    .filter(|current| same_manager_region(current, &region))
                    .count()
                    != 1
                {
                    return None;
                }
                let overlaps =
                    overlapping_regions(&self.exception_regions, pop, pop + 3, &positions);
                let endpoints = endpoints(items, &unique, &overlaps, setup)?;
                let roles = overlaps
                    .iter()
                    .map(|region| region_roles(region, pop, &positions))
                    .collect();
                Some(OriginalOwner {
                    region,
                    setup,
                    endpoints,
                    overlaps,
                    roles,
                })
            })
            .collect::<Vec<_>>();
        for (origin, owner) in self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .iter_mut()
            .zip(bound)
        {
            if origin.source.take().is_some() {
                origin.owner = owner;
            }
        }
    }

    pub(in crate::assembler) fn remap_folded_normal_finally_nops(
        &mut self,
        indices: &[Option<u32>],
    ) {
        if self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .is_empty()
        {
            return;
        }
        let items: std::borrow::Cow<'_, [Item]> = match &self.code {
            UnoptimizedCode::Linear(items) => std::borrow::Cow::Borrowed(items),
            UnoptimizedCode::Prepared { graph, .. } => std::borrow::Cow::Owned(graph.copy_items()),
        };
        let labels = UniqueLabelPositions::new(&items);
        let mut origins =
            std::mem::take(&mut self.exception_owner_provenance.folded_normal_finally_nops);
        origins.retain_mut(|origin| {
            origin.matches_prefix(&items, &labels, false).is_some()
                && origin.metadata_matches(&self.exception_owner_provenance)
                && origin.remap_constant(indices)
        });
        self.exception_owner_provenance.folded_normal_finally_nops = origins;
    }
}

impl OptimizationPipeline {
    pub(in crate::assembler) fn remap_folded_normal_finally_constants(
        &mut self,
        indices: &[Option<u32>],
    ) {
        if self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .is_empty()
        {
            return;
        }
        let items = self.graph.copy_items();
        let labels = UniqueLabelPositions::new(&items);
        let Some(positions) = positions(&items, &labels) else {
            self.exception_owner_provenance
                .folded_normal_finally_nops
                .clear();
            return;
        };
        let mut origins =
            std::mem::take(&mut self.exception_owner_provenance.folded_normal_finally_nops);
        origins.retain_mut(|origin| {
            origin
                .live_owner(
                    &items,
                    &self.exception_regions,
                    &self.exception_owner_provenance,
                    &labels,
                    &positions,
                    true,
                )
                .is_some()
                && origin.remap_constant(indices)
        });
        self.exception_owner_provenance.folded_normal_finally_nops = origins;
    }

    pub(in crate::assembler) fn retain_folded_normal_finally_nops(
        &mut self,
        located: bool,
    ) -> Option<Vec<ExceptionRegion>> {
        if self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .is_empty()
        {
            return None;
        }
        let items = self.graph.copy_items();
        let labels = UniqueLabelPositions::new(&items);
        let Some(positions) = positions(&items, &labels) else {
            self.exception_owner_provenance
                .folded_normal_finally_nops
                .clear();
            return None;
        };
        let mut origins =
            std::mem::take(&mut self.exception_owner_provenance.folded_normal_finally_nops);
        origins.retain(|origin| {
            origin
                .live_owner(
                    &items,
                    &self.exception_regions,
                    &self.exception_owner_provenance,
                    &labels,
                    &positions,
                    located,
                )
                .is_some()
        });
        self.exception_owner_provenance.folded_normal_finally_nops = origins;
        (!self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .is_empty())
        .then(|| self.exception_regions.clone())
    }

    pub(in crate::assembler) fn transport_folded_normal_finally_nops(
        &mut self,
        before: Option<&[ExceptionRegion]>,
        retargets: &[(ExceptionRegion, ExceptionRegion)],
        copied: bool,
    ) {
        if self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .is_empty()
        {
            return;
        }
        if before.is_none_or(|before| {
            !authorized_region_rebuild(before, &self.exception_regions, retargets, copied)
        }) {
            self.exception_owner_provenance
                .folded_normal_finally_nops
                .clear();
            return;
        }
        let items = self.graph.copy_items();
        let labels = UniqueLabelPositions::new(&items);
        let Some(positions) = positions(&items, &labels) else {
            self.exception_owner_provenance
                .folded_normal_finally_nops
                .clear();
            return;
        };
        let mut origins =
            std::mem::take(&mut self.exception_owner_provenance.folded_normal_finally_nops);
        origins.retain_mut(|origin| {
            origin
                .transport_regions(
                    &items,
                    &self.exception_regions,
                    &self.exception_owner_provenance,
                    retargets,
                    &labels,
                    &positions,
                )
                .is_some()
        });
        self.exception_owner_provenance.folded_normal_finally_nops = origins;
    }
}

impl LayoutAssembler {
    pub(super) fn folded_normal_finally_nop_owners(
        &self,
        positions: &OwnerLabelPositions,
    ) -> FxHashMap<usize, ExceptionOwnerDecision> {
        let mut result = FxHashMap::default();
        if self
            .exception_owner_provenance
            .folded_normal_finally_nops
            .is_empty()
        {
            return result;
        }
        let labels = UniqueLabelPositions::new(&self.items);
        if !regions_valid(&self.exception_regions, &labels) {
            return result;
        }
        for origin in &self.exception_owner_provenance.folded_normal_finally_nops {
            let Some((pop, index)) = origin.live_owner(
                &self.items,
                &self.exception_regions,
                &self.exception_owner_provenance,
                &labels,
                positions,
                true,
            ) else {
                continue;
            };
            result.insert(
                pop,
                ExceptionOwnerDecision::override_with_region(Some(index)),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::opcodes::{JUMP_FORWARD, LOAD_FAST, PUSH_EXC_INFO, RERAISE, RETURN_VALUE};

    fn fixture() -> Assembler {
        fixture_with_constant(false)
    }

    fn fixture_with_constant(deferred: bool) -> Assembler {
        let mut assembler = Assembler::with_python_version(crate::PythonVersion::Py314);
        let setup = assembler.label();
        let entry = assembler.label();
        let protected_end = assembler.label();
        let header = assembler.label();
        let end = assembler.label();
        let handler = assembler.label();
        let done = assembler.label();
        assembler.record_synchronous_with_exception_setup(setup);
        assembler.mark(setup);
        assembler.set_location(SourceLocation::new(3, 3, 7, 11));
        assembler.emit(LOAD_FAST, 0);
        assembler.emit_forward_with_depth(POP_JUMP_IF_TRUE, entry, 2, ExceptionPolicy::default());
        assembler.emit(RAISE_VARARGS, 1);
        assembler.record_exception_owner_block_boundary(entry);
        assembler.mark(entry);
        assembler.mark(protected_end);
        assembler.set_location(SourceLocation::NONE);
        let ordinal = assembler.instruction_count();
        assembler.emit_with_depth(NOP, 0, 2, ExceptionPolicy::AlwaysAbsent);
        assembler.mark_last_as_converted_pop_block();
        let location = SourceLocation::new(5, 5, 17, 23);
        assembler.set_location(location);
        let patch = if deferred {
            Some(assembler.emit_placeholder_with_depth(LOAD_CONST, 3))
        } else {
            assembler.emit_with_depth(LOAD_CONST, 1, 3, ExceptionPolicy::default());
            None
        };
        assembler.add_last_constant_fold_slots(2);
        assembler.record_last_uniform_fold_owner_slots(2);
        assembler.emit_with_depth(GET_ITER, 0, 3, ExceptionPolicy::default());
        assembler.mark(header);
        assembler.emit_forward_with_depth(FOR_ITER, end, 4, ExceptionPolicy::default());
        assembler.emit_with_depth(RETURN_VALUE, 0, 0, ExceptionPolicy::default());
        assembler.mark(end);
        assembler.emit_forward_with_depth(JUMP_FORWARD, done, 0, ExceptionPolicy::default());
        assembler.record_folded_normal_finally_nop(ordinal, protected_end, location, 2);
        assembler.mark(handler);
        assembler.emit(PUSH_EXC_INFO, 0);
        assembler.emit(RERAISE, 1);
        assembler.mark(done);
        assembler.emit(RETURN_VALUE, 0);
        assembler.add_exception_region_with_origin(setup, end, handler, 2, true, (setup, end));
        if let Some(patch) = patch {
            assembler.patch_argument(patch, 1).unwrap();
            assembler.resolve_operand_patches().unwrap();
        }
        assembler
    }

    fn layout(mut assembler: Assembler) -> LayoutAssembler {
        let origin = &assembler
            .exception_owner_provenance
            .folded_normal_finally_nops[0];
        let expected = origin.pop();
        let location = origin.guard.location;
        for item in assembler.linear_items_mut() {
            if let Item::Instruction(pop) = item
                && *pop == expected
            {
                pop.location = location;
            }
        }
        LayoutAssembler {
            python_version: crate::PythonVersion::Py314,
            legacy_uniform_jump_seed: false,
            items: assembler.linear_items().to_vec(),
            exception_regions: assembler.exception_regions,
            exception_owner_provenance: assembler.exception_owner_provenance,
            preserved_block_boundaries: assembler.preserved_block_boundaries,
            removed_max_depth: None,
            next_label: assembler.next_label,
            has_next_location: false,
        }
    }

    fn owners(layout: &LayoutAssembler) -> FxHashMap<usize, ExceptionOwnerDecision> {
        let mut positions = OwnerLabelPositions::new(layout.items.len());
        let mut index = 0;
        for item in &layout.items {
            match item {
                Item::Label(label) => positions.insert(*label, index),
                Item::Instruction(_) => index += 1,
            }
        }
        layout.folded_normal_finally_nop_owners(&positions)
    }

    #[test]
    fn folded_normal_pop_replays_only_the_original_owned_literal_cell() {
        let mut assembler = fixture();
        assert_eq!(
            assembler
                .exception_owner_provenance
                .folded_normal_finally_nops
                .len(),
            1
        );
        assembler.bind_folded_normal_finally_nops();
        assert!(
            assembler
                .exception_owner_provenance
                .folded_normal_finally_nops[0]
                .owner
                .is_some()
        );
        let layout = layout(assembler);
        let owners = owners(&layout);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners.values().next().unwrap().source_region(), Some(0));
    }

    #[test]
    fn folded_normal_pop_rejects_changed_sources_and_boundaries() {
        for mutation in [
            "stream",
            "handle",
            "guard handle",
            "pop",
            "load",
            "iterator",
            "header",
            "guard",
            "pop policy",
            "load policy",
            "pop hidden",
            "load folded",
            "aliases",
            "duplicate label",
            "duplicate pop",
            "duplicate load",
            "duplicate iterator",
            "duplicate guard",
            "foreign edge",
            "uniform witness",
            "source boundary",
            "foreign head boundary",
            "foreign handler",
            "fusion boundary",
            "owner depth",
            "owner priority",
            "owner target",
            "duplicate region",
        ] {
            let mut assembler = fixture();
            let origin = &assembler
                .exception_owner_provenance
                .folded_normal_finally_nops[0];
            let pop = origin.source.unwrap().item_index as usize;
            let guard = origin.guard_source.item_index as usize;
            let entry = origin.entry;
            match mutation {
                "stream" => assembler.emission_stream = EmissionStreamId::next(),
                "handle" => {
                    assembler
                        .exception_owner_provenance
                        .folded_normal_finally_nops[0]
                        .source
                        .as_mut()
                        .unwrap()
                        .item_index += 1;
                }
                "guard handle" => {
                    assembler
                        .exception_owner_provenance
                        .folded_normal_finally_nops[0]
                        .guard_source
                        .item_index += 1;
                }
                "aliases" => assembler.linear_items_mut().swap(pop - 1, pop - 2),
                "duplicate label" => assembler.push_linear_item(Item::Label(entry)),
                "duplicate pop" | "duplicate load" | "duplicate iterator" | "duplicate guard" => {
                    let index = match mutation {
                        "duplicate pop" => pop,
                        "duplicate load" => pop + 1,
                        "duplicate iterator" => pop + 2,
                        _ => guard,
                    };
                    let instruction = assembler.linear_items()[index];
                    assembler.push_linear_item(instruction);
                }
                "foreign edge" => {
                    assembler.emit_forward_with_depth(
                        JUMP_FORWARD,
                        entry,
                        2,
                        ExceptionPolicy::default(),
                    );
                }
                "uniform witness" => assembler
                    .exception_owner_provenance
                    .uniform_fold_owner_slots
                    .clear(),
                "source boundary" => {
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&entry);
                }
                "foreign head boundary" => {
                    let Item::Label(label) = assembler.linear_items()[pop - 1] else {
                        unreachable!()
                    };
                    assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .insert(label);
                }
                "foreign handler" => {
                    let region = assembler.exception_regions[0];
                    assembler.exception_regions.push(ExceptionRegion {
                        start: region.end,
                        target: entry,
                        ..region
                    });
                }
                "fusion boundary" => {
                    assembler
                        .exception_owner_provenance
                        .fusion_boundaries
                        .insert(entry);
                }
                "owner depth" => assembler.exception_regions[0].depth += 1,
                "owner priority" => {
                    assembler.exception_regions[0].priority =
                        ExceptionRegionPriority::OriginalRange(entry, entry);
                }
                "owner target" => assembler.exception_regions[0].target = Label::from_u32(1000),
                "duplicate region" => assembler
                    .exception_regions
                    .push(assembler.exception_regions[0]),
                _ => {
                    let index = match mutation {
                        "pop" | "pop policy" | "pop hidden" => pop,
                        "load" | "load policy" | "load folded" => pop + 1,
                        "iterator" => pop + 2,
                        "header" => pop + 4,
                        "guard" => guard,
                        _ => unreachable!(),
                    };
                    let Item::Instruction(instruction) = &mut assembler.linear_items_mut()[index]
                    else {
                        unreachable!()
                    };
                    match mutation {
                        "pop policy" | "load policy" => {
                            instruction.no_location_policy.allow_block_inlining();
                        }
                        "pop hidden" => {
                            instruction.local_policy.add_hidden_exception_owner_slots(1);
                        }
                        "load folded" => instruction.add_pre_fold_extra_instructions(1),
                        _ => instruction.location = SourceLocation::new(40, 40, 1, 2),
                    }
                }
            }
            assembler.bind_folded_normal_finally_nops();
            let origin = &assembler
                .exception_owner_provenance
                .folded_normal_finally_nops[0];
            assert!(origin.source.is_none(), "{mutation}");
            assert!(origin.owner.is_none(), "{mutation}");
            assembler.bind_folded_normal_finally_nops();
            assert!(
                assembler
                    .exception_owner_provenance
                    .folded_normal_finally_nops[0]
                    .owner
                    .is_none(),
                "{mutation}"
            );
        }
    }

    #[test]
    fn folded_normal_pop_constant_transport_and_prepared_reentry_are_exact() {
        for deferred in [false, true] {
            let mut assembler = fixture_with_constant(deferred);
            assembler.used_constant_indices(2);
            assert!(matches!(assembler.code, UnoptimizedCode::Prepared { .. }));
            assert!(
                assembler
                    .exception_owner_provenance
                    .folded_normal_finally_nops[0]
                    .owner
                    .is_some()
            );
            assembler.used_constant_indices(2);
            assembler.remap_constant_indices(&[Some(0), Some(2)]);
            assert_eq!(
                assembler
                    .exception_owner_provenance
                    .folded_normal_finally_nops[0]
                    .load()
                    .operand(),
                Operand::Value(2)
            );
            let UnoptimizedCode::Prepared { graph, .. } = &assembler.code else {
                unreachable!()
            };
            assembler.code = UnoptimizedCode::Linear(graph.copy_items());
            assert_eq!(owners(&layout(assembler)).len(), 1);
        }
        let mut removed = fixture();
        removed.remap_constant_indices(&[Some(0), None]);
        assert!(
            removed
                .exception_owner_provenance
                .folded_normal_finally_nops
                .is_empty()
        );
        let mut stale = fixture();
        let items = std::mem::replace(&mut stale.code, UnoptimizedCode::Linear(Vec::new()));
        let UnoptimizedCode::Linear(items) = items else {
            unreachable!()
        };
        let graph = crate::assembler::block_graph::BlockGraph::from_items(
            items,
            &stale.preserved_block_boundaries,
        );
        let reachable = graph.reachable_blocks(&stale.exception_regions);
        stale.code = UnoptimizedCode::Prepared { graph, reachable };
        stale.bind_folded_normal_finally_nops();
        assert!(
            stale.exception_owner_provenance.folded_normal_finally_nops[0]
                .owner
                .is_none()
        );
        assert!(
            stale.exception_owner_provenance.folded_normal_finally_nops[0]
                .source
                .is_none()
        );
        for prepared in [false, true] {
            let mut changed = fixture();
            if prepared {
                changed.used_constant_indices(3);
            }
            let change = |instruction: &mut Instruction| {
                if instruction.opcode == LOAD_CONST {
                    instruction.set_operand(Operand::Value(2));
                }
            };
            match &mut changed.code {
                UnoptimizedCode::Linear(items) => {
                    for item in items {
                        if let Item::Instruction(instruction) = item {
                            change(instruction);
                        }
                    }
                }
                UnoptimizedCode::Prepared { graph, .. } => graph.for_each_instruction_mut(change),
            }
            changed.remap_constant_indices(&[Some(0), Some(1), Some(1)]);
            assert!(
                changed
                    .exception_owner_provenance
                    .folded_normal_finally_nops
                    .is_empty(),
                "prepared={prepared}"
            );
        }
    }

    #[test]
    fn folded_normal_pop_region_transport_requires_complete_groups_and_cell_roles() {
        for mutation in [
            "intact",
            "missing member",
            "foreign original",
            "duplicate member",
            "wrong handler",
            "wrong priority",
            "wrong role",
            "foreign overlap",
            "changed aliases",
            "changed cell",
        ] {
            let mut assembler = fixture();
            assembler.bind_folded_normal_finally_nops();
            let mut layout = layout(assembler);
            let before = layout.exception_regions.clone();
            let original = before[0];
            let header = match layout.exception_owner_provenance.folded_normal_finally_nops[0]
                .prefix
                .last()
                .unwrap()
            {
                Item::Label(label) => *label,
                Item::Instruction(_) => unreachable!(),
            };
            let first = ExceptionRegion {
                end: header,
                ..original
            };
            let second = ExceptionRegion {
                start: header,
                ..original
            };
            layout.exception_regions = vec![first, second];
            let mut retargets = vec![(original, first), (original, second)];
            match mutation {
                "missing member" => {
                    retargets.pop();
                }
                "foreign original" => retargets[0].0.depth += 1,
                "duplicate member" => {
                    retargets.push(retargets[0]);
                }
                "wrong handler" => {
                    layout.exception_regions[0].target = original.start;
                    retargets[0].1 = layout.exception_regions[0];
                }
                "wrong priority" => {
                    layout.exception_regions[0].priority =
                        ExceptionRegionPriority::OriginalRange(header, original.end);
                    retargets[0].1 = layout.exception_regions[0];
                }
                "wrong role" => {
                    let entry =
                        layout.exception_owner_provenance.folded_normal_finally_nops[0].entry;
                    layout.exception_regions[0].end = entry;
                    retargets[0].1 = layout.exception_regions[0];
                }
                "foreign overlap" => layout
                    .exception_regions
                    .push(ExceptionRegion { depth: 1, ..first }),
                "changed aliases" => {
                    let at = layout
                        .items
                        .iter()
                        .position(|item| *item == Item::Label(original.target))
                        .unwrap();
                    layout.items.insert(at, Item::Label(Label::from_u32(100)));
                }
                "changed cell" => {
                    let at = layout.items.iter().position(|item| matches!(item, Item::Instruction(instruction) if instruction.opcode == GET_ITER)).unwrap();
                    let Item::Instruction(instruction) = &mut layout.items[at] else {
                        unreachable!()
                    };
                    instruction.no_location_policy.allow_block_inlining();
                }
                _ => {}
            }
            let global =
                authorized_region_rebuild(&before, &layout.exception_regions, &retargets, false);
            let mut origin = layout
                .exception_owner_provenance
                .folded_normal_finally_nops
                .pop()
                .unwrap();
            let labels = UniqueLabelPositions::new(&layout.items);
            let positions = positions(&layout.items, &labels).unwrap();
            let accepted = global
                && origin
                    .transport_regions(
                        &layout.items,
                        &layout.exception_regions,
                        &layout.exception_owner_provenance,
                        &retargets,
                        &labels,
                        &positions,
                    )
                    .is_some();
            assert_eq!(accepted, mutation == "intact", "{mutation}");
            if accepted {
                layout
                    .exception_owner_provenance
                    .folded_normal_finally_nops
                    .push(origin);
                assert_eq!(owners(&layout).len(), 1);
            }
        }
    }

    #[test]
    fn folded_normal_pop_late_replay_rejects_foreign_or_changed_records() {
        for mutation in [
            "load index",
            "pop location",
            "pop policy",
            "duplicate source",
            "duplicate iterator",
            "foreign edge",
            "missing boundary",
            "foreign head boundary",
            "foreign handler",
            "endpoint alias",
            "owner order",
            "owner depth",
        ] {
            let mut assembler = fixture();
            assembler.bind_folded_normal_finally_nops();
            let mut layout = layout(assembler);
            let origin = &layout.exception_owner_provenance.folded_normal_finally_nops[0];
            let labels = UniqueLabelPositions::new(&layout.items);
            let (start, _) = labels.get(origin.entry).unwrap();
            let pop = start + origin.pop_at;
            let entry = origin.entry;
            match mutation {
                "duplicate source" => layout.items.push(Item::Instruction(origin.guard)),
                "duplicate iterator" => layout.items.push(layout.items[pop + 2]),
                "foreign edge" => layout.items.push(Item::Instruction(Instruction::new(
                    JUMP_FORWARD,
                    Operand::Forward(entry),
                    SourceLocation::NONE,
                    None,
                ))),
                "missing boundary" => {
                    layout
                        .exception_owner_provenance
                        .block_boundaries
                        .remove(&entry);
                }
                "foreign head boundary" => {
                    let Item::Label(label) = layout.items[pop - 1] else {
                        unreachable!()
                    };
                    layout
                        .exception_owner_provenance
                        .block_boundaries
                        .insert(label);
                }
                "foreign handler" => {
                    let region = layout.exception_regions[0];
                    layout.exception_regions.push(ExceptionRegion {
                        start: region.end,
                        target: entry,
                        ..region
                    });
                }
                "endpoint alias" => {
                    let end = layout.exception_regions[0].end;
                    let at = labels.get(end).unwrap().0;
                    layout.items.insert(at, Item::Label(Label::from_u32(100)));
                }
                "owner order" => layout.exception_regions.insert(
                    0,
                    ExceptionRegion {
                        depth: 1,
                        ..layout.exception_regions[0]
                    },
                ),
                "owner depth" => layout.exception_regions[0].depth += 1,
                _ => {
                    let at = if mutation == "load index" {
                        pop + 1
                    } else {
                        pop
                    };
                    let Item::Instruction(instruction) = &mut layout.items[at] else {
                        unreachable!()
                    };
                    if mutation == "load index" {
                        instruction.set_operand(Operand::Value(2));
                    } else if mutation == "pop location" {
                        instruction.location = SourceLocation::NONE;
                    } else {
                        instruction.no_location_policy.allow_block_inlining();
                    }
                }
            }
            assert!(owners(&layout).is_empty(), "{mutation}");
        }
    }

    #[test]
    fn folded_normal_pop_second_compaction_transports_only_the_live_old_operand() {
        for changed in [false, true] {
            let mut assembler = fixture_with_constant(true);
            assembler.remap_constant_indices(&[Some(0), Some(2)]);
            assembler.bind_folded_normal_finally_nops();
            let origin = &assembler
                .exception_owner_provenance
                .folded_normal_finally_nops[0];
            assert_eq!(origin.load().operand(), Operand::Value(2));
            let pop = origin.pop();
            let location = origin.guard.location;
            let mut pipeline = assembler.into_optimization_pipeline();
            pipeline.graph.for_each_instruction_mut(|instruction| {
                if *instruction == pop {
                    instruction.location = location;
                }
                if changed && instruction.opcode == LOAD_CONST {
                    instruction.set_operand(Operand::Value(1));
                }
            });
            let indices = [Some(0), Some(1), Some(1)];
            pipeline.remap_folded_normal_finally_constants(&indices);
            pipeline.graph.for_each_instruction_mut(|instruction| {
                if instruction.opcode == LOAD_CONST
                    && let Operand::Value(index) = instruction.operand()
                {
                    instruction.set_operand(Operand::Value(indices[index as usize].unwrap()));
                }
            });
            assert_eq!(
                pipeline.retain_folded_normal_finally_nops(true).is_some(),
                !changed
            );
            assert_eq!(
                pipeline
                    .exception_owner_provenance
                    .folded_normal_finally_nops
                    .len(),
                usize::from(!changed)
            );
        }
    }

    #[test]
    fn folded_normal_pop_distinguishes_source_aliases_from_next_loop_header() {
        for next_header in [false, true] {
            let mut assembler = fixture();
            assembler.bind_folded_normal_finally_nops();
            let mut layout = layout(assembler);
            let origin = &layout.exception_owner_provenance.folded_normal_finally_nops[0];
            let Item::Label(label) = (if next_header {
                *origin.prefix.last().unwrap()
            } else {
                origin.prefix[origin.pop_at - 1]
            }) else {
                unreachable!()
            };
            layout
                .exception_owner_provenance
                .block_boundaries
                .insert(label);
            assert_eq!(owners(&layout).len(), usize::from(next_header));
        }
    }
}
