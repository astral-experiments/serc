async def probe(a0, a1, a2, a3, a4, a5, a6, a7,
                a8, a9, a10, a11, a12, a13, a14, a15,
                work, fatal, retryable, server):
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
