async def target(items, enabled, value):
    if enabled:
        value = None
        async for item in items:
            value = transform(item)
    else:
        value = transform(None)
    return value
