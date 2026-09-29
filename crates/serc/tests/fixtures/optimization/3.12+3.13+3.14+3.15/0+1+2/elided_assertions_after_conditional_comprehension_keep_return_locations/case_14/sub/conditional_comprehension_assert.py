def outer():
    a = z = None
    def f(x, y):
        nonlocal a, z
        a = [(z := v) for v in x] if y else []
        assert x
        assert y
    return f
