def f(x, y):
    a = {v for v in x} if y else y
    assert x
    assert y
