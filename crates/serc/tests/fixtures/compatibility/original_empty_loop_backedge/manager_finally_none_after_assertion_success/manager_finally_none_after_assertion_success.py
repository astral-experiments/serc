async def f(manager, pending, task):
    async with manager:
        try:
            result = await task()
            assert result is not None
        finally:
            if pending is not None:
                await pending.close()
