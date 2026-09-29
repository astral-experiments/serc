async def f(items):
    count = 0
    skipped = 0
    async for item in items:
        count += 1
        if item is None:
            skipped += 1
            continue
        assert item.valid
        assert item.ready
    ratio = count / skipped
    assert ratio
