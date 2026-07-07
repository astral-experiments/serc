use crate::compiler::{CodeObject, Constant};

use super::key::{
    ObjectKey, code_bytes_key, code_key, constant_key, constant_sort_key, constants_tuple_key,
    local_kinds_key, locals_tuple_key, should_intern, slice_member_uses_identity,
    strings_tuple_key,
};

// Equal value keys share an ID; identity-based keys remain distinct exactly as `ObjectKey` does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectId(pub(super) usize);

pub(super) struct PlannedObject<'a> {
    pub(super) key: ObjectKey<'a>,
    pub(super) interned: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum PlannedValue<'a> {
    Code(&'a CodeObject),
    CodeBytes(&'a CodeObject),
    Constants(&'a [Constant]),
    Strings(&'a [String]),
    Locals(&'a CodeObject),
    LocalKinds(&'a CodeObject),
    MetadataString(&'a str),
    FirstLine(u32),
    Bytes(&'a [u8]),
    Constant(&'a Constant),
    SliceMember(&'a Constant),
}

impl<'a> PlannedValue<'a> {
    pub(super) fn object(self) -> Option<PlannedObject<'a>> {
        let (key, interned) = match self {
            Self::Code(code) => (code_key(code), false),
            Self::CodeBytes(code) => (code_bytes_key(code), false),
            Self::Constants(values) => (constants_tuple_key(values), false),
            Self::Strings(values) => (strings_tuple_key(values), false),
            Self::Locals(code) => (locals_tuple_key(code), false),
            Self::LocalKinds(code) => (local_kinds_key(code), false),
            Self::MetadataString(value) => (ObjectKey::String(value), true),
            Self::FirstLine(_) => return None,
            Self::Bytes(value) => (ObjectKey::Bytes(value), false),
            Self::Constant(value) => {
                if matches!(
                    value,
                    Constant::None | Constant::Bool(_) | Constant::Ellipsis
                ) {
                    return None;
                }
                (
                    constant_key(value),
                    matches!(value, Constant::String(value) if should_intern(value)),
                )
            }
            Self::SliceMember(value) => {
                let address = std::ptr::from_ref(value).addr();
                (ObjectKey::SliceMember(address), false)
            }
        };
        Some(PlannedObject { key, interned })
    }
}

#[derive(Debug)]
pub(super) struct PlanEntry<'a> {
    pub(super) value: PlannedValue<'a>,
    pub(super) object: Option<ObjectId>,
    pub(super) force_reference: bool,
}

/// An immutable, flat preorder shared by graph analysis and byte emission.
///
/// Graph construction makes repeated objects leaf entries, so the writer can consume the plan
/// linearly without reconstructing the tree or maintaining a second occurrence cursor.
#[derive(Debug, Default)]
pub(super) struct MarshalPlan<'a> {
    entries: Vec<PlanEntry<'a>>,
}

impl<'a> MarshalPlan<'a> {
    pub(super) fn for_code<F>(code: &'a CodeObject, enter: F) -> Self
    where
        F: FnMut(PlannedValue<'a>, PlannedObject<'a>) -> (ObjectId, bool),
    {
        // Graph construction decides whether a value is the first occurrence. Repeated values
        // stay in the plan as reference-bearing leaves, matching the writer's early-return path.
        let mut builder = PlanBuilder {
            entries: Vec::new(),
            enter,
        };
        builder.push_code(code, true);
        Self {
            entries: builder.entries,
        }
    }

    pub(super) fn entries(&self) -> impl Iterator<Item = &PlanEntry<'a>> {
        self.entries.iter()
    }
}

struct PlanBuilder<'a, F> {
    entries: Vec<PlanEntry<'a>>,
    enter: F,
}

impl<'a, F> PlanBuilder<'a, F>
where
    F: FnMut(PlannedValue<'a>, PlannedObject<'a>) -> (ObjectId, bool),
{
    fn begin(&mut self, value: PlannedValue<'a>, force_reference: bool) -> bool {
        let (object, descend) = value.object().map_or((None, true), |object| {
            let (object, descend) = (self.enter)(value, object);
            (Some(object), descend)
        });
        self.entries.push(PlanEntry {
            value,
            object,
            force_reference,
        });
        descend
    }

    fn push_code(&mut self, code: &'a CodeObject, force_reference: bool) {
        if self.begin(PlannedValue::Code(code), force_reference) {
            self.push_leaf(PlannedValue::CodeBytes(code), true);
            self.push_constants(&code.constants, false);
            self.push_strings(&code.names, false);
            self.push_locals(code);
            self.push_leaf(PlannedValue::LocalKinds(code), false);
            self.push_leaf(PlannedValue::MetadataString(&code.filename), false);
            self.push_leaf(PlannedValue::MetadataString(&code.name), false);
            self.push_leaf(PlannedValue::MetadataString(&code.qualified_name), false);
            self.push_leaf(PlannedValue::FirstLine(code.first_line_number), false);
            self.push_leaf(PlannedValue::Bytes(&code.line_table), false);
            self.push_leaf(PlannedValue::Bytes(&code.exception_table), false);
        }
    }

    fn push_constants(&mut self, values: &'a [Constant], force_reference: bool) {
        if self.begin(PlannedValue::Constants(values), force_reference) {
            for value in values {
                self.push_constant(value, false);
            }
        }
    }

    fn push_strings(&mut self, values: &'a [String], force_reference: bool) {
        if self.begin(PlannedValue::Strings(values), force_reference) {
            for value in values {
                self.push_leaf(PlannedValue::MetadataString(value), false);
            }
        }
    }

    fn push_locals(&mut self, code: &'a CodeObject) {
        if self.begin(PlannedValue::Locals(code), false) {
            for value in &code.locals {
                self.push_leaf(PlannedValue::MetadataString(value), false);
            }
        }
    }

    fn push_constant(&mut self, value: &'a Constant, force_reference: bool) {
        match value {
            Constant::Tuple(values) => self.push_constants(values, force_reference),
            Constant::Code(code) => self.push_code(code, false),
            Constant::FrozenSet(values) => {
                if self.begin(PlannedValue::Constant(value), force_reference) {
                    let mut values: Vec<_> = values.iter().collect();
                    values.sort_by_key(|value| constant_sort_key(value));
                    for value in values {
                        self.push_constant(value, true);
                    }
                }
            }
            Constant::Slice { lower, upper, step } => {
                if self.begin(PlannedValue::Constant(value), force_reference) {
                    self.push_slice_member(lower);
                    self.push_slice_member(upper);
                    self.push_slice_member(step);
                }
            }
            _ => self.push_leaf(PlannedValue::Constant(value), force_reference),
        }
    }

    fn push_slice_member(&mut self, value: &'a Constant) {
        if slice_member_uses_identity(value) {
            self.push_leaf(PlannedValue::SliceMember(value), false);
        } else {
            self.push_constant(value, false);
        }
    }

    fn push_leaf(&mut self, value: PlannedValue<'a>, force_reference: bool) {
        self.begin(value, force_reference);
    }
}
