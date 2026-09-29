async def f(manager, work, close):
    async with manager:
        try:
            await work()
            pass
        finally:
            await close()
