def outer():
    value = None
    async def f(chunks):
        nonlocal value
        value = [chunk async for chunk in chunks]
        return value
    return f
