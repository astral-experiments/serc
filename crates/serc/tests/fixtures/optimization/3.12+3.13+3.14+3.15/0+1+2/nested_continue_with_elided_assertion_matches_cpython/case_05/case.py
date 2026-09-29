def f(xs, a, b):
    for x in xs:
        if a:
            if b:
                continue
            assert x
    else:
        cleanup()
