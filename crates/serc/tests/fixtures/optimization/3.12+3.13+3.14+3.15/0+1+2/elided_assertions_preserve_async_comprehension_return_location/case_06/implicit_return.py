async def f(xs):
    result = await collect(xs)
    assert result
