async def f(items, value, manager):
    try:
        async with manager:
            for item in items:
                try:
                    return value
                except ValueError:
                    recover()
    finally:
        cleanup()
