use crate::common;

#[test]
fn matches_cpython_finally_calls_reached_by_retained_breaks() {
    for (prefix, cleanup) in [("", "x.close()"), ("async ", "await x.close()")] {
        for (name, body) in [
            (
                "else_arm",
                "try:\n    while True:\n        if pred:\n            x.work()\n        else:\n            if x.ready():\n                break\n            x.wait()\nfinally:\n    x.close()",
            ),
            (
                "after_manager",
                "try:\n    while True:\n        with cm:\n            x.work()\n        if pred:\n            break\n        x.wait()\nfinally:\n    x.close()",
            ),
            (
                "outer_manager",
                "with cm:\n    try:\n        while True:\n            if pred:\n                break\n            x.work()\n    finally:\n        x.close()",
            ),
        ] {
            let source = format!(
                "{prefix}def f(x, pred, cm):\n    {}\n",
                body.replace("x.close()", cleanup).replace('\n', "\n    ")
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("retained_break_finally_{name}.py"),
            );
        }
    }
}

#[test]
fn preserves_cpython_removed_break_and_return_call_entries() {
    for (prefix, cleanup) in [("", "x.close()"), ("async ", "await x.close()")] {
        for (name, body) in [
            (
                "tail_break",
                "try:\n    while True:\n        x.work()\n        if pred:\n            break\nfinally:\n    x.close()",
            ),
            (
                "tail_break_with_manager",
                "with cm:\n    try:\n        while True:\n            x.work()\n            if pred:\n                break\n    finally:\n        x.close()",
            ),
            (
                "ordinary_return_call",
                "while True:\n    if pred:\n        break\n    x.work()\nreturn cleanup(x)",
            ),
        ] {
            let source = format!(
                "{prefix}def f(x, pred, cm):\n    {}\n",
                body.replace("x.close()", cleanup).replace('\n', "\n    ")
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("removed_break_finally_{name}.py"),
            );
        }
    }
}

#[test]
fn matches_cpython_borrows_after_copied_async_manager_exits() {
    for (mode, work) in [("coroutine", "x.work()"), ("async_generator", "yield x")] {
        for (handler_name, handler) in [("ordinary", "except"), ("group", "except*")] {
            let source = format!(
                r"async def f(x, pred, cm):
    timed_out = False
    try:
        async with cm:
            try:
                while True:
                    if pred:
                        timed_out = True
                        break
                    {work}
            finally:
                x.close()
    {handler} E:
        pass
    if timed_out:
        x.done()
"
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("copied_manager_exit_{mode}_{handler_name}.py"),
            );
        }
    }
}
