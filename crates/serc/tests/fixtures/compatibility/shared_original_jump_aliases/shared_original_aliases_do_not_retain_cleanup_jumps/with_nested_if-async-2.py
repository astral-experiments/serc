async def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    with manager:
                        await call()
        except Exception:
            await call()
        finally:
            await finish()
