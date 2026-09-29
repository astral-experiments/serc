def outer():
    a = None
    def f(x, y):
        nonlocal a
        if x:
            a = [i for i in y]
        assert x
    return f
