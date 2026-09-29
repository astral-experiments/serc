async def f(items):
    for item in items:
        if item:
            assert item
        else:
            assert items
        work(item)
