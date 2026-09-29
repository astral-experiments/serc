def f(xs, a, b, c, gate):
    if gate:
        for x in xs:
            if x == a:
                work(x)
            elif x == b and x == c:
                work(x)
            else:
                assert x
    else:
        fallback()
