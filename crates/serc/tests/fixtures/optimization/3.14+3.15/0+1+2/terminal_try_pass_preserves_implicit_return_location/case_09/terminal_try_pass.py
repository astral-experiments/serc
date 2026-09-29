def f(x, y):
    try:
        if y:
            assert x
        else:
            assert y
        z = x + 1
        pass
    except ValueError:
        recover()
