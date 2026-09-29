async def retry(work, enabled, sink):
    success = False
    try:
        await work()
        success = True
    except OSError as outer_error:
        if enabled:
            for attempt in range(3):
                try:
                    await work()
                    return
                except OSError as inner_error:
                    pass
    finally:
        sink(success)
