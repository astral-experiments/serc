async def f(work, after, flag):
    try:
        if flag:
            await work()
    except Exception:
        pass
    await after()
