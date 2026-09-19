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

#[test]
fn ordinary_try_inside_iterator_return_finally_keyword_truth() {
    for (filename, source) in [
        (
            "iterator_inner_try_name_return_five_globals.py",
            r#"async def target(client, attempts, model, operation):
    succeeded = False
    try:
        for attempt in attempts:
            try:
                response = await client.run(attempt)
                if response is None:
                    raise Error
                sanitized = build(
                    first=select(),
                    second=select(),
                    third=select(),
                    fourth=select(),
                    extra=operation,
                )
                succeeded = True
                return sanitized
            except Error as exc:
                client.record(exc)
    finally:
        emit(
            model=model,
            operation=operation,
            response_tag="success" if succeeded else "failure",
        )
"#,
        ),
        (
            "iterator_inner_try_name_return_four_globals_control.py",
            r#"async def target(client, attempts, model, operation):
    succeeded = False
    try:
        for attempt in attempts:
            try:
                response = await client.run(attempt)
                if response is None:
                    raise Error
                sanitized = build(
                    first=select(),
                    second=select(),
                    third=select(),
                    fourth=None,
                    extra=operation,
                )
                succeeded = True
                return sanitized
            except Error as exc:
                client.record(exc)
    finally:
        emit(
            model=model,
            operation=operation,
            response_tag="success" if succeeded else "failure",
        )
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
