async def f(manager, work, flag, cleanup):
    while True:
        try:
            async with manager:
                yield await work()
                break
        except Exception:
            await work()
        finally:
            if flag:
                cleanup()
