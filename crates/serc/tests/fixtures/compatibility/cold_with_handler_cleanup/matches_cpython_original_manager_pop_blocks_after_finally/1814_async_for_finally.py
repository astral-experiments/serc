async def f(manager, items):
    count = 0
    with manager:
        try:
            async for item in items:
                count += 1
                yield item
        finally:
            report(count)
