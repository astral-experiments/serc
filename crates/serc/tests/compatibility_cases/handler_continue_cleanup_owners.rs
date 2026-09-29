use crate::common;

#[test]
fn handler_continues_end_enclosing_tries_after_exception_cleanup() {
    for prefix in ["", "async "] {
        for name in ["", " as error"] {
            for loop_header in ["while flag:", "for item in items:"] {
                for nested in [false, true] {
                    let handler = format!(
                        "try:\n    work()\nexcept Error{name}:\n    observe()\n    continue"
                    );
                    let mut body = format!(
                        "try:\n    {}\nexcept Other:\n    recover()",
                        handler.replace('\n', "\n    "),
                    );
                    if nested {
                        body = format!(
                            "try:\n    {}\nexcept Outer:\n    recover()",
                            body.replace('\n', "\n    "),
                        );
                    }
                    let source = format!(
                        "{prefix}def f(flag, items):\n    {loop_header}\n        {}\n    finish()\n",
                        body.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_continue_owner.py");
                }
            }
        }
    }
}

#[test]
fn handler_continue_after_await_keeps_only_the_coroutine_jump_owner() {
    for name in ["", " as error"] {
        let source = format!(
            "async def f():\n    while True:\n        try:\n            await sleep()\n            try:\n                work()\n            except Error{name}:\n                observe()\n                continue\n        except Other:\n            break\n"
        );
        common::assert_matches_cpython_marshal(&source, "await_handler_continue_owner.py");
    }
}

#[test]
fn handler_breaks_end_enclosing_tries_after_exception_cleanup() {
    // The handler cleanup retains the middle try's owner. Removing the loop
    // iterator happens afterward, while only the outer try remains active.
    for (prefix, suspend) in [("", ""), ("async ", "await ")] {
        for (name, value) in [("", "item"), (" as error", "error")] {
            let source = format!(
                "{prefix}def f(items, stop):\n    try:\n        for item in items:\n            try:\n                try:\n                    {suspend}work(item)\n                except ValueError{name}:\n                    observe({value})\n                    if stop:\n                        break\n            except TypeError:\n                recover()\n        {suspend}finish()\n    except RuntimeError:\n        report()\n    return None\n"
            );
            common::assert_matches_cpython_marshal(&source, "handler_break_enclosing_try_owner.py");
        }
    }
}

#[test]
fn matches_cpython_break_try_inside_manager_pop_owner() {
    for (definition, manager, suspension) in [
        ("def", "with", ""),
        ("async def", "with", "await "),
        ("async def", "async with", "await "),
    ] {
        let source = format!(
            "{definition} f(manager, action, recover, Error):\n    while True:\n        {manager} manager:\n            try:\n                {suspension}action()\n                break\n            except Error:\n                recover()\n    return 0\n"
        );
        common::assert_matches_cpython_marshal(&source, "break_try_manager_pop.py");
    }
}
