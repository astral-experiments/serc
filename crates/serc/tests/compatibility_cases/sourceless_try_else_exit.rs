use crate::common;

#[test]
fn shared_false_else_exit_in_nested_loop_try_stays_unlocated() {
    for condition in ["a and b", "a and b and c"] {
        let source = format!(
            "def f(items, a, b, c, value):\n    while a:\n        try:\n            prepare()\n        except OuterError:\n            pass\n        else:\n            try:\n                work()\n            except InnerError:\n                pass\n            else:\n                if {condition}:\n                    return value\n        advance()\n    return None\n"
        );
        common::assert_matches_cpython_marshal(&source, "shared_false_nested_try_else_exit.py");
    }
}
