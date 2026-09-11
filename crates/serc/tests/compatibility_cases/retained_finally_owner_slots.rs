use crate::common;

#[test]
fn matches_cpython_retained_break_pop_finally_marker() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if not task.done():
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "retained_break_finally.py",
    );
}

#[test]
fn matches_cpython_retained_break_short_finally_marker() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if task:
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "retained_break_short_finally.py",
    );
}

#[test]
fn matches_cpython_shared_break_finally_control() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error, other_ready):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            if other_ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if not task.done():
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "shared_break_finally.py",
    );
}

#[test]
fn matches_cpython_compacted_global_finally_control() {
    common::assert_matches_cpython_marshal(
        r#"async def f(task, clock, ready, sleep, suppress, Error):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if not done(task):
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
"#,
        "compacted_global_finally.py",
    );
}
