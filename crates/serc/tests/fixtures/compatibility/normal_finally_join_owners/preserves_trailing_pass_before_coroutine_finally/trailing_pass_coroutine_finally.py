async def f(work, close):
    await work()
    try:
        await work()
        pass
    finally:
        await close()
        await close()
