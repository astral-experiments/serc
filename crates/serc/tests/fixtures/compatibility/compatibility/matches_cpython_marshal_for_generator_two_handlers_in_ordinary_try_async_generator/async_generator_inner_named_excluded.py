async def f(value):
    yield value
    try:
        await prepare(value)
        try:
            first(value)
        except FirstError:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return
        return
    except OuterError:
        return
