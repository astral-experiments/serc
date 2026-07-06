use std::collections::hash_map::Entry;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::compiler::{CodeObject, Constant};

use super::key::{
    ObjectKey, code_bytes_key, code_key, constant_key, constant_sort_key, constants_tuple_key,
    local_kinds_key, locals_tuple_key, should_intern, slice_member_uses_identity,
    strings_tuple_key,
};

// Source addresses are graph-local lookup keys only. They remain valid while `encode_code` holds
// the immutable `CodeObject`, and are never written to the marshal stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ObjectLocation {
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

// Equal value keys share an ID; identity-based keys remain distinct exactly as `ObjectKey` does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectId(pub(super) usize);

#[derive(Debug)]
struct Object {
    count: usize,
    cached: bool,
    interned: bool,
}

#[derive(Debug, Default)]
pub(super) struct ObjectGraph<'a> {
    objects: FxHashMap<ObjectKey<'a>, ObjectId>,
    objects_by_id: Vec<Object>,
    locations: FxHashMap<ObjectLocation, ObjectId>,
}

impl<'a> ObjectGraph<'a> {
    pub(super) fn for_code(code: &'a CodeObject) -> Self {
        let mut explicitly_interned = FxHashSet::default();
        collect_explicitly_interned(code, &mut explicitly_interned);
        let mut graph = Self::default();
        graph.visit_code(code, &explicitly_interned);
        graph
    }

    #[cfg(test)]
    pub(super) fn record(&mut self, key: ObjectKey<'a>) -> bool {
        self.record_object(key, false).1
    }

    fn record_at(&mut self, location: ObjectLocation, key: ObjectKey<'a>, interned: bool) -> bool {
        let (id, first) = self.record_object(key, interned);
        if let Some(previous) = self.locations.insert(location, id) {
            debug_assert_eq!(previous, id, "object location changed identity");
        }
        first
    }

    fn record_object(&mut self, key: ObjectKey<'a>, interned: bool) -> (ObjectId, bool) {
        let next_id = ObjectId(self.objects_by_id.len());
        match self.objects.entry(key) {
            Entry::Occupied(entry) => {
                let cached = interned || Self::is_cached(entry.key());
                let id = *entry.get();
                let object = &mut self.objects_by_id[id.0];
                object.count += 1;
                object.cached |= cached;
                object.interned |= interned;
                (id, false)
            }
            Entry::Vacant(entry) => {
                let cached = interned || Self::is_cached(entry.key());
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

    fn visit_code(&mut self, code: &'a CodeObject, explicitly_interned: &FxHashSet<&'a str>) {
        let address = std::ptr::from_ref(code).addr();
        if !self.record_at(ObjectLocation::Code(address), code_key(code), false) {
            return;
        }
        self.record_at(
            ObjectLocation::CodeBytes(address),
            code_bytes_key(code),
            false,
        );
        self.visit_constants_tuple(&code.constants, explicitly_interned);
        self.visit_strings_tuple(&code.names);
        self.visit_locals_tuple(code);
        self.record_at(
            ObjectLocation::LocalKinds(address),
            local_kinds_key(code),
            false,
        );
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

    fn visit_constants_tuple(
        &mut self,
        values: &'a [Constant],
        explicitly_interned: &FxHashSet<&'a str>,
    ) {
        let key = constants_tuple_key(values);
        if !self.record_at(
            ObjectLocation::ConstantsTuple(values.as_ptr().addr()),
            key,
            false,
        ) {
            return;
        }
        for value in values {
            self.visit_constant(value, explicitly_interned);
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
        let key = locals_tuple_key(code);
        if !self.record_at(
            ObjectLocation::LocalsTuple(std::ptr::from_ref(code).addr()),
            key,
            false,
        ) {
            return;
        }
        for value in &code.locals {
            self.visit_metadata_string(value);
        }
    }

    fn visit_metadata_string(&mut self, value: &'a str) {
        self.record_at(
            ObjectLocation::MetadataString(value.as_ptr().addr()),
            ObjectKey::String(value.into()),
            true,
        );
    }

    fn visit_constant(&mut self, value: &'a Constant, explicitly_interned: &FxHashSet<&'a str>) {
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
                self.record_at(
                    location,
                    ObjectKey::String(value.as_str().into()),
                    explicitly_interned.contains(value.as_str()) || should_intern(value),
                );
            }
            Constant::SurrogateString(value) => {
                self.record_at(location, ObjectKey::SurrogateString(value), false);
            }
            Constant::Bytes(value) => {
                self.record_at(location, ObjectKey::Bytes(value), false);
            }
            Constant::Tuple(values) => self.visit_constants_tuple(values, explicitly_interned),
            Constant::FrozenSet(values) => {
                let key = constant_key(value);
                if self.record_at(location, key, false) {
                    let mut values: Vec<_> = values.iter().collect();
                    values.sort_by_key(|value| constant_sort_key(value));
                    for value in values {
                        self.visit_constant(value, explicitly_interned);
                    }
                }
            }
            Constant::Slice { lower, upper, step } => {
                let key = constant_key(value);
                if self.record_at(location, key, false) {
                    self.visit_slice_member(lower, explicitly_interned);
                    self.visit_slice_member(upper, explicitly_interned);
                    self.visit_slice_member(step, explicitly_interned);
                }
            }
            Constant::Code(code) => self.visit_code(code, explicitly_interned),
        }
    }

    fn visit_slice_member(
        &mut self,
        value: &'a Constant,
        explicitly_interned: &FxHashSet<&'a str>,
    ) {
        if slice_member_uses_identity(value) {
            let address = std::ptr::from_ref(value).addr();
            self.record_at(
                ObjectLocation::SliceMember(address),
                ObjectKey::SliceMember(address),
                false,
            );
        } else {
            self.visit_constant(value, explicitly_interned);
        }
    }

    #[cfg(test)]
    pub(super) fn is_shared(&self, key: &ObjectKey<'a>, force_reference: bool) -> bool {
        if force_reference
            || self
                .objects
                .get(key)
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

    pub(super) fn is_shared_object(&self, id: ObjectId, force_reference: bool) -> bool {
        let object = &self.objects_by_id[id.0];
        force_reference || object.count > 1 || object.cached
    }

    pub(super) fn object_count(&self) -> usize {
        self.objects_by_id.len()
    }

    fn object_at(&self, location: ObjectLocation) -> ObjectId {
        self.locations[&location]
    }

    pub(super) fn bytes(&self, value: &[u8]) -> ObjectId {
        self.object_at(ObjectLocation::Bytes(value.as_ptr().addr()))
    }

    pub(super) fn code(&self, code: &CodeObject) -> ObjectId {
        self.object_at(ObjectLocation::Code(std::ptr::from_ref(code).addr()))
    }

    pub(super) fn code_bytes(&self, code: &CodeObject) -> ObjectId {
        self.object_at(ObjectLocation::CodeBytes(std::ptr::from_ref(code).addr()))
    }

    pub(super) fn constant(&self, value: &Constant) -> ObjectId {
        self.object_at(ObjectLocation::Constant(std::ptr::from_ref(value).addr()))
    }

    pub(super) fn constants_tuple(&self, values: &[Constant]) -> ObjectId {
        self.object_at(ObjectLocation::ConstantsTuple(values.as_ptr().addr()))
    }

    pub(super) fn local_kinds(&self, code: &CodeObject) -> ObjectId {
        self.object_at(ObjectLocation::LocalKinds(std::ptr::from_ref(code).addr()))
    }

    pub(super) fn locals_tuple(&self, code: &CodeObject) -> ObjectId {
        self.object_at(ObjectLocation::LocalsTuple(std::ptr::from_ref(code).addr()))
    }

    pub(super) fn metadata_string(&self, value: &str) -> ObjectId {
        self.object_at(ObjectLocation::MetadataString(value.as_ptr().addr()))
    }

    pub(super) fn slice_member(&self, value: &Constant) -> ObjectId {
        self.object_at(ObjectLocation::SliceMember(
            std::ptr::from_ref(value).addr(),
        ))
    }

    pub(super) fn strings_tuple(&self, values: &[String]) -> ObjectId {
        self.object_at(ObjectLocation::StringsTuple(values.as_ptr().addr()))
    }

    pub(super) fn is_interned_object(&self, id: ObjectId) -> bool {
        self.objects_by_id[id.0].interned
    }

    #[cfg(test)]
    pub(super) fn is_interned(&self, value: &str) -> bool {
        self.objects
            .iter()
            .find_map(|(key, id)| match key {
                ObjectKey::String(string) if string.as_str() == value => Some(*id),
                _ => None,
            })
            .is_some_and(|id| self.objects_by_id[id.0].interned)
    }
}

fn collect_explicitly_interned<'a>(code: &'a CodeObject, strings: &mut FxHashSet<&'a str>) {
    strings.extend(code.interned_constant_strings.iter().map(String::as_str));
    for constant in &code.constants {
        collect_nested_explicitly_interned(constant, strings);
    }
}

fn collect_nested_explicitly_interned<'a>(
    constant: &'a Constant,
    strings: &mut FxHashSet<&'a str>,
) {
    match constant {
        Constant::Tuple(values) | Constant::FrozenSet(values) => {
            for value in values {
                collect_nested_explicitly_interned(value, strings);
            }
        }
        Constant::Slice { lower, upper, step } => {
            collect_nested_explicitly_interned(lower, strings);
            collect_nested_explicitly_interned(upper, strings);
            collect_nested_explicitly_interned(step, strings);
        }
        Constant::Code(code) => collect_explicitly_interned(code, strings),
        _ => {}
    }
}
