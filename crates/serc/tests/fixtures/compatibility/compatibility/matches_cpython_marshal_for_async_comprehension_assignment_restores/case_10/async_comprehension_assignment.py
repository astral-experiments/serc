async def f(chunks):
    global value
    value = [chunk async for chunk in chunks]
    return value
