def f(x, y):
    if x:
        if y:
            work()
        assert y
    else:
        fallback()
