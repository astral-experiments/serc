use super::*;
use crate::assembler::Assembler;

#[derive(Clone, Copy, Debug)]
enum Boundary {
    Split,
    SingleBlock,
    EndOutsideCopy,
    Sibling,
    DifferentSiblingOwner,
    DifferentSiblingDepth,
}

fn fixture(boundary: Boundary) -> OptimizationPipeline {
    let label = Label::from_u32;
    let location = SourceLocation::new(2, 2, 4, 12);
    let instruction = |opcode, operand, location| {
        Item::Instruction(Instruction::new(opcode, operand, location, None))
    };
    let mut items = vec![
        Item::Label(label(0)),
        instruction(NOP, Operand::Value(0), location),
    ];
    for source in [1, 2] {
        items.extend([
            Item::Label(label(source)),
            instruction(LOAD_CONST, Operand::Value(0), location),
            instruction(JUMP_FORWARD, Operand::Forward(label(3)), location),
        ]);
    }
    items.push(Item::Label(label(3)));
    if !matches!(boundary, Boundary::SingleBlock) {
        items.extend([
            instruction(POP_TOP, Operand::Value(0), SourceLocation::NONE),
            Item::Label(label(4)),
        ]);
    }
    items.push(instruction(
        JUMP_BACKWARD,
        Operand::Backward(label(0)),
        SourceLocation::NONE,
    ));
    for end in 5..=8 {
        items.extend([
            Item::Label(label(end)),
            instruction(RAISE_VARARGS, Operand::Value(0), location),
        ]);
    }
    let mut assembler = Assembler::from_linear_items(items);
    assembler.next_label = 9;
    assembler.python_version = crate::PythonVersion::Py315;
    assembler.add_exception_region(label(1), label(5), label(6), 0, false);
    match boundary {
        Boundary::Split => {
            assembler.add_exception_region(label(1), label(4), label(7), 0, false);
        }
        Boundary::SingleBlock | Boundary::EndOutsideCopy => {
            assembler.add_exception_region(label(1), label(5), label(7), 0, false);
            if matches!(boundary, Boundary::EndOutsideCopy) {
                // Keep the target multiblock, but not at the source owner's end.
                assembler.add_exception_region(label(3), label(4), label(8), 0, false);
            }
        }
        Boundary::Sibling | Boundary::DifferentSiblingOwner | Boundary::DifferentSiblingDepth => {
            assembler.add_exception_region(label(1), label(2), label(7), 0, false);
            let owner = if matches!(boundary, Boundary::DifferentSiblingOwner) {
                8
            } else {
                7
            };
            let depth = u32::from(matches!(boundary, Boundary::DifferentSiblingDepth));
            assembler.add_exception_region(label(3), label(4), label(owner), depth, false);
        }
    }
    assembler.into_optimization_pipeline()
}

fn copied_owners(
    pipeline: &OptimizationPipeline,
    opcode: crate::target::Opcode,
) -> FxHashSet<(Label, u32, bool)> {
    let items = pipeline.graph.iter_items().collect::<Vec<_>>();
    let labels = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            Item::Label(label) => Some((*label, index)),
            Item::Instruction(_) => None,
        })
        .collect::<FxHashMap<_, _>>();
    let start = labels[&Label::from_u32(1)];
    let end = labels[&Label::from_u32(2)];
    let copies = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (start < index
                && index < end
                && matches!(item, Item::Instruction(instruction) if instruction.opcode == opcode))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let [position] = copies.as_slice() else {
        panic!("expected one actual copied {opcode:?}, found {copies:?}");
    };
    pipeline
        .exception_regions
        .iter()
        .filter_map(|region| {
            (labels[&region.start] <= *position && *position < labels[&region.end]).then_some((
                region.target,
                region.depth,
                region.preserve_lasti,
            ))
        })
        .collect()
}

#[test]
fn copied_backedge_respects_physical_owner_end() {
    let outer = (Label::from_u32(6), 0, false);
    let inner = (Label::from_u32(7), 0, false);
    for boundary in [
        Boundary::Split,
        Boundary::SingleBlock,
        Boundary::EndOutsideCopy,
        Boundary::Sibling,
        Boundary::DifferentSiblingOwner,
        Boundary::DifferentSiblingDepth,
    ] {
        let mut pipeline = fixture(boundary);
        let absence = pipeline.capture_original_location_absence();
        pipeline.duplicate_exit_blocks(&absence, &mut Vec::new());
        let backedge = copied_owners(&pipeline, JUMP_BACKWARD);
        assert!(
            backedge.contains(&outer),
            "{boundary:?}: outer backedge owner"
        );
        assert_eq!(
            backedge.contains(&inner),
            matches!(boundary, Boundary::SingleBlock | Boundary::EndOutsideCopy),
            "{boundary:?}: inner backedge owner",
        );
        if matches!(boundary, Boundary::SingleBlock) {
            continue;
        }
        let pop = copied_owners(&pipeline, POP_TOP);
        let target_owner = match boundary {
            Boundary::DifferentSiblingOwner => (Label::from_u32(8), 0, false),
            Boundary::DifferentSiblingDepth => (Label::from_u32(7), 1, false),
            _ => inner,
        };
        assert!(
            pop.contains(&outer) && pop.contains(&target_owner),
            "{boundary:?}: POP_TOP owners"
        );
        assert!(
            !backedge.contains(&target_owner) || matches!(boundary, Boundary::EndOutsideCopy),
            "{boundary:?}: target backedge owner"
        );
        if matches!(
            boundary,
            Boundary::DifferentSiblingOwner | Boundary::DifferentSiblingDepth
        ) {
            assert!(
                !pop.contains(&inner),
                "{boundary:?}: unrelated source owner"
            );
        }
    }
}
