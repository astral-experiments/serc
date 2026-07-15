use crate::code_object::{CodeObject, Constant, LocalsIdentity, MarshalProvenance};

use super::encode_code;
use super::graph::{ObjectGraph, OccurrenceKind};
use super::key::{
    ObjectKey, code_bytes_key, code_key, constant_key, constant_sort_key, constants_tuple_key,
    local_kinds_key, locals_tuple_key, slice_member_uses_identity, strings_tuple_key,
};
use super::plan::{MarshalVisitor, ReferenceMode, traverse_code};

fn code_object(constants: Vec<Constant>) -> CodeObject {
    CodeObject {
        arg_count: 0,
        positional_only_arg_count: 0,
        keyword_only_arg_count: 0,
        stack_size: 1,
        flags: 0,
        bytecode: Vec::new(),
        constants,
        names: Vec::new(),
        locals: Vec::new(),
        local_kinds: Vec::new(),
        filename: "module.py".to_string(),
        name: "<module>".to_string(),
        qualified_name: "<module>".to_string(),
        first_line_number: 1,
        line_table: Vec::new(),
        exception_table: Vec::new(),
        marshal_provenance: MarshalProvenance::default(),
    }
}

#[test]
#[should_panic(expected = "marshal occurrence 0 mismatch")]
fn occurrence_cursor_rejects_a_mismatched_kind() {
    let code = code_object(Vec::new());
    let graph = ObjectGraph::for_code(&code);

    graph
        .occurrence_cursor()
        .consume(OccurrenceKind::MetadataString);
}

#[test]
#[should_panic(expected = "marshal writer requested occurrence 0")]
fn occurrence_cursor_rejects_over_consumption() {
    let graph = ObjectGraph::default();

    graph.occurrence_cursor().consume(OccurrenceKind::Code);
}

#[test]
#[should_panic(expected = "marshal writer consumed 0 of")]
fn occurrence_cursor_rejects_under_consumption() {
    let code = code_object(Vec::new());
    let graph = ObjectGraph::for_code(&code);

    graph.occurrence_cursor().finish();
}

#[test]
fn counts_repeated_objects_without_reexpanding_them() {
    let mut graph = ObjectGraph::default();
    let key = ObjectKey::Float(1.0_f64.to_bits());

    assert!(graph.record(key.clone()));
    assert!(!graph.is_shared(&key, false));
    assert!(!graph.record(key.clone()));
    assert!(graph.is_shared(&key, false));
}

#[test]
fn uses_cpython_reference_cache_boundaries() {
    let graph = ObjectGraph::default();
    for key in [
        ObjectKey::Int(256),
        ObjectKey::SignedInt(-5),
        ObjectKey::Bytes(&[]),
        ObjectKey::Bytes(&[0]),
        ObjectKey::String("x"),
        ObjectKey::String("\u{80}"),
        ObjectKey::String("é"),
        ObjectKey::String("ÿ"),
        ObjectKey::EmptyTuple,
    ] {
        assert!(graph.is_shared(&key, false), "expected cached key {key:?}");
    }
    for key in [
        ObjectKey::Int(257),
        ObjectKey::SignedInt(-6),
        ObjectKey::Bytes(&[0, 1]),
        ObjectKey::String("éx"),
        ObjectKey::String("Ā"),
        ObjectKey::Float(1.0_f64.to_bits()),
    ] {
        assert!(
            !graph.is_shared(&key, false),
            "unexpected cached key {key:?}"
        );
    }
    assert!(graph.is_shared(&ObjectKey::Float(0), true));
}

#[test]
fn bypasses_value_lookup_for_common_cpython_cached_objects() {
    let mut graph = ObjectGraph::default();
    for key in [
        ObjectKey::Bytes(&[]),
        ObjectKey::EmptyTuple,
        ObjectKey::Int(256),
        ObjectKey::SignedInt(-5),
        ObjectKey::Bytes(&[0]),
        ObjectKey::String("é"),
    ] {
        assert!(graph.record(key.clone()));
        assert!(!graph.record(key));
    }

    assert_eq!(graph.object_count(), 6);
    assert_eq!(graph.keyed_object_count(), 6);
    assert_eq!(graph.hashed_object_count(), 0);
}

#[test]
fn cached_scalar_slots_preserve_domains_and_cross_source_identity() {
    let mut graph = ObjectGraph::default();
    let first_character = String::from("é");
    let second_character = String::from("é");
    let first_byte = [0];
    let second_byte = [0];

    assert!(graph.record(ObjectKey::Int(1)));
    assert!(graph.record(ObjectKey::SignedInt(1)));
    assert!(!graph.record(ObjectKey::Int(1)));
    assert!(!graph.record(ObjectKey::SignedInt(1)));
    assert!(graph.record(ObjectKey::Bytes(&first_byte)));
    assert!(!graph.record(ObjectKey::Bytes(&second_byte)));
    assert!(graph.record(ObjectKey::String(&first_character)));
    assert!(!graph.record(ObjectKey::String(&second_character)));
    assert_eq!(graph.object_count(), 4);
    assert_eq!(graph.keyed_object_count(), 4);
    assert_eq!(graph.hashed_object_count(), 0);
}

#[test]
fn cached_scalar_graph_routes_constants_metadata_and_structural_bytes() {
    let mut code = code_object(vec![
        Constant::Int(256),
        Constant::SignedInt(-5),
        Constant::Bytes(vec![0]),
        Constant::String("é".to_string()),
    ]);
    code.constants.resize(32, Constant::None);
    code.names.push("é".to_string());
    code.line_table.push(0);
    code.exception_table.push(0);
    let graph = ObjectGraph::for_code(&code);

    assert!(graph.is_interned("é"));
    assert!(graph.is_shared(&ObjectKey::Int(256), false));
    assert!(graph.is_shared(&ObjectKey::SignedInt(-5), false));
    assert!(graph.is_shared(&ObjectKey::Bytes(&[0]), false));
    assert!(graph.is_shared(&ObjectKey::String("é"), false));
}

#[test]
fn tracks_interned_strings_from_constants_and_metadata() {
    let mut code = code_object(vec![
        Constant::String("identifier".to_string()),
        Constant::String("méthode".to_string()),
        Constant::String("not interned".to_string()),
    ]);
    code.marshal_provenance
        .explicitly_interned_strings
        .insert("méthode".to_string());
    code.names.push("metadata name with spaces".to_string());
    let graph = ObjectGraph::for_code(&code);

    assert!(graph.is_interned("identifier"));
    assert!(graph.is_interned("méthode"));
    assert!(graph.is_interned("metadata name with spaces"));
    assert!(graph.is_interned("module.py"));
    assert!(!graph.is_interned("not interned"));
    assert!(graph.is_shared(&ObjectKey::String("identifier"), false));
    assert!(graph.is_shared(&ObjectKey::String("méthode"), false));
    assert!(graph.is_shared(&ObjectKey::String("metadata name with spaces"), false));
}

#[test]
fn keeps_identity_and_value_keys_separate() {
    let mut first = code_object(Vec::new());
    let mut second = first.clone();
    assert_ne!(code_key(&first), code_key(&second));
    assert_ne!(code_bytes_key(&first), code_bytes_key(&second));

    assert_eq!(local_kinds_key(&first), local_kinds_key(&second));
    first.local_kinds.push(1);
    second.local_kinds.push(1);
    assert_ne!(local_kinds_key(&first), local_kinds_key(&second));

    first.locals.push("value".to_string());
    second.locals.push("value".to_string());
    assert_eq!(locals_tuple_key(&first), locals_tuple_key(&second));
    second.marshal_provenance.locals_identity = LocalsIdentity::Fresh;
    assert_ne!(locals_tuple_key(&first), locals_tuple_key(&second));
    first.marshal_provenance.locals_identity = LocalsIdentity::Fresh;
    assert_ne!(locals_tuple_key(&first), locals_tuple_key(&second));
}

#[test]
fn bypasses_value_lookup_for_structurally_unique_objects() {
    let mut inner = code_object(vec![Constant::Slice {
        lower: Box::new(Constant::Int(257)),
        upper: Box::new(Constant::None),
        step: Box::new(Constant::None),
    }]);
    inner.bytecode.push(0);
    inner.locals.push("value".to_string());
    inner.local_kinds.push(1);
    inner.marshal_provenance.locals_identity = LocalsIdentity::Fresh;

    let code = code_object(vec![Constant::Code(Box::new(inner))]);
    let graph = ObjectGraph::for_code(&code);

    // The code objects, their bytecode buffers, the outer identity-bearing constants tuple, fresh
    // locals, nonempty local kinds, and uncached scalar slice member all have a unique parent.
    assert_eq!(graph.object_count() - graph.keyed_object_count(), 8);
}

#[test]
fn borrowed_string_tuple_keys_compare_across_sources() {
    let constants = vec![
        Constant::String("alpha".to_string()),
        Constant::String("beta".to_string()),
    ];
    let strings = vec!["alpha".to_string(), "beta".to_string()];
    let constant_key = constants_tuple_key(&constants);
    let string_key = strings_tuple_key(&strings);

    assert_eq!(constant_key, string_key);

    let mut graph = ObjectGraph::default();
    assert!(graph.record(constant_key));
    assert!(!graph.record(string_key));
}

#[test]
fn borrowed_constant_tuple_keys_preserve_recursive_value_and_identity() {
    let first = vec![
        Constant::Tuple(vec![Constant::Int(1), Constant::Float(-0.0)]),
        Constant::FrozenSet(vec![
            Constant::String("value".to_string()),
            Constant::SignedInt(-1),
        ]),
        Constant::Slice {
            lower: Box::new(Constant::None),
            upper: Box::new(Constant::Int(10)),
            step: Box::new(Constant::Int(2)),
        },
    ];
    let second = vec![
        Constant::Tuple(vec![Constant::Int(1), Constant::Float(-0.0)]),
        Constant::FrozenSet(vec![
            Constant::SignedInt(-1),
            Constant::String("value".to_string()),
        ]),
        Constant::Slice {
            lower: Box::new(Constant::None),
            upper: Box::new(Constant::Int(10)),
            step: Box::new(Constant::Int(2)),
        },
    ];

    let mut graph = ObjectGraph::default();
    assert!(graph.record(constants_tuple_key(&first)));
    assert!(!graph.record(constants_tuple_key(&second)));

    let first_code = vec![Constant::Code(Box::new(code_object(Vec::new())))];
    let second_code = vec![Constant::Code(Box::new(code_object(Vec::new())))];
    assert_ne!(
        constants_tuple_key(&first_code),
        constants_tuple_key(&second_code)
    );
}

#[test]
fn distinguishes_cached_slice_members_from_fresh_objects() {
    for value in [
        Constant::Int(256),
        Constant::SignedInt(-5),
        Constant::String("é".to_string()),
        Constant::Bytes(vec![0]),
    ] {
        assert!(!slice_member_uses_identity(&value), "cached {value:?}");
    }
    for value in [
        Constant::Int(257),
        Constant::SignedInt(-6),
        Constant::String("Ā".to_string()),
        Constant::Bytes(vec![0, 1]),
        Constant::Float(1.0),
    ] {
        assert!(slice_member_uses_identity(&value), "fresh {value:?}");
    }
}

#[test]
fn frozen_set_keys_and_reference_order_ignore_input_order() {
    let values = vec![
        Constant::Int(1),
        Constant::String("value".to_string()),
        Constant::Tuple(vec![Constant::SignedInt(-1), Constant::Bool(true)]),
    ];
    let mut reversed = values.clone();
    reversed.reverse();

    let first = Constant::FrozenSet(values.clone());
    let second = Constant::FrozenSet(reversed.clone());
    let first_key = constant_key(&first);
    let second_key = constant_key(&second);
    assert_eq!(first_key, second_key);

    let first = code_object(vec![Constant::FrozenSet(values)]);
    let second = code_object(vec![Constant::FrozenSet(reversed)]);
    assert_eq!(encode_code(&first), encode_code(&second));
}

#[test]
fn nested_frozen_set_wire_order_uses_recursive_member_keys() {
    let first_nested = Constant::FrozenSet(vec![
        Constant::Int(1),
        Constant::String("alpha".to_string()),
    ]);
    let second_nested =
        Constant::FrozenSet(vec![Constant::Int(2), Constant::String("beta".to_string())]);
    assert_ne!(
        constant_sort_key(&first_nested),
        constant_sort_key(&second_nested)
    );

    let first = code_object(vec![Constant::FrozenSet(vec![
        first_nested.clone(),
        second_nested.clone(),
    ])]);
    let second = code_object(vec![Constant::FrozenSet(vec![second_nested, first_nested])]);

    assert_eq!(encode_code(&first), encode_code(&second));
}

#[derive(Clone, Copy)]
enum RecordedValue<'a> {
    Code(&'a CodeObject),
    Constants(&'a [Constant]),
    Constant(&'a Constant),
    SliceMember(&'a Constant),
}

#[derive(Default)]
struct RecordingVisitor<'a> {
    entries: Vec<(RecordedValue<'a>, ReferenceMode)>,
    pruned: Option<&'a [Constant]>,
}

impl<'a> RecordingVisitor<'a> {
    fn push(&mut self, value: RecordedValue<'a>, reference_mode: ReferenceMode) {
        self.entries.push((value, reference_mode));
    }
}

impl<'a> MarshalVisitor<'a> for RecordingVisitor<'a> {
    fn code(&mut self, code: &'a CodeObject, reference_mode: ReferenceMode) -> bool {
        self.push(RecordedValue::Code(code), reference_mode);
        true
    }

    fn code_bytes(&mut self, _code: &'a CodeObject) {}

    fn constants(&mut self, values: &'a [Constant], reference_mode: ReferenceMode) -> bool {
        self.push(RecordedValue::Constants(values), reference_mode);
        !self
            .pruned
            .is_some_and(|pruned| std::ptr::eq(pruned, values))
    }

    fn strings(&mut self, _values: &'a [String]) -> bool {
        true
    }

    fn locals(&mut self, _code: &'a CodeObject) -> bool {
        true
    }

    fn local_kinds(&mut self, _code: &'a CodeObject) {}

    fn metadata_string(&mut self, _value: &'a str) {}

    fn first_line(&mut self, _value: u32) {}

    fn bytes(&mut self, _value: &'a [u8]) {}

    fn constant(&mut self, value: &'a Constant, reference_mode: ReferenceMode) -> bool {
        self.push(RecordedValue::Constant(value), reference_mode);
        true
    }

    fn slice_member(&mut self, value: &'a Constant) {
        self.push(RecordedValue::SliceMember(value), ReferenceMode::IfShared);
    }
}

#[test]
fn marshal_traversal_is_preorder() {
    let nested = code_object(vec![Constant::Tuple(vec![Constant::Int(7)])]);
    let code = code_object(vec![
        Constant::Code(Box::new(nested)),
        Constant::Tuple(vec![Constant::String("outer".to_string())]),
    ]);
    let mut visitor = RecordingVisitor::default();
    traverse_code(&code, &mut visitor);

    assert!(
        matches!(visitor.entries[0].0, RecordedValue::Code(value) if std::ptr::eq(value, std::ptr::from_ref(&code)))
    );
    let root_constants = visitor
        .entries
        .iter()
        .position(|(value, _)| {
            matches!(value, RecordedValue::Constants(values) if std::ptr::eq(*values, code.constants.as_slice()))
        })
        .unwrap();
    let Constant::Code(nested) = &code.constants[0] else {
        unreachable!();
    };
    let nested_code = visitor
        .entries
        .iter()
        .position(|(value, _)| {
            matches!(value, RecordedValue::Code(candidate) if std::ptr::eq(*candidate, nested.as_ref()))
        })
        .unwrap();
    let nested_constants = visitor
        .entries
        .iter()
        .position(|(value, _)| {
            matches!(value, RecordedValue::Constants(values) if std::ptr::eq(*values, nested.constants.as_slice()))
        })
        .unwrap();

    assert!(root_constants < nested_code);
    assert!(nested_code < nested_constants);
}

#[test]
fn marshal_traversal_honors_container_pruning() {
    let code = code_object(vec![Constant::Tuple(vec![Constant::String(
        "pruned".to_string(),
    )])]);
    let Constant::Tuple(pruned) = &code.constants[0] else {
        unreachable!();
    };
    let child = std::ptr::from_ref(&pruned[0]);
    let mut visitor = RecordingVisitor {
        pruned: Some(pruned),
        ..RecordingVisitor::default()
    };
    traverse_code(&code, &mut visitor);

    assert!(visitor.entries.iter().any(
        |(value, _)| matches!(value, RecordedValue::Constants(values) if std::ptr::eq(*values, pruned.as_slice()))
    ));
    assert!(!visitor.entries.iter().any(
        |(value, _)| matches!(value, RecordedValue::Constant(value) if std::ptr::eq(*value, child))
    ));
}

#[test]
fn marshal_traversal_preserves_reference_mode_and_slice_member_identity() {
    let code = code_object(vec![
        Constant::FrozenSet(vec![Constant::String("forced".to_string())]),
        Constant::Slice {
            lower: Box::new(Constant::Float(2.5)),
            upper: Box::new(Constant::Int(256)),
            step: Box::new(Constant::None),
        },
        Constant::Slice {
            lower: Box::new(Constant::Float(2.5)),
            upper: Box::new(Constant::Int(256)),
            step: Box::new(Constant::Bool(false)),
        },
    ]);
    let mut visitor = RecordingVisitor::default();
    traverse_code(&code, &mut visitor);

    let Constant::FrozenSet(values) = &code.constants[0] else {
        unreachable!();
    };
    let forced = visitor
        .entries
        .iter()
        .find(|(value, _)| {
            matches!(value, RecordedValue::Constant(value) if std::ptr::eq(*value, std::ptr::from_ref(&values[0])))
        })
        .unwrap();
    assert_eq!(forced.1, ReferenceMode::Forced);

    let slices = [1, 2].map(|index| {
        let Constant::Slice { lower, upper, .. } = &code.constants[index] else {
            unreachable!();
        };
        (lower.as_ref(), upper.as_ref())
    });
    let entry_for = |value: &Constant| {
        visitor
            .entries
            .iter()
            .find(|(planned, _)| match planned {
                RecordedValue::Constant(candidate) | RecordedValue::SliceMember(candidate) => {
                    std::ptr::eq(*candidate, value)
                }
                _ => false,
            })
            .unwrap()
    };
    assert!(matches!(
        entry_for(slices[0].0).0,
        RecordedValue::SliceMember(_)
    ));
    assert_eq!(entry_for(slices[0].0).1, ReferenceMode::IfShared);
    assert!(matches!(
        entry_for(slices[1].0).0,
        RecordedValue::SliceMember(_)
    ));
    assert!(matches!(
        entry_for(slices[0].1).0,
        RecordedValue::Constant(_)
    ));
    assert!(matches!(
        entry_for(slices[1].1).0,
        RecordedValue::Constant(_)
    ));
}
