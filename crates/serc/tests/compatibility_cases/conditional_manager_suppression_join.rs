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

#[test]
fn original_populated_manager_branches_keep_their_pop_record() {
    for prefix in ["def", "async def"] {
        for donor in [
            "value = work()",
            "box.total += value",
            "value: int = work()",
        ] {
            for terminal in ["break", "return value"] {
                for outer_try in [false, true] {
                    let body = format!(
                        "while ready():\n    with manager():\n        if flag:\n            {donor}\n        else:\n            {terminal}\n"
                    );
                    let body = if outer_try {
                        format!(
                            "try:\n    {}\nexcept ValueError:\n    recover()\n",
                            body.trim_end().replace('\n', "\n    ")
                        )
                    } else {
                        body
                    };
                    let source = format!(
                        "{prefix} f(manager, ready, flag, work, value, box):\n    {}\n    finish(value)\n",
                        body.trim_end().replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(&source, "populated_manager_branch.py");
                }
            }
        }
    }
}
