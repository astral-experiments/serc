def f(x,g):
    try:
        pass
        yield g() if x else g()
    finally:
        g()
