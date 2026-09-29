async def f():
    return [[x async for x in xs] for xs in groups]
