use crate::common;

#[test]
fn matches_cpython_named_handler_conditional_test_cleanup_location() {
    common::assert_matches_cpython_marshal(
        r#"def query(flag, fallback):
    try:
        return request()
    except TimeoutError as exc:
        record("timeout")
        if flag if flag is not None else fallback:
            raise RuntimeError("request timed out") from exc
    except OSError as exc:
        record("error", str(exc))
        if flag if flag is not None else fallback:
            raise RuntimeError(str(exc)) from exc
    return None
"#,
        "named_handler_conditional_test_cleanup_location.py",
    );
}

#[test]
fn matches_cpython_named_handler_conditional_test_cleanup_location_single_edge_control() {
    common::assert_matches_cpython_marshal(
        r#"def query(flag, fallback):
    try:
        return request()
    except TimeoutError as exc:
        record("timeout")
        if flag:
            raise RuntimeError("request timed out") from exc
    except OSError as exc:
        record("error", str(exc))
        if flag:
            raise RuntimeError(str(exc)) from exc
    return None
"#,
        "named_handler_conditional_test_cleanup_location_single_edge_control.py",
    );
}

#[test]
fn matches_cpython_named_handler_finally_false_join_location() {
    common::assert_matches_cpython_marshal(
        r#"def publish(path, data, *, key):
    parent = storage.dirname(path)
    if not storage.exists(parent):
        try:
            storage.makedirs(parent)
        except Exception:
            if not storage.isdir(parent):
                raise
    temporary_path = f"{path}.tmp-{new_id()}"
    try:
        with storage.open(temporary_path, "wb", partial_writes_on_exc=False) as stream:
            stream.write(data)
        try:
            storage.copy(temporary_path, path, overwrite=False)
        except Exception as exc:
            if not storage.exists(path):
                raise
            existing = storage.read_bytes(path)
            if existing != data and not records_match(
                decode(existing, key=key).state,
                decode(data, key=key).state,
            ):
                raise ValueError(
                    f"conflicting records were produced for {path}"
                ) from exc
    finally:
        if storage.exists(temporary_path):
            storage.remove(temporary_path)
"#,
        "named_handler_finally_false_join_location.py",
    );
}

#[test]
fn matches_cpython_named_handler_finally_false_join_location_single_edge_control() {
    common::assert_matches_cpython_marshal(
        r#"def publish(path, data, *, key):
    parent = storage.dirname(path)
    if not storage.exists(parent):
        try:
            storage.makedirs(parent)
        except Exception:
            if not storage.isdir(parent):
                raise
    temporary_path = f"{path}.tmp-{new_id()}"
    try:
        with storage.open(temporary_path, "wb", partial_writes_on_exc=False) as stream:
            stream.write(data)
        try:
            storage.copy(temporary_path, path, overwrite=False)
        except Exception as exc:
            if not storage.exists(path):
                raise
            existing = storage.read_bytes(path)
            if existing != data:
                raise ValueError(
                    f"conflicting records were produced for {path}"
                ) from exc
    finally:
        if storage.exists(temporary_path):
            storage.remove(temporary_path)
"#,
        "named_handler_finally_false_join_location_single_edge_control.py",
    );
}

#[test]
fn coroutine_named_recovery_raising_guard_keeps_cleanup_unlocated() {
    for (filename, source) in [
        (
            "async_named_recovery_raising_guard_cleanup.py",
            r"async def handle(ready, attempt, recover, finish, close):
    try:
        try:
            result = await attempt()
        except Exception as error:
            result = error
            if ready:
                marker = result
                try:
                    status = await recover(marker)
                except Exception as recovery_error:
                    raise RuntimeError('recovery failed') from recovery_error
                if status != 0:
                    raise RuntimeError('bad status')
        finish(result)
    finally:
        await close()
",
        ),
        (
            "async_named_linear_raising_guard_cleanup_control.py",
            r"async def handle(ready, attempt, recover, finish, close):
    try:
        try:
            result = await attempt()
        except Exception as error:
            result = error
            if ready:
                marker = result
                status = await recover(marker)
                if status != 0:
                    raise RuntimeError('bad status')
        finish(result)
    finally:
        await close()
",
        ),
        (
            "async_named_recovery_return_guard_cleanup_control.py",
            r"async def handle(ready, attempt, recover, finish, close):
    try:
        try:
            result = await attempt()
        except Exception as error:
            result = error
            if ready:
                marker = result
                try:
                    status = await recover(marker)
                except Exception as recovery_error:
                    return recovery_error
                if status != 0:
                    raise RuntimeError('bad status')
        finish(result)
    finally:
        await close()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_shared_named_handler_normal_finally_cleanup() {
    for (filename, source) in [
        (
            "conditional_named_raise_shared_finally_nop_target.py",
            r"async def f(call, first, second, should_raise, cleanup):
    try:
        return await call()
    except RuntimeError as error:
        if first or (second and should_raise(error)):
            raise
    finally:
        cleanup()
",
        ),
        (
            "conditional_named_raise_unique_finally_nop_control.py",
            r"async def f(call, first, cleanup):
    try:
        return await call()
    except RuntimeError as error:
        if first:
            raise
    finally:
        cleanup()
",
        ),
        (
            "conditional_named_assert_shared_finally_nop_target.py",
            r"def f(work, first, second, enabled, cleanup):
    try:
        try:
            work()
        except RuntimeError as error:
            assert first(error)
            if enabled:
                assert second(error)
        else:
            raise AssertionError()
    finally:
        cleanup()
",
        ),
        (
            "conditional_named_assert_unique_finally_nop_control.py",
            r"def f(work, first, second, cleanup):
    try:
        try:
            work()
        except RuntimeError as error:
            assert first(error)
            assert second(error)
        else:
            raise AssertionError()
    finally:
        cleanup()
",
        ),
        (
            "conditional_named_raise_no_finally_control.py",
            r"async def f(call, first, second, should_raise):
    try:
        return await call()
    except RuntimeError as error:
        if first or (second and should_raise(error)):
            raise
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_coroutine_guarded_finally_borrow_cleanup() {
    for guard in ["flag", "flag is None", "flag is not None"] {
        for prefix in ["", "                replacement = error.value\n"] {
            let source = format!(
                "async def f(value, flag, other):\n    try:\n        value = await work()\n    finally:\n        prepared = await prepare(value)\n        try:\n            recover(prepared)\n        except Error as error:\n            if {guard}:\n{prefix}                if other and value is not None:\n                    consume(value)\n                else:\n                    value = error\n    return value\n"
            );
            common::assert_matches_cpython_marshal(&source, "coroutine_guarded_finally_cleanup.py");
        }
    }
}
