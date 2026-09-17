use crate::common;

#[test]
fn matches_cpython_manager_assertion_arm_cleanup_join() {
    let source = r#"def check(outer, inner, enabled, scores):
    with (
        outer,
        inner,
    ):
        if enabled:
            assert scores["value"] < 1.0, f"unexpected value: {scores['value']}"
        else:
            raise ValueError(f"disabled: {enabled}")
"#;
    common::assert_matches_cpython_marshal(source, "manager_assertion_arm_cleanup_join.py");
}
