use crate::common;

#[test]
fn matches_cpython_empty_finally_join_locations() {
    for outer in ["while flag:", "for outer in values:"] {
        for finalizer in [
            "while flag:\n                break",
            "while flag:\n                call()\n                break",
            "while flag:\n                if other:\n                    break\n                call()",
            "while flag:\n                break\n            call()",
            "if other:\n                while flag:\n                    break",
            "for item in values:\n                break",
            "pass",
            "if other:\n                pass",
        ] {
            let source = format!(
                "def f(values, flag, other, call):\n    {outer}\n        try:\n            if flag:\n                break\n        finally:\n            {finalizer}\n"
            );
            common::assert_matches_cpython_marshal(&source, "empty_finally_join_locations.py");
        }
    }
}

#[test]
fn matches_cpython_terminal_while_assertion_joins() {
    common::assert_matches_cpython_marshal(
        r"def f(flag, stop, first, second):
    while flag:
        if stop:
            break
        try:
            assert first or second
        except Exception:
            pass
",
        "terminal_while_assertion_joins.py",
    );
}
