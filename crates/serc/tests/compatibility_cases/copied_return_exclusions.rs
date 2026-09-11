use crate::common;

#[test]
fn et3_nested_async_return() {
    common::assert_matches_cpython_marshal(
        r"async def f(handle, enabled):
    try:
        try:
            description = await handle.describe()
            if description.status == enabled:
                await handle.terminate()
        except Exception:
            try:
                await handle.terminate()
            except Exception as err:
                report(err)
    except Exception as err:
        report(err)
",
        "et3_nested_async.py",
    );
}

#[test]
fn et3_nested_sync_return() {
    common::assert_matches_cpython_marshal(
        r"def f(handle, enabled):
    try:
        try:
            description = handle.describe()
            if description.status == enabled:
                handle.terminate()
        except Exception:
            try:
                handle.terminate()
            except Exception as err:
                report(err)
    except Exception as err:
        report(err)
",
        "et3_nested_sync.py",
    );
}

#[test]
fn et3_conditional_async_finally_return() {
    common::assert_matches_cpython_marshal(
        r"async def f(state):
    try:
        try:
            await action()
        finally:
            if state is not None:
                state.clear()
    except Exception as err:
        report(err)
",
        "et3_async_finally.py",
    );
}

#[test]
fn et3_conditional_sync_finally_return() {
    common::assert_matches_cpython_marshal(
        r"def f(state):
    try:
        try:
            action()
        finally:
            if state is not None:
                state.clear()
    except Exception as err:
        report(err)
",
        "et3_sync_finally.py",
    );
}

#[test]
fn et3_multiple_conditional_async_returns() {
    common::assert_matches_cpython_marshal(
        r"async def f(state, ready):
    try:
        try:
            await action()
        finally:
            if state is not None and ready and state.active:
                state.clear()
    except Exception as err:
        report(err)
",
        "et3_multiple_async_finally.py",
    );
}

#[test]
fn et3_multiple_conditional_sync_returns() {
    common::assert_matches_cpython_marshal(
        r"def f(state, ready):
    try:
        try:
            action()
        finally:
            if state is not None and ready and state.active:
                state.clear()
    except Exception as err:
        report(err)
",
        "et3_multiple_sync_finally.py",
    );
}
