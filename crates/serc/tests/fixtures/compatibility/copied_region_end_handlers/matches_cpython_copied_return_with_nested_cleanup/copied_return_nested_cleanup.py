async def f(manager, values):
    try:
        async with manager:
            async for item in values:
                yield item
    except Error:
        return
    finally:
        try:
            cleanup()
        except Error:
            pass
