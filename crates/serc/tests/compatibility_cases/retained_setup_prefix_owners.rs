use crate::common::{assert_matches_cpython_marshal, indent};

#[test]
fn matches_cpython_retained_setup_original_prefix_owners() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        for with_manager in [false, true] {
            for (joined, calls) in [
                (true, 2),
                (true, 3),
                (true, 4),
                (true, 7),
                (false, 4),
                (false, 7),
            ] {
                let body = format!(
                    "{}{}marker = None\ntry:\n    if value is not None:\n        {}consume(value)\n    else:\n        fallback()\nexcept Error:\n    recover()\n",
                    if joined {
                        "if flag:\n    prepare()\n"
                    } else {
                        ""
                    },
                    "work()\n".repeat(calls),
                    if matches!(kind, "coroutine" | "async_generator") {
                        "await "
                    } else {
                        ""
                    },
                );
                let body = if with_manager {
                    format!("with manager:\n{}", indent(&body))
                } else {
                    body
                };
                let source = format!(
                    "{}def f(value, flag, manager):\n{}{}",
                    if matches!(kind, "coroutine" | "async_generator") {
                        "async "
                    } else {
                        ""
                    },
                    if matches!(kind, "generator" | "async_generator") {
                        "    yield None\n"
                    } else {
                        ""
                    },
                    indent(&body),
                );
                assert_matches_cpython_marshal(
                    &source,
                    &format!("retained_setup_prefix_{kind}_{with_manager}_{joined}_{calls}.py"),
                );
            }
        }
    }
}

#[test]
fn matches_cpython_retained_setup_predecessor_owners() {
    for (filename, source) in [
        (
            "sync_nested_finally_none_global3.py",
            r"def f(resource, result, skip):
    try:
        if skip:
            return result
        tick_1()
        tick_2()
        tick_3()
        return result
    finally:
        try:
            if resource is not None:
                resource.close()
        except Exception:
            pass
",
        ),
        (
            "async_cell_nested_finally_none_global3.py",
            r"async def f(resource, items, skip):
    tasks = (resource for item in items)
    try:
        if skip:
            return
        tick_1()
        tick_2()
        tick_3()
    finally:
        try:
            if resource is not None:
                await resource.close()
        except Exception:
            pass
",
        ),
        (
            "sync_nested_finally_none_global4.py",
            r"def f(resource, result, skip):
    try:
        if skip:
            return result
        tick_1()
        tick_2()
        tick_3()
        tick_4()
        return result
    finally:
        try:
            if resource is not None:
                resource.close()
        except Exception:
            pass
",
        ),
        (
            "async_cell_nested_finally_none_global4.py",
            r"async def f(resource, items, skip):
    tasks = (resource for item in items)
    try:
        if skip:
            return
        tick_1()
        tick_2()
        tick_3()
        tick_4()
    finally:
        try:
            if resource is not None:
                await resource.close()
        except Exception:
            pass
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
