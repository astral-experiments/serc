use crate::compiler::{CodeObject, Constant};

use super::graph::ObjectGraph;
use super::plan::{ObjectId, PlannedValue};
use super::v5::{
    FLAG_REF, TYPE_ASCII, TYPE_ASCII_INTERNED, TYPE_BINARY_COMPLEX, TYPE_BINARY_FLOAT, TYPE_CODE,
    TYPE_ELLIPSIS, TYPE_FALSE, TYPE_FROZENSET, TYPE_INT, TYPE_INTERNED, TYPE_LONG, TYPE_NONE,
    TYPE_REF, TYPE_SHORT_ASCII, TYPE_SHORT_ASCII_INTERNED, TYPE_SLICE, TYPE_SMALL_TUPLE,
    TYPE_STRING, TYPE_TRUE, TYPE_TUPLE, TYPE_UNICODE,
};

pub(crate) fn encode_code(code: &CodeObject) -> Vec<u8> {
    let mut output = Vec::new();
    encode_code_into(code, &mut output);
    output
}

pub(crate) fn encode_code_into(code: &CodeObject, output: &mut Vec<u8>) {
    let graph = ObjectGraph::for_code(code);
    let mut writer = Writer {
        graph: &graph,
        output,
        references: vec![None; graph.object_count()],
        reference_count: 0,
    };
    writer.write_entry(graph.plan().root());
}

struct Writer<'output, 'graph, 'code> {
    graph: &'graph ObjectGraph<'code>,
    output: &'output mut Vec<u8>,
    references: Vec<Option<u32>>,
    reference_count: usize,
}

impl Writer<'_, '_, '_> {
    fn write_entry(&mut self, index: usize) -> usize {
        let (value, object, force_reference, subtree_end) = {
            let entry = self.graph.plan().entry(index);
            (
                entry.value,
                entry.object,
                entry.force_reference,
                entry.subtree_end,
            )
        };
        self.write_value(value, object, force_reference);
        let mut child = index + 1;
        while child < subtree_end {
            child = self.write_entry(child);
        }
        subtree_end
    }

    fn write_value(
        &mut self,
        value: PlannedValue<'_>,
        object: Option<ObjectId>,
        force_reference: bool,
    ) {
        match value {
            PlannedValue::Code(code) => {
                let object = object.expect("code has an object");
                let Some(flag) = self.begin_object(object, force_reference) else {
                    return;
                };
                self.byte(TYPE_CODE | flag);
                self.long(code.arg_count);
                self.long(code.positional_only_arg_count);
                self.long(code.keyword_only_arg_count);
                self.long(code.stack_size);
                self.long(code.flags);
            }
            PlannedValue::CodeBytes(code) => {
                let object = object.expect("code bytes have an object");
                self.bytes_with_object(object, &code.bytecode, force_reference);
            }
            PlannedValue::Constants(values) => {
                let object = object.expect("constants have an object");
                self.tuple_header(object, values.len(), force_reference);
            }
            PlannedValue::Strings(values) => {
                let object = object.expect("names have an object");
                self.tuple_header(object, values.len(), force_reference);
            }
            PlannedValue::Locals(code) => {
                let object = object.expect("locals have an object");
                self.tuple_header(object, code.locals.len(), force_reference);
            }
            PlannedValue::LocalKinds(code) => {
                let object = object.expect("local kinds have an object");
                self.bytes_with_object(object, &code.local_kinds, force_reference);
            }
            PlannedValue::MetadataString(string) => {
                let object = object.expect("metadata has an object");
                self.string_with_object(object, string, force_reference);
            }
            PlannedValue::FirstLine(first_line_number) => {
                self.long(first_line_number);
            }
            PlannedValue::Bytes(bytes) => {
                let object = object.expect("bytes have an object");
                self.bytes_with_object(object, bytes, force_reference);
            }
            PlannedValue::Constant(constant) => {
                self.write_constant_value(object, constant, force_reference);
            }
            PlannedValue::SliceMember(constant) => {
                let object = object.expect("slice member has an object");
                self.write_slice_member(object, constant);
            }
        }
    }

    fn byte(&mut self, value: u8) {
        self.output.push(value);
    }

    fn long(&mut self, value: u32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn begin_object(&mut self, object: ObjectId, force_reference: bool) -> Option<u8> {
        if !self.graph.is_shared_object(object, force_reference) {
            return Some(0);
        }
        if let Some(index) = self.references[object.0] {
            self.byte(TYPE_REF);
            self.long(index);
            return None;
        }
        let index = u32::try_from(self.reference_count).expect("marshal reference limit exceeded");
        self.reference_count += 1;
        self.references[object.0] = Some(index);
        Some(FLAG_REF)
    }

    fn bytes_with_object(&mut self, object: ObjectId, value: &[u8], force_reference: bool) {
        let Some(flag) = self.begin_object(object, force_reference) else {
            return;
        };
        self.byte(TYPE_STRING | flag);
        self.long(value.len().try_into().expect("marshal value exceeds 4 GiB"));
        self.output.extend_from_slice(value);
    }

    fn string_with_object(&mut self, object: ObjectId, value: &str, force_reference: bool) {
        let interned = self.graph.is_interned_object(object);
        let Some(flag) = self.begin_object(object, force_reference || interned) else {
            return;
        };
        let bytes = value.as_bytes();
        if value.is_ascii() && bytes.len() < 256 {
            self.byte(
                if interned {
                    TYPE_SHORT_ASCII_INTERNED
                } else {
                    TYPE_SHORT_ASCII
                } | flag,
            );
            self.byte(u8::try_from(bytes.len()).expect("short ASCII string length fits in u8"));
        } else {
            self.byte(
                if value.is_ascii() {
                    if interned {
                        TYPE_ASCII_INTERNED
                    } else {
                        TYPE_ASCII
                    }
                } else if interned {
                    TYPE_INTERNED
                } else {
                    TYPE_UNICODE
                } | flag,
            );
            self.long(
                bytes
                    .len()
                    .try_into()
                    .expect("marshal string exceeds 4 GiB"),
            );
        }
        self.output.extend_from_slice(bytes);
    }

    fn surrogate_string_with_object(
        &mut self,
        object: ObjectId,
        value: &[u8],
        force_reference: bool,
    ) {
        let Some(flag) = self.begin_object(object, force_reference) else {
            return;
        };
        self.byte(TYPE_UNICODE | flag);
        self.long(
            value
                .len()
                .try_into()
                .expect("marshal string exceeds 4 GiB"),
        );
        self.output.extend_from_slice(value);
    }

    fn tuple_header(&mut self, object: ObjectId, length: usize, force_reference: bool) {
        let Some(flag) = self.begin_object(object, force_reference) else {
            return;
        };
        if let Ok(length) = u8::try_from(length) {
            self.byte(TYPE_SMALL_TUPLE | flag);
            self.byte(length);
        } else {
            self.byte(TYPE_TUPLE | flag);
            self.long(length.try_into().expect("marshal tuple exceeds 4 GiB"));
        }
    }

    fn write_constant_value(
        &mut self,
        object: Option<ObjectId>,
        value: &Constant,
        force_reference: bool,
    ) {
        match value {
            Constant::None => self.byte(TYPE_NONE),
            Constant::Bool(false) => self.byte(TYPE_FALSE),
            Constant::Bool(true) => self.byte(TYPE_TRUE),
            Constant::Ellipsis => self.byte(TYPE_ELLIPSIS),
            Constant::Int(integer) => {
                let object = object.expect("integer has an object");
                self.integer(object, *integer, force_reference);
            }
            Constant::SignedInt(integer) => {
                let object = object.expect("signed integer has an object");
                self.signed_integer(object, *integer, force_reference);
            }
            Constant::BigInt { negative, digits } => {
                let object = object.expect("big integer has an object");
                self.big_integer(object, digits, *negative, force_reference);
            }
            Constant::Float(float) => {
                let object = object.expect("float has an object");
                let Some(flag) = self.begin_object(object, force_reference) else {
                    return;
                };
                self.byte(TYPE_BINARY_FLOAT | flag);
                self.output.extend_from_slice(&float.to_le_bytes());
            }
            Constant::Complex { real, imag } => {
                let object = object.expect("complex has an object");
                let Some(flag) = self.begin_object(object, force_reference) else {
                    return;
                };
                self.byte(TYPE_BINARY_COMPLEX | flag);
                self.output.extend_from_slice(&real.to_le_bytes());
                self.output.extend_from_slice(&imag.to_le_bytes());
            }
            Constant::String(string) => {
                let object = object.expect("string has an object");
                self.string_with_object(object, string, force_reference);
            }
            Constant::SurrogateString(string) => {
                let object = object.expect("surrogate string has an object");
                self.surrogate_string_with_object(object, string, force_reference);
            }
            Constant::Bytes(bytes) => {
                let object = object.expect("bytes have an object");
                self.bytes_with_object(object, bytes, force_reference);
            }
            Constant::Tuple(_) | Constant::Code(_) => {
                unreachable!("tuple and code constants have dedicated plan entries")
            }
            Constant::FrozenSet(values) => {
                let object = object.expect("frozen set has an object");
                let Some(flag) = self.begin_object(object, force_reference) else {
                    return;
                };
                self.byte(TYPE_FROZENSET | flag);
                self.long(
                    values
                        .len()
                        .try_into()
                        .expect("marshal frozenset exceeds 4 GiB"),
                );
            }
            Constant::Slice { .. } => {
                let object = object.expect("slice has an object");
                let Some(flag) = self.begin_object(object, force_reference) else {
                    return;
                };
                self.byte(TYPE_SLICE | flag);
            }
        }
    }

    fn write_slice_member(&mut self, object: ObjectId, value: &Constant) {
        let Some(flag) = self.begin_object(object, false) else {
            return;
        };
        match value {
            Constant::Int(value) => {
                if let Ok(value) = i32::try_from(*value) {
                    self.byte(TYPE_INT | flag);
                    self.output.extend_from_slice(&value.to_le_bytes());
                } else {
                    self.byte(TYPE_LONG | flag);
                    let digit_count = (64 - value.leading_zeros()).div_ceil(15).max(1);
                    self.long(digit_count);
                    let mut remaining = *value;
                    for _ in 0..digit_count {
                        self.output
                            .extend_from_slice(&((remaining & 0x7fff) as u16).to_le_bytes());
                        remaining >>= 15;
                    }
                }
            }
            Constant::SignedInt(value) => {
                if let Ok(value) = i32::try_from(*value) {
                    self.byte(TYPE_INT | flag);
                    self.output.extend_from_slice(&value.to_le_bytes());
                } else {
                    self.byte(TYPE_LONG | flag);
                    let magnitude = value.unsigned_abs();
                    let digit_count = (64 - magnitude.leading_zeros()).div_ceil(15).max(1);
                    self.long(0_u32.wrapping_sub(digit_count));
                    let mut remaining = magnitude;
                    for _ in 0..digit_count {
                        self.output
                            .extend_from_slice(&((remaining & 0x7fff) as u16).to_le_bytes());
                        remaining >>= 15;
                    }
                }
            }
            Constant::BigInt { negative, digits } => {
                self.byte(TYPE_LONG | flag);
                let digit_count =
                    u32::try_from(digits.len()).expect("marshal integer limit exceeded");
                self.long(if *negative {
                    0_u32.wrapping_sub(digit_count)
                } else {
                    digit_count
                });
                for digit in digits {
                    self.output.extend_from_slice(&digit.to_le_bytes());
                }
            }
            Constant::Float(value) => {
                self.byte(TYPE_BINARY_FLOAT | flag);
                self.output.extend_from_slice(&value.to_le_bytes());
            }
            Constant::Complex { real, imag } => {
                self.byte(TYPE_BINARY_COMPLEX | flag);
                self.output.extend_from_slice(&real.to_le_bytes());
                self.output.extend_from_slice(&imag.to_le_bytes());
            }
            Constant::String(value) => {
                let bytes = value.as_bytes();
                if value.is_ascii() && bytes.len() < 256 {
                    self.byte(TYPE_SHORT_ASCII | flag);
                    self.byte(
                        u8::try_from(bytes.len()).expect("short ASCII string length fits in u8"),
                    );
                } else {
                    self.byte(if value.is_ascii() {
                        TYPE_ASCII | flag
                    } else {
                        TYPE_UNICODE | flag
                    });
                    self.long(
                        bytes
                            .len()
                            .try_into()
                            .expect("marshal string exceeds 4 GiB"),
                    );
                }
                self.output.extend_from_slice(bytes);
            }
            Constant::SurrogateString(value) => {
                self.byte(TYPE_UNICODE | flag);
                self.long(
                    value
                        .len()
                        .try_into()
                        .expect("marshal string exceeds 4 GiB"),
                );
                self.output.extend_from_slice(value);
            }
            Constant::Bytes(value) => {
                self.byte(TYPE_STRING | flag);
                self.long(value.len().try_into().expect("marshal value exceeds 4 GiB"));
                self.output.extend_from_slice(value);
            }
            _ => unreachable!("only non-cached scalar slice members use identity keys"),
        }
    }

    fn integer(&mut self, object: ObjectId, value: u64, force_reference: bool) {
        let Some(flag) = self.begin_object(object, force_reference) else {
            return;
        };
        if let Ok(value) = i32::try_from(value) {
            self.byte(TYPE_INT | flag);
            self.output.extend_from_slice(&value.to_le_bytes());
            return;
        }

        self.byte(TYPE_LONG | flag);
        let digit_count = (64 - value.leading_zeros()).div_ceil(15).max(1);
        self.long(digit_count);
        let mut remaining = value;
        for _ in 0..digit_count {
            self.output
                .extend_from_slice(&((remaining & 0x7fff) as u16).to_le_bytes());
            remaining >>= 15;
        }
    }

    fn signed_integer(&mut self, object: ObjectId, value: i64, force_reference: bool) {
        let Some(flag) = self.begin_object(object, force_reference) else {
            return;
        };
        if let Ok(value) = i32::try_from(value) {
            self.byte(TYPE_INT | flag);
            self.output.extend_from_slice(&value.to_le_bytes());
            return;
        }

        self.byte(TYPE_LONG | flag);
        let magnitude = value.unsigned_abs();
        let digit_count = (64 - magnitude.leading_zeros()).div_ceil(15).max(1);
        self.long(0_u32.wrapping_sub(digit_count));
        let mut remaining = magnitude;
        for _ in 0..digit_count {
            self.output
                .extend_from_slice(&((remaining & 0x7fff) as u16).to_le_bytes());
            remaining >>= 15;
        }
    }

    fn big_integer(
        &mut self,
        object: ObjectId,
        value: &[u16],
        negative: bool,
        force_reference: bool,
    ) {
        let Some(flag) = self.begin_object(object, force_reference) else {
            return;
        };
        self.byte(TYPE_LONG | flag);
        let digit_count = u32::try_from(value.len()).expect("marshal integer limit exceeded");
        self.long(if negative {
            0_u32.wrapping_sub(digit_count)
        } else {
            digit_count
        });
        for digit in value {
            self.output.extend_from_slice(&digit.to_le_bytes());
        }
    }
}
