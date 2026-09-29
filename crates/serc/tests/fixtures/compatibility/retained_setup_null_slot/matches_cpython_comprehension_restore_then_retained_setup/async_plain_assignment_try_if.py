async def f(value):
    ready = value
    try:
        if ready:
            return await consume(value)
    except Exception:
        return None
    return value
