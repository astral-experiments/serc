async def target(items, enabled, value):
    if enabled:
        async for item in items:
            value = transform(item)
        else:
            finish()
    else:
        value = transform(None)
    return value
