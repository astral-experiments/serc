use crate::common;

#[test]
fn returns_unwind_try_markers_before_manager_cleanup() {
    for (prefix, manager) in [("", "with"), ("async ", "with"), ("async ", "async with")] {
        for value in ["", "True", "None", "3", "value", "work()"] {
            for binding in ["", " as error"] {
                let source = format!(
                    "{prefix}def f(value):\n    {manager} manager():\n        try:\n            consume()\n            return {value}\n        except Error{binding}:\n            recover()\n            return False\n"
                );
                common::assert_matches_cpython_marshal(&source, "return_try_manager_nop.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_try_and_loop_manager_exit_records() {
    for (filename, source) in [
        (
            "outer_finally_sync_manager_try_exits.py",
            r"def f(manager):
    try:
        while True:
            with manager:
                try:
                    if retry():
                        continue
                    return 70
                except ValueError:
                    recover()
            break
    finally:
        cleanup()
",
        ),
        (
            "nested_ordinary_try_manager_computed_return.py",
            r"async def f(manager):
    with manager:
        try:
            prepare()
            try:
                result = await fetch()
                return (
                    accept()
                    if result
                    else reject()
                )
            except ValueError:
                recover()
            return fallback()
        except TypeError:
            return fallback()
",
        ),
        (
            "outer_finally_async_manager_try_return.py",
            r"async def f(manager, active):
    try:
        async with manager:
            while active:
                await tick()
                try:
                    if ready():
                        record()
                        return
                except ValueError:
                    recover()
    finally:
        if active:
            await cleanup()
",
        ),
        (
            "single_ordinary_try_manager_computed_return_control.py",
            r"async def f(manager):
    with manager:
        prepare()
        try:
            result = await fetch()
            return (
                accept()
                if result
                else reject()
            )
        except ValueError:
            recover()
        return fallback()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
