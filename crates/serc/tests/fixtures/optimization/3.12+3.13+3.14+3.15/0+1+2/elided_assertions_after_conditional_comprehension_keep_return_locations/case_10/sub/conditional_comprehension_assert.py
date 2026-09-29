def outer():
    a = z = w = None
    def f(x, y):
        nonlocal a, z, w
        a = [v for v in x] if y else []
        assert (z := y)
    return f
