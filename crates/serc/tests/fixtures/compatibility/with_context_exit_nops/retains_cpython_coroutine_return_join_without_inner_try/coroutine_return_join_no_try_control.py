async def f(manager, read, flag):
    async with manager:
        value = await read()
        return value if flag else None
