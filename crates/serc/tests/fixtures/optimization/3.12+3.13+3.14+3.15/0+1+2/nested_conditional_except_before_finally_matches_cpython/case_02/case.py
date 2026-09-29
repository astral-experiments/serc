async def f(x, d):
    try:
        try:
            if x:
                work()
        except:
            pass
    finally:
        d.clear()
