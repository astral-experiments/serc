
def f(xs):
    for x in xs:
        if x:
            yield x
        else:
            assert x
