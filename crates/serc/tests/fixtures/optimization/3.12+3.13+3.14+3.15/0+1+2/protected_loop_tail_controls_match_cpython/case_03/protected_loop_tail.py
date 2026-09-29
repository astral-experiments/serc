
def f(xs):
    try:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
    finally:
        cleanup()
