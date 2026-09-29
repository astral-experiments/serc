async def f(manager, poll):
    async with manager:
        try:
            while True:
                value = await poll()
                if value is not None:
                    break
        except OSError:
            return None
    return value
