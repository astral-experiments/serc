async def f(manager, enabled, stop, work, recover):
    async with manager:
        try:
            if enabled:
                while True:
                    if stop():
                        break
                    await work()
        except Exception:
            recover()
        await work()
        return enabled
