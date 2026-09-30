use crate::code_object::{CodeFilename, CodeObject, Constant, MetadataId, MetadataName};
use crate::{InitialInternState, PythonVersion};

use super::graph::{ObjectGraph, ObjectReference, Occurrence, OccurrenceCursor, OccurrenceKind};
use super::key::FrozenSetMembers;
use super::plan::{MarshalVisitor, ObjectId, ReferenceMode, traverse_code_with_mode};
use super::v5::{
    FLAG_REF, TYPE_ASCII, TYPE_ASCII_INTERNED, TYPE_BINARY_COMPLEX, TYPE_BINARY_FLOAT, TYPE_CODE,
    TYPE_ELLIPSIS, TYPE_FALSE, TYPE_FROZENSET, TYPE_INT, TYPE_INTERNED, TYPE_LONG, TYPE_NONE,
    TYPE_REF, TYPE_SHORT_ASCII, TYPE_SHORT_ASCII_INTERNED, TYPE_SLICE, TYPE_SMALL_TUPLE,
    TYPE_STRING, TYPE_TRUE, TYPE_TUPLE, TYPE_UNICODE,
};

// Marshal v5 adds slices and v6 adds frozendicts. The shared tags retain their wire
// representation across all supported targets. Verify the generated release tables
// so target regeneration cannot silently change the encoder's assumptions.
const _: () = {
    assert!(super::v4::MARSHAL_VERSION == 4);
    assert!(super::v5::MARSHAL_VERSION == 5);
    assert!(super::v6::MARSHAL_VERSION == 6);
    assert!(super::v4::FLAG_REF == super::v5::FLAG_REF);
    assert!(super::v4::TYPE_ASCII == super::v5::TYPE_ASCII);
    assert!(super::v4::TYPE_ASCII_INTERNED == super::v5::TYPE_ASCII_INTERNED);
    assert!(super::v4::TYPE_BINARY_COMPLEX == super::v5::TYPE_BINARY_COMPLEX);
    assert!(super::v4::TYPE_BINARY_FLOAT == super::v5::TYPE_BINARY_FLOAT);
    assert!(super::v4::TYPE_CODE == super::v5::TYPE_CODE);
    assert!(super::v4::TYPE_ELLIPSIS == super::v5::TYPE_ELLIPSIS);
    assert!(super::v4::TYPE_FALSE == super::v5::TYPE_FALSE);
    assert!(super::v4::TYPE_FROZENSET == super::v5::TYPE_FROZENSET);
    assert!(super::v4::TYPE_INT == super::v5::TYPE_INT);
    assert!(super::v4::TYPE_INTERNED == super::v5::TYPE_INTERNED);
    assert!(super::v4::TYPE_LONG == super::v5::TYPE_LONG);
    assert!(super::v4::TYPE_NONE == super::v5::TYPE_NONE);
    assert!(super::v4::TYPE_REF == super::v5::TYPE_REF);
    assert!(super::v4::TYPE_SHORT_ASCII == super::v5::TYPE_SHORT_ASCII);
    assert!(super::v4::TYPE_SHORT_ASCII_INTERNED == super::v5::TYPE_SHORT_ASCII_INTERNED);
    assert!(super::v4::TYPE_SMALL_TUPLE == super::v5::TYPE_SMALL_TUPLE);
    assert!(super::v4::TYPE_STRING == super::v5::TYPE_STRING);
    assert!(super::v4::TYPE_TRUE == super::v5::TYPE_TRUE);
    assert!(super::v4::TYPE_TUPLE == super::v5::TYPE_TUPLE);
    assert!(super::v4::TYPE_UNICODE == super::v5::TYPE_UNICODE);
    assert!(super::v5::FLAG_REF == super::v6::FLAG_REF);
    assert!(super::v5::TYPE_ASCII == super::v6::TYPE_ASCII);
    assert!(super::v5::TYPE_ASCII_INTERNED == super::v6::TYPE_ASCII_INTERNED);
    assert!(super::v5::TYPE_BINARY_COMPLEX == super::v6::TYPE_BINARY_COMPLEX);
    assert!(super::v5::TYPE_BINARY_FLOAT == super::v6::TYPE_BINARY_FLOAT);
    assert!(super::v5::TYPE_CODE == super::v6::TYPE_CODE);
    assert!(super::v5::TYPE_ELLIPSIS == super::v6::TYPE_ELLIPSIS);
    assert!(super::v5::TYPE_FALSE == super::v6::TYPE_FALSE);
    assert!(super::v5::TYPE_FROZENSET == super::v6::TYPE_FROZENSET);
    assert!(super::v5::TYPE_INT == super::v6::TYPE_INT);
    assert!(super::v5::TYPE_INTERNED == super::v6::TYPE_INTERNED);
    assert!(super::v5::TYPE_LONG == super::v6::TYPE_LONG);
    assert!(super::v5::TYPE_NONE == super::v6::TYPE_NONE);
    assert!(super::v5::TYPE_REF == super::v6::TYPE_REF);
    assert!(super::v5::TYPE_SHORT_ASCII == super::v6::TYPE_SHORT_ASCII);
    assert!(super::v5::TYPE_SHORT_ASCII_INTERNED == super::v6::TYPE_SHORT_ASCII_INTERNED);
    assert!(super::v5::TYPE_SLICE == super::v6::TYPE_SLICE);
    assert!(super::v5::TYPE_SMALL_TUPLE == super::v6::TYPE_SMALL_TUPLE);
    assert!(super::v5::TYPE_STRING == super::v6::TYPE_STRING);
    assert!(super::v5::TYPE_TRUE == super::v6::TYPE_TRUE);
    assert!(super::v5::TYPE_TUPLE == super::v6::TYPE_TUPLE);
    assert!(super::v5::TYPE_UNICODE == super::v6::TYPE_UNICODE);
};

#[cfg(test)]
pub(crate) fn encode_code(code: &CodeObject) -> Vec<u8> {
    encode_code_with_options(code, PythonVersion::Py314, &InitialInternState::default())
}

pub(crate) fn encode_code_with_options(
    code: &CodeObject,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
) -> Vec<u8> {
    encode_code_with_mode(
        code,
        python_version,
        initial_intern_state,
        &[],
        ReferenceMode::Forced,
    )
}

pub(crate) fn encode_pyc_code_with_options(
    code: &CodeObject,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
    header: &[u8; 16],
) -> Vec<u8> {
    // CPython 3.15's borrowed local loads leave py_compile's root code object
    // uniquely owned at marshal.dumps. A retained-code marshal call still has
    // an external owner, which encode_code_with_options models separately.
    let root_reference = match python_version {
        PythonVersion::Py312 | PythonVersion::Py313 | PythonVersion::Py314 => ReferenceMode::Forced,
        PythonVersion::Py315 => ReferenceMode::IfShared,
    };
    encode_code_with_mode(
        code,
        python_version,
        initial_intern_state,
        header,
        root_reference,
    )
}

fn encode_code_with_mode(
    code: &CodeObject,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
    prefix: &[u8],
    root_reference: ReferenceMode,
) -> Vec<u8> {
    let mut graph = ObjectGraph::for_code_with_options(code, python_version, initial_intern_state);
    // The root bytecode is only a small part of many modules. Use the complete graph estimate for
    // both bare marshal and header-prefixed .pyc output so the writer starts with one allocation.
    let mut output = Vec::with_capacity(prefix.len().saturating_add(graph.estimated_wire_size()));
    output.extend_from_slice(prefix);
    let occurrences = graph.take_occurrences();
    let mut writer = Writer {
        graph: &mut graph,
        output: &mut output,
        reference_count: 0,
        occurrences,
        frozen_set_order_index: 0,
    };
    traverse_code_with_mode(code, root_reference, &mut writer);
    writer.finish();
    output
}

struct Writer<'output, 'graph, 'code> {
    graph: &'graph mut ObjectGraph<'code>,
    output: &'output mut Vec<u8>,
    reference_count: usize,
    occurrences: OccurrenceCursor<Vec<Occurrence>>,
    frozen_set_order_index: usize,
}

impl Writer<'_, '_, '_> {
    fn finish(self) {
        self.occurrences.finish();
        assert_eq!(
            self.frozen_set_order_index,
            self.graph.frozen_set_order_count(),
            "marshal writer did not consume every frozen set recorded by the graph"
        );
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

    fn byte_and_long(&mut self, byte: u8, value: u32) {
        let [a, b, c, d] = value.to_le_bytes();
        self.output.extend_from_slice(&[byte, a, b, c, d]);
    }

    fn byte_and_float(&mut self, byte: u8, value: f64) {
        let mut bytes = [0; 9];
        bytes[0] = byte;
        bytes[1..].copy_from_slice(&value.to_le_bytes());
        self.output.extend_from_slice(&bytes);
    }

    fn byte_and_complex(&mut self, byte: u8, real: f64, imag: f64) {
        let mut bytes = [0; 17];
        bytes[0] = byte;
        bytes[1..9].copy_from_slice(&real.to_le_bytes());
        bytes[9..].copy_from_slice(&imag.to_le_bytes());
        self.output.extend_from_slice(&bytes);
    }

    fn code_header(&mut self, flag: u8, code: &CodeObject) {
        let mut header = [0; 21];
        header[0] = TYPE_CODE | flag;
        header[1..5].copy_from_slice(&code.arg_count.to_le_bytes());
        header[5..9].copy_from_slice(&code.positional_only_arg_count.to_le_bytes());
        header[9..13].copy_from_slice(&code.keyword_only_arg_count.to_le_bytes());
        header[13..17].copy_from_slice(&code.stack_size.to_le_bytes());
        header[17..21].copy_from_slice(&code.flags.to_le_bytes());
        self.output.extend_from_slice(&header);
    }

    #[expect(
        clippy::inline_always,
        reason = "specialize reference checks at the forced structural-object call sites"
    )]
    #[inline(always)]
    fn begin_object(&mut self, object: ObjectId, reference_mode: ReferenceMode) -> Option<u8> {
        let reference = self
            .graph
            .object_for_writing(object)
            .reference_for_write(reference_mode, &mut self.reference_count);
        self.write_reference(reference)
    }

    #[inline]
    fn write_reference(&mut self, reference: ObjectReference) -> Option<u8> {
        match reference {
            ObjectReference::Unshared => Some(0),
            ObjectReference::New => Some(FLAG_REF),
            ObjectReference::Existing(index) => {
                self.byte_and_long(TYPE_REF, index.as_u32());
                None
            }
        }
    }

    fn bytes_with_object(&mut self, object: ObjectId, value: &[u8], reference_mode: ReferenceMode) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
            return;
        };
        self.byte_and_long(
            TYPE_STRING | flag,
            value.len().try_into().expect("marshal value exceeds 4 GiB"),
        );
        self.output.extend_from_slice(value);
    }

    fn string_with_object(&mut self, object: ObjectId, value: &str, reference_mode: ReferenceMode) {
        let object = self.graph.object_for_writing(object);
        let interned = object.is_interned();
        let reference_mode = if interned {
            ReferenceMode::Forced
        } else {
            reference_mode
        };
        let reference = object.reference_for_write(reference_mode, &mut self.reference_count);
        let Some(flag) = self.write_reference(reference) else {
            return;
        };
        if interned {
            self.string_payload::<true>(value, flag);
        } else {
            self.string_payload::<false>(value, flag);
        }
    }

    #[expect(
        clippy::inline_always,
        reason = "keep metadata reference resolution in the traversal loop"
    )]
    #[inline(always)]
    fn interned_string_with_object(&mut self, object: ObjectId, value: &str) {
        self.interned_string_with_value(object, || value);
    }

    #[expect(
        clippy::inline_always,
        reason = "decode metadata names only when emitting their payload"
    )]
    #[inline(always)]
    fn interned_string_with_value<'a>(
        &mut self,
        object: ObjectId,
        value: impl FnOnce() -> &'a str,
    ) {
        let object = self.graph.object_for_writing(object);
        debug_assert!(object.is_interned(), "marshal metadata must be interned");
        let reference =
            object.reference_for_write(ReferenceMode::Forced, &mut self.reference_count);
        match reference {
            ObjectReference::Existing(index) => self.byte_and_long(TYPE_REF, index.as_u32()),
            ObjectReference::New => self.string_payload::<true>(value(), FLAG_REF),
            ObjectReference::Unshared => {
                unreachable!("forced marshal reference cannot be unshared")
            }
        }
    }

    #[inline]
    fn string_payload<const INTERNED: bool>(&mut self, value: &str, flag: u8) {
        let bytes = value.as_bytes();
        let ascii = value.is_ascii();
        if ascii && bytes.len() < 256 {
            self.output.extend_from_slice(&[
                if INTERNED {
                    TYPE_SHORT_ASCII_INTERNED
                } else {
                    TYPE_SHORT_ASCII
                } | flag,
                u8::try_from(bytes.len()).expect("short ASCII string length fits in u8"),
            ]);
        } else {
            self.byte_and_long(
                if ascii {
                    if INTERNED {
                        TYPE_ASCII_INTERNED
                    } else {
                        TYPE_ASCII
                    }
                } else if INTERNED {
                    TYPE_INTERNED
                } else {
                    TYPE_UNICODE
                } | flag,
                bytes
                    .len()
                    .try_into()
                    .expect("marshal string exceeds 4 GiB"),
            );
        }
        self.output.extend_from_slice(bytes);
    }

    fn code_metadata_string(&mut self, value: &str) {
        let object = self.next_object(OccurrenceKind::MetadataString);
        if self.graph.python_version() == PythonVersion::Py312 {
            // Qualified names and generic-parameter names can retain uninterned identities.
            self.string_with_object(object, value, ReferenceMode::IfShared);
        } else {
            self.interned_string_with_object(object, value);
        }
    }

    fn surrogate_string_with_object(
        &mut self,
        object: ObjectId,
        value: &[u8],
        reference_mode: ReferenceMode,
    ) {
        let object = self.graph.object_for_writing(object);
        let interned = object.is_interned();
        let reference_mode = if interned {
            ReferenceMode::Forced
        } else {
            reference_mode
        };
        let reference = object.reference_for_write(reference_mode, &mut self.reference_count);
        let Some(flag) = self.write_reference(reference) else {
            return;
        };
        self.byte_and_long(
            (if interned {
                TYPE_INTERNED
            } else {
                TYPE_UNICODE
            }) | flag,
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
            self.output
                .extend_from_slice(&[TYPE_SMALL_TUPLE | flag, length]);
        } else {
            self.byte_and_long(
                TYPE_TUPLE | flag,
                length.try_into().expect("marshal tuple exceeds 4 GiB"),
            );
        }
        true
    }

    fn write_constant_value(
        &mut self,
        value: &Constant,
        reference_mode: ReferenceMode,
        kind: OccurrenceKind,
    ) -> bool {
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
                let object = self.next_object(kind);
                self.integer(object, *integer, reference_mode);
                true
            }
            Constant::SignedInt(integer) => {
                let object = self.next_object(kind);
                self.signed_integer(object, *integer, reference_mode);
                true
            }
            Constant::BigInt { negative, digits } => {
                let object = self.next_object(kind);
                self.big_integer(object, digits, *negative, reference_mode);
                true
            }
            Constant::NonReflexive(number) => {
                self.write_constant_value(&number.value(), reference_mode, kind)
            }
            Constant::Float(float) => {
                let object = self.next_object(kind);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte_and_float(TYPE_BINARY_FLOAT | flag, *float);
                true
            }
            Constant::Complex { real, imag } => {
                let object = self.next_object(kind);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte_and_complex(TYPE_BINARY_COMPLEX | flag, *real, *imag);
                true
            }
            Constant::String(string) => {
                let object = self.next_object(kind);
                self.string_with_object(object, string, reference_mode);
                true
            }
            Constant::SurrogateString(string) => {
                let object = self.next_object(kind);
                self.surrogate_string_with_object(object, string, reference_mode);
                true
            }
            Constant::Bytes(bytes) => {
                let object = self.next_object(kind);
                self.bytes_with_object(object, bytes, reference_mode);
                true
            }
            Constant::Tuple(_) | Constant::Code(_) => {
                unreachable!("tuple and code constants have dedicated traversal methods")
            }
            Constant::FrozenSet(values) => {
                let object = self.next_object(kind);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte_and_long(
                    TYPE_FROZENSET | flag,
                    values
                        .len()
                        .try_into()
                        .expect("marshal frozenset exceeds 4 GiB"),
                );
                true
            }
            Constant::Slice { .. } => {
                assert!(
                    self.graph.python_version() >= PythonVersion::Py314,
                    "slice constants require marshal v5 or later"
                );
                let object = self.next_object(kind);
                let Some(flag) = self.begin_object(object, reference_mode) else {
                    return false;
                };
                self.byte(TYPE_SLICE | flag);
                true
            }
        }
    }

    fn integer(&mut self, object: ObjectId, value: u64, reference_mode: ReferenceMode) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
            return;
        };
        if let Ok(value) = i32::try_from(value) {
            self.byte_and_long(TYPE_INT | flag, value.cast_unsigned());
            return;
        }

        self.long_integer(flag, value, false);
    }

    fn signed_integer(&mut self, object: ObjectId, value: i64, reference_mode: ReferenceMode) {
        let Some(flag) = self.begin_object(object, reference_mode) else {
            return;
        };
        if let Ok(value) = i32::try_from(value) {
            self.byte_and_long(TYPE_INT | flag, value.cast_unsigned());
            return;
        }

        self.long_integer(flag, value.unsigned_abs(), true);
    }

    /// Writes base-2^15 digits least significant first, with the sign encoded in the digit count.
    /// `flag` comes from the caller's completed reference registration.
    fn long_integer(&mut self, flag: u8, magnitude: u64, negative: bool) {
        let digit_count = (64 - magnitude.leading_zeros()).div_ceil(15).max(1);
        self.byte_and_long(
            TYPE_LONG | flag,
            if negative {
                0_u32.wrapping_sub(digit_count)
            } else {
                digit_count
            },
        );
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
        let digit_count = u32::try_from(value.len()).expect("marshal integer limit exceeded");
        self.byte_and_long(
            TYPE_LONG | flag,
            if negative {
                0_u32.wrapping_sub(digit_count)
            } else {
                digit_count
            },
        );
        for digit in value {
            self.output.extend_from_slice(&digit.to_le_bytes());
        }
    }
}

impl<'code> MarshalVisitor<'code> for Writer<'_, '_, 'code> {
    const USES_METADATA_IDS: bool = false;

    fn python_version(&self) -> PythonVersion {
        self.graph.python_version()
    }

    fn frozen_set_members(&mut self, values: &'code [Constant]) -> FrozenSetMembers<'code> {
        if values.len() <= 1 {
            return values.iter().collect();
        }

        let index = self.frozen_set_order_index;
        self.frozen_set_order_index += 1;
        self.graph.frozen_set_members(values, index)
    }

    #[inline]
    fn code(&mut self, code: &'code CodeObject, reference_mode: ReferenceMode) -> bool {
        let object = self.next_object(OccurrenceKind::Code);
        let Some(flag) = self.begin_object(object, reference_mode) else {
            return false;
        };
        self.code_header(flag, code);
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
    fn names(&mut self, code: &'code CodeObject) -> bool {
        let object = self.next_object(OccurrenceKind::Strings);
        self.tuple_header(object, code.names.len(), ReferenceMode::IfShared)
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
    fn metadata_string(&mut self, value: &'code str, _metadata_id: Option<MetadataId>) {
        let object = self.next_object(OccurrenceKind::MetadataString);
        self.interned_string_with_object(object, value);
    }

    #[inline]
    fn metadata_name(&mut self, value: &'code MetadataName, _metadata_id: Option<MetadataId>) {
        let object = self.next_object(OccurrenceKind::MetadataString);
        self.interned_string_with_value(object, || value.as_str());
    }

    fn code_filename(&mut self, code: &'code CodeObject, _metadata_id: Option<MetadataId>) {
        let legacy = self.graph.python_version() == PythonVersion::Py312;
        let object = self.next_object(OccurrenceKind::MetadataString);
        match &code.filename {
            CodeFilename::Utf8(filename) => {
                if legacy {
                    // The compile caller retains its filename alongside the code object.
                    self.string_with_object(object, filename, ReferenceMode::Forced);
                } else {
                    self.interned_string_with_object(object, filename);
                }
            }
            CodeFilename::Surrogate(filename) => {
                let reference_mode = if legacy {
                    ReferenceMode::Forced
                } else {
                    ReferenceMode::IfShared
                };
                self.surrogate_string_with_object(object, filename, reference_mode);
            }
        }
    }

    #[inline]
    fn code_name(&mut self, code: &'code CodeObject, _metadata_id: Option<MetadataId>) {
        self.code_metadata_string(&code.name);
    }

    #[inline]
    fn code_qualified_name(&mut self, code: &'code CodeObject, _metadata_id: Option<MetadataId>) {
        self.code_metadata_string(&code.qualified_name);
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
        self.write_constant_value(value, reference_mode, OccurrenceKind::Constant)
    }

    #[inline]
    fn slice_member(&mut self, value: &'code Constant) {
        // The graph selects scalar identity, including the cached empty-bytes singleton.
        match value {
            Constant::Int(_)
            | Constant::SignedInt(_)
            | Constant::BigInt { .. }
            | Constant::NonReflexive(_)
            | Constant::Float(_)
            | Constant::Complex { .. }
            | Constant::String(_)
            | Constant::SurrogateString(_)
            | Constant::Bytes(_) => {
                self.write_constant_value(
                    value,
                    ReferenceMode::IfShared,
                    OccurrenceKind::SliceMember,
                );
            }
            _ => unreachable!("only scalar slice members use dedicated traversal"),
        }
    }
}
