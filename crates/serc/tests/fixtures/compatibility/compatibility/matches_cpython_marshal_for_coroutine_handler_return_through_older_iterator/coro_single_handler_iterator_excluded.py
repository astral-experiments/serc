async def f(values):
    try:
        for value in values:
            try:
                first(value)
            except FirstError:
                try:
                    return second(value)
                except SecondError:
                    observe(value)
    except OuterError:
        return None
