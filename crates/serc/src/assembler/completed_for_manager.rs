use super::{
    Assembler, EmittedInstruction, EmittedJump, ExceptionRegion, ExceptionRegionPriority,
    Instruction, Item, Label, Operand, SourceLocation, UnoptimizedCode, trailing_labels,
};
use crate::PythonVersion;
use crate::target::opcodes::{
    END_FOR, FOR_ITER, JUMP_FORWARD, LOAD_FAST, POP_ITER, POP_JUMP_IF_FALSE, POP_JUMP_IF_NONE,
    PUSH_EXC_INFO,
};

#[derive(Debug, Default)]
pub(crate) struct State {
    selected_condition: Option<SourceLocation>,
    guards: Vec<Guard>,
    incoming: Vec<Incoming>,
    exits: Vec<Exit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Head {
    label: Label,
    source: bool,
    block: bool,
    shared: bool,
    incoming: Vec<(usize, Instruction)>,
    regions: Vec<Region>,
}

#[derive(Clone, Copy, Debug)]
struct Region(ExceptionRegion);

impl PartialEq for Region {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.0, other.0);
        a.is_scope_fallback == b.is_scope_fallback
            && a.start == b.start
            && a.end == b.end
            && a.target == b.target
            && a.depth == b.depth
            && a.preserve_lasti == b.preserve_lasti
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
}

impl Eq for Region {}

#[derive(Debug)]
struct Guard {
    record: Record,
    prefix: Vec<Item>,
}

impl Head {
    fn capture(assembler: &Assembler, label: Label) -> Option<Self> {
        Self::capture_with_count(assembler, label, 1)
    }

    fn capture_with_count(assembler: &Assembler, label: Label, count: usize) -> Option<Self> {
        let UnoptimizedCode::Linear(items) = &assembler.code else {
            return None;
        };
        let p = &assembler.exception_owner_provenance;
        if items
            .iter()
            .filter(|item| **item == Item::Label(label))
            .count()
            != count
            || assembler.folded_away_condition_targets.contains(&label)
            || assembler.folded_away_loop_else_targets.contains(&label)
            || assembler.folded_condition_exit_targets.contains(&label)
            || assembler.preserved_block_boundaries.contains(&label)
            || assembler.borrow_unreachable_blocks.contains(&label)
            || assembler.borrow_jump_entry_labels.contains(&label)
            || assembler.borrow_block_aliases.contains(&label)
            || assembler.borrow_loop_exit_blocks.contains_key(&label)
            || assembler.borrow_finally_entries.contains_key(&label)
            || assembler.conditional_threading_targets.contains_key(&label)
            || assembler.original_nop_targets.contains(&label)
            || assembler
                .original_unlocated_cleanup_targets
                .contains(&label)
            || assembler.retained_pop_block_origins.contains_key(&label)
            || assembler.deferred_shared_jump_blocks.contains(&label)
            || p.ordinary_try_setups.contains_key(&label)
            || p.omitted_nop_counts.contains_key(&label)
            || p.omitted_nop_locations.contains_key(&label)
        {
            return None;
        }
        Some(Self {
            label,
            source: p.source_boundaries.contains(&label),
            block: p.block_boundaries.contains(&label),
            shared: assembler.shared_jump_copy_location_blocks.contains(&label),
            incoming: incoming(items, &[label]),
            regions: assembler
                .exception_regions
                .iter()
                .filter(|region| {
                    region.start == label || region.end == label || region.target == label
                })
                .copied()
                .map(Region)
                .collect(),
        })
    }
}

fn incoming(items: &[Item], labels: &[Label]) -> Vec<(usize, Instruction)> {
    items.iter().enumerate().filter_map(|(index, item)| match item {
        Item::Instruction(value) if matches!(value.operand(), Operand::Forward(label) | Operand::Backward(label) if labels.contains(&label)) => Some((index, *value)),
        _ => None,
    }).collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Record {
    emitted: EmittedInstruction,
    begin: usize,
    items: Vec<Item>,
    heads: Vec<Head>,
}

impl Record {
    fn capture(assembler: &Assembler, emitted: EmittedInstruction, begin: usize) -> Option<Self> {
        let UnoptimizedCode::Linear(items) = &assembler.code else {
            return None;
        };
        let index = usize::try_from(emitted.item_index).ok()?;
        if assembler.python_version < PythonVersion::Py314
            || emitted.stream != assembler.emission_stream
            || index + 1 != items.len()
        {
            return None;
        }
        let records = items.get(begin..=index)?;
        let heads = records
            .iter()
            .filter_map(|item| match item {
                Item::Label(label) => Some(*label),
                Item::Instruction(_) => None,
            })
            .map(|label| Head::capture(assembler, label))
            .collect::<Option<Vec<_>>>()?;
        if heads.is_empty()
            || heads
                .iter()
                .any(|head| head.shared || !head.regions.is_empty())
        {
            return None;
        }
        Some(Self {
            emitted,
            begin,
            items: records.to_vec(),
            heads,
        })
    }

    fn instruction(&self) -> Instruction {
        match self.items.last().unwrap() {
            Item::Instruction(value) => *value,
            Item::Label(_) => unreachable!(),
        }
    }

    fn set_instruction(&mut self, value: Instruction) {
        *self.items.last_mut().unwrap() = Item::Instruction(value);
    }

    fn matches(&self, assembler: &Assembler) -> bool {
        let UnoptimizedCode::Linear(items) = &assembler.code else {
            return false;
        };
        self.emitted.stream == assembler.emission_stream
            && usize::try_from(self.emitted.item_index).ok()
                == self.begin.checked_add(self.items.len() - 1)
            && items.get(self.begin..self.begin + self.items.len()) == Some(self.items.as_slice())
            && items
                .iter()
                .filter(|item| **item == Item::Instruction(self.instruction()))
                .count()
                == 1
            && self
                .heads
                .iter()
                .all(|head| Head::capture(assembler, head.label).as_ref() == Some(head))
            && self.heads.iter().all(|head| {
                head.incoming.iter().all(|(_, instruction)| {
                    items
                        .iter()
                        .filter(|item| **item == Item::Instruction(*instruction))
                        .count()
                        == 1
                })
            })
    }
}

#[derive(Debug)]
pub(super) struct Incoming {
    record: Record,
    entry: Label,
    target: Label,
    emptied: bool,
    region: Option<ExceptionRegion>,
    protected: Option<(usize, Vec<Item>)>,
    handler_item: Option<usize>,
}

impl Incoming {
    fn matches(&self, assembler: &Assembler) -> bool {
        let labels = self
            .record
            .heads
            .iter()
            .map(|head| head.label)
            .collect::<Vec<_>>();
        let pairs = assembler
            .exception_owner_provenance
            .normal_try_exits
            .iter()
            .filter(|(entry, target)| labels.contains(entry) || labels.contains(target))
            .collect::<Vec<_>>();
        pairs == [&(self.entry, self.target)]
            && self.record.matches(assembler)
            && self.protected.as_ref().is_none_or(|(start, items)| {
                assembler.linear_items().get(*start..*start + items.len()) == Some(items.as_slice())
            })
            && self.region.is_none_or(|region| {
                assembler
                    .exception_owner_provenance
                    .ordinary_try_setups
                    .get(&region.start)
                    == Some(&region.target)
                    && assembler
                        .linear_items()
                        .iter()
                        .filter(|item| **item == Item::Label(region.start))
                        .count()
                        == 1
                    && incoming(assembler.linear_items(), &[region.start, region.target]).is_empty()
            })
    }
}

#[derive(Debug)]
struct Exit {
    record: Record,
    retained: bool,
}

pub(super) struct BorrowRoute {
    index: usize,
    origins: usize,
    record: Record,
    retained: bool,
}

impl Assembler {
    pub(crate) fn take_completed_for_manager_capture(&mut self) -> State {
        std::mem::take(&mut self.completed_for_manager)
    }

    pub(crate) fn restore_completed_for_manager_capture(&mut self, state: State) {
        self.completed_for_manager = state;
    }

    pub(crate) fn select_completed_for_manager_guard(&mut self, condition: SourceLocation) {
        self.completed_for_manager = State {
            selected_condition: Some(condition),
            ..State::default()
        };
    }

    pub(crate) fn retire_completed_for_manager_capture(&mut self) {
        self.completed_for_manager = State::default();
    }

    /// Captures the original completed-For None test without joining legacy mutable origins.
    pub(super) fn capture_completed_for_none(&mut self, begin: usize, entry: Label) {
        let Some(selected) = self.completed_for_manager.selected_condition else {
            return;
        };
        if !self.completed_for_manager.guards.is_empty() {
            return;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return;
        };
        let Some(
            [
                Item::Label(label),
                Item::Instruction(end),
                Item::Instruction(pop),
                Item::Instruction(load),
                Item::Instruction(guard),
            ],
        ) = items.get(begin..)
        else {
            return;
        };
        let Some(depth) = guard.depth_after() else {
            return;
        };
        let slots = if self.python_version >= PythonVersion::Py315 {
            2
        } else {
            1
        };
        let mut expected = Instruction::new(
            POP_JUMP_IF_NONE,
            guard.operand(),
            guard.location,
            Some(depth),
        );
        expected.local_policy.add_hidden_exception_owner_slots(3);
        if guard.location != selected
            || *label != entry
            || *guard != expected
            || guard.location.line < 0
            || !matches!(guard.operand(), Operand::Forward(_))
            || !matches!(load.operand(), Operand::Value(_))
            || *end
                != Instruction::new(
                    END_FOR,
                    Operand::Value(0),
                    end.location,
                    Some(depth + slots),
                )
            || *pop != Instruction::new(POP_ITER, Operand::Value(0), end.location, Some(depth))
            || *load != Instruction::new(LOAD_FAST, load.operand(), load.location, Some(depth + 1))
        {
            return;
        }
        let emitted = EmittedInstruction {
            stream: self.emission_stream,
            item_index: (items.len() - 1).try_into().unwrap(),
        };
        let Some(record) = Record::capture(self, emitted, begin) else {
            return;
        };
        let [head] = record.heads.as_slice() else {
            return;
        };
        let [(position, source)] = head.incoming.as_slice() else {
            return;
        };
        if !head.source
            || !head.block
            || *position >= begin
            || *source
                != Instruction::new(
                    FOR_ITER,
                    Operand::Forward(entry),
                    source.location,
                    Some(depth + slots + 1),
                )
            || !record.matches(self)
        {
            return;
        }
        let prefix = items[..begin].to_vec();
        self.completed_for_manager
            .guards
            .push(Guard { record, prefix });
    }

    /// Captures a normal-try source immediately before the real shared-location writer.
    pub(super) fn capture_completed_for_shared(&self, emitted: EmittedJump) -> Option<Incoming> {
        if self.completed_for_manager.guards.is_empty() {
            return None;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        let index = usize::try_from(emitted.instruction().item_index).ok()?;
        let heads = trailing_labels(items.get(..index)?).collect::<Vec<_>>();
        if heads.len() != 2 {
            return None;
        }
        let record = Record::capture(self, emitted.instruction(), index.checked_sub(heads.len())?)?;
        let original = record.instruction();
        let Operand::Forward(target) = original.operand() else {
            return None;
        };
        if original
            != Instruction::new(
                JUMP_FORWARD,
                Operand::Forward(target),
                SourceLocation::NONE,
                original.depth_after(),
            )
        {
            return None;
        }
        let entry = heads[0];
        let ordinary = Incoming {
            record,
            entry,
            target,
            emptied: false,
            region: None,
            protected: None,
            handler_item: None,
        };
        let [other, selected] = ordinary.record.heads.as_slice() else {
            return None;
        };
        let [(index, condition)] = other.incoming.as_slice() else {
            return None;
        };
        if selected.label != entry
            || selected.source
            || selected.block
            || !selected.incoming.is_empty()
            || !other.source
            || !other.block
            || *index >= ordinary.record.begin
            || *condition
                != Instruction::new(
                    POP_JUMP_IF_FALSE,
                    Operand::Forward(other.label),
                    condition.location,
                    original.depth_after(),
                )
            || condition.location.line < 0
            || !ordinary.matches(self)
        {
            return None;
        }
        Some(ordinary)
    }

    pub(super) fn seal_completed_for_shared(&mut self, original: Option<Incoming>) {
        let Some(mut origin) = original else {
            return;
        };
        let mut instruction = origin.record.instruction();
        instruction.no_location_policy.preserve_location();
        origin.record.set_instruction(instruction);
        origin.record.heads.last_mut().unwrap().shared = true;
        if origin.matches(self) {
            self.completed_for_manager.incoming.push(origin);
        }
    }

    /// Advances only the normal-finalizer's real unretained-empty write.
    pub(crate) fn mark_completed_for_incoming_empty(&mut self, emitted: EmittedJump) {
        let mut origins = std::mem::take(&mut self.completed_for_manager.incoming);
        let eligible = origins
            .iter()
            .map(|origin| {
                origin.record.emitted == emitted.instruction()
                    && !origin.emptied
                    && origin.region.is_none()
                    && origin.matches(self)
            })
            .collect::<Vec<_>>();
        self.mark_borrow_empty_block_if_unretained(emitted);
        for (origin, valid) in origins.iter_mut().zip(eligible) {
            if valid {
                let mut expected = origin.record.instruction();
                expected.jump_policy.mark_borrow_empty_block();
                origin.record.set_instruction(expected);
                origin.emptied = origin.matches(self);
            }
        }
        self.completed_for_manager.incoming = origins;
    }

    /// Seals the complete unsplit region using labels from the ordinary-try caller.
    pub(crate) fn seal_completed_for_incoming_region(
        &mut self,
        start: Label,
        end: Label,
        target: Label,
        depth: u32,
    ) {
        if self.completed_for_manager.incoming.is_empty() {
            return;
        }
        let region = ExceptionRegion {
            is_scope_fallback: false,
            priority: ExceptionRegionPriority::OriginalRange(start, end),
            start,
            end,
            target,
            depth,
            preserve_lasti: false,
        };
        let mut origins = std::mem::take(&mut self.completed_for_manager.incoming);
        for origin in &mut origins {
            if origin.entry != end
                || !origin.emptied
                || origin.region.is_some()
                || origin.record.instruction().depth_after() != Some(depth)
                || self
                    .exception_regions
                    .iter()
                    .filter(|candidate| Region(**candidate) == Region(region))
                    .count()
                    != 1
            {
                continue;
            }
            let selected = origin.record.heads.last_mut().unwrap();
            if !selected.regions.is_empty() {
                continue;
            }
            selected.regions.push(Region(region));
            if origin.matches(self) {
                let items = self.linear_items();
                if let Some(position) = items.iter().position(|item| *item == Item::Label(start))
                    && position < origin.record.begin
                    && items
                        .iter()
                        .filter(|item| **item == Item::Label(start))
                        .count()
                        == 1
                    && self
                        .exception_owner_provenance
                        .ordinary_try_setups
                        .get(&start)
                        == Some(&target)
                {
                    origin.protected =
                        Some((position, items[position..origin.record.begin].to_vec()));
                    origin.handler_item = Some(items.len());
                    origin.region = Some(region);
                }
            }
        }
        self.completed_for_manager.incoming = origins;
    }

    pub(super) fn capture_completed_for_exit(&self, emitted: EmittedJump) -> Option<Record> {
        if self.completed_for_manager.guards.is_empty() {
            return None;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return None;
        };
        let index = usize::try_from(emitted.instruction().item_index).ok()?;
        let count = trailing_labels(items.get(..index)?).count();
        let record = Record::capture(self, emitted.instruction(), index.checked_sub(count)?)?;
        let original = record.instruction();
        if original.location.line < 0
            || !matches!(original.operand(), Operand::Forward(_))
            || original
                != Instruction::new(
                    JUMP_FORWARD,
                    original.operand(),
                    original.location,
                    original.depth_after(),
                )
            || !record.matches(self)
        {
            return None;
        }
        Some(record)
    }

    pub(super) fn seal_completed_for_exit_empty(&mut self, original: Option<Record>) {
        let Some(mut record) = original else {
            return;
        };
        let mut instruction = record.instruction();
        instruction.jump_policy.mark_borrow_empty_block();
        record.set_instruction(instruction);
        if record.matches(self) {
            self.completed_for_manager.exits.push(Exit {
                record,
                retained: false,
            });
        }
    }

    pub(super) fn capture_completed_for_retention(&self, emitted: EmittedJump) -> Option<usize> {
        let mut origins =
            self.completed_for_manager
                .exits
                .iter()
                .enumerate()
                .filter(|(_, origin)| {
                    !origin.retained
                        && origin.record.emitted == emitted.instruction()
                        && origin.record.matches(self)
                });
        let (index, _) = origins.next()?;
        origins.next().is_none().then_some(index)
    }

    pub(super) fn capture_completed_for_borrow_route(
        &self,
        emitted: EmittedJump,
    ) -> Option<BorrowRoute> {
        let mut origins =
            self.completed_for_manager
                .exits
                .iter()
                .enumerate()
                .filter(|(_, origin)| {
                    origin.record.emitted == emitted.instruction()
                        && origin
                            .record
                            .instruction()
                            .jump_policy
                            .allows_borrow_threading_target()
                        && origin.record.matches(self)
                });
        let (index, origin) = origins.next()?;
        origins.next().is_none().then(|| BorrowRoute {
            index,
            origins: self.completed_for_manager.exits.len(),
            record: origin.record.clone(),
            retained: origin.retained,
        })
    }

    pub(super) fn seal_completed_for_borrow_route(&mut self, original: Option<BorrowRoute>) {
        let Some(original) = original else {
            return;
        };
        let Some(origin) = self.completed_for_manager.exits.get(original.index) else {
            return;
        };
        if self.completed_for_manager.exits.len() != original.origins
            || origin.record != original.record
            || origin.retained != original.retained
        {
            return;
        }
        let mut expected = original.record;
        let mut instruction = expected.instruction();
        instruction.jump_policy.prevent_borrow_threading_target();
        expected.set_instruction(instruction);
        if expected.matches(self) {
            self.completed_for_manager.exits[original.index].record = expected;
        }
    }

    pub(super) fn seal_completed_for_retention(&mut self, index: Option<usize>) {
        let Some(index) = index else {
            return;
        };
        let mut origins = std::mem::take(&mut self.completed_for_manager.exits);
        let origin = &mut origins[index];
        let mut instruction = origin.record.instruction();
        instruction.jump_policy.mark_original_empty_join();
        instruction
            .jump_policy
            .retain_redundant_nop_after_threading();
        origin.record.set_instruction(instruction);
        origin.retained = origin.record.matches(self);
        self.completed_for_manager.exits = origins;
    }

    /// Certifies the immediate completed-For join, without admitting default manager joins.
    pub(crate) fn original_completed_for_manager_join(
        &self,
        emitted: EmittedJump,
        continuation: Label,
        condition: SourceLocation,
        lower: usize,
    ) -> bool {
        if self.completed_for_manager.selected_condition != Some(condition) {
            return false;
        }
        let UnoptimizedCode::Linear(items) = &self.code else {
            return false;
        };
        let mut exits = self.completed_for_manager.exits.iter().filter(|origin| {
            origin.record.emitted == emitted.instruction()
                && origin.retained
                && origin.record.matches(self)
        });
        let Some(exit) = exits.next() else {
            return false;
        };
        let record = &exit.record;
        let instruction = record.instruction();
        if exits.next().is_some()
            || record.begin < lower
            || record.begin + record.items.len() != items.len()
            || instruction.location != condition
            || instruction.operand() != Operand::Forward(continuation)
            || items.contains(&Item::Label(continuation))
            || self
                .exception_regions
                .iter()
                .any(|region| region.target == continuation)
            || !record.heads.iter().any(|head| head.source && head.block)
        {
            return false;
        }
        let labels = record
            .heads
            .iter()
            .map(|head| head.label)
            .collect::<Vec<_>>();
        let Some(future) = Head::capture_with_count(self, continuation, 0) else {
            return false;
        };
        if future.source
            || future.block
            || future.shared
            || !future.regions.is_empty()
            || future.incoming != [(items.len() - 1, instruction)]
        {
            return false;
        }
        let mut unique = super::FxHashSet::default();
        if items
            .iter()
            .any(|item| matches!(item, Item::Label(label) if !unique.insert(*label)))
        {
            return false;
        }
        if self.join_labels_have_authored_location(&labels) {
            return false;
        }
        let edges = incoming(items, &labels);
        if edges.iter().any(|(_, value)| {
            !matches!(value.operand(), Operand::Forward(target)
            if record.heads.iter().any(|head| head.label == target && head.source && head.block))
        }) {
            return false;
        }
        let [
            (guard_index, guard),
            (normal_index, normal),
            (recovery_index, recovery),
        ] = edges.as_slice()
        else {
            return false;
        };
        if *guard_index < lower
            || *recovery_index >= record.begin
            || guard.location != condition
            || guard.depth_after() != instruction.depth_after()
            || normal.depth_after() != instruction.depth_after()
            || recovery.location.line < 0
            || recovery.depth_after() != instruction.depth_after()
            || *recovery
                != Instruction::new(
                    JUMP_FORWARD,
                    recovery.operand(),
                    recovery.location,
                    instruction.depth_after(),
                )
            || !matches!(recovery.operand(), Operand::Forward(_))
            || items
                .iter()
                .filter(|item| **item == Item::Instruction(*recovery))
                .count()
                != 1
            || incoming(items, &[continuation]) != [(items.len() - 1, instruction)]
        {
            return false;
        }
        let mut guards = self.completed_for_manager.guards.iter().filter(|origin| {
            origin.record.begin >= lower
                && usize::try_from(origin.record.emitted.item_index).ok() == Some(*guard_index)
                && origin.record.instruction() == *guard
                && origin.record.matches(self)
                && items.get(..origin.record.begin) == Some(origin.prefix.as_slice())
        });
        let Some(source) = guards.next() else {
            return false;
        };
        let origins = self
            .exception_owner_provenance
            .source_none_conditions
            .iter()
            .filter(|origin| origin.instruction == *guard)
            .collect::<Vec<_>>();
        if guards.next().is_some()
            || !matches!(origins.as_slice(), [origin] if origin.entry == source.record.heads[0].label && origin.labels == [source.record.heads[0].label])
        {
            return false;
        }
        self.completed_for_manager
            .incoming
            .iter()
            .filter(|origin| {
                origin.emptied
                    && origin.region.is_some()
                    && origin.record.begin > *guard_index
                    && usize::try_from(origin.record.emitted.item_index).ok() == Some(*normal_index)
                    && origin.record.instruction() == *normal
                    && origin.matches(self)
                    && origin.region.is_some_and(|region| {
                        origin.handler_item.is_some_and(|position| {
                            items.get(position..position + 2)
                                == Some(&[
                                    Item::Label(region.target),
                                    Item::Instruction(Instruction::new(
                                        PUSH_EXC_INFO,
                                        Operand::Value(0),
                                        SourceLocation::NONE,
                                        Some(region.depth + 2),
                                    )),
                                ])
                                && Head::capture(self, region.target)
                                    .is_some_and(|head| !head.shared && head.incoming.is_empty())
                        })
                    })
            })
            .count()
            == 1
    }
}

#[cfg(test)]
mod tests;
