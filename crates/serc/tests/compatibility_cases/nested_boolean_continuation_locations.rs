use crate::common;

#[test]
fn matches_cpython_nested_boolean_threaded_jump_markers() {
    for (operator, other) in [("and", "or"), ("or", "and")] {
        for count in 2..=4 {
            let inner = ["c", "d", "e", "g"][..count].join(&format!("\n            {operator} "));
            let source = format!(
                "def process(a, b, c, d, e, f, g):\n    return (\n        a {operator} b {other} (\n            {inner}\n        )\n    ) {operator} not f\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_boolean_threaded_markers.py");
        }
    }
}

#[test]
fn matches_cpython_nested_boolean_continuation_locations() {
    let expressions = [
        "a and ((b and c) or (d and e)) and f",
        "a or ((b or c) and (d or e)) or f",
        "a and ((b and (c or d)) or (e and f))",
        "a or ((b or (c and d)) and (e or f))",
        "((a and (b and c)) and d) and e",
        "((a or (b or c)) or d) or e",
        "((a and (b or (c and d))) or e) and f",
        "((a or (b and (c or d))) and e) or f",
        "a and (b or (c and (d or e))) and f",
        "a or (b and (c or (d and e))) or f",
        "a and ((False or True) or b) and c",
        "a or ((True and False) and b) or c",
    ];
    for expression in expressions {
        let source = format!("def f(a, b, c, d, e, f):\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "nested_boolean_locations.py");
    }
}

#[test]
fn matches_cpython_threaded_boolean_value_exception_owners() {
    for expression in [
        "(a and (b or c)) or d",
        "(a or (b and c)) and d",
        "(a and b) or c",
        "(a or b) and c",
    ] {
        for source in [
            format!("async def f(a, b, c, d):\n    return {expression}\n"),
            format!(
                "def f(a, b, c, d):\n    try:\n        return {expression}\n    except Exception:\n        return None\n"
            ),
        ] {
            common::assert_matches_cpython_marshal(&source, "threaded_boolean_owners.py");
        }
    }
}
