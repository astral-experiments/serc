async def f(manager):
    async with manager:
        work()
        pass
