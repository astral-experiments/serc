def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        x6 = seed()
        x7 = seed()
        y0 = pair(a, b)
        stamp = clock.now()
        try:
            receiver.stream(
                a,
                flag or DEFAULT,
            )
        except Error:
            recover()
