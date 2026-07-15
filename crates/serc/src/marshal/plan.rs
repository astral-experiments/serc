use crate::code_object::{CodeObject, Constant};

use super::key::{frozen_set_members_in_wire_order, slice_member_uses_identity};

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

pub(super) trait MarshalVisitor<'a> {
    const USES_METADATA_IDS: bool;

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
fn traverse_code_with_mode<'a, V>(
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
    let metadata_ids = if V::USES_METADATA_IDS {
        code.marshal_provenance.metadata_ids.as_ref()
    } else {
        None
    };
    traverse_strings(
        &code.names,
        metadata_ids.map(|ids| ids.names.as_slice()),
        visitor,
    );
    traverse_locals(code, metadata_ids.map(|ids| ids.locals.as_slice()), visitor);
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

#[inline]
fn traverse_strings<'a, V>(values: &'a [String], metadata_ids: Option<&[u32]>, visitor: &mut V)
where
    V: MarshalVisitor<'a>,
{
    if visitor.strings(values) {
        if let Some(metadata_ids) = metadata_ids.filter(|ids| ids.len() == values.len()) {
            for (value, metadata_id) in values.iter().zip(metadata_ids) {
                visitor.metadata_string(value, Some(*metadata_id));
            }
        } else {
            for value in values {
                visitor.metadata_string(value, None);
            }
        }
    }
}

#[inline]
fn traverse_locals<'a, V>(code: &'a CodeObject, metadata_ids: Option<&[u32]>, visitor: &mut V)
where
    V: MarshalVisitor<'a>,
{
    if visitor.locals(code) {
        if let Some(metadata_ids) = metadata_ids.filter(|ids| ids.len() == code.locals.len()) {
            for (value, metadata_id) in code.locals.iter().zip(metadata_ids) {
                visitor.metadata_string(value, Some(*metadata_id));
            }
        } else {
            for value in &code.locals {
                visitor.metadata_string(value, None);
            }
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
                for (_, value) in frozen_set_members_in_wire_order(values) {
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
    if slice_member_uses_identity(value) {
        visitor.slice_member(value);
    } else {
        traverse_constant(value, ReferenceMode::IfShared, visitor);
    }
}
