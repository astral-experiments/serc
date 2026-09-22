use crate::common::assert_matches_cpython_marshal;

#[test]
fn keeps_protected_chained_comparison_exit_forward() {
    assert_matches_cpython_marshal(
        include_str!("../fixtures/non_final_generator_chained_exit.py"),
        "non_final_generator_chained_exit.py",
    );
}

#[test]
fn keeps_non_final_try_while_exit_forward() {
    assert_matches_cpython_marshal(
        include_str!("../fixtures/non_final_normal_try_while_exit.py"),
        "non_final_normal_try_while_exit.py",
    );
}
