async def f(xs):
    assert xs
    result = [item async for item in xs]
