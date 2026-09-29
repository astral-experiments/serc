def f(xs, work, recover, x, y):
    try:
        try:
            work()
        except ValueError:
            recover()
    except Exception:
        recover()
    for item in xs:
        if not item:
            assert False
        if not item:
            assert False
    if x:
        if y:
            work()
        assert y
    else:
        recover()
