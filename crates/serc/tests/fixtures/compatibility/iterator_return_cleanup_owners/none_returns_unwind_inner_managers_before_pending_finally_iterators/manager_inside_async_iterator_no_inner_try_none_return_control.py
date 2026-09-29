async def run(items, manager, ready, work, recover, cleanup):
    try:
        async for item in items:
            with manager:
                if ready(item):
                    await work(item)
                    return
                await work(item)
                return
    except Exception:
        raise
    finally:
        cleanup()
