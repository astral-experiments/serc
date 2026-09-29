async def f(manager, pending):
    async with manager:
        try:
            yield
        finally:
            if pending is not None:
                await pending.close()
