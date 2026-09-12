use crate::common;

#[test]
fn async_for_inherited_assert_cold_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(enabled, items, value, recover, after):
    if enabled:
        value = None
    else:
        try:
            async for item in items:
                value = item
            assert value is not None
        except ValueError:
            recover()
    after(value)
",
        "async_for_inherited_assert_cold_control.py",
    );
}

#[test]
fn sync_for_inherited_assert_positive() {
    common::assert_matches_cpython_marshal(
        r"def f(enabled, items, value, recover, after):
    if enabled:
        value = None
    else:
        try:
            for item in items:
                value = item
            assert value is not None
        except ValueError:
            recover()
    after(value)
",
        "sync_for_inherited_assert_positive.py",
    );
}

#[test]
fn await_inherited_assert_positive() {
    common::assert_matches_cpython_marshal(
        r"async def f(enabled, work, value, recover, after):
    if enabled:
        value = None
    else:
        try:
            value = await work()
            assert value is not None
        except ValueError:
            recover()
    after(value)
",
        "await_inherited_assert_positive.py",
    );
}

#[test]
fn async_for_break_inherited_assert_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(enabled, items, value, recover, after):
    if enabled:
        value = None
    else:
        try:
            async for item in items:
                value = item
                if value:
                    break
            assert value is not None
        except ValueError:
            recover()
    after(value)
",
        "async_for_break_inherited_assert_control.py",
    );
}

#[test]
fn while_break_retry_large_successor() {
    common::assert_matches_cpython_marshal(
        r"def f(ready, other, step, finish, result):
    try:
        while True:
            step()
            if ready() or other():
                break
    except RuntimeError:
        try:
            while True:
                step()
                if ready() or other():
                    break
        except ValueError:
            step()
    finish(result)
    return result
",
        "while_break_retry_large_successor.py",
    );
}

#[test]
fn while_break_named_retry_large_successor() {
    common::assert_matches_cpython_marshal(
        r"def f(ready, other, step, finish, result):
    try:
        while True:
            step()
            if ready() or other():
                break
    except RuntimeError as error:
        try:
            while True:
                step()
                if ready() or other():
                    break
        except ValueError as retry_error:
            step()
    finish(result)
    return result
",
        "while_break_named_retry_large_successor.py",
    );
}
