use crate::common;

#[test]
fn matches_cpython_shared_handler_break_jumps() {
    for prefix in [
        "",
        "            if tick():\n                await refresh()\n",
    ] {
        for protected in ["await task", "work()"] {
            for handler in ["pass", "report()"] {
                for body in ["await work()", "yield value"] {
                    for cleanup in [
                        "cleanup()",
                        "if pending():\n            await cleanup()",
                        "cleanup(task)",
                        "if task.done():\n            await cleanup(task)",
                    ] {
                        let source = format!(
                            "async def f(task, value):\n    try:\n        while True:\n{prefix}            if ready():\n                try:\n                    {protected}\n                except Error:\n                    {handler}\n                break\n            {body}\n    finally:\n        {cleanup}\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "shared_handler_break_jumps.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_overriding_return_after_with_breaks() {
    for (prefix, suspend) in [("", ""), ("async ", ""), ("", "    yield\n")] {
        for condition in ["True", "1"] {
            for footer in ["", "    finish()\n"] {
                let source = format!(
                    "{prefix}def f():\n{suspend}    try:\n        while {condition}:\n            with context():\n                break\n    finally:\n        return\n{footer}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "overriding_return_after_with_break.py",
                );
            }
        }
    }
}
