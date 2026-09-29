
def f(xs):
    for x in xs:
        try:
            if x:
                work(x)
            else:
                assert x
        except ValueError:
            cleanup()
