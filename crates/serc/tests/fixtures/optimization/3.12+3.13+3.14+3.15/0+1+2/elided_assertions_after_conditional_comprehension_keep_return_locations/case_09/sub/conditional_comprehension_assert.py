def f(x, y):
    a = [(z := v) for v in x] if y else []
    assert (z := y)
