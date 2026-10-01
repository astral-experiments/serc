use super::{
    BlockGraph, ExceptionOwnerProvenance, ExceptionRegion, FxHashSet, Instruction, Item,
    ItemPosition, Label, Operand, RetainedSetupTruth, anchored_start, instruction, label,
    same_regions,
};

pub(in crate::assembler) struct RetainedSetupCollapse {
    join: Label,
    position: ItemPosition,
    before: Vec<Item>,
    incoming: Vec<Instruction>,
    regions: Vec<ExceptionRegion>,
}

fn incoming(items: &[Item], labels: &FxHashSet<Label>) -> Vec<Instruction> {
    items.iter().filter_map(instruction).filter(|value|
        matches!(value.operand(), Operand::Forward(label) | Operand::Backward(label) if labels.contains(&label))).collect()
}

impl RetainedSetupCollapse {
    pub(in crate::assembler) fn capture(
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        origins: &mut Vec<RetainedSetupTruth>,
        removed: &FxHashSet<ItemPosition>,
    ) -> Vec<Self> {
        if origins.is_empty() {
            return Vec::new();
        }
        let items = graph.copy_items();
        let mut receipts = Vec::new();
        origins.retain(|origin| {
            if !origin.records(&items)
                || !origin.roles_match(provenance, regions)
                || !origin
                    .regions
                    .as_ref()
                    .is_some_and(|old| same_regions(old, regions))
            {
                return false;
            }
            let Some(collapse) = &origin.target_collapse else {
                return true;
            };
            if collapse.removed {
                return true;
            }
            let Some(block) = graph.label_block(collapse.source) else {
                return false;
            };
            if graph[block].items() != collapse.original {
                return false;
            }
            let position = ItemPosition::new(block, 1);
            if !removed.contains(&position) {
                return true;
            }
            let Some(start) = anchored_start(&items, &collapse.original) else {
                return false;
            };
            if items.get(start + 2..start + 2 + origin.target_head.len())
                != Some(origin.target_head.as_slice())
                || !origin.target_head.contains(&Item::Label(collapse.target))
            {
                return false;
            }
            let labels = origin
                .target_head
                .iter()
                .filter_map(label)
                .chain([collapse.source])
                .collect();
            receipts.push(Self {
                join: origin.source.join,
                position,
                before: collapse.original.to_vec(),
                incoming: incoming(&items, &labels),
                regions: regions.to_vec(),
            });
            true
        });
        receipts
    }

    pub(in crate::assembler) fn finish(
        receipts: &[Self],
        graph: &BlockGraph,
        regions: &[ExceptionRegion],
        provenance: &ExceptionOwnerProvenance,
        origins: &mut Vec<RetainedSetupTruth>,
    ) {
        if origins.is_empty() {
            return;
        }
        let items = graph.copy_items();
        origins.retain_mut(|origin| {
            let matched = receipts
                .iter()
                .filter(|receipt| receipt.join == origin.source.join)
                .collect::<Vec<_>>();
            if matched.is_empty() {
                return origin.records(&items) && origin.roles_match(provenance, regions);
            }
            let [receipt] = matched.as_slice() else {
                return false;
            };
            let Some(collapse) = &mut origin.target_collapse else {
                return false;
            };
            if collapse.removed
                || receipt.before != collapse.original
                || receipt.position.offset() != 1
                || graph[receipt.position.block()].items() != [Item::Label(collapse.source)]
                || !same_regions(&receipt.regions, regions)
                || items.iter().any(|item| *item == collapse.original[1])
            {
                return false;
            }
            let labels = origin
                .target_head
                .iter()
                .filter_map(label)
                .chain([collapse.source])
                .collect();
            let Some(jump) = instruction(&collapse.original[1]) else {
                return false;
            };
            if receipt
                .incoming
                .iter()
                .filter(|value| **value == jump)
                .count()
                != 1
                || incoming(&items, &labels)
                    != receipt
                        .incoming
                        .iter()
                        .filter(|value| **value != jump)
                        .copied()
                        .collect::<Vec<_>>()
            {
                return false;
            }
            origin.target_head.insert(0, Item::Label(collapse.source));
            collapse.removed = true;
            origin.records(&items) && origin.roles_match(provenance, regions)
        });
    }
}
