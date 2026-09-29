async def f(manager, handle, attempts):
    try:
        for attempt in range(attempts):
            try:
                await work()
            except Exception:
                if should_stop(attempt):
                    record("abandoned", attempt, handle)
                    return
                if await cancelled():
                    return
                continue
            record("success", attempt, handle)
            return
    finally:
        async with manager():
            await release(handle)
