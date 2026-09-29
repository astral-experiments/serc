def f(x, y):
    if x and y:
        assert x
    else:
        assert y
    work()
    yield 1
