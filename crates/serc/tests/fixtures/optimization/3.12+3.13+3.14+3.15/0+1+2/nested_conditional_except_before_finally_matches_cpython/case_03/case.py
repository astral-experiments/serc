def f(x, d):
    try:
        try:
            if x:
                pass
        except Exception:
            pass
        else:
            work()
    finally:
        d.clear()
