async def f(items, outer, inner, work, cleanup):
    try:
        for item in items:
            async with outer:
                async with inner:
                    return await work()
    finally:
        cleanup()
