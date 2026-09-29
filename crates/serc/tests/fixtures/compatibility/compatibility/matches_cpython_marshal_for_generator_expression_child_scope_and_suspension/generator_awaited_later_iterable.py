async def f(items):
    return (value for item in items for value in await expand(item))
