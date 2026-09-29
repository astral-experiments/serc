async def f(manager, pending, task):
    async with manager:
        try:
            try:
                await task()
            except ValueError:
                report()
        finally:
            if pending is not None:
                pending.close()
