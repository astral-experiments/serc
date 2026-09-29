async def f(manager, flag, left, right):
    async with manager:
        while True:
            try:
                await prepare()
                return await left() if flag else right
            except Error:
                continue
