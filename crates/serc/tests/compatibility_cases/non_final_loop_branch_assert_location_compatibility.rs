use crate::common;

#[test]
fn matches_cpython_non_final_loop_conditional_assertion_successes() {
    for loop_header in ["for item in items", "while items"] {
        for condition in [
            "left if selector else right",
            "left == expected if check(selector) else right",
            "left == expected if selector is expected else RIGHT",
            "left is None if selector else right is not None",
            "not left if selector else not right",
            "left if True else right",
        ] {
            let source = format!(
                "def f(items, selector, left, right, expected, branch):\n    {loop_header}:\n        if branch:\n            assert {condition}\n        else:\n            consume(left)\n"
            );
            common::assert_matches_cpython_marshal(&source, "loop_conditional_assertion.py");
        }
    }
}
