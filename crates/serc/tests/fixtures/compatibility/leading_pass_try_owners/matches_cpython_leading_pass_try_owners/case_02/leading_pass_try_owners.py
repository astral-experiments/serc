def f(x,g):
    a=True
    try:
        pass
        a=g() if x else g()
    finally:
        g()
    return a
