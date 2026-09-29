
def f(xs):
    n = None
    for count in xs:
        if n is None:
            n = count
        else:
            assert n == count
    return n
