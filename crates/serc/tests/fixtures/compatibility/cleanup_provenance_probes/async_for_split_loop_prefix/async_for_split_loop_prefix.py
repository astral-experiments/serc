async def collect(items, ready, done):
    try:
        while ready():
            async for item in items:
                await consume(item)
            if done():
                break
    except RuntimeError:
        recover()
