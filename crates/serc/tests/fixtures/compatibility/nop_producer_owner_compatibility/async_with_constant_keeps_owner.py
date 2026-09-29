async def f(manager):
    async with manager:
        work()
        'comment'
