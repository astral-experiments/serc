
def f(xs, flag):
    for x in xs:
        if x:
            work(x)
        elif flag:
            if x > 1:
                work_else(x)
        else:
            assert x
