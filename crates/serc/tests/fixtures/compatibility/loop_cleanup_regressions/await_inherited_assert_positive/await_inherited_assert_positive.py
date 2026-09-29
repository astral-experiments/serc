async def f(enabled, work, value, recover, after):
    if enabled:
        value = None
    else:
        try:
            value = await work()
            assert value is not None
        except ValueError:
            recover()
    after(value)
