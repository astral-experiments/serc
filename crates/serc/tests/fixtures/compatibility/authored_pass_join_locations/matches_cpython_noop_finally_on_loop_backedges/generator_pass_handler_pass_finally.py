def f(x, y):
    yield
    for q in y:
        try:
            x()
        except Exception:
            pass
        finally:
            pass
