def f(x, y):
    if x:
        if y:
            try:
                work()
            finally:
                cleanup()
        assert y
    else:
        raise ValueError()
