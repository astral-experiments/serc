async def f(items, flag):
    async for item in items:
        assert item
        if flag:
            if item is None:
                pass
            assert item is not None
        else:
            if item is not None:
                pass
            assert item is None
    consume(items)
