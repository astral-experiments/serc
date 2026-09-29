async def f(enabled, items, value, recover, after):
    if enabled:
        value = None
    else:
        try:
            async for item in items:
                value = item
                if value:
                    break
            assert value is not None
        except ValueError:
            recover()
    after(value)
