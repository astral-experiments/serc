use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_guarded_break_finally_target() {
    let source = r#"async def worker(queue, clock, record):
    while True:
        started = None
        try:
            item = await queue.get()
            if item is None:
                record("stopping")
                break
            started = clock()
            consume(item)
            record("completed", item=item)
        except Exception as exc:
            record("failed", error=exc)
        finally:
            if started is not None:
                record("duration", clock() - started)
            queue.task_done()
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_guarded_break_finally_target.py");
}
