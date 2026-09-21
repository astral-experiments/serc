use std::cmp::Reverse;

use rustc_hash::{FxHashMap, FxHashSet};

use super::super::block_graph::{BasicBlockId, ItemPosition};
use super::super::{
    Assembler, ConstantTable, ExceptionPolicy, ExceptionRegionPriority, Instruction, Item,
    LayoutAssembler, Operand, OptimizationPipeline, SourceLocation, block_has_fallthrough,
    block_jump_target,
};
use crate::target::opcodes::{
    COMPARE_OP, CONTAINS_OP, COPY, IS_OP, JUMP_BACKWARD, JUMP_BACKWARD_NO_INTERRUPT, JUMP_FORWARD,
    LOAD_CLOSURE, LOAD_CONST, LOAD_FAST, LOAD_FAST_LOAD_FAST, LOAD_GLOBAL, NOP, POP_TOP, PUSH_NULL,
    RETURN_GENERATOR, STORE_FAST, STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST, TO_BOOL, UNARY_NOT,
    YIELD_VALUE,
};
use crate::target::{is_conditional_jump_for, is_unconditional_jump};
use crate::{CompileError, PythonVersion};

// Original priorities are indices into a Vec of IR items, so usize::MAX cannot
// name an original instruction. Reserve it for exact legacy copied owners:
// their selected donor handler takes precedence over destination nesting.
const LEGACY_COPY_PRIORITY_START: usize = usize::MAX;

impl Assembler {
    /// Captures 3.12's exception-stack operand before CFG copying changes protected ranges.
    pub(super) fn prepare_legacy_yield_arguments(&mut self) {
        if self.python_version != PythonVersion::Py312 {
            return;
        }
        let labels = self
            .linear_items()
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let regions = self
            .exception_regions
            .iter()
            .filter_map(|region| {
                let start = *labels.get(&region.start)?;
                let end = *labels.get(&region.end)?;
                let priority = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*labels.get(&start)?, *labels.get(&end)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                Some((start, end, (priority, region.target)))
            })
            .collect::<Vec<_>>();
        for (index, item) in self.linear_items_mut().iter_mut().enumerate() {
            if let Item::Instruction(instruction) = item
                && instruction.opcode == YIELD_VALUE
            {
                let owners = regions
                    .iter()
                    .filter(|(start, end, _)| *start <= index && index < *end)
                    .map(|(_, _, owner)| *owner)
                    .collect::<FxHashSet<_>>();
                instruction.set_operand(Operand::Value(
                    u32::try_from(owners.len()).expect("exception nesting fits in an operand"),
                ));
            }
        }
    }
}

impl OptimizationPipeline {
    /// Runs the pre-3.14 CFG schedule before preparing target-specific instruction layout.
    pub(super) fn optimize_legacy(
        mut self,
        local_count: usize,
        parameter_count: usize,
        mut constants: Option<&mut dyn ConstantTable>,
    ) -> Result<LayoutAssembler, CompileError> {
        let original_arrays = self.legacy_instruction_arrays();
        self.preserve_legacy_setup_nops();
        self.inline_legacy_exit_blocks();
        if self.python_version == PythonVersion::Py313 {
            self.remove_unreachable_instructions();
            self.resolve_legacy_locations(self.legacy_predecessors());
        }
        if let Some(constants) = constants.as_mut() {
            self.optimize_legacy_load_constants(&mut **constants)?;
            self.optimize_legacy_constant_tuples(&mut **constants)?;
        }
        self.optimize_legacy_adjacent_instructions();
        self.thread_legacy_jumps();
        let (swap_blocks, capture_cleanup_swaps, _) = self.optimize_swap_runs();
        self.apply_static_swaps((swap_blocks, capture_cleanup_swaps));
        self.remove_legacy_nops_and_pairs(&original_arrays);
        if self.python_version == PythonVersion::Py312 {
            self.inline_legacy_exit_blocks();
        }
        self.remove_unreachable_instructions();
        loop {
            let before = self.graph.copy_items();
            self.remove_legacy_nops(&original_arrays, false);
            self.remove_redundant_forward_jumps(false);
            if self.python_version == PythonVersion::Py312 || self.graph.copy_items() == before {
                break;
            }
        }
        if let Some(constants) = constants {
            self.prune_legacy_constants(constants);
        }
        self.add_checks_for_uninitialized_loads(local_count, parameter_count);
        if self.python_version == PythonVersion::Py313 {
            for block in self.graph.order().to_vec() {
                self.graph
                    .fuse_adjacent_instructions(block, fuse_legacy_fast_pair);
            }
            self.remove_legacy_nops(&original_arrays, false);
        }
        // Copied instructions retain their donor's handler, even when the copy lies
        // inside another protected range. Cold placement rebuilds ordinary regions
        // by whole block; restore the exact copied spans after that reconstruction.
        let copied_owners = self
            .exception_regions
            .iter()
            .copied()
            .filter(|region| {
                matches!(
                    region.priority,
                    ExceptionRegionPriority::Resolved {
                        start: LEGACY_COPY_PRIORITY_START,
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        // Native cold relocation leaves newly inserted bridge blocks with zero
        // predecessors. Preserve that provenance across physical graph cleanup.
        let blocks_before_cold_relocation =
            self.graph.order().iter().copied().collect::<FxHashSet<_>>();
        self.push_cold_blocks_to_end(None);
        if !copied_owners.is_empty() {
            self.exception_regions.retain(|region| {
                !matches!(
                    region.priority,
                    ExceptionRegionPriority::Resolved {
                        start: LEGACY_COPY_PRIORITY_START,
                        ..
                    }
                )
            });
            self.exception_regions
                .extend(copied_owners.into_iter().filter(|region| {
                    self.graph.label_block(region.start).is_some()
                        && self.graph.label_block(region.end).is_some()
                        && self.graph.label_block(region.target).is_some()
                }));
        }
        // Moving cold handlers can leave a jump to the immediately following block.
        // 3.12 retains its NOP through line resolution; 3.13 compacts it immediately.
        if self.python_version == PythonVersion::Py312 {
            self.remove_legacy_redundant_jumps();
        } else {
            self.remove_legacy_nops_and_jumps(&original_arrays, false);
        }
        // Relocation also drops unreachable physical blocks. Recount surviving
        // edges so their stale predecessors cannot trigger extra exit copies,
        // while retaining native zero counts for the newly inserted bridges.
        let mut predecessors = self.legacy_predecessors();
        predecessors.retain(|block, _| blocks_before_cold_relocation.contains(block));
        self.resolve_legacy_locations(predecessors);
        // LOAD_CLOSURE remains a distinct pseudo-op until after the owned-local
        // fusion pass in 3.13; 3.12 still has a physical LOAD_CLOSURE instruction.
        if self.python_version == PythonVersion::Py313 {
            self.graph.for_each_instruction_mut(|instruction| {
                if instruction.opcode == LOAD_CLOSURE {
                    instruction.opcode = LOAD_FAST;
                }
            });
            self.remove_legacy_nops_and_jumps(&original_arrays, true);
        } else {
            // 3.12 converts POP_BLOCK only after CFG optimization and line resolution.
            self.remove_legacy_nops(&original_arrays, true);
        }
        self.normalize_legacy_jump_directions();
        self.normalize_backward_conditionals()?;
        Ok(LayoutAssembler {
            python_version: self.python_version,
            items: self.graph.into_items(),
            exception_regions: self.exception_regions,
            exception_owner_provenance: self.exception_owner_provenance,
            preserved_block_boundaries: self.preserved_block_boundaries,
            removed_max_depth: None,
            next_label: self.next_label,
            has_next_location: self.has_next_location,
        })
    }

    /// Keeps certified SETUP records present until native pseudo-op conversion.
    fn preserve_legacy_setup_nops(&mut self) {
        let mut labels = FxHashMap::default();
        let mut instructions = Vec::new();
        for block in self.graph.order().iter().copied() {
            for (offset, item) in self.graph[block].items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        labels.insert(*label, instructions.len());
                    }
                    Item::Instruction(_) => instructions.push(ItemPosition::new(block, offset)),
                }
            }
        }
        for (start, end) in &self.exception_owner_provenance.setup_nop_ranges {
            let (Some(&start), Some(&end)) = (labels.get(start), labels.get(end)) else {
                continue;
            };
            if end == start + 1
                && let Some(&position) = instructions.get(start)
            {
                let instruction = self.graph.instruction_mut(position).unwrap();
                if instruction.opcode == NOP {
                    instruction.jump_policy.preserve_explicit_nop();
                }
            }
        }
    }

    /// Captures original CPython instruction arrays before folding removes their jumps.
    ///
    /// Source and preserved CFG entries, jump targets and exception handlers start arrays.
    /// Exception-range endpoints alone do not; they only split Serc's physical blocks.
    pub(super) fn legacy_instruction_arrays(&self) -> FxHashMap<BasicBlockId, usize> {
        let mut boundaries = self.preserved_block_boundaries.clone();
        boundaries.extend(&self.exception_owner_provenance.block_boundaries);
        boundaries.extend(&self.exception_owner_provenance.source_boundaries);
        boundaries.extend(self.exception_regions.iter().map(|region| region.target));
        boundaries.extend(self.graph.iter_items().filter_map(|item| match item {
            Item::Instruction(instruction) => match instruction.operand() {
                Operand::Forward(target) | Operand::Backward(target) => Some(target),
                Operand::Value(_) | Operand::Deferred(_) => None,
            },
            Item::Label(_) => None,
        }));

        let mut arrays = FxHashMap::default();
        let mut array = 0;
        for block in self.graph.order().iter().copied() {
            if self.graph[block]
                .items()
                .iter()
                .any(|item| matches!(item, Item::Label(label) if boundaries.contains(label)))
            {
                array += 1;
            }
            arrays.insert(block, array);
            if self.graph[block]
                .last_instruction()
                .is_some_and(|instruction| {
                    matches!(
                        instruction.operand(),
                        Operand::Forward(_) | Operand::Backward(_)
                    ) || crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                })
            {
                array += 1;
            }
        }
        arrays
    }

    /// Copies native four-slot exit blocks, plus 3.13's unlocated terminal blocks.
    fn inline_legacy_exit_blocks(&mut self) {
        loop {
            let mut changed = false;
            for block in self.graph.order().to_vec() {
                let Some(jump) = self.graph[block].last_instruction().copied() else {
                    continue;
                };
                if !is_unconditional_jump(jump.opcode) {
                    continue;
                }
                let Some(target) = block_jump_target(self.graph[block].items())
                    .and_then(|label| self.graph.label_block(label))
                else {
                    continue;
                };
                if target == block {
                    continue;
                }
                let instructions = self.graph[target]
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        Item::Instruction(instruction) => Some(*instruction),
                        Item::Label(_) => None,
                    })
                    .collect::<Vec<_>>();
                let Some(last) = instructions.last() else {
                    continue;
                };
                let small_exit = crate::target::is_scope_exit_for(self.python_version, last.opcode)
                    && instructions.len() <= 4;
                let unlocated_terminal = self.python_version == PythonVersion::Py313
                    && instructions
                        .iter()
                        .all(|instruction| instruction.location.line < 0)
                    && !block_has_fallthrough(self.graph[target].items());
                if !small_exit && !unlocated_terminal {
                    continue;
                }
                legacy_nop(self.graph.instructions_mut(block).next_back().unwrap());
                for mut item in self.copy_legacy_block_items(target, block) {
                    if let Item::Instruction(instruction) = &mut item
                        && unlocated_terminal
                        && is_unconditional_jump(instruction.opcode)
                        && legacy_jump_has_eval_check(&jump)
                    {
                        // Native JUMP preserves its eval check when its unlocated
                        // successor is copied, including a forward successor.
                        instruction.jump_policy.mark_interruptible_after_threading();
                        if instruction.opcode == JUMP_BACKWARD_NO_INTERRUPT {
                            instruction.opcode = JUMP_BACKWARD;
                        }
                    }
                    let offset = self.graph[block].items().len();
                    self.graph
                        .insert_item(ItemPosition::new(block, offset), item);
                }
                changed = true;
            }
            if self.python_version == PythonVersion::Py312 || !changed {
                break;
            }
        }
    }

    /// Copies the exception target already assigned to each native instruction.
    ///
    /// CPython labels handlers before copying exit blocks. A copied `POP_EXCEPT`
    /// or return must therefore keep its donor's owner, rather than inherit the
    /// protected region surrounding the jump that it replaces.
    fn copy_legacy_block_items(
        &mut self,
        target: BasicBlockId,
        destination: BasicBlockId,
    ) -> Vec<Item> {
        if self.exception_regions.is_empty() {
            return self.graph[target]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(Item::Instruction(*instruction)),
                    Item::Label(_) => None,
                })
                .collect();
        }
        let positions = self
            .graph
            .iter_items()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let regions = self
            .exception_regions
            .iter()
            .enumerate()
            .filter_map(|(index, region)| {
                let start = *positions.get(&region.start)?;
                let end = *positions.get(&region.end)?;
                let (priority_start, priority_end) = match region.priority {
                    ExceptionRegionPriority::OriginalRange(start, end) => {
                        (*positions.get(&start)?, *positions.get(&end)?)
                    }
                    ExceptionRegionPriority::Resolved { start, end } => (start, end),
                };
                Some((
                    start,
                    end,
                    (
                        region.is_scope_fallback,
                        Reverse(priority_start),
                        priority_end,
                        index,
                    ),
                    *region,
                ))
            })
            .collect::<Vec<_>>();
        // Appending inserts before the following block's labels: ranges ending
        // at that boundary still cover the copy, while ranges starting there do not.
        let mut destination_end = 0;
        for block in self.graph.order() {
            destination_end += self.graph[*block].items().len();
            if *block == destination {
                break;
            }
        }
        let destination_owner = regions
            .iter()
            .filter(|(begin, end, _, _)| *begin < destination_end && destination_end <= *end)
            .min_by_key(|(_, _, priority, _)| *priority)
            .map(|(_, _, _, region)| (region.target, region.depth, region.preserve_lasti));
        let mut start = 0;
        for block in self.graph.order() {
            if *block == target {
                break;
            }
            start += self.graph[*block].items().len();
        }
        let instructions = self.graph[target]
            .items()
            .iter()
            .enumerate()
            .filter_map(|(offset, item)| {
                let Item::Instruction(instruction) = item else {
                    return None;
                };
                let owner = if instruction.exception_policy().exclusion()
                    == super::super::ExceptionExclusion::Always
                {
                    None
                } else {
                    regions
                        .iter()
                        .filter(|(begin, end, _, _)| {
                            *begin <= start + offset && start + offset < *end
                        })
                        .min_by_key(|(_, _, priority, _)| *priority)
                        .map(|(_, _, _, region)| *region)
                };
                Some((*instruction, owner))
            })
            .collect::<Vec<_>>();
        let mut copied = Vec::with_capacity(instructions.len());
        let mut index = 0;
        while index < instructions.len() {
            let owner = instructions[index].1;
            let key = owner.map(|region| (region.target, region.depth, region.preserve_lasti));
            let end = (index + 1..instructions.len())
                .find(|next| {
                    instructions[*next]
                        .1
                        .map(|region| (region.target, region.depth, region.preserve_lasti))
                        != key
                })
                .unwrap_or(instructions.len());
            // Avoid adding a boundary when the existing destination region
            // already supplies the exact donor owner.
            let copied_owner = owner.filter(|_| key != destination_owner);
            let copy_start = copied_owner.map(|_| self.label());
            if let Some(start) = copy_start {
                copied.push(Item::Label(start));
            }
            for (mut instruction, _) in instructions[index..end].iter().copied() {
                if owner.is_none() {
                    instruction.set_exception_policy(ExceptionPolicy::AlwaysAbsent);
                }
                copied.push(Item::Instruction(instruction));
            }
            if let Some(mut region) = copied_owner {
                region.start = copy_start.expect("owned copy has a start label");
                region.end = self.label();
                // This is an exact donor instruction owner, including an outer
                // scope handler copied into an otherwise more deeply nested try.
                region.is_scope_fallback = false;
                region.priority = ExceptionRegionPriority::Resolved {
                    start: LEGACY_COPY_PRIORITY_START,
                    end: 0,
                };
                copied.push(Item::Label(region.end));
                self.exception_regions.push(region);
            }
            index = end;
        }
        copied
    }

    fn optimize_legacy_adjacent_instructions(&mut self) {
        let version = self.python_version;
        self.graph
            .for_each_adjacent_instruction_pair_mut(|first, second| {
                if version == PythonVersion::Py312 {
                    if first.opcode == PUSH_NULL
                        && second.opcode == LOAD_GLOBAL
                        && let Operand::Value(argument) = second.operand()
                        && argument & 1 == 0
                    {
                        legacy_nop(first);
                        second.set_operand(Operand::Value(argument | 1));
                    }
                    return;
                }
                if first.opcode == LOAD_GLOBAL
                    && second.opcode == PUSH_NULL
                    && let Operand::Value(argument) = first.operand()
                    && argument & 1 == 0
                {
                    first.set_operand(Operand::Value(argument | 1));
                    legacy_nop(second);
                } else if second.opcode == TO_BOOL {
                    match first.opcode {
                        COMPARE_OP | CONTAINS_OP | IS_OP | UNARY_NOT => {
                            let opcode = first.opcode;
                            let operand = if opcode == COMPARE_OP {
                                let Operand::Value(argument) = first.operand() else {
                                    unreachable!("comparison has a value operand")
                                };
                                Operand::Value(argument | 16)
                            } else {
                                first.operand()
                            };
                            legacy_nop(first);
                            second.opcode = opcode;
                            second.set_operand(operand);
                        }
                        TO_BOOL => legacy_nop(first),
                        _ => {}
                    }
                } else if first.opcode == UNARY_NOT && second.opcode == UNARY_NOT {
                    legacy_nop(first);
                    legacy_nop(second);
                }
            });
        if version == PythonVersion::Py313 {
            self.optimize_redundant_store_fast();
        }
    }

    fn thread_legacy_jumps(&mut self) {
        for block in self.graph.order().to_vec() {
            for _ in 0..self.graph.len() {
                let Some(jump) = self.graph[block].last_instruction().copied() else {
                    break;
                };
                let unconditional = is_unconditional_jump(jump.opcode);
                if (!unconditional && !is_conditional_jump_for(self.python_version, jump.opcode))
                    || (self.python_version == PythonVersion::Py312
                        && jump.opcode == JUMP_BACKWARD_NO_INTERRUPT)
                {
                    break;
                }
                let Some(target) = block_jump_target(self.graph[block].items())
                    .and_then(|label| self.graph.label_block(label))
                else {
                    break;
                };
                let Some(bridge) = self.graph[target]
                    .items()
                    .iter()
                    .find_map(|item| match item {
                        Item::Instruction(instruction) => Some(*instruction),
                        Item::Label(_) => None,
                    })
                else {
                    break;
                };
                // Authored break/continue blocks begin with a temporary native NOP.
                // Its provenance must still block threading after emission elides it.
                // Inlineable unlocated joins use the same flag for a later modern
                // copy phase; they contain no native NOP and remain threadable here.
                if !is_unconditional_jump(bridge.opcode)
                    || (!bridge.jump_policy.allows_threading_target()
                        && !bridge.no_location_policy.allows_block_inlining())
                    // 3.13 does not thread conditional jumps through JUMP_NO_INTERRUPT.
                    // A forward instance shares its wire opcode with ordinary JUMP.
                    || (self.python_version == PythonVersion::Py313
                        && !unconditional
                        && !legacy_jump_has_eval_check(&bridge))
                    || (bridge.opcode == JUMP_BACKWARD_NO_INTERRUPT
                        && self.python_version == PythonVersion::Py312)
                    || bridge.operand() == jump.operand()
                    || (self.python_version == PythonVersion::Py312
                        && bridge.location.line >= 0
                        && bridge.location.line != jump.location.line)
                {
                    break;
                }
                // Threading through JUMP_NO_INTERRUPT must retain an existing eval check.
                let opcode = if unconditional && bridge.opcode != JUMP_BACKWARD_NO_INTERRUPT {
                    bridge.opcode
                } else {
                    jump.opcode
                };
                let instruction = self.graph.instructions_mut(block).next_back().unwrap();
                if self.python_version == PythonVersion::Py312 {
                    instruction.opcode = opcode;
                    instruction.retarget(bridge.operand());
                } else {
                    // 3.13 retains the source line as a NOP and appends a jump carrying
                    // the bridge's location, leaving later NOP cleanup to merge lines.
                    legacy_nop(instruction);
                    let mut threaded = Instruction::synthetic(
                        opcode,
                        bridge.operand(),
                        bridge.location,
                        jump.depth_after(),
                    );
                    if unconditional
                        && (legacy_jump_has_eval_check(&jump)
                            || legacy_jump_has_eval_check(&bridge))
                    {
                        threaded.jump_policy.mark_interruptible();
                    }
                    threaded.set_exception_policy(super::super::ExceptionPolicy::AlwaysAbsent);
                    let offset = self.graph[block].items().len();
                    self.graph.insert_item(
                        ItemPosition::new(block, offset),
                        Item::Instruction(threaded),
                    );
                }
            }
        }
    }

    fn normalize_legacy_jump_directions(&mut self) {
        let labels = self
            .graph
            .iter_items()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Label(label) => Some((*label, index)),
                Item::Instruction(_) => None,
            })
            .collect::<FxHashMap<_, _>>();
        let mut index = 0;
        for block in self.graph.order().to_vec() {
            for item in self.graph.iter_items_mut(block) {
                if let super::super::block_graph::ItemMut::Instruction(instruction) = item
                    && let Operand::Forward(target) | Operand::Backward(target) =
                        instruction.operand()
                {
                    let backward = labels[&target] <= index;
                    instruction.retarget(if backward {
                        Operand::Backward(target)
                    } else {
                        Operand::Forward(target)
                    });
                    if is_unconditional_jump(instruction.opcode) {
                        instruction.opcode = if backward {
                            let interruptible = if self.python_version == PythonVersion::Py313 {
                                legacy_jump_has_eval_check(instruction)
                            } else {
                                instruction.opcode != JUMP_BACKWARD_NO_INTERRUPT
                            };
                            if interruptible {
                                JUMP_BACKWARD
                            } else {
                                JUMP_BACKWARD_NO_INTERRUPT
                            }
                        } else {
                            JUMP_FORWARD
                        };
                    }
                }
                index += 1;
            }
        }
    }

    /// Replaces jumps to the next physical instruction with line-preserving NOPs.
    fn remove_legacy_redundant_jumps(&mut self) -> bool {
        let mut candidate = None;
        let mut redundant = Vec::new();
        for block in self.graph.order().iter().copied() {
            for (offset, item) in self.graph[block].items().iter().enumerate() {
                match item {
                    Item::Label(label) => {
                        if let Some((position, target)) = candidate
                            && target == *label
                        {
                            redundant.push(position);
                            candidate = None;
                        }
                    }
                    Item::Instruction(instruction) => {
                        candidate = None;
                        if is_unconditional_jump(instruction.opcode)
                            && let Operand::Forward(target) | Operand::Backward(target) =
                                instruction.operand()
                        {
                            candidate = Some((ItemPosition::new(block, offset), target));
                        }
                    }
                }
            }
        }
        let changed = !redundant.is_empty();
        for position in redundant {
            legacy_nop(self.graph.instruction_mut(position).unwrap());
        }
        changed
    }

    fn remove_legacy_nops_and_jumps(
        &mut self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
        convert_pseudo_ops: bool,
    ) {
        loop {
            let removed_nops = self.remove_legacy_nops(original_arrays, convert_pseudo_ops);
            let removed_jumps = self.remove_legacy_redundant_jumps();
            if !removed_nops && !removed_jumps {
                break;
            }
        }
    }

    fn remove_legacy_nops_and_pairs(&mut self, original_arrays: &FxHashMap<BasicBlockId, usize>) {
        loop {
            self.remove_legacy_nops(original_arrays, false);
            let mut changed = false;
            let mut previous = None;
            for block in self.graph.order().to_vec() {
                for offset in 0..self.graph[block].items().len() {
                    let Item::Instruction(instruction) = self.graph[block].items()[offset] else {
                        previous = None;
                        continue;
                    };
                    let position = ItemPosition::new(block, offset);
                    if instruction.opcode == POP_TOP
                        && let Some(previous_position) = previous
                    {
                        let predecessor = *self.graph.instruction_mut(previous_position).unwrap();
                        if predecessor.opcode == LOAD_CONST
                            || (predecessor.opcode == COPY
                                && predecessor.operand() == Operand::Value(1))
                        {
                            // Preserve both spans until the next NOP cleanup decides
                            // which records are required for line tracing.
                            legacy_nop(self.graph.instruction_mut(previous_position).unwrap());
                            legacy_nop(self.graph.instruction_mut(position).unwrap());
                            changed = true;
                        }
                    }
                    previous = Some(position);
                }
                if !block_has_fallthrough(self.graph[block].items())
                    || self.graph[block]
                        .last_instruction()
                        .is_some_and(|instruction| {
                            instruction.opcode.has_jump_for(self.python_version)
                        })
                {
                    previous = None;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn remove_legacy_nops(
        &mut self,
        original_arrays: &FxHashMap<BasicBlockId, usize>,
        convert_pseudo_ops: bool,
    ) -> bool {
        let preserve_pop_blocks =
            self.python_version == PythonVersion::Py312 && !convert_pseudo_ops;
        let mut changed = false;
        let mut previous_array = None;
        let mut previous_line = -1;
        let order = self.graph.order().to_vec();
        for (block_index, block) in order.iter().copied().enumerate() {
            let array = original_arrays.get(&block).copied();
            // Copies inserted after the snapshot have their own native arrays.
            if array.is_none() || array != previous_array {
                previous_line = -1;
            }
            previous_array = array;
            let positions = self.graph[block]
                .items()
                .iter()
                .enumerate()
                .filter_map(|(offset, item)| match item {
                    Item::Instruction(_) => Some(ItemPosition::new(block, offset)),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            let mut removed = Vec::new();
            for (index, position) in positions.iter().copied().enumerate() {
                let instruction = *self.graph.instruction_mut(position).unwrap();
                // SETUP records, and 3.12 POP_BLOCK, stay real records until
                // pseudo-op conversion. Their locations still precede later NOPs.
                if instruction.opcode == NOP
                    && (convert_pseudo_ops || !instruction.jump_policy.preserves_explicit_nop())
                    && !(preserve_pop_blocks && instruction.fusion_policy.is_converted_pop_block())
                {
                    let mut remove =
                        instruction.location.line < 0 || instruction.location.line == previous_line;
                    if !remove {
                        if let Some(next) = positions.get(index + 1) {
                            let next = self.graph.instruction_mut(*next).unwrap();
                            if next.location.line < 0 {
                                next.location = instruction.location;
                                remove = true;
                            } else {
                                remove = next.location.line == instruction.location.line;
                            }
                        } else {
                            let next = order[block_index + 1..].iter().find_map(|block| {
                                self.graph[*block]
                                    .items()
                                    .iter()
                                    .find_map(|item| match item {
                                        Item::Instruction(next)
                                            if next.opcode != NOP
                                                || next.location.line >= 0
                                                || (!convert_pseudo_ops
                                                    && next
                                                        .jump_policy
                                                        .preserves_explicit_nop())
                                                || (preserve_pop_blocks
                                                    && next
                                                        .fusion_policy
                                                        .is_converted_pop_block()) =>
                                        {
                                            Some(next.location.line)
                                        }
                                        _ => None,
                                    })
                            });
                            remove = next == Some(instruction.location.line);
                        }
                    }
                    if remove {
                        removed.push(position);
                        continue;
                    }
                }
                previous_line = instruction.location.line;
            }
            changed |= !removed.is_empty();
            self.graph.remove_ordered_instructions(&removed);
            if self.graph[block]
                .last_instruction()
                .is_some_and(|instruction| {
                    matches!(
                        instruction.operand(),
                        Operand::Forward(_) | Operand::Backward(_)
                    ) || crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                })
            {
                previous_array = None;
            }
        }
        changed
    }

    fn legacy_predecessors(&self) -> FxHashMap<BasicBlockId, usize> {
        let mut predecessors = FxHashMap::default();
        for (index, block) in self.graph.order().iter().copied().enumerate() {
            if block_has_fallthrough(self.graph[block].items())
                && let Some(next) = self.graph.order().get(index + 1)
            {
                *predecessors.entry(*next).or_default() += 1;
            }
            if let Some(target) = block_jump_target(self.graph[block].items())
                .and_then(|label| self.graph.label_block(label))
            {
                *predecessors.entry(target).or_default() += 1;
            }
        }
        predecessors
    }

    fn resolve_legacy_locations(&mut self, mut predecessors: FxHashMap<BasicBlockId, usize>) {
        for block in self.graph.order().to_vec() {
            let Some(jump) = self.graph[block].last_instruction().copied() else {
                continue;
            };
            let Some(target) = block_jump_target(self.graph[block].items())
                .and_then(|label| self.graph.label_block(label))
            else {
                continue;
            };
            if predecessors.get(&target).copied().unwrap_or(0) <= 1 {
                continue;
            }
            let instructions = self.graph[target]
                .items()
                .iter()
                .filter_map(|item| match item {
                    Item::Instruction(instruction) => Some(*instruction),
                    Item::Label(_) => None,
                })
                .collect::<Vec<_>>();
            if !instructions.last().is_some_and(|instruction| {
                crate::target::is_scope_exit_for(self.python_version, instruction.opcode)
                    // 3.13 also copies unlocated eval-check blocks. A shared
                    // backedge must inherit each condition's complete span before
                    // jump threading removes the original conditional record.
                    || (self.python_version == PythonVersion::Py313
                        && legacy_jump_has_eval_check(instruction))
            }) || instructions
                .iter()
                .any(|instruction| instruction.location.line >= 0)
            {
                continue;
            }
            let entry = self.label();
            let mut copied = self.copy_legacy_block_items(target, target);
            let Some(Item::Instruction(first)) = copied
                .iter_mut()
                .find(|item| matches!(item, Item::Instruction(_)))
            else {
                unreachable!("eligible legacy exit contains an instruction");
            };
            first.location = jump.location;
            let index = self
                .graph
                .order()
                .iter()
                .position(|id| *id == target)
                .unwrap()
                + 1;
            let copy = self.graph.insert_block(
                index,
                std::iter::once(Item::Label(entry)).chain(copied).collect(),
            );
            self.graph
                .instructions_mut(block)
                .next_back()
                .unwrap()
                .retarget(Operand::Forward(entry));
            *predecessors.get_mut(&target).unwrap() -= 1;
            predecessors.insert(copy, 1);
        }
        let order = self.graph.order().to_vec();
        for (index, block) in order.iter().copied().enumerate() {
            let mut location = SourceLocation::NONE;
            let mut follows_generator_prefix = false;
            for instruction in self.graph.instructions_mut(block) {
                // 3.12 inserts this startup POP_TOP after CFG line propagation. It
                // retains NO_LOCATION even though RETURN_GENERATOR has a source line.
                let inserted_generator_pop = self.python_version == PythonVersion::Py312
                    && follows_generator_prefix
                    && instruction.opcode == POP_TOP;
                if instruction.location.line < 0 {
                    // Modern preservation flags do not apply to native legacy CFG
                    // propagation; only later prefix insertion escapes this phase.
                    if !inserted_generator_pop {
                        instruction.location = location;
                    }
                } else {
                    location = instruction.location;
                }
                follows_generator_prefix = instruction.opcode == RETURN_GENERATOR;
            }
            let fallthrough = block_has_fallthrough(self.graph[block].items())
                .then(|| order.get(index + 1).copied())
                .flatten();
            let target = block_jump_target(self.graph[block].items())
                .and_then(|label| self.graph.label_block(label));
            for successor in fallthrough.into_iter().chain(target) {
                if predecessors.get(&successor) == Some(&1)
                    && let Some(first) = self.graph.instructions_mut(successor).next()
                    && first.location.line < 0
                {
                    first.location = location;
                }
            }
        }
    }
}

/// Recovers native 3.13's direction-independent JUMP pseudo-op.
///
/// Forward JUMP and JUMP_NO_INTERRUPT share a wire opcode. Authored breaks
/// carry the ordinary kind in their emission certificate; copied and threaded
/// jumps retain it through the interruptible policy bit.
fn legacy_jump_has_eval_check(instruction: &Instruction) -> bool {
    is_unconditional_jump(instruction.opcode)
        && (instruction.opcode == JUMP_BACKWARD
            || instruction.jump_policy.is_interruptible()
            || instruction.fusion_policy.is_authored_break())
}

fn legacy_nop(instruction: &mut Instruction) {
    instruction.opcode = NOP;
    instruction.set_operand(Operand::Value(0));
}

fn fuse_legacy_fast_pair(first: &Instruction, second: &Instruction) -> Option<Instruction> {
    // CPython still represents these records as LOAD_CLOSURE and
    // STORE_FAST_MAYBE_NULL when it forms 3.13's owned-local superinstructions.
    if first.fusion_policy.prevents_with_next()
        || second.fusion_policy.prevents_with_previous()
        || first.local_policy.is_post_analysis_closure_load()
        || second.local_policy.is_post_analysis_closure_load()
        || first.local_policy.may_store_null()
        || second.local_policy.may_store_null()
    {
        return None;
    }
    let opcode = match (first.opcode, second.opcode) {
        (LOAD_FAST, LOAD_FAST) => LOAD_FAST_LOAD_FAST,
        (STORE_FAST, LOAD_FAST) => STORE_FAST_LOAD_FAST,
        (STORE_FAST, STORE_FAST) => STORE_FAST_STORE_FAST,
        _ => return None,
    };
    let (Operand::Value(first_argument), Operand::Value(second_argument)) =
        (first.operand(), second.operand())
    else {
        return None;
    };
    if first_argument >= 16
        || second_argument >= 16
        || (first.location.line >= 0
            && second.location.line >= 0
            && first.location.line != second.location.line)
    {
        return None;
    }
    Some(Instruction::fused(
        opcode,
        Operand::Value((first_argument << 4) | second_argument),
        *first,
        *second,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::{ExceptionPolicy, Label};
    use crate::target::opcodes::{
        LOAD_CONST, POP_JUMP_IF_FALSE, RERAISE, RETURN_CONST, RETURN_VALUE,
    };

    #[test]
    fn legacy_exit_copies_keep_donor_exception_owners() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        // Exact tables from pinned CPython 3.12.14 / 3.13.15. These independent
        // public cases have matching code bytes but used to retain the caller's
        // handler on copied returns and POP_EXCEPT/name-clearing instructions.
        let cases: &[(PythonVersion, &str, &[u8])] = &[
            (
                PythonVersion::Py312,
                "def terminal_try_manager(manager, work, Error, handle):\n    try:\n        with manager:\n            work()\n    except Error:\n        handle()\n",
                &[
                    130, 2, 33, 0, 132, 8, 21, 3, 140, 8, 33, 0, 149, 5, 30, 7, 154, 3, 33, 0, 158,
                    3, 33, 0, 161, 12, 48, 3, 175, 1, 48, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                "def terminal_try_manager(manager, work, Error, handle):\n    try:\n        with manager:\n            work()\n    except Error:\n        handle()\n",
                &[
                    130, 2, 38, 0, 132, 8, 21, 3, 140, 8, 38, 0, 149, 10, 35, 7, 159, 3, 38, 0,
                    163, 3, 38, 0, 166, 13, 54, 3, 181, 1, 54, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                "def named_handler_reraise(work, Error, handled, finish):\n    try:\n        work()\n    except Error as error:\n        if handled(error):\n            pass\n        else:\n            raise\n    finish()\n",
                &[
                    130, 7, 17, 0, 145, 6, 47, 3, 151, 13, 42, 3, 169, 1, 42, 3, 170, 5, 47, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                "def manager_nested_handler_finally(manager, work, Error, handle, cleanup, finish):\n    with manager:\n        try:\n            work()\n        except Error as error:\n            try:\n                handle(error)\n            finally:\n                cleanup()\n    finish()\n",
                &[
                    131, 1, 65, 11, 3, 133, 7, 28, 2, 156, 6, 65, 8, 5, 163, 8, 55, 4, 171, 7, 65,
                    3, 5, 178, 5, 65, 11, 3, 183, 9, 65, 0, 7, 193, 0, 3, 65, 3, 5, 193, 3, 5, 65,
                    8, 5, 193, 8, 3, 65, 11, 3, 193, 11, 10, 65, 25, 7,
                ],
            ),
        ];
        for &(version, source, expected) in cases {
            let module = compile_with_options(
                source,
                "legacy_copy_owners.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                function.exception_table, expected,
                "{version:?}: {}",
                function.name
            );
        }
    }

    #[test]
    fn legacy_generator_break_resolves_none_before_constant_optimization() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let sources = [
            "def f(xs):\n    for x in xs:\n        yield x\n        break\n",
            "def f():\n    while True:\n        value = yield 7\n        if value:\n            break\n",
            "def f(xs):\n    for x in xs:\n        yield x\n        if x:\n            break\n        yield 7\n",
            "def f(xs):\n    \"A public generator docstring.\"\n    for x in xs:\n        yield x\n        break\n",
        ];
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for (index, source) in sources.iter().enumerate() {
                let module = compile_with_options(
                    source,
                    "generator_break.py",
                    &CompileOptions {
                        python_version: version,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .code;
                let function = module
                    .constants
                    .iter()
                    .find_map(|constant| match constant {
                        Constant::Code(code) if code.name == "f" => Some(code),
                        _ => None,
                    })
                    .unwrap();
                // Both pinned native compilers preserve admission order. A docstring
                // occupies slot zero, so the break must use the actual None index.
                let none_index = u8::from(index == 3);
                if index == 1 {
                    // Native constant-true loops retain the original backedge after
                    // the break return, without synthesizing an ownerless edge.
                    let expected: &[u8] = if version == PythonVersion::Py312 {
                        &[
                            75, 0, 1, 0, 151, 0, 9, 0, 100, 1, 150, 1, 151, 1, 125, 0, 124, 0, 114,
                            1, 121, 0, 140, 8, 173, 3, 119, 1,
                        ]
                    } else {
                        &[
                            35, 0, 32, 0, 149, 0, 30, 0, 83, 1, 118, 0, 149, 5, 110, 0, 85, 0, 40,
                            0, 0, 0, 0, 0, 0, 0, 97, 1, 0, 0, 103, 0, 77, 14, 0, 0, 55, 3, 102, 1,
                        ]
                    };
                    assert_eq!(function.bytecode, expected, "{version:?}: {source}");
                }
                assert!(
                    match index {
                        0 => matches!(function.constants.as_slice(), [Constant::None]),
                        3 => matches!(
                            function.constants.as_slice(),
                            [Constant::String(doc), Constant::None]
                                if doc == "A public generator docstring."
                        ),
                        _ => matches!(
                            function.constants.as_slice(),
                            [Constant::None, Constant::Int(7)]
                        ),
                    },
                    "{version:?}: {source}: {:?}",
                    function.constants
                );
                assert!(
                    function
                        .bytecode
                        .as_chunks::<2>()
                        .0
                        .contains(&[RETURN_CONST.code_for(version), none_index]),
                    "{version:?}: {source}"
                );
            }
        }
    }

    #[test]
    fn legacy_conditional_continue_retains_the_authored_jump_block() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(xs, a, b):\n    for x in xs:\n        if a or b:\n            continue\n        yield x\n";
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            let module = compile_with_options(
                source,
                "cfg.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "f" => Some(code),
                    _ => None,
                })
                .unwrap();
            // Captured from pinned CPython 3.12.14 and 3.13.15. The continue's
            // temporary NOP prevents the first conditional from being threaded
            // into a duplicated backedge with a gap in the generator handler.
            let (bytecode, exception_table): (&[u8], &[u8]) = match version {
                PythonVersion::Py312 => (
                    &[
                        75, 0, 1, 0, 151, 0, 124, 0, 68, 0, 93, 11, 0, 0, 125, 3, 124, 1, 115, 2,
                        124, 2, 114, 1, 140, 8, 124, 3, 150, 1, 151, 1, 1, 0, 140, 13, 4, 0, 121,
                        0, 173, 3, 119, 1,
                    ],
                    &[130, 18, 20, 1],
                ),
                PythonVersion::Py313 => (
                    &[
                        35, 0, 32, 0, 149, 0, 85, 0, 19, 0, 72, 23, 0, 0, 110, 3, 85, 1, 40, 0, 0,
                        0, 0, 0, 0, 0, 100, 7, 0, 0, 85, 2, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0,
                        77, 19, 0, 0, 85, 3, 118, 0, 149, 5, 32, 0, 77, 25, 0, 0, 11, 0, 32, 0,
                        103, 0, 55, 3, 102, 1,
                    ],
                    &[130, 31, 33, 1],
                ),
                _ => unreachable!(),
            };
            assert_eq!(function.bytecode, bytecode, "{version:?}");
            assert_eq!(function.exception_table, exception_table, "{version:?}");
        }
    }

    #[test]
    fn legacy_implicit_backedge_remains_threadable() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(xs):\n for x in xs:\n  if x: break\n else: return 1\n return 2\n";
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            let module = compile_with_options(
                source,
                "cfg.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "f" => Some(code),
                    _ => None,
                })
                .unwrap();
            // Unlike an authored continue, the loop's implicit unlocated jump
            // has no leading NOP. Native threading exposes a backward conditional
            // that final normalization reverses before the break cleanup.
            let expected: &[u8] = if version == PythonVersion::Py312 {
                &[
                    151, 0, 124, 0, 68, 0, 93, 6, 0, 0, 125, 1, 124, 1, 115, 1, 140, 6, 1, 0, 121,
                    2, 4, 0, 121, 1,
                ]
            } else {
                &[
                    149, 0, 85, 0, 19, 0, 72, 12, 0, 0, 110, 1, 85, 1, 40, 0, 0, 0, 0, 0, 0, 0,
                    100, 2, 0, 0, 77, 12, 0, 0, 32, 0, 103, 2, 11, 0, 32, 0, 103, 1,
                ]
            };
            assert_eq!(function.bytecode, expected, "{version:?}");
        }
    }

    #[test]
    fn python_312_try_body_pop_block_prevents_early_conditional_threading() {
        use crate::{CompileOptions, compile_with_options};

        let source = "try:\n    first = acquire()\n    second = prepare(first)\n    if (second / \"item\").ready():\n        consume(second)\nexcept Failure:\n    recover()\nfinish()\n";
        let code = compile_with_options(
            source,
            "terminal_try_condition.py",
            &CompileOptions {
                python_version: PythonVersion::Py312,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        // The shared unlocated POP_BLOCK keeps both normal predecessors until
        // native location propagation. It then disappears without a line event.
        // These boundaries are captured from pinned CPython 3.12.14.
        assert_eq!(code.bytecode.len(), 140);
        assert_eq!(code.bytecode[70..72], [114, 8]);
        assert_eq!(
            code.exception_table,
            [130, 42, 52, 0, 180, 12, 65, 3, 3, 193, 2, 1, 65, 3, 3]
        );
    }

    #[test]
    fn python_313_conditional_retains_no_interrupt_join() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(a, b, c):\n    if a:\n        if b:\n            raise ValueError\n    elif c:\n        raise TypeError\n    result(a)\n";
        let module = compile_with_options(
            source,
            "cfg.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        // CPython 3.13.15 keeps the inner false branch targeting the separate
        // JUMP_NO_INTERRUPT join, even after assigning that jump a source line.
        assert_eq!(
            function.bytecode,
            [
                149, 0, 85, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 14, 0, 0, 85, 1, 40, 0, 0, 0, 0, 0, 0,
                0, 97, 6, 0, 0, 91, 0, 0, 0, 0, 0, 0, 0, 0, 0, 101, 1, 79, 13, 85, 2, 40, 0, 0, 0,
                0, 0, 0, 0, 97, 6, 0, 0, 91, 2, 0, 0, 0, 0, 0, 0, 0, 0, 101, 1, 91, 5, 0, 0, 0, 0,
                0, 0, 0, 0, 85, 0, 53, 1, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0,
            ]
        );
    }

    #[test]
    fn python_313_break_copy_retains_its_eval_check() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def f(values):\n    try:\n        work()\n    except Error as error:\n        for value in values:\n            break\n    finish()\n";
        let module = compile_with_options(
            source,
            "cfg.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .code;
        let function = module
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Code(code) if code.name == "f" => Some(code),
                _ => None,
            })
            .unwrap();
        // Pinned CPython 3.13.15 copies the named-handler cleanup onto the
        // authored break. That copy keeps JUMP's eval check, while normal
        // exhaustion retains the cleanup's original JUMP_NO_INTERRUPT.
        assert_eq!(
            function.bytecode,
            [
                149, 0, 30, 0, 91, 1, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 91,
                5, 0, 0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 91, 2, 0,
                0, 0, 0, 0, 0, 0, 0, 7, 0, 97, 24, 0, 0, 110, 1, 85, 0, 19, 0, 72, 8, 0, 0, 110, 2,
                32, 0, 31, 0, 83, 0, 110, 1, 65, 1, 77, 33, 0, 0, 11, 0, 32, 0, 31, 0, 83, 0, 110,
                1, 65, 1, 78, 40, 83, 0, 110, 1, 65, 1, 102, 1, 102, 0, 61, 3, 31, 0, 102, 1,
            ]
        );
        assert_eq!(
            function.exception_table,
            [
                130, 10, 23, 0, 151, 10, 57, 3, 161, 6, 52, 3, 173, 2, 52, 3, 180, 5, 57, 3
            ]
        );
    }

    #[test]
    fn legacy_setup_nop_keeps_its_span_until_pseudo_conversion() {
        let start = Label(0);
        let end = Label(1);
        let setup_location = SourceLocation::new(2, 3, 1, 16);
        let return_location = SourceLocation::new(2, 2, 6, 16);
        let mut assembler = Assembler::from_linear_items(vec![
            Item::Label(start),
            Item::Instruction(Instruction::new(
                NOP,
                Operand::Value(0),
                setup_location,
                None,
            )),
            Item::Label(end),
            Item::Instruction(Instruction::new(
                NOP,
                Operand::Value(0),
                return_location,
                None,
            )),
            Item::Instruction(Instruction::new(
                RETURN_CONST,
                Operand::Value(0),
                SourceLocation::new(3, 3, 10, 16),
                None,
            )),
        ]);
        assembler.python_version = PythonVersion::Py313;
        assembler.next_label = 2;
        assembler.record_setup_nop_boundaries(start, end);
        let mut pipeline = assembler.into_optimization_pipeline();
        let original_arrays = pipeline.legacy_instruction_arrays();
        pipeline.preserve_legacy_setup_nops();
        pipeline.remove_legacy_nops(&original_arrays, false);
        pipeline.remove_legacy_nops(&original_arrays, true);
        let locations = pipeline
            .graph
            .iter_items()
            .filter_map(|item| match item {
                Item::Instruction(instruction) if instruction.opcode == NOP => {
                    Some(instruction.location)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(locations, [setup_location]);
        pipeline.graph.validate().unwrap();
    }

    #[test]
    fn legacy_small_exit_copying_includes_raising_blocks() {
        for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
            for opcode in [crate::target::opcodes::RAISE_VARARGS, RERAISE] {
                let target = Label(0);
                let location = SourceLocation::new(2, 2, 0, 5);
                let exit = Instruction::new(opcode, Operand::Value(0), location, Some(0));
                let mut assembler = Assembler::from_linear_items(vec![
                    Item::Instruction(Instruction::new(
                        JUMP_FORWARD,
                        Operand::Forward(target),
                        location,
                        Some(0),
                    )),
                    Item::Label(target),
                    Item::Instruction(exit),
                ]);
                assembler.python_version = python_version;
                assembler.next_label = 1;
                let mut pipeline = assembler.into_optimization_pipeline();
                pipeline.inline_legacy_exit_blocks();
                let first = pipeline.graph.block_at(0);
                assert_eq!(pipeline.graph[first].last_instruction(), Some(&exit));
                pipeline.graph.validate().unwrap();
            }
        }
    }

    #[test]
    fn legacy_nops_respect_original_arrays_across_exception_range_ends() {
        for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
            for (source_boundary, removed_jump) in [
                (false, None),
                (true, None),
                (false, Some(JUMP_FORWARD)),
                (false, Some(POP_JUMP_IF_FALSE)),
            ] {
                let start = Label(0);
                let range_end = Label(1);
                let handler = Label(2);
                let location = SourceLocation::new(2, 2, 0, 1);
                let mut marker = Instruction::new(NOP, Operand::Value(0), location, None);
                marker.fusion_policy.mark_converted_pop_block();
                marker.set_exception_policy(ExceptionPolicy::EXCLUDE_ALWAYS);
                let mut items = vec![
                    Item::Label(start),
                    Item::Instruction(Instruction::new(
                        LOAD_FAST,
                        Operand::Value(0),
                        location,
                        None,
                    )),
                ];
                if let Some(opcode) = removed_jump {
                    items.push(Item::Instruction(Instruction::new(
                        opcode,
                        Operand::Forward(range_end),
                        location,
                        None,
                    )));
                }
                items.extend([
                    Item::Label(range_end),
                    Item::Instruction(marker),
                    Item::Instruction(Instruction::new(
                        LOAD_CONST,
                        Operand::Value(0),
                        SourceLocation::new(3, 3, 0, 1),
                        None,
                    )),
                    Item::Instruction(Instruction::new(
                        RETURN_VALUE,
                        Operand::Value(0),
                        SourceLocation::new(3, 3, 0, 1),
                        None,
                    )),
                    Item::Label(handler),
                    Item::Instruction(Instruction::new(
                        RERAISE,
                        Operand::Value(0),
                        SourceLocation::NONE,
                        None,
                    )),
                ]);
                let mut assembler = Assembler::from_linear_items(items);
                assembler.python_version = python_version;
                assembler.next_label = 3;
                assembler.add_exception_region(start, range_end, handler, 0, false);
                if source_boundary {
                    assembler.record_exception_owner_block_boundary(range_end);
                }
                let mut pipeline = assembler.into_optimization_pipeline();
                let original_arrays = pipeline.legacy_instruction_arrays();
                if removed_jump.is_some() {
                    let block = pipeline.graph.label_block(start).unwrap();
                    legacy_nop(pipeline.graph.instructions_mut(block).next_back().unwrap());
                }
                pipeline.remove_legacy_nops(&original_arrays, true);
                assert_eq!(
                    pipeline
                        .graph
                        .iter_items()
                        .any(|item| *item == Item::Instruction(marker)),
                    source_boundary || removed_jump.is_some(),
                );
                pipeline.graph.validate().unwrap();
            }
        }
    }

    #[test]
    fn legacy_constant_if_expression_arms_fold_after_exit_copying() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def conditional_true(manager, values, choose, check):\n    with manager:\n        for value in values:\n            assert check(value) if choose(value) else True\n";
        // Exact pinned native code and tables from the independent public reducer.
        let cases: &[(PythonVersion, &[u8], &[u8])] = &[
            (
                PythonVersion::Py312,
                &[
                    151, 0, 124, 0, 53, 0, 1, 0, 124, 1, 68, 0, 93, 21, 0, 0, 125, 4, 2, 0, 124, 2,
                    124, 4, 171, 1, 0, 0, 0, 0, 0, 0, 114, 11, 2, 0, 124, 3, 124, 4, 171, 1, 0, 0,
                    0, 0, 0, 0, 114, 1, 140, 20, 74, 0, 130, 1, 140, 23, 4, 0, 9, 0, 100, 0, 100,
                    0, 100, 0, 171, 2, 0, 0, 0, 0, 0, 0, 1, 0, 121, 0, 35, 0, 49, 0, 115, 1, 119,
                    2, 1, 0, 89, 0, 1, 0, 1, 0, 121, 0, 120, 3, 89, 0, 119, 1,
                ],
                &[131, 22, 40, 3, 154, 4, 40, 3, 168, 5, 49, 7],
            ),
            (
                PythonVersion::Py313,
                &[
                    149, 0, 85, 0, 2, 0, 32, 0, 85, 1, 19, 0, 72, 33, 0, 0, 110, 4, 85, 2, 34, 0,
                    85, 4, 53, 1, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 17, 0, 0, 85, 3,
                    34, 0, 85, 4, 53, 1, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 97, 2, 0, 0,
                    77, 31, 0, 0, 23, 0, 101, 1, 77, 35, 0, 0, 11, 0, 32, 0, 83, 0, 83, 0, 83, 0,
                    53, 2, 0, 0, 0, 0, 0, 0, 32, 0, 103, 0, 33, 0, 44, 0, 40, 0, 0, 0, 0, 0, 0, 0,
                    100, 1, 0, 0, 102, 2, 32, 0, 31, 0, 32, 0, 32, 0, 103, 0, 61, 3, 31, 0, 102, 1,
                ],
                &[
                    131, 30, 52, 3, 165, 2, 52, 3, 169, 2, 52, 3, 180, 10, 65, 2, 7,
                ],
            ),
        ];
        for &(version, bytecode, table) in cases {
            let module = compile_with_options(
                source,
                "legacy_constant_arms.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.bytecode, bytecode, "{version:?}");
            assert_eq!(function.exception_table, table, "{version:?}");
        }
    }

    #[test]
    fn legacy_with_suppression_jump_keeps_version_specific_owner() {
        use crate::code_object::Constant;
        use crate::{CompileOptions, compile_with_options};

        let source = "def ordinary_try(manager, work, Error, handle, finish):\n    try:\n        with manager:\n            work()\n    except Error:\n        handle()\n    finish()\n";
        // Pinned CPython 3.12.14 keeps the enclosing handler on the suppression
        // JUMP_BACKWARD. CPython 3.13.15 copies the ownerless outer-try continuation.
        let tables: &[(PythonVersion, &[u8])] = &[
            (
                PythonVersion::Py312,
                &[
                    130, 2, 40, 0, 132, 8, 28, 3, 140, 8, 40, 0, 156, 5, 37, 7, 161, 7, 40, 0, 168,
                    12, 55, 3, 182, 1, 55, 3,
                ],
            ),
            (
                PythonVersion::Py313,
                &[
                    130, 2, 45, 0, 132, 8, 28, 3, 140, 8, 45, 0, 156, 10, 42, 7, 166, 3, 45, 0,
                    170, 3, 45, 0, 173, 13, 61, 3, 188, 1, 61, 3,
                ],
            ),
        ];
        for &(version, expected) in tables {
            let module = compile_with_options(
                source,
                "legacy_with_suppression.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .code;
            let function = module
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) => Some(code),
                    _ => None,
                })
                .unwrap();
            assert_eq!(function.exception_table, expected, "{version:?}");
        }
    }
}
