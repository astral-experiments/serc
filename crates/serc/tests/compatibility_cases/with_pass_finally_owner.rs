use crate::common;

#[test]
fn empty_finally_at_manager_tail_donates_its_line_to_pop_block() {
    for (prefix, body) in [
        ("def f(manager):", "work()"),
        ("async def f(manager):", "await work()"),
        ("def f(manager):", "yield 1"),
        ("async def f(manager):", "yield 1"),
    ] {
        for following in ["", "    other()\n"] {
            let suite =
                format!("    try:\n        {body}\n    finally:\n        pass\n{following}");
            for nested in [false, true] {
                let source = if nested {
                    format!(
                        "{prefix}\n    with manager:\n        with manager:\n{}",
                        common::indent(&common::indent(&suite))
                    )
                } else {
                    format!("{prefix}\n    with manager:\n{}", common::indent(&suite))
                };
                common::assert_matches_cpython_marshal(&source, "with_pass_finally_owner.py");
            }
        }
    }
}
