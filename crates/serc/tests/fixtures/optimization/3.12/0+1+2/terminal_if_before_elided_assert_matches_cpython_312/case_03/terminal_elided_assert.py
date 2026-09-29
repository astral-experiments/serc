def f(x, y):
    if x:
        with context():
            call()
    else:
        if x:
            assert x
        assert x
        return 1
