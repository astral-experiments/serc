async def f(x, y, manager):
    async with manager:
        if x or y:
            assert x
        else:
            assert y
        work()
