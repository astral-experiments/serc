def f(xs, work, recover, x, y):
    try:
        work()
    except ValueError:
        recover()
    for item in xs:
        if item:
            assert False
        if item:
            assert False
    if x:
        if y:
            pass
        assert y
