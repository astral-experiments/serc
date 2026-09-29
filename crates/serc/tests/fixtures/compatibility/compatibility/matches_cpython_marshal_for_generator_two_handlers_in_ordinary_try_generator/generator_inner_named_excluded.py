def f(value):
    yield value
    try:
        prepare(value)
        try:
            first(value)
        except FirstError:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return None
        return value
    except OuterError:
        return None
