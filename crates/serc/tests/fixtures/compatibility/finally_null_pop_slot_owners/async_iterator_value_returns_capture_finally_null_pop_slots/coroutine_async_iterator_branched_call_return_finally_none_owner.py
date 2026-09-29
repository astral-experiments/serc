async def f(source, prepare, stop, process, cleanup):
    reload = False
    status = None
    try:
        if await prepare():
            status = Result.SKIPPED
            return Finish(retry=True, reload=reload)
        accepted = True
        async for item in source:
            process(item)
            if stop(item):
                status = (
                    Result.SUCCESS
                    if accepted
                    else Result.ERROR
                )
                return Finish(
                    retry=False, reload=reload
                )
            if item.failed:
                accepted = False
        return Finish(retry=True, reload=reload)
    except Exception:
        status = Result.ERROR
        raise
    finally:
        if status is None:
            status = Result.MISSING
        record(status.value)
        cleanup(status)
