def f(cm, x, y):
    with cm:
        try:
            work()
        finally:
            cleanup()
        assert x; assert y
