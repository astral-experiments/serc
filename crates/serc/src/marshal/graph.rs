use std::collections::hash_map::Entry;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::code_object::{CodeObject, Constant};

use super::key::{
    HashedObjectKey, ObjectKey, code_bytes_key, code_key, constant_key, constants_tuple_key,
    local_kinds_key, locals_tuple_key, should_intern, strings_tuple_key,
};
use super::plan::{MarshalVisitor, ObjectId, ReferenceMode, traverse_code};

#[derive(Debug)]
struct Object {
    count: usize,
    cached: bool,
    interned: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum OccurrenceKind {
    Bytes,
    Code,
    CodeBytes,
    Constant,
    Constants,
    LocalKinds,
    Locals,
    MetadataString,
    SliceMember,
    Strings,
}

#[derive(Clone, Copy, Debug)]
struct Occurrence {
    object: ObjectId,
    kind: OccurrenceKind,
}

const _: [(); 8] = [(); std::mem::size_of::<Occurrence>()];

#[derive(Debug)]
pub(super) struct OccurrenceCursor<'a> {
    occurrences: &'a [Occurrence],
    next: usize,
}

impl OccurrenceCursor<'_> {
    #[inline]
    pub(super) fn consume(&mut self, expected: OccurrenceKind) -> ObjectId {
        let index = self.next;
        let Some(occurrence) = self.occurrences.get(index).copied() else {
            missing_occurrence(index, expected, self.occurrences.len());
        };
        if occurrence.kind != expected {
            mismatched_occurrence(index, expected, occurrence.kind);
        }
        self.next += 1;
        occurrence.object
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
fn missing_occurrence(index: usize, expected: OccurrenceKind, count: usize) -> ! {
    panic!(
        "marshal writer requested occurrence {index} ({expected:?}), but the graph recorded {count} occurrences"
    );
}

#[cold]
#[inline(never)]
fn mismatched_occurrence(index: usize, expected: OccurrenceKind, recorded: OccurrenceKind) -> ! {
    panic!(
        "marshal occurrence {index} mismatch: writer requested {expected:?}, graph recorded {recorded:?}"
    );
}

const CODE_OBJECT_CHILDREN: usize = 10;

#[derive(Clone, Copy, Debug)]
enum CachedObject {
    EmptyBytes,
    EmptyTuple,
}

impl CachedObject {
    #[cfg(test)]
    fn for_key(key: &ObjectKey<'_>) -> Option<Self> {
        match key {
            ObjectKey::Bytes([]) => Some(Self::EmptyBytes),
            ObjectKey::EmptyTuple => Some(Self::EmptyTuple),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ObjectGraph<'a> {
    objects: FxHashMap<HashedObjectKey<'a>, ObjectId>,
    cached_empty_bytes: Option<ObjectId>,
    cached_empty_tuple: Option<ObjectId>,
    objects_by_id: Vec<Object>,
    occurrences: Vec<Occurrence>,
    #[cfg(debug_assertions)]
    unique_objects: FxHashSet<HashedObjectKey<'a>>,
}

impl<'a> ObjectGraph<'a> {
    pub(super) fn for_code(code: &'a CodeObject) -> Self {
        let mut explicitly_interned = FxHashSet::default();
        collect_explicitly_interned(code, &mut explicitly_interned);
        let mut graph = Self::default();
        traverse_code(
            code,
            &mut GraphVisitor {
                graph: &mut graph,
                explicitly_interned: &explicitly_interned,
            },
        );
        graph
    }

    fn reserve_for_code(&mut self, code: &CodeObject) {
        // Every code object contributes ten object-bearing structural children, plus at most one
        // object for each direct constant, name, and local. Leave geometric headroom for
        // descendants of container constants; nested code objects refine the estimate when they
        // are visited.
        let direct_children = code
            .constants
            .len()
            .saturating_add(code.names.len())
            .saturating_add(code.locals.len())
            .saturating_add(CODE_OBJECT_CHILDREN);
        let estimated_descendants = direct_children.saturating_mul(2);
        self.objects.reserve(estimated_descendants);
        self.objects_by_id
            .reserve(estimated_descendants.saturating_add(1));
        self.occurrences
            .reserve(estimated_descendants.saturating_add(1));
    }

    fn record_keyed(&mut self, kind: OccurrenceKind, key: ObjectKey<'a>, interned: bool) -> bool {
        let (id, first) = self.record_object(key.into(), interned);
        self.occurrences.push(Occurrence { object: id, kind });
        first
    }

    fn record_cached_value(&mut self, kind: OccurrenceKind, cached: CachedObject) -> bool {
        let (id, first) = self.record_cached(cached);
        self.occurrences.push(Occurrence { object: id, kind });
        first
    }

    fn record_unique_value(&mut self, kind: OccurrenceKind, key: ObjectKey<'a>) -> bool {
        let id = self.record_unique(key);
        self.occurrences.push(Occurrence { object: id, kind });
        true
    }

    #[cfg(test)]
    pub(super) fn record(&mut self, key: ObjectKey<'a>) -> bool {
        if let Some(cached) = CachedObject::for_key(&key) {
            self.record_cached(cached).1
        } else {
            self.record_object(key.into(), false).1
        }
    }

    /// Records an identity-bearing object that is structurally owned by a unique parent.
    fn record_unique(&mut self, _key: ObjectKey<'a>) -> ObjectId {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.unique_objects.insert(_key.into()),
            "identity-bearing marshal object was recorded twice"
        );

        let id = ObjectId::from_index(self.objects_by_id.len());
        self.objects_by_id.push(Object {
            count: 1,
            cached: false,
            interned: false,
        });
        id
    }

    fn record_cached(&mut self, cached: CachedObject) -> (ObjectId, bool) {
        let slot = match cached {
            CachedObject::EmptyBytes => &mut self.cached_empty_bytes,
            CachedObject::EmptyTuple => &mut self.cached_empty_tuple,
        };
        if let Some(id) = *slot {
            let object = &mut self.objects_by_id[id.index()];
            object.count += 1;
            return (id, false);
        }

        let id = ObjectId::from_index(self.objects_by_id.len());
        self.objects_by_id.push(Object {
            count: 1,
            cached: true,
            interned: false,
        });
        debug_assert!(slot.is_none());
        *slot = Some(id);
        (id, true)
    }

    fn record_object(&mut self, key: HashedObjectKey<'a>, interned: bool) -> (ObjectId, bool) {
        let next_id = ObjectId::from_index(self.objects_by_id.len());
        match self.objects.entry(key) {
            Entry::Occupied(entry) => {
                let cached = interned || Self::is_cached(entry.key().key());
                let id = *entry.get();
                let object = &mut self.objects_by_id[id.index()];
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

    #[cfg(test)]
    pub(super) fn is_shared(&self, key: &ObjectKey<'a>, force_reference: bool) -> bool {
        if force_reference
            || self
                .objects
                .get(&HashedObjectKey::from(key.clone()))
                .is_some_and(|id| self.objects_by_id[id.index()].count > 1)
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
        let object = &self.objects_by_id[id.index()];
        object.count > 1 || object.cached
    }

    pub(super) fn object_count(&self) -> usize {
        self.objects_by_id.len()
    }

    #[cfg(test)]
    pub(super) fn keyed_object_count(&self) -> usize {
        self.objects.len()
            + usize::from(self.cached_empty_bytes.is_some())
            + usize::from(self.cached_empty_tuple.is_some())
    }

    #[cfg(test)]
    pub(super) fn hashed_object_count(&self) -> usize {
        self.objects.len()
    }

    pub(super) fn occurrence_cursor(&self) -> OccurrenceCursor<'_> {
        OccurrenceCursor {
            occurrences: &self.occurrences,
            next: 0,
        }
    }

    pub(super) fn is_interned_object(&self, id: ObjectId) -> bool {
        self.objects_by_id[id.index()].interned
    }

    #[cfg(test)]
    pub(super) fn is_interned(&self, value: &str) -> bool {
        self.objects
            .iter()
            .find_map(|(key, id)| match key.key() {
                ObjectKey::String(string) if *string == value => Some(*id),
                _ => None,
            })
            .is_some_and(|id| self.objects_by_id[id.index()].interned)
    }
}

fn collect_explicitly_interned<'a>(code: &'a CodeObject, values: &mut FxHashSet<&'a str>) {
    // A class body loads its qualified name from `co_consts` while the same string is also the
    // code object's interned metadata. CPython deliberately reuses that identity.
    values.insert(&code.qualified_name);
    values.extend(
        code.marshal_hints
            .explicitly_interned_strings
            .iter()
            .map(String::as_str),
    );
    for constant in &code.constants {
        collect_explicitly_interned_from_constant(constant, values);
    }
}

fn collect_explicitly_interned_from_constant<'a>(
    constant: &'a Constant,
    values: &mut FxHashSet<&'a str>,
) {
    match constant {
        Constant::Code(code) => collect_explicitly_interned(code, values),
        Constant::Tuple(constants) | Constant::FrozenSet(constants) => {
            for constant in constants {
                collect_explicitly_interned_from_constant(constant, values);
            }
        }
        Constant::Slice { lower, upper, step } => {
            collect_explicitly_interned_from_constant(lower, values);
            collect_explicitly_interned_from_constant(upper, values);
            collect_explicitly_interned_from_constant(step, values);
        }
        Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::String(_)
        | Constant::SurrogateString(_)
        | Constant::Bytes(_) => {}
    }
}

struct GraphVisitor<'graph, 'code> {
    graph: &'graph mut ObjectGraph<'code>,
    explicitly_interned: &'graph FxHashSet<&'code str>,
}

impl<'code> MarshalVisitor<'code> for GraphVisitor<'_, 'code> {
    #[inline]
    fn code(&mut self, code: &'code CodeObject, _reference_mode: ReferenceMode) -> bool {
        self.graph.reserve_for_code(code);
        self.graph
            .record_unique_value(OccurrenceKind::Code, code_key(code))
    }

    #[inline]
    fn code_bytes(&mut self, code: &'code CodeObject) {
        self.graph
            .record_unique_value(OccurrenceKind::CodeBytes, code_bytes_key(code));
    }

    #[inline]
    fn constants(&mut self, values: &'code [Constant], _reference_mode: ReferenceMode) -> bool {
        if values.is_empty() {
            return self
                .graph
                .record_cached_value(OccurrenceKind::Constants, CachedObject::EmptyTuple);
        }
        let key = constants_tuple_key(values);
        if values
            .iter()
            .any(|value| matches!(value, Constant::Code(_)))
        {
            self.graph
                .record_unique_value(OccurrenceKind::Constants, key)
        } else {
            self.graph
                .record_keyed(OccurrenceKind::Constants, key, false)
        }
    }

    #[inline]
    fn strings(&mut self, values: &'code [String]) -> bool {
        if values.is_empty() {
            return self
                .graph
                .record_cached_value(OccurrenceKind::Strings, CachedObject::EmptyTuple);
        }
        self.graph
            .record_keyed(OccurrenceKind::Strings, strings_tuple_key(values), false)
    }

    #[inline]
    fn locals(&mut self, code: &'code CodeObject) -> bool {
        if code.locals.is_empty() {
            return self
                .graph
                .record_cached_value(OccurrenceKind::Locals, CachedObject::EmptyTuple);
        }
        let key = locals_tuple_key(code);
        if matches!(&key, ObjectKey::AnnotationLocals(_)) {
            self.graph.record_unique_value(OccurrenceKind::Locals, key)
        } else {
            self.graph.record_keyed(OccurrenceKind::Locals, key, false)
        }
    }

    #[inline]
    fn local_kinds(&mut self, code: &'code CodeObject) {
        if code.local_kinds.is_empty() {
            self.graph
                .record_cached_value(OccurrenceKind::LocalKinds, CachedObject::EmptyBytes);
        } else {
            self.graph
                .record_unique_value(OccurrenceKind::LocalKinds, local_kinds_key(code));
        }
    }

    #[inline]
    fn metadata_string(&mut self, value: &'code str) {
        self.graph.record_keyed(
            OccurrenceKind::MetadataString,
            ObjectKey::String(value),
            true,
        );
    }

    #[inline]
    fn first_line(&mut self, _value: u32) {}

    #[inline]
    fn bytes(&mut self, value: &'code [u8]) {
        if value.is_empty() {
            self.graph
                .record_cached_value(OccurrenceKind::Bytes, CachedObject::EmptyBytes);
        } else {
            self.graph
                .record_keyed(OccurrenceKind::Bytes, ObjectKey::Bytes(value), false);
        }
    }

    #[inline]
    fn constant(&mut self, value: &'code Constant, _reference_mode: ReferenceMode) -> bool {
        match value {
            Constant::None | Constant::Bool(_) | Constant::Ellipsis => true,
            Constant::Tuple(_) | Constant::Code(_) => {
                unreachable!("tuple and code constants have dedicated traversal methods")
            }
            Constant::String(value) => {
                let interned =
                    should_intern(value) || self.explicitly_interned.contains(value.as_str());
                self.graph.record_keyed(
                    OccurrenceKind::Constant,
                    if interned {
                        ObjectKey::String(value)
                    } else {
                        ObjectKey::ConstantString(value)
                    },
                    interned,
                )
            }
            Constant::Bytes(value) if value.is_empty() => self
                .graph
                .record_cached_value(OccurrenceKind::Constant, CachedObject::EmptyBytes),
            _ => self
                .graph
                .record_keyed(OccurrenceKind::Constant, constant_key(value), false),
        }
    }

    #[inline]
    fn slice_member(&mut self, value: &'code Constant) {
        let address = std::ptr::from_ref(value).addr();
        self.graph
            .record_unique_value(OccurrenceKind::SliceMember, ObjectKey::SliceMember(address));
    }
}
