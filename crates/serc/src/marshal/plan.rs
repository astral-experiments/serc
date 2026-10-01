use ruff_index::newtype_index;

use crate::PythonVersion;
use crate::code_object::{CodeObject, Constant, MetadataId, MetadataName as Name};

use super::key::{
    FrozenSetMembers, frozen_set_members_in_wire_order, slice_member_uses_identity_for,
};

// Equal value keys share an ID; identity-based keys remain distinct exactly as `ObjectKey` does.
#[newtype_index]
pub(super) struct ObjectId;

/// Identifies an object in the serialized marshal reference table.
#[newtype_index]
pub(super) struct ReferenceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReferenceMode {
    IfShared,
    Forced,
}

/// Callbacks returning `bool` control whether traversal visits an object's children.
/// Returning `false` skips children when the object has already been handled as a reference.
pub(super) trait MarshalVisitor<'a> {
    const USES_METADATA_IDS: bool;

    fn python_version(&self) -> PythonVersion {
        PythonVersion::Py314
    }

    fn code(&mut self, code: &'a CodeObject, reference_mode: ReferenceMode) -> bool;
    fn code_bytes(&mut self, code: &'a CodeObject);
    /// Visits metadata tuple members in order.
    ///
    /// Compiler-owned IDs follow the same order and length as the metadata tuple.
    #[inline]
    fn metadata_names(
        &mut self,
        values: &'a [Name],
        metadata_ids: Option<&[MetadataId]>,
        _automatically_interned: bool,
    ) {
        if let Some(metadata_ids) = metadata_ids {
            assert_eq!(metadata_ids.len(), values.len(), "metadata ID tuple length");
            for (value, metadata_id) in values.iter().zip(metadata_ids) {
                self.metadata_name(value, Some(*metadata_id));
            }
        } else {
            for value in values {
                self.metadata_name(value, None);
            }
        }
    }

    #[expect(
        clippy::ptr_arg,
        reason = "marshal keys borrow immutable vector headers"
    )]
    fn constants(&mut self, values: &'a Vec<Constant>, reference_mode: ReferenceMode) -> bool;
    /// Visits the constants tuple with access to its cached hash before descending into members.
    fn code_constants(&mut self, code: &'a CodeObject) -> bool {
        self.constants(&code.constants, ReferenceMode::IfShared)
    }
    fn names(&mut self, code: &'a CodeObject) -> bool;
    fn locals(&mut self, code: &'a CodeObject) -> bool;
    fn local_kinds(&mut self, code: &'a CodeObject);
    fn metadata_string(&mut self, value: &'a str, metadata_id: Option<MetadataId>);
    /// Keeps inline metadata names encoded until a visitor needs their spelling.
    fn metadata_name(&mut self, value: &'a Name, metadata_id: Option<MetadataId>) {
        self.metadata_string(value, metadata_id);
    }
    fn code_filename(&mut self, code: &'a CodeObject, metadata_id: Option<MetadataId>);
    fn code_name(&mut self, code: &'a CodeObject, metadata_id: Option<MetadataId>) {
        self.metadata_string(&code.name, metadata_id);
    }
    fn code_qualified_name(&mut self, code: &'a CodeObject, metadata_id: Option<MetadataId>) {
        self.metadata_string(&code.qualified_name, metadata_id);
    }
    fn first_line(&mut self, value: u32);
    fn bytes(&mut self, value: &'a [u8]);
    /// Visits table bytes with an optional marshal-key hash; scalar identity rules still apply.
    fn bytes_with_hash(&mut self, value: &'a [u8], _hash: Option<u64>) {
        self.bytes(value);
    }
    fn constant(&mut self, value: &'a Constant, reference_mode: ReferenceMode) -> bool;

    // Typed scalar callbacks let the graph and writer use the traversal's existing dispatch.
    // Default visitors retain the complete constant callback and reference-mode contract.
    fn none_constant(&mut self, value: &'a Constant, mode: ReferenceMode) {
        self.constant(value, mode);
    }
    fn bool_constant(&mut self, value: &'a Constant, _boolean: bool, mode: ReferenceMode) {
        self.constant(value, mode);
    }
    fn ellipsis_constant(&mut self, value: &'a Constant, mode: ReferenceMode) {
        self.constant(value, mode);
    }
    fn integer_constant(
        &mut self,
        value: &'a Constant,
        _integer: u64,
        mode: ReferenceMode,
    ) -> bool {
        self.constant(value, mode)
    }
    fn string_constant(
        &mut self,
        value: &'a Constant,
        _string: &'a String,
        mode: ReferenceMode,
    ) -> bool {
        self.constant(value, mode)
    }
    fn signed_integer_constant(
        &mut self,
        value: &'a Constant,
        _integer: i64,
        mode: ReferenceMode,
    ) -> bool {
        self.constant(value, mode)
    }
    fn float_constant(&mut self, value: &'a Constant, _float: f64, mode: ReferenceMode) -> bool {
        self.constant(value, mode)
    }
    fn bytes_constant(
        &mut self,
        value: &'a Constant,
        _bytes: &'a [u8],
        mode: ReferenceMode,
    ) -> bool {
        self.constant(value, mode)
    }
    fn slice_member(&mut self, value: &'a Constant);
    fn cached_slice_string(&mut self, value: &'a Constant) {
        self.constant(value, ReferenceMode::IfShared);
    }

    fn frozen_set_members(&mut self, values: &'a [Constant]) -> FrozenSetMembers<'a> {
        frozen_set_members_in_wire_order(values)
    }
}

/// Traverses marshal values in their exact wire order.
///
/// Graph analysis and byte emission use the same recursion, while each visitor owns the work that
/// differs between the two passes.
#[inline]
pub(super) fn traverse_code<'a, V>(code: &'a CodeObject, visitor: &mut V)
where
    V: MarshalVisitor<'a>,
{
    traverse_code_with_mode(code, ReferenceMode::Forced, visitor);
}

#[inline]
pub(super) fn traverse_code_with_mode<'a, V>(
    code: &'a CodeObject,
    reference_mode: ReferenceMode,
    visitor: &mut V,
) where
    V: MarshalVisitor<'a>,
{
    if !visitor.code(code, reference_mode) {
        return;
    }
    visitor.code_bytes(code);
    if visitor.code_constants(code) {
        for value in &code.constants {
            traverse_constant(value, ReferenceMode::IfShared, visitor);
        }
    }
    let metadata_ids = V::USES_METADATA_IDS.then_some(&code.marshal_provenance.metadata_ids);
    if visitor.names(code) {
        visitor.metadata_names(
            &code.names,
            metadata_ids.map(|ids| ids.names.as_slice()),
            code.marshal_provenance
                .metadata_tuple_properties
                .is_some_and(|properties| properties.names.automatically_interned),
        );
    }
    if visitor.locals(code) {
        visitor.metadata_names(
            &code.locals,
            metadata_ids.map(|ids| ids.locals.as_slice()),
            code.marshal_provenance
                .metadata_tuple_properties
                .is_some_and(|properties| properties.locals.automatically_interned),
        );
    }
    visitor.local_kinds(code);
    visitor.code_filename(code, metadata_ids.map(|ids| ids.filename));
    visitor.code_name(code, metadata_ids.and_then(|ids| ids.name));
    visitor.code_qualified_name(code, metadata_ids.and_then(|ids| ids.qualified_name));
    visitor.first_line(code.first_line_number);
    let hashes = code.marshal_provenance.table_hashes;
    visitor.bytes_with_hash(&code.line_table, hashes.map(|hashes| hashes[0]));
    visitor.bytes_with_hash(&code.exception_table, hashes.map(|hashes| hashes[1]));
}

#[inline]
fn traverse_constants<'a, V>(
    values: &'a Vec<Constant>,
    reference_mode: ReferenceMode,
    visitor: &mut V,
) where
    V: MarshalVisitor<'a>,
{
    if visitor.constants(values, reference_mode) {
        for value in values {
            traverse_constant(value, ReferenceMode::IfShared, visitor);
        }
    }
}

#[expect(
    clippy::inline_always,
    reason = "merge container and scalar dispatch at the code object's constant loop"
)]
#[inline(always)]
fn traverse_constant<'a, V>(value: &'a Constant, reference_mode: ReferenceMode, visitor: &mut V)
where
    V: MarshalVisitor<'a>,
{
    match value {
        Constant::None => visitor.none_constant(value, reference_mode),
        Constant::Bool(boolean) => visitor.bool_constant(value, *boolean, reference_mode),
        Constant::Ellipsis => visitor.ellipsis_constant(value, reference_mode),
        Constant::Int(integer) => {
            visitor.integer_constant(value, *integer, reference_mode);
        }
        Constant::String(string) => {
            visitor.string_constant(value, string, reference_mode);
        }

        Constant::SignedInt(integer) => {
            visitor.signed_integer_constant(value, *integer, reference_mode);
        }
        Constant::Float(float) => {
            visitor.float_constant(value, *float, reference_mode);
        }
        Constant::Bytes(bytes) => {
            visitor.bytes_constant(value, bytes, reference_mode);
        }
        Constant::Tuple(values) => traverse_constants(values, reference_mode, visitor),
        Constant::Code(code) => {
            traverse_code_with_mode(code, ReferenceMode::IfShared, visitor);
        }
        Constant::FrozenSet(values) => {
            if visitor.constant(value, reference_mode) {
                for value in visitor.frozen_set_members(values) {
                    traverse_constant(value, ReferenceMode::Forced, visitor);
                }
            }
        }
        Constant::Slice { lower, upper, step } => {
            if visitor.constant(value, reference_mode) {
                traverse_slice_member(lower, visitor);
                traverse_slice_member(upper, visitor);
                traverse_slice_member(step, visitor);
            }
        }
        _ => {
            visitor.constant(value, reference_mode);
        }
    }
}

#[inline]
fn traverse_slice_member<'a, V>(value: &'a Constant, visitor: &mut V)
where
    V: MarshalVisitor<'a>,
{
    if slice_member_uses_identity_for(value, visitor.python_version())
        || matches!(value, Constant::Bytes(value) if value.is_empty())
    {
        visitor.slice_member(value);
    } else if matches!(value, Constant::String(_)) {
        visitor.cached_slice_string(value);
    } else {
        traverse_constant(value, ReferenceMode::IfShared, visitor);
    }
}
