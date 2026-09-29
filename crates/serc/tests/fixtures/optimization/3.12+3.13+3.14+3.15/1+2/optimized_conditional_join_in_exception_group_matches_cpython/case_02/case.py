def f(xs, a, b):
    try:
        work()
    except* ValueError as err:
        for x in xs:
            if a or b:
                assert x
            else:
                assert x
