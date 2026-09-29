def f(x, y, callback, manager):
    with manager:
        if x:
            assert x
        else:
            assert y
        callback(x)
    a = [v for v in x] if y else []
    assert x
