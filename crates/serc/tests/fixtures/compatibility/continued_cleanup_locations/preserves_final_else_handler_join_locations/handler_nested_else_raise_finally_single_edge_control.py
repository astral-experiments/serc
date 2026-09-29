async def f(call, log, dev, enabled, finalize):
    try:
        try:
            await call()
        except BaseException:
            if dev:
                raise
            else:
                def report():
                    log()
                if enabled:
                    raise
        else:
            await log(None)
    finally:
        finalize()
    return 1
