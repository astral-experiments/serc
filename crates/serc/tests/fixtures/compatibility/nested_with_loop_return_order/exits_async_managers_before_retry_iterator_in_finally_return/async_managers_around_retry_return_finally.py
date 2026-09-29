async def f(items, outer, inner, value, cleanup):
    try:
        async with outer:
            for item in items:
                async with inner:
                    return value
    finally:
        cleanup()
