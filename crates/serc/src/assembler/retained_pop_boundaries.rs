//! Retained original POP boundaries across the cold pass's region-end rewrite.
use rustc_hash::{FxHashMap, FxHashSet};

use super::exception_owners::ExceptionOwnerProvenance;
use super::fusion_owner_slots::same_manager_region;
use super::normal_finally_locations::{array_start, unique_label_positions};
use super::{ExceptionExclusion, ExceptionRegion, Item, Label, Operand, SourceLocation};
use crate::target::opcodes::{LOAD_FAST, LOAD_FAST_BORROW, NOP};

#[cfg(test)]
mod tests;

#[derive(Debug, Default)]
pub(super) struct RetainedPopBoundaries {
    regions: Vec<ExceptionRegion>,
    origins: Vec<Origin>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PopExclusion {
    pub(super) region_end: Label,
    pub(super) start: Label,
    pub(super) end: Label,
    pub(super) handlers: Vec<Label>,
}

#[derive(Debug)]
struct Origin {
    region_index: usize,
    head: Vec<Item>,
    following: Vec<Item>,
    roles: Vec<(Label, bool, bool)>,
    setups: Vec<(Label, Option<SourceLocation>)>,
    incoming: Vec<Vec<Item>>,
    ends: Vec<Label>,
    exclusion: Option<PopExclusion>,
}

pub(super) struct ColdRewrite {
    original: Vec<ExceptionRegion>,
    rebuilt: Vec<ExceptionRegion>,
    origins: Vec<Origin>,
    groups: usize,
    valid: bool,
}

fn same_regions(a: &[ExceptionRegion], b: &[ExceptionRegion]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_manager_region(a, b))
}

impl PopExclusion {
    fn matches_owners(&self, regions: &[ExceptionRegion]) -> bool {
        let before = regions
            .iter()
            .filter(|owner| owner.end == self.start)
            .collect::<Vec<_>>();
        let after = regions
            .iter()
            .filter(|owner| owner.start == self.end)
            .collect::<Vec<_>>();
        if self.handlers.is_empty()
            || before.len() != self.handlers.len()
            || after.len() != self.handlers.len()
        {
            return false;
        }
        let mut previous_depth = None;
        self.handlers.iter().enumerate().all(|(index, handler)| {
            if self.handlers[..index].contains(handler) {
                return false;
            }
            let before = before
                .iter()
                .filter(|owner| owner.target == *handler)
                .copied()
                .collect::<Vec<_>>();
            let after = after
                .iter()
                .filter(|owner| owner.target == *handler)
                .copied()
                .collect::<Vec<_>>();
            let ([before], [after]) = (before.as_slice(), after.as_slice()) else {
                return false;
            };
            let valid = before.preserve_lasti
                && !before.is_scope_fallback
                && before.depth == after.depth
                && before.preserve_lasti == after.preserve_lasti
                && before.is_scope_fallback == after.is_scope_fallback
                && super::normal_finally_locations::same_priority(before.priority, after.priority)
                && previous_depth.is_none_or(|depth| depth < before.depth);
            previous_depth = Some(before.depth);
            valid
        })
    }
}

fn incoming(items: &[Item], labels: &[Label]) -> Option<Vec<Vec<Item>>> {
    items.iter().enumerate().filter_map(|(index, item)| match item {
        Item::Instruction(i) if matches!(i.operand(), Operand::Forward(label) | Operand::Backward(label) if labels.contains(&label)) => Some(array_start(items, index).map(|start| items[start..=index].to_vec())),
        _ => None,
    }).collect()
}

fn same_array(actual: &[Item], original: &[Item]) -> bool {
    actual.len() == original.len()
        && actual.iter().zip(original).all(|(actual, original)| {
            if let (Item::Instruction(actual), Item::Instruction(original)) = (actual, original) {
                let mut expected = *original;
                if expected.opcode == LOAD_FAST && actual.opcode == LOAD_FAST_BORROW {
                    expected.opcode = LOAD_FAST_BORROW;
                }
                *actual == expected
            } else {
                actual == original
            }
        })
}

impl Origin {
    fn matches(
        &self,
        items: &[Item],
        positions: &FxHashMap<Label, usize>,
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> bool {
        let labels = self
            .roles
            .iter()
            .map(|(label, _, _)| *label)
            .collect::<Vec<_>>();
        let Some(&start) = positions.get(&labels[0]) else {
            return false;
        };
        !matches!(items.get(start.wrapping_sub(1)), Some(Item::Label(_)))
            && items.get(start..start + self.head.len()) == Some(self.head.as_slice())
            && items
                .get(start + self.head.len()..start + self.head.len() + self.following.len())
                .is_some_and(|following| same_array(following, &self.following))
            && self.roles.iter().all(|(label, source, block)| {
                provenance.source_boundaries.contains(label) == *source
                    && provenance.block_boundaries.contains(label) == *block
                    && !preserved.contains(label)
                    && !provenance.removed_pop_block_boundaries.contains(label)
                    && !provenance.omitted_nop_counts.contains_key(label)
                    && !provenance.omitted_nop_locations.contains_key(label)
                    && !provenance.fusion_boundaries.contains(label)
                    && !provenance.retired_normal_try_copy_labels.contains(label)
                    && !provenance.is_normal_try_copy_label(*label)
                    && !regions.iter().any(|region| region.target == *label)
            })
            && provenance
                .setups
                .iter()
                .filter(|(label, _)| labels.contains(label))
                .copied()
                .collect::<Vec<_>>()
                == self.setups
            && self.exclusion.as_ref().is_none_or(|exclusion| {
                provenance
                    .retained_pop_exclusions
                    .iter()
                    .filter(|original| *original == exclusion)
                    .count()
                    == 1
                    && !provenance.source_boundaries.contains(&exclusion.end)
                    && !provenance.block_boundaries.contains(&exclusion.end)
                    && !provenance
                        .setups
                        .iter()
                        .any(|(label, _)| *label == exclusion.end)
                    && !preserved.contains(&exclusion.end)
                    && !provenance
                        .removed_pop_block_boundaries
                        .contains(&exclusion.end)
                    && !provenance.omitted_nop_counts.contains_key(&exclusion.end)
                    && !provenance
                        .omitted_nop_locations
                        .contains_key(&exclusion.end)
                    && !provenance.fusion_boundaries.contains(&exclusion.end)
                    && !provenance
                        .retired_normal_try_copy_labels
                        .contains(&exclusion.end)
                    && !provenance.is_normal_try_copy_label(exclusion.end)
                    && !regions.iter().any(|region| region.target == exclusion.end)
                    && incoming(items, &[exclusion.end]).is_some_and(|incoming| incoming.is_empty())
            })
            && incoming(items, &labels).is_some_and(|incoming| {
                incoming.len() == self.incoming.len()
                    && incoming
                        .iter()
                        .zip(&self.incoming)
                        .all(|(actual, original)| same_array(actual, original))
            })
    }
}

impl ColdRewrite {
    /// Captures an actual ordinary region end immediately before its surviving POP.
    pub(super) fn capture(
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> Option<Self> {
        let positions = unique_label_positions(items)?;
        let mut origins = Vec::new();
        for (region_index, region) in regions.iter().enumerate() {
            if region.is_scope_fallback || region.preserve_lasti {
                continue;
            }
            let Some(&at) = positions.get(&region.end) else {
                continue;
            };
            let mut start = at;
            while start > 0 && matches!(items[start - 1], Item::Label(_)) {
                start -= 1;
            }
            let mut end = at;
            while matches!(items.get(end), Some(Item::Label(_))) {
                end += 1;
            }
            let Some(Item::Instruction(pop)) = items.get(end) else {
                continue;
            };
            if pop.opcode != NOP
                || pop.operand() != Operand::Value(0)
                || !pop.fusion_policy.is_converted_pop_block()
                || pop.exception_policy().exclusion() != ExceptionExclusion::Always
                || pop.depth_after() != Some(region.depth)
                || pop.pre_fold_extra_instructions() != 0
                || pop.local_policy.hidden_exception_owner_slots() > 1
            {
                continue;
            }
            let labels = items[start..end]
                .iter()
                .map(|item| {
                    let Item::Label(label) = item else {
                        unreachable!()
                    };
                    *label
                })
                .collect::<Vec<_>>();
            let mut following_end = end + 1;
            while matches!(items.get(following_end), Some(Item::Label(_))) {
                following_end += 1;
            }
            if !matches!(items.get(following_end), Some(Item::Instruction(_))) {
                continue;
            }
            let Some(incoming) = incoming(items, &labels) else {
                continue;
            };
            let exclusions = provenance
                .retained_pop_exclusions
                .iter()
                .filter(|exclusion| {
                    exclusion.region_end == region.end
                        && labels.last() == Some(&exclusion.start)
                        && items.get(end + 1..=following_end).is_some_and(|following| {
                            matches!(following,
                        [Item::Label(label), Item::Instruction(_)] if *label == exclusion.end)
                        })
                        && exclusion.matches_owners(regions)
                })
                .collect::<Vec<_>>();
            let exclusion = match exclusions.as_slice() {
                [exclusion] => Some((*exclusion).clone()),
                _ => None,
            };
            let origin = Origin {
                region_index,
                head: items[start..=end].to_vec(),
                following: items[end + 1..=following_end].to_vec(),
                roles: labels
                    .iter()
                    .map(|label| {
                        (
                            *label,
                            provenance.source_boundaries.contains(label),
                            provenance.block_boundaries.contains(label),
                        )
                    })
                    .collect(),
                setups: provenance
                    .setups
                    .iter()
                    .filter(|(label, _)| labels.contains(label))
                    .copied()
                    .collect(),
                incoming,
                ends: vec![region.end],
                exclusion,
            };
            if origin.matches(items, &positions, regions, provenance, preserved) {
                origins.push(origin);
            }
        }
        (!origins.is_empty()).then(|| Self {
            original: regions.to_vec(),
            rebuilt: Vec::new(),
            origins,
            groups: 0,
            valid: true,
        })
    }

    /// Binds every original region to the ordered group emitted by this writer.
    pub(super) fn group(
        &mut self,
        index: usize,
        region: ExceptionRegion,
        outputs: &[ExceptionRegion],
    ) {
        self.valid &= index == self.groups
            && self
                .original
                .get(index)
                .is_some_and(|original| same_manager_region(original, &region));
        self.groups += 1;
        self.origins.retain_mut(|origin| {
            if origin.region_index != index {
                return true;
            }
            if outputs.is_empty() {
                return region.start == region.end;
            }
            if !outputs.iter().all(|output| {
                output.target == region.target
                    && output.depth == region.depth
                    && output.preserve_lasti == region.preserve_lasti
                    && output.is_scope_fallback == region.is_scope_fallback
                    && super::normal_finally_locations::same_priority(
                        output.priority,
                        region.priority,
                    )
            }) {
                return false;
            }
            let mut matching = outputs.iter().filter(|output| {
                origin
                    .roles
                    .iter()
                    .any(|(label, _, _)| *label == output.end)
            });
            let Some(output) = matching.next() else {
                return false;
            };
            if matching.next().is_some() {
                return false;
            }
            if !origin.ends.contains(&output.end) {
                origin.ends.push(output.end);
            }
            true
        });
        self.rebuilt.extend_from_slice(outputs);
    }

    /// Seals only unchanged POP occurrences after the complete region rewrite.
    pub(super) fn finish(
        mut self,
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> RetainedPopBoundaries {
        let Some(positions) = unique_label_positions(items) else {
            return RetainedPopBoundaries::default();
        };
        if !self.valid
            || self.groups != self.original.len()
            || !same_regions(&self.rebuilt, regions)
        {
            return RetainedPopBoundaries::default();
        }
        self.origins
            .retain(|origin| origin.matches(items, &positions, regions, provenance, preserved));
        RetainedPopBoundaries {
            regions: self.rebuilt,
            origins: self.origins,
        }
    }
}

impl RetainedPopBoundaries {
    /// Returns aliases whose exact retained POP and region census still survive.
    pub(super) fn valid_labels(
        &self,
        items: &[Item],
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        preserved: &FxHashSet<Label>,
    ) -> FxHashSet<Label> {
        let mut result = FxHashSet::default();
        if self.origins.is_empty() || !same_regions(&self.regions, regions) {
            return result;
        }
        let Some(positions) = unique_label_positions(items) else {
            return result;
        };
        for origin in &self.origins {
            if origin.matches(items, &positions, regions, provenance, preserved) {
                result.extend(&origin.ends);
                if let Some(exclusion) = &origin.exclusion {
                    result.insert(exclusion.end);
                }
            }
        }
        result
    }
}
