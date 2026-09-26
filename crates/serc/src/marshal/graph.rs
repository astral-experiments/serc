use ruff_index::IndexVec;

use std::collections::hash_map::Entry;

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use crate::code_object::{
    CodeFilename, CodeInterningSummary, CodeObject, CompilationInterningEvent, Constant,
    MetadataId, MetadataName as Name,
};
use crate::target::{is_preinterned_string_for, small_int_bounds_for};
use crate::{CompileError, InitialInternState, PythonVersion};

use super::key::{
    FrozenSetMembers, HashedObjectKey, ObjectKey, code_bytes_key, code_key, constant_key,
    constants_tuple_key, frozen_set_members_in_wire_order, frozen_set_members_with_properties,
    is_cached_character, local_kinds_key, locals_tuple_key, names_tuple_key, should_intern,
    strings_tuple_key,
};
use super::plan::{MarshalVisitor, ObjectId, ReferenceId, ReferenceMode, traverse_code};

#[derive(Debug)]
pub(super) struct Object {
    shared: bool,
    cached: bool,
    interned: bool,
    reference: Option<ReferenceId>,
}

const _: [(); 8] = [(); std::mem::size_of::<Object>()];

impl Object {
    pub(super) fn is_interned(&self) -> bool {
        self.interned
    }

    /// Resolves the next wire occurrence without looking up the same object a second time.
    #[inline]
    pub(super) fn reference_for_write(
        &mut self,
        mode: ReferenceMode,
        reference_count: &mut usize,
    ) -> ObjectReference {
        if mode == ReferenceMode::IfShared && !self.shared && !self.cached {
            return ObjectReference::Unshared;
        }
        if let Some(index) = self.reference {
            return ObjectReference::Existing(index);
        }
        let index = ReferenceId::from_usize(*reference_count);
        *reference_count += 1;
        self.reference = Some(index);
        ObjectReference::New
    }
}

#[derive(Clone, Copy)]
pub(super) enum ObjectReference {
    Unshared,
    New,
    Existing(ReferenceId),
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
pub(super) struct Occurrence {
    object: ObjectId,
    kind: OccurrenceKind,
}

const _: [(); 8] = [(); std::mem::size_of::<Occurrence>()];

#[derive(Debug)]
pub(super) struct OccurrenceCursor<T> {
    occurrences: T,
    next: usize,
}

impl<T: AsRef<[Occurrence]>> OccurrenceCursor<T> {
    #[inline]
    pub(super) fn consume(&mut self, expected: OccurrenceKind) -> ObjectId {
        let index = self.next;
        let occurrences = self.occurrences.as_ref();
        let Some(occurrence) = occurrences.get(index).copied() else {
            missing_occurrence(index, expected, occurrences.len());
        };
        if occurrence.kind != expected {
            mismatched_occurrence(index, expected, occurrence.kind);
        }
        self.next += 1;
        occurrence.object
    }

    pub(super) fn finish(self) {
        let occurrences = self.occurrences.as_ref();
        assert_eq!(
            self.next,
            occurrences.len(),
            "marshal writer consumed {} of {} recorded occurrences",
            self.next,
            occurrences.len()
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
const ESTIMATED_OCCURRENCE_BYTES: usize = 8;
const CACHED_BYTE_SLOTS: usize = 256;
const CACHED_CHARACTER_SLOTS: usize = 256;
fn cached_integer_slots(python_version: PythonVersion) -> (usize, usize) {
    let (minimum, maximum) = small_int_bounds_for(python_version);
    (
        usize::try_from(maximum + 1).expect("positive small-int cache size fits"),
        usize::try_from(maximum - minimum + 1).expect("signed small-int cache size fits"),
    )
}

fn cached_scalar_slots(python_version: PythonVersion) -> usize {
    let (positive, signed) = cached_integer_slots(python_version);
    positive + signed + CACHED_BYTE_SLOTS + CACHED_CHARACTER_SLOTS
}
const MIN_CACHED_SCALAR_CONSTANTS: usize = 32;
const MAX_DENSE_METADATA_SLOTS: usize = 1 << 20;

#[derive(Clone, Copy, Debug)]
enum CachedObject {
    EmptyBytes,
    EmptyTuple,
    Int(u16),
    SignedInt(u16),
    Byte(u8),
    Character(u8),
}

impl CachedObject {
    fn character(value: &str) -> Option<Self> {
        match value.as_bytes() {
            [character @ ..=0x7f] => Some(Self::Character(*character)),
            [lead @ 0xc2..=0xc3, continuation @ 0x80..=0xbf] => Some(Self::Character(
                ((*lead & 0x1f) << 6) | (*continuation & 0x3f),
            )),
            _ => None,
        }
    }

    fn scalar_index(self, python_version: PythonVersion) -> usize {
        let (positive, signed) = cached_integer_slots(python_version);
        match self {
            Self::EmptyBytes | Self::EmptyTuple => {
                unreachable!("empty cached object does not have a scalar index")
            }
            Self::Int(value) => usize::from(value),
            Self::SignedInt(value) => positive + usize::from(value),
            Self::Byte(value) => positive + signed + usize::from(value),
            Self::Character(value) => positive + signed + CACHED_BYTE_SLOTS + usize::from(value),
        }
    }

    #[cfg(test)]
    fn for_key(key: &ObjectKey<'_>, python_version: PythonVersion) -> Option<Self> {
        let (minimum, maximum) = small_int_bounds_for(python_version);
        match key {
            ObjectKey::Bytes([]) => Some(Self::EmptyBytes),
            ObjectKey::Bytes([value]) => Some(Self::Byte(*value)),
            ObjectKey::EmptyTuple => Some(Self::EmptyTuple),
            ObjectKey::Int(value) if *value <= maximum.cast_unsigned() => {
                Some(Self::Int(u16::try_from(*value).expect("cached int fits")))
            }
            ObjectKey::SignedInt(value) if (minimum..=maximum).contains(value) => {
                Some(Self::SignedInt(
                    u16::try_from(*value - minimum).expect("cached signed int index fits"),
                ))
            }
            ObjectKey::String(value) => Self::character(value),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ObjectGraph<'a> {
    python_version: PythonVersion,
    objects: FxHashMap<HashedObjectKey<'a>, ObjectId>,
    interned_strings: FxHashMap<&'a str, ObjectId>,
    metadata_aliased_constants: FxHashSet<usize>,
    metadata_slots: Box<[Option<ObjectId>]>,
    cached_empty_bytes: Option<ObjectId>,
    cached_empty_tuple: Option<ObjectId>,
    cached_scalar_ids: Option<Box<[Option<ObjectId>]>>,
    cache_scalars: bool,
    preallocated_interned_strings: bool,
    uncached_percent_constant: bool,
    fresh_empty_bytes_constant: bool,
    needs_reference_sorting: bool,
    legacy_qualified_name_aliases: FxHashSet<usize>,
    objects_by_id: IndexVec<ObjectId, Object>,
    occurrences: Vec<Occurrence>,
    frozen_set_orders: Vec<(usize, FrozenSetMembers<'a>)>,
    estimated_payload_bytes: usize,
    #[cfg(debug_assertions)]
    unique_objects: FxHashSet<HashedObjectKey<'a>>,
}

impl<'a> ObjectGraph<'a> {
    #[cfg(test)]
    pub(super) fn for_code(code: &'a CodeObject) -> Self {
        Self::for_code_with_options(code, PythonVersion::Py314, &InitialInternState::default())
    }

    pub(super) fn for_code_with_options(
        code: &'a CodeObject,
        python_version: PythonVersion,
        initial_intern_state: &InitialInternState,
    ) -> Self {
        let graph = Self::build_for_code(code, python_version, initial_intern_state, None);
        if graph.needs_reference_sorting {
            // Tuple members sort by complete marshal streams: their descendants' reference flags
            // depend on sharing across the entire code graph. Resolve that sharing before recording
            // occurrences; explicitly interned scalar strings also sort by their interned tag.
            // Python 3.12 also needs this pass for scalar members because it borrows their roots
            // while computing sort keys.
            Self::build_for_code(code, python_version, initial_intern_state, Some(&graph))
        } else {
            graph
        }
    }

    fn build_for_code(
        code: &'a CodeObject,
        python_version: PythonVersion,
        initial_intern_state: &InitialInternState,
        sort_graph: Option<&Self>,
    ) -> Self {
        let mut explicitly_interned = FxHashSet::default();
        let mut metadata_tuples = FxHashSet::default();
        let needs_metadata_alias_scan = !code.marshal_provenance.metadata_alias_scan_unnecessary;
        if needs_metadata_alias_scan {
            for values in &code.marshal_provenance.interned_metadata_tuples {
                metadata_tuples.insert(strings_tuple_key(values).into());
            }
        }
        let frozenset_recreation_need = collect_explicitly_interned(
            code,
            python_version,
            initial_intern_state,
            &mut explicitly_interned,
            &mut metadata_tuples,
            needs_metadata_alias_scan,
        );
        let mut recreated_frozensets = FxHashSet::default();
        collect_recreated_frozensets(
            code,
            frozenset_recreation_need,
            python_version,
            initial_intern_state,
            &mut recreated_frozensets,
        )
        .expect("compiled module must have resolved byte-relevant intern state");
        let explicitly_interned = (!explicitly_interned.is_empty()).then_some(&explicitly_interned);
        let recreated_frozensets =
            (!recreated_frozensets.is_empty()).then_some(&recreated_frozensets);
        let metadata_domain = usize::try_from(code.marshal_provenance.metadata_ids.domain_len)
            .ok()
            .filter(|len| *len <= MAX_DENSE_METADATA_SLOTS);
        let metadata_slots = metadata_domain
            .map(|len| vec![None; len].into_boxed_slice())
            .unwrap_or_default();
        // The compiler's metadata domain spans every nested code object. Reserving that map once
        // avoids repeated growth from recounting the same local and metadata spellings per code.
        // A sorting replay already knows the exact number of strings its first graph discovered.
        let interned_capacity = sort_graph
            .map(|graph| graph.interned_strings.len())
            .or_else(|| {
                code.marshal_provenance
                    .metadata_tuple_properties
                    .as_ref()
                    .and(metadata_domain)
            });
        let mut interned_strings = FxHashMap::default();
        if let Some(capacity) = interned_capacity {
            interned_strings.reserve(capacity);
        }
        let mut graph = Self {
            python_version,
            legacy_qualified_name_aliases: if python_version == PythonVersion::Py312 {
                legacy_qualified_name_aliases(code)
            } else {
                FxHashSet::default()
            },
            metadata_aliased_constants: collect_metadata_aliased_constants(code, &metadata_tuples),
            cache_scalars: code.constants.len() >= MIN_CACHED_SCALAR_CONSTANTS,
            preallocated_interned_strings: interned_capacity.is_some(),
            interned_strings,
            uncached_percent_constant: code.marshal_provenance.uncached_percent_constant,
            fresh_empty_bytes_constant: code.marshal_provenance.fresh_empty_bytes_constant,
            metadata_slots,
            ..Self::default()
        };
        traverse_code(
            code,
            &mut GraphVisitor {
                graph: &mut graph,
                explicitly_interned,
                recreated_frozensets,
                sort_graph,
                root_constants: true,
            },
        );
        graph
    }

    fn sort_frozen_set_members(&self, values: &'a [Constant]) -> FrozenSetMembers<'a> {
        if self.needs_reference_sorting {
            frozen_set_members_with_properties(values, self.python_version, &|value| {
                self.sort_key_properties(value)
            })
        } else {
            frozen_set_members_in_wire_order(values)
        }
    }

    pub(super) fn frozen_set_members(
        &self,
        values: &'a [Constant],
        index: usize,
    ) -> FrozenSetMembers<'a> {
        let (address, members) = &self.frozen_set_orders[index];
        assert_eq!(
            *address,
            values.as_ptr().addr(),
            "marshal writer requested a different frozen set than the graph recorded"
        );
        debug_assert!(
            members
                .iter()
                .zip(self.sort_frozen_set_members(values))
                .all(|(recorded, recalculated)| std::ptr::eq(*recorded, recalculated)),
            "frozen set order changed after recording the final graph"
        );
        members.clone()
    }

    pub(super) fn frozen_set_order_count(&self) -> usize {
        self.frozen_set_orders.len()
    }

    fn sort_key_properties(&self, value: &'a Constant) -> (bool, bool) {
        if let Constant::String(value) = value {
            if self
                .metadata_aliased_constants
                .contains(&std::ptr::from_ref(value).addr())
            {
                return (true, true);
            }
            if let Some(id) = self.objects.get(&ObjectKey::ConstantString(value).into()) {
                return (self.is_shared_object(*id), false);
            }
            return (
                true,
                should_intern(value) || self.interned_strings.contains_key(value.as_str()),
            );
        }
        let key = if self.fresh_empty_bytes_constant
            && matches!(value, Constant::Bytes(value) if value.is_empty())
        {
            ObjectKey::FreshEmptyBytes
        } else {
            constant_key(value)
        };
        let shared = Self::is_cached(&key, self.python_version)
            || self
                .objects
                .get(&key.into())
                .is_some_and(|id| self.is_shared_object(*id));
        (shared, false)
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
        let interned_strings = code
            .names
            .len()
            .saturating_add(code.locals.len())
            .saturating_add(3);
        self.objects
            .reserve(estimated_descendants.saturating_sub(interned_strings));
        if !self.preallocated_interned_strings {
            self.interned_strings.reserve(interned_strings);
        }
        self.objects_by_id
            .raw
            .reserve(estimated_descendants.saturating_add(1));
        self.occurrences
            .reserve(estimated_descendants.saturating_add(1));
        self.estimated_payload_bytes = self
            .estimated_payload_bytes
            .saturating_add(code.bytecode.len())
            .saturating_add(code.local_kinds.len());
    }

    fn estimate_long_payload(&mut self, length: usize) {
        self.estimate_payload(length.saturating_sub(ESTIMATED_OCCURRENCE_BYTES));
    }

    fn estimate_payload(&mut self, length: usize) {
        self.estimated_payload_bytes = self.estimated_payload_bytes.saturating_add(length);
    }

    fn record_keyed(&mut self, kind: OccurrenceKind, key: ObjectKey<'a>) -> bool {
        self.record_hashed(kind, key.into())
    }

    fn record_hashed(&mut self, kind: OccurrenceKind, key: HashedObjectKey<'a>) -> bool {
        let (id, first) = self.record_object(key);
        self.occurrences.push(Occurrence { object: id, kind });
        first
    }

    fn record_interned_string(&mut self, kind: OccurrenceKind, value: &'a str) -> bool {
        let (id, first) = self.interned_string(value);
        self.occurrences.push(Occurrence { object: id, kind });
        first
    }

    fn record_metadata_string(
        &mut self,
        kind: OccurrenceKind,
        value: &'a str,
        metadata_id: Option<MetadataId>,
    ) -> bool {
        let slot_index = metadata_id.map(MetadataId::index);
        if let Some(id) = slot_index
            .and_then(|index| self.metadata_slots.get(index).copied())
            .flatten()
        {
            debug_assert_eq!(
                self.interned_strings.get(value),
                Some(&id),
                "metadata ID must identify one string"
            );
            self.objects_by_id[id].shared = true;
            self.occurrences.push(Occurrence { object: id, kind });
            return false;
        }

        // Seed the ordinary value table even for hinted strings: an equal interned constant can
        // be traversed before or after this metadata and must retain the same object identity.
        let (id, first) = self.interned_string(value);
        if let Some(slot) = slot_index.and_then(|index| self.metadata_slots.get_mut(index)) {
            *slot = Some(id);
        }
        self.occurrences.push(Occurrence { object: id, kind });
        first
    }

    fn record_surrogate_filename(&mut self, value: &'a [u8]) -> bool {
        let (id, first) = self.record_object(ObjectKey::SurrogateFilename(value).into());
        if self.python_version >= PythonVersion::Py313 {
            let object = &mut self.objects_by_id[id];
            object.cached = true;
            object.interned = true;
        }
        self.occurrences.push(Occurrence {
            object: id,
            kind: OccurrenceKind::MetadataString,
        });
        first
    }

    fn interned_string(&mut self, value: &'a str) -> (ObjectId, bool) {
        match self.interned_strings.entry(value) {
            Entry::Occupied(entry) => {
                let id = *entry.get();
                self.objects_by_id[id].shared = true;
                (id, false)
            }
            Entry::Vacant(entry) => {
                let next_id = self.objects_by_id.next_index();
                entry.insert(next_id);
                self.objects_by_id.push(Object {
                    shared: false,
                    cached: true,
                    interned: true,
                    reference: None,
                });
                (next_id, true)
            }
        }
    }

    fn record_cached_value(&mut self, kind: OccurrenceKind, cached: CachedObject) -> bool {
        let (id, first) = self.record_cached(cached);
        self.occurrences.push(Occurrence { object: id, kind });
        first
    }

    /// Records bytes using the compilation's empty-byte identity and optional single-byte cache.
    /// The occurrence kind distinguishes structural bytes from constant visits without splitting
    /// their shared object identity.
    fn record_bytes(&mut self, kind: OccurrenceKind, value: &'a [u8]) -> bool {
        match value {
            [] if self.fresh_empty_bytes_constant => {
                self.record_keyed(kind, ObjectKey::FreshEmptyBytes)
            }
            [] => self.record_cached_value(kind, CachedObject::EmptyBytes),
            [value] if self.cache_scalars => {
                self.record_cached_value(kind, CachedObject::Byte(*value))
            }
            _ => self.record_keyed(kind, ObjectKey::Bytes(value)),
        }
    }

    fn record_unique_value(&mut self, kind: OccurrenceKind, key: ObjectKey<'a>) -> bool {
        let id = self.record_unique(key);
        self.occurrences.push(Occurrence { object: id, kind });
        true
    }

    #[cfg(test)]
    pub(super) fn record(&mut self, key: ObjectKey<'a>) -> bool {
        if let Some(cached) = CachedObject::for_key(&key, self.python_version) {
            self.record_cached(cached).1
        } else {
            self.record_object(key.into()).1
        }
    }

    /// Records an identity-bearing object that is structurally owned by a unique parent.
    fn record_unique(&mut self, _key: ObjectKey<'a>) -> ObjectId {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.unique_objects.insert(_key.into()),
            "identity-bearing marshal object was recorded twice"
        );

        let id = self.objects_by_id.next_index();
        self.objects_by_id.push(Object {
            shared: false,
            cached: false,
            interned: false,
            reference: None,
        });
        id
    }

    fn record_cached(&mut self, cached: CachedObject) -> (ObjectId, bool) {
        let empty_slot = match cached {
            CachedObject::EmptyBytes => Some(&mut self.cached_empty_bytes),
            CachedObject::EmptyTuple => Some(&mut self.cached_empty_tuple),
            _ => None,
        };
        if let Some(slot) = empty_slot {
            if let Some(id) = *slot {
                self.objects_by_id[id].shared = true;
                return (id, false);
            }
            let id = self.objects_by_id.next_index();
            self.objects_by_id.push(Object {
                shared: false,
                cached: true,
                interned: false,
                reference: None,
            });
            *slot = Some(id);
            return (id, true);
        }

        let slot = &mut self.cached_scalar_ids.get_or_insert_with(|| {
            vec![None; cached_scalar_slots(self.python_version)].into_boxed_slice()
        })[cached.scalar_index(self.python_version)];
        if let Some(id) = *slot {
            self.objects_by_id[id].shared = true;
            return (id, false);
        }

        let id = self.objects_by_id.next_index();
        self.objects_by_id.push(Object {
            shared: false,
            cached: true,
            interned: matches!(cached, CachedObject::Character(_)),
            reference: None,
        });
        *slot = Some(id);
        (id, true)
    }

    fn record_object(&mut self, key: HashedObjectKey<'a>) -> (ObjectId, bool) {
        match self.objects.entry(key) {
            Entry::Occupied(entry) => {
                let id = *entry.get();
                // Cache eligibility was fixed by this same stored key when first inserted.
                self.objects_by_id[id].shared = true;
                (id, false)
            }
            Entry::Vacant(entry) => {
                let next_id = self.objects_by_id.next_index();
                let cached = Self::is_cached(entry.key().key(), self.python_version);
                entry.insert(next_id);
                self.objects_by_id.push(Object {
                    shared: false,
                    cached,
                    interned: false,
                    reference: None,
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
                .get(&HashedObjectKey::from(*key))
                .is_some_and(|id| self.objects_by_id[*id].shared)
            || matches!(
                key,
                ObjectKey::String(value)
                    if self
                        .interned_strings
                        .get(*value)
                        .is_some_and(|id| self.is_shared_object(*id))
            )
        {
            return true;
        }
        Self::is_cached(key, self.python_version)
    }

    fn is_cached(key: &ObjectKey<'_>, python_version: PythonVersion) -> bool {
        let (minimum, maximum) = small_int_bounds_for(python_version);
        match key {
            ObjectKey::Int(value) => *value <= maximum.cast_unsigned(),
            ObjectKey::SignedInt(value) => (minimum..=maximum).contains(value),
            ObjectKey::Bytes(value) => value.len() <= 1,
            ObjectKey::String(value) => is_cached_character(value),
            ObjectKey::EmptyTuple => true,
            _ => false,
        }
    }

    pub(super) fn is_shared_object(&self, id: ObjectId) -> bool {
        let object = &self.objects_by_id[id];
        object.shared || object.cached
    }

    pub(super) fn python_version(&self) -> PythonVersion {
        self.python_version
    }

    #[cfg(test)]
    pub(super) fn object_count(&self) -> usize {
        self.objects_by_id.len()
    }

    pub(super) fn object_for_writing(&mut self, id: ObjectId) -> &mut Object {
        &mut self.objects_by_id[id]
    }

    /// Estimates space for the writer from the same traversal that records object identities.
    /// Structural bytes and long first-seen values supplement the allowance for each occurrence.
    pub(super) fn estimated_wire_size(&self) -> usize {
        self.occurrences
            .len()
            .saturating_mul(ESTIMATED_OCCURRENCE_BYTES)
            .saturating_add(self.estimated_payload_bytes)
    }

    #[cfg(test)]
    pub(super) fn keyed_object_count(&self) -> usize {
        self.objects.len()
            + self.interned_strings.len()
            + usize::from(self.cached_empty_bytes.is_some())
            + usize::from(self.cached_empty_tuple.is_some())
            + self
                .cached_scalar_ids
                .as_deref()
                .map_or(0, |slots| slots.iter().filter(|id| id.is_some()).count())
    }

    #[cfg(test)]
    pub(super) fn hashed_object_count(&self) -> usize {
        self.objects.len()
    }

    #[cfg(test)]
    pub(super) fn occurrence_cursor(&self) -> OccurrenceCursor<&[Occurrence]> {
        OccurrenceCursor {
            occurrences: &self.occurrences,
            next: 0,
        }
    }

    /// Separates the read-only occurrence stream before the writer mutates object references.
    pub(super) fn take_occurrences(&mut self) -> OccurrenceCursor<Vec<Occurrence>> {
        OccurrenceCursor {
            occurrences: std::mem::take(&mut self.occurrences),
            next: 0,
        }
    }

    #[cfg(test)]
    pub(super) fn is_interned_object(&self, id: ObjectId) -> bool {
        self.objects_by_id[id].interned
    }

    #[cfg(test)]
    pub(super) fn is_interned(&self, value: &str) -> bool {
        if CachedObject::character(value)
            .map(|cached| cached.scalar_index(self.python_version))
            .is_some_and(|index| {
                self.cached_scalar_ids
                    .as_deref()
                    .is_some_and(|slots| slots[index].is_some())
            })
        {
            return true;
        }
        self.interned_strings.contains_key(value)
    }
}

/// Finds constant tuple elements replaced by metadata-string interning. CPython's
/// constant cache can share an all-string tuple with `co_names` or `co_varnames` before
/// interning metadata, including tuples from a different nested code object. Only
/// those tuple slots change; an equal standalone non-ASCII constant stays distinct.
fn collect_metadata_aliased_constants<'a>(
    code: &'a CodeObject,
    keys: &FxHashSet<HashedObjectKey<'a>>,
) -> FxHashSet<usize> {
    fn constants<'a>(
        values: &'a [Constant],
        keys: &FxHashSet<HashedObjectKey<'a>>,
        interned: &mut FxHashSet<usize>,
    ) {
        if !values.is_empty()
            && values
                .iter()
                .all(|value| matches!(value, Constant::String(_)))
            && keys.contains(&constants_tuple_key(values).into())
        {
            for value in values {
                let Constant::String(value) = value else {
                    unreachable!("all-string tuple")
                };
                if !should_intern(value) {
                    interned.insert(std::ptr::from_ref(value).addr());
                }
            }
            return;
        }
        for value in values {
            constant(value, keys, interned);
        }
    }

    fn constant<'a>(
        value: &'a Constant,
        keys: &FxHashSet<HashedObjectKey<'a>>,
        interned: &mut FxHashSet<usize>,
    ) {
        match value {
            Constant::Tuple(values) => constants(values, keys, interned),
            Constant::FrozenSet(values) => {
                for value in values {
                    constant(value, keys, interned);
                }
            }
            Constant::Code(child) => constants(&child.constants, keys, interned),
            Constant::Slice { lower, upper, step } => {
                for value in [lower.as_ref(), upper.as_ref(), step.as_ref()] {
                    constant(value, keys, interned);
                }
            }
            _ => {}
        }
    }

    let mut interned = FxHashSet::default();
    if !keys.is_empty() {
        constants(&code.constants, keys, &mut interned);
    }
    interned
}

fn collect_metadata_tuple_keys<'a>(
    code: &'a CodeObject,
    keys: &mut FxHashSet<HashedObjectKey<'a>>,
) {
    if let Some(properties) = code.marshal_provenance.metadata_tuple_properties {
        if !properties.names.automatically_interned {
            keys.insert(HashedObjectKey::with_hash(
                names_tuple_key(&code.names),
                properties.names.hash,
            ));
        }
        if !properties.locals.automatically_interned {
            keys.insert(hashed_locals_tuple_key(code, properties.locals.hash));
        }
    } else {
        if code.names.iter().any(|value| !should_intern(value)) {
            keys.insert(names_tuple_key(&code.names).into());
        }
        if code.locals.iter().any(|value| !should_intern(value)) {
            keys.insert(locals_tuple_key(code).into());
        }
    }
}

fn hashed_locals_tuple_key(code: &CodeObject, hash: u64) -> HashedObjectKey<'_> {
    let key = locals_tuple_key(code);
    if matches!(&key, ObjectKey::AnnotationLocals(_)) {
        // Fresh annotation locals use the code object's eventual address, not their values.
        key.into()
    } else {
        HashedObjectKey::with_hash(key, hash)
    }
}

fn collect_explicitly_interned<'a>(
    code: &'a CodeObject,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
    values: &mut FxHashSet<&'a str>,
    metadata_tuples: &mut FxHashSet<HashedObjectKey<'a>>,
    needs_metadata_alias_scan: bool,
) -> FrozensetRecreationNeed {
    // Identifier objects can enter the compilation-wide constant cache before an equal
    // ordinary string. Preserve that recorded origin when sharing constant and metadata strings.
    values.extend(
        code.marshal_provenance
            .canonical_constant_strings
            .iter()
            .map(String::as_str),
    );
    let mut qualified_names = QualifiedNameConstants {
        track: python_version != PythonVersion::Py312,
        ..QualifiedNameConstants::default()
    };
    let frozenset_recreation_need = if !code.marshal_provenance.subtree_has_frozenset_constants {
        collect_interning_provenance_without_frozensets(
            code,
            values,
            metadata_tuples,
            needs_metadata_alias_scan,
            &mut qualified_names,
        );
        FrozensetRecreationNeed::None
    } else {
        collect_interning_provenance(
            code,
            python_version,
            values,
            metadata_tuples,
            needs_metadata_alias_scan,
            &mut qualified_names,
        )
    };
    if qualified_names.origins.is_empty() {
        return frozenset_recreation_need;
    }

    // A class body normally loads the same qualified-name object that becomes its interned code
    // metadata. If an equal ordinary constant reached the compilation-wide constant cache first,
    // the class constant instead reuses that plain object and remains distinct from the metadata.
    let mut ordinary_first = FxHashSet::default();
    if qualified_names.origins.values().any(Option::is_none) {
        // Compiler provenance includes earlier admissions from discarded code. Only spellings
        // without a recorded origin require reconstructing their surviving constant order.
        collect_qualified_name_origins(
            code,
            &qualified_names.constants.into_iter().collect(),
            &qualified_names.origins,
            &mut FxHashSet::default(),
            &mut ordinary_first,
        );
    }
    let additional_values: Vec<_> = qualified_names
        .origins
        .into_iter()
        .filter_map(|(value, origin)| {
            // An already recorded interned constant does not depend on initial membership.
            // Otherwise, interning metadata can replace its pointer with an equal canonical
            // object, leaving the class's fresh constant as a separate plain string.
            (!values.contains(value)
                && origin.unwrap_or_else(|| !ordinary_first.contains(value))
                && !initial_intern_state.contains_for(python_version, value))
            .then_some(value)
        })
        .collect();
    values.extend(additional_values);
    frozenset_recreation_need
}

/// Names already interned by their scalar value do not require qualified-name origin analysis.
fn qualified_name_constant_requiring_origin(code: &CodeObject) -> Option<(&Constant, &str)> {
    let index = code.marshal_provenance.qualified_name_constant_index?;
    let index = usize::try_from(index).expect("constant index always fits in usize");
    let constant = code.constants.get(index)?;
    let Constant::String(value) = constant else {
        return None;
    };
    (!should_intern(value)).then_some((constant, value))
}

fn collect_interning_provenance_without_frozensets<'a>(
    code: &'a CodeObject,
    values: &mut FxHashSet<&'a str>,
    metadata_tuples: &mut FxHashSet<HashedObjectKey<'a>>,
    needs_metadata_alias_scan: bool,
    qualified_names: &mut QualifiedNameConstants<'a>,
) {
    if needs_metadata_alias_scan {
        collect_metadata_tuple_keys(code, metadata_tuples);
    }
    values.extend(
        code.marshal_provenance
            .explicitly_interned_strings
            .iter()
            .map(String::as_str),
    );
    qualified_names.record(code);
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            collect_interning_provenance_without_frozensets(
                code,
                values,
                metadata_tuples,
                needs_metadata_alias_scan,
                qualified_names,
            );
        }
    }
}

fn collect_interning_provenance<'a>(
    code: &'a CodeObject,
    python_version: PythonVersion,
    values: &mut FxHashSet<&'a str>,
    metadata_tuples: &mut FxHashSet<HashedObjectKey<'a>>,
    needs_metadata_alias_scan: bool,
    qualified_names: &mut QualifiedNameConstants<'a>,
) -> FrozensetRecreationNeed {
    if needs_metadata_alias_scan {
        collect_metadata_tuple_keys(code, metadata_tuples);
    }
    values.extend(
        code.marshal_provenance
            .explicitly_interned_strings
            .iter()
            .map(String::as_str),
    );
    qualified_names.record(code);
    // Reuse the provenance walk to classify frozenset recreation for code trees that can contain
    // one (and for hand-built objects without a conservative compiler hint).
    let mut frozenset_recreation_need = FrozensetRecreationNeed::None;
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            let recreation_need = collect_interning_provenance(
                code,
                python_version,
                values,
                metadata_tuples,
                needs_metadata_alias_scan,
                qualified_names,
            );
            frozenset_recreation_need = frozenset_recreation_need.max(recreation_need);
        } else if frozenset_recreation_need != FrozensetRecreationNeed::IdentifierDependent {
            frozenset_recreation_need = frozenset_recreation_need
                .max(constant_frozenset_recreation_need(constant, python_version));
        }
    }
    frozenset_recreation_need
}

/// Identifies class qualified names that first entered the compilation's constant cache.
///
/// Python 3.12 keeps these objects uninterned. The first class can share its qualified name
/// with the cached constant, while an earlier equal literal or class leaves later metadata
/// distinct from the shared constant value.
fn legacy_qualified_name_aliases(code: &CodeObject) -> FxHashSet<usize> {
    fn collect<'a>(
        code: &'a CodeObject,
        owners: &mut FxHashMap<usize, usize>,
        values: &mut FxHashSet<&'a str>,
    ) {
        if let Some(index) = code.marshal_provenance.qualified_name_constant_index
            && let Some(constant @ Constant::String(value)) = code.constants.get(index as usize)
        {
            owners.insert(
                std::ptr::from_ref(constant).addr(),
                std::ptr::from_ref(code).addr(),
            );
            values.insert(value);
        }
        for constant in &code.constants {
            if let Constant::Code(code) = constant {
                collect(code, owners, values);
            }
        }
    }

    fn visit<'a>(
        constant: &'a Constant,
        owners: &FxHashMap<usize, usize>,
        values: &FxHashSet<&'a str>,
        seen: &mut FxHashSet<&'a str>,
        aliases: &mut FxHashSet<usize>,
    ) {
        match constant {
            Constant::String(value) if values.contains(value.as_str()) => {
                if seen.insert(value)
                    && let Some(owner) = owners.get(&std::ptr::from_ref(constant).addr())
                {
                    aliases.insert(*owner);
                }
            }
            Constant::Tuple(members) | Constant::FrozenSet(members) => {
                for value in members {
                    visit(value, owners, values, seen, aliases);
                }
            }
            Constant::Code(code) => {
                for value in &code.constants {
                    visit(value, owners, values, seen, aliases);
                }
            }
            Constant::Slice { lower, upper, step } => {
                for value in [lower.as_ref(), upper.as_ref(), step.as_ref()] {
                    visit(value, owners, values, seen, aliases);
                }
            }
            _ => {}
        }
    }

    let mut owners = FxHashMap::default();
    let mut values = FxHashSet::default();
    collect(code, &mut owners, &mut values);
    let mut aliases = FxHashSet::default();
    if !values.is_empty() {
        let mut seen = FxHashSet::default();
        for constant in &code.constants {
            visit(constant, &owners, &values, &mut seen, &mut aliases);
        }
    }
    aliases
}

#[derive(Default)]
struct QualifiedNameConstants<'a> {
    track: bool,
    constants: SmallVec<[usize; 4]>,
    origins: FxHashMap<&'a str, Option<bool>>,
}

impl<'a> QualifiedNameConstants<'a> {
    fn record(&mut self, code: &'a CodeObject) {
        if !self.track {
            return;
        }
        let Some((constant, value)) = qualified_name_constant_requiring_origin(code) else {
            return;
        };
        self.constants.push(std::ptr::from_ref(constant).addr());
        let origin = self.origins.entry(value).or_default();
        if let Some(canonical) = code.marshal_provenance.qualified_name_is_canonical {
            *origin = Some(canonical);
        }
    }
}

fn collect_qualified_name_origins<'a>(
    code: &'a CodeObject,
    qualified_name_constants: &FxHashSet<usize>,
    qualified_name_values: &FxHashMap<&'a str, Option<bool>>,
    seen: &mut FxHashSet<&'a str>,
    ordinary_first: &mut FxHashSet<&'a str>,
) {
    for constant in &code.constants {
        collect_qualified_name_origin_from_constant(
            constant,
            qualified_name_constants,
            qualified_name_values,
            seen,
            ordinary_first,
        );
    }
}

fn collect_qualified_name_origin_from_constant<'a>(
    constant: &'a Constant,
    qualified_name_constants: &FxHashSet<usize>,
    qualified_name_values: &FxHashMap<&'a str, Option<bool>>,
    seen: &mut FxHashSet<&'a str>,
    ordinary_first: &mut FxHashSet<&'a str>,
) {
    match constant {
        Constant::String(value) if qualified_name_values.contains_key(value.as_str()) => {
            let value = value.as_str();
            if seen.insert(value)
                && !qualified_name_constants.contains(&std::ptr::from_ref(constant).addr())
            {
                ordinary_first.insert(value);
            }
        }
        Constant::Tuple(constants) | Constant::FrozenSet(constants) => {
            for constant in constants {
                collect_qualified_name_origin_from_constant(
                    constant,
                    qualified_name_constants,
                    qualified_name_values,
                    seen,
                    ordinary_first,
                );
            }
        }
        Constant::Slice { lower, upper, step } => {
            for constant in [lower.as_ref(), upper.as_ref(), step.as_ref()] {
                collect_qualified_name_origin_from_constant(
                    constant,
                    qualified_name_constants,
                    qualified_name_values,
                    seen,
                    ordinary_first,
                );
            }
        }
        Constant::Code(code) => collect_qualified_name_origins(
            code,
            qualified_name_constants,
            qualified_name_values,
            seen,
            ordinary_first,
        ),
        Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::NonReflexive(_)
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::String(_)
        | Constant::SurrogateString(_)
        | Constant::Bytes(_) => {}
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
enum FrozensetRecreationNeed {
    None,
    PreinternedOnly,
    IdentifierDependent,
}

impl FrozensetRecreationNeed {
    /// No later constant can increase the classification once an identifier is relevant.
    fn maximum(needs: impl IntoIterator<Item = Self>) -> Self {
        let mut maximum = Self::None;
        for need in needs {
            if need == Self::IdentifierDependent {
                return need;
            }
            maximum = maximum.max(need);
        }
        maximum
    }
}

#[derive(Clone, Copy)]
enum Replacement<'code> {
    Never,
    Always,
    Unknown(&'code str),
}

impl Replacement<'_> {
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Always, _) | (_, Self::Always) => Self::Always,
            (Self::Never, value) | (value, Self::Never) => value,
            (Self::Unknown(left), Self::Unknown(right)) => Self::Unknown(left.min(right)),
        }
    }
}

struct PartialInternDependencies<'code> {
    strings: FxHashSet<&'code str>,
    sensitive_frozensets: FxHashSet<usize>,
}

/// Compilation-local additions layered over the immutable initial state. Keeping
/// these separate avoids copying a complete bootstrap profile for every module.
struct InternedStrings<'code, 'state> {
    initial: &'state InitialInternState,
    python_version: PythonVersion,
    added: FxHashSet<&'code str>,
    possibly_added: FxHashSet<&'code str>,
    partial: Option<PartialInternDependencies<'code>>,
    uncertain_origins: FxHashSet<&'code str>,
    unresolved_frozenset: Option<&'code str>,
}

impl<'code, 'state> InternedStrings<'code, 'state> {
    fn new(
        initial: &'state InitialInternState,
        python_version: PythonVersion,
        partial: Option<PartialInternDependencies<'code>>,
    ) -> Self {
        Self {
            initial,
            python_version,
            added: FxHashSet::default(),
            possibly_added: FxHashSet::default(),
            partial,
            uncertain_origins: FxHashSet::default(),
            unresolved_frozenset: None,
        }
    }

    fn contains(&self, value: &str) -> Option<bool> {
        if self.added.contains(value) {
            Some(true)
        } else {
            match self.initial.membership_for(self.python_version, value) {
                Some(false) if self.possibly_added.contains(value) => None,
                membership => membership,
            }
        }
    }

    fn tracks_origin(&self, value: &str) -> bool {
        self.partial
            .as_ref()
            .is_none_or(|partial| partial.strings.contains(value))
    }

    fn record_unresolved_frozenset(&mut self, constant: &Constant, value: &'code str) {
        if self.partial.as_ref().is_some_and(|partial| {
            partial
                .sensitive_frozensets
                .contains(&std::ptr::from_ref(constant).addr())
        }) {
            self.unresolved_frozenset = Some(
                self.unresolved_frozenset
                    .map_or(value, |previous| previous.min(value)),
            );
        }
    }

    fn insert(&mut self, value: &'code str) {
        self.added.insert(value);
    }

    fn extend(&mut self, values: impl IntoIterator<Item = &'code str>) {
        self.added.extend(values);
    }
}

/// Reject unresolved canonical origins before exposing an infallibly serializable module.
pub(crate) fn validate_initial_intern_state(
    code: &CodeObject,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
) -> Result<(), CompileError> {
    if !initial_intern_state.has_unknown_entries() {
        return Ok(());
    }
    if python_version != PythonVersion::Py312 {
        validate_qualified_name_intern_state(code, python_version, initial_intern_state)?;
    }
    collect_recreated_frozensets(
        code,
        frozenset_recreation_need(code, python_version),
        python_version,
        initial_intern_state,
        &mut FxHashSet::default(),
    )
}

fn validate_qualified_name_intern_state(
    code: &CodeObject,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
) -> Result<(), CompileError> {
    if code.marshal_provenance.qualified_name_is_canonical == Some(true)
        && !should_intern(&code.qualified_name)
        && initial_intern_state
            .membership_for(python_version, &code.qualified_name)
            .is_none()
    {
        return Err(CompileError::UnresolvedInternState {
            value: code.qualified_name.clone(),
        });
    }
    for constant in &code.constants {
        if let Constant::Code(child) = constant {
            validate_qualified_name_intern_state(child, python_version, initial_intern_state)?;
        }
    }
    Ok(())
}

/// Initial canonical identity matters for direct frozen-set string members unless
/// reconstruction preserves a container with one physical owner slot. Dependencies
/// remain global: an earlier independent set can affect a later set's origins.
fn frozenset_intern_dependencies(
    code: &CodeObject,
    python_version: PythonVersion,
) -> PartialInternDependencies<'_> {
    #[derive(PartialEq, Eq)]
    enum OwnerSlot<'a> {
        Tuple(HashedObjectKey<'a>, usize),
        Occurrence(usize),
    }

    struct Owners<'a> {
        first: OwnerSlot<'a>,
        multiple: bool,
        inside_frozenset: bool,
    }

    fn safe_tuple_member(constant: &Constant) -> bool {
        match constant {
            Constant::None
            | Constant::Bool(_)
            | Constant::Ellipsis
            | Constant::Int(_)
            | Constant::SignedInt(_)
            | Constant::BigInt { .. }
            | Constant::String(_)
            | Constant::SurrogateString(_)
            | Constant::Bytes(_) => true,
            Constant::Tuple(members) | Constant::FrozenSet(members) => {
                members.iter().all(safe_tuple_member)
            }
            Constant::NonReflexive(_)
            | Constant::Float(_)
            | Constant::Complex { .. }
            | Constant::Slice { .. }
            | Constant::Code(_) => false,
        }
    }

    fn count_tuple<'a>(
        constants: &'a [Constant],
        inside_frozenset: bool,
        share_tuple_slots: bool,
        owners: &mut FxHashMap<HashedObjectKey<'a>, Owners<'a>>,
    ) {
        // Ordinary tuples and co_consts use the same compiler cache. Restrict
        // value-based owner merging to keys without code equality, slices, or
        // floating-point ambiguities, including anywhere in the parent value.
        let key: Option<HashedObjectKey<'a>> = (share_tuple_slots
            && constants.iter().all(safe_tuple_member))
        .then(|| constants_tuple_key(constants).into());
        for (index, constant) in constants.iter().enumerate() {
            let owner = key.as_ref().map_or_else(
                || OwnerSlot::Occurrence(std::ptr::from_ref(constant).addr()),
                |key| OwnerSlot::Tuple(*key, index),
            );
            count(constant, owner, inside_frozenset, share_tuple_slots, owners);
        }
    }

    fn count<'a>(
        constant: &'a Constant,
        owner: OwnerSlot<'a>,
        inside_frozenset: bool,
        share_tuple_slots: bool,
        owners: &mut FxHashMap<HashedObjectKey<'a>, Owners<'a>>,
    ) {
        match constant {
            Constant::FrozenSet(members) => {
                match owners.entry(constant_key(constant).into()) {
                    Entry::Occupied(mut entry) => {
                        let prior = entry.get_mut();
                        prior.multiple |= prior.first != owner;
                        prior.inside_frozenset |= inside_frozenset;
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(Owners {
                            first: owner,
                            multiple: false,
                            inside_frozenset,
                        });
                    }
                }
                for member in members {
                    count(
                        member,
                        OwnerSlot::Occurrence(std::ptr::from_ref(member).addr()),
                        true,
                        share_tuple_slots,
                        owners,
                    );
                }
            }
            Constant::Tuple(constants) => {
                count_tuple(constants, inside_frozenset, share_tuple_slots, owners);
            }
            Constant::Code(code) => {
                count_tuple(&code.constants, inside_frozenset, share_tuple_slots, owners);
            }
            Constant::Slice { lower, upper, step } => {
                // Slice children do not participate in recursive tuple merging.
                for member in [lower.as_ref(), upper.as_ref(), step.as_ref()] {
                    count(
                        member,
                        OwnerSlot::Occurrence(std::ptr::from_ref(member).addr()),
                        inside_frozenset,
                        false,
                        owners,
                    );
                }
            }
            _ => {}
        }
    }

    fn collect<'a>(
        constant: &'a Constant,
        owners: &FxHashMap<HashedObjectKey<'a>, Owners<'a>>,
        dependencies: &mut PartialInternDependencies<'a>,
    ) {
        match constant {
            Constant::FrozenSet(members) => {
                // One physical tuple slot can appear through many equal raw
                // parents. Replacing its set preserves an unshared identity.
                // Unsafe parents still admit the original raw-singleton proof.
                // Exclude all frozen ancestors and floating scalar members whose
                // NaN-like sort-key ties could make reconstruction order matter.
                let independent = owners
                    .get(&constant_key(constant).into())
                    .is_some_and(|owners| !owners.multiple && !owners.inside_frozenset)
                    && members.iter().all(|member| {
                        matches!(
                            member,
                            Constant::None
                                | Constant::Bool(_)
                                | Constant::Ellipsis
                                | Constant::Int(_)
                                | Constant::SignedInt(_)
                                | Constant::BigInt { .. }
                                | Constant::String(_)
                                | Constant::SurrogateString(_)
                                | Constant::Bytes(_)
                        )
                    });
                if !independent {
                    dependencies
                        .sensitive_frozensets
                        .insert(std::ptr::from_ref(constant).addr());
                }
                for member in members {
                    if !independent
                        && let Constant::String(value) = member
                        && should_intern(value)
                        && !value.is_empty()
                        && !is_cached_character(value)
                    {
                        dependencies.strings.insert(value);
                    }
                    collect(member, owners, dependencies);
                }
            }
            Constant::Tuple(constants) => {
                for constant in constants {
                    collect(constant, owners, dependencies);
                }
            }
            Constant::Code(code) => {
                for constant in &code.constants {
                    collect(constant, owners, dependencies);
                }
            }
            _ => {}
        }
    }

    let mut owners = FxHashMap::default();
    // The tuple-cache and in-place slot mutation proof is pinned to Python 3.13.
    count_tuple(
        &code.constants,
        false,
        python_version == PythonVersion::Py313,
        &mut owners,
    );
    let mut dependencies = PartialInternDependencies {
        strings: FxHashSet::default(),
        sensitive_frozensets: FxHashSet::default(),
    };
    for constant in &code.constants {
        collect(constant, &owners, &mut dependencies);
    }
    dependencies
}

fn collect_recreated_frozensets(
    code: &CodeObject,
    need: FrozensetRecreationNeed,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
    values: &mut FxHashSet<usize>,
) -> Result<(), CompileError> {
    // A complete state may omit built-in entries whose later identifier interning
    // still matters. Keep the existing shortcuts for supplemental states.
    let need =
        if !initial_intern_state.uses_startup_state() && need != FrozensetRecreationNeed::None {
            FrozensetRecreationNeed::IdentifierDependent
        } else {
            need
        };
    match need {
        FrozensetRecreationNeed::None => return Ok(()),
        FrozensetRecreationNeed::PreinternedOnly
            if code.marshal_provenance.interning_events.is_empty()
                && !has_explicit_interns(code)
                && code
                    .marshal_provenance
                    .canonical_constant_strings
                    .is_empty() =>
        {
            collect_preinterned_recreated_frozensets(
                code,
                python_version,
                initial_intern_state,
                values,
            );
            return Ok(());
        }
        FrozensetRecreationNeed::PreinternedOnly | FrozensetRecreationNeed::IdentifierDependent => {
        }
    }

    if need == FrozensetRecreationNeed::IdentifierDependent
        && initial_intern_state.uses_startup_state()
        && code.marshal_provenance.interning_events.is_empty()
        && !has_identifier_or_explicit_dependent_frozenset(code, python_version)
        && code
            .marshal_provenance
            .canonical_constant_strings
            .is_empty()
    {
        collect_preinterned_recreated_frozensets(
            code,
            python_version,
            initial_intern_state,
            values,
        );
        return Ok(());
    }

    let partial = initial_intern_state
        .has_unknown_entries()
        .then(|| frozenset_intern_dependencies(code, python_version));
    let mut interned_strings = InternedStrings::new(initial_intern_state, python_version, partial);
    let mut interned_constant_strings = code
        .marshal_provenance
        .canonical_constant_strings
        .iter()
        .map(String::as_str)
        .collect();
    if need == FrozensetRecreationNeed::IdentifierDependent {
        collect_identifier_strings(
            code,
            &code.marshal_provenance.generated_mangled_strings,
            &mut interned_strings,
        );
    }
    if code.marshal_provenance.interning_events.is_empty() {
        collect_code_interning(
            code,
            values,
            &code.marshal_provenance.parser_interned_strings,
            &mut interned_strings,
            &mut interned_constant_strings,
        );
    } else {
        collect_compilation_interning(
            code,
            values,
            &mut interned_strings,
            &mut interned_constant_strings,
        );
    }
    if let Some(value) = interned_strings.unresolved_frozenset {
        Err(CompileError::UnresolvedInternState {
            value: value.to_string(),
        })
    } else {
        Ok(())
    }
}

fn has_explicit_interns(code: &CodeObject) -> bool {
    !code
        .marshal_provenance
        .explicitly_interned_strings
        .is_empty()
        || code.constants.iter().any(|constant| match constant {
            Constant::Code(code) => has_explicit_interns(code),
            _ => false,
        })
}

fn frozenset_recreation_need(
    code: &CodeObject,
    python_version: PythonVersion,
) -> FrozensetRecreationNeed {
    FrozensetRecreationNeed::maximum(
        code.constants
            .iter()
            .map(|constant| constant_frozenset_recreation_need(constant, python_version)),
    )
}

fn constant_frozenset_recreation_need(
    constant: &Constant,
    python_version: PythonVersion,
) -> FrozensetRecreationNeed {
    match constant {
        Constant::FrozenSet(members) => FrozensetRecreationNeed::maximum(
            members
                .iter()
                .map(|constant| constant_string_recreation_need(constant, python_version)),
        ),
        Constant::Tuple(constants) => FrozensetRecreationNeed::maximum(
            constants
                .iter()
                .map(|constant| constant_frozenset_recreation_need(constant, python_version)),
        ),
        Constant::Slice { lower, upper, step } => FrozensetRecreationNeed::maximum(
            [lower.as_ref(), upper.as_ref(), step.as_ref()]
                .into_iter()
                .map(|constant| constant_frozenset_recreation_need(constant, python_version)),
        ),
        Constant::Code(code) => frozenset_recreation_need(code, python_version),
        Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::NonReflexive(_)
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::String(_)
        | Constant::SurrogateString(_)
        | Constant::Bytes(_) => FrozensetRecreationNeed::None,
    }
}

// The isolated baseline is a conservative upper bound for supplemental profiles:
// None covers only ineligible or cached strings. Newly preinterned members remain
// IdentifierDependent, and both fast fallbacks and chronology consult the profile.
fn constant_string_recreation_need(
    constant: &Constant,
    python_version: PythonVersion,
) -> FrozensetRecreationNeed {
    match constant {
        Constant::String(value) => {
            if !should_intern(value) || value.is_empty() || is_cached_character(value) {
                FrozensetRecreationNeed::None
            } else if is_preinterned_string_for(python_version, value) {
                FrozensetRecreationNeed::PreinternedOnly
            } else {
                FrozensetRecreationNeed::IdentifierDependent
            }
        }
        Constant::Tuple(constants) | Constant::FrozenSet(constants) => {
            FrozensetRecreationNeed::maximum(
                constants
                    .iter()
                    .map(|constant| constant_string_recreation_need(constant, python_version)),
            )
        }
        Constant::Slice { lower, upper, step } => FrozensetRecreationNeed::maximum(
            [lower.as_ref(), upper.as_ref(), step.as_ref()]
                .into_iter()
                .map(|constant| constant_string_recreation_need(constant, python_version)),
        ),
        Constant::Code(_)
        | Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::NonReflexive(_)
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::SurrogateString(_)
        | Constant::Bytes(_) => FrozensetRecreationNeed::None,
    }
}

fn collect_preinterned_recreated_frozensets(
    code: &CodeObject,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
    values: &mut FxHashSet<usize>,
) {
    for constant in &code.constants {
        if matches!(
            constant,
            Constant::Tuple(_) | Constant::FrozenSet(_) | Constant::Code(_)
        ) {
            collect_preinterned_recreated_constant(
                constant,
                python_version,
                initial_intern_state,
                values,
            );
        }
    }
}

fn has_identifier_or_explicit_dependent_frozenset(
    code: &CodeObject,
    python_version: PythonVersion,
) -> bool {
    constants_have_identifier_or_explicit_dependent_frozenset(&code.constants, code, python_version)
}

fn constants_have_identifier_or_explicit_dependent_frozenset<'a>(
    constants: impl IntoIterator<Item = &'a Constant>,
    root: &CodeObject,
    python_version: PythonVersion,
) -> bool {
    constants.into_iter().any(|constant| {
        matches!(
            constant,
            Constant::FrozenSet(_)
                | Constant::Tuple(_)
                | Constant::Slice { .. }
                | Constant::Code(_)
        ) && constant_has_identifier_or_explicit_dependent_frozenset(constant, root, python_version)
    })
}

fn constant_has_identifier_or_explicit_dependent_frozenset(
    constant: &Constant,
    root: &CodeObject,
    python_version: PythonVersion,
) -> bool {
    match constant {
        Constant::FrozenSet(members) => {
            constants_contain_identifier_or_explicit_dependent_string(members, root, python_version)
        }
        Constant::Tuple(constants) => constants_have_identifier_or_explicit_dependent_frozenset(
            constants,
            root,
            python_version,
        ),
        Constant::Slice { lower, upper, step } => {
            constants_have_identifier_or_explicit_dependent_frozenset(
                [lower.as_ref(), upper.as_ref(), step.as_ref()],
                root,
                python_version,
            )
        }
        Constant::Code(code) => constants_have_identifier_or_explicit_dependent_frozenset(
            &code.constants,
            root,
            python_version,
        ),
        Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::NonReflexive(_)
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::String(_)
        | Constant::SurrogateString(_)
        | Constant::Bytes(_) => false,
    }
}

fn constants_contain_identifier_or_explicit_dependent_string<'a>(
    constants: impl IntoIterator<Item = &'a Constant>,
    root: &CodeObject,
    python_version: PythonVersion,
) -> bool {
    constants.into_iter().any(|constant| {
        matches!(
            constant,
            Constant::String(_)
                | Constant::Tuple(_)
                | Constant::FrozenSet(_)
                | Constant::Slice { .. }
        ) && constant_contains_identifier_or_explicit_dependent_string(
            constant,
            root,
            python_version,
        )
    })
}

fn constant_contains_identifier_or_explicit_dependent_string(
    constant: &Constant,
    root: &CodeObject,
    python_version: PythonVersion,
) -> bool {
    match constant {
        Constant::String(value) => {
            should_intern(value)
                && !value.is_empty()
                && !is_cached_character(value)
                && !is_preinterned_string_for(python_version, value)
                && code_tree_interns_string(root, value)
        }
        Constant::Tuple(constants) | Constant::FrozenSet(constants) => {
            constants_contain_identifier_or_explicit_dependent_string(
                constants,
                root,
                python_version,
            )
        }
        Constant::Slice { lower, upper, step } => {
            constants_contain_identifier_or_explicit_dependent_string(
                [lower.as_ref(), upper.as_ref(), step.as_ref()],
                root,
                python_version,
            )
        }
        Constant::Code(_)
        | Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::NonReflexive(_)
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::SurrogateString(_)
        | Constant::Bytes(_) => false,
    }
}

fn code_tree_interns_string(code: &CodeObject, value: &str) -> bool {
    code.marshal_provenance
        .parser_interned_strings
        .contains(value)
        || code
            .marshal_provenance
            .explicitly_interned_strings
            .iter()
            .any(|interned| interned == value)
        || code.names.iter().any(|name| name == value)
        || code.locals.iter().any(|local| local == value)
        || code.name == value
        || code.constants.iter().any(|constant| match constant {
            Constant::Code(code) => code_tree_interns_string(code, value),
            _ => false,
        })
}

fn collect_preinterned_recreated_constant(
    constant: &Constant,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
    recreated_frozensets: &mut FxHashSet<usize>,
) -> bool {
    match constant {
        Constant::String(value) => {
            should_intern(value)
                && !value.is_empty()
                && !is_cached_character(value)
                && initial_intern_state.contains_for(python_version, value)
        }
        Constant::FrozenSet(members) => {
            let recreated = members.iter().any(|member| {
                matches!(
                    member,
                    Constant::String(_)
                        | Constant::Tuple(_)
                        | Constant::FrozenSet(_)
                        | Constant::Code(_)
                ) && collect_preinterned_recreated_constant(
                    member,
                    python_version,
                    initial_intern_state,
                    recreated_frozensets,
                )
            });
            if recreated {
                recreated_frozensets.insert(std::ptr::from_ref(constant).addr());
            }
            recreated
        }
        Constant::Tuple(constants) => {
            // Tuple elements cannot recreate an enclosing frozen set on this path. Still find
            // nested frozen sets, without visiting unrelated scalar constants along the way.
            for constant in constants.iter().rev() {
                if matches!(
                    constant,
                    Constant::Tuple(_) | Constant::FrozenSet(_) | Constant::Code(_)
                ) {
                    collect_preinterned_recreated_constant(
                        constant,
                        python_version,
                        initial_intern_state,
                        recreated_frozensets,
                    );
                }
            }
            false
        }
        Constant::Code(code) => {
            collect_preinterned_recreated_frozensets(
                code,
                python_version,
                initial_intern_state,
                recreated_frozensets,
            );
            false
        }
        Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::NonReflexive(_)
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::SurrogateString(_)
        | Constant::Bytes(_)
        | Constant::Slice { .. } => false,
    }
}

fn collect_identifier_strings<'a>(
    code: &'a CodeObject,
    generated_mangled_strings: &FxHashSet<String>,
    values: &mut InternedStrings<'a, '_>,
) {
    values.extend(
        code.marshal_provenance
            .parser_interned_strings
            .iter()
            .map(String::as_str),
    );
    values.extend(
        code.names
            .iter()
            .chain(&code.locals)
            .map(Name::as_str)
            .filter(|value| should_intern(value) && !generated_mangled_strings.contains(*value)),
    );
    if should_intern(&code.name) && !generated_mangled_strings.contains(&code.name) {
        values.insert(&code.name);
    }
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            collect_identifier_strings(code, generated_mangled_strings, values);
        }
    }
}

fn collect_code_interning<'a>(
    code: &'a CodeObject,
    recreated_frozensets: &mut FxHashSet<usize>,
    tracked_string_origins: &FxHashSet<String>,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) {
    // Nested scopes are assembled into code objects before the containing scope.
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            collect_code_interning(
                code,
                recreated_frozensets,
                tracked_string_origins,
                interned_strings,
                interned_constant_strings,
            );
        }
    }

    intern_code_object(
        code,
        CodeTupleOrigins::default(),
        recreated_frozensets,
        tracked_string_origins,
        interned_strings,
        interned_constant_strings,
    );
}

/// Whether a metadata tuple reuses strings from the compiler's ordinary constant cache.
#[derive(Clone, Copy, Default)]
struct CodeTupleOrigins {
    names: bool,
    locals: bool,
}

impl CodeTupleOrigins {
    fn record<'a>(
        summary: &'a CodeInterningSummary,
        tuple_origins: &mut FxHashMap<Vec<&'a str>, bool>,
    ) -> Self {
        // assemble.c merges names, constants, then locals before _PyCode_New interns them.
        let names = *tuple_origins
            .entry(summary.names.iter().map(Name::as_str).collect())
            .or_insert(false);
        if let Some(constants) = &summary.constants_tuple {
            tuple_origins
                .entry(constants.iter().map(String::as_str).collect())
                .or_insert(true);
        }
        let locals = *tuple_origins
            .entry(summary.locals.iter().map(Name::as_str).collect())
            .or_insert(false);
        Self { names, locals }
    }
}

fn collect_compilation_interning<'a>(
    code: &'a CodeObject,
    recreated_frozensets: &mut FxHashSet<usize>,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) {
    fn collect_surviving<'a>(code: &'a CodeObject, codes: &mut FxHashMap<usize, &'a CodeObject>) {
        if let Some(order) = code.marshal_provenance.assembly_order {
            codes.insert(order, code);
        }
        for constant in &code.constants {
            if let Constant::Code(child) = constant {
                collect_surviving(child, codes);
            }
        }
    }

    let mut surviving = FxHashMap::default();
    collect_surviving(code, &mut surviving);
    let mut tuple_origins = FxHashMap::default();
    for (order, event) in code.marshal_provenance.interning_events.iter().enumerate() {
        match event {
            CompilationInterningEvent::CachedTuple(values) => {
                tuple_origins
                    .entry(values.iter().map(String::as_str).collect())
                    .or_insert(true);
            }
            CompilationInterningEvent::CodeAssembly(summary) => {
                let origins = CodeTupleOrigins::record(summary, &mut tuple_origins);
                if let Some(current) = surviving.get(&order) {
                    intern_code_object(
                        current,
                        origins,
                        recreated_frozensets,
                        &code.marshal_provenance.parser_interned_strings,
                        interned_strings,
                        interned_constant_strings,
                    );
                } else {
                    intern_code_summary(
                        summary,
                        origins,
                        &code.marshal_provenance.parser_interned_strings,
                        interned_strings,
                        interned_constant_strings,
                    );
                }
            }
        }
    }
}

fn intern_explicit_constants<'a>(
    explicitly_interned: &'a FxHashSet<String>,
    tracked_string_origins: &FxHashSet<String>,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) {
    if interned_strings.initial.has_unknown_entries() {
        let mut values: Vec<_> = explicitly_interned.iter().collect();
        values.sort_unstable();
        for value in values {
            if tracked_string_origins.contains(value) {
                interned_strings.insert(value);
            } else if interned_strings.tracks_origin(value)
                && should_intern(value)
                && !value.is_empty()
                && !is_cached_character(value)
                && !interned_constant_strings.contains(value.as_str())
                && interned_strings.contains(value) != Some(true)
            {
                // This aggregate hook does not prove when a generated name was
                // interned or which object it used. Initial absence cannot prove
                // compiler ownership before the real metadata/constant phases.
                interned_strings.uncertain_origins.insert(value);
                interned_strings.possibly_added.insert(value);
            } else {
                intern_constant_string(value, interned_strings, interned_constant_strings);
            }
        }
        return;
    }
    #[expect(
        clippy::iter_over_hash_type,
        reason = "These sets are only queried for membership after all explicit constants are interned."
    )]
    for value in explicitly_interned {
        interned_strings.insert(value);
        // A later key tuple must preserve an earlier ordinary literal's cache identity.
        if !tracked_string_origins.contains(value) {
            interned_constant_strings.insert(value);
        }
    }
}

fn intern_metadata_tuple<'a>(
    values: &'a [Name],
    constant_origin: bool,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) {
    for value in values.iter().rev() {
        if constant_origin {
            intern_constant_string(value, interned_strings, interned_constant_strings);
        } else {
            interned_strings.insert(value);
        }
    }
}

fn intern_code_object<'a>(
    code: &'a CodeObject,
    origins: CodeTupleOrigins,
    recreated_frozensets: &mut FxHashSet<usize>,
    tracked_string_origins: &FxHashSet<String>,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) {
    intern_explicit_constants(
        &code.marshal_provenance.explicitly_interned_strings,
        tracked_string_origins,
        interned_strings,
        interned_constant_strings,
    );
    intern_metadata_tuple(
        &code.names,
        origins.names,
        interned_strings,
        interned_constant_strings,
    );
    // CPython walks each code object's constants tuple backwards.
    for constant in code.constants.iter().rev() {
        intern_code_constant(
            constant,
            recreated_frozensets,
            interned_strings,
            interned_constant_strings,
        );
    }
    intern_metadata_tuple(
        &code.locals,
        origins.locals,
        interned_strings,
        interned_constant_strings,
    );
    if interned_strings.python_version >= PythonVersion::Py313 {
        if let Some(filename) = code.filename.as_utf8() {
            interned_strings.insert(filename);
        }
        interned_strings.insert(&code.name);
        interned_strings.insert(&code.qualified_name);
    }
}

fn intern_code_summary<'a>(
    summary: &'a CodeInterningSummary,
    origins: CodeTupleOrigins,
    tracked_string_origins: &FxHashSet<String>,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) {
    intern_explicit_constants(
        &summary.explicitly_interned_strings,
        tracked_string_origins,
        interned_strings,
        interned_constant_strings,
    );
    intern_metadata_tuple(
        &summary.names,
        origins.names,
        interned_strings,
        interned_constant_strings,
    );
    for value in &summary.constant_strings {
        intern_constant_string(value, interned_strings, interned_constant_strings);
    }
    intern_metadata_tuple(
        &summary.locals,
        origins.locals,
        interned_strings,
        interned_constant_strings,
    );
    // Discarded code may release these mortal owners. In partial states, this
    // invalidates initial absence without establishing retained presence.
    if interned_strings.python_version >= PythonVersion::Py313 {
        for value in summary
            .filename
            .as_utf8()
            .into_iter()
            .chain([summary.name.as_str(), summary.qualified_name.as_str()])
        {
            if interned_strings.initial.has_unknown_entries() {
                if interned_strings.tracks_origin(value) {
                    interned_strings.possibly_added.insert(value);
                }
            } else {
                interned_strings.insert(value);
            }
        }
    }
}

/// Returns whether CPython replaces the constant while recursively interning code constants.
fn intern_code_constant<'a>(
    constant: &'a Constant,
    recreated_frozensets: &mut FxHashSet<usize>,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) -> Replacement<'a> {
    match constant {
        Constant::String(value) => {
            intern_constant_string(value, interned_strings, interned_constant_strings)
        }
        Constant::FrozenSet(members) => {
            let mut recreated = Replacement::Never;
            for member in members {
                recreated = recreated.or(intern_code_constant(
                    member,
                    recreated_frozensets,
                    interned_strings,
                    interned_constant_strings,
                ));
            }
            match recreated {
                Replacement::Always => {
                    recreated_frozensets.insert(std::ptr::from_ref(constant).addr());
                }
                Replacement::Unknown(value) => {
                    interned_strings.record_unresolved_frozenset(constant, value);
                }
                Replacement::Never => {}
            }
            recreated
        }
        Constant::Tuple(constants) => {
            for constant in constants.iter().rev() {
                intern_code_constant(
                    constant,
                    recreated_frozensets,
                    interned_strings,
                    interned_constant_strings,
                );
            }
            // Tuples are updated in place, so replacing an item does not replace the tuple.
            Replacement::Never
        }
        Constant::Code(_)
        | Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::NonReflexive(_)
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::SurrogateString(_)
        | Constant::Bytes(_)
        | Constant::Slice { .. } => Replacement::Never,
    }
}

fn intern_constant_string<'a>(
    value: &'a str,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) -> Replacement<'a> {
    if value.is_empty() || is_cached_character(value) {
        interned_strings.insert(value);
        interned_constant_strings.insert(value);
        return Replacement::Never;
    }
    if !should_intern(value)
        || interned_constant_strings.contains(value)
        || !interned_strings.tracks_origin(value)
    {
        return Replacement::Never;
    }

    // Membership can become known without resolving whether the compiler's
    // original constant was canonical. Keep that uncertainty across every later
    // interning event, including metadata and discarded-code transitions.
    let replaced = if interned_strings.uncertain_origins.contains(value) {
        Replacement::Unknown(value)
    } else {
        match interned_strings.contains(value) {
            Some(true) => Replacement::Always,
            Some(false) => {
                interned_constant_strings.insert(value);
                Replacement::Never
            }
            None => {
                interned_strings.uncertain_origins.insert(value);
                Replacement::Unknown(value)
            }
        }
    };
    interned_strings.insert(value);
    replaced
}

struct GraphVisitor<'graph, 'code> {
    graph: &'graph mut ObjectGraph<'code>,
    explicitly_interned: Option<&'graph FxHashSet<&'code str>>,
    recreated_frozensets: Option<&'graph FxHashSet<usize>>,
    sort_graph: Option<&'graph ObjectGraph<'code>>,
    root_constants: bool,
}

impl<'code> MarshalVisitor<'code> for GraphVisitor<'_, 'code> {
    const USES_METADATA_IDS: bool = true;

    fn python_version(&self) -> PythonVersion {
        self.graph.python_version
    }

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
        let root_constants = std::mem::take(&mut self.root_constants);
        if values.is_empty() {
            return self
                .graph
                .record_cached_value(OccurrenceKind::Constants, CachedObject::EmptyTuple);
        }
        let key = constants_tuple_key(values);
        // The root constant tuple cannot equal one of its own descendants. Only all-string
        // roots can alias a metadata tuple, and those retain their ordinary value lookup.
        if matches!(&key, ObjectKey::ConstantTuple(_))
            && (root_constants
                || values
                    .iter()
                    .any(|value| matches!(value, Constant::Code(_))))
        {
            self.graph
                .record_unique_value(OccurrenceKind::Constants, key)
        } else {
            // The key has already scanned every member of an all-string tuple. It cannot own
            // code, and equal literal and metadata tuples must share the same value lookup.
            self.graph.record_keyed(OccurrenceKind::Constants, key)
        }
    }

    #[inline]
    fn names(&mut self, code: &'code CodeObject) -> bool {
        if code.names.is_empty() {
            self.graph
                .record_cached_value(OccurrenceKind::Strings, CachedObject::EmptyTuple)
        } else if let Some(properties) = code.marshal_provenance.metadata_tuple_properties {
            self.graph.record_hashed(
                OccurrenceKind::Strings,
                HashedObjectKey::with_hash(names_tuple_key(&code.names), properties.names.hash),
            )
        } else {
            self.graph
                .record_keyed(OccurrenceKind::Strings, names_tuple_key(&code.names))
        }
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
        } else if let Some(properties) = code.marshal_provenance.metadata_tuple_properties {
            self.graph.record_hashed(
                OccurrenceKind::Locals,
                HashedObjectKey::with_hash(key, properties.locals.hash),
            )
        } else {
            self.graph.record_keyed(OccurrenceKind::Locals, key)
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
    fn metadata_string(&mut self, value: &'code str, metadata_id: Option<MetadataId>) {
        let first = if self.graph.cache_scalars
            && let Some(character) = CachedObject::character(value)
        {
            self.graph
                .record_cached_value(OccurrenceKind::MetadataString, character)
        } else {
            self.graph
                .record_metadata_string(OccurrenceKind::MetadataString, value, metadata_id)
        };
        if first {
            self.graph.estimate_long_payload(value.len());
        }
    }

    #[inline]
    fn first_line(&mut self, _value: u32) {}

    fn code_filename(&mut self, code: &'code CodeObject, metadata_id: Option<MetadataId>) {
        match &code.filename {
            CodeFilename::Utf8(filename) => {
                if self.graph.python_version != PythonVersion::Py312
                    || filename.is_empty()
                    || is_cached_character(filename)
                {
                    self.metadata_string(filename, metadata_id);
                } else if self.graph.record_keyed(
                    OccurrenceKind::MetadataString,
                    ObjectKey::Filename(filename),
                ) {
                    self.graph.estimate_long_payload(filename.len());
                }
            }
            CodeFilename::Surrogate(filename) => {
                if self.graph.record_surrogate_filename(filename) {
                    self.graph.estimate_long_payload(filename.len());
                }
            }
        }
    }

    fn code_name(&mut self, code: &'code CodeObject, metadata_id: Option<MetadataId>) {
        if self.graph.python_version == PythonVersion::Py312
            && code.name.starts_with("<generic parameters of ")
        {
            if self.graph.record_keyed(
                OccurrenceKind::MetadataString,
                ObjectKey::UninternedMetadata(std::ptr::from_ref(&code.name).addr()),
            ) {
                self.graph.estimate_long_payload(code.name.len());
            }
        } else {
            self.metadata_string(&code.name, metadata_id);
        }
    }

    fn code_qualified_name(&mut self, code: &'code CodeObject, metadata_id: Option<MetadataId>) {
        if self.graph.python_version != PythonVersion::Py312 {
            self.metadata_string(&code.qualified_name, metadata_id);
        } else if code.name == code.qualified_name {
            self.code_name(code, metadata_id);
        } else {
            let aliases_constant = code
                .marshal_provenance
                .qualified_name_is_canonical
                .unwrap_or_else(|| {
                    self.graph
                        .legacy_qualified_name_aliases
                        .contains(&std::ptr::from_ref(code).addr())
                });
            let key = if aliases_constant {
                ObjectKey::ConstantString(&code.qualified_name)
            } else {
                ObjectKey::UninternedMetadata(std::ptr::from_ref(&code.qualified_name).addr())
            };
            if self.graph.record_keyed(OccurrenceKind::MetadataString, key) {
                self.graph.estimate_long_payload(code.qualified_name.len());
            }
        }
    }

    #[inline]
    fn bytes(&mut self, value: &'code [u8]) {
        if self.graph.record_bytes(OccurrenceKind::Bytes, value) {
            self.graph.estimate_payload(value.len());
        }
    }

    #[inline]
    fn constant(&mut self, value: &'code Constant, _reference_mode: ReferenceMode) -> bool {
        if let Constant::FrozenSet(values) = value {
            self.graph.needs_reference_sorting |=
                (self.graph.python_version == PythonVersion::Py312 && values.len() > 1)
                    || (values.len() > 1
                        && values.iter().any(|value| {
                            let Constant::String(value) = value else {
                                return false;
                            };
                            !should_intern(value)
                                && (self
                                    .graph
                                    .metadata_aliased_constants
                                    .contains(&std::ptr::from_ref(value).addr())
                                    || self
                                        .explicitly_interned
                                        .is_some_and(|values| values.contains(value.as_str())))
                        }))
                    || (self.graph.uncached_percent_constant
                        && values
                            .iter()
                            .any(|value| matches!(value, Constant::String(value) if value == "%")))
                    || values
                        .iter()
                        .any(|value| matches!(value, Constant::Tuple(_) | Constant::FrozenSet(_)));
        }
        match value {
            Constant::None | Constant::Bool(_) | Constant::Ellipsis => true,
            Constant::Tuple(_) | Constant::Code(_) => {
                unreachable!("tuple and code constants have dedicated traversal methods")
            }
            Constant::String(value) => {
                if self.graph.uncached_percent_constant && value == "%" {
                    return self
                        .graph
                        .record_keyed(OccurrenceKind::Constant, ObjectKey::ConstantString(value));
                }
                if self.graph.cache_scalars
                    && let Some(character) = CachedObject::character(value)
                {
                    return self
                        .graph
                        .record_cached_value(OccurrenceKind::Constant, character);
                }
                let interned = should_intern(value)
                    || self
                        .graph
                        .metadata_aliased_constants
                        .contains(&std::ptr::from_ref(value).addr())
                    || self
                        .explicitly_interned
                        .is_some_and(|values| values.contains(value.as_str()));
                let first = if interned {
                    self.graph
                        .record_interned_string(OccurrenceKind::Constant, value)
                } else {
                    self.graph
                        .record_keyed(OccurrenceKind::Constant, ObjectKey::ConstantString(value))
                };
                if first {
                    self.graph.estimate_long_payload(value.len());
                }
                first
            }
            Constant::FrozenSet(_)
                if self
                    .recreated_frozensets
                    .is_some_and(|values| values.contains(&std::ptr::from_ref(value).addr())) =>
            {
                let address = std::ptr::from_ref(value).addr();
                self.graph.record_unique_value(
                    OccurrenceKind::Constant,
                    ObjectKey::RecreatedFrozenSet(address),
                )
            }
            Constant::Int(value)
                if self.graph.cache_scalars
                    && *value
                        <= small_int_bounds_for(self.graph.python_version)
                            .1
                            .cast_unsigned() =>
            {
                self.graph.record_cached_value(
                    OccurrenceKind::Constant,
                    CachedObject::Int(u16::try_from(*value).expect("cached int fits")),
                )
            }
            Constant::SignedInt(value)
                if self.graph.cache_scalars && {
                    let (minimum, maximum) = small_int_bounds_for(self.graph.python_version);
                    (minimum..=maximum).contains(value)
                } =>
            {
                self.graph.record_cached_value(
                    OccurrenceKind::Constant,
                    CachedObject::SignedInt(
                        u16::try_from(*value - small_int_bounds_for(self.graph.python_version).0)
                            .expect("cached signed int index fits"),
                    ),
                )
            }
            Constant::Bytes(value) => {
                let first = self.graph.record_bytes(OccurrenceKind::Constant, value);
                if first {
                    self.graph.estimate_long_payload(value.len());
                }
                first
            }
            Constant::SurrogateString(bytes) => {
                let first = self
                    .graph
                    .record_keyed(OccurrenceKind::Constant, constant_key(value));
                if first {
                    self.graph.estimate_long_payload(bytes.len());
                }
                first
            }
            Constant::BigInt { digits, .. } => {
                let first = self
                    .graph
                    .record_keyed(OccurrenceKind::Constant, constant_key(value));
                if first {
                    self.graph
                        .estimate_long_payload(digits.len().saturating_mul(2));
                }
                first
            }
            _ => self
                .graph
                .record_keyed(OccurrenceKind::Constant, constant_key(value)),
        }
    }

    fn frozen_set_members(&mut self, values: &'code [Constant]) -> FrozenSetMembers<'code> {
        if values.len() <= 1 {
            return values.iter().collect();
        }

        let members = if let Some(graph) = self.sort_graph {
            graph.sort_frozen_set_members(values)
        } else {
            frozen_set_members_in_wire_order(values)
        };
        // Graph traversal and writer traversal visit precisely the same frozen sets. Preserve the
        // final graph's member order so the writer need not rebuild independent marshal sort keys.
        if self.sort_graph.is_some() || !self.graph.needs_reference_sorting {
            self.graph
                .frozen_set_orders
                .push((values.as_ptr().addr(), members.clone()));
        }
        members
    }

    fn cached_slice_string(&mut self, value: &'code Constant) {
        if self.graph.uncached_percent_constant
            && matches!(value, Constant::String(value) if value == "%")
        {
            // Slice members bypass the scalar constant cache. The parsed percent
            // singleton stays distinct from a fresh PyUnicode_Replace winner.
            self.graph.record_cached_value(
                OccurrenceKind::Constant,
                CachedObject::character("%").expect("percent is a Latin-1 character"),
            );
        } else {
            // Other characters retain ordinary canonical interning and can alias
            // previously emitted names, literals, and other slice members.
            self.constant(value, ReferenceMode::IfShared);
        }
    }

    #[inline]
    fn slice_member(&mut self, value: &'code Constant) {
        if self.graph.python_version >= PythonVersion::Py314
            && matches!(value, Constant::Bytes(value) if value.is_empty())
        {
            self.graph
                .record_cached_value(OccurrenceKind::SliceMember, CachedObject::EmptyBytes);
            return;
        }
        let address = std::ptr::from_ref(value).addr();
        self.graph
            .record_unique_value(OccurrenceKind::SliceMember, ObjectKey::SliceMember(address));
        match value {
            Constant::String(value) => self.graph.estimate_long_payload(value.len()),
            Constant::SurrogateString(value) | Constant::Bytes(value) => {
                self.graph.estimate_long_payload(value.len());
            }
            Constant::BigInt { digits, .. } => self
                .graph
                .estimate_long_payload(digits.len().saturating_mul(2)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod snapshots;

#[cfg(test)]
mod wire_reference_tests {
    use super::{Object, ObjectReference, ReferenceMode};

    #[test]
    fn object_references_preserve_forcing_and_the_full_reference_index() {
        let mut unique = Object {
            shared: false,
            cached: false,
            interned: false,
            reference: None,
        };
        let mut cached = Object {
            shared: false,
            cached: true,
            interned: false,
            reference: None,
        };
        let mut next_reference = (u32::MAX - 2) as usize;
        assert!(matches!(
            unique.reference_for_write(ReferenceMode::IfShared, &mut next_reference),
            ObjectReference::Unshared
        ));
        assert!(matches!(
            unique.reference_for_write(ReferenceMode::Forced, &mut next_reference),
            ObjectReference::New
        ));
        assert!(matches!(
            unique.reference_for_write(ReferenceMode::Forced, &mut next_reference),
            ObjectReference::Existing(index) if index.as_u32() == u32::MAX - 2
        ));
        assert!(matches!(
            unique.reference_for_write(ReferenceMode::IfShared, &mut next_reference),
            ObjectReference::Unshared
        ));
        assert!(matches!(
            cached.reference_for_write(ReferenceMode::IfShared, &mut next_reference),
            ObjectReference::New
        ));
        assert!(matches!(
            cached.reference_for_write(ReferenceMode::IfShared, &mut next_reference),
            ObjectReference::Existing(index) if index.as_u32() == u32::MAX - 1
        ));
        assert_eq!(next_reference, u32::MAX as usize);
    }
}
