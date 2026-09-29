async def probe(work, fatal, retryable, server):
    for attempt in range(3):
        try:
            return await work()
        except fatal:
            raise
        except (retryable, server):
            if attempt == 2:
                raise
            await pause(attempt)
    raise AssertionError("retry did not return")
