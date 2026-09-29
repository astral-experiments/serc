def outer():
    g = None
    def f(xs, a, cm):
        for x in xs:
            if a:
                if x:
                    continue
                nonlocal g
                assert x
    return f
