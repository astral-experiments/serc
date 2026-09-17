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
