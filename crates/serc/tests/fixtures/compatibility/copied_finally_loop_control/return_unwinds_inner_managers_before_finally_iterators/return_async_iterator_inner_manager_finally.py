async def f(items, manager, value):
    try:
        async for item in items:
            with manager:
                return item, value
    finally:
        try:
            await finish()
        except Exception:
            report()
