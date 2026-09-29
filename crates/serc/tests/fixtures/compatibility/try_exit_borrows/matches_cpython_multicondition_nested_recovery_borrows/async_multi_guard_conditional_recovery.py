async def probe(value, ready, enabled):
    try:
        if ready and enabled:
            try:
                await observe(value)
            except ValueError:
                result = await recover(value)
                enabled = result is None or not valid(result)
                if not enabled:
                    log(value)
    except Exception:
        enabled = False
    if not enabled:
        return None
    consume(value)
    finish(value)
    return value
