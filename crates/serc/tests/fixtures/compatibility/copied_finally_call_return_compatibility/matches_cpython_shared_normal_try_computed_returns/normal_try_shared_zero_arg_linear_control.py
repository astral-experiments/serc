async def run(enabled, ready, work, first, second, missing, report):
    if enabled:
        try:
            await work()
            report()
        except RuntimeError:
            report()
    return Result()
