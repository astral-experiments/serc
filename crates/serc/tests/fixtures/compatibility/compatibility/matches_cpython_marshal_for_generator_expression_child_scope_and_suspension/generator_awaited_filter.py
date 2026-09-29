async def f(items):
    return (item for item in items if await accept(item))
