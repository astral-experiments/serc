def f(cm):
    with cm:
        try:
            action()
        finally:
            cleanup()
        pass
        assert value
