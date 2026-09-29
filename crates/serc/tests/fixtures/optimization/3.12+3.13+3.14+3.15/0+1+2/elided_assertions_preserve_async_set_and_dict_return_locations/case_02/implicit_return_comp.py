async def f(xs):
    result = {item: item async for item in xs}
    assert result
