use crate::common;

#[test]
fn matches_cpython_nested_setup_nop_raise_copies() {
    common::assert_matches_cpython_marshal(
        include_str!("../fixtures/nested_setup_nop_raise.py"),
        "nested_setup_nop_raise.py",
    );
}
