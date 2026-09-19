use crate::common;

#[test]
fn matches_cpython_conditional_value_boolean_continuation_locations() {
    let source = include_str!("../fixtures/conditional_value_boolean_continuation.py");
    common::assert_matches_cpython_marshal(source, "conditional_value_boolean_continuation.py");
}

#[test]
fn matches_cpython_multiline_negated_comparison_markers() {
    for (filename, source) in [
        (
            "generator_multiline_negated_comparison_marker_target.py",
            r#"def rows(values):
    for value in values:
        value["flag"] = not (
            total(value) >= 2
        )
        yield value
"#,
        ),
        (
            "generator_same_line_negated_comparison_marker_control.py",
            r#"def rows(values):
    for value in values:
        value["flag"] = not (total(value) >= 2)
        yield value
"#,
        ),
        (
            "generator_multiline_comparison_distinct_predecessor_control.py",
            r#"def rows(values):
    for value in values:
        value["flag"] = not (
            total(value)
            >= limit(value)
        )
        yield value
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
