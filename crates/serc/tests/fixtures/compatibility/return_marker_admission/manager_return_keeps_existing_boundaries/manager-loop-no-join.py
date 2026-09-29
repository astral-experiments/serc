async def f(manager, flag, left, right):
    async with manager:
        while True:
            try:
                await prepare()
                return left
            except Error:
                continue
