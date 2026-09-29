async def f(chunks):
    inner = [[inner for inner in chunk] async for chunk in chunks]
    return inner
