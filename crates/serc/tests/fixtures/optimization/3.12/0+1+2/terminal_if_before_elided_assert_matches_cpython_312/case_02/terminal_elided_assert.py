def f(x, y):
    if x:
        with context():
            call()
    else:
        if y:
            pass
        assert x
        call()
