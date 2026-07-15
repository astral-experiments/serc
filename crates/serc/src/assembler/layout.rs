use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::{
    EXTENDED_ARG, Item, LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_LOAD_FAST, LOAD_GLOBAL,
    NOT_TAKEN, Operand, POP_TOP, STORE_FAST, STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST,
    ends_scope, is_conditional_jump,
};
use crate::CompileError;
use crate::assembler::{
    Instruction, Label, LayoutAssembler, NormalizedExceptionOwner, SourceLocation,
};

type FinalArtifacts = (Vec<u8>, Vec<u8>, u32);

impl LayoutAssembler {
    pub(in crate::assembler) fn exception_table(
        &self,
        extended_args: &[u8],
        labels: &[Option<u32>],
    ) -> Result<Vec<u8>, CompileError> {
        if self.exception_regions.is_empty() {
            return Ok(Vec::new());
        }

        let mut regions = self
            .exception_regions
            .iter()
            .enumerate()
            .map(|(index, region)| {
                let start = label_position(labels, region.start).ok_or_else(|| {
                    CompileError::Internal("unbound exception region start".to_string())
                })?;
                let end = label_position(labels, region.end).ok_or_else(|| {
                    CompileError::Internal("unbound exception region end".to_string())
                })?;
                let target = label_position(labels, region.target).ok_or_else(|| {
                    CompileError::Internal("unbound exception handler target".to_string())
                })?;
                Ok((
                    start,
                    end,
                    target,
                    region.depth,
                    region.preserve_lasti,
                    index,
                ))
            })
            .collect::<Result<Vec<_>, CompileError>>()?;
        let mut late_exception_exclusions = Vec::new();
        let mut block_labels = vec![false; labels.len()];
        let mut setup_labels = vec![false; labels.len()];
        for region in &self.exception_regions {
            if let Some(block_label) =
                block_labels.get_mut(usize::try_from(region.target.0).unwrap())
            {
                *block_label = true;
            }
            if let Some(setup_label) =
                setup_labels.get_mut(usize::try_from(region.start.0).unwrap())
            {
                *setup_label = true;
            }
        }
        for item in &self.items {
            match item {
                Item::Label(label) if self.preserved_block_boundaries.contains(label) => {
                    if let Some(block_label) =
                        block_labels.get_mut(usize::try_from(label.0).unwrap())
                    {
                        *block_label = true;
                    }
                }
                Item::Instruction(instruction) => {
                    let (Operand::Forward(label) | Operand::Backward(label)) =
                        instruction.operand()
                    else {
                        continue;
                    };
                    if let Some(block_label) =
                        block_labels.get_mut(usize::try_from(label.0).unwrap())
                    {
                        *block_label = true;
                    }
                }
                Item::Label(_) => {}
            }
        }
        let mut position = 0;
        let mut instruction_index = 0;
        let mut block_has_instruction = false;
        let mut block_has_stale_exception_owner = false;
        let mut block_has_applied_comprehension_swap = false;
        let mut late_exception_inclusions = Vec::new();
        let mut late_exception_overrides = Vec::new();
        let mut previous_instruction = None;
        for item in &self.items {
            match item {
                Item::Label(label) => {
                    let label = usize::try_from(label.0).unwrap();
                    if block_has_instruction && block_labels.get(label).copied().unwrap_or(false) {
                        block_has_instruction = false;
                        block_has_stale_exception_owner = false;
                        block_has_applied_comprehension_swap = false;
                        previous_instruction = None;
                    }
                    if setup_labels.get(label).copied().unwrap_or(false) {
                        block_has_stale_exception_owner = true;
                    }
                }
                Item::Instruction(instruction) => {
                    // CPython's static swap optimization removes the instruction before the
                    // first comprehension restore, leaving that CFG slot's exception owner for
                    // a later normalized marker.
                    let is_comprehension_restore = instruction.opcode == STORE_FAST
                        && instruction.fusion_policy.prevents_with_previous();
                    let applied_comprehension_swap = is_comprehension_restore
                        && previous_instruction.is_some_and(|previous: Instruction| {
                            matches!(previous.opcode, POP_TOP | STORE_FAST)
                                && !(previous.opcode == STORE_FAST
                                    && previous.fusion_policy.prevents_with_previous())
                        });

                    let end = position
                        + u32::from(extended_args[instruction_index])
                        + 1
                        + u32::from(instruction.opcode.caches());
                    if instruction
                        .exception_policy
                        .exclusion()
                        .applies(extended_args[instruction_index])
                    {
                        late_exception_exclusions.push((position, end));
                    }
                    if instruction.exception_policy.owner() == NormalizedExceptionOwner::Absent {
                        if block_has_applied_comprehension_swap {
                            late_exception_inclusions.push((position, end));
                        }
                        if !block_has_stale_exception_owner {
                            late_exception_exclusions.push((position, end));
                        }
                    }
                    if instruction.exception_policy.owner() == NormalizedExceptionOwner::Present {
                        late_exception_inclusions.push((position, end));
                    }
                    if instruction.exception_policy.owner()
                        == NormalizedExceptionOwner::OverridePresent
                    {
                        late_exception_overrides.push((position, end));
                    }
                    if matches!(
                        instruction.opcode,
                        LOAD_FAST_BORROW_LOAD_FAST_BORROW
                            | LOAD_FAST_LOAD_FAST
                            | STORE_FAST_LOAD_FAST
                            | STORE_FAST_STORE_FAST
                    ) || matches!(
                        (instruction.opcode, instruction.operand()),
                        (LOAD_GLOBAL, Operand::Value(argument)) if argument & 1 != 0
                    ) || applied_comprehension_swap
                    {
                        block_has_stale_exception_owner = true;
                    }
                    if applied_comprehension_swap {
                        block_has_applied_comprehension_swap = true;
                    }
                    position = end;
                    instruction_index += 1;
                    block_has_instruction = true;
                    if instruction.opcode == NOT_TAKEN
                        || (!is_conditional_jump(instruction.opcode)
                            && (!matches!(instruction.operand(), Operand::Value(_))
                                || ends_scope(instruction.opcode)))
                    {
                        block_has_instruction = false;
                        block_has_stale_exception_owner = false;
                        block_has_applied_comprehension_swap = false;
                        previous_instruction = None;
                    } else {
                        previous_instruction = Some(*instruction);
                    }
                }
            }
        }
        // Compiler-side region exclusions can leave a normalized marker structurally
        // uncovered. Recreate its requested owner from the innermost owner of the immediately
        // preceding instruction.
        for (start, end) in late_exception_inclusions {
            if regions
                .iter()
                .any(|(region_start, region_end, _, _, _, _)| {
                    *region_start <= start && *region_end >= end
                })
            {
                continue;
            }
            let Some((_, _, target, depth, preserve_lasti, index)) = regions
                .iter()
                .filter(|(region_start, region_end, _, _, _, _)| {
                    *region_start < start && *region_end >= start
                })
                .min_by_key(|(region_start, region_end, _, _, _, index)| {
                    (region_end - region_start, *index)
                })
                .copied()
            else {
                continue;
            };
            regions.push((start, end, target, depth, preserve_lasti, index));
        }
        for (start, end) in late_exception_overrides {
            let Some((_, _, target, depth, preserve_lasti, index)) = regions
                .iter()
                .filter(|(region_start, region_end, _, _, _, _)| {
                    *region_start < start && *region_end >= start
                })
                .min_by_key(|(region_start, region_end, _, _, _, index)| {
                    (region_end - region_start, *index)
                })
                .copied()
            else {
                continue;
            };
            if regions.iter().any(
                |(region_start, region_end, region_target, region_depth, region_lasti, _)| {
                    *region_start <= start
                        && *region_end >= end
                        && *region_target == target
                        && *region_depth == depth
                        && *region_lasti == preserve_lasti
                },
            ) {
                continue;
            }
            regions.push((start, end, target, depth, preserve_lasti, index));
        }
        let mut boundaries = regions
            .iter()
            .flat_map(|(start, end, _, _, _, _)| [*start, *end])
            .chain(
                late_exception_exclusions
                    .iter()
                    .flat_map(|(start, end)| [*start, *end]),
            )
            .collect::<Vec<_>>();
        boundaries.sort_unstable();
        boundaries.dedup();

        let mut region_starts = (0..regions.len()).collect::<Vec<_>>();
        region_starts.sort_by_key(|index| regions[*index].0);
        let mut exclusion_starts = late_exception_exclusions;
        exclusion_starts.sort_unstable_by_key(|(start, _)| *start);
        let mut next_region = 0;
        let mut next_exclusion = 0;
        let mut excluded_through = 0;
        let mut active_regions = BinaryHeap::<Reverse<(u32, usize, usize)>>::new();

        let mut flattened = Vec::<(u32, u32, u32, u32, bool)>::new();
        for boundary in boundaries.windows(2) {
            let start = boundary[0];
            let end = boundary[1];
            if end <= start {
                continue;
            }
            while next_exclusion < exclusion_starts.len()
                && exclusion_starts[next_exclusion].0 <= start
            {
                excluded_through = excluded_through.max(exclusion_starts[next_exclusion].1);
                next_exclusion += 1;
            }
            if excluded_through >= end {
                continue;
            }
            while next_region < region_starts.len()
                && regions[region_starts[next_region]].0 <= start
            {
                let region = region_starts[next_region];
                let (region_start, region_end, _, _, _, original_index) = regions[region];
                active_regions.push(Reverse((region_end - region_start, original_index, region)));
                next_region += 1;
            }
            while active_regions
                .peek()
                .is_some_and(|Reverse((_, _, region))| regions[*region].1 <= start)
            {
                active_regions.pop();
            }
            let Some(Reverse((_, _, region))) = active_regions.peek().copied() else {
                continue;
            };
            let (_, region_end, target, depth, preserve_lasti, _) = regions[region];
            debug_assert!(region_end >= end);
            if let Some((_, previous_end, previous_target, previous_depth, previous_lasti)) =
                flattened.last_mut()
                && *previous_end == start
                && *previous_target == target
                && *previous_depth == depth
                && *previous_lasti == preserve_lasti
            {
                *previous_end = end;
            } else {
                flattened.push((start, end, target, depth, preserve_lasti));
            }
        }

        let mut output = Vec::with_capacity(flattened.len().saturating_mul(8));
        for (start, end, target, depth, preserve_lasti) in flattened {
            write_exception_varint(&mut output, start, true);
            write_exception_varint(&mut output, end - start, false);
            write_exception_varint(&mut output, target, false);
            write_exception_varint(&mut output, (depth << 1) | u32::from(preserve_lasti), false);
        }
        Ok(output)
    }

    pub(in crate::assembler) fn encode_final_artifacts(
        &self,
        extended_args: &[u8],
        resolved_arguments: &[u32],
        code_units: u32,
        first_line_number: u32,
    ) -> (Vec<u8>, Vec<u8>, u32) {
        let mut bytecode = vec![0; usize::try_from(code_units).unwrap().saturating_mul(2)];
        let mut line_table = Vec::with_capacity(extended_args.len().saturating_mul(3));
        let mut byte_offset = 0;
        let mut current_line = i32::try_from(first_line_number).unwrap_or(i32::MAX);
        let mut run = None::<(SourceLocation, u32)>;
        let mut max_depth = 0;
        let mut instruction_index = 0_usize;

        for item in &self.items {
            let Item::Instruction(instruction) = *item else {
                continue;
            };
            let argument = resolved_arguments[instruction_index];
            let extended_arg_count = extended_args[instruction_index];

            for index in (1..=extended_arg_count).rev() {
                bytecode[byte_offset] = EXTENDED_ARG.code();
                bytecode[byte_offset + 1] = ((argument >> (u32::from(index) * 8)) & 0xff) as u8;
                byte_offset += 2;
            }
            bytecode[byte_offset] = instruction.opcode.code();
            bytecode[byte_offset + 1] = (argument & 0xff) as u8;
            byte_offset += 2 + usize::from(instruction.opcode.caches()) * 2;

            let size = u32::from(extended_arg_count) + 1 + u32::from(instruction.opcode.caches());
            if let Some((location, length)) = &mut run
                && *location == instruction.location
            {
                *length += size;
            } else {
                if let Some((location, length)) = run {
                    write_location_run(&mut line_table, location, length, &mut current_line);
                }
                run = Some((instruction.location, size));
            }
            max_depth = max_depth.max(instruction.depth_after().unwrap_or(0));
            instruction_index += 1;
        }

        if let Some((location, length)) = run {
            write_location_run(&mut line_table, location, length, &mut current_line);
        }
        debug_assert_eq!(byte_offset, bytecode.len());

        (bytecode, line_table, max_depth)
    }

    pub(in crate::assembler) fn encode_unextended_final_artifacts(
        &self,
        labels: &[Option<u32>],
        code_units: u32,
        first_line_number: u32,
    ) -> Result<Option<FinalArtifacts>, CompileError> {
        let mut bytecode = vec![0; usize::try_from(code_units).unwrap().saturating_mul(2)];
        let mut line_table = Vec::with_capacity(self.items.len().saturating_mul(3));
        let mut byte_offset = 0;
        let mut position = 0;
        let mut current_line = i32::try_from(first_line_number).unwrap_or(i32::MAX);
        let mut run = None::<(SourceLocation, u32)>;
        let mut max_depth = 0;

        for item in &self.items {
            let Item::Instruction(instruction) = *item else {
                continue;
            };
            let jump_base = position + 1 + u32::from(instruction.opcode.caches());
            let argument = match instruction.operand() {
                Operand::Value(argument) => argument,
                Operand::Deferred(id) => {
                    return Err(CompileError::Internal(format!(
                        "unresolved operand patch {} reached final layout",
                        id.0
                    )));
                }
                Operand::Forward(label) => {
                    let target = label_position(labels, label).ok_or_else(|| {
                        CompileError::Internal("unbound forward jump label".to_string())
                    })?;
                    target.checked_sub(jump_base).ok_or_else(|| {
                        CompileError::Internal("forward jump points backward".to_string())
                    })?
                }
                Operand::Backward(label) => {
                    let target = label_position(labels, label).ok_or_else(|| {
                        CompileError::Internal("unbound backward jump label".to_string())
                    })?;
                    jump_base.checked_sub(target).ok_or_else(|| {
                        CompileError::Internal("backward jump points forward".to_string())
                    })?
                }
            };
            let Ok(argument) = u8::try_from(argument) else {
                return Ok(None);
            };

            bytecode[byte_offset] = instruction.opcode.code();
            bytecode[byte_offset + 1] = argument;
            let size = 1 + u32::from(instruction.opcode.caches());
            byte_offset += usize::try_from(size).unwrap() * 2;
            position = jump_base;

            if let Some((location, length)) = &mut run
                && *location == instruction.location
            {
                *length += size;
            } else {
                if let Some((location, length)) = run {
                    write_location_run(&mut line_table, location, length, &mut current_line);
                }
                run = Some((instruction.location, size));
            }
            max_depth = max_depth.max(instruction.depth_after().unwrap_or(0));
        }

        if let Some((location, length)) = run {
            write_location_run(&mut line_table, location, length, &mut current_line);
        }
        debug_assert_eq!(byte_offset, bytecode.len());

        Ok(Some((bytecode, line_table, max_depth)))
    }

    pub(in crate::assembler) fn label_positions(
        &self,
        extended_args: &[u8],
        labels: &mut [Option<u32>],
    ) -> u32 {
        let mut position = 0_u32;
        let mut instruction_index = 0;

        for item in &self.items {
            match *item {
                Item::Instruction(instruction) => {
                    position += u32::from(extended_args[instruction_index])
                        + 1
                        + u32::from(instruction.opcode.caches());
                    instruction_index += 1;
                }
                Item::Label(label) => {
                    labels[usize::try_from(label.0).unwrap()] = Some(position);
                }
            }
        }

        position
    }
}

pub(super) fn label_position(labels: &[Option<u32>], label: Label) -> Option<u32> {
    labels
        .get(usize::try_from(label.0).unwrap())
        .copied()
        .flatten()
}

fn write_location_run(
    output: &mut Vec<u8>,
    location: SourceLocation,
    mut length: u32,
    current_line: &mut i32,
) {
    while length > 8 {
        write_location(output, location, 8, current_line);
        length -= 8;
    }
    if length > 0 {
        write_location(output, location, length, current_line);
    }
}

fn write_location(
    output: &mut Vec<u8>,
    location: SourceLocation,
    length: u32,
    current_line: &mut i32,
) {
    if location.line < 0 {
        write_location_start(output, 15, length);
        return;
    }
    let line_delta = location.line - *current_line;
    if location.column < 0 || location.end_column < 0 {
        if location.end_line == location.line || location.end_line < 0 {
            write_location_start(output, 13, length);
            write_signed_varint(output, line_delta);
            *current_line = location.line;
            return;
        }
    } else if location.end_line == location.line {
        let column_width = location.end_column - location.column;
        if line_delta == 0 && location.column < 80 && (0..16).contains(&column_width) {
            let column_group = location.column >> 3;
            let column_low_bits = location.column & 7;
            output.extend_from_slice(&[
                location_start(column_group, length),
                u8::try_from((column_low_bits << 4) | column_width)
                    .expect("short-form location columns fit in one byte"),
            ]);
            return;
        }
        if (0..3).contains(&line_delta) && location.column < 128 && location.end_column < 128 {
            output.extend_from_slice(&[
                location_start(10 + line_delta, length),
                u8::try_from(location.column).expect("one-line column fits in one byte"),
                u8::try_from(location.end_column).expect("one-line column fits in one byte"),
            ]);
            *current_line = location.line;
            return;
        }
    }

    write_location_start(output, 14, length);
    write_signed_varint(output, line_delta);
    write_varint(output, (location.end_line - location.line).cast_unsigned());
    write_varint(output, (location.column + 1).cast_unsigned());
    write_varint(output, (location.end_column + 1).cast_unsigned());
    *current_line = location.line;
}

fn write_location_start(output: &mut Vec<u8>, code: i32, length: u32) {
    output.push(location_start(code, length));
}

fn location_start(code: i32, length: u32) -> u8 {
    let code = u8::try_from(code).expect("location code is four bits");
    let length = u8::try_from(length).expect("location run is at most eight code units");
    0x80 | (code << 3) | (length - 1)
}

fn write_signed_varint(output: &mut Vec<u8>, value: i32) {
    let value = if value < 0 {
        ((0_u32.wrapping_sub(value.cast_unsigned())) << 1) | 1
    } else {
        value.cast_unsigned() << 1
    };
    write_varint(output, value);
}

fn write_varint(output: &mut Vec<u8>, mut value: u32) {
    while value >= 64 {
        output.push(64 | (value & 63) as u8);
        value >>= 6;
    }
    output.push(u8::try_from(value).expect("varint tail is six bits"));
}

fn write_exception_varint(output: &mut Vec<u8>, value: u32, mark_entry_start: bool) {
    let mut shift = if value >= 1 << 24 {
        24
    } else if value >= 1 << 18 {
        18
    } else if value >= 1 << 12 {
        12
    } else if value >= 1 << 6 {
        6
    } else {
        0
    };
    let mut first = true;
    loop {
        let mut byte = u8::try_from((value >> shift) & 0x3f).expect("exception varint chunk");
        if shift != 0 {
            byte |= 0x40;
        }
        if first && mark_entry_start {
            byte |= 0x80;
        }
        output.push(byte);
        first = false;
        if shift == 0 {
            break;
        }
        shift -= 6;
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::*;
    use crate::assembler::{ExceptionPolicy, ExceptionRegion, NOP};

    fn instruction(policy: ExceptionPolicy) -> Item {
        let mut instruction = Instruction::new(NOP, Operand::Value(0), SourceLocation::NONE, None);
        instruction.exception_policy = policy;
        Item::Instruction(instruction)
    }

    fn layout(items: Vec<Item>, exception_regions: Vec<ExceptionRegion>) -> LayoutAssembler {
        LayoutAssembler {
            items,
            exception_regions,
            preserved_block_boundaries: FxHashSet::default(),
            removed_max_depth: None,
            next_label: 0,
            has_next_location: false,
        }
    }

    fn region(
        start: u32,
        end: u32,
        target: u32,
        depth: u32,
        preserve_lasti: bool,
    ) -> ExceptionRegion {
        ExceptionRegion {
            start: Label(start),
            end: Label(end),
            target: Label(target),
            depth,
            preserve_lasti,
        }
    }

    fn read_varint(table: &[u8], index: &mut usize) -> u32 {
        let mut byte = table[*index];
        *index += 1;
        let mut value = u32::from(byte & 0x3f);
        while byte & 0x40 != 0 {
            byte = table[*index];
            *index += 1;
            value = (value << 6) | u32::from(byte & 0x3f);
        }
        value
    }

    fn entries(table: &[u8]) -> Vec<(u32, u32, u32, u32, bool)> {
        let mut index = 0;
        let mut entries = Vec::new();
        while index < table.len() {
            assert_ne!(table[index] & 0x80, 0);
            let start = read_varint(table, &mut index);
            let length = read_varint(table, &mut index);
            let target = read_varint(table, &mut index);
            let depth_and_lasti = read_varint(table, &mut index);
            entries.push((
                start,
                start + length,
                target,
                depth_and_lasti >> 1,
                depth_and_lasti & 1 != 0,
            ));
        }
        entries
    }

    #[test]
    fn exception_sweep_prefers_innermost_then_original_order_and_keeps_adjacent_entries() {
        let assembler = layout(
            vec![instruction(ExceptionPolicy::default()); 8],
            vec![
                region(0, 3, 5, 1, false),
                region(1, 2, 6, 2, true),
                region(1, 2, 7, 3, false),
                region(3, 4, 8, 4, true),
            ],
        );
        let labels = [
            Some(0),
            Some(1),
            Some(4),
            Some(6),
            Some(8),
            Some(20),
            Some(21),
            Some(22),
            Some(23),
        ];

        let table = assembler.exception_table(&[0; 8], &labels).unwrap();

        assert_eq!(
            entries(&table),
            [
                (0, 1, 20, 1, false),
                (1, 4, 21, 2, true),
                (4, 6, 20, 1, false),
                (6, 8, 23, 4, true),
            ]
        );
    }

    #[test]
    fn exception_sweep_respects_always_and_extended_exclusions_as_half_open_ranges() {
        let assembler = layout(
            vec![
                instruction(ExceptionPolicy::default()),
                instruction(ExceptionPolicy::default()),
                instruction(ExceptionPolicy::EXCLUDE_ALWAYS),
                instruction(ExceptionPolicy::default()),
                instruction(ExceptionPolicy::EXCLUDE_IF_EXTENDED),
                instruction(ExceptionPolicy::default()),
            ],
            vec![region(0, 3, 4, 1, false), region(1, 2, 5, 2, true)],
        );
        let labels = [Some(0), Some(1), Some(6), Some(7), Some(20), Some(21)];

        let table = assembler
            .exception_table(&[0, 0, 0, 0, 1, 0], &labels)
            .unwrap();

        assert_eq!(
            entries(&table),
            [
                (0, 1, 20, 1, false),
                (1, 2, 21, 2, true),
                (3, 4, 21, 2, true),
                (6, 7, 20, 1, false),
            ]
        );
    }

    #[test]
    fn exception_sweep_keeps_late_inclusion_and_override_repairs_ordered() {
        let included = layout(
            vec![
                instruction(ExceptionPolicy::default()),
                instruction(ExceptionPolicy::default()),
                instruction(ExceptionPolicy::normalized_owner(true)),
            ],
            vec![region(0, 1, 2, 1, true)],
        );
        let included_labels = [Some(0), Some(2), Some(10)];
        let included_table = included.exception_table(&[0; 3], &included_labels).unwrap();
        assert_eq!(entries(&included_table), [(0, 3, 10, 1, true)]);

        let mut override_policy = ExceptionPolicy::default();
        override_policy.set_owner(NormalizedExceptionOwner::OverridePresent);
        let overridden = layout(
            vec![
                instruction(ExceptionPolicy::default()),
                instruction(ExceptionPolicy::default()),
                instruction(override_policy),
                instruction(ExceptionPolicy::default()),
            ],
            vec![region(0, 1, 3, 1, false), region(1, 2, 4, 2, true)],
        );
        let overridden_labels = [Some(0), Some(2), Some(4), Some(10), Some(11)];
        let overridden_table = overridden
            .exception_table(&[0; 4], &overridden_labels)
            .unwrap();

        assert_eq!(
            entries(&overridden_table),
            [(0, 3, 10, 1, false), (3, 4, 11, 2, true)]
        );
    }
}
