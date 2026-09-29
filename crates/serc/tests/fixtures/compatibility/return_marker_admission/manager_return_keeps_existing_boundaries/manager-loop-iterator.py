async def f(manager, flag, left, right):
    async with manager:
        for item in left:
            try:
                await prepare()
                return left if flag else right
            except Error:
                continue
