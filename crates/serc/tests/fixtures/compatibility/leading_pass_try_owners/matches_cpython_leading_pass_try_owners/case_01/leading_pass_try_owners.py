def f(x,g):
    a = True
    try:
        pass
        a = g() if x else g()
    except Exception:
        a = False
    return a
