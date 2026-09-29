def f(xs):
    while xs:
        x = xs.pop()
        match x:
            case str():
                assert x
            case _:
                other()
        pass
