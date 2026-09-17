use crate::common;

#[test]
fn nested_recovery_source_array_borrow_target() {
    let source = r"async def worker(queue, clock, record):
    while True:
        started = None
        try:
            item = await queue.get()
            if item is None:
                record('stopping')
                break
            started = clock()
            try:
                checkpoint = clock()
                await consume(item)
                record('checkpoint', clock() - checkpoint, item=item)
            except Exception as error:
                record('retry failed', error=error)
                record('retry count', item=item)
            record('completed', item=item)
        except Exception as error:
            record('failed', error=error)
        finally:
            if started is not None:
                record('duration', clock() - started)
            queue.task_done()
";
    common::assert_matches_cpython_marshal(source, "nested_recovery_source_array_borrow_target.py");
}

#[test]
fn nested_recovery_source_array_borrow_inline_control() {
    let source = r"async def worker(queue, clock, record):
    while True:
        started = None
        try:
            item = await queue.get()
            if item is None:
                record('stopping')
                break
            started = clock()
            checkpoint = clock()
            await consume(item)
            record('checkpoint', clock() - checkpoint, item=item)
            record('completed', item=item)
        except Exception as error:
            record('failed', error=error)
        finally:
            if started is not None:
                record('duration', clock() - started)
            queue.task_done()
";
    common::assert_matches_cpython_marshal(
        source,
        "nested_recovery_source_array_borrow_inline_control.py",
    );
}
