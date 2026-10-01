use super::{
    BasicBlockId, BlockGraph, ExceptionRegion, FxHashMap, FxHashSet, Instruction, Item, Label,
    RetainedSetupTruth, label, same_manager_region, same_regions, unique_label_positions,
};

/// Actual block membership and indexed output of the cold-region writer.
pub(in crate::assembler) struct RetainedSetupColdRewrite {
    regions: Vec<ExceptionRegion>,
    order: Vec<BasicBlockId>,
    heads: FxHashMap<BasicBlockId, Label>,
    ranges: Vec<std::ops::Range<usize>>,
    bridges: Vec<(BasicBlockId, Label, Instruction)>,
    outputs: Vec<usize>,
    groups: Vec<(usize, usize)>,
}

impl RetainedSetupColdRewrite {
    pub(in crate::assembler) fn capture(
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        origins: &mut Vec<RetainedSetupTruth>,
    ) -> Option<Self> {
        if origins.is_empty() {
            return None;
        }
        let items = graph.copy_items();
        origins.retain(|origin| {
            origin.records(&items)
                && origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, regions))
        });
        if origins.is_empty() {
            return None;
        }
        let captured = (|| {
            unique_label_positions(&items)?;
            let order = graph.order().to_vec();
            let positions = order
                .iter()
                .enumerate()
                .map(|(index, block)| (*block, index))
                .collect::<FxHashMap<_, _>>();
            let heads = order
                .iter()
                .map(|block| Some((*block, graph[*block].first_label()?)))
                .collect::<Option<FxHashMap<_, _>>>()?;
            let ranges = regions
                .iter()
                .map(|region| {
                    graph.label_block(region.target)?;
                    let start = *positions.get(&graph.label_block(region.start)?)?;
                    let end = *positions.get(&graph.label_block(region.end)?)?;
                    (start <= end).then_some(start..end)
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Self {
                regions: regions.to_vec(),
                order,
                heads,
                ranges,
                bridges: Vec::new(),
                outputs: Vec::new(),
                groups: Vec::new(),
            })
        })();
        if captured.is_none() {
            origins.clear();
        }
        captured
    }

    pub(in crate::assembler) fn bridge(
        &mut self,
        block: BasicBlockId,
        label: Label,
        instruction: Instruction,
    ) {
        self.bridges.push((block, label, instruction));
    }

    pub(in crate::assembler) fn output(&mut self, original_index: usize) {
        self.outputs.push(original_index);
    }

    pub(in crate::assembler) fn group(&mut self, original_index: usize, count: usize) {
        self.groups.push((original_index, count));
    }

    pub(in crate::assembler) fn finish(
        self,
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        final_label: Option<Label>,
        origins: &mut Vec<RetainedSetupTruth>,
    ) {
        let items = graph.copy_items();
        let valid = (|| {
            let labels = unique_label_positions(&items)?;
            if regions.iter().any(|region| {
                [region.start, region.end, region.target]
                    .iter()
                    .any(|label| !labels.contains_key(label))
            }) {
                return None;
            }
            let mut fresh = FxHashSet::default();
            for (block, label, instruction) in &self.bridges {
                if self.heads.contains_key(block)
                    || !fresh.insert(*block)
                    || graph[*block].items()
                        != [Item::Label(*label), Item::Instruction(*instruction)]
                {
                    return None;
                }
            }
            let mut order = graph.order();
            if let Some(label) = final_label {
                let (last, preceding) = order.split_last()?;
                if self.heads.contains_key(last)
                    || fresh.contains(last)
                    || graph[*last].items() != [Item::Label(label)]
                {
                    return None;
                }
                order = preceding;
            }
            if order
                .iter()
                .any(|block| !self.heads.contains_key(block) && !fresh.contains(block))
                || self.heads.iter().any(|(block, label)| {
                    order.contains(block) && graph[*block].first_label() != Some(*label)
                })
            {
                return None;
            }
            let positions = self
                .order
                .iter()
                .enumerate()
                .map(|(index, block)| (*block, index))
                .collect::<FxHashMap<_, _>>();
            if self.groups.len() != self.regions.len() {
                return None;
            }
            let mut expected = Vec::new();
            for (original_index, (region, range)) in
                self.regions.iter().zip(&self.ranges).enumerate()
            {
                let member = |block: &BasicBlockId| {
                    positions
                        .get(block)
                        .is_some_and(|position| range.contains(position))
                };
                let before = expected.len();
                let mut position = 0;
                while position < order.len() {
                    while position < order.len() && !member(&order[position]) {
                        position += 1;
                    }
                    if position == order.len() {
                        break;
                    }
                    let start = graph[order[position]].first_label()?;
                    while position < order.len() && member(&order[position]) {
                        position += 1;
                    }
                    let end = if position == order.len() {
                        final_label?
                    } else {
                        graph[order[position]].first_label()?
                    };
                    expected.push((
                        original_index,
                        ExceptionRegion {
                            start,
                            end,
                            ..*region
                        },
                    ));
                }
                let count = expected.len() - before;
                if self.groups[original_index] != (original_index, count)
                    || (count == 0 && !range.is_empty())
                {
                    return None;
                }
            }
            if expected.len() != regions.len()
                || self.outputs.len() != regions.len()
                || !expected.iter().zip(regions).zip(&self.outputs).all(
                    |(((index, before), after), emitted)| {
                        index == emitted && same_manager_region(before, after)
                    },
                )
            {
                return None;
            }
            Some(())
        })()
        .is_some();
        origins.retain_mut(|origin| {
            if !valid
                || !origin.records(&items)
                || !origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, &self.regions))
            {
                return false;
            }
            let protected = origin
                .current
                .iter()
                .chain(&origin.current_predecessor)
                .chain(&origin.target_head)
                .filter_map(label)
                .filter_map(|label| graph.label_block(label))
                .collect::<FxHashSet<_>>();
            if self
                .groups
                .iter()
                .filter(|(_, count)| *count == 0)
                .any(|(index, _)| {
                    let region = &self.regions[*index];
                    region.target == origin.handler
                        || [region.start, region.end, region.target]
                            .iter()
                            .any(|label| {
                                graph
                                    .label_block(*label)
                                    .is_none_or(|block| protected.contains(&block))
                            })
                })
            {
                return false;
            }
            origin.regions = Some(regions.to_vec());
            true
        });
    }
}
