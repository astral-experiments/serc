use crate::common;

#[test]
fn matches_cpython_terminal_condition_assertion_exits() {
    for condition in [
        "a if selector else b",
        "(a and b) if selector() else (c or d)",
    ] {
        let source =
            format!("def f(a, b, c, d, selector):\n    if outer:\n        assert {condition}\n");
        common::assert_matches_cpython_marshal(&source, "terminal_condition_assertion.py");
    }
}

#[test]
fn matches_cpython_terminal_condition_try_exits() {
    for condition in ["a is None or check(a)", "a if selector else b", "a or True"] {
        let source = format!(
            "def f(a, b, selector):\n    try:\n        work()\n    except Error:\n        pass\n    else:\n        assert {condition}\n"
        );
        common::assert_matches_cpython_marshal(&source, "terminal_condition_try.py");
    }
}

#[test]
fn matches_cpython_terminal_condition_loop_exits() {
    for condition in ["a or (b and c)", "a if selector else b", "a and False"] {
        let source = format!(
            "def f(a, b, c, selector):\n    if outer:\n        yield from work()\n    else:\n        while {condition}:\n            work()\n"
        );
        common::assert_matches_cpython_marshal(&source, "terminal_condition_loop.py");
    }
}
