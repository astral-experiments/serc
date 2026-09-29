async def f():
    return (item for item in await source())
