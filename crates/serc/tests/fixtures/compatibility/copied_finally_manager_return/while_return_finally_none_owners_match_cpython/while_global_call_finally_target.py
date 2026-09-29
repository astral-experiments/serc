async def f(source, sink, manager, cancel_task):
    sink.attached = True
    cursor = 0
    try:
        try:
            events, terminal, latest = await source.snapshot(cursor)
        except LookupError:
            sink.gap()
            await sink.send({"type": "error", "code": "gap"})
            return
        await sink.send({"type": "ready", "cursor": cursor})
        while True:
            for event in events:
                await sink.send(event.value)
                cursor = event.cursor
            if terminal and cursor >= latest:
                record(sink, "completed")
                return
            try:
                async with manager:
                    await source.wait(cursor)
            except TimeoutError:
                await sink.heartbeat()
            try:
                events, terminal, latest = await source.snapshot(cursor)
            except LookupError:
                sink.gap()
                await sink.send({"type": "error", "code": "gap"})
                return
    except ConnectionError:
        sink.disconnected()
        raise
    finally:
        if cancel_task is not None:
            cancel_task.cancel()
            await source.join(cancel_task, return_exceptions=True)
        sink.attached = False
