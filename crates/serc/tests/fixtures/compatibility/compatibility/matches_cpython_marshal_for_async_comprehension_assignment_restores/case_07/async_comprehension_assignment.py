async def f(chunks):
    a = b = [chunk async for chunk in chunks]
    return a, b
