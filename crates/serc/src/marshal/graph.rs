use std::collections::hash_map::Entry;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::code_object::{CodeObject, Constant};
use crate::target::is_preinterned_string;

use super::key::{
    HashedObjectKey, ObjectKey, code_bytes_key, code_key, constant_key, constants_tuple_key,
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
const CACHED_INT_SLOTS: usize = 257;
const CACHED_SIGNED_INT_SLOTS: usize = 262;
const CACHED_BYTE_SLOTS: usize = 256;
const CACHED_CHARACTER_SLOTS: usize = 256;
const CACHED_SCALAR_SLOTS: usize =
    CACHED_INT_SLOTS + CACHED_SIGNED_INT_SLOTS + CACHED_BYTE_SLOTS + CACHED_CHARACTER_SLOTS;
const UNSEEN_CACHED_OBJECT: u32 = u32::MAX;
const MIN_CACHED_SCALAR_CONSTANTS: usize = 32;

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

    fn scalar_index(self) -> usize {
        match self {
            Self::EmptyBytes | Self::EmptyTuple => {
                unreachable!("empty cached object does not have a scalar index")
            }
            Self::Int(value) => usize::from(value),
            Self::SignedInt(value) => CACHED_INT_SLOTS + usize::from(value),
            Self::Byte(value) => CACHED_INT_SLOTS + CACHED_SIGNED_INT_SLOTS + usize::from(value),
            Self::Character(value) => {
                CACHED_INT_SLOTS + CACHED_SIGNED_INT_SLOTS + CACHED_BYTE_SLOTS + usize::from(value)
            }
        }
    }

    #[cfg(test)]
    fn for_key(key: &ObjectKey<'_>) -> Option<Self> {
        match key {
            ObjectKey::Bytes([]) => Some(Self::EmptyBytes),
            ObjectKey::Bytes([value]) => Some(Self::Byte(*value)),
            ObjectKey::EmptyTuple => Some(Self::EmptyTuple),
            ObjectKey::Int(value) if *value <= 256 => {
                Some(Self::Int(u16::try_from(*value).expect("cached int fits")))
            }
            ObjectKey::SignedInt(value) if (-5..=256).contains(value) => Some(Self::SignedInt(
                u16::try_from(*value + 5).expect("cached signed int index fits"),
            )),
            ObjectKey::String(value) => Self::character(value),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ObjectGraph<'a> {
    objects: FxHashMap<HashedObjectKey<'a>, ObjectId>,
    interned_strings: FxHashMap<&'a str, ObjectId>,
    cached_empty_bytes: Option<ObjectId>,
    cached_empty_tuple: Option<ObjectId>,
    cached_scalar_ids: Option<Box<[u32]>>,
    cache_scalars: bool,
    objects_by_id: Vec<Object>,
    occurrences: Vec<Occurrence>,
    #[cfg(debug_assertions)]
    unique_objects: FxHashSet<HashedObjectKey<'a>>,
}

impl<'a> ObjectGraph<'a> {
    pub(super) fn for_code(code: &'a CodeObject) -> Self {
        let mut explicitly_interned = FxHashSet::default();
        collect_explicitly_interned(code, &mut explicitly_interned);
        let mut recreated_frozensets = FxHashSet::default();
        collect_recreated_frozensets(code, &mut recreated_frozensets);
        let explicitly_interned = (!explicitly_interned.is_empty()).then_some(&explicitly_interned);
        let recreated_frozensets =
            (!recreated_frozensets.is_empty()).then_some(&recreated_frozensets);
        let mut graph = Self {
            cache_scalars: code.constants.len() >= MIN_CACHED_SCALAR_CONSTANTS,
            ..Self::default()
        };
        traverse_code(
            code,
            &mut GraphVisitor {
                graph: &mut graph,
                explicitly_interned,
                recreated_frozensets,
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
        let next_id = ObjectId::from_index(self.objects_by_id.len());
        let (id, first) = match self.interned_strings.entry(value) {
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
        };
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
            vec![UNSEEN_CACHED_OBJECT; CACHED_SCALAR_SLOTS].into_boxed_slice()
        })[cached.scalar_index()];
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
                let cached = Self::is_cached(entry.key().key());
                let id = *entry.get();
                let object = &mut self.objects_by_id[id.index()];
                object.shared = true;
                object.cached |= cached;
                (id, false)
            }
            Entry::Vacant(entry) => {
                let cached = Self::is_cached(entry.key().key());
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
        Self::is_cached(key)
    }

    fn is_cached(key: &ObjectKey<'_>) -> bool {
        match key {
            ObjectKey::Int(value) => *value <= 256,
            ObjectKey::SignedInt(value) => (-5..=256).contains(value),
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
            .map(CachedObject::scalar_index)
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

fn collect_explicitly_interned<'a>(code: &'a CodeObject, values: &mut FxHashSet<&'a str>) {
    if !collect_explicit_intern_hints(code, values) {
        return;
    }

    // A class body normally loads the same qualified-name object that becomes its interned code
    // metadata. If an equal ordinary constant reached the compilation-wide constant cache first,
    // the class constant instead reuses that plain object and remains distinct from the metadata.
    let mut qualified_name_constants = FxHashSet::default();
    let mut qualified_name_values = FxHashSet::default();
    collect_qualified_name_constants(
        code,
        &mut qualified_name_constants,
        &mut qualified_name_values,
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
    values.extend(qualified_name_values.difference(&ordinary_first).copied());
}

fn collect_explicit_intern_hints<'a>(
    code: &'a CodeObject,
    values: &mut FxHashSet<&'a str>,
) -> bool {
    values.extend(
        code.marshal_hints
            .explicitly_interned_strings
            .iter()
            .map(String::as_str),
    );
    let mut has_qualified_name_constant =
        code.marshal_hints.qualified_name_constant_index.is_some();
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            has_qualified_name_constant |= collect_explicit_intern_hints(code, values);
        }
    }
    has_qualified_name_constant
}

fn collect_qualified_name_constants<'a>(
    code: &'a CodeObject,
    constants: &mut FxHashSet<usize>,
    values: &mut FxHashSet<&'a str>,
) {
    if let Some(index) = code.marshal_hints.qualified_name_constant_index {
        let index = usize::try_from(index).expect("constant index always fits in usize");
        if let Some(constant @ Constant::String(value)) = code.constants.get(index) {
            constants.insert(std::ptr::from_ref(constant).addr());
            values.insert(value.as_str());
        }
    }
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            collect_qualified_name_constants(code, constants, values);
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

fn collect_recreated_frozensets(code: &CodeObject, values: &mut FxHashSet<usize>) {
    let need = frozenset_recreation_need(code);
    match need {
        FrozensetRecreationNeed::None => return,
        FrozensetRecreationNeed::PreinternedOnly if !has_explicit_intern_hints(code) => {
            collect_preinterned_recreated_frozensets(code, values);
            return;
        }
        FrozensetRecreationNeed::PreinternedOnly | FrozensetRecreationNeed::IdentifierDependent => {
        }
    }

    if need == FrozensetRecreationNeed::IdentifierDependent
        && !has_identifier_or_explicit_dependent_frozenset(code)
    {
        collect_preinterned_recreated_frozensets(code, values);
        return;
    }

    let mut interned_strings = FxHashSet::default();
    let mut interned_constant_strings = FxHashSet::default();
    if need == FrozensetRecreationNeed::IdentifierDependent {
        collect_identifier_strings(code, &mut interned_strings);
    }
    collect_code_interning(
        code,
        values,
        &mut interned_strings,
        &mut interned_constant_strings,
    );
}

fn has_explicit_intern_hints(code: &CodeObject) -> bool {
    !code.marshal_hints.explicitly_interned_strings.is_empty()
        || code.constants.iter().any(|constant| match constant {
            Constant::Code(code) => has_explicit_intern_hints(code),
            _ => false,
        })
}

fn frozenset_recreation_need(code: &CodeObject) -> FrozensetRecreationNeed {
    code.constants
        .iter()
        .map(constant_frozenset_recreation_need)
        .max()
        .unwrap_or(FrozensetRecreationNeed::None)
}

fn constant_frozenset_recreation_need(constant: &Constant) -> FrozensetRecreationNeed {
    match constant {
        Constant::FrozenSet(members) => members
            .iter()
            .map(constant_string_recreation_need)
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Tuple(constants) => constants
            .iter()
            .map(constant_frozenset_recreation_need)
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .map(constant_frozenset_recreation_need)
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Code(code) => frozenset_recreation_need(code),
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

fn constant_string_recreation_need(constant: &Constant) -> FrozensetRecreationNeed {
    match constant {
        Constant::String(value) => {
            if !should_intern(value) || value.is_empty() || is_cached_character(value) {
                FrozensetRecreationNeed::None
            } else if is_preinterned_string(value) {
                FrozensetRecreationNeed::PreinternedOnly
            } else {
                FrozensetRecreationNeed::IdentifierDependent
            }
        }
        Constant::Tuple(constants) | Constant::FrozenSet(constants) => constants
            .iter()
            .map(constant_string_recreation_need)
            .max()
            .unwrap_or(FrozensetRecreationNeed::None),
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .map(constant_string_recreation_need)
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

fn collect_preinterned_recreated_frozensets(code: &CodeObject, values: &mut FxHashSet<usize>) {
    for constant in &code.constants {
        collect_preinterned_recreated_constant(constant, values);
    }
}

fn has_identifier_or_explicit_dependent_frozenset(code: &CodeObject) -> bool {
    code.constants
        .iter()
        .any(|constant| constant_has_identifier_or_explicit_dependent_frozenset(constant, code))
}

fn constant_has_identifier_or_explicit_dependent_frozenset(
    constant: &Constant,
    root: &CodeObject,
) -> bool {
    match constant {
        Constant::FrozenSet(members) => members
            .iter()
            .any(|member| constant_contains_identifier_or_explicit_dependent_string(member, root)),
        Constant::Tuple(constants) => constants.iter().any(|constant| {
            constant_has_identifier_or_explicit_dependent_frozenset(constant, root)
        }),
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .any(|constant| {
                constant_has_identifier_or_explicit_dependent_frozenset(constant, root)
            }),
        Constant::Code(code) => code.constants.iter().any(|constant| {
            constant_has_identifier_or_explicit_dependent_frozenset(constant, root)
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
) -> bool {
    match constant {
        Constant::String(value) => {
            should_intern(value)
                && !value.is_empty()
                && !is_cached_character(value)
                && !is_preinterned_string(value)
                && code_tree_interns_string(root, value)
        }
        Constant::Tuple(constants) | Constant::FrozenSet(constants) => {
            constants.iter().any(|constant| {
                constant_contains_identifier_or_explicit_dependent_string(constant, root)
            })
        }
        Constant::Slice { lower, upper, step } => [lower.as_ref(), upper.as_ref(), step.as_ref()]
            .into_iter()
            .any(|constant| {
                constant_contains_identifier_or_explicit_dependent_string(constant, root)
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
    code.marshal_hints
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
    recreated_frozensets: &mut FxHashSet<usize>,
) -> bool {
    match constant {
        Constant::String(value) => {
            should_intern(value)
                && !value.is_empty()
                && !is_cached_character(value)
                && is_preinterned_string(value)
        }
        Constant::FrozenSet(members) => {
            let recreated = members
                .iter()
                .any(|member| collect_preinterned_recreated_constant(member, recreated_frozensets));
            if recreated {
                recreated_frozensets.insert(std::ptr::from_ref(constant).addr());
            }
            recreated
        }
        Constant::Tuple(constants) => {
            for constant in constants.iter().rev() {
                collect_preinterned_recreated_constant(constant, recreated_frozensets);
            }
            false
        }
        Constant::Code(code) => {
            collect_preinterned_recreated_frozensets(code, recreated_frozensets);
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

fn collect_identifier_strings<'a>(code: &'a CodeObject, values: &mut FxHashSet<&'a str>) {
    values.extend(
        code.names
            .iter()
            .chain(&code.locals)
            .map(String::as_str)
            .filter(|value| should_intern(value)),
    );
    if should_intern(&code.name) {
        values.insert(&code.name);
    }
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            collect_identifier_strings(code, values);
        }
    }
}

fn collect_code_interning<'a>(
    code: &'a CodeObject,
    recreated_frozensets: &mut FxHashSet<usize>,
    interned_strings: &mut FxHashSet<&'a str>,
    interned_constant_strings: &mut FxHashSet<&'a str>,
) {
    // Nested scopes are assembled into code objects before the containing scope.
    for constant in &code.constants {
        if let Constant::Code(code) = constant {
            collect_code_interning(
                code,
                recreated_frozensets,
                interned_strings,
                interned_constant_strings,
            );
        }
    }

    // _PyCode_New interns names before constants. Explicit import/fromlist hints identify constant
    // strings that are the exact same objects as those names, rather than merely equal values.
    if !code.marshal_hints.explicitly_interned_strings.is_empty() {
        let mut explicit_interns: Vec<_> = code
            .marshal_hints
            .explicitly_interned_strings
            .iter()
            .collect();
        explicit_interns.sort_unstable();
        for value in explicit_interns {
            interned_strings.insert(value);
            interned_constant_strings.insert(value);
        }
    }
    interned_strings.extend(code.names.iter().map(String::as_str));

    // CPython walks each code object's constants tuple backwards.
    for constant in code.constants.iter().rev() {
        intern_code_constant(
            constant,
            recreated_frozensets,
            interned_strings,
            interned_constant_strings,
        );
    }

    interned_strings.extend(code.locals.iter().map(String::as_str));
    interned_strings.insert(&code.filename);
    interned_strings.insert(&code.name);
    interned_strings.insert(&code.qualified_name);
}

/// Returns whether CPython replaces the constant while recursively interning code constants.
fn intern_code_constant<'a>(
    constant: &'a Constant,
    recreated_frozensets: &mut FxHashSet<usize>,
    interned_strings: &mut FxHashSet<&'a str>,
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
    interned_strings: &mut FxHashSet<&'a str>,
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

    let replaced = is_preinterned_string(value) || interned_strings.contains(value);
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
    fn metadata_string(&mut self, value: &'code str) {
        if self.graph.cache_scalars
            && let Some(character) = CachedObject::character(value)
        {
            self.graph
                .record_cached_value(OccurrenceKind::MetadataString, character);
        } else {
            self.graph
                .record_interned_string(OccurrenceKind::MetadataString, value);
        }
    }

    #[inline]
    fn first_line(&mut self, _value: u32) {}

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
            Constant::Int(value) if self.graph.cache_scalars && *value <= 256 => {
                self.graph.record_cached_value(
                    OccurrenceKind::Constant,
                    CachedObject::Int(u16::try_from(*value).expect("cached int fits")),
                )
            }
            Constant::SignedInt(value)
                if self.graph.cache_scalars && (-5..=256).contains(value) =>
            {
                self.graph.record_cached_value(
                    OccurrenceKind::Constant,
                    CachedObject::SignedInt(
                        u16::try_from(*value + 5).expect("cached signed int index fits"),
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

    #[inline]
    fn slice_member(&mut self, value: &'code Constant) {
        let address = std::ptr::from_ref(value).addr();
        self.graph
            .record_unique_value(OccurrenceKind::SliceMember, ObjectKey::SliceMember(address));
    }
}
