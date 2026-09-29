async def f(manager, flag, left, right):
    async with manager:
        try:
            while True:
                try:
                    return left if flag else right
                except Inner:
                    continue
        except Outer:
            raise
