def f(xs,b):
    while xs:
        x=xs.pop()
        match x:
            case str():
                assert x
            case int():
                work(x)
            case _:
                other()
        assert x
