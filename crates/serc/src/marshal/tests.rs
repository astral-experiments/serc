use crate::code_object::{
    CodeObject, Constant, LocalsIdentity, MarshalMetadataIds, MarshalProvenance, MetadataId,
    MetadataName as Name,
};

use super::encode_code;
use super::graph::{ObjectGraph, OccurrenceKind};
use super::key::{
    HashedObjectKey, ObjectKey, code_bytes_key, code_key, constant_key, constant_sort_key,
    constants_tuple_key, local_kinds_key, locals_tuple_key, metadata_tuple_properties,
    names_tuple_key, slice_member_uses_identity, strings_tuple_key,
};
use super::plan::{MarshalVisitor, ObjectId, ReferenceMode, traverse_code};

#[test]
fn cached_constant_tuple_hashes_preserve_cloned_benchmark_streams() {
    fn clear_hashes(code: &mut CodeObject) {
        code.marshal_provenance.constants_tuple_hash = None;
        code.marshal_provenance.marshal_occurrence_bound = None;
        for value in &mut code.constants {
            if let Constant::Code(child) = value {
                clear_hashes(child);
            }
        }
    }

    fn check(module: &crate::CompiledModule, label: &str) {
        let cloned = module.clone();
        let mut uncached = module.clone();
        clear_hashes(&mut uncached.code);
        let expected = uncached.marshal();
        assert_eq!(module.marshal(), expected, "{label}");
        assert_eq!(cloned.marshal(), expected, "{label}");
    }

    for workload in serc_fixtures::COMPILE_MICRO_WORKLOADS
        .iter()
        .chain(serc_fixtures::MARSHAL_MICRO_WORKLOADS)
        .chain(serc_fixtures::COMPOSITE_WORKLOADS)
        .chain(serc_fixtures::LARGE_WORKLOADS)
    {
        check(
            &crate::compile(workload.source, workload.filename).unwrap(),
            workload.id,
        );
    }
    // NaN keys use allocation identity; cloning must preserve their cached hashes.
    for python_version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        check(
            &crate::compile_with_options(
                "def result():\n    return (1e1000 - 1e1000, (1e1000 - 1e1000,))\n",
                "non_reflexive.py",
                &crate::CompileOptions {
                    python_version,
                    ..crate::CompileOptions::default()
                },
            )
            .unwrap(),
            "non-reflexive constants",
        );
    }
}

#[test]
fn code_identity_descendants_cannot_cache_a_constant_tuple_hash() {
    let child = Constant::Code(Box::new(code_object(Vec::new())));
    for value in [
        child.clone(),
        Constant::Tuple(vec![child.clone()]),
        Constant::FrozenSet(vec![child.clone()]),
        Constant::Slice {
            lower: Box::new(Constant::None),
            upper: Box::new(child),
            step: Box::new(Constant::None),
        },
    ] {
        assert_eq!(super::key::constants_tuple_hash(&[value]), None);
    }
}

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
        filename: "module.py".into(),
        name: "<module>".to_string(),
        qualified_name: "<module>".to_string(),
        first_line_number: 1,
        line_table: Vec::new(),
        exception_table: Vec::new(),
        marshal_provenance: MarshalProvenance {
            metadata_ids: MarshalMetadataIds {
                domain_len: 2,
                filename: MetadataId::from_u32(1),
                ..MarshalMetadataIds::default()
            },
            subtree_has_frozenset_constants: true,
            ..MarshalProvenance::default()
        },
    }
}

#[test]
fn cached_constants_tuple_hashes_preserve_marshal_and_pyc_after_cloning() {
    use crate::{CompileOptions, PythonVersion, compile_with_options};

    fn clear_hashes(code: &mut CodeObject) {
        code.marshal_provenance.constants_tuple_hash = None;
        code.marshal_provenance.table_hashes = None;
        for value in &mut code.constants {
            if let Constant::Code(child) = value {
                clear_hashes(child);
            }
        }
    }

    for python_version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        for workload in serc_fixtures::MARSHAL_MICRO_WORKLOADS
            .iter()
            .chain(serc_fixtures::COMPOSITE_WORKLOADS)
            .chain(serc_fixtures::LARGE_WORKLOADS)
        {
            // The annotation fixture uses type parameter defaults introduced in 3.13.
            if python_version == PythonVersion::Py312 && workload.id == "annotation_scopes" {
                continue;
            }
            let module =
                compile_with_options(workload.source, workload.filename, &options).unwrap();
            let mut fallback = module.clone();
            clear_hashes(&mut fallback.code);
            let expected = fallback.marshal();
            assert_eq!(module.marshal(), expected, "{}", workload.id);
            assert_eq!(module.clone().marshal(), expected, "{}", workload.id);
            assert_eq!(
                module.to_timestamp_pyc(0, 0),
                fallback.to_timestamp_pyc(0, 0),
                "{}",
                workload.id,
            );
        }
    }
}

#[test]
fn partial_intern_state_checks_unrecorded_class_qualified_names() {
    use crate::{
        CompileError, CompileOptions, InitialInternState, OptimizationLevel, PythonVersion,
        compile_bytes_with_options, compile_with_options,
    };

    for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
        for optimization_level in [
            OptimizationLevel::Zero,
            OptimizationLevel::One,
            OptimizationLevel::Two,
        ] {
            for model_codec in [true, false] {
                let options = |state: InitialInternState| CompileOptions {
                    python_version,
                    optimization_level,
                    initial_intern_state: if model_codec {
                        state
                    } else {
                        state.without_standard_source_codec_effects()
                    },
                    ..CompileOptions::default()
                };
                let unknown = options(InitialInternState::from_partial([] as [&str; 0]));
                for (source, name) in [
                    ("class Δ:\n pass\n", "Δ"),
                    ("class 変数:\n pass\n", "変数"),
                    ("class A:\n class Δ:\n  pass\n", "A.Δ"),
                ] {
                    assert_eq!(
                        compile_with_options(source, "tests/class.py", &unknown).unwrap_err(),
                        CompileError::UnresolvedInternState {
                            value: name.to_string(),
                        },
                    );
                }
                for (source, name) in [
                    (b"# coding: utf-8\nclass \xce\x94:\n pass\n".as_slice(), "Δ"),
                    (b"# coding: cp1251\nclass \xd4:\n pass\n".as_slice(), "Ф"),
                ] {
                    assert_eq!(
                        compile_bytes_with_options(source, "tests/class.py", &unknown).unwrap_err(),
                        CompileError::UnresolvedInternState {
                            value: name.to_string(),
                        },
                    );
                }

                for (partial, complete) in [
                    (
                        InitialInternState::from_partial(["Δ"]),
                        InitialInternState::from_complete(["Δ"]),
                    ),
                    (
                        InitialInternState::from_partial_with_absence(
                            [] as [&str; 0],
                            |_, value| value == "Δ",
                        ),
                        InitialInternState::from_complete([] as [&str; 0]),
                    ),
                ] {
                    let partial = compile_with_options(
                        "class Δ:\n pass\n",
                        "tests/class.py",
                        &options(partial),
                    )
                    .unwrap()
                    .marshal();
                    let complete = compile_with_options(
                        "class Δ:\n pass\n",
                        "tests/class.py",
                        &options(complete),
                    )
                    .unwrap()
                    .marshal();
                    assert_eq!(partial, complete);
                }

                // An earlier ordinary constant leaves the class constant distinct,
                // so its initial canonical state is not queried by the serializer.
                let source = "value = 'Δ'\nclass Δ:\n pass\n";
                let partial = compile_with_options(source, "tests/class.py", &unknown)
                    .unwrap()
                    .marshal();
                for state in [
                    InitialInternState::from_complete([] as [&str; 0]),
                    InitialInternState::from_complete(["Δ"]),
                ] {
                    let complete = compile_with_options(source, "tests/class.py", &options(state))
                        .unwrap()
                        .marshal();
                    assert_eq!(partial, complete);
                }
            }
        }
    }
}

#[test]
fn optional_marshal_ids_remain_four_bytes() {
    assert_eq!(std::mem::size_of::<Option<MetadataId>>(), 4);
    assert_eq!(std::mem::size_of::<Option<ObjectId>>(), 4);
}

#[test]
fn reserves_output_for_nested_code_and_shared_long_values() {
    let value = "abcde".repeat(2_000);
    let bytes = "12345".repeat(2_000);
    let identifier = format!("long_identifier_{}", "x".repeat(2_000));
    let source = format!(
        "{identifier} = ({value:?}, b{bytes:?})\ndef nested():\n    return {identifier}, ({value:?}, b{bytes:?})\n"
    );
    let module = crate::compile(&source, "marshal_reservation.py").unwrap();
    let estimated = ObjectGraph::for_code(&module.code).estimated_wire_size();
    let encoded = module.marshal();
    assert!(
        estimated >= encoded.len(),
        "{estimated} < {}",
        encoded.len()
    );
    assert!(
        estimated < encoded.len() + 1_024,
        "shared values should not be reserved again: {estimated} vs {}",
        encoded.len()
    );

    for workload in serc_fixtures::MARSHAL_MICRO_WORKLOADS
        .iter()
        .chain(serc_fixtures::COMPOSITE_WORKLOADS)
        .chain(serc_fixtures::LARGE_WORKLOADS)
    {
        let module = crate::compile(workload.source, workload.filename).unwrap();
        let estimated = ObjectGraph::for_code(&module.code).estimated_wire_size();
        let encoded = module.marshal();
        assert!(
            estimated >= encoded.len(),
            "{}: {estimated} < {}",
            workload.id,
            encoded.len()
        );
    }
}

#[test]
fn timestamp_pyc_reserves_header_and_graph_for_every_target() {
    use super::writer::encode_pyc_code_with_options;
    use crate::{CompileOptions, PythonVersion};

    let source = format!(
        "first = {:?}\ndef nested():\n    return first, {:?}\n",
        "first value ".repeat(1_000),
        "caf\u{e9} ".repeat(1_000),
    );
    let source_mtime = 0x1234_5678;
    let source_size = 0x9abc_def0;
    for python_version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        let module = crate::compile_with_options(&source, "reservation.py", &options).unwrap();
        let estimated = ObjectGraph::for_code_with_options(
            &module.code,
            python_version,
            &module.initial_intern_state,
        )
        .estimated_wire_size();
        let bare = module.marshal();
        let pyc = module.to_timestamp_pyc(source_mtime, source_size);
        let mut header = [0; 16];
        header[..4].copy_from_slice(&python_version.target().magic_number);
        header[8..12].copy_from_slice(&source_mtime.to_le_bytes());
        header[12..].copy_from_slice(&source_size.to_le_bytes());
        assert_eq!(&pyc[..16], &header);
        assert!(estimated > module.bytecode().len());
        assert!(pyc.capacity() >= header.len() + estimated);
        assert!(bare.capacity() >= estimated);

        let custom_header = [0xab; 16];
        let with_custom_header = encode_pyc_code_with_options(
            &module.code,
            python_version,
            &module.initial_intern_state,
            &custom_header,
        );
        assert_eq!(&with_custom_header[..16], &custom_header);
        assert_eq!(&with_custom_header[16..], &pyc[16..]);
        if python_version == PythonVersion::Py315 {
            assert_eq!(pyc[16] | super::v5::FLAG_REF, bare[0]);
            assert_ne!(&pyc[16..], bare);
        } else {
            assert_eq!(&pyc[16..], bare);
        }
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

    assert!(graph.record(key));
    assert!(!graph.is_shared(&key, false));
    assert!(!graph.record(key));
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
fn precomputed_metadata_hashes_match_borrowed_tuples_from_either_source() {
    let names = ["value", "méthode", ".0", "é"].map(Name::new);
    let locals = ["ascii_name", "A09", "_"].map(Name::new);
    let properties = metadata_tuple_properties(&names, &locals);
    assert!(!properties.names.automatically_interned);
    assert!(properties.locals.automatically_interned);

    for (values, properties) in [
        (names.as_slice(), properties.names),
        (locals.as_slice(), properties.locals),
        (&[], metadata_tuple_properties(&[], &[]).names),
    ] {
        let constants = values
            .iter()
            .map(|value| Constant::String(value.to_string()))
            .collect::<Vec<_>>();
        let hinted = HashedObjectKey::with_hash(names_tuple_key(values), properties.hash);
        assert_eq!(hinted, HashedObjectKey::from(names_tuple_key(values)));
        assert_eq!(
            hinted,
            HashedObjectKey::from(constants_tuple_key(&constants))
        );
    }
}

fn remove_constant_tuple_hashes(code: &mut CodeObject) {
    code.marshal_provenance.constants_tuple_hash = None;
    for constant in &mut code.constants {
        if let Constant::Code(child) = constant {
            remove_constant_tuple_hashes(child);
        }
    }
}

#[test]
fn constant_tuple_hashes_preserve_sharing_after_cloning() {
    let source = "\
marker = ('méthode',)
def first():
    return None, (1, -1, -0.0, 2j), 'méthode'
def second():
    return None, (1, -1, -0.0, 2j), 'méthode'
def lookup():
    return méthode
def membership(value):
    return value in {(1, 2), (3, 4)}
";
    for python_version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        let options = crate::CompileOptions {
            python_version,
            ..crate::CompileOptions::default()
        };
        let module = crate::compile_with_options(source, "constant_hints.py", &options).unwrap();
        let mut fallback = module.clone();
        remove_constant_tuple_hashes(&mut fallback.code);
        assert_eq!(module.marshal(), fallback.marshal());
        assert_eq!(module.marshal(), module.clone().marshal());
        assert_eq!(
            module.to_timestamp_pyc(0, 0),
            fallback.to_timestamp_pyc(0, 0)
        );
    }

    let nested_code = Constant::Tuple(vec![Constant::Code(Box::new(code_object(vec![])))]);
    assert!(super::key::constants_tuple_hash(&[nested_code]).is_none());
}

#[test]
fn benchmark_constant_tuple_hashes_match_uncached_marshal() {
    for workload in serc_fixtures::COMPILE_MICRO_WORKLOADS
        .iter()
        .chain(serc_fixtures::MARSHAL_MICRO_WORKLOADS)
        .chain(serc_fixtures::COMPOSITE_WORKLOADS)
        .chain(serc_fixtures::LARGE_WORKLOADS)
    {
        let module = crate::compile(workload.source, workload.filename).unwrap();
        let mut fallback = module.clone();
        remove_constant_tuple_hashes(&mut fallback.code);
        assert_eq!(module.marshal(), fallback.marshal(), "{}", workload.id);
    }
}

#[test]
fn compiled_metadata_hashes_and_interning_flags_match_uncached_marshal() {
    fn remove_properties(code: &mut CodeObject, observed: &mut [bool; 4]) {
        let properties = code
            .marshal_provenance
            .metadata_tuple_properties
            .take()
            .unwrap();
        observed[0] |= properties.names.automatically_interned;
        observed[1] |= !properties.names.automatically_interned;
        observed[2] |= !properties.locals.automatically_interned;
        observed[3] |= code.marshal_provenance.locals_identity == LocalsIdentity::Fresh;
        for constant in &mut code.constants {
            if let Constant::Code(child) = constant {
                remove_properties(child, observed);
            }
        }
    }

    let source = "\
import xml.etree
marker = ('méthode',)
def lookup():
    return méthode
def with_unicode_local(méthode, café):
    return méthode, café
class Annotated:
    field: int
";
    for python_version in [
        crate::PythonVersion::Py312,
        crate::PythonVersion::Py313,
        crate::PythonVersion::Py314,
        crate::PythonVersion::Py315,
    ] {
        let options = crate::CompileOptions {
            python_version,
            ..crate::CompileOptions::default()
        };
        let module = crate::compile_with_options(source, "metadata_hints.py", &options).unwrap();
        let mut fallback = module.clone();
        let mut observed = [false; 4];
        remove_properties(&mut fallback.code, &mut observed);
        assert_eq!(observed[..3], [true; 3]);
        if python_version >= crate::PythonVersion::Py314 {
            assert!(
                observed[3],
                "annotation locals missing for {python_version}"
            );
        }
        assert_eq!(module.marshal(), fallback.marshal(), "{python_version}");
        assert_eq!(
            module.to_timestamp_pyc(123, 456),
            fallback.to_timestamp_pyc(123, 456),
            "{python_version}"
        );
    }
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
        assert!(graph.record(key));
        assert!(!graph.record(key));
    }

    assert_eq!(graph.object_count(), 6);
    assert_eq!(graph.keyed_object_count(), 6);
    assert_eq!(graph.hashed_object_count(), 0);
}

#[test]
fn cached_scalar_slots_preserve_domains_and_cross_source_identity() {
    let first_character = String::from("é");
    let second_character = String::from("é");
    let first_byte = [0];
    let second_byte = [0];
    let mut graph = ObjectGraph::default();

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
    code.names.push("é".into());
    code.marshal_provenance
        .metadata_ids
        .names
        .push(MetadataId::from_u32(0));
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
    code.names.push("metadata name with spaces".into());
    code.marshal_provenance
        .metadata_ids
        .names
        .push(MetadataId::from_u32(0));
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
fn qualified_name_scalar_interning_and_recorded_origins_match_fallback() {
    fn class(name: &str, origin: Option<bool>) -> Constant {
        let mut code = code_object(vec![Constant::String(name.to_string())]);
        code.name = name.rsplit('.').next().unwrap().to_string();
        code.qualified_name = name.to_string();
        code.marshal_provenance.qualified_name_constant_index = Some(0);
        code.marshal_provenance.qualified_name_is_canonical = origin;
        Constant::Code(Box::new(code))
    }

    fn module(simple: Option<bool>, shared: Option<bool>, separate: Option<bool>) -> CodeObject {
        code_object(vec![
            Constant::String("SercOuter.Separate".to_string()),
            class("SercSimple", simple),
            class("SercOuter.Shared", shared),
            class("SercOuter.Separate", separate),
        ])
    }

    let fallback = encode_code(&module(None, None, None));
    assert_eq!(fallback, encode_code(&module(Some(false), None, None)));
    assert_eq!(
        fallback,
        encode_code(&module(Some(false), Some(true), Some(false)))
    );
    assert_eq!(
        fallback,
        encode_code(&module(Some(false), None, Some(false)))
    );
    assert_ne!(fallback, encode_code(&module(None, Some(false), None)));
    assert_ne!(fallback, encode_code(&module(None, None, Some(true))));
}

#[test]
fn class_qualified_name_origins_preserve_hints_and_surviving_order() {
    fn class(name: &str, canonical: Option<bool>, frozenset_hint: bool) -> Constant {
        let mut code = code_object(vec![Constant::String(name.to_string())]);
        code.name = name.rsplit('.').next().unwrap().to_string();
        code.qualified_name = name.to_string();
        code.marshal_provenance.qualified_name_constant_index = Some(0);
        code.marshal_provenance.qualified_name_is_canonical = canonical;
        code.marshal_provenance.subtree_has_frozenset_constants = frozenset_hint;
        Constant::Code(Box::new(code))
    }

    fn encoded(
        name: &str,
        ordinary_first: bool,
        canonical: Option<bool>,
        frozenset_hint: bool,
    ) -> Vec<u8> {
        let ordinary = Constant::String(name.to_string());
        let class = class(name, canonical, frozenset_hint);
        let mut root = code_object(if ordinary_first {
            vec![ordinary, class]
        } else {
            vec![class, ordinary]
        });
        root.marshal_provenance.subtree_has_frozenset_constants = frozenset_hint;
        encode_code(&root)
    }

    fn spelling_count(bytes: &[u8], name: &str) -> usize {
        bytes
            .windows(name.len())
            .filter(|bytes| *bytes == name.as_bytes())
            .count()
    }

    let dotted = "outer.<locals>.MarshalOriginClass";
    let identifier = "MarshalIdentifierClass";
    for frozenset_hint in [false, true] {
        for ordinary_first in [false, true] {
            let fallback = encoded(dotted, ordinary_first, None, frozenset_hint);
            assert_eq!(
                fallback,
                encoded(
                    dotted,
                    ordinary_first,
                    Some(!ordinary_first),
                    frozenset_hint
                ),
            );
            assert_eq!(
                spelling_count(&fallback, dotted),
                if ordinary_first { 2 } else { 1 },
            );

            // Recorded discarded-code admissions remain authoritative even when the order of
            // the surviving constants alone would give the opposite identity.
            let contrary = encoded(dotted, ordinary_first, Some(ordinary_first), frozenset_hint);
            assert_eq!(
                spelling_count(&contrary, dotted),
                if ordinary_first { 1 } else { 2 },
            );

            let automatic = encoded(identifier, ordinary_first, None, frozenset_hint);
            assert_eq!(spelling_count(&automatic, identifier), 1);
            for origin in [false, true] {
                assert_eq!(
                    automatic,
                    encoded(identifier, ordinary_first, Some(origin), frozenset_hint),
                );
            }
        }

        let hinted = "outer.<locals>.HintedOriginClass";
        let unhinted = "outer.<locals>.UnhintedOriginClass";
        let mut mixed = code_object(vec![
            Constant::String(hinted.to_string()),
            class(hinted, Some(true), frozenset_hint),
            class(hinted, None, frozenset_hint),
            Constant::String(unhinted.to_string()),
            class(unhinted, None, frozenset_hint),
        ]);
        mixed.marshal_provenance.subtree_has_frozenset_constants = frozenset_hint;
        let bytes = encode_code(&mixed);
        assert_eq!(spelling_count(&bytes, hinted), 1);
        assert_eq!(spelling_count(&bytes, unhinted), 2);
    }
}

#[test]
fn compiled_class_qualified_names_match_unhinted_surviving_provenance() {
    fn remove_hints(code: &mut CodeObject, observed: &mut Vec<(String, bool)>) {
        if let Some(origin) = code.marshal_provenance.qualified_name_is_canonical.take() {
            observed.push((code.qualified_name.clone(), origin));
        }
        for constant in &mut code.constants {
            if let Constant::Code(code) = constant {
                remove_hints(code, observed);
            }
        }
    }

    let source = "\
class IdentifierClass:
    pass

class Namespace:
    earlier = 'Namespace.LiteralFirst'
    class LiteralFirst:
        pass
    class ClassFirst:
        pass
    later = 'Namespace.ClassFirst'
";
    let module = crate::compile(source, "class_origins.py").unwrap();
    let expected = module.marshal();
    let mut fallback = module.code;
    let mut observed = Vec::new();
    remove_hints(&mut fallback, &mut observed);
    assert!(observed.contains(&("Namespace.LiteralFirst".to_string(), false)));
    assert!(observed.contains(&("Namespace.ClassFirst".to_string(), true)));
    assert_eq!(encode_code(&fallback), expected);
}

#[test]
fn dense_metadata_ids_preserve_nested_and_constant_string_identity() {
    let mut nested = code_object(vec![Constant::String("shared_before".to_string())]);
    nested.names = vec!["shared_after".into(), "nested_only".into()];
    nested.locals = vec!["shared_local".into()];
    nested.local_kinds = vec![1];
    nested.name = "shared_before".to_string();
    nested.qualified_name = "shared_after".to_string();
    nested.marshal_provenance.metadata_ids = MarshalMetadataIds {
        domain_len: 8,
        names: vec![MetadataId::from_u32(1), MetadataId::from_u32(2)],
        locals: vec![MetadataId::from_u32(3)],
        filename: MetadataId::from_u32(7),
        name: Some(MetadataId::from_u32(0)),
        qualified_name: Some(MetadataId::from_u32(1)),
    };
    let mut hinted = code_object(vec![
        Constant::String("shared_before".to_string()),
        Constant::Code(Box::new(nested)),
        Constant::String("shared_after".to_string()),
        Constant::String("shared_local".to_string()),
    ]);
    hinted.names = vec!["shared_before".into(), "shared_after".into()];
    hinted.locals = vec!["shared_local".into()];
    hinted.local_kinds = vec![1];
    hinted.name = "shared_after".to_string();
    hinted.qualified_name = "shared_before".to_string();
    hinted.marshal_provenance.metadata_ids = MarshalMetadataIds {
        domain_len: 8,
        names: vec![MetadataId::from_u32(0), MetadataId::from_u32(1)],
        locals: vec![MetadataId::from_u32(3)],
        filename: MetadataId::from_u32(7),
        name: Some(MetadataId::from_u32(1)),
        qualified_name: Some(MetadataId::from_u32(0)),
    };

    let mut fallback = hinted.clone();
    fallback.marshal_provenance.metadata_ids.domain_len = u32::MAX;
    fallback.marshal_provenance.metadata_ids.filename = MetadataId::from_u32(u32::MAX - 1);
    let Constant::Code(nested) = &mut fallback.constants[1] else {
        unreachable!();
    };
    nested.marshal_provenance.metadata_ids.domain_len = u32::MAX;
    nested.marshal_provenance.metadata_ids.filename = MetadataId::from_u32(u32::MAX - 1);

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
    code.names.push("untracked".into());
    ObjectGraph::for_code(&code);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "metadata ID must identify one string")]
fn rejects_colliding_metadata_ids() {
    let mut code = code_object(Vec::new());
    code.names = vec!["first".into(), "second".into()];
    code.marshal_provenance.metadata_ids.names =
        vec![MetadataId::from_u32(0), MetadataId::from_u32(0)];
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

    first.locals.push("value".into());
    second.locals.push("value".into());
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
    inner.locals.push("value".into());
    inner
        .marshal_provenance
        .metadata_ids
        .locals
        .push(MetadataId::from_u32(0));
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
    let long = "metadata_identifier_that_does_not_fit_in_inline_name_storage";
    let constants = [
        Constant::String("alpha".to_string()),
        Constant::String("beta".to_string()),
        Constant::String(long.to_string()),
    ];
    let strings = ["alpha".to_string(), "beta".to_string(), long.to_string()];
    let names = ["alpha", "beta", long].map(Name::new);
    let keys = [0..1, 0..2, 1..2, 2..3].map(|range| {
        [
            constants_tuple_key(&constants[range.clone()]),
            strings_tuple_key(&strings[range.clone()]),
            names_tuple_key(&names[range]),
        ]
    });
    for (left_index, left_keys) in keys.iter().enumerate() {
        for (right_index, right_keys) in keys.iter().enumerate() {
            for left in left_keys {
                for right in right_keys {
                    assert_eq!(left == right, left_index == right_index);
                    assert_eq!(left.cmp(right), left_index.cmp(&right_index));
                }
            }
        }
        let mut graph = ObjectGraph::default();
        assert!(graph.record(left_keys[0]));
        assert!(!graph.record(left_keys[1]));
        assert!(!graph.record(left_keys[2]));
    }
}

#[test]
fn short_and_long_compiler_names_keep_marshaled_metadata_identity() {
    fn assert_send_and_sync<T: Send + Sync>() {}

    assert_send_and_sync::<crate::CompiledModule>();
    let long_local = "local_identifier_that_does_not_fit_in_inline_name_storage";
    let long_global = "global_identifier_that_does_not_fit_in_inline_name_storage";
    let source = format!(
        "def example(short, {long_local}):\n    return short, {long_local}, global_short, {long_global}\n"
    );
    let module = crate::compile(&source, "metadata.py").unwrap();
    let expected = module.marshal();
    let mut fallback = module.code;
    let child = fallback
        .constants
        .iter_mut()
        .find_map(|constant| match constant {
            Constant::Code(child) => Some(child),
            _ => None,
        })
        .unwrap();
    assert_eq!(child.locals, ["short", long_local]);
    assert_eq!(child.names, ["global_short", long_global]);
    assert_eq!(
        format!("{:?}", child.locals),
        format!("{:?}", ["short", long_local])
    );
    child.marshal_provenance.metadata_tuple_properties = None;
    fallback.marshal_provenance.metadata_tuple_properties = None;
    assert_eq!(encode_code(&fallback), expected);
}

#[test]
fn root_constant_tuples_are_unique_unless_they_alias_string_metadata() {
    let tuple = Constant::Tuple(vec![Constant::Int(1000)]);
    let code = code_object(vec![tuple.clone(), tuple]);
    let graph = ObjectGraph::for_code(&code);
    // The nested tuple and its integer need value lookup; the root tuple cannot be shared.
    assert_eq!(graph.hashed_object_count(), 2);
    assert!(!graph.is_shared(&constants_tuple_key(&code.constants), false));
    let Constant::Tuple(values) = &code.constants[0] else {
        unreachable!();
    };
    assert!(graph.is_shared(&constants_tuple_key(values), false));

    let mut code = code_object(vec![Constant::String("alpha".to_string())]);
    code.names.push("alpha".into());
    code.marshal_provenance.metadata_ids.names = vec![MetadataId::from_u32(0)];
    let graph = ObjectGraph::for_code(&code);
    assert!(graph.is_shared(&constants_tuple_key(&code.constants), false));
    assert!(graph.is_shared(&names_tuple_key(&code.names), false));
    let mut occurrences = graph.occurrence_cursor();
    occurrences.consume(OccurrenceKind::Code);
    occurrences.consume(OccurrenceKind::CodeBytes);
    let constants = occurrences.consume(OccurrenceKind::Constants);
    occurrences.consume(OccurrenceKind::Constant);
    assert_eq!(occurrences.consume(OccurrenceKind::Strings), constants);
    assert!(!encode_code(&code).is_empty());
}

#[test]
fn graph_shares_long_string_tuples_with_literal_and_metadata_occurrences() {
    let strings: Vec<_> = (0..32).map(|index| format!("name_{index}")).collect();
    let constants: Vec<_> = strings.iter().cloned().map(Constant::String).collect();
    let mut code = code_object(vec![
        Constant::Tuple(constants.clone()),
        Constant::Tuple(constants),
        Constant::Code(Box::new(code_object(Vec::new()))),
    ]);
    code.marshal_provenance.metadata_ids.domain_len = 34;
    code.marshal_provenance.metadata_ids.names = (2..34).map(MetadataId::from_u32).collect();
    code.names = strings.into_iter().map(Name::from).collect();

    let graph = ObjectGraph::for_code(&code);
    let Constant::Tuple(first) = &code.constants[0] else {
        unreachable!();
    };
    assert!(graph.is_shared(&constants_tuple_key(first), false));
    assert!(graph.is_shared(&names_tuple_key(&code.names), false));
    assert!(!graph.is_shared(&constants_tuple_key(&code.constants), false));
    assert!(!encode_code(&code).is_empty());
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
fn borrowed_set_and_slice_keys_keep_canonical_hashes_without_drop_glue() {
    use std::hash::{Hash, Hasher};

    use rustc_hash::FxHasher;

    assert!(!std::mem::needs_drop::<ObjectKey<'_>>());
    assert!(!std::mem::needs_drop::<HashedObjectKey<'_>>());

    let nested = Constant::FrozenSet(vec![
        Constant::FrozenSet(vec![Constant::Int(2), Constant::Int(1)]),
        Constant::Tuple(vec![
            Constant::Float(-0.0),
            Constant::String("value".into()),
        ]),
    ]);
    for value in [
        Constant::FrozenSet(Vec::new()),
        Constant::FrozenSet((0..8).rev().map(Constant::Int).collect()),
        nested.clone(),
        Constant::Slice {
            lower: Box::new(nested),
            upper: Box::new(Constant::Float(f64::NAN)),
            step: Box::new(Constant::SignedInt(-1)),
        },
    ] {
        let key = constant_key(&value);
        let mut expected = FxHasher::default();
        std::mem::discriminant(&key).hash(&mut expected);
        match &value {
            Constant::FrozenSet(values) => {
                let mut members: Vec<_> = values.iter().map(constant_key).collect();
                members.sort_unstable();
                members.hash(&mut expected);
            }
            Constant::Slice { lower, upper, step } => {
                for member in [lower, upper, step] {
                    constant_key(member).hash(&mut expected);
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(
            HashedObjectKey::from(key),
            HashedObjectKey::with_hash(key, expected.finish())
        );
    }
}

#[test]
fn nested_frozen_keys_preserve_order_and_hash_through_tuple_and_slice_members() {
    use std::cmp::Ordering;

    fn tree(depth: usize, seed: u64, reversed: bool) -> Constant {
        if depth == 0 {
            return Constant::Tuple(vec![
                Constant::Tuple(Vec::new()),
                Constant::Tuple(vec![Constant::String("shared".into())]),
                Constant::Int(seed),
            ]);
        }
        let mut members = vec![
            tree(depth - 1, seed, reversed),
            tree(depth - 1, seed + 1, reversed),
        ];
        if reversed {
            members.reverse();
        }
        Constant::FrozenSet(members)
    }

    fn wrapped(value: Constant) -> Constant {
        Constant::FrozenSet(vec![
            Constant::Tuple(vec![value.clone(), Constant::Float(-0.0)]),
            Constant::Slice {
                lower: Box::new(value),
                upper: Box::new(Constant::Float(f64::NAN).with_non_reflexive_origin(17)),
                step: Box::new(Constant::None),
            },
        ])
    }

    let first = wrapped(tree(5, 0, false));
    let reversed = wrapped(tree(5, 0, true));
    let greater = wrapped(tree(5, 1, true));
    let first_key = constant_key(&first);
    let reversed_key = constant_key(&reversed);
    let greater_key = constant_key(&greater);
    assert_eq!(first_key, reversed_key);
    assert_eq!(first_key.cmp(&reversed_key), Ordering::Equal);
    assert_eq!(first_key.cmp(&greater_key), Ordering::Less);
    assert_ne!(first_key, greater_key);
    assert_eq!(
        HashedObjectKey::from(first_key),
        HashedObjectKey::from(reversed_key)
    );
}

#[test]
fn distinguishes_cached_slice_members_from_fresh_objects() {
    for value in [
        Constant::Int(256),
        Constant::SignedInt(-5),
        Constant::String(String::new()),
        Constant::String("é".to_string()),
        Constant::Bytes(vec![0]),
    ] {
        assert!(!slice_member_uses_identity(&value), "cached {value:?}");
    }
    for value in [
        Constant::Int(257),
        Constant::SignedInt(-6),
        Constant::String("Ā".to_string()),
        Constant::String("aa".to_string()),
        Constant::Bytes(vec![0, 1]),
        Constant::Float(1.0),
    ] {
        assert!(slice_member_uses_identity(&value), "fresh {value:?}");
    }
}

#[test]
fn non_reflexive_slice_members_preserve_scalar_bytes_and_occurrence_identity() {
    for scalar in [
        Constant::Float(f64::NAN),
        Constant::Complex {
            real: f64::NAN,
            imag: -0.0,
        },
        Constant::Complex {
            real: 0.0,
            imag: f64::NAN,
        },
    ] {
        let tracked = scalar.clone().with_non_reflexive_origin(17);
        assert!(tracked.is_truthy());
        let slice_code = |value: Constant| {
            code_object(vec![Constant::Slice {
                lower: Box::new(value.clone()),
                upper: Box::new(value),
                step: Box::new(Constant::None),
            }])
        };
        let plain = slice_code(scalar);
        let tracked = slice_code(tracked);
        for python_version in [crate::PythonVersion::Py314, crate::PythonVersion::Py315] {
            assert_eq!(
                super::writer::encode_code_with_options(
                    &tracked,
                    python_version,
                    &crate::InitialInternState::default(),
                ),
                super::writer::encode_code_with_options(
                    &plain,
                    python_version,
                    &crate::InitialInternState::default(),
                ),
            );
        }
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
                let ordered = graph.frozen_set_members(members, 0);
                let first_index = usize::from(version != PythonVersion::Py312 || shared);
                assert!(
                    std::ptr::eq(ordered[0], &raw const members[first_index]),
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
    code.filename = "filename".into();
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
fn metadata_string_tags_and_legacy_exceptions_match_each_target() {
    use super::v5::{
        FLAG_REF, TYPE_ASCII, TYPE_ASCII_INTERNED, TYPE_INTERNED, TYPE_SHORT_ASCII,
        TYPE_SHORT_ASCII_INTERNED, TYPE_UNICODE,
    };
    use super::writer::encode_code_with_options;
    use crate::code_object::CodeFilename;
    use crate::{InitialInternState, PythonVersion};

    fn assert_tag(bytes: &[u8], value: &[u8], header_size: usize, tag: u8) {
        let mut matches = bytes
            .windows(value.len())
            .enumerate()
            .filter_map(|(offset, bytes)| (bytes == value).then_some(offset));
        let offset = matches.next().expect("string must be serialized");
        assert!(matches.next().is_none(), "string must only be emitted once");
        assert_eq!(bytes[offset - header_size], tag, "{value:?}");
    }

    let ordinary = "ordinary marker!";
    let unicode_ordinary = "ordinary café!";
    let long_ordinary = "ordinary!".repeat(35);
    let name = "normal_code_name";
    let qualified_name = "Different.Qualified";
    let metadata = "short_metadata";
    let unicode_metadata = "méthode";
    let long_metadata = "long_metadata_".repeat(24);
    let filename = "répertoire-metadata.py";
    let generic_name = "<generic parameters of marker>";
    let surrogate_filename = b"surro\xed\xa0\x80gate.py";
    let state = InitialInternState::default();

    let mut code = code_object(vec![
        Constant::String(ordinary.to_string()),
        Constant::String(unicode_ordinary.to_string()),
        Constant::String(long_ordinary.clone()),
    ]);
    code.name = name.to_string();
    code.qualified_name = qualified_name.to_string();
    code.filename = filename.into();
    code.names = vec![
        metadata.into(),
        unicode_metadata.into(),
        long_metadata.as_str().into(),
    ];
    code.locals = vec![unicode_metadata.into()];
    code.local_kinds = vec![1];
    code.marshal_provenance.metadata_ids.domain_len = 5;
    code.marshal_provenance.metadata_ids.names = vec![
        MetadataId::from_u32(0),
        MetadataId::from_u32(2),
        MetadataId::from_u32(3),
    ];
    code.marshal_provenance.metadata_ids.locals = vec![MetadataId::from_u32(2)];

    let mut generic = code_object(Vec::new());
    generic.name = generic_name.to_string();
    generic.qualified_name = generic_name.to_string();
    generic.filename = CodeFilename::Surrogate(surrogate_filename.as_slice().into());

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let old = version == PythonVersion::Py312;
        let bytes = encode_code_with_options(&code, version, &state);
        assert_tag(&bytes, ordinary.as_bytes(), 2, TYPE_SHORT_ASCII);
        assert_tag(&bytes, unicode_ordinary.as_bytes(), 5, TYPE_UNICODE);
        assert_tag(&bytes, long_ordinary.as_bytes(), 5, TYPE_ASCII);
        assert_tag(
            &bytes,
            metadata.as_bytes(),
            2,
            TYPE_SHORT_ASCII_INTERNED | FLAG_REF,
        );
        assert_tag(
            &bytes,
            unicode_metadata.as_bytes(),
            5,
            TYPE_INTERNED | FLAG_REF,
        );
        assert_tag(
            &bytes,
            long_metadata.as_bytes(),
            5,
            TYPE_ASCII_INTERNED | FLAG_REF,
        );
        assert_tag(
            &bytes,
            name.as_bytes(),
            2,
            TYPE_SHORT_ASCII_INTERNED | FLAG_REF,
        );
        assert_tag(
            &bytes,
            qualified_name.as_bytes(),
            2,
            if old {
                TYPE_SHORT_ASCII
            } else {
                TYPE_SHORT_ASCII_INTERNED | FLAG_REF
            },
        );
        assert_tag(
            &bytes,
            filename.as_bytes(),
            5,
            (if old { TYPE_UNICODE } else { TYPE_INTERNED }) | FLAG_REF,
        );

        let bytes = encode_code_with_options(&generic, version, &state);
        assert_tag(
            &bytes,
            generic_name.as_bytes(),
            2,
            if old {
                TYPE_SHORT_ASCII | FLAG_REF
            } else {
                TYPE_SHORT_ASCII_INTERNED | FLAG_REF
            },
        );
        assert_tag(
            &bytes,
            surrogate_filename,
            5,
            (if old { TYPE_UNICODE } else { TYPE_INTERNED }) | FLAG_REF,
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

#[test]
fn frozen_set_order_replay_handles_nested_sets_and_skips_references() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};

    fn reverse_sets(value: &mut Constant) {
        if let Constant::FrozenSet(members) = value {
            members.reverse();
            for member in members {
                reverse_sets(member);
            }
        }
    }

    let outer = Constant::FrozenSet(vec![
        Constant::Tuple(vec![Constant::Int(1000), Constant::Int(1001)]),
        Constant::FrozenSet(vec![Constant::Int(2000), Constant::Int(2001)]),
        Constant::FrozenSet(vec![Constant::Int(2002)]),
        Constant::FrozenSet(vec![]),
    ]);
    let code = code_object(vec![
        outer.clone(),
        outer,
        Constant::FrozenSet(vec![Constant::Int(2002)]),
        Constant::FrozenSet(vec![Constant::Int(4), Constant::Int(2)]),
    ]);
    let mut reversed = code.clone();
    for constant in &mut reversed.constants {
        reverse_sets(constant);
    }

    let state = InitialInternState::default();
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let graph = ObjectGraph::for_code_with_options(&code, version, &state);
        assert_eq!(graph.frozen_set_order_count(), 3);
        assert_eq!(
            encode_code_with_options(&code, version, &state),
            encode_code_with_options(&reversed, version, &state)
        );
    }
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

    fn names(&mut self, _code: &'a CodeObject) -> bool {
        true
    }

    fn locals(&mut self, _code: &'a CodeObject) -> bool {
        true
    }

    fn local_kinds(&mut self, _code: &'a CodeObject) {}

    fn metadata_string(&mut self, _value: &'a str, _metadata_id: Option<MetadataId>) {}

    fn code_filename(&mut self, _code: &'a CodeObject, _metadata_id: Option<MetadataId>) {}

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
fn complete_intern_state_preserves_identifier_dependent_frozenset_recreation() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};

    let marker = "aggregate_class";
    let empty = InitialInternState::from_complete(std::iter::empty::<&str>());
    let retained = InitialInternState::from_complete([marker]);
    for mixed in [false, true] {
        let mut members = vec![Constant::String(marker.to_string())];
        if mixed {
            members.push(Constant::String("__serc_not_interned__".to_string()));
        }
        let mut code = code_object(vec![
            Constant::FrozenSet(members.clone()),
            Constant::Code(Box::new(code_object(vec![Constant::FrozenSet(members)]))),
        ]);
        let graph = ObjectGraph::for_code_with_options(&code, PythonVersion::Py314, &empty);
        assert!(graph.is_shared(&constant_key(&code.constants[0]), false));
        drop(graph);
        let graph = ObjectGraph::for_code_with_options(&code, PythonVersion::Py314, &retained);
        assert!(!graph.is_shared(&constant_key(&code.constants[0]), false));
        drop(graph);

        code.marshal_provenance
            .parser_interned_strings
            .insert(marker.to_string());
        let graph = ObjectGraph::for_code_with_options(&code, PythonVersion::Py314, &empty);
        assert!(!graph.is_shared(&constant_key(&code.constants[0]), false));
        assert_eq!(
            encode_code_with_options(&code, PythonVersion::Py314, &empty),
            encode_code_with_options(&code, PythonVersion::Py314, &retained),
            "identifier interning must apply with mixed members: {mixed}"
        );
    }
}

#[test]
fn partial_intern_state_accepts_unique_scalar_frozensets() {
    use crate::{CompileOptions, InitialInternState, PythonVersion, compile_with_options};

    let source = "def choice(x):\n    return x in {'unique_member_984', 'second_member_984'}\n";
    let mut options = CompileOptions {
        python_version: PythonVersion::Py313,
        initial_intern_state: InitialInternState::from_partial(std::iter::empty::<&str>()),
        ..CompileOptions::default()
    };
    let bytes = compile_with_options(source, "unique.py", &options)
        .unwrap()
        .marshal();
    for state in [
        InitialInternState::from_complete(std::iter::empty::<&str>()),
        InitialInternState::from_complete(["unique_member_984", "second_member_984"]),
    ] {
        options.initial_intern_state = state;
        assert_eq!(
            bytes,
            compile_with_options(source, "unique.py", &options)
                .unwrap()
                .marshal()
        );
    }
}

#[test]
fn partial_intern_state_accepts_shared_co_consts_with_one_frozenset_slot() {
    use crate::{CompileOptions, InitialInternState, PythonVersion, compile_with_options};

    let source = "def first(x):\n    return x in {'tuple_owner_999', 'tuple_companion_999'}\ndef second(x):\n    return x in {'tuple_owner_999', 'tuple_companion_999'}\n";
    let mut options = CompileOptions {
        python_version: PythonVersion::Py313,
        initial_intern_state: InitialInternState::from_partial(std::iter::empty::<&str>()),
        ..CompileOptions::default()
    };
    let bytes = compile_with_options(source, "tuple_owner.py", &options)
        .unwrap()
        .marshal();
    for members in [
        vec![],
        vec!["tuple_owner_999"],
        vec!["tuple_companion_999"],
        vec!["tuple_owner_999", "tuple_companion_999"],
    ] {
        options.initial_intern_state = InitialInternState::from_complete(members);
        assert_eq!(
            bytes,
            compile_with_options(source, "tuple_owner.py", &options)
                .unwrap()
                .marshal()
        );
    }
}

#[test]
fn partial_intern_state_unifies_ordinary_tuple_and_co_consts_owner_slots() {
    use super::graph::validate_initial_intern_state;
    use super::writer::encode_code_with_options;
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "tuple_owner_999";
    let frozen = Constant::FrozenSet(vec![Constant::String(marker.to_string())]);
    let tuple = Constant::Tuple(vec![frozen.clone()]);
    let cases = [
        vec![tuple.clone(), tuple.clone()],
        vec![
            Constant::Tuple(vec![Constant::Int(1), tuple.clone()]),
            Constant::Tuple(vec![Constant::Int(2), tuple.clone()]),
        ],
        vec![
            tuple,
            Constant::Code(Box::new(code_object(vec![frozen.clone()]))),
        ],
    ];
    let partial = InitialInternState::from_partial(std::iter::empty::<&str>());
    for constants in cases {
        let code = code_object(constants);
        assert!(validate_initial_intern_state(&code, PythonVersion::Py313, &partial).is_ok());
        let bytes = encode_code_with_options(&code, PythonVersion::Py313, &partial);
        for complete in [
            InitialInternState::from_complete(std::iter::empty::<&str>()),
            InitialInternState::from_complete([marker]),
        ] {
            assert_eq!(
                bytes,
                encode_code_with_options(&code, PythonVersion::Py313, &complete)
            );
        }
        // The owner-alias proof is scoped to the pinned 3.13 target.
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert_eq!(
                validate_initial_intern_state(&code, version, &partial),
                Err(CompileError::UnresolvedInternState {
                    value: marker.to_string()
                })
            );
        }
    }

    // A single raw occurrence remains independent even with an unsafe sibling.
    let code = code_object(vec![frozen, Constant::Float(f64::NAN)]);
    assert!(validate_initial_intern_state(&code, PythonVersion::Py313, &partial).is_ok());
}

#[test]
fn partial_intern_state_does_not_merge_unsafe_tuple_owner_slots() {
    use super::graph::validate_initial_intern_state;
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "unsafe_tuple_owner_999";
    let frozen = Constant::FrozenSet(vec![Constant::String(marker.to_string())]);
    let partial = InitialInternState::from_partial(std::iter::empty::<&str>());
    let mut cases = vec![
        vec![Constant::Tuple(vec![frozen.clone(), frozen.clone()])],
        vec![
            Constant::Tuple(vec![Constant::Int(1), frozen.clone()]),
            Constant::Tuple(vec![Constant::Int(2), frozen.clone()]),
        ],
    ];
    for unsafe_member in [
        Constant::Float(f64::NAN),
        Constant::Complex {
            real: 1.0,
            imag: 0.0,
        },
        Constant::Code(Box::new(code_object(Vec::new()))),
        Constant::Slice {
            lower: Box::new(Constant::None),
            upper: Box::new(Constant::None),
            step: Box::new(Constant::None),
        },
    ] {
        let parent = Constant::Tuple(vec![frozen.clone(), Constant::Tuple(vec![unsafe_member])]);
        cases.push(vec![parent.clone(), parent]);
    }
    let tuple = Constant::Tuple(vec![frozen]);
    cases.push(vec![
        tuple.clone(),
        Constant::Slice {
            lower: Box::new(tuple.clone()),
            upper: Box::new(Constant::None),
            step: Box::new(Constant::None),
        },
    ]);
    for reverse in [false, true] {
        let mut constants = vec![tuple.clone(), Constant::FrozenSet(vec![tuple.clone()])];
        if reverse {
            constants.reverse();
        }
        cases.push(constants);
    }
    for constants in cases {
        assert_eq!(
            validate_initial_intern_state(&code_object(constants), PythonVersion::Py313, &partial),
            Err(CompileError::UnresolvedInternState {
                value: marker.to_string()
            })
        );
    }
}

#[test]
fn partial_intern_state_preserves_unique_set_origins_needed_by_later_shared_sets() {
    use crate::{
        CompileError, CompileOptions, InitialInternState, PythonVersion, compile_with_options,
    };

    let source = "def first(x):\n    return x in {'shared_origin_984', 'unique_companion_984'}\ndef second(x):\n    return x in {'shared_origin_984'}\ndef third(x):\n    'different tuple'\n    return x in {'shared_origin_984'}\n";
    let options = CompileOptions {
        python_version: PythonVersion::Py313,
        initial_intern_state: InitialInternState::from_partial(std::iter::empty::<&str>()),
        ..CompileOptions::default()
    };
    assert_eq!(
        compile_with_options(source, "earlier_unique.py", &options).unwrap_err(),
        CompileError::UnresolvedInternState {
            value: "shared_origin_984".to_string()
        }
    );
}

#[test]
fn partial_intern_state_keeps_shared_tuple_origins_for_later_sensitive_sets() {
    use crate::{
        CompileError, CompileOptions, InitialInternState, PythonVersion, compile_with_options,
    };

    let source = "def first(x):\n    return x in {'shared_origin_999', 'other_member_999'}\ndef second(x):\n    return x in {'shared_origin_999', 'other_member_999'}\ndef third(x):\n    return x in {'shared_origin_999'}\ndef fourth(x):\n    'different tuple'\n    return x in {'shared_origin_999'}\n";
    let options = CompileOptions {
        python_version: PythonVersion::Py313,
        initial_intern_state: InitialInternState::from_partial(std::iter::empty::<&str>()),
        ..CompileOptions::default()
    };
    assert_eq!(
        compile_with_options(source, "shared_tuple_origin.py", &options).unwrap_err(),
        CompileError::UnresolvedInternState {
            value: "shared_origin_999".to_string()
        }
    );
}

#[test]
fn partial_intern_state_keeps_uncertain_aliased_or_nested_frozensets() {
    use super::graph::validate_initial_intern_state;
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "aliased_member_984";
    let frozen = Constant::FrozenSet(vec![Constant::String(marker.to_string())]);
    let cases = [
        vec![
            Constant::Tuple(vec![frozen.clone()]),
            Constant::Tuple(vec![Constant::None, frozen.clone()]),
        ],
        vec![
            frozen.clone(),
            Constant::Code(Box::new(code_object(vec![frozen.clone()]))),
        ],
        vec![
            frozen.clone(),
            Constant::Slice {
                lower: Box::new(frozen.clone()),
                upper: Box::new(Constant::None),
                step: Box::new(Constant::None),
            },
        ],
        vec![Constant::FrozenSet(vec![frozen])],
        vec![Constant::FrozenSet(vec![
            Constant::String(marker.to_string()),
            Constant::Float(f64::NAN),
        ])],
    ];
    let state = InitialInternState::from_partial(std::iter::empty::<&str>());
    for constants in cases {
        assert_eq!(
            validate_initial_intern_state(&code_object(constants), PythonVersion::Py313, &state),
            Err(CompileError::UnresolvedInternState {
                value: marker.to_string()
            })
        );
    }
}

#[test]
fn partial_intern_state_accepts_definite_reconstruction_with_unknown_members() {
    use crate::{CompileOptions, InitialInternState, PythonVersion, compile_with_options};

    let known = "retained_owner_995";
    let unknown = "uncertain_member_995";
    for members in [
        "'retained_owner_995', 'uncertain_member_995'",
        "'uncertain_member_995', 'retained_owner_995'",
    ] {
        for prefix in [
            "",
            "def earlier(x):\n    return x in {'uncertain_member_995', 'unique_companion_995'}\n",
            "if False:\n    def discarded():\n        return 'uncertain_member_995'\n",
        ] {
            let source = format!(
                "{prefix}def first(x):\n    'first tuple'\n    return x in {{{members}}}\ndef second(x):\n    'second tuple'\n    return x in {{{members}}}\n"
            );
            let mut options = CompileOptions {
                python_version: PythonVersion::Py313,
                initial_intern_state: InitialInternState::from_partial([known]),
                ..CompileOptions::default()
            };
            let bytes = compile_with_options(&source, "dominated.py", &options)
                .unwrap()
                .marshal();
            for state in [
                InitialInternState::from_complete([known]),
                InitialInternState::from_complete([known, unknown]),
            ] {
                options.initial_intern_state = state;
                assert_eq!(
                    bytes,
                    compile_with_options(&source, "dominated.py", &options)
                        .unwrap()
                        .marshal()
                );
            }
        }
    }
}

#[test]
fn partial_intern_state_preserves_dominated_origins_across_later_promotions() {
    use super::graph::validate_initial_intern_state;
    use crate::code_object::{CodeInterningSummary, CompilationInterningEvent};
    use crate::{CompileError, InitialInternState, PythonVersion};

    let known = "retained_owner_995";
    let unknown = "uncertain_member_995";
    for reverse in [false, true] {
        let mut members = vec![
            Constant::String(known.to_string()),
            Constant::String(unknown.to_string()),
        ];
        if reverse {
            members.reverse();
        }
        let mut children = Vec::new();
        for (order, tag, members) in [
            (0, "first tuple", members.clone()),
            (1, "second tuple", members),
            (
                3,
                "third tuple",
                vec![Constant::String(unknown.to_string())],
            ),
            (
                4,
                "fourth tuple",
                vec![Constant::String(unknown.to_string())],
            ),
        ] {
            let mut child = code_object(vec![
                Constant::String(tag.to_string()),
                Constant::FrozenSet(members),
            ]);
            child.marshal_provenance.assembly_order = Some(order);
            children.push(child);
        }
        let mut promoter = code_object(Vec::new());
        promoter.names.push(unknown.into());
        promoter
            .marshal_provenance
            .explicitly_interned_strings
            .insert(unknown.to_string());
        let mut events = Vec::new();
        for (index, child) in children.iter().enumerate() {
            if index == 2 {
                events.push(CompilationInterningEvent::CodeAssembly(
                    CodeInterningSummary::from_code(&promoter),
                ));
            }
            events.push(CompilationInterningEvent::CodeAssembly(
                CodeInterningSummary::from_code(child),
            ));
        }
        let mut code = code_object(
            children
                .into_iter()
                .map(|child| Constant::Code(Box::new(child)))
                .collect(),
        );
        code.marshal_provenance.assembly_order = Some(5);
        events.push(CompilationInterningEvent::CodeAssembly(
            CodeInterningSummary::from_code(&code),
        ));
        code.marshal_provenance.interning_events = events;
        assert_eq!(
            validate_initial_intern_state(
                &code,
                PythonVersion::Py313,
                &InitialInternState::from_partial([known])
            ),
            Err(CompileError::UnresolvedInternState {
                value: unknown.to_string()
            })
        );
    }
}

#[test]
fn partial_intern_state_keeps_nested_uncertainty_and_a_deterministic_witness() {
    use super::graph::validate_initial_intern_state;
    use crate::{CompileError, InitialInternState, PythonVersion};

    let known = "retained_owner_995";
    let state = InitialInternState::from_partial([known]);
    let nested = code_object(vec![Constant::FrozenSet(vec![
        Constant::String(known.to_string()),
        Constant::FrozenSet(vec![Constant::String("nested_unknown_995".to_string())]),
    ])]);
    assert_eq!(
        validate_initial_intern_state(&nested, PythonVersion::Py313, &state),
        Err(CompileError::UnresolvedInternState {
            value: "nested_unknown_995".to_string()
        })
    );

    // Reverse constant processing encounters z first, but replay must collect all
    // unresolved surviving decisions before selecting its stable diagnostic.
    let constants = [
        "a_unknown_995",
        "a_unknown_995",
        "z_unknown_995",
        "z_unknown_995",
    ]
    .map(|value| Constant::FrozenSet(vec![Constant::String(value.to_string())]));
    assert_eq!(
        validate_initial_intern_state(&code_object(constants.into()), PythonVersion::Py313, &state),
        Err(CompileError::UnresolvedInternState {
            value: "a_unknown_995".to_string()
        })
    );
}

#[test]
fn partial_intern_state_rejects_relevant_unknowns_before_returning_a_module() {
    use crate::{
        CompileError, CompileOptions, InitialInternState, PythonVersion, compile_with_options,
    };

    let marker = "unresolved_member_973";
    let source = "unrelated = 'unrelated_unknown_973'\ndef first(x):\n    return x in {'unresolved_member_973'}\ndef second(x):\n    'different tuple'\n    return x in {'unresolved_member_973'}\n";
    let mut options = CompileOptions {
        python_version: PythonVersion::Py313,
        initial_intern_state: InitialInternState::from_partial(std::iter::empty::<&str>()),
        ..CompileOptions::default()
    };
    assert_eq!(
        compile_with_options(source, "partial.py", &options).unwrap_err(),
        CompileError::UnresolvedInternState {
            value: marker.to_string()
        }
    );
    options.initial_intern_state = options
        .initial_intern_state
        .with_additional_strings([marker]);
    let module = compile_with_options(source, "partial.py", &options).unwrap();
    options.initial_intern_state = InitialInternState::from_complete([marker]);
    assert_eq!(
        module.marshal(),
        compile_with_options(source, "partial.py", &options)
            .unwrap()
            .marshal()
    );
}

#[test]
fn partial_intern_state_keeps_earlier_unknown_origins_from_discarded_code() {
    use super::graph::validate_initial_intern_state;
    use crate::code_object::{CodeInterningSummary, CompilationInterningEvent};
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "earlier_origin_973";
    let frozen = Constant::FrozenSet(vec![Constant::String(marker.to_string())]);
    let mut code = code_object(vec![
        Constant::Tuple(vec![frozen.clone()]),
        Constant::Tuple(vec![Constant::None, frozen]),
    ]);
    code.names.push(marker.into());
    code.marshal_provenance
        .generated_mangled_strings
        .insert(marker.to_string());
    code.marshal_provenance.assembly_order = Some(1);
    let discarded = code_object(vec![Constant::String(marker.to_string())]);
    code.marshal_provenance.interning_events = vec![
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&discarded)),
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&code)),
    ];
    let unknown = InitialInternState::from_partial(std::iter::empty::<&str>());
    assert_eq!(
        validate_initial_intern_state(&code, PythonVersion::Py313, &unknown),
        Err(CompileError::UnresolvedInternState {
            value: marker.to_string()
        })
    );
    assert!(
        validate_initial_intern_state(
            &code,
            PythonVersion::Py313,
            &unknown.with_additional_strings([marker]),
        )
        .is_ok()
    );
}

#[test]
fn partial_intern_state_does_not_retain_discarded_mortal_metadata() {
    use super::graph::validate_initial_intern_state;
    use crate::code_object::{CodeInterningSummary, CompilationInterningEvent};
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "discarded_filename_973";
    let mut code = code_object(vec![Constant::FrozenSet(vec![Constant::String(
        marker.to_string(),
    )])]);
    // Keep this dependency potentially shared so the origin decision matters.
    code.constants
        .push(Constant::Tuple(vec![code.constants[0].clone()]));
    code.marshal_provenance.assembly_order = Some(1);
    let mut discarded = code_object(Vec::new());
    discarded.filename = marker.into();
    code.marshal_provenance.interning_events = vec![
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&discarded)),
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&code)),
    ];
    let unknown = InitialInternState::from_partial(std::iter::empty::<&str>());
    assert_eq!(
        validate_initial_intern_state(&code, PythonVersion::Py313, &unknown),
        Err(CompileError::UnresolvedInternState {
            value: marker.to_string()
        })
    );
    assert!(
        validate_initial_intern_state(
            &code,
            PythonVersion::Py313,
            &unknown.with_additional_strings([marker]),
        )
        .is_ok()
    );
}

#[test]
fn partial_intern_state_respects_metadata_tuple_origins_and_generated_names() {
    use super::graph::validate_initial_intern_state;
    use crate::code_object::{CodeInterningSummary, CompilationInterningEvent};
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "metadata_origin_973";
    let mut code = code_object(vec![Constant::FrozenSet(vec![Constant::String(
        marker.to_string(),
    )])]);
    // Keep this dependency potentially shared so the origin decision matters.
    code.constants
        .push(Constant::Tuple(vec![code.constants[0].clone()]));
    code.marshal_provenance.assembly_order = Some(2);
    let mut prior = code_object(Vec::new());
    prior.names.push(marker.into());
    code.marshal_provenance.interning_events = vec![
        CompilationInterningEvent::CachedTuple(vec![marker.to_string()]),
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&prior)),
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&code)),
    ];
    let unknown = InitialInternState::from_partial(std::iter::empty::<&str>());
    assert_eq!(
        validate_initial_intern_state(&code, PythonVersion::Py313, &unknown),
        Err(CompileError::UnresolvedInternState {
            value: marker.to_string()
        })
    );
    code.marshal_provenance.interning_events.remove(0);
    code.marshal_provenance.assembly_order = Some(1);
    assert!(validate_initial_intern_state(&code, PythonVersion::Py313, &unknown).is_ok());

    code.marshal_provenance.interning_events.clear();
    code.marshal_provenance
        .explicitly_interned_strings
        .insert(marker.to_string());
    assert_eq!(
        validate_initial_intern_state(&code, PythonVersion::Py313, &unknown),
        Err(CompileError::UnresolvedInternState {
            value: marker.to_string()
        })
    );
}

#[test]
fn partial_absence_proofs_preserve_shared_constants_and_input_exceptions() {
    use crate::{
        CompileError, CompileOptions, InitialInternState, PythonVersion, compile_with_options,
    };

    let marker = "proved_absent_1001";
    let source = "def first(x):\n    return x in {'proved_absent_1001'}\ndef second(x):\n    'different tuple'\n    return x in {'proved_absent_1001'}\n";
    let state = InitialInternState::from_partial_with_absence(
        std::iter::empty::<&str>(),
        move |version, value| version == PythonVersion::Py313 && value == marker,
    );
    let mut options = CompileOptions {
        python_version: PythonVersion::Py313,
        initial_intern_state: state.clone(),
        ..CompileOptions::default()
    };
    let module = compile_with_options(source, "absence.py", &options).unwrap();
    let cloned_module = module.clone();
    let absent_bytes = module.marshal();
    drop(module);
    assert_eq!(absent_bytes, cloned_module.marshal());
    options.initial_intern_state = InitialInternState::from_complete(std::iter::empty::<&str>());
    assert_eq!(
        absent_bytes,
        compile_with_options(source, "absence.py", &options)
            .unwrap()
            .marshal()
    );
    options.initial_intern_state = InitialInternState::from_complete([marker]);
    let present_bytes = compile_with_options(source, "absence.py", &options)
        .unwrap()
        .marshal();
    assert_ne!(absent_bytes, present_bytes);

    options.initial_intern_state = state.with_absence_exceptions([marker]);
    assert_eq!(
        compile_with_options(source, "absence.py", &options).unwrap_err(),
        CompileError::UnresolvedInternState {
            value: marker.to_string()
        }
    );
    options.initial_intern_state = options
        .initial_intern_state
        .with_additional_strings([marker]);
    assert_eq!(
        present_bytes,
        compile_with_options(source, "absence.py", &options)
            .unwrap()
            .marshal()
    );

    // Ordinary constant interning has a proved phase and can establish the
    // compiler's canonical owner before later shared frozen-set occurrences.
    let source = format!("def earlier():\n    return '{marker}'\n{source}");
    options.initial_intern_state = state;
    let bytes = compile_with_options(&source, "absence.py", &options)
        .unwrap()
        .marshal();
    options.initial_intern_state = InitialInternState::from_complete(std::iter::empty::<&str>());
    assert_eq!(
        bytes,
        compile_with_options(&source, "absence.py", &options)
            .unwrap()
            .marshal()
    );
}

#[test]
fn partial_absence_is_invalidated_by_discarded_mortal_metadata() {
    use super::graph::validate_initial_intern_state;
    use super::writer::encode_code_with_options;
    use crate::code_object::{CodeInterningSummary, CompilationInterningEvent};
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "mortal_absence_1001";
    let state = InitialInternState::from_partial_with_absence(
        std::iter::empty::<&str>(),
        move |_, value| value == marker,
    );
    for field in ["filename", "name", "qualified_name"] {
        let frozen = Constant::FrozenSet(vec![Constant::String(marker.to_string())]);
        let mut code = code_object(vec![frozen.clone(), frozen]);
        code.marshal_provenance.assembly_order = Some(1);
        let mut discarded = code_object(Vec::new());
        match field {
            "filename" => discarded.filename = marker.into(),
            "name" => discarded.name = marker.to_string(),
            _ => discarded.qualified_name = marker.to_string(),
        }
        code.marshal_provenance.interning_events = vec![
            CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&discarded)),
            CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&code)),
        ];
        assert_eq!(
            validate_initial_intern_state(&code, PythonVersion::Py313, &state),
            Err(CompileError::UnresolvedInternState {
                value: marker.to_string()
            })
        );
        assert!(validate_initial_intern_state(&code, PythonVersion::Py312, &state).is_ok());
        let retained = state
            .with_absence_exceptions([marker])
            .with_additional_strings([marker]);
        assert!(validate_initial_intern_state(&code, PythonVersion::Py313, &retained).is_ok());
        assert_eq!(
            encode_code_with_options(&code, PythonVersion::Py313, &retained),
            encode_code_with_options(
                &code,
                PythonVersion::Py313,
                &InitialInternState::from_complete([marker])
            )
        );

        // A later immortal metadata phase can resolve possible membership,
        // provided no earlier ordinary query has already made the origin unknown.
        code.names.push(marker.into());
        code.marshal_provenance
            .metadata_ids
            .names
            .push(MetadataId::from_u32(0));
        code.marshal_provenance
            .generated_mangled_strings
            .insert(marker.to_string());
        assert!(validate_initial_intern_state(&code, PythonVersion::Py313, &state).is_ok());
        assert_eq!(
            encode_code_with_options(&code, PythonVersion::Py313, &state),
            encode_code_with_options(
                &code,
                PythonVersion::Py313,
                &InitialInternState::from_complete([marker])
            )
        );
        code.names.clear();
        code.marshal_provenance.metadata_ids.names.clear();

        // A real ordinary constant retains the canonical object even if a later
        // discarded metadata event has no independently proved owner.
        discarded
            .constants
            .push(Constant::String(marker.to_string()));
        code.marshal_provenance.interning_events[0] =
            CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&discarded));
        assert!(validate_initial_intern_state(&code, PythonVersion::Py313, &state).is_ok());
        assert_eq!(
            encode_code_with_options(&code, PythonVersion::Py313, &state),
            encode_code_with_options(
                &code,
                PythonVersion::Py313,
                &InitialInternState::from_complete(std::iter::empty::<&str>())
            )
        );
    }
}

#[test]
fn partial_absence_does_not_resolve_early_generated_name_origins() {
    use super::graph::validate_initial_intern_state;
    use crate::code_object::{CodeInterningSummary, CompilationInterningEvent};
    use crate::{CompileError, InitialInternState, PythonVersion};

    let marker = "generated_absence_1001";
    let frozen = Constant::FrozenSet(vec![Constant::String(marker.to_string())]);
    let mut code = code_object(vec![frozen.clone(), frozen]);
    code.marshal_provenance.assembly_order = Some(1);
    let mut generated = code_object(Vec::new());
    generated.names.push(marker.into());
    generated
        .marshal_provenance
        .explicitly_interned_strings
        .insert(marker.to_string());
    code.marshal_provenance.interning_events = vec![
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&generated)),
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&code)),
    ];
    let state = InitialInternState::from_partial_with_absence(
        std::iter::empty::<&str>(),
        move |_, value| value == marker,
    );
    assert_eq!(
        validate_initial_intern_state(&code, PythonVersion::Py313, &state),
        Err(CompileError::UnresolvedInternState {
            value: marker.to_string()
        })
    );
    assert!(
        validate_initial_intern_state(
            &code,
            PythonVersion::Py313,
            &state.with_additional_strings([marker])
        )
        .is_ok()
    );

    // Without the uncertain early hook, co_names establishes retained presence
    // at its real phase, before the later frozen constants are assembled.
    generated
        .marshal_provenance
        .explicitly_interned_strings
        .clear();
    code.marshal_provenance.interning_events[0] =
        CompilationInterningEvent::CodeAssembly(CodeInterningSummary::from_code(&generated));
    assert!(validate_initial_intern_state(&code, PythonVersion::Py313, &state).is_ok());
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
    child.names = vec!["café".into()];
    child.marshal_provenance.metadata_ids.names = vec![MetadataId::from_u32(0)];
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
fn compiled_metadata_tuple_aliases_match_pinned_cpython() {
    use sha2::{Digest, Sha256};

    // SHA-256 of complete marshal streams from CPython 3.14.5. The positive cases
    // retain a plain scalar beside a distinct, interned member of the shared tuple.
    let cases = [
        (
            "plain = \"pkg.child\"\ndef imported():\n    import pkg.child\nshared = (\"pkg.child\",)\n",
            false,
            "a2f9b34a96f7ec64eb93865e092a49e8e0364604f9da23a67731f9c29290e042",
        ),
        (
            "plain = \"café\"\ndef metadata():\n    return café\nshared = (\"café\",)\n",
            false,
            "639c5f08fa280abcf32468a72816feb04cc600a533fe24c3e67b38c8b7e3ddf5",
        ),
        (
            "plain = \"café\"\nif False:\n    def discarded():\n        return café\nshared = (\"café\",)\n",
            false,
            "cc6c43a785f740c2f42e66c2212195a305846bdbc9bc629b8159d86cb4d83c7b",
        ),
        (
            "plain = \".0\"\nshared = (\".0\", \"value\")\nitems = (value for value in ())\n",
            false,
            "59a2507c0e2ec332178e0db90f2e71e065db1763b5cc57b591a19292349c4d5a",
        ),
        (
            "plain = \"not an identifier!\"\ndef imported():\n    import pkg.child\nshared = (\"not an identifier!\",)\n",
            true,
            "49be849abd0950f75d79683c70aa62ca1bfd0343d2ef7902cfc52daac162e276",
        ),
    ];

    for (source, scan_unnecessary, cpython_sha256) in cases {
        let module = crate::compile(source, "metadata_alias.py").unwrap();
        assert_eq!(
            module
                .code
                .marshal_provenance
                .metadata_alias_scan_unnecessary,
            scan_unnecessary,
            "{source}"
        );
        let encoded = module.marshal();
        let mut fallback = module.code.clone();
        fallback.marshal_provenance.metadata_alias_scan_unnecessary = false;
        assert_eq!(encoded, encode_code(&fallback), "{source}");
        assert_eq!(
            format!("{:x}", Sha256::digest(&encoded)),
            cpython_sha256,
            "{source}"
        );
    }
}

#[test]
fn skipping_unrelated_metadata_tuple_aliases_preserves_benchmark_streams() {
    let workloads = [
        &serc_fixtures::MARSHAL_MICRO_WORKLOADS[1],
        &serc_fixtures::LARGE_WORKLOADS[0],
    ];
    for workload in workloads {
        let module = crate::compile(workload.source, workload.filename).unwrap();
        assert!(
            module
                .code
                .marshal_provenance
                .metadata_alias_scan_unnecessary,
            "{}",
            workload.id
        );
        let mut fallback = module.code.clone();
        fallback.marshal_provenance.metadata_alias_scan_unnecessary = false;
        assert_eq!(module.marshal(), encode_code(&fallback), "{}", workload.id);
    }
}

#[test]
fn metadata_tuple_aliases_do_not_intern_equal_frozenset_members() {
    use super::writer::encode_code_with_options;
    use crate::{InitialInternState, PythonVersion};
    let mut code = code_object(vec![Constant::FrozenSet(vec![Constant::String(
        "café".to_string(),
    )])]);
    code.names = vec!["café".into()];
    code.marshal_provenance.metadata_ids.names = vec![MetadataId::from_u32(0)];
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
