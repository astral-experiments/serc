async def f(value):
    ready = False
    try:
        ready = await consume(value)
    finally:
        try:
            if not ready:
                await cleanup(value)
        finally:
            await finish(value)
