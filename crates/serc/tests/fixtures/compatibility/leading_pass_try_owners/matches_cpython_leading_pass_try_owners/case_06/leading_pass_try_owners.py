def f(x,g):
    try:
        g()
    except Exception:
        try:
            pass
            if x:
                return g()
        except RuntimeError:
            return g()
