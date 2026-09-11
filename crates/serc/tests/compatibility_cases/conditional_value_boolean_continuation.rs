use crate::common;

#[test]
fn matches_cpython_conditional_value_boolean_continuation_locations() {
    let source = include_str!("../fixtures/conditional_value_boolean_continuation.py");
    common::assert_matches_cpython_marshal(source, "conditional_value_boolean_continuation.py");
}
