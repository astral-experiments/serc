async def f(manager, ready, test, work, after, cleanup):
    async with manager:
        try:
            if ready():
                while test():
                    await work()
            after()
        finally:
            cleanup()
