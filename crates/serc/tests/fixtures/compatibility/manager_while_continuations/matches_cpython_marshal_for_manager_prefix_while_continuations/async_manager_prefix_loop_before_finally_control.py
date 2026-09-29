async def f(manager, ready, work, cleanup):
    async with manager:
        while ready():
            await work()
        try:
            await work()
        finally:
            await cleanup()
