use crate::common;

#[test]
fn matches_cpython_sync_manager_outside() {
    let source = r"def f(manager, flag):
    with manager:
        try:
            ping()
        except Exception:
            pong()
        finally:
            if flag:
                cleanup()
";
    common::assert_matches_cpython_marshal(source, "shared_try_exit_sync_manager_outside.py");
}

#[test]
fn matches_cpython_async_manager_outside() {
    let source = r"async def f(manager, flag):
    async with manager:
        try:
            ping()
        except Exception:
            pong()
        finally:
            if flag:
                cleanup()
";
    common::assert_matches_cpython_marshal(source, "shared_try_exit_async_manager_outside.py");
}

#[test]
fn matches_cpython_sync_manager_in_try() {
    let source = r"def f(manager, flag):
    try:
        with manager:
            ping()
    except Exception:
        pong()
    finally:
        if flag:
            cleanup()
";
    common::assert_matches_cpython_marshal(source, "shared_try_exit_sync_manager_in_try.py");
}

#[test]
fn matches_cpython_except_return_copied_finally() {
    let source = r"def f(flag):
    try:
        ping()
    except Exception:
        pong()
        return 1
    finally:
        if flag:
            cleanup()
";
    common::assert_matches_cpython_marshal(
        source,
        "shared_try_exit_except_return_copied_finally.py",
    );
}

#[test]
fn matches_cpython_yield_nested_try_finally_one_cleanup() {
    let source = r"def f(value):
    try:
        yield value
        try:
            ping()
        except Exception:
            pass
    finally:
        try:
            cleanup()
        except Exception:
            pass
";
    common::assert_matches_cpython_marshal(
        source,
        "shared_try_exit_yield_nested_try_finally_one_cleanup.py",
    );
}
