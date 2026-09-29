def f(xs, a, b, c):
    for x in xs:
        if x == a:
            assert x
        elif x == b or x == c:
            assert x
        work(x)
