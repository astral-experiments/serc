
def f(xs, cm):
    with cm:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
