def f(x, y):
    a = [v + w for v in x for w in x] if y else []
    assert x
    assert y
