use std::collections::hash_map::Entry;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::code_object::{CodeInterningSummary, CodeObject, CompilationInterningEvent, Constant};
use crate::target::{is_preinterned_string_for, small_int_bounds_for};
use crate::{InitialInternState, PythonVersion};

use super::key::{
    FrozenSetMembers, HashedObjectKey, ObjectKey, code_bytes_key, code_key, constant_key,
    constants_tuple_key, frozen_set_members_in_wire_order, frozen_set_members_with_properties,
    is_cached_character, local_kinds_key, locals_tuple_key, should_intern, strings_tuple_key,
};
use super::plan::{MarshalVisitor, ObjectId, ReferenceMode, traverse_code};

#[derive(Debug)]
struct Object {
    shared: bool,
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
const UNSEEN_CACHED_OBJECT: u32 = u32::MAX;
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
    cached_scalar_ids: Option<Box<[u32]>>,
    cache_scalars: bool,
    needs_reference_sorting: bool,
    legacy_qualified_name_aliases: FxHashSet<usize>,
    objects_by_id: Vec<Object>,
    occurrences: Vec<Occurrence>,
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
            // occurrences in the final member order. Python 3.12 also needs this pass for scalar
            // members because it borrows their roots while computing sort keys.
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
        let frozenset_recreation_need = collect_explicitly_interned(
            code,
            python_version,
            &mut explicitly_interned,
            &mut metadata_tuples,
        );
        let mut recreated_frozensets = FxHashSet::default();
        collect_recreated_frozensets(
            code,
            frozenset_recreation_need,
            python_version,
            initial_intern_state,
            &mut recreated_frozensets,
        );
        let explicitly_interned = (!explicitly_interned.is_empty()).then_some(&explicitly_interned);
        let recreated_frozensets =
            (!recreated_frozensets.is_empty()).then_some(&recreated_frozensets);
        let metadata_slots = usize::try_from(code.marshal_provenance.metadata_ids.domain_len)
            .ok()
            .filter(|len| *len <= MAX_DENSE_METADATA_SLOTS)
            .map(|len| vec![None; len].into_boxed_slice())
            .unwrap_or_default();
        let mut graph = Self {
            python_version,
            legacy_qualified_name_aliases: if python_version == PythonVersion::Py312 {
                legacy_qualified_name_aliases(code)
            } else {
                FxHashSet::default()
            },
            metadata_aliased_constants: collect_metadata_aliased_constants(code, &metadata_tuples),
            cache_scalars: code.constants.len() >= MIN_CACHED_SCALAR_CONSTANTS,
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
            },
        );
        graph
    }

    pub(super) fn frozen_set_members(&self, values: &'a [Constant]) -> FrozenSetMembers<'a> {
        if self.needs_reference_sorting {
            frozen_set_members_with_properties(values, self.python_version, &|value| {
                self.sort_key_properties(value)
            })
        } else {
            frozen_set_members_in_wire_order(values)
        }
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
        let key = constant_key(value);
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
        self.interned_strings.reserve(interned_strings);
        self.objects_by_id
            .reserve(estimated_descendants.saturating_add(1));
        self.occurrences
            .reserve(estimated_descendants.saturating_add(1));
    }

    fn record_keyed(&mut self, kind: OccurrenceKind, key: ObjectKey<'a>) -> bool {
        let (id, first) = self.record_object(key.into());
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
        metadata_id: Option<u32>,
    ) -> bool {
        let slot_index = metadata_id.map(|id| id as usize);
        if let Some(id) = slot_index
            .and_then(|index| self.metadata_slots.get(index).copied())
            .flatten()
        {
            debug_assert_eq!(
                self.interned_strings.get(value),
                Some(&id),
                "metadata ID must identify one string"
            );
            self.objects_by_id[id.index()].shared = true;
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

    fn interned_string(&mut self, value: &'a str) -> (ObjectId, bool) {
        let next_id = ObjectId::from_index(self.objects_by_id.len());
        match self.interned_strings.entry(value) {
            Entry::Occupied(entry) => {
                let id = *entry.get();
                self.objects_by_id[id.index()].shared = true;
                (id, false)
            }
            Entry::Vacant(entry) => {
                entry.insert(next_id);
                self.objects_by_id.push(Object {
                    shared: false,
                    cached: true,
                    interned: true,
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

        let id = ObjectId::from_index(self.objects_by_id.len());
        self.objects_by_id.push(Object {
            shared: false,
            cached: false,
            interned: false,
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
                self.objects_by_id[id.index()].shared = true;
                return (id, false);
            }
            let id = ObjectId::from_index(self.objects_by_id.len());
            self.objects_by_id.push(Object {
                shared: false,
                cached: true,
                interned: false,
            });
            *slot = Some(id);
            return (id, true);
        }

        let slot = &mut self.cached_scalar_ids.get_or_insert_with(|| {
            vec![UNSEEN_CACHED_OBJECT; cached_scalar_slots(self.python_version)].into_boxed_slice()
        })[cached.scalar_index(self.python_version)];
        if *slot != UNSEEN_CACHED_OBJECT {
            let id = ObjectId::from_index(*slot as usize);
            self.objects_by_id[id.index()].shared = true;
            return (id, false);
        }

        let index = self.objects_by_id.len();
        let id = ObjectId::from_index(index);
        self.objects_by_id.push(Object {
            shared: false,
            cached: true,
            interned: matches!(cached, CachedObject::Character(_)),
        });
        *slot = u32::try_from(index).expect("marshal object limit exceeded");
        (id, true)
    }

    fn record_object(&mut self, key: HashedObjectKey<'a>) -> (ObjectId, bool) {
        let next_id = ObjectId::from_index(self.objects_by_id.len());
        match self.objects.entry(key) {
            Entry::Occupied(entry) => {
                let cached = Self::is_cached(entry.key().key(), self.python_version);
                let id = *entry.get();
                let object = &mut self.objects_by_id[id.index()];
                object.shared = true;
                object.cached |= cached;
                (id, false)
            }
            Entry::Vacant(entry) => {
                let cached = Self::is_cached(entry.key().key(), self.python_version);
                entry.insert(next_id);
                self.objects_by_id.push(Object {
                    shared: false,
                    cached,
                    interned: false,
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
                .is_some_and(|id| self.objects_by_id[id.index()].shared)
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
        let object = &self.objects_by_id[id.index()];
        object.shared || object.cached
    }

    pub(super) fn python_version(&self) -> PythonVersion {
        self.python_version
    }

    pub(super) fn object_count(&self) -> usize {
        self.objects_by_id.len()
    }

    #[cfg(test)]
    pub(super) fn keyed_object_count(&self) -> usize {
        self.objects.len()
            + self.interned_strings.len()
            + usize::from(self.cached_empty_bytes.is_some())
            + usize::from(self.cached_empty_tuple.is_some())
            + self.cached_scalar_ids.as_deref().map_or(0, |slots| {
                slots
                    .iter()
                    .filter(|id| **id != UNSEEN_CACHED_OBJECT)
                    .count()
            })
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
        if CachedObject::character(value)
            .map(|cached| cached.scalar_index(self.python_version))
            .is_some_and(|index| {
                self.cached_scalar_ids
                    .as_deref()
                    .is_some_and(|slots| slots[index] != UNSEEN_CACHED_OBJECT)
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
    if code.names.iter().any(|value| !should_intern(value)) {
        keys.insert(strings_tuple_key(&code.names).into());
    }
    if code.locals.iter().any(|value| !should_intern(value)) {
        keys.insert(locals_tuple_key(code).into());
    }
}

fn collect_explicitly_interned<'a>(
    code: &'a CodeObject,
    python_version: PythonVersion,
    values: &mut FxHashSet<&'a str>,
    metadata_tuples: &mut FxHashSet<HashedObjectKey<'a>>,
) -> FrozensetRecreationNeed {
    // Identifier objects can enter the compilation-wide constant cache before an equal
    // ordinary string. Preserve that recorded origin when sharing constant and metadata strings.
    values.extend(
        code.marshal_provenance
            .canonical_constant_strings
            .iter()
            .map(String::as_str),
    );
    let (has_qualified_name_constant, frozenset_recreation_need) =
        if !code.marshal_provenance.subtree_has_frozenset_constants {
            (
                collect_interning_provenance_without_frozensets(code, values, metadata_tuples),
                FrozensetRecreationNeed::None,
            )
        } else {
            collect_interning_provenance(code, python_version, values, metadata_tuples)
        };
    if !has_qualified_name_constant || python_version == PythonVersion::Py312 {
        return frozenset_recreation_need;
    }

    // A class body normally loads the same qualified-name object that becomes its interned code
    // metadata. If an equal ordinary constant reached the compilation-wide constant cache first,
    // the class constant instead reuses that plain object and remains distinct from the metadata.
    let mut qualified_name_constants = FxHashSet::default();
    let mut qualified_name_values = FxHashSet::default();
    let mut recorded_origins = FxHashMap::default();
    collect_qualified_name_constants(
        code,
        &mut qualified_name_constants,
        &mut qualified_name_values,
        &mut recorded_origins,
    );
    let mut seen = FxHashSet::default();
    let mut ordinary_first = FxHashSet::default();
    collect_qualified_name_origins(
        code,
        &qualified_name_constants,
        &qualified_name_values,
        &mut seen,
        &mut ordinary_first,
    );
    values.extend(qualified_name_values.into_iter().filter(|value| {
        recorded_origins
            .get(value)
            .copied()
            .unwrap_or_else(|| !ordinary_first.contains(value))
    }));
    frozenset_recreation_need
}

fn collect_interning_provenance_without_frozensets<'a>(
    code: &'a CodeObject,
    values: &mut FxHashSet<&'a str>,
    metadata_tuples: &mut FxHashSet<HashedObjectKey<'a>>,
) -> bool {
    collect_metadata_tuple_keys(code, metadata_tuples);
    values.extend(
        code.marshal_provenance
            .explicitly_interned_strings
            .iter()
            .map(String::as_str),
    );
    let mut has_qualified_name_constant = code
        .marshal_provenance
        .qualified_name_constant_index
        .is_some();
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            has_qualified_name_constant |=
                collect_interning_provenance_without_frozensets(code, values, metadata_tuples);
        }
    }
    has_qualified_name_constant
}

fn collect_interning_provenance<'a>(
    code: &'a CodeObject,
    python_version: PythonVersion,
    values: &mut FxHashSet<&'a str>,
    metadata_tuples: &mut FxHashSet<HashedObjectKey<'a>>,
) -> (bool, FrozensetRecreationNeed) {
    collect_metadata_tuple_keys(code, metadata_tuples);
    values.extend(
        code.marshal_provenance
            .explicitly_interned_strings
            .iter()
            .map(String::as_str),
    );
    let mut has_qualified_name_constant = code
        .marshal_provenance
        .qualified_name_constant_index
        .is_some();
    // Reuse the provenance walk to classify frozenset recreation for code trees that can contain
    // one (and for hand-built objects without a conservative compiler hint).
    let mut frozenset_recreation_need = FrozensetRecreationNeed::None;
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            let (has_qualified_name, recreation_need) =
                collect_interning_provenance(code, python_version, values, metadata_tuples);
            has_qualified_name_constant |= has_qualified_name;
            frozenset_recreation_need = frozenset_recreation_need.max(recreation_need);
        } else {
            frozenset_recreation_need = frozenset_recreation_need
                .max(constant_frozenset_recreation_need(constant, python_version));
        }
    }
    (has_qualified_name_constant, frozenset_recreation_need)
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

fn collect_qualified_name_constants<'a>(
    code: &'a CodeObject,
    constants: &mut FxHashSet<usize>,
    values: &mut FxHashSet<&'a str>,
    recorded_origins: &mut FxHashMap<&'a str, bool>,
) {
    if let Some(index) = code.marshal_provenance.qualified_name_constant_index {
        let index = usize::try_from(index).expect("constant index always fits in usize");
        if let Some(constant @ Constant::String(value)) = code.constants.get(index) {
            constants.insert(std::ptr::from_ref(constant).addr());
            values.insert(value.as_str());
            if let Some(canonical) = code.marshal_provenance.qualified_name_is_canonical {
                // Compilation provenance includes earlier admissions from discarded code.
                recorded_origins.insert(value.as_str(), canonical);
            }
        }
    }
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            collect_qualified_name_constants(code, constants, values, recorded_origins);
        }
    }
}

fn collect_qualified_name_origins<'a>(
    code: &'a CodeObject,
    qualified_name_constants: &FxHashSet<usize>,
    qualified_name_values: &FxHashSet<&'a str>,
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
    qualified_name_values: &FxHashSet<&'a str>,
    seen: &mut FxHashSet<&'a str>,
    ordinary_first: &mut FxHashSet<&'a str>,
) {
    match constant {
        Constant::String(value) if qualified_name_values.contains(value.as_str()) => {
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

/// Compilation-local additions layered over the immutable initial state. Keeping
/// these separate avoids copying a complete bootstrap profile for every module.
struct InternedStrings<'code, 'state> {
    initial: &'state InitialInternState,
    python_version: PythonVersion,
    added: FxHashSet<&'code str>,
}

impl<'code, 'state> InternedStrings<'code, 'state> {
    fn new(initial: &'state InitialInternState, python_version: PythonVersion) -> Self {
        Self {
            initial,
            python_version,
            added: FxHashSet::default(),
        }
    }

    fn contains(&self, value: &str) -> bool {
        self.initial.contains_for(self.python_version, value) || self.added.contains(value)
    }

    fn insert(&mut self, value: &'code str) {
        self.added.insert(value);
    }

    fn extend(&mut self, values: impl IntoIterator<Item = &'code str>) {
        self.added.extend(values);
    }
}

fn collect_recreated_frozensets(
    code: &CodeObject,
    need: FrozensetRecreationNeed,
    python_version: PythonVersion,
    initial_intern_state: &InitialInternState,
    values: &mut FxHashSet<usize>,
) {
    match need {
        FrozensetRecreationNeed::None => return,
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
            return;
        }
        FrozensetRecreationNeed::PreinternedOnly | FrozensetRecreationNeed::IdentifierDependent => {
        }
    }

    if need == FrozensetRecreationNeed::IdentifierDependent
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
        return;
    }

    let mut interned_strings = InternedStrings::new(initial_intern_state, python_version);
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
    code.constants
        .iter()
        .map(|constant| constant_frozenset_recreation_need(constant, python_version))
        .max()
        .unwrap_or(FrozensetRecreationNeed::None)
}

fn constant_frozenset_recreation_need(
    constant: &Constant,
    python_version: PythonVersion,
) -> FrozensetRecreationNeed {
    match constant {
        Constant::FrozenSet(members) => members
            .iter()
            .map(|constant| constant_string_recreation_need(constant, python_version))
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Tuple(constants) => constants
            .iter()
            .map(|constant| constant_frozenset_recreation_need(constant, python_version))
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .map(|constant| constant_frozenset_recreation_need(constant, python_version))
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Code(code) => frozenset_recreation_need(code, python_version),
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
        Constant::Tuple(constants) | Constant::FrozenSet(constants) => constants
            .iter()
            .map(|constant| constant_string_recreation_need(constant, python_version))
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .map(|constant| constant_string_recreation_need(constant, python_version))
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Code(_)
        | Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
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
        collect_preinterned_recreated_constant(
            constant,
            python_version,
            initial_intern_state,
            values,
        );
    }
}

fn has_identifier_or_explicit_dependent_frozenset(
    code: &CodeObject,
    python_version: PythonVersion,
) -> bool {
    code.constants.iter().any(|constant| {
        constant_has_identifier_or_explicit_dependent_frozenset(constant, code, python_version)
    })
}

fn constant_has_identifier_or_explicit_dependent_frozenset(
    constant: &Constant,
    root: &CodeObject,
    python_version: PythonVersion,
) -> bool {
    match constant {
        Constant::FrozenSet(members) => members.iter().any(|member| {
            constant_contains_identifier_or_explicit_dependent_string(member, root, python_version)
        }),
        Constant::Tuple(constants) => constants.iter().any(|constant| {
            constant_has_identifier_or_explicit_dependent_frozenset(constant, root, python_version)
        }),
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .any(|constant| {
                constant_has_identifier_or_explicit_dependent_frozenset(
                    constant,
                    root,
                    python_version,
                )
            }),
        Constant::Code(code) => code.constants.iter().any(|constant| {
            constant_has_identifier_or_explicit_dependent_frozenset(constant, root, python_version)
        }),
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
        | Constant::Bytes(_) => false,
    }
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
            constants.iter().any(|constant| {
                constant_contains_identifier_or_explicit_dependent_string(
                    constant,
                    root,
                    python_version,
                )
            })
        }
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .any(|constant| {
                constant_contains_identifier_or_explicit_dependent_string(
                    constant,
                    root,
                    python_version,
                )
            }),
        Constant::Code(_)
        | Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
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
                collect_preinterned_recreated_constant(
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
            for constant in constants.iter().rev() {
                collect_preinterned_recreated_constant(
                    constant,
                    python_version,
                    initial_intern_state,
                    recreated_frozensets,
                );
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
            .map(String::as_str)
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
            .entry(summary.names.iter().map(String::as_str).collect())
            .or_insert(false);
        if let Some(constants) = &summary.constants_tuple {
            tuple_origins
                .entry(constants.iter().map(String::as_str).collect())
                .or_insert(true);
        }
        let locals = *tuple_origins
            .entry(summary.locals.iter().map(String::as_str).collect())
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
    values: &'a [String],
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
        interned_strings.insert(&code.filename);
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
    if interned_strings.python_version >= PythonVersion::Py313 {
        interned_strings.insert(&summary.filename);
        interned_strings.insert(&summary.name);
        interned_strings.insert(&summary.qualified_name);
    }
}

/// Returns whether CPython replaces the constant while recursively interning code constants.
fn intern_code_constant<'a>(
    constant: &'a Constant,
    recreated_frozensets: &mut FxHashSet<usize>,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) -> bool {
    match constant {
        Constant::String(value) => {
            intern_constant_string(value, interned_strings, interned_constant_strings)
        }
        Constant::FrozenSet(members) => {
            let mut recreated = false;
            for member in members {
                recreated |= intern_code_constant(
                    member,
                    recreated_frozensets,
                    interned_strings,
                    interned_constant_strings,
                );
            }
            if recreated {
                recreated_frozensets.insert(std::ptr::from_ref(constant).addr());
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
            false
        }
        Constant::Code(_)
        | Constant::None
        | Constant::Bool(_)
        | Constant::Ellipsis
        | Constant::Int(_)
        | Constant::SignedInt(_)
        | Constant::BigInt { .. }
        | Constant::Float(_)
        | Constant::Complex { .. }
        | Constant::SurrogateString(_)
        | Constant::Bytes(_)
        | Constant::Slice { .. } => false,
    }
}

fn intern_constant_string<'a>(
    value: &'a str,
    interned_strings: &mut InternedStrings<'a, '_>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) -> bool {
    if value.is_empty() || is_cached_character(value) {
        interned_strings.insert(value);
        interned_constant_strings.insert(value);
        return false;
    }
    if !should_intern(value) || interned_constant_strings.contains(value) {
        return false;
    }

    let replaced = interned_strings.contains(value);
    interned_strings.insert(value);
    if !replaced {
        // With no earlier canonical string, CPython interns the compiler constant-cache object
        // itself. Later equal constants therefore already have the canonical identity.
        interned_constant_strings.insert(value);
    }
    replaced
}
struct GraphVisitor<'graph, 'code> {
    graph: &'graph mut ObjectGraph<'code>,
    explicitly_interned: Option<&'graph FxHashSet<&'code str>>,
    recreated_frozensets: Option<&'graph FxHashSet<usize>>,
    sort_graph: Option<&'graph ObjectGraph<'code>>,
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
            self.graph.record_keyed(OccurrenceKind::Constants, key)
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
            .record_keyed(OccurrenceKind::Strings, strings_tuple_key(values))
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
    fn metadata_string(&mut self, value: &'code str, metadata_id: Option<u32>) {
        if self.graph.cache_scalars
            && let Some(character) = CachedObject::character(value)
        {
            self.graph
                .record_cached_value(OccurrenceKind::MetadataString, character);
        } else {
            self.graph
                .record_metadata_string(OccurrenceKind::MetadataString, value, metadata_id);
        }
    }

    #[inline]
    fn first_line(&mut self, _value: u32) {}

    fn code_filename(&mut self, code: &'code CodeObject, metadata_id: Option<u32>) {
        if self.graph.python_version != PythonVersion::Py312
            || code.filename.is_empty()
            || is_cached_character(&code.filename)
        {
            self.metadata_string(&code.filename, metadata_id);
        } else {
            self.graph.record_keyed(
                OccurrenceKind::MetadataString,
                ObjectKey::Filename(&code.filename),
            );
        }
    }

    fn code_name(&mut self, code: &'code CodeObject, metadata_id: Option<u32>) {
        if self.graph.python_version == PythonVersion::Py312
            && code.name.starts_with("<generic parameters of ")
        {
            self.graph.record_keyed(
                OccurrenceKind::MetadataString,
                ObjectKey::UninternedMetadata(std::ptr::from_ref(&code.name).addr()),
            );
        } else {
            self.metadata_string(&code.name, metadata_id);
        }
    }

    fn code_qualified_name(&mut self, code: &'code CodeObject, metadata_id: Option<u32>) {
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
            self.graph.record_keyed(OccurrenceKind::MetadataString, key);
        }
    }

    #[inline]
    fn bytes(&mut self, value: &'code [u8]) {
        match value {
            [] => {
                self.graph
                    .record_cached_value(OccurrenceKind::Bytes, CachedObject::EmptyBytes);
            }
            [value] if self.graph.cache_scalars => {
                self.graph
                    .record_cached_value(OccurrenceKind::Bytes, CachedObject::Byte(*value));
            }
            _ => {
                self.graph
                    .record_keyed(OccurrenceKind::Bytes, ObjectKey::Bytes(value));
            }
        }
    }

    #[inline]
    fn constant(&mut self, value: &'code Constant, _reference_mode: ReferenceMode) -> bool {
        if let Constant::FrozenSet(values) = value {
            self.graph.needs_reference_sorting |=
                (self.graph.python_version == PythonVersion::Py312 && values.len() > 1)
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
                if interned {
                    self.graph
                        .record_interned_string(OccurrenceKind::Constant, value)
                } else {
                    self.graph
                        .record_keyed(OccurrenceKind::Constant, ObjectKey::ConstantString(value))
                }
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
            Constant::Bytes(value) => match value.as_slice() {
                [] => self
                    .graph
                    .record_cached_value(OccurrenceKind::Constant, CachedObject::EmptyBytes),
                [value] if self.graph.cache_scalars => self
                    .graph
                    .record_cached_value(OccurrenceKind::Constant, CachedObject::Byte(*value)),
                _ => self
                    .graph
                    .record_keyed(OccurrenceKind::Constant, ObjectKey::Bytes(value)),
            },
            _ => self
                .graph
                .record_keyed(OccurrenceKind::Constant, constant_key(value)),
        }
    }

    fn frozen_set_members(&self, values: &'code [Constant]) -> FrozenSetMembers<'code> {
        if let Some(graph) = self.sort_graph {
            graph.frozen_set_members(values)
        } else {
            frozen_set_members_in_wire_order(values)
        }
    }

    #[inline]
    fn slice_member(&mut self, value: &'code Constant) {
        let address = std::ptr::from_ref(value).addr();
        self.graph
            .record_unique_value(OccurrenceKind::SliceMember, ObjectKey::SliceMember(address));
    }
}
