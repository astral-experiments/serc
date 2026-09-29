def f(select, branch, a, b, c):
    if select:
        assert not a
    else:
        sink(2)
        sink(3)
    sink(4)
