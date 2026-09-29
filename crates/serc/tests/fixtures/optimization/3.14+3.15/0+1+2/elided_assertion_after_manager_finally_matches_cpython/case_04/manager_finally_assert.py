def f(cm, other, x):
    with cm, other:
        try:
            work()
        finally:
            cleanup()
        assert x
