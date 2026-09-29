def f(manager, x, y):
    try:
        with manager:
            pass
            assert x
            work()
    finally:
        cleanup()
    if x:
        if y:
            pass
        assert y
