async def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        x6 = seed()
        x7 = seed()
        x8 = seed()
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
