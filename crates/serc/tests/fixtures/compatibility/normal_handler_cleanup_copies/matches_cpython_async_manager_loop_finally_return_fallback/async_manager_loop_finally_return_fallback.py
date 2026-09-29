async def f(manager, items, cleanup):
    try:
        async with manager:
            async for item in items:
                try:
                    if item is None:
                        continue
                    if item:
                        return item
                    return None
                finally:
                    try:
                        if item is not None:
                            cleanup(item)
                    except Exception:
                        pass
    except Exception:
        pass
    return None
