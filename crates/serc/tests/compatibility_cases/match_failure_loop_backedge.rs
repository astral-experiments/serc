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

#[test]
fn matches_cpython_or_pattern_before_pass_loop_tail() {
    for guard in ["", " if True", " if predicate(value)"] {
        let source = format!(
            "def f(value, flag):\n    while flag:\n        match value:\n            case 1 | 2{guard}:\n                pass\n        pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "or_before_pass_loop_tail.py");
    }
}
