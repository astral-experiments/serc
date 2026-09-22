use crate::common;

#[test]
fn preserves_original_chained_comparison_exit_copies() {
    common::assert_matches_cpython_marshal(
        include_str!("../fixtures/chained_comparison_original_exit_copies.py"),
        "chained_comparison_original_exit_copies.py",
    );
}
