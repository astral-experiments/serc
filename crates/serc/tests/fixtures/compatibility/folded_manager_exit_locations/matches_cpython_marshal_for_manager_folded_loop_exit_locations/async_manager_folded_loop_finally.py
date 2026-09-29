async def f(manager, fetch, pause, cleanup):
    value = None
    try:
        async with manager:
            while True:
                try:
                    value = await fetch()
                    return value
                except LookupError:
                    await pause()
    except TimeoutError as exc:
        raise ValueError("timeout") from exc
    finally:
        if value is None:
            await cleanup()
