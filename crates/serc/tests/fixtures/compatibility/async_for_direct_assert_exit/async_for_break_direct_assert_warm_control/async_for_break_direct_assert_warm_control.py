async def f(attempts, connect, report, finish):
    value = None
    try:
        async for attempt in attempts:
            with attempt:
                value = await connect()
            if value is not None:
                break
        assert value is not None
    except Exception:
        report()
        raise
    await finish(value)
