def f(x, y):
    if x and y:
        if x:
            pass
        assert y
    elif y:
        fallback()
    else:
        return 1
