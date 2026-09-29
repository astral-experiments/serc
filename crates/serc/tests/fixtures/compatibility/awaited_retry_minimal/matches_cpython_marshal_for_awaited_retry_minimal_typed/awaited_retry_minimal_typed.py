async def retry(work, enabled, sink):
    success = False
    try:
        await work()
        success = True
    except OSError:
        if enabled:
            for attempt in range(3):
                try:
                    await work()
                    return
                except OSError:
                    pass
    finally:
        sink(success)
