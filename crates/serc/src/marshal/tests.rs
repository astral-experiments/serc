use crate::code_object::{CodeObject, Constant, LocalsIdentity, MarshalHints};

use super::encode_code;
use super::graph::ObjectGraph;
use super::key::{
    ObjectKey, code_bytes_key, code_key, constant_key, constant_sort_key, constants_tuple_key,
    local_kinds_key, locals_tuple_key, slice_member_uses_identity, strings_tuple_key,
};

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
        marshal_hints: MarshalHints::default(),
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
    code.marshal_hints
        .explicitly_interned_strings
        .insert("méthode".to_string());
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
    second.marshal_hints.locals_identity = LocalsIdentity::Fresh;
    assert_ne!(locals_tuple_key(&first), locals_tuple_key(&second));
    first.marshal_hints.locals_identity = LocalsIdentity::Fresh;
    assert_ne!(locals_tuple_key(&first), locals_tuple_key(&second));
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
