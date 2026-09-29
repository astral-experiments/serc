async def f(pages, finish):
    try:
        async for page in pages:
            items = [item async for item in page]
            if not items:
                return
            for item in items:
                yield item
    finally:
        finish()
