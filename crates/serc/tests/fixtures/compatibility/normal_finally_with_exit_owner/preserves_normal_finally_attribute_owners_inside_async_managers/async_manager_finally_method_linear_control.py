async def f(manager, task, consume):
    async with manager as stream:
        try:
            await consume(stream)
        finally:
            if not task.done():
                task.cancel()
                await task
