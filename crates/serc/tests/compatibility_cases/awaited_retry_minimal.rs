use crate::common;

#[test]
fn matches_cpython_marshal_for_awaited_retry_minimal_typed() {
    let source = r"async def retry(work, enabled, sink):
    success = False
    try:
        await work()
        success = True
    except OSError:
        if enabled:
            for attempt in range(3):
                try:
                    await work()
                    return
                except OSError:
                    pass
    finally:
        sink(success)
";
    common::assert_matches_cpython_marshal(source, "awaited_retry_minimal_typed.py");
}

#[test]
fn matches_cpython_marshal_for_awaited_retry_minimal_named() {
    let source = r"async def retry(work, enabled, sink):
    success = False
    try:
        await work()
        success = True
    except OSError as outer_error:
        if enabled:
            for attempt in range(3):
                try:
                    await work()
                    return
                except OSError as inner_error:
                    pass
    finally:
        sink(success)
";
    common::assert_matches_cpython_marshal(source, "awaited_retry_minimal_named.py");
}

#[test]
fn matches_cpython_marshal_for_awaited_retry_minimal_bare() {
    let source = r"async def retry(work, enabled, sink):
    success = False
    try:
        await work()
        success = True
    except OSError:
        if enabled:
            for attempt in range(3):
                try:
                    await work()
                    return
                except:
                    pass
    finally:
        sink(success)
";
    common::assert_matches_cpython_marshal(source, "awaited_retry_minimal_bare.py");
}
