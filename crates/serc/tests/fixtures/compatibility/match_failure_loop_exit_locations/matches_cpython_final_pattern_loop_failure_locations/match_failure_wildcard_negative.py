def f(xs, flag):
    for x in xs:
        if flag:
            match x:
                case 0:
                    observe(x)
                case _:
                    observe(x)
