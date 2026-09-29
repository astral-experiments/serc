use crate::common;

#[test]
fn matches_cpython_marshal_for_boolean_conditional_assertion_joins() {
    for (prefix, suspension) in [
        ("", ""),
        ("async ", ""),
        ("async ", "    await call()\n"),
        ("", "    yield None\n"),
    ] {
        for expression in [
            "value if flag else other and tail",
            "check(value) if flag else check(other) and check(tail)",
            "value if flag else other or tail",
        ] {
            for multiline in [false, true] {
                let condition = if multiline {
                    format!("(\n        {expression}\n    )")
                } else {
                    format!("({expression})")
                };
                for message in ["value", "call(value)"] {
                    let source = format!(
                        "{prefix}def f(value, flag, other, tail):\n{suspension}    assert {condition}, {message}\n    return value\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "boolean_conditional_assertion_joins.py",
                    );
                }
            }
        }
    }
}
