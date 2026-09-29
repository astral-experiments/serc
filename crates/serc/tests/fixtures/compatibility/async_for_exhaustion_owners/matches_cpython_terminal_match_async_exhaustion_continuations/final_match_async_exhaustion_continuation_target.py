async def worker(queue, source, sink):
    while True:
        kind = await queue.get()
        match kind:
            case 0:
                sink.ready()
            case 1:
                async for item in source:
                    await sink.send(item)
            case 2:
                async for item in source:
                    await sink.send(item)
        await sink.send(None)
        queue.task_done()
