use crate::common;

#[test]
fn matches_cpython_return_copies_after_chained_boolean_joins() {
    for prefix in ["", "async "] {
        for condition in [
            "ready or a < b < c",
            "not (ready and a < b < c)",
            "a < b < c or ready",
            "first or a < b < c or ready",
            "ready and a < b < c",
            "first and a < b < c and ready",
        ] {
            for returned in [
                "return",
                "return None",
                "return value",
                "return value + 1",
                "return value.attr",
                "return value.method()",
                "return work(value)",
            ] {
                let source = format!(
                    "{prefix}def f(a, b, c, ready, first, value, fallback):\n    if {condition}:\n        {returned}\n    return fallback\n",
                );
                common::assert_matches_cpython_marshal(&source, "chained_boolean_return_copies.py");
            }
        }
    }
}

#[test]
fn matches_cpython_protected_return_copies_after_chained_boolean_joins() {
    for returned in ["return", "return value", "return value.attr"] {
        let source = format!(
            "def f(a, b, c, ready, value, fallback):\n    try:\n        if not (ready and a < b < c):\n            {returned}\n    except Error:\n        report()\n    return fallback\n",
        );
        common::assert_matches_cpython_marshal(
            &source,
            "protected_chained_boolean_return_copies.py",
        );
    }
}
