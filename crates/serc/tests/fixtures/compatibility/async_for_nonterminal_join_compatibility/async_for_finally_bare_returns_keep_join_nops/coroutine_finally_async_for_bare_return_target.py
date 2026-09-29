async def target(enabled, items):
    if enabled:
        try:
            await work()
        finally:
            cleanup = items
            async for item in cleanup:
                consume(item)
        return
    await finish()
