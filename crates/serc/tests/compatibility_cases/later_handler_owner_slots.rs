use crate::common;

#[test]
fn later_handler_global_call() {
    common::assert_matches_cpython_marshal(
        r"async def probe(work):
    try:
        return await work()
    except FatalError:
        return None
    except retryable_errors():
        raise
",
        "later_handler_global_call.py",
    );
}

#[test]
fn later_handler_local_tuple() {
    common::assert_matches_cpython_marshal(
        r#"async def probe(work, fatal, retryable, server):
    for attempt in range(3):
        try:
            return await work()
        except fatal:
            raise
        except (retryable, server):
            if attempt == 2:
                raise
            await pause(attempt)
    raise AssertionError("retry did not return")
"#,
        "later_handler_local_tuple.py",
    );
}

#[test]
fn later_handler_plain_name_control() {
    common::assert_matches_cpython_marshal(
        r"async def probe(work, fatal, retryable):
    try:
        return await work()
    except fatal:
        raise
    except retryable:
        raise
",
        "later_handler_plain_name_control.py",
    );
}

#[test]
fn later_handler_wide_local_tuple_control() {
    common::assert_matches_cpython_marshal(
        r#"async def probe(a0, a1, a2, a3, a4, a5, a6, a7,
                a8, a9, a10, a11, a12, a13, a14, a15,
                work, fatal, retryable, server):
    for attempt in range(3):
        try:
            return await work()
        except fatal:
            raise
        except (retryable, server):
            if attempt == 2:
                raise
            await pause(attempt)
    raise AssertionError("retry did not return")
"#,
        "later_handler_wide_local_tuple_control.py",
    );
}

#[test]
fn later_handler_uninitialized_local_control() {
    // The call may raise before local_error is stored. Its checked load must
    // prevent the low-index pair from creating a reused dispatch slot.
    common::assert_matches_cpython_marshal(
        r"def probe(work, error):
    try:
        local_error = work()
    except KeyError:
        pass
    except (local_error, error):
        pass
",
        "later_handler_uninitialized_local_control.py",
    );
}
