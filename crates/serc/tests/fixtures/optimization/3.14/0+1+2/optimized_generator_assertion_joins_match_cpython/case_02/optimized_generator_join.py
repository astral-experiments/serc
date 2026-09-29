async def f(x, y, manager):
    async with manager:
        if x:
            assert x
        else:
            assert y
        await work()
