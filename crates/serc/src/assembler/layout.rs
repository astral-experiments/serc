use rustc_hash::FxHashSet;

use super::{
    EXTENDED_ARG, Item, LOAD_FAST_BORROW_LOAD_FAST_BORROW, LOAD_FAST_LOAD_FAST, LOAD_GLOBAL,
    NOT_TAKEN, Operand, POP_TOP, STORE_FAST, STORE_FAST_LOAD_FAST, STORE_FAST_STORE_FAST,
    ends_scope, is_conditional_jump,
};
use crate::CompileError;
use crate::assembler::{
    Instruction, Label, LayoutAssembler, NormalizedExceptionOwner, SourceLocation,
};

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
        let mut block_labels = self.preserved_block_boundaries.clone();
        let mut setup_labels = FxHashSet::default();
        for region in &self.exception_regions {
            block_labels.insert(region.target);
            setup_labels.insert(region.start);
        }
        for item in &self.items {
            if let Item::Instruction(Instruction {
                operand: Operand::Forward(label) | Operand::Backward(label),
                ..
            }) = item
            {
                block_labels.insert(*label);
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
                    if block_has_instruction && block_labels.contains(label) {
                        block_has_instruction = false;
                        block_has_stale_exception_owner = false;
                        block_has_applied_comprehension_swap = false;
                        previous_instruction = None;
                    }
                    if setup_labels.contains(label) {
                        block_has_stale_exception_owner = true;
                    }
                }
                Item::Instruction(instruction) => {
                    // CPython's static swap optimization removes the instruction before the
                    // first comprehension restore, leaving that CFG slot's exception owner for
                    // a later normalized marker.
                    let is_comprehension_restore = instruction.opcode == STORE_FAST
                        && instruction.fusion_policy.prevent_with_previous;
                    let applied_comprehension_swap = is_comprehension_restore
                        && previous_instruction.is_some_and(|previous: Instruction| {
                            matches!(previous.opcode, POP_TOP | STORE_FAST)
                                && !(previous.opcode == STORE_FAST
                                    && previous.fusion_policy.prevent_with_previous)
                        });

                    let end = position
                        + u32::from(extended_args[instruction_index])
                        + 1
                        + u32::from(instruction.opcode.caches());
                    if instruction
                        .exception_policy
                        .exclusion
                        .applies(extended_args[instruction_index])
                    {
                        late_exception_exclusions.push((position, end));
                    }
                    if instruction.exception_policy.normalized_owner
                        == NormalizedExceptionOwner::Absent
                    {
                        if block_has_applied_comprehension_swap {
                            late_exception_inclusions.push((position, end));
                        }
                        if !block_has_stale_exception_owner {
                            late_exception_exclusions.push((position, end));
                        }
                    }
                    if instruction.exception_policy.normalized_owner
                        == NormalizedExceptionOwner::Present
                    {
                        late_exception_inclusions.push((position, end));
                    }
                    if instruction.exception_policy.normalized_owner
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
                        (instruction.opcode, instruction.operand),
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
                            && (!matches!(instruction.operand, Operand::Value(_))
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

        let mut flattened = Vec::<(u32, u32, u32, u32, bool)>::new();
        for boundary in boundaries.windows(2) {
            let start = boundary[0];
            let end = boundary[1];
            if end <= start {
                continue;
            }
            if late_exception_exclusions
                .iter()
                .any(|(excluded_start, excluded_end)| {
                    *excluded_start <= start && *excluded_end >= end
                })
            {
                continue;
            }
            let Some((_, _, target, depth, preserve_lasti, _)) = regions
                .iter()
                .filter(|(region_start, region_end, _, _, _, _)| {
                    *region_start <= start && *region_end >= end
                })
                .min_by_key(|(region_start, region_end, _, _, _, index)| {
                    (region_end - region_start, *index)
                })
            else {
                continue;
            };
            if let Some((_, previous_end, previous_target, previous_depth, previous_lasti)) =
                flattened.last_mut()
                && *previous_end == start
                && *previous_target == *target
                && *previous_depth == *depth
                && *previous_lasti == *preserve_lasti
            {
                *previous_end = end;
            } else {
                flattened.push((start, end, *target, *depth, *preserve_lasti));
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

    pub(in crate::assembler) fn line_table(
        &self,
        extended_args: &[u8],
        first_line_number: u32,
    ) -> Vec<u8> {
        let mut output = Vec::with_capacity(extended_args.len().saturating_mul(2));
        let mut current_line = i32::try_from(first_line_number).unwrap_or(i32::MAX);
        let mut run = None::<(SourceLocation, u32)>;
        let mut instruction_index = 0_usize;
        for item in &self.items {
            let Item::Instruction(instruction) = item else {
                continue;
            };
            let size = u32::from(extended_args[instruction_index])
                + 1
                + u32::from(instruction.opcode.caches());
            if let Some((location, length)) = &mut run
                && *location == instruction.location
            {
                *length += size;
            } else {
                if let Some((location, length)) = run {
                    write_location_run(&mut output, location, length, &mut current_line);
                }
                run = Some((instruction.location, size));
            }
            instruction_index += 1;
        }
        if let Some((location, length)) = run {
            write_location_run(&mut output, location, length, &mut current_line);
        }
        output
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

    pub(in crate::assembler) fn encode(
        &self,
        extended_args: &[u8],
        resolved_arguments: &[u32],
        code_units: u32,
    ) -> Vec<u8> {
        let mut output = Vec::with_capacity(usize::try_from(code_units).unwrap().saturating_mul(2));
        let mut instruction_index = 0;

        for item in &self.items {
            let Item::Instruction(instruction) = *item else {
                continue;
            };
            let argument = resolved_arguments[instruction_index];
            let extended_arg_count = extended_args[instruction_index];

            for index in (1..=extended_arg_count).rev() {
                output.push(EXTENDED_ARG.code());
                output.push(((argument >> (u32::from(index) * 8)) & 0xff) as u8);
            }
            output.push(instruction.opcode.code());
            output.push((argument & 0xff) as u8);
            output.resize(
                output.len() + usize::from(instruction.opcode.caches()) * 2,
                0,
            );

            instruction_index += 1;
        }

        output
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
            write_location_start(output, column_group, length);
            output.push(
                u8::try_from((column_low_bits << 4) | column_width)
                    .expect("short-form location columns fit in one byte"),
            );
            return;
        }
        if (0..3).contains(&line_delta) && location.column < 128 && location.end_column < 128 {
            write_location_start(output, 10 + line_delta, length);
            output.push(u8::try_from(location.column).expect("one-line column fits in one byte"));
            output
                .push(u8::try_from(location.end_column).expect("one-line column fits in one byte"));
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
    let code = u8::try_from(code).expect("location code is four bits");
    let length = u8::try_from(length).expect("location run is at most eight code units");
    output.push(0x80 | (code << 3) | (length - 1));
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
