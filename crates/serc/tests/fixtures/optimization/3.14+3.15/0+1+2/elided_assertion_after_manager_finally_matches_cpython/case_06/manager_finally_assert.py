async def f(cm, x):
    await before()
    with cm:
        try:
            work()
        finally:
            cleanup()
        assert x
