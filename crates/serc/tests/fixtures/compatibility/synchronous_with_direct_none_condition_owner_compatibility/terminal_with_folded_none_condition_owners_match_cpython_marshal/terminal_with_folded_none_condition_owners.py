async def f(manager, work, write):
    with manager:
        value = await work()
        if value is not None:
            return value
    return await write()

async def g(manager, work, write):
    with manager:
        value = await work()
        if value:
            return value
    return await write()

def h(manager, work, write):
    with manager:
        value = work()
        if value is not None:
            return value
    return write()
