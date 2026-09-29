def f(select, branch, a, b, c):
    if select:
        if branch:
            sink(1)
        else:
            assert a is None and b is None and c is None
    else:
        sink(2)
        sink(3)
    sink(4)
