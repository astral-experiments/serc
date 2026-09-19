use crate::common;

#[test]
fn matches_cpython_exceptional_finally_nested_continuations() {
    for (function, body) in [
        ("def", "work()"),
        ("def", "yield value"),
        ("async def", "await work()"),
        ("async def", "yield value"),
    ] {
        for cleanup in [
            "        if flag:\n            cleanup()\n        finish()\n",
            "        try:\n            if flag:\n                cleanup()\n        finally:\n            finish()\n",
        ] {
            let source = format!(
                "{function} f(value, flag):\n    try:\n        {body}\n    finally:\n{cleanup}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "exceptional_finally_nested_continuations.py",
            );
        }
    }
}
