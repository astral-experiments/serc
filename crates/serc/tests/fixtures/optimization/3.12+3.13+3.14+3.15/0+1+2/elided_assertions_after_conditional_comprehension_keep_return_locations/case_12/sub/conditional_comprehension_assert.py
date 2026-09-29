def f(x, y):
    global a, z
    a = [v for v in x] if y else []
    assert x
    assert y
