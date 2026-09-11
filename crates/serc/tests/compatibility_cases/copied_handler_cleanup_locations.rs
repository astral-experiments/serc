use crate::common;

#[test]
fn matches_cpython_sync_return_finally_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(payload, cleanup, record):
    try:
        return payload
    finally:
        try:
            cleanup()
        except Exception as exc:
            if payload is not None:
                record(payload, exc)
",
        "sync_return_finally_handler.py",
    );
}

#[test]
fn matches_cpython_async_return_finally_handler() {
    common::assert_matches_cpython_marshal(
        r"async def f(payload, cleanup, record):
    try:
        return payload
    finally:
        try:
            await cleanup()
        except Exception as exc:
            if payload is not None:
                record(payload, exc)
",
        "async_return_finally_handler.py",
    );
}

#[test]
fn matches_cpython_loop_handler_with_continuation() {
    common::assert_matches_cpython_marshal(
        r"def f(items, report):
    for value in items:
        try:
            risky(value)
        except Exception as exc:
            if value is not None:
                report(exc)
        checkpoint()
",
        "loop_handler_with_continuation.py",
    );
}

#[test]
fn matches_cpython_named_handler_return() {
    common::assert_matches_cpython_marshal(
        r"def f(value, action):
    try:
        action()
    except Exception as exc:
        if value:
            consume(exc)
    return value
",
        "named_handler_return.py",
    );
}

#[test]
fn matches_cpython_bare_handler_return() {
    common::assert_matches_cpython_marshal(
        r"def f(value, action):
    try:
        action()
    except Exception:
        if value:
            consume(value)
    return value
",
        "bare_handler_return.py",
    );
}
