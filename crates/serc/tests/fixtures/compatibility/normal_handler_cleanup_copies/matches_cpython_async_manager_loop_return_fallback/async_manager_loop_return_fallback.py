async def f(manager, items, ready):
    try:
        async with manager:
            if await ready():
                return 1
            async for item in items:
                if item:
                    return item
    except LookupError:
        return None
    except Exception:
        return None
    return None
