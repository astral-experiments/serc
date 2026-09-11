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
