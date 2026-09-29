async def f(work, after):
    try:
        if False:
            await work()
    except Exception:
        pass
    await after()
