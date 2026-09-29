def f(manager, x, y):
    try:
        with manager:
            pass
            work()
    finally:
        cleanup()
    if x:
        if y:
            pass
        assert y
