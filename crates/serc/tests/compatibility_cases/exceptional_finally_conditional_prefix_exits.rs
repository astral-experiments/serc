use crate::common;

#[test]
fn matches_cpython_marshal_for_exceptional_finally_conditional_prefix_exits() {
    for condition in [
        "first",
        "first and second",
        "first or second",
        "not (first and second)",
        "not (first or second)",
        "first is None and second == 1",
        "first and second and third",
        "first or second or third",
        "first and (second or third)",
        "first if third else second",
    ] {
        for body in ["cleanup()", "if third:\n                cleanup()"] {
            let source = format!(
                "def f(first, second, third):\n    try:\n        work()\n    finally:\n        prepare()\n        if {condition}:\n            {body}\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "exceptional_finally_conditional_prefix_exits.py",
            );
        }
    }
}

#[test]
fn matches_cpython_marshal_for_copied_finally_condition_nops() {
    let source = r"def boolean_condition(manager, left, right):
    with manager:
        try:
            assert body()
        finally:
            if left and right:
                cleanup()


def conditional_expression(manager, condition, left, right):
    with manager:
        try:
            assert body()
        finally:
            if left if condition else right:
                cleanup()


def negated_boolean_condition(manager, left, right):
    with manager:
        try:
            assert body()
        finally:
            if not (left or right):
                cleanup()
";
    common::assert_matches_cpython_marshal(source, "copied_finally_condition_nops.py");
}
