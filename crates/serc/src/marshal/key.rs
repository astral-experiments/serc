use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::slice;

use rustc_hash::{FxHashMap, FxHasher};
use smallvec::SmallVec;

use crate::PythonVersion;
use crate::code_object::{
    CodeObject, Constant, LocalsIdentity, MarshalMetadataTupleProperties,
    MarshalStringTupleProperties, MetadataName as Name, NonReflexiveKey,
};
use crate::target::small_int_bounds_for;

use super::v5::{
    FLAG_REF, TYPE_ASCII, TYPE_ASCII_INTERNED, TYPE_BINARY_COMPLEX, TYPE_BINARY_FLOAT, TYPE_CODE,
    TYPE_ELLIPSIS, TYPE_FALSE, TYPE_FROZENSET, TYPE_INT, TYPE_INTERNED, TYPE_LONG, TYPE_NONE,
    TYPE_REF, TYPE_SHORT_ASCII, TYPE_SHORT_ASCII_INTERNED, TYPE_SLICE, TYPE_SMALL_TUPLE,
    TYPE_STRING, TYPE_TRUE, TYPE_TUPLE, TYPE_UNICODE,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum ObjectKey<'a> {
    // Scalar and immutable-container keys use Python value equality.
    None,
    Bool(bool),
    Ellipsis,
    Int(u64),
    SignedInt(i64),
    BigInt(bool, &'a [u16]),
    NonReflexive(NonReflexiveKey),
    Float(u64),
    Complex(u64, u64),
    // A noninterned constant does not share identity with an equal metadata string. CPython's
    // compiler constant cache still shares equal constants with each other.
    ConstantString(&'a str),
    String(&'a str),
    // Python 3.12 retains the caller's filename object without interning it.
    Filename(&'a str),
    // Lone surrogates cannot occur in Python identifiers or share an interned source constant.
    SurrogateFilename(&'a [u8]),
    // Dynamically allocated code names stay distinct even when their values match.
    UninternedMetadata(usize),
    SurrogateString(&'a [u8]),
    Bytes(&'a [u8]),
    ConstantTuple(ConstantTupleKey<'a>),
    FrozenSet(FrozenSetKey<'a>),
    StringTuple(StringTupleKey<'a>),
    // Annotation-scope locals retain identity even when their contents compare equal.
    AnnotationLocals(usize),
    // CPython reuses one canonical empty tuple.
    EmptyTuple,
    Slice(SliceKey<'a>),
    // These objects also retain identity. The address is used only as a marshal-graph key and is
    // never written to the output.
    SliceMember(usize),
    RecreatedFrozenSet(usize),
    Code(usize),
    CodeBytes(usize),
    LocalKinds(usize),
    // A zero repetition can seed the constant cache with an empty object distinct from b"".
    FreshEmptyBytes,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct HashedObjectKey<'a> {
    key: ObjectKey<'a>,
    hash: u64,
}

impl<'a> HashedObjectKey<'a> {
    fn new(key: ObjectKey<'a>) -> Self {
        let mut hasher = FxHasher::default();
        key.hash(&mut hasher);
        Self {
            key,
            hash: hasher.finish(),
        }
    }

    pub(super) fn key(&self) -> &ObjectKey<'a> {
        &self.key
    }

    pub(super) fn with_hash(key: ObjectKey<'a>, hash: u64) -> Self {
        debug_assert_eq!(Self::new(key).hash, hash, "marshal key hash");
        Self { key, hash }
    }
}

impl<'a> From<ObjectKey<'a>> for HashedObjectKey<'a> {
    fn from(key: ObjectKey<'a>) -> Self {
        Self::new(key)
    }
}

impl PartialEq for HashedObjectKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.key == other.key
    }
}

impl Eq for HashedObjectKey<'_> {}

impl Hash for HashedObjectKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
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
pub(super) struct FrozenSetKey<'a>(&'a [Constant]);

impl<'a> FrozenSetKey<'a> {
    fn has_nested_sets(self) -> bool {
        fn contains_set(value: &Constant) -> bool {
            match value {
                Constant::FrozenSet(_) => true,
                Constant::Tuple(values) => values.iter().any(contains_set),
                Constant::Slice { lower, upper, step } => {
                    contains_set(lower) || contains_set(upper) || contains_set(step)
                }
                _ => false,
            }
        }
        self.0.iter().any(contains_set)
    }

    fn sorted_members(self) -> SmallVec<[ObjectKey<'a>; 4]> {
        let mut members: SmallVec<_> = self.0.iter().map(constant_key).collect();
        members.sort_unstable();
        members
    }

    #[inline(never)]
    fn canonical_members(self) -> Vec<CanonicalKey<'a>> {
        let mut members: Vec<_> = self.0.iter().map(CanonicalKey::new).collect();
        members.sort_unstable();
        members
    }
}

impl PartialEq for FrozenSetKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        if std::ptr::eq(self.0, other.0) {
            true
        } else if self.0.len() != other.0.len() {
            false
        } else if self.has_nested_sets() || other.has_nested_sets() {
            self.canonical_members() == other.canonical_members()
        } else {
            self.sorted_members() == other.sorted_members()
        }
    }
}

impl Eq for FrozenSetKey<'_> {}

impl Hash for FrozenSetKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.has_nested_sets() {
            self.canonical_members().hash(state);
        } else {
            self.sorted_members().as_slice().hash(state);
        }
    }
}

impl PartialOrd for FrozenSetKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FrozenSetKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        if std::ptr::eq(self.0, other.0) {
            Ordering::Equal
        } else if self.has_nested_sets() || other.has_nested_sets() {
            self.canonical_members().cmp(&other.canonical_members())
        } else {
            self.sorted_members().cmp(&other.sorted_members())
        }
    }
}

/// Prepares descendants once before comparing nested set members. Retaining this tree for the
/// operation prevents each parent comparison from recursively sorting the same descendants.
struct CanonicalKey<'a> {
    value: &'a Constant,
    key: ObjectKey<'a>,
    children: CanonicalChildren<'a>,
}

enum CanonicalChildren<'a> {
    None,
    Tuple(Vec<CanonicalKey<'a>>),
    FrozenSet(Vec<CanonicalKey<'a>>),
    Slice(Box<[CanonicalKey<'a>; 3]>),
}

impl<'a> CanonicalKey<'a> {
    fn new(value: &'a Constant) -> Self {
        let key = constant_key(value);
        let children = match value {
            Constant::Tuple(values) if matches!(key, ObjectKey::ConstantTuple(_)) => {
                CanonicalChildren::Tuple(values.iter().map(Self::new).collect())
            }
            Constant::FrozenSet(values) => {
                let mut members: Vec<_> = values.iter().map(Self::new).collect();
                members.sort_unstable();
                CanonicalChildren::FrozenSet(members)
            }
            Constant::Slice { lower, upper, step } => CanonicalChildren::Slice(Box::new([
                Self::new(lower),
                Self::new(upper),
                Self::new(step),
            ])),
            _ => CanonicalChildren::None,
        };
        Self {
            value,
            key,
            children,
        }
    }

    // ConstantTupleKey has a distinct hash stream from ObjectKey's derived enum hash.
    fn hash_constant<H: Hasher>(&self, state: &mut H) {
        match &self.children {
            CanonicalChildren::None => hash_constant(self.value, state),
            CanonicalChildren::Tuple(members) => {
                11_u8.hash(state);
                members.len().hash(state);
                for member in members {
                    member.hash_constant(state);
                }
            }
            CanonicalChildren::FrozenSet(_) => {
                12_u8.hash(state);
                self.hash(state);
            }
            CanonicalChildren::Slice(members) => {
                13_u8.hash(state);
                for member in members.iter() {
                    member.hash_constant(state);
                }
            }
        }
    }
}

impl PartialEq for CanonicalKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for CanonicalKey<'_> {}

impl Hash for CanonicalKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if matches!(self.children, CanonicalChildren::None) {
            self.key.hash(state);
            return;
        }
        std::mem::discriminant(&self.key).hash(state);
        match &self.children {
            CanonicalChildren::None => unreachable!(),
            CanonicalChildren::Tuple(members) => {
                members.len().hash(state);
                for member in members {
                    member.hash_constant(state);
                }
            }
            CanonicalChildren::FrozenSet(members) => members.hash(state),
            CanonicalChildren::Slice(members) => {
                for member in members.iter() {
                    member.hash(state);
                }
            }
        }
    }
}

impl PartialOrd for CanonicalKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CanonicalKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.children, &other.children) {
            (CanonicalChildren::Tuple(left), CanonicalChildren::Tuple(right))
            | (CanonicalChildren::FrozenSet(left), CanonicalChildren::FrozenSet(right)) => {
                left.cmp(right)
            }
            (CanonicalChildren::Slice(left), CanonicalChildren::Slice(right)) => left.cmp(right),
            // Different prepared kinds have different ObjectKey variants. Leaf keys include
            // empty and all-string tuples, preserving their existing metadata alias domains.
            _ => self.key.cmp(&other.key),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SliceKey<'a> {
    lower: &'a Constant,
    upper: &'a Constant,
    step: &'a Constant,
}

impl<'a> SliceKey<'a> {
    fn members(self) -> [ObjectKey<'a>; 3] {
        [
            constant_key(self.lower),
            constant_key(self.upper),
            constant_key(self.step),
        ]
    }
}

impl PartialEq for SliceKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.members() == other.members()
    }
}

impl Eq for SliceKey<'_> {}

impl Hash for SliceKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for member in self.members() {
            member.hash(state);
        }
    }
}

impl PartialOrd for SliceKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SliceKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.members().cmp(&other.members())
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct StringTupleKey<'a>(StringTupleKind<'a>);

#[derive(Clone, Copy, Debug)]
enum StringTupleKind<'a> {
    Constants(&'a [Constant]),
    Strings(&'a [String]),
    Names(&'a [Name]),
}

enum StringTupleIter<'a> {
    Constants(slice::Iter<'a, Constant>),
    Strings(slice::Iter<'a, String>),
    Names(slice::Iter<'a, Name>),
}

impl<'a> Iterator for StringTupleIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Constants(values) => values.next().map(constant_string),
            Self::Strings(values) => values.next().map(String::as_str),
            Self::Names(values) => values.next().map(Name::as_str),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Constants(values) => values.size_hint(),
            Self::Strings(values) => values.size_hint(),
            Self::Names(values) => values.size_hint(),
        }
    }
}

impl ExactSizeIterator for StringTupleIter<'_> {}

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

    fn names(values: &'a [Name]) -> Self {
        Self(StringTupleKind::Names(values))
    }

    fn iter(&self) -> impl ExactSizeIterator<Item = &'a str> {
        match self.0 {
            StringTupleKind::Constants(values) => StringTupleIter::Constants(values.iter()),
            StringTupleKind::Strings(values) => StringTupleIter::Strings(values.iter()),
            StringTupleKind::Names(values) => StringTupleIter::Names(values.iter()),
        }
    }
}

impl PartialEq for StringTupleKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl Eq for StringTupleKey<'_> {}

impl Hash for StringTupleKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let length = match self.0 {
            StringTupleKind::Constants(values) => values.len(),
            StringTupleKind::Strings(values) => values.len(),
            StringTupleKind::Names(values) => values.len(),
        };
        length.hash(state);
        for value in self.iter() {
            value.hash(state);
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
        self.iter().cmp(other.iter())
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
        (Constant::NonReflexive(left), Constant::NonReflexive(right)) => left.key() == right.key(),
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
            ConstantTupleKey(left) == ConstantTupleKey(right)
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
        Constant::NonReflexive(number) => {
            15_u8.hash(state);
            number.key().hash(state);
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
            ConstantTupleKey(values).hash(state);
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
#[cfg(test)]
pub(super) fn slice_member_uses_identity(value: &Constant) -> bool {
    slice_member_uses_identity_for(value, PythonVersion::Py314)
}

pub(super) fn slice_member_uses_identity_for(
    value: &Constant,
    python_version: PythonVersion,
) -> bool {
    let (minimum, maximum) = small_int_bounds_for(python_version);
    match value {
        Constant::Int(value) => *value > maximum.cast_unsigned(),
        Constant::SignedInt(value) => !(minimum..=maximum).contains(value),
        Constant::BigInt { .. }
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::NonReflexive(_) => true,
        Constant::String(value) => !value.is_empty() && !is_cached_character(value),
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
        Constant::NonReflexive(number) => ObjectKey::NonReflexive(number.key()),
        Constant::Float(value) => ObjectKey::Float(value.to_bits()),
        Constant::Complex { real, imag } => ObjectKey::Complex(real.to_bits(), imag.to_bits()),
        Constant::String(value) => ObjectKey::String(value),
        Constant::SurrogateString(value) => ObjectKey::SurrogateString(value),
        Constant::Bytes(value) => ObjectKey::Bytes(value),
        Constant::Tuple(values) => constants_tuple_key(values),
        Constant::FrozenSet(values) => ObjectKey::FrozenSet(FrozenSetKey(values)),
        Constant::Slice { lower, upper, step } => ObjectKey::Slice(SliceKey { lower, upper, step }),
        Constant::Code(code) => code_key(code),
    }
}

pub(super) type ConstantSortKey = SmallVec<[u8; 16]>;
pub(super) type FrozenSetMembers<'a> = SmallVec<[&'a Constant; 4]>;

pub(super) fn constant_sort_key(value: &Constant) -> ConstantSortKey {
    constant_sort_key_with_properties(value, PythonVersion::Py314, &|value| {
        (
            true,
            matches!(value, Constant::String(value) if should_intern(value)),
        )
    })
}

/// Encodes an independent marshal stream, including references local to the sort key.
fn constant_sort_key_with_properties<'a>(
    value: &'a Constant,
    python_version: PythonVersion,
    properties: &impl Fn(&'a Constant) -> (bool, bool),
) -> ConstantSortKey {
    fn append<'a>(
        output: &mut ConstantSortKey,
        value: &'a Constant,
        force_reference: bool,
        python_version: PythonVersion,
        properties: &impl Fn(&'a Constant) -> (bool, bool),
        references: &mut FxHashMap<ObjectKey<'a>, u32>,
    ) {
        let (shared, interned) = properties(value);
        let flag = if !matches!(
            value,
            Constant::None | Constant::Bool(_) | Constant::Ellipsis
        ) && (force_reference || shared || interned)
        {
            // A scalar root cannot contain a reference, so keep the common scalar-set path free
            // of reference-table allocations.
            if !output.is_empty()
                || matches!(
                    value,
                    Constant::Tuple(_) | Constant::FrozenSet(_) | Constant::Slice { .. }
                )
            {
                let key = constant_key(value);
                if let Some(index) = references.get(&key) {
                    output.push(TYPE_REF);
                    output.extend_from_slice(&index.to_le_bytes());
                    return;
                }
                let index = u32::try_from(references.len()).expect("sort-key reference count fits");
                references.insert(key, index);
            }
            FLAG_REF
        } else {
            0
        };
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
            Constant::NonReflexive(number) => match number.value() {
                Constant::Float(value) => {
                    output.push(TYPE_BINARY_FLOAT | flag);
                    output.extend_from_slice(&value.to_le_bytes());
                }
                Constant::Complex { real, imag } => {
                    output.push(TYPE_BINARY_COMPLEX | flag);
                    output.extend_from_slice(&real.to_le_bytes());
                    output.extend_from_slice(&imag.to_le_bytes());
                }
                _ => unreachable!("non-reflexive values are NaN-bearing numbers"),
            },
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
                    append(output, value, false, python_version, properties, references);
                }
            }
            Constant::FrozenSet(values) => {
                output.push(TYPE_FROZENSET | flag);
                output.extend_from_slice(
                    &u32::try_from(values.len())
                        .expect("sort-key frozenset length fits in u32")
                        .to_le_bytes(),
                );
                for (_, value) in
                    sorted_frozen_set_members_with_properties(values, python_version, properties)
                {
                    append(output, value, true, python_version, properties, references);
                }
            }
            Constant::Slice { lower, upper, step } => {
                output.push(TYPE_SLICE | flag);
                append(output, lower, false, python_version, properties, references);
                append(output, upper, false, python_version, properties, references);
                append(output, step, false, python_version, properties, references);
            }
            Constant::Code(_) => output.push(TYPE_CODE | flag),
        }
    }

    let mut output = ConstantSortKey::new();
    append(
        &mut output,
        value,
        // Python 3.12 borrows each member while computing its independent sort key.
        // Later targets retain a reference through _PySet_NextEntryRef.
        python_version >= PythonVersion::Py313,
        python_version,
        properties,
        &mut FxHashMap::default(),
    );
    output
}

pub(super) fn frozen_set_members_in_wire_order(values: &[Constant]) -> FrozenSetMembers<'_> {
    let mut members = SmallVec::<[(ConstantSortKey, &Constant); 4]>::new();
    for value in values {
        members.push((constant_sort_key(value), value));
    }
    members.sort_by(|(left, _), (right, _)| left.cmp(right));
    members.into_iter().map(|(_, value)| value).collect()
}

pub(super) fn frozen_set_members_with_properties<'a>(
    values: &'a [Constant],
    python_version: PythonVersion,
    properties: &impl Fn(&'a Constant) -> (bool, bool),
) -> FrozenSetMembers<'a> {
    sorted_frozen_set_members_with_properties(values, python_version, properties)
        .into_iter()
        .map(|(_, value)| value)
        .collect()
}

fn sorted_frozen_set_members_with_properties<'a>(
    values: &'a [Constant],
    python_version: PythonVersion,
    properties: &impl Fn(&'a Constant) -> (bool, bool),
) -> SmallVec<[(ConstantSortKey, &'a Constant); 4]> {
    let mut members = SmallVec::<[(ConstantSortKey, &Constant); 4]>::new();
    for value in values {
        members.push((
            constant_sort_key_with_properties(value, python_version, properties),
            value,
        ));
    }
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

pub(super) fn names_tuple_key(values: &[Name]) -> ObjectKey<'_> {
    if values.is_empty() {
        ObjectKey::EmptyTuple
    } else {
        ObjectKey::StringTuple(StringTupleKey::names(values))
    }
}

pub(super) fn locals_tuple_key(code: &CodeObject) -> ObjectKey<'_> {
    if code.locals.is_empty() {
        ObjectKey::EmptyTuple
    } else {
        match code.marshal_provenance.locals_identity {
            LocalsIdentity::ByValue => names_tuple_key(&code.locals),
            LocalsIdentity::Fresh => ObjectKey::AnnotationLocals(std::ptr::from_ref(code).addr()),
        }
    }
}

/// Precomputes the value-only metadata work shared by all marshal graph traversals.
pub(crate) fn metadata_tuple_properties(
    names: &[Name],
    locals: &[Name],
) -> MarshalMetadataTupleProperties {
    fn strings(values: &[Name]) -> MarshalStringTupleProperties {
        let key = names_tuple_key(values);
        if values.is_empty() {
            return MarshalStringTupleProperties {
                hash: HashedObjectKey::from(key).hash,
                automatically_interned: true,
            };
        }
        // Match the derived ObjectKey::StringTuple hash and StringTupleKey's value hash while
        // classifying the same strings. The borrowed key remains the equality check on lookup.
        let mut hasher = FxHasher::default();
        std::mem::discriminant(&key).hash(&mut hasher);
        values.len().hash(&mut hasher);
        let mut automatically_interned = true;
        for value in values {
            value.as_str().hash(&mut hasher);
            automatically_interned = automatically_interned && should_intern(value);
        }
        let hash = hasher.finish();
        debug_assert_eq!(HashedObjectKey::from(key).hash, hash, "metadata tuple hash");
        MarshalStringTupleProperties {
            hash,
            automatically_interned,
        }
    }

    MarshalMetadataTupleProperties {
        names: strings(names),
        locals: strings(locals),
    }
}

pub(super) fn should_intern(value: &str) -> bool {
    if is_cached_character(value) {
        return true;
    }

    let (chunks, remainder) = value.as_bytes().as_chunks::<8>();
    for &chunk in chunks {
        let word = u64::from_ne_bytes(chunk);
        if !is_ascii_identifier_word(word) {
            return false;
        }
    }
    remainder
        .iter()
        .all(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
}

/// Tests the eight ASCII identifier bytes independently without branches between characters.
#[inline]
fn is_ascii_identifier_word(word: u64) -> bool {
    const HIGH: u64 = u64::from_ne_bytes([0x80; 8]);
    const LOW: u64 = !HIGH;
    const CASE: u64 = u64::from_ne_bytes([0x20; 8]);
    const UNDERSCORE: u64 = u64::from_ne_bytes([b'_'; 8]);

    let in_range = |bytes: u64, lower: u8, upper: u8| {
        // Setting each minuend's high bit prevents borrows from crossing byte boundaries.
        let at_least = (bytes | HIGH).wrapping_sub(u64::from_ne_bytes([lower; 8]));
        let at_most = (u64::from_ne_bytes([upper; 8]) | HIGH).wrapping_sub(bytes & LOW);
        at_least & at_most
    };
    let letters = in_range(word | CASE, b'a', b'z');
    let digits = in_range(word, b'0', b'9');
    let underscores = !((word ^ UNDERSCORE) & LOW).wrapping_add(LOW);
    (letters | digits | underscores) & !word & HIGH == HIGH
}

pub(super) fn is_cached_character(value: &str) -> bool {
    matches!(value.as_bytes(), [0x00..=0x7f] | [0xc2..=0xc3, 0x80..=0xbf])
}

#[cfg(test)]
mod intern_tests {
    use super::{is_ascii_identifier_word, is_cached_character, should_intern};

    #[test]
    fn classifies_every_byte_at_every_word_position() {
        for position in 0..8 {
            for byte in u8::MIN..=u8::MAX {
                let mut word = *b"Az_09bY7";
                word[position] = byte;
                assert_eq!(
                    is_ascii_identifier_word(u64::from_ne_bytes(word)),
                    word.iter()
                        .all(|byte| *byte == b'_' || byte.is_ascii_alphanumeric()),
                    "{position}: {byte}"
                );
            }
        }
    }

    #[test]
    fn matches_character_cache_and_scalar_rule_across_word_boundaries() {
        let scalar = |value: &str| {
            let mut chars = value.chars();
            chars
                .next()
                .is_some_and(|first| u32::from(first) <= 0xff && chars.next().is_none())
                || value
                    .bytes()
                    .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        };
        for length in 0..=24 {
            let mut value = "a".repeat(length);
            assert_eq!(should_intern(&value), scalar(&value));
            for position in 0..=length {
                for ch in [
                    '\0', '.', '_', 'Z', '9', '\u{7f}', '\u{80}', 'é', 'ÿ', 'Ā', '🦀',
                ] {
                    value.insert(position, ch);
                    assert_eq!(should_intern(&value), scalar(&value), "{value:?}");
                    value.remove(position);
                }
            }
        }
        for ch in ['\0', '.', '\u{7f}', '\u{80}', 'é', 'ÿ', 'Ā', '🦀'] {
            assert_eq!(is_cached_character(&ch.to_string()), u32::from(ch) <= 0xff);
        }
        assert!(!is_cached_character(""));
        assert!(!is_cached_character("éa"));
    }
}
