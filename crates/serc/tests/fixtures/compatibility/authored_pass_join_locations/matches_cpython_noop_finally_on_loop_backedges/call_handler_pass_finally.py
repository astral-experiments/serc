def f(x, y):
    for q in y:
        try:
            x()
        except Exception:
            x()
        finally:
            pass
