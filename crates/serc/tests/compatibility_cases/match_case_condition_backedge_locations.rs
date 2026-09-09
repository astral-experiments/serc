use crate::common;

#[test]
fn matches_cpython_match_case_condition_backedge_locations() {
    for (function, suspension, return_statement) in [
        ("def", "", "return other"),
        ("async def", "", "return other"),
        ("def", "    yield a\n", "return other"),
        ("async def", "    yield a\n", "return"),
    ] {
        for loop_header in ["for item in items:", "while active:"] {
            for condition in [
                "a and b",
                "not (a or b)",
                "(\n                    a is not None\n                    and b is not None\n                )",
                "a or b",
            ] {
                for body in [return_statement, "raise Error()", "work()", "pass"] {
                    let source = format!(
                        "{function} f(items, active, item, a, b, other):\n{suspension}    {loop_header}\n        match item:\n            case Kind.X:\n                if {condition}:\n                    {body}\n            case _:\n                pass\n    return\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "match_case_condition_backedge_locations.py",
                    );
                }
            }
        }
    }
}
