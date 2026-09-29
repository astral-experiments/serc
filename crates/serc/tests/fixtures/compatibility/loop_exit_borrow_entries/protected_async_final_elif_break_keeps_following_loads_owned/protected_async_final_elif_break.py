async def f(events, worker, ready, choose):
    try:
        async for event in events:
            if ready:
                worker.update(event)
            elif choose(event):
                break
    finally:
        worker.cancel()
        await worker.wait()
    emit(worker)
