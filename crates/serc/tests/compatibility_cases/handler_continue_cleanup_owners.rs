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
fn ordinary_try_continue_before_sync_manager_exit() {
    let source = r"def f(manager, values, skip, observe):
    for value in values:
        with manager():
            try:
                observe(value)
                if skip(value):
                    continue
            except ValueError:
                observe(None)
    return None
";
    common::assert_matches_cpython_marshal(
        source,
        "ordinary_try_continue_before_sync_manager_exit.py",
    );
}

#[test]
fn ordinary_try_continue_before_async_manager_exit() {
    let source = r"async def f(manager, flag, skip, observe):
    while flag():
        async with manager():
            try:
                await observe()
                if skip():
                    continue
            except ValueError:
                await observe()
    return None
";
    common::assert_matches_cpython_marshal(
        source,
        "ordinary_try_continue_before_async_manager_exit.py",
    );
}

#[test]
fn nested_tries_continue_before_manager_exits() {
    let source = r"def f(outer, inner, values, skip, observe):
    for value in values:
        with outer():
            with inner():
                try:
                    try:
                        if skip(value):
                            observe(value)
                            continue
                    except ValueError:
                        observe(None)
                except TypeError:
                    observe(None)
    return None
";
    common::assert_matches_cpython_marshal(source, "nested_tries_continue_before_manager_exits.py");
}

#[test]
fn continue_inside_manager_that_stays_active() {
    let source = r"def f(manager, values, skip, observe):
    with manager():
        for value in values:
            try:
                if skip(value):
                    continue
                observe(value)
            except ValueError:
                observe(None)
    return None
";
    common::assert_matches_cpython_marshal(source, "continue_inside_manager_that_stays_active.py");
}

#[test]
fn continue_in_manager_inside_ordinary_try() {
    let source = r"def f(manager, values, skip, observe):
    for value in values:
        try:
            with manager():
                if skip(value):
                    continue
                observe(value)
        except ValueError:
            observe(None)
    return None
";
    common::assert_matches_cpython_marshal(source, "continue_in_manager_inside_ordinary_try.py");
}

#[test]
fn continue_without_manager_cleanup() {
    let source = r"def f(values, skip, observe):
    for value in values:
        try:
            if skip(value):
                continue
            observe(value)
        except ValueError:
            observe(None)
    return None
";
    common::assert_matches_cpython_marshal(source, "continue_without_manager_cleanup.py");
}

#[test]
fn continue_with_terminating_finally_keeps_existing_cleanup() {
    let source = r#"def f(manager, values, skip, observe):
    for value in values:
        with manager():
            try:
                if skip(value):
                    continue
                observe(value)
            finally:
                raise RuntimeError("stop")
"#;
    common::assert_matches_cpython_marshal(
        source,
        "continue_with_terminating_finally_keeps_existing_cleanup.py",
    );
}

#[test]
fn raising_finally_overrides_break_before_manager_exit() {
    let source = r#"def f(manager, values, stop, observe):
    for value in values:
        with manager():
            try:
                if stop(value):
                    break
                observe(value)
            finally:
                raise RuntimeError("stop")
"#;
    common::assert_matches_cpython_marshal(
        source,
        "raising_finally_overrides_break_before_manager_exit.py",
    );
}

#[test]
fn raising_finally_prefix_runs_before_async_manager_exit() {
    let source = r#"async def f(manager, flag, skip, observe, report):
    while flag():
        async with manager():
            try:
                if skip():
                    continue
                await observe()
            finally:
                report("cleanup")
                raise RuntimeError("stop")
"#;
    common::assert_matches_cpython_marshal(
        source,
        "raising_finally_prefix_runs_before_async_manager_exit.py",
    );
}

#[test]
fn fallthrough_finally_keeps_existing_continue_cleanup() {
    let source = r"def f(manager, values, skip, observe, cleanup):
    for value in values:
        with manager():
            try:
                if skip(value):
                    continue
                observe(value)
            finally:
                cleanup(value)
";
    common::assert_matches_cpython_marshal(
        source,
        "fallthrough_finally_keeps_existing_continue_cleanup.py",
    );
}

#[test]
fn returning_finally_keeps_existing_loop_override() {
    let source = r#"def f(manager, values, skip, observe):
    for value in values:
        with manager():
            try:
                if skip(value):
                    continue
                observe(value)
            finally:
                return "done"
"#;
    common::assert_matches_cpython_marshal(
        source,
        "returning_finally_keeps_existing_loop_override.py",
    );
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

#[test]
fn matches_cpython_break_manager_pop_controls() {
    for source in [
        "def f(manager, action):\n    while True:\n        with manager:\n            action()\n            break\n    return 0\n",
        "def f(manager, action, Error):\n    while True:\n        try:\n            with manager:\n                action()\n                break\n        except Error:\n            pass\n    return 0\n",
        "def f(manager, action, Error):\n    while True:\n        with manager:\n            try:\n                action()\n            except Error:\n                break\n    return 0\n",
    ] {
        common::assert_matches_cpython_marshal(source, "break_manager_pop_controls.py");
    }
}

#[test]
fn direct_handler_continues_leave_enclosing_try_regions() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        try:
            try:
                work(item)
            except ValueError:
                continue
        except RuntimeError:
            recover()
",
        "ordinary_for.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(ready):
    while ready():
        try:
            try:
                work()
            except ValueError:
                continue
        except RuntimeError:
            recover()
",
        "ordinary_while.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items, manager):
    with manager:
        for item in items:
            try:
                try:
                    work(item)
                except ValueError:
                    continue
            except RuntimeError:
                recover()
",
        "manager_for.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        try:
            try:
                work(item)
            except ValueError as error:
                continue
        except RuntimeError:
            recover()
",
        "named_for.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(items):
    for item in items:
        try:
            try:
                await work(item)
            except ValueError:
                continue
        except RuntimeError:
            recover()
",
        "coroutine_for.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        try:
            work(item)
        except ValueError:
            continue
",
        "direct_handler_control.py",
    );
}
