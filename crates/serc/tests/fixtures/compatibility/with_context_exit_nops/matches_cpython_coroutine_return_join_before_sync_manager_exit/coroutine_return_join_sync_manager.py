async def f(manager, read, flag):
    with manager:
        try:
            value = await read()
            return value if flag else None
        except ValueError:
            raise
