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

#[test]
fn matches_cpython_handler_cleanup_after_loop_control_arm() {
    common::assert_matches_cpython_marshal(
        r"def probe(items, parse, recover, choose, finish):
    for item in items:
        try:
            value = parse(item)
            break
        except KeyError:
            if not choose(item):
                break
            else:
                recover(item)
                finish(item)
    return None
",
        "handler_break_before_normal_arm_control.py",
    );
    common::assert_matches_cpython_marshal(
        r"def probe(items, parse, recover, keep, finish):
    for item in items:
        try:
            value = parse(item)
        except ValueError:
            if not keep(item):
                continue
            else:
                value = recover(item)
        finish(value)
    return None
",
        "handler_continue_before_normal_arm_control.py",
    );
    common::assert_matches_cpython_marshal(
        r"def probe(items, parse, recover, choose, finish):
    for item in items:
        try:
            value = parse(item)
            break
        except KeyError:
            if choose(item):
                recover(item)
                finish(item)
            else:
                break
    return None
",
        "handler_normal_arm_before_break.py",
    );
    common::assert_matches_cpython_marshal(
        r"def probe(items, parse, recover, keep, finish):
    for item in items:
        try:
            value = parse(item)
        except ValueError:
            if keep(item):
                value = recover(item)
            else:
                continue
        finish(value)
    return None
",
        "handler_normal_arm_before_continue.py",
    );
}
