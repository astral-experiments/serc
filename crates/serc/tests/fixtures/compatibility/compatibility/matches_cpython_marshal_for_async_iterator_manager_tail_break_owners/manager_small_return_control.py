async def sample(attempts, work, value):
    try:
        async for attempt in attempts:
            with attempt:
                await work(value)
                break
    except Exception:
        pass
    return value
