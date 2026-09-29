async def f(xs):
    result = {item async for item in xs}
    assert result
