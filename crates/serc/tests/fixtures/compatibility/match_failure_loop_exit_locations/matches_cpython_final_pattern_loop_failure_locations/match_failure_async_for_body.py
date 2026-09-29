async def f(xs):
    for x in xs:
        try:
            match x:
                case 0:
                    await observe(x)
                case Kind.VALUE:
                    await observe(x)
        except Error:
            recover()
