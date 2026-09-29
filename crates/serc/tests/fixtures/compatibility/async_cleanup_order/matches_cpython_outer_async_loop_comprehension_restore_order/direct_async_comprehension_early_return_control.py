async def f(page, finish):
    try:
        items = [item async for item in page]
        if not items:
            return
        for item in items:
            yield item
    finally:
        finish()
