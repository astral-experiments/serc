def f(flag, x, y):
    try:
        if True:
            if flag:
                work()
        else:
            other()
        assert flag
    except ValueError:
        recover()
    if x:
        if y:
            work()
        assert y
