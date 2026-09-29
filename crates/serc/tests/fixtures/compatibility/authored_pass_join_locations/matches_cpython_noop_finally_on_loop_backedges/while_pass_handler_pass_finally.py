def f(x, y):
    while y:
        try:
            x()
        except Exception:
            pass
        finally:
            pass
