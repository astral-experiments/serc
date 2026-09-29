async def sample(a, b, call, finish):
    try:
        if a:
            if b:
                if a is not None:
                    try:
                        await call()
                    except Exception:
                        await call()
    except BaseException:
        await call()
    finally:
        await finish()
