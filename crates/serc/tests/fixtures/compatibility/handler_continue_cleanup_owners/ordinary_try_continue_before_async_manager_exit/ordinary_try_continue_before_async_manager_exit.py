async def f(manager, flag, skip, observe):
    while flag():
        async with manager():
            try:
                await observe()
                if skip():
                    continue
            except ValueError:
                await observe()
    return None
