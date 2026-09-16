use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_break_nop_target() {
    let source = r"async def start(context):
    async def worker():
        while True:
            item = await context.queue.get()
            if item is None:
                context.queue.task_done()
                break
            backoff = 1
            retry_count = 0
            while True:
                token = None
                try:
                    token = pin(item)
                    async with session():
                        await process(item)
                    record(item)
                    break
                except (RetryError, StopError) as error:
                    report(error)
                    await context.results.put(error)
                    break
                except Exception as error:
                    report(error)
                    await sleep(backoff)
                    backoff = min(backoff * 2, 10)
                    retry_count += 1
                finally:
                    if token is not None:
                        reset(token)
            context.queue.task_done()
    context.workers = [create_task(worker()) for _ in range(context.parallelism)]
";
    common::assert_matches_cpython_marshal(source, "conditional_finally_break_nop_target.py");
}

#[test]
fn matches_cpython_marshal_for_copied_finally_break_nop_populated_control() {
    let source = r"async def start(context):
    async def worker():
        while True:
            item = await context.queue.get()
            if item is None:
                context.queue.task_done()
                break
            backoff = 1
            retry_count = 0
            while True:
                token = None
                try:
                    token = pin(item)
                    async with session():
                        await process(item)
                    record(item)
                    break
                except (RetryError, StopError) as error:
                    report(error)
                    await context.results.put(error)
                    break
                except Exception as error:
                    report(error)
                    await sleep(backoff)
                    backoff = min(backoff * 2, 10)
                    retry_count += 1
                finally:
                    if token is not None:
                        reset(token)
                    cleanup_finished()
            context.queue.task_done()
    context.workers = [create_task(worker()) for _ in range(context.parallelism)]
";
    common::assert_matches_cpython_marshal(source, "populated_finally_break_join_control.py");
}
