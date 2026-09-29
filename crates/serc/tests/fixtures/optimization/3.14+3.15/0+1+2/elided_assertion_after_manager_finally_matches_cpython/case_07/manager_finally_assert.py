def f(cm, x, y, value):
    with cm:
        try:
            if x:
                if y:
                    work()
                assert value
            else:
                other()
        finally:
            cleanup()
        assert value
