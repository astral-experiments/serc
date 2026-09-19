use crate::common;

#[test]
fn async_for_direct_assert_cold_exit() {
    common::assert_matches_cpython_marshal(
        r"async def f(attempts, connect, report, finish):
    value = None
    try:
        async for attempt in attempts:
            with attempt:
                value = await connect()
        assert value is not None
    except Exception:
        report()
        raise
    await finish(value)
",
        "async_for_direct_assert_cold_exit.py",
    );
}

#[test]
fn async_for_break_direct_assert_warm_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(attempts, connect, report, finish):
    value = None
    try:
        async for attempt in attempts:
            with attempt:
                value = await connect()
            if value is not None:
                break
        assert value is not None
    except Exception:
        report()
        raise
    await finish(value)
",
        "async_for_break_direct_assert_warm_control.py",
    );
}
