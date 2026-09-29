async def f(manager, read, flag):
    async with manager:
        try:
            value = await read()
            return value if flag else None
        except ValueError:
            raise
