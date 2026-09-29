async def try_around_async_with(manager, items):
    try:
        async with manager:
            async for item in items:
                consume(item)
    finally:
        cleanup()
