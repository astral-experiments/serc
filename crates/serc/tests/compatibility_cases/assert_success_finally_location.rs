use crate::common;

#[test]
fn matches_cpython_marshal_for_async_and_success_finally_location() {
    common::assert_matches_cpython_marshal(
        r"async def f(left, right, close):
    try:
        assert left and right
    except Exception:
        raise
    finally:
        await close()
",
        "async_and_success_finally.py",
    );
}

#[test]
fn matches_cpython_marshal_for_sync_and_success_finally_location() {
    common::assert_matches_cpython_marshal(
        r"def f(left, right, close):
    try:
        assert left and right
    except Exception:
        raise
    finally:
        close()
",
        "sync_and_success_finally.py",
    );
}

#[test]
fn matches_cpython_marshal_for_async_named_message_success_finally_location() {
    common::assert_matches_cpython_marshal(
        r"async def f(value, allowed, close, recover):
    try:
        assert isinstance(value, str) and value in allowed, f'Unexpected: {value!r}'
    except Exception as exc:
        recover(exc)
        raise
    finally:
        await close()
",
        "async_named_message_success_finally.py",
    );
}

#[test]
fn matches_cpython_marshal_for_async_nested_managers_success_finally_location() {
    common::assert_matches_cpython_marshal(
        r"async def f(value, allowed, outer, inner, close):
    async with outer:
        async with inner:
            try:
                assert isinstance(value, str) and value in allowed, f'Unexpected: {value!r}'
            except Exception as exc:
                raise ValueError() from exc
            finally:
                await close()
",
        "async_nested_managers_success_finally.py",
    );
}

#[test]
fn matches_cpython_finally_assertion_success_exit_copies() {
    for (filename, source) in [
        (
            "finally_runtime_assertion_success_raise_target.py",
            r"def check(value, cleanup, wanted):
    try:
        if value:
            return value
    finally:
        item = cleanup()
        assert item == wanted
    raise AssertionError
",
        ),
        (
            "generator_finally_runtime_assertion_success_return_target.py",
            r"def read_items(stream, process):
    if stream:
        try:
            yield stream
        finally:
            stream.close()
            _, error = process.communicate()
            assert process.returncode == 0, error.decode()
        return
    yield None
",
        ),
        (
            "finally_folded_assertion_success_raise_control.py",
            r"def check(value, cleanup):
    try:
        if value:
            return value
    finally:
        cleanup()
        assert True
    raise AssertionError
",
        ),
        (
            "finally_assertion_followed_by_call_raise_control.py",
            r"def check(value, cleanup, wanted, finish):
    try:
        if value:
            return value
    finally:
        item = cleanup()
        assert item == wanted
        finish()
    raise AssertionError
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
