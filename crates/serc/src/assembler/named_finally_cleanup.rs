use super::{Assembler, Instruction, Item, Operand, SourceLocation, trailing_labels};
use crate::PythonVersion;
use crate::target::opcodes::{
    JUMP_FORWARD, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE, POP_JUMP_IF_NOT_NONE, STORE_FAST,
};

impl Assembler {
    /// Certifies the original if/elif join before a finalizer's named-handler cleanup.
    pub(crate) fn original_named_finally_cleanup_join(
        &self,
        body_start: usize,
        guard_location: SourceLocation,
        then_location: SourceLocation,
        fallthrough_location: SourceLocation,
    ) -> bool {
        if self.python_version < PythonVersion::Py314 {
            return false;
        }
        let items = self.linear_items();
        let aliases = trailing_labels(items).collect::<Vec<_>>();
        let provenance = &self.exception_owner_provenance;
        if aliases.is_empty()
            || self.join_labels_have_authored_location(&aliases)
            || self.current_join_has_located_compacted_predecessor()
            || aliases.iter().any(|label| {
                self.preserved_block_boundaries.contains(label)
                    || self.folded_away_condition_targets.contains(label)
                    || self.folded_away_loop_else_targets.contains(label)
                    || self.folded_condition_exit_targets.contains(label)
                    || provenance.omitted_nop_counts.contains_key(label)
                    || provenance.omitted_nop_locations.contains_key(label)
                    || provenance.setups.iter().any(|(target, _)| target == label)
                    || self
                        .exception_regions
                        .iter()
                        .any(|region| region.target == *label)
            })
        {
            return false;
        }
        let Some(previous_index) = items.len().checked_sub(aliases.len() + 1) else {
            return false;
        };
        let Some(Item::Instruction(previous)) = items.get(previous_index) else {
            return false;
        };
        let Some(depth) = previous.depth_after() else {
            return false;
        };
        if body_start >= previous_index
            || previous.opcode != STORE_FAST
            || !matches!(previous.operand(), Operand::Value(_))
            || *previous
                != Instruction::new(
                    STORE_FAST,
                    previous.operand(),
                    fallthrough_location,
                    Some(depth),
                )
        {
            return false;
        }
        let mut labels = rustc_hash::FxHashSet::default();
        let mut then_edge = None;
        let mut guard_edge = None;
        for (index, item) in items.iter().enumerate() {
            let instruction = match item {
                Item::Label(label) => {
                    if !labels.insert(*label) {
                        return false;
                    }
                    continue;
                }
                Item::Instruction(instruction) => instruction,
            };
            let (Operand::Forward(target) | Operand::Backward(target)) = instruction.operand()
            else {
                continue;
            };
            if !aliases.contains(&target) {
                continue;
            }
            if index < body_start
                || index >= previous_index
                || !provenance.source_boundaries.contains(&target)
                || !provenance.block_boundaries.contains(&target)
            {
                return false;
            }
            let location = match instruction.opcode {
                JUMP_FORWARD if then_edge.is_none() => then_location,
                POP_JUMP_IF_FALSE | POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE
                    if guard_edge.is_none() =>
                {
                    guard_location
                }
                _ => return false,
            };
            let mut expected = Instruction::new(
                instruction.opcode,
                Operand::Forward(target),
                location,
                Some(depth),
            );
            if matches!(instruction.opcode, POP_JUMP_IF_NONE | POP_JUMP_IF_NOT_NONE) {
                // This source receipt binds the three compacted None-comparison records.
                expected.local_policy.add_hidden_exception_owner_slots(3);
                if provenance
                    .source_none_conditions
                    .iter()
                    .filter(|origin| origin.instruction == *instruction)
                    .count()
                    != 1
                {
                    return false;
                }
            }
            if *instruction != expected {
                return false;
            }
            if instruction.opcode == JUMP_FORWARD {
                then_edge = Some((index, target));
            } else {
                guard_edge = Some((index, target));
            }
        }
        matches!((then_edge, guard_edge), (Some((then, then_target)), Some((guard, guard_target)))
            if then < guard && then_target == guard_target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::ExceptionPolicy;
    use crate::target::opcodes::{LOAD_FAST, NOP};

    struct Join {
        assembler: Assembler,
        then: usize,
        guard: usize,
        store: usize,
        target: super::super::Label,
        body_start: usize,
    }

    const THEN: SourceLocation = SourceLocation::new(4, 4, 8, 17);
    const GUARD: SourceLocation = SourceLocation::new(5, 5, 9, 14);
    const STORE: SourceLocation = SourceLocation::new(6, 6, 8, 21);

    fn fixture(version: PythonVersion, none: bool) -> Join {
        let mut assembler = Assembler::with_python_version(version);
        let target = assembler.label();
        assembler.set_location(THEN);
        let then = assembler.item_count();
        assembler.emit_forward_with_depth(JUMP_FORWARD, target, 1, ExceptionPolicy::default());
        let entry = assembler.label();
        assembler.mark(entry);
        assembler.record_exception_owner_block_boundary(entry);
        assembler.set_location(GUARD);
        assembler.emit_with_depth(LOAD_FAST, 0, 2, ExceptionPolicy::default());
        let guard = assembler.item_count();
        assembler.emit_forward_with_depth(
            if none {
                POP_JUMP_IF_NOT_NONE
            } else {
                POP_JUMP_IF_FALSE
            },
            target,
            1,
            ExceptionPolicy::default(),
        );
        if none {
            assembler.add_last_hidden_exception_owner_slots(3);
            assembler.record_source_none_condition();
        }
        assembler.set_location(STORE);
        let store = assembler.item_count();
        assembler.emit_with_depth(STORE_FAST, 2, 1, ExceptionPolicy::default());
        assembler.mark(target);
        assembler.record_exception_owner_block_boundary(target);
        Join {
            assembler,
            then,
            guard,
            store,
            target,
            body_start: 0,
        }
    }

    fn accepts(join: &Join) -> bool {
        join.assembler
            .original_named_finally_cleanup_join(join.body_start, GUARD, THEN, STORE)
    }

    fn instruction(join: &mut Join, index: usize) -> &mut Instruction {
        let Item::Instruction(instruction) = &mut join.assembler.linear_items_mut()[index] else {
            unreachable!()
        };
        instruction
    }

    #[test]
    fn original_named_finally_join_requires_complete_records() {
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            for none in [false, true] {
                assert!(accepts(&fixture(version, none)));
                for position in [0, 1, 2] {
                    for mutation in [
                        "location", "depth", "owner", "hidden", "folded", "policy", "opcode",
                    ] {
                        let mut join = fixture(version, none);
                        let index = [join.then, join.guard, join.store][position];
                        let record = instruction(&mut join, index);
                        match mutation {
                            "location" => record.location = SourceLocation::NONE,
                            "depth" => {
                                *record = Instruction::new(
                                    record.opcode,
                                    record.operand(),
                                    record.location,
                                    Some(7),
                                );
                            }
                            "owner" => record.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS),
                            "hidden" => record.local_policy.add_hidden_exception_owner_slots(1),
                            "folded" => record.add_pre_fold_extra_instructions(1),
                            "policy" => record.jump_policy.prevent_threading_target(),
                            "opcode" => record.opcode = NOP,
                            _ => unreachable!(),
                        }
                        assert!(!accepts(&join), "{version:?} {none} {position} {mutation}");
                    }
                }
            }
        }
    }

    #[test]
    fn original_named_finally_join_requires_original_edges_and_aliases() {
        for mutation in [
            "outside",
            "duplicate alias",
            "extra edge",
            "missing source",
            "missing block",
            "folded",
            "handler",
            "none receipt",
        ] {
            let mut join = fixture(PythonVersion::Py314, true);
            match mutation {
                "outside" => join.body_start = join.then + 1,
                "duplicate alias" => join.assembler.mark(join.target),
                "extra edge" => {
                    let edge = join.assembler.linear_items()[join.then];
                    let super::super::UnoptimizedCode::Linear(items) = &mut join.assembler.code
                    else {
                        unreachable!()
                    };
                    items.insert(join.guard, edge);
                }
                "missing source" => {
                    join.assembler
                        .exception_owner_provenance
                        .source_boundaries
                        .remove(&join.target);
                }
                "missing block" => {
                    join.assembler
                        .exception_owner_provenance
                        .block_boundaries
                        .remove(&join.target);
                }
                "folded" => {
                    join.assembler
                        .folded_away_condition_targets
                        .insert(join.target);
                }
                "handler" => join
                    .assembler
                    .exception_owner_provenance
                    .setups
                    .push((join.target, None)),
                "none receipt" => join
                    .assembler
                    .exception_owner_provenance
                    .source_none_conditions
                    .clear(),
                _ => unreachable!(),
            }
            assert!(!accepts(&join), "{mutation}");
        }
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            assert!(!accepts(&fixture(version, false)));
        }
    }
}
