async def worker(queue, clock, record):
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
