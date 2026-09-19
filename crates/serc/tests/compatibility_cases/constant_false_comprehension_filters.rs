use crate::common;

#[test]
fn matches_cpython_marshal_for_constant_false_comprehension_filters() {
    for filter in [
        "if False",
        "if check(item) if False",
        "if False if check(item)",
        "if 0",
        "if 1 == 0",
    ] {
        let clause = format!("item for item in values {filter}");
        for expression in [
            format!("[{clause}]"),
            format!("({clause})"),
            format!("{{item: {clause}}}"),
        ] {
            let source = format!("def f(values):\n    return {expression}\n");
            common::assert_matches_cpython_marshal(&source, "constant_false_filter.py");
        }
    }
}

#[test]
fn matches_cpython_marshal_for_multiline_constant_false_filters() {
    for filter in [
        "if False",
        "if check(item) if False",
        "if False if check(item)",
    ] {
        for (open, close) in [("[", "]"), ("(", ")")] {
            let source = format!(
                "async def f(values):\n    return {open}\n        item\n        for item in values\n        {filter}\n    {close}\n"
            );
            common::assert_matches_cpython_marshal(&source, "multiline_constant_false_filter.py");
        }
    }
}
