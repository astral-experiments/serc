use crate::common;

#[test]
fn nested_raising_finally_keeps_manager_exit_marker() {
    for manager in ["with", "async with"] {
        for body in [
            "try:\n    raise ValueError\nexcept Exception as error:\n    try:\n        raise KeyError\n    finally:\n        cleanup()",
            "try:\n    try:\n        raise ValueError\n    finally:\n        await checkpoint()\nfinally:\n    try:\n        raise KeyError\n    finally:\n        cleanup()",
            "try:\n    raise ValueError\nexcept Exception:\n    cleanup()",
            "try:\n    if flag:\n        raise ValueError\nfinally:\n    cleanup()",
            "try:\n    raise ValueError\nfinally:\n    return result",
        ] {
            let body = body.replace('\n', "\n            ");
            let source = format!(
                "async def f(flag, result):
    with outer():
        {manager} inner():
            {body}
    finish()
"
            );
            common::assert_matches_cpython_marshal(&source, "nested_raising_manager_marker.py");
        }
    }
}
