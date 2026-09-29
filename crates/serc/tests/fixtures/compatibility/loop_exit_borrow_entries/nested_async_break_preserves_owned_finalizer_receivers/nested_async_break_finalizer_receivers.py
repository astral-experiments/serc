async def f(client, terminal, worker, events, value, choose, emit):
    try:
        async with client:
            with terminal:
                worker.start()
                try:
                    async for event in events:
                        terminal.update(event)
                        if event.changed:
                            status = event.status
                            if choose(status if isinstance(status, str) else None):
                                break
                finally:
                    worker.cancel()
                    await worker.wait()
            emit(value.first)
            emit(value.second)
    finally:
        await worker.stop()
    emit(value.third)
