async def consume(manager, items, write, managed):
    try:
        if managed:
            async with manager:
                async for item in items:
                    await write(item)
        else:
            async for item in items:
                await write(item)
    except RuntimeError:
        raise
    except BaseException:
        raise
