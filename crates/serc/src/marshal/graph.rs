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
}

#[derive(Debug, Default)]
pub(super) struct ObjectGraph {
    objects: FxHashMap<ObjectKey, ObjectId>,
    objects_by_id: Vec<Object>,
    locations: FxHashMap<ObjectLocation, ObjectId>,
    interned_strings: FxHashSet<String>,
}

impl ObjectGraph {
    pub(super) fn for_code(code: &CodeObject) -> Self {
        let mut graph = Self::default();
        graph.visit_code(code);
        graph
    }

    #[cfg(test)]
    pub(super) fn record(&mut self, key: ObjectKey) -> bool {
        self.record_object(key).1
    }

    fn record_at(&mut self, location: ObjectLocation, key: ObjectKey) -> bool {
        let (id, first) = self.record_object(key);
        if let Some(previous) = self.locations.insert(location, id) {
            debug_assert_eq!(previous, id, "object location changed identity");
        }
        first
    }

    fn record_object(&mut self, key: ObjectKey) -> (ObjectId, bool) {
        let cached = self.is_cached(&key);
        if let Some(&id) = self.objects.get(&key) {
            let object = &mut self.objects_by_id[id.0];
            object.count += 1;
            object.cached |= cached;
            return (id, false);
        }

        let id = ObjectId(self.objects_by_id.len());
        self.objects.insert(key, id);
        self.objects_by_id.push(Object { count: 1, cached });
        (id, true)
    }

    fn visit_code(&mut self, code: &CodeObject) {
        let address = std::ptr::from_ref(code).addr();
        if !self.record_at(ObjectLocation::Code(address), code_key(code)) {
            return;
        }
        self.record_at(ObjectLocation::CodeBytes(address), code_bytes_key(code));
        self.interned_strings
            .extend(code.interned_constant_strings.iter().cloned());
        self.visit_constants_tuple(&code.constants);
        self.visit_strings_tuple(&code.names);
        self.visit_locals_tuple(code);
        self.record_at(ObjectLocation::LocalKinds(address), local_kinds_key(code));
        self.visit_metadata_string(&code.filename);
        self.visit_metadata_string(&code.name);
        self.visit_metadata_string(&code.qualified_name);
        self.record_at(
            ObjectLocation::Bytes(code.line_table.as_ptr().addr()),
            ObjectKey::Bytes(code.line_table.clone()),
        );
        self.record_at(
            ObjectLocation::Bytes(code.exception_table.as_ptr().addr()),
            ObjectKey::Bytes(code.exception_table.clone()),
        );
    }

    fn visit_constants_tuple(&mut self, values: &[Constant]) {
        let key = constants_tuple_key(values);
        if !self.record_at(ObjectLocation::ConstantsTuple(values.as_ptr().addr()), key) {
            return;
        }
        for value in values {
            self.visit_constant(value);
        }
    }

    fn visit_strings_tuple(&mut self, values: &[String]) {
        let key = strings_tuple_key(values);
        if !self.record_at(ObjectLocation::StringsTuple(values.as_ptr().addr()), key) {
            return;
        }
        for value in values {
            self.visit_metadata_string(value);
        }
    }

    fn visit_locals_tuple(&mut self, code: &CodeObject) {
        let key = locals_tuple_key(code);
        if !self.record_at(
            ObjectLocation::LocalsTuple(std::ptr::from_ref(code).addr()),
            key,
        ) {
            return;
        }
        for value in &code.locals {
            self.visit_metadata_string(value);
        }
    }

    fn visit_metadata_string(&mut self, value: &str) {
        self.interned_strings.insert(value.to_string());
        self.record_at(
            ObjectLocation::MetadataString(value.as_ptr().addr()),
            ObjectKey::String(value.to_string()),
        );
    }

    fn visit_constant(&mut self, value: &Constant) {
        let location = ObjectLocation::Constant(std::ptr::from_ref(value).addr());
        match value {
            Constant::None | Constant::Bool(_) | Constant::Ellipsis => {}
            Constant::Int(value) => {
                self.record_at(location, ObjectKey::Int(*value));
            }
            Constant::SignedInt(value) => {
                self.record_at(location, ObjectKey::SignedInt(*value));
            }
            Constant::BigInt { negative, digits } => {
                self.record_at(location, ObjectKey::BigInt(*negative, digits.clone()));
            }
            Constant::Float(value) => {
                self.record_at(location, ObjectKey::Float(value.to_bits()));
            }
            Constant::Complex { real, imag } => {
                self.record_at(location, ObjectKey::Complex(real.to_bits(), imag.to_bits()));
            }
            Constant::String(value) => {
                if should_intern(value) {
                    self.interned_strings.insert(value.clone());
                }
                self.record_at(location, ObjectKey::String(value.clone()));
            }
            Constant::SurrogateString(value) => {
                self.record_at(location, ObjectKey::SurrogateString(value.clone()));
            }
            Constant::Bytes(value) => {
                self.record_at(location, ObjectKey::Bytes(value.clone()));
            }
            Constant::Tuple(values) => self.visit_constants_tuple(values),
            Constant::FrozenSet(values) => {
                let key = constant_key(value);
                if self.record_at(location, key) {
                    let mut values: Vec<_> = values.iter().collect();
                    values.sort_by_key(|value| constant_sort_key(value));
                    for value in values {
                        self.visit_constant(value);
                    }
                }
            }
            Constant::Slice { lower, upper, step } => {
                let key = constant_key(value);
                if self.record_at(location, key) {
                    self.visit_slice_member(lower);
                    self.visit_slice_member(upper);
                    self.visit_slice_member(step);
                }
            }
            Constant::Code(code) => self.visit_code(code),
        }
    }

    fn visit_slice_member(&mut self, value: &Constant) {
        if slice_member_uses_identity(value) {
            let address = std::ptr::from_ref(value).addr();
            self.record_at(
                ObjectLocation::SliceMember(address),
                ObjectKey::SliceMember(address),
            );
        } else {
            self.visit_constant(value);
        }
    }

    #[cfg(test)]
    pub(super) fn is_shared(&self, key: &ObjectKey, force_reference: bool) -> bool {
        if force_reference
            || self
                .objects
                .get(key)
                .is_some_and(|id| self.objects_by_id[id.0].count > 1)
        {
            return true;
        }
        self.is_cached(key)
    }

    fn is_cached(&self, key: &ObjectKey) -> bool {
        match key {
            ObjectKey::Int(value) => *value <= 256,
            ObjectKey::SignedInt(value) => (-5..=256).contains(value),
            ObjectKey::String(value) => self.interned_strings.contains(value),
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

    pub(super) fn is_interned(&self, value: &str) -> bool {
        self.interned_strings.contains(value)
    }
}
