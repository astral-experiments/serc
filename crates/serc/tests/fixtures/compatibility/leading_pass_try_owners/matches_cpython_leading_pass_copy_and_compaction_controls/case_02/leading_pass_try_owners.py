def f(x,g):
    with x:
        try:
            pass
            a=g() if x else g()
        finally:
            g()
    return a
