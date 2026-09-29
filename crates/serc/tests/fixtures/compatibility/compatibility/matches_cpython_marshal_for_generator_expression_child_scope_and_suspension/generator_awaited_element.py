async def f(items):
    return (await transform(item) for item in items)
