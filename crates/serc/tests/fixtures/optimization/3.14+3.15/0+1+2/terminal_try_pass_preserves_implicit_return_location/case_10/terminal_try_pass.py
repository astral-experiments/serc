def f(x, y):
    try:
        if y:
            assert x
        else:
            assert y
        del x
        pass
    except ValueError:
        recover()
