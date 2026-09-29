async def f(flag, manager, work, close):
    try:
        if flag:
            return
    finally:
        with manager:
            try:
                work()
            except ValueError:
                raise
            finally:
                await close()
