def f(cm):
    with cm:
        token = action()
        try:
            action()
        finally:
            cleanup()
        assert token
