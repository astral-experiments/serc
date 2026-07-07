use crate::code_object::{CodeObject, Constant};

use super::graph::{ObjectGraph, OccurrenceCursor, OccurrenceKind};
use super::plan::{MarshalVisitor, ObjectId, ReferenceMode, traverse_code};
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
        occurrences: graph.occurrence_cursor(),
    };
    traverse_code(code, &mut writer);
    writer.finish();
}

struct Writer<'output, 'graph, 'code> {
    graph: &'graph ObjectGraph<'code>,
    output: &'output mut Vec<u8>,
    references: Vec<Option<u32>>,
    reference_count: usize,
    occurrences: OccurrenceCursor<'graph>,
}

impl Writer<'_, '_, '_> {
    fn finish(self) {
        self.occurrences.finish();
    }

    fn next_object(&mut self, kind: OccurrenceKind) -> ObjectId {
        self.occurrences.consume(kind)
    }

    fn byte(&mut self, value: u8) {
        self.output.push(value);
    }

    fn long(&mut self, value: u32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn begin_object(&mut self, object: ObjectId, reference_mode: ReferenceMode) -> Option<u8> {
        if reference_mode == ReferenceMode::IfShared && !self.graph.is_shared_object(object) {
            return Some(0);
        }
        if let Some(index) = self.references[object.index()] {
            self.byte(TYPE_REF);
            self.long(index);
            return None;
        }
        let index = u32::try_from(self.reference_count).expect("marshal reference limit exceeded");
        self.reference_count += 1;
        self.references[object.index()] = Some(index);
        Some(FLAG_REF)
    }

    fn bytes_with_object(&mut self, object: ObjectId, value: &[u8], reference_mode: ReferenceMode) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
            return;
        };
        self.byte(TYPE_STRING | flag);
        self.long(value.len().try_into().expect("marshal value exceeds 4 GiB"));
        self.output.extend_from_slice(value);
    }

    fn string_with_object(&mut self, object: ObjectId, value: &str, reference_mode: ReferenceMode) {
        let interned = self.graph.is_interned_object(object);
        let reference_mode = if interned {
            ReferenceMode::Forced
        } else {
            reference_mode
        };
        let Some(flag) = self.begin_object(object, reference_mode) else {
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
        reference_mode: ReferenceMode,
    ) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
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

    fn tuple_header(
        &mut self,
        object: ObjectId,
        length: usize,
        reference_mode: ReferenceMode,
    ) -> bool {
        let Some(flag) = self.begin_object(object, reference_mode) else {
            return false;
        };
        if let Ok(length) = u8::try_from(length) {
            self.byte(TYPE_SMALL_TUPLE | flag);
            self.byte(length);
        } else {
            self.byte(TYPE_TUPLE | flag);
            self.long(length.try_into().expect("marshal tuple exceeds 4 GiB"));
        }
        true
    }

    fn write_constant_value(&mut self, value: &Constant, reference_mode: ReferenceMode) -> bool {
        match value {
            Constant::None => {
                self.byte(TYPE_NONE);
                true
            }
            Constant::Bool(false) => {
                self.byte(TYPE_FALSE);
                true
            }
            Constant::Bool(true) => {
                self.byte(TYPE_TRUE);
                true
            }
            Constant::Ellipsis => {
                self.byte(TYPE_ELLIPSIS);
                true
            }
            Constant::Int(integer) => {
                let object = self.next_object(OccurrenceKind::Constant);
                self.integer(object, *integer, reference_mode);
                true
            }
            Constant::SignedInt(integer) => {
                let object = self.next_object(OccurrenceKind::Constant);
                self.signed_integer(object, *integer, reference_mode);
                true
            }
            Constant::BigInt { negative, digits } => {
                let object = self.next_object(OccurrenceKind::Constant);
                self.big_integer(object, digits, *negative, reference_mode);
                true
            }
            Constant::Float(float) => {
                let object = self.next_object(OccurrenceKind::Constant);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte(TYPE_BINARY_FLOAT | flag);
                self.output.extend_from_slice(&float.to_le_bytes());
                true
            }
            Constant::Complex { real, imag } => {
                let object = self.next_object(OccurrenceKind::Constant);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte(TYPE_BINARY_COMPLEX | flag);
                self.output.extend_from_slice(&real.to_le_bytes());
                self.output.extend_from_slice(&imag.to_le_bytes());
                true
            }
            Constant::String(string) => {
                let object = self.next_object(OccurrenceKind::Constant);
                self.string_with_object(object, string, reference_mode);
                true
            }
            Constant::SurrogateString(string) => {
                let object = self.next_object(OccurrenceKind::Constant);
                self.surrogate_string_with_object(object, string, reference_mode);
                true
            }
            Constant::Bytes(bytes) => {
                let object = self.next_object(OccurrenceKind::Constant);
                self.bytes_with_object(object, bytes, reference_mode);
                true
            }
            Constant::Tuple(_) | Constant::Code(_) => {
                unreachable!("tuple and code constants have dedicated traversal methods")
            }
            Constant::FrozenSet(values) => {
                let object = self.next_object(OccurrenceKind::Constant);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte(TYPE_FROZENSET | flag);
                self.long(
                    values
                        .len()
                        .try_into()
                        .expect("marshal frozenset exceeds 4 GiB"),
                );
                true
            }
            Constant::Slice { .. } => {
                let object = self.next_object(OccurrenceKind::Constant);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte(TYPE_SLICE | flag);
                true
            }
        }
    }

    fn write_slice_member(&mut self, object: ObjectId, value: &Constant) {
        let Some(flag) = self.begin_object(object, ReferenceMode::IfShared) else {
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

    fn integer(&mut self, object: ObjectId, value: u64, reference_mode: ReferenceMode) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
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

    fn signed_integer(&mut self, object: ObjectId, value: i64, reference_mode: ReferenceMode) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
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
        reference_mode: ReferenceMode,
    ) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
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

impl<'code> MarshalVisitor<'code> for Writer<'_, '_, 'code> {
    #[inline]
    fn code(&mut self, code: &'code CodeObject, reference_mode: ReferenceMode) -> bool {
        let object = self.next_object(OccurrenceKind::Code);
        let Some(flag) = self.begin_object(object, reference_mode) else {
            return false;
        };
        self.byte(TYPE_CODE | flag);
        self.long(code.arg_count);
        self.long(code.positional_only_arg_count);
        self.long(code.keyword_only_arg_count);
        self.long(code.stack_size);
        self.long(code.flags);
        true
    }

    #[inline]
    fn code_bytes(&mut self, code: &'code CodeObject) {
        let object = self.next_object(OccurrenceKind::CodeBytes);
        self.bytes_with_object(object, &code.bytecode, ReferenceMode::Forced);
    }

    #[inline]
    fn constants(&mut self, values: &'code [Constant], reference_mode: ReferenceMode) -> bool {
        let object = self.next_object(OccurrenceKind::Constants);
        self.tuple_header(object, values.len(), reference_mode)
    }

    #[inline]
    fn strings(&mut self, values: &'code [String]) -> bool {
        let object = self.next_object(OccurrenceKind::Strings);
        self.tuple_header(object, values.len(), ReferenceMode::IfShared)
    }

    #[inline]
    fn locals(&mut self, code: &'code CodeObject) -> bool {
        let object = self.next_object(OccurrenceKind::Locals);
        self.tuple_header(object, code.locals.len(), ReferenceMode::IfShared)
    }

    #[inline]
    fn local_kinds(&mut self, code: &'code CodeObject) {
        let object = self.next_object(OccurrenceKind::LocalKinds);
        self.bytes_with_object(object, &code.local_kinds, ReferenceMode::IfShared);
    }

    #[inline]
    fn metadata_string(&mut self, value: &'code str) {
        let object = self.next_object(OccurrenceKind::MetadataString);
        self.string_with_object(object, value, ReferenceMode::IfShared);
    }

    #[inline]
    fn first_line(&mut self, value: u32) {
        self.long(value);
    }

    #[inline]
    fn bytes(&mut self, value: &'code [u8]) {
        let object = self.next_object(OccurrenceKind::Bytes);
        self.bytes_with_object(object, value, ReferenceMode::IfShared);
    }

    #[inline]
    fn constant(&mut self, value: &'code Constant, reference_mode: ReferenceMode) -> bool {
        self.write_constant_value(value, reference_mode)
    }

    #[inline]
    fn slice_member(&mut self, value: &'code Constant) {
        let object = self.next_object(OccurrenceKind::SliceMember);
        self.write_slice_member(object, value);
    }
}
