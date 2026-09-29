async def f(manager):
    async with manager as value:
        global marker
    return value
