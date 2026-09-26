use std::borrow::Cow;
use std::fmt;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use ruff_index::newtype_index;
use ruff_python_ast::name::Name;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::CompileError;
use crate::target::code_flags::{CO_VARARGS, CO_VARKEYWORDS};
use crate::target::local_kinds::CO_FAST_LOCAL;

/// A Python filename, preserving lone surrogates that Rust strings cannot represent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodeFilename {
    Utf8(Arc<str>),
    Surrogate(Arc<[u8]>),
}

impl CodeFilename {
    pub(crate) fn from_path(path: &Path) -> Self {
        if let Some(value) = path.to_str() {
            return Self::from(value);
        }
        Self::Surrogate(Self::path_surrogate_bytes(path).into())
    }

    pub(crate) fn as_utf8(&self) -> Option<&str> {
        match self {
            Self::Utf8(value) => Some(value),
            Self::Surrogate(_) => None,
        }
    }

    #[cfg(not(windows))]
    fn path_surrogate_bytes(path: &Path) -> Vec<u8> {
        let mut remaining = path.as_os_str().as_encoded_bytes();
        let mut output = Vec::with_capacity(remaining.len());
        while let Err(error) = std::str::from_utf8(remaining) {
            let (valid, invalid) = remaining.split_at(error.valid_up_to());
            output.extend_from_slice(valid);
            let invalid_length = error.error_len().unwrap_or(invalid.len());
            let (invalid, rest) = invalid.split_at(invalid_length);
            for &byte in invalid {
                Self::push_surrogate(&mut output, 0xdc00 | u16::from(byte));
            }
            remaining = rest;
        }
        output.extend_from_slice(remaining);
        output
    }

    #[cfg(windows)]
    fn path_surrogate_bytes(path: &Path) -> Vec<u8> {
        use std::os::windows::ffi::OsStrExt;

        let mut output = Vec::new();
        for character in char::decode_utf16(path.as_os_str().encode_wide()) {
            match character {
                Ok(character) => {
                    output.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                }
                Err(error) => Self::push_surrogate(&mut output, error.unpaired_surrogate()),
            }
        }
        output
    }

    fn push_surrogate(output: &mut Vec<u8>, value: u16) {
        debug_assert!((0xd800..=0xdfff).contains(&value));
        output.extend_from_slice(&[
            0xe0 | u8::try_from(value >> 12).expect("UTF-8 leading surrogate bits fit in u8"),
            0x80 | u8::try_from((value >> 6) & 0x3f).expect("UTF-8 continuation bits fit in u8"),
            0x80 | u8::try_from(value & 0x3f).expect("UTF-8 continuation bits fit in u8"),
        ]);
    }

    #[cfg(test)]
    pub(crate) fn same_allocation(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Utf8(left), Self::Utf8(right)) => Arc::ptr_eq(left, right),
            (Self::Surrogate(left), Self::Surrogate(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl From<&str> for CodeFilename {
    fn from(value: &str) -> Self {
        Self::Utf8(Arc::from(value))
    }
}

/// An immutable metadata spelling that keeps short names inline.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub(crate) struct MetadataName(Name);

impl MetadataName {
    #[inline]
    pub(crate) fn new(value: &str) -> Self {
        Self(Name::new(value))
    }

    #[inline]
    pub(crate) const fn new_static(value: &'static str) -> Self {
        Self(Name::new_static(value))
    }

    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Deref for MetadataName {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for MetadataName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}

impl fmt::Display for MetadataName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), formatter)
    }
}

impl From<&str> for MetadataName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MetadataName {
    fn from(value: String) -> Self {
        Self(Name::from(value))
    }
}

impl From<Cow<'_, str>> for MetadataName {
    fn from(value: Cow<'_, str>) -> Self {
        Self(Name::from(value))
    }
}

impl PartialEq<str> for MetadataName {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for MetadataName {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for MetadataName {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum LocalsIdentity {
    #[default]
    ByValue,
    Fresh,
}

/// Identifies a string in a module's marshal metadata domain, including its filename.
#[newtype_index]
pub(crate) struct MetadataId;

#[cfg(test)]
impl Default for MetadataId {
    fn default() -> Self {
        Self::from_u32(0)
    }
}

/// Dense, module-wide name IDs that let marshal graph planning bypass repeated string lookups.
///
/// Every code object in a module shares the compiler's immutable name domain, with one additional
/// ID for the filename. IDs never affect wire order or code-constant equality.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub(crate) struct MarshalMetadataIds {
    pub(crate) domain_len: u32,
    pub(crate) names: Vec<MetadataId>,
    pub(crate) locals: Vec<MetadataId>,
    pub(crate) filename: MetadataId,
    pub(crate) name: Option<MetadataId>,
    pub(crate) qualified_name: Option<MetadataId>,
}

/// A constant-cache admission, external retention, or real code assembly in compilation order.
#[derive(Clone, Debug)]
pub(crate) enum CompilationInterningEvent {
    CachedTuple(Vec<String>),
    /// Complete additions whose canonical owners remain alive through serialization.
    ExternalRetain(Vec<String>),
    /// These canonical owners remain retained; other additions are unknown.
    PartialExternalRetain(Vec<String>),
    /// Additional retained canonical strings may have appeared at this boundary.
    UnknownExternalAdditions,
    CodeAssembly(CodeInterningSummary),
}

/// The string effects of one assembled code object, without retaining its bytecode or children.
#[derive(Clone, Debug)]
pub(crate) struct CodeInterningSummary {
    pub(crate) names: Vec<MetadataName>,
    pub(crate) locals: Vec<MetadataName>,
    pub(crate) constant_strings: Vec<String>,
    pub(crate) explicitly_interned_strings: FxHashSet<String>,
    pub(crate) constants_tuple: Option<Vec<String>>,
    pub(crate) filename: CodeFilename,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    /// Class scalar cache ownership, before assembly consults external intern additions.
    pub(crate) qualified_name_is_canonical: Option<bool>,
}

impl CodeInterningSummary {
    pub(crate) fn from_code(code: &CodeObject) -> Self {
        fn collect_strings(constant: &Constant, strings: &mut Vec<String>) {
            match constant {
                Constant::String(value) => strings.push(value.clone()),
                Constant::Tuple(values) => {
                    for value in values.iter().rev() {
                        collect_strings(value, strings);
                    }
                }
                Constant::FrozenSet(values) => {
                    for value in values {
                        collect_strings(value, strings);
                    }
                }
                // Children have their own assembly events; slices do not intern their fields.
                _ => {}
            }
        }

        let mut constant_strings = Vec::new();
        for constant in code.constants.iter().rev() {
            collect_strings(constant, &mut constant_strings);
        }
        Self {
            names: code.names.clone(),
            locals: code.locals.clone(),
            constant_strings,
            explicitly_interned_strings: code
                .marshal_provenance
                .explicitly_interned_strings
                .clone(),
            constants_tuple: code
                .constants
                .iter()
                .map(|constant| match constant {
                    Constant::String(value) => Some(value.clone()),
                    _ => None,
                })
                .collect(),
            filename: code.filename.clone(),
            name: code.name.clone(),
            qualified_name: code.qualified_name.clone(),
            qualified_name_is_canonical: code.marshal_provenance.qualified_name_is_canonical,
        }
    }
}

/// Non-wire compiler provenance needed to reproduce CPython's marshal object identities.
///
/// Each nested code object retains its own provenance because marshal graph planning happens after
/// the compiler has assembled the complete constant tree.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub(crate) struct MarshalProvenance {
    pub(crate) assembly_order: Option<usize>,
    pub(crate) interning_events: Vec<CompilationInterningEvent>,
    /// Non-ASCII metadata tuples interned during assembly, including discarded code.
    pub(crate) interned_metadata_tuples: Vec<Vec<String>>,
    pub(crate) locals_identity: LocalsIdentity,
    pub(crate) explicitly_interned_strings: FxHashSet<String>,
    /// Parser-interned names used as constants, including names absent from emitted tables.
    pub(crate) parser_interned_strings: FxHashSet<String>,
    /// Fresh names produced by private-name mangling, interned only during code assembly.
    pub(crate) generated_mangled_strings: FxHashSet<String>,
    /// Strings whose first compiler-cache object was already interned.
    pub(crate) canonical_constant_strings: FxHashSet<String>,
    /// The class-body constant loaded into `__qualname__`, when present.
    pub(crate) qualified_name_constant_index: Option<u32>,
    /// Qualified-name ownership recorded before discarded constants are pruned.
    pub(crate) qualified_name_is_canonical: Option<bool>,
    /// Whether `%` first entered the compiler cache as a fresh `PyUnicode_Replace` result.
    pub(crate) uncached_percent_constant: bool,
    pub(crate) metadata_ids: MarshalMetadataIds,
    /// Value properties computed for the emitted metadata; absent for hand-built code objects.
    pub(crate) metadata_tuple_properties: Option<MarshalMetadataTupleProperties>,
    /// Whether this code tree can contain a frozenset constant.
    pub(crate) subtree_has_frozenset_constants: bool,
    /// Whether no surviving constant tuple can share a non-identifier string with code metadata.
    pub(crate) metadata_alias_scan_unnecessary: bool,
    /// Whether the constant cache first admitted a fresh empty bytes object.
    pub(crate) fresh_empty_bytes_constant: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MarshalMetadataTupleProperties {
    pub(crate) names: MarshalStringTupleProperties,
    pub(crate) locals: MarshalStringTupleProperties,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MarshalStringTupleProperties {
    pub(crate) hash: u64,
    pub(crate) automatically_interned: bool,
}

impl MarshalProvenance {
    /// Compares the subset of marshal provenance that participates in code-constant equality.
    pub(crate) fn equal_for_constant_pool(&self, other: &Self) -> bool {
        // CPython gives annotation-scope locals fresh identity, which distinguishes otherwise
        // equal code constants. String origins, qualified-name origins, metadata IDs and tuple
        // properties, frozenset and metadata-alias hints, and assembly events only affect graph
        // planning and are omitted from constant-pool equality.
        self.locals_identity == other.locals_identity
    }
}

/// Identifies one native allocation without changing its serialized numeric value.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum NonReflexiveKey {
    Source(u64),
    Allocated(usize),
}

#[derive(Clone, Debug)]
enum NonReflexiveIdentity {
    Source(u64),
    Allocated(Arc<()>),
}

#[derive(Clone, Copy, Debug)]
enum NonReflexiveValue {
    Float(f64),
    Complex { real: f64, imag: f64 },
}

/// A NaN-bearing number whose identity survives container copies.
#[derive(Clone, Debug)]
pub(crate) struct NonReflexiveNumber {
    value: NonReflexiveValue,
    identity: NonReflexiveIdentity,
}

pub(crate) type NonReflexiveRemap = FxHashMap<NonReflexiveKey, Arc<()>>;

impl NonReflexiveNumber {
    pub(crate) fn key(&self) -> NonReflexiveKey {
        match &self.identity {
            NonReflexiveIdentity::Source(origin) => NonReflexiveKey::Source(*origin),
            NonReflexiveIdentity::Allocated(owner) => {
                NonReflexiveKey::Allocated(Arc::as_ptr(owner).addr())
            }
        }
    }

    pub(crate) fn value(&self) -> Constant {
        match self.value {
            NonReflexiveValue::Float(value) => Constant::Float(value),
            NonReflexiveValue::Complex { real, imag } => Constant::Complex { real, imag },
        }
    }

    fn reallocate(&mut self, remap: &mut NonReflexiveRemap) {
        let owner = remap.entry(self.key()).or_insert_with(|| Arc::new(()));
        self.identity = NonReflexiveIdentity::Allocated(Arc::clone(owner));
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Constant {
    None,
    Bool(bool),
    Ellipsis,
    Int(u64),
    SignedInt(i64),
    BigInt {
        negative: bool,
        digits: Vec<u16>,
    },
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    NonReflexive(Box<NonReflexiveNumber>),
    String(String),
    SurrogateString(Vec<u8>),
    Bytes(Vec<u8>),
    Tuple(Vec<Self>),
    FrozenSet(Vec<Self>),
    Slice {
        lower: Box<Self>,
        upper: Box<Self>,
        step: Box<Self>,
    },
    Code(Box<CodeObject>),
}

impl Constant {
    /// Returns the Python truth value of this constant.
    pub(crate) fn is_truthy(&self) -> bool {
        match self {
            Self::None => false,
            Self::Bool(value) => *value,
            Self::Ellipsis | Self::Slice { .. } | Self::Code(_) => true,
            Self::Int(value) => *value != 0,
            Self::SignedInt(value) => *value != 0,
            Self::BigInt { digits, .. } => !digits.is_empty(),
            Self::NonReflexive(_) => true,
            Self::Float(value) => *value != 0.0,
            Self::Complex { real, imag } => *real != 0.0 || *imag != 0.0,
            Self::String(value) => !value.is_empty(),
            Self::SurrogateString(value) | Self::Bytes(value) => !value.is_empty(),
            Self::Tuple(value) | Self::FrozenSet(value) => !value.is_empty(),
        }
    }

    /// Records the producing AST operation while pure fold analyses recompute its value.
    pub(crate) fn with_non_reflexive_origin(self, origin: u64) -> Self {
        let value = match self {
            Self::Float(value) if value.is_nan() => NonReflexiveValue::Float(value),
            Self::Complex { real, imag } if real.is_nan() || imag.is_nan() => {
                NonReflexiveValue::Complex { real, imag }
            }
            other => return other,
        };
        Self::NonReflexive(Box::new(NonReflexiveNumber {
            value,
            identity: NonReflexiveIdentity::Source(origin),
        }))
    }

    /// Returns numeric operands without the allocation identity consumed by an operation.
    pub(crate) fn into_numeric_value(self) -> Self {
        match self {
            Self::NonReflexive(number) => number.value(),
            other => other,
        }
    }

    pub(crate) fn has_non_reflexive_number(&self) -> bool {
        match self {
            Self::NonReflexive(_) => true,
            Self::Tuple(values) | Self::FrozenSet(values) => {
                values.iter().any(Self::has_non_reflexive_number)
            }
            Self::Slice { lower, upper, step } => [lower, upper, step]
                .into_iter()
                .any(|value| value.has_non_reflexive_number()),
            // A copied code constant reuses its already compiled numeric objects.
            _ => false,
        }
    }

    pub(crate) fn has_source_non_reflexive_number(&self) -> bool {
        match self {
            Self::NonReflexive(number) => {
                matches!(number.identity, NonReflexiveIdentity::Source(_))
            }
            Self::Tuple(values) | Self::FrozenSet(values) => {
                values.iter().any(Self::has_source_non_reflexive_number)
            }
            Self::Slice { lower, upper, step } => [lower, upper, step]
                .into_iter()
                .any(|value| value.has_source_non_reflexive_number()),
            _ => false,
        }
    }

    /// Rebinds one native fold event, preserving aliases throughout its result graph.
    pub(crate) fn reallocate_non_reflexive_numbers(&mut self, remap: &mut NonReflexiveRemap) {
        match self {
            Self::NonReflexive(number) => number.reallocate(remap),
            Self::Tuple(values) | Self::FrozenSet(values) => {
                for value in values {
                    value.reallocate_non_reflexive_numbers(remap);
                }
            }
            Self::Slice { lower, upper, step } => {
                for value in [lower, upper, step] {
                    value.reallocate_non_reflexive_numbers(remap);
                }
            }
            _ => {}
        }
    }

    /// Applies Python numeric negation, leaving nonnumeric operands to raise at runtime.
    pub(crate) fn negated(self) -> Option<Self> {
        match self {
            Self::Bool(false) | Self::Int(0) => Some(Self::Int(0)),
            Self::Bool(true) => Some(Self::SignedInt(-1)),
            Self::Int(value) => Some(match value.cmp(&i64::MIN.unsigned_abs()) {
                std::cmp::Ordering::Less => Self::SignedInt(
                    -i64::try_from(value).expect("magnitude below 2^63 fits in i64"),
                ),
                std::cmp::Ordering::Equal => Self::SignedInt(i64::MIN),
                std::cmp::Ordering::Greater => {
                    let mut magnitude = value;
                    let mut digits = Vec::new();
                    while magnitude != 0 {
                        digits.push((magnitude & 0x7fff) as u16);
                        magnitude >>= 15;
                    }
                    Self::BigInt {
                        negative: true,
                        digits,
                    }
                }
            }),
            Self::SignedInt(value) => Some(if value <= 0 {
                Self::Int(value.unsigned_abs())
            } else {
                Self::SignedInt(-value)
            }),
            Self::BigInt { negative, digits } => {
                if negative
                    && let Some(value) = digits.iter().rev().try_fold(0_u64, |value, digit| {
                        value.checked_mul(1 << 15)?.checked_add(u64::from(*digit))
                    })
                {
                    Some(Self::Int(value))
                } else {
                    Some(Self::BigInt {
                        negative: !negative,
                        digits,
                    })
                }
            }
            Self::NonReflexive(number) => {
                let mut result = number.value().negated()?.with_non_reflexive_origin(0);
                result.reallocate_non_reflexive_numbers(&mut NonReflexiveRemap::default());
                Some(result)
            }
            Self::Float(value) => Some(Self::Float(-value)),
            Self::Complex { real, imag } => Some(Self::Complex {
                real: -real,
                imag: -imag,
            }),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CodeObject {
    pub(crate) arg_count: u32,
    pub(crate) positional_only_arg_count: u32,
    pub(crate) keyword_only_arg_count: u32,
    pub(crate) stack_size: u32,
    pub(crate) flags: u32,
    pub(crate) bytecode: Vec<u8>,
    pub(crate) constants: Vec<Constant>,
    pub(crate) names: Vec<MetadataName>,
    pub(crate) locals: Vec<MetadataName>,
    pub(crate) local_kinds: Vec<u8>,
    /// The caller's immutable filename, shared by all scopes in the compilation.
    pub(crate) filename: CodeFilename,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) first_line_number: u32,
    pub(crate) line_table: Vec<u8>,
    pub(crate) exception_table: Vec<u8>,
    /// Identity provenance consumed by marshal graph planning, not a serialized code-object field.
    pub(crate) marshal_provenance: MarshalProvenance,
}

impl CodeObject {
    /// Checks the model invariants required before a code object can escape the compiler.
    pub(crate) fn validate(&self) -> Result<(), CompileError> {
        let invalid = |message| {
            CompileError::Internal(format!(
                "invalid code object `{}`: {message}",
                self.qualified_name
            ))
        };
        // Marshal writes these as 32-bit words that CPython reads into signed integer fields.
        for (field, value) in [
            ("argument count", self.arg_count),
            (
                "positional-only argument count",
                self.positional_only_arg_count,
            ),
            ("keyword-only argument count", self.keyword_only_arg_count),
            ("stack size", self.stack_size),
            ("flags", self.flags),
        ] {
            if value > i32::MAX as u32 {
                return Err(invalid(format!(
                    "{field} {value} exceeds CPython's signed 32-bit limit"
                )));
            }
        }
        if let Some(message) = bytecode_length_error(self.bytecode.len()) {
            return Err(invalid(message));
        }
        if self.locals.len() != self.local_kinds.len() {
            return Err(invalid(format!(
                "locals metadata has {} names but {} kind entries",
                self.locals.len(),
                self.local_kinds.len()
            )));
        }
        if self.positional_only_arg_count > self.arg_count {
            return Err(invalid(format!(
                "declares {} positional-only arguments but only {} total positional arguments",
                self.positional_only_arg_count, self.arg_count
            )));
        }
        let parameter_count = u64::from(self.arg_count)
            + u64::from(self.keyword_only_arg_count)
            + u64::from(self.flags & CO_VARARGS != 0)
            + u64::from(self.flags & CO_VARKEYWORDS != 0);
        // CPython counts parameter capacity from local-variable entries, excluding cell-only and
        // free-variable entries in the parallel locals-plus arrays.
        let local_slots = self
            .local_kinds
            .iter()
            .filter(|kind| **kind & CO_FAST_LOCAL != 0)
            .count();
        if usize::try_from(parameter_count)
            .ok()
            .is_none_or(|count| count > local_slots)
        {
            return Err(invalid(format!(
                "declares {parameter_count} parameter slots but only {local_slots} local-variable slots"
            )));
        }
        Ok(())
    }
}

fn bytecode_length_error(bytes: usize) -> Option<String> {
    if bytes > i32::MAX as usize {
        Some(format!(
            "bytecode length {bytes} exceeds CPython's signed 32-bit limit"
        ))
    } else if !bytes.is_multiple_of(2) {
        Some(format!(
            "bytecode length {bytes} is not divisible by CPython's two-byte code unit"
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CodeObject, Constant, LocalsIdentity, MarshalMetadataIds, MarshalProvenance, MetadataId,
        bytecode_length_error,
    };
    use crate::CompileError;
    use crate::target::local_kinds::CO_FAST_FREE;

    fn code_object() -> CodeObject {
        CodeObject {
            arg_count: 0,
            positional_only_arg_count: 0,
            keyword_only_arg_count: 0,
            stack_size: 1,
            flags: 0,
            bytecode: Vec::new(),
            constants: vec![Constant::None],
            names: Vec::new(),
            locals: Vec::new(),
            local_kinds: Vec::new(),
            filename: "module.py".into(),
            name: "<module>".to_string(),
            qualified_name: "<module>".to_string(),
            first_line_number: 1,
            line_table: Vec::new(),
            exception_table: Vec::new(),
            marshal_provenance: MarshalProvenance {
                metadata_ids: MarshalMetadataIds {
                    domain_len: 1,
                    ..MarshalMetadataIds::default()
                },
                ..MarshalProvenance::default()
            },
        }
    }

    #[test]
    fn accepts_a_valid_code_object() {
        code_object().validate().unwrap();
    }

    #[test]
    fn marshal_metadata_ids_do_not_participate_in_constant_pool_equality() {
        let first = MarshalProvenance::default();
        let mut second = MarshalProvenance {
            metadata_ids: MarshalMetadataIds {
                domain_len: 3,
                names: vec![MetadataId::from_u32(0), MetadataId::from_u32(1)],
                locals: vec![MetadataId::from_u32(1)],
                filename: MetadataId::from_u32(2),
                name: Some(MetadataId::from_u32(0)),
                qualified_name: Some(MetadataId::from_u32(1)),
            },
            metadata_tuple_properties: Some(crate::marshal::metadata_tuple_properties(&[], &[])),
            subtree_has_frozenset_constants: true,
            metadata_alias_scan_unnecessary: true,
            ..MarshalProvenance::default()
        };

        assert!(first.equal_for_constant_pool(&second));
        second.locals_identity = LocalsIdentity::Fresh;
        assert!(!first.equal_for_constant_pool(&second));
    }

    #[test]
    fn rejects_misaligned_bytecode_through_the_internal_error_path() {
        let mut code = code_object();
        code.bytecode.push(0);

        assert_eq!(
            code.validate().unwrap_err(),
            CompileError::Internal(
                "invalid code object `<module>`: bytecode length 1 is not divisible by CPython's two-byte code unit"
                    .to_string()
            )
        );
    }

    #[test]
    fn rejects_bytecode_larger_than_cpython_can_index() {
        let bytes = i32::MAX as usize + 1;
        assert_eq!(
            bytecode_length_error(bytes),
            Some(format!(
                "bytecode length {bytes} exceeds CPython's signed 32-bit limit"
            ))
        );
    }

    #[test]
    fn rejects_code_fields_that_marshal_as_negative() {
        let value = i32::MAX as u32 + 1;
        let invalid = [
            ("argument count", {
                let mut code = code_object();
                code.arg_count = value;
                code
            }),
            ("positional-only argument count", {
                let mut code = code_object();
                code.positional_only_arg_count = value;
                code
            }),
            ("keyword-only argument count", {
                let mut code = code_object();
                code.keyword_only_arg_count = value;
                code
            }),
            ("stack size", {
                let mut code = code_object();
                code.stack_size = value;
                code
            }),
            ("flags", {
                let mut code = code_object();
                code.flags = value;
                code
            }),
        ];

        for (field, code) in invalid {
            assert_eq!(
                code.validate().unwrap_err(),
                CompileError::Internal(format!(
                    "invalid code object `<module>`: {field} {value} exceeds CPython's signed 32-bit limit"
                ))
            );
        }
    }

    #[test]
    fn rejects_mismatched_locals_metadata() {
        let mut code = code_object();
        code.locals.push("value".into());

        assert_eq!(
            code.validate().unwrap_err(),
            CompileError::Internal(
                "invalid code object `<module>`: locals metadata has 1 names but 0 kind entries"
                    .to_string()
            )
        );
    }

    #[test]
    fn rejects_positional_only_arguments_outside_the_positional_count() {
        let mut code = code_object();
        code.positional_only_arg_count = 1;

        assert_eq!(
            code.validate().unwrap_err(),
            CompileError::Internal(
                "invalid code object `<module>`: declares 1 positional-only arguments but only 0 total positional arguments".to_string()
            )
        );
    }

    #[test]
    fn rejects_parameters_without_local_slots() {
        let mut code = code_object();
        code.arg_count = 1;
        code.locals.push("captured".into());
        code.local_kinds.push(CO_FAST_FREE);

        assert_eq!(
            code.validate().unwrap_err(),
            CompileError::Internal(
                "invalid code object `<module>`: declares 1 parameter slots but only 0 local-variable slots".to_string()
            )
        );
    }
}
