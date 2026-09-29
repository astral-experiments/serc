async def probe(work):
    try:
        return await work()
    except FatalError:
        return None
    except retryable_errors():
        raise
