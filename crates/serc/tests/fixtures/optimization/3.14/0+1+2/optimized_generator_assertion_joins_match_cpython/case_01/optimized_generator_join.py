async def f(x, y):
    if x:
        assert x
    else:
        assert y
    await work()
