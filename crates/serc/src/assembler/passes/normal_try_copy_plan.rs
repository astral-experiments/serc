// Pure model of CPython's ascending, once-per-block small-exit copy rounds.
// The caller must certify a closed original cleanup graph and supply emission order.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PrefixSlot {
    pub(super) is_nop: bool,
    pub(super) located: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OriginalTail {
    Jump {
        target: usize,
        located: bool,
        interruptible: bool,
    },
    ConstantReturn {
        // Authored return marker, LOAD_CONST, RETURN_VALUE.
        located: [bool; 3],
    },
    LocalTupleReturn {
        // Two unfused local loads, BUILD_TUPLE, RETURN_VALUE.
        located: [bool; 4],
    },
    ComputedReturn {
        // Intact located global attribute/call slots, including a fused PUSH_NULL.
        width: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OriginalNode {
    pub(super) id: usize,
    pub(super) prefix: Vec<PrefixSlot>,
    pub(super) tail: OriginalTail,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SlotOrigin {
    pub(super) node: usize,
    pub(super) slot: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SlotKind {
    Other,
    Nop,
    LoadConst,
    ReturnValue,
    Jump { target: usize, interruptible: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Slot {
    pub(super) kind: SlotKind,
    pub(super) located: bool,
    #[cfg(test)]
    pub(super) origin: SlotOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CopyPlan {
    pub(super) id: usize,
    #[cfg(test)]
    pub(super) original_target: Option<usize>,
    #[cfg(test)]
    pub(super) original_array_width: usize,
    #[cfg(test)]
    pub(super) final_target: Option<usize>,
    #[cfg(test)]
    pub(super) first_nop: Option<Slot>,
    pub(super) final_slots: Vec<Slot>,
    // Each identity is the actual target copied at that round, never the final return
    // substituted for an intermediate original target.
    #[cfg(test)]
    pub(super) copied_targets: Vec<usize>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CopyKind {
    SmallExit,
    Unlocated,
    SmallExitAndUnlocated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CopyStep {
    /// One-based native copy round; entries retain global emission-order visitation.
    #[cfg(test)]
    pub(super) round: usize,
    pub(super) source: usize,
    pub(super) target: usize,
    #[cfg(test)]
    pub(super) kind: CopyKind,
    #[cfg(test)]
    pub(super) source_width_before: usize,
    pub(super) target_width: usize,
    #[cfg(test)]
    pub(super) source_jump_origin: SlotOrigin,
    #[cfg(test)]
    pub(super) target_tail_origin: SlotOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CopyResult {
    #[cfg(test)]
    pub(super) plans: Vec<CopyPlan>,
    pub(super) steps: Vec<CopyStep>,
}

/// Simulates only a certified acyclic subgraph of original no-fallthrough arrays.
///
/// All source blocks, including normal-try blocks, must be present in native emission
/// order. Located ordinary prefix slots count toward size and no-line eligibility;
/// omitted authored pass NOPs must be supplied explicitly. This function neither
/// removes NOPs nor propagates locations, threads jumps, or infers exception owners.
pub(super) fn plan_normal_try_copies(nodes: &[OriginalNode]) -> Option<CopyResult> {
    if nodes.is_empty() {
        return None;
    }
    let index_of = |id| nodes.iter().position(|node| node.id == id);
    for (index, node) in nodes.iter().enumerate() {
        if index_of(node.id) != Some(index) {
            return None;
        }
        // Every target is declared, and following the original terminal jumps must
        // reach a return. A loop or an external target is not this certificate.
        let mut seen = vec![false; nodes.len()];
        let mut current = index;
        loop {
            if std::mem::replace(&mut seen[current], true) {
                return None;
            }
            match nodes[current].tail {
                OriginalTail::Jump { target, .. } => current = index_of(target)?,
                OriginalTail::ConstantReturn { .. }
                | OriginalTail::LocalTupleReturn { .. }
                | OriginalTail::ComputedReturn { .. } => break,
            }
        }
    }
    let mut plans = Vec::with_capacity(nodes.len());
    let mut total_original_slots = 0usize;
    for node in nodes {
        let mut slots = Vec::new();
        for prefix in &node.prefix {
            slots.push(Slot {
                kind: if prefix.is_nop {
                    SlotKind::Nop
                } else {
                    SlotKind::Other
                },
                located: prefix.located,
                #[cfg(test)]
                origin: SlotOrigin {
                    node: node.id,
                    slot: slots.len(),
                },
            });
        }
        let _original_target = match node.tail {
            OriginalTail::Jump {
                target,
                located,
                interruptible,
            } => {
                slots.push(Slot {
                    kind: SlotKind::Jump {
                        target,
                        interruptible,
                    },
                    located,
                    #[cfg(test)]
                    origin: SlotOrigin {
                        node: node.id,
                        slot: slots.len(),
                    },
                });
                Some(target)
            }
            OriginalTail::ConstantReturn { located } => {
                for (kind, located) in [SlotKind::Nop, SlotKind::LoadConst, SlotKind::ReturnValue]
                    .into_iter()
                    .zip(located)
                {
                    slots.push(Slot {
                        kind,
                        located,
                        #[cfg(test)]
                        origin: SlotOrigin {
                            node: node.id,
                            slot: slots.len(),
                        },
                    });
                }
                None
            }
            OriginalTail::LocalTupleReturn { located } => {
                for (kind, located) in [
                    SlotKind::Other,
                    SlotKind::Other,
                    SlotKind::Other,
                    SlotKind::ReturnValue,
                ]
                .into_iter()
                .zip(located)
                {
                    slots.push(Slot {
                        kind,
                        located,
                        #[cfg(test)]
                        origin: SlotOrigin {
                            node: node.id,
                            slot: slots.len(),
                        },
                    });
                }
                None
            }
            OriginalTail::ComputedReturn { width } => {
                if !(3..=4).contains(&width) {
                    return None;
                }
                for index in 0..width {
                    slots.push(Slot {
                        kind: if index + 1 == width {
                            SlotKind::ReturnValue
                        } else {
                            SlotKind::Other
                        },
                        located: true,
                        #[cfg(test)]
                        origin: SlotOrigin {
                            node: node.id,
                            slot: slots.len(),
                        },
                    });
                }
                None
            }
        };
        total_original_slots = total_original_slots.checked_add(slots.len())?;
        plans.push(CopyPlan {
            id: node.id,
            #[cfg(test)]
            original_target: _original_target,
            #[cfg(test)]
            original_array_width: slots.len(),
            #[cfg(test)]
            final_target: _original_target,
            #[cfg(test)]
            first_nop: None,
            final_slots: slots,
            #[cfg(test)]
            copied_targets: Vec::new(),
        });
    }
    // Each successful copy advances this source along its original acyclic path.
    // There are at most nodes.len()-1 such advances per source, plus a quiescent round.
    let mut steps = Vec::new();
    for _round in 0..=nodes.len() {
        let mut changed = false;
        for source in 0..plans.len() {
            let last = plans[source].final_slots.last()?;
            let SlotKind::Jump {
                target,
                interruptible,
            } = last.kind
            else {
                continue;
            };
            let target_index = index_of(target)?;
            let target_slots = &plans[target_index].final_slots;
            let exits_scope = target_slots.last()?.kind == SlotKind::ReturnValue;
            let all_unlocated = target_slots.iter().all(|slot| !slot.located);
            let small_exit = exits_scope && target_slots.len() <= 4;
            // MAX_COPY_SIZE applies only to scope exits, not to a located small jump.
            if !(small_exit || all_unlocated) {
                continue;
            }
            let step = CopyStep {
                #[cfg(test)]
                round: _round + 1,
                source: plans[source].id,
                target,
                #[cfg(test)]
                kind: match (small_exit, all_unlocated) {
                    (true, true) => CopyKind::SmallExitAndUnlocated,
                    (true, false) => CopyKind::SmallExit,
                    (false, true) => CopyKind::Unlocated,
                    (false, false) => return None,
                },
                #[cfg(test)]
                source_width_before: plans[source].final_slots.len(),
                target_width: target_slots.len(),
                #[cfg(test)]
                source_jump_origin: last.origin,
                #[cfg(test)]
                target_tail_origin: target_slots.last()?.origin,
            };
            let target_slots = target_slots.clone();
            let plan = &mut plans[source];
            if plan.final_slots.len().checked_add(target_slots.len())? > total_original_slots {
                return None;
            }
            // INSTR_SET_OP0 changes only the opcode/argument. Preserve the removed
            // jump's location and origin, then append the target's current array.
            plan.final_slots.last_mut()?.kind = SlotKind::Nop;
            plan.final_slots.extend(target_slots);
            if all_unlocated
                && interruptible
                && let SlotKind::Jump { interruptible, .. } = &mut plan.final_slots.last_mut()?.kind
            {
                *interruptible = true;
            }
            #[cfg(test)]
            plan.copied_targets.push(target);
            steps.push(step);
            changed = true;
        }
        if !changed {
            #[cfg(test)]
            for plan in &mut plans {
                plan.final_target = match plan.final_slots.last()?.kind {
                    SlotKind::Jump { target, .. } => Some(target),
                    SlotKind::ReturnValue => None,
                    _ => return None,
                };
                plan.first_nop = plan
                    .final_slots
                    .iter()
                    .find(|slot| slot.kind == SlotKind::Nop)
                    .copied();
            }
            return Some(CopyResult {
                #[cfg(test)]
                plans,
                steps,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(hops: usize, authored_pass: bool) -> Vec<OriginalNode> {
        let mut nodes = vec![OriginalNode {
            id: 0,
            prefix: vec![],
            tail: OriginalTail::Jump {
                target: 1,
                located: true,
                interruptible: true,
            },
        }];
        for id in 1..=hops {
            let mut prefix = Vec::new();
            if authored_pass && id == 1 {
                prefix.push(PrefixSlot {
                    is_nop: true,
                    located: true,
                });
            }
            prefix.push(PrefixSlot {
                is_nop: true,
                located: false,
            });
            nodes.push(OriginalNode {
                id,
                prefix,
                tail: OriginalTail::Jump {
                    target: id + 1,
                    located: false,
                    interruptible: false,
                },
            });
        }
        nodes.push(OriginalNode {
            id: hops + 1,
            prefix: vec![],
            tail: OriginalTail::ConstantReturn { located: [true; 3] },
        });
        nodes
    }

    #[test]
    fn tuple_return_growth_retains_the_earlier_jump() {
        let mut nodes = (0..3)
            .map(|id| OriginalNode {
                id,
                prefix: Vec::new(),
                tail: OriginalTail::Jump {
                    target: id + 1,
                    located: false,
                    interruptible: false,
                },
            })
            .collect::<Vec<_>>();
        nodes[0].prefix = vec![
            PrefixSlot {
                is_nop: false,
                located: true,
            };
            2
        ];
        nodes.push(OriginalNode {
            id: 3,
            prefix: Vec::new(),
            tail: OriginalTail::LocalTupleReturn { located: [true; 4] },
        });
        let result = plan_normal_try_copies(&nodes).unwrap();
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| (step.round, step.source, step.target, step.target_width))
                .collect::<Vec<_>>(),
            [(1, 0, 1, 1), (1, 1, 2, 1), (1, 2, 3, 4), (2, 1, 3, 4)]
        );
        assert_eq!(result.plans[0].final_target, Some(2));
        assert_eq!(result.plans[2].final_slots.len(), 5);
        assert_eq!(result.plans[1].final_slots.len(), 6);
    }

    #[test]
    fn one_two_three_cleanup_arrays_keep_native_visit_order() {
        let one = plan_normal_try_copies(&chain(1, false)).unwrap().plans;
        assert_eq!(one[0].final_target, None);
        assert_eq!(one[0].final_slots.len(), 6);
        let two = plan_normal_try_copies(&chain(2, false)).unwrap();
        assert_eq!(
            two.steps
                .iter()
                .map(|step| (step.round, step.source, step.target))
                .collect::<Vec<_>>(),
            [(1, 0, 1), (1, 1, 2), (1, 2, 3), (2, 1, 3)]
        );
        assert_eq!(two.steps[0].kind, CopyKind::Unlocated);
        assert_eq!(two.steps[2].kind, CopyKind::SmallExit);
        let two = two.plans;
        assert_eq!(two[0].original_target, Some(1));
        assert_eq!(two[0].final_target, Some(2));
        assert_eq!(two[0].copied_targets, [1]);
        assert_eq!(two[0].final_slots.len(), 3);
        assert_eq!(
            two[0].final_slots.last().unwrap().origin,
            SlotOrigin { node: 1, slot: 1 }
        );
        assert_eq!(two[1].final_target, None);
        assert_eq!(two[1].final_slots.len(), 7);
        assert_eq!(two[2].final_slots.len(), 5);
        let three = plan_normal_try_copies(&chain(3, false)).unwrap().plans;
        assert_eq!(three[0].final_target, None);
        assert_eq!(three[0].copied_targets, [1, 2, 4]);
        assert_eq!(three[0].final_slots.len(), 10);
        // The normal-try source itself stops at a different original cleanup array.
        assert_eq!(three[1].final_target, Some(3));
        assert_eq!(three[1].final_slots.len(), 4);
    }

    #[test]
    fn authored_pass_blocks_copy_even_before_its_target_exits_scope() {
        let plans = plan_normal_try_copies(&chain(2, true)).unwrap().plans;
        assert_eq!(plans[0].final_target, Some(1));
        assert!(plans[0].copied_targets.is_empty());
        assert_eq!(plans[1].original_array_width, 3);
        assert_eq!(plans[1].final_slots.len(), 8);
        assert_eq!(
            plans[1].first_nop.unwrap().origin,
            SlotOrigin { node: 1, slot: 0 }
        );
        assert!(plans[1].first_nop.unwrap().located);
    }

    #[test]
    fn closed_graph_certificate_rejects_missing_targets_and_cycles() {
        let mut nodes = chain(1, false);
        nodes.pop();
        assert!(plan_normal_try_copies(&nodes).is_none());
        nodes[1].tail = OriginalTail::Jump {
            target: 0,
            located: false,
            interruptible: false,
        };
        assert!(plan_normal_try_copies(&nodes).is_none());
    }
}
