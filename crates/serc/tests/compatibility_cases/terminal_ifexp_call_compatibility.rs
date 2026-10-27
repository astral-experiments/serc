use crate::common;

#[test]
fn preserves_terminal_if_discarded_conditional_before_shared_footer() {
    for (function, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for outer in ["flag", "True"] {
            for expression in [
                "left() if choose else right()",
                "left if choose else right",
                "(left() if other else fallback()) if choose else right()",
            ] {
                let source = format!(
                    "{function} f(flag, choose, other, left, right, value):\n{prefix}    if {outer}:\n        {expression}\n"
                );
                common::assert_matches_cpython_marshal(&source, "terminal_if_discarded_join.py");
            }
        }
    }
}
