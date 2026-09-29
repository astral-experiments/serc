async def run(items, manager, ready, cleanup):
    try:
        for item in items:
            async with manager:
                if ready(item):
                    return
    finally:
        cleanup()
