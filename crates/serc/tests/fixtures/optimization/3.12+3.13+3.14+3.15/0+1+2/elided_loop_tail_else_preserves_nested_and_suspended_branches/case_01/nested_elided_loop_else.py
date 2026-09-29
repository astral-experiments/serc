
def f(xs, flag):
    for x in xs:
        if x:
            if flag:
                work(x)
        else:
            assert x
