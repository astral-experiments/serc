use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_try_assert_else_exit() {
    let source = r"async def body_assert_else(value, prepare, cleanup, finish):
    try:
        await prepare()
        assert value.ready
        assert value.last_action is None
    except AssertionError, TimeoutError:
        await cleanup()
        raise
    else:
        await finish()
        assert not value.ready
        assert value.last_action is None
    return value

async def body_name_assert_else(value, prepare, cleanup, finish):
    try:
        await prepare()
        assert value
    except AssertionError, TimeoutError:
        await cleanup()
        raise
    else:
        await finish()
    return value

def synchronous_control(value, prepare, cleanup, finish):
    try:
        prepare()
        assert value.last_action is None
    except AssertionError, TimeoutError:
        cleanup()
        raise
    else:
        finish()
    return value
";
    common::assert_matches_cpython_marshal(source, "coroutine_try_assert_else_exit.py");
}

#[test]
fn matches_cpython_named_handler_assertion_original_exit() {
    for (filename, source) in [
        (
            "coroutine_named_handler_assertion_same_array_exit.py",
            r"async def probe(operation, ready):
    try:
        await operation()
    except ValueError as original:
        try:
            code = await operation()
            assert code == 0
        except TypeError as recovery:
            print(recovery)
        if ready(original):
            raise RuntimeError() from original
        raise
",
        ),
        (
            "coroutine_named_handler_assertion_else_boundary_control.py",
            r"async def probe(operation, ready):
    try:
        await operation()
    except ValueError as original:
        try:
            code = await operation()
            assert code == 0
        except TypeError as recovery:
            print(recovery)
        else:
            print(code)
        if ready(original):
            raise RuntimeError() from original
        raise
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
