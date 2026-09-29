async def f(manager, task, consume):
    async with manager as stream:
        try:
            async for item in stream:
                if not item:
                    break
                await consume(item)
        finally:
            if not task.done():
                task.cancel()
                await task
