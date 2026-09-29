async def f(manager):
    async with manager:
        try:
            1
        finally:
            pass
