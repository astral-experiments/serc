def f(xs,b):
    for x in xs:
        match x:
            case str():
                assert x
            case int():
                work(x)
            case _:
                other()
        assert x
