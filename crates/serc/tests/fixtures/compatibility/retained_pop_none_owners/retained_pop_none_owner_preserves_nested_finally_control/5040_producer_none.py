async def f(it, queue, count):
    err = None
    try:
        while True:
            try:
                item = await anext(it)
            except StopAsyncIteration:
                break
            await queue.put(item)
    except Exception as error:
        log(error)
        err = error
    finally:
        for _ in range(count):
            try:
                if err is not None:
                    queue.put_nowait(err)
                else:
                    queue.put_nowait(None)
            except QueueFull:
                break
        done()
