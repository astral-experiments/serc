use crate::common;

#[test]
fn retains_while_break_owner_before_split_source_continuation() {
    for (scope, work) in [
        ("def", "work()"),
        ("def", "yield work()"),
        ("async def", "await work()"),
        ("async def", "yield await work()"),
    ] {
        for condition in ["True", "active"] {
            for conditional_arm in [false, true] {
                let body =
                    format!("while {condition}:\n    if pid == 0:\n        break\n    {work}\n");
                let body = if conditional_arm {
                    format!("if ready:\n{}", common::indent(&body))
                } else {
                    body
                };
                let protected = format!("{body}if next_ready:\n    pass\n");
                let body = format!(
                    "while True:\n    try:\n{}    except Error:\n        pass\n",
                    common::indent(&common::indent(&protected)),
                );
                let source = format!(
                    "{scope} f(ready, next_ready, active, pid):\n{}",
                    common::indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "while_split_tail_continuation.py");
            }
        }
    }
}
