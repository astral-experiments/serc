async def f(items):
    return ((await transform(value) for value in expand(item)) for item in items)
