async def f(chunks):
    chunk = [chunk async for chunk in chunks]
    return chunk
