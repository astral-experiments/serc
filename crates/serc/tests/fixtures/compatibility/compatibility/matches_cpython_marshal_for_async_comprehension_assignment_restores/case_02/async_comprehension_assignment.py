async def f(chunks):
    value = [chunk async for chunk in chunks]
    return value
