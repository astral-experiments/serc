async def probe(value, ready, enabled, manager):
    try:
        if ready:
            async with manager:
                observe(value)
    except Exception:
        recover(value)
    consume(value)
