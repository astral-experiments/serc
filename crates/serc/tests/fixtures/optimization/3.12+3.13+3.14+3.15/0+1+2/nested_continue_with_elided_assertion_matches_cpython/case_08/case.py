def f(xs, a, b, cm):
    with cm:
        for x in xs:
            if a:
                if b:
                    continue
                assert x
