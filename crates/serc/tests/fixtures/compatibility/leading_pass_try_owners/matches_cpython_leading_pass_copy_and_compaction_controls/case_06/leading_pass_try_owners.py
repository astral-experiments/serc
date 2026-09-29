def f(x,g):
    a=2+3
    try:
        pass
        a=g() if x else g()
    except Exception:
        a=False
    return a
