async def f(events, worker, ready, choose):
    try:
        async for event in events:
            if ready:
                if event.changed:
                    worker.update(event)
                if choose(event):
                    break
    finally:
        worker.cancel()
        await worker.wait()
    emit(worker)
