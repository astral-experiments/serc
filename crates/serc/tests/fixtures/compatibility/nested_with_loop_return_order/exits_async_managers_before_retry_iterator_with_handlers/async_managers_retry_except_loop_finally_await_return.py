async def f(items, outer, inner, work, parts, cleanup):
    try:
        async with outer:
            for item in items:
                async with inner:
                    return await work(item)
    except ValueError:
        raise
    finally:
        for part in parts:
            await cleanup(part)
