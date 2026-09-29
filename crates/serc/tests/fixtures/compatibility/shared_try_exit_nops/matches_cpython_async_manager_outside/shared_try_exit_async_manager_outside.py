async def f(manager, flag):
    async with manager:
        try:
            ping()
        except Exception:
            pong()
        finally:
            if flag:
                cleanup()
