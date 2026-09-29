async def worker(queue, clock, record):
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
