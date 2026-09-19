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

#[test]
fn matches_cpython_terminal_try_nested_cleanup_nop_owners() {
    for (filename, source) in [
        (
            "coroutine_terminal_try_nested_cleanup_nop_owner_target.py",
            r#"async def run(client, value):
    try:
        await client.prepare()
        if not isinstance(value, str) or not value:
            raise ValueError(value)
    except Exception as error:
        raise RuntimeError("failed") from error
    finally:
        try:
            if value is not None:
                await client.remove(value)
        finally:
            await client.close()
"#,
        ),
        (
            "async_generator_terminal_try_nested_cleanup_nop_owner_target.py",
            r#"async def run(client, code):
    try:
        await client.prepare()
        if code != 0:
            client.warn(code)
            yield "failed"
            return
    except Exception as error:
        yield str(error)
        return
    finally:
        try:
            await client.close()
        except Exception:
            client.warn("cleanup")
    yield "finished"
"#,
        ),
        (
            "coroutine_terminal_try_linear_cleanup_nop_owner_control.py",
            r#"async def run(client, value):
    try:
        await client.prepare()
        if not isinstance(value, str) or not value:
            raise ValueError(value)
    except Exception as error:
        raise RuntimeError("failed") from error
    finally:
        await client.close()
"#,
        ),
        (
            "async_generator_terminal_try_linear_cleanup_nop_owner_control.py",
            r#"async def run(client, code):
    try:
        await client.prepare()
        if code != 0:
            client.warn(code)
            yield "failed"
            return
    except Exception as error:
        yield str(error)
        return
    finally:
        await client.close()
    yield "finished"
"#,
        ),
        (
            "coroutine_terminal_try_shared_false_nop_owner_control.py",
            r#"async def run(client, value):
    try:
        await client.prepare()
        if isinstance(value, str) and value:
            raise ValueError(value)
    except Exception as error:
        raise RuntimeError("failed") from error
    finally:
        try:
            if value is not None:
                await client.remove(value)
        finally:
            await client.close()
"#,
        ),
        (
            "async_generator_terminal_try_shared_false_nop_owner_control.py",
            r#"async def run(client, code):
    try:
        await client.prepare()
        if code != 0 and client.ready:
            client.warn(code)
            yield "failed"
            return
    except Exception as error:
        yield str(error)
        return
    finally:
        try:
            await client.close()
        except Exception:
            client.warn("cleanup")
    yield "finished"
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
