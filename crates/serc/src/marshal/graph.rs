use std::collections::hash_map::Entry;

use rustc_hash::FxHashMap;

use crate::compiler::CodeObject;

use super::key::ObjectKey;
use super::plan::{MarshalPlan, ObjectId, PlannedObject, PlannedValue};

#[derive(Debug)]
struct Object {
    count: usize,
    cached: bool,
    interned: bool,
}

#[derive(Debug, Default)]
pub(super) struct ObjectGraph<'a> {
    plan: MarshalPlan<'a>,
    objects: FxHashMap<ObjectKey<'a>, ObjectId>,
    objects_by_id: Vec<Object>,
    explicitly_interned: Vec<&'a str>,
}

impl<'a> ObjectGraph<'a> {
    pub(super) fn for_code(code: &'a CodeObject) -> Self {
        let mut graph = Self::default();
        let plan = MarshalPlan::for_code(code, |value, object| graph.record_value(value, object));
        graph.plan = plan;
        graph.mark_explicitly_interned();
        graph
    }

    fn record_value(
        &mut self,
        value: PlannedValue<'a>,
        object: PlannedObject<'a>,
    ) -> (ObjectId, bool) {
        let (id, first) = self.record_object(object.key, object.interned);
        if first && let PlannedValue::Code(code) = value {
            self.explicitly_interned
                .extend(code.interned_constant_strings.iter().map(String::as_str));
        }
        (id, first)
    }

    fn mark_explicitly_interned(&mut self) {
        for value in std::mem::take(&mut self.explicitly_interned) {
            let Some(id) = self.objects.get(&ObjectKey::String(value)).copied() else {
                continue;
            };
            let object = &mut self.objects_by_id[id.0];
            object.cached = true;
            object.interned = true;
        }
    }

    #[cfg(test)]
    pub(super) fn record(&mut self, key: ObjectKey<'a>) -> bool {
        self.record_object(key, false).1
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

    pub(super) fn plan(&self) -> &MarshalPlan<'a> {
        &self.plan
    }

    pub(super) fn is_interned_object(&self, id: ObjectId) -> bool {
        self.objects_by_id[id.0].interned
    }

    #[cfg(test)]
    pub(super) fn is_interned(&self, value: &str) -> bool {
        self.objects
            .iter()
            .find_map(|(key, id)| match key {
                ObjectKey::String(string) if *string == value => Some(*id),
                _ => None,
            })
            .is_some_and(|id| self.objects_by_id[id.0].interned)
    }
}
