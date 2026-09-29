async def target(items, enabled):
    if enabled:
        async for item in items:
            consume(item)
    else:
        consume(None)
    return None
