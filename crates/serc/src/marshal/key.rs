use crate::compiler::{CodeObject, Constant};

use super::scalar::{Scalar, write_scalar};
use super::v5::{FLAG_REF, TYPE_CODE, TYPE_FROZENSET, TYPE_SLICE, TYPE_SMALL_TUPLE, TYPE_TUPLE};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum ObjectKey<'a> {
    // Scalar and immutable-container keys use Python value equality.
    None,
    Bool(bool),
    Ellipsis,
    Int(u64),
    SignedInt(i64),
    BigInt(bool, &'a [u16]),
    Float(u64),
    Complex(u64, u64),
    String(&'a str),
    SurrogateString(&'a [u8]),
    Bytes(&'a [u8]),
    ConstantTuple(Vec<Self>),
    FrozenSet(Vec<Self>),
    StringTuple(Vec<&'a str>),
    // Annotation-scope locals retain identity even when their contents compare equal.
    AnnotationLocals(usize),
    // CPython reuses one canonical empty tuple.
    EmptyTuple,
    Slice(Box<Self>, Box<Self>, Box<Self>),
    // These objects also retain identity. The address is used only as a marshal-graph key and is
    // never written to the output.
    SliceMember(usize),
    Code(usize),
    CodeBytes(usize),
    LocalKinds(usize),
}

pub(super) fn code_key(code: &CodeObject) -> ObjectKey<'_> {
    ObjectKey::Code(std::ptr::from_ref(code).addr())
}

pub(super) fn code_bytes_key(code: &CodeObject) -> ObjectKey<'_> {
    ObjectKey::CodeBytes(std::ptr::from_ref(code).addr())
}

pub(super) fn local_kinds_key(code: &CodeObject) -> ObjectKey<'_> {
    if code.local_kinds.is_empty() {
        ObjectKey::Bytes(&code.local_kinds)
    } else {
        ObjectKey::LocalKinds(std::ptr::from_ref(code).addr())
    }
}

/// Returns whether CPython gives a scalar slice member fresh identity instead of using its cache.
pub(super) fn slice_member_uses_identity(value: &Constant) -> bool {
    match value {
        Constant::Int(value) => *value > 256,
        Constant::SignedInt(value) => !(-5..=256).contains(value),
        Constant::BigInt { .. } | Constant::Float(_) | Constant::Complex { .. } => true,
        Constant::String(value) => !is_cached_character(value),
        Constant::SurrogateString(_) => true,
        Constant::Bytes(value) => value.len() > 1,
        Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Tuple(_)
        | Constant::FrozenSet(_)
        | Constant::Slice { .. }
        | Constant::Code(_) => false,
    }
}

/// Builds the value key used for constants, except for code objects whose identity is significant.
pub(super) fn constant_key(value: &Constant) -> ObjectKey<'_> {
    match value {
        Constant::None => ObjectKey::None,
        Constant::Bool(value) => ObjectKey::Bool(*value),
        Constant::Ellipsis => ObjectKey::Ellipsis,
        Constant::Int(value) => ObjectKey::Int(*value),
        Constant::SignedInt(value) => ObjectKey::SignedInt(*value),
        Constant::BigInt { negative, digits } => ObjectKey::BigInt(*negative, digits),
        Constant::Float(value) => ObjectKey::Float(value.to_bits()),
        Constant::Complex { real, imag } => ObjectKey::Complex(real.to_bits(), imag.to_bits()),
        Constant::String(value) => ObjectKey::String(value),
        Constant::SurrogateString(value) => ObjectKey::SurrogateString(value),
        Constant::Bytes(value) => ObjectKey::Bytes(value),
        Constant::Tuple(values) => constants_tuple_key(values),
        Constant::FrozenSet(values) => {
            let mut values: Vec<_> = values.iter().map(constant_key).collect();
            values.sort_unstable();
            ObjectKey::FrozenSet(values)
        }
        Constant::Slice { lower, upper, step } => ObjectKey::Slice(
            Box::new(constant_key(lower)),
            Box::new(constant_key(upper)),
            Box::new(constant_key(step)),
        ),
        Constant::Code(code) => code_key(code),
    }
}

pub(super) fn constant_sort_key(value: &Constant) -> Vec<u8> {
    fn append(output: &mut Vec<u8>, value: &Constant, force_reference: bool) {
        let flag = if force_reference { FLAG_REF } else { 0 };
        match value {
            Constant::None => write_scalar(output, Scalar::None, flag),
            Constant::Bool(value) => write_scalar(output, Scalar::Bool(*value), flag),
            Constant::Ellipsis => write_scalar(output, Scalar::Ellipsis, flag),
            Constant::Int(value) => write_scalar(output, Scalar::Int(*value), flag),
            Constant::SignedInt(value) => write_scalar(output, Scalar::SignedInt(*value), flag),
            Constant::BigInt { negative, digits } => write_scalar(
                output,
                Scalar::BigInt {
                    negative: *negative,
                    digits,
                },
                flag,
            ),
            Constant::Float(value) => write_scalar(output, Scalar::Float(*value), flag),
            Constant::Complex { real, imag } => write_scalar(
                output,
                Scalar::Complex {
                    real: *real,
                    imag: *imag,
                },
                flag,
            ),
            Constant::String(value) => write_scalar(
                output,
                Scalar::String {
                    value,
                    interned: should_intern(value),
                },
                flag,
            ),
            Constant::SurrogateString(value) => {
                write_scalar(output, Scalar::SurrogateString(value), flag);
            }
            Constant::Bytes(value) => write_scalar(output, Scalar::Bytes(value), flag),
            Constant::Tuple(values) => {
                output.push(
                    (if values.len() < 256 {
                        TYPE_SMALL_TUPLE
                    } else {
                        TYPE_TUPLE
                    }) | flag,
                );
                if values.len() < 256 {
                    output.push(
                        u8::try_from(values.len()).expect("small tuple sort-key length fits in u8"),
                    );
                } else {
                    output.extend_from_slice(
                        &u32::try_from(values.len())
                            .expect("sort-key tuple length fits in u32")
                            .to_le_bytes(),
                    );
                }
                for value in values {
                    append(output, value, true);
                }
            }
            Constant::FrozenSet(values) => {
                output.push(TYPE_FROZENSET | flag);
                output.extend_from_slice(
                    &u32::try_from(values.len())
                        .expect("sort-key frozenset length fits in u32")
                        .to_le_bytes(),
                );
                for (sort_key, _) in frozen_set_members_in_wire_order(values) {
                    output.extend_from_slice(&sort_key);
                }
            }
            Constant::Slice { lower, upper, step } => {
                output.push(TYPE_SLICE | flag);
                append(output, lower, false);
                append(output, upper, false);
                append(output, step, false);
            }
            Constant::Code(_) => output.push(TYPE_CODE | flag),
        }
    }

    let mut output = Vec::new();
    append(&mut output, value, true);
    output
}

pub(super) fn frozen_set_members_in_wire_order(values: &[Constant]) -> Vec<(Vec<u8>, &Constant)> {
    let mut members = values
        .iter()
        .map(|value| (constant_sort_key(value), value))
        .collect::<Vec<_>>();
    members.sort_by(|(left, _), (right, _)| left.cmp(right));
    members
}

pub(super) fn constants_tuple_key(values: &[Constant]) -> ObjectKey<'_> {
    if values.is_empty() {
        ObjectKey::EmptyTuple
    } else if let Some(values) = values
        .iter()
        .map(|value| match value {
            Constant::String(value) => Some(value.as_str()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
    {
        ObjectKey::StringTuple(values)
    } else {
        ObjectKey::ConstantTuple(values.iter().map(constant_key).collect())
    }
}

pub(super) fn strings_tuple_key(values: &[String]) -> ObjectKey<'_> {
    if values.is_empty() {
        ObjectKey::EmptyTuple
    } else {
        ObjectKey::StringTuple(values.iter().map(String::as_str).collect())
    }
}

pub(super) fn locals_tuple_key(code: &CodeObject) -> ObjectKey<'_> {
    if code.locals.is_empty() {
        ObjectKey::EmptyTuple
    } else if code.annotation_thunk {
        ObjectKey::AnnotationLocals(std::ptr::from_ref(code).addr())
    } else {
        strings_tuple_key(&code.locals)
    }
}

pub(super) fn should_intern(value: &str) -> bool {
    is_cached_character(value)
        || value
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn is_cached_character(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| characters.next().is_none() && u32::from(character) <= 0xff)
}
