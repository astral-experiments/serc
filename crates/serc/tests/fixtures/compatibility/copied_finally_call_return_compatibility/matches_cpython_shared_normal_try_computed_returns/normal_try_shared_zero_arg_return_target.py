async def run(enabled, ready, work, first, second, missing, report):
    if enabled:
        try:
            await work()
            if ready:
                if first:
                    second()
                else:
                    missing()
            else:
                report()
        except RuntimeError:
            report()
    return Result()
