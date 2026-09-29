async def guarded_cleanup(enabled, work, release, pack):
    result = None
    try:
        if enabled:
            try:
                result = await work()
            finally:
                try:
                    await release()
                except Exception:
                    pass
    except Exception:
        result = False
    return pack(result)
