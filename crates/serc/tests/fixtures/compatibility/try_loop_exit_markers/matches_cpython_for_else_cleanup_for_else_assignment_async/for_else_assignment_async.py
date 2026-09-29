async def f(items, ready, recover, after, cleanup):
    try:
        try:
            for item in items:
                if await ready(item):
                    break
            else:
                completed = True
        except Error:
            await recover()
        await after()
    finally:
        await cleanup()
