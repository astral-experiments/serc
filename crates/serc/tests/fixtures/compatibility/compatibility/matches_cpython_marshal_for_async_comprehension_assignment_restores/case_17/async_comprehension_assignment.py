async def f(chunks):
    a = [a async for a, b in chunks]
    return a
