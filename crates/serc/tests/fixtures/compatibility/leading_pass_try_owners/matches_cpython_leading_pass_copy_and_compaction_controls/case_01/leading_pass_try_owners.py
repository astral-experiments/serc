def f(x,g):
    with x:
        try:
            pass
            a=g() if x else g()
        except Exception:
            a=False
    return a
