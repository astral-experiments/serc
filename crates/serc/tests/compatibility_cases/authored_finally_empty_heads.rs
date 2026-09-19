use crate::common;

#[test]
fn matches_cpython_authored_finally_break_through_empty_head() {
    let source = r"async def stream_events(task, response):
    try:
        while True:
            events = task.snapshot()
            if events:
                for event in events:
                    await response.write(event)
                continue
            if task.closed:
                break
            try:
                await task.wait()
            except TimeoutError:
                await response.ping()
    except CancelledError:
        pass
    finally:
        try:
            await response.close()
        except ConnectionResetError:
            pass
    return response
";
    common::assert_matches_cpython_marshal(source, "authored_finally_empty_head.py");
}
