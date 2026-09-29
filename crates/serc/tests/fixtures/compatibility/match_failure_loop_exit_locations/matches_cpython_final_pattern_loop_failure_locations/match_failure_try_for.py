def f(xs):
    for x in xs:
        try:
            match x:
                case 0:
                    observe(x)
                case Kind.VALUE:
                    observe(x)
        except Error:
            recover()
