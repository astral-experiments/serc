def f(xs,b):
    for x in xs:
        match x:
            case str() | bytes():
                assert (z := x)
            case int():
                work(x)
            case _:
                other()
        z=x
    else:
        finished()
