async def f(x, y):
    async for q in y:
        try:
            x()
        except Exception:
            pass
        finally:
            pass
