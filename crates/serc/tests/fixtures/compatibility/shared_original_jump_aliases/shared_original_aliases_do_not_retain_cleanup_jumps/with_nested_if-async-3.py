async def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    if a is not None:
                        with manager:
                            await call()
        except Exception:
            await call()
        finally:
            await finish()
