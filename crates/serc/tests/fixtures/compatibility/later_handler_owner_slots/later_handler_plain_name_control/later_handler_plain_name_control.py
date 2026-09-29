async def probe(work, fatal, retryable):
    try:
        return await work()
    except fatal:
        raise
    except retryable:
        raise
