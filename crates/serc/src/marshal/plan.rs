use crate::PythonVersion;
use crate::code_object::{CodeObject, Constant};

use super::key::{
    FrozenSetMembers, frozen_set_members_in_wire_order, slice_member_uses_identity_for,
};

// Equal value keys share an ID; identity-based keys remain distinct exactly as `ObjectKey` does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectId(u32);

impl ObjectId {
    pub(super) fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("marshal object limit exceeded"))
    }

    pub(super) fn index(self) -> usize {
        self.0 as usize
    }
}

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
    fn constants(&mut self, values: &'a [Constant], reference_mode: ReferenceMode) -> bool;
    fn strings(&mut self, values: &'a [String]) -> bool;
    fn locals(&mut self, code: &'a CodeObject) -> bool;
    fn local_kinds(&mut self, code: &'a CodeObject);
    fn metadata_string(&mut self, value: &'a str, metadata_id: Option<u32>);
    fn first_line(&mut self, value: u32);
    fn bytes(&mut self, value: &'a [u8]);
    fn constant(&mut self, value: &'a Constant, reference_mode: ReferenceMode) -> bool;
    fn slice_member(&mut self, value: &'a Constant);

    fn frozen_set_members(&self, values: &'a [Constant]) -> FrozenSetMembers<'a> {
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
    traverse_constants(&code.constants, ReferenceMode::IfShared, visitor);
    let metadata_ids = V::USES_METADATA_IDS.then_some(&code.marshal_provenance.metadata_ids);
    if visitor.strings(&code.names) {
        traverse_metadata_strings(
            &code.names,
            metadata_ids.map(|ids| ids.names.as_slice()),
            visitor,
        );
    }
    if visitor.locals(code) {
        traverse_metadata_strings(
            &code.locals,
            metadata_ids.map(|ids| ids.locals.as_slice()),
            visitor,
        );
    }
    visitor.local_kinds(code);
    visitor.metadata_string(&code.filename, metadata_ids.map(|ids| ids.filename));
    visitor.metadata_string(&code.name, metadata_ids.and_then(|ids| ids.name));
    visitor.metadata_string(
        &code.qualified_name,
        metadata_ids.and_then(|ids| ids.qualified_name),
    );
    visitor.first_line(code.first_line_number);
    visitor.bytes(&code.line_table);
    visitor.bytes(&code.exception_table);
}

#[inline]
fn traverse_constants<'a, V>(values: &'a [Constant], reference_mode: ReferenceMode, visitor: &mut V)
where
    V: MarshalVisitor<'a>,
{
    if visitor.constants(values, reference_mode) {
        for value in values {
            traverse_constant(value, ReferenceMode::IfShared, visitor);
        }
    }
}

/// Visits metadata tuple members in order.
///
/// Compiler-owned IDs follow the same order and length as the metadata tuple.
#[inline]
fn traverse_metadata_strings<'a, V>(
    values: &'a [String],
    metadata_ids: Option<&[u32]>,
    visitor: &mut V,
) where
    V: MarshalVisitor<'a>,
{
    if let Some(metadata_ids) = metadata_ids {
        assert_eq!(metadata_ids.len(), values.len(), "metadata ID tuple length");
        for (value, metadata_id) in values.iter().zip(metadata_ids) {
            visitor.metadata_string(value, Some(*metadata_id));
        }
    } else {
        for value in values {
            visitor.metadata_string(value, None);
        }
    }
}

#[inline]
fn traverse_constant<'a, V>(value: &'a Constant, reference_mode: ReferenceMode, visitor: &mut V)
where
    V: MarshalVisitor<'a>,
{
    match value {
        Constant::Tuple(values) => traverse_constants(values, reference_mode, visitor),
        Constant::Code(code) => {
            traverse_code_with_mode(code, ReferenceMode::IfShared, visitor);
        }
        Constant::FrozenSet(values) => {
            if visitor.constant(value, reference_mode) {
                for (_, value) in visitor.frozen_set_members(values) {
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
    if slice_member_uses_identity_for(value, visitor.python_version()) {
        visitor.slice_member(value);
    } else {
        traverse_constant(value, ReferenceMode::IfShared, visitor);
    }
}
