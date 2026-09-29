async def consume(manager, items, write, finish):
    try:
        async with manager:
            async for item in items:
                await write(item)
        await finish()
    except RuntimeError:
        raise
