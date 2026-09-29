def f(cm):
    with cm:
        action()
        try:
            action()
        finally:
            cleanup()
        assert value
