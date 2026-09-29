def f(x, y):
    try:
        a = [v for v in x] if y else []
        assert x
    finally:
        clean()
