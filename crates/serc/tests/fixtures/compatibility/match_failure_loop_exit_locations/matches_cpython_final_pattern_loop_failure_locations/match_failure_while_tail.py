def f(xs):
    while xs:
        x = take(xs)
        try:
            match x:
                case 0:
                    observe(x)
                case Kind.VALUE:
                    observe(x)
        except Error:
            recover()
