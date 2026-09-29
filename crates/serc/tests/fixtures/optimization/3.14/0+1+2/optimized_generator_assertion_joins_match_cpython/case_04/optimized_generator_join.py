async def f(x, y, manager, other):
    async with manager:
        with other:
            if x:
                assert x
            else:
                assert y
            await work()
    yield 1
