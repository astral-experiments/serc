async def run(items, manager, ready, work, recover, cleanup):
    try:
        async for item in items:
            with manager:
                if ready(item):
                    try:
                        await work(item)
                        return
                    except ValueError as error:
                        recover(error)
                await work(item)
                return
    except Exception:
        raise
    finally:
        cleanup()
