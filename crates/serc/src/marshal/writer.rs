use crate::compiler::{CodeObject, Constant};

use super::graph::ObjectGraph;
use super::plan::{ObjectId, PlannedValue};
use super::scalar::{Scalar, write_scalar};
use super::v5::{
    FLAG_REF, TYPE_CODE, TYPE_FROZENSET, TYPE_REF, TYPE_SLICE, TYPE_SMALL_TUPLE, TYPE_TUPLE,
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
    writer.write_plan();
}

struct Writer<'output, 'graph, 'code> {
    graph: &'graph ObjectGraph<'code>,
    output: &'output mut Vec<u8>,
    references: Vec<Option<u32>>,
    reference_count: usize,
}

impl Writer<'_, '_, '_> {
    fn write_plan(&mut self) {
        let graph = self.graph;
        for entry in graph.plan().entries() {
            self.write_value(entry.value, entry.object, entry.force_reference);
        }
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
        write_scalar(self.output, Scalar::Bytes(value), flag);
    }

    fn string_with_object(&mut self, object: ObjectId, value: &str, force_reference: bool) {
        let interned = self.graph.is_interned_object(object);
        let Some(flag) = self.begin_object(object, force_reference || interned) else {
            return;
        };
        write_scalar(self.output, Scalar::String { value, interned }, flag);
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
        write_scalar(self.output, Scalar::SurrogateString(value), flag);
    }

    fn scalar_with_object(&mut self, object: ObjectId, value: Scalar<'_>, force_reference: bool) {
        let Some(flag) = self.begin_object(object, force_reference) else {
            return;
        };
        write_scalar(self.output, value, flag);
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
            Constant::None => write_scalar(self.output, Scalar::None, 0),
            Constant::Bool(value) => write_scalar(self.output, Scalar::Bool(*value), 0),
            Constant::Ellipsis => write_scalar(self.output, Scalar::Ellipsis, 0),
            Constant::Int(integer) => {
                let object = object.expect("integer has an object");
                self.scalar_with_object(object, Scalar::Int(*integer), force_reference);
            }
            Constant::SignedInt(integer) => {
                let object = object.expect("signed integer has an object");
                self.scalar_with_object(object, Scalar::SignedInt(*integer), force_reference);
            }
            Constant::BigInt { negative, digits } => {
                let object = object.expect("big integer has an object");
                self.scalar_with_object(
                    object,
                    Scalar::BigInt {
                        negative: *negative,
                        digits,
                    },
                    force_reference,
                );
            }
            Constant::Float(float) => {
                let object = object.expect("float has an object");
                self.scalar_with_object(object, Scalar::Float(*float), force_reference);
            }
            Constant::Complex { real, imag } => {
                let object = object.expect("complex has an object");
                self.scalar_with_object(
                    object,
                    Scalar::Complex {
                        real: *real,
                        imag: *imag,
                    },
                    force_reference,
                );
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
        let value = match value {
            Constant::Int(value) => Scalar::Int(*value),
            Constant::SignedInt(value) => Scalar::SignedInt(*value),
            Constant::BigInt { negative, digits } => Scalar::BigInt {
                negative: *negative,
                digits,
            },
            Constant::Float(value) => Scalar::Float(*value),
            Constant::Complex { real, imag } => Scalar::Complex {
                real: *real,
                imag: *imag,
            },
            Constant::String(value) => Scalar::String {
                value,
                interned: false,
            },
            Constant::SurrogateString(value) => Scalar::SurrogateString(value),
            Constant::Bytes(value) => Scalar::Bytes(value),
            _ => unreachable!("only non-cached scalar slice members use identity keys"),
        };
        write_scalar(self.output, value, flag);
    }
}
