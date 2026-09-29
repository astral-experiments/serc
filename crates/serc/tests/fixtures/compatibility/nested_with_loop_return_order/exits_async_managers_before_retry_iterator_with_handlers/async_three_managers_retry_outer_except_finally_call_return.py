async def f(items, outer, middle, inner, make, cleanup):
    try:
        async with outer:
            for item in items:
                try:
                    async with middle:
                        async with inner:
                            return make(item)
                except ValueError:
                    continue
    except RuntimeError:
        raise
    finally:
        cleanup()
