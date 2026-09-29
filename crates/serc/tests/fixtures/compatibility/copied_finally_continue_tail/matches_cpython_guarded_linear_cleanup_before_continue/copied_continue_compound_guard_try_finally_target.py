async def f(items, preserve, attempts, last_error):
    for attempt in items:
        try:
            await work(attempt)
        except Exception:
            if retry(attempt):
                continue
            raise
        finally:
            if not preserve and (attempt >= attempts or last_error is None):
                started = clock()
                try:
                    await cleanup()
                finally:
                    record(started)
