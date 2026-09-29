def f(cm, x):
    with cm:
        try:
            work()
        finally:
            cleanup()
        assert x
