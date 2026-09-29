def f(x, y, z, cm):
    with cm:
        if x:
            a = [i for i in y]
        assert x
