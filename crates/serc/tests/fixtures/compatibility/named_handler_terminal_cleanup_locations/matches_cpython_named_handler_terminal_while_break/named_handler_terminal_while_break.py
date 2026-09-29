async def consume(queue, process, fail):
    pending = []
    closed = False
    while pending or not closed:
        try:
            await process()
        except Exception as error:
            fail(error)
            pending.clear()
            while True:
                try:
                    queued = queue.get_nowait()
                except LookupError:
                    break
                if queued is None:
                    closed = True
                else:
                    fail(queued, error)
