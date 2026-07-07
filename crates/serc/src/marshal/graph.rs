use std::collections::hash_map::Entry;

use rustc_hash::FxHashMap;
#[cfg(debug_assertions)]
use rustc_hash::FxHashSet;

use crate::code_object::{CodeObject, Constant};

use super::key::{
    HashedObjectKey, ObjectKey, code_bytes_key, code_key, constant_key, constants_tuple_key,
    frozen_set_members_in_wire_order, local_kinds_key, locals_tuple_key, should_intern,
    slice_member_uses_identity, strings_tuple_key,
};

// Source addresses are graph-local lookup keys only. They remain valid while `encode_code` holds
// the immutable `CodeObject`, and are never written to the marshal stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ObjectLocation {
    Bytes(usize),
    Code(usize),
    CodeBytes(usize),
    Constant(usize),
    ConstantsTuple(usize),
    LocalKinds(usize),
    LocalsTuple(usize),
    MetadataString(usize),
    SliceMember(usize),
    StringsTuple(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum ObjectLocationKind {
    Bytes,
    Code,
    CodeBytes,
    Constant,
    ConstantsTuple,
    LocalKinds,
    LocalsTuple,
    MetadataString,
    SliceMember,
    StringsTuple,
}

impl ObjectLocation {
    fn into_parts(self) -> (ObjectLocationKind, usize) {
        match self {
            Self::Bytes(address) => (ObjectLocationKind::Bytes, address),
            Self::Code(address) => (ObjectLocationKind::Code, address),
            Self::CodeBytes(address) => (ObjectLocationKind::CodeBytes, address),
            Self::Constant(address) => (ObjectLocationKind::Constant, address),
            Self::ConstantsTuple(address) => (ObjectLocationKind::ConstantsTuple, address),
            Self::LocalKinds(address) => (ObjectLocationKind::LocalKinds, address),
            Self::LocalsTuple(address) => (ObjectLocationKind::LocalsTuple, address),
            Self::MetadataString(address) => (ObjectLocationKind::MetadataString, address),
            Self::SliceMember(address) => (ObjectLocationKind::SliceMember, address),
            Self::StringsTuple(address) => (ObjectLocationKind::StringsTuple, address),
        }
    }
}

// Equal value keys share an ID; identity-based keys remain distinct exactly as `ObjectKey` does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectId(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Occurrence {
    address: usize,
    object: u32,
    kind: ObjectLocationKind,
}

#[cfg(target_pointer_width = "64")]
const _: [(); 16] = [(); std::mem::size_of::<Occurrence>()];

impl Occurrence {
    fn new(location: ObjectLocation, object: ObjectId) -> Self {
        let (kind, address) = location.into_parts();
        Self {
            address,
            object: object.0.try_into().expect("marshal object limit exceeded"),
            kind,
        }
    }

    fn location(self) -> ObjectLocation {
        match self.kind {
            ObjectLocationKind::Bytes => ObjectLocation::Bytes(self.address),
            ObjectLocationKind::Code => ObjectLocation::Code(self.address),
            ObjectLocationKind::CodeBytes => ObjectLocation::CodeBytes(self.address),
            ObjectLocationKind::Constant => ObjectLocation::Constant(self.address),
            ObjectLocationKind::ConstantsTuple => ObjectLocation::ConstantsTuple(self.address),
            ObjectLocationKind::LocalKinds => ObjectLocation::LocalKinds(self.address),
            ObjectLocationKind::LocalsTuple => ObjectLocation::LocalsTuple(self.address),
            ObjectLocationKind::MetadataString => ObjectLocation::MetadataString(self.address),
            ObjectLocationKind::SliceMember => ObjectLocation::SliceMember(self.address),
            ObjectLocationKind::StringsTuple => ObjectLocation::StringsTuple(self.address),
        }
    }
}

#[derive(Debug)]
pub(super) struct OccurrenceCursor<'a> {
    occurrences: &'a [Occurrence],
    next: usize,
}

impl OccurrenceCursor<'_> {
    #[inline]
    pub(super) fn consume(&mut self, expected: ObjectLocation) -> ObjectId {
        let index = self.next;
        let Some(occurrence) = self.occurrences.get(index) else {
            missing_occurrence(index, expected, self.occurrences.len());
        };
        let (kind, address) = expected.into_parts();
        if occurrence.kind != kind || occurrence.address != address {
            mismatched_occurrence(index, expected, occurrence.location());
        }
        self.next += 1;
        ObjectId(occurrence.object as usize)
    }

    pub(super) fn finish(self) {
        assert_eq!(
            self.next,
            self.occurrences.len(),
            "marshal writer consumed {} of {} recorded occurrences",
            self.next,
            self.occurrences.len()
        );
    }
}

#[cold]
#[inline(never)]
fn missing_occurrence(index: usize, expected: ObjectLocation, count: usize) -> ! {
    panic!(
        "marshal writer requested occurrence {index} ({expected:?}), but the graph recorded {count} occurrences"
    );
}

#[cold]
#[inline(never)]
fn mismatched_occurrence(index: usize, expected: ObjectLocation, recorded: ObjectLocation) -> ! {
    panic!(
        "marshal occurrence {index} mismatch: writer requested {expected:?}, graph recorded {recorded:?}"
    );
}

#[derive(Debug)]
struct Object {
    count: usize,
    cached: bool,
    interned: bool,
}

const CODE_STRUCTURAL_CHILDREN: usize = 10;

#[derive(Debug, Default)]
pub(super) struct ObjectGraph<'a> {
    objects: FxHashMap<HashedObjectKey<'a>, ObjectId>,
    objects_by_id: Vec<Object>,
    occurrences: Vec<Occurrence>,
    #[cfg(debug_assertions)]
    unique_objects: FxHashSet<HashedObjectKey<'a>>,
    explicitly_interned: Vec<&'a str>,
}

impl<'a> ObjectGraph<'a> {
    pub(super) fn for_code(code: &'a CodeObject) -> Self {
        let mut graph = Self::default();
        graph.visit_code(code);
        graph.mark_explicitly_interned();
        graph
    }

    fn mark_explicitly_interned(&mut self) {
        for value in std::mem::take(&mut self.explicitly_interned) {
            let Some(id) = self
                .objects
                .get(&HashedObjectKey::from(ObjectKey::String(value)))
                .copied()
            else {
                continue;
            };
            let object = &mut self.objects_by_id[id.0];
            object.cached = true;
            object.interned = true;
        }
    }

    #[cfg(test)]
    pub(super) fn record(&mut self, key: ObjectKey<'a>) -> bool {
        self.record_object(key.into(), false).1
    }

    fn record_at(&mut self, location: ObjectLocation, key: ObjectKey<'a>, interned: bool) -> bool {
        let (id, first) = self.record_object(key.into(), interned);
        self.occurrences.push(Occurrence::new(location, id));
        first
    }

    /// Records an identity-bearing object that is structurally owned by a unique parent.
    fn record_unique_at(&mut self, location: ObjectLocation, _key: ObjectKey<'a>) {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.unique_objects.insert(_key.into()),
            "identity-bearing marshal object was recorded twice at {location:?}"
        );

        let id = ObjectId(self.objects_by_id.len());
        self.objects_by_id.push(Object {
            count: 1,
            cached: false,
            interned: false,
        });
        self.occurrences.push(Occurrence::new(location, id));
    }

    fn record_object(&mut self, key: HashedObjectKey<'a>, interned: bool) -> (ObjectId, bool) {
        let next_id = ObjectId(self.objects_by_id.len());
        match self.objects.entry(key) {
            Entry::Occupied(entry) => {
                let cached = interned || Self::is_cached(entry.key().key());
                let id = *entry.get();
                let object = &mut self.objects_by_id[id.0];
                object.count += 1;
                object.cached |= cached;
                object.interned |= interned;
                (id, false)
            }
            Entry::Vacant(entry) => {
                let cached = interned || Self::is_cached(entry.key().key());
                entry.insert(next_id);
                self.objects_by_id.push(Object {
                    count: 1,
                    cached,
                    interned,
                });
                (next_id, true)
            }
        }
    }

    fn visit_code(&mut self, code: &'a CodeObject) {
        let address = std::ptr::from_ref(code).addr();
        // Every code object contributes ten object-bearing structural children, plus at most one
        // object for each direct constant, name, and local. Leave geometric headroom for
        // descendants of container constants; nested code objects refine the estimate when they
        // are visited.
        let direct_children = code
            .constants
            .len()
            .saturating_add(code.names.len())
            .saturating_add(code.locals.len())
            .saturating_add(CODE_STRUCTURAL_CHILDREN);
        let estimated_descendants = direct_children.saturating_mul(2);
        self.objects.reserve(estimated_descendants);
        self.objects_by_id
            .reserve(estimated_descendants.saturating_add(1));
        self.occurrences
            .reserve(estimated_descendants.saturating_add(1));
        self.record_unique_at(ObjectLocation::Code(address), code_key(code));
        self.explicitly_interned.extend(
            code.marshal_hints
                .explicitly_interned_strings
                .iter()
                .map(String::as_str),
        );
        self.record_unique_at(ObjectLocation::CodeBytes(address), code_bytes_key(code));
        self.visit_constants_tuple(&code.constants);
        self.visit_strings_tuple(&code.names);
        self.visit_locals_tuple(code);
        let local_kinds_key = local_kinds_key(code);
        if matches!(&local_kinds_key, ObjectKey::LocalKinds(_)) {
            self.record_unique_at(ObjectLocation::LocalKinds(address), local_kinds_key);
        } else {
            self.record_at(ObjectLocation::LocalKinds(address), local_kinds_key, false);
        }
        self.visit_metadata_string(&code.filename);
        self.visit_metadata_string(&code.name);
        self.visit_metadata_string(&code.qualified_name);
        self.record_at(
            ObjectLocation::Bytes(code.line_table.as_ptr().addr()),
            ObjectKey::Bytes(&code.line_table),
            false,
        );
        self.record_at(
            ObjectLocation::Bytes(code.exception_table.as_ptr().addr()),
            ObjectKey::Bytes(&code.exception_table),
            false,
        );
    }

    fn visit_constants_tuple(&mut self, values: &'a [Constant]) {
        let location = ObjectLocation::ConstantsTuple(values.as_ptr().addr());
        let key = constants_tuple_key(values);
        if values
            .iter()
            .any(|value| matches!(value, Constant::Code(_)))
        {
            self.record_unique_at(location, key);
        } else if !self.record_at(location, key, false) {
            return;
        }
        for value in values {
            self.visit_constant(value);
        }
    }

    fn visit_strings_tuple(&mut self, values: &'a [String]) {
        let key = strings_tuple_key(values);
        if !self.record_at(
            ObjectLocation::StringsTuple(values.as_ptr().addr()),
            key,
            false,
        ) {
            return;
        }
        for value in values {
            self.visit_metadata_string(value);
        }
    }

    fn visit_locals_tuple(&mut self, code: &'a CodeObject) {
        let location = ObjectLocation::LocalsTuple(std::ptr::from_ref(code).addr());
        let key = locals_tuple_key(code);
        let first = if matches!(&key, ObjectKey::AnnotationLocals(_)) {
            self.record_unique_at(location, key);
            true
        } else {
            self.record_at(location, key, false)
        };
        if !first {
            return;
        }
        for value in &code.locals {
            self.visit_metadata_string(value);
        }
    }

    fn visit_metadata_string(&mut self, value: &'a str) {
        self.record_at(
            ObjectLocation::MetadataString(value.as_ptr().addr()),
            ObjectKey::String(value),
            true,
        );
    }

    fn visit_constant(&mut self, value: &'a Constant) {
        let location = ObjectLocation::Constant(std::ptr::from_ref(value).addr());
        match value {
            Constant::None | Constant::Bool(_) | Constant::Ellipsis => {}
            Constant::Int(value) => {
                self.record_at(location, ObjectKey::Int(*value), false);
            }
            Constant::SignedInt(value) => {
                self.record_at(location, ObjectKey::SignedInt(*value), false);
            }
            Constant::BigInt { negative, digits } => {
                self.record_at(location, ObjectKey::BigInt(*negative, digits), false);
            }
            Constant::Float(value) => {
                self.record_at(location, ObjectKey::Float(value.to_bits()), false);
            }
            Constant::Complex { real, imag } => {
                self.record_at(
                    location,
                    ObjectKey::Complex(real.to_bits(), imag.to_bits()),
                    false,
                );
            }
            Constant::String(value) => {
                self.record_at(location, ObjectKey::String(value), should_intern(value));
            }
            Constant::SurrogateString(value) => {
                self.record_at(location, ObjectKey::SurrogateString(value), false);
            }
            Constant::Bytes(value) => {
                self.record_at(location, ObjectKey::Bytes(value), false);
            }
            Constant::Tuple(values) => self.visit_constants_tuple(values),
            Constant::FrozenSet(values) => {
                let key = constant_key(value);
                if self.record_at(location, key, false) {
                    for (_, value) in frozen_set_members_in_wire_order(values) {
                        self.visit_constant(value);
                    }
                }
            }
            Constant::Slice { lower, upper, step } => {
                let key = constant_key(value);
                if self.record_at(location, key, false) {
                    self.visit_slice_member(lower);
                    self.visit_slice_member(upper);
                    self.visit_slice_member(step);
                }
            }
            Constant::Code(code) => self.visit_code(code),
        }
    }

    fn visit_slice_member(&mut self, value: &'a Constant) {
        if slice_member_uses_identity(value) {
            let address = std::ptr::from_ref(value).addr();
            self.record_unique_at(
                ObjectLocation::SliceMember(address),
                ObjectKey::SliceMember(address),
            );
        } else {
            self.visit_constant(value);
        }
    }

    #[cfg(test)]
    pub(super) fn is_shared(&self, key: &ObjectKey<'a>, force_reference: bool) -> bool {
        if force_reference
            || self
                .objects
                .get(&HashedObjectKey::from(key.clone()))
                .is_some_and(|id| self.objects_by_id[id.0].count > 1)
        {
            return true;
        }
        Self::is_cached(key)
    }

    fn is_cached(key: &ObjectKey<'_>) -> bool {
        match key {
            ObjectKey::Int(value) => *value <= 256,
            ObjectKey::SignedInt(value) => (-5..=256).contains(value),
            ObjectKey::Bytes(value) => value.len() <= 1,
            ObjectKey::EmptyTuple => true,
            _ => false,
        }
    }

    pub(super) fn is_shared_object(&self, id: ObjectId) -> bool {
        let object = &self.objects_by_id[id.0];
        object.count > 1 || object.cached
    }

    pub(super) fn object_count(&self) -> usize {
        self.objects_by_id.len()
    }

    #[cfg(test)]
    pub(super) fn keyed_object_count(&self) -> usize {
        self.objects.len()
    }

    pub(super) fn occurrence_cursor(&self) -> OccurrenceCursor<'_> {
        OccurrenceCursor {
            occurrences: &self.occurrences,
            next: 0,
        }
    }

    pub(super) fn is_interned_object(&self, id: ObjectId) -> bool {
        self.objects_by_id[id.0].interned
    }

    #[cfg(test)]
    pub(super) fn is_interned(&self, value: &str) -> bool {
        self.objects
            .iter()
            .find_map(|(key, id)| match key.key() {
                ObjectKey::String(string) if *string == value => Some(*id),
                _ => None,
            })
            .is_some_and(|id| self.objects_by_id[id.0].interned)
    }
}
