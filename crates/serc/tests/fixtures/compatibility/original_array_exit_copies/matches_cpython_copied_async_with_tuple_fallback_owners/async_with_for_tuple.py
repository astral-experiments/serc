async def probe(manager):
    left = []
    right = {}
    try:
        async with manager as stream:
            async for item in stream:
                if not item:
                    continue
                left.append(item)
                right[item] = item
    except Exception:
        report()
    return left, right
