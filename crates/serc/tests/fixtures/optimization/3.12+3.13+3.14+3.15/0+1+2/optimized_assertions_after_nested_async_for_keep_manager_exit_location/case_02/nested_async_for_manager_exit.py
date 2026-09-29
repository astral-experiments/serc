async def f(first, second, third, items):
    with first, second:
        with third:
            async for item in items:
                work(item)
            assert items
