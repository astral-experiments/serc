async def f(items, pending, call, enabled, retryable, save):
    for item in items:
        try:
            value = await call(item)
        except Exception as exc:
            save(exc)
            if not enabled and not retryable(exc):
                raise
        else:
            save(value)
        while pending and len(pending) > 1:
            save(pending.pop())
