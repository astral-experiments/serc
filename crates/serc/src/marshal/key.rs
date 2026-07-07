use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use crate::code_object::{CodeObject, Constant, LocalsIdentity};

use super::v5::{
    FLAG_REF, TYPE_ASCII, TYPE_ASCII_INTERNED, TYPE_BINARY_COMPLEX, TYPE_BINARY_FLOAT, TYPE_CODE,
    TYPE_ELLIPSIS, TYPE_FALSE, TYPE_FROZENSET, TYPE_INT, TYPE_INTERNED, TYPE_LONG, TYPE_NONE,
    TYPE_SHORT_ASCII, TYPE_SHORT_ASCII_INTERNED, TYPE_SLICE, TYPE_SMALL_TUPLE, TYPE_STRING,
    TYPE_TRUE, TYPE_TUPLE, TYPE_UNICODE,
};

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
    ConstantTuple(ConstantTupleKey<'a>),
    FrozenSet(Vec<Self>),
    StringTuple(StringTupleKey<'a>),
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

#[derive(Clone, Copy, Debug)]
pub(super) struct ConstantTupleKey<'a>(&'a [Constant]);

impl PartialEq for ConstantTupleKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0)
                .all(|(left, right)| constants_equal(left, right))
    }
}

impl Eq for ConstantTupleKey<'_> {}

impl Hash for ConstantTupleKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for value in self.0 {
            hash_constant(value, state);
        }
    }
}

impl PartialOrd for ConstantTupleKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ConstantTupleKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .iter()
            .map(constant_key)
            .cmp(other.0.iter().map(constant_key))
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct StringTupleKey<'a>(StringTupleKind<'a>);

#[derive(Clone, Copy, Debug)]
enum StringTupleKind<'a> {
    Constants(&'a [Constant]),
    Strings(&'a [String]),
}

impl<'a> StringTupleKey<'a> {
    fn constants(values: &'a [Constant]) -> Self {
        debug_assert!(
            values
                .iter()
                .all(|value| matches!(value, Constant::String(_)))
        );
        Self(StringTupleKind::Constants(values))
    }

    fn strings(values: &'a [String]) -> Self {
        Self(StringTupleKind::Strings(values))
    }
}

impl PartialEq for StringTupleKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self.0, other.0) {
            (StringTupleKind::Constants(left), StringTupleKind::Constants(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| constant_string(left) == constant_string(right))
            }
            (StringTupleKind::Constants(left), StringTupleKind::Strings(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| constant_string(left) == right)
            }
            (StringTupleKind::Strings(left), StringTupleKind::Constants(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| left == constant_string(right))
            }
            (StringTupleKind::Strings(left), StringTupleKind::Strings(right)) => left == right,
        }
    }
}

impl Eq for StringTupleKey<'_> {}

impl Hash for StringTupleKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.0 {
            StringTupleKind::Constants(values) => {
                values.len().hash(state);
                for value in values {
                    constant_string(value).hash(state);
                }
            }
            StringTupleKind::Strings(values) => values.hash(state),
        }
    }
}

impl PartialOrd for StringTupleKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StringTupleKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.0, other.0) {
            (StringTupleKind::Constants(left), StringTupleKind::Constants(right)) => left
                .iter()
                .map(constant_string)
                .cmp(right.iter().map(constant_string)),
            (StringTupleKind::Constants(left), StringTupleKind::Strings(right)) => left
                .iter()
                .map(constant_string)
                .cmp(right.iter().map(String::as_str)),
            (StringTupleKind::Strings(left), StringTupleKind::Constants(right)) => left
                .iter()
                .map(String::as_str)
                .cmp(right.iter().map(constant_string)),
            (StringTupleKind::Strings(left), StringTupleKind::Strings(right)) => left.cmp(right),
        }
    }
}

fn constant_string(value: &Constant) -> &str {
    let Constant::String(value) = value else {
        unreachable!("string tuple contains a non-string constant");
    };
    value
}

fn constants_equal(left: &Constant, right: &Constant) -> bool {
    match (left, right) {
        (Constant::None, Constant::None) | (Constant::Ellipsis, Constant::Ellipsis) => true,
        (Constant::Bool(left), Constant::Bool(right)) => left == right,
        (Constant::Int(left), Constant::Int(right)) => left == right,
        (Constant::SignedInt(left), Constant::SignedInt(right)) => left == right,
        (
            Constant::BigInt {
                negative: left_negative,
                digits: left_digits,
            },
            Constant::BigInt {
                negative: right_negative,
                digits: right_digits,
            },
        ) => left_negative == right_negative && left_digits == right_digits,
        (Constant::Float(left), Constant::Float(right)) => left.to_bits() == right.to_bits(),
        (
            Constant::Complex {
                real: left_real,
                imag: left_imag,
            },
            Constant::Complex {
                real: right_real,
                imag: right_imag,
            },
        ) => {
            left_real.to_bits() == right_real.to_bits()
                && left_imag.to_bits() == right_imag.to_bits()
        }
        (Constant::String(left), Constant::String(right)) => left == right,
        (Constant::SurrogateString(left), Constant::SurrogateString(right)) => left == right,
        (Constant::Bytes(left), Constant::Bytes(right)) => left == right,
        (Constant::Tuple(left), Constant::Tuple(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| constants_equal(left, right))
        }
        (Constant::FrozenSet(_), Constant::FrozenSet(_)) => {
            constant_key(left) == constant_key(right)
        }
        (
            Constant::Slice {
                lower: left_lower,
                upper: left_upper,
                step: left_step,
            },
            Constant::Slice {
                lower: right_lower,
                upper: right_upper,
                step: right_step,
            },
        ) => {
            constants_equal(left_lower, right_lower)
                && constants_equal(left_upper, right_upper)
                && constants_equal(left_step, right_step)
        }
        (Constant::Code(left), Constant::Code(right)) => {
            std::ptr::eq(left.as_ref(), right.as_ref())
        }
        (_, _) => false,
    }
}

fn hash_constant<H: Hasher>(value: &Constant, state: &mut H) {
    match value {
        Constant::None => 0_u8.hash(state),
        Constant::Bool(value) => {
            1_u8.hash(state);
            value.hash(state);
        }
        Constant::Ellipsis => 2_u8.hash(state),
        Constant::Int(value) => {
            3_u8.hash(state);
            value.hash(state);
        }
        Constant::SignedInt(value) => {
            4_u8.hash(state);
            value.hash(state);
        }
        Constant::BigInt { negative, digits } => {
            5_u8.hash(state);
            negative.hash(state);
            digits.hash(state);
        }
        Constant::Float(value) => {
            6_u8.hash(state);
            value.to_bits().hash(state);
        }
        Constant::Complex { real, imag } => {
            7_u8.hash(state);
            real.to_bits().hash(state);
            imag.to_bits().hash(state);
        }
        Constant::String(value) => {
            8_u8.hash(state);
            value.hash(state);
        }
        Constant::SurrogateString(value) => {
            9_u8.hash(state);
            value.hash(state);
        }
        Constant::Bytes(value) => {
            10_u8.hash(state);
            value.hash(state);
        }
        Constant::Tuple(values) => {
            11_u8.hash(state);
            values.len().hash(state);
            for value in values {
                hash_constant(value, state);
            }
        }
        Constant::FrozenSet(_) => {
            12_u8.hash(state);
            constant_key(value).hash(state);
        }
        Constant::Slice { lower, upper, step } => {
            13_u8.hash(state);
            hash_constant(lower, state);
            hash_constant(upper, state);
            hash_constant(step, state);
        }
        Constant::Code(code) => {
            14_u8.hash(state);
            std::ptr::from_ref(code.as_ref()).addr().hash(state);
        }
    }
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
            Constant::None => output.push(TYPE_NONE),
            Constant::Bool(false) => output.push(TYPE_FALSE),
            Constant::Bool(true) => output.push(TYPE_TRUE),
            Constant::Ellipsis => output.push(TYPE_ELLIPSIS),
            Constant::Int(value) => {
                if let Ok(value) = i32::try_from(*value) {
                    output.push(TYPE_INT | flag);
                    output.extend_from_slice(&value.to_le_bytes());
                } else {
                    output.push(TYPE_LONG | flag);
                    let digit_count = (64 - value.leading_zeros()).div_ceil(15).max(1);
                    output.extend_from_slice(&digit_count.to_le_bytes());
                    let mut remaining = *value;
                    for _ in 0..digit_count {
                        output.extend_from_slice(&((remaining & 0x7fff) as u16).to_le_bytes());
                        remaining >>= 15;
                    }
                }
            }
            Constant::SignedInt(value) => {
                if let Ok(value) = i32::try_from(*value) {
                    output.push(TYPE_INT | flag);
                    output.extend_from_slice(&value.to_le_bytes());
                } else {
                    output.push(TYPE_LONG | flag);
                    let magnitude = value.unsigned_abs();
                    let digit_count = (64 - magnitude.leading_zeros()).div_ceil(15).max(1);
                    output.extend_from_slice(&0_u32.wrapping_sub(digit_count).to_le_bytes());
                    let mut remaining = magnitude;
                    for _ in 0..digit_count {
                        output.extend_from_slice(&((remaining & 0x7fff) as u16).to_le_bytes());
                        remaining >>= 15;
                    }
                }
            }
            Constant::BigInt { negative, digits } => {
                output.push(TYPE_LONG | flag);
                let digit_count =
                    u32::try_from(digits.len()).expect("sort-key integer length fits in u32");
                output.extend_from_slice(
                    &if *negative {
                        0_u32.wrapping_sub(digit_count)
                    } else {
                        digit_count
                    }
                    .to_le_bytes(),
                );
                for digit in digits {
                    output.extend_from_slice(&digit.to_le_bytes());
                }
            }
            Constant::Float(value) => {
                output.push(TYPE_BINARY_FLOAT | flag);
                output.extend_from_slice(&value.to_le_bytes());
            }
            Constant::Complex { real, imag } => {
                output.push(TYPE_BINARY_COMPLEX | flag);
                output.extend_from_slice(&real.to_le_bytes());
                output.extend_from_slice(&imag.to_le_bytes());
            }
            Constant::String(value) => {
                let bytes = value.as_bytes();
                let interned = should_intern(value);
                if value.is_ascii() && bytes.len() < 256 {
                    output.push(
                        (if interned {
                            TYPE_SHORT_ASCII_INTERNED
                        } else {
                            TYPE_SHORT_ASCII
                        }) | flag,
                    );
                    output.push(
                        u8::try_from(bytes.len()).expect("short ASCII sort-key length fits in u8"),
                    );
                } else {
                    output.push(
                        (if value.is_ascii() {
                            if interned {
                                TYPE_ASCII_INTERNED
                            } else {
                                TYPE_ASCII
                            }
                        } else if interned {
                            TYPE_INTERNED
                        } else {
                            TYPE_UNICODE
                        }) | flag,
                    );
                    output.extend_from_slice(
                        &u32::try_from(bytes.len())
                            .expect("sort-key string length fits in u32")
                            .to_le_bytes(),
                    );
                }
                output.extend_from_slice(bytes);
            }
            Constant::SurrogateString(value) => {
                output.push(TYPE_UNICODE | flag);
                output.extend_from_slice(
                    &u32::try_from(value.len())
                        .expect("sort-key string length fits in u32")
                        .to_le_bytes(),
                );
                output.extend_from_slice(value);
            }
            Constant::Bytes(value) => {
                output.push(TYPE_STRING | flag);
                output.extend_from_slice(
                    &u32::try_from(value.len())
                        .expect("sort-key bytes length fits in u32")
                        .to_le_bytes(),
                );
                output.extend_from_slice(value);
            }
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
    } else if values
        .iter()
        .all(|value| matches!(value, Constant::String(_)))
    {
        ObjectKey::StringTuple(StringTupleKey::constants(values))
    } else {
        ObjectKey::ConstantTuple(ConstantTupleKey(values))
    }
}

pub(super) fn strings_tuple_key(values: &[String]) -> ObjectKey<'_> {
    if values.is_empty() {
        ObjectKey::EmptyTuple
    } else {
        ObjectKey::StringTuple(StringTupleKey::strings(values))
    }
}

pub(super) fn locals_tuple_key(code: &CodeObject) -> ObjectKey<'_> {
    if code.locals.is_empty() {
        ObjectKey::EmptyTuple
    } else {
        match code.marshal_hints.locals_identity {
            LocalsIdentity::ByValue => strings_tuple_key(&code.locals),
            LocalsIdentity::Fresh => ObjectKey::AnnotationLocals(std::ptr::from_ref(code).addr()),
        }
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
