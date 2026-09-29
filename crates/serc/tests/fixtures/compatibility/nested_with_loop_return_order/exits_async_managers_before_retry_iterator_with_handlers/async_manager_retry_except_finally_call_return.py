async def f(items, manager, make, cleanup):
    try:
        for item in items:
            async with manager:
                return make(item)
    except ValueError:
        raise
    finally:
        cleanup()
