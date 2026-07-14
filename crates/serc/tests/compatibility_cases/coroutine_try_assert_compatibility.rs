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
