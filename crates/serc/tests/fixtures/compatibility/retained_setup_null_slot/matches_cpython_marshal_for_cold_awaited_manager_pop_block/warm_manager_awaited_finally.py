async def f(manager, work, close):
    with manager:
        try:
            work()
        except ValueError:
            raise
        finally:
            await close()
