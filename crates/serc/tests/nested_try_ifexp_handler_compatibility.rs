#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_nested_try_ifexp_handler_layouts() {
    let source = include_str!("fixtures/nested_try_ifexp_handler_matrix.py");
    common::assert_matches_cpython_marshal(source, "nested_try_ifexp_handler_matrix.py");
}

#[test]
fn matches_cpython_marshal_for_nested_try_ifexp_owner_slots() {
    let source = include_str!("fixtures/nested_try_ifexp_owner_slots.py");
    common::assert_matches_cpython_marshal(source, "nested_try_ifexp_owner_slots.py");
}
