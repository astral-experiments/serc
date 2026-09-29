async def f(manager, pending, task):
    def current():
        return pending
    with manager:
        try:
            await task(current)
        finally:
            if pending is not None:
                pending.close()
