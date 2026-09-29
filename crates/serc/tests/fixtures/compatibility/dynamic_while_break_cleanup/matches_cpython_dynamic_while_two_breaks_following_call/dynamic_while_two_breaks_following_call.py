async def drain(pending, expired, work, finish):
    try:
        while pending():
            if expired():
                break
            done = await work()
            if not done:
                break
    except Exception:
        raise
    finish()
    return None
