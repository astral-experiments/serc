async def probe(value, ready, enabled, manager):
    try:
        if ready and enabled:
            async with manager:
                observe(value)
    except Exception:
        recover(value)
    consume(value)
