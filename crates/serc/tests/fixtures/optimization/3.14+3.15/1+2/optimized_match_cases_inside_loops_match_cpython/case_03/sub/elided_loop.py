def f(xs,b):
    for group in xs:
        for x in group:
            match x:
                case str() | bytes():
                    assert x
                case int():
                    work(x)
                case _:
                    other()
            z=[v for v in xs]
