async def f(manager, value):
    async with manager:
        try:
            return value
        except Error:
            return None
