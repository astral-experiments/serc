use crate::common;

#[test]
fn matches_cpython_marshal_for_negated_none_comprehension_filters() {
    for condition in [
        "item is None",
        "item is not None",
        "item.left is None and item.right is None",
        "item.left is None or item.right is None",
        "item.left is not None and item.right is not None",
        "item.left is not None or item.right is not None",
    ] {
        let clause = format!("item for item in values if not ({condition})");
        for expression in [
            format!("[{clause}]"),
            format!("({clause})"),
            format!("{{{clause}}}"),
            format!("{{item: {clause}}}"),
        ] {
            let source = format!("def f(values):\n    return {expression}\n");
            common::assert_matches_cpython_marshal(&source, "negated_none_filter.py");
        }
    }
}

#[test]
fn matches_cpython_marshal_for_protected_negated_none_filters() {
    for condition in [
        "item is None",
        "item.left is None and item.right is None",
        "item.left is None\n        and item.right is None",
    ] {
        let expression = format!("[item for item in values if not ({condition})]");
        for body in [
            format!("    with manager():\n        return {expression}\n"),
            format!(
                "    try:\n        return {expression}\n    except Error:\n        recover()\n"
            ),
        ] {
            for prefix in ["def", "async def"] {
                let source = format!("{prefix} f(values):\n{body}");
                common::assert_matches_cpython_marshal(&source, "protected_negated_none_filter.py");
            }
        }
    }
}
