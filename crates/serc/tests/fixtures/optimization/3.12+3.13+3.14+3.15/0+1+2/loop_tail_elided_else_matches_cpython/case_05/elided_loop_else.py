
def f(xs):
    for x in xs:
        if x:
            work(x)
        else:
            assert x
            work_else(x)
