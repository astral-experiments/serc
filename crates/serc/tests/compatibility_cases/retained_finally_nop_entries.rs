use crate::common;

#[test]
fn matches_cpython_retained_normal_exit_into_finally_loop() {
    for (prefix, manager, produce) in [
        ("", "with", "yield pending[0]"),
        ("async ", "async with", "yield await pending[0]"),
    ] {
        let source = format!(
            "{prefix}def f(manager, pending, exhausted):\n    {manager} manager:\n        try:\n            while pending or not exhausted:\n                if pending:\n                    {produce}\n                    pending.pop()\n                else:\n                    exhausted = True\n        except GeneratorExit:\n            pass\n        finally:\n            for task in pending:\n                task.cancel()\n"
        );
        common::assert_matches_cpython_marshal(&source, "retained_finally_loop.py");
    }
}

#[test]
fn preserves_empty_normal_finally_arrays_and_erased_exit_jumps() {
    for (name, body, cleanup) in [
        (
            "empty_cleanup_array",
            "while pending or not exhausted:\n    consume(pending)\n    exhausted = True",
            "while pending:\n    consume(pending.pop())",
        ),
        (
            "erased_normal_exit",
            "consume(pending)",
            "for task in pending:\n    task.cancel()",
        ),
        (
            "conditional_exit",
            "if exhausted:\n    consume(pending)\nelse:\n    consume(pending)",
            "for task in pending:\n    task.cancel()",
        ),
        (
            "copied_return_cleanup",
            "if exhausted:\n    return pending\nconsume(pending)",
            "for task in pending:\n    task.cancel()",
        ),
    ] {
        let source = format!(
            "def f(pending, exhausted):\n    try:\n        {}\n    except Error:\n        pass\n    finally:\n        {}\n",
            body.replace('\n', "\n        "),
            cleanup.replace('\n', "\n        "),
        );
        common::assert_matches_cpython_marshal(&source, &format!("retained_finally_{name}.py"));
    }
}
