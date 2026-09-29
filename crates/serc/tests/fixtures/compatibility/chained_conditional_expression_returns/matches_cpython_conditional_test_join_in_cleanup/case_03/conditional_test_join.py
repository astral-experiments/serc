async def f(value, flag, other, items, manager):
    if (check(value) if flag else False):
        return None
    return await work(value)
