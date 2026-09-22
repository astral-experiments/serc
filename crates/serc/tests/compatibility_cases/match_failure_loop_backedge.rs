use crate::common;

#[test]
fn matches_cpython_non_final_match_failure_backedges() {
    for pattern in [
        "C(value=False)",
        "C(value='typing')",
        "C(False, value=False)",
        "[False]",
    ] {
        for body in ["return True", "consume(x)"] {
            common::assert_matches_cpython_marshal(
                &format!(
                    "def f(values):\n    for x in values:\n        if cond(x):\n            match g(x):\n                case {pattern}:\n                    {body}\n        elif other(x):\n            match g(x):\n                case D(name='typing'):\n                    return True\n    return False\n"
                ),
                "non_final_match_failure.py",
            );
        }
    }
}
