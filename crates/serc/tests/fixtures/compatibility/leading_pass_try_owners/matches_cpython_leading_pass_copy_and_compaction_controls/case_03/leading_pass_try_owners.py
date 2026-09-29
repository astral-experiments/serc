async def f(x,g):
    async with x:
        try:
            pass
            a=g() if x else g()
        except Exception:
            a=False
    return a
