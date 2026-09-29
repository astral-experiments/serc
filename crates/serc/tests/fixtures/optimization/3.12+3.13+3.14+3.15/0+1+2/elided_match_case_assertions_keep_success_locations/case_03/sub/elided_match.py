def f(x, y):
    match x:
        case [1] | [2]:
            assert x
        case False:
            work()
        case True:
            work()
