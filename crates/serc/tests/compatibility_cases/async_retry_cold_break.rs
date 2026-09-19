use crate::common;

#[test]
fn async_retry_cold_break_target() {
    let source = r"async def poll(worker, clock, pause, report, writer):
    attempt = 0
    started = clock()
    while True:
        attempt += 1
        try:
            responses = await worker()
            break
        except LookupError as error:
            report(error, level=1)
            await pause()
        except Exception as error:
            report(error, level=0)
            await pause()
        if attempt >= 3 and clock() - started > 5:
            responses = [b'unavailable']
            break
    report(responses, level=1)
    writer.write(responses)
    await writer.drain()
";
    common::assert_matches_cpython_marshal(source, "async_retry_cold_break_target.py");
}

#[test]
fn async_retry_cold_break_warm_break_control() {
    let source = r"async def poll(worker, clock, pause, report, writer):
    attempt = 0
    started = clock()
    while True:
        attempt += 1
        try:
            responses = await worker()
            if responses:
                break
        except LookupError as error:
            report(error, level=1)
            await pause()
        except Exception as error:
            report(error, level=0)
            await pause()
        if attempt >= 3 and clock() - started > 5:
            responses = [b'unavailable']
            break
    report(responses, level=1)
    writer.write(responses)
    await writer.drain()
";
    common::assert_matches_cpython_marshal(source, "async_retry_cold_break_warm_break_control.py");
}

#[test]
fn runtime_while_cold_breaks_keep_original_exit_anchors() {
    for (filename, source) in [
        (
            "runtime_while_cold_break_empty_anchor.py",
            r"def f(running, poll, stopped, finish, Retry):
    while running():
        try:
            poll()
            break
        except Retry:
            pass
        if stopped():
            break
    result = finish()
    return result
",
        ),
        (
            "async_generator_runtime_while_cold_break_empty_anchor.py",
            r"async def f(running, poll, stopped, finish, Retry):
    while running():
        try:
            yield await poll()
            break
        except Retry:
            pass
        if stopped():
            break
    result = finish()
    yield result
",
        ),
        (
            "runtime_while_cold_break_populated_else_control.py",
            r"def f(running, poll, stopped, finish, Retry):
    while running():
        try:
            poll()
            break
        except Retry:
            pass
        if stopped():
            break
    else:
        finish()
    result = finish()
    return result
",
        ),
        (
            "runtime_while_cold_break_finally_cleanup_control.py",
            r"def f(running, poll, stopped, cleanup, finish, Retry):
    while running():
        try:
            poll()
            break
        except Retry:
            pass
        if stopped():
            try:
                break
            finally:
                cleanup()
    result = finish()
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
