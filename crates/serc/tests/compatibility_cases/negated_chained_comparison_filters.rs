use crate::common;

#[test]
fn matches_cpython_negated_chained_comparison_filters() {
    for expression in [
        "(item for item in items if CONDITION)",
        "[item for item in items if CONDITION]",
        "{item for item in items if CONDITION}",
        "{item: item for item in items if CONDITION}",
    ] {
        for condition in [
            "not (0 < item < 10)",
            "flag and not (lower() <= item < upper())",
            "not (left is item is right) or flag",
            "not (left is item is None)",
        ] {
            let expression = expression.replace("CONDITION", condition);
            let source = format!("def f(items, left, right, flag):\n    return {expression}\n");
            common::assert_matches_cpython_marshal(&source, "negated_chained_filters.py");
        }
    }
}

#[test]
fn matches_cpython_multiline_chained_none_filters() {
    for condition in [
        "left is\n            item is\n            None",
        "left is\n            item is not None",
    ] {
        for prefix in ["", "not "] {
            let source = format!(
                "def f(items, left):\n    return [item for item in items if {prefix}({condition})]\n"
            );
            common::assert_matches_cpython_marshal(&source, "multiline_chained_none_filters.py");
        }
    }
}

#[test]
fn matches_cpython_chained_none_filter_separate_element_line() {
    let source = r"def f(items, left):
    return [item for item in items
            if left is item is None]
";
    common::assert_matches_cpython_marshal(
        source,
        "matches_cpython_chained_none_filter_separate_element_line.py",
    );
}
