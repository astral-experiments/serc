def f(x,g):
    try:
        pass; a=g() if x else g()
    except Exception:
        a=False
    return a
