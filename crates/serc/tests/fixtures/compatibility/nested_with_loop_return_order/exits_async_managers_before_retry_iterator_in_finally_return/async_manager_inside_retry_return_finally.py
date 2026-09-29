async def f(items, manager, value, cleanup):
    try:
        for item in items:
            async with manager:
                return value
    finally:
        cleanup()
