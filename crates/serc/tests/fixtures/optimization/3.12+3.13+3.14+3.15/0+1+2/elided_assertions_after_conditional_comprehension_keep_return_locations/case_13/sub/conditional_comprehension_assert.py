def f(x, y):
    def inner():
        return a, z
    a = [(z := v) for v in x] if y else []
    assert x
    assert y
