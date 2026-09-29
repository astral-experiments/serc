def f(x, y):
    global v
    match x:
        case False:
            work()
        case str():
            assert (v := x)
            assert y
        case True:
            work()
