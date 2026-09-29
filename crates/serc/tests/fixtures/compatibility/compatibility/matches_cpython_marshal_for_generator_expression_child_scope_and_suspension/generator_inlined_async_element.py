async def f(items):
    return ([value async for value in expand(item)] for item in items)
