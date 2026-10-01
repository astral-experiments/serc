async def direct(items, first, second):
    async for item in items:
        break
    if first:
        assert first
    if second:
        assert second
    else:
        assert first


async def conditional(items, first, second):
    async for item in items:
        if item:
            break
    if first:
        assert first
    if second:
        assert second
    else:
        assert first


async def nested(items, first, second):
    async for outer in items:
        async for inner in outer:
            break
        if first:
            break
    if first:
        assert first
    if second:
        assert second
    else:
        assert first
