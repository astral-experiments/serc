async def f(items, manager, recover):
    last = None
    for item in items:
        try:
            async with manager:
                return item
        except Exception as error:
            last = error
            await recover()
    raise RuntimeError(f'{items}: {last}')
