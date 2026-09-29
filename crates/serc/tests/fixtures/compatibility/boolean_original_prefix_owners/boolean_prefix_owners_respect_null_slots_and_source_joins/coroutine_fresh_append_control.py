async def f(values, a, b, flag, receiver):
    for item in values:
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
