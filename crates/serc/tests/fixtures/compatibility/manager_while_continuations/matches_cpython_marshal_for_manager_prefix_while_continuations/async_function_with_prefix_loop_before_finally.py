async def f(manager, ready, work, cleanup):
    with manager:
        while not ready():
            await work()
        try:
            await work()
        finally:
            await cleanup()
