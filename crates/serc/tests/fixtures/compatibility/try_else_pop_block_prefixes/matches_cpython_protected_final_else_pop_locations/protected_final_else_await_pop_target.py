async def f(flag, work, cleanup):
    try:
        if flag:
            raise ValueError(flag)
        else:
            await work()
    finally:
        cleanup()
