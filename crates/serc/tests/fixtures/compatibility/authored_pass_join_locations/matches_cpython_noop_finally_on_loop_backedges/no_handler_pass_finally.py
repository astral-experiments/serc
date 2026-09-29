def f(x, y):
    for q in y:
        try:
            x()
        finally:
            pass
