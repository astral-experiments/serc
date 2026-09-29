def f(cm):
    with cm:
        token = action()
        pass
        assert token
