async def target(items, enabled, value):
    if enabled:
        value = None
        for item in items:
            value = transform(item)
    else:
        value = transform(None)
    return value
