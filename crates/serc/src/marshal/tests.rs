use crate::code_object::{
    CodeObject, Constant, LocalsIdentity, MarshalMetadataIds, MarshalProvenance,
};

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
        marshal_provenance: MarshalProvenance {
            metadata_ids: MarshalMetadataIds {
                domain_len: 2,
                filename: 1,
                ..MarshalMetadataIds::default()
            },
            subtree_has_frozenset_constants: true,
            ..MarshalProvenance::default()
        },
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
    code.marshal_provenance.metadata_ids.names.push(0);
    code.line_table.push(0);
    code.exception_table.push(0);
    let graph = ObjectGraph::for_code(&code);

    let mut occurrences = graph.occurrence_cursor();
    occurrences.consume(OccurrenceKind::Code);
    occurrences.consume(OccurrenceKind::CodeBytes);
    occurrences.consume(OccurrenceKind::Constants);
    let integer = occurrences.consume(OccurrenceKind::Constant);
    let signed_integer = occurrences.consume(OccurrenceKind::Constant);
    let byte = occurrences.consume(OccurrenceKind::Constant);
    let character = occurrences.consume(OccurrenceKind::Constant);
    for object in [integer, signed_integer, byte, character] {
        assert!(graph.is_shared_object(object));
    }
    assert!(graph.is_interned_object(character));

    occurrences.consume(OccurrenceKind::Strings);
    assert_eq!(
        occurrences.consume(OccurrenceKind::MetadataString),
        character
    );
    occurrences.consume(OccurrenceKind::Locals);
    occurrences.consume(OccurrenceKind::LocalKinds);
    for _ in 0..3 {
        occurrences.consume(OccurrenceKind::MetadataString);
    }
    assert_eq!(occurrences.consume(OccurrenceKind::Bytes), byte);
    assert_eq!(occurrences.consume(OccurrenceKind::Bytes), byte);
    occurrences.finish();
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
    code.marshal_provenance.metadata_ids.names.push(0);
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
fn frozenset_free_provenance_walk_preserves_nested_interns_and_qualified_names() {
    let mut nested = code_object(vec![Constant::String("Nested".to_string())]);
    nested.name = "Nested".to_string();
    nested.qualified_name = "Nested".to_string();
    nested
        .marshal_provenance
        .explicitly_interned_strings
        .insert("méthode".to_string());
    nested.marshal_provenance.qualified_name_constant_index = Some(0);
    nested.marshal_provenance.subtree_has_frozenset_constants = false;
    let mut hinted = code_object(vec![
        Constant::Code(Box::new(nested)),
        Constant::String("méthode".to_string()),
    ]);
    hinted.marshal_provenance.subtree_has_frozenset_constants = false;

    let mut fallback = hinted.clone();
    fallback.marshal_provenance.subtree_has_frozenset_constants = true;
    let Constant::Code(nested) = &mut fallback.constants[0] else {
        unreachable!();
    };
    nested.marshal_provenance.subtree_has_frozenset_constants = true;

    assert_eq!(encode_code(&hinted), encode_code(&fallback));
    let graph = ObjectGraph::for_code(&hinted);
    assert!(graph.is_interned("méthode"));
    assert!(graph.is_interned("Nested"));
}

#[test]
fn dense_metadata_ids_preserve_nested_and_constant_string_identity() {
    let mut nested = code_object(vec![Constant::String("shared_before".to_string())]);
    nested.names = vec!["shared_after".to_string(), "nested_only".to_string()];
    nested.locals = vec!["shared_local".to_string()];
    nested.local_kinds = vec![1];
    nested.name = "shared_before".to_string();
    nested.qualified_name = "shared_after".to_string();
    nested.marshal_provenance.metadata_ids = MarshalMetadataIds {
        domain_len: 8,
        names: vec![1, 2],
        locals: vec![3],
        filename: 7,
        name: Some(0),
        qualified_name: Some(1),
    };
    let mut hinted = code_object(vec![
        Constant::String("shared_before".to_string()),
        Constant::Code(Box::new(nested)),
        Constant::String("shared_after".to_string()),
        Constant::String("shared_local".to_string()),
    ]);
    hinted.names = vec!["shared_before".to_string(), "shared_after".to_string()];
    hinted.locals = vec!["shared_local".to_string()];
    hinted.local_kinds = vec![1];
    hinted.name = "shared_after".to_string();
    hinted.qualified_name = "shared_before".to_string();
    hinted.marshal_provenance.metadata_ids = MarshalMetadataIds {
        domain_len: 8,
        names: vec![0, 1],
        locals: vec![3],
        filename: 7,
        name: Some(1),
        qualified_name: Some(0),
    };

    let mut fallback = hinted.clone();
    fallback.marshal_provenance.metadata_ids.domain_len = u32::MAX;
    fallback.marshal_provenance.metadata_ids.filename = u32::MAX - 1;
    let Constant::Code(nested) = &mut fallback.constants[1] else {
        unreachable!();
    };
    nested.marshal_provenance.metadata_ids.domain_len = u32::MAX;
    nested.marshal_provenance.metadata_ids.filename = u32::MAX - 1;

    assert_eq!(encode_code(&hinted), encode_code(&fallback));
    let graph = ObjectGraph::for_code(&hinted);
    for value in [
        "shared_before",
        "shared_after",
        "shared_local",
        "nested_only",
    ] {
        assert!(
            graph.is_interned(value),
            "expected interned metadata {value}"
        );
        assert!(graph.is_shared(&ObjectKey::String(value), false));
    }
}

#[test]
#[should_panic(expected = "metadata ID tuple length")]
fn rejects_mismatched_metadata_id_lengths() {
    let mut code = code_object(Vec::new());
    code.names.push("untracked".to_string());
    ObjectGraph::for_code(&code);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "metadata ID must identify one string")]
fn rejects_colliding_metadata_ids() {
    let mut code = code_object(Vec::new());
    code.names = vec!["first".to_string(), "second".to_string()];
    code.marshal_provenance.metadata_ids.names = vec![0, 0];
    ObjectGraph::for_code(&code);
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
    inner.marshal_provenance.metadata_ids.locals.push(0);
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
fn python_312_frozenset_sort_keys_borrow_member_roots() {
    use crate::{InitialInternState, PythonVersion};

    // CPython 3.12 borrows the root used by its independent marshal sort key.
    // A unique integer or nonidentifier string sorts before an interned string;
    // a shared integer, or any member retained by 3.13+, sorts after it.
    for first in [
        Constant::Int(1000),
        Constant::String("marshal probe text".to_string()),
    ] {
        for shared in [false, true] {
            let mut code = code_object(vec![Constant::FrozenSet(vec![
                first.clone(),
                Constant::String("marshal_probe_identifier".to_string()),
            ])]);
            if shared {
                code.constants.push(first.clone());
            }
            let Constant::FrozenSet(members) = &code.constants[0] else {
                unreachable!()
            };
            for version in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                let graph = ObjectGraph::for_code_with_options(
                    &code,
                    version,
                    &InitialInternState::default(),
                );
                let ordered = graph.frozen_set_members(members);
                let first_index = usize::from(version != PythonVersion::Py312 || shared);
                assert!(
                    std::ptr::eq(ordered[0].1, &raw const members[first_index]),
                    "unexpected member order for {version}, shared={shared}, first={first:?}"
                );
            }
        }
    }
}

#[test]
fn python_312_filename_remains_distinct_from_an_equal_interned_constant() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};

    let mut code = code_object(vec![Constant::String("filename".to_string())]);
    code.filename = "filename".to_string();
    let state = InitialInternState::default();
    let old = encode_code_with_options(&code, PythonVersion::Py312, &state);
    let new = encode_code_with_options(&code, PythonVersion::Py313, &state);
    let interned = b"\xda\x08filename";
    let plain = b"\xfa\x08filename";
    assert!(old.windows(interned.len()).any(|bytes| bytes == interned));
    assert!(old.windows(plain.len()).any(|bytes| bytes == plain));
    assert!(new.windows(interned.len()).any(|bytes| bytes == interned));
    assert!(!new.windows(plain.len()).any(|bytes| bytes == plain));
}

#[test]
fn python_312_dynamic_code_names_keep_per_definition_identity() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};

    let name = "<generic parameters of f>";
    let mut child = code_object(vec![]);
    child.name = name.to_string();
    child.qualified_name = name.to_string();
    let code = code_object(vec![
        Constant::Code(Box::new(child.clone())),
        Constant::Code(Box::new(child)),
    ]);
    let state = InitialInternState::default();
    for (version, expected) in [(PythonVersion::Py312, 2), (PythonVersion::Py313, 1)] {
        let bytes = encode_code_with_options(&code, version, &state);
        assert_eq!(
            bytes
                .windows(name.len())
                .filter(|bytes| *bytes == name.as_bytes())
                .count(),
            expected,
        );
    }
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
    const USES_METADATA_IDS: bool = false;

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

    fn metadata_string(&mut self, _value: &'a str, _metadata_id: Option<u32>) {}

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

#[test]
fn frozenset_recreation_uses_selected_target_intern_state() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};

    let members = vec![Constant::String("aggregate_class".to_string())];
    let code = code_object(vec![
        Constant::FrozenSet(members.clone()),
        Constant::Code(Box::new(code_object(vec![Constant::FrozenSet(members)]))),
    ]);
    let isolated = InitialInternState::default();
    let py314 = encode_code_with_options(&code, PythonVersion::Py314, &isolated);
    let py315 = encode_code_with_options(&code, PythonVersion::Py315, &isolated);
    assert_ne!(py314, py315);

    let explicit = InitialInternState::new(["aggregate_class"]);
    assert_eq!(
        encode_code_with_options(&code, PythonVersion::Py315, &explicit),
        py314
    );
    assert_eq!(
        encode_code_with_options(&code, PythonVersion::Py314, &explicit),
        py314
    );
}

#[test]
fn python_315_small_integer_cache_preserves_all_scalar_slot_boundaries() {
    use crate::{InitialInternState, PythonVersion};
    let mut constants = Vec::new();
    for value in 250..=1030 {
        constants.push(Constant::Int(value));
        constants.push(Constant::SignedInt(i64::try_from(value).unwrap()));
    }
    constants.extend([
        Constant::SignedInt(-6),
        Constant::SignedInt(-5),
        Constant::Bytes(vec![0]),
        Constant::Bytes(vec![255]),
        Constant::String("é".to_string()),
        Constant::String("ÿ".to_string()),
    ]);
    let code = code_object(constants);
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let mut graph =
            ObjectGraph::for_code_with_options(&code, version, &InitialInternState::default());
        let maximum = if version == PythonVersion::Py314 {
            256
        } else {
            1024
        };
        for value in [256, 257, 1000, 1024, 1025] {
            assert_eq!(
                graph.is_shared(&ObjectKey::Int(value), false),
                value <= maximum
            );
            assert_eq!(
                graph.is_shared(&ObjectKey::SignedInt(i64::try_from(value).unwrap()), false),
                value <= maximum
            );
        }
        for key in [
            ObjectKey::Int(maximum),
            ObjectKey::SignedInt(i64::try_from(maximum).unwrap()),
            ObjectKey::SignedInt(-5),
            ObjectKey::Bytes(&[0]),
            ObjectKey::Bytes(&[255]),
            ObjectKey::String("é"),
            ObjectKey::String("ÿ"),
        ] {
            assert!(
                !graph.record(key),
                "cached scalar slot was lost for {version}"
            );
        }
    }
}

#[test]
fn python_315_slice_members_use_the_selected_small_integer_cache() {
    use super::key::slice_member_uses_identity_for;
    use crate::PythonVersion;
    for value in [
        Constant::Int(257),
        Constant::Int(1024),
        Constant::SignedInt(1000),
    ] {
        assert!(slice_member_uses_identity_for(&value, PythonVersion::Py314));
        assert!(!slice_member_uses_identity_for(
            &value,
            PythonVersion::Py315
        ));
    }
    for value in [Constant::Int(1025), Constant::SignedInt(-6)] {
        assert!(slice_member_uses_identity_for(&value, PythonVersion::Py314));
        assert!(slice_member_uses_identity_for(&value, PythonVersion::Py315));
    }
}

#[test]
fn metadata_tuple_aliases_intern_only_the_shared_constant_slots() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};
    let mut child = code_object(vec![Constant::None]);
    child.names = vec!["café".to_string()];
    child.marshal_provenance.metadata_ids.names = vec![0];
    let code = code_object(vec![
        Constant::String("café".to_string()),
        Constant::Code(Box::new(child)),
        Constant::Tuple(vec![Constant::String("café".to_string())]),
    ]);
    let plain = [b'u', 5, 0, 0, 0, b'c', b'a', b'f', 0xc3, 0xa9];
    let interned = [b't' | 0x80, 5, 0, 0, 0, b'c', b'a', b'f', 0xc3, 0xa9];
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let bytes = encode_code_with_options(&code, version, &InitialInternState::default());
        assert_eq!(
            bytes
                .windows(plain.len())
                .filter(|window| *window == plain)
                .count(),
            1
        );
        assert_eq!(
            bytes
                .windows(interned.len())
                .filter(|window| *window == interned)
                .count(),
            1
        );
    }
}

#[test]
fn metadata_tuple_aliases_do_not_intern_equal_frozenset_members() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};
    let mut code = code_object(vec![Constant::FrozenSet(vec![Constant::String(
        "café".to_string(),
    )])]);
    code.names = vec!["café".to_string()];
    code.marshal_provenance.metadata_ids.names = vec![0];
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let bytes = encode_code_with_options(&code, version, &InitialInternState::default());
        let plain_member = [b'u' | 0x80, 5, 0, 0, 0, b'c', b'a', b'f', 0xc3, 0xa9];
        assert!(
            bytes
                .windows(plain_member.len())
                .any(|window| window == plain_member)
        );
    }
}

#[test]
fn static_attribute_cache_admission_controls_frozenset_identity() {
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    // A metadata tuple hit skips recursive scalar admission. A cache miss admits the
    // parser's existing identifier, so later sets retain the cached frozenset identity.
    let cases = [
        (
            "class C:\n    def __init__(self):\n        self.exec = factory()\n",
            "exec",
            true,
        ),
        (
            "class C:\n    def __init__(self):\n        self.exec = 1\n",
            "exec",
            false,
        ),
        (
            "ordinary = 'exec'\nclass C:\n    def __init__(self):\n        self.exec = factory()\n",
            "exec",
            false,
        ),
        (
            "class C:\n    def __init__(self):\n        self.__exec = factory()\n",
            "__exec",
            true,
        ),
        (
            "def prior(x: int):\n    pass\nclass C:\n    def __init__(self):\n        self.format = factory()\n",
            "format",
            true,
        ),
    ];
    for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
        for (prefix, member, shared) in cases {
            // Distinct tags prevent whole co_consts tuples from masking the set identity.
            let source = format!(
                "{prefix}def first(item):\n    return 'tag A', item in {{'{member}', 'x', 'y', 'z'}}\ndef second(item):\n    return 'tag B', item in {{'{member}', 'x', 'y', 'z'}}\n"
            );
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            let module = compile_with_options(&source, "static_attributes.py", &options).unwrap();
            let first = module
                .code
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Code(code) if code.name == "first" => Some(code),
                    _ => None,
                })
                .unwrap();
            let set = first
                .constants
                .iter()
                .find(|constant| matches!(constant, Constant::FrozenSet(_)))
                .unwrap();
            let graph = ObjectGraph::for_code_with_options(
                &module.code,
                python_version,
                &options.initial_intern_state,
            );
            assert_eq!(
                graph.is_shared(&constant_key(set), false),
                shared,
                "{python_version}: {prefix}"
            );
        }
    }
}

#[test]
fn legacy_discarded_constants_preserve_qualified_name_identity() {
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    for python_version in [PythonVersion::Py312, PythonVersion::Py313] {
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        for (prefix, copies) in [
            ("", 1),
            ("if False:\n    earlier = 'Outer.Inner'\n", 2),
            ("earlier = 'Outer.Inner' if False else None\n", 2),
            (
                "if False:\n    def earlier():\n        return 'Outer.Inner'\n",
                2,
            ),
            (
                "if False:\n    class Outer:\n        class Inner:\n            pass\n",
                if python_version == PythonVersion::Py312 {
                    2
                } else {
                    1
                },
            ),
        ] {
            let source = format!("{prefix}class Outer:\n    class Inner:\n        pass\n");
            let module = compile_with_options(&source, "qualified_name.py", &options).unwrap();
            let bytes = super::writer::encode_code_with_options(
                &module.code,
                options.python_version,
                &options.initial_intern_state,
            );
            let name = b"Outer.Inner";
            assert_eq!(
                bytes
                    .windows(name.len())
                    .filter(|value| *value == name)
                    .count(),
                copies,
                "{python_version}: {source}"
            );
        }
    }
}
