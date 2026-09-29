async def f(value):
    try:
        await prepare(value)
        try:
            first(value)
        except FirstError as outer_error:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return None
        return value
    except OuterError:
        return None
