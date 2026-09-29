async def f(manager, pending, task):
    async with manager:
        try:
            await task()
        except ValueError:
            raise
        finally:
            if pending is not None:
                await pending.close()
