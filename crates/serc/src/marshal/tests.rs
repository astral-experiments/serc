use std::collections::HashSet;

use crate::compiler::{CodeObject, Constant};

use super::encode_code;
use super::graph::ObjectGraph;
use super::key::{
    ObjectKey, code_bytes_key, code_key, constant_key, local_kinds_key, locals_tuple_key,
    slice_member_uses_identity,
};
use super::plan::PlannedValue;

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
        annotation_thunk: false,
        interned_constant_strings: HashSet::default(),
    }
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
        ObjectKey::EmptyTuple,
    ] {
        assert!(graph.is_shared(&key, false), "expected cached key {key:?}");
    }
    for key in [
        ObjectKey::Int(257),
        ObjectKey::SignedInt(-6),
        ObjectKey::Bytes(&[0, 1]),
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
fn tracks_interned_strings_from_constants_and_metadata() {
    let mut code = code_object(vec![
        Constant::String("identifier".to_string()),
        Constant::String("méthode".to_string()),
        Constant::String("not interned".to_string()),
    ]);
    code.interned_constant_strings.insert("méthode".to_string());
    code.names.push("metadata name with spaces".to_string());
    let graph = ObjectGraph::for_code(&code);

    assert!(graph.is_interned("identifier"));
    assert!(graph.is_interned("méthode"));
    assert!(graph.is_interned("metadata name with spaces"));
    assert!(graph.is_interned("module.py"));
    assert!(!graph.is_interned("not interned"));
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
    first.annotation_thunk = true;
    second.annotation_thunk = true;
    assert_ne!(locals_tuple_key(&first), locals_tuple_key(&second));
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
fn marshal_plan_is_preorder_with_nested_subtree_boundaries() {
    let nested = code_object(vec![Constant::Tuple(vec![Constant::Int(7)])]);
    let code = code_object(vec![
        Constant::Code(Box::new(nested)),
        Constant::Tuple(vec![Constant::String("outer".to_string())]),
    ]);
    let graph = ObjectGraph::for_code(&code);
    let entries = graph.plan().entries().collect::<Vec<_>>();

    assert!(
        matches!(entries[0].value, PlannedValue::Code(value) if std::ptr::eq(value, std::ptr::from_ref(&code)))
    );
    assert_eq!(entries[0].subtree_end, entries.len());
    let root_constants = entries
        .iter()
        .position(|entry| {
            matches!(entry.value, PlannedValue::Constants(values) if std::ptr::eq(values, code.constants.as_slice()))
        })
        .unwrap();
    let Constant::Code(nested) = &code.constants[0] else {
        unreachable!();
    };
    let nested_code = entries
        .iter()
        .position(|entry| {
            matches!(entry.value, PlannedValue::Code(value) if std::ptr::eq(value, nested.as_ref()))
        })
        .unwrap();
    let nested_constants = entries
        .iter()
        .position(|entry| {
            matches!(entry.value, PlannedValue::Constants(values) if std::ptr::eq(values, nested.constants.as_slice()))
        })
        .unwrap();

    assert!(root_constants < nested_code);
    assert!(nested_code < nested_constants);
    assert!(entries[nested_code].subtree_end <= entries[root_constants].subtree_end);
}

#[test]
fn marshal_plan_reuses_ids_and_prunes_repeated_container_subtrees() {
    let tuple = Constant::Tuple(vec![Constant::String("shared".to_string())]);
    let frozen = Constant::FrozenSet(vec![Constant::Int(1), Constant::Int(2)]);
    let code = code_object(vec![tuple.clone(), tuple, frozen.clone(), frozen]);
    let graph = ObjectGraph::for_code(&code);
    let entries = graph.plan().entries().collect::<Vec<_>>();

    for indices in [[0, 1], [2, 3]] {
        let containers = indices.map(|constant_index| {
            entries
                .iter()
                .enumerate()
                .find(
                    |(_, entry)| match (&code.constants[constant_index], entry.value) {
                        (Constant::Tuple(values), PlannedValue::Constants(candidate)) => {
                            std::ptr::eq(values.as_slice(), candidate)
                        }
                        (Constant::FrozenSet(_), PlannedValue::Constant(candidate)) => {
                            std::ptr::eq(
                                std::ptr::from_ref(&code.constants[constant_index]),
                                candidate,
                            )
                        }
                        _ => false,
                    },
                )
                .unwrap()
        });
        assert_eq!(containers[0].1.object, containers[1].1.object);
        assert!(containers[0].1.subtree_end > containers[0].0 + 1);
        assert_eq!(containers[1].1.subtree_end, containers[1].0 + 1);
    }
}

#[test]
fn marshal_plan_preserves_force_mode_and_slice_member_identity() {
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
    let graph = ObjectGraph::for_code(&code);

    let Constant::FrozenSet(values) = &code.constants[0] else {
        unreachable!();
    };
    let forced = graph
        .plan()
        .entries()
        .find(|entry| {
            matches!(entry.value, PlannedValue::Constant(value) if std::ptr::eq(value, std::ptr::from_ref(&values[0])))
        })
        .unwrap();
    assert!(forced.force_reference);

    let slices = [1, 2].map(|index| {
        let Constant::Slice { lower, upper, .. } = &code.constants[index] else {
            unreachable!();
        };
        (lower.as_ref(), upper.as_ref())
    });
    let entry_for = |value: &Constant| {
        graph
            .plan()
            .entries()
            .find(|entry| match entry.value {
                PlannedValue::Constant(candidate) | PlannedValue::SliceMember(candidate) => {
                    std::ptr::eq(candidate, value)
                }
                _ => false,
            })
            .unwrap()
    };
    assert!(!entry_for(slices[0].0).force_reference);
    assert_ne!(entry_for(slices[0].0).object, entry_for(slices[1].0).object);
    assert_eq!(entry_for(slices[0].1).object, entry_for(slices[1].1).object);
}
